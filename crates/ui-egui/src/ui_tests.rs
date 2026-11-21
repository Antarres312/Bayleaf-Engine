//! Editor tests driven through the accessibility tree, the way assistive technology sees it.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crate::Editor;

fn harness() -> Harness<'static, Editor> {
    let mut harness = Harness::new_ui_state(|ui, editor: &mut Editor| editor.ui(ui), Editor::default());
    harness.run();
    harness
}

#[test]
fn create_button_adds_a_selected_object_shown_in_the_hierarchy() {
    let mut harness = harness();
    harness.get_by_label("Create").click();
    harness.run();
    assert_eq!(harness.state().session.scene().object_count(), 1);
    assert!(harness.state().selected.is_some());
    // With a selection, the button creates a child.
    harness.get_by_label("Create Child").click();
    harness.run();
    let editor = harness.state();
    let root = editor.session.scene().children(None).first().copied();
    assert_eq!(editor.session.scene().children(root).len(), 1);
}

#[test]
fn outdent_moves_a_child_back_to_the_root_and_undo_restores_it() {
    let mut harness = harness();
    harness.get_by_label("Create").click();
    harness.run();
    harness.get_by_label("Create Child").click();
    harness.run();
    harness.get_by_label("Outdent").click();
    harness.run();
    assert_eq!(harness.state().session.scene().children(None).len(), 2);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run();
    assert_eq!(harness.state().session.scene().children(None).len(), 1);
}

#[test]
fn delete_key_removes_the_selected_object() {
    let mut harness = harness();
    harness.get_by_label("Create").click();
    harness.run();
    harness.key_press(egui::Key::Delete);
    harness.run();
    assert_eq!(harness.state().session.scene().object_count(), 0);
}

#[test]
fn inspector_shows_the_components_of_the_selection() {
    let mut harness = harness();
    harness.get_by_label("Create").click();
    harness.run();
    for label in ["Name", "Transform", "Add Component"] {
        assert!(harness.query_all_by_label(label).next().is_some(), "inspector has no {label:?}");
    }
}
