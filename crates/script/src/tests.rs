use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use bayleaf_core::StableId;
use bayleaf_scene::Scene;
use bayleaf_scene::components::{DeclaredProperty, PropertyType, PropertyValue, ScriptRef, Scripts, Transform};

use crate::{Input, ScriptError, ScriptHost};

const IMPORTS: &str = r#"
  (import "bayleaf" "log" (func $log (param i32 i32)))
  (import "bayleaf" "transform_get" (func $get (param i32)))
  (import "bayleaf" "transform_set" (func $set (param i32)))
  (import "bayleaf" "property" (func $prop (param i32 i32) (result f32)))
  (import "bayleaf" "key_down" (func $key (param i32 i32) (result i32)))
"#;

/// A module with the standard imports, one page of memory, `bayleaf_abi` returning `abi`, and `body`.
fn module(abi: i32, body: &str) -> Vec<u8> {
    let text = format!(
        r#"(module {IMPORTS}
            (memory (export "memory") 1)
            (data (i32.const 100) "speed")
            (data (i32.const 120) "hello")
            (data (i32.const 140) "W")
            (func (export "bayleaf_abi") (result i32) i32.const {abi})
            {body})"#
    );
    wat::parse_str(text).unwrap()
}

/// Turns the object `speed` degrees per second around Y.
const ROTATE: &str = r#"
  (func (export "bayleaf_update") (param $dt f32)
    (call $get (i32.const 0))
    (f32.store (i32.const 16)
      (f32.add (f32.load (i32.const 16)) (f32.mul (call $prop (i32.const 100) (i32.const 5)) (local.get $dt))))
    (call $set (i32.const 0)))"#;

fn setup(properties: &[(&str, f32)]) -> (Scene, StableId, ScriptRef) {
    let mut scene = Scene::new();
    let id = scene.create_object(None, "Scripted").unwrap();
    let script = ScriptRef {
        module: "test.wasm".into(),
        properties: properties.iter().map(|(k, v)| ((*k).to_owned(), PropertyValue::Number(f64::from(*v)))).collect::<BTreeMap<_, _>>(),
    };
    (scene, id, script)
}

fn load(host: &mut ScriptHost, scene: &mut Scene, id: StableId, script: &ScriptRef, bytes: &[u8]) -> Result<(), ScriptError> {
    load_at(host, scene, id, 0, script, bytes)
}

fn load_at(host: &mut ScriptHost, scene: &mut Scene, id: StableId, index: usize, script: &ScriptRef, bytes: &[u8]) -> Result<(), ScriptError> {
    host.load_bytes(id, index, Path::new("test.wasm"), None, bytes, script, scene, 0.0, &Arc::new(Input::default()))
}

fn rotation_y(scene: &Scene, id: StableId) -> f32 {
    scene.get::<Transform>(id).unwrap().rotation[1]
}

#[test]
fn update_moves_the_object_by_its_property() {
    let (mut scene, id, script) = setup(&[("speed", 90.0)]);
    let mut host = ScriptHost::new();
    load(&mut host, &mut scene, id, &script, &module(1, ROTATE)).unwrap();
    let input = Arc::new(Input::default());
    for _ in 0..10 {
        host.update(&mut scene, 0.1, 0.0, &input);
    }
    assert!((rotation_y(&scene, id) - 90.0).abs() < 1e-3, "one second at 90°/s");
    assert!(host.take_faults().is_empty());
}

#[test]
fn start_runs_once_and_its_log_reaches_the_host() {
    let (mut scene, id, script) = setup(&[]);
    let mut host = ScriptHost::new();
    let body = r#"(func (export "bayleaf_start") (call $log (i32.const 120) (i32.const 5)))"#;
    load(&mut host, &mut scene, id, &script, &module(1, body)).unwrap();
    let logs = host.take_logs();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].text, "hello");
    assert_eq!(logs[0].object, id);
}

