//! Editor tests driven through the accessibility tree, the way assistive technology sees it.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::Editor;

fn harness() -> Harness<'static, Editor> {
    // The size of a typical editor window, so panels have their real widths.
    let mut harness = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(|ui, editor: &mut Editor| editor.ui(ui, None), Editor::default());
    harness.run();
    harness
}

fn create_named(harness: &mut Harness<'static, Editor>, name: &str) {
    harness.state_mut().dispatch("scene.object.create", &json!({ "name": name }), None);
    harness.run();
}

fn add_empty_object(harness: &mut Harness<'static, Editor>) {
    harness.get_by_label("Add Object").click();
    harness.run();
    harness.get_by_label("Empty Object").click();
    harness.run();
}

#[test]
fn add_menu_creates_a_selected_object_at_the_root() {
    let mut harness = harness();
    add_empty_object(&mut harness);
    add_empty_object(&mut harness);
    let editor = harness.state();
    assert_eq!(editor.session.scene().children(None).len(), 2, "both objects are roots, whatever was selected");
    assert_eq!(editor.selected, editor.session.scene().children(None).last().copied());
}

#[test]
fn keyboard_moves_nest_and_unnest_without_dragging_and_undo_restores() {
    let mut harness = harness();
    create_named(&mut harness, "First");
    create_named(&mut harness, "Second");
    harness.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowRight);
    harness.run();
    assert_eq!(harness.state().session.scene().children(None).len(), 1, "Alt+Right makes it a child of the previous object");
    harness.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowLeft);
    harness.run();
    assert_eq!(harness.state().session.scene().children(None).len(), 2, "Alt+Left moves it out again");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run();
    assert_eq!(harness.state().session.scene().children(None).len(), 1);
}

#[test]
fn dragging_a_row_onto_another_makes_it_a_child() {
    let mut harness = harness();
    create_named(&mut harness, "Parent");
    create_named(&mut harness, "Mover");
    let from = harness.get_by_label("Mover").rect().center();
    let to = harness.get_by_label("Parent").rect().center();
    harness.hover_at(from);
    harness.drag_at(from);
    harness.run();
    for step in 1..=6 {
        harness.hover_at(from + (to - from) * (step as f32 / 6.0));
        harness.run();
    }
    harness.drop_at(to);
    harness.run();
    let scene = harness.state().session.scene();
    let roots = scene.children(None);
    assert_eq!(roots.len(), 1, "the dragged object left the root");
    assert_eq!(scene.children(roots.first().copied()).len(), 1);
}

#[test]
fn delete_key_removes_the_selected_object() {
    let mut harness = harness();
    add_empty_object(&mut harness);
    harness.key_press(egui::Key::Delete);
    harness.run();
    assert_eq!(harness.state().session.scene().object_count(), 0);
}

#[test]
fn inspector_shows_the_components_of_the_selection() {
    let mut harness = harness();
    add_empty_object(&mut harness);
    for label in ["Name", "Transform", "Add Component"] {
        assert!(harness.query_all_by_label(label).next().is_some(), "inspector has no {label:?}");
    }
}

#[test]
fn right_clicking_a_row_offers_moves_and_delete() {
    let mut harness = harness();
    create_named(&mut harness, "Target");
    harness.get_by_label("Target").click_secondary();
    harness.run();
    for label in ["Move Up", "Move Down", "Make Child of Previous", "Move Out of Parent", "Delete"] {
        assert!(harness.query_all_by_label(label).next().is_some(), "context menu has no {label:?}");
    }
    harness.get_by_label("Delete").click();
    harness.run();
    assert_eq!(harness.state().session.scene().object_count(), 0);
}

#[test]
fn clicking_a_row_selects_it_even_if_the_mouse_moves_a_little() {
    let mut harness = harness();
    create_named(&mut harness, "First");
    create_named(&mut harness, "Second");
    let first = harness.state().session.scene().children(None)[0];
    assert_ne!(harness.state().selected, Some(first));
    // A real click: press, a small hand movement, release.
    let at = harness.get_by_label("First").rect().center();
    harness.hover_at(at);
    harness.run();
    harness.drag_at(at);
    harness.run();
    harness.hover_at(at + egui::vec2(2.0, 1.0));
    harness.run();
    harness.drop_at(at + egui::vec2(2.0, 1.0));
    harness.run();
    assert_eq!(harness.state().selected, Some(first));
    assert_eq!(harness.state().session.scene().children(None).len(), 2, "a click must not move anything");
}

