//! L4: WebAssembly script host. Implements the ABI in `docs/script-abi.md`.
//!
//! Each `script` component gets its own WebAssembly instance (wasmi, a pure-Rust interpreter that
//! also runs inside the web build). Every call is metered with fuel and memory is capped, so a
//! broken script stops itself, not the editor.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use bayleaf_core::StableId;
use bayleaf_scene::Scene;
use bayleaf_scene::components::{ScriptRef, Scripts, Transform};
use wasmi::{Caller, Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc};

pub mod visual;

/// File extension of visual scripts, which compile to modules when loaded.
pub const VISUAL_EXTENSION: &str = "bscript";

/// The WebAssembly module of a script file: a `.wasm` file as it is, a visual script compiled.
pub fn module_bytes(path: &Path) -> Result<Vec<u8>, ScriptError> {
    let read_error = |e: std::io::Error| ScriptError::Read { path: path.display().to_string(), message: e.to_string() };
    if path.extension().is_some_and(|e| e == VISUAL_EXTENSION) {
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        let text = std::fs::read_to_string(path).map_err(read_error)?;
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
}

/// Something a script asked for that changes which objects exist. The play loop does these
/// after the step, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Place a copy of a prefab file at a world position. `by` is the asking script's object.
    Spawn { by: StableId, prefab: String, position: [f32; 3] },
    /// Delete an object and its children.
    Destroy(StableId),
}

/// Most requests one script call may make.
const MAX_REQUESTS_PER_CALL: usize = 64;

/// The objects scripts can see during one step: names and local transforms as the step began.
/// A handle is an object's position in this list plus one; 0 means none.
#[derive(Default)]
struct WorldView {
    objects: Vec<(StableId, String, [f32; 9])>,
}

impl WorldView {
    fn of(scene: &Scene) -> Arc<Self> {
        let objects =
            scene.world_matrices().into_iter().map(|(id, _)| (id, scene.name(id).unwrap_or_default().to_owned(), read_transform(scene, id))).collect();
        Arc::new(Self { objects })
    }

    fn handle_of(&self, id: StableId) -> i32 {
        self.objects.iter().position(|(o, _, _)| *o == id).map_or(0, |i| i as i32 + 1)
    }

    fn get(&self, handle: i32) -> Option<&(StableId, String, [f32; 9])> {
        usize::try_from(handle).ok().and_then(|h| h.checked_sub(1)).and_then(|i| self.objects.get(i))
    }
}

/// State one instance's host functions can see.
struct HostState {
    /// The object this instance belongs to.
    object: StableId,
    world: Arc<WorldView>,
    /// Transforms of other objects this call set, applied after it.
    writes: Vec<(StableId, [f32; 9])>,
    requests: Vec<Request>,
    transform: [f32; 9],
    transform_changed: bool,
    properties: BTreeMap<String, f32>,
    time: f64,
    input: Arc<Input>,
    logs: Vec<String>,
    /// Set by a host function that detected a guest error, so the trap message says why.
    fault: Option<String>,
    limits: StoreLimits,
}

