//! Hierarchy panel: the object tree, with buttons to create, delete and move objects.
//! Every move also works without dragging (accessibility standard).

use bayleaf_core::StableId;
use bayleaf_engine::scene::Scene;
use serde_json::{Value, json};

use crate::Editor;

/// Width of the open/close arrow, so leaves line up with branches.
const ARROW_WIDTH: f32 = 18.0;

pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    ui.heading("Hierarchy");
    toolbar(editor, ui);
    ui.separator();
    let mut selected = editor.selected;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let scene = editor.session.scene();
        if scene.object_count() == 0 {
            ui.label("The scene is empty. Use Create to add an object.");
        }
        tree(ui, scene, None, &mut selected);
    });
    editor.selected = selected;
}

fn tree(ui: &mut egui::Ui, scene: &Scene, parent: Option<StableId>, selected: &mut Option<StableId>) {
    for &id in scene.children(parent) {
        let name = scene.name(id).unwrap_or("Object").to_owned();
        let is_selected = *selected == Some(id);
        if scene.children(Some(id)).is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(ARROW_WIDTH);
                if ui.selectable_label(is_selected, name).clicked() {
                    *selected = Some(id);
                }
            });
        } else {
            let state_id = ui.make_persistent_id(("hierarchy", id.as_u128()));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), state_id, true)
                .show_header(ui, |ui| {
                    if ui.selectable_label(is_selected, name).clicked() {
                        *selected = Some(id);
                    }
                })
                .body(|ui| tree(ui, scene, Some(id), selected));
        }
    }
}

/// A planned move of the selected object: `scene.object.reparent` params.
fn planned_move(scene: &Scene, id: StableId, action: Move) -> Option<Value> {
    let parent = scene.parent(id);
    let siblings = scene.children(parent);
    let index = siblings.iter().position(|s| *s == id)?;
    match action {
        Move::Up => (index > 0).then(|| json!({ "id": id, "parent": parent, "index": index - 1 })),
        Move::Down => (index + 1 < siblings.len()).then(|| json!({ "id": id, "parent": parent, "index": index + 1 })),
        Move::Indent => {
            let new_parent = *siblings.get(index.checked_sub(1)?)?;
            Some(json!({ "id": id, "parent": new_parent }))
        }
        Move::Outdent => {
            let old_parent = parent?;
            let grandparent = scene.parent(old_parent);
            let at = scene.children(grandparent).iter().position(|s| *s == old_parent)? + 1;
            Some(json!({ "id": id, "parent": grandparent, "index": at }))
        }
    }
}

/// Ready-made objects: `scene.object.create` params with their components.
fn presets() -> Vec<(&'static str, Value)> {
    let mesh = |name: &str, shape: &str, position: [f32; 3], scale: [f32; 3]| {
        json!({
            "name": name,
            "components": {
                "mesh": {"shape": shape, "color": [0.8, 0.8, 0.8, 1.0], "metallic": 0.0, "roughness": 0.5},
                "transform": {"position": position, "rotation": [0.0, 0.0, 0.0], "scale": scale},
            },
        })
    };
    vec![
        ("Cube", mesh("Cube", "cube", [0.0, 0.5, 0.0], [1.0; 3])),
        ("Sphere", mesh("Sphere", "sphere", [0.0, 0.5, 0.0], [1.0; 3])),
        ("Plane", mesh("Plane", "plane", [0.0, 0.0, 0.0], [10.0, 1.0, 10.0])),
        (
            "Sprite",
            json!({"name": "Sprite", "components": {"sprite": null, "transform": {"position": [0.0, 0.5, 0.0], "rotation": [0.0, 0.0, 0.0], "scale": [1.0, 1.0, 1.0]}}}),
        ),
        (
            "Camera",
            json!({"name": "Camera", "components": {"camera": null, "transform": {"position": [0.0, 2.0, 6.0], "rotation": [-15.0, 0.0, 0.0], "scale": [1.0, 1.0, 1.0]}}}),
        ),
        (
            "Directional Light",
            json!({"name": "Directional Light", "components": {"directionalLight": null, "transform": {"position": [0.0, 5.0, 0.0], "rotation": [-50.0, 30.0, 0.0], "scale": [1.0, 1.0, 1.0]}}}),
        ),
    ]
}

#[derive(Clone, Copy)]
enum Move {
    Up,
    Down,
    Indent,
    Outdent,
}

fn toolbar(editor: &mut Editor, ui: &mut egui::Ui) {
    let selected = editor.selected;
    let mut action: Option<(&'static str, Value)> = None;
    ui.horizontal_wrapped(|ui| {
        let create_label = if selected.is_some() { "Create Child" } else { "Create" };
        if ui.button(create_label).on_hover_text("Create an empty object").clicked() {
            action = Some(("scene.object.create", json!({ "parent": selected })));
        }
        ui.menu_button("Add", |ui| {
            for (label, params) in presets() {
                if ui.button(label).clicked() {
                    action = Some(("scene.object.create", params));
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Add a ready-made object at the root of the scene");
        if ui.add_enabled(selected.is_some(), egui::Button::new("Delete")).clicked()
            && let Some(id) = selected
        {
            action = Some(("scene.object.delete", json!({ "id": id })));
        }
        let scene = editor.session.scene();
        for (label, hint, kind) in [
            ("Up", "Move before the previous sibling", Move::Up),
            ("Down", "Move after the next sibling", Move::Down),
            ("Indent", "Make a child of the previous sibling", Move::Indent),
            ("Outdent", "Move out of the parent", Move::Outdent),
        ] {
            let params = selected.and_then(|id| planned_move(scene, id, kind));
            if ui.add_enabled(params.is_some(), egui::Button::new(label)).on_hover_text(hint).clicked()
                && let Some(params) = params
            {
                action = Some(("scene.object.reparent", params));
            }
        }
    });
    if let Some((id, params)) = action {
        editor.dispatch(id, &params, None);
    }
}
