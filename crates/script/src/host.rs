//! The script host, written once and compiled for each runtime: the module that includes this
//! file names its WebAssembly runtime `rt` (wasmi or wasmtime, whose interfaces match) and gives
//! the few things that differ: making the engine, instantiating, errors, traps and store limits.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use bayleaf_core::StableId;
use bayleaf_core::math::Vec3;
use bayleaf_scene::Scene;
use bayleaf_scene::components::{DeclaredProperty, PropertyShape, PropertyType, PropertyValue, ScriptRef, Scripts};

use super::rt::{self, Caller, Engine, Linker, Module, Store, TypedFunc};
use super::{Error, LinkError, error, instantiate, limits, out_of_fuel, trap_message};
#[cfg(not(target_arch = "wasm32"))]
use super::{from_val, to_val};
use crate::{
    ABI_VERSION, DeclaredEvent, Description, EVERY_OBJECT, EventArg, FUEL_PER_CALL, GameState, Input, LogLine, MAX_COLLECTION_ITEMS, MAX_DECLARED_EVENTS,
    MAX_DESCRIPTION_BYTES, MAX_EVENT_ARGS, MAX_EVENT_TEXT, MAX_LOG_BYTES, MAX_LOG_LINES, MAX_MESSAGE_NAME, MAX_MESSAGES_PER_CALL, MAX_OBJECT_REFS,
    MAX_REQUESTS_PER_CALL, MAX_STORED_COLLECTIONS, MAX_SUBSCRIPTIONS, MapKey, Message, Ordered, Properties, PropertyWrite, Request, Scope, ScriptError,
    ScriptFault, ScriptValues, SharedState, Stored, Subscription, Velocities, WorldView, actions, field_numbers, kind_key, lock, menu_path, merged_position,
    module_bytes, read_position, read_transform, readable_component, script_keys, transform_from, visual,
};
use bayleaf_script_sdk::native::NativeObject;
#[cfg(not(target_arch = "wasm32"))]
use bayleaf_script_sdk::native::{Bridge, Value};

/// What one instance's host functions see, with this runtime's store limits.
type HostState = crate::HostState<rt::StoreLimits>;

/// Calls the optional `bayleaf_describe` export and parses the declared properties.
/// The export returns `(ptr << 32) | len` of UTF-8 JSON: `{"properties": [["speed", 90.0], ["label",
/// "Hi", "text"], ...]}`: each property a name, a default and its type (`number` when left out).
fn describe_instance(store: &mut Store<HostState>, instance: &rt::Instance, name: &str) -> Result<Description, ScriptError> {
    let Ok(describe) = instance.get_typed_func::<(), i64>(&mut *store, "bayleaf_describe") else {
        return Ok(Description::default());
    };
    let invalid = |message: String| ScriptError::Invalid(name.to_owned(), format!("bayleaf_describe: {message}"));
    store.set_fuel(FUEL_PER_CALL).map_err(|e| invalid(e.to_string()))?;
    let packed = describe.call(&mut *store, ()).map_err(|e| ScriptError::Trapped { module: name.to_owned(), message: trap_message(&e) })? as u64;
    let (ptr, len) = ((packed >> 32) as u32, packed as u32);
    if len > MAX_DESCRIPTION_BYTES {
        return Err(invalid(format!("description of {len} bytes is longer than {MAX_DESCRIPTION_BYTES}")));
    }
    let memory = instance.get_export(&mut *store, "memory").and_then(rt::Extern::into_memory).ok_or_else(|| invalid("no memory export".to_owned()))?;
    let mut bytes = vec![0; len as usize];
    memory.read(&*store, ptr as usize, &mut bytes).map_err(|_| invalid("description is outside the script's memory".to_owned()))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))?;
    let out = described_properties(value.get("properties"), "property").map_err(invalid)?;
    // Lists and maps a visual script's variables start with.
    let kept = described_properties(value.get("variables"), "variable").map_err(invalid)?;
    let menu = match value.get("menu").and_then(serde_json::Value::as_str) {
        Some(text) => Some(menu_path(text).map_err(invalid)?),
        None => None,
    };
    let inspector = match value.get("inspector") {
        Some(layout) => bayleaf_scene::inspector::parse(layout, &out).map_err(|e| invalid(format!("inspector: {e}")))?,
        None => Vec::new(),
    };
    let events = described_events(value.get("events")).map_err(invalid)?;
    Ok(Description { properties: out, menu, inspector, events, kept })
}

/// Properties (or variables) a description lists: each `[name, default]` or `[name, default, type]`,
/// the type a code (`number` when left out), or a shape and a code (`list:number`, `map:text`) with
/// a list or map default.
fn described_properties(value: Option<&serde_json::Value>, what: &str) -> Result<Vec<DeclaredProperty>, String> {
    let mut out: Vec<DeclaredProperty> = Vec::new();
    for entry in value.and_then(serde_json::Value::as_array).map_or(&[][..], Vec::as_slice) {
        let parts = entry.as_array().filter(|p| (2..=3).contains(&p.len()));
        let Some(parts) = parts else {
            return Err(format!("each {what} must be [name, default] or [name, default, type], got {entry}"));
        };
        let key = parts[0].as_str().unwrap_or_default();
        if key.is_empty() || key.len() > 64 || key.contains(':') {
            return Err(format!("{what} {key:?} needs a name of 1 to 64 bytes without ':'"));
        }
        let code = match parts.get(2) {
            None => "number",
            Some(code) => code.as_str().ok_or_else(|| format!("{what} {key:?} has an unknown type {code}"))?,
        };
        let Some((ty, shape)) = PropertyShape::parse(code) else {
            return Err(format!("{what} {key:?} has an unknown type {}", parts.get(2).map_or_else(String::new, ToString::to_string)));
        };
        let Some(default) = PropertyValue::from_described(&parts[1], ty, shape) else {
            return Err(format!("{what} {key:?}: the default {} is not a {}", parts[1], ty.label()));
        };
        if out.iter().any(|p| p.name == key) {
            return Err(format!("{what} {key:?} is declared twice"));
        }
        out.push(DeclaredProperty { name: key.to_owned(), ty, shape, default });
    }
    Ok(out)
}

/// The events a description lists: `[{"name": "Damaged", "params": ["amount:number"]}]`.
fn described_events(value: Option<&serde_json::Value>) -> Result<Vec<DeclaredEvent>, String> {
    let Some(list) = value else { return Ok(Vec::new()) };
    let list = list.as_array().ok_or("events must be a list")?;
    if list.len() > MAX_DECLARED_EVENTS {
        return Err(format!("at most {MAX_DECLARED_EVENTS} events"));
    }
    let mut events: Vec<DeclaredEvent> = Vec::new();
    for entry in list {
        let name = entry.get("name").and_then(serde_json::Value::as_str).unwrap_or_default();
        if name.is_empty() || name.len() > MAX_MESSAGE_NAME || events.iter().any(|e| e.name == name) {
            return Err(format!("event {name:?} needs a name of 1 to {MAX_MESSAGE_NAME} bytes, once"));
        }
        let params: Vec<String> =
            entry.get("params").and_then(serde_json::Value::as_array).into_iter().flatten().filter_map(|p| p.as_str().map(str::to_owned)).collect();
        if params.len() > MAX_EVENT_ARGS
            || params.iter().any(|p| p.split_once(':').is_none_or(|(n, t)| n.is_empty() || bayleaf_graph::VariableType::from_shaped_code(t).is_none()))
        {
            return Err(format!("event {name}: at most {MAX_EVENT_ARGS} values, each name:type"));
        }
        events.push(DeclaredEvent { name: name.to_owned(), params });
    }
    Ok(events)
}

/// A store, shared by every object a module that runs many objects runs.
type Shared = Arc<std::sync::Mutex<Store<HostState>>>;

/// How a script's object gets its calls: a module of its own, or a slot of a module that runs every
/// object of its file in one instance (it exports `bayleaf_update_all`; see `docs/script-abi.md`).
#[derive(Clone)]
enum Calls {
    Own {
        update: Option<TypedFunc<f32, ()>>,
        /// `bayleaf_message(value)`, if the module takes messages.
        on_message: Option<TypedFunc<f32, ()>>,
    },
    Batch {
        /// `bayleaf_update_all(dt)`: every object of the module in one call.
        update_all: TypedFunc<f32, ()>,
        /// `bayleaf_message_slot(slot, value)`, if the module takes messages.
        on_message: Option<TypedFunc<(i32, f32), ()>>,
        /// `bayleaf_detach(slot)`: the object is gone from the module.
        detach: TypedFunc<i32, ()>,
    },
}

/// An init or a start still to run: the module's own, or a slot's of a module that runs many.
#[derive(Clone)]
enum Phase {
    Own(TypedFunc<(), ()>),
    Slot(TypedFunc<i32, ()>),
}

struct Instance {
    object: StableId,
    /// Position in the object's `scripts` list.
    index: usize,
    script_name: String,
    path: PathBuf,
    module_name: String,
    modified: Option<SystemTime>,
    /// The store it runs in: its own, or the one every object of a batched module shares.
    store: Shared,
    /// Its object's slot in the store (0 in a store of its own).
    slot: usize,
    /// The module's instance, to attach more objects of the same file to it.
    module: rt::Instance,
    /// Which module bytes it runs, so objects of the same file share a batched module only while
    /// the file is the same.
    key: u64,
    /// What it declares, kept for objects attached to the same batched module.
    description: Arc<Description>,
    calls: Calls,
    /// Stopped after an error; it no longer runs.
    stopped: bool,
    /// `bayleaf_init`, until it has run. Scripts loaded together all init once all of them are
    /// loaded, then all start, so each one sees the others (their properties, as components).
    pending_init: Option<Phase>,
    /// `bayleaf_start`, until it has run (after every script loaded with it has run its init).
    pending_start: Option<Phase>,
    /// Its object's place in the view of the scene with this structure (`u64::MAX`: not found yet).
    view_index: usize,
    view_structure: u64,
}

impl Instance {
    /// The store it runs in, its object selected for the host functions.
    fn store(&self) -> std::sync::MutexGuard<'_, Store<HostState>> {
        let mut store = self.store.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        store.data_mut().select(self.slot);
        store
    }

    fn batched(&self) -> bool {
        matches!(self.calls, Calls::Batch { .. })
    }

    /// Takes its object out of a batched module (it goes, or loads again): the module drops what
    /// it kept for it, and the slot no longer runs.
    fn release(&self) {
        let Calls::Batch { detach, .. } = &self.calls else { return };
        let mut store = self.store();
        store.data_mut().running = false;
        let _ = store.set_fuel(FUEL_PER_CALL);
        let _ = detach.call(&mut *store, self.slot as i32);
    }
}

/// A copy of a runtime handle: `Copy` in one runtime, `Clone` in the other, so the same code serves both.
fn handle_copy<T: Clone>(value: &T) -> T {
    value.clone()
}

