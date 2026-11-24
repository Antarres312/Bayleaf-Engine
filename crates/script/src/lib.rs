//! L4: WebAssembly script host. Implements the ABI in `docs/script-abi.md`.
//!
//! Each `script` component gets its own WebAssembly instance: interpreted by wasmi (everywhere, the
//! web build included) or, with the `compiled` feature, compiled to machine code by wasmtime. The
//! host (`host.rs`) is the same code for both. Every call is metered with fuel and memory is capped,
//! so a broken script stops itself, not the editor.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use bayleaf_core::StableId;
use bayleaf_core::math::{DMat4, DVec3};
use bayleaf_scene::Scene;
use bayleaf_scene::components::Transform;
use bayleaf_scene::components::{DeclaredProperty, PropertyType, PropertyValue};

pub mod actions;
mod native;
pub use native::NativeScript;
pub mod visual;

/// File extension of visual scripts, which compile to modules when loaded.
pub const VISUAL_EXTENSION: &str = "bscript";

/// The first bytes of every WebAssembly module.
pub const WASM_MAGIC: &[u8] = b"\0asm";

/// The WebAssembly module of a script file: a `.wasm` file as it is, a visual script compiled (or
/// as it is, when an export already compiled it).
pub fn module_bytes(path: &Path) -> Result<Vec<u8>, ScriptError> {
    let read_error = |e: std::io::Error| ScriptError::Read { path: path.display().to_string(), message: e.to_string() };
    if path.extension().is_some_and(|e| e == VISUAL_EXTENSION) {
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        let bytes = bayleaf_core::files::read(path).map_err(read_error)?;
        // An exported game keeps its visual scripts already compiled.
        if bytes.starts_with(WASM_MAGIC) {
            return Ok(bytes);
        }
        let text = String::from_utf8(bytes).map_err(|e| ScriptError::Invalid(name.clone(), e.to_string()))?;
        let graph = bayleaf_graph::Graph::from_json(&text, &visual::CATALOG).map_err(|e| ScriptError::Invalid(name.clone(), e.to_string()))?;
        return visual::compile(&graph).map_err(|p| ScriptError::Invalid(name, bayleaf_graph::summary(&p)));
    }
    bayleaf_core::files::read(path).map_err(read_error)
}

/// ABI version the host implements.
pub const ABI_VERSION: i32 = 1;
/// Instructions one call may run before it is stopped.
pub const FUEL_PER_CALL: u64 = 10_000_000;
/// Largest linear memory a script may grow to.
pub const MEMORY_LIMIT: usize = 64 << 20;
/// Longest text a single `log` call may pass.
const MAX_LOG_BYTES: usize = 4096;
/// Log lines kept; older ones are dropped.
const MAX_LOG_LINES: usize = 1000;

/// Why a script could not load or stopped running. Messages are shown to the user.
#[derive(Clone, Debug, thiserror::Error, PartialEq)]
pub enum ScriptError {
    #[error("cannot read {path}: {message}")]
    Read { path: String, message: String },
    #[error("{0} is not a valid script: {1}")]
    Invalid(String, String),
    #[error("{module}: {message}")]
    Trapped { module: String, message: String },
    #[error("{module} uses script ABI {found}, but this engine needs {expected}")]
    AbiMismatch { module: String, found: i32, expected: i32 },
    /// Scripts cannot run the way asked here (a runtime this build lacks).
    #[error("{0}")]
    Unavailable(String),
}

/// Each object's named interface elements as they show: their value and their text, by object and
/// element name. The engine fills it before scripts run.
pub type InterfaceValues = std::collections::HashMap<(StableId, String), (f64, String)>;

/// What a script changes in an Interface element while the game plays.
#[derive(Clone, Debug, PartialEq)]
pub enum InterfaceChange {
    Text(String),
    Value(f64),
    Visible(bool),
    /// A new element named as the request's element, into the element named `parent` ("" for the
    /// root): `what` is a kind (`button`) or a layout file (`.bui`) used as a part.
    Add {
        parent: String,
        what: String,
    },
    /// The element goes, with what it holds.
    Remove,
}

/// What a script asks of a Particle Emitter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleAction {
    Play,
    Stop,
    /// Start this many particles at once.
    Burst(u32),
}

