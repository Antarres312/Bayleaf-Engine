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

/// A data file of the project (`.bdata`), by its path from the project folder (`data/goblin.bdata`).
/// Its fields read as the file was saved when play started, each the shape's default where the
/// file sets none; play never writes them. A field of another type reads as `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Data {
    path: String,
}

/// The data file at `path`, from the project folder. Reading a file the project lacks gives `None`
/// for every field.
pub fn data(path: &str) -> Data {
    Data { path: path.to_owned() }
}

/// The data file a data file property ([`Property::Data`]) names, if one is chosen.
pub fn property_data(name: &str) -> Option<Data> {
    Some(property_text(name)).filter(|p| !p.is_empty()).map(|path| Data { path })
}

impl Data {
    /// Its path from the project folder.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// A number field; a whole number or true/false reads as a number too.
    pub fn number(&self, field: &str) -> Option<f32> {
        abi::data_get(&self.path, field).map(|v| v[0])
    }

    /// A whole-number field (a number rounded).
    pub fn whole(&self, field: &str) -> Option<i32> {
        self.number(field).map(|v| v.round() as i32)
    }

    /// A true/false field.
    pub fn flag(&self, field: &str) -> Option<bool> {
        self.number(field).map(|v| v != 0.0)
    }

    /// A vector or colour field.
    pub fn vector(&self, field: &str) -> Option<[f32; 3]> {
        abi::data_get(&self.path, field)
    }

    /// A text field, an object field's object name, or a file field's path (empty for none).
    pub fn text(&self, field: &str) -> Option<String> {
        abi::data_text_of(&self.path, field)
    }

    /// The data file a data file field names, if one is chosen.
    pub fn data(&self, field: &str) -> Option<Data> {
        self.text(field).filter(|p| !p.is_empty()).map(|path| Data { path })
    }

    /// How many items a list field has, or entries a map field.
    pub fn len(&self, field: &str) -> Option<usize> {
        abi::data_len(&self.path, field)
    }

    /// Item `index` (from 0) of a list field of numbers, vectors or colours (a number in the first place).
    pub fn number_at(&self, field: &str, index: usize) -> Option<[f32; 3]> {
        abi::data_item(&self.path, field, index)
    }

    /// Item `index` of a list field of texts, objects or files.
    pub fn text_at(&self, field: &str, index: usize) -> Option<String> {
        abi::data_item_text(&self.path, field, index)
    }

    /// The `index`-th key (from 0, in order) of a map field, as text: `3`, `true`, `1,2,3`, a name.
    pub fn key_at(&self, field: &str, index: usize) -> Option<String> {
        abi::data_key(&self.path, field, index)
    }

    /// The value under `key` (as [`Data::key_at`] writes keys) of a map field of numbers, vectors or colours.
    pub fn number_for(&self, field: &str, key: &str) -> Option<[f32; 3]> {
        abi::data_entry(&self.path, field, key)
    }