#[test]
fn play_and_stop_buttons_run_play_mode_and_restore_the_scene() {
    let mut harness = harness();
    create_named(&mut harness, "Actor");
    let before = harness.state().session.scene().to_doc();
    harness.get_by_label("Play scene").click();
    // Play mode repaints every frame, so step instead of waiting for the UI to settle.
    harness.run_steps(3);
    assert!(harness.state().session.is_playing());
    assert!(harness.query_by_label("Stop playing").is_some(), "the toolbar offers Stop while playing");
    // Edits during play are thrown away on stop.
    harness.state_mut().dispatch("scene.object.create", &json!({ "name": "Temporary" }), None);
    harness.run_steps(2);
    harness.get_by_label("Stop playing").click();
    harness.run();
    assert!(!harness.state().session.is_playing());
    assert_eq!(harness.state().session.scene().to_doc(), before);
    assert!(!harness.state().session.console().is_empty(), "start and stop are reported in the console");
}

#[test]
fn shortcuts_and_game_keys_work_while_a_non_text_widget_has_focus() {
    let mut harness = harness();
    create_named(&mut harness, "Actor");
    // Focus a button (keyboard navigation), not a text field.
    harness.get_by_label("Play scene").focus();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.run_steps(2);
    assert!(harness.state().session.is_playing(), "Ctrl+P starts play mode with a button focused");
    harness.key_down(egui::Key::D);
    harness.run_steps(2);
    assert!(harness.state().session.held_keys().contains("D"), "held keys reach the scripts");
}

fn name_of_first(harness: &Harness<'static, Editor>) -> String {
    let scene = harness.state().session.scene();
    scene.name(scene.children(None)[0]).unwrap_or_default().to_owned()
}

#[test]
fn rename_from_the_context_menu_edits_in_place_and_enter_keeps_the_new_name() {
    // Double-click opens the same field; kittest cannot simulate a double-click, so that path was
    // checked by hand in the editor.
    let mut harness = harness();
    create_named(&mut harness, "Old");
    harness.get_by_label("Old").click_secondary();
    harness.run();
    harness.get_by_label("Rename").click();
    harness.run();
    let field = harness.get_by_label("Object name");
    assert!(field.is_focused(), "the name field takes focus");
    // The whole name is selected, so typing replaces it.
    field.type_text("Hero");
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert_eq!(name_of_first(&harness), "Hero");
    assert!(harness.query_by_label("Object name").is_none(), "the field closes after Enter");
}

#[test]
fn f2_renames_the_selection_and_escape_keeps_the_old_name() {
    let mut harness = harness();
    create_named(&mut harness, "Keep");
    harness.key_press(egui::Key::F2);
    harness.run();
    harness.get_by_label("Object name").type_text("Discarded");
    harness.run();
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert_eq!(name_of_first(&harness), "Keep");
    harness.key_press(egui::Key::F2);
    harness.run();
    harness.get_by_label("Object name").type_text("Renamed");
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert_eq!(name_of_first(&harness), "Renamed");
}

#[test]
fn ctrl_and_mouse_wheel_over_the_files_resize_the_icons() {
    let mut harness = harness();
    let before = harness.state().project.icon();
    // A point in the file area: below the Project panel's toolbar, left of its search field.
    let search = harness.get_by_label("Search project files").rect();
    let over_files = egui::pos2(search.left() - 200.0, search.bottom() + 40.0);
    harness.hover_at(over_files);
    harness.run();
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, 3.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: egui::Modifiers::COMMAND,
    });
    harness.run_steps(20);
    assert!(harness.state().project.icon() > before, "Ctrl + wheel up makes the icons bigger");
}

#[test]
fn each_script_is_its_own_card_named_after_its_module() {
    let mut harness = harness();
    let params = json!({ "name": "Actor", "components": { "scripts": [{ "module": "scripts/missing_thing.wasm" }, { "module": "other.wasm" }] } });
    harness.state_mut().dispatch("scene.object.create", &params, None);
    harness.run();
    assert!(harness.query_by_label("Missing Thing").is_some(), "the card is titled after the module");
    assert!(harness.query_by_label("Other").is_some());
    assert!(harness.query_all_by_label_contains("Script file not found").next().is_some(), "a missing module says so");
    assert!(harness.query_by_label("Scripts").is_none(), "there is no wrapper card");
    harness.get_by_label("Remove Missing Thing").click();
    harness.run();
    let id = harness.state().selected.unwrap();
    let scripts = harness.state().session.scene().component(id, "scripts").unwrap();
    assert_eq!(scripts.as_array().map(Vec::len), Some(1));
    assert_eq!(scripts[0]["module"], "other.wasm");
}

