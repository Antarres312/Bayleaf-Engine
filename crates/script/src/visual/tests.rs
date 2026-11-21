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
        assert!((got - x.sin()).abs() < 1e-4, "sin({x}) = {got}");
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
