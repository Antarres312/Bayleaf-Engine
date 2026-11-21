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
| `bayleaf_start` | `() -> ()` | no | once, when play mode starts or after a hot reload |
| `bayleaf_update` | `(dt: f32) -> ()` | no | every simulation step; `dt` is in seconds |
| `bayleaf_message` | `(value: f32) -> ()` | no | for each message sent to this object or to every object, after the step; `message_name` or `message_is` tell which |
| `bayleaf_describe` | `() -> i64` | no | when the editor or play mode loads the module; returns `(ptr << 32) \| len` of UTF-8 JSON `{"properties": [["speed", 90.0], ...], "menu": "Light/Flicker"}`: the properties the editor shows, with defaults, and optionally where Add Component lists the script (1 to 4 names of up to 64 characters separated by `/`; without it, under Scripts), at most 64 KiB |

## Imports (module `bayleaf`, the host provides)

| Name | Signature | Meaning |
|---|---|---|
| `log` | `(ptr: i32, len: i32)` | writes UTF-8 text to the editor console |
| `transform_get` | `(out: i32)` | writes 9 little-endian `f32`: position xyz, rotation xyz (degrees), scale xyz |
| `transform_set` | `(ptr: i32)` | reads 9 `f32` in the same layout; non-finite values are an error |
| `property` | `(key_ptr: i32, key_len: i32) -> f32` | the property's value: the one set in the scene, else the declared default, else `NaN` |
| `time` | `() -> f64` | seconds since play mode started |
| `key_down` | `(name_ptr: i32, name_len: i32) -> i32` | `1` while the key is held (names like `W`, `Space`, `ArrowUp`), else `0` |
| `pointer` | `(out: i32) -> i32` | writes 2 `f32`: the pointer over the game view, from -1 to 1 (x right, y up); returns `1` when it is over the view, else `0` (and writes zeros) |
| `mouse_down` | `(button: i32) -> i32` | `1` while the button is held: `0` left, `1` right, `2` middle |
| `self_handle` | `() -> i32` | the handle of this script's object |
| `find` | `(name_ptr: i32, name_len: i32) -> i32` | the handle of the first object with that name, in hierarchy order, or `0` |
| `find_with` | `(kind_ptr: i32, kind_len: i32, index: i32) -> i32` | the handle of the `index`-th object (from 0, in hierarchy order, as the step began) that has a component (by its name or label: `pointLight`, `Point Light`), a tag, or runs a script (by its file name without the extension, or the name the Inspector shows); case, spaces, `_` and `-` are ignored. `0` when there is none |
| `count_with` | `(kind_ptr: i32, kind_len: i32) -> i32` | how many objects `find_with` can find for that kind |
| `component_get` | `(handle: i32, c_ptr: i32, c_len: i32, f_ptr: i32, f_len: i32, out: i32) -> i32` | writes 3 `f32`: a field of a component (by its name or label, as for `find_with`; the field by its name in scene files, such as `intensity`), as the step began: a number in the first, true/false as `1` or `0`, a vector or colour in all three. Returns `1` when the object has that component and field, else `0` (and writes zeros). An empty field asks whether the object has the component. The components are those visual scripts list (`visual::COMPONENTS`), or a script on the object by its name (as for `find_with`), whose properties are numbers |
| `component_set` | `(handle: i32, c_ptr: i32, c_len: i32, f_ptr: i32, f_len: i32, ptr: i32) -> i32` | reads 3 `f32` and asks to set the field after the step, the way `component_get` reads it (a colour keeps its alpha). `1` when asked; `0` when the object has no such field, the component is Rigid Body, Collider or Character (physics reads them when play starts), or this call already asked 64 times. A script's property is set on every script of that name on the object that declares it, when the step ends. Non-finite numbers are an error |
| `transform_get_of` | `(handle: i32, out: i32)` | like `transform_get`, for any object |
| `transform_set_of` | `(handle: i32, ptr: i32)` | like `transform_set`, for any object; applied when the call ends |
| `position_get_of` | `(handle: i32, out: i32)` | writes 3 little-endian `f64`: the object's local position in double precision, as this call last set it, else as the step began |
| `position_set_of` | `(handle: i32, ptr: i32)` | reads 3 `f64` and sets the position, keeping rotation and scale; applied when the call ends; non-finite values are an error |
| `spawn` | `(path_ptr: i32, path_len: i32, x: f32, y: f32, z: f32) -> i32` | asks for a copy of a `.bprefab` file (relative to the project folder) at a world position, after the step; `1` when asked, `0` when this call already asked 64 times |
| `destroy` | `(handle: i32) -> i32` | asks to delete the object and its children after the step; `1` or `0` as for `spawn` |
| `send` | `(handle: i32, name_ptr: i32, name_len: i32, value: f32) -> i32` | sends a message (a name of 1 to 64 bytes and a finite number) to the scripts of the object, or of every object when `handle` is `-1`; `1` when sent, `0` when this call already sent 64 |
| `message_is` | `(name_ptr: i32, name_len: i32) -> i32` | during `bayleaf_message`, `1` when the message has this name, else `0` |
| `message_name` | `(out: i32, cap: i32) -> i32` | during `bayleaf_message`, writes up to `cap` bytes of the message's name and returns its full length; `0` outside it |
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

In a build with physics, contacts arrive as messages to both objects' scripts after the physics
step: `collision` and `collisionEnd` when colliders start and stop touching, `triggerEnter` and
`triggerExit` when one of them is a trigger. The value is the other object's handle. When the game ends (`game_end`, or
the time limit of the scene's Game Rules), every object's scripts get `gameWon` or `gameLost` once,
with the value 0.

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

## Limits

- Each call may run about 10 million WebAssembly instructions ("fuel"). A script that runs out,
  traps, panics, or reads or writes outside its memory is stopped for the rest of play mode. The
  error is reported in the console and in `play.status`. Other scripts keep running.
- Linear memory may grow to 64 MiB.
- Scripts reach the world only through these imports: no files, network or clock beyond `time`.

## Scripts in Rust

In the editor, Project panel, + > New Script makes `<name>/` in the open folder: a crate with
`Cargo.toml` and `src/lib.rs` from a template that uses `bayleaf-script-sdk`. The editor builds it
at once, in the background, into `<name>.wasm` beside the folder (build files go to `.build/` in the
project); right-click the folder, Build Script, to build again. Building needs Rust and the target:
`rustup target add wasm32-unknown-unknown`. A script that is already running reloads when its
`.wasm` changes.

## Hot reload

While play mode runs, the host checks each module file for changes about twice a second. A changed
module is loaded again and its instances restart: module state is lost and `bayleaf_start` runs
again. The scene keeps the play-mode changes made so far.
