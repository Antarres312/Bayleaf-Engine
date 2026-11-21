//! Visual scripts: their node kinds and the compiler that turns a graph into a WebAssembly module
//! with the same ABI as any other script (`docs/script-abi.md`).
//!
//! Event nodes (On Start, On Update) begin chains of actions joined by flow links. Data nodes are
//! pure: they are computed where an action needs them. The compiler writes WebAssembly text and
//! assembles it, so the result runs under the same fuel and memory limits as hand-written scripts.

use std::fmt::Write as _;

use bayleaf_graph::{Catalog, Graph, NodeDef, PinDef, PinDefault, PinType, Problem, SettingDef, SettingKind, VariableType};

use PinType::{Any, Bool, Float, Flow, Vec3};

mod fields;
pub use fields::{COMPONENTS, ComponentNodes, component_of};

const fn pin(name: &'static str, label: &'static str, ty: PinType) -> PinDef {
    PinDef::new(name, label, ty)
}

const fn number(name: &'static str, label: &'static str, ty: PinType, value: f32) -> PinDef {
    PinDef::with(name, label, ty, PinDefault::Float(value))
}

const fn vector(name: &'static str, label: &'static str, value: [f32; 3]) -> PinDef {
    PinDef::with(name, label, Vec3, PinDefault::Vec3(value))
}

const RUN: PinDef = pin("run", "Run", Flow);
const THEN: PinDef = pin("then", "Then", Flow);
const MATH_OUT: &[PinDef] = &[pin("result", "Result", Any)];
const MESSAGE: SettingDef = SettingDef { name: "message", label: "Message", kind: SettingKind::Name("hit") };
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
/// The object an action works on or a value is read from, from a search or This Object; when it is not
/// linked, the node uses the object named in Object Name (or its own default).
const TARGET: PinDef = pin("target", "Object", PinType::Object);
const OBJECT_NAME: SettingDef = SettingDef { name: "object", label: "Object Name", kind: SettingKind::Text("") };
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
    nodes: &[
        node("start", "On Start", "Events", "Runs once when play mode starts.", &[], &[THEN]),
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
            inputs: &[RUN, pin("value", "Value", Float)],
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
        NodeDef {
            kind: "property",
            label: "Property",
            category: "Values",
            description: "A number set per object in the Inspector, with a default.",
            inputs: &[],
            outputs: &[pin("value", "Value", Float)],
            settings: &[
                SettingDef { name: "name", label: "Name", kind: SettingKind::Name("speed") },
                SettingDef { name: "default", label: "Default", kind: SettingKind::Float(1.0) },
            ],
            unique: false,
        },
        node("position", "Position", "Object", "The object's position, relative to its parent.", &[], &[pin("position", "Position", Vec3)]),
        node("rotation", "Rotation", "Object", "The object's rotation in degrees.", &[], &[pin("rotation", "Rotation", Vec3)]),
        node("scale", "Scale", "Object", "The object's size along X, Y and Z.", &[], &[pin("scale", "Scale", Vec3)]),
        node("time", "Time", "Inputs", "Seconds since play mode started.", &[], &[pin("seconds", "Seconds", Float)]),
        node(
            "pointer",
            "Pointer Position",
            "Inputs",
            "Where the pointer is over the game view, from -1 to 1 (x right, y up), and whether it is over the view.",
            &[],
            &[pin("position", "Position", Vec3), pin("inside", "Over the View", Bool)],
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
            inputs: &[],
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
            outputs: &[pin("count", "Count", Float)],
            settings: &[KIND],
            unique: false,
        },
        NodeDef {
            kind: "forEachWith",
            label: "For Each Object With",
            category: "Find",
            description: "Runs Each once for every object that has a component, a tag, or runs a script, in hierarchy order as the step began, with that object and its index (from 0); then runs Then.",
            inputs: &[RUN],
            outputs: &[pin("each", "Each", Flow), pin("object", "Object", PinType::Object), pin("index", "Index", Float), THEN],
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
            label: "Raycast",
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
        node("divide", "Divide", "Math", "A ÷ B.", &[number("a", "A", Any, 1.0), number("b", "B", Any, 1.0)], MATH_OUT),
        node("sine", "Sine", "Math", "The sine of the input, in radians.", &[number("value", "Value", Any, 0.0)], MATH_OUT),
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
    ],
};

