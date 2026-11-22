//! Sound in play mode (the `audio` feature, `crates/audio`): Audio Source components, scripts'
//! requests and the `audio.*` commands. Sounds are heard only in sessions that turn sound on (the
//! editor and the player); the others, such as the CLI, agents and tests, play them silently, in
//! time, so what plays and when stays the same. Builds without the feature say so instead.

use bayleaf_core::StableId;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::play::playing;
use crate::{CommandError, CommandSpec, Mutates, Prompt, Session, parse};

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "audio.playing",
            label: "Sounds Playing",
            menu: "",
            shortcut: "",
            params: "none; returns {heard, sounds: [{id, source, object, seconds, looping, volume}]}: the sounds playing now (heard is false where play is silent)",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: |s| playing(s),
            run: playing_command,
        },
        CommandSpec {
            id: "audio.play",
            label: "Play Sound",
            menu: "",
            shortcut: "",
            params: "{id}: plays the object's Audio Source; returns {sound}",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: |s| playing(s),
            run: play_command,
        },
        CommandSpec {
            id: "audio.stop",
            label: "Stop Sound",
            menu: "",
            shortcut: "",
            params: "{id}: stops the sounds the object plays; returns {stopped}",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: |s| playing(s),
            run: stop_command,
        },
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdParams {
    id: StableId,
}

fn playing_command(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    parse::<crate::play::NoParams>(params)?;
    imp::playing(session).map_err(CommandError::Failed)
}

fn play_command(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: IdParams = parse(params)?;
    let sound = imp::play_object(session, p.id).map_err(CommandError::Failed)?;
    Ok(json!({ "sound": sound }))
}

fn stop_command(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: IdParams = parse(params)?;
    let stopped = imp::stop_object(session, p.id).map_err(CommandError::Failed)?;
    Ok(json!({ "stopped": stopped }))
}

pub(crate) use imp::{play_at, play_object, start_sources, stop_object, update};

#[cfg(feature = "audio")]
mod imp {
    use std::collections::HashMap;

    use bayleaf_audio::{Audio, Listener, Place};
    use bayleaf_core::StableId;
    use bayleaf_core::math::DVec3;
    use bayleaf_scene::components::{AudioSource, Camera};
    use serde_json::{Value, json};

    use crate::Session;

    /// Sound for a new play session: heard when the session turns sound on and there is a device.
    pub(crate) fn open(session: &Session) -> (Audio, Option<String>) {
        if !session.sound {
            return (Audio::silent(bayleaf_audio::SILENT_RATE), None);
        }
        let audio = Audio::open();
        let why = audio.silent_because.clone().map(|why| format!("Sounds play silently: {why}."));
        (audio, why)
    }

    fn audio(session: &mut Session) -> Result<&mut Audio, String> {
        session.play.as_mut().and_then(|p| p.audio.as_mut()).ok_or_else(|| "play mode is not running".to_owned())
    }

    /// The sound file of a project path, read once: inside the readable folders.
    fn clip(session: &mut Session, relative: &str) -> Result<std::sync::Arc<bayleaf_audio::Clip>, String> {
        let path = session.allowed(&session.script_dir().join(relative), crate::confine::Access::Read).map_err(|e| e.to_string())?;
        audio(session)?.clip(&path)
    }

    /// Plays `object`'s Audio Source; returns the sound's id.
    pub(crate) fn play_object(session: &mut Session, object: StableId) -> Result<u64, String> {
        let source = session.scene.get::<AudioSource>(object).cloned().ok_or("it has no Audio Source")?;
        if source.clip.is_empty() {
            return Err("its Audio Source has no sound file".to_owned());
        }
        let clip = clip(session, &source.clip)?;
        let at = session.scene.world_matrix(object).map_or(DVec3::ZERO, |m| m.w_axis.truncate());
        let place = source.spatial.then_some(Place { at, range: f64::from(source.range) });
        audio(session)?
            .with(|mixer| mixer.play(clip, &source.clip, Some(object), source.volume, source.looping, place))
            .ok_or_else(|| format!("{} sounds already play", bayleaf_audio::MAX_VOICES))
    }

    pub(crate) fn stop_object(session: &mut Session, object: StableId) -> Result<usize, String> {
        Ok(audio(session)?.with(|mixer| mixer.stop(Some(object), None)))
    }

