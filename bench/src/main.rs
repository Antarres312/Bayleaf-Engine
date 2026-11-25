//! L7: internal performance benchmark. Builds a scene of N objects, each with a transform, a mesh, a
//! collider, a tag, a point light on every tenth, and three scripts (spin, bob, follow a target),
//! then times either the script step alone (`ScriptHost::update`, what play mode calls) or the whole
//! play step (`play.step` on a headless session). Prints one JSON line per run.
//!
//! `bench/README.md` says how to build the scripts, run every case and read the numbers.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use bayleaf_core::StableId;
use bayleaf_engine::{CommandRegistry, Level, Session};
use bayleaf_scene::Scene;
use bayleaf_scene::components::{Collider, Mesh, PointLight, PropertyValue, ScriptRef, Scripts, Tags, Transform};
use bayleaf_script::{Input, NativeScript, Runtime, ScriptHost};
use clap::{Parser, ValueEnum};
use serde_json::json;

// The benchmark's Rust scripts, built in as a native-mode player builds them: the same sources
// that `bench/scripts` compiles to WebAssembly, here compiled for this computer.
#[path = "../scripts/bob/src/lib.rs"]
mod bob;
#[path = "../scripts/empty/src/lib.rs"]
mod empty;
#[path = "../scripts/follow/src/lib.rs"]
mod follow;
#[path = "../scripts/spin/src/lib.rs"]
mod spin;

/// The script names, in the order each object runs them.
const SCRIPTS: [&str; 3] = ["spin", "bob", "follow"];
/// The script whose update does nothing.
const EMPTY: &str = "empty";
/// The same, in a module that imports the host's component functions.
const FIELDS: &str = "fields";

