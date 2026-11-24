//! Variables and properties of a visual script that are lists or maps: the pins their nodes take from what they hold, and the
//! actions and values those nodes compile to. The host keeps each one by its name for this script
//! on this object (`list_*` and `map_*` in `docs/script-abi.md`).

use std::fmt::Write as _;

use bayleaf_graph::{Node, NodeDef, PinDef, PinDefault, PinType, Problem, Shape, VariableType};

use super::{Compiler, Val, as_bool, as_float, as_int, as_object, as_text, as_vec};

/// Variants made so far, by node kind and what the list or map holds.
type Variants = Vec<(String, VariableType, &'static NodeDef)>;
static VARIANTS: std::sync::Mutex<Variants> = std::sync::Mutex::new(Vec::new());

/// What a list or map node holds, from its `item` setting (the type code, kept in step with the
/// declaration by `graph.variable.set` and `graph.property.set`).
fn item_of(node: &Node) -> Option<VariableType> {
    let code = node.values.get("item")?.as_array()?.first()?.as_str()?;
    VariableType::from_code(code)
}

/// The pin of a list or map node's value, of the type it holds.
fn value_pin(ty: VariableType, input: bool) -> PinDef {
    let (pin, default) = match ty {
        VariableType::Number => (PinType::Float, PinDefault::Float(0.0)),
        VariableType::WholeNumber => (PinType::Int, PinDefault::Int(0)),
        VariableType::TrueFalse => (PinType::Bool, PinDefault::Bool(false)),
        VariableType::Vector => (PinType::Vec3, PinDefault::Vec3([0.0; 3])),
        VariableType::Color => (PinType::Color, PinDefault::Color([1.0; 3])),
        VariableType::Text => (PinType::Text, PinDefault::Text("")),
        VariableType::Object => (PinType::Object, PinDefault::None),
    };
    PinDef { name: "value", label: "Value", ty: pin, default: if input { default } else { PinDefault::None } }
}

/// The pins of a list or map node that takes or gives a value: its catalog pins with a Value of the
/// type the list or map holds. `None` for a node that has none, or names nothing yet.
pub(super) fn variant(node: &Node) -> Option<&'static NodeDef> {
    // Where the Value goes: an input at the end, or an output at this place.
    let output_at = match node.kind.as_str() {
        "listAdd" | "listInsert" | "listSet" | "listFind" | "mapSet" => None,
        "listItem" | "mapGet" => Some(0),
        "listForEach" => Some(1),
        "mapForEach" => Some(2),
        _ => return None,
    };
    let ty = item_of(node)?;
    let mut made = VARIANTS.lock().ok()?;
    if let Some((_, _, def)) = made.iter().find(|(kind, t, _)| *kind == node.kind && *t == ty) {
        return Some(def);
    }
    let base = super::CATALOG.nodes.iter().find(|d| d.kind == node.kind)?;
    let leak = |pins: Vec<PinDef>| -> &'static [PinDef] { Box::leak(pins.into_boxed_slice()) };
    let def = match output_at {
        None => {
            let mut inputs = base.inputs.to_vec();
            inputs.push(value_pin(ty, true));
            NodeDef { inputs: leak(inputs), ..*base }
        }
        Some(at) => {
            let mut outputs = base.outputs.to_vec();
            outputs.insert(at.min(outputs.len()), value_pin(ty, false));
            NodeDef { outputs: leak(outputs), ..*base }
        }
    };
    // At most one per kind and type, so the leak is bounded.
    let def: &'static NodeDef = Box::leak(Box::new(def));
    made.push((node.kind.clone(), ty, def));
    Some(def)
}

/// A value as a fresh 16-byte slot, as `event_send` lists values: the slot's address.
fn slot(node: u32, ty: VariableType, value: Val) -> Result<String, Problem> {
    Ok(match ty {
        VariableType::Number => format!("(call $slot_f {})", as_float(node, value)?),
        VariableType::WholeNumber => format!("(call $slot_i (i32.const 1) {})", as_int(node, value)?),
        VariableType::TrueFalse => format!("(call $slot_i (i32.const 2) {})", as_bool(node, value)?),
        VariableType::Object => format!("(call $slot_i (i32.const 5) {})", as_object(node, value)?),
        VariableType::Vector | VariableType::Color => {
            let [x, y, z] = as_vec(node, value)?;
            format!("(call $slot_v {x} {y} {z})")
        }
        VariableType::Text => format!("(call $slot_t {})", as_text(node, value)?),
    })
}

