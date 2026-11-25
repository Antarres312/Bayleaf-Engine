use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use bayleaf_core::StableId;
use bayleaf_graph::{Graph, Link, PinRef, Shape, VariableType};
use bayleaf_scene::Scene;
use bayleaf_scene::components::{DeclaredProperty, PropertyType, PropertyValue, ScriptRef, Transform};
use serde_json::json;

use super::{CATALOG, FUNCTION_KINDS, compile};
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

    /// A Property node naming `name`, declaring the property with `default` if it is not yet.
    fn property(&mut self, kind: &str, name: &str, default: serde_json::Value) -> u32 {
        let ty = super::property_type(kind).unwrap();
        if self.0.property(name).is_none() {
            self.0.add_property(name, ty, Some(default)).unwrap();
        }
        self.node(kind, json!({ "name": name }))
    }

    fn link(&mut self, from: u32, from_pin: &str, to: u32, to_pin: &str) {
        self.0.connect(&CATALOG, Link { from: PinRef::new(from, from_pin), to: PinRef::new(to, to_pin) }).unwrap();
    }

    /// Sets an input's typed value or a setting.
    fn set_value_for_tests(&mut self, node: u32, name: &str, value: serde_json::Value) {
        self.0.set_value(&CATALOG, node, name, value).unwrap();
    }

    fn bytes(&self) -> Vec<u8> {
        compile(&self.0).unwrap_or_else(|p| panic!("{p:?}"))
    }
}

