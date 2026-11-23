//! Project panel: the game's files under **Project** (the project folder, the open scene's folder, or the folder the
//! editor started in) and the engine's own files under **Engine**, so the two never mix. Folders on
//! the left; the open folder's files on the right, as icons or as a list. The Engine folder is only
//! listed on the left, never shown as a tile among the project's files.
//!
//! Double-click a folder to open it and a scene to load it. Drag a file onto a text field in the
//! inspector to use its path (a script module, for example); the context menu offers the same
//! without dragging.

use crate::i18n::tl;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bayleaf_engine::asset_cmds::AssetKind;
use serde_json::json;

use crate::theme::{self, LIST_ROW, Tokens};
use crate::thumbnails::{Thumb, Thumbnails};
use crate::{Editor, inspector};

/// Files listed at most; bigger folders are cut off with a notice.
const MAX_ENTRIES: usize = 5000;
/// How often the folder is read again while the panel is open.
const RESCAN_INTERVAL: Duration = Duration::from_secs(2);
/// Folders that hold build output or tools, not project files.
const SKIPPED: &[&str] = &["target", "node_modules"];
/// Icon size in the icon view: default and limits, in points.
const ICON_DEFAULT: f32 = 40.0;
const ICON_MIN: f32 = 24.0;
const ICON_MAX: f32 = 128.0;
const SLIDER_WIDTH: f32 = 72.0;
const SEARCH_WIDTH: f32 = 160.0;

/// Drag-and-drop payload: a project file.
#[derive(Clone, Debug)]
pub(crate) struct DraggedAsset {
    /// Path relative to the project folder, with `/` separators.
    pub relative: String,
}

impl DraggedAsset {
    /// Whether it is a script an object can run: a built module or a visual script.
    pub(crate) fn is_script(&self) -> bool {
        self.relative.ends_with(".wasm") || self.relative.ends_with(".bscript")
    }
}

/// The script being dragged from the Project panel, if one is.
pub(crate) fn dragged_script(ctx: &egui::Context) -> Option<String> {
    egui::DragAndDrop::payload::<DraggedAsset>(ctx).filter(|a| a.is_script()).map(|a| a.relative.clone())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Folder,
    /// A script written in Rust: a folder with a Cargo.toml.
    ScriptSource,
    Scene,
    /// A project file (`.bproject`).
    Project,
    /// A prefab (`.bprefab`): objects to place in scenes.
    Prefab,
    /// A 3D model (`.glb`, `.gltf`).
    Model,
    Script,
    /// A visual script (`.bscript`): a graph that is also a script.
    VisualScript,
    Material,
    /// An Input Map (`.binput`): input actions in maps.
    InputMap,
    /// A lighting asset (`.blight`): a scene's sky, light around everything, fog and sun.
    Lighting,
    /// A post-processing asset (`.bpost`): what a camera does to the drawn picture.
    PostProcessing,
    /// A game interface layout (`.bui`).
    Interface,
    /// A style file for game interfaces (`.bstyle`).
    InterfaceStyle,
    Image,
    /// A sound file an Audio Source plays.
    Sound,
    Other,
}

impl Kind {
    fn of(path: &Path) -> Self {
        match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some(bayleaf_engine::scene::doc::EXTENSION) => Kind::Scene,
            Some("wasm") => Kind::Script,
            Some(bayleaf_engine::project_cmds::PROJECT_EXTENSION) => Kind::Project,
            Some(bayleaf_engine::prefab_cmds::PREFAB_EXTENSION) => Kind::Prefab,
            Some("bscript") => Kind::VisualScript,
            Some("bmaterial") => Kind::Material,
            Some(bayleaf_engine::input_cmds::INPUT_EXTENSION) => Kind::InputMap,
            Some(bayleaf_engine::scene::lighting::EXTENSION) => Kind::Lighting,
            Some(bayleaf_engine::scene::post::EXTENSION) => Kind::PostProcessing,
            Some(bayleaf_interface::LAYOUT_EXTENSION) => Kind::Interface,
            Some(bayleaf_interface::STYLE_EXTENSION) => Kind::InterfaceStyle,
            Some("png" | "jpg" | "jpeg" | "exr" | "webp" | "ktx2") => Kind::Image,
            Some(ext) if bayleaf_engine::render::assets::MODEL_EXTENSIONS.contains(&ext) => Kind::Model,
            Some(ext) if bayleaf_engine::scene::components::SOUND_EXTENSIONS.contains(&ext) => Kind::Sound,
            _ => Kind::Other,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Kind::Folder => "Folder",
            Kind::ScriptSource => "Rust Script",
            Kind::Scene => "Scene",
            Kind::Project => "Project",
            Kind::Prefab => "Prefab",
            Kind::Model => "Model",
            Kind::Script => "Script",
            Kind::VisualScript => "Visual Script",
            Kind::Material => "Material",
            Kind::InputMap => "Input Map",
            Kind::Lighting => "Lighting",
            Kind::PostProcessing => "Post-processing",
            Kind::Interface => "Interface",
            Kind::InterfaceStyle => "Interface Style",
            Kind::Image => "Image",
            Kind::Sound => "Sound",
            Kind::Other => "File",
        }
    }
}

#[derive(Clone, Debug)]
struct Entry {
    path: PathBuf,
    name: String,
    kind: Kind,
}

