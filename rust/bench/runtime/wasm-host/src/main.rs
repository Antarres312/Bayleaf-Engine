//! Times the small runtime and its game as one WebAssembly module (bench/runtime/rust built for
//! `wasm32-unknown-unknown`): one call per step into one instance, under wasmtime (compiled to
//! machine code, as Bayleaf's compiled scripts) and wasmi (interpreted, as the editor runs scripts).
//! The `copy` cases also copy every transform into the module before the step and back out after
//! it, as an engine that keeps its transforms outside the module would have to.
//!
//! Usage: runtime-wasm-host <runtime_rust.wasm> <objects>

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::time::Instant;

/// Errors are only shown: their text.
type Result<T> = std::result::Result<T, String>;

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

const WARMUP: usize = 30;
const MIN_FRAMES: usize = 100;
const MIN_SECONDS: f64 = 3.0;
const MAX_FRAMES: usize = 20000;
/// Bytes of one transform in the module (3 f64, 3 f32, 3 f32).
const TRANSFORM: usize = 48;

struct Stats {
    frames: usize,
    mean: f64,
    median: f64,
    p5: f64,
    p95: f64,
    stdev: f64,
}

fn measure(mut step: impl FnMut() -> Result<()>) -> Result<Stats> {
    for _ in 0..WARMUP {
        step()?;
    }
    let mut ms: Vec<f64> = Vec::new();
    let started = Instant::now();
    while ms.len() < MAX_FRAMES && (ms.len() < MIN_FRAMES || started.elapsed().as_secs_f64() < MIN_SECONDS) {
        let t = Instant::now();
        step()?;
        ms.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    ms.sort_by(f64::total_cmp);
    let mean = ms.iter().sum::<f64>() / ms.len() as f64;
    let var = ms.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / ms.len() as f64;
    let at = |q: f64| ms[((ms.len() as f64 - 1.0) * q).round() as usize];
    Ok(Stats { frames: ms.len(), mean, median: at(0.5), p5: at(0.05), p95: at(0.95), stdev: var.sqrt() })
}

/// One timed case: which scripts, which part, and whether transforms are copied in and out.
struct Case {
    style: &'static str,
    typed: u32,
    whole: u32,
    copy: bool,
}

const CASES: [Case; 5] = [
    Case { style: "typed", typed: 1, whole: 0, copy: false },
    Case { style: "typed", typed: 1, whole: 1, copy: false },
    Case { style: "virtual", typed: 0, whole: 0, copy: false },
    Case { style: "virtual", typed: 0, whole: 1, copy: false },
    Case { style: "typed-copy", typed: 1, whole: 1, copy: true },
];

fn print(engine: &str, case: &Case, n: u32, check: f64, s: &Stats) {
    println!(
        "{{\"mode\":\"wasm-{engine}-{}\",\"scripts\":\"rust\",\"part\":\"{}\",\"objects\":{n},\"check\":{check:.9e},\"stats\":{{\"frames\":{},\"meanMs\":{:.6},\"medianMs\":{:.6},\"p5Ms\":{:.6},\"p95Ms\":{:.6},\"stdevMs\":{:.6}}}}}",
        case.style,
        if case.whole != 0 { "step" } else { "scripts" },
        s.frames,
        s.mean,
        s.median,
        s.p5,
        s.p95,
        s.stdev
    );
}

fn compiled(bytes: &[u8], n: u32) -> Result<()> {
    let engine = wasmtime::Engine::default();
    let module = wasmtime::Module::new(&engine, bytes).map_err(text)?;
    for case in &CASES {
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).map_err(text)?;
        instance.get_typed_func::<u32, ()>(&mut store, "setup").map_err(text)?.call(&mut store, n).map_err(text)?;
        let step = instance.get_typed_func::<(u32, u32), f64>(&mut store, "step").map_err(text)?;
        let memory = instance.get_memory(&mut store, "memory").ok_or("the module exports no memory")?;
        let at = instance.get_typed_func::<(), u32>(&mut store, "transforms_ptr").map_err(text)?.call(&mut store, ()).map_err(text)? as usize;
        // The engine's own copy of the transforms.
        let mut outside = vec![0u8; (n as usize + 1) * TRANSFORM];
        memory.read(&store, at, &mut outside).map_err(text)?;
        let stats = measure(|| {
            if case.copy {
                memory.write(&mut store, at, &outside).map_err(text)?;
            }
            step.call(&mut store, (case.typed, case.whole)).map_err(text)?;
            if case.copy {
                memory.read(&store, at, &mut outside).map_err(text)?;
            }
            Ok(())
        })?;
        let check = instance.get_typed_func::<(u32, u32), f64>(&mut store, "check").map_err(text)?.call(&mut store, (n, case.typed)).map_err(text)?;
        print("compiled", case, n, check, &stats);
    }
    Ok(())
}

fn interpreted(bytes: &[u8], n: u32) -> Result<()> {
    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, bytes).map_err(text)?;
    for case in &CASES {
        let mut store = wasmi::Store::new(&engine, ());
        let linker = wasmi::Linker::<()>::new(&engine);
        let instance = linker.instantiate_and_start(&mut store, &module).map_err(text)?;
        instance.get_typed_func::<u32, ()>(&store, "setup").map_err(text)?.call(&mut store, n).map_err(text)?;
        let step = instance.get_typed_func::<(u32, u32), f64>(&store, "step").map_err(text)?;
        let memory = instance.get_memory(&store, "memory").ok_or("the module exports no memory")?;
        let at = instance.get_typed_func::<(), u32>(&store, "transforms_ptr").map_err(text)?.call(&mut store, ()).map_err(text)? as usize;
        let mut outside = vec![0u8; (n as usize + 1) * TRANSFORM];
        memory.read(&store, at, &mut outside).map_err(text)?;
        let stats = measure(|| {
            if case.copy {
                memory.write(&mut store, at, &outside).map_err(text)?;
            }
            step.call(&mut store, (case.typed, case.whole)).map_err(text)?;
            if case.copy {
                memory.read(&store, at, &mut outside).map_err(text)?;
            }
            Ok(())
        })?;
        let check = instance.get_typed_func::<(u32, u32), f64>(&store, "check").map_err(text)?.call(&mut store, (n, case.typed)).map_err(text)?;
        print("interpreted", case, n, check, &stats);
    }
    Ok(())
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("give the module: runtime-wasm-host <runtime_rust.wasm> <objects>")?;
    let n: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(5000);
    let bytes = std::fs::read(&path).map_err(|e| format!("reading {path}: {e}"))?;
    compiled(&bytes, n)?;
    interpreted(&bytes, n)
}
