//! L2: node-graph data model shared by the material editor and visual scripting.
//!
//! A [`Graph`] is nodes joined by links between typed pins. What a node kind means, which pins it
//! has and how a graph turns into code belong to a domain (`material` in `bayleaf-render`,
//! `script` in `bayleaf-script`); this crate only knows the shapes, given as a [`Catalog`], and
//! keeps every graph valid: links join an output to a compatible input, a data input has at most
//! one link, a flow output has at most one link, and there are no cycles.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `format` field of graph files.
pub const FORMAT: &str = "bayleaf.graph";
/// Current graph file version. Version 2 added declared `variables`; version 3 moved a visual script's
/// Menu Path node into `menu`; version 4 added `functions`; version 5 added declared `properties`;
/// version 6 added how each property shows in the Inspector (`properties[].inspector`); version 7
/// added declared `events`; version 8 let variables and properties hold lists or maps (`shape`).
/// Older files still read: a Menu Path node becomes `menu`, and a catalog's `upgrade` moves what its
/// own nodes kept (a visual script's Property nodes' defaults become `properties`).
pub const VERSION: u32 = 8;
/// The node kind versions before 3 kept a visual script's menu path in, as its `path` value.
const OLD_MENU_NODE: &str = "menuPath";
/// Longest `menu`, in characters.
pub const MAX_MENU: usize = 256;
/// Most nodes in one graph.
pub const MAX_NODES: usize = 2000;
/// Most links in one graph.
pub const MAX_LINKS: usize = 8000;
/// Longest text value.
pub const MAX_TEXT: usize = 256;
/// Positions are kept inside ±this many canvas units.
pub const MAX_COORD: f32 = 1.0e6;

/// What a pin carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PinType {
    /// Order of actions (visual scripts): "then do this".
    Flow,
    Float,
    Vec3,
    Color,
    Bool,
    /// A number or a vector, decided by what is connected (math nodes).
    Any,
    /// An object in the scene (visual scripts), or none.
    Object,
    /// Text (visual scripts).
    Text,
    /// A whole number (visual scripts): 32 bits, from -2147483648 to 2147483647.
    Int,
    /// A script on an object (visual scripts): which one, on which object, or none. Rust scripts and
    /// visual scripts alike are such components.
    Component,
    /// A list of values of one type (visual scripts): a list variable or property, or one passed on.
    List(VariableType),
    /// A map of values of one type under keys of one type (visual scripts): the key's type, then
    /// the value's.
    Map(VariableType, VariableType),
}

impl PinType {
    /// English name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            PinType::Flow => "Flow",
            PinType::Float => "Number",
            PinType::Vec3 => "Vector",
            PinType::Color => "Color",
            PinType::Bool => "True/False",
            PinType::Any => "Number or Vector",
            PinType::Object => "Object",
            PinType::Text => "Text",
            PinType::Int => "Whole Number",
            PinType::Component => "Component",
            PinType::List(item) => item.list_label(),
            PinType::Map(key, item) => item.map_label(key),
        }
    }

    /// Whether an output of type `self` may feed an input of type `to`. Numbers widen to vectors
    /// and colours; vectors and colours convert into each other. Whether an `Any` output really
    /// fits is checked when the graph is compiled.
    pub fn connects_to(self, to: PinType) -> bool {
        use PinType::{Any, Bool, Color, Component, Float, Flow, Int, List, Map, Object, Text, Vec3};
        match (self, to) {
            // A list or map goes only where one of the same type is taken.
            (List(a), List(b)) => a == b,
            (Map(k, a), Map(l, b)) => k == l && a == b,
            (List(_) | Map(..), _) | (_, List(_) | Map(..)) => false,
            (Flow, Flow) | (Bool, Bool) | (Object, Object) | (Text, Text) | (Int, Int) | (Component, Component) => true,
            (Flow | Bool | Object | Text | Component, _) | (_, Flow | Bool | Object | Text | Component) => false,
            // A whole number widens to a number (and on to a vector); a number becomes whole only
            // through To Whole Number, which says how to round.
            (Int, Float | Vec3 | Color | Any) => true,
            // Math on whole numbers stays whole; the compiler checks what an `Any` output carries.
            (Any, Int) => true,
            (_, Int) => false,
            (Float | Vec3 | Color | Any, Float | Vec3 | Color | Any) => !(matches!(self, Vec3 | Color) && to == Float),
        }
    }
}

/// Default of an input pin that has no link.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PinDefault {
    None,
    Float(f32),
    Vec3([f32; 3]),
    Color([f32; 3]),
    Bool(bool),
    Text(&'static str),
    Int(i32),
    /// Filled in by the domain (for example the surface position in a material).
    Implicit(&'static str),
}

/// One input or output of a node kind.
#[derive(Clone, Copy, Debug)]
pub struct PinDef {
    /// Stable id used in files and commands.
    pub name: &'static str,
    /// English label. It is the translation key.
    pub label: &'static str,
    pub ty: PinType,
    pub default: PinDefault,
}

impl PinDef {
    pub const fn new(name: &'static str, label: &'static str, ty: PinType) -> Self {
        Self { name, label, ty, default: PinDefault::None }
    }

    pub const fn with(name: &'static str, label: &'static str, ty: PinType, default: PinDefault) -> Self {
        Self { name, label, ty, default }
    }
}

/// A value on a node that is not a pin, such as a key name or a constant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SettingKind {
    Float(f32),
    Color([f32; 3]),
    Vec3([f32; 3]),
    /// Free text up to [`MAX_TEXT`] characters.
    Text(&'static str),
    /// Text that names something: letters, digits and `_` only.
    Name(&'static str),
    /// One of a fixed list of words; the first is the default.
    Choice(&'static [&'static str]),
    /// A list of up to 64 names (1 to 64 characters each) the node keeps for itself, such as the
    /// properties of the script a Get Component names. Not shown as a row of the node.
    Names,
}

#[derive(Clone, Copy, Debug)]
pub struct SettingDef {
    pub name: &'static str,
    pub label: &'static str,
    pub kind: SettingKind,
}

impl SettingDef {
    /// Whether the node shows it as a row (a list of names is kept, not shown).
    pub fn shown(&self) -> bool {
        !matches!(self.kind, SettingKind::Names)
    }
}

/// One kind of node in a domain.
#[derive(Clone, Copy, Debug)]
pub struct NodeDef {
    /// Stable id used in files and commands, e.g. `multiply`.
    pub kind: &'static str,
    /// English label. It is the translation key.
    pub label: &'static str,
    /// Group in the add-node menu.
    pub category: &'static str,
    pub description: &'static str,
    pub inputs: &'static [PinDef],
    pub outputs: &'static [PinDef],
    pub settings: &'static [SettingDef],
    /// At most one node of this kind per graph (for example the material output).
    pub unique: bool,
}

impl NodeDef {
    pub fn input(&self, name: &str) -> Option<&PinDef> {
        self.inputs.iter().find(|p| p.name == name)
    }

    pub fn output(&self, name: &str) -> Option<&PinDef> {
        self.outputs.iter().find(|p| p.name == name)
    }

    pub fn setting(&self, name: &str) -> Option<&SettingDef> {
        self.settings.iter().find(|s| s.name == name)
    }
}

/// The node kinds of one domain.
#[derive(Clone, Copy, Debug)]
pub struct Catalog {
    /// `material` or `script`.
    pub domain: &'static str,
    /// English name of graphs in this domain, e.g. "Material".
    pub label: &'static str,
    /// File extension of this domain's graphs, without the dot.
    pub extension: &'static str,
    pub nodes: &'static [NodeDef],
    /// Kinds a new graph starts with, and where.
    pub starter: &'static [(&'static str, [f32; 2])],
    /// A node's own pins when they depend on its settings (Get Component shows the fields of the
    /// component it names); `None` for the kind's pins. See `no_variant`.
    pub variant: fn(&Node) -> Option<&'static NodeDef>,
    /// Moves what this domain's nodes kept in files older than [`VERSION`] (the version read is
    /// given) into where it belongs now. See `no_upgrade`.
    pub upgrade: fn(&mut Graph, u32),
    /// Kinds whose number-or-vector output is a number even when every input is a whole number
    /// (Sine, Square Root). Other kinds keep whole numbers whole. See [`Graph::output_type`].
    pub fractional: &'static [&'static str],
}

/// For catalogs whose files need nothing moved between versions.
pub fn no_upgrade(_: &mut Graph, _: u32) {}

/// For catalogs whose node kinds always have the same pins.
pub fn no_variant(_: &Node) -> Option<&'static NodeDef> {
    None
}

impl Catalog {
    pub fn get(&self, kind: &str) -> Option<&NodeDef> {
        self.nodes.iter().find(|n| n.kind == kind)
    }

    /// The definition a node is drawn and compiled with: its variant, else its kind's.
    pub fn def_of(&self, node: &Node) -> Option<&NodeDef> {
        (self.variant)(node).or_else(|| self.get(&node.kind))
    }

