//! Native mode for exported games: the game's Rust scripts are built into its own player, so they
//! run as machine code with no sandbox. The export writes a small Cargo workspace (a player crate
//! that calls `bayleaf_player::run` with the scripts, and copies of the project's script crates)
//! and builds it with the engine's source, so this needs that source and Rust (cargo).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use toml_edit::{DocumentMut, Item, Table, value};

/// The engine's source folder (with `apps/bayleaf-player`): `BAYLEAF_SOURCE_DIR`, else the one this
/// program was built from.
pub fn engine_source() -> Option<PathBuf> {
    let has_player = |dir: &Path| dir.join("apps").join("bayleaf-player").join("Cargo.toml").is_file();
    if let Some(dir) = std::env::var_os("BAYLEAF_SOURCE_DIR").map(PathBuf::from).filter(|d| has_player(d)) {
        return Some(dir);
    }
    // No canonical form: Cargo does not take `\\?\` paths.
    let built_with = Path::new(env!("CARGO_MANIFEST_DIR")).parent()?.parent()?.to_path_buf();
    has_player(&built_with).then_some(built_with)
}

/// Whether Rust (cargo) runs here; asked once.
pub fn cargo_found() -> bool {
    static FOUND: OnceLock<bool> = OnceLock::new();
    *FOUND.get_or_init(|| Command::new(cargo()).arg("--version").output().is_ok_and(|o| o.status.success()))
}

fn cargo() -> std::ffi::OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}

/// A Rust script built into the game: the module it stands for, relative to the project
/// (`scripts/mover.wasm`), and its crate's folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeSource {
    pub module: String,
    pub crate_dir: PathBuf,
}

/// The Rust scripts among the game's `files` (relative to `project`): each `.wasm` module with its
/// crate beside it (`scripts/mover.wasm` and `scripts/mover/Cargo.toml`).
pub fn sources(project: &Path, files: &[PathBuf]) -> Vec<NativeSource> {
    files
        .iter()
        .filter(|f| f.extension().is_some_and(|e| e == "wasm"))
        .filter_map(|f| {
            let crate_dir = project.join(f).with_extension("");
            crate_dir.join("Cargo.toml").is_file().then(|| NativeSource { module: f.to_string_lossy().replace('\\', "/"), crate_dir })
        })
        .collect()
}

/// A native player ready to build: its Cargo workspace, and where the built program goes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeBuild {
    /// The generated workspace (its build output goes in `target` inside it, kept for next time).
    pub folder: PathBuf,
    /// The game's program, written when the build succeeds.
    pub program: PathBuf,
}

/// The player crate's binary.
const BINARY: &str = "game";

/// Writes the workspace that builds `program` with `scripts` built in, in `folder` (made if
/// needed; its earlier build output is kept, so building again is quicker).
pub fn prepare(engine: &Path, project: &Path, scripts: &[NativeSource], folder: &Path, program: &Path) -> Result<NativeBuild, String> {
    let fail = |what: &Path, e: std::io::Error| format!("cannot write {}: {e}", what.display());
    std::fs::create_dir_all(folder.join("player").join("src")).map_err(|e| fail(folder, e))?;
    let copies = folder.join("scripts");
    if copies.exists() {
        std::fs::remove_dir_all(&copies).map_err(|e| fail(&copies, e))?;
    }
    let mut names = Vec::new();
    for script in scripts {
        let name = package_name(&script.crate_dir)?;
        if names.contains(&name) {
            return Err(format!("two Rust scripts are both named {name}; rename one of their crates"));
        }
        copy_crate(&script.crate_dir, &copies.join(&name))?;
        names.push(name);
    }
    let root = folder.join("Cargo.toml");
    std::fs::write(&root, workspace_manifest(engine, project, &names)?).map_err(|e| fail(&root, e))?;
    // The engine's versions of every dependency, so the player builds as the engine does.
    let lock = engine.join("Cargo.lock");
    if lock.is_file() {
        std::fs::copy(&lock, folder.join("Cargo.lock")).map_err(|e| fail(&folder.join("Cargo.lock"), e))?;
    }
    let player = folder.join("player").join("Cargo.toml");
    std::fs::write(&player, player_manifest(engine, &names)).map_err(|e| fail(&player, e))?;
    let main = folder.join("player").join("src").join("main.rs");
    std::fs::write(&main, player_main(scripts, &names)).map_err(|e| fail(&main, e))?;
    Ok(NativeBuild { folder: folder.to_path_buf(), program: program.to_path_buf() })
}

/// How far a native build has got: the parts of the build done and in all (0 until cargo has
/// counted them), and what it works on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub doing: String,
}

/// Lines of cargo's output kept to say why a build failed.
const KEPT_LINES: usize = 40;

/// Builds the player and writes it as the game's program. Runs cargo: minutes the first time.
pub fn build(plan: &NativeBuild) -> Result<(), String> {
    build_reporting(plan, &mut |_| {})
}