/// Most WebAssembly text one graph may produce (shared chains are copied, so this can grow).
const MAX_TEXT: usize = 8 << 20;
/// Where static text starts in the module's memory. Bytes 0..36 hold the transform.
const DATA_START: u32 = 1024;
/// Room after a log message for the number.
const NUMBER_ROOM: u32 = 32;

/// A compiled value: WebAssembly expressions that leave it on the stack.
#[derive(Clone)]
enum Val {
    F(String),
    V([String; 3]),
    B(String),
    /// An object's handle (`i32`; 0 for none).
    O(String),
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
    properties: Vec<(String, f32)>,
    /// Globals actions need (the object and index of a For Each), declared once each.
    globals: Vec<String>,
}

/// Compiles a visual script into a WebAssembly module.
/// The node kinds that read and set a variable of `ty`, by their `name` setting: the Get node first.
pub fn variable_kinds(ty: VariableType) -> [&'static str; 2] {
    match ty {
        VariableType::Number => ["variable", "setVariable"],
        VariableType::Vector => ["vectorVariable", "setVectorVariable"],
        VariableType::TrueFalse => ["flag", "setFlag"],
    }
}

pub fn compile(graph: &Graph) -> Result<Vec<u8>, Vec<Problem>> {
    let text = to_wat(graph)?;
    wat::parse_str(&text).map_err(|e| vec![Problem::graph(format!("internal compiler error: {e}"))])
}