impl HostState {
    fn new(object: StableId, transform: [f32; 9], properties: BTreeMap<String, f32>, time: f64, input: &Arc<Input>) -> Self {
        Self {
            object,
            world: Arc::new(WorldView::default()),
            writes: Vec::new(),
            requests: Vec::new(),
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

/// Calls the optional `bayleaf_describe` export and parses the declared properties.
/// The export returns `(ptr << 32) | len` of UTF-8 JSON: `{"properties": [["speed", 90.0], ...]}`.
fn describe_instance(store: &mut Store<HostState>, instance: &wasmi::Instance, name: &str) -> Result<Vec<(String, f32)>, ScriptError> {
    let Ok(describe) = instance.get_typed_func::<(), i64>(&*store, "bayleaf_describe") else {
        return Ok(Vec::new());
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
        let pair = entry.as_array().filter(|p| p.len() == 2);
        let (Some(key), Some(default)) = (pair.and_then(|p| p[0].as_str()), pair.and_then(|p| p[1].as_f64())) else {
            return Err(invalid(format!("each property must be [name, number], got {entry}")));
        };
        let default = default as f32;
        if key.is_empty() || key.len() > 64 || !default.is_finite() {
            return Err(invalid(format!("property {key:?} needs a name of 1 to 64 bytes and a finite default")));
        }
        out.push((key.to_owned(), default));
    }
    Ok(out)
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
    /// Stopped after an error; it no longer runs.
    stopped: bool,
}

/// All script instances of one play session.
pub struct ScriptHost {
    engine: Engine,
    linker: Linker<HostState>,
    instances: Vec<Instance>,
    faults: Vec<ScriptFault>,
    logs: Vec<LogLine>,
    requests: Vec<Request>,
}

impl Default for ScriptHost {
    fn default() -> Self {
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let linker = make_linker(&engine);
        Self { engine, linker, instances: Vec::new(), faults: Vec::new(), logs: Vec::new(), requests: Vec::new() }
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
                if let Err(error) = self.load_file(object, index, &path, script, scene, time, input) {
                    self.faults.push(ScriptFault { object, script: script.display_name(), error });
                }
            }
        }
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

    /// The properties a module declares, with their defaults, in declaration order. Runs no script
    /// code but `bayleaf_describe`; a module without it declares nothing.
    pub fn describe(&self, name: &str, bytes: &[u8]) -> Result<Vec<(String, f32)>, ScriptError> {
        let module = Module::new(&self.engine, bytes).map_err(|e| ScriptError::Invalid(name.to_owned(), e.to_string()))?;
        let mut store = Store::new(&self.engine, HostState::new(StableId::from_u128(0), [0.0; 9], BTreeMap::new(), 0.0, &Arc::new(Input::default())));
        store.limiter(|state| &mut state.limits);
        store.set_fuel(FUEL_PER_CALL).map_err(|e| ScriptError::Trapped { module: name.to_owned(), message: e.to_string() })?;
        let instance =
            self.linker.instantiate_and_start(&mut store, &module).map_err(|e| ScriptError::Trapped { module: name.to_owned(), message: e.to_string() })?;
        describe_instance(&mut store, &instance, name)
    }

    /// Loads a module from memory; `path` names it in messages and for hot reload.
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
        let module_name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        let module = Module::new(&self.engine, bytes).map_err(|e| ScriptError::Invalid(module_name.clone(), e.to_string()))?;
        let mut state = HostState::new(object, read_transform(scene, object), script.properties.clone(), time, input);
        state.world = WorldView::of(scene);
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
        // Declared defaults, overridden by the values saved in the scene.
        let mut properties: BTreeMap<String, f32> = describe_instance(&mut store, &instance, &module_name)?.into_iter().collect();
        properties.extend(script.properties.iter().map(|(k, v)| (k.clone(), *v)));
        store.data_mut().properties = properties;
        let update = instance.get_typed_func::<f32, ()>(&store, "bayleaf_update").ok();
        let start = instance.get_typed_func::<(), ()>(&store, "bayleaf_start").ok();

        // Replace an earlier instance in the same slot (hot reload).
        self.instances.retain(|i| !(i.object == object && i.index == index));
        let script_name = script.display_name();
        let mut entry = Instance { object, index, script_name, path: path.to_path_buf(), module_name, modified, store, update, stopped: false };
        if let Some(start) = start {
            let result = call(&mut entry, |store| start.call(store, ()));
            self.collect(&mut entry);
            after_call(scene, &mut entry, &mut self.requests);
            if let Err(error) = result {
                entry.stopped = true;
                self.instances.push(entry);
                return Err(error);
            }
        }
        self.instances.push(entry);
        Ok(())
    }

    /// Runs `bayleaf_update(dt)` on every running script and writes changed transforms back.
    pub fn update(&mut self, scene: &mut Scene, dt: f32, time: f64, input: &Arc<Input>) {
        let mut faults = Vec::new();
        let mut logs = Vec::new();
        // Every script sees the objects as the step began.
        let world = WorldView::of(scene);
        for entry in self.instances.iter_mut().filter(|i| !i.stopped) {
            if !scene.contains(entry.object) {
                entry.stopped = true;
                continue;
            }
            let Some(update) = entry.update else { continue };
            {
                let state = entry.store.data_mut();
                state.transform = read_transform(scene, entry.object);
                state.transform_changed = false;
                state.time = time;
                state.input = Arc::clone(input);
                state.world = Arc::clone(&world);
            }
            let result = call(entry, |store| update.call(store, dt));
            let script = entry.script_name.clone();
            logs.extend(entry.store.data_mut().logs.drain(..).map(|text| LogLine { object: entry.object, script: script.clone(), text }));
            match result {
                Ok(()) => after_call(scene, entry, &mut self.requests),
                Err(error) => {
                    entry.stopped = true;
                    faults.push(ScriptFault { object: entry.object, script: entry.script_name.clone(), error });
                }
            }
        }
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

    fn collect(&mut self, entry: &mut Instance) {
        let script = entry.script_name.clone();
        let lines: Vec<LogLine> = entry.store.data_mut().logs.drain(..).map(|text| LogLine { object: entry.object, script: script.clone(), text }).collect();
        self.push_logs(lines);
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

fn read_transform(scene: &Scene, object: StableId) -> [f32; 9] {
    let t = scene.get::<Transform>(object).cloned().unwrap_or_default();
    [t.position[0], t.position[1], t.position[2], t.rotation[0], t.rotation[1], t.rotation[2], t.scale[0], t.scale[1], t.scale[2]]
}

/// Applies what a call changed: its own transform, other objects' transforms, and its requests.
fn after_call(scene: &mut Scene, entry: &mut Instance, requests: &mut Vec<Request>) {
    write_transform(scene, entry);
    let state = entry.store.data_mut();
    for (object, v) in state.writes.drain(..) {
        if scene.contains(object) {
            let _ = scene.set(object, Transform { position: [v[0], v[1], v[2]], rotation: [v[3], v[4], v[5]], scale: [v[6], v[7], v[8]] });
        }
    }
    requests.append(&mut state.requests);
}

fn write_transform(scene: &mut Scene, entry: &mut Instance) {
    let state = entry.store.data_mut();
    if !state.transform_changed {
        return;
    }
    state.transform_changed = false;
    let v = state.transform;
    let transform = Transform { position: [v[0], v[1], v[2]], rotation: [v[3], v[4], v[5]], scale: [v[6], v[7], v[8]] };
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
            state.transform = values;
            state.transform_changed = true;
            Ok(())
        })?;
        linker.func_wrap("bayleaf", "property", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<f32, wasmi::Error> {
            let key = read_text(&caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("property: {m}")))?;
            Ok(caller.data().properties.get(&key).copied().unwrap_or(f32::NAN))
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
        linker.func_wrap("bayleaf", "self_handle", |caller: Caller<'_, HostState>| -> i32 {
            let state = caller.data();
            state.world.handle_of(state.object)
        })?;
        linker.func_wrap("bayleaf", "find", |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<i32, wasmi::Error> {
            let name = read_text(&caller, ptr, len, 256).map_err(|m| fault(&mut caller, format!("find: {m}")))?;
            Ok(caller.data().world.objects.iter().position(|(_, n, _)| *n == name).map_or(0, |i| i as i32 + 1))
        })?;
        linker.func_wrap("bayleaf", "transform_get_of", |mut caller: Caller<'_, HostState>, handle: i32, out: i32| -> Result<(), wasmi::Error> {
            let state = caller.data();
            let Some((object, _, start)) = state.world.get(handle).cloned() else {
                return Err(fault(&mut caller, format!("transform_get_of: there is no object with handle {handle}")));
            };
            // The latest value this call set, else the value at the start of the step.
            let value = if object == state.object { state.transform } else { state.writes.iter().rev().find(|(o, _)| *o == object).map_or(start, |(_, v)| *v) };
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
            let Some((object, _, _)) = caller.data().world.get(handle).cloned() else {
                return Err(fault(&mut caller, format!("transform_set_of: there is no object with handle {handle}")));
            };
            let state = caller.data_mut();
            if object == state.object {
                state.transform = values;
                state.transform_changed = true;
            } else {
                state.writes.push((object, values));
            }
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
        Ok(())
    })();
    // The names are fixed and unique, so defining them cannot fail.
    debug_assert!(result.is_ok());
    linker
}

#[cfg(test)]
mod tests;
