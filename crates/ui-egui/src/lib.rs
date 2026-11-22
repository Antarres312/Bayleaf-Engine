//! L6: the egui editor shell. Panels change engine state only through registered commands.
//!
//! The menu bar and keyboard shortcuts are built from the command registry, so every menu entry
//! is also reachable from the CLI, the control channel and MCP. Panels are docked (see [`dock`])
//! and every colour and size comes from the theme tokens in [`theme`].

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod brand;
mod console;
mod dock;
mod file_inspector;
mod fonts;
mod game_view;
pub mod gamepad;
mod gizmo;
mod graph_view;
mod hierarchy;
mod input_editor;
mod inspector;
mod markdown_view;
mod project;
mod project_settings;
mod scene_view;
mod script_info;
mod script_panel;
mod theme;
mod value_editor;

pub use dock::{Layout, Panel};
pub use fonts::install_fonts;

use bayleaf_core::StableId;
use bayleaf_engine::scene::doc::EXTENSION;
use bayleaf_engine::{CommandInfo, CommandRegistry, Prompt, Session};
use serde_json::{Value, json};

/// Menus in the order they appear; menus not listed here follow in registry order. The Window
/// menu is the editor's own and sits before Help.
const MENU_ORDER: &[&str] = &["File", "Edit", "Object", "Play", "Help"];

/// Editor state shared by every panel.
pub struct Editor {
    registry: CommandRegistry,
    session: Session,
    status: String,
    /// The primary selection: the object the Inspector shows and the handles act from.
    selected: Option<StableId>,
    /// The other selected objects (Ctrl or Shift + click), in the order they were added.
    also_selected: Vec<StableId>,
    /// Bumped when an edit gesture (a drag, a text field) ends, so the next edit gets its own undo step.
    gesture: u64,
    title: String,
    scene_view: scene_view::SceneView,
    game_view: game_view::GameView,
    /// Play mode as last frame saw it: starting raises the Game view, stopping the scene view.
    was_playing: bool,
    /// Projects opened lately, newest first, for File > Open Recent.
    recent_projects: Vec<std::path::PathBuf>,
    /// The project folder as last frame saw it, to notice when another one opens.
    last_project: Option<std::path::PathBuf>,
    dock: egui_dock::DockState<Panel>,
    /// The panel filling the whole dock area, when one is maximised (double-click its tab).
    maximized: Option<Panel>,
    hierarchy_filter: String,
    /// The object being renamed in the hierarchy, if any.
    renaming: Option<hierarchy::Rename>,
    /// Shown in the status bar while an agent can control the editor, e.g. its address.
    automation_note: Option<String>,
    theme_installed: bool,
    /// Logo texture and whether it was made for the dark theme.
    logo: Option<(egui::TextureHandle, bool)>,
    project: project::ProjectView,
    script_info: script_info::ScriptInfo,
    graph_view: graph_view::GraphView,
    /// A panel to bring to the front after this frame's panels are drawn.
    raise_panel: Option<Panel>,
    /// The objects last copied, as clipboard text, for Paste from a menu.
    clipboard: Option<String>,
    /// Text to put on the system clipboard at the end of the frame.
    copy_out: Option<String>,
    /// The window was asked to close while there are unsaved changes.
    close_asked: bool,
    /// Closing was confirmed; the next close request is let through.
    close_confirmed: bool,
    /// Script builds running in the background: the script's name and where the result arrives.
    builds: Vec<(String, std::sync::mpsc::Receiver<Result<bayleaf_engine::code_scripts::Built, String>>)>,
    /// The file clicked in the Project panel, shown in the Inspector while no object is selected.
    inspected_file: Option<file_inspector::Inspected>,
    /// Gamepads, read in play mode.
    gamepads: gamepad::Gamepads,
    /// The settings the Project Settings panel shows.
    project_settings: project_settings::Shown,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            registry: CommandRegistry::with_builtins(),
            session: Session::default(),
            status: String::new(),
            selected: None,
            also_selected: Vec::new(),
            gesture: 0,
            title: String::new(),
            scene_view: scene_view::SceneView::new(),
            game_view: game_view::GameView::default(),
            was_playing: false,
            recent_projects: Vec::new(),
            last_project: None,
            dock: Layout::Standard.build(),
            maximized: None,
            hierarchy_filter: String::new(),
            renaming: None,
            automation_note: None,
            theme_installed: false,
            logo: None,
            project: project::ProjectView::default(),
            script_info: script_info::ScriptInfo::default(),
            graph_view: graph_view::GraphView::default(),
            raise_panel: None,
            clipboard: None,
            copy_out: None,
            close_asked: false,
            close_confirmed: false,
            builds: Vec::new(),
            inspected_file: None,
            gamepads: gamepad::Gamepads::default(),
            project_settings: project_settings::Shown::default(),
        }
    }
}

