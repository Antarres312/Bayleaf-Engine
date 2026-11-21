use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use bayleaf_core::StableId;
use bayleaf_graph::{Graph, Link, PinRef};
use bayleaf_scene::Scene;
use bayleaf_scene::components::{ScriptRef, Transform};
use serde_json::json;

use super::{CATALOG, compile};
use crate::{Input, ScriptHost};

struct Builder(Graph);

impl Builder {
    fn new() -> Self {
        Self(Graph::empty(&CATALOG))
    }

    fn node(&mut self, kind: &str, values: serde_json::Value) -> u32 {
        let id = self.0.add_node(&CATALOG, kind, [0.0, 0.0]).unwrap();
        for (name, value) in values.as_object().cloned().unwrap_or_default() {
            self.0.set_value(&CATALOG, id, &name, value).unwrap();
        }
        id
    }

    fn link(&mut self, from: u32, from_pin: &str, to: u32, to_pin: &str) {
        self.0.connect(&CATALOG, Link { from: PinRef::new(from, from_pin), to: PinRef::new(to, to_pin) }).unwrap();
    }

    fn bytes(&self) -> Vec<u8> {
        compile(&self.0).unwrap_or_else(|p| panic!("{p:?}"))
    }
}

/// A scene with one object and the host running `bytes` as its script.
fn run(bytes: &[u8], properties: &[(&str, f32)]) -> (Scene, StableId, ScriptHost) {
    let mut scene = Scene::new();
    let id = scene.create_object(None, "Scripted").unwrap();
    let script = ScriptRef { module: "test.bscript".into(), properties: properties.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect::<BTreeMap<_, _>>() };
    let mut host = ScriptHost::new();
    host.load_bytes(id, 0, Path::new("test.bscript"), None, bytes, &script, &mut scene, 0.0, &Arc::new(Input::default())).unwrap();
    (scene, id, host)
}

fn transform(scene: &Scene, id: StableId) -> Transform {
    scene.get::<Transform>(id).cloned().unwrap()
}

fn spinner() -> Builder {
    let mut b = Builder::new();
    let update = b.node("update", json!({}));
    let speed = b.node("property", json!({"name": "speed", "default": 90.0}));
    let per_step = b.node("multiply", json!({}));
    b.link(speed, "value", per_step, "a");
    b.link(update, "dt", per_step, "b");
    let degrees = b.node("vector", json!({}));
    b.link(per_step, "result", degrees, "y");
    let rotate = b.node("rotateBy", json!({}));
    b.link(update, "then", rotate, "run");
    b.link(degrees, "vector", rotate, "degrees");
    b
}

#[test]
fn a_visual_script_rotates_by_its_declared_property() {
    let bytes = spinner().bytes();
    assert_eq!(ScriptHost::new().describe("spin.bscript", &bytes).unwrap(), vec![("speed".to_owned(), 90.0)]);
    let (mut scene, id, mut host) = run(&bytes, &[]);
    for _ in 0..10 {
        host.update(&mut scene, 0.1, 0.0, &Arc::new(Input::default()));
    }
    assert!((transform(&scene, id).rotation[1] - 90.0).abs() < 1e-3, "one second at the default 90°/s");
    // A value set on the object wins over the declared default.
    let (mut scene, id, mut host) = run(&bytes, &[("speed", 10.0)]);
    host.update(&mut scene, 1.0, 0.0, &Arc::new(Input::default()));
    assert!((transform(&scene, id).rotation[1] - 10.0).abs() < 1e-3);
}