impl Entry {
    /// The name the panel shows: without its extension when `hide` is set (folders and Rust
    /// scripts keep their names; so does a name that is only an extension).
    fn shown(&self, hide: bool) -> &str {
        match self.name.rfind('.') {
            Some(dot) if hide && dot > 0 && !matches!(self.kind, Kind::Folder | Kind::ScriptSource) => &self.name[..dot],
            _ => &self.name,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ViewMode {
    #[default]
    Icons,
    List,
}

/// Most folders kept to go back to.
const HISTORY: usize = 64;

/// The scanned folder and the panel's own state, kept between frames.
#[derive(Default)]
pub(crate) struct ProjectView {
    /// The project folder; `None` when there is none (no scene open, and the editor runs from the
    /// engine's own folder).
    root: Option<PathBuf>,
    /// The engine's own folder, if it can be found.
    engine: Option<PathBuf>,
    /// Folder -> its entries, folders first, then by name.
    tree: BTreeMap<PathBuf, Vec<Entry>>,
    scanned: Option<Instant>,
    truncated: bool,
    filter: String,
    /// The folder whose files are shown on the right.
    pub(crate) current: PathBuf,
    /// Folders shown before, the newest last, for going back; and the ones gone back from.
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    /// The folder shown in the last frame, to tell when another one opened.
    shown: PathBuf,
    /// The folder's contents take the keyboard next frame: a folder opened from the keyboard keeps it.
    focus_contents: bool,
    mode: ViewMode,
    pub(crate) selected: Option<PathBuf>,
    /// Icon size in points; 0 means the default.
    icon_size: f32,
    /// A new file being named; nothing is written until the name is confirmed.
    naming: Option<Naming>,
    /// A file or folder had keyboard focus last frame: plain keys such as Delete are the panel's.
    pub(crate) focused: bool,
    /// A file or folder waiting for the delete to be confirmed.
    deleting: Option<PathBuf>,
    /// Names show without their extensions (the project's preference).
    pub(crate) hide_extensions: bool,
    /// Tests delete for good instead of filling the Recycle Bin.
    #[cfg(test)]
    pub(crate) delete_permanently: bool,
}

impl ProjectView {
    /// Remembers the folder left when another opened (by a click, a path crumb or a new file),
    /// so going back returns to it; a new way forward forgets the old one.
    fn remember(&mut self) {
        if self.current == self.shown {
            return;
        }
        let left = std::mem::replace(&mut self.shown, self.current.clone());
        if !left.as_os_str().is_empty() {
            self.back.push(left);
            if self.back.len() > HISTORY {
                self.back.remove(0);
            }
        }
        self.forward.clear();
    }

    /// Opens the folder shown before (`back`) or the one gone back from; folders that are gone are
    /// skipped.
    fn go(&mut self, back: bool) {
        loop {
            let next = if back { self.back.pop() } else { self.forward.pop() };
            let Some(folder) = next else { return };
            if folder == self.current || !self.tree.contains_key(&folder) {
                continue;
            }
            let left = std::mem::replace(&mut self.current, folder);
            if back {
                self.forward.push(left)
            } else {
                self.back.push(left)
            }
            self.shown = self.current.clone();
            self.filter.clear();
            return;
        }
    }

    /// Every script (`.wasm` and `.bscript`) in the project, as paths relative to the project folder.
    pub(crate) fn script_files(&self) -> Vec<String> {
        self.files_of(&[Kind::Script, Kind::VisualScript])
    }

    /// Every material graph in the project, as paths relative to the project folder.
    pub(crate) fn material_files(&self) -> Vec<String> {
        self.files_of(&[Kind::Material])
    }

    /// Every prefab (`.bprefab`) in the project.
    pub(crate) fn prefab_files(&self) -> Vec<String> {
        self.files_of(&[Kind::Prefab])
    }

    /// Every Input Map (`.binput`) in the project.
    pub(crate) fn input_map_files(&self) -> Vec<String> {
        self.files_of(&[Kind::InputMap])
    }

    /// Every lighting asset in the project, as paths relative to the project folder.
    pub(crate) fn lighting_files(&self) -> Vec<String> {
        self.files_of(&[Kind::Lighting])
    }

    /// Every post-processing asset in the project, as paths relative to the project folder.
    pub(crate) fn post_files(&self) -> Vec<String> {
        self.files_of(&[Kind::PostProcessing])
    }

    /// Every game interface layout (`.bui`) in the project.
    pub(crate) fn interface_files(&self) -> Vec<String> {
        self.files_of(&[Kind::Interface])
    }

    /// Every 3D model (`.glb`, `.gltf`) in the project.
    pub(crate) fn model_files(&self) -> Vec<String> {
        self.files_of(&[Kind::Model])
    }

    /// Every scene file in the project.
    pub(crate) fn scene_files(&self) -> Vec<String> {
        self.files_of(&[Kind::Scene])
    }

    /// Every sound file in the project.
    pub(crate) fn sound_files(&self) -> Vec<String> {
        self.files_of(&[Kind::Sound])
    }

    /// Every PNG and JPEG image in the project (the image formats the engine reads).
    pub(crate) fn image_files(&self) -> Vec<String> {
        self.files_of(&[Kind::Image])
            .into_iter()
            .filter(|f| {
                let f = f.to_ascii_lowercase();
                bayleaf_render::assets::IMAGE_EXTENSIONS.iter().any(|e| f.ends_with(&format!(".{e}")))
            })
            .collect()
    }

    fn files_of(&self, kinds: &[Kind]) -> Vec<String> {
        let Some(root) = &self.root else { return Vec::new() };
        let mut files: Vec<String> =
            self.tree.values().flatten().filter(|e| kinds.contains(&e.kind) && e.path.starts_with(root)).map(|e| relative_path(root, &e.path)).collect();
        files.sort();
        files
    }
}

impl ProjectView {
    /// Reads the folders again on the next frame (after a file was made).
    pub(crate) fn rescan_soon(&mut self) {
        self.scanned = None;
    }

    pub(crate) fn icon(&self) -> f32 {
        if self.icon_size > 0.0 { self.icon_size } else { ICON_DEFAULT }
    }

    /// The top folder `path` belongs to, with its name: the project first, as it may lie inside
    /// the engine's folder (the bundled examples do).
    fn top_of(&self, path: &Path) -> Option<(&'static str, &Path)> {
        if let Some(root) = self.root.as_deref().filter(|r| path.starts_with(r)) {
            return Some(("Project", root));
        }
        self.engine.as_deref().filter(|e| path.starts_with(e)).map(|e| ("Engine", e))
    }

    /// True when the open folder belongs to the project, so new files may be made there.
    fn in_project(&self) -> bool {
        self.root.as_deref().is_some_and(|r| !self.current.as_os_str().is_empty() && self.current.starts_with(r))
    }

    /// A `.wasm` module built from a Rust script folder beside it: listed as part of that script.
    fn is_built_module(&self, entry: &Entry) -> bool {
        entry.kind == Kind::Script && entry.path.with_extension("").join("Cargo.toml").is_file()
    }

    /// Whether `path` is inside the project folder (new assets go only there).
    fn is_in_project(&self, path: &Path) -> bool {
        self.root.as_deref().is_some_and(|r| !path.as_os_str().is_empty() && path.starts_with(r))
    }
}

/// What the Create panel offers: the kind, its name, what it is, and the name a new one starts with.
const CREATABLE: [(AssetKind, &str, &str, &str); 10] = [
    (AssetKind::Folder, "Folder", "A folder to sort files into", "New Folder"),
    (AssetKind::Scene, "Scene", "A level or screen, with a camera and a light", "New Scene"),
    (AssetKind::Material, "Material", "How a surface looks, as a graph", "New Material"),
    (AssetKind::VisualScript, "Visual Script", "Behaviour as a graph of nodes", "New Visual Script"),
    (AssetKind::Script, "Rust Script", "Behaviour in Rust, built to WebAssembly", "New Script"),
    (AssetKind::InputMap, "Input Map", "Actions in maps for keys, mouse and gamepads", "New Input Map"),
    (AssetKind::Lighting, "Lighting", "A scene's sky, light, fog and sun, shared by the scenes that name it", "New Lighting"),
    (AssetKind::PostProcessing, "Post-processing", "Bloom, tone mapping, colour, vignette and more, for a scene or a camera", "New Post-processing"),
    (AssetKind::Interface, "Interface", "A game's menu or display: panels, text, buttons and more", "New Interface"),
    (AssetKind::InterfaceStyle, "Interface Style", "How interfaces look: rules for kinds, classes and names", "New Interface Style"),
];

/// The lowercase name of a kind, for "New â¦" prompts.
fn kind_name(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Folder => "folder",
        AssetKind::Scene => "scene",
        AssetKind::Material => "material",
        AssetKind::VisualScript => "visual script",
        AssetKind::Script => "script",
        AssetKind::InputMap => "input map",
        AssetKind::Lighting => "lighting",
        AssetKind::PostProcessing => "post-processing",
        AssetKind::Interface => "interface",
        AssetKind::InterfaceStyle => "interface style",
    }
}

/// The icon a kind of new asset shows.
fn kind_icon(kind: AssetKind) -> Kind {
    match kind {
        AssetKind::Folder => Kind::Folder,
        AssetKind::Scene => Kind::Scene,
        AssetKind::Material => Kind::Material,
        AssetKind::VisualScript => Kind::VisualScript,
        AssetKind::Script => Kind::ScriptSource,
        AssetKind::InputMap => Kind::InputMap,
        AssetKind::Lighting => Kind::Lighting,
        AssetKind::PostProcessing => Kind::PostProcessing,
        AssetKind::Interface => Kind::Interface,
        AssetKind::InterfaceStyle => Kind::InterfaceStyle,
    }
}

/// What a name being typed is for.
#[derive(Clone, Debug, PartialEq)]
enum NameTarget {
    /// A new asset of this kind in this folder; nothing is written before the name.
    New { kind: AssetKind, folder: PathBuf },
    /// A file or folder getting a new name. `hidden` is the extension (with its dot) the panel
    /// hides, added back to the typed name.
    Rename { path: PathBuf, hidden: Option<String> },
}

/// A name typed in place, in the file's own tile or row: a new asset's, or a renamed one's.
pub(crate) struct Naming {
    target: NameTarget,
    text: String,
    /// The field has had focus (and its text selected) once.
    focused: bool,
    /// Why the last name was refused.
    error: Option<String>,
}

enum Action {
    OpenScene(PathBuf),
    PlacePrefab(PathBuf),
    /// Edits a prefab on its own (`prefab.open`).
    OpenPrefab(PathBuf),
    OpenGraph(PathBuf),
    AddScript(String),
    /// Start naming a new asset of this kind, in this folder or else the open one.
    StartNew(AssetKind, Option<PathBuf>),
    /// Make the file being named, with this name.
    Create(String),
    /// Start renaming this file or folder, in its own tile or row.
    StartRename(PathBuf),
    /// Give the file being renamed this name.
    RenameTo(String),
    /// Build the script crate in this folder.
    BuildScript(PathBuf),
    /// Open a file with the program the system uses for it.
    OpenExternal(PathBuf),
    /// Show a file or folder in the system's file manager.
    Reveal(PathBuf),
    /// Read the folders again now.
    Refresh,
    /// Ask before deleting this file or folder.
    AskDelete(PathBuf),
    /// Save this object (and its children) as a new prefab in this folder.
    MakePrefab(bayleaf_core::StableId, PathBuf),
    /// Edit an interface layout in the Interface panel.
    OpenInterface(PathBuf),
    /// Bring files from the system into this folder.
    Import(Vec<PathBuf>, PathBuf),
}

/// What the panel's parts ask for after drawing.
#[derive(Default)]
struct Outcome {
    action: Option<Action>,
    enter: Option<PathBuf>,
    select: Option<PathBuf>,
    /// A file clicked (not only pressed to drag): the Inspector shows it.
    inspect: Option<PathBuf>,
    /// A file or folder has keyboard focus, so plain keys (Delete) are the panel's.
    focused: bool,
    /// The empty space between the files was clicked: nothing stays selected.
    deselect: bool,
}

pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    let hide_extensions = crate::project_settings::current(editor).editor.hide_extensions;
    let mut view = std::mem::take(&mut editor.project);
    view.hide_extensions = hide_extensions;
    if view.scanned.is_none() {
        view.engine = engine_root();
    }
    let root = project_root(editor, view.engine.as_deref());
    if view.root != root || view.scanned.is_none_or(|at| at.elapsed() > RESCAN_INTERVAL) {
        view.rescan(root);
        ui.ctx().request_repaint_after(RESCAN_INTERVAL);
    }
    if !view.tree.contains_key(&view.current) {
        view.current = view.root.clone().unwrap_or_default();
    }
    view.remember();
    // The mouse's back and forward buttons over the panel, or Alt+Left and Alt+Right while a file
    // or folder has the keyboard, go back and forth through the folders opened.
    let (back, forward) = ui.input_mut(|i| {
        let over = i.pointer.hover_pos().is_some_and(|p| ui.max_rect().contains(p));
        let keys = view.focused;
        (
            over && i.pointer.button_pressed(egui::PointerButton::Extra1) || keys && i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowLeft),
            over && i.pointer.button_pressed(egui::PointerButton::Extra2) || keys && i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowRight),
        )
    });
    if back || forward {
        view.go(back);
        // The tile that had the keyboard may not be in this folder: the folder's contents take it.
        view.focus_contents = view.focused;
    }

    let mut outcome = Outcome::default();
    toolbar(ui, &mut view, &mut outcome);
    ui.separator();
    let selected_object = editor.selected.is_some();

    egui::Panel::left("project folders").resizable(true).default_size(170.0).min_size(90.0).frame(egui::Frame::new()).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            theme::dense(ui);
            let slot = ui.painter().add(egui::Shape::Noop);
            let full_width = ui.max_rect().x_range();
            let mut rows = Vec::new();
            top_folders(ui, &view, &mut rows, &mut outcome);
            theme::stripes(ui, slot, rows.into_iter(), full_width);
        });
    });
    // A name being typed is drawn in its own tile; it is held apart while the files are drawn.
    let mut naming = view.naming.take();
    let files_area = egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(egui::Margin { left: 6, ..egui::Margin::ZERO })).show(ui, |ui| {
        let thumbnails = &mut editor.thumbnails;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| files(ui, &view, &mut naming, selected_object, thumbnails, &mut outcome));
    });
    view.naming = naming;
    if outcome.deselect {
        view.selected = None;
    }
    // Ctrl + mouse wheel over the files resizes the icons. egui reports it as a zoom gesture.
    if view.mode == ViewMode::Icons && files_area.response.contains_pointer() {
        let zoom = ui.input(|i| i.zoom_delta());
        if zoom != 1.0 {
            view.icon_size = (view.icon() * zoom).clamp(ICON_MIN, ICON_MAX);
        }
    }

    if let Some(folder) = outcome.enter {
        view.current = folder;
        view.filter.clear();
    }
    view.focused = outcome.focused;
    view.focus_contents = false;
    if let Some(file) = outcome.select {
        view.selected = Some(file);
    }
    editor.project = view;
    if outcome.deselect {
        editor.inspected_file = None;
    }
    if let Some(path) = outcome.inspect {
        // A clicked file takes the Inspector from the selected objects.
        editor.select_only(None);
        editor.inspected_file = Some(crate::file_inspector::Inspected::new(path));
    }
    match outcome.action {
        Some(Action::OpenScene(path)) => editor.open_scene(&path),
        Some(Action::PlacePrefab(path)) => editor.place_prefab(&path),
        Some(Action::OpenPrefab(path)) => {
            editor.dispatch("prefab.open", &json!({ "path": path }), None);
        }
        Some(Action::OpenGraph(path)) => editor.open_graph(&path),
        Some(Action::StartNew(kind, folder)) => {
            // New files go into the project only, never into the engine's folder.
            let project = &editor.project;
            let folder = folder
                .filter(|f| project.is_in_project(f))
                .unwrap_or_else(|| if project.in_project() { project.current.clone() } else { project.root.clone().unwrap_or_default() });
            if !folder.as_os_str().is_empty() {
                let text = CREATABLE.iter().find(|(k, ..)| *k == kind).map_or("New Asset", |(.., start)| start);
                // The new name is typed where the files are shown: open the folder it goes into.
                editor.project.current = folder.clone();
                editor.project.naming = Some(Naming { target: NameTarget::New { kind, folder }, text: text.to_owned(), focused: false, error: None });
            }
        }
        Some(Action::Create(name)) => {
            if let Some(naming) = editor.project.naming.take() {
                match create(editor, &naming, &name) {
                    Ok(made) => {
                        editor.project.scanned = None;
                        editor.project.selected = Some(made);
                    }
                    // Keep the field open with the reason, so the name can be fixed.
                    Err(error) => editor.project.naming = Some(Naming { error: Some(error), focused: false, text: name, ..naming }),
                }
            }
        }
        Some(Action::OpenInterface(path)) => editor.open_interface(&path),
        Some(Action::Import(files, folder)) => {
            // Only into the project; a drop on the engine's folders goes to the open project folder.
            let folder = editor.project.is_in_project(&folder).then_some(folder);
            crate::file_drop::bring_in(editor, files, folder, crate::file_drop::Place::Project);
            editor.project.scanned = None;
        }
        Some(Action::MakePrefab(object, folder)) => {
            let name = editor.session.scene().name(object).unwrap_or("Prefab").to_owned();
            let path = new_prefab_path(&folder, &name);
            if editor.dispatch("scene.prefab.create", &json!({ "id": object, "path": path }), None).is_some() {
                editor.project.scanned = None;
                editor.project.current = folder;
                editor.project.selected = Some(path);
            }
        }
        Some(Action::StartRename(path)) => {
            // The name is typed in the file's own tile: open the folder that lists it.
            if let Some(parent) = path.parent() {
                editor.project.current = parent.to_path_buf();
            }
            editor.project.filter.clear();
            let full = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            // With extensions hidden, only the name before the extension is typed; the extension stays.
            let (text, hidden) = match full.rfind('.').filter(|&dot| dot > 0 && editor.project.hide_extensions && path.is_file()) {
                Some(dot) => (full[..dot].to_owned(), Some(full[dot..].to_owned())),
                None => (full, None),
            };
            editor.project.selected = Some(path.clone());
            editor.project.naming = Some(Naming { target: NameTarget::Rename { path, hidden }, text, focused: false, error: None });
        }
        Some(Action::RenameTo(name)) => {
            if let Some(naming) = editor.project.naming.take()
                && let NameTarget::Rename { path, hidden } = &naming.target
            {
                let full = format!("{name}{}", hidden.as_deref().unwrap_or_default());
                match editor.dispatch("asset.rename", &json!({ "path": path, "name": full }), None) {
                    Some(renamed) => {
                        let renamed = PathBuf::from(renamed["path"].as_str().unwrap_or_default());
                        editor.project.scanned = None;
                        editor.project.selected = Some(renamed.clone());
                        if editor.inspected_file.as_ref().is_some_and(|f| f.path == *path) {
                            editor.inspected_file = Some(crate::file_inspector::Inspected::new(renamed));
                        }
                    }
                    None => {
                        let error = editor.status.clone();
                        editor.project.naming = Some(Naming { error: Some(error), focused: false, text: name, ..naming });
                    }
                }
            }
        }
        Some(Action::OpenExternal(path)) => {
            if let Err(error) = system_open(&path, false) {
                editor.status = crate::i18n::tf("Could not open {path}: {error}", &[("path", &path.display().to_string()), ("error", &error.to_string())]);
            }
        }
        Some(Action::Reveal(path)) => {
            if let Err(error) = system_open(&path, true) {
                editor.status = crate::i18n::tf("Could not show {path}: {error}", &[("path", &path.display().to_string()), ("error", &error.to_string())]);
            }
        }
        Some(Action::Refresh) => editor.project.scanned = None,
        Some(Action::AskDelete(path)) => editor.project.deleting = Some(path),
        Some(Action::BuildScript(dir)) => editor.build_script(&dir),
        Some(Action::AddScript(module)) => {
            if let Some(id) = editor.selected {
                editor.dispatch("scene.script.add", &json!({ "id": id, "module": module }), None);
            }
        }
        None => {}
    }
    delete_dialog(editor, ui.ctx());
}