    /// The catalog's description as JSON (for `graph.catalog` and agents).
    pub fn to_json(&self) -> Value {
        let pin = |p: &PinDef| serde_json::json!({ "name": p.name, "label": p.label, "type": p.ty });
        let nodes: Vec<Value> = self
            .nodes
            .iter()
            .map(|n| {
                serde_json::json!({
                    "kind": n.kind,
                    "label": n.label,
                    "category": n.category,
                    "description": n.description,
                    "inputs": n.inputs.iter().map(pin).collect::<Vec<_>>(),
                    "outputs": n.outputs.iter().map(pin).collect::<Vec<_>>(),
                    "settings": n.settings.iter().map(|s| serde_json::json!({ "name": s.name, "label": s.label })).collect::<Vec<_>>(),
                })
            })
            .collect();
        serde_json::json!({ "domain": self.domain, "label": self.label, "extension": self.extension, "nodes": nodes })
    }
}

/// One end of a link.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinRef {
    pub node: u32,
    pub pin: String,
}

impl PinRef {
    pub fn new(node: u32, pin: &str) -> Self {
        Self { node, pin: pin.to_owned() }
    }
}

/// A link from an output pin to an input pin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub from: PinRef,
    pub to: PinRef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: u32,
    pub kind: String,
    /// Top-left corner on the canvas.
    pub position: [f32; 2],
    /// Settings and the values of inputs without a link, by name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Value>,
}

/// Why an edit or a file was refused. Messages are shown to the user.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum GraphError {
    #[error("there is no node {0}")]
    NoNode(u32),
    #[error("there is no comment {0}")]
    NoComment(u32),
    #[error("there is no node kind \"{0}\" in {1} graphs")]
    UnknownKind(String, &'static str),
    #[error("{node} has no {side} \"{pin}\"")]
    NoPin { node: String, side: &'static str, pin: String },
    #[error("{0}")]
    Refused(String),
    #[error("not a graph file: {0}")]
    Format(String),
}

/// `format` of copied nodes on the clipboard.
pub const FRAGMENT_FORMAT: &str = "bayleaf.nodes";

/// Copied nodes and the links between them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fragment {
    pub format: String,
    pub domain: String,
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub links: Vec<Link>,
}

/// What a paste added.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pasted {
    /// The new nodes.
    pub ids: Vec<u32>,
    /// Nodes left out because the graph may have only one of them (their labels).
    pub skipped: Vec<String>,
}

/// Most comments in one graph.
pub const MAX_COMMENTS: usize = 500;
/// Most characters in one comment.
pub const MAX_COMMENT_TEXT: usize = 2000;
/// Smallest comment box, in canvas units.
pub const MIN_COMMENT_SIZE: [f32; 2] = [120.0, 60.0];
/// Size of a new comment box when none is given.
pub const COMMENT_SIZE: [f32; 2] = [400.0, 240.0];

/// A note on the canvas: a titled box drawn behind the nodes it groups. It does not change what
/// the graph compiles to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Comment {
    pub id: u32,
    pub text: String,
    /// Top-left corner on the canvas.
    pub position: [f32; 2],
    pub size: [f32; 2],
}

/// A graph of one domain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Graph {
    pub format: String,
    pub version: u32,
    pub domain: String,
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub links: Vec<Link>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<Comment>,
    /// Variables a visual script declares, with what they start at.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<Variable>,
    /// Where Add Component lists a visual script (`Light/Flicker`); empty for under Scripts.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub menu: String,
    /// Functions a visual script declares: what each takes and gives back.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub functions: Vec<Function>,
    /// Events a visual script declares: what each brings. Any script may trigger them on an object.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<Event>,
    /// Properties a visual script declares: values the Inspector sets per object, with defaults.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<Property>,
}

/// A property a visual script declares: a value the Inspector sets per object, its type and the
/// default it has where the object sets none. An object property's default is none (`null`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Property {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: VariableType,
    /// One value, a list or a map; one value when left out.
    #[serde(default, skip_serializing_if = "Shape::is_single")]
    pub shape: Shape,
    /// A map's keys' type; text when left out.
    #[serde(default = "text_key", skip_serializing_if = "is_text_key")]
    pub key: VariableType,
    pub default: Value,
    /// How the Inspector shows it; empty for a plain row.
    #[serde(default, skip_serializing_if = "PropertyOptions::is_empty")]
    pub inspector: PropertyOptions,
}

/// How a visual script's property shows in the Inspector, as a Rust script's layout says it (see
/// `docs/script-abi.md`): its label and hint, a group it goes in, a range and slider, read-only, a
/// list of choices, and a condition on another property.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PropertyOptions {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hint: String,
    /// The group it shows in, by title; properties of the same group show together.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub slider: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
    /// Shown only while the property named first has the value second.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_if: Option<(String, Value)>,
}

impl PropertyOptions {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Most properties a graph declares.
pub const MAX_PROPERTIES: usize = 256;

/// What a variable holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VariableType {
    Number,
    Vector,
    TrueFalse,
    /// A 32-bit whole number.
    WholeNumber,
    Text,
    /// An object of the scene; it starts as none and is set while the game plays.
    Object,
    Color,
}

impl VariableType {
    pub const ALL: [VariableType; 7] = [
        VariableType::Number,
        VariableType::WholeNumber,
        VariableType::Vector,
        VariableType::TrueFalse,
        VariableType::Text,
        VariableType::Object,
        VariableType::Color,
    ];

