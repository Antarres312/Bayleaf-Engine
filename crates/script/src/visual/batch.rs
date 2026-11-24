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
        // The slots from `base`, then the frame (an entry of 64 bytes per slot), then the heap.
        let _ = writeln!(out, "  (global $frame_at (mut i32) (i32.const {base}))");
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
        (global.set $frame_at (i32.add (i32.const {base}) (i32.mul (global.get $slots) (i32.const {size}))))
        (global.set $heap_start (i32.add (global.get $frame_at) (i32.mul (global.get $slots) (i32.const 64))))
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
        // Outside an update of all, properties and the time are read afresh.
        let forget = "    (call $properties_forget)\n    (global.set $time_ok (i32.const 0))\n";
        let mut slot_function = |name: &str, body: &str| {
            let _ = writeln!(
                out,
                "  (func (export \"{name}\") (param $k i32)\n    (call $slot_load (local.get $k))\n{forget}{fresh}{body}    (call $slot_save (local.get $k)))"
            );
        };
        if let Some(init) = init {
            slot_function("bayleaf_init_slot", init);
        }
        slot_function("bayleaf_start_slot", start);
        let _ = writeln!(
            out,
            "  (func (export \"bayleaf_update_all\") (param $dt f32) (local $k i32) (local $selected i32) (local $n i32)
    (global.set $dt (local.get $dt))
    (global.set $time_ok (i32.const 0))
    ;; Every object's transform from the host in one call; each object's reads and writes use it.
    (local.set $n (call $frame_get (global.get $frame_at) (global.get $slots)))
    (global.set $in_frame (i32.le_u (local.get $n) (global.get $slots)))
    (block $all (loop $each
      (br_if $all (i32.ge_u (local.get $k) (global.get $slots)))
      (local.set $selected (call $slot_select (local.get $k)))
      (if (local.get $selected)
        (then
          (call $slot_load (local.get $k))
          (global.set $frame_cur (i32.add (global.get $frame_at) (i32.mul (local.get $k) (i32.const 64))))
          ;; Its properties changed since last read: the host says so at select, or in the frame.
          (if (i32.or (i32.eq (local.get $selected) (i32.const 2))
                (i32.and (global.get $in_frame) (i32.ne (i32.and (i32.load offset=36 (global.get $frame_cur)) (i32.const 2)) (i32.const 0))))
            (then (call $properties_forget)))
      {fresh}{update}          (call $slot_save (local.get $k))))
      (local.set $k (i32.add (local.get $k) (i32.const 1)))
      (br $each)))
    ;; The changed transforms back in one call.
    (if (global.get $in_frame)
      (then (global.set $in_frame (i32.const 0)) (call $frame_set (global.get $frame_at) (local.get $n)))))"
        );
        if let Some(message) = message {
            let _ = writeln!(
                out,
                "  (func (export \"bayleaf_message_slot\") (param $k i32) (param $value f32)\n    (call $slot_load (local.get $k))\n{forget}{fresh}    (global.set $message (local.get $value))\n{message}    (call $slot_save (local.get $k)))"
            );
        }
        out
    }
}

/// A property the script reads with a constant name, whose value a module that runs many objects
/// keeps per object as last read: where its name is, and how it is read.
pub(super) struct Cached {
    at: u32,
    len: u32,
    kind: CacheKind,
}

#[derive(Clone, Copy, PartialEq)]
enum CacheKind {
    Number,
    Whole,
    Vector,
}

impl CacheKind {
    /// How the event code reads it, before the name's place.
    fn call(self) -> &'static str {
        match self {
            CacheKind::Number => "(call $property (i32.const ",
            CacheKind::Whole => "(call $property_whole (i32.const ",
            CacheKind::Vector => "(call $property_part (i32.const ",
        }
    }

    fn tag(self) -> &'static str {
        match self {
            CacheKind::Number => "f",
            CacheKind::Whole => "w",
            CacheKind::Vector => "v",
        }
    }
}

