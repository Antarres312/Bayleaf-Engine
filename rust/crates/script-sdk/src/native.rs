//! Native mode: a game exported this way builds its Rust scripts into its player, so they run as
//! machine code with no sandbox. The script's code is the same; the SDK reaches the host through a
//! [`Bridge`] instead of WebAssembly imports, and each object gets its own value of the script.
//!
//! Engine-side only: scripts do not use this module themselves.

use crate::Script;

/// A number passed to or returned from a host function, as WebAssembly types it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
}

impl Value {
    pub fn i32(self) -> Option<i32> {
        match self {
            Value::I32(v) => Some(v),
            _ => None,
        }
    }

    pub fn f32(self) -> Option<f32> {
        match self {
            Value::F32(v) => Some(v),
            _ => None,
        }
    }

    pub fn f64(self) -> Option<f64> {
        match self {
            Value::F64(v) => Some(v),
            _ => None,
        }
    }
}

/// How a native script reaches the host while one of its functions runs.
pub trait Bridge {
    /// Calls the host function `name` (an import of `docs/script-abi.md`) with `args`, where texts
    /// and numbers passed by address are offsets in the exchange memory. Returns its result
    /// (`I32(0)` for none), or `None` once a call has failed: the script is then stopped and further
    /// calls do nothing.
    fn call(&mut self, name: &str, args: &[Value]) -> Option<Value>;
    /// Reads exchange memory at `at`; false when it is outside.
    fn read(&mut self, at: u32, out: &mut [u8]) -> bool;
    /// Writes exchange memory at `at`; false when it is outside.
    fn write(&mut self, at: u32, bytes: &[u8]) -> bool;
}

/// Runs `f` with `bridge` as the host of every SDK call it makes on this thread.
#[cfg(not(target_arch = "wasm32"))]
pub fn with_bridge<R>(bridge: &mut dyn Bridge, f: impl FnOnce() -> R) -> R {
    crate::abi::with_bridge(bridge, f)
}

/// One object's value of a native script.
pub trait NativeObject: Send {
    /// The `bayleaf_describe` text: the script's properties and menu.
    fn describe(&self) -> String;
    fn init(&mut self);
    fn start(&mut self);
    fn update(&mut self, dt: f32);
    /// A message arrived; its name comes from the host (`message_name`).
    fn message(&mut self, value: f32);
}

/// One object's value of a native script `T`, kept as a module keeps each slot's.
struct Native<T> {
    script: Option<T>,
    /// Whether it has read its properties yet.
    read: bool,
}

impl<T: Script> Native<T> {
    /// Runs `f` on the script, after it read its properties if they may have changed since it last
    /// did: the host says so when asked about the object (slot 0 of its own), and a script that set
    /// one itself reads them again before it next runs.
    fn run(&mut self, f: impl FnOnce(&mut Option<T>)) {
        let Some(script) = self.script.as_mut() else { return };
        if crate::abi::slot_select(0) == 2 || !std::mem::replace(&mut self.read, true) {
            script.read_properties();
        }
        f(&mut self.script);
        if crate::batch::take_own_set() {
            self.read = false;
        }
    }
}

impl<T: Script> NativeObject for Native<T> {
    fn describe(&self) -> String {
        crate::describe(T::PROPERTIES, T::MENU, T::INSPECTOR, T::EVENTS)
    }

    fn init(&mut self) {
        self.run(|script| script.iter_mut().for_each(Script::init));
    }

    fn start(&mut self) {
        self.run(|script| script.iter_mut().for_each(Script::start));
    }

    fn update(&mut self, dt: f32) {
        self.run(|script| T::update_many(&mut crate::Many::one(script), dt));
    }

    fn message(&mut self, value: f32) {
        self.run(|script| {
            if let Some(script) = script {
                script.message(&crate::message_name(), value);
            }
        });
    }
}

/// A script built into the game: makes one value of it per object.
#[derive(Clone, Copy)]
pub struct NativeScript {
    new: fn() -> Box<dyn NativeObject>,
}

impl NativeScript {
    /// The script type `T`, as [`crate::script!`] registers it.
    pub fn of<T: Script>() -> Self {
        Self { new: || Box::new(Native { script: Some(T::default()), read: false }) }
    }

    /// A fresh value of the script for one object.
    pub fn create(&self) -> Box<dyn NativeObject> {
        (self.new)()
    }
}

impl std::fmt::Debug for NativeScript {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeScript")
    }
}