/// A key for module bytes (within one run of the program).
fn bytes_key(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// All script instances of one play session.
pub struct ScriptHost {
    /// What scripts see of the scene that changes only with its structure, kept from the last step.
    layout: Option<Arc<crate::Layout>>,
    engine: Engine,
    linker: Linker<HostState>,
    /// Modules by their bytes, so a script on many objects is compiled once.
    modules: HashMap<Vec<u8>, Module>,
    /// Scripts built into the game, by the module path they stand for (native mode).
    natives: Vec<(PathBuf, crate::NativeScript)>,
    /// How long each script's update took since they were last taken, by script name.
    /// Each script's time, by name, since the profiler last asked; kept only while it records.
    timings: Vec<(String, std::time::Duration)>,
    /// Whether to time scripts (the profiler records).
    timing: bool,
    instances: Vec<Instance>,
    faults: Vec<ScriptFault>,
    logs: Vec<LogLine>,
    requests: Vec<Request>,
    /// Messages sent and not yet delivered.
    messages: Vec<Message>,
    /// Which scripts hear which components' events.
    subscriptions: Vec<Subscription>,
    velocities: Arc<Velocities>,
    grounded: Arc<HashSet<StableId>>,
    interface: Arc<crate::InterfaceValues>,
    input_maps: actions::SharedInput,
    state: SharedState,
    /// A loaded script reads or sets component fields, so each step takes their values along.
    reads_fields: bool,
    /// Script properties set during the step, applied when it ends.
    property_writes: Vec<PropertyWrite>,
    /// The objects as scripts see them, kept from step to step; `view_mark` is where the scene's log
    /// of transform writes stood when it last caught up.
    view: Arc<WorldView>,
    view_mark: u64,
    /// The transforms scripts set during a step, until it ends.
    poses: Poses,
}

impl ScriptHost {
    /// A host whose scripts run on `engine` (see the runtime's `engine`).
    pub(crate) fn from_engine(engine: Engine) -> Self {
        let linker = make_linker(&engine);
        Self {
            engine,
            linker,
            layout: None,
            modules: HashMap::new(),
            natives: Vec::new(),
            timings: Vec::new(),
            timing: false,
            instances: Vec::new(),
            faults: Vec::new(),
            logs: Vec::new(),
            requests: Vec::new(),
            messages: Vec::new(),
            subscriptions: Vec::new(),
            velocities: Arc::new(Velocities::new()),
            grounded: Arc::new(HashSet::new()),
            interface: Arc::new(crate::InterfaceValues::new()),
            input_maps: actions::SharedInput::default(),
            state: SharedState::default(),
            reads_fields: false,
            property_writes: Vec::new(),
            view: Arc::default(),
            view_mark: 0,
            poses: Poses::default(),
        }
    }

    /// Runs the modules at these paths (relative to the game, `scripts/mover.wasm`) as the native
    /// scripts built into the game instead.
    pub(crate) fn set_natives(&mut self, natives: Vec<(PathBuf, crate::NativeScript)>) {
        self.natives = natives;
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
        let modified = bayleaf_core::files::modified(path);
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
        let modified = bayleaf_core::files::modified(path);
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
        let mut store = Store::new(&self.engine, HostState::new(StableId::from_u128(0), [0.0; 9], BTreeMap::new(), 0.0, &Arc::new(Input::default()), limits()));
        store.limiter(|state| &mut state.limits);
        store.set_fuel(FUEL_PER_CALL).map_err(|e| ScriptError::Trapped { module: name.to_owned(), message: trap_message(&e) })?;
        let instance =
            instantiate(&self.linker, &mut store, &module).map_err(|e| ScriptError::Trapped { module: name.to_owned(), message: trap_message(&e) })?;
        describe_instance(&mut store, &instance, name)
    }

    /// Loads a module from memory and runs its `bayleaf_start`; `path` names it in messages and for
    /// hot reload.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn load_bytes(
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
        // A script built into the game runs natively behind the trampoline module.
        let native = crate::native::find(&self.natives, path);
        let bytes = match native {
            Some(_) => crate::native::trampoline().map_err(|e| ScriptError::Invalid(module_name.clone(), e))?,
            None => bytes,
        };
        let module = match self.modules.get(bytes) {
            Some(module) => module.clone(),
            None => {
                let module = Module::new(&self.engine, bytes).map_err(|e| ScriptError::Invalid(module_name.clone(), e.to_string()))?;
                self.modules.insert(bytes.to_vec(), module.clone());
                module
            }
        };
        self.reads_fields |= module.imports().any(|i| matches!(i.name(), "component_get" | "component_set"));
        let key = bytes_key(bytes);
        // Replace an earlier instance in the same place (hot reload).
        for gone in self.instances.iter().filter(|i| i.object == object && i.index == index) {
            gone.release();
        }
        self.instances.retain(|i| !(i.object == object && i.index == index));
        // A module that runs many objects in one instance: every object of the same file shares it.
        let batched = native.is_none() && module.exports().any(|e| e.name() == "bayleaf_update_all");
        let shared = self.instances.iter().find(|i| batched && i.batched() && i.key == key && i.path == path);
        let (store, slot, instance, description) = match shared {
            Some(other) => {
                let store = Arc::clone(&other.store);
                let slot = {
                    let mut locked = store.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    let own =
                        crate::Own { object, index, transform: read_transform(scene, object), position: read_position(scene, object), ..Default::default() };
                    locked.data_mut().add_slot(own)
                };
                (store, slot, handle_copy(&other.module), Arc::clone(&other.description))
            }
            None => {
                let (store, instance, description) = self.new_store(object, index, &module, &module_name, native, scene, time, input)?;
                (Arc::new(std::sync::Mutex::new(store)), 0, instance, Arc::new(description))
            }
        };
        let mut locked = store.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        locked.data_mut().select(slot);
        // Declared defaults, overridden by the values saved in the scene that fit the declared type.
        // A saved value the script does not declare is still there to read, typed by its shape.
        // A list or map (a property, or a visual script's variable) is kept apart: it fills that
        // list or map the first time the script uses it.
        let (single, shaped): (Vec<_>, Vec<_>) = description.properties.iter().cloned().partition(|p| p.shape == PropertyShape::Single);
        let mut properties: Properties = single.into_iter().map(|p| (p.name, (p.ty, p.default))).collect();
        let mut seeds: HashMap<String, DeclaredProperty> = shaped.into_iter().chain(description.kept.iter().cloned()).map(|p| (p.name.clone(), p)).collect();
        for (name, value) in &script.properties {
            if let Some(seed) = seeds.get_mut(name) {
                if seed.accepts(value) {
                    seed.default = value.clone();
                }
                continue;
            }
            match properties.get_mut(name) {
                Some((ty, slot)) if value.fits(*ty) => *slot = value.clone(),
                Some(_) => {}
                None => {
                    properties.insert(name.clone(), (value.shape_type(), value.clone()));
                }
            }
        }
        locked.data_mut().properties = properties;
        locked.data_mut().seeds = seeds;
        let trapped = |module: &str, e: Error| ScriptError::Trapped { module: module.to_owned(), message: trap_message(&e) };
        let (calls, init, start) = if batched {
            let func = |locked: &mut Store<HostState>, name: &str| {
                instance
                    .get_typed_func::<i32, ()>(&mut *locked, name)
                    .map_err(|_| ScriptError::Invalid(module_name.clone(), format!("it runs many objects but has no {name} export")))
            };
            let attach = func(&mut locked, "bayleaf_attach")?;
            let detach = func(&mut locked, "bayleaf_detach")?;
            let update_all = instance
                .get_typed_func::<f32, ()>(&mut *locked, "bayleaf_update_all")
                .map_err(|_| ScriptError::Invalid(module_name.clone(), "its bayleaf_update_all is not (f32)".to_owned()))?;
            let on_message = instance.get_typed_func::<(i32, f32), ()>(&mut *locked, "bayleaf_message_slot").ok();
            let init = instance.get_typed_func::<i32, ()>(&mut *locked, "bayleaf_init_slot").ok().map(Phase::Slot);
            let start = instance.get_typed_func::<i32, ()>(&mut *locked, "bayleaf_start_slot").ok().map(Phase::Slot);
            locked.set_fuel(FUEL_PER_CALL).map_err(|e| trapped(&module_name, e))?;
            attach.call(&mut *locked, slot as i32).map_err(|e| trapped(&module_name, e))?;
            (Calls::Batch { update_all, on_message, detach }, init, start)
        } else {
            let update = instance.get_typed_func::<f32, ()>(&mut *locked, "bayleaf_update").ok();
            let on_message = instance.get_typed_func::<f32, ()>(&mut *locked, "bayleaf_message").ok();
            let init = instance.get_typed_func::<(), ()>(&mut *locked, "bayleaf_init").ok().map(Phase::Own);
            let start = instance.get_typed_func::<(), ()>(&mut *locked, "bayleaf_start").ok().map(Phase::Own);
            (Calls::Own { update, on_message }, init, start)
        };
        drop(locked);
        let script_name = script.display_name();
        self.instances.push(Instance {
            object,
            index,
            script_name,
            path: path.to_path_buf(),
            module_name,
            modified,
            store,
            slot,
            module: instance,
            key,
            description,
            calls,
            stopped: false,
            pending_init: init,
            pending_start: start,
            view_index: 0,
            view_structure: u64::MAX,
        });
        Ok(())
    }

    /// A store of its own for a module's first object: the instance made, its ABI checked, what it
    /// declares.
    #[allow(clippy::too_many_arguments)]
    fn new_store(
        &self,
        object: StableId,
        index: usize,
        module: &Module,
        module_name: &str,
        native: Option<crate::NativeScript>,
        scene: &Scene,
        time: f64,
        input: &Arc<Input>,
    ) -> Result<(Store<HostState>, rt::Instance, Description), ScriptError> {
        let mut state = HostState::new(object, read_transform(scene, object), Properties::new(), time, input, limits());
        state.position = read_position(scene, object);
        state.index = index;
        state.state = Arc::clone(&self.state);
        state.input_maps = Arc::clone(&self.input_maps);
        // The world view comes before each call (init, start, every step), so a script just loaded
        // keeps none: loading stays linear in the scene, not objects times scripts.
        state.world = Arc::default();
        let mut store = Store::new(&self.engine, state);
        store.limiter(|state| &mut state.limits);
        let trapped = |e: Error| ScriptError::Trapped { module: module_name.to_owned(), message: trap_message(&e) };
        store.set_fuel(FUEL_PER_CALL).map_err(trapped)?;
        let instance = instantiate(&self.linker, &mut store, module).map_err(trapped)?;
        store.data_mut().native = native.map(|script| script.create());
        let abi = instance
            .get_typed_func::<(), i32>(&mut store, "bayleaf_abi")
            .map_err(|_| ScriptError::Invalid(module_name.to_owned(), "it has no bayleaf_abi export".to_owned()))?;
        let found = abi.call(&mut store, ()).map_err(trapped)?;
        if found != ABI_VERSION {
            return Err(ScriptError::AbiMismatch { module: module_name.to_owned(), found, expected: ABI_VERSION });
        }
        if instance.get_export(&mut store, "memory").and_then(rt::Extern::into_memory).is_none() {
            return Err(ScriptError::Invalid(module_name.to_owned(), "it exports no memory".to_owned()));
        }
        let description = describe_instance(&mut store, &instance, module_name)?;
        Ok((store, instance, description))
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
        take: impl Fn(&mut Instance) -> Option<Phase>,
    ) -> Vec<(StableId, usize, ScriptError)> {
        let world = self.world_view(scene);
        let mut failed = Vec::new();
        let mut logs = Vec::new();
        for entry in self.instances.iter_mut().filter(|e| !e.stopped) {
            let Some(phase) = take(entry) else { continue };
            let mut store = entry.store();
            {
                let state = store.data_mut();
                state.world = Arc::clone(&world);
                state.transform = read_transform(scene, entry.object);
                state.position = read_position(scene, entry.object);
                state.transform_changed = false;
                state.time = time;
                state.input = Arc::clone(input);
            }
            let result = call(&mut store, &entry.module_name, FUEL_PER_CALL, |store| match &phase {
                Phase::Own(start) => start.call(store, ()),
                Phase::Slot(start) => start.call(store, entry.slot as i32),
            });
            drain_logs(&mut store, &entry.script_name, &mut logs);
            after_call(scene, &mut store, &mut self.requests, &mut self.messages, &mut self.property_writes);
            release_view(store.data_mut());
            if let Err(error) = result {
                store.data_mut().running = false;
                drop(store);
                entry.stopped = true;
                failed.push((entry.object, entry.index, error));
            }
        }
        self.push_logs(logs);
        self.apply_property_writes();
        failed
    }

    /// Runs every running script's update and writes changed transforms back: `bayleaf_update(dt)`
    /// once per object for a module of its own, `bayleaf_update_all(dt)` once for all the objects of
    /// a module that runs many.
    pub fn update(&mut self, scene: &mut Scene, dt: f32, time: f64, input: &Arc<Input>) {
        let mut faults = Vec::new();
        let mut logs = Vec::new();
        // Every script sees the objects as the step began; what scripts set during it goes to the
        // poses, which later scripts of the step read, and reaches the scene once per object.
        let world = self.world_view(scene);
        let Some(layout) = world.layout.clone() else { return };
        self.poses.fit(layout.ids.len());
        let shared = Shares {
            world: &world,
            time,
            input,
            velocities: &self.velocities,
            grounded: &self.grounded,
            interface: &self.interface,
            input_maps: &self.input_maps,
            state: &self.state,
        };
        // The batched modules, each once: the place of each of its objects' instances.
        let mut batches: Vec<(Shared, TypedFunc<f32, ()>, Vec<usize>)> = Vec::new();
        for (place, entry) in self.instances.iter_mut().enumerate().filter(|(_, i)| !i.stopped) {
            // Where its object is in the view, found again only when the scene's structure changed.
            if entry.view_structure != layout.structure {
                let Some(index) = layout.index.get(&entry.object) else {
                    entry.stopped = true;
                    if entry.batched() {
                        entry.store().data_mut().running = false;
                    }
                    continue;
                };
                entry.view_index = *index;
                entry.view_structure = layout.structure;
            }
            let update = match &entry.calls {
                Calls::Own { update: Some(update), .. } => handle_copy(update),
                Calls::Own { update: None, .. } => continue,
                Calls::Batch { update_all, .. } => {
                    match batches.iter_mut().find(|(store, _, _)| Arc::ptr_eq(store, &entry.store)) {
                        Some((_, _, places)) => places.push(place),
                        None => batches.push((Arc::clone(&entry.store), handle_copy(update_all), vec![place])),
                    }
                    continue;
                }
            };
            let mut store = entry.store();
            {
                let state = store.data_mut();
                (state.transform, state.position) = self.poses.get(&world, entry.view_index);
                state.transform_changed = false;
                shared.prepare_shared(state);
            }
            let started = self.timing.then(bayleaf_core::time::Instant::now);
            let result = call(&mut store, &entry.module_name, FUEL_PER_CALL, |store| update.call(store, dt));
            if let Some(started) = started {
                add_timing(&mut self.timings, &entry.script_name, started.elapsed());
            }
            drain_logs(&mut store, &entry.script_name, &mut logs);
            let state = store.data_mut();
            match result {
                Ok(()) => {
                    if std::mem::take(&mut state.transform_changed) {
                        self.poses.put(entry.view_index, state.transform, state.position);
                    }
                    after_shared_posed(state, &layout, &mut self.poses, &mut self.requests, &mut self.messages, &mut self.property_writes);
                    release_view(state);
                }
                Err(error) => {
                    state.writes.clear();
                    release_view(state);
                    drop(store);
                    entry.stopped = true;
                    faults.push(ScriptFault { object: entry.object, script: entry.script_name.clone(), error });
                }
            }
        }
        for (store, update_all, places) in batches {
            let mut store = store.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            shared.prepare_shared(store.data_mut());
            // Each object's transform where it is kept, without selecting it.
            for &place in &places {
                let entry = &self.instances[place];
                let Some(view) = store.data_mut().slot_view(entry.slot) else { continue };
                (*view.transform, *view.position) = self.poses.get(&world, entry.view_index);
                *view.changed = false;
                *view.running = true;
            }
            let Some(first) = places.first().map(|p| &self.instances[*p]) else { continue };
            let (module_name, script_name) = (first.module_name.clone(), first.script_name.clone());
            let started = self.timing.then(bayleaf_core::time::Instant::now);
            let fuel = FUEL_PER_CALL.saturating_mul(places.len() as u64);
            let result = call(&mut store, &module_name, fuel, |store| update_all.call(store, dt));
            if let Some(started) = started {
                add_timing(&mut self.timings, &script_name, started.elapsed());
            }
            // The object whose update failed, if one did: the one its frame marks as being updated,
            // else the one selected when it stopped.
            let failing = if result.is_err() { failing_object(&mut store, &first.module, &self.instances, &places) } else { store.data().object };
            // What the call did for all its objects, then for each: its transform and its logs.
            if result.is_ok() {
                after_shared_posed(store.data_mut(), &layout, &mut self.poses, &mut self.requests, &mut self.messages, &mut self.property_writes);
            } else {
                store.data_mut().writes.clear();
            }
            for &place in &places {
                let entry = &self.instances[place];
                let Some(view) = store.data_mut().slot_view(entry.slot) else { continue };
                if result.is_err() {
                    *view.running = false;
                } else if std::mem::take(view.changed) {
                    self.poses.put(entry.view_index, *view.transform, *view.position);
                }
                if !view.logs.is_empty() {
                    logs.extend(view.logs.drain(..).map(|text| LogLine { object: entry.object, script: entry.script_name.clone(), text }));
                }
            }
            release_view(store.data_mut());
            drop(store);
            if let Err(error) = result {
                // An error stops the module, and with it every object it runs: the error is the one
                // object's whose update it was in.
                for &place in &places {
                    self.instances[place].stopped = true;
                }
                let others = places.len().saturating_sub(1);
                let error = if others == 0 { error } else { with_others(error, others) };
                faults.push(ScriptFault { object: failing, script: script_name, error });
            }
        }
        drop(world);
        self.write_poses(scene);
        self.faults.extend(faults);
        self.push_logs(logs);
        self.apply_property_writes();
        self.deliver(scene, time, input);
        self.apply_property_writes();
    }

    /// The view of the objects for a call: the one kept from the last, with what moved since read
    /// again; a new one when the scene's structure changed, the scene no longer remembers that far
    /// back, or scripts read component fields (which change with any component).
    fn world_view(&mut self, scene: &Scene) -> Arc<WorldView> {
        let values = self.script_values();
        let same = self.view.layout.as_ref().is_some_and(|l| l.structure == scene.structure() && l.ids.len() == scene.object_count());
        match (values, scene.transforms_written_since(self.view_mark)) {
            (None, Some(moved)) if same => {
                if !moved.is_empty() {
                    let view = Arc::make_mut(&mut self.view);
                    if let Some(layout) = view.layout.clone() {
                        for id in moved {
                            if let Some(index) = layout.index.get(id) {
                                view.read(scene, *index);
                            }
                        }
                    }
                }
            }
            (values, _) => self.view = Arc::new(WorldView::of(scene, values, &mut self.layout)),
        }
        self.view_mark = scene.transform_mark();
        Arc::clone(&self.view)
    }

    /// Writes the transforms scripts set this step to the scene, each object once, and keeps them in
    /// the view for the next.
    fn write_poses(&mut self, scene: &mut Scene) {
        if self.poses.order.is_empty() {
            return;
        }
        // Anything else that moved since the view was made comes first, so it is not lost.
        drop(self.world_view(scene));
        let view = Arc::make_mut(&mut self.view);
        let Some(layout) = view.layout.clone() else { return };
        for index in std::mem::take(&mut self.poses.order) {
            let (values, position) = (self.poses.values[index], self.poses.positions[index]);
            self.poses.set[index] = false;
            // The host functions already refused non-finite numbers.
            if scene.set(layout.ids[index], transform_from(&values, position)).is_ok() {
                view.set(index, values, position);
            }
        }
        self.view_mark = scene.transform_mark();
    }

    /// Each script's properties, by object, when scripts read or set component fields.
    fn script_values(&self) -> Option<HashMap<StableId, Vec<ScriptValues>>> {
        if !self.reads_fields {
            return None;
        }
        let mut out: HashMap<StableId, Vec<ScriptValues>> = HashMap::new();
        for entry in self.instances.iter().filter(|i| !i.stopped) {
            let values = ScriptValues { keys: script_keys(&entry.script_name, &entry.path), properties: entry.store().data().properties.clone() };
            out.entry(entry.object).or_default().push(values);
        }
        Some(out)
    }

    /// Sets the script properties scripts asked for: on every script of the object with that name
    /// that declares the property.
    fn apply_property_writes(&mut self) {
        for write in std::mem::take(&mut self.property_writes) {
            for entry in self.instances.iter_mut().filter(|i| i.object == write.object && script_keys(&i.script_name, &i.path).contains(&write.key)) {
                let mut store = entry.store();
                let state = store.data_mut();
                if let Some((ty, slot)) = state.properties.get_mut(&write.name)
                    && let Some(value) = PropertyValue::from_numbers(*ty, write.value)
                {
                    *slot = value;
                    state.properties_changed = true;
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
        let world = self.world_view(scene);
        let shared = Shares {
            world: &world,
            time,
            input,
            velocities: &self.velocities,
            grounded: &self.grounded,
            interface: &self.interface,
            input_maps: &self.input_maps,
            state: &self.state,
        };
        let mut later = Vec::new();
        let mut faults = Vec::new();
        let mut logs = Vec::new();
        take_subscribing(&self.instances, &mut self.subscriptions);
        // A subscriber's copy of an event goes after the event, in this same delivery; copies have no
        // subscribers of their own, so nothing loops.
        let mut queue: std::collections::VecDeque<Message> = messages.into();
        while let Some(message) = queue.pop_front() {
            let reaches = |i: &Instance| !i.stopped && message.to.is_none_or(|to| to == i.object) && message.script.is_none_or(|s| s == i.index);
            if message.from.is_none() {
                for entry in self.instances.iter().filter(|i| reaches(i)) {
                    for sub in self.subscriptions.iter().filter(|s| s.publisher == (entry.object, entry.index) && s.event == message.name) {
                        queue.push_back(Message {
                            to: Some(sub.subscriber.0),
                            name: format!("{}/{}", entry.script_name, message.name),
                            value: message.value,
                            args: Arc::clone(&message.args),
                            script: Some(sub.subscriber.1),
                            from: Some(sub.publisher),
                        });
                    }
                }
            }
            let mut stopped = Vec::new();
            for (place, entry) in self.instances.iter().enumerate().filter(|(_, i)| reaches(i)) {
                let on_message = match &entry.calls {
                    Calls::Own { on_message: Some(f), .. } => Phase2::Own(handle_copy(f)),
                    Calls::Batch { on_message: Some(f), .. } => Phase2::Slot(handle_copy(f)),
                    _ => continue,
                };
                if !scene.contains(entry.object) {
                    continue;
                }
                let mut store = entry.store();
                {
                    let state = store.data_mut();
                    shared.prepare(state, scene, entry.object);
                    state.message = Some(message.name.clone());
                    state.message_args = Arc::clone(&message.args);
                    state.message_from = message.from;
                }
                let result = call(&mut store, &entry.module_name, FUEL_PER_CALL, |store| match &on_message {
                    Phase2::Own(f) => f.call(store, message.value),
                    Phase2::Slot(f) => f.call(store, (entry.slot as i32, message.value)),
                });
                {
                    let state = store.data_mut();
                    state.message = None;
                    state.message_args = Arc::from([]);
                    state.message_from = None;
                }
                take_subscribing_of(&mut store, &mut self.subscriptions);
                drain_logs(&mut store, &entry.script_name, &mut logs);
                release_view(store.data_mut());
                match result {
                    Ok(()) => after_call(scene, &mut store, &mut self.requests, &mut later, &mut self.property_writes),
                    Err(error) => {
                        store.data_mut().running = false;
                        stopped.push(place);
                        faults.push(ScriptFault { object: entry.object, script: entry.script_name.clone(), error });
                    }
                }
            }
            for place in stopped {
                self.instances[place].stopped = true;
            }
        }
        self.messages = later;
        self.faults.extend(faults);
        self.push_logs(logs);
    }

    /// Reloads modules whose file changed since it was loaded. Returns the reloaded module names.
    pub fn reload_changed(&mut self, scene: &mut Scene, time: f64, input: &Arc<Input>) -> Vec<String> {
        // Each module file's time once, however many objects run it.
        let mut times: HashMap<&Path, Option<SystemTime>> = HashMap::new();
        for entry in &self.instances {
            times.entry(entry.path.as_path()).or_insert_with(|| bayleaf_core::files::modified(&entry.path));
        }
        let changed: Vec<(StableId, usize, PathBuf)> = self
            .instances
            .iter()
            .filter(|i| {
                let now = times.get(i.path.as_path()).copied().flatten();
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
                        entry.modified = bayleaf_core::files::modified(&path);
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

    /// Whether to time each script's update (for the profiler, while it records). Off, scripts are
    /// not timed at all.
    pub fn set_timing(&mut self, on: bool) {
        self.timing = on;
    }

    /// How long the scripts of each name took in all since this was last asked (for the profiler),
    /// while timing is on.
    pub fn take_timings(&mut self) -> Vec<(String, std::time::Duration)> {
        std::mem::take(&mut self.timings)
    }

    /// Sends the scripts of `to` a message now (a button of the Inspector, `play.message`).
    pub fn send(&mut self, scene: &mut Scene, to: StableId, name: &str, value: f32, time: f64, input: &Arc<Input>) {
        self.messages.push(Message { to: Some(to), name: name.to_owned(), value, args: Arc::from([]), script: None, from: None });
        self.deliver(scene, time, input);
    }

    /// Sends a message to every script now, as `notify` does for one object.
    pub fn broadcast(&mut self, scene: &mut Scene, name: &str, value: f32, time: f64, input: &Arc<Input>) {
        self.messages.push(Message { to: None, name: name.to_owned(), value, args: Arc::from([]), script: None, from: None });
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

    /// The interface elements as they show, as scripts read them in the next calls.
    pub fn set_interface_values(&mut self, values: crate::InterfaceValues) {
        self.interface = Arc::new(values);
    }

    /// Tells scripts about physics contacts: each `(object, name, other)` becomes a message called
    /// `name` to `object`'s scripts, its value the other object's handle, delivered now.
    pub fn notify(&mut self, scene: &mut Scene, contacts: &[(StableId, &str, StableId)], time: f64, input: &Arc<Input>) {
        if contacts.is_empty() {
            return;
        }
        let world = self.world_view(scene);
        for (object, name, other) in contacts {
            self.messages.push(Message {
                to: Some(*object),
                name: (*name).to_owned(),
                value: world.handle_of(*other) as f32,
                args: Arc::from([]),
                script: None,
                from: None,
            });
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

/// What every call of one step shares: the world view, the time and input, and what physics and
/// the interface said.
struct Shares<'a> {
    world: &'a Arc<WorldView>,
    time: f64,
    input: &'a Arc<Input>,
    velocities: &'a Arc<Velocities>,
    grounded: &'a Arc<HashSet<StableId>>,
    interface: &'a Arc<crate::InterfaceValues>,
    input_maps: &'a actions::SharedInput,
    state: &'a SharedState,
}

impl Shares<'_> {
    /// Readies the selected object's call: its transform as the scene has it, and what calls share.
    fn prepare(&self, state: &mut HostState, scene: &Scene, object: StableId) {
        Self::prepare_own(state, scene, object);
        self.prepare_shared(state);
    }

    /// The selected object's transform as the scene has it.
    fn prepare_own(state: &mut HostState, scene: &Scene, object: StableId) {
        let t = scene.get::<bayleaf_scene::components::Transform>(object).cloned().unwrap_or_default();
        state.transform = crate::abi_transform(&t);
        state.position = t.position;
        state.transform_changed = false;
    }

    /// What every object of a store shares in a call.
    fn prepare_shared(&self, state: &mut HostState) {
        state.time = self.time;
        state.input = Arc::clone(self.input);
        state.world = Arc::clone(self.world);
        state.velocities = Arc::clone(self.velocities);
        state.grounded = Arc::clone(self.grounded);
        state.interface = Arc::clone(self.interface);
        state.input_maps = Arc::clone(self.input_maps);
        state.state = Arc::clone(self.state);
    }
}

/// A message call: a module's own, or a slot's of a module that runs many objects.
enum Phase2 {
    Own(TypedFunc<f32, ()>),
    Slot(TypedFunc<(i32, f32), ()>),
}

/// The object a module that runs many objects was updating when its update stopped: the one its
/// frame marks (`FRAME_CURRENT`), else the one it selected last.
fn failing_object(store: &mut Store<HostState>, module: &rt::Instance, instances: &[Instance], places: &[usize]) -> StableId {
    let selected = store.data().object;
    let Some((at, count)) = store.data_mut().frame_at.take() else { return selected };
    let Some(memory) = module.get_export(&mut *store, "memory").and_then(rt::Extern::into_memory) else { return selected };
    let mut bytes = vec![0u8; count * FRAME_ENTRY];
    if usize::try_from(at).ok().and_then(|at| memory.read(&*store, at, &mut bytes).ok()).is_none() {
        return selected;
    }
    let marked =
        bytes.as_chunks::<FRAME_ENTRY>().0.iter().position(|entry| u32::from_le_bytes([entry[36], entry[37], entry[38], entry[39]]) & FRAME_CURRENT != 0);
    marked.and_then(|slot| places.iter().map(|p| &instances[*p]).find(|i| i.slot == slot)).map_or(selected, |i| i.object)
}

/// Adds a script's time to the profiler's, by its name.
fn add_timing(timings: &mut Vec<(String, std::time::Duration)>, name: &str, took: std::time::Duration) {
    match timings.iter_mut().find(|(n, _)| n == name) {
        Some((_, total)) => *total += took,
        None => timings.push((name.to_owned(), took)),
    }
}

/// An error of a module that runs many objects, saying how many others stopped with it.
fn with_others(error: ScriptError, others: usize) -> ScriptError {
    match error {
        ScriptError::Trapped { module, message } => {
            ScriptError::Trapped { module, message: format!("{message} (the {others} other objects running this script stopped with it)") }
        }
        other => other,
    }
}

/// The selected object's log lines, as lines of `script`.
fn drain_logs(store: &mut Store<HostState>, script: &str, out: &mut Vec<LogLine>) {
    let state = store.data_mut();
    if state.logs.is_empty() {
        return;
    }
    let object = state.object;
    out.extend(state.logs.drain(..).map(|text| LogLine { object, script: script.to_owned(), text }));
}

/// Subscriptions scripts asked for or cancelled since this was last asked; those of objects that
/// are gone go too.
fn take_subscribing(instances: &[Instance], subscriptions: &mut Vec<Subscription>) {
    for entry in instances {
        take_subscribing_of(&mut entry.store(), subscriptions);
    }
    let live: HashSet<(StableId, usize)> = instances.iter().filter(|i| !i.stopped).map(|i| (i.object, i.index)).collect();
    subscriptions.retain(|s| live.contains(&s.subscriber));
}

/// Runs one guest call with a fuel budget and turns a trap into a [`ScriptError`].
fn call(store: &mut Store<HostState>, module_name: &str, fuel: u64, run: impl FnOnce(&mut Store<HostState>) -> Result<(), Error>) -> Result<(), ScriptError> {
    let trapped = |message: String| ScriptError::Trapped { module: module_name.to_owned(), message };
    store.set_fuel(fuel).map_err(|e| trapped(e.to_string()))?;
    store.data_mut().fault = None;
    match run(store) {
        Ok(()) => Ok(()),
        Err(error) => {
            let reason = store.data_mut().fault.take().unwrap_or_else(|| {
                if out_of_fuel(&error) { format!("ran longer than the limit of {FUEL_PER_CALL} instructions in one call") } else { trap_message(&error) }
            });
            Err(ScriptError::Trapped { module: module_name.to_owned(), message: reason })
        }
    }
}

/// Applies the subscriptions the selected object's script asked for or cancelled.
fn take_subscribing_of(store: &mut Store<HostState>, subscriptions: &mut Vec<Subscription>) {
    let state = store.data_mut();
    let subscriber = (state.object, state.index);
    for (add, publisher, event) in state.subscribing.drain(..) {
        let same = |s: &Subscription| s.publisher == publisher && s.event == event && s.subscriber == subscriber;
        if !add {
            subscriptions.retain(|s| !same(s));
        } else if !subscriptions.iter().any(same) && subscriptions.len() < MAX_SUBSCRIPTIONS {
            subscriptions.push(Subscription { publisher, event, subscriber });
        }
    }
}

/// The transforms scripts set during a step, by their object's place in the view: what later
/// scripts of the step read, written to the scene once per object as it ends.
#[derive(Default)]
struct Poses {
    set: Vec<bool>,
    values: Vec<[f32; 9]>,
    positions: Vec<[f64; 3]>,
    /// The places set, in the order first set.
    order: Vec<usize>,
}

impl Poses {
    /// Room for `n` objects; what was set is forgotten when the number changes.
    fn fit(&mut self, n: usize) {
        if self.set.len() != n {
            *self = Self { set: vec![false; n], values: vec![[0.0; 9]; n], positions: vec![[0.0; 3]; n], order: Vec::new() };
        }
    }

    /// The transform of the object at `index`: as set this step, else as the step began.
    fn get(&self, view: &WorldView, index: usize) -> ([f32; 9], [f64; 3]) {
        if self.set.get(index).copied().unwrap_or(false) {
            (self.values[index], self.positions[index])
        } else {
            (view.transforms.get(index).copied().unwrap_or_default(), view.positions.get(index).copied().unwrap_or_default())
        }
    }

    fn put(&mut self, index: usize, values: [f32; 9], position: [f64; 3]) {
        let Some(set) = self.set.get_mut(index) else { return };
        if !*set {
            *set = true;
            self.order.push(index);
        }
        self.values[index] = values;
        self.positions[index] = position;
    }
}

/// A view with nothing in it, which a store holds between calls so the host can change the kept one
/// in place.
static NO_VIEW: std::sync::LazyLock<Arc<WorldView>> = std::sync::LazyLock::new(Arc::default);

fn release_view(state: &mut HostState) {
    state.world = Arc::clone(&NO_VIEW);
}

/// As [`after_shared`], for a step: other objects' transforms go to the poses.
fn after_shared_posed(
    state: &mut HostState,
    layout: &crate::Layout,
    poses: &mut Poses,
    requests: &mut Vec<Request>,
    messages: &mut Vec<Message>,
    properties: &mut Vec<PropertyWrite>,
) {
    for (object, values, position) in state.writes.drain(..) {
        if let Some(index) = layout.index.get(&object) {
            poses.put(*index, values, position);
        }
    }
    requests.append(&mut state.requests);
    messages.append(&mut state.outbox);
    properties.append(&mut state.property_writes);
}

/// Applies what a call changed for the selected object: its own transform, other objects'
/// transforms, and its requests.
fn after_call(scene: &mut Scene, store: &mut Store<HostState>, requests: &mut Vec<Request>, messages: &mut Vec<Message>, properties: &mut Vec<PropertyWrite>) {
    let state = store.data_mut();
    if state.transform_changed && scene.contains(state.object) {
        state.transform_changed = false;
        // `transform_set` already refused non-finite numbers.
        let _ = scene.set(state.object, transform_from(&state.transform, state.position));
    }
    after_shared(scene, store, requests, messages, properties);
}

/// Applies what a call changed apart from its objects' own transforms: other objects' transforms,
/// requests, messages and property writes.
fn after_shared(
    scene: &mut Scene,
    store: &mut Store<HostState>,
    requests: &mut Vec<Request>,
    messages: &mut Vec<Message>,
    properties: &mut Vec<PropertyWrite>,
) {
    let state = store.data_mut();
    for (object, v, position) in state.writes.drain(..) {
        if scene.contains(object) {
            let _ = scene.set(object, transform_from(&v, position));
        }
    }
    requests.append(&mut state.requests);
    messages.append(&mut state.outbox);
    properties.append(&mut state.property_writes);
}

/// Bytes each value takes in `event_send`'s list: a kind, then up to three words.
const EVENT_SLOT: usize = 16;

/// The values of an event as a script lists them for `event_send`: per value a kind (0 number,
/// 1 whole number, 2 true/false, 3 vector, 4 text at a pointer and length, 5 object handle, 0 for
/// none) and its words.
fn read_event_args(caller: &mut Caller<'_, HostState>, ptr: i32, count: i32) -> Result<Vec<EventArg>, String> {
    let count = usize::try_from(count).map_err(|_| "a negative number of values".to_owned())?;
    if count > MAX_EVENT_ARGS {
        return Err(format!("an event brings at most {MAX_EVENT_ARGS} values"));
    }
    let bytes = read_guest(caller, ptr, (count * EVENT_SLOT) as i32, MAX_EVENT_ARGS * EVENT_SLOT)?;
    let mut args = Vec::with_capacity(count);
    for slot in bytes.as_chunks::<EVENT_SLOT>().0 {
        let word = |i: usize| u32::from_le_bytes([slot[i * 4], slot[i * 4 + 1], slot[i * 4 + 2], slot[i * 4 + 3]]);
        let number = |i: usize| f32::from_bits(word(i));
        args.push(match word(0) {
            0 => EventArg::Number(number(1)),
            1 => EventArg::Whole(word(1) as i32),
            2 => EventArg::Flag(word(1) != 0),
            3 => EventArg::Vector([number(1), number(2), number(3)]),
            4 => EventArg::Text(read_text(caller, word(1) as i32, word(2) as i32, MAX_EVENT_TEXT)?),
            5 => EventArg::Object(caller.data().world.get(word(1) as i32).map(|(id, _)| id)),
            6 | 7 => {
                let name = read_text(caller, word(1) as i32, word(2) as i32, MAX_EVENT_TEXT)?;
                copied(caller, &name, word(0) == 6)?
            }
            kind => return Err(format!("{kind} is not a kind of event value")),
        });
        if let Some(EventArg::Number(v) | EventArg::Vector([v, ..])) = args.last()
            && !v.is_finite()
        {
            return Err("event values must be finite numbers".to_owned());
        }
    }
    Ok(args)
}

/// A copy of the list (`list`) or map named `name` this script keeps (as its property or variable
/// starts it, if it has not used it yet), for an event to bring; empty when it keeps none.
fn copied(caller: &Caller<'_, HostState>, name: &str, list: bool) -> Result<EventArg, String> {
    let state = caller.data();
    let kept = match state.collections.get(name) {
        Some(stored) => Some(stored.clone()),
        None => state.seeds.get(name).map(|seed| seeded(caller, seed)),
    };
    match (kept, list) {
        (Some(Stored::List(items)), true) => Ok(EventArg::List(items)),
        (Some(Stored::Map(entries)), false) => Ok(EventArg::Map(entries)),
        (None, true) => Ok(EventArg::List(Vec::new())),
        (None, false) => Ok(EventArg::Map(Default::default())),
        (Some(_), _) => Err(format!("{name} is a {}", if list { "map" } else { "list" })),
    }
}

/// The object a component handle's value names, and which of its scripts: `None` when the object is
/// gone or the component is none.
fn component_at(caller: &Caller<'_, HostState>, handle: i32, script: i32) -> Option<(StableId, usize)> {
    let (object, _) = caller.data().world.get(handle)?;
    let script = usize::try_from(script).ok()?.checked_sub(1)?;
    Some((object, script))
}

/// A script's components: finding one on an object, triggering an event on it, subscribing to its
/// events, and which component an event a subscriber hears happened on.
fn link_components(linker: &mut Linker<HostState>) -> Result<(), LinkError> {
    linker.func_wrap("bayleaf", "script_component", |mut caller: Caller<'_, HostState>, handle: i32, ptr: i32, len: i32| -> Result<i32, Error> {
        let name = read_text(&mut caller, ptr, len, MAX_MESSAGE_NAME).map_err(|m| fault(&mut caller, format!("script_component: {m}")))?;
        Ok(caller.data().world.script_on(handle, &name))
    })?;
    linker.func_wrap("bayleaf", "self_component", |caller: Caller<'_, HostState>| -> i32 { i32::try_from(caller.data().index + 1).unwrap_or(0) })?;
    linker.func_wrap(
        "bayleaf",
        "event_send_to",
        |mut caller: Caller<'_, HostState>, handle: i32, script: i32, ptr: i32, len: i32, args: i32, count: i32| -> Result<i32, Error> {
            let name = read_text(&mut caller, ptr, len, MAX_MESSAGE_NAME).map_err(|m| fault(&mut caller, format!("event_send_to: {m}")))?;
            if name.is_empty() {
                return Err(fault(&mut caller, "event_send_to: give the event's name".to_owned()));
            }
            let values = read_event_args(&mut caller, args, count).map_err(|m| fault(&mut caller, format!("event_send_to: {m}")))?;
            // A component that is none, or on an object that is gone, hears nothing.
            let Some((object, script)) = component_at(&caller, handle, script) else { return Ok(0) };
            let state = caller.data_mut();
            if state.outbox.len() >= MAX_MESSAGES_PER_CALL {
                return Ok(0);
            }
            state.outbox.push(Message { to: Some(object), name, value: 0.0, args: Arc::from(values), script: Some(script), from: None });
            Ok(1)
        },
    )?;
    for (export, add) in [("event_subscribe", true), ("event_unsubscribe", false)] {
        linker.func_wrap("bayleaf", export, move |mut caller: Caller<'_, HostState>, handle: i32, script: i32, ptr: i32, len: i32| -> Result<i32, Error> {
            let name = read_text(&mut caller, ptr, len, MAX_MESSAGE_NAME).map_err(|m| fault(&mut caller, format!("{export}: {m}")))?;
            let Some(publisher) = component_at(&caller, handle, script).filter(|_| !name.is_empty()) else { return Ok(0) };
            let state = caller.data_mut();
            if state.subscribing.len() >= MAX_REQUESTS_PER_CALL {
                return Ok(0);
            }
            state.subscribing.push((add, publisher, name));
            Ok(1)
        })?;
    }
    linker.func_wrap("bayleaf", "event_source", |caller: Caller<'_, HostState>| -> i32 {
        caller.data().message_from.map_or(0, |(object, _)| caller.data().world.handle_of(object))
    })?;
    linker.func_wrap("bayleaf", "event_source_component", |caller: Caller<'_, HostState>| -> i32 {
        caller.data().message_from.map_or(0, |(_, script)| i32::try_from(script + 1).unwrap_or(0))
    })?;
    Ok(())
}

/// Triggering events (`event_send`) and reading the values of the one being delivered.
fn link_events(linker: &mut Linker<HostState>) -> Result<(), LinkError> {
    linker.func_wrap(
        "bayleaf",
        "event_send",
        |mut caller: Caller<'_, HostState>, handle: i32, ptr: i32, len: i32, args: i32, count: i32| -> Result<i32, Error> {
            let name = read_text(&mut caller, ptr, len, MAX_MESSAGE_NAME).map_err(|m| fault(&mut caller, format!("event_send: {m}")))?;
            if name.is_empty() {
                return Err(fault(&mut caller, "event_send: give the event's name".to_owned()));
            }
            let values = read_event_args(&mut caller, args, count).map_err(|m| fault(&mut caller, format!("event_send: {m}")))?;
            let to = if handle == EVERY_OBJECT {
                None
            } else {
                let Some(object) = caller.data().world.get(handle).map(|(id, _)| id) else {
                    return Err(fault(&mut caller, format!("event_send: there is no object with handle {handle}")));
                };
                Some(object)
            };
            let state = caller.data_mut();
            if state.outbox.len() >= MAX_MESSAGES_PER_CALL {
                return Ok(0);
            }
            state.outbox.push(Message { to, name, value: 0.0, args: Arc::from(values), script: None, from: None });
            Ok(1)
        },
    )?;
    link_components(linker)?;
    let arg = |caller: &Caller<'_, HostState>, index: i32| usize::try_from(index).ok().and_then(|i| caller.data().message_args.get(i).cloned());
    linker.func_wrap("bayleaf", "event_number", move |caller: Caller<'_, HostState>, index: i32| -> f32 {
        match arg(&caller, index) {
            Some(EventArg::Number(v)) => v,
            Some(EventArg::Whole(v)) => v as f32,
            _ => 0.0,
        }
    })?;
    linker.func_wrap("bayleaf", "event_whole", move |caller: Caller<'_, HostState>, index: i32| -> i32 {
        match arg(&caller, index) {
            Some(EventArg::Whole(v)) => v,
            Some(EventArg::Number(v)) => v as i32,
            Some(EventArg::Flag(b)) => i32::from(b),
            _ => 0,
        }
    })?;
    linker.func_wrap("bayleaf", "event_flag", move |caller: Caller<'_, HostState>, index: i32| -> i32 {
        i32::from(matches!(arg(&caller, index), Some(EventArg::Flag(true))))
    })?;
    linker.func_wrap("bayleaf", "event_vector", move |caller: Caller<'_, HostState>, index: i32, part: i32| -> f32 {
        match (arg(&caller, index), usize::try_from(part)) {
            (Some(EventArg::Vector(v)), Ok(part)) => v.get(part).copied().unwrap_or(0.0),
            _ => 0.0,
        }
    })?;
    linker.func_wrap("bayleaf", "event_object", move |caller: Caller<'_, HostState>, index: i32| -> i32 {
        match arg(&caller, index) {
            Some(EventArg::Object(Some(id))) => caller.data().world.handle_of(id),
            _ => 0,
        }
    })?;
    // A list or map the event brings becomes one this script keeps under `name`, so list and map
    // nodes work on it.
    linker.func_wrap("bayleaf", "event_keep", move |mut caller: Caller<'_, HostState>, index: i32, map: i32, p: i32, l: i32| -> Result<i32, Error> {
        let name = read_text(&mut caller, p, l, MAX_MESSAGE_NAME).map_err(|m| fault(&mut caller, format!("event_keep: {m}")))?;
        let kept = match arg(&caller, index) {
            Some(EventArg::List(items)) if map == 0 => Stored::List(items),
            Some(EventArg::Map(entries)) if map != 0 => Stored::Map(entries),
            _ if map == 0 => Stored::List(Vec::new()),
            _ => Stored::Map(Default::default()),
        };
        let state = caller.data_mut();
        if !state.collections.contains_key(&name) && state.collections.len() >= MAX_STORED_COLLECTIONS {
            return Err(fault(&mut caller, format!("event_keep: a script keeps at most {MAX_STORED_COLLECTIONS} lists and maps")));
        }
        state.seeds.remove(&name);
        state.collections.insert(name, kept);
        Ok(1)
    })?;
    linker.func_wrap("bayleaf", "event_text", move |mut caller: Caller<'_, HostState>, index: i32, out: i32, cap: i32| -> Result<i32, Error> {
        let Some(EventArg::Text(text)) = arg(&caller, index) else { return Ok(0) };
        let room = usize::try_from(cap).map_err(|_| fault(&mut caller, "event_text: negative length".to_owned()))?;
        if room > 0 {
            write_guest(&mut caller, out, &text.as_bytes()[..text.len().min(room)], "event_text")?;
        }
        Ok(text.len() as i32)
    })?;
    Ok(())
}

