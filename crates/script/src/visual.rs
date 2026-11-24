//! Visual scripts: their node kinds and the compiler that turns a graph into a WebAssembly module
//! with the same ABI as any other script (`docs/script-abi.md`).
//!
//! Event nodes (On Start, On Update) begin chains of actions joined by flow links. Data nodes are
//! pure: they are computed where an action needs them. The compiler writes WebAssembly text and
//! assembles it, so the result runs under the same fuel and memory limits as hand-written scripts.

use std::fmt::Write as _;

use bayleaf_graph::{Catalog, Graph, NodeDef, PinDef, PinDefault, PinType, Problem, SettingDef, SettingKind, Shape, VariableType};
use bayleaf_scene::components::{DeclaredProperty, PropertyShape, PropertyType, PropertyValue};

use PinType::{Any, Bool, Float, Flow, Vec3};

mod collections;
pub use collections::adopted_item;
mod fields;
pub use fields::{COMPONENTS, ComponentNodes, component_of, script_fields};

const fn pin(name: &'static str, label: &'static str, ty: PinType) -> PinDef {
    PinDef::new(name, label, ty)
}

const fn number(name: &'static str, label: &'static str, ty: PinType, value: f32) -> PinDef {
    PinDef::with(name, label, ty, PinDefault::Float(value))
}

const fn vector(name: &'static str, label: &'static str, value: [f32; 3]) -> PinDef {
    PinDef::with(name, label, Vec3, PinDefault::Vec3(value))
}

const fn text(name: &'static str, label: &'static str, value: &'static str) -> PinDef {
    PinDef::with(name, label, PinType::Text, PinDefault::Text(value))
}

const TEXT_OUT: &[PinDef] = &[pin("result", "Result", PinType::Text)];

const RUN: PinDef = pin("run", "Run", Flow);
const THEN: PinDef = pin("then", "Then", Flow);
const MATH_OUT: &[PinDef] = &[pin("result", "Result", Any)];
const MESSAGE: SettingDef = SettingDef { name: "message", label: "Message", kind: SettingKind::Name("hit") };
const CONTACT_OUT: &[PinDef] = &[THEN, pin("other", "Other Object", PinType::Object)];
/// An event node for a physics contact: what it runs after, and the other object.
const fn contact(kind: &'static str, label: &'static str, description: &'static str) -> NodeDef {
    NodeDef { kind, label, category: "Physics", description, inputs: &[], outputs: CONTACT_OUT, settings: &[], unique: false }
}

/// Whether an event node runs on a message: On Message, the interface's events and contacts.
fn takes_messages(kind: &str) -> bool {
    matches!(kind, "message" | "interfacePressed" | "interfaceChanged" | "eventStart" | "componentEventStart") || contact_message(kind).is_some()
}

/// The message the physics step sends for a contact event node.
fn contact_message(kind: &str) -> Option<&'static str> {
    match kind {
        "triggerEntered" => Some("triggerEnter"),
        "triggerLeft" => Some("triggerExit"),
        "collisionStarted" => Some("collision"),
        "collisionEnded" => Some("collisionEnd"),
        _ => None,
    }
}
const VARIABLE: SettingDef = SettingDef { name: "name", label: "Name", kind: SettingKind::Name("count") };
const VECTOR_VARIABLE: SettingDef = SettingDef { name: "name", label: "Name", kind: SettingKind::Name("target") };
/// An action of an Input Map: `Jump` in the active map, or `Player/Jump`, live only while Player is active.
const ACTION: SettingDef = SettingDef { name: "action", label: "Action", kind: SettingKind::Text("Jump") };
const STATE_NAME: SettingDef = SettingDef { name: "name", label: "Name", kind: SettingKind::Name("score") };
/// Whose value: the game's, a player's (the one named in Player, else the first), this object's own
/// when it is a player, or a saved one that lasts from one play to the next.
const STATE_SCOPE: SettingDef = SettingDef { name: "scope", label: "Scope", kind: SettingKind::Choice(&["Game", "Player", "This Object", "Saved"]) };
const STATE_PLAYER: SettingDef = SettingDef { name: "player", label: "Player", kind: SettingKind::Text("") };
const FLAG: SettingDef = SettingDef { name: "name", label: "Name", kind: SettingKind::Name("done") };
/// A scene file of the project, relative to its folder.
const SCENE_FILE: SettingDef = SettingDef { name: "scene", label: "Scene File", kind: SettingKind::Text("") };
const TEXT_VARIABLE: SettingDef = SettingDef { name: "name", label: "Name", kind: SettingKind::Name("label") };
const OBJECT_VARIABLE: SettingDef = SettingDef { name: "name", label: "Name", kind: SettingKind::Name("thing") };
const COLOR_VARIABLE: SettingDef = SettingDef { name: "name", label: "Name", kind: SettingKind::Name("tint") };
const WHOLE_VARIABLE: SettingDef = SettingDef { name: "name", label: "Name", kind: SettingKind::Name("total") };
/// Most bytes a text variable keeps; longer text is cut at a character.
pub const TEXT_VARIABLE_BYTES: u32 = 1024;
/// The object an action works on or a value is read from, from a search or This Object; when it is not
/// linked, the node uses the object named in Object Name (or its own default).
pub(crate) const TARGET: PinDef = pin("target", "Object", PinType::Object);
/// The property a Property or Set Property node names, declared in the script's panel.
const PROPERTY_NAME: SettingDef = SettingDef { name: "name", label: "Property", kind: SettingKind::Name("") };

/// A Property node of one type: the property's value as an output.
macro_rules! get_property {
    ($kind:literal, $label:literal, $description:literal, $ty:expr) => {
        NodeDef {
            kind: $kind,
            label: $label,
            category: "Properties",
            description: $description,
            inputs: &[],
            outputs: &[pin("value", "Value", $ty)],
            settings: &[PROPERTY_NAME],
            unique: false,
        }
    };
}

/// A Set Property node of one type: sets the property to its Value input.
macro_rules! set_property {
    ($kind:literal, $label:literal, $description:literal, $value:expr) => {
        NodeDef {
            kind: $kind,
            label: $label,
            category: "Properties",
            description: $description,
            inputs: &[RUN, $value],
            outputs: &[THEN],
            settings: &[PROPERTY_NAME],
            unique: false,
        }
    };
}
/// The function a Function, Return or Call Function node belongs to, and its pins (`name:type`),
/// kept in step with the function by `graph.function.set`.
pub(crate) const FUNCTION_SETTINGS: [SettingDef; 3] = [
    SettingDef { name: "function", label: "Function", kind: SettingKind::Name("") },
    SettingDef { name: "inputs", label: "Inputs", kind: SettingKind::Names },
    SettingDef { name: "outputs", label: "Outputs", kind: SettingKind::Names },
];

/// The node kinds that name a function in their `function` setting.
pub const FUNCTION_KINDS: [&str; 3] = ["functionStart", "functionReturn", "callFunction"];
/// The event an On Event or Trigger Event node names, and its values' pins (`name:type`), kept in
/// step with the graph's declaration by `graph.event.set`.
pub(crate) const EVENT_SETTINGS: [SettingDef; 2] =
    [SettingDef { name: "event", label: "Event", kind: SettingKind::Name("") }, SettingDef { name: "params", label: "Values", kind: SettingKind::Names }];
/// The node kinds that name an event in their `event` setting.
pub const EVENT_KINDS: [&str; 2] = ["eventStart", "triggerEvent"];
/// A component input: a script on an object, from a Get Component that names a script or This
/// Component.
pub(crate) const COMPONENT_IN: PinDef = pin("component", "Component", PinType::Component);
/// The script whose event a node names (its name as the Inspector shows it), the event, and its
/// values' pins (`name:type`). The Graph panel picks the event from the script's own list.
pub(crate) const COMPONENT_EVENT_SETTINGS: [SettingDef; 3] =
    [SettingDef { name: "script", label: "Script", kind: SettingKind::Text("") }, EVENT_SETTINGS[0], EVENT_SETTINGS[1]];
/// The node kinds that name another script's event, with values: triggering it, and hearing it.
pub const COMPONENT_EVENT_KINDS: [&str; 2] = ["triggerComponentEvent", "componentEventStart"];
/// Every node kind that names another script's event (subscribing too).
pub const SCRIPT_EVENT_KINDS: [&str; 4] = ["triggerComponentEvent", "componentEventStart", "subscribeEvent", "unsubscribeEvent"];
/// The list a list node works on, one the script declares, and the type code of what it holds
/// (`["number"]`), kept in step with the declaration by `graph.variable.set` and `graph.property.set`.
pub(crate) const LIST_SETTINGS: [SettingDef; 2] =
    [SettingDef { name: "name", label: "List", kind: SettingKind::Name("") }, SettingDef { name: "item", label: "Holds", kind: SettingKind::Names }];
/// The map a map node works on, and what it holds, as for a list.
pub(crate) const MAP_SETTINGS: [SettingDef; 2] =
    [SettingDef { name: "name", label: "Map", kind: SettingKind::Name("") }, SettingDef { name: "item", label: "Holds", kind: SettingKind::Names }];
/// The node kinds that work on a declared list.
pub const LIST_KINDS: [&str; 10] =
    ["getList", "listAdd", "listInsert", "listSet", "listRemove", "listClear", "listItem", "listLength", "listFind", "listForEach"];
/// The node kinds that work on a declared map.
pub const MAP_KINDS: [&str; 7] = ["getMap", "mapSet", "mapRemove", "mapClear", "mapGet", "mapCount", "mapForEach"];
/// Every node kind that names a declared list or map in its `name` setting.
pub const COLLECTION_KINDS: [&str; 17] = [
    "getList",
    "getMap",
    "listAdd",
    "listInsert",
    "listSet",
    "listRemove",
    "listClear",
    "listItem",
    "listLength",
    "listFind",
    "listForEach",
    "mapSet",
    "mapRemove",
    "mapClear",
    "mapGet",
    "mapCount",
    "mapForEach",
];
/// A list index, from 0.
const INDEX: PinDef = PinDef::with("index", "Index", PinType::Int, PinDefault::Int(0));
/// A map key.
const KEY: PinDef = text("key", "Key", "");
/// The list a list node works on, as a pin (of numbers here; a node's own type is its `item`).
const LIST: PinDef = PinDef::new("list", "List", PinType::List(VariableType::Number));
/// The map a map node works on, as a pin.
const MAP: PinDef = PinDef::new("map", "Map", PinType::Map(VariableType::Text, VariableType::Number));
/// The value a list or map node puts in, and gives.
const VALUE_IN: PinDef = number("value", "Value", Float, 0.0);
const VALUE_OUT: PinDef = pin("value", "Value", Float);
/// A list node.
const fn list_node(kind: &'static str, label: &'static str, description: &'static str, inputs: &'static [PinDef], outputs: &'static [PinDef]) -> NodeDef {
    NodeDef { kind, label, category: "Lists", description, inputs, outputs, settings: &LIST_SETTINGS, unique: false }
}
/// A map node.
const fn map_node(kind: &'static str, label: &'static str, description: &'static str, inputs: &'static [PinDef], outputs: &'static [PinDef]) -> NodeDef {
    NodeDef { kind, label, category: "Maps", description, inputs, outputs, settings: &MAP_SETTINGS, unique: false }
}
const OBJECT_NAME: SettingDef = SettingDef { name: "object", label: "Object Name", kind: SettingKind::Text("") };
/// The interface element a node works on, by its name.
const ELEMENT: SettingDef = SettingDef { name: "element", label: "Element", kind: SettingKind::Text("Start") };
/// What a search looks for: a component (Point Light, Character), a tag, or a script (by its name).
const KIND: SettingDef = SettingDef { name: "kind", label: "Component, Tag or Script", kind: SettingKind::Text("Point Light") };

const fn node(
    kind: &'static str,
    label: &'static str,
    category: &'static str,
    description: &'static str,
    inputs: &'static [PinDef],
    outputs: &'static [PinDef],
) -> NodeDef {
    NodeDef { kind, label, category, description, inputs, outputs, settings: &[], unique: false }
}

const fn action(kind: &'static str, label: &'static str, description: &'static str, inputs: &'static [PinDef]) -> NodeDef {
    NodeDef { kind, label, category: "Actions", description, inputs, outputs: &[THEN], settings: &[], unique: false }
}

