//! Hierarchy panel: the object tree.
//!
//! Drag an object onto another to make it a child; drop it on the top or bottom edge of a row to
//! place it before or after that object; drop it on the empty space below the tree to move it to
//! the root. Without dragging, the same moves are in each row's context menu and in the Object
//! menu with keyboard shortcuts (accessibility standard: anything done by dragging also works
//! without dragging).

use crate::i18n::tl;
use bayleaf_core::StableId;
use bayleaf_engine::scene::Scene;
use serde_json::{Value, json};

use crate::theme::{self, LIST_ROW, ROW_HEIGHT, Tokens};
use crate::{Editor, inspector};

/// Width of the open/close arrow, so leaves line up with branches.
const ARROW_WIDTH: f32 = 18.0;
/// Share of a row's height at the top and bottom that means "before" and "after" instead of "into".
const EDGE_ZONE: f32 = 0.28;

/// Row context menu: label and the command it runs on the row's object.
const ROW_MENU: &[(&str, &str)] = &[
    ("Move Up", "scene.object.moveUp"),
    ("Move Down", "scene.object.moveDown"),
    ("Make Child of Previous", "scene.object.indent"),
    ("Move Out of Parent", "scene.object.outdent"),
    ("Duplicate", "scene.object.duplicate"),
    ("Save as Prefab…", SAVE_PREFAB),
    ("Copy", COPY),
    ("Paste After", PASTE),
    ("Delete", "scene.object.delete"),
];

/// Context-menu entries handled by the editor (they use its clipboard), not engine commands.
const COPY: &str = "editor.copy";
const PASTE: &str = "editor.paste";
const SAVE_PREFAB: &str = "editor.savePrefab";

/// Drag-and-drop payload: the object being dragged.
#[derive(Clone, Copy)]
struct Dragged(StableId);

/// The object being dragged from the hierarchy, if one is.
pub(crate) fn dragged_object(ctx: &egui::Context) -> Option<StableId> {
    egui::DragAndDrop::payload::<Dragged>(ctx).map(|d| d.0)
}

/// Where an object would land if dropped now.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Drop {
    Before(StableId),
    After(StableId),
    Into(StableId),
    Root,
}

