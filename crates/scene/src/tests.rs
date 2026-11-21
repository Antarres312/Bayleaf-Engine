use serde_json::json;

use crate::doc::{self, SceneDoc};
use crate::{Scene, SceneError};

fn tree() -> (Scene, [bayleaf_core::StableId; 4]) {
    // root_a { child { grandchild } }, root_b
    let mut scene = Scene::new();
    let root_a = scene.create_object(None, "A").unwrap();
    let child = scene.create_object(Some(root_a), "Child").unwrap();
    let grandchild = scene.create_object(Some(child), "Grandchild").unwrap();
    let root_b = scene.create_object(None, "B").unwrap();
    (scene, [root_a, child, grandchild, root_b])
}

#[test]
fn new_objects_have_the_required_components() {
    let (scene, [a, ..]) = tree();
    let names: Vec<_> = scene.components(a).unwrap().into_iter().map(|(n, _)| n).collect();
    assert!(names.contains(&"name") && names.contains(&"transform"));
    assert_eq!(scene.name(a), Some("A"));
}

#[test]
fn deleting_an_object_deletes_its_descendants_and_their_entities() {
    let (mut scene, [a, child, grandchild, b]) = tree();
    assert_eq!(scene.delete_object(a).unwrap(), 3);
    assert!(!scene.contains(child) && !scene.contains(grandchild));
    assert_eq!(scene.children(None), &[b]);
    assert_eq!(scene.world().len(), 1);
}

#[test]
fn reparenting_under_a_descendant_is_refused_and_changes_nothing() {
    let (mut scene, [a, _, grandchild, _]) = tree();
    let before = scene.to_doc();
    assert!(matches!(scene.reparent(a, Some(grandchild), None), Err(SceneError::Cycle)));
    assert!(matches!(scene.reparent(a, Some(a), None), Err(SceneError::Cycle)));
    assert_eq!(scene.to_doc(), before);
}

#[test]
fn reparent_places_the_object_at_the_requested_index() {
    let (mut scene, [a, child, _, b]) = tree();
    scene.reparent(b, Some(a), Some(0)).unwrap();
    assert_eq!(scene.children(Some(a)), &[b, child]);
    assert_eq!(scene.parent(b), Some(a));
    scene.reparent(b, None, Some(99)).unwrap();
    assert_eq!(scene.children(None), &[a, b]);
}

#[test]
fn invalid_component_values_are_refused() {
    let (mut scene, [a, ..]) = tree();
    let before = scene.component(a, "transform").unwrap();
    assert!(scene.set_component(a, "transform", json!({"position": "up"})).is_err());
    assert!(scene.set_component(a, "transform", json!({"position": [0, 0, 0], "rotation": [0, 0, 0], "scale": [1e300, 1, 1]})).is_err());
    assert!(scene.set_component(a, "nothing", json!(1)).is_err());
    assert_eq!(scene.component(a, "transform").unwrap(), before);
}

#[test]
fn required_components_cannot_be_removed_but_optional_ones_can() {
    let (mut scene, [a, ..]) = tree();
    assert!(matches!(scene.remove_component(a, "transform"), Err(SceneError::NotRemovable(_))));
    scene.add_component(a, "tags").unwrap();
    assert!(scene.add_component(a, "tags").is_err());
    scene.remove_component(a, "tags").unwrap();
    assert!(scene.component(a, "tags").is_err());
}