impl Editor {
    /// Draws the editor for one frame. `gpu` is the window's wgpu state; without it the scene view
    /// shows a notice instead of the scene.
    pub fn ui(&mut self, ui: &mut egui::Ui, gpu: Option<&egui_wgpu::RenderState>) {
        if !self.theme_installed {
            theme::install(ui.ctx());
            self.theme_installed = true;
        }
        let t = theme::tokens(ui);
        self.handle_shortcuts(ui.ctx());
        self.poll_builds(ui.ctx());
        self.run_play_frame(ui.ctx());
        self.note_recent_project();
        if self.session.is_playing() != self.was_playing {
            self.was_playing = self.session.is_playing();
            self.show_panel(if self.was_playing { Panel::Game } else { Panel::Scene });
        }
        egui::Panel::top("menu_bar").frame(egui::Frame::new().fill(t.backdrop).inner_margin(egui::Margin::symmetric(8, 4))).show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                self.logo_mark(ui);
                self.menus(ui);
            });
        });
        egui::Panel::bottom("status_bar").frame(egui::Frame::new().fill(t.backdrop).inner_margin(egui::Margin::symmetric(10, 4))).show(ui, |ui| {
            ui.horizontal(|ui| {
                if self.session.is_dirty() {
                    theme::tag(ui, "Modified", t.surface_active, t.text);
                }
                if let Some(note) = &self.automation_note {
                    // A visible sign that an agent can change the scene.
                    theme::tag(ui, &format!("Agent control: {note}"), t.accent, t.on_accent);
                }
                ui.label(egui::RichText::new(&self.status).color(t.text_muted));
            });
        });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.backdrop)).show(ui, |ui| dock::show(self, ui, gpu));
        self.apply_raise();
        self.update_title(ui.ctx());
        if let Some(text) = self.copy_out.take() {
            ui.ctx().copy_text(text);
        }
        self.guard_close(ui.ctx());
        if self.selected.is_some_and(|id| !self.session.scene().contains(id)) {
            self.selected = self.also_selected.pop();
        }
        let scene = self.session.scene();
        let primary = self.selected;
        self.also_selected.retain(|id| scene.contains(*id) && Some(*id) != primary);
        if self.selected.is_none() {
            self.also_selected.clear();
        }
    }

    /// Every selected object, the primary one first.
    pub(crate) fn selection(&self) -> Vec<StableId> {
        self.selected.into_iter().chain(self.also_selected.iter().copied()).collect()
    }

    pub(crate) fn is_selected(&self, id: StableId) -> bool {
        self.selected == Some(id) || self.also_selected.contains(&id)
    }

    /// Selects only `id` (or nothing).
    pub(crate) fn select_only(&mut self, id: Option<StableId>) {
        self.selected = id;
        self.also_selected.clear();
    }

    /// Adds `id` to the selection as the primary object, or takes it out if it is already in.
    pub(crate) fn toggle_selected(&mut self, id: StableId) {
        if self.is_selected(id) {
            if self.selected == Some(id) {
                self.selected = self.also_selected.pop();
            } else {
                self.also_selected.retain(|o| *o != id);
            }
        } else {
            if let Some(previous) = self.selected {
                self.also_selected.push(previous);
            }
            self.selected = Some(id);
        }
    }

    /// The selected objects that are not inside another selected object, the primary one first.
    pub(crate) fn selection_roots(&self) -> Vec<StableId> {
        let all = self.selection();
        let scene = self.session.scene();
        all.iter().copied().filter(|id| !all.iter().any(|other| other != id && scene.is_inside(*id, *other))).collect()
    }

    /// Runs `command` with `{"id"}` for every selected object, as one undo step.
    pub(crate) fn for_each_selected(&mut self, command: &str) -> Vec<Value> {
        let key = format!("selection {command} {}", self.gesture);
        let results: Vec<Value> = self.selection_roots().into_iter().filter_map(|id| self.dispatch(command, &json!({ "id": id }), Some(&key))).collect();
        self.gesture += 1;
        results
    }

    /// Shows (or hides, with `None`) the agent-control badge in the status bar.
    pub fn set_automation_note(&mut self, note: Option<String>) {
        self.automation_note = note;
    }

    /// The command list, as automation's `engine.commands` returns it.
    pub fn automation_commands(&self) -> serde_json::Value {
        serde_json::to_value(self.registry.list(&self.session)).unwrap_or(serde_json::Value::Null)
    }

    /// Runs a command for an agent, with file access confined to its folders. The status bar shows
    /// what the agent did, so the person at the editor can follow along.
    pub fn execute_automation(
        &mut self,
        command: &str,
        params: &serde_json::Value,
        confinement: &bayleaf_engine::confine::Confinement,
    ) -> Result<serde_json::Value, String> {
        self.session.set_confinement(Some(confinement.clone()));
        let result = self.registry.execute(&mut self.session, command, params).map_err(|e| e.to_string());
        self.session.set_confinement(None);
        let label = self.registry.get(command).map_or(command, |c| c.label).trim_end_matches(['…', '.']);
        self.status = match &result {
            Ok(_) => format!("Agent: {label}: done"),
            Err(e) => format!("Agent: {label}: {e}"),
        };
        result
    }

    /// Opens a scene file, asking first if the open scene has unsaved changes.
    pub fn open_scene(&mut self, path: &std::path::Path) {
        if self.session.is_dirty() && !confirm_discard() {
            return;
        }
        // A project file opens the project's main scene.
        let command = if path.extension().is_some_and(|e| e == bayleaf_engine::project_cmds::PROJECT_EXTENSION) { "project.open" } else { "scene.open" };
        self.dispatch(command, &json!({ "path": path }), None);
    }

    /// Opens a material graph or visual script in the graph editor, asking first if the open graph has unsaved changes.
    pub fn open_graph(&mut self, path: &std::path::Path) {
        if self.session.graph().is_some_and(|g| g.dirty) && !confirm_discard_graph() {
            return;
        }
        self.dispatch("graph.open", &json!({ "path": path }), None);
    }

    /// Opens a panel if it is hidden and brings its tab to the front, once the panels are drawn
    /// (while they draw, the dock is out of the editor).
    /// Shows a file in the Inspector in place of the selected objects, bringing the Inspector up.
    pub(crate) fn inspect_file(&mut self, path: std::path::PathBuf) {
        self.select_only(None);
        self.inspected_file = Some(file_inspector::Inspected::new(path));
        self.show_panel(Panel::Inspector);
    }

    pub(crate) fn show_panel(&mut self, panel: Panel) {
        self.raise_panel = Some(panel);
        self.apply_raise();
    }

    fn apply_raise(&mut self) {
        if self.dock.iter_all_tabs().next().is_none() {
            return;
        }
        if let Some(panel) = self.raise_panel.take() {
            dock::set_open(&mut self.dock, panel, true);
            if let Some(path) = self.dock.find_tab(&panel) {
                let _ = self.dock.set_active_tab(path);
            }
        }
    }

    /// In play mode: passes the held keys to the scripts and advances the simulation by one frame.
    fn run_play_frame(&mut self, ctx: &egui::Context) {
        if !self.session.is_playing() {
            return;
        }
        // Keys typed into a text field are not game input.
        let keys = if ctx.text_edit_focused() { Default::default() } else { ctx.input(|i| i.keys_down.iter().map(|k| k.name().to_owned()).collect()) };
        self.session.set_keys(keys);
        let pad = self.gamepads.read();
        self.session.set_pad(pad);
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        self.session.tick(dt);
        // The Game view sets the pointer again while it is shown; hidden, the game sees none.
        self.session.set_pointer(None, 0);
        if !self.session.is_paused() {
            ctx.request_repaint();
        }
    }

    /// Most projects File > Open Recent lists.
    const MAX_RECENT: usize = 10;

    /// Puts a project that just opened first in the recent list.
    fn note_recent_project(&mut self) {
        let current = self.session.project_dir().map(std::path::Path::to_path_buf);
        if current == self.last_project {
            return;
        }
        self.last_project = current.clone();
        if let Some(dir) = current {
            self.recent_projects.retain(|p| *p != dir);
            self.recent_projects.insert(0, dir);
            self.recent_projects.truncate(Self::MAX_RECENT);
        }
    }

    /// File > Open Recent: the projects opened lately. One whose folder is gone shows, greyed out.
    fn recent_menu(&mut self, ui: &mut egui::Ui, open: &mut Option<std::path::PathBuf>) {
        ui.menu_button("Open Recent", |ui| {
            if self.recent_projects.is_empty() {
                ui.add_enabled(false, egui::Button::new("No recent projects"));
                return;
            }
            for dir in &self.recent_projects {
                let name = dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned());
                let here = self.session.project_dir() == Some(dir.as_path());
                let button = ui.add_enabled(dir.is_dir() && !here, egui::Button::new(name)).on_hover_text(dir.display().to_string());
                let button =
                    if here { button.on_disabled_hover_text("Open now") } else { button.on_disabled_hover_text(format!("Not found: {}", dir.display())) };
                if button.clicked() {
                    *open = Some(dir.clone());
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Clear Recent").clicked() {
                self.recent_projects.retain(|p| Some(p.as_path()) == self.session.project_dir());
                ui.close();
            }
        });
    }

    /// The recent projects as text, for saving between runs.
    pub fn recent_projects_text(&self) -> String {
        serde_json::to_string(&self.recent_projects).unwrap_or_default()
    }

    /// Restores the recent projects saved by [`Self::recent_projects_text`]; broken text is ignored.
    pub fn restore_recent_projects(&mut self, text: &str) {
        if let Ok(list) = serde_json::from_str::<Vec<std::path::PathBuf>>(text) {
            self.recent_projects = list.into_iter().take(Self::MAX_RECENT).collect();
        }
    }

    /// The panel layout as text, for saving between runs.
    pub fn saved_layout(&self) -> Option<String> {
        dock::to_text(&self.dock)
    }

    /// Restores a layout saved by [`Self::saved_layout`]. Returns false and keeps the current
    /// layout when the data is from another version or broken.
    pub fn restore_layout(&mut self, text: &str) -> bool {
        match dock::from_text(text) {
            Some(dock) => {
                self.dock = dock;
                true
            }
            None => false,
        }
    }

    /// The logo at the start of the menu bar, in the theme's brand colour.
    fn logo_mark(&mut self, ui: &mut egui::Ui) {
        let dark = ui.visuals().dark_mode;
        if self.logo.as_ref().is_none_or(|(_, made_dark)| *made_dark != dark)
            && let Some(image) = brand::logo(64, theme::tokens(ui).brand)
        {
            let color = egui::ColorImage::from_rgba_unmultiplied([image.size as usize, image.size as usize], &image.rgba);
            self.logo = Some((ui.ctx().load_texture("bayleaf logo", color, egui::TextureOptions::LINEAR), dark));
        }
        if let Some((texture, _)) = &self.logo {
            ui.add(egui::Image::new(egui::load::SizedTexture::new(texture.id(), egui::vec2(18.0, 18.0))).alt_text("Bayleaf Engine"));
            ui.add_space(4.0);
        }
    }

    fn window_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Window", |ui| {
            for panel in Panel::ALL {
                let mut open = dock::is_open(&self.dock, panel);
                let response = ui.add_enabled(panel != Panel::Scene, egui::Checkbox::new(&mut open, panel.title()));
                if response.changed() {
                    dock::set_open(&mut self.dock, panel, open);
                }
            }
            ui.separator();
            for layout in Layout::ALL {
                if ui.button(layout.label()).clicked() {
                    self.dock = layout.build();
                    self.status = format!("{}: applied", layout.label());
                    ui.close();
                }
            }
        });
    }

    fn menus(&mut self, ui: &mut egui::Ui) {
        let entries: Vec<CommandInfo> = self.registry.list(&self.session).into_iter().filter(|c| !c.menu.is_empty()).collect();
        let mut top_menus: Vec<&str> = MENU_ORDER.to_vec();
        for top in entries.iter().filter_map(|c| c.menu.split('/').next()) {
            if !top_menus.contains(&top) {
                top_menus.push(top);
            }
        }
        let mut clicked = None;
        let mut clipboard_action = None;
        let mut open_recent = None;
        for top in top_menus {
            let items: Vec<&CommandInfo> = entries.iter().filter(|c| c.menu.split('/').next() == Some(top)).collect();
            if top == "Help" {
                self.window_menu(ui);
            }
            if items.is_empty() {
                continue;
            }
            ui.menu_button(top, |ui| {
                for entry in items {
                    let label = entry.menu.rsplit('/').next().unwrap_or(entry.label);
                    let enabled = entry.enabled && (entry.prompt != Prompt::SelectedObject || self.selected.is_some());
                    let button = egui::Button::new(label).shortcut_text(entry.shortcut);
                    let response = ui.add_enabled(enabled, button);
                    let response = match &entry.disabled_reason {
                        Some(reason) => response.on_disabled_hover_text(reason),
                        None => response,
                    };
                    if response.clicked() {
                        clicked = Some(entry.id);
                        ui.close();
                    }
                }
                if top == "File" {
                    ui.separator();
                    self.recent_menu(ui, &mut open_recent);
                }
                if top == "Edit" {
                    // Clipboard entries belong to the editor: they use the system clipboard.
                    ui.separator();
                    let has_selection = self.selected.is_some();
                    if ui.add_enabled(has_selection, egui::Button::new("Cut").shortcut_text("Ctrl+X")).clicked() {
                        clipboard_action = Some("cut");
                        ui.close();
                    }
                    if ui.add_enabled(has_selection, egui::Button::new("Copy").shortcut_text("Ctrl+C")).clicked() {
                        clipboard_action = Some("copy");
                        ui.close();
                    }
                    let paste = ui.add_enabled(self.clipboard.is_some(), egui::Button::new("Paste").shortcut_text("Ctrl+V"));
                    if paste.on_disabled_hover_text("Copy an object first").clicked() {
                        clipboard_action = Some("paste");
                        ui.close();
                    }
                }
            });
        }
        if let Some(id) = clicked {
            self.run_with_prompt(id);
        }
        if let Some(path) = open_recent
            && !(self.session.is_dirty() && !confirm_discard())
        {
            self.dispatch("project.open", &json!({ "path": path }), None);
        }
        match clipboard_action {
            Some("cut") => self.copy_selection(true),
            Some("copy") => self.copy_selection(false),
            Some("paste") => self.paste(None),
            _ => {}
        }
    }

    /// True when the scene or the open graph has changes that are not saved.
    pub fn has_unsaved_changes(&self) -> bool {
        self.session.is_dirty() || self.session.graph().is_some_and(|g| g.dirty)
    }

    /// Copies the selected objects and their children to the clipboard; `cut` then deletes them.
    pub(crate) fn copy_selection(&mut self, cut: bool) {
        let roots = self.selection_roots();
        if roots.is_empty() {
            self.status = "Copy: select an object first.".to_owned();
            return;
        }
        let Some(data) = self.dispatch("scene.object.copy", &json!({ "ids": roots }), None) else { return };
        let text = serde_json::to_string_pretty(&data).unwrap_or_default();
        self.clipboard = Some(text.clone());
        self.copy_out = Some(text);
        let what = if roots.len() == 1 { "the object is".to_owned() } else { format!("{} objects are", roots.len()) };
        if cut {
            self.for_each_selected("scene.object.delete");
            self.status = format!("Cut: {what} on the clipboard");
        } else {
            self.status = format!("Copy: {what} on the clipboard");
        }
    }

    /// Pastes copied objects after the selected object (or at the end), and selects the first.
    /// `text` is the system clipboard; `None` uses the objects this editor copied last.
    pub(crate) fn paste(&mut self, text: Option<String>) {
        let Some(text) = text.or_else(|| self.clipboard.clone()) else {
            self.status = "Paste: copy an object first.".to_owned();
            return;
        };
        let Ok(objects) = serde_json::from_str::<Value>(&text) else {
            self.status = "Paste: the clipboard holds no objects.".to_owned();
            return;
        };
        if objects.get("format").and_then(Value::as_str) != Some(bayleaf_engine::CLIPBOARD_FORMAT) {
            self.status = "Paste: the clipboard holds no objects.".to_owned();
            return;
        }
        let scene = self.session.scene();
        let (parent, index) = match self.selected {
            Some(id) => {
                let parent = scene.parent(id);
                (parent, scene.children(parent).iter().position(|c| *c == id).map(|i| i + 1))
            }
            None => (None, None),
        };
        if let Some(result) = self.dispatch("scene.object.paste", &json!({ "objects": objects, "parent": parent, "index": index }), None) {
            // Everything pasted is selected, the first one as the primary object.
            let ids: Vec<StableId> =
                result["ids"].as_array().into_iter().flatten().filter_map(|v| v.as_str().and_then(|s| StableId::parse_hex(s).ok())).collect();
            self.select_only(ids.first().copied());
            self.also_selected = ids.into_iter().skip(1).collect();
        }
    }

    /// Builds a Rust script's crate in the background; the console reports how it went.
    pub(crate) fn build_script(&mut self, dir: &std::path::Path) {
        let name = dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned());
        let (sender, receiver) = std::sync::mpsc::channel();
        let (dir, target) = (dir.to_path_buf(), bayleaf_engine::code_scripts::target_dir_for(&self.session));
        let spawned = std::thread::Builder::new().name("bayleaf script build".into()).spawn(move || {
            let _ = sender.send(bayleaf_engine::code_scripts::build_script(&dir, &target));
        });
        match spawned {
            Ok(_) => {
                self.status = format!("Building {name}…");
                self.builds.push((name, receiver));
            }
            Err(error) => self.status = format!("Build Script: {error}"),
        }
    }

    /// Reports finished script builds in the console and the status bar.
    fn poll_builds(&mut self, ctx: &egui::Context) {
        let mut finished = Vec::new();
        self.builds.retain(|(name, receiver)| match receiver.try_recv() {
            Ok(result) => {
                finished.push((name.clone(), result));
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                finished.push((name.clone(), Err("the build stopped without a result".to_owned())));
                false
            }
        });
        for (name, result) in finished {
            match result {
                Ok(built) => {
                    let module = built.module.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.status = format!("Built {name}: {module} is ready to add to objects");
                    self.session.log_line(bayleaf_engine::Level::Info, format!("Built {name} into {module}."));
                }
                Err(error) => {
                    self.status = format!("{name} did not build; the console says why");
                    self.session.log_line(bayleaf_engine::Level::Error, format!("{name}: {error}"));
                }
            }
            self.project.rescan_soon();
        }
        if !self.builds.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    /// Asks where to save `id` and its children as a prefab, then links `id` to the new file.
    pub(crate) fn save_as_prefab(&mut self, id: StableId) {
        let extension = bayleaf_engine::prefab_cmds::PREFAB_EXTENSION;
        let base = self.session.script_dir();
        let folder = base.join("prefabs");
        let name = self.session.scene().name(id).unwrap_or("Prefab").to_owned();
        let dialog = rfd::FileDialog::new()
            .add_filter("Bayleaf prefab", &[extension])
            .set_directory(if folder.is_dir() { &folder } else { &base })
            .set_file_name(format!("{name}.{extension}"));
        if let Some(path) = dialog.save_file() {
            self.dispatch("scene.prefab.create", &json!({ "id": id, "path": path.with_extension(extension) }), None);
        }
    }

    /// Places a linked copy of a prefab file after the selected object (or at the end) and selects it.
    pub(crate) fn place_prefab(&mut self, path: &std::path::Path) {
        let scene = self.session.scene();
        let (parent, index) = match self.selected {
            Some(id) => {
                let parent = scene.parent(id);
                (parent, scene.children(parent).iter().position(|c| *c == id).map(|i| i + 1))
            }
            None => (None, None),
        };
        if let Some(result) = self.dispatch("scene.prefab.place", &json!({ "path": path, "parent": parent, "index": index }), None) {
            self.select_only(result["id"].as_str().and_then(|s| StableId::parse_hex(s).ok()));
        }
    }

    /// Stops the window from closing while there are unsaved changes, and asks what to do.
    fn guard_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.close_confirmed && self.has_unsaved_changes() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_asked = true;
        }
        if !self.close_asked {
            return;
        }
        let t = theme::tokens_for(ctx);
        let mut choice = None;
        let modal = egui::Modal::new(egui::Id::unique("unsaved changes")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.heading("Save changes before closing?");
            ui.add_space(theme::GAP);
            if self.session.is_dirty() {
                let name =
                    self.session.scene_path().and_then(|p| p.file_name()).map_or_else(|| "Untitled scene".to_owned(), |n| n.to_string_lossy().into_owned());
                ui.label(egui::RichText::new(format!("Scene: {name}")).color(t.text));
            }
            if let Some(graph) = self.session.graph().filter(|g| g.dirty) {
                let name = graph.path.as_ref().and_then(|p| p.file_name()).map_or_else(|| "Untitled graph".to_owned(), |n| n.to_string_lossy().into_owned());
                ui.label(egui::RichText::new(format!("Graph: {name}")).color(t.text));
            }
            ui.add_space(theme::GAP * 2.0);
            ui.horizontal(|ui| {
                let save = ui.add(egui::Button::new(egui::RichText::new("Save and Close").color(t.on_accent)).fill(t.accent));
                if save.clicked() {
                    choice = Some(true);
                }
                if ui.button("Close Without Saving").clicked() {
                    choice = Some(false);
                }
                if ui.button("Cancel").clicked() {
                    self.close_asked = false;
                }
            });
        });
        if modal.should_close() {
            self.close_asked = false;
        }
        match choice {
            Some(true) if self.save_everything() => self.close_now(ctx),
            Some(true) => self.close_asked = false,
            Some(false) => self.close_now(ctx),
            None => {}
        }
    }

    fn close_now(&mut self, ctx: &egui::Context) {
        self.close_asked = false;
        self.close_confirmed = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Saves the scene and the open graph, asking for a file where one has none. False when a
    /// save failed or was cancelled; the status bar says which.
    fn save_everything(&mut self) -> bool {
        if self.session.is_playing() {
            self.dispatch("play.stop", &Value::Null, None);
        }
        if self.session.is_dirty() {
            let saved = if self.session.scene_path().is_some() {
                self.dispatch("scene.save", &Value::Null, None)
            } else {
                match rfd::FileDialog::new().add_filter("Bayleaf scene", &[EXTENSION]).set_file_name(format!("scene.{EXTENSION}")).save_file() {
                    Some(path) => self.dispatch("scene.saveAs", &json!({ "path": path }), None),
                    None => None,
                }
            };
            if saved.is_none() {
                return false;
            }
        }
        if let Some(graph) = self.session.graph().filter(|g| g.dirty).cloned() {
            let saved = match (&graph.path, graph.catalog()) {
                (Some(_), _) => self.dispatch("graph.save", &Value::Null, None),
                (None, Some(catalog)) => match rfd::FileDialog::new().add_filter(catalog.label, &[catalog.extension]).save_file() {
                    Some(path) => self.dispatch("graph.save", &json!({ "path": path.with_extension(catalog.extension) }), None),
                    None => None,
                },
                (None, None) => None,
            };
            if saved.is_none() {
                return false;
            }
        }
        true
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        // Only typing blocks shortcuts; a focused button or the scene view does not.
        if ctx.text_edit_focused() {
            return;
        }
        // While a graph node or pin has focus, plain keys such as Delete and the arrows belong to the graph.
        let graph_keys = self.graph_view.focused;
        // Likewise a file in the Project panel: Delete moves the file, not the selected object.
        let panel_keys = graph_keys || self.project.focused;
        // F2 renames the selected object in place, in the hierarchy.
        if let Some(id) = self.selected
            && !panel_keys
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F2))
        {
            self.renaming = Some(hierarchy::Rename { id, text: String::new(), focused: false });
        }
        // Ctrl+C, Ctrl+X and Ctrl+V arrive as clipboard events, not as keys.
        let (copy, cut, paste) = ctx.input(|i| {
            let mut found = (false, false, None);
            for event in &i.events {
                match event {
                    egui::Event::Copy => found.0 = true,
                    egui::Event::Cut => found.1 = true,
                    egui::Event::Paste(text) => found.2 = Some(text.clone()),
                    _ => {}
                }
            }
            found
        });
        // While a graph node has focus, the clipboard keys and Ctrl+D act on the graph's nodes.
        if graph_keys {
            let duplicate = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::D));
            graph_view::clipboard_keys(self, copy, cut, paste, duplicate);
        } else {
            if copy || cut {
                self.copy_selection(cut);
            }
            if let Some(text) = paste {
                self.paste(Some(text));
            }
        }
        // Only enabled commands: Play and Stop share Ctrl+P, and only one of them is enabled at a time.
        let shortcuts: Vec<(&'static str, egui::KeyboardShortcut)> = self
            .registry
            .list(&self.session)
            .into_iter()
            .filter(|c| c.enabled)
            .filter_map(|c| parse_shortcut(c.shortcut).map(|s| (c.id, s)))
            .filter(|(_, s)| !(panel_keys && s.modifiers.is_none()))
            .collect();
        for (id, shortcut) in shortcuts {
            if ctx.input_mut(|i| i.consume_shortcut(&shortcut)) {
                self.run_with_prompt(id);
            }
        }
    }

    /// Runs a command, first asking the user for what its [`Prompt`] needs.
    fn run_with_prompt(&mut self, id: &str) {
        let Some(info) = self.registry.list(&self.session).into_iter().find(|c| c.id == id) else {
            return;
        };
        if matches!(id, "scene.new" | "scene.open" | "project.new" | "project.open") && self.session.is_dirty() && !confirm_discard() {
            return;
        }
        let params = match info.prompt {
            Prompt::None => Value::Null,
            Prompt::OpenScene => match rfd::FileDialog::new().add_filter("Bayleaf scene", &[EXTENSION]).pick_file() {
                Some(path) => json!({ "path": path }),
                None => return,
            },
            Prompt::SaveScene => match rfd::FileDialog::new().add_filter("Bayleaf scene", &[EXTENSION]).set_file_name(format!("scene.{EXTENSION}")).save_file()
            {
                Some(path) => json!({ "path": path }),
                None => return,
            },
            Prompt::NewProject => match rfd::FileDialog::new().set_title("Name and place the new project folder").set_file_name("My Game").save_file() {
                Some(path) => json!({ "path": path }),
                None => return,
            },
            Prompt::OpenProject => match rfd::FileDialog::new().add_filter("Bayleaf project", &[bayleaf_engine::project_cmds::PROJECT_EXTENSION]).pick_file() {
                Some(path) => json!({ "path": path }),
                None => return,
            },
            // Deleting and duplicating act on every selected object, as one undo step.
            Prompt::SelectedObject if !self.also_selected.is_empty() && matches!(id, "scene.object.delete" | "scene.object.duplicate") => {
                let results = self.for_each_selected(id);
                if id == "scene.object.duplicate" {
                    let copies: Vec<StableId> = results.iter().filter_map(|r| r["id"].as_str().and_then(|s| StableId::parse_hex(s).ok())).collect();
                    self.select_only(copies.first().copied());
                    self.also_selected = copies.into_iter().skip(1).collect();
                }
                return;
            }
            Prompt::SelectedObject => match self.selected {
                Some(selected) => json!({ "id": selected }),
                None => {
                    self.status = format!("{}: select an object first.", info.label);
                    return;
                }
            },
        };
        self.dispatch(id, &params, None);
    }

    /// Runs a command and reports the outcome in the status bar. Returns the result on success.
    pub(crate) fn dispatch(&mut self, id: &str, params: &Value, merge_key: Option<&str>) -> Option<Value> {
        let label = self.registry.get(id).map_or(id, |c| c.label).trim_end_matches(['…', '.']);
        match self.registry.execute_merging(&mut self.session, id, params, merge_key) {
            Ok(result) => {
                self.status = format!("{label}: done");
                if matches!(id, "scene.object.duplicate")
                    && let Some(copy) = result.get("id").and_then(Value::as_str).and_then(|s| StableId::parse_hex(s).ok())
                {
                    self.selected = Some(copy);
                }
                if id == "scene.object.create"
                    && let Some(created) = result.get("id").and_then(Value::as_str).and_then(|s| StableId::parse_hex(s).ok())
                {
                    self.selected = Some(created);
                }
                if matches!(id, "graph.new" | "graph.open") {
                    self.show_panel(Panel::Graph);
                }
                Some(result)
            }
            Err(error) => {
                self.status = format!("{label}: {error}");
                None
            }
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let file = self.session.scene_path().and_then(|p| p.file_name()).map_or_else(|| "Untitled scene".to_owned(), |n| n.to_string_lossy().into_owned());
        let dirty = if self.session.is_dirty() { " (modified)" } else { "" };
        let title = format!("{file}{dirty} - Bayleaf Engine");
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

/// Asks before closing a graph with unsaved changes.
pub(crate) fn confirm_discard_graph() -> bool {
    let answer = rfd::MessageDialog::new()
        .set_title("Unsaved changes")
        .set_description("The graph has unsaved changes. Discard them?")
        .set_buttons(rfd::MessageButtons::YesNo)
        .show();
    answer == rfd::MessageDialogResult::Yes
}

fn confirm_discard() -> bool {
    let answer = rfd::MessageDialog::new()
        .set_title("Unsaved changes")
        .set_description("The scene has unsaved changes. Discard them?")
        .set_buttons(rfd::MessageButtons::YesNo)
        .show();
    answer == rfd::MessageDialogResult::Yes
}

/// Parses shortcuts such as `Ctrl+Shift+S` or `Delete`. `Ctrl` means Cmd on macOS.
fn parse_shortcut(text: &str) -> Option<egui::KeyboardShortcut> {
    let mut parts: Vec<&str> = text.split('+').collect();
    let key = egui::Key::from_name(parts.pop()?)?;
    let mut modifiers = egui::Modifiers::NONE;
    for part in parts {
        modifiers |= match part {
            "Ctrl" => egui::Modifiers::COMMAND,
            "Shift" => egui::Modifiers::SHIFT,
            "Alt" => egui::Modifiers::ALT,
            _ => return None,
        };
    }
    Some(egui::KeyboardShortcut::new(modifiers, key))
}

#[cfg(test)]
mod ui_tests;

#[cfg(test)]
mod tests {
    use super::parse_shortcut;

    #[test]
    fn every_registered_shortcut_parses() {
        let registry = bayleaf_engine::CommandRegistry::with_builtins();
        for command in registry.list(&bayleaf_engine::Session::default()) {
            if !command.shortcut.is_empty() {
                assert!(parse_shortcut(command.shortcut).is_some(), "{} has shortcut {:?}", command.id, command.shortcut);
            }
        }
    }
}
