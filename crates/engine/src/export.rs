//! Game export (`project.export`): the open project as a game people run without the editor. The
//! folder holds the player program under the game's name and, beside it, a `game` folder with the
//! project's scenes, assets and scripts. Visual scripts are compiled now, so the game starts
//! without compiling them; Rust script sources, build output and saved values stay behind.
//!
//! `project.export.check` tells, without writing anything, whether the project can be exported for
//! a platform with its scripts run in a mode, and why not. Today a game is exported for the system
//! the editor runs on, with its scripts interpreted, and with the player program as it was built
//! (with every module the apps turn on); other systems, the other script modes and leaving unused
//! modules out are still to come, and the check says so.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
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
    bayleaf_scene::lighting::EXTENSION,
    bayleaf_scene::post::EXTENSION,
    "glb",
    "gltf",
    "bin",
    "obj",
    "mtl",
    bayleaf_render::assets::import::SETTINGS_EXTENSION,
    bayleaf_interface::LAYOUT_EXTENSION,
    bayleaf_interface::STYLE_EXTENSION,
];

/// Folders whose files a game does not need: build output, saved values and version control.
const SKIPPED_FOLDERS: &[&str] = &["target", "saves", "node_modules"];

/// Most files one export copies.
const MAX_FILES: usize = 100_000;

/// Export needs an open project, outside play mode.
fn exportable(session: &Session) -> Result<(), String> {
    not_playing(session)?;
    if session.project_dir().is_some() { Ok(()) } else { Err("open a project first".to_owned()) }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "project.export",
            label: "Export Game…",
            menu: "File/Export Game…",
            shortcut: "",
            params: "{folder, platform?, mode?, player?}: writes the open project as a game into a new folder named after it inside folder: the player program under the game's name, and its scenes, assets and scripts in a game folder beside it (visual scripts compiled; Rust sources, build output and saved values left out). platform is windows, linux, mac or web (default: this computer's); mode is how scripts run: interpreted, compiled or native (default: the project's choice for the platform); player is the program built on another system, or for the web the folder of the web player (default: the one cargo xtask web-player built). For the web the folder holds index.html, the web player and game.pack (the game's files in one file) instead. Refused, with the reasons, when project.export.check finds an error; returns {path, program, files, compiled, skipped}",
            prompt: Prompt::Dialog,
            mutates: Mutates::No,
            enabled: exportable,
            run: export_command,
        },
        CommandSpec {
            id: "project.export.check",
            label: "Check Export",
            menu: "",
            shortcut: "",
            params: "{platform?, mode?, player?}: whether the open project can be exported for platform with its scripts run in mode (defaults as for project.export), without writing anything; returns {platform, mode, ready, findings: [{severity: error|warning|note, message, file?}], scripts: [{path, kind, runs, note}], files}",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: exportable,
            run: check_command,
        },
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportParams {
    folder: PathBuf,
    #[serde(default)]
    platform: Option<Platform>,
    #[serde(default)]
    mode: Option<ScriptMode>,
    #[serde(default)]
    player: Option<PathBuf>,
    /// Native mode: wait for the game's player to build (the default), or return when the build
    /// is ready to run (`build` in the result), for a program that builds it in the background.
    #[serde(default = "yes")]
    wait: bool,
}