    /// The word nodes keep a pin's type as (`name:type`): `number`, `vector`, `bool`, `whole`,
    /// `text`, `object` or `color`.
    pub fn code(self) -> &'static str {
        match self {
            VariableType::Number => "number",
            VariableType::Vector => "vector",
            VariableType::TrueFalse => "bool",
            VariableType::WholeNumber => "whole",
            VariableType::Text => "text",
            VariableType::Object => "object",
            VariableType::Color => "color",
        }
    }

    pub fn from_code(code: &str) -> Option<VariableType> {
        VariableType::ALL.into_iter().find(|t| t.code() == code)
    }

    /// English name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            VariableType::Number => "Number",
            VariableType::Vector => "Vector",
            VariableType::TrueFalse => "True/False",
            VariableType::WholeNumber => "Whole Number",
            VariableType::Text => "Text",
            VariableType::Object => "Object",
            VariableType::Color => "Color",
        }
    }

    /// What a new variable of this type starts at.
    pub fn zero(self) -> Value {
        match self {
            VariableType::Number => Value::from(0.0),
            VariableType::Vector => serde_json::json!([0.0, 0.0, 0.0]),
            VariableType::TrueFalse => Value::Bool(false),
            VariableType::WholeNumber => Value::from(0),
            VariableType::Text => Value::from(""),
            VariableType::Object => Value::Null,
            VariableType::Color => serde_json::json!([1.0, 1.0, 1.0]),
        }
    }

    /// Whether `value` fits this type: a finite number, three finite numbers (a vector or a colour),
    /// true or false, text of at most [`MAX_TEXT`] characters, or none for an object.
    pub fn fits(self, value: &Value) -> bool {
        let number = |v: &Value| v.as_f64().is_some_and(|f| f.is_finite() && f.abs() < f64::from(f32::MAX));
        match self {
            VariableType::Number => number(value),
            VariableType::Vector | VariableType::Color => value.as_array().is_some_and(|a| a.len() == 3 && a.iter().all(number)),
            VariableType::TrueFalse => value.is_boolean(),
            VariableType::WholeNumber => value.as_i64().is_some_and(|n| i32::try_from(n).is_ok()),
            VariableType::Text => value.as_str().is_some_and(|t| t.chars().count() <= MAX_TEXT),
            VariableType::Object => value.is_null(),
        }
    }

    /// "List of Number", and so on: a list pin's name shown to people.
    pub fn list_label(self) -> &'static str {
        match self {
            VariableType::Number => "List of Number",
            VariableType::Vector => "List of Vector",
            VariableType::TrueFalse => "List of True/False",
            VariableType::WholeNumber => "List of Whole Number",
            VariableType::Text => "List of Text",
            VariableType::Object => "List of Object",
            VariableType::Color => "List of Color",
        }
    }

    /// "Map from Text to Number", and so on: a map pin's name, its keys' type (`key`) first.
    pub fn map_label(self, key: VariableType) -> &'static str {
        static LABELS: std::sync::OnceLock<Vec<(VariableType, VariableType, &'static str)>> = std::sync::OnceLock::new();
        let labels = LABELS.get_or_init(|| {
            let all = VariableType::ALL;
            all.iter()
                .flat_map(|k| all.iter().map(move |v| (*k, *v)))
                .map(|(k, v)| (k, v, &*Box::leak(format!("Map from {} to {}", k.label(), v.label()).into_boxed_str())))
                .collect()
        });
        labels.iter().find(|(k, v, _)| *k == key && *v == self).map_or("Map", |(_, _, label)| label)
    }

    /// The pin type of this type in `shape`: one value's, or a list's or map's of it (a map's keys
    /// of type `key`).
    pub fn pin(self, shape: Shape, key: VariableType) -> PinType {
        match shape {
            Shape::List => PinType::List(self),
            Shape::Map => PinType::Map(key, self),
            Shape::Single => match self {
                VariableType::Number => PinType::Float,
                VariableType::Vector => PinType::Vec3,
                VariableType::TrueFalse => PinType::Bool,
                VariableType::WholeNumber => PinType::Int,
                VariableType::Text => PinType::Text,
                VariableType::Object => PinType::Object,
                VariableType::Color => PinType::Color,
            },
        }
    }

    /// The code of this type in `shape`, as pins and descriptions list it: `number`, `list:number`,
    /// or a map's keys' type then its values' (`map:whole:number`).
    pub fn shaped_code(self, shape: Shape, key: VariableType) -> String {
        match shape {
            Shape::Single => self.code().to_owned(),
            Shape::List => format!("list:{}", self.code()),
            Shape::Map => format!("map:{}:{}", key.code(), self.code()),
        }
    }

    /// The type, shape and key type a code names: `number`, `list:number`, `map:whole:number`, or
    /// `map:number` for a map with text keys. The key type is text for all but a map.
    pub fn from_shaped_code(code: &str) -> Option<(VariableType, Shape, VariableType)> {
        let text = VariableType::Text;
        match code.split(':').collect::<Vec<_>>().as_slice() {
            [one] => Some((VariableType::from_code(one)?, Shape::Single, text)),
            ["list", item] => Some((VariableType::from_code(item)?, Shape::List, text)),
            ["map", item] => Some((VariableType::from_code(item)?, Shape::Map, text)),
            ["map", key, item] => Some((VariableType::from_code(item)?, Shape::Map, VariableType::from_code(key)?)),
            _ => None,
        }
    }

    /// Whether `key`, a map key as files keep it (always text), is a key of this type: text of at
    /// most [`MAX_TEXT`] characters, a finite number, a whole number, `true` or `false`, three
    /// numbers `x,y,z`, or an object's name.
    pub fn fits_key(self, key: &str) -> bool {
        let number = |t: &str| t.trim().parse::<f32>().is_ok_and(f32::is_finite);
        match self {
            VariableType::Text | VariableType::Object => key.chars().count() <= MAX_TEXT,
            VariableType::Number => number(key),
            VariableType::WholeNumber => key.trim().parse::<i32>().is_ok(),
            VariableType::TrueFalse => matches!(key, "true" | "false"),
            VariableType::Vector | VariableType::Color => {
                let parts: Vec<&str> = key.split(',').collect();
                parts.len() == 3 && parts.iter().all(|p| number(p))
            }
        }
    }

    /// A map key of this type that no entry has yet (`taken` says which are): `key`, `key2`… for
    /// text and objects, `0`, `1`… for numbers, `false` then `true`, `0,0,0`, `1,0,0`… for vectors.
    pub fn fresh_key(self, taken: impl Fn(&str) -> bool) -> Option<String> {
        let candidates: Vec<String> = match self {
            VariableType::Text | VariableType::Object => (1..=MAX_ITEMS + 1).map(|n| if n == 1 { "key".to_owned() } else { format!("key{n}") }).collect(),
            VariableType::Number | VariableType::WholeNumber => (0..=MAX_ITEMS).map(|n| n.to_string()).collect(),
            VariableType::TrueFalse => vec!["false".to_owned(), "true".to_owned()],
            VariableType::Vector | VariableType::Color => (0..=MAX_ITEMS).map(|n| format!("{n},0,0")).collect(),
        };
        candidates.into_iter().find(|k| !taken(k))
    }

    /// What a new variable of this type and shape starts at: the type's zero, or an empty list or map.
    pub fn zero_of(self, shape: Shape) -> Value {
        match shape {
            Shape::Single => self.zero(),
            Shape::List => Value::Array(Vec::new()),
            Shape::Map => Value::Object(serde_json::Map::new()),
        }
    }

    /// Whether `value` fits this type in `shape`: one value that [`fits`](Self::fits), or a list
    /// (an array) or map (an object, its keys of type `key` as [`fits_key`](Self::fits_key) reads them) of at most
    /// [`MAX_ITEMS`] of them.
    pub fn fits_shape(self, shape: Shape, key: VariableType, value: &Value) -> bool {
        match shape {
            Shape::Single => self.fits(value),
            Shape::List => value.as_array().is_some_and(|items| items.len() <= MAX_ITEMS && items.iter().all(|v| self.fits(v))),
            Shape::Map => value.as_object().is_some_and(|entries| entries.len() <= MAX_ITEMS && entries.iter().all(|(k, v)| key.fits_key(k) && self.fits(v))),
        }
    }
}

/// One input or output of a function, or a value an event brings: a name and what it holds (one
/// value, or a list or map of them).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Param {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: VariableType,
    /// One value, a list or a map; one value when left out.
    #[serde(default, skip_serializing_if = "Shape::is_single")]
    pub shape: Shape,
    /// A map's keys' type; text when left out.
    #[serde(default = "text_key", skip_serializing_if = "is_text_key")]
    pub key: VariableType,
}

impl Param {
    /// One value of `ty` named `name`.
    pub fn new(name: impl Into<String>, ty: VariableType) -> Self {
        Self { name: name.into(), ty, shape: Shape::Single, key: VariableType::Text }
    }
}

/// A function a visual script declares. Its Function node begins its actions with the inputs as
/// values, a Return node gives back the outputs, and Call Function nodes run it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Function {
    pub name: String,
    #[serde(default)]
    pub inputs: Vec<Param>,
    #[serde(default)]
    pub outputs: Vec<Param>,
}

/// An event a visual script declares: its name and what it brings. Its On Event nodes run when a
/// script, the interface or the engine triggers it on the script's object; Trigger Event nodes
/// trigger it, here or on another object.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Event {
    pub name: String,
    #[serde(default)]
    pub params: Vec<Param>,
}

/// Whether a variable or property holds one value, a list of them (in order, by index from 0) or a
/// map of them (under text keys, in the keys' order).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Shape {
    #[default]
    Single,
    List,
    Map,
}

impl Shape {
    pub const ALL: [Shape; 3] = [Shape::Single, Shape::List, Shape::Map];

    pub fn is_single(&self) -> bool {
        *self == Shape::Single
    }

    pub fn label(self) -> &'static str {
        match self {
            Shape::Single => "Single Value",
            Shape::List => "List",
            Shape::Map => "Map",
        }
    }

    /// The word a description uses before a type code (`list:number`); none for one value.
    pub fn code(self) -> &'static str {
        match self {
            Shape::Single => "single",
            Shape::List => "list",
            Shape::Map => "map",
        }
    }
}

/// What a map's keys are when a file does not say: text.
fn text_key() -> VariableType {
    VariableType::Text
}

fn is_text_key(key: &VariableType) -> bool {
    *key == VariableType::Text
}

/// Most items a list or map starts with: a variable's start, a property's default.
pub const MAX_ITEMS: usize = 1024;

/// Most events a graph declares.
pub const MAX_EVENTS: usize = 64;

/// Most functions a graph declares, and most inputs or outputs one has.
pub const MAX_FUNCTIONS: usize = 64;
pub const MAX_PARAMS: usize = 16;

/// Pin names a function's inputs and outputs cannot take: the flow pins of its nodes.
const FLOW_PINS: [&str; 2] = ["run", "then"];

/// The pins a list of params becomes, as nodes keep them: `name:type`.
pub fn param_pins(params: &[Param]) -> Value {
    Value::from(params.iter().map(|p| format!("{}:{}", p.name, p.ty.shaped_code(p.shape, p.key))).collect::<Vec<_>>())
}

/// A function's inputs or outputs: at most [`MAX_PARAMS`], each named like a variable, no name twice
/// and none a flow pin's.
fn check_params(function: &str, params: &[Param]) -> Result<(), String> {
    if params.len() > MAX_PARAMS {
        return Err(format!("a function has at most {MAX_PARAMS} inputs and {MAX_PARAMS} outputs"));
    }
    for (index, param) in params.iter().enumerate() {
        check_variable_name(&param.name)?;
        if FLOW_PINS.contains(&param.name.as_str()) {
            return Err(format!("{function}: {} is the name of a flow pin", param.name));
        }
        if params[..index].iter().any(|p| p.name == param.name) {
            return Err(format!("{function}: {} is there twice", param.name));
        }
    }
    Ok(())
}

/// A variable a visual script declares: its name, what it holds, and what it starts at.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Variable {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: VariableType,
    /// One value, a list or a map; one value when left out.
    #[serde(default, skip_serializing_if = "Shape::is_single")]
    pub shape: Shape,
    /// A map's keys' type; text when left out.
    #[serde(default = "text_key", skip_serializing_if = "is_text_key")]
    pub key: VariableType,
    pub start: Value,
}