/// One problem with one of an object's scripts.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptFault {
    pub object: StableId,
    /// Display name of the script, e.g. `Spinner`.
    pub script: String,
    pub error: ScriptError,
}

/// A line written by a script.
#[derive(Clone, Debug, PartialEq)]
pub struct LogLine {
    pub object: StableId,
    /// Display name of the script, e.g. `Spinner`.
    pub script: String,
    pub text: String,
}

/// Largest description text a script may return from `bayleaf_describe`.
const MAX_DESCRIPTION_BYTES: u32 = 64 * 1024;

/// Per-frame input the host passes to scripts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Input {
    /// Names of the keys held down, as in `docs/script-abi.md`.
    pub keys: HashSet<String>,
    /// The pointer over the game view, from (-1, -1) at the bottom left to (1, 1) at the top right;
    /// `None` when it is outside the view.
    pub pointer: Option<[f32; 2]>,
    /// Mouse buttons held: bit 0 left, bit 1 right, bit 2 middle. None while the pointer is over
    /// an interface element that takes it.
    pub buttons: u8,
    /// The pointer is over an interface element that takes it (so its clicks are the interface's).
    pub over_interface: bool,
    /// The first connected gamepad, if any.
    pub pad: Option<Pad>,
}

/// A gamepad: sticks and triggers (by [`PAD_AXES`]) and the buttons held (bit i: [`PAD_BUTTONS`][i]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pad {
    /// Sticks from -1 to 1 (x right, y up), triggers from 0 to 1.
    pub axes: [f32; 6],
    pub buttons: u32,
}

/// Names of a gamepad's axes, by index.
pub const PAD_AXES: [&str; 6] = ["LeftX", "LeftY", "RightX", "RightY", "LeftTrigger", "RightTrigger"];
/// Names of a gamepad's buttons, by bit. South is the bottom face button, North the top one.
pub const PAD_BUTTONS: [&str; 16] = [
    "South",
    "East",
    "West",
    "North",
    "LeftBumper",
    "RightBumper",
    "LeftTrigger",
    "RightTrigger",
    "Select",
    "Start",
    "LeftStick",
    "RightStick",
    "Up",
    "Down",
    "Left",
    "Right",
];

/// Something a script asked for that changes which objects exist. The play loop does these
/// after the step, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Place a copy of a prefab file at a world position. `by` is the asking script's object.
    Spawn { by: StableId, prefab: String, position: [f32; 3] },
    /// Delete an object and its children.
    Destroy(StableId),
    /// Push a physics body by an impulse (world space). `by` is the asking script's object.
    Push { by: StableId, object: StableId, impulse: [f32; 3] },
    /// Set a physics body's velocity (world units per second).
    SetVelocity { by: StableId, object: StableId, velocity: [f32; 3] },
    /// Set a character's walking velocity (world units per second).
    Walk { by: StableId, object: StableId, velocity: [f32; 3] },
    /// Make a character on the ground jump with an upward speed.
    Jump { by: StableId, object: StableId, speed: f32 },
    /// Start, stop or burst the object's Particle Emitter.
    Particles { by: StableId, object: StableId, action: ParticleAction },
    /// Change an element of the object's Interface (an empty `element`: the whole interface).
    Interface { by: StableId, object: StableId, element: String, change: InterfaceChange },
    /// Play the object's Audio Source.
    PlaySound { by: StableId, object: StableId },
    /// Stop the sounds the object plays.
    StopSound { by: StableId, object: StableId },
    /// Play a sound file once (relative to the project folder) from a world position, at a volume.
    PlaySoundAt { by: StableId, path: String, position: [f32; 3], volume: f32 },
    /// Load a scene file (relative to the project folder): in place of the playing one, or added to it.
    LoadScene { by: StableId, path: String, additive: bool },
    /// Remove the scenes added from this file while playing.
    UnloadScene { by: StableId, path: String },
    /// Set a field of a component (`pointLight`, `intensity`): a number takes `value[0]`, true/false
    /// whether it is not 0, a vector or colour its first parts from `value`.
    SetField { by: StableId, object: StableId, component: &'static str, field: String, value: [f32; 3] },
}