#[test]
fn save_and_load_round_trip_keeps_hierarchy_order_and_values() {
    let (mut scene, [a, child, _, b]) = tree();
    scene.reparent(b, Some(a), Some(0)).unwrap();
    scene.set_component(child, "transform", json!({"position": [1.5, 2.0, -3.0], "rotation": [0, 90, 0], "scale": [2, 2, 2]})).unwrap();
    scene.add_component(child, "tags").unwrap();
    scene.set_component(child, "tags", json!(["enemy"])).unwrap();
    let dir = std::env::temp_dir().join(format!("bayleaf-scene-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("round-trip.{}", doc::EXTENSION));
    scene.save(&path).unwrap();
    let loaded = Scene::load(&path).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(loaded.to_doc(), scene.to_doc());
}

#[test]
fn world_matrices_compose_parent_transforms() {
    let (mut scene, [a, child, grandchild, _]) = tree();
    scene.set_component(a, "transform", json!({"position": [10, 0, 0], "rotation": [0, 0, 0], "scale": [2, 2, 2]})).unwrap();
    scene.set_component(child, "transform", json!({"position": [1, 0, 0], "rotation": [0, 0, 0], "scale": [1, 1, 1]})).unwrap();
    let all = scene.world_matrices();
    let from_list = all.iter().find(|(id, _)| *id == grandchild).unwrap().1;
    let direct = scene.world_matrix(grandchild).unwrap();
    assert!(from_list.abs_diff_eq(direct, 1e-5));
    // The parent's scale stretches the child's offset: 10 + 2 * 1.
    assert!((direct.w_axis.x - 12.0).abs() < 1e-5);
    // Parents come before children.
    let pos = |id| all.iter().position(|(o, _)| *o == id).unwrap();
    assert!(pos(a) < pos(child) && pos(child) < pos(grandchild));
}

#[test]
fn reparenting_can_keep_the_world_placement() {
    let (mut scene, [a, _, _, b]) = tree();
    scene.set_component(a, "transform", json!({"position": [3, 1, 0], "rotation": [0, 90, 0], "scale": [2, 2, 2]})).unwrap();
    scene.set_component(b, "transform", json!({"position": [5, 0.5, -2], "rotation": [10, 0, 0], "scale": [1, 1, 1]})).unwrap();
    let before = scene.world_matrix(b).unwrap();
    scene.reparent_keeping_world(b, Some(a), None).unwrap();
    assert!(scene.world_matrix(b).unwrap().abs_diff_eq(before, 1e-3));
    scene.reparent_keeping_world(b, None, None).unwrap();
    assert!(scene.world_matrix(b).unwrap().abs_diff_eq(before, 1e-3));
}

#[test]
fn render_components_validate_their_ranges() {
    let (mut scene, [a, ..]) = tree();
    for (component, bad) in [
        ("mesh", json!({"shape": "teapot", "color": [1, 1, 1, 1], "metallic": 0, "roughness": 0.5})),
        ("mesh", json!({"shape": "cube", "color": [2, 1, 1, 1], "metallic": 0, "roughness": 0.5})),
        ("camera", json!({"projection": "perspective", "fov": 0, "size": 5, "near": 0.1, "far": 100})),
        ("camera", json!({"projection": "perspective", "fov": 60, "size": 5, "near": 10, "far": 1})),
        ("directionalLight", json!({"color": [1, 1, 1], "intensity": -1})),
        ("sprite", json!({"color": [1, 1, 1, 1], "size": [0, 1]})),
    ] {
        scene.add_component(a, component).ok();
        assert!(scene.set_component(a, component, bad.clone()).is_err(), "{component} accepted {bad}");
    }
}

#[test]
fn loading_rejects_broken_documents() {
    let (scene, _) = tree();
    let good = scene.to_doc();

    let mut wrong_format = good.clone();
    wrong_format.format = "something.else".into();
    let mut child_first = good.clone();
    child_first.objects.swap(0, 1);
    let mut duplicate = good.clone();
    duplicate.objects.push(good.objects[0].clone());
    let mut unknown_component = good.clone();
    unknown_component.objects[0].components.insert("mystery".into(), json!(1));

    for (what, broken) in [("format", wrong_format), ("order", child_first), ("duplicate", duplicate), ("component", unknown_component)] {
        assert!(Scene::from_doc(&broken).is_err(), "accepted a document with a bad {what}");
    }
    let _: SceneDoc = good;
}

#[test]
fn version_1_scenes_load_with_their_script_moved_into_the_list() {
    let v1 = json!({
        "format": "bayleaf.scene",
        "version": 1,
        "objects": [{"id": "0123456789abcdef0123456789abcdef", "parent": null, "components": {
            "name": "Old", "script": {"module": "scripts/spinner.wasm", "properties": {"speed": 60}}
        }}]
    });
    let doc: SceneDoc = serde_json::from_value(v1).unwrap();
    let scene = Scene::from_doc(&doc).unwrap();
    let id = scene.children(None)[0];
    let scripts = scene.get::<crate::components::Scripts>(id).unwrap();
    assert_eq!(scripts.0.len(), 1);
    assert_eq!(scripts.0[0].display_name(), "Spinner");
    assert_eq!(scene.to_doc().version, doc::VERSION);
}

#[test]
fn script_names_come_from_the_module_file() {
    let name = |module: &str| crate::components::ScriptRef { module: module.into(), properties: Default::default() }.display_name();
    assert_eq!(name("scripts/player_controller.wasm"), "Player Controller");
    assert_eq!(name(r"a\b\mover.wasm"), "Mover");
}
