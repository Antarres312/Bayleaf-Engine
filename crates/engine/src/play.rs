//! Play mode: runs the scene's scripts on a copy of the scene and restores the edited scene on stop.

use bayleaf_scene::Scene;
use bayleaf_scene::doc::SceneDoc;
use bayleaf_script::{Request, ScriptFault, ScriptHost};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{CommandError, CommandSpec, Level, Mutates, Prompt, Session, parse};

/// How often play mode looks for changed script modules, in simulated seconds.
const RELOAD_CHECK_INTERVAL: f32 = 0.5;
/// Longest simulation step; longer frames (a breakpoint, a slow disk) are split.
const MAX_STEP: f32 = 0.1;
/// Most steps one `play.step` call may run.
const MAX_STEPS: u32 = 100_000;

pub(crate) struct Play {
    /// The scene as it was when play mode started; restored on stop.
    snapshot: SceneDoc,
    host: ScriptHost,
    pub(crate) time: f64,
    pub(crate) paused: bool,
    since_reload_check: f32,
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "play.start",
            label: "Play",
            menu: "Play/Play",
            shortcut: "Ctrl+P",
            params: "none; returns {scripts, running}",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: |s| if s.is_playing() { Err("play mode is already running".to_owned()) } else { Ok(()) },
            run: start,
        },
        CommandSpec {
            id: "play.stop",
            label: "Stop",
            menu: "Play/Stop",
            shortcut: "Ctrl+P",
            params: "none; restores the scene as it was before play",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: playing,
            run: stop,
        },
        CommandSpec {
            id: "play.pause",
            label: "Pause or Resume",
            menu: "Play/Pause or Resume",
            shortcut: "Ctrl+Shift+P",
            params: "none; toggles pause",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: playing,
            run: pause,
        },
        CommandSpec {
            id: "play.step",
            label: "Step",
            menu: "Play/Step",
            shortcut: "",
            params: "{seconds? = 1/60, steps? = 1}; advances play mode, also when paused",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: playing,
            run: step_command,
        },
        CommandSpec {
            id: "play.input",
            label: "Set Held Keys",
            menu: "",
            shortcut: "",
            params: "{keys: [names], pointer?: [x, y] (-1 to 1, +y up; null when outside the view), buttons?: n (bit 0 left, 1 right, 2 middle)}; what scripts see as held, until the next call or the next editor frame",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: playing,
            run: input,
        },
        CommandSpec {
            id: "console.clear",
            label: "Clear Console",
            menu: "",
            shortcut: "",
            params: "none",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: crate::always,
            run: |s, p| {
                parse::<NoParams>(p)?;
                s.clear_console();
                Ok(json!({}))
            },
        },
        CommandSpec {
            id: "play.status",
            label: "Play Status",
            menu: "",
            shortcut: "",
            params: "none; returns {playing, paused, time, scripts, running, console}",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: crate::always,
            run: status,
        },
    ]
}

