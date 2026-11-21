//! Project panel: the game's files under **Project** (the project folder, the open scene's folder, or the folder the
//! editor started in) and the engine's own files under **Engine**, so the two never mix. Folders on
//! the left; the open folder's files on the right, as icons or as a list. The Engine folder is only
//! listed on the left, never shown as a tile among the project's files.
//!
//! Double-click a folder to open it and a scene to load it. Drag a file onto a text field in the
//! inspector to use its path (a script module, for example); the context menu offers the same
//! without dragging.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::json;

use crate::theme::{self, LIST_ROW, Tokens};
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
    Image,
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
            Some("png" | "jpg" | "jpeg" | "webp" | "ktx2") => Kind::Image,
            Some("glb" | "gltf") => Kind::Model,
            _ => Kind::Other,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Kind::Folder => "Folder",
            Kind::ScriptSource => "Script Source",
            Kind::Scene => "Scene",
            Kind::Project => "Project",
            Kind::Prefab => "Prefab",
            Kind::Model => "Model",
            Kind::Script => "Script",
            Kind::VisualScript => "Visual Script",
            Kind::Material => "Material",
            Kind::Image => "Image",
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ViewMode {
    #[default]
    Icons,
    List,
}

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
    current: PathBuf,
    mode: ViewMode,
    selected: Option<PathBuf>,
    /// Icon size in points; 0 means the default.
    icon_size: f32,
    /// A new file being named; nothing is written until the name is confirmed.
    naming: Option<Naming>,
}

impl ProjectView {
    /// Every script (`.wasm` and `.bscript`) in the project, as paths relative to the project folder.
    pub(crate) fn script_files(&self) -> Vec<String> {
        self.files_of(&[Kind::Script, Kind::VisualScript])
    }

    /// Every material graph in the project, as paths relative to the project folder.
    pub(crate) fn material_files(&self) -> Vec<String> {
        self.files_of(&[Kind::Material])
    }

    /// Every 3D model (`.glb`, `.gltf`) in the project.
    pub(crate) fn model_files(&self) -> Vec<String> {
        self.files_of(&[Kind::Model])
    }

    /// Every PNG image in the project (the only image format read so far).
    pub(crate) fn image_files(&self) -> Vec<String> {
        self.files_of(&[Kind::Image]).into_iter().filter(|f| f.to_ascii_lowercase().ends_with(".png")).collect()
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
}

/// What a new file will be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NewKind {
    Material,
    VisualScript,
    RustScript,
}

impl NewKind {
    fn label(self) -> &'static str {
        match self {
            NewKind::Material => "material",
            NewKind::VisualScript => "visual script",
            NewKind::RustScript => "script",
        }
    }

    /// The extension the file gets, or none for a script's folder.
    fn extension(self) -> Option<&'static str> {
        match self {
            NewKind::Material => Some("bmaterial"),
            NewKind::VisualScript => Some("bscript"),
            NewKind::RustScript => None,
        }
    }
}

/// A new file being named in the Project panel.
pub(crate) struct Naming {
    kind: NewKind,
    folder: PathBuf,
    text: String,
    /// The field has had focus (and its text selected) once.
    focused: bool,
    /// Why the last name was refused.
    error: Option<String>,
}

enum Action {
    OpenScene(PathBuf),
    PlacePrefab(PathBuf),
    OpenGraph(PathBuf),
    AddScript(String),
    /// Start naming a new file of this kind in the open folder.
    StartNew(NewKind),
    /// Make the file being named, with this name.
    Create(String),
    /// Build the script crate in this folder.
    BuildScript(PathBuf),
}

/// What the panel's parts ask for after drawing.
#[derive(Default)]
struct Outcome {
    action: Option<Action>,
    enter: Option<PathBuf>,
    select: Option<PathBuf>,
}

pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    let mut view = std::mem::take(&mut editor.project);
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
    let files_area = egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(egui::Margin { left: 6, ..egui::Margin::ZERO })).show(ui, |ui| {
        naming_row(ui, &mut view, &mut outcome);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| files(ui, &view, selected_object, &mut outcome));
    });
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
    if let Some(file) = outcome.select {
        view.selected = Some(file);
    }
    editor.project = view;
    match outcome.action {
        Some(Action::OpenScene(path)) => editor.open_scene(&path),
        Some(Action::PlacePrefab(path)) => editor.place_prefab(&path),
        Some(Action::OpenGraph(path)) => editor.open_graph(&path),
        Some(Action::StartNew(kind)) => {
            // New files go into the project only, never into the engine's folder.
            let folder = if editor.project.in_project() { editor.project.current.clone() } else { editor.project.root.clone().unwrap_or_default() };
            if !folder.as_os_str().is_empty() {
                let text = match kind {
                    NewKind::Material => "New Material",
                    NewKind::VisualScript => "New Visual Script",
                    NewKind::RustScript => "New Script",
                };
                editor.project.naming = Some(Naming { kind, folder, text: text.to_owned(), focused: false, error: None });
            }
        }
        Some(Action::Create(name)) => {
            if let Some(naming) = editor.project.naming.take() {
                match create(editor, &naming, &name) {
                    Ok(()) => editor.project.scanned = None,
                    // Keep the field open with the reason, so the name can be fixed.
                    Err(error) => editor.project.naming = Some(Naming { error: Some(error), focused: false, text: name, ..naming }),
                }
            }
        }
        Some(Action::BuildScript(dir)) => editor.build_script(&dir),
        Some(Action::AddScript(module)) => {
            if let Some(id) = editor.selected {
                editor.dispatch("scene.script.add", &json!({ "id": id, "module": module }), None);
            }
        }
        None => {}
    }
}

/// The name field of a new file, above the files. Enter makes the file; Escape, or clicking
/// elsewhere, cancels and nothing is written.
fn naming_row(ui: &mut egui::Ui, view: &mut ProjectView, outcome: &mut Outcome) {
    let Some(naming) = view.naming.as_mut() else { return };
    let t = theme::tokens(ui);
    let mut cancel = false;
    egui::Frame::new().fill(t.surface).corner_radius(theme::SECTION_RADIUS).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("New {}:", naming.kind.label())).color(t.text_muted));
            let field = ui.add_sized([ui.available_width().min(260.0), LIST_ROW], egui::TextEdit::singleline(&mut naming.text).margin(egui::vec2(6.0, 1.0)));
            ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(format!("Name of the new {}", naming.kind.label())));
            if !naming.focused {
                // Focus the field with its text selected, so typing replaces it.
                field.request_focus();
                if let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), field.id) {
                    let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(naming.text.chars().count()));
                    state.cursor.set_char_range(Some(all));
                    state.store(ui.ctx(), field.id);
                }
                naming.focused = true;
            } else if field.lost_focus() {
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    outcome.action = Some(Action::Create(naming.text.trim().to_owned()));
                } else {
                    cancel = true;
                }
            }
            if let Some(extension) = naming.kind.extension() {
                ui.label(egui::RichText::new(format!(".{extension}")).color(t.text_muted));
            }
            ui.label(egui::RichText::new("Enter to create, Esc to cancel").small().color(t.text_muted));
        });
        if let Some(error) = &naming.error {
            ui.label(egui::RichText::new(error).color(t.error));
        }
    });
    if cancel {
        view.naming = None;
    }
}

/// Makes the file being named. Fails with a reason when the name is not usable.
fn create(editor: &mut Editor, naming: &Naming, name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Type a name first.".to_owned());
    }
    if name.chars().count() > 100 || name.chars().any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()) {
        return Err(r#"A name may have up to 100 characters and none of / \ : * ? " < > |."#.to_owned());
    }
    match naming.kind.extension() {
        Some(extension) => {
            let file = if name.to_ascii_lowercase().ends_with(&format!(".{extension}")) { name.to_owned() } else { format!("{name}.{extension}") };
            let path = naming.folder.join(file);
            if path.exists() {
                return Err(format!("{} already exists.", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
            }
            if editor.session.graph().is_some_and(|g| g.dirty) && !crate::confirm_discard_graph() {
                return Ok(());
            }
            let domain = if naming.kind == NewKind::Material { "material" } else { "script" };
            editor.dispatch("graph.new", &json!({ "domain": domain }), None).ok_or_else(|| editor.status.clone())?;
            editor.dispatch("graph.save", &json!({ "path": path }), None).ok_or_else(|| editor.status.clone())?;
            Ok(())
        }
        None => {
            let made = editor.dispatch("script.new", &json!({ "folder": naming.folder, "name": name }), None).ok_or_else(|| editor.status.clone())?;
            if let Some(dir) = made["path"].as_str() {
                editor.build_script(std::path::Path::new(dir));
            }
            Ok(())
        }
    }
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
                ui.label(egui::RichText::new("Project").small().color(t.text));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            toolbar_right(ui, view);
        });
    });
}

fn create_menu(ui: &mut egui::Ui, outcome: &mut Outcome) {
    let create = ui.menu_button("+", |ui| {
        let items = [
            (NewKind::RustScript, "New Script", "A script written in Rust, in this folder; it builds to WebAssembly"),
            (NewKind::VisualScript, "New Visual Script", "A visual script in this folder"),
            (NewKind::Material, "New Material", "A material graph in this folder"),
        ];
        for (kind, label, hint) in items {
            if ui.button(label).on_hover_text(hint).clicked() {
                outcome.action = Some(Action::StartNew(kind));
                ui.close();
            }
        }
    });
    ui.ctx().accesskit_node_builder(create.response.id, |node| node.set_label("Create file"));
    create.response.on_hover_text("Create a script, visual script or material in the project");
}

