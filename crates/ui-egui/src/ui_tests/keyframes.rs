//! The Keyframes panel, driven by keyboard and through the accessibility tree.

use bayleaf_engine::scene::timeline::{Ease, Target};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::harness;
use crate::Editor;

/// An editor with a Lamp prefab (and its Bulb child) open on its own, and a keyframe animation made
/// for it from the panel. Returns the folder, to remove afterwards.
fn lamp(name: &str) -> (Harness<'static, Editor>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("bayleaf-ui-keyframes-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let mut harness = harness();
    let editor = harness.state_mut();
    editor.dispatch("scene.saveAs", &json!({ "path": dir.join("s.bscene") }), None);
    editor.dispatch("scene.object.create", &json!({ "name": "Lamp" }), None);
    let lamp = editor.selected.unwrap();
    editor.dispatch("scene.object.create", &json!({ "name": "Bulb", "parent": lamp }), None);
    editor.dispatch("scene.prefab.create", &json!({ "id": lamp, "path": "lamp.bprefab" }), None);
    editor.dispatch("prefab.open", &json!({ "path": "lamp.bprefab" }), None);
    crate::timeline_view::new_for_prefab(editor, &dir.join("lamp.bprefab"));
    harness.run();
    assert!(dir.join("lamp.btimeline").exists(), "the panel makes the file beside the prefab");
    (harness, dir)
}

fn key_times(harness: &Harness<'static, Editor>) -> Vec<f32> {
    let doc = harness.state().session.timeline().unwrap();
    let position = Target::field("transform", "position");
    doc.timeline.parts.iter().filter_map(|p| p.track(&position)).flat_map(|t| t.keys.iter().map(|k| k.time)).collect()
}

fn press(harness: &mut Harness<'static, Editor>, modifiers: egui::Modifiers, key: egui::Key) {
    harness.key_press_modifiers(modifiers, key);
    harness.run();
}

#[test]
fn a_keyframe_animation_is_built_by_keyboard_and_each_edit_undoes() {
    let (mut harness, dir) = lamp("keys");
    harness.get_by_label("Add Part").click();
    harness.run();
    // The menu's entry comes before the hierarchy's row of the same name.
    harness.query_all_by_label("Lamp").next().unwrap().click();
    harness.run();
    harness.get_by_label("Add Track to Lamp").click();
    harness.run();
    harness.get_by_label("Position").click();
    harness.run();
    let none = egui::Modifiers::NONE;
    harness.get_by_label("Position, 0 keys").focus();
    harness.run();
    press(&mut harness, none, egui::Key::K);
    assert_eq!(key_times(&harness), [0.0], "K places a key at the playhead");

    // Up to the ruler, the playhead on by half a second, back down to the track.
    press(&mut harness, none, egui::Key::ArrowUp);
    assert!(harness.get_by_label("Playhead").is_focused(), "Up goes from the first track to the ruler");
    for _ in 0..5 {
        press(&mut harness, none, egui::Key::ArrowRight);
    }
    let playhead = harness.state().session.timeline().unwrap().playhead;
    assert!((playhead - 0.5).abs() < 1e-4, "the ruler's arrows move the playhead: {playhead}");
    harness.get_by_label("Position, 1 key").focus();
    harness.run();
    press(&mut harness, none, egui::Key::Enter);
    assert_eq!(key_times(&harness), [0.0, 0.5]);

    // Onto the key at the playhead, nudged later, then made to step.
    press(&mut harness, none, egui::Key::ArrowRight);
    assert!(harness.get_by_label("Key of Position at 0.50 seconds, Smooth").is_focused(), "Right goes to the key at the playhead");
    press(&mut harness, egui::Modifiers::ALT, egui::Key::ArrowRight);
    let times = key_times(&harness);
    assert!(times[1] > 0.5 && times[1] < 0.7, "Alt+Right nudges the key later: {times:?}");
    assert!(harness.query_all_by_label_contains("Key of Position at 0.60 seconds").next().is_some_and(|k| k.is_focused()), "the focus follows the key");
    press(&mut harness, none, egui::Key::E);
    let eases: Vec<Ease> = harness.state().session.timeline().unwrap().timeline.parts[0].tracks[0].keys.iter().map(|k| k.ease).collect();
    assert_eq!(eases[1], Ease::Step, "E changes how it eases: {eases:?}");
    assert!(harness.query_all_by_label_contains("Eases: Step").next().is_some(), "and says so");

    press(&mut harness, none, egui::Key::Delete);
    assert_eq!(key_times(&harness), [0.0], "Delete deletes the key");
    press(&mut harness, egui::Modifiers::COMMAND, egui::Key::Z);
    assert_eq!(key_times(&harness).len(), 2, "Ctrl+Z brings it back in one step");
    assert!(harness.state().session.timeline().unwrap().dirty);
    harness.get_by_label("Position, 2 keys").focus();
    harness.run();
    press(&mut harness, egui::Modifiers::COMMAND, egui::Key::S);
    assert!(!harness.state().session.timeline().unwrap().dirty, "Ctrl+S in the panel saves the animation");
    assert!(harness.state().session.open_prefabs().len() == 1 && !harness.state().session.is_dirty(), "and not the prefab");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn recording_keys_what_the_scene_view_moves_and_the_ruler_shows_the_pose() {
    let (mut harness, dir) = lamp("record");
    harness.get_by_label("Record").click();
    harness.run();
    assert!(harness.state().session.timeline().unwrap().recording);
    let bulb = harness.state().session.scene().children(harness.state().session.scene().children(None).first().copied())[0];
    // What the scene view's handles send while a part is dragged.
    let moved = json!({ "position": [0, 2, 0], "rotation": [0, 0, 0], "scale": [1, 1, 1] });
    harness.state_mut().dispatch("scene.component.set", &json!({ "id": bulb, "component": "transform", "value": moved }), Some("drag"));
    harness.run();
    assert_eq!(key_times(&harness), [0.0], "the move became a key");
    assert!(harness.query_by_label("Position, 1 key").is_some(), "its track shows in the panel");
    assert!(!harness.state().session.is_dirty(), "the prefab itself did not change");
    std::fs::remove_dir_all(&dir).ok();
}
