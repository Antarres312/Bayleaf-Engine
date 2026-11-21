//! Headless Bayleaf Engine: run commands, batch jobs and (later) the MCP server.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::PathBuf;

use anyhow::{Context, bail};
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
    /// Start the MCP server on stdio. It requires both automation roots (AA 18).
    Mcp {
        #[arg(long = "automation-read-root", required = true)]
        read_roots: Vec<PathBuf>,
        #[arg(long = "automation-write-root", required = true)]
        write_roots: Vec<PathBuf>,
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
        Cmd::Mcp { .. } => {
            bail!("the MCP server is not implemented yet (milestone M4)");
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
