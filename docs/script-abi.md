# Script ABI, version 1

A game script is a WebAssembly module (`wasm32`, no WASI). An object carries a list of scripts (the
`scripts` component); the inspector shows each as its own component, named after its module file
(`player_controller.wasm` -> "Player Controller"). Every script on every object gets its own instance,
so module-level state belongs to that one script slot. Any language that compiles to WebAssembly can implement this ABI. The Rust SDK is
`crates/script-sdk`, and visual scripts (`.bscript`, see `docs/graphs.md`) compile to the same ABI when they are loaded.

## Exports (the script provides)

| Name | Signature | Required | Called |
|---|---|---|---|
| `memory` | linear memory | yes | the host reads and writes strings and transforms here |
| `bayleaf_abi` | `() -> i32` | yes | at load; must return `1` |
| `bayleaf_init` | `() -> ()` | no | once, when the object is ready: scripts loaded together (a scene when play starts, a placed prefab) all run `bayleaf_init` before any runs `bayleaf_start` |
| `bayleaf_start` | `() -> ()` | no | once, after every script loaded with it ran `bayleaf_init`: when play mode starts, or after a hot reload |
| `bayleaf_update` | `(dt: f32) -> ()` | no | every simulation step; `dt` is in seconds |
| `bayleaf_message` | `(value: f32) -> ()` | no | for each message sent to this object or to every object, after the step; `message_name` or `message_is` tell which |
| `bayleaf_describe` | `() -> i64` | no | when the editor or play mode loads the module; returns `(ptr << 32) \| len` of UTF-8 JSON `{"properties": [["speed", 90.0], ["label", "Hi", "text"], ...], "menu": "Light/Flicker"}`: the properties the editor shows, each a name (1 to 64 bytes, no `:`), a default and a type (`number` when left out; `whole`, `bool`, `vector`, `color`, `text` or `object`; the default is a number, a whole number, `true`/`false`, three numbers (a colour from 0 to 1), text, or `""` for an object). A type `list:<type>`, `map:<type>` (text keys) or `map:<key type>:<type>` makes the property a list or map of that type, its default an array or an object of such values (at most 1,024; a map's keys always written as text: `3`, `true`, `1,2,3`, an object's name); it fills the list or map of that name that the `list_*` and `map_*` functions keep, with what the object sets in the scene (saved as `{"list": [...]}` or `{"map": {...}}`) or else the default. `"variables"` lists, the same way, the lists and maps a visual script's variables start with. And optionally where Add Component lists the script (1 to 4 names of up to 64 characters separated by `/`; without it, under Scripts), at most 64 KiB |

### How properties show in the Inspector

The description may also say how the Inspector lays the properties out, as `"inspector": [items]`.
The editor draws it with its own widgets; no script code runs in the editor. Each item is an
object with exactly one of these keys:

| Item | Options |
|---|---|
| `{"property": "speed"}` | `label`, `hint` (said on hover and to screen readers), `range: [low, high]` (numbers), `slider: true` (needs a range), `readOnly: true`, `choices: ["Walk", "Run"]` (a whole number by place, 0 first, or a text) |
| `{"group": "Movement", "items": [...]}` | `closed: true` starts it folded; groups nest 4 deep |
| `{"button": "Jump now", "message": "jump"}` | `hint`; while the game plays it sends the object's scripts the message (value 0), as `play.message` does |
| `{"note": "Keep it low"}` | `warning: true` marks it as a warning |

Any item may take `showIf: ["mode", 1]`: shown only while that property has that value. A property is
placed at most once; those the layout does not place follow it, one row each. Each name is a declared
property, labels and names have 1 to 64 characters, hints and notes up to 512, a layout up to 256
items. A layout that breaks these rules stops the module loading, with the reason. In Rust,
`Script::INSPECTOR` lists `Layout` items (`Layout::property("speed").slider(0.0, 20.0)`,
`Layout::foldout`, `Layout::button`, `Layout::warning`, `.when("mode", Is::Whole(1))`); visual
scripts set each property's options in their panel. Properties with list values are not part of
this: there is no list property type yet.

