//! Inspector panel: the selected object's name and its components as cards, edited in place.
//! Each script on the object is its own card, named after its module, with the properties the
//! script declares. Its width comes from the dock; nothing inside sets it.

use bayleaf_engine::scene::components::{SceneComponent, ScriptRef, Scripts};
use serde_json::{Value, json};

use crate::theme::{self, ROW_HEIGHT};
use crate::{Editor, project, value_editor};

enum Pending {
    Set { component: &'static str, value: Value },
    Add(&'static str),
    Remove(&'static str),
    AddScript(String),
    RemoveScript(usize),
    ScriptProperty { index: usize, name: String, value: Option<f32> },
}

/// One script card's data, gathered before drawing.
struct ScriptCard {
    script: ScriptRef,
    /// Declared properties with their defaults, or why they could not be read.
    declared: Result<Vec<(String, f32)>, String>,
}

pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    let t = theme::tokens(ui);
    let Some(id) = editor.selected else {
        empty_state(ui, "Nothing selected", "Select an object in the hierarchy or click it in the scene view.");
        return;
    };
    let Ok(components) = editor.session.scene().components(id) else {
        empty_state(ui, "Object not found", "The selected object no longer exists.");
        return;
    };
    let kinds = editor.session.scene().registry().kinds().to_vec();
    let script_dir = editor.session.script_dir();
    let scripts: Vec<ScriptRef> = editor.session.scene().get::<Scripts>(id).map(|s| s.0.clone()).unwrap_or_default();
    let cards: Vec<ScriptCard> =
        scripts.into_iter().map(|script| ScriptCard { declared: editor.script_info.properties(&script_dir.join(&script.module)), script }).collect();
    let script_files = editor.project.script_files();
    let mut pending = None;
    let mut gesture_ended = false;

    let panel = egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(4.0);
        // Name: a large field at the top instead of a card.
        if let Some((_, Value::String(name))) = components.iter().find(|(n, _)| *n == "name") {
            let mut edited = name.clone();
            let label = ui.label(egui::RichText::new("Name").small().color(t.text_muted));
            let response = ui
                .add_sized(
                    [ui.available_width(), ROW_HEIGHT + 4.0],
                    egui::TextEdit::singleline(&mut edited).font(egui::TextStyle::Heading).align(egui::Align2::LEFT_CENTER),
                )
                .labelled_by(label.id);
            gesture_ended |= response.lost_focus();
            if response.changed() {
                pending = Some(Pending::Set { component: "name", value: Value::String(edited) });
            }
            ui.label(egui::RichText::new(format!("ID {}", &id.to_hex()[..8])).small().color(t.text_muted)).on_hover_text(id.to_hex());
            ui.add_space(8.0);
        }

        for (name, value) in components.iter().filter(|(n, _)| *n != "name" && *n != Scripts::NAME) {
            let Some(kind) = kinds.iter().find(|k| k.name == *name) else { continue };
            card(ui, t, ("component card", *name), kind.label, None, kind.removable, |ui| {
                let mut edited = value.clone();
                let outcome = value_editor::edit_fields(ui, &mut edited, kind.name, &|field| kind.options(field));
                gesture_ended |= outcome.ended;
                if outcome.changed {
                    pending = Some(Pending::Set { component: kind.name, value: edited });
                }
            })
            .then(|| pending = Some(Pending::Remove(kind.name)));
        }

        for (index, card_data) in cards.iter().enumerate() {
            let title = card_data.script.display_name();
            let removed = card(ui, t, ("script card", index), &title, Some(&card_data.script.module), true, |ui| match &card_data.declared {
                Err(message) => {
                    ui.add(egui::Label::new(egui::RichText::new(format!("Warning: {message}")).color(t.warning)).wrap());
                }
                Ok(declared) => {
                    let outcome = script_properties(ui, index, &card_data.script, declared, &mut pending);
                    gesture_ended |= outcome.ended;
                }
            });
            if removed {
                pending = Some(Pending::RemoveScript(index));
            }
        }

        let missing: Vec<_> = kinds.iter().filter(|k| k.listed && !components.iter().any(|(n, _)| n == &k.name)).collect();
        ui.add_space(4.0);
        let width = ui.available_width();
        egui::containers::menu::MenuButton::from_button(egui::Button::new("Add Component").min_size(egui::vec2(width, ROW_HEIGHT + 2.0))).ui(ui, |ui| {
            ui.set_min_width(width.min(260.0));
            for kind in missing {
                if ui.button(kind.label).clicked() {
                    pending = Some(Pending::Add(kind.name));
                    ui.close();
                }
            }
            ui.separator();
            ui.label(egui::RichText::new("Scripts").small().color(t.text_muted));
            if script_files.is_empty() {
                ui.label(egui::RichText::new("No .wasm files in the project folder").color(t.text_muted));
            }
            for module in &script_files {
                let name = ScriptRef { module: module.clone(), properties: Default::default() }.display_name();
                if ui.button(name).on_hover_text(module).clicked() {
                    pending = Some(Pending::AddScript(module.clone()));
                    ui.close();
                }
            }
        });
        ui.label(egui::RichText::new("Tip: drag a script from the Project panel onto this panel.").small().color(t.text_muted));
    });

