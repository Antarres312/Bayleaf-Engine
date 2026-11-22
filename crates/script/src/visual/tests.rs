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
    b.0.menu = "Tests/Every Node".to_owned();
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
    // Searches, behind the branch that stays false: every object with a Transform, one by name.
    let each = b.node("forEachWith", json!({ "kind": "Transform" }));
    b.link(finish, "then", each, "run");
    let each_log = b.node("log", json!({ "message": "each" }));
    b.link(each, "each", each_log, "run");
    b.link(each, "index", each_log, "value");
    let place = b.node("setPositionOf", json!({}));
    b.link(each_log, "then", place, "run");
    b.link(each, "object", place, "target");
    let named = b.node("findNamed", json!({ "name": "Scripted" }));
    let where_named = b.node("positionOf", json!({}));
    b.link(named, "object", where_named, "target");
    b.link(where_named, "position", place, "position");
    let counted = b.node("countWith", json!({ "kind": "transform" }));
    let first_with = b.node("findWith", json!({ "kind": "transform" }));
    b.link(counted, "count", first_with, "index");
    let any_found = b.node("and", json!({}));
    b.link(first_with, "found", any_found, "a");
    b.link(named, "found", any_found, "b");
    let found_gate = b.node("branch", json!({}));
    b.link(each, "then", found_gate, "run");
    b.link(any_found, "result", found_gate, "condition");
    let remove = b.node("destroyObject", json!({}));
    b.link(found_gate, "true", remove, "run");
    b.link(first_with, "object", remove, "target");
    let me = b.node("thisObject", json!({}));
    let push_me = b.node("addImpulse", json!({}));
    b.link(remove, "then", push_me, "run");
    b.link(me, "object", push_me, "target");
    // Component fields: read this object's Transform, and set a light behind a found check.
    let read_transform = b.node("getComponent", json!({ "component": "Transform" }));
    let lamp = b.node("tryGetComponent", json!({ "component": "pointLight" }));
    let lamp_gate = b.node("branch", json!({}));
    b.link(push_me, "then", lamp_gate, "run");
    b.link(lamp, "found", lamp_gate, "condition");
    let brighten = b.node("setComponent", json!({ "component": "Point Light" }));
    b.link(lamp_gate, "true", brighten, "run");
    b.link(lamp, "intensity", brighten, "intensity");
    b.link(read_transform, "scale", brighten, "color");
    b.link(lamp, "shadows", brighten, "shadows");
    // On Init, and text: every text node feeding a log behind a branch that stays false.
    let init = b.node("init", json!({}));
    let text_gate = b.node("branch", json!({}));
    b.link(init, "then", text_gate, "run");
    let me_named = b.node("nameOf", json!({}));
    let words = b.node("text", json!({ "value": " Hi " }));
    let trimmed = b.node("trimText", json!({}));
    b.link(words, "result", trimmed, "text");
    let upper = b.node("upperCase", json!({}));
    b.link(trimmed, "result", upper, "text");
    let lower = b.node("lowerCase", json!({}));
    b.link(upper, "result", lower, "text");
    let joined = b.node("joinText", json!({}));
    b.link(lower, "result", joined, "a");
    b.link(me_named, "result", joined, "b");
    let length = b.node("textLength", json!({}));
    b.link(joined, "result", length, "text");
    let spelled = b.node("numberToText", json!({}));
    b.link(length, "result", spelled, "value");
    let read_back = b.node("textToNumber", json!({}));
    b.link(spelled, "result", read_back, "text");
    let cut = b.node("textPart", json!({ "start": 1.0, "count": 2.0 }));
    b.link(joined, "result", cut, "text");
    let swapped = b.node("replaceText", json!({ "find": "i", "with": "I" }));
    b.link(cut, "result", swapped, "text");
    let where_in = b.node("findText", json!({ "part": "S" }));
    b.link(swapped, "result", where_in, "text");
    let checks = [("sameText", "a", "b"), ("textContains", "text", "part"), ("startsWith", "text", "part"), ("endsWith", "text", "part")];
    let mut condition = read_back;
    let mut condition_pin = "valid";
    for (kind, first, second) in checks {
        let check = b.node(kind, json!({}));
        b.link(swapped, "result", check, first);
        b.link(joined, "result", check, second);
        let both = b.node("and", json!({}));
        b.link(condition, condition_pin, both, "a");
        b.link(check, "result", both, "b");
        condition = both;
        condition_pin = "result";
    }
    b.link(condition, condition_pin, text_gate, "condition");
    let text_log = b.node("log", json!({ "message": "text" }));
    b.link(text_gate, "true", text_log, "run");
    b.link(swapped, "result", text_log, "text");
    b.link(where_in, "result", text_log, "value");
    // Text, object and colour variables.
    let keep_text = b.node("setTextVariable", json!({ "value": "kept" }));
    let keep_object = b.node("setObjectVariable", json!({}));
    let keep_color = b.node("setColorVariable", json!({}));
    b.link(keep_text, "then", keep_object, "run");
    b.link(keep_object, "then", keep_color, "run");
    b.link(me, "object", keep_object, "value");
    let read_text = b.node("textVariable", json!({}));
    let read_object = b.node("objectVariable", json!({}));
    let read_color = b.node("colorVariable", json!({}));
    b.link(read_color, "value", keep_color, "value");
    let object_name = b.node("nameOf", json!({}));
    b.link(read_object, "value", object_name, "target");
    let both_texts = b.node("joinText", json!({}));
    b.link(read_text, "value", both_texts, "a");
    b.link(object_name, "result", both_texts, "b");
    let kept_gate = b.node("branch", json!({}));
    b.link(keep_color, "then", kept_gate, "run");
    b.link(read_object, "found", kept_gate, "condition");
    let kept_log = b.node("log", json!({ "message": "kept" }));
    b.link(kept_gate, "false", kept_log, "run");
    b.link(both_texts, "result", kept_log, "text");
    let by_name = b.node("findNamed", json!({}));
    b.link(me_named, "result", by_name, "named");
    let named_gate = b.node("branch", json!({}));
    b.link(text_log, "then", named_gate, "run");
    b.link(by_name, "found", named_gate, "condition");
    b.link(named_gate, "true", keep_text, "run");
    // Sounds, behind a branch that stays false.
    let beep = b.node("playSoundAt", json!({ "sound": "beep.wav" }));
    let play_mine = b.node("playSound", json!({}));
    let stop_mine = b.node("stopSound", json!({}));
    b.link(named_gate, "false", beep, "run");
    b.link(beep, "then", play_mine, "run");
    b.link(play_mine, "then", stop_mine, "run");
    // Scenes, behind a branch that stays false.
    let add_room = b.node("loadScene", json!({ "scene": "room.bscene", "additive": true }));
    let drop_room = b.node("unloadScene", json!({ "scene": "room.bscene" }));
    b.link(stop_mine, "then", add_room, "run");
    b.link(add_room, "then", drop_room, "run");
    // Whole numbers: typed, made whole, divided with a remainder, kept in a variable.
    let seven = b.node("wholeNumber", json!({ "value": 7 }));
    let made = b.node("toWhole", json!({ "rounding": "Up" }));
    b.link(time, "seconds", made, "value");
    let rest = b.node("remainder", json!({}));
    b.link(seven, "result", rest, "a");
    b.link(made, "result", rest, "b");
    let keep_whole = b.node("setWholeVariable", json!({}));
    b.link(keep_color, "then", keep_whole, "run");
    b.link(rest, "result", keep_whole, "value");
    let read_whole = b.node("wholeVariable", json!({}));
    let whole_log = b.node("log", json!({ "message": "whole" }));
    b.link(keep_whole, "then", whole_log, "run");
    b.link(read_whole, "value", whole_log, "value");
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
fn the_menu_path_tells_where_add_component_lists_the_script_and_old_menu_nodes_move_into_it() {
    let mut b = spinner();
    b.0.menu = " Light / Flicker ".to_owned();
    let described = ScriptHost::new().describe_all("flicker.bscript", &b.bytes()).unwrap();
    assert_eq!(described.menu.as_deref(), Some("Light/Flicker"), "parts are trimmed");
    assert_eq!(ScriptHost::new().describe_all("spin.bscript", &spinner().bytes()).unwrap().menu, None, "no path, under Scripts");

    let mut bad = spinner();
    bad.0.menu = "Light//Flicker".to_owned();
    assert!(compile(&bad.0).is_err(), "an empty part is refused");

    // A version 2 file kept the path in a Menu Path node; reading it moves the path into the field.
    let mut old = serde_json::to_value(&spinner().0).unwrap();
    old["version"] = json!(2);
    old["nodes"].as_array_mut().unwrap().push(json!({ "id": 99, "kind": "menuPath", "position": [0, 0], "values": { "path": "Light/Old" } }));
    let read = Graph::from_json(&old.to_string(), &CATALOG).unwrap();
    assert_eq!(read.menu, "Light/Old");
    assert!(read.nodes.iter().all(|n| n.kind != "menuPath"), "the old node is gone");
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

/// A script finds the objects that have a component, a tag or a script, one by one or all at once.
#[test]
fn searches_find_objects_by_component_tag_and_script_in_hierarchy_order() {
    let mut b = Builder::new();
    let start = b.node("start", json!({}));
    // Log how many lamps there are, then line every lamp up along X by its index.
    let count = b.node("countWith", json!({ "kind": "Point Light" }));
    let log = b.node("log", json!({ "message": "lamps" }));
    b.link(start, "then", log, "run");
    b.link(count, "count", log, "value");
    let each = b.node("forEachWith", json!({ "kind": "point_light" }));
    b.link(log, "then", each, "run");
    let place = b.node("setPositionOf", json!({}));
    b.link(each, "each", place, "run");
    b.link(each, "object", place, "target");
    let at = b.node("vector", json!({}));
    b.link(each, "index", at, "x");
    b.link(at, "vector", place, "position");
    // The second object tagged "goal" moves up; a name that finds nothing says so.
    let goal = b.node("findWith", json!({ "kind": "Goal", "index": 1.0 }));
    let lift = b.node("setPositionOf", json!({ "position": [0.0, 5.0, 0.0] }));
    b.link(each, "then", lift, "run");
    b.link(goal, "object", lift, "target");
    let nobody = b.node("findNamed", json!({ "name": "Nobody" }));
    let said = b.node("choose", json!({ "a": 1.0, "b": 0.0 }));
    b.link(nobody, "found", said, "condition");
    let report = b.node("log", json!({ "message": "nobody" }));
    b.link(lift, "then", report, "run");
    b.link(said, "result", report, "value");
    let movers = b.node("countWith", json!({ "kind": "My Mover" }));
    let tell = b.node("log", json!({ "message": "movers" }));
    b.link(report, "then", tell, "run");
    b.link(movers, "count", tell, "value");
    let bytes = b.bytes();

    let mut scene = Scene::new();
    let id = scene.create_object(None, "Scripted").unwrap();
    let mut make = |name: &str, component: Option<&str>, tags: &[&str]| {
        let object = scene.create_object(None, name).unwrap();
        if let Some(component) = component {
            scene.add_component(object, component).unwrap();
        }
        if !tags.is_empty() {
            scene.add_component(object, "tags").unwrap();
            scene.set_component(object, "tags", json!(tags)).unwrap();
        }
        object
    };
    let lamp_a = make("Lamp A", Some("pointLight"), &[]);
    let crate_box = make("Crate", None, &["goal"]);
    let lamp_b = make("Lamp B", Some("pointLight"), &[]);
    let flag = make("Flag", None, &["Goal"]);
    let mover = make("Mover", Some("scripts"), &[]);
    scene.set_component(mover, "scripts", json!([{ "module": "scripts/my_mover.wasm" }])).unwrap();
    let script = ScriptRef { module: "seeker.bscript".into(), properties: BTreeMap::new() };
    let mut host = ScriptHost::new();
    host.load_bytes(id, 0, Path::new("seeker.bscript"), None, &bytes, &script, &mut scene, 0.0, &Arc::new(Input::default())).unwrap();

    assert!(host.take_faults().is_empty());
    let lines: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(lines, ["lamps 2", "nobody 0", "movers 1"]);
    let x = |object| transform(&scene, object).position[0];
    assert!(x(lamp_b) > x(lamp_a), "the lamps line up in hierarchy order: {} then {}", x(lamp_a), x(lamp_b));
    assert!(transform(&scene, flag).position[1] > transform(&scene, crate_box).position[1], "the second goal moved, not the first");

    // A search left at the Point Light the editor shows compiles as that.
    let mut plain = Builder::new();
    let go = plain.node("start", json!({}));
    let counted = plain.node("countWith", json!({}));
    let said = plain.node("log", json!({ "message": "lights" }));
    plain.link(go, "then", said, "run");
    plain.link(counted, "count", said, "value");
    assert!(compile(&plain.0).is_ok(), "{:?}", compile(&plain.0).err());
}

/// A script reads a lamp's fields, asks to change some of them, and learns when an object has no
/// such component.
#[test]
fn component_fields_are_read_as_pins_and_set_only_where_linked() {
    let mut b = Builder::new();
    let start = b.node("start", json!({}));
    let lamp = b.node("findNamed", json!({ "name": "Lamp" }));
    let light = b.node("tryGetComponent", json!({ "component": "Point Light" }));
    b.link(lamp, "object", light, "target");
    let log = b.node("log", json!({ "message": "intensity" }));
    b.link(start, "then", log, "run");
    b.link(light, "intensity", log, "value");
    let doubled = b.node("multiply", json!({ "b": 2.0 }));
    b.link(light, "intensity", doubled, "a");
    let brighten = b.node("setComponent", json!({ "component": "pointLight" }));
    b.link(log, "then", brighten, "run");
    b.link(lamp, "object", brighten, "target");
    b.link(doubled, "result", brighten, "intensity");
    // This object has no light: Found is false and its fields read as zeros.
    let mine = b.node("tryGetComponent", json!({ "component": "Point Light" }));
    let said = b.node("choose", json!({ "a": 1.0, "b": 0.0 }));
    b.link(mine, "found", said, "condition");
    let report = b.node("log", json!({ "message": "mine" }));
    b.link(brighten, "then", report, "run");
    b.link(said, "result", report, "value");
    let bytes = b.bytes();

    let mut scene = Scene::new();
    let id = scene.create_object(None, "Scripted").unwrap();
    let lamp_id = scene.create_object(None, "Lamp").unwrap();
    scene.add_component(lamp_id, "pointLight").unwrap();
    scene.set_component(lamp_id, "pointLight", json!({ "color": [1, 0.5, 0], "intensity": 3, "range": 10, "shadows": false })).unwrap();
    let script = ScriptRef { module: "lamps.bscript".into(), properties: BTreeMap::new() };
    let mut host = ScriptHost::new();
    host.load_bytes(id, 0, Path::new("lamps.bscript"), None, &bytes, &script, &mut scene, 0.0, &Arc::new(Input::default())).unwrap();

    assert!(host.take_faults().is_empty());
    let lines: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(lines, ["intensity 3", "mine 0"]);
    let requests = host.take_requests();
    assert_eq!(requests.len(), 1, "only the linked field is set: {requests:?}");
    let crate::Request::SetField { object, component, field, value, .. } = &requests[0] else { panic!("{requests:?}") };
    assert_eq!((*object, *component, field.as_str()), (lamp_id, "pointLight", "intensity"));
    assert!(value[0] > 3.0, "the doubled intensity: {value:?}");

    // A colour set from three numbers keeps its alpha; a true/false field takes whether it is not 0.
    assert_eq!(crate::with_field(&json!([1, 1, 1, 0.5]), [0.0, 0.25, 1.0]), Some(json!([0.0, 0.25, 1.0, 0.5])));
    assert_eq!(crate::with_field(&json!(false), [1.0, 0.0, 0.0]), Some(json!(true)));
    assert_eq!(crate::with_field(&json!("sphere"), [1.0, 0.0, 0.0]), None, "a text field is not set");

    // Physics reads its components when play starts, so setting them is refused with a reason.
    let mut physics = Builder::new();
    let go = physics.node("start", json!({}));
    let heavier = physics.node("setComponent", json!({ "component": "Rigid Body" }));
    physics.link(go, "then", heavier, "run");
    let problems = compile(&physics.0).unwrap_err();
    assert!(problems.iter().any(|p| p.message.contains("read only")), "{problems:?}");
}

/// A script reads another object's script properties by the script's name and sets one; the other
/// script sees the new value from the next step.
#[test]
fn a_script_on_another_object_is_read_and_set_like_a_component() {
    // Bob logs its speed every step.
    let mut bob = Builder::new();
    let update = bob.node("update", json!({}));
    let speed = bob.node("property", json!({ "name": "speed", "default": 2.0 }));
    let log = bob.node("log", json!({ "message": "speed" }));
    bob.link(update, "then", log, "run");
    bob.link(speed, "value", log, "value");
    // The reader reads Bob's speed on the object Target and sets it to 9.
    let mut reader = Builder::new();
    let update = reader.node("update", json!({}));
    let target = reader.node("findNamed", json!({ "name": "Target" }));
    let read = reader.node("tryGetComponent", json!({ "component": "Bob", "fields": ["speed"] }));
    reader.link(target, "object", read, "target");
    let said = reader.node("log", json!({ "message": "read" }));
    reader.link(update, "then", said, "run");
    reader.link(read, "speed", said, "value");
    let nine = reader.node("number", json!({ "value": 9.0 }));
    let write = reader.node("setComponent", json!({ "component": "Bob", "fields": ["speed"] }));
    reader.link(said, "then", write, "run");
    reader.link(target, "object", write, "target");
    reader.link(nine, "value", write, "speed");
    let found = reader.node("choose", json!({ "a": 1.0, "b": 0.0 }));
    reader.link(read, "found", found, "condition");
    let told = reader.node("log", json!({ "message": "found" }));
    reader.link(write, "then", told, "run");
    reader.link(found, "result", told, "value");

    let mut scene = Scene::new();
    let target_id = scene.create_object(None, "Target").unwrap();
    let reader_id = scene.create_object(None, "Reader").unwrap();
    let mut host = ScriptHost::new();
    let input = Arc::new(Input::default());
    let bob_ref = ScriptRef { module: "bob.bscript".into(), properties: BTreeMap::new() };
    host.load_bytes(target_id, 0, Path::new("bob.bscript"), None, &bob.bytes(), &bob_ref, &mut scene, 0.0, &input).unwrap();
    let reader_ref = ScriptRef { module: "reader.bscript".into(), properties: BTreeMap::new() };
    host.load_bytes(reader_id, 0, Path::new("reader.bscript"), None, &reader.bytes(), &reader_ref, &mut scene, 0.0, &input).unwrap();

    host.update(&mut scene, 0.1, 0.1, &input);
    host.update(&mut scene, 0.1, 0.2, &input);
    assert!(host.take_faults().is_empty());
    let lines: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(lines, ["speed 2", "read 2", "found 1", "speed 9", "read 9", "found 1"]);
}

/// Every script of a scene runs On Init, then every one runs On Start, whatever the hierarchy
/// order: a script above another reads the other's properties in its On Start.
#[test]
fn scripts_init_then_start_after_the_whole_scene_is_loaded() {
    let dir = std::env::temp_dir().join(format!("bayleaf-init-order-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // The reader, first in the hierarchy: On Init logs, On Start logs Health's maxHealth.
    let mut reader = Builder::new();
    let init = reader.node("init", json!({}));
    let said = reader.node("log", json!({ "message": "reader init" }));
    reader.link(init, "then", said, "run");
    let start = reader.node("start", json!({}));
    let health = reader.node("findWith", json!({ "kind": "Health" }));
    let get = reader.node("getComponent", json!({ "component": "Health", "fields": ["maxHealth"] }));
    reader.link(health, "object", get, "target");
    let log = reader.node("log", json!({ "message": "max" }));
    reader.link(start, "then", log, "run");
    reader.link(get, "maxHealth", log, "value");
    std::fs::write(dir.join("reader.bscript"), reader.0.to_json()).unwrap();
    // Health, below it: declares maxHealth 100 and logs its own On Init.
    let mut health = Builder::new();
    health.node("property", json!({ "name": "maxHealth", "default": 100.0 }));
    let init = health.node("init", json!({}));
    let said = health.node("log", json!({ "message": "health init" }));
    health.link(init, "then", said, "run");
    std::fs::write(dir.join("health.bscript"), health.0.to_json()).unwrap();

    let mut scene = Scene::new();
    let first = scene.create_object(None, "Reader").unwrap();
    let second = scene.create_object(None, "Player").unwrap();
    scene.add_component(first, "scripts").unwrap();
    scene.set_component(first, "scripts", json!([{ "module": "reader.bscript" }])).unwrap();
    scene.add_component(second, "scripts").unwrap();
    scene.set_component(second, "scripts", json!([{ "module": "health.bscript" }])).unwrap();
    let mut host = ScriptHost::new();
    host.load_scene(&mut scene, &dir, 0.0, &Arc::new(Input::default()));

    assert!(host.take_faults().is_empty());
    let lines: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(lines, ["reader init", "health init", "max 100"], "every init, then every start");
    std::fs::remove_dir_all(&dir).ok();
}

/// Text nodes count, cut and search by characters, so letters outside A to Z count once.
#[test]
fn text_nodes_join_measure_search_cut_replace_and_convert() {
    let mut b = Builder::new();
    let start = b.node("start", json!({}));
    let mut previous = (start, "then");
    let mut line = |b: &mut Builder, message: &str, text: Option<(u32, &str)>, value: Option<(u32, &str)>| {
        let log = b.node("log", json!({ "message": message }));
        b.link(previous.0, previous.1, log, "run");
        if let Some((node, pin)) = text {
            b.link(node, pin, log, "text");
        }
        if let Some((node, pin)) = value {
            b.link(node, pin, log, "value");
        }
        previous = (log, "then");
    };
    let word = b.node("text", json!({ "value": "  Zażółć gęślą  " }));
    let trimmed = b.node("trimText", json!({}));
    b.link(word, "result", trimmed, "text");
    let length = b.node("textLength", json!({}));
    b.link(trimmed, "result", length, "text");
    line(&mut b, "trimmed", Some((trimmed, "result")), Some((length, "result")));
    let cut = b.node("textPart", json!({ "start": 2.0, "count": 4.0 }));
    b.link(trimmed, "result", cut, "text");
    line(&mut b, "part", Some((cut, "result")), None);
    let past_end = b.node("textPart", json!({ "start": 10.0, "count": 100.0 }));
    b.link(trimmed, "result", past_end, "text");
    line(&mut b, "end", Some((past_end, "result")), None);
    let found = b.node("findText", json!({ "part": "gę" }));
    b.link(trimmed, "result", found, "text");
    line(&mut b, "index", None, Some((found, "result")));
    let missing = b.node("findText", json!({ "part": "xyz" }));
    b.link(trimmed, "result", missing, "text");
    line(&mut b, "missing", None, Some((missing, "result")));
    let swapped = b.node("replaceText", json!({ "text": "a-b-c", "find": "-", "with": " + " }));
    line(&mut b, "replaced", Some((swapped, "result")), None);
    let loud = b.node("upperCase", json!({ "text": "Mixed Case 1" }));
    line(&mut b, "upper", Some((loud, "result")), None);
    let joined = b.node("joinText", json!({ "a": "Level ", "b": "" }));
    let number = b.node("numberToText", json!({ "value": 3.5 }));
    b.link(number, "result", joined, "b");
    line(&mut b, "joined", Some((joined, "result")), None);
    let parsed = b.node("textToNumber", json!({ "text": " -2.25 " }));
    line(&mut b, "parsed", None, Some((parsed, "result")));
    let bad = b.node("textToNumber", json!({ "text": "12abc" }));
    let valid = b.node("choose", json!({ "a": 1.0, "b": 0.0 }));
    b.link(bad, "valid", valid, "condition");
    line(&mut b, "bad", None, Some((valid, "result")));
    let same = b.node("sameText", json!({ "a": "abc", "b": "abc" }));
    let ends = b.node("endsWith", json!({ "text": "file.png", "part": ".png" }));
    let both = b.node("and", json!({}));
    b.link(same, "result", both, "a");
    b.link(ends, "result", both, "b");
    let said = b.node("choose", json!({ "a": 1.0, "b": 0.0 }));
    b.link(both, "result", said, "condition");
    line(&mut b, "checks", None, Some((said, "result")));
    let name = b.node("nameOf", json!({}));
    line(&mut b, "name", Some((name, "result")), None);

    let (_, _, mut host) = run(&b.bytes(), &[]);
    assert!(host.take_faults().is_empty());
    let lines: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(
        lines,
        [
            "trimmed Zażółć gęślą 12",
            "part żółć",
            "end lą",
            "index 7",
            "missing -1",
            "replaced a + b + c",
            "upper MIXED CASE 1",
            "joined Level 3.5",
            "parsed -2.25",
            "bad 0",
            "checks 1",
            "name Scripted",
        ]
    );
}

/// Object, text and colour variables keep their values from step to step; an object variable says
/// when its object is gone, and a text variable cuts long text at a character.
#[test]
fn object_text_and_color_variables_last_from_step_to_step() {
    let mut b = Builder::new();
    b.0.add_variable("note", bayleaf_graph::VariableType::Text, Some(json!("Hi"))).unwrap();
    b.0.add_variable("tint", bayleaf_graph::VariableType::Color, Some(json!([0.5, 0.25, 1.0]))).unwrap();
    // On Start: keep the Box, and a text of 256 groups "aéé" (5 bytes each, 1280 bytes).
    let start = b.node("start", json!({}));
    let found = b.node("findNamed", json!({ "name": "Box" }));
    let keep = b.node("setObjectVariable", json!({ "name": "box" }));
    b.link(start, "then", keep, "run");
    b.link(found, "object", keep, "value");
    let long = b.node("replaceText", json!({ "text": "x".repeat(256), "find": "x", "with": "aéé" }));
    let keep_long = b.node("setTextVariable", json!({ "name": "long" }));
    b.link(keep, "then", keep_long, "run");
    b.link(long, "result", keep_long, "value");
    // On Update: say whether the Box is still there, the note, the long text's length and the tint.
    let update = b.node("update", json!({}));
    let read = b.node("objectVariable", json!({ "name": "box" }));
    let there = b.node("choose", json!({ "a": 1.0, "b": 0.0 }));
    b.link(read, "found", there, "condition");
    let log = b.node("log", json!({ "message": "box" }));
    b.link(update, "then", log, "run");
    b.link(there, "result", log, "value");
    let note = b.node("textVariable", json!({ "name": "note" }));
    let said = b.node("log", json!({ "message": "note" }));
    b.link(log, "then", said, "run");
    b.link(note, "value", said, "text");
    let long_now = b.node("textVariable", json!({ "name": "long" }));
    let length = b.node("textLength", json!({}));
    b.link(long_now, "value", length, "text");
    let measured = b.node("log", json!({ "message": "long" }));
    b.link(said, "then", measured, "run");
    b.link(length, "result", measured, "value");
    let tint = b.node("colorVariable", json!({ "name": "tint" }));
    let green = b.node("split", json!({}));
    b.link(tint, "value", green, "vector");
    let shade = b.node("log", json!({ "message": "green" }));
    b.link(measured, "then", shade, "run");
    b.link(green, "y", shade, "value");
    let bytes = b.bytes();

    let mut scene = Scene::new();
    let id = scene.create_object(None, "Scripted").unwrap();
    let crate_box = scene.create_object(None, "Box").unwrap();
    let script = ScriptRef { module: "keeper.bscript".into(), properties: BTreeMap::new() };
    let input = Arc::new(Input::default());
    let mut host = ScriptHost::new();
    host.load_bytes(id, 0, Path::new("keeper.bscript"), None, &bytes, &script, &mut scene, 0.0, &input).unwrap();
    host.update(&mut scene, 0.1, 0.1, &input);
    // A new object before the Box changes its handle, which the variable outlasts; then it goes.
    scene.create_object(None, "Newcomer").unwrap();
    scene.reparent(crate_box, None, Some(2)).unwrap();
    host.update(&mut scene, 0.1, 0.2, &input);
    scene.delete_object(crate_box).unwrap();
    host.update(&mut scene, 0.1, 0.3, &input);
    assert!(host.take_faults().is_empty());
    let lines: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    // 204 whole groups (1020 bytes), then "a" and one "é": 1023 bytes, cut before the next "é".
    let step = |there: &str| [format!("box {there}"), "note Hi".to_owned(), "long 614".to_owned(), "green 0.25".to_owned()];
    let expected: Vec<String> = [step("1"), step("1"), step("0")].concat();
    assert_eq!(lines, expected);
}

/// Whole numbers stay whole through math, divide without trapping, round as asked, and print every
/// digit; mixed with a number they take part as a number.
#[test]
fn whole_numbers_add_divide_round_and_print_exactly() {
    let mut b = Builder::new();
    let start = b.node("start", json!({}));
    let mut previous = (start, "then");
    let mut line = |b: &mut Builder, message: &str, value: (u32, &str)| {
        let log = b.node("log", json!({ "message": message }));
        b.link(previous.0, previous.1, log, "run");
        b.link(value.0, value.1, log, "value");
        previous = (log, "then");
    };
    let whole = |b: &mut Builder, n: i64| b.node("wholeNumber", json!({ "value": n }));
    let math = |b: &mut Builder, kind: &str, a: u32, c: u32| {
        let node = b.node(kind, json!({}));
        b.link(a, "result", node, "a");
        b.link(c, "result", node, "b");
        node
    };
    let (seven, two, zero, minus_seven) = (whole(&mut b, 7), whole(&mut b, 2), whole(&mut b, 0), whole(&mut b, -7));
    let sum = math(&mut b, "add", seven, two);
    line(&mut b, "sum", (sum, "result"));
    let quotient = math(&mut b, "divide", seven, two);
    line(&mut b, "quotient", (quotient, "result"));
    let by_zero = math(&mut b, "divide", seven, zero);
    line(&mut b, "by zero", (by_zero, "result"));
    let rest = math(&mut b, "remainder", minus_seven, two);
    line(&mut b, "remainder", (rest, "result"));
    let big = whole(&mut b, 123_456_789);
    line(&mut b, "big", (big, "result"));
    for (value, rounding) in [(2.5, "Nearest"), (-2.5, "Nearest"), (-1.5, "Down"), (1.2, "Up"), (-1.9, "Toward Zero")] {
        let made = b.node("toWhole", json!({ "value": value, "rounding": rounding }));
        line(&mut b, rounding, (made, "result"));
    }
    let half = b.node("number", json!({ "value": 0.5 }));
    let mixed = b.node("add", json!({}));
    b.link(seven, "result", mixed, "a");
    b.link(half, "value", mixed, "b");
    line(&mut b, "mixed", (mixed, "result"));

    let (_, _, mut host) = run(&b.bytes(), &[]);
    assert!(host.take_faults().is_empty());
    let lines: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(
        lines,
        ["sum 9", "quotient 3", "by zero 0", "remainder -1", "big 123456789", "Nearest 3", "Nearest -3", "Down -2", "Up 2", "Toward Zero -1", "mixed 7.5",]
    );
}
