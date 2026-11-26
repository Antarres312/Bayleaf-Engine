//! The C# engine must read, write back and compile every animation states file (`.banim`) of the
//! examples, the benchmark and `tests/files` as this one does (docs/csharp-redesign.md). For each,
//! this test writes into `tests/files/expected/` what it writes back after reading it with the
//! animation states' catalog (`<name>`, or `refused`), and a summary of what it compiles to and how
//! that runs on the arm fixture (`<name>.summary`). It writes them when `BAYLEAF_BLESS=1` is set and
//! compares otherwise; `tests/Bayleaf.Animation.Tests` compares the C# engine's with the same files.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bayleaf_assets::skeleton::{Pose, Skeleton};
use bayleaf_graph::Graph;

use crate::states::{Amount, CATALOG, Condition, Machine, ParameterKind, Test, compile};
use crate::{ClipEvent, Clips, Motion, Number, Parameter, Player, default_root};

fn num(v: f32) -> String {
    if v.is_nan() {
        "NaN".to_owned()
    } else if v.is_infinite() {
        if v > 0.0 { "inf".to_owned() } else { "-inf".to_owned() }
    } else {
        format!("{v:.4}")
    }
}

fn number(n: &Number) -> String {
    match n {
        Number::Value(v) => format!("value {}", num(*v)),
        Number::Parameter(name) => format!("parameter {name:?}"),
    }
}

fn motion(m: &Motion) -> String {
    match m {
        Motion::Rest => "rest".to_owned(),
        Motion::Clip { clip, speed, looping } => format!("( clip {clip:?} speed {} {} )", num(*speed), if *looping { "loop" } else { "once" }),
        Motion::Mix { items, by } => {
            let parts: Vec<String> = items.iter().map(|(m, at)| format!("{} at {}", motion(m), num(*at))).collect();
            format!("( mix by {} {} )", number(by), parts.join(" "))
        }
        Motion::Layer { base, top, joint, weight } => {
            format!("( layer joint {joint:?} weight {} base {} top {} )", number(weight), motion(base), motion(top))
        }
    }
}

fn amount(a: &Amount) -> String {
    match a {
        Amount::Value(v) => format!("value {}", num(*v)),
        Amount::Parameter(name) => format!("parameter {name:?}"),
        Amount::Seconds => "seconds".to_owned(),
        Amount::Runs => "runs".to_owned(),
    }
}

fn condition(c: &Condition) -> String {
    match c {
        Condition::Value(b) => b.to_string(),
        Condition::Flag(name) => format!("flag {name:?}"),
        Condition::Trigger(name) => format!("trigger {name:?}"),
        Condition::Compare(a, test, b) => {
            let test = match test {
                Test::Greater => "greater",
                Test::Less => "less",
                Test::AtLeast => "at-least",
                Test::AtMost => "at-most",
                Test::Equal => "equal",
                Test::NotEqual => "not-equal",
            };
            format!("( compare {} {test} {} )", amount(a), amount(b))
        }
        Condition::And(a, b) => format!("( and {} {} )", condition(a), condition(b)),
        Condition::Or(a, b) => format!("( or {} {} )", condition(a), condition(b)),
        Condition::Not(c) => format!("( not {} )", condition(c)),
    }
}