/// The node kinds of visual scripts.
pub const CATALOG: Catalog = Catalog {
    domain: "script",
    label: "Visual Script",
    extension: "bscript",
    starter: &[("start", [40.0, 40.0]), ("update", [40.0, 200.0])],
    variant: fields::variant,
    upgrade,
    fractional: &["sine", "squareRoot", "lerp"],
    nodes: &[
        node(
            "init",
            "On Init",
            "Events",
            "Runs once when the object is ready, before any script's On Start: every script of the scene runs On Init first.",
            &[],
            &[THEN],
        ),
        node("start", "On Start", "Events", "Runs once when play mode starts, after every script ran On Init.", &[], &[THEN]),
        node("update", "On Update", "Events", "Runs every simulation step.", &[], &[THEN, pin("dt", "Step Seconds", Float)]),
        NodeDef {
            kind: "message",
            label: "On Message",
            category: "Events",
            description: "Runs when a script sends this message to this object, or to every object. Value is the number sent with it.",
            inputs: &[],
            outputs: &[THEN, pin("value", "Value", Float)],
            settings: &[MESSAGE],
            unique: false,
        },
        contact(
            "triggerEntered",
            "On Trigger Entered",
            "Runs when another collider comes into this object's trigger, or this object comes into another's (needs physics).",
        ),
        contact("triggerLeft", "On Trigger Left", "Runs when another collider leaves this object's trigger, or this object leaves another's (needs physics)."),
        contact("collisionStarted", "On Collision Started", "Runs when this object's collider starts touching another (needs physics)."),
        contact("collisionEnded", "On Collision Ended", "Runs when this object's collider stops touching another (needs physics)."),
        NodeDef {
            kind: "sendMessage",
            label: "Send Message",
            category: "Actions",
            description: "Sends a message and a number to the scripts of the linked object, else the first object named in Object Name, else every object. It arrives after this step.",
            inputs: &[RUN, TARGET, number("value", "Value", Float, 0.0)],
            outputs: &[THEN],
            settings: &[MESSAGE, OBJECT_NAME],
            unique: false,
        },
        NodeDef {
            kind: "action",
            label: "Action",
            category: "Inputs",
            description: "An input action's value by its kind (a trigger 1 in the step it goes down, held 1 while down, value how far it is pressed), and whether it is down. Name it Jump for the active map, or Player/Jump; actions of maps that are not active read 0.",
            inputs: &[],
            outputs: &[pin("value", "Value", Float), pin("held", "Held", Bool)],
            settings: &[ACTION],
            unique: false,
        },
        NodeDef {
            kind: "actionPressed",
            label: "On Action Pressed",
            category: "Events",
            description: "Runs once in the step an input action goes down, while its map is active.",
            inputs: &[],
            outputs: &[THEN],
            settings: &[ACTION],
            unique: false,
        },
        NodeDef {
            kind: "keyPressed",
            label: "On Key Pressed",
            category: "Events",
            description: "Runs once in the step when the key goes down (names like W, Space, ArrowUp).",
            inputs: &[],
            outputs: &[THEN],
            settings: &[SettingDef { name: "key", label: "Key", kind: SettingKind::Name("Space") }],
            unique: false,
        },
        NodeDef {
            kind: "timer",
            label: "On Timer",
            category: "Events",
            description: "Runs every given number of seconds of play.",
            inputs: &[],
            outputs: &[THEN],
            settings: &[SettingDef { name: "seconds", label: "Seconds", kind: SettingKind::Float(1.0) }],
            unique: false,
        },
        NodeDef {
            kind: "setVectorVariable",
            label: "Set Vector Variable",
            category: "Variables",
            description: "Keeps a vector under a name, for this script on this object. It starts where the Variables panel says, or at 0, 0, 0.",
            inputs: &[RUN, vector("value", "Value", [0.0; 3])],
            outputs: &[THEN],
            settings: &[VECTOR_VARIABLE],
            unique: false,
        },
        NodeDef {
            kind: "vectorVariable",
            label: "Vector Variable",
            category: "Variables",
            description: "The vector Set Vector Variable last kept under this name, or 0, 0, 0.",
            inputs: &[],
            outputs: &[pin("value", "Value", Vec3)],
            settings: &[VECTOR_VARIABLE],
            unique: false,
        },
        NodeDef {
            kind: "setFlag",
            label: "Set True/False Variable",
            category: "Variables",
            description: "Keeps true or false under a name, for this script on this object. It starts as the Variables panel says, or false.",
            inputs: &[RUN, PinDef::with("value", "Value", Bool, PinDefault::Bool(false))],
            outputs: &[THEN],
            settings: &[FLAG],
            unique: false,
        },
        NodeDef {
            kind: "flag",
            label: "True/False Variable",
            category: "Variables",
            description: "What Set True/False Variable last kept under this name, or false.",
            inputs: &[],
            outputs: &[pin("value", "Value", Bool)],
            settings: &[FLAG],
            unique: false,
        },
        NodeDef {
            kind: "setWholeVariable",
            label: "Set Whole Number Variable",
            category: "Variables",
            description: "Keeps a whole number under a name, for this script on this object. It starts where the Variables panel says, or at 0.",
            inputs: &[RUN, PinDef::with("value", "Value", PinType::Int, PinDefault::Int(0))],
            outputs: &[THEN],
            settings: &[WHOLE_VARIABLE],
            unique: false,
        },
        NodeDef {
            kind: "wholeVariable",
            label: "Whole Number Variable",
            category: "Variables",
            description: "The whole number Set Whole Number Variable last kept under this name, or what it starts at.",
            inputs: &[],
            outputs: &[pin("value", "Value", PinType::Int)],
            settings: &[WHOLE_VARIABLE],
            unique: false,
        },
        NodeDef {
            kind: "setTextVariable",
            label: "Set Text Variable",
            category: "Variables",
            description: "Keeps text under a name, for this script on this object (up to 1024 bytes; longer text is cut). It starts as the Variables panel says, or empty.",
            inputs: &[RUN, text("value", "Value", "")],
            outputs: &[THEN],
            settings: &[TEXT_VARIABLE],
            unique: false,
        },
        NodeDef {
            kind: "textVariable",
            label: "Text Variable",
            category: "Variables",
            description: "The text Set Text Variable last kept under this name, or what it starts as.",
            inputs: &[],
            outputs: &[pin("value", "Value", PinType::Text)],
            settings: &[TEXT_VARIABLE],
            unique: false,
        },
        NodeDef {
            kind: "setObjectVariable",
            label: "Set Object Variable",
            category: "Variables",
            description: "Keeps an object under a name, for this script on this object, from step to step. It starts as none.",
            inputs: &[RUN, pin("value", "Value", PinType::Object)],
            outputs: &[THEN],
            settings: &[OBJECT_VARIABLE],
            unique: false,
        },
        NodeDef {
            kind: "objectVariable",
            label: "Object Variable",
            category: "Variables",
            description: "The object Set Object Variable last kept under this name; none (Found false) before that, or once the object is gone.",
            inputs: &[],
            outputs: &[pin("value", "Value", PinType::Object), pin("found", "Found", Bool)],
            settings: &[OBJECT_VARIABLE],
            unique: false,
        },
        NodeDef {
            kind: "setColorVariable",
            label: "Set Color Variable",
            category: "Variables",
            description: "Keeps a colour under a name, for this script on this object. It starts as the Variables panel says, or white.",
            inputs: &[RUN, PinDef::with("value", "Value", PinType::Color, PinDefault::Color([1.0; 3]))],
            outputs: &[THEN],
            settings: &[COLOR_VARIABLE],
            unique: false,
        },
        NodeDef {
            kind: "colorVariable",
            label: "Color Variable",
            category: "Variables",
            description: "The colour Set Color Variable last kept under this name, or what it starts as.",
            inputs: &[],
            outputs: &[pin("value", "Value", PinType::Color)],
            settings: &[COLOR_VARIABLE],
            unique: false,
        },
        NodeDef {
            kind: "setVariable",
            label: "Set Variable",
            category: "Variables",
            description: "Keeps a number under a name, for this script on this object. It starts where the Variables panel says, or at 0.",
            inputs: &[RUN, number("value", "Value", Float, 0.0)],
            outputs: &[THEN],
            settings: &[VARIABLE],
            unique: false,
        },
        NodeDef {
            kind: "variable",
            label: "Variable",
            category: "Variables",
            description: "The number Set Variable last kept under this name, or 0.",
            inputs: &[],
            outputs: &[pin("value", "Value", Float)],
            settings: &[VARIABLE],
            unique: false,
        },
        NodeDef {
            kind: "log",
            label: "Log",
            category: "Actions",
            description: "Writes the message, and the value if one is linked, to the console.",
            inputs: &[RUN, text("text", "Text", ""), pin("value", "Value", Float)],
            outputs: &[THEN],
            settings: &[SettingDef { name: "message", label: "Message", kind: SettingKind::Text("Hello") }],
            unique: false,
        },
        action("setPosition", "Set Position", "Moves the object to a position (relative to its parent).", &[RUN, vector("position", "Position", [0.0; 3])]),
        action("moveBy", "Move By", "Moves the object by an offset.", &[RUN, vector("offset", "Offset", [0.0; 3])]),
        action("setRotation", "Set Rotation", "Sets the rotation, in degrees about X, Y and Z.", &[RUN, vector("rotation", "Rotation", [0.0; 3])]),
        action("rotateBy", "Rotate By", "Turns the object by degrees about X, Y and Z.", &[RUN, vector("degrees", "Degrees", [0.0; 3])]),
        action("setScale", "Set Scale", "Sets the size along X, Y and Z.", &[RUN, vector("scale", "Scale", [1.0; 3])]),
        NodeDef {
            kind: "branch",
            label: "Branch",
            category: "Flow",
            description: "Continues with True or False, depending on the condition.",
            inputs: &[RUN, PinDef::with("condition", "Condition", Bool, PinDefault::Bool(false))],
            outputs: &[pin("true", "True", Flow), pin("false", "False", Flow)],
            settings: &[],
            unique: false,
        },
        NodeDef {
            kind: "number",
            label: "Number",
            category: "Values",
            description: "A fixed number.",
            inputs: &[],
            outputs: &[pin("value", "Value", Float)],
            settings: &[SettingDef { name: "value", label: "Value", kind: SettingKind::Float(1.0) }],
            unique: false,
        },
        node(
            "vector",
            "Make Vector",
            "Values",
            "A vector from three numbers.",
            &[number("x", "X", Float, 0.0), number("y", "Y", Float, 0.0), number("z", "Z", Float, 0.0)],
            &[pin("vector", "Vector", Vec3)],
        ),
        node(
            "split",
            "Split Vector",
            "Values",
            "The three numbers of a vector.",
            &[pin("vector", "Vector", Vec3)],
            &[pin("x", "X", Float), pin("y", "Y", Float), pin("z", "Z", Float)],
        ),
        get_property!("property", "Property", "A number property: set per object in the Inspector, else its default in the script's panel.", Float),
        get_property!("wholeProperty", "Whole Number Property", "A whole-number property: set per object in the Inspector, else its default.", PinType::Int),
        get_property!("flagProperty", "True/False Property", "A true/false property: set per object in the Inspector, else its default.", Bool),
        get_property!("vectorProperty", "Vector Property", "A vector property: set per object in the Inspector, else its default.", Vec3),
        get_property!("colorProperty", "Color Property", "A colour property: set per object in the Inspector, else its default.", PinType::Color),
        get_property!("textProperty", "Text Property", "A text property: set per object in the Inspector, else its default.", PinType::Text),
        NodeDef {
            kind: "objectProperty",
            label: "Object Property",
            category: "Properties",
            description: "An object property: the object of the scene chosen per object in the Inspector (by its name); none (Found false) when none is chosen or it is not there.",
            inputs: &[],
            outputs: &[pin("value", "Value", PinType::Object), pin("found", "Found", Bool)],
            settings: &[PROPERTY_NAME],
            unique: false,
        },
        set_property!(
            "setProperty",
            "Set Property",
            "Sets this script's number property on this object, for the rest of play.",
            number("value", "Value", Float, 0.0)
        ),
        set_property!(
            "setWholeProperty",
            "Set Whole Number Property",
            "Sets this script's whole-number property on this object, for the rest of play.",
            PinDef::with("value", "Value", PinType::Int, PinDefault::Int(0))
        ),
        set_property!(
            "setFlagProperty",
            "Set True/False Property",
            "Sets this script's true/false property on this object, for the rest of play.",
            PinDef::with("value", "Value", Bool, PinDefault::Bool(false))
        ),
        set_property!(
            "setVectorProperty",
            "Set Vector Property",
            "Sets this script's vector property on this object, for the rest of play.",
            vector("value", "Value", [0.0; 3])
        ),
        set_property!(
            "setColorProperty",
            "Set Color Property",
            "Sets this script's colour property on this object, for the rest of play.",
            PinDef::with("value", "Value", PinType::Color, PinDefault::Color([1.0; 3]))
        ),
        set_property!(
            "setTextProperty",
            "Set Text Property",
            "Sets this script's text property on this object, for the rest of play.",
            text("value", "Value", "")
        ),
        set_property!(
            "setObjectProperty",
            "Set Object Property",
            "Sets this script's object property on this object to the linked object (by its name), for the rest of play.",
            pin("value", "Object", PinType::Object)
        ),
        NodeDef {
            kind: "functionStart",
            label: "Function",
            category: "Functions",
            description: "Where a function's actions begin; its outputs are the values the function is called with.",
            inputs: &[],
            outputs: &[THEN],
            settings: &FUNCTION_SETTINGS,
            unique: false,
        },
        NodeDef {
            kind: "functionReturn",
            label: "Return",
            category: "Functions",
            description: "Ends the function and gives back its outputs to the Call Function that ran it.",
            inputs: &[RUN],
            outputs: &[],
            settings: &FUNCTION_SETTINGS,
            unique: false,
        },
        NodeDef {
            kind: "callFunction",
            label: "Call Function",
            category: "Functions",
            description: "Runs a function of this script with its inputs, then goes on; its outputs are what the function gave back.",
            inputs: &[RUN],
            outputs: &[THEN],
            settings: &FUNCTION_SETTINGS,
            unique: false,
        },
        NodeDef {
            kind: "eventStart",
            label: "On Event",
            category: "Events",
            description: "Runs when the event is triggered on this object (by a script's Trigger Event, by the interface, or by every object's); its outputs are the values the event brings.",
            inputs: &[],
            outputs: &[THEN],
            settings: &EVENT_SETTINGS,
            unique: false,
        },
        NodeDef {
            kind: "triggerEvent",
            label: "Trigger Event",
            category: "Actions",
            description: "Triggers an event with its values on the linked object, else the first object named in Object Name, else every object; their On Event nodes run after this step.",
            inputs: &[RUN, TARGET],
            outputs: &[THEN],
            settings: &[EVENT_SETTINGS[0], EVENT_SETTINGS[1], OBJECT_NAME],
            unique: false,
        },
        node(
            "thisComponent",
            "This Component",
            "Find",
            "This script, as a component of its object: what other scripts trigger events on and subscribe to.",
            &[],
            &[pin("component", "Component", PinType::Component)],
        ),
        node(
            "componentObject",
            "Object of Component",
            "Find",
            "The object a component is on, or none.",
            &[COMPONENT_IN],
            &[pin("object", "Object", PinType::Object)],
        ),
        NodeDef {
            kind: "triggerComponentEvent",
            label: "Trigger Component Event",
            category: "Actions",
            description: "Triggers an event its script declares on one component (a script on an object), with its values; the script's On Event, and the scripts subscribed to it, run after this step. The event is picked from the script's own list.",
            inputs: &[RUN, COMPONENT_IN],
            outputs: &[THEN],
            settings: &COMPONENT_EVENT_SETTINGS,
            unique: false,
        },
        NodeDef {
            kind: "subscribeEvent",
            label: "Subscribe to Event",
            category: "Actions",
            description: "From now on, this script hears the event whenever it happens on the component, with On Component Event; once per component and event.",
            inputs: &[RUN, COMPONENT_IN],
            outputs: &[THEN],
            settings: &COMPONENT_EVENT_SETTINGS,
            unique: false,
        },
        NodeDef {
            kind: "unsubscribeEvent",
            label: "Unsubscribe from Event",
            category: "Actions",
            description: "Stops this script hearing the event of the component it subscribed to.",
            inputs: &[RUN, COMPONENT_IN],
            outputs: &[THEN],
            settings: &COMPONENT_EVENT_SETTINGS,
            unique: false,
        },
        NodeDef {
            kind: "componentEventStart",
            label: "On Component Event",
            category: "Events",
            description: "Runs when the event happens on a component this script subscribed to; Source is that component, and the other outputs are the values the event brings.",
            inputs: &[],
            outputs: &[THEN],
            settings: &COMPONENT_EVENT_SETTINGS,
            unique: false,
        },
        list_node("getList", "Get List", "The list itself, to pass on: into a list node, a function or an event.", &[], &[LIST]),
        list_node("listAdd", "Add to List", "Puts the value at the end of the list: the one linked in, else the one named.", &[RUN, LIST, VALUE_IN], &[THEN]),
        list_node(
            "listInsert",
            "Insert into List",
            "Puts the value in the list at the index (from 0), moving the items from there on along; an index past the end does nothing.",
            &[RUN, LIST, INDEX, VALUE_IN],
            &[THEN],
        ),
        list_node(
            "listSet",
            "Set List Item",
            "Replaces the list's item at the index (from 0); an index past the end does nothing.",
            &[RUN, LIST, INDEX, VALUE_IN],
            &[THEN],
        ),
        list_node(
            "listRemove",
            "Remove from List",
            "Takes the item at the index (from 0) out of the list; the items after it move back.",
            &[RUN, LIST, INDEX],
            &[THEN],
        ),
        list_node("listClear", "Clear List", "Takes every item out of the list.", &[RUN, LIST], &[THEN]),
        list_node(
            "listItem",
            "List Item",
            "The list's item at the index (from 0). Past the end, Found is false and the value is empty.",
            &[LIST, INDEX],
            &[VALUE_OUT, pin("found", "Found", Bool)],
        ),
        list_node("listLength", "List Length", "How many items the list holds.", &[LIST], &[pin("length", "Length", PinType::Int)]),
        list_node(
            "listFind",
            "Find in List",
            "The index of the first item equal to the value, or -1 when there is none.",
            &[LIST, VALUE_IN],
            &[pin("index", "Index", PinType::Int)],
        ),
        list_node(
            "listForEach",
            "For Each in List",
            "Runs Each once for every item of the list, in order, with the item and its index (from 0); then runs Then. Items added or taken out on the way count.",
            &[RUN, LIST],
            &[pin("each", "Each", Flow), VALUE_OUT, pin("index", "Index", PinType::Int), THEN],
        ),
        map_node("getMap", "Get Map", "The map itself, to pass on: into a map node, a function or an event.", &[], &[MAP]),
        map_node(
            "mapSet",
            "Set Map Value",
            "Keeps the value under the key, in place of what was there, in the map linked in, else the one named.",
            &[RUN, MAP, KEY, VALUE_IN],
            &[THEN],
        ),
        map_node("mapRemove", "Remove from Map", "Takes the key, and its value, out of the map.", &[RUN, MAP, KEY], &[THEN]),
        map_node("mapClear", "Clear Map", "Takes every key out of the map.", &[RUN, MAP], &[THEN]),
        map_node(
            "mapGet",
            "Map Value",
            "The value under the key. When the map has no such key, Found is false and the value is empty.",
            &[MAP, KEY],
            &[VALUE_OUT, pin("found", "Found", Bool)],
        ),
        map_node("mapCount", "Map Count", "How many keys the map holds.", &[MAP], &[pin("count", "Count", PinType::Int)]),
        map_node(
            "mapForEach",
            "For Each in Map",
            "Runs Each once for every key of the map, in the keys' order (sorted by their characters), with the key and its value; then runs Then. Keys added or taken out on the way count.",
            &[RUN, MAP],
            &[pin("each", "Each", Flow), pin("key", "Key", PinType::Text), VALUE_OUT, THEN],
        ),
        node("position", "Position", "Object", "The object's position, relative to its parent.", &[], &[pin("position", "Position", Vec3)]),
        node("rotation", "Rotation", "Object", "The object's rotation in degrees.", &[], &[pin("rotation", "Rotation", Vec3)]),
        node("scale", "Scale", "Object", "The object's size along X, Y and Z.", &[], &[pin("scale", "Scale", Vec3)]),
        node("time", "Time", "Inputs", "Seconds since play mode started.", &[], &[pin("seconds", "Seconds", Float)]),
        node(
            "pointer",
            "Pointer Position",
            "Inputs",
            "Where the pointer is over the game view, from -1 to 1 (x right, y up), whether it is over the view, and whether it is over an interface element that takes it (its clicks are then the interface's, not the game's).",
            &[],
            &[pin("position", "Position", Vec3), pin("inside", "Over the View", Bool), pin("overInterface", "Over the Interface", Bool)],
        ),
        NodeDef {
            kind: "mouseDown",
            label: "Mouse Button Held",
            category: "Inputs",
            description: "True while the mouse button (Left, Right or Middle) is held.",
            inputs: &[],
            outputs: &[pin("down", "Held", Bool)],
            settings: &[SettingDef { name: "button", label: "Button", kind: SettingKind::Name("Left") }],
            unique: false,
        },
        NodeDef {
            kind: "positionOf",
            label: "Position Of",
            category: "Object",
            description: "The position of the linked object, else the first object named in Object Name (relative to its parent).",
            inputs: &[TARGET],
            outputs: &[pin("position", "Position", Vec3)],
            settings: &[OBJECT_NAME],
            unique: false,
        },
        NodeDef {
            kind: "setPositionOf",
            label: "Set Position Of",
            category: "Actions",
            description: "Moves the linked object, else the first object named in Object Name, to a position.",
            inputs: &[RUN, TARGET, vector("position", "Position", [0.0; 3])],
            outputs: &[THEN],
            settings: &[OBJECT_NAME],
            unique: false,
        },
        NodeDef {
            kind: "spawn",
            label: "Spawn Prefab",
            category: "Actions",
            description: "Places a copy of a prefab file at a world position, after this step.",
            inputs: &[RUN, vector("position", "Position", [0.0; 3])],
            outputs: &[THEN],
            settings: &[SettingDef { name: "prefab", label: "Prefab File", kind: SettingKind::Text("") }],
            unique: false,
        },
        node("thisObject", "This Object", "Find", "The object this script runs on.", &[], &[pin("object", "Object", PinType::Object)]),
        NodeDef {
            kind: "findNamed",
            label: "Find Object Named",
            category: "Find",
            description: "The first object with this name, in hierarchy order, as the step began. Found is false when there is none.",
            inputs: &[text("named", "Name", "")],
            outputs: &[pin("object", "Object", PinType::Object), pin("found", "Found", Bool)],
            settings: &[SettingDef { name: "name", label: "Name", kind: SettingKind::Text("Player") }],
            unique: false,
        },
        NodeDef {
            kind: "findWith",
            label: "Find Object With",
            category: "Find",
            description: "An object that has a component (Point Light, Character…), a tag, or runs a script (by its name), in hierarchy order as the step began: Index 0 is the first. Found is false when there is none.",
            inputs: &[number("index", "Index", Float, 0.0)],
            outputs: &[pin("object", "Object", PinType::Object), pin("found", "Found", Bool)],
            settings: &[KIND],
            unique: false,
        },
        NodeDef {
            kind: "countWith",
            label: "Count Objects With",
            category: "Find",
            description: "How many objects have a component (Point Light, Character…), a tag, or run a script (by its name), as the step began.",
            inputs: &[],
            outputs: &[pin("count", "Count", PinType::Int)],
            settings: &[KIND],
            unique: false,
        },
        NodeDef {
            kind: "forEachWith",
            label: "For Each Object With",
            category: "Find",
            description: "Runs Each once for every object that has a component, a tag, or runs a script, in hierarchy order as the step began, with that object and its index (from 0); then runs Then.",
            inputs: &[RUN],
            outputs: &[pin("each", "Each", Flow), pin("object", "Object", PinType::Object), pin("index", "Index", PinType::Int), THEN],
            settings: &[KIND],
            unique: false,
        },
        NodeDef {
            kind: "destroyObject",
            label: "Destroy Object",
            category: "Actions",
            description: "Deletes the linked object (else this object) and its children after this step.",
            inputs: &[RUN, TARGET],
            outputs: &[THEN],
            settings: &[],
            unique: false,
        },
        NodeDef {
            kind: "getComponent",
            label: "Get Component",
            category: "Find",
            description: fields::GET,
            inputs: &[TARGET],
            outputs: &[],
            settings: &[fields::COMPONENT, fields::FIELDS],
            unique: false,
        },
        NodeDef {
            kind: "tryGetComponent",
            label: "Try Get Component",
            category: "Find",
            description: fields::TRY_GET,
            inputs: &[TARGET],
            outputs: &[pin("found", "Found", Bool)],
            settings: &[fields::COMPONENT, fields::FIELDS],
            unique: false,
        },
        NodeDef {
            kind: "setComponent",
            label: "Set Component",
            category: "Actions",
            description: fields::SET,
            inputs: &[RUN, TARGET],
            outputs: &[THEN],
            settings: &[fields::COMPONENT, fields::FIELDS],
            unique: false,
        },
        action("playSound", "Play Sound", "Plays the Audio Source of the linked object (else this object), after this step.", &[RUN, TARGET]),
        action("stopSound", "Stop Sound", "Stops the sounds the linked object (else this object) plays, after this step.", &[RUN, TARGET]),
        NodeDef {
            kind: "playSoundAt",
            label: "Play Sound At",
            category: "Actions",
            description: "Plays a sound file once from a world position, after this step, heard from the first camera.",
            inputs: &[RUN, vector("position", "Position", [0.0; 3]), number("volume", "Volume", Float, 1.0)],
            outputs: &[THEN],
            settings: &[SettingDef { name: "sound", label: "Sound File", kind: SettingKind::Text("") }],
            unique: false,
        },
        NodeDef {
            kind: "loadScene",
            label: "Load Scene",
            category: "Actions",
            description: "After this step, loads a scene file: in place of the playing scene (every script starts again; the game's values stay), or, with Add to This Scene, its objects join this one and Unload Scene removes them again.",
            inputs: &[RUN, PinDef::with("additive", "Add to This Scene", Bool, PinDefault::Bool(false))],
            outputs: &[THEN],
            settings: &[SCENE_FILE],
            unique: false,
        },
        NodeDef {
            kind: "unloadScene",
            label: "Unload Scene",
            category: "Actions",
            description: "After this step, removes the objects of the scenes added from this file while playing.",
            inputs: &[RUN],
            outputs: &[THEN],
            settings: &[SCENE_FILE],
            unique: false,
        },
        action("destroySelf", "Destroy This Object", "Deletes this object and its children after this step.", &[RUN]),
        NodeDef {
            kind: "gameValue",
            label: "Game Value",
            category: "Game",
            description: "A number the game keeps under a name: the game's own (Scope Game), a player's (Scope Player: the object named in Player, else the first player) or this object's (Scope This Object, when it has Player Control); Scope Saved keeps it from one play to the next, as for a best score. Nothing is set at first: Value is 0 and Is Set is false until something sets it.",
            inputs: &[],
            outputs: &[pin("value", "Value", Float), pin("set", "Is Set", Bool)],
            settings: &[STATE_NAME, STATE_SCOPE, STATE_PLAYER],
            unique: false,
        },
        NodeDef {
            kind: "setGameValue",
            label: "Set Game Value",
            category: "Game",
            description: "Keeps a number under a name in the game's state or a player's (see Game Value for the scopes).",
            inputs: &[RUN, number("value", "Value", Float, 0.0)],
            outputs: &[THEN],
            settings: &[STATE_NAME, STATE_SCOPE, STATE_PLAYER],
            unique: false,
        },
        NodeDef {
            kind: "changeGameValue",
            label: "Change Game Value",
            category: "Game",
            description: "Adds an amount to a number in the game's state or a player's; a name not set yet counts as 0.",
            inputs: &[RUN, number("amount", "Amount", Float, 1.0)],
            outputs: &[THEN],
            settings: &[STATE_NAME, STATE_SCOPE, STATE_PLAYER],
            unique: false,
        },
        NodeDef {
            kind: "setInputMap",
            label: "Set Input Map",
            category: "Inputs",
            description: "Makes another map of an Input Map active (Player, Menu…): the project's, or this object's own when it has an Input Map component. Only the active map's actions work.",
            inputs: &[RUN],
            outputs: &[THEN],
            settings: &[
                SettingDef { name: "map", label: "Map", kind: SettingKind::Name("Menu") },
                SettingDef { name: "scope", label: "Scope", kind: SettingKind::Choice(&["Project", "This Object"]) },
            ],
            unique: false,
        },
        NodeDef {
            kind: "endGame",
            label: "End Game",
            category: "Game",
            description: "Ends the game, won or lost; every script then gets the message gameWon or gameLost. Only the first ending counts.",
            inputs: &[RUN, PinDef::with("won", "Won", Bool, PinDefault::Bool(true))],
            outputs: &[THEN],
            settings: &[],
            unique: false,
        },
        NodeDef {
            kind: "playParticles",
            label: "Play Particles",
            category: "Effects",
            description: "Starts the Particle Emitter of the linked object, else the first object named in Object Name, else this object's: a new cycle, with its burst. After this step.",
            inputs: &[RUN, TARGET],
            outputs: &[THEN],
            settings: &[OBJECT_NAME],
            unique: false,
        },
        NodeDef {
            kind: "stopParticles",
            label: "Stop Particles",
            category: "Effects",
            description: "Stops the emitter starting particles; those alive live out their lives. Which emitter: as Play Particles.",
            inputs: &[RUN, TARGET],
            outputs: &[THEN],
            settings: &[OBJECT_NAME],
            unique: false,
        },
        NodeDef {
            kind: "burstParticles",
            label: "Burst Particles",
            category: "Effects",
            description: "Starts Count particles at once from the emitter (as many as it has room for). Which emitter: as Play Particles.",
            inputs: &[RUN, TARGET, PinDef::with("count", "Count", PinType::Int, PinDefault::Int(20))],
            outputs: &[THEN],
            settings: &[OBJECT_NAME],
            unique: false,
        },
        NodeDef {
            kind: "interfacePressed",
            label: "On Interface Pressed",
            category: "Interface",
            description: "Runs when the player presses the button named in Element, in this object's Interface.",
            inputs: &[],
            outputs: &[THEN],
            settings: &[ELEMENT],
            unique: false,
        },
        NodeDef {
            kind: "interfaceChanged",
            label: "On Interface Changed",
            category: "Interface",
            description: "Runs when the player changes the toggle, slider or text field named in Element, in this object's Interface. Value is its new value (a text field's length).",
            inputs: &[],
            outputs: &[THEN, pin("value", "Value", Float)],
            settings: &[ELEMENT],
            unique: false,
        },
        NodeDef {
            kind: "setInterfaceText",
            label: "Set Interface Text",
            category: "Interface",
            description: "Sets the text of the element named in Element, in the Interface of the linked object, else the first object named in Object Name, else this object. It shows after this step.",
            inputs: &[RUN, TARGET, text("text", "Text", "")],
            outputs: &[THEN],
            settings: &[ELEMENT, OBJECT_NAME],
            unique: false,
        },
        NodeDef {
            kind: "setInterfaceValue",
            label: "Set Interface Value",
            category: "Interface",
            description: "Sets an element's value: a toggle's state (0 or 1), a slider's number, a progress bar's fill (0 to 1). Which interface: as Set Interface Text.",
            inputs: &[RUN, TARGET, number("value", "Value", Float, 0.0)],
            outputs: &[THEN],
            settings: &[ELEMENT, OBJECT_NAME],
            unique: false,
        },
        NodeDef {
            kind: "showInterface",
            label: "Show Interface",
            category: "Interface",
            description: "Shows or hides the element named in Element, or the whole interface when Element is empty. Which interface: as Set Interface Text.",
            inputs: &[RUN, TARGET, PinDef::with("visible", "Visible", Bool, PinDefault::Bool(true))],
            outputs: &[THEN],
            settings: &[SettingDef { name: "element", label: "Element", kind: SettingKind::Text("") }, OBJECT_NAME],
            unique: false,
        },
        NodeDef {
            kind: "addInterfaceElement",
            label: "Add Interface Element",
            category: "Interface",
            description: "Builds the interface while the game plays: adds an element named Name into the element named in Into (the root when empty). What is a kind (button, text, progress...) or a layout file (.bui) used as a part, whose elements are then named Name/element.",
            inputs: &[RUN, TARGET, text("name", "Name", "Item")],
            outputs: &[THEN],
            settings: &[
                SettingDef { name: "into", label: "Into", kind: SettingKind::Text("") },
                SettingDef { name: "what", label: "What", kind: SettingKind::Text("button") },
                OBJECT_NAME,
            ],
            unique: false,
        },
        NodeDef {
            kind: "interfaceValue",
            label: "Interface Value",
            category: "Interface",
            description: "The value of the element named in Element as it shows now: a toggle's 0 or 1, a slider's number, a bound game value. Found is false when the interface has no such element. Which interface: the linked object's, else the first object named in Object Name, else this object's.",
            inputs: &[TARGET],
            outputs: &[pin("value", "Value", Float), pin("found", "Found", Bool)],
            settings: &[ELEMENT, OBJECT_NAME],
            unique: false,
        },
        NodeDef {
            kind: "interfaceText",
            label: "Interface Text",
            category: "Interface",
            description: "The text of the element named in Element as it shows now, what the player typed into a text field too; empty when there is no such element. Which interface: as Interface Value.",
            inputs: &[TARGET],
            outputs: &[pin("text", "Text", PinType::Text)],
            settings: &[ELEMENT, OBJECT_NAME],
            unique: false,
        },
        NodeDef {
            kind: "removeInterfaceElement",
            label: "Remove Interface Element",
            category: "Interface",
            description: "Removes the element named Name, with what it holds. Which interface: as Set Interface Text.",
            inputs: &[RUN, TARGET, text("name", "Name", "Item")],
            outputs: &[THEN],
            settings: &[OBJECT_NAME],
            unique: false,
        },
        NodeDef {
            kind: "walk",
            label: "Walk",
            category: "Physics",
            description: "Sets a character's walking velocity (the linked object, else the first one named in Object Name, else this object), in world units per second, until changed.",
            inputs: &[RUN, TARGET, vector("velocity", "Velocity", [0.0; 3])],
            outputs: &[THEN],
            settings: &[OBJECT_NAME],
            unique: false,
        },
        NodeDef {
            kind: "jump",
            label: "Jump",
            category: "Physics",
            description: "Makes a character on the ground jump with an upward speed (the linked object, else the first one named in Object Name, else this object).",
            inputs: &[RUN, TARGET, number("speed", "Speed", Float, 5.0)],
            outputs: &[THEN],
            settings: &[OBJECT_NAME],
            unique: false,
        },
        node("onGround", "On Ground", "Physics", "True while this object's character stands on something.", &[], &[pin("grounded", "On Ground", Bool)]),
        NodeDef {
            kind: "addImpulse",
            label: "Add Impulse",
            category: "Physics",
            description: "Pushes a physics body (the linked object, else the first one named in Object Name, else this object) by an impulse in world space, after this step.",
            inputs: &[RUN, TARGET, vector("impulse", "Impulse", [0.0, 5.0, 0.0])],
            outputs: &[THEN],
            settings: &[OBJECT_NAME],
            unique: false,
        },
        NodeDef {
            kind: "setVelocity",
            label: "Set Velocity",
            category: "Physics",
            description: "Sets a physics body's velocity (the linked object, else the first one named in Object Name, else this object), in world units per second, after this step.",
            inputs: &[RUN, TARGET, vector("velocity", "Velocity", [0.0; 3])],
            outputs: &[THEN],
            settings: &[OBJECT_NAME],
            unique: false,
        },
        node(
            "velocity",
            "Velocity",
            "Physics",
            "This object's physics velocity as the step began (0 without a body).",
            &[],
            &[pin("velocity", "Velocity", Vec3)],
        ),
        NodeDef {
            kind: "raycast",
            label: "Cast Ray",
            category: "Object",
            description: "Sends a ray from Origin along Direction, up to Distance, and says whether it hits another object's shape or sprite, how far, and where.",
            inputs: &[vector("origin", "Origin", [0.0; 3]), vector("direction", "Direction", [0.0, -1.0, 0.0]), number("distance", "Distance", Float, 100.0)],
            outputs: &[pin("hit", "Hit", Bool), pin("distance", "Hit Distance", Float), pin("point", "Hit Point", Vec3)],
            settings: &[],
            unique: false,
        },
        NodeDef {
            kind: "padStick",
            label: "Gamepad Stick",
            category: "Inputs",
            description: "Where a gamepad stick (Left or Right) points, from -1 to 1 (x right, y up); 0 without a gamepad.",
            inputs: &[],
            outputs: &[pin("direction", "Direction", Vec3), pin("connected", "Gamepad Connected", Bool)],
            settings: &[SettingDef { name: "stick", label: "Stick", kind: SettingKind::Name("Left") }],
            unique: false,
        },
        NodeDef {
            kind: "padTrigger",
            label: "Gamepad Trigger",
            category: "Inputs",
            description: "How far a gamepad trigger (Left or Right) is pressed, from 0 to 1.",
            inputs: &[],
            outputs: &[pin("amount", "Amount", Float)],
            settings: &[SettingDef { name: "trigger", label: "Trigger", kind: SettingKind::Name("Right") }],
            unique: false,
        },
        NodeDef {
            kind: "padButton",
            label: "Gamepad Button Held",
            category: "Inputs",
            description: "True while the gamepad button is held: South, East, West, North, LeftBumper, RightBumper, Select, Start, Up, Down, Left, Right and others.",
            inputs: &[],
            outputs: &[pin("down", "Held", Bool)],
            settings: &[SettingDef { name: "button", label: "Button", kind: SettingKind::Name("South") }],
            unique: false,
        },
        NodeDef {
            kind: "keyDown",
            label: "Key Held",
            category: "Inputs",
            description: "True while the key is held (names like W, Space, ArrowUp).",
            inputs: &[],
            outputs: &[pin("down", "Held", Bool)],
            settings: &[SettingDef { name: "key", label: "Key", kind: SettingKind::Name("Space") }],
            unique: false,
        },
        node("add", "Add", "Math", "A + B.", &[number("a", "A", Any, 0.0), number("b", "B", Any, 0.0)], MATH_OUT),
        node("subtract", "Subtract", "Math", "A − B.", &[number("a", "A", Any, 0.0), number("b", "B", Any, 0.0)], MATH_OUT),
        node("multiply", "Multiply", "Math", "A × B.", &[number("a", "A", Any, 1.0), number("b", "B", Any, 1.0)], MATH_OUT),
        node(
            "negate",
            "Negate",
            "Math",
            "The value with its sign turned: 5 becomes −5 and −5 becomes 5; a vector points the other way.",
            &[number("value", "Value", Any, 0.0)],
            MATH_OUT,
        ),
        node("divide", "Divide", "Math", "A ÷ B.", &[number("a", "A", Any, 1.0), number("b", "B", Any, 1.0)], MATH_OUT),
        node("sine", "Sine", "Math", "The sine of the input, in radians.", &[number("value", "Value", Any, 0.0)], MATH_OUT),
        node(
            "power",
            "Power",
            "Math",
            "Base to the power of the exponent: whole numbers stay whole (a negative exponent gives 0, or ±1 for 1 and −1); for a vector, each part.",
            &[number("base", "Base", Any, 1.0), number("exponent", "Exponent", Float, 2.0)],
            MATH_OUT,
        ),
        node(
            "squareRoot",
            "Square Root",
            "Math",
            "The number that times itself gives the input (0 below 0); for a vector, of each part.",
            &[number("value", "Value", Any, 1.0)],
            MATH_OUT,
        ),
        node(
            "vectorLength",
            "Vector Length",
            "Math",
            "How long a vector is: the distance from 0 to the point it names.",
            &[vector("vector", "Vector", [0.0, 0.0, 0.0])],
            &[pin("length", "Length", Float)],
        ),
        node(
            "normalize",
            "Normalize",
            "Math",
            "The vector's direction at length 1: a move as fast diagonally as straight. A vector of length 0 stays 0, so a player standing still stays still.",
            &[vector("vector", "Vector", [0.0, 0.0, 0.0])],
            &[pin("vector", "Vector", Vec3)],
        ),
        node(
            "lerp",
            "Lerp",
            "Math",
            "Linear interpolation: from A (factor 0) to B (factor 1) on a straight line; factors outside 0 to 1 go past A or B. For vectors, each part.",
            &[number("a", "A", Any, 0.0), number("b", "B", Any, 1.0), number("factor", "Factor", Float, 0.5)],
            MATH_OUT,
        ),
        node(
            "inverseLerp",
            "Inverse Lerp",
            "Math",
            "Where the value lies from A to B: 0 at A, 1 at B, 0.5 halfway, and past 0 or 1 beyond them; 0 when A and B are equal.",
            &[number("a", "A", Float, 0.0), number("b", "B", Float, 1.0), number("value", "Value", Float, 0.5)],
            &[pin("result", "Result", Float)],
        ),
        node(
            "choose",
            "Choose",
            "Math",
            "A when the condition is true, otherwise B.",
            &[PinDef::with("condition", "Condition", Bool, PinDefault::Bool(false)), number("a", "A", Any, 1.0), number("b", "B", Any, 0.0)],
            MATH_OUT,
        ),
        node(
            "greater",
            "Greater Than",
            "Logic",
            "True when A > B.",
            &[number("a", "A", Float, 0.0), number("b", "B", Float, 0.0)],
            &[pin("result", "Result", Bool)],
        ),
        node("and", "And", "Logic", "True when both are true.", &[pin("a", "A", Bool), pin("b", "B", Bool)], &[pin("result", "Result", Bool)]),
        node("or", "Or", "Logic", "True when either is true.", &[pin("a", "A", Bool), pin("b", "B", Bool)], &[pin("result", "Result", Bool)]),
        node("not", "Not", "Logic", "True when the input is false.", &[pin("value", "Value", Bool)], &[pin("result", "Result", Bool)]),
        node(
            "wholeNumber",
            "Whole Number",
            "Values",
            "A whole number typed here.",
            &[PinDef::with("value", "Value", PinType::Int, PinDefault::Int(0))],
            &[pin("result", "Result", PinType::Int)],
        ),
        NodeDef {
            kind: "toWhole",
            label: "To Whole Number",
            category: "Math",
            description: "A number made whole: Nearest (halves away from zero), Down, Up or Toward Zero. Numbers past the 32-bit range stop at its ends.",
            inputs: &[number("value", "Value", Float, 0.0)],
            outputs: &[pin("result", "Result", PinType::Int)],
            settings: &[SettingDef { name: "rounding", label: "Rounding", kind: SettingKind::Choice(&["Nearest", "Down", "Up", "Toward Zero"]) }],
            unique: false,
        },
        node(
            "remainder",
            "Remainder",
            "Math",
            "What is left after dividing A by B as whole numbers (its sign is A's); 0 when B is 0.",
            &[PinDef::with("a", "A", PinType::Int, PinDefault::Int(0)), PinDef::with("b", "B", PinType::Int, PinDefault::Int(1))],
            &[pin("result", "Result", PinType::Int)],
        ),
        node("text", "Text", "Text", "Text typed here.", &[text("value", "Value", "")], TEXT_OUT),
        node("joinText", "Join Text", "Text", "A followed by B.", &[text("a", "A", ""), text("b", "B", "")], TEXT_OUT),
        node("textLength", "Text Length", "Text", "How many characters the text has.", &[text("text", "Text", "")], &[pin("result", "Length", PinType::Int)]),
        node(
            "numberToText",
            "To Text",
            "Text",
            "A number (up to three decimals), a whole number (every digit) or a vector, as (x, y, z), written as text.",
            &[number("value", "Value", Any, 0.0)],
            TEXT_OUT,
        ),
        node(
            "textToNumber",
            "Text to Number",
            "Text",
            "The number the text spells, such as -2.5 (spaces around it are fine). Valid is false, and the number 0, when it spells none.",
            &[text("text", "Text", "")],
            &[pin("result", "Number", Float), pin("valid", "Valid", Bool)],
        ),
        node(
            "sameText",
            "Same Text",
            "Text",
            "True when A and B are the same text (upper and lower case differ).",
            &[text("a", "A", ""), text("b", "B", "")],
            &[pin("result", "Result", Bool)],
        ),
        node(
            "textContains",
            "Text Contains",
            "Text",
            "True when Part appears in the text.",
            &[text("text", "Text", ""), text("part", "Part", "")],
            &[pin("result", "Result", Bool)],
        ),
        node(
            "startsWith",
            "Starts With",
            "Text",
            "True when the text begins with Part.",
            &[text("text", "Text", ""), text("part", "Part", "")],
            &[pin("result", "Result", Bool)],
        ),
        node(
            "endsWith",
            "Ends With",
            "Text",
            "True when the text ends with Part.",
            &[text("text", "Text", ""), text("part", "Part", "")],
            &[pin("result", "Result", Bool)],
        ),
        node(
            "findText",
            "Find in Text",
            "Text",
            "Where Part first appears in the text, counted in characters from 0; -1 when it does not.",
            &[text("text", "Text", ""), text("part", "Part", "")],
            &[pin("result", "Index", PinType::Int)],
        ),
        node(
            "textPart",
            "Part of Text",
            "Text",
            "Count characters of the text from Start (0 is the first), cut short at its end.",
            &[text("text", "Text", ""), number("start", "Start", Float, 0.0), number("count", "Count", Float, 1.0)],
            TEXT_OUT,
        ),
        node(
            "replaceText",
            "Replace Text",
            "Text",
            "The text with every Find replaced by With.",
            &[text("text", "Text", ""), text("find", "Find", ""), text("with", "With", "")],
            TEXT_OUT,
        ),
        node("upperCase", "Upper Case", "Text", "The text with a to z made A to Z.", &[text("text", "Text", "")], TEXT_OUT),
        node("lowerCase", "Lower Case", "Text", "The text with A to Z made a to z.", &[text("text", "Text", "")], TEXT_OUT),
        node("trimText", "Trim Text", "Text", "The text without spaces, tabs and line breaks at its start and end.", &[text("text", "Text", "")], TEXT_OUT),
        node("nameOf", "Name Of", "Text", "The name of the linked object, else this object.", &[TARGET], TEXT_OUT),
    ],
};