/// Named numbers the game keeps while it plays: the game's own, each player's (a player is an
/// object with Player Control), the saved ones that last from one play to the next, and how the
/// game ended. Everything starts empty, except saved values the project kept; reading a name that
/// was never set gives nothing, not an error.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GameState {
    pub game: BTreeMap<String, f64>,
    pub players: BTreeMap<StableId, BTreeMap<String, f64>>,
    /// Kept between plays (the engine loads and writes them), such as a best score.
    pub saved: BTreeMap<String, f64>,
    /// `Some(true)` won, `Some(false)` lost; the first ending counts.
    pub outcome: Option<bool>,
}

/// Whose value: the game's, a player's, or a saved one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Game,
    Player(StableId),
    Saved,
}

/// Most values in the game's state, in each player's, and saved.
pub const MAX_STATE_VALUES: usize = 1024;

/// Whether a name and number may be kept: a name of 1 to 64 bytes and a finite number.
pub fn valid_value(name: &str, value: f64) -> bool {
    !name.is_empty() && name.len() <= 64 && value.is_finite()
}

impl GameState {
    /// Sets a value; false when the name is not 1 to 64 bytes, the number is not finite, or the
    /// state is full.
    pub fn set(&mut self, scope: Scope, name: &str, value: f64) -> bool {
        if !valid_value(name, value) {
            return false;
        }
        let map = match scope {
            Scope::Game => &mut self.game,
            Scope::Player(id) => self.players.entry(id).or_default(),
            Scope::Saved => &mut self.saved,
        };
        if map.len() >= MAX_STATE_VALUES && !map.contains_key(name) {
            return false;
        }
        map.insert(name.to_owned(), value);
        true
    }

    pub fn get(&self, scope: Scope, name: &str) -> Option<f64> {
        match scope {
            Scope::Game => self.game.get(name).copied(),
            Scope::Player(id) => self.players.get(&id)?.get(name).copied(),
            Scope::Saved => self.saved.get(name).copied(),
        }
    }

    /// Ends the game, unless it already ended. Returns true when this call ended it.
    pub fn end(&mut self, won: bool) -> bool {
        if self.outcome.is_some() {
            return false;
        }
        self.outcome = Some(won);
        true
    }
}

/// The state shared by every script of a play session.
pub type SharedState = Arc<std::sync::Mutex<GameState>>;

fn lock(state: &SharedState) -> std::sync::MutexGuard<'_, GameState> {
    state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Physics bodies' velocities, as scripts see them during a step.
pub type Velocities = std::collections::HashMap<StableId, [f32; 3]>;

/// Most requests one script call may make.
const MAX_REQUESTS_PER_CALL: usize = 64;
/// Most messages one script call may send.
const MAX_MESSAGES_PER_CALL: usize = 64;
/// Longest message name, in bytes.
pub const MAX_MESSAGE_NAME: usize = 64;
/// Most objects one script keeps with `object_ref`.
pub const MAX_OBJECT_REFS: usize = 4096;
/// `send`'s handle for every object.
pub const EVERY_OBJECT: i32 = -1;

/// One value an event brings (`event_send`; a visual script's Trigger Event). An object is kept by
/// its id, so it outlasts the step it was sent in; it reads as none once it is gone.
#[derive(Clone, Debug, PartialEq)]
pub enum EventArg {
    Number(f32),
    Whole(i32),
    Flag(bool),
    Vector([f32; 3]),
    Text(String),
    Object(Option<StableId>),
}

/// Most values one event brings, and the longest text among them, in bytes.
pub const MAX_EVENT_ARGS: usize = 16;
pub const MAX_EVENT_TEXT: usize = 4096;

/// A message one script sent, delivered after the step to the scripts of `to` (`None`: every
/// object) that export `bayleaf_message`.
#[derive(Clone, Debug, PartialEq)]
struct Message {
    to: Option<StableId>,
    name: String,
    value: f32,
    /// What an event brings; empty for a plain message.
    args: Arc<[EventArg]>,
    /// Only this script of the object (its place in the object's scripts), when the event was
    /// triggered on one component.
    script: Option<usize>,
    /// The component an event happened on, when this is a subscriber's copy of it.
    from: Option<(StableId, usize)>,
}

/// A script's subscription to an event of another component: when `event` happens on `publisher`
/// (an object and the place of its script), `subscriber` hears it as `<publisher's script>/<event>`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Subscription {
    publisher: (StableId, usize),
    event: String,
    subscriber: (StableId, usize),
}

