use serde_json::{Value, json};

use crate::{CommandError, CommandRegistry, Session};

fn create(registry: &CommandRegistry, session: &mut Session, params: Value) -> String {
    let result = registry.execute(session, "scene.object.create", &params).unwrap();
    result["id"].as_str().unwrap().to_owned()
}

#[test]
fn command_ids_are_unique() {
    let registry = CommandRegistry::with_builtins();
    let mut ids: Vec<_> = registry.list(&Session::default()).into_iter().map(|c| c.id).collect();
    let count = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), count);
}

/// Hostile-input sweep: every command, every hostile params value. Nothing may panic, nothing
/// hostile may succeed in changing the scene.
#[test]
fn hostile_params_are_refused_for_every_command() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let real = create(&registry, &mut session, json!({"name": "Target"}));
    let ids: Vec<_> = registry.list(&session).into_iter().map(|c| c.id).collect();
    for id in &ids {
        for bad in [json!(1), json!("x"), json!([1, 2]), json!(true)] {
            assert!(matches!(registry.execute(&mut session, id, &bad), Err(CommandError::ParamsNotObject)), "{id} accepted {bad}");
        }
    }
    let hostile_objects = [
        json!({"id": 5, "path": [], "parent": {}, "component": 3}),
        json!({"id": "not-hex", "component": "transform", "value": null}),
        json!({"id": "0".repeat(32), "component": "transform", "value": {}}),
        json!({"id": real, "component": "transform", "value": {"position": "up"}}),
        json!({"id": real, "component": "../../etc", "value": 1}),
        json!({"id": real, "parent": real}),
        json!({"id": real, "index": -1}),
        json!({"unexpected": true}),
    ];
    for id in ids.iter().filter(|id| !matches!(**id, "scene.new" | "edit.undo" | "edit.redo")) {
        for bad in &hostile_objects {
            let before = session.scene().to_doc();
            let result = registry.execute(&mut session, id, bad);
            if result.is_err() {
                assert_eq!(session.scene().to_doc(), before, "{id} changed the scene on failure with {bad}");
            }
        }
    }
    let real = bayleaf_core::StableId::parse_hex(&real).unwrap();
    assert!(session.scene().component(real, "transform").is_ok());
}

#[test]
fn unknown_command_is_an_error() {
    let registry = CommandRegistry::with_builtins();
    assert!(matches!(registry.execute(&mut Session::default(), "no.suchCommand", &json!({})), Err(CommandError::Unknown(_))));
}

#[test]
fn undo_and_redo_walk_the_history() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let empty = session.scene().to_doc();
    create(&registry, &mut session, json!({"name": "A"}));
    let with_a = session.scene().to_doc();
    registry.execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert_eq!(session.scene().to_doc(), empty);
    registry.execute(&mut session, "edit.redo", &Value::Null).unwrap();
    assert_eq!(session.scene().to_doc(), with_a);
    assert!(registry.execute(&mut session, "edit.redo", &Value::Null).is_err());
}

#[test]
fn the_history_keeps_the_newest_steps_up_to_its_limit() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let id = create(&registry, &mut session, json!({"name": "Moved"}));
    let extra = 10;
    for x in 0..crate::MAX_UNDO_STEPS + extra {
        let params = json!({"id": id, "component": "transform", "value": {"position": [x, 0, 0], "rotation": [0, 0, 0], "scale": [1, 1, 1]}});
        registry.execute(&mut session, "scene.component.set", &params).unwrap();
    }
    assert!(session.history_bytes() <= crate::UNDO_BUDGET_BYTES);
    let mut undone = 0;
    while registry.execute(&mut session, "edit.undo", &Value::Null).is_ok() {
        undone += 1;
    }
    assert_eq!(undone, crate::MAX_UNDO_STEPS, "the oldest steps were dropped");
    let x = session.scene().get::<bayleaf_scene::components::Transform>(bayleaf_core::StableId::parse_hex(&id).unwrap()).unwrap().position[0];
    assert!(x > 0.0, "undo stops at the oldest kept step, not at the start: x = {x}");
    registry.execute(&mut session, "edit.redo", &Value::Null).unwrap();
}

#[test]
fn a_new_change_clears_redo() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    create(&registry, &mut session, json!({}));
    registry.execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert!(session.can_redo());
    create(&registry, &mut session, json!({}));
    assert!(!session.can_redo());
}

#[test]
fn merged_edits_undo_as_one_step() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let id = create(&registry, &mut session, json!({}));
    let before_drag = session.scene().to_doc();
    for x in 1..=5 {
        let params = json!({"id": id, "component": "transform", "value": {"position": [x, 0, 0], "rotation": [0, 0, 0], "scale": [1, 1, 1]}});
        registry.execute_merging(&mut session, "scene.component.set", &params, Some("drag-1")).unwrap();
    }
    registry.execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert_eq!(session.scene().to_doc(), before_drag);
}

#[test]
fn read_only_commands_record_no_history() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let id = create(&registry, &mut session, json!({}));
    let steps = session.history().len();
    for cmd in ["app.version", "scene.objects", "scene.components"] {
        registry.execute(&mut session, cmd, &Value::Null).unwrap();
    }
    registry.execute(&mut session, "scene.object.inspect", &json!({"id": id})).unwrap();
    assert_eq!(session.history().len(), steps);
}

#[test]
fn create_with_components_is_one_undo_step_and_fails_atomically() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let id = create(
        &registry,
        &mut session,
        json!({"name": "Ball", "components": {"mesh": {"shape": "sphere", "color": [1, 0, 0, 1], "metallic": 0, "roughness": 0.5}, "tags": null}}),
    );
    let id = bayleaf_core::StableId::parse_hex(&id).unwrap();
    assert_eq!(session.scene().component(id, "mesh").unwrap()["shape"], "sphere");
    assert!(session.scene().component(id, "tags").is_ok());
    registry.execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert_eq!(session.scene().object_count(), 0);
    // An invalid component value refuses the whole creation.
    assert!(registry.execute(&mut session, "scene.object.create", &json!({"components": {"mesh": {"shape": "teapot"}}})).is_err());
    assert_eq!(session.scene().object_count(), 0);
}

#[test]
fn step_moves_reorder_and_nest_and_refuse_at_the_edges() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let a = create(&registry, &mut session, json!({"name": "A"}));
    let b = create(&registry, &mut session, json!({"name": "B"}));
    let roots = |s: &Session| s.scene().children(None).iter().map(|id| id.to_hex()).collect::<Vec<_>>();
    assert!(registry.execute(&mut session, "scene.object.moveUp", &json!({"id": a})).is_err(), "first object cannot move up");
    registry.execute(&mut session, "scene.object.moveUp", &json!({"id": b})).unwrap();
    assert_eq!(roots(&session), [b.clone(), a.clone()]);
    registry.execute(&mut session, "scene.object.indent", &json!({"id": a})).unwrap();
    assert_eq!(roots(&session), std::slice::from_ref(&b));
    registry.execute(&mut session, "scene.object.outdent", &json!({"id": a})).unwrap();
    assert_eq!(roots(&session), [b, a.clone()]);
    assert!(registry.execute(&mut session, "scene.object.outdent", &json!({"id": a})).is_err(), "a root cannot move out");
}

#[test]
fn render_refuses_bad_sizes_and_non_camera_objects_before_writing() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let id = create(&registry, &mut session, json!({}));
    let path = std::env::temp_dir().join(format!("bayleaf-render-refused-{}.png", std::process::id()));
    for params in [json!({"path": path, "width": 0}), json!({"path": path, "height": 100_000}), json!({"path": path, "camera": id})] {
        assert!(registry.execute(&mut session, "scene.render", &params).is_err(), "accepted {params}");
    }
    assert!(!path.exists());
}

#[test]
fn saving_clears_dirty_and_reopening_restores_the_scene() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    assert!(registry.execute(&mut session, "scene.save", &Value::Null).is_err(), "save without a file must be refused");
    create(&registry, &mut session, json!({"name": "Saved"}));
    assert!(session.is_dirty());
    let dir = std::env::temp_dir().join(format!("bayleaf-engine-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.bscene");
    registry.execute(&mut session, "scene.saveAs", &json!({"path": path})).unwrap();
    assert!(!session.is_dirty());
    let saved = session.scene().to_doc();
    registry.execute(&mut session, "scene.new", &Value::Null).unwrap();
    registry.execute(&mut session, "scene.open", &json!({"path": path})).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(session.scene().to_doc(), saved);
    assert!(!session.can_undo());
}

/// A spinner module: turns its object `speed` degrees per second around Y.
fn spinner_wasm() -> Vec<u8> {
    wat::parse_str(
        r#"(module
            (import "bayleaf" "transform_get" (func $get (param i32)))
            (import "bayleaf" "transform_set" (func $set (param i32)))
            (import "bayleaf" "property" (func $prop (param i32 i32) (result f32)))
            (import "bayleaf" "log" (func $log (param i32 i32)))
            (memory (export "memory") 1)
            (data (i32.const 100) "speed")
            (data (i32.const 120) "spinning")
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_start") (call $log (i32.const 120) (i32.const 8)))
            (func (export "bayleaf_update") (param $dt f32)
              (call $get (i32.const 0))
              (f32.store (i32.const 16)
                (f32.add (f32.load (i32.const 16)) (f32.mul (call $prop (i32.const 100) (i32.const 5)) (local.get $dt))))
              (call $set (i32.const 0))))"#,
    )
    .unwrap()
}

