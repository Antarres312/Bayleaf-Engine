use serde_json::json;

use super::*;

const NUM: PinDef = PinDef::new("value", "Value", PinType::Float);
const CATALOG: Catalog = Catalog {
    domain: "test",
    label: "Test",
    extension: "btest",
    nodes: &[
        NodeDef {
            kind: "number",
            label: "Number",
            category: "Values",
            description: "",
            inputs: &[],
            outputs: &[NUM],
            settings: &[SettingDef { name: "value", label: "Value", kind: SettingKind::Float(0.0) }],
            unique: false,
        },
        NodeDef {
            kind: "add",
            label: "Add",
            category: "Math",
            description: "",
            inputs: &[PinDef::new("a", "A", PinType::Any), PinDef::new("b", "B", PinType::Any)],
            outputs: &[PinDef::new("result", "Result", PinType::Any)],
            settings: &[],
            unique: false,
        },
        NodeDef {
            kind: "vector",
            label: "Vector",
            category: "Values",
            description: "",
            inputs: &[],
            outputs: &[PinDef::new("vector", "Vector", PinType::Vec3)],
            settings: &[],
            unique: false,
        },
        NodeDef {
            kind: "out",
            label: "Output",
            category: "Output",
            description: "",
            inputs: &[PinDef::new("flow", "Run", PinType::Flow), PinDef::new("amount", "Amount", PinType::Float), PinDef::new("on", "On", PinType::Bool)],
            outputs: &[PinDef::new("then", "Then", PinType::Flow)],
            settings: &[],
            unique: true,
        },
    ],
    starter: &[("out", [300.0, 0.0])],
    variant: crate::no_variant,
    upgrade: crate::no_upgrade,
};

fn link(from: u32, from_pin: &str, to: u32, to_pin: &str) -> Link {
    Link { from: PinRef::new(from, from_pin), to: PinRef::new(to, to_pin) }
}

#[test]
fn incompatible_pins_are_refused_with_both_types_named() {
    let mut g = Graph::new(&CATALOG);
    let out = g.nodes[0].id;
    let v = g.add_node(&CATALOG, "vector", [0.0, 0.0]).unwrap();
    let error = g.connect(&CATALOG, link(v, "vector", out, "amount")).unwrap_err().to_string();
    assert!(error.contains("Vector") && error.contains("Number"), "{error}");
    assert!(g.connect(&CATALOG, link(v, "vector", out, "on")).is_err(), "vectors are not true/false");
    let n = g.add_node(&CATALOG, "number", [0.0, 0.0]).unwrap();
    assert!(g.connect(&CATALOG, link(n, "value", out, "amount")).is_ok());
}

#[test]
fn a_link_that_would_close_a_loop_is_refused() {
    let mut g = Graph::empty(&CATALOG);
    let a = g.add_node(&CATALOG, "add", [0.0, 0.0]).unwrap();
    let b = g.add_node(&CATALOG, "add", [0.0, 0.0]).unwrap();
    g.connect(&CATALOG, link(a, "result", b, "a")).unwrap();
    assert!(g.connect(&CATALOG, link(b, "result", a, "a")).unwrap_err().to_string().contains("loop"));
    assert!(g.connect(&CATALOG, link(a, "result", a, "b")).is_err(), "no self links");
}

#[test]
fn a_data_input_keeps_only_its_newest_link() {
    let mut g = Graph::empty(&CATALOG);
    let one = g.add_node(&CATALOG, "number", [0.0, 0.0]).unwrap();
    let two = g.add_node(&CATALOG, "number", [0.0, 0.0]).unwrap();
    let add = g.add_node(&CATALOG, "add", [0.0, 0.0]).unwrap();
    g.connect(&CATALOG, link(one, "value", add, "a")).unwrap();
    g.connect(&CATALOG, link(two, "value", add, "a")).unwrap();
    assert_eq!(g.links.len(), 1);
    assert_eq!(g.link_into(add, "a").map(|l| l.from.node), Some(two));
    // One output may feed many inputs.
    g.connect(&CATALOG, link(two, "value", add, "b")).unwrap();
    assert_eq!(g.links_from(two, "value").count(), 2);
}