/// Reads `len` bytes at `ptr` from the guest's memory.
fn read_guest(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32, limit: usize) -> Result<Vec<u8>, String> {
    let memory = caller.get_export("memory").and_then(rt::Extern::into_memory).ok_or("the script exports no memory")?;
    let (Ok(ptr), Ok(len)) = (usize::try_from(ptr), usize::try_from(len)) else {
        return Err("negative pointer or length".to_owned());
    };
    if len > limit {
        return Err(format!("text of {len} bytes is longer than the limit of {limit}"));
    }
    let mut buffer = vec![0; len];
    memory.read(&*caller, ptr, &mut buffer).map_err(|_| format!("reading {len} bytes at {ptr} is outside the script's memory"))?;
    Ok(buffer)
}

fn read_text(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32, limit: usize) -> Result<String, String> {
    let bytes = read_guest(caller, ptr, len, limit)?;
    String::from_utf8(bytes).map_err(|_| "text is not valid UTF-8".to_owned())
}

/// Records a guest error so the trap message explains it, and returns the trap.
/// Writes `bytes` into the guest's memory at `ptr`.
fn write_guest(caller: &mut Caller<'_, HostState>, ptr: i32, bytes: &[u8], what: &str) -> Result<(), Error> {
    let memory = caller.get_export("memory").and_then(rt::Extern::into_memory).ok_or_else(|| error("no memory"))?;
    let at = usize::try_from(ptr).map_err(|_| fault(caller, format!("{what}: negative pointer")))?;
    memory.write(&mut *caller, at, bytes).map_err(|_| fault(caller, format!("{what}: writing outside the script's memory")))
}