/// A saved scene in a temp folder with one object running `module` (written as `spin.wasm`).
fn scripted_scene(name: &str, module: &[u8]) -> (CommandRegistry, Session, String, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("bayleaf-play-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("spin.wasm"), module).unwrap();
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let id = create(&registry, &mut session, json!({"name": "Spinner", "components": {"scripts": [{"module": "spin.wasm", "properties": {"speed": 90}}]}}));
    registry.execute(&mut session, "scene.saveAs", &json!({"path": dir.join("play.bscene")})).unwrap();
    (registry, session, id, dir)
}

fn rotation_y(session: &Session, id: &str) -> f64 {
    let id = bayleaf_core::StableId::parse_hex(id).unwrap();
    session.scene().component(id, "transform").unwrap()["rotation"][1].as_f64().unwrap()
}

#[test]
fn play_runs_scripts_and_stop_restores_the_edited_scene() {
    let (registry, mut session, id, dir) = scripted_scene("restore", &spinner_wasm());
    let edited = session.scene().to_doc();
    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    assert!(session.is_playing());
    registry.execute(&mut session, "play.step", &json!({"seconds": 0.1, "steps": 10})).unwrap();
    assert!((rotation_y(&session, &id) - 90.0).abs() < 1e-3, "one second at 90°/s");
    assert!(session.console().iter().any(|l| l.text.contains("spinning")), "the script's log reached the console");
    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
    assert!(!session.is_playing());
    assert_eq!(session.scene().to_doc(), edited);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn pause_stops_ticks_but_step_still_advances() {
    let (registry, mut session, id, dir) = scripted_scene("pause", &spinner_wasm());
    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    registry.execute(&mut session, "play.pause", &Value::Null).unwrap();
    session.tick(1.0);
    assert_eq!(rotation_y(&session, &id), 0.0);
    registry.execute(&mut session, "play.step", &json!({"seconds": 0.5})).unwrap();
    assert!(rotation_y(&session, &id) > 0.0);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn files_and_history_are_locked_during_play_and_edits_are_not_recorded() {
    let (registry, mut session, id, dir) = scripted_scene("lock", &spinner_wasm());
    let steps = session.history().len();
    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    for cmd in ["scene.save", "scene.new", "edit.undo"] {
        assert!(matches!(registry.execute(&mut session, cmd, &Value::Null), Err(CommandError::Disabled { .. })), "{cmd} ran during play");
    }
    assert!(registry.execute(&mut session, "play.start", &Value::Null).is_err(), "play cannot start twice");
    registry.execute(&mut session, "scene.component.add", &json!({"id": id, "component": "tags"})).unwrap();
    assert_eq!(session.history().len(), steps, "play-mode edits are not undo steps");
    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
    let id = bayleaf_core::StableId::parse_hex(&id).unwrap();
    assert!(session.scene().component(id, "tags").is_err(), "play-mode edits are thrown away");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_missing_or_broken_script_is_reported_and_play_still_runs() {
    let (registry, mut session, _, dir) = scripted_scene("broken", b"not wasm");
    let result = registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    assert_eq!(result["running"], 0);
    assert!(session.console().iter().any(|l| l.level == crate::Level::Error && l.text.contains("spin.wasm")));
    registry.execute(&mut session, "play.step", &Value::Null).unwrap();
    assert!(registry.execute(&mut session, "play.step", &json!({"steps": 0})).is_err());
    assert!(registry.execute(&mut session, "play.step", &json!({"seconds": -1})).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn confined_sessions_refuse_files_outside_the_roots() {
    let base = std::env::temp_dir().join(format!("bayleaf-engine-confine-{}", std::process::id()));
    let (inside, outside) = (base.join("inside"), base.join("outside"));
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    create(&registry, &mut session, json!({"name": "A"}));
    session.set_confinement(Some(crate::confine::Confinement::new(std::slice::from_ref(&inside), std::slice::from_ref(&inside)).unwrap()));

    let refused = registry.execute(&mut session, "scene.saveAs", &json!({"path": outside.join("x.bscene")})).unwrap_err();
    assert!(refused.to_string().contains("outside the allowed folders"), "{refused}");
    assert!(!outside.join("x.bscene").exists());
    assert!(registry.execute(&mut session, "scene.saveAs", &json!({"path": "../outside/y.bscene"})).is_err(), ".. cannot climb out");
    assert!(registry.execute(&mut session, "scene.render", &json!({"path": outside.join("x.png")})).is_err());

    // Relative paths land in the first write root.
    registry.execute(&mut session, "scene.saveAs", &json!({"path": "kept.bscene"})).unwrap();
    assert!(inside.join("kept.bscene").exists());
    registry.execute(&mut session, "scene.open", &json!({"path": "kept.bscene"})).unwrap();

    // A script module outside the roots keeps play mode from starting.
    let id = bayleaf_core::StableId::parse_hex(&create(&registry, &mut session, json!({}))).unwrap();
    registry.execute(&mut session, "scene.script.add", &json!({"id": id, "module": "../outside/evil.wasm"})).unwrap();
    assert!(registry.execute(&mut session, "play.start", &Value::Null).is_err());
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn scripts_are_added_edited_and_removed_one_by_one() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let id = create(&registry, &mut session, json!({}));
    let first = registry.execute(&mut session, "scene.script.add", &json!({"id": id, "module": "scripts/spinner.wasm"})).unwrap();
    let second = registry.execute(&mut session, "scene.script.add", &json!({"id": id, "module": r"scripts\mover.wasm"})).unwrap();
    assert_eq!((first["index"].as_u64(), second["index"].as_u64()), (Some(0), Some(1)));
    registry.execute(&mut session, "scene.script.setProperty", &json!({"id": id, "index": 1, "name": "speed", "value": 7})).unwrap();
    let sid = bayleaf_core::StableId::parse_hex(&id).unwrap();
    let scripts = session.scene().component(sid, "scripts").unwrap();
    assert_eq!(scripts[1]["module"], "scripts/mover.wasm", "paths are stored with forward slashes");
    assert_eq!(scripts[1]["properties"]["speed"], 7.0);
    registry.execute(&mut session, "scene.script.setProperty", &json!({"id": id, "index": 1, "name": "speed", "value": null})).unwrap();
    assert!(session.scene().component(sid, "scripts").unwrap()[1].get("properties").is_none(), "null returns to the default");
    assert!(registry.execute(&mut session, "scene.script.remove", &json!({"id": id, "index": 5})).is_err());
    registry.execute(&mut session, "scene.script.remove", &json!({"id": id, "index": 0})).unwrap();
    registry.execute(&mut session, "scene.script.remove", &json!({"id": id, "index": 0})).unwrap();
    assert!(session.scene().component(sid, "scripts").is_err(), "the last removal removes the list");
    // One undo step per change.
    registry.execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert_eq!(session.scene().component(sid, "scripts").unwrap().as_array().map(Vec::len), Some(1));
}

fn node(registry: &CommandRegistry, session: &mut Session, kind: &str) -> u64 {
    registry.execute(session, "graph.node.add", &json!({"kind": kind, "position": [0, 0]})).unwrap()["node"].as_u64().unwrap()
}

fn pins(from: u64, from_pin: &str, to: u64, to_pin: &str) -> Value {
    json!({"from": {"node": from, "pin": from_pin}, "to": {"node": to, "pin": to_pin}})
}

#[test]
fn graph_edits_share_the_undo_history_and_saving_clears_dirty() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    assert!(registry.execute(&mut session, "graph.node.add", &json!({"kind": "number"})).is_err(), "no graph open");
    registry.execute(&mut session, "graph.new", &json!({"domain": "material"})).unwrap();
    create(&registry, &mut session, json!({"name": "Scene edit"}));
    let color = node(&registry, &mut session, "color");
    let output = session.graph().unwrap().graph.nodes[0].id;
    registry.execute(&mut session, "graph.link.add", &pins(color, "color", u64::from(output), "baseColor")).unwrap();
    assert!(session.graph().unwrap().dirty);
    assert_eq!(session.history(), ["scene.object.create", "graph.node.add", "graph.link.add"]);

    // A refused link names the clash and changes nothing.
    let refused = registry.execute(&mut session, "graph.link.add", &pins(color, "color", u64::from(output), "roughness")).unwrap_err();
    assert!(refused.to_string().contains("Number"), "{refused}");
    assert_eq!(session.history().len(), 3);

    registry.execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert!(session.graph().unwrap().graph.links.is_empty());
    registry.execute(&mut session, "edit.redo", &Value::Null).unwrap();
    assert_eq!(session.graph().unwrap().graph.links.len(), 1);

    let dir = std::env::temp_dir().join(format!("bayleaf-graph-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    assert!(registry.execute(&mut session, "graph.save", &Value::Null).is_err(), "a new graph needs a path");
    assert!(registry.execute(&mut session, "graph.save", &json!({"path": dir.join("red.bscript")})).is_err(), "materials end in .bmaterial");
    registry.execute(&mut session, "graph.save", &json!({"path": dir.join("red.bmaterial")})).unwrap();
    assert!(!session.graph().unwrap().dirty);
    let saved = session.graph().unwrap().graph.clone();

    // Opening a graph drops the old graph's undo steps but keeps the scene's.
    registry.execute(&mut session, "graph.open", &json!({"path": dir.join("red.bmaterial")})).unwrap();
    assert_eq!(session.graph().unwrap().graph, saved);
    assert_eq!(session.history(), ["scene.object.create"]);
    assert!(registry.execute(&mut session, "graph.get", &Value::Null).unwrap()["problems"].as_array().unwrap().is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn hostile_graph_params_change_nothing() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    registry.execute(&mut session, "graph.new", &json!({"domain": "script"})).unwrap();
    let start = session.graph().unwrap().graph.nodes[0].id;
    let hostile = [
        json!({"kind": "../../etc", "position": [0, 0]}),
        json!({"kind": "number", "position": [1e39, 0]}),
        json!({"node": 99999, "position": [0, 0]}),
        json!({"node": -1}),
        json!({"node": start, "name": "missing", "value": 1}),
        json!({"node": start, "pin": "then"}),
        json!({"from": {"node": start, "pin": "then"}, "to": {"node": start, "pin": "run"}}),
        json!({"from": {"node": start, "pin": "nope"}, "to": {"node": 7, "pin": "run"}}),
        json!({"domain": "unknown"}),
        json!({"path": "../../escape.bscript"}),
    ];
    let ids: Vec<_> = registry
        .list(&session)
        .into_iter()
        .map(|c| c.id)
        .filter(|id| id.starts_with("graph.") && !matches!(*id, "graph.new" | "graph.close" | "graph.open" | "graph.save"))
        .collect();
    for id in &ids {
        for bad in &hostile {
            let before = session.graph().cloned();
            if registry.execute(&mut session, id, bad).is_err() {
                assert_eq!(session.graph().cloned(), before, "{id} changed the graph on failure with {bad}");
            }
        }
    }
}

#[test]
fn a_visual_script_from_the_graph_editor_runs_in_play_mode() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    registry.execute(&mut session, "graph.new", &json!({"domain": "script"})).unwrap();
    let update = session.graph().unwrap().graph.nodes.iter().find(|n| n.kind == "update").unwrap().id;
    let rotate = node(&registry, &mut session, "rotateBy");
    registry.execute(&mut session, "graph.node.set", &json!({"node": rotate, "name": "degrees", "value": [0, 45, 0]})).unwrap();
    registry.execute(&mut session, "graph.link.add", &pins(u64::from(update), "then", rotate, "run")).unwrap();

    let dir = std::env::temp_dir().join(format!("bayleaf-graph-play-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    registry.execute(&mut session, "graph.save", &json!({"path": dir.join("turn.bscript")})).unwrap();
    let id = create(&registry, &mut session, json!({"name": "Turner", "components": {"scripts": [{"module": "turn.bscript"}]}}));
    registry.execute(&mut session, "scene.saveAs", &json!({"path": dir.join("turn.bscene")})).unwrap();
    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    registry.execute(&mut session, "play.step", &json!({"seconds": 0.1, "steps": 2})).unwrap();
    assert!((rotation_y(&session, &id) - 90.0).abs() < 1e-3, "45° per step");
    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn duplicate_copy_and_paste_make_new_objects_as_one_undo_step_each() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let crate_id = create(&registry, &mut session, json!({"name": "Crate"}));
    let other = create(&registry, &mut session, json!({"name": "Other"}));
    create(&registry, &mut session, json!({"name": "Lid", "parent": crate_id}));

    let copy = registry.execute(&mut session, "scene.object.duplicate", &json!({"id": crate_id})).unwrap()["id"].as_str().unwrap().to_owned();
    let names: Vec<_> = session.scene().children(None).iter().map(|id| session.scene().name(*id).unwrap().to_owned()).collect();
    assert_eq!(names, ["Crate", "Crate 2", "Other"], "the copy follows the original and gets the next free number");
    let copy_id = bayleaf_core::StableId::parse_hex(&copy).unwrap();
    assert_eq!(session.scene().children(Some(copy_id)).len(), 1, "children come along");
    registry.execute(&mut session, "scene.object.duplicate", &json!({"id": copy})).unwrap();
    assert!(session.scene().children(None).iter().any(|id| session.scene().name(*id) == Some("Crate 3")));

    let clipboard = registry.execute(&mut session, "scene.object.copy", &json!({"ids": [crate_id]})).unwrap();
    let text = serde_json::to_string(&clipboard).unwrap();
    let pasted =
        registry.execute(&mut session, "scene.object.paste", &json!({"objects": serde_json::from_str::<Value>(&text).unwrap(), "parent": other})).unwrap();
    assert_eq!(pasted["ids"].as_array().map(Vec::len), Some(1));
    let other_id = bayleaf_core::StableId::parse_hex(&other).unwrap();
    assert_eq!(session.scene().children(Some(other_id)).len(), 1, "pasted under the given parent");

    let before = session.history().len();
    registry.execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert!(session.scene().children(Some(other_id)).is_empty());
    assert_eq!(session.history().len(), before - 1);
    assert!(registry.execute(&mut session, "scene.object.paste", &json!({"objects": {"format": "something else"}})).is_err());
}

#[test]
fn a_new_project_has_a_working_starting_scene_and_paths_start_at_its_folder() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let base = std::env::temp_dir().join(format!("bayleaf-new-project-{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    let dir = base.join("My Game");
    let made = registry.execute(&mut session, "project.new", &json!({"path": dir})).unwrap();
    assert_eq!(made["name"], "My Game");
    for file in ["My_Game.bproject", "scenes/main.bscene", "materials/ground.bmaterial", "scripts/spin.bscript", "assets/README.md", ".gitignore", "README.md"]
    {
        assert!(dir.join(file).is_file(), "{file} is missing");
    }
    // The scene sits in scenes/, but its paths start at the project folder.
    assert_eq!(session.project_dir(), Some(dir.as_path()));
    assert_eq!(session.script_dir(), dir);
    assert!(
        registry.execute(&mut session, "graph.open", &json!({"path": dir.join("materials/ground.bmaterial")})).unwrap()["problems"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        registry.execute(&mut session, "graph.open", &json!({"path": dir.join("scripts/spin.bscript")})).unwrap()["problems"].as_array().unwrap().is_empty()
    );

    let cube = session.scene().children(None).iter().copied().find(|id| session.scene().name(*id) == Some("Cube")).unwrap().to_hex();
    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    registry.execute(&mut session, "play.step", &json!({"seconds": 1.0})).unwrap();
    assert!((rotation_y(&session, &cube) - 45.0).abs() < 1e-3, "the starter script turns the cube");
    assert!(!session.console().iter().any(|l| l.level == crate::Level::Error), "{:?}", session.console());
    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();

    assert!(registry.execute(&mut session, "project.new", &json!({"path": dir})).unwrap_err().to_string().contains("not empty"));
    registry.execute(&mut session, "scene.new", &Value::Null).unwrap();
    assert!(session.project_dir().is_none());
    registry.execute(&mut session, "project.open", &json!({"path": dir})).unwrap();
    assert_eq!(registry.execute(&mut session, "project.info", &Value::Null).unwrap()["mainScene"], "scenes/main.bscene");
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn prefabs_are_placed_applied_to_every_copy_and_reverted() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-prefab-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    registry.execute(&mut session, "scene.saveAs", &json!({"path": dir.join("level.bscene")})).unwrap();
    let crate_id = create(
        &registry,
        &mut session,
        json!({"name": "Crate", "components": {"mesh": {"shape": "cube", "color": [0.5, 0.3, 0.1, 1], "metallic": 0, "roughness": 0.5}}}),
    );
    create(&registry, &mut session, json!({"name": "Lid", "parent": crate_id}));
    let made = registry.execute(&mut session, "scene.prefab.create", &json!({"id": crate_id, "path": "prefabs/crate.bprefab"})).unwrap();
    assert_eq!(made["source"], "prefabs/crate.bprefab");
    assert!(dir.join("prefabs/crate.bprefab").is_file());

    let second = registry.execute(&mut session, "scene.prefab.place", &json!({"path": "prefabs/crate.bprefab"})).unwrap()["id"].as_str().unwrap().to_owned();
    let second_id = bayleaf_core::StableId::parse_hex(&second).unwrap();
    registry
        .execute(
            &mut session,
            "scene.component.set",
            &json!({"id": second, "component": "transform", "value": {"position": [5, 0, 0], "rotation": [0, 0, 0], "scale": [1, 1, 1]}}),
        )
        .unwrap();
    assert_eq!(session.scene().children(Some(second_id)).len(), 1, "the copy has the lid");

    // Edit the first copy and apply: the second copy follows but stays where it is.
    registry
        .execute(
            &mut session,
            "scene.component.set",
            &json!({"id": crate_id, "component": "mesh", "value": {"shape": "sphere", "color": [0.5, 0.3, 0.1, 1], "metallic": 0, "roughness": 0.5}}),
        )
        .unwrap();
    create(&registry, &mut session, json!({"name": "Handle", "parent": crate_id}));
    let steps = session.history().len();
    assert_eq!(registry.execute(&mut session, "scene.prefab.apply", &json!({"id": crate_id})).unwrap()["updated"], 1);
    assert_eq!(session.history().len(), steps + 1, "one undo step");
    assert_eq!(session.scene().component(second_id, "mesh").unwrap()["shape"], "sphere");
    assert_eq!(session.scene().children(Some(second_id)).len(), 2);
    assert_eq!(session.scene().component(second_id, "transform").unwrap()["position"][0], 5.0, "a copy keeps its own transform");
    assert!(session.scene().component(second_id, "prefab").is_ok(), "and its link");

    // Revert throws away the copy's own changes.
    registry
        .execute(
            &mut session,
            "scene.component.set",
            &json!({"id": second, "component": "mesh", "value": {"shape": "plane", "color": [1, 1, 1, 1], "metallic": 0, "roughness": 0.5}}),
        )
        .unwrap();
    registry.execute(&mut session, "scene.prefab.revert", &json!({"id": second})).unwrap();
    assert_eq!(session.scene().component(second_id, "mesh").unwrap()["shape"], "sphere");
    let plain = create(&registry, &mut session, json!({}));
    assert!(registry.execute(&mut session, "scene.prefab.apply", &json!({"id": plain})).is_err(), "not a placed prefab");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn scripts_spawn_prefabs_and_destroy_objects_in_play_mode_and_stop_undoes_it() {
    let module = wat::parse_str(
        r#"(module
            (import "bayleaf" "mouse_down" (func $mouse (param i32) (result i32)))
            (import "bayleaf" "find" (func $find (param i32 i32) (result i32)))
            (import "bayleaf" "spawn" (func $spawn (param i32 i32 f32 f32 f32) (result i32)))
            (import "bayleaf" "destroy" (func $destroy (param i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 100) "ball.bprefab")
            (data (i32.const 120) "Junk")
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_update") (param f32)
              (if (call $mouse (i32.const 0))
                (then
                  (drop (call $spawn (i32.const 100) (i32.const 12) (f32.const 0) (f32.const 5) (f32.const 0)))
                  (drop (call $destroy (call $find (i32.const 120) (i32.const 4))))))))"#,
    )
    .unwrap();
    let (registry, mut session, _, dir) = scripted_scene("spawn", &module);
    create(&registry, &mut session, json!({"name": "Junk"}));
    let ball = create(&registry, &mut session, json!({"name": "Ball"}));
    registry.execute(&mut session, "scene.prefab.create", &json!({"id": ball, "path": "ball.bprefab"})).unwrap();
    registry.execute(&mut session, "scene.object.delete", &json!({"id": ball})).unwrap();
    registry.execute(&mut session, "scene.save", &Value::Null).unwrap();
    let edited = session.scene().to_doc();
    let named = |session: &Session, name: &str| session.scene().world_matrices().into_iter().filter(|(id, _)| session.scene().name(*id) == Some(name)).count();

    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    registry.execute(&mut session, "play.step", &Value::Null).unwrap();
    assert_eq!(named(&session, "Ball"), 0, "nothing without the click");
    registry.execute(&mut session, "play.input", &json!({"keys": [], "pointer": [0.1, 0.2], "buttons": 1})).unwrap();
    assert!(registry.execute(&mut session, "play.input", &json!({"keys": [], "pad": {"buttons": ["Jump"]}})).is_err(), "unknown gamepad buttons are refused");
    registry
        .execute(
            &mut session,
            "play.input",
            &json!({"keys": [], "pointer": [0.1, 0.2], "buttons": 1, "pad": {"axes": [2, 0, 0, 0, 0, 0], "buttons": ["South"]}}),
        )
        .unwrap();
    registry.execute(&mut session, "play.step", &Value::Null).unwrap();
    assert_eq!(named(&session, "Ball"), 1, "the click spawned a ball");
    assert_eq!(named(&session, "Junk"), 0, "and destroyed the junk");
    let spawned = session.scene().world_matrices().into_iter().find(|(id, _)| session.scene().name(*id) == Some("Ball")).unwrap().0;
    assert_eq!(session.scene().component(spawned, "transform").unwrap()["position"][1], 5.0);
    assert!(session.console().iter().all(|l| l.level != crate::Level::Error), "{:?}", session.console());
    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
    assert_eq!(session.scene().to_doc(), edited, "stop restores the scene");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_new_rust_script_builds_to_a_module_that_runs() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-rust-script-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("scripts")).unwrap();
    registry.execute(&mut session, "scene.saveAs", &json!({"path": dir.join("s.bscene")})).unwrap();
    let made = registry.execute(&mut session, "script.new", &json!({"folder": "scripts", "name": "Turn Table"})).unwrap();
    assert!(dir.join("scripts/turn_table/src/lib.rs").is_file());
    assert!(registry.execute(&mut session, "script.new", &json!({"folder": "scripts", "name": "turn table"})).is_err(), "the name is taken");

    // A shared build folder keeps later runs fast.
    let target = std::env::temp_dir().join("bayleaf-script-build-cache");
    match crate::code_scripts::build_script(&dir.join("scripts/turn_table"), &target) {
        Ok(built) => {
            assert_eq!(built.module, std::path::PathBuf::from(made["module"].as_str().unwrap()));
            let id = create(&registry, &mut session, json!({"name": "Table", "components": {"scripts": [{"module": "scripts/turn_table.wasm"}]}}));
            registry.execute(&mut session, "play.start", &Value::Null).unwrap();
            registry.execute(&mut session, "play.step", &json!({"seconds": 0.125})).unwrap();
            assert!((rotation_y(&session, &id) - 45.0).abs() < 1e-3, "the template turns one turn per second");
            registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
        }
        Err(error) if error.contains("rustup target add") || error.contains("needs Rust") => eprintln!("skipped: {error}"),
        Err(error) => panic!("{error}"),
    }
    session.set_confinement(Some(crate::confine::Confinement::new(std::slice::from_ref(&dir), std::slice::from_ref(&dir)).unwrap()));
    assert!(registry.execute(&mut session, "script.build", &json!({"path": "scripts/turn_table"})).unwrap_err().to_string().contains("agents may not"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn file_info_describes_project_files_and_stays_inside_the_roots() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-file-info-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    registry.execute(&mut session, "project.new", &json!({"path": dir})).unwrap();
    let info = |session: &mut Session, path: &str| registry.execute(session, "asset.info", &json!({ "path": path })).unwrap();
    let row = |info: &Value, label: &str| info["rows"].as_array().unwrap().iter().find(|r| r[0] == label).map(|r| r[1].as_str().unwrap().to_owned());

    let scene = info(&mut session, "scenes/main.bscene");
    assert_eq!(scene["kind"], "Scene");
    assert_eq!(row(&scene, "Objects").as_deref(), Some("4"));
    let script = info(&mut session, "scripts/spin.bscript");
    assert_eq!(script["kind"], "Visual Script");
    assert!(script["problems"].as_array().unwrap().is_empty(), "{script}");
    assert!(row(&script, "Properties").unwrap().contains("speed"), "the declared property is listed");

    // A long text file is cut, and never inside a character.
    let long = "ż".repeat(crate::asset_info::TEXT_PREVIEW_BYTES);
    std::fs::write(dir.join("notes.txt"), &long).unwrap();
    let text = info(&mut session, "notes.txt")["text"].as_str().unwrap().to_owned();
    assert!(text.len() < long.len() && text.ends_with('…'));

    let broken = info(&mut session, "missing.png");
    assert!(!broken["problems"].as_array().unwrap().is_empty());

    let inside = dir.join("scenes");
    session.set_confinement(Some(crate::confine::Confinement::new(std::slice::from_ref(&inside), std::slice::from_ref(&inside)).unwrap()));
    assert!(registry.execute(&mut session, "asset.info", &json!({"path": dir.join("README.md")})).is_err(), "outside the read roots");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn copied_nodes_paste_with_new_ids_and_their_links_and_one_of_a_kind_nodes_stay_single() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    registry.execute(&mut session, "graph.new", &json!({"domain": "material"})).unwrap();
    let output = u64::from(session.graph().unwrap().graph.nodes[0].id);
    let color = node(&registry, &mut session, "color");
    let blend = node(&registry, &mut session, "blend");
    registry.execute(&mut session, "graph.node.set", &json!({"node": color, "name": "color", "value": [0.1, 0.2, 0.3]})).unwrap();
    registry.execute(&mut session, "graph.link.add", &pins(color, "color", blend, "a")).unwrap();
    registry.execute(&mut session, "graph.link.add", &pins(blend, "result", output, "baseColor")).unwrap();

    let text = registry.execute(&mut session, "graph.node.copy", &json!({"nodes": [color, blend, output]})).unwrap()["text"].as_str().unwrap().to_owned();
    let pasted = registry.execute(&mut session, "graph.node.paste", &json!({"text": text})).unwrap();
    assert_eq!(pasted["skipped"], json!(["Material Output"]), "the output node may appear once");
    let new: Vec<u32> = pasted["nodes"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect();
    let graph = &session.graph().unwrap().graph;
    assert_eq!(graph.nodes.len(), 5);
    assert!(new.iter().all(|id| ![color, blend, output].contains(&u64::from(*id))), "copies get new ids");
    assert!(graph.links.iter().any(|l| l.from.node == new[0] && l.to.node == new[1]), "the link between the copies is copied");
    assert_eq!(graph.links.iter().filter(|l| l.to.node == output as u32).count(), 1, "links to nodes left out are not");
    assert_eq!(graph.value(new[0], "color"), graph.value(color as u32, "color"), "values come along");

    registry.execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert_eq!(session.graph().unwrap().graph.nodes.len(), 3, "a paste is one undo step");
    let duplicated = registry.execute(&mut session, "graph.node.duplicate", &json!({"nodes": [blend]})).unwrap();
    assert_eq!(duplicated["nodes"].as_array().unwrap().len(), 1);
    assert!(registry.execute(&mut session, "graph.node.duplicate", &json!({"nodes": [output]})).is_err(), "nothing to duplicate");

    registry.execute(&mut session, "graph.new", &json!({"domain": "script"})).unwrap();
    let refused = registry.execute(&mut session, "graph.node.paste", &json!({"text": text})).unwrap_err();
    assert!(refused.to_string().contains("material nodes"), "{refused}");
}

#[test]
fn a_render_without_a_path_comes_back_as_base64_png() {
    assert_eq!([b"M".as_slice(), b"Ma", b"Man"].map(crate::scene_cmds::base64), ["TQ==", "TWE=", "TWFu"]);
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    create(&registry, &mut session, json!({}));
    assert!(registry.execute(&mut session, "scene.render", &json!({"width": 4096, "height": 8})).is_err(), "inline renders stay small");
    match registry.execute(&mut session, "scene.render", &json!({"width": 8, "height": 8})) {
        // The PNG signature, in base64.
        Ok(result) => assert!(result["png"].as_str().unwrap().starts_with("iVBORw0KGgo"), "{result}"),
        Err(e) if std::env::var_os("BAYLEAF_GPU_REQUIRED").is_none() => eprintln!("no GPU, skipped: {e}"),
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn prefab_copies_keep_their_own_changes_nest_and_update_when_the_scene_opens() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-prefab-overrides-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    registry.execute(&mut session, "scene.saveAs", &json!({"path": dir.join("room.bscene")})).unwrap();
    let run = |session: &mut Session, id: &str, params: Value| registry.execute(session, id, &params).unwrap();
    let mesh = |color: [f32; 3]| json!({"shape": "sphere", "color": [color[0], color[1], color[2], 1], "metallic": 0, "roughness": 0.5});
    let color = |session: &Session, id: bayleaf_core::StableId| session.scene().component(id, "mesh").unwrap()["color"][0].as_f64().unwrap();
    let bulb_of = |session: &Session, lamp: &str| session.scene().children(Some(bayleaf_core::StableId::parse_hex(lamp).unwrap()))[0];

    // A lamp prefab with a bulb, placed twice.
    let first = create(&registry, &mut session, json!({"name": "Lamp"}));
    create(&registry, &mut session, json!({"name": "Bulb", "parent": first, "components": {"mesh": mesh([0.9, 0.0, 0.0])}}));
    run(&mut session, "scene.prefab.create", json!({"id": first, "path": "lamp.bprefab"}));
    let second = run(&mut session, "scene.prefab.place", json!({"path": "lamp.bprefab"}))["id"].as_str().unwrap().to_owned();

    // The second copy's bulb is its own colour: an override.
    let bulb = bulb_of(&session, &second);
    run(&mut session, "scene.component.set", json!({"id": bulb, "component": "mesh", "value": mesh([0.1, 0.0, 0.9])}));
    assert_eq!(session.scene().component(bayleaf_core::StableId::parse_hex(&second).unwrap(), "prefab").unwrap()["overrides"], json!(["1/mesh"]));

    // The first copy gains a light and is applied: the second gets the light and keeps its colour.
    let first_bulb = bulb_of(&session, &first);
    run(&mut session, "scene.component.add", json!({"id": first_bulb, "component": "pointLight"}));
    run(&mut session, "scene.prefab.apply", json!({"id": first}));
    let bulb = bulb_of(&session, &second);
    assert!(session.scene().component(bulb, "pointLight").is_ok(), "the change from the file arrived");
    assert!((color(&session, bulb) - 0.1).abs() < 1e-6, "the copy's own colour stayed");

    // A room prefab holding a lamp copy; the lamp inside follows the lamp file.
    let room = create(&registry, &mut session, json!({"name": "Room"}));
    run(&mut session, "scene.prefab.place", json!({"path": "lamp.bprefab", "parent": room}));
    run(&mut session, "scene.prefab.create", json!({"id": room, "path": "room.bprefab"}));
    let placed_room = run(&mut session, "scene.prefab.place", json!({"path": "room.bprefab"}))["id"].as_str().unwrap().to_owned();
    assert!(registry.execute(&mut session, "scene.prefab.place", &json!({"path": "lamp.bprefab", "parent": first})).is_ok());
    let nested_self = session.scene().children(Some(bayleaf_core::StableId::parse_hex(&first).unwrap())).last().copied().unwrap();
    assert!(registry.execute(&mut session, "scene.prefab.apply", &json!({"id": first})).is_err(), "a prefab may not hold itself");
    run(&mut session, "scene.object.delete", json!({"id": nested_self}));

    // The lamp file changes outside this scene; opening the scene brings it in, overrides kept.
    run(&mut session, "scene.save", json!({}));
    let file = dir.join("lamp.bprefab");
    let mut prefab: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    prefab["objects"][1]["components"]["mesh"]["color"][0] = json!(0.5);
    std::fs::write(&file, prefab.to_string()).unwrap();
    run(&mut session, "scene.open", json!({"path": dir.join("room.bscene")}));
    assert!(!session.is_dirty(), "updating copies on open does not mark the scene changed");
    assert!((color(&session, bulb_of(&session, &first)) - 0.5).abs() < 1e-6, "a plain copy follows the file");
    assert!((color(&session, bulb_of(&session, &second)) - 0.1).abs() < 1e-6, "a copy's own colour stays");
    let lamp_in_room = session.scene().children(Some(bayleaf_core::StableId::parse_hex(&placed_room).unwrap()))[0];
    let bulb_in_room = session.scene().children(Some(lamp_in_room))[0];
    assert!((color(&session, bulb_in_room) - 0.5).abs() < 1e-6, "the lamp inside the placed room follows the lamp file");

    // Revert drops the copy's own changes.
    run(&mut session, "scene.prefab.revert", json!({"id": second}));
    assert!((color(&session, bulb_of(&session, &second)) - 0.5).abs() < 1e-6);
    assert!(session.scene().component(bayleaf_core::StableId::parse_hex(&second).unwrap(), "prefab").unwrap().get("overrides").is_none());
    std::fs::remove_dir_all(&dir).ok();
}

/// A box with a script falls onto a floor in play mode: physics moves it, the script hears of the
/// landing, and stopping play puts the box back.
#[cfg(feature = "physics")]
#[test]
fn physics_drops_a_box_on_the_floor_and_its_script_hears_the_collision() {
    use bayleaf_graph::{Graph, Link, PinRef};
    use bayleaf_script::visual::CATALOG;
    let dir = std::env::temp_dir().join(format!("bayleaf-physics-play-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut graph = Graph::empty(&CATALOG);
    let landed = graph.add_node(&CATALOG, "message", [0.0, 0.0]).unwrap();
    graph.set_value(&CATALOG, landed, "message", json!("collision")).unwrap();
    let log = graph.add_node(&CATALOG, "log", [0.0, 0.0]).unwrap();
    graph.set_value(&CATALOG, log, "message", json!("landed")).unwrap();
    graph.connect(&CATALOG, Link { from: PinRef::new(landed, "then"), to: PinRef::new(log, "run") }).unwrap();
    std::fs::write(dir.join("land.bscript"), graph.to_json()).unwrap();

    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    registry.execute(&mut session, "scene.saveAs", &json!({"path": dir.join("drop.bscene")})).unwrap();
    let collider = json!({"shape": "box", "size": [1, 1, 1], "friction": 0.5, "restitution": 0, "trigger": false});
    create(
        &registry,
        &mut session,
        json!({"name": "Floor", "components": {"collider": collider, "transform": {"position": [0, -0.5, 0], "rotation": [0, 0, 0], "scale": [20, 1, 20]}}}),
    );
    let body = json!({"kind": "dynamic", "mass": 1, "gravityScale": 1, "lockRotation": false});
    let id = create(
        &registry,
        &mut session,
        json!({"name": "Box", "components": {"rigidBody": body, "collider": collider, "transform": {"position": [0, 3, 0], "rotation": [0, 0, 0], "scale": [1, 1, 1]}}}),
    );
    registry.execute(&mut session, "scene.script.add", &json!({"id": id, "module": "land.bscript"})).unwrap();
    let box_id = bayleaf_core::StableId::parse_hex(&id).unwrap();
    let height = |s: &Session| s.scene().get::<bayleaf_scene::components::Transform>(box_id).unwrap().position[1];

    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    registry.execute(&mut session, "play.step", &json!({"seconds": 0.05, "steps": 60})).unwrap();
    assert!((height(&session) - 0.5).abs() < 0.05, "the box rests on the floor: {}", height(&session));
    let status = registry.execute(&mut session, "play.status", &Value::Null).unwrap();
    assert!(status["console"].to_string().contains("landed"), "the script heard the collision: {status}");
    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
    assert_eq!(height(&session), 3.0, "stopping play puts it back");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn project_settings_are_kept_in_the_project_file_and_checked() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let outside = registry.execute(&mut session, "project.settings", &Value::Null).unwrap();
    assert_eq!(outside["project"], false);
    assert_eq!(outside["settings"]["physics"]["enabled"], true, "defaults outside a project");
    assert_eq!(outside["settings"]["editor"]["hideExtensions"], true, "file extensions are hidden unless the project says otherwise");
    assert!(registry.execute(&mut session, "project.settings.set", &json!({"settings": {"physics": {"enabled": false, "gravity": [0, 0, 0]}}})).is_err());

    let dir = std::env::temp_dir().join(format!("bayleaf-settings-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    registry.execute(&mut session, "project.new", &json!({"path": dir})).unwrap();
    let set = json!({"settings": {"physics": {"enabled": false, "gravity": [0, -3.7, 0]}, "editor": {"hideExtensions": false}}});
    registry.execute(&mut session, "project.settings.set", &set).unwrap();
    let bad = json!({"settings": {"physics": {"enabled": true, "gravity": [0, -1e9, 0]}}});
    assert!(registry.execute(&mut session, "project.settings.set", &bad).is_err(), "gravity has a limit");

    // A fresh session that opens the project reads them back from the file.
    let mut other = Session::default();
    registry.execute(&mut other, "project.open", &json!({"path": dir})).unwrap();
    let read = registry.execute(&mut other, "project.settings", &Value::Null).unwrap();
    assert_eq!(read["settings"]["physics"]["enabled"], false);
    assert!((read["settings"]["physics"]["gravity"][1].as_f64().unwrap() + 3.7).abs() < 1e-5);
    assert_eq!(read["settings"]["editor"]["hideExtensions"], false);
    std::fs::remove_dir_all(&dir).ok();
}

/// Turned off in the project settings, physics leaves bodies where they are; turned on with
/// gravity pointing up, it lifts them.
#[cfg(feature = "physics")]
#[test]
fn play_mode_follows_the_physics_settings() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-settings-play-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    registry.execute(&mut session, "project.new", &json!({"path": dir})).unwrap();
    let collider = json!({"shape": "box", "size": [1, 1, 1], "friction": 0.5, "restitution": 0, "trigger": false});
    let body = json!({"kind": "dynamic", "mass": 1, "gravityScale": 1, "lockRotation": false});
    let id = create(
        &registry,
        &mut session,
        json!({"name": "Box", "components": {"rigidBody": body, "collider": collider, "transform": {"position": [0, 20, 0], "rotation": [0, 0, 0], "scale": [1, 1, 1]}}}),
    );
    let box_id = bayleaf_core::StableId::parse_hex(&id).unwrap();
    let height_after = |session: &mut Session, physics: serde_json::Value| {
        registry.execute(session, "project.settings.set", &json!({"settings": {"physics": physics}})).unwrap();
        registry.execute(session, "play.start", &Value::Null).unwrap();
        registry.execute(session, "play.step", &json!({"seconds": 0.05, "steps": 20})).unwrap();
        let y = session.scene().get::<bayleaf_scene::components::Transform>(box_id).unwrap().position[1];
        registry.execute(session, "play.stop", &Value::Null).unwrap();
        y
    };
    assert_eq!(height_after(&mut session, json!({"enabled": false, "gravity": [0, -9.81, 0]})), 20.0, "physics off: it stays");
    assert!(height_after(&mut session, json!({"enabled": true, "gravity": [0, 9.81, 0]})) > 20.5, "gravity up: it rises");
    std::fs::remove_dir_all(&dir).ok();
}

/// A character with a visual script walks across the floor in play mode, and an agent's
/// physics.raycast finds the floor and the character.
#[cfg(feature = "physics")]
#[test]
fn a_scripted_character_walks_and_rays_find_it() {
    use bayleaf_graph::{Graph, Link, PinRef};
    use bayleaf_script::visual::CATALOG;
    let dir = std::env::temp_dir().join(format!("bayleaf-character-play-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut graph = Graph::empty(&CATALOG);
    let update = graph.add_node(&CATALOG, "update", [0.0, 0.0]).unwrap();
    let walk = graph.add_node(&CATALOG, "walk", [0.0, 0.0]).unwrap();
    graph.set_value(&CATALOG, walk, "velocity", json!([2.0, 0.0, 0.0])).unwrap();
    graph.connect(&CATALOG, Link { from: PinRef::new(update, "then"), to: PinRef::new(walk, "run") }).unwrap();
    std::fs::write(dir.join("walk.bscript"), graph.to_json()).unwrap();

    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    registry.execute(&mut session, "scene.saveAs", &json!({"path": dir.join("walk.bscene")})).unwrap();
    let collider = json!({"shape": "box", "size": [1, 1, 1], "friction": 0.5, "restitution": 0, "trigger": false});
    create(
        &registry,
        &mut session,
        json!({"name": "Floor", "components": {"collider": collider, "transform": {"position": [0, -0.5, 0], "rotation": [0, 0, 0], "scale": [40, 1, 40]}}}),
    );
    let character = json!({"height": 1.8, "radius": 0.4, "maxSlope": 45, "stepHeight": 0.3});
    let id = create(
        &registry,
        &mut session,
        json!({"name": "Hero", "components": {"characterBody": character, "transform": {"position": [0, 1, 0], "rotation": [0, 0, 0], "scale": [1, 1, 1]}}}),
    );
    registry.execute(&mut session, "scene.script.add", &json!({"id": id, "module": "walk.bscript"})).unwrap();

    let down = registry.execute(&mut session, "physics.raycast", &json!({"origin": [5, 10, 0], "direction": [0, -1, 0]})).unwrap();
    assert_eq!(down["name"], "Floor", "outside play mode the ray uses the scene as it is: {down}");
    assert!((down["distance"].as_f64().unwrap() - 10.0).abs() < 1e-3);

    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    registry.execute(&mut session, "play.step", &json!({"seconds": 0.05, "steps": 40})).unwrap();
    let hero = bayleaf_core::StableId::parse_hex(&id).unwrap();
    let p = session.scene().get::<bayleaf_scene::components::Transform>(hero).unwrap().position;
    assert!(p[0] > 2.5 && p[0] < 4.5, "2 m/s for about 2 s: x = {}", p[0]);
    assert!((p[1] - 0.9).abs() < 0.15, "on its feet on the floor: y = {}", p[1]);
    let at_hero = registry.execute(&mut session, "physics.raycast", &json!({"origin": [p[0], 10.0, p[2]], "direction": [0, -1, 0]})).unwrap();
    assert_eq!(at_hero["name"], "Hero", "in play mode the ray meets the character where it walked to");
    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
    std::fs::remove_dir_all(&dir).ok();
}

/// Player Control: W walks the hero the way the camera looks, Space jumps, and the camera follows.
#[cfg(all(feature = "physics", feature = "gameplay"))]
#[test]
fn player_control_walks_where_the_camera_looks_and_the_camera_follows() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-player-control-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    registry.execute(&mut session, "project.new", &json!({"path": dir})).unwrap();
    // The starter scene has a camera looking down -Z from +Z, and ground with no collider; add a floor.
    let collider = json!({"shape": "box", "size": [1, 1, 1], "friction": 0.5, "restitution": 0, "trigger": false});
    create(
        &registry,
        &mut session,
        json!({"name": "Floor", "components": {"collider": collider, "transform": {"position": [0, -0.5, 0], "rotation": [0, 0, 0], "scale": [200, 1, 200]}}}),
    );
    let character = json!({"height": 1.8, "radius": 0.4, "maxSlope": 45, "stepHeight": 0.3});
    let control = json!({"speed": 4, "jumpSpeed": 6, "turnToMove": true, "camera": "follow", "cameraDistance": 6, "cameraHeight": 3});
    let id = create(
        &registry,
        &mut session,
        json!({"name": "Hero", "components": {"characterBody": character, "playerControl": control, "transform": {"position": [0, 1, 0], "rotation": [0, 0, 0], "scale": [1, 1, 1]}}}),
    );
    let hero = bayleaf_core::StableId::parse_hex(&id).unwrap();
    let position = |s: &Session| s.scene().get::<bayleaf_scene::components::Transform>(hero).unwrap().position;

    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    registry.execute(&mut session, "play.step", &json!({"seconds": 0.05, "steps": 10})).unwrap();
    registry.execute(&mut session, "play.input", &json!({"keys": ["W"]})).unwrap();
    registry.execute(&mut session, "play.step", &json!({"seconds": 0.05, "steps": 20})).unwrap();
    let walked = position(&session);
    assert!(walked[2] < -3.0 && walked[0].abs() < 0.5, "W walks toward -Z, where the camera looks: {walked:?}");

    let camera = session
        .scene()
        .world_matrices()
        .into_iter()
        .map(|(id, _)| id)
        .find(|id| session.scene().get::<bayleaf_scene::components::Camera>(*id).is_some())
        .unwrap();
    let eye = session.scene().get::<bayleaf_scene::components::Transform>(camera).unwrap().position;
    let flat = ((eye[0] - walked[0]).powi(2) + (eye[2] - walked[2]).powi(2)).sqrt();
    assert!((flat - 6.0).abs() < 0.3, "the camera trails 6 m behind: {flat}");

    registry.execute(&mut session, "play.input", &json!({"keys": ["Space"]})).unwrap();
    registry.execute(&mut session, "play.step", &json!({"seconds": 0.05, "steps": 3})).unwrap();
    assert!(position(&session)[1] > walked[1] + 0.3, "Space jumps");
    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
    std::fs::remove_dir_all(&dir).ok();
}

/// Game Rules: the player prefab starts at the Player Start, a fall sends it back there, and the
/// time limit ends the game once, which every script hears. The state starts empty.
#[cfg(all(feature = "physics", feature = "gameplay"))]
#[test]
fn game_rules_place_restart_and_end_the_game_and_the_state_starts_empty() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-game-rules-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    registry.execute(&mut session, "project.new", &json!({"path": dir})).unwrap();
    // A referee that notes the win in the game's state.
    let referee = wat::parse_str(
        r#"(module
            (import "bayleaf" "message_is" (func $is (param i32 i32) (result i32)))
            (import "bayleaf" "state_set" (func $set (param i32 i32 i32 i32 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 100) "gameWon")
            (data (i32.const 120) "cheered")
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_update") (param f32))
            (func (export "bayleaf_message") (param f32)
              (if (call $is (i32.const 100) (i32.const 7))
                (then (drop (call $set (i32.const 0) (i32.const 0) (i32.const 120) (i32.const 7) (f64.const 1)))))))"#,
    )
    .unwrap();
    let base = session.script_dir();
    std::fs::write(base.join("referee.wasm"), referee).unwrap();
    create(&registry, &mut session, json!({"name": "Referee", "components": {"scripts": [{"module": "referee.wasm"}]}}));
    let character = json!({"height": 1.8, "radius": 0.4, "maxSlope": 45, "stepHeight": 0.3});
    let hero = create(
        &registry,
        &mut session,
        json!({"name": "Hero", "components": {"characterBody": character, "playerControl": {"speed": 4, "jumpSpeed": 6, "turnToMove": true, "camera": "none", "cameraDistance": 6, "cameraHeight": 3}}}),
    );
    registry.execute(&mut session, "scene.prefab.create", &json!({"id": hero, "path": "hero.bprefab"})).unwrap();
    registry.execute(&mut session, "scene.object.delete", &json!({"id": hero})).unwrap();
    // The start is in the air with nothing below: the player falls and starts over.
    let start = json!({"position": [40, 2, 0], "rotation": [0, 90, 0], "scale": [1, 1, 1]});
    create(&registry, &mut session, json!({"name": "Start", "components": {"playerStart": {}, "transform": start}}));
    let rules = json!({"playerPrefab": "hero.bprefab", "respawn": true, "respawnHeight": -5, "timeLimit": 3, "timeUp": "win"});
    create(&registry, &mut session, json!({"name": "Rules", "components": {"gameRules": rules}}));
    let edited = session.scene().to_doc();
    let players = |s: &Session| {
        s.scene().world_matrices().into_iter().filter(|(id, _)| s.scene().get::<bayleaf_scene::components::PlayerControl>(*id).is_some()).collect::<Vec<_>>()
    };

    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    let placed = players(&session);
    assert_eq!(placed.len(), 1, "the rules placed one player");
    let (player, world) = placed[0];
    assert!((world.w_axis.x - 40.0).abs() < 1e-6, "at the Player Start: {:?}", world.w_axis);
    let state = registry.execute(&mut session, "game.state", &Value::Null).unwrap();
    assert!(state["game"].as_object().unwrap().is_empty() && state["players"].as_array().unwrap().is_empty() && state["outcome"].is_null(), "{state}");
    assert!(
        registry.execute(&mut session, "game.state.set", &json!({"name": "lives", "value": 3, "player": bayleaf_core::StableId::random().unwrap()})).is_err(),
        "only players have a player state"
    );
    let state = registry.execute(&mut session, "game.state.set", &json!({"name": "lives", "value": 3, "player": player})).unwrap();
    assert_eq!(state["players"][0]["values"]["lives"], 3.0);

    registry.execute(&mut session, "play.step", &json!({"seconds": 0.05, "steps": 40})).unwrap();
    assert!(session.console().iter().any(|l| l.text.contains("starts over")), "the fall was noticed: {:?}", session.console());
    let y = session.scene().world_matrix(player).unwrap().w_axis.y;
    assert!(y > -5.0, "and the player is back above the line: {y}");
    assert!(registry.execute(&mut session, "game.state", &Value::Null).unwrap()["outcome"].is_null(), "the game still runs");

    registry.execute(&mut session, "play.step", &json!({"seconds": 0.05, "steps": 30})).unwrap();
    let state = registry.execute(&mut session, "game.state", &Value::Null).unwrap();
    assert_eq!(state["outcome"], "won", "time ran out, which wins here: {state}");
    assert_eq!(state["game"]["cheered"], 1.0, "scripts heard gameWon");
    assert_eq!(session.console().iter().filter(|l| l.text == "Game won.").count(), 1, "announced once");
    assert!(session.console().iter().all(|l| l.level != crate::Level::Error), "{:?}", session.console());

    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
    assert_eq!(session.scene().to_doc(), edited, "stop takes the placed player away");
    assert!(registry.execute(&mut session, "game.state", &Value::Null).unwrap()["game"].as_object().unwrap().is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

/// Saved values last from one play to the next in the project, a broken file is never written
/// over, and agents may change them only inside their write folders.
#[test]
fn saved_values_last_between_plays_and_stay_inside_the_confinement() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-saved-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    registry.execute(&mut session, "project.new", &json!({"path": dir})).unwrap();
    // Counts the plays in a saved value.
    let counter = wat::parse_str(
        r#"(module
            (import "bayleaf" "state_get" (func $get (param i32 i32 i32 i32 i32) (result i32)))
            (import "bayleaf" "state_set" (func $set (param i32 i32 i32 i32 f64) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 100) "plays")
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_start")
              (drop (call $get (i32.const 2) (i32.const 0) (i32.const 100) (i32.const 5) (i32.const 200)))
              (drop (call $set (i32.const 2) (i32.const 0) (i32.const 100) (i32.const 5) (f64.add (f64.load (i32.const 200)) (f64.const 1))))))"#,
    )
    .unwrap();
    std::fs::write(session.script_dir().join("counter.wasm"), counter).unwrap();
    create(&registry, &mut session, json!({"name": "Counter", "components": {"scripts": [{"module": "counter.wasm"}]}}));
    let plays = |registry: &CommandRegistry, session: &mut Session| registry.execute(session, "game.state", &Value::Null).unwrap()["saved"]["plays"].clone();

    assert!(plays(&registry, &mut session).is_null(), "nothing saved yet");
    for _ in 0..2 {
        registry.execute(&mut session, "play.start", &Value::Null).unwrap();
        registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
    }
    assert_eq!(plays(&registry, &mut session), 2.0, "each play counted on from the last");

    // A file that does not read is reported and left as it is.
    let file = dir.join(crate::saved::SAVED_FILE);
    std::fs::write(&file, "not json").unwrap();
    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
    assert!(session.console().iter().any(|l| l.level == crate::Level::Error && l.text.contains("could not be read")));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "not json", "not written over");
    assert_eq!(registry.execute(&mut session, "game.saved.clear", &Value::Null).unwrap()["removed"], true);
    assert!(!file.exists());

    // An agent that may write only the scenes folder may not keep or clear saved values.
    let scenes = dir.join("scenes");
    session.set_confinement(Some(crate::confine::Confinement::new(std::slice::from_ref(&dir), std::slice::from_ref(&scenes)).unwrap()));
    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
    assert!(!file.exists(), "nothing written outside the write folders");
    assert!(session.console().iter().any(|l| l.text.contains("could not be written")));
    assert!(registry.execute(&mut session, "game.saved.clear", &Value::Null).is_err(), "clearing is refused too");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn asset_create_makes_each_kind_that_opens_and_never_writes_over_a_file() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-asset-create-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    registry.execute(&mut session, "project.new", &json!({"path": dir})).unwrap();
    let create = |session: &mut Session, kind: &str, folder: &str, name: &str| {
        registry.execute(session, "asset.create", &json!({"kind": kind, "folder": folder, "name": name}))
    };

    let folder = create(&mut session, "folder", "assets", "Levels").unwrap();
    assert!(std::path::Path::new(folder["path"].as_str().unwrap()).is_dir());
    let scene = create(&mut session, "scene", "assets/Levels", "Level 1").unwrap();
    let scene = std::path::PathBuf::from(scene["path"].as_str().unwrap());
    assert!(bayleaf_scene::Scene::load(&scene).unwrap().object_count() > 0, "a new scene has a camera and a light to start from");
    for (kind, name) in [("material", "Rock"), ("visualScript", "Door")] {
        let made = create(&mut session, kind, "assets/Levels", name).unwrap();
        registry.execute(&mut session, "graph.open", &json!({"path": made["path"]})).unwrap();
    }
    assert!(create(&mut session, "scene", "assets/Levels", "Level 1.bscene").is_err(), "an existing file is never written over");
    assert!(create(&mut session, "folder", "assets", "a/b").is_err(), "a name is one file, not a path");
    assert!(create(&mut session, "folder", "missing", "X").is_err(), "the folder must exist");

    // Agents create only inside their write folders.
    let scenes = dir.join("scenes");
    session.set_confinement(Some(crate::confine::Confinement::new(std::slice::from_ref(&dir), std::slice::from_ref(&scenes)).unwrap()));
    assert!(create(&mut session, "material", "assets", "Outside").is_err());
    assert!(!dir.join("assets/Outside.bmaterial").exists());
    assert!(create(&mut session, "scene", "scenes", "Inside").is_ok());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn rust_scripts_in_a_project_share_one_workspace_and_build() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-script-workspace-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    registry.execute(&mut session, "project.new", &json!({"path": dir})).unwrap();
    for name in ["Turn Table", "Door Opener"] {
        let made = registry.execute(&mut session, "script.new", &json!({"folder": "scripts", "name": name})).unwrap();
        assert!(made["workspace"].is_string(), "a project's script joins its workspace");
    }
    let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
    let workspace: toml_edit::DocumentMut = manifest.parse().unwrap();
    let members: Vec<&str> = workspace["workspace"]["members"].as_array().unwrap().iter().filter_map(|m| m.as_str()).collect();
    assert_eq!(members, ["scripts/turn_table", "scripts/door_opener"]);
    let script: toml_edit::DocumentMut = std::fs::read_to_string(dir.join("scripts/door_opener/Cargo.toml")).unwrap().parse().unwrap();
    assert!(script.get("workspace").is_none() && script.get("profile").is_none(), "settings live in the workspace, once");

    // A Cargo.toml that is no workspace is left alone, and the script is not made.
    let other = std::env::temp_dir().join(format!("bayleaf-script-no-workspace-{}", std::process::id()));
    std::fs::remove_dir_all(&other).ok();
    let mut elsewhere = Session::default();
    registry.execute(&mut elsewhere, "project.new", &json!({"path": other})).unwrap();
    std::fs::write(other.join("Cargo.toml"), "[package]\nname = \"game\"\n").unwrap();
    assert!(registry.execute(&mut elsewhere, "script.new", &json!({"folder": "scripts", "name": "Mover"})).is_err());
    assert!(!other.join("scripts/mover").exists(), "nothing is left behind");
    assert_eq!(std::fs::read_to_string(other.join("Cargo.toml")).unwrap(), "[package]\nname = \"game\"\n");
    std::fs::remove_dir_all(&other).ok();

    // A member builds on its own, with the workspace's settings.
    let target = std::env::temp_dir().join("bayleaf-script-build-cache");
    match crate::code_scripts::build_script(&dir.join("scripts/turn_table"), &target) {
        Ok(built) => assert!(built.module.is_file()),
        Err(error) if error.contains("rustup target add") || error.contains("needs Rust") => eprintln!("skipped: {error}"),
        Err(error) => panic!("{error}"),
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn asset_delete_removes_files_and_scripts_from_the_workspace_but_never_the_project() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-asset-delete-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    registry.execute(&mut session, "project.new", &json!({"path": dir})).unwrap();
    for name in ["Keep Me", "Drop Me"] {
        registry.execute(&mut session, "script.new", &json!({"folder": "scripts", "name": name})).unwrap();
    }
    let delete = |session: &mut Session, path: &str| registry.execute(session, "asset.delete", &json!({"path": path, "permanent": true}));

    delete(&mut session, "materials/ground.bmaterial").unwrap();
    assert!(!dir.join("materials/ground.bmaterial").exists());
    delete(&mut session, "scripts/drop_me").unwrap();
    assert!(!dir.join("scripts/drop_me").exists());
    let workspace: toml_edit::DocumentMut = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap().parse().unwrap();
    let members: Vec<&str> = workspace["workspace"]["members"].as_array().unwrap().iter().filter_map(|m| m.as_str()).collect();
    assert_eq!(members, ["scripts/keep_me"], "a deleted script leaves the workspace, the others stay");

    assert!(delete(&mut session, ".").is_err(), "never the project folder");
    assert!(delete(&mut session, "missing.txt").is_err());
    // Agents delete only inside their write folders.
    let scenes = dir.join("scenes");
    session.set_confinement(Some(crate::confine::Confinement::new(std::slice::from_ref(&dir), std::slice::from_ref(&scenes)).unwrap()));
    assert!(delete(&mut session, "scripts/spin.bscript").is_err());
    assert!(dir.join("scripts/spin.bscript").exists());
    std::fs::remove_dir_all(&dir).ok();
}

/// A module that writes the value of `action` into the game value `key` every step.
fn action_reader(pairs: &[(&str, &str)]) -> Vec<u8> {
    let mut data = String::new();
    let mut body = String::new();
    for (index, (action, key)) in pairs.iter().enumerate() {
        let (a, k) = (100 + index * 64, 132 + index * 64);
        data.push_str(&format!("(data (i32.const {a}) \"{action}\") (data (i32.const {k}) \"{key}\")\n"));
        body.push_str(&format!(
            "(drop (call $set (i32.const 0) (i32.const 0) (i32.const {k}) (i32.const {}) (f64.promote_f32 (call $action (i32.const {a}) (i32.const {})))))\n",
            key.len(),
            action.len()
        ));
    }
    wat::parse_str(format!(
        r#"(module
            (import "bayleaf" "action" (func $action (param i32 i32) (result f32)))
            (import "bayleaf" "state_set" (func $set (param i32 i32 i32 i32 f64) (result i32)))
            (memory (export "memory") 1)
            {data}
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_update") (param f32) {body}))"#
    ))
    .unwrap()
}

#[test]
fn input_maps_answer_per_object_count_triggers_once_and_switch() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-input-maps-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    registry.execute(&mut session, "project.new", &json!({"path": dir})).unwrap();
    let car = json!({"start": "", "maps": [{"name": "Car", "actions": [{"name": "Horn", "kind": "trigger", "bindings": ["key:H"]}]}]});
    registry.execute(&mut session, "input.asset.set", &json!({"path": "input/car.binput", "asset": car})).unwrap();
    std::fs::write(dir.join("walker.wasm"), action_reader(&[("Jump", "walkerJump")])).unwrap();
    std::fs::write(dir.join("driver.wasm"), action_reader(&[("Horn", "driverHorn"), ("Jump", "driverJump")])).unwrap();
    create(&registry, &mut session, json!({"name": "Walker", "components": {"scripts": [{"module": "walker.wasm"}]}}));
    let driver = create(
        &registry,
        &mut session,
        json!({"name": "Driver", "components": {"scripts": [{"module": "driver.wasm"}], "inputMap": {"asset": "input/car.binput", "start": ""}}}),
    );
    let game =
        |registry: &CommandRegistry, session: &mut Session, key: &str| registry.execute(session, "game.state", &Value::Null).unwrap()["game"][key].as_f64();

    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    registry.execute(&mut session, "play.input", &json!({"keys": ["Space", "H"]})).unwrap();
    registry.execute(&mut session, "play.step", &Value::Null).unwrap();
    assert_eq!(game(&registry, &mut session, "walkerJump"), Some(1.0), "the project's map: Player/Jump");
    assert_eq!(game(&registry, &mut session, "driverHorn"), Some(1.0), "an object's own map comes first");
    assert_eq!(game(&registry, &mut session, "driverJump"), Some(0.0), "and its own map has no Jump");
    registry.execute(&mut session, "play.step", &Value::Null).unwrap();
    assert_eq!(game(&registry, &mut session, "walkerJump"), Some(0.0), "a trigger counts in one step only");

    let state = registry.execute(&mut session, "input.map.set", &json!({"map": "Menu"})).unwrap();
    assert_eq!(state["project"]["active"], "Menu");
    assert_eq!(state["objects"][0]["object"], driver.as_str());
    assert!(registry.execute(&mut session, "input.map.set", &json!({"map": "Nowhere"})).is_err());
    registry.execute(&mut session, "play.input", &json!({"keys": []})).unwrap();
    registry.execute(&mut session, "play.step", &Value::Null).unwrap();
    registry.execute(&mut session, "play.input", &json!({"keys": ["Space"]})).unwrap();
    registry.execute(&mut session, "play.step", &Value::Null).unwrap();
    assert_eq!(game(&registry, &mut session, "walkerJump"), Some(0.0), "Player's actions are off while Menu is active");
    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();

    // Assets are checked, and agents write them only inside their folders.
    let bad = json!({"start": "", "maps": [{"name": "Player", "actions": [{"name": "Jump/High", "kind": "held", "bindings": []}]}]});
    assert!(registry.execute(&mut session, "input.asset.set", &json!({"path": "input/bad.binput", "asset": bad})).is_err(), "an action name is one name");
    // A version 1 file, with its actions named map/action in one list, still reads, sorted into maps.
    let flat = json!({"format": "bayleaf.input", "version": 1, "start": "", "actions": [{"name": "Boat/Row", "kind": "value", "bindings": ["key:R"]}]});
    std::fs::write(dir.join("input/old.binput"), flat.to_string()).unwrap();
    let old = registry.execute(&mut session, "input.asset", &json!({"path": "input/old.binput"})).unwrap();
    assert_eq!((&old["asset"]["maps"][0]["name"], &old["asset"]["maps"][0]["actions"][0]["name"]), (&json!("Boat"), &json!("Row")));
    let scenes = dir.join("scenes");
    session.set_confinement(Some(crate::confine::Confinement::new(std::slice::from_ref(&dir), std::slice::from_ref(&scenes)).unwrap()));
    assert!(registry.execute(&mut session, "input.asset.set", &json!({"path": "input/car.binput", "asset": car})).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn opening_an_old_project_moves_its_inline_actions_into_an_input_map() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-input-migrate-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    registry.execute(&mut session, "project.new", &json!({"path": dir})).unwrap();
    // A project file as format 2 wrote it.
    let file = crate::project_cmds::project_file_in(&dir).unwrap();
    let old = json!({
        "format": "bayleaf.project", "version": 2, "name": "Old", "mainScene": "scenes/main.bscene",
        "settings": {"input": {"actions": [{"name": "Jump", "bindings": ["key:Space"]}]}}
    });
    std::fs::write(&file, old.to_string()).unwrap();
    let mut session = Session::default();
    registry.execute(&mut session, "project.open", &json!({"path": dir})).unwrap();
    let settings = registry.execute(&mut session, "project.settings", &Value::Null).unwrap();
    assert_eq!(settings["settings"]["input"]["map"], "input/actions.binput");
    let moved = registry.execute(&mut session, "input.asset", &json!({"path": "input/actions.binput"})).unwrap();
    assert_eq!(moved["asset"]["maps"][0]["name"], "Default", "the actions go into the map Default");
    assert_eq!(moved["asset"]["maps"][0]["actions"][0]["name"], "Jump", "keeping their names");
    assert!(std::fs::read_to_string(&file).unwrap().contains("\"version\": 4"), "written at the current project version");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn asset_rename_keeps_extensions_moves_script_crates_and_follows_open_files() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-asset-rename-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    registry.execute(&mut session, "project.new", &json!({"path": dir})).unwrap();
    let rename = |session: &mut Session, path: &str, name: &str| registry.execute(session, "asset.rename", &json!({"path": path, "name": name}));

    registry.execute(&mut session, "graph.open", &json!({"path": dir.join("materials/ground.bmaterial")})).unwrap();
    let renamed = rename(&mut session, "materials/ground.bmaterial", "Grass").unwrap();
    assert!(renamed["path"].as_str().unwrap().ends_with("Grass.bmaterial"), "the extension stays: {renamed}");
    assert!(dir.join("materials/Grass.bmaterial").is_file() && !dir.join("materials/ground.bmaterial").exists());
    let open = registry.execute(&mut session, "graph.get", &Value::Null).unwrap();
    assert!(open["path"].as_str().unwrap().ends_with("Grass.bmaterial"), "the open graph follows its file");

    registry.execute(&mut session, "script.new", &json!({"folder": "scripts", "name": "Mover"})).unwrap();
    std::fs::write(dir.join("scripts/mover.wasm"), b"\0asm").unwrap();
    rename(&mut session, "scripts/mover", "Door Opener").unwrap();
    assert!(dir.join("scripts/door_opener/src/lib.rs").is_file() && dir.join("scripts/door_opener.wasm").is_file(), "the folder and its module move");
    let crate_manifest: toml_edit::DocumentMut = std::fs::read_to_string(dir.join("scripts/door_opener/Cargo.toml")).unwrap().parse().unwrap();
    assert_eq!(crate_manifest["package"]["name"].as_str(), Some("door_opener"));
    let workspace: toml_edit::DocumentMut = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap().parse().unwrap();
    assert_eq!(workspace["workspace"]["members"].as_array().unwrap().iter().filter_map(|m| m.as_str()).collect::<Vec<_>>(), ["scripts/door_opener"]);

    assert!(rename(&mut session, "scripts/spin.bscript", "door_opener.wasm").is_ok(), "a different extension is kept as part of the name");
    assert!(rename(&mut session, "materials/Grass.bmaterial", "a/b").is_err());
    std::fs::write(dir.join("materials/Rock.bmaterial"), "{}").unwrap();
    assert!(rename(&mut session, "materials/Grass.bmaterial", "Rock").is_err(), "never over another file");
    let scenes = dir.join("scenes");
    session.set_confinement(Some(crate::confine::Confinement::new(std::slice::from_ref(&dir), std::slice::from_ref(&scenes)).unwrap()));
    assert!(rename(&mut session, "materials/Grass.bmaterial", "Moss").is_err(), "agents rename only inside their folders");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn visual_script_variables_are_declared_renamed_with_their_nodes_undone_and_saved() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    registry.execute(&mut session, "graph.new", &json!({"domain": "script"})).unwrap();
    registry.execute(&mut session, "graph.variable.add", &json!({"name": "lives", "type": "number", "start": 3})).unwrap();
    let node = registry.execute(&mut session, "graph.node.add", &json!({"kind": "setVariable", "position": [0, 0]})).unwrap()["node"].clone();
    registry.execute(&mut session, "graph.node.set", &json!({"node": node, "name": "name", "value": "lives"})).unwrap();
    assert!(registry.execute(&mut session, "graph.variable.add", &json!({"name": "lives", "type": "vector"})).is_err(), "names are unique");
    assert!(registry.execute(&mut session, "graph.variable.add", &json!({"name": "on", "type": "trueFalse", "start": 2})).is_err(), "the start fits the type");

    registry.execute(&mut session, "graph.variable.set", &json!({"name": "lives", "rename": "hearts", "start": 5})).unwrap();
    let graph = registry.execute(&mut session, "graph.get", &Value::Null).unwrap()["graph"].clone();
    assert_eq!(graph["variables"][0], json!({"name": "hearts", "type": "number", "start": 5}));
    let set_node = graph["nodes"].as_array().unwrap().iter().find(|n| n["kind"] == "setVariable").unwrap();
    assert_eq!(set_node["values"]["name"], "hearts", "its nodes follow the new name");
    registry.execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert_eq!(registry.execute(&mut session, "graph.get", &Value::Null).unwrap()["graph"]["variables"][0]["name"], "lives", "undo brings the old name back");

    let path = std::env::temp_dir().join(format!("bayleaf-variables-{}.bscript", std::process::id()));
    registry.execute(&mut session, "graph.save", &json!({"path": path})).unwrap();
    registry.execute(&mut session, "graph.open", &json!({"path": path})).unwrap();
    assert_eq!(registry.execute(&mut session, "graph.get", &Value::Null).unwrap()["graph"]["variables"][0]["start"], 3, "variables are saved with the graph");
    registry.execute(&mut session, "graph.variable.remove", &json!({"name": "lives"})).unwrap();
    std::fs::remove_file(&path).ok();

    registry.execute(&mut session, "graph.new", &json!({"domain": "material"})).unwrap();
    assert!(registry.execute(&mut session, "graph.variable.add", &json!({"name": "x", "type": "number"})).is_err(), "materials have no variables");
}

#[test]
fn a_visual_scripts_menu_path_is_a_checked_field_that_undoes() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    registry.execute(&mut session, "graph.new", &json!({"domain": "script"})).unwrap();
    let set = registry.execute(&mut session, "graph.menu.set", &json!({"path": " Light / Flicker "})).unwrap();
    assert_eq!(set["path"], "Light/Flicker", "parts are trimmed");
    let menu = |session: &mut Session| registry.execute(session, "graph.get", &Value::Null).unwrap()["graph"]["menu"].clone();
    assert_eq!(menu(&mut session), "Light/Flicker");
    assert!(registry.execute(&mut session, "graph.menu.set", &json!({"path": "Light//Flicker"})).is_err(), "an empty part is refused");
    registry.execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert_eq!(menu(&mut session), Value::Null, "undo takes the path away again");
    registry.execute(&mut session, "graph.new", &json!({"domain": "material"})).unwrap();
    assert!(registry.execute(&mut session, "graph.menu.set", &json!({"path": "Light/Flicker"})).is_err(), "materials have no menu path");
}

#[test]
fn set_component_changes_a_field_in_play_and_stop_restores_it() {
    use bayleaf_graph::{Graph, Link, PinRef};
    let catalog = &bayleaf_script::visual::CATALOG;
    let mut graph = Graph::new(catalog);
    let update = graph.nodes.iter().find(|n| n.kind == "update").unwrap().id;
    let set = graph.add_node(catalog, "setComponent", [0.0, 0.0]).unwrap();
    let number = graph.add_node(catalog, "number", [0.0, 0.0]).unwrap();
    graph.set_value(catalog, number, "value", json!(7.0)).unwrap();
    graph.connect(catalog, Link { from: PinRef::new(update, "then"), to: PinRef::new(set, "run") }).unwrap();
    graph.connect(catalog, Link { from: PinRef::new(number, "value"), to: PinRef::new(set, "intensity") }).unwrap();
    let bytes = bayleaf_script::visual::compile(&graph).unwrap();
    let (registry, mut session, id, dir) = scripted_scene("fields", &bytes);
    registry.execute(&mut session, "scene.component.add", &json!({"id": id, "component": "pointLight"})).unwrap();
    let object = bayleaf_core::StableId::parse_hex(&id).unwrap();
    let intensity = |session: &Session| session.scene().component(object, "pointLight").unwrap()["intensity"].as_f64().unwrap();
    let before = intensity(&session);
    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    registry.execute(&mut session, "play.step", &json!({"seconds": 0.1, "steps": 2})).unwrap();
    assert!((intensity(&session) - 7.0).abs() < 1e-6, "the script set the light's intensity");
    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
    assert_eq!(intensity(&session), before, "stopping play puts the light back");
    std::fs::remove_dir_all(&dir).ok();
}

/// A visual script with an On Start chain of `actions`, each `(kind, settings)`, saved as `file`.
fn script_file(file: &std::path::Path, actions: &[(&str, Value)]) {
    use bayleaf_graph::{Graph, Link, PinRef};
    let catalog = &bayleaf_script::visual::CATALOG;
    let mut graph = Graph::empty(catalog);
    let mut previous = (graph.add_node(catalog, "start", [0.0, 0.0]).unwrap(), "then");
    for (kind, settings) in actions {
        let node = graph.add_node(catalog, kind, [0.0, 0.0]).unwrap();
        for (name, value) in settings.as_object().cloned().unwrap_or_default() {
            graph.set_value(catalog, node, &name, value).unwrap();
        }
        graph.connect(catalog, Link { from: PinRef::new(previous.0, previous.1), to: PinRef::new(node, "run") }).unwrap();
        previous = (node, "then");
    }
    std::fs::write(file, graph.to_json()).unwrap();
}

#[test]
fn scenes_load_in_play_added_or_in_place_and_added_ones_unload() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-scene-load-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    // The room scene: one object whose script says it started.
    script_file(&dir.join("room.bscript"), &[("log", json!({ "message": "room started" }))]);
    create(&registry, &mut session, json!({"name": "Room", "components": {"scripts": [{"module": "room.bscript"}]}}));
    registry.execute(&mut session, "scene.saveAs", &json!({"path": dir.join("room.bscene")})).unwrap();
    // The hub scene: its script adds the room when play starts.
    registry.execute(&mut session, "scene.new", &Value::Null).unwrap();
    script_file(&dir.join("hub.bscript"), &[("loadScene", json!({ "scene": "room.bscene", "additive": true }))]);
    create(&registry, &mut session, json!({"name": "Hub", "components": {"scripts": [{"module": "hub.bscript"}]}}));
    registry.execute(&mut session, "scene.saveAs", &json!({"path": dir.join("hub.bscene")})).unwrap();
    let edited = session.scene().to_doc();
    let names = |session: &Session| -> Vec<String> {
        session.scene().world_matrices().iter().filter_map(|(id, _)| session.scene().name(*id).map(str::to_owned)).collect()
    };
    let room_starts = |session: &Session| session.console().iter().filter(|l| l.text.contains("room started")).count();

    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    registry.execute(&mut session, "play.step", &json!({"seconds": 0.1})).unwrap();
    assert_eq!(names(&session), ["Hub", "Room"], "the hub's Load Scene added the room");
    assert_eq!(room_starts(&session), 1, "the added room's script started");
    let listed = registry.execute(&mut session, "play.scene.list", &Value::Null).unwrap();
    assert_eq!(listed["added"][0]["path"], "room.bscene");

    let removed = registry.execute(&mut session, "play.scene.unload", &json!({"path": "room.bscene"})).unwrap();
    assert_eq!(removed["removed"], 1);
    assert_eq!(names(&session), ["Hub"], "unloading removes what was added");
    assert!(registry.execute(&mut session, "play.scene.unload", &json!({"path": "room.bscene"})).is_err(), "nothing left to unload");

    // In place of the playing scene: the room alone, its script starts, the game's values stay.
    registry.execute(&mut session, "game.state.set", &json!({"name": "coins", "value": 3})).unwrap();
    registry.execute(&mut session, "play.scene.load", &json!({"path": "room.bscene"})).unwrap();
    assert_eq!(names(&session), ["Room"]);
    assert_eq!(room_starts(&session), 2);
    assert_eq!(registry.execute(&mut session, "game.state", &Value::Null).unwrap()["game"]["coins"], 3.0, "the game's values carry over");
    assert!(registry.execute(&mut session, "play.scene.load", &json!({"path": "room.bmaterial"})).is_err(), "only scene files load");

    // An agent cannot load a scene from outside its folders.
    let outside = std::env::temp_dir().join(format!("bayleaf-scene-load-outside-{}", std::process::id()));
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::copy(dir.join("room.bscene"), outside.join("room.bscene")).unwrap();
    session.set_confinement(Some(crate::confine::Confinement::new(std::slice::from_ref(&dir), std::slice::from_ref(&dir)).unwrap()));
    let escape = format!("../{}/room.bscene", outside.file_name().unwrap().to_string_lossy());
    assert!(registry.execute(&mut session, "play.scene.load", &json!({"path": escape, "additive": true})).is_err(), "outside the readable folders");
    session.set_confinement(None);

    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
    assert_eq!(session.scene().to_doc(), edited, "stopping brings back the scene as edited");
    std::fs::remove_dir_all(&dir).ok();
    std::fs::remove_dir_all(&outside).ok();
}

#[test]
fn agents_read_the_console_newest_last_and_can_ask_for_errors_only() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    for n in 0..5 {
        session.log_line(crate::Level::Info, format!("note {n}"));
    }
    session.log_line(crate::Level::Error, "it broke".to_owned());
    let read = registry.execute(&mut session, "console.read", &json!({"last": 2})).unwrap();
    assert_eq!(read["lines"], json!([{"level": "info", "text": "note 4"}, {"level": "error", "text": "it broke"}]));
    let errors = registry.execute(&mut session, "console.read", &json!({"errors": true})).unwrap();
    assert_eq!(errors["total"], 1);
    assert_eq!(errors["lines"][0]["text"], "it broke");
}

#[test]
fn agents_list_a_projects_files_as_the_project_panel_does_inside_their_folders() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-asset-list-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    registry.execute(&mut session, "project.new", &json!({"path": dir})).unwrap();
    std::fs::create_dir_all(dir.join("target")).unwrap();
    std::fs::write(dir.join(".hidden"), "x").unwrap();
    let top = registry.execute(&mut session, "asset.list", &Value::Null).unwrap();
    let paths: Vec<&str> = top["entries"].as_array().unwrap().iter().filter_map(|e| e["path"].as_str()).collect();
    assert!(paths.contains(&"materials") && !paths.iter().any(|p| p.contains("target") || p.contains(".hidden")), "{paths:?}");
    let all = registry.execute(&mut session, "asset.list", &json!({"recursive": true})).unwrap();
    assert!(all["entries"].as_array().unwrap().iter().any(|e| e["kind"] == "Material" && e["path"].as_str().unwrap().starts_with("materials/")), "{all}");
    session.set_confinement(Some(
        crate::confine::Confinement::new(std::slice::from_ref(&dir.join("materials")), std::slice::from_ref(&dir.join("materials"))).unwrap(),
    ));
    assert!(registry.execute(&mut session, "asset.list", &json!({"path": "scenes"})).is_err(), "outside the readable folders");
    assert!(registry.execute(&mut session, "asset.list", &json!({"path": "materials"})).is_ok());
    std::fs::remove_dir_all(&dir).ok();
}

/// A mono 16-bit WAV file of `seconds` at 8 kHz.
#[cfg(feature = "audio")]
fn beep(seconds: f64) -> Vec<u8> {
    let frames = (8_000.0 * seconds) as u32;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + frames * 2).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    for value in [1u16, 1] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in [8_000u32, 16_000] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in [2u16, 16] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(frames * 2).to_le_bytes());
    for _ in 0..frames {
        bytes.extend_from_slice(&8_000i16.to_le_bytes());
    }
    bytes
}

#[cfg(feature = "audio")]
#[test]
fn sounds_play_from_audio_sources_scripts_and_agents_silently_in_tests() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-sound-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("hum.wav"), beep(1.0)).unwrap();
    std::fs::write(dir.join("click.wav"), beep(0.25)).unwrap();
    script_file(&dir.join("clicker.bscript"), &[("playSoundAt", json!({ "sound": "click.wav" }))]);
    let speaker = create(
        &registry,
        &mut session,
        json!({"name": "Speaker", "components": {"audioSource": {"clip": "hum.wav", "volume": 1, "looping": true, "playOnStart": true, "spatial": true, "range": 30}}}),
    );
    create(&registry, &mut session, json!({"name": "Clicker", "components": {"scripts": [{"module": "clicker.bscript"}]}}));
    registry.execute(&mut session, "scene.saveAs", &json!({"path": dir.join("noise.bscene")})).unwrap();
    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    registry.execute(&mut session, "play.step", &json!({"seconds": 0.05})).unwrap();
    let playing = registry.execute(&mut session, "audio.playing", &Value::Null).unwrap();
    assert_eq!(playing["heard"], false, "tests play sound silently");
    let sources: Vec<&str> = playing["sounds"].as_array().unwrap().iter().filter_map(|s| s["source"].as_str()).collect();
    assert!(sources.contains(&"hum.wav") && sources.contains(&"click.wav"), "the Audio Source and the script's sound: {sources:?}");

    // The click is a quarter second long; the hum loops.
    registry.execute(&mut session, "play.step", &json!({"seconds": 0.5})).unwrap();
    let playing = registry.execute(&mut session, "audio.playing", &Value::Null).unwrap();
    assert_eq!(playing["sounds"].as_array().unwrap().len(), 1, "only the looping hum is left: {playing}");
    assert_eq!(registry.execute(&mut session, "audio.stop", &json!({"id": speaker})).unwrap()["stopped"], 1);
    assert!(registry.execute(&mut session, "audio.playing", &Value::Null).unwrap()["sounds"].as_array().unwrap().is_empty());
    registry.execute(&mut session, "audio.play", &json!({"id": speaker})).unwrap();
    assert_eq!(registry.execute(&mut session, "audio.playing", &Value::Null).unwrap()["sounds"][0]["object"], speaker.as_str());
    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
    std::fs::remove_dir_all(&dir).ok();
}

#[cfg(not(feature = "audio"))]
#[test]
fn a_build_without_sound_says_so() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    let error = registry.execute(&mut session, "audio.playing", &Value::Null).unwrap_err().to_string();
    assert!(error.contains("no sound"), "{error}");
}
