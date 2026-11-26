//! Variables and properties of a visual script that are lists or maps: the pins their nodes take
//! from what they hold (and a map from the type of its keys), and the actions and values those
//! nodes compile to. The host keeps each one by its name for this script on this object (`list_*`
//! and `map_*` in `docs/script-abi.md`).

use std::fmt::Write as _;

use bayleaf_graph::{Node, NodeDef, PinDef, PinDefault, PinType, Problem, Shape, VariableType};

use super::{Compiler, Val, as_bool, as_float, as_int, as_list, as_map, as_object, as_text, as_vec};

/// Variants made so far, by node kind, what the list or map holds, and the map's keys' type.
type Variants = Vec<(String, VariableType, VariableType, &'static NodeDef)>;
static VARIANTS: std::sync::Mutex<Variants> = std::sync::Mutex::new(Vec::new());

/// What a list or map node holds, from its `item` setting (the type codes of the values, then of a
/// map's keys; kept in step with the declaration by `graph.variable.set` and `graph.property.set`).
fn item_of(node: &Node) -> Option<VariableType> {
    let code = node.values.get("item")?.as_array()?.first()?.as_str()?;
    VariableType::from_code(code)
}

/// The type of a map node's keys, from its `item` setting; text when it says none.
fn key_of(node: &Node) -> VariableType {
    let code = node.values.get("item").and_then(|v| v.as_array()?.get(1)?.as_str().map(str::to_owned));
    code.and_then(|c| VariableType::from_code(&c)).unwrap_or(VariableType::Text)
}

/// A pin of one value of `ty`, named `name`, with its type's default when it is an input.
fn typed_pin(name: &'static str, label: &'static str, ty: VariableType, input: bool) -> PinDef {
    let (pin, default) = match ty {
        VariableType::Number => (PinType::Float, PinDefault::Float(0.0)),
        VariableType::WholeNumber => (PinType::Int, PinDefault::Int(0)),
        VariableType::TrueFalse => (PinType::Bool, PinDefault::Bool(false)),
        VariableType::Vector => (PinType::Vec3, PinDefault::Vec3([0.0; 3])),
        VariableType::Color => (PinType::Color, PinDefault::Color([1.0; 3])),
        VariableType::Text => (PinType::Text, PinDefault::Text("")),
        VariableType::Object => (PinType::Object, PinDefault::None),
    };
    PinDef { name, label, ty: pin, default: if input { default } else { PinDefault::None } }
}

/// The pins of a list or map node of the type it holds and, a map's, of the type of its keys (its
/// `item`): the catalog's are for numbers under text keys, and here the list or map pin, the Value
/// and the Key take the types it names.
pub(super) fn variant(node: &Node) -> Option<&'static NodeDef> {
    let ty = item_of(node).unwrap_or(VariableType::Number);
    let key = key_of(node);
    let base = super::CATALOG.nodes.iter().find(|d| d.kind == node.kind)?;
    if ty == VariableType::Number && key == VariableType::Text {
        return Some(base);
    }
    let mut made = VARIANTS.lock().ok()?;
    if let Some((_, _, _, def)) = made.iter().find(|(kind, t, k, _)| *kind == node.kind && *t == ty && *k == key) {
        return Some(def);
    }
    let retyped = |pins: &[PinDef], input: bool| -> &'static [PinDef] {
        let pins: Vec<PinDef> = pins
            .iter()
            .map(|p| match (p.name, p.ty) {
                ("list", PinType::List(_)) => PinDef { ty: PinType::List(ty), ..*p },
                ("map", PinType::Map(..)) => PinDef { ty: PinType::Map(key, ty), ..*p },
                ("value", _) => typed_pin("value", "Value", ty, input),
                ("key", _) if on_map(&node.kind) => typed_pin("key", "Key", key, input),
                _ => *p,
            })
            .collect();
        Box::leak(pins.into_boxed_slice())
    };
    let def = NodeDef { inputs: retyped(base.inputs, true), outputs: retyped(base.outputs, false), ..*base };
    // At most one per kind, type and key type, so the leak is bounded.
    let def: &'static NodeDef = Box::leak(Box::new(def));
    made.push((node.kind.clone(), ty, key, def));
    Some(def)
}

