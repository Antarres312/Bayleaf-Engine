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