#[test]
fn removing_a_node_removes_its_links_and_unique_kinds_stay_unique() {
    let mut g = Graph::new(&CATALOG);
    let out = g.nodes[0].id;
    assert!(g.add_node(&CATALOG, "out", [0.0, 0.0]).is_err());
    let n = g.add_node(&CATALOG, "number", [0.0, 0.0]).unwrap();
    g.connect(&CATALOG, link(n, "value", out, "amount")).unwrap();
    g.remove_node(n).unwrap();
    assert!(g.links.is_empty());
    assert!(g.remove_node(n).is_err());
}

#[test]
fn values_are_checked_against_the_pin_or_setting() {
    let mut g = Graph::new(&CATALOG);
    let out = g.nodes[0].id;
    assert!(g.set_value(&CATALOG, out, "amount", json!(2.5)).is_ok());
    assert!(g.set_value(&CATALOG, out, "amount", json!("lots")).is_err());
    assert!(g.set_value(&CATALOG, out, "on", json!(true)).is_ok());
    assert!(g.set_value(&CATALOG, out, "flow", json!(1)).is_err(), "flow inputs hold no value");
    assert!(g.set_value(&CATALOG, out, "missing", json!(1)).is_err());
    g.set_value(&CATALOG, out, "amount", serde_json::Value::Null).unwrap();
    assert!(g.value(out, "amount").is_none(), "null restores the default");
}

#[test]
fn files_round_trip_and_broken_files_are_refused() {
    let mut g = Graph::new(&CATALOG);
    let n = g.add_node(&CATALOG, "number", [10.0, 20.0]).unwrap();
    g.set_value(&CATALOG, n, "value", json!(3.0)).unwrap();
    g.connect(&CATALOG, link(n, "value", g.nodes[0].id, "amount")).unwrap();
    let text = g.to_json();
    assert_eq!(Graph::from_json(&text, &CATALOG).unwrap(), g);

    let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
    value["links"][0]["to"]["pin"] = json!("on");
    assert!(Graph::from_json(&value.to_string(), &CATALOG).is_err(), "a number into a true/false input");
    let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
    value["nodes"][1]["kind"] = json!("explode");
    assert!(Graph::from_json(&value.to_string(), &CATALOG).unwrap_err().to_string().contains("explode"));
    let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
    value["domain"] = json!("material");
    assert!(Graph::from_json(&value.to_string(), &CATALOG).is_err());
    assert!(Graph::from_json("{", &CATALOG).is_err());
}

#[test]
fn comments_keep_their_limits_and_survive_a_file_round_trip() {
    let mut graph = Graph::empty(&CATALOG);
    let id = graph.add_comment("Movement", [10.0, 20.0], [1.0, 1.0]).unwrap();
    let small = graph.comment(id).unwrap().size;
    assert!(small[0] >= MIN_COMMENT_SIZE[0] && small[1] >= MIN_COMMENT_SIZE[1], "a tiny box grows to the smallest size");
    let long = "x".repeat(MAX_COMMENT_TEXT + 1);
    assert!(graph.edit_comment(id, Some(&long), None, None).is_err());
    assert!(graph.edit_comment(id, None, Some([f32::NAN, 0.0]), None).is_err());
    assert_eq!(graph.comment(id).unwrap().text, "Movement", "a refused edit changes nothing");
    graph.edit_comment(id, Some("Jumping"), Some([5.0, 5.0]), None).unwrap();

    let loaded = Graph::from_json(&graph.to_json(), &CATALOG).unwrap();
    assert_eq!(loaded.comments, graph.comments);
    let mut bad: serde_json::Value = serde_json::from_str(&graph.to_json()).unwrap();
    bad["comments"][0]["size"] = json!([1.0, 1.0]);
    assert!(Graph::from_json(&bad.to_string(), &CATALOG).is_err(), "a file with a box below the smallest size is refused");
    graph.remove_comment(id).unwrap();
    assert!(graph.remove_comment(id).is_err());
}