/// Asks before a file or folder goes to the Recycle Bin. The confirm button has focus, so Enter
/// deletes and Escape cancels.
fn delete_dialog(editor: &mut Editor, ctx: &egui::Context) {
    let Some(path) = editor.project.deleting.clone() else { return };
    let t = theme::tokens_for(ctx);
    let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    let folder = path.is_dir();
    let mut confirmed = false;
    let mut cancelled = false;
    let modal = egui::Modal::new(egui::Id::unique("delete asset")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.heading(tl("Move to the Recycle Bin?"));
        ui.add_space(theme::GAP);
        let what = if folder { crate::i18n::tf("The folder {name} and everything in it", &[("name", &name.to_string())]) } else { name.clone() };
        ui.add(egui::Label::new(egui::RichText::new(what).color(t.text)).wrap());
        ui.label(egui::RichText::new(tl("You can restore it from the Recycle Bin.")).color(t.text_muted));
        ui.add_space(theme::GAP * 2.0);
        ui.horizontal(|ui| {
            let delete = ui.add(egui::Button::new(egui::RichText::new(tl("Move to Recycle Bin")).color(t.on_accent)).fill(t.accent));
            // Focus starts on the confirm button once; Tab still reaches Cancel.
            let opened = egui::Id::unique("delete asset focused");
            if !ui.ctx().data(|d| d.get_temp::<bool>(opened).unwrap_or(false)) {
                delete.request_focus();
                ui.ctx().data_mut(|d| d.insert_temp(opened, true));
            }
            confirmed = delete.clicked();
            cancelled = ui.button(tl("Cancel")).clicked();
        });
    });
    if !(modal.should_close() || cancelled || confirmed) {
        return;
    }
    editor.project.deleting = None;
    ctx.data_mut(|d| d.remove::<bool>(egui::Id::unique("delete asset focused")));
    if !confirmed {
        return;
    }
    #[cfg(test)]
    let permanent = editor.project.delete_permanently;
    #[cfg(not(test))]
    let permanent = false;
    if editor.dispatch("asset.delete", &json!({ "path": path, "permanent": permanent }), None).is_some() {
        editor.project.scanned = None;
        if editor.project.selected.as_ref().is_some_and(|s| s.starts_with(&path)) {
            editor.project.selected = None;
        }
        if editor.inspected_file.as_ref().is_some_and(|f| f.path.starts_with(&path)) {
            editor.inspected_file = None;
        }
        editor.status = crate::i18n::tf("Moved {name} to the Recycle Bin.", &[("name", &name.to_string())]);
    }
}

