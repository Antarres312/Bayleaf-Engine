//! Raw imports from the host (module `bayleaf`), wrapped in safe functions.
//! This is the only module in the SDK with `unsafe`; see `docs/script-abi.md` for the contract.

#![allow(unsafe_code)]

#[cfg(target_arch = "wasm32")]
mod raw {
    #[link(wasm_import_module = "bayleaf")]
    unsafe extern "C" {
        pub fn log(ptr: *const u8, len: usize);
        pub fn transform_get(out: *mut f32);
        pub fn transform_set(ptr: *const f32);
        pub fn property(key_ptr: *const u8, key_len: usize) -> f32;
        pub fn time() -> f64;
        pub fn key_down(name_ptr: *const u8, name_len: usize) -> i32;
        pub fn pointer(out: *mut f32) -> i32;
        pub fn mouse_down(button: i32) -> i32;
        pub fn self_handle() -> i32;
        pub fn find(name_ptr: *const u8, name_len: usize) -> i32;
        pub fn transform_get_of(handle: i32, out: *mut f32);
        pub fn transform_set_of(handle: i32, ptr: *const f32);
        pub fn position_get_of(handle: i32, out: *mut f64);
        pub fn position_set_of(handle: i32, ptr: *const f64);
        pub fn spawn(path_ptr: *const u8, path_len: usize, x: f32, y: f32, z: f32) -> i32;
        pub fn destroy(handle: i32) -> i32;
        pub fn send(handle: i32, name_ptr: *const u8, name_len: usize, value: f32) -> i32;
        pub fn message_name(out: *mut u8, cap: usize) -> i32;
        pub fn pad_connected() -> i32;
        pub fn apply_impulse(handle: i32, x: f32, y: f32, z: f32) -> i32;
        pub fn character_walk(handle: i32, x: f32, y: f32, z: f32) -> i32;
        pub fn action(name_ptr: *const u8, name_len: usize) -> f32;
        pub fn character_jump(handle: i32, speed: f32) -> i32;
        pub fn character_grounded(handle: i32) -> i32;
        pub fn set_velocity(handle: i32, x: f32, y: f32, z: f32) -> i32;
        pub fn velocity_of(handle: i32, out: *mut f32) -> i32;
        pub fn raycast(ox: f32, oy: f32, oz: f32, dx: f32, dy: f32, dz: f32, max: f32, out: *mut f32) -> i32;
        pub fn pad_axis(index: i32) -> f32;
        pub fn pad_button(index: i32) -> i32;
        pub fn state_get(scope: i32, handle: i32, name_ptr: *const u8, name_len: usize, out: *mut f64) -> i32;
        pub fn state_set(scope: i32, handle: i32, name_ptr: *const u8, name_len: usize, value: f64) -> i32;
        pub fn game_end(won: i32) -> i32;
        pub fn input_map_set(handle: i32, name_ptr: *const u8, name_len: usize) -> i32;
    }
}

/// Off WebAssembly (unit tests on the host machine) there is no host: calls do nothing.
#[cfg(not(target_arch = "wasm32"))]
mod raw {
    pub unsafe fn log(_: *const u8, _: usize) {}
    pub unsafe fn transform_get(out: *mut f32) {
        // SAFETY: callers pass a pointer to 9 writable f32.
        unsafe { std::ptr::copy_nonoverlapping([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0f32].as_ptr(), out, 9) }
    }
    pub unsafe fn transform_set(_: *const f32) {}
    pub unsafe fn property(_: *const u8, _: usize) -> f32 {
        f32::NAN
    }
    pub unsafe fn time() -> f64 {
        0.0
    }
    pub unsafe fn key_down(_: *const u8, _: usize) -> i32 {
        0
    }
    pub unsafe fn pointer(_: *mut f32) -> i32 {
        0
    }
    pub unsafe fn mouse_down(_: i32) -> i32 {
        0
    }
    pub unsafe fn self_handle() -> i32 {
        0
    }
    pub unsafe fn find(_: *const u8, _: usize) -> i32 {
        0
    }
    pub unsafe fn transform_get_of(_: i32, out: *mut f32) {
        // SAFETY: callers pass a pointer to 9 writable f32.
        unsafe { std::ptr::copy_nonoverlapping([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0f32].as_ptr(), out, 9) }
    }
    pub unsafe fn transform_set_of(_: i32, _: *const f32) {}
    pub unsafe fn position_get_of(_: i32, out: *mut f64) {
        // SAFETY: callers pass a pointer to 3 writable f64.
        unsafe { std::ptr::copy_nonoverlapping([0.0f64; 3].as_ptr(), out, 3) }
    }
    pub unsafe fn position_set_of(_: i32, _: *const f64) {}
    pub unsafe fn spawn(_: *const u8, _: usize, _: f32, _: f32, _: f32) -> i32 {
        0
    }
    pub unsafe fn destroy(_: i32) -> i32 {
        0
    }
    pub unsafe fn send(_: i32, _: *const u8, _: usize, _: f32) -> i32 {
        0
    }
    pub unsafe fn message_name(_: *mut u8, _: usize) -> i32 {
        0
    }
    pub unsafe fn pad_connected() -> i32 {
        0
    }
    pub unsafe fn apply_impulse(_: i32, _: f32, _: f32, _: f32) -> i32 {
        0
    }
    pub unsafe fn character_walk(_: i32, _: f32, _: f32, _: f32) -> i32 {
        0
    }
    pub unsafe fn action(_: *const u8, _: usize) -> f32 {
        0.0
    }
    pub unsafe fn character_jump(_: i32, _: f32) -> i32 {
        0
    }
    pub unsafe fn character_grounded(_: i32) -> i32 {
        0
    }
    pub unsafe fn set_velocity(_: i32, _: f32, _: f32, _: f32) -> i32 {
        0
    }
    pub unsafe fn velocity_of(_: i32, _: *mut f32) -> i32 {
        0
    }
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn raycast(_: f32, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32, _: *mut f32) -> i32 {
        0
    }
    pub unsafe fn pad_axis(_: i32) -> f32 {
        0.0
    }
    pub unsafe fn pad_button(_: i32) -> i32 {
        0
    }
    pub unsafe fn state_get(_: i32, _: i32, _: *const u8, _: usize, _: *mut f64) -> i32 {
        0
    }
    pub unsafe fn state_set(_: i32, _: i32, _: *const u8, _: usize, _: f64) -> i32 {
        0
    }
    pub unsafe fn game_end(_: i32) -> i32 {
        0
    }
    pub unsafe fn input_map_set(_: i32, _: *const u8, _: usize) -> i32 {
        0
    }
}