/// View switch, icon size and search, laid out from the right.
fn toolbar_right(ui: &mut egui::Ui, view: &mut ProjectView) {
    ui.spacing_mut().item_spacing.x = 4.0;
    let search = ui.add_sized([SEARCH_WIDTH, LIST_ROW], egui::TextEdit::singleline(&mut view.filter).hint_text("Search").align(egui::Align2::LEFT_CENTER));
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
    if slider.on_hover_text("Icon size (Ctrl + mouse wheel)").changed() {
        view.icon_size = size;
    }
    for (mode, label) in [(ViewMode::List, "List"), (ViewMode::Icons, "Icons")] {
        if ui.add(egui::Button::selectable(view.mode == mode, label).small()).on_hover_text(format!("Show files as {}", label.to_lowercase())).clicked() {
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
                if response.clicked() {
                    outcome.enter = Some(entry.path.clone());
                }
                rows.push(response.rect);
            });
        }
    }
}

fn folder_row(ui: &mut egui::Ui, name: &str, selected: bool) -> egui::Response {
    let width = ui.available_width();
    ui.add(egui::Button::selectable(selected, (name, egui::Atom::grow())).truncate().min_size(egui::vec2(width, LIST_ROW)))
}

/// The open folder's entries, or every file matching the search.
fn files(ui: &mut egui::Ui, view: &ProjectView, selected_object: bool, outcome: &mut Outcome) {
    if view.current.as_os_str().is_empty() {
        inspector::empty_state(ui, "No project folder", "Open or save a scene to see its folder here. The engine's own files are under Engine, on the left.");
        return;
    }
    let filter = view.filter.trim().to_lowercase();
    // The engine's folder is never a tile: it is chosen from the folder list only.
    let shown = |e: &&Entry| Some(&e.path) != view.engine.as_ref();
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
    if entries.is_empty() {
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
                for entry in entries {
                    let response = tile(ui, entry, view.selected.as_deref() == Some(entry.path.as_path()), view.icon());
                    interact(ui, &response, view, entry, selected_object, outcome);
                }
            });
        }
        ViewMode::List => {
            theme::dense(ui);
            let slot = ui.painter().add(egui::Shape::Noop);
            let full_width = ui.max_rect().x_range();
            let mut rows = Vec::new();
            for entry in entries {
                let response = list_row(ui, entry, view.selected.as_deref() == Some(entry.path.as_path()));
                rows.push(response.rect);
                interact(ui, &response, view, entry, selected_object, outcome);
            }
            theme::stripes(ui, slot, rows.into_iter(), full_width);
        }
    }
}

/// One file or folder in the icon view: drawn icon, name below.
fn tile(ui: &mut egui::Ui, entry: &Entry, selected: bool, icon_size: f32) -> egui::Response {
    let t = theme::tokens(ui);
    // Room for the icon, a little padding, and two lines of name below.
    let tile_size = egui::vec2((icon_size + 36.0).max(72.0), icon_size + 38.0);
    let (rect, response) = ui.allocate_exact_size(tile_size, egui::Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, format!("{}, {}", entry.name, entry.kind.label())));
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
        paint_icon(painter, icon, entry.kind, t);
        let color = if selected { t.on_accent } else { t.text };
        let mut job = egui::text::LayoutJob::simple(entry.name.clone(), egui::TextStyle::Small.resolve(ui.style()), color, tile_size.x - 6.0);
        job.wrap.max_rows = 2;
        job.wrap.break_anywhere = true;
        job.halign = egui::Align::Center;
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        painter.galley(egui::pos2(rect.center().x, icon.bottom() + 4.0), galley, color);
    }
    response.on_hover_text(&entry.name)
}

/// One file or folder in the list view: small icon, name, kind.
fn list_row(ui: &mut egui::Ui, entry: &Entry, selected: bool) -> egui::Response {
    let t = theme::tokens(ui);
    let width = ui.available_width();
    let kind = entry.kind;
    // The icon is an atom of the button, so it takes its own space before the name.
    let icon = egui::Atom::paint(egui::vec2(14.0, 14.0), move |ui, args| paint_icon(ui.painter(), args.rect, kind, t));
    let button = egui::Button::selectable(
        selected,
        (icon, entry.name.as_str(), egui::Atom::grow(), egui::RichText::new(entry.kind.label()).small().color(t.text_muted)),
    )
    .truncate()
    .min_size(egui::vec2(width, LIST_ROW))
    .sense(egui::Sense::click_and_drag());
    let response = ui.add(button);
    ui.ctx().accesskit_node_builder(response.id, |node| node.set_label(format!("{}, {}", entry.name, entry.kind.label())));
    response
}

