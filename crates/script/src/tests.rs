use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use bayleaf_core::StableId;
use bayleaf_scene::Scene;
use bayleaf_scene::components::{DeclaredProperty, PropertyType, PropertyValue, ScriptRef, Scripts, Transform};

use super::host::ScriptHost;
use crate::{Input, ScriptError};

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
    let mut host = super::test_host();
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
    let mut host = super::test_host();
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
    let mut host = super::test_host();
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
fn memory_stays_within_the_limit() {
    let (mut scene, id, script) = setup(&[]);
    let mut host = super::test_host();
    let pages = |bytes: usize| bytes / 65536;
    // Growing past the limit fails (the script sees -1 and logs); growing within it works.
    let body = format!(
        r#"(func (export "bayleaf_start")
             (if (i32.eq (memory.grow (i32.const {over})) (i32.const -1)) (then (call $log (i32.const 120) (i32.const 5))))
             (if (i32.ne (memory.grow (i32.const 4)) (i32.const -1)) (then (call $log (i32.const 140) (i32.const 1)))))"#,
        over = pages(crate::MEMORY_LIMIT) + 1
    );
    load(&mut host, &mut scene, id, &script, &module(1, &body)).unwrap();
    let logs: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(logs, ["hello", "W"], "the large growth was refused and the small one allowed");

    // A module that asks for more memory than the limit at the start does not load.
    let greedy = wat::parse_str(format!(
        r#"(module (memory (export "memory") {}) (func (export "bayleaf_abi") (result i32) i32.const 1))"#,
        pages(crate::MEMORY_LIMIT) + 1
    ))
    .unwrap();
    let (mut scene, id, script) = setup(&[]);
    assert!(load(&mut host, &mut scene, id, &script, &greedy).is_err());
}

#[test]
fn modules_that_break_the_abi_are_refused() {
    let (mut scene, id, script) = setup(&[]);
    let mut host = super::test_host();
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
        let mut host = super::test_host();
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
    let mut host = super::test_host();
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
    let dir = std::env::temp_dir().join(format!("bayleaf-script-reload-{}-{}", std::process::id(), module_path!().replace("::", "-")));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("hot.wasm");
    std::fs::write(&path, module(1, "")).unwrap();
    let (mut scene, id, mut script) = setup(&[]);
    script.module = "hot.wasm".into();
    scene.set(id, Scripts(vec![script.clone()])).unwrap();

    let mut host = super::test_host();
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
    let mut host = super::test_host();
    load(&mut host, &mut scene, id, &script, &described("")).unwrap();
    host.update(&mut scene, 1.0, 0.0, &input);
    assert!((rotation_y(&scene, id) - 45.0).abs() < 1e-3, "the declared default 45 is used");

    let (mut scene, id, script) = setup(&[("speed", 10.0)]);
    let mut host = super::test_host();
    load(&mut host, &mut scene, id, &script, &described("")).unwrap();
    host.update(&mut scene, 1.0, 0.0, &input);
    assert!((rotation_y(&scene, id) - 10.0).abs() < 1e-3, "the scene's value wins");

    let declared = host.describe("d.wasm", &described("")).unwrap();
    assert_eq!(
        declared,
        [DeclaredProperty {
            name: "speed".to_owned(),
            ty: PropertyType::Number,
            shape: bayleaf_scene::components::PropertyShape::Single,
            default: PropertyValue::Number(45.0)
        }]
    );
    assert!(host.describe("plain.wasm", &module(1, "")).unwrap().is_empty());
}

#[test]
fn two_scripts_on_one_object_both_run() {
    let (mut scene, id, script) = setup(&[("speed", 30.0)]);
    let mut host = super::test_host();
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
    let mut host = super::test_host();
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
    let mut host = super::test_host();
    load(&mut host, &mut scene, id, &script, &wat::parse_str(text).unwrap()).unwrap();
    host.update(&mut scene, 0.1, 0.0, &Arc::new(Input::default()));
    let faults = host.take_faults();
    assert!(faults[0].error.to_string().contains("no object with handle 999"), "{}", faults[0].error);
}

/// Scripts built into the game (native mode), registered as the SDK's `script!` does natively.
mod natives {
    use bayleaf_script_sdk::{Property, Script, Transform, log, property};

    /// Turns around Y by `speed` degrees a second, and moves to x = the steps it counted itself.
    #[derive(Default)]
    pub struct Spin {
        steps: u32,
    }

    impl Script for Spin {
        const PROPERTIES: &'static [Property] = &[Property::Number("speed", 90.0)];

        fn start(&mut self) {
            log("native start");
        }

        fn update(&mut self, dt: f32) {
            self.steps += 1;
            let mut t = Transform::get();
            t.rotation[1] += property("speed").unwrap_or(0.0) * dt;
            t.position[0] = self.steps as f32;
            t.set();
        }
    }

    /// Sets a transform that is not a number.
    #[derive(Default)]
    pub struct Broken;

    impl Script for Broken {
        fn update(&mut self, _dt: f32) {
            let mut t = Transform::get();
            t.position[0] = f32::NAN;
            t.set();
        }
    }

    /// Panics on its first step.
    #[derive(Default)]
    pub struct Panicky;

    impl Script for Panicky {
        fn update(&mut self, _dt: f32) {
            let steps: Vec<u32> = Vec::new();
            let _ = steps[3];
        }
    }

    /// Declares Ring with a level, and says when it rings.
    #[derive(Default)]
    pub struct Alarm;

