//! Inspector panel: the components of the selected object, edited in place.

use serde_json::{Value, json};

use crate::Editor;
use crate::value_editor;

enum Pending {
    Set { component: &'static str, value: Value },
    Add(&'static str),
    Remove(&'static str),
}

pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    ui.heading("Inspector");
    let Some(id) = editor.selected else {
        ui.label("Select an object in the hierarchy.");
        return;
    };
    let scene = editor.session.scene();
    let Ok(components) = scene.components(id) else {
        ui.label("The selected object no longer exists.");
        return;
    };
    let kinds = scene.registry().kinds().to_vec();
    let mut pending = None;
    let mut gesture_ended = false;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for (name, value) in &components {
            let Some(kind) = kinds.iter().find(|k| k.name == *name) else { continue };
            egui::CollapsingHeader::new(kind.label).id_salt(("component", *name)).default_open(true).show(ui, |ui| {
                let mut edited = value.clone();
                let outcome = if edited.is_object() || edited.is_array() {
                    value_editor::edit(ui, &mut edited, ("value", *name), None)
                } else {
                    ui.horizontal(|ui| {
                        let label = ui.label(kind.label);
                        value_editor::edit(ui, &mut edited, ("value", *name), Some(label.id))
                    })
                    .inner
                };
                gesture_ended |= outcome.ended;
                if outcome.changed {
                    pending = Some(Pending::Set { component: kind.name, value: edited });
                }
                if kind.removable && ui.button(format!("Remove {}", kind.label)).clicked() {
                    pending = Some(Pending::Remove(kind.name));
                }
            });
        }
        ui.separator();
        let missing: Vec<_> = kinds.iter().filter(|k| !components.iter().any(|(n, _)| n == &k.name)).collect();
        ui.add_enabled_ui(!missing.is_empty(), |ui| {
            ui.menu_button("Add Component", |ui| {
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