fn fault(caller: &mut Caller<'_, HostState>, message: String) -> Error {
    caller.data_mut().fault = Some(message.clone());
    error(message)
}

/// Records a request to the Particle Emitter of the object with `handle`, applied after the step.
fn particles_request(caller: &mut Caller<'_, HostState>, what: &str, handle: i32, action: crate::ParticleAction) -> Result<i32, Error> {
    let Some(object) = caller.data().world.get(handle).map(|(id, _)| id) else {
        return Err(fault(caller, format!("{what}: there is no object with handle {handle}")));
    };
    let state = caller.data_mut();
    if state.requests.len() >= MAX_REQUESTS_PER_CALL {
        return Ok(0);
    }
    let by = state.object;
    state.requests.push(Request::Particles { by, object, action });
    Ok(1)
}

/// Longest text a script sets on an Interface element, as layouts allow.
const MAX_INTERFACE_TEXT: usize = 4096;

/// Records a change to an element of the Interface of the object with `handle`, applied after the step.
fn interface_request(caller: &mut Caller<'_, HostState>, what: &str, handle: i32, name: (i32, i32), change: crate::InterfaceChange) -> Result<i32, Error> {
    let element = read_text(caller, name.0, name.1, 64).map_err(|m| fault(caller, format!("{what}: {m}")))?;
    let Some(object) = caller.data().world.get(handle).map(|(id, _)| id) else {
        return Err(fault(caller, format!("{what}: there is no object with handle {handle}")));
    };
    let state = caller.data_mut();
    if state.requests.len() >= MAX_REQUESTS_PER_CALL {
        return Ok(0);
    }
    let by = state.object;
    state.requests.push(Request::Interface { by, object, element, change });
    Ok(1)
}

