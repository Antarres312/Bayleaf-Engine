use serde_json::json;

use crate::components::{PropertyShape, PropertyType, PropertyValue};
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
    assert_eq!(name("scripts/door_opener.bscript"), "Door Opener", "visual scripts lose their extension too");
}

#[test]
fn exported_subtrees_import_as_copies_with_new_ids_in_place() {
    let mut scene = Scene::new();
    let a = scene.create_object(None, "A").unwrap();
    let b = scene.create_object(None, "B").unwrap();
    let child = scene.create_object(Some(a), "Child").unwrap();
    scene.create_object(Some(child), "Grandchild").unwrap();
    scene.set_component(a, "transform", serde_json::json!({"position": [1, 2, 3], "rotation": [0, 0, 0], "scale": [1, 1, 1]})).unwrap();

    // A root inside another root is copied once.
    let copied = scene.export(&[a, child]).unwrap();
    assert_eq!(copied.len(), 3);
    let tops = scene.import(&copied, None, Some(1)).unwrap();
    assert_eq!(tops.len(), 1);
    let copy = tops[0];
    assert_ne!(copy, a, "copies get new ids");
    assert_eq!(scene.children(None), [a, copy, b], "placed at the index");
    assert_eq!(scene.component(copy, "transform").unwrap(), scene.component(a, "transform").unwrap());
    let copy_child = scene.children(Some(copy))[0];
    assert_eq!(scene.name(copy_child), Some("Child"));
    assert_eq!(scene.children(Some(copy_child)).len(), 1, "the whole subtree is copied");
    assert!(scene.is_inside(scene.children(Some(copy_child))[0], copy));

    // Broken data is refused.
    let mut orphan = copied.clone();
    orphan.remove(0);
    assert!(scene.import(&orphan, None, None).is_err(), "a child listed without its parent");
    let mut bad = copied;
    bad[0].components.insert("transform".into(), serde_json::json!({"position": "up"}));
    assert!(scene.import(&bad, None, None).is_err());
}

#[test]
fn an_inspector_layout_is_checked_against_the_declared_properties() {
    use crate::components::{DeclaredProperty, PropertyType, PropertyValue};
    use crate::inspector::{InspectorItem, parse};
    let declared = [
        DeclaredProperty {
            name: "speed".into(),
            ty: PropertyType::Number,
            shape: crate::components::PropertyShape::Single,
            default: PropertyValue::Number(1.0),
        },
        DeclaredProperty {
            name: "mode".into(),
            ty: PropertyType::WholeNumber,
            shape: crate::components::PropertyShape::Single,
            default: PropertyValue::Number(0.0),
        },
        DeclaredProperty {
            name: "label".into(),
            ty: PropertyType::Text,
            shape: crate::components::PropertyShape::Single,
            default: PropertyValue::Text(String::new()),
        },
    ];
    let layout = serde_json::json!([
        {"group": "Movement", "closed": true, "items": [
            {"property": "speed", "label": "Top speed", "hint": "Units a second", "range": [0, 20], "slider": true, "showIf": ["mode", 1]},
            {"button": "Jump", "message": "jump"}
        ]},
        {"property": "mode", "choices": ["Walk", "Run"]},
        {"note": "Careful", "warning": true}
    ]);
    let items = parse(&layout, &declared).unwrap();
    let InspectorItem::Group { closed: true, items: inner, .. } = &items[0] else { panic!("a closed group first: {items:?}") };
    let InspectorItem::Property(speed) = &inner[0] else { panic!("{inner:?}") };
    assert_eq!((speed.range, speed.slider), (Some((0.0, 20.0)), true));
    assert!(speed.show_if.as_ref().unwrap().holds(&PropertyValue::Number(1.0)));
    assert!(!speed.show_if.as_ref().unwrap().holds(&PropertyValue::Number(0.0)));

    for (bad, why) in [
        (serde_json::json!([{"property": "nope"}]), "does not declare"),
        (serde_json::json!([{"property": "speed"}, {"property": "speed"}]), "twice"),
        (serde_json::json!([{"property": "label", "range": [0, 1]}]), "a range is for numbers"),
        (serde_json::json!([{"property": "speed", "range": [5, 1]}]), "lower first"),
        (serde_json::json!([{"property": "speed", "slider": true}]), "needs a range"),
        (serde_json::json!([{"property": "speed", "choices": ["a"]}]), "choices are for"),
        (serde_json::json!([{"button": "Go"}]), "needs the message"),
        (serde_json::json!([{"note": "x", "group": "y"}]), "exactly one"),
        (serde_json::json!([{"note": "x", "slider": true}]), "does not go with"),
        (serde_json::json!([{"property": "speed", "showIf": ["mode", "fast"]}]), "is not a"),
        (
            serde_json::json!([{"group": "a", "items": [{"group": "b", "items": [{"group": "c", "items": [{"group": "d", "items": [{"group": "e"}]}]}]}]}]),
            "nest",
        ),
    ] {
        let error = parse(&bad, &declared).expect_err(why);
        assert!(error.contains(why), "{bad}: {error}");
    }
}

