//! L6: the egui editor shell. Panels change engine state only through registered commands.
//!
//! The menu bar and keyboard shortcuts are built from the command registry, so every menu entry
//! is also reachable from the CLI, the control channel and MCP. Theme tokens from craft-ui are
//! not wired in yet; panels use egui's default style until then.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod hierarchy;
mod inspector;
mod value_editor;

use bayleaf_core::StableId;
use bayleaf_engine::scene::doc::EXTENSION;
use bayleaf_engine::{CommandInfo, CommandRegistry, Prompt, Session};
use serde_json::{Value, json};

/// Menus in the order they appear; menus not listed here follow in registry order.
const MENU_ORDER: &[&str] = &["File", "Edit", "Object", "Help"];

/// Editor state shared by every panel.
pub struct Editor {
    registry: CommandRegistry,
    session: Session,
    status: String,
    selected: Option<StableId>,
    /// Bumped when an edit gesture (a drag, a text field) ends, so the next edit gets its own undo step.
    gesture: u64,
    title: String,
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
        }
    }
}

impl Editor {
    /// Draws the editor for one frame.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.handle_shortcuts(ui.ctx());
        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| self.menus(ui));
        });
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                if self.session.is_dirty() {
                    ui.label("Modified");
                    ui.separator();
                }
                ui.label(&self.status);
            });
        });
        egui::Panel::left("hierarchy").resizable(true).default_size(240.0).show(ui, |ui| hierarchy::show(self, ui));
        egui::Panel::right("inspector").resizable(true).default_size(300.0).show(ui, |ui| inspector::show(self, ui));
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Scene");
            ui.label("The scene view arrives in milestone M2. Use the hierarchy and the inspector to edit the scene.");
        });
        self.update_title(ui.ctx());
        if self.selected.is_some_and(|id| !self.session.scene().contains(id)) {
            self.selected = None;
        }
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
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let shortcuts: Vec<(&'static str, egui::KeyboardShortcut)> =
            self.registry.list(&self.session).into_iter().filter_map(|c| parse_shortcut(c.shortcut).map(|s| (c.id, s))).collect();
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