/// Most subscriptions one game keeps.
pub const MAX_SUBSCRIPTIONS: usize = 65_536;

/// The objects scripts can see during one step: names and local transforms as the step began.
/// A handle is an object's position in this list plus one; 0 means none.
#[derive(Default)]
struct WorldView {
    objects: Vec<(StableId, String, [f32; 9])>,
    /// What rays can hit, by the same index: the world matrix and the box of the object's shape or
    /// sprite, in its own space.
    shapes: Vec<Option<(DMat4, RayShape)>>,
    /// Each object's position in double precision, by the same index.
    positions: Vec<[f64; 3]>,
    /// The fields of each object's readable components (`visual::COMPONENTS`), by the same index;
    /// empty unless a loaded script reads or sets component fields.
    fields: Vec<Vec<(&'static str, serde_json::Value)>>,
    /// The scripts on each object with their properties as the step began, by object; empty unless a
    /// loaded script reads or sets component fields.
    scripts: HashMap<StableId, Vec<ScriptValues>>,
    /// What each object is, by the same index, for searches: its components' names and labels,
    /// its tags and its scripts' names, each as `kind_key` makes them.
    kinds: Vec<Vec<String>>,
    /// Objects with Player Control, in hierarchy order.
    players: Vec<StableId>,
    /// Each object's scripts in order, by the same index: each one's name as `kind_key` makes it.
    script_names: Vec<Vec<String>>,
}

impl WorldView {
    /// `scripts` is `Some` when scripts read or set component fields: then their values come along.
    fn of(scene: &Scene, scripts: Option<HashMap<StableId, Vec<ScriptValues>>>) -> Arc<Self> {
        let with_fields = scripts.is_some();
        let all = scene.world_matrices();
        let objects = all.iter().map(|(id, _)| (*id, scene.name(*id).unwrap_or_default().to_owned(), read_transform(scene, *id))).collect();
        let shapes = all.iter().map(|(id, world)| ray_shape(scene, *id, *world)).collect();
        let positions = all.iter().map(|(id, _)| read_position(scene, *id)).collect();
        let players = all.iter().map(|(id, _)| *id).filter(|id| scene.get::<bayleaf_scene::components::PlayerControl>(*id).is_some()).collect();
        let kinds = all.iter().map(|(id, _)| object_kinds(scene, *id)).collect();
        let fields = if with_fields { all.iter().map(|(id, _)| object_fields(scene, *id)).collect() } else { Vec::new() };
        let script_names = all
            .iter()
            .map(|(id, _)| {
                scene
                    .get::<bayleaf_scene::components::Scripts>(*id)
                    .map(|s| s.0.iter().map(|script| kind_key(&script.display_name())).collect())
                    .unwrap_or_default()
            })
            .collect();
        Arc::new(Self { objects, shapes, positions, players, kinds, fields, scripts: scripts.unwrap_or_default(), script_names })
    }

    /// The nearest shape a ray hits within `max`, other than `skip`: its index and the distance.
    fn raycast(&self, origin: DVec3, direction: DVec3, max: f64, skip: StableId) -> Option<(usize, f64)> {
        let mut best: Option<(usize, f64)> = None;
        for (index, shape) in self.shapes.iter().enumerate() {
            let Some((pose, shape)) = shape else { continue };
            if self.objects[index].0 == skip || pose.determinant().abs() < 1e-12 {
                continue;
            }
            // In the shape's own space; the ray's parameter stays the world distance.
            let inverse = pose.inverse();
            let (o, d) = (inverse.transform_point3(origin), inverse.transform_vector3(direction));
            if let Some(t) = shape.hit(o, d).filter(|t| *t <= max)
                && best.is_none_or(|(_, b)| t < b)
            {
                best = Some((index, t));
            }
        }
        best
    }
    /// The handles of the objects that are `kind` (a component or a script, see `kind_key`), in
    /// hierarchy order.
    fn with_kind(&self, kind: &str) -> impl Iterator<Item = i32> + '_ {
        let key = kind_key(kind);
        self.kinds.iter().enumerate().filter(move |(_, kinds)| !key.is_empty() && kinds.contains(&key)).map(|(i, _)| i as i32 + 1)
    }

