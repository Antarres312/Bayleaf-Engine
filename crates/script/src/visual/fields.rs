//! The components visual scripts read and set field by field: Get Component, Try Get Component
//! and Set Component show the fields of the component named in their Component setting as pins.
//! Only fields that are numbers, vectors, colours or true/false are listed; a field's pin name is
//! its name in scene files (`intensity`, `skyColor`).

use bayleaf_graph::{Node, NodeDef, PinDef, PinDefault, PinType};

use super::{RUN, TARGET, THEN};
use PinType::{Bool, Color, Float, Vec3};

/// The Component setting of Get Component, Try Get Component and Set Component.
pub(super) const COMPONENT: bayleaf_graph::SettingDef =
    bayleaf_graph::SettingDef { name: "component", label: "Component", kind: bayleaf_graph::SettingKind::Text("Point Light") };

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
                settings: &[COMPONENT],
                unique: false,
            },
            try_get: NodeDef {
                kind: "tryGetComponent",
                label: "Try Get Component",
                category: "Find",
                description: TRY_GET,
                inputs: &[TARGET],
                outputs: &[PinDef::new("found", "Found", Bool), $(PinDef::new($field, $field_label, $ty)),*],
                settings: &[COMPONENT],
                unique: false,
            },
            set: NodeDef {
                kind: "setComponent",
                label: "Set Component",
                category: "Actions",
                description: SET,
                inputs: &[RUN, TARGET, $(PinDef::with($field, $field_label, $ty, PinDefault::Implicit("unchanged"))),*],
                outputs: &[THEN],
                settings: &[COMPONENT],
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
    component!("gameRules", "Game Rules", [("respawn", "Respawn", Bool), ("respawnHeight", "Respawn Height", Float), ("timeLimit", "Time Limit", Float)]),
];

/// The component a node's Component setting names (by its name or label; case and spaces do not
/// matter), if visual scripts can read it.
pub fn component_of(node: &Node) -> Option<&'static ComponentNodes> {
    let named = node.values.get("component").and_then(serde_json::Value::as_str).unwrap_or("Point Light");
    let key = crate::kind_key(named);
    COMPONENTS.iter().find(|c| crate::kind_key(c.name) == key || crate::kind_key(c.label) == key)
}

/// The pins of a Get Component, Try Get Component or Set Component node: its component's fields.
pub fn variant(node: &Node) -> Option<&'static NodeDef> {
    let component = component_of(node)?;
    match node.kind.as_str() {
        "getComponent" => Some(&component.get),
        "tryGetComponent" => Some(&component.try_get),
        "setComponent" => Some(&component.set),
        _ => None,
    }
}