/// Records a physics request for the object with `handle`, applied after the step.
fn physics_request(
    caller: &mut Caller<'_, HostState>,
    what: &str,
    handle: i32,
    v: [f32; 3],
    make: impl FnOnce(StableId, StableId, [f32; 3]) -> Request,
) -> Result<i32, Error> {
    if !v.iter().all(|c| c.is_finite()) {
        return Err(fault(caller, format!("{what}: give finite numbers")));
    }
    let Some(object) = caller.data().world.get(handle).map(|(id, _)| id) else {
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
fn state_scope(caller: &mut Caller<'_, HostState>, what: &str, scope: i32, handle: i32) -> Result<Option<Scope>, Error> {
    match scope {
        0 => Ok(Some(Scope::Game)),
        2 => Ok(Some(Scope::Saved)),
        1 if handle == 0 => Ok(caller.data().world.players().first().copied().map(Scope::Player)),
        // -1 names no object, as a search that found nothing; it is no player.
        1 if handle == -1 => Ok(None),
        1 => match caller.data().world.object(handle) {
            // Only objects with Player Control have a player state.
            Some((id, _)) => Ok(caller.data().world.players().contains(&id).then_some(Scope::Player(id))),
            None => Err(fault(caller, format!("{what}: there is no object with handle {handle}"))),
        },
        _ => Err(fault(caller, format!("{what}: scope is 0 (the game), 1 (a player) or 2 (saved)"))),
    }
}

/// The SDK's way to the host for a native script: each call goes through the trampoline's
/// `call_<name>` export, so the host function runs as for a WebAssembly script.
#[cfg(not(target_arch = "wasm32"))]
struct NativeBridge<'a, 'c> {
    caller: &'a mut Caller<'c, HostState>,
    /// The first call that failed; later calls do nothing.
    failed: Option<Error>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Bridge for NativeBridge<'_, '_> {
    fn call(&mut self, name: &str, args: &[Value]) -> Option<Value> {
        if self.failed.is_some() {
            return None;
        }
        let Some(func) = self.caller.get_export(&format!("call_{name}")).and_then(rt::Extern::into_func) else {
            self.failed = Some(fault(self.caller, format!("{name}: there is no such host function")));
            return None;
        };
        let params: Vec<rt::Val> = args.iter().map(|a| to_val(*a)).collect();
        let mut results = vec![rt::Val::I32(0); func.ty(&*self.caller).results().len()];
        match func.call(&mut *self.caller, &params, &mut results) {
            Ok(()) => Some(results.first().map_or(Value::I32(0), from_val)),
            Err(e) => {
                self.failed = Some(e);
                None
            }
        }
    }

    fn read(&mut self, at: u32, out: &mut [u8]) -> bool {
        let Some(memory) = self.caller.get_export("memory").and_then(rt::Extern::into_memory) else { return false };
        memory.read(&*self.caller, at as usize, out).is_ok()
    }

    fn write(&mut self, at: u32, bytes: &[u8]) -> bool {
        let Some(memory) = self.caller.get_export("memory").and_then(rt::Extern::into_memory) else { return false };
        memory.write(&mut *self.caller, at as usize, bytes).is_ok()
    }
}

/// Runs one of a native script's functions with the bridge set. A failed host call, or a panic,
/// stops the script with its reason.
#[cfg(not(target_arch = "wasm32"))]
fn run_native(caller: &mut Caller<'_, HostState>, run: impl FnOnce(&mut dyn NativeObject)) -> Result<(), Error> {
    let Some(mut object) = caller.data_mut().native.take() else { return Ok(()) };
    let mut bridge = NativeBridge { caller: &mut *caller, failed: None };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        bayleaf_script_sdk::native::with_bridge(&mut bridge, || run(object.as_mut()));
    }));
    let failed = bridge.failed.take();
    caller.data_mut().native = Some(object);
    if let Err(payload) = outcome {
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "no message".to_owned());
        return Err(fault(caller, format!("the script panicked: {message}")));
    }
    failed.map_or(Ok(()), Err)
}

/// In a browser the SDK is a guest's, with no bridge to the host: the web player builds no script
/// in, so no native script gets here, and one that did would stop with this reason.
#[cfg(target_arch = "wasm32")]
fn run_native(caller: &mut Caller<'_, HostState>, _run: impl FnOnce(&mut dyn NativeObject)) -> Result<(), Error> {
    if caller.data().native.is_none() {
        return Ok(());
    }
    Err(fault(caller, "native scripts do not run in a browser".to_owned()))
}