/// Most WebAssembly text one graph may produce (shared chains are copied, so this can grow).
const MAX_TEXT: usize = 8 << 20;
/// Where static text starts in the module's memory. Bytes 0..36 hold the transform.
const DATA_START: u32 = 1024;

/// A compiled value: WebAssembly expressions that leave it on the stack.
#[derive(Clone)]
enum Val {
    F(String),
    V([String; 3]),
    B(String),
    /// An object's handle (`i32`; 0 for none).
    O(String),
    /// Text: `i64` of its address << 32 | its length in bytes, in this module's memory.
    T(String),
    /// A whole number (`i32`).
    I(String),
    /// A script on an object: `i64` of the object's handle << 32 | the script's place plus one (0 for none).
    C(String),
    /// A list this script keeps: `i64` of where its name is << 32 | the name's length.
    L(String),
    /// A map this script keeps, as a list is.
    M(String),
}

/// What a Get, Try Get or Set Component node works on.
struct Target {
    /// As the host knows it: `pointLight`, or a script's name.
    name: String,
    label: String,
    fields: &'static [bayleaf_graph::PinDef],
    settable: bool,
}

struct Compiler<'g> {
    graph: &'g Graph,
    data: Vec<(u32, Vec<u8>)>,
    next: u32,
    /// The function whose actions are being compiled, if any: its Function node's values and
    /// Return nodes belong there only.
    scope: Option<String>,
    /// Declared properties: name, type and default, in the order first met.
    properties: Vec<(String, String, serde_json::Value)>,
    /// Globals actions need (the object and index of a For Each), declared once each.
    globals: Vec<String>,
    /// Each text variable's buffer and starting length, by name.
    text_vars: std::collections::BTreeMap<String, (u32, u32)>,
}

/// Compiles a visual script into a WebAssembly module.
/// The node kinds that read and set a variable of `ty`, by their `name` setting: the Get node first.
pub fn variable_kinds(ty: VariableType) -> [&'static str; 2] {
    match ty {
        VariableType::Number => ["variable", "setVariable"],
        VariableType::Vector => ["vectorVariable", "setVectorVariable"],
        VariableType::TrueFalse => ["flag", "setFlag"],
        VariableType::WholeNumber => ["wholeVariable", "setWholeVariable"],
        VariableType::Text => ["textVariable", "setTextVariable"],
        VariableType::Object => ["objectVariable", "setObjectVariable"],
        VariableType::Color => ["colorVariable", "setColorVariable"],
    }
}

/// The Property and Set Property node kinds for each property type.
pub fn property_kinds(ty: VariableType) -> [&'static str; 2] {
    match ty {
        VariableType::Number => ["property", "setProperty"],
        VariableType::WholeNumber => ["wholeProperty", "setWholeProperty"],
        VariableType::TrueFalse => ["flagProperty", "setFlagProperty"],
        VariableType::Vector => ["vectorProperty", "setVectorProperty"],
        VariableType::Color => ["colorProperty", "setColorProperty"],
        VariableType::Text => ["textProperty", "setTextProperty"],
        VariableType::Object => ["objectProperty", "setObjectProperty"],
    }
}

/// The property type a Property or Set Property node kind reads or sets.
pub fn property_type(kind: &str) -> Option<VariableType> {
    VariableType::ALL.into_iter().find(|ty| property_kinds(*ty).contains(&kind))
}

/// How the scene and the Inspector call a property of this type.
pub fn scene_property_type(ty: VariableType) -> PropertyType {
    match ty {
        VariableType::Number => PropertyType::Number,
        VariableType::WholeNumber => PropertyType::WholeNumber,
        VariableType::TrueFalse => PropertyType::TrueFalse,
        VariableType::Vector => PropertyType::Vector,
        VariableType::Color => PropertyType::Color,
        VariableType::Text => PropertyType::Text,
        VariableType::Object => PropertyType::Object,
    }
}

/// A declared default as `bayleaf_describe` lists it: an object property's none is `""`, a colour
/// stays within 0 to 1.
fn described_default(ty: VariableType, value: &serde_json::Value) -> serde_json::Value {
    match ty {
        VariableType::Object => serde_json::json!(""),
        VariableType::Color => {
            let parts: Vec<f32> = (0..3).map(|i| value.get(i).and_then(bayleaf_graph::as_f32).unwrap_or(1.0).clamp(0.0, 1.0)).collect();
            serde_json::json!(parts)
        }
        _ => value.clone(),
    }
}

/// A declared default or start in `shape` as `bayleaf_describe` lists it: one value as
/// [`described_default`] makes it, or a list (array) or map (object) of such values.
fn described_value(ty: VariableType, shape: Shape, value: &serde_json::Value) -> serde_json::Value {
    match shape {
        Shape::Single => described_default(ty, value),
        Shape::List => value.as_array().map_or_else(|| serde_json::json!([]), |items| items.iter().map(|v| described_default(ty, v)).collect()),
        Shape::Map => serde_json::Value::Object(
            value.as_object().map(|entries| entries.iter().map(|(k, v)| (k.clone(), described_default(ty, v))).collect()).unwrap_or_default(),
        ),
    }
}

/// The type code a description gives a value of `ty` in `shape`: `number`, `list:number`, or a
/// map's keys' type and then its values' (`map:whole:number`).
fn described_type(ty: VariableType, shape: Shape, key: VariableType) -> String {
    let code = scene_property_type(ty).code();
    match shape {
        Shape::Single => code.to_owned(),
        Shape::List => format!("list:{code}"),
        Shape::Map => format!("map:{}:{code}", scene_property_type(key).code()),
    }
}

/// How the scene and the Inspector call a shape.
pub fn scene_property_shape(shape: Shape, key: VariableType) -> PropertyShape {
    match shape {
        Shape::Single => PropertyShape::Single,
        Shape::List => PropertyShape::List,
        Shape::Map => PropertyShape::Map(scene_property_type(key)),
    }
}

/// What a Property node's name was before files said otherwise, by kind (graphs before version 5).
const OLD_PROPERTY_NAMES: [(&str, &str); 7] = [
    ("property", "speed"),
    ("wholeProperty", "count"),
    ("flagProperty", "enabled"),
    ("vectorProperty", "offset"),
    ("colorProperty", "tint"),
    ("textProperty", "label"),
    ("objectProperty", "target"),
];

/// Moves what visual scripts before version 5 kept on their Property nodes (each one's name and
/// default) into the script's declared properties; the first node of a name gives its default.
fn upgrade(graph: &mut Graph, read: u32) {
    if read >= 5 {
        return;
    }
    let mut declared = Vec::new();
    for node in &mut graph.nodes {
        let Some(ty) = property_type(&node.kind) else { continue };
        let old_name = OLD_PROPERTY_NAMES.iter().find(|(k, _)| *k == node.kind).map_or("speed", |(_, n)| *n);
        let name = node.values.get("name").and_then(serde_json::Value::as_str).unwrap_or(old_name).to_owned();
        node.values.insert("name".to_owned(), serde_json::json!(name));
        let old = node.values.remove("default");
        let default = match (ty, old) {
            (VariableType::Number, Some(v)) => serde_json::json!(bayleaf_graph::as_f32(&v).unwrap_or(1.0)),
            (VariableType::Number, None) => serde_json::json!(1.0),
            (VariableType::WholeNumber, v) => serde_json::json!(v.as_ref().and_then(bayleaf_graph::as_f32).unwrap_or(0.0).round() as i32),
            (VariableType::TrueFalse, v) => serde_json::json!(v.as_ref().and_then(serde_json::Value::as_str) == Some("True")),
            (VariableType::Text, v) => serde_json::json!(v.as_ref().and_then(serde_json::Value::as_str).unwrap_or_default()),
            (VariableType::Object, _) => serde_json::Value::Null,
            (VariableType::Vector | VariableType::Color, Some(v)) if ty.fits(&v) => v,
            (_, _) => ty.zero(),
        };
        if !declared.iter().any(|(n, _, _): &(String, VariableType, serde_json::Value)| *n == name) {
            declared.push((name, ty, default));
        }
    }
    for (name, ty, default) in declared {
        if graph.property(&name).is_none() && ty.fits(&default) {
            let _ = graph.add_property(&name, ty, Some(default));
        }
    }
}

pub fn compile(graph: &Graph) -> Result<Vec<u8>, Vec<Problem>> {
    let text = to_wat(graph)?;
    wat::parse_str(&text).map_err(|e| vec![Problem::graph(format!("internal compiler error: {e}"))])
}

