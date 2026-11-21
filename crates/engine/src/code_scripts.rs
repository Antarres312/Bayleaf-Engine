//! Scripts written in Rust: a small crate per script, made from a template, built with cargo for
//! `wasm32-unknown-unknown` into a `.wasm` module beside the crate's folder.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::confine::Access;
use crate::{CommandError, CommandSpec, Mutates, Prompt, Session, always, parse};

/// The target scripts are built for.
pub const SCRIPT_TARGET: &str = "wasm32-unknown-unknown";

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "script.new",
            label: "New Script",
            menu: "",
            shortcut: "",
            params: "{folder, name}: makes a Rust script crate `folder/<name>/` from a template; returns {path, module}",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: always,
            run: script_new,
        },
        CommandSpec {
            id: "script.build",
            label: "Build Script",
            menu: "",
            shortcut: "",
            params: "{path}: a script crate folder; builds it to WebAssembly next to the folder; returns {module, log}",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: |s| if s.confinement.is_some() { Err(BUILD_REFUSED.to_owned()) } else { Ok(()) },
            run: script_build,
        },
    ]
}

/// Why agents may not build: cargo runs the crate's build scripts, which could reach any file.
const BUILD_REFUSED: &str = "building runs the crate's own build code, so agents may not start it; build from the editor";

/// A crate name from a script name: lower case words joined by `_`.
pub fn crate_name(name: &str) -> Option<String> {
    let words: Vec<String> = name.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_ascii_lowercase).collect();
    let joined = words.join("_");
    (!joined.is_empty() && joined.len() <= 64 && !joined.starts_with(|c: char| c.is_ascii_digit())).then_some(joined)
}

fn type_name(crate_name: &str) -> String {
    crate_name.split('_').map(|w| w[..1].to_ascii_uppercase() + &w[1..]).collect()
}

/// The script SDK this engine was built with.
fn sdk_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("BAYLEAF_SDK_DIR").map(PathBuf::from).filter(|d| d.join("Cargo.toml").is_file()) {
        return Some(dir);
    }
    // Beside this crate in the source tree (no canonical form: Cargo does not take `\?` paths).
    let built_with = Path::new(env!("CARGO_MANIFEST_DIR")).parent()?.join("script-sdk");
    built_with.join("Cargo.toml").is_file().then_some(built_with)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewParams {
    folder: PathBuf,
    name: String,
}