    /// The value under `key` of a map field of texts, objects or files.
    pub fn text_for(&self, field: &str, key: &str) -> Option<String> {
        abi::data_entry_text(&self.path, field, key)
    }
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
    /// A prefab file of the project, chosen per object; none by default.
    Prefab(&'static str),
    /// A sound file of the project, chosen per object; none by default.
    Sound(&'static str),
    /// A material of the project, chosen per object; none by default.
    Material(&'static str),
    /// A data file of the project ([`Data`]), chosen per object; none by default. Read it with
    /// [`property_data`].
    Data(&'static str),
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
    /// The other object of a physics contact, from the number its message brings: `name` is one
    /// of the [`contact`] messages and `value` the number that came with it ([`Script::message`]).
    pub fn from_message(value: f32) -> Option<Self> {
        let handle = value as i32;
        (handle != 0 && handle as f32 == value).then_some(Self(handle))
    }

    /// The script named `name` on this object (as the Inspector shows it, or its file's name), as a
    /// component; `None` when it has none.
    pub fn component(self, name: &str) -> Option<Component> {
        Some(Component { object: self, script: abi::script_component(self.0, name) }).filter(|c| c.script != 0)
    }

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

    /// Plays the clip `clip` of the object's model on its whole skeleton after this step, fading from
    /// what played over `fade` seconds (0 to 60), from its end round to its start when `looping`.
    /// A clip the model lacks is reported in the console. False when this call already asked too
    /// much.
    pub fn play_clip(self, clip: &str, fade: f32, looping: bool) -> bool {
        abi::animation_play(self.0, clip, fade, looping)
    }

    /// Fades the object's model to its rest pose over `fade` seconds, after this step.
    pub fn stop_clips(self, fade: f32) -> bool {
        abi::animation_stop(self.0, fade)
    }

    /// Sets a number parameter of the object's animation (what mixes and animation states read).
    pub fn set_animation_number(self, name: &str, value: f32) -> bool {
        abi::animation_number(self.0, name, value)
    }

    /// Sets a true/false parameter of the object's animation.
    pub fn set_animation_flag(self, name: &str, value: bool) -> bool {
        abi::animation_flag(self.0, name, value)
    }

    /// Sets a trigger of the object's animation, which the first transition it lets through uses up.
    pub fn trigger_animation(self, name: &str) -> bool {
        abi::animation_trigger(self.0, name)
    }

    /// The animation state the object is in, when it runs animation states (as of the last step).
    pub fn animation_state(self) -> Option<String> {
        abi::animation_state(self.0)
    }

    /// Where the joint `joint` of the object's model is in the world (position, rotation, scale),
    /// as the last step posed it; `None` when the object is not animated or has no such joint.
    pub fn joint(self, joint: &str) -> Option<Transform> {
        let v = abi::joint_transform(self.0, joint)?;
        Some(Transform { position: [v[0], v[1], v[2]], rotation: [v[3], v[4], v[5]], scale: [v[6], v[7], v[8]] })
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

    /// The value of the element named `element` in the object's Interface as it shows now (a
    /// toggle's 0 or 1, a slider's number, a bound game value); `None` when it has no such element.
    pub fn interface_value(self, element: &str) -> Option<f64> {
        abi::ui_value(self.0, element)
    }

    /// The text of the element named `element` as it shows now (what the player typed into a text
    /// field, too); `None` when it has no such element.
    pub fn interface_text(self, element: &str) -> Option<String> {
        abi::ui_text(self.0, element)
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

    /// Plays its Keyframe Animation from the start after this step; false when this call asked
    /// too often.
    pub fn play_keyframes(self) -> bool {
        abi::timeline_play(self.0)
    }

    /// Stops its Keyframe Animation where it is, after this step.
    pub fn stop_keyframes(self) -> bool {
        abi::timeline_stop(self.0)
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

/// The messages a physics step sends both objects of a contact, after the step; the number is the
/// other object ([`Object::from_message`]).
pub mod contact {
    /// Another collider came into this object's trigger, or this object into another's.
    pub const TRIGGER_ENTER: &str = "triggerEnter";
    /// It left the trigger.
    pub const TRIGGER_EXIT: &str = "triggerExit";
    /// The colliders started touching.
    pub const COLLISION: &str = "collision";
    /// They stopped touching.
    pub const COLLISION_END: &str = "collisionEnd";
}

/// What a value of an event holds, as a script declares it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    Number,
    Whole,
    Flag,
    Vector,
    Color,
    Text,
    Object,
}

impl ValueKind {
    fn code(self) -> &'static str {
        match self {
            ValueKind::Number => "number",
            ValueKind::Whole => "whole",
            ValueKind::Flag => "bool",
            ValueKind::Vector => "vector",
            ValueKind::Color => "color",
            ValueKind::Text => "text",
            ValueKind::Object => "object",
        }
    }
}

/// An event a script declares ([`Script::EVENTS`]): its name and its values, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub name: &'static str,
    pub values: &'static [(&'static str, ValueKind)],
}

/// A script on an object, Rust or visual alike: what events are triggered on and subscribed to.
/// Like an [`Object`], it lasts for one step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Component {
    object: Object,
    /// The script's place among its object's scripts, plus one.
    script: i32,
}

impl Component {
    /// This script, as a component of its object.
    pub fn this() -> Option<Self> {
        Some(Self { object: Object::this()?, script: abi::self_component() }).filter(|c| c.script != 0)
    }

    /// The object it is on.
    pub fn object(self) -> Object {
        self.object
    }

    /// Triggers its script's event `name` with `values` on it alone, after this step; the scripts
    /// subscribed to it hear it too. False when this call already sent too many.
    pub fn trigger_event(self, name: &str, values: &[EventValue<'_>]) -> bool {
        abi::event_send_to(self.object.0, self.script, name, &event_slots(values))
    }

    /// From now on this script hears the event `name` whenever it happens on this component, in
    /// [`Script::message`] as `<its script>/<name>`, with [`event::source`] saying where.
    pub fn subscribe(self, name: &str) -> bool {
        abi::event_subscribe(self.object.0, self.script, name, true)
    }

    /// Stops this script hearing the event it subscribed to.
    pub fn unsubscribe(self, name: &str) -> bool {
        abi::event_subscribe(self.object.0, self.script, name, false)
    }
}

fn event_slots<'a>(values: &[EventValue<'a>]) -> Vec<abi::EventSlot<'a>> {
    values
        .iter()
        .map(|value| match *value {
            EventValue::Number(v) => abi::EventSlot::Number(v),
            EventValue::Whole(v) => abi::EventSlot::Whole(v),
            EventValue::Flag(v) => abi::EventSlot::Flag(v),
            EventValue::Vector(v) => abi::EventSlot::Vector(v),
            EventValue::Text(t) => abi::EventSlot::Text(t),
            EventValue::Object(o) => abi::EventSlot::Object(o.map_or(0, |o| o.0)),
        })
        .collect()
}