/// The WebAssembly text of a visual script.
pub fn to_wat(graph: &Graph) -> Result<String, Vec<Problem>> {
    graph.check(&CATALOG).map_err(|e| vec![Problem::graph(e.to_string())])?;
    let mut c = Compiler {
        graph,
        data: Vec::new(),
        next: DATA_START,
        scope: None,
        properties: Vec::new(),
        globals: Vec::new(),
        text_vars: std::collections::BTreeMap::new(),
    };
    declare_storage(&mut c);
    let mut problems = Vec::new();
    let mut init = String::new();
    let mut start = String::new();
    let mut update = String::new();
    let mut message = String::new();
    // Globals the events keep between steps.
    let mut state: Vec<String> = Vec::new();
    let mut events: Vec<&bayleaf_graph::Node> = graph.nodes.iter().filter(|n| is_event(&n.kind)).collect();
    events.sort_by_key(|n| n.id);
    for event in events {
        match event_chain(&mut c, graph, event) {
            Ok(code) if event.kind == "init" => init.push_str(&code),
            Ok(code) if event.kind == "start" => start.push_str(&code),
            Ok(code) if event.kind == "update" => update.push_str(&code),
            Ok(code) if event.kind == "keyPressed" || event.kind == "actionPressed" => {
                // Runs when the key or action is down now and was not in the last step.
                let held = if event.kind == "keyPressed" {
                    let key = c.setting_text(event.id, "key", "Space");
                    let (at, len) = c.text(key.as_bytes());
                    format!("(call $key_down (i32.const {at}) (i32.const {len}))")
                } else {
                    let action = c.setting_text(event.id, "action", "Jump");
                    let (at, len) = c.text(action.as_bytes());
                    format!("(f32.gt (call $action (i32.const {at}) (i32.const {len})) (f32.const 0.5))")
                };
                let was = format!("$was_{}", event.id);
                state.push(format!("(global {was} (mut i32) (i32.const 0))"));
                let _ = writeln!(update, "    (if (i32.and {held} (i32.eqz (global.get {was})))\n      (then\n{code}      ))\n    (global.set {was} {held})");
            }
            Ok(code) if event.kind == "timer" => {
                let seconds = c.setting_number(event.id, "seconds", 1.0);
                if !(seconds.is_finite() && seconds >= 0.01) {
                    problems.push(Problem::on(event.id, "the timer needs at least 0.01 seconds"));
                    continue;
                }
                let clock = format!("$clock_{}", event.id);
                state.push(format!("(global {clock} (mut f32) (f32.const 0))"));
                let s = lit(seconds);
                let _ = writeln!(
                    update,
                    "    (global.set {clock} (f32.add (global.get {clock}) (global.get $dt)))\n    (if (f32.ge (global.get {clock}) {s})\n      (then\n        (global.set {clock} (f32.sub (global.get {clock}) {s}))\n{code}      ))"
                );
            }
            Ok(code) => {
                // Each On Message runs only for its own message name; an interface sends its own.
                let name = match event.kind.as_str() {
                    "interfacePressed" => format!("pressed:{}", c.setting_text(event.id, "element", "Start")),
                    "interfaceChanged" => format!("changed:{}", c.setting_text(event.id, "element", "Start")),
                    "eventStart" => c.setting_text(event.id, "event", ""),
                    // A subscribed component's event arrives as `<its script>/<event>`.
                    "componentEventStart" => format!("{}/{}", c.setting_text(event.id, "script", ""), c.setting_text(event.id, "event", "")),
                    kind => contact_message(kind).map_or_else(|| c.setting_text(event.id, "message", "hit"), str::to_owned),
                };
                let (at, len) = c.text(name.as_bytes());
                let _ = writeln!(message, "    (if (call $message_is (i32.const {at}) (i32.const {len}))\n      (then\n{code}      ))");
            }
            Err(p) => problems.push(p),
        }
        if init.len() + start.len() + update.len() + message.len() > MAX_TEXT {
            return Err(vec![Problem::graph("the script is too large; reuse fewer shared chains")]);
        }
    }
    let functions = compile_functions(&mut c, &mut problems);
    // A variable node names one value, never a list or map.
    for node in graph.nodes.iter().filter(|n| VariableType::ALL.iter().any(|ty| variable_kinds(*ty).contains(&n.kind.as_str()))) {
        let name = c.setting_text(node.id, "name", "");
        if let Some(v) = graph.variable(&name).filter(|v| !v.shape.is_single()) {
            problems.push(Problem::on(node.id, format!("{name} is a {}: use the list and map nodes", v.shape.label().to_lowercase())));
        }
    }
    // Nodes naming a property that is not declared still declare it, at the type's zero.
    for node in graph.nodes.iter().filter(|n| property_type(&n.kind).is_some()) {
        if let Err(p) = c.property_of(node.id) {
            problems.push(p);
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    let description = description(graph, &c.properties).map_err(|p| vec![p])?;
    let (describe_at, describe_len) = c.text(description.as_bytes());
    let pages = c.next.div_ceil(65536).max(1);
    // Text made while a call runs goes after the module's own data; each call starts it empty.
    let heap = c.next.next_multiple_of(16);
    let fresh = format!("    (global.set $heap (i32.const {heap}))\n");

    let mut out = String::new();
    out.push_str(PRELUDE);
    out.push_str(collections::HELPERS);
    // One global per variable name; names are letters, digits and _, so they are valid identifiers.
    // Declared variables start at their values; names nodes use without a declaration start at zero.
    let named = |ty: VariableType, default: &str| -> std::collections::BTreeMap<String, serde_json::Value> {
        let kinds = variable_kinds(ty);
        let mut names: std::collections::BTreeMap<String, serde_json::Value> =
            graph.nodes.iter().filter(|n| kinds.contains(&n.kind.as_str())).map(|n| (c.setting_text(n.id, "name", default), ty.zero())).collect();
        names.extend(graph.variables.iter().filter(|v| v.ty == ty && v.shape.is_single()).map(|v| (v.name.clone(), v.start.clone())));
        names
    };
    let number = |v: &serde_json::Value| v.as_f64().unwrap_or(0.0) as f32;
    for (name, start) in named(VariableType::Number, "count") {
        let _ = writeln!(out, "  (global $var_{name} (mut f32) (f32.const {:?}))", number(&start));
    }
    for (name, start) in named(VariableType::Vector, "target") {
        for (index, axis) in ["x", "y", "z"].into_iter().enumerate() {
            let _ = writeln!(out, "  (global $vec_{name}_{axis} (mut f32) (f32.const {:?}))", number(&start[index]));
        }
    }
    for (name, start) in named(VariableType::TrueFalse, "done") {
        let _ = writeln!(out, "  (global $flag_{name} (mut i32) (i32.const {}))", i32::from(start.as_bool().unwrap_or(false)));
    }
    for (name, start) in named(VariableType::Color, "tint") {
        let start = if start.is_array() { start } else { VariableType::Color.zero() };
        for (index, part) in ["r", "g", "b"].into_iter().enumerate() {
            let _ = writeln!(out, "  (global $col_{name}_{part} (mut f32) (f32.const {:?}))", number(&start[index]));
        }
    }
    for (name, start) in named(VariableType::WholeNumber, "total") {
        let _ = writeln!(out, "  (global $int_{name} (mut i32) (i32.const {}))", start.as_i64().and_then(|n| i32::try_from(n).ok()).unwrap_or(0));
    }
    for (name, _) in named(VariableType::Object, "thing") {
        let _ = writeln!(out, "  (global $obj_{name} (mut i32) (i32.const 0))");
    }
    for (name, (_, len)) in &c.text_vars {
        let _ = writeln!(out, "  (global $txt_{name} (mut i32) (i32.const {len}))");
    }
    for global in state.iter().chain(&c.globals) {
        let _ = writeln!(out, "  {global}");
    }
    let _ = writeln!(out, "  (memory (export \"memory\") {pages})");
    for (at, bytes) in &c.data {
        let mut escaped = String::with_capacity(bytes.len() * 3);
        for b in bytes {
            let _ = write!(escaped, "\\{b:02x}");
        }
        let _ = writeln!(out, "  (data (i32.const {at}) \"{escaped}\")");
    }
    let _ = writeln!(
        out,
        "  (func (export \"bayleaf_describe\") (result i64) (i64.or (i64.shl (i64.const {describe_at}) (i64.const 32)) (i64.const {describe_len})))"
    );
    if graph.nodes.iter().any(|n| n.kind == "init") {
        let _ = writeln!(out, "  (func (export \"bayleaf_init\")\n{fresh}{init}  )");
    }
    let _ = writeln!(out, "  (func (export \"bayleaf_start\")\n{fresh}{start}  )");
    let _ = writeln!(out, "  (func (export \"bayleaf_update\") (param $dt f32)\n{fresh}    (global.set $dt (local.get $dt))\n{update}  )");
    if graph.nodes.iter().any(|n| takes_messages(&n.kind)) {
        let _ = writeln!(out, "  (func (export \"bayleaf_message\") (param $value f32)\n{fresh}    (global.set $message (local.get $value))\n{message}  )");
    }
    out.push_str(&functions);
    out.push_str(")\n");
    Ok(out)
}

/// A name as a label: `jumpHeight` and `jump_height` become `Jump Height`.
fn title(name: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for c in name.chars() {
        if c == '_' || c == ' ' {
            upper = true;
            continue;
        }
        if (upper || c.is_uppercase()) && !out.is_empty() {
            out.push(' ');
        }
        if upper {
            out.extend(c.to_uppercase())
        } else {
            out.push(c)
        }
        upper = false;
    }
    out
}

/// A WebAssembly f32 literal.
fn lit(value: f32) -> String {
    format!("(f32.const {value:?})")
}

impl Compiler<'_> {
    /// Places bytes in the module's memory and returns their address and length.
    fn text(&mut self, bytes: &[u8]) -> (u32, u32) {
        self.reserve(bytes, 0)
    }

    /// Text placed in memory, as a text value.
    fn literal(&mut self, text: &str) -> String {
        let (at, len) = self.text(text.as_bytes());
        format!("(i64.const {})", (i64::from(at) << 32) | i64::from(len))
    }

    /// A number, whole number or vector as text; a vector as "(x, y, z)".
    fn value_text(&mut self, node: u32, value: Val) -> Result<String, Problem> {
        let Val::V([x, y, z]) = value else { return number_text(node, value) };
        let (open, comma, close) = (self.literal("("), self.literal(", "), self.literal(")"));
        let mut text = open;
        for (i, part) in [x, y, z].into_iter().enumerate() {
            if i > 0 {
                text = format!("(call $join {text} {comma})");
            }
            text = format!("(call $join {text} (call $num_text {part}))");
        }
        Ok(format!("(call $join {text} {close})"))
    }

    /// Places bytes followed by `room` free bytes.
    fn reserve(&mut self, bytes: &[u8], room: u32) -> (u32, u32) {
        let at = self.next;
        let len = bytes.len() as u32;
        self.data.push((at, bytes.to_vec()));
        self.next = (at + len + room).next_multiple_of(4);
        (at, len)
    }

    fn setting_text(&self, node: u32, name: &str, default: &str) -> String {
        self.graph.value(node, name).and_then(|v| v.as_str()).unwrap_or(default).to_owned()
    }

    fn setting_number(&self, node: u32, name: &str, default: f32) -> f32 {
        self.graph.value(node, name).and_then(bayleaf_graph::as_f32).unwrap_or(default)
    }

    /// Where a game value node reads and writes: the scope and handle arguments of `state_get` and
    /// `state_set` (as WebAssembly), and the name placed in memory.
    fn state_target(&mut self, node: u32) -> Result<(String, u32, u32), Problem> {
        let target = match self.setting_text(node, "scope", "Game").as_str() {
            "Game" => "(i32.const 0) (i32.const 0)".to_owned(),
            "Saved" => "(i32.const 2) (i32.const 0)".to_owned(),
            "This Object" => "(i32.const 1) (call $self_handle)".to_owned(),
            "Player" => {
                let player = self.setting_text(node, "player", "");
                if player.trim().is_empty() {
                    "(i32.const 1) (i32.const 0)".to_owned()
                } else {
                    // A name that finds nothing is no player (-1), not the first one (0).
                    let (at, len) = self.text(player.as_bytes());
                    format!("(i32.const 1) (call $or_none (call $find (i32.const {at}) (i32.const {len})))")
                }
            }
            _ => return Err(Problem::on(node, "the scope is Game, Player, This Object or Saved")),
        };
        let name = self.setting_text(node, "name", "score");
        let (at, len) = self.text(name.as_bytes());
        Ok((target, at, len))
    }

    /// The text of a name setting (or its default, as the editor shows it) placed in memory, or a
    /// problem when it is empty.
    fn named(&mut self, node: u32, setting: &str, missing: &str) -> Result<(u32, u32), Problem> {
        let default = self.graph.node_def(&CATALOG, node).ok().and_then(|(_, def)| def.setting(setting)).map_or("", |s| match s.kind {
            SettingKind::Text(d) | SettingKind::Name(d) => d,
            _ => "",
        });
        let name = self.setting_text(node, setting, default);
        if name.trim().is_empty() {
            return Err(Problem::on(node, missing));
        }
        Ok(self.text(name.as_bytes()))
    }

    /// The handle of the object a node works on: its linked Object, else the first object named in
    /// its Object Name, else `otherwise` (WebAssembly), or the problem `otherwise` holds.
    /// This Component, Object of Component, and On Component Event's outputs.
    fn component_output(&mut self, node: u32, pin: &str) -> Result<Val, Problem> {
        let kind = self.graph.node(node).map(|n| n.kind.clone()).unwrap_or_default();
        Ok(match (kind.as_str(), pin) {
            ("thisComponent", _) => Val::C(component_value("(call $self_handle)", "(call $self_component)")),
            ("componentObject", _) => {
                let component = self.input(node, "component").and_then(|v| as_component(node, v))?;
                Val::O(component_handle(&component))
            }
            (_, "source") => {
                if self.scope.as_deref() != Some(event_scope(node).as_str()) {
                    return Err(Problem::on(node, "the outputs of an On Component Event are read only on its own actions"));
                }
                Val::C(component_value("(call $event_source)", "(call $event_source_component)"))
            }
            _ => self.event_value(node, pin)?,
        })
    }

    /// An On Event's value `pin`, read from the event being delivered; only on its own actions.
    fn event_value(&mut self, node: u32, pin: &str) -> Result<Val, Problem> {
        if self.scope.as_deref() != Some(event_scope(node).as_str()) {
            return Err(Problem::on(node, "the values of an On Event are read only on its own actions"));
        }
        let params = self.event_params(node);
        let (k, param) = params.iter().enumerate().find(|(_, p)| p.name == pin).ok_or_else(|| Problem::on(node, format!("this event brings no {pin}")))?;
        let at = format!("(i32.const {k})");
        // A list or map it brings is kept as the chain begins (see `event_chain`).
        if !param.shape.is_single() {
            let (name_at, name_len) = self.text(event_list_name(node, pin).as_bytes());
            let name = format!("(call $mk (i32.const {name_at}) (i32.const {name_len}))");
            return Ok(if param.shape == Shape::Map { Val::M(name) } else { Val::L(name) });
        }
        Ok(match param.ty {
            VariableType::Number => Val::F(format!("(call $event_number {at})")),
            VariableType::WholeNumber => Val::I(format!("(call $event_whole {at})")),
            VariableType::TrueFalse => Val::B(format!("(call $event_flag {at})")),
            VariableType::Text => Val::T(format!("(call $event_string {at})")),
            VariableType::Object => Val::O(format!("(call $event_object {at})")),
            VariableType::Vector | VariableType::Color => Val::V([0, 1, 2].map(|j| format!("(call $event_vector {at} (i32.const {j}))"))),
        })
    }

    /// The values an On Event or Trigger Event node keeps (`params`, each `name:type`), in order.
    fn event_params(&self, node: u32) -> Vec<bayleaf_graph::Param> {
        event_params_of(self.graph, node)
    }

    /// Trigger Event: its values listed for `event_send` (per value a kind and three words, in
    /// memory taken for this call), then the event sent.
    /// The component linked into `node`'s `component` input, kept once in a global for this call:
    /// the global's name.
    fn component_of_input(&mut self, node: u32, indent: &str, code: &mut String) -> Result<String, Problem> {
        if self.graph.link_into(node, "component").is_none() {
            return Err(Problem::on(node, "link the component: a Get Component that names a script, or This Component"));
        }
        let value = self.input(node, "component").and_then(|v| as_component(node, v))?;
        let kept = format!("$component_{node}");
        self.global(format!("(global {kept} (mut i64) (i64.const 0))"));
        let _ = writeln!(code, "{indent}(global.set {kept} {value})");
        Ok(format!("(global.get {kept})"))
    }

    /// Subscribe to Event and Unsubscribe from Event: the component's event, for this script.
    fn subscribe(&mut self, node: u32, add: bool, indent: &str, code: &mut String) -> Result<(), Problem> {
        let name = self.setting_text(node, "event", "");
        if name.is_empty() {
            return Err(Problem::on(node, "choose the event"));
        }
        let (at, len) = self.text(name.as_bytes());
        let component = self.component_of_input(node, indent, code)?;
        let call = if add { "$event_subscribe" } else { "$event_unsubscribe" };
        let _ =
            writeln!(code, "{indent}(drop (call {call} {} {} (i32.const {at}) (i32.const {len})))", component_handle(&component), component_script(&component));
        Ok(())
    }

    fn trigger_event(&mut self, node: u32, indent: &str, code: &mut String) -> Result<(), Problem> {
        let name = self.setting_text(node, "event", "");
        if name.is_empty() {
            return Err(Problem::on(node, "choose the event to trigger"));
        }
        let (at, len) = self.text(name.as_bytes());
        // To one component, or to an object (else every object).
        let to_component = self.graph.node(node).is_some_and(|n| n.kind == "triggerComponentEvent");
        let send = if to_component {
            let component = self.component_of_input(node, indent, code)?;
            format!("(call $event_send_to {} {}", component_handle(&component), component_script(&component))
        } else {
            format!("(call $event_send {}", self.target(node, Ok("(i32.const -1)"))?)
        };
        let params = self.event_params(node);
        let list = format!("$events_{node}");
        self.global(format!("(global {list} (mut i32) (i32.const 0))"));
        let _ = writeln!(code, "{indent}(global.set {list} (call $alloc (i32.const {})))", params.len().max(1) * 16);
        for (k, p) in params.iter().enumerate() {
            let (param, ty) = (&p.name, &p.ty);
            let value = self.input(node, param)?;
            let place = |offset: usize| format!("(i32.add (global.get {list}) (i32.const {}))", k * 16 + offset);
            // A list or map goes by its name; the host copies it.
            if !p.shape.is_single() {
                let (kind, name) = if p.shape == Shape::Map { (7, as_map(node, value)?) } else { (6, as_list(node, value)?) };
                let _ = writeln!(code, "{indent}(i32.store {} (i32.const {kind}))", place(0));
                let _ = writeln!(code, "{indent}(i32.store {} (call $tp {name}))", place(4));
                let _ = writeln!(code, "{indent}(i32.store {} (call $tl {name}))", place(8));
                continue;
            }
            let kind = match ty {
                VariableType::Number => 0,
                VariableType::WholeNumber => 1,
                VariableType::TrueFalse => 2,
                VariableType::Vector | VariableType::Color => 3,
                VariableType::Text => 4,
                VariableType::Object => 5,
            };
            let _ = writeln!(code, "{indent}(i32.store {} (i32.const {kind}))", place(0));
            match ty {
                VariableType::Number => {
                    let _ = writeln!(code, "{indent}(f32.store {} {})", place(4), as_float(node, value)?);
                }
                VariableType::WholeNumber => {
                    let _ = writeln!(code, "{indent}(i32.store {} {})", place(4), as_int(node, value)?);
                }
                VariableType::TrueFalse => {
                    let _ = writeln!(code, "{indent}(i32.store {} {})", place(4), as_bool(node, value)?);
                }
                VariableType::Object => {
                    let _ = writeln!(code, "{indent}(i32.store {} {})", place(4), as_object(node, value)?);
                }
                VariableType::Vector | VariableType::Color => {
                    for (j, part) in as_vec(node, value)?.iter().enumerate() {
                        let _ = writeln!(code, "{indent}(f32.store {} {part})", place(4 + j * 4));
                    }
                }
                VariableType::Text => {
                    // The text once, then where it is and how long.
                    let kept = format!("$event_text_{node}_{k}");
                    self.global(format!("(global {kept} (mut i64) (i64.const 0))"));
                    let _ = writeln!(code, "{indent}(global.set {kept} {})", as_text(node, value)?);
                    let _ = writeln!(code, "{indent}(i32.store {} (call $tp (global.get {kept})))", place(4));
                    let _ = writeln!(code, "{indent}(i32.store {} (call $tl (global.get {kept})))", place(8));
                }
            }
        }
        let _ = writeln!(code, "{indent}(drop {send} (i32.const {at}) (i32.const {len}) (global.get {list}) (i32.const {})))", params.len());
        Ok(())
    }

    fn target(&mut self, node: u32, otherwise: Result<&str, &str>) -> Result<String, Problem> {
        if self.graph.link_into(node, "target").is_some() {
            return self.input(node, "target").and_then(|v| as_object(node, v));
        }
        let name = self.setting_text(node, "object", "");
        if !name.trim().is_empty() {
            let (at, len) = self.text(name.as_bytes());
            return Ok(format!("(call $find (i32.const {at}) (i32.const {len}))"));
        }
        otherwise.map(str::to_owned).map_err(|missing| Problem::on(node, missing))
    }

    /// The Component or Script a search looks for, placed in memory.
    fn kind(&mut self, node: u32) -> Result<(u32, u32), Problem> {
        self.named(node, "kind", "name a component (Point Light), a tag or a script to look for")
    }

    /// The component a Get, Try Get or Set Component node names, as the host knows it: a built-in
    /// component (`pointLight`) with its fields, or a script (by its name) with the properties the
    /// node keeps.
    fn component_target(&self, node: &bayleaf_graph::Node, id: u32) -> Result<Target, Problem> {
        if let Some(c) = component_of(node) {
            return Ok(Target { name: c.name.to_owned(), label: c.label.to_owned(), fields: c.fields(), settable: c.settable() });
        }
        let name = self.setting_text(id, "component", "");
        match fields::variant(node) {
            Some(def) if !name.trim().is_empty() => {
                let pins = if node.kind == "setComponent" { def.inputs } else { def.outputs };
                let skip = usize::from(node.kind != "getComponent") * if node.kind == "setComponent" { 2 } else { 1 };
                Ok(Target { label: name.clone(), name, fields: &pins[skip..], settable: true })
            }
            _ => Err(Problem::on(id, "choose a component, or a script and the properties it declares")),
        }
    }

    /// Declares a global once.
    fn global(&mut self, declaration: String) {
        if !self.globals.contains(&declaration) {
            self.globals.push(declaration);
        }
    }

    /// The function a Function, Return or Call Function node names.
    fn function_of(&self, node: u32) -> Result<bayleaf_graph::Function, Problem> {
        let name = self.setting_text(node, "function", "");
        if name.is_empty() {
            return Err(Problem::on(node, "choose a function"));
        }
        self.graph.function(&name).cloned().ok_or_else(|| Problem::on(node, format!("this script has no function {name}")))
    }

    /// The property a Property or Set Property node names, after checking its type: a declared one,
    /// or one the node declares at the type's zero.
    fn property_of(&mut self, node: u32) -> Result<String, Problem> {
        let ty = self.graph.node(node).and_then(|n| property_type(&n.kind)).unwrap_or(VariableType::Number);
        let name = self.setting_text(node, "name", "");
        if name.is_empty() {
            return Err(Problem::on(node, "choose a property"));
        }
        match self.graph.property(&name) {
            Some(declared) if !declared.shape.is_single() => {
                Err(Problem::on(node, format!("{name} is a {}: use the list and map nodes", declared.shape.label().to_lowercase())))
            }
            Some(declared) if declared.ty != ty => Err(Problem::on(node, format!("{name} is a {} property, not a {}", declared.ty.label(), ty.label()))),
            _ => {
                if !self.properties.iter().any(|(n, _, _)| *n == name) {
                    self.properties.push((name.clone(), described_type(ty, Shape::Single, VariableType::Text), described_default(ty, &ty.zero())));
                }
                Ok(name)
            }
        }
    }

    /// The actions that follow the flow output `pin` of `node`.
    fn chain(&mut self, node: u32, pin: &str, depth: usize) -> Result<String, Problem> {
        let graph = self.graph;
        let Some(link) = graph.links_from(node, pin).next() else { return Ok(String::new()) };
        let next = link.to.node;
        let (n, def) = graph.node_def(&CATALOG, next).map_err(|e| Problem::on(next, e.to_string()))?;
        let indent = "    ".repeat(depth + 1);
        let mut code = String::new();
        let vec3 = |c: &mut Self, name: &str| c.input(next, name).and_then(|v| as_vec(next, v));
        match n.kind.as_str() {
            "log" => {
                // The message, then the linked text, then the linked number, with spaces between.
                let message = self.setting_text(next, "message", "Hello");
                let mut line = self.literal(&message);
                let mut joined = !message.is_empty();
                for (input, is_text) in [("text", true), ("value", false)] {
                    if graph.link_into(next, input).is_none() {
                        continue;
                    }
                    let part = if is_text {
                        self.input(next, input).and_then(|v| as_text(next, v))?
                    } else {
                        self.input(next, input).and_then(|v| number_text(next, v))?
                    };
                    if joined {
                        let space = self.literal(" ");
                        line = format!("(call $join {line} {space})");
                    }
                    line = format!("(call $join {line} {part})");
                    joined = true;
                }
                let _ = writeln!(code, "{indent}(call $log_text {line})");
            }
            "setPosition" | "setRotation" | "setScale" | "moveBy" | "rotateBy" => {
                let (input, offset, call) = match n.kind.as_str() {
                    "setPosition" => ("position", 0, "$set3"),
                    "setRotation" => ("rotation", 12, "$set3"),
                    "setScale" => ("scale", 24, "$set3"),
                    "moveBy" => ("offset", 0, "$add3"),
                    _ => ("degrees", 12, "$add3"),
                };
                let [x, y, z] = vec3(self, input)?;
                if n.kind == "moveBy" {
                    let _ = writeln!(code, "{indent}(call $move_by {x} {y} {z})");
                } else {
                    let _ = writeln!(code, "{indent}(call {call} (i32.const {offset}) {x} {y} {z})");
                }
            }
            "setPositionOf" => {
                let handle = self.target(next, Err("link or name the object to move"))?;
                let [x, y, z] = vec3(self, "position")?;
                let _ = writeln!(code, "{indent}(call $set_position_of {handle} {x} {y} {z})");
            }
            "setComponent" => {
                let component = self.component_target(n, next)?;
                if !component.settable {
                    return Err(Problem::on(
                        next,
                        format!("{} is read only in play: physics reads it when play starts; use Add Impulse, Set Velocity or Walk", component.label),
                    ));
                }
                let handle = self.target(next, Ok("(call $self_handle)"))?;
                let (c_at, c_len) = self.text(component.name.as_bytes());
                // Only linked fields change; the others keep their values.
                for field in component.fields {
                    if graph.link_into(next, field.name).is_none() {
                        continue;
                    }
                    let value = self.input(next, field.name)?;
                    let zero = lit(0.0);
                    let [x, y, z] = match field.ty {
                        Float => [as_float(next, value)?, zero.clone(), zero],
                        PinType::Int => [format!("(f32.convert_i32_s {})", as_int(next, value)?), zero.clone(), zero],
                        Bool => [format!("(f32.convert_i32_u {})", as_bool(next, value)?), zero.clone(), zero],
                        _ => as_vec(next, value)?,
                    };
                    let (f_at, f_len) = self.text(field.name.as_bytes());
                    let _ = writeln!(
                        code,
                        "{indent}(drop (call $set_field {handle} (i32.const {c_at}) (i32.const {c_len}) (i32.const {f_at}) (i32.const {f_len}) {x} {y} {z}))"
                    );
                }
            }
            "destroyObject" => {
                let handle = self.target(next, Ok("(call $self_handle)"))?;
                let _ = writeln!(code, "{indent}(drop (call $destroy {handle}))");
            }
            "listForEach" | "mapForEach" => self.collection_loop(next, &n.kind, depth, &indent, &mut code)?,
            kind if COLLECTION_KINDS.contains(&kind) => self.collection_action(next, kind, &indent, &mut code)?,
            "forEachWith" => {
                // The object and index live in globals, so the nodes in Each can read them.
                let (at, len) = self.kind(next)?;
                let (object, index) = (format!("$each_{next}"), format!("$each_index_{next}"));
                self.global(format!("(global {object} (mut i32) (i32.const 0))"));
                self.global(format!("(global {index} (mut i32) (i32.const 0))"));
                let body = self.chain(next, "each", depth + 2)?;
                let _ = writeln!(
                    code,
                    "{indent}(global.set {index} (i32.const 0))\n{indent}(block $done_{next} (loop $next_{next}\n{indent}  (global.set {object} (call $find_with (i32.const {at}) (i32.const {len}) (global.get {index})))\n{indent}  (br_if $done_{next} (i32.eqz (global.get {object})))\n{body}{indent}  (global.set {index} (i32.add (global.get {index}) (i32.const 1)))\n{indent}  (br $next_{next})))"
                );
            }
            "spawn" => {
                let prefab = self.setting_text(next, "prefab", "");
                if !prefab.ends_with(".bprefab") {
                    return Err(Problem::on(next, "choose a .bprefab file to spawn"));
                }
                let (at, len) = self.text(prefab.as_bytes());
                let [x, y, z] = vec3(self, "position")?;
                let _ = writeln!(code, "{indent}(drop (call $spawn (i32.const {at}) (i32.const {len}) {x} {y} {z}))");
            }
            "playSound" | "stopSound" => {
                let handle = self.target(next, Ok("(call $self_handle)"))?;
                let _ = writeln!(code, "{indent}(drop (call $sound_{} {handle}))", if n.kind == "playSound" { "play" } else { "stop" });
            }
            "playSoundAt" => {
                let sound = self.setting_text(next, "sound", "");
                let extension = sound.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
                if !bayleaf_scene::components::SOUND_EXTENSIONS.contains(&extension.as_str()) {
                    return Err(Problem::on(next, format!("choose a sound file ({})", bayleaf_scene::components::SOUND_EXTENSIONS.join(", "))));
                }
                let (at, len) = self.text(sound.as_bytes());
                let [x, y, z] = vec3(self, "position")?;
                let volume = self.input(next, "volume").and_then(|v| as_float(next, v))?;
                let _ = writeln!(code, "{indent}(drop (call $sound_play_at (i32.const {at}) (i32.const {len}) {x} {y} {z} {volume}))");
            }
            "loadScene" | "unloadScene" => {
                let scene = self.setting_text(next, "scene", "");
                if !scene.ends_with(".bscene") {
                    return Err(Problem::on(next, "choose a .bscene file"));
                }
                let (at, len) = self.text(scene.as_bytes());
                if n.kind == "loadScene" {
                    let additive = self.input(next, "additive").and_then(|v| as_bool(next, v))?;
                    let _ = writeln!(code, "{indent}(drop (call $scene_load (i32.const {at}) (i32.const {len}) {additive}))");
                } else {
                    let _ = writeln!(code, "{indent}(drop (call $scene_unload (i32.const {at}) (i32.const {len})))");
                }
            }
            "destroySelf" => {
                let _ = writeln!(code, "{indent}(drop (call $destroy (call $self_handle)))");
            }
            "setGameValue" | "changeGameValue" => {
                let (target, at, len) = self.state_target(next)?;
                let input = if n.kind == "setGameValue" { "value" } else { "amount" };
                let value = self.input(next, input).and_then(|v| as_float(next, v))?;
                let new = if n.kind == "setGameValue" {
                    format!("(f64.promote_f32 {value})")
                } else {
                    format!("(f64.add (call $state_f64 {target} (i32.const {at}) (i32.const {len})) (f64.promote_f32 {value}))")
                };
                let _ = writeln!(code, "{indent}(drop (call $state_set {target} (i32.const {at}) (i32.const {len}) {new}))");
            }
            "setInputMap" => {
                let (at, len) = self.named(next, "map", "name the map to make active")?;
                let handle = if self.setting_text(next, "scope", "Project") == "This Object" { "(call $self_handle)" } else { "(i32.const 0)" };
                let _ = writeln!(code, "{indent}(drop (call $input_map_set {handle} (i32.const {at}) (i32.const {len})))");
            }
            "endGame" => {
                let won = self.input(next, "won").and_then(|v| as_bool(next, v))?;
                let _ = writeln!(code, "{indent}(drop (call $game_end {won}))");
            }
            "playParticles" | "stopParticles" | "burstParticles" => {
                let handle = self.target(next, Ok("(call $self_handle)"))?;
                match n.kind.as_str() {
                    "playParticles" => {
                        let _ = writeln!(code, "{indent}(drop (call $particles_play {handle}))");
                    }
                    "stopParticles" => {
                        let _ = writeln!(code, "{indent}(drop (call $particles_stop {handle}))");
                    }
                    _ => {
                        let count = self.input(next, "count").and_then(|v| as_int(next, v))?;
                        // A count below 0 starts none.
                        let _ =
                            writeln!(code, "{indent}(drop (call $particles_burst {handle} (select (i32.const 0) {count} (i32.lt_s {count} (i32.const 0)))))");
                    }
                }
            }
            "setInterfaceText" | "setInterfaceValue" | "showInterface" => {
                let handle = self.target(next, Ok("(call $self_handle)"))?;
                let element = self.setting_text(next, "element", if n.kind == "showInterface" { "" } else { "Start" });
                if element.trim().is_empty() && n.kind != "showInterface" {
                    return Err(Problem::on(next, "name the element"));
                }
                let (at, len) = self.text(element.as_bytes());
                match n.kind.as_str() {
                    "setInterfaceText" => {
                        let text = self.input(next, "text").and_then(|v| as_text(next, v))?;
                        let _ = writeln!(code, "{indent}(call $ui_text {handle} (i32.const {at}) (i32.const {len}) {text})");
                    }
                    "setInterfaceValue" => {
                        let value = self.input(next, "value").and_then(|v| as_float(next, v))?;
                        let _ = writeln!(code, "{indent}(drop (call $ui_set_value {handle} (i32.const {at}) (i32.const {len}) (f64.promote_f32 {value})))");
                    }
                    _ => {
                        let visible = self.input(next, "visible").and_then(|v| as_bool(next, v))?;
                        let _ = writeln!(code, "{indent}(drop (call $ui_show {handle} (i32.const {at}) (i32.const {len}) {visible}))");
                    }
                }
            }
            "addInterfaceElement" | "removeInterfaceElement" => {
                let handle = self.target(next, Ok("(call $self_handle)"))?;
                let name = self.input(next, "name").and_then(|v| as_text(next, v))?;
                if n.kind == "addInterfaceElement" {
                    let into = self.setting_text(next, "into", "");
                    let what = self.setting_text(next, "what", "button");
                    if what.trim().is_empty() {
                        return Err(Problem::on(next, "say what to add: a kind (button, text...) or a .bui file"));
                    }
                    let (into_at, into_len) = self.text(into.as_bytes());
                    let (what_at, what_len) = self.text(what.as_bytes());
                    let _ = writeln!(
                        code,
                        "{indent}(call $ui_add_named {handle} (i32.const {into_at}) (i32.const {into_len}) (i32.const {what_at}) (i32.const {what_len}) {name})"
                    );
                } else {
                    let _ = writeln!(code, "{indent}(call $ui_remove_named {handle} {name})");
                }
            }
            "walk" | "jump" => {
                let handle = self.target(next, Ok("(call $self_handle)"))?;
                if n.kind == "walk" {
                    let [x, y, z] = vec3(self, "velocity")?;
                    let _ = writeln!(code, "{indent}(drop (call $character_walk {handle} {x} {y} {z}))");
                } else {
                    let speed = self.input(next, "speed").and_then(|v| as_float(next, v))?;
                    let _ = writeln!(code, "{indent}(drop (call $character_jump {handle} {speed}))");
                }
            }
            "addImpulse" | "setVelocity" => {
                let (input, call) = if n.kind == "addImpulse" { ("impulse", "$apply_impulse") } else { ("velocity", "$set_velocity") };
                let handle = self.target(next, Ok("(call $self_handle)"))?;
                let [x, y, z] = vec3(self, input)?;
                let _ = writeln!(code, "{indent}(drop (call {call} {handle} {x} {y} {z}))");
            }
            "triggerEvent" | "triggerComponentEvent" => self.trigger_event(next, &indent, &mut code)?,
            "subscribeEvent" => self.subscribe(next, true, &indent, &mut code)?,
            "unsubscribeEvent" => self.subscribe(next, false, &indent, &mut code)?,
            "sendMessage" => {
                let name = self.setting_text(next, "message", "hit");
                let (at, len) = self.text(name.as_bytes());
                let handle = self.target(next, Ok("(i32.const -1)"))?;
                let value = self.input(next, "value").and_then(|v| as_float(next, v))?;
                let _ = writeln!(code, "{indent}(drop (call $send {handle} (i32.const {at}) (i32.const {len}) {value}))");
            }
            "setVectorVariable" => {
                let name = self.setting_text(next, "name", "target");
                let [x, y, z] = vec3(self, "value")?;
                let _ =
                    writeln!(code, "{indent}(global.set $vec_{name}_x {x})\n{indent}(global.set $vec_{name}_y {y})\n{indent}(global.set $vec_{name}_z {z})");
            }
            "setWholeVariable" => {
                let name = self.setting_text(next, "name", "total");
                let value = self.input(next, "value").and_then(|v| as_int(next, v))?;
                let _ = writeln!(code, "{indent}(global.set $int_{name} {value})");
            }
            "setTextVariable" => {
                let name = self.setting_text(next, "name", "label");
                let at = self.text_vars.get(&name).map_or(0, |(at, _)| *at);
                let value = self.input(next, "value").and_then(|v| as_text(next, v))?;
                let _ = writeln!(code, "{indent}(global.set $txt_{name} (call $keep (i32.const {at}) (i32.const {TEXT_VARIABLE_BYTES}) {value}))");
            }
            "setObjectVariable" => {
                let name = self.setting_text(next, "name", "thing");
                let value = self.input(next, "value").and_then(|v| as_object(next, v))?;
                let _ = writeln!(code, "{indent}(global.set $obj_{name} (call $object_ref {value}))");
            }
            "setColorVariable" => {
                let name = self.setting_text(next, "name", "tint");
                let [r, g, b] = vec3(self, "value")?;
                let _ =
                    writeln!(code, "{indent}(global.set $col_{name}_r {r})\n{indent}(global.set $col_{name}_g {g})\n{indent}(global.set $col_{name}_b {b})");
            }
            "setFlag" => {
                let name = self.setting_text(next, "name", "done");
                let value = self.input(next, "value").and_then(|v| as_bool(next, v))?;
                let _ = writeln!(code, "{indent}(global.set $flag_{name} {value})");
            }
            "setVariable" => {
                let name = self.setting_text(next, "name", "count");
                let value = self.input(next, "value").and_then(|v| as_float(next, v))?;
                let _ = writeln!(code, "{indent}(global.set $var_{name} {value})");
            }
            "branch" => {
                let condition = self.input(next, "condition").and_then(|v| as_bool(next, v))?;
                let yes = self.chain(next, "true", depth + 1)?;
                let no = self.chain(next, "false", depth + 1)?;
                let _ = writeln!(code, "{indent}(if {condition}\n{indent}  (then\n{yes}{indent}  )\n{indent}  (else\n{no}{indent}  ))");
                return Ok(code);
            }
            "setProperty" | "setWholeProperty" | "setFlagProperty" | "setVectorProperty" | "setColorProperty" | "setTextProperty" | "setObjectProperty" => {
                let name = self.property_of(next)?;
                let ty = property_type(&n.kind).unwrap_or(VariableType::Number);
                let (at, len) = self.text(name.as_bytes());
                let key = format!("(i32.const {at}) (i32.const {len})");
                let value = self.input(next, "value")?;
                let zero = lit(0.0);
                let set = match ty {
                    VariableType::Text => format!("(call $property_set_string {key} {})", as_text(next, value)?),
                    VariableType::Object => format!("(call $property_set_string {key} (call $name {}))", as_object(next, value)?),
                    VariableType::Number => format!("(call $property_set {key} {} {zero} {zero})", as_float(next, value)?),
                    VariableType::WholeNumber => format!("(call $property_set {key} (f32.convert_i32_s {}) {zero} {zero})", as_int(next, value)?),
                    VariableType::TrueFalse => format!("(call $property_set {key} (f32.convert_i32_u {}) {zero} {zero})", as_bool(next, value)?),
                    VariableType::Vector | VariableType::Color => {
                        let [x, y, z] = as_vec(next, value)?;
                        format!("(call $property_set {key} {x} {y} {z})")
                    }
                };
                let _ = writeln!(code, "{indent}(drop {set})");
            }
            "callFunction" => {
                let function = self.function_of(next)?;
                let mut args = Vec::new();
                for param in &function.inputs {
                    let value = self.input(next, &param.name)?;
                    args.extend(parts_of(next, param, value)?);
                }
                let mut kept = Vec::new();
                for (k, param) in function.outputs.iter().enumerate() {
                    for (j, ty) in wasm_types(param).iter().enumerate() {
                        let global = format!("$ret_{next}_{k}_{j}");
                        self.global(format!("(global {global} (mut {ty}) ({ty}.const 0))"));
                        kept.push(global);
                    }
                }
                let _ = writeln!(code, "{indent}(call $fn_{} {})", function.name, args.join(" "));
                // What it gave back, last first, kept for the nodes that read its outputs.
                for global in kept.iter().rev() {
                    let _ = writeln!(code, "{indent}(global.set {global})");
                }
            }
            "functionReturn" => {
                let function = self.function_of(next)?;
                if self.scope.as_deref() != Some(function.name.as_str()) {
                    return Err(Problem::on(
                        next,
                        format!("a Return of {} goes only on {}'s own actions, after its Function node", function.name, function.name),
                    ));
                }
                let mut values = Vec::new();
                for param in &function.outputs {
                    let value = self.input(next, &param.name)?;
                    values.extend(parts_of(next, param, value)?);
                }
                let _ = writeln!(code, "{indent}(return {})", values.join(" "));
            }
            _ => return Err(Problem::on(next, format!("{} is not an action; link flow only into actions", def.label))),
        }
        code.push_str(&self.chain(next, "then", depth)?);
        Ok(code)
    }

    /// The value of an input: its link, its stored value, or its default.
    fn input(&mut self, node: u32, pin: &str) -> Result<Val, Problem> {
        let graph = self.graph;
        let (_, def) = graph.node_def(&CATALOG, node).map_err(|e| Problem::on(node, e.to_string()))?;
        let pin_def = def.input(pin).ok_or_else(|| Problem::on(node, format!("{} has no input {pin}", def.label)))?;
        if let Some(link) = graph.link_into(node, pin) {
            return self.output(link.from.node, &link.from.pin);
        }
        if pin_def.ty == PinType::Int {
            let stored = graph.value(node, pin).and_then(serde_json::Value::as_i64).and_then(|n| i32::try_from(n).ok());
            let default = match pin_def.default {
                PinDefault::Int(n) => n,
                _ => 0,
            };
            return Ok(Val::I(format!("(i32.const {})", stored.unwrap_or(default))));
        }
        if pin_def.ty == PinType::Text {
            let typed = graph.value(node, pin).and_then(|v| v.as_str()).map(str::to_owned);
            let default = match pin_def.default {
                PinDefault::Text(text) => text,
                _ => "",
            };
            return Ok(Val::T(self.literal(typed.as_deref().unwrap_or(default))));
        }
        if let Some(value) = graph.value(node, pin) {
            if let Some(v) = value.as_bool() {
                return Ok(Val::B(format!("(i32.const {})", i32::from(v))));
            }
            if let Some(v) = bayleaf_graph::as_f32(value) {
                return Ok(if pin_def.ty == Vec3 { Val::V([lit(v), lit(v), lit(v)]) } else { Val::F(lit(v)) });
            }
            if let Some([x, y, z]) = bayleaf_graph::as_triple(value) {
                return Ok(Val::V([lit(x), lit(y), lit(z)]));
            }
        }
        Ok(match (pin_def.default, pin_def.ty) {
            (_, PinType::Object) => Val::O("(call $self_handle)".to_owned()),
            (PinDefault::Bool(b), _) => Val::B(format!("(i32.const {})", i32::from(b))),
            (_, Bool) => Val::B("(i32.const 0)".to_owned()),
            (PinDefault::Vec3(v) | PinDefault::Color(v), _) => Val::V(v.map(lit)),
            (PinDefault::Float(v), Vec3 | PinType::Color) => Val::V([lit(v), lit(v), lit(v)]),
            (PinDefault::Float(v), _) => Val::F(lit(v)),
            (_, Vec3 | PinType::Color) => Val::V([lit(0.0), lit(0.0), lit(0.0)]),
            _ => Val::F(lit(0.0)),
        })
    }

    /// Power, Square Root and Vector Length.
    fn power_and_roots(&mut self, node: u32, kind: &str) -> Result<Val, Problem> {
        Ok(match kind {
            "power" => {
                let base = self.input(node, "base")?;
                let exponent = self.input(node, "exponent")?;
                match (base, exponent) {
                    (Val::I(b), Val::I(e)) => Val::I(format!("(call $ipow {b} {e})")),
                    (base, exponent) => {
                        let e = as_float(node, exponent)?;
                        match base {
                            Val::F(b) => Val::F(format!("(call $math_pow {b} {e})")),
                            Val::I(b) => Val::F(format!("(call $math_pow (f32.convert_i32_s {b}) {e})")),
                            Val::V(v) => Val::V(v.map(|c| format!("(call $math_pow {c} {e})"))),
                            Val::B(_) | Val::O(_) | Val::T(_) | Val::C(_) | Val::L(_) | Val::M(_) => {
                                return Err(Problem::on(node, "Power needs a number or a vector"));
                            }
                        }
                    }
                }
            }
            "squareRoot" => {
                let root = |v: &str| format!("(f32.sqrt (f32.max {v} (f32.const 0)))");
                match self.input(node, "value")? {
                    Val::F(v) => Val::F(root(&v)),
                    Val::I(i) => Val::F(root(&format!("(f32.convert_i32_s {i})"))),
                    Val::V(v) => Val::V(v.map(|c| root(&c))),
                    Val::B(_) | Val::O(_) | Val::T(_) | Val::C(_) | Val::L(_) | Val::M(_) => {
                        return Err(Problem::on(node, "Square Root needs a number or a vector"));
                    }
                }
            }
            _ => {
                let [x, y, z] = as_vec(node, self.input(node, "vector")?)?;
                Val::F(format!("(call $len3 {x} {y} {z})"))
            }
        })
    }

    /// The value of one output of a data node.
    fn output(&mut self, node: u32, pin: &str) -> Result<Val, Problem> {
        let graph = self.graph;
        let (n, def) = graph.node_def(&CATALOG, node).map_err(|e| Problem::on(node, e.to_string()))?;
        let transform = |base: u32| Val::V([0, 1, 2].map(|i| format!("(call $t (i32.const {}))", base + i)));
        Ok(match n.kind.as_str() {
            "eventStart" => self.event_value(node, pin)?,
            "functionStart" => {
                let function = self.function_of(node)?;
                if self.scope.as_deref() != Some(function.name.as_str()) {
                    return Err(Problem::on(node, format!("the values of {}'s Function node are read only on {}'s own actions", function.name, function.name)));
                }
                let (k, param) = function
                    .inputs
                    .iter()
                    .enumerate()
                    .find(|(_, p)| p.name == pin)
                    .ok_or_else(|| Problem::on(node, format!("{} has no input {pin}", function.name)))?;
                val_of(param, (0..wasm_types(param).len()).map(|j| format!("(local.get $a{k}_{j})")).collect())
            }
            "callFunction" => {
                let function = self.function_of(node)?;
                let (k, param) = function
                    .outputs
                    .iter()
                    .enumerate()
                    .find(|(_, p)| p.name == pin)
                    .ok_or_else(|| Problem::on(node, format!("{} has no output {pin}", function.name)))?;
                let parts: Vec<String> = wasm_types(param)
                    .iter()
                    .enumerate()
                    .map(|(j, ty)| {
                        let global = format!("$ret_{node}_{k}_{j}");
                        self.global(format!("(global {global} (mut {ty}) ({ty}.const 0))"));
                        format!("(global.get {global})")
                    })
                    .collect();
                val_of(param, parts)
            }
            "update" if pin == "dt" => Val::F("(global.get $dt)".to_owned()),
            "message" | "interfaceChanged" if pin == "value" => Val::F("(global.get $message)".to_owned()),
            // The other object comes as its handle in the message's number.
            "triggerEntered" | "triggerLeft" | "collisionStarted" | "collisionEnded" => Val::O("(i32.trunc_sat_f32_s (global.get $message))".to_owned()),
            "vectorVariable" => {
                let name = self.setting_text(node, "name", "target");
                Val::V(["x", "y", "z"].map(|axis| format!("(global.get $vec_{name}_{axis})")))
            }
            "flag" => Val::B(format!("(global.get $flag_{})", self.setting_text(node, "name", "done"))),
            "wholeVariable" => Val::I(format!("(global.get $int_{})", self.setting_text(node, "name", "total"))),
            "textVariable" => {
                let name = self.setting_text(node, "name", "label");
                let at = self.text_vars.get(&name).map_or(0, |(at, _)| *at);
                Val::T(format!("(call $mk (i32.const {at}) (global.get $txt_{name}))"))
            }
            "objectVariable" => {
                let name = self.setting_text(node, "name", "thing");
                let handle = format!("(call $object_of (global.get $obj_{name}))");
                if pin == "found" { Val::B(format!("(i32.ne {handle} (i32.const 0))")) } else { Val::O(handle) }
            }
            "colorVariable" => {
                let name = self.setting_text(node, "name", "tint");
                Val::V(["r", "g", "b"].map(|part| format!("(global.get $col_{name}_{part})")))
            }
            "variable" => Val::F(format!("(global.get $var_{})", self.setting_text(node, "name", "count"))),
            "number" => Val::F(lit(self.setting_number(node, "value", 1.0))),
            "vector" => {
                let x = self.input(node, "x").and_then(|v| as_float(node, v))?;
                let y = self.input(node, "y").and_then(|v| as_float(node, v))?;
                let z = self.input(node, "z").and_then(|v| as_float(node, v))?;
                Val::V([x, y, z])
            }
            "split" => {
                let [x, y, z] = self.input(node, "vector").and_then(|v| as_vec(node, v))?;
                Val::F(match pin {
                    "x" => x,
                    "y" => y,
                    _ => z,
                })
            }
            "property" | "wholeProperty" | "flagProperty" | "vectorProperty" | "colorProperty" | "textProperty" | "objectProperty" => {
                let name = self.property_of(node)?;
                let (at, len) = self.text(name.as_bytes());
                let key = format!("(i32.const {at}) (i32.const {len})");
                match n.kind.as_str() {
                    "property" => Val::F(format!("(call $property {key})")),
                    "wholeProperty" => Val::I(format!("(call $property_whole {key})")),
                    "flagProperty" => Val::B(format!("(i32.ne (call $property_whole {key}) (i32.const 0))")),
                    "vectorProperty" | "colorProperty" => Val::V([0, 1, 2].map(|i| format!("(call $property_part {key} (i32.const {i}))"))),
                    "textProperty" => Val::T(format!("(call $property_string {key})")),
                    _ => {
                        let handle = format!("(call $property_object {key})");
                        if pin == "found" { Val::B(format!("(i32.ne {handle} (i32.const 0))")) } else { Val::O(handle) }
                    }
                }
            }
            "position" => transform(0),
            "rotation" => transform(3),
            "scale" => transform(6),
            "time" => Val::F("(f32.demote_f64 (call $time))".to_owned()),
            "pointer" if pin == "inside" => Val::B("(call $pointer_inside)".to_owned()),
            "pointer" if pin == "overInterface" => Val::B("(call $pointer_over_interface)".to_owned()),
            "pointer" => Val::V(["(call $pointer_x)".to_owned(), "(call $pointer_y)".to_owned(), lit(0.0)]),
            "mouseDown" => {
                let button = match self.setting_text(node, "button", "Left").as_str() {
                    "Left" => 0,
                    "Right" => 1,
                    "Middle" => 2,
                    _ => return Err(Problem::on(node, "the button is Left, Right or Middle")),
                };
                Val::B(format!("(call $mouse_down (i32.const {button}))"))
            }
            "positionOf" => {
                let handle = self.target(node, Err("link or name the object to read"))?;
                Val::V([0, 1, 2].map(|i| format!("(call $t_of {handle} (i32.const {i}))")))
            }
            "getComponent" | "tryGetComponent" => {
                let component = self.component_target(n, node)?;
                let handle = self.target(node, Ok("(call $self_handle)"))?;
                let (c_at, c_len) = self.text(component.name.as_bytes());
                if pin == "component" {
                    return Ok(Val::C(format!("(call $component_of {handle} (i32.const {c_at}) (i32.const {c_len}))")));
                }
                if pin == "found" {
                    // An empty field asks whether the object has the component.
                    Val::B(format!("(call $component_get {handle} (i32.const {c_at}) (i32.const {c_len}) (i32.const 0) (i32.const 0) (i32.const 160))"))
                } else {
                    let field =
                        component.fields.iter().find(|f| f.name == pin).ok_or_else(|| Problem::on(node, format!("{} has no field {pin}", component.label)))?;
                    let (f_at, f_len) = self.text(field.name.as_bytes());
                    let part = |offset: u32| {
                        format!(
                            "(call $field_value {handle} (i32.const {c_at}) (i32.const {c_len}) (i32.const {f_at}) (i32.const {f_len}) (i32.const {offset}))"
                        )
                    };
                    match field.ty {
                        Float => Val::F(part(0)),
                        PinType::Int => Val::I(format!("(i32.trunc_sat_f32_s {})", part(0))),
                        Bool => Val::B(format!("(f32.ne {} (f32.const 0))", part(0))),
                        _ => Val::V([part(0), part(4), part(8)]),
                    }
                }
            }
            "text" => Val::T(self.input(node, "value").and_then(|v| as_text(node, v))?),
            "wholeNumber" => Val::I(self.input(node, "value").and_then(|v| as_int(node, v))?),
            "toWhole" => {
                let v = self.input(node, "value").and_then(|v| as_float(node, v))?;
                let rounded = match self.setting_text(node, "rounding", "Nearest").as_str() {
                    "Down" => format!("(f32.floor {v})"),
                    "Up" => format!("(f32.ceil {v})"),
                    "Toward Zero" => v,
                    _ => format!("(call $round {v})"),
                };
                Val::I(format!("(i32.trunc_sat_f32_s {rounded})"))
            }
            "remainder" => {
                let a = self.input(node, "a").and_then(|v| as_int(node, v))?;
                let b = self.input(node, "b").and_then(|v| as_int(node, v))?;
                Val::I(format!("(call $irem {a} {b})"))
            }
            "joinText" | "sameText" | "textContains" | "startsWith" | "endsWith" | "findText" => {
                let (first, second) = if matches!(n.kind.as_str(), "joinText" | "sameText") { ("a", "b") } else { ("text", "part") };
                let a = self.input(node, first).and_then(|v| as_text(node, v))?;
                let b = self.input(node, second).and_then(|v| as_text(node, v))?;
                match n.kind.as_str() {
                    "joinText" => Val::T(format!("(call $join {a} {b})")),
                    "sameText" => Val::B(format!("(call $same {a} {b})")),
                    "textContains" => Val::B(format!("(i32.ge_s (call $find_bytes {a} {b} (i32.const 0)) (i32.const 0))")),
                    "startsWith" => Val::B(format!("(call $at {a} {b} (i32.const 0))")),
                    "endsWith" => Val::B(format!("(call $ends {a} {b})")),
                    _ => Val::I(format!("(call $find_chars {a} {b})")),
                }
            }
            "textLength" => Val::I(format!("(call $length {})", self.input(node, "text").and_then(|v| as_text(node, v))?)),
            "numberToText" => {
                let value = self.input(node, "value")?;
                Val::T(self.value_text(node, value)?)
            }
            "textToNumber" => {
                let t = self.input(node, "text").and_then(|v| as_text(node, v))?;
                if pin == "valid" { Val::B(format!("(call $is_number {t})")) } else { Val::F(format!("(call $number_or_zero {t})")) }
            }
            "upperCase" | "lowerCase" => {
                let t = self.input(node, "text").and_then(|v| as_text(node, v))?;
                Val::T(format!("(call $case {t} (i32.const {}))", i32::from(n.kind == "upperCase")))
            }
            "trimText" => Val::T(format!("(call $trim {})", self.input(node, "text").and_then(|v| as_text(node, v))?)),
            "textPart" => {
                let t = self.input(node, "text").and_then(|v| as_text(node, v))?;
                let start = self.input(node, "start").and_then(|v| as_float(node, v))?;
                let count = self.input(node, "count").and_then(|v| as_float(node, v))?;
                Val::T(format!("(call $part {t} (call $index {start}) (call $index {count}))"))
            }
            "replaceText" => {
                let t = self.input(node, "text").and_then(|v| as_text(node, v))?;
                let find = self.input(node, "find").and_then(|v| as_text(node, v))?;
                let with = self.input(node, "with").and_then(|v| as_text(node, v))?;
                Val::T(format!("(call $replace {t} {find} {with})"))
            }
            "nameOf" => Val::T(format!("(call $name {})", self.target(node, Ok("(call $self_handle)"))?)),
            "interfaceValue" | "interfaceText" => {
                let handle = self.target(node, Ok("(call $self_handle)"))?;
                let (at, len) = self.named(node, "element", "name the element")?;
                match (n.kind.as_str(), pin) {
                    ("interfaceText", _) => Val::T(format!("(call $ui_text_of {handle} (i32.const {at}) (i32.const {len}))")),
                    (_, "found") => Val::B(format!(
                        "(f64.eq (call $ui_value {handle} (i32.const {at}) (i32.const {len})) (call $ui_value {handle} (i32.const {at}) (i32.const {len})))"
                    )),
                    _ => Val::F(format!("(call $ui_value_or_zero {handle} (i32.const {at}) (i32.const {len}))")),
                }
            }
            "thisObject" => Val::O("(call $self_handle)".to_owned()),
            "thisComponent" | "componentObject" | "componentEventStart" => self.component_output(node, pin)?,
            "findNamed" => {
                let found = if graph.link_into(node, "named").is_some() {
                    format!("(call $find_text {})", self.input(node, "named").and_then(|v| as_text(node, v))?)
                } else {
                    let (at, len) = self.named(node, "name", "name the object to find")?;
                    format!("(call $find (i32.const {at}) (i32.const {len}))")
                };
                if pin == "found" { Val::B(format!("(i32.ne {found} (i32.const 0))")) } else { Val::O(found) }
            }
            "findWith" => {
                let (at, len) = self.kind(node)?;
                let index = self.input(node, "index").and_then(|v| as_float(node, v))?;
                let found = format!("(call $find_with (i32.const {at}) (i32.const {len}) (i32.trunc_sat_f32_s {index}))");
                if pin == "found" { Val::B(format!("(i32.ne {found} (i32.const 0))")) } else { Val::O(found) }
            }
            "countWith" => {
                let (at, len) = self.kind(node)?;
                Val::I(format!("(call $count_with (i32.const {at}) (i32.const {len}))"))
            }
            "forEachWith" if pin == "object" => Val::O(format!("(global.get $each_{node})")),
            "forEachWith" if pin == "index" => Val::I(format!("(global.get $each_index_{node})")),
            "gameValue" => {
                let (target, at, len) = self.state_target(node)?;
                let get = format!("(call $state_get {target} (i32.const {at}) (i32.const {len}) (i32.const 128))");
                if pin == "set" { Val::B(get) } else { Val::F(format!("(f32.demote_f64 (call $state_f64 {target} (i32.const {at}) (i32.const {len})))")) }
            }
            "action" => {
                let name = self.setting_text(node, "action", "Jump");
                let (at, len) = self.text(name.as_bytes());
                let value = format!("(call $action (i32.const {at}) (i32.const {len}))");
                if pin == "held" { Val::B(format!("(f32.gt {value} (f32.const 0.5))")) } else { Val::F(value) }
            }
            "onGround" => Val::B("(call $character_grounded (call $self_handle))".to_owned()),
            "velocity" => Val::V([0, 4, 8].map(|offset| format!("(call $velocity_value (i32.const {offset}))"))),
            "raycast" => {
                let [ox, oy, oz] = self.input(node, "origin").and_then(|v| as_vec(node, v))?;
                let [dx, dy, dz] = self.input(node, "direction").and_then(|v| as_vec(node, v))?;
                let max = self.input(node, "distance").and_then(|v| as_float(node, v))?;
                let args = format!("{ox} {oy} {oz} {dx} {dy} {dz} {max}");
                let at = |offset: u32| format!("(call $ray_value {args} (i32.const {offset}))");
                match pin {
                    "hit" => Val::B(format!("(i32.ne (call $raycast {args} (i32.const 48)) (i32.const 0))")),
                    "distance" => Val::F(at(0)),
                    _ => Val::V([at(4), at(8), at(12)]),
                }
            }
            "padStick" => {
                let base = match self.setting_text(node, "stick", "Left").as_str() {
                    "Left" => 0,
                    "Right" => 2,
                    _ => return Err(Problem::on(node, "the stick is Left or Right")),
                };
                if pin == "connected" {
                    Val::B("(call $pad_connected)".to_owned())
                } else {
                    Val::V([format!("(call $pad_axis (i32.const {base}))"), format!("(call $pad_axis (i32.const {}))", base + 1), lit(0.0)])
                }
            }
            "padTrigger" => match self.setting_text(node, "trigger", "Right").as_str() {
                "Left" => Val::F("(call $pad_axis (i32.const 4))".to_owned()),
                "Right" => Val::F("(call $pad_axis (i32.const 5))".to_owned()),
                _ => return Err(Problem::on(node, "the trigger is Left or Right")),
            },
            "padButton" => {
                let name = self.setting_text(node, "button", "South");
                let index = crate::PAD_BUTTONS
                    .iter()
                    .position(|b| *b == name)
                    .ok_or_else(|| Problem::on(node, format!("there is no gamepad button {name}; use {}", crate::PAD_BUTTONS.join(", "))))?;
                Val::B(format!("(call $pad_button (i32.const {index}))"))
            }
            "keyDown" => {
                let key = self.setting_text(node, "key", "Space");
                let (at, len) = self.text(key.as_bytes());
                Val::B(format!("(call $key_down (i32.const {at}) (i32.const {len}))"))
            }
            "add" | "subtract" | "multiply" | "divide" => {
                let op = match n.kind.as_str() {
                    "add" => "f32.add",
                    "subtract" => "f32.sub",
                    "multiply" => "f32.mul",
                    _ => "f32.div",
                };
                let a = self.input(node, "a")?;
                let b = self.input(node, "b")?;
                // Two whole numbers stay whole: they wrap past the 32-bit range, and division gives the
                // whole part (0 when dividing by 0).
                if let (Val::I(a), Val::I(b)) = (&a, &b) {
                    Val::I(match n.kind.as_str() {
                        "add" => format!("(i32.add {a} {b})"),
                        "subtract" => format!("(i32.sub {a} {b})"),
                        "multiply" => format!("(i32.mul {a} {b})"),
                        _ => format!("(call $idiv {a} {b})"),
                    })
                } else {
                    zip(node, a, b, |a, b| format!("({op} {a} {b})"))?
                }
            }
            "negate" => match self.input(node, "value")? {
                Val::F(v) => Val::F(format!("(f32.neg {v})")),
                Val::I(i) => Val::I(format!("(i32.sub (i32.const 0) {i})")),
                Val::V(v) => Val::V(v.map(|c| format!("(f32.neg {c})"))),
                _ => return Err(Problem::on(node, "Negate takes a number, a whole number or a vector")),
            },
            "sine" => match self.input(node, "value")? {
                Val::F(v) => Val::F(format!("(call $sin {v})")),
                Val::V(v) => Val::V(v.map(|c| format!("(call $sin {c})"))),
                Val::I(i) => Val::F(format!("(call $sin (f32.convert_i32_s {i}))")),
                Val::B(_) | Val::O(_) | Val::T(_) | Val::C(_) | Val::L(_) | Val::M(_) => return Err(Problem::on(node, "Sine needs a number or a vector")),
            },
            "power" | "squareRoot" | "vectorLength" => self.power_and_roots(node, &n.kind)?,
            "normalize" => {
                let [x, y, z] = self.input(node, "vector").and_then(|v| as_vec(node, v))?;
                Val::V([&x, &y, &z].map(|c| format!("(call $unit {c} {x} {y} {z})")))
            }
            "lerp" => {
                let a = self.input(node, "a")?;
                let b = self.input(node, "b")?;
                let factor = self.input(node, "factor").and_then(|v| as_float(node, v))?;
                zip(node, a, b, |a, b| format!("(call $lerp {a} {b} {factor})"))?
            }
            "inverseLerp" => {
                let a = self.input(node, "a").and_then(|v| as_float(node, v))?;
                let b = self.input(node, "b").and_then(|v| as_float(node, v))?;
                let value = self.input(node, "value").and_then(|v| as_float(node, v))?;
                Val::F(format!("(call $unlerp {a} {b} {value})"))
            }
            "choose" => {
                let condition = self.input(node, "condition").and_then(|v| as_bool(node, v))?;
                let a = self.input(node, "a")?;
                let b = self.input(node, "b")?;
                if let (Val::I(a), Val::I(b)) = (&a, &b) {
                    Val::I(format!("(select {a} {b} {condition})"))
                } else {
                    zip(node, a, b, |a, b| format!("(select {a} {b} {condition})"))?
                }
            }
            "greater" => {
                let a = self.input(node, "a").and_then(|v| as_float(node, v))?;
                let b = self.input(node, "b").and_then(|v| as_float(node, v))?;
                Val::B(format!("(f32.gt {a} {b})"))
            }
            "and" | "or" => {
                let a = self.input(node, "a").and_then(|v| as_bool(node, v))?;
                let b = self.input(node, "b").and_then(|v| as_bool(node, v))?;
                Val::B(format!("(i32.{} {a} {b})", n.kind))
            }
            "not" => Val::B(format!("(i32.eqz {})", self.input(node, "value").and_then(|v| as_bool(node, v))?)),
            "listItem" | "listLength" | "listFind" | "listForEach" | "mapGet" | "mapCount" | "mapForEach" | "getList" | "getMap" => {
                self.collection_value(node, &n.kind, pin)?
            }
            _ => return Err(Problem::on(node, format!("{} gives no value here", def.label))),
        })
    }
}

/// Applies `op` to two numbers, or to each part when either is a vector.
fn zip(node: u32, a: Val, b: Val, op: impl Fn(&str, &str) -> String) -> Result<Val, Problem> {
    // A whole number beside a number or a vector takes part as a number.
    let widen = |v: Val| match v {
        Val::I(i) => Val::F(format!("(f32.convert_i32_s {i})")),
        other => other,
    };
    Ok(match (widen(a), widen(b)) {
        (Val::F(a), Val::F(b)) => Val::F(op(&a, &b)),
        (Val::V(a), Val::V(b)) => Val::V([op(&a[0], &b[0]), op(&a[1], &b[1]), op(&a[2], &b[2])]),
        (Val::V(a), Val::F(b)) => Val::V(a.map(|c| op(&c, &b))),
        (Val::F(a), Val::V(b)) => Val::V(b.map(|c| op(&a, &c))),
        _ => return Err(Problem::on(node, "math needs numbers or vectors, not true/false")),
    })
}

/// What the script keeps before any code: its declared properties, in the order the panel lists
/// them, and a buffer of its own for each text variable, holding what it starts as.
fn declare_storage(c: &mut Compiler<'_>) {
    let graph = c.graph;
    for p in &graph.properties {
        c.properties.push((p.name.clone(), described_type(p.ty, p.shape, p.key), described_value(p.ty, p.shape, &p.default)));
    }
    let mut starts: std::collections::BTreeMap<String, String> = graph
        .nodes
        .iter()
        .filter(|n| variable_kinds(VariableType::Text).contains(&n.kind.as_str()))
        .map(|n| (c.setting_text(n.id, "name", "label"), String::new()))
        .collect();
    starts.extend(
        graph
            .variables
            .iter()
            .filter(|v| v.ty == VariableType::Text && v.shape.is_single())
            .map(|v| (v.name.clone(), v.start.as_str().unwrap_or_default().to_owned())),
    );
    for (name, start) in starts {
        let mut end = start.len().min(TEXT_VARIABLE_BYTES as usize);
        while !start.is_char_boundary(end) {
            end -= 1;
        }
        let bytes = &start.as_bytes()[..end];
        let (at, len) = c.reserve(bytes, TEXT_VARIABLE_BYTES - end as u32);
        c.text_vars.insert(name, (at, len));
    }
}

/// Each function as a WebAssembly function: its inputs as parameters, its outputs as results.
fn compile_functions(c: &mut Compiler<'_>, problems: &mut Vec<Problem>) -> String {
    let graph = c.graph;
    let mut functions = String::new();
    for function in &graph.functions {
        let starts: Vec<u32> = graph
            .nodes
            .iter()
            .filter(|n| n.kind == "functionStart" && n.values.get("function").and_then(serde_json::Value::as_str) == Some(function.name.as_str()))
            .map(|n| n.id)
            .collect();
        if let Some(second) = starts.get(1) {
            problems.push(Problem::on(*second, format!("{} has another Function node; a function begins in one place", function.name)));
            continue;
        }
        c.scope = Some(function.name.clone());
        let body = match starts.first() {
            Some(start) => c.chain(*start, "then", 1),
            None => Ok(String::new()),
        };
        c.scope = None;
        match body {
            Ok(body) => {
                let mut signature = String::new();
                for (k, param) in function.inputs.iter().enumerate() {
                    for (j, ty) in wasm_types(param).iter().enumerate() {
                        let _ = write!(signature, " (param $a{k}_{j} {ty})");
                    }
                }
                let results: Vec<&str> = function.outputs.iter().flat_map(|p| wasm_types(p).iter().copied()).collect();
                if !results.is_empty() {
                    let _ = write!(signature, " (result {})", results.join(" "));
                }
                // Actions that end without a Return give back zeros.
                let zeros: Vec<String> = results.iter().map(|ty| format!("({ty}.const 0)")).collect();
                let tail = if zeros.is_empty() { String::new() } else { format!("    (return {})\n", zeros.join(" ")) };
                let _ = write!(functions, "  (func $fn_{}{signature}\n{body}{tail}  )\n", function.name);
            }
            Err(p) => problems.push(p),
        }
    }
    functions
}

/// The WebAssembly value types a value of `ty` is passed as, in order.
fn wasm_types(param: &bayleaf_graph::Param) -> &'static [&'static str] {
    if !param.shape.is_single() {
        // A list or map passes as its name in the script's memory.
        return &["i64"];
    }
    match param.ty {
        VariableType::Number => &["f32"],
        VariableType::WholeNumber | VariableType::TrueFalse | VariableType::Object => &["i32"],
        VariableType::Text => &["i64"],
        VariableType::Vector | VariableType::Color => &["f32", "f32", "f32"],
    }
}