fn yes() -> bool {
    true
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

    // Nothing is written unless the check finds no error.
    let platform = p.platform.unwrap_or_else(Platform::current);
    let mode = p.mode.unwrap_or_else(|| chosen_mode(session, platform));
    let report = check(session, platform, mode, p.player.as_deref())?;
    if !report.ready {
        let errors: Vec<&str> = report.findings.iter().filter(|f| f.severity == Severity::Error).map(|f| f.message.as_str()).collect();
        return Err(CommandError::Failed(format!("the game cannot be exported yet: {}", errors.join(" "))));
    }
    let player = if mode == ScriptMode::Native || platform == Platform::Web { None } else { Some(player_program(session, p.player.as_deref())?) };

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

    if platform == Platform::Web {
        return export_web(
            session,
            &WebExport { out: &out, name: &name, project: &project, files: &files, left: &left, compiled: &compiled, mode, player: p.player.as_deref() },
        );
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
    record_mode(&game, platform, mode)?;
    let program = out.join(format!("{name}{}", if platform == Platform::Windows { ".exe" } else { "" }));
    // The plain player, or (native mode) one built with the game's Rust scripts in it.
    let mut pending = None;
    let natives = crate::native_export::sources(&project, &files);
    match &player {
        Some(player) => {
            std::fs::copy(player, &program).map_err(|e| CommandError::Failed(format!("cannot write {}: {e}", program.display())))?;
        }
        None => {
            let engine = crate::native_export::engine_source().ok_or_else(|| CommandError::Failed("the engine's source is not found".to_owned()))?;
            let folder = session.allowed(&project.join(".build").join("native").join(platform.code()), Access::Write)?;
            let plan = crate::native_export::prepare(&engine, &project, &natives, &folder, &program).map_err(CommandError::Failed)?;
            if p.wait {
                crate::native_export::build(&plan).map_err(CommandError::Failed)?;
            } else {
                pending = Some(plan);
            }
        }
    }

    if platform == Platform::Linux {
        linux_files(&out, &name, &project)?;
    }

    let shown = |list: &[PathBuf]| list.iter().map(|p| p.to_string_lossy().replace('\\', "/")).collect::<Vec<_>>();
    session.log(crate::Level::Info, format!("Exported {name} to {}: {} files, {} visual scripts compiled.", out.display(), files.len(), compiled.len()));
    Ok(json!({
        "path": out,
        "program": program,
        "files": files.len(),
        "compiled": shown(&compiled.iter().map(|(f, _)| f.clone()).collect::<Vec<_>>()),
        "skipped": shown(&left),
        "native": if mode == ScriptMode::Native { natives.iter().map(|s| s.module.clone()).collect::<Vec<_>>() } else { Vec::new() },
        "build": pending,
    }))
}

/// What a web export writes from.
struct WebExport<'a> {
    out: &'a Path,
    name: &'a str,
    project: &'a Path,
    files: &'a [PathBuf],
    left: &'a [PathBuf],
    compiled: &'a [(PathBuf, Vec<u8>)],
    mode: ScriptMode,
    player: Option<&'a Path>,
}

/// Writes a game for the web: `index.html`, the web player and the game's files in one pack.
fn export_web(session: &mut Session, web: &WebExport<'_>) -> Result<Value, CommandError> {
    let failed = |what: &Path, e: std::io::Error| CommandError::Failed(format!("cannot write {}: {e}", what.display()));
    let folder = crate::web_export::player_folder(web.player).ok_or_else(|| CommandError::Failed(WEB_PLAYER_MISSING.to_owned()))?;
    let folder = session.allowed(&folder, Access::Read)?;
    let mut packed = Vec::with_capacity(web.files.len());
    for relative in web.files {
        let full = web.project.join(relative);
        let mut bytes = match web.compiled.iter().find(|(f, _)| f == relative) {
            Some((_, bytes)) => bytes.clone(),
            None => std::fs::read(&full).map_err(|e| CommandError::Failed(format!("cannot read {}: {e}", full.display())))?,
        };
        // The project file says how the game's scripts run on the web.
        if relative.extension().is_some_and(|e| e == crate::project_cmds::PROJECT_EXTENSION) && relative.parent() == Some(Path::new("")) {
            let text = String::from_utf8(bytes).map_err(|e| CommandError::Failed(format!("{} is not text: {e}", full.display())))?;
            bytes = with_mode(&text, Platform::Web, web.mode).map_err(|e| CommandError::Failed(format!("{}: {e}", full.display())))?.into_bytes();
        }
        packed.push((relative.to_string_lossy().replace('\\', "/"), bytes));
    }
    std::fs::create_dir_all(web.out).map_err(|e| failed(web.out, e))?;
    let page = web.out.join("index.html");
    std::fs::write(&page, crate::web_export::index_html(web.name)).map_err(|e| failed(&page, e))?;
    // The player with what wasm-bindgen wrote beside it (its `snippets` folder).
    crate::web_export::copy_player(&folder, web.out).map_err(CommandError::Failed)?;
    let pack = web.out.join(crate::web_export::PACK_FILE);
    std::fs::write(&pack, crate::web_export::pack(&packed)).map_err(|e| failed(&pack, e))?;

    let shown = |list: &[PathBuf]| list.iter().map(|p| p.to_string_lossy().replace('\\', "/")).collect::<Vec<_>>();
    session.log(
        crate::Level::Info,
        format!("Exported {} for the web to {}: {} files, {} visual scripts compiled.", web.name, web.out.display(), web.files.len(), web.compiled.len()),
    );
    Ok(json!({
        "path": web.out,
        "program": page,
        "files": web.files.len(),
        "compiled": shown(&web.compiled.iter().map(|(f, _)| f.clone()).collect::<Vec<_>>()),
        "skipped": shown(web.left),
        "native": Vec::<String>::new(),
        "build": Value::Null,
    }))
}

