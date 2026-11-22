//! L4: WebAssembly script host. Implements the ABI in `docs/script-abi.md`.
//!
//! Each `script` component gets its own WebAssembly instance (wasmi, a pure-Rust interpreter that
//! also runs inside the web build). Every call is metered with fuel and memory is capped, so a
//! broken script stops itself, not the editor.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use bayleaf_core::StableId;
use bayleaf_core::math::{DMat4, DVec3, Vec3};
use bayleaf_scene::Scene;
use bayleaf_scene::components::{DeclaredProperty, PropertyType, PropertyValue};
use bayleaf_scene::components::{ScriptRef, Scripts, Transform};
use wasmi::{Caller, Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc};

pub mod actions;
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
        let bytes = std::fs::read(path).map_err(read_error)?;
        // An exported game keeps its visual scripts already compiled.
        if bytes.starts_with(WASM_MAGIC) {
            return Ok(bytes);
        }
        let text = String::from_utf8(bytes).map_err(|e| ScriptError::Invalid(name.clone(), e.to_string()))?;
        let graph = bayleaf_graph::Graph::from_json(&text, &visual::CATALOG).map_err(|e| ScriptError::Invalid(name.clone(), e.to_string()))?;
        return visual::compile(&graph).map_err(|p| ScriptError::Invalid(name, bayleaf_graph::summary(&p)));
    }
    std::fs::read(path).map_err(read_error)
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
    /// Mouse buttons held: bit 0 left, bit 1 right, bit 2 middle.
    pub buttons: u8,
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

/// A message one script sent, delivered after the step to the scripts of `to` (`None`: every
/// object) that export `bayleaf_message`.
#[derive(Clone, Debug, PartialEq)]
struct Message {
    to: Option<StableId>,
    name: String,
    value: f32,
}

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
        Arc::new(Self { objects, shapes, positions, players, kinds, fields, scripts: scripts.unwrap_or_default() })
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
struct HostState {
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
    velocities: Arc<Velocities>,
    grounded: Arc<HashSet<StableId>>,
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
    limits: StoreLimits,
}

impl HostState {
    fn new(object: StableId, transform: [f32; 9], properties: Properties, time: f64, input: &Arc<Input>) -> Self {
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
            velocities: Arc::new(Velocities::new()),
            grounded: Arc::new(HashSet::new()),
            input_maps: actions::SharedInput::default(),
            state: SharedState::default(),
            transform,
            transform_changed: false,
            properties,
            time,
            input: Arc::clone(input),
            logs: Vec::new(),
            fault: None,
            limits: StoreLimitsBuilder::new().memory_size(MEMORY_LIMIT).instances(1).build(),
        }
    }
}

impl HostState {
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

/// Calls the optional `bayleaf_describe` export and parses the declared properties.
/// The export returns `(ptr << 32) | len` of UTF-8 JSON: `{"properties": [["speed", 90.0], ["label",
/// "Hi", "text"], ...]}`: each property a name, a default and its type (`number` when left out).
fn describe_instance(store: &mut Store<HostState>, instance: &wasmi::Instance, name: &str) -> Result<Description, ScriptError> {
    let Ok(describe) = instance.get_typed_func::<(), i64>(&*store, "bayleaf_describe") else {
        return Ok(Description::default());
    };
    let invalid = |message: String| ScriptError::Invalid(name.to_owned(), format!("bayleaf_describe: {message}"));
    store.set_fuel(FUEL_PER_CALL).map_err(|e| invalid(e.to_string()))?;
    let packed = describe.call(&mut *store, ()).map_err(|e| ScriptError::Trapped { module: name.to_owned(), message: e.to_string() })? as u64;
    let (ptr, len) = ((packed >> 32) as u32, packed as u32);
    if len > MAX_DESCRIPTION_BYTES {
        return Err(invalid(format!("description of {len} bytes is longer than {MAX_DESCRIPTION_BYTES}")));
    }
    let memory = instance.get_export(&*store, "memory").and_then(wasmi::Extern::into_memory).ok_or_else(|| invalid("no memory export".to_owned()))?;
    let mut bytes = vec![0; len as usize];
    memory.read(&*store, ptr as usize, &mut bytes).map_err(|_| invalid("description is outside the script's memory".to_owned()))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))?;
    let mut out = Vec::new();
    for entry in value.get("properties").and_then(serde_json::Value::as_array).map_or(&[][..], Vec::as_slice) {
        let parts = entry.as_array().filter(|p| (2..=3).contains(&p.len()));
        let Some(parts) = parts else {
            return Err(invalid(format!("each property must be [name, default] or [name, default, type], got {entry}")));
        };
        let key = parts[0].as_str().unwrap_or_default();
        if key.is_empty() || key.len() > 64 || key.contains(':') {
            return Err(invalid(format!("property {key:?} needs a name of 1 to 64 bytes without ':'")));
        }
        let ty = match parts.get(2) {
            None => PropertyType::Number,
            Some(code) => code.as_str().and_then(PropertyType::from_code).ok_or_else(|| invalid(format!("property {key:?} has an unknown type {code}")))?,
        };
        let default = serde_json::from_value::<PropertyValue>(parts[1].clone()).ok().filter(|v| v.fits(ty));
        let Some(default) = default else {
            return Err(invalid(format!("property {key:?}: the default {} is not a {}", parts[1], ty.label())));
        };
        if out.iter().any(|p: &DeclaredProperty| p.name == key) {
            return Err(invalid(format!("property {key:?} is declared twice")));
        }
        out.push(DeclaredProperty { name: key.to_owned(), ty, default });
    }
    let menu = match value.get("menu").and_then(serde_json::Value::as_str) {
        Some(text) => Some(menu_path(text).map_err(invalid)?),
        None => None,
    };
    Ok(Description { properties: out, menu })
}

/// What a script says about itself (`bayleaf_describe`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Description {
    /// Properties the editor shows, with their types and defaults.
    pub properties: Vec<DeclaredProperty>,
    /// Where Add Component lists it: folders, then its name (`Light/Flicker`).
    pub menu: Option<String>,
}

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

struct Instance {
    object: StableId,
    /// Position in the object's `scripts` list.
    index: usize,
    script_name: String,
    path: PathBuf,
    module_name: String,
    modified: Option<SystemTime>,
    store: Store<HostState>,
    update: Option<TypedFunc<f32, ()>>,
    /// `bayleaf_message(value)`, if the module takes messages.
    on_message: Option<TypedFunc<f32, ()>>,
    /// Stopped after an error; it no longer runs.
    stopped: bool,
    /// `bayleaf_init`, until it has run. Scripts loaded together all init once all of them are
    /// loaded, then all start, so each one sees the others (their properties, as components).
    pending_init: Option<TypedFunc<(), ()>>,
    /// `bayleaf_start`, until it has run (after every script loaded with it has run its init).
    pending_start: Option<TypedFunc<(), ()>>,
}

/// All script instances of one play session.
pub struct ScriptHost {
    engine: Engine,
    linker: Linker<HostState>,
    instances: Vec<Instance>,
    faults: Vec<ScriptFault>,
    logs: Vec<LogLine>,
    requests: Vec<Request>,
    /// Messages sent and not yet delivered.
    messages: Vec<Message>,
    velocities: Arc<Velocities>,
    grounded: Arc<HashSet<StableId>>,
    input_maps: actions::SharedInput,
    state: SharedState,
    /// A loaded script reads or sets component fields, so each step takes their values along.
    reads_fields: bool,
    /// Script properties set during the step, applied when it ends.
    property_writes: Vec<PropertyWrite>,
}

impl Default for ScriptHost {
    fn default() -> Self {
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let linker = make_linker(&engine);
        Self {
            engine,
            linker,
            instances: Vec::new(),
            faults: Vec::new(),
            logs: Vec::new(),
            requests: Vec::new(),
            messages: Vec::new(),
            velocities: Arc::new(Velocities::new()),
            grounded: Arc::new(HashSet::new()),
            input_maps: actions::SharedInput::default(),
            state: SharedState::default(),
            reads_fields: false,
            property_writes: Vec::new(),
        }
    }
}

