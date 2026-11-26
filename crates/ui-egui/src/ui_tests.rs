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

fn right_click_at(harness: &mut Harness<'static, Editor>, pos: egui::Pos2) {
    harness.hover_at(pos);
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
    }
    harness.run();
}

#[test]
fn right_clicking_the_empty_hierarchy_adds_a_top_object_named_in_place_and_one_undo_removes_it() {
    let mut harness = harness();
    create_named(&mut harness, "Parent");
    let parent = harness.state().selected;
    let below = harness.get_by_label("Parent").rect().center() + egui::vec2(0.0, 120.0);
    right_click_at(&mut harness, below);
    let steps = harness.state().session.history().len();
    harness.get_by_label("Cube").click();
    harness.run();
    let editor = harness.state();
    let scene = editor.session.scene();
    assert_eq!(scene.children(None).len(), 2, "the new object is at the top, not inside the selected one");
    assert_ne!(editor.selected, parent, "the new object is selected");
    assert_eq!(editor.selected, scene.children(None).last().copied());
    assert_eq!(editor.session.history().len(), steps + 1, "one undo step");
    assert!(harness.get_by_label("Object name").is_focused(), "its name field takes focus");
    // Escape keeps the ready-made name and leaves the field, so the undo shortcut works again.
    harness.key_press(egui::Key::Escape);
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run();
    assert_eq!(harness.state().session.scene().object_count(), 1, "one undo removes it");
}

#[test]
fn an_objects_add_menu_makes_a_child_of_it_and_names_it_in_place() {
    let mut harness = harness();
    create_named(&mut harness, "Parent");
    harness.get_by_label("Parent").click_secondary();
    harness.run();
    harness.get_by_label("Add").click();
    harness.run();
    harness.get_by_label("Empty Object").click();
    harness.run();
    assert!(harness.query_by_label("Move Up").is_none(), "the menu closes");
    let field = harness.get_by_label("Object name");
    assert!(field.is_focused(), "the name field takes focus");
    field.type_text("Lid");
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    let editor = harness.state();
    let scene = editor.session.scene();
    let parent = scene.children(None)[0];
    let children = scene.children(Some(parent));
    assert_eq!(children.len(), 1, "the new object is a child of the clicked one");
    assert_eq!(editor.selected, Some(children[0]));
    assert_eq!(scene.name(children[0]), Some("Lid"));
}

