//! The profiler: while it records, each play step's time by part (scripts, each script, physics,
//! streaming, sound, particles) and, from the views, the time to gather and draw the frame and
//! what was drawn. It keeps the last few thousand frames; `profile.*` commands read them.

use std::collections::VecDeque;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};

use crate::{CommandError, CommandSpec, Mutates, Prompt, Session, always, parse};

/// Frames kept, the oldest dropped first.
pub const MAX_FRAMES: usize = 3000;

/// What one frame took, in milliseconds, and what it drew.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameProfile {
    /// Play time at the frame's end, seconds.
    pub time: f64,
    /// The whole play step.
    pub step: f32,
    pub scripts: f32,
    /// Each script's time, by its name, slowest first.
    pub by_script: Vec<(String, f32)>,
    pub physics: f32,
    pub streaming: f32,
    pub sound: f32,
    pub particles: f32,
    /// Gathering the frame from the scene (the view's own work), when a view drew it.
    pub extract: Option<f32>,
    /// Recording and sending the drawing to the GPU.
    pub draw: Option<f32>,
    pub draw_calls: u32,
    pub triangles: u64,
    /// Bytes of meshes and of images on the GPU.
    pub mesh_bytes: u64,
    pub image_bytes: u64,
}

/// A part of the play step the profiler times.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Scripts,
    Physics,
    Streaming,
    Sound,
    Particles,
}

/// What the renderer drew in a frame, as a view reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DrawnStats {
    pub extract: Duration,
    pub draw: Duration,
    pub draw_calls: u32,
    pub triangles: u64,
    pub mesh_bytes: u64,
    pub image_bytes: u64,
}

#[derive(Default)]
pub struct Profiler {
    recording: bool,
    frames: VecDeque<FrameProfile>,
    current: FrameProfile,
}

fn ms(d: Duration) -> f32 {
    d.as_secs_f32() * 1000.0
}

impl Profiler {
    pub fn is_recording(&self) -> bool {
        self.recording
    }

    /// Adds `took` to a part of the step being timed.
    pub fn add(&mut self, part: Part, took: Duration) {
        if !self.recording {
            return;
        }
        let slot = match part {
            Part::Scripts => &mut self.current.scripts,
            Part::Physics => &mut self.current.physics,
            Part::Streaming => &mut self.current.streaming,
            Part::Sound => &mut self.current.sound,
            Part::Particles => &mut self.current.particles,
        };
        *slot += ms(took);
    }

    /// Adds each script's time this step.
    pub fn add_scripts(&mut self, timings: Vec<(String, Duration)>) {
        if !self.recording {
            return;
        }
        for (name, took) in timings {
            match self.current.by_script.iter_mut().find(|(n, _)| *n == name) {
                Some((_, total)) => *total += ms(took),
                None => self.current.by_script.push((name, ms(took))),
            }
        }
    }

    /// Ends the frame's step: it took `took` in all, at play time `time`.
    pub fn end_step(&mut self, time: f64, took: Duration) {
        if !self.recording {
            return;
        }
        let mut frame = std::mem::take(&mut self.current);
        frame.time = time;
        frame.step = ms(took);
        frame.by_script.sort_by(|a, b| b.1.total_cmp(&a.1));
        if self.frames.len() >= MAX_FRAMES {
            self.frames.pop_front();
        }
        self.frames.push_back(frame);
    }

    /// What a view drew for the latest frame.
    pub fn add_drawn(&mut self, stats: DrawnStats) {
        if !self.recording {
            return;
        }
        if let Some(frame) = self.frames.back_mut() {
            frame.extract = Some(frame.extract.unwrap_or(0.0) + ms(stats.extract));
            frame.draw = Some(frame.draw.unwrap_or(0.0) + ms(stats.draw));
            frame.draw_calls += stats.draw_calls;
            frame.triangles += stats.triangles;
            frame.mesh_bytes = frame.mesh_bytes.max(stats.mesh_bytes);
            frame.image_bytes = frame.image_bytes.max(stats.image_bytes);
        }
    }

    pub fn frames(&self) -> &VecDeque<FrameProfile> {
        &self.frames
    }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    let spec =
        |id, label, params, run| CommandSpec { id, label, menu: "", shortcut: "", params, prompt: Prompt::None, mutates: Mutates::No, enabled: always, run };
    vec![
        spec("profile.start", "Record Profile", "none: records each play step's time by part, and what the views draw, until profile.stop", profile_start),
        spec("profile.stop", "Stop Profile", "none: stops recording; the frames recorded stay", profile_stop),
        spec("profile.clear", "Clear Profile", "none: forgets the frames recorded", profile_clear),
        spec(
            "profile.frames",
            "Profile Frames",
            "{last?}: the last frames recorded (all, at most 3000): {recording, frames: [{time, step, scripts, byScript, physics, streaming, sound, particles, extract, draw, drawCalls, triangles, meshBytes, imageBytes}], average}",
            profile_frames,
        ),
    ]
}

fn profile_start(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    parse::<crate::play::NoParams>(params)?;
    session.profiler.recording = true;
    Ok(json!({ "recording": true }))
}

fn profile_stop(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    parse::<crate::play::NoParams>(params)?;
    session.profiler.recording = false;
    session.profiler.current = FrameProfile::default();
    Ok(json!({ "recording": false, "frames": session.profiler.frames.len() }))
}

fn profile_clear(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    parse::<crate::play::NoParams>(params)?;
    session.profiler.frames.clear();
    Ok(json!({}))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FramesParams {
    #[serde(default)]
    last: Option<usize>,
}

fn profile_frames(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: FramesParams = parse(params)?;
    let frames = &session.profiler.frames;
    let take = p.last.unwrap_or(MAX_FRAMES).min(frames.len());
    let chosen: Vec<&FrameProfile> = frames.iter().skip(frames.len() - take).collect();
    let n = chosen.len().max(1) as f32;
    let average = |f: &dyn Fn(&FrameProfile) -> f32| chosen.iter().map(|frame| f(frame)).sum::<f32>() / n;
    Ok(json!({
        "recording": session.profiler.recording,
        "frames": chosen,
        "average": {
            "step": average(&|f| f.step),
            "scripts": average(&|f| f.scripts),
            "physics": average(&|f| f.physics),
            "streaming": average(&|f| f.streaming),
            "sound": average(&|f| f.sound),
            "particles": average(&|f| f.particles),
            "draw": average(&|f| f.draw.unwrap_or(0.0)),
        },
    }))
}