impl ScriptHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads every script of every object. Module paths are relative to `base_dir`. Problems are
    /// recorded as faults; the other scripts still load.
    pub fn load_scene(&mut self, scene: &mut Scene, base_dir: &Path, time: f64, input: &Arc<Input>) {
        let objects: Vec<StableId> = scene.world_matrices().into_iter().map(|(id, _)| id).collect();
        for object in objects {
            let scripts = scene.get::<Scripts>(object).cloned().unwrap_or_default();
            for (index, script) in scripts.0.iter().enumerate() {
                let path = base_dir.join(&script.module);
                if let Err(error) = self.instantiate_file(object, index, &path, script, scene, time, input) {
                    self.faults.push(ScriptFault { object, script: script.display_name(), error });
                }
            }
        }
        // Every script of the scene is loaded before any starts.
        for (object, index, error) in self.run_starts(scene, time, input) {
            let script = scene.get::<Scripts>(object).and_then(|s| s.0.get(index).map(ScriptRef::display_name)).unwrap_or_default();
            self.faults.push(ScriptFault { object, script, error });
        }
    }

    /// Loads several module files (the scripts of a placed prefab), then runs their inits and
    /// starts together, as `load_scene` does. Returns the scripts that could not load or start.
    pub fn load_files(
        &mut self,
        scripts: &[(StableId, usize, PathBuf, ScriptRef)],
        scene: &mut Scene,
        time: f64,
        input: &Arc<Input>,
    ) -> Vec<(StableId, usize, ScriptError)> {
        let mut failed = Vec::new();
        for (object, index, path, script) in scripts {
            if let Err(error) = self.instantiate_file(*object, *index, path, script, scene, time, input) {
                failed.push((*object, *index, error));
            }
        }
        failed.extend(self.run_starts(scene, time, input));
        failed
    }

    /// Loads one module file as script `index` of `object` and runs its `bayleaf_start`.
    #[allow(clippy::too_many_arguments)]
    pub fn load_file(
        &mut self,
        object: StableId,
        index: usize,
        path: &Path,
        script: &ScriptRef,
        scene: &mut Scene,
        time: f64,
        input: &Arc<Input>,
    ) -> Result<(), ScriptError> {
        let bytes = module_bytes(path)?;
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        self.load_bytes(object, index, path, modified, &bytes, script, scene, time, input)
    }

    /// Loads one module file without running its `bayleaf_start` (see `run_starts`).
    #[allow(clippy::too_many_arguments)]
    fn instantiate_file(
        &mut self,
        object: StableId,
        index: usize,
        path: &Path,
        script: &ScriptRef,
        scene: &mut Scene,
        time: f64,
        input: &Arc<Input>,
    ) -> Result<(), ScriptError> {
        let bytes = module_bytes(path)?;
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        self.instantiate(object, index, path, modified, &bytes, script, scene, time, input)
    }

    /// The properties a module declares, with their defaults, in declaration order. Runs no script
    /// code but `bayleaf_describe`; a module without it declares nothing.
    pub fn describe(&self, name: &str, bytes: &[u8]) -> Result<Vec<DeclaredProperty>, ScriptError> {
        self.describe_all(name, bytes).map(|d| d.properties)
    }

    /// What a module says about itself: its properties and where Add Component lists it.
    pub fn describe_all(&self, name: &str, bytes: &[u8]) -> Result<Description, ScriptError> {
        let module = Module::new(&self.engine, bytes).map_err(|e| ScriptError::Invalid(name.to_owned(), e.to_string()))?;
        let mut store = Store::new(&self.engine, HostState::new(StableId::from_u128(0), [0.0; 9], BTreeMap::new(), 0.0, &Arc::new(Input::default())));
        store.limiter(|state| &mut state.limits);
        store.set_fuel(FUEL_PER_CALL).map_err(|e| ScriptError::Trapped { module: name.to_owned(), message: e.to_string() })?;
        let instance =
            self.linker.instantiate_and_start(&mut store, &module).map_err(|e| ScriptError::Trapped { module: name.to_owned(), message: e.to_string() })?;
        describe_instance(&mut store, &instance, name)
    }

    /// Loads a module from memory and runs its `bayleaf_start`; `path` names it in messages and for
    /// hot reload.
    #[allow(clippy::too_many_arguments)]
    fn load_bytes(
        &mut self,
        object: StableId,
        index: usize,
        path: &Path,
        modified: Option<SystemTime>,
        bytes: &[u8],
        script: &ScriptRef,
        scene: &mut Scene,
        time: f64,
        input: &Arc<Input>,
    ) -> Result<(), ScriptError> {
        self.instantiate(object, index, path, modified, bytes, script, scene, time, input)?;
        let mut result = Ok(());
        for (o, i, error) in self.run_starts(scene, time, input) {
            if (o, i) == (object, index) {
                result = Err(error);
            } else {
                let script = self.instances.iter().find(|e| e.object == o && e.index == i).map(|e| e.script_name.clone()).unwrap_or_default();
                self.faults.push(ScriptFault { object: o, script, error });
            }
        }
        result
    }

    /// Loads a module from memory without running its `bayleaf_start`.
    #[allow(clippy::too_many_arguments)]
    fn instantiate(
        &mut self,
        object: StableId,
        index: usize,
        path: &Path,
        modified: Option<SystemTime>,
        bytes: &[u8],
        script: &ScriptRef,
        scene: &mut Scene,
        time: f64,
        input: &Arc<Input>,
    ) -> Result<(), ScriptError> {
        let module_name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        let module = Module::new(&self.engine, bytes).map_err(|e| ScriptError::Invalid(module_name.clone(), e.to_string()))?;
        self.reads_fields |= module.imports().any(|i| matches!(i.name(), "component_get" | "component_set"));
        let mut state = HostState::new(object, read_transform(scene, object), Properties::new(), time, input);
        state.position = read_position(scene, object);
        state.state = Arc::clone(&self.state);
        state.input_maps = Arc::clone(&self.input_maps);
        state.world = WorldView::of(scene, self.script_values());
        let mut store = Store::new(&self.engine, state);
        store.limiter(|state| &mut state.limits);
        let trapped = |module: &str, e: wasmi::Error| ScriptError::Trapped { module: module.to_owned(), message: e.to_string() };
        store.set_fuel(FUEL_PER_CALL).map_err(|e| trapped(&module_name, e))?;
        let instance = self.linker.instantiate_and_start(&mut store, &module).map_err(|e| trapped(&module_name, e))?;

        let abi = instance
            .get_typed_func::<(), i32>(&store, "bayleaf_abi")
            .map_err(|_| ScriptError::Invalid(module_name.clone(), "it has no bayleaf_abi export".to_owned()))?;
        let found = abi.call(&mut store, ()).map_err(|e| trapped(&module_name, e))?;
        if found != ABI_VERSION {
            return Err(ScriptError::AbiMismatch { module: module_name, found, expected: ABI_VERSION });
        }
        if instance.get_export(&store, "memory").and_then(wasmi::Extern::into_memory).is_none() {
            return Err(ScriptError::Invalid(module_name, "it exports no memory".to_owned()));
        }
        // Declared defaults, overridden by the values saved in the scene that fit the declared type.
        // A saved value the script does not declare is still there to read, typed by its shape.
        let mut properties: Properties =
            describe_instance(&mut store, &instance, &module_name)?.properties.into_iter().map(|p| (p.name, (p.ty, p.default))).collect();
        for (name, value) in &script.properties {
            match properties.get_mut(name) {
                Some((ty, slot)) if value.fits(*ty) => *slot = value.clone(),
                Some(_) => {}
                None => {
                    properties.insert(name.clone(), (value.shape_type(), value.clone()));
                }
            }
        }
        store.data_mut().properties = properties;
        let update = instance.get_typed_func::<f32, ()>(&store, "bayleaf_update").ok();
        let on_message = instance.get_typed_func::<f32, ()>(&store, "bayleaf_message").ok();
        let init = instance.get_typed_func::<(), ()>(&store, "bayleaf_init").ok();
        let start = instance.get_typed_func::<(), ()>(&store, "bayleaf_start").ok();

        // Replace an earlier instance in the same slot (hot reload).
        self.instances.retain(|i| !(i.object == object && i.index == index));
        let script_name = script.display_name();
        self.instances.push(Instance {
            object,
            index,
            script_name,
            path: path.to_path_buf(),
            module_name,
            modified,
            store,
            update,
            on_message,
            stopped: false,
            pending_init: init,
            pending_start: start,
        });
        Ok(())
    }

    /// Runs `bayleaf_init` of every script loaded and not initialised yet, then `bayleaf_start` of
    /// every script not started yet, each phase in load order and seeing all loaded scripts as the
    /// phase began. Returns the scripts whose init or start failed (they are stopped).
    fn run_starts(&mut self, scene: &mut Scene, time: f64, input: &Arc<Input>) -> Vec<(StableId, usize, ScriptError)> {
        let mut failed = self.run_phase(scene, time, input, |entry| entry.pending_init.take());
        failed.extend(self.run_phase(scene, time, input, |entry| entry.pending_start.take()));
        failed
    }

    /// Runs the function `take` hands out for each running script.
    fn run_phase(
        &mut self,
        scene: &mut Scene,
        time: f64,
        input: &Arc<Input>,
        take: impl Fn(&mut Instance) -> Option<TypedFunc<(), ()>>,
    ) -> Vec<(StableId, usize, ScriptError)> {
        let world = WorldView::of(scene, self.script_values());
        let mut failed = Vec::new();
        let mut logs = Vec::new();
        for entry in self.instances.iter_mut().filter(|e| !e.stopped) {
            let Some(start) = take(entry) else { continue };
            {
                let state = entry.store.data_mut();
                state.world = Arc::clone(&world);
                state.transform = read_transform(scene, entry.object);
                state.position = read_position(scene, entry.object);
                state.transform_changed = false;
                state.time = time;
                state.input = Arc::clone(input);
            }
            let result = call(entry, |store| start.call(store, ()));
            let script = entry.script_name.clone();
            logs.extend(entry.store.data_mut().logs.drain(..).map(|text| LogLine { object: entry.object, script: script.clone(), text }));
            after_call(scene, entry, &mut self.requests, &mut self.messages, &mut self.property_writes);
            if let Err(error) = result {
                entry.stopped = true;
                failed.push((entry.object, entry.index, error));
            }
        }
        self.push_logs(logs);
        self.apply_property_writes();
        failed
    }

    /// Runs `bayleaf_update(dt)` on every running script and writes changed transforms back.
    pub fn update(&mut self, scene: &mut Scene, dt: f32, time: f64, input: &Arc<Input>) {
        let mut faults = Vec::new();
        let mut logs = Vec::new();
        // Every script sees the objects as the step began.
        let world = WorldView::of(scene, self.script_values());
        for entry in self.instances.iter_mut().filter(|i| !i.stopped) {
            if !scene.contains(entry.object) {
                entry.stopped = true;
                continue;
            }
            let Some(update) = entry.update else { continue };
            {
                let state = entry.store.data_mut();
                state.transform = read_transform(scene, entry.object);
                state.position = read_position(scene, entry.object);
                state.transform_changed = false;
                state.time = time;
                state.input = Arc::clone(input);
                state.world = Arc::clone(&world);
                state.velocities = Arc::clone(&self.velocities);
                state.grounded = Arc::clone(&self.grounded);
                state.input_maps = Arc::clone(&self.input_maps);
                state.state = Arc::clone(&self.state);
            }
            let result = call(entry, |store| update.call(store, dt));
            let script = entry.script_name.clone();
            logs.extend(entry.store.data_mut().logs.drain(..).map(|text| LogLine { object: entry.object, script: script.clone(), text }));
            match result {
                Ok(()) => after_call(scene, entry, &mut self.requests, &mut self.messages, &mut self.property_writes),
                Err(error) => {
                    entry.stopped = true;
                    faults.push(ScriptFault { object: entry.object, script: entry.script_name.clone(), error });
                }
            }
        }
        self.faults.extend(faults);
        self.push_logs(logs);
        self.apply_property_writes();
        self.deliver(scene, time, input);
        self.apply_property_writes();
    }

    /// Each script's properties, by object, when scripts read or set component fields.
    fn script_values(&self) -> Option<HashMap<StableId, Vec<ScriptValues>>> {
        if !self.reads_fields {
            return None;
        }
        let mut out: HashMap<StableId, Vec<ScriptValues>> = HashMap::new();
        for entry in self.instances.iter().filter(|i| !i.stopped) {
            let values = ScriptValues { keys: script_keys(&entry.script_name, &entry.path), properties: entry.store.data().properties.clone() };
            out.entry(entry.object).or_default().push(values);
        }
        Some(out)
    }

    /// Sets the script properties scripts asked for: on every script of the object with that name
    /// that declares the property.
    fn apply_property_writes(&mut self) {
        for write in std::mem::take(&mut self.property_writes) {
            for entry in self.instances.iter_mut().filter(|i| i.object == write.object && script_keys(&i.script_name, &i.path).contains(&write.key)) {
                if let Some((ty, slot)) = entry.store.data_mut().properties.get_mut(&write.name)
                    && let Some(value) = PropertyValue::from_numbers(*ty, write.value)
                {
                    *slot = value;
                }
            }
        }
    }

    /// Delivers the messages sent so far, in the order sent. Messages sent while handling one wait
    /// for the next step, so two scripts answering each other cannot run forever.
    fn deliver(&mut self, scene: &mut Scene, time: f64, input: &Arc<Input>) {
        let messages = std::mem::take(&mut self.messages);
        if messages.is_empty() {
            return;
        }
        let world = WorldView::of(scene, self.script_values());
        let mut later = Vec::new();
        let mut faults = Vec::new();
        let mut logs = Vec::new();
        for message in messages {
            for entry in self.instances.iter_mut().filter(|i| !i.stopped && message.to.is_none_or(|to| to == i.object)) {
                let Some(on_message) = entry.on_message else { continue };
                if !scene.contains(entry.object) {
                    continue;
                }
                {
                    let state = entry.store.data_mut();
                    state.transform = read_transform(scene, entry.object);
                    state.position = read_position(scene, entry.object);
                    state.transform_changed = false;
                    state.time = time;
                    state.input = Arc::clone(input);
                    state.world = Arc::clone(&world);
                    state.velocities = Arc::clone(&self.velocities);
                    state.grounded = Arc::clone(&self.grounded);
                    state.input_maps = Arc::clone(&self.input_maps);
                    state.state = Arc::clone(&self.state);
                    state.message = Some(message.name.clone());
                }
                let result = call(entry, |store| on_message.call(store, message.value));
                entry.store.data_mut().message = None;
                let script = entry.script_name.clone();
                logs.extend(entry.store.data_mut().logs.drain(..).map(|text| LogLine { object: entry.object, script: script.clone(), text }));
                match result {
                    Ok(()) => after_call(scene, entry, &mut self.requests, &mut later, &mut self.property_writes),
                    Err(error) => {
                        entry.stopped = true;
                        faults.push(ScriptFault { object: entry.object, script: entry.script_name.clone(), error });
                    }
                }
            }
        }
        self.messages = later;
        self.faults.extend(faults);
        self.push_logs(logs);
    }

    /// Reloads modules whose file changed since it was loaded. Returns the reloaded module names.
    pub fn reload_changed(&mut self, scene: &mut Scene, time: f64, input: &Arc<Input>) -> Vec<String> {
        let changed: Vec<(StableId, usize, PathBuf)> = self
            .instances
            .iter()
            .filter(|i| {
                let now = std::fs::metadata(&i.path).and_then(|m| m.modified()).ok();
                now.is_some() && now != i.modified
            })
            .map(|i| (i.object, i.index, i.path.clone()))
            .collect();
        let mut reloaded = Vec::new();
        for (object, index, path) in changed {
            let Some(script) = scene.get::<Scripts>(object).and_then(|s| s.0.get(index)).cloned() else { continue };
            match self.load_file(object, index, &path, &script, scene, time, input) {
                Ok(()) => reloaded.push(path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())),
                Err(error) => {
                    // Keep the old module's timestamp current so a broken file is reported once.
                    if let Some(entry) = self.instances.iter_mut().find(|i| i.object == object && i.index == index) {
                        entry.modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                    }
                    self.faults.push(ScriptFault { object, script: script.display_name(), error });
                }
            }
        }
        reloaded.sort();
        reloaded.dedup();
        reloaded
    }

    /// The physics bodies' velocities scripts read in the next calls.
    pub fn set_velocities(&mut self, velocities: Velocities) {
        self.velocities = Arc::new(velocities);
    }

    /// The game and player state scripts share (empty at the start of play).
    pub fn state(&self) -> GameState {
        lock(&self.state).clone()
    }

    /// Changes the shared state from outside the scripts (commands, game rules).
    pub fn with_state<T>(&self, change: impl FnOnce(&mut GameState) -> T) -> T {
        change(&mut lock(&self.state))
    }

    /// Sends a message to every script now, as `notify` does for one object.
    pub fn broadcast(&mut self, scene: &mut Scene, name: &str, value: f32, time: f64, input: &Arc<Input>) {
        self.messages.push(Message { to: None, name: name.to_owned(), value });
        self.deliver(scene, time, input);
    }

    /// The input maps scripts read with `action` and switch with `input_map_set`: the project's and
    /// objects' own. The engine fills them when play starts and steps them each step.
    pub fn with_input<T>(&self, change: impl FnOnce(&mut actions::InputSet) -> T) -> T {
        change(&mut actions::lock(&self.input_maps))
    }

    /// The characters standing on something, as scripts see them in the next calls.
    pub fn set_grounded(&mut self, grounded: impl IntoIterator<Item = StableId>) {
        self.grounded = Arc::new(grounded.into_iter().collect());
    }

    /// Tells scripts about physics contacts: each `(object, name, other)` becomes a message called
    /// `name` to `object`'s scripts, its value the other object's handle, delivered now.
    pub fn notify(&mut self, scene: &mut Scene, contacts: &[(StableId, &str, StableId)], time: f64, input: &Arc<Input>) {
        if contacts.is_empty() {
            return;
        }
        let world = WorldView::of(scene, self.script_values());
        for (object, name, other) in contacts {
            self.messages.push(Message { to: Some(*object), name: (*name).to_owned(), value: world.handle_of(*other) as f32 });
        }
        self.deliver(scene, time, input);
    }

    /// Spawns and destroys scripts asked for since the last call, in order.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    /// Number of loaded instances, running or stopped.
    pub fn len(&self) -> usize {
        self.instances.len()
    }

    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    pub fn running(&self) -> usize {
        self.instances.iter().filter(|i| !i.stopped).count()
    }

    /// Faults recorded since the last call.
    pub fn take_faults(&mut self) -> Vec<ScriptFault> {
        std::mem::take(&mut self.faults)
    }

    /// Log lines recorded since the last call.
    pub fn take_logs(&mut self) -> Vec<LogLine> {
        std::mem::take(&mut self.logs)
    }

    fn push_logs(&mut self, lines: Vec<LogLine>) {
        self.logs.extend(lines);
        if self.logs.len() > MAX_LOG_LINES {
            let excess = self.logs.len() - MAX_LOG_LINES;
            self.logs.drain(..excess);
        }
    }
}