/// The value in the slot at `at` (not a text: a text is read with its own call).
fn from_slot(ty: VariableType, at: &str) -> Val {
    match ty {
        VariableType::Number => Val::F(format!("(f32.load offset=4 {at})")),
        VariableType::WholeNumber => Val::I(format!("(i32.load offset=4 {at})")),
        VariableType::TrueFalse => Val::B(format!("(i32.load offset=4 {at})")),
        VariableType::Object => Val::O(format!("(i32.load offset=4 {at})")),
        VariableType::Vector | VariableType::Color => Val::V([4, 8, 12].map(|o| format!("(f32.load offset={o} {at})"))),
        VariableType::Text => Val::T("(i64.const 0)".to_owned()),
    }
}

/// Whether a node kind takes or gives a value of the type its list or map holds.
fn takes_value(kind: &str) -> bool {
    matches!(kind, "listAdd" | "listInsert" | "listSet" | "listFind" | "mapSet" | "listItem" | "mapGet" | "listForEach" | "mapForEach")
}

/// Whether a node kind works on a map (else on a list).
fn on_map(kind: &str) -> bool {
    kind.starts_with("map")
}

impl Compiler<'_> {
    /// The list or map a node names: a variable or property of the script in that shape. Where its
    /// name is placed, and what it holds. A node out of step with the declaration (made before the
    /// type changed) is a problem, and so is a name both a variable and a property have.
    fn collection(&mut self, node: u32) -> Result<(String, VariableType), Problem> {
        let n = self.graph.node(node).ok_or_else(|| Problem::on(node, "no such node"))?;
        let map = on_map(&n.kind);
        let what = if map { "map" } else { "list" };
        let name = self.setting_text(node, "name", "");
        if name.is_empty() {
            return Err(Problem::on(node, format!("choose the {what}")));
        }
        let variable = self.graph.variable(&name).filter(|v| !v.shape.is_single()).map(|v| (v.ty, v.shape));
        let property = self.graph.property(&name).filter(|p| !p.shape.is_single()).map(|p| (p.ty, p.shape));
        let (item, shape) = match (variable, property) {
            (Some(_), Some(_)) => return Err(Problem::on(node, format!("{name} is both a variable and a property: rename one"))),
            (Some(found), None) | (None, Some(found)) => found,
            (None, None) => return Err(Problem::on(node, format!("this script has no {what} variable or property {name}"))),
        };
        if shape != if map { Shape::Map } else { Shape::List } {
            return Err(Problem::on(node, format!("{name} is a {}, not a {what}", shape.label().to_lowercase())));
        }
        if item_of(n) != Some(item) && (takes_value(&n.kind) || n.values.contains_key("item")) {
            return Err(Problem::on(node, format!("{name} holds {}: choose it again", item.code())));
        }
        let (at, len) = self.text(name.as_bytes());
        Ok((format!("(i32.const {at}) (i32.const {len})"), item))
    }

    /// The value input of a list or map node, as a slot.
    fn value_slot(&mut self, node: u32, ty: VariableType) -> Result<String, Problem> {
        let value = self.input(node, "value")?;
        slot(node, ty, value)
    }

    /// The actions of list and map nodes, but For Each.
    pub(super) fn collection_action(&mut self, node: u32, kind: &str, indent: &str, code: &mut String) -> Result<(), Problem> {
        let (name, ty) = self.collection(node)?;
        let line = match kind {
            "listAdd" => format!("(drop (call $list_put {name} (i32.const 0) (i32.const 0) {}))", self.value_slot(node, ty)?),
            "listInsert" | "listSet" => {
                let index = self.input(node, "index").and_then(|v| as_int(node, v))?;
                let op = if kind == "listInsert" { 1 } else { 2 };
                format!("(drop (call $list_put {name} (i32.const {op}) {index} {}))", self.value_slot(node, ty)?)
            }
            "listRemove" => format!("(drop (call $list_remove {name} {}))", self.input(node, "index").and_then(|v| as_int(node, v))?),
            "listClear" => format!("(call $list_clear {name})"),
            "mapSet" => {
                let key = self.input(node, "key").and_then(|v| as_text(node, v))?;
                format!("(call $map_put_key {name} {key} {})", self.value_slot(node, ty)?)
            }
            "mapRemove" => format!("(call $map_remove_key {name} {})", self.input(node, "key").and_then(|v| as_text(node, v))?),
            "mapClear" => format!("(call $map_clear {name})"),
            _ => return Err(Problem::on(node, "not a list or map action")),
        };
        let _ = writeln!(code, "{indent}{line}");
        Ok(())
    }

    /// For Each in List and For Each in Map: the item (or key) and its value live in globals, so the
    /// nodes in Each can read them; the length is asked again each time round, and a map goes on from
    /// the key it was at.
    pub(super) fn collection_loop(&mut self, node: u32, kind: &str, depth: usize, indent: &str, code: &mut String) -> Result<(), Problem> {
        let (name, ty) = self.collection(node)?;
        let (index, at, text, key) = (format!("$each_index_{node}"), format!("$each_slot_{node}"), format!("$each_text_{node}"), format!("$each_key_{node}"));
        self.global(format!("(global {at} (mut i32) (i32.const 0))"));
        self.global(format!("(global {text} (mut i64) (i64.const 0))"));
        let body = self.chain(node, "each", depth + 2)?;
        // The value of this round, read once.
        let read = if ty == VariableType::Text {
            if kind == "mapForEach" {
                format!("(global.set {text} (call $map_string {name} (global.get {key})))")
            } else {
                format!("(global.set {text} (call $list_string {name} (global.get {index})))")
            }
        } else if kind == "mapForEach" {
            format!("(global.set {at} (call $map_slot {name} (global.get {key})))")
        } else {
            format!("(global.set {at} (call $list_slot {name} (global.get {index})))")
        };
        if kind == "mapForEach" {
            self.global(format!("(global {key} (mut i64) (i64.const 0))"));
            let _ = writeln!(
                code,
                "{indent}(global.set {key} (call $map_after {name} (i64.const 0) (i32.const 1)))\n{indent}(block $done_{node} (loop $next_{node}\n{indent}  (br_if $done_{node} (i64.eq (global.get {key}) (i64.const -1)))\n{indent}  {read}\n{body}{indent}  (global.set {key} (call $map_after {name} (global.get {key}) (i32.const 0)))\n{indent}  (br $next_{node})))"
            );
        } else {
            self.global(format!("(global {index} (mut i32) (i32.const 0))"));
            let _ = writeln!(
                code,
                "{indent}(global.set {index} (i32.const 0))\n{indent}(block $done_{node} (loop $next_{node}\n{indent}  (br_if $done_{node} (i32.ge_s (global.get {index}) (call $list_len {name})))\n{indent}  {read}\n{body}{indent}  (global.set {index} (i32.add (global.get {index}) (i32.const 1)))\n{indent}  (br $next_{node})))"
            );
        }
        Ok(())
    }

    /// The values list and map nodes give.
    pub(super) fn collection_value(&mut self, node: u32, kind: &str, pin: &str) -> Result<Val, Problem> {
        let (name, ty) = self.collection(node)?;
        Ok(match (kind, pin) {
            ("listLength", _) => Val::I(format!("(call $list_len {name})")),
            ("mapCount", _) => Val::I(format!("(call $map_count {name})")),
            ("listFind", _) => Val::I(format!("(call $list_find {name} {})", self.value_slot(node, ty)?)),
            ("listItem", "found") => {
                let index = self.input(node, "index").and_then(|v| as_int(node, v))?;
                Val::B(format!("(call $list_get {name} {index} (call $alloc (i32.const 16)))"))
            }
            ("listItem", _) => {
                let index = self.input(node, "index").and_then(|v| as_int(node, v))?;
                if ty == VariableType::Text {
                    Val::T(format!("(call $list_string {name} {index})"))
                } else {
                    from_slot(ty, &format!("(call $list_slot {name} {index})"))
                }
            }
            ("mapGet", "found") => Val::B(format!("(call $map_has {name} {})", self.input(node, "key").and_then(|v| as_text(node, v))?)),
            ("mapGet", _) => {
                let key = self.input(node, "key").and_then(|v| as_text(node, v))?;
                if ty == VariableType::Text {
                    Val::T(format!("(call $map_string {name} {key})"))
                } else {
                    from_slot(ty, &format!("(call $map_slot {name} {key})"))
                }
            }
            ("listForEach" | "mapForEach", "index") => Val::I(format!("(global.get $each_index_{node})")),
            ("mapForEach", "key") => Val::T(format!("(global.get $each_key_{node})")),
            ("listForEach" | "mapForEach", _) if ty == VariableType::Text => Val::T(format!("(global.get $each_text_{node})")),
            ("listForEach" | "mapForEach", _) => from_slot(ty, &format!("(global.get $each_slot_{node})")),
            _ => return Err(Problem::on(node, format!("no output {pin}"))),
        })
    }
}

