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
    assert!(registry.execute(&mut session, "project.settings.set", &json!({"settings": {"physics": {"enabled": false, "gravity": [0, 0, 0]}}})).is_err());

    let dir = std::env::temp_dir().join(format!("bayleaf-settings-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    registry.execute(&mut session, "project.new", &json!({"path": dir})).unwrap();
    let set = json!({"settings": {"physics": {"enabled": false, "gravity": [0, -3.7, 0]}}});
    registry.execute(&mut session, "project.settings.set", &set).unwrap();
    let bad = json!({"settings": {"physics": {"enabled": true, "gravity": [0, -1e9, 0]}}});
    assert!(registry.execute(&mut session, "project.settings.set", &bad).is_err(), "gravity has a limit");

    // A fresh session that opens the project reads them back from the file.
    let mut other = Session::default();
    registry.execute(&mut other, "project.open", &json!({"path": dir})).unwrap();
    let read = registry.execute(&mut other, "project.settings", &Value::Null).unwrap();
    assert_eq!(read["settings"]["physics"]["enabled"], false);
    assert!((read["settings"]["physics"]["gravity"][1].as_f64().unwrap() + 3.7).abs() < 1e-5);
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