/// Most variables a graph declares.
pub const MAX_VARIABLES: usize = 256;

/// A variable name: 1 to 64 letters, digits or `_`.
pub fn check_variable_name(name: &str) -> Result<(), String> {
    if !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Ok(())
    } else {
        Err(format!("variable name {name:?} must be 1 to 64 letters, digits or _"))
    }
}

impl Graph {
    pub fn variable(&self, name: &str) -> Option<&Variable> {
        self.variables.iter().find(|v| v.name == name)
    }

    /// Declares a variable; `start` defaults to the type's zero.
    pub fn add_variable(&mut self, name: &str, ty: VariableType, start: Option<Value>) -> Result<(), GraphError> {
        check_variable_name(name).map_err(GraphError::Refused)?;
        if self.variable(name).is_some() {
            return Err(GraphError::Refused(format!("there is already a variable {name}")));
        }
        if self.variables.len() >= MAX_VARIABLES {
            return Err(GraphError::Refused(format!("a graph declares at most {MAX_VARIABLES} variables")));
        }
        let start = start.unwrap_or_else(|| ty.zero());
        if !ty.fits(&start) {
            return Err(GraphError::Refused(format!("a {} variable cannot start at {start}", ty.label())));
        }
        self.variables.push(Variable { name: name.to_owned(), ty, shape: Shape::Single, key: VariableType::Text, start });
        Ok(())
    }

    /// Changes a variable's start value, or renames it; nodes of `kinds` that name it by their
    /// `name` setting follow the new name.
    pub fn set_variable(&mut self, name: &str, rename: Option<&str>, start: Option<Value>, kinds: &[&str]) -> Result<(), GraphError> {
        let index = self.variables.iter().position(|v| v.name == name).ok_or_else(|| GraphError::Refused(format!("there is no variable {name}")))?;
        if let Some(start) = start {
            let (ty, shape, key) = (self.variables[index].ty, self.variables[index].shape, self.variables[index].key);
            if !ty.fits_shape(shape, key, &start) {
                return Err(GraphError::Refused(format!("a {} variable cannot start at {start}", ty.label())));
            }
            self.variables[index].start = start;
        }
        if let Some(new) = rename.filter(|n| *n != name) {
            check_variable_name(new).map_err(GraphError::Refused)?;
            if self.variable(new).is_some() {
                return Err(GraphError::Refused(format!("there is already a variable {new}")));
            }
            self.variables[index].name = new.to_owned();
            for node in self.nodes.iter_mut().filter(|n| kinds.contains(&n.kind.as_str())) {
                if node.values.get("name").and_then(Value::as_str) == Some(name) {
                    node.values.insert("name".to_owned(), Value::from(new));
                }
            }
        }
        Ok(())
    }

    pub fn function(&self, name: &str) -> Option<&Function> {
        self.functions.iter().find(|f| f.name == name)
    }

    /// Declares a function with no inputs or outputs.
    pub fn add_function(&mut self, name: &str) -> Result<(), GraphError> {
        check_variable_name(name).map_err(|_| GraphError::Refused(format!("function name {name:?} must be 1 to 64 letters, digits or _")))?;
        if self.function(name).is_some() {
            return Err(GraphError::Refused(format!("there is already a function {name}")));
        }
        if self.functions.len() >= MAX_FUNCTIONS {
            return Err(GraphError::Refused(format!("a graph declares at most {MAX_FUNCTIONS} functions")));
        }
        self.functions.push(Function { name: name.to_owned(), inputs: Vec::new(), outputs: Vec::new() });
        Ok(())
    }

    /// Renames a function or changes its inputs or outputs. Nodes of `kinds` that name it (their
    /// `function` setting) follow: the new name, and their `inputs` and `outputs` pins; links to
    /// pins that went away go too.
    pub fn set_function(
        &mut self,
        catalog: &Catalog,
        name: &str,
        rename: Option<&str>,
        inputs: Option<Vec<Param>>,
        outputs: Option<Vec<Param>>,
        kinds: &[&str],
    ) -> Result<(), GraphError> {
        let mut function = self.function(name).cloned().ok_or_else(|| GraphError::Refused(format!("there is no function {name}")))?;
        if let Some(new) = rename.filter(|n| *n != name) {
            check_variable_name(new).map_err(|_| GraphError::Refused(format!("function name {new:?} must be 1 to 64 letters, digits or _")))?;
            if self.function(new).is_some() {
                return Err(GraphError::Refused(format!("there is already a function {new}")));
            }
            function.name = new.to_owned();
        }
        if let Some(inputs) = inputs {
            check_params(name, &inputs).map_err(GraphError::Refused)?;
            function.inputs = inputs;
        }
        if let Some(outputs) = outputs {
            check_params(name, &outputs).map_err(GraphError::Refused)?;
            function.outputs = outputs;
        }
        // Try it on a copy, so a refused node change leaves the graph as it was.
        let mut trial = self.clone();
        if let Some(slot) = trial.functions.iter_mut().find(|f| f.name == name) {
            *slot = function.clone();
        }
        let named: Vec<u32> = trial
            .nodes
            .iter()
            .filter(|n| kinds.contains(&n.kind.as_str()) && n.values.get("function").and_then(Value::as_str) == Some(name))
            .map(|n| n.id)
            .collect();
        for id in named {
            trial.set_value(catalog, id, "inputs", param_pins(&function.inputs))?;
            trial.set_value(catalog, id, "outputs", param_pins(&function.outputs))?;
            trial.drop_unfit_links(catalog, id);
            trial.set_value(catalog, id, "function", Value::from(function.name.as_str()))?;
        }
        *self = trial;
        Ok(())
    }

    pub fn event(&self, name: &str) -> Option<&Event> {
        self.events.iter().find(|e| e.name == name)
    }

    /// Declares an event that brings nothing.
    pub fn add_event(&mut self, name: &str) -> Result<(), GraphError> {
        check_variable_name(name).map_err(|_| GraphError::Refused(format!("event name {name:?} must be 1 to 64 letters, digits or _")))?;
        if self.event(name).is_some() {
            return Err(GraphError::Refused(format!("there is already an event {name}")));
        }
        if self.events.len() >= MAX_EVENTS {
            return Err(GraphError::Refused(format!("a graph declares at most {MAX_EVENTS} events")));
        }
        self.events.push(Event { name: name.to_owned(), params: Vec::new() });
        Ok(())
    }

    /// Renames an event or changes what it brings. Nodes of `kinds` that name it (their `event`
    /// setting) follow: the new name, and their `params` pins; links to pins that went away go too.
    pub fn set_event(&mut self, catalog: &Catalog, name: &str, rename: Option<&str>, params: Option<Vec<Param>>, kinds: &[&str]) -> Result<(), GraphError> {
        let mut event = self.event(name).cloned().ok_or_else(|| GraphError::Refused(format!("there is no event {name}")))?;
        if let Some(new) = rename.filter(|n| *n != name) {
            check_variable_name(new).map_err(|_| GraphError::Refused(format!("event name {new:?} must be 1 to 64 letters, digits or _")))?;
            if self.event(new).is_some() {
                return Err(GraphError::Refused(format!("there is already an event {new}")));
            }
            event.name = new.to_owned();
        }
        if let Some(params) = params {
            check_params(name, &params).map_err(GraphError::Refused)?;
            event.params = params;
        }
        let mut trial = self.clone();
        if let Some(slot) = trial.events.iter_mut().find(|e| e.name == name) {
            *slot = event.clone();
        }
        let named: Vec<u32> = trial
            .nodes
            .iter()
            .filter(|n| kinds.contains(&n.kind.as_str()) && n.values.get("event").and_then(Value::as_str) == Some(name))
            .map(|n| n.id)
            .collect();
        for id in named {
            trial.set_value(catalog, id, "params", param_pins(&event.params))?;
            trial.drop_unfit_links(catalog, id);
            trial.set_value(catalog, id, "event", Value::from(event.name.as_str()))?;
        }
        *self = trial;
        Ok(())
    }

    /// Gives a variable another type, shape or key type (a map's); its start stays where it still fits, else it goes
    /// back to the new type's zero (an empty list or map). Returns the old type and shape. Its
    /// nodes are the caller's to follow (they depend on the domain's node kinds).
    pub fn retype_variable(&mut self, name: &str, ty: VariableType, shape: Shape, key: VariableType) -> Result<(VariableType, Shape), GraphError> {
        let variable = self.variables.iter_mut().find(|v| v.name == name).ok_or_else(|| GraphError::Refused(format!("there is no variable {name}")))?;
        let old = (variable.ty, variable.shape);
        (variable.ty, variable.shape, variable.key) = (ty, shape, key);
        if !ty.fits_shape(shape, key, &variable.start) {
            variable.start = ty.zero_of(shape);
        }
        Ok(old)
    }

