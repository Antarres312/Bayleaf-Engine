//! L5: editing session, command registry, undo history and play mode.
//!
//! Every action is a [`CommandSpec`] in the [`CommandRegistry`]. The editor, `bayleaf-cli`, the
//! JSON control channel and the MCP server all dispatch the same command ids (AA 14).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod asset_cmds;
pub mod asset_info;
mod builtin;
pub mod code_scripts;
pub mod confine;
pub mod export;
mod graph_cmds;
pub mod import;
pub mod input_cmds;
mod lighting_cmds;
pub mod native_export;
mod play;
pub mod prefab_cmds;
pub mod profiler;
pub mod project_cmds;
pub mod saved;
mod scene_cmds;
mod sound;
mod stream;

/// Optional world streaming, present only with the `streaming` feature.
#[cfg(feature = "streaming")]
pub use bayleaf_streaming as streaming;

pub use bayleaf_graph as graph;
pub use bayleaf_particles as particles;
pub use bayleaf_render as render;
pub use bayleaf_scene as scene;
pub use bayleaf_script as script;
pub use bayleaf_script::NativeScript;
pub use graph_cmds::{GraphDoc, catalog_for_domain, catalog_for_path, problems};
pub use scene_cmds::CLIPBOARD_FORMAT;

use std::path::{Path, PathBuf};

use bayleaf_scene::Scene;
use bayleaf_scene::doc::SceneDoc;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Errors returned when running a command.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("unknown command: {0}")]
    Unknown(String),
    #[error("command {id} is disabled: {reason}")]
    Disabled { id: String, reason: String },
    #[error("params must be a JSON object, null or omitted")]
    ParamsNotObject,
    #[error("invalid params: {0}")]
    BadParams(String),
    #[error("{0}")]
    Failed(String),
}

impl From<bayleaf_scene::SceneError> for CommandError {
    fn from(error: bayleaf_scene::SceneError) -> Self {
        Self::Failed(error.to_string())
    }
}

/// What an undo step restores, as compact JSON bytes: a parsed document takes several times more
/// memory than its text, and big scenes make many steps.
enum Snapshot {
    Scene(Vec<u8>),
    Graph(Vec<u8>),
}

/// Undo and redo together keep at most this many bytes of snapshots; the oldest steps go first.
pub const UNDO_BUDGET_BYTES: usize = 256 << 20;
/// Most undo steps kept.
pub const MAX_UNDO_STEPS: usize = 500;

impl Snapshot {
    fn scene(scene: &Scene) -> Self {
        // A scene document is plain data, so writing it as JSON does not fail.
        Self::Scene(serde_json::to_vec(&scene.to_doc()).unwrap_or_default())
    }

    fn graph(graph: &bayleaf_graph::Graph) -> Self {
        Self::Graph(serde_json::to_vec(graph).unwrap_or_default())
    }

    fn len(&self) -> usize {
        match self {
            Self::Scene(bytes) | Self::Graph(bytes) => bytes.len(),
        }
    }

    fn to_scene(&self) -> Result<Scene, CommandError> {
        let Self::Scene(bytes) = self else { return Err(CommandError::Failed("this undo step is not a scene".to_owned())) };
        let doc: SceneDoc = serde_json::from_slice(bytes).map_err(|e| CommandError::Failed(format!("the undo step cannot be read: {e}")))?;
        Ok(Scene::from_doc(&doc)?)
    }

    fn to_graph(&self) -> Result<bayleaf_graph::Graph, CommandError> {
        let Self::Graph(bytes) = self else { return Err(CommandError::Failed("this undo step is not a graph".to_owned())) };
        serde_json::from_slice(bytes).map_err(|e| CommandError::Failed(format!("the undo step cannot be read: {e}")))
    }
}

struct HistoryStep {
    command: String,
    before: Snapshot,
    merge_key: Option<String>,
}

/// Which document a command changes, and so what its undo step restores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mutates {
    /// Changes nothing that undo covers (reads, files, play mode).
    No,
    /// Changes the scene; one undo step, and the scene file is marked modified.
    Scene,
    /// Changes the open graph; one undo step, and the graph file is marked modified.
    Graph,
}