    fn handle_of(&self, id: StableId) -> i32 {
        self.objects.iter().position(|(o, _, _)| *o == id).map_or(0, |i| i as i32 + 1)
    }

    /// The object a handle names and its index.
    fn object(&self, handle: i32) -> Option<(StableId, usize)> {
        let index = usize::try_from(handle).ok()?.checked_sub(1)?;
        Some((self.objects.get(index)?.0, index))
    }

    /// The place (plus one) of the first script named `name` on the object with `handle`; 0 for none.
    fn script_on(&self, handle: i32, name: &str) -> i32 {
        let key = kind_key(name);
        self.object(handle)
            .and_then(|(_, index)| self.script_names.get(index)?.iter().position(|k| *k == key))
            .map_or(0, |place| i32::try_from(place + 1).unwrap_or(0))
    }

    fn get(&self, handle: i32) -> Option<&(StableId, String, [f32; 9])> {
        usize::try_from(handle).ok().and_then(|h| h.checked_sub(1)).and_then(|i| self.objects.get(i))
    }
}

/// One script on an object, as a component other scripts read: its names as search keys (the name
/// the Inspector shows and its file name without the extension) and its properties.
#[derive(Clone, Debug)]
struct ScriptValues {
    keys: [String; 2],
    properties: Properties,
}

/// A script's properties as it runs: each with its declared type and current value.
type Properties = BTreeMap<String, (PropertyType, PropertyValue)>;

/// A property of a script on an object, set by another script after the step.
#[derive(Clone, Debug)]
struct PropertyWrite {
    object: StableId,
    key: String,
    name: String,
    /// The value as three numbers; a property of one number takes the first.
    value: [f32; 3],
}

/// The search keys of a script: its shown name and its file name without the extension.
fn script_keys(name: &str, path: &Path) -> [String; 2] {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    [kind_key(name), kind_key(&stem)]
}

/// The readable components an object has, with their values.
fn object_fields(scene: &Scene, id: StableId) -> Vec<(&'static str, serde_json::Value)> {
    visual::COMPONENTS.iter().filter_map(|c| scene.component(id, c.name).ok().map(|value| (c.name, value))).collect()
}

/// The readable component a name or label means (`pointLight`, `Point Light`).
fn readable_component(text: &str) -> Option<&'static str> {
    let key = kind_key(text);
    visual::COMPONENTS.iter().find(|c| kind_key(c.name) == key || kind_key(c.label) == key).map(|c| c.name)
}

/// A field's value as up to three numbers: a number, true/false as 1 or 0, or the first parts of a
/// vector or colour. `None` for other fields.
fn field_numbers(value: &serde_json::Value) -> Option<[f32; 3]> {
    match value {
        serde_json::Value::Number(n) => Some([n.as_f64()? as f32, 0.0, 0.0]),
        serde_json::Value::Bool(b) => Some([f32::from(u8::from(*b)), 0.0, 0.0]),
        serde_json::Value::Array(parts) if (2..=4).contains(&parts.len()) => {
            let mut out = [0.0; 3];
            for (slot, part) in out.iter_mut().zip(parts) {
                *slot = part.as_f64()? as f32;
            }
            Some(out)
        }
        _ => None,
    }
}

/// A field set to `numbers` the way `field_numbers` reads it, keeping its shape (a colour keeps its
/// alpha). `None` for a field that is not a number, true/false, vector or colour.
pub fn with_field(value: &serde_json::Value, numbers: [f32; 3]) -> Option<serde_json::Value> {
    field_numbers(value)?;
    Some(match value {
        serde_json::Value::Bool(_) => serde_json::json!(numbers[0] != 0.0),
        serde_json::Value::Array(parts) => {
            serde_json::Value::Array(parts.iter().enumerate().map(|(i, part)| numbers.get(i).map_or_else(|| part.clone(), |n| serde_json::json!(n))).collect())
        }
        _ => serde_json::json!(numbers[0]),
    })
}

/// A search key for a kind of object: lowercase, without spaces, `_` or `-`, so `pointLight`, `Point
/// Light` and `point_light` all match.
pub(crate) fn kind_key(text: &str) -> String {
    text.chars().filter(|c| !matches!(c, ' ' | '_' | '-')).flat_map(char::to_lowercase).collect()
}