    /// Gives a property another type or shape, as [`Graph::retype_variable`] does a variable.
    pub fn retype_property(&mut self, name: &str, ty: VariableType, shape: Shape, key: VariableType) -> Result<(VariableType, Shape), GraphError> {
        let property = self.properties.iter_mut().find(|p| p.name == name).ok_or_else(|| GraphError::Refused(format!("there is no property {name}")))?;
        let old = (property.ty, property.shape);
        (property.ty, property.shape, property.key) = (ty, shape, key);
        if !ty.fits_shape(shape, key, &property.default) {
            property.default = ty.zero_of(shape);
        }
        Ok(old)
    }

    /// Makes node `id` a node of another kind in place: it keeps its id, place and the values the
    /// new kind still has, and loses the links the new kind's pins no longer take.
    pub fn change_kind(&mut self, catalog: &Catalog, id: u32, kind: &str) -> Result<(), GraphError> {
        catalog.get(kind).ok_or_else(|| GraphError::UnknownKind(kind.to_owned(), catalog.domain))?;
        let mut trial = self.clone();
        trial.node_mut(id)?.kind = kind.to_owned();
        let def = trial.node(id).and_then(|n| catalog.def_of(n)).ok_or_else(|| GraphError::UnknownKind(kind.to_owned(), catalog.domain))?;
        trial.node_mut(id)?.values.retain(|name, _| def.input(name).is_some() || def.setting(name).is_some());
        trial.drop_unfit_links(catalog, id);
        *self = trial;
        Ok(())
    }

    /// The nodes of `kinds` that name `name` (their `name` setting) keep the type codes of what it
    /// holds and of its keys in `item` (`["number", "text"]`); when they change, links that no
    /// longer fit go.
    pub fn follow_item(&mut self, catalog: &Catalog, name: &str, item: VariableType, key: VariableType, kinds: &[&str]) -> Result<(), GraphError> {
        let code = Value::from(vec![item.code(), key.code()]);
        let named: Vec<u32> = self
            .nodes
            .iter()
            .filter(|n| kinds.contains(&n.kind.as_str()) && n.values.get("name").and_then(Value::as_str) == Some(name))
            .map(|n| n.id)
            .collect();
        let mut trial = self.clone();
        for id in named {
            if trial.value(id, "item") != Some(&code) {
                trial.set_value(catalog, id, "item", code.clone())?;
                trial.drop_unfit_links(catalog, id);
            }
        }
        *self = trial;
        Ok(())
    }

    /// Takes an event away; nodes naming it stay (a Trigger Event may name another script's event).
    pub fn remove_event(&mut self, name: &str) -> Result<(), GraphError> {
        let before = self.events.len();
        self.events.retain(|e| e.name != name);
        if self.events.len() == before {
            return Err(GraphError::Refused(format!("there is no event {name}")));
        }
        Ok(())
    }

    /// Takes a function away; nodes naming it stay, and the compiler says they name no function.
    pub fn remove_function(&mut self, name: &str) -> Result<(), GraphError> {
        let before = self.functions.len();
        self.functions.retain(|f| f.name != name);
        if self.functions.len() == before {
            return Err(GraphError::Refused(format!("there is no function {name}")));
        }
        Ok(())
    }

    pub fn property(&self, name: &str) -> Option<&Property> {
        self.properties.iter().find(|p| p.name == name)
    }

    /// Declares a property; `default` defaults to the type's zero.
    pub fn add_property(&mut self, name: &str, ty: VariableType, default: Option<Value>) -> Result<(), GraphError> {
        check_variable_name(name).map_err(|_| GraphError::Refused(format!("property name {name:?} must be 1 to 64 letters, digits or _")))?;
        if self.property(name).is_some() {
            return Err(GraphError::Refused(format!("there is already a property {name}")));
        }
        if self.properties.len() >= MAX_PROPERTIES {
            return Err(GraphError::Refused(format!("a graph declares at most {MAX_PROPERTIES} properties")));
        }
        let default = default.unwrap_or_else(|| ty.zero());
        if !ty.fits(&default) {
            return Err(GraphError::Refused(format!("a {} property cannot default to {default}", ty.label())));
        }
        self.properties.push(Property {
            name: name.to_owned(),
            ty,
            shape: Shape::Single,
            key: VariableType::Text,
            default,
            inspector: PropertyOptions::default(),
        });
        Ok(())
    }

    /// Changes a property's default, or renames it; nodes of `kinds` that name it by their `name`
    /// setting follow the new name.
    pub fn set_property(&mut self, name: &str, rename: Option<&str>, default: Option<Value>, kinds: &[&str]) -> Result<(), GraphError> {
        let index = self.properties.iter().position(|p| p.name == name).ok_or_else(|| GraphError::Refused(format!("there is no property {name}")))?;
        if let Some(default) = default {
            let (ty, shape, key) = (self.properties[index].ty, self.properties[index].shape, self.properties[index].key);
            if !ty.fits_shape(shape, key, &default) {
                return Err(GraphError::Refused(format!("a {} property cannot default to {default}", ty.label())));
            }
            self.properties[index].default = default;
        }
        if let Some(new) = rename.filter(|n| *n != name) {
            check_variable_name(new).map_err(|_| GraphError::Refused(format!("property name {new:?} must be 1 to 64 letters, digits or _")))?;
            if self.property(new).is_some() {
                return Err(GraphError::Refused(format!("there is already a property {new}")));
            }
            self.properties[index].name = new.to_owned();
            for node in self.nodes.iter_mut().filter(|n| kinds.contains(&n.kind.as_str())) {
                if node.values.get("name").and_then(Value::as_str) == Some(name) {
                    node.values.insert("name".to_owned(), Value::from(new));
                }
            }
        }
        Ok(())
    }

    /// Sets how a property shows in the Inspector (checked by whoever knows the rules: the script host).
    pub fn set_property_options(&mut self, name: &str, options: PropertyOptions) -> Result<(), GraphError> {
        let property = self.properties.iter_mut().find(|p| p.name == name).ok_or_else(|| GraphError::Refused(format!("there is no property {name}")))?;
        property.inspector = options;
        Ok(())
    }

    /// Takes a property away; nodes naming it stay, reading the type's zero unless an object sets it.
    pub fn remove_property(&mut self, name: &str) -> Result<(), GraphError> {
        let before = self.properties.len();
        self.properties.retain(|p| p.name != name);
        if self.properties.len() == before {
            return Err(GraphError::Refused(format!("there is no property {name}")));
        }
        Ok(())
    }

    /// Takes a declaration away; nodes naming it keep working, starting from the type's zero.
    pub fn remove_variable(&mut self, name: &str) -> Result<(), GraphError> {
        let before = self.variables.len();
        self.variables.retain(|v| v.name != name);
        if self.variables.len() == before {
            return Err(GraphError::Refused(format!("there is no variable {name}")));
        }
        Ok(())
    }
}

impl Graph {
    /// An empty graph of `catalog`'s domain.
    pub fn empty(catalog: &Catalog) -> Self {
        Self {
            format: FORMAT.to_owned(),
            version: VERSION,
            domain: catalog.domain.to_owned(),
            nodes: Vec::new(),
            links: Vec::new(),
            comments: Vec::new(),
            variables: Vec::new(),
            menu: String::new(),
            functions: Vec::new(),
            events: Vec::new(),
            properties: Vec::new(),
        }
    }

    pub fn comment(&self, id: u32) -> Option<&Comment> {
        self.comments.iter().find(|c| c.id == id)
    }

    /// Adds a comment box and returns its id.
    pub fn add_comment(&mut self, text: &str, position: [f32; 2], size: [f32; 2]) -> Result<u32, GraphError> {
        if self.comments.len() >= MAX_COMMENTS {
            return Err(GraphError::Refused(format!("a graph may have at most {MAX_COMMENTS} comments")));
        }
        check_comment_text(text)?;
        let id = self.comments.iter().map(|c| c.id).max().map_or(1, |m| m.saturating_add(1));
        self.comments.push(Comment { id, text: text.to_owned(), position: clamp_position(position)?, size: clamp_size(size)? });
        Ok(id)
    }

    /// Changes a comment's text, place or size; `None` keeps that part.
    pub fn edit_comment(&mut self, id: u32, text: Option<&str>, position: Option<[f32; 2]>, size: Option<[f32; 2]>) -> Result<(), GraphError> {
        if let Some(text) = text {
            check_comment_text(text)?;
        }
        let position = position.map(clamp_position).transpose()?;
        let size = size.map(clamp_size).transpose()?;
        let comment = self.comments.iter_mut().find(|c| c.id == id).ok_or(GraphError::NoComment(id))?;
        if let Some(text) = text {
            text.clone_into(&mut comment.text);
        }
        if let Some(position) = position {
            comment.position = position;
        }
        if let Some(size) = size {
            comment.size = size;
        }
        Ok(())
    }