/// The functions the trampoline module calls for a native script.
fn link_native(linker: &mut Linker<HostState>) -> Result<(), LinkError> {
    linker.func_wrap("native", "init", |mut caller: Caller<'_, HostState>| -> Result<(), Error> { run_native(&mut caller, |o| o.init()) })?;
    linker.func_wrap("native", "start", |mut caller: Caller<'_, HostState>| -> Result<(), Error> { run_native(&mut caller, |o| o.start()) })?;
    linker.func_wrap("native", "update", |mut caller: Caller<'_, HostState>, dt: f32| -> Result<(), Error> { run_native(&mut caller, |o| o.update(dt)) })?;
    linker.func_wrap("native", "message", |mut caller: Caller<'_, HostState>, value: f32| -> Result<(), Error> {
        run_native(&mut caller, |o| o.message(value))
    })?;
    // The description is written at the start of the exchange memory: `(0 << 32) | len`.
    linker.func_wrap("native", "describe", |mut caller: Caller<'_, HostState>| -> Result<i64, Error> {
        let text = caller.data().native.as_ref().map(|o| o.describe()).unwrap_or_default();
        write_guest(&mut caller, 0, text.as_bytes(), "describe")?;
        Ok(i64::try_from(text.len()).unwrap_or(0))
    })?;
    Ok(())
}

fn make_linker(engine: &Engine) -> Linker<HostState> {
    let mut linker = Linker::new(engine);
    let result = (|| -> Result<(), LinkError> {
        // A module that runs many objects picks the one the next calls are about (see `bayleaf_update_all`):
        // 0 when it does not run, 1 when it does, 2 when its properties changed since last asked.
        linker.func_wrap("bayleaf", "slot_select", |mut caller: Caller<'_, HostState>, slot: i32| -> i32 {
            let state = caller.data_mut();
            let Ok(slot) = usize::try_from(slot) else { return 0 };
            if !state.select(slot) || !state.running {
                return 0;
            }
            1 + i32::from(std::mem::take(&mut state.properties_changed))
        })?;
        linker.func_wrap("bayleaf", "log", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<(), Error> {
            let text = read_text(&mut caller, ptr, len, MAX_LOG_BYTES).map_err(|m| fault(&mut caller, format!("log: {m}")))?;
            caller.data_mut().logs.push(text);
            Ok(())
        })?;
        linker.func_wrap("bayleaf", "transform_get", |mut caller: Caller<'_, HostState>, out: i32| -> Result<(), Error> {
            let memory = caller.get_export("memory").and_then(rt::Extern::into_memory).ok_or_else(|| error("no memory"))?;
            let bytes: Vec<u8> = caller.data().transform.iter().flat_map(|v| v.to_le_bytes()).collect();
            let ptr = usize::try_from(out).map_err(|_| fault(&mut caller, "transform_get: negative pointer".to_owned()))?;
            memory.write(&mut caller, ptr, &bytes).map_err(|_| error("transform_get: writing outside the script's memory"))?;
            Ok(())
        })?;
        linker.func_wrap("bayleaf", "transform_set", |mut caller: Caller<'_, HostState>, ptr: i32| -> Result<(), Error> {
            let bytes = read_guest(&mut caller, ptr, 36, 36).map_err(|m| fault(&mut caller, format!("transform_set: {m}")))?;
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
        linker.func_wrap("bayleaf", "position_get_of", |mut caller: Caller<'_, HostState>, handle: i32, out: i32| -> Result<(), Error> {
            let Some((object, index)) = caller.data().world.object(handle) else {
                return Err(fault(&mut caller, format!("position_get_of: there is no object with handle {handle}")));
            };
            let (_, position) = caller.data().current(object, index);
            let bytes: Vec<u8> = position.iter().flat_map(|v| v.to_le_bytes()).collect();
            write_guest(&mut caller, out, &bytes, "position_get_of")
        })?;
        linker.func_wrap("bayleaf", "position_set_of", |mut caller: Caller<'_, HostState>, handle: i32, ptr: i32| -> Result<(), Error> {
            let bytes = read_guest(&mut caller, ptr, 24, 24).map_err(|m| fault(&mut caller, format!("position_set_of: {m}")))?;
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
        linker.func_wrap("bayleaf", "property", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<f32, Error> {
            let key = read_text(&mut caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("property: {m}")))?;
            Ok(caller.data().properties.get(&key).and_then(|(_, v)| v.number()).unwrap_or(f32::NAN))
        })?;
        linker.func_wrap("bayleaf", "property_whole", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<i32, Error> {
            let key = read_text(&mut caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("property_whole: {m}")))?;
            Ok(caller.data().properties.get(&key).and_then(|(_, v)| v.number()).map_or(0, |n| n.round() as i32))
        })?;
        linker.func_wrap("bayleaf", "property_vector", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32, out: i32| -> Result<i32, Error> {
            let key = read_text(&mut caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("property_vector: {m}")))?;
            let numbers = caller.data().properties.get(&key).and_then(|(_, v)| v.numbers());
            let bytes: Vec<u8> = numbers.unwrap_or([0.0; 3]).iter().flat_map(|v| v.to_le_bytes()).collect();
            write_guest(&mut caller, out, &bytes, "property_vector")?;
            Ok(i32::from(numbers.is_some()))
        })?;
        linker.func_wrap("bayleaf", "property_text", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32, out: i32, cap: i32| -> Result<i32, Error> {
            let key = read_text(&mut caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("property_text: {m}")))?;
            let text = caller.data().properties.get(&key).and_then(|(_, v)| v.text().map(str::to_owned)).unwrap_or_default();
            let cap = usize::try_from(cap).map_err(|_| fault(&mut caller, "property_text: the room must not be negative".to_owned()))?;
            let bytes = text.as_bytes();
            write_guest(&mut caller, out, &bytes[..bytes.len().min(cap)], "property_text")?;
            Ok(i32::try_from(bytes.len()).unwrap_or(i32::MAX))
        })?;
        linker.func_wrap("bayleaf", "property_set", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32, x: f32, y: f32, z: f32| -> Result<i32, Error> {
            let key = read_text(&mut caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("property_set: {m}")))?;
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
        })?;
        linker.func_wrap(
            "bayleaf",
            "property_set_text",
            |mut caller: Caller<'_, HostState>, ptr: i32, len: i32, t_ptr: i32, t_len: i32| -> Result<i32, Error> {
                let key = read_text(&mut caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("property_set_text: {m}")))?;
                let text = read_text(&mut caller, t_ptr, t_len, bayleaf_scene::components::MAX_PROPERTY_TEXT)
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
        linker.func_wrap("bayleaf", "property_object", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<i32, Error> {
            let key = read_text(&mut caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("property_object: {m}")))?;
            let state = caller.data();
            let name = state.properties.get(&key).filter(|(ty, _)| *ty == PropertyType::Object).and_then(|(_, v)| v.text()).unwrap_or_default();
            if name.is_empty() {
                return Ok(0);
            }
            Ok(state.world.find(name))
        })?;
        linker.func_wrap("bayleaf", "time", |caller: Caller<'_, HostState>| -> f64 { caller.data().time })?;
        linker.func_wrap("bayleaf", "key_down", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<i32, Error> {
            let name = read_text(&mut caller, ptr, len, 64).map_err(|m| fault(&mut caller, format!("key_down: {m}")))?;
            Ok(i32::from(caller.data().input.keys.contains(&name)))
        })?;
        linker.func_wrap("bayleaf", "pointer", |mut caller: Caller<'_, HostState>, out: i32| -> Result<i32, Error> {
            let (xy, inside) = match caller.data().input.pointer {
                Some(p) => (p, 1),
                None => ([0.0; 2], 0),
            };
            let bytes: Vec<u8> = xy.iter().flat_map(|v| v.to_le_bytes()).collect();
            write_guest(&mut caller, out, &bytes, "pointer")?;
            Ok(inside)
        })?;
        linker.func_wrap("bayleaf", "pointer_over_interface", |caller: Caller<'_, HostState>| -> i32 { i32::from(caller.data().input.over_interface) })?;
        // Power for visual scripts, which WebAssembly has no instruction for.
        linker.func_wrap("bayleaf", "math_pow", |_: Caller<'_, HostState>, base: f32, exponent: f32| -> f32 { base.powf(exponent) })?;
        linker.func_wrap("bayleaf", "mouse_down", |caller: Caller<'_, HostState>, button: i32| -> i32 {
            match u32::try_from(button) {
                Ok(b) if b < 3 => i32::from((caller.data().input.buttons >> b) & 1),
                _ => 0,
            }
        })?;
        linker.func_wrap("bayleaf", "apply_impulse", |mut caller: Caller<'_, HostState>, handle: i32, x: f32, y: f32, z: f32| -> Result<i32, Error> {
            physics_request(&mut caller, "apply_impulse", handle, [x, y, z], |by, object, impulse| Request::Push { by, object, impulse })
        })?;
        linker.func_wrap("bayleaf", "set_velocity", |mut caller: Caller<'_, HostState>, handle: i32, x: f32, y: f32, z: f32| -> Result<i32, Error> {
            physics_request(&mut caller, "set_velocity", handle, [x, y, z], |by, object, velocity| Request::SetVelocity { by, object, velocity })
        })?;
        linker.func_wrap(
            "bayleaf",
            "state_get",
            |mut caller: Caller<'_, HostState>, scope: i32, handle: i32, ptr: i32, len: i32, out: i32| -> Result<i32, Error> {
                let name = read_text(&mut caller, ptr, len, 64).map_err(|m| fault(&mut caller, format!("state_get: {m}")))?;
                let value = state_scope(&mut caller, "state_get", scope, handle)?.and_then(|scope| lock(&caller.data().state).get(scope, &name));
                write_guest(&mut caller, out, &value.unwrap_or(0.0).to_le_bytes(), "state_get")?;
                Ok(i32::from(value.is_some()))
            },
        )?;
        linker.func_wrap(
            "bayleaf",
            "state_set",
            |mut caller: Caller<'_, HostState>, scope: i32, handle: i32, ptr: i32, len: i32, value: f64| -> Result<i32, Error> {
                let name = read_text(&mut caller, ptr, len, 64).map_err(|m| fault(&mut caller, format!("state_set: {m}")))?;
                let scope = state_scope(&mut caller, "state_set", scope, handle)?;
                let stored = scope.is_some_and(|scope| lock(&caller.data().state).set(scope, &name, value));
                Ok(i32::from(stored))
            },
        )?;
        linker.func_wrap("bayleaf", "game_end", |caller: Caller<'_, HostState>, won: i32| -> i32 { i32::from(lock(&caller.data().state).end(won != 0)) })?;
        linker.func_wrap("bayleaf", "action", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<f32, Error> {
            // `map/action`: two names of up to 64 bytes and the slash.
            let name = read_text(&mut caller, ptr, len, 129).map_err(|m| fault(&mut caller, format!("action: {m}")))?;
            let state = caller.data();
            Ok(actions::lock(&state.input_maps).of(state.object).map_or(0.0, |maps| maps.value(&name, &state.input)))
        })?;
        linker.func_wrap("bayleaf", "input_map_set", |mut caller: Caller<'_, HostState>, handle: i32, ptr: i32, len: i32| -> Result<i32, Error> {
            let name = read_text(&mut caller, ptr, len, 64).map_err(|m| fault(&mut caller, format!("input_map_set: {m}")))?;
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
        linker.func_wrap("bayleaf", "character_walk", |mut caller: Caller<'_, HostState>, handle: i32, x: f32, y: f32, z: f32| -> Result<i32, Error> {
            physics_request(&mut caller, "character_walk", handle, [x, y, z], |by, object, velocity| Request::Walk { by, object, velocity })
        })?;
        linker.func_wrap("bayleaf", "character_jump", |mut caller: Caller<'_, HostState>, handle: i32, speed: f32| -> Result<i32, Error> {
            physics_request(&mut caller, "character_jump", handle, [speed, 0.0, 0.0], |by, object, v| Request::Jump { by, object, speed: v[0] })
        })?;
        linker.func_wrap("bayleaf", "character_grounded", |mut caller: Caller<'_, HostState>, handle: i32| -> Result<i32, Error> {
            let Some((object, _)) = caller.data().world.object(handle) else {
                return Err(fault(&mut caller, format!("character_grounded: there is no object with handle {handle}")));
            };
            Ok(i32::from(caller.data().grounded.contains(&object)))
        })?;
        linker.func_wrap("bayleaf", "velocity_of", |mut caller: Caller<'_, HostState>, handle: i32, out: i32| -> Result<i32, Error> {
            let Some(object) = caller.data().world.get(handle).map(|(id, _)| id) else {
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
            |mut caller: Caller<'_, HostState>, ox: f32, oy: f32, oz: f32, dx: f32, dy: f32, dz: f32, max: f32, out: i32| -> Result<i32, Error> {
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
        linker.func_wrap("bayleaf", "find", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<i32, Error> {
            let name = read_text(&mut caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("find: {m}")))?;
            Ok(caller.data().world.find(&name))
        })?;
        linker.func_wrap("bayleaf", "find_with", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32, index: i32| -> Result<i32, Error> {
            let kind = read_text(&mut caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("find_with: {m}")))?;
            let Ok(index) = usize::try_from(index) else { return Ok(0) };
            Ok(caller.data().world.with_kind(&kind).nth(index).unwrap_or(0))
        })?;
        linker.func_wrap(
            "bayleaf",
            "component_get",
            |mut caller: Caller<'_, HostState>, handle: i32, c_ptr: i32, c_len: i32, f_ptr: i32, f_len: i32, out: i32| -> Result<i32, Error> {
                let component = read_text(&mut caller, c_ptr, c_len, 256).map_err(|m| fault(&mut caller, format!("component_get: {m}")))?;
                let field = read_text(&mut caller, f_ptr, f_len, 256).map_err(|m| fault(&mut caller, format!("component_get: {m}")))?;
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
            |mut caller: Caller<'_, HostState>, handle: i32, c_ptr: i32, c_len: i32, f_ptr: i32, f_len: i32, ptr: i32| -> Result<i32, Error> {
                let component = read_text(&mut caller, c_ptr, c_len, 256).map_err(|m| fault(&mut caller, format!("component_set: {m}")))?;
                let field = read_text(&mut caller, f_ptr, f_len, 256).map_err(|m| fault(&mut caller, format!("component_set: {m}")))?;
                let bytes = read_guest(&mut caller, ptr, 12, 12).map_err(|m| fault(&mut caller, format!("component_set: {m}")))?;
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
        linker.func_wrap("bayleaf", "object_ref", |mut caller: Caller<'_, HostState>, handle: i32| -> Result<i32, Error> {
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
        linker.func_wrap("bayleaf", "name_of", |mut caller: Caller<'_, HostState>, handle: i32, out: i32, cap: i32| -> Result<i32, Error> {
            let name = caller.data().world.get(handle).map(|(_, name)| name.clone()).unwrap_or_default();
            let cap = usize::try_from(cap).map_err(|_| fault(&mut caller, "name_of: the room must not be negative".to_owned()))?;
            let bytes = name.as_bytes();
            write_guest(&mut caller, out, &bytes[..bytes.len().min(cap)], "name_of")?;
            Ok(i32::try_from(bytes.len()).unwrap_or(i32::MAX))
        })?;
        linker.func_wrap("bayleaf", "count_with", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<i32, Error> {
            let kind = read_text(&mut caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("count_with: {m}")))?;
            Ok(i32::try_from(caller.data().world.with_kind(&kind).count()).unwrap_or(i32::MAX))
        })?;
        linker.func_wrap("bayleaf", "transform_get_of", |mut caller: Caller<'_, HostState>, handle: i32, out: i32| -> Result<(), Error> {
            let Some((object, index)) = caller.data().world.object(handle) else {
                return Err(fault(&mut caller, format!("transform_get_of: there is no object with handle {handle}")));
            };
            let (value, _) = caller.data().current(object, index);
            let bytes: Vec<u8> = value.iter().flat_map(|v| v.to_le_bytes()).collect();
            write_guest(&mut caller, out, &bytes, "transform_get_of")
        })?;
        linker.func_wrap("bayleaf", "transform_set_of", |mut caller: Caller<'_, HostState>, handle: i32, ptr: i32| -> Result<(), Error> {
            let bytes = read_guest(&mut caller, ptr, 36, 36).map_err(|m| fault(&mut caller, format!("transform_set_of: {m}")))?;
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
        linker.func_wrap("bayleaf", "spawn", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32, x: f32, y: f32, z: f32| -> Result<i32, Error> {
            let prefab = read_text(&mut caller, ptr, len, 512).map_err(|m| fault(&mut caller, format!("spawn: {m}")))?;
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
        for (name, action) in [("particles_play", crate::ParticleAction::Play), ("particles_stop", crate::ParticleAction::Stop)] {
            linker.func_wrap("bayleaf", name, move |mut caller: Caller<'_, HostState>, handle: i32| -> Result<i32, Error> {
                particles_request(&mut caller, name, handle, action)
            })?;
        }
        linker.func_wrap("bayleaf", "particles_burst", |mut caller: Caller<'_, HostState>, handle: i32, count: i32| -> Result<i32, Error> {
            let count = u32::try_from(count).map_err(|_| fault(&mut caller, "particles_burst: the count must not be negative".to_owned()))?;
            particles_request(&mut caller, "particles_burst", handle, crate::ParticleAction::Burst(count))
        })?;
        linker.func_wrap(
            "bayleaf",
            "ui_set_text",
            |mut caller: Caller<'_, HostState>, handle: i32, name_ptr: i32, name_len: i32, ptr: i32, len: i32| -> Result<i32, Error> {
                let text = read_text(&mut caller, ptr, len, MAX_INTERFACE_TEXT).map_err(|m| fault(&mut caller, format!("ui_set_text: {m}")))?;
                interface_request(&mut caller, "ui_set_text", handle, (name_ptr, name_len), crate::InterfaceChange::Text(text))
            },
        )?;
        linker.func_wrap(
            "bayleaf",
            "ui_set_value",
            |mut caller: Caller<'_, HostState>, handle: i32, name_ptr: i32, name_len: i32, value: f64| -> Result<i32, Error> {
                if !value.is_finite() {
                    return Err(fault(&mut caller, "ui_set_value: the value must be a finite number".to_owned()));
                }
                interface_request(&mut caller, "ui_set_value", handle, (name_ptr, name_len), crate::InterfaceChange::Value(value))
            },
        )?;
        linker.func_wrap("bayleaf", "ui_value", |mut caller: Caller<'_, HostState>, handle: i32, name_ptr: i32, name_len: i32| -> Result<f64, Error> {
            let name = read_text(&mut caller, name_ptr, name_len, 64).map_err(|m| fault(&mut caller, format!("ui_value: {m}")))?;
            let Some(object) = caller.data().world.get(handle).map(|(id, _)| id) else {
                return Err(fault(&mut caller, format!("ui_value: there is no object with handle {handle}")));
            };
            Ok(caller.data().interface.get(&(object, name)).map_or(f64::NAN, |(value, _)| *value))
        })?;
        linker.func_wrap(
            "bayleaf",
            "ui_text",
            |mut caller: Caller<'_, HostState>, handle: i32, name_ptr: i32, name_len: i32, out: i32, cap: i32| -> Result<i32, Error> {
                let name = read_text(&mut caller, name_ptr, name_len, 64).map_err(|m| fault(&mut caller, format!("ui_text: {m}")))?;
                let Some(object) = caller.data().world.get(handle).map(|(id, _)| id) else {
                    return Err(fault(&mut caller, format!("ui_text: there is no object with handle {handle}")));
                };
                let Some(text) = caller.data().interface.get(&(object, name)).map(|(_, text)| text.clone()) else { return Ok(-1) };
                let cap = usize::try_from(cap).map_err(|_| fault(&mut caller, "ui_text: the room must not be negative".to_owned()))?;
                let bytes = text.as_bytes();
                write_guest(&mut caller, out, &bytes[..bytes.len().min(cap)], "ui_text")?;
                Ok(i32::try_from(bytes.len()).unwrap_or(i32::MAX))
            },
        )?;
        linker.func_wrap(
            "bayleaf",
            "ui_add",
            |mut caller: Caller<'_, HostState>,
             handle: i32,
             parent_ptr: i32,
             parent_len: i32,
             what_ptr: i32,
             what_len: i32,
             name_ptr: i32,
             name_len: i32|
             -> Result<i32, Error> {
                let parent = read_text(&mut caller, parent_ptr, parent_len, 64).map_err(|m| fault(&mut caller, format!("ui_add: {m}")))?;
                let what = read_text(&mut caller, what_ptr, what_len, 512).map_err(|m| fault(&mut caller, format!("ui_add: {m}")))?;
                interface_request(&mut caller, "ui_add", handle, (name_ptr, name_len), crate::InterfaceChange::Add { parent, what })
            },
        )?;
        linker.func_wrap("bayleaf", "ui_remove", |mut caller: Caller<'_, HostState>, handle: i32, name_ptr: i32, name_len: i32| -> Result<i32, Error> {
            interface_request(&mut caller, "ui_remove", handle, (name_ptr, name_len), crate::InterfaceChange::Remove)
        })?;
        linker.func_wrap(
            "bayleaf",
            "ui_show",
            |mut caller: Caller<'_, HostState>, handle: i32, name_ptr: i32, name_len: i32, visible: i32| -> Result<i32, Error> {
                interface_request(&mut caller, "ui_show", handle, (name_ptr, name_len), crate::InterfaceChange::Visible(visible != 0))
            },
        )?;
        for (name, stop) in [("sound_play", false), ("sound_stop", true)] {
            linker.func_wrap("bayleaf", name, move |mut caller: Caller<'_, HostState>, handle: i32| -> Result<i32, Error> {
                let Some(object) = caller.data().world.get(handle).map(|(id, _)| id) else {
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
            |mut caller: Caller<'_, HostState>, ptr: i32, len: i32, x: f32, y: f32, z: f32, volume: f32| -> Result<i32, Error> {
                let path = read_text(&mut caller, ptr, len, 512).map_err(|m| fault(&mut caller, format!("sound_play_at: {m}")))?;
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
        linker.func_wrap("bayleaf", "scene_load", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32, additive: i32| -> Result<i32, Error> {
            let path = read_text(&mut caller, ptr, len, 512).map_err(|m| fault(&mut caller, format!("scene_load: {m}")))?;
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
        linker.func_wrap("bayleaf", "scene_unload", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<i32, Error> {
            let path = read_text(&mut caller, ptr, len, 512).map_err(|m| fault(&mut caller, format!("scene_unload: {m}")))?;
            let state = caller.data_mut();
            if state.requests.len() >= MAX_REQUESTS_PER_CALL {
                return Ok(0);
            }
            let by = state.object;
            state.requests.push(Request::UnloadScene { by, path });
            Ok(1)
        })?;
        linker.func_wrap("bayleaf", "destroy", |mut caller: Caller<'_, HostState>, handle: i32| -> Result<i32, Error> {
            let Some(object) = caller.data().world.get(handle).map(|(id, _)| id) else {
                return Err(fault(&mut caller, format!("destroy: there is no object with handle {handle}")));
            };
            let state = caller.data_mut();
            if state.requests.len() >= MAX_REQUESTS_PER_CALL {
                return Ok(0);
            }
            state.requests.push(Request::Destroy(object));
            Ok(1)
        })?;
        linker.func_wrap("bayleaf", "send", |mut caller: Caller<'_, HostState>, handle: i32, ptr: i32, len: i32, value: f32| -> Result<i32, Error> {
            let name = read_text(&mut caller, ptr, len, MAX_MESSAGE_NAME).map_err(|m| fault(&mut caller, format!("send: {m}")))?;
            if name.is_empty() || !value.is_finite() {
                return Err(fault(&mut caller, "send: give a message name and a finite number".to_owned()));
            }
            let to = if handle == EVERY_OBJECT {
                None
            } else {
                let Some(object) = caller.data().world.get(handle).map(|(id, _)| id) else {
                    return Err(fault(&mut caller, format!("send: there is no object with handle {handle}")));
                };
                Some(object)
            };
            let state = caller.data_mut();
            if state.outbox.len() >= MAX_MESSAGES_PER_CALL {
                return Ok(0);
            }
            state.outbox.push(Message { to, name, value, args: Arc::from([]), script: None, from: None });
            Ok(1)
        })?;
        linker.func_wrap("bayleaf", "message_is", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<i32, Error> {
            let name = read_guest(&mut caller, ptr, len, 4096).map_err(|m| fault(&mut caller, format!("message_is: {m}")))?;
            Ok(i32::from(caller.data().message.as_ref().is_some_and(|m| m.as_bytes() == name)))
        })?;
        linker.func_wrap("bayleaf", "message_name", |mut caller: Caller<'_, HostState>, out: i32, cap: i32| -> Result<i32, Error> {
            let Some(name) = caller.data().message.clone() else { return Ok(0) };
            let room = usize::try_from(cap).map_err(|_| fault(&mut caller, "message_name: negative length".to_owned()))?;
            let bytes = &name.as_bytes()[..name.len().min(room)];
            write_guest(&mut caller, out, bytes, "message_name")?;
            Ok(name.len() as i32)
        })?;
        link_events(&mut linker)?;
        link_collections(&mut linker)?;
        link_frames(&mut linker)?;
        link_native(&mut linker)?;
        Ok(())
    })();
    // The names are fixed and unique, so defining them cannot fail.
    debug_assert!(result.is_ok());
    linker
}