/// Why a web export cannot start without the web player.
const WEB_PLAYER_MISSING: &str = "Export for the web needs the web player: build it with cargo xtask web-player (it needs rustup target add wasm32-unknown-unknown and wasm-bindgen-cli), or choose the folder it was built into.";

/// A system a game can be exported for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Platform {
    Windows,
    Linux,
    Mac,
    Web,
}

impl Platform {
    pub const ALL: [Platform; 4] = [Platform::Windows, Platform::Linux, Platform::Mac, Platform::Web];

    /// English name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            Platform::Windows => "Windows",
            Platform::Linux => "Linux",
            Platform::Mac => "macOS",
            Platform::Web => "Web",
        }
    }

    /// The word commands and project files use: `windows`, `linux`, `mac` or `web`.
    pub fn code(self) -> &'static str {
        match self {
            Platform::Windows => "windows",
            Platform::Linux => "linux",
            Platform::Mac => "mac",
            Platform::Web => "web",
        }
    }

    /// The system this program runs on.
    pub fn current() -> Platform {
        if cfg!(target_os = "windows") {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::Mac
        } else if cfg!(target_arch = "wasm32") {
            Platform::Web
        } else {
            Platform::Linux
        }
    }
}

/// How an exported game runs its scripts (docs/vision.md, M8).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScriptMode {
    /// WebAssembly run by an interpreter: starts at once and reloads while playing; slowest.
    #[default]
    Interpreted,
    /// The same WebAssembly, still sandboxed, compiled to machine code: close to native speed.
    Compiled,
    /// Rust scripts built into the player, with no sandbox: fastest, for trusted code.
    Native,
}

impl ScriptMode {
    pub const ALL: [ScriptMode; 3] = [ScriptMode::Interpreted, ScriptMode::Compiled, ScriptMode::Native];

