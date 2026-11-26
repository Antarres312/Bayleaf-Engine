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
        pub fn property_whole(key_ptr: *const u8, key_len: usize) -> i32;
        pub fn property_vector(key_ptr: *const u8, key_len: usize, out: *mut f32) -> i32;
        pub fn property_text(key_ptr: *const u8, key_len: usize, out: *mut u8, cap: usize) -> i32;
        pub fn property_object(key_ptr: *const u8, key_len: usize) -> i32;
        pub fn property_set(key_ptr: *const u8, key_len: usize, x: f32, y: f32, z: f32) -> i32;
        pub fn property_set_text(key_ptr: *const u8, key_len: usize, text_ptr: *const u8, text_len: usize) -> i32;
        pub fn time() -> f64;
        pub fn key_down(name_ptr: *const u8, name_len: usize) -> i32;
        pub fn pointer(out: *mut f32) -> i32;
        pub fn mouse_down(button: i32) -> i32;
        pub fn pointer_over_interface() -> i32;
        pub fn self_handle() -> i32;
        pub fn find(name_ptr: *const u8, name_len: usize) -> i32;
        pub fn find_with(kind_ptr: *const u8, kind_len: usize, index: i32) -> i32;
        pub fn count_with(kind_ptr: *const u8, kind_len: usize) -> i32;
        pub fn name_of(handle: i32, out: *mut u8, cap: usize) -> i32;
        pub fn object_ref(handle: i32) -> i32;
        pub fn scene_load(path_ptr: *const u8, path_len: usize, additive: i32) -> i32;
        pub fn particles_play(handle: i32) -> i32;
        pub fn particles_stop(handle: i32) -> i32;
        pub fn particles_burst(handle: i32, count: i32) -> i32;
        pub fn ui_set_text(handle: i32, name_ptr: *const u8, name_len: usize, text_ptr: *const u8, text_len: usize) -> i32;
        pub fn ui_set_value(handle: i32, name_ptr: *const u8, name_len: usize, value: f64) -> i32;
        pub fn ui_show(handle: i32, name_ptr: *const u8, name_len: usize, visible: i32) -> i32;
        pub fn ui_add(handle: i32, parent_ptr: *const u8, parent_len: usize, what_ptr: *const u8, what_len: usize, name_ptr: *const u8, name_len: usize)
        -> i32;
        pub fn ui_remove(handle: i32, name_ptr: *const u8, name_len: usize) -> i32;
        pub fn ui_value(handle: i32, name_ptr: *const u8, name_len: usize) -> f64;
        pub fn ui_text(handle: i32, name_ptr: *const u8, name_len: usize, out: *mut u8, cap: usize) -> i32;
        pub fn sound_play(handle: i32) -> i32;
        pub fn sound_stop(handle: i32) -> i32;
        pub fn sound_play_at(path_ptr: *const u8, path_len: usize, x: f32, y: f32, z: f32, volume: f32) -> i32;
        pub fn scene_unload(path_ptr: *const u8, path_len: usize) -> i32;
        pub fn object_of(reference: i32) -> i32;
        pub fn component_get(handle: i32, c_ptr: *const u8, c_len: usize, f_ptr: *const u8, f_len: usize, out: *mut f32) -> i32;
        pub fn component_set(handle: i32, c_ptr: *const u8, c_len: usize, f_ptr: *const u8, f_len: usize, value: *const f32) -> i32;
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
        pub fn event_send(handle: i32, name_ptr: *const u8, name_len: usize, values: *const u8, count: i32) -> i32;
        pub fn event_number(index: i32) -> f32;
        pub fn event_whole(index: i32) -> i32;
        pub fn event_flag(index: i32) -> i32;
        pub fn event_vector(index: i32, part: i32) -> f32;
        pub fn event_object(index: i32) -> i32;
        pub fn event_text(index: i32, out: *mut u8, cap: usize) -> i32;
        pub fn script_component(handle: i32, name_ptr: *const u8, name_len: usize) -> i32;
        pub fn self_component() -> i32;
        pub fn event_send_to(handle: i32, script: i32, name_ptr: *const u8, name_len: usize, values: *const u8, count: i32) -> i32;
        pub fn event_subscribe(handle: i32, script: i32, name_ptr: *const u8, name_len: usize) -> i32;
        pub fn event_unsubscribe(handle: i32, script: i32, name_ptr: *const u8, name_len: usize) -> i32;
        pub fn event_source() -> i32;
        pub fn event_source_component() -> i32;
        pub fn slot_select(slot: i32) -> i32;
        pub fn frame_get(out: *mut u8, cap: i32) -> i32;
        pub fn frame_set(ptr: *const u8, count: i32);
        pub fn world_stamp() -> i64;
    }
}

/// Off WebAssembly, a script is built into the game (native mode): each import goes through the
/// [`crate::native::Bridge`] the host sets around the call, its texts and numbers copied through
/// the bridge's exchange memory. Without a bridge (a script's own unit tests) calls do nothing.
#[cfg(not(target_arch = "wasm32"))]
mod raw {
    use super::bridge::{Exchange, run};
    use crate::native::Value::{F32, F64, I32};

