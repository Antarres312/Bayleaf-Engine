//! Clips: what poses a skeleton's parts over time, and the moments they mark.

use std::collections::BTreeMap;
use std::sync::Arc;

use bayleaf_assets::skeleton::{Pose, Property, Skeleton, sample_keys};
use bayleaf_core::math::Quat;

/// A moment a clip marks (a footstep): when it plays past `time` seconds, the object gets the
/// script event `name`.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipEvent {
    pub time: f32,
    pub name: String,
}

/// Something that poses named parts over time. A model's own animations are clips
/// ([`ModelClip`]); so is anything else that can say where each part is at a moment, such as a
/// timeline's ([`NamedClip`] is a ready one, its parts found by name).
///
/// To give an object a clip of another kind, implement this and add it to the object's [`Clips`]
/// under a name ([`Clips::insert`]); motions, animation states, scripts and the `animation.*`
/// commands then play it by that name like any other.
pub trait ClipSource: std::fmt::Debug + Send + Sync {
    /// Seconds from its start to its end.
    fn duration(&self) -> f32;

    /// Writes where the clip puts the parts it moves at `time` seconds (from 0 to its duration)
    /// over `pose`, finding them in `skeleton` (each part is a node of it, by index or by name);
    /// parts it does not move keep their place.
    fn sample(&self, skeleton: &Skeleton, time: f32, pose: &mut Pose);

    /// The moments it marks, by rising time.
    fn events(&self) -> &[ClipEvent] {
        &[]
    }
}

/// One of a model's own animations, by its index in the model.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelClip {
    pub animation: usize,
    pub duration: f32,
    pub events: Vec<ClipEvent>,
}

impl ClipSource for ModelClip {
    fn duration(&self) -> f32 {
        self.duration
    }

    fn sample(&self, skeleton: &Skeleton, time: f32, pose: &mut Pose) {
        skeleton.sample_into(self.animation, time, pose);
    }

    fn events(&self) -> &[ClipEvent] {
        &self.events
    }
}

/// Keys of one property of a part named `part`.
#[derive(Clone, Debug, PartialEq)]
pub struct NamedTrack {
    pub part: String,
    pub property: Property,
    /// Holds each key until the next instead of moving between them.
    pub step: bool,
    /// Seconds, rising.
    pub times: Vec<f32>,
    /// x, y, z (and w for a rotation, as a quaternion) of each key.
    pub values: Vec<[f32; 4]>,
}

/// A clip that poses parts by their names: what a clip made outside the model (on a timeline)
/// becomes. A part no node of the skeleton is called is left out.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct NamedClip {
    pub duration: f32,
    pub tracks: Vec<NamedTrack>,
    pub events: Vec<ClipEvent>,
}

impl ClipSource for NamedClip {
    fn duration(&self) -> f32 {
        self.duration
    }

    fn sample(&self, skeleton: &Skeleton, time: f32, pose: &mut Pose) {
        for track in &self.tracks {
            let Some(node) = skeleton.node_named(&track.part) else { continue };
            let Some(v) = sample_keys(track.property, track.step, &track.times, &track.values, time) else { continue };
            let Some(local) = pose.locals.get_mut(node) else { continue };
            match track.property {
                Property::Translation => local.translation = v.truncate(),
                Property::Rotation => local.rotation = Quat::from_vec4(v).normalize(),
                Property::Scale => local.scale = v.truncate(),
            }
        }
    }

    fn events(&self) -> &[ClipEvent] {
        &self.events
    }
}

/// An object's clips by name.
#[derive(Clone, Debug, Default)]
pub struct Clips {
    by_name: BTreeMap<String, Arc<dyn ClipSource>>,
}

impl Clips {
    /// `skeleton`'s own animations, under their names, with the moments `events` marks in them
    /// (`(clip, event)` pairs).
    pub fn of_model(skeleton: &Skeleton, events: &[(String, ClipEvent)]) -> Self {
        let mut clips = Clips::default();
        for (index, animation) in skeleton.animations.iter().enumerate() {
            let mut own: Vec<ClipEvent> = events.iter().filter(|(clip, _)| *clip == animation.name).map(|(_, e)| e.clone()).collect();
            own.sort_by(|a, b| a.time.total_cmp(&b.time));
            clips.insert(&animation.name, Arc::new(ModelClip { animation: index, duration: animation.duration, events: own }));
        }
        clips
    }

    /// Adds `clip` as `name`, in place of one of that name.
    pub fn insert(&mut self, name: &str, clip: Arc<dyn ClipSource>) {
        self.by_name.insert(name.to_owned(), clip);
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn ClipSource>> {
        self.by_name.get(name)
    }

    /// The clips' names, in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str)
    }
}
