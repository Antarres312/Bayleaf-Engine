//! `cargo xtask licences`: every dependency's licence must be on the craft-contrib list
//! (`standards/licences.md`), and NOTICE must list every dependency with its licence.
//! `cargo xtask licences --write` writes NOTICE again.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, bail};
use serde_json::Value;

/// The licences allowed by craft-contrib `standards/licences.md`. MPL-2.0 is allowed only for
/// third-party code used unmodified, which is the only way dependencies are used here.
const ALLOWED: &[&str] = &[
    "ISC",
    "MIT",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "Unicode-3.0",
    "Zlib",
    "BSL-1.0",
    "0BSD",
    "Unlicense",
    "IJG",
    "NCSA",
    "CDLA-Permissive-2.0",
    "CC0-1.0",
    "CC-BY-4.0",
    "CC-BY-SA-4.0",
    "OFL-1.1",
    "Ubuntu-font-1.0",
    "Bitstream-Vera",
    "LicenseRef-Public-Domain",
    "MPL-2.0",
];

/// Crates offered under a choice of licences where one choice is not on the list, used under the one
/// that is: (crate, the licence it declares, the licence we use it under), as the other Craft apps do
/// (craft-anything, anicraft). A crate that declares another licence later needs a new decision.
const ELECTED: &[(&str, &str, &str)] = &[("self_cell", "Apache-2.0 OR GPL-2.0-only", "Apache-2.0")];

/// The targets the apps are built for; dependencies only other targets use (UEFI, for example)
/// are not shipped.
const SHIPPED_TARGETS: &[&str] = &["x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu", "aarch64-apple-darwin", "wasm32-unknown-unknown"];

/// The identifiers in a licence expression. An expression is allowed when every one is.
pub fn identifiers(expression: &str) -> Vec<String> {
    let spaced = expression.replace(['(', ')'], " ");
    let mut out = Vec::new();
    // Old-style "MIT/Apache-2.0" means the same as "MIT OR Apache-2.0".
    for part in spaced.split(" OR ").flat_map(|p| p.split(" AND ")).flat_map(|p| p.split('/')) {
        let id = part.split_whitespace().collect::<Vec<_>>().join(" ");
        let id = id.strip_suffix('+').map(str::to_owned).unwrap_or(id);
        if !id.is_empty() {
            out.push(id);
        }
    }
    out
}

pub fn allowed(expression: &str) -> bool {
    let ids = identifiers(expression);
    !ids.is_empty() && ids.iter().all(|id| ALLOWED.contains(&id.as_str()))
}

/// The licence a crate is used under when it is in [`ELECTED`] with the licence it declares: one of
/// its choices, and allowed.
pub fn elected(name: &str, expression: &str) -> Option<&'static str> {
    ELECTED
        .iter()
        .find(|(crate_name, declared, _)| *crate_name == name && *declared == expression)
        .map(|(_, _, chosen)| *chosen)
        .filter(|chosen| allowed(chosen) && expression.split(" OR ").any(|choice| choice.trim() == *chosen))
}

/// Whether a package's licence passes: allowed as declared, or by a recorded choice.
fn passes(p: &Package) -> bool {
    p.licence.as_deref().is_some_and(|l| allowed(l) || elected(&p.name, l).is_some())
}

struct Package {
    name: String,
    version: String,
    licence: Option<String>,
    repository: String,
}