    /// The `len` bytes at `ptr`.
    ///
    /// # Safety
    /// `ptr` and `len` describe a live slice (any pointer when `len` is 0).
    unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
        if len == 0 {
            return &[];
        }
        // SAFETY: the caller promises `len` readable bytes at `ptr`.
        unsafe { std::slice::from_raw_parts(ptr, len) }
    }

    /// The `n` numbers at `ptr` as little-endian bytes, as the host reads them.
    ///
    /// # Safety
    /// `ptr` points to `n` readable `T`.
    unsafe fn numbers<T: Copy + Bytes>(ptr: *const T, n: usize) -> Vec<u8> {
        // SAFETY: the caller promises `n` readable values at `ptr`.
        let values = unsafe { std::slice::from_raw_parts(ptr, n) };
        values.iter().flat_map(|v| v.le()).collect()
    }

    /// Writes the numbers in `data` (little-endian, as the host wrote them) to `out`.
    ///
    /// # Safety
    /// `out` points to room for `data.len() / size_of::<T>()` values of `T`.
    unsafe fn numbers_out<T: Bytes>(out: *mut T, data: &[u8]) {
        for (i, chunk) in data.chunks_exact(std::mem::size_of::<T>()).enumerate() {
            // SAFETY: the caller promises room for every whole value in `data`.
            unsafe { out.add(i).write(T::from_le(chunk)) }
        }
    }

    /// Writes at most `cap` bytes of `data` to `out`.
    ///
    /// # Safety
    /// `out` points to `cap` writable bytes (any pointer when `cap` is 0).
    unsafe fn bytes_out(out: *mut u8, cap: usize, data: &[u8]) {
        let n = data.len().min(cap);
        if n > 0 {
            // SAFETY: `n` is at most `cap`, which the caller promises is writable at `out`.
            unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), out, n) }
        }
    }

    /// Numbers as the host stores them.
    trait Bytes: Sized {
        fn le(&self) -> Vec<u8>;
        fn from_le(bytes: &[u8]) -> Self;
    }

    impl Bytes for f32 {
        fn le(&self) -> Vec<u8> {
            self.to_le_bytes().to_vec()
        }
        fn from_le(bytes: &[u8]) -> Self {
            let mut b = [0u8; 4];
            b.copy_from_slice(bytes);
            f32::from_le_bytes(b)
        }
    }

    impl Bytes for f64 {
        fn le(&self) -> Vec<u8> {
            self.to_le_bytes().to_vec()
        }
        fn from_le(bytes: &[u8]) -> Self {
            let mut b = [0u8; 8];
            b.copy_from_slice(bytes);
            f64::from_le_bytes(b)
        }
    }

    fn len(n: usize) -> crate::native::Value {
        I32(i32::try_from(n).unwrap_or(i32::MAX))
    }

    /// A call with one text argument, returning a whole number (0 without a host).
    fn text_call(name: &'static str, text: &[u8], more: &[crate::native::Value]) -> i32 {
        run(|x| {
            let at = x.put(text)?;
            let mut args = vec![at, len(text.len())];
            args.extend_from_slice(more);
            x.call(name, &args)?.i32()
        })
        .unwrap_or(0)
    }

    /// A call with numbers first, then one text, returning a whole number (0 without a host).
    fn numbers_then_text(name: &'static str, before: &[crate::native::Value], text: &[u8]) -> i32 {
        run(|x| {
            let at = x.put(text)?;
            let mut args = before.to_vec();
            args.extend([at, len(text.len())]);
            x.call(name, &args)?.i32()
        })
        .unwrap_or(0)
    }

    /// A call with plain numbers, returning a whole number (0 without a host).
    fn plain(name: &'static str, args: &[crate::native::Value]) -> i32 {
        run(|x| x.call(name, args)?.i32()).unwrap_or(0)
    }

    /// A call that writes `n` numbers of `T` for the script at the end of its arguments.
    fn with_out<T: Bytes>(name: &'static str, args: &[crate::native::Value], n: usize, x: &mut Exchange) -> Option<(crate::native::Value, Vec<u8>)> {
        let room = x.room(n * std::mem::size_of::<T>())?;
        let mut all = args.to_vec();
        all.push(room);
        let result = x.call(name, &all)?;
        Some((result, x.get(room, n * std::mem::size_of::<T>())?))
    }

    pub unsafe fn log(ptr: *const u8, n: usize) {
        // SAFETY: the safe wrappers pass a live slice.
        let text = unsafe { bytes(ptr, n) };
        text_call("log", text, &[]);
    }
    pub unsafe fn transform_get(out: *mut f32) {
        let data = run(|x| with_out::<f32>("transform_get", &[], 9, x).map(|(_, d)| d))
            .unwrap_or_else(|| [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0f32].iter().flat_map(|v| v.to_le_bytes()).collect());
        // SAFETY: callers pass room for 9 f32.
        unsafe { numbers_out(out, &data) }
    }
    pub unsafe fn transform_set(ptr: *const f32) {
        // SAFETY: callers pass 9 readable f32.
        let data = unsafe { numbers(ptr, 9) };
        run(|x| {
            let at = x.put(&data)?;
            x.call("transform_set", &[at])
        });
    }
    pub unsafe fn property(ptr: *const u8, n: usize) -> f32 {
        // SAFETY: the safe wrappers pass a live slice.
        let key = unsafe { bytes(ptr, n) };
        run(|x| {
            let at = x.put(key)?;
            x.call("property", &[at, len(key.len())])?.f32()
        })
        .unwrap_or(f32::NAN)
    }
    pub unsafe fn property_whole(ptr: *const u8, n: usize) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        text_call("property_whole", unsafe { bytes(ptr, n) }, &[])
    }
    pub unsafe fn property_vector(ptr: *const u8, n: usize, out: *mut f32) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        let key = unsafe { bytes(ptr, n) };
        let (found, data) = run(|x| {
            let at = x.put(key)?;
            let (found, data) = with_out::<f32>("property_vector", &[at, len(key.len())], 3, x)?;
            Some((found.i32()?, data))
        })
        .unwrap_or((0, Vec::new()));
        // SAFETY: callers pass room for 3 f32.
        unsafe { numbers_out(out, &data) }
        found
    }
    pub unsafe fn property_text(ptr: *const u8, n: usize, out: *mut u8, cap: usize) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        let key = unsafe { bytes(ptr, n) };
        let (full, data) = run(|x| {
            let at = x.put(key)?;
            let room = x.room(cap)?;
            let full = x.call("property_text", &[at, len(key.len()), room, len(cap)])?.i32()?;
            let got = usize::try_from(full).unwrap_or(0).min(cap);
            Some((full, x.get(room, got)?))
        })
        .unwrap_or((0, Vec::new()));
        // SAFETY: callers pass `cap` writable bytes at `out`.
        unsafe { bytes_out(out, cap, &data) }
        full
    }
    pub unsafe fn property_object(ptr: *const u8, n: usize) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        text_call("property_object", unsafe { bytes(ptr, n) }, &[])
    }
    pub unsafe fn property_set(ptr: *const u8, n: usize, x: f32, y: f32, z: f32) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        text_call("property_set", unsafe { bytes(ptr, n) }, &[F32(x), F32(y), F32(z)])
    }
    pub unsafe fn property_set_text(ptr: *const u8, n: usize, t_ptr: *const u8, t_len: usize) -> i32 {
        // SAFETY: the safe wrappers pass live slices.
        let (key, text) = unsafe { (bytes(ptr, n), bytes(t_ptr, t_len)) };
        run(|x| {
            let k = x.put(key)?;
            let t = x.put(text)?;
            x.call("property_set_text", &[k, len(key.len()), t, len(text.len())])?.i32()
        })
        .unwrap_or(0)
    }
    pub unsafe fn time() -> f64 {
        run(|x| x.call("time", &[])?.f64()).unwrap_or(0.0)
    }
    pub unsafe fn key_down(ptr: *const u8, n: usize) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        text_call("key_down", unsafe { bytes(ptr, n) }, &[])
    }
    pub unsafe fn pointer(out: *mut f32) -> i32 {
        let (inside, data) = run(|x| {
            let (inside, data) = with_out::<f32>("pointer", &[], 2, x)?;
            Some((inside.i32()?, data))
        })
        .unwrap_or((0, Vec::new()));
        // SAFETY: callers pass room for 2 f32.
        unsafe { numbers_out(out, &data) }
        inside
    }
    pub unsafe fn mouse_down(button: i32) -> i32 {
        plain("mouse_down", &[I32(button)])
    }
    pub unsafe fn pointer_over_interface() -> i32 {
        plain("pointer_over_interface", &[])
    }
    pub unsafe fn self_handle() -> i32 {
        plain("self_handle", &[])
    }
    pub unsafe fn find(ptr: *const u8, n: usize) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        text_call("find", unsafe { bytes(ptr, n) }, &[])
    }
    pub unsafe fn find_with(ptr: *const u8, n: usize, index: i32) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        text_call("find_with", unsafe { bytes(ptr, n) }, &[I32(index)])
    }
    pub unsafe fn count_with(ptr: *const u8, n: usize) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        text_call("count_with", unsafe { bytes(ptr, n) }, &[])
    }
    pub unsafe fn name_of(handle: i32, out: *mut u8, cap: usize) -> i32 {
        let (full, data) = run(|x| {
            let room = x.room(cap)?;
            let full = x.call("name_of", &[I32(handle), room, len(cap)])?.i32()?;
            let got = usize::try_from(full).unwrap_or(0).min(cap);
            Some((full, x.get(room, got)?))
        })
        .unwrap_or((0, Vec::new()));
        // SAFETY: callers pass `cap` writable bytes at `out`.
        unsafe { bytes_out(out, cap, &data) }
        full
    }
    pub unsafe fn object_ref(handle: i32) -> i32 {
        plain("object_ref", &[I32(handle)])
    }
    pub unsafe fn slot_select(slot: i32) -> i32 {
        plain("slot_select", &[I32(slot)])
    }
    pub unsafe fn scene_load(ptr: *const u8, n: usize, additive: i32) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        text_call("scene_load", unsafe { bytes(ptr, n) }, &[I32(additive)])
    }
    pub unsafe fn particles_play(handle: i32) -> i32 {
        plain("particles_play", &[I32(handle)])
    }
    pub unsafe fn particles_stop(handle: i32) -> i32 {
        plain("particles_stop", &[I32(handle)])
    }
    pub unsafe fn particles_burst(handle: i32, count: i32) -> i32 {
        plain("particles_burst", &[I32(handle), I32(count)])
    }
    pub unsafe fn ui_set_text(handle: i32, name_ptr: *const u8, name_len: usize, text_ptr: *const u8, text_len: usize) -> i32 {
        // SAFETY: the safe wrappers pass live slices.
        let (name, text) = unsafe { (bytes(name_ptr, name_len), bytes(text_ptr, text_len)) };
        run(|x| {
            let at = x.put(name)?;
            let text_at = x.put(text)?;
            x.call("ui_set_text", &[I32(handle), at, len(name.len()), text_at, len(text.len())])?.i32()
        })
        .unwrap_or(0)
    }
    pub unsafe fn ui_set_value(handle: i32, name_ptr: *const u8, name_len: usize, value: f64) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        let name = unsafe { bytes(name_ptr, name_len) };
        run(|x| {
            let at = x.put(name)?;
            x.call("ui_set_value", &[I32(handle), at, len(name.len()), F64(value)])?.i32()
        })
        .unwrap_or(0)
    }
    pub unsafe fn ui_add(
        handle: i32,
        parent_ptr: *const u8,
        parent_len: usize,
        what_ptr: *const u8,
        what_len: usize,
        name_ptr: *const u8,
        name_len: usize,
    ) -> i32 {
        // SAFETY: the safe wrappers pass live slices.
        let (parent, what, name) = unsafe { (bytes(parent_ptr, parent_len), bytes(what_ptr, what_len), bytes(name_ptr, name_len)) };
        run(|x| {
            let p = x.put(parent)?;
            let w = x.put(what)?;
            let n = x.put(name)?;
            x.call("ui_add", &[I32(handle), p, len(parent.len()), w, len(what.len()), n, len(name.len())])?.i32()
        })
        .unwrap_or(0)
    }
    pub unsafe fn ui_remove(handle: i32, name_ptr: *const u8, name_len: usize) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        let name = unsafe { bytes(name_ptr, name_len) };
        run(|x| {
            let at = x.put(name)?;
            x.call("ui_remove", &[I32(handle), at, len(name.len())])?.i32()
        })
        .unwrap_or(0)
    }
    pub unsafe fn ui_value(handle: i32, name_ptr: *const u8, name_len: usize) -> f64 {
        // SAFETY: the safe wrappers pass a live slice.
        let name = unsafe { bytes(name_ptr, name_len) };
        run(|x| {
            let at = x.put(name)?;
            x.call("ui_value", &[I32(handle), at, len(name.len())])?.f64()
        })
        .unwrap_or(f64::NAN)
    }
    pub unsafe fn ui_text(handle: i32, name_ptr: *const u8, name_len: usize, out: *mut u8, cap: usize) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        let name = unsafe { bytes(name_ptr, name_len) };
        let (full, data) = run(|x| {
            let at = x.put(name)?;
            let room = x.room(cap)?;
            let full = x.call("ui_text", &[I32(handle), at, len(name.len()), room, len(cap)])?.i32()?;
            let got = usize::try_from(full).unwrap_or(0).min(cap);
            Some((full, x.get(room, got)?))
        })
        .unwrap_or((-1, Vec::new()));
        // SAFETY: callers pass `cap` writable bytes at `out`.
        unsafe { bytes_out(out, cap, &data) }
        full
    }
    pub unsafe fn ui_show(handle: i32, name_ptr: *const u8, name_len: usize, visible: i32) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        let name = unsafe { bytes(name_ptr, name_len) };
        run(|x| {
            let at = x.put(name)?;
            x.call("ui_show", &[I32(handle), at, len(name.len()), I32(visible)])?.i32()
        })
        .unwrap_or(0)
    }
    pub unsafe fn sound_play(handle: i32) -> i32 {
        plain("sound_play", &[I32(handle)])
    }
    pub unsafe fn sound_stop(handle: i32) -> i32 {
        plain("sound_stop", &[I32(handle)])
    }
    pub unsafe fn sound_play_at(ptr: *const u8, n: usize, x: f32, y: f32, z: f32, volume: f32) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        text_call("sound_play_at", unsafe { bytes(ptr, n) }, &[F32(x), F32(y), F32(z), F32(volume)])
    }
    pub unsafe fn scene_unload(ptr: *const u8, n: usize) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        text_call("scene_unload", unsafe { bytes(ptr, n) }, &[])
    }
    pub unsafe fn object_of(reference: i32) -> i32 {
        plain("object_of", &[I32(reference)])
    }
    pub unsafe fn component_get(handle: i32, c_ptr: *const u8, c_len: usize, f_ptr: *const u8, f_len: usize, out: *mut f32) -> i32 {
        // SAFETY: the safe wrappers pass live slices.
        let (component, field) = unsafe { (bytes(c_ptr, c_len), bytes(f_ptr, f_len)) };
        let (found, data) = run(|x| {
            let c = x.put(component)?;
            let f = x.put(field)?;
            let (found, data) = with_out::<f32>("component_get", &[I32(handle), c, len(component.len()), f, len(field.len())], 3, x)?;
            Some((found.i32()?, data))
        })
        .unwrap_or((0, Vec::new()));
        // SAFETY: callers pass room for 3 f32.
        unsafe { numbers_out(out, &data) }
        found
    }
    pub unsafe fn component_set(handle: i32, c_ptr: *const u8, c_len: usize, f_ptr: *const u8, f_len: usize, value: *const f32) -> i32 {
        // SAFETY: the safe wrappers pass live slices and 3 readable f32.
        let (component, field, value) = unsafe { (bytes(c_ptr, c_len), bytes(f_ptr, f_len), numbers(value, 3)) };
        run(|x| {
            let c = x.put(component)?;
            let f = x.put(field)?;
            let v = x.put(&value)?;
            x.call("component_set", &[I32(handle), c, len(component.len()), f, len(field.len()), v])?.i32()
        })
        .unwrap_or(0)
    }
    pub unsafe fn transform_get_of(handle: i32, out: *mut f32) {
        let data = run(|x| with_out::<f32>("transform_get_of", &[I32(handle)], 9, x).map(|(_, d)| d)).unwrap_or_default();
        // SAFETY: callers pass room for 9 f32.
        unsafe { numbers_out(out, &data) }
    }
    pub unsafe fn transform_set_of(handle: i32, ptr: *const f32) {
        // SAFETY: callers pass 9 readable f32.
        let data = unsafe { numbers(ptr, 9) };
        run(|x| {
            let at = x.put(&data)?;
            x.call("transform_set_of", &[I32(handle), at])
        });
    }
    pub unsafe fn position_get_of(handle: i32, out: *mut f64) {
        let data = run(|x| with_out::<f64>("position_get_of", &[I32(handle)], 3, x).map(|(_, d)| d)).unwrap_or_default();
        // SAFETY: callers pass room for 3 f64.
        unsafe { numbers_out(out, &data) }
    }
    pub unsafe fn position_set_of(handle: i32, ptr: *const f64) {
        // SAFETY: callers pass 3 readable f64.
        let data = unsafe { numbers(ptr, 3) };
        run(|x| {
            let at = x.put(&data)?;
            x.call("position_set_of", &[I32(handle), at])
        });
    }
    pub unsafe fn spawn(ptr: *const u8, n: usize, x: f32, y: f32, z: f32) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        text_call("spawn", unsafe { bytes(ptr, n) }, &[F32(x), F32(y), F32(z)])
    }
    pub unsafe fn destroy(handle: i32) -> i32 {
        plain("destroy", &[I32(handle)])
    }
    pub unsafe fn send(handle: i32, ptr: *const u8, n: usize, value: f32) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        let name = unsafe { bytes(ptr, n) };
        run(|x| {
            let at = x.put(name)?;
            x.call("send", &[I32(handle), at, len(name.len()), F32(value)])?.i32()
        })
        .unwrap_or(0)
    }
    pub unsafe fn message_name(out: *mut u8, cap: usize) -> i32 {
        let (full, data) = run(|x| {
            let room = x.room(cap)?;
            let full = x.call("message_name", &[room, len(cap)])?.i32()?;
            let got = usize::try_from(full).unwrap_or(0).min(cap);
            Some((full, x.get(room, got)?))
        })
        .unwrap_or((0, Vec::new()));
        // SAFETY: callers pass `cap` writable bytes at `out`.
        unsafe { bytes_out(out, cap, &data) }
        full
    }
    pub unsafe fn pad_connected() -> i32 {
        plain("pad_connected", &[])
    }
    pub unsafe fn apply_impulse(handle: i32, x: f32, y: f32, z: f32) -> i32 {
        plain("apply_impulse", &[I32(handle), F32(x), F32(y), F32(z)])
    }
    pub unsafe fn character_walk(handle: i32, x: f32, y: f32, z: f32) -> i32 {
        plain("character_walk", &[I32(handle), F32(x), F32(y), F32(z)])
    }
    pub unsafe fn action(ptr: *const u8, n: usize) -> f32 {
        // SAFETY: the safe wrappers pass a live slice.
        let name = unsafe { bytes(ptr, n) };
        run(|x| {
            let at = x.put(name)?;
            x.call("action", &[at, len(name.len())])?.f32()
        })
        .unwrap_or(0.0)
    }
    pub unsafe fn character_jump(handle: i32, speed: f32) -> i32 {
        plain("character_jump", &[I32(handle), F32(speed)])
    }
    pub unsafe fn character_grounded(handle: i32) -> i32 {
        plain("character_grounded", &[I32(handle)])
    }
    pub unsafe fn set_velocity(handle: i32, x: f32, y: f32, z: f32) -> i32 {
        plain("set_velocity", &[I32(handle), F32(x), F32(y), F32(z)])
    }
    pub unsafe fn velocity_of(handle: i32, out: *mut f32) -> i32 {
        let (found, data) = run(|x| {
            let (found, data) = with_out::<f32>("velocity_of", &[I32(handle)], 3, x)?;
            Some((found.i32()?, data))
        })
        .unwrap_or((0, Vec::new()));
        // SAFETY: callers pass room for 3 f32.
        unsafe { numbers_out(out, &data) }
        found
    }
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn raycast(ox: f32, oy: f32, oz: f32, dx: f32, dy: f32, dz: f32, max: f32, out: *mut f32) -> i32 {
        let (handle, data) = run(|x| {
            let (handle, data) = with_out::<f32>("raycast", &[F32(ox), F32(oy), F32(oz), F32(dx), F32(dy), F32(dz), F32(max)], 4, x)?;
            Some((handle.i32()?, data))
        })
        .unwrap_or((0, Vec::new()));
        // SAFETY: callers pass room for 4 f32.
        unsafe { numbers_out(out, &data) }
        handle
    }
    pub unsafe fn pad_axis(index: i32) -> f32 {
        run(|x| x.call("pad_axis", &[I32(index)])?.f32()).unwrap_or(0.0)
    }
    pub unsafe fn pad_button(index: i32) -> i32 {
        plain("pad_button", &[I32(index)])
    }
    pub unsafe fn state_get(scope: i32, handle: i32, ptr: *const u8, n: usize, out: *mut f64) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        let name = unsafe { bytes(ptr, n) };
        let (set, data) = run(|x| {
            let at = x.put(name)?;
            let (set, data) = with_out::<f64>("state_get", &[I32(scope), I32(handle), at, len(name.len())], 1, x)?;
            Some((set.i32()?, data))
        })
        .unwrap_or((0, Vec::new()));
        // SAFETY: callers pass room for one f64.
        unsafe { numbers_out(out, &data) }
        set
    }
    pub unsafe fn state_set(scope: i32, handle: i32, ptr: *const u8, n: usize, value: f64) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        let name = unsafe { bytes(ptr, n) };
        run(|x| {
            let at = x.put(name)?;
            x.call("state_set", &[I32(scope), I32(handle), at, len(name.len()), F64(value)])?.i32()
        })
        .unwrap_or(0)
    }
    pub unsafe fn game_end(won: i32) -> i32 {
        plain("game_end", &[I32(won)])
    }
    pub unsafe fn input_map_set(handle: i32, ptr: *const u8, n: usize) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        // The host takes the object first, then the map's name.
        numbers_then_text("input_map_set", &[I32(handle)], unsafe { bytes(ptr, n) })
    }
    pub unsafe fn script_component(handle: i32, ptr: *const u8, n: usize) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        numbers_then_text("script_component", &[I32(handle)], unsafe { bytes(ptr, n) })
    }
    pub unsafe fn self_component() -> i32 {
        plain("self_component", &[])
    }
    pub unsafe fn event_subscribe(handle: i32, script: i32, ptr: *const u8, n: usize) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        numbers_then_text("event_subscribe", &[I32(handle), I32(script)], unsafe { bytes(ptr, n) })
    }
    pub unsafe fn event_unsubscribe(handle: i32, script: i32, ptr: *const u8, n: usize) -> i32 {
        // SAFETY: the safe wrappers pass a live slice.
        numbers_then_text("event_unsubscribe", &[I32(handle), I32(script)], unsafe { bytes(ptr, n) })
    }
    pub unsafe fn event_source() -> i32 {
        plain("event_source", &[])
    }
    pub unsafe fn event_source_component() -> i32 {
        plain("event_source_component", &[])
    }

    /// Texts go into the exchange memory first, and the values list where they went.
    pub fn event_send(handle: i32, name: &[u8], values: &[super::EventSlot<'_>]) -> i32 {
        event_send_to(handle, None, name, values)
    }

    /// [`event_send`], to one script of the object when `script` is given.
    pub fn event_send_to(handle: i32, script: Option<i32>, name: &[u8], values: &[super::EventSlot<'_>]) -> i32 {
        run(|x| {
            let at = x.put(name)?;
            let mut places = Vec::new();
            for value in values {
                if let super::EventSlot::Text(text) = value {
                    let crate::native::Value::I32(place) = x.put(text.as_bytes())? else { return None };
                    places.push(u32::try_from(place).ok()?);
                }
            }
            let mut places = places.into_iter();
            let listed = super::encode_event(values, |_| places.next())?;
            let list = x.put(&listed)?;
            match script {
                Some(script) => x.call("event_send_to", &[I32(handle), I32(script), at, len(name.len()), list, len(values.len())])?.i32(),
                None => x.call("event_send", &[I32(handle), at, len(name.len()), list, len(values.len())])?.i32(),
            }
        })
        .unwrap_or(0)
    }
    pub unsafe fn event_number(index: i32) -> f32 {
        run(|x| x.call("event_number", &[I32(index)])?.f32()).unwrap_or(0.0)
    }
    pub unsafe fn event_whole(index: i32) -> i32 {
        plain("event_whole", &[I32(index)])
    }
    pub unsafe fn event_flag(index: i32) -> i32 {
        plain("event_flag", &[I32(index)])
    }
    pub unsafe fn event_vector(index: i32, part: i32) -> f32 {
        run(|x| x.call("event_vector", &[I32(index), I32(part)])?.f32()).unwrap_or(0.0)
    }
    pub unsafe fn event_object(index: i32) -> i32 {
        plain("event_object", &[I32(index)])
    }
    pub unsafe fn event_text(index: i32, out: *mut u8, cap: usize) -> i32 {
        let (full, data) = run(|x| {
            let room = x.room(cap)?;
            let full = x.call("event_text", &[I32(index), room, len(cap)])?.i32()?;
            let got = usize::try_from(full).unwrap_or(0).min(cap);
            Some((full, x.get(room, got)?))
        })
        .unwrap_or((0, Vec::new()));
        if !out.is_null() {
            // SAFETY: the safe wrapper passes `cap` writable bytes at `out`; `data` is at most `cap` long.
            unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), out, data.len()) };
        }
        full
    }
}