/// Runs one guest call with a fresh fuel budget and turns a trap into a [`ScriptError`].
fn call(entry: &mut Instance, run: impl FnOnce(&mut Store<HostState>) -> Result<(), wasmi::Error>) -> Result<(), ScriptError> {
    let trapped = |message: String| ScriptError::Trapped { module: entry.module_name.clone(), message };
    entry.store.set_fuel(FUEL_PER_CALL).map_err(|e| trapped(e.to_string()))?;
    entry.store.data_mut().fault = None;
    match run(&mut entry.store) {
        Ok(()) => Ok(()),
        Err(error) => {
            let reason = entry.store.data_mut().fault.take().unwrap_or_else(|| match error.as_trap_code() {
                Some(wasmi::TrapCode::OutOfFuel) => format!("ran longer than the limit of {FUEL_PER_CALL} instructions in one call"),
                _ => error.to_string(),
            });
            Err(ScriptError::Trapped { module: entry.module_name.clone(), message: reason })
        }
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

/// Applies what a call changed: its own transform, other objects' transforms, and its requests.
fn after_call(scene: &mut Scene, entry: &mut Instance, requests: &mut Vec<Request>, messages: &mut Vec<Message>, properties: &mut Vec<PropertyWrite>) {
    write_transform(scene, entry);
    let state = entry.store.data_mut();
    for (object, v, position) in state.writes.drain(..) {
        if scene.contains(object) {
            let _ = scene.set(object, transform_from(&v, position));
        }
    }
    requests.append(&mut state.requests);
    messages.append(&mut state.outbox);
    properties.append(&mut state.property_writes);
}

fn write_transform(scene: &mut Scene, entry: &mut Instance) {
    let state = entry.store.data_mut();
    if !state.transform_changed {
        return;
    }
    state.transform_changed = false;
    let transform = transform_from(&state.transform, state.position);
    // `transform_set` already refused non-finite numbers, and the object exists (checked by the caller).
    let _ = scene.set(entry.object, transform);
}

/// Reads `len` bytes at `ptr` from the guest's memory.
fn read_guest(caller: &Caller<'_, HostState>, ptr: i32, len: i32, limit: usize) -> Result<Vec<u8>, String> {
    let memory = caller.get_export("memory").and_then(wasmi::Extern::into_memory).ok_or("the script exports no memory")?;
    let (Ok(ptr), Ok(len)) = (usize::try_from(ptr), usize::try_from(len)) else {
        return Err("negative pointer or length".to_owned());
    };
    if len > limit {
        return Err(format!("text of {len} bytes is longer than the limit of {limit}"));
    }
    let mut buffer = vec![0; len];
    memory.read(caller, ptr, &mut buffer).map_err(|_| format!("reading {len} bytes at {ptr} is outside the script's memory"))?;
    Ok(buffer)
}

fn read_text(caller: &Caller<'_, HostState>, ptr: i32, len: i32, limit: usize) -> Result<String, String> {
    let bytes = read_guest(caller, ptr, len, limit)?;
    String::from_utf8(bytes).map_err(|_| "text is not valid UTF-8".to_owned())
}

/// Records a guest error so the trap message explains it, and returns the trap.
/// Writes `bytes` into the guest's memory at `ptr`.
fn write_guest(caller: &mut Caller<'_, HostState>, ptr: i32, bytes: &[u8], what: &str) -> Result<(), wasmi::Error> {
    let memory = caller.get_export("memory").and_then(wasmi::Extern::into_memory).ok_or_else(|| wasmi::Error::new("no memory"))?;
    let at = usize::try_from(ptr).map_err(|_| fault(caller, format!("{what}: negative pointer")))?;
    memory.write(&mut *caller, at, bytes).map_err(|_| fault(caller, format!("{what}: writing outside the script's memory")))
}

fn fault(caller: &mut Caller<'_, HostState>, message: String) -> wasmi::Error {
    caller.data_mut().fault = Some(message.clone());
    wasmi::Error::new(message)
}

/// Records a physics request for the object with `handle`, applied after the step.
fn physics_request(
    caller: &mut Caller<'_, HostState>,
    what: &str,
    handle: i32,
    v: [f32; 3],
    make: impl FnOnce(StableId, StableId, [f32; 3]) -> Request,
) -> Result<i32, wasmi::Error> {
    if !v.iter().all(|c| c.is_finite()) {
        return Err(fault(caller, format!("{what}: give finite numbers")));
    }
    let Some((object, _, _)) = caller.data().world.get(handle).cloned() else {
        return Err(fault(caller, format!("{what}: there is no object with handle {handle}")));
    };
    let state = caller.data_mut();
    if state.requests.len() >= MAX_REQUESTS_PER_CALL {
        return Ok(0);
    }
    let by = state.object;
    state.requests.push(make(by, object, v));
    Ok(1)
}

/// Which state a call means: `None` the game's; `Some(Some(id))` a player's; `Some(None)` a
/// player that is not there (handle 0 with no player). Scope 0 is the game, 1 a player: handle 0
/// the first player, else that object.
/// The store a `state_get` or `state_set` call names; `None` when it names a player that is not
/// there.
fn state_scope(caller: &mut Caller<'_, HostState>, what: &str, scope: i32, handle: i32) -> Result<Option<Scope>, wasmi::Error> {
    match scope {
        0 => Ok(Some(Scope::Game)),
        2 => Ok(Some(Scope::Saved)),
        1 if handle == 0 => Ok(caller.data().world.players.first().copied().map(Scope::Player)),
        // -1 names no object, as a search that found nothing; it is no player.
        1 if handle == -1 => Ok(None),
        1 => match caller.data().world.object(handle) {
            // Only objects with Player Control have a player state.
            Some((id, _)) => Ok(caller.data().world.players.contains(&id).then_some(Scope::Player(id))),
            None => Err(fault(caller, format!("{what}: there is no object with handle {handle}"))),
        },
        _ => Err(fault(caller, format!("{what}: scope is 0 (the game), 1 (a player) or 2 (saved)"))),
    }
}

fn make_linker(engine: &Engine) -> Linker<HostState> {
    let mut linker = Linker::new(engine);
    let result = (|| -> Result<(), wasmi::errors::LinkerError> {
        linker.func_wrap("bayleaf", "log", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<(), wasmi::Error> {
            let text = read_text(&caller, ptr, len, MAX_LOG_BYTES).map_err(|m| fault(&mut caller, format!("log: {m}")))?;
            caller.data_mut().logs.push(text);
            Ok(())
        })?;
        linker.func_wrap("bayleaf", "transform_get", |mut caller: Caller<'_, HostState>, out: i32| -> Result<(), wasmi::Error> {
            let memory = caller.get_export("memory").and_then(wasmi::Extern::into_memory).ok_or_else(|| wasmi::Error::new("no memory"))?;
            let bytes: Vec<u8> = caller.data().transform.iter().flat_map(|v| v.to_le_bytes()).collect();
            let ptr = usize::try_from(out).map_err(|_| fault(&mut caller, "transform_get: negative pointer".to_owned()))?;
            memory.write(&mut caller, ptr, &bytes).map_err(|_| wasmi::Error::new("transform_get: writing outside the script's memory"))?;
            Ok(())
        })?;
        linker.func_wrap("bayleaf", "transform_set", |mut caller: Caller<'_, HostState>, ptr: i32| -> Result<(), wasmi::Error> {
            let bytes = read_guest(&caller, ptr, 36, 36).map_err(|m| fault(&mut caller, format!("transform_set: {m}")))?;
            let mut values = [0.0f32; 9];
            for (value, chunk) in values.iter_mut().zip(bytes.as_chunks::<4>().0) {
                *value = f32::from_le_bytes(*chunk);
            }
            if !values.iter().all(|v| v.is_finite()) {
                return Err(fault(&mut caller, "transform_set: values must be finite numbers".to_owned()));
            }
            let state = caller.data_mut();
            state.position = merged_position(&values, state.position);
            state.transform = values;
            state.transform_changed = true;
            Ok(())
        })?;
        linker.func_wrap("bayleaf", "position_get_of", |mut caller: Caller<'_, HostState>, handle: i32, out: i32| -> Result<(), wasmi::Error> {
            let Some((object, index)) = caller.data().world.object(handle) else {
                return Err(fault(&mut caller, format!("position_get_of: there is no object with handle {handle}")));
            };
            let (_, position) = caller.data().current(object, index);
            let bytes: Vec<u8> = position.iter().flat_map(|v| v.to_le_bytes()).collect();
            write_guest(&mut caller, out, &bytes, "position_get_of")
        })?;
        linker.func_wrap("bayleaf", "position_set_of", |mut caller: Caller<'_, HostState>, handle: i32, ptr: i32| -> Result<(), wasmi::Error> {
            let bytes = read_guest(&caller, ptr, 24, 24).map_err(|m| fault(&mut caller, format!("position_set_of: {m}")))?;
            let mut position = [0.0f64; 3];
            for (value, chunk) in position.iter_mut().zip(bytes.as_chunks::<8>().0) {
                *value = f64::from_le_bytes(*chunk);
            }
            if !position.iter().all(|v| v.is_finite()) {
                return Err(fault(&mut caller, "position_set_of: values must be finite numbers".to_owned()));
            }
            let Some((object, index)) = caller.data().world.object(handle) else {
                return Err(fault(&mut caller, format!("position_set_of: there is no object with handle {handle}")));
            };
            let state = caller.data_mut();
            let (mut values, _) = state.current(object, index);
            values[..3].copy_from_slice(&position.map(|v| v as f32));
            state.set(object, values, position);
            Ok(())
        })?;
        linker.func_wrap("bayleaf", "property", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<f32, wasmi::Error> {
            let key = read_text(&caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("property: {m}")))?;
            Ok(caller.data().properties.get(&key).and_then(|(_, v)| v.number()).unwrap_or(f32::NAN))
        })?;
        linker.func_wrap("bayleaf", "property_whole", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<i32, wasmi::Error> {
            let key = read_text(&caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("property_whole: {m}")))?;
            Ok(caller.data().properties.get(&key).and_then(|(_, v)| v.number()).map_or(0, |n| n.round() as i32))
        })?;
        linker.func_wrap("bayleaf", "property_vector", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32, out: i32| -> Result<i32, wasmi::Error> {
            let key = read_text(&caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("property_vector: {m}")))?;
            let numbers = caller.data().properties.get(&key).and_then(|(_, v)| v.numbers());
            let bytes: Vec<u8> = numbers.unwrap_or([0.0; 3]).iter().flat_map(|v| v.to_le_bytes()).collect();
            write_guest(&mut caller, out, &bytes, "property_vector")?;
            Ok(i32::from(numbers.is_some()))
        })?;
        linker.func_wrap(
            "bayleaf",
            "property_text",
            |mut caller: Caller<'_, HostState>, ptr: i32, len: i32, out: i32, cap: i32| -> Result<i32, wasmi::Error> {
                let key = read_text(&caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("property_text: {m}")))?;
                let text = caller.data().properties.get(&key).and_then(|(_, v)| v.text().map(str::to_owned)).unwrap_or_default();
                let cap = usize::try_from(cap).map_err(|_| fault(&mut caller, "property_text: the room must not be negative".to_owned()))?;
                let bytes = text.as_bytes();
                write_guest(&mut caller, out, &bytes[..bytes.len().min(cap)], "property_text")?;
                Ok(i32::try_from(bytes.len()).unwrap_or(i32::MAX))
            },
        )?;
        linker.func_wrap(
            "bayleaf",
            "property_set",
            |mut caller: Caller<'_, HostState>, ptr: i32, len: i32, x: f32, y: f32, z: f32| -> Result<i32, wasmi::Error> {
                let key = read_text(&caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("property_set: {m}")))?;
                if ![x, y, z].iter().all(|v| v.is_finite()) {
                    return Err(fault(&mut caller, "property_set: give finite numbers".to_owned()));
                }
                let Some((ty, slot)) = caller.data_mut().properties.get_mut(&key) else { return Ok(0) };
                match PropertyValue::from_numbers(*ty, [x, y, z]) {
                    Some(value) => {
                        *slot = value;
                        Ok(1)
                    }
                    None => Ok(0),
                }
            },
        )?;
        linker.func_wrap(
            "bayleaf",
            "property_set_text",
            |mut caller: Caller<'_, HostState>, ptr: i32, len: i32, t_ptr: i32, t_len: i32| -> Result<i32, wasmi::Error> {
                let key = read_text(&caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("property_set_text: {m}")))?;
                let text = read_text(&caller, t_ptr, t_len, bayleaf_scene::components::MAX_PROPERTY_TEXT)
                    .map_err(|m| fault(&mut caller, format!("property_set_text: {m}")))?;
                let Some((ty, slot)) = caller.data_mut().properties.get_mut(&key) else { return Ok(0) };
                let value = PropertyValue::Text(text);
                if !value.fits(*ty) {
                    return Ok(0);
                }
                *slot = value;
                Ok(1)
            },
        )?;
        linker.func_wrap("bayleaf", "property_object", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<i32, wasmi::Error> {
            let key = read_text(&caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("property_object: {m}")))?;
            let state = caller.data();
            let name = state.properties.get(&key).filter(|(ty, _)| *ty == PropertyType::Object).and_then(|(_, v)| v.text()).unwrap_or_default();
            if name.is_empty() {
                return Ok(0);
            }
            Ok(state.world.objects.iter().position(|(_, n, _)| n == name).map_or(0, |i| i as i32 + 1))
        })?;
        linker.func_wrap("bayleaf", "time", |caller: Caller<'_, HostState>| -> f64 { caller.data().time })?;
        linker.func_wrap("bayleaf", "key_down", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<i32, wasmi::Error> {
            let name = read_text(&caller, ptr, len, 64).map_err(|m| fault(&mut caller, format!("key_down: {m}")))?;
            Ok(i32::from(caller.data().input.keys.contains(&name)))
        })?;
        linker.func_wrap("bayleaf", "pointer", |mut caller: Caller<'_, HostState>, out: i32| -> Result<i32, wasmi::Error> {
            let (xy, inside) = match caller.data().input.pointer {
                Some(p) => (p, 1),
                None => ([0.0; 2], 0),
            };
            let bytes: Vec<u8> = xy.iter().flat_map(|v| v.to_le_bytes()).collect();
            write_guest(&mut caller, out, &bytes, "pointer")?;
            Ok(inside)
        })?;
        linker.func_wrap("bayleaf", "mouse_down", |caller: Caller<'_, HostState>, button: i32| -> i32 {
            match u32::try_from(button) {
                Ok(b) if b < 3 => i32::from((caller.data().input.buttons >> b) & 1),
                _ => 0,
            }
        })?;
        linker.func_wrap(
            "bayleaf",
            "apply_impulse",
            |mut caller: Caller<'_, HostState>, handle: i32, x: f32, y: f32, z: f32| -> Result<i32, wasmi::Error> {
                physics_request(&mut caller, "apply_impulse", handle, [x, y, z], |by, object, impulse| Request::Push { by, object, impulse })
            },
        )?;
        linker.func_wrap("bayleaf", "set_velocity", |mut caller: Caller<'_, HostState>, handle: i32, x: f32, y: f32, z: f32| -> Result<i32, wasmi::Error> {
            physics_request(&mut caller, "set_velocity", handle, [x, y, z], |by, object, velocity| Request::SetVelocity { by, object, velocity })
        })?;
        linker.func_wrap(
            "bayleaf",
            "state_get",
            |mut caller: Caller<'_, HostState>, scope: i32, handle: i32, ptr: i32, len: i32, out: i32| -> Result<i32, wasmi::Error> {
                let name = read_text(&caller, ptr, len, 64).map_err(|m| fault(&mut caller, format!("state_get: {m}")))?;
                let value = state_scope(&mut caller, "state_get", scope, handle)?.and_then(|scope| lock(&caller.data().state).get(scope, &name));
                write_guest(&mut caller, out, &value.unwrap_or(0.0).to_le_bytes(), "state_get")?;
                Ok(i32::from(value.is_some()))
            },
        )?;
        linker.func_wrap(
            "bayleaf",
            "state_set",
            |mut caller: Caller<'_, HostState>, scope: i32, handle: i32, ptr: i32, len: i32, value: f64| -> Result<i32, wasmi::Error> {
                let name = read_text(&caller, ptr, len, 64).map_err(|m| fault(&mut caller, format!("state_set: {m}")))?;
                let scope = state_scope(&mut caller, "state_set", scope, handle)?;
                let stored = scope.is_some_and(|scope| lock(&caller.data().state).set(scope, &name, value));
                Ok(i32::from(stored))
            },
        )?;
        linker.func_wrap("bayleaf", "game_end", |caller: Caller<'_, HostState>, won: i32| -> i32 { i32::from(lock(&caller.data().state).end(won != 0)) })?;
        linker.func_wrap("bayleaf", "action", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<f32, wasmi::Error> {
            // `map/action`: two names of up to 64 bytes and the slash.
            let name = read_text(&caller, ptr, len, 129).map_err(|m| fault(&mut caller, format!("action: {m}")))?;
            let state = caller.data();
            Ok(actions::lock(&state.input_maps).of(state.object).map_or(0.0, |maps| maps.value(&name, &state.input)))
        })?;
        linker.func_wrap("bayleaf", "input_map_set", |mut caller: Caller<'_, HostState>, handle: i32, ptr: i32, len: i32| -> Result<i32, wasmi::Error> {
            let name = read_text(&caller, ptr, len, 64).map_err(|m| fault(&mut caller, format!("input_map_set: {m}")))?;
            // 0 is the project's map; a handle an object with its own Input Map.
            let object = match handle {
                0 => None,
                _ => match caller.data().world.object(handle) {
                    Some((id, _)) => Some(id),
                    None => return Err(fault(&mut caller, format!("input_map_set: there is no object with handle {handle}"))),
                },
            };
            let switched = actions::lock(&caller.data().input_maps).get_mut(object).is_some_and(|maps| maps.set_active(&name));
            Ok(i32::from(switched))
        })?;
        linker.func_wrap(
            "bayleaf",
            "character_walk",
            |mut caller: Caller<'_, HostState>, handle: i32, x: f32, y: f32, z: f32| -> Result<i32, wasmi::Error> {
                physics_request(&mut caller, "character_walk", handle, [x, y, z], |by, object, velocity| Request::Walk { by, object, velocity })
            },
        )?;
        linker.func_wrap("bayleaf", "character_jump", |mut caller: Caller<'_, HostState>, handle: i32, speed: f32| -> Result<i32, wasmi::Error> {
            physics_request(&mut caller, "character_jump", handle, [speed, 0.0, 0.0], |by, object, v| Request::Jump { by, object, speed: v[0] })
        })?;
        linker.func_wrap("bayleaf", "character_grounded", |mut caller: Caller<'_, HostState>, handle: i32| -> Result<i32, wasmi::Error> {
            let Some((object, _)) = caller.data().world.object(handle) else {
                return Err(fault(&mut caller, format!("character_grounded: there is no object with handle {handle}")));
            };
            Ok(i32::from(caller.data().grounded.contains(&object)))
        })?;
        linker.func_wrap("bayleaf", "velocity_of", |mut caller: Caller<'_, HostState>, handle: i32, out: i32| -> Result<i32, wasmi::Error> {
            let Some((object, _, _)) = caller.data().world.get(handle).cloned() else {
                return Err(fault(&mut caller, format!("velocity_of: there is no object with handle {handle}")));
            };
            let found = caller.data().velocities.get(&object).copied();
            let bytes: Vec<u8> = found.unwrap_or([0.0; 3]).iter().flat_map(|v| v.to_le_bytes()).collect();
            write_guest(&mut caller, out, &bytes, "velocity_of")?;
            Ok(i32::from(found.is_some()))
        })?;
        linker.func_wrap(
            "bayleaf",
            "raycast",
            |mut caller: Caller<'_, HostState>, ox: f32, oy: f32, oz: f32, dx: f32, dy: f32, dz: f32, max: f32, out: i32| -> Result<i32, wasmi::Error> {
                let (origin, direction) = (Vec3::new(ox, oy, oz).as_dvec3(), Vec3::new(dx, dy, dz).as_dvec3());
                if !(origin.is_finite() && direction.is_finite() && max.is_finite()) || direction.length_squared() < 1e-12 || max < 0.0 {
                    return Err(fault(&mut caller, "raycast: give a finite origin, a direction that is not zero, and a distance of 0 or more".to_owned()));
                }
                let direction = direction.normalize();
                let state = caller.data();
                let hit = state.world.raycast(origin, direction, f64::from(max), state.object);
                let (handle, values) = match hit {
                    Some((index, distance)) => {
                        let point = (origin + direction * distance).as_vec3();
                        (index as i32 + 1, [distance as f32, point.x, point.y, point.z])
                    }
                    None => (0, [0.0; 4]),
                };
                let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
                write_guest(&mut caller, out, &bytes, "raycast")?;
                Ok(handle)
            },
        )?;
        linker.func_wrap("bayleaf", "pad_connected", |caller: Caller<'_, HostState>| -> i32 { i32::from(caller.data().input.pad.is_some()) })?;
        linker.func_wrap("bayleaf", "pad_axis", |caller: Caller<'_, HostState>, index: i32| -> f32 {
            let pad = caller.data().input.pad.unwrap_or_default();
            usize::try_from(index).ok().and_then(|i| pad.axes.get(i).copied()).unwrap_or(0.0)
        })?;
        linker.func_wrap("bayleaf", "pad_button", |caller: Caller<'_, HostState>, index: i32| -> i32 {
            let pad = caller.data().input.pad.unwrap_or_default();
            match u32::try_from(index) {
                Ok(b) if b < 32 => i32::from((pad.buttons >> b) & 1 == 1),
                _ => 0,
            }
        })?;
        linker.func_wrap("bayleaf", "self_handle", |caller: Caller<'_, HostState>| -> i32 {
            let state = caller.data();
            state.world.handle_of(state.object)
        })?;
        linker.func_wrap("bayleaf", "find", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<i32, wasmi::Error> {
            let name = read_text(&caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("find: {m}")))?;
            Ok(caller.data().world.objects.iter().position(|(_, n, _)| *n == name).map_or(0, |i| i as i32 + 1))
        })?;
        linker.func_wrap("bayleaf", "find_with", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32, index: i32| -> Result<i32, wasmi::Error> {
            let kind = read_text(&caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("find_with: {m}")))?;
            let Ok(index) = usize::try_from(index) else { return Ok(0) };
            Ok(caller.data().world.with_kind(&kind).nth(index).unwrap_or(0))
        })?;
        linker.func_wrap(
            "bayleaf",
            "component_get",
            |mut caller: Caller<'_, HostState>, handle: i32, c_ptr: i32, c_len: i32, f_ptr: i32, f_len: i32, out: i32| -> Result<i32, wasmi::Error> {
                let component = read_text(&caller, c_ptr, c_len, 256).map_err(|m| fault(&mut caller, format!("component_get: {m}")))?;
                let field = read_text(&caller, f_ptr, f_len, 256).map_err(|m| fault(&mut caller, format!("component_get: {m}")))?;
                let world = Arc::clone(&caller.data().world);
                let numbers = match readable_component(&component) {
                    Some(name) => {
                        let value = world.object(handle).and_then(|(_, index)| world.fields.get(index)?.iter().find(|(n, _)| *n == name).map(|(_, v)| v));
                        // An empty field asks only whether the object has the component.
                        match value {
                            Some(_) if field.is_empty() => Some([0.0; 3]),
                            Some(v) => v.get(&field).and_then(field_numbers),
                            None => None,
                        }
                    }
                    // A script on the object, by its name: its properties are numbers.
                    None => {
                        let key = kind_key(&component);
                        let script = world.object(handle).and_then(|(id, _)| world.scripts.get(&id)?.iter().find(|s| s.keys.contains(&key)));
                        match script {
                            Some(_) if field.is_empty() => Some([0.0; 3]),
                            Some(s) => s.properties.get(&field).and_then(|(_, v)| v.numbers()),
                            None => None,
                        }
                    }
                };
                let bytes: Vec<u8> = numbers.unwrap_or([0.0; 3]).iter().flat_map(|v| v.to_le_bytes()).collect();
                write_guest(&mut caller, out, &bytes, "component_get")?;
                Ok(i32::from(numbers.is_some()))
            },
        )?;
        linker.func_wrap(
            "bayleaf",
            "component_set",
            |mut caller: Caller<'_, HostState>, handle: i32, c_ptr: i32, c_len: i32, f_ptr: i32, f_len: i32, ptr: i32| -> Result<i32, wasmi::Error> {
                let component = read_text(&caller, c_ptr, c_len, 256).map_err(|m| fault(&mut caller, format!("component_set: {m}")))?;
                let field = read_text(&caller, f_ptr, f_len, 256).map_err(|m| fault(&mut caller, format!("component_set: {m}")))?;
                let bytes = read_guest(&caller, ptr, 12, 12).map_err(|m| fault(&mut caller, format!("component_set: {m}")))?;
                let mut value = [0.0f32; 3];
                for (slot, chunk) in value.iter_mut().zip(bytes.as_chunks::<4>().0) {
                    *slot = f32::from_le_bytes(*chunk);
                }
                let [x, y, z] = value;
                if ![x, y, z].iter().all(|v| v.is_finite()) {
                    return Err(fault(&mut caller, "component_set: give finite numbers".to_owned()));
                }
                let world = Arc::clone(&caller.data().world);
                let Some((object, index)) = world.object(handle) else { return Ok(0) };
                let Some(name) = readable_component(&component) else {
                    // A script on the object, by its name: set one of its properties after the step.
                    let key = kind_key(&component);
                    // Text and objects are not set through numbers.
                    let declares = world.scripts.get(&object).is_some_and(|scripts| {
                        scripts.iter().any(|s| {
                            s.keys.contains(&key) && s.properties.get(&field).is_some_and(|(ty, _)| PropertyValue::from_numbers(*ty, [0.0; 3]).is_some())
                        })
                    });
                    let state = caller.data_mut();
                    if !declares || state.property_writes.len() >= MAX_REQUESTS_PER_CALL {
                        return Ok(0);
                    }
                    state.property_writes.push(PropertyWrite { object, key, name: field, value: [x, y, z] });
                    return Ok(1);
                };
                if !visual::COMPONENTS.iter().any(|c| c.name == name && c.settable()) {
                    return Ok(0);
                }
                let settable = world
                    .fields
                    .get(index)
                    .and_then(|fields| fields.iter().find(|(n, _)| *n == name))
                    .and_then(|(_, v)| v.get(&field))
                    .is_some_and(|v| field_numbers(v).is_some());
                let state = caller.data_mut();
                if !settable || state.requests.len() >= MAX_REQUESTS_PER_CALL {
                    return Ok(0);
                }
                let by = state.object;
                state.requests.push(Request::SetField { by, object, component: name, field, value: [x, y, z] });
                Ok(1)
            },
        )?;
        linker.func_wrap("bayleaf", "object_ref", |mut caller: Caller<'_, HostState>, handle: i32| -> Result<i32, wasmi::Error> {
            let Some((id, _)) = caller.data().world.object(handle) else { return Ok(0) };
            let state = caller.data_mut();
            let index = match state.refs.iter().position(|r| *r == id) {
                Some(index) => index,
                None if state.refs.len() < MAX_OBJECT_REFS => {
                    state.refs.push(id);
                    state.refs.len() - 1
                }
                None => return Err(fault(&mut caller, format!("object_ref: a script keeps at most {MAX_OBJECT_REFS} objects"))),
            };
            Ok(i32::try_from(index + 1).unwrap_or(0))
        })?;
        linker.func_wrap("bayleaf", "object_of", |caller: Caller<'_, HostState>, reference: i32| -> i32 {
            let state = caller.data();
            usize::try_from(reference).ok().and_then(|r| r.checked_sub(1)).and_then(|i| state.refs.get(i)).map_or(0, |id| state.world.handle_of(*id))
        })?;
        linker.func_wrap("bayleaf", "name_of", |mut caller: Caller<'_, HostState>, handle: i32, out: i32, cap: i32| -> Result<i32, wasmi::Error> {
            let name = caller.data().world.get(handle).map(|(_, name, _)| name.clone()).unwrap_or_default();
            let cap = usize::try_from(cap).map_err(|_| fault(&mut caller, "name_of: the room must not be negative".to_owned()))?;
            let bytes = name.as_bytes();
            write_guest(&mut caller, out, &bytes[..bytes.len().min(cap)], "name_of")?;
            Ok(i32::try_from(bytes.len()).unwrap_or(i32::MAX))
        })?;
        linker.func_wrap("bayleaf", "count_with", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<i32, wasmi::Error> {
            let kind = read_text(&caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("count_with: {m}")))?;
            Ok(i32::try_from(caller.data().world.with_kind(&kind).count()).unwrap_or(i32::MAX))
        })?;
        linker.func_wrap("bayleaf", "transform_get_of", |mut caller: Caller<'_, HostState>, handle: i32, out: i32| -> Result<(), wasmi::Error> {
            let Some((object, index)) = caller.data().world.object(handle) else {
                return Err(fault(&mut caller, format!("transform_get_of: there is no object with handle {handle}")));
            };
            let (value, _) = caller.data().current(object, index);
            let bytes: Vec<u8> = value.iter().flat_map(|v| v.to_le_bytes()).collect();
            write_guest(&mut caller, out, &bytes, "transform_get_of")
        })?;
        linker.func_wrap("bayleaf", "transform_set_of", |mut caller: Caller<'_, HostState>, handle: i32, ptr: i32| -> Result<(), wasmi::Error> {
            let bytes = read_guest(&caller, ptr, 36, 36).map_err(|m| fault(&mut caller, format!("transform_set_of: {m}")))?;
            let mut values = [0.0f32; 9];
            for (value, chunk) in values.iter_mut().zip(bytes.as_chunks::<4>().0) {
                *value = f32::from_le_bytes(*chunk);
            }
            if !values.iter().all(|v| v.is_finite()) {
                return Err(fault(&mut caller, "transform_set_of: values must be finite numbers".to_owned()));
            }
            let Some((object, index)) = caller.data().world.object(handle) else {
                return Err(fault(&mut caller, format!("transform_set_of: there is no object with handle {handle}")));
            };
            let state = caller.data_mut();
            let (_, precise) = state.current(object, index);
            state.set(object, values, merged_position(&values, precise));
            Ok(())
        })?;
        linker.func_wrap("bayleaf", "spawn", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32, x: f32, y: f32, z: f32| -> Result<i32, wasmi::Error> {
            let prefab = read_text(&caller, ptr, len, 512).map_err(|m| fault(&mut caller, format!("spawn: {m}")))?;
            if !prefab.ends_with(".bprefab") || ![x, y, z].iter().all(|v| v.is_finite()) {
                return Err(fault(&mut caller, "spawn: give a .bprefab path and a finite position".to_owned()));
            }
            let state = caller.data_mut();
            if state.requests.len() >= MAX_REQUESTS_PER_CALL {
                return Ok(0);
            }
            let by = state.object;
            state.requests.push(Request::Spawn { by, prefab, position: [x, y, z] });
            Ok(1)
        })?;
        for (name, stop) in [("sound_play", false), ("sound_stop", true)] {
            linker.func_wrap("bayleaf", name, move |mut caller: Caller<'_, HostState>, handle: i32| -> Result<i32, wasmi::Error> {
                let Some((object, _, _)) = caller.data().world.get(handle).cloned() else {
                    return Err(fault(&mut caller, format!("{name}: there is no object with handle {handle}")));
                };
                let state = caller.data_mut();
                if state.requests.len() >= MAX_REQUESTS_PER_CALL {
                    return Ok(0);
                }
                let by = state.object;
                state.requests.push(if stop { Request::StopSound { by, object } } else { Request::PlaySound { by, object } });
                Ok(1)
            })?;
        }
        linker.func_wrap(
            "bayleaf",
            "sound_play_at",
            |mut caller: Caller<'_, HostState>, ptr: i32, len: i32, x: f32, y: f32, z: f32, volume: f32| -> Result<i32, wasmi::Error> {
                let path = read_text(&caller, ptr, len, 512).map_err(|m| fault(&mut caller, format!("sound_play_at: {m}")))?;
                if ![x, y, z, volume].iter().all(|v| v.is_finite()) {
                    return Err(fault(&mut caller, "sound_play_at: give a finite position and volume".to_owned()));
                }
                let state = caller.data_mut();
                if state.requests.len() >= MAX_REQUESTS_PER_CALL {
                    return Ok(0);
                }
                let by = state.object;
                state.requests.push(Request::PlaySoundAt { by, path, position: [x, y, z], volume: volume.clamp(0.0, 4.0) });
                Ok(1)
            },
        )?;
        linker.func_wrap("bayleaf", "scene_load", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32, additive: i32| -> Result<i32, wasmi::Error> {
            let path = read_text(&caller, ptr, len, 512).map_err(|m| fault(&mut caller, format!("scene_load: {m}")))?;
            if !path.ends_with(".bscene") {
                return Err(fault(&mut caller, "scene_load: give a .bscene path".to_owned()));
            }
            let state = caller.data_mut();
            if state.requests.len() >= MAX_REQUESTS_PER_CALL {
                return Ok(0);
            }
            let by = state.object;
            state.requests.push(Request::LoadScene { by, path, additive: additive != 0 });
            Ok(1)
        })?;
        linker.func_wrap("bayleaf", "scene_unload", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<i32, wasmi::Error> {
            let path = read_text(&caller, ptr, len, 512).map_err(|m| fault(&mut caller, format!("scene_unload: {m}")))?;
            let state = caller.data_mut();
            if state.requests.len() >= MAX_REQUESTS_PER_CALL {
                return Ok(0);
            }
            let by = state.object;
            state.requests.push(Request::UnloadScene { by, path });
            Ok(1)
        })?;
        linker.func_wrap("bayleaf", "destroy", |mut caller: Caller<'_, HostState>, handle: i32| -> Result<i32, wasmi::Error> {
            let Some((object, _, _)) = caller.data().world.get(handle).cloned() else {
                return Err(fault(&mut caller, format!("destroy: there is no object with handle {handle}")));
            };
            let state = caller.data_mut();
            if state.requests.len() >= MAX_REQUESTS_PER_CALL {
                return Ok(0);
            }
            state.requests.push(Request::Destroy(object));
            Ok(1)
        })?;
        linker.func_wrap("bayleaf", "send", |mut caller: Caller<'_, HostState>, handle: i32, ptr: i32, len: i32, value: f32| -> Result<i32, wasmi::Error> {
            let name = read_text(&caller, ptr, len, MAX_MESSAGE_NAME).map_err(|m| fault(&mut caller, format!("send: {m}")))?;
            if name.is_empty() || !value.is_finite() {
                return Err(fault(&mut caller, "send: give a message name and a finite number".to_owned()));
            }
            let to = if handle == EVERY_OBJECT {
                None
            } else {
                let Some((object, _, _)) = caller.data().world.get(handle).cloned() else {
                    return Err(fault(&mut caller, format!("send: there is no object with handle {handle}")));
                };
                Some(object)
            };
            let state = caller.data_mut();
            if state.outbox.len() >= MAX_MESSAGES_PER_CALL {
                return Ok(0);
            }
            state.outbox.push(Message { to, name, value });
            Ok(1)
        })?;
        linker.func_wrap("bayleaf", "message_is", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<i32, wasmi::Error> {
            let name = read_guest(&caller, ptr, len, 4096).map_err(|m| fault(&mut caller, format!("message_is: {m}")))?;
            Ok(i32::from(caller.data().message.as_ref().is_some_and(|m| m.as_bytes() == name)))
        })?;
        linker.func_wrap("bayleaf", "message_name", |mut caller: Caller<'_, HostState>, out: i32, cap: i32| -> Result<i32, wasmi::Error> {
            let Some(name) = caller.data().message.clone() else { return Ok(0) };
            let room = usize::try_from(cap).map_err(|_| fault(&mut caller, "message_name: negative length".to_owned()))?;
            let bytes = &name.as_bytes()[..name.len().min(room)];
            write_guest(&mut caller, out, bytes, "message_name")?;
            Ok(name.len() as i32)
        })?;
        Ok(())
    })();
    // The names are fixed and unique, so defining them cannot fail.
    debug_assert!(result.is_ok());
    linker
}

#[cfg(test)]
mod tests;