/// What the rows report back while the tree is drawn.
#[derive(Default)]
struct RowState {
    selected: Option<StableId>,
    /// The other selected objects.
    also: Vec<StableId>,
    /// A click with Ctrl or Shift held, resolved once every row is drawn.
    modified_click: Option<(StableId, egui::Modifiers)>,
    rows: Vec<(StableId, egui::Rect)>,
    /// A context-menu command and the object it applies to.
    command: Option<(&'static str, StableId)>,
    /// The object being renamed in place, if any.
    renaming: Option<Rename>,
    /// A finished rename to apply: object and new name.
    commit: Option<(StableId, String)>,
    /// The scene's own row is selected.
    scene_selected: bool,
    /// Where the scene's row is, as a place to drop objects to the top.
    scene_row: Option<egui::Rect>,
}

/// An in-place rename: started by a double-click, F2, or the context menu.
#[derive(Clone, Debug)]
pub(crate) struct Rename {
    pub id: StableId,
    pub text: String,
    /// False until the field has taken keyboard focus once.
    pub focused: bool,
}

pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    let t = theme::tokens(ui);
    ui.add_space(4.0);
    toolbar(editor, ui);
    ui.add_space(2.0);
    ui.separator();
    let mut state = RowState {
        selected: editor.selected,
        also: editor.also_selected.clone(),
        renaming: editor.renaming.take(),
        scene_selected: editor.scene_selected,
        ..RowState::default()
    };
    let scene_name = editor.session.scene_path().and_then(|p| p.file_stem()).map_or_else(|| tl("Untitled").to_owned(), |n| n.to_string_lossy().into_owned());
    let mut moved: Option<Value> = None;
    let mut script_dropped: Option<(StableId, String)> = None;
    let filter = editor.hierarchy_filter.trim().to_lowercase();
    let area = egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        theme::dense(ui);
        // Reserved first, so the stripes are painted under the rows drawn after it.
        let stripe_slot = ui.painter().add(egui::Shape::Noop);
        let full_width = ui.max_rect().x_range();
        let scene = editor.session.scene();
        if filter.is_empty() {
            // The scene itself heads the tree and holds every top object.
            let state_id = ui.make_persistent_id("hierarchy scene");
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), state_id, true)
                .show_header(ui, |ui| scene_row(ui, &scene_name, &mut state))
                .body(|ui| {
                    if scene.object_count() == 0 {
                        inspector::empty_state(ui, "Empty scene", "Use + to add an object.");
                    } else {
                        tree(ui, scene, None, &mut state);
                    }
                });
            theme::stripes(ui, stripe_slot, state.rows.iter().map(|(_, r)| *r), full_width);
            // The space below the tree, and the scene's row, take drops to the top.
            let root_zone = ui.allocate_space(egui::vec2(ui.available_width(), ui.available_height().max(ROW_HEIGHT * 2.0))).1;
            moved = handle_drop(ui, t, scene, &state.rows, root_zone, state.scene_row);
        } else {
            let matches: Vec<StableId> =
                scene.world_matrices().into_iter().map(|(id, _)| id).filter(|id| scene.name(*id).is_some_and(|n| n.to_lowercase().contains(&filter))).collect();
            if matches.is_empty() {
                inspector::empty_state(ui, "No matches", "No object name contains the filter text.");
            }
            for id in matches {
                draggable_row(ui, id, scene.name(id).unwrap_or("Object"), is_prefab(scene, id), &mut state);
            }
            theme::stripes(ui, stripe_slot, state.rows.iter().map(|(_, r)| *r), full_width);
        }
        script_dropped = script_drop(ui, t, &state.rows);
    });
    if state.scene_selected && !editor.scene_selected {
        editor.select_scene();
    } else if state.selected.is_some() {
        editor.scene_selected = false;
    }
    if !state.scene_selected || state.selected.is_some() {
        editor.selected = state.selected;
        editor.also_selected = std::mem::take(&mut state.also);
    }
    if let Some((id, modifiers)) = state.modified_click {
        if modifiers.shift
            && let Some(anchor) = editor.selected
        {
            // Shift: everything between the primary object and this row, in the order shown.
            let order: Vec<StableId> = state.rows.iter().map(|(row, _)| *row).collect();
            if let (Some(a), Some(b)) = (order.iter().position(|r| *r == anchor), order.iter().position(|r| *r == id)) {
                let (from, to) = (a.min(b), a.max(b));
                editor.select_only(Some(id));
                editor.also_selected = order[from..=to].iter().copied().filter(|r| *r != id).collect();
            }
        } else {
            editor.toggle_selected(id);
        }
    }
    // Files from the system: models and sounds become objects at the top.
    if let Some(files) = crate::file_drop::take(ui, area.inner_rect) {
        crate::file_drop::bring_in(editor, files, None, crate::file_drop::Place::Top);
    }
    if let Some(params) = moved {
        editor.dispatch("scene.object.reparent", &params, None);
    }
    if let Some((id, module)) = script_dropped {
        editor.dispatch("scene.script.add", &json!({ "id": id, "module": module }), None);
    }
    match state.command {
        Some((COPY, _)) => editor.copy_selection(false),
        Some((PASTE, _)) => editor.paste(None),
        Some((SAVE_PREFAB, id)) => editor.save_as_prefab(id),
        Some((command, id)) => {
            editor.dispatch(command, &json!({ "id": id }), None);
        }
        None => {}
    }
    editor.renaming = state.renaming;
    if let Some((id, name)) = state.commit {
        let unchanged = editor.session.scene().name(id) == Some(name.as_str());
        if !name.is_empty() && !unchanged {
            editor.dispatch("scene.component.set", &json!({ "id": id, "component": "name", "value": name }), None);
        }
    }
}

/// Right-click menu of a row. Opening it also selects the row's object.
fn row_menu(response: &egui::Response, id: StableId, state: &mut RowState) {
    response.context_menu(|ui| {
        state.selected = Some(id);
        if ui.button(tl("Rename")).on_hover_text(tl("F2 or double-click")).clicked() {
            state.renaming = Some(Rename { id, text: String::new(), focused: false });
            ui.close();
        }
        for (label, command) in ROW_MENU {
            if ui.button(*label).clicked() {
                state.command = Some((command, id));
                ui.close();
            }
        }
    });
}