    // A script dragged from the Project panel and dropped anywhere on the inspector is added.
    let area = panel.inner_rect;
    if let Some(asset) = egui::DragAndDrop::payload::<project::DraggedAsset>(ui.ctx())
        && asset.relative.ends_with(".wasm")
        && ui.ctx().pointer_interact_pos().is_some_and(|p| area.contains(p))
    {
        ui.painter().rect_stroke(area, theme::RADIUS, egui::Stroke::new(2.0, t.focus), egui::StrokeKind::Inside);
        if ui.input(|i| i.pointer.any_released()) {
            egui::DragAndDrop::clear_payload(ui.ctx());
            pending = Some(Pending::AddScript(asset.relative.clone()));
        }
    }

    apply(editor, id, pending);
    if gesture_ended {
        editor.gesture += 1;
    }
}

/// A collapsible card with a title, an optional muted subtitle and a Remove button.
/// Returns true when Remove was clicked.
fn card(
    ui: &mut egui::Ui,
    t: &theme::Tokens,
    salt: impl std::hash::Hash + std::fmt::Debug,
    title: &str,
    subtitle: Option<&str>,
    removable: bool,
    body: impl FnOnce(&mut egui::Ui),
) -> bool {
    let mut removed = false;
    theme::card(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let state_id = ui.make_persistent_id(salt);
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), state_id, true)
            .show_header(ui, |ui| {
                let label = ui.label(egui::RichText::new(title).strong());
                if let Some(subtitle) = subtitle {
                    label.on_hover_text(subtitle);
                }
                if removable {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let button = ui.small_button("Remove");
                        // Several cards have a Remove button; the accessible name says which.
                        button.ctx.accesskit_node_builder(button.id, |node| node.set_label(format!("Remove {title}")));
                        if button.on_hover_text(format!("Remove {title}")).clicked() {
                            removed = true;
                        }
                    });
                }
            })
            .body(body);
    });
    ui.add_space(6.0);
    removed
}

/// The declared properties of one script, plus any saved values it no longer declares.
fn script_properties(ui: &mut egui::Ui, index: usize, script: &ScriptRef, declared: &[(String, f32)], pending: &mut Option<Pending>) -> value_editor::Outcome {
    let t = theme::tokens(ui);
    let mut outcome = value_editor::Outcome::default();
    if declared.is_empty() && script.properties.is_empty() {
        ui.label(egui::RichText::new("This script declares no properties.").color(t.text_muted));
        return outcome;
    }
    let mut rows: Vec<(String, f32, bool)> =
        declared.iter().map(|(name, default)| (name.clone(), *script.properties.get(name).unwrap_or(default), script.properties.contains_key(name))).collect();
    for (name, value) in &script.properties {
        if !declared.iter().any(|(d, _)| d == name) {
            rows.push((name.clone(), *value, true));
        }
    }
    for (name, value, overridden) in rows {
        let mut field = Value::from(f64::from(value));
        let (row_outcome, reset) = value_editor::named_number(ui, &name, &mut field, overridden, ("script property", index, name.as_str()));
        outcome.ended |= row_outcome.ended;
        if row_outcome.changed {
            let number = field.as_f64().map(|v| v as f32);
            *pending = Some(Pending::ScriptProperty { index, name: name.clone(), value: number });
        }
        if reset {
            *pending = Some(Pending::ScriptProperty { index, name, value: None });
        }
    }
    outcome
}

fn apply(editor: &mut Editor, id: bayleaf_core::StableId, pending: Option<Pending>) {
    match pending {
        Some(Pending::Set { component, value }) => {
            let key = format!("set:{id}:{component}:{}", editor.gesture);
            editor.dispatch("scene.component.set", &json!({ "id": id, "component": component, "value": value }), Some(&key));
        }
        Some(Pending::Add(component)) => {
            editor.dispatch("scene.component.add", &json!({ "id": id, "component": component }), None);
        }
        Some(Pending::Remove(component)) => {
            editor.dispatch("scene.component.remove", &json!({ "id": id, "component": component }), None);
        }
        Some(Pending::AddScript(module)) => {
            editor.dispatch("scene.script.add", &json!({ "id": id, "module": module }), None);
        }
        Some(Pending::RemoveScript(index)) => {
            editor.dispatch("scene.script.remove", &json!({ "id": id, "index": index }), None);
        }
        Some(Pending::ScriptProperty { index, name, value }) => {
            let key = format!("script property:{id}:{index}:{name}:{}", editor.gesture);
            editor.dispatch("scene.script.setProperty", &json!({ "id": id, "index": index, "name": name, "value": value }), Some(&key));
        }
        None => {}
    }
}

/// Centred title and hint for a panel with nothing to show.
pub(crate) fn empty_state(ui: &mut egui::Ui, title: &str, hint: &str) {
    let t = theme::tokens(ui);
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.3).min(120.0));
        ui.label(egui::RichText::new(title).strong());
        ui.add(egui::Label::new(egui::RichText::new(hint).color(t.text_muted)).wrap());
    });
}