    /// English name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            ScriptMode::Interpreted => "Interpreted",
            ScriptMode::Compiled => "Compiled to machine code",
            ScriptMode::Native => "Native",
        }
    }

    /// What the mode gives and what it gives up.
    pub fn summary(self) -> &'static str {
        match self {
            ScriptMode::Interpreted => "Scripts start at once and stay sandboxed; the slowest of the modes.",
            ScriptMode::Compiled => "The same sandboxed scripts, compiled to machine code: close to native speed, with the same limits and faults.",
            ScriptMode::Native => {
                "Rust scripts built into the game with no sandbox: the fastest, for code you trust; a native script can do anything a program can, and a fault in it stops the game."
            }
        }
    }

    /// Whether this version of the engine can export games for `platform` in this mode: compiled
    /// scripts need a desktop system (the player built with them); native ones build the player on
    /// this computer, for its own system, with the engine's source and Rust (cargo).
    pub fn available_for(self, platform: Platform) -> bool {
        match self {
            ScriptMode::Interpreted => true,
            ScriptMode::Compiled => platform != Platform::Web,
            ScriptMode::Native => platform == Platform::current() && crate::native_export::engine_source().is_some() && crate::native_export::cargo_found(),
        }
    }

    /// How play mode runs scripts exported in this mode (native scripts are not built yet, so they
    /// stay compiled WebAssembly).
    pub fn runtime(self) -> bayleaf_script::Runtime {
        match self {
            ScriptMode::Interpreted => bayleaf_script::Runtime::Interpreted,
            ScriptMode::Compiled | ScriptMode::Native => bayleaf_script::Runtime::Compiled,
        }
    }
}

/// How serious a finding of the export check is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    /// The game cannot be exported this way.
    Error,
    /// The game exports, but something may not be as meant.
    Warning,
    /// Worth knowing.
    Note,
}

/// One thing the export check found.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Finding {
    pub severity: Severity,
    pub message: String,
    /// The project file it is about, relative to the project folder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

/// How one script of the project would run in the chosen mode.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ScriptPlan {
    /// The module, relative to the project folder.
    pub path: String,
    /// `visual`, `rust` (built from a Rust crate in the project) or `module` (WebAssembly only).
    pub kind: &'static str,
    /// The mode it would run in.
    pub runs: ScriptMode,
    pub note: String,
}

/// What `project.export.check` found for a platform and a mode.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Report {
    pub platform: Platform,
    pub mode: ScriptMode,
    /// No errors: the game can be exported this way.
    pub ready: bool,
    pub findings: Vec<Finding>,
    pub scripts: Vec<ScriptPlan>,
    /// Files the game would take along.
    pub files: usize,
}

/// The script mode Project Settings choose for `platform`.
pub fn chosen_mode(session: &Session, platform: Platform) -> ScriptMode {
    crate::project_cmds::settings_of(session).game.script_modes.get(&platform).copied().unwrap_or_default()
}

/// Writes into the exported game's project file the mode its scripts were exported in, so the
/// player runs them that way.
fn record_mode(game: &Path, platform: Platform, mode: ScriptMode) -> Result<(), CommandError> {
    let Some(file) = crate::project_cmds::project_file_in(game) else { return Ok(()) };
    let failed = |e: String| CommandError::Failed(format!("cannot write {}: {e}", file.display()));
    let text = std::fs::read_to_string(&file).map_err(|e| failed(e.to_string()))?;
    std::fs::write(&file, with_mode(&text, platform, mode).map_err(failed)?).map_err(|e| failed(e.to_string()))
}

/// A project file's text with the script mode for `platform` set to `mode`.
fn with_mode(text: &str, platform: Platform, mode: ScriptMode) -> Result<String, String> {
    let mut project: crate::project_cmds::ProjectFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
    project.settings.game.script_modes.insert(platform, mode);
    serde_json::to_string_pretty(&project).map_err(|e| e.to_string())
}