/// State that commands read and change: the open scene, its file, the undo history and play mode.
#[derive(Default)]
pub struct Session {
    scene: Scene,
    scene_path: Option<PathBuf>,
    dirty: bool,
    undo: Vec<HistoryStep>,
    redo: Vec<HistoryStep>,
    play: Option<play::Play>,
    input: std::sync::Arc<bayleaf_script::Input>,
    console: Vec<ConsoleLine>,
    /// Set while an automation request runs: the folders it may read and write.
    confinement: Option<confine::Confinement>,
    /// The graph open in the graph editor.
    graph: Option<GraphDoc>,
    /// The project folder the open scene belongs to, if any.
    project_dir: Option<PathBuf>,
    /// Sounds are heard in play mode (the editor and the player); otherwise they play silently.
    sound: bool,
    /// Scripts built into the game (native mode), by the module path they stand for; play mode runs
    /// them instead of those modules.
    native_scripts: Vec<(PathBuf, bayleaf_script::NativeScript)>,
    /// Prefabs open on their own, the innermost last, each with the scene it stands in front of.
    prefab_edits: Vec<prefab_cmds::PrefabEdit>,
    /// Play steps' times by part, while it records.
    profiler: profiler::Profiler,
}

/// How important a console line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    Info,
    Error,
}

/// One line in the editor console: script output and play-mode messages.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConsoleLine {
    pub level: Level,
    pub text: String,
}

/// Console lines kept; older ones are dropped.
const CONSOLE_LIMIT: usize = 2000;

impl Session {
    /// True while play mode runs (also when paused).
    pub fn is_playing(&self) -> bool {
        self.play.is_some()
    }

    pub fn is_paused(&self) -> bool {
        self.play.as_ref().is_some_and(|p| p.paused)
    }

    /// Seconds of simulated time since play mode started.
    pub fn play_time(&self) -> f64 {
        self.play.as_ref().map_or(0.0, |p| p.time)
    }

    /// Keys held down this frame, by name (see `docs/script-abi.md`).
    pub fn set_keys(&mut self, keys: std::collections::HashSet<String>) {
        if self.input.keys != keys {
            self.input = std::sync::Arc::new(bayleaf_script::Input { keys, ..(*self.input).clone() });
        }
    }

    /// The pointer over the game view (-1..1, +Y up; `None` outside it) and the mouse buttons held
    /// (bit 0 left, 1 right, 2 middle), as scripts see them.
    pub fn set_pointer(&mut self, pointer: Option<[f32; 2]>, buttons: u8) {
        if self.input.pointer != pointer || self.input.buttons != buttons {
            self.input = std::sync::Arc::new(bayleaf_script::Input { pointer, buttons, ..(*self.input).clone() });
        }
    }

    /// Plays sounds on the output device in play mode, from the next start. The editor and the
    /// player turn it on; the CLI, agents and tests leave it off, and sounds play silently, in time.
    pub fn set_sound(&mut self, on: bool) {
        self.sound = on;
    }

    /// Scripts built into this program (a game exported in native mode), each standing for the
    /// module at its path in the game (`scripts/mover.wasm`). Play mode runs them, from its next
    /// start, as machine code with no sandbox.
    pub fn set_native_scripts(&mut self, natives: Vec<(PathBuf, bayleaf_script::NativeScript)>) {
        self.native_scripts = natives;
    }

    /// The gamepad scripts see (`None`: none connected). Axes are clamped to their ranges.
    pub fn set_pad(&mut self, pad: Option<bayleaf_script::Pad>) {
        let pad = pad.map(|mut p| {
            for (i, a) in p.axes.iter_mut().enumerate() {
                let low = if i < 4 { -1.0 } else { 0.0 };
                *a = if a.is_finite() { a.clamp(low, 1.0) } else { 0.0 };
            }
            p
        });
        if self.input.pad != pad {
            self.input = std::sync::Arc::new(bayleaf_script::Input { pad, ..(*self.input).clone() });
        }
    }

    /// Keys the scripts currently see as held.
    pub fn held_keys(&self) -> &std::collections::HashSet<String> {
        &self.input.keys
    }