    /// Removes a comment box; the nodes in it stay.
    pub fn remove_comment(&mut self, id: u32) -> Result<(), GraphError> {
        let before = self.comments.len();
        self.comments.retain(|c| c.id != id);
        if self.comments.len() == before { Err(GraphError::NoComment(id)) } else { Ok(()) }
    }

    /// A new graph with the catalog's starter nodes.
    pub fn new(catalog: &Catalog) -> Self {
        let mut graph = Self::empty(catalog);
        for (kind, position) in catalog.starter {
            // Starter kinds come from the catalog itself.
            let _ = graph.add_node(catalog, kind, *position);
        }
        graph
    }

    /// Parses and checks a graph file of `catalog`'s domain.
    pub fn from_json(text: &str, catalog: &Catalog) -> Result<Self, GraphError> {
        let mut graph: Graph = serde_json::from_str(text).map_err(|e| GraphError::Format(e.to_string()))?;
        let read = graph.version;
        graph.upgrade();
        if read < VERSION {
            (catalog.upgrade)(&mut graph, read);
            graph.version = VERSION;
        }
        graph.check(catalog)?;
        Ok(graph)
    }

    /// Moves what older versions kept elsewhere into its current place: a Menu Path node (before
    /// version 3) becomes `menu`.
    fn upgrade(&mut self) {
        if self.version >= 3 {
            return;
        }
        let old: Vec<u32> = self.nodes.iter().filter(|n| n.kind == OLD_MENU_NODE).map(|n| n.id).collect();
        for id in &old {
            if self.menu.is_empty()
                && let Some(path) = self.value(*id, "path").and_then(Value::as_str)
            {
                self.menu = path.to_owned();
            }
        }
        self.nodes.retain(|n| !old.contains(&n.id));
        self.links.retain(|l| !old.contains(&l.from.node) && !old.contains(&l.to.node));
        self.version = VERSION;
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Checks a whole graph, as loaded from a file or an agent.
    pub fn check(&self, catalog: &Catalog) -> Result<(), GraphError> {
        if self.format != FORMAT {
            return Err(GraphError::Format(format!("format must be \"{FORMAT}\"")));
        }
        if !(1..=VERSION).contains(&self.version) {
            return Err(GraphError::Format(format!("graph version {} is not supported (this engine reads versions 1 to {VERSION})", self.version)));
        }
        if self.menu.chars().count() > MAX_MENU {
            return Err(GraphError::Format(format!("the menu path is at most {MAX_MENU} characters")));
        }
        if self.variables.len() > MAX_VARIABLES {
            return Err(GraphError::Format(format!("a graph declares at most {MAX_VARIABLES} variables")));
        }
        for (index, variable) in self.variables.iter().enumerate() {
            check_variable_name(&variable.name).map_err(GraphError::Format)?;
            if !variable.ty.fits_shape(variable.shape, variable.key, &variable.start) {
                return Err(GraphError::Format(format!("variable {} starts at a value that is not a {}", variable.name, variable.ty.label())));
            }
            if self.variables[..index].iter().any(|v| v.name == variable.name) {
                return Err(GraphError::Format(format!("variable {} is declared twice", variable.name)));
            }
        }
        if self.properties.len() > MAX_PROPERTIES {
            return Err(GraphError::Format(format!("a graph declares at most {MAX_PROPERTIES} properties")));
        }
        for (index, property) in self.properties.iter().enumerate() {
            check_variable_name(&property.name).map_err(GraphError::Format)?;
            if !property.ty.fits_shape(property.shape, property.key, &property.default) {
                return Err(GraphError::Format(format!("property {} defaults to a value that is not a {}", property.name, property.ty.label())));
            }
            if self.properties[..index].iter().any(|p| p.name == property.name) {
                return Err(GraphError::Format(format!("property {} is declared twice", property.name)));
            }
        }
        if self.functions.len() > MAX_FUNCTIONS {
            return Err(GraphError::Format(format!("a graph declares at most {MAX_FUNCTIONS} functions")));
        }
        for (index, function) in self.functions.iter().enumerate() {
            check_variable_name(&function.name).map_err(GraphError::Format)?;
            check_params(&function.name, &function.inputs).map_err(GraphError::Format)?;
            check_params(&function.name, &function.outputs).map_err(GraphError::Format)?;
            if self.functions[..index].iter().any(|f| f.name == function.name) {
                return Err(GraphError::Format(format!("function {} is declared twice", function.name)));
            }
        }
        if self.events.len() > MAX_EVENTS {
            return Err(GraphError::Format(format!("a graph declares at most {MAX_EVENTS} events")));
        }
        for (index, event) in self.events.iter().enumerate() {
            check_variable_name(&event.name).map_err(GraphError::Format)?;
            check_params(&event.name, &event.params).map_err(GraphError::Format)?;
            if self.events[..index].iter().any(|e| e.name == event.name) {
                return Err(GraphError::Format(format!("event {} is declared twice", event.name)));
            }
        }
        if self.domain != catalog.domain {
            return Err(GraphError::Format(format!("this is a {} graph, not a {} graph", self.domain, catalog.domain)));
        }
        if self.nodes.len() > MAX_NODES || self.links.len() > MAX_LINKS {
            return Err(GraphError::Format(format!("a graph may have at most {MAX_NODES} nodes and {MAX_LINKS} links")));
        }
        let mut ids = HashSet::new();
        for node in &self.nodes {
            if !ids.insert(node.id) {
                return Err(GraphError::Format(format!("node id {} is used twice", node.id)));
            }
            let def = catalog.def_of(node).ok_or_else(|| GraphError::UnknownKind(node.kind.clone(), catalog.domain))?;
            if def.unique && self.nodes.iter().filter(|n| n.kind == node.kind).count() > 1 {
                return Err(GraphError::Refused(format!("a graph may have only one {} node", def.label)));
            }
            if !node.position.iter().all(|c| c.is_finite() && c.abs() <= MAX_COORD) {
                return Err(GraphError::Format(format!("node {} has an invalid position", node.id)));
            }
            for (name, value) in &node.values {
                check_value(def, name, value)?;
            }
        }
        if self.comments.len() > MAX_COMMENTS {
            return Err(GraphError::Format(format!("a graph may have at most {MAX_COMMENTS} comments")));
        }
        let mut comment_ids = HashSet::new();
        for comment in &self.comments {
            if !comment_ids.insert(comment.id) {
                return Err(GraphError::Format(format!("comment id {} is used twice", comment.id)));
            }
            check_comment_text(&comment.text)?;
            let finite = |v: &[f32; 2]| v.iter().all(|c| c.is_finite() && c.abs() <= MAX_COORD);
            let big_enough = comment.size[0] >= MIN_COMMENT_SIZE[0] && comment.size[1] >= MIN_COMMENT_SIZE[1];
            if !finite(&comment.position) || !finite(&comment.size) || !big_enough {
                return Err(GraphError::Format(format!("comment {} has an invalid position or size", comment.id)));
            }
        }
        let mut seen = Graph { links: Vec::new(), ..self.clone() };
        for link in &self.links {
            // A file is read by the pins' own types, so a graph saved with a link that no longer
            // fits still opens (its compiler says what is wrong, on the node, to mend there).
            seen.check_link(catalog, link, None)?;
            seen.links.push(link.clone());
        }
        Ok(())
    }

    pub fn node(&self, id: u32) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    fn node_mut(&mut self, id: u32) -> Result<&mut Node, GraphError> {
        self.nodes.iter_mut().find(|n| n.id == id).ok_or(GraphError::NoNode(id))
    }

    /// The node and its kind.
    pub fn node_def<'c>(&self, catalog: &'c Catalog, id: u32) -> Result<(&Node, &'c NodeDef), GraphError> {
        let node = self.node(id).ok_or(GraphError::NoNode(id))?;
        let def = catalog.def_of(node).ok_or_else(|| GraphError::UnknownKind(node.kind.clone(), catalog.domain))?;
        Ok((node, def))
    }

    /// Adds a node and returns its id.
    pub fn add_node(&mut self, catalog: &Catalog, kind: &str, position: [f32; 2]) -> Result<u32, GraphError> {
        let def = catalog.get(kind).ok_or_else(|| GraphError::UnknownKind(kind.to_owned(), catalog.domain))?;
        if self.nodes.len() >= MAX_NODES {
            return Err(GraphError::Refused(format!("a graph may have at most {MAX_NODES} nodes")));
        }
        if def.unique && self.nodes.iter().any(|n| n.kind == kind) {
            return Err(GraphError::Refused(format!("a graph may have only one {} node", def.label)));
        }
        let id = self.nodes.iter().map(|n| n.id).max().map_or(1, |m| m.saturating_add(1));
        self.nodes.push(Node { id, kind: kind.to_owned(), position: clamp_position(position)?, values: BTreeMap::new() });
        Ok(id)
    }