/// [`build`], telling `report` how far it has got as cargo goes.
pub fn build_reporting(plan: &NativeBuild, report: &mut dyn FnMut(&Progress)) -> Result<(), String> {
    use std::io::Read;
    // Half the processors, so the computer stays usable meanwhile.
    let jobs = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).max(1));
    let mut child = Command::new(cargo())
        .args(["build", "--release", "--manifest-path"])
        .arg(plan.folder.join("Cargo.toml"))
        .args(["-p", "bayleaf-native-player", "-j", &jobs.to_string()])
        .env("CARGO_TARGET_DIR", plan.folder.join("target"))
        // Cargo's progress line also when its output is not a terminal, wide enough to read.
        .env("CARGO_TERM_PROGRESS_WHEN", "always")
        .env("CARGO_TERM_PROGRESS_WIDTH", "200")
        .env("CARGO_TERM_COLOR", "never")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("building the native player needs Rust (cargo): {e}"))?;
    let mut progress = Progress::default();
    let mut kept: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    if let Some(mut stderr) = child.stderr.take() {
        let (mut line, mut chunk) = (Vec::new(), [0u8; 4096]);
        while let Ok(read) = stderr.read(&mut chunk) {
            if read == 0 {
                break;
            }
            for &byte in &chunk[..read] {
                // The progress line is redrawn after a carriage return; other lines end with a newline.
                if byte != b'\r' && byte != b'\n' {
                    line.push(byte);
                    continue;
                }
                let text = String::from_utf8_lossy(&line).into_owned();
                line.clear();
                if let Some((done, total, doing)) = progress_line(&text) {
                    progress = Progress { done, total, doing };
                    report(&progress);
                } else if !text.trim().is_empty() {
                    if let Some(name) = text.trim_start().strip_prefix("Compiling ") {
                        progress.doing = name.to_owned();
                        report(&progress);
                    }
                    kept.push_back(text);
                    if kept.len() > KEPT_LINES {
                        kept.pop_front();
                    }
                }
            }
        }
    }
    let status = child.wait().map_err(|e| format!("the native player's build did not finish: {e}"))?;
    if !status.success() {
        return Err(format!("the native player did not build:\n{}", Vec::from(kept).join("\n")));
    }
    let built = plan.folder.join("target").join("release").join(format!("{BINARY}{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(&built, &plan.program).map_err(|e| format!("cannot write {}: {e}", plan.program.display()))?;
    Ok(())
}

/// Reads cargo's progress line, `Building [=====>    ] 67/290: serde, syn(build)`: the parts done,
/// in all, and what it works on.
pub fn progress_line(line: &str) -> Option<(usize, usize, String)> {
    let rest = line.trim_start().strip_prefix("Building [")?;
    let (_, rest) = rest.split_once("] ")?;
    let (count, doing) = rest.split_once(':').map_or((rest, ""), |(count, doing)| (count, doing.trim()));
    let (done, total) = count.trim().split_once('/')?;
    Some((done.parse().ok()?, total.parse().ok()?, doing.to_owned()))
}

fn package_name(crate_dir: &Path) -> Result<String, String> {
    let manifest = crate_dir.join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest).map_err(|e| format!("cannot read {}: {e}", manifest.display()))?;
    let doc: DocumentMut = text.parse().map_err(|e| format!("{} does not read: {e}", manifest.display()))?;
    doc.get("package").and_then(|p| p.get("name")).and_then(Item::as_str).map(str::to_owned).ok_or_else(|| format!("{} names no package", manifest.display()))
}

