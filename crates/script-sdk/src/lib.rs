//! Rust SDK for Bayleaf game scripts.
//!
//! A script is a type that implements [`Script`], registered with [`script!`], built for
//! `wasm32-unknown-unknown` as a `cdylib`. A game exported in native mode builds the same crate into
//! the game instead, with no sandbox: see [`native`].
//!
//! ```ignore
//! use bayleaf_script_sdk::{Property, Script, Transform, property, script};
//!
//! #[derive(Default)]
//! struct Spinner;
//!
//! impl Script for Spinner {
//!     // The Inspector shows Speed on each object; property("speed") reads it.
//!     const PROPERTIES: &'static [Property] = &[Property::Number("speed", 90.0)];
//!
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
mod inspector;
pub use inspector::{Is, Layout};
pub mod native;

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

/// A number property ([`Property::Number`]) as the scene sets it, else its default; a whole number
/// or true/false reads as a number too. `None` when the script declares no such property.
pub fn property(name: &str) -> Option<f32> {
    let value = abi::property(name);
    (!value.is_nan()).then_some(value)
}

/// A whole-number property ([`Property::Whole`]); 0 when there is none.
pub fn property_whole(name: &str) -> i32 {
    abi::property_whole(name)
}

/// A true/false property ([`Property::Flag`]); false when there is none.
pub fn property_flag(name: &str) -> bool {
    abi::property_whole(name) != 0
}

/// A vector or colour property ([`Property::Vector`], [`Property::Color`]), if there is one.
pub fn property_vector(name: &str) -> Option<[f32; 3]> {
    abi::property_vector(name)
}

/// A text property ([`Property::Text`]); empty when there is none.
pub fn property_text(name: &str) -> String {
    abi::property_text(name)
}

/// The object an object property ([`Property::Object`]) names, if it is chosen and in the scene.
pub fn property_object(name: &str) -> Option<Object> {
    Some(abi::property_object(name)).filter(|h| *h != 0).map(Object)
}

/// Sets a number property of this script on this object, for the rest of play. A whole-number
/// property takes it rounded, a true/false one non-zero as true. False when there is no such
/// property of a number type.
pub fn set_property(name: &str, value: f32) -> bool {
    abi::property_set(name, [value, 0.0, 0.0])
}

/// Sets a whole-number property; false when there is none.
pub fn set_property_whole(name: &str, value: i32) -> bool {
    abi::property_set(name, [value as f32, 0.0, 0.0])
}

/// Sets a true/false property; false when there is none.
pub fn set_property_flag(name: &str, value: bool) -> bool {
    abi::property_set(name, [f32::from(u8::from(value)), 0.0, 0.0])
}

/// Sets a vector or colour property; false when there is none.
pub fn set_property_vector(name: &str, value: [f32; 3]) -> bool {
    abi::property_set(name, value)
}

/// Sets a text property; false when there is none.
pub fn set_property_text(name: &str, value: &str) -> bool {
    abi::property_set_text(name, value)
}

/// Sets an object property to an object (by its name), or to none; false when there is none.
pub fn set_property_object(name: &str, object: Option<Object>) -> bool {
    abi::property_set_text(name, &object.map(Object::name).unwrap_or_default())
}

