//! The components visual scripts read and set field by field: Get Component, Try Get Component
//! and Set Component show the fields of the component named in their Component setting as pins.
//! Only fields that are numbers, vectors, colours or true/false are listed; a field's pin name is
//! its name in scene files (`intensity`, `skyColor`).
//!
//! A script on an object is a component too: named by its name (`Bob`), its properties are number
//! pins. The node keeps the property names in its `fields` setting, chosen in the editor from what
//! the script declares, so a graph compiles without reading other scripts.

use bayleaf_graph::{Node, NodeDef, PinDef, PinDefault, PinType};

use super::{RUN, TARGET, THEN};
use PinType::{Bool, Color, Float, Vec3};

/// The Component setting of Get Component, Try Get Component and Set Component.
pub(super) const COMPONENT: bayleaf_graph::SettingDef =
    bayleaf_graph::SettingDef { name: "component", label: "Component", kind: bayleaf_graph::SettingKind::Text("Point Light") };
/// The properties of the script a node names, kept by the node.
pub(super) const FIELDS: bayleaf_graph::SettingDef = bayleaf_graph::SettingDef { name: "fields", label: "Fields", kind: bayleaf_graph::SettingKind::Names };

pub(super) const GET: &str = "The fields of a component on the linked object (else this object), as the step began. A missing component reads as zeros; Try Get Component says whether it was found.";
pub(super) const TRY_GET: &str = "The fields of a component on the linked object (else this object), as the step began, and whether the object has it.";
pub(super) const SET: &str =
    "Sets the linked fields of a component on the linked object (else this object), after this step. Fields without a link keep their values.";

/// One component's field pins, as the three node kinds show them.
pub struct ComponentNodes {
    /// The component's name in scene files (`pointLight`).
    pub name: &'static str,
    /// Its name in the editor (`Point Light`).
    pub label: &'static str,
    get: NodeDef,
    try_get: NodeDef,
    set: NodeDef,
}

impl ComponentNodes {
    /// Whether Set Component can change it while the game plays. Physics reads Rigid Body,
    /// Collider and Character once, when play starts, so those are read only.
    pub fn settable(&self) -> bool {
        !matches!(self.name, "rigidBody" | "collider" | "characterBody")
    }

    /// Its fields, as Get Component shows them.
    pub fn fields(&self) -> &'static [PinDef] {
        self.get.outputs
    }
}

macro_rules! component {
    ($name:literal, $label:literal, [$(($field:literal, $field_label:literal, $ty:expr)),* $(,)?]) => {
        ComponentNodes {
            name: $name,
            label: $label,
            get: NodeDef {
                kind: "getComponent",
                label: "Get Component",
                category: "Find",
                description: GET,
                inputs: &[TARGET],
                outputs: &[$(PinDef::new($field, $field_label, $ty)),*],
                settings: &[COMPONENT, FIELDS],
                unique: false,
            },
            try_get: NodeDef {
                kind: "tryGetComponent",
                label: "Try Get Component",
                category: "Find",
                description: TRY_GET,
                inputs: &[TARGET],
                outputs: &[PinDef::new("found", "Found", Bool), $(PinDef::new($field, $field_label, $ty)),*],
                settings: &[COMPONENT, FIELDS],
                unique: false,
            },
            set: NodeDef {
                kind: "setComponent",
                label: "Set Component",
                category: "Actions",
                description: SET,
                inputs: &[RUN, TARGET, $(PinDef::with($field, $field_label, $ty, PinDefault::Implicit("unchanged"))),*],
                outputs: &[THEN],
                settings: &[COMPONENT, FIELDS],
                unique: false,
            },
        }
    };
}