    /// Plays a sound file once from a world position.
    pub(crate) fn play_at(session: &mut Session, relative: &str, position: [f32; 3], volume: f32) -> Result<u64, String> {
        let clip = clip(session, relative)?;
        let place = Place { at: DVec3::from(position.map(f64::from)), range: 30.0 };
        audio(session)?
            .with(|mixer| mixer.play(clip, relative, None, volume, false, Some(place)))
            .ok_or_else(|| format!("{} sounds already play", bayleaf_audio::MAX_VOICES))
    }

    /// Starts the Audio Sources that play on start: the whole scene's (`roots` `None`), or those of
    /// objects under `roots`. Returns what could not play.
    pub(crate) fn start_sources(session: &mut Session, roots: Option<&[StableId]>) -> Vec<String> {
        let starting: Vec<StableId> = session
            .scene
            .world_matrices()
            .into_iter()
            .map(|(id, _)| id)
            .filter(|id| roots.is_none_or(|roots| roots.iter().any(|r| r == id || session.scene.is_inside(*id, *r))))
            .filter(|id| session.scene.get::<AudioSource>(*id).is_some_and(|s| s.play_on_start && !s.clip.is_empty()))
            .collect();
        let mut problems = Vec::new();
        for id in starting {
            if let Err(why) = play_object(session, id) {
                let who = session.scene.name(id).unwrap_or("Object").to_owned();
                problems.push(format!("[{who}] cannot play its sound: {why}"));
            }
        }
        problems
    }

    /// Moves the listener (the first camera) and the sounds of objects, drops the sounds of objects
    /// that are gone, and, where play is silent, moves the sounds on by `seconds`.
    pub(crate) fn update(session: &mut Session, seconds: f64) {
        let Some(play) = session.play.as_ref() else { return };
        let Some(audio) = play.audio.as_ref() else { return };
        let scene = &session.scene;
        let matrices = scene.world_matrices();
        let listener = matrices
            .iter()
            .find(|(id, _)| scene.get::<Camera>(*id).is_some())
            .map_or_else(Listener::default, |(_, m)| Listener { at: m.w_axis.truncate(), right: m.x_axis.truncate() });
        let owners: Vec<StableId> = audio.with(|mixer| mixer.voices().iter().filter_map(|v| v.owner).collect());
        let places: HashMap<StableId, DVec3> = matrices.iter().filter(|(id, _)| owners.contains(id)).map(|(id, m)| (*id, m.w_axis.truncate())).collect();
        audio.with(|mixer| mixer.place(listener, &places, |id| !scene.contains(id)));
        audio.advance(seconds);
    }

    pub(crate) fn playing(session: &mut Session) -> Result<Value, String> {
        let audio = audio(session)?;
        let rate = audio.rate();
        let sounds: Vec<Value> = audio.with(|mixer| {
            mixer
                .voices()
                .iter()
                .map(|v| json!({ "id": v.id, "source": v.source, "object": v.owner, "seconds": v.seconds(rate), "looping": v.looping, "volume": v.volume }))
                .collect()
        });
        Ok(json!({ "heard": audio.is_heard(), "sounds": sounds }))
    }
}

#[cfg(not(feature = "audio"))]
mod imp {
    use bayleaf_core::StableId;
    use serde_json::Value;

    use crate::Session;

    const NO_SOUND: &str = "this build of Bayleaf has no sound (the `audio` feature)";

    pub(crate) fn play_object(_: &mut Session, _: StableId) -> Result<u64, String> {
        Err(NO_SOUND.to_owned())
    }

    pub(crate) fn stop_object(_: &mut Session, _: StableId) -> Result<usize, String> {
        Err(NO_SOUND.to_owned())
    }

    pub(crate) fn play_at(_: &mut Session, _: &str, _: [f32; 3], _: f32) -> Result<u64, String> {
        Err(NO_SOUND.to_owned())
    }

    pub(crate) fn start_sources(_: &mut Session, _: Option<&[StableId]>) -> Vec<String> {
        Vec::new()
    }

    pub(crate) fn update(_: &mut Session, _: f64) {}

    pub(crate) fn playing(_: &mut Session) -> Result<Value, String> {
        Err(NO_SOUND.to_owned())
    }
}

#[cfg(feature = "audio")]
pub(crate) use imp::open;
