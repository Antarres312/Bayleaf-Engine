use std::path::PathBuf;

use bayleaf_core::StableId;
use bayleaf_scene::components::Transform;
use serde_json::{Value, json};

use crate::{CommandRegistry, Session};

/// A scene saved in a fresh folder, with a Lamp (and its Bulb child) saved as `lamp.bprefab`.
struct Fixture {
    registry: CommandRegistry,
    session: Session,
    dir: PathBuf,
    lamp: StableId,
    bulb: StableId,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("bayleaf-timeline-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let registry = CommandRegistry::with_builtins();
        let mut session = Session::default();
        registry.execute(&mut session, "scene.saveAs", &json!({"path": dir.join("main.bscene")})).unwrap();
        let id = |v: Value| StableId::parse_hex(v["id"].as_str().unwrap()).unwrap();
        let lamp = id(registry.execute(&mut session, "scene.object.create", &json!({"name": "Lamp"})).unwrap());
        let bulb = id(registry.execute(&mut session, "scene.object.create", &json!({"name": "Bulb", "parent": lamp})).unwrap());
        registry.execute(&mut session, "scene.component.add", &json!({"id": bulb, "component": "pointLight"})).unwrap();
        registry.execute(&mut session, "scene.prefab.create", &json!({"id": lamp, "path": "lamp.bprefab"})).unwrap();
        Self { registry, session, dir, lamp, bulb }
    }

    fn run(&mut self, id: &str, params: Value) -> Value {
        self.registry.execute(&mut self.session, id, &params).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
    }

    fn position(&self, id: StableId) -> [f64; 3] {
        self.session.scene().get::<Transform>(id).map(|t| t.position).unwrap()
    }

