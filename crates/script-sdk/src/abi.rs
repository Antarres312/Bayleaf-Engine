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
        pub fn spawn(path_ptr: *const u8, path_len: usize, x: f32, y: f32, z: f32) -> i32;
        pub fn destroy(handle: i32) -> i32;
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
    pub unsafe fn spawn(_: *const u8, _: usize, _: f32, _: f32, _: f32) -> i32 {
        0
    }
    pub unsafe fn destroy(_: i32) -> i32 {
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