    /// Removes a node and its links.
    pub fn remove_node(&mut self, id: u32) -> Result<(), GraphError> {
        let before = self.nodes.len();
        self.nodes.retain(|n| n.id != id);
        if self.nodes.len() == before {
            return Err(GraphError::NoNode(id));
        }
        self.links.retain(|l| l.from.node != id && l.to.node != id);
        Ok(())
    }

    pub fn move_node(&mut self, id: u32, position: [f32; 2]) -> Result<(), GraphError> {
        let position = clamp_position(position)?;
        self.node_mut(id)?.position = position;
        Ok(())
    }

    /// Sets a setting or the value of an input without a link; `null` restores the default.
    pub fn set_value(&mut self, catalog: &Catalog, id: u32, name: &str, value: Value) -> Result<(), GraphError> {
        let (_, def) = self.node_def(catalog, id)?;
        let node = self.node_mut(id)?;
        if value.is_null() {
            if def.input(name).is_none() && def.setting(name).is_none() {
                return Err(GraphError::NoPin { node: def.label.to_owned(), side: "value", pin: name.to_owned() });
            }
            node.values.remove(name);
            return Ok(());
        }
        check_value(def, name, &value)?;
        node.values.insert(name.to_owned(), value);
        self.drop_dangling_links(catalog, id);
        Ok(())
    }

    /// Removes the links of `id` to pins its definition no longer has, or whose types no longer fit
    /// (a setting changed what a pin carries).
    pub fn drop_unfit_links(&mut self, catalog: &Catalog, id: u32) {
        self.drop_dangling_links(catalog, id);
        let touching: Vec<Link> = self.links.iter().filter(|l| l.from.node == id || l.to.node == id).cloned().collect();
        for link in touching {
            let others = Graph { links: self.links.iter().filter(|l| **l != link).cloned().collect(), ..self.clone() };
            if others.check_link(catalog, &link, Some(&others)).is_err() {
                self.links.retain(|l| *l != link);
            }
        }
    }

    /// Removes the links of `id` to pins its definition no longer has (a setting changed its pins).
    fn drop_dangling_links(&mut self, catalog: &Catalog, id: u32) {
        let Some(def) = self.node(id).and_then(|n| catalog.def_of(n)) else { return };
        self.links.retain(|l| (l.from.node != id || def.output(&l.from.pin).is_some()) && (l.to.node != id || def.input(&l.to.pin).is_some()));
    }

    /// Joins an output to an input. A data input keeps only the newest link, and so does a flow output.
    pub fn connect(&mut self, catalog: &Catalog, link: Link) -> Result<(), GraphError> {
        let (from_ty, _) = self.pin_types(catalog, &link)?;
        let mut trial = self.clone();
        // The new link replaces the one it supersedes.
        trial.links.retain(|l| l != &link && if from_ty == PinType::Flow { l.from != link.from } else { l.to != link.to });
        trial.check_link(catalog, &link, Some(&trial))?;
        if trial.links.len() >= MAX_LINKS {
            return Err(GraphError::Refused(format!("a graph may have at most {MAX_LINKS} links")));
        }
        trial.links.push(link);
        // What the new link brings may turn an output further on into a vector that its links no
        // longer fit.
        for later in &trial.links {
            let (from, to) = trial.pin_types(catalog, later)?;
            let from = if from == PinType::Any { trial.output_type(catalog, &later.from) } else { from };
            if !from.connects_to(to) {
                let label = |id: u32| trial.node(id).and_then(|n| catalog.def_of(n)).map_or("a node", |d| d.label);
                return Err(GraphError::Format(format!(
                    "then {} would give {} to {}, which takes {}",
                    label(later.from.node),
                    from.label(),
                    label(later.to.node),
                    to.label()
                )));
            }
        }
        *self = trial;
        Ok(())
    }

    /// Removes every link at a pin.
    pub fn disconnect(&mut self, pin: &PinRef) -> usize {
        let before = self.links.len();
        self.links.retain(|l| &l.from != pin && &l.to != pin);
        before - self.links.len()
    }

    /// Nodes `ids` and the links between them, for the clipboard. Links to nodes outside are left out.
    pub fn copy(&self, ids: &[u32]) -> Fragment {
        let nodes: Vec<Node> = self.nodes.iter().filter(|n| ids.contains(&n.id)).cloned().collect();
        let links = self.links.iter().filter(|l| ids.contains(&l.from.node) && ids.contains(&l.to.node)).cloned().collect();
        Fragment { format: FRAGMENT_FORMAT.to_owned(), domain: self.domain.clone(), nodes, links }
    }

    /// Adds copies of `fragment`'s nodes, moved by `offset`, with new ids and the links between
    /// them. A node that may appear only once and is already here is left out. All or nothing:
    /// on an error the graph is unchanged. Returns the new ids, in the fragment's order.
    pub fn paste(&mut self, catalog: &Catalog, fragment: &Fragment, offset: [f32; 2]) -> Result<Pasted, GraphError> {
        if fragment.format != FRAGMENT_FORMAT {
            return Err(GraphError::Format(format!("the clipboard does not hold {FRAGMENT_FORMAT} nodes")));
        }
        if fragment.domain != self.domain {
            return Err(GraphError::Refused(format!("these are {} nodes; they cannot go into a {} graph", fragment.domain, self.domain)));
        }
        let mut graph = self.clone();
        let mut ids = BTreeMap::new();
        let mut skipped = Vec::new();
        for node in &fragment.nodes {
            let def = catalog.def_of(node).ok_or_else(|| GraphError::UnknownKind(node.kind.clone(), catalog.domain))?;
            if def.unique && graph.nodes.iter().any(|n| n.kind == node.kind) {
                skipped.push(def.label.to_owned());
                continue;
            }
            let position = [node.position[0] + offset[0], node.position[1] + offset[1]];
            let id = graph.add_node(catalog, &node.kind, position)?;
            for (name, value) in &node.values {
                graph.set_value(catalog, id, name, value.clone())?;
            }
            ids.insert(node.id, id);
        }
        for link in &fragment.links {
            if let (Some(from), Some(to)) = (ids.get(&link.from.node), ids.get(&link.to.node)) {
                graph.connect(catalog, Link { from: PinRef::new(*from, &link.from.pin), to: PinRef::new(*to, &link.to.pin) })?;
            }
        }
        *self = graph;
        Ok(Pasted { ids: fragment.nodes.iter().filter_map(|n| ids.get(&n.id).copied()).collect(), skipped })
    }

    /// The link into an input pin, if any (for flow inputs, the first one).
    pub fn link_into(&self, node: u32, pin: &str) -> Option<&Link> {
        self.links.iter().find(|l| l.to.node == node && l.to.pin == pin)
    }

    /// The links out of an output pin.
    pub fn links_from(&self, node: u32, pin: &str) -> impl Iterator<Item = &Link> {
        self.links.iter().filter(move |l| l.from.node == node && l.from.pin == pin)
    }

    /// The value stored for `name`, if any.
    pub fn value(&self, node: u32, name: &str) -> Option<&Value> {
        self.node(node).and_then(|n| n.values.get(name))
    }

    /// What an output pin gives. A number-or-vector output (`Any`) takes it from its node's
    /// inputs: a vector when one of its number-or-vector inputs is a vector (linked, or a vector
    /// typed in), else a whole number when every number input is whole (unless the kind is
    /// [`Catalog::fractional`]), else a number. `Any` while nothing decides it (a loop of links).
    pub fn output_type(&self, catalog: &Catalog, pin: &PinRef) -> PinType {
        self.resolve_output(catalog, pin, 0)
    }

    fn resolve_output(&self, catalog: &Catalog, pin: &PinRef, depth: usize) -> PinType {
        let Some(def) = self.node(pin.node).and_then(|n| catalog.def_of(n)) else { return PinType::Any };
        let Some(out) = def.output(&pin.pin) else { return PinType::Any };
        if out.ty != PinType::Any {
            return out.ty;
        }
        if depth > 64 {
            return PinType::Any;
        }
        let mut vector = false;
        let mut whole = !catalog.fractional.contains(&def.kind);
        let mut numbers = false;
        for input in def.inputs.iter().filter(|p| matches!(p.ty, PinType::Any | PinType::Float | PinType::Int)) {
            numbers = true;
            let ty = match self.link_into(pin.node, input.name) {
                Some(link) => self.resolve_output(catalog, &link.from, depth + 1),
                None => match (input.ty, self.value(pin.node, input.name)) {
                    (PinType::Int, _) => PinType::Int,
                    (PinType::Any, Some(v)) if as_triple(v).is_some() => PinType::Vec3,
                    (PinType::Any, None) if matches!(input.default, PinDefault::Vec3(_) | PinDefault::Color(_)) => PinType::Vec3,
                    _ => PinType::Float,
                },
            };
            match ty {
                PinType::Vec3 | PinType::Color if input.ty == PinType::Any => vector = true,
                PinType::Int => {}
                PinType::Any => return PinType::Any,
                _ => whole = false,
            }
        }
        match (numbers, vector, whole) {
            (false, _, _) => PinType::Any,
            (true, true, _) => PinType::Vec3,
            (true, false, true) => PinType::Int,
            (true, false, false) => PinType::Float,
        }
    }