/// Clicks, double-clicks, dragging and the context menu, shared by tiles and list rows.
fn interact(ui: &egui::Ui, response: &egui::Response, view: &ProjectView, entry: &Entry, selected_object: bool, outcome: &mut Outcome) {
    // Project files are referred to relative to the project; engine files by their full path.
    let relative = match &view.root {
        Some(root) if entry.path.starts_with(root) => relative_path(root, &entry.path),
        _ => relative_path(Path::new(""), &entry.path),
    };
    if response.clicked() || (response.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed())) {
        outcome.select = Some(entry.path.clone());
    }
    if response.double_clicked() {
        match entry.kind {
            Kind::Folder | Kind::ScriptSource => outcome.enter = Some(entry.path.clone()),
            Kind::Scene | Kind::Project => outcome.action = Some(Action::OpenScene(entry.path.clone())),
            Kind::Material | Kind::VisualScript => outcome.action = Some(Action::OpenGraph(entry.path.clone())),
            Kind::Prefab => outcome.action = Some(Action::PlacePrefab(entry.path.clone())),
            _ => {}
        }
    }
    if entry.kind != Kind::Folder {
        if response.drag_started_by(egui::PointerButton::Primary) {
            egui::DragAndDrop::set_payload(ui.ctx(), DraggedAsset { relative: relative.clone() });
        }
        if response.dragged_by(egui::PointerButton::Primary)
            && let Some(pointer) = ui.ctx().pointer_interact_pos()
        {
            drag_ghost(ui, &entry.name, pointer);
        }
    }
    response.context_menu(|ui| {
        if entry.kind == Kind::ScriptSource && ui.button("Build Script").on_hover_text("Build it to WebAssembly, next to its folder").clicked() {
            outcome.action = Some(Action::BuildScript(entry.path.clone()));
            ui.close();
        }
        if matches!(entry.kind, Kind::Folder | Kind::ScriptSource) && ui.button("Open Folder").clicked() {
            outcome.enter = Some(entry.path.clone());
            ui.close();
        }
        if entry.kind == Kind::Scene && ui.button("Open Scene").clicked() {
            outcome.action = Some(Action::OpenScene(entry.path.clone()));
            ui.close();
        }
        if entry.kind == Kind::Prefab && ui.button("Place in Scene").on_hover_text("Adds a linked copy after the selected object").clicked() {
            outcome.action = Some(Action::PlacePrefab(entry.path.clone()));
            ui.close();
        }
        if entry.kind == Kind::Project && ui.button("Open Project").clicked() {
            outcome.action = Some(Action::OpenScene(entry.path.clone()));
            ui.close();
        }
        if matches!(entry.kind, Kind::Material | Kind::VisualScript) && ui.button("Open in Graph Editor").clicked() {
            outcome.action = Some(Action::OpenGraph(entry.path.clone()));
            ui.close();
        }
        if matches!(entry.kind, Kind::Script | Kind::VisualScript) {
            let button = ui.add_enabled(selected_object, egui::Button::new("Add to Selected Object"));
            if button.on_disabled_hover_text("Select an object first").clicked() {
                outcome.action = Some(Action::AddScript(relative.clone()));
                ui.close();
            }
        }
        if ui.button("Copy Relative Path").clicked() {
            ui.ctx().copy_text(relative.clone());
            ui.close();
        }
    });
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
        Kind::Script | Kind::ScriptSource | Kind::VisualScript | Kind::Image | Kind::Other => {
            // A page with a folded corner.
            let fold = s * 0.22;
            let (l, r, top, bottom) = (rect.left() + s * 0.18, rect.right() - s * 0.18, rect.top() + s * 0.08, rect.bottom() - s * 0.08);
            let page = vec![egui::pos2(l, top), egui::pos2(r - fold, top), egui::pos2(r, top + fold), egui::pos2(r, bottom), egui::pos2(l, bottom)];
            let accent = match kind {
                Kind::Script | Kind::ScriptSource => t.axis_z,
                Kind::VisualScript => t.pin_any,
                Kind::Image => t.axis_y,
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
        if self.root != root {
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
    fn scans_folders_first_and_skips_hidden_and_build_folders() {
        let dir = std::env::temp_dir().join(format!("bayleaf-project-{}", std::process::id()));
        for sub in ["scripts", "target", ".git"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        std::fs::write(dir.join("a.bscene"), "{}").unwrap();
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