/// A value an event brings: what a visual script's Trigger Event sends and On Event gives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EventValue<'a> {
    Number(f32),
    Whole(i32),
    Flag(bool),
    Vector([f32; 3]),
    Text(&'a str),
    Object(Option<Object>),
}

/// Triggers the event `name` with `values` on the scripts of `to`, or of every object with
/// `None`, after this step. Visual scripts hear it with On Event; Rust scripts in
/// [`Script::message`] under the event's name, reading its values with [`event`]. False when this
/// call already sent too many.
pub fn trigger_event(to: Option<Object>, name: &str, values: &[EventValue<'_>]) -> bool {
    abi::event_send(to.map_or(-1, |o| o.0), name, &event_slots(values))
}

/// The values of the event being delivered, by position, read in [`Script::message`]. A value of
/// another kind, or past the last, reads as zero, false, empty or none.
pub mod event {
    use super::{Object, abi};

    fn at(index: usize) -> i32 {
        i32::try_from(index).unwrap_or(i32::MAX)
    }

    pub fn number(index: usize) -> f32 {
        abi::event_number(at(index))
    }

    pub fn whole(index: usize) -> i32 {
        abi::event_whole(at(index))
    }

    pub fn flag(index: usize) -> bool {
        abi::event_flag(at(index))
    }

    pub fn vector(index: usize) -> [f32; 3] {
        abi::event_vector(at(index))
    }

    pub fn text(index: usize) -> String {
        abi::event_text(at(index))
    }

    pub fn object(index: usize) -> Option<Object> {
        Some(abi::event_object(at(index))).filter(|h| *h != 0).map(Object)
    }

    /// The component a subscribed event happened on, while this script hears it; `None` otherwise.
    pub fn source() -> Option<super::Component> {
        let object = Object(abi::event_source());
        let script = abi::event_source_component();
        (object.0 != 0 && script != 0).then_some(super::Component { object, script })
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
    /// Events this script declares: what other scripts (Rust or visual) and the interface can
    /// trigger on its component, and subscribe to. They arrive in [`Script::message`] under their name.
    const EVENTS: &'static [Event] = &[];

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

        // One instance runs every object of this script: each one's value by its slot, which the
        // host hands out (`docs/script-abi.md`, "Modules that run many objects").
        #[cfg(target_arch = "wasm32")]
        static BAYLEAF_SLOTS: ::std::sync::Mutex<Vec<Option<$ty>>> = ::std::sync::Mutex::new(Vec::new());

        #[cfg(target_arch = "wasm32")]
        #[unsafe(no_mangle)]
        pub extern "C" fn bayleaf_abi() -> i32 {
            $crate::ABI_VERSION
        }

        #[cfg(target_arch = "wasm32")]
        #[unsafe(no_mangle)]
        pub extern "C" fn bayleaf_describe() -> u64 {
            static DESCRIPTION: ::std::sync::OnceLock<String> = ::std::sync::OnceLock::new();
            let text = DESCRIPTION.get_or_init(|| {
                $crate::describe(
                    <$ty as $crate::Script>::PROPERTIES,
                    <$ty as $crate::Script>::MENU,
                    <$ty as $crate::Script>::INSPECTOR,
                    <$ty as $crate::Script>::EVENTS,
                )
            });
            $crate::pack(text)
        }

        #[cfg(target_arch = "wasm32")]
        #[unsafe(no_mangle)]
        pub extern "C" fn bayleaf_attach(slot: i32) {
            let mut slots = BAYLEAF_SLOTS.lock().unwrap_or_else(|e| e.into_inner());
            let Ok(slot) = usize::try_from(slot) else { return };
            if slots.len() <= slot {
                slots.resize_with(slot + 1, || None);
            }
            slots[slot] = Some(<$ty as ::core::default::Default>::default());
            $crate::batch::attached(slot);
        }

        #[cfg(target_arch = "wasm32")]
        #[unsafe(no_mangle)]
        pub extern "C" fn bayleaf_detach(slot: i32) {
            let mut slots = BAYLEAF_SLOTS.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(value) = usize::try_from(slot).ok().and_then(|slot| slots.get_mut(slot)) {
                *value = None;
            }
        }

        #[cfg(target_arch = "wasm32")]
        #[unsafe(no_mangle)]
        pub extern "C" fn bayleaf_init_slot(slot: i32) {
            let mut slots = BAYLEAF_SLOTS.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(Some(script)) = usize::try_from(slot).ok().and_then(|slot| slots.get_mut(slot))
                && $crate::batch::select(slot)
            {
                $crate::Script::init(script);
            }
        }

        #[cfg(target_arch = "wasm32")]
        #[unsafe(no_mangle)]
        pub extern "C" fn bayleaf_start_slot(slot: i32) {
            let mut slots = BAYLEAF_SLOTS.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(Some(script)) = usize::try_from(slot).ok().and_then(|slot| slots.get_mut(slot))
                && $crate::batch::select(slot)
            {
                $crate::Script::start(script);
            }
        }

        #[cfg(target_arch = "wasm32")]
        #[unsafe(no_mangle)]
        pub extern "C" fn bayleaf_update_all(dt: f32) {
            let mut slots = BAYLEAF_SLOTS.lock().unwrap_or_else(|e| e.into_inner());
            $crate::batch::begin();
            for (slot, script) in slots.iter_mut().enumerate() {
                if let Some(script) = script
                    && $crate::batch::select(slot as i32)
                {
                    $crate::Script::update(script, dt);
                }
            }
            $crate::batch::end();
        }

        #[cfg(target_arch = "wasm32")]
        #[unsafe(no_mangle)]
        pub extern "C" fn bayleaf_message_slot(slot: i32, value: f32) {
            let mut slots = BAYLEAF_SLOTS.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(Some(script)) = usize::try_from(slot).ok().and_then(|slot| slots.get_mut(slot))
                && $crate::batch::select(slot)
            {
                $crate::Script::message(script, &$crate::message_name(), value);
            }
        }
    };
}

/// What a module that runs many objects keeps about the update of all of them (used by
/// [`script!`]): which object the calls are about, every object's transform for the step (read from
/// the host in one call and written back in one), and each object's properties as last read, so its
/// update needs no host call for them. Outside such an update (and off WebAssembly) every call goes
/// to the host as usual.
#[doc(hidden)]
pub mod batch {
    /// Bytes of one object's entry in the frame, and its flags (see `docs/script-abi.md`).
    #[cfg(target_arch = "wasm32")]
    const ENTRY: usize = 64;
    #[cfg(target_arch = "wasm32")]
    const RUNNING: u32 = 1;
    #[cfg(target_arch = "wasm32")]
    const PROPERTIES: u32 = 2;
    #[cfg(target_arch = "wasm32")]
    const CHANGED: u32 = 4;
    /// The object being updated, so the host knows whose update an error was in.
    #[cfg(target_arch = "wasm32")]
    const CURRENT: u32 = 8;
    /// The flags take the low bits of their word; the object's handle is the rest.
    #[cfg(target_arch = "wasm32")]
    const HANDLE_SHIFT: u32 = 4;
    /// Other objects' transforms kept during an update of all.
    #[cfg(target_arch = "wasm32")]
    const OTHERS: usize = 4;

    #[cfg(target_arch = "wasm32")]
    struct Frame {
        data: Vec<u8>,
        /// How many objects the frame holds; 0 outside an update of all.
        count: usize,
        /// The object the calls are about, while the frame holds it.
        current: Option<usize>,
        /// The play time, once asked in this update of all.
        time: Option<f64>,
        /// Each object's number properties as last read, by slot.
        properties: Vec<Vec<(String, f32)>>,
        /// The object the host has selected, as far as this module knows: it is selected only before
        /// a call to the host needs it.
        host: Option<usize>,
        /// Other objects' transforms read in this update of all, by handle (0: an empty place), and
        /// the place the next one takes. Forgotten when this module sets another object's transform.
        others: [(i32, [f32; 9]); OTHERS],
        next_other: usize,
        /// The handles of the objects each slot keeps, by what it keeps them as, found while the
        /// world's stamp was `stamp`: handles name the same objects until it changes.
        kept: Vec<Vec<(i32, i32)>>,
        stamp: i64,
    }

    #[cfg(target_arch = "wasm32")]
    static FRAME: std::sync::Mutex<Frame> = std::sync::Mutex::new(Frame {
        data: Vec::new(),
        count: 0,
        current: None,
        time: None,
        properties: Vec::new(),
        host: None,
        others: [(0, [0.0; 9]); OTHERS],
        next_other: 0,
        kept: Vec::new(),
        stamp: 0,
    });

    #[cfg(target_arch = "wasm32")]
    fn frame() -> std::sync::MutexGuard<'static, Frame> {
        FRAME.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[cfg(target_arch = "wasm32")]
    fn flags(frame: &Frame, slot: usize) -> u32 {
        let at = slot * ENTRY + 36;
        frame.data.get(at..at + 4).map_or(0, |w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
    }

    #[cfg(target_arch = "wasm32")]
    fn set_flags(frame: &mut Frame, slot: usize, change: impl FnOnce(u32) -> u32) {
        let at = slot * ENTRY + 36;
        if let Some(word) = frame.data.get_mut(at..at + 4) {
            let flags = change(u32::from_le_bytes([word[0], word[1], word[2], word[3]]));
            word.copy_from_slice(&flags.to_le_bytes());
        }
    }

    /// Reads every object's transform before an update of all.
    pub fn begin() {
        #[cfg(target_arch = "wasm32")]
        {
            let mut frame = frame();
            loop {
                let room = frame.data.len() / ENTRY;
                let count = usize::try_from(crate::abi::frame_get(&mut frame.data, room)).unwrap_or(0);
                if count <= room {
                    frame.count = count;
                    break;
                }
                frame.data.resize(count * ENTRY, 0);
            }
            if frame.properties.len() < frame.count {
                let count = frame.count;
                frame.properties.resize_with(count, Vec::new);
            }
            frame.time = None;
            frame.current = None;
            frame.host = None;
            frame.others = [(0, [0.0; 9]); OTHERS];
            // Handles found under another structure of the world may name other objects now.
            let stamp = crate::abi::world_stamp();
            if stamp != frame.stamp {
                frame.stamp = stamp;
                frame.kept.iter_mut().for_each(Vec::clear);
            }
            if frame.kept.len() < frame.count {
                let count = frame.count;
                frame.kept.resize_with(count, Vec::new);
            }
        }
    }

    /// Writes back the transforms the update of all changed.
    pub fn end() {
        #[cfg(target_arch = "wasm32")]
        {
            let mut frame = frame();
            let count = frame.count;
            crate::abi::frame_set(&frame.data[..count * ENTRY], count);
            frame.count = 0;
            frame.current = None;
            frame.host = None;
        }
    }

    /// Makes `slot` the object the next calls are about; false when it does not run (its object
    /// stopped or went).
    #[cfg(target_arch = "wasm32")]
    pub fn select(slot: i32) -> bool {
        let Ok(at) = usize::try_from(slot) else { return false };
        {
            let mut frame = frame();
            if at < frame.count {
                let flags = flags(&frame, at);
                if flags & RUNNING == 0 {
                    return false;
                }
                if flags & PROPERTIES != 0 {
                    frame.properties[at].clear();
                }
                // The host selects it only when one of its calls needs it (`sync`); the frame says
                // which object it is, for an error.
                if let Some(before) = frame.current {
                    set_flags(&mut frame, before, |f| f & !CURRENT);
                }
                set_flags(&mut frame, at, |f| f | CURRENT);
                frame.current = Some(at);
                return true;
            }
            frame.current = None;
        }
        let selected = crate::abi::slot_select(slot);
        frame().host = (selected != 0).then_some(at);
        if selected == 2
            && let Some(kept) = frame().properties.get_mut(at)
        {
            kept.clear();
        }
        selected != 0
    }

    /// Before a call to the host: the object the calls are about is the one the host has selected.
    pub(crate) fn sync() {
        #[cfg(target_arch = "wasm32")]
        {
            let mut frame = frame();
            let Some(current) = frame.current else { return };
            if frame.host == Some(current) {
                return;
            }
            frame.host = Some(current);
            drop(frame);
            if crate::abi::slot_select(current as i32) == 2
                && let Some(kept) = self::frame().properties.get_mut(current)
            {
                kept.clear();
            }
        }
    }

    /// A new object came to the module, in `slot`: it starts with no properties read.
    pub fn attached(slot: usize) {
        #[cfg(target_arch = "wasm32")]
        {
            let mut frame = frame();
            if let Some(kept) = frame.properties.get_mut(slot) {
                kept.clear();
            }
            if let Some(kept) = frame.kept.get_mut(slot) {
                kept.clear();
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = slot;
    }

    /// The current object's handle from the frame, during an update of all.
    pub(crate) fn own_handle() -> Option<i32> {
        #[cfg(target_arch = "wasm32")]
        {
            let frame = frame();
            let handle = flags(&frame, frame.current?) >> HANDLE_SHIFT;
            (handle != 0).then_some(handle as i32)
        }
        #[cfg(not(target_arch = "wasm32"))]
        None
    }

    /// The handle of the object the current object keeps as `reference`: as found since the world's
    /// structure last changed during an update of all, else `read`.
    pub(crate) fn kept_object(reference: i32, read: impl FnOnce() -> i32) -> i32 {
        #[cfg(target_arch = "wasm32")]
        {
            let frame = frame();
            let Some(slot) = frame.current else {
                drop(frame);
                return read();
            };
            if let Some((_, handle)) = frame.kept.get(slot).and_then(|kept| kept.iter().find(|(r, _)| *r == reference)) {
                return *handle;
            }
            drop(frame);
            let handle = read();
            if let Some(kept) = self::frame().kept.get_mut(slot) {
                kept.push((reference, handle));
            }
            handle
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = reference;
            read()
        }
    }

    /// Another object's transform: as read earlier in this update of all, else `read`. The current
    /// object's own, asked by its handle, is the frame's.
    pub(crate) fn other_transform(handle: i32, read: impl FnOnce() -> [f32; 9]) -> [f32; 9] {
        #[cfg(target_arch = "wasm32")]
        {
            let frame = frame();
            let Some(slot) = frame.current.filter(|_| handle > 0) else {
                drop(frame);
                return read();
            };
            if flags(&frame, slot) >> HANDLE_SHIFT == handle as u32 {
                drop(frame);
                return transform().unwrap_or_else(read);
            }
            if let Some((_, values)) = frame.others.iter().find(|(h, _)| *h == handle) {
                return *values;
            }
            drop(frame);
            let values = read();
            let mut frame = self::frame();
            let at = frame.next_other;
            frame.others[at] = (handle, values);
            frame.next_other = (at + 1) % OTHERS;
            values
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = handle;
            read()
        }
    }

    /// This module set another object's transform: what it read of others is read again.
    pub(crate) fn forget_others() {
        #[cfg(target_arch = "wasm32")]
        {
            frame().others = [(0, [0.0; 9]); OTHERS];
        }
    }

    /// The current object's transform from the frame, during an update of all.
    pub(crate) fn transform() -> Option<[f32; 9]> {
        #[cfg(target_arch = "wasm32")]
        {
            let frame = frame();
            let at = frame.current? * ENTRY;
            let bytes = frame.data.get(at..at + 36)?;
            Some(std::array::from_fn(|i| f32::from_le_bytes([bytes[i * 4], bytes[i * 4 + 1], bytes[i * 4 + 2], bytes[i * 4 + 3]])))
        }
        #[cfg(not(target_arch = "wasm32"))]
        None
    }

    /// Sets the current object's transform in the frame, during an update of all; false outside one
    /// (the host gets it then).
    pub(crate) fn set_transform(values: &[f32; 9]) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            let mut frame = frame();
            let Some(slot) = frame.current else { return false };
            let at = slot * ENTRY;
            let Some(bytes) = frame.data.get_mut(at..at + 40) else { return false };
            for (i, v) in values.iter().enumerate() {
                bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
            let flags = u32::from_le_bytes([bytes[36], bytes[37], bytes[38], bytes[39]]) | CHANGED;
            bytes[36..40].copy_from_slice(&flags.to_le_bytes());
            true
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = values;
            false
        }
    }

    /// A number property of the current object: as last read during an update of all, else `read`.
    pub(crate) fn property(name: &str, read: impl FnOnce() -> f32) -> f32 {
        #[cfg(target_arch = "wasm32")]
        {
            let frame = frame();
            let Some(slot) = frame.current else {
                drop(frame);
                return read();
            };
            if let Some((_, value)) = frame.properties.get(slot).and_then(|kept| kept.iter().find(|(n, _)| n == name)) {
                return *value;
            }
            drop(frame);
            let value = read();
            if let Some(kept) = self::frame().properties.get_mut(slot) {
                kept.push((name.to_owned(), value));
            }
            value
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = name;
            read()
        }
    }

    /// The current object's script set a property itself: read them again.
    pub(crate) fn forget_properties() {
        #[cfg(target_arch = "wasm32")]
        {
            let mut frame = frame();
            if let Some(slot) = frame.current
                && let Some(kept) = frame.properties.get_mut(slot)
            {
                kept.clear();
            }
        }
    }

    /// The play time, read once per update of all.
    pub(crate) fn time(read: impl FnOnce() -> f64) -> f64 {
        #[cfg(target_arch = "wasm32")]
        {
            let frame = frame();
            if frame.count == 0 {
                drop(frame);
                return read();
            }
            if let Some(time) = frame.time {
                return time;
            }
            drop(frame);
            let time = read();
            self::frame().time = Some(time);
            time
        }
        #[cfg(not(target_arch = "wasm32"))]
        read()
    }
}

/// The `bayleaf_describe` text for `properties` (used by [`script!`]).
#[doc(hidden)]
pub fn describe(properties: &[Property], menu: &str, inspector: &[Layout], events: &[Event]) -> String {
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
                Property::Prefab(name) => (name, "\"\"".to_owned(), "prefab"),
                Property::Sound(name) => (name, "\"\"".to_owned(), "sound"),
                Property::Material(name) => (name, "\"\"".to_owned(), "material"),
                Property::Data(name) => (name, "\"\"".to_owned(), "data"),
            };
            format!("[\"{}\", {default}, \"{ty}\"]", escape(name))
        })
        .collect();
    let menu = if menu.is_empty() { String::new() } else { format!(", \"menu\": \"{}\"", escape(menu)) };
    let inspector =
        if inspector.is_empty() { String::new() } else { format!(", \"inspector\": [{}]", inspector.iter().map(Layout::json).collect::<Vec<_>>().join(", ")) };
    let events = if events.is_empty() {
        String::new()
    } else {
        let listed: Vec<String> = events
            .iter()
            .map(|e| {
                let values: Vec<String> = e.values.iter().map(|(name, kind)| format!("\"{}:{}\"", escape(name), kind.code())).collect();
                format!("{{\"name\": \"{}\", \"params\": [{}]}}", escape(e.name), values.join(", "))
            })
            .collect();
        format!(", \"events\": [{}]", listed.join(", "))
    };
    format!("{{\"properties\": [{}]{menu}{inspector}{events}}}", entries.join(", "))
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
        let text = super::describe(&[Property::Number("speed", 90.0), Property::Text("odd \"name\"", "a\\b"), Property::Object("target")], "", &[], &[]);
        assert_eq!(text, r#"{"properties": [["speed", 90.0, "number"], ["odd \"name\"", "a\\b", "text"], ["target", "", "object"]]}"#);
        let text = super::describe(&[], "Light/\"Flicker\"", &[], &[]);
        assert_eq!(text, r#"{"properties": [], "menu": "Light/\"Flicker\""}"#);
        let events = [super::Event { name: "Died", values: &[("by", super::ValueKind::Object), ("amount", super::ValueKind::Number)] }];
        let text = super::describe(&[], "", &[], &events);
        assert_eq!(text, r#"{"properties": [], "events": [{"name": "Died", "params": ["by:object", "amount:number"]}]}"#, "events as visual scripts list them");
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