#[test]
fn log_writes_the_message_and_the_number() {
    let mut b = Builder::new();
    let start = b.node("start", json!({}));
    let mut previous = (start, "then");
    for value in [2.5, -0.125, 3.0, 1234.5678] {
        let number = b.node("number", json!({ "value": value }));
        let log = b.node("log", json!({ "message": "x =" }));
        b.link(number, "value", log, "value");
        b.link(previous.0, previous.1, log, "run");
        previous = (log, "then");
    }
    let plain = b.node("log", json!({ "message": "done" }));
    b.link(previous.0, previous.1, plain, "run");
    let (_, _, mut host) = run(&b.bytes(), &[]);
    let lines: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(lines, ["x = 2.5", "x = -0.125", "x = 3", "x = 1234.568", "done"]);
}

#[test]
fn branch_follows_the_key_and_sine_is_accurate() {
    let mut b = Builder::new();
    let update = b.node("update", json!({}));
    let key = b.node("keyDown", json!({ "key": "W" }));
    let branch = b.node("branch", json!({}));
    b.link(update, "then", branch, "run");
    b.link(key, "down", branch, "condition");
    let forward = b.node("moveBy", json!({ "offset": [0.0, 0.0, -1.0] }));
    b.link(branch, "true", forward, "run");
    let place = b.node("setPosition", json!({}));
    b.link(branch, "false", place, "run");
    let sine = b.node("sine", json!({ "value": [1.0, 4.0, -7.0] }));
    b.link(sine, "result", place, "position");
    let (mut scene, id, mut host) = run(&b.bytes(), &[]);

    host.update(&mut scene, 0.1, 0.0, &Arc::new(Input::default()));
    let p = transform(&scene, id).position;
    for (got, x) in p.iter().zip([1.0f32, 4.0, -7.0]) {
        assert!((got - f64::from(x.sin())).abs() < 1e-4, "sin({x}) = {got}");
    }
    let held = Arc::new(Input { keys: ["W".to_owned()].into_iter().collect(), ..Input::default() });
    host.update(&mut scene, 0.1, 0.0, &held);
    let after = transform(&scene, id).position;
    assert!((after[2] - (p[2] - 1.0)).abs() < 1e-5, "W moves by the offset");
}