/// What an object is, for searches: its components' names and labels, its tags and its scripts' names
/// (the name the Inspector shows and the file name without its extension).
fn object_kinds(scene: &Scene, id: StableId) -> Vec<String> {
    let mut kinds: Vec<String> = scene.component_kinds(id).into_iter().flat_map(|(name, label)| [kind_key(name), kind_key(label)]).collect();
    if let Some(tags) = scene.get::<bayleaf_scene::components::Tags>(id) {
        kinds.extend(tags.0.iter().map(|tag| kind_key(tag)));
    }
    if let Some(scripts) = scene.get::<bayleaf_scene::components::Scripts>(id) {
        for script in &scripts.0 {
            let file = script.module.rsplit(['/', '\\']).next().unwrap_or(&script.module);
            let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
            kinds.push(kind_key(stem));
            kinds.push(kind_key(&script.display_name()));
        }
    }
    kinds.sort_unstable();
    kinds.dedup();
    kinds
}

/// State one instance's host functions can see.
struct HostState<L> {
    /// The object this instance belongs to.
    object: StableId,
    world: Arc<WorldView>,
    /// Transforms of other objects this call set, applied after it.
    writes: Vec<(StableId, [f32; 9], [f64; 3])>,
    /// This object's position in double precision.
    position: [f64; 3],
    requests: Vec<Request>,
    /// Messages this call sent.
    outbox: Vec<Message>,
    /// Script properties this call set on other objects.
    property_writes: Vec<PropertyWrite>,
    /// Objects this script keeps in Object variables, by `object_ref` number (index + 1). They outlast
    /// the step, unlike handles.
    refs: Vec<StableId>,
    /// The name of the message being delivered, during `bayleaf_message`.
    message: Option<String>,
    /// What the message being delivered brings, when it is an event.
    message_args: Arc<[EventArg]>,
    /// The component the event being delivered happened on, for a subscriber.
    message_from: Option<(StableId, usize)>,
    /// This script's place in its object's scripts.
    index: usize,
    /// Subscriptions this call asked for (true) or cancelled (false): the publisher and the event.
    subscribing: Vec<(bool, (StableId, usize), String)>,
    velocities: Arc<Velocities>,
    grounded: Arc<HashSet<StableId>>,
    /// Interface elements as they show, for `ui_value` and `ui_text`.
    interface: Arc<InterfaceValues>,
    input_maps: actions::SharedInput,
    state: SharedState,
    transform: [f32; 9],
    transform_changed: bool,
    properties: Properties,
    time: f64,
    input: Arc<Input>,
    logs: Vec<String>,
    /// Set by a host function that detected a guest error, so the trap message says why.
    fault: Option<String>,
    /// This object's value of a native script, when the module is one built into the game.
    native: Option<Box<dyn bayleaf_script_sdk::native::NativeObject>>,
    /// The runtime's memory and instance limits for this store.
    limits: L,
}

impl<L> HostState<L> {
    fn new(object: StableId, transform: [f32; 9], properties: Properties, time: f64, input: &Arc<Input>, limits: L) -> Self {
        Self {
            object,
            world: Arc::new(WorldView::default()),
            writes: Vec::new(),
            position: [transform[0], transform[1], transform[2]].map(f64::from),
            requests: Vec::new(),
            outbox: Vec::new(),
            property_writes: Vec::new(),
            refs: Vec::new(),
            message: None,
            message_args: Arc::from([]),
            message_from: None,
            index: 0,
            subscribing: Vec::new(),
            velocities: Arc::new(Velocities::new()),
            grounded: Arc::new(HashSet::new()),
            interface: Arc::new(InterfaceValues::new()),
            input_maps: actions::SharedInput::default(),
            state: SharedState::default(),
            transform,
            transform_changed: false,
            properties,
            time,
            input: Arc::clone(input),
            logs: Vec::new(),
            fault: None,
            native: None,
            limits,
        }
    }
}

impl<L> HostState<L> {
    /// The single-precision transform and precise position of `object` (index `index` in the
    /// world view) as this call last set them, else as the step began.
    fn current(&self, object: StableId, index: usize) -> ([f32; 9], [f64; 3]) {
        if object == self.object {
            return (self.transform, self.position);
        }
        self.writes
            .iter()
            .rev()
            .find(|(o, _, _)| *o == object)
            .map(|(_, v, p)| (*v, *p))
            .unwrap_or_else(|| (self.world.objects[index].2, self.world.positions[index]))
    }

