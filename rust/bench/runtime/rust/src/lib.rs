//! The small runtime of bench/README.md ("The same runtime in C++ and Rust") and its game. The
//! program in `main.rs` times them natively; built for `wasm32-unknown-unknown`, the same code is
//! one WebAssembly module (`wasm.rs`) that `bench/runtime/wasm-host` times.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod game;
pub mod runtime;
#[cfg(target_arch = "wasm32")]
mod wasm;

/// The step every program times with: 60 a second.
pub const DT: f32 = 1.0 / 60.0;
/// Steps of the fixed run whose checksum every version must agree on.
pub const CHECK_STEPS: usize = 100;

/// The checksum after a fixed number of whole steps: equal across languages, styles and builds when
/// they do the same work.
pub fn check(n: usize, typed: bool) -> f64 {
    let mut runtime = runtime::Runtime::default();
    game::build(&mut runtime, n, typed);
    runtime.start();
    let mut time = 0.0f64;
    for _ in 0..CHECK_STEPS {
        time += f64::from(DT);
        runtime.update(&runtime::Frame { dt: DT, time });
        runtime.world.finish_step();
    }
    runtime.world.checksum()
}