/// Every component whose fields visual scripts can read and set.
pub const COMPONENTS: &[ComponentNodes] = &[
    component!("transform", "Transform", [("position", "Position", Vec3), ("rotation", "Rotation", Vec3), ("scale", "Scale", Vec3)]),
    component!("mesh", "Mesh", [("color", "Color", Color), ("metallic", "Metallic", Float), ("roughness", "Roughness", Float)]),
    component!("sprite", "Sprite", [("color", "Color", Color)]),
    component!("camera", "Camera", [("fov", "Field of View", Float), ("size", "Size", Float), ("near", "Near", Float), ("far", "Far", Float)]),
    component!("directionalLight", "Directional Light", [("color", "Color", Color), ("intensity", "Intensity", Float), ("shadows", "Shadows", Bool)]),
    component!(
        "pointLight",
        "Point Light",
        [("color", "Color", Color), ("intensity", "Intensity", Float), ("range", "Range", Float), ("shadows", "Shadows", Bool)]
    ),
    component!(
        "spotLight",
        "Spot Light",
        [
            ("color", "Color", Color),
            ("intensity", "Intensity", Float),
            ("range", "Range", Float),
            ("angle", "Angle", Float),
            ("softness", "Softness", Float),
            ("shadows", "Shadows", Bool),
        ]
    ),
    component!(
        "environment",
        "Environment",
        [
            ("skyColor", "Sky Color", Color),
            ("horizonColor", "Horizon Color", Color),
            ("groundColor", "Ground Color", Color),
            ("ambient", "Ambient", Float),
            ("fogDistance", "Fog Distance", Float),
            ("showSky", "Show Sky", Bool),
        ]
    ),
    component!("rigidBody", "Rigid Body", [("mass", "Mass", Float), ("gravityScale", "Gravity Scale", Float), ("lockRotation", "Lock Rotation", Bool)]),
    component!(
        "collider",
        "Collider",
        [("size", "Size", Vec3), ("friction", "Friction", Float), ("restitution", "Restitution", Float), ("trigger", "Trigger", Bool)]
    ),
    component!(
        "characterBody",
        "Character",
        [("height", "Height", Float), ("radius", "Radius", Float), ("maxSlope", "Max Slope", Float), ("stepHeight", "Step Height", Float)]
    ),
    component!(
        "playerControl",
        "Player Control",
        [
            ("speed", "Speed", Float),
            ("jumpSpeed", "Jump Speed", Float),
            ("turnToMove", "Turn to Move", Bool),
            ("cameraDistance", "Camera Distance", Float),
            ("cameraHeight", "Camera Height", Float),
        ]
    ),
    component!(
        "joint",
        "Joint",
        [
            ("motorSpeed", "Motor Speed", Float),
            ("motorForce", "Motor Force", Float),
            ("limited", "Limited", Bool),
            ("lower", "Lower", Float),
            ("upper", "Upper", Float),
        ]
    ),
    component!("gameRules", "Game Rules", [("respawn", "Respawn", Bool), ("respawnHeight", "Respawn Height", Float), ("timeLimit", "Time Limit", Float)]),
];

/// The component a node's Component setting names (by its name or label; case and spaces do not
/// matter), if visual scripts can read it.
pub fn component_of(node: &Node) -> Option<&'static ComponentNodes> {
    let named = node.values.get("component").and_then(serde_json::Value::as_str).unwrap_or("Point Light");
    let key = crate::kind_key(named);
    COMPONENTS.iter().find(|c| crate::kind_key(c.name) == key || crate::kind_key(c.label) == key)
}

/// The property names a node keeps for the script it names, if it names a script.
pub fn script_fields(node: &Node) -> Option<Vec<String>> {
    if component_of(node).is_some() {
        return None;
    }
    let names = node.values.get("fields")?.as_array()?;
    Some(names.iter().filter_map(|n| n.as_str().map(str::to_owned)).collect())
}

/// A script's Get Component also gives the script itself, as a component.
const COMPONENT_OUT: PinDef = PinDef::new("component", "Component", PinType::Component);

/// Most distinct script pin sets kept at once; each is made once and kept for the program's life.
const MAX_SCRIPT_VARIANTS: usize = 1024;