/// The properties `code` reads by constant names (`(call $property (i32.const AT) (i32.const LEN))`
/// and the like), each once.
pub(super) fn cached_properties(code: &str) -> Vec<Cached> {
    let mut found: Vec<Cached> = Vec::new();
    for kind in [CacheKind::Number, CacheKind::Whole, CacheKind::Vector] {
        let call = kind.call();
        let mut rest = code;
        while let Some(start) = rest.find(call) {
            let after = &rest[start + call.len()..];
            rest = after;
            let Some((at, after)) = after.split_once(") (i32.const ") else { continue };
            let Some((len, _)) = after.split_once(')') else { continue };
            let (Ok(at), Ok(len)) = (at.parse::<u32>(), len.parse::<u32>()) else { continue };
            if !found.iter().any(|c| c.kind == kind && c.at == at) {
                found.push(Cached { at, len, kind });
            }
        }
    }
    found
}

/// The globals that keep each cached property per object (part of what a slot keeps).
pub(super) fn cache_globals(cached: &[Cached]) -> String {
    let mut out = String::new();
    for c in cached {
        let tag = c.kind.tag();
        let _ = writeln!(out, "  (global $pc{tag}_{}_ok (mut i32) (i32.const 0))", c.at);
        match c.kind {
            CacheKind::Number => {
                let _ = writeln!(out, "  (global $pc{tag}_{} (mut f32) (f32.const 0))", c.at);
            }
            CacheKind::Whole => {
                let _ = writeln!(out, "  (global $pc{tag}_{} (mut i32) (i32.const 0))", c.at);
            }
            CacheKind::Vector => {
                for i in 0..3 {
                    let _ = writeln!(out, "  (global $pc{tag}_{}_{i} (mut f32) (f32.const 0))", c.at);
                }
            }
        }
    }
    out
}

/// The module, for running many objects: every read of this object's transform, position, the time
/// and the cached properties goes to the frame and the caches instead of the host, while an update of
/// all runs; then the helpers that do it.
pub(super) fn rewrite(module: &str, cached: &[Cached]) -> String {
    let mut out = module
        .replace("(call $transform_get ", "(call $frame_transform_get ")
        .replace("(call $transform_set ", "(call $frame_transform_set ")
        .replace("(call $position_get_of (call $self_handle) ", "(call $frame_position_get ")
        .replace("(call $position_set_of (call $self_handle) ", "(call $frame_position_set ")
        .replace("(call $time)", "(call $time_kept)")
        .replace("(call $property_set ", "(call $property_set_forgetting ")
        .replace("(call $property_set_text ", "(call $property_set_text_forgetting ");
    for c in cached {
        let from = format!("{}{}) (i32.const {})", c.kind.call(), c.at, c.len);
        let to = format!("(call $pc{}_{}", c.kind.tag(), c.at);
        // The read's own closing parenthesis stays: `(call $pcf_AT)`, `(call $pcv_AT (i32.const i))`.
        out = out.replace(&from, &to);
    }
    out
}