#[test]
fn every_node_kind_compiles_and_runs() {
    let mut b = Builder::new();
    let start = b.node("start", json!({}));
    let update = b.node("update", json!({}));
    let time = b.node("time", json!({}));
    let position = b.node("position", json!({}));
    let rotation = b.node("rotation", json!({}));
    let scale = b.node("scale", json!({}));
    let split = b.node("split", json!({}));
    b.link(scale, "scale", split, "vector");
    let add = b.node("add", json!({}));
    b.link(position, "position", add, "a");
    b.link(rotation, "rotation", add, "b");
    let sub = b.node("subtract", json!({}));
    b.link(add, "result", sub, "a");
    b.link(time, "seconds", sub, "b");
    let div = b.node("divide", json!({ "b": 1000.0 }));
    b.link(sub, "result", div, "a");
    let greater = b.node("greater", json!({}));
    b.link(split, "x", greater, "a");
    let key = b.node("keyDown", json!({}));
    let not = b.node("not", json!({}));
    b.link(key, "down", not, "value");
    let and = b.node("and", json!({}));
    b.link(greater, "result", and, "a");
    b.link(not, "result", and, "b");
    let or = b.node("or", json!({}));
    b.link(and, "result", or, "a");
    b.link(key, "down", or, "b");
    let choose = b.node("choose", json!({}));
    b.link(or, "result", choose, "condition");
    let first = b.node("split", json!({}));
    b.link(div, "result", first, "vector");
    b.link(first, "x", choose, "a");
    let make = b.node("vector", json!({ "y": 0.5 }));
    b.link(choose, "result", make, "x");
    let number = b.node("number", json!({ "value": 0.0 }));
    let prop = b.node("property", json!({ "name": "unused", "default": 2.0 }));
    let mul = b.node("multiply", json!({}));
    b.link(number, "value", mul, "a");
    b.link(prop, "value", mul, "b");
    let sine = b.node("sine", json!({}));
    b.link(mul, "result", sine, "value");

    let log = b.node("log", json!({}));
    b.link(start, "then", log, "run");
    b.link(sine, "result", log, "value");
    let mut previous = (log, "then");
    for (kind, pin) in [("setPosition", "position"), ("moveBy", "offset"), ("setRotation", "rotation"), ("rotateBy", "degrees"), ("setScale", "scale")] {
        let action = b.node(kind, json!({}));
        b.link(previous.0, previous.1, action, "run");
        if kind != "setScale" {
            b.link(make, "vector", action, pin);
        }
        previous = (action, "then");
    }
    let branch = b.node("branch", json!({}));
    b.link(update, "then", branch, "run");
    b.link(or, "result", branch, "condition");
    // Mouse, other objects, spawning and destroying, behind a branch that stays false here.
    let pointer = b.node("pointer", json!({}));
    let mouse = b.node("mouseDown", json!({ "button": "Right" }));
    let clicked = b.node("and", json!({}));
    b.link(pointer, "inside", clicked, "a");
    b.link(mouse, "down", clicked, "b");
    let gate = b.node("branch", json!({}));
    b.link(branch, "true", gate, "run");
    b.link(clicked, "result", gate, "condition");
    let position_of = b.node("positionOf", json!({ "object": "Scripted" }));
    let set_of = b.node("setPositionOf", json!({ "object": "Scripted" }));
    b.link(gate, "true", set_of, "run");
    b.link(position_of, "position", set_of, "position");
    let spawn = b.node("spawn", json!({ "prefab": "thing.bprefab" }));
    b.link(set_of, "then", spawn, "run");
    b.link(pointer, "position", spawn, "position");
    let destroy = b.node("destroySelf", json!({}));
    b.link(spawn, "then", destroy, "run");
    // Variables and messages: On Message keeps the value; a send happens behind the false branch.
    let on_message = b.node("message", json!({ "message": "ping" }));
    let keep = b.node("setVariable", json!({ "name": "pings" }));
    b.link(on_message, "then", keep, "run");
    b.link(on_message, "value", keep, "value");
    let read = b.node("variable", json!({ "name": "pings" }));
    // Vector and true/false variables, a key press and a timer.
    let pressed = b.node("keyPressed", json!({ "key": "Q" }));
    let keep_vector = b.node("setVectorVariable", json!({}));
    b.link(pressed, "then", keep_vector, "run");
    b.link(make, "vector", keep_vector, "value");
    let read_vector = b.node("vectorVariable", json!({}));
    let timer = b.node("timer", json!({ "seconds": 0.5 }));
    let keep_flag = b.node("setFlag", json!({ "value": true }));
    b.link(timer, "then", keep_flag, "run");
    let read_flag = b.node("flag", json!({}));
    let gate_on_flag = b.node("branch", json!({}));
    b.link(keep_flag, "then", gate_on_flag, "run");
    b.link(read_flag, "value", gate_on_flag, "condition");
    // A gamepad: the stick moves, a button and the trigger decide.
    let stick = b.node("padStick", json!({ "stick": "Right" }));
    let trigger = b.node("padTrigger", json!({}));
    let pad_button = b.node("padButton", json!({ "button": "North" }));
    let pad_gate = b.node("and", json!({}));
    b.link(pad_button, "down", pad_gate, "a");
    b.link(stick, "connected", pad_gate, "b");
    let pull = b.node("greater", json!({ "b": 0.5 }));
    b.link(trigger, "amount", pull, "a");
    let pad_branch = b.node("branch", json!({}));
    b.link(keep_vector, "then", pad_branch, "run");
    b.link(pad_gate, "result", pad_branch, "condition");
    let pad_move = b.node("moveBy", json!({}));
    b.link(pad_branch, "true", pad_move, "run");
    b.link(stick, "direction", pad_move, "offset");
    let pad_log = b.node("log", json!({ "message": "pulled" }));
    b.link(pad_move, "then", pad_log, "run");
    let pulled = b.node("choose", json!({ "a": 1.0, "b": 0.0 }));
    b.link(pull, "result", pulled, "condition");
    b.link(pulled, "result", pad_log, "value");
    let ray = b.node("raycast", json!({}));
    let ray_gate = b.node("branch", json!({}));
    b.link(pad_log, "then", ray_gate, "run");
    b.link(ray, "hit", ray_gate, "condition");
    let ray_log = b.node("log", json!({ "message": "hit" }));
    b.link(ray_gate, "true", ray_log, "run");
    b.link(ray, "distance", ray_log, "value");
    let ray_move = b.node("setPosition", json!({}));
    b.link(ray_log, "then", ray_move, "run");
    b.link(ray, "point", ray_move, "position");
    let push = b.node("addImpulse", json!({}));
    b.link(ray_move, "then", push, "run");
    let speed = b.node("velocity", json!({}));
    b.link(speed, "velocity", push, "impulse");
    let set_speed = b.node("setVelocity", json!({ "object": "Scripted" }));
    b.link(push, "then", set_speed, "run");
    let walk = b.node("walk", json!({}));
    b.link(set_speed, "then", walk, "run");
    let jump = b.node("jump", json!({ "object": "Scripted" }));
    let standing = b.node("onGround", json!({}));
    let jump_gate = b.node("branch", json!({}));
    b.link(walk, "then", jump_gate, "run");
    b.link(standing, "grounded", jump_gate, "condition");
    b.link(jump_gate, "true", jump, "run");
    let fire = b.node("actionPressed", json!({ "action": "Fire" }));
    let fire_log = b.node("log", json!({ "message": "fire" }));
    b.link(fire, "then", fire_log, "run");
    let aim = b.node("action", json!({ "action": "Aim" }));
    b.link(aim, "value", fire_log, "value");
    let aim_gate = b.node("branch", json!({}));
    b.link(fire_log, "then", aim_gate, "run");
    b.link(aim, "held", aim_gate, "condition");
    let score = b.node("gameValue", json!({}));
    let keep_score = b.node("setGameValue", json!({ "scope": "Player" }));
    b.link(jump, "then", keep_score, "run");
    b.link(score, "value", keep_score, "value");
    let more = b.node("changeGameValue", json!({}));
    b.link(keep_score, "then", more, "run");
    let to_menu = b.node("setInputMap", json!({ "map": "Menu", "scope": "This Object" }));
    b.node("menuPath", json!({ "path": "Tests/Every Node" }));
    b.link(more, "then", to_menu, "run");
    let finish = b.node("endGame", json!({}));
    b.link(to_menu, "then", finish, "run");
    b.link(score, "set", finish, "won");
    let move_to = b.node("setPositionOf", json!({ "object": "Scripted" }));
    b.link(gate_on_flag, "false", move_to, "run");
    b.link(read_vector, "value", move_to, "position");
    let send = b.node("sendMessage", json!({ "message": "ping", "object": "Scripted" }));
    b.link(destroy, "then", send, "run");
    b.link(read, "value", send, "value");
    for kind in CATALOG.nodes.iter().map(|d| d.kind) {
        assert!(b.0.nodes.iter().any(|n| n.kind == kind), "{kind} is not in the test graph");
    }
    let (mut scene, _, mut host) = run(&b.bytes(), &[]);
    host.update(&mut scene, 0.1, 1.0, &Arc::new(Input::default()));
    assert!(host.take_faults().is_empty());
    assert_eq!(host.take_logs()[0].text, "Hello 0");
}

