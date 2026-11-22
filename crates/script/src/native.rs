//! Native scripts: Rust scripts built into an exported game's player (native mode). Each one runs
//! as machine code with no sandbox, and reaches the host through the same host functions as a
//! WebAssembly script: a small generated module (the trampoline) imports every host function and
//! exports each again as `call_<name>`, and the SDK's bridge copies a call's texts and numbers into
//! that module's memory and calls through it. So every check and fault of the host applies.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub use bayleaf_script_sdk::native::NativeScript;

/// Every host function: name, parameter types and result type (empty for none), as WebAssembly
/// types them. The trampoline imports them all, so one missing from the host stops it loading.
const HOST_FUNCTIONS: &[(&str, &str, &str)] = &[
    ("log", "i32 i32", ""),
    ("transform_get", "i32", ""),
    ("transform_set", "i32", ""),
    ("property", "i32 i32", "f32"),
    ("property_whole", "i32 i32", "i32"),
    ("property_vector", "i32 i32 i32", "i32"),
    ("property_text", "i32 i32 i32 i32", "i32"),
    ("property_object", "i32 i32", "i32"),
    ("property_set", "i32 i32 f32 f32 f32", "i32"),
    ("property_set_text", "i32 i32 i32 i32", "i32"),
    ("time", "", "f64"),
    ("key_down", "i32 i32", "i32"),
    ("pointer", "i32", "i32"),
    ("mouse_down", "i32", "i32"),
    ("self_handle", "", "i32"),
    ("find", "i32 i32", "i32"),
    ("find_with", "i32 i32 i32", "i32"),
    ("count_with", "i32 i32", "i32"),
    ("name_of", "i32 i32 i32", "i32"),
    ("object_ref", "i32", "i32"),
    ("scene_load", "i32 i32 i32", "i32"),
    ("sound_play", "i32", "i32"),
    ("sound_stop", "i32", "i32"),
    ("sound_play_at", "i32 i32 f32 f32 f32 f32", "i32"),
    ("scene_unload", "i32 i32", "i32"),
    ("object_of", "i32", "i32"),
    ("component_get", "i32 i32 i32 i32 i32 i32", "i32"),
    ("component_set", "i32 i32 i32 i32 i32 i32", "i32"),
    ("transform_get_of", "i32 i32", ""),
    ("transform_set_of", "i32 i32", ""),
    ("position_get_of", "i32 i32", ""),
    ("position_set_of", "i32 i32", ""),
    ("spawn", "i32 i32 f32 f32 f32", "i32"),
    ("destroy", "i32", "i32"),
    ("send", "i32 i32 i32 f32", "i32"),
    ("message_name", "i32 i32", "i32"),
    ("pad_connected", "", "i32"),
    ("apply_impulse", "i32 f32 f32 f32", "i32"),
    ("character_walk", "i32 f32 f32 f32", "i32"),
    ("action", "i32 i32", "f32"),
    ("character_jump", "i32 f32", "i32"),
    ("character_grounded", "i32", "i32"),
    ("set_velocity", "i32 f32 f32 f32", "i32"),
    ("velocity_of", "i32 i32", "i32"),
    ("raycast", "f32 f32 f32 f32 f32 f32 f32 i32", "i32"),
    ("pad_axis", "i32", "f32"),
    ("pad_button", "i32", "i32"),
    ("state_get", "i32 i32 i32 i32 i32", "i32"),
    ("state_set", "i32 i32 i32 i32 f64", "i32"),
    ("game_end", "i32", "i32"),
    ("input_map_set", "i32 i32 i32", "i32"),
];

/// Pages of exchange memory (64 KiB each) a native script's calls copy their texts and numbers through.
const EXCHANGE_PAGES: u32 = 4;

/// The WebAssembly text of the trampoline module.
fn trampoline_text() -> String {
    let mut imports = String::new();
    let mut exports = String::new();
    for (name, params, result) in HOST_FUNCTIONS {
        let params: Vec<&str> = params.split_whitespace().collect();
        let declared = if params.is_empty() { String::new() } else { format!(" (param {})", params.join(" ")) };
        let returns = if result.is_empty() { String::new() } else { format!(" (result {result})") };
        imports.push_str(&format!("  (import \"bayleaf\" \"{name}\" (func ${name}{declared}{returns}))\n"));
        let gets: String = (0..params.len()).map(|i| format!(" local.get {i}")).collect();
        exports.push_str(&format!("  (func (export \"call_{name}\"){declared}{returns}{gets} call ${name})\n"));
    }
    format!(
        "(module\n{imports}  (import \"native\" \"init\" (func $init))\n  (import \"native\" \"start\" (func $start))\n  (import \"native\" \"update\" (func $update (param f32)))\n  (import \"native\" \"message\" (func $message (param f32)))\n  (import \"native\" \"describe\" (func $describe (result i64)))\n  (memory (export \"memory\") {EXCHANGE_PAGES})\n  (func (export \"bayleaf_abi\") (result i32) i32.const {abi})\n  (func (export \"bayleaf_init\") call $init)\n  (func (export \"bayleaf_start\") call $start)\n  (func (export \"bayleaf_update\") (param f32) local.get 0 call $update)\n  (func (export \"bayleaf_message\") (param f32) local.get 0 call $message)\n  (func (export \"bayleaf_describe\") (result i64) call $describe)\n{exports})",
        abi = crate::ABI_VERSION
    )
}

/// The trampoline module's bytes, assembled once.
pub(crate) fn trampoline() -> Result<&'static [u8], String> {
    static BYTES: OnceLock<Result<Vec<u8>, String>> = OnceLock::new();
    BYTES
        .get_or_init(|| wat::parse_str(trampoline_text()).map_err(|e| format!("the native script bridge does not assemble: {e}")))
        .as_deref()
        .map_err(Clone::clone)
}

/// The native script built in for the module at `path`, if any: one registered under a path that
/// `path` ends with (`scripts/mover.wasm`).
pub(crate) fn find(natives: &[(PathBuf, NativeScript)], path: &Path) -> Option<NativeScript> {
    natives.iter().find(|(module, _)| path.ends_with(module)).map(|(_, script)| *script)
}