/// What a list or map node should hold for `link` to fit: linking a list of text into a list node
/// makes it a node for lists of text, a map's keys' type comes along too. The node, the value type
/// and the key type; `None` when the link goes elsewhere or fits already.
pub fn adopted_item(graph: &bayleaf_graph::Graph, link: &bayleaf_graph::Link) -> Option<(u32, VariableType, VariableType)> {
    let to = graph.node(link.to.node)?;
    if !super::COLLECTION_KINDS.contains(&to.kind.as_str()) || !matches!(link.to.pin.as_str(), "list" | "map") {
        return None;
    }
    let (_, from) = graph.node_def(&super::CATALOG, link.from.node).ok()?;
    let (item, key) = match (from.output(&link.from.pin)?.ty, link.to.pin.as_str()) {
        (PinType::List(item), "list") => (item, VariableType::Text),
        (PinType::Map(key, item), "map") => (item, key),
        _ => return None,
    };
    let now = (item_of(to).unwrap_or(VariableType::Number), if link.to.pin == "map" { key_of(to) } else { VariableType::Text });
    (now != (item, key)).then_some((to.id, item, key))
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

/// The value in the slot at `at`. A text slot is one a map's walk filled in (where its bytes are,
/// and how many); a list's or map's text value is read with its own call.
fn from_slot(ty: VariableType, at: &str) -> Val {
    match ty {
        VariableType::Number => Val::F(format!("(f32.load offset=4 {at})")),
        VariableType::WholeNumber => Val::I(format!("(i32.load offset=4 {at})")),
        VariableType::TrueFalse => Val::B(format!("(i32.load offset=4 {at})")),
        VariableType::Object => Val::O(format!("(i32.load offset=4 {at})")),
        VariableType::Vector | VariableType::Color => Val::V([4, 8, 12].map(|o| format!("(f32.load offset={o} {at})"))),
        VariableType::Text => Val::T(format!("(call $mk (i32.load offset=4 {at}) (i32.load offset=8 {at}))")),
    }
}

/// Whether a node kind takes or gives a value of the type its list or map holds.
fn takes_value(kind: &str) -> bool {
    matches!(kind, "listAdd" | "listInsert" | "listSet" | "listFind" | "mapSet" | "listItem" | "mapGet" | "listForEach" | "mapForEach" | "getList" | "getMap")
}

/// Whether a node kind works on a map (else on a list).
fn on_map(kind: &str) -> bool {
    kind.starts_with("map") || kind == "getMap"
}

/// A list or map a node works on: where its name is, what it holds, and a map's keys' type.
struct Named {
    name: String,
    item: VariableType,
    key: VariableType,
}

impl Compiler<'_> {
    /// The list or map a node names: a variable or property of the script in that shape. Where its
    /// name is placed, what it holds and the type of a map's keys. A node out of step with the
    /// declaration (made before the type changed) is a problem, and so is a name both a variable and
    /// a property have.
    fn collection(&mut self, node: u32) -> Result<Named, Problem> {
        let n = self.graph.node(node).ok_or_else(|| Problem::on(node, "no such node"))?;
        let map = on_map(&n.kind);
        let what = if map { "map" } else { "list" };
        // One linked in: its name, as the pin brings it, and what the node's pins say it holds.
        if self.graph.link_into(node, what).is_some() {
            let (item, key) = (item_of(n).unwrap_or(VariableType::Number), key_of(n));
            let value = self.input(node, what)?;
            let name = if map { as_map(node, value)? } else { as_list(node, value)? };
            return Ok(Named { name: format!("(call $tp {name}) (call $tl {name})"), item, key });
        }
        let name = self.setting_text(node, "name", "");
        if name.is_empty() {
            return Err(Problem::on(node, format!("link a {what}, or choose one of this script's")));
        }
        let variable = self.graph.variable(&name).filter(|v| !v.shape.is_single()).map(|v| (v.ty, v.shape, v.key));
        let property = self.graph.property(&name).filter(|p| !p.shape.is_single()).map(|p| (p.ty, p.shape, p.key));
        let (item, shape, key) = match (variable, property) {
            (Some(_), Some(_)) => return Err(Problem::on(node, format!("{name} is both a variable and a property: rename one"))),
            (Some(found), None) | (None, Some(found)) => found,
            (None, None) => return Err(Problem::on(node, format!("this script has no {what} variable or property {name}"))),
        };
        if shape != if map { Shape::Map } else { Shape::List } {
            return Err(Problem::on(node, format!("{name} is a {}, not a {what}", shape.label().to_lowercase())));
        }
        let stale = item_of(n) != Some(item) || (map && key_of(n) != key);
        if stale && (takes_value(&n.kind) || n.values.contains_key("item")) {
            let holds = if map { item.shaped_code(Shape::Map, key) } else { item.code().to_owned() };
            return Err(Problem::on(node, format!("{name} holds {holds}: choose it again")));
        }
        let (at, len) = self.text(name.as_bytes());
        Ok(Named { name: format!("(i32.const {at}) (i32.const {len})"), item, key })
    }

    /// The value input of a list or map node, as a slot.
    fn value_slot(&mut self, node: u32, ty: VariableType) -> Result<String, Problem> {
        let value = self.input(node, "value")?;
        slot(node, ty, value)
    }

    /// The key input of a map node, as a slot.
    fn key_slot(&mut self, node: u32, key: VariableType) -> Result<String, Problem> {
        let value = self.input(node, "key")?;
        slot(node, key, value)
    }

    /// The actions of list and map nodes, but For Each.
    pub(super) fn collection_action(&mut self, node: u32, kind: &str, indent: &str, code: &mut String) -> Result<(), Problem> {
        let Named { name, item, key } = self.collection(node)?;
        let line = match kind {
            "listAdd" => format!("(drop (call $list_put {name} (i32.const 0) (i32.const 0) {}))", self.value_slot(node, item)?),
            "listInsert" | "listSet" => {
                let index = self.input(node, "index").and_then(|v| as_int(node, v))?;
                let op = if kind == "listInsert" { 1 } else { 2 };
                format!("(drop (call $list_put {name} (i32.const {op}) {index} {}))", self.value_slot(node, item)?)
            }
            "listRemove" => format!("(drop (call $list_remove {name} {}))", self.input(node, "index").and_then(|v| as_int(node, v))?),
            "listClear" => format!("(call $list_clear {name})"),
            "mapSet" => {
                let at = self.key_slot(node, key)?;
                format!("(drop (call $map_put {name} {at} {}))", self.value_slot(node, item)?)
            }
            "mapRemove" => format!("(drop (call $map_remove {name} {}))", self.key_slot(node, key)?),
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
        let Named { name, item, .. } = self.collection(node)?;
        let (index, at, text, key) = (format!("$each_index_{node}"), format!("$each_slot_{node}"), format!("$each_text_{node}"), format!("$each_key_{node}"));
        self.global(format!("(global {at} (mut i32) (i32.const 0))"));
        self.global(format!("(global {text} (mut i64) (i64.const 0))"));
        let body = self.chain(node, "each", depth + 2)?;
        // The value of this round, read once.
        let read = if item == VariableType::Text {
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
            // The key of this round, as a slot (0 once there are no more).
            self.global(format!("(global {key} (mut i32) (i32.const 0))"));
            let _ = writeln!(
                code,
                "{indent}(global.set {key} (call $map_after {name} (i32.const 0)))\n{indent}(block $done_{node} (loop $next_{node}\n{indent}  (br_if $done_{node} (i32.eqz (global.get {key})))\n{indent}  {read}\n{body}{indent}  (global.set {key} (call $map_after {name} (global.get {key})))\n{indent}  (br $next_{node})))"
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
        let Named { name, item, key } = self.collection(node)?;
        Ok(match (kind, pin) {
            ("getList", _) => Val::L(format!("(call $mk {name})")),
            ("getMap", _) => Val::M(format!("(call $mk {name})")),
            ("listLength", _) => Val::I(format!("(call $list_len {name})")),
            ("mapCount", _) => Val::I(format!("(call $map_count {name})")),
            ("listFind", _) => Val::I(format!("(call $list_find {name} {})", self.value_slot(node, item)?)),
            ("listItem", "found") => {
                let index = self.input(node, "index").and_then(|v| as_int(node, v))?;
                Val::B(format!("(call $list_get {name} {index} (call $alloc (i32.const 16)))"))
            }
            ("listItem", _) => {
                let index = self.input(node, "index").and_then(|v| as_int(node, v))?;
                if item == VariableType::Text {
                    Val::T(format!("(call $list_string {name} {index})"))
                } else {
                    from_slot(item, &format!("(call $list_slot {name} {index})"))
                }
            }
            ("mapGet", "found") => Val::B(format!("(call $map_get {name} {} (call $alloc (i32.const 16)))", self.key_slot(node, key)?)),
            ("mapGet", _) => {
                let at = self.key_slot(node, key)?;
                if item == VariableType::Text {
                    Val::T(format!("(call $map_string {name} {at})"))
                } else {
                    from_slot(item, &format!("(call $map_slot {name} {at})"))
                }
            }
            ("listForEach" | "mapForEach", "index") => Val::I(format!("(global.get $each_index_{node})")),
            ("mapForEach", "key") => from_slot(key, &format!("(global.get $each_key_{node})")),
            ("listForEach" | "mapForEach", _) if item == VariableType::Text => Val::T(format!("(global.get $each_text_{node})")),
            ("listForEach" | "mapForEach", _) => from_slot(item, &format!("(global.get $each_slot_{node})")),
            _ => return Err(Problem::on(node, format!("no output {pin}"))),
        })
    }
}

/// The helpers, after the imports: a value into a fresh slot, a list's item or a map's value out of
/// one (all zero when there is none) or as text, and a map's keys one after another.
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
  (func $map_slot (param $p i32) (param $l i32) (param $k i32) (result i32) (local $s i32)
    (local.set $s (call $slot (i32.const 0)))
    (drop (call $map_get (local.get $p) (local.get $l) (local.get $k) (local.get $s)))
    (local.get $s))
  (func $map_string (param $p i32) (param $l i32) (param $k i32) (result i64) (local $n i32) (local $b i32)
    (local.set $n (call $map_text (local.get $p) (local.get $l) (local.get $k) (i32.const 0) (i32.const 0)))
    (local.set $b (call $alloc (local.get $n)))
    (drop (call $map_text (local.get $p) (local.get $l) (local.get $k) (local.get $b) (local.get $n)))
    (call $mk (local.get $b) (local.get $n)))
  ;; The map's first key (after 0), or the first after the key in slot $after, as a slot of its
  ;; own; a text key's bytes are copied in, so the slot is a whole value. 0 when there is none.
  (func $map_after (param $p i32) (param $l i32) (param $after i32) (result i32) (local $from i32) (local $s i32) (local $n i32) (local $b i32)
    (local.set $from (select (local.get $after) (i32.const -1) (local.get $after)))
    (local.set $s (call $slot (i32.const 0)))
    (if (i32.eqz (call $map_next (local.get $p) (local.get $l) (local.get $from) (local.get $s))) (then (return (i32.const 0))))
    (if (i32.eq (i32.load (local.get $s)) (i32.const 4))
      (then
        (local.set $n (i32.load offset=4 (local.get $s)))
        (local.set $b (call $alloc (local.get $n)))
        (drop (call $map_next_text (local.get $p) (local.get $l) (local.get $from) (local.get $b) (local.get $n)))
        (i32.store offset=4 (local.get $s) (local.get $b))
        (i32.store offset=8 (local.get $s) (local.get $n))))
    (local.get $s))
"#;