    /// Advances play mode by `dt` seconds unless it is paused. Does nothing outside play mode.
    pub fn tick(&mut self, dt: f32) {
        if self.play.as_ref().is_some_and(|p| !p.paused) {
            play::step(self, dt);
        }
    }

    pub fn console(&self) -> &[ConsoleLine] {
        &self.console
    }

    pub fn clear_console(&mut self) {
        self.console.clear();
    }

    /// Adds a line to the console.
    pub fn log_line(&mut self, level: Level, text: String) {
        self.log(level, text);
    }

    fn log(&mut self, level: Level, text: String) {
        self.console.push(ConsoleLine { level, text });
        if self.console.len() > CONSOLE_LIMIT {
            let excess = self.console.len() - CONSOLE_LIMIT;
            self.console.drain(..excess);
        }
    }

    /// Limits file access to the given folders (automation), or lifts the limit with `None`.
    pub fn set_confinement(&mut self, confinement: Option<confine::Confinement>) {
        self.confinement = confinement;
    }

    /// The path a command may use for `access`: unchanged without confinement, resolved and checked with it.
    fn allowed(&self, path: &Path, access: confine::Access) -> Result<PathBuf, CommandError> {
        match &self.confinement {
            Some(confinement) => confinement.check(path, access).map_err(CommandError::Failed),
            None => Ok(path.to_path_buf()),
        }
    }

    /// The folder that paths in the scene (script modules, materials) start from: the project
    /// folder, or the scene's own folder when it belongs to no project.
    pub fn script_dir(&self) -> PathBuf {
        if let Some(dir) = &self.project_dir {
            return dir.clone();
        }
        self.scene_path.as_deref().and_then(Path::parent).map_or_else(|| PathBuf::from("."), Path::to_path_buf)
    }

    /// The project folder the open scene belongs to: the nearest folder above it with a `.bproject` file.
    pub fn project_dir(&self) -> Option<&Path> {
        self.project_dir.as_deref()
    }

    fn set_scene_path(&mut self, path: Option<PathBuf>) {
        self.project_dir = path.as_deref().and_then(project_cmds::project_dir_of);
        self.scene_path = path;
    }
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// File the scene was opened from or last saved to.
    pub fn scene_path(&self) -> Option<&Path> {
        self.scene_path.as_deref()
    }

    /// True when the scene has changes that are not saved.
    pub fn profiler(&self) -> &profiler::Profiler {
        &self.profiler
    }

    /// What a view drew for the frame just stepped, while the game plays and the profiler records.
    pub fn profile_drawn(&mut self, stats: profiler::DrawnStats) {
        if self.is_playing() {
            self.profiler.add_drawn(stats);
        }
    }

    /// The prefabs open on their own (`prefab.open`), outermost first, by their paths in the
    /// project; empty while the scene itself is edited.
    pub fn open_prefabs(&self) -> Vec<String> {
        self.prefab_edits.iter().map(|e| e.source.clone()).collect()
    }

