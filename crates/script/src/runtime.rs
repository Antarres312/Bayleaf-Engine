//! The script host as the rest of the engine sees it: the same calls whichever runtime runs the
//! scripts, interpreted (everywhere) or compiled to machine code (exported desktop games).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bayleaf_core::StableId;
use bayleaf_scene::Scene;
use bayleaf_scene::components::{DeclaredProperty, ScriptRef};

use crate::{Description, GameState, Input, LogLine, Request, ScriptError, ScriptFault, Velocities, actions};

/// How scripts run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Runtime {
    /// By an interpreter: they start at once, everywhere.
    #[default]
    Interpreted,
    /// Compiled to machine code when they load: much faster, desktop only.
    Compiled,
}

impl Runtime {
    /// Whether this build can run scripts this way.
    pub fn available(self) -> bool {
        match self {
            Runtime::Interpreted => true,
            Runtime::Compiled => cfg!(feature = "compiled"),
        }
    }
}

enum Backend {
    Interpreted(crate::interpreted::host::ScriptHost),
    #[cfg(feature = "compiled")]
    Compiled(crate::compiled::host::ScriptHost),
}

/// All script instances of one play session, in the runtime chosen when it was made.
pub struct ScriptHost(Backend);

/// Calls `$call` on the host whichever runtime it is.
macro_rules! each {
    ($backend:expr, $host:ident => $call:expr) => {
        match $backend {
            Backend::Interpreted($host) => $call,
            #[cfg(feature = "compiled")]
            Backend::Compiled($host) => $call,
        }
    };
}

impl Default for ScriptHost {
    fn default() -> Self {
        Self(Backend::Interpreted(crate::interpreted::host::ScriptHost::from_engine(crate::interpreted::engine())))
    }
}

impl ScriptHost {
    /// A host whose scripts are interpreted.
    pub fn new() -> Self {
        Self::default()
    }

    /// A host whose scripts run as `runtime` says; an error when this build cannot run them so,
    /// or the compiler cannot start.
    pub fn with_runtime(runtime: Runtime) -> Result<Self, ScriptError> {
        match runtime {
            Runtime::Interpreted => Ok(Self::new()),
            #[cfg(feature = "compiled")]
            Runtime::Compiled => {
                let engine = crate::compiled::engine().map_err(ScriptError::Unavailable)?;
                Ok(Self(Backend::Compiled(crate::compiled::host::ScriptHost::from_engine(engine))))
            }
            #[cfg(not(feature = "compiled"))]
            Runtime::Compiled => Err(ScriptError::Unavailable("this program runs scripts only by interpreting them".to_owned())),
        }
    }

    /// Runs the modules at these paths (relative to the game, `scripts/mover.wasm`) as the native
    /// scripts built into the game instead: machine code with no sandbox (native mode).
    pub fn with_natives(mut self, natives: Vec<(PathBuf, crate::NativeScript)>) -> Self {
        each!(&mut self.0, host => host.set_natives(natives));
        self
    }

    pub fn runtime(&self) -> Runtime {
        match &self.0 {
            Backend::Interpreted(_) => Runtime::Interpreted,
            #[cfg(feature = "compiled")]
            Backend::Compiled(_) => Runtime::Compiled,
        }
    }

    /// Loads every script of every object. Module paths are relative to `base_dir`. Problems are
    /// recorded as faults; the other scripts still load.
    pub fn load_scene(&mut self, scene: &mut Scene, base_dir: &Path, time: f64, input: &Arc<Input>) {
        each!(&mut self.0, host => host.load_scene(scene, base_dir, time, input))
    }

    /// Loads several module files (the scripts of a placed prefab), then runs their inits and
    /// starts together. Returns the scripts that could not load or start.
    pub fn load_files(
        &mut self,
        scripts: &[(StableId, usize, PathBuf, ScriptRef)],
        scene: &mut Scene,
        time: f64,
        input: &Arc<Input>,
    ) -> Vec<(StableId, usize, ScriptError)> {
        each!(&mut self.0, host => host.load_files(scripts, scene, time, input))
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
        each!(&mut self.0, host => host.load_file(object, index, path, script, scene, time, input))
    }

