# Script ABI, version 1

A game script is a WebAssembly module (`wasm32`, no WASI). An object carries a list of scripts (the
`scripts` component); the inspector shows each as its own component, named after its module file
(`player_controller.wasm` -> "Player Controller"). Every script on every object gets its own instance,
so module-level state belongs to that one script slot. Any language that compiles to WebAssembly can implement this ABI. The Rust SDK is
`crates/script-sdk`, and visual scripts will compile to the same ABI (milestone M5).

## Exports (the script provides)

| Name | Signature | Required | Called |
|---|---|---|---|
| `memory` | linear memory | yes | the host reads and writes strings and transforms here |
| `bayleaf_abi` | `() -> i32` | yes | at load; must return `1` |
| `bayleaf_start` | `() -> ()` | no | once, when play mode starts or after a hot reload |
| `bayleaf_update` | `(dt: f32) -> ()` | no | every simulation step; `dt` is in seconds |
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

The transform is the object's local transform, relative to its parent.

## Limits

- Each call may run about 10 million WebAssembly instructions ("fuel"). A script that runs out,
  traps, panics, or reads or writes outside its memory is stopped for the rest of play mode. The
  error is reported in the console and in `play.status`. Other scripts keep running.
- Linear memory may grow to 64 MiB.
- Scripts reach the world only through these imports: no files, network or clock beyond `time`.

## Hot reload

While play mode runs, the host checks each module file for changes about twice a second. A changed
module is loaded again and its instances restart: module state is lost and `bayleaf_start` runs
again. The scene keeps the play-mode changes made so far.