#[test]
fn shift_f10_and_the_menu_key_open_the_hierarchy_menus_by_keyboard() {
    let mut harness = harness();
    create_named(&mut harness, "Target");
    harness.get_by_label("Target").focus();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::F10);
    harness.run();
    assert!(harness.query_by_label("Move Up").is_some(), "Shift+F10 opens the focused row's menu");
    assert!(harness.get_by_label("Rename").is_focused(), "its first entry takes focus");
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(harness.query_by_label("Move Up").is_none(), "Escape closes it");
    assert!(harness.get_by_label("Target").is_focused(), "and gives focus back to the row");

    // The scene's row stands for the top level: its menu adds a top object.
    harness.get_by_label_contains("Untitled").focus();
    harness.run();
    harness.key_press(egui::Key::Menu);
    harness.run();
    assert!(harness.get_by_label("Empty Object").is_focused(), "the menu key opens the Add menu with focus on its first entry");
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert_eq!(harness.state().session.scene().children(None).len(), 2);
    assert!(harness.get_by_label("Object name").is_focused(), "the new object is named in place");
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
    // Removed from the card's menu.
    harness.get_by_label("More for Missing Thing").click();
    harness.run();
    harness.get_by_label("Remove").click();
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
    {
        let open = harness.state().session.graph().map_or(0, |g| g.id);
        harness.state_mut().show_panel(crate::Panel::Graph(open));
    }
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
fn a_renamed_node_is_found_by_its_former_label_which_is_not_shown() {
    let mut harness = graph_harness("script");
    harness.get_by_label("Add Node").click();
    harness.run();
    harness.get_by_label("Search nodes").type_text("raycast");
    harness.run();
    assert!(harness.query_by_label("Raycast").is_none(), "the former label is not shown");
    harness.get_by_label("Cast Ray").click();
    harness.run();
    assert!(graph(&harness).nodes.iter().any(|n| n.kind == "raycast"));
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
    // The card's menu button sits at the right end of its title row.
    let section_right = |harness: &Harness<'static, Editor>, title: &str| harness.get_by_label(&format!("More for {title}")).rect().right();
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

/// The GPU tests take turns: two D3D12 devices drawing at once from two threads crash inside the
/// driver (an access violation, not a test failure), which Vulkan and the editor's one device do
/// not do. A test holds its turn until it ends.
static GPU_TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The editor drawn offscreen with its real renderer, or `None` without a GPU (a failure instead
/// with `BAYLEAF_GPU_REQUIRED=1`), with this test's turn on the GPU.
fn gpu_harness() -> Option<(std::sync::MutexGuard<'static, ()>, Harness<'static, Editor>)> {
    // A test that failed during its turn still lets the others have theirs.
    let turn = GPU_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
    Some((turn, harness))
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

/// A material graph whose surface is `color`, or one that does not compile (no Material Output).
fn material_text(color: Option<[f32; 3]>) -> String {
    let nodes = match color {
        Some(color) => json!([{ "id": 1, "kind": "output", "position": [0.0, 0.0], "values": { "baseColor": color } }]),
        None => json!([]),
    };
    json!({ "format": "bayleaf.graph", "version": 1, "domain": "material", "nodes": nodes, "links": [] }).to_string()
}

#[test]
fn a_material_that_does_not_compile_shows_a_warning_mark_and_why_in_the_inspector() {
    let (mut harness, dir) = project_harness("thumbs");
    std::fs::write(dir.join("Good.bmaterial"), material_text(Some([0.9, 0.1, 0.1]))).unwrap();
    std::fs::write(dir.join("Broken.bmaterial"), material_text(None)).unwrap();
    harness.state_mut().project.rescan_soon();
    harness.run_steps(3);
    assert!(harness.query_by_label("Broken, Material, does not compile").is_some(), "the broken material's tile says so");
    assert!(harness.query_by_label("Good, Material").is_some(), "a material that compiles has no warning");

    harness.get_by_label("Broken, Material, does not compile").click();
    harness.run_steps(3);
    assert!(harness.query_by_label("Preview of Broken.bmaterial: Does not compile").is_some(), "the Inspector says why there is no picture");
    harness.get_by_label("Good, Material").click();
    harness.run_steps(3);
    assert!(
        harness.query_by_label("Preview of Good.bmaterial: Drawing previews needs a graphics adapter").is_some(),
        "without a GPU it says so instead of showing an empty picture"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn with_a_gpu_a_material_tile_shows_the_material_and_follows_its_file() {
    let Some((_turn, mut harness)) = gpu_harness() else { return };
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-gpu-thumbs-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    harness.state_mut().dispatch("scene.saveAs", &json!({ "path": dir.join("s.bscene") }), None);
    let file = dir.join("Paint.bmaterial");
    std::fs::write(&file, material_text(Some([0.9, 0.05, 0.05]))).unwrap();
    harness.state_mut().project.rescan_soon();
    harness.run_steps(4);
    let tile = harness.get_by_label("Paint, Material").rect();
    // The tile's picture area, above its name.
    let area = egui::Rect::from_min_size(tile.min, egui::vec2(tile.width(), tile.height() * 0.6)).shrink(4.0);
    let mean = |picture: &Picture| {
        let (mut sum, mut count) = ([0u64; 3], 0u64);
        for y in area.top() as u32..area.bottom() as u32 {
            for x in area.left() as u32..area.right() as u32 {
                let at = ((y * picture.width + x) * 4) as usize;
                for (c, s) in sum.iter_mut().enumerate() {
                    *s += u64::from(picture.rgba[at + c]);
                }
                count += 1;
            }
        }
        sum.map(|s| s / count.max(1))
    };
    let red = picture(&mut harness);
    keep_for_review(&red, "material_tile_red");
    let before = mean(&red);
    assert!(before[0] > before[2] + 20, "the tile shows the red material: {before:?}");
    assert!(colours_in(&red, area) > 20, "shaded like a sphere, not a flat icon");

    // Changed on disk, the picture is drawn again.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(&file, material_text(Some([0.05, 0.1, 0.9]))).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    harness.run_steps(4);
    let blue = picture(&mut harness);
    let after = mean(&blue);
    assert!(after[2] > after[0] + 20, "the tile follows the file: before {before:?}, after {after:?}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn with_a_gpu_the_game_tab_draws_the_game_through_the_scene_camera() {
    let Some((_turn, mut harness)) = gpu_harness() else { return };
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
    {
        let open = harness.state().session.graph().map_or(0, |g| g.id);
        harness.state_mut().show_panel(crate::Panel::Graph(open));
    }
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
fn graphics_settings_change_the_shadow_maps_the_views_draw() {
    let mut harness = harness();
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-settings-graphics-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.state_mut().show_panel(crate::Panel::ProjectSettings);
    harness.run_steps(2);
    harness.get_by_label("Graphics").click();
    harness.run();
    assert!(harness.query_by_label("Shadow Lights").is_some() && harness.query_by_label("Cascades").is_some(), "Graphics shows the shadow settings");
    harness.get_by_label("Resolution").click();
    harness.run();
    harness.get_by_label("4096 × 4096").click();
    harness.run();
    let settings = harness.state_mut().dispatch("project.settings", &json!({}), None).unwrap();
    assert_eq!(settings["settings"]["graphics"]["shadowResolution"], 4096, "the choice went through project.settings.set into the file");
    assert_eq!(crate::project_settings::current(harness.state_mut()).graphics.shadows().resolution, 4096, "the views draw with it at once");
    std::fs::remove_dir_all(&dir).ok();
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
    {
        let open = harness.state().session.graph().map_or(0, |g| g.id);
        harness.state_mut().show_panel(crate::Panel::Graph(open));
    }
    // Beside On Start and On Update, not on them, and in view.
    let add = harness.state_mut().dispatch("graph.node.add", &json!({ "kind": "add", "position": [320, 120] }), None).unwrap()["node"].as_u64().unwrap();
    harness.run_steps(2);
    harness.get_by_label("Fit All").click();
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
    std::fs::write(dir.join("Rock.bmaterial"), material_text(Some([0.5, 0.45, 0.4]))).unwrap();
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
fn an_object_dragged_from_the_hierarchy_onto_a_script_field_fills_it_and_keeps_the_inspector() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-object-field-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let script = json!({ "format": "bayleaf.graph", "version": 1, "domain": "script", "nodes": [
        { "id": 1, "kind": "objectProperty", "position": [0.0, 0.0], "values": { "name": "target" } }] });
    let module = dir.join("follow.bscript");
    std::fs::write(&module, serde_json::to_string_pretty(&script).unwrap()).unwrap();
    let mut harness = harness();
    create_named(&mut harness, "Goal");
    let params = json!({ "name": "Follower", "components": { "scripts": [{ "module": module.to_string_lossy().replace('\\', "/") }] } });
    harness.state_mut().dispatch("scene.object.create", &params, None);
    harness.run();
    let follower = harness.state().selected.expect("the new object is selected");
    let target = |harness: &Harness<'static, Editor>| {
        let scripts = harness.state().session.scene().get::<bayleaf_engine::scene::components::Scripts>(follower).unwrap().clone();
        scripts.0[0].properties.get("target").and_then(|v| v.text()).map(str::to_owned)
    };
    let from = harness.get_by_label("Goal").rect().center();
    let to = harness.get_by_label("Target").rect().center();
    harness.hover_at(from);
    harness.drag_at(from);
    harness.run();
    for step in 1..=6 {
        harness.hover_at(from + (to - from) * (step as f32 / 6.0));
        harness.run();
        assert_eq!(harness.state().selected, Some(follower), "the drag leaves the Inspector on the script's object");
    }
    harness.drop_at(to);
    harness.run();
    assert_eq!(target(&harness).as_deref(), Some("Goal"), "the dropped object fills the field");
    assert_eq!(harness.state().selected, Some(follower));
    assert_eq!(harness.state().session.scene().children(None).len(), 2, "nothing moved in the hierarchy");
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
fn dragging_a_number_field_to_the_window_edge_wraps_the_pointer_and_keeps_going() {
    let mut harness = harness();
    add_empty_object(&mut harness);
    let x = |harness: &Harness<'static, Editor>| {
        let id = harness.state().selected.unwrap();
        harness.state().session.scene().get::<bayleaf_engine::scene::components::Transform>(id).unwrap().position[0]
    };
    let jumps = |harness: &Harness<'static, Editor>| -> Vec<egui::Pos2> {
        let commands = harness.output().viewport_output.values().flat_map(|viewport| viewport.commands.iter());
        commands.filter_map(|command| if let egui::ViewportCommand::CursorPosition(at) = command { Some(*at) } else { None }).collect()
    };
    let field = harness.get_all_by_role_and_label(egui::accesskit::Role::SpinButton, "X").next().unwrap().rect().center();
    let edge = harness.ctx.viewport_rect().max.x - 1.0;
    harness.hover_at(field);
    harness.drag_at(field);
    harness.run();
    for step in 1..6 {
        harness.hover_at(egui::pos2(field.x + (edge - 20.0 - field.x) * step as f32 / 6.0, field.y));
        harness.run();
    }
    harness.hover_at(egui::pos2(edge, field.y));
    harness.step();
    let at_edge = x(&harness);
    let target = *jumps(&harness).first().expect("at the window's edge the pointer is moved");
    assert!(target.x < field.x, "the pointer goes to the opposite edge");
    harness.run();
    // The platform moves the pointer where it was asked to.
    harness.hover_at(target);
    harness.run();
    assert_eq!(x(&harness), at_edge, "the jump does not change the value");
    let on = target + egui::vec2(60.0, 0.0);
    harness.hover_at(on);
    harness.run();
    assert!(x(&harness) > at_edge, "the drag goes on past the edge: {at_edge} then {}", x(&harness));
    // Only the release, so this step's output is the frame that handles it.
    harness.event(egui::Event::PointerButton { pos: on, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    harness.step();
    let back = jumps(&harness);
    assert!(back.iter().any(|at| at.distance(field) < 1.0), "letting go brings the pointer back to where the drag began: {back:?}");
    harness.run();
    let field_now = harness.get_all_by_role_and_label(egui::accesskit::Role::SpinButton, "X").next().unwrap().rect().center();
    harness.hover_at(field_now);
    harness.run();
    assert!(x(&harness) > at_edge, "the value stays where the drag left it");
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
    let declared = graph(&harness).property("label").cloned();
    assert_eq!(declared.map(|p| p.ty), Some(bayleaf_engine::graph::VariableType::Text), "Add Property > Text declares a text property");
    // Get and Set place a node that reads it and one that sets it.
    harness.get_by_label("Get property label").click();
    harness.run();
    harness.get_by_label("Set property label").click();
    harness.run();
    let kinds: Vec<String> = graph(&harness).nodes.iter().filter(|n| n.values.get("name") == Some(&json!("label"))).map(|n| n.kind.clone()).collect();
    assert!(kinds.contains(&"textProperty".to_owned()) && kinds.contains(&"setTextProperty".to_owned()), "{kinds:?}");
    // The default is edited in the panel.
    harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Start of label").click();
    harness.run();
    harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Start of label").type_text("Open");
    harness.run();
    assert_eq!(graph(&harness).property("label").map(|p| p.default.clone()), Some(json!("Open")));
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
    assert_eq!(
        scripts.0[0].properties.get("label").and_then(|v| v.text()),
        Some("OpenShop"),
        "the field starts at the panel's default, and the typed text is the object's value"
    );
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

#[test]
fn export_game_opens_its_own_window_that_checks_the_project_for_the_chosen_mode() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-export-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let mut harness = harness();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir.join("Game") }), None);
    harness.run();
    harness.get_by_label("File").click();
    harness.run();
    harness.query_all_by_label_contains("Export Game…").next().expect("File > Export Game").click();
    harness.run_steps(3);
    assert!(harness.query_by_label_contains("How each script would run").is_some(), "the window lists the check's findings and scripts");

    // Native says plainly what it gives up; a project with an error cannot export, and says why.
    std::fs::remove_file(dir.join("Game").join("scenes").join("main.bscene")).unwrap();
    harness.query_all_by_label_contains("Native").next().expect("the modes").click();
    harness.run_steps(3);
    assert!(harness.query_all_by_label_contains("Warning: Native scripts run with no sandbox").next().is_some(), "warnings are named as warnings");
    assert!(harness.query_all_by_label_contains("Error: The main scene").next().is_some(), "errors are named as errors, not only coloured");
    harness.get_by_role_and_label(egui::accesskit::Role::Button, "Export").click();
    harness.run_steps(2);
    assert!(harness.query_all_by_label_contains("xported").next().is_none(), "nothing exports while the check finds errors");
    let platform = bayleaf_engine::export::Platform::current();
    let chosen = bayleaf_engine::export::chosen_mode(&harness.state().session, platform);
    assert_eq!(chosen, bayleaf_engine::export::ScriptMode::Native, "the choice is kept in the project's settings for this system");

    harness.get_by_label("Close").click();
    harness.run_steps(2);
    assert!(harness.state().export_window.is_none(), "Close closes the window");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn ctrl_s_saves_the_graph_while_it_is_in_use_and_the_scene_otherwise() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-ctrl-s-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let mut harness = graph_harness("script");
    let editor = harness.state_mut();
    editor.dispatch("scene.saveAs", &json!({ "path": dir.join("level.bscene") }), None);
    editor.dispatch("graph.save", &json!({ "path": dir.join("mover.bscript") }), None);
    editor.dispatch("graph.node.add", &json!({ "kind": "log", "position": [300, 40] }), None);
    editor.dispatch("scene.object.create", &json!({ "name": "Box" }), None);
    harness.run();
    let dirty = |h: &Harness<'static, Editor>| (h.state().session.graph().is_some_and(|g| g.dirty), h.state().session.is_dirty());
    assert_eq!(dirty(&harness), (true, true));

    // In use: a click on the graph's canvas, then Ctrl+S saves the graph and not the scene.
    let canvas = harness.get_by_label("Graph canvas").rect();
    let spot = canvas.right_bottom() - egui::vec2(20.0, 20.0);
    harness.hover_at(spot);
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton { pos: spot, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        harness.run();
    }
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    harness.run();
    assert_eq!(dirty(&harness), (false, true), "the graph was saved, the scene left as it is");

    // After a click in the hierarchy, Ctrl+S saves the scene.
    harness.state_mut().dispatch("graph.node.add", &json!({ "kind": "log", "position": [300, 200] }), None);
    harness.get_by_label("Box").click();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    harness.run();
    assert_eq!(dirty(&harness), (true, false), "the scene was saved, the graph left as it is");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_scene_heads_the_hierarchy_takes_dropped_objects_to_the_top_and_shows_in_the_inspector() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-scene-row-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let mut harness = harness();
    assert!(harness.query_by_label("Untitled scene").is_some(), "an unsaved scene is Untitled");
    create_named(&mut harness, "Parent");
    create_named(&mut harness, "Child");
    harness.state_mut().dispatch("scene.saveAs", &json!({ "path": dir.join("castle.bscene") }), None);
    harness.run();
    let scene_row = || "castle scene";
    assert!(harness.query_by_label(scene_row()).is_some(), "the scene's row is named after its file");

    // Child goes under Parent, then is dropped on the scene's row: back to the top.
    let editor = harness.state_mut();
    let ids: Vec<_> = editor.session.scene().children(None).to_vec();
    editor.dispatch("scene.object.reparent", &json!({ "id": ids[1], "parent": ids[0] }), None);
    harness.run();
    assert_eq!(harness.state().session.scene().children(None).len(), 1);
    let from = harness.get_by_label("Child").rect().center();
    let to = harness.get_by_label(scene_row()).rect().center();
    left_drag(&mut harness, from, to);
    assert_eq!(harness.state().session.scene().children(None).len(), 2, "dropped on the scene, the object is at the top");

    // Selecting the scene shows its file in the Inspector; Delete removes nothing.
    harness.get_by_label(scene_row()).click();
    harness.run();
    assert!(harness.state().scene_selected && harness.state().selected.is_none());
    assert!(harness.state().inspected_file.as_ref().is_some_and(|f| f.path.ends_with("castle.bscene")), "the Inspector shows the scene's file");
    harness.key_press(egui::Key::Delete);
    harness.run();
    assert_eq!(harness.state().session.scene().object_count(), 2, "the scene is not an object to delete");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_scene_s_settings_name_a_lighting_asset_the_inspector_edits() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-lighting-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let mut harness = harness();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir.join("Lit") }), None);
    harness.state_mut().project.rescan_soon();
    harness.run_steps(3);
    let project = dir.join("Lit");
    // The scene's row shows its settings: the starter project's lighting asset.
    harness.get_by_label("main scene").click();
    harness.run();
    assert!(harness.query_by_label_contains("Scene Settings").is_some());
    harness.get_by_label("Edit lighting").click();
    harness.run();
    harness.run_steps(2);
    // Its file in the Inspector: a change is written to the file.
    let before = bayleaf_engine::scene::lighting::Lighting::load(&project.join("scenes/daylight.blight")).unwrap();
    harness.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Show Sky").click();
    harness.run();
    let after = bayleaf_engine::scene::lighting::Lighting::load(&project.join("scenes/daylight.blight")).unwrap();
    assert_ne!(before.show_sky, after.show_sky, "unticking Show Sky changes the file");

    // Back on the scene: the engine's light, then New makes an asset beside the scene and uses it.
    harness.state_mut().dispatch("scene.settings.set", &json!({ "lighting": "" }), None);
    harness.get_by_label("main scene").click();
    harness.run();
    harness.get_by_label("New lighting").click();
    harness.run();
    let named = harness.state().session.scene().settings().lighting.clone();
    assert!(named.ends_with(".blight") && project.join(&named).is_file(), "a new asset the scene names: {named}");
    std::fs::remove_dir_all(&dir).ok();
}

/// A script module whose `bayleaf_describe` returns `description`.
fn described_module(description: &str) -> Vec<u8> {
    let bytes: String = description.bytes().map(|b| format!("\\{b:02x}")).collect();
    wat::parse_str(format!(
        r#"(module (memory (export "memory") 1) (data (i32.const 0) "{bytes}")
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_describe") (result i64) i64.const {}))"#,
        description.len()
    ))
    .unwrap()
}

#[test]
fn a_script_lays_out_its_own_inspector_card() {
    let (mut harness, dir) = project_harness("layout");
    let description = r#"{"properties": [["speed", 3.0], ["mode", 0, "whole"], ["boost", 1.0], ["score", 0, "whole"]],
        "inspector": [
            {"group": "Movement", "closed": true, "items": [{"property": "speed", "label": "Top speed", "hint": "Units a second", "range": [0, 20], "slider": true}]},
            {"property": "mode", "choices": ["Walk", "Run"]},
            {"property": "boost", "showIf": ["mode", 1]},
            {"property": "score", "readOnly": true},
            {"button": "Jump now", "message": "jump"},
            {"note": "Keep speed low on small maps.", "warning": true}
        ]}"#;
    std::fs::write(dir.join("layout.wasm"), described_module(description)).unwrap();
    let editor = harness.state_mut();
    let id = editor.dispatch("scene.object.create", &json!({ "name": "Runner" }), None).unwrap()["id"].as_str().map(str::to_owned).unwrap();
    let id = bayleaf_core::StableId::parse_hex(&id).unwrap();
    editor.dispatch("scene.script.add", &json!({ "id": id, "module": "layout.wasm" }), None);
    editor.select_only(Some(id));
    harness.run_steps(2);

    assert!(harness.query_by_label_contains("Movement").is_some() && harness.query_by_label("Top speed").is_none(), "a foldout starts closed");
    harness.query_all_by_label_contains("Movement").next().unwrap().click();
    harness.run_steps(2);
    assert!(harness.query_all_by_label_contains("Top speed").next().is_some(), "opened, it shows the property under its own label");
    assert!(harness.query_by_label_contains("Warning: Keep speed low").is_some(), "a warning is named as one");
    assert!(harness.query_all_by_label_contains("Boost").next().is_none(), "shown only while Mode is Run");
    let jump = harness.get_by_role_and_label(egui::accesskit::Role::Button, "Jump now");
    assert!(egui_kittest::kittest::NodeT::accesskit_node(&jump).is_disabled(), "a button sends its message only while the game plays");

    // Choosing Run from Mode's list sets the whole number and shows Boost.
    harness.get_by_label("Mode").click();
    harness.run();
    harness.get_by_label("Run").click();
    harness.run_steps(2);
    let mode = harness.state().session.scene().get::<bayleaf_engine::scene::components::Scripts>(id).unwrap().0[0].properties.get("mode").cloned();
    assert_eq!(mode, Some(bayleaf_engine::scene::components::PropertyValue::Number(1.0)), "the choice went through scene.script.setProperty");
    assert!(harness.query_all_by_label_contains("Boost").next().is_some(), "Boost shows now");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_script_panel_sets_how_a_property_shows_in_the_inspector() {
    let mut harness = graph_harness("script");
    harness.state_mut().dispatch("graph.property.add", &json!({ "name": "speed", "type": "number" }), None);
    harness.run();
    harness.get_by_label("How the Inspector shows it").click();
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Label").click();
    harness.run();
    harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Label").type_text("Top speed");
    harness.run_steps(2);
    assert_eq!(graph(&harness).property("speed").unwrap().inspector.label, "Top speed", "through graph.property.set");
    harness.get_by_label("Range").click();
    harness.run_steps(2);
    assert!(graph(&harness).property("speed").unwrap().inspector.range.is_some(), "a range turned on");
    harness.get_by_label("Slider").click();
    harness.run_steps(2);
    assert!(graph(&harness).property("speed").unwrap().inspector.slider);
}

#[test]
fn the_export_window_keeps_its_width_for_every_mode() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-export-width-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let mut harness = harness();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir.join("Game") }), None);
    harness.run();
    harness.get_by_label("File").click();
    harness.run();
    harness.query_all_by_label_contains("Export Game…").next().unwrap().click();
    harness.run_steps(3);
    let choose = |harness: &Harness<'static, Editor>| harness.get_by_role_and_label(egui::accesskit::Role::Button, "Choose…").rect();
    let opened = choose(&harness);
    for mode in ["Interpreted", "Compiled to machine code", "Native"] {
        harness.query_all_by_label_contains(mode).next().unwrap().click();
        harness.run_steps(4);
        let now = choose(&harness);
        assert!(now.right() <= opened.right() + 1.0, "{mode}: Choose… moved from {opened:?} to {now:?}, so the window grew");
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_prefab_opens_from_its_copy_and_back_asks_about_unsaved_changes() {
    let (mut harness, dir) = project_harness("prefab-edit");
    let editor = harness.state_mut();
    let id = editor.dispatch("scene.object.create", &json!({ "name": "Crate" }), None).unwrap()["id"].as_str().map(str::to_owned).unwrap();
    let id = bayleaf_core::StableId::parse_hex(&id).unwrap();
    editor.dispatch("scene.prefab.create", &json!({ "id": id, "path": "crate.bprefab" }), None);
    editor.select_only(Some(id));
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::accesskit::Role::Button, "Open").click();
    harness.run_steps(2);
    assert!(harness.query_by_label_contains("Editing prefab: s › crate").is_some(), "the bar shows the way back");
    assert_eq!(harness.state().selected, None, "the scene's selection waits underneath");
    let root = harness.state().session.scene().children(None)[0];
    harness.state_mut().dispatch("scene.object.create", &json!({ "name": "Lid", "parent": root }), None);
    harness.run();
    harness.get_by_label("Back from the prefab").click();
    harness.run();
    assert!(harness.query_by_label("Save the changes?").is_some(), "unsaved changes are not dropped unasked");
    harness.get_by_label("Save and Go Back").click();
    harness.run_steps(2);
    let editor = harness.state();
    assert!(editor.session.open_prefabs().is_empty());
    assert_eq!(editor.selected, Some(id), "the scene's selection is back");
    assert_eq!(editor.session.scene().children(Some(id)).len(), 1, "the copy follows the saved prefab");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_scene_s_settings_make_a_post_processing_asset_the_inspector_edits() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-post-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let mut harness = harness();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir.join("Looks") }), None);
    harness.state_mut().project.rescan_soon();
    harness.run_steps(3);
    let project = dir.join("Looks");
    harness.get_by_label("main scene").click();
    harness.run();
    harness.get_by_label("New post-processing").click();
    harness.run_steps(2);
    let named = harness.state().session.scene().settings().post_processing.clone();
    assert!(named.ends_with(".bpost") && project.join(&named).is_file(), "a new asset the scene names: {named}");

    harness.get_by_label("Edit post-processing").click();
    harness.run_steps(3);
    let before = bayleaf_engine::scene::post::PostProcessing::load(&project.join(&named)).unwrap();
    // The first section's switch: Tone Mapping, on in a new asset.
    assert!(before.tone_mapping.enabled);
    harness.query_all(egui_kittest::kittest::By::new().role(egui::accesskit::Role::CheckBox).label("On")).next().expect("each effect has its switch").click();
    harness.run();
    let after = bayleaf_engine::scene::post::PostProcessing::load(&project.join(&named)).unwrap();
    assert!(!after.tone_mapping.enabled, "unticking Tone Mapping writes the file through post.set");

    // A neutral lookup image is written beside the asset and named in it, and the next one gets a
    // name of its own.
    for _ in 0..2 {
        harness.get_by_label("Make Neutral Lookup").click();
        harness.run_steps(2);
    }
    let looked = bayleaf_engine::scene::post::PostProcessing::load(&project.join(&named)).unwrap();
    assert!(looked.color.enabled && project.join(&looked.color.lookup).is_file(), "{:?}", looked.color);
    let lookups = std::fs::read_dir(project.join(&named).parent().unwrap())
        .unwrap()
        .filter(|e| e.as_ref().is_ok_and(|e| e.file_name().to_string_lossy().contains(" lookup")))
        .count();
    assert_eq!(lookups, 2, "the first lookup is not written over");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_selected_emitter_plays_its_particles_in_the_scene_view_and_pauses() {
    let mut harness = harness();
    let editor = harness.state_mut();
    let id = editor.dispatch("scene.object.create", &json!({ "name": "Smoke" }), None).unwrap()["id"].as_str().map(str::to_owned).unwrap();
    let id = bayleaf_core::StableId::parse_hex(&id).unwrap();
    editor.dispatch("scene.component.add", &json!({ "id": id, "component": "particleEmitter" }), None);
    editor.select_only(Some(id));
    harness.run_steps(2);
    let count = |harness: &Harness<'static, Editor>| harness.state().scene_view.preview.count(Some(id));
    // The preview runs on time as it passes between frames.
    for _ in 0..5 {
        std::thread::sleep(std::time::Duration::from_millis(60));
        harness.run_steps(1);
    }
    assert!(count(&harness) > 0, "the preview emits");
    assert!(harness.query_by_label("Image").is_some(), "the emitter's image is picked from the project's files");
    harness.get_by_label("Pause particles").click();
    harness.run_steps(1);
    let held = count(&harness);
    std::thread::sleep(std::time::Duration::from_millis(60));
    harness.run_steps(2);
    assert_eq!(count(&harness), held, "paused, nothing moves on");
    harness.get_by_label("Restart particles").click();
    harness.run_steps(1);
    assert!(count(&harness) <= 1, "restart starts from nothing");
}