#[test]
fn add_component_menu_lists_components_and_scripts() {
    let mut harness = harness();
    create_named(&mut harness, "Actor");
    harness.get_by_label("Add Component").click();
    harness.run();
    for label in ["Mesh", "Camera", "Scripts"] {
        assert!(harness.query_all_by_label(label).next().is_some(), "the menu has {label:?}");
    }
}

#[test]
fn agent_commands_are_confined_and_the_users_own_commands_are_not() {
    let base = std::env::temp_dir().join(format!("bayleaf-ui-agent-{}", std::process::id()));
    let (inside, outside) = (base.join("inside"), base.join("outside"));
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let confinement = bayleaf_engine::confine::Confinement::new(std::slice::from_ref(&inside), std::slice::from_ref(&inside)).unwrap();
    let mut harness = harness();
    let editor = harness.state_mut();
    let refused = editor.execute_automation("scene.saveAs", &json!({ "path": outside.join("a.bscene") }), &confinement).unwrap_err();
    assert!(refused.contains("outside"), "{refused}");
    assert!(editor.execute_automation("scene.saveAs", &json!({ "path": "agent.bscene" }), &confinement).is_ok(), "relative paths land in the root");
    assert!(inside.join("agent.bscene").exists());
    editor.dispatch("scene.saveAs", &json!({ "path": outside.join("mine.bscene") }), None);
    assert!(outside.join("mine.bscene").exists(), "the limit applies to the agent's request only");
    std::fs::remove_dir_all(&base).ok();
}

fn graph_harness(domain: &str) -> Harness<'static, Editor> {
    let mut harness = harness();
    harness.state_mut().dispatch("graph.new", &json!({ "domain": domain }), None);
    harness.run();
    harness
}

fn graph(harness: &Harness<'static, Editor>) -> bayleaf_engine::graph::Graph {
    harness.state().session.graph().unwrap().graph.clone()
}

#[test]
fn the_graph_panel_starts_empty_and_makes_a_new_material() {
    let mut harness = harness();
    harness.state_mut().show_panel(crate::Panel::Graph);
    harness.run();
    harness.get_by_label("New Material").click();
    harness.run();
    assert_eq!(graph(&harness).domain, "material");
    assert!(harness.query_by_label_contains("Material Output node").is_some());
    assert!(harness.query_by_label_contains("this material compiles").is_some(), "the problem strip says it is ready");
}

#[test]
fn the_add_node_menu_searches_and_adds_and_undo_removes() {
    let mut harness = graph_harness("material");
    harness.get_by_label("Add Node").click();
    harness.run();
    harness.get_by_label("Search nodes").type_text("noi");
    harness.run();
    assert!(harness.query_by_label("Checker").is_none(), "the search filters the list");
    harness.get_by_label("Noise").click();
    harness.run();
    assert_eq!(graph(&harness).nodes.len(), 2);
    // Enter in the search takes the first match.
    harness.get_by_label("Add Node").click();
    harness.run();
    harness.get_by_label("Search nodes").type_text("check");
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert!(graph(&harness).nodes.iter().any(|n| n.kind == "checker"));
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run();
    assert_eq!(graph(&harness).nodes.len(), 1);
}

#[test]
fn pins_link_from_the_keyboard_and_the_link_is_announced_on_both_pins() {
    let mut harness = graph_harness("material");
    harness.state_mut().dispatch("graph.node.add", &json!({ "kind": "color", "position": [0, 0] }), None);
    harness.run();
    harness.get_by_label_contains("Output Color (Color) of Color 2").click();
    harness.run();
    harness.get_by_label_contains("Input Base Color (Color) of Material Output 1").click();
    harness.run();
    assert_eq!(graph(&harness).links.len(), 1);
    assert!(harness.query_by_label_contains("Base Color (Color) of Material Output 1, linked from Color 2").is_some());
    // A number cannot feed a colour-only pin? It can (numbers widen), but a colour cannot feed a number.
    harness.get_by_label_contains("Output Color (Color) of Color 2").click();
    harness.run();
    harness.get_by_label_contains("Input Roughness (Number) of Material Output 1").click();
    harness.run();
    assert_eq!(graph(&harness).links.len(), 1, "the refused link is not made");
    assert!(harness.state().status.contains("Number"), "the status bar says why: {}", harness.state().status);
}