    impl Script for Alarm {
        const EVENTS: &'static [bayleaf_script_sdk::Event] =
            &[bayleaf_script_sdk::Event { name: "Ring", values: &[("level", bayleaf_script_sdk::ValueKind::Number)] }];

        fn message(&mut self, name: &str, _value: f32) {
            if name == "Ring" {
                log(&format!("ring {}", bayleaf_script_sdk::event::number(0)));
            }
        }
    }

    /// Subscribes to the Bell's Alarm, rings it, and says what it heard and from where.
    #[derive(Default)]
    pub struct Listener;

    impl Script for Listener {
        fn start(&mut self) {
            let alarm = bayleaf_script_sdk::Object::find("Bell").and_then(|bell| bell.component("Alarm"));
            if let Some(alarm) = alarm {
                alarm.subscribe("Ring");
                alarm.trigger_event("Ring", &[bayleaf_script_sdk::EventValue::Number(2.0)]);
            }
        }

        fn message(&mut self, name: &str, _value: f32) {
            if name == "Alarm/Ring" {
                let from = bayleaf_script_sdk::event::source().map(|c| c.object().name()).unwrap_or_default();
                log(&format!("heard {from} {}", bayleaf_script_sdk::event::number(0)));
            }
        }
    }

    /// Keeps `speed` in a field, counting how often it read it, and turns all its objects at once:
    /// around Y by `speed`, to x = how many times it read its properties.
    #[derive(Default)]
    pub struct Counted {
        speed: f32,
        reads: u32,
    }

    impl Script for Counted {
        const PROPERTIES: &'static [Property] = &[Property::Number("speed", 0.0)];

        fn read_properties(&mut self) {
            self.reads += 1;
            self.speed = property("speed").unwrap_or(0.0);
        }

        fn update_many(objects: &mut bayleaf_script_sdk::Many<'_, Self>, dt: f32) {
            for (counted, t) in objects.iter_mut() {
                t.rotation[1] += counted.speed * dt;
                t.position[0] = counted.reads as f32;
            }
        }
    }

    pub mod spin {
        bayleaf_script_sdk::script!(super::Spin);
    }
    pub mod counted {
        bayleaf_script_sdk::script!(super::Counted);
    }
    pub mod alarm {
        bayleaf_script_sdk::script!(super::Alarm);
    }
    pub mod listener {
        bayleaf_script_sdk::script!(super::Listener);
    }
    pub mod broken {
        bayleaf_script_sdk::script!(super::Broken);
    }
    pub mod panicky {
        bayleaf_script_sdk::script!(super::Panicky);
    }
}

/// Loads the module at `path` (bytes unused when a native script stands for it) on `object`.
fn load_native(host: &mut ScriptHost, scene: &mut Scene, object: StableId, path: &str) -> Result<(), ScriptError> {
    let script =
        ScriptRef { module: path.into(), properties: scene.get::<Scripts>(object).and_then(|s| s.0.first().map(|s| s.properties.clone())).unwrap_or_default() };
    host.load_bytes(object, 0, Path::new(path), None, b"", &script, scene, 0.0, &Arc::new(Input::default()))
}

#[test]
fn native_scripts_trigger_and_subscribe_to_a_components_event_as_visual_scripts_do() {
    let mut host = super::test_host();
    host.set_natives(vec![
        (std::path::PathBuf::from("scripts/alarm.wasm"), natives::alarm::bayleaf_native()),
        (std::path::PathBuf::from("scripts/listener.wasm"), natives::listener::bayleaf_native()),
    ]);
    let mut scene = Scene::new();
    let bell = scene.create_object(None, "Bell").unwrap();
    let ear = scene.create_object(None, "Ear").unwrap();
    let script = |module: &str| ScriptRef { module: module.into(), properties: Default::default() };
    scene.set(bell, Scripts(vec![script("game/scripts/alarm.wasm")])).unwrap();
    scene.set(ear, Scripts(vec![script("game/scripts/listener.wasm")])).unwrap();
    load_native(&mut host, &mut scene, bell, "game/scripts/alarm.wasm").unwrap();
    load_native(&mut host, &mut scene, ear, "game/scripts/listener.wasm").unwrap();
    host.update(&mut scene, 0.1, 0.0, &Arc::new(Input::default()));
    assert!(host.take_faults().is_empty());
    let logs: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(logs, ["ring 2", "heard Bell 2"], "the alarm rings, and the listener hears it with where it rang");
}

#[test]
fn native_scripts_run_through_the_host_each_object_with_its_own_value() {
    let mut host = super::test_host();
    host.set_natives(vec![(std::path::PathBuf::from("scripts/spin.wasm"), natives::spin::bayleaf_native())]);
    let (mut scene, fast, script) = setup(&[("speed", 45.0)]);
    scene.set(fast, Scripts(vec![script])).unwrap();
    let plain = scene.create_object(None, "Plain").unwrap();
    load_native(&mut host, &mut scene, fast, "game/scripts/spin.wasm").unwrap();
    load_native(&mut host, &mut scene, plain, "game/scripts/spin.wasm").unwrap();
    let logs: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(logs, ["native start", "native start"], "each object's script started");

    let input = Arc::new(Input::default());
    host.update(&mut scene, 1.0, 0.0, &input);
    host.update(&mut scene, 1.0, 0.0, &input);
    assert!(host.take_faults().is_empty());
    assert!((rotation_y(&scene, fast) - 90.0).abs() < 1e-3, "the scene's speed: {}", rotation_y(&scene, fast));
    assert!((rotation_y(&scene, plain) - 180.0).abs() < 1e-3, "the declared default: {}", rotation_y(&scene, plain));
    let x = |id| scene.get::<Transform>(id).unwrap().position[0];
    assert_eq!((x(fast), x(plain)), (2.0, 2.0), "each object counts its own steps");
}

