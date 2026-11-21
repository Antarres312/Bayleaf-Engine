//! Inspector panel: the selected object's name and its components as cards, edited in place.
//! Its width comes from the dock; nothing inside sets it.

use serde_json::{Value, json};

use crate::theme::{self, ROW_HEIGHT};
use crate::{Editor, value_editor};

enum Pending {
    Set { component: &'static str, value: Value },
    Add(&'static str),
    Remove(&'static str),
}

pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    let t = theme::tokens(ui);
    let Some(id) = editor.selected else {
        empty_state(ui, "Nothing selected", "Select an object in the hierarchy or click it in the scene view.");
        return;
    };
    let scene = editor.session.scene();
    let Ok(components) = scene.components(id) else {
        empty_state(ui, "Object not found", "The selected object no longer exists.");
        return;
    };
    let kinds = scene.registry().kinds().to_vec();
    let mut pending = None;
    let mut gesture_ended = false;

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
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

        for (name, value) in components.iter().filter(|(n, _)| *n != "name") {
            let Some(kind) = kinds.iter().find(|k| k.name == *name) else { continue };
            theme::card(t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let state_id = ui.make_persistent_id(("component card", *name));
                egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), state_id, true)
                    .show_header(ui, |ui| {
                        ui.label(egui::RichText::new(kind.label).strong());
                        if kind.removable {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("Remove").on_hover_text(format!("Remove the {} component", kind.label)).clicked() {
                                    pending = Some(Pending::Remove(kind.name));
                                }
                            });
                        }
                    })
                    .body(|ui| {
                        let mut edited = value.clone();
                        let outcome = value_editor::edit_fields(ui, &mut edited, kind.name, &|field| kind.options(field));
                        gesture_ended |= outcome.ended;
                        if outcome.changed {
                            pending = Some(Pending::Set { component: kind.name, value: edited });
                        }
                    });
            });
            ui.add_space(6.0);
        }

        let missing: Vec<_> = kinds.iter().filter(|k| !components.iter().any(|(n, _)| n == &k.name)).collect();
        ui.add_space(4.0);
        ui.add_enabled_ui(!missing.is_empty(), |ui| {
            let width = ui.available_width();
            egui::containers::menu::MenuButton::from_button(egui::Button::new("Add Component").min_size(egui::vec2(width, ROW_HEIGHT + 2.0))).ui(ui, |ui| {
                ui.set_min_width(width.min(260.0));
                for kind in missing {
                    if ui.button(kind.label).clicked() {
                        pending = Some(Pending::Add(kind.name));
                        ui.close();
                    }
                }
            });
        });
    });

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
        None => {}
    }
    if gesture_ended {
        editor.gesture += 1;
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
