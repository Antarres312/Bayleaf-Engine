//! Keyframe animations in play mode: each object's Keyframe Animation component plays its
//! `.btimeline` file on the parts of the prefab the object is the root of.
//!
//! A part is found by the prefab file's id: the root's own copy holds the file's objects in the
//! same hierarchy order. Animation states can drive the same files through
//! [`bayleaf_scene::timeline::Timeline::sample`] and [`bayleaf_scene::timeline::apply`].

use std::collections::HashMap;
use std::sync::Arc;

use bayleaf_core::StableId;
use bayleaf_scene::components::{KeyframeAnimation, Prefab};
use bayleaf_scene::timeline::{self, Binding, Timeline};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{CommandError, CommandSpec, Level, Mutates, Prompt, Session, parse};

/// One object's Keyframe Animation while the game plays.
struct Player {
    root: StableId,
    timeline: Arc<Timeline>,
    binding: Binding,
    /// Seconds played since it started.
    time: f64,
    playing: bool,
    /// Whether its problems were already reported.
    told: bool,
}

/// The Keyframe Animations of the playing scene.
#[derive(Default)]
pub(crate) struct Players {
    players: Vec<Player>,
    /// Timeline files read this play, by path: the timeline, or why it could not be read.
    files: HashMap<String, Result<Arc<Timeline>, String>>,
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    let spec = |id, label, params, run| CommandSpec {
        id,
        label,
        menu: "",
        shortcut: "",
        params,
        prompt: Prompt::None,
        mutates: Mutates::No,
        enabled: crate::play::playing,
        run,
    };
    vec![
        spec("timeline.play", "Play Keyframe Animation", "{id}: in play mode, plays the object's Keyframe Animation from its start", play_command),
        spec("timeline.stop", "Stop Keyframe Animation", "{id}: in play mode, stops the object's Keyframe Animation where it is", stop_command),
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdParams {
    id: StableId,
}

fn play_command(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: IdParams = parse(params)?;
    set_playing(session, p.id, true).map_err(CommandError::Failed)?;
    Ok(json!({}))
}

fn stop_command(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: IdParams = parse(params)?;
    set_playing(session, p.id, false).map_err(CommandError::Failed)?;
    Ok(json!({}))
}

/// Plays the Keyframe Animation of `object` from its start, or stops it where it is.
pub(crate) fn set_playing(session: &mut Session, object: StableId, play: bool) -> Result<(), String> {
    if !session.play.as_ref().is_some_and(|p| p.timelines.players.iter().any(|t| t.root == object)) {
        // Added since it started, or never started: it is read now.
        if let Some(problem) = start_under(session, Some(&[object])).into_iter().next() {
            return Err(problem);
        }
    }
    let player = session.play.as_mut().and_then(|p| p.timelines.players.iter_mut().find(|t| t.root == object));
    let Some(player) = player else { return Err("it has no Keyframe Animation with an animation file".to_owned()) };
    if play {
        player.time = 0.0;
    }
    player.playing = play;
    Ok(())
}

/// Reads the Keyframe Animations of `roots` and everything under them (the whole scene for
/// `None`), and starts those that play on start. Returns what could not be read.
pub(crate) fn start_under(session: &mut Session, roots: Option<&[StableId]>) -> Vec<String> {
    let found: Vec<(StableId, KeyframeAnimation)> = session
        .scene
        .world_matrices()
        .into_iter()
        .map(|(id, _)| id)
        .filter(|id| roots.is_none_or(|roots| roots.iter().any(|r| r == id || session.scene.is_inside(*id, *r))))
        .filter_map(|id| Some((id, session.scene.get::<KeyframeAnimation>(id).filter(|k| !k.animation.is_empty())?.clone())))
        .collect();
    let mut problems = Vec::new();
    for (root, component) in found {
        let who = session.scene.name(root).unwrap_or("Object").to_owned();
        let timeline = match read(session, &component.animation) {
            Ok(timeline) => timeline,
            Err(why) => {
                problems.push(format!("[{who}] cannot play {}: {why}", component.animation));
                continue;
            }
        };
        let binding = bind(session, root, &timeline);
        let Some(play) = session.play.as_mut() else { return problems };
        play.timelines.players.retain(|p| p.root != root);
        play.timelines.players.push(Player { root, timeline, binding, time: 0.0, playing: component.play_on_start, told: false });
    }
    problems
}

/// The timeline at `relative` (to the project folder), read once per play.
fn read(session: &mut Session, relative: &str) -> Result<Arc<Timeline>, String> {
    if let Some(known) = session.play.as_ref().and_then(|p| p.timelines.files.get(relative)) {
        return known.clone();
    }
    let read = (|| {
        let path = session.allowed(&session.script_dir().join(relative), crate::confine::Access::Read).map_err(|e| e.to_string())?;
        let text = bayleaf_core::files::read_to_string(&path).map_err(|e| format!("cannot read it: {e}"))?;
        Timeline::from_json(&text).map(Arc::new)
    })();
    if let Some(play) = session.play.as_mut() {
        play.timelines.files.insert(relative.to_owned(), read.clone());
    }
    read
}

/// The objects of `root`'s copy that stand for the timeline's parts: by hierarchy order when it is
/// a copy of the timeline's prefab, else by the parts' own ids under it.
fn bind(session: &Session, root: StableId, timeline: &Timeline) -> Binding {
    if session.scene.get::<Prefab>(root).is_some_and(|p| p.source == timeline.prefab)
        && let Ok(objects) = crate::prefab_cmds::read_prefab(session, &timeline.prefab)
    {
        let ids: Vec<StableId> = objects.into_iter().map(|o| o.id).collect();
        return Binding::by_order(&session.scene, root, &ids, timeline);
    }
    let mut binding = Binding::same_ids(&session.scene, timeline);
    binding.objects.retain(|_, object| *object == root || session.scene.is_inside(*object, root));
    binding
}

/// Moves every playing Keyframe Animation on by `dt` seconds and poses its parts.
pub(crate) fn step(session: &mut Session, dt: f32) {
    let Some(play) = session.play.as_mut() else { return };
    // Objects that went take their animation with them.
    play.timelines.players.retain(|p| session.scene.contains(p.root));
    let mut reports = Vec::new();
    for player in play.timelines.players.iter_mut().filter(|p| p.playing) {
        let speed = session.scene.get::<KeyframeAnimation>(player.root).map_or(1.0, |k| k.speed);
        player.time += f64::from(dt) * f64::from(speed);
        let samples = player.timeline.sample(player.timeline.local_time(player.time));
        let problems = timeline::apply(&mut session.scene, &player.binding, &samples);
        if player.timeline.finished(player.time) {
            player.playing = false;
        }
        if !problems.is_empty() && !std::mem::replace(&mut player.told, true) {
            let who = session.scene.name(player.root).unwrap_or("Object").to_owned();
            reports.push(format!("[{who}] its Keyframe Animation cannot set everything: {}", problems.join("; ")));
        }
    }
    for report in reports {
        session.log(Level::Error, report);
    }
}