/// Copies a script crate (its manifest and sources, not its build output) to `to`, with a manifest
/// fit to join the native player's workspace: a plain library, the workspace's SDK, no workspace or
/// profiles of its own, and paths that still lead where they did.
fn copy_crate(from: &Path, to: &Path) -> Result<(), String> {
    copy_tree(from, to)?;
    let manifest = from.join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest).map_err(|e| format!("cannot read {}: {e}", manifest.display()))?;
    let mut doc: DocumentMut = text.parse().map_err(|e| format!("{} does not read: {e}", manifest.display()))?;
    doc.remove("workspace");
    doc.remove("profile");
    if let Some(lib) = doc.get_mut("lib").and_then(Item::as_table_like_mut) {
        lib.remove("crate-type");
    }
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let Some(deps) = doc.get_mut(section).and_then(Item::as_table_like_mut) else { continue };
        for (name, dep) in deps.iter_mut() {
            if name.get() == "bayleaf-script-sdk" {
                let mut inherited = toml_edit::InlineTable::new();
                inherited.insert("workspace", true.into());
                *dep = value(inherited);
            } else if let Some(table) = dep.as_table_like_mut()
                && let Some(path) = table.get("path").and_then(Item::as_str)
                && Path::new(path).is_relative()
            {
                let absolute = from.join(path).to_string_lossy().replace('\\', "/");
                table.insert("path", value(absolute));
            }
        }
    }
    let target = to.join("Cargo.toml");
    std::fs::write(&target, doc.to_string()).map_err(|e| format!("cannot write {}: {e}", target.display()))
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("cannot make {}: {e}", to.display()))?;
    for entry in std::fs::read_dir(from).map_err(|e| format!("cannot read {}: {e}", from.display()))?.flatten() {
        let name = entry.file_name();
        let skip = name.to_string_lossy().starts_with('.') || name == "target";
        let path = entry.path();
        if skip {
            continue;
        }
        if path.is_dir() {
            copy_tree(&path, &to.join(&name))?;
        } else {
            std::fs::copy(&path, to.join(&name)).map_err(|e| format!("cannot copy {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// The workspace: the player and the scripts (`scripts`, their crate names); the project's shared script settings; the SDK from
/// the engine; the engine's patches (which Cargo takes only from the workspace being built) and
/// release profile.
fn workspace_manifest(engine: &Path, project: &Path, scripts: &[String]) -> Result<String, String> {
    let read = |path: &Path| -> Result<DocumentMut, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        text.parse().map_err(|e| format!("{} does not read: {e}", path.display()))
    };
    let engine_doc = read(&engine.join("Cargo.toml"))?;
    let project_manifest = project.join(crate::code_scripts::WORKSPACE_MANIFEST);
    let project_doc = if project_manifest.is_file() { Some(read(&project_manifest)?) } else { None };
    let shared = |key: &str| project_doc.as_ref().and_then(|d| d.get("workspace")).and_then(|w| w.get(key)).and_then(Item::as_table).cloned();

    let mut doc = DocumentMut::new();
    doc.decor_mut().set_prefix("# Builds a game's player with its Rust scripts in (native mode). Written by the export; do not edit.\n");
    let mut workspace = Table::new();
    workspace.insert("resolver", value("3"));
    let mut members = toml_edit::Array::new();
    members.push("player");
    // Each script by name: a pattern for a folder that is not there (a game with no Rust scripts)
    // stops Cargo.
    for name in scripts {
        members.push(format!("scripts/{name}"));
    }
    workspace.insert("members", value(members));
    if let Some(package) = shared("package") {
        workspace.insert("package", Item::Table(package));
    }
    let mut dependencies = shared("dependencies").unwrap_or_default();
    let mut sdk = toml_edit::InlineTable::new();
    sdk.insert("path", engine.join("crates").join("script-sdk").to_string_lossy().replace('\\', "/").into());
    dependencies.insert("bayleaf-script-sdk", value(sdk));
    workspace.insert("dependencies", Item::Table(dependencies));
    doc.insert("workspace", Item::Table(workspace));
    if let Some(patch) = engine_doc.get("patch") {
        doc.insert("patch", patch.clone());
    }
    if let Some(release) = engine_doc.get("profile").and_then(|p| p.get("release")) {
        let mut profile = Table::new();
        profile.set_implicit(true);
        profile.insert("release", release.clone());
        doc.insert("profile", Item::Table(profile));
    }
    Ok(doc.to_string())
}

fn player_manifest(engine: &Path, names: &[String]) -> String {
    let path = |p: PathBuf| p.to_string_lossy().replace('\\', "/");
    let mut text = format!(
        "[package]\nname = \"bayleaf-native-player\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[[bin]]\nname = \"{BINARY}\"\npath = \"src/main.rs\"\n\n[dependencies]\nbayleaf-player = {{ path = \"{}\" }}\n",
        path(engine.join("apps").join("bayleaf-player"))
    );
    for (i, name) in names.iter().enumerate() {
        text.push_str(&format!("script_{i} = {{ path = \"../scripts/{name}\", package = \"{name}\" }}\n"));
    }
    text
}

fn player_main(scripts: &[NativeSource], names: &[String]) -> String {
    let entries: String = scripts
        .iter()
        .zip(names)
        .enumerate()
        .map(|(i, (script, name))| format!("        // {name}\n        (PathBuf::from({:?}), script_{i}::bayleaf_native()),\n", script.module))
        .collect();
    format!(
        "//! The game's player with its Rust scripts built in (native mode). Written by the export; do not edit.\n\nuse std::path::PathBuf;\n\nfn main() -> std::process::ExitCode {{\n    let natives = vec![\n{entries}    ];\n    match bayleaf_player::run(natives) {{\n        Ok(()) => std::process::ExitCode::SUCCESS,\n        Err(error) => {{\n            eprintln!(\"{{error:#}}\");\n            std::process::ExitCode::FAILURE\n        }}\n    }}\n}}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::progress_line;

    #[test]
    fn cargo_progress_lines_give_the_parts_done_in_all_and_what_is_built() {
        let (done, total, doing) = progress_line("    Building [>                           ] 1/16: proc-macro2(build.rs), unicode-ident   ").unwrap();
        assert_eq!((done, total), (1, 16));
        assert_eq!(doing, "proc-macro2(build.rs), unicode-ident");
        assert_eq!(progress_line("    Building [=====>      ] 290/290").map(|p| (p.0, p.1)), Some((290, 290)), "the last line names nothing");
        for other in ["   Compiling serde v1.0.0", "    Finished `release` profile", "Building [oops"] {
            assert!(progress_line(other).is_none(), "{other}");
        }
    }
}