    /// Refuses a command that needs the scene itself while a prefab is open.
    fn no_prefab_open(&self, what: &str) -> Result<(), CommandError> {
        match self.prefab_edits.last() {
            Some(edit) => Err(CommandError::Failed(format!("{what}: the prefab {} is open; save it and go back first (prefab.close)", edit.source))),
            None => Ok(()),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Ids of the commands that can be undone, oldest first.
    pub fn history(&self) -> Vec<&str> {
        self.undo.iter().map(|s| s.command.as_str()).collect()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Opens `scene` in place of the current one. Prefab copies update from their files; the
    /// problems with files that cannot be read are returned.
    fn replace_scene(&mut self, scene: Scene, path: Option<PathBuf>) -> Vec<String> {
        self.scene = scene;
        self.set_scene_path(path);
        let mut problems = prefab_cmds::refresh_all(self);
        self.dirty = false;
        // An older scene's Environment moves into a lighting asset; the scene is then changed.
        if let Some((level, message, changed)) = lighting_cmds::move_environment(self) {
            self.dirty = changed;
            self.log(level, message.clone());
            problems.push(message);
        }
        self.forget(|s| matches!(s, Snapshot::Scene(_)));
        problems
    }

    /// Drops the undo and redo steps `which` selects.
    fn forget(&mut self, which: impl Fn(&Snapshot) -> bool) {
        self.undo.retain(|s| !which(&s.before));
        self.redo.retain(|s| !which(&s.before));
    }

    /// Drops the oldest undo steps beyond [`MAX_UNDO_STEPS`] or [`UNDO_BUDGET_BYTES`]. The newest
    /// step always stays, so the last change can be undone however big the scene is.
    fn trim_history(&mut self) {
        let mut total: usize = self.undo.iter().chain(&self.redo).map(|s| s.before.len()).sum();
        let mut drop = 0;
        while self.undo.len() - drop > 1 && (self.undo.len() - drop > MAX_UNDO_STEPS || total > UNDO_BUDGET_BYTES) {
            total -= self.undo[drop].before.len();
            drop += 1;
        }
        self.undo.drain(..drop);
    }

    /// Bytes the undo and redo steps keep.
    pub fn history_bytes(&self) -> usize {
        self.undo.iter().chain(&self.redo).map(|s| s.before.len()).sum()
    }

    /// The graph open in the graph editor.
    pub fn graph(&self) -> Option<&GraphDoc> {
        self.graph.as_ref()
    }

    /// A material library that shows the open material graph as it is, before it is saved.
    pub fn material_library(&self) -> bayleaf_render::MaterialLibrary {
        let mut library = bayleaf_render::MaterialLibrary::new();
        if let Some(doc) = &self.graph
            && doc.graph.domain == bayleaf_render::material::CATALOG.domain
            && let Some(path) = &doc.path
        {
            library.set_live(Some((path, &doc.graph)));
        }
        library
    }
}

pub(crate) type RunFn = fn(&mut Session, &Value) -> Result<Value, CommandError>;
pub(crate) type EnabledFn = fn(&Session) -> Result<(), String>;

/// What a front end has to ask the user for before running a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Prompt {
    /// Params can be null.
    None,
    /// `{"path"}` of an existing scene file.
    OpenScene,
    /// `{"path"}` to write a scene file to.
    SaveScene,
    /// `{"id"}` of the selected object.
    SelectedObject,
    /// `{"path"}` of a new, empty project folder.
    NewProject,
    /// `{"path"}` of a `.bproject` file.
    OpenProject,
    /// `{"folder"}`: a folder to write into.
    Folder,
    /// The front end shows a window of its own for the command's choices (Export Game).
    Dialog,
    /// A form of named values, sent as an object of them.
    Fields(&'static [PromptField]),
}

/// One value a [`Prompt::Fields`] form asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptField {
    /// The params key.
    pub name: &'static str,
    /// English label. It is the translation key.
    pub label: &'static str,
    /// What the form starts with.
    pub default: &'static str,
    /// Sent as a number rather than text.
    pub number: bool,
}

/// One registered command.
pub struct CommandSpec {
    /// Dotted id with camelCase segments, e.g. `scene.object.create`.
    pub id: &'static str,
    /// English label. It is the translation key.
    pub label: &'static str,
    /// Menu path, empty when the command has no menu entry.
    pub menu: &'static str,
    /// Default shortcut such as `Ctrl+Shift+S`, empty for none.
    pub shortcut: &'static str,
    /// Human-readable description of the params object.
    pub params: &'static str,
    pub prompt: Prompt,
    /// What the command changes; changes record one undo step.
    pub mutates: Mutates,
    pub enabled: EnabledFn,
    pub run: RunFn,
}

/// One row of `engine.commands`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub menu: &'static str,
    pub shortcut: &'static str,
    pub params: &'static str,
    pub prompt: Prompt,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

/// Registry of every command the engine knows.
pub struct CommandRegistry {
    commands: Vec<CommandSpec>,
}

impl CommandRegistry {
    /// Registry with the built-in commands.
    pub fn with_builtins() -> Self {
        let mut commands = builtin::commands();
        commands.extend(scene_cmds::commands());
        commands.extend(play::commands());
        commands.extend(sound::commands());
        commands.extend(stream::commands());
        commands.extend(export::commands());
        commands.extend(lighting_cmds::commands());
        commands.extend(profiler::commands());
        commands.extend(graph_cmds::commands());
        commands.extend(project_cmds::commands());
        commands.extend(prefab_cmds::commands());
        commands.extend(code_scripts::commands());
        commands.extend(asset_info::commands());
        commands.extend(asset_cmds::commands());
        commands.extend(import::commands());
        commands.extend(input_cmds::commands());
        Self { commands }
    }

    pub fn get(&self, id: &str) -> Option<&CommandSpec> {
        self.commands.iter().find(|c| c.id == id)
    }

    pub fn list(&self, session: &Session) -> Vec<CommandInfo> {
        self.commands
            .iter()
            .map(|c| {
                let state = (c.enabled)(session);
                CommandInfo {
                    id: c.id,
                    label: c.label,
                    menu: c.menu,
                    shortcut: c.shortcut,
                    params: c.params,
                    prompt: c.prompt,
                    enabled: state.is_ok(),
                    disabled_reason: state.err(),
                }
            })
            .collect()
    }

    /// Runs `id` with `params`, which must be a JSON object or null.
    pub fn execute(&self, session: &mut Session, id: &str, params: &Value) -> Result<Value, CommandError> {
        self.execute_merging(session, id, params, None)
    }

    /// Like [`Self::execute`], but consecutive mutating runs with the same `merge_key` share one
    /// undo step. The editor uses this while a value is being dragged or typed.
    pub fn execute_merging(&self, session: &mut Session, id: &str, params: &Value, merge_key: Option<&str>) -> Result<Value, CommandError> {
        if !(params.is_object() || params.is_null()) {
            return Err(CommandError::ParamsNotObject);
        }
        let spec = self.get(id).ok_or_else(|| CommandError::Unknown(id.to_owned()))?;
        (spec.enabled)(session).map_err(|reason| CommandError::Disabled { id: id.to_owned(), reason })?;
        let before = match spec.mutates {
            Mutates::No => return (spec.run)(session, params),
            Mutates::Scene => Snapshot::scene(&session.scene),
            Mutates::Graph => Snapshot::graph(&session.graph.as_ref().ok_or_else(|| CommandError::Failed(graph_cmds::NO_GRAPH.to_owned()))?.graph),
        };
        match (spec.run)(session, params) {
            // Play-mode changes to the scene are thrown away on stop, so they are not undo steps and do not dirty the file.
            Ok(result) if session.is_playing() && spec.mutates == Mutates::Scene => Ok(result),
            Ok(result) => {
                let merges = merge_key.is_some() && session.undo.last().is_some_and(|s| s.merge_key.as_deref() == merge_key);
                session.redo.clear();
                if !merges {
                    session.undo.push(HistoryStep { command: id.to_owned(), before, merge_key: merge_key.map(str::to_owned) });
                    session.trim_history();
                }
                match spec.mutates {
                    Mutates::Graph => {
                        if let Some(graph) = session.graph.as_mut() {
                            graph.dirty = true;
                        }
                    }
                    _ => session.dirty = true,
                }
                Ok(result)
            }
            Err(error) => {
                // A failed command leaves its document as it was.
                if let Ok(scene) = before.to_scene() {
                    session.scene = scene;
                } else if let (Ok(graph), Some(open)) = (before.to_graph(), session.graph.as_mut()) {
                    open.graph = graph;
                }
                Err(error)
            }
        }
    }
}

/// Parses a params object (null counts as `{}`) into `T`.
fn parse<T: DeserializeOwned>(params: &Value) -> Result<T, CommandError> {
    let value = if params.is_null() { Value::Object(serde_json::Map::new()) } else { params.clone() };
    serde_json::from_value(value).map_err(|e| CommandError::BadParams(e.to_string()))
}

fn always(_: &Session) -> Result<(), String> {
    Ok(())
}

/// For commands that would lose or save play-mode state: scene files, undo and redo.
fn not_playing(session: &Session) -> Result<(), String> {
    if session.is_playing() { Err("stop play mode first".to_owned()) } else { Ok(()) }
}

#[cfg(test)]
mod tests;