#[test]
fn an_endless_loop_is_stopped_and_other_scripts_keep_running() {
    let (mut scene, looping, script) = setup(&[("speed", 90.0)]);
    let healthy = scene.create_object(None, "Healthy").unwrap();
    let mut host = ScriptHost::new();
    let endless = r#"(func (export "bayleaf_update") (param f32) (loop $l (br $l)))"#;
    load(&mut host, &mut scene, looping, &script, &module(1, endless)).unwrap();
    load(&mut host, &mut scene, healthy, &script, &module(1, ROTATE)).unwrap();
    host.update(&mut scene, 1.0, 0.0, &Arc::new(Input::default()));
    let faults = host.take_faults();
    assert_eq!(faults.len(), 1);
    assert_eq!(faults[0].object, looping);
    assert!(faults[0].error.to_string().contains("limit"), "{}", faults[0].error);
    assert!(rotation_y(&scene, healthy) > 0.0);
    assert_eq!(host.running(), 1);
}

#[test]
fn modules_that_break_the_abi_are_refused() {
    let (mut scene, id, script) = setup(&[]);
    let mut host = ScriptHost::new();
    assert!(matches!(load(&mut host, &mut scene, id, &script, &module(7, "")), Err(ScriptError::AbiMismatch { found: 7, .. })));
    let no_abi = wat::parse_str(r#"(module (memory (export "memory") 1))"#).unwrap();
    assert!(matches!(load(&mut host, &mut scene, id, &script, &no_abi), Err(ScriptError::Invalid(..))));
    assert!(matches!(load(&mut host, &mut scene, id, &script, b"not wasm"), Err(ScriptError::Invalid(..))));
    let unknown_import = wat::parse_str(
        r#"(module (import "bayleaf" "launch_missiles" (func)) (memory (export "memory") 1) (func (export "bayleaf_abi") (result i32) i32.const 1))"#,
    )
    .unwrap();
    assert!(load(&mut host, &mut scene, id, &script, &unknown_import).is_err());
}

#[test]
fn bad_pointers_and_values_stop_only_that_script_with_a_reason() {
    let input = Arc::new(Input::default());
    for (body, expected) in [
        (r#"(func (export "bayleaf_update") (param f32) (call $log (i32.const 65530) (i32.const 100)))"#, "outside"),
        (r#"(func (export "bayleaf_update") (param f32) (call $log (i32.const -1) (i32.const 3)))"#, "negative"),
        (r#"(func (export "bayleaf_update") (param f32) (f32.store (i32.const 0) (f32.div (f32.const 0) (f32.const 0))) (call $set (i32.const 0)))"#, "finite"),
        (r#"(func (export "bayleaf_update") (param f32) unreachable)"#, "unreachable"),
    ] {
        let (mut scene, id, script) = setup(&[]);
        let before = scene.get::<Transform>(id).cloned();
        let mut host = ScriptHost::new();
        load(&mut host, &mut scene, id, &script, &module(1, body)).unwrap();
        host.update(&mut scene, 0.1, 0.0, &input);
        let faults = host.take_faults();
        assert_eq!(faults.len(), 1, "{body}");
        assert!(faults[0].error.to_string().contains(expected), "{} does not mention {expected}", faults[0].error);
        assert_eq!(scene.get::<Transform>(id).cloned(), before, "a failing call must not change the object");
        host.update(&mut scene, 0.1, 0.0, &input);
        assert!(host.take_faults().is_empty(), "a stopped script does not run again");
    }
}

#[test]
fn key_down_sees_the_keys_held_this_frame() {
    let (mut scene, id, script) = setup(&[]);
    let mut host = ScriptHost::new();
    // position.x = key_down("W")
    let body = r#"(func (export "bayleaf_update") (param f32)
        (call $get (i32.const 0))
        (f32.store (i32.const 0) (f32.convert_i32_s (call $key (i32.const 140) (i32.const 1))))
        (call $set (i32.const 0)))"#;
    load(&mut host, &mut scene, id, &script, &module(1, body)).unwrap();
    host.update(&mut scene, 0.1, 0.0, &Arc::new(Input::default()));
    assert_eq!(scene.get::<Transform>(id).unwrap().position[0], 0.0);
    let held = Arc::new(Input { keys: ["W".to_owned()].into_iter().collect(), ..Input::default() });
    host.update(&mut scene, 0.1, 0.0, &held);
    assert_eq!(scene.get::<Transform>(id).unwrap().position[0], 1.0);
}

#[test]
fn a_changed_module_file_is_reloaded_and_restarted() {
    let dir = std::env::temp_dir().join(format!("bayleaf-script-reload-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("hot.wasm");
    std::fs::write(&path, module(1, "")).unwrap();
    let (mut scene, id, mut script) = setup(&[]);
    script.module = "hot.wasm".into();
    scene.set(id, Scripts(vec![script.clone()])).unwrap();

    let mut host = ScriptHost::new();
    let input = Arc::new(Input::default());
    host.load_scene(&mut scene, &dir, 0.0, &input);
    assert!(host.take_faults().is_empty());
    assert!(host.reload_changed(&mut scene, 0.0, &input).is_empty(), "nothing changed yet");

    let body = r#"(func (export "bayleaf_start") (call $log (i32.const 120) (i32.const 5)))"#;
    std::fs::write(&path, module(1, body)).unwrap();
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    std::fs::File::options().write(true).open(&path).unwrap().set_modified(later).unwrap();
    assert_eq!(host.reload_changed(&mut scene, 0.0, &input), ["hot.wasm"]);
    assert_eq!(host.take_logs().len(), 1, "the new module's start ran");
    assert_eq!(host.len(), 1, "the old instance was replaced");
    std::fs::remove_dir_all(&dir).ok();
}

/// A module declaring `speed` = 45 through `bayleaf_describe`, then rotating by it.
fn described(body_extra: &str) -> Vec<u8> {
    let json = r#"{\"properties\": [[\"speed\", 45.0]]}"#;
    let len = json.replace("\\\"", "\"").len();
    module(
        1,
        &format!(
            r#"(data (i32.const 400) "{json}")
               (func (export "bayleaf_describe") (result i64) (i64.or (i64.shl (i64.const 400) (i64.const 32)) (i64.const {len})))
               {ROTATE} {body_extra}"#
        ),
    )
}

#[test]
fn declared_defaults_apply_unless_the_scene_overrides_them() {
    let input = Arc::new(Input::default());
    let (mut scene, id, script) = setup(&[]);
    let mut host = ScriptHost::new();
    load(&mut host, &mut scene, id, &script, &described("")).unwrap();
    host.update(&mut scene, 1.0, 0.0, &input);
    assert!((rotation_y(&scene, id) - 45.0).abs() < 1e-3, "the declared default 45 is used");

    let (mut scene, id, script) = setup(&[("speed", 10.0)]);
    let mut host = ScriptHost::new();
    load(&mut host, &mut scene, id, &script, &described("")).unwrap();
    host.update(&mut scene, 1.0, 0.0, &input);
    assert!((rotation_y(&scene, id) - 10.0).abs() < 1e-3, "the scene's value wins");

    let declared = host.describe("d.wasm", &described("")).unwrap();
    assert_eq!(declared, [DeclaredProperty { name: "speed".to_owned(), ty: PropertyType::Number, default: PropertyValue::Number(45.0) }]);
    assert!(host.describe("plain.wasm", &module(1, "")).unwrap().is_empty());
}

#[test]
fn two_scripts_on_one_object_both_run() {
    let (mut scene, id, script) = setup(&[("speed", 30.0)]);
    let mut host = ScriptHost::new();
    load_at(&mut host, &mut scene, id, 0, &script, &module(1, ROTATE)).unwrap();
    load_at(&mut host, &mut scene, id, 1, &script, &module(1, ROTATE)).unwrap();
    assert_eq!(host.len(), 2);
    host.update(&mut scene, 1.0, 0.0, &Arc::new(Input::default()));
    assert!((rotation_y(&scene, id) - 60.0).abs() < 1e-3, "each script turned the object 30°");
}

#[test]
fn scripts_move_other_objects_read_the_mouse_and_ask_for_spawns_and_destroys() {
    let text = r#"(module
        (import "bayleaf" "find" (func $find (param i32 i32) (result i32)))
        (import "bayleaf" "transform_get_of" (func $get_of (param i32 i32)))
        (import "bayleaf" "transform_set_of" (func $set_of (param i32 i32)))
        (import "bayleaf" "mouse_down" (func $mouse (param i32) (result i32)))
        (import "bayleaf" "pointer" (func $pointer (param i32) (result i32)))
        (import "bayleaf" "spawn" (func $spawn (param i32 i32 f32 f32 f32) (result i32)))
        (import "bayleaf" "destroy" (func $destroy (param i32) (result i32)))
        (memory (export "memory") 1)
        (data (i32.const 100) "Target")
        (data (i32.const 120) "Junk")
        (data (i32.const 140) "crate.bprefab")
        (func (export "bayleaf_abi") (result i32) i32.const 1)
        (func (export "bayleaf_update") (param $dt f32) (local $h i32)
          (local.set $h (call $find (i32.const 100) (i32.const 6)))
          (call $get_of (local.get $h) (i32.const 0))
          (f32.store (i32.const 0) (f32.add (f32.load (i32.const 0)) (f32.const 1)))
          (call $set_of (local.get $h) (i32.const 0))
          ;; The pointer's x goes into the target's z.
          (drop (call $pointer (i32.const 64)))
          (call $get_of (local.get $h) (i32.const 0))
          (f32.store (i32.const 8) (f32.load (i32.const 64)))
          (call $set_of (local.get $h) (i32.const 0))
          (if (call $mouse (i32.const 0))
            (then
              (drop (call $spawn (i32.const 140) (i32.const 13) (f32.const 1) (f32.const 2) (f32.const 3)))
              (drop (call $destroy (call $find (i32.const 120) (i32.const 4))))))))"#;
    let bytes = wat::parse_str(text).unwrap();
    let (mut scene, mover, script) = setup(&[]);
    let target = scene.create_object(None, "Target").unwrap();
    let junk = scene.create_object(None, "Junk").unwrap();
    let mut host = ScriptHost::new();
    load(&mut host, &mut scene, mover, &script, &bytes).unwrap();

    let idle = Arc::new(Input { pointer: Some([0.25, -0.5]), ..Input::default() });
    host.update(&mut scene, 0.1, 0.0, &idle);
    let t = scene.get::<Transform>(target).unwrap().clone();
    assert_eq!(t.position[0], 1.0, "another object moved");
    assert_eq!(t.position[2], 0.25, "the pointer was read");
    assert!(host.take_requests().is_empty(), "nothing asked without the button");
    assert!(host.take_faults().is_empty());

    let clicking = Arc::new(Input { buttons: 1, ..Input::default() });
    host.update(&mut scene, 0.1, 0.0, &clicking);
    assert_eq!(
        host.take_requests(),
        [crate::Request::Spawn { by: mover, prefab: "crate.bprefab".into(), position: [1.0, 2.0, 3.0] }, crate::Request::Destroy(junk)]
    );
    assert!(scene.contains(junk), "the play loop does the destroy, not the host");
}

#[test]
fn bad_handles_stop_the_script_with_a_reason() {
    let text = r#"(module
        (import "bayleaf" "transform_get_of" (func $get_of (param i32 i32)))
        (memory (export "memory") 1)
        (func (export "bayleaf_abi") (result i32) i32.const 1)
        (func (export "bayleaf_update") (param f32) (call $get_of (i32.const 999) (i32.const 0))))"#;
    let (mut scene, id, script) = setup(&[]);
    let mut host = ScriptHost::new();
    load(&mut host, &mut scene, id, &script, &wat::parse_str(text).unwrap()).unwrap();
    host.update(&mut scene, 0.1, 0.0, &Arc::new(Input::default()));
    let faults = host.take_faults();
    assert!(faults[0].error.to_string().contains("no object with handle 999"), "{}", faults[0].error);
}