    /// Records a new transform for `object`: this script's own, or another object's, applied after the call.
    fn set(&mut self, object: StableId, values: [f32; 9], position: [f64; 3]) {
        if object == self.object {
            self.transform = values;
            self.position = position;
            self.transform_changed = true;
        } else {
            self.writes.push((object, values, position));
        }
    }
}

/// What a script says about itself (`bayleaf_describe`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Description {
    /// Properties the editor shows, with their types and defaults.
    pub properties: Vec<DeclaredProperty>,
    /// Where Add Component lists it: folders, then its name (`Light/Flicker`).
    pub menu: Option<String>,
    /// How its properties show in the Inspector; empty for one row each, in declaration order.
    pub inspector: Vec<bayleaf_scene::inspector::InspectorItem>,
    /// Events it declares, which other scripts and the interface can trigger on its object.
    pub events: Vec<DeclaredEvent>,
}

/// An event a script declares: its name and its values, each `name:type` (`amount:number`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeclaredEvent {
    pub name: String,
    pub params: Vec<String>,
}

/// Most events a description lists.
pub const MAX_DECLARED_EVENTS: usize = 64;

/// Most folders in a menu path, and longest part.
const MAX_MENU_DEPTH: usize = 4;
const MAX_MENU_PART: usize = 64;

/// A menu path as written: 1 to 4 parts separated by `/`, each 1 to 64 characters with no control
/// characters, spaces around parts trimmed.
pub fn menu_path(text: &str) -> Result<String, String> {
    let parts: Vec<&str> = text.split('/').map(str::trim).collect();
    let fine = parts.len() <= MAX_MENU_DEPTH && parts.iter().all(|p| !p.is_empty() && p.chars().count() <= MAX_MENU_PART && !p.chars().any(char::is_control));
    if fine {
        Ok(parts.join("/"))
    } else {
        Err(format!("menu {text:?} needs 1 to {MAX_MENU_DEPTH} names of 1 to {MAX_MENU_PART} characters separated by / (Light/Flicker)"))
    }
}

/// What a ray can hit on one object, in the space of its pose.
#[derive(Clone, Copy, Debug)]
enum RayShape {
    Box(DVec3, DVec3),
    Sphere(f64),
    /// Upright: half the straight part's height, and the radius.
    Capsule(f64, f64),
}

impl RayShape {
    /// The smallest parameter `t >= 0` where the ray `o + t d` meets the shape.
    fn hit(self, o: DVec3, d: DVec3) -> Option<f64> {
        match self {
            RayShape::Box(lo, hi) => {
                let (mut near, mut far) = (0.0f64, f64::INFINITY);
                for axis in 0..3 {
                    if d[axis].abs() < 1e-12 {
                        if o[axis] < lo[axis] || o[axis] > hi[axis] {
                            return None;
                        }
                    } else {
                        let (a, b) = ((lo[axis] - o[axis]) / d[axis], (hi[axis] - o[axis]) / d[axis]);
                        near = near.max(a.min(b));
                        far = far.min(a.max(b));
                    }
                }
                (near <= far).then_some(near)
            }
            RayShape::Sphere(r) => sphere_hit(o, d, DVec3::ZERO, r),
            RayShape::Capsule(half, r) => {
                // The side (an upright cylinder between the ends), then the two end spheres.
                let mut best = [sphere_hit(o, d, DVec3::Y * half, r), sphere_hit(o, d, -DVec3::Y * half, r)]
                    .into_iter()
                    .flatten()
                    .fold(None, |a: Option<f64>, t| Some(a.map_or(t, |a| a.min(t))));
                let (a, b, c) = (d.x * d.x + d.z * d.z, o.x * d.x + o.z * d.z, o.x * o.x + o.z * o.z - r * r);
                if a > 1e-12 && b * b - a * c >= 0.0 {
                    let s = (b * b - a * c).sqrt();
                    for t in [(-b - s) / a, (-b + s) / a] {
                        if t >= 0.0 && (o.y + t * d.y).abs() <= half {
                            best = Some(best.map_or(t, |x| x.min(t)));
                            break;
                        }
                    }
                } else if c <= 0.0 && o.y.abs() <= half {
                    best = Some(0.0);
                }
                best
            }
        }
    }
}