/// What exporting for `platform` from this computer needs: the web takes the web player, and another
/// desktop system a player built there, since this computer cannot build one.
fn system_findings(session: &Session, platform: Platform, player: Option<&Path>) -> Vec<(Severity, String)> {
    let mut findings = Vec::new();
    let mut push = |severity: Severity, message: String| findings.push((severity, message));
    if platform == Platform::Web {
        match crate::web_export::player_folder(player).map(|p| session.allowed(&p, Access::Read)) {
            None => push(Severity::Error, WEB_PLAYER_MISSING.to_owned()),
            Some(Err(e)) => push(Severity::Error, e.to_string()),
            Some(Ok(folder)) if !crate::web_export::has_player(&folder) => push(
                Severity::Error,
                format!(
                    "{} holds no web player ({} and {}); build it with cargo xtask web-player.",
                    folder.display(),
                    crate::web_export::PLAYER_SCRIPT,
                    crate::web_export::PLAYER_MODULE
                ),
            ),
            Some(Ok(folder)) => push(Severity::Note, format!("The game uses the web player in {}.", folder.display())),
        }
        push(
            Severity::Note,
            "The export is a folder with index.html: put it on a web server, or zip it with index.html at the top and upload it to a game site as a game played in the browser. A browser does not run it opened from a file on this computer; try it with cargo xtask serve <folder>.".to_owned(),
        );
        push(
            Severity::Note,
            "In a browser, scripts run interpreted, sound plays from the first click or key (as browsers require) with every sound loaded whole, and saved values are kept in the browser's storage for the site.".to_owned(),
        );
        push(Severity::Note, "In a browser, screen readers do not read the game's interface yet, as they do in the desktop player.".to_owned());
    } else if platform != Platform::current() {
        // This computer cannot build a player for another system: the export copies one built there.
        match player.map(|p| session.allowed(p, Access::Read)) {
            None => push(
                Severity::Error,
                format!(
                    "Export for {} needs a player built for {}: choose one (bayleaf-player, built on {} with cargo build --release -p bayleaf-player).",
                    platform.label(),
                    platform.label(),
                    platform.label()
                ),
            ),
            Some(Err(e)) => push(Severity::Error, e.to_string()),
            Some(Ok(path)) => match program_system(&path) {
                Some(system) if system == platform => {
                    push(Severity::Note, format!("The game uses the {} player you chose: {}.", platform.label(), path.display()));
                }
                Some(other) => {
                    push(Severity::Error, format!("{} is a program for {}, not for {}.", path.display(), other.label(), platform.label()));
                }
                None => push(Severity::Error, format!("{} is not a program for {}.", path.display(), platform.label())),
            },
        }
        if platform == Platform::Linux {
            push(Severity::Note,
                "On Linux, run install.sh in the game's folder once: it makes the game runnable and adds it to the applications menu with its icon. The game runs on Wayland and X11.".to_owned());
        }
    }
    findings
}

/// Which system a program is for, by its first bytes: an ELF file is Linux's, `MZ` Windows', and
/// a Mach-O file the Mac's.
pub fn program_system(path: &Path) -> Option<Platform> {
    let mut head = [0u8; 4];
    let mut file = std::fs::File::open(path).ok()?;
    std::io::Read::read_exact(&mut file, &mut head).ok()?;
    match head {
        [0x7F, b'E', b'L', b'F'] => Some(Platform::Linux),
        [b'M', b'Z', _, _] => Some(Platform::Windows),
        [0xCF, 0xFA, 0xED, 0xFE] | [0xCE, 0xFA, 0xED, 0xFE] | [0xCA, 0xFE, 0xBA, 0xBE] => Some(Platform::Mac),
        _ => None,
    }
}

