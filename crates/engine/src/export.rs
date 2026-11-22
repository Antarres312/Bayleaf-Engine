//! Game export (`project.export`): the open project as a game people run without the editor. The
//! folder holds the player program under the game's name and, beside it, a `game` folder with the
//! project's scenes, assets and scripts. Visual scripts are compiled now, so the game starts
//! without compiling them; Rust script sources, build output and saved values stay behind.
//!
//! Today a game is exported for the system the editor runs on, with the player program as it was
//! built (with every module the apps turn on); other systems and leaving unused modules out are
//! still to come, and the command says so.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::confine::Access;
use crate::{CommandError, CommandSpec, Mutates, Prompt, Session, not_playing, parse};

/// The folder beside the player program that holds the game.
pub const GAME_FOLDER: &str = "game";

/// File extensions a game uses: projects, scenes, prefabs, materials, scripts, input maps, models,
/// images and sounds. Other files stay behind.
const GAME_EXTENSIONS: &[&str] = &[
    crate::project_cmds::PROJECT_EXTENSION,
    bayleaf_scene::doc::EXTENSION,
    crate::prefab_cmds::PREFAB_EXTENSION,
    "bmaterial",
    bayleaf_script::VISUAL_EXTENSION,
    "wasm",
    crate::input_cmds::INPUT_EXTENSION,
    "glb",
    "gltf",
    "bin",
];

/// Folders whose files a game does not need: build output, saved values and version control.
const SKIPPED_FOLDERS: &[&str] = &["target", "saves", "node_modules"];

/// Most files one export copies.
const MAX_FILES: usize = 100_000;

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "project.export",
        label: "Export Game…",
        menu: "File/Export Game…",
        shortcut: "",
        params: "{folder, player?}: writes the open project as a game into a new folder named after it inside folder: the player program under the game's name, and its scenes, assets and scripts in a game folder beside it (visual scripts compiled; Rust sources, build output and saved values left out). player is the player program to use (default: bayleaf-player beside this program). Exports for this computer's system only; returns {path, program, files, compiled, skipped}",
        prompt: Prompt::Folder,
        mutates: Mutates::No,
        enabled: |s| {
            not_playing(s)?;
            if s.project_dir().is_some() { Ok(()) } else { Err("open a project first".to_owned()) }
        },
        run: export_command,
    }]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportParams {
    folder: PathBuf,
    #[serde(default)]
    player: Option<PathBuf>,
}

/// The game's name as a file name: letters, digits, spaces, `-` and `_`; anything else becomes `_`.
fn file_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { '_' }).collect();
    let cleaned = cleaned.trim().trim_matches('.').to_owned();
    if cleaned.is_empty() { "Game".to_owned() } else { cleaned }
}

/// Whether a folder of the project is one a game leaves behind: hidden, build output, saved values,
/// or a Rust script's crate (its built module sits beside it).
fn skipped_folder(dir: &Path) -> bool {
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    name.starts_with('.') || SKIPPED_FOLDERS.contains(&name.as_str()) || dir.join("Cargo.toml").is_file()
}

/// Whether the game uses a file of this extension.
fn game_file(path: &Path) -> bool {
    let extension = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    GAME_EXTENSIONS.contains(&extension.as_str())
        || bayleaf_scene::components::SOUND_EXTENSIONS.contains(&extension.as_str())
        || bayleaf_render::assets::IMAGE_EXTENSIONS.contains(&extension.as_str())
}

/// Every file under `dir` a game may use, and the ones it leaves behind, relative to `dir`.
fn gather(dir: &Path) -> Result<(Vec<PathBuf>, Vec<PathBuf>), CommandError> {
    let (mut used, mut left) = (Vec::new(), Vec::new());
    let mut pending = vec![dir.to_path_buf()];
    while let Some(folder) = pending.pop() {
        let entries = std::fs::read_dir(&folder).map_err(|e| CommandError::Failed(format!("cannot read {}: {e}", folder.display())))?;
        let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            let relative = path.strip_prefix(dir).map(Path::to_path_buf).unwrap_or_default();
            if path.is_dir() {
                if skipped_folder(&path) {
                    left.push(relative);
                } else {
                    pending.push(path);
                }
            } else if game_file(&path) {
                used.push(relative);
            } else {
                left.push(relative);
            }
            if used.len() > MAX_FILES {
                return Err(CommandError::Failed(format!("the project has more than {MAX_FILES} files to export")));
            }
        }
    }
    used.sort();
    left.sort();
    Ok((used, left))
}