/// The WebAssembly text of a visual script.
pub fn to_wat(graph: &Graph) -> Result<String, Vec<Problem>> {
    graph.check(&CATALOG).map_err(|e| vec![Problem::graph(e.to_string())])?;
    let mut c = Compiler { graph, data: Vec::new(), next: DATA_START, properties: Vec::new(), globals: Vec::new() };
    let mut problems = Vec::new();
    let mut start = String::new();
    let mut update = String::new();
    let mut message = String::new();
    // Globals the events keep between steps.
    let mut state: Vec<String> = Vec::new();
    let mut events: Vec<&bayleaf_graph::Node> =
        graph.nodes.iter().filter(|n| matches!(n.kind.as_str(), "start" | "update" | "message" | "keyPressed" | "actionPressed" | "timer")).collect();
    events.sort_by_key(|n| n.id);
    for event in events {
        match c.chain(event.id, "then", 1) {
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
                // Each On Message runs only for its own message name.
                let name = c.setting_text(event.id, "message", "hit");
                let (at, len) = c.text(name.as_bytes());
                let _ = writeln!(message, "    (if (call $message_is (i32.const {at}) (i32.const {len}))\n      (then\n{code}      ))");
            }
            Err(p) => problems.push(p),
        }
        if start.len() + update.len() + message.len() > MAX_TEXT {
            return Err(vec![Problem::graph("the script is too large; reuse fewer shared chains")]);
        }
    }
    // Property nodes declare properties even when nothing reads them yet.
    for node in graph.nodes.iter().filter(|n| n.kind == "property") {
        c.declare_property(node.id);
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    let mut description = serde_json::json!({ "properties": c.properties.iter().map(|(n, v)| serde_json::json!([n, v])).collect::<Vec<_>>() });
    if !graph.menu.trim().is_empty() {
        match crate::menu_path(&graph.menu) {
            Ok(path) => description["menu"] = serde_json::json!(path),
            Err(problem) => return Err(vec![Problem::graph(format!("menu path: {problem}"))]),
        }
    }
    let description = description.to_string();
    let (describe_at, describe_len) = c.text(description.as_bytes());
    let pages = c.next.div_ceil(65536).max(1);

    let mut out = String::new();
    out.push_str(PRELUDE);
    // One global per variable name; names are letters, digits and _, so they are valid identifiers.
    // Declared variables start at their values; names nodes use without a declaration start at zero.
    let named = |ty: VariableType, default: &str| -> std::collections::BTreeMap<String, serde_json::Value> {
        let kinds = variable_kinds(ty);
        let mut names: std::collections::BTreeMap<String, serde_json::Value> =
            graph.nodes.iter().filter(|n| kinds.contains(&n.kind.as_str())).map(|n| (c.setting_text(n.id, "name", default), ty.zero())).collect();
        names.extend(graph.variables.iter().filter(|v| v.ty == ty).map(|v| (v.name.clone(), v.start.clone())));
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
    let _ = writeln!(out, "  (func (export \"bayleaf_start\")\n{start}  )");
    let _ = writeln!(out, "  (func (export \"bayleaf_update\") (param $dt f32)\n    (global.set $dt (local.get $dt))\n{update}  )");
    if graph.nodes.iter().any(|n| n.kind == "message") {
        let _ = writeln!(out, "  (func (export \"bayleaf_message\") (param $value f32)\n    (global.set $message (local.get $value))\n{message}  )");
    }
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

    fn declare_property(&mut self, node: u32) -> String {
        let name = self.setting_text(node, "name", "speed");
        let default = self.setting_number(node, "default", 1.0);
        if !self.properties.iter().any(|(n, _)| *n == name) {
            self.properties.push((name.clone(), default));
        }
        name
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
                let message = self.setting_text(next, "message", "Hello");
                if graph.link_into(next, "value").is_some() {
                    let value = self.input(next, "value").and_then(|v| as_float(next, v))?;
                    let mut bytes = message.into_bytes();
                    bytes.push(b' ');
                    let (at, len) = self.reserve(&bytes, NUMBER_ROOM);
                    let _ = writeln!(code, "{indent}(call $log (i32.const {at}) (i32.add (i32.const {len}) (call $fmt {value} (i32.const {}))))", at + len);
                } else {
                    let (at, len) = self.text(message.as_bytes());
                    let _ = writeln!(code, "{indent}(call $log (i32.const {at}) (i32.const {len}))");
                }
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

    /// The value of one output of a data node.
    fn output(&mut self, node: u32, pin: &str) -> Result<Val, Problem> {
        let graph = self.graph;
        let (n, def) = graph.node_def(&CATALOG, node).map_err(|e| Problem::on(node, e.to_string()))?;
        let transform = |base: u32| Val::V([0, 1, 2].map(|i| format!("(call $t (i32.const {}))", base + i)));
        Ok(match n.kind.as_str() {
            "update" if pin == "dt" => Val::F("(global.get $dt)".to_owned()),
            "message" if pin == "value" => Val::F("(global.get $message)".to_owned()),
            "vectorVariable" => {
                let name = self.setting_text(node, "name", "target");
                Val::V(["x", "y", "z"].map(|axis| format!("(global.get $vec_{name}_{axis})")))
            }
            "flag" => Val::B(format!("(global.get $flag_{})", self.setting_text(node, "name", "done"))),
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
            "property" => {
                let name = self.declare_property(node);
                let (at, len) = self.text(name.as_bytes());
                Val::F(format!("(call $property (i32.const {at}) (i32.const {len}))"))
            }
            "position" => transform(0),
            "rotation" => transform(3),
            "scale" => transform(6),
            "time" => Val::F("(f32.demote_f64 (call $time))".to_owned()),
            "pointer" if pin == "inside" => Val::B("(call $pointer_inside)".to_owned()),
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
                        Bool => Val::B(format!("(f32.ne {} (f32.const 0))", part(0))),
                        _ => Val::V([part(0), part(4), part(8)]),
                    }
                }
            }
            "thisObject" => Val::O("(call $self_handle)".to_owned()),
            "findNamed" => {
                let (at, len) = self.named(node, "name", "name the object to find")?;
                let found = format!("(call $find (i32.const {at}) (i32.const {len}))");
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
                Val::F(format!("(f32.convert_i32_s (call $count_with (i32.const {at}) (i32.const {len})))"))
            }
            "forEachWith" if pin == "object" => Val::O(format!("(global.get $each_{node})")),
            "forEachWith" if pin == "index" => Val::F(format!("(f32.convert_i32_s (global.get $each_index_{node}))")),
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
                zip(node, a, b, |a, b| format!("({op} {a} {b})"))?
            }
            "sine" => match self.input(node, "value")? {
                Val::F(v) => Val::F(format!("(call $sin {v})")),
                Val::V(v) => Val::V(v.map(|c| format!("(call $sin {c})"))),
                Val::B(_) | Val::O(_) => return Err(Problem::on(node, "Sine needs a number or a vector")),
            },
            "choose" => {
                let condition = self.input(node, "condition").and_then(|v| as_bool(node, v))?;
                let a = self.input(node, "a")?;
                let b = self.input(node, "b")?;
                zip(node, a, b, |a, b| format!("(select {a} {b} {condition})"))?
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
            _ => return Err(Problem::on(node, format!("{} gives no value here", def.label))),
        })
    }
}