/// Script variants made so far, by node kind and property names.
static SCRIPT_VARIANTS: std::sync::Mutex<Vec<(String, Vec<String>, &'static NodeDef)>> = std::sync::Mutex::new(Vec::new());

/// The pins of a node that names a script: one pin per property, typed as `name:type` says
/// (`speed`, a number, or `count:whole`, `on:bool`, `offset:vector`, `tint:color`). Text and object
/// properties have no pin: fields are read and set as numbers.
fn script_variant(node: &Node, fields: Vec<String>) -> Option<&'static NodeDef> {
    let mut made = SCRIPT_VARIANTS.lock().ok()?;
    if let Some((_, _, def)) = made.iter().find(|(kind, names, _)| *kind == node.kind && *names == fields) {
        return Some(def);
    }
    if made.len() >= MAX_SCRIPT_VARIANTS {
        return None;
    }
    let leak = |text: String| -> &'static str { Box::leak(text.into_boxed_str()) };
    let pins = |default: PinDefault| -> Vec<PinDef> {
        fields
            .iter()
            .filter_map(|field| {
                let (name, code) = field.split_once(':').unwrap_or((field, "number"));
                let ty = match code {
                    "number" => Float,
                    "whole" => PinType::Int,
                    "bool" => Bool,
                    "vector" => Vec3,
                    "color" => PinType::Color,
                    _ => return None,
                };
                Some(PinDef { name: leak(name.to_owned()), label: leak(super::title(name)), ty, default })
            })
            .collect()
    };
    let def = match node.kind.as_str() {
        "getComponent" => NodeDef {
            kind: "getComponent",
            label: "Get Component",
            category: "Find",
            description: GET,
            inputs: &[TARGET],
            outputs: Box::leak([vec![COMPONENT_OUT], pins(PinDefault::None)].concat().into_boxed_slice()),
            settings: &[COMPONENT, FIELDS],
            unique: false,
        },
        "tryGetComponent" => {
            let mut outputs = vec![PinDef::new("found", "Found", Bool), COMPONENT_OUT];
            outputs.extend(pins(PinDefault::None));
            NodeDef {
                kind: "tryGetComponent",
                label: "Try Get Component",
                category: "Find",
                description: TRY_GET,
                inputs: &[TARGET],
                outputs: Box::leak(outputs.into_boxed_slice()),
                settings: &[COMPONENT, FIELDS],
                unique: false,
            }
        }
        "setComponent" => {
            let mut inputs = vec![RUN, TARGET];
            inputs.extend(pins(PinDefault::Implicit("unchanged")));
            NodeDef {
                kind: "setComponent",
                label: "Set Component",
                category: "Actions",
                description: SET,
                inputs: Box::leak(inputs.into_boxed_slice()),
                outputs: &[THEN],
                settings: &[COMPONENT, FIELDS],
                unique: false,
            }
        }
        _ => return None,
    };
    let def: &'static NodeDef = Box::leak(Box::new(def));
    made.push((node.kind.clone(), fields, def));
    Some(def)
}

/// The pins of a node whose pins depend on its settings: a Get Component, Try Get Component or
/// Set Component node (its component's fields, or the properties of the script it names), or a
/// Function, Return or Call Function node (its function's inputs and outputs).
pub fn variant(node: &Node) -> Option<&'static NodeDef> {
    if super::FUNCTION_KINDS.contains(&node.kind.as_str()) {
        return function_variant(node);
    }
    if super::EVENT_KINDS.contains(&node.kind.as_str()) {
        return function_variant(node);
    }
    if super::COMPONENT_EVENT_KINDS.contains(&node.kind.as_str()) {
        return function_variant(node);
    }
    if !matches!(node.kind.as_str(), "getComponent" | "tryGetComponent" | "setComponent") {
        return None;
    }
    let Some(component) = component_of(node) else { return script_variant(node, script_fields(node)?) };
    match node.kind.as_str() {
        "getComponent" => Some(&component.get),
        "tryGetComponent" => Some(&component.try_get),
        "setComponent" => Some(&component.set),
        _ => None,
    }
}