fn sphere_hit(o: DVec3, d: DVec3, center: DVec3, r: f64) -> Option<f64> {
    let oc = o - center;
    let (a, b, c) = (d.length_squared(), oc.dot(d), oc.length_squared() - r * r);
    if c <= 0.0 {
        return Some(0.0);
    }
    let disc = b * b - a * c;
    if disc < 0.0 || a < 1e-12 {
        return None;
    }
    let t = (-b - disc.sqrt()) / a;
    (t >= 0.0).then_some(t)
}

/// What a ray hits on an object, and the pose it is in: a character's capsule or a collider's
/// shape (triggers are left out, as physics leaves them out), else its mesh shape or sprite.
/// Colliders are sized as physics sizes them: by the object's world scale, without skewing.
fn ray_shape(scene: &Scene, id: StableId, world: DMat4) -> Option<(DMat4, RayShape)> {
    use bayleaf_scene::components::{CharacterBody, Collider, ColliderShape, Mesh, Shape, Sprite};
    let (scale, rotation, translation) = world.to_scale_rotation_translation();
    let unscaled = DMat4::from_rotation_translation(rotation, translation);
    let scale = scale.abs();
    if let Some(c) = scene.get::<CharacterBody>(id) {
        let r = f64::from(c.radius);
        return Some((unscaled, RayShape::Capsule((f64::from(c.height) * 0.5 - r).max(0.0), r)));
    }
    if let Some(c) = scene.get::<Collider>(id) {
        if c.trigger {
            return None;
        }
        let size = DVec3::from(c.size.map(f64::from)) * scale;
        return Some(match c.shape {
            ColliderShape::Box => (unscaled, RayShape::Box(-size * 0.5, size * 0.5)),
            ColliderShape::Sphere => (unscaled, RayShape::Sphere(size.max_element() * 0.5)),
            ColliderShape::Capsule => {
                let r = size.x.max(size.z) * 0.5;
                (unscaled, RayShape::Capsule((size.y * 0.5 - r).max(0.0), r))
            }
        });
    }
    let half = if let Some(mesh) = scene.get::<Mesh>(id) {
        // A mesh with a model counts as its shape's unit box; models do not keep their exact size here.
        match mesh.shape {
            Shape::Plane => DVec3::new(0.5, 0.001, 0.5),
            Shape::Quad => DVec3::new(0.5, 0.5, 0.001),
            Shape::Cube | Shape::Sphere => DVec3::splat(0.5),
        }
    } else {
        let sprite = scene.get::<Sprite>(id)?;
        DVec3::new(f64::from(sprite.size[0]) * 0.5, f64::from(sprite.size[1]) * 0.5, 0.001)
    };
    Some((world, RayShape::Box(-half, half)))
}
/// The transform as the 9 numbers of the ABI (the position narrowed to single precision).
fn read_transform(scene: &Scene, object: StableId) -> [f32; 9] {
    let t = scene.get::<Transform>(object).cloned().unwrap_or_default();
    let p = t.position.map(|v| v as f32);
    [p[0], p[1], p[2], t.rotation[0], t.rotation[1], t.rotation[2], t.scale[0], t.scale[1], t.scale[2]]
}

fn read_position(scene: &Scene, object: StableId) -> [f64; 3] {
    scene.get::<Transform>(object).map_or([0.0; 3], |t| t.position)
}

/// The precise position after a script set the single-precision transform `v`: unchanged when the
/// script left the position as it read it, so far-away objects do not snap to the nearest `f32`.
fn merged_position(v: &[f32; 9], precise: [f64; 3]) -> [f64; 3] {
    let given = [v[0], v[1], v[2]];
    if given == precise.map(|p| p as f32) { precise } else { given.map(f64::from) }
}

fn transform_from(v: &[f32; 9], position: [f64; 3]) -> Transform {
    Transform { position, rotation: [v[3], v[4], v[5]], scale: [v[6], v[7], v[8]] }
}

#[cfg(feature = "compiled")]
mod compiled;
mod interpreted;
mod runtime;
pub use runtime::{Runtime, ScriptHost};