/// The in-place name field of the object being renamed. Enter or leaving the field keeps the new
/// name; Escape keeps the old one.
fn rename_field(ui: &mut egui::Ui, id: StableId, current: &str, width: f32, state: &mut RowState) {
    let Some(rename) = state.renaming.as_mut() else { return };
    if !rename.focused {
        rename.text = current.to_owned();
    }
    let response = ui.add_sized([width, LIST_ROW], egui::TextEdit::singleline(&mut rename.text).margin(egui::vec2(6.0, 1.0)).align(egui::Align2::LEFT_CENTER));
    ui.ctx().accesskit_node_builder(response.id, |node| node.set_label("Object name"));
    if !rename.focused {
        response.request_focus();
        // Select the whole name, so typing replaces it.
        if let Some(mut text_state) = egui::text_edit::TextEditState::load(ui.ctx(), response.id) {
            let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(rename.text.chars().count()));
            text_state.cursor.set_char_range(Some(all));
            text_state.store(ui.ctx(), response.id);
        }
        rename.focused = true;
    } else if response.lost_focus() {
        let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let text = rename.text.trim().to_owned();
        state.renaming = None;
        if !cancelled {
            state.commit = Some((id, text));
        }
    }
    state.rows.push((id, response.rect));
}

/// A row that can also be dragged with the primary button. Records its rectangle for drop targeting.
/// The secondary button stays free for the context menu.
fn draggable_row(ui: &mut egui::Ui, id: StableId, name: &str, prefab: bool, state: &mut RowState) {
    let width = ui.available_width();
    if state.renaming.as_ref().is_some_and(|r| r.id == id) {
        rename_field(ui, id, name, width, state);
        return;
    }
    let is_selected = state.selected == Some(id) || state.also.contains(&id);
    // A placed prefab says so after its name, in words, not only by colour.
    let t = theme::tokens(ui);
    // No empty marker on other rows: it would add a space to their accessible name.
    let button = if prefab {
        egui::Button::selectable(is_selected, (name, egui::Atom::grow(), egui::RichText::new(tl("prefab")).small().color(t.text_muted)))
    } else {
        egui::Button::selectable(is_selected, (name, egui::Atom::grow()))
    };
    let button = button.truncate().min_size(egui::vec2(width, LIST_ROW)).sense(egui::Sense::click_and_drag());
    let response = ui.add(button);
    let modifiers = ui.input(|i| i.modifiers);
    let adds = modifiers.command || modifiers.shift;
    if response.clicked() {
        if adds {
            state.modified_click = Some((id, modifiers));
        } else {
            state.selected = Some(id);
            state.also.clear();
        }
    }
    if response.double_clicked() {
        state.selected = Some(id);
        state.renaming = Some(Rename { id, text: String::new(), focused: false });
    }
    // Select on press, like a click, so a click with a little hand movement still selects.
    if !adds && response.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed()) && !is_selected {
        state.selected = Some(id);
        state.also.clear();
    }
    if response.drag_started_by(egui::PointerButton::Primary) {
        if !is_selected {
            state.also.clear();
        }
        state.selected = Some(id);
        egui::DragAndDrop::set_payload(ui.ctx(), Dragged(id));
    }
    if response.dragged_by(egui::PointerButton::Primary)
        && let Some(pointer) = ui.ctx().pointer_interact_pos()
    {
        // A ghost of the name follows the pointer.
        let t = theme::tokens(ui);
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("hierarchy drag ghost")));
        let galley = painter.layout_no_wrap(name.to_owned(), egui::TextStyle::Body.resolve(ui.style()), t.on_accent);
        let rect = egui::Rect::from_min_size(pointer + egui::vec2(12.0, 4.0), galley.size() + egui::vec2(16.0, 8.0));
        painter.rect_filled(rect, theme::RADIUS, t.accent);
        painter.galley(rect.min + egui::vec2(8.0, 4.0), galley, t.on_accent);
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    row_menu(&response, id, state);
    state.rows.push((id, response.rect));
}

/// The scene's own row, named after its file: selecting it shows the scene in the Inspector. It is
/// not an object, so it cannot be dragged, renamed or deleted; objects dropped on it go to the top.
fn scene_row(ui: &mut egui::Ui, name: &str, state: &mut RowState) {
    let t = theme::tokens(ui);
    let selected = state.scene_selected && state.selected.is_none();
    let button = egui::Button::selectable(selected, (name, egui::Atom::grow(), egui::RichText::new(tl("scene")).small().color(t.text_muted)))
        .truncate()
        .min_size(egui::vec2(ui.available_width(), LIST_ROW));
    let response = ui.add(button).on_hover_text(tl("The scene: select it to see its file and settings in the Inspector"));
    if response.clicked() {
        state.scene_selected = true;
        state.selected = None;
        state.also.clear();
    }
    state.scene_row = Some(response.rect);
}

fn is_prefab(scene: &Scene, id: StableId) -> bool {
    scene.get::<bayleaf_engine::scene::components::Prefab>(id).is_some()
}