/// The bridge to the host while a native script runs: set by [`with_bridge`] around each call.
#[cfg(not(target_arch = "wasm32"))]
mod bridge {
    use std::cell::Cell;

    use crate::native::{Bridge, Value};

    thread_local! {
        static CURRENT: Cell<Option<*mut (dyn Bridge + 'static)>> = const { Cell::new(None) };
    }

    /// Runs `f` with `bridge` as the host of every import it makes on this thread.
    pub fn with_bridge<R>(bridge: &mut dyn Bridge, f: impl FnOnce() -> R) -> R {
        // SAFETY: the pointer is only used by `run` on this thread while `f` runs, and `f` cannot
        // outlive `bridge`; the guard below takes it away again when `f` returns or unwinds, so the
        // erased lifetime is never relied on beyond `bridge`'s.
        let erased: *mut (dyn Bridge + 'static) = unsafe { std::mem::transmute::<*mut (dyn Bridge + '_), *mut (dyn Bridge + 'static)>(bridge) };
        struct Restore(Option<*mut (dyn Bridge + 'static)>);
        impl Drop for Restore {
            fn drop(&mut self) {
                CURRENT.with(|c| c.set(self.0));
            }
        }
        let _restore = Restore(CURRENT.with(|c| c.replace(Some(erased))));
        f()
    }

    /// One import's use of the exchange memory: each value put or room taken comes after the last.
    pub struct Exchange<'a> {
        bridge: &'a mut dyn Bridge,
        next: u32,
    }

    impl Exchange<'_> {
        pub fn put(&mut self, bytes: &[u8]) -> Option<Value> {
            let at = self.room(bytes.len())?;
            let Value::I32(offset) = at else { return None };
            self.bridge.write(u32::try_from(offset).ok()?, bytes).then_some(at)
        }

        pub fn room(&mut self, n: usize) -> Option<Value> {
            let at = self.next;
            // Values start on 8-byte boundaries, as the host's numbers expect.
            let end = at.checked_add(u32::try_from(n).ok()?)?;
            self.next = end.checked_add(7)? & !7;
            Some(Value::I32(i32::try_from(at).ok()?))
        }

        pub fn get(&mut self, at: Value, n: usize) -> Option<Vec<u8>> {
            let Value::I32(offset) = at else { return None };
            let mut out = vec![0; n];
            self.bridge.read(u32::try_from(offset).ok()?, &mut out).then_some(out)
        }

        pub fn call(&mut self, name: &'static str, args: &[Value]) -> Option<Value> {
            self.bridge.call(name, args)
        }
    }

    /// Runs `f` against the current bridge; `None` without one, or when a call failed.
    pub fn run<R>(f: impl FnOnce(&mut Exchange<'_>) -> Option<R>) -> Option<R> {
        let current = CURRENT.with(Cell::get)?;
        // SAFETY: `with_bridge` set the pointer from a live `&mut dyn Bridge` on this thread and
        // clears it before that borrow ends; host calls do not re-enter the script, so no other
        // reference to the bridge is in use while this one is.
        let bridge = unsafe { &mut *current };
        f(&mut Exchange { bridge, next: 0 })
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use bridge::with_bridge;

pub(crate) fn log(text: &str) {
    crate::batch::sync();
    // SAFETY: the pointer and length describe `text`, which outlives the call; the host only reads.
    unsafe { raw::log(text.as_ptr(), text.len()) }
}

pub(crate) fn transform_get() -> [f32; 9] {
    if let Some(values) = crate::batch::transform() {
        return values;
    }
    crate::batch::sync();
    let mut values = [0.0f32; 9];
    // SAFETY: `values` is 9 writable f32, exactly what the host writes.
    unsafe { raw::transform_get(values.as_mut_ptr()) }
    values
}

pub(crate) fn transform_set(values: &[f32; 9]) {
    if crate::batch::set_transform(values) {
        return;
    }
    crate::batch::sync();
    // SAFETY: `values` is 9 readable f32, exactly what the host reads.
    unsafe { raw::transform_set(values.as_ptr()) }
}

pub(crate) fn property(name: &str) -> f32 {
    crate::batch::property(name, || {
        crate::batch::sync();
        // SAFETY: the pointer and length describe `name`; the host only reads.
        unsafe { raw::property(name.as_ptr(), name.len()) }
    })
}

pub(crate) fn property_whole(name: &str) -> i32 {
    crate::batch::sync();
    // SAFETY: the pointer and length describe `name`; the host only reads.
    unsafe { raw::property_whole(name.as_ptr(), name.len()) }
}

pub(crate) fn property_vector(name: &str) -> Option<[f32; 3]> {
    crate::batch::sync();
    let mut out = [0.0f32; 3];
    // SAFETY: the pointer and length describe `name`; `out` is 3 writable f32, what the host writes.
    let found = unsafe { raw::property_vector(name.as_ptr(), name.len(), out.as_mut_ptr()) };
    (found != 0).then_some(out)
}

pub(crate) fn property_text(name: &str) -> String {
    crate::batch::sync();
    // SAFETY: a null pointer with no room only asks for the length.
    let len = unsafe { raw::property_text(name.as_ptr(), name.len(), std::ptr::null_mut(), 0) };
    let mut bytes = vec![0u8; usize::try_from(len).unwrap_or(0)];
    // SAFETY: `bytes` has exactly `len` writable bytes, at most what the host writes.
    unsafe { raw::property_text(name.as_ptr(), name.len(), bytes.as_mut_ptr(), bytes.len()) };
    String::from_utf8_lossy(&bytes).into_owned()
}

pub(crate) fn property_object(name: &str) -> i32 {
    crate::batch::sync();
    // SAFETY: the pointer and length describe `name`; the host only reads.
    unsafe { raw::property_object(name.as_ptr(), name.len()) }
}

pub(crate) fn property_set(name: &str, [x, y, z]: [f32; 3]) -> bool {
    crate::batch::sync();
    crate::batch::forget_properties();
    // SAFETY: the pointer and length describe `name`; the host only reads.
    unsafe { raw::property_set(name.as_ptr(), name.len(), x, y, z) != 0 }
}

pub(crate) fn property_set_text(name: &str, text: &str) -> bool {
    crate::batch::sync();
    crate::batch::forget_properties();
    // SAFETY: both pointers and lengths describe `name` and `text`; the host only reads.
    unsafe { raw::property_set_text(name.as_ptr(), name.len(), text.as_ptr(), text.len()) != 0 }
}

pub(crate) fn time() -> f64 {
    crate::batch::time(|| {
        crate::batch::sync();
        // SAFETY: no arguments; the host returns a plain number.
        unsafe { raw::time() }
    })
}

pub(crate) fn key_down(name: &str) -> bool {
    crate::batch::sync();
    // SAFETY: the pointer and length describe `name`; the host only reads.
    unsafe { raw::key_down(name.as_ptr(), name.len()) != 0 }
}

pub(crate) fn pointer() -> Option<[f32; 2]> {
    crate::batch::sync();
    let mut xy = [0.0f32; 2];
    // SAFETY: `xy` is 2 writable f32, exactly what the host writes.
    let inside = unsafe { raw::pointer(xy.as_mut_ptr()) };
    (inside != 0).then_some(xy)
}

pub(crate) fn pointer_over_interface() -> bool {
    crate::batch::sync();
    // SAFETY: no arguments; a plain number out.
    unsafe { raw::pointer_over_interface() != 0 }
}

pub(crate) fn mouse_down(button: i32) -> bool {
    crate::batch::sync();
    // SAFETY: a plain number in and out.
    unsafe { raw::mouse_down(button) != 0 }
}

pub(crate) fn self_handle() -> i32 {
    if let Some(handle) = crate::batch::own_handle() {
        return handle;
    }
    crate::batch::sync();
    // SAFETY: no arguments; the host returns a plain number.
    unsafe { raw::self_handle() }
}

pub(crate) fn find(name: &str) -> i32 {
    crate::batch::sync();
    // SAFETY: the pointer and length describe `name`; the host only reads.
    unsafe { raw::find(name.as_ptr(), name.len()) }
}

pub(crate) fn find_with(kind: &str, index: i32) -> i32 {
    crate::batch::sync();
    // SAFETY: the pointer and length describe `kind`; the host only reads.
    unsafe { raw::find_with(kind.as_ptr(), kind.len(), index) }
}

pub(crate) fn count_with(kind: &str) -> i32 {
    crate::batch::sync();
    // SAFETY: the pointer and length describe `kind`; the host only reads.
    unsafe { raw::count_with(kind.as_ptr(), kind.len()) }
}

pub(crate) fn particles_play(handle: i32) -> bool {
    crate::batch::sync();
    // SAFETY: a plain number in and out.
    unsafe { raw::particles_play(handle) != 0 }
}

pub(crate) fn particles_stop(handle: i32) -> bool {
    crate::batch::sync();
    // SAFETY: a plain number in and out.
    unsafe { raw::particles_stop(handle) != 0 }
}

pub(crate) fn particles_burst(handle: i32, count: u32) -> bool {
    crate::batch::sync();
    // SAFETY: plain numbers in and out.
    unsafe { raw::particles_burst(handle, i32::try_from(count).unwrap_or(i32::MAX)) != 0 }
}

pub(crate) fn ui_set_text(handle: i32, element: &str, text: &str) -> bool {
    crate::batch::sync();
    // SAFETY: the pointers and lengths describe `element` and `text`; the host only reads.
    unsafe { raw::ui_set_text(handle, element.as_ptr(), element.len(), text.as_ptr(), text.len()) != 0 }
}

pub(crate) fn ui_set_value(handle: i32, element: &str, value: f64) -> bool {
    crate::batch::sync();
    // SAFETY: the pointer and length describe `element`; the host only reads.
    unsafe { raw::ui_set_value(handle, element.as_ptr(), element.len(), value) != 0 }
}

pub(crate) fn ui_add(handle: i32, parent: &str, what: &str, name: &str) -> bool {
    crate::batch::sync();
    // SAFETY: the pointers and lengths describe the three texts; the host only reads.
    unsafe { raw::ui_add(handle, parent.as_ptr(), parent.len(), what.as_ptr(), what.len(), name.as_ptr(), name.len()) != 0 }
}

pub(crate) fn ui_remove(handle: i32, name: &str) -> bool {
    crate::batch::sync();
    // SAFETY: the pointer and length describe `name`; the host only reads.
    unsafe { raw::ui_remove(handle, name.as_ptr(), name.len()) != 0 }
}

pub(crate) fn ui_value(handle: i32, element: &str) -> Option<f64> {
    crate::batch::sync();
    // SAFETY: the pointer and length describe `element`; the host only reads.
    let value = unsafe { raw::ui_value(handle, element.as_ptr(), element.len()) };
    (!value.is_nan()).then_some(value)
}

pub(crate) fn ui_text(handle: i32, element: &str) -> Option<String> {
    crate::batch::sync();
    let mut room = vec![0u8; 256];
    // SAFETY: `room` has `room.len()` writable bytes; the name is read only.
    let full = unsafe { raw::ui_text(handle, element.as_ptr(), element.len(), room.as_mut_ptr(), room.len()) };
    let full = usize::try_from(full).ok()?;
    if full > room.len() {
        room = vec![0u8; full];
        // SAFETY: as above, with room for the whole text.
        unsafe { raw::ui_text(handle, element.as_ptr(), element.len(), room.as_mut_ptr(), room.len()) };
    }
    room.truncate(full);
    String::from_utf8(room).ok()
}

pub(crate) fn ui_show(handle: i32, element: &str, visible: bool) -> bool {
    crate::batch::sync();
    // SAFETY: the pointer and length describe `element`; the host only reads.
    unsafe { raw::ui_show(handle, element.as_ptr(), element.len(), i32::from(visible)) != 0 }
}

pub(crate) fn sound_play(handle: i32) -> bool {
    crate::batch::sync();
    // SAFETY: a plain number in and out.
    unsafe { raw::sound_play(handle) != 0 }
}

pub(crate) fn sound_stop(handle: i32) -> bool {
    crate::batch::sync();
    // SAFETY: a plain number in and out.
    unsafe { raw::sound_stop(handle) != 0 }
}

pub(crate) fn sound_play_at(path: &str, position: [f32; 3], volume: f32) -> bool {
    crate::batch::sync();
    let [x, y, z] = position;
    // SAFETY: the pointer and length describe `path`; the host only reads.
    unsafe { raw::sound_play_at(path.as_ptr(), path.len(), x, y, z, volume) != 0 }
}

pub(crate) fn scene_load(path: &str, additive: bool) -> bool {
    crate::batch::sync();
    // SAFETY: the pointer and length describe `path`; the host only reads.
    unsafe { raw::scene_load(path.as_ptr(), path.len(), i32::from(additive)) != 0 }
}

pub(crate) fn scene_unload(path: &str) -> bool {
    crate::batch::sync();
    // SAFETY: the pointer and length describe `path`; the host only reads.
    unsafe { raw::scene_unload(path.as_ptr(), path.len()) != 0 }
}

pub(crate) fn object_ref(handle: i32) -> i32 {
    crate::batch::sync();
    // SAFETY: plain numbers in and out.
    unsafe { raw::object_ref(handle) }
}

pub(crate) fn object_of(reference: i32) -> i32 {
    crate::batch::kept_object(reference, || {
        crate::batch::sync();
        // SAFETY: plain numbers in and out.
        unsafe { raw::object_of(reference) }
    })
}

pub(crate) fn name_of(handle: i32) -> String {
    crate::batch::sync();
    // SAFETY: a null pointer with no room only asks for the length.
    let len = unsafe { raw::name_of(handle, std::ptr::null_mut(), 0) };
    let mut bytes = vec![0u8; usize::try_from(len).unwrap_or(0)];
    // SAFETY: `bytes` has exactly `len` writable bytes, at most what the host writes.
    unsafe { raw::name_of(handle, bytes.as_mut_ptr(), bytes.len()) };
    String::from_utf8_lossy(&bytes).into_owned()
}

pub(crate) fn component_get(handle: i32, component: &str, field: &str) -> Option<[f32; 3]> {
    crate::batch::sync();
    let mut out = [0.0f32; 3];
    // SAFETY: the pointers and lengths describe `component` and `field`, which the host only reads;
    // `out` is 3 writable f32, exactly what the host writes.
    let found = unsafe { raw::component_get(handle, component.as_ptr(), component.len(), field.as_ptr(), field.len(), out.as_mut_ptr()) };
    (found != 0).then_some(out)
}

pub(crate) fn component_set(handle: i32, component: &str, field: &str, value: [f32; 3]) -> bool {
    crate::batch::sync();
    // SAFETY: the pointers and lengths describe `component`, `field` and the 3 f32 of `value`; the
    // host only reads.
    unsafe { raw::component_set(handle, component.as_ptr(), component.len(), field.as_ptr(), field.len(), value.as_ptr()) != 0 }
}

pub(crate) fn transform_get_of(handle: i32) -> [f32; 9] {
    crate::batch::other_transform(handle, || {
        crate::batch::sync();
        let mut values = [0.0f32; 9];
        // SAFETY: `values` is 9 writable f32, exactly what the host writes.
        unsafe { raw::transform_get_of(handle, values.as_mut_ptr()) }
        values
    })
}

pub(crate) fn transform_set_of(handle: i32, values: &[f32; 9]) {
    crate::batch::forget_others();
    crate::batch::sync();
    // SAFETY: `values` is 9 readable f32, exactly what the host reads.
    unsafe { raw::transform_set_of(handle, values.as_ptr()) }
}

pub(crate) fn spawn(prefab: &str, position: [f32; 3]) -> bool {
    crate::batch::sync();
    // SAFETY: the pointer and length describe `prefab`; the host only reads.
    unsafe { raw::spawn(prefab.as_ptr(), prefab.len(), position[0], position[1], position[2]) != 0 }
}

pub(crate) fn destroy(handle: i32) -> bool {
    crate::batch::sync();
    // SAFETY: a plain number in and out.
    unsafe { raw::destroy(handle) != 0 }
}

pub(crate) fn send(handle: i32, name: &str, value: f32) -> bool {
    crate::batch::sync();
    // SAFETY: the pointer and length describe `name`; the host only reads.
    unsafe { raw::send(handle, name.as_ptr(), name.len(), value) != 0 }
}

/// The name of the message being delivered, or an empty string outside `bayleaf_message`.
pub(crate) fn message_name() -> String {
    crate::batch::sync();
    let mut buffer = [0u8; 64];
    // SAFETY: `buffer` is 64 writable bytes and the host writes at most `cap` of them.
    let len = unsafe { raw::message_name(buffer.as_mut_ptr(), buffer.len()) };
    let len = usize::try_from(len).unwrap_or(0).min(buffer.len());
    String::from_utf8_lossy(&buffer[..len]).into_owned()
}

pub(crate) fn pad_connected() -> bool {
    crate::batch::sync();
    // SAFETY: no arguments; the host returns a plain number.
    unsafe { raw::pad_connected() != 0 }
}

pub(crate) fn pad_axis(index: i32) -> f32 {
    crate::batch::sync();
    // SAFETY: a plain number in and out.
    unsafe { raw::pad_axis(index) }
}

pub(crate) fn pad_button(index: i32) -> bool {
    crate::batch::sync();
    // SAFETY: a plain number in and out.
    unsafe { raw::pad_button(index) != 0 }
}

/// The handle of the first object a ray hits (0 for none), its distance and the hit point.
pub(crate) fn raycast(origin: [f32; 3], direction: [f32; 3], max: f32) -> (i32, [f32; 4]) {
    crate::batch::sync();
    let mut out = [0.0f32; 4];
    let [ox, oy, oz] = origin;
    let [dx, dy, dz] = direction;
    // SAFETY: `out` is 4 writable f32, exactly what the host writes.
    let handle = unsafe { raw::raycast(ox, oy, oz, dx, dy, dz, max, out.as_mut_ptr()) };
    (handle, out)
}

pub(crate) fn apply_impulse(handle: i32, v: [f32; 3]) -> bool {
    crate::batch::sync();
    // SAFETY: plain numbers in and out.
    unsafe { raw::apply_impulse(handle, v[0], v[1], v[2]) != 0 }
}

pub(crate) fn set_velocity(handle: i32, v: [f32; 3]) -> bool {
    crate::batch::sync();
    // SAFETY: plain numbers in and out.
    unsafe { raw::set_velocity(handle, v[0], v[1], v[2]) != 0 }
}

pub(crate) fn velocity_of(handle: i32) -> Option<[f32; 3]> {
    crate::batch::sync();
    let mut out = [0.0f32; 3];
    // SAFETY: `out` is 3 writable f32, exactly what the host writes.
    let found = unsafe { raw::velocity_of(handle, out.as_mut_ptr()) };
    (found != 0).then_some(out)
}

pub(crate) fn position_get_of(handle: i32) -> [f64; 3] {
    crate::batch::sync();
    let mut values = [0.0f64; 3];
    // SAFETY: `values` is 3 writable f64, exactly what the host writes.
    unsafe { raw::position_get_of(handle, values.as_mut_ptr()) }
    values
}

pub(crate) fn position_set_of(handle: i32, values: &[f64; 3]) {
    crate::batch::forget_others();
    crate::batch::sync();
    // SAFETY: `values` is 3 readable f64, exactly what the host reads.
    unsafe { raw::position_set_of(handle, values.as_ptr()) }
}

pub(crate) fn character_walk(handle: i32, v: [f32; 3]) -> bool {
    crate::batch::sync();
    // SAFETY: plain numbers in and out.
    unsafe { raw::character_walk(handle, v[0], v[1], v[2]) != 0 }
}

pub(crate) fn character_jump(handle: i32, speed: f32) -> bool {
    crate::batch::sync();
    // SAFETY: plain numbers in and out.
    unsafe { raw::character_jump(handle, speed) != 0 }
}

pub(crate) fn character_grounded(handle: i32) -> bool {
    crate::batch::sync();
    // SAFETY: a plain number in and out.
    unsafe { raw::character_grounded(handle) != 0 }
}

pub(crate) fn action(name: &str) -> f32 {
    crate::batch::sync();
    // SAFETY: the pointer and length describe `name`; the host only reads.
    unsafe { raw::action(name.as_ptr(), name.len()) }
}

/// A value of the game's state (scope 0), a player's (scope 1; handle 0 for the first player) or a
/// saved one (scope 2).
pub(crate) fn state_get(scope: i32, handle: i32, name: &str) -> Option<f64> {
    crate::batch::sync();
    let mut out = 0.0f64;
    // SAFETY: the pointer and length describe `name`, which the host only reads; `out` is one
    // writable f64 that outlives the call.
    let set = unsafe { raw::state_get(scope, handle, name.as_ptr(), name.len(), &mut out) != 0 };
    set.then_some(out)
}

pub(crate) fn state_set(scope: i32, handle: i32, name: &str, value: f64) -> bool {
    crate::batch::sync();
    // SAFETY: the pointer and length describe `name`; the host only reads.
    unsafe { raw::state_set(scope, handle, name.as_ptr(), name.len(), value) != 0 }
}

/// Makes a map active: the project's Input Map (handle 0) or an object's own.
pub(crate) fn input_map_set(handle: i32, map: &str) -> bool {
    crate::batch::sync();
    // SAFETY: the pointer and length describe `map`; the host only reads.
    unsafe { raw::input_map_set(handle, map.as_ptr(), map.len()) != 0 }
}

pub(crate) fn game_end(won: bool) -> bool {
    crate::batch::sync();
    // SAFETY: a plain number in and out.
    unsafe { raw::game_end(i32::from(won)) != 0 }
}

/// One value of an event as `event_send` lists it.
pub(crate) enum EventSlot<'a> {
    Number(f32),
    Whole(i32),
    Flag(bool),
    Vector([f32; 3]),
    Text(&'a str),
    Object(i32),
}

/// The list `event_send` reads: per value a kind and three words; `text_at` gives where each
/// text is in the memory the host reads.
fn encode_event(values: &[EventSlot<'_>], mut text_at: impl FnMut(&str) -> Option<u32>) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(values.len() * 16);
    for value in values {
        let words: [u32; 4] = match value {
            EventSlot::Number(v) => [0, v.to_bits(), 0, 0],
            EventSlot::Whole(v) => [1, *v as u32, 0, 0],
            EventSlot::Flag(v) => [2, u32::from(*v), 0, 0],
            EventSlot::Vector([x, y, z]) => [3, x.to_bits(), y.to_bits(), z.to_bits()],
            EventSlot::Text(text) => [4, text_at(text)?, u32::try_from(text.len()).ok()?, 0],
            EventSlot::Object(handle) => [5, *handle as u32, 0, 0],
        };
        words.iter().for_each(|w| out.extend_from_slice(&w.to_le_bytes()));
    }
    Some(out)
}

pub(crate) fn event_send(handle: i32, name: &str, values: &[EventSlot<'_>]) -> bool {
    crate::batch::sync();
    #[cfg(target_arch = "wasm32")]
    {
        // In WebAssembly a text's place is its address in the script's memory.
        let Some(listed) = encode_event(values, |text| u32::try_from(text.as_ptr() as usize).ok()) else { return false };
        // SAFETY: the pointers and lengths describe `name` and `listed`, and each text in the list
        // lives for this call; the host only reads.
        unsafe { raw::event_send(handle, name.as_ptr(), name.len(), listed.as_ptr(), values.len() as i32) != 0 }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        raw::event_send(handle, name.as_bytes(), values) != 0
    }
}

pub(crate) fn event_number(index: i32) -> f32 {
    crate::batch::sync();
    // SAFETY: a plain number in and out.
    unsafe { raw::event_number(index) }
}

pub(crate) fn event_whole(index: i32) -> i32 {
    crate::batch::sync();
    // SAFETY: a plain number in and out.
    unsafe { raw::event_whole(index) }
}

pub(crate) fn event_flag(index: i32) -> bool {
    crate::batch::sync();
    // SAFETY: a plain number in and out.
    unsafe { raw::event_flag(index) != 0 }
}

pub(crate) fn event_vector(index: i32) -> [f32; 3] {
    crate::batch::sync();
    // SAFETY: plain numbers in and out.
    [0, 1, 2].map(|part| unsafe { raw::event_vector(index, part) })
}

pub(crate) fn event_object(index: i32) -> i32 {
    crate::batch::sync();
    // SAFETY: a plain number in and out.
    unsafe { raw::event_object(index) }
}

pub(crate) fn event_text(index: i32) -> String {
    crate::batch::sync();
    // SAFETY: a null pointer with no room only asks for the length.
    let len = unsafe { raw::event_text(index, std::ptr::null_mut(), 0) };
    let mut bytes = vec![0u8; usize::try_from(len).unwrap_or(0)];
    // SAFETY: `bytes` has exactly `len` writable bytes, at most what the host writes.
    unsafe { raw::event_text(index, bytes.as_mut_ptr(), bytes.len()) };
    String::from_utf8_lossy(&bytes).into_owned()
}

pub(crate) fn script_component(handle: i32, name: &str) -> i32 {
    crate::batch::sync();
    // SAFETY: the pointer and length describe `name`; the host only reads.
    unsafe { raw::script_component(handle, name.as_ptr(), name.len()) }
}

pub(crate) fn self_component() -> i32 {
    crate::batch::sync();
    // SAFETY: no arguments; a plain number out.
    unsafe { raw::self_component() }
}

pub(crate) fn event_send_to(handle: i32, script: i32, name: &str, values: &[EventSlot<'_>]) -> bool {
    crate::batch::sync();
    #[cfg(target_arch = "wasm32")]
    {
        let Some(listed) = encode_event(values, |text| u32::try_from(text.as_ptr() as usize).ok()) else { return false };
        // SAFETY: the pointers and lengths describe `name` and `listed`, and each text in the list
        // lives for this call; the host only reads.
        unsafe { raw::event_send_to(handle, script, name.as_ptr(), name.len(), listed.as_ptr(), values.len() as i32) != 0 }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        raw::event_send_to(handle, Some(script), name.as_bytes(), values) != 0
    }
}

pub(crate) fn event_subscribe(handle: i32, script: i32, name: &str, add: bool) -> bool {
    crate::batch::sync();
    // SAFETY: the pointer and length describe `name`; the host only reads.
    unsafe {
        if add {
            raw::event_subscribe(handle, script, name.as_ptr(), name.len()) != 0
        } else {
            raw::event_unsubscribe(handle, script, name.as_ptr(), name.len()) != 0
        }
    }
}

pub(crate) fn event_source() -> i32 {
    crate::batch::sync();
    // SAFETY: no arguments; a plain number out.
    unsafe { raw::event_source() }
}

pub(crate) fn event_source_component() -> i32 {
    crate::batch::sync();
    // SAFETY: no arguments; a plain number out.
    unsafe { raw::event_source_component() }
}

/// Makes `slot` the object the next calls are about, in a module that runs many objects: 0 when it
/// does not run, 1 when it does, 2 when its properties changed since last asked. A native script
/// asks it of its own object, slot 0.
pub(crate) fn slot_select(slot: i32) -> i32 {
    // SAFETY: a plain number in and out.
    unsafe { raw::slot_select(slot) }
}

/// Writes every object's frame entry of a module that runs many objects at `buffer` when it has room
/// for all; how many objects there are.
#[cfg(target_arch = "wasm32")]
pub(crate) fn frame_get(buffer: &mut [crate::Entry]) -> i32 {
    let room = i32::try_from(buffer.len()).unwrap_or(0);
    // SAFETY: the host writes at most `room` entries of 64 bytes, which `buffer` holds; an entry is
    // 64 bytes laid out as the host writes them (`repr(C)`, checked at compile time), and any bytes
    // are valid numbers.
    unsafe { raw::frame_get(buffer.as_mut_ptr().cast::<u8>(), room) }
}

/// A number that stays the same while handles name the same objects (the world's structure).
#[cfg(target_arch = "wasm32")]
pub(crate) fn world_stamp() -> i64 {
    // SAFETY: no arguments; a plain number out.
    unsafe { raw::world_stamp() }
}

/// Hands the frame back: the host applies the transforms marked changed.
#[cfg(target_arch = "wasm32")]
pub(crate) fn frame_set(buffer: &[crate::Entry]) {
    let count = i32::try_from(buffer.len()).unwrap_or(0);
    // SAFETY: the host reads `count` entries of 64 bytes, which `buffer` holds.
    unsafe { raw::frame_set(buffer.as_ptr().cast::<u8>(), count) }
}
