//! The runtime and the game as one WebAssembly module: the engine and the game's code together
//! behind one call per step, with no call out to a host. `bench/runtime/wasm-host` runs it.
//!
//! The exports are the module's whole interface, so they carry `no_mangle`; nothing here is unsafe
//! code.

use std::cell::RefCell;

use crate::runtime::{Frame, Runtime};
use crate::{DT, game};

struct State {
    typed: Runtime,
    boxed: Runtime,
    time: f64,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn started(n: usize, typed: bool) -> Runtime {
    let mut runtime = Runtime::default();
    game::build(&mut runtime, n, typed);
    runtime.start();
    runtime.world.finish_step();
    runtime
}

/// Builds the scene twice: with typed scripts and with boxed ones.
#[unsafe(no_mangle)]
pub extern "C" fn setup(n: u32) {
    let n = n as usize;
    STATE.with(|s| *s.borrow_mut() = Some(State { typed: started(n, true), boxed: started(n, false), time: 0.0 }));
}

/// One step of the typed (`typed` not 0) or the boxed scripts; `whole` not 0 also works out the
/// world matrices of what moved and reads the moved list. Returns what the reader of that list took.
#[unsafe(no_mangle)]
pub extern "C" fn step(typed: u32, whole: u32) -> f64 {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let Some(s) = s.as_mut() else { return 0.0 };
        s.time += f64::from(DT);
        let frame = Frame { dt: DT, time: s.time };
        let runtime = if typed != 0 { &mut s.typed } else { &mut s.boxed };
        runtime.update(&frame);
        if whole != 0 {
            runtime.world.finish_step()
        } else {
            runtime.world.forget_moved();
            0.0
        }
    })
}

/// Where the typed runtime's transforms are in this module's memory (48 bytes each), so a host can
/// copy an engine's own in before a step and back out after it.
#[unsafe(no_mangle)]
pub extern "C" fn transforms_ptr() -> u32 {
    STATE.with(|s| s.borrow().as_ref().map_or(0, |s| s.typed.world.transforms_at() as u32))
}

/// [`crate::check`], for comparing with the native programs.
#[unsafe(no_mangle)]
pub extern "C" fn check(n: u32, typed: u32) -> f64 {
    crate::check(n as usize, typed != 0)
}