pub(crate) fn script_new(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: NewParams = parse(params)?;
    let name = crate_name(&p.name).ok_or_else(|| {
        CommandError::BadParams("a script name needs letters or digits, may not start with a digit, and has at most 64 characters".to_owned())
    })?;
    let folder = session.allowed(&session.script_dir().join(&p.folder), Access::Write)?;
    let dir = folder.join(&name);
    if dir.exists() || folder.join(format!("{name}.wasm")).exists() {
        return Err(CommandError::Failed(format!("{} already exists; pick another name", dir.display())));
    }
    let sdk = sdk_dir().ok_or_else(|| CommandError::Failed("the script SDK (crates/script-sdk) is not found; set BAYLEAF_SDK_DIR to its folder".to_owned()))?;
    let sdk = sdk.to_string_lossy().replace('\\', "/");
    // In a project the script joins the project's workspace (one Cargo.lock, one build folder,
    // dependencies built once); elsewhere, or when the workspace may not be written, it stands alone.
    let workspace = session
        .project_dir()
        .map(|p| p.join(WORKSPACE_MANIFEST))
        .filter(|m| session.allowed(m, Access::Write).is_ok() && dir.starts_with(m.parent().unwrap_or(m)));
    let package = format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[lib]\ncrate-type = [\"cdylib\"]\n\n");
    let manifest = match &workspace {
        Some(_) => format!(
            "{package}[dependencies]\n# Versions and build settings are the project's: see the Cargo.toml in the project folder.\nbayleaf-script-sdk = {{ workspace = true }}\n"
        ),
        None => format!(
            "{package}[dependencies]\nbayleaf-script-sdk = {{ path = \"{sdk}\" }}\n\n# A workspace of its own, so it never joins a workspace in a folder above it.\n[workspace]\n\n{RELEASE_PROFILE}"
        ),
    };
    let ty = type_name(&name);
    let title = p.name.trim();
    let source = format!(
        "//! {title}: a Bayleaf Engine script in Rust.\n//!\n//! Build it from the Project panel (right-click its folder, Build Script); then add the built\n//! `{name}.wasm` to an object in the Inspector. The SDK is documented in its own crate.\n\nuse bayleaf_script_sdk::{{Script, Transform, property, script}};\n\n#[derive(Default)]\nstruct {ty};\n\nimpl Script for {ty} {{\n    /// Numbers the Inspector shows for each object, with their defaults.\n    const PROPERTIES: &'static [(&'static str, f32)] = &[(\"speed\", 1.0)];\n\n    fn start(&mut self) {{}}\n\n    fn update(&mut self, dt: f32) {{\n        // An example to replace: turn around Y by `speed` turns per second.\n        let speed = property(\"speed\").unwrap_or(1.0);\n        let mut transform = Transform::get();\n        transform.rotation[1] = (transform.rotation[1] + speed * 360.0 * dt) % 360.0;\n        transform.set();\n    }}\n}}\n\nscript!({ty});\n"
    );
    std::fs::create_dir_all(dir.join("src")).map_err(|e| CommandError::Failed(format!("cannot make {}: {e}", dir.display())))?;
    std::fs::write(dir.join("Cargo.toml"), manifest).map_err(|e| CommandError::Failed(e.to_string()))?;
    std::fs::write(dir.join("src").join("lib.rs"), source).map_err(|e| CommandError::Failed(e.to_string()))?;
    // The crate is complete before the workspace lists it; a workspace that cannot take it leaves nothing behind.
    if let Some(workspace) = &workspace
        && let Err(error) = join_workspace(workspace, &dir, &sdk)
    {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(CommandError::Failed(error));
    }
    Ok(json!({ "path": dir, "module": folder.join(format!("{name}.wasm")), "workspace": workspace }))
}

/// The project's Rust script workspace, in the project folder.
pub const WORKSPACE_MANIFEST: &str = "Cargo.toml";

/// Small, fast WebAssembly modules for scripts.
const RELEASE_PROFILE: &str = "[profile.release]\nopt-level = \"s\"\nlto = true\ncodegen-units = 1\npanic = \"abort\"\nstrip = true\n";

/// Adds the crate at `member` to the workspace `manifest`, making the workspace when the project
/// has none. An existing manifest keeps its own entries, formatting and comments; one that is not
/// a workspace is refused rather than changed.
pub fn join_workspace(manifest: &Path, member: &Path, sdk: &str) -> Result<(), String> {
    let root = manifest.parent().ok_or("the workspace manifest has no folder")?;
    let relative = member.strip_prefix(root).map_err(|_| format!("{} is outside the project folder", member.display()))?;
    let relative = relative.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
    let text = match std::fs::read_to_string(manifest) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => format!(
            "# The project's Rust scripts: one crate each, built together, so they share one Cargo.lock,\n# one build folder and their dependencies. The editor adds each new script to `members`.\n[workspace]\nresolver = \"3\"\nmembers = []\n\n[workspace.dependencies]\nbayleaf-script-sdk = {{ path = \"{sdk}\" }}\n\n{RELEASE_PROFILE}"
        ),
        Err(e) => return Err(format!("cannot read {}: {e}", manifest.display())),
    };
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{} does not read as TOML: {e}", manifest.display()))?;
    let workspace = doc
        .get_mut("workspace")
        .and_then(toml_edit::Item::as_table_mut)
        .ok_or_else(|| format!("{} is not a Cargo workspace, so the script cannot join it", manifest.display()))?;
    let members = workspace.entry("members").or_insert_with(|| toml_edit::value(toml_edit::Array::new()));
    let members = members.as_array_mut().ok_or("the workspace's members are not a list")?;
    if !members.iter().any(|m| m.as_str() == Some(relative.as_str())) {
        members.push(relative.as_str());
    }
    let dependencies =
        workspace.entry("dependencies").or_insert_with(toml_edit::table).as_table_like_mut().ok_or("the workspace's dependencies are not a table")?;
    if !dependencies.contains_key("bayleaf-script-sdk") {
        let mut sdk_table = toml_edit::InlineTable::new();
        sdk_table.insert("path", sdk.into());
        dependencies.insert("bayleaf-script-sdk", toml_edit::value(sdk_table));
    }
    std::fs::write(manifest, doc.to_string()).map_err(|e| format!("cannot write {}: {e}", manifest.display()))
}