    fn keys(&self, part: StableId, component: &str, field: &str) -> Vec<f32> {
        let target = bayleaf_scene::timeline::Target::field(component, field);
        let doc = self.session.timeline().unwrap();
        doc.timeline.part(part).and_then(|p| p.track(&target)).map(|t| t.keys.iter().map(|k| k.time).collect()).unwrap_or_default()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

const POSITION: fn() -> Value = || json!({"component": "transform", "field": "position"});

#[test]
fn an_agent_builds_a_timeline_and_the_playhead_poses_a_placed_copy_without_changing_the_scene() {
    let mut f = Fixture::new("build");
    let path = f.dir.join("bob.btimeline");
    f.run("timeline.new", json!({"prefab": "lamp.bprefab", "length": 2.0, "path": path}));
    assert!(path.exists(), "the file is written at once");
    // The parts are the prefab file's objects; the scene's object is a copy linked to it.
    let file: Value = serde_json::from_str(&std::fs::read_to_string(f.dir.join("lamp.bprefab")).unwrap()).unwrap();
    let file_bulb = StableId::parse_hex(file["objects"][1]["id"].as_str().unwrap()).unwrap();
    f.run("timeline.key.set", json!({"part": file_bulb, "track": POSITION(), "time": 0.0, "value": [0, 0, 0], "ease": "linear"}));
    f.run("timeline.key.set", json!({"part": file_bulb, "track": POSITION(), "time": 2.0, "value": [0, 4, 0]}));
    let history = f.session.history().len();
    f.run("timeline.key.set", json!({"part": file_bulb, "track": {"component": "pointLight", "field": "intensity"}, "time": 1.0, "value": 7.0}));
    assert_eq!(f.session.history().len(), history + 1, "each edit is one undo step");

    let dirty = f.session.is_dirty();
    let rest = f.position(f.bulb);
    let posed = f.run("timeline.seek", json!({"time": 1.0}));
    assert_eq!(posed["problems"], json!([]));
    let mid = f.position(f.bulb);
    assert!(mid[1] > rest[1] && mid[1] < 4.0, "halfway between the keys: {mid:?}");
    assert_eq!(f.session.is_dirty(), dirty, "the pose does not dirty the scene");
    assert_eq!(f.session.history().len(), history + 1, "nor make undo steps");

    // Anything else sees the scene as it is.
    f.run("scene.object.create", json!({"name": "Other"}));
    assert_eq!(f.position(f.bulb), rest);
    assert!(!f.session.timeline().unwrap().is_posed());

    f.run("timeline.save", json!({}));
    let saved = bayleaf_scene::timeline::Timeline::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved.part(file_bulb).map(|p| p.tracks.len()), Some(2));
    assert!(f.session.timeline().is_some_and(|d| !d.dirty));
    let _ = f.lamp;
}

#[test]
fn keys_move_copy_paste_and_delete_as_single_undo_steps() {
    let mut f = Fixture::new("keys");
    f.run("prefab.open", json!({"path": "lamp.bprefab"}));
    f.run("timeline.new", json!({"prefab": "lamp.bprefab", "length": 3.0}));
    let lamp = f.session.scene().children(None)[0];
    assert_eq!(lamp, f.lamp, "an open prefab keeps the ids of its file");
    for t in [0.0, 1.0] {
        f.run("timeline.key.set", json!({"part": lamp, "track": POSITION(), "time": t, "value": [t, 0, 0]}));
    }
    let key = |t: f32| json!({"part": lamp, "track": POSITION(), "time": t});
    let moved = f.run("timeline.key.move", json!({"keys": [key(1.0)], "by": 0.5}));
    assert_eq!(moved["keys"][0]["time"], json!(1.5));
    assert_eq!(f.keys(lamp, "transform", "position"), [0.0, 1.5]);
    f.run("timeline.key.move", json!({"keys": [key(1.5)], "by": 10.0}));
    assert_eq!(f.keys(lamp, "transform", "position"), [0.0, 3.0], "a key stays inside the length");

    let text = f.run("timeline.key.copy", json!({"keys": [key(0.0), key(3.0)]}))["text"].as_str().unwrap().to_owned();
    f.run("timeline.key.paste", json!({"text": text, "time": 0.5, "part": lamp, "track": {"component": "transform", "field": "scale"}}));
    assert_eq!(f.keys(lamp, "transform", "scale"), [0.5, 3.0], "the earliest copied key lands at the time; later ones stay inside the length");

    f.run("timeline.key.remove", json!({"keys": [key(0.0)]}));
    assert_eq!(f.keys(lamp, "transform", "position"), [3.0]);
    f.run("edit.undo", json!(null));
    assert_eq!(f.keys(lamp, "transform", "position"), [0.0, 3.0], "undo brings the key back");
    f.run("edit.redo", json!(null));
    assert_eq!(f.keys(lamp, "transform", "position"), [3.0]);
    assert!(f.registry.execute(&mut f.session, "timeline.key.remove", &json!({"keys": [key(0.0)]})).is_err(), "a key that is not there");
}

#[test]
fn recording_turns_a_moved_part_into_a_key_at_the_playhead() {
    let mut f = Fixture::new("record");
    f.run("prefab.open", json!({"path": "lamp.bprefab"}));
    f.run("timeline.new", json!({"prefab": "lamp.bprefab", "length": 2.0}));
    f.run("timeline.seek", json!({"time": 1.0}));
    f.run("timeline.record", json!({"on": true}));
    let moved = json!({"position": [0, 3, 0], "rotation": [0, 0, 0], "scale": [1, 1, 1]});
    f.run("scene.component.set", json!({"id": f.bulb, "component": "transform", "value": moved}));
    assert_eq!(f.keys(f.bulb, "transform", "position"), [1.0], "only the changed field is keyed");
    assert!(f.keys(f.bulb, "transform", "scale").is_empty());
    assert_eq!(f.position(f.bulb), [0.0, 3.0, 0.0], "the pose shows the key");
    assert!(!f.session.is_dirty(), "the prefab itself is unchanged");
    // A drag merges into one step, as for scene edits.
    for y in [4, 5] {
        let value = json!({"position": [0, y, 0], "rotation": [0, 0, 0], "scale": [1, 1, 1]});
        f.registry
            .execute_merging(&mut f.session, "scene.component.set", &json!({"id": f.bulb, "component": "transform", "value": value}), Some("drag"))
            .unwrap();
    }
    let steps = f.session.history().len();
    f.run("edit.undo", json!(null));
    assert_eq!(f.session.history().len(), steps - 1);
    assert_eq!(f.position(f.bulb), [0.0, 3.0, 0.0], "undo goes back over the whole drag, and the pose follows");

    f.run("timeline.record", json!({"on": false}));
    f.run("scene.component.set", json!({"id": f.bulb, "component": "pointLight", "value": {"color": [1, 0, 0], "intensity": 2, "range": 5, "shadows": false}}));
    assert!(f.session.is_dirty(), "without recording, edits change the prefab");
    assert!(f.keys(f.bulb, "pointLight", "color").is_empty());
}

#[test]
fn shortening_drops_later_keys_and_a_once_or_looping_choice_is_kept() {
    let mut f = Fixture::new("length");
    f.run("timeline.new", json!({"prefab": "lamp.bprefab", "length": 4.0}));
    let lamp = f.lamp;
    let file: Value = serde_json::from_str(&std::fs::read_to_string(f.dir.join("lamp.bprefab")).unwrap()).unwrap();
    let file_lamp = StableId::parse_hex(file["objects"][0]["id"].as_str().unwrap()).unwrap();
    assert_eq!(file_lamp, lamp, "the object saved as a prefab keeps its ids");
    for t in [0.0, 2.0, 4.0] {
        f.run("timeline.key.set", json!({"part": lamp, "track": POSITION(), "time": t, "value": [t, 0, 0]}));
    }
    assert_eq!(f.run("timeline.set", json!({"length": 3.0, "looping": true}))["dropped"], json!(1));
    let doc = f.session.timeline().unwrap();
    assert!(doc.timeline.looping && doc.timeline.length == 3.0);
    assert!(f.registry.execute(&mut f.session, "timeline.key.set", &json!({"part": lamp, "track": POSITION(), "time": 3.5, "value": [0, 0, 0]})).is_err());
    let stranger = StableId::from_u128(42);
    assert!(f.registry.execute(&mut f.session, "timeline.part.add", &json!({"part": stranger})).is_err(), "only the prefab's own objects take part");
}

#[test]
fn hostile_timeline_params_change_nothing() {
    let mut f = Fixture::new("hostile");
    f.run("timeline.new", json!({"prefab": "lamp.bprefab"}));
    f.run("timeline.key.set", json!({"part": f.lamp, "track": POSITION(), "time": 0.0, "value": [0, 0, 0]}));
    let lamp = f.lamp;
    let hostile = [
        json!({"part": lamp, "track": POSITION(), "time": f32::MAX, "value": [0, 0, 0]}),
        json!({"part": lamp, "track": POSITION(), "time": -1.0}),
        json!({"part": lamp, "track": POSITION(), "value": {"x": 1}}),
        json!({"part": lamp, "track": POSITION(), "value": vec![1; 1000]}),
        json!({"part": lamp, "track": {"component": "name", "field": "x"}, "value": 1}),
        json!({"part": lamp, "track": {"component": "", "field": ""}, "value": 1}),
        json!({"part": lamp, "track": {"script": "a.wasm"}, "value": 1}),
        json!({"part": "0".repeat(32), "track": POSITION(), "value": [0, 0, 0]}),
        json!({"keys": [{"part": lamp, "track": POSITION(), "time": 9.0}], "by": 1.0}),
        json!({"keys": [{"part": lamp, "track": POSITION(), "time": 0.0}], "by": f32::NAN}),
        json!({"text": "{\"format\": \"bayleaf.scene\"}"}),
        json!({"text": "not json"}),
        json!({"length": -1.0}),
        json!({"length": 1e30}),
        json!({"path": "../../escape.btimeline"}),
        json!({"path": "wrong.bscene"}),
        json!({"prefab": "missing.bprefab"}),
        json!({"id": lamp, "component": "name", "value": "x"}),
        json!({"id": lamp, "index": 3, "name": "speed", "value": 1}),
        json!({"time": 1e9}),
    ];
    let ids: Vec<_> = f
        .registry
        .list(&f.session)
        .into_iter()
        .map(|c| c.id)
        .filter(|id| id.starts_with("timeline.") && !matches!(*id, "timeline.close" | "timeline.new" | "timeline.save"))
        .collect();
    for id in &ids {
        for bad in &hostile {
            let before = (f.session.timeline().cloned(), f.session.scene().to_doc());
            if f.registry.execute(&mut f.session, id, bad).is_err() {
                assert_eq!((f.session.timeline().cloned(), f.session.scene().to_doc()), before, "{id} changed something on failure with {bad}");
            }
        }
    }
    assert!(!f.dir.parent().unwrap().join("escape.btimeline").exists());
}

#[test]
fn a_confined_session_refuses_timeline_files_outside_its_roots() {
    let mut f = Fixture::new("confine");
    let outside = std::env::temp_dir().join(format!("bayleaf-timeline-outside-{}", std::process::id()));
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("far.btimeline"), bayleaf_scene::timeline::Timeline::new("lamp.bprefab", 1.0).to_json()).unwrap();
    let inside = f.dir.clone();
    f.session.set_confinement(Some(crate::confine::Confinement::new(std::slice::from_ref(&inside), std::slice::from_ref(&inside)).unwrap()));

    let refused = f.registry.execute(&mut f.session, "timeline.open", &json!({"path": outside.join("far.btimeline")})).unwrap_err();
    assert!(refused.to_string().contains("outside the allowed folders"), "{refused}");
    assert!(f.registry.execute(&mut f.session, "timeline.new", &json!({"prefab": "lamp.bprefab", "path": outside.join("new.btimeline")})).is_err());
    assert!(!outside.join("new.btimeline").exists());
    f.run("timeline.new", json!({"prefab": "lamp.bprefab"}));
    assert!(f.registry.execute(&mut f.session, "timeline.save", &json!({"path": outside.join("saved.btimeline")})).is_err());
    assert!(!outside.join("saved.btimeline").exists());
    f.run("timeline.save", json!({"path": "kept.btimeline"}));
    assert!(inside.join("kept.btimeline").exists(), "inside the roots it is written");
    std::fs::remove_dir_all(&outside).ok();
}