#[test]
fn post_processing_antialiasing_reads_old_on_and_off_and_the_new_choices() {
    use crate::post::{Antialiasing, PostProcessing};
    let file = |version: u32, value: &str| format!(r#"{{"format": "bayleaf.post", "version": {version}, "post": {{"antialiasing": {value}}}}}"#);
    assert_eq!(PostProcessing::from_text(&file(2, "true")).unwrap().antialiasing, Antialiasing::Fast, "on was the quick blend");
    assert_eq!(PostProcessing::from_text(&file(2, "false")).unwrap().antialiasing, Antialiasing::Off);
    assert_eq!(PostProcessing::from_text(&file(3, r#""fine""#)).unwrap().antialiasing, Antialiasing::Fine);
    assert!(PostProcessing::from_text(&file(3, r#""best""#)).is_err(), "only off, fast or fine");
    let written = PostProcessing { antialiasing: Antialiasing::Fine, ..PostProcessing::default() }.to_text();
    assert!(written.contains(r#""antialiasing": "fine""#), "{written}");
}

#[test]
fn list_and_map_script_properties_come_back_from_the_file_as_they_went() {
    use crate::components::{PropertyShape, PropertyType, PropertyValue, ScriptRef, Scripts};
    let mut scene = Scene::new();
    let id = scene.create_object(None, "Spawner").unwrap();
    // Three numbers in a list are a list, not a vector.
    let list = PropertyValue::List { list: [1.0, 2.0, 3.0].map(PropertyValue::Number).to_vec() };
    let map = PropertyValue::Map { map: [("easy".to_owned(), PropertyValue::Number(3.0))].into() };
    let script = ScriptRef { module: "waves.bscript".into(), properties: [("waves".to_owned(), list.clone()), ("by".to_owned(), map.clone())].into() };
    scene.set(id, Scripts(vec![script])).unwrap();
    let saved = serde_json::to_string(&scene.to_doc()).unwrap();
    let back = Scene::from_doc(&serde_json::from_str::<SceneDoc>(&saved).unwrap()).unwrap();
    let properties = &back.get::<Scripts>(id).unwrap().0[0].properties;
    assert_eq!(properties.get("waves"), Some(&list));
    assert_eq!(properties.get("by"), Some(&map));
    assert!(list.fits_shape(PropertyType::WholeNumber, PropertyShape::List) && !list.fits_shape(PropertyType::Vector, PropertyShape::Single));
}

#[test]
fn a_key_sets_its_object_s_transform_until_the_object_is_deleted() {
    use crate::components::Transform;
    let (mut scene, [a, _, _, b]) = tree();
    let key = scene.key(a).unwrap();
    let mark = scene.transform_mark();
    let moved = Transform { position: [3.0, 0.0, 0.0], ..Transform::default() };
    assert!(scene.set_transform(key, moved.clone()));
    assert_eq!(scene.get::<Transform>(a), Some(&moved));
    assert_eq!(scene.transforms_written_since(mark), Some(&[a][..]), "logged as a write");
    assert!(!scene.set_transform(key, Transform { position: [f64::NAN, 0.0, 0.0], ..Transform::default() }), "not finite");
    scene.delete_object(a).unwrap();
    // The deleted object's place may be another object's now; the key reaches neither.
    let c = scene.create_object(None, "C").unwrap();
    assert!(!scene.set_transform(key, moved.clone()));
    assert_ne!(scene.get::<Transform>(c), Some(&moved));
    assert_ne!(scene.get::<Transform>(b), Some(&moved));
}

#[test]
fn transform_writes_since_a_mark_are_listed_and_a_mark_too_old_says_so() {
    let (mut scene, [a, child, _, b]) = tree();
    let mark = scene.transform_mark();
    let moved = crate::components::Transform { position: [1.0, 2.0, 3.0], rotation: [0.0; 3], scale: [1.0; 3] };
    scene.set(a, moved.clone()).unwrap();
    scene.set_component(b, "transform", json!({"position": [4, 5, 6], "rotation": [0, 0, 0], "scale": [1, 1, 1]})).unwrap();
    scene.set(child, crate::components::Name("Renamed".to_owned())).unwrap();
    assert_eq!(scene.transforms_written_since(mark), Some(&[a, b][..]), "the moved objects, not the renamed one");
    assert_eq!(scene.transforms_written_since(scene.transform_mark()), Some(&[][..]), "nothing since now");
    // Many more writes than objects: the log starts again, and the first mark no longer reaches.
    for _ in 0..5000 {
        scene.set(a, moved.clone()).unwrap();
    }
    assert_eq!(scene.transforms_written_since(mark), None);
}

#[test]
fn data_files_read_their_values_where_they_fit_and_the_shape_defaults_elsewhere() {
    use crate::data::{DataField, DataFile, DataShape};
    let mut health = DataField::new("health", PropertyType::Number, PropertyShape::Single);
    health.default = PropertyValue::Number(100.0);
    let mut waves = DataField::new("waves", PropertyType::WholeNumber, PropertyShape::List);
    waves.default = PropertyValue::List { list: vec![PropertyValue::Number(3.0)] };
    let shape = DataShape { fields: vec![health, waves, DataField::new("drop", PropertyType::Prefab, PropertyShape::Single)] };
    shape.validate().unwrap();
    let mut data = DataFile::new("data/enemy.bshape");
    data.values.insert("health".to_owned(), PropertyValue::Number(40.0));
    data.values.insert("drop".to_owned(), PropertyValue::Text("coin.bsound".to_owned()));
    data.values.insert("gone".to_owned(), PropertyValue::Flag(true));
    let read = data.resolved(&shape);
    assert_eq!(read["health"].value, PropertyValue::Number(40.0), "its own value");
    assert_eq!(read["waves"].value, PropertyValue::List { list: vec![PropertyValue::Number(3.0)] }, "unset: the default");
    assert_eq!(read["drop"].value, PropertyValue::Text(String::new()), "a path of the wrong kind reads as the default");
    assert!(!read.contains_key("gone"), "only the shape's fields are read");
    assert_eq!(data.problems(&shape).len(), 2, "the misfit and the unknown value are reported: {:?}", data.problems(&shape));
    // The files read back as written.
    assert_eq!(DataShape::from_text(&shape.to_text()).unwrap(), shape);
    assert_eq!(DataFile::from_text(&data.to_text()).unwrap(), data);
}

#[test]
fn shapes_refuse_fields_that_cannot_be_read_and_lay_groups_out_together() {
    use crate::data::{DataField, DataShape};
    use crate::inspector::InspectorItem;
    let field = |name: &str| DataField::new(name, PropertyType::Number, PropertyShape::Single);
    assert!(DataShape { fields: vec![field("a"), field("a")] }.validate().is_err(), "two fields of one name");
    let mut bad = field("a");
    bad.default = PropertyValue::Text("x".to_owned());
    assert!(DataShape { fields: vec![bad] }.validate().is_err(), "a default of another type");
    let mut ranged = DataField::new("name", PropertyType::Text, PropertyShape::Single);
    ranged.range = Some([0.0, 1.0]);
    assert!(DataShape { fields: vec![ranged] }.validate().is_err(), "a range on text");
    let mut unknown = field("a");
    unknown.code = "lists:number".to_owned();
    assert!(DataShape { fields: vec![unknown] }.validate().is_err(), "a type code that names nothing");

    let grouped = |name: &str| DataField { group: Some("Combat".to_owned()), ..field(name) };
    let shape = DataShape { fields: vec![grouped("damage"), field("speed"), grouped("range")] };
    let layout = shape.layout();
    assert_eq!(layout.len(), 2, "the group's fields show together where its first is");
    assert!(matches!(&layout[0], InspectorItem::Group { items, .. } if items.len() == 2));
}

#[test]
fn file_properties_hold_paths_of_their_kind_or_nothing() {
    let path = |t: &str| PropertyValue::Text(t.to_owned());
    assert!(path("").fits(PropertyType::Data), "none");
    assert!(path("data/Goblin.BDATA").fits(PropertyType::Data), "the extension in any case");
    assert!(!path("data/goblin.bshape").fits(PropertyType::Data));
    assert!(path("sounds/hit.ogg").fits(PropertyType::Sound) && !path("sounds/hit.ogg").fits(PropertyType::Prefab));
    assert!(!path(&format!("{}.bprefab", "a".repeat(600))).fits(PropertyType::Prefab), "too long a path");
    for ty in [PropertyType::Prefab, PropertyType::Sound, PropertyType::Material, PropertyType::Data] {
        assert_eq!(PropertyType::from_code(ty.code()), Some(ty), "{} reads back", ty.code());
        assert!(ty.zero().fits(ty), "{}'s none fits", ty.code());
    }
}
