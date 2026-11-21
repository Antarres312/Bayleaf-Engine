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