/// A value of `ty` from the expressions of its parts ([`wasm_types`]).
fn val_of(param: &bayleaf_graph::Param, mut parts: Vec<String>) -> Val {
    let mut next = || if parts.is_empty() { String::new() } else { parts.remove(0) };
    match param.shape {
        Shape::List => return Val::L(next()),
        Shape::Map => return Val::M(next()),
        Shape::Single => {}
    }
    match param.ty {
        VariableType::Number => Val::F(next()),
        VariableType::WholeNumber => Val::I(next()),
        VariableType::TrueFalse => Val::B(next()),
        VariableType::Text => Val::T(next()),
        VariableType::Object => Val::O(next()),
        VariableType::Vector | VariableType::Color => Val::V([next(), next(), next()]),
    }
}

/// The expressions of a value passed as `ty` ([`wasm_types`]).
fn parts_of(node: u32, param: &bayleaf_graph::Param, value: Val) -> Result<Vec<String>, Problem> {
    match param.shape {
        Shape::List => return Ok(vec![as_list(node, value)?]),
        Shape::Map => return Ok(vec![as_map(node, value)?]),
        Shape::Single => {}
    }
    Ok(match param.ty {
        VariableType::Number => vec![as_float(node, value)?],
        VariableType::WholeNumber => vec![as_int(node, value)?],
        VariableType::TrueFalse => vec![as_bool(node, value)?],
        VariableType::Text => vec![as_text(node, value)?],
        VariableType::Object => vec![as_object(node, value)?],
        VariableType::Vector | VariableType::Color => as_vec(node, value)?.to_vec(),
    })
}