/// A property a script declares in [`Script::PROPERTIES`]: its name and default. The Inspector
/// edits it per object with a field of its type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Property {
    Number(&'static str, f32),
    Whole(&'static str, i32),
    Flag(&'static str, bool),
    Vector(&'static str, [f32; 3]),
    /// Red, green and blue from 0 to 1.
    Color(&'static str, [f32; 3]),
    Text(&'static str, &'static str),
    /// An object of the scene, chosen per object; none by default.
    Object(&'static str),
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

/// True while the pointer is over an interface element that takes it: its clicks are then the
/// interface's, and `mouse_down` reads nothing.
pub fn pointer_over_interface() -> bool {
    abi::pointer_over_interface()
}

/// True while the mouse button is held.
pub fn mouse_down(button: Button) -> bool {
    abi::mouse_down(button as i32)
}

/// Plays a sound file (relative to the project folder) once from a world position, after this step.
pub fn play_sound_at(path: &str, position: [f32; 3], volume: f32) -> bool {
    abi::sound_play_at(path, position, volume)
}

/// After this step, loads a scene file (a `.bscene` relative to the project folder): in place of the
/// playing scene (every script starts again; the game's values stay), or with `additive` its objects
/// join the playing scene. False when this call already asked too often.
pub fn load_scene(path: &str, additive: bool) -> bool {
    abi::scene_load(path, additive)
}

/// After this step, removes the objects of the scenes added from `path` while playing.
pub fn unload_scene(path: &str) -> bool {
    abi::scene_unload(path)
}

/// An object kept from step to step, from [`Object::keep`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kept(i32);

impl Kept {
    /// The object in this step, or `None` once it is gone.
    pub fn object(self) -> Option<Object> {
        Some(abi::object_of(self.0)).filter(|h| *h != 0).map(Object)
    }
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

    /// The `index`-th object (from 0, in hierarchy order) that has a component (`Point Light`,
    /// `pointLight`), a tag, or runs a script (by its name); case, spaces, `_` and `-` do not matter.
    pub fn find_with(kind: &str, index: usize) -> Option<Self> {
        let index = i32::try_from(index).ok()?;
        Some(abi::find_with(kind, index)).filter(|h| *h != 0).map(Self)
    }

    /// Every object that has a component, a tag or a script (see `find_with`), in hierarchy order.
    pub fn all_with(kind: &str) -> impl Iterator<Item = Self> + '_ {
        (0..).map_while(move |index| Self::find_with(kind, index))
    }

    /// How many objects have a component, a tag or a script (see `find_with`).
    pub fn count_with(kind: &str) -> usize {
        usize::try_from(abi::count_with(kind)).unwrap_or(0)
    }

    /// Starts the object's Particle Emitter (a new cycle, with its burst). False when it has none.
    pub fn play_particles(self) -> bool {
        abi::particles_play(self.0)
    }

    /// Stops the object's Particle Emitter starting particles; those alive live out their lives.
    pub fn stop_particles(self) -> bool {
        abi::particles_stop(self.0)
    }

    /// Starts `count` particles at once from the object's Particle Emitter.
    pub fn burst_particles(self, count: u32) -> bool {
        abi::particles_burst(self.0, count)
    }

    /// Sets the text of the element named `element` in the object's Interface, after this step.
    /// Buttons send the object's scripts `pressed:<name>`, and toggles, sliders and text fields
    /// `changed:<name>` with their new value.
    pub fn set_interface_text(self, element: &str, text: &str) -> bool {
        abi::ui_set_text(self.0, element, text)
    }

    /// Sets an element's value: a toggle's state (0 or 1), a slider's number, a progress bar's fill.
    pub fn set_interface_value(self, element: &str, value: f64) -> bool {
        abi::ui_set_value(self.0, element, value)
    }

    /// Adds an element named `name` to the object's Interface, after this step, inside the element
    /// named `parent` ("" for the root): `what` is a kind (`button`, `text`, `progress`...) or a
    /// layout file (`ui/slot.bui`) used as a part, whose elements are then named `<name>/<element>`.
    pub fn add_interface_element(self, parent: &str, what: &str, name: &str) -> bool {
        abi::ui_add(self.0, parent, what, name)
    }

    /// Removes the element named `name` from the object's Interface, with what it holds.
    pub fn remove_interface_element(self, name: &str) -> bool {
        abi::ui_remove(self.0, name)
    }

    /// Shows or hides an element of the object's Interface, or the whole interface when `element`
    /// is empty.
    pub fn show_interface(self, element: &str, visible: bool) -> bool {
        abi::ui_show(self.0, element, visible)
    }

    /// Plays its Audio Source after this step; false when this call asked too often.
    pub fn play_sound(self) -> bool {
        abi::sound_play(self.0)
    }

    /// Stops the sounds it plays after this step.
    pub fn stop_sound(self) -> bool {
        abi::sound_stop(self.0)
    }

    /// Its name, as the step began.
    pub fn name(self) -> String {
        abi::name_of(self.0)
    }

    /// A way to find this object again in later steps (handles last one step): see [`Kept`].
    pub fn keep(self) -> Option<Kept> {
        Some(abi::object_ref(self.0)).filter(|r| *r != 0).map(Kept)
    }

    /// Whether it has a component whose fields scripts read (`Point Light`, `pointLight`).
    pub fn has(self, component: &str) -> bool {
        abi::component_get(self.0, component, "").is_some()
    }

    /// A field of one of its components (`"Point Light"`, `"intensity"`), or a property of a script on
    /// it by the script's name (`"Bob"`, `"speed"`), as the step began: a
    /// number in the first part, true/false as 1 or 0, a vector or colour in all three.
    pub fn field(self, component: &str, field: &str) -> Option<[f32; 3]> {
        abi::component_get(self.0, component, field)
    }

    /// Sets a field of one of its components after this step, the way `field` reads it. False when
    /// it has no such field, the component is read only in play (Rigid Body, Collider, Character),
    /// or this call asked too often.
    pub fn set_field(self, component: &str, field: &str, value: [f32; 3]) -> bool {
        abi::component_set(self.0, component, field, value)
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

    /// Its position in double precision, relative to its parent: exact far from the world's centre,
    /// where the single-precision [`Transform`] rounds.
    pub fn position(self) -> [f64; 3] {
        abi::position_get_of(self.0)
    }

    /// Sets its position in double precision when this call ends.
    pub fn set_position(self, position: [f64; 3]) {
        abi::position_set_of(self.0, &position);
    }

    /// Sets its character's walking velocity (world units per second) after this step; it keeps
    /// walking until changed.
    pub fn walk(self, velocity: [f32; 3]) -> bool {
        abi::character_walk(self.0, velocity)
    }

    /// Makes its character jump with an upward speed after this step, if it stands on something.
    pub fn jump(self, speed: f32) -> bool {
        abi::character_jump(self.0, speed)
    }

    /// True while its character stands on something (as the step began).
    pub fn on_ground(self) -> bool {
        abi::character_grounded(self.0)
    }

    /// Pushes its physics body by an impulse (world space, mass times change of velocity) after
    /// this step. False when this call already asked too much.
    pub fn push(self, impulse: [f32; 3]) -> bool {
        abi::apply_impulse(self.0, impulse)
    }

    /// Sets its physics body's velocity (world units per second) after this step.
    pub fn set_velocity(self, velocity: [f32; 3]) -> bool {
        abi::set_velocity(self.0, velocity)
    }

    /// Its physics body's velocity as the step began, if it has a body.
    pub fn velocity(self) -> Option<[f32; 3]> {
        abi::velocity_of(self.0)
    }

    /// Sends a message and a number to its scripts ([`Script::message`]), after this step.
    /// False when this call already sent too many.
    pub fn send(self, message: &str, value: f32) -> bool {
        abi::send(self.0, message, value)
    }

    /// Makes another map of this object's own Input Map active. False when it has none of its own
    /// or no such map.
    pub fn set_input_map(self, map: &str) -> bool {
        abi::input_map_set(self.0, map)
    }
}

/// Sends a message and a number to the scripts of every object, after this step. False when
/// this call already sent too many.
pub fn send_to_all(message: &str, value: f32) -> bool {
    abi::send(-1, message, value)
}

/// The name of the message being delivered (used by [`script!`]).
#[doc(hidden)]
pub fn message_name() -> String {
    abi::message_name()
}

/// Places a copy of a prefab file (a path relative to the project folder) at a world position
/// after this step. False when this call already asked too much.
pub fn spawn(prefab: &str, position: [f32; 3]) -> bool {
    abi::spawn(prefab, position)
}

/// What a ray hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub object: Object,
    /// Distance from the ray's origin, in world units.
    pub distance: f32,
    /// Where the ray hit, in world space.
    pub point: [f32; 3],
}

/// The first object's shape or sprite a ray from `origin` along `direction` hits within `max`
/// (this script's own object is left out), as the objects were when the step began.
pub fn raycast(origin: [f32; 3], direction: [f32; 3], max: f32) -> Option<Hit> {
    let (handle, v) = abi::raycast(origin, direction, max);
    (handle != 0).then_some(Hit { object: Object(handle), distance: v[0], point: [v[1], v[2], v[3]] })
}

/// A gamepad's sticks and triggers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadAxis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    LeftTrigger,
    RightTrigger,
}

/// A gamepad's buttons. South is the bottom face button, North the top one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadButton {
    South,
    East,
    West,
    North,
    LeftBumper,
    RightBumper,
    LeftTrigger,
    RightTrigger,
    Select,
    Start,
    LeftStick,
    RightStick,
    Up,
    Down,
    Left,
    Right,
}

/// An input action's value by its kind: a trigger 1 in the step it goes down, held 1 while down,
/// value how far it is pressed (0 to 1). Name it `Jump` for the active map or `Player/Jump`; an
/// action of a map that is not active, or an unknown one, reads 0. This object's own Input Map
/// (an Input Map component) answers before the project's.
pub fn action(name: &str) -> f32 {
    abi::action(name)
}

/// True while an input action reads more than half: held down, or a trigger in its step.
pub fn action_held(name: &str) -> bool {
    abi::action(name) > 0.5
}

/// Makes another map of the project's Input Map active (`Menu`, `Player`). False when it has no
/// such map. Only the active map's actions read.
pub fn set_input_map(map: &str) -> bool {
    abi::input_map_set(0, map)
}

/// A value of the game's state, shared by every script while the game runs; `None` until a
/// script or an agent sets it (the state starts empty).
pub fn game_value(name: &str) -> Option<f64> {
    abi::state_get(0, 0, name)
}

/// Sets a value of the game's state. False for an empty or too long name (over 64 bytes), a
/// number that is not finite, or a state that already holds too many values.
pub fn set_game_value(name: &str, value: f64) -> bool {
    abi::state_set(0, 0, name, value)
}

/// A value of a player's state (`None` for the first player, an object with Player Control);
/// `None` until it is set, or when there is no such player.
pub fn player_value(player: Option<Object>, name: &str) -> Option<f64> {
    abi::state_get(1, player.map_or(0, |p| p.0), name)
}

/// Sets a value of a player's state (`None` for the first player). False as for
/// [`set_game_value`], or when there is no such player.
pub fn set_player_value(player: Option<Object>, name: &str, value: f64) -> bool {
    abi::state_set(1, player.map_or(0, |p| p.0), name, value)
}

/// A saved value: kept from one play to the next in the project (such as a best score); `None`
/// until it is set.
pub fn saved_value(name: &str) -> Option<f64> {
    abi::state_get(2, 0, name)
}

/// Sets a saved value; it is written to the project when play stops or the game ends. False as
/// for [`set_game_value`].
pub fn set_saved_value(name: &str, value: f64) -> bool {
    abi::state_set(2, 0, name, value)
}

/// Ends the game as won or lost. Every script then gets the message `gameWon` or `gameLost`.
/// Only the first ending counts: false when the game has already ended.
pub fn end_game(won: bool) -> bool {
    abi::game_end(won)
}

/// True while a gamepad is connected.
pub fn pad_connected() -> bool {
    abi::pad_connected()
}

/// A stick from -1 to 1 (x right, y up) or a trigger from 0 to 1; 0 without a gamepad.
pub fn pad_axis(axis: PadAxis) -> f32 {
    abi::pad_axis(axis as i32)
}

/// True while the button is held on the gamepad.
pub fn pad_button(button: PadButton) -> bool {
    abi::pad_button(button as i32)
}

/// Behaviour of one scripted object. One value lives per object for the whole play session.
pub trait Script: Default + Send + 'static {
    /// Properties the editor shows on this script, with their defaults. `property(name)` returns
    /// the value set in the scene, or this default.
    const PROPERTIES: &'static [Property] = &[];
    /// Where Add Component lists the script: folders, then its name (`"Light/Flicker"`). Empty
    /// for the Scripts folder, under the module's name.
    const MENU: &'static str = "";
    /// How the properties show in the Inspector: groups, labels, sliders, buttons and more (see
    /// [`Layout`]). Empty for one row each, in the order of [`Script::PROPERTIES`].
    const INSPECTOR: &'static [Layout] = &[];

    /// Runs once when the script's object is ready: every script loaded with it (all of a scene
    /// when play starts) runs `init` before any runs `start`.
    fn init(&mut self) {}
    /// Runs once after every script loaded with it ran `init`: when play mode starts, or after a
    /// hot reload.
    fn start(&mut self) {}
    /// Runs every simulation step; `dt` is the step length in seconds.
    fn update(&mut self, dt: f32) {
        let _ = dt;
    }
    /// Runs when a script sends a message to this object or to every object.
    fn message(&mut self, name: &str, value: f32) {
        let _ = (name, value);
    }
}

/// Exports `ty` (a [`Script`]) to the host. Use once per script crate. Built for WebAssembly it
/// exports the ABI's functions; built natively (a game exported in native mode) it gives
/// `bayleaf_native()`, which the game's player registers.
#[macro_export]
macro_rules! script {
    ($ty:ty) => {
        /// This script for a game that builds it in natively.
        #[cfg(not(target_arch = "wasm32"))]
        pub fn bayleaf_native() -> $crate::native::NativeScript {
            $crate::native::NativeScript::of::<$ty>()
        }

        #[cfg(target_arch = "wasm32")]
        static BAYLEAF_SCRIPT: ::std::sync::Mutex<Option<$ty>> = ::std::sync::Mutex::new(None);

        #[cfg(target_arch = "wasm32")]
        #[unsafe(no_mangle)]
        pub extern "C" fn bayleaf_abi() -> i32 {
            $crate::ABI_VERSION
        }

        #[cfg(target_arch = "wasm32")]
        #[unsafe(no_mangle)]
        pub extern "C" fn bayleaf_describe() -> u64 {
            static DESCRIPTION: ::std::sync::OnceLock<String> = ::std::sync::OnceLock::new();
            let text = DESCRIPTION
                .get_or_init(|| $crate::describe(<$ty as $crate::Script>::PROPERTIES, <$ty as $crate::Script>::MENU, <$ty as $crate::Script>::INSPECTOR));
            $crate::pack(text)
        }

        #[cfg(target_arch = "wasm32")]
        #[unsafe(no_mangle)]
        pub extern "C" fn bayleaf_init() {
            let mut state = BAYLEAF_SCRIPT.lock().unwrap_or_else(|e| e.into_inner());
            let script = state.get_or_insert_with(<$ty as ::core::default::Default>::default);
            $crate::Script::init(script);
        }

        #[cfg(target_arch = "wasm32")]
        #[unsafe(no_mangle)]
        pub extern "C" fn bayleaf_start() {
            let mut state = BAYLEAF_SCRIPT.lock().unwrap_or_else(|e| e.into_inner());
            let script = state.get_or_insert_with(<$ty as ::core::default::Default>::default);
            $crate::Script::start(script);
        }

        #[cfg(target_arch = "wasm32")]
        #[unsafe(no_mangle)]
        pub extern "C" fn bayleaf_update(dt: f32) {
            let mut state = BAYLEAF_SCRIPT.lock().unwrap_or_else(|e| e.into_inner());
            let script = state.get_or_insert_with(<$ty as ::core::default::Default>::default);
            $crate::Script::update(script, dt);
        }

        #[cfg(target_arch = "wasm32")]
        #[unsafe(no_mangle)]
        pub extern "C" fn bayleaf_message(value: f32) {
            let mut state = BAYLEAF_SCRIPT.lock().unwrap_or_else(|e| e.into_inner());
            let script = state.get_or_insert_with(<$ty as ::core::default::Default>::default);
            $crate::Script::message(script, &$crate::message_name(), value);
        }
    };
}

/// The `bayleaf_describe` text for `properties` (used by [`script!`]).
#[doc(hidden)]
pub fn describe(properties: &[Property], menu: &str, inspector: &[Layout]) -> String {
    let number = |v: f32| if v.is_finite() { v } else { 0.0 };
    let three = |v: [f32; 3]| format!("[{:?}, {:?}, {:?}]", number(v[0]), number(v[1]), number(v[2]));
    let entries: Vec<String> = properties
        .iter()
        .map(|property| {
            let (name, default, ty) = match property {
                Property::Number(name, v) => (name, format!("{:?}", number(*v)), "number"),
                Property::Whole(name, v) => (name, v.to_string(), "whole"),
                Property::Flag(name, v) => (name, v.to_string(), "bool"),
                Property::Vector(name, v) => (name, three(*v), "vector"),
                Property::Color(name, v) => (name, three(v.map(|c| c.clamp(0.0, 1.0))), "color"),
                Property::Text(name, v) => (name, format!("\"{}\"", escape(v)), "text"),
                Property::Object(name) => (name, "\"\"".to_owned(), "object"),
            };
            format!("[\"{}\", {default}, \"{ty}\"]", escape(name))
        })
        .collect();
    let menu = if menu.is_empty() { String::new() } else { format!(", \"menu\": \"{}\"", escape(menu)) };
    let inspector =
        if inspector.is_empty() { String::new() } else { format!(", \"inspector\": [{}]", inspector.iter().map(Layout::json).collect::<Vec<_>>().join(", ")) };
    format!("{{\"properties\": [{}]{menu}{inspector}}}", entries.join(", "))
}

/// Text inside a JSON string: quotes and backslashes escaped, control characters left out.
fn escape(text: &str) -> String {
    text.chars()
        .flat_map(|c| match c {
            '"' | '\\' => vec!['\\', c],
            c if c.is_control() => vec![],
            c => vec![c],
        })
        .collect()
}

/// Packs a string's address and length as `(ptr << 32) | len` (used by [`script!`]).
#[doc(hidden)]
pub fn pack(text: &str) -> u64 {
    ((text.as_ptr() as usize as u64) << 32) | (text.len() as u64 & 0xFFFF_FFFF)
}

#[cfg(test)]
mod tests {
    use super::{Property, Transform, property};

    #[test]
    fn descriptions_are_json_the_host_can_read() {
        let text = super::describe(&[Property::Number("speed", 90.0), Property::Text("odd \"name\"", "a\\b"), Property::Object("target")], "", &[]);
        assert_eq!(text, r#"{"properties": [["speed", 90.0, "number"], ["odd \"name\"", "a\\b", "text"], ["target", "", "object"]]}"#);
        let text = super::describe(&[], "Light/\"Flicker\"", &[]);
        assert_eq!(text, r#"{"properties": [], "menu": "Light/\"Flicker\""}"#);
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
