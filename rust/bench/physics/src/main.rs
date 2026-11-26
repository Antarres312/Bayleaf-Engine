//! L7: internal performance benchmark of the physics step. Builds a scene of N dynamic boxes in
//! columns of ten above a wide static floor, each box a little off its column and turned, so the
//! columns fall over into each other and the boxes keep touching, then times `Physics::step` (what
//! play mode calls each step: Rapier's step, kinematic bodies in, dynamic bodies written back to
//! the scene). Prints one JSON line per run, with a checksum of every box's final place so runs
//! can be compared for the same result.
//!
//! `bench/README.md` ("Physics on several cores") says how to run it and read the numbers.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::time::Instant;

use anyhow::Context;
use bayleaf_core::StableId;
use bayleaf_physics::{GRAVITY, Physics, Precision, STEP};
use bayleaf_scene::Scene;
use bayleaf_scene::components::{BodyKind, Collider, RigidBody, Transform};
use clap::{Parser, ValueEnum};
use serde_json::json;

/// Boxes per column.
const LAYERS: usize = 10;
/// Distance between columns, metres (the boxes are 1 m).
const SPACING: f64 = 1.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Precise {
    Single,
    Double,
}

#[derive(Parser, Debug)]
#[command(about = "Times Bayleaf's physics step on a scene of many dynamic boxes")]
struct Args {
    /// Dynamic bodies (the floor is one more).
    #[arg(long, default_value_t = 5000)]
    bodies: usize,
    /// Steps run before timing starts.
    #[arg(long, default_value_t = 30)]
    warmup: usize,
    /// Steps timed, each alone.
    #[arg(long, default_value_t = 240)]
    steps: usize,
    #[arg(long, value_enum, default_value = "single")]
    precision: Precise,
    /// Worker threads for this world (0: the shared pool, as play mode steps).
    #[arg(long, default_value_t = 0)]
    threads: usize,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let (mut scene, boxes) = build_scene(args.bodies)?;
    let precision = match args.precision {
        Precise::Single => Precision::Single,
        Precise::Double => Precision::Double,
    };
    let started = Instant::now();
    let mut physics = Physics::build(&scene, GRAVITY, precision);
    let build_ms = started.elapsed().as_secs_f64() * 1000.0;
    if args.threads > 0 {
        physics.set_threads(args.threads);
    }
    for _ in 0..args.warmup {
        physics.step(&mut scene, STEP);
    }
    let mut times = Vec::with_capacity(args.steps);
    for _ in 0..args.steps {
        let start = Instant::now();
        physics.step(&mut scene, STEP);
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let stats = Stats::of(&times);
    let result = json!({
        "bodies": args.bodies,
        "precision": format!("{:?}", args.precision).to_lowercase(),
        "threads": physics.threads(),
        "warmup": args.warmup,
        "steps": args.steps,
        "build_ms": build_ms,
        "mean_ms": stats.mean,
        "median_ms": stats.median,
        "p5_ms": stats.p5,
        "p95_ms": stats.p95,
        "stddev_ms": stats.stddev,
        "checksum": format!("{:016x}", checksum(&scene, &boxes)),
    });
    println!("{result}");
    Ok(())
}

/// A wide floor and `bodies` boxes in columns of [`LAYERS`] on a square grid.
fn build_scene(bodies: usize) -> anyhow::Result<(Scene, Vec<StableId>)> {
    let mut scene = Scene::new();
    let columns = bodies.div_ceil(LAYERS);
    let side = (columns as f64).sqrt().ceil().max(1.0) as usize;
    let width = side as f64 * SPACING;
    let floor = scene.create_object(None, "Floor").context("floor")?;
    scene.set(floor, Collider::default()).context("floor collider")?;
    let floor_size = (width + 20.0) as f32;
    scene
        .set(floor, Transform { position: [width / 2.0, -0.5, width / 2.0], rotation: [0.0; 3], scale: [floor_size, 1.0, floor_size] })
        .context("floor transform")?;
    let mut boxes = Vec::with_capacity(bodies);
    for k in 0..bodies {
        let column = k / LAYERS;
        let layer = k % LAYERS;
        // Fixed offsets in [-0.2, 0.2] m and turns in [0, 90) degrees, the same in every run.
        let jitter = |seed: usize| ((seed.wrapping_mul(7919) % 41) as f64) / 100.0 - 0.2;
        let x = (column % side) as f64 * SPACING + jitter(k);
        let z = (column / side) as f64 * SPACING + jitter(k + 17);
        let y = 1.0 + layer as f64 * 1.1;
        let id = scene.create_object(None, &format!("Box {k}")).context("box")?;
        scene.set(id, RigidBody { kind: BodyKind::Dynamic, ..RigidBody::default() }).context("box body")?;
        scene.set(id, Collider::default()).context("box collider")?;
        let turn = ((k * 37) % 90) as f32;
        scene.set(id, Transform { position: [x, y, z], rotation: [0.0, turn, 0.0], scale: [1.0; 3] }).context("box transform")?;
        boxes.push(id);
    }
    Ok((scene, boxes))
}

/// FNV-1a over the bits of every box's position and rotation, in scene order.
fn checksum(scene: &Scene, boxes: &[StableId]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for id in boxes {
        if let Some(t) = scene.get::<Transform>(*id) {
            for v in t.position {
                eat(&v.to_bits().to_le_bytes());
            }
            for v in t.rotation {
                eat(&v.to_bits().to_le_bytes());
            }
        }
    }
    hash
}

struct Stats {
    mean: f64,
    median: f64,
    p5: f64,
    p95: f64,
    stddev: f64,
}

impl Stats {
    fn of(times: &[f64]) -> Self {
        let mut sorted = times.to_vec();
        sorted.sort_by(f64::total_cmp);
        let n = sorted.len().max(1) as f64;
        let mean = sorted.iter().sum::<f64>() / n;
        let at = |q: f64| sorted.get(((sorted.len().saturating_sub(1)) as f64 * q).round() as usize).copied().unwrap_or(0.0);
        let stddev = (sorted.iter().map(|t| (t - mean).powi(2)).sum::<f64>() / n).sqrt();
        Self { mean, median: at(0.5), p5: at(0.05), p95: at(0.95), stddev }
    }
}