#[test]
fn a_native_script_reads_its_properties_once_and_again_after_another_script_sets_one() {
    let input = Arc::new(Input::default());
    let mut host = super::test_host();
    host.set_natives(vec![(std::path::PathBuf::from("test.wasm"), natives::counted::bayleaf_native())]);
    let (mut scene, id, script) = setup(&[("speed", 45.0)]);
    scene.set(id, Scripts(vec![script])).unwrap();
    load_native(&mut host, &mut scene, id, "test.wasm").unwrap();
    // In its second update, sets `speed` of the script `test` on the object named Scripted.
    let writer = format!(
        r#"(module {IMPORTS}
            (import "bayleaf" "find" (func $find (param i32 i32) (result i32)))
            (import "bayleaf" "component_set" (func $component_set (param i32 i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 16) "Scriptedtestspeed")
            (global $steps (mut i32) (i32.const 0))
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_update") (param $dt f32)
              (global.set $steps (i32.add (global.get $steps) (i32.const 1)))
              (if (i32.eq (global.get $steps) (i32.const 2))
                (then
                  (f32.store (i32.const 64) (f32.const 5))
                  (drop (call $component_set (call $find (i32.const 16) (i32.const 8)) (i32.const 24) (i32.const 4) (i32.const 28) (i32.const 5) (i32.const 64)))))))"#
    );
    let setter = scene.create_object(None, "Setter").unwrap();
    let none = ScriptRef { module: "setter.wasm".into(), properties: BTreeMap::new() };
    host.load_bytes(setter, 0, Path::new("setter.wasm"), None, &wat::parse_str(writer).unwrap(), &none, &mut scene, 0.0, &input).unwrap();
    let mut step = |scene: &mut Scene| {
        host.update(scene, 1.0, 0.0, &input);
        let t = scene.get::<Transform>(id).unwrap();
        (t.position[0], t.rotation[1])
    };
    let (reads, _) = step(&mut scene);
    let (same, turned) = step(&mut scene);
    assert_eq!(reads, same, "not read again while nothing changed");
    assert!((turned - 90.0).abs() < 1e-3, "the scene's speed, from the field: {turned}");
    let (again, _) = step(&mut scene);
    assert!(again > same, "read again after another script set one");
    let (_, faster) = step(&mut scene);
    let (_, fastest) = step(&mut scene);
    assert!((fastest - faster - 5.0).abs() < 1e-3, "the new speed: {}", fastest - faster);
    assert!(host.take_faults().is_empty());
}

#[test]
fn a_native_script_that_breaks_the_rules_or_panics_stops_alone_with_a_reason() {
    let input = Arc::new(Input::default());
    for (native, expected) in [(natives::broken::bayleaf_native(), "finite"), (natives::panicky::bayleaf_native(), "panicked")] {
        let mut host = super::test_host();
        host.set_natives(vec![(std::path::PathBuf::from("bad.wasm"), native), (std::path::PathBuf::from("spin.wasm"), natives::spin::bayleaf_native())]);
        let (mut scene, bad, _) = setup(&[]);
        let good = scene.create_object(None, "Good").unwrap();
        load_native(&mut host, &mut scene, bad, "bad.wasm").unwrap();
        load_native(&mut host, &mut scene, good, "spin.wasm").unwrap();
        host.update(&mut scene, 1.0, 0.0, &input);
        let faults = host.take_faults();
        assert_eq!(faults.len(), 1, "{expected}: {faults:?}");
        assert!(faults[0].error.to_string().contains(expected), "{} does not mention {expected}", faults[0].error);
        assert!(rotation_y(&scene, good) > 0.0, "the other script keeps running");
        host.update(&mut scene, 1.0, 0.0, &input);
        assert!(host.take_faults().is_empty(), "a stopped script does not run again");
    }
}

#[test]
fn a_layout_the_sdk_writes_is_one_the_host_reads() {
    use bayleaf_scene::inspector::InspectorItem;
    use bayleaf_script_sdk::{Is, Layout, Property};
    const LAYOUT: &[Layout] = &[
        Layout::foldout(
            "Movement",
            &[Layout::property("speed").label("Top speed").slider(0.0, 20.0).hint("Units a second"), Layout::property("boost").when("mode", Is::Whole(1))],
        ),
        Layout::property("mode").choices(&["Walk", "Run"]),
        Layout::button("Jump now", "jump"),
        Layout::warning("Careful \"here\""),
    ];
    let text = bayleaf_script_sdk::describe(&[Property::Number("speed", 3.0), Property::Number("boost", 1.0), Property::Whole("mode", 0)], "", LAYOUT, &[]);
    let bytes = text.as_bytes();
    let wat = format!(
        r#"(module (memory (export "memory") 1) (data (i32.const 0) "{}")
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_describe") (result i64) i64.const {}))"#,
        bytes.iter().map(|b| format!("\\{b:02x}")).collect::<String>(),
        bytes.len()
    );
    let described = super::test_host().describe_all("layout.wasm", &wat::parse_str(wat).unwrap()).unwrap();
    assert_eq!(described.inspector.len(), 4, "{:?}", described.inspector);
    let InspectorItem::Group { closed: true, items, .. } = &described.inspector[0] else { panic!("a foldout first: {:?}", described.inspector) };
    let InspectorItem::Property(speed) = &items[0] else { panic!("{items:?}") };
    assert_eq!((speed.label.as_deref(), speed.range, speed.slider), (Some("Top speed"), Some((0.0, 20.0)), true));
    assert!(matches!(&described.inspector[2], InspectorItem::Button { message, .. } if message == "jump"));
    assert!(matches!(&described.inspector[3], InspectorItem::Note { warning: true, text, .. } if text == "Careful \"here\""));
}