#[test]
fn arrow_keys_move_the_focused_node_and_delete_removes_it() {
    let mut harness = graph_harness("script");
    let before = graph(&harness).nodes[0].position;
    harness.get_by_label_contains("On Start node 1").focus();
    harness.run();
    harness.key_press(egui::Key::ArrowRight);
    harness.run();
    let after = graph(&harness).nodes[0].position;
    assert!(after[0] > before[0] && after[1] == before[1], "{before:?} -> {after:?}");
    harness.key_press(egui::Key::Delete);
    harness.run();
    assert!(graph(&harness).node(1).is_none());
}

#[test]
fn node_values_edit_through_commands_and_problems_name_their_node() {
    let mut harness = graph_harness("script");
    let editor = harness.state_mut();
    editor.dispatch("graph.node.add", &json!({ "kind": "log", "position": [300, 40] }), None);
    editor.dispatch("graph.node.add", &json!({ "kind": "position", "position": [0, 300] }), None);
    editor.dispatch("graph.link.add", &json!({ "from": { "node": 1, "pin": "then" }, "to": { "node": 3, "pin": "run" } }), None);
    editor.dispatch("graph.node.add", &json!({ "kind": "split", "position": [0, 300] }), None);
    harness.run();
    assert!(harness.query_by_label_contains("this visual script compiles").is_some());
    harness.get_by_label("Message of node 3").focus();
    harness.run();
    harness.get_by_label("Message of node 3").type_text(" there");
    harness.run();
    assert_eq!(graph(&harness).value(3, "message").and_then(|v| v.as_str()), Some("Hello there"));
    assert!(harness.state().session.history().len() <= 6, "typing merges into one undo step");
}

#[test]
fn buttons_at_the_end_of_inspector_rows_stay_inside_their_section() {
    let mut harness = harness();
    // The demo's visual script declares properties; overriding one shows Reset at the end of its row.
    let bob = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo/scripts/bob.bscript");
    let params = json!({ "name": "Bobber", "components": {
        "mesh": { "shape": "cube", "color": [1, 1, 1, 1], "metallic": 0, "roughness": 0.5, "material": "missing.bmaterial" },
        "scripts": [{ "module": bob.to_string_lossy().replace('\\', "/"), "properties": { "speed": 5.0 } }] } });
    harness.state_mut().dispatch("scene.object.create", &params, None);
    harness.run();
    let section_right = |harness: &Harness<'static, Editor>, title: &str| harness.get_by_label(&format!("Remove {title}")).rect().right();
    let reset = harness.get_by_label("Reset").rect();
    assert!(reset.right() <= section_right(&harness, "Bob") + 0.5, "Reset ends at {} past the section edge {}", reset.right(), section_right(&harness, "Bob"));
    let edit = harness.get_by_label("Edit material").rect();
    assert!(edit.right() <= section_right(&harness, "Mesh") + 0.5, "Edit ends at {} past the section edge", edit.right());
}

#[test]
fn copy_and_paste_go_through_the_clipboard_and_ctrl_d_duplicates() {
    let mut harness = harness();
    create_named(&mut harness, "Crate");
    harness.event(egui::Event::Copy);
    harness.run();
    let copied = harness.state().clipboard.clone().expect("the copy is on the clipboard");
    assert!(copied.contains("Crate"));
    harness.event(egui::Event::Paste(copied));
    harness.run();
    let editor = harness.state();
    assert_eq!(editor.session.scene().children(None).len(), 2);
    assert_ne!(editor.selected, editor.session.scene().children(None).first().copied(), "the pasted copy is selected");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
    harness.run();
    let editor = harness.state();
    assert_eq!(editor.session.scene().children(None).len(), 3);
    // The pasted copy keeps its name; its duplicate gets the next free number.
    assert_eq!(editor.selected.and_then(|id| editor.session.scene().name(id)), Some("Crate 2"), "the duplicate is selected and numbered");
    // Text that is not copied objects is refused with a reason.
    harness.event(egui::Event::Paste("hello".to_owned()));
    harness.run();
    assert_eq!(harness.state().session.scene().children(None).len(), 3);
    assert!(harness.state().status.contains("no objects"));
}

#[test]
fn closing_with_unsaved_changes_is_stopped_and_asks_first() {
    let mut harness = harness();
    create_named(&mut harness, "Unsaved");
    harness.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
    harness.run();
    assert!(harness.state().close_asked, "the close is held back");
    assert!(harness.query_by_label("Save changes before closing?").is_some());
    assert!(harness.query_by_label("Scene: Untitled scene").is_some(), "it says what is unsaved");
    harness.get_by_label("Cancel").click();
    harness.run();
    assert!(!harness.state().close_asked && !harness.state().close_confirmed);
}