/// Two objects: "Sender" sends `ping` with its counter to "Receiver" on each step, and "Receiver"
/// moves to the number it gets.
#[test]
fn messages_reach_the_named_object_after_the_step_and_variables_keep_numbers() {
    let mut sender = Builder::new();
    let update = sender.node("update", json!({}));
    let count = sender.node("variable", json!({ "name": "count" }));
    let plus = sender.node("add", json!({ "b": 1.0 }));
    sender.link(count, "value", plus, "a");
    let keep = sender.node("setVariable", json!({ "name": "count" }));
    sender.link(update, "then", keep, "run");
    sender.link(plus, "result", keep, "value");
    let send = sender.node("sendMessage", json!({ "message": "ping", "object": "Receiver" }));
    sender.link(keep, "then", send, "run");
    let again = sender.node("variable", json!({ "name": "count" }));
    sender.link(again, "value", send, "value");

    let mut receiver = Builder::new();
    let on_ping = receiver.node("message", json!({ "message": "ping" }));
    let to = receiver.node("vector", json!({}));
    receiver.link(on_ping, "value", to, "x");
    let place = receiver.node("setPosition", json!({}));
    receiver.link(on_ping, "then", place, "run");
    receiver.link(to, "vector", place, "position");
    let on_other = receiver.node("message", json!({ "message": "other" }));
    let log = receiver.node("log", json!({ "message": "other" }));
    receiver.link(on_other, "then", log, "run");

    let mut scene = Scene::new();
    let a = scene.create_object(None, "Sender").unwrap();
    let b = scene.create_object(None, "Receiver").unwrap();
    let script = ScriptRef { module: "x.bscript".into(), properties: BTreeMap::new() };
    let input = Arc::new(Input::default());
    let mut host = ScriptHost::new();
    host.load_bytes(a, 0, Path::new("sender.bscript"), None, &sender.bytes(), &script, &mut scene, 0.0, &input).unwrap();
    host.load_bytes(b, 0, Path::new("receiver.bscript"), None, &receiver.bytes(), &script, &mut scene, 0.0, &input).unwrap();
    for _ in 0..3 {
        host.update(&mut scene, 0.1, 0.0, &input);
    }
    assert!(host.take_faults().is_empty());
    assert_eq!(transform(&scene, b).position[0], 3.0, "the third ping carries the counter 3");
    assert_eq!(transform(&scene, a).position[0], 0.0, "the sender did not get its own message");
    assert!(host.take_logs().is_empty(), "On Message runs only for its own name");
}