#[test]
fn a_layout_that_does_not_fit_the_properties_stops_the_module_loading() {
    let text = r#"{"properties": [["speed", 1.0]], "inspector": [{"property": "speed", "choices": ["a"]}]}"#;
    let wat = format!(
        r#"(module (memory (export "memory") 1) (data (i32.const 0) "{}")
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_describe") (result i64) i64.const {}))"#,
        text.replace('"', "\\\""),
        text.len()
    );
    let error = super::test_host().describe_all("bad.wasm", &wat::parse_str(wat).unwrap()).unwrap_err();
    assert!(error.to_string().contains("inspector") && error.to_string().contains("choices"), "{error}");
}

#[test]
fn a_message_sent_to_one_object_reaches_only_its_scripts() {
    let name = r#"(import "bayleaf" "message_name" (func $name (param i32 i32) (result i32)))"#;
    let listens = r#"(func (export "bayleaf_message") (param f32) (call $log (i32.const 400) (call $name (i32.const 400) (i32.const 32))))"#;
    let text = format!(r#"(module {IMPORTS} {name} (memory (export "memory") 1) (func (export "bayleaf_abi") (result i32) i32.const 1) {listens})"#);
    let bytes = wat::parse_str(text).unwrap();
    let (mut scene, first, script) = setup(&[]);
    let second = scene.create_object(None, "Second").unwrap();
    let mut host = super::test_host();
    load(&mut host, &mut scene, first, &script, &bytes).unwrap();
    load(&mut host, &mut scene, second, &script, &bytes).unwrap();
    host.send(&mut scene, second, "jump", 0.0, 0.0, &Arc::new(Input::default()));
    let logs = host.take_logs();
    assert_eq!(logs.iter().map(|l| (l.object, l.text.as_str())).collect::<Vec<_>>(), [(second, "jump")]);
}

#[test]
fn a_list_used_as_a_map_or_grown_past_its_limit_stops_the_script_with_a_reason() {
    let input = Arc::new(Input::default());
    // A number at 0 (a 16-byte value as event_send lists them); "speed" names the list.
    let imports = r#"
        (import "bayleaf" "list_put" (func $list_put (param i32 i32 i32 i32 i32) (result i32)))
        (import "bayleaf" "map_count" (func $map_count (param i32 i32) (result i32)))
        (import "bayleaf" "list_len" (func $list_len (param i32 i32) (result i32)))"#;
    let put = "(drop (call $list_put (i32.const 100) (i32.const 5) (i32.const 0) (i32.const 0) (i32.const 0)))";
    let as_map = format!(r#"(func (export "bayleaf_update") (param f32) {put} (drop (call $map_count (i32.const 100) (i32.const 5))))"#);
    let grow = format!(
        r#"(func (export "bayleaf_update") (param f32) (loop $more {put} (br_if $more (i32.le_u (call $list_len (i32.const 100) (i32.const 5)) (i32.const {})))))"#,
        crate::MAX_COLLECTION_ITEMS
    );
    for (body, expected) in [(as_map, "is a list"), (grow, "at most")] {
        let (mut scene, id, script) = setup(&[]);
        let mut host = super::test_host();
        let text = format!(
            r#"(module {IMPORTS} {imports}
                (memory (export "memory") 1)
                (data (i32.const 100) "speed")
                (func (export "bayleaf_abi") (result i32) i32.const 1)
                {body})"#
        );
        load(&mut host, &mut scene, id, &script, &wat::parse_str(text).unwrap()).unwrap();
        host.update(&mut scene, 0.1, 0.0, &input);
        let faults = host.take_faults();
        assert_eq!(faults.len(), 1, "{body}");
        assert!(faults[0].error.to_string().contains(expected), "{} does not mention {expected}", faults[0].error);
    }
}

#[test]
fn a_script_finds_an_object_by_the_name_it_has_now_after_the_scene_changes_between_steps() {
    let input = Arc::new(Input::default());
    let (mut scene, id, script) = setup(&[]);
    // Logs "found" while an object named Door is in the scene.
    let text = format!(
        r#"(module {IMPORTS}
            (import "bayleaf" "find" (func $find (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 100) "Door")
            (data (i32.const 120) "found")
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_update") (param f32)
              (if (call $find (i32.const 100) (i32.const 4)) (then (call $log (i32.const 120) (i32.const 5))))))"#
    );
    let mut host = super::test_host();
    load(&mut host, &mut scene, id, &script, &wat::parse_str(text).unwrap()).unwrap();
    host.update(&mut scene, 0.1, 0.0, &input);
    assert!(host.take_logs().is_empty(), "no Door yet");
    let door = scene.create_object(None, "Gate").unwrap();
    host.update(&mut scene, 0.1, 0.1, &input);
    assert!(host.take_logs().is_empty(), "a Gate is no Door");
    scene.set(door, bayleaf_scene::components::Name("Door".to_owned())).unwrap();
    host.update(&mut scene, 0.1, 0.2, &input);
    assert_eq!(host.take_logs().len(), 1, "renamed, it is found the next step");
    scene.delete_object(door).unwrap();
    host.update(&mut scene, 0.1, 0.3, &input);
    assert!(host.take_logs().is_empty(), "and not once it is gone");
    assert!(host.take_faults().is_empty());
}