/// What `bayleaf_describe` says, as JSON: the properties, the menu path, how the Inspector lays the
/// properties out, and the events.
fn description(graph: &Graph, properties: &[(String, String, serde_json::Value)]) -> Result<String, Problem> {
    let mut description = serde_json::json!({ "properties": properties.iter().map(|(n, ty, v)| serde_json::json!([n, v, ty])).collect::<Vec<_>>() });
    // Lists and maps of variables, with what they start with, for the host to keep.
    let kept: Vec<serde_json::Value> = graph
        .variables
        .iter()
        .filter(|v| !v.shape.is_single())
        .map(|v| serde_json::json!([v.name, described_value(v.ty, v.shape, &v.start), described_type(v.ty, v.shape, v.key)]))
        .collect();
    if !kept.is_empty() {
        description["variables"] = serde_json::Value::Array(kept);
    }
    if !graph.menu.trim().is_empty() {
        let path = crate::menu_path(&graph.menu).map_err(|problem| Problem::graph(format!("menu path: {problem}")))?;
        description["menu"] = serde_json::json!(path);
    }
    if let Some(layout) = inspector_layout(graph) {
        description["inspector"] = layout;
    }
    if !graph.events.is_empty() {
        description["events"] = graph.events.iter().map(|e| serde_json::json!({ "name": e.name, "params": bayleaf_graph::param_pins(&e.params) })).collect();
    }
    Ok(description.to_string())
}

/// An event node's chain. An On Event's compiles in its own scope, where its values are read, once
/// it names an event the script declares.
fn event_chain(c: &mut Compiler<'_>, graph: &Graph, event: &bayleaf_graph::Node) -> Result<String, Problem> {
    match event.kind.as_str() {
        "eventStart" if graph.event(&c.setting_text(event.id, "event", "")).is_none() => {
            return Err(Problem::on(event.id, "this On Event names no event this script declares; add it under Events"));
        }
        "componentEventStart" if c.setting_text(event.id, "script", "").is_empty() || c.setting_text(event.id, "event", "").is_empty() => {
            return Err(Problem::on(event.id, "choose the event: a script's, from the list"));
        }
        "eventStart" | "componentEventStart" => {}
        _ => return c.chain(event.id, "then", 1),
    }
    c.scope = Some(event_scope(event.id));
    let chained = c.chain(event.id, "then", 1);
    c.scope = None;
    let mut kept = String::new();
    for (k, param) in event_params_of(graph, event.id).iter().enumerate().filter(|(_, p)| !p.shape.is_single()) {
        let (at, len) = c.text(event_list_name(event.id, &param.name).as_bytes());
        let map = i32::from(param.shape == Shape::Map);
        let _ = writeln!(kept, "        (drop (call $event_keep (i32.const {k}) (i32.const {map}) (i32.const {at}) (i32.const {len})))");
    }
    chained.map(|chain| if chain.is_empty() { chain } else { kept + &chain })
}

/// The values an On Event or Trigger Event node keeps (`params`, each `name:type`, a list's
/// `name:list:type`), in order.
fn event_params_of(graph: &Graph, node: u32) -> Vec<bayleaf_graph::Param> {
    let listed = graph.value(node, "params").and_then(serde_json::Value::as_array);
    listed
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let (name, code) = entry.as_str()?.split_once(':')?;
            let (ty, shape, key) = VariableType::from_shaped_code(code)?;
            Some(bayleaf_graph::Param { name: name.to_owned(), ty, shape, key })
        })
        .collect()
}

/// The name a list or map an event brings is kept under while its On Event runs: never a variable's
/// or property's (they have no spaces).
fn event_list_name(node: u32, param: &str) -> String {
    format!("event {node} {param}")
}

/// The scope an On Event's chain compiles in, which its values are read in.
fn event_scope(node: u32) -> String {
    format!("on event {node}")
}

