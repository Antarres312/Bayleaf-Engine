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
| `bayleaf_describe` | `() -> i64` | no | when the editor or play mode loads the module; returns `(ptr << 32) \| len` of UTF-8 JSON `{"properties": [["speed", 90.0], ...]}`: the properties the editor shows, with defaults, at most 64 KiB |

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
| `transform_get_of` | `(handle: i32, out: i32)` | like `transform_get`, for any object |
| `transform_set_of` | `(handle: i32, ptr: i32)` | like `transform_set`, for any object; applied when the call ends |
| `spawn` | `(path_ptr: i32, path_len: i32, x: f32, y: f32, z: f32) -> i32` | asks for a copy of a `.bprefab` file (relative to the project folder) at a world position, after the step; `1` when asked, `0` when this call already asked 64 times |
| `destroy` | `(handle: i32) -> i32` | asks to delete the object and its children after the step; `1` or `0` as for `spawn` |
| `send` | `(handle: i32, name_ptr: i32, name_len: i32, value: f32) -> i32` | sends a message (a name of 1 to 64 bytes and a finite number) to the scripts of the object, or of every object when `handle` is `-1`; `1` when sent, `0` when this call already sent 64 |
| `message_is` | `(name_ptr: i32, name_len: i32) -> i32` | during `bayleaf_message`, `1` when the message has this name, else `0` |
| `message_name` | `(out: i32, cap: i32) -> i32` | during `bayleaf_message`, writes up to `cap` bytes of the message's name and returns its full length; `0` outside it |

### Messages

Messages are delivered after every script's update in the step, in the order sent. Each script of
the target object that exports `bayleaf_message` gets the call, with the same limits as any other
call. A message sent while handling one is delivered after the next step, so scripts that answer
each other cannot run forever. Messages sent from `bayleaf_start` arrive after the first step. The
Rust SDK calls `Script::message(name, value)`; `Object::send` and `send_to_all` send.

### Other objects

A handle names an object for one step: it is the object's place in the scene as the step began,
plus one (`0` means none). During a step every script sees the objects as they were when it began,
except for the transforms its own call has set. Using a handle that names no object stops the
script with a reason. Spawns and destroys happen after the step, in the order asked; a spawned
copy's scripts start at once and run from the next step. Play mode puts everything back on stop.
These imports came after the first ABI release; a module that uses them needs an engine that has
them, and fails to load on an older one with a message naming the missing import.

The transform is the object's local transform, relative to its parent.

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