/// The name field in a tile or row, `width` wide. It takes focus with the name selected (a file's
/// name without its extension), so typing replaces it. Enter confirms; Escape cancels; clicking
/// elsewhere confirms a rename and cancels a new asset, which nothing has been written for yet.
/// Why a name was refused shows just under the field.
fn name_field(ui: &mut egui::Ui, naming: &mut Option<Naming>, width: f32, outcome: &mut Outcome) {
    let Some(name) = naming.as_mut() else { return };
    let t = theme::tokens(ui);
    let label = match &name.target {
        NameTarget::New { kind, .. } => crate::i18n::tf("Name of the new {kind}", &[("kind", (tl(kind_name(*kind))))]),
        NameTarget::Rename { path, .. } => {
            crate::i18n::tf("New name of {name}", &[("name", &(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).to_string())])
        }
    };
    let field =
        ui.add_sized([width, LIST_ROW + 2.0], egui::TextEdit::singleline(&mut name.text).margin(egui::vec2(4.0, 1.0)).align(egui::Align2::CENTER_CENTER));
    ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(label));
    if let Some(error) = &name.error {
        ui.painter().rect_stroke(field.rect, theme::RADIUS, egui::Stroke::new(1.5, t.error), egui::StrokeKind::Outside);
        egui::Area::new(field.id.with("problem")).order(egui::Order::Foreground).fixed_pos(field.rect.left_bottom() + egui::vec2(0.0, 4.0)).show(
            ui.ctx(),
            |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(280.0);
                    ui.add(egui::Label::new(egui::RichText::new(error).color(t.text)).wrap());
                });
            },
        );
    }
    if !name.focused {
        field.request_focus();
        // A file's name is selected without its extension; a folder's or a new asset's whole.
        let selected = match &name.target {
            NameTarget::Rename { path, hidden: None } if path.is_file() => {
                name.text.rfind('.').filter(|&dot| dot > 0).map_or(name.text.chars().count(), |dot| name.text[..dot].chars().count())
            }
            _ => name.text.chars().count(),
        };
        if let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), field.id) {
            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(selected))));
            state.store(ui.ctx(), field.id);
        }
        name.focused = true;
        return;
    }
    if !field.lost_focus() {
        return;
    }
    let (enter, escape) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
    let text = name.text.trim().to_owned();
    match (&name.target, enter, escape) {
        (_, _, true) | (NameTarget::New { .. }, false, false) => *naming = None,
        (NameTarget::New { .. }, true, _) => outcome.action = Some(Action::Create(text)),
        (NameTarget::Rename { .. }, ..) => outcome.action = Some(Action::RenameTo(text)),
    }
}

/// Makes the asset being named, through `asset.create`, and opens what has an editor here: a
/// graph in the graph editor; a Rust script starts building. Returns the new file or folder, or
/// why it was not made.
fn create(editor: &mut Editor, naming: &Naming, name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Type a name first.".to_owned());
    }
    let NameTarget::New { kind, folder } = &naming.target else { return Err("this name is not for a new asset".to_owned()) };
    if *kind != AssetKind::Script && !bayleaf_engine::asset_cmds::valid_file_name(name) {
        return Err(r#"A name may have up to 100 characters and none of / \ : * ? " < > |."#.to_owned());
    }
    let made = editor.dispatch("asset.create", &json!({ "kind": kind, "folder": folder, "name": name }), None).ok_or_else(|| editor.status.clone())?;
    let path = PathBuf::from(made["path"].as_str().unwrap_or_default());
    match kind {
        AssetKind::Material | AssetKind::VisualScript => editor.open_graph(&path),
        AssetKind::Script => editor.build_script(&path),
        // An Input Map opens in the Inspector, where its actions are edited.
        AssetKind::InputMap | AssetKind::Lighting | AssetKind::PostProcessing | AssetKind::InterfaceStyle => {
            editor.inspect_file(path.clone());
        }
        AssetKind::Interface => editor.open_interface(&path),
        AssetKind::Folder | AssetKind::Scene => {}
    }
    Ok(path)
}

/// Breadcrumbs to the open folder on the left; view switch and a compact search on the right.
fn toolbar(ui: &mut egui::Ui, view: &mut ProjectView, outcome: &mut Outcome) {
    let t = theme::tokens(ui);
    ui.horizontal(|ui| {
        // A slim bar: list-row height, thin slider rail, small handle.
        let s = ui.spacing_mut();
        s.item_spacing.x = 2.0;
        s.interact_size.y = LIST_ROW;
        s.button_padding = egui::vec2(6.0, 1.0);
        s.slider_rail_height = 3.0;
        // New files are made in the project only.
        let can_create = view.root.is_some();
        ui.add_enabled_ui(can_create, |ui| {
            create_menu(ui, outcome);
        });
        // The first crumb names the top folder: Project or Engine.
        match view.top_of(&view.current).map(|(name, root)| (name, root.to_path_buf())) {
            Some((top, root)) => {
                let mut crumbs: Vec<PathBuf> = view.current.ancestors().take_while(|p| p.starts_with(&root)).map(Path::to_path_buf).collect();
                crumbs.reverse();
                for (i, crumb) in crumbs.iter().enumerate() {
                    if i > 0 {
                        ui.label(egui::RichText::new("/").color(t.text_muted));
                    }
                    let name = if i == 0 {
                        top.to_owned()
                    } else {
                        crumb.file_name().map_or_else(|| crumb.display().to_string(), |n| n.to_string_lossy().into_owned())
                    };
                    if ui.add(egui::Button::new(name).frame_when_inactive(false).small()).on_hover_text(crumb.display().to_string()).clicked() {
                        outcome.enter = Some(crumb.clone());
                    }
                }
            }
            None => {
                ui.label(egui::RichText::new(tl("Project")).small().color(t.text));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            toolbar_right(ui, view);
        });
    });
}

fn create_menu(ui: &mut egui::Ui, outcome: &mut Outcome) {
    let create = ui.menu_button("+", |ui| create_panel(ui, None, outcome));
    ui.ctx().accesskit_node_builder(create.response.id, |node| node.set_label("Create asset"));
    create.response.on_hover_text(tl("Create a folder, scene, material or script in the open folder"));
}

/// The kinds of asset that can be made, each with its icon and what it is. Choosing one starts
/// naming it in `folder` (the open folder when `None`); nothing is written before the name.
fn create_panel(ui: &mut egui::Ui, folder: Option<&Path>, outcome: &mut Outcome) {
    let t = theme::tokens(ui);
    // A fixed width: rows that grow would take all the room a menu offers.
    const WIDTH: f32 = 300.0;
    ui.set_width(WIDTH);
    ui.label(egui::RichText::new(tl("Create")).small().color(t.text_muted));
    for (kind, title, what, _) in CREATABLE {
        let icon = egui::Atom::paint(egui::vec2(22.0, 22.0), move |ui, args| paint_icon(ui.painter(), args.rect, kind_icon(kind), t));
        let mut job = egui::text::LayoutJob::default();
        job.append(title, 0.0, egui::TextFormat::simple(egui::TextStyle::Body.resolve(ui.style()), t.text));
        job.append("\n", 0.0, egui::TextFormat::default());
        job.append(what, 0.0, egui::TextFormat::simple(egui::TextStyle::Small.resolve(ui.style()), t.text_muted));
        let button = ui.add(egui::Button::new((icon, job, egui::Atom::grow())).frame_when_inactive(false).min_size(egui::vec2(WIDTH, 36.0)));
        // The name says what is made; the description follows.
        ui.ctx().accesskit_node_builder(button.id, |node| {
            node.set_label(crate::i18n::tf("New {title}", &[("title", title)]));
            node.set_description(what.to_owned());
        });
        if button.clicked() {
            outcome.action = Some(Action::StartNew(kind, folder.map(Path::to_path_buf)));
            ui.close();
        }
    }
}

/// View switch, icon size and search, laid out from the right.
fn toolbar_right(ui: &mut egui::Ui, view: &mut ProjectView) {
    ui.spacing_mut().item_spacing.x = 4.0;
    let search = ui.add_sized([SEARCH_WIDTH, LIST_ROW], egui::TextEdit::singleline(&mut view.filter).hint_text(tl("Search")).align(egui::Align2::LEFT_CENTER));
    ui.ctx().accesskit_node_builder(search.id, |node| node.set_label("Search project files"));
    // A short icon-size slider; Ctrl+mouse wheel over the files does the same.
    let mut size = view.icon();
    ui.spacing_mut().slider_width = SLIDER_WIDTH;
    let slider = ui.add_enabled(
        view.mode == ViewMode::Icons,
        egui::Slider::new(&mut size, ICON_MIN..=ICON_MAX)
            .show_value(false)
            .logarithmic(true)
            .handle_shape(egui::style::HandleShape::Rect { aspect_ratio: 0.5 }),
    );
    ui.ctx().accesskit_node_builder(slider.id, |node| node.set_label("Icon size"));
    if slider.on_hover_text(tl("Icon size (Ctrl + mouse wheel)")).changed() {
        view.icon_size = size;
    }
    for (mode, label) in [(ViewMode::List, "List"), (ViewMode::Icons, "Icons")] {
        if ui
            .add(egui::Button::selectable(view.mode == mode, label).small())
            .on_hover_text(crate::i18n::tf("Show files as {view}", &[("view", &tl(label).to_lowercase().to_string())]))
            .clicked()
        {
            view.mode = mode;
        }
    }
}

/// The two top folders: Project (open) and Engine (closed until opened).
fn top_folders(ui: &mut egui::Ui, view: &ProjectView, rows: &mut Vec<egui::Rect>, outcome: &mut Outcome) {
    let project = view.root.clone().unwrap_or_default();
    let tops = [("Project", Some(project), true), ("Engine", view.engine.clone(), false)];
    for (name, dir, open) in tops {
        let Some(dir) = dir else { continue };
        let at_top = view.current == dir;
        let id = ui.make_persistent_id(("project top folder", name));
        let hover = if dir.as_os_str().is_empty() { "No project folder: open or save a scene".to_owned() } else { dir.display().to_string() };
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open || view.current.starts_with(&dir) && !dir.as_os_str().is_empty())
            .show_header(ui, |ui| {
                let response = folder_row(ui, name, at_top).on_hover_text(hover);
                object_drop(ui, &response, &dir, outcome);
                folder_menu(&response, view, &dir, outcome);
                if response.clicked() {
                    outcome.enter = Some(dir.clone());
                }
                rows.push(response.rect);
            })
            .body(|ui| folder_tree(ui, view, &dir, rows, outcome));
    }
}