/// The words of one value as `event_send` lists them, written at `out` in the script's memory; a
/// text gives its length (its bytes come from `list_text` and `map_text`), an object its handle.
fn write_value(caller: &mut Caller<'_, HostState>, out: i32, value: &EventArg, what: &str) -> Result<(), Error> {
    let words: [u32; 4] = match value {
        EventArg::Number(v) => [0, v.to_bits(), 0, 0],
        EventArg::Whole(v) => [1, *v as u32, 0, 0],
        EventArg::Flag(v) => [2, u32::from(*v), 0, 0],
        EventArg::Vector([x, y, z]) => [3, x.to_bits(), y.to_bits(), z.to_bits()],
        EventArg::Text(text) => [4, text.len() as u32, 0, 0],
        EventArg::Object(id) => [5, id.map_or(0, |id| caller.data().world.handle_of(id)) as u32, 0, 0],
        EventArg::List(_) | EventArg::Map(_) => [0; 4],
    };
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    write_guest(caller, out, &bytes, what)
}

/// The one value at `ptr`, as `event_send` lists values.
fn read_value(caller: &mut Caller<'_, HostState>, ptr: i32, what: &str) -> Result<EventArg, Error> {
    let mut values = read_event_args(caller, ptr, 1).map_err(|m| fault(caller, format!("{what}: {m}")))?;
    if matches!(values.first(), Some(EventArg::List(_) | EventArg::Map(_))) {
        return Err(fault(caller, format!("{what}: a list or map holds no lists or maps")));
    }
    values.pop().ok_or_else(|| fault(caller, format!("{what}: no value")))
}

/// Writes a text's bytes, up to `cap`, at `out`; its whole length, or 0 when it is not a text.
fn write_text(caller: &mut Caller<'_, HostState>, value: Option<EventArg>, out: i32, cap: i32, what: &str) -> Result<i32, Error> {
    let Some(EventArg::Text(text)) = value else { return Ok(0) };
    let room = usize::try_from(cap).map_err(|_| fault(caller, format!("{what}: negative length")))?;
    if room > 0 {
        write_guest(caller, out, &text.as_bytes()[..text.len().min(room)], what)?;
    }
    Ok(text.len() as i32)
}

/// Runs `f` on the list (`list`) or map named at `ptr`, made empty the first time. A name that does
/// not read, a script keeping too many, or a list named as a map (or the other way) stops the script.
fn with_stored<R>(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32, list: bool, what: &str, f: impl FnOnce(&mut Stored) -> R) -> Result<R, Error> {
    let name = read_text(caller, ptr, len, MAX_MESSAGE_NAME).map_err(|m| fault(caller, format!("{what}: {m}")))?;
    let known = caller.data().collections.contains_key(&name);
    if !known && caller.data().collections.len() >= MAX_STORED_COLLECTIONS {
        return Err(fault(caller, format!("{what}: a script keeps at most {MAX_STORED_COLLECTIONS} lists and maps")));
    }
    let other = caller.data().collections.get(&name).is_some_and(|s| matches!(s, Stored::List(_)) != list);
    if other {
        return Err(fault(caller, format!("{what}: {name} is a {}", if list { "map" } else { "list" })));
    }
    if !known && let Some(seed) = caller.data_mut().seeds.remove(&name) {
        let seeded = seeded(caller, &seed);
        if matches!(seeded, Stored::List(_)) != list {
            return Err(fault(caller, format!("{what}: {name} is a {}", if list { "map" } else { "list" })));
        }
        caller.data_mut().collections.insert(name.clone(), seeded);
    }
    let stored = caller.data_mut().collections.entry(name).or_insert_with(|| if list { Stored::List(Vec::new()) } else { Stored::Map(Default::default()) });
    Ok(f(stored))
}

/// A list or map as a property or variable starts it, its values as the script reads them: an
/// object by its name, found in the scene as the step began (none when there is no such object).
fn seeded(caller: &Caller<'_, HostState>, seed: &DeclaredProperty) -> Stored {
    let world = &caller.data().world;
    let arg = |value: &PropertyValue| -> EventArg {
        match (seed.ty, value) {
            (PropertyType::WholeNumber, PropertyValue::Number(n)) => EventArg::Whole(*n as i32),
            (PropertyType::Object, PropertyValue::Text(name)) => EventArg::Object(world.get(world.find(name)).map(|(id, _)| id)),
            (_, PropertyValue::Number(n)) => EventArg::Number(*n as f32),
            (_, PropertyValue::Flag(b)) => EventArg::Flag(*b),
            (_, PropertyValue::Triple(v)) => EventArg::Vector(*v),
            (_, PropertyValue::Text(t)) => EventArg::Text(t.clone()),
            (_, PropertyValue::List { .. } | PropertyValue::Map { .. }) => EventArg::Number(0.0),
        }
    };
    match (&seed.shape, &seed.default) {
        (PropertyShape::Map(key), PropertyValue::Map { map }) => {
            Stored::Map(map.iter().filter_map(|(k, v)| Some((seeded_key(world, *key, k)?, arg(v)))).collect())
        }
        (PropertyShape::Map(_), _) => Stored::Map(Default::default()),
        (_, PropertyValue::List { list }) => Stored::List(list.iter().map(arg).collect()),
        _ => Stored::List(Vec::new()),
    }
}