fn playing(session: &Session) -> Result<(), String> {
    if session.is_playing() { Ok(()) } else { Err("play mode is not running".to_owned()) }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoParams {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StepParams {
    #[serde(default = "default_seconds")]
    seconds: f32,
    #[serde(default = "one")]
    steps: u32,
}

fn default_seconds() -> f32 {
    1.0 / 60.0
}

fn one() -> u32 {
    1
}

fn start(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    parse::<NoParams>(params)?;
    let dir = session.script_dir();
    // Under automation every script module must be inside the readable folders.
    let modules: Vec<std::path::PathBuf> = session
        .scene
        .world_matrices()
        .into_iter()
        .flat_map(|(id, _)| {
            session.scene.get::<bayleaf_scene::components::Scripts>(id).map(|s| s.0.iter().map(|r| dir.join(&r.module)).collect::<Vec<_>>()).unwrap_or_default()
        })
        .collect();
    for module in modules {
        session.allowed(&module, crate::confine::Access::Read)?;
    }
    let snapshot = session.scene.to_doc();
    let mut host = ScriptHost::new();
    let input = std::sync::Arc::clone(&session.input);
    host.load_scene(&mut session.scene, &dir, 0.0, &input);
    let (scripts, running) = (host.len(), host.running());
    session.play = Some(Play { snapshot, host, time: 0.0, paused: false, since_reload_check: 0.0 });
    session.log(Level::Info, format!("Play mode started: {running} of {scripts} scripts running."));
    drain(session);
    Ok(json!({ "scripts": scripts, "running": running }))
}

fn stop(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    parse::<NoParams>(params)?;
    let play = session.play.take().ok_or_else(|| CommandError::Failed("play mode is not running".to_owned()))?;
    session.scene = Scene::from_doc(&play.snapshot)?;
    session.log(Level::Info, format!("Play mode stopped after {:.2} s; the scene is back as it was.", play.time));
    Ok(json!({ "time": play.time }))
}

fn pause(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    parse::<NoParams>(params)?;
    let play = session.play.as_mut().ok_or_else(|| CommandError::Failed("play mode is not running".to_owned()))?;
    play.paused = !play.paused;
    Ok(json!({ "paused": play.paused }))
}

fn step_command(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: StepParams = parse(params)?;
    if !(p.seconds.is_finite() && p.seconds > 0.0 && p.seconds <= 1.0) {
        return Err(CommandError::BadParams("seconds must be more than 0 and at most 1".to_owned()));
    }
    if !(1..=MAX_STEPS).contains(&p.steps) {
        return Err(CommandError::BadParams(format!("steps must be between 1 and {MAX_STEPS}")));
    }
    for _ in 0..p.steps {
        step(session, p.seconds);
    }
    Ok(json!({ "time": session.play_time() }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputParams {
    keys: Vec<String>,
    #[serde(default)]
    pointer: Option<[f32; 2]>,
    #[serde(default)]
    buttons: u8,
}

fn input(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: InputParams = parse(params)?;
    if p.keys.len() > 64 || p.keys.iter().any(|k| k.len() > 32) {
        return Err(CommandError::BadParams("at most 64 keys of at most 32 characters".to_owned()));
    }
    if p.pointer.is_some_and(|[x, y]| !(x.is_finite() && y.is_finite())) {
        return Err(CommandError::BadParams("pointer must be two finite numbers".to_owned()));
    }
    session.set_keys(p.keys.into_iter().collect());
    session.set_pointer(p.pointer.map(|p| p.map(|c| c.clamp(-1.0, 1.0))), p.buttons & 0b111);
    Ok(json!({}))
}

fn status(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    parse::<NoParams>(params)?;
    let (scripts, running) = session.play.as_ref().map_or((0, 0), |p| (p.host.len(), p.host.running()));
    let tail: Vec<&crate::ConsoleLine> = session.console.iter().rev().take(50).rev().collect();
    Ok(json!({
        "playing": session.is_playing(),
        "paused": session.is_paused(),
        "time": session.play_time(),
        "scripts": scripts,
        "running": running,
        "console": tail,
    }))
}

/// Advances play mode by `dt` seconds, split into steps of at most [`MAX_STEP`].
pub(crate) fn step(session: &mut Session, dt: f32) {
    let mut remaining = dt.max(0.0);
    while remaining > 0.0 {
        let dt = remaining.min(MAX_STEP);
        remaining -= dt;
        let input = std::sync::Arc::clone(&session.input);
        let Some(play) = session.play.as_mut() else { return };
        play.time += f64::from(dt);
        play.host.update(&mut session.scene, dt, play.time, &input);
        apply_requests(session);
        let Some(play) = session.play.as_mut() else { return };
        play.since_reload_check += dt;
        if play.since_reload_check >= RELOAD_CHECK_INTERVAL {
            play.since_reload_check = 0.0;
            let reloaded = play.host.reload_changed(&mut session.scene, play.time, &input);
            for name in reloaded {
                session.log(Level::Info, format!("Reloaded {name}; its scripts restarted."));
            }
        }
        drain(session);
    }
}

/// Moves script output and errors into the console, labelled with the object's name.
fn drain(session: &mut Session) {
    let Some(play) = session.play.as_mut() else { return };
    let logs = play.host.take_logs();
    let faults: Vec<ScriptFault> = play.host.take_faults();
    let name = |session: &Session, id| session.scene.name(id).unwrap_or("Object").to_owned();
    for line in logs {
        let who = name(session, line.object);
        session.log(Level::Info, format!("[{who} · {}] {}", line.script, line.text));
    }
    for fault in faults {
        let who = name(session, fault.object);
        session.log(Level::Error, format!("[{who} · {}] script stopped: {}", fault.script, fault.error));
    }
}

/// Most objects a scene may hold while scripts spawn more.
const MAX_OBJECTS: usize = 100_000;

/// Does what scripts asked for in the last step: destroys and spawns, in order.
fn apply_requests(session: &mut Session) {
    let Some(play) = session.play.as_mut() else { return };
    for request in play.host.take_requests() {
        match request {
            Request::Destroy(object) => {
                if session.scene.contains(object) {
                    let _ = session.scene.delete_object(object);
                }
            }
            Request::Spawn { by, prefab, position } => {
                if let Err(error) = spawn(session, &prefab, position) {
                    let who = session.scene.name(by).unwrap_or("Object").to_owned();
                    session.log(Level::Error, format!("[{who}] could not spawn {prefab}: {error}"));
                }
            }
        }
    }
}

/// Places a linked copy of a prefab at a world position and starts its scripts.
fn spawn(session: &mut Session, prefab: &str, position: [f32; 3]) -> Result<(), String> {
    if session.scene.object_count() >= MAX_OBJECTS {
        return Err(format!("the scene already has {MAX_OBJECTS} objects"));
    }
    let objects = crate::prefab_cmds::read_prefab(session, prefab).map_err(|e| e.to_string())?;
    let id = *session.scene.import(&objects, None, None).map_err(|e| e.to_string())?.first().ok_or("the prefab is empty")?;
    let mut transform = session.scene.get::<bayleaf_scene::components::Transform>(id).cloned().unwrap_or_default();
    transform.position = position;
    session.scene.set(id, transform).map_err(|e| e.to_string())?;
    session.scene.set(id, bayleaf_scene::components::Prefab { source: prefab.to_owned() }).map_err(|e| e.to_string())?;
    // Start the scripts of every object in the new copy.
    let base = session.script_dir();
    let input = std::sync::Arc::clone(&session.input);
    let mut started = Vec::new();
    for (object, _) in session.scene.world_matrices() {
        if object != id && !session.scene.is_inside(object, id) {
            continue;
        }
        for (index, script) in session.scene.get::<bayleaf_scene::components::Scripts>(object).map(|s| s.0.clone()).unwrap_or_default().into_iter().enumerate()
        {
            started.push((object, index, script));
        }
    }
    for (object, index, script) in started {
        let path = session.allowed(&base.join(&script.module), crate::confine::Access::Read).map_err(|e| e.to_string())?;
        let Some(play) = session.play.as_mut() else { return Ok(()) };
        if let Err(error) = play.host.load_file(object, index, &path, &script, &mut session.scene, play.time, &input) {
            let who = session.scene.name(object).unwrap_or("Object").to_owned();
            session.log(Level::Error, format!("[{who} · {}] {error}", script.display_name()));
        }
    }
    Ok(())
}