/// A module that runs many objects: each step one `bayleaf_update_all` turns every object it runs
/// by one degree and logs "all" once, for the object selected first.
fn batched_turner(trap_on_second_step: bool) -> Vec<u8> {
    let trap = if trap_on_second_step { "(if (i32.eq (global.get $steps) (i32.const 2)) (then unreachable))" } else { "" };
    let text = format!(
        r#"(module {IMPORTS}
            (import "bayleaf" "slot_select" (func $select (param i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 120) "all")
            (global $n (mut i32) (i32.const 0))
            (global $steps (mut i32) (i32.const 0))
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_attach") (param $slot i32)
              (if (i32.ge_u (local.get $slot) (global.get $n)) (then (global.set $n (i32.add (local.get $slot) (i32.const 1))))))
            (func (export "bayleaf_detach") (param $slot i32))
            (func (export "bayleaf_update_all") (param $dt f32) (local $k i32) (local $logged i32)
              (global.set $steps (i32.add (global.get $steps) (i32.const 1)))
              (block $done (loop $next
                (br_if $done (i32.ge_u (local.get $k) (global.get $n)))
                (if (call $select (local.get $k))
                  (then
                    {trap}
                    (if (i32.eqz (local.get $logged)) (then (call $log (i32.const 120) (i32.const 3)) (local.set $logged (i32.const 1))))
                    (call $get (i32.const 0))
                    (f32.store (i32.const 16) (f32.add (f32.load (i32.const 16)) (f32.const 1)))
                    (call $set (i32.const 0))))
                (local.set $k (i32.add (local.get $k) (i32.const 1)))
                (br $next)))))"#
    );
    wat::parse_str(text).unwrap()
}

#[test]
fn a_module_that_runs_many_objects_runs_them_all_in_one_call_per_step() {
    let input = Arc::new(Input::default());
    let (mut scene, first, script) = setup(&[]);
    let others: Vec<StableId> = (0..2).map(|i| scene.create_object(None, &format!("Other {i}")).unwrap()).collect();
    let mut host = super::test_host();
    let bytes = batched_turner(false);
    for id in std::iter::once(first).chain(others.iter().copied()) {
        load(&mut host, &mut scene, id, &script, &bytes).unwrap();
    }
    host.update(&mut scene, 0.1, 0.0, &input);
    assert_eq!(host.take_logs().len(), 1, "one call for all three objects");
    for id in std::iter::once(first).chain(others.iter().copied()) {
        assert!((rotation_y(&scene, id) - 1.0).abs() < 1e-6, "each object turned once");
    }
    // An object that goes stops; the others go on in the same call.
    scene.delete_object(others[0]).unwrap();
    host.update(&mut scene, 0.1, 0.1, &input);
    assert_eq!(host.take_logs().len(), 1);
    assert!((rotation_y(&scene, first) - 2.0).abs() < 1e-6 && (rotation_y(&scene, others[1]) - 2.0).abs() < 1e-6);
    assert_eq!(host.running(), 2);
    assert!(host.take_faults().is_empty());
}