#[test]
fn the_profiler_panel_records_the_game_and_shows_a_frame_by_part() {
    let mut harness = harness();
    harness.state_mut().show_panel(crate::Panel::Profiler);
    harness.run();
    assert!(harness.query_by_label_contains("Press Record").is_some(), "it says how to start");
    harness.get_by_label("Record").click();
    harness.run();
    harness.state_mut().dispatch("play.start", &json!({}), None);
    for _ in 0..4 {
        harness.state_mut().dispatch("play.step", &json!({ "seconds": 0.02 }), None);
        harness.step();
    }
    assert!(harness.query_by_label_contains("Frame times:").is_some(), "the graph says what it shows");
    for part in ["Scripts", "Physics", "Drawing", "Draw Calls"] {
        assert!(harness.query_all_by_label(part).next().is_some(), "{part} has its row and legend");
    }
    harness.get_by_label("Earlier Frame").click();
    harness.run_steps(2);
    assert!(harness.query_by_label("Newest").is_some(), "an earlier frame chosen, the newest is a click away");
    harness.state_mut().dispatch("play.stop", &json!({}), None);
}

/// A file the system drops on the window.
#[derive(Debug)]
struct SystemFile(std::path::PathBuf);

impl egui::DroppedFile for SystemFile {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

/// Drops `files` on the window, with the pointer at `at` (or, like Windows, with no position).
fn drop_files(harness: &mut Harness<'static, Editor>, files: &[std::path::PathBuf], at: Option<egui::Pos2>) {
    if let Some(at) = at {
        harness.input_mut().events.push(egui::Event::PointerMoved(at));
    } else {
        harness.input_mut().events.push(egui::Event::PointerGone);
    }
    for file in files {
        harness.input_mut().dropped_files.push(std::sync::Arc::new(SystemFile(file.clone())));
    }
    // Without a position, the drop waits a moment for one.
    let steps = if at.is_some() { 2 } else { 60 };
    for _ in 0..steps {
        harness.step();
    }
}

#[test]
fn files_dropped_from_the_system_come_into_the_folder_or_the_scene_they_land_on() {
    let mut harness = harness();
    let base = std::env::temp_dir().join(format!("bayleaf-ui-drop-{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    let (dir, outside) = (base.join("Game"), base.join("outside"));
    std::fs::create_dir_all(&outside).unwrap();
    let logo = outside.join("logo.png");
    std::fs::write(&logo, include_bytes!("../../../assets/brand/bayleaf-logo.png")).unwrap();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.state_mut().project.current = dir.join("materials");
    harness.run_steps(2);

    // No pointer position with the drop: into the Project panel's open folder, and shown there.
    drop_files(&mut harness, std::slice::from_ref(&logo), None);
    assert!(dir.join("materials/logo.png").is_file(), "{}", harness.state().status);
    assert_eq!(harness.state().project.selected.as_deref(), Some(dir.join("materials/logo.png").as_path()));

    // Onto a folder's tile: into that folder.
    harness.state_mut().project.current = dir.clone();
    harness.run_steps(2);
    let tile = harness.get_by_label("assets, Folder").rect().center();
    drop_files(&mut harness, std::slice::from_ref(&logo), Some(tile));
    assert!(dir.join("assets/logo.png").is_file(), "{}", harness.state().status);

    // Onto the hierarchy: a model becomes an object at the top, selected.
    let cube_row = harness.get_by_label("Cube").rect().center();
    let before = harness.state().session.scene().children(None).len();
    let model = outside.join("tri.glb");
    std::fs::write(&model, triangle_glb()).unwrap();
    drop_files(&mut harness, std::slice::from_ref(&model), Some(cube_row));
    let editor = harness.state();
    assert_eq!(editor.session.scene().children(None).len(), before + 1, "{}", editor.status);
    let placed = editor.selected.unwrap();
    assert!(editor.session.scene().component(placed, "mesh").unwrap()["model"].as_str().unwrap().ends_with("tri.glb"));

    // Its import settings show in the Inspector, and a change is saved beside it.
    let imported = harness.state().session.script_dir().join(harness.state().session.scene().component(placed, "mesh").unwrap()["model"].as_str().unwrap());
    harness.state_mut().select_only(None);
    harness.state_mut().inspected_file = Some(crate::file_inspector::Inspected::new(imported.clone()));
    harness.run_steps(2);
    harness.get_by_label("Z Up").click();
    harness.run_steps(2);
    let saved = std::fs::read_to_string(bayleaf_render::assets::import::settings_path(&imported)).unwrap();
    assert!(saved.contains("\"z\""), "{saved}");
    std::fs::remove_dir_all(&base).ok();
}

/// A one-triangle binary glTF model.
fn triangle_glb() -> Vec<u8> {
    let mut bin: Vec<u8> = [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]].iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
    bin.extend([0u8, 0, 1, 0, 2, 0, 0, 0]);
    let gltf = json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 36}, {"buffer": 0, "byteOffset": 36, "byteLength": 6}],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, -1], "max": [1, 0, 0]},
            {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}
        ]
    });
    let mut text = gltf.to_string().into_bytes();
    while !text.len().is_multiple_of(4) {
        text.push(b' ');
    }
    let total = 12 + 8 + text.len() + 8 + bin.len();
    let mut out = Vec::new();
    out.extend(b"glTF");
    out.extend(2u32.to_le_bytes());
    out.extend((total as u32).to_le_bytes());
    out.extend((text.len() as u32).to_le_bytes());
    out.extend(b"JSON");
    out.extend(&text);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(b"BIN\0");
    out.extend(&bin);
    out
}

