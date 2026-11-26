//! Times the small runtime of `runtime.rs` running the game of `game.rs` (bench/README.md, "The same
//! runtime in C++ and Rust"). Prints one JSON line per style (typed, virtual) and part (scripts:
//! every script's update; step: that, then world matrices of what moved and reading the moved list).
//!
//! Build: cargo build --release (in this folder). Run: runtime-rust <objects>

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::time::Instant;

use runtime_rust::runtime::{Frame, Runtime};
use runtime_rust::{DT, check, game};

const WARMUP: usize = 30;
const MIN_FRAMES: usize = 100;
const MIN_SECONDS: f64 = 3.0;
const MAX_FRAMES: usize = 20000;

struct Stats {
    frames: usize,
    mean: f64,
    median: f64,
    p5: f64,
    p95: f64,
    stdev: f64,
}

fn measure(mut step: impl FnMut()) -> Stats {
    for _ in 0..WARMUP {
        step();
    }
    let mut ms: Vec<f64> = Vec::new();
    let started = Instant::now();
    while ms.len() < MAX_FRAMES && (ms.len() < MIN_FRAMES || started.elapsed().as_secs_f64() < MIN_SECONDS) {
        let t = Instant::now();
        step();
        ms.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    ms.sort_by(f64::total_cmp);
    let mean = ms.iter().sum::<f64>() / ms.len() as f64;
    let var = ms.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / ms.len() as f64;
    let at = |q: f64| ms[((ms.len() as f64 - 1.0) * q).round() as usize];
    Stats { frames: ms.len(), mean, median: at(0.5), p5: at(0.05), p95: at(0.95), stdev: var.sqrt() }
}

fn run(n: usize, typed: bool, whole_step: bool) {
    let mut runtime = Runtime::default();
    game::build(&mut runtime, n, typed);
    runtime.start();
    runtime.world.finish_step();
    let (mut time, mut taken) = (0.0f64, 0.0f64);
    let s = measure(|| {
        time += f64::from(DT);
        runtime.update(&Frame { dt: DT, time });
        if whole_step {
            taken += runtime.world.finish_step();
        } else {
            runtime.world.forget_moved();
        }
    });
    std::hint::black_box(taken);
    println!(
        "{{\"mode\":\"rust-runtime-{}\",\"scripts\":\"rust\",\"part\":\"{}\",\"objects\":{},\"check\":{:.9e},\"world\":{},\"data\":{:.9e},\"stats\":{{\"frames\":{},\"meanMs\":{:.6},\"medianMs\":{:.6},\"p5Ms\":{:.6},\"p95Ms\":{:.6},\"stdevMs\":{:.6}}}}}",
        if typed { "typed" } else { "virtual" },
        if whole_step { "step" } else { "scripts" },
        n,
        check(n, typed),
        runtime.world.len(),
        runtime.world.data_sum(),
        s.frames,
        s.mean,
        s.median,
        s.p5,
        s.p95,
        s.stdev
    );
}

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(5000);
    for typed in [true, false] {
        for whole_step in [false, true] {
            run(n, typed, whole_step);
        }
    }
}