/// Function node pin sets made so far, by node kind, inputs and outputs.
type FunctionVariants = Vec<(String, Vec<String>, Vec<String>, &'static NodeDef)>;
static FUNCTION_VARIANTS: std::sync::Mutex<FunctionVariants> = std::sync::Mutex::new(Vec::new());

/// The pins a list of `name:type` entries becomes; an entry of an unknown type is left out.
fn typed_pins(entries: &[String], with_defaults: bool) -> Vec<PinDef> {
    let leak = |text: String| -> &'static str { Box::leak(text.into_boxed_str()) };
    entries
        .iter()
        .filter_map(|entry| {
            let (name, code) = entry.split_once(':')?;
            let (ty, default) = match code {
                "number" => (Float, PinDefault::Float(0.0)),
                "whole" => (PinType::Int, PinDefault::Int(0)),
                "bool" => (Bool, PinDefault::Bool(false)),
                "vector" => (Vec3, PinDefault::Vec3([0.0; 3])),
                "color" => (Color, PinDefault::Color([1.0; 3])),
                "text" => (PinType::Text, PinDefault::Text("")),
                "object" => (PinType::Object, PinDefault::None),
                _ => return None,
            };
            let default = if with_defaults { default } else { PinDefault::None };
            Some(PinDef { name: leak(name.to_owned()), label: leak(super::title(name)), ty, default })
        })
        .collect()
}

/// The pins of a Function, Return or Call Function node: the function's inputs and outputs as the
/// node keeps them (`inputs` and `outputs`, each `name:type`).
pub(super) fn function_variant(node: &Node) -> Option<&'static NodeDef> {
    let list = |key: &str| -> Vec<String> {
        node.values.get(key).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|n| n.as_str().map(str::to_owned)).collect()).unwrap_or_default()
    };
    // An event node keeps its values as `params`: the inputs of a Trigger Event, the outputs of an
    // On Event.
    let (inputs, outputs) = if super::EVENT_KINDS.contains(&node.kind.as_str()) || super::COMPONENT_EVENT_KINDS.contains(&node.kind.as_str()) {
        (list("params"), Vec::new())
    } else {
        (list("inputs"), list("outputs"))
    };
    let mut made = FUNCTION_VARIANTS.lock().ok()?;
    if let Some((_, _, _, def)) = made.iter().find(|(kind, i, o, _)| *kind == node.kind && *i == inputs && *o == outputs) {
        return Some(def);
    }
    if made.len() >= MAX_SCRIPT_VARIANTS {
        return None;
    }
    let base = super::CATALOG.nodes.iter().find(|d| d.kind == node.kind)?;
    let leak_pins = |pins: Vec<PinDef>| -> &'static [PinDef] { Box::leak(pins.into_boxed_slice()) };
    let def = match node.kind.as_str() {
        "functionStart" => {
            let mut outs = vec![THEN];
            outs.extend(typed_pins(&inputs, false));
            NodeDef { outputs: leak_pins(outs), ..*base }
        }
        "functionReturn" => {
            let mut ins = vec![RUN];
            ins.extend(typed_pins(&outputs, true));
            NodeDef { inputs: leak_pins(ins), ..*base }
        }
        "callFunction" => {
            let mut ins = vec![RUN];
            ins.extend(typed_pins(&inputs, true));
            let mut outs = vec![THEN];
            outs.extend(typed_pins(&outputs, false));
            NodeDef { inputs: leak_pins(ins), outputs: leak_pins(outs), ..*base }
        }
        "eventStart" => {
            let mut outs = vec![THEN];
            outs.extend(typed_pins(&inputs, false));
            NodeDef { outputs: leak_pins(outs), ..*base }
        }
        "triggerEvent" => {
            let mut ins = vec![RUN, super::TARGET];
            ins.extend(typed_pins(&inputs, true));
            NodeDef { inputs: leak_pins(ins), ..*base }
        }
        "triggerComponentEvent" => {
            let mut ins = vec![RUN, super::COMPONENT_IN];
            ins.extend(typed_pins(&inputs, true));
            NodeDef { inputs: leak_pins(ins), ..*base }
        }
        "componentEventStart" => {
            let mut outs = vec![THEN, PinDef::new("source", "Source", PinType::Component)];
            outs.extend(typed_pins(&inputs, false));
            NodeDef { outputs: leak_pins(outs), ..*base }
        }
        _ => return None,
    };
    let def: &'static NodeDef = Box::leak(Box::new(def));
    made.push((node.kind.clone(), inputs, outputs, def));
    Some(def)
}