fn tree(ui: &mut egui::Ui, scene: &Scene, parent: Option<StableId>, state: &mut RowState) {
    for &id in scene.children(parent) {
        let name = scene.name(id).unwrap_or("Object").to_owned();
        if scene.children(Some(id)).is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(ARROW_WIDTH);
                draggable_row(ui, id, &name, is_prefab(scene, id), state);
            });
        } else {
            let state_id = ui.make_persistent_id(("hierarchy", id.as_u128()));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), state_id, true)
                .show_header(ui, |ui| draggable_row(ui, id, &name, is_prefab(scene, id), state))
                .body(|ui| tree(ui, scene, Some(id), state));
        }
    }
}

/// While a script is dragged from the Project panel: outlines the object row under the pointer,
/// and on release returns that object and the script to add to it.
fn script_drop(ui: &egui::Ui, t: &Tokens, rows: &[(StableId, egui::Rect)]) -> Option<(StableId, String)> {
    let module = crate::project::dragged_script(ui.ctx())?;
    let pointer = ui.ctx().pointer_interact_pos()?;
    let full_width = |rect: egui::Rect| egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), rect.y_range());
    let (id, rect) = rows.iter().find(|(_, rect)| full_width(*rect).contains(pointer))?;
    ui.painter().rect_stroke(full_width(*rect), theme::RADIUS, egui::Stroke::new(2.0, t.focus), egui::StrokeKind::Inside);
    if !ui.input(|i| i.pointer.any_released()) {
        return None;
    }
    egui::DragAndDrop::clear_payload(ui.ctx());
    Some((*id, module))
}

/// While an object is dragged: shows where it would land, and on release returns the
/// `scene.object.reparent` params.
fn handle_drop(
    ui: &mut egui::Ui,
    t: &Tokens,
    scene: &Scene,
    rows: &[(StableId, egui::Rect)],
    root_zone: egui::Rect,
    scene_row: Option<egui::Rect>,
) -> Option<Value> {
    let dragged = egui::DragAndDrop::payload::<Dragged>(ui.ctx())?.0;
    let pointer = ui.ctx().pointer_interact_pos()?;
    let full_width = |rect: egui::Rect| egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), rect.y_range());
    let target = rows
        .iter()
        .find(|(_, rect)| full_width(*rect).contains(pointer))
        .map(|(id, rect)| {
            let fraction = (pointer.y - rect.top()) / rect.height().max(1.0);
            let drop = if fraction < EDGE_ZONE {
                Drop::Before(*id)
            } else if fraction > 1.0 - EDGE_ZONE {
                Drop::After(*id)
            } else {
                Drop::Into(*id)
            };
            (drop, *rect)
        })
        .or_else(|| scene_row.filter(|row| full_width(*row).contains(pointer)).map(|row| (Drop::Root, row)))
        .or_else(|| root_zone.contains(pointer).then_some((Drop::Root, root_zone)));
    let (drop, rect) = target?;
    let params = drop_params(scene, dragged, drop);

    // Text colour: it contrasts with both plain rows and the accent-filled selected row.
    let stroke = egui::Stroke::new(2.0, if params.is_some() { t.text } else { t.error });
    let painter = ui.painter();
    match drop {
        Drop::Before(_) => {
            painter.hline(rect.x_range(), rect.top(), stroke);
        }
        Drop::After(_) => {
            painter.hline(rect.x_range(), rect.bottom(), stroke);
        }
        Drop::Into(_) => {
            painter.rect_stroke(rect, theme::RADIUS, stroke, egui::StrokeKind::Inside);
        }
        Drop::Root => {
            painter.hline(rect.x_range(), rect.top() + 2.0, stroke);
        }
    }
    if params.is_none() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::NoDrop);
    }

    if ui.input(|i| i.pointer.any_released()) {
        egui::DragAndDrop::clear_payload(ui.ctx());
        return params;
    }
    None
}

/// `scene.object.reparent` params for dropping `dragged` at `drop`, or `None` when the drop would
/// change nothing or would put an object inside itself.
fn drop_params(scene: &Scene, dragged: StableId, drop: Drop) -> Option<Value> {
    let (parent, index) = match drop {
        Drop::Root => (None, None),
        Drop::Into(target) => (Some(target), None),
        Drop::Before(target) | Drop::After(target) => {
            let parent = scene.parent(target);
            let siblings = scene.children(parent);
            let mut index = siblings.iter().position(|s| *s == target)?;
            if matches!(drop, Drop::After(_)) {
                index += 1;
            }
            // The index is taken after `dragged` leaves its old place.
            if let Some(old) = siblings.iter().position(|s| *s == dragged)
                && old < index
            {
                index -= 1;
            }
            (parent, Some(index))
        }
    };
    if let Some(parent) = parent
        && (parent == dragged || is_descendant(scene, parent, dragged))
    {
        return None;
    }
    let same_place = scene.parent(dragged) == parent
        && match index {
            Some(index) => scene.children(parent).get(index) == Some(&dragged),
            None => scene.children(parent).last() == Some(&dragged),
        };
    if same_place {
        return None;
    }
    Some(json!({ "id": dragged, "parent": parent, "index": index }))
}

