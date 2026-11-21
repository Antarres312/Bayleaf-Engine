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
/// Current graph file version.
pub const VERSION: u32 = 1;
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
        }
    }

    /// Whether an output of type `self` may feed an input of type `to`. Numbers widen to vectors
    /// and colours; vectors and colours convert into each other. Whether an `Any` output really
    /// fits is checked when the graph is compiled.
    pub fn connects_to(self, to: PinType) -> bool {
        use PinType::{Any, Bool, Color, Float, Flow, Vec3};
        match (self, to) {
            (Flow, Flow) | (Bool, Bool) => true,
            (Flow | Bool, _) | (_, Flow | Bool) => false,
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
}

#[derive(Clone, Copy, Debug)]
pub struct SettingDef {
    pub name: &'static str,
    pub label: &'static str,
    pub kind: SettingKind,
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
}

impl Catalog {
    pub fn get(&self, kind: &str) -> Option<&NodeDef> {
        self.nodes.iter().find(|n| n.kind == kind)
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
    #[error("there is no node kind \"{0}\" in {1} graphs")]
    UnknownKind(String, &'static str),
    #[error("{node} has no {side} \"{pin}\"")]
    NoPin { node: String, side: &'static str, pin: String },
    #[error("{0}")]
    Refused(String),
    #[error("not a graph file: {0}")]
    Format(String),
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
}

impl Graph {
    /// An empty graph of `catalog`'s domain.
    pub fn empty(catalog: &Catalog) -> Self {
        Self { format: FORMAT.to_owned(), version: VERSION, domain: catalog.domain.to_owned(), nodes: Vec::new(), links: Vec::new() }
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
        let graph: Graph = serde_json::from_str(text).map_err(|e| GraphError::Format(e.to_string()))?;
        graph.check(catalog)?;
        Ok(graph)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Checks a whole graph, as loaded from a file or an agent.
    pub fn check(&self, catalog: &Catalog) -> Result<(), GraphError> {
        if self.format != FORMAT {
            return Err(GraphError::Format(format!("format must be \"{FORMAT}\"")));
        }
        if self.version != VERSION {
            return Err(GraphError::Format(format!("graph version {} is not supported (this engine reads version {VERSION})", self.version)));
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
            let def = catalog.get(&node.kind).ok_or_else(|| GraphError::UnknownKind(node.kind.clone(), catalog.domain))?;
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
        let mut seen = Graph { links: Vec::new(), ..self.clone() };
        for link in &self.links {
            seen.check_link(catalog, link)?;
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
        let def = catalog.get(&node.kind).ok_or_else(|| GraphError::UnknownKind(node.kind.clone(), catalog.domain))?;
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
        Ok(())
    }

    /// Joins an output to an input. A data input keeps only the newest link, and so does a flow output.
    pub fn connect(&mut self, catalog: &Catalog, link: Link) -> Result<(), GraphError> {
        let (from_ty, _) = self.pin_types(catalog, &link)?;
        let mut trial = self.clone();
        // The new link replaces the one it supersedes.
        trial.links.retain(|l| l != &link && if from_ty == PinType::Flow { l.from != link.from } else { l.to != link.to });
        trial.check_link(catalog, &link)?;
        if trial.links.len() >= MAX_LINKS {
            return Err(GraphError::Refused(format!("a graph may have at most {MAX_LINKS} links")));
        }
        trial.links.push(link);
        *self = trial;
        Ok(())
    }

    /// Removes every link at a pin.
    pub fn disconnect(&mut self, pin: &PinRef) -> usize {
        let before = self.links.len();
        self.links.retain(|l| &l.from != pin && &l.to != pin);
        before - self.links.len()
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

    fn pin_types(&self, catalog: &Catalog, link: &Link) -> Result<(PinType, PinType), GraphError> {
        let (_, from_def) = self.node_def(catalog, link.from.node)?;
        let (_, to_def) = self.node_def(catalog, link.to.node)?;
        let from =
            from_def.output(&link.from.pin).ok_or_else(|| GraphError::NoPin { node: from_def.label.to_owned(), side: "output", pin: link.from.pin.clone() })?;
        let to = to_def.input(&link.to.pin).ok_or_else(|| GraphError::NoPin { node: to_def.label.to_owned(), side: "input", pin: link.to.pin.clone() })?;
        Ok((from.ty, to.ty))
    }

    /// Checks one link against the graph without it. Clashes with existing links are `Refused`.
    fn check_link(&self, catalog: &Catalog, link: &Link) -> Result<(PinType, PinType), GraphError> {
        let (from, to) = self.pin_types(catalog, link)?;
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