/// One simulation step: 60 per second.
const DT: f32 = 1.0 / 60.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Mode {
    /// Interpreted (wasmi), as the editor runs scripts.
    Interpreted,
    /// Compiled to machine code (wasmtime), as the export mode of that name.
    Compiled,
    /// Rust scripts built into the program (native mode): machine code behind the compiled host.
    Native,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Kind {
    /// Rust scripts (`bench/scripts`).
    Rust,
    /// Visual scripts (`bench/graphs`).
    Graph,
    /// Three Rust scripts per object whose update does nothing: the cost of running a script.
    Empty,
    /// As `empty`, in a module that imports the component functions (visual scripts and native
    /// scripts do), which makes every step gather every object's component fields.
    Fields,
    /// No scripts: the objects alone.
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Part {
    /// The script step alone: every script's update, as play mode runs it.
    Scripts,
    /// The whole play step through the `play.step` command.
    Step,
}

#[derive(Parser, Debug)]
#[command(about = "Times Bayleaf's script step or play step on a generated scene")]
struct Args {
    #[arg(long, value_enum, default_value = "interpreted")]
    mode: Mode,
    #[arg(long, value_enum, default_value = "rust")]
    scripts: Kind,
    #[arg(long, value_enum, default_value = "scripts")]
    part: Part,
    /// Objects with scripts (one more, the target, has none).
    #[arg(long, default_value_t = 5000)]
    objects: usize,
    /// Steps run before timing starts.
    #[arg(long, default_value_t = 30)]
    warmup: usize,
    /// Timing runs at least this many steps...
    #[arg(long, default_value_t = 100)]
    min_frames: usize,
    /// ...and at least this long, in seconds...
    #[arg(long, default_value_t = 3.0)]
    seconds: f64,
    /// ...but no more than this many steps.
    #[arg(long, default_value_t = 5000)]
    max_frames: usize,
    /// Scripts part: load with `ScriptHost::load_scene` on the whole scene, as play mode does
    /// (slow past a few thousand objects; see `load_against_target`).
    #[arg(long)]
    full_load: bool,
    /// The built Rust scripts (`.wasm`).
    #[arg(long, default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/target/wasm32-unknown-unknown/release"))]
    wasm: PathBuf,
    /// Where the scene and its script files are written.
    #[arg(long, default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/../target/bench-work"))]
    work: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if args.mode == Mode::Native && args.scripts == Kind::Graph {
        bail!("visual scripts have no native form: they stay compiled WebAssembly in native mode");
    }
    let work = prepare_work(&args)?;
    let scene = build_scene(args.objects, args.scripts)?;
    let result = match args.part {
        Part::Scripts => time_scripts(&args, &work, scene),
        Part::Step => time_step(&args, &work, scene),
    };
    // Each run writes its own folder; it is not needed afterwards.
    let _ = std::fs::remove_dir_all(&work);
    println!("{}", result?);
    Ok(())
}

/// The work folder with the script files the scene names: `scripts/<name>.wasm` and `.bscript`.
fn prepare_work(args: &Args) -> anyhow::Result<PathBuf> {
    let work = args.work.join(format!("{}-{}", std::process::id(), args.objects));
    let scripts = work.join("scripts");
    std::fs::create_dir_all(&scripts).with_context(|| format!("cannot make {}", scripts.display()))?;
    let graphs = Path::new(env!("CARGO_MANIFEST_DIR")).join("graphs");
    for name in SCRIPTS.into_iter().chain([EMPTY, FIELDS]) {
        let wasm = args.wasm.join(format!("{name}.wasm"));
        std::fs::copy(&wasm, scripts.join(format!("{name}.wasm")))
            .with_context(|| format!("cannot copy {}; build bench/scripts first (bench/README.md)", wasm.display()))?;
        if name == EMPTY || name == FIELDS {
            continue;
        }
        // Compiled once, as an export keeps visual scripts: the host would otherwise compile the
        // graph again for every object that runs it, which only slows loading.
        let graph = graphs.join(format!("{name}.bscript"));
        let module = bayleaf_script::module_bytes(&graph).with_context(|| format!("{} does not compile", graph.display()))?;
        std::fs::write(scripts.join(format!("{name}.bscript")), module).with_context(|| format!("cannot write {name}.bscript"))?;
    }
    Ok(work)
}

/// Loads every script of `scene` as `ScriptHost::load_scene` does (each instance, then every init,
/// then every start), but against a copy of the scene that holds only the target. Loading builds a
/// view of the whole scene for every instance and keeps it until the first step, which grows with
/// objects × instances (minutes and many gigabytes past a few thousand objects); the steps timed
/// afterwards run on the whole scene, exactly as in play mode.
///
/// When a loaded module imports the component functions (visual scripts and the native bridge do),
/// each new instance also copies every loaded script's properties, kept until the next step. Then
/// `batch` is the number of instances loaded before a step lets those copies go; the steps during
/// loading are not timed.
fn load_against_target(host: &mut ScriptHost, scene: &mut Scene, work: &Path, input: &Arc<Input>, batch: usize) -> anyhow::Result<()> {
    let mut doc = scene.to_doc();
    doc.objects.truncate(1);
    let mut small = Scene::from_doc(&doc)?;
    let mut list = Vec::new();
    for (id, _) in scene.world_matrices() {
        for (index, script) in scene.get::<Scripts>(id).map(|s| s.0.clone()).unwrap_or_default().into_iter().enumerate() {
            list.push((id, index, work.join(&script.module), script));
        }
    }
    for chunk in list.chunks(batch.max(1)) {
        if let Some((_, _, error)) = host.load_files(chunk, &mut small, 0.0, input).first() {
            bail!("a script did not load: {error}");
        }
        if chunk.len() < list.len() {
            host.update(scene, DT, 0.0, input);
            host.take_timings();
            host.take_logs();
        }
    }
    Ok(())
}

/// Per-object property values: varied, and the same formulas as `bench/cpp/bench.cpp`.
fn properties(name: &str, i: usize) -> BTreeMap<String, PropertyValue> {
    let n = |v: f64| PropertyValue::Number(v);
    let pairs: Vec<(&str, f64)> = match name {
        "spin" => vec![("speed", 30.0 + (i % 7) as f64 * 10.0)],
        "bob" => vec![("speed", 1.0 + (i % 5) as f64 * 0.25), ("height", 0.25 + (i % 3) as f64 * 0.1), ("base", 0.5)],
        "follow" => vec![("rate", 0.05 + (i % 4) as f64 * 0.05)],
        _ => Vec::new(),
    };
    pairs.into_iter().map(|(k, v)| (k.to_owned(), n(v))).collect()
}

/// The target (first, so lookups by name or kept reference find it at once), then `n` objects on a
/// grid, each with a mesh, a collider, a tag, a point light on every tenth, and three scripts.
fn build_scene(n: usize, kind: Kind) -> anyhow::Result<Scene> {
    let mut scene = Scene::new();
    let target = scene.create_object(None, "Target")?;
    scene.set(target, Transform::default())?;
    let side = (n as f64).sqrt().ceil().max(1.0) as usize;
    let extension = match kind {
        Kind::Graph => "bscript",
        _ => "wasm",
    };
    for i in 0..n {
        let id = scene.create_object(None, &format!("Object {i}"))?;
        let (x, z) = ((i % side) as f64 * 2.0 - side as f64, (i / side) as f64 * 2.0 - side as f64);
        scene.set(id, Transform { position: [x, 0.0, z], ..Transform::default() })?;
        scene.set(id, Mesh::default())?;
        scene.set(id, Collider::default())?;
        scene.set(id, Tags(vec!["bench".to_owned()]))?;
        if i % 10 == 0 {
            scene.set(id, PointLight::default())?;
        }
        if kind != Kind::None {
            let names = match kind {
                Kind::Empty => [EMPTY; 3],
                Kind::Fields => [FIELDS; 3],
                _ => SCRIPTS,
            };
            let scripts = names.iter().map(|name| ScriptRef { module: format!("scripts/{name}.{extension}"), properties: properties(name, i) }).collect();
            scene.set(id, Scripts(scripts))?;
        }
    }
    Ok(scene)
}

/// The native scripts, by the module each stands for.
fn natives() -> Vec<(PathBuf, NativeScript)> {
    vec![
        (PathBuf::from("scripts/spin.wasm"), spin::bayleaf_native()),
        (PathBuf::from("scripts/bob.wasm"), bob::bayleaf_native()),
        (PathBuf::from("scripts/follow.wasm"), follow::bayleaf_native()),
        (PathBuf::from("scripts/empty.wasm"), empty::bayleaf_native()),
    ]
}

/// Mean, spread and percentiles of the timed steps, in milliseconds.
fn stats(times: &[Duration]) -> serde_json::Value {
    let mut ms: Vec<f64> = times.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    ms.sort_by(f64::total_cmp);
    let count = ms.len().max(1) as f64;
    let mean = ms.iter().sum::<f64>() / count;
    let stdev = (ms.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / count).sqrt();
    let at = |q: f64| ms.get(((ms.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0.0);
    json!({ "frames": ms.len(), "meanMs": mean, "medianMs": at(0.5), "p5Ms": at(0.05), "p95Ms": at(0.95), "stdevMs": stdev })
}

/// Runs `step` through warm-up, then times it until the frame and time minimums are met.
fn measure(args: &Args, mut step: impl FnMut() -> anyhow::Result<()>) -> anyhow::Result<Vec<Duration>> {
    for _ in 0..args.warmup {
        step()?;
    }
    let mut times = Vec::new();
    let started = Instant::now();
    while times.len() < args.max_frames && (times.len() < args.min_frames || started.elapsed().as_secs_f64() < args.seconds) {
        let t = Instant::now();
        step()?;
        times.push(t.elapsed());
    }
    Ok(times)
}

/// A sum over every object's position and turn: equal between two runs of the same number of steps
/// (`--warmup`, `--min-frames` and `--max-frames` alike, `--seconds 0`) when the scripts did the
/// same work.
fn checksum(scene: &Scene) -> f64 {
    scene.world_matrices().iter().filter_map(|(id, _)| scene.get::<Transform>(*id)).map(|t| t.position.iter().sum::<f64>() + f64::from(t.rotation[1])).sum()
}

/// The first scripted object's transform, to show the scripts moved it.
fn probe(scene: &Scene) -> Option<Transform> {
    let first: StableId = *scene.children(None).get(1)?;
    scene.get::<Transform>(first).cloned()
}

fn moved(before: &Option<Transform>, after: &Option<Transform>) -> bool {
    before != after
}

fn label(args: &Args) -> serde_json::Value {
    json!({
        "mode": format!("{:?}", args.mode).to_lowercase(),
        "scripts": format!("{:?}", args.scripts).to_lowercase(),
        "part": format!("{:?}", args.part).to_lowercase(),
        "objects": args.objects,
    })
}

/// The script step alone: `ScriptHost::update`, the call play mode makes each step.
fn time_scripts(args: &Args, work: &Path, mut scene: Scene) -> anyhow::Result<serde_json::Value> {
    let mut host = match args.mode {
        Mode::Interpreted => ScriptHost::new(),
        Mode::Compiled | Mode::Native => ScriptHost::with_runtime(Runtime::Compiled)?,
    };
    if args.mode == Mode::Native {
        host = host.with_natives(natives());
    }
    let input = Arc::new(Input::default());
    let before = probe(&scene);
    let loading = Instant::now();
    if args.full_load {
        host.load_scene(&mut scene, work, 0.0, &input);
    } else {
        // Visual scripts, the native bridge and the `fields` control import the component functions.
        let fields = args.scripts == Kind::Graph || args.scripts == Kind::Fields || args.mode == Mode::Native;
        load_against_target(&mut host, &mut scene, work, &input, if fields { 300 } else { usize::MAX })?;
    }
    let load_s = loading.elapsed().as_secs_f64();
    if let Some(fault) = host.take_faults().first() {
        bail!("a script did not load: {} ({})", fault.error, fault.script);
    }
    let instances = host.len();
    let mut time = 0.0f64;
    let times = measure(args, || {
        time += f64::from(DT);
        host.update(&mut scene, DT, time, &input);
        // Play mode takes these every step; left alone they would pile up.
        host.take_timings();
        host.take_logs();
        if let Some(fault) = host.take_faults().first() {
            bail!("a script stopped: {} ({})", fault.error, fault.script);
        }
        Ok(())
    })?;
    let mut out = label(args);
    out["loadSeconds"] = json!(load_s);
    out["fullLoad"] = json!(args.full_load);
    out["instances"] = json!(instances);
    out["running"] = json!(host.running());
    out["moved"] = json!(moved(&before, &probe(&scene)));
    out["check"] = json!(checksum(&scene));
    out["stats"] = stats(&times);
    Ok(out)
}

/// The whole play step: `play.step` on a headless session with the scene open.
fn time_step(args: &Args, work: &Path, scene: Scene) -> anyhow::Result<serde_json::Value> {
    let path = work.join("bench.bscene");
    scene.save(&path)?;
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    if args.mode == Mode::Native {
        session.set_native_scripts(natives());
    }
    let run = |session: &mut Session, id: &str, params: serde_json::Value| registry.execute(session, id, &params).map_err(|e| anyhow::anyhow!("{id}: {e}"));
    run(&mut session, "scene.open", json!({ "path": path }))?;
    let before = probe(session.scene());
    let runtime = if args.mode == Mode::Interpreted { "interpreted" } else { "compiled" };
    let loading = Instant::now();
    let started = run(&mut session, "play.start", json!({ "scripts": runtime }))?;
    let load_s = loading.elapsed().as_secs_f64();
    if started["scripts"] != started["running"] {
        let errors: Vec<&str> = session.console().iter().filter(|l| l.level == Level::Error).map(|l| l.text.as_str()).take(3).collect();
        bail!("not every script runs: {started}; {errors:?}");
    }
    let step = json!({ "seconds": DT, "steps": 1 });
    let times = measure(args, || run(&mut session, "play.step", step.clone()).map(|_| ()))?;
    let errors = session.console().iter().filter(|l| l.level == Level::Error).count();
    if errors > 0 {
        bail!("scripts reported {errors} errors while playing");
    }
    let mut out = label(args);
    out["loadSeconds"] = json!(load_s);
    out["instances"] = started["scripts"].clone();
    out["running"] = started["running"].clone();
    out["moved"] = json!(moved(&before, &probe(session.scene())));
    out["stats"] = stats(&times);
    Ok(out)
}