/// True when `candidate` is somewhere below `ancestor`.
fn is_descendant(scene: &Scene, candidate: StableId, ancestor: StableId) -> bool {
    let mut current = scene.parent(candidate);
    while let Some(parent) = current {
        if parent == ancestor {
            return true;
        }
        current = scene.parent(parent);
    }
    false
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
        (
            "Point Light",
            json!({"name": "Point Light", "components": {"pointLight": null, "transform": {"position": [0.0, 2.0, 0.0], "rotation": [0.0, 0.0, 0.0], "scale": [1.0, 1.0, 1.0]}}}),
        ),
        (
            "Spot Light",
            json!({"name": "Spot Light", "components": {"spotLight": null, "transform": {"position": [0.0, 4.0, 0.0], "rotation": [-90.0, 0.0, 0.0], "scale": [1.0, 1.0, 1.0]}}}),
        ),
    ]
}

/// A small Add button (empty object and ready-made objects) followed by the search field.
fn toolbar(editor: &mut Editor, ui: &mut egui::Ui) {
    let mut action: Option<Value> = None;
    ui.horizontal(|ui| {
        let add_size = egui::vec2(ROW_HEIGHT, ROW_HEIGHT);
        let (add, _) = egui::containers::menu::MenuButton::from_button(egui::Button::new(egui::RichText::new("+").strong()).min_size(add_size)).ui(ui, |ui| {
            ui.set_min_width(180.0);
            if ui.button(tl("Empty Object")).clicked() {
                action = Some(json!({ "name": "Object" }));
                ui.close();
            }
            ui.separator();
            for (label, params) in presets() {
                if ui.button(label).clicked() {
                    action = Some(params);
                    ui.close();
                }
            }
        });
        ui.ctx().accesskit_node_builder(add.id, |node| node.set_label("Add Object"));
        add.on_hover_text(tl("Add an object"));
        let search = ui.add_sized(
            [ui.available_width(), ROW_HEIGHT],
            egui::TextEdit::singleline(&mut editor.hierarchy_filter).hint_text(tl("Search")).align(egui::Align2::LEFT_CENTER),
        );
        // The hint is not an accessible name; give the field one.
        ui.ctx().accesskit_node_builder(search.id, |node| node.set_label("Search objects"));
    });
    if let Some(params) = action {
        editor.dispatch("scene.object.create", &params, None);
    }
}

#[cfg(test)]
mod tests {
    use super::{Drop, drop_params};
    use bayleaf_engine::scene::Scene;

    #[test]
    fn drops_are_planned_like_the_tree_shows_them() {
        let mut scene = Scene::new();
        let a = scene.create_object(None, "A").unwrap();
        let b = scene.create_object(None, "B").unwrap();
        let c = scene.create_object(None, "C").unwrap();
        // Into: b becomes a child of a.
        assert_eq!(drop_params(&scene, b, Drop::Into(a)).unwrap()["parent"], a.to_hex());
        // After c, moving a forward: a leaves index 0, so it lands at the end (index 2).
        assert_eq!(drop_params(&scene, a, Drop::After(c)).unwrap()["index"], 2);
        // Before a, moving c backward: index 0.
        assert_eq!(drop_params(&scene, c, Drop::Before(a)).unwrap()["index"], 0);
        // Dropping where it already is changes nothing.
        assert!(drop_params(&scene, b, Drop::After(a)).is_none());
    }

    #[test]
    fn an_object_cannot_be_dropped_into_itself_or_its_descendants() {
        let mut scene = Scene::new();
        let a = scene.create_object(None, "A").unwrap();
        let child = scene.create_object(Some(a), "Child").unwrap();
        assert!(drop_params(&scene, a, Drop::Into(a)).is_none());
        assert!(drop_params(&scene, a, Drop::Into(child)).is_none());
        assert!(drop_params(&scene, child, Drop::Root).is_some());
    }
}