    /// Loads a module from memory and runs its `bayleaf_start`, for tests.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn load_bytes(
        &mut self,
        object: StableId,
        index: usize,
        path: &Path,
        modified: Option<std::time::SystemTime>,
        bytes: &[u8],
        script: &ScriptRef,
        scene: &mut Scene,
        time: f64,
        input: &Arc<Input>,
    ) -> Result<(), ScriptError> {
        each!(&mut self.0, host => host.load_bytes(object, index, path, modified, bytes, script, scene, time, input))
    }

    /// The properties a module declares, with their defaults, in declaration order.
    pub fn describe(&self, name: &str, bytes: &[u8]) -> Result<Vec<DeclaredProperty>, ScriptError> {
        each!(&self.0, host => host.describe(name, bytes))
    }

    /// What a module says about itself: its properties and where Add Component lists it.
    pub fn describe_all(&self, name: &str, bytes: &[u8]) -> Result<Description, ScriptError> {
        each!(&self.0, host => host.describe_all(name, bytes))
    }

    /// Runs `bayleaf_update(dt)` on every running script and writes changed transforms back.
    pub fn update(&mut self, scene: &mut Scene, dt: f32, time: f64, input: &Arc<Input>) {
        each!(&mut self.0, host => host.update(scene, dt, time, input))
    }

    /// Loads again the modules whose files changed; returns their names.
    pub fn reload_changed(&mut self, scene: &mut Scene, time: f64, input: &Arc<Input>) -> Vec<String> {
        each!(&mut self.0, host => host.reload_changed(scene, time, input))
    }

    pub fn set_velocities(&mut self, velocities: Velocities) {
        each!(&mut self.0, host => host.set_velocities(velocities))
    }

    pub fn state(&self) -> GameState {
        each!(&self.0, host => host.state())
    }

    pub fn with_state<T>(&self, change: impl FnOnce(&mut GameState) -> T) -> T {
        each!(&self.0, host => host.with_state(change))
    }

    pub fn broadcast(&mut self, scene: &mut Scene, name: &str, value: f32, time: f64, input: &Arc<Input>) {
        each!(&mut self.0, host => host.broadcast(scene, name, value, time, input))
    }

    /// Sends the scripts of `to` a message now.
    pub fn send(&mut self, scene: &mut Scene, to: StableId, name: &str, value: f32, time: f64, input: &Arc<Input>) {
        each!(&mut self.0, host => host.send(scene, to, name, value, time, input))
    }

    pub fn with_input<T>(&self, change: impl FnOnce(&mut actions::InputSet) -> T) -> T {
        each!(&self.0, host => host.with_input(change))
    }

    pub fn set_grounded(&mut self, grounded: impl IntoIterator<Item = StableId>) {
        each!(&mut self.0, host => host.set_grounded(grounded))
    }

    pub fn notify(&mut self, scene: &mut Scene, contacts: &[(StableId, &str, StableId)], time: f64, input: &Arc<Input>) {
        each!(&mut self.0, host => host.notify(scene, contacts, time, input))
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        each!(&mut self.0, host => host.take_requests())
    }

    pub fn len(&self) -> usize {
        each!(&self.0, host => host.len())
    }

    pub fn is_empty(&self) -> bool {
        each!(&self.0, host => host.is_empty())
    }

    pub fn running(&self) -> usize {
        each!(&self.0, host => host.running())
    }

    pub fn take_faults(&mut self) -> Vec<ScriptFault> {
        each!(&mut self.0, host => host.take_faults())
    }

    pub fn take_logs(&mut self) -> Vec<LogLine> {
        each!(&mut self.0, host => host.take_logs())
    }
}