#[test]
fn ctrl_and_shift_clicks_select_several_objects_and_delete_removes_them_as_one_step() {
    let mut harness = harness();
    for name in ["One", "Two", "Three", "Four"] {
        create_named(&mut harness, name);
    }
    harness.get_by_label("One").click();
    harness.run();
    harness.get_by_label("Three").click_modifiers(egui::Modifiers::COMMAND);
    harness.run();
    assert_eq!(harness.state().selection().len(), 2, "Ctrl adds");
    assert!(harness.query_by_label_contains("2 objects selected").is_some(), "the Inspector says what applies to all");
    harness.get_by_label("Four").click_modifiers(egui::Modifiers::SHIFT);
    harness.run();
    assert_eq!(harness.state().selection().len(), 2, "Shift selects the range from Three to Four");
    harness.get_by_label("One").click_modifiers(egui::Modifiers::SHIFT);
    harness.run();
    assert_eq!(harness.state().selection().len(), 4);
    harness.get_by_label("Two").click_modifiers(egui::Modifiers::COMMAND);
    harness.run();
    assert_eq!(harness.state().selection().len(), 3, "Ctrl on a selected object takes it out");

    let steps = harness.state().session.history().len();
    harness.key_press(egui::Key::Delete);
    harness.run();
    let editor = harness.state();
    assert_eq!(editor.session.scene().children(None).len(), 1, "only Two is left");
    assert_eq!(editor.session.history().len(), steps + 1, "one undo step");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run();
    assert_eq!(harness.state().session.scene().children(None).len(), 4);
}

#[test]
fn a_placed_prefab_is_marked_and_its_card_unlinks_it() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-prefab-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut harness = harness();
    let editor = harness.state_mut();
    editor.dispatch("scene.saveAs", &json!({ "path": dir.join("s.bscene") }), None);
    editor.dispatch("scene.object.create", &json!({ "name": "Crate" }), None);
    let id = editor.selected.unwrap();
    editor.dispatch("scene.prefab.create", &json!({ "id": id, "path": "crate.bprefab" }), None);
    harness.run();
    assert!(harness.query_by_label_contains("Crate prefab").is_some(), "the hierarchy row says it is a prefab");
    assert!(harness.query_by_label("Apply").is_some() && harness.query_by_label("Revert").is_some());
    assert!(harness.query_by_label_contains("Same as the prefab file").is_some());
    let editor = harness.state_mut();
    editor.dispatch("scene.component.add", &json!({ "id": id, "component": "mesh" }), None);
    harness.run();
    assert!(harness.query_by_label_contains("Own changes, kept when the file changes: Mesh").is_some(), "the card lists the copy's own changes");
    harness.get_by_label("Unlink").click();
    harness.run();
    assert!(harness.state().session.scene().component(id, "prefab").is_err());
    assert!(harness.query_by_label("Apply").is_none());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn meshes_pick_a_model_and_sprites_pick_an_image_from_the_project() {
    let mut harness = harness();
    harness.state_mut().dispatch("scene.object.create", &json!({ "name": "Thing", "components": { "mesh": null, "sprite": null } }), None);
    harness.run();
    let shows = |harness: &Harness<'static, Editor>, label: &str, value: &str| harness.query_all_by_label(label).any(|n| n.value().as_deref() == Some(value));
    assert!(shows(&harness, "Model", "None (the shape above)"), "the Mesh card offers a model");
    assert!(shows(&harness, "Image", "None (plain colour)"), "the Sprite card offers an image");
    let id = harness.state().selected.unwrap();
    harness.state_mut().dispatch(
        "scene.component.set",
        &json!({ "id": id, "component": "sprite", "value": { "color": [1, 1, 1, 1], "size": [1, 1], "image": "missing.png" } }),
        None,
    );
    harness.run();
    assert!(harness.query_all_by_label_contains("Warning: cannot read").next().is_some(), "a missing image says so");
}

/// An editor whose project folder is a fresh temporary folder.
fn project_harness(name: &str) -> (Harness<'static, Editor>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-new-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let mut harness = harness();
    harness.state_mut().dispatch("scene.saveAs", &json!({ "path": dir.join("s.bscene") }), None);
    harness.run();
    (harness, dir)
}

fn start_new(harness: &mut Harness<'static, Editor>, item: &str) {
    harness.get_by_label("Create file").click();
    harness.run();
    harness.get_by_label(item).click();
    harness.run();
    harness.run();
}