/// A map key of type `ty` as a property or variable starts it (always text: `3`, `true`, `1,2,3`, an
/// object's name); `None` when it is no such key, or names no object in the scene.
fn seeded_key(world: &WorldView, ty: PropertyType, key: &str) -> Option<MapKey> {
    let number = |t: &str| t.trim().parse::<f32>().ok().filter(|v| v.is_finite());
    Some(match ty {
        PropertyType::Text => MapKey::Text(key.to_owned()),
        PropertyType::Object => MapKey::Object(world.get(world.find(key)).map(|(id, _)| id)?),
        PropertyType::Number => MapKey::Number(Ordered::new(number(key)?)),
        PropertyType::WholeNumber => MapKey::Whole(key.trim().parse().ok()?),
        PropertyType::TrueFalse => MapKey::Flag(key.parse().ok()?),
        PropertyType::Vector | PropertyType::Color => {
            let parts: Vec<f32> = key.split(',').map(number).collect::<Option<_>>()?;
            let [x, y, z] = parts.as_slice() else { return None };
            MapKey::Vector([*x, *y, *z].map(Ordered::new))
        }
    })
}

/// Bytes of one object's entry in a frame (`frame_get`, `frame_set`): its transform (nine `f32`),
/// a word of flags, and its position in double precision.
const FRAME_ENTRY: usize = 64;
/// Frame flags: the object runs; its properties changed since last asked; (from the script) its
/// transform changed.
const FRAME_RUNNING: u32 = 1;
const FRAME_PROPERTIES: u32 = 2;
const FRAME_CHANGED: u32 = 4;
/// (From the script) the object being updated, so an error is reported on it.
const FRAME_CURRENT: u32 = 8;

/// The frame functions: a module that runs many objects reads every object's transform in one
/// call before its update and writes the changed ones back in one call after it, instead of a call
/// per object for each.
fn link_frames(linker: &mut Linker<HostState>) -> Result<(), LinkError> {
    linker.func_wrap("bayleaf", "frame_get", |mut caller: Caller<'_, HostState>, out: i32, cap: i32| -> Result<i32, Error> {
        let state = caller.data_mut();
        let count = state.parked.len();
        let room = usize::try_from(cap).unwrap_or(0);
        if room < count {
            return Ok(count as i32);
        }
        let mut bytes = vec![0u8; count * FRAME_ENTRY];
        for (slot, entry) in bytes.as_chunks_mut::<FRAME_ENTRY>().0.iter_mut().enumerate() {
            let Some(view) = state.slot_view(slot) else { continue };
            for (i, v) in view.transform.iter().enumerate() {
                entry[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
            let mut flags = if *view.running { FRAME_RUNNING } else { 0 };
            if std::mem::take(view.properties_changed) {
                flags |= FRAME_PROPERTIES;
            }
            entry[36..40].copy_from_slice(&flags.to_le_bytes());
            for (i, v) in view.position.iter().enumerate() {
                entry[40 + i * 8..48 + i * 8].copy_from_slice(&v.to_le_bytes());
            }
        }
        write_guest(&mut caller, out, &bytes, "frame_get")?;
        caller.data_mut().frame_at = Some((out, count));
        Ok(count as i32)
    })?;
    linker.func_wrap("bayleaf", "frame_set", |mut caller: Caller<'_, HostState>, ptr: i32, count: i32| -> Result<(), Error> {
        let slots = caller.data().parked.len();
        let count = usize::try_from(count).map_err(|_| fault(&mut caller, "frame_set: negative count".to_owned()))?;
        if count > slots {
            return Err(fault(&mut caller, format!("frame_set: {count} objects, but the module runs {slots}")));
        }
        let bytes =
            read_guest(&mut caller, ptr, (count * FRAME_ENTRY) as i32, slots * FRAME_ENTRY).map_err(|m| fault(&mut caller, format!("frame_set: {m}")))?;
        for (slot, entry) in bytes.as_chunks::<FRAME_ENTRY>().0.iter().enumerate() {
            let word = |at: usize| [entry[at], entry[at + 1], entry[at + 2], entry[at + 3]];
            if u32::from_le_bytes(word(36)) & FRAME_CHANGED == 0 {
                continue;
            }
            let values: [f32; 9] = std::array::from_fn(|i| f32::from_le_bytes(word(i * 4)));
            if !values.iter().all(|v| v.is_finite()) {
                return Err(fault(&mut caller, "frame_set: values must be finite numbers".to_owned()));
            }
            let state = caller.data_mut();
            if let Some(view) = state.slot_view(slot)
                && *view.running
            {
                // The entry's precise position, when the module set it (it narrows to the single-
                // precision one); else the single-precision one, keeping the old where it agrees.
                let precise: [f64; 3] = std::array::from_fn(|i| f64::from_le_bytes(std::array::from_fn(|b| entry[40 + i * 8 + b])));
                let narrowed = precise.map(|p| p as f32);
                *view.position = if narrowed == [values[0], values[1], values[2]] && precise.iter().all(|p| p.is_finite()) {
                    precise
                } else {
                    merged_position(&values, *view.position)
                };
                *view.transform = values;
                *view.changed = true;
            }
        }
        Ok(())
    })?;
    Ok(())
}

/// Lists and maps a visual script keeps (`list_*`, `map_*`): each by its name, made empty the first
/// time it is used. Values go and come as `event_send` lists them; an index past the end, or a key
/// a map lacks, reads as nothing and changes nothing.
fn link_collections(linker: &mut Linker<HostState>) -> Result<(), LinkError> {
    linker.func_wrap("bayleaf", "list_put", |mut caller: Caller<'_, HostState>, p: i32, l: i32, op: i32, index: i32, value: i32| -> Result<i32, Error> {
        let value = read_value(&mut caller, value, "list_put")?;
        let put = with_stored(&mut caller, p, l, true, "list_put", |s| {
            let Stored::List(items) = s else { return Ok(0) };
            let at = usize::try_from(index).ok();
            if op != 2 && items.len() >= MAX_COLLECTION_ITEMS {
                return Err(());
            }
            Ok(match op {
                0 => {
                    items.push(value);
                    1
                }
                1 => match at.filter(|i| *i <= items.len()) {
                    Some(i) => {
                        items.insert(i, value);
                        1
                    }
                    None => 0,
                },
                _ => match at.and_then(|i| items.get_mut(i)) {
                    Some(slot) => {
                        *slot = value;
                        1
                    }
                    None => 0,
                },
            })
        })?;
        put.map_err(|()| fault(&mut caller, format!("list_put: a list holds at most {MAX_COLLECTION_ITEMS} items")))
    })?;
    linker.func_wrap("bayleaf", "list_remove", |mut caller: Caller<'_, HostState>, p: i32, l: i32, index: i32| -> Result<i32, Error> {
        with_stored(&mut caller, p, l, true, "list_remove", |s| match (s, usize::try_from(index)) {
            (Stored::List(items), Ok(i)) if i < items.len() => {
                items.remove(i);
                1
            }
            _ => 0,
        })
    })?;
    linker.func_wrap("bayleaf", "list_clear", |mut caller: Caller<'_, HostState>, p: i32, l: i32| -> Result<(), Error> {
        with_stored(&mut caller, p, l, true, "list_clear", |s| {
            if let Stored::List(items) = s {
                items.clear();
            }
        })
    })?;
    linker.func_wrap("bayleaf", "list_len", |mut caller: Caller<'_, HostState>, p: i32, l: i32| -> Result<i32, Error> {
        with_stored(&mut caller, p, l, true, "list_len", |s| if let Stored::List(items) = s { items.len() as i32 } else { 0 })
    })?;
    linker.func_wrap("bayleaf", "list_get", |mut caller: Caller<'_, HostState>, p: i32, l: i32, index: i32, out: i32| -> Result<i32, Error> {
        match list_item(&mut caller, p, l, index, "list_get")? {
            Some(value) => write_value(&mut caller, out, &value, "list_get").map(|()| 1),
            None => Ok(0),
        }
    })?;
    linker.func_wrap("bayleaf", "list_text", |mut caller: Caller<'_, HostState>, p: i32, l: i32, index: i32, out: i32, cap: i32| -> Result<i32, Error> {
        let value = list_item(&mut caller, p, l, index, "list_text")?;
        write_text(&mut caller, value, out, cap, "list_text")
    })?;
    linker.func_wrap("bayleaf", "list_find", |mut caller: Caller<'_, HostState>, p: i32, l: i32, value: i32| -> Result<i32, Error> {
        let value = read_value(&mut caller, value, "list_find")?;
        with_stored(&mut caller, p, l, true, "list_find", |s| match s {
            Stored::List(items) => items.iter().position(|item| *item == value).map_or(-1, |i| i as i32),
            Stored::Map(_) => -1,
        })
    })?;
    link_maps(linker)
}

/// A list's item at `index`, if it has one.
fn list_item(caller: &mut Caller<'_, HostState>, p: i32, l: i32, index: i32, what: &str) -> Result<Option<EventArg>, Error> {
    with_stored(caller, p, l, true, what, |s| match (s, usize::try_from(index)) {
        (Stored::List(items), Ok(i)) => items.get(i).cloned(),
        _ => None,
    })
}

/// A map key read from the 16-byte value at `ptr`, as `event_send` lists values; `None` for an
/// object that is none.
fn map_key_at(caller: &mut Caller<'_, HostState>, ptr: i32, what: &str) -> Result<Option<MapKey>, Error> {
    let value = read_value(caller, ptr, what)?;
    Ok(MapKey::of(&value))
}

/// A map's value under the key at `key`, if it has one.
fn map_value(caller: &mut Caller<'_, HostState>, p: i32, l: i32, key: i32, what: &str) -> Result<Option<EventArg>, Error> {
    let Some(key) = map_key_at(caller, key, what)? else { return Ok(None) };
    with_stored(caller, p, l, false, what, |s| match s {
        Stored::Map(entries) => entries.get(&key).cloned(),
        Stored::List(_) => None,
    })
}

/// The map's first key, or the first after the key at `after` (below 0: from the start). For
/// Each in Map goes on from where it is, so keys added or removed on the way are no trouble.
fn map_next_key(caller: &mut Caller<'_, HostState>, p: i32, l: i32, after: i32, what: &str) -> Result<Option<MapKey>, Error> {
    let after = if after < 0 { None } else { map_key_at(caller, after, what)? };
    with_stored(caller, p, l, false, what, |s| match s {
        Stored::Map(entries) => match &after {
            None => entries.keys().next().cloned(),
            Some(after) => entries.range::<MapKey, _>((std::ops::Bound::Excluded(after), std::ops::Bound::Unbounded)).next().map(|(k, _)| k.clone()),
        },
        Stored::List(_) => None,
    })
}

/// The map half of [`link_collections`]: values by keys of one type, in key order. A key goes and
/// comes as a value does (16 bytes, as `event_send` lists values).
fn link_maps(linker: &mut Linker<HostState>) -> Result<(), LinkError> {
    linker.func_wrap("bayleaf", "map_put", |mut caller: Caller<'_, HostState>, p: i32, l: i32, key: i32, value: i32| -> Result<i32, Error> {
        let Some(key) = map_key_at(&mut caller, key, "map_put")? else { return Ok(0) };
        let value = read_value(&mut caller, value, "map_put")?;
        let put = with_stored(&mut caller, p, l, false, "map_put", |s| {
            let Stored::Map(entries) = s else { return Ok(0) };
            if !entries.contains_key(&key) && entries.len() >= MAX_COLLECTION_ITEMS {
                return Err(());
            }
            entries.insert(key, value);
            Ok(1)
        })?;
        put.map_err(|()| fault(&mut caller, format!("map_put: a map holds at most {MAX_COLLECTION_ITEMS} keys")))
    })?;
    linker.func_wrap("bayleaf", "map_remove", |mut caller: Caller<'_, HostState>, p: i32, l: i32, key: i32| -> Result<i32, Error> {
        let Some(key) = map_key_at(&mut caller, key, "map_remove")? else { return Ok(0) };
        with_stored(&mut caller, p, l, false, "map_remove", |s| match s {
            Stored::Map(entries) => i32::from(entries.remove(&key).is_some()),
            Stored::List(_) => 0,
        })
    })?;
    linker.func_wrap("bayleaf", "map_clear", |mut caller: Caller<'_, HostState>, p: i32, l: i32| -> Result<(), Error> {
        with_stored(&mut caller, p, l, false, "map_clear", |s| {
            if let Stored::Map(entries) = s {
                entries.clear();
            }
        })
    })?;
    linker.func_wrap("bayleaf", "map_count", |mut caller: Caller<'_, HostState>, p: i32, l: i32| -> Result<i32, Error> {
        with_stored(&mut caller, p, l, false, "map_count", |s| if let Stored::Map(entries) = s { entries.len() as i32 } else { 0 })
    })?;
    linker.func_wrap("bayleaf", "map_get", |mut caller: Caller<'_, HostState>, p: i32, l: i32, key: i32, out: i32| -> Result<i32, Error> {
        match map_value(&mut caller, p, l, key, "map_get")? {
            Some(value) => write_value(&mut caller, out, &value, "map_get").map(|()| 1),
            None => Ok(0),
        }
    })?;
    linker.func_wrap("bayleaf", "map_text", |mut caller: Caller<'_, HostState>, p: i32, l: i32, key: i32, out: i32, cap: i32| -> Result<i32, Error> {
        let value = map_value(&mut caller, p, l, key, "map_text")?;
        write_text(&mut caller, value, out, cap, "map_text")
    })?;
    linker.func_wrap("bayleaf", "map_next", |mut caller: Caller<'_, HostState>, p: i32, l: i32, after: i32, out: i32| -> Result<i32, Error> {
        match map_next_key(&mut caller, p, l, after, "map_next")? {
            Some(key) => write_value(&mut caller, out, &key.arg(), "map_next").map(|()| 1),
            None => Ok(0),
        }
    })?;
    linker.func_wrap(
        "bayleaf",
        "map_next_text",
        |mut caller: Caller<'_, HostState>, p: i32, l: i32, after: i32, out: i32, cap: i32| -> Result<i32, Error> {
            let key = map_next_key(&mut caller, p, l, after, "map_next_text")?;
            write_text(&mut caller, key.map(|k| k.arg()), out, cap, "map_next_text")
        },
    )?;
    Ok(())
}