#[test]
fn with_a_gpu_a_game_interface_answers_the_player_and_reads_in_the_accessibility_tree() {
    let Some((_turn, mut harness)) = gpu_harness() else { return };
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-interface-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.state_mut().dispatch("asset.create", &json!({ "kind": "interface", "folder": "", "name": "menu" }), None);
    harness.state_mut().dispatch("asset.create", &json!({ "kind": "interfaceStyle", "folder": "", "name": "look" }), None);
    // The Start button's script changes the title.
    let menu = wat::parse_str(
        r#"(module
            (import "bayleaf" "message_is" (func $is (param i32 i32) (result i32)))
            (import "bayleaf" "self_handle" (func $self (result i32)))
            (import "bayleaf" "ui_set_text" (func $text (param i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 100) "pressed:Start")
            (data (i32.const 120) "Title")
            (data (i32.const 140) "Started")
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_update") (param f32))
            (func (export "bayleaf_message") (param f32)
              (if (call $is (i32.const 100) (i32.const 13))
                (then (drop (call $text (call $self) (i32.const 120) (i32.const 5) (i32.const 140) (i32.const 7)))))))"#,
    )
    .unwrap();
    std::fs::write(dir.join("menu.wasm"), menu).unwrap();
    let components = json!({ "interface": { "layout": "menu.bui", "visible": true, "order": 0 }, "scripts": [{ "module": "menu.wasm" }] });
    harness.state_mut().dispatch("scene.object.create", &json!({ "name": "Menu", "components": components }), None);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.run_steps(4);

    let view = harness.get_by_label("Game view, playing").rect();
    let button = harness
        .query_all(egui_kittest::kittest::By::new().predicate(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some("Start")))
        .next()
        .expect("the interface's Start button is in the accessibility tree");
    assert!(view.contains_rect(button.rect()), "drawn over the game");
    let title = harness.get_by_label("My Game").rect();
    assert!(title.bottom() <= button.rect().top(), "the title above the button, as the layout says");
    button.click();
    harness.run_steps(3);
    assert!(harness.query_by_label("Started").is_some(), "the script heard the press and the title changed: {}", harness.state().status);
    let shown = picture(&mut harness);
    keep_for_review(&shown, "game-interface");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.run_steps(2);
    // The layout in the Interface panel, for review.
    harness.state_mut().open_interface(&dir.join("menu.bui"));
    harness.run_steps(3);
    let panel = picture(&mut harness);
    keep_for_review(&panel, "interface-panel");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_inspector_previews_an_interface_at_the_screen_size_chosen() {
    let mut harness = harness();
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-interface-preview-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.state_mut().dispatch("asset.create", &json!({ "kind": "interface", "folder": "", "name": "menu" }), None);
    harness.state_mut().inspected_file = Some(crate::file_inspector::Inspected::new(dir.join("menu.bui")));
    harness.run_steps(2);
    let start = |harness: &Harness<'static, Editor>| {
        harness
            .query_all(egui_kittest::kittest::By::new().predicate(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some("Start")))
            .next()
            .expect("the preview shows the Start button")
            .rect()
    };
    let wide = start(&harness);
    harness.get_by_label("Screen").click();
    harness.run();
    harness.get_by_label("Phone, upright (390 x 844)").click();
    harness.run_steps(2);
    let upright = start(&harness);
    assert!(upright.center().y > wide.center().y + 20.0, "an upright screen is taller, so the centred button sits lower: {wide:?} then {upright:?}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_interface_panel_edits_a_layout_from_its_tree_fields_and_preview() {
    let mut harness = harness();
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-interface-panel-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.state_mut().dispatch("asset.create", &json!({ "kind": "interface", "folder": "", "name": "menu" }), None);
    let file = dir.join("menu.bui");
    harness.state_mut().open_interface(&file);
    harness.run_steps(2);
    let read = || bayleaf_interface::Layout::load(&file).unwrap();

    // The tree lists the starter's elements; picking one shows its fields.
    harness.get_by_label("Button: Start").click();
    harness.run_steps(2);
    assert_eq!(harness.state().interface_view.selected, vec![1]);
    // Added after the picked button, picked, and renamed in its Name field.
    harness.get_by_label("Add").click();
    harness.run();
    harness.get_by_label("Toggle").click();
    harness.run_steps(2);
    assert_eq!(read().root.children[2].kind, bayleaf_interface::Kind::Toggle);
    assert_eq!(harness.state().interface_view.selected, vec![2]);
    harness.get_by_label("Name").click();
    harness.run();
    harness.get_by_label("Name").type_text("Sound");
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    assert_eq!(read().root.children[2].name, "Sound", "{}", harness.state().status);
    // A style property, added from the list with a value to start from; Undo takes it back.
    harness.get_by_label("Add Property").click();
    harness.run();
    harness.get_by_label("fontSize").click();
    harness.run_steps(2);
    assert!(read().root.children[2].style.contains_key("fontSize"), "{} {:?}", harness.state().status, read().root.children[2]);
    harness.get_by_label("Undo").click();
    harness.run_steps(2);
    assert!(!read().root.children[2].style.contains_key("fontSize"), "undone");
    assert_eq!(read().root.children[2].name, "Sound", "only the last change");

    // A click in the preview picks the element under it.
    let start = harness
        .query_all(egui_kittest::kittest::By::new().predicate(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some("Start")))
        .next()
        .expect("the preview shows the Start button")
        .rect();
    harness.input_mut().events.push(egui::Event::PointerMoved(start.center()));
    harness.run();
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: start.center(),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run();
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: start.center(),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run_steps(2);
    assert_eq!(harness.state().interface_view.selected, vec![1], "the Start button, picked in the preview");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn save_as_part_in_the_interface_panel_puts_a_part_in_its_place() {
    let mut harness = harness();
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-save-part-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.state_mut().dispatch("asset.create", &json!({ "kind": "interface", "folder": "", "name": "menu" }), None);
    harness.state_mut().open_interface(&dir.join("menu.bui"));
    harness.run_steps(2);
    harness.get_by_label("Button: Start").click();
    harness.run_steps(2);
    harness.get_by_label("Save as Part").click();
    harness.run_steps(2);
    assert!(dir.join("Start.bui").is_file(), "{}", harness.state().status);
    assert!(harness.query_by_label("Part: Start").is_some(), "the tree shows the part in the button's place");
    // The part's text changes here only: a row for each named element inside it.
    harness.get_by_label("Part: Start").click();
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Start Text").click();
    harness.run();
    harness.get_by_role_and_label(egui::accesskit::Role::TextInput, "Start Text").type_text("Begin");
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    let menu = bayleaf_interface::Layout::load(&dir.join("menu.bui")).unwrap();
    assert_eq!(menu.root.children[1].texts.get("Start").map(String::as_str), Some("Begin"), "{}", harness.state().status);
    let part = bayleaf_interface::Layout::load(&dir.join("Start.bui")).unwrap();
    assert_eq!(part.root.text, "Start", "the part's own file keeps its text");
    // Open Part edits the part's own layout.
    harness.get_by_label("Open Part").click();
    harness.run_steps(2);
    assert_eq!(harness.state().interface_view.path.as_deref(), Some(dir.join("Start.bui").as_path()));
    std::fs::remove_dir_all(&dir).ok();
}

/// A project with a character card part, a display using it, and a visual script that adds a
/// second card when Start is pressed. Returns the project folder.
fn display_with_a_card(harness: &mut Harness<'static, Editor>, name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let editor = harness.state_mut();
    editor.dispatch("project.new", &json!({ "path": dir }), None);
    let card = json!({
        "format": "bayleaf.interface", "version": 1,
        "root": { "kind": "panel", "name": "Card",
            "style": { "width": "200px", "padding": "12px", "gap": "6px", "background": "#2d3440e6", "radius": "8px" },
            "children": [
                { "kind": "text", "name": "Name", "text": "Hero", "style": { "fontSize": "22px" } },
                { "kind": "text", "text": "Health" },
                { "kind": "progress", "name": "Health", "value": 0.7, "style": { "width": "100%" } },
                { "kind": "button", "name": "Use", "text": "Use Potion" }
            ] }
    });
    editor.dispatch("ui.layout.set", &json!({ "path": "card.bui", "layout": card }), None);
    let display = json!({
        "format": "bayleaf.interface", "version": 1,
        "root": { "kind": "column", "name": "Display",
            "style": { "width": "100%", "height": "100%", "padding": "24px", "gap": "16px" },
            "children": [
                { "kind": "row", "style": { "alignItems": "start" }, "children": [
                    { "kind": "part", "name": "Character", "layout": "card.bui" },
                    { "kind": "panel", "style": { "grow": "1" } },
                    { "kind": "button", "name": "Start", "text": "Add to Party" }
                ] },
                { "kind": "row", "name": "Party", "style": { "gap": "16px" } }
            ] }
    });
    editor.dispatch("ui.layout.set", &json!({ "path": "hud.bui", "layout": display }), None);
    // The visual script: pressing the button adds a card to the party column.
    editor.dispatch("graph.new", &json!({ "domain": "script" }), None);
    let add_node = |editor: &mut Editor, kind: &str, x: f32| {
        editor.dispatch("graph.node.add", &json!({ "kind": kind, "position": [x, 40.0] }), None).unwrap()["node"].as_u64().unwrap()
    };
    let pressed = add_node(editor, "interfacePressed", 40.0);
    let add = add_node(editor, "addInterfaceElement", 320.0);
    for (node, setting, value) in
        [(pressed, "element", json!("Start")), (add, "into", json!("Party")), (add, "what", json!("card.bui")), (add, "name", json!("Ally"))]
    {
        editor.dispatch("graph.node.set", &json!({ "node": node, "name": setting, "value": value }), None);
    }
    editor.dispatch("graph.link.add", &json!({ "from": { "node": pressed, "pin": "then" }, "to": { "node": add, "pin": "run" } }), None);
    editor.dispatch("graph.save", &json!({ "path": dir.join("hud.bscript") }), None);
    let components = json!({ "interface": { "layout": "hud.bui", "visible": true, "order": 0 }, "scripts": [{ "module": "hud.bscript" }] });
    editor.dispatch("scene.object.create", &json!({ "name": "HUD", "components": components }), None);
    dir
}

#[test]
fn with_a_gpu_a_visual_script_adds_a_saved_part_to_the_display() {
    let Some((_turn, mut harness)) = gpu_harness() else { return };
    let dir = display_with_a_card(&mut harness, "party");
    harness.run_steps(3);
    let graph = picture(&mut harness);
    keep_for_review(&graph, "interface-visual-script");
    harness.state_mut().open_interface(&dir.join("hud.bui"));
    harness.run_steps(3);
    assert!(harness.query_by_label("Part: Character").is_some(), "the display's tree shows the part");
    let panel = picture(&mut harness);
    keep_for_review(&panel, "interface-part-in-panel");

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.run_steps(4);
    let heroes = |harness: &Harness<'static, Editor>| harness.query_all_by_label("Hero").count();
    assert_eq!(heroes(&harness), 1, "one card before");
    let button = harness
        .query_all(egui_kittest::kittest::By::new().predicate(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some("Add to Party")))
        .next()
        .expect("the display's button");
    button.click();
    harness.run_steps(3);
    assert_eq!(heroes(&harness), 2, "the script added a second card: {}", harness.state().status);
    let playing = picture(&mut harness);
    keep_for_review(&playing, "interface-part-added-while-playing");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.run_steps(2);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn with_a_gpu_clicking_a_panel_with_on_click_reaches_the_scripts() {
    let Some((_turn, mut harness)) = gpu_harness() else { return };
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-on-click-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let editor = harness.state_mut();
    editor.dispatch("project.new", &json!({ "path": dir }), None);
    // A treasure chest drawn as a panel with a label: not a button, but it answers a click.
    let layout = json!({
        "format": "bayleaf.interface", "version": 1,
        "root": { "kind": "column", "name": "Screen",
            "style": { "width": "100%", "height": "100%", "padding": "24px", "gap": "12px", "alignItems": "center" },
            "children": [
                { "kind": "text", "name": "Title", "text": "Click the chest", "style": { "fontSize": "26px" } },
                { "kind": "panel", "name": "Chest", "onClick": "openChest",
                  "style": { "width": "160px", "height": "100px", "background": "#8a5a2be6", "radius": "10px", "alignItems": "center", "justify": "center" },
                  "children": [ { "kind": "text", "text": "Chest" } ] }
            ] }
    });
    editor.dispatch("ui.layout.set", &json!({ "path": "chest.bui", "layout": layout }), None);
    editor.dispatch("graph.new", &json!({ "domain": "script" }), None);
    let add_node = |editor: &mut Editor, kind: &str, x: f32| {
        editor.dispatch("graph.node.add", &json!({ "kind": kind, "position": [x, 40.0] }), None).unwrap()["node"].as_u64().unwrap()
    };
    let on_message = add_node(editor, "message", 40.0);
    let set = add_node(editor, "setInterfaceText", 320.0);
    for (node, setting, value) in [(on_message, "message", json!("openChest")), (set, "element", json!("Title")), (set, "text", json!("You found 50 gold"))] {
        editor.dispatch("graph.node.set", &json!({ "node": node, "name": setting, "value": value }), None);
    }
    editor.dispatch("graph.link.add", &json!({ "from": { "node": on_message, "pin": "then" }, "to": { "node": set, "pin": "run" } }), None);
    editor.dispatch("graph.save", &json!({ "path": dir.join("chest.bscript") }), None);
    let components = json!({ "interface": { "layout": "chest.bui", "visible": true, "order": 0 }, "scripts": [{ "module": "chest.bscript" }] });
    editor.dispatch("scene.object.create", &json!({ "name": "HUD", "components": components }), None);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.run_steps(4);

    // The panel reads as a button by its name, and a click on it reaches the script.
    let chest = harness
        .query_all(egui_kittest::kittest::By::new().predicate(|n| n.role() == egui::accesskit::Role::Button && n.label().as_deref() == Some("Chest")))
        .next()
        .expect("the clickable panel is a button in the accessibility tree");
    chest.click();
    harness.run_steps(3);
    assert!(harness.query_by_label("You found 50 gold").is_some(), "{}", harness.state().status);
    let shown = picture(&mut harness);
    keep_for_review(&shown, "interface-on-click");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.run_steps(2);
    // The chest picked in the Interface panel, its On Click field showing, for review.
    harness.state_mut().open_interface(&dir.join("chest.bui"));
    harness.state_mut().interface_view.selected = vec![1];
    harness.run_steps(3);
    assert!(harness.query_by_label("On Click").is_some(), "the panel shows the On Click field");
    let panel = picture(&mut harness);
    keep_for_review(&panel, "interface-on-click-field");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn with_a_gpu_a_click_on_the_interface_does_not_reach_the_game() {
    let Some((_turn, mut harness)) = gpu_harness() else { return };
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-pointer-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let editor = harness.state_mut();
    editor.dispatch("project.new", &json!({ "path": dir }), None);
    // A bar with a background along the top; the rest of the screen is the game.
    let layout = json!({
        "format": "bayleaf.interface", "version": 1,
        "root": { "kind": "column", "style": { "width": "100%", "height": "100%" }, "children": [
            { "kind": "panel", "name": "Bar", "style": { "height": "80px", "background": "#202020e0" },
              "children": [ { "kind": "text", "text": "Inventory" } ] }
        ] }
    });
    editor.dispatch("ui.layout.set", &json!({ "path": "hud.bui", "layout": layout }), None);
    // Counts the steps the left button is down for the game, and those the pointer is over the interface.
    let counter = wat::parse_str(
        r#"(module
            (import "bayleaf" "mouse_down" (func $down (param i32) (result i32)))
            (import "bayleaf" "pointer_over_interface" (func $over (result i32)))
            (import "bayleaf" "state_get" (func $get (param i32 i32 i32 i32 i32) (result i32)))
            (import "bayleaf" "state_set" (func $set (param i32 i32 i32 i32 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 100) "clicks")
            (data (i32.const 120) "over")
            (func $bump (param $at i32) (param $len i32)
              (drop (call $get (i32.const 0) (i32.const 0) (local.get $at) (local.get $len) (i32.const 200)))
              (drop (call $set (i32.const 0) (i32.const 0) (local.get $at) (local.get $len) (f64.add (f64.load (i32.const 200)) (f64.const 1)))))
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_update") (param f32)
              (if (call $down (i32.const 0)) (then (call $bump (i32.const 100) (i32.const 6))))
              (if (call $over) (then (call $bump (i32.const 120) (i32.const 4))))))"#,
    )
    .unwrap();
    std::fs::write(dir.join("counter.wasm"), counter).unwrap();
    let components = json!({ "interface": { "layout": "hud.bui", "visible": true, "order": 0 }, "scripts": [{ "module": "counter.wasm" }] });
    editor.dispatch("scene.object.create", &json!({ "name": "HUD", "components": components }), None);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.run_steps(4);
    let view = harness.get_by_label("Game view, playing").rect();
    let value = |harness: &Harness<'static, Editor>, name: &str| harness.state().session.game_state().and_then(|s| s.game.get(name).copied()).unwrap_or(0.0);
    let hold = |harness: &mut Harness<'static, Editor>, at: egui::Pos2| {
        harness.input_mut().events.push(egui::Event::PointerMoved(at));
        harness.run_steps(1);
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run_steps(4);
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run_steps(1);
    };

    // Held on the bar: the interface's, not the game's.
    hold(&mut harness, view.center_top() + egui::vec2(0.0, 40.0));
    assert_eq!(value(&harness, "clicks"), 0.0, "the bar takes the click");
    assert!(value(&harness, "over") > 0.0, "scripts can tell the pointer is over the interface");
    // Held on the bare game: the game's.
    hold(&mut harness, view.center());
    assert!(value(&harness, "clicks") > 0.0, "below the bar the game gets the click");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.run_steps(2);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn each_graph_opens_in_a_tab_of_its_own_and_its_tab_says_when_it_has_changes() {
    let mut harness = harness();
    let tab = |harness: &Harness<'static, Editor>, name: &str| {
        let name = name.to_owned();
        harness
            .query_all(
                egui_kittest::kittest::By::new().predicate(move |n| n.role() == egui::accesskit::Role::Tab && n.label().as_deref() == Some(name.as_str())),
            )
            .count()
    };
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-graph-tabs-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.state_mut().open_graph(&dir.join("materials/ground.bmaterial"));
    harness.run_steps(3);
    harness.state_mut().open_graph(&dir.join("scripts/spin.bscript"));
    harness.run_steps(3);
    assert_eq!((tab(&harness, "ground.bmaterial"), tab(&harness, "spin.bscript")), (1, 1), "a tab for each");
    assert_eq!(tab(&harness, "Graph"), 0, "the start tab goes once a graph is open");
    assert_eq!(harness.state().session.graphs().len(), 2);

    // Opening one that is open brings its tab forward; its tab then is the graph in use.
    harness.state_mut().open_graph(&dir.join("materials/ground.bmaterial"));
    harness.run_steps(3);
    assert_eq!(harness.state().session.graphs().len(), 2, "not opened twice");
    let ground = harness.state().session.graph().unwrap().id;
    assert!(harness.state().session.graph().unwrap().path.as_ref().unwrap().ends_with("ground.bmaterial"));
    // A change shows in its tab's name.
    harness.state_mut().dispatch("graph.node.add", &json!({ "kind": "color", "position": [0, 0] }), None);
    harness.run_steps(3);
    assert_eq!(tab(&harness, "ground.bmaterial •"), 1, "the tab says it has unsaved changes");
    // The other tab, clicked, is the graph in use.
    harness
        .query_all(egui_kittest::kittest::By::new().predicate(|n| n.role() == egui::accesskit::Role::Tab && n.label().as_deref() == Some("spin.bscript")))
        .next()
        .unwrap()
        .click();
    harness.run_steps(3);
    assert!(harness.state().session.graph().unwrap().path.as_ref().unwrap().ends_with("spin.bscript"));
    // Closed by a command, its tab goes; the other stays.
    harness.state_mut().dispatch("graph.close", &json!({ "id": ground }), None);
    harness.run_steps(3);
    assert_eq!((tab(&harness, "ground.bmaterial •"), tab(&harness, "spin.bscript")), (0, 1));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn with_a_gpu_two_graphs_show_side_by_side_in_tabs_of_their_own() {
    let Some((_turn, mut harness)) = gpu_harness() else { return };
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-graphs-side-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.state_mut().open_graph(&dir.join("materials/ground.bmaterial"));
    harness.run_steps(3);
    harness.state_mut().open_graph(&dir.join("scripts/spin.bscript"));
    harness.run_steps(3);
    // The script's tab moved beside the material's, as a drag of the tab would put it.
    let spin = harness.state().session.graph().unwrap().id;
    let editor = harness.state_mut();
    if let Some(at) = editor.dock.find_tab(&crate::Panel::Graph(spin)) {
        editor.dock.remove_tab(at);
    }
    let scene = editor.dock.find_tab(&crate::Panel::Scene).unwrap().node_path().node;
    editor.dock.main_surface_mut().split_right(scene, 0.5, vec![crate::Panel::Graph(spin)]);
    harness.run_steps(4);
    let canvases = harness.query_all_by_label("Graph canvas").count();
    assert_eq!(canvases, 2, "both graphs are drawn at once");
    let shown = picture(&mut harness);
    keep_for_review(&shown, "graph-tabs-side-by-side");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn with_a_gpu_an_interface_stands_in_the_world_and_answers_the_pointer_there() {
    let Some((_turn, mut harness)) = gpu_harness() else { return };
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-world-panel-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let editor = harness.state_mut();
    editor.dispatch("project.new", &json!({ "path": dir }), None);
    // A blue sign with a button in its middle.
    let layout = json!({
        "format": "bayleaf.interface", "version": 1,
        "root": { "kind": "column", "name": "Sign",
            "style": { "width": "100%", "height": "100%", "background": "#2050c0ff", "radius": "16px", "alignItems": "center", "justify": "center", "gap": "8px" },
            "children": [
                { "kind": "text", "name": "Title", "text": "A sign in the world", "style": { "fontSize": "28px" } },
                { "kind": "button", "name": "Open", "text": "Open the door", "style": { "fontSize": "24px" } }
            ] }
    });
    editor.dispatch("ui.layout.set", &json!({ "path": "sign.bui", "layout": layout }), None);
    let script = wat::parse_str(
        r#"(module
            (import "bayleaf" "message_is" (func $is (param i32 i32) (result i32)))
            (import "bayleaf" "self_handle" (func $self (result i32)))
            (import "bayleaf" "ui_set_text" (func $text (param i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 100) "pressed:Open")
            (data (i32.const 120) "Title")
            (data (i32.const 140) "The door opens")
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_update") (param f32))
            (func (export "bayleaf_message") (param f32)
              (if (call $is (i32.const 100) (i32.const 12))
                (then (drop (call $text (call $self) (i32.const 120) (i32.const 5) (i32.const 140) (i32.const 14)))))))"#,
    )
    .unwrap();
    std::fs::write(dir.join("sign.wasm"), script).unwrap();
    let interface =
        json!({ "layout": "sign.bui", "visible": true, "order": 0, "space": "world", "size": [2.4, 1.2], "pointsPerUnit": 200, "faceCamera": true });
    let transform = json!({ "position": [0.0, 1.8, 0.0], "rotation": [0.0, 0.0, 0.0], "scale": [1.0, 1.0, 1.0] });
    let made = editor
        .dispatch(
            "scene.object.create",
            &json!({ "name": "Sign", "components": { "transform": transform, "interface": interface, "scripts": [{ "module": "sign.wasm" }] } }),
            None,
        )
        .unwrap();
    let sign = bayleaf_core::StableId::parse_hex(made["id"].as_str().unwrap()).unwrap();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.run_steps(4);

    // Where the sign's middle is on the game's picture.
    let view = harness.get_by_label("Game view, playing").rect();
    let at = {
        let scene = harness.state().session.scene();
        let camera = bayleaf_render::game_camera(scene, view.width() / view.height());
        let world = scene.world_matrix(sign).unwrap().w_axis.truncate();
        let clip = camera.view_proj() * camera.relative_point(world).extend(1.0);
        let ndc = clip.truncate() / clip.w;
        view.min + egui::vec2((ndc.x + 1.0) / 2.0 * view.width(), (1.0 - ndc.y) / 2.0 * view.height())
    };
    assert!(view.contains(at), "the sign is in view");
    // Its elements are in the window's accessibility tree, where they show in the view.
    let open = harness.get_by_role_and_label(egui::accesskit::Role::Button, "Open the door").rect();
    assert!(view.contains(open.center()), "the sign's button is found over the view: {open:?} in {view:?}");
    assert!(harness.query_by_label("A sign in the world").is_some(), "and its text");
    let shown = picture(&mut harness);
    keep_for_review(&shown, "world-interface");
    // The sign's blue shows a little above its middle (the title), where the scene had sky or the cube.
    let probe = at + egui::vec2(0.0, -view.height() * 0.06);
    let pixel = |p: egui::Pos2| {
        let i = ((p.y as u32 * shown.width + p.x as u32) * 4) as usize;
        [shown.rgba[i], shown.rgba[i + 1], shown.rgba[i + 2]]
    };
    let blue = pixel(probe + egui::vec2(-view.width() * 0.08, 0.0));
    assert!(blue[2] > blue[0] + 40 && blue[2] > blue[1], "the panel is drawn in the scene: {blue:?}");

    // A click on the button, through the scene, reaches the sign's script.
    let press = |harness: &mut Harness<'static, Editor>, pressed: bool| {
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run_steps(2);
    };
    harness.input_mut().events.push(egui::Event::PointerMoved(at));
    harness.run_steps(2);
    press(&mut harness, true);
    press(&mut harness, false);
    harness.run_steps(2);
    let state = harness.state_mut().dispatch("play.ui.state", &json!({ "id": sign }), None).unwrap();
    let title = state["elements"].as_array().unwrap().iter().find(|e| e["name"] == "Title").unwrap()["text"].clone();
    assert_eq!(title, "The door opens", "{}", harness.state().status);
    let after = picture(&mut harness);
    keep_for_review(&after, "world-interface-pressed");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.run_steps(2);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn with_a_gpu_the_gamepad_moves_between_the_interface_s_buttons_and_presses_one() {
    let Some((_turn, mut harness)) = gpu_harness() else { return };
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-pad-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let editor = harness.state_mut();
    editor.dispatch("project.new", &json!({ "path": dir }), None);
    let layout = json!({
        "format": "bayleaf.interface", "version": 1,
        "root": { "kind": "column", "style": { "width": "100%", "height": "100%", "alignItems": "center", "justify": "center", "gap": "12px" }, "children": [
            { "kind": "text", "name": "Title", "text": "Pick one" },
            { "kind": "button", "name": "Play", "text": "Play" },
            { "kind": "button", "name": "Quit", "text": "Quit" }
        ] }
    });
    editor.dispatch("ui.layout.set", &json!({ "path": "menu.bui", "layout": layout }), None);
    let script = wat::parse_str(
        r#"(module
            (import "bayleaf" "message_is" (func $is (param i32 i32) (result i32)))
            (import "bayleaf" "self_handle" (func $self (result i32)))
            (import "bayleaf" "ui_set_text" (func $text (param i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 100) "pressed:Quit")
            (data (i32.const 120) "Title")
            (data (i32.const 140) "Quitting")
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_update") (param f32))
            (func (export "bayleaf_message") (param f32)
              (if (call $is (i32.const 100) (i32.const 12))
                (then (drop (call $text (call $self) (i32.const 120) (i32.const 5) (i32.const 140) (i32.const 8)))))))"#,
    )
    .unwrap();
    std::fs::write(dir.join("menu.wasm"), script).unwrap();
    let made = editor
        .dispatch("scene.object.create", &json!({ "name": "Menu", "components": { "interface": { "layout": "menu.bui", "visible": true, "order": 0 }, "scripts": [{ "module": "menu.wasm" }] } }), None)
        .unwrap();
    let menu = made["id"].clone();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.run_steps(4);
    // Down, down, then the bottom face button: from nothing to Play, to Quit, pressed.
    let tap = |harness: &mut Harness<'static, Editor>, bit: u32| {
        harness.state_mut().pad_for_tests = Some(bayleaf_engine::script::Pad { buttons: bit, ..Default::default() });
        harness.run_steps(2);
        harness.state_mut().pad_for_tests = Some(bayleaf_engine::script::Pad::default());
        harness.run_steps(2);
    };
    tap(&mut harness, 1 << 13);
    tap(&mut harness, 1 << 13);
    tap(&mut harness, 1 << 0);
    let state = harness.state_mut().dispatch("play.ui.state", &json!({ "id": menu }), None).unwrap();
    let title = state["elements"].as_array().unwrap().iter().find(|e| e["name"] == "Title").unwrap()["text"].clone();
    assert_eq!(title, "Quitting", "the D-pad reached Quit and the face button pressed it");
    harness.state_mut().pad_for_tests = None;
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.run_steps(2);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_style_file_is_edited_rule_by_rule_in_the_inspector() {
    let mut harness = harness();
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-style-edit-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.state_mut().dispatch("asset.create", &json!({ "kind": "interfaceStyle", "folder": "", "name": "look" }), None);
    let file = dir.join("look.bstyle");
    harness.state_mut().inspected_file = Some(crate::file_inspector::Inspected::new(file.clone()));
    harness.run_steps(2);
    let read = || bayleaf_interface::StyleSheet::load(&file).unwrap();
    assert!(harness.query_by_label("Every button").is_some(), "the starter's rule, by what it picks");
    let before = read().rules[0].set.len();

    harness.get_by_label("Add Property").click();
    harness.run();
    harness.get_by_label("fontSize").click();
    harness.run_steps(2);
    assert_eq!(read().rules[0].set.len(), before + 1, "{}", harness.state().status);
    assert!(read().rules[0].set.contains_key("fontSize"));

    harness.get_by_label("Add Rule").click();
    harness.run_steps(2);
    assert_eq!(read().rules.len(), 2);
    let removes: Vec<egui::Rect> = harness.query_all_by_label("Remove Rule").map(|n| n.rect()).collect();
    assert_eq!(removes.len(), 2);
    harness.query_all_by_label("Remove Rule").last().unwrap().click();
    harness.run_steps(2);
    assert_eq!(read().rules.len(), 1, "the new rule went again");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn with_a_gpu_a_wall_keeps_the_pointer_from_a_world_panel_and_the_keyboard_types_into_one() {
    let Some((_turn, mut harness)) = gpu_harness() else { return };
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-world-keys-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let editor = harness.state_mut();
    editor.dispatch("project.new", &json!({ "path": dir }), None);
    let field = json!({
        "format": "bayleaf.interface", "version": 1,
        "root": { "kind": "column", "style": { "width": "100%", "height": "100%", "background": "#203040ff", "alignItems": "center", "justify": "center" },
            "children": [ { "kind": "textField", "name": "Name", "style": { "width": "80%", "fontSize": "28px" } } ] }
    });
    editor.dispatch("ui.layout.set", &json!({ "path": "field.bui", "layout": field }), None);
    let button = json!({
        "format": "bayleaf.interface", "version": 1,
        "root": { "kind": "column", "style": { "width": "100%", "height": "100%", "background": "#402020ff", "alignItems": "center", "justify": "center" },
            "children": [ { "kind": "button", "name": "Hidden", "text": "Behind the wall", "style": { "width": "90%", "height": "80%" } } ] }
    });
    editor.dispatch("ui.layout.set", &json!({ "path": "button.bui", "layout": button }), None);
    let panel = |layout: &str, x: f64| {
        json!({
            "transform": { "position": [x, 1.6, 0.0], "rotation": [0.0, 0.0, 0.0], "scale": [1.0, 1.0, 1.0] },
            "interface": { "layout": layout, "visible": true, "order": 0, "space": "world", "size": [1.6, 0.8], "pointsPerUnit": 200, "faceCamera": true }
        })
    };
    let field_id = editor.dispatch("scene.object.create", &json!({ "name": "Field", "components": panel("field.bui", -1.0) }), None).unwrap()["id"].clone();
    let button_id = editor.dispatch("scene.object.create", &json!({ "name": "Behind", "components": panel("button.bui", 1.0) }), None).unwrap()["id"].clone();
    // A wall halfway between the camera and the second panel.
    let (eye, behind) = {
        let scene = editor.session.scene();
        let camera = bayleaf_render::game_camera(scene, 16.0 / 9.0);
        let behind = scene.world_matrix(bayleaf_core::StableId::parse_hex(button_id.as_str().unwrap()).unwrap()).unwrap().w_axis.truncate();
        (camera.origin + camera.position.as_dvec3(), behind)
    };
    let wall = (eye + behind) * 0.5;
    editor.dispatch(
        "scene.object.create",
        &json!({ "name": "Wall", "components": { "transform": { "position": [wall.x, wall.y, wall.z], "rotation": [0.0, 0.0, 0.0], "scale": [1.5, 1.5, 1.5] }, "mesh": null } }),
        None,
    );
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.run_steps(4);
    let view = harness.get_by_label("Game view, playing").rect();
    let on_screen = |harness: &Harness<'static, Editor>, id: &serde_json::Value| {
        let scene = harness.state().session.scene();
        let camera = bayleaf_render::game_camera(scene, view.width() / view.height());
        let world = scene.world_matrix(bayleaf_core::StableId::parse_hex(id.as_str().unwrap()).unwrap()).unwrap().w_axis.truncate();
        let clip = camera.view_proj() * camera.relative_point(world).extend(1.0);
        let ndc = clip.truncate() / clip.w;
        view.min + egui::vec2((ndc.x + 1.0) / 2.0 * view.width(), (1.0 - ndc.y) / 2.0 * view.height())
    };
    let click = |harness: &mut Harness<'static, Editor>, at: egui::Pos2| {
        harness.input_mut().events.push(egui::Event::PointerMoved(at));
        harness.run_steps(2);
        for pressed in [true, false] {
            harness.input_mut().events.push(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
            harness.run_steps(2);
        }
    };
    let shown = |harness: &mut Harness<'static, Editor>, id: &serde_json::Value, element: &str| {
        let state = harness.state_mut().dispatch("play.ui.state", &json!({ "id": id }), None).unwrap();
        state["elements"].as_array().unwrap().iter().find(|e| e["name"] == element).cloned().unwrap()
    };

    // The wall takes the click meant for the panel behind it: no message reaches its interface.
    let behind_at = on_screen(&harness, &button_id);
    click(&mut harness, behind_at);
    assert!(
        harness.state().session.interface_state(bayleaf_core::StableId::parse_hex(button_id.as_str().unwrap()).unwrap()).is_none(),
        "nothing reached the hidden panel"
    );

    // A click on the other panel's field gives it the keyboard; what is typed goes into it.
    let field_at = on_screen(&harness, &field_id);
    click(&mut harness, field_at);
    harness.input_mut().events.push(egui::Event::Text("Ada".to_owned()));
    harness.run_steps(3);
    assert_eq!(shown(&mut harness, &field_id, "Name")["text"], "Ada", "{}", harness.state().status);
    let picture_now = picture(&mut harness);
    keep_for_review(&picture_now, "world-interface-keyboard");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.run_steps(2);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn dragging_a_row_in_the_interface_tree_moves_the_element_before_into_or_after_another() {
    let mut harness = harness();
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-interface-drag-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.state_mut().dispatch("asset.create", &json!({ "kind": "interface", "folder": "", "name": "menu" }), None);
    let file = dir.join("menu.bui");
    harness.state_mut().open_interface(&file);
    harness.run_steps(2);
    let names = || bayleaf_interface::Layout::load(&file).unwrap().root.children.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
    let drag = |harness: &mut Harness<'static, Editor>, from: &str, to: &str, at: f32| {
        let (start, target) = (harness.get_by_label(from).rect(), harness.get_by_label(to).rect());
        let end = egui::pos2(target.center().x, target.top() + target.height() * at);
        harness.input_mut().events.push(egui::Event::PointerMoved(start.center()));
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: start.center(),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run_steps(2);
        for step in 1..=4 {
            harness.input_mut().events.push(egui::Event::PointerMoved(start.center().lerp(end, step as f32 / 4.0)));
            harness.run_steps(1);
        }
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run_steps(3);
    };

    // Start dropped on Title's upper part goes before it.
    drag(&mut harness, "Button: Start", "Text: Title", 0.1);
    assert_eq!(names(), ["Start", "Title"], "{}", harness.state().status);
    // A panel added after them takes Title dropped on its middle.
    harness.get_by_label("Text: Title").click();
    harness.run();
    harness.get_by_label("Add").click();
    harness.run();
    harness.get_by_label("Panel").click();
    harness.run_steps(2);
    assert_eq!(names().len(), 3);
    let panel = harness.query_all_by_label("Panel").map(|n| n.rect()).find(|r| r.width() > 100.0).is_some();
    assert!(panel, "the new panel has its row");
    drag(&mut harness, "Text: Title", "Panel", 0.5);
    let layout = bayleaf_interface::Layout::load(&file).unwrap();
    assert_eq!(layout.root.children.len(), 2, "Title left the root: {:?}", names());
    assert_eq!(layout.root.children[1].children[0].name, "Title", "and went into the panel");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_interface_panels_keep_their_widths_while_the_mouse_moves_and_leave_the_preview_room() {
    // A wide window and a narrow one, so the panel's tab is narrow too.
    for width in [1440.0_f32, 820.0] {
        let dir = std::env::temp_dir().join(format!("bayleaf-ui-interface-widths-{width}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let mut harness =
            Harness::builder().with_size(egui::vec2(width, 760.0)).build_ui_state(|ui, editor: &mut Editor| editor.ui(ui, None), Editor::default());
        harness.run();
        harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
        harness.state_mut().dispatch("asset.create", &json!({ "kind": "interface", "folder": "", "name": "menu" }), None);
        harness.state_mut().open_interface(&dir.join("menu.bui"));
        harness.run_steps(2);
        // The title has a style property, so its row with a remove button shows.
        harness.get_by_label("Text: Title").click();
        harness.run_steps(2);
        let fields = |harness: &Harness<'static, Editor>| harness.get_by_label("Element").rect();
        let preview = |harness: &Harness<'static, Editor>| harness.query_by_label("Interface preview").map(|n| n.rect().width()).unwrap_or(0.0);
        let before = fields(&harness);
        for step in 0..30 {
            harness.hover_at(before.center() + egui::vec2((step % 7) as f32 * 9.0, (step % 5) as f32 * 30.0));
            harness.run();
        }
        let after = fields(&harness);
        assert!((after.left() - before.left()).abs() < 1.0, "{width}: the fields' column grew from {before:?} to {after:?} as the mouse moved");
        assert!(preview(&harness) > 100.0, "{width}: the preview is squeezed to {}", preview(&harness));
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[test]
fn the_mouse_back_and_forward_buttons_go_through_the_folders_opened_in_the_project_panel() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-folder-history-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let mut harness = harness();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    std::fs::create_dir_all(dir.join("art").join("trees")).unwrap();
    std::fs::create_dir_all(dir.join("sounds")).unwrap();
    harness.state_mut().project.rescan_soon();
    harness.run_steps(2);
    let root = harness.state().project.current.clone();
    // However a folder opens, the panel remembers the one it left.
    for folder in [dir.join("art"), dir.join("art").join("trees")] {
        harness.state_mut().project.current = folder;
        harness.run();
    }
    let at = harness.get_by_label("Folder contents").rect().center();
    let press = |harness: &mut Harness<'static, Editor>, button: egui::PointerButton| {
        harness.event(egui::Event::PointerMoved(at));
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton { pos: at, button, pressed, modifiers: egui::Modifiers::NONE });
        }
        harness.run();
    };
    let current = |harness: &Harness<'static, Editor>| harness.state().project.current.clone();
    press(&mut harness, egui::PointerButton::Extra1);
    assert_eq!(current(&harness), dir.join("art"), "back goes to the folder before");
    press(&mut harness, egui::PointerButton::Extra1);
    assert_eq!(current(&harness), root, "and on to the first");
    press(&mut harness, egui::PointerButton::Extra1);
    assert_eq!(current(&harness), root, "with nothing before it, back stays");
    press(&mut harness, egui::PointerButton::Extra2);
    assert_eq!(current(&harness), dir.join("art"), "forward goes the way back came");

    // Opening another folder forgets the way forward.
    harness.state_mut().project.current = dir.join("sounds");
    harness.run();
    press(&mut harness, egui::PointerButton::Extra2);
    assert_eq!(current(&harness), dir.join("sounds"));

    // From the keyboard: Alt+Left while a file or folder has it, without moving objects.
    harness.state_mut().project.current = dir.clone();
    harness.run();
    create_named(&mut harness, "First");
    create_named(&mut harness, "Second");
    harness.get_by_label("art, Folder").focus();
    harness.run();
    let roots = harness.state().session.scene().children(None).len();
    harness.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowLeft);
    harness.run();
    assert_eq!(current(&harness), dir.join("sounds"), "Alt+Left goes back");
    harness.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowRight);
    harness.run();
    assert_eq!(current(&harness), dir, "Alt+Right forward");
    assert_eq!(harness.state().session.scene().children(None).len(), roots, "the selected object did not move into the one before");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_script_cards_menu_opens_the_script_to_edit_or_says_why_not_and_removes_it() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-card-menu-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let graph = json!({ "format": "bayleaf.graph", "version": 6, "domain": "script", "nodes": [{ "id": 1, "kind": "start", "position": [0.0, 0.0] }] });
    let visual = dir.join("follow.bscript");
    std::fs::write(&visual, serde_json::to_string_pretty(&graph).unwrap()).unwrap();
    // A built module with no Rust source beside it.
    let built = dir.join("mover.wasm");
    std::fs::write(&built, b"\0asm\x01\0\0\0").unwrap();
    let mut harness = harness();
    let module = |p: &std::path::Path| p.to_string_lossy().replace('\\', "/");
    let params = json!({ "name": "Player", "components": { "scripts": [{ "module": module(&visual) }, { "module": module(&built) }] } });
    harness.state_mut().dispatch("scene.object.create", &params, None);
    harness.run();
    let player = harness.state().selected.unwrap();

    // Right-clicking the card's title opens its menu; Edit Script opens the graph.
    let title = harness.get_by_label("Follow").rect().center();
    harness.hover_at(title);
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton { pos: title, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE });
    }
    harness.run();
    harness.get_by_label("Edit Script").click();
    harness.run();
    let open = harness.state().session.graph().and_then(|g| g.path.clone()).map(|p| p.canonicalize().unwrap());
    assert_eq!(open, Some(visual.canonicalize().unwrap()), "the visual script opens in its graph tab");

    // The same menu from its button: a script with nothing to edit has Edit Script turned off.
    harness.get_by_label("More for Mover").click();
    harness.run();
    assert!(egui_kittest::kittest::NodeT::accesskit_node(&harness.get_by_label("Edit Script")).is_disabled(), "no source to open");
    harness.get_by_label("Remove").click();
    harness.run();
    let scripts = harness.state().session.scene().get::<bayleaf_engine::scene::components::Scripts>(player).unwrap().0.clone();
    assert_eq!(scripts.len(), 1, "Remove in the menu removes that script");
    assert!(scripts[0].module.ends_with("follow.bscript"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_cards_menu_resets_it_from_the_keyboard_and_offers_reset_to_prefab_on_a_copy() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-reset-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let mut harness = harness();
    let mesh = |red: f32| json!({ "shape": "sphere", "color": [red, 0.2, 0.3, 1], "metallic": 1, "roughness": 0.1 });
    harness.state_mut().dispatch("scene.saveAs", &json!({ "path": dir.join("room.bscene") }), None).unwrap();
    harness.state_mut().dispatch("scene.object.create", &json!({ "name": "Lamp", "components": { "mesh": mesh(0.1) } }), None).unwrap();
    harness.run();
    let lamp = harness.state().selected.unwrap();
    let red = |harness: &Harness<'static, Editor>| harness.state().session.scene().component(lamp, "mesh").unwrap()["color"][0].as_f64().unwrap();

    // By keyboard: the card's menu button, then Reset.
    harness.get_by_label("More for Mesh").focus();
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert!(harness.query_by_label("Reset to Prefab").is_none(), "a plain object resets to the defaults");
    harness.get_by_label("Reset").focus();
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    let defaults = bayleaf_engine::scene::registry::ComponentRegistry::builtin().find("mesh").unwrap().default_value();
    assert_eq!(harness.state().session.scene().component(lamp, "mesh").unwrap(), defaults, "every field is back to its default");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run();
    assert!((red(&harness) - 0.1).abs() < 1e-6, "one undo step brings the values back");

    // On a prefab copy the menu says it resets to the prefab, and does.
    harness.state_mut().dispatch("scene.prefab.create", &json!({ "id": lamp, "path": "lamp.bprefab" }), None).unwrap();
    harness.state_mut().dispatch("scene.component.set", &json!({ "id": lamp, "component": "mesh", "value": mesh(0.7) }), None).unwrap();
    harness.run();
    harness.get_by_label("More for Mesh").click();
    harness.run();
    assert!(harness.query_by_label("Reset").is_none(), "the plain label is not offered on a copy");
    harness.get_by_label("Reset to Prefab").click();
    harness.run();
    assert!((red(&harness) - 0.1).abs() < 1e-6, "the prefab's colour, not the default");
    assert!(harness.state().status.contains("lamp.bprefab"), "the status says which values came back: {}", harness.state().status);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_native_build_shows_how_far_it_is_in_the_status_bar_and_says_how_it_went() {
    let mut harness = harness();
    let progress = std::sync::Arc::new(std::sync::Mutex::new(bayleaf_engine::native_export::Progress { done: 3, total: 10, doing: "serde".to_owned() }));
    let (sender, answer) = std::sync::mpsc::channel();
    harness.state_mut().native_build =
        Some(crate::export_window::Building { path: "out/My Game".to_owned(), progress: std::sync::Arc::clone(&progress), answer });
    harness.run();
    assert!(harness.query_by_label_contains("3 of 10 parts").is_some(), "the status bar shows the parts built, with the Export window closed");
    if let Ok(mut now) = progress.lock() {
        now.done = 7;
    }
    harness.run();
    assert!(harness.query_by_label_contains("7 of 10 parts").is_some(), "and follows the build");
    sender.send(Ok(())).unwrap();
    harness.run();
    assert!(harness.state().native_build.is_none(), "a finished build is over");
    assert!(harness.state().status.contains("out/My Game"), "the status says where it went: {}", harness.state().status);
}

#[test]
fn events_are_declared_given_values_and_placed_from_the_script_panel() {
    let mut harness = graph_harness("script");
    harness.get_by_label("Add Event").click();
    harness.run();
    let g = graph(&harness);
    assert_eq!(g.events.len(), 1, "Add Event declares one");
    let name = g.events[0].name.clone();
    let on = g.nodes.iter().find(|n| n.kind == "eventStart").map(|n| n.id).expect("and places its On Event node");

    harness.get_by_label("Add Value").click();
    harness.run();
    harness.get_by_label("Number").click();
    harness.run();
    let g = graph(&harness);
    assert_eq!(g.event(&name).unwrap().params.len(), 1);
    assert_eq!(g.value(on, "params"), Some(&json!(["value:number"])), "the On Event node follows");

    harness.get_by_label(&format!("Trigger {name}")).click();
    harness.run();
    let trigger = graph(&harness).nodes.iter().find(|n| n.kind == "triggerEvent").map(|n| n.id).expect("Trigger places a Trigger Event node");
    assert_eq!(graph(&harness).value(trigger, "params"), Some(&json!(["value:number"])));
    assert_eq!(graph(&harness).value(trigger, "event"), Some(&json!(name)));
    assert!(harness.query_by_label_contains("this visual script compiles").is_some(), "{}", harness.state().status);
}

#[test]
fn another_scripts_event_is_subscribed_to_from_the_script_panel_with_its_component_linked() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-subscribe-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let mut harness = harness();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    // Another script of the project declares Died, bringing an amount.
    let health = json!({ "format": "bayleaf.graph", "version": 7, "domain": "script", "nodes": [],
        "events": [{ "name": "Died", "params": [{ "name": "amount", "type": "number" }] }] });
    std::fs::write(dir.join("scripts").join("health.bscript"), serde_json::to_string_pretty(&health).unwrap()).unwrap();
    harness.state_mut().project.rescan_soon();
    harness.run_steps(2);
    harness.state_mut().dispatch("graph.new", &json!({ "domain": "script" }), None);
    harness.run();
    harness.get_by_label("Other Scripts' Events").click();
    harness.run();
    harness.get_by_label("Died · Health").click();
    harness.run();
    harness.get_by_label("Subscribe").click();
    harness.run();
    let g = graph(&harness);
    let of = |kind: &str| g.nodes.iter().find(|n| n.kind == kind).unwrap_or_else(|| panic!("no {kind} node"));
    assert_eq!(of("getComponent").values.get("component"), Some(&json!("Health")), "a Get Component names the script");
    let subscribe = of("subscribeEvent");
    assert_eq!((subscribe.values.get("script"), subscribe.values.get("event")), (Some(&json!("Health")), Some(&json!("Died"))));
    assert_eq!(g.link_into(subscribe.id, "component").map(|l| l.from.node), Some(of("getComponent").id), "its component is linked");
    assert_eq!(of("componentEventStart").values.get("params"), Some(&json!(["amount:number"])), "On Component Event brings the event's values");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_property_becomes_a_list_in_the_panel_and_the_inspector_adds_to_each_objects_own() {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-list-property-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let mut harness = harness();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    harness.state_mut().dispatch("graph.new", &json!({ "domain": "script" }), None);
    harness.run();
    harness.get_by_label("Add Property").click();
    harness.run();
    harness.get_by_label("Whole Number").click();
    harness.run();
    // Its type menu makes it a list; it starts empty, and the panel adds an item.
    harness.get_by_label("What count holds: Whole Number").click();
    harness.run();
    // The open menu's choice: the one drawn last, on top.
    harness.get_all_by_label("List").last().expect("the menu offers List").click();
    harness.run();
    let declared = graph(&harness).property("count").cloned().unwrap();
    assert_eq!((declared.shape, declared.default.clone()), (bayleaf_engine::graph::Shape::List, json!([])), "a list of whole numbers, empty");
    harness.get_by_label("Add an item to count").click();
    harness.run();
    assert_eq!(graph(&harness).property("count").map(|p| p.default.clone()), Some(json!([0])));
    // A list has list nodes, not Get and Set.
    assert!(harness.query_by_label("Get property count").is_none());
    harness.get_by_label("Place a node for count").click();
    harness.run();
    harness.get_by_label("List Length").click();
    harness.run();
    let length = graph(&harness).nodes.iter().find(|n| n.kind == "listLength").cloned().expect("Place Node places it");
    assert_eq!(length.values.get("item"), Some(&json!(["whole", "text"])), "holding what the list holds");
    harness.state_mut().dispatch("graph.save", &json!({ "path": dir.join("waves.bscript") }), None);

    // An object with the script has its own list, starting as the default, and adds to it.
    let editor = harness.state_mut();
    let id = editor.dispatch("scene.object.create", &json!({ "name": "Spawner" }), None).unwrap()["id"].as_str().map(str::to_owned).unwrap();
    let id = bayleaf_core::StableId::parse_hex(&id).unwrap();
    editor.dispatch("scene.script.add", &json!({ "id": id, "module": "waves.bscript" }), None);
    editor.select_only(Some(id));
    harness.run();
    harness.get_by_label("Add an item to Count").click();
    harness.run();
    let scripts = harness.state().session.scene().get::<bayleaf_engine::scene::components::Scripts>(id).cloned().unwrap();
    let two = bayleaf_engine::scene::components::PropertyValue::List { list: vec![bayleaf_engine::scene::components::PropertyValue::Number(0.0); 2] };
    assert_eq!(scripts.0[0].properties.get("count"), Some(&two), "the default's item and the added one");
    assert!(harness.query_by_label("Remove item 1 of Count").is_some(), "each item can be taken out");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_variables_type_changes_from_its_menu_and_its_nodes_follow() {
    let mut harness = graph_harness("script");
    harness.get_by_label("Add Variable").click();
    harness.run();
    harness.get_by_label("Number").click();
    harness.run();
    harness.get_by_label("Get number").click();
    harness.run();
    harness.get_by_label("What number holds: Number").click();
    harness.run();
    harness.get_by_label("Text").click();
    harness.run();
    let g = graph(&harness);
    assert_eq!(g.variable("number").map(|v| v.ty), Some(bayleaf_engine::graph::VariableType::Text));
    assert!(g.nodes.iter().any(|n| n.kind == "textVariable"), "its Get node reads text now");
    assert!(harness.query_by_label_contains("this visual script compiles").is_some(), "{}", harness.state().status);
}

#[test]
fn list_pins_say_what_their_list_holds_and_a_fresh_list_node_has_its_value_pin() {
    let mut harness = graph_harness("script");
    let editor = harness.state_mut();
    editor.dispatch("graph.variable.add", &json!({ "name": "words", "type": "text", "shape": "list" }), None);
    let get = editor.dispatch("graph.node.add", &json!({ "kind": "getList", "position": [0, 0] }), None).unwrap()["node"].clone();
    editor.dispatch("graph.node.set", &json!({ "node": get, "name": "item", "value": ["text"] }), None);
    editor.dispatch("graph.node.set", &json!({ "node": get, "name": "name", "value": "words" }), None);
    editor.dispatch("graph.node.add", &json!({ "kind": "listAdd", "position": [0, 200] }), None);
    harness.run();
    assert!(harness.query_by_label_contains("List (List of Text) of Get List").is_some(), "the list pin says it is a list of text");
    // A list node just placed already has its list and its value, as numbers until it takes another type.
    assert!(harness.query_by_label_contains("List (List of Number) of Add to List").is_some());
    assert!(harness.query_by_label_contains("Value (Number) of Add to List").is_some());
}

#[test]
fn a_maps_keys_type_is_chosen_from_its_type_menu() {
    let mut harness = graph_harness("script");
    harness.state_mut().dispatch("graph.variable.add", &json!({ "name": "waves", "type": "text", "shape": "map" }), None);
    harness.run();
    harness.get_by_label("What waves holds: Map from Text to Text").click();
    harness.run();
    harness.get_by_label("Keys of Whole Number").click();
    harness.run();
    let variable = graph(&harness).variable("waves").cloned().unwrap();
    assert_eq!(variable.key, bayleaf_engine::graph::VariableType::WholeNumber, "the map's keys are whole numbers now");
    // A key it adds is one of that type.
    harness.get_by_label("Add a key to waves").click();
    harness.run();
    assert_eq!(graph(&harness).variable("waves").map(|v| v.start.clone()), Some(json!({ "0": "" })));
    assert!(harness.query_by_label("What waves holds: Map from Whole Number to Text").is_some());
}

#[test]
fn data_files_show_their_shape_s_fields_in_groups_and_resets_and_new_fields_are_saved() {
    let mut harness = harness();
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-data-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    harness.state_mut().dispatch("project.new", &json!({ "path": dir }), None);
    let shape = json!({ "fields": [
        { "name": "hp", "type": "number", "default": 100.0, "group": "Combat" },
        { "name": "drop", "type": "prefab", "default": "" },
    ] });
    assert!(harness.state_mut().dispatch("data.shape.set", &json!({ "path": "Enemy.bshape", "shape": shape }), None).is_some());
    let data = json!({ "shape": "Enemy.bshape", "values": { "hp": 40.0 } });
    assert!(harness.state_mut().dispatch("data.set", &json!({ "path": "Goblin.bdata", "data": data }), None).is_some());
    let hp = |harness: &mut Harness<'static, Editor>| {
        let read = harness.state_mut().dispatch("data.get", &json!({ "path": "Goblin.bdata" }), None).unwrap();
        (read["fields"][0]["value"].clone(), read["fields"][0]["set"].clone())
    };

    // The data file shows its shape's fields, a group's under its title, each with a reset.
    harness.state_mut().inspect_file(dir.join("Goblin.bdata"));
    harness.run_steps(2);
    assert!(harness.query_by_label("Combat").is_some(), "the group's title");
    assert!(harness.query_by_label("Hp").is_some() && harness.query_by_label("Drop").is_some(), "a row per field");
    harness.get_by_label("Reset Hp").click();
    harness.run_steps(2);
    assert_eq!(hp(&mut harness), (json!(100.0), json!(false)), "reset returns the field to the shape's default");

    // The shape: Add Field writes a third field, which the data file then shows.
    harness.state_mut().inspect_file(dir.join("Enemy.bshape"));
    harness.run_steps(2);
    // By keyboard: the button is below the fold of the Inspector's scroll area.
    harness.get_by_label("Add Field").focus();
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    let fields = harness.state_mut().dispatch("data.shape.get", &json!({ "path": "Enemy.bshape" }), None).unwrap()["shape"]["fields"].clone();
    assert_eq!(fields.as_array().map(Vec::len), Some(3));
    harness.state_mut().inspect_file(dir.join("Goblin.bdata"));
    harness.run_steps(2);
    assert!(harness.query_by_label("Field").is_some(), "the new field shows in the data file");
    std::fs::remove_dir_all(&dir).ok();
}
