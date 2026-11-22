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
use bayleaf_scene::components::{DeclaredProperty, PropertyType, PropertyValue, ScriptRef, Scripts};

use super::rt::{self, Caller, Engine, Linker, Module, Store, TypedFunc};
use super::{Error, LinkError, error, from_val, instantiate, limits, out_of_fuel, to_val, trap_message};
use crate::{
    ABI_VERSION, Description, EVERY_OBJECT, FUEL_PER_CALL, GameState, Input, LogLine, MAX_DESCRIPTION_BYTES, MAX_LOG_BYTES, MAX_LOG_LINES, MAX_MESSAGE_NAME,
    MAX_MESSAGES_PER_CALL, MAX_OBJECT_REFS, MAX_REQUESTS_PER_CALL, Message, Properties, PropertyWrite, Request, Scope, ScriptError, ScriptFault, ScriptValues,
    SharedState, Velocities, WorldView, actions, field_numbers, kind_key, lock, menu_path, merged_position, module_bytes, read_position, read_transform,
    readable_component, script_keys, transform_from, visual,
};
use bayleaf_script_sdk::native::{Bridge, NativeObject, Value};

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
    let inspector = match value.get("inspector") {
        Some(layout) => bayleaf_scene::inspector::parse(layout, &out).map_err(|e| invalid(format!("inspector: {e}")))?,
        None => Vec::new(),
    };
    Ok(Description { properties: out, menu, inspector })
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
    /// Modules by their bytes, so a script on many objects is compiled once.
    modules: HashMap<Vec<u8>, Module>,
    /// Scripts built into the game, by the module path they stand for (native mode).
    natives: Vec<(PathBuf, crate::NativeScript)>,
    /// How long each script's update took since they were last taken, by script name.
    timings: Vec<(String, std::time::Duration)>,
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

impl ScriptHost {
    /// A host whose scripts run on `engine` (see the runtime's `engine`).
    pub(crate) fn from_engine(engine: Engine) -> Self {
        let linker = make_linker(&engine);
        Self {
            engine,
            linker,
            modules: HashMap::new(),
            natives: Vec::new(),
            timings: Vec::new(),
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
        let mut state = HostState::new(object, read_transform(scene, object), Properties::new(), time, input, limits());
        state.position = read_position(scene, object);
        state.state = Arc::clone(&self.state);
        state.input_maps = Arc::clone(&self.input_maps);
        state.world = WorldView::of(scene, self.script_values());
        let mut store = Store::new(&self.engine, state);
        store.limiter(|state| &mut state.limits);
        let trapped = |module: &str, e: Error| ScriptError::Trapped { module: module.to_owned(), message: trap_message(&e) };
        store.set_fuel(FUEL_PER_CALL).map_err(|e| trapped(&module_name, e))?;
        let instance = instantiate(&self.linker, &mut store, &module).map_err(|e| trapped(&module_name, e))?;
        store.data_mut().native = native.map(|script| script.create());

        let abi = instance
            .get_typed_func::<(), i32>(&mut store, "bayleaf_abi")
            .map_err(|_| ScriptError::Invalid(module_name.clone(), "it has no bayleaf_abi export".to_owned()))?;
        let found = abi.call(&mut store, ()).map_err(|e| trapped(&module_name, e))?;
        if found != ABI_VERSION {
            return Err(ScriptError::AbiMismatch { module: module_name, found, expected: ABI_VERSION });
        }
        if instance.get_export(&mut store, "memory").and_then(rt::Extern::into_memory).is_none() {
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
        let update = instance.get_typed_func::<f32, ()>(&mut store, "bayleaf_update").ok();
        let on_message = instance.get_typed_func::<f32, ()>(&mut store, "bayleaf_message").ok();
        let init = instance.get_typed_func::<(), ()>(&mut store, "bayleaf_init").ok();
        let start = instance.get_typed_func::<(), ()>(&mut store, "bayleaf_start").ok();

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
            let Some(update) = entry.update.as_ref().cloned() else { continue };
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
            let started = std::time::Instant::now();
            let result = call(entry, |store| update.call(store, dt));
            self.timings.push((entry.script_name.clone(), started.elapsed()));
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
                let Some(on_message) = entry.on_message.as_ref().cloned() else { continue };
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

    /// How long each script's update took since this was last asked (for the profiler).
    pub fn take_timings(&mut self) -> Vec<(String, std::time::Duration)> {
        std::mem::take(&mut self.timings)
    }

    /// Sends the scripts of `to` a message now (a button of the Inspector, `play.message`).
    pub fn send(&mut self, scene: &mut Scene, to: StableId, name: &str, value: f32, time: f64, input: &Arc<Input>) {
        self.messages.push(Message { to: Some(to), name: name.to_owned(), value });
        self.deliver(scene, time, input);
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
fn call(entry: &mut Instance, run: impl FnOnce(&mut Store<HostState>) -> Result<(), Error>) -> Result<(), ScriptError> {
    let trapped = |message: String| ScriptError::Trapped { module: entry.module_name.clone(), message };
    entry.store.set_fuel(FUEL_PER_CALL).map_err(|e| trapped(e.to_string()))?;
    entry.store.data_mut().fault = None;
    match run(&mut entry.store) {
        Ok(()) => Ok(()),
        Err(error) => {
            let reason = entry.store.data_mut().fault.take().unwrap_or_else(|| {
                if out_of_fuel(&error) { format!("ran longer than the limit of {FUEL_PER_CALL} instructions in one call") } else { trap_message(&error) }
            });
            Err(ScriptError::Trapped { module: entry.module_name.clone(), message: reason })
        }
    }
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
    let Some((object, _, _)) = caller.data().world.get(handle).cloned() else {
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
fn state_scope(caller: &mut Caller<'_, HostState>, what: &str, scope: i32, handle: i32) -> Result<Option<Scope>, Error> {
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

/// The SDK's way to the host for a native script: each call goes through the trampoline's
/// `call_<name>` export, so the host function runs as for a WebAssembly script.
struct NativeBridge<'a, 'c> {
    caller: &'a mut Caller<'c, HostState>,
    /// The first call that failed; later calls do nothing.
    failed: Option<Error>,
}

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
            Ok(state.world.objects.iter().position(|(_, n, _)| n == name).map_or(0, |i| i as i32 + 1))
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
            Ok(caller.data().world.objects.iter().position(|(_, n, _)| *n == name).map_or(0, |i| i as i32 + 1))
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
            let name = caller.data().world.get(handle).map(|(_, name, _)| name.clone()).unwrap_or_default();
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
        for (name, stop) in [("sound_play", false), ("sound_stop", true)] {
            linker.func_wrap("bayleaf", name, move |mut caller: Caller<'_, HostState>, handle: i32| -> Result<i32, Error> {
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
        linker.func_wrap("bayleaf", "send", |mut caller: Caller<'_, HostState>, handle: i32, ptr: i32, len: i32, value: f32| -> Result<i32, Error> {
            let name = read_text(&mut caller, ptr, len, MAX_MESSAGE_NAME).map_err(|m| fault(&mut caller, format!("send: {m}")))?;
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
        link_native(&mut linker)?;
        Ok(())
    })();
    // The names are fixed and unique, so defining them cannot fail.
    debug_assert!(result.is_ok());
    linker
}