#[test]
fn a_scene_loads_each_file_s_objects_into_that_file_s_module_even_when_two_files_are_alike() {
    // Two files with the same bytes, two objects on each.
    let dir = std::env::temp_dir().join(format!("bayleaf-script-alike-{}-{}", std::process::id(), module_path!().replace("::", "-")));
    std::fs::create_dir_all(&dir).unwrap();
    let bytes = batched_turner(false);
    let mut scene = Scene::new();
    let mut objects = Vec::new();
    for file in ["one.wasm", "two.wasm"] {
        std::fs::write(dir.join(file), &bytes).unwrap();
        for n in 0..2 {
            let id = scene.create_object(None, &format!("{file} {n}")).unwrap();
            scene.set(id, Scripts(vec![ScriptRef { module: file.into(), properties: BTreeMap::new() }])).unwrap();
            objects.push(id);
        }
    }
    let mut host = super::test_host();
    let input = Arc::new(Input::default());
    host.load_scene(&mut scene, &dir, 0.0, &input);
    assert!(host.take_faults().is_empty());
    assert_eq!(host.len(), 4);
    host.update(&mut scene, 0.1, 0.0, &input);
    assert_eq!(host.take_logs().len(), 2, "one call per file, not one for both and not one per object");
    for id in &objects {
        assert!((rotation_y(&scene, *id) - 1.0).abs() < 1e-6, "each object turned once");
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_error_in_a_module_that_runs_many_objects_stops_them_all_with_one_fault() {
    let input = Arc::new(Input::default());
    let (mut scene, first, script) = setup(&[]);
    let other = scene.create_object(None, "Other").unwrap();
    let mut host = super::test_host();
    let bytes = batched_turner(true);
    load(&mut host, &mut scene, first, &script, &bytes).unwrap();
    load(&mut host, &mut scene, other, &script, &bytes).unwrap();
    host.update(&mut scene, 0.1, 0.0, &input);
    host.update(&mut scene, 0.1, 0.1, &input);
    let faults = host.take_faults();
    assert_eq!(faults.len(), 1, "one fault for the module");
    assert_eq!(faults[0].object, first, "on the object whose update it was in");
    assert!(faults[0].error.to_string().contains("1 other"), "{}", faults[0].error);
    assert_eq!(host.running(), 0, "and every object it runs stops");
    host.update(&mut scene, 0.1, 0.2, &input);
    assert!((rotation_y(&scene, other) - 1.0).abs() < 1e-6, "a stopped module does not run again");
}

#[test]
fn a_module_that_runs_many_objects_moves_them_all_through_one_frame() {
    let input = Arc::new(Input::default());
    let (mut scene, first, script) = setup(&[]);
    let other = scene.create_object(None, "Other").unwrap();
    // Reads every object's entry, turns each running one by 2 degrees, marks it changed, hands
    // the frame back: no call per object.
    let text = format!(
        r#"(module {IMPORTS}
            (import "bayleaf" "frame_get" (func $frame_get (param i32 i32) (result i32)))
            (import "bayleaf" "frame_set" (func $frame_set (param i32 i32)))
            (memory (export "memory") 1)
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_attach") (param i32))
            (func (export "bayleaf_detach") (param i32))
            (func (export "bayleaf_update_all") (param $dt f32) (local $n i32) (local $k i32) (local $at i32)
              (local.set $n (call $frame_get (i32.const 4096) (i32.const 16)))
              (block $done (loop $next
                (br_if $done (i32.ge_u (local.get $k) (local.get $n)))
                (local.set $at (i32.add (i32.const 4096) (i32.mul (local.get $k) (i32.const 64))))
                (if (i32.and (i32.load offset=36 (local.get $at)) (i32.const 1))
                  (then
                    (f32.store offset=16 (local.get $at) (f32.add (f32.load offset=16 (local.get $at)) (f32.const 2)))
                    (i32.store offset=36 (local.get $at) (i32.or (i32.load offset=36 (local.get $at)) (i32.const 4)))))
                (local.set $k (i32.add (local.get $k) (i32.const 1)))
                (br $next)))
              (call $frame_set (i32.const 4096) (local.get $n))))"#
    );
    let bytes = wat::parse_str(text).unwrap();
    let mut host = super::test_host();
    load(&mut host, &mut scene, first, &script, &bytes).unwrap();
    load(&mut host, &mut scene, other, &script, &bytes).unwrap();
    host.update(&mut scene, 0.1, 0.0, &input);
    host.update(&mut scene, 0.1, 0.1, &input);
    assert!(host.take_faults().is_empty());
    for id in [first, other] {
        assert!((rotation_y(&scene, id) - 4.0).abs() < 1e-6, "each object turned through the frame, twice: {}", rotation_y(&scene, id));
    }
}

#[test]
fn a_frame_given_back_with_a_number_that_is_not_finite_stops_the_module_and_moves_nothing() {
    let input = Arc::new(Input::default());
    let (mut scene, first, script) = setup(&[]);
    let other = scene.create_object(None, "Other").unwrap();
    // Turns the first object by 2 degrees and sets the second's to not-a-number, both marked
    // changed, and hands the frame back.
    let text = format!(
        r#"(module {IMPORTS}
            (import "bayleaf" "frame_get" (func $frame_get (param i32 i32) (result i32)))
            (import "bayleaf" "frame_set" (func $frame_set (param i32 i32)))
            (memory (export "memory") 1)
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_attach") (param i32))
            (func (export "bayleaf_detach") (param i32))
            (func (export "bayleaf_update_all") (param $dt f32) (local $n i32)
              (local.set $n (call $frame_get (i32.const 4096) (i32.const 16)))
              (f32.store offset=16 (i32.const 4096) (f32.const 2))
              (i32.store offset=36 (i32.const 4096) (i32.or (i32.load offset=36 (i32.const 4096)) (i32.const 4)))
              (f32.store offset=16 (i32.const 4160) (f32.div (f32.const 0) (f32.const 0)))
              (i32.store offset=36 (i32.const 4160) (i32.or (i32.load offset=36 (i32.const 4160)) (i32.const 4)))
              (call $frame_set (i32.const 4096) (local.get $n))))"#
    );
    let bytes = wat::parse_str(text).unwrap();
    let mut host = super::test_host();
    load(&mut host, &mut scene, first, &script, &bytes).unwrap();
    load(&mut host, &mut scene, other, &script, &bytes).unwrap();
    host.update(&mut scene, 0.1, 0.0, &input);
    let faults = host.take_faults();
    assert_eq!(faults.len(), 1, "{faults:?}");
    assert!(faults[0].error.to_string().contains("finite"), "{}", faults[0].error);
    assert_eq!(host.running(), 0, "the module stops, with every object it runs");
    for id in [first, other] {
        assert_eq!(rotation_y(&scene, id), 0.0, "nothing of that frame is applied");
    }
}

