//! Repository checks. Run `cargo xtask layers`.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod layers;

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

fn main() -> anyhow::Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("layers") => check_layers(&workspace_root()?),
        _ => bail!("usage: cargo xtask layers"),
    }
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