#[test]
fn a_new_file_is_named_first_and_written_only_on_enter() {
    let (mut harness, dir) = project_harness("enter");
    start_new(&mut harness, "New Material");
    let field = harness.get_by_label("Name of the new material");
    assert!(field.is_focused(), "the name field takes focus");
    assert!(!dir.join("New Material.bmaterial").exists(), "nothing is written before the name is confirmed");
    field.type_text("Rock");
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert!(dir.join("Rock.bmaterial").is_file());
    assert!(harness.query_by_label("Name of the new material").is_none());
    assert_eq!(harness.state().session.graph().and_then(|g| g.path.clone()).and_then(|p| p.file_name().map(|n| n.to_owned())), Some("Rock.bmaterial".into()));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn escape_cancels_a_new_file_and_bad_names_are_refused_with_a_reason() {
    let (mut harness, dir) = project_harness("escape");
    start_new(&mut harness, "New Visual Script");
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(harness.query_by_label("Name of the new visual script").is_none(), "Escape closes the field");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "only the scene is there: nothing was written");

    start_new(&mut harness, "New Visual Script");
    harness.get_by_label("Name of the new visual script").type_text("a/b");
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert!(harness.query_by_label_contains("none of /").is_some(), "the reason is shown");
    assert!(harness.query_by_label("Name of the new visual script").is_some(), "and the name can be fixed");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn clicking_a_file_shows_it_in_the_inspector_until_an_object_is_selected() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-file-card-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.md"), "Remember the lighthouse.\n").unwrap();
    let mut harness = harness();
    harness.state_mut().dispatch("scene.saveAs", &json!({ "path": dir.join("s.bscene") }), None);
    harness.run();
    create_named(&mut harness, "Keeper");
    assert!(harness.query_by_label("Transform").is_some(), "the object's components show first");

    harness.get_by_label("notes.md, File").click();
    harness.run();
    assert!(harness.state().selected.is_none(), "a clicked file takes the Inspector from the objects");
    assert!(harness.query_by_label_contains("Remember the lighthouse").is_some(), "the start of a text file is shown");
    assert!(harness.query_by_label("Transform").is_none());

    harness.get_by_label("Keeper").click();
    harness.run();
    assert!(harness.query_by_label("Transform").is_some(), "selecting an object shows it again");
    assert!(harness.query_by_label_contains("Remember the lighthouse").is_none());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn graph_nodes_copy_paste_and_duplicate_with_their_links_and_ctrl_click_selects_several() {
    let mut harness = graph_harness("script");
    let editor = harness.state_mut();
    let log = editor.dispatch("graph.node.add", &json!({ "kind": "log", "position": [300, 40] }), None).unwrap()["node"].as_u64().unwrap();
    editor.dispatch("graph.link.add", &json!({ "from": { "node": 1, "pin": "then" }, "to": { "node": log, "pin": "run" } }), None);
    harness.run();
    let count = graph(&harness).nodes.len();

    // Ctrl+D on a focused node adds a copy beside it.
    harness.get_by_label_contains(&format!("Log node {log}")).focus();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
    harness.run();
    assert_eq!(graph(&harness).nodes.len(), count + 1);
    assert_eq!(harness.state().session.scene().object_count(), 0, "Ctrl+D did not duplicate scene objects");

    // Ctrl + click adds the Log node to the selection; copying both keeps the link between them.
    harness.get_by_label_contains("On Start node 1").click();
    harness.run();
    harness.get_by_label_contains(&format!("Log node {log}")).click_modifiers(egui::Modifiers::COMMAND);
    harness.run();
    harness.get_by_label_contains("On Start node 1").focus();
    harness.run();
    harness.event(egui::Event::Copy);
    harness.run();
    let text = harness.state().clipboard.clone().unwrap();
    assert!(text.contains(bayleaf_engine::graph::FRAGMENT_FORMAT), "{text}");
    harness.event(egui::Event::Paste(text));
    harness.run();
    let pasted = graph(&harness);
    assert_eq!(pasted.nodes.len(), count + 3, "both copied nodes are added");
    assert_eq!(pasted.links.len(), 2, "with a link between the copies");
}

#[test]
fn ctrl_plus_and_minus_zoom_the_graph_and_not_the_whole_editor() {
    let mut harness = graph_harness("script");
    harness.get_by_label_contains("On Start node 1").focus();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Minus);
    harness.run();
    let out: f32 = harness.state().status.trim_start_matches("Graph zoom ").trim_end_matches('%').parse().unwrap();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Equals);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Equals);
    harness.run();
    let r#in: f32 = harness.state().status.trim_start_matches("Graph zoom ").trim_end_matches('%').parse().unwrap();
    assert!(r#in > out, "{out}% then {in}%");
    assert_eq!(harness.ctx.zoom_factor(), 1.0, "the editor's own scale is unchanged");
}