#[test]
fn a_frame_names_each_object_by_its_handle_and_the_stamp_changes_with_the_structure() {
    let input = Arc::new(Input::default());
    let (mut scene, first, script) = setup(&[]);
    let other = scene.create_object(None, "Other").unwrap();
    // Logs the name of each entry's handle (the word of flags without its low four bits), then
    // "same" or "changed" for the world's stamp against the step before.
    let text = format!(
        r#"(module {IMPORTS}
            (import "bayleaf" "frame_get" (func $frame_get (param i32 i32) (result i32)))
            (import "bayleaf" "world_stamp" (func $stamp (result i64)))
            (import "bayleaf" "name_of" (func $name_of (param i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 16) "samechanged")
            (global $before (mut i64) (i64.const 0))
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_attach") (param i32))
            (func (export "bayleaf_detach") (param i32))
            (func (export "bayleaf_update_all") (param $dt f32) (local $n i32) (local $k i32)
              (local.set $n (call $frame_get (i32.const 4096) (i32.const 16)))
              (block $done (loop $next
                (br_if $done (i32.ge_u (local.get $k) (local.get $n)))
                (call $log (i32.const 256)
                  (call $name_of
                    (i32.shr_u (i32.load offset=36 (i32.add (i32.const 4096) (i32.mul (local.get $k) (i32.const 64)))) (i32.const 4))
                    (i32.const 256) (i32.const 64)))
                (local.set $k (i32.add (local.get $k) (i32.const 1)))
                (br $next)))
              (if (i64.eq (call $stamp) (global.get $before))
                (then (call $log (i32.const 16) (i32.const 4)))
                (else (call $log (i32.const 20) (i32.const 7))))
              (global.set $before (call $stamp))))"#
    );
    let bytes = wat::parse_str(text).unwrap();
    let mut host = super::test_host();
    load(&mut host, &mut scene, first, &script, &bytes).unwrap();
    load(&mut host, &mut scene, other, &script, &bytes).unwrap();
    let mut step = |scene: &mut Scene| -> Vec<String> {
        host.update(scene, 0.1, 0.0, &input);
        host.take_logs().into_iter().map(|l| l.text).collect()
    };
    let _ = step(&mut scene);
    assert_eq!(step(&mut scene).last().map(String::as_str), Some("same"), "nothing came or went");
    // A new object before the others: every handle moves, and the stamp says so.
    let new = scene.create_object(None, "New").unwrap();
    scene.reparent(new, None, Some(0)).unwrap();
    let logs = step(&mut scene);
    assert_eq!(logs.last().map(String::as_str), Some("changed"));
    assert!(logs.contains(&"Scripted".to_owned()) && logs.contains(&"Other".to_owned()), "each entry's handle still names its own object: {logs:?}");
}

#[test]
fn a_frame_tells_an_object_once_that_another_script_set_its_property() {
    let input = Arc::new(Input::default());
    let (mut scene, first, script) = setup(&[("speed", 1.0)]);
    let other = scene.create_object(None, "Other").unwrap();
    let setter = scene.create_object(None, "Setter").unwrap();
    // Reads the frame only (never selects an object) and logs "told" for each entry whose flags say
    // its properties changed.
    let reader = format!(
        r#"(module {IMPORTS}
            (import "bayleaf" "frame_get" (func $frame_get (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 16) "told")
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_attach") (param i32))
            (func (export "bayleaf_detach") (param i32))
            (func (export "bayleaf_update_all") (param $dt f32) (local $n i32) (local $k i32)
              (local.set $n (call $frame_get (i32.const 4096) (i32.const 16)))
              (block $done (loop $next
                (br_if $done (i32.ge_u (local.get $k) (local.get $n)))
                (if (i32.and (i32.load offset=36 (i32.add (i32.const 4096) (i32.mul (local.get $k) (i32.const 64)))) (i32.const 2))
                  (then (call $log (i32.const 16) (i32.const 4))))
                (local.set $k (i32.add (local.get $k) (i32.const 1)))
                (br $next)))))"#
    );
    // In its first update, sets `speed` of the script `test` on the object named Other.
    let writer = format!(
        r#"(module {IMPORTS}
            (import "bayleaf" "find" (func $find (param i32 i32) (result i32)))
            (import "bayleaf" "component_set" (func $component_set (param i32 i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 16) "Othertestspeed")
            (global $done (mut i32) (i32.const 0))
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_update") (param $dt f32)
              (if (i32.eqz (global.get $done))
                (then
                  (global.set $done (i32.const 1))
                  (f32.store (i32.const 64) (f32.const 5))
                  (drop (call $component_set (call $find (i32.const 16) (i32.const 5)) (i32.const 21) (i32.const 4) (i32.const 25) (i32.const 5) (i32.const 64)))))))"#
    );
    let mut host = super::test_host();
    let reader = wat::parse_str(reader).unwrap();
    load(&mut host, &mut scene, first, &script, &reader).unwrap();
    load(&mut host, &mut scene, other, &script, &reader).unwrap();
    let none = ScriptRef { module: "setter.wasm".into(), properties: BTreeMap::new() };
    host.load_bytes(setter, 0, Path::new("setter.wasm"), None, &wat::parse_str(writer).unwrap(), &none, &mut scene, 0.0, &input).unwrap();
    let mut step = |scene: &mut Scene| -> Vec<String> {
        host.update(scene, 0.1, 0.0, &input);
        host.take_logs().into_iter().map(|l| l.text).collect()
    };
    assert!(step(&mut scene).is_empty(), "nothing set before the step");
    assert_eq!(step(&mut scene), ["told"], "the one object whose property was set, in the next frame");
    assert!(step(&mut scene).is_empty(), "told once");
    assert!(host.take_faults().is_empty());
}