/// The helpers, after the imports: a value into a fresh slot, and a list's item or a map's value
/// out of one (all zero when there is none), or as text.
pub(super) const HELPERS: &str = r#"
  (func $slot (param $k i32) (result i32) (local $s i32)
    (local.set $s (call $alloc (i32.const 16)))
    (i32.store (local.get $s) (local.get $k))
    (i64.store offset=4 (local.get $s) (i64.const 0))
    (i32.store offset=12 (local.get $s) (i32.const 0))
    (local.get $s))
  (func $slot_f (param $v f32) (result i32) (local $s i32)
    (local.set $s (call $slot (i32.const 0)))
    (f32.store offset=4 (local.get $s) (local.get $v))
    (local.get $s))
  (func $slot_i (param $k i32) (param $v i32) (result i32) (local $s i32)
    (local.set $s (call $slot (local.get $k)))
    (i32.store offset=4 (local.get $s) (local.get $v))
    (local.get $s))
  (func $slot_v (param $x f32) (param $y f32) (param $z f32) (result i32) (local $s i32)
    (local.set $s (call $slot (i32.const 3)))
    (f32.store offset=4 (local.get $s) (local.get $x))
    (f32.store offset=8 (local.get $s) (local.get $y))
    (f32.store offset=12 (local.get $s) (local.get $z))
    (local.get $s))
  (func $slot_t (param $t i64) (result i32) (local $s i32)
    (local.set $s (call $slot (i32.const 4)))
    (i32.store offset=4 (local.get $s) (call $tp (local.get $t)))
    (i32.store offset=8 (local.get $s) (call $tl (local.get $t)))
    (local.get $s))
  (func $list_slot (param $p i32) (param $l i32) (param $i i32) (result i32) (local $s i32)
    (local.set $s (call $slot (i32.const 0)))
    (drop (call $list_get (local.get $p) (local.get $l) (local.get $i) (local.get $s)))
    (local.get $s))
  (func $list_string (param $p i32) (param $l i32) (param $i i32) (result i64) (local $n i32) (local $b i32)
    (local.set $n (call $list_text (local.get $p) (local.get $l) (local.get $i) (i32.const 0) (i32.const 0)))
    (local.set $b (call $alloc (local.get $n)))
    (drop (call $list_text (local.get $p) (local.get $l) (local.get $i) (local.get $b) (local.get $n)))
    (call $mk (local.get $b) (local.get $n)))
  (func $map_put_key (param $p i32) (param $l i32) (param $k i64) (param $s i32)
    (drop (call $map_put (local.get $p) (local.get $l) (call $tp (local.get $k)) (call $tl (local.get $k)) (local.get $s))))
  (func $map_remove_key (param $p i32) (param $l i32) (param $k i64)
    (drop (call $map_remove (local.get $p) (local.get $l) (call $tp (local.get $k)) (call $tl (local.get $k)))))
  (func $map_has (param $p i32) (param $l i32) (param $k i64) (result i32)
    (call $map_get (local.get $p) (local.get $l) (call $tp (local.get $k)) (call $tl (local.get $k)) (call $alloc (i32.const 16))))
  (func $map_slot (param $p i32) (param $l i32) (param $k i64) (result i32) (local $s i32)
    (local.set $s (call $slot (i32.const 0)))
    (drop (call $map_get (local.get $p) (local.get $l) (call $tp (local.get $k)) (call $tl (local.get $k)) (local.get $s)))
    (local.get $s))
  (func $map_string (param $p i32) (param $l i32) (param $k i64) (result i64) (local $n i32) (local $b i32)
    (local.set $n (call $map_text (local.get $p) (local.get $l) (call $tp (local.get $k)) (call $tl (local.get $k)) (i32.const 0) (i32.const 0)))
    (local.set $b (call $alloc (local.get $n)))
    (drop (call $map_text (local.get $p) (local.get $l) (call $tp (local.get $k)) (call $tl (local.get $k)) (local.get $b) (local.get $n)))
    (call $mk (local.get $b) (local.get $n)))
  ;; The map's first key (when $first), or the first after $k; -1 when there is none.
  (func $map_after (param $p i32) (param $l i32) (param $k i64) (param $first i32) (result i64) (local $kl i32) (local $n i32) (local $b i32)
    (local.set $kl (select (i32.const -1) (call $tl (local.get $k)) (local.get $first)))
    (local.set $n (call $map_next (local.get $p) (local.get $l) (call $tp (local.get $k)) (local.get $kl) (i32.const 0) (i32.const 0)))
    (if (i32.lt_s (local.get $n) (i32.const 0)) (then (return (i64.const -1))))
    (local.set $b (call $alloc (local.get $n)))
    (drop (call $map_next (local.get $p) (local.get $l) (call $tp (local.get $k)) (local.get $kl) (local.get $b) (local.get $n)))
    (call $mk (local.get $b) (local.get $n)))
"#;