fn folder_tree(ui: &mut egui::Ui, view: &ProjectView, dir: &Path, rows: &mut Vec<egui::Rect>, outcome: &mut Outcome) {
    if dir.as_os_str().is_empty() {
        return;
    }
    // The engine's folder is listed only at the top, never inside the project.
    let folders = view.tree.get(dir).map_or(&[][..], Vec::as_slice).iter().filter(|e| e.kind == Kind::Folder && Some(&e.path) != view.engine.as_ref());
    for entry in folders {
        let has_subfolders = view.tree.get(&entry.path).is_some_and(|children| children.iter().any(|c| c.kind == Kind::Folder));
        let is_current = view.current == entry.path;
        if has_subfolders {
            let id = ui.make_persistent_id(("project folder", &entry.path));
            let open_by_default = view.current.starts_with(&entry.path);
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open_by_default)
                .show_header(ui, |ui| {
                    let response = folder_row(ui, &entry.name, is_current);
                    object_drop(ui, &response, &entry.path, outcome);
                    folder_menu(&response, view, &entry.path, outcome);
                    if response.clicked() {
                        outcome.enter = Some(entry.path.clone());
                    }
                    rows.push(response.rect);
                })
                .body(|ui| folder_tree(ui, view, &entry.path, rows, outcome));
        } else {
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                let response = folder_row(ui, &entry.name, is_current);
                object_drop(ui, &response, &entry.path, outcome);
                if response.clicked() {
                    outcome.enter = Some(entry.path.clone());
                }
                rows.push(response.rect);
            });
        }
    }
}

/// An object dragged from the hierarchy onto a folder (a row, a tile, or the open folder's free
/// space) becomes a prefab in it: the folder is ringed while the object is over it.
fn object_drop(ui: &egui::Ui, response: &egui::Response, folder: &Path, outcome: &mut Outcome) {
    // Files from the system come into the folder the same way.
    if let Some(files) = crate::file_drop::take(ui, response.interact_rect) {
        outcome.action = Some(Action::Import(files, folder.to_path_buf()));
        return;
    }
    let Some(object) = crate::hierarchy::dragged_object(ui.ctx()) else { return };
    // The visible part of the folder; egui does not count it as under the pointer while another
    // widget is being dragged.
    let area = response.interact_rect;
    if !ui.ctx().pointer_interact_pos().is_some_and(|p| area.contains(p)) {
        return;
    }
    let t = theme::tokens(ui);
    ui.painter().rect_stroke(response.rect, theme::RADIUS, egui::Stroke::new(2.0, t.focus), egui::StrokeKind::Inside);
    if ui.input(|i| i.pointer.any_released()) {
        egui::DragAndDrop::clear_payload(ui.ctx());
        outcome.action = Some(Action::MakePrefab(object, folder.to_path_buf()));
    }
}

/// A prefab file for an object named `name` in `folder` that is not there yet: `Crate.bprefab`,
/// then `Crate 2.bprefab`, … Characters file names cannot hold become `_`.
fn new_prefab_path(folder: &Path, name: &str) -> PathBuf {
    let extension = bayleaf_engine::prefab_cmds::PREFAB_EXTENSION;
    let clean: String =
        name.trim().chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { '_' } else { c }).collect();
    let stem = if clean.trim_matches('.').is_empty() { "Prefab".to_owned() } else { clean };
    (1..)
        .map(|n| folder.join(if n == 1 { format!("{stem}.{extension}") } else { format!("{stem} {n}.{extension}") }))
        .find(|path| !path.exists())
        .unwrap_or_else(|| folder.join(format!("{stem}.{extension}")))
}

fn folder_row(ui: &mut egui::Ui, name: &str, selected: bool) -> egui::Response {
    let width = ui.available_width();
    ui.add(egui::Button::selectable(selected, (name, egui::Atom::grow())).truncate().min_size(egui::vec2(width, LIST_ROW)))
}

/// The open folder's entries, or every file matching the search.
/// Right-clicking the space between them opens the folder's menu.
fn files(ui: &mut egui::Ui, view: &ProjectView, naming: &mut Option<Naming>, selected_object: bool, thumbnails: &mut Thumbnails, outcome: &mut Outcome) {
    // Sensed before the files, so a file under the pointer takes the click instead.
    let background = ui.interact(ui.max_rect(), egui::Id::unique("project files background"), egui::Sense::click());
    background.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Group, true, "Folder contents"));
    if view.focus_contents {
        background.request_focus();
    }
    outcome.focused |= background.has_focus();
    file_entries(ui, view, naming, selected_object, thumbnails, outcome);
    // After the folder tiles, which take a drop onto them first.
    if !view.current.as_os_str().is_empty() {
        object_drop(ui, &background, &view.current, outcome);
    }
    if background.clicked() {
        outcome.deselect = true;
    }
    if !view.current.as_os_str().is_empty() {
        folder_menu(&background, view, &view.current, outcome);
    }
}

fn file_entries(ui: &mut egui::Ui, view: &ProjectView, naming: &mut Option<Naming>, selected_object: bool, thumbnails: &mut Thumbnails, outcome: &mut Outcome) {
    if view.current.as_os_str().is_empty() {
        inspector::empty_state(ui, "No project folder", "Open or save a scene to see its folder here. The engine's own files are under Engine, on the left.");
        return;
    }
    let filter = view.filter.trim().to_lowercase();
    // The engine's folder is never a tile: it is chosen from the folder list only.
    // A Rust script is one entry: its built module beside its folder is not listed again.
    let shown = |e: &&Entry| Some(&e.path) != view.engine.as_ref() && !view.is_built_module(e);
    let entries: Vec<&Entry> = if filter.is_empty() {
        view.tree.get(&view.current).map(|v| v.iter().filter(shown).collect()).unwrap_or_default()
    } else {
        // Search the top folder the open folder belongs to: the project, or the engine.
        let scope = view.top_of(&view.current).map(|(_, root)| root.to_path_buf()).unwrap_or_else(|| view.current.clone());
        let in_scope = |e: &&Entry| e.path.starts_with(&scope);
        let mut found: Vec<&Entry> =
            view.tree.values().flatten().filter(|e| e.kind != Kind::Folder && e.name.to_lowercase().contains(&filter)).filter(in_scope).filter(shown).collect();
        found.sort_by(|a, b| a.path.cmp(&b.path));
        found.dedup_by(|a, b| a.path == b.path);
        found
    };
    // A new asset being named shows first in the folder it goes into.
    let new_here = match naming.as_ref().map(|n| &n.target) {
        Some(NameTarget::New { kind, folder }) if *folder == view.current && filter.is_empty() => Some(*kind),
        _ => None,
    };
    let renamed: Option<PathBuf> = match naming.as_ref().map(|n| &n.target) {
        Some(NameTarget::Rename { path, .. }) => Some(path.clone()),
        _ => None,
    };
    let renaming = |entry: &Entry| renamed.as_ref() == Some(&entry.path);
    if entries.is_empty() && new_here.is_none() {
        let (title, hint) = if filter.is_empty() {
            ("Empty folder", "Save files into this folder to see them here.")
        } else {
            ("No matches", "No file name contains the search text.")
        };
        inspector::empty_state(ui, title, hint);
        return;
    }
    match view.mode {
        ViewMode::Icons => {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                if let Some(kind) = new_here {
                    naming_tile(ui, kind_icon(kind), view.icon(), naming, outcome);
                }
                for entry in entries {
                    if renaming(entry) {
                        naming_tile(ui, entry.kind, view.icon(), naming, outcome);
                        continue;
                    }
                    let picture = picture(ui, thumbnails, entry);
                    let response = tile(ui, entry, &picture, view.selected.as_deref() == Some(entry.path.as_path()), view.icon(), view.hide_extensions);
                    interact(ui, &response, view, entry, selected_object, outcome);
                }
            });
        }
        ViewMode::List => {
            theme::dense(ui);
            let slot = ui.painter().add(egui::Shape::Noop);
            let full_width = ui.max_rect().x_range();
            let mut rows = Vec::new();
            if let Some(kind) = new_here {
                rows.push(naming_row(ui, kind_icon(kind), naming, outcome));
            }
            for entry in entries {
                if renaming(entry) {
                    rows.push(naming_row(ui, entry.kind, naming, outcome));
                    continue;
                }
                let picture = picture(ui, thumbnails, entry);
                let response = list_row(ui, entry, &picture, view.selected.as_deref() == Some(entry.path.as_path()), view.hide_extensions);
                rows.push(response.rect);
                interact(ui, &response, view, entry, selected_object, outcome);
            }
            theme::stripes(ui, slot, rows.into_iter(), full_width);
        }
    }
}

/// A tile whose name is being typed: the icon, and the name field where the name goes.
fn naming_tile(ui: &mut egui::Ui, kind: Kind, icon_size: f32, naming: &mut Option<Naming>, outcome: &mut Outcome) {
    let t = theme::tokens(ui);
    let tile_size = egui::vec2((icon_size + 36.0).max(72.0), icon_size + 38.0);
    ui.allocate_ui_with_layout(tile_size, egui::Layout::top_down(egui::Align::Center), |ui| {
        ui.set_min_size(tile_size);
        ui.painter().rect_filled(ui.max_rect(), theme::RADIUS, t.surface_hover);
        let (icon, _) = ui.allocate_exact_size(egui::vec2(icon_size, icon_size + 6.0), egui::Sense::hover());
        paint_icon(ui.painter(), egui::Rect::from_min_size(icon.min + egui::vec2(0.0, 6.0), egui::vec2(icon_size, icon_size)), kind, t);
        name_field(ui, naming, tile_size.x - 4.0, outcome);
    });
}