/// Beside a Linux game: a desktop entry that starts it from its own folder, its icon, and
/// `install.sh`, which marks the program runnable (a copy made on Windows loses that) and adds
/// the game to the applications menu with full paths, as desktops want.
fn linux_files(out: &Path, name: &str, project: &Path) -> Result<(), CommandError> {
    let write = |file: &str, text: &str| {
        let path = out.join(file);
        std::fs::write(&path, text).map_err(|e| CommandError::Failed(format!("cannot write {}: {e}", path.display())))
    };
    let icon = crate::project_cmds::project_file_in(project)
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["settings"]["game"]["icon"].as_str().map(str::to_owned))
        .filter(|i| !i.is_empty())
        .map(|i| project.join(i))
        .filter(|p| p.is_file());
    if let Some(icon) = &icon {
        std::fs::copy(icon, out.join("icon.png")).map_err(|e| CommandError::Failed(format!("cannot copy the icon {}: {e}", icon.display())))?;
    }
    // Quotes and backslashes cannot reach the entry: the name has only letters, digits, spaces, - and _.
    let icon_line = if icon.is_some() { "Icon=icon.png\n" } else { "" };
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec=sh -c 'cd \"$(dirname \"%k\")\" && exec ./\"{name}\"'\n{icon_line}Terminal=false\nCategories=Game;\n"
    );
    write(&format!("{name}.desktop"), &entry)?;
    let id: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let icon_install = if icon.is_some() { "Icon=$here/icon.png" } else { "" };
    let install = format!(
        "#!/bin/sh\n# Makes {name} runnable and adds it to the applications menu.\nset -e\nhere=\"$(cd \"$(dirname \"$0\")\" && pwd)\"\nchmod +x \"$here/{name}\"\nmkdir -p \"$HOME/.local/share/applications\"\ncat > \"$HOME/.local/share/applications/{id}.desktop\" <<ENTRY\n[Desktop Entry]\nType=Application\nName={name}\nExec=\"$here/{name}\"\nPath=$here\n{icon_install}\nTerminal=false\nCategories=Game;\nENTRY\necho \"{name} is in the applications menu.\"\n"
    );
    write("install.sh", &install)
}

/// The player program to copy: `given`, else the one built beside this program.
fn player_program(session: &Session, given: Option<&Path>) -> Result<PathBuf, CommandError> {
    match given {
        Some(path) => session.allowed(path, Access::Read),
        None => {
            let here = std::env::current_exe().map_err(|e| CommandError::Failed(format!("cannot find this program: {e}")))?;
            Ok(here.with_file_name(format!("bayleaf-player{}", std::env::consts::EXE_SUFFIX)))
        }
    }
}

/// File extensions a scene or prefab may name a file of the project by.
const REFERENCED_EXTENSIONS: &[&str] = &[
    "bscene",
    "bui",
    "blight",
    "bpost",
    "bprefab",
    "bmaterial",
    "bscript",
    "wasm",
    "binput",
    "glb",
    "gltf",
    "obj",
    "png",
    "jpg",
    "jpeg",
    "exr",
    "wav",
    "flac",
    "ogg",
    "mp3",
    "aif",
    "aiff",
    "opus",
    "m4a",
    "aac",
];

/// Every text in a scene file's components that names a project file, by its extension.
fn referenced_files(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            let lower = text.to_ascii_lowercase();
            if !text.is_empty() && REFERENCED_EXTENSIONS.iter().any(|e| lower.ends_with(&format!(".{e}"))) {
                out.push(text.clone());
            }
        }
        Value::Array(items) => items.iter().for_each(|v| referenced_files(v, out)),
        Value::Object(map) => map.values().for_each(|v| referenced_files(v, out)),
        _ => {}
    }
}

/// What a scene or prefab file of the project needs: the files it names that are not there, and, for
/// the web, what does not work in a browser.
fn scene_findings(project: &Path, path: &Path, shown: &str, platform: Platform) -> Vec<(Severity, String)> {
    let Some(value) = std::fs::read_to_string(project.join(path)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else {
        return vec![(Severity::Error, format!("{shown} cannot be read."))];
    };
    let mut named = Vec::new();
    referenced_files(&value["objects"], &mut named);
    // A scene's settings name its lighting asset.
    referenced_files(&value["settings"], &mut named);
    named.sort();
    named.dedup();
    let mut findings: Vec<(Severity, String)> = named
        .into_iter()
        .filter(|n| !project.join(n).is_file())
        .map(|missing| (Severity::Error, format!("{shown} uses {missing}, which is not in the project.")))
        .collect();
    let streamed = <bayleaf_scene::components::StreamedArea as bayleaf_scene::components::SceneComponent>::NAME;
    if platform == Platform::Web && names_key(&value["objects"], streamed) {
        findings.push((Severity::Warning, format!("{shown} has Streamed Areas, which do not load in a browser yet: they are read on worker threads.")));
    }
    findings
}

/// Whether any object in a scene file's JSON has a member named `key` (a component).
fn names_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Array(items) => items.iter().any(|v| names_key(v, key)),
        Value::Object(map) => map.contains_key(key) || map.values().any(|v| names_key(v, key)),
        _ => false,
    }
}

