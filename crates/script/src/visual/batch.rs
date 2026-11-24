//! A visual script compiled to run every object of its file in one instance (see "Modules that run
//! many objects" in `docs/script-abi.md`). The code of each event is the same as for one object; what
//! the script keeps per object (its variables, timers, loops' places, text variables) lives in a slot
//! of memory per object, loaded into the module's globals before that object's code runs and saved
//! after.

use std::fmt::Write as _;

/// The most a script keeps per object for it to run this way; a script that keeps more (many text
/// variables) runs one instance per object.
pub(super) const MAX_SLOT_BYTES: u32 = 4096;

/// Bytes each global takes in a slot (every one gets a word of eight, so slots stay aligned).
const GLOBAL_BYTES: u32 = 8;

/// One global the script keeps per object: its name and its WebAssembly type.
struct Kept {
    name: String,
    ty: String,
    /// The value it starts at, as the declaration has it.
    start: String,
}

/// The name, type and start of a global declared `(global $name (mut ty) (ty.const value))`.
fn kept(declaration: &str) -> Option<Kept> {
    let rest = declaration.trim().strip_prefix("(global ")?;
    let (name, rest) = rest.split_once(' ')?;
    let rest = rest.strip_prefix("(mut ")?;
    let (ty, rest) = rest.split_once(')')?;
    let start = rest.trim().strip_suffix(')')?.trim().to_owned();
    Some(Kept { name: name.to_owned(), ty: ty.to_owned(), start })
}

/// A text variable: its buffer, the global with its length, and where its starting text is kept.
pub(super) struct TextSlot {
    pub(super) buffer: u32,
    pub(super) length: String,
    pub(super) start_at: u32,
    pub(super) start_len: u32,
}

/// The slot layout of a script that runs many objects: what it keeps per object, and how big a slot is.
pub(super) struct Layout {
    globals: Vec<Kept>,
    texts: Vec<TextSlot>,
    size: u32,
}

impl Layout {
    /// The layout for these declarations (every global the script keeps per object) and text
    /// variables; `None` when a slot would be bigger than [`MAX_SLOT_BYTES`] or a declaration is not
    /// one it can keep.
    pub(super) fn of<'a>(declarations: impl Iterator<Item = &'a str>, texts: Vec<TextSlot>) -> Option<Self> {
        let globals: Vec<Kept> = declarations.map(kept).collect::<Option<_>>()?;
        let size = (globals.len() as u32 * GLOBAL_BYTES + texts.len() as u32 * super::TEXT_VARIABLE_BYTES).next_multiple_of(16);
        (size <= MAX_SLOT_BYTES).then_some(Self { globals, texts, size })
    }

    /// The module's functions and exports for running many objects: each slot's state at `base`
    /// onwards, the heap after the last slot; `init`, `start`, `update` and `message` are the code of
    /// those events (`message` is empty when the script takes no messages).
    pub(super) fn exports(&self, base: u32, init: Option<&str>, start: &str, update: &str, message: Option<&str>) -> String {
        let size = self.size;
        let mut out = String::new();
        let _ = writeln!(out, "  (global $slots (mut i32) (i32.const 0))");
        let _ = writeln!(out, "  (global $heap_start (mut i32) (i32.const {base}))");
        // Puts every kept global and text back as the script starts.
        let mut reset = String::new();
        for g in &self.globals {
            let _ = writeln!(reset, "    (global.set {} {})", g.name, g.start);
        }
        for t in &self.texts {
            let _ = writeln!(reset, "    (memory.copy (i32.const {}) (i32.const {}) (i32.const {}))", t.buffer, t.start_at, t.start_len);
        }
        let _ = writeln!(out, "  (func $slot_reset\n{reset}  )");
        // Saves the globals and texts into slot $k, and loads them from it.
        let (mut save, mut load) = (String::new(), String::new());
        for (i, g) in self.globals.iter().enumerate() {
            let offset = i as u32 * GLOBAL_BYTES;
            let _ = writeln!(save, "    ({}.store offset={offset} (local.get $p) (global.get {}))", g.ty, g.name);
            let _ = writeln!(load, "    (global.set {} ({}.load offset={offset} (local.get $p)))", g.name, g.ty);
        }
        let texts_at = self.globals.len() as u32 * GLOBAL_BYTES;
        for (i, t) in self.texts.iter().enumerate() {
            let offset = texts_at + i as u32 * super::TEXT_VARIABLE_BYTES;
            // The length global comes and goes with the others, so only the text's own bytes move.
            let _ = writeln!(save, "    (memory.copy (i32.add (local.get $p) (i32.const {offset})) (i32.const {}) (global.get {}))", t.buffer, t.length);
            let _ = writeln!(load, "    (memory.copy (i32.const {}) (i32.add (local.get $p) (i32.const {offset})) (global.get {}))", t.buffer, t.length);
        }
        let place = format!("(local.set $p (i32.add (i32.const {base}) (i32.mul (local.get $k) (i32.const {size}))))");
        let _ = writeln!(out, "  (func $slot_save (param $k i32) (local $p i32)\n    {place}\n{save}  )");
        let _ = writeln!(out, "  (func $slot_load (param $k i32) (local $p i32)\n    {place}\n{load}  )");
        // A new object: the slots grow (and the memory with them), and its slot starts as the script does.
        let _ = writeln!(
            out,
            "  (func (export \"bayleaf_attach\") (param $k i32) (local $end i32)
    (if (i32.ge_u (local.get $k) (global.get $slots))
      (then
        (global.set $slots (i32.add (local.get $k) (i32.const 1)))
        (global.set $heap_start (i32.add (i32.const {base}) (i32.mul (global.get $slots) (i32.const {size}))))
        (local.set $end (i32.add (global.get $heap_start) (i32.const 65536)))
        (if (i32.gt_u (local.get $end) (i32.shl (memory.size) (i32.const 16)))
          (then
            (if (i32.lt_s (memory.grow (i32.add (i32.shr_u (i32.sub (local.get $end) (i32.shl (memory.size) (i32.const 16))) (i32.const 16)) (i32.const 1))) (i32.const 0))
              (then unreachable))))))
    (call $slot_reset)
    (call $slot_save (local.get $k)))"
        );
        let _ = writeln!(out, "  (func (export \"bayleaf_detach\") (param $k i32))");
        let fresh = "    (global.set $heap (global.get $heap_start))\n";
        let mut slot_function = |name: &str, body: &str| {
            let _ = writeln!(
                out,
                "  (func (export \"{name}\") (param $k i32)\n    (call $slot_load (local.get $k))\n{fresh}{body}    (call $slot_save (local.get $k)))"
            );
        };
        if let Some(init) = init {
            slot_function("bayleaf_init_slot", init);
        }
        slot_function("bayleaf_start_slot", start);
        let _ = writeln!(
            out,
            "  (func (export \"bayleaf_update_all\") (param $dt f32) (local $k i32)
    (global.set $dt (local.get $dt))
    (block $all (loop $each
      (br_if $all (i32.ge_u (local.get $k) (global.get $slots)))
      (if (call $slot_select (local.get $k))
        (then
          (call $slot_load (local.get $k))
      {fresh}{update}          (call $slot_save (local.get $k))))
      (local.set $k (i32.add (local.get $k) (i32.const 1)))
      (br $each))))"
        );
        if let Some(message) = message {
            let _ = writeln!(
                out,
                "  (func (export \"bayleaf_message_slot\") (param $k i32) (param $value f32)\n    (call $slot_load (local.get $k))\n{fresh}    (global.set $message (local.get $value))\n{message}    (call $slot_save (local.get $k)))"
            );
        }
        out
    }
}
