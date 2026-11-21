//! Repository tasks.
//!
//! - `cargo xtask layers`: checks the crate layering.
//! - `cargo xtask scripts`: builds the example game scripts for WebAssembly and copies them into
//!   `examples/demo/scripts/`.
//! - `cargo xtask licences [--write]`: checks every dependency's licence against the craft-contrib
//!   list and that NOTICE lists them all; `--write` writes NOTICE.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod layers;
mod licences;

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

/// WebAssembly target the example scripts build for.
const SCRIPT_TARGET: &str = "wasm32-unknown-unknown";

fn main() -> anyhow::Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("layers") => check_layers(&workspace_root()?),
        Some("scripts") => build_scripts(&workspace_root()?),
        Some("licences") => licences::check(&workspace_root()?, std::env::args().any(|a| a == "--write")),
        _ => bail!("usage: cargo xtask layers | scripts | licences [--write]"),
    }
}

fn build_scripts(root: &Path) -> anyhow::Result<()> {
    let manifest = root.join("scripts").join("Cargo.toml");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = std::process::Command::new(cargo)
        .args(["build", "--release", "--target", SCRIPT_TARGET, "--manifest-path"])
        .arg(&manifest)
        .status()
        .context("could not run cargo")?;
    if !status.success() {
        bail!("building the scripts failed; if the target is missing, run: rustup target add {SCRIPT_TARGET}");
    }
    let built = root.join("scripts").join("target").join(SCRIPT_TARGET).join("release");
    let out = root.join("examples").join("demo").join("scripts");
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    let mut copied = 0;
    for entry in std::fs::read_dir(&built).with_context(|| format!("reading {}", built.display()))? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "wasm")
            && let Some(name) = path.file_name()
        {
            std::fs::copy(&path, out.join(name)).with_context(|| format!("copying {}", path.display()))?;
            println!("scripts: {}", out.join(name).display());
            copied += 1;
        }
    }
    if copied == 0 {
        bail!("no .wasm files were built in {}", built.display());
    }
    Ok(())
}

fn workspace_root() -> anyhow::Result<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().map(Path::to_path_buf).context("xtask has no parent directory")
}

fn check_layers(root: &Path) -> anyhow::Result<()> {
    let mut problems = Vec::new();
    for group in ["crates", "apps"] {
        for entry in std::fs::read_dir(root.join(group)).with_context(|| format!("reading {group}/"))? {
            let manifest_path = entry?.path().join("Cargo.toml");
            if manifest_path.is_file() {
                problems.extend(check_manifest(&manifest_path)?);
            }
        }
    }
    if problems.is_empty() {
        println!("layers: ok");
        Ok(())
    } else {
        for p in &problems {
            eprintln!("layers: {p}");
        }
        bail!("{} layer problem(s)", problems.len())
    }
}

fn check_manifest(path: &Path) -> anyhow::Result<Vec<String>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let manifest: toml::Table = text.parse().with_context(|| format!("parsing {}", path.display()))?;
    let name = manifest.get("package").and_then(|p| p.get("name")).and_then(|n| n.as_str()).context("package.name missing")?.to_owned();
    let Some(own) = layers::layer_of(&name) else {
        return Ok(vec![format!("{name} is not registered in xtask/src/layers.rs")]);
    };
    let mut problems = Vec::new();
    let deps = manifest.get("dependencies").and_then(|d| d.as_table()).cloned().unwrap_or_default();
    for dep in deps.keys() {
        if let Some(dep_layer) = layers::layer_of(dep) {
            if dep_layer >= own {
                problems.push(format!("{name} (L{own}) depends on {dep} (L{dep_layer})"));
            }
        } else if own < 6 && layers::UI_ONLY.contains(&dep.as_str()) {
            problems.push(format!("{name} (L{own}) uses UI crate {dep}"));
        }
    }
    Ok(problems)
}