#[test]
fn an_error_in_an_update_of_all_is_reported_on_the_object_the_frame_marks() {
    let input = Arc::new(Input::default());
    let (mut scene, first, script) = setup(&[]);
    let other = scene.create_object(None, "Other").unwrap();
    // Marks the second object as being updated, then stops, without selecting it at the host.
    let text = format!(
        r#"(module {IMPORTS}
            (import "bayleaf" "frame_get" (func $frame_get (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (func (export "bayleaf_abi") (result i32) i32.const 1)
            (func (export "bayleaf_attach") (param i32))
            (func (export "bayleaf_detach") (param i32))
            (func (export "bayleaf_update_all") (param $dt f32)
              (drop (call $frame_get (i32.const 4096) (i32.const 16)))
              (i32.store offset=36 (i32.const 4160) (i32.or (i32.load offset=36 (i32.const 4160)) (i32.const 8)))
              unreachable))"#
    );
    let bytes = wat::parse_str(text).unwrap();
    let mut host = super::test_host();
    load(&mut host, &mut scene, first, &script, &bytes).unwrap();
    load(&mut host, &mut scene, other, &script, &bytes).unwrap();
    host.update(&mut scene, 0.1, 0.0, &input);
    let faults = host.take_faults();
    assert_eq!(faults.len(), 1);
    assert_eq!(faults[0].object, other, "the object being updated, not the one selected last");
}

/// The data files a test plays with: `data/goblin.bdata` with a number, a list, a map and a text.
fn goblin_data() -> std::collections::HashMap<String, crate::DataValues> {
    use bayleaf_scene::components::PropertyShape;
    let mut values = crate::DataValues::new();
    values.insert("hp".to_owned(), (PropertyType::Number, PropertyShape::Single, PropertyValue::Number(40.0)));
    values.insert(
        "waves".to_owned(),
        (PropertyType::WholeNumber, PropertyShape::List, PropertyValue::List { list: vec![PropertyValue::Number(3.0), PropertyValue::Number(7.0)] }),
    );
    let drops = BTreeMap::from([("gold".to_owned(), PropertyValue::Number(5.0))]);
    values.insert("drops".to_owned(), (PropertyType::Number, PropertyShape::Map(PropertyType::Text), PropertyValue::Map { map: drops }));
    values.insert("name".to_owned(), (PropertyType::Text, PropertyShape::Single, PropertyValue::Text("Gob".to_owned())));
    std::collections::HashMap::from([("data/goblin.bdata".to_owned(), values)])
}

#[test]
fn scripts_read_data_files_fields_lists_and_maps_and_nothing_from_files_that_are_not_there() {
    let text = r#"(module
        (import "bayleaf" "transform_get" (func $get (param i32)))
        (import "bayleaf" "transform_set" (func $set (param i32)))
        (import "bayleaf" "data_get" (func $dget (param i32 i32 i32 i32 i32) (result i32)))
        (import "bayleaf" "data_text" (func $dtext (param i32 i32 i32 i32 i32 i32) (result i32)))
        (import "bayleaf" "data_len" (func $dlen (param i32 i32 i32 i32) (result i32)))
        (import "bayleaf" "data_item" (func $ditem (param i32 i32 i32 i32 i32 i32) (result i32)))
        (import "bayleaf" "data_entry" (func $dentry (param i32 i32 i32 i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (data (i32.const 200) "data/goblin.bdata")
        (data (i32.const 220) "data\5cother.bdata")
        (data (i32.const 240) "hp")
        (data (i32.const 250) "waves")
        (data (i32.const 260) "drops")
        (data (i32.const 270) "gold")
        (data (i32.const 280) "name")
        (func (export "bayleaf_abi") (result i32) i32.const 1)
        (func (export "bayleaf_update") (param $dt f32)
          (call $get (i32.const 0))
          (drop (call $dget (i32.const 200) (i32.const 17) (i32.const 240) (i32.const 2) (i32.const 64)))
          (f32.store (i32.const 0) (f32.load (i32.const 64)))
          (f32.store (i32.const 4) (f32.convert_i32_s (call $dlen (i32.const 200) (i32.const 17) (i32.const 250) (i32.const 5))))
          (drop (call $ditem (i32.const 200) (i32.const 17) (i32.const 250) (i32.const 5) (i32.const 1) (i32.const 64)))
          (f32.store (i32.const 8) (f32.load (i32.const 64)))
          (drop (call $dentry (i32.const 200) (i32.const 17) (i32.const 260) (i32.const 5) (i32.const 270) (i32.const 4) (i32.const 64)))
          (f32.store (i32.const 12) (f32.load (i32.const 64)))
          (f32.store (i32.const 16) (f32.convert_i32_s (call $dtext (i32.const 200) (i32.const 17) (i32.const 280) (i32.const 4) (i32.const 0) (i32.const 0))))
          ;; A file the project lacks: nothing found, and the length reads -1.
          (f32.store (i32.const 20) (f32.convert_i32_s
            (i32.add (call $dget (i32.const 220) (i32.const 16) (i32.const 240) (i32.const 2) (i32.const 64))
                     (call $dlen (i32.const 220) (i32.const 16) (i32.const 250) (i32.const 5)))))
          (call $set (i32.const 0))))"#;
    let bytes = wat::parse_str(text).unwrap();
    let (mut scene, id, script) = setup(&[]);
    let mut host = super::test_host();
    host.set_data(goblin_data());
    load(&mut host, &mut scene, id, &script, &bytes).unwrap();
    host.update(&mut scene, 0.1, 1.0, &Arc::new(Input::default()));
    assert!(host.take_faults().is_empty());
    let t = scene.get::<Transform>(id).unwrap();
    assert_eq!(t.position, [40.0, 2.0, 7.0], "a number, a list's length and its second item");
    assert_eq!(&t.rotation, &[5.0, 3.0, -1.0], "a map's value by key, a text's length, and a file that is not there");
}
