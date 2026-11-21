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
use bayleaf_scene::components::{Script, Transform};
use wasmi::{Caller, Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc};

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
    #[error("{0} is not a valid WebAssembly module: {1}")]
    Invalid(String, String),
    #[error("{module}: {message}")]
    Trapped { module: String, message: String },
    #[error("{module} uses script ABI {found}, but this engine needs {expected}")]
    AbiMismatch { module: String, found: i32, expected: i32 },
}

/// One problem with one object's script.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptFault {
    pub object: StableId,
    pub error: ScriptError,
}

/// A line written by a script.
#[derive(Clone, Debug, PartialEq)]
pub struct LogLine {
    pub object: StableId,
    pub text: String,
}

/// Per-frame input the host passes to scripts.
#[derive(Clone, Debug, Default)]
pub struct Input {
    /// Names of the keys held down, as in `docs/script-abi.md`.
    pub keys: HashSet<String>,
}

/// State one instance's host functions can see.
struct HostState {
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

struct Instance {
    object: StableId,
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
}

impl Default for ScriptHost {
    fn default() -> Self {
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let linker = make_linker(&engine);
        Self { engine, linker, instances: Vec::new(), faults: Vec::new(), logs: Vec::new() }
    }
}

impl ScriptHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads the scripts of every object that has a `script` component. Module paths are relative
    /// to `base_dir`. Problems are recorded as faults; the other scripts still load.
    pub fn load_scene(&mut self, scene: &mut Scene, base_dir: &Path, time: f64, input: &Arc<Input>) {
        let objects: Vec<StableId> = scene.world_matrices().into_iter().map(|(id, _)| id).collect();
        for object in objects {
            if let Some(script) = scene.get::<Script>(object).cloned() {
                let path = base_dir.join(&script.module);
                if let Err(error) = self.load_file(object, &path, &script, scene, time, input) {
                    self.faults.push(ScriptFault { object, error });
                }
            }
        }
    }

    /// Loads one module file for `object` and runs its `bayleaf_start`.
    pub fn load_file(&mut self, object: StableId, path: &Path, script: &Script, scene: &mut Scene, time: f64, input: &Arc<Input>) -> Result<(), ScriptError> {
        let bytes = std::fs::read(path).map_err(|e| ScriptError::Read { path: path.display().to_string(), message: e.to_string() })?;
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        self.load_bytes(object, path, modified, &bytes, script, scene, time, input)
    }

    /// Loads a module from memory; `path` names it in messages and for hot reload.
    #[allow(clippy::too_many_arguments)]
    fn load_bytes(
        &mut self,
        object: StableId,
        path: &Path,
        modified: Option<SystemTime>,
        bytes: &[u8],
        script: &Script,
        scene: &mut Scene,
        time: f64,
        input: &Arc<Input>,
    ) -> Result<(), ScriptError> {
        let module_name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        let module = Module::new(&self.engine, bytes).map_err(|e| ScriptError::Invalid(module_name.clone(), e.to_string()))?;
        let state = HostState {
            transform: read_transform(scene, object),
            transform_changed: false,
            properties: script.properties.clone(),
            time,
            input: Arc::clone(input),
            logs: Vec::new(),
            fault: None,
            limits: StoreLimitsBuilder::new().memory_size(MEMORY_LIMIT).instances(1).build(),
        };
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
        let update = instance.get_typed_func::<f32, ()>(&store, "bayleaf_update").ok();
        let start = instance.get_typed_func::<(), ()>(&store, "bayleaf_start").ok();

        // Replace an earlier instance of the same object and module (hot reload).
        self.instances.retain(|i| !(i.object == object && i.path == path));
        let mut entry = Instance { object, path: path.to_path_buf(), module_name, modified, store, update, stopped: false };
        if let Some(start) = start {
            let result = call(&mut entry, |store| start.call(store, ()));
            self.collect(&mut entry);
            write_transform(scene, &mut entry);
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
            }
            let result = call(entry, |store| update.call(store, dt));
            logs.extend(entry.store.data_mut().logs.drain(..).map(|text| LogLine { object: entry.object, text }));
            match result {
                Ok(()) => write_transform(scene, entry),
                Err(error) => {
                    entry.stopped = true;
                    faults.push(ScriptFault { object: entry.object, error });
                }
            }
        }
        self.faults.extend(faults);
        self.push_logs(logs);
    }

    /// Reloads modules whose file changed since it was loaded. Returns the reloaded module names.
    pub fn reload_changed(&mut self, scene: &mut Scene, time: f64, input: &Arc<Input>) -> Vec<String> {
        let changed: Vec<(StableId, PathBuf)> = self
            .instances
            .iter()
            .filter(|i| {
                let now = std::fs::metadata(&i.path).and_then(|m| m.modified()).ok();
                now.is_some() && now != i.modified
            })
            .map(|i| (i.object, i.path.clone()))
            .collect();
        let mut reloaded = Vec::new();
        for (object, path) in changed {
            let Some(script) = scene.get::<Script>(object).cloned() else { continue };
            match self.load_file(object, &path, &script, scene, time, input) {
                Ok(()) => reloaded.push(path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())),
                Err(error) => {
                    // Keep the old module's timestamp current so a broken file is reported once.
                    if let Some(entry) = self.instances.iter_mut().find(|i| i.object == object && i.path == path) {
                        entry.modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                    }
                    self.faults.push(ScriptFault { object, error });
                }
            }
        }
        reloaded.sort();
        reloaded.dedup();
        reloaded
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
        let lines: Vec<LogLine> = entry.store.data_mut().logs.drain(..).map(|text| LogLine { object: entry.object, text }).collect();
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
        Ok(())
    })();
    // The names are fixed and unique, so defining them cannot fail.
    debug_assert!(result.is_ok());
    linker
}

#[cfg(test)]
mod tests;
