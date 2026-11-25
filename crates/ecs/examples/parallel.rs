//! Micro-benchmark of the ECS on one thread and on the thread pool (bench/README.md, "The ECS on
//! several cores"). Run with `bench/run-ecs.ps1`, or one case by hand:
//!
//! `cargo run --release -p bayleaf-ecs --features parallel --example parallel -- light 20000 on`
//!
//! Arguments: the case (`light`, `heavy` or `schedule`), the number of entities, and `on` or `off`
//! for the parallel switch. Prints one JSON line.

use std::error::Error;
use std::time::{Duration, Instant};

use bayleaf_ecs::{Access, Schedule, View, World};

const DT: f32 = 1.0 / 60.0;
const WARM_UP: usize = 30;
const MIN_STEPS: usize = 100;
const MIN_TIME: Duration = Duration::from_secs(3);
const MAX_STEPS: usize = 20_000;
const CHECK_STEPS: usize = 100;

struct Transform {
    position: [f32; 3],
    yaw: f32,
    scale: f32,
}
struct Motion {
    velocity: [f32; 3],
    spin: f32,
}
struct Matrix([f32; 16]);
struct Health(f32);
struct Regen(f32);
struct Phase(f32);
struct Rate(f32);
struct Light;

fn populate(world: &mut World, n: usize) -> Result<(), Box<dyn Error>> {
    for i in 0..n {
        let f = i as f32;
        let e = world.spawn()?;
        world.insert(e, Transform { position: [f % 100.0, 0.0, f / 100.0], yaw: 0.0, scale: 1.0 + (i % 5) as f32 * 0.1 })?;
        world.insert(e, Motion { velocity: [(i % 7) as f32 * 0.1, 0.0, (i % 3) as f32 * 0.1], spin: 0.5 + (i % 11) as f32 * 0.1 })?;
        world.insert(e, Matrix([0.0; 16]))?;
        world.insert(e, Health(f % 100.0))?;
        world.insert(e, Regen(0.25))?;
        world.insert(e, Phase(0.0))?;
        world.insert(e, Rate(1.0 + (i % 13) as f32 * 0.05))?;
        if i % 10 == 0 {
            world.insert(e, Light)?;
        }
    }
    Ok(())
}

fn integrate(t: &mut Transform, m: &Motion) {
    for (p, v) in t.position.iter_mut().zip(m.velocity) {
        *p += v * DT;
    }
    t.yaw += m.spin * DT;
}

/// The world matrix: yaw, a tilt that follows it, scale and position.
fn matrix(out: &mut Matrix, t: &Transform) {
    let (sy, cy) = t.yaw.sin_cos();
    let (sp, cp) = (t.yaw * 0.5).sin_cos();
    let s = t.scale;
    let rows = [[cy * s, sy * sp * s, sy * cp * s], [0.0, cp * s, -sp * s], [-sy * s, cy * sp * s, cy * cp * s]];
    for (r, row) in rows.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            if let Some(slot) = out.0.get_mut(c * 4 + r) {
                *slot = *v;
            }
        }
    }
    out.0[12..15].copy_from_slice(&t.position);
    out.0[15] = 1.0;
}

fn schedule(case: &str) -> Result<Schedule, Box<dyn Error>> {
    let mut s = Schedule::new();
    let moves = |v: &mut View<'_>| v.for_each_mut_with::<Transform, Motion>(|_, t, m| integrate(t, m));
    let matrices = |v: &mut View<'_>| v.for_each_mut_with::<Matrix, Transform>(|_, out, t| matrix(out, t));
    match case {
        "light" => s.add_system("move", Access::new().write::<Transform>().read::<Motion>(), moves),
        "heavy" => s.add_system("matrices", Access::new().write::<Matrix>().read::<Transform>(), matrices),
        "schedule" => {
            s.add_system("move", Access::new().write::<Transform>().read::<Motion>(), moves);
            s.add_system("heal", Access::new().write::<Health>().read::<Regen>(), |v: &mut View<'_>| {
                v.for_each_mut_with::<Health, Regen>(|_, h, r| h.0 = (h.0 + r.0 * DT).min(100.0))
            });
            s.add_system("animate", Access::new().write::<Phase>().read::<Rate>(), |v: &mut View<'_>| {
                v.for_each_mut_with::<Phase, Rate>(|_, p, r| p.0 = (p.0 + r.0 * DT).fract() + (p.0 * std::f32::consts::TAU).sin() * 1e-3)
            });
            s.add_system("matrices", Access::new().write::<Matrix>().read::<Transform>(), matrices);
        }
        other => return Err(format!("unknown case {other:?}; use light, heavy or schedule").into()),
    }
    Ok(s)
}

/// Sum over every entity's matrix, transform, health and phase, to check both modes end the same.
fn checksum(world: &World) -> f64 {
    let m: f64 = world.query::<Matrix>().map(|(_, m)| m.0.iter().map(|v| f64::from(*v)).sum::<f64>()).sum();
    let t: f64 = world.query::<Transform>().map(|(_, t)| f64::from(t.position[0] + t.yaw)).sum();
    let h: f64 = world.query::<Health>().map(|(_, h)| f64::from(h.0)).sum();
    let p: f64 = world.query::<Phase>().map(|(_, p)| f64::from(p.0)).sum();
    m + t + h + p
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [case, n, mode] = args.as_slice() else {
        return Err("usage: parallel <light|heavy|schedule> <entities> <on|off>".into());
    };
    let n: usize = n.parse()?;
    let mut world = World::new();
    populate(&mut world, n)?;
    let wanted = mode == "on";
    let parallel = world.set_parallel(wanted);
    if wanted && !parallel {
        return Err("this build has no thread pool; build with --features parallel".into());
    }
    // The same steps on a fresh world, a fixed number of times, for the checksum.
    let check = {
        let mut w = World::new();
        populate(&mut w, n)?;
        w.set_parallel(parallel);
        let mut s = schedule(case)?;
        for _ in 0..CHECK_STEPS {
            s.run(&mut w);
        }
        checksum(&w)
    };
    let mut s = schedule(case)?;
    for _ in 0..WARM_UP {
        s.run(&mut world);
    }
    let mut times = Vec::new();
    let start = Instant::now();
    while times.len() < MAX_STEPS && (times.len() < MIN_STEPS || start.elapsed() < MIN_TIME) {
        let t = Instant::now();
        s.run(&mut world);
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let steps = times.len();
    let mean = times.iter().sum::<f64>() / steps as f64;
    let sd = (times.iter().map(|t| (t - mean).powi(2)).sum::<f64>() / steps as f64).sqrt();
    times.sort_by(f64::total_cmp);
    let at = |q: f64| times.get(((steps - 1) as f64 * q).round() as usize).copied().unwrap_or(0.0);
    let threads = std::env::var("RAYON_NUM_THREADS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |v| v.get()));
    println!(
        "{{\"case\":\"{case}\",\"entities\":{n},\"parallel\":{parallel},\"threads\":{},\"steps\":{steps},\"stats\":{{\"meanMs\":{mean},\"medianMs\":{},\"p5Ms\":{},\"p95Ms\":{},\"sdMs\":{sd}}},\"check\":{}}}",
        if parallel { threads } else { 1 },
        at(0.5),
        at(0.05),
        at(0.95),
        check
    );
    Ok(())
}