/// The newest change to a Rust script crate's sources (`src`, `Cargo.toml`).
fn newest_source(dir: &Path) -> Option<std::time::SystemTime> {
    let mut newest = std::fs::metadata(dir.join("Cargo.toml")).and_then(|m| m.modified()).ok();
    let mut pending = vec![dir.join("src")];
    while let Some(folder) = pending.pop() {
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                newest = newest.max(Some(modified));
            }
        }
    }
    newest
}

/// Whether the game can be exported for `platform` with its scripts run in `mode`, and what it would
/// take: the platform and mode this version supports, the player program, the main scene, visual
/// scripts that compile, Rust scripts built from their latest sources, files the scenes name, and
/// how each script would run.
pub fn check(session: &Session, platform: Platform, mode: ScriptMode, player: Option<&Path>) -> Result<Report, CommandError> {
    let project = session.project_dir().map(Path::to_path_buf).ok_or_else(|| CommandError::Failed("open a project first".to_owned()))?;
    let project = session.allowed(&project, Access::Read)?;
    let mut findings = Vec::new();
    let mut found = |severity: Severity, message: String, file: Option<String>| findings.push(Finding { severity, message, file });
    let relative = |path: &Path| path.strip_prefix(&project).unwrap_or(path).to_string_lossy().replace('\\', "/");

    for (severity, message) in system_findings(session, platform, player) {
        found(severity, message, None);
    }
    if !mode.available_for(platform) {
        let why = match mode {
            ScriptMode::Compiled => format!("{} is not available for {}: browsers run scripts interpreted.", mode.label(), platform.label()),
            ScriptMode::Native if platform != Platform::current() => {
                format!("{} builds the game's player on this computer, so only for {}.", mode.label(), Platform::current().label())
            }
            ScriptMode::Native if crate::native_export::engine_source().is_none() => {
                format!("{} builds the game's player from the engine's source, which is not found; set BAYLEAF_SOURCE_DIR to its folder.", mode.label())
            }
            _ => format!("{} builds the game's player with Rust (cargo), which is not installed; get it from rustup.rs.", mode.label()),
        };
        found(Severity::Error, why, None);
    }
    if mode == ScriptMode::Native {
        found(
            Severity::Warning,
            "Native scripts run with no sandbox: such a script can do anything a program can, and a fault in one stops the game. Use them only for code you trust.".to_owned(),
            None,
        );
        found(Severity::Note, "The export builds the game's own player; the first build takes several minutes, later ones less.".to_owned(), None);
    } else if platform == Platform::current() {
        match player_program(session, player) {
            Ok(path) if path.is_file() => {}
            Ok(path) => found(
                Severity::Error,
                format!("The player program {} is not there; build it first (cargo build --release -p bayleaf-player).", path.display()),
                None,
            ),
            Err(e) => found(Severity::Error, e.to_string(), None),
        }
    }
    // The scene the game starts with.
    let file =
        crate::project_cmds::project_file_in(&project).and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| serde_json::from_str::<Value>(&t).ok());
    match file.as_ref().and_then(|f| f["mainScene"].as_str()).filter(|s| !s.is_empty()) {
        Some(main) if project.join(main).is_file() => {}
        Some(main) => found(Severity::Error, format!("The main scene {main} is not there."), Some(main.to_owned())),
        None => found(Severity::Error, "The project names no main scene to start with.".to_owned(), None),
    }

    let (files, _) = gather(&project)?;
    let mut scripts = Vec::new();
    for path in &files {
        let full = project.join(path);
        let shown = relative(&full);
        let extension = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if extension == bayleaf_script::VISUAL_EXTENSION {
            if let Err(e) = bayleaf_script::module_bytes(&full) {
                found(Severity::Error, format!("{shown} does not compile: {e}"), Some(shown.clone()));
            }
            let (runs, note) = match mode {
                ScriptMode::Native => (ScriptMode::Compiled, "a visual script stays compiled WebAssembly until visual scripts can be native"),
                other => (other, ""),
            };
            scripts.push(ScriptPlan { path: shown, kind: "visual", runs, note: note.to_owned() });
        } else if extension == "wasm" {
            let crate_dir = full.with_extension("");
            let from_source = crate_dir.join("Cargo.toml").is_file();
            if from_source
                && let (Some(source), Ok(built)) = (newest_source(&crate_dir), std::fs::metadata(&full).and_then(|m| m.modified()))
                && source > built
            {
                found(Severity::Warning, format!("{shown} is older than its Rust source; build it again to export the latest."), Some(shown.clone()));
            }
            let (runs, note) = match (mode, from_source) {
                (ScriptMode::Native, true) => (ScriptMode::Native, "built into the game from its Rust source"),
                (ScriptMode::Native, false) => (ScriptMode::Compiled, "no Rust source in the project, so it stays compiled WebAssembly"),
                (other, _) => (other, ""),
            };
            scripts.push(ScriptPlan { path: shown, kind: if from_source { "rust" } else { "module" }, runs, note: note.to_owned() });
        }
    }
    // Rust script crates whose module was never built.
    let mut pending = vec![project.clone()];
    while let Some(folder) = pending.pop() {
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if path.join("Cargo.toml").is_file() {
                let module = path.with_extension("wasm");
                if !module.is_file() {
                    found(
                        Severity::Error,
                        format!("The Rust script {} is not built yet; build it (Build Script) before exporting.", relative(&path)),
                        Some(relative(&path)),
                    );
                }
            } else if !skipped_folder(&path) {
                pending.push(path);
            }
        }
    }
    // Files the scenes and prefabs name that are not in the project.
    for path in files.iter().filter(|f| f.extension().is_some_and(|e| e == bayleaf_scene::doc::EXTENSION || e == crate::prefab_cmds::PREFAB_EXTENSION)) {
        let shown = relative(&project.join(path));
        for (severity, message) in scene_findings(&project, path, &shown, platform) {
            found(severity, message, Some(shown.clone()));
        }
    }
    if mode == ScriptMode::Native && !scripts.iter().any(|s| s.runs == ScriptMode::Native) {
        let message =
            "There is no Rust script with its source in the project, so Native builds a player that runs the scripts as Compiled to machine code does.";
        findings.push(Finding { severity: Severity::Note, message: message.to_owned(), file: None });
    }
    // Native's warning already says what it gives up.
    if mode != ScriptMode::Native && findings.iter().all(|f| f.severity != Severity::Error) {
        findings.push(Finding { severity: Severity::Note, message: mode.summary().to_owned(), file: None });
    }
    let ready = findings.iter().all(|f| f.severity != Severity::Error);
    Ok(Report { platform, mode, ready, findings, scripts, files: files.len() })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckParams {
    #[serde(default)]
    platform: Option<Platform>,
    #[serde(default)]
    mode: Option<ScriptMode>,
    #[serde(default)]
    player: Option<PathBuf>,
}

fn check_command(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: CheckParams = parse(params)?;
    let platform = p.platform.unwrap_or_else(Platform::current);
    let mode = p.mode.unwrap_or_else(|| chosen_mode(session, platform));
    let report = check(session, platform, mode, p.player.as_deref())?;
    serde_json::to_value(report).map_err(|e| CommandError::Failed(e.to_string()))
}

/// The project of a game exported beside `program`: the project file in its `game` folder.
pub fn packaged_game(program: &Path) -> Option<PathBuf> {
    crate::project_cmds::project_file_in(&program.parent()?.join(GAME_FOLDER))
}