/// A list row whose name is being typed: the icon, then the name field.
fn naming_row(ui: &mut egui::Ui, kind: Kind, naming: &mut Option<Naming>, outcome: &mut Outcome) -> egui::Rect {
    let t = theme::tokens(ui);
    ui.horizontal(|ui| {
        let (icon, _) = ui.allocate_exact_size(egui::vec2(14.0, LIST_ROW), egui::Sense::hover());
        paint_icon(ui.painter(), egui::Rect::from_center_size(icon.center(), egui::vec2(14.0, 14.0)), kind, t);
        let width = ui.available_width().min(320.0);
        name_field(ui, naming, width, outcome);
    })
    .response
    .rect
}

/// A material's picture, asked for only while its place in the list is on screen.
fn picture(ui: &egui::Ui, thumbnails: &mut Thumbnails, entry: &Entry) -> Thumb {
    let visible = ui.is_rect_visible(egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width().max(1.0), 160.0)));
    if entry.kind == Kind::Material && visible { thumbnails.get(&entry.path) } else { Thumb::Waiting }
}

/// What a file is called to a screen reader: its name, its kind, and a material that does not compile.
fn spoken(entry: &Entry, name: &str, picture: &Thumb) -> String {
    let kind = tl(entry.kind.label());
    match picture {
        Thumb::Failed(_) => crate::i18n::tf("{name}, {kind}, does not compile", &[("name", name), ("kind", kind)]),
        _ => format!("{name}, {kind}"),
    }
}

/// The picture where there is one, else the kind's icon; with a warning mark when it does not compile.
fn paint_picture(painter: &egui::Painter, rect: egui::Rect, kind: Kind, picture: &Thumb, t: &Tokens) {
    match picture {
        Thumb::Ready(texture) => {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.add(egui::epaint::RectShape::filled(rect, theme::RADIUS, egui::Color32::WHITE).with_texture(*texture, uv));
        }
        Thumb::Failed(_) => {
            paint_icon(painter, rect, kind, t);
            crate::thumbnails::paint_warning(painter, rect, t);
        }
        Thumb::Waiting | Thumb::Unavailable => paint_icon(painter, rect, kind, t),
    }
}

/// One file or folder in the icon view: drawn icon (a material's picture), name below.
fn tile(ui: &mut egui::Ui, entry: &Entry, picture: &Thumb, selected: bool, icon_size: f32, hide: bool) -> egui::Response {
    let name = entry.shown(hide);
    let t = theme::tokens(ui);
    // Room for the icon, a little padding, and two lines of name below.
    let tile_size = egui::vec2((icon_size + 36.0).max(72.0), icon_size + 38.0);
    let (rect, response) = ui.allocate_exact_size(tile_size, egui::Sense::click_and_drag());
    let label = spoken(entry, name, picture);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &label));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let fill = if selected {
            Some(t.accent)
        } else if response.hovered() {
            Some(t.surface_hover)
        } else {
            None
        };
        if let Some(fill) = fill {
            painter.rect_filled(rect, theme::RADIUS, fill);
        }
        if response.has_focus() {
            painter.rect_stroke(rect, theme::RADIUS, egui::Stroke::new(1.5, t.focus), egui::StrokeKind::Inside);
        }
        let icon = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 6.0 + icon_size / 2.0), egui::vec2(icon_size, icon_size));
        paint_picture(painter, icon, entry.kind, picture, t);
        let color = if selected { t.on_accent } else { t.text };
        let mut job = egui::text::LayoutJob::simple(name.to_owned(), egui::TextStyle::Small.resolve(ui.style()), color, tile_size.x - 6.0);
        job.wrap.max_rows = 2;
        job.wrap.break_anywhere = true;
        job.halign = egui::Align::Center;
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        painter.galley(egui::pos2(rect.center().x, icon.bottom() + 4.0), galley, color);
    }
    hover(response, entry, picture)
}

/// The file's full name on hover, and why a material does not compile.
fn hover(response: egui::Response, entry: &Entry, picture: &Thumb) -> egui::Response {
    match picture {
        Thumb::Failed(error) => response.on_hover_text(format!("{}\n{}", entry.name, crate::i18n::tf("Does not compile: {error}", &[("error", error)]))),
        _ => response.on_hover_text(&entry.name),
    }
}

/// One file or folder in the list view: small icon (a material's picture), name, kind.
fn list_row(ui: &mut egui::Ui, entry: &Entry, picture: &Thumb, selected: bool, hide: bool) -> egui::Response {
    let name = entry.shown(hide);
    let t = theme::tokens(ui);
    let width = ui.available_width();
    let kind = entry.kind;
    // The icon is an atom of the button, so it takes its own space before the name.
    let shown = picture.clone();
    let icon = egui::Atom::paint(egui::vec2(14.0, 14.0), move |ui, args| paint_picture(ui.painter(), args.rect, kind, &shown, t));
    let button = egui::Button::selectable(selected, (icon, name, egui::Atom::grow(), egui::RichText::new(entry.kind.label()).small().color(t.text_muted)))
        .truncate()
        .min_size(egui::vec2(width, LIST_ROW))
        .sense(egui::Sense::click_and_drag());
    let response = ui.add(button);
    let label = spoken(entry, name, picture);
    ui.ctx().accesskit_node_builder(response.id, |node| node.set_label(label));
    hover(response, entry, picture)
}

/// Clicks, double-clicks, dragging and the context menu, shared by tiles and list rows.
fn interact(ui: &egui::Ui, response: &egui::Response, view: &ProjectView, entry: &Entry, selected_object: bool, outcome: &mut Outcome) {
    if entry.kind == Kind::Folder {
        object_drop(ui, response, &entry.path, outcome);
    }
    // Project files are referred to relative to the project; engine files by their full path.
    let relative = match &view.root {
        Some(root) if entry.path.starts_with(root) => relative_path(root, &entry.path),
        _ => relative_path(Path::new(""), &entry.path),
    };
    if response.clicked() || (response.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed())) {
        outcome.select = Some(entry.path.clone());
    }
    if response.clicked() {
        outcome.inspect = Some(entry.path.clone());
        // Keyboard keys (Delete) then act on it.
        response.request_focus();
    }
    outcome.focused |= response.has_focus();
    if response.has_focus() && view.is_in_project(&entry.path) && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
        outcome.action = Some(Action::AskDelete(entry.path.clone()));
    }
    if response.has_focus() && view.is_in_project(&entry.path) && ui.input(|i| i.key_pressed(egui::Key::F2)) {
        outcome.action = Some(Action::StartRename(entry.path.clone()));
    }
    if response.double_clicked() {
        outcome.action = default_action(entry);
        if entry.kind == Kind::Folder {
            outcome.enter = Some(entry.path.clone());
        }
    }
    // A Rust script is dragged as its module, and built first when it has none yet.
    let module = (entry.kind == Kind::ScriptSource).then(|| format!("{relative}.wasm"));
    if entry.kind != Kind::Folder {
        if response.drag_started_by(egui::PointerButton::Primary) {
            egui::DragAndDrop::set_payload(ui.ctx(), DraggedAsset { relative: module.clone().unwrap_or_else(|| relative.clone()) });
            if module.is_some() && !entry.path.with_extension("wasm").is_file() {
                outcome.action = Some(Action::BuildScript(entry.path.clone()));
            }
        }
        if response.dragged_by(egui::PointerButton::Primary)
            && let Some(pointer) = ui.ctx().pointer_interact_pos()
        {
            drag_ghost(ui, entry.shown(view.hide_extensions), pointer);
        }
    }
    response.context_menu(|ui| {
        if entry.kind == Kind::ScriptSource && ui.button(tl("Build Script")).on_hover_text(tl("Build it to WebAssembly, next to its folder")).clicked() {
            outcome.action = Some(Action::BuildScript(entry.path.clone()));
            ui.close();
        }
        if matches!(entry.kind, Kind::Folder | Kind::ScriptSource) && ui.button(tl("Open Folder")).clicked() {
            outcome.enter = Some(entry.path.clone());
            ui.close();
        }
        if entry.kind == Kind::Scene && ui.button(tl("Open Scene")).clicked() {
            outcome.action = Some(Action::OpenScene(entry.path.clone()));
            ui.close();
        }
        if entry.kind == Kind::Prefab && ui.button(tl("Place in Scene")).on_hover_text(tl("Adds a linked copy after the selected object")).clicked() {
            outcome.action = Some(Action::PlacePrefab(entry.path.clone()));
            ui.close();
        }
        if entry.kind == Kind::Project && ui.button(tl("Open Project")).clicked() {
            outcome.action = Some(Action::OpenScene(entry.path.clone()));
            ui.close();
        }
        if matches!(entry.kind, Kind::Material | Kind::VisualScript) && ui.button(tl("Open in Graph Editor")).clicked() {
            outcome.action = Some(Action::OpenGraph(entry.path.clone()));
            ui.close();
        }
        if matches!(entry.kind, Kind::Script | Kind::VisualScript | Kind::ScriptSource) {
            let button = ui.add_enabled(selected_object, egui::Button::new(tl("Add to Selected Object")));
            if button.on_disabled_hover_text(tl("Select an object first")).clicked() {
                outcome.action = Some(Action::AddScript(module.clone().unwrap_or_else(|| relative.clone())));
                ui.close();
            }
        }
        if entry.kind != Kind::Folder && ui.button(tl("Open with System App")).on_hover_text(tl("Open it with the program your system uses for it")).clicked() {
            outcome.action = Some(Action::OpenExternal(source_file(entry).unwrap_or_else(|| entry.path.clone())));
            ui.close();
        }
        if ui.button(tl("Show in File Explorer")).clicked() {
            outcome.action = Some(Action::Reveal(entry.path.clone()));
            ui.close();
        }
        if ui.button(tl("Copy Relative Path")).clicked() {
            ui.ctx().copy_text(relative.clone());
            ui.close();
        }
        if entry.kind == Kind::Folder && view.is_in_project(&entry.path) {
            ui.separator();
            ui.menu_button(tl("Create"), |ui| create_panel(ui, Some(&entry.path), outcome));
        }
        if view.is_in_project(&entry.path) {
            ui.separator();
            if ui.add(egui::Button::new(tl("Rename")).shortcut_text("F2")).clicked() {
                outcome.action = Some(Action::StartRename(entry.path.clone()));
                ui.close();
            }
            if ui.button(tl("Delete…")).on_hover_text(tl("Move it to the Recycle Bin (Delete)")).clicked() {
                outcome.action = Some(Action::AskDelete(entry.path.clone()));
                ui.close();
            }
        }
    });
}

