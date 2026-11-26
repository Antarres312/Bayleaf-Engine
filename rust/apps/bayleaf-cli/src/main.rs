//! Headless Bayleaf Engine: run commands, batch jobs and (later) the MCP server.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::PathBuf;

use anyhow::{Context, bail};
use bayleaf_automation::control::ControlClient;
use bayleaf_automation::mcp::{self, Backend, BridgeBackend, LocalBackend};
use bayleaf_engine::confine::Confinement;
use bayleaf_engine::{CommandRegistry, Session};
use clap::{Parser, Subcommand};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Parser)]
#[command(name = "bayleaf-cli", version, about = "Headless Bayleaf Engine")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print every registered command as a JSON array.
    Commands,
    /// Run one command and print its JSON result.
    Run {
        /// Command id, e.g. `scene.objects`.
        #[arg(long)]
        cmd: String,
        /// Params as a JSON object.
        #[arg(long, default_value = "{}")]
        params: String,
        /// Scene file to open before running the command.
        #[arg(long)]
        scene: Option<PathBuf>,
    },
    /// Run a JSON array of steps `[{"cmd": "...", "params": {...}}]` in one session.
    /// Stops at the first failing step. `$N.field` in a string param refers to step N's result.
    Batch {
        /// Steps file.
        steps: PathBuf,
        /// Scene file to open before the first step.
        #[arg(long)]
        scene: Option<PathBuf>,
    },
    /// Run an MCP server on stdio for AI agents. It refuses to start without both automation roots,
    /// and refuses file access outside them (AA 18).
    Mcp {
        /// A folder the agent may read. Repeat for more.
        #[arg(long = "automation-read-root", required = true)]
        read_roots: Vec<PathBuf>,
        /// A folder the agent may write (and read). Repeat for more. Relative paths start here.
        #[arg(long = "automation-write-root", required = true)]
        write_roots: Vec<PathBuf>,
        /// Drive a running editor instead of a session of its own: its control channel address,
        /// e.g. 127.0.0.1:7878. The editor applies its own automation roots.
        #[arg(long)]
        bridge: Option<std::net::SocketAddr>,
        /// Token file of the editor's control channel (with --bridge).
        #[arg(long = "control-token-file", requires = "bridge")]
        token_file: Option<PathBuf>,
        /// Scene file to open first (local session only).
        #[arg(long, conflicts_with = "bridge")]
        scene: Option<PathBuf>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    cmd: String,
    #[serde(default)]
    params: Value,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    match cli.command {
        Cmd::Commands => {
            println!("{}", serde_json::to_string_pretty(&registry.list(&session))?);
        }
        Cmd::Run { cmd, params, scene } => {
            open_scene(&registry, &mut session, scene)?;
            let params: Value = serde_json::from_str(&params).context("--params is not valid JSON")?;
            let result = registry.execute(&mut session, &cmd, &params)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Cmd::Batch { steps, scene } => {
            open_scene(&registry, &mut session, scene)?;
            let text = std::fs::read_to_string(&steps).with_context(|| format!("cannot read {}", steps.display()))?;
            let steps: Vec<Step> = serde_json::from_str(&text).context("the steps file must be a JSON array of {cmd, params}")?;
            let mut results: Vec<Value> = Vec::with_capacity(steps.len());
            for (index, step) in steps.into_iter().enumerate() {
                let params = substitute(step.params, &results);
                match registry.execute(&mut session, &step.cmd, &params) {
                    Ok(result) => results.push(result),
                    Err(error) => {
                        println!("{}", serde_json::to_string_pretty(&json!({ "results": results, "failedStep": index, "error": error.to_string() }))?);
                        bail!("step {index} ({}) failed: {error}", step.cmd);
                    }
                }
            }
            println!("{}", serde_json::to_string_pretty(&json!({ "results": results }))?);
        }
        Cmd::Mcp { read_roots, write_roots, bridge, token_file, scene } => {
            // Checked in both modes, so the roots are always real folders (AA 18).
            let confinement = Confinement::new(&read_roots, &write_roots).map_err(anyhow::Error::msg)?;
            let stdout = std::io::stdout();
            match bridge {
                Some(addr) => {
                    let path = token_file.context("--bridge needs --control-token-file, the token file the editor printed")?;
                    let token = std::fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
                    let client = ControlClient::connect(addr, token.trim()).map_err(anyhow::Error::msg)?;
                    eprintln!("bayleaf mcp: bridged to the editor at {addr}");
                    mcp::serve(&mut BridgeBackend::new(client), std::io::BufReader::new(std::io::stdin()), stdout.lock())?;
                }
                None => {
                    let mut backend = LocalBackend::new(confinement);
                    if let Some(scene) = scene {
                        backend.execute("scene.open", &json!({ "path": scene })).map_err(anyhow::Error::msg)?;
                    }
                    eprintln!("bayleaf mcp: ready on stdio");
                    mcp::serve(&mut backend, std::io::BufReader::new(std::io::stdin()), stdout.lock())?;
                }
            }
        }
    }
    Ok(())
}

fn open_scene(registry: &CommandRegistry, session: &mut Session, scene: Option<PathBuf>) -> anyhow::Result<()> {
    if let Some(path) = scene {
        registry.execute(session, "scene.open", &json!({ "path": path }))?;
    }
    Ok(())
}

/// Replaces string params of the form `$N.field` with `field` of step N's result.
fn substitute(value: Value, results: &[Value]) -> Value {
    match value {
        Value::String(text) => reference(&text, results).unwrap_or(Value::String(text)),
        Value::Array(items) => Value::Array(items.into_iter().map(|v| substitute(v, results)).collect()),
        Value::Object(fields) => Value::Object(fields.into_iter().map(|(k, v)| (k, substitute(v, results))).collect()),
        other => other,
    }
}

fn reference(text: &str, results: &[Value]) -> Option<Value> {
    let (step, field) = text.strip_prefix('$')?.split_once('.')?;
    results.get(step.parse::<usize>().ok()?)?.get(field).cloned()
}

#[cfg(test)]
mod tests {
    use super::substitute;
    use serde_json::json;

    #[test]
    fn step_references_resolve_and_other_text_is_kept() {
        let results = vec![json!({"id": "abc"})];
        let params = json!({"parent": "$0.id", "name": "$price", "missing": "$3.id"});
        assert_eq!(substitute(params, &results), json!({"parent": "abc", "name": "$price", "missing": "$3.id"}));
    }
}