## Imports (module `bayleaf`, the host provides)

| Name | Signature | Meaning |
|---|---|---|
| `log` | `(ptr: i32, len: i32)` | writes UTF-8 text to the editor console |
| `transform_get` | `(out: i32)` | writes 9 little-endian `f32`: position xyz, rotation xyz (degrees), scale xyz |
| `transform_set` | `(ptr: i32)` | reads 9 `f32` in the same layout; non-finite values are an error |
| `property` | `(key_ptr: i32, key_len: i32) -> f32` | the property's value: the one set in the scene (when it fits the declared type), else the declared default, else `NaN`; a whole number reads as a number and true/false as 1 or 0 |
| `property_whole` | `(key_ptr: i32, key_len: i32) -> i32` | a whole-number or true/false property (1 or 0), or a number rounded; `0` when there is none |
| `property_vector` | `(key_ptr: i32, key_len: i32, out: i32) -> i32` | writes a vector or colour property as 3 `f32` at `out`; `1` when there is one, else `0` (and zeros) |
| `property_text` | `(key_ptr: i32, key_len: i32, out: i32, cap: i32) -> i32` | copies at most `cap` bytes of a text or object property (the object's name) to `out` and returns its whole length; `cap` 0 only asks for the length |
| `property_set` | `(key_ptr: i32, key_len: i32, x: f32, y: f32, z: f32) -> i32` | sets this script's property on this object for the rest of play: a number takes `x`, a whole number `x` rounded, true/false `x` non-zero, a vector or colour all three (a colour kept within 0 to 1); `1` when set, `0` when there is no such property of those types. Non-finite numbers are an error |
| `property_set_text` | `(key_ptr: i32, key_len: i32, text_ptr: i32, text_len: i32) -> i32` | sets a text property, or an object property to an object's name (empty for none), for the rest of play; `1` when set, `0` when there is no such property or the text is longer than 1024 bytes |
| `property_object` | `(key_ptr: i32, key_len: i32) -> i32` | the handle of the object an object property names (the first with that name), or `0` for none |
| `time` | `() -> f64` | seconds since play mode started |
| `key_down` | `(name_ptr: i32, name_len: i32) -> i32` | `1` while the key is held (names like `W`, `Space`, `ArrowUp`), else `0` |
| `pointer` | `(out: i32) -> i32` | writes 2 `f32`: the pointer over the game view, from -1 to 1 (x right, y up); returns `1` when it is over the view, else `0` (and writes zeros) |
| `mouse_down` | `(button: i32) -> i32` | `1` while the button is held: `0` left, `1` right, `2` middle |
| `self_handle` | `() -> i32` | the handle of this script's object |
| `find` | `(name_ptr: i32, name_len: i32) -> i32` | the handle of the first object with that name, in hierarchy order, or `0` |
| `name_of` | `(handle: i32, out: i32, cap: i32) -> i32` | writes up to `cap` bytes of the object's UTF-8 name (as the step began) and returns its full length; `0` for no object |
| `object_ref` | `(handle: i32) -> i32` | a number for the object that stays the same from step to step (handles do not): `1` and up, `0` for no object. One script keeps at most 4096 objects; more is an error |
| `object_of` | `(ref: i32) -> i32` | the object's handle in this step, or `0` when the number is not one `object_ref` gave or the object is gone |
| `find_with` | `(kind_ptr: i32, kind_len: i32, index: i32) -> i32` | the handle of the `index`-th object (from 0, in hierarchy order, as the step began) that has a component (by its name or label: `pointLight`, `Point Light`), a tag, or runs a script (by its file name without the extension, or the name the Inspector shows); case, spaces, `_` and `-` are ignored. `0` when there is none |
| `count_with` | `(kind_ptr: i32, kind_len: i32) -> i32` | how many objects `find_with` can find for that kind |
| `component_get` | `(handle: i32, c_ptr: i32, c_len: i32, f_ptr: i32, f_len: i32, out: i32) -> i32` | writes 3 `f32`: a field of a component (by its name or label, as for `find_with`; the field by its name in scene files, such as `intensity`), as the step began: a number in the first, true/false as `1` or `0`, a vector or colour in all three. Returns `1` when the object has that component and field, else `0` (and writes zeros). An empty field asks whether the object has the component. The components are those visual scripts list (`visual::COMPONENTS`), or a script on the object by its name (as for `find_with`), whose properties are numbers |
| `component_set` | `(handle: i32, c_ptr: i32, c_len: i32, f_ptr: i32, f_len: i32, ptr: i32) -> i32` | reads 3 `f32` and asks to set the field after the step, the way `component_get` reads it (a colour keeps its alpha). `1` when asked; `0` when the object has no such field, the component is Rigid Body, Collider or Character (physics reads them when play starts), or this call already asked 64 times. A script's property is set on every script of that name on the object that declares it, when the step ends. Non-finite numbers are an error |
| `transform_get_of` | `(handle: i32, out: i32)` | like `transform_get`, for any object |
| `transform_set_of` | `(handle: i32, ptr: i32)` | like `transform_set`, for any object; applied when the call ends |
| `position_get_of` | `(handle: i32, out: i32)` | writes 3 little-endian `f64`: the object's local position in double precision, as this call last set it, else as the step began |
| `position_set_of` | `(handle: i32, ptr: i32)` | reads 3 `f64` and sets the position, keeping rotation and scale; applied when the call ends; non-finite values are an error |
| `spawn` | `(path_ptr: i32, path_len: i32, x: f32, y: f32, z: f32) -> i32` | asks for a copy of a `.bprefab` file (relative to the project folder) at a world position, after the step; `1` when asked, `0` when this call already asked 64 times |
| `particles_play` | `(handle: i32) -> i32` | starts the object's Particle Emitter after the step (a new cycle, with its burst); `1` when asked, `0` when this call already asked 64 times; Rust: `Object::play_particles` |
| `particles_stop` | `(handle: i32) -> i32` | stops it starting particles (those alive live out their lives); as `particles_play`; Rust: `Object::stop_particles` |
| `particles_burst` | `(handle: i32, count: i32) -> i32` | starts `count` particles at once (as many as it has room for); as `particles_play`; Rust: `Object::burst_particles` |
| `ui_set_text` | `(handle: i32, name_ptr: i32, name_len: i32, text_ptr: i32, text_len: i32) -> i32` | sets the text of the element named so (up to 64 bytes) in the object's Interface after the step, up to 4096 bytes; `1` when asked, `0` when this call already asked 64 times; a name no element has changes nothing. Rust: `Object::set_interface_text` |
| `ui_set_value` | `(handle: i32, name_ptr: i32, name_len: i32, value: f64) -> i32` | sets an element's value after the step: a toggle's state (0 or 1), a slider's number, a progress bar's fill (0 to 1); a value that is not finite is a fault; as `ui_set_text`; Rust: `Object::set_interface_value` |
| `ui_value` | `(handle: i32, name_ptr: i32, name_len: i32) -> f64` | the value of the element named so in the object's Interface as it shows (a toggle's 0 or 1, a slider's number, a progress bar's fill, a bound game value, a list's scroll); NaN when it has no such element. Rust: `Object::interface_value` |
| `ui_text` | `(handle: i32, name_ptr: i32, name_len: i32, out: i32, cap: i32) -> i32` | writes up to `cap` bytes of the element's text as it shows (what the player typed into a text field, too) and returns its full length; `-1` when there is no such element. Rust: `Object::interface_text` |
| `ui_add` | `(handle: i32, into_ptr: i32, into_len: i32, what_ptr: i32, what_len: i32, name_ptr: i32, name_len: i32) -> i32` | after the step, adds an element named so (1 to 48 bytes, no `:`) into the element named by `into` (empty for the root): `what` is a kind (`button`, `text`, `progress`...) or a layout file (`.bui`, up to 512 bytes) used as a part, whose elements are then named `<name>/<element>`; one that cannot be made is logged. `1` when asked, `0` when this call already asked 64 times. Rust: `Object::add_interface_element` |
| `ui_remove` | `(handle: i32, name_ptr: i32, name_len: i32) -> i32` | after the step, removes the element named so, with what it holds; as `ui_add`; Rust: `Object::remove_interface_element` |
| `ui_show` | `(handle: i32, name_ptr: i32, name_len: i32, visible: i32) -> i32` | shows (`1`) or hides (`0`) an element after the step, or the whole interface when the name is empty; as `ui_set_text`; Rust: `Object::show_interface` |
| `sound_play` | `(handle: i32) -> i32` | plays the object's Audio Source after the step; `1` when asked, `0` when this call already asked 64 times |
| `sound_stop` | `(handle: i32) -> i32` | stops the sounds the object plays, after the step; `1` or `0` as above |
| `sound_play_at` | `(path_ptr: i32, path_len: i32, x: f32, y: f32, z: f32, volume: f32) -> i32` | plays a sound file (relative to the project folder) once from a world position after the step, at a volume from 0 to 4; `1` or `0` as above |
| `scene_load` | `(path_ptr: i32, path_len: i32, additive: i32) -> i32` | asks to load a `.bscene` file (relative to the project folder) after the step: in place of the playing scene (every script starts again with the game's state kept; physics starts over), or, when `additive` is not `0`, its objects join the playing scene and their scripts init and start. `1` when asked, `0` when this call already asked 64 times |
| `scene_unload` | `(path_ptr: i32, path_len: i32) -> i32` | asks to remove, after the step, the objects of the scenes added from that file while playing; `1` or `0` as above |
| `destroy` | `(handle: i32) -> i32` | asks to delete the object and its children after the step; `1` or `0` as for `spawn` |
| `send` | `(handle: i32, name_ptr: i32, name_len: i32, value: f32) -> i32` | sends a message (a name of 1 to 64 bytes and a finite number) to the scripts of the object, or of every object when `handle` is `-1`; `1` when sent, `0` when this call already sent 64 |
| `message_is` | `(name_ptr: i32, name_len: i32) -> i32` | during `bayleaf_message`, `1` when the message has this name, else `0` |
| `message_name` | `(out: i32, cap: i32) -> i32` | during `bayleaf_message`, writes up to `cap` bytes of the message's name and returns its full length; `0` outside it |
| `event_send` | `(handle, name_ptr, name_len, values_ptr, count: i32) -> i32` | triggers an event: a message named `name` with values, to the object with `handle` (`-1` every object), after this step; `values_ptr` lists `count` values of 16 bytes each, a kind then three words: `0` number (`f32`), `1` whole number (`i32`), `2` true/false (`i32`), `3` vector (three `f32`), `4` text (pointer and length), `5` object (handle, `0` none), `6` a list and `7` a map this script keeps (its name's pointer and length; the host copies it as it is then). At most 16 values and 4096 bytes of text each. `1` when sent, `0` when this call sent too many |
| `event_number`, `event_whole`, `event_flag`, `event_object` | `(index: i32) -> f32 / i32` | during `bayleaf_message`, value `index` of the event being delivered: a number (a whole number converts), a whole number, `1` for true, an object's handle (`0` once it is gone); zero for a value of another kind or none |
| `event_vector` | `(index, part: i32) -> f32` | part `0`, `1` or `2` of vector value `index` |
| `event_text` | `(index, out, cap: i32) -> i32` | writes up to `cap` bytes of text value `index` and returns its full length |
| `script_component` | `(handle, name_ptr, name_len: i32) -> i32` | the place plus one of the first script named `name` (as the Inspector shows it) on the object with `handle`: with the handle, a *component*; `0` for none |
| `self_component` | `() -> i32` | this script's place plus one among its object's scripts |
| `event_send_to` | `(handle, script, name_ptr, name_len, values_ptr, count: i32) -> i32` | as `event_send`, to that one script of the object (`script`: its place plus one); nothing when the component is none or gone |
| `event_subscribe`, `event_unsubscribe` | `(handle, script, name_ptr, name_len: i32) -> i32` | from now on, this script hears event `name` whenever it happens on the component, until it unsubscribes; it arrives in `bayleaf_message` as `<the component's script>/<name>`, with the event's values |
| `event_source`, `event_source_component` | `() -> i32` | while a subscriber hears an event, the handle of the object it happened on and its script's place plus one; `0` otherwise |
| `event_keep` | `(index, map, name_ptr, name_len: i32) -> i32` | while an event is delivered, makes the list (`map` `0`) or map (`1`) it brings as value `index` one this script keeps under `name`, in place of one of that name (empty when the value is not one); visual scripts keep each one under a name no variable has, as their On Event begins |
| `list_put` | `(name_ptr, name_len, op, index, value_ptr: i32) -> i32` | the list `name` this script keeps (made the first time it is named: empty, or as the list property or variable of that name starts; at most 64 lists and maps per script) gets the value at `value_ptr` (16 bytes, as `event_send` lists values): `op` `0` at the end, `1` inserted at `index`, `2` in place of the item at `index`. `1` when it changed, `0` for an index past the end. A list of 65,536 items, or a name that is a map's, stops the script |
| `list_remove` | `(name_ptr, name_len, index: i32) -> i32` | takes the item at `index` out; `1` when there was one |
| `list_clear`, `list_len` | `(name_ptr, name_len: i32)`, `-> i32` | empties the list; how many items it holds |
| `list_get` | `(name_ptr, name_len, index, out: i32) -> i32` | writes the item at `index` to `out` as 16 bytes (a text gives its length, its bytes come from `list_text`); `1` when there is one, `0` past the end |
| `list_text` | `(name_ptr, name_len, index, out, cap: i32) -> i32` | the bytes of the text item at `index`, up to `cap` of them at `out`; its length, `0` for none |
| `list_find` | `(name_ptr, name_len, value_ptr: i32) -> i32` | the index of the first item equal to the value at `value_ptr`; `-1` for none |
| `map_put` | `(name_ptr, name_len, key_ptr, value_ptr: i32) -> i32` | the map `name` keeps the value at `value_ptr` under the key at `key_ptr`, both 16 bytes as `event_send` lists values (a key of any type but a list or map: text keys are ordered by their bytes, numbers by size, objects by id), in place of what was there; as for lists, at most 65,536 keys, and a name that is a list's stops the script. An object key that is none puts nothing (`0`) |
| `map_remove` | `(name_ptr, name_len, key_ptr: i32) -> i32` | takes the key out; `1` when it was there |
| `map_clear`, `map_count` | `(name_ptr, name_len: i32)`, `-> i32` | empties the map; how many keys it holds |
| `map_get`, `map_text` | `(name_ptr, name_len, key_ptr, out: i32) -> i32`, `(…, out, cap: i32) -> i32` | as `list_get` and `list_text`, for the value under the key |
| `map_next`, `map_next_text` | `(name_ptr, name_len, after_ptr, out: i32) -> i32`, `(name_ptr, name_len, after_ptr, out, cap: i32) -> i32` | the map's first key in key order (`after_ptr` below 0), or the first key after the one at `after_ptr`: written at `out` as a 16-byte value (`1`; `0` when there is none), a text key giving its length; `map_next_text` gives a text key's bytes, up to `cap` at `out`, and its length. Going on from a key, not by place, keeps a walk through the map right when keys come and go on the way |
| `math_pow` | `(base: f32, exponent: f32) -> f32` | `base` to the power of `exponent`, for visual scripts' Power (WebAssembly has no instruction for it); Rust scripts use `f32::powf` |
| `pointer_over_interface` | `() -> i32` | `1` while the pointer is over an interface element that takes it (its mouse buttons then read `0` here); Rust: `pointer_over_interface()` |
| `apply_impulse` | `(handle: i32, x: f32, y: f32, z: f32) -> i32` | pushes the object's dynamic physics body by an impulse in world space, after the step; `1` when asked, `0` when this call already asked 64 times |
| `set_velocity` | `(handle: i32, x: f32, y: f32, z: f32) -> i32` | sets the body's velocity (world units per second) after the step; `1` or `0` as above |
| `action` | `(name_ptr: i32, name_len: i32) -> f32` | an input action's value by its kind: a trigger `1` in the step it goes down, held `1` while down, value how far it is pressed, 0 to 1 (the strongest of its key, mouse, gamepad button and stick bindings; sticks past a dead zone of 0.15). Name it `Jump` for the active map or `Player/Jump`; an action of a map that is not active, or an unknown one, reads `0`. The script's object's own Input Map (an Input Map component) answers before the project's |
| `input_map_set` | `(handle: i32, name_ptr: i32, name_len: i32) -> i32` | makes a map active: the project's Input Map (`handle` 0) or an object's own; `1` when it has that map, else `0`. Only the active map's actions read |
| `character_walk` | `(handle: i32, x: f32, y: f32, z: f32) -> i32` | sets the object's Character walking velocity (world units per second) after the step; it keeps walking until changed. `1` when asked, `0` when this call already asked 64 times |
| `character_jump` | `(handle: i32, speed: f32) -> i32` | makes the Character jump with an upward speed after the step, if it stands on something |
| `character_grounded` | `(handle: i32) -> i32` | `1` while the Character stands on something (as the step began), else `0` |
| `velocity_of` | `(handle: i32, out: i32) -> i32` | writes 3 `f32`: the body's velocity as the step began; returns `1` when the object has a body, else `0` (and writes zeros) |
| `raycast` | `(ox, oy, oz, dx, dy, dz, max: f32, out: i32) -> i32` | the handle of the first object whose mesh shape or sprite a ray from the origin along the direction hits within `max` world units (this script's object is left out). Objects with a Collider or a Character are hit by that exact shape (triggers are left out), others by their mesh shape or sprite box, or `0`; writes 4 `f32`: distance and hit point xyz. Objects are as the step began; a mesh with a model counts as its shape's unit box |
| `pad_connected` | `() -> i32` | `1` while a gamepad is connected (the first one), else `0` |
| `pad_axis` | `(index: i32) -> f32` | `0` left stick x, `1` left stick y, `2` right stick x, `3` right stick y (-1 to 1, y up), `4` left trigger, `5` right trigger (0 to 1); `0` without a gamepad or for another index |
| `pad_button` | `(index: i32) -> i32` | `1` while the button is held: 0 South, 1 East, 2 West, 3 North, 4 LeftBumper, 5 RightBumper, 6 LeftTrigger, 7 RightTrigger, 8 Select, 9 Start, 10 LeftStick, 11 RightStick, 12 Up, 13 Down, 14 Left, 15 Right |
| `state_get` | `(scope: i32, handle: i32, name_ptr: i32, name_len: i32, out: i32) -> i32` | reads a value of the game's state (`scope` 0), a player's (`scope` 1: the player object's handle, `0` for the first player, `-1` for none) or a saved one (`scope` 2, `handle` 0): writes one `f64` and returns `1` when it is set, else `0` (and writes 0). The game's and players' state start empty when play starts; saved values come from the project's `saves/progress.json` |
| `state_set` | `(scope: i32, handle: i32, name_ptr: i32, name_len: i32, value: f64) -> i32` | sets a value at once, seen by every script; `1` when set, `0` for an empty name, a number that is not finite, a state already holding 1024 values, or no such player (handle `-1`, or an object without Player Control); a name over 64 bytes is a fault. Saved values are written to the project when play stops or the game ends |
| `game_end` | `(won: i32) -> i32` | ends the game, won (`1`) or lost (`0`); `1` when this ended it, `0` when it had already ended. Every script then gets the message `gameWon` or `gameLost` |

### Messages

A script declares its events in `bayleaf_describe` as `"events": [{"name": "Damaged", "params":
["amount:number", "by:object"]}]`, so the editor offers them to other scripts and the interface; an
event arrives in `bayleaf_message` under its name like any message (value `0`), its values read with
`event_*`. In the Rust SDK `trigger_event` sends one and `event::number(0)` and the others read it. Every
script, Rust or visual, is a *component* of its object: in the Rust SDK `Object::component(name)`
and `Component::this()` give one, `trigger_event`, `subscribe` and `unsubscribe` act on it,
`event::source()` says where a subscribed event happened, and `Script::EVENTS` declares a Rust
script's events as visual scripts declare theirs.

In a build with physics, contacts arrive as messages to both objects' scripts after the physics
step: `collision` and `collisionEnd` when colliders start and stop touching, `triggerEnter` and
`triggerExit` when one of them is a trigger. The value is the other object's handle. In the Rust SDK the names are
`contact::TRIGGER_ENTER` and the others, and `Object::from_message(value)` gives the other object. When the game ends (`game_end`, or
the time limit of the scene's Game Rules), every object's scripts get `gameWon` or `gameLost` once,
with the value 0.

An Interface sends its object's scripts `pressed:<name>` when the player presses one of its buttons
(value 0), and `changed:<name>` when a toggle, slider or text field changes (the new value; a text
field's length in characters). They arrive at once, between steps (`docs/interface.md`).

Messages are delivered after every script's update in the step, in the order sent. Each script of
the target object that exports `bayleaf_message` gets the call, with the same limits as any other
call. A message sent while handling one is delivered after the next step, so scripts that answer
each other cannot run forever. Messages sent from `bayleaf_start` arrive after the first step. The
Rust SDK calls `Script::message(name, value)`; `Object::send` and `send_to_all` send.
Its `game_value`, `set_game_value`, `player_value`, `set_player_value`, `saved_value`,
`set_saved_value` and `end_game` use the game state. `action`, `action_held`, `set_input_map` and
`Object::set_input_map` use the input maps, and `Script::MENU` names where Add Component lists the
script.

### Other objects

A handle names an object for one step: it is the object's place in the scene as the step began,
plus one (`0` means none). During a step every script sees the objects as they were when it began,
except for the transforms its own call has set. Using a handle that names no object stops the
script with a reason. Spawns and destroys happen after the step, in the order asked; a spawned
copy's scripts start at once and run from the next step. Play mode puts everything back on stop.
These imports came after the first ABI release; a module that uses them needs an engine that has
them, and fails to load on an older one with a message naming the missing import.

The transform is the object's local transform, relative to its parent. Positions are kept in double
precision; `transform_get` gives them rounded to `f32`, and a `transform_set` that leaves the
position as it was read keeps the exact one. Far from the world's centre (beyond about 10 km), move
objects with `position_get_of` and `position_set_of`, as visual scripts' Move By does.

## Modules that run many objects

A module may run every object of its file in one instance, so a step is one call for all of them
instead of one per object. It says so by exporting `bayleaf_update_all`; then the host gives each
object a *slot* (0, 1, 2, … in the order the objects load) and calls these instead of
`bayleaf_init`, `bayleaf_start`, `bayleaf_update` and `bayleaf_message`:

| Export | Signature | Called |
|---|---|---|
| `bayleaf_attach` | `(slot: i32) -> ()` | an object comes to the module, before its init |
| `bayleaf_detach` | `(slot: i32) -> ()` | the object goes (it is destroyed, or its script loads again); the slot is not used again |
| `bayleaf_init_slot`, `bayleaf_start_slot` | `(slot: i32) -> ()` | as `bayleaf_init` and `bayleaf_start`, for that object |
| `bayleaf_update_all` | `(dt: f32) -> ()` | every step, once for all the module's objects |
| `bayleaf_message_slot` | `(slot: i32, value: f32) -> ()` | as `bayleaf_message`, for that object |

| Import | Signature | What it does |
|---|---|---|
| `slot_select` | `(slot: i32) -> i32` | makes that object the one every other import is about ("this object": its transform, properties, logs, lists and maps, subscriptions); `0` when it does not run (its object stopped or went), `1` when it does, `2` when its properties changed since last asked (another script or the Inspector set one) |
| `frame_get` | `(out, cap: i32) -> i32` | how many slots the module has; when `cap` slots fit, writes for each, at `out + slot × 64`, its transform (nine `f32`, as `transform_get`), at byte 36 a word of flags (`1` it runs, `2` its properties changed since last asked) and at byte 40 its position in double precision (three `f64`) |
| `frame_set` | `(ptr, count: i32) -> ()` | takes back `count` such entries: each whose flags have `4` (changed) set becomes its object's transform, as `transform_set` would make it. A module sets `8` on the entry of the object it is updating, so an error is reported on that object even when it made no call for it |

Within `bayleaf_update_all` the module selects each object before a call about it (`slot_select`; the SDK does it only before a call that needs it); the host
gives its calls the fuel of one call per object. An error in it stops the module, and every object
it runs, with one fault, on the object selected when it stopped. A transform the module writes
through the frame reaches the host when `frame_set` gives it back, so within the step other imports
still see the object where the step began. Rust scripts built with `bayleaf-script-sdk` run this
way: `script!` keeps each object's value by slot, reads the frame before the update of all and
writes it back after it, and keeps each object's number properties and the play time as last read,
so `Transform::get`, `set`, `property` and `time` need no host call during it. Visual scripts run this way too, unless one keeps more than 4 KB per object (each text variable keeps 1 KB): their variables, timers and the places of their loops are kept per object in the module's memory and loaded before that object's code runs. During the update of all their own transform and position, number, whole-number and vector properties, and the time come from the frame and from what each object last read, as for Rust scripts (a property is read again when the host says it changed, or when the script set one). Other objects' transforms they read are kept for the update of all too, four at a time, until the module sets one; the object an object variable names and the object's own handle are kept for that object's update.

## Limits

- Each call may run about 10 million WebAssembly instructions ("fuel"). A script that runs out,
  traps, panics, or reads or writes outside its memory is stopped for the rest of play mode. The
  error is reported in the console and in `play.status`. Other scripts keep running.
- Linear memory may grow to 64 MiB.
- Scripts reach the world only through these imports: no files, network or clock beyond `time`.
- The editor interprets scripts (wasmi). A game exported in the mode "Compiled to machine code"
  compiles each module with wasmtime when the game starts, then runs it with the same imports, fuel
  and memory limits and faults; the script tests run against both.

## Scripts in Rust

In the editor, Project panel, + > New Script makes `<name>/` in the open folder: a crate with
`Cargo.toml` and `src/lib.rs` from a template that uses `bayleaf-script-sdk`. The editor builds it
at once, in the background, into `<name>.wasm` beside the folder (build files go to `.build/` in the
project); right-click the folder, Build Script, to build again. Building needs Rust and the target:
`rustup target add wasm32-unknown-unknown`. A script that is already running reloads when its
`.wasm` changes.

## Native scripts

A game exported in native mode builds its Rust scripts into its own player: the same crates, built
for the computer instead of WebAssembly, run as machine code with **no sandbox**. Such a script can
do anything a program can, and a fault in one (a panic, in a player built to unwind) stops it with
its reason; built to abort, it stops the game. Fuel and the memory limit do not apply to it.

Off WebAssembly, `script!` gives `bayleaf_native()` instead of the exports above, and the game's
player registers it under the module it stands for (`scripts/mover.wasm`). Each object gets its own
value of the script, as each gets its own instance in WebAssembly; a `static` in the crate is shared
by every object, unlike in WebAssembly. Host calls are the same imports: the SDK copies their texts
and numbers through a small exchange module that imports every host function, so each check and
fault applies as for a WebAssembly script. Modules without a crate beside them, and visual
scripts, stay compiled WebAssembly.

The export writes a Cargo workspace in `.build/native/<platform>/` in the project (the player,
copies of the script crates, the engine's patches and `Cargo.lock`) and builds it with the engine's
source (`BAYLEAF_SOURCE_DIR`, else the folder the editor was built from) and cargo. The first build
takes several minutes; later ones reuse its output. On Windows keep the project in a short path:
build files nest deep, and paths over 260 characters fail to link.

## Hot reload

While play mode runs, the host checks each module file for changes about twice a second. A changed
module is loaded again and its instances restart: module state is lost and `bayleaf_start` runs
again. The scene keeps the play-mode changes made so far.
