//! L6: the egui editor shell. Panels change engine state only through registered commands.
//!
//! The menu bar and keyboard shortcuts are built from the command registry, so every menu entry
//! is also reachable from the CLI, the control channel and MCP. Panels are docked (see [`dock`])
//! and every colour and size comes from the theme tokens in [`theme`].

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod brand;
mod console;
mod dock;
mod hierarchy;
mod inspector;
mod project;
mod scene_view;
mod theme;
mod value_editor;

pub use dock::{Layout, Panel};

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
    selected: Option<StableId>,
    /// Bumped when an edit gesture (a drag, a text field) ends, so the next edit gets its own undo step.
    gesture: u64,
    title: String,
    scene_view: scene_view::SceneView,
    dock: egui_dock::DockState<Panel>,
    hierarchy_filter: String,
    /// The object being renamed in the hierarchy, if any.
    renaming: Option<hierarchy::Rename>,
    theme_installed: bool,
    /// Logo texture and whether it was made for the dark theme.
    logo: Option<(egui::TextureHandle, bool)>,
    project: project::ProjectView,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            registry: CommandRegistry::with_builtins(),
            session: Session::default(),
            status: String::new(),
            selected: None,
            gesture: 0,
            title: String::new(),
            scene_view: scene_view::SceneView::new(),
            dock: Layout::Standard.build(),
            hierarchy_filter: String::new(),
            renaming: None,
            theme_installed: false,
            logo: None,
            project: project::ProjectView::default(),
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
        self.run_play_frame(ui.ctx());
        egui::Panel::top("menu_bar").frame(egui::Frame::new().fill(t.backdrop).inner_margin(egui::Margin::symmetric(8, 4))).show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                self.logo_mark(ui);
                self.menus(ui);
            });
        });
        egui::Panel::bottom("status_bar").frame(egui::Frame::new().fill(t.backdrop).inner_margin(egui::Margin::symmetric(10, 4))).show(ui, |ui| {
            ui.horizontal(|ui| {
                if self.session.is_dirty() {
                    egui::Frame::new().fill(t.surface_active).corner_radius(theme::RADIUS).inner_margin(egui::Margin::symmetric(8, 1)).show(ui, |ui| {
                        ui.label(egui::RichText::new("Modified").small().color(t.text));
                    });
                }
                ui.label(egui::RichText::new(&self.status).color(t.text_muted));
            });
        });
        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.backdrop)).show(ui, |ui| dock::show(self, ui, gpu));
        self.update_title(ui.ctx());
        if self.selected.is_some_and(|id| !self.session.scene().contains(id)) {
            self.selected = None;
        }
    }

    /// Opens a scene file, asking first if the open scene has unsaved changes.
    pub fn open_scene(&mut self, path: &std::path::Path) {
        if self.session.is_dirty() && !confirm_discard() {
            return;
        }
        self.dispatch("scene.open", &json!({ "path": path }), None);
    }

    /// In play mode: passes the held keys to the scripts and advances the simulation by one frame.
    fn run_play_frame(&mut self, ctx: &egui::Context) {
        if !self.session.is_playing() {
            return;
        }
        // Keys typed into a text field are not game input.
        let keys = if ctx.text_edit_focused() { Default::default() } else { ctx.input(|i| i.keys_down.iter().map(|k| k.name().to_owned()).collect()) };
        self.session.set_keys(keys);
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        self.session.tick(dt);
        if !self.session.is_paused() {
            ctx.request_repaint();
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
            });
        }
        if let Some(id) = clicked {
            self.run_with_prompt(id);
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        // Only typing blocks shortcuts; a focused button or the scene view does not.
        if ctx.text_edit_focused() {
            return;
        }
        // F2 renames the selected object in place, in the hierarchy.
        if let Some(id) = self.selected
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F2))
        {
            self.renaming = Some(hierarchy::Rename { id, text: String::new(), focused: false });
        }
        // Only enabled commands: Play and Stop share Ctrl+P, and only one of them is enabled at a time.
        let shortcuts: Vec<(&'static str, egui::KeyboardShortcut)> =
            self.registry.list(&self.session).into_iter().filter(|c| c.enabled).filter_map(|c| parse_shortcut(c.shortcut).map(|s| (c.id, s))).collect();
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
        if matches!(id, "scene.new" | "scene.open") && self.session.is_dirty() && !confirm_discard() {
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
        let label = self.registry.get(id).map_or(id, |c| c.label);
        match self.registry.execute_merging(&mut self.session, id, params, merge_key) {
            Ok(result) => {
                self.status = format!("{label}: done");
                if id == "scene.object.create"
                    && let Some(created) = result.get("id").and_then(Value::as_str).and_then(|s| StableId::parse_hex(s).ok())
                {
                    self.selected = Some(created);
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
