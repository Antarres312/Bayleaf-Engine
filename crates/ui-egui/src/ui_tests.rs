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
    harness.query_all_by_label_contains("Rename").next().unwrap().click();
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
fn add_component_menu_lists_components_and_scripts_in_folders_by_their_menu_paths() {
    let mut harness = harness();
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-add-menu-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    // A visual script that puts itself under Light.
    let editor = harness.state_mut();
    editor.dispatch("graph.new", &json!({ "domain": "script" }), None);
    editor.dispatch("graph.menu.set", &json!({ "path": "Light/Flicker" }), None).unwrap();
    editor.dispatch("graph.save", &json!({ "path": dir.join("scripts").join("flicker.bscript") }), None);
    editor.project.rescan_soon();
    create_named(&mut harness, "Actor");
    harness.run_steps(2);
    harness.get_by_label("Add Component").click();
    harness.run();
    for folder in ["Rendering", "Light", "Physics", "Scripts"] {
        assert!(harness.query_by_label(folder).is_some(), "the menu has the folder {folder:?}");
    }
    assert!(harness.query_by_label("Point Light").is_none(), "components sit in their folders");
    harness.get_by_label("Light").click();
    harness.run();
    assert!(harness.query_by_label("Point Light").is_some(), "Light holds Point Light");
    assert!(harness.query_by_label("Flicker").is_some(), "and the script that names Light/Flicker");
    harness.get_by_label("Flicker").click();
    harness.run();
    let actor = harness.state().selected.unwrap();
    let scripts = harness.state().session.scene().component(actor, "scripts").unwrap();
    assert!(scripts.to_string().contains("flicker.bscript"), "choosing it adds the script: {scripts}");
    std::fs::remove_dir_all(&dir).ok();
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
    let reset = harness.get_by_label("Reset Speed").rect();
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
    harness.get_by_label("Create asset").click();
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

    harness.get_by_label("notes, File").click();
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
    // Fit All first: the duplicate moved the view, and the script panel may cover a node.
    harness.get_by_label("Fit All").click();
    harness.run();
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
fn input_map_edits_save_when_they_read_right_explain_when_not_and_stay_inside_the_panel() {
    let mut harness = harness();
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-actions-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.state_mut().show_panel(crate::Panel::ProjectSettings);
    harness.run_steps(2);
    // The project's Input Map opens from Project Settings into the Inspector.
    harness.get_by_label("Input").click();
    harness.run();
    harness.get_by_label("Edit the Input Map").click();
    harness.run_steps(2);
    let jump = |harness: &mut Harness<'static, Editor>| {
        let asset = harness.state_mut().dispatch("input.asset", &json!({ "path": "input/game.binput" }), None).unwrap();
        let player = asset["asset"]["maps"].as_array().unwrap().iter().find(|m| m["name"] == "Player").unwrap().clone();
        player["actions"].as_array().unwrap().iter().find(|a| a["name"] == "Jump").unwrap()["bindings"].clone()
    };
    let retype = |harness: &mut Harness<'static, Editor>, label: &str, text: &str| {
        harness.get_by_label(label).focus();
        harness.run();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.get_by_label(label).type_text(text);
        harness.run();
    };
    retype(&mut harness, "Bindings of Player/Jump", "key:Space, key:J");
    assert!(jump(&mut harness).as_array().unwrap().iter().any(|b| b == "key:J"), "a good binding is saved");
    retype(&mut harness, "Bindings of Player/Jump", "key:J, pad:Nope");
    assert!(harness.query_by_label_contains("Not saved").is_some(), "a bad one is explained");
    assert!(!jump(&mut harness).as_array().unwrap().iter().any(|b| b == "pad:Nope"), "and not saved");
    retype(&mut harness, "Bindings of Player/Jump", "key:J");
    retype(&mut harness, "Name of Player/Jump", "Jump/High");
    assert!(harness.query_by_label_contains("letters, digits or _").is_some(), "an action name with a / is explained: the map is the section it sits in");

    // Every action row stays inside the Inspector: the last Remove button ends where the first does.
    let removes: Vec<egui::Rect> = harness
        .query_all(egui_kittest::kittest::By::new().predicate(|n| n.label().is_some_and(|l| l.starts_with("Remove Player/") || l.starts_with("Remove Menu/"))))
        .map(|n| n.rect())
        .collect();
    assert!(removes.len() > 3);
    assert!(removes.iter().all(|r| (r.right() - removes[0].right()).abs() < 1.0), "rows do not drift: {removes:?}");
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

