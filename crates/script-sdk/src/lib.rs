//! Rust SDK for Bayleaf game scripts.
//!
//! A script is a type that implements [`Script`], registered with [`script!`], built for
//! `wasm32-unknown-unknown` as a `cdylib`:
//!
//! ```ignore
//! use bayleaf_script_sdk::{Script, Transform, property, script};
//!
//! #[derive(Default)]
//! struct Spinner;
//!
//! impl Script for Spinner {
//!     fn update(&mut self, dt: f32) {
//!         let speed = property("speed").unwrap_or(90.0);
//!         let mut t = Transform::get();
//!         t.rotation[1] += speed * dt;
//!         t.set();
//!     }
//! }
//!
//! script!(Spinner);
//! ```

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod abi;

/// ABI version this SDK implements; the host checks it at load.
pub const ABI_VERSION: i32 = 1;

/// The object's local transform: position, rotation (Euler degrees, X then Y then Z) and scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}

impl Transform {
    /// Reads the object's transform.
    pub fn get() -> Self {
        let v = abi::transform_get();
        Self { position: [v[0], v[1], v[2]], rotation: [v[3], v[4], v[5]], scale: [v[6], v[7], v[8]] }
    }

    /// Writes the object's transform. Non-finite numbers stop the script with an error.
    pub fn set(&self) {
        let [p, r, s] = [self.position, self.rotation, self.scale];
        abi::transform_set(&[p[0], p[1], p[2], r[0], r[1], r[2], s[0], s[1], s[2]]);
    }
}

/// Writes a line to the editor console.
pub fn log(text: &str) {
    abi::log(text);
}

/// A number from the script component's `properties`, if set.
pub fn property(name: &str) -> Option<f32> {
    let value = abi::property(name);
    (!value.is_nan()).then_some(value)
}

/// Seconds since play mode started.
pub fn time() -> f64 {
    abi::time()
}

/// True while the key is held, e.g. `"W"`, `"Space"`, `"ArrowUp"`.
pub fn key_down(name: &str) -> bool {
    abi::key_down(name)
}

/// Where the pointer is over the game view, from (-1, -1) at the bottom left to (1, 1) at the top
/// right; `None` when it is outside the view.
pub fn pointer() -> Option<[f32; 2]> {
    abi::pointer()
}

/// A mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left = 0,
    Right = 1,
    Middle = 2,
}

/// True while the mouse button is held.
pub fn mouse_down(button: Button) -> bool {
    abi::mouse_down(button as i32)
}

/// Another object of the scene, as it was when the step began. Handles last for one step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Object(i32);

impl Object {
    /// The object this script is on.
    pub fn this() -> Option<Self> {
        Some(abi::self_handle()).filter(|h| *h != 0).map(Self)
    }

    /// The first object with this name, in hierarchy order.
    pub fn find(name: &str) -> Option<Self> {
        Some(abi::find(name)).filter(|h| *h != 0).map(Self)
    }

    /// Its local transform.
    pub fn transform(self) -> Transform {
        let v = abi::transform_get_of(self.0);
        Transform { position: [v[0], v[1], v[2]], rotation: [v[3], v[4], v[5]], scale: [v[6], v[7], v[8]] }
    }

    /// Sets its local transform when this call ends.
    pub fn set_transform(self, t: &Transform) {
        let [p, r, s] = [t.position, t.rotation, t.scale];
        abi::transform_set_of(self.0, &[p[0], p[1], p[2], r[0], r[1], r[2], s[0], s[1], s[2]]);
    }

    /// Deletes it and its children after this step. False when this call already asked too much.
    pub fn destroy(self) -> bool {
        abi::destroy(self.0)
    }
}

/// Places a copy of a prefab file (a path relative to the project folder) at a world position
/// after this step. False when this call already asked too much.
pub fn spawn(prefab: &str, position: [f32; 3]) -> bool {
    abi::spawn(prefab, position)
}

/// Behaviour of one scripted object. One value lives per object for the whole play session.
pub trait Script: Default + Send + 'static {
    /// Properties the editor shows on this script, with their defaults. `property(name)` returns
    /// the value set in the scene, or this default.
    const PROPERTIES: &'static [(&'static str, f32)] = &[];

    /// Runs once when play mode starts (or after a hot reload).
    fn start(&mut self) {}
    /// Runs every simulation step; `dt` is the step length in seconds.
    fn update(&mut self, dt: f32) {
        let _ = dt;
    }
}

/// Exports `ty` (a [`Script`]) to the host. Use once per script crate.
#[macro_export]
macro_rules! script {
    ($ty:ty) => {
        static BAYLEAF_SCRIPT: ::std::sync::Mutex<Option<$ty>> = ::std::sync::Mutex::new(None);

        #[unsafe(no_mangle)]
        pub extern "C" fn bayleaf_abi() -> i32 {
            $crate::ABI_VERSION
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn bayleaf_describe() -> u64 {
            static DESCRIPTION: ::std::sync::OnceLock<String> = ::std::sync::OnceLock::new();
            let text = DESCRIPTION.get_or_init(|| $crate::describe(<$ty as $crate::Script>::PROPERTIES));
            $crate::pack(text)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn bayleaf_start() {
            let mut state = BAYLEAF_SCRIPT.lock().unwrap_or_else(|e| e.into_inner());
            let script = state.get_or_insert_with(<$ty as ::core::default::Default>::default);
            $crate::Script::start(script);
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn bayleaf_update(dt: f32) {
            let mut state = BAYLEAF_SCRIPT.lock().unwrap_or_else(|e| e.into_inner());
            let script = state.get_or_insert_with(<$ty as ::core::default::Default>::default);
            $crate::Script::update(script, dt);
        }
    };
}

/// The `bayleaf_describe` text for `properties` (used by [`script!`]).
#[doc(hidden)]
pub fn describe(properties: &[(&str, f32)]) -> String {
    let entries: Vec<String> = properties
        .iter()
        .map(|(name, default)| {
            let escaped: String = name
                .chars()
                .flat_map(|c| match c {
                    '"' | '\\' => vec!['\\', c],
                    c if c.is_control() => vec![],
                    c => vec![c],
                })
                .collect();
            let value = if default.is_finite() { *default } else { 0.0 };
            format!("[\"{escaped}\", {value:?}]")
        })
        .collect();
    format!("{{\"properties\": [{}]}}", entries.join(", "))
}

/// Packs a string's address and length as `(ptr << 32) | len` (used by [`script!`]).
#[doc(hidden)]
pub fn pack(text: &str) -> u64 {
    ((text.as_ptr() as usize as u64) << 32) | (text.len() as u64 & 0xFFFF_FFFF)
}

#[cfg(test)]
mod tests {
    use super::{Transform, property};

    #[test]
    fn descriptions_are_json_the_host_can_read() {
        let text = super::describe(&[("speed", 90.0), ("odd \"name\"", 1.5)]);
        assert_eq!(text, r#"{"properties": [["speed", 90.0], ["odd \"name\"", 1.5]]}"#);
    }

    #[test]
    fn missing_properties_are_none_off_the_host() {
        assert_eq!(property("speed"), None);
    }

    #[test]
    fn transform_round_trips_through_the_flat_layout() {
        let t = Transform::get();
        assert_eq!(t.scale, [1.0; 3]);
        t.set();
    }
}
