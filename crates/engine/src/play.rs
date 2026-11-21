//! Play mode: runs the scene's scripts on a copy of the scene and restores the edited scene on stop.

use bayleaf_scene::Scene;
use bayleaf_scene::doc::SceneDoc;
use bayleaf_script::{Request, ScriptFault, ScriptHost};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{CommandError, CommandSpec, Level, Mutates, Prompt, Session, always, parse};

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
    /// Bodies of the objects with a Rigid Body or a Collider; `None` when the project turns physics off.
    #[cfg(feature = "physics")]
    physics: Option<bayleaf_physics::Physics>,
    /// Whether play mode already said physics is not there.
    told_no_physics: bool,
    /// Whether play mode already said single-precision physics is too far from the centre.
    #[cfg(feature = "physics")]
    told_far: bool,
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
            params: "{keys: [names], pointer?: [x, y] (-1 to 1, +y up; null when outside the view), buttons?: n (bit 0 left, 1 right, 2 middle), pad?: {axes: [left x, left y, right x, right y, left trigger, right trigger], buttons: [South, East, …]}}; what scripts see as held, until the next call or the next editor frame",
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
            id: "physics.raycast",
            label: "Physics Raycast",
            menu: "",
            shortcut: "",
            params: "{origin: [x, y, z], direction: [x, y, z], distance? = 1000}; the first collider or character hit (triggers left out), in play mode or in the scene as it is: {hit, object?, name?, distance?, point?, normal?}",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: always,
            run: physics_raycast,
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
    #[cfg(feature = "physics")]
    let physics = {
        let settings = crate::project_cmds::settings_of(session).physics;
        let precision = match settings.precision {
            crate::project_cmds::PhysicsPrecision::Single => bayleaf_physics::Precision::Single,
            crate::project_cmds::PhysicsPrecision::Double => bayleaf_physics::Precision::Double,
        };
        settings.enabled.then(|| bayleaf_physics::Physics::build(&session.scene, settings.gravity.map(f64::from), precision))
    };
    session.play = Some(Play {
        #[cfg(feature = "physics")]
        physics,
        told_no_physics: false,
        #[cfg(feature = "physics")]
        told_far: false,
        snapshot,
        host,
        time: 0.0,
        paused: false,
        since_reload_check: 0.0,
    });
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
    /// `{axes: [6 numbers], buttons: [names]}`; leave out when no gamepad is connected.
    #[serde(default)]
    pad: Option<PadParams>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PadParams {
    #[serde(default)]
    axes: [f32; 6],
    #[serde(default)]
    buttons: Vec<String>,
}

fn input(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: InputParams = parse(params)?;
    if p.keys.len() > 64 || p.keys.iter().any(|k| k.len() > 32) {
        return Err(CommandError::BadParams("at most 64 keys of at most 32 characters".to_owned()));
    }
    if p.pointer.is_some_and(|[x, y]| !(x.is_finite() && y.is_finite())) {
        return Err(CommandError::BadParams("pointer must be two finite numbers".to_owned()));
    }
    let pad = match p.pad {
        Some(pad) => {
            let mut bits = 0u32;
            for name in &pad.buttons {
                let index = bayleaf_script::PAD_BUTTONS
                    .iter()
                    .position(|b| b == name)
                    .ok_or_else(|| CommandError::BadParams(format!("unknown gamepad button {name:?}; use {}", bayleaf_script::PAD_BUTTONS.join(", "))))?;
                bits |= 1 << index;
            }
            Some(bayleaf_script::Pad { axes: pad.axes, buttons: bits })
        }
        None => None,
    };
    session.set_pad(pad);
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
        #[cfg(feature = "physics")]
        if let Some(physics) = &play.physics {
            play.host.set_velocities(physics.velocities());
            play.host.set_grounded(physics.grounded());
        }
        play.host.update(&mut session.scene, dt, play.time, &input);
        apply_requests(session);
        #[cfg(feature = "physics")]
        if let Some(play) = session.play.as_mut()
            && let Some(physics) = play.physics.as_mut()
        {
            let contacts = physics.step(&mut session.scene, dt);
            let named: Vec<(bayleaf_core::StableId, &str, bayleaf_core::StableId)> =
                contacts.iter().map(|c| (c.object, contact_name(c.kind), c.other)).collect();
            let far = physics.out_of_range() && !std::mem::replace(&mut play.told_far, true);
            play.host.notify(&mut session.scene, &named, play.time, &input);
            if far {
                session.log(
                    Level::Info,
                    "Warning: physics bodies are more than 10 km from the world's centre, where single precision loses accuracy; set Physics Precision to Double in Project Settings.".to_owned(),
                );
            }
            apply_requests(session);
        }
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
            Request::Push { by, object, impulse } => physics_request(session, by, object, "has no dynamic Rigid Body to push", |_p| {
                #[cfg(feature = "physics")]
                return _p.physics.as_mut().map(|p| p.apply_impulse(object, impulse));
                #[cfg(not(feature = "physics"))]
                return {
                    let _ = impulse;
                    None
                };
            }),
            Request::SetVelocity { by, object, velocity } => physics_request(session, by, object, "has no dynamic Rigid Body to push", |_p| {
                #[cfg(feature = "physics")]
                return _p.physics.as_mut().map(|p| p.set_velocity(object, velocity));
                #[cfg(not(feature = "physics"))]
                return {
                    let _ = velocity;
                    None
                };
            }),
            Request::Walk { by, object, velocity } => physics_request(session, by, object, "has no Character to walk", |_p| {
                #[cfg(feature = "physics")]
                return _p.physics.as_mut().map(|p| p.walk(object, velocity));
                #[cfg(not(feature = "physics"))]
                return {
                    let _ = velocity;
                    None
                };
            }),
            Request::Jump { by, object, speed } => physics_request(session, by, object, "has no Character to jump", |_p| {
                // A character in the air does not jump; that is not a mistake.
                #[cfg(feature = "physics")]
                return _p.physics.as_mut().map(|p| {
                    p.jump(object, speed);
                    p.is_character(object)
                });
                #[cfg(not(feature = "physics"))]
                return {
                    let _ = speed;
                    None
                };
            }),
            Request::Spawn { by, prefab, position } => {
                if let Err(error) = spawn(session, &prefab, position) {
                    let who = session.scene.name(by).unwrap_or("Object").to_owned();
                    session.log(Level::Error, format!("[{who}] could not spawn {prefab}: {error}"));
                }
            }
        }
    }
}