/// Node kinds that start a chain on their own.
fn is_event(kind: &str) -> bool {
    matches!(kind, "init" | "start" | "update" | "message" | "keyPressed" | "actionPressed" | "timer" | "interfacePressed" | "interfaceChanged")
        || contact_message(kind).is_some()
        || kind == "eventStart"
        || kind == "componentEventStart"
}

fn as_float(node: u32, value: Val) -> Result<String, Problem> {
    match value {
        Val::F(f) => Ok(f),
        Val::I(i) => Ok(format!("(f32.convert_i32_s {i})")),
        Val::V(_) => Err(Problem::on(node, "this input takes a number but gets a vector; use Split Vector")),
        Val::B(_) => Err(Problem::on(node, "this input takes a number but gets true/false; use Choose")),
        Val::O(_) => Err(Problem::on(node, "this input takes a number but gets an object; use Position Of")),
        Val::C(_) => Err(Problem::on(node, "this input takes a number but gets a component")),
        Val::L(_) | Val::M(_) => Err(Problem::on(node, "this input takes a number but gets a list or map; use List Item or Map Value")),
        Val::T(_) => Err(Problem::on(node, "this input takes a number but gets text; use Text to Number")),
    }
}

fn as_vec(node: u32, value: Val) -> Result<[String; 3], Problem> {
    match value {
        Val::V(v) => Ok(v),
        Val::I(i) => {
            let f = format!("(f32.convert_i32_s {i})");
            Ok([f.clone(), f.clone(), f])
        }
        Val::F(f) => Ok([f.clone(), f.clone(), f]),
        Val::B(_) => Err(Problem::on(node, "this input takes a vector but gets true/false")),
        Val::O(_) => Err(Problem::on(node, "this input takes a vector but gets an object; use Position Of")),
        Val::C(_) => Err(Problem::on(node, "this input takes a vector but gets a component")),
        Val::L(_) | Val::M(_) => Err(Problem::on(node, "this input takes a vector but gets a list or map")),
        Val::T(_) => Err(Problem::on(node, "this input takes a vector but gets text")),
    }
}

fn as_int(node: u32, value: Val) -> Result<String, Problem> {
    match value {
        Val::I(i) => Ok(i),
        Val::F(_) => Err(Problem::on(node, "this input takes a whole number but gets a number; use To Whole Number")),
        _ => Err(Problem::on(node, "this input takes a whole number")),
    }
}

/// A number or a whole number as text: a whole number keeps every digit.
fn number_text(node: u32, value: Val) -> Result<String, Problem> {
    match value {
        Val::I(i) => Ok(format!("(call $int_text {i})")),
        other => Ok(format!("(call $num_text {})", as_float(node, other)?)),
    }
}

fn as_text(node: u32, value: Val) -> Result<String, Problem> {
    match value {
        Val::T(text) => Ok(text),
        _ => Err(Problem::on(node, "this input takes text; use To Text or Name Of")),
    }
}

/// A list's name, from a list pin.
fn as_list(node: u32, value: Val) -> Result<String, Problem> {
    match value {
        Val::L(name) => Ok(name),
        Val::M(_) => Err(Problem::on(node, "this input takes a list but gets a map")),
        _ => Err(Problem::on(node, "this input takes a list: link a Get List, a list input or a list an event brings")),
    }
}

/// A map's name, from a map pin.
fn as_map(node: u32, value: Val) -> Result<String, Problem> {
    match value {
        Val::M(name) => Ok(name),
        Val::L(_) => Err(Problem::on(node, "this input takes a map but gets a list")),
        _ => Err(Problem::on(node, "this input takes a map: link a Get Map, a map input or a map an event brings")),
    }
}

fn as_component(node: u32, value: Val) -> Result<String, Problem> {
    match value {
        Val::C(component) => Ok(component),
        _ => Err(Problem::on(node, "this input takes a component: a Get Component that names a script, or This Component")),
    }
}

/// A component value from its object's handle and its script's place plus one.
fn component_value(handle: &str, script: &str) -> String {
    format!("(i64.or (i64.shl (i64.extend_i32_u {handle}) (i64.const 32)) (i64.extend_i32_u {script}))")
}

/// A component's object handle.
fn component_handle(component: &str) -> String {
    format!("(i32.wrap_i64 (i64.shr_u {component} (i64.const 32)))")
}

/// A component's script place plus one (0 for none).
fn component_script(component: &str) -> String {
    format!("(i32.wrap_i64 {component})")
}

fn as_object(node: u32, value: Val) -> Result<String, Problem> {
    match value {
        Val::O(handle) => Ok(handle),
        _ => Err(Problem::on(node, "this input takes an object; use a Find node or This Object")),
    }
}

fn as_bool(node: u32, value: Val) -> Result<String, Problem> {
    match value {
        Val::B(b) => Ok(b),
        _ => Err(Problem::on(node, "this input takes true/false; use Greater Than")),
    }
}

