//! Play mode: runs the scene's scripts on a copy of the scene and restores the edited scene on stop.

use bayleaf_scene::Scene;
use bayleaf_scene::doc::SceneDoc;
use bayleaf_script::{Request, Runtime, ScriptFault, ScriptHost};
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
    pub(crate) host: ScriptHost,
    pub(crate) time: f64,
    pub(crate) paused: bool,
    /// The scene's particles while it plays.
    pub(crate) particles: bayleaf_particles::Particles,
    since_reload_check: f32,
    /// Bodies of the objects with a Rigid Body or a Collider; `None` when the project turns physics off.
    #[cfg(feature = "physics")]
    physics: Option<bayleaf_physics::Physics>,
    /// Whether play mode already said physics is not there.
    told_no_physics: bool,
    /// Input Map assets read in this session, by path: their maps, or why they could not be read.
    input_assets: InputAssets,
    /// Whether the end of the game was already announced.
    told_outcome: bool,
    /// The saved values as the project holds them; `None` when its file could not be read, so it
    /// is not written over.
    saved_on_disk: Option<std::collections::BTreeMap<String, f64>>,
    /// Whether play mode already said single-precision physics is too far from the centre.
    #[cfg(feature = "physics")]
    told_far: bool,
    /// Sound for this play session (heard or silent).
    #[cfg(feature = "audio")]
    pub(crate) audio: Option<bayleaf_audio::Audio>,
    /// Scenes added while playing (Load Scene with Add), with their top objects, in load order.
    loaded: Vec<(String, Vec<bayleaf_core::StableId>)>,
    /// Streamed Areas that are not away.
    #[cfg(feature = "streaming")]
    pub(crate) streamer: bayleaf_streaming::Streamer,
    /// Whether play mode already said Streamed Areas cannot work (no camera, or no streaming in this build).
    pub(crate) told_streaming: bool,
    /// What scripts and the player changed of each object's Interface.
    pub(crate) interfaces: std::collections::HashMap<bayleaf_core::StableId, crate::interface::InterfaceState>,
    /// Each Interface's layout as read this play (with its parts in place), by object, with the
    /// file it came from; `None` when it did not read.
    pub(crate) interface_roots: std::collections::HashMap<bayleaf_core::StableId, (String, Option<bayleaf_interface::Element>)>,
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "play.start",
            label: "Play",
            menu: "Play/Play",
            shortcut: "Ctrl+P",
            params: "{scripts?: \"interpreted\" | \"compiled\"} (default interpreted); returns {scripts, running, runtime}",
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
            id: "console.read",
            label: "Read Console",
            menu: "",
            shortcut: "",
            params: "{last?: n (default 100), errors?: true for errors only}; returns {lines: [{level, text}], total}: the newest lines, oldest first",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: crate::always,
            run: console_read,
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
            id: "game.state",
            label: "Game State",
            menu: "",
            shortcut: "",
            params: "none; the game's values and each player's (both empty at first), saved values kept between plays (in the project's saves/progress.json), and outcome: \"won\", \"lost\" or null",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: always,
            run: game_state,
        },
        CommandSpec {
            id: "game.state.set",
            label: "Set Game Value",
            menu: "",
            shortcut: "",
            params: "{name, value, player?, saved?}: sets a value in the game's state, a player's (an object with Player Control) or, with saved: true, a saved one; play mode only",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: playing,
            run: game_state_set,
        },
        CommandSpec {
            id: "game.saved.clear",
            label: "Clear Saved Values",
            menu: "",
            shortcut: "",
            params: "none; forgets the values the game saved in the project, and in play mode the ones it holds",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: always,
            run: game_saved_clear,
        },
        CommandSpec {
            id: "physics.raycast",
            label: "Cast Physics Ray",
            menu: "",
            shortcut: "",
            params: "{origin: [x, y, z], direction: [x, y, z], distance? = 1000}; the first collider or character hit (triggers left out), in play mode or in the scene as it is: {hit, object?, name?, distance?, point?, normal?}",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: always,
            run: physics_raycast,
        },
        CommandSpec {
            id: "play.scene.load",
            label: "Load Scene in Play",
            menu: "",
            shortcut: "",
            params: "{path, additive?}: a .bscene relative to the project; added to the playing scene, or in its place (scripts restart, the game state stays); returns {objects}",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: |s| playing(s),
            run: scene_load_command,
        },
        CommandSpec {
            id: "play.scene.unload",
            label: "Unload Scene in Play",
            menu: "",
            shortcut: "",
            params: "{path}: removes the scenes added from that file while playing; returns {removed}",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: |s| playing(s),
            run: scene_unload_command,
        },
        CommandSpec {
            id: "play.scene.list",
            label: "Scenes Added in Play",
            menu: "",
            shortcut: "",
            params: "none; returns {added: [{path, objects}]}",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: |s| playing(s),
            run: scene_list_command,
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
        CommandSpec {
            id: "play.message",
            label: "Send Message",
            menu: "",
            shortcut: "",
            params: "{name, value?, id?}: sends the scripts of object id (or of every object) the message now, as a script's send does",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: |s| if s.is_playing() { Ok(()) } else { Err("messages go to scripts while the game plays".to_owned()) },
            run: message,
        },
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageParams {
    name: String,
    #[serde(default)]
    value: f32,
    #[serde(default)]
    id: Option<bayleaf_core::StableId>,
}

/// Delivers a message to an object's scripts (an Inspector button) or to every script.
fn message(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: MessageParams = parse(params)?;
    let fine = !p.name.is_empty() && p.name.len() <= bayleaf_script::MAX_MESSAGE_NAME && !p.name.chars().any(char::is_control);
    if !fine {
        return Err(CommandError::BadParams(format!("name needs 1 to {} bytes and no control characters", bayleaf_script::MAX_MESSAGE_NAME)));
    }
    if !p.value.is_finite() {
        return Err(CommandError::BadParams("value must be a finite number".to_owned()));
    }
    if let Some(id) = p.id
        && !session.scene.contains(id)
    {
        return Err(CommandError::BadParams(format!("there is no object {id}")));
    }
    let input = std::sync::Arc::clone(&session.input);
    let Some(play) = session.play.as_mut() else { return Err(CommandError::Failed("play mode is not running".to_owned())) };
    match p.id {
        Some(id) => play.host.send(&mut session.scene, id, &p.name, p.value, play.time, &input),
        None => play.host.broadcast(&mut session.scene, &p.name, p.value, play.time, &input),
    }
    apply_requests(session);
    drain(session);
    Ok(json!({ "sent": p.name }))
}

pub(crate) fn playing(session: &Session) -> Result<(), String> {
    if session.is_playing() { Ok(()) } else { Err("play mode is not running".to_owned()) }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoParams {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartParams {
    /// How the scripts run: interpreted (the default) or compiled to machine code.
    #[serde(default)]
    scripts: Runtime,
}

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
    let p: StartParams = parse(params)?;
    session.no_prefab_open("play mode")?;
    let dir = session.script_dir();
    readable_modules(session, &session.scene)?;
    let snapshot = session.scene.to_doc();
    let input = std::sync::Arc::clone(&session.input);
    // Input: the project's Input Map, and objects' own, ready before any script starts. A map that
    // cannot be read is reported, and play runs without it.
    let (mut host, input_assets, input_problems) = fresh_host(session, p.scripts);
    // Saved values from earlier plays, before any script starts.
    let (saved_on_disk, saved_problem) = match crate::saved::read_saved(session) {
        Ok(values) => (Some(values), None),
        Err(problem) => (None, Some(problem)),
    };
    if let Some(values) = &saved_on_disk {
        host.with_state(|state| state.saved = values.clone());
    }
    host.load_scene(&mut session.scene, &dir, 0.0, &input);
    let (scripts, running) = (host.len(), host.running());
    #[cfg(feature = "audio")]
    let sound_open = crate::sound::open(session);
    #[cfg(feature = "physics")]
    let physics = build_physics(session);
    // Particles starting on a model's surface read it from the project.
    let mut particles = bayleaf_particles::Particles::new();
    particles.set_base(session.script_dir());
    session.play = Some(Play {
        #[cfg(feature = "physics")]
        physics,
        told_no_physics: false,
        input_assets,
        told_outcome: false,
        saved_on_disk,
        #[cfg(feature = "physics")]
        told_far: false,
        snapshot,
        host,
        time: 0.0,
        particles,
        interfaces: std::collections::HashMap::new(),
        interface_roots: std::collections::HashMap::new(),
        paused: false,
        since_reload_check: 0.0,
        loaded: Vec::new(),
        #[cfg(feature = "streaming")]
        streamer: bayleaf_streaming::Streamer::new(),
        told_streaming: false,
        #[cfg(feature = "audio")]
        audio: Some(sound_open.0),
    });
    #[cfg(feature = "audio")]
    if let Some(why) = sound_open.1 {
        session.log(Level::Info, why);
    }
    for problem in crate::sound::start_sources(session, None) {
        session.log(Level::Error, problem);
    }
    session.log(Level::Info, format!("Play mode started: {running} of {scripts} scripts running."));
    crate::stream::update(session);
    for problem in input_problems {
        session.log(Level::Error, problem);
    }
    if let Some(problem) = saved_problem {
        session.log(Level::Error, format!("Saved values could not be read, so play starts without them and they are not saved: {problem}"));
    }
    #[cfg(feature = "gameplay")]
    spawn_player(session);
    drain(session);
    let runtime = session.play.as_ref().map(|p| p.host.runtime());
    Ok(json!({ "scripts": scripts, "running": running, "runtime": runtime }))
}

/// Under automation every script module of `scene` must be inside the readable folders.
fn readable_modules(session: &Session, scene: &Scene) -> Result<(), CommandError> {
    let dir = session.script_dir();
    for (id, _) in scene.world_matrices() {
        for script in scene.get::<bayleaf_scene::components::Scripts>(id).map(|s| s.0.clone()).unwrap_or_default() {
            session.allowed(&dir.join(&script.module), crate::confine::Access::Read)?;
        }
    }
    Ok(())
}

/// A script host with the project's Input Map and objects' own, and its data files, before any
/// script starts, and what could not be read.
fn fresh_host(session: &Session, runtime: Runtime) -> (ScriptHost, InputAssets, Vec<String>) {
    let mut problems = Vec::new();
    // Scripts that cannot run as asked are interpreted, and the console says so.
    let mut host = ScriptHost::with_runtime(runtime)
        .unwrap_or_else(|why| {
            problems.push(format!("Scripts are interpreted: {why}."));
            ScriptHost::new()
        })
        .with_natives(session.native_scripts.clone());
    match project_input(session) {
        Ok(maps) => host.with_input(|set| set.project = maps.map(bayleaf_script::actions::InputState::new)),
        Err(problem) => problems.push(format!("The project's Input Map: {problem}")),
    }
    let mut assets = InputAssets::new();
    problems.extend(sync_object_input(session, &host, &mut assets));
    // Data files read as they were saved; play never writes them.
    let (data, data_problems) = crate::data_cmds::load_all(session);
    host.set_data(data);
    problems.extend(data_problems);
    (host, assets, problems)
}

/// The physics world for the playing scene, as Project Settings say; `None` when they turn it off.
#[cfg(feature = "physics")]
fn build_physics(session: &Session) -> Option<bayleaf_physics::Physics> {
    let settings = crate::project_cmds::settings_of(session).physics;
    let precision = match settings.precision {
        crate::project_cmds::PhysicsPrecision::Single => bayleaf_physics::Precision::Single,
        crate::project_cmds::PhysicsPrecision::Double => bayleaf_physics::Precision::Double,
    };
    settings.enabled.then(|| bayleaf_physics::Physics::build(&session.scene, settings.gravity.map(f64::from), precision))
}

/// Reads a scene file of the project for play: a `.bscene`, relative to the project folder.
fn read_scene(session: &Session, relative: &str) -> Result<Scene, String> {
    if !relative.ends_with(".bscene") {
        return Err("give a .bscene file".to_owned());
    }
    let path = session.allowed(&session.script_dir().join(relative), crate::confine::Access::Read).map_err(|e| e.to_string())?;
    let scene = Scene::load(&path).map_err(|e| e.to_string())?;
    readable_modules(session, &scene).map_err(|e| e.to_string())?;
    Ok(scene)
}

/// Adds a scene read while playing to the playing scene, under `parent` (or at the top): its
/// objects join, their scripts and sounds start. Returns the new top objects.
pub(crate) fn add_scene(session: &mut Session, loaded: &Scene, parent: Option<bayleaf_core::StableId>) -> Result<Vec<bayleaf_core::StableId>, String> {
    readable_modules(session, loaded).map_err(|e| e.to_string())?;
    if session.scene.object_count() + loaded.object_count() > MAX_OBJECTS {
        return Err(format!("the scene would have more than {MAX_OBJECTS} objects"));
    }
    let objects = loaded.export(loaded.children(None)).map_err(|e| e.to_string())?;
    let roots = session.scene.import(&objects, parent, None).map_err(|e| e.to_string())?;
    start_scripts_of(session, &roots)?;
    for problem in crate::sound::start_sources(session, Some(&roots)) {
        session.log(Level::Error, problem);
    }
    drain(session);
    Ok(roots)
}

/// Loads a scene while playing. Added (`additive`), its objects join the playing scene, their
/// scripts start, and Unload Scene removes them again. Otherwise it takes the playing scene's
/// place: every script starts again with the game's state kept, and physics starts over. Stop
/// still brings back the scene as it was before play. Returns the new top objects.
pub(crate) fn load_scene(session: &mut Session, relative: &str, additive: bool) -> Result<Vec<bayleaf_core::StableId>, String> {
    if session.play.is_none() {
        return Err("play mode is not running".to_owned());
    }
    let loaded = read_scene(session, relative)?;
    if additive {
        let roots = add_scene(session, &loaded, None)?;
        if let Some(play) = session.play.as_mut() {
            play.loaded.push((relative.to_owned(), roots.clone()));
        }
        session.log(Level::Info, format!("Added the scene {relative}."));
        return Ok(roots);
    }
    // What the scripts said so far reaches the console before their host goes.
    drain(session);
    let state = session.play.as_ref().map(|p| p.host.state()).unwrap_or_default();
    session.scene = loaded;
    let input = std::sync::Arc::clone(&session.input);
    let runtime = session.play.as_ref().map_or_else(Runtime::default, |p| p.host.runtime());
    let (mut host, input_assets, problems) = fresh_host(session, runtime);
    host.with_state(|s| *s = state);
    let time = session.play.as_ref().map_or(0.0, |p| p.time);
    let dir = session.script_dir();
    host.load_scene(&mut session.scene, &dir, time, &input);
    #[cfg(feature = "physics")]
    let physics = build_physics(session);
    if let Some(play) = session.play.as_mut() {
        play.host = host;
        play.input_assets = input_assets;
        play.loaded.clear();
        crate::stream::clear(play);
        #[cfg(feature = "audio")]
        play.audio.as_ref().inspect(|audio| audio.with(bayleaf_audio::Mixer::stop_all));
        #[cfg(feature = "physics")]
        {
            play.physics = physics;
        }
    }
    for problem in problems {
        session.log(Level::Error, problem);
    }
    session.log(Level::Info, format!("Loaded the scene {relative}."));
    for problem in crate::sound::start_sources(session, None) {
        session.log(Level::Error, problem);
    }
    #[cfg(feature = "gameplay")]
    spawn_player(session);
    drain(session);
    Ok(session.scene.children(None).to_vec())
}

/// Removes every scene added from `relative` while playing. Returns how many objects went.
pub(crate) fn unload_scene(session: &mut Session, relative: &str) -> Result<usize, String> {
    let play = session.play.as_mut().ok_or("play mode is not running")?;
    let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut play.loaded).into_iter().partition(|(path, _)| path == relative);
    play.loaded = kept;
    if gone.is_empty() {
        return Err(format!("{relative} was not added while playing"));
    }
    let mut removed = 0;
    for root in gone.into_iter().flat_map(|(_, roots)| roots) {
        if session.scene.contains(root) {
            removed += session.scene.delete_object(root).unwrap_or(0);
        }
    }
    session.log(Level::Info, format!("Removed the scene {relative}."));
    Ok(removed)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneLoadParams {
    path: String,
    #[serde(default)]
    additive: bool,
}

fn scene_load_command(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: SceneLoadParams = parse(params)?;
    let objects = load_scene(session, &p.path, p.additive).map_err(CommandError::Failed)?;
    Ok(json!({ "objects": objects }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneUnloadParams {
    path: String,
}

fn scene_unload_command(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: SceneUnloadParams = parse(params)?;
    let removed = unload_scene(session, &p.path).map_err(CommandError::Failed)?;
    Ok(json!({ "removed": removed }))
}

fn scene_list_command(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    parse::<NoParams>(params)?;
    let added: Vec<Value> =
        session.play.as_ref().map(|p| p.loaded.iter().map(|(path, objects)| json!({ "path": path, "objects": objects })).collect()).unwrap_or_default();
    Ok(json!({ "added": added }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConsoleReadParams {
    #[serde(default = "hundred")]
    last: usize,
    #[serde(default)]
    errors: bool,
}

fn hundred() -> usize {
    100
}

/// The newest console lines (script output, play messages, errors), oldest first.
fn console_read(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: ConsoleReadParams = parse(params)?;
    let lines: Vec<&crate::ConsoleLine> = session.console().iter().filter(|l| !p.errors || l.level == Level::Error).collect();
    let start = lines.len().saturating_sub(p.last);
    Ok(json!({ "lines": lines[start..], "total": lines.len() }))
}

fn stop(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    parse::<NoParams>(params)?;
    keep_saved(session);
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
        "particles": session.play.as_ref().map_or(0, |p| p.particles.count(None)),
        "console": tail,
    }))
}

/// Advances play mode by `dt` seconds, split into steps of at most [`MAX_STEP`].
pub(crate) fn step(session: &mut Session, dt: f32) {
    let started = bayleaf_core::time::Instant::now();
    // Scripts are timed one by one only while the profiler records.
    let recording = session.profiler.is_recording();
    if let Some(play) = session.play.as_mut() {
        play.host.set_timing(recording);
    }
    step_parts(session, dt);
    // Each script's time, and the whole step, for the profiler.
    let timings = session.play.as_mut().map(|p| p.host.take_timings()).unwrap_or_default();
    let time = session.play_time();
    session.profiler.add_scripts(timings);
    session.profiler.end_step(time, started.elapsed());
}

/// Times one part of the step for the profiler.
fn timed<T>(session: &mut Session, part: crate::profiler::Part, run: impl FnOnce(&mut Session) -> T) -> T {
    let started = bayleaf_core::time::Instant::now();
    let out = run(session);
    session.profiler.add(part, started.elapsed());
    out
}

fn step_parts(session: &mut Session, dt: f32) {
    use crate::profiler::Part;
    let mut remaining = dt.max(0.0);
    while remaining > 0.0 {
        let dt = remaining.min(MAX_STEP);
        remaining -= dt;
        let input = std::sync::Arc::clone(&session.input);
        // Objects that came or went (spawned, destroyed) get or drop their own Input Map.
        let mut assets = match session.play.as_mut() {
            Some(play) => std::mem::take(&mut play.input_assets),
            None => return,
        };
        let problems = session.play.as_ref().map(|play| sync_object_input(session, &play.host, &mut assets)).unwrap_or_default();
        if let Some(play) = session.play.as_mut() {
            play.input_assets = assets;
        }
        for problem in problems {
            session.log(Level::Error, problem);
        }
        let Some(play) = session.play.as_mut() else { return };
        play.time += f64::from(dt);
        play.host.with_input(|set| set.step(&input));
        #[cfg(feature = "physics")]
        if let Some(physics) = &play.physics {
            play.host.set_velocities(physics.velocities());
            play.host.set_grounded(physics.grounded());
        }
        #[cfg(all(feature = "physics", feature = "gameplay"))]
        drive_players(play, &mut session.scene, &input);
        // Scripts read the interfaces as they show at the step's start.
        crate::interface::refresh_values(session);
        let Some(play) = session.play.as_mut() else { return };
        let scripts = bayleaf_core::time::Instant::now();
        play.host.update(&mut session.scene, dt, play.time, &input);
        session.profiler.add(Part::Scripts, scripts.elapsed());
        apply_requests(session);
        timed(session, Part::Particles, |session| {
            if let Some(play) = session.play.as_mut() {
                play.particles.step(&session.scene, dt);
            }
        });
        let physics = bayleaf_core::time::Instant::now();
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
        session.profiler.add(Part::Physics, physics.elapsed());
        // The camera follows after physics moved the player.
        #[cfg(feature = "gameplay")]
        if let Some((camera, placed)) = bayleaf_gameplay::controller::follow_camera(&session.scene) {
            let _ = session.scene.set(camera, placed);
        }
        timed(session, Part::Sound, |session| crate::sound::update(session, f64::from(dt)));
        timed(session, Part::Streaming, crate::stream::update);
        #[cfg(feature = "gameplay")]
        apply_rules(session);
        report_outcome(session, &input);
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
/// After a message sent from outside a step: what its scripts asked for, and what they logged.
pub(crate) fn after_message(session: &mut Session) {
    apply_requests(session);
    drain(session);
}

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
            Request::Particles { object, action, .. } => {
                let Some(play) = session.play.as_mut() else { continue };
                match action {
                    bayleaf_script::ParticleAction::Play => play.particles.play(object),
                    bayleaf_script::ParticleAction::Stop => play.particles.stop(object),
                    bayleaf_script::ParticleAction::Burst(count) => play.particles.burst(&session.scene, object, count),
                }
            }
            Request::Interface { by, object, element, change } => crate::interface::apply(session, by, object, element, change),
            Request::PlaySound { by, object } => {
                if let Err(error) = crate::sound::play_object(session, object) {
                    sound_problem(session, by, object, &error);
                }
            }
            Request::StopSound { by, object } => {
                if let Err(error) = crate::sound::stop_object(session, object) {
                    sound_problem(session, by, object, &error);
                }
            }
            Request::PlaySoundAt { by, path, position, volume } => {
                if let Err(error) = crate::sound::play_at(session, &path, position, volume) {
                    let who = session.scene.name(by).unwrap_or("Object").to_owned();
                    session.log(Level::Error, format!("[{who}] cannot play {path}: {error}"));
                }
            }
            Request::LoadScene { by, path, additive } => {
                if let Err(error) = load_scene(session, &path, additive) {
                    let who = session.scene.name(by).unwrap_or("Object").to_owned();
                    session.log(Level::Error, format!("[{who}] could not load the scene {path}: {error}"));
                }
            }
            Request::UnloadScene { by, path } => {
                if let Err(error) = unload_scene(session, &path) {
                    let who = session.scene.name(by).unwrap_or("Object").to_owned();
                    session.log(Level::Error, format!("[{who}] could not unload the scene {path}: {error}"));
                }
            }
            Request::Spawn { by, prefab, position } => {
                if let Err(error) = spawn(session, &prefab, position) {
                    let who = session.scene.name(by).unwrap_or("Object").to_owned();
                    session.log(Level::Error, format!("[{who}] could not spawn {prefab}: {error}"));
                }
            }
            Request::SetField { by, object, component, field, value } => {
                // The field keeps its shape (a colour keeps its alpha); the component checks the value.
                let updated = session.scene.component(object, component).ok().and_then(|mut current| {
                    let new = bayleaf_script::with_field(current.get(&field)?, value)?;
                    current[field.as_str()] = new;
                    Some(current)
                });
                let result = match updated {
                    Some(value) => session.scene.set_component(object, component, value).map_err(|e| e.to_string()),
                    None => Err(format!("it has no {component} field {field}")),
                };
                if let Err(error) = result {
                    let who = session.scene.name(by).unwrap_or("Object").to_owned();
                    let target = session.scene.name(object).unwrap_or("Object").to_owned();
                    session.log(Level::Error, format!("[{who}] could not set {field} of {target}'s {component}: {error}"));
                }
            }
        }
    }
}

/// Says once per play session why a script's sound request did nothing.
fn sound_problem(session: &mut Session, by: bayleaf_core::StableId, object: bayleaf_core::StableId, error: &str) {
    let who = session.scene.name(by).unwrap_or("Object").to_owned();
    let target = session.scene.name(object).unwrap_or("Object").to_owned();
    session.log(Level::Error, format!("[{who}] cannot play the sound of {target}: {error}"));
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

/// Input Map assets read in a play session, by path: their maps, or why they could not be read.
type InputAssets = std::collections::HashMap<String, Result<std::sync::Arc<bayleaf_script::actions::InputMaps>, String>>;

/// The project's Input Map: the asset Project Settings names, actions still in an old project
/// file, or for a scene outside any project the usual controls. `None` when the project has none.
fn project_input(session: &Session) -> Result<Option<std::sync::Arc<bayleaf_script::actions::InputMaps>>, String> {
    if session.project_dir().is_none() {
        return crate::input_cmds::InputAsset::starter().maps().map(|m| Some(std::sync::Arc::new(m)));
    }
    let settings = crate::project_cmds::settings_of(session).input;
    if settings.map.is_empty() {
        return match settings.legacy_asset() {
            Some(asset) => asset.maps().map(|m| Some(std::sync::Arc::new(m))),
            None => Ok(None),
        };
    }
    crate::input_cmds::load_maps(session, std::path::Path::new(&settings.map)).map(Some)
}

/// Gives each object with an Input Map component its own map state (starting with the
/// component's map when it names one), and drops those of objects that are gone. Returns what
/// went wrong with assets read for the first time and with starting maps.
fn sync_object_input(session: &Session, host: &ScriptHost, assets: &mut InputAssets) -> Vec<String> {
    let mut problems = Vec::new();
    let wanted: Vec<(bayleaf_core::StableId, bayleaf_scene::components::InputMap)> = session
        .scene
        .world_matrices()
        .into_iter()
        .filter_map(|(id, _)| session.scene.get::<bayleaf_scene::components::InputMap>(id).filter(|c| !c.asset.is_empty()).map(|c| (id, c.clone())))
        .collect();
    host.with_input(|set| {
        set.objects.retain(|id, _| wanted.iter().any(|(w, _)| w == id));
        for (id, component) in wanted {
            if set.objects.contains_key(&id) {
                continue;
            }
            let maps = assets.entry(component.asset.clone()).or_insert_with(|| {
                let loaded = crate::input_cmds::load_maps(session, std::path::Path::new(&component.asset));
                if let Err(problem) = &loaded {
                    problems.push(format!("Input Map {}: {problem}", component.asset));
                }
                loaded
            });
            let Ok(maps) = maps else { continue };
            let mut state = bayleaf_script::actions::InputState::new(std::sync::Arc::clone(maps));
            if !component.start.is_empty() && !state.set_active(&component.start) {
                let name = session.scene.name(id).unwrap_or("Object");
                problems.push(format!("[{name}] Input Map {} has no map {}; it starts with {}", component.asset, component.start, state.active()));
            }
            set.objects.insert(id, state);
        }
    });
    problems
}

/// Player Control: steers each controlled Character with the input actions.
#[cfg(all(feature = "physics", feature = "gameplay"))]
fn drive_players(play: &mut Play, scene: &mut Scene, input: &bayleaf_script::Input) {
    let Some(physics) = play.physics.as_mut() else { return };
    // Each player reads its own Input Map when it has one, else the project's.
    let commands = play.host.with_input(|set| {
        let value = |id, name: &str| set.of(id).map_or(0.0, |maps| maps.value(name, input));
        let pressed = |id, name: &str| set.of(id).is_some_and(|maps| maps.pressed(name));
        bayleaf_gameplay::controller::drive(scene, &value, &pressed)
    });
    for command in commands {
        physics.walk(command.object, command.walk);
        if let Some(speed) = command.jump {
            physics.jump(command.object, speed);
        }
        if let Some(yaw) = command.facing
            && let Some(mut transform) = scene.get::<bayleaf_scene::components::Transform>(command.object).cloned()
        {
            transform.rotation[1] = yaw;
            let _ = scene.set(command.object, transform);
        }
    }
}

/// Game Rules: places the player prefab at the Player Start when the scene has no player yet.
#[cfg(feature = "gameplay")]
fn spawn_player(session: &mut Session) {
    let Some(rules) = bayleaf_gameplay::rules::rules(&session.scene) else { return };
    if rules.player_prefab.is_empty() || !bayleaf_gameplay::rules::players(&session.scene).is_empty() {
        return;
    }
    let start = bayleaf_gameplay::rules::start(&session.scene);
    let placed = spawn_with(session, &rules.player_prefab, |transform| {
        let scale = transform.scale;
        *transform = bayleaf_scene::components::Transform::from_matrix(start);
        transform.scale = scale;
    });
    if let Err(error) = placed {
        session.log(Level::Error, format!("Game Rules could not place the player {}: {error}", rules.player_prefab));
    }
}

/// Game Rules each step: fallen players start over at the Player Start; the time limit ends the game.
#[cfg(feature = "gameplay")]
fn apply_rules(session: &mut Session) {
    let Some(rules) = bayleaf_gameplay::rules::rules(&session.scene) else { return };
    let mut restarted = Vec::new();
    for id in bayleaf_gameplay::rules::fallen(&session.scene, &rules) {
        let Some(placed) = bayleaf_gameplay::rules::placed_at_start(&session.scene, id) else { continue };
        let _ = session.scene.set(id, placed);
        #[cfg(feature = "physics")]
        if let (Some(play), Some(world)) = (session.play.as_mut(), session.scene.world_matrix(id))
            && let Some(physics) = play.physics.as_mut()
        {
            physics.teleport(id, world);
        }
        restarted.push(session.scene.name(id).unwrap_or("Player").to_owned());
    }
    for name in restarted {
        session.log(Level::Info, format!("[{name}] fell and starts over at the Player Start"));
    }
    let Some(play) = session.play.as_mut() else { return };
    if bayleaf_gameplay::rules::time_up(&rules, play.time) {
        let won = rules.time_up == bayleaf_scene::components::TimeUp::Win;
        play.host.with_state(|state| state.end(won));
    }
}

/// Announces the end of the game once: in the console and to every script (gameWon or gameLost).
fn report_outcome(session: &mut Session, input: &std::sync::Arc<bayleaf_script::Input>) {
    let Some(play) = session.play.as_mut() else { return };
    let Some(won) = play.host.with_state(|state| state.outcome) else { return };
    if std::mem::replace(&mut play.told_outcome, true) {
        return;
    }
    play.host.broadcast(&mut session.scene, if won { "gameWon" } else { "gameLost" }, 0.0, play.time, input);
    session.log(Level::Info, if won { "Game won." } else { "Game lost." }.to_owned());
    apply_requests(session);
    keep_saved(session);
}

impl Session {
    /// The scene's particles while it plays.
    pub fn particles(&self) -> Option<&bayleaf_particles::Particles> {
        self.play.as_ref().map(|p| &p.particles)
    }

    /// The game's state while it plays: the game's values, each player's, the saved ones and how
    /// the game ended. `None` outside play mode.
    pub fn game_state(&self) -> Option<bayleaf_script::GameState> {
        self.play.as_ref().map(|play| play.host.state())
    }

    /// Writes the saved values now when scripts changed them, as stopping does: for a player that
    /// may be closed without stopping the game (a browser tab).
    pub fn keep_saved_values(&mut self) {
        keep_saved(self);
    }
}

/// Writes the saved values to the project when scripts changed them. Outside a project they last
/// for this play only, which the console says once.
fn keep_saved(session: &mut Session) {
    let Some(play) = session.play.as_mut() else { return };
    let saved = play.host.with_state(|state| state.saved.clone());
    let Some(on_disk) = play.saved_on_disk.as_mut() else { return };
    if *on_disk == saved {
        return;
    }
    *on_disk = saved.clone();
    if session.project_dir().is_none() {
        session.log(Level::Info, "Saved values are kept only in a project; open or make one to keep them between plays.".to_owned());
        return;
    }
    if let Err(error) = crate::saved::write_saved(session, &saved) {
        session.log(Level::Error, format!("Saved values could not be written: {error}"));
    }
}

/// The game's state as JSON: the game's values, each player's, the saved ones and how the game
/// ended. Outside play mode only saved values exist, as the project holds them.
fn state_json(session: &Session) -> Value {
    let (state, problem) = match session.play.as_ref() {
        Some(play) => (play.host.state(), None),
        None => match crate::saved::read_saved(session) {
            Ok(saved) => (bayleaf_script::GameState { saved, ..Default::default() }, None),
            Err(problem) => (bayleaf_script::GameState::default(), Some(problem)),
        },
    };
    let players: Vec<Value> = state.players.iter().map(|(id, values)| json!({ "object": id, "name": session.scene.name(*id), "values": values })).collect();
    let outcome = state.outcome.map(|won| if won { "won" } else { "lost" });
    let file = crate::saved::saved_path(session);
    json!({
        "playing": session.is_playing(),
        "game": state.game,
        "players": players,
        "saved": state.saved,
        "savedFile": file,
        "savedProblem": problem,
        "outcome": outcome,
    })
}

fn game_state(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    parse::<NoParams>(params)?;
    Ok(state_json(session))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StateSetParams {
    name: String,
    value: f64,
    /// A player object; the game's own state when left out.
    #[serde(default)]
    player: Option<bayleaf_core::StableId>,
    /// A saved value, kept between plays.
    #[serde(default)]
    saved: bool,
}

fn game_state_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: StateSetParams = parse(params)?;
    let scope = match (p.player, p.saved) {
        (Some(_), true) => return Err(CommandError::BadParams("give player or saved, not both".to_owned())),
        (Some(player), false) if session.scene.get::<bayleaf_scene::components::PlayerControl>(player).is_none() => {
            return Err(CommandError::BadParams(format!("{player} is not a player (an object with Player Control)")));
        }
        (Some(player), false) => bayleaf_script::Scope::Player(player),
        (None, true) => bayleaf_script::Scope::Saved,
        (None, false) => bayleaf_script::Scope::Game,
    };
    let play = session.play.as_mut().ok_or_else(|| CommandError::Failed("the game state exists only in play mode".to_owned()))?;
    if !play.host.with_state(|state| state.set(scope, &p.name, p.value)) {
        return Err(CommandError::BadParams(format!(
            "give a name of 1 to 64 bytes and a finite number; a state holds at most {} values",
            bayleaf_script::MAX_STATE_VALUES
        )));
    }
    Ok(state_json(session))
}

/// Forgets the project's saved values, and in play mode the ones the game holds.
fn game_saved_clear(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    parse::<NoParams>(params)?;
    let removed = crate::saved::clear_saved(session).map_err(CommandError::Failed)?;
    if let Some(play) = session.play.as_mut() {
        play.host.with_state(|state| state.saved.clear());
        play.saved_on_disk = Some(std::collections::BTreeMap::new());
    }
    session.log(Level::Info, "Saved values cleared.".to_owned());
    Ok(json!({ "removed": removed }))
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
fn spawn(session: &mut Session, prefab: &str, position: [f32; 3]) -> Result<bayleaf_core::StableId, String> {
    spawn_with(session, prefab, |transform| transform.position = position.map(f64::from))
}

/// Places a linked copy of a prefab, `place` setting its transform, and starts its scripts.
fn spawn_with(session: &mut Session, prefab: &str, place: impl FnOnce(&mut bayleaf_scene::components::Transform)) -> Result<bayleaf_core::StableId, String> {
    if session.scene.object_count() >= MAX_OBJECTS {
        return Err(format!("the scene already has {MAX_OBJECTS} objects"));
    }
    let objects = crate::prefab_cmds::read_prefab(session, prefab).map_err(|e| e.to_string())?;
    let id = *session.scene.import(&objects, None, None).map_err(|e| e.to_string())?.first().ok_or("the prefab is empty")?;
    let mut transform = session.scene.get::<bayleaf_scene::components::Transform>(id).cloned().unwrap_or_default();
    place(&mut transform);
    session.scene.set(id, transform).map_err(|e| e.to_string())?;
    session.scene.set(id, bayleaf_scene::components::Prefab::new(prefab)).map_err(|e| e.to_string())?;
    start_scripts_of(session, &[id])?;
    for problem in crate::sound::start_sources(session, Some(&[id])) {
        session.log(Level::Error, problem);
    }
    Ok(id)
}

/// Starts the scripts of `roots` and everything under them: all load, then all run On Init, then
/// On Start.
fn start_scripts_of(session: &mut Session, roots: &[bayleaf_core::StableId]) -> Result<(), String> {
    let base = session.script_dir();
    let input = std::sync::Arc::clone(&session.input);
    let mut scripts = Vec::new();
    for (object, _) in session.scene.world_matrices() {
        if !roots.iter().any(|root| object == *root || session.scene.is_inside(object, *root)) {
            continue;
        }
        for (index, script) in session.scene.get::<bayleaf_scene::components::Scripts>(object).map(|s| s.0.clone()).unwrap_or_default().into_iter().enumerate()
        {
            let path = session.allowed(&base.join(&script.module), crate::confine::Access::Read).map_err(|e| e.to_string())?;
            scripts.push((object, index, path, script));
        }
    }
    let Some(play) = session.play.as_mut() else { return Ok(()) };
    for (object, index, error) in play.host.load_files(&scripts, &mut session.scene, play.time, &input) {
        let who = session.scene.name(object).unwrap_or("Object").to_owned();
        let script = scripts.iter().find(|(o, i, _, _)| (*o, *i) == (object, index)).map(|(_, _, _, s)| s.display_name()).unwrap_or_default();
        session.log(Level::Error, format!("[{who} · {script}] {error}"));
    }
    Ok(())
}
