//! The C# engine's port of the visual script catalog (`src/Bayleaf.Script`) must read and write every
//! visual script file as this crate does (docs/csharp-redesign.md): node kinds, their pins as their
//! settings make them, and the upgrade of older files. Both engines' tests compare what they give with
//! `tests/files/expected/`, which this test writes when `BAYLEAF_BLESS=1` is set. The WebAssembly
//! compiler is not part of it: the C# engine's scripts are C#.

use std::path::{Path, PathBuf};

use bayleaf_graph::Graph;
use serde_json::{Value, json};

use crate::visual::{CATALOG, COMPONENTS};

fn files_below(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "expected") {
                files_below(&path, out);
            }
        } else {
            out.push(path);
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("..")
}

/// Compares `written` with the expected file `name`, first writing it there when blessing.
fn check(name: &str, written: &str, what: &Path) {
    let expected_dir = root().join("tests").join("files").join("expected");
    let expected_path = expected_dir.join(name);
    if std::env::var_os("BAYLEAF_BLESS").is_some_and(|v| v == "1") {
        std::fs::create_dir_all(&expected_dir).unwrap();
        std::fs::write(&expected_path, written).unwrap();
    }
    let expected = std::fs::read_to_string(&expected_path).unwrap_or_default().replace("\r\n", "\n");
    assert_eq!(written, expected, "{} gives other text than {}", what.display(), expected_path.display());
}

/// The file as this crate writes it back after reading it with the visual script catalog, or
/// `refused` when it does not read.
fn rewrite(path: &Path) -> String {
    let Ok(text) = std::fs::read_to_string(path) else { return "refused\n".to_owned() };
    Graph::from_json(&text, &CATALOG).map_or_else(|_| "refused\n".to_owned(), |graph| graph.to_json())
}

/// Every `.bscript` of the examples, the benchmark and `tests/files` reads and writes back as the
/// expected files say.
#[test]
fn the_shared_visual_script_files_rewrite_as_the_expected_files_say() {
    let root = root();
    let mut inputs = Vec::new();
    files_below(&root.join("examples"), &mut inputs);
    files_below(&root.join("rust").join("bench").join("graphs"), &mut inputs);
    files_below(&root.join("tests").join("files"), &mut inputs);
    let mut checked = 0;
    for input in inputs.iter().filter(|p| p.extension().is_some_and(|e| e == "bscript")) {
        let relative = input.strip_prefix(&root).unwrap_or(input).components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>();
        let name = relative.iter().skip_while(|c| *c == "tests" || *c == "files").cloned().collect::<Vec<_>>().join("__");
        check(&name, &rewrite(input), input);
        checked += 1;
    }
    assert!(checked > 10, "only {checked} shared visual script files were found");
}

/// Nodes whose pins come from their settings, each with the settings it is given.
fn variant_samples() -> Vec<(&'static str, Value)> {
    let params = json!([
        "height:number",
        "count:whole",
        "on:bool",
        "aim:vector",
        "tint:color",
        "label:text",
        "who:object",
        "heights:list:number",
        "byName:map:object:whole",
        "words:map:text",
        "odd:nothing",
        "plain"
    ]);
    vec![
        ("getComponent", json!({})),
        ("getComponent", json!({ "component": "point light" })),
        ("tryGetComponent", json!({ "component": "Character" })),
        ("setComponent", json!({ "component": "transform" })),
        ("setComponent", json!({ "component": "Game_Rules" })),
        (
            "getComponent",
            json!({ "component": "Bob", "fields": ["speed", "count:whole", "on:bool", "offset:vector", "tint:color", "label:text", "jump_height", "élanVital", "two words"] }),
        ),
        ("tryGetComponent", json!({ "component": "Bob", "fields": ["speed", "count:whole"] })),
        ("setComponent", json!({ "component": "Bob", "fields": ["speed", "on:bool", "offset:vector"] })),
        ("getComponent", json!({ "component": "Bob" })),
        ("dataFile", json!({})),
        (
            "dataFile",
            json!({ "fields": ["hp:number", "level:whole", "boss:bool", "spawn:vector", "tint:color", "home:object", "name:text", "drop:prefab", "cry:sound", "skin:material", "more:data", "loot:list:text", "odd:nothing", "plain"] }),
        ),
        ("functionStart", json!({ "function": "jump", "inputs": params, "outputs": params })),
        ("functionReturn", json!({ "function": "jump", "inputs": params, "outputs": params })),
        ("callFunction", json!({ "function": "jump", "inputs": params, "outputs": params })),
        ("callFunction", json!({})),
        ("eventStart", json!({ "event": "hit", "params": params })),
        ("triggerEvent", json!({ "event": "hit", "params": params })),
        ("triggerComponentEvent", json!({ "event": "hit", "params": params })),
        ("componentEventStart", json!({ "event": "hit", "params": params })),
        ("listAdd", json!({})),
        ("listItem", json!({ "item": ["text"] })),
        ("listForEach", json!({ "item": ["vector", "whole"] })),
        ("getList", json!({ "item": ["object"] })),
        ("mapGet", json!({ "item": ["vector", "whole"] })),
        ("mapSet", json!({ "item": ["number", "bool"] })),
        ("mapForEach", json!({ "item": ["color"] })),
        ("getMap", json!({ "item": ["text", "object"] })),
        ("mapRemove", json!({ "item": ["unknown"] })),
    ]
}

/// The visual script catalog in every detail, with the pins of the components visual scripts read
/// and of sample nodes whose pins follow their settings, so the C# port is checked kind by kind.
#[test]
fn the_visual_script_catalog_is_as_the_expected_file_says() {
    let components: Vec<Value> = COMPONENTS
        .iter()
        .map(|c| json!({ "name": c.name, "label": c.label, "settable": c.settable(), "fields": c.fields().iter().map(|f| f.name).collect::<Vec<_>>() }))
        .collect();
    let variants: Vec<Value> = variant_samples()
        .into_iter()
        .map(|(kind, values)| {
            let mut graph = Graph::empty(&CATALOG);
            let id = graph.add_node(&CATALOG, kind, [0.0, 0.0]).unwrap();
            for (name, value) in values.as_object().cloned().unwrap_or_default() {
                graph.set_value(&CATALOG, id, &name, value).unwrap();
            }
            let def = graph.node_def(&CATALOG, id).map(|(_, def)| def.spec()).unwrap_or(Value::Null);
            json!({ "kind": kind, "values": values, "def": def })
        })
        .collect();
    let spec = json!({ "catalog": CATALOG.spec(), "components": components, "variants": variants });
    check("script.catalog.json", &(serde_json::to_string_pretty(&spec).unwrap() + "\n"), Path::new("the visual script catalog"));
}