pub(crate) fn log(text: &str) {
    // SAFETY: the pointer and length describe `text`, which outlives the call; the host only reads.
    unsafe { raw::log(text.as_ptr(), text.len()) }
}

pub(crate) fn transform_get() -> [f32; 9] {
    let mut values = [0.0f32; 9];
    // SAFETY: `values` is 9 writable f32, exactly what the host writes.
    unsafe { raw::transform_get(values.as_mut_ptr()) }
    values
}

pub(crate) fn transform_set(values: &[f32; 9]) {
    // SAFETY: `values` is 9 readable f32, exactly what the host reads.
    unsafe { raw::transform_set(values.as_ptr()) }
}

pub(crate) fn property(name: &str) -> f32 {
    // SAFETY: the pointer and length describe `name`; the host only reads.
    unsafe { raw::property(name.as_ptr(), name.len()) }
}

pub(crate) fn time() -> f64 {
    // SAFETY: no arguments; the host returns a plain number.
    unsafe { raw::time() }
}

pub(crate) fn key_down(name: &str) -> bool {
    // SAFETY: the pointer and length describe `name`; the host only reads.
    unsafe { raw::key_down(name.as_ptr(), name.len()) != 0 }
}

pub(crate) fn pointer() -> Option<[f32; 2]> {
    let mut xy = [0.0f32; 2];
    // SAFETY: `xy` is 2 writable f32, exactly what the host writes.
    let inside = unsafe { raw::pointer(xy.as_mut_ptr()) };
    (inside != 0).then_some(xy)
}

pub(crate) fn mouse_down(button: i32) -> bool {
    // SAFETY: a plain number in and out.
    unsafe { raw::mouse_down(button) != 0 }
}

pub(crate) fn self_handle() -> i32 {
    // SAFETY: no arguments; the host returns a plain number.
    unsafe { raw::self_handle() }
}

pub(crate) fn find(name: &str) -> i32 {
    // SAFETY: the pointer and length describe `name`; the host only reads.
    unsafe { raw::find(name.as_ptr(), name.len()) }
}

pub(crate) fn transform_get_of(handle: i32) -> [f32; 9] {
    let mut values = [0.0f32; 9];
    // SAFETY: `values` is 9 writable f32, exactly what the host writes.
    unsafe { raw::transform_get_of(handle, values.as_mut_ptr()) }
    values
}

pub(crate) fn transform_set_of(handle: i32, values: &[f32; 9]) {
    // SAFETY: `values` is 9 readable f32, exactly what the host reads.
    unsafe { raw::transform_set_of(handle, values.as_ptr()) }
}

pub(crate) fn spawn(prefab: &str, position: [f32; 3]) -> bool {
    // SAFETY: the pointer and length describe `prefab`; the host only reads.
    unsafe { raw::spawn(prefab.as_ptr(), prefab.len(), position[0], position[1], position[2]) != 0 }
}

pub(crate) fn destroy(handle: i32) -> bool {
    // SAFETY: a plain number in and out.
    unsafe { raw::destroy(handle) != 0 }
}

pub(crate) fn send(handle: i32, name: &str, value: f32) -> bool {
    // SAFETY: the pointer and length describe `name`; the host only reads.
    unsafe { raw::send(handle, name.as_ptr(), name.len(), value) != 0 }
}