#[test]
fn a_comment_wraps_the_selected_nodes_takes_typed_text_and_moves_them_by_keyboard() {
    let mut harness = graph_harness("script");
    harness.get_by_label_contains("On Start node 1").click();
    harness.run();
    harness.get_by_label("Add Comment").click();
    harness.run();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.get_by_label("Comment text").type_text("Movement");
    harness.key_press(egui::Key::Enter);
    harness.run();
    let before = graph(&harness);
    assert_eq!(before.comments.len(), 1);
    assert_eq!(before.comments[0].text, "Movement");
    let (comment, start) = (&before.comments[0], before.node(1).unwrap());
    assert!(comment.position[0] < start.position[0] && comment.position[1] < start.position[1], "the box is around the node");

    harness.get_by_label("Comment: Movement").focus();
    harness.run();
    harness.key_press(egui::Key::ArrowRight);
    harness.run();
    let after = graph(&harness);
    assert!(after.comments[0].position[0] > comment.position[0], "the box moved");
    assert!(after.node(1).unwrap().position[0] > start.position[0], "and the node inside moved with it");
    assert!(after.node(2).unwrap().position == before.node(2).unwrap().position, "a node outside stayed");

    harness.key_press(egui::Key::Delete);
    harness.run();
    let removed = graph(&harness);
    assert!(removed.comments.is_empty());
    assert_eq!(removed.nodes.len(), before.nodes.len(), "removing a comment keeps its nodes");
}

#[test]
fn a_material_graph_shows_a_preview_that_can_be_hidden() {
    let mut harness = graph_harness("material");
    assert!(harness.query_by_label_contains("Material preview").is_some());
    harness.get_by_label("Hide material preview").click();
    harness.run();
    assert!(harness.query_by_label_contains("Material preview").is_none());
    harness.get_by_label("Show Preview").click();
    harness.run();
    assert!(harness.query_by_label_contains("Material preview").is_some());
}