fn export_command(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: ExportParams = parse(params)?;
    let project = session.project_dir().map(Path::to_path_buf).ok_or_else(|| CommandError::Failed("open a project first".to_owned()))?;
    let project = session.allowed(&project, Access::Read)?;
    let name = crate::project_cmds::project_file_in(&project)
        .and_then(|file| std::fs::read_to_string(file).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|v| v["name"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "Game".to_owned());
    let name = file_name(&name);

    // The player program: given, else the one built beside this program.
    let player = match p.player {
        Some(path) => session.allowed(&path, Access::Read)?,
        None => {
            let here = std::env::current_exe().map_err(|e| CommandError::Failed(format!("cannot find this program: {e}")))?;
            here.with_file_name(format!("bayleaf-player{}", std::env::consts::EXE_SUFFIX))
        }
    };
    if !player.is_file() {
        return Err(CommandError::Failed(format!(
            "the player program {} is not there; build it first (cargo build -p bayleaf-player), or give its path as player",
            player.display()
        )));
    }

    let out = session.allowed(&p.folder.join(&name), Access::Write)?;
    if out.starts_with(&project) {
        return Err(CommandError::Failed("export the game outside its project folder".to_owned()));
    }
    if out.exists() && std::fs::read_dir(&out).map(|mut d| d.next().is_some()).unwrap_or(true) {
        return Err(CommandError::Failed(format!("{} already exists and is not empty; pick another folder", out.display())));
    }

    let (files, left) = gather(&project)?;
    // Compile every visual script first, so a broken one stops the export before anything is written.
    let mut compiled = Vec::new();
    for relative in files.iter().filter(|f| f.extension().is_some_and(|e| e == bayleaf_script::VISUAL_EXTENSION)) {
        let bytes =
            bayleaf_script::module_bytes(&project.join(relative)).map_err(|e| CommandError::Failed(format!("{} does not compile: {e}", relative.display())))?;
        compiled.push((relative.clone(), bytes));
    }

    let game = out.join(GAME_FOLDER);
    let made = |dir: &Path| std::fs::create_dir_all(dir).map_err(|e| CommandError::Failed(format!("cannot make {}: {e}", dir.display())));
    made(&game)?;
    for relative in &files {
        let target = game.join(relative);
        if let Some(parent) = target.parent() {
            made(parent)?;
        }
        match compiled.iter().find(|(f, _)| f == relative) {
            Some((_, bytes)) => std::fs::write(&target, bytes),
            None => std::fs::copy(project.join(relative), &target).map(|_| ()),
        }
        .map_err(|e| CommandError::Failed(format!("cannot write {}: {e}", target.display())))?;
    }
    let program = out.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(&player, &program).map_err(|e| CommandError::Failed(format!("cannot write {}: {e}", program.display())))?;

    let shown = |list: &[PathBuf]| list.iter().map(|p| p.to_string_lossy().replace('\\', "/")).collect::<Vec<_>>();
    session.log(crate::Level::Info, format!("Exported {name} to {}: {} files, {} visual scripts compiled.", out.display(), files.len(), compiled.len()));
    Ok(json!({
        "path": out,
        "program": program,
        "files": files.len(),
        "compiled": shown(&compiled.iter().map(|(f, _)| f.clone()).collect::<Vec<_>>()),
        "skipped": shown(&left),
    }))
}

/// The project of a game exported beside `program`: the project file in its `game` folder.
pub fn packaged_game(program: &Path) -> Option<PathBuf> {
    crate::project_cmds::project_file_in(&program.parent()?.join(GAME_FOLDER))
}