    fn pin_types(&self, catalog: &Catalog, link: &Link) -> Result<(PinType, PinType), GraphError> {
        let (_, from_def) = self.node_def(catalog, link.from.node)?;
        let (_, to_def) = self.node_def(catalog, link.to.node)?;
        let from =
            from_def.output(&link.from.pin).ok_or_else(|| GraphError::NoPin { node: from_def.label.to_owned(), side: "output", pin: link.from.pin.clone() })?;
        let to = to_def.input(&link.to.pin).ok_or_else(|| GraphError::NoPin { node: to_def.label.to_owned(), side: "input", pin: link.to.pin.clone() })?;
        Ok((from.ty, to.ty))
    }

    /// Checks one link against the graph without it. Clashes with existing links are `Refused`.
    /// `types`, when given, is the graph whose links decide what a number-or-vector output gives;
    /// without it such an output fits any number input.
    fn check_link(&self, catalog: &Catalog, link: &Link, types: Option<&Graph>) -> Result<(PinType, PinType), GraphError> {
        let (from, to) = self.pin_types(catalog, link)?;
        // A number-or-vector output links as what its inputs make it.
        let from = match types {
            Some(graph) if from == PinType::Any => graph.output_type(catalog, &link.from),
            _ => from,
        };
        let (_, from_def) = self.node_def(catalog, link.from.node)?;
        let (_, to_def) = self.node_def(catalog, link.to.node)?;
        if link.from.node == link.to.node {
            return Err(GraphError::Format("a node cannot be linked to itself".to_owned()));
        }
        if !from.connects_to(to) {
            return Err(GraphError::Format(format!(
                "{} · {} gives {}, but {} · {} takes {}",
                from_def.label,
                pin_label(from_def.outputs, &link.from.pin),
                from.label(),
                to_def.label,
                pin_label(to_def.inputs, &link.to.pin),
                to.label()
            )));
        }
        if self.reaches(link.to.node, link.from.node) {
            return Err(GraphError::Format(format!("linking {} to {} would make a loop", from_def.label, to_def.label)));
        }
        if self.links.contains(link) {
            return Err(GraphError::Refused("these pins are already linked".to_owned()));
        }
        if from == PinType::Flow && self.links.iter().any(|l| l.from == link.from) {
            return Err(GraphError::Refused("a flow output leads to one node; remove its link first".to_owned()));
        }
        if from != PinType::Flow && self.links.iter().any(|l| l.to == link.to) {
            return Err(GraphError::Refused("this input already has a link; remove it first".to_owned()));
        }
        Ok((from, to))
    }

    /// True when following links forward from `start` arrives at `goal`.
    pub fn reaches(&self, start: u32, goal: u32) -> bool {
        let mut stack = vec![start];
        let mut seen = HashSet::new();
        while let Some(id) = stack.pop() {
            if id == goal {
                return true;
            }
            if seen.insert(id) {
                stack.extend(self.links.iter().filter(|l| l.from.node == id).map(|l| l.to.node));
            }
        }
        false
    }
}

fn pin_label<'a>(pins: &'a [PinDef], name: &'a str) -> &'a str {
    pins.iter().find(|p| p.name == name).map_or(name, |p| p.label)
}

fn clamp_position(position: [f32; 2]) -> Result<[f32; 2], GraphError> {
    if position.iter().all(|c| c.is_finite()) {
        Ok(position.map(|c| c.clamp(-MAX_COORD, MAX_COORD)))
    } else {
        Err(GraphError::Refused("positions must be finite numbers".to_owned()))
    }
}

fn check_comment_text(text: &str) -> Result<(), GraphError> {
    if text.chars().count() > MAX_COMMENT_TEXT {
        return Err(GraphError::Refused(format!("a comment may have at most {MAX_COMMENT_TEXT} characters")));
    }
    Ok(())
}

/// A comment size, at least the smallest box.
fn clamp_size(size: [f32; 2]) -> Result<[f32; 2], GraphError> {
    if size.iter().all(|c| c.is_finite()) {
        Ok([size[0].clamp(MIN_COMMENT_SIZE[0], MAX_COORD), size[1].clamp(MIN_COMMENT_SIZE[1], MAX_COORD)])
    } else {
        Err(GraphError::Refused("sizes must be finite numbers".to_owned()))
    }
}

/// Checks a value for a setting or an input pin of `def`.
fn check_value(def: &NodeDef, name: &str, value: &Value) -> Result<(), GraphError> {
    let bad = |what: &str| GraphError::Refused(format!("{} · {name} must be {what}", def.label));
    let number = |v: &Value| v.as_f64().filter(|f| f.is_finite() && f.abs() < f64::from(f32::MAX)).is_some();
    let triple = |v: &Value| v.as_array().is_some_and(|a| a.len() == 3 && a.iter().all(number));
    if let Some(setting) = def.setting(name) {
        return match setting.kind {
            SettingKind::Float(_) if number(value) => Ok(()),
            SettingKind::Float(_) => Err(bad("a finite number")),
            SettingKind::Color(_) | SettingKind::Vec3(_) if triple(value) => Ok(()),
            SettingKind::Color(_) | SettingKind::Vec3(_) => Err(bad("three finite numbers")),
            SettingKind::Text(_) => match value.as_str() {
                Some(text) if text.chars().count() <= MAX_TEXT => Ok(()),
                _ => Err(bad(&format!("text of at most {MAX_TEXT} characters"))),
            },
            SettingKind::Name(_) => match value.as_str() {
                Some(text) if !text.is_empty() && text.len() <= 64 && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => Ok(()),
                _ => Err(bad("a name of letters, digits and _ (at most 64)")),
            },
            SettingKind::Choice(choices) => match value.as_str() {
                Some(text) if choices.contains(&text) => Ok(()),
                _ => Err(bad(&format!("one of {}", choices.join(", ")))),
            },
            SettingKind::Names => match value.as_array() {
                Some(names)
                    if names.len() <= 64
                        && names.iter().all(|n| n.as_str().is_some_and(|t| (1..=64).contains(&t.chars().count()) && !t.chars().any(char::is_control))) =>
                {
                    Ok(())
                }
                _ => Err(bad("a list of up to 64 names of 1 to 64 characters")),
            },
        };
    }
    let pin = def.input(name).ok_or_else(|| GraphError::NoPin { node: def.label.to_owned(), side: "input or setting", pin: name.to_owned() })?;
    match pin.ty {
        PinType::Flow => Err(GraphError::Refused(format!("{} · {name} is a flow input and holds no value", def.label))),
        PinType::Float | PinType::Any if number(value) => Ok(()),
        PinType::Any if triple(value) => Ok(()),
        PinType::Float | PinType::Any => Err(bad("a finite number")),
        PinType::Vec3 | PinType::Color if triple(value) => Ok(()),
        PinType::Vec3 | PinType::Color => Err(bad("three finite numbers")),
        PinType::Bool if value.is_boolean() => Ok(()),
        PinType::Bool => Err(bad("true or false")),
        PinType::Object => Err(GraphError::Refused(format!("{} · {name} takes an object only from a link", def.label))),
        PinType::Component => Err(GraphError::Refused(format!("{} · {name} takes a component only from a link", def.label))),
        PinType::List(_) | PinType::Map(..) => Err(GraphError::Refused(format!("{} · {name} takes a list or map only from a link", def.label))),
        PinType::Int => match value.as_i64() {
            Some(n) if i32::try_from(n).is_ok() => Ok(()),
            _ => Err(bad("a whole number from -2147483648 to 2147483647")),
        },
        PinType::Text => match value.as_str() {
            Some(text) if text.chars().count() <= MAX_TEXT => Ok(()),
            _ => Err(bad(&format!("text of at most {MAX_TEXT} characters"))),
        },
    }
}

/// One problem that stops a graph from compiling. Messages are shown to the user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Problem {
    /// The node the problem is on, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<u32>,
    pub message: String,
}

impl Problem {
    pub fn on(node: u32, message: impl Into<String>) -> Self {
        Self { node: Some(node), message: message.into() }
    }

    pub fn graph(message: impl Into<String>) -> Self {
        Self { node: None, message: message.into() }
    }
}

/// Problems as one line.
pub fn summary(problems: &[Problem]) -> String {
    problems.iter().map(|p| p.message.as_str()).collect::<Vec<_>>().join("; ")
}

/// Reads a number value.
pub fn as_f32(value: &Value) -> Option<f32> {
    value.as_f64().map(|v| v as f32)
}

/// Reads a three-number value.
pub fn as_triple(value: &Value) -> Option<[f32; 3]> {
    let a = value.as_array()?;
    match a.as_slice() {
        [x, y, z] => Some([as_f32(x)?, as_f32(y)?, as_f32(z)?]),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