/// The arm fixture (`bayleaf_assets::fixtures`), with a step and a landing marked in Stretch.
fn arm() -> (Arc<Skeleton>, Clips) {
    let dir = std::env::temp_dir().join(format!("bayleaf-animation-shared-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let model = bayleaf_assets::load_model(&bayleaf_assets::fixtures::skinned_arm(&dir).unwrap()).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let skeleton = model.skeleton.unwrap();
    let marks =
        [("Stretch".to_owned(), ClipEvent { time: 0.25, name: "step".to_owned() }), ("Stretch".to_owned(), ClipEvent { time: 0.75, name: "land".to_owned() })];
    let clips = Clips::of_model(&skeleton, &marks);
    (skeleton, clips)
}

fn pose_line(pose: &Pose) -> String {
    let mut line = String::new();
    for (i, t) in pose.locals.iter().enumerate() {
        let (p, r) = (t.translation, t.rotation);
        let _ = write!(line, " n{i} {} {} {} {} {} {} {}", num(p.x), num(p.y), num(p.z), num(r.x), num(r.y), num(r.z), num(r.w));
    }
    line
}

/// The parameters a run sets before step `i`: every one of a kind at once, by the schedule below.
fn schedule(i: usize, machine: &Machine, player: &mut Player) {
    for (name, kind) in &machine.parameters {
        let value = match (i, kind) {
            (3 | 18 | 30, ParameterKind::Trigger) => Parameter::Trigger(true),
            (8, ParameterKind::Number) => Parameter::Number(1.0),
            (24, ParameterKind::Number) => Parameter::Number(6.0),
            (14, ParameterKind::Flag) => Parameter::Flag(true),
            (30, ParameterKind::Flag) => Parameter::Flag(false),
            _ => continue,
        };
        player.set_parameter(name, value).unwrap();
    }
}

/// What the states compile to, and how they run on the arm for 40 steps of a tenth of a second
/// with root motion on, setting their parameters by [`schedule`].
fn machine_summary(machine: Machine, skeleton: &Skeleton, clips: &Clips) -> String {
    let mut s = String::new();
    let start = machine.states.get(machine.start).map_or("-", |st| st.name.as_str());
    let _ = writeln!(s, "states {} start {start:?}", machine.states.len());
    for state in &machine.states {
        let _ = writeln!(s, "state {:?} {}", state.name, motion(&state.motion));
    }
    for t in &machine.transitions {
        let from = t.from.and_then(|f| machine.states.get(f)).map_or("any".to_owned(), |st| format!("{:?}", st.name));
        let to = machine.states.get(t.to).map_or("-", |st| st.name.as_str());
        let _ = writeln!(s, "transition from {from} to {to:?} fade {} when {}", num(t.fade), condition(&t.when));
    }
    for (name, kind) in &machine.parameters {
        let _ = writeln!(s, "parameter {name:?} {}", kind.label());
    }
    for state in &machine.states {
        for problem in state.motion.problems(skeleton, clips) {
            let _ = writeln!(s, "lacks {:?} {problem}", state.name);
        }
    }
    let machine = Arc::new(machine);
    let mut player = Player::new();
    player.root = default_root(skeleton);
    player.run_states(Arc::clone(&machine));
    for i in 0..40 {
        schedule(i, &machine, &mut player);
        let step = player.advance(skeleton, clips, 0.1);
        let events = if step.events.is_empty() { "-".to_owned() } else { step.events.join(",") };
        let fading = player.layers().first().is_some_and(crate::Layer::fading);
        let _ = writeln!(
            s,
            "step {i} state {:?} fading {fading} playing {} events {events} root {} {} {}{}",
            player.state().unwrap_or("-"),
            player.is_playing(),
            num(step.root.x),
            num(step.root.y),
            num(step.root.z),
            pose_line(&player.pose(skeleton, clips))
        );
    }
    s
}

/// What this crate writes back after reading the file at `path` with the animation states'
/// catalog, or `refused`; and its summary: what it compiles to, or `refused`.
fn rewrite(path: &Path, skeleton: &Skeleton, clips: &Clips) -> (String, String) {
    let refused = ("refused\n".to_owned(), "refused\n".to_owned());
    let Ok(text) = std::fs::read_to_string(path) else { return refused };
    let Ok(graph) = Graph::from_json(&text, &CATALOG) else { return refused };
    let summary = match compile(&graph) {
        Ok(machine) => machine_summary(machine, skeleton, clips),
        Err(problems) => {
            let mut s = String::new();
            for p in problems {
                let on = p.node.map_or("graph".to_owned(), |node| format!("node {node}"));
                let _ = writeln!(s, "problem {on} {}", p.message);
            }
            s
        }
    };
    (graph.to_json(), summary)
}

fn files_below(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "expected") {
                files_below(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == CATALOG.extension) {
            out.push(path);
        }
    }
}

#[test]
fn the_shared_animation_states_files_read_and_run_as_the_expected_files_say() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("..");
    let expected_dir = root.join("tests").join("files").join("expected");
    let bless = std::env::var_os("BAYLEAF_BLESS").is_some_and(|v| v == "1");
    let (skeleton, clips) = arm();
    let mut inputs = Vec::new();
    files_below(&root.join("examples"), &mut inputs);
    files_below(&root.join("rust").join("bench").join("graphs"), &mut inputs);
    files_below(&root.join("tests").join("files"), &mut inputs);
    inputs.sort();
    for input in &inputs {
        let (written, summary) = rewrite(input, &skeleton, &clips);
        let relative = input.strip_prefix(&root).unwrap_or(input).components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>();
        let name = relative.iter().skip_while(|c| *c == "tests" || *c == "files").cloned().collect::<Vec<_>>().join("__");
        for (path, text) in [(expected_dir.join(&name), &written), (expected_dir.join(format!("{name}.summary")), &summary)] {
            if bless {
                std::fs::create_dir_all(&expected_dir).unwrap();
                std::fs::write(&path, text).unwrap();
            }
            let expected = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
            assert_eq!(*text, expected, "{} gives otherwise than {}", input.display(), path.display());
        }
    }
    assert!(inputs.len() >= 10, "only {} shared animation states files were found", inputs.len());
}