/// Applies `op` to two numbers, or to each part when either is a vector.
fn zip(node: u32, a: Val, b: Val, op: impl Fn(&str, &str) -> String) -> Result<Val, Problem> {
    Ok(match (a, b) {
        (Val::F(a), Val::F(b)) => Val::F(op(&a, &b)),
        (Val::V(a), Val::V(b)) => Val::V([op(&a[0], &b[0]), op(&a[1], &b[1]), op(&a[2], &b[2])]),
        (Val::V(a), Val::F(b)) => Val::V(a.map(|c| op(&c, &b))),
        (Val::F(a), Val::V(b)) => Val::V(b.map(|c| op(&a, &c))),
        _ => return Err(Problem::on(node, "math needs numbers or vectors, not true/false")),
    })
}

fn as_float(node: u32, value: Val) -> Result<String, Problem> {
    match value {
        Val::F(f) => Ok(f),
        Val::V(_) => Err(Problem::on(node, "this input takes a number but gets a vector; use Split Vector")),
        Val::B(_) => Err(Problem::on(node, "this input takes a number but gets true/false; use Choose")),
        Val::O(_) => Err(Problem::on(node, "this input takes a number but gets an object; use Position Of")),
    }
}

fn as_vec(node: u32, value: Val) -> Result<[String; 3], Problem> {
    match value {
        Val::V(v) => Ok(v),
        Val::F(f) => Ok([f.clone(), f.clone(), f]),
        Val::B(_) => Err(Problem::on(node, "this input takes a vector but gets true/false")),
        Val::O(_) => Err(Problem::on(node, "this input takes a vector but gets an object; use Position Of")),
    }
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
  (import "bayleaf" "time" (func $time (result f64)))
  (import "bayleaf" "key_down" (func $key_down (param i32 i32) (result i32)))
  (import "bayleaf" "pointer" (func $pointer (param i32) (result i32)))
  (import "bayleaf" "mouse_down" (func $mouse_down (param i32) (result i32)))
  (import "bayleaf" "self_handle" (func $self_handle (result i32)))
  (import "bayleaf" "find" (func $find (param i32 i32) (result i32)))
  (import "bayleaf" "find_with" (func $find_with (param i32 i32 i32) (result i32)))
  (import "bayleaf" "count_with" (func $count_with (param i32 i32) (result i32)))
  (import "bayleaf" "component_get" (func $component_get (param i32 i32 i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "component_set" (func $component_set (param i32 i32 i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "transform_get_of" (func $transform_get_of (param i32 i32)))
  (import "bayleaf" "transform_set_of" (func $transform_set_of (param i32 i32)))
  (import "bayleaf" "position_get_of" (func $position_get_of (param i32 i32)))
  (import "bayleaf" "position_set_of" (func $position_set_of (param i32 i32)))
  (import "bayleaf" "spawn" (func $spawn (param i32 i32 f32 f32 f32) (result i32)))
  (import "bayleaf" "destroy" (func $destroy (param i32) (result i32)))
  (import "bayleaf" "send" (func $send (param i32 i32 i32 f32) (result i32)))
  (import "bayleaf" "message_is" (func $message_is (param i32 i32) (result i32)))
  (import "bayleaf" "state_get" (func $state_get (param i32 i32 i32 i32 i32) (result i32)))
  (import "bayleaf" "state_set" (func $state_set (param i32 i32 i32 i32 f64) (result i32)))
  (import "bayleaf" "game_end" (func $game_end (param i32) (result i32)))
  (import "bayleaf" "action" (func $action (param i32 i32) (result f32)))
  (import "bayleaf" "input_map_set" (func $input_map_set (param i32 i32 i32) (result i32)))
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