#[test]
fn a_vector_into_a_number_input_is_a_problem_on_that_node() {
    let mut b = Builder::new();
    let start = b.node("start", json!({}));
    let position = b.node("position", json!({}));
    let log = b.node("log", json!({}));
    b.link(start, "then", log, "run");
    let add = b.node("add", json!({}));
    b.link(position, "position", add, "a");
    b.link(add, "result", log, "value");
    let problems = compile(&b.0).unwrap_err();
    assert_eq!(problems[0].node, Some(log));
    assert!(problems[0].message.contains("Split Vector"), "{problems:?}");
}

#[test]
fn a_click_spawns_and_names_must_be_given() {
    let mut b = Builder::new();
    let update = b.node("update", json!({}));
    let mouse = b.node("mouseDown", json!({}));
    let branch = b.node("branch", json!({}));
    b.link(update, "then", branch, "run");
    b.link(mouse, "down", branch, "condition");
    let spawn = b.node("spawn", json!({ "prefab": "ball.bprefab" }));
    b.link(branch, "true", spawn, "run");
    let pointer = b.node("pointer", json!({}));
    b.link(pointer, "position", spawn, "position");
    let (mut scene, id, mut host) = run(&b.bytes(), &[]);
    host.update(&mut scene, 0.1, 0.0, &Arc::new(Input { pointer: Some([0.5, -0.25]), buttons: 1, ..Input::default() }));
    assert_eq!(host.take_requests(), [crate::Request::Spawn { by: id, prefab: "ball.bprefab".into(), position: [0.5, -0.25, 0.0] }]);

    let mut b = Builder::new();
    let start = b.node("start", json!({}));
    let set_of = b.node("setPositionOf", json!({}));
    b.link(start, "then", set_of, "run");
    let problems = compile(&b.0).unwrap_err();
    assert_eq!(problems[0].node, Some(set_of));
    assert!(problems[0].message.contains("name the object"));
}