/// Applies a script's push or velocity change: `apply` gives `None` without physics (a build
/// without it, or a project that turns it off; said once per play session) and `Some(false)` when
/// the object cannot take it (`missing` says why).
fn physics_request(
    session: &mut Session,
    by: bayleaf_core::StableId,
    object: bayleaf_core::StableId,
    missing: &str,
    apply: impl FnOnce(&mut Play) -> Option<bool>,
) {
    let Some(play) = session.play.as_mut() else { return };
    let who = session.scene.name(by).unwrap_or("Object").to_owned();
    match apply(play) {
        Some(true) => {}
        Some(false) => {
            let target = session.scene.name(object).unwrap_or("Object").to_owned();
            session.log(Level::Error, format!("[{who}] {target} {missing}"));
        }
        None => {
            if std::mem::replace(&mut play.told_no_physics, true) {
                return;
            }
            let why = if cfg!(feature = "physics") { "physics is off in the project settings" } else { "this build has no physics" };
            session.log(Level::Error, format!("[{who}] {why}; impulses and velocities do nothing"));
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RaycastParams {
    origin: [f64; 3],
    direction: [f64; 3],
    #[serde(default = "default_ray_distance")]
    distance: f64,
}

fn default_ray_distance() -> f64 {
    1000.0
}

fn physics_raycast(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: RaycastParams = parse(params)?;
    let finite = p.origin.iter().chain(&p.direction).all(|v| v.is_finite()) && p.distance.is_finite() && p.distance >= 0.0;
    if !finite || p.direction.iter().all(|v| *v == 0.0) {
        return Err(CommandError::BadParams("give a finite origin, a direction that is not zero, and a distance of 0 or more".to_owned()));
    }
    cast(session, p)
}

#[cfg(feature = "physics")]
fn cast(session: &mut Session, p: RaycastParams) -> Result<Value, CommandError> {
    use bayleaf_core::math::DVec3;
    let (origin, direction) = (DVec3::from(p.origin), DVec3::from(p.direction));
    let hit = match session.play.as_ref().and_then(|play| play.physics.as_ref()) {
        Some(physics) => physics.raycast(origin, direction, p.distance, None),
        None => {
            let settings = crate::project_cmds::settings_of(session).physics;
            if !settings.enabled {
                return Err(CommandError::Failed("physics is off in the project settings".to_owned()));
            }
            let precision = match settings.precision {
                crate::project_cmds::PhysicsPrecision::Single => bayleaf_physics::Precision::Single,
                crate::project_cmds::PhysicsPrecision::Double => bayleaf_physics::Precision::Double,
            };
            bayleaf_physics::Physics::build(&session.scene, settings.gravity.map(f64::from), precision).raycast(origin, direction, p.distance, None)
        }
    };
    Ok(match hit {
        Some(hit) => json!({
            "hit": true,
            "object": hit.object,
            "name": session.scene.name(hit.object),
            "distance": hit.distance,
            "point": hit.point.to_array(),
            "normal": hit.normal.to_array(),
        }),
        None => json!({ "hit": false }),
    })
}

#[cfg(not(feature = "physics"))]
fn cast(_: &mut Session, p: RaycastParams) -> Result<Value, CommandError> {
    let _ = p.distance;
    Err(CommandError::Failed("this build has no physics".to_owned()))
}

/// The message name scripts get for a physics contact.
#[cfg(feature = "physics")]
fn contact_name(kind: bayleaf_physics::ContactKind) -> &'static str {
    match kind {
        bayleaf_physics::ContactKind::CollisionStart => "collision",
        bayleaf_physics::ContactKind::CollisionEnd => "collisionEnd",
        bayleaf_physics::ContactKind::TriggerEnter => "triggerEnter",
        bayleaf_physics::ContactKind::TriggerExit => "triggerExit",
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
    transform.position = position.map(f64::from);
    session.scene.set(id, transform).map_err(|e| e.to_string())?;
    session.scene.set(id, bayleaf_scene::components::Prefab::new(prefab)).map_err(|e| e.to_string())?;
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