/// A folder's context menu: create in it (inside the project), show it in the system's file
/// manager, and read the folders again.
fn folder_menu(response: &egui::Response, view: &ProjectView, folder: &Path, outcome: &mut Outcome) {
    response.context_menu(|ui| {
        if view.is_in_project(folder) {
            ui.menu_button(tl("Create"), |ui| create_panel(ui, Some(folder), outcome));
            ui.separator();
        }
        if ui.button(tl("Show in File Explorer")).clicked() {
            outcome.action = Some(Action::Reveal(folder.to_path_buf()));
            ui.close();
        }
        if ui.button(tl("Refresh")).on_hover_text(tl("Read the folders again now")).clicked() {
            outcome.action = Some(Action::Refresh);
            ui.close();
        }
    });
}

/// What double-clicking a file does: the editor's own tool for what it has one for, else the
/// program the system uses for it. Folders open in the panel instead.
fn default_action(entry: &Entry) -> Option<Action> {
    let path = entry.path.clone();
    match entry.kind {
        // A click already shows an Input Map in the Inspector, where it is edited.
        Kind::Folder | Kind::InputMap | Kind::Lighting | Kind::PostProcessing | Kind::InterfaceStyle => None,
        Kind::Interface => Some(Action::OpenInterface(path)),
        Kind::Scene | Kind::Project => Some(Action::OpenScene(path)),
        Kind::Material | Kind::VisualScript => Some(Action::OpenGraph(path)),
        Kind::Prefab => Some(Action::OpenPrefab(path)),
        // A Rust script: its source, in the code editor.
        Kind::ScriptSource | Kind::Script => source_file(entry).map(Action::OpenExternal),
        Kind::Model | Kind::Image | Kind::Sound | Kind::Other => Some(Action::OpenExternal(path)),
    }
}

/// The source file of a Rust script: `src/lib.rs` of its crate folder, which sits beside a built
/// `.wasm` under the same name.
fn source_file(entry: &Entry) -> Option<PathBuf> {
    let crate_dir = match entry.kind {
        Kind::ScriptSource => entry.path.clone(),
        Kind::Script => entry.path.with_extension(""),
        _ => return None,
    };
    let source = crate_dir.join("src").join("lib.rs");
    source.is_file().then_some(source)
}

/// Opens `path` with the program the system uses for it, or with `reveal`, shows it in the
/// system's file manager.
fn system_open(path: &Path, reveal: bool) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("explorer");
        if reveal {
            command.arg(format!("/select,{}", path.display()));
        } else {
            command.arg(path);
        }
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        if reveal {
            command.arg("-R");
        }
        command.arg(path);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(if reveal { path.parent().unwrap_or(path) } else { path });
        command
    };
    #[cfg(any(target_os = "windows", unix))]
    return command.spawn().map(|_| ());
    #[cfg(not(any(target_os = "windows", unix)))]
    let _ = (path, reveal);
    #[cfg(not(any(target_os = "windows", unix)))]
    return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "opening files with other programs is not supported here"));
}

fn drag_ghost(ui: &egui::Ui, name: &str, pointer: egui::Pos2) {
    let t = theme::tokens(ui);
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("project drag ghost")));
    let galley = painter.layout_no_wrap(name.to_owned(), egui::TextStyle::Body.resolve(ui.style()), t.on_accent);
    let rect = egui::Rect::from_min_size(pointer + egui::vec2(12.0, 4.0), galley.size() + egui::vec2(16.0, 8.0));
    painter.rect_filled(rect, theme::RADIUS, t.accent);
    painter.galley(rect.min + egui::vec2(8.0, 4.0), galley, t.on_accent);
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
}

/// Simple drawn icons, one per kind; no image files needed.
fn paint_icon(painter: &egui::Painter, rect: egui::Rect, kind: Kind, t: &Tokens) {
    let s = rect.width();
    let stroke = |color| egui::Stroke::new((s / 20.0).max(1.0), color);
    let at = |x: f32, y: f32| rect.min + egui::vec2(x * s, y * s);
    match kind {
        Kind::Folder => {
            let tab = egui::Rect::from_min_max(at(0.08, 0.18), at(0.45, 0.32));
            let body = egui::Rect::from_min_max(at(0.08, 0.28), at(0.92, 0.82));
            painter.rect_filled(tab, s * 0.04, t.warning.gamma_multiply(0.8));
            painter.rect_filled(body, s * 0.06, t.warning);
        }
        Kind::Scene | Kind::Project | Kind::Prefab | Kind::Model => {
            // An outlined cube seen from above and in front; filled for a project file, in the
            // accent colour for a prefab.
            let filled = kind != Kind::Scene;
            let color = match kind {
                Kind::Prefab => t.pin_vector,
                Kind::Model => t.pin_color,
                _ => t.brand,
            };
            let (top, left, right, mid, bottom, lb, rb) =
                (at(0.5, 0.12), at(0.14, 0.32), at(0.86, 0.32), at(0.5, 0.52), at(0.5, 0.9), at(0.14, 0.7), at(0.86, 0.7));
            let side = if filled { color.gamma_multiply(0.6) } else { egui::Color32::TRANSPARENT };
            painter.add(egui::Shape::convex_polygon(vec![top, right, mid, left], color.gamma_multiply(if filled { 0.9 } else { 0.35 }), stroke(color)));
            painter.add(egui::Shape::convex_polygon(vec![left, mid, bottom, lb], side, stroke(color)));
            painter.add(egui::Shape::convex_polygon(vec![mid, right, rb, bottom], side, stroke(color)));
        }
        Kind::Material => {
            // A shaded ball with a highlight.
            let center = rect.center();
            painter.circle(center, s * 0.36, t.surface_active, stroke(t.pin_color));
            painter.circle_filled(center + egui::vec2(-s * 0.1, -s * 0.1), s * 0.14, t.pin_color);
        }
        Kind::Script
        | Kind::ScriptSource
        | Kind::VisualScript
        | Kind::InputMap
        | Kind::Lighting
        | Kind::PostProcessing
        | Kind::Interface
        | Kind::InterfaceStyle
        | Kind::Image
        | Kind::Sound
        | Kind::Other => {
            // A page with a folded corner.
            let fold = s * 0.22;
            let (l, r, top, bottom) = (rect.left() + s * 0.18, rect.right() - s * 0.18, rect.top() + s * 0.08, rect.bottom() - s * 0.08);
            let page = vec![egui::pos2(l, top), egui::pos2(r - fold, top), egui::pos2(r, top + fold), egui::pos2(r, bottom), egui::pos2(l, bottom)];
            let accent = match kind {
                Kind::Script | Kind::ScriptSource => t.axis_z,
                Kind::VisualScript => t.pin_any,
                Kind::InputMap => t.pin_bool,
                Kind::Lighting => t.warning,
                Kind::PostProcessing => t.pin_vector,
                Kind::Interface | Kind::InterfaceStyle => t.pin_object,
                Kind::Image => t.axis_y,
                Kind::Sound => t.pin_text,
                _ => t.text_muted,
            };
            painter.add(egui::Shape::convex_polygon(page, t.surface_active, stroke(accent)));
            painter.add(egui::Shape::line(vec![egui::pos2(r - fold, top), egui::pos2(r - fold, top + fold), egui::pos2(r, top + fold)], stroke(accent)));
            let inner = egui::Rect::from_min_max(egui::pos2(l + s * 0.1, top + fold + s * 0.06), egui::pos2(r - s * 0.1, bottom - s * 0.1));
            match kind {
                Kind::VisualScript if s >= 24.0 => {
                    // Two linked nodes.
                    let a = egui::Rect::from_min_size(inner.min, inner.size() * egui::vec2(0.4, 0.35));
                    let b = egui::Rect::from_min_size(inner.min + inner.size() * egui::vec2(0.6, 0.65), inner.size() * egui::vec2(0.4, 0.35));
                    painter.rect_filled(a, 2.0, accent);
                    painter.rect_filled(b, 2.0, accent);
                    painter.line_segment([a.right_center(), b.left_center()], stroke(accent));
                }
                Kind::InputMap if s >= 24.0 => {
                    // Four keys.
                    let key = inner.size() * egui::vec2(0.42, 0.36);
                    for (x, y) in [(0.0, 0.0), (0.58, 0.0), (0.0, 0.64), (0.58, 0.64)] {
                        let min = inner.min + inner.size() * egui::vec2(x, y);
                        painter.rect_stroke(egui::Rect::from_min_size(min, key), 2.0, stroke(accent), egui::StrokeKind::Inside);
                    }
                }
                Kind::Interface if s >= 24.0 => {
                    // A screen with a title bar and a button.
                    let bar = egui::Rect::from_min_size(inner.min, egui::vec2(inner.width(), inner.height() * 0.22));
                    painter.rect_filled(bar, 1.0, accent);
                    let button = egui::Rect::from_center_size(inner.center() + egui::vec2(0.0, inner.height() * 0.2), inner.size() * egui::vec2(0.55, 0.24));
                    painter.rect_stroke(button, 2.0, stroke(accent), egui::StrokeKind::Inside);
                }
                Kind::InterfaceStyle if s >= 24.0 => {
                    // Lines of rules, indented.
                    for (i, (x, w)) in [(0.0, 0.7), (0.2, 0.6), (0.2, 0.45), (0.0, 0.7)].into_iter().enumerate() {
                        let y = inner.top() + inner.height() * (0.15 + 0.23 * i as f32);
                        painter.line_segment(
                            [egui::pos2(inner.left() + inner.width() * x, y), egui::pos2(inner.left() + inner.width() * (x + w), y)],
                            stroke(accent),
                        );
                    }
                }
                Kind::PostProcessing if s >= 24.0 => {
                    // A lens: a ring with a glint.
                    let center = inner.center();
                    let radius = inner.width().min(inner.height()) * 0.36;
                    painter.circle_stroke(center, radius, stroke(accent));
                    painter.circle_filled(center + egui::vec2(-radius * 0.35, -radius * 0.35), radius * 0.25, accent);
                }
                Kind::Lighting if s >= 24.0 => {
                    // A sun: a disc with rays.
                    let center = inner.center();
                    let radius = inner.width().min(inner.height()) * 0.22;
                    painter.circle_filled(center, radius, accent);
                    for i in 0..8 {
                        let angle = i as f32 * std::f32::consts::TAU / 8.0;
                        let dir = egui::vec2(angle.cos(), angle.sin());
                        painter.line_segment([center + dir * radius * 1.4, center + dir * radius * 2.0], stroke(accent));
                    }
                }
                Kind::Script if s >= 24.0 => {
                    painter.text(inner.center(), egui::Align2::CENTER_CENTER, "{ }", egui::FontId::monospace(s * 0.24), accent);
                }
                Kind::ScriptSource if s >= 24.0 => {
                    painter.text(inner.center(), egui::Align2::CENTER_CENTER, "rs", egui::FontId::monospace(s * 0.24), accent);
                }
                Kind::Image => {
                    painter.add(egui::Shape::convex_polygon(
                        vec![inner.left_bottom(), inner.center_top() + egui::vec2(-s * 0.05, s * 0.1), inner.right_bottom()],
                        accent,
                        egui::Stroke::NONE,
                    ));
                    painter.circle_filled(inner.right_top() + egui::vec2(-s * 0.06, s * 0.06), s * 0.05, t.warning);
                }
                Kind::Sound => {
                    // A sound wave: bars of different heights.
                    let bars = [0.35, 0.8, 0.55, 1.0, 0.45];
                    let width = inner.width() / bars.len() as f32;
                    for (i, height) in bars.into_iter().enumerate() {
                        let x = inner.left() + width * (i as f32 + 0.5);
                        let half = inner.height() * height * 0.5;
                        painter.line_segment([egui::pos2(x, inner.center().y - half), egui::pos2(x, inner.center().y + half)], stroke(accent));
                    }
                }
                Kind::Other if s >= 24.0 => {
                    for i in 0..3 {
                        let y = inner.top() + inner.height() * (0.2 + 0.3 * i as f32);
                        painter.line_segment([egui::pos2(inner.left(), y), egui::pos2(inner.right(), y)], stroke(accent));
                    }
                }
                _ => {}
            }
        }
    }
}