#[test]
fn key_presses_run_once_per_press_and_timers_keep_time() {
    let mut b = Builder::new();
    let pressed = b.node("keyPressed", json!({ "key": "Space" }));
    let count = b.node("variable", json!({}));
    let plus = b.node("add", json!({ "b": 1.0 }));
    b.link(count, "value", plus, "a");
    let keep = b.node("setVariable", json!({}));
    b.link(pressed, "then", keep, "run");
    b.link(plus, "result", keep, "value");
    // Every 0.25 s, x of the vector variable grows by 1 and the object moves there.
    let timer = b.node("timer", json!({ "seconds": 0.25 }));
    let at = b.node("vectorVariable", json!({ "name": "spot" }));
    let step = b.node("add", json!({ "b": [1.0, 0.0, 0.0] }));
    b.link(at, "value", step, "a");
    let keep_spot = b.node("setVectorVariable", json!({ "name": "spot" }));
    b.link(timer, "then", keep_spot, "run");
    b.link(step, "result", keep_spot, "value");
    let place = b.node("setPosition", json!({}));
    b.link(keep_spot, "then", place, "run");
    let now = b.node("vectorVariable", json!({ "name": "spot" }));
    b.link(now, "value", place, "position");
    let log = b.node("log", json!({ "message": "presses" }));
    b.link(place, "then", log, "run");
    b.link(count, "value", log, "value");
    let (mut scene, id, mut host) = run(&b.bytes(), &[]);
    let held = Arc::new(Input { keys: ["Space".to_owned()].into_iter().collect(), ..Input::default() });
    let free = Arc::new(Input::default());
    // Held for three steps, let go, pressed again: two presses. Six steps of 0.1 s: two timer runs.
    for input in [&held, &held, &held, &free, &held, &free] {
        host.update(&mut scene, 0.1, 0.0, input);
    }
    assert!(host.take_faults().is_empty());
    assert_eq!(transform(&scene, id).position[0], 2.0, "the timer ran twice in 0.6 s");
    assert_eq!(host.take_logs().last().unwrap().text, "presses 2");
}
#[test]
fn gamepad_sticks_and_buttons_reach_visual_scripts() {
    let mut b = Builder::new();
    let update = b.node("update", json!({}));
    let held = b.node("padButton", json!({ "button": "South" }));
    let branch = b.node("branch", json!({}));
    b.link(update, "then", branch, "run");
    b.link(held, "down", branch, "condition");
    let stick = b.node("padStick", json!({}));
    let place = b.node("setPosition", json!({}));
    b.link(branch, "true", place, "run");
    b.link(stick, "direction", place, "position");
    let (mut scene, id, mut host) = run(&b.bytes(), &[]);
    host.update(&mut scene, 0.1, 0.0, &Arc::new(Input::default()));
    assert_eq!(transform(&scene, id).position, [0.0; 3], "no gamepad, nothing held");
    let pad = crate::Pad { axes: [0.5, -0.25, 0.0, 0.0, 0.0, 0.0], buttons: 1 };
    host.update(&mut scene, 0.1, 0.0, &Arc::new(Input { pad: Some(pad), ..Input::default() }));
    assert_eq!(transform(&scene, id).position, [0.5, -0.25, 0.0], "South held: the object goes where the left stick points");

    let mut wrong = Builder::new();
    let start = wrong.node("start", json!({}));
    let button = wrong.node("padButton", json!({ "button": "Jump" }));
    let gate = wrong.node("branch", json!({}));
    wrong.link(start, "then", gate, "run");
    wrong.link(button, "down", gate, "condition");
    assert!(compile(&wrong.0).unwrap_err()[0].message.contains("no gamepad button Jump"));
}