/// What a build made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Built {
    /// The `.wasm` module, beside the crate's folder.
    pub module: PathBuf,
    /// Cargo's messages.
    pub log: String,
}

/// Builds the script crate in `dir` for WebAssembly, with build output under `target_dir`, and
/// copies the module beside the folder. Runs cargo; takes seconds to minutes.
pub fn build_script(dir: &Path, target_dir: &Path) -> Result<Built, String> {
    let manifest = dir.join("Cargo.toml");
    if !manifest.is_file() {
        return Err(format!("{} is not a script crate (it has no Cargo.toml)", dir.display()));
    }
    let name = dir.file_name().and_then(|n| n.to_str()).and_then(crate_name).ok_or("the script folder's name is not a crate name")?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["build", "--release", "--target", SCRIPT_TARGET, "--manifest-path"])
        .arg(&manifest)
        .env("CARGO_TARGET_DIR", target_dir)
        .output()
        .map_err(|e| format!("building scripts needs Rust (cargo): {e}"))?;
    let log = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        let hint = if log.contains("target may not be installed") || log.contains("can't find crate for `core`") {
            format!("\nAdd the WebAssembly target once with: rustup target add {SCRIPT_TARGET}")
        } else {
            String::new()
        };
        return Err(format!("the script did not build:\n{}{hint}", tail(&log, 40)));
    }
    let built = target_dir.join(SCRIPT_TARGET).join("release").join(format!("{name}.wasm"));
    let module = dir.parent().unwrap_or(dir).join(format!("{name}.wasm"));
    std::fs::copy(&built, &module).map_err(|e| format!("cannot copy {} to {}: {e}", built.display(), module.display()))?;
    Ok(Built { module, log })
}

/// The last `lines` lines of `text`.
fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// Where a project's script builds go.
pub fn target_dir_for(session: &Session) -> PathBuf {
    // A hidden folder, so the Project panel does not list thousands of build files.
    session.script_dir().join(".build").join("scripts")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildParams {
    path: PathBuf,
}

fn script_build(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: BuildParams = parse(params)?;
    let dir = session.script_dir().join(&p.path);
    let dir = if dir.file_name().is_some_and(|n| n == "Cargo.toml") { dir.parent().map(Path::to_path_buf).unwrap_or(dir) } else { dir };
    let built = build_script(&dir, &target_dir_for(session)).map_err(CommandError::Failed)?;
    Ok(json!({ "module": built.module, "log": tail(&built.log, 20) }))
}

#[cfg(test)]
mod tests {
    use super::crate_name;

    #[test]
    fn script_names_become_crate_names() {
        assert_eq!(crate_name("Player Controller").as_deref(), Some("player_controller"));
        assert_eq!(crate_name("door-opener 2").as_deref(), Some("door_opener_2"));
        assert_eq!(crate_name("  "), None);
        assert_eq!(crate_name("2fast"), None, "crate names start with a letter");
        assert_eq!(crate_name("../evil").as_deref(), Some("evil"), "no path parts survive");
    }
}