#[test]
fn a_narrow_inspector_keeps_every_control_inside_its_panel() {
    // A small window leaves the Inspector about 230 points wide.
    let mut harness = Harness::builder().with_size(egui::vec2(980.0, 760.0)).build_ui_state(|ui, editor: &mut Editor| editor.ui(ui, None), Editor::default());
    harness.run();
    let dir = std::env::temp_dir().join(format!("bayleaf-narrow-inspector-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    let ground = {
        let scene = harness.state().session.scene();
        scene.world_matrices().into_iter().map(|(id, _)| id).find(|id| scene.name(*id) == Some("Ground")).unwrap()
    };
    harness.state_mut().select_only(Some(ground));
    harness.run();
    // The name field spans the panel: nothing below it may reach past its right edge.
    let name = harness.get_by_label("Name").rect();
    let field = harness.query_all(egui_kittest::kittest::By::new().predicate(|n| n.label().as_deref() == Some("Name"))).last().unwrap().rect();
    let edge = field.right().max(name.right());
    let overflowing: Vec<String> = harness
        .query_all(egui_kittest::kittest::By::new().predicate(|_| true))
        .filter(|n| egui_kittest::kittest::NodeT::accesskit_node(n).bounding_box().is_some())
        .filter(|n| {
            let r = n.rect();
            r.left() >= name.left() - 1.0 && r.top() > field.bottom() && r.width() > 0.0 && r.right() > edge + 1.0
        })
        .map(|n| format!("{:?} {:?}", egui_kittest::kittest::NodeT::accesskit_node(&n).label(), n.rect()))
        .collect();
    assert!(overflowing.is_empty(), "inspector controls past the panel's edge ({edge}): {overflowing:#?}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_folder_menu_creates_a_named_material_and_opens_it() {
    let mut harness = harness();
    let dir = std::env::temp_dir().join(format!("bayleaf-create-menu-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.state_mut().project.current = dir.join("materials");
    harness.run_steps(2);
    harness.get_by_label("Folder contents").click_secondary();
    harness.run();
    harness.get_by_label("Create").click();
    harness.run();
    harness.get_by_label("New Material").click();
    harness.run_steps(2);
    let field = harness.get_by_label("Name of the new material");
    field.focus();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.get_by_label("Name of the new material").type_text("Rock");
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    let made = dir.join("materials").join("Rock.bmaterial");
    assert!(made.is_file(), "the material is written once it has a name");
    let open = harness.state().session.graph().and_then(|g| g.path.clone());
    assert_eq!(open.map(|p| p.file_name().map(|n| n.to_owned())), Some(made.file_name().map(|n| n.to_owned())), "and opens in the graph editor");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn inspector_lists_sit_beside_their_labels_not_over_them() {
    let mut harness = harness();
    let dir = std::env::temp_dir().join(format!("bayleaf-inspector-labels-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    let cube = {
        let scene = harness.state().session.scene();
        scene.world_matrices().into_iter().map(|(id, _)| id).find(|id| scene.name(*id) == Some("Cube")).unwrap()
    };
    harness.state_mut().select_only(Some(cube));
    harness.run();
    for label in ["Shape", "Material", "Model"] {
        let at = harness
            .query_all(egui_kittest::kittest::By::new().predicate(move |n| n.role() == egui::accesskit::Role::Label && n.value().as_deref() == Some(label)))
            .next()
            .unwrap_or_else(|| panic!("no {label} label"))
            .rect();
        let row = |r: egui::Rect| r.top() < at.bottom() && r.bottom() > at.top();
        let lists: Vec<egui::Rect> = harness
            .query_all(egui_kittest::kittest::By::new().predicate(|n| n.role() == egui::accesskit::Role::ComboBox))
            .map(|n| n.rect())
            .filter(|r| row(*r))
            .collect();
        assert!(!lists.is_empty(), "{label} has a list on its row");
        assert!(lists.iter().all(|r| r.left() >= at.right()), "{label}'s list starts after the label: {at:?} {lists:?}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn delete_asks_first_escape_keeps_the_file_and_confirming_removes_it() {
    let mut harness = harness();
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-delete-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.state_mut().project.current = dir.join("materials");
    harness.state_mut().project.delete_permanently = true;
    harness.run_steps(2);
    let file = dir.join("materials").join("ground.bmaterial");
    let tile = "ground, Material";

    harness.get_by_label(tile).click();
    harness.run();
    harness.key_press(egui::Key::Delete);
    harness.run_steps(2);
    assert!(harness.query_by_label("Move to Recycle Bin").is_some(), "Delete asks first");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert!(harness.query_by_label("Move to Recycle Bin").is_none() && file.exists(), "Escape keeps the file");

    harness.get_by_label(tile).click_secondary();
    harness.run();
    harness.get_by_label("Delete…").click();
    harness.run_steps(2);
    assert!(harness.get_by_label("Move to Recycle Bin").is_focused(), "the confirm button has focus, so Enter deletes");
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert!(!file.exists(), "confirming deletes it");
    assert!(harness.query_by_label(tile).is_none(), "and it leaves the panel");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_markdown_file_shows_formatted_in_the_inspector_and_as_written_with_source() {
    let mut harness = harness();
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-markdown-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.run_steps(2);
    harness.get_by_label("README, File").click();
    harness.run_steps(2);
    assert!(harness.query_all_by_value("•").next().is_some(), "list items get bullets");
    assert!(harness.query_all_by_label_contains("- `scenes/`").next().is_none(), "no Markdown marks show");
    harness.get_by_label("Source").click();
    harness.run_steps(2);
    assert!(harness.query_all_by_label_contains("- `scenes/`").next().is_some(), "Source shows the text as written");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_mouse_wheel_zooms_the_graph_around_the_pointer() {
    let mut harness = graph_harness("material");
    harness.state_mut().show_panel(crate::Panel::Graph);
    harness.run_steps(2);
    let canvas = harness.get_by_label("Graph canvas").rect();
    let before = harness.state().graph_view.scene_rect;
    harness.hover_at(canvas.center());
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, 3.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
        source: egui::MouseWheelSource::Wheel,
    });
    harness.run_steps(8);
    let after = harness.state().graph_view.scene_rect;
    assert!(after.width() < before.width() - 1.0, "scrolling up zooms in: {before:?} -> {after:?}");
}

#[test]
fn a_rust_script_is_one_entry_that_drags_onto_an_object_as_its_module() {
    let mut harness = harness();
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-one-script-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.state_mut().dispatch("script.new", &json!({ "folder": "scripts", "name": "Mover" }), None);
    // As if it was built: the module beside its folder.
    std::fs::write(dir.join("scripts").join("mover.wasm"), b"\0asm\x01\0\0\0").unwrap();
    harness.state_mut().project.current = dir.join("scripts");
    harness.state_mut().project.rescan_soon();
    harness.run_steps(2);
    assert!(harness.query_by_label("mover, Rust Script").is_some(), "the script shows once");
    assert!(harness.query_by_label("mover, Script").is_none(), "its module is not listed again");

    let from = harness.get_by_label("mover, Rust Script").rect().center();
    let to = harness.get_by_label("Cube").rect().center();
    harness.hover_at(from);
    harness.drag_at(from);
    harness.run();
    for step in 1..=6 {
        harness.hover_at(from + (to - from) * (step as f32 / 6.0));
        harness.run();
    }
    harness.drop_at(to);
    harness.run_steps(2);
    let scene = harness.state().session.scene();
    let cube = scene.world_matrices().into_iter().map(|(id, _)| id).find(|id| scene.name(*id) == Some("Cube")).unwrap();
    let scripts = scene.component(cube, "scripts").unwrap();
    assert!(scripts.to_string().contains("scripts/mover.wasm"), "the object runs the built module: {scripts}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn open_recent_lists_projects_newest_first_and_opens_one_again() {
    let mut harness = harness();
    let base = std::env::temp_dir().join(format!("bayleaf-ui-recent-{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    for name in ["Forest", "Castle"] {
        harness.state_mut().dispatch("project.new", &json!({ "path": base.join(name) }), None);
        harness.run();
    }
    harness.get_by_label("File").click();
    harness.run();
    harness.get_by_label("Open Recent").click();
    harness.run();
    let forest = harness.get_by_label("Forest").rect();
    let castle = harness.get_by_label("Castle").rect();
    assert!(castle.top() < forest.top(), "the newest project comes first");
    harness.get_by_label("Forest").click();
    harness.run_steps(2);
    assert_eq!(harness.state().session.project_dir(), Some(base.join("Forest").as_path()), "choosing it opens it again");

    // The list is kept between runs, including a project whose folder is gone since.
    let saved = harness.state().recent_projects_text();
    std::fs::remove_dir_all(base.join("Castle")).ok();
    let mut next = Editor::default();
    next.restore_recent_projects(&saved);
    assert_eq!(next.recent_projects_text(), saved);
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn project_settings_list_their_categories_on_the_left_physics_first() {
    let mut harness = harness();
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-settings-categories-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.state_mut().show_panel(crate::Panel::ProjectSettings);
    harness.run_steps(2);
    let item = |harness: &Harness<'static, Editor>, name: &'static str| {
        harness
            .query_all(egui_kittest::kittest::By::new().predicate(move |n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some(name)))
            .next()
            .unwrap()
            .rect()
    };
    let (physics, input) = (item(&harness, "Physics"), item(&harness, "Input"));
    assert!(physics.top() < input.top(), "Physics comes first");
    assert!(physics.right() < harness.get_by_label("Physics in play mode").rect().left(), "the list is on the left of the settings");
    assert!(harness.query_by_label("Edit the Input Map").is_none(), "only the chosen category shows");
    harness.get_by_label("Input").click();
    harness.run();
    assert!(harness.query_by_label("Edit the Input Map").is_some() && harness.query_by_label("Physics in play mode").is_none(), "Input shows its own settings");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn f2_renames_a_file_in_its_own_tile_and_a_click_on_empty_space_clears_the_selection() {
    let mut harness = harness();
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-rename-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.state_mut().project.current = dir.join("materials");
    harness.run_steps(2);
    let tile = harness.get_by_label("ground, Material").rect();
    harness.get_by_label("ground, Material").click();
    harness.run();
    harness.key_press(egui::Key::F2);
    harness.run_steps(2);
    let field = harness.get_by_label("New name of ground.bmaterial");
    assert!(field.is_focused() && tile.contains_rect(field.rect().shrink(1.0)), "the name is typed in the file's own tile");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert!(dir.join("materials/ground.bmaterial").is_file(), "Escape keeps the old name");

    harness.get_by_label("ground, Material").click_secondary();
    harness.run();
    harness.query_all_by_label_contains("Rename").next().unwrap().click();
    harness.run_steps(2);
    // The name is selected without its extension, so typing replaces just the name.
    harness.get_by_label("New name of ground.bmaterial").type_text("Grass");
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert!(dir.join("materials/Grass.bmaterial").is_file() && !dir.join("materials/ground.bmaterial").exists());

    harness.get_by_label("Grass, Material").click();
    harness.run();
    assert!(harness.state().project.selected.is_some());
    // The middle of the files area is empty: the few tiles sit at its top left.
    harness.get_by_label("Folder contents").click();
    harness.run();
    assert!(harness.state().project.selected.is_none(), "a click on empty space clears the selection");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn hovering_a_pin_its_label_or_its_value_shows_the_type_it_carries() {
    let mut harness = graph_harness("script");
    harness.state_mut().show_panel(crate::Panel::Graph);
    let add = harness.state_mut().dispatch("graph.node.add", &json!({ "kind": "add", "position": [200, 40] }), None).unwrap()["node"].as_u64().unwrap();
    harness.run_steps(2);
    let pin = harness.get_by_label_contains(&format!("Input A (Number or Vector) of Add {add}")).rect();
    let value = harness.get_by_label(&format!("A of Add {add}")).rect();
    for at in [pin.center(), pin.center() + egui::vec2(24.0, 0.0), value.center()] {
        harness.hover_at(at);
        harness.run_steps(30);
        assert!(harness.query_all_by_label_contains("A: Number or Vector").next().is_some(), "hovering at {at:?} names the type");
        harness.hover_at(egui::pos2(1.0, 1.0));
        harness.run_steps(5);
    }
}

#[test]
fn the_script_panel_declares_variables_and_their_nodes_pick_one_from_a_list() {
    let mut harness = graph_harness("script");
    harness.get_by_label("Add Variable").click();
    harness.run();
    harness.get_by_label("Number").click();
    harness.run();
    assert!(graph(&harness).variable("number").is_some(), "the menu declares a number variable");
    harness.get_by_label("Set number").click();
    harness.run();
    let set =
        graph(&harness).nodes.iter().find(|n| n.kind == "setVariable").map(|n| n.id).unwrap_or_else(|| panic!("Set places a node: {}", harness.state().status));
    assert_eq!(graph(&harness).value(set, "name").and_then(|v| v.as_str()), Some("number"));
    // Renaming the variable in the panel renames it on its nodes.
    harness.get_by_label("Name of variable number").focus();
    harness.run();
    harness.get_by_label("Name of variable number").type_text("s");
    harness.run();
    harness.key_press(egui::Key::Tab);
    harness.run();
    assert!(graph(&harness).variable("numbers").is_some());
    assert_eq!(graph(&harness).value(set, "name").and_then(|v| v.as_str()), Some("numbers"), "the node follows the rename");
    // The node's name is a list of the script's variables, with a way to declare another.
    harness.get_by_label(&format!("Name of node {set}: numbers")).click();
    harness.run();
    harness.get_by_label("New Variable\u{2026}").click();
    harness.run();
    let fresh = graph(&harness).value(set, "name").and_then(|v| v.as_str()).map(str::to_owned).expect("a name");
    assert_ne!(fresh, "numbers");
    assert!(graph(&harness).variable(&fresh).is_some(), "the new name is declared");
    harness.get_by_label(&format!("Name of node {set}: {fresh}")).click();
    harness.run();
    harness.get_by_label("numbers").click();
    harness.run();
    assert_eq!(graph(&harness).value(set, "name").and_then(|v| v.as_str()), Some("numbers"), "choosing from the list sets it");
}

/// Drags the Project panel entry `from` onto the widget labelled `to`, in small steps.
fn drag_entry(harness: &mut Harness<'static, Editor>, from: &str, to: &str) {
    let start = harness.get_by_label(from).rect().center();
    let end = harness.get_by_label(to).rect().center();
    harness.hover_at(start);
    harness.drag_at(start);
    harness.run();
    for step in 1..=6 {
        harness.hover_at(start + (end - start) * (step as f32 / 6.0));
        harness.run();
    }
    harness.drop_at(end);
    harness.run();
}

#[test]
fn a_file_field_takes_a_dragged_file_of_its_kind_and_refuses_others() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-drop-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.md"), "Not a material.\n").unwrap();
    std::fs::write(dir.join("Rock.bmaterial"), "{}").unwrap();
    let mut harness = harness();
    harness.state_mut().dispatch("scene.saveAs", &json!({ "path": dir.join("s.bscene") }), None);
    harness.run();
    harness.state_mut().dispatch(
        "scene.object.create",
        &json!({ "name": "Boulder", "components": { "mesh": { "shape": "cube", "color": [1, 1, 1, 1], "metallic": 0, "roughness": 0.5 } } }),
        None,
    );
    harness.run();
    harness.get_by_label("Boulder").click();
    harness.run();
    let material = |harness: &Harness<'static, Editor>| {
        let scene = harness.state().session.scene();
        let id = harness.state().selected.unwrap();
        scene.get::<bayleaf_engine::scene::components::Mesh>(id).unwrap().material.clone()
    };
    drag_entry(&mut harness, "notes, File", "Material");
    assert_eq!(material(&harness), "", "a file of another kind is refused");
    drag_entry(&mut harness, "Rock, Material", "Material");
    assert_eq!(material(&harness), "Rock.bmaterial", "a material dropped on the field is chosen");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn dragging_a_script_property_from_its_default_keeps_going_and_reset_turns_on_and_off() {
    let mut harness = harness();
    let bob = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo/scripts/bob.bscript");
    let params = json!({ "name": "Bobber", "components": {
        "scripts": [{ "module": bob.to_string_lossy().replace('\\', "/") }] } });
    harness.state_mut().dispatch("scene.object.create", &params, None);
    harness.run();
    let height = |harness: &Harness<'static, Editor>| {
        let id = harness.state().selected.unwrap();
        let scripts = harness.state().session.scene().get::<bayleaf_engine::scene::components::Scripts>(id).unwrap().clone();
        scripts.0[0].properties.get("height").and_then(|v| v.number())
    };
    assert!(egui_kittest::kittest::NodeT::accesskit_node(&harness.get_by_label("Reset Height")).is_disabled(), "at its default, reset is off");
    let field = harness.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Height").rect().center();
    harness.hover_at(field);
    harness.drag_at(field);
    harness.run();
    let mut seen = Vec::new();
    for step in 1..=6 {
        harness.hover_at(field + egui::vec2(8.0 * step as f32, 0.0));
        harness.run();
        seen.push(height(&harness));
    }
    harness.drop_at(field + egui::vec2(48.0, 0.0));
    harness.run();
    let first = seen.iter().position(Option::is_some).expect("the drag changes the value");
    assert!(seen[first] < seen[5], "the drag keeps changing it after it leaves its default: {seen:?}");
    assert!(!egui_kittest::kittest::NodeT::accesskit_node(&harness.get_by_label("Reset Height")).is_disabled(), "away from its default, reset is on");
    harness.get_by_label("Reset Height").click();
    harness.run();
    assert_eq!(height(&harness), None, "reset goes back to the default");
}

#[test]
fn the_add_node_menu_opens_categories_on_a_click_and_keeps_its_size_while_searching() {
    let mut harness = graph_harness("script");
    harness.get_by_label("Add Node").click();
    harness.run();
    harness.run();
    assert!(harness.query_by_label("Find Object With").is_none(), "categories start closed");
    let before = harness.get_by_label("Search nodes").rect();
    harness.get_by_label("Find").click();
    harness.run();
    assert!(harness.query_by_label("Find Object With").is_some(), "a click opens the category");
    harness.get_by_label("Search nodes").focus();
    harness.run();
    harness.get_by_label("Search nodes").type_text("velo");
    harness.run();
    harness.run();
    assert!(harness.query_by_label("Set Velocity").is_some(), "the search shows matches from closed categories too");
    assert!(harness.query_by_label("Find Object With").is_none(), "and only matches");
    let after = harness.get_by_label("Search nodes").rect();
    assert_eq!(before.width(), after.width(), "the popup keeps its width while the list narrows");
}

#[test]
fn the_project_panel_hides_file_extensions_until_the_preference_is_turned_off() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-extensions-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let mut harness = harness();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    std::fs::write(dir.join("notes.md"), "A note.\n").unwrap();
    harness.state_mut().project.rescan_soon();
    harness.run_steps(2);
    assert!(harness.query_by_label("notes, File").is_some(), "extensions are hidden by default");
    harness.state_mut().show_panel(crate::Panel::ProjectSettings);
    harness.run();
    harness.get_by_label("Preferences").click();
    harness.run();
    harness.get_by_label("Hide file extensions").click();
    harness.run_steps(2);
    let settings = harness.state_mut().dispatch("project.settings", &json!({}), None).unwrap();
    assert_eq!(settings["settings"]["editor"]["hideExtensions"], false, "the preference is kept in the project file");
    assert!(harness.query_by_label("notes.md, File").is_some(), "names show their extensions again");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_menu_path_is_typed_in_the_script_panel_and_kept_while_it_does_not_read_right_yet() {
    let mut harness = graph_harness("script");
    harness.get_by_label("Menu path in Add Component").focus();
    harness.run();
    harness.get_by_label("Menu path in Add Component").type_text("Light/");
    harness.run();
    assert_eq!(graph(&harness).menu, "", "a trailing / is not a path yet");
    assert!(harness.query_by_label_contains("Not set:").is_some(), "the reason shows under the field");
    harness.get_by_label("Menu path in Add Component").type_text("Flicker");
    harness.run();
    assert_eq!(graph(&harness).menu, "Light/Flicker", "what is typed is kept and set once it reads right");
    assert!(harness.query_by_label_contains("Not set:").is_none());
}

#[test]
fn a_search_node_picks_a_component_from_a_list_or_takes_a_dragged_script() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-search-kind-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let mut harness = harness();
    let editor = harness.state_mut();
    editor.dispatch("project.new", &json!({ "path": dir }), None);
    editor.dispatch("graph.new", &json!({ "domain": "script" }), None);
    editor.dispatch("graph.save", &json!({ "path": dir.join("seeker.bscript") }), None);
    let node = editor.dispatch("graph.node.add", &json!({ "kind": "findWith", "position": [40, 220] }), None).unwrap()["node"].as_u64().unwrap() as u32;
    editor.project.rescan_soon();
    harness.run_steps(2);
    let kind = |harness: &Harness<'static, Editor>| graph(harness).value(node, "kind").and_then(|v| v.as_str()).map(str::to_owned);
    harness.get_by_label_contains(&format!("Component, Tag or Script of node {node}:")).click();
    harness.run();
    harness.get_by_label("Mesh").click();
    harness.run();
    assert_eq!(kind(&harness).as_deref(), Some("Mesh"), "a component chosen from the list");
    drag_entry(&mut harness, "seeker, Visual Script", &format!("Component, Tag or Script of node {node}: Mesh"));
    assert_eq!(kind(&harness).as_deref(), Some("Seeker"), "a dragged script is looked for by its name");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn get_component_shows_the_chosen_components_fields_as_pins_and_drops_links_to_fields_it_loses() {
    let mut harness = graph_harness("script");
    let editor = harness.state_mut();
    let get = editor.dispatch("graph.node.add", &json!({ "kind": "getComponent", "position": [40, 260] }), None).unwrap()["node"].as_u64().unwrap() as u32;
    let log = editor.dispatch("graph.node.add", &json!({ "kind": "log", "position": [320, 260] }), None).unwrap()["node"].as_u64().unwrap() as u32;
    editor.dispatch("graph.link.add", &json!({ "from": { "node": get, "pin": "intensity" }, "to": { "node": log, "pin": "value" } }), None).unwrap();
    harness.run();
    assert!(harness.query_by_label_contains(&format!("Output Intensity (Number) of Get Component {get}")).is_some(), "a Point Light's fields at first");
    harness.get_by_label_contains(&format!("Component of node {get}:")).click();
    harness.run();
    harness.get_by_label("Camera").click();
    harness.run();
    assert!(harness.query_by_label_contains(&format!("Output Field of View (Number) of Get Component {get}")).is_some(), "the Camera's fields now");
    assert!(harness.query_by_label_contains(&format!("Output Intensity (Number) of Get Component {get}")).is_none());
    assert!(graph(&harness).links.is_empty(), "the link to a field the node lost is gone");
}

#[test]
fn a_script_is_chosen_as_a_component_from_the_list_or_by_dragging_and_its_properties_become_pins() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-script-component-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let mut harness = harness();
    let editor = harness.state_mut();
    editor.dispatch("project.new", &json!({ "path": dir }), None);
    editor.dispatch("graph.new", &json!({ "domain": "script" }), None);
    let property = editor.dispatch("graph.node.add", &json!({ "kind": "property", "position": [40, 420] }), None).unwrap()["node"].clone();
    editor.dispatch("graph.node.set", &json!({ "node": property, "name": "name", "value": "speed" }), None);
    editor.dispatch("graph.save", &json!({ "path": dir.join("bob.bscript") }), None);
    let read = editor.dispatch("graph.node.add", &json!({ "kind": "tryGetComponent", "position": [300, 20] }), None).unwrap()["node"].as_u64().unwrap() as u32;
    let get = editor.dispatch("graph.node.add", &json!({ "kind": "getComponent", "position": [300, 300] }), None).unwrap()["node"].as_u64().unwrap() as u32;
    editor.project.rescan_soon();
    harness.run_steps(2);

    harness.get_by_label_contains(&format!("Component of node {read}:")).click();
    harness.run();
    // Scripts follow the components in the list, below its fold.
    harness.get_by_label("Bob").scroll_to_me();
    harness.run();
    harness.get_by_label("Bob").click();
    harness.run();
    assert_eq!(graph(&harness).value(read, "component").and_then(|v| v.as_str()), Some("Bob"));
    assert_eq!(graph(&harness).value(read, "fields"), Some(&json!(["speed"])), "the node keeps the script's properties");
    assert!(harness.query_by_label_contains(&format!("Output Speed (Number) of Try Get Component {read}")).is_some(), "they are its pins");

    drag_entry(&mut harness, "bob, Visual Script", &format!("Component of node {get}: Point Light"));
    assert_eq!(graph(&harness).value(get, "component").and_then(|v| v.as_str()), Some("Bob"), "a dragged script is chosen too");
    assert!(harness.query_by_label_contains(&format!("Output Speed (Number) of Get Component {get}")).is_some());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn nodes_naming_a_script_follow_its_properties_when_the_script_changes() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-script-refresh-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let mut harness = harness();
    let editor = harness.state_mut();
    editor.dispatch("project.new", &json!({ "path": dir }), None);
    // Bob declares speed.
    editor.dispatch("graph.new", &json!({ "domain": "script" }), None);
    let property = editor.dispatch("graph.node.add", &json!({ "kind": "property", "position": [40, 420] }), None).unwrap()["node"].clone();
    editor.dispatch("graph.node.set", &json!({ "node": property, "name": "name", "value": "speed" }), None);
    editor.dispatch("graph.save", &json!({ "path": dir.join("bob.bscript") }), None);
    // Another script reads Bob's speed.
    editor.dispatch("graph.new", &json!({ "domain": "script" }), None);
    let read = editor.dispatch("graph.node.add", &json!({ "kind": "tryGetComponent", "position": [300, 20] }), None).unwrap()["node"].as_u64().unwrap() as u32;
    editor.dispatch("graph.node.set", &json!({ "node": read, "name": "component", "value": "Bob" }), None).unwrap();
    editor.dispatch("graph.node.set", &json!({ "node": read, "name": "fields", "value": ["speed"] }), None).unwrap();
    let log = editor.dispatch("graph.node.add", &json!({ "kind": "log", "position": [600, 20] }), None).unwrap()["node"].as_u64().unwrap() as u32;
    editor.dispatch("graph.link.add", &json!({ "from": { "node": read, "pin": "speed" }, "to": { "node": log, "pin": "value" } }), None).unwrap();
    editor.project.rescan_soon();
    harness.run_steps(2);
    assert_eq!(graph(&harness).value(read, "fields"), Some(&json!(["speed"])), "nothing changes while the script does not");

    // Bob's speed becomes pace, and it gains height.
    let file = dir.join("bob.bscript");
    let mut bob: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    for node in bob["nodes"].as_array_mut().unwrap() {
        if node["kind"] == "property" {
            node["values"]["name"] = json!("pace");
        }
    }
    bob["nodes"].as_array_mut().unwrap().push(json!({ "id": 99, "kind": "property", "position": [0, 0], "values": { "name": "height" } }));
    std::fs::write(&file, bob.to_string()).unwrap();
    harness.run_steps(2);

    let fields = graph(&harness).value(read, "fields").cloned().unwrap();
    let names: Vec<&str> = fields.as_array().unwrap().iter().filter_map(serde_json::Value::as_str).collect();
    assert!(names.contains(&"pace") && names.contains(&"height") && !names.contains(&"speed"), "the node follows the script: {names:?}");
    assert!(harness.query_by_label_contains(&format!("Output Height (Number) of Try Get Component {read}")).is_some(), "its pins too");
    assert!(graph(&harness).links.is_empty(), "the link from the property the script dropped is gone");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_unlinked_text_input_is_typed_in_the_node() {
    let mut harness = graph_harness("script");
    let join =
        harness.state_mut().dispatch("graph.node.add", &json!({ "kind": "joinText", "position": [300, 260] }), None).unwrap()["node"].as_u64().unwrap() as u32;
    harness.run();
    let field = format!("A of Join Text {join}");
    harness.get_by_label(&field).focus();
    harness.run();
    harness.get_by_label(&field).type_text("Score: ");
    harness.run();
    assert_eq!(graph(&harness).value(join, "a").and_then(|v| v.as_str()), Some("Score: "));
    assert!(harness.query_by_label_contains(&format!("Output Result (Text) of Join Text {join}")).is_some(), "the output says it carries text");
}

#[test]
fn a_tab_maximises_its_panel_and_restores_the_layout() {
    // Frames of 1/60 s, so two clicks a few frames apart make a double-click.
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(60)
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui, None), Editor::default());
    harness.run();
    assert!(harness.query_by_label("Hierarchy").is_some());
    harness.get_by_label("Inspector").click_secondary();
    harness.run();
    harness.get_by_label("Maximise").click();
    harness.run_steps(2);
    assert_eq!(harness.state().maximized, Some(crate::Panel::Inspector));
    assert!(harness.query_by_label("Hierarchy").is_none(), "only the maximised panel shows");
    assert!(harness.query_by_label("Inspector").is_some());
    // A double-click on the tab restores the layout.
    let tab = harness.get_by_label("Inspector").rect().center();
    harness.hover_at(tab);
    // Half a second, so the menu click before does not make this a triple-click.
    harness.run_steps(30);
    for pressed in [true, false, true, false] {
        harness.event(egui::Event::PointerButton { pos: tab, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        harness.step();
    }
    harness.run();
    harness.run_steps(2);
    assert_eq!(harness.state().maximized, None);
    assert!(harness.query_by_label("Hierarchy").is_some(), "the layout is back as it was");
}

#[test]
fn dragging_an_object_onto_a_folder_makes_a_prefab_there() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-drag-prefab-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let mut harness = harness();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    std::fs::create_dir_all(dir.join("things")).unwrap();
    harness.state_mut().project.rescan_soon();
    harness.run_steps(2);
    create_named(&mut harness, "Crate");
    drag_entry(&mut harness, "Crate", "things, Folder");
    assert!(dir.join("things").join("Crate.bprefab").is_file(), "the object became a prefab in the folder");
    let crate_id = harness.state().selected.unwrap();
    assert!(harness.state().session.scene().get::<bayleaf_engine::scene::components::Prefab>(crate_id).is_some(), "and is linked to it");
    // The panel now shows the folder; its free space takes the next one. Linked to a prefab, the
    // object's row says so.
    drag_entry(&mut harness, "Crate prefab", "Folder contents");
    assert!(dir.join("things").join("Crate 2.bprefab").is_file(), "a second one does not overwrite the first");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn text_and_object_variables_are_added_in_the_panel_with_their_own_start_values() {
    let mut harness = graph_harness("script");
    harness.get_by_label("Add Variable").click();
    harness.run();
    harness.get_by_label("Text").click();
    harness.run();
    let declared = |harness: &Harness<'static, Editor>, name: &str| graph(harness).variable(name).map(|v| (v.ty, v.start.clone()));
    assert_eq!(declared(&harness, "text"), Some((bayleaf_engine::graph::VariableType::Text, json!(""))));
    harness.get_by_label("Start of text").focus();
    harness.run();
    harness.get_by_label("Start of text").type_text("Ready");
    harness.run();
    assert_eq!(declared(&harness, "text").map(|(_, start)| start), Some(json!("Ready")), "the start is typed in the panel");
    harness.get_by_label("Set text").click();
    harness.run();
    assert!(graph(&harness).nodes.iter().any(|n| n.kind == "setTextVariable"), "Set places a Set Text Variable node");

    harness.get_by_label("Add Variable").click();
    harness.run();
    // "Object" is also the menu bar's Object menu: take the entry in the Add Variable menu below it.
    harness.query_all_by_label("Object").find(|n| n.rect().top() > 60.0).expect("the Object entry").click();
    harness.run();
    assert_eq!(declared(&harness, "object"), Some((bayleaf_engine::graph::VariableType::Object, serde_json::Value::Null)), "an object starts as none");
}

#[test]
fn whole_number_variables_and_pins_say_what_they_carry() {
    let mut harness = graph_harness("script");
    let whole = harness.state_mut().dispatch("graph.node.add", &json!({ "kind": "wholeNumber", "position": [300, 260] }), None).unwrap()["node"]
        .as_u64()
        .unwrap() as u32;
    harness.run();
    assert!(harness.query_by_label_contains(&format!("Output Result (Whole Number) of Whole Number {whole}")).is_some());
    harness.get_by_label("Add Variable").click();
    harness.run();
    harness.query_all_by_label("Whole Number").find(|n| n.rect().top() > 60.0 && n.rect().width() < 400.0).expect("the menu entry").click();
    harness.run();
    let declared = graph(&harness).variable("whole").map(|v| (v.ty, v.start.clone()));
    assert_eq!(declared, Some((bayleaf_engine::graph::VariableType::WholeNumber, json!(0))));
}

#[test]
fn a_scene_file_is_dragged_onto_a_load_scene_node() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-scene-node-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let mut harness = harness();
    harness.state_mut().dispatch("scene.saveAs", &json!({ "path": dir.join("level.bscene") }), None);
    harness.state_mut().dispatch("graph.new", &json!({ "domain": "script" }), None);
    let load =
        harness.state_mut().dispatch("graph.node.add", &json!({ "kind": "loadScene", "position": [320, 260] }), None).unwrap()["node"].as_u64().unwrap() as u32;
    harness.run_steps(2);
    drag_entry(&mut harness, "level, Scene", &format!("Scene File of node {load}"));
    assert_eq!(graph(&harness).value(load, "scene").and_then(|v| v.as_str()), Some("level.bscene"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn split_into_streamed_areas_asks_with_a_form_that_takes_the_keyboard() {
    let mut harness = harness();
    let base = std::env::temp_dir().join(format!("bayleaf-ui-split-{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": base.join("World") }), None);
    harness.state_mut().dispatch("scene.object.create", &json!({ "name": "Tree" }), None);
    harness.run();
    // The menu bar's Object, above the panels' own.
    harness.query_all_by_label("Object").min_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).unwrap().click();
    harness.run();
    harness.query_all_by_label_contains("Split into Streamed Areas…").next().unwrap().click();
    harness.run();
    let size = harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Cell Size");
    assert!(size.is_focused(), "the first field takes focus");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Cell Size").type_text("wide");
    harness.run();
    harness.get_by_role_and_label(egui::accesskit::Role::Button, "Split into Streamed Areas").click();
    harness.run();
    assert!(harness.query_by_label_contains("Cell Size must be a number").is_some(), "the form says what is wrong and stays");
    assert!(harness.state().session.scene().children(None).iter().all(|id| harness.state().session.scene().name(*id) != Some("Streamed World")));

    harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Cell Size").click();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Cell Size").type_text("50");
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert!(
        harness.query_by_role_and_label(egui::accesskit::Role::TextInput, "Cell Size").is_none(),
        "the form closes once it ran: {}",
        harness.state().status
    );
    let scene = harness.state().session.scene();
    assert!(scene.children(None).iter().any(|id| scene.name(*id) == Some("Streamed World")), "the split happened");
    assert!(base.join("World/cells").is_dir());
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn the_pseudo_language_marks_menus_tabs_cards_and_command_labels() {
    // Text the pseudo language marks went through translation; plain English did not. The marks
    // (U+27E6, U+27E7) need a math face that only a craft-fonts build embeds.
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .allow_missing_glyphs()
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui, None), Editor::default());
    harness.state_mut().set_language("pseudo");
    harness.state_mut().dispatch("scene.object.create", &json!({ "name": "Box" }), None);
    harness.run();
    assert!(harness.query_by_label("⟦File⟧").is_some(), "top menus");
    assert!(harness.query_by_label_contains(": done⟧").is_some(), "messages built while running: {}", harness.state().status);
    assert!(harness.query_by_label("⟦Hierarchy⟧").is_some(), "panel tabs");
    harness.get_by_label("Box").click();
    harness.run();
    assert!(harness.query_by_label("⟦Transform⟧").is_some(), "component cards");
    harness.get_by_label("⟦Object⟧").click();
    harness.run();
    harness.query_all_by_label_contains("⟦Split into Streamed Areas…⟧").next().expect("command labels in menus").click();
    harness.run();
    assert!(harness.query_by_role_and_label(egui::accesskit::Role::TextInput, "⟦Cell Size⟧").is_some(), "form fields");
    assert!(crate::i18n::looked_up().contains("Inspector"));

    // Back in English, and an unknown language is English too.
    harness.key_press(egui::Key::Escape);
    harness.state_mut().set_language("xx-unknown");
    harness.run();
    assert!(harness.query_by_label("File").is_some() && harness.query_by_label("⟦File⟧").is_none());
}

/// Drags with the left button from `from` to `to` in a few steps.
fn left_drag(harness: &mut Harness<'static, Editor>, from: egui::Pos2, to: egui::Pos2) {
    harness.hover_at(from);
    harness.drag_at(from);
    harness.run();
    for step in 1..=6 {
        harness.hover_at(from + (to - from) * (step as f32 / 6.0));
        harness.run();
    }
    harness.drop_at(to);
    harness.run();
}

#[test]
fn a_dragged_node_keeps_up_with_the_pointer_when_zoomed_and_delete_removes_it() {
    let mut harness = graph_harness("script");
    let log = harness.state_mut().dispatch("graph.node.add", &json!({ "kind": "log", "position": [300, 40] }), None).unwrap()["node"].as_u64().unwrap();
    harness.run();
    harness.get_by_label("Fit All").click();
    harness.run();
    // Zoomed in, a canvas unit is bigger than a screen pixel.
    harness.get_by_label_contains("On Start node 1").focus();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Equals);
    harness.run();
    let name = format!("Log node {log}");
    let before = harness.get_by_label_contains(&name).rect();
    let from = before.left_center() + egui::vec2(12.0, 0.0);
    left_drag(&mut harness, from, from + egui::vec2(60.0, 30.0));
    let after = harness.get_by_label_contains(&name).rect();
    let moved = after.min - before.min;
    assert!((moved - egui::vec2(60.0, 30.0)).length() < 1.5, "the node moved as far as the pointer: {moved:?}");

    // The dragged node has the keyboard: Delete removes it, not a scene object.
    harness.key_press(egui::Key::Delete);
    harness.run();
    assert!(graph(&harness).node(log as u32).is_none(), "{}", harness.state().status);
}

#[test]
fn the_left_button_on_the_background_selects_a_box_and_the_right_one_pans_even_over_a_node() {
    let mut harness = graph_harness("script");
    harness.state_mut().dispatch("graph.node.add", &json!({ "kind": "log", "position": [300, 40] }), None);
    harness.run();
    harness.get_by_label("Fit All").click();
    harness.run();

    // The right button held over a node moves the view, not the node.
    let positions = |h: &Harness<'static, Editor>| graph(h).nodes.iter().map(|n| n.position).collect::<Vec<_>>();
    let before = positions(&harness);
    let title = harness.get_by_label_contains("On Start node 1").rect();
    let from = title.left_center() + egui::vec2(12.0, 0.0);
    let to = from + egui::vec2(-50.0, 40.0);
    harness.hover_at(from);
    harness.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Secondary, pressed: true, modifiers: egui::Modifiers::NONE });
    harness.run();
    for step in 1..=6 {
        harness.hover_at(from + (to - from) * (step as f32 / 6.0));
        harness.run();
    }
    harness.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Secondary, pressed: false, modifiers: egui::Modifiers::NONE });
    harness.run();
    assert_eq!(positions(&harness), before, "no node moved");
    let panned = harness.get_by_label_contains("On Start node 1").rect().min - title.min;
    assert!((panned - (to - from)).length() < 1.5, "the canvas followed the pointer: {panned:?}");

    // A box over the whole canvas selects every node; Delete then removes them.
    harness.get_by_label("Fit All").click();
    harness.run();
    let canvas = harness.get_by_label("Graph canvas").rect();
    left_drag(&mut harness, canvas.left_top() + egui::vec2(4.0, 4.0), canvas.right_bottom() - egui::vec2(4.0, 4.0));
    harness.key_press(egui::Key::Delete);
    harness.run();
    assert!(graph(&harness).nodes.is_empty(), "every node was in the box: {}", harness.state().status);
}

#[test]
fn a_whole_number_variable_does_not_widen_the_script_panel() {
    let mut harness = graph_harness("script");
    let canvas = harness.get_by_label("Graph canvas").rect();
    harness.get_by_label("Add Variable").click();
    harness.run();
    harness.get_by_label("Whole Number").click();
    harness.run_steps(10);
    assert!(graph(&harness).variable("whole").is_some());
    assert_eq!(harness.get_by_label("Graph canvas").rect().left(), canvas.left(), "the panel beside the canvas kept its width");
}

#[test]
fn a_typed_property_is_added_in_the_panel_and_edited_with_its_own_field_in_the_inspector() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-typed-property-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let mut harness = harness();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.state_mut().dispatch("graph.new", &json!({ "domain": "script" }), None);
    harness.run();
    harness.get_by_label("Add Property").click();
    harness.run();
    harness.get_by_label("Text").click();
    harness.run();
    let placed = graph(&harness).nodes.iter().find(|n| n.kind == "textProperty").map(|n| n.id);
    assert!(placed.is_some(), "Add Property > Text places a Text Property node");
    harness.state_mut().dispatch("graph.save", &json!({ "path": dir.join("sign.bscript") }), None);

    // An object with the script shows the property as a text field.
    let editor = harness.state_mut();
    let id = editor.dispatch("scene.object.create", &json!({ "name": "Sign" }), None).unwrap()["id"].as_str().map(str::to_owned).unwrap();
    let id = bayleaf_core::StableId::parse_hex(&id).unwrap();
    editor.dispatch("scene.script.add", &json!({ "id": id, "module": "sign.bscript" }), None);
    editor.select_only(Some(id));
    harness.run();
    let field = harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Label");
    field.click();
    harness.run();
    harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Label").type_text("Shop");
    harness.run();
    let scripts = harness.state().session.scene().get::<bayleaf_engine::scene::components::Scripts>(id).cloned().unwrap();
    assert_eq!(scripts.0[0].properties.get("label").and_then(|v| v.text()), Some("Shop"), "the typed text is the object's value");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn functions_are_added_given_inputs_and_called_from_the_script_panel() {
    let mut harness = graph_harness("script");
    harness.get_by_label("Add Function").click();
    harness.run();
    let g = graph(&harness);
    assert_eq!(g.functions.len(), 1, "Add Function declares one");
    let name = g.functions[0].name.clone();
    let start = g.nodes.iter().find(|n| n.kind == "functionStart").map(|n| n.id).expect("and places its Function node");
    assert!(g.nodes.iter().any(|n| n.kind == "functionReturn"), "and its Return node");

    harness.get_by_label("Add Input").click();
    harness.run();
    harness.get_by_label("Whole Number").click();
    harness.run();
    let g = graph(&harness);
    assert_eq!(g.function(&name).unwrap().inputs.len(), 1);
    assert_eq!(g.value(start, "inputs"), Some(&json!(["input:whole"])), "the Function node follows");
    harness.get_by_label("Fit All").click();
    harness.run();
    assert!(harness.query_by_label_contains(&format!("Output Input (Whole Number) of Function {start}")).is_some(), "as a pin");

    harness.get_by_label(&format!("Call {name}")).click();
    harness.run();
    let call = graph(&harness).nodes.iter().find(|n| n.kind == "callFunction").map(|n| n.id).expect("Call places a Call Function node");
    assert_eq!(graph(&harness).value(call, "inputs"), Some(&json!(["input:whole"])));
    harness.get_by_label(&format!("Remove input from {name}")).click();
    harness.run();
    assert_eq!(graph(&harness).value(call, "inputs"), Some(&json!([])), "removing the input takes the pin off every node");
    assert!(harness.query_by_label_contains("this visual script compiles").is_some(), "{}", harness.state().status);
}
