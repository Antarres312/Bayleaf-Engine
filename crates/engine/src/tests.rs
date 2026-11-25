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

    // Opening a graph that is open brings it forward, with its undo steps.
    registry.execute(&mut session, "graph.open", &json!({"path": dir.join("red.bmaterial")})).unwrap();
    assert_eq!(session.graph().unwrap().graph, saved);
    assert_eq!(session.graphs().len(), 1, "not opened twice");
    assert_eq!(session.history().len(), 3);
    assert!(registry.execute(&mut session, "graph.get", &Value::Null).unwrap()["problems"].as_array().unwrap().is_empty());
    // Closing a graph takes its undo steps with it, and keeps the scene's.
    registry.execute(&mut session, "graph.close", &Value::Null).unwrap();
    assert_eq!(session.history(), ["scene.object.create"]);
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
    for graphics in [json!({"shadowResolution": 3000}), json!({"shadowCascades": 0}), json!({"shadowLights": 9}), json!({"shadowDistance": -1})] {
        assert!(registry.execute(&mut session, "project.settings.set", &json!({"settings": {"graphics": graphics}})).is_err(), "{graphics} is refused");
    }
    let graphics = json!({"settings": {"physics": {"enabled": false, "gravity": [0, -3.7, 0]}, "editor": {"hideExtensions": false}, "graphics": {"shadowResolution": 4096, "shadowLights": 3}}});
    registry.execute(&mut session, "project.settings.set", &graphics).unwrap();

    // A fresh session that opens the project reads them back from the file.
    let mut other = Session::default();
    registry.execute(&mut other, "project.open", &json!({"path": dir})).unwrap();
    let read = registry.execute(&mut other, "project.settings", &Value::Null).unwrap();
    assert_eq!(read["settings"]["physics"]["enabled"], false);
    assert!((read["settings"]["physics"]["gravity"][1].as_f64().unwrap() + 3.7).abs() < 1e-5);
    assert_eq!(read["settings"]["editor"]["hideExtensions"], false);
    let shadows = crate::project_cmds::settings_of(&other).graphics.shadows();
    assert_eq!((shadows.resolution, shadows.lights), (4096, 3), "the views draw shadows as the project says");
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
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(written["version"], crate::project_cmds::PROJECT_VERSION, "written at the current project version");
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

/// The object named `name` in the session's scene, if any.
#[cfg(feature = "streaming")]
fn named(session: &Session, name: &str) -> Option<bayleaf_core::StableId> {
    session.scene().world_matrices().into_iter().map(|(id, _)| id).find(|id| session.scene().name(*id) == Some(name))
}

#[cfg(feature = "streaming")]
#[test]
fn a_split_world_streams_its_cells_in_and_out_around_the_camera() {
    use bayleaf_scene::components::Transform;
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-stream-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let place = |x: f64, z: f64| json!({"position": [x, 0.0, z], "rotation": [0.0, 0.0, 0.0], "scale": [1.0, 1.0, 1.0]});
    let camera = create(
        &registry,
        &mut session,
        json!({"name": "Camera", "components": {"transform": place(0.0, 0.0), "camera": serde_json::to_value(bayleaf_scene::components::Camera::default()).unwrap()}}),
    );
    create(&registry, &mut session, json!({"name": "Tree", "components": {"transform": place(30.0, 0.0)}}));
    create(&registry, &mut session, json!({"name": "Rock", "components": {"transform": place(210.0, 5.0)}}));
    create(
        &registry,
        &mut session,
        json!({"name": "Lost", "components": {"streamedArea": {"scene": "cells/missing.bscene", "loadDistance": 10, "keepMargin": 1}}}),
    );
    registry.execute(&mut session, "scene.saveAs", &json!({"path": dir.join("world.bscene")})).unwrap();

    assert!(registry.execute(&mut session, "stream.split", &json!({"cellSize": 100, "folder": "../outside"})).is_err());
    let split = registry.execute(&mut session, "stream.split", &json!({"cellSize": 100, "folder": "cells"})).unwrap();
    assert_eq!(split["moved"], 2, "the camera and the area stay: {split}");
    assert_eq!(split["areas"].as_array().unwrap().len(), 2, "one area per cell with objects");
    assert!(named(&session, "Tree").is_none() && named(&session, "Camera").is_some());
    assert!(dir.join("cells/cell_0_0.bscene").is_file() && dir.join("cells/cell_2_0.bscene").is_file());
    assert!(registry.execute(&mut session, "stream.split", &json!({"cellSize": 100, "folder": "cells"})).is_err(), "nothing left to split");

    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    registry.execute(&mut session, "stream.wait", &json!({})).unwrap();
    let areas = registry.execute(&mut session, "stream.areas", &Value::Null).unwrap();
    let state = |areas: &Value, scene: &str| areas["areas"].as_array().unwrap().iter().find(|a| a["scene"] == scene).map(|a| a["state"].clone()).unwrap();
    assert_eq!(state(&areas, "cells/cell_0_0.bscene"), "here", "the near cell came: {areas}");
    assert_eq!(state(&areas, "cells/cell_2_0.bscene"), "away", "the far one did not");
    assert_eq!(state(&areas, "cells/missing.bscene"), "failed");
    assert!(session.console.iter().any(|l| l.text.contains("missing.bscene could not be added")), "the failure is in the console");
    let tree = named(&session, "Tree").expect("the tree came back with its cell");
    let at = session.scene().world_matrix(tree).unwrap().w_axis.truncate();
    assert!(at.distance(bayleaf_core::math::DVec3::new(30.0, 0.0, 0.0)) < 1e-9, "where it was before the split: {at}");

    let camera = bayleaf_core::StableId::parse_hex(&camera).unwrap();
    session.scene.set(camera, Transform { position: [250.0, 0.0, 50.0], ..Transform::default() }).unwrap();
    registry.execute(&mut session, "play.step", &json!({"seconds": 0.02})).unwrap();
    registry.execute(&mut session, "stream.wait", &json!({})).unwrap();
    let areas = registry.execute(&mut session, "stream.areas", &Value::Null).unwrap();
    assert_eq!(state(&areas, "cells/cell_0_0.bscene"), "away", "the cell left behind went: {areas}");
    assert_eq!(state(&areas, "cells/cell_2_0.bscene"), "here");
    assert!(named(&session, "Tree").is_none() && named(&session, "Rock").is_some());

    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
    assert!(named(&session, "Rock").is_none(), "stop brings back the scene without streamed objects");
    registry.execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert!(named(&session, "Tree").is_some() && named(&session, "Streamed World").is_none(), "the split undoes");
    std::fs::remove_dir_all(&dir).ok();
}

#[cfg(not(feature = "streaming"))]
#[test]
fn a_build_without_streaming_says_so() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    create(&registry, &mut session, json!({"name": "Area", "components": {"streamedArea": {"scene": "a.bscene", "loadDistance": 10, "keepMargin": 1}}}));
    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    assert!(registry.execute(&mut session, "stream.areas", &Value::Null).is_err());
    assert!(session.console.iter().any(|l| l.text.contains("no world streaming")));
    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
}