/// The helpers [`rewrite`] calls, and the reset of the cached properties.
pub(super) fn helpers(cached: &[Cached]) -> String {
    let mut out = String::from(
        r#"  (global $in_frame (mut i32) (i32.const 0))
  (global $frame_cur (mut i32) (i32.const 0))
  (global $time_ok (mut i32) (i32.const 0))
  (global $time_value (mut f64) (f64.const 0))
  (func $frame_changed
    (i32.store offset=36 (global.get $frame_cur) (i32.or (i32.load offset=36 (global.get $frame_cur)) (i32.const 4))))
  (func $frame_transform_get (param $out i32)
    (if (global.get $in_frame)
      (then (memory.copy (local.get $out) (global.get $frame_cur) (i32.const 36)))
      (else (call $transform_get (local.get $out)))))
  (func $frame_transform_set (param $ptr i32)
    (if (global.get $in_frame)
      (then (memory.copy (global.get $frame_cur) (local.get $ptr) (i32.const 36)) (call $frame_changed))
      (else (call $transform_set (local.get $ptr)))))
  (func $frame_position_get (param $out i32)
    (if (global.get $in_frame)
      (then (memory.copy (local.get $out) (i32.add (global.get $frame_cur) (i32.const 40)) (i32.const 24)))
      (else (call $position_get_of (call $self_handle) (local.get $out)))))
  (func $frame_position_set (param $ptr i32)
    (if (global.get $in_frame)
      (then
        (memory.copy (i32.add (global.get $frame_cur) (i32.const 40)) (local.get $ptr) (i32.const 24))
        (f32.store (global.get $frame_cur) (f32.demote_f64 (f64.load (local.get $ptr))))
        (f32.store offset=4 (global.get $frame_cur) (f32.demote_f64 (f64.load offset=8 (local.get $ptr))))
        (f32.store offset=8 (global.get $frame_cur) (f32.demote_f64 (f64.load offset=16 (local.get $ptr))))
        (call $frame_changed))
      (else (call $position_set_of (call $self_handle) (local.get $ptr)))))
  (func $time_kept (result f64)
    (if (i32.eqz (global.get $time_ok))
      (then (global.set $time_value (call $time)) (global.set $time_ok (i32.const 1))))
    (global.get $time_value))
  (func $property_set_forgetting (param $a i32) (param $l i32) (param $x f32) (param $y f32) (param $z f32) (result i32)
    (call $properties_forget)
    (call $property_set (local.get $a) (local.get $l) (local.get $x) (local.get $y) (local.get $z)))
  (func $property_set_text_forgetting (param $a i32) (param $l i32) (param $t i32) (param $tl i32) (result i32)
    (call $properties_forget)
    (call $property_set_text (local.get $a) (local.get $l) (local.get $t) (local.get $tl)))
"#,
    );
    let mut forget = String::new();
    for c in cached {
        let (tag, at, len) = (c.kind.tag(), c.at, c.len);
        let _ = writeln!(forget, "    (global.set $pc{tag}_{at}_ok (i32.const 0))");
        let fill = |out: &mut String, read: &str| {
            let _ = writeln!(out, "    (if (i32.eqz (global.get $pc{tag}_{at}_ok))\n      (then {read} (global.set $pc{tag}_{at}_ok (i32.const 1))))");
        };
        match c.kind {
            CacheKind::Number => {
                let _ = writeln!(out, "  (func $pc{tag}_{at} (result f32)");
                fill(&mut out, &format!("(global.set $pc{tag}_{at} (call $property (i32.const {at}) (i32.const {len})))"));
                let _ = writeln!(out, "    (global.get $pc{tag}_{at}))");
            }
            CacheKind::Whole => {
                let _ = writeln!(out, "  (func $pc{tag}_{at} (result i32)");
                fill(&mut out, &format!("(global.set $pc{tag}_{at} (call $property_whole (i32.const {at}) (i32.const {len})))"));
                let _ = writeln!(out, "    (global.get $pc{tag}_{at}))");
            }
            CacheKind::Vector => {
                let read = format!(
                    "(drop (call $property_vector (i32.const {at}) (i32.const {len}) (i32.const 192))) (global.set $pc{tag}_{at}_0 (f32.load (i32.const 192))) (global.set $pc{tag}_{at}_1 (f32.load (i32.const 196))) (global.set $pc{tag}_{at}_2 (f32.load (i32.const 200)))"
                );
                let _ = writeln!(out, "  (func $pc{tag}_{at} (param $i i32) (result f32)");
                fill(&mut out, &read);
                let _ = writeln!(
                    out,
                    "    (if (result f32) (i32.eqz (local.get $i)) (then (global.get $pc{tag}_{at}_0)) (else (if (result f32) (i32.eq (local.get $i) (i32.const 1)) (then (global.get $pc{tag}_{at}_1)) (else (global.get $pc{tag}_{at}_2))))))"
                );
            }
        }
    }
    let _ = writeln!(out, "  (func $properties_forget\n{forget}  )");
    out
}