/// Imports, the transform helpers, sine and number formatting shared by every compiled script.
const PRELUDE: &str = r#"(module
  (import "bayleaf" "log" (func $log (param i32 i32)))
  (import "bayleaf" "transform_get" (func $transform_get (param i32)))
  (import "bayleaf" "transform_set" (func $transform_set (param i32)))
  (import "bayleaf" "property" (func $property (param i32 i32) (result f32)))
  (import "bayleaf" "property_whole" (func $property_whole (param i32 i32) (result i32)))
  (import "bayleaf" "property_vector" (func $property_vector (param i32 i32 i32) (result i32)))
  (import "bayleaf" "property_text" (func $property_text (param i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "property_object" (func $property_object (param i32 i32) (result i32)))
  (import "bayleaf" "property_set" (func $property_set (param i32 i32 f32 f32 f32) (result i32)))
  (import "bayleaf" "property_set_text" (func $property_set_text (param i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "time" (func $time (result f64)))
  (import "bayleaf" "math_pow" (func $math_pow (param f32 f32) (result f32)))
  (import "bayleaf" "key_down" (func $key_down (param i32 i32) (result i32)))
  (import "bayleaf" "pointer" (func $pointer (param i32) (result i32)))
  (import "bayleaf" "pointer_over_interface" (func $pointer_over_interface (result i32)))
  (import "bayleaf" "mouse_down" (func $mouse_down (param i32) (result i32)))
  (import "bayleaf" "self_handle" (func $self_handle (result i32)))
  (import "bayleaf" "find" (func $find (param i32 i32) (result i32)))
  (import "bayleaf" "find_with" (func $find_with (param i32 i32 i32) (result i32)))
  (import "bayleaf" "count_with" (func $count_with (param i32 i32) (result i32)))
  (import "bayleaf" "component_get" (func $component_get (param i32 i32 i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "component_set" (func $component_set (param i32 i32 i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "name_of" (func $name_of (param i32 i32 i32) (result i32)))
  (import "bayleaf" "scene_load" (func $scene_load (param i32 i32 i32) (result i32)))
  (import "bayleaf" "sound_play" (func $sound_play (param i32) (result i32)))
  (import "bayleaf" "sound_stop" (func $sound_stop (param i32) (result i32)))
  (import "bayleaf" "sound_play_at" (func $sound_play_at (param i32 i32 f32 f32 f32 f32) (result i32)))
  (import "bayleaf" "scene_unload" (func $scene_unload (param i32 i32) (result i32)))
  (import "bayleaf" "object_ref" (func $object_ref (param i32) (result i32)))
  (import "bayleaf" "object_of" (func $object_of (param i32) (result i32)))
  (import "bayleaf" "transform_get_of" (func $transform_get_of (param i32 i32)))
  (import "bayleaf" "transform_set_of" (func $transform_set_of (param i32 i32)))
  (import "bayleaf" "position_get_of" (func $position_get_of (param i32 i32)))
  (import "bayleaf" "position_set_of" (func $position_set_of (param i32 i32)))
  (import "bayleaf" "spawn" (func $spawn (param i32 i32 f32 f32 f32) (result i32)))
  (import "bayleaf" "destroy" (func $destroy (param i32) (result i32)))
  (import "bayleaf" "send" (func $send (param i32 i32 i32 f32) (result i32)))
  (import "bayleaf" "message_is" (func $message_is (param i32 i32) (result i32)))
  (import "bayleaf" "event_send" (func $event_send (param i32 i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "event_number" (func $event_number (param i32) (result f32)))
  (import "bayleaf" "event_whole" (func $event_whole (param i32) (result i32)))
  (import "bayleaf" "event_flag" (func $event_flag (param i32) (result i32)))
  (import "bayleaf" "event_vector" (func $event_vector (param i32 i32) (result f32)))
  (import "bayleaf" "event_object" (func $event_object (param i32) (result i32)))
  (import "bayleaf" "event_text" (func $event_text (param i32 i32 i32) (result i32)))
  (import "bayleaf" "script_component" (func $script_component (param i32 i32 i32) (result i32)))
  (import "bayleaf" "self_component" (func $self_component (result i32)))
  (import "bayleaf" "event_send_to" (func $event_send_to (param i32 i32 i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "event_subscribe" (func $event_subscribe (param i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "event_unsubscribe" (func $event_unsubscribe (param i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "event_source" (func $event_source (result i32)))
  (import "bayleaf" "event_source_component" (func $event_source_component (result i32)))
  (import "bayleaf" "state_get" (func $state_get (param i32 i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "state_set" (func $state_set (param i32 i32 i32 i32 f64) (result i32)))
  (import "bayleaf" "game_end" (func $game_end (param i32) (result i32)))
  (import "bayleaf" "action" (func $action (param i32 i32) (result f32)))
  (import "bayleaf" "input_map_set" (func $input_map_set (param i32 i32 i32) (result i32)))
  (import "bayleaf" "list_put" (func $list_put (param i32 i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "list_remove" (func $list_remove (param i32 i32 i32) (result i32)))
  (import "bayleaf" "list_clear" (func $list_clear (param i32 i32)))
  (import "bayleaf" "list_len" (func $list_len (param i32 i32) (result i32)))
  (import "bayleaf" "list_get" (func $list_get (param i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "list_text" (func $list_text (param i32 i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "list_find" (func $list_find (param i32 i32 i32) (result i32)))
  (import "bayleaf" "map_put" (func $map_put (param i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "map_remove" (func $map_remove (param i32 i32 i32) (result i32)))
  (import "bayleaf" "map_clear" (func $map_clear (param i32 i32)))
  (import "bayleaf" "map_count" (func $map_count (param i32 i32) (result i32)))
  (import "bayleaf" "map_get" (func $map_get (param i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "map_text" (func $map_text (param i32 i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "map_next" (func $map_next (param i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "map_next_text" (func $map_next_text (param i32 i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "event_keep" (func $event_keep (param i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "particles_play" (func $particles_play (param i32) (result i32)))
  (import "bayleaf" "particles_stop" (func $particles_stop (param i32) (result i32)))
  (import "bayleaf" "particles_burst" (func $particles_burst (param i32 i32) (result i32)))
  (import "bayleaf" "ui_set_text" (func $ui_set_text (param i32 i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "ui_set_value" (func $ui_set_value (param i32 i32 i32 f64) (result i32)))
  (import "bayleaf" "ui_show" (func $ui_show (param i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "ui_add" (func $ui_add (param i32 i32 i32 i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "ui_remove" (func $ui_remove (param i32 i32 i32) (result i32)))
  (import "bayleaf" "ui_value" (func $ui_value (param i32 i32 i32) (result f64)))
  (import "bayleaf" "ui_text" (func $ui_text_get (param i32 i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "character_walk" (func $character_walk (param i32 f32 f32 f32) (result i32)))
  (import "bayleaf" "character_jump" (func $character_jump (param i32 f32) (result i32)))
  (import "bayleaf" "character_grounded" (func $character_grounded (param i32) (result i32)))
  (import "bayleaf" "apply_impulse" (func $apply_impulse (param i32 f32 f32 f32) (result i32)))
  (import "bayleaf" "set_velocity" (func $set_velocity (param i32 f32 f32 f32) (result i32)))
  (import "bayleaf" "velocity_of" (func $velocity_of (param i32 i32) (result i32)))
  (import "bayleaf" "raycast" (func $raycast (param f32 f32 f32 f32 f32 f32 f32 i32) (result i32)))
  (import "bayleaf" "pad_connected" (func $pad_connected (result i32)))
  (import "bayleaf" "pad_axis" (func $pad_axis (param i32) (result f32)))
  (import "bayleaf" "pad_button" (func $pad_button (param i32) (result i32)))
  (global $dt (mut f32) (f32.const 0))
  (global $message (mut f32) (f32.const 0))
  (global $heap (mut i32) (i32.const 0))
  (func (export "bayleaf_abi") (result i32) (i32.const 1))
  ;; Transform value i (0..8: position, rotation, scale).
  (func $t (param $i i32) (result f32)
    (call $transform_get (i32.const 0))
    (f32.load (i32.shl (local.get $i) (i32.const 2))))
  ;; Pointer x, y (at bytes 40..48), and whether it is over the view.
  (func $pointer_inside (result i32) (call $pointer (i32.const 40)))
  (func $pointer_x (result f32) (drop (call $pointer (i32.const 40))) (f32.load (i32.const 40)))
  (func $pointer_y (result f32) (drop (call $pointer (i32.const 40))) (f32.load (i32.const 44)))
  ;; Transform value i of the object with handle h (0: none, which stops the script with a reason).
  (func $t_of (param $h i32) (param $i i32) (result f32)
    (call $transform_get_of (local.get $h) (i32.const 0))
    (f32.load (i32.shl (local.get $i) (i32.const 2))))
  ;; This object's velocity part at $offset (0, 4 or 8; bytes 64..76).
  (func $velocity_value (param $offset i32) (result f32)
    (drop (call $velocity_of (call $self_handle) (i32.const 64)))
    (f32.load (i32.add (i32.const 64) (local.get $offset))))
  ;; A handle, or -1 (no object) for 0, so a player name that finds nothing is no player.
  (func $or_none (param $h i32) (result i32)
    (select (local.get $h) (i32.const -1) (local.get $h)))
  ;; A state value (0 when not set), read through bytes 128..136.
  (func $state_f64 (param $scope i32) (param $handle i32) (param $at i32) (param $len i32) (result f64)
    (drop (call $state_get (local.get $scope) (local.get $handle) (local.get $at) (local.get $len) (i32.const 128)))
    (f64.load (i32.const 128)))
  ;; Casts a ray and returns the result value at $offset (0 distance, 4/8/12 hit point x/y/z; bytes 48..64).
  (func $ray_value (param $ox f32) (param $oy f32) (param $oz f32) (param $dx f32) (param $dy f32) (param $dz f32) (param $max f32) (param $offset i32) (result f32)
    (drop (call $raycast (local.get $ox) (local.get $oy) (local.get $oz) (local.get $dx) (local.get $dy) (local.get $dz) (local.get $max) (i32.const 48)))
    (f32.load (i32.add (i32.const 48) (local.get $offset))))
  ;; Part $offset (0, 4 or 8) of a component field of the object with handle h, read through bytes 160..172.
  (func $field_value (param $h i32) (param $c i32) (param $cl i32) (param $f i32) (param $fl i32) (param $offset i32) (result f32)
    (drop (call $component_get (local.get $h) (local.get $c) (local.get $cl) (local.get $f) (local.get $fl) (i32.const 160)))
    (f32.load (i32.add (i32.const 160) (local.get $offset))))
  ;; Asks to set a component field of the object with handle h to x, y, z (through bytes 176..188).
  (func $set_field (param $h i32) (param $c i32) (param $cl i32) (param $f i32) (param $fl i32) (param $x f32) (param $y f32) (param $z f32) (result i32)
    (f32.store (i32.const 176) (local.get $x))
    (f32.store (i32.const 180) (local.get $y))
    (f32.store (i32.const 184) (local.get $z))
    (call $component_set (local.get $h) (local.get $c) (local.get $cl) (local.get $f) (local.get $fl) (i32.const 176)))
  ;; Text is (address << 32) | length in bytes, in this module's memory. Text made while a call
  ;; runs goes on a heap that each call starts empty; the memory grows when it needs to.
  (func $alloc (param $n i32) (result i32) (local $p i32) (local $end i32)
    (local.set $p (global.get $heap))
    (local.set $end (i32.add (local.get $p) (local.get $n)))
    (if (i32.gt_u (local.get $end) (i32.shl (memory.size) (i32.const 16)))
      (then
        (if (i32.lt_s (memory.grow (i32.add (i32.shr_u (i32.sub (local.get $end) (i32.shl (memory.size) (i32.const 16))) (i32.const 16)) (i32.const 1))) (i32.const 0))
          (then unreachable))))
    (global.set $heap (local.get $end))
    (local.get $p))
  (func $tp (param $t i64) (result i32) (i32.wrap_i64 (i64.shr_u (local.get $t) (i64.const 32))))
  (func $tl (param $t i64) (result i32) (i32.wrap_i64 (local.get $t)))
  (func $mk (param $p i32) (param $l i32) (result i64)
    (i64.or (i64.shl (i64.extend_i32_u (local.get $p)) (i64.const 32)) (i64.extend_i32_u (local.get $l))))
  (func $log_text (param $t i64) (call $log (call $tp (local.get $t)) (call $tl (local.get $t))))
  (func $find_text (param $t i64) (result i32) (call $find (call $tp (local.get $t)) (call $tl (local.get $t))))
  (func $ui_text (param $h i32) (param $np i32) (param $nl i32) (param $t i64)
    (drop (call $ui_set_text (local.get $h) (local.get $np) (local.get $nl) (call $tp (local.get $t)) (call $tl (local.get $t)))))
  (func $ui_add_named (param $h i32) (param $pp i32) (param $pl i32) (param $wp i32) (param $wl i32) (param $t i64)
    (drop (call $ui_add (local.get $h) (local.get $pp) (local.get $pl) (local.get $wp) (local.get $wl) (call $tp (local.get $t)) (call $tl (local.get $t)))))
  ;; An element's value as a number, 0 when the interface has no such element.
  (func $ui_value_or_zero (param $h i32) (param $np i32) (param $nl i32) (result f32) (local $v f64)
    (local.set $v (call $ui_value (local.get $h) (local.get $np) (local.get $nl)))
    (if (result f32) (f64.eq (local.get $v) (local.get $v)) (then (f32.demote_f64 (local.get $v))) (else (f32.const 0))))
  ;; An element's text, empty when the interface has no such element.
  (func $ui_text_of (param $h i32) (param $np i32) (param $nl i32) (result i64) (local $n i32) (local $p i32)
    (local.set $n (call $ui_text_get (local.get $h) (local.get $np) (local.get $nl) (i32.const 0) (i32.const 0)))
    (if (i32.lt_s (local.get $n) (i32.const 0)) (then (local.set $n (i32.const 0))))
    (local.set $p (call $alloc (local.get $n)))
    (drop (call $ui_text_get (local.get $h) (local.get $np) (local.get $nl) (local.get $p) (local.get $n)))
    (call $mk (local.get $p) (local.get $n)))
  (func $ui_remove_named (param $h i32) (param $t i64)
    (drop (call $ui_remove (local.get $h) (call $tp (local.get $t)) (call $tl (local.get $t)))))
  (func $join (param $a i64) (param $b i64) (result i64) (local $p i32)
    (local.set $p (call $alloc (i32.add (call $tl (local.get $a)) (call $tl (local.get $b)))))
    (memory.copy (local.get $p) (call $tp (local.get $a)) (call $tl (local.get $a)))
    (memory.copy (i32.add (local.get $p) (call $tl (local.get $a))) (call $tp (local.get $b)) (call $tl (local.get $b)))
    (call $mk (local.get $p) (i32.add (call $tl (local.get $a)) (call $tl (local.get $b)))))
  ;; Whether the bytes of $b are at byte $at of $a.
  (func $at (param $a i64) (param $b i64) (param $at i32) (result i32) (local $i i32)
    (if (i32.or (i32.lt_s (local.get $at) (i32.const 0)) (i32.gt_u (i32.add (local.get $at) (call $tl (local.get $b))) (call $tl (local.get $a))))
      (then (return (i32.const 0))))
    (block $done (loop $next
      (br_if $done (i32.ge_u (local.get $i) (call $tl (local.get $b))))
      (if (i32.ne (i32.load8_u (i32.add (i32.add (call $tp (local.get $a)) (local.get $at)) (local.get $i))) (i32.load8_u (i32.add (call $tp (local.get $b)) (local.get $i))))
        (then (return (i32.const 0))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (i32.const 1))
  (func $same (param $a i64) (param $b i64) (result i32)
    (if (result i32) (i32.eq (call $tl (local.get $a)) (call $tl (local.get $b)))
      (then (call $at (local.get $a) (local.get $b) (i32.const 0)))
      (else (i32.const 0))))
  (func $ends (param $a i64) (param $b i64) (result i32)
    (call $at (local.get $a) (local.get $b) (i32.sub (call $tl (local.get $a)) (call $tl (local.get $b)))))
  ;; The byte where $b first appears in $a, from byte $from, or -1.
  (func $find_bytes (param $a i64) (param $b i64) (param $from i32) (result i32)
    (block $none (loop $next
      (br_if $none (i32.gt_s (i32.add (local.get $from) (call $tl (local.get $b))) (call $tl (local.get $a))))
      (if (call $at (local.get $a) (local.get $b) (local.get $from)) (then (return (local.get $from))))
      (local.set $from (i32.add (local.get $from) (i32.const 1)))
      (br $next)))
    (i32.const -1))
  ;; Characters in the first $n bytes of $t: bytes that do not continue a UTF-8 character.
  (func $chars (param $t i64) (param $n i32) (result i32) (local $i i32) (local $c i32)
    (block $done (loop $next
      (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
      (if (i32.ne (i32.and (i32.load8_u (i32.add (call $tp (local.get $t)) (local.get $i))) (i32.const 192)) (i32.const 128))
        (then (local.set $c (i32.add (local.get $c) (i32.const 1)))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (local.get $c))
  (func $length (param $t i64) (result i32) (call $chars (local.get $t) (call $tl (local.get $t))))
  (func $find_chars (param $a i64) (param $b i64) (result i32) (local $at i32)
    (local.set $at (call $find_bytes (local.get $a) (local.get $b) (i32.const 0)))
    (if (result i32) (i32.lt_s (local.get $at) (i32.const 0))
      (then (i32.const -1))
      (else (call $chars (local.get $a) (local.get $at)))))
  ;; The byte where character $n of $t starts (its length past the end).
  (func $offset (param $t i64) (param $n i32) (result i32) (local $i i32) (local $c i32)
    (block $done (loop $next
      (br_if $done (i32.ge_u (local.get $i) (call $tl (local.get $t))))
      (if (i32.ne (i32.and (i32.load8_u (i32.add (call $tp (local.get $t)) (local.get $i))) (i32.const 192)) (i32.const 128))
        (then
          (if (i32.eq (local.get $c) (local.get $n)) (then (return (local.get $i))))
          (local.set $c (i32.add (local.get $c) (i32.const 1)))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (call $tl (local.get $t)))
  ;; A count or position from a number: whole, from 0 to 2^30.
  (func $index (param $v f32) (result i32) (local $i i32)
    (local.set $i (i32.trunc_sat_f32_s (local.get $v)))
    (select (i32.const 0) (select (i32.const 1073741824) (local.get $i) (i32.gt_s (local.get $i) (i32.const 1073741824))) (i32.lt_s (local.get $i) (i32.const 0))))
  (func $part (param $t i64) (param $start i32) (param $count i32) (result i64) (local $a i32) (local $b i32)
    (local.set $a (call $offset (local.get $t) (local.get $start)))
    (local.set $b (call $offset (local.get $t) (i32.add (local.get $start) (local.get $count))))
    (call $mk (i32.add (call $tp (local.get $t)) (local.get $a)) (i32.sub (local.get $b) (local.get $a))))
  ;; A to Z (when $upper) or a to z.
  (func $case (param $t i64) (param $upper i32) (result i64) (local $p i32) (local $i i32) (local $b i32)
    (local.set $p (call $alloc (call $tl (local.get $t))))
    (block $done (loop $next
      (br_if $done (i32.ge_u (local.get $i) (call $tl (local.get $t))))
      (local.set $b (i32.load8_u (i32.add (call $tp (local.get $t)) (local.get $i))))
      (if (local.get $upper)
        (then (if (i32.le_u (i32.sub (local.get $b) (i32.const 97)) (i32.const 25)) (then (local.set $b (i32.sub (local.get $b) (i32.const 32))))))
        (else (if (i32.le_u (i32.sub (local.get $b) (i32.const 65)) (i32.const 25)) (then (local.set $b (i32.add (local.get $b) (i32.const 32)))))))
      (i32.store8 (i32.add (local.get $p) (local.get $i)) (local.get $b))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (call $mk (local.get $p) (call $tl (local.get $t))))
  (func $space (param $b i32) (result i32)
    (i32.or (i32.eq (local.get $b) (i32.const 32)) (i32.le_u (i32.sub (local.get $b) (i32.const 9)) (i32.const 4))))
  (func $trim (param $t i64) (result i64) (local $p i32) (local $l i32)
    (local.set $p (call $tp (local.get $t)))
    (local.set $l (call $tl (local.get $t)))
    (block $front (loop $next
      (br_if $front (i32.eqz (local.get $l)))
      (br_if $front (i32.eqz (call $space (i32.load8_u (local.get $p)))))
      (local.set $p (i32.add (local.get $p) (i32.const 1)))
      (local.set $l (i32.sub (local.get $l) (i32.const 1)))
      (br $next)))
    (block $back (loop $next
      (br_if $back (i32.eqz (local.get $l)))
      (br_if $back (i32.eqz (call $space (i32.load8_u (i32.sub (i32.add (local.get $p) (local.get $l)) (i32.const 1))))))
      (local.set $l (i32.sub (local.get $l) (i32.const 1)))
      (br $next)))
    (call $mk (local.get $p) (local.get $l)))
  (func $replace (param $t i64) (param $find i64) (param $with i64) (result i64) (local $count i32) (local $i i32) (local $at i32) (local $p i32) (local $w i32) (local $size i64)
    (if (i32.eqz (call $tl (local.get $find))) (then (return (local.get $t))))
    ;; Count the matches first, so the new text is made at its size.
    (block $counted (loop $next
      (local.set $at (call $find_bytes (local.get $t) (local.get $find) (local.get $i)))
      (br_if $counted (i32.lt_s (local.get $at) (i32.const 0)))
      (local.set $count (i32.add (local.get $count) (i32.const 1)))
      (local.set $i (i32.add (local.get $at) (call $tl (local.get $find))))
      (br $next)))
    (if (i32.eqz (local.get $count)) (then (return (local.get $t))))
    (local.set $size
      (i64.add (i64.extend_i32_u (call $tl (local.get $t)))
        (i64.mul (i64.extend_i32_u (local.get $count)) (i64.sub (i64.extend_i32_u (call $tl (local.get $with))) (i64.extend_i32_u (call $tl (local.get $find)))))))
    (if (i64.gt_s (local.get $size) (i64.const 1073741824)) (then unreachable))
    (local.set $p (call $alloc (i32.wrap_i64 (local.get $size))))
    (local.set $w (local.get $p))
    (local.set $i (i32.const 0))
    (block $done (loop $next
      (local.set $at (call $find_bytes (local.get $t) (local.get $find) (local.get $i)))
      (if (i32.lt_s (local.get $at) (i32.const 0)) (then (local.set $at (call $tl (local.get $t)))))
      (memory.copy (local.get $w) (i32.add (call $tp (local.get $t)) (local.get $i)) (i32.sub (local.get $at) (local.get $i)))
      (local.set $w (i32.add (local.get $w) (i32.sub (local.get $at) (local.get $i))))
      (br_if $done (i32.ge_u (local.get $at) (call $tl (local.get $t))))
      (memory.copy (local.get $w) (call $tp (local.get $with)) (call $tl (local.get $with)))
      (local.set $w (i32.add (local.get $w) (call $tl (local.get $with))))
      (local.set $i (i32.add (local.get $at) (call $tl (local.get $find))))
      (br $next)))
    (call $mk (local.get $p) (i32.sub (local.get $w) (local.get $p))))
  ;; Copies $t into the $cap bytes at $dst, cut at a character when it is longer; returns the length kept.
  (func $keep (param $dst i32) (param $cap i32) (param $t i64) (result i32) (local $n i32)
    (local.set $n (call $tl (local.get $t)))
    (if (i32.gt_u (local.get $n) (local.get $cap))
      (then
        (local.set $n (local.get $cap))
        (block $cut (loop $back
          (br_if $cut (i32.eqz (local.get $n)))
          (br_if $cut (i32.ne (i32.and (i32.load8_u (i32.add (call $tp (local.get $t)) (local.get $n))) (i32.const 192)) (i32.const 128)))
          (local.set $n (i32.sub (local.get $n) (i32.const 1)))
          (br $back)))))
    (memory.copy (local.get $dst) (call $tp (local.get $t)) (local.get $n))
    (local.get $n))
  ;; Whole-number division and remainder that never trap: 0 for a divisor of 0; the one overflow
  ;; (-2147483648 / -1) wraps.
  (func $idiv (param $a i32) (param $b i32) (result i32)
    (if (result i32) (i32.eqz (local.get $b))
      (then (i32.const 0))
      (else
        (if (result i32) (i32.and (i32.eq (local.get $a) (i32.const -2147483648)) (i32.eq (local.get $b) (i32.const -1)))
          (then (local.get $a))
          (else (i32.div_s (local.get $a) (local.get $b)))))))
  (func $irem (param $a i32) (param $b i32) (result i32)
    (if (result i32) (i32.eqz (local.get $b)) (then (i32.const 0)) (else (i32.rem_s (local.get $a) (local.get $b)))))
  ;; Rounds to the nearest whole number, halves away from zero.
  (func $round (param $v f32) (result f32)
    (f32.trunc (f32.add (local.get $v) (f32.copysign (f32.const 0.5) (local.get $v)))))
  ;; A whole number as text, every digit.
  (func $int_text (param $v i32) (result i64) (local $p i32) (local $n i64) (local $i i32) (local $neg i32)
    (local.set $p (call $alloc (i32.const 12)))
    (local.set $n (i64.extend_i32_s (local.get $v)))
    (local.set $neg (i64.lt_s (local.get $n) (i64.const 0)))
    (if (local.get $neg) (then (local.set $n (i64.sub (i64.const 0) (local.get $n)))))
    (local.set $i (i32.const 12))
    (loop $digits
      (local.set $i (i32.sub (local.get $i) (i32.const 1)))
      (i32.store8 (i32.add (local.get $p) (local.get $i)) (i32.add (i32.const 48) (i32.wrap_i64 (i64.rem_u (local.get $n) (i64.const 10)))))
      (local.set $n (i64.div_u (local.get $n) (i64.const 10)))
      (br_if $digits (i64.ne (local.get $n) (i64.const 0))))
    (if (local.get $neg)
      (then
        (local.set $i (i32.sub (local.get $i) (i32.const 1)))
        (i32.store8 (i32.add (local.get $p) (local.get $i)) (i32.const 45))))
    (call $mk (i32.add (local.get $p) (local.get $i)) (i32.sub (i32.const 12) (local.get $i))))
  (func $num_text (param $v f32) (result i64) (local $p i32)
    (local.set $p (call $alloc (i32.const 32)))
    (call $mk (local.get $p) (call $fmt (local.get $v) (local.get $p))))
  ;; The number $t spells (spaces around it are fine), or NaN when it spells none.
  (func $parse (param $t i64) (result f32) (local $p i32) (local $e i32) (local $b i32) (local $v f64) (local $scale f64) (local $neg i32) (local $digits i32) (local $dot i32)
    (local.set $t (call $trim (local.get $t)))
    (local.set $p (call $tp (local.get $t)))
    (local.set $e (i32.add (local.get $p) (call $tl (local.get $t))))
    (local.set $scale (f64.const 1))
    (if (i32.lt_u (local.get $p) (local.get $e))
      (then
        (local.set $b (i32.load8_u (local.get $p)))
        (if (i32.or (i32.eq (local.get $b) (i32.const 45)) (i32.eq (local.get $b) (i32.const 43)))
          (then
            (local.set $neg (i32.eq (local.get $b) (i32.const 45)))
            (local.set $p (i32.add (local.get $p) (i32.const 1)))))))
    (block $done (loop $next
      (br_if $done (i32.ge_u (local.get $p) (local.get $e)))
      (local.set $b (i32.load8_u (local.get $p)))
      (if (i32.eq (local.get $b) (i32.const 46))
        (then
          (if (local.get $dot) (then (return (f32.const nan))))
          (local.set $dot (i32.const 1)))
        (else
          (if (i32.gt_u (i32.sub (local.get $b) (i32.const 48)) (i32.const 9)) (then (return (f32.const nan))))
          (local.set $digits (i32.const 1))
          (local.set $v (f64.add (f64.mul (local.get $v) (f64.const 10)) (f64.convert_i32_u (i32.sub (local.get $b) (i32.const 48)))))
          (if (local.get $dot) (then (local.set $scale (f64.mul (local.get $scale) (f64.const 10)))))))
      (local.set $p (i32.add (local.get $p) (i32.const 1)))
      (br $next)))
    (if (i32.eqz (local.get $digits)) (then (return (f32.const nan))))
    (local.set $v (f64.div (local.get $v) (local.get $scale)))
    (f32.demote_f64 (select (f64.neg (local.get $v)) (local.get $v) (local.get $neg))))
  (func $is_number (param $t i64) (result i32) (local $v f32)
    (local.set $v (call $parse (local.get $t)))
    (f32.eq (local.get $v) (local.get $v)))
  (func $number_or_zero (param $t i64) (result f32) (local $v f32)
    (local.set $v (call $parse (local.get $t)))
    (select (local.get $v) (f32.const 0) (f32.eq (local.get $v) (local.get $v))))
  ;; Part $i (0, 1 or 2) of a vector or colour property, read through bytes 192..204.
  (func $property_part (param $a i32) (param $l i32) (param $i i32) (result f32)
    (drop (call $property_vector (local.get $a) (local.get $l) (i32.const 192)))
    (f32.load (i32.add (i32.const 192) (i32.shl (local.get $i) (i32.const 2)))))
  ;; Sets a text (or object name) property to text $t.
  (func $property_set_string (param $a i32) (param $l i32) (param $t i64) (result i32)
    (call $property_set_text (local.get $a) (local.get $l) (call $tp (local.get $t)) (call $tl (local.get $t))))
  ;; A text property, copied onto the heap.
  (func $property_string (param $a i32) (param $l i32) (result i64) (local $n i32) (local $p i32)
    (local.set $n (call $property_text (local.get $a) (local.get $l) (i32.const 0) (i32.const 0)))
    (local.set $p (call $alloc (local.get $n)))
    (drop (call $property_text (local.get $a) (local.get $l) (local.get $p) (local.get $n)))
    (call $mk (local.get $p) (local.get $n)))
  ;; The name of the object with handle $h.
  ;; The script named by $p, $l on the object with handle $h, as a component (its script's place 0: none).
  (func $component_of (param $h i32) (param $p i32) (param $l i32) (result i64)
    (i64.or (i64.shl (i64.extend_i32_u (local.get $h)) (i64.const 32))
      (i64.extend_i32_u (call $script_component (local.get $h) (local.get $p) (local.get $l)))))
  ;; Text value $k of the event being delivered.
  (func $event_string (param $k i32) (result i64) (local $n i32) (local $p i32)
    (local.set $n (call $event_text (local.get $k) (i32.const 0) (i32.const 0)))
    (local.set $p (call $alloc (local.get $n)))
    (drop (call $event_text (local.get $k) (local.get $p) (local.get $n)))
    (call $mk (local.get $p) (local.get $n)))
  (func $name (param $h i32) (result i64) (local $n i32) (local $p i32)
    (local.set $n (call $name_of (local.get $h) (i32.const 0) (i32.const 0)))
    (local.set $p (call $alloc (local.get $n)))
    (drop (call $name_of (local.get $h) (local.get $p) (local.get $n)))
    (call $mk (local.get $p) (local.get $n)))
  ;; Sets the position of the object with handle h.
  (func $set_position_of (param $h i32) (param $x f32) (param $y f32) (param $z f32)
    (call $transform_get_of (local.get $h) (i32.const 0))
    (f32.store (i32.const 0) (local.get $x))
    (f32.store (i32.const 4) (local.get $y))
    (f32.store (i32.const 8) (local.get $z))
    (call $transform_set_of (local.get $h) (i32.const 0)))
  ;; Sets three transform values starting at byte offset $at.
  (func $set3 (param $at i32) (param $x f32) (param $y f32) (param $z f32)
    (call $transform_get (i32.const 0))
    (f32.store (local.get $at) (local.get $x))
    (f32.store offset=4 (local.get $at) (local.get $y))
    (f32.store offset=8 (local.get $at) (local.get $z))
    (call $transform_set (i32.const 0)))
  ;; Moves this object by an offset in double precision (bytes 96..120), so far from the centre it moves smoothly.
  (func $move_by (param $x f32) (param $y f32) (param $z f32)
    (call $position_get_of (call $self_handle) (i32.const 96))
    (f64.store (i32.const 96) (f64.add (f64.load (i32.const 96)) (f64.promote_f32 (local.get $x))))
    (f64.store (i32.const 104) (f64.add (f64.load (i32.const 104)) (f64.promote_f32 (local.get $y))))
    (f64.store (i32.const 112) (f64.add (f64.load (i32.const 112)) (f64.promote_f32 (local.get $z))))
    (call $position_set_of (call $self_handle) (i32.const 96)))
  ;; Adds to three transform values starting at byte offset $at.
  (func $add3 (param $at i32) (param $x f32) (param $y f32) (param $z f32)
    (call $transform_get (i32.const 0))
    (f32.store (local.get $at) (f32.add (f32.load (local.get $at)) (local.get $x)))
    (f32.store offset=4 (local.get $at) (f32.add (f32.load offset=4 (local.get $at)) (local.get $y)))
    (f32.store offset=8 (local.get $at) (f32.add (f32.load offset=8 (local.get $at)) (local.get $z)))
    (call $transform_set (i32.const 0)))
  ;; A whole number to a whole power, by squaring; it wraps past the 32-bit range. A negative
  ;; power is 0, except of 1 (1) and -1 (1 or -1).
  (func $ipow (param $b i32) (param $e i32) (result i32) (local $r i32)
    (if (i32.lt_s (local.get $e) (i32.const 0)) (then
      (if (i32.eq (local.get $b) (i32.const 1)) (then (return (i32.const 1))))
      (if (i32.eq (local.get $b) (i32.const -1)) (then
        (return (select (i32.const -1) (i32.const 1) (i32.and (local.get $e) (i32.const 1))))))
      (return (i32.const 0))))
    (local.set $r (i32.const 1))
    (block $done (loop $more
      (br_if $done (i32.eqz (local.get $e)))
      (if (i32.and (local.get $e) (i32.const 1)) (then (local.set $r (i32.mul (local.get $r) (local.get $b)))))
      (local.set $b (i32.mul (local.get $b) (local.get $b)))
      (local.set $e (i32.shr_u (local.get $e) (i32.const 1)))
      (br $more)))
    (local.get $r))
  ;; The length of the vector (x, y, z).
  (func $len3 (param $x f32) (param $y f32) (param $z f32) (result f32)
    (f32.sqrt (f32.add (f32.add (f32.mul (local.get $x) (local.get $x)) (f32.mul (local.get $y) (local.get $y))) (f32.mul (local.get $z) (local.get $z)))))
  ;; Part $c of the vector ($x, $y, $z) at length 1; 0 when the vector has no length.
  (func $unit (param $c f32) (param $x f32) (param $y f32) (param $z f32) (result f32) (local $l f32)
    (local.set $l (call $len3 (local.get $x) (local.get $y) (local.get $z)))
    (if (result f32) (f32.gt (local.get $l) (f32.const 1e-6))
      (then (f32.div (local.get $c) (local.get $l)))
      (else (f32.const 0))))
  ;; From a (t 0) to b (t 1) on a straight line.
  (func $lerp (param $a f32) (param $b f32) (param $t f32) (result f32)
    (f32.add (local.get $a) (f32.mul (f32.sub (local.get $b) (local.get $a)) (local.get $t))))
  ;; Where v lies from a (0) to b (1); 0 when a and b are equal.
  (func $unlerp (param $a f32) (param $b f32) (param $v f32) (result f32)
    (if (f32.eq (local.get $a) (local.get $b)) (then (return (f32.const 0))))
    (f32.div (f32.sub (local.get $v) (local.get $a)) (f32.sub (local.get $b) (local.get $a))))
  ;; Sine: reduce to [-pi/2, pi/2], then the Taylor series to x^9.
  (func $sin (param $x f32) (result f32) (local $x2 f32)
    (local.set $x (f32.sub (local.get $x) (f32.mul (f32.nearest (f32.mul (local.get $x) (f32.const 0.15915494))) (f32.const 6.2831855))))
    (if (f32.gt (local.get $x) (f32.const 1.5707964)) (then (local.set $x (f32.sub (f32.const 3.1415927) (local.get $x)))))
    (if (f32.lt (local.get $x) (f32.const -1.5707964)) (then (local.set $x (f32.sub (f32.const -3.1415927) (local.get $x)))))
    (local.set $x2 (f32.mul (local.get $x) (local.get $x)))
    (f32.mul (local.get $x)
      (f32.add (f32.const 1)
        (f32.mul (local.get $x2)
          (f32.add (f32.const -0.16666667)
            (f32.mul (local.get $x2)
              (f32.add (f32.const 0.008333333)
                (f32.mul (local.get $x2)
                  (f32.add (f32.const -0.0001984127)
                    (f32.mul (local.get $x2) (f32.const 0.0000027557319)))))))))))
  ;; Writes $v with up to three decimals at $at; returns the length.
  (func $fmt (param $v f32) (param $at i32) (result i32) (local $p i32) (local $n i64) (local $ip i64) (local $fr i64) (local $tmp i32) (local $end i32)
    (local.set $p (local.get $at))
    (if (f32.ne (local.get $v) (local.get $v)) (then
      (i32.store8 (local.get $p) (i32.const 78))
      (i32.store8 offset=1 (local.get $p) (i32.const 97))
      (i32.store8 offset=2 (local.get $p) (i32.const 78))
      (return (i32.const 3))))
    (if (f32.lt (local.get $v) (f32.const 0)) (then
      (i32.store8 (local.get $p) (i32.const 45))
      (local.set $p (i32.add (local.get $p) (i32.const 1)))
      (local.set $v (f32.neg (local.get $v)))))
    (if (f32.ge (local.get $v) (f32.const 1e9)) (then
      (i32.store8 (local.get $p) (i32.const 62))
      (i32.store8 offset=1 (local.get $p) (i32.const 49))
      (i32.store8 offset=2 (local.get $p) (i32.const 101))
      (i32.store8 offset=3 (local.get $p) (i32.const 57))
      (return (i32.sub (i32.add (local.get $p) (i32.const 4)) (local.get $at)))))
    (local.set $n (i64.trunc_f64_u (f64.nearest (f64.mul (f64.promote_f32 (local.get $v)) (f64.const 1000)))))
    (local.set $ip (i64.div_u (local.get $n) (i64.const 1000)))
    (local.set $fr (i64.rem_u (local.get $n) (i64.const 1000)))
    (local.set $end (i32.add (local.get $p) (i32.const 20)))
    (local.set $tmp (local.get $end))
    (loop $digits
      (local.set $tmp (i32.sub (local.get $tmp) (i32.const 1)))
      (i32.store8 (local.get $tmp) (i32.add (i32.const 48) (i32.wrap_i64 (i64.rem_u (local.get $ip) (i64.const 10)))))
      (local.set $ip (i64.div_u (local.get $ip) (i64.const 10)))
      (br_if $digits (i64.ne (local.get $ip) (i64.const 0))))
    (block $moved (loop $move
      (br_if $moved (i32.ge_u (local.get $tmp) (local.get $end)))
      (i32.store8 (local.get $p) (i32.load8_u (local.get $tmp)))
      (local.set $p (i32.add (local.get $p) (i32.const 1)))
      (local.set $tmp (i32.add (local.get $tmp) (i32.const 1)))
      (br $move)))
    (if (i64.ne (local.get $fr) (i64.const 0)) (then
      (i32.store8 (local.get $p) (i32.const 46))
      (i32.store8 offset=1 (local.get $p) (i32.add (i32.const 48) (i32.wrap_i64 (i64.div_u (local.get $fr) (i64.const 100)))))
      (i32.store8 offset=2 (local.get $p) (i32.add (i32.const 48) (i32.wrap_i64 (i64.rem_u (i64.div_u (local.get $fr) (i64.const 10)) (i64.const 10)))))
      (i32.store8 offset=3 (local.get $p) (i32.add (i32.const 48) (i32.wrap_i64 (i64.rem_u (local.get $fr) (i64.const 10)))))
      (local.set $p (i32.add (local.get $p) (i32.const 4)))
      (block $trimmed (loop $trim
        (br_if $trimmed (i32.ne (i32.load8_u (i32.sub (local.get $p) (i32.const 1))) (i32.const 48)))
        (local.set $p (i32.sub (local.get $p) (i32.const 1)))
        (br $trim)))))
    (i32.sub (local.get $p) (local.get $at)))
"#;

#[cfg(test)]
mod tests;

/// How the graph's properties show in the Inspector, as `bayleaf_describe`'s `inspector` lists it:
/// each property with options in order, those of a group together in it (where the group first
/// appears). `None` when no property has options (one plain row each).
pub fn inspector_layout(graph: &Graph) -> Option<serde_json::Value> {
    if graph.properties.iter().all(|p| p.inspector.is_empty()) {
        return None;
    }
    let item = |p: &bayleaf_graph::Property| {
        let o = &p.inspector;
        let mut item = serde_json::json!({ "property": p.name });
        if !o.label.is_empty() {
            item["label"] = serde_json::json!(o.label);
        }
        if !o.hint.is_empty() {
            item["hint"] = serde_json::json!(o.hint);
        }
        if let Some(range) = o.range {
            item["range"] = serde_json::json!(range);
        }
        if o.slider {
            item["slider"] = serde_json::json!(true);
        }
        if o.read_only {
            item["readOnly"] = serde_json::json!(true);
        }
        if !o.choices.is_empty() {
            item["choices"] = serde_json::json!(o.choices);
        }
        if let Some((name, value)) = &o.show_if {
            item["showIf"] = serde_json::json!([name, value]);
        }
        item
    };
    let mut layout: Vec<serde_json::Value> = Vec::new();
    // Where each group sits in `layout`.
    let mut groups: Vec<(String, usize)> = Vec::new();
    for p in graph.properties.iter().filter(|p| !p.inspector.is_empty()) {
        if p.inspector.group.is_empty() {
            layout.push(item(p));
            continue;
        }
        let at = match groups.iter().find(|(g, _)| *g == p.inspector.group) {
            Some((_, at)) => *at,
            None => {
                layout.push(serde_json::json!({ "group": p.inspector.group, "items": [] }));
                groups.push((p.inspector.group.clone(), layout.len() - 1));
                layout.len() - 1
            }
        };
        if let Some(items) = layout[at]["items"].as_array_mut() {
            items.push(item(p));
        }
    }
    Some(serde_json::Value::Array(layout))
}

/// Checks the graph's Inspector options as the script host will when it loads the compiled script:
/// properties named are declared, options fit their types, and so on.
pub fn check_inspector(graph: &Graph) -> Result<(), String> {
    let Some(layout) = inspector_layout(graph) else { return Ok(()) };
    let declared: Vec<DeclaredProperty> = graph
        .properties
        .iter()
        .map(|p| {
            let (ty, shape) = (scene_property_type(p.ty), scene_property_shape(p.shape, p.key));
            let default = PropertyValue::from_described(&described_value(p.ty, p.shape, &p.default), ty, shape).unwrap_or_else(|| ty.zero_of(shape));
            DeclaredProperty { name: p.name.clone(), ty, shape, default }
        })
        .collect();
    bayleaf_scene::inspector::parse(&layout, &declared).map(|_| ())
}