#[test]
fn a_project_exports_as_a_game_folder_that_runs_without_the_editor() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let base = std::env::temp_dir().join(format!("bayleaf-export-{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    let project = base.join("Space Race");
    registry.execute(&mut session, "project.new", &json!({"path": project})).unwrap();
    // A Rust script's crate (its built module beside it), saved values, and a note the game does not use.
    std::fs::create_dir_all(project.join("scripts/mover/src")).unwrap();
    std::fs::write(project.join("scripts/mover/Cargo.toml"), "[package]").unwrap();
    std::fs::write(project.join("scripts/mover/src/lib.rs"), "// source").unwrap();
    std::fs::write(project.join("scripts/mover.wasm"), b"\0asm\x01\0\0\0").unwrap();
    std::fs::create_dir_all(project.join("saves")).unwrap();
    std::fs::write(project.join("saves/progress.json"), "{}").unwrap();
    std::fs::write(project.join("notes.txt"), "todo").unwrap();
    let player = base.join("player-program");
    std::fs::write(&player, b"the player").unwrap();
    let out = base.join("out");

    let missing = registry.execute(&mut session, "project.export", &json!({"folder": out, "player": base.join("nothing")}));
    assert!(matches!(missing, Err(CommandError::Failed(m)) if m.contains("player program")), "a missing player says so");
    assert!(registry.execute(&mut session, "project.export", &json!({"folder": project, "player": player})).is_err(), "not into the project itself");

    let result = registry.execute(&mut session, "project.export", &json!({"folder": out, "player": player})).unwrap();
    let game = out.join("Space Race");
    let program = game.join(format!("Space Race{}", std::env::consts::EXE_SUFFIX));
    assert_eq!(std::fs::read(&program).unwrap(), b"the player", "the player under the game's name");
    // The game's files are in one pack beside the program, not readable one by one.
    assert!(!game.join(crate::export::GAME_FOLDER).exists(), "no folder of the game's files");
    let packed = crate::pack::unpack(&std::fs::read(game.join(crate::pack::PACK_FILE)).unwrap()).unwrap();
    let file = |name: &str| packed.iter().find(|(p, _)| p == name).map(|(_, b)| b.clone());
    assert!(file("scripts/spin.bscript").unwrap().starts_with(bayleaf_script::WASM_MAGIC), "visual scripts are compiled");
    assert!(file("scripts/mover.wasm").is_some(), "built modules go along");
    assert!(file("scenes/daylight.blight").is_some(), "the lighting asset the scene names goes along");
    for left in ["scripts/mover/src/lib.rs", "scripts/mover/Cargo.toml", "saves/progress.json", "notes.txt"] {
        assert!(file(left).is_none(), "{left} stays behind");
    }
    let skipped: Vec<&str> = result["skipped"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(skipped.contains(&"notes.txt") && skipped.contains(&"saves"), "and is listed: {skipped:?}");
    assert!(registry.execute(&mut session, "project.export", &json!({"folder": out, "player": player})).is_err(), "not over an earlier export");

    // The player finds the game beside it, and it plays from the compiled scripts.
    let found = crate::export::packaged_game(&program, None).expect("the game beside the program");
    let mut game_session = Session::default();
    registry.execute(&mut game_session, "project.open", &json!({"path": found})).unwrap();
    let started = registry.execute(&mut game_session, "play.start", &Value::Null).unwrap();
    assert!(started["running"].as_u64().unwrap() > 0, "its scripts run: {started}");
    assert!(game_session.console().iter().all(|l| l.level != crate::Level::Error), "{:?}", game_session.console());
    registry.execute(&mut game_session, "play.stop", &Value::Null).unwrap();
    assert_eq!(started["runtime"], "interpreted", "exported interpreted, it plays interpreted");

    // Exported compiled to machine code, the game says so, and the player runs it that way (or
    // says why it interprets them).
    let fast = base.join("fast");
    registry.execute(&mut session, "project.export", &json!({"folder": fast, "player": player, "mode": "compiled"})).unwrap();
    let found = crate::export::packaged_game(&fast.join("Space Race").join(format!("Space Race{}", std::env::consts::EXE_SUFFIX)), None).unwrap();
    let mut game_session = Session::default();
    registry.execute(&mut game_session, "project.open", &json!({"path": found})).unwrap();
    let mode = crate::export::chosen_mode(&game_session, crate::export::Platform::current());
    assert_eq!(mode, crate::export::ScriptMode::Compiled, "the exported project keeps the mode");
    let started = registry.execute(&mut game_session, "play.start", &json!({"scripts": mode.runtime()})).unwrap();
    assert!(started["running"].as_u64().unwrap() > 0, "its scripts run: {started}");
    if cfg!(feature = "compiled-scripts") {
        assert_eq!(started["runtime"], "compiled");
    } else {
        assert_eq!(started["runtime"], "interpreted");
        assert!(game_session.console().iter().any(|l| l.text.contains("Scripts are interpreted")), "it says why: {:?}", game_session.console());
    }
    registry.execute(&mut game_session, "play.stop", &Value::Null).unwrap();

    // Agents export only into their write roots.
    session.set_confinement(Some(crate::confine::Confinement::new(std::slice::from_ref(&base), std::slice::from_ref(&project)).unwrap()));
    assert!(registry.execute(&mut session, "project.export", &json!({"folder": base.join("elsewhere"), "player": player})).is_err());
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn the_export_check_says_what_stops_an_export_for_each_platform_and_script_mode() {
    use crate::export::{Platform, ScriptMode};
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let base = std::env::temp_dir().join(format!("bayleaf-export-check-{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    let project = base.join("Checked");
    registry.execute(&mut session, "project.new", &json!({"path": project})).unwrap();
    let player = base.join("player-program");
    std::fs::write(&player, b"the player").unwrap();
    // A built Rust script with its source beside the visual one the starter project has.
    std::fs::create_dir_all(project.join("scripts/mover/src")).unwrap();
    std::fs::write(project.join("scripts/mover/Cargo.toml"), "[package]").unwrap();
    std::fs::write(project.join("scripts/mover/src/lib.rs"), "// source").unwrap();
    std::fs::write(project.join("scripts/mover.wasm"), b"\0asm\x01\0\0\0").unwrap();
    let check = |session: &mut Session, platform: Platform, mode: ScriptMode| {
        registry.execute(session, "project.export.check", &json!({"platform": platform, "mode": mode, "player": player})).unwrap()
    };
    let errors = |report: &Value| -> Vec<String> {
        report["findings"].as_array().unwrap().iter().filter(|f| f["severity"] == "error").map(|f| f["message"].as_str().unwrap().to_owned()).collect()
    };

    let here = check(&mut session, Platform::current(), ScriptMode::Interpreted);
    assert_eq!(here["ready"], true, "a fresh project exports for this system, interpreted: {here}");
    let elsewhere = Platform::ALL.into_iter().find(|p| *p != Platform::current()).unwrap();
    // Another system takes a player built there; this "player" is not a program for it.
    assert!(errors(&check(&mut session, elsewhere, ScriptMode::Interpreted)).iter().any(|e| e.contains("not")), "other systems say what is missing");
    let compiled = check(&mut session, Platform::current(), ScriptMode::Compiled);
    assert_eq!(compiled["ready"], true, "scripts compiled to machine code export for this system: {compiled}");
    assert!(errors(&check(&mut session, Platform::Web, ScriptMode::Compiled)).iter().any(|e| e.contains("browsers")), "but not for the web");
    let native = check(&mut session, Platform::current(), ScriptMode::Native);
    let said = |report: &Value, severity: &str| -> Vec<String> {
        report["findings"].as_array().unwrap().iter().filter(|f| f["severity"] == severity).map(|f| f["message"].as_str().unwrap().to_owned()).collect()
    };
    assert!(said(&native, "warning").iter().any(|w| w.contains("no sandbox")), "native says plainly what it gives up: {native}");
    // It builds the player here, from the engine's source with cargo: ready when both are found.
    assert_eq!(native["ready"] == true, ScriptMode::Native.available_for(Platform::current()), "{native}");
    assert!(errors(&check(&mut session, Platform::Web, ScriptMode::Native)).iter().any(|e| e.contains("on this computer")), "only for this system");
    let runs = |path: &str| native["scripts"].as_array().unwrap().iter().find(|s| s["path"] == path).map(|s| s["runs"].clone()).unwrap();
    assert_eq!(runs("scripts/mover.wasm"), json!("native"), "a Rust script with its source would be native");
    assert_eq!(runs("scripts/spin.bscript"), json!("compiled"), "a visual script would stay compiled WebAssembly");

    // A broken visual script, an unbuilt Rust script and a file a scene names that is gone.
    std::fs::write(project.join("scripts/broken.bscript"), "{ not a graph").unwrap();
    std::fs::create_dir_all(project.join("scripts/unbuilt/src")).unwrap();
    std::fs::write(project.join("scripts/unbuilt/Cargo.toml"), "[package]").unwrap();
    let material = std::fs::read_dir(project.join("materials")).unwrap().flatten().next().unwrap().path();
    std::fs::remove_file(&material).unwrap();
    std::fs::remove_file(project.join("scenes/daylight.blight")).unwrap();
    let broken = check(&mut session, Platform::current(), ScriptMode::Interpreted);
    let found = errors(&broken);
    assert!(found.iter().any(|e| e.contains("broken.bscript")), "{found:?}");
    assert!(found.iter().any(|e| e.contains("unbuilt") && e.contains("not built")), "{found:?}");
    assert!(found.iter().any(|e| e.contains("not in the project")), "{found:?}");
    assert!(found.iter().any(|e| e.contains("daylight.blight") && e.contains("not in the project")), "a scene's lighting asset too: {found:?}");

    // The export refuses what the check refuses, and writes nothing.
    let out = base.join("out");
    let refused = registry.execute(&mut session, "project.export", &json!({"folder": out, "player": player}));
    assert!(matches!(&refused, Err(CommandError::Failed(m)) if m.contains("cannot be exported yet")), "{refused:?}");
    assert!(!out.exists());
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn an_older_scene_s_environment_moves_into_a_lighting_asset_the_scene_then_names() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-lighting-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let old = json!({
        "format": "bayleaf.scene", "version": 16,
        "objects": [{ "id": "0b1f6e3a8c2d4e5f9a7b6c5d4e3f2a11", "parent": null, "components": {
            "name": "Sky",
            "environment": { "skyColor": [0.1, 0.2, 0.3], "horizonColor": [0.4, 0.5, 0.6], "groundColor": [0.2, 0.2, 0.2], "ambient": 2.0, "showSky": true, "fogDistance": 50.0 }
        }}]
    });
    std::fs::write(dir.join("cave.bscene"), old.to_string()).unwrap();

    registry.execute(&mut session, "scene.open", &json!({"path": dir.join("cave.bscene")})).unwrap();
    let lighting = registry.execute(&mut session, "scene.settings", &Value::Null).unwrap()["lighting"].as_str().unwrap().to_owned();
    assert!(lighting.ends_with(".blight") && dir.join(&lighting).is_file(), "a lighting asset beside the scene: {lighting}");
    let moved = registry.execute(&mut session, "lighting.get", &json!({"path": dir.join(&lighting)})).unwrap();
    assert_eq!(moved["lighting"]["ambient"], 2.0, "it keeps the Environment's light");
    assert_eq!(moved["lighting"]["fogDistance"], 50.0);
    let sky = bayleaf_core::StableId::parse_hex("0b1f6e3a8c2d4e5f9a7b6c5d4e3f2a11").unwrap();
    assert!(session.scene().component(sky, "environment").is_err(), "the component is gone from the scene");
    assert!(session.is_dirty(), "the scene changed, so it can be saved at the new version");

    // Settings change as one undo step; a lighting file changes every scene that names it.
    registry.execute(&mut session, "scene.settings.set", &json!({"lighting": ""})).unwrap();
    registry.execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert_eq!(registry.execute(&mut session, "scene.settings", &Value::Null).unwrap()["lighting"], lighting.as_str());
    let mut changed = moved["lighting"].clone();
    changed["ambient"] = json!(0.5);
    registry.execute(&mut session, "lighting.set", &json!({"path": dir.join(&lighting), "lighting": changed})).unwrap();
    assert_eq!(registry.execute(&mut session, "lighting.get", &json!({"path": dir.join(&lighting)})).unwrap()["lighting"]["ambient"], 0.5);
    assert!(registry.execute(&mut session, "lighting.set", &json!({"path": dir.join("cave.bscene"), "lighting": changed})).is_err(), "only .blight files");
    assert!(registry.execute(&mut session, "scene.settings.set", &json!({"lighting": "notes.txt"})).is_err());

    // Agents only write lighting inside their roots.
    std::fs::create_dir_all(dir.join("elsewhere")).unwrap();
    session.set_confinement(Some(crate::confine::Confinement::new(std::slice::from_ref(&dir), &[dir.join("elsewhere")]).unwrap()));
    assert!(registry.execute(&mut session, "lighting.set", &json!({"path": dir.join(&lighting), "lighting": changed})).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_native_export_writes_a_workspace_that_builds_the_player_with_the_rust_scripts() {
    use crate::export::{Platform, ScriptMode};
    assert!(
        ScriptMode::Native.available_for(Platform::current()),
        "native export needs the engine's source and cargo, which a test run from the source tree has"
    );
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let base = std::env::temp_dir().join(format!("bayleaf-native-export-{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    let project = base.join("Native Game");
    registry.execute(&mut session, "project.new", &json!({"path": project})).unwrap();
    // A Rust script as a crate of its own: a WebAssembly library with its own workspace, profile and
    // a dependency by a relative path.
    std::fs::create_dir_all(project.join("scripts/mover/src")).unwrap();
    std::fs::write(
        project.join("scripts/mover/Cargo.toml"),
        "[package]\nname = \"mover\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[lib]\ncrate-type = [\"cdylib\"]\n\n[dependencies]\nbayleaf-script-sdk = { path = \"../../sdk\" }\nhelper = { path = \"../../../helper\" }\n\n[workspace]\n\n[profile.release]\nopt-level = \"s\"\n",
    )
    .unwrap();
    std::fs::write(project.join("scripts/mover/src/lib.rs"), "// the mover").unwrap();
    // A library of its own beside the project, which the script uses.
    std::fs::create_dir_all(base.join("helper/src")).unwrap();
    std::fs::write(base.join("helper/Cargo.toml"), "[package]\nname = \"helper\"\nversion = \"0.1.0\"\nedition = \"2024\"\n").unwrap();
    std::fs::write(base.join("helper/src/lib.rs"), "").unwrap();
    std::fs::write(project.join("scripts/mover.wasm"), b"\0asm\x01\0\0\0").unwrap();
    let out = base.join("out");

    let result = registry.execute(&mut session, "project.export", &json!({"folder": out, "mode": "native", "wait": false})).unwrap();
    assert_eq!(result["native"], json!(["scripts/mover.wasm"]), "the Rust script is built in");
    let build: crate::native_export::NativeBuild = serde_json::from_value(result["build"].clone()).unwrap();
    assert!(build.folder.starts_with(&project) && !build.program.exists(), "built later, in the project's build folder: {build:?}");
    let read = |path: &str| std::fs::read_to_string(build.folder.join(path)).unwrap();

    let root = read("Cargo.toml");
    assert!(root.contains("[patch.crates-io]"), "the engine's patches, which Cargo takes only from the workspace built:\n{root}");
    assert!(root.contains("script-sdk") && root.contains("\"scripts/mover\""), "{root}");
    assert!(build.folder.join("Cargo.lock").is_file(), "the engine's dependency versions");
    let main = read("player/src/main.rs");
    assert!(main.contains("PathBuf::from(\"scripts/mover.wasm\")") && main.contains("script_0::bayleaf_native()"), "{main}");
    assert!(read("player/Cargo.toml").contains("package = \"mover\""));
    let copy = read("scripts/mover/Cargo.toml");
    assert!(!copy.contains("crate-type") && !copy.contains("[workspace]") && !copy.contains("[profile"), "a plain library of the workspace:\n{copy}");
    assert!(copy.contains("bayleaf-script-sdk = { workspace = true }"), "the engine's SDK:\n{copy}");
    let helper = project.join("scripts/mover").join("../../../helper").to_string_lossy().replace('\\', "/");
    assert!(copy.contains(&helper), "relative paths still lead where they did:\n{copy}");
    assert_eq!(read("scripts/mover/src/lib.rs"), "// the mover");

    assert_cargo_reads(&build.folder);

    // The game's files are built into the program: nothing is beside it but the program itself.
    assert!(main.contains("include_bytes!") && main.contains("run_game(natives, Some(GAME))"), "{main}");
    let pack = std::fs::read(build.folder.join(crate::pack::PACK_FILE)).unwrap();
    let packed = crate::pack::unpack(&pack).unwrap();
    assert!(packed.iter().all(|(p, _)| p != "scripts/mover.wasm"), "a script built in leaves its module out");
    let out_game = build.program.parent().unwrap();
    assert!(!out_game.join(crate::pack::PACK_FILE).exists() && !out_game.join(crate::export::GAME_FOLDER).exists());

    // The game is exported as native, so its player runs it that way.
    let found = crate::export::packaged_game(&build.program, Some(&pack)).unwrap();
    let mut game = Session::default();
    registry.execute(&mut game, "project.open", &json!({"path": found})).unwrap();
    assert_eq!(crate::export::chosen_mode(&game, Platform::current()), ScriptMode::Native);
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn play_message_sends_while_playing_and_refuses_what_does_not_fit() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let id = registry.execute(&mut session, "scene.object.create", &json!({"name": "Target"})).unwrap()["id"].clone();
    assert!(registry.execute(&mut session, "play.message", &json!({"name": "jump", "id": id})).is_err(), "only while the game plays");
    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    registry.execute(&mut session, "play.message", &json!({"name": "jump", "id": id})).unwrap();
    registry.execute(&mut session, "play.message", &json!({"name": "everyone", "value": 2.5})).unwrap();
    let long = "x".repeat(bayleaf_script::MAX_MESSAGE_NAME + 1);
    for bad in [json!({"name": ""}), json!({"name": long}), json!({"name": "jump", "id": bayleaf_core::StableId::from_u128(42)})] {
        assert!(matches!(registry.execute(&mut session, "play.message", &bad), Err(CommandError::BadParams(_))), "{bad}");
    }
    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
}

/// Cargo loads the generated workspace: every member is there and every manifest reads.
fn assert_cargo_reads(folder: &std::path::Path) {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = std::process::Command::new(cargo)
        .args(["metadata", "--no-deps", "--offline", "--format-version", "1", "--manifest-path"])
        .arg(folder.join("Cargo.toml"))
        .output()
        .unwrap();
    assert!(output.status.success(), "cargo does not read the workspace:\n{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn a_native_export_of_a_game_without_rust_scripts_builds_a_workspace_cargo_reads() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let base = std::env::temp_dir().join(format!("bayleaf-native-plain-{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    let project = base.join("Plain");
    registry.execute(&mut session, "project.new", &json!({"path": project})).unwrap();
    let check = registry.execute(&mut session, "project.export.check", &json!({"mode": "native"})).unwrap();
    let notes: Vec<&str> = check["findings"].as_array().unwrap().iter().filter_map(|f| f["message"].as_str()).collect();
    assert!(notes.iter().any(|n| n.contains("no Rust script")), "it says Native gains nothing here: {notes:?}");
    let result = registry.execute(&mut session, "project.export", &json!({"folder": base.join("out"), "mode": "native", "wait": false})).unwrap();
    assert_eq!(result["native"], json!([]));
    let build: crate::native_export::NativeBuild = serde_json::from_value(result["build"].clone()).unwrap();
    assert_cargo_reads(&build.folder);
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn a_visual_script_property_takes_inspector_options_the_host_accepts() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    registry.execute(&mut session, "graph.new", &json!({"domain": "script"})).unwrap();
    registry.execute(&mut session, "graph.property.add", &json!({"name": "speed", "type": "number"})).unwrap();
    let options = json!({"label": "Top speed", "range": [0, 10], "slider": true, "group": "Motion"});
    registry.execute(&mut session, "graph.property.set", &json!({"name": "speed", "inspector": options})).unwrap();
    let kept = session.graph().unwrap().graph.property("speed").unwrap().inspector.clone();
    assert_eq!((kept.label.as_str(), kept.slider), ("Top speed", true));
    // A slider with no range is refused, and the options stay as they were.
    let refused = registry.execute(&mut session, "graph.property.set", &json!({"name": "speed", "inspector": {"slider": true}}));
    assert!(matches!(refused, Err(CommandError::BadParams(m)) if m.contains("range")));
    assert_eq!(session.graph().unwrap().graph.property("speed").unwrap().inspector, kept);
}

#[test]
fn a_prefab_opens_on_its_own_and_its_copies_follow_when_it_is_saved() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-prefab-edit-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    registry.execute(&mut session, "scene.saveAs", &json!({"path": dir.join("level.bscene")})).unwrap();
    let first = create(&registry, &mut session, json!({"name": "Crate"}));
    registry.execute(&mut session, "scene.prefab.create", &json!({"id": first, "path": "crate.bprefab"})).unwrap();
    let second = registry.execute(&mut session, "scene.prefab.place", &json!({"path": "crate.bprefab"})).unwrap()["id"].as_str().unwrap().to_owned();
    let second = bayleaf_core::StableId::parse_hex(&second).unwrap();
    let moved = json!({"position": [5, 0, 0], "rotation": [0, 0, 0], "scale": [1, 1, 1]});
    registry.execute(&mut session, "scene.component.set", &json!({"id": second, "component": "transform", "value": moved})).unwrap();
    let outer_steps = session.undo.len();

    let opened = registry.execute(&mut session, "prefab.open", &json!({"path": "crate.bprefab"})).unwrap();
    assert_eq!((opened["source"].as_str(), opened["depth"].as_u64()), (Some("crate.bprefab"), Some(1)));
    assert_eq!(session.open_prefabs(), ["crate.bprefab"]);
    let roots = session.scene().children(None).to_vec();
    assert_eq!(roots.len(), 1, "only the prefab's objects");
    assert_eq!(session.scene().get::<bayleaf_scene::components::Transform>(roots[0]).unwrap().position, [0.0; 3], "around the origin");
    assert!(session.undo.is_empty(), "its own undo history");
    create(&registry, &mut session, json!({"name": "Lid", "parent": roots[0]}));
    for (command, params) in [("play.start", json!({})), ("scene.save", json!({})), ("scene.new", json!({}))] {
        assert!(registry.execute(&mut session, command, &params).is_err(), "{command} waits until the prefab is closed");
    }
    assert!(registry.execute(&mut session, "prefab.close", &json!({})).is_err(), "unsaved changes are not dropped unasked");
    registry.execute(&mut session, "prefab.save", &json!({})).unwrap();
    registry.execute(&mut session, "prefab.close", &json!({})).unwrap();

    assert!(session.open_prefabs().is_empty());
    assert_eq!(session.undo.len(), outer_steps, "the scene's undo history waited underneath");
    let first = bayleaf_core::StableId::parse_hex(&first).unwrap();
    for copy in [first, second] {
        let names: Vec<String> = session.scene().children(Some(copy)).iter().map(|c| session.scene().name(*c).unwrap_or_default().to_owned()).collect();
        assert_eq!(names, ["Lid"], "every copy follows the saved prefab");
    }
    assert_eq!(session.scene().get::<bayleaf_scene::components::Transform>(second).unwrap().position[0], 5.0, "a copy keeps its own transform");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_scenes_post_processing_asset_changes_its_render_as_far_as_the_project_allows() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let base = std::env::temp_dir().join(format!("bayleaf-post-{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    let project = base.join("Looks");
    registry.execute(&mut session, "project.new", &json!({"path": project})).unwrap();
    let made = registry.execute(&mut session, "asset.create", &json!({"kind": "postProcessing", "folder": project, "name": "grey"})).unwrap();
    let path = made["path"].as_str().unwrap().to_owned();
    let mut post = registry.execute(&mut session, "post.get", &json!({"path": path})).unwrap()["post"].clone();
    post["color"] = json!({"enabled": true, "contrast": 1.0, "saturation": 0.0});
    post["toneMapping"]["enabled"] = json!(false);
    registry.execute(&mut session, "post.set", &json!({"path": path, "post": post})).unwrap();
    let mut bad = post.clone();
    bad["vignette"]["strength"] = json!(3.0);
    assert!(registry.execute(&mut session, "post.set", &json!({"path": path, "post": bad})).is_err(), "out of range");
    create(
        &registry,
        &mut session,
        json!({"name": "Red", "components": {"mesh": {"shape": "cube", "color": [0.9, 0.1, 0.1, 1], "metallic": 0, "roughness": 0.5}}}),
    );

    let centre = |session: &mut Session, name: &str| -> Option<[u8; 4]> {
        let png = base.join(name);
        registry.execute(session, "scene.render", &json!({"path": png, "width": 32, "height": 32})).ok()?;
        let image = bayleaf_render::assets::load_image(&png).ok()?;
        let at = (16 * image.width as usize + 16) * 4;
        Some([image.rgba[at], image.rgba[at + 1], image.rgba[at + 2], 255])
    };
    let Some(plain) = centre(&mut session, "plain.png") else { return }; // no GPU here
    assert!(plain[0] > plain[1] + 40, "red as drawn: {plain:?}");
    registry.execute(&mut session, "scene.settings.set", &json!({"postProcessing": "grey.bpost"})).unwrap();
    let grey = centre(&mut session, "grey.png").unwrap();
    assert!(grey[0].abs_diff(grey[1]) < 8, "the scene's post-processing takes the colour out: {grey:?}");

    // A colour lookup image that swaps red and blue: 8 squares of 8 by 8.
    let steps = 8u32;
    let mut pixels = Vec::new();
    for g in 0..steps {
        for b in 0..steps {
            for r in 0..steps {
                let level = |v: u32| (v * 255 / (steps - 1)) as u8;
                pixels.extend_from_slice(&[level(b), level(g), level(r), 255]);
            }
        }
    }
    bayleaf_render::Image { width: steps * steps, height: steps, pixels }.write_png(&project.join("swap.png")).unwrap();
    let mut swapped = post.clone();
    swapped["color"] = json!({"enabled": true, "contrast": 1.0, "saturation": 1.0, "lookup": "swap.png", "lookupStrength": 1.0});
    let set = registry.execute(&mut session, "post.set", &json!({"path": path, "post": swapped})).unwrap();
    assert!(set["warnings"].as_array().unwrap().is_empty(), "{set}");
    let blue = centre(&mut session, "swapped.png").unwrap();
    assert!(blue[2] > blue[0] + 40, "the lookup turns the red cube blue: {blue:?}");
    // An image of another size is refused, and one not there yet is only a warning.
    bayleaf_render::Image { width: 10, height: 10, pixels: vec![255; 400] }.write_png(&project.join("square.png")).unwrap();
    let mut wrong = swapped.clone();
    wrong["color"]["lookup"] = json!("square.png");
    let refused = registry.execute(&mut session, "post.set", &json!({"path": path, "post": wrong})).unwrap_err();
    assert!(refused.to_string().contains("10 by 10"), "{refused}");
    wrong["color"]["lookup"] = json!("later.png");
    let later = registry.execute(&mut session, "post.set", &json!({"path": path, "post": wrong})).unwrap();
    assert!(!later["warnings"].as_array().unwrap().is_empty(), "{later}");
    // A neutral lookup changes nothing, and is not written over an image already there.
    let neutral = registry.execute(&mut session, "post.lookup.neutral", &json!({"path": "neutral.png", "steps": 8})).unwrap();
    let mut kept = post.clone();
    kept["color"] = json!({"enabled": true, "contrast": 1.0, "saturation": 1.0, "lookup": neutral["relative"], "lookupStrength": 1.0});
    registry.execute(&mut session, "post.set", &json!({"path": path, "post": kept})).unwrap();
    let same = centre(&mut session, "neutral-render.png").unwrap();
    assert!((0..3).all(|c| same[c].abs_diff(plain[c]) < 8), "a neutral lookup keeps the colour: {same:?} against {plain:?}");
    assert!(registry.execute(&mut session, "post.lookup.neutral", &json!({"path": "swap.png"})).is_err(), "not over an image");
    registry.execute(&mut session, "post.set", &json!({"path": path, "post": post})).unwrap();

    // A project that does not allow colour adjustment shows the colour again.
    let mut settings = registry.execute(&mut session, "project.settings", &Value::Null).unwrap()["settings"].clone();
    settings["graphics"]["effects"] = json!({"color": false});
    registry.execute(&mut session, "project.settings.set", &json!({"settings": settings})).unwrap();
    let allowed = centre(&mut session, "allowed.png").unwrap();
    assert!(allowed[0] > allowed[1] + 40, "colour adjustment is not allowed: {allowed:?}");

    // Agents read and write post-processing assets only inside their folders.
    session.set_confinement(Some(crate::confine::Confinement::new(std::slice::from_ref(&project), std::slice::from_ref(&project)).unwrap()));
    assert!(registry.execute(&mut session, "post.get", &json!({"path": base.join("elsewhere.bpost")})).is_err());
    assert!(registry.execute(&mut session, "post.lookup.neutral", &json!({"path": base.join("elsewhere.png")})).is_err(), "a lookup outside is refused");
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn particles_play_with_the_game_and_scripts_burst_them() {
    // A script that bursts 40 particles from its own object when it starts.
    let burster = wat::parse_str(
        r#"(module
            (import "bayleaf" "self_handle" (func $me (result i32)))
            (import "bayleaf" "particles_burst" (func $burst (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_start") (drop (call $burst (call $me) (i32.const 40)))))"#,
    )
    .unwrap();
    let (registry, mut session, id, dir) = scripted_scene("particles", &burster);
    let emitter = json!({"playing": false, "rate": 0, "lifetime": [5, 5]});
    let object = bayleaf_core::StableId::parse_hex(&id).unwrap();
    registry.execute(&mut session, "scene.component.add", &json!({"id": object, "component": "particleEmitter"})).unwrap();
    let mut value = session.scene().component(object, "particleEmitter").unwrap();
    for (key, v) in emitter.as_object().unwrap() {
        value[key] = v.clone();
    }
    registry.execute(&mut session, "scene.component.set", &json!({"id": object, "component": "particleEmitter", "value": value})).unwrap();
    let rising = create(
        &registry,
        &mut session,
        json!({"name": "Fountain", "components": {"particleEmitter": serde_json::to_value(bayleaf_scene::components::ParticleEmitter::default()).unwrap()}}),
    );

    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    registry.execute(&mut session, "play.step", &json!({"seconds": 1.0 / 60.0, "steps": 60})).unwrap();
    let particles = session.particles().unwrap();
    assert_eq!(particles.count(Some(object)), 40, "the script's burst, from an emitter that is not playing");
    let fountain = bayleaf_core::StableId::parse_hex(&rising).unwrap();
    assert!(particles.count(Some(fountain)) > 0, "a playing emitter emits with the game");
    let status = registry.execute(&mut session, "play.status", &Value::Null).unwrap();
    assert!(status["particles"].as_u64().unwrap() >= 40);
    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
    assert!(session.particles().is_none(), "stopping takes them away");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_profiler_records_each_step_by_part_while_asked() {
    let (registry, mut session, _, dir) = scripted_scene("profile", &spinner_wasm());
    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    registry.execute(&mut session, "play.step", &json!({"seconds": 0.02, "steps": 5})).unwrap();
    assert!(session.profiler().frames().is_empty(), "nothing is kept unless it records");

    registry.execute(&mut session, "profile.start", &json!({})).unwrap();
    registry.execute(&mut session, "play.step", &json!({"seconds": 0.02, "steps": 10})).unwrap();
    let read = registry.execute(&mut session, "profile.frames", &json!({"last": 4})).unwrap();
    assert_eq!(read["frames"].as_array().unwrap().len(), 4, "the last frames asked for");
    let frame = &read["frames"][0];
    assert!(frame["step"].as_f64().unwrap() >= frame["scripts"].as_f64().unwrap(), "the step holds its parts: {frame}");
    let scripts: Vec<&str> = frame["byScript"].as_array().unwrap().iter().filter_map(|s| s[0].as_str()).collect();
    assert_eq!(scripts, ["Spin"], "each script by name");

    registry.execute(&mut session, "profile.stop", &json!({})).unwrap();
    registry.execute(&mut session, "play.step", &json!({"seconds": 0.02, "steps": 5})).unwrap();
    assert_eq!(session.profiler().frames().len(), 10, "stopped, it keeps what it had and adds nothing");
    registry.execute(&mut session, "profile.clear", &json!({})).unwrap();
    assert!(session.profiler().frames().is_empty());
    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
    std::fs::remove_dir_all(&dir).ok();
}

/// A one-triangle glTF model naming `tri.bin` and an image in `tex/`, both beside it.
fn write_gltf_with_files(dir: &std::path::Path) {
    let mut bin: Vec<u8> = [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]].iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
    bin.extend([0u8, 0, 1, 0, 2, 0, 0, 0]);
    std::fs::write(dir.join("tri.bin"), &bin).unwrap();
    std::fs::create_dir_all(dir.join("tex")).unwrap();
    std::fs::write(dir.join("tex/logo.png"), include_bytes!("../../../assets/brand/bayleaf-logo.png")).unwrap();
    let gltf = json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
        "buffers": [{"uri": "tri.bin", "byteLength": bin.len()}],
        "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 36}, {"buffer": 0, "byteOffset": 36, "byteLength": 6}],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, -1], "max": [1, 0, 0]},
            {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}
        ],
        "images": [{"uri": "tex/logo.png"}]
    });
    std::fs::write(dir.join("tri.gltf"), gltf.to_string()).unwrap();
}

/// A short silent WAV file.
fn wav_bytes() -> Vec<u8> {
    let samples = 800u32;
    let mut b = Vec::new();
    b.extend(b"RIFF");
    b.extend((36 + samples * 2).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(8000u32.to_le_bytes());
    b.extend(16000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend((samples * 2).to_le_bytes());
    b.extend(vec![0u8; samples as usize * 2]);
    b
}

/// Every file under `dir`, relative to it, sorted.
fn files_under(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(folder) = pending.pop() {
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(path.strip_prefix(dir).unwrap().to_path_buf());
            }
        }
    }
    found.sort();
    found
}

#[test]
fn files_brought_in_are_checked_copied_with_their_settings_and_placed() {
    use bayleaf_core::StableId;
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let base = std::env::temp_dir().join(format!("bayleaf-import-{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    let (project, outside) = (base.join("Game"), base.join("outside"));
    registry.execute(&mut session, "project.new", &json!({"path": project})).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("logo.png"), include_bytes!("../../../assets/brand/bayleaf-logo.png")).unwrap();
    std::fs::write(outside.join("broken.png"), b"not an image").unwrap();
    std::fs::write(outside.join("crate.obj"), b"v 0 0 0").unwrap();
    std::fs::write(outside.join("crate.fbx"), b"Kaydara FBX Binary").unwrap();
    // A model with its material library and image beside it.
    std::fs::write(outside.join("box.obj"), "mtllib box.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl Lit\nf 1 2 3\n").unwrap();
    std::fs::write(outside.join("box.mtl"), "newmtl Lit\nKd 1 1 1\nmap_Kd logo.png\n").unwrap();
    std::fs::write(outside.join("tone.wav"), wav_bytes()).unwrap();
    write_gltf_with_files(&outside);
    let before = files_under(&project.join("assets"));

    // One unreadable file stops the whole import, and nothing is left behind.
    let broken = registry.execute(&mut session, "asset.import", &json!({"files": [outside.join("logo.png"), outside.join("broken.png")], "folder": "assets"}));
    assert!(broken.unwrap_err().to_string().contains("broken.png"));
    let obj = registry.execute(&mut session, "asset.import", &json!({"files": [outside.join("crate.obj")], "folder": "assets"})).unwrap_err().to_string();
    assert!(obj.contains("crate.obj"), "an OBJ with nothing to draw is refused: {obj}");
    let fbx = registry.execute(&mut session, "asset.import", &json!({"files": [outside.join("crate.fbx")], "folder": "assets"})).unwrap_err().to_string();
    assert!(fbx.contains("glTF"), "says what to bring instead: {fbx}");
    assert_eq!(files_under(&project.join("assets")), before);

    let files = [outside.join("logo.png"), outside.join("tri.gltf"), outside.join("tone.wav"), outside.join("box.obj")];
    let done = registry.execute(&mut session, "asset.import", &json!({"files": files, "folder": "assets"})).unwrap();
    let paths: Vec<&str> = done["imported"].as_array().unwrap().iter().map(|i| i["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["assets/logo.png", "assets/tri/tri.gltf", "assets/tone.wav", "assets/box/box.obj"]);
    for file in
        ["assets/logo.png.bimport", "assets/tri/tri.bin", "assets/tri/tex/logo.png", "assets/tri/tri.gltf.bimport", "assets/box/box.mtl", "assets/box/logo.png"]
    {
        assert!(project.join(file).is_file(), "{file} came in");
    }
    assert!(project.join("assets/tone.wav.bimport").is_file(), "sounds get settings too: streamed or loaded");
    // A WAV can be played from its file; a file that is not a sound cannot.
    registry.execute(&mut session, "asset.importSettings.set", &json!({"path": "assets/tone.wav", "settings": {"sound": {"stream": true}}})).unwrap();
    let got = registry.execute(&mut session, "asset.importSettings.get", &json!({"path": "assets/tone.wav"})).unwrap();
    assert_eq!(got["settings"]["sound"]["stream"], true);
    #[cfg(feature = "audio")]
    {
        std::fs::write(project.join("assets/fake.ogg"), b"OggS but not really").unwrap();
        let fake = registry.execute(&mut session, "asset.importSettings.set", &json!({"path": "assets/fake.ogg", "settings": {"sound": {"stream": true}}}));
        assert!(fake.unwrap_err().to_string().contains("loaded whole"));
    }
    assert_eq!(
        bayleaf_render::assets::import::SOUND_EXTENSIONS,
        bayleaf_scene::components::SOUND_EXTENSIONS,
        "import settings and the Audio Source take the same sounds"
    );
    let again = registry.execute(&mut session, "asset.import", &json!({"files": [outside.join("logo.png")], "folder": "assets"})).unwrap();
    assert_ne!(again["imported"][0]["path"], "assets/logo.png", "a name already taken is not written over");
    let listed = registry.execute(&mut session, "asset.list", &json!({"path": "assets", "recursive": true})).unwrap();
    assert!(!listed.to_string().contains(".bimport"), "settings belong to their file: {listed}");

    // Settings: only the file's own kind, checked, saved beside it.
    let model = "assets/tri/tri.gltf";
    registry.execute(&mut session, "asset.importSettings.set", &json!({"path": model, "settings": {"model": {"scale": 2.0}}})).unwrap();
    let got = registry.execute(&mut session, "asset.importSettings.get", &json!({"path": model})).unwrap();
    assert_eq!((got["settings"]["model"]["scale"].as_f64(), got["saved"].as_bool()), (Some(2.0), Some(true)));
    assert!(registry.execute(&mut session, "asset.importSettings.set", &json!({"path": model, "settings": {"image": {"mipmaps": false}}})).is_err());
    assert!(registry.execute(&mut session, "asset.importSettings.set", &json!({"path": model, "settings": {"model": {"scale": -1.0}}})).is_err());
    // Block compression only for sides in multiples of 4; plain numbers for any image.
    bayleaf_render::Image { width: 6, height: 4, pixels: vec![200; 96] }.write_png(&project.join("assets/odd.png")).unwrap();
    bayleaf_render::Image { width: 8, height: 4, pixels: vec![200; 128] }.write_png(&project.join("assets/even.png")).unwrap();
    let odd = registry.execute(&mut session, "asset.importSettings.set", &json!({"path": "assets/odd.png", "settings": {"image": {"compression": "bc1"}}}));
    assert!(odd.unwrap_err().to_string().contains("multiples of 4"));
    registry
        .execute(
            &mut session,
            "asset.importSettings.set",
            &json!({"path": "assets/even.png", "settings": {"image": {"compression": "bc3", "colorSpace": "data"}}}),
        )
        .unwrap();
    let even = bayleaf_render::assets::load_image(&project.join("assets/even.png")).unwrap();
    assert!(even.data && even.blocks.is_some(), "the settings apply when the image is read");

    // Placing a model and a sound, and an image as an object's look; each one undo step.
    let placed = registry.execute(&mut session, "scene.file.place", &json!({"path": model, "position": [1.0, 0.0, 2.0]})).unwrap();
    let id = StableId::parse_hex(placed["id"].as_str().unwrap()).unwrap();
    assert_eq!(session.scene().component(id, "mesh").unwrap()["model"], model);
    assert_eq!(session.scene().component(id, "transform").unwrap()["position"], json!([1.0, 0.0, 2.0]));
    let sound = registry.execute(&mut session, "scene.file.place", &json!({"path": "assets/tone.wav"})).unwrap();
    let sound = StableId::parse_hex(sound["id"].as_str().unwrap()).unwrap();
    assert_eq!(session.scene().component(sound, "audioSource").unwrap()["clip"], "assets/tone.wav");
    registry.execute(&mut session, "edit.undo", &Value::Null).unwrap();
    assert!(session.scene().component(sound, "audioSource").is_err(), "undone");
    let cube = session.scene().children(None).iter().copied().find(|id| session.scene().name(*id) == Some("Cube")).unwrap();
    let applied = registry.execute(&mut session, "scene.image.apply", &json!({"id": cube, "image": "assets/logo.png"})).unwrap();
    let material = applied["material"].as_str().unwrap().to_owned();
    assert_eq!(session.scene().component(cube, "mesh").unwrap()["material"], material.as_str());
    let opened = registry.execute(&mut session, "graph.open", &json!({"path": project.join(&material)})).unwrap();
    assert!(opened["problems"].as_array().unwrap().is_empty(), "{opened}");

    // Confined: a file outside the read roots is refused.
    session.set_confinement(Some(crate::confine::Confinement::new(std::slice::from_ref(&project), std::slice::from_ref(&project)).unwrap()));
    assert!(registry.execute(&mut session, "asset.import", &json!({"files": [outside.join("logo.png")], "folder": "assets"})).is_err());
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn an_interface_tells_its_scripts_what_the_player_did_and_shows_what_they_set() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-interface-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    registry.execute(&mut session, "project.new", &json!({"path": dir})).unwrap();
    let made = registry.execute(&mut session, "asset.create", &json!({"kind": "interface", "folder": "", "name": "menu"})).unwrap();
    assert!(made["path"].as_str().unwrap().ends_with("menu.bui"));
    // A score bound to the game's state, added through the layout commands.
    let mut layout = registry.execute(&mut session, "ui.layout.get", &json!({"path": "menu.bui"})).unwrap()["layout"].clone();
    layout["root"]["children"].as_array_mut().unwrap().push(json!({"kind": "text", "name": "Score", "bind": "game:score"}));
    registry.execute(&mut session, "ui.layout.set", &json!({"path": "menu.bui", "layout": layout})).unwrap();
    let mut broken = layout.clone();
    broken["root"]["children"][0]["children"] = json!([{"kind": "text"}]);
    assert!(registry.execute(&mut session, "ui.layout.set", &json!({"path": "menu.bui", "layout": broken})).is_err(), "text holds no children");

    // A script that answers the Start button by changing the title.
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
    std::fs::write(session.script_dir().join("menu.wasm"), menu).unwrap();
    let id = create(
        &registry,
        &mut session,
        json!({"name": "Menu", "components": {"interface": {"layout": "menu.bui", "visible": true, "order": 0}, "scripts": [{"module": "menu.wasm"}]}}),
    );
    let text_of = |session: &mut Session, name: &str| {
        let state = registry.execute(session, "play.ui.state", &json!({"id": id})).unwrap();
        state["elements"].as_array().unwrap().iter().find(|e| e["name"] == name).map(|e| e["text"].as_str().unwrap().to_owned()).unwrap()
    };

    assert!(registry.execute(&mut session, "play.ui.state", &json!({"id": id})).is_err(), "only while playing");
    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    assert_eq!(text_of(&mut session, "Title"), "My Game");
    registry.execute(&mut session, "game.state.set", &json!({"name": "score", "value": 7})).unwrap();
    assert_eq!(text_of(&mut session, "Score"), "7", "bound to the game's state with no script");
    registry.execute(&mut session, "play.ui.press", &json!({"id": id, "element": "Start"})).unwrap();
    assert_eq!(text_of(&mut session, "Title"), "Started", "the script heard the press and changed the title");
    assert!(registry.execute(&mut session, "play.ui.press", &json!({"id": id, "element": "Title"})).is_err(), "text is not pressed");
    assert!(registry.execute(&mut session, "play.ui.press", &json!({"id": id, "element": "Nothing"})).is_err());

    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
    registry.execute(&mut session, "play.start", &Value::Null).unwrap();
    assert_eq!(text_of(&mut session, "Title"), "My Game", "stopping put the interface back");
    registry.execute(&mut session, "play.stop", &Value::Null).unwrap();
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_game_exports_for_another_system_with_a_player_built_there() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let base = std::env::temp_dir().join(format!("bayleaf-export-other-{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    let project = base.join("Space Game");
    registry.execute(&mut session, "project.new", &json!({"path": project})).unwrap();
    let other = if crate::export::Platform::current() == crate::export::Platform::Linux { "windows" } else { "linux" };
    let errors = |report: &Value| report["findings"].as_array().unwrap().iter().filter(|f| f["severity"] == "error").count();

    // Without a player built there, the check says what to choose.
    let check = registry.execute(&mut session, "project.export.check", &json!({"platform": other, "mode": "interpreted"})).unwrap();
    assert_eq!(check["ready"], false);
    assert!(check.to_string().contains("needs a player built for"), "{check}");
    // A program for the wrong system is refused.
    let wrong = base.join("wrong-player");
    std::fs::write(&wrong, if other == "linux" { b"MZ\x90\x00rest".to_vec() } else { b"\x7fELFrest".to_vec() }).unwrap();
    let check = registry.execute(&mut session, "project.export.check", &json!({"platform": other, "mode": "interpreted", "player": wrong})).unwrap();
    assert!(errors(&check) > 0 && check.to_string().contains("is a program for"), "{check}");
    let player = base.join("bayleaf-player-other");
    std::fs::write(&player, if other == "linux" { b"\x7fELF\x02\x01\x01rest".to_vec() } else { b"MZ\x90\x00rest".to_vec() }).unwrap();
    let check = registry.execute(&mut session, "project.export.check", &json!({"platform": other, "mode": "interpreted", "player": player})).unwrap();
    assert_eq!(errors(&check), 0, "{check}");
    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let done = registry.execute(&mut session, "project.export", &json!({"folder": out, "platform": other, "mode": "interpreted", "player": player})).unwrap();
    let program = std::path::PathBuf::from(done["program"].as_str().unwrap());
    assert_eq!(std::fs::read(&program).unwrap(), std::fs::read(&player).unwrap(), "the chosen player is the game's program");
    let game = program.parent().unwrap();
    if other == "linux" {
        assert!(program.extension().is_none(), "no .exe on Linux: {}", program.display());
        let entry = std::fs::read_to_string(game.join("Space Game.desktop")).unwrap();
        assert!(entry.contains("[Desktop Entry]") && entry.contains("Exec=") && entry.contains("Space Game"), "{entry}");
        let install = std::fs::read_to_string(game.join("install.sh")).unwrap();
        assert!(install.starts_with("#!/bin/sh") && install.contains("chmod +x"), "{install}");
    } else {
        assert!(program.extension().is_some_and(|e| e == "exe"));
    }
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn a_game_exports_for_the_web_as_a_page_with_its_files_in_one_pack_that_plays() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let base = std::env::temp_dir().join(format!("bayleaf-export-web-{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    let project = base.join("Space Game");
    registry.execute(&mut session, "project.new", &json!({"path": project})).unwrap();
    std::fs::create_dir_all(project.join("saves")).unwrap();
    std::fs::write(project.join("saves/progress.json"), "{}").unwrap();
    let errors = |report: &Value| report["findings"].as_array().unwrap().iter().filter(|f| f["severity"] == "error").count();

    // Without the web player the check says how to build it; a folder without one is refused.
    let empty = base.join("no-player");
    std::fs::create_dir_all(&empty).unwrap();
    let check = registry.execute(&mut session, "project.export.check", &json!({"platform": "web", "mode": "interpreted", "player": empty})).unwrap();
    assert!(errors(&check) > 0 && check.to_string().contains("cargo xtask web-player"), "{check}");
    assert!(
        errors(&registry.execute(&mut session, "project.export.check", &json!({"platform": "web", "mode": "compiled"})).unwrap()) > 0,
        "browsers interpret"
    );

    let player = base.join("web-player");
    std::fs::create_dir_all(&player).unwrap();
    std::fs::write(player.join(crate::web_export::PLAYER_SCRIPT), "// loader").unwrap();
    std::fs::write(player.join(crate::web_export::PLAYER_MODULE), b"\0asm\x01\0\0\0").unwrap();
    std::fs::create_dir_all(player.join("snippets/eframe-1")).unwrap();
    std::fs::write(player.join("snippets/eframe-1/inline0.js"), "// kept by wasm-bindgen").unwrap();
    let check = registry.execute(&mut session, "project.export.check", &json!({"platform": "web", "mode": "interpreted", "player": player})).unwrap();
    assert_eq!(errors(&check), 0, "{check}");
    let out = base.join("out");
    let done = registry.execute(&mut session, "project.export", &json!({"folder": out, "platform": "web", "mode": "interpreted", "player": player})).unwrap();
    let site = out.join("Space Game");
    assert_eq!(std::path::PathBuf::from(done["program"].as_str().unwrap()), site.join("index.html"));
    let page = std::fs::read_to_string(site.join("index.html")).unwrap();
    assert!(page.contains(crate::web_export::PLAYER_SCRIPT) && page.contains("Space Game"), "the page loads the player: {page}");
    assert_eq!(std::fs::read(site.join(crate::web_export::PLAYER_MODULE)).unwrap(), b"\0asm\x01\0\0\0", "the web player goes along");
    assert!(site.join("snippets/eframe-1/inline0.js").is_file(), "with the scripts wasm-bindgen keeps beside it");
    assert!(page.contains("[hidden]"), "the status line hides once the game runs");
    assert!(!site.join(crate::export::GAME_FOLDER).exists(), "the game's files are in the pack, not beside it");

    // The pack holds the game, visual scripts compiled and saved values left behind, and the
    // game plays from it.
    let files = crate::pack::unpack(&std::fs::read(site.join(crate::web_export::PACK_FILE)).unwrap()).unwrap();
    let file = |name: &str| files.iter().find(|(p, _)| p == name).map(|(_, b)| b.clone());
    assert!(file("scripts/spin.bscript").unwrap().starts_with(bayleaf_script::WASM_MAGIC), "visual scripts are compiled");
    assert!(file("saves/progress.json").is_none(), "saved values stay behind");
    let unpacked = base.join("unpacked");
    for (path, bytes) in &files {
        let target = unpacked.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, bytes).unwrap();
    }
    let mut game = Session::default();
    registry.execute(&mut game, "project.open", &json!({"path": unpacked})).unwrap();
    assert_eq!(crate::export::chosen_mode(&game, crate::export::Platform::Web), crate::export::ScriptMode::Interpreted, "the pack says how scripts run");
    let started = registry.execute(&mut game, "play.start", &Value::Null).unwrap();
    assert!(started["running"].as_u64().unwrap() > 0, "its scripts run: {started}");
    assert!(game.console().iter().all(|l| l.level != crate::Level::Error), "{:?}", game.console());
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn interface_elements_are_added_moved_changed_and_removed_one_by_one() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-interface-elements-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    registry.execute(&mut session, "project.new", &json!({"path": dir})).unwrap();
    registry.execute(&mut session, "asset.create", &json!({"kind": "interface", "folder": "", "name": "menu"})).unwrap();
    let run = |session: &mut Session, id: &str, params: Value| registry.execute(session, id, &params);
    let root = |session: &mut Session| registry.execute(session, "ui.layout.get", &json!({"path": "menu.bui"})).unwrap()["layout"]["root"].clone();
    let path = "menu.bui";

    // The starter is a title and a Start button; a row goes after them, a button into the row.
    let row = run(&mut session, "ui.element.add", json!({"path": path, "kind": "row"})).unwrap()["at"].clone();
    assert_eq!(row, json!([2]));
    let quit = run(&mut session, "ui.element.add", json!({"path": path, "into": [2], "kind": "button"})).unwrap()["at"].clone();
    assert_eq!(quit, json!([2, 0]));
    run(&mut session, "ui.element.set", json!({"path": path, "at": [2, 0], "element": {"name": "Quit", "text": "Quit", "style": {"grow": "1"}}})).unwrap();
    // The Start button moves into the row, before Quit; the row moves up as Start leaves.
    let moved = run(&mut session, "ui.element.move", json!({"path": path, "at": [1], "into": [2], "index": 0})).unwrap()["at"].clone();
    assert_eq!(moved, json!([1, 0]));
    let tree = root(&mut session);
    let names: Vec<&str> = tree["children"][1]["children"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Start", "Quit"]);
    assert_eq!(tree["children"][1]["children"][1]["style"]["grow"], "1");

    // What would break the layout is refused and nothing is written.
    let before = root(&mut session);
    assert!(run(&mut session, "ui.element.add", json!({"path": path, "into": [0], "kind": "button"})).is_err(), "text holds nothing");
    assert!(run(&mut session, "ui.element.move", json!({"path": path, "at": [1], "into": [1, 0], "index": 0})).is_err(), "not into itself");
    assert!(
        run(&mut session, "ui.element.set", json!({"path": path, "at": [1, 1], "element": {"style": {"width": "12"}}})).is_err(),
        "a length needs its unit"
    );
    assert!(run(&mut session, "ui.element.set", json!({"path": path, "at": [1, 1], "element": {"name": "Title"}})).is_err(), "names are used once");
    assert!(run(&mut session, "ui.element.set", json!({"path": path, "at": [1], "element": {"kind": "text"}})).is_err(), "a row with children stays a holder");
    assert!(run(&mut session, "ui.element.remove", json!({"path": path, "at": []})).is_err(), "the root stays");
    assert_eq!(root(&mut session), before);

    run(&mut session, "ui.element.remove", json!({"path": path, "at": [1]})).unwrap();
    assert_eq!(root(&mut session)["children"].as_array().unwrap().len(), 1, "the row went with both buttons");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_part_saved_from_a_menu_is_added_to_the_display_by_a_visual_script() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-interface-parts-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    registry.execute(&mut session, "project.new", &json!({"path": dir})).unwrap();
    let run = |session: &mut Session, id: &str, params: Value| registry.execute(session, id, &params);

    // A character menu built in one layout: a panel with a name and a health bar.
    run(&mut session, "asset.create", json!({"kind": "interface", "folder": "", "name": "menu"})).unwrap();
    let card = run(&mut session, "ui.element.add", json!({"path": "menu.bui", "kind": "panel"})).unwrap()["at"].clone();
    run(&mut session, "ui.element.set", json!({"path": "menu.bui", "at": card, "element": {"name": "Card"}})).unwrap();
    let name_at = run(&mut session, "ui.element.add", json!({"path": "menu.bui", "into": card, "kind": "text"})).unwrap()["at"].clone();
    run(&mut session, "ui.element.set", json!({"path": "menu.bui", "at": name_at, "element": {"name": "Name", "text": "Hero"}})).unwrap();
    // Saved as a part of its own; the menu now uses it in its place.
    run(&mut session, "ui.element.saveAsPart", json!({"path": "menu.bui", "at": card, "to": "card.bui"})).unwrap();
    let menu = run(&mut session, "ui.layout.get", json!({"path": "menu.bui"})).unwrap()["layout"]["root"].clone();
    assert_eq!(menu["children"][2]["kind"], "part");
    assert_eq!(menu["children"][2]["layout"], "card.bui");
    assert!(run(&mut session, "ui.element.saveAsPart", json!({"path": "menu.bui", "at": card, "to": "card.bui"})).is_err(), "never over a file");

    // A visual script: pressing Start adds another card, named Second, to the display.
    run(&mut session, "graph.new", json!({"domain": "script"})).unwrap();
    let pressed = node(&registry, &mut session, "interfacePressed");
    let add = node(&registry, &mut session, "addInterfaceElement");
    for (n, name, value) in [(pressed, "element", json!("Start")), (add, "what", json!("card.bui")), (add, "name", json!("Second"))] {
        run(&mut session, "graph.node.set", json!({"node": n, "name": name, "value": value})).unwrap();
    }
    run(&mut session, "graph.link.add", pins(pressed, "then", add, "run")).unwrap();
    run(&mut session, "graph.save", json!({"path": dir.join("hud.bscript")})).unwrap();
    let id = create(
        &registry,
        &mut session,
        json!({"name": "HUD", "components": {"interface": {"layout": "menu.bui", "visible": true, "order": 0}, "scripts": [{"module": "hud.bscript"}]}}),
    );
    let names = |session: &mut Session| -> Vec<String> {
        let state = registry.execute(session, "play.ui.state", &json!({"id": id})).unwrap();
        state["elements"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap().to_owned()).collect()
    };

    run(&mut session, "play.start", Value::Null).unwrap();
    let before = names(&mut session);
    assert!(before.contains(&"Card/Name".to_owned()), "the part's names live under the part: {before:?}");
    run(&mut session, "play.ui.press", json!({"id": id, "element": "Start"})).unwrap();
    let after = names(&mut session);
    assert!(after.contains(&"Second".to_owned()) && after.contains(&"Second/Name".to_owned()), "added while playing: {after:?}");
    assert!(session.console().iter().all(|l| l.level != crate::Level::Error), "{:?}", session.console());
    run(&mut session, "play.stop", Value::Null).unwrap();
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn any_element_with_on_click_sends_its_message_to_the_scripts() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-interface-click-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    registry.execute(&mut session, "project.new", &json!({"path": dir})).unwrap();
    let run = |session: &mut Session, id: &str, params: Value| registry.execute(session, id, &params);
    run(&mut session, "asset.create", json!({"kind": "interface", "folder": "", "name": "menu"})).unwrap();
    // A plain panel that opens a menu when clicked.
    let at = run(&mut session, "ui.element.add", json!({"path": "menu.bui", "kind": "panel"})).unwrap()["at"].clone();
    run(&mut session, "ui.element.set", json!({"path": "menu.bui", "at": at, "element": {"name": "Chest", "onClick": "openMenu"}})).unwrap();
    assert!(
        run(&mut session, "ui.element.set", json!({"path": "menu.bui", "at": at, "element": {"onClick": "x".repeat(65)}})).is_err(),
        "a message name is at most 64 bytes"
    );

    // A visual script: On Message openMenu sets the title.
    run(&mut session, "graph.new", json!({"domain": "script"})).unwrap();
    let on_message = node(&registry, &mut session, "message");
    let set = node(&registry, &mut session, "setInterfaceText");
    for (n, name, value) in [(on_message, "message", json!("openMenu")), (set, "element", json!("Title")), (set, "text", json!("Chest opened"))] {
        run(&mut session, "graph.node.set", json!({"node": n, "name": name, "value": value})).unwrap();
    }
    run(&mut session, "graph.link.add", pins(on_message, "then", set, "run")).unwrap();
    run(&mut session, "graph.save", json!({"path": dir.join("chest.bscript")})).unwrap();
    let id = create(
        &registry,
        &mut session,
        json!({"name": "HUD", "components": {"interface": {"layout": "menu.bui", "visible": true, "order": 0}, "scripts": [{"module": "chest.bscript"}]}}),
    );
    run(&mut session, "play.start", Value::Null).unwrap();
    run(&mut session, "play.ui.press", json!({"id": id, "element": "Chest"})).unwrap();
    let state = run(&mut session, "play.ui.state", json!({"id": id})).unwrap();
    let title = state["elements"].as_array().unwrap().iter().find(|e| e["name"] == "Title").unwrap()["text"].clone();
    assert_eq!(title, "Chest opened", "{:?}", session.console());
    assert!(run(&mut session, "play.ui.press", json!({"id": id, "element": "Title"})).is_err(), "text without On Click is not pressed");
    run(&mut session, "play.stop", Value::Null).unwrap();
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn several_graphs_stay_open_and_undo_goes_back_in_the_graph_each_step_was_made_in() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let run = |session: &mut Session, id: &str, params: Value| registry.execute(session, id, &params).unwrap();
    let material = run(&mut session, "graph.new", json!({"domain": "material"}))["id"].as_u64().unwrap();
    node(&registry, &mut session, "color");
    let script = run(&mut session, "graph.new", json!({"domain": "script"}))["id"].as_u64().unwrap();
    node(&registry, &mut session, "log");
    let list = run(&mut session, "graph.list", Value::Null);
    let open: Vec<(u64, bool)> = list["graphs"].as_array().unwrap().iter().map(|g| (g["id"].as_u64().unwrap(), g["inUse"].as_bool().unwrap())).collect();
    assert_eq!(open, [(material, false), (script, true)], "both stay open; the newest is in use");

    // Undo takes back the script's node, then the material's, each in its own graph.
    let nodes = |session: &Session, id: u64| session.graph_by_id(id).unwrap().graph.nodes.len();
    let (material_nodes, script_nodes) = (nodes(&session, material), nodes(&session, script));
    run(&mut session, "edit.undo", Value::Null);
    assert_eq!((nodes(&session, material), nodes(&session, script)), (material_nodes, script_nodes - 1));
    run(&mut session, "edit.undo", Value::Null);
    assert_eq!((nodes(&session, material), nodes(&session, script)), (material_nodes - 1, script_nodes - 1));
    assert_eq!(session.graph().unwrap().id, material, "the material's step made it the graph in use");

    // Commands work on the graph in use; activate picks another.
    run(&mut session, "graph.activate", json!({"id": script}));
    node(&registry, &mut session, "log");
    assert_eq!(nodes(&session, script), script_nodes);
    assert!(registry.execute(&mut session, "graph.activate", &json!({"id": 999})).is_err());
    // Closing one leaves the other in use.
    run(&mut session, "graph.close", json!({"id": script}));
    assert_eq!(session.graphs().len(), 1);
    assert_eq!(session.graph().unwrap().id, material);
}

#[test]
fn scripts_read_what_the_player_typed_and_set_in_the_interface() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let dir = std::env::temp_dir().join(format!("bayleaf-interface-read-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    registry.execute(&mut session, "project.new", &json!({"path": dir})).unwrap();
    let run = |session: &mut Session, id: &str, params: Value| registry.execute(session, id, &params).unwrap();
    let layout = json!({
        "format": "bayleaf.interface", "version": 1,
        "root": { "kind": "column", "children": [
            { "kind": "text", "name": "Title", "text": "Who?" },
            { "kind": "textField", "name": "Name" },
            { "kind": "slider", "name": "Volume", "max": 1.0 },
            { "kind": "progress", "name": "Bar" }
        ] }
    });
    run(&mut session, "ui.layout.set", json!({"path": "form.bui", "layout": layout}));
    // Typing a name puts it in the title; moving the slider fills the bar to match.
    run(&mut session, "graph.new", json!({"domain": "script"}));
    let typed = node(&registry, &mut session, "interfaceChanged");
    let name = node(&registry, &mut session, "interfaceText");
    let title = node(&registry, &mut session, "setInterfaceText");
    let moved = node(&registry, &mut session, "interfaceChanged");
    let volume = node(&registry, &mut session, "interfaceValue");
    let bar = node(&registry, &mut session, "setInterfaceValue");
    for (n, setting, value) in [
        (typed, "element", "Name"),
        (name, "element", "Name"),
        (title, "element", "Title"),
        (moved, "element", "Volume"),
        (volume, "element", "Volume"),
        (bar, "element", "Bar"),
    ] {
        run(&mut session, "graph.node.set", json!({"node": n, "name": setting, "value": value}));
    }
    run(&mut session, "graph.link.add", pins(typed, "then", title, "run"));
    run(&mut session, "graph.link.add", pins(name, "text", title, "text"));
    run(&mut session, "graph.link.add", pins(moved, "then", bar, "run"));
    run(&mut session, "graph.link.add", pins(volume, "value", bar, "value"));
    run(&mut session, "graph.save", json!({"path": dir.join("form.bscript")}));
    let id = create(
        &registry,
        &mut session,
        json!({"name": "Form", "components": {"interface": {"layout": "form.bui", "visible": true, "order": 0}, "scripts": [{"module": "form.bscript"}]}}),
    );
    let shown = |session: &mut Session, element: &str| {
        let state = registry.execute(session, "play.ui.state", &json!({"id": id})).unwrap();
        state["elements"].as_array().unwrap().iter().find(|e| e["name"] == element).cloned().unwrap()
    };
    run(&mut session, "play.start", Value::Null);
    run(&mut session, "play.ui.change", json!({"id": id, "element": "Name", "text": "Ada"}));
    assert_eq!(shown(&mut session, "Title")["text"], "Ada", "{:?}", session.console());
    run(&mut session, "play.ui.change", json!({"id": id, "element": "Volume", "value": 0.4}));
    run(&mut session, "play.step", json!({"seconds": 0.02}));
    let filled = shown(&mut session, "Bar")["value"].as_f64().unwrap();
    assert!((filled - 0.4).abs() < 1e-5, "the bar follows the slider: {filled}");
    run(&mut session, "play.stop", Value::Null);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_visual_script_bursts_and_stops_an_emitter() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let run = |session: &mut Session, id: &str, params: Value| registry.execute(session, id, &params).unwrap();
    run(&mut session, "graph.new", json!({"domain": "script"}));
    let start = session.graph().unwrap().graph.nodes.iter().find(|n| n.kind == "start").unwrap().id;
    let burst = node(&registry, &mut session, "burstParticles");
    run(&mut session, "graph.node.set", json!({"node": burst, "name": "count", "value": 10}));
    let stop = node(&registry, &mut session, "stopParticles");
    run(&mut session, "graph.link.add", pins(u64::from(start), "then", burst, "run"));
    run(&mut session, "graph.link.add", pins(burst, "then", stop, "run"));
    let dir = std::env::temp_dir().join(format!("bayleaf-particle-nodes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    run(&mut session, "graph.save", json!({"path": dir.join("burst.bscript")}));
    let emitter = json!({"rate": 0.0, "burst": 0, "lifetime": [5.0, 5.0], "playing": true});
    let mut value = serde_json::to_value(bayleaf_scene::components::ParticleEmitter::default()).unwrap();
    for (k, v) in emitter.as_object().unwrap() {
        value[k] = v.clone();
    }
    create(&registry, &mut session, json!({"name": "Fountain", "components": {"particleEmitter": value, "scripts": [{"module": "burst.bscript"}]}}));
    run(&mut session, "scene.saveAs", json!({"path": dir.join("burst.bscene")}));
    run(&mut session, "play.start", Value::Null);
    run(&mut session, "play.step", json!({"seconds": 0.05, "steps": 3}));
    assert_eq!(session.particles().unwrap().count(None), 10, "the burst's ten, and no more once it stopped");
    run(&mut session, "play.stop", Value::Null);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_showcase_example_opens_and_everything_it_names_reads() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let project = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/showcase/Showcase.bproject");
    registry.execute(&mut session, "project.open", &json!({"path": project})).unwrap();
    for model in ["models/windmill.gltf", "models/crate.obj", "materials/ground.bmaterial"] {
        let info = registry.execute(&mut session, "asset.info", &json!({"path": model})).unwrap();
        assert_eq!(info["problems"], json!([]), "{model}: {info}");
    }
    let windmill = registry.execute(&mut session, "asset.info", &json!({"path": "models/windmill.gltf"})).unwrap();
    assert!(windmill.to_string().contains("Spin"), "the windmill keeps its animation");
    for layout in ["ui/hud.bui", "ui/sign.bui", "ui/card.bui"] {
        registry.execute(&mut session, "ui.layout.get", &json!({"path": layout})).unwrap();
    }
    let hud = bayleaf_interface::Layout::load(&session.script_dir().join("ui/hud.bui")).unwrap();
    let mut load = |path: &str| bayleaf_interface::Layout::load(&session.script_dir().join(path)).map_err(|e| e.to_string());
    bayleaf_interface::expand(&hud, &mut load).expect("the cards' texts name elements inside the card");
    let post = registry.execute(&mut session, "post.get", &json!({"path": "post/showcase.bpost"})).unwrap();
    assert_eq!(post["post"]["color"]["lookup"], "post/warm.png");
    let objects = registry.execute(&mut session, "scene.objects", &json!({})).unwrap().to_string();
    for name in ["Windmill", "Crate", "Spark Orb", "Sign", "HUD"] {
        assert!(objects.contains(name), "{name} is in the scene");
    }
}

#[test]
fn an_events_nodes_follow_its_new_name_and_values() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    registry.execute(&mut session, "graph.new", &json!({"domain": "script"})).unwrap();
    registry.execute(&mut session, "graph.event.add", &json!({"name": "Open"})).unwrap();
    assert!(registry.execute(&mut session, "graph.event.add", &json!({"name": "Open"})).is_err(), "one event of a name");
    let on = registry.execute(&mut session, "graph.node.add", &json!({"kind": "eventStart", "position": [0, 0]})).unwrap()["node"].clone();
    let trigger = registry.execute(&mut session, "graph.node.add", &json!({"kind": "triggerEvent", "position": [0, 200]})).unwrap()["node"].clone();
    for node in [&on, &trigger] {
        registry.execute(&mut session, "graph.node.set", &json!({"node": node, "name": "event", "value": "Open"})).unwrap();
    }
    let speed = json!([{"name": "speed", "type": "number"}]);
    registry.execute(&mut session, "graph.event.set", &json!({"name": "Open", "rename": "OpenDoor", "params": speed})).unwrap();
    let graph = &session.graph().unwrap().graph;
    for node in [&on, &trigger] {
        let id = u32::try_from(node.as_u64().unwrap()).unwrap();
        assert_eq!(graph.value(id, "event"), Some(&json!("OpenDoor")), "the node follows the new name");
        assert_eq!(graph.value(id, "params"), Some(&json!(["speed:number"])), "and its values");
    }
    assert!(
        registry.execute(&mut session, "graph.event.set", &json!({"name": "OpenDoor", "params": [{"name": "run", "type": "number"}]})).is_err(),
        "a flow pin's name"
    );
    registry.execute(&mut session, "graph.event.remove", &json!({"name": "OpenDoor"})).unwrap();
    assert!(session.graph().unwrap().graph.events.is_empty());
}

#[test]
fn a_variables_nodes_follow_a_new_type_and_shape_and_lose_links_that_no_longer_fit() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let run = |session: &mut Session, id: &str, params: Value| registry.execute(session, id, &params);
    run(&mut session, "graph.new", json!({"domain": "script"})).unwrap();
    run(&mut session, "graph.variable.add", json!({"name": "count", "type": "number", "start": 3.0})).unwrap();
    let get = run(&mut session, "graph.node.add", json!({"kind": "variable", "position": [0, 0]})).unwrap()["node"].clone();
    run(&mut session, "graph.node.set", json!({"node": get, "name": "name", "value": "count"})).unwrap();
    let log = run(&mut session, "graph.node.add", json!({"kind": "log", "position": [200, 0]})).unwrap()["node"].clone();
    run(&mut session, "graph.link.add", json!({"from": {"node": get, "pin": "value"}, "to": {"node": log, "pin": "value"}})).unwrap();

    // A number becomes text: its Get node becomes a Get of text, and the number link goes.
    run(&mut session, "graph.variable.set", json!({"name": "count", "type": "text"})).unwrap();
    let graph = &session.graph().unwrap().graph;
    let id = u32::try_from(get.as_u64().unwrap()).unwrap();
    assert_eq!(graph.node(id).map(|n| n.kind.as_str()), Some("textVariable"), "the node keeps its place as the new type's");
    assert_eq!(graph.variable("count").unwrap().start, json!(""), "3 is no text, so it starts empty");
    assert!(graph.links.iter().all(|l| l.from.node != id), "text does not go into a number input");

    // One value becomes a list, which starts with items and gets list nodes.
    run(&mut session, "graph.variable.set", json!({"name": "count", "shape": "list", "start": ["a", "b"]})).unwrap();
    assert!(run(&mut session, "graph.variable.set", json!({"name": "count", "start": [1, 2]})).is_err(), "a list of text holds text");
    let add = run(&mut session, "graph.node.add", json!({"kind": "listAdd", "position": [0, 200]})).unwrap()["node"].clone();
    run(&mut session, "graph.node.set", json!({"node": add, "name": "item", "value": ["text"]})).unwrap();
    run(&mut session, "graph.node.set", json!({"node": add, "name": "name", "value": "count"})).unwrap();
    let name = run(&mut session, "graph.node.add", json!({"kind": "nameOf", "position": [0, 400]})).unwrap()["node"].clone();
    run(&mut session, "graph.link.add", json!({"from": {"node": name, "pin": "result"}, "to": {"node": add, "pin": "value"}})).unwrap();
    run(&mut session, "graph.variable.set", json!({"name": "count", "type": "number", "rename": "scores"})).unwrap();
    let graph = &session.graph().unwrap().graph;
    let id = u32::try_from(add.as_u64().unwrap()).unwrap();
    assert_eq!(graph.value(id, "name"), Some(&json!("scores")), "list nodes follow the new name");
    assert_eq!(graph.value(id, "item"), Some(&json!(["number", "text"])), "and what the list now holds");
    assert!(graph.links.iter().all(|l| l.to.node != id || l.to.pin != "value"), "text no longer goes into a list of numbers");
    assert_eq!(graph.variable("scores").unwrap().start, json!([]), "text items do not fit numbers");

    // A property's shape and type change the same way.
    run(&mut session, "graph.property.add", json!({"name": "waves", "type": "wholeNumber", "shape": "map", "default": {"easy": 3}})).unwrap();
    run(&mut session, "graph.property.set", json!({"name": "waves", "shape": "list"})).unwrap();
    assert_eq!(session.graph().unwrap().graph.property("waves").unwrap().default, json!([]), "a map is no list, so the list starts empty");
}

#[test]
fn linking_a_list_into_a_list_node_gives_the_node_its_type_in_one_step() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let run = |session: &mut Session, id: &str, params: Value| registry.execute(session, id, &params);
    run(&mut session, "graph.new", json!({"domain": "script"})).unwrap();
    run(&mut session, "graph.variable.add", json!({"name": "words", "type": "text", "shape": "list"})).unwrap();
    let get = run(&mut session, "graph.node.add", json!({"kind": "getList", "position": [0, 0]})).unwrap()["node"].clone();
    run(&mut session, "graph.node.set", json!({"node": get, "name": "item", "value": ["text"]})).unwrap();
    run(&mut session, "graph.node.set", json!({"node": get, "name": "name", "value": "words"})).unwrap();
    // A fresh For Each holds numbers; a number goes into nothing it has, so no link is lost.
    let each = run(&mut session, "graph.node.add", json!({"kind": "listForEach", "position": [200, 0]})).unwrap()["node"].clone();
    run(&mut session, "graph.link.add", json!({"from": {"node": get, "pin": "list"}, "to": {"node": each, "pin": "list"}})).unwrap();
    let graph = &session.graph().unwrap().graph;
    let id = u32::try_from(each.as_u64().unwrap()).unwrap();
    assert_eq!(graph.value(id, "item"), Some(&json!(["text", "text"])), "the node now goes through text");
    assert!(graph.links.iter().any(|l| l.to.node == id && l.to.pin == "list"), "and the list is linked");
    // Undo takes back both.
    run(&mut session, "edit.undo", Value::Null).unwrap();
    assert_eq!(session.graph().unwrap().graph.value(id, "item"), None);
}

#[test]
fn a_maps_key_type_changes_and_its_nodes_and_start_follow() {
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let run = |session: &mut Session, id: &str, params: Value| registry.execute(session, id, &params);
    run(&mut session, "graph.new", json!({"domain": "script"})).unwrap();
    run(&mut session, "graph.variable.add", json!({"name": "waves", "type": "text", "shape": "map", "start": {"1": "one", "easy": "?"}})).unwrap();
    assert!(
        run(&mut session, "graph.variable.add", json!({"name": "bad", "type": "text", "shape": "map", "key": "wholeNumber", "start": {"easy": "?"}})).is_err(),
        "easy is no whole number"
    );
    let set = run(&mut session, "graph.node.add", json!({"kind": "mapSet", "position": [0, 0]})).unwrap()["node"].clone();
    run(&mut session, "graph.node.set", json!({"node": set, "name": "item", "value": ["text", "text"]})).unwrap();
    run(&mut session, "graph.node.set", json!({"node": set, "name": "name", "value": "waves"})).unwrap();
    run(&mut session, "graph.variable.set", json!({"name": "waves", "key": "wholeNumber"})).unwrap();
    let graph = &session.graph().unwrap().graph;
    let id = u32::try_from(set.as_u64().unwrap()).unwrap();
    assert_eq!(graph.value(id, "item"), Some(&json!(["text", "whole"])), "the node's key is a whole number now");
    assert_eq!(graph.variable("waves").unwrap().start, json!({}), "easy is no whole number, so the map starts empty");
    let (_, def) = graph.node_def(&bayleaf_script::visual::CATALOG, id).unwrap();
    assert_eq!(def.input("key").map(|p| p.ty), Some(bayleaf_graph::PinType::Int), "and its Key pin takes one");
}

/// Moves its object to x = the `hp` field of `Goblin.bdata`, each step.
fn data_reader_wasm() -> Vec<u8> {
    wat::parse_str(
        r#"(module
            (import "bayleaf" "transform_get" (func $get (param i32)))
            (import "bayleaf" "transform_set" (func $set (param i32)))
            (import "bayleaf" "data_get" (func $data (param i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 100) "Goblin.bdata")
            (data (i32.const 120) "hp")
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_update") (param $dt f32)
              (call $get (i32.const 0))
              (drop (call $data (i32.const 100) (i32.const 12) (i32.const 120) (i32.const 2) (i32.const 64)))
              (f32.store (i32.const 0) (f32.load (i32.const 64)))
              (call $set (i32.const 0))))"#,
    )
    .unwrap()
}

#[test]
fn data_files_change_through_commands_and_play_reads_them_as_saved_never_writing() {
    let (registry, mut session, id, dir) = scripted_scene("data", &data_reader_wasm());
    let run = |session: &mut Session, id: &str, params: Value| registry.execute(session, id, &params);
    let x = |session: &Session| {
        let id = bayleaf_core::StableId::parse_hex(&id).unwrap();
        session.scene().component(id, "transform").unwrap()["position"][0].as_f64().unwrap()
    };
    run(&mut session, "asset.create", json!({"kind": "dataShape", "folder": ".", "name": "Enemy"})).unwrap();
    run(&mut session, "data.shape.set", json!({"path": "Enemy.bshape", "shape": {"fields": [{"name": "hp", "type": "number", "default": 100.0}]}})).unwrap();
    run(&mut session, "asset.create", json!({"kind": "dataFile", "folder": ".", "name": "Goblin"})).unwrap();
    run(&mut session, "data.set", json!({"path": "Goblin.bdata", "data": {"shape": "Enemy.bshape"}})).unwrap();
    let read = run(&mut session, "data.get", json!({"path": "Goblin.bdata"})).unwrap();
    assert_eq!((read["fields"][0]["value"].clone(), read["fields"][0]["set"].clone()), (json!(100.0), json!(false)), "the shape's default, not set");
    run(&mut session, "data.field.set", json!({"path": "Goblin.bdata", "name": "hp", "value": 40.0})).unwrap();
    assert!(run(&mut session, "data.field.set", json!({"path": "Goblin.bdata", "name": "speed", "value": 1.0})).is_err(), "a field the shape lacks");
    assert!(run(&mut session, "data.field.set", json!({"path": "Goblin.bdata", "name": "hp", "value": "lots"})).is_err(), "a value of another type");
    assert!(
        run(&mut session, "data.shape.set", json!({"path": "Enemy.bshape", "shape": {"fields": [{"name": "hp", "type": "number", "default": "x"}]}})).is_err()
    );

    // Play reads the file as it was saved when play started; a change while playing waits for the
    // next play, and play itself never writes the file.
    run(&mut session, "play.start", Value::Null).unwrap();
    run(&mut session, "play.step", json!({"seconds": 0.1})).unwrap();
    assert_eq!(x(&session), 40.0);
    run(&mut session, "data.field.set", json!({"path": "Goblin.bdata", "name": "hp", "value": 7.0})).unwrap();
    let saved = std::fs::read_to_string(dir.join("Goblin.bdata")).unwrap();
    run(&mut session, "play.step", json!({"seconds": 0.1})).unwrap();
    assert_eq!(x(&session), 40.0, "still the value play started with");
    run(&mut session, "play.stop", Value::Null).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("Goblin.bdata")).unwrap(), saved, "play wrote nothing");
    run(&mut session, "play.start", Value::Null).unwrap();
    run(&mut session, "play.step", json!({"seconds": 0.1})).unwrap();
    assert_eq!(x(&session), 7.0, "the next play reads the change");
    run(&mut session, "play.stop", Value::Null).unwrap();

    run(&mut session, "data.field.set", json!({"path": "Goblin.bdata", "name": "hp", "value": null})).unwrap();
    assert_eq!(run(&mut session, "data.get", json!({"path": "Goblin.bdata"})).unwrap()["fields"][0]["value"], json!(100.0), "back to the default");

    // Agents read inside their read folders and write only inside their write folders.
    let elsewhere = dir.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    session.set_confinement(Some(crate::confine::Confinement::new(std::slice::from_ref(&dir), std::slice::from_ref(&elsewhere)).unwrap()));
    let before = std::fs::read_to_string(dir.join("Goblin.bdata")).unwrap();
    assert!(run(&mut session, "data.get", json!({"path": dir.join("Goblin.bdata")})).is_ok());
    assert!(run(&mut session, "data.field.set", json!({"path": dir.join("Goblin.bdata"), "name": "hp", "value": 1.0})).is_err());
    assert!(run(&mut session, "data.shape.set", json!({"path": dir.join("Enemy.bshape"), "shape": {"fields": []}})).is_err());
    assert_eq!(std::fs::read_to_string(dir.join("Goblin.bdata")).unwrap(), before, "a refused write changes nothing");
    std::fs::remove_dir_all(&dir).ok();
}