/// The project folder, the open scene's folder, or the folder the editor runs in.
fn project_root(editor: &Editor, engine: Option<&Path>) -> Option<PathBuf> {
    if let Some(dir) = editor.session.project_dir() {
        return Some(dir.to_path_buf());
    }
    let scene_folder = editor.session.scene_path().and_then(Path::parent).map(Path::to_path_buf);
    let folder = scene_folder.filter(|f| !f.as_os_str().is_empty()).or_else(|| std::env::current_dir().ok())?;
    project_folder(folder, engine)
}

/// `folder` as the project, unless it is the engine's folder or holds it: then there is no project,
/// so the engine's files are never shown as the project's.
fn project_folder(folder: PathBuf, engine: Option<&Path>) -> Option<PathBuf> {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    match engine {
        Some(engine) if canonical(engine).starts_with(canonical(&folder)) => None,
        _ => Some(folder),
    }
}

/// The engine's own folder: `BAYLEAF_ENGINE_DIR` when set; else, from the editor program, an
/// `engine` folder beside it (an installed engine) or the source checkout it was built in.
fn engine_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("BAYLEAF_ENGINE_DIR").map(PathBuf::from).filter(|d| d.is_dir()) {
        return Some(dir);
    }
    let exe = std::env::current_exe().ok()?;
    let beside = exe.parent()?.join("engine");
    if beside.is_dir() {
        return Some(beside);
    }
    exe.ancestors().find(|dir| dir.join("crates").join("engine").join("Cargo.toml").is_file()).map(Path::to_path_buf)
}

fn relative_path(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

impl ProjectView {
    /// Reads the project folder and the engine's folder again, each with its own entry budget.
    fn rescan(&mut self, root: Option<PathBuf>) {
        self.tree.clear();
        self.truncated = false;
        for top in [root.clone(), self.engine.clone()].into_iter().flatten() {
            self.scan(&top);
        }
        // A new project folder (another scene opened) keeps the open folder when it is still listed,
        // so opening a file never jumps back to the top.
        if self.root != root && !self.tree.contains_key(&self.current) {
            self.current = root.clone().unwrap_or_default();
            self.selected = None;
        }
        self.root = root;
        self.scanned = Some(Instant::now());
    }

    fn scan(&mut self, top: &Path) {
        let mut count = 0;
        let mut truncated = false;
        let mut pending = vec![top.to_path_buf()];
        while let Some(dir) = pending.pop() {
            let Ok(read) = std::fs::read_dir(&dir) else { continue };
            let mut entries: Vec<Entry> = read
                .flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
                    if name.starts_with('.') || (is_dir && SKIPPED.contains(&name.as_str())) {
                        return None;
                    }
                    let path = e.path();
                    let kind = match (is_dir, is_dir && path.join("Cargo.toml").is_file()) {
                        (_, true) => Kind::ScriptSource,
                        (true, false) => Kind::Folder,
                        _ => Kind::of(&path),
                    };
                    // The project file is the project itself, opened from the File menu; it is not listed.
                    if kind == Kind::Project {
                        return None;
                    }
                    Some(Entry { path, name, kind })
                })
                .collect();
            entries.sort_by(|a, b| (a.kind != Kind::Folder).cmp(&(b.kind != Kind::Folder)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
            count += entries.len();
            if count > MAX_ENTRIES {
                truncated = true;
                entries.truncate(entries.len().saturating_sub(count - MAX_ENTRIES));
            }
            pending.extend(entries.iter().filter(|e| e.kind == Kind::Folder).map(|e| e.path.clone()));
            self.tree.insert(dir, entries);
            if truncated {
                break;
            }
        }
        self.truncated |= truncated;
    }
}

#[cfg(test)]
mod tests {
    use super::{Kind, ProjectView, project_folder, relative_path};
    use std::path::Path;

    #[test]
    fn scans_folders_first_and_skips_hidden_and_build_folders_and_the_project_file() {
        let dir = std::env::temp_dir().join(format!("bayleaf-project-{}", std::process::id()));
        for sub in ["scripts", "target", ".git"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        std::fs::write(dir.join("a.bscene"), "{}").unwrap();
        std::fs::write(dir.join("game.bproject"), "{}").unwrap();
        std::fs::write(dir.join("scripts").join("spin.wasm"), [0u8]).unwrap();
        let mut view = ProjectView::default();
        view.rescan(Some(dir.clone()));
        let top: Vec<_> = view.tree[&dir].iter().map(|e| (e.name.as_str(), e.kind)).collect();
        assert_eq!(top, [("scripts", Kind::Folder), ("a.bscene", Kind::Scene)]);
        assert_eq!(view.tree[&dir.join("scripts")][0].kind, Kind::Script);
        assert_eq!(view.current, dir, "a new project opens at its root");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn opening_another_project_keeps_an_open_folder_that_is_still_listed() {
        let base = std::env::temp_dir().join(format!("bayleaf-keep-folder-{}", std::process::id()));
        let (first, second, engine) = (base.join("first"), base.join("second"), base.join("engine"));
        for dir in [&first, &second, &engine.join("examples")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let mut view = ProjectView { engine: Some(engine.clone()), ..ProjectView::default() };
        view.rescan(Some(first.clone()));
        view.current = engine.join("examples");
        // A scene opened from the engine's examples: the project changes, the open folder stays.
        view.rescan(Some(second.clone()));
        assert_eq!(view.current, engine.join("examples"));
        // A folder that is no longer listed goes back to the new project's top.
        view.current = first.clone();
        view.rescan(Some(second.clone()));
        view.rescan(Some(base.join("missing")));
        assert_eq!(view.current, base.join("missing"));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn the_engine_folder_and_folders_holding_it_are_never_the_project() {
        let engine = std::env::temp_dir().join(format!("bayleaf-engine-root-{}", std::process::id()));
        let demo = engine.join("examples").join("demo");
        std::fs::create_dir_all(&demo).unwrap();
        assert_eq!(project_folder(engine.clone(), Some(&engine)), None, "running from the engine's folder");
        assert_eq!(project_folder(std::env::temp_dir(), Some(&engine)), None, "a folder that holds the engine");
        assert_eq!(project_folder(demo.clone(), Some(&engine)), Some(demo.clone()), "a project inside the engine's examples");
        assert_eq!(project_folder(demo.clone(), None), Some(demo));
        std::fs::remove_dir_all(&engine).ok();
    }

    #[test]
    fn relative_paths_use_forward_slashes() {
        let root = Path::new("project");
        assert_eq!(relative_path(root, &root.join("scripts").join("spin.wasm")), "scripts/spin.wasm");
    }
}