/// A scene with one object and the host running `bytes` as its script.
fn run(bytes: &[u8], properties: &[(&str, f32)]) -> (Scene, StableId, ScriptHost) {
    let mut scene = Scene::new();
    let id = scene.create_object(None, "Scripted").unwrap();
    let script = ScriptRef {
        module: "test.bscript".into(),
        properties: properties.iter().map(|(k, v)| ((*k).to_owned(), PropertyValue::Number(f64::from(*v)))).collect::<BTreeMap<_, _>>(),
    };
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
    let speed = b.property("property", "speed", json!(90.0));
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
    let declared = ScriptHost::new().describe("spin.bscript", &bytes).unwrap();
    assert_eq!(
        declared,
        [DeclaredProperty {
            name: "speed".to_owned(),
            ty: PropertyType::Number,
            shape: bayleaf_scene::components::PropertyShape::Single,
            default: PropertyValue::Number(90.0)
        }]
    );
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
    let prop = b.property("property", "unused", json!(2.0));
    let mul = b.node("multiply", json!({}));
    b.link(number, "value", mul, "a");
    b.link(prop, "value", mul, "b");
    let sine = b.node("sine", json!({}));
    b.link(mul, "result", sine, "value");

    // Power, Square Root and Vector Length, added into what is logged.
    let power = b.node("power", json!({}));
    b.link(sine, "result", power, "base");
    let root = b.node("squareRoot", json!({}));
    b.link(power, "result", root, "value");
    let length = b.node("vectorLength", json!({}));
    b.link(make, "vector", length, "vector");
    let total = b.node("add", json!({}));
    b.link(root, "result", total, "a");
    b.link(length, "length", total, "b");
    // Lerp from the total to itself is the total, whatever Inverse Lerp gives as the factor.
    let unlerp = b.node("inverseLerp", json!({}));
    // Normalize feeds the factor, which the total does not depend on.
    let unit = b.node("normalize", json!({}));
    b.link(make, "vector", unit, "vector");
    let unit_length = b.node("vectorLength", json!({}));
    b.link(unit, "vector", unit_length, "vector");
    b.link(unit_length, "length", unlerp, "value");
    let lerp = b.node("lerp", json!({}));
    b.link(total, "result", lerp, "a");
    b.link(total, "result", lerp, "b");
    b.link(unlerp, "result", lerp, "factor");
    let log = b.node("log", json!({}));
    b.link(start, "then", log, "run");
    b.link(lerp, "result", log, "value");
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
    // Typed properties, each read once.
    let count = b.property("wholeProperty", "count", json!(3));
    let count_log = b.node("log", json!({ "message": "count" }));
    b.link(whole_log, "then", count_log, "run");
    b.link(count, "value", count_log, "value");
    let on = b.property("flagProperty", "on", json!(true));
    let on_gate = b.node("branch", json!({}));
    b.link(count_log, "then", on_gate, "run");
    b.link(on, "value", on_gate, "condition");
    let offset = b.property("vectorProperty", "offset", json!([1.0, 2.0, 3.0]));
    let offset_parts = b.node("split", json!({}));
    b.link(offset, "value", offset_parts, "vector");
    let offset_log = b.node("log", json!({ "message": "offset" }));
    b.link(on_gate, "true", offset_log, "run");
    b.link(offset_parts, "y", offset_log, "value");
    let tint = b.property("colorProperty", "tint", json!([0.5, 0.5, 1.0]));
    let keep_tint = b.node("setColorVariable", json!({}));
    b.link(offset_log, "then", keep_tint, "run");
    b.link(tint, "value", keep_tint, "value");
    let label = b.property("textProperty", "label", json!("hi"));
    let label_log = b.node("log", json!({ "message": "label" }));
    b.link(keep_tint, "then", label_log, "run");
    b.link(label, "value", label_log, "text");
    let other = b.property("objectProperty", "target", json!(null));
    let other_name = b.node("nameOf", json!({}));
    b.link(other, "value", other_name, "target");
    let other_gate = b.node("branch", json!({}));
    b.link(label_log, "then", other_gate, "run");
    b.link(other, "found", other_gate, "condition");
    let other_log = b.node("log", json!({ "message": "target" }));
    b.link(other_gate, "true", other_log, "run");
    b.link(other_name, "result", other_log, "text");
    // A function, called once.
    declare(&mut b, "helper", &[("x", VariableType::Number)], &[("y", VariableType::Number)]);
    let helper = function_node(&mut b, "functionStart", "helper");
    let helper_back = function_node(&mut b, "functionReturn", "helper");
    b.link(helper, "then", helper_back, "run");
    b.link(helper, "x", helper_back, "y");
    let helped = function_node(&mut b, "callFunction", "helper");
    b.link(other_log, "then", helped, "run");
    // Negate, on a number.
    let flipped = b.node("negate", json!({}));
    b.link(time, "seconds", flipped, "value");
    let flipped_log = b.node("log", json!({ "message": "flipped" }));
    b.link(helped, "then", flipped_log, "run");
    b.link(flipped, "result", flipped_log, "value");
    // Set Property, one of each type, on the properties declared above.
    let mut after = (flipped_log, "then");
    for (kind, name, value) in [
        ("setProperty", "unused", json!(4.0)),
        ("setWholeProperty", "count", json!(5)),
        ("setFlagProperty", "on", json!(true)),
        ("setVectorProperty", "offset", json!([1.0, 2.0, 3.0])),
        ("setColorProperty", "tint", json!([0.2, 0.3, 0.4])),
        ("setTextProperty", "label", json!("set")),
    ] {
        let set = b.node(kind, json!({ "name": name, "value": value }));
        b.link(after.0, after.1, set, "run");
        after = (set, "then");
    }
    let set_target = b.node("setObjectProperty", json!({ "name": "target" }));
    b.link(after.0, after.1, set_target, "run");
    b.link(me, "object", set_target, "value");
    // The interface: its events, and building and changing it (the object has none, so nothing shows).
    let pressed = b.node("interfacePressed", json!({ "element": "Start" }));
    let set_text = b.node("setInterfaceText", json!({ "element": "Title" }));
    b.link(pressed, "then", set_text, "run");
    b.link(other_name, "result", set_text, "text");
    let changed = b.node("interfaceChanged", json!({ "element": "Volume" }));
    let set_value = b.node("setInterfaceValue", json!({ "element": "Bar" }));
    b.link(changed, "then", set_value, "run");
    b.link(changed, "value", set_value, "value");
    let show = b.node("showInterface", json!({}));
    b.link(set_target, "then", show, "run");
    let add = b.node("addInterfaceElement", json!({ "into": "List", "what": "ui/slot.bui" }));
    b.link(show, "then", add, "run");
    let remove_element = b.node("removeInterfaceElement", json!({}));
    b.link(add, "then", remove_element, "run");
    // Reading the interface: a value and a text, logged.
    let read_value = b.node("interfaceValue", json!({ "element": "Volume" }));
    let read_text = b.node("interfaceText", json!({ "element": "Name" }));
    let read_log = b.node("log", json!({ "message": "read" }));
    b.link(remove_element, "then", read_log, "run");
    b.link(read_value, "value", read_log, "value");
    b.link(read_text, "text", read_log, "text");
    // Particles: started, burst and stopped (the object has no emitter, so nothing shows).
    let play = b.node("playParticles", json!({}));
    b.link(read_log, "then", play, "run");
    let burst = b.node("burstParticles", json!({ "count": 5 }));
    b.link(play, "then", burst, "run");
    let stop = b.node("stopParticles", json!({}));
    b.link(burst, "then", stop, "run");
    let found_gate = b.node("branch", json!({}));
    b.link(stop, "then", found_gate, "run");
    b.link(read_value, "found", found_gate, "condition");
    // An event this script declares: triggered on every object, and heard here.
    b.0.add_event("Hit").unwrap();
    let hit = vec![bayleaf_graph::Param::new("power".to_owned(), VariableType::Number)];
    b.0.set_event(&CATALOG, "Hit", None, Some(hit.clone()), &super::EVENT_KINDS).unwrap();
    let hit_pins = bayleaf_graph::param_pins(&hit);
    let trigger_hit = b.node("triggerEvent", json!({ "event": "Hit", "params": hit_pins }));
    b.link(found_gate, "true", trigger_hit, "run");
    let on_hit = b.node("eventStart", json!({ "event": "Hit", "params": hit_pins }));
    let heard = b.node("log", json!({ "message": "hit" }));
    b.link(on_hit, "then", heard, "run");
    b.link(on_hit, "power", heard, "value");
    // This script as a component: subscribed to, triggered on, and heard; its object named.
    let me_component = b.node("thisComponent", json!({}));
    let my_object = b.node("componentObject", json!({}));
    b.link(me_component, "component", my_object, "component");
    let named = b.node("nameOf", json!({}));
    b.link(my_object, "object", named, "target");
    let subscribe = b.node("subscribeEvent", json!({ "script": "Test", "event": "Hit", "params": hit_pins }));
    b.link(trigger_hit, "then", subscribe, "run");
    b.link(me_component, "component", subscribe, "component");
    let on_me = b.node("triggerComponentEvent", json!({ "script": "Test", "event": "Hit", "params": hit_pins }));
    b.link(subscribe, "then", on_me, "run");
    b.link(me_component, "component", on_me, "component");
    let unsubscribe = b.node("unsubscribeEvent", json!({ "script": "Test", "event": "Hit", "params": hit_pins }));
    b.link(on_me, "then", unsubscribe, "run");
    b.link(me_component, "component", unsubscribe, "component");
    let heard_from = b.node("componentEventStart", json!({ "script": "Test", "event": "Hit", "params": hit_pins }));
    let from = b.node("componentObject", json!({}));
    b.link(heard_from, "source", from, "component");
    let heard_log = b.node("log", json!({ "message": "" }));
    b.link(heard_from, "then", heard_log, "run");
    b.link(named, "result", heard_log, "text");
    // A list and a map the script declares: changed, read, and gone through.
    shaped_variable(&mut b, "Seen", VariableType::Vector, Shape::List, None);
    shaped_property(&mut b, "Names", VariableType::Object, Shape::Map, None);
    let list = json!({ "name": "Seen", "item": ["vector"] });
    let map = json!({ "name": "Names", "item": ["object"] });
    let mut previous = unsubscribe;
    for kind in ["listAdd", "listInsert", "listSet", "listRemove", "listForEach", "listClear", "mapSet", "mapForEach", "mapRemove", "mapClear"] {
        let action = b.node(kind, if kind.starts_with("list") { list.clone() } else { map.clone() });
        b.link(previous, "then", action, "run");
        previous = action;
    }
    let me_too = b.node("thisObject", json!({}));
    let set_name = b.0.nodes.iter().find(|n| n.kind == "mapSet").map(|n| n.id).unwrap();
    b.link(me_too, "object", set_name, "value");
    let item = b.node("listItem", list.clone());
    let item_at = b.node("split", json!({}));
    b.link(item, "value", item_at, "vector");
    let value = b.node("mapGet", map.clone());
    let named_too = b.node("nameOf", json!({}));
    b.link(value, "value", named_too, "target");
    for (kind, settings, pin, text) in
        [("listLength", list.clone(), "length", Some((named_too, "result"))), ("listFind", list, "index", None), ("mapCount", map, "count", None)]
    {
        let read = b.node(kind, settings);
        let said = b.node("log", json!({ "message": kind }));
        b.link(previous, "then", said, "run");
        b.link(read, pin, said, "value");
        if let Some((node, pin)) = text {
            b.link(node, pin, said, "text");
        }
        previous = said;
    }
    let gate = b.node("branch", json!({}));
    b.link(previous, "then", gate, "run");
    let either = b.node("or", json!({}));
    b.link(item, "found", either, "a");
    b.link(value, "found", either, "b");
    b.link(either, "result", gate, "condition");
    let at = b.node("log", json!({ "message": "at" }));
    b.link(gate, "true", at, "run");
    b.link(item_at, "x", at, "value");
    // The list and the map passed on through pins: Get List into List Length, Get Map into Map Count.
    let whole_list = b.node("getList", json!({ "name": "Seen", "item": ["vector"] }));
    let whole_map = b.node("getMap", json!({ "name": "Names", "item": ["object"] }));
    let counted = b.node("listLength", json!({ "item": ["vector"] }));
    b.link(whole_list, "list", counted, "list");
    let keys = b.node("mapCount", json!({ "item": ["object"] }));
    b.link(whole_map, "map", keys, "map");
    let sizes = b.node("log", json!({ "message": "sizes" }));
    b.link(at, "then", sizes, "run");
    b.link(counted, "length", sizes, "value");
    let more = b.node("log", json!({ "message": "keys" }));
    b.link(sizes, "then", more, "run");
    b.link(keys, "count", more, "value");
    // Physics contacts: each names the other object (none arrive here).
    for kind in ["triggerEntered", "triggerLeft", "collisionStarted", "collisionEnded"] {
        let contact = b.node(kind, json!({}));
        let other = b.node("nameOf", json!({}));
        b.link(contact, "other", other, "target");
        let said = b.node("log", json!({ "message": "" }));
        b.link(contact, "then", said, "run");
        b.link(other, "result", said, "text");
    }
    for kind in CATALOG.nodes.iter().map(|d| d.kind) {
        assert!(b.0.nodes.iter().any(|n| n.kind == kind), "{kind} is not in the test graph");
    }
    let (mut scene, _, mut host) = run(&b.bytes(), &[]);
    host.update(&mut scene, 0.1, 1.0, &Arc::new(Input::default()));
    assert!(host.take_faults().is_empty());
    // Sine of 0 is 0, so its power and root are 0; the made vector (0, 0.5, 0) is 0.5 long.
    assert_eq!(host.take_logs()[0].text, "Hello 0.5");
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
    // Add gives a vector here, so linking it into a number input is refused.
    let into_log = Link { from: PinRef::new(add, "result"), to: PinRef::new(log, "value") };
    let refused = b.0.connect(&CATALOG, into_log.clone()).unwrap_err().to_string();
    assert!(refused.contains("Vector"), "{refused}");
    // A file saved with such a link still opens, and the compiler names the node to mend.
    b.0.links.push(into_log);
    let read = Graph::from_json(&b.0.to_json(), &CATALOG).expect("it opens");
    let problems = compile(&read).unwrap_err();
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
fn a_raycast_finds_a_child_where_its_parent_was_moved_between_steps() {
    let mut b = Builder::new();
    let update = b.node("update", json!({}));
    let ray = b.node("raycast", json!({ "direction": [1.0, 0.0, 0.0], "distance": 50.0 }));
    let log = b.node("log", json!({ "message": "" }));
    b.link(update, "then", log, "run");
    b.link(ray, "distance", log, "value");
    let (mut scene, _, mut host) = run(&b.bytes(), &[]);
    // A cube 5 ahead of its parent, the parent first at the origin.
    let parent = scene.create_object(None, "Holder").unwrap();
    let cube = scene.create_object(Some(parent), "Cube").unwrap();
    scene.add_component(cube, "mesh").unwrap();
    scene.set_component(cube, "mesh", json!({"shape": "cube", "color": [1, 1, 1, 1], "metallic": 0, "roughness": 0.5})).unwrap();
    scene.set(cube, Transform { position: [5.0, 0.0, 0.0], rotation: [0.0; 3], scale: [1.0; 3] }).unwrap();
    let input = Arc::new(Input::default());
    let distance = |host: &mut ScriptHost, scene: &mut Scene| -> f32 {
        host.update(scene, 0.1, 0.0, &input);
        host.take_logs()[0].text.parse().unwrap()
    };
    let before = distance(&mut host, &mut scene);
    // Only the parent moves, outside the scripts: the cube goes with it.
    scene.set(parent, Transform { position: [10.0, 0.0, 0.0], rotation: [0.0; 3], scale: [1.0; 3] }).unwrap();
    let after = distance(&mut host, &mut scene);
    assert!(after > before + 5.0, "the ray meets the cube farther once its parent moved: {before} then {after}");
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
    let speed = bob.property("property", "speed", json!(2.0));
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
    health.property("property", "maxHealth", json!(100.0));
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
    // To Text also writes vectors and whole numbers.
    let point = b.node("vector", json!({ "x": 1.5, "y": -2.0, "z": 0.25 }));
    let spelled_point = b.node("numberToText", json!({}));
    b.link(point, "vector", spelled_point, "value");
    line(&mut b, "vector", Some((spelled_point, "result")), None);
    let big = b.node("wholeNumber", json!({ "value": 123456789 }));
    let spelled_big = b.node("numberToText", json!({}));
    b.link(big, "result", spelled_big, "value");
    line(&mut b, "whole", Some((spelled_big, "result")), None);

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
            "vector (1.5, -2, 0.25)",
            "whole 123456789",
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

#[test]
fn typed_properties_read_their_defaults_and_the_values_an_object_sets() {
    let mut b = Builder::new();
    let start = b.node("start", json!({}));
    let mut previous = (start, "then");
    let mut log = |b: &mut Builder, message: &str, pin: &str, from: (u32, &str)| {
        let node = b.node("log", json!({ "message": message }));
        b.link(previous.0, previous.1, node, "run");
        b.link(from.0, from.1, node, pin);
        previous = (node, "then");
    };
    let count = b.property("wholeProperty", "count", json!(3));
    log(&mut b, "count", "value", (count, "value"));
    let label = b.property("textProperty", "label", json!("hi"));
    log(&mut b, "label", "text", (label, "value"));
    let on = b.property("flagProperty", "on", json!(false));
    let on_number = b.node("choose", json!({ "a": 1.0, "b": 0.0 }));
    b.link(on, "value", on_number, "condition");
    log(&mut b, "on", "value", (on_number, "result"));
    let offset = b.property("vectorProperty", "offset", json!([1.0, 2.0, 3.0]));
    let parts = b.node("split", json!({}));
    b.link(offset, "value", parts, "vector");
    log(&mut b, "offset", "value", (parts, "z"));
    let target = b.property("objectProperty", "target", json!(null));
    let target_name = b.node("nameOf", json!({}));
    b.link(target, "value", target_name, "target");
    log(&mut b, "target", "text", (target_name, "result"));
    let bytes = b.bytes();

    // What the script declares, with types and defaults.
    let declared = ScriptHost::new().describe("typed.bscript", &bytes).unwrap();
    let types: Vec<(&str, PropertyType)> = declared.iter().map(|p| (p.name.as_str(), p.ty)).collect();
    assert_eq!(
        types,
        [
            ("count", PropertyType::WholeNumber),
            ("label", PropertyType::Text),
            ("on", PropertyType::TrueFalse),
            ("offset", PropertyType::Vector),
            ("target", PropertyType::Object)
        ]
    );

    let lines = |properties: BTreeMap<String, PropertyValue>| -> Vec<String> {
        let mut scene = Scene::new();
        let id = scene.create_object(None, "Scripted").unwrap();
        scene.create_object(None, "Box").unwrap();
        let script = ScriptRef { module: "typed.bscript".into(), properties };
        let mut host = ScriptHost::new();
        let input = Arc::new(Input::default());
        host.load_bytes(id, 0, Path::new("typed.bscript"), None, &bytes, &script, &mut scene, 0.0, &input).unwrap();
        host.update(&mut scene, 0.1, 0.1, &input);
        assert!(host.take_faults().is_empty());
        host.take_logs().into_iter().map(|l| l.text).collect()
    };
    assert_eq!(lines(BTreeMap::new()), ["count 3", "label hi", "on 0", "offset 3", "target "], "the defaults");
    let set = BTreeMap::from([
        ("count".to_owned(), PropertyValue::Number(7.0)),
        ("label".to_owned(), PropertyValue::Text("hello".to_owned())),
        ("on".to_owned(), PropertyValue::Flag(true)),
        ("offset".to_owned(), PropertyValue::Triple([0.0, 0.0, 9.0])),
        ("target".to_owned(), PropertyValue::Text("Box".to_owned())),
    ]);
    assert_eq!(lines(set), ["count 7", "label hello", "on 1", "offset 9", "target Box"], "the object's values");
    // A value of the wrong type is ignored for the default.
    let wrong = BTreeMap::from([("count".to_owned(), PropertyValue::Text("seven".to_owned()))]);
    assert_eq!(lines(wrong)[0], "count 3");
}

#[test]
fn another_script_s_whole_number_property_is_read_and_set_as_a_whole_number() {
    let mut counter = Builder::new();
    let update = counter.node("update", json!({}));
    let count = counter.property("wholeProperty", "count", json!(2));
    let log = counter.node("log", json!({ "message": "count" }));
    counter.link(update, "then", log, "run");
    counter.link(count, "value", log, "value");
    // The setter sets Counter's count on Target to 7.6 made whole: 8.
    let mut setter = Builder::new();
    let update = setter.node("update", json!({}));
    let target = setter.node("findNamed", json!({ "name": "Target" }));
    let value = setter.node("toWhole", json!({ "value": 7.6, "rounding": "Nearest" }));
    let write = setter.node("setComponent", json!({ "component": "Counter", "fields": ["count:whole"] }));
    setter.link(update, "then", write, "run");
    setter.link(target, "object", write, "target");
    setter.link(value, "result", write, "count");
    let read = setter.node("getComponent", json!({ "component": "Counter", "fields": ["count:whole"] }));
    setter.link(target, "object", read, "target");
    let said = setter.node("log", json!({ "message": "read" }));
    setter.link(write, "then", said, "run");
    setter.link(read, "count", said, "value");

    let mut scene = Scene::new();
    let target_id = scene.create_object(None, "Target").unwrap();
    let setter_id = scene.create_object(None, "Setter").unwrap();
    let mut host = ScriptHost::new();
    let input = Arc::new(Input::default());
    let plain = |module: &str| ScriptRef { module: module.into(), properties: BTreeMap::new() };
    host.load_bytes(target_id, 0, Path::new("counter.bscript"), None, &counter.bytes(), &plain("counter.bscript"), &mut scene, 0.0, &input).unwrap();
    host.load_bytes(setter_id, 0, Path::new("setter.bscript"), None, &setter.bytes(), &plain("setter.bscript"), &mut scene, 0.0, &input).unwrap();
    host.update(&mut scene, 0.1, 0.1, &input);
    host.update(&mut scene, 0.1, 0.2, &input);
    assert!(host.take_faults().is_empty());
    let lines: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(lines, ["count 2", "read 2", "count 8", "read 8"]);
}

/// Declares a function on the builder's graph with typed inputs and outputs.
fn declare(b: &mut Builder, name: &str, inputs: &[(&str, VariableType)], outputs: &[(&str, VariableType)]) {
    let params = |list: &[(&str, VariableType)]| list.iter().map(|(n, ty)| bayleaf_graph::Param::new((*n).to_owned(), *ty)).collect::<Vec<_>>();
    b.0.add_function(name).unwrap();
    b.0.set_function(&CATALOG, name, None, Some(params(inputs)), Some(params(outputs)), &FUNCTION_KINDS).unwrap();
}

/// A node of a function: Function, Return or Call Function, with the function's pins.
fn function_node(b: &mut Builder, kind: &str, function: &str) -> u32 {
    let f = b.0.function(function).cloned().unwrap();
    b.node(kind, json!({ "inputs": bayleaf_graph::param_pins(&f.inputs), "outputs": bayleaf_graph::param_pins(&f.outputs), "function": function }))
}

#[test]
fn functions_take_inputs_give_back_outputs_and_may_call_themselves() {
    let mut b = Builder::new();
    // double(x, label) gives back y = 2x and said = label + "!".
    declare(&mut b, "double", &[("x", VariableType::Number), ("label", VariableType::Text)], &[("y", VariableType::Number), ("said", VariableType::Text)]);
    let begin = function_node(&mut b, "functionStart", "double");
    let twice = b.node("multiply", json!({ "b": 2.0 }));
    b.link(begin, "x", twice, "a");
    let bang = b.node("joinText", json!({ "b": "!" }));
    b.link(begin, "label", bang, "a");
    let give = function_node(&mut b, "functionReturn", "double");
    b.link(begin, "then", give, "run");
    b.link(twice, "result", give, "y");
    b.link(bang, "result", give, "said");
    // fact(n) = n > 1 ? n * fact(n - 1) : 1.
    declare(&mut b, "fact", &[("n", VariableType::Number)], &[("r", VariableType::Number)]);
    let fact = function_node(&mut b, "functionStart", "fact");
    let more = b.node("greater", json!({ "b": 1.0 }));
    b.link(fact, "n", more, "a");
    let gate = b.node("branch", json!({}));
    b.link(fact, "then", gate, "run");
    b.link(more, "result", gate, "condition");
    let less = b.node("subtract", json!({ "b": 1.0 }));
    b.link(fact, "n", less, "a");
    let again = function_node(&mut b, "callFunction", "fact");
    b.link(gate, "true", again, "run");
    b.link(less, "result", again, "n");
    let product = b.node("multiply", json!({}));
    b.link(fact, "n", product, "a");
    b.link(again, "r", product, "b");
    let give_product = function_node(&mut b, "functionReturn", "fact");
    b.link(again, "then", give_product, "run");
    b.link(product, "result", give_product, "r");
    let give_one = function_node(&mut b, "functionReturn", "fact");
    b.link(gate, "false", give_one, "run");
    b.set_value_for_tests(give_one, "r", json!(1.0));
    // On Start calls both and logs what they gave back.
    let start = b.node("start", json!({}));
    let call = function_node(&mut b, "callFunction", "double");
    b.set_value_for_tests(call, "x", json!(21.0));
    b.set_value_for_tests(call, "label", json!("hi"));
    b.link(start, "then", call, "run");
    let said = b.node("log", json!({ "message": "double" }));
    b.link(call, "then", said, "run");
    b.link(call, "said", said, "text");
    b.link(call, "y", said, "value");
    let five = function_node(&mut b, "callFunction", "fact");
    b.set_value_for_tests(five, "n", json!(5.0));
    b.link(said, "then", five, "run");
    let told = b.node("log", json!({ "message": "fact" }));
    b.link(five, "then", told, "run");
    b.link(five, "r", told, "value");

    let (_, _, mut host) = run(&b.bytes(), &[]);
    assert!(host.take_faults().is_empty());
    let lines: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(lines, ["double hi! 42", "fact 120"]);
}

#[test]
fn a_function_s_values_and_return_belong_to_its_own_actions() {
    let mut b = Builder::new();
    declare(&mut b, "f", &[("x", VariableType::Number)], &[("y", VariableType::Number)]);
    let begin = function_node(&mut b, "functionStart", "f");
    let start = b.node("start", json!({}));
    let log = b.node("log", json!({}));
    b.link(start, "then", log, "run");
    b.link(begin, "x", log, "value");
    let problems = compile(&b.0).unwrap_err();
    assert!(problems.iter().any(|p| p.node == Some(begin) && p.message.contains("read only on f's own actions")), "{problems:?}");

    let mut b = Builder::new();
    declare(&mut b, "f", &[], &[("y", VariableType::Number)]);
    let start = b.node("start", json!({}));
    let give = function_node(&mut b, "functionReturn", "f");
    b.link(start, "then", give, "run");
    let problems = compile(&b.0).unwrap_err();
    assert!(problems.iter().any(|p| p.node == Some(give)), "{problems:?}");

    let mut b = Builder::new();
    let start = b.node("start", json!({}));
    let call = b.node("callFunction", json!({ "function": "missing" }));
    b.link(start, "then", call, "run");
    let problems = compile(&b.0).unwrap_err();
    assert!(problems.iter().any(|p| p.node == Some(call) && p.message.contains("no function missing")), "{problems:?}");
}

#[test]
fn negate_turns_the_sign_of_numbers_whole_numbers_and_vectors() {
    let mut b = Builder::new();
    let start = b.node("start", json!({}));
    let number = b.node("negate", json!({ "value": 5.0 }));
    let whole = b.node("negate", json!({}));
    let minus_seven = b.node("wholeNumber", json!({ "value": -7 }));
    b.link(minus_seven, "result", whole, "value");
    let vector = b.node("negate", json!({}));
    let made = b.node("vector", json!({ "x": 1.0, "y": -2.0, "z": 3.0 }));
    b.link(made, "vector", vector, "value");
    let parts = b.node("split", json!({}));
    b.link(vector, "result", parts, "vector");
    let mut previous = (start, "then");
    for (message, from, pin) in [("number", number, "result"), ("whole", whole, "result"), ("y", parts, "y")] {
        let log = b.node("log", json!({ "message": message }));
        b.link(previous.0, previous.1, log, "run");
        b.link(from, pin, log, "value");
        previous = (log, "then");
    }
    let (_, _, mut host) = run(&b.bytes(), &[]);
    let lines: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(lines, ["number -5", "whole 7", "y 2"]);
}

#[test]
fn set_property_changes_this_object_s_property_for_the_rest_of_play() {
    let mut b = Builder::new();
    let start = b.node("start", json!({}));
    let set_count = b.node("setWholeProperty", json!({ "name": "count", "value": 9 }));
    b.link(start, "then", set_count, "run");
    let set_label = b.node("setTextProperty", json!({ "name": "label", "value": "changed" }));
    b.link(set_count, "then", set_label, "run");
    let count = b.property("wholeProperty", "count", json!(3));
    let label = b.property("textProperty", "label", json!("hi"));
    let log = b.node("log", json!({ "message": "now" }));
    b.link(set_label, "then", log, "run");
    b.link(label, "value", log, "text");
    b.link(count, "value", log, "value");
    // A Set Property of the wrong type for the declared property is a problem on that node.
    let wrong = b.node("setProperty", json!({ "name": "label" }));
    b.link(log, "then", wrong, "run");
    let problems = compile(&b.0).unwrap_err();
    assert!(problems.iter().any(|p| p.node == Some(wrong) && p.message.contains("Text property")), "{problems:?}");
    b.0.remove_node(wrong).unwrap();

    let (_, _, mut host) = run(&b.bytes(), &[]);
    assert!(host.take_faults().is_empty());
    let lines: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(lines, ["now changed 9"]);
}

#[test]
fn property_defaults_kept_on_nodes_in_older_scripts_move_into_declared_properties() {
    let old = r#"{
      "format": "bayleaf.graph", "version": 4, "domain": "script",
      "nodes": [
        { "id": 1, "kind": "property", "position": [0, 0], "values": { "name": "speed", "default": 45.0 } },
        { "id": 2, "kind": "flagProperty", "position": [0, 60], "values": { "name": "on", "default": "True" } },
        { "id": 3, "kind": "textProperty", "position": [0, 120] }
      ]
    }"#;
    let graph = Graph::from_json(old, &CATALOG).unwrap();
    let declared: Vec<(&str, VariableType, &serde_json::Value)> = graph.properties.iter().map(|p| (p.name.as_str(), p.ty, &p.default)).collect();
    assert_eq!(
        declared,
        [("speed", VariableType::Number, &json!(45.0)), ("on", VariableType::TrueFalse, &json!(true)), ("label", VariableType::Text, &json!(""))]
    );
    assert!(graph.nodes.iter().all(|n| !n.values.contains_key("default")), "the nodes keep only the name");
    assert_eq!(graph.value(3, "name"), Some(&json!("label")), "a node without a name keeps the name it had");
}

#[test]
fn a_visual_scripts_inspector_options_reach_its_description() {
    use bayleaf_graph::PropertyOptions;
    use bayleaf_scene::inspector::InspectorItem;
    let mut b = spinner();
    let options = PropertyOptions { label: "Turn rate".into(), group: "Motion".into(), range: Some([0.0, 360.0]), slider: true, ..PropertyOptions::default() };
    b.0.set_property_options("speed", options).unwrap();
    super::check_inspector(&b.0).unwrap();
    let described = ScriptHost::new().describe_all("spin.bscript", &b.bytes()).unwrap();
    let [InspectorItem::Group { title, items, .. }] = described.inspector.as_slice() else { panic!("one group: {:?}", described.inspector) };
    assert_eq!(title, "Motion");
    let [InspectorItem::Property(speed)] = items.as_slice() else { panic!("{items:?}") };
    assert_eq!((speed.label.as_deref(), speed.range, speed.slider), (Some("Turn rate"), Some((0.0, 360.0)), true));

    // Options the host would refuse are found before the script is saved.
    b.0.set_property_options("speed", PropertyOptions { choices: vec!["a".into()], ..PropertyOptions::default() }).unwrap();
    assert!(super::check_inspector(&b.0).unwrap_err().contains("choices"));
}

#[test]
fn lerp_goes_between_and_past_its_ends_and_inverse_lerp_undoes_it() {
    use bayleaf_graph::PinType;
    let mut b = Builder::new();
    let start = b.node("start", json!({}));
    let quarter = b.node("lerp", json!({ "a": 2.0, "b": 6.0, "factor": 0.25 }));
    let past = b.node("lerp", json!({ "a": 2.0, "b": 6.0, "factor": 1.5 }));
    let (two, three) = (b.node("wholeNumber", json!({ "value": 2 })), b.node("wholeNumber", json!({ "value": 3 })));
    let whole = b.node("lerp", json!({}));
    b.link(two, "result", whole, "a");
    b.link(three, "result", whole, "b");
    let position = b.node("vector", json!({ "x": 4.0, "y": 8.0 }));
    let moved = b.node("lerp", json!({ "factor": 0.5 }));
    b.link(position, "vector", moved, "b");
    let parts = b.node("split", json!({}));
    b.link(moved, "result", parts, "vector");
    let back = b.node("inverseLerp", json!({ "a": 2.0, "b": 6.0 }));
    b.link(quarter, "result", back, "value");
    let flat = b.node("inverseLerp", json!({ "a": 3.0, "b": 3.0, "value": 7.0 }));
    let mut previous = (start, "then");
    for (message, node, pin) in [
        ("quarter", quarter, "result"),
        ("past", past, "result"),
        ("whole", whole, "result"),
        ("y", parts, "y"),
        ("back", back, "result"),
        ("flat", flat, "result"),
    ] {
        let log = b.node("log", json!({ "message": message }));
        b.link(previous.0, previous.1, log, "run");
        b.link(node, pin, log, "value");
        previous = (log, "then");
    }
    let ty = |b: &Builder, node: u32| b.0.output_type(&CATALOG, &PinRef::new(node, "result"));
    assert_eq!(ty(&b, whole), PinType::Float, "between two whole numbers lies a fraction");
    assert_eq!(ty(&b, moved), PinType::Vec3, "between vectors lies a vector");

    let (mut scene, _, mut host) = run(&b.bytes(), &[]);
    host.update(&mut scene, 0.1, 0.0, &Arc::new(Input::default()));
    let logs: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(logs, ["quarter 3", "past 8", "whole 2.5", "y 4", "back 0.25", "flat 0"]);
}

#[test]
fn power_root_length_and_normalize_give_their_values_and_math_outputs_take_their_inputs_type() {
    use bayleaf_graph::PinType;
    let mut b = Builder::new();
    let start = b.node("start", json!({}));
    let whole = |b: &mut Builder, v: i64| b.node("wholeNumber", json!({ "value": v }));
    let (three, four) = (whole(&mut b, 3), whole(&mut b, 4));
    let (minus_two, minus_three) = (whole(&mut b, -2), whole(&mut b, -3));
    let power = |b: &mut Builder, base: u32, exponent: u32| {
        let p = b.node("power", json!({}));
        b.link(base, "result", p, "base");
        b.link(exponent, "result", p, "exponent");
        p
    };
    let whole_power = power(&mut b, three, four);
    let negative_power = power(&mut b, minus_two, minus_three);
    let half_power = b.node("power", json!({ "base": 2.0, "exponent": 0.5 }));
    let root = b.node("squareRoot", json!({ "value": 16.0 }));
    let below_zero = b.node("squareRoot", json!({ "value": -4.0 }));
    let length = b.node("vectorLength", json!({ "vector": [3.0, 4.0, 0.0] }));
    // Normalize keeps the direction at length 1, and leaves a vector of no length at 0 (not NaN).
    let unit = b.node("normalize", json!({ "vector": [3.0, 4.0, 0.0] }));
    let unit_x = b.node("split", json!({}));
    b.link(unit, "vector", unit_x, "vector");
    let zero = b.node("normalize", json!({}));
    let zero_length = b.node("vectorLength", json!({}));
    b.link(zero, "vector", zero_length, "vector");
    let mut previous = (start, "then");
    for (message, node, pin) in [
        ("whole", whole_power, "result"),
        ("negative", negative_power, "result"),
        ("half", half_power, "result"),
        ("root", root, "result"),
        ("below", below_zero, "result"),
        ("length", length, "length"),
        ("unit", unit_x, "x"),
        ("still", zero_length, "length"),
    ] {
        let log = b.node("log", json!({ "message": message }));
        b.link(previous.0, previous.1, log, "run");
        b.link(node, pin, log, "value");
        previous = (log, "then");
    }

    // What each output gives, from its inputs.
    let ty = |b: &Builder, node: u32| b.0.output_type(&CATALOG, &PinRef::new(node, "result"));
    assert_eq!(ty(&b, whole_power), PinType::Int, "whole to a whole power stays whole");
    assert_eq!(ty(&b, half_power), PinType::Float);
    assert_eq!(ty(&b, root), PinType::Float, "a root is a number even of a whole number");
    let sum = b.node("add", json!({}));
    b.link(three, "result", sum, "a");
    b.link(four, "result", sum, "b");
    assert_eq!(ty(&b, sum), PinType::Int);
    let position = b.node("position", json!({}));
    let moved = b.node("add", json!({}));
    b.link(position, "position", moved, "a");
    assert_eq!(ty(&b, moved), PinType::Vec3, "a vector in makes a vector out");
    // A whole sum feeding a whole number input cannot then take a vector.
    let remainder = b.node("remainder", json!({}));
    b.link(sum, "result", remainder, "a");
    let vector = b.0.connect(&CATALOG, Link { from: PinRef::new(position, "position"), to: PinRef::new(sum, "a") });
    assert!(vector.is_err(), "the remainder would get a vector");

    let (mut scene, _, mut host) = run(&b.bytes(), &[]);
    host.update(&mut scene, 0.1, 0.0, &Arc::new(Input::default()));
    let logs: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(logs, ["whole 81", "negative 0", "half 1.414", "root 4", "below 0", "length 5", "unit 0.6", "still 0"]);
}

#[test]
fn contact_events_run_for_their_own_contact_with_the_other_object() {
    let mut b = Builder::new();
    let entered = b.node("triggerEntered", json!({}));
    let name = b.node("nameOf", json!({}));
    b.link(entered, "other", name, "target");
    let log = b.node("log", json!({ "message": "" }));
    b.link(entered, "then", log, "run");
    b.link(name, "result", log, "text");
    for (kind, message) in [("triggerLeft", "left"), ("collisionStarted", "hit"), ("collisionEnded", "parted")] {
        let event = b.node(kind, json!({}));
        let log = b.node("log", json!({ "message": message }));
        b.link(event, "then", log, "run");
    }
    let (mut scene, id, mut host) = run(&b.bytes(), &[]);
    let coin = scene.create_object(None, "Coin").unwrap();
    let input = Arc::new(Input::default());
    for contact in ["triggerEnter", "collision", "triggerExit", "collisionEnd"] {
        host.notify(&mut scene, &[(id, contact, coin)], 0.0, &input);
    }
    let logs: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(logs, ["Coin", "hit", "left", "parted"], "each event once, for its own contact, naming the other object");
}

#[test]
fn an_event_triggered_with_its_values_runs_on_event_which_reads_each_of_them() {
    use bayleaf_graph::Param;
    let mut b = Builder::new();
    b.0.add_event("Damaged").unwrap();
    let declared: Vec<Param> = [
        ("amount", VariableType::Number),
        ("by", VariableType::Object),
        ("note", VariableType::Text),
        ("at", VariableType::Vector),
        ("crit", VariableType::TrueFalse),
        ("count", VariableType::WholeNumber),
    ]
    .into_iter()
    .map(|(name, ty)| Param::new(name, ty))
    .collect();
    b.0.set_event(&CATALOG, "Damaged", None, Some(declared.clone()), &super::EVENT_KINDS).unwrap();
    let pins = bayleaf_graph::param_pins(&declared);
    // On start the object triggers the event on itself, with a value of each kind.
    let start = b.node("start", json!({}));
    let trigger = b.node("triggerEvent", json!({ "event": "Damaged", "params": pins }));
    for (name, value) in [("amount", json!(2.5)), ("note", json!("ouch")), ("at", json!([1.0, 2.0, 3.0])), ("crit", json!(true)), ("count", json!(7))] {
        b.0.set_value(&CATALOG, trigger, name, value).unwrap();
    }
    b.link(start, "then", trigger, "run");
    let me = b.node("thisObject", json!({}));
    b.link(me, "object", trigger, "target");
    b.link(me, "object", trigger, "by");
    // On Event logs what it brought.
    let on = b.node("eventStart", json!({ "event": "Damaged", "params": pins }));
    let by = b.node("nameOf", json!({}));
    b.link(on, "by", by, "target");
    let at = b.node("split", json!({}));
    b.link(on, "at", at, "vector");
    let mut previous = (on, "then");
    for (message, value, text) in [
        ("amount", Some((on, "amount")), None),
        ("by", None, Some((by, "result"))),
        ("note", None, Some((on, "note"))),
        ("at", Some((at, "y")), None),
        ("count", Some((on, "count")), None),
    ] {
        let log = b.node("log", json!({ "message": message }));
        b.link(previous.0, previous.1, log, "run");
        if let Some((node, pin)) = value {
            b.link(node, pin, log, "value");
        }
        if let Some((node, pin)) = text {
            b.link(node, pin, log, "text");
        }
        previous = (log, "then");
    }
    let crit = b.node("branch", json!({}));
    b.link(previous.0, previous.1, crit, "run");
    b.link(on, "crit", crit, "condition");
    let critical = b.node("log", json!({ "message": "critical" }));
    b.link(crit, "true", critical, "run");

    let (mut scene, _, mut host) = run(&b.bytes(), &[]);
    host.update(&mut scene, 0.1, 0.0, &Arc::new(Input::default()));
    let logs: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(logs, ["amount 2.5", "by Scripted", "note ouch", "at 2", "count 7", "critical"], "each value arrives as it was sent");
    // The script says which events it declares, so the editor can offer them.
    let declared = ScriptHost::new().describe_all("events.bscript", &b.bytes()).unwrap().events;
    assert_eq!(declared.len(), 1);
    assert_eq!((declared[0].name.as_str(), declared[0].params.len()), ("Damaged", 6));
}

#[test]
fn an_on_events_values_are_read_only_on_its_own_actions() {
    let mut b = Builder::new();
    b.0.add_event("Ping").unwrap();
    let declared = vec![bayleaf_graph::Param::new("n".to_owned(), VariableType::Number)];
    b.0.set_event(&CATALOG, "Ping", None, Some(declared.clone()), &super::EVENT_KINDS).unwrap();
    let on = b.node("eventStart", json!({ "event": "Ping", "params": bayleaf_graph::param_pins(&declared) }));
    let update = b.node("update", json!({}));
    let log = b.node("log", json!({}));
    b.link(update, "then", log, "run");
    b.link(on, "n", log, "value");
    let problems = compile(&b.0).unwrap_err();
    assert!(problems.iter().any(|p| p.message.contains("read only on its own actions")), "{problems:?}");
}

#[test]
fn a_plain_message_such_as_an_interface_click_runs_on_event_with_its_values_at_zero() {
    let mut b = Builder::new();
    b.0.add_event("OpenDoor").unwrap();
    let declared = vec![bayleaf_graph::Param::new("speed".to_owned(), VariableType::Number)];
    b.0.set_event(&CATALOG, "OpenDoor", None, Some(declared.clone()), &super::EVENT_KINDS).unwrap();
    let on = b.node("eventStart", json!({ "event": "OpenDoor", "params": bayleaf_graph::param_pins(&declared) }));
    let log = b.node("log", json!({ "message": "open" }));
    b.link(on, "then", log, "run");
    b.link(on, "speed", log, "value");
    let (mut scene, id, mut host) = run(&b.bytes(), &[]);
    // What an element's On Click sends: the event's name, with no values.
    host.send(&mut scene, id, "OpenDoor", 0.0, 0.0, &Arc::new(Input::default()));
    host.send(&mut scene, id, "SomethingElse", 0.0, 0.0, &Arc::new(Input::default()));
    let logs: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(logs, ["open 0"], "the event runs once, for its own name, its value read as zero");
}

#[test]
fn a_script_subscribes_to_a_components_event_hears_it_with_its_source_and_values_and_can_stop() {
    use bayleaf_scene::components::Scripts;
    let died = vec![bayleaf_graph::Param::new("amount".to_owned(), VariableType::Number)];
    let pins = bayleaf_graph::param_pins(&died);
    // The publisher declares Died and logs when it happens on it.
    let mut publisher = Builder::new();
    publisher.0.add_event("Died").unwrap();
    publisher.0.set_event(&CATALOG, "Died", None, Some(died.clone()), &super::EVENT_KINDS).unwrap();
    let on = publisher.node("eventStart", json!({ "event": "Died", "params": pins }));
    let said = publisher.node("log", json!({ "message": "died" }));
    publisher.link(on, "then", said, "run");
    publisher.link(on, "amount", said, "value");

    // The subscriber finds the Publisher component, subscribes to Died, and triggers it there; a
    // message later it cancels and triggers it again.
    let mut subscriber = Builder::new();
    let component = |b: &mut Builder| {
        let found = b.node("findNamed", json!({ "name": "Boss" }));
        let get = b.node("getComponent", json!({ "component": "Publisher", "fields": [] }));
        b.link(found, "object", get, "target");
        get
    };
    let start = subscriber.node("start", json!({}));
    let get = component(&mut subscriber);
    let subscribe = subscriber.node("subscribeEvent", json!({ "script": "Publisher", "event": "Died", "params": pins }));
    subscriber.link(start, "then", subscribe, "run");
    subscriber.link(get, "component", subscribe, "component");
    let trigger = subscriber.node("triggerComponentEvent", json!({ "script": "Publisher", "event": "Died", "params": pins }));
    subscriber.0.set_value(&CATALOG, trigger, "amount", json!(3.0)).unwrap();
    subscriber.link(subscribe, "then", trigger, "run");
    subscriber.link(get, "component", trigger, "component");
    let heard = subscriber.node("componentEventStart", json!({ "script": "Publisher", "event": "Died", "params": pins }));
    let source = subscriber.node("componentObject", json!({}));
    subscriber.link(heard, "source", source, "component");
    let name = subscriber.node("nameOf", json!({}));
    subscriber.link(source, "object", name, "target");
    let log = subscriber.node("log", json!({ "message": "heard" }));
    subscriber.link(heard, "then", log, "run");
    subscriber.link(name, "result", log, "text");
    subscriber.link(heard, "amount", log, "value");
    let stop = subscriber.node("message", json!({ "message": "stop" }));
    let get_again = component(&mut subscriber);
    let unsubscribe = subscriber.node("unsubscribeEvent", json!({ "script": "Publisher", "event": "Died", "params": pins }));
    subscriber.link(stop, "then", unsubscribe, "run");
    subscriber.link(get_again, "component", unsubscribe, "component");
    let again = subscriber.node("triggerComponentEvent", json!({ "script": "Publisher", "event": "Died", "params": pins }));
    subscriber.0.set_value(&CATALOG, again, "amount", json!(4.0)).unwrap();
    subscriber.link(unsubscribe, "then", again, "run");
    subscriber.link(get_again, "component", again, "component");

    let mut scene = Scene::new();
    let input = Arc::new(Input::default());
    let boss = scene.create_object(None, "Boss").unwrap();
    let watcher = scene.create_object(None, "Watcher").unwrap();
    let script = |module: &str| ScriptRef { module: module.to_owned(), properties: BTreeMap::new() };
    scene.set(boss, Scripts(vec![script("publisher.bscript")])).unwrap();
    scene.set(watcher, Scripts(vec![script("watcher.bscript")])).unwrap();
    let mut host = ScriptHost::new();
    host.load_bytes(boss, 0, Path::new("publisher.bscript"), None, &publisher.bytes(), &script("publisher.bscript"), &mut scene, 0.0, &input).unwrap();
    host.load_bytes(watcher, 0, Path::new("watcher.bscript"), None, &subscriber.bytes(), &script("watcher.bscript"), &mut scene, 0.0, &input).unwrap();
    host.update(&mut scene, 0.1, 0.0, &input);
    let logs: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(logs, ["died 3", "heard Boss 3"], "the event runs on the component, then the subscriber hears it with where it happened");
    host.send(&mut scene, watcher, "stop", 0.0, 0.0, &input);
    host.update(&mut scene, 0.1, 0.1, &input);
    let logs: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(logs, ["died 4"], "after cancelling, the subscriber no longer hears it");
}

#[test]
fn lists_keep_their_order_and_maps_their_keys_for_the_whole_of_play() {
    let mut b = Builder::new();
    shaped_variable(&mut b, "Scores", VariableType::Number, Shape::List, Some(json!([3.0])));
    shaped_variable(&mut b, "Words", VariableType::Text, Shape::Map, None);
    let list = |extra: serde_json::Value| {
        let mut values = json!({ "name": "Scores", "item": ["number"] });
        values.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
        values
    };
    let map = |extra: serde_json::Value| {
        let mut values = json!({ "name": "Words", "item": ["text"] });
        values.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
        values
    };
    let start = b.node("start", json!({}));
    // [3] (as it starts) → [3, 1] → [5, 3, 1] → [5, 3, 7] → [5, 7]; inserting past the end does nothing.
    let steps = [
        b.node("listAdd", list(json!({ "value": 1.0 }))),
        b.node("listInsert", list(json!({ "index": 0, "value": 5.0 }))),
        b.node("listInsert", list(json!({ "index": 9, "value": 8.0 }))),
        b.node("listSet", list(json!({ "index": 2, "value": 7.0 }))),
        b.node("listRemove", list(json!({ "index": 1 }))),
        b.node("mapSet", map(json!({ "key": "b", "value": "two" }))),
        b.node("mapSet", map(json!({ "key": "c", "value": "three" }))),
        b.node("mapSet", map(json!({ "key": "a", "value": "one" }))),
        b.node("mapRemove", map(json!({ "key": "c" }))),
    ];
    let mut previous = (start, "then");
    for step in steps {
        b.link(previous.0, previous.1, step, "run");
        previous = (step, "then");
    }
    // Every item in order, then every key in key order with its value.
    let each = b.node("listForEach", list(json!({})));
    b.link(previous.0, previous.1, each, "run");
    let item = b.node("log", json!({ "message": "item" }));
    b.link(each, "each", item, "run");
    b.link(each, "value", item, "value");
    let entries = b.node("mapForEach", map(json!({})));
    b.link(each, "then", entries, "run");
    let key = b.node("log", json!({ "message": "" }));
    b.link(entries, "each", key, "run");
    b.link(entries, "key", key, "text");
    let word = b.node("log", json!({ "message": "" }));
    b.link(key, "then", word, "run");
    b.link(entries, "value", word, "text");
    // What the list and map hold now, and what they do not.
    let mut previous = (entries, "then");
    let reads = [
        ("listLength", list(json!({})), "length", "length"),
        ("listFind", list(json!({ "value": 7.0 })), "index", "seven at"),
        ("listFind", list(json!({ "value": 3.0 })), "index", "three at"),
        ("mapCount", map(json!({})), "count", "count"),
    ];
    for (kind, settings, pin, message) in reads {
        let read = b.node(kind, settings);
        let said = b.node("log", json!({ "message": message }));
        b.link(previous.0, previous.1, said, "run");
        b.link(read, pin, said, "value");
        previous = (said, "then");
    }
    let past = b.node("listItem", list(json!({ "index": 2 })));
    let missing = b.node("mapGet", map(json!({ "key": "c" })));
    let neither = b.node("or", json!({}));
    b.link(past, "found", neither, "a");
    b.link(missing, "found", neither, "b");
    let gate = b.node("branch", json!({}));
    b.link(previous.0, previous.1, gate, "run");
    b.link(neither, "result", gate, "condition");
    let none = b.node("log", json!({ "message": "none past the end" }));
    b.link(gate, "false", none, "run");
    let first = b.node("listItem", list(json!({ "index": 1 })));
    let one = b.node("mapGet", map(json!({ "key": "a" })));
    let found = b.node("log", json!({ "message": "second" }));
    b.link(none, "then", found, "run");
    b.link(first, "value", found, "value");
    b.link(one, "value", found, "text");
    // Each step adds to the list, which lasts from step to step.
    let update = b.node("update", json!({}));
    let grow = b.node("listAdd", list(json!({ "value": 2.0 })));
    b.link(update, "then", grow, "run");
    let length = b.node("listLength", list(json!({})));
    let grown = b.node("log", json!({ "message": "now" }));
    b.link(grow, "then", grown, "run");
    b.link(length, "length", grown, "value");

    let (mut scene, _, mut host) = run(&b.bytes(), &[]);
    for _ in 0..2 {
        host.update(&mut scene, 0.1, 0.0, &Arc::new(Input::default()));
    }
    assert!(host.take_faults().is_empty());
    let logs: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(
        logs,
        ["item 5", "item 7", "a", "one", "b", "two", "length 2", "seven at 1", "three at -1", "count 2", "none past the end", "second one 7", "now 3", "now 4"],
        "lists keep their order, maps their keys, and both last from step to step"
    );
}

#[test]
fn a_list_node_naming_a_map_or_nothing_is_a_problem_on_that_node() {
    let mut b = Builder::new();
    shaped_variable(&mut b, "Words", VariableType::Text, Shape::Map, None);
    let start = b.node("start", json!({}));
    let clear = b.node("listClear", json!({ "name": "Words", "item": ["text"] }));
    b.link(start, "then", clear, "run");
    let problem = compile(&b.0).unwrap_err();
    assert!(problem.iter().any(|p| p.node == Some(clear)), "the node that names a map as a list is the one in trouble: {problem:?}");
    b.0.remove_variable("Words").unwrap();
    assert!(compile(&b.0).unwrap_err().iter().any(|p| p.node == Some(clear)), "and so is one naming a list or map the script lost");
}

/// Declares a variable of `ty` in `shape`, starting with `start` (else empty).
fn shaped_variable(b: &mut Builder, name: &str, ty: VariableType, shape: Shape, start: Option<serde_json::Value>) {
    b.0.add_variable(name, ty, None).unwrap();
    b.0.retype_variable(name, ty, shape, VariableType::Text).unwrap();
    if let Some(start) = start {
        b.0.set_variable(name, None, Some(start), &[]).unwrap();
    }
}

/// Declares a property of `ty` in `shape`, with `default` (else empty).
fn shaped_property(b: &mut Builder, name: &str, ty: VariableType, shape: Shape, default: Option<serde_json::Value>) {
    b.0.add_property(name, ty, None).unwrap();
    b.0.retype_property(name, ty, shape, VariableType::Text).unwrap();
    if let Some(default) = default {
        b.0.set_property(name, None, Some(default), &[]).unwrap();
    }
}

#[test]
fn a_list_property_starts_with_what_the_object_sets_objects_found_by_name() {
    let mut b = Builder::new();
    shaped_property(&mut b, "Targets", VariableType::Object, Shape::List, None);
    let start = b.node("start", json!({}));
    let each = b.node("listForEach", json!({ "name": "Targets", "item": ["object"] }));
    b.link(start, "then", each, "run");
    let named = b.node("nameOf", json!({}));
    b.link(each, "value", named, "target");
    let said = b.node("log", json!({ "message": "target" }));
    b.link(each, "each", said, "run");
    b.link(named, "result", said, "text");

    // The script says the property is a list of objects, so the Inspector can edit it as one.
    let declared = ScriptHost::new().describe_all("targets.bscript", &b.bytes()).unwrap().properties;
    assert_eq!((declared[0].ty, declared[0].shape), (PropertyType::Object, bayleaf_scene::components::PropertyShape::List));

    let mut scene = Scene::new();
    let id = scene.create_object(None, "Scripted").unwrap();
    scene.create_object(None, "Door").unwrap();
    let names = PropertyValue::List { list: vec![PropertyValue::Text("Door".into()), PropertyValue::Text("Nobody".into())] };
    let script = ScriptRef { module: "test.bscript".into(), properties: BTreeMap::from([("Targets".to_owned(), names)]) };
    let mut host = ScriptHost::new();
    host.load_bytes(id, 0, Path::new("test.bscript"), None, &b.bytes(), &script, &mut scene, 0.0, &Arc::new(Input::default())).unwrap();
    host.update(&mut scene, 0.1, 0.0, &Arc::new(Input::default()));
    assert!(host.take_faults().is_empty());
    let logs: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(logs, ["target Door", "target "], "the object's own items, in order; a name no object has is none (no name)");
}

#[test]
fn a_list_goes_through_pins_into_functions_as_itself_and_into_events_as_a_copy() {
    let mut b = Builder::new();
    shaped_variable(&mut b, "Scores", VariableType::Number, Shape::List, Some(json!([1.0, 2.0])));
    let scores = json!({ "name": "Scores", "item": ["number"] });
    let list_of_numbers = bayleaf_graph::Param { name: "scores".to_owned(), ty: VariableType::Number, shape: Shape::List, key: VariableType::Text };
    // A function that adds 10 to the list it is given.
    b.0.add_function("addTen").unwrap();
    b.0.set_function(&CATALOG, "addTen", None, Some(vec![list_of_numbers.clone()]), Some(Vec::new()), &FUNCTION_KINDS).unwrap();
    let begin = function_node(&mut b, "functionStart", "addTen");
    let add = b.node("listAdd", json!({ "item": ["number"], "value": 10.0 }));
    b.link(begin, "then", add, "run");
    b.link(begin, "scores", add, "list");
    // On start: the function gets Scores itself, so Scores grows.
    let start = b.node("start", json!({}));
    let call = function_node(&mut b, "callFunction", "addTen");
    b.link(start, "then", call, "run");
    let whole = b.node("getList", scores.clone());
    b.link(whole, "list", call, "scores");
    let length = b.node("listLength", scores.clone());
    let after = b.node("log", json!({ "message": "length" }));
    b.link(call, "then", after, "run");
    b.link(length, "length", after, "value");
    // An event brings Scores to this object as a copy.
    b.0.add_event("Report").unwrap();
    b.0.set_event(&CATALOG, "Report", None, Some(vec![list_of_numbers]), &super::EVENT_KINDS).unwrap();
    let pins = bayleaf_graph::param_pins(&b.0.event("Report").unwrap().params);
    let report = b.node("triggerEvent", json!({ "event": "Report", "params": pins }));
    b.link(after, "then", report, "run");
    b.link(whole, "list", report, "scores");
    let me = b.node("thisObject", json!({}));
    b.link(me, "object", report, "target");
    let heard = b.node("eventStart", json!({ "event": "Report", "params": pins }));
    let each = b.node("listForEach", json!({ "item": ["number"] }));
    b.link(heard, "then", each, "run");
    b.link(heard, "scores", each, "list");
    let got = b.node("log", json!({ "message": "got" }));
    b.link(each, "each", got, "run");
    b.link(each, "value", got, "value");
    // Changing the copy leaves Scores as it was.
    let change = b.node("listClear", json!({ "item": ["number"] }));
    b.link(each, "then", change, "run");
    b.link(heard, "scores", change, "list");
    let original = b.node("log", json!({ "message": "still" }));
    b.link(change, "then", original, "run");
    b.link(length, "length", original, "value");

    let (mut scene, _, mut host) = run(&b.bytes(), &[]);
    host.update(&mut scene, 0.1, 0.0, &Arc::new(Input::default()));
    host.update(&mut scene, 0.1, 0.1, &Arc::new(Input::default()));
    assert!(host.take_faults().is_empty());
    let logs: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(logs, ["length 3", "got 1", "got 2", "got 10", "still 3"], "the function changes Scores; the event's copy is its own");
}

#[test]
fn a_list_pin_takes_only_a_list_of_its_type_and_a_list_node_takes_the_type_linked_into_it() {
    let mut b = Builder::new();
    shaped_variable(&mut b, "Words", VariableType::Text, Shape::List, None);
    let words = b.node("getList", json!({ "name": "Words", "item": ["text"] }));
    let count = b.node("listLength", json!({}));
    assert!(
        super::adopted_item(&b.0, &Link { from: PinRef::new(words, "list"), to: PinRef::new(count, "list") })
            == Some((count, VariableType::Text, VariableType::Text))
    );
    // A list of text does not go into a list of numbers as it is.
    let refused = b.0.connect(&CATALOG, Link { from: PinRef::new(words, "list"), to: PinRef::new(count, "list") });
    assert!(refused.is_err(), "a fresh list node holds numbers until it takes another type");
    b.set_value_for_tests(count, "item", json!(["text"]));
    b.link(words, "list", count, "list");
    // Nor into a number input.
    let log = b.node("log", json!({}));
    assert!(b.0.connect(&CATALOG, Link { from: PinRef::new(words, "list"), to: PinRef::new(log, "value") }).is_err());
}

#[test]
fn a_maps_keys_have_a_type_of_their_own_and_go_in_that_types_order() {
    let mut b = Builder::new();
    // Whole-number keys, starting as the object sets them in the scene.
    b.0.add_property("Waves", VariableType::Text, None).unwrap();
    b.0.retype_property("Waves", VariableType::Text, Shape::Map, VariableType::WholeNumber).unwrap();
    // Object keys, set while playing.
    b.0.add_variable("Scores", VariableType::Number, None).unwrap();
    b.0.retype_variable("Scores", VariableType::Number, Shape::Map, VariableType::Object).unwrap();
    let waves = json!({ "name": "Waves", "item": ["text", "whole"] });
    let scores = json!({ "name": "Scores", "item": ["number", "object"] });
    let start = b.node("start", json!({}));
    let more = b.node("mapSet", waves.clone());
    b.set_value_for_tests(more, "key", json!(5));
    b.set_value_for_tests(more, "value", json!("five"));
    b.link(start, "then", more, "run");
    // 1, 2, 5, 10: numbers by size, not as text would sort them.
    let each = b.node("mapForEach", waves.clone());
    b.link(more, "then", each, "run");
    let said = b.node("log", json!({ "message": "wave" }));
    b.link(each, "each", said, "run");
    b.link(each, "key", said, "value");
    b.link(each, "value", said, "text");
    // An object as a key: this one.
    let me = b.node("thisObject", json!({}));
    let score = b.node("mapSet", scores.clone());
    b.set_value_for_tests(score, "value", json!(7.0));
    b.link(me, "object", score, "key");
    b.link(each, "then", score, "run");
    let mine = b.node("mapGet", scores);
    b.link(me, "object", mine, "key");
    let got = b.node("log", json!({ "message": "mine" }));
    b.link(score, "then", got, "run");
    b.link(mine, "value", got, "value");
    let ten = b.node("mapGet", waves);
    b.set_value_for_tests(ten, "key", json!(10));
    let said_ten = b.node("log", json!({ "message": "ten is" }));
    b.link(got, "then", said_ten, "run");
    b.link(ten, "value", said_ten, "text");

    let mut scene = Scene::new();
    let id = scene.create_object(None, "Scripted").unwrap();
    let text = |t: &str| PropertyValue::Text(t.to_owned());
    let set = PropertyValue::Map { map: [("10".to_owned(), text("ten")), ("2".to_owned(), text("two")), ("1".to_owned(), text("one"))].into() };
    let script = ScriptRef { module: "test.bscript".into(), properties: BTreeMap::from([("Waves".to_owned(), set)]) };
    let mut host = ScriptHost::new();
    host.load_bytes(id, 0, Path::new("test.bscript"), None, &b.bytes(), &script, &mut scene, 0.0, &Arc::new(Input::default())).unwrap();
    host.update(&mut scene, 0.1, 0.0, &Arc::new(Input::default()));
    assert!(host.take_faults().is_empty());
    let logs: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(logs, ["wave one 1", "wave two 2", "wave five 5", "wave ten 10", "mine 7", "ten is ten"]);
}

#[test]
fn objects_running_one_graph_each_keep_their_own_variables_and_text() {
    let mut b = Builder::new();
    // Each step: count += 1, and the label becomes "<this object's name>"; log both.
    let update = b.node("update", json!({}));
    let count = b.node("variable", json!({ "name": "count" }));
    let more = b.node("add", json!({ "b": 1.0 }));
    b.link(count, "value", more, "a");
    let set = b.node("setVariable", json!({ "name": "count" }));
    b.link(update, "then", set, "run");
    b.link(more, "result", set, "value");
    let me = b.node("thisObject", json!({}));
    let name = b.node("nameOf", json!({}));
    b.link(me, "object", name, "target");
    let label = b.node("setTextVariable", json!({ "name": "label" }));
    b.link(set, "then", label, "run");
    b.link(name, "result", label, "value");
    let said = b.node("log", json!({ "message": "" }));
    b.link(label, "then", said, "run");
    let read = b.node("textVariable", json!({ "name": "label" }));
    b.link(read, "value", said, "text");
    let now = b.node("variable", json!({ "name": "count" }));
    b.link(now, "value", said, "value");
    let bytes = b.bytes();
    assert!(wat_exports(&bytes).contains(&"bayleaf_update_all".to_owned()), "a graph runs every object of its file in one instance");

    let mut scene = Scene::new();
    let first = scene.create_object(None, "First").unwrap();
    let second = scene.create_object(None, "Second").unwrap();
    let mut host = ScriptHost::new();
    let input = Arc::new(Input::default());
    let script = ScriptRef { module: "count.bscript".into(), properties: BTreeMap::new() };
    for id in [first, second] {
        host.load_bytes(id, 0, Path::new("count.bscript"), None, &bytes, &script, &mut scene, 0.0, &input).unwrap();
    }
    host.update(&mut scene, 0.1, 0.0, &input);
    host.update(&mut scene, 0.1, 0.1, &input);
    assert!(host.take_faults().is_empty());
    let logs: Vec<String> = host.take_logs().into_iter().map(|l| l.text).collect();
    assert_eq!(logs, ["First 1", "Second 1", "First 2", "Second 2"], "each object counts on its own and keeps its own text");
}

#[test]
fn objects_running_one_graph_see_another_object_where_they_last_moved_it() {
    let mut b = Builder::new();
    // Each step: Target's x += 1, then log Target's x, read again.
    let update = b.node("update", json!({}));
    let before = b.node("positionOf", json!({ "object": "Target" }));
    let parts = b.node("split", json!({}));
    b.link(before, "position", parts, "vector");
    let more = b.node("add", json!({ "b": 1.0 }));
    b.link(parts, "x", more, "a");
    let moved = b.node("vector", json!({}));
    b.link(more, "result", moved, "x");
    let set = b.node("setPositionOf", json!({ "object": "Target" }));
    b.link(update, "then", set, "run");
    b.link(moved, "vector", set, "position");
    let after = b.node("positionOf", json!({ "object": "Target" }));
    let now = b.node("split", json!({}));
    b.link(after, "position", now, "vector");
    let said = b.node("log", json!({ "message": "" }));
    b.link(set, "then", said, "run");
    b.link(now, "x", said, "value");
    let bytes = b.bytes();
    assert!(wat_exports(&bytes).contains(&"bayleaf_update_all".to_owned()), "a graph runs every object of its file in one instance");

    let mut scene = Scene::new();
    scene.create_object(None, "Target").unwrap();
    let movers = [scene.create_object(None, "First").unwrap(), scene.create_object(None, "Second").unwrap()];
    let mut host = ScriptHost::new();
    let input = Arc::new(Input::default());
    let script = ScriptRef { module: "push.bscript".into(), properties: BTreeMap::new() };
    for id in movers {
        host.load_bytes(id, 0, Path::new("push.bscript"), None, &bytes, &script, &mut scene, 0.0, &input).unwrap();
    }
    let read = |host: &mut ScriptHost| -> Vec<f64> { host.take_logs().into_iter().map(|l| l.text.parse().unwrap()).collect() };
    host.update(&mut scene, 0.1, 0.0, &input);
    let first_step = read(&mut host);
    host.update(&mut scene, 0.1, 0.1, &input);
    let second_step = read(&mut host);
    assert!(host.take_faults().is_empty());
    // Within a step, each object sees the move it just made; the next step starts where the last left it.
    assert!(first_step.iter().all(|x| *x > 0.0), "a move shows at once: {first_step:?}");
    assert!(
        second_step.iter().zip(&first_step).all(|(later, earlier)| later > earlier),
        "each step goes on from the last: {first_step:?} then {second_step:?}"
    );
}

#[test]
fn objects_running_one_graph_lose_the_object_they_keep_when_it_goes() {
    let mut b = Builder::new();
    // At start, keep the object named Target; each step, log 1 while it is still there, else 0.
    let start = b.node("start", json!({}));
    let found = b.node("findNamed", json!({ "name": "Target" }));
    let keep = b.node("setObjectVariable", json!({ "name": "target" }));
    b.link(start, "then", keep, "run");
    b.link(found, "object", keep, "value");
    let update = b.node("update", json!({}));
    let kept = b.node("objectVariable", json!({ "name": "target" }));
    let there = b.node("choose", json!({ "a": 1.0, "b": 0.0 }));
    b.link(kept, "found", there, "condition");
    let said = b.node("log", json!({ "message": "" }));
    b.link(update, "then", said, "run");
    b.link(there, "result", said, "value");
    let bytes = b.bytes();

    let mut scene = Scene::new();
    let watchers = [scene.create_object(None, "First").unwrap(), scene.create_object(None, "Second").unwrap()];
    let target = scene.create_object(None, "Target").unwrap();
    scene.create_object(None, "Bystander").unwrap();
    let mut host = ScriptHost::new();
    let input = Arc::new(Input::default());
    let script = ScriptRef { module: "watch.bscript".into(), properties: BTreeMap::new() };
    for id in watchers {
        host.load_bytes(id, 0, Path::new("watch.bscript"), None, &bytes, &script, &mut scene, 0.0, &input).unwrap();
    }
    let step = |host: &mut ScriptHost, scene: &mut Scene| -> Vec<String> {
        host.update(scene, 0.1, 0.0, &input);
        host.take_logs().into_iter().map(|l| l.text).collect()
    };
    assert_eq!(step(&mut host, &mut scene), ["1", "1"]);
    assert_eq!(step(&mut host, &mut scene), ["1", "1"], "kept from step to step");
    // The object after it takes its place in the hierarchy; what each watcher keeps is still gone.
    scene.delete_object(target).unwrap();
    assert_eq!(step(&mut host, &mut scene), ["0", "0"]);
    assert!(host.take_faults().is_empty());
}

/// The names a module exports.
fn wat_exports(bytes: &[u8]) -> Vec<String> {
    let module = wasmi::Module::new(&wasmi::Engine::default(), bytes).unwrap();
    module.exports().map(|e| e.name().to_owned()).collect()
}