#[test]
fn a_raycast_finds_the_nearest_turned_box_and_misses_when_nothing_is_there() {
    let mut b = Builder::new();
    let update = b.node("update", json!({}));
    let ray = b.node("raycast", json!({ "direction": [1.0, 0.0, 0.0], "distance": 20.0 }));
    let branch = b.node("branch", json!({}));
    b.link(update, "then", branch, "run");
    b.link(ray, "hit", branch, "condition");
    let log = b.node("log", json!({ "message": "hit at" }));
    b.link(branch, "true", log, "run");
    b.link(ray, "distance", log, "value");
    let miss = b.node("log", json!({ "message": "miss" }));
    b.link(branch, "false", miss, "run");

    let (mut scene, _, mut host) = run(&b.bytes(), &[]);
    host.update(&mut scene, 0.1, 0.0, &Arc::new(Input::default()));
    assert_eq!(host.take_logs()[0].text, "miss", "nothing to hit yet");

    // A cube turned 45° about Y at x = 5, and a farther one behind it.
    let cube = json!({"shape": "cube", "color": [1, 1, 1, 1], "metallic": 0, "roughness": 0.5});
    for (name, x) in [("Near", 5.0), ("Far", 9.0)] {
        let id = scene.create_object(None, name).unwrap();
        scene.add_component(id, "mesh").unwrap();
        scene.set_component(id, "mesh", cube.clone()).unwrap();
        scene.set_component(id, "transform", json!({"position": [x, 0, 0], "rotation": [0, 45, 0], "scale": [1, 1, 1]})).unwrap();
    }
    host.update(&mut scene, 0.1, 0.0, &Arc::new(Input::default()));
    let text = host.take_logs()[0].text.clone();
    let distance: f32 = text.trim_start_matches("hit at ").parse().unwrap();
    assert!((distance - (5.0 - std::f32::consts::FRAC_1_SQRT_2)).abs() < 0.01, "the corner of the turned cube is hit first: {text}");
}

#[test]
fn move_by_keeps_small_steps_far_from_the_centre() {
    // Single precision rounds to 1/16 of a unit at 1000 km; double precision keeps every millimetre.
    let mut b = Builder::new();
    let update = b.node("update", json!({}));
    let step = b.node("moveBy", json!({ "offset": [0.001, 0.0, 0.0] }));
    b.link(update, "then", step, "run");
    let (mut scene, id, mut host) = run(&b.bytes(), &[]);
    let far = 1.0e6;
    scene.set(id, Transform { position: [far, 0.0, 0.0], rotation: [0.0; 3], scale: [1.0; 3] }).unwrap();
    for _ in 0..100 {
        host.update(&mut scene, 0.1, 0.0, &Arc::new(Input::default()));
    }
    assert!(host.take_faults().is_empty());
    let moved = transform(&scene, id).position[0] - far;
    assert!((moved - 0.1).abs() < 1e-6, "100 steps of 1 mm make 10 cm: {moved}");
}

#[test]
fn game_state_starts_empty_counts_from_zero_and_the_first_ending_counts() {
    let mut b = Builder::new();
    let start = b.node("start", json!({}));
    let before = b.node("gameValue", json!({ "name": "coins" }));
    let report = b.node("log", json!({ "message": "set before" }));
    b.link(start, "then", report, "run");
    let as_number = b.node("choose", json!({ "a": 1.0, "b": 0.0 }));
    b.link(before, "set", as_number, "condition");
    b.link(as_number, "result", report, "value");
    let add = b.node("changeGameValue", json!({ "name": "coins", "amount": 2.0 }));
    b.link(report, "then", add, "run");
    let again = b.node("changeGameValue", json!({ "name": "coins", "amount": 3.0 }));
    b.link(add, "then", again, "run");
    let lives = b.node("setGameValue", json!({ "name": "lives", "scope": "Player", "value": 3.0 }));
    b.link(again, "then", lives, "run");
    let win = b.node("endGame", json!({}));
    b.link(lives, "then", win, "run");
    let lose = b.node("endGame", json!({ "won": false }));
    b.link(win, "then", lose, "run");
    let (_, _, mut host) = run(&b.bytes(), &[]);
    assert!(host.take_faults().is_empty());
    assert_eq!(host.take_logs()[0].text, "set before 0", "nothing is set at first");
    let state = host.state();
    assert_eq!(state.game.get("coins"), Some(&5.0), "a missing value counts from 0");
    assert!(state.players.is_empty(), "with no player, player values go nowhere");
    assert_eq!(state.outcome, Some(true), "the first ending counts");
}