/// The name of the message being delivered, or an empty string outside `bayleaf_message`.
pub(crate) fn message_name() -> String {
    let mut buffer = [0u8; 64];
    // SAFETY: `buffer` is 64 writable bytes and the host writes at most `cap` of them.
    let len = unsafe { raw::message_name(buffer.as_mut_ptr(), buffer.len()) };
    let len = usize::try_from(len).unwrap_or(0).min(buffer.len());
    String::from_utf8_lossy(&buffer[..len]).into_owned()
}

pub(crate) fn pad_connected() -> bool {
    // SAFETY: no arguments; the host returns a plain number.
    unsafe { raw::pad_connected() != 0 }
}

pub(crate) fn pad_axis(index: i32) -> f32 {
    // SAFETY: a plain number in and out.
    unsafe { raw::pad_axis(index) }
}

pub(crate) fn pad_button(index: i32) -> bool {
    // SAFETY: a plain number in and out.
    unsafe { raw::pad_button(index) != 0 }
}

/// The handle of the first object a ray hits (0 for none), its distance and the hit point.
pub(crate) fn raycast(origin: [f32; 3], direction: [f32; 3], max: f32) -> (i32, [f32; 4]) {
    let mut out = [0.0f32; 4];
    let [ox, oy, oz] = origin;
    let [dx, dy, dz] = direction;
    // SAFETY: `out` is 4 writable f32, exactly what the host writes.
    let handle = unsafe { raw::raycast(ox, oy, oz, dx, dy, dz, max, out.as_mut_ptr()) };
    (handle, out)
}

pub(crate) fn apply_impulse(handle: i32, v: [f32; 3]) -> bool {
    // SAFETY: plain numbers in and out.
    unsafe { raw::apply_impulse(handle, v[0], v[1], v[2]) != 0 }
}

pub(crate) fn set_velocity(handle: i32, v: [f32; 3]) -> bool {
    // SAFETY: plain numbers in and out.
    unsafe { raw::set_velocity(handle, v[0], v[1], v[2]) != 0 }
}

pub(crate) fn velocity_of(handle: i32) -> Option<[f32; 3]> {
    let mut out = [0.0f32; 3];
    // SAFETY: `out` is 3 writable f32, exactly what the host writes.
    let found = unsafe { raw::velocity_of(handle, out.as_mut_ptr()) };
    (found != 0).then_some(out)
}

pub(crate) fn position_get_of(handle: i32) -> [f64; 3] {
    let mut values = [0.0f64; 3];
    // SAFETY: `values` is 3 writable f64, exactly what the host writes.
    unsafe { raw::position_get_of(handle, values.as_mut_ptr()) }
    values
}

pub(crate) fn position_set_of(handle: i32, values: &[f64; 3]) {
    // SAFETY: `values` is 3 readable f64, exactly what the host reads.
    unsafe { raw::position_set_of(handle, values.as_ptr()) }
}

pub(crate) fn character_walk(handle: i32, v: [f32; 3]) -> bool {
    // SAFETY: plain numbers in and out.
    unsafe { raw::character_walk(handle, v[0], v[1], v[2]) != 0 }
}

pub(crate) fn character_jump(handle: i32, speed: f32) -> bool {
    // SAFETY: plain numbers in and out.
    unsafe { raw::character_jump(handle, speed) != 0 }
}

pub(crate) fn character_grounded(handle: i32) -> bool {
    // SAFETY: a plain number in and out.
    unsafe { raw::character_grounded(handle) != 0 }
}

pub(crate) fn action(name: &str) -> f32 {
    // SAFETY: the pointer and length describe `name`; the host only reads.
    unsafe { raw::action(name.as_ptr(), name.len()) }
}

/// A value of the game's state (scope 0), a player's (scope 1; handle 0 for the first player) or a
/// saved one (scope 2).
pub(crate) fn state_get(scope: i32, handle: i32, name: &str) -> Option<f64> {
    let mut out = 0.0f64;
    // SAFETY: the pointer and length describe `name`, which the host only reads; `out` is one
    // writable f64 that outlives the call.
    let set = unsafe { raw::state_get(scope, handle, name.as_ptr(), name.len(), &mut out) != 0 };
    set.then_some(out)
}

pub(crate) fn state_set(scope: i32, handle: i32, name: &str, value: f64) -> bool {
    // SAFETY: the pointer and length describe `name`; the host only reads.
    unsafe { raw::state_set(scope, handle, name.as_ptr(), name.len(), value) != 0 }
}

/// Makes a map active: the project's Input Map (handle 0) or an object's own.
pub(crate) fn input_map_set(handle: i32, map: &str) -> bool {
    // SAFETY: the pointer and length describe `map`; the host only reads.
    unsafe { raw::input_map_set(handle, map.as_ptr(), map.len()) != 0 }
}

pub(crate) fn game_end(won: bool) -> bool {
    // SAFETY: a plain number in and out.
    unsafe { raw::game_end(i32::from(won)) != 0 }
}