#[test]
fn project_settings_turn_physics_off_through_the_command() {
    let mut harness = harness();
    harness.state_mut().show_panel(crate::Panel::ProjectSettings);
    harness.run();
    assert!(harness.query_by_label_contains("Open a project to change its settings").is_some(), "outside a project the defaults are shown");

    let dir = std::env::temp_dir().join(format!("bayleaf-ui-settings-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.run();
    harness.get_by_label("Physics in play mode").click();
    harness.run();
    let settings = harness.state_mut().dispatch("project.settings", &json!({}), None).unwrap();
    assert_eq!(settings["settings"]["physics"]["enabled"], false, "the click went through project.settings.set into the file");
    assert!(harness.query_by_label_contains("Gravity Y").is_some());
    harness.get_by_label("Physics in play mode").click();
    harness.run();
    harness.get_by_label("Double physics precision").click();
    harness.run();
    let settings = harness.state_mut().dispatch("project.settings", &json!({}), None).unwrap();
    assert_eq!(settings["settings"]["physics"]["precision"], "double");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn input_actions_save_when_they_read_right_and_explain_when_not() {
    let mut harness = harness();
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-actions-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.state_mut().show_panel(crate::Panel::ProjectSettings);
    harness.run();
    let jump = |harness: &mut Harness<'static, Editor>| {
        let settings = harness.state_mut().dispatch("project.settings", &json!({}), None).unwrap();
        settings["settings"]["input"]["actions"].as_array().unwrap().iter().find(|a| a["name"] == "Jump").unwrap()["bindings"].clone()
    };
    let retype = |harness: &mut Harness<'static, Editor>, text: &str| {
        harness.get_by_label("Bindings of Jump").focus();
        harness.run();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.get_by_label("Bindings of Jump").type_text(text);
        harness.run();
    };
    retype(&mut harness, "key:Space, key:J");
    assert!(jump(&mut harness).as_array().unwrap().iter().any(|b| b == "key:J"), "a good binding is saved");
    retype(&mut harness, "key:J, pad:Nope");
    assert!(harness.query_by_label_contains("Not saved").is_some(), "a bad one is explained");
    assert!(!jump(&mut harness).as_array().unwrap().iter().any(|b| b == "pad:Nope"), "and not saved");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn play_raises_the_game_view_which_shows_the_state_and_stop_returns_to_the_scene() {
    let mut harness = harness();
    create_named(&mut harness, "Actor");
    // Without a GPU neither view draws; each tab's own toolbar shows which is in front.
    assert!(harness.query_by_label("State").is_none() && harness.query_by_label("Grid").is_some(), "the Scene tab is in front before play");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.run_steps(3);
    assert!(harness.query_by_label("State").is_some(), "play brings the Game tab to the front");
    assert!(harness.query_all_by_label("Nothing set yet.").next().is_some(), "the state starts empty");
    harness.state_mut().dispatch("game.state.set", &json!({ "name": "coins", "value": 3 }), None);
    harness.run_steps(2);
    assert!(harness.query_by_label("coins 3, Game").is_some(), "the value shows with its name");
    harness.get_by_label("Stop playing").click();
    harness.run();
    assert!(harness.query_by_label("State").is_none() && harness.query_by_label("Grid").is_some(), "stop brings the Scene tab back");
}

/// The editor drawn offscreen with its real renderer, or `None` without a GPU (a failure instead
/// with `BAYLEAF_GPU_REQUIRED=1`).
fn gpu_harness() -> Option<Harness<'static, Editor>> {
    let setup = egui_kittest::wgpu::default_wgpu_setup();
    let instance = pollster::block_on(setup.new_instance());
    let config = egui_wgpu::WgpuConfiguration { wgpu_setup: setup, ..Default::default() };
    let state = match pollster::block_on(egui_wgpu::RenderState::create(&config, &instance, None, egui_wgpu::RendererOptions::default())) {
        Ok(state) => state,
        Err(error) if std::env::var_os("BAYLEAF_GPU_REQUIRED").is_none() => {
            eprintln!("no GPU, skipped: {error}");
            return None;
        }
        Err(error) => panic!("BAYLEAF_GPU_REQUIRED is set but there is no GPU: {error}"),
    };
    let renderer = egui_kittest::wgpu::WgpuTestRenderer::from_render_state(state.clone());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .renderer(renderer)
        .build_ui_state(move |ui, editor: &mut Editor| editor.ui(ui, Some(&state)), Editor::default());
    harness.run_steps(2);
    Some(harness)
}

/// The editor as drawn: width, height and RGBA bytes.
struct Picture {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn picture(harness: &mut Harness<'static, Editor>) -> Picture {
    let image = harness.render().unwrap();
    Picture { width: image.width(), height: image.height(), rgba: image.into_raw() }
}

/// How many different colours the picture has inside `rect` (points at one pixel per point).
fn colours_in(picture: &Picture, rect: egui::Rect) -> usize {
    let mut seen = std::collections::HashSet::new();
    for y in (rect.top().max(0.0) as u32..rect.bottom().min(picture.height as f32) as u32).step_by(3) {
        for x in (rect.left().max(0.0) as u32..rect.right().min(picture.width as f32) as u32).step_by(3) {
            let at = ((y * picture.width + x) * 4) as usize;
            seen.insert([picture.rgba[at], picture.rgba[at + 1], picture.rgba[at + 2]]);
        }
    }
    seen.len()
}

/// With `BAYLEAF_REVIEW_DIR` set, keeps a picture of the editor there for a look by eye.
fn keep_for_review(picture: &Picture, name: &str) {
    let Some(dir) = std::env::var_os("BAYLEAF_REVIEW_DIR") else { return };
    let path = std::path::Path::new(&dir).join(format!("{name}.png"));
    let file = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
    let mut encoder = png::Encoder::new(file, picture.width, picture.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&picture.rgba).unwrap();
}

#[test]
fn with_a_gpu_the_game_tab_draws_the_game_through_the_scene_camera() {
    let Some(mut harness) = gpu_harness() else { return };
    let dir = std::env::temp_dir().join(format!("bayleaf-game-tab-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.run_steps(3);
    let scene = picture(&mut harness);
    keep_for_review(&scene, "editor-scene");

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.run_steps(4);
    let view = harness.get_by_label("Game view, playing").rect();
    let playing = picture(&mut harness);
    keep_for_review(&playing, "editor-game");
    // The starter scene's sky, ground and light: a drawn game, not an empty canvas.
    assert!(colours_in(&playing, view) > 12, "the Game view shows the scene through its camera");
    std::fs::remove_dir_all(&dir).ok();
}