#[test]
fn game_values_reach_this_player_a_named_player_or_the_first_and_never_a_missing_one() {
    let mut b = Builder::new();
    let start = b.node("start", json!({}));
    let mut last = (start, "then");
    for (scope, player, value) in [("This Object", "", 2.0), ("Player", "Red", 1.0), ("Player", "", 7.0), ("Player", "Nobody", 9.0), ("Player", "Rock", 9.0)] {
        let set = b.node("setGameValue", json!({ "name": "lives", "scope": scope, "player": player, "value": value }));
        b.link(last.0, last.1, set, "run");
        last = (set, "then");
    }
    let mut scene = Scene::new();
    let red = scene.create_object(None, "Red").unwrap();
    let blue = scene.create_object(None, "Blue").unwrap();
    scene.create_object(None, "Rock").unwrap();
    for player in [red, blue] {
        scene.set(player, bayleaf_scene::components::PlayerControl::default()).unwrap();
    }
    let script = ScriptRef { module: "test.bscript".into(), properties: BTreeMap::new() };
    let mut host = ScriptHost::new();
    host.load_bytes(blue, 0, Path::new("test.bscript"), None, &b.bytes(), &script, &mut scene, 0.0, &Arc::new(Input::default())).unwrap();
    assert!(host.take_faults().is_empty(), "a missing player is no fault");
    let state = host.state();
    assert_eq!(state.get(crate::Scope::Player(blue), "lives"), Some(2.0), "This Object is the script's own player");
    assert_eq!(state.get(crate::Scope::Player(red), "lives"), Some(7.0), "a named player, then the first player, which is Red");
    assert_eq!(state.players.len(), 2, "no state for a missing name or an object that is no player");
}

#[test]
fn a_menu_path_node_tells_where_add_component_lists_the_script() {
    let mut b = spinner();
    b.node("menuPath", json!({ "path": " Light / Flicker " }));
    let described = ScriptHost::new().describe_all("flicker.bscript", &b.bytes()).unwrap();
    assert_eq!(described.menu.as_deref(), Some("Light/Flicker"), "parts are trimmed");
    assert_eq!(ScriptHost::new().describe_all("spin.bscript", &spinner().bytes()).unwrap().menu, None, "no node, no path");

    let mut bad = spinner();
    bad.node("menuPath", json!({ "path": "Light//Flicker" }));
    assert!(compile(&bad.0).is_err(), "an empty part is refused");
}

#[test]
fn declared_variables_start_at_their_values_and_undeclared_ones_at_zero() {
    let mut b = Builder::new();
    b.0.add_variable("lives", bayleaf_graph::VariableType::Number, Some(json!(3.0))).unwrap();
    let start = b.node("start", json!({}));
    let lives = b.node("variable", json!({ "name": "lives" }));
    let other = b.node("variable", json!({ "name": "unnamed" }));
    let say_lives = b.node("log", json!({ "message": "lives" }));
    let say_other = b.node("log", json!({ "message": "other" }));
    b.link(start, "then", say_lives, "run");
    b.link(lives, "value", say_lives, "value");
    b.link(say_lives, "then", say_other, "run");
    b.link(other, "value", say_other, "value");
    let (_, _, mut host) = run(&b.bytes(), &[]);
    let logs: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(logs, ["lives 3", "other 0"]);
}