fn dependencies(root: &Path) -> anyhow::Result<Vec<Package>> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = std::process::Command::new(cargo)
        .args(["metadata", "--format-version", "1"])
        // Only what the shipped targets build: Windows, Linux, macOS and the web build.
        .args(SHIPPED_TARGETS.iter().flat_map(|t| ["--filter-platform", t]))
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .output()
        .context("could not run cargo metadata")?;
    if !output.status.success() {
        bail!("cargo metadata failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    let metadata: Value = serde_json::from_slice(&output.stdout).context("cargo metadata gave invalid JSON")?;
    let members: Vec<&str> = metadata["workspace_members"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let mut packages: BTreeMap<(String, String), Package> = BTreeMap::new();
    for package in metadata["packages"].as_array().into_iter().flatten() {
        let id = package["id"].as_str().unwrap_or_default();
        if members.contains(&id) {
            continue;
        }
        let name = package["name"].as_str().unwrap_or_default().to_owned();
        let version = package["version"].as_str().unwrap_or_default().to_owned();
        let licence = package["license"].as_str().map(str::to_owned);
        let repository = package["repository"].as_str().unwrap_or_default().to_owned();
        packages.insert((name.clone(), version.clone()), Package { name, version, licence, repository });
    }
    Ok(packages.into_values().collect())
}

fn notice(packages: &[Package]) -> String {
    let mut text = String::from(
        "Bayleaf Engine\nCopyright the Bayleaf Engine contributors. Licensed under MIT OR Apache-2.0.\n\n\
         This software uses the following third-party Rust crates, unmodified, under the licences shown.\n\
         Each crate's licence text and its own NOTICE, where it has one, come with its source.\n\
         Written by `cargo xtask licences --write`; do not edit by hand.\n\n",
    );
    for p in packages {
        let licence = match p.licence.as_deref() {
            Some(l) => match elected(&p.name, l) {
                Some(chosen) => format!("{l}, used under {chosen}"),
                None => l.to_owned(),
            },
            None => "(no licence field; see the crate)".to_owned(),
        };
        let source = if p.repository.is_empty() { String::new() } else { format!(" <{}>", p.repository) };
        text.push_str(&format!("{} {}: {licence}{source}\n", p.name, p.version));
    }
    text
}

pub fn check(root: &Path, write: bool) -> anyhow::Result<()> {
    let packages = dependencies(root)?;
    let refused: Vec<String> =
        packages.iter().filter(|p| !passes(p)).map(|p| format!("{} {}: {}", p.name, p.version, p.licence.as_deref().unwrap_or("no licence field"))).collect();
    let path = root.join("NOTICE");
    let wanted = notice(&packages);
    if write {
        // NOTICE lists every shipped dependency, also one whose licence still needs a decision.
        std::fs::write(&path, &wanted).with_context(|| format!("writing {}", path.display()))?;
        println!("licences: {} dependencies, NOTICE written", packages.len());
    }
    if !refused.is_empty() {
        bail!("licences not on the craft-contrib list (standards/licences.md); remove the dependency or get an RFC first:\n  {}", refused.join("\n  "));
    }
    let current = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
    if current != wanted {
        bail!("NOTICE does not list the current dependencies; run: cargo xtask licences --write");
    }
    println!("licences: ok ({} dependencies)", packages.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{allowed, elected, identifiers};

    #[test]
    fn expressions_are_allowed_only_when_every_identifier_is() {
        assert!(allowed("MIT OR Apache-2.0"));
        assert!(allowed("(MIT OR Apache-2.0) AND Unicode-3.0"));
        assert!(allowed("Apache-2.0 WITH LLVM-exception"));
        assert!(allowed("MIT/Apache-2.0"));
        assert!(!allowed("MIT OR GPL-3.0"), "every identifier must be on the list, even in an OR");
        assert!(!allowed("LGPL-2.1"));
        assert!(!allowed(""));
        assert_eq!(identifiers("MPL-2.0+"), ["MPL-2.0"]);
    }

    #[test]
    fn a_recorded_choice_passes_only_for_its_crate_and_the_licence_it_declared() {
        assert_eq!(elected("self_cell", "Apache-2.0 OR GPL-2.0-only"), Some("Apache-2.0"));
        assert_eq!(elected("other", "Apache-2.0 OR GPL-2.0-only"), None, "the choice is per crate");
        assert_eq!(elected("self_cell", "GPL-2.0-only"), None, "a changed licence needs a new decision");
    }
}
