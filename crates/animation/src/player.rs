//! A player: the motions playing on one object, by layer, with their fades and parameters.

use std::collections::BTreeMap;

use bayleaf_assets::skeleton::{Pose, Skeleton};
use bayleaf_core::math::Vec3;

use crate::clip::Clips;
use crate::motion::{Motion, Parameter, Rig, pin_root};

/// Longest fade, in seconds.
pub const MAX_FADE: f32 = 60.0;
/// Most layers one object plays at once.
pub const MAX_LAYERS: usize = 16;
/// Most parameters one object holds.
pub const MAX_PARAMETERS: usize = 256;

/// A motion playing, and how far it has faded in.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    motion: Motion,
    /// Seconds since it started.
    time: f32,
    /// Seconds it takes to fade in; 0 for at once.
    fade: f32,
}

impl Entry {
    fn weight(&self) -> f32 {
        if self.fade <= 0.0 { 1.0 } else { (self.time / self.fade).clamp(0.0, 1.0) }
    }
}

/// The motions playing on the whole skeleton (the base layer) or on one joint and the nodes under
/// it, over the base.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    /// The joint the layer plays on, with every node under it; empty for the base layer.
    pub joint: String,
    /// What it played and plays, oldest first: each fades in over the ones before.
    entries: Vec<Entry>,
    /// While a layer over the base stops: seconds left of its fade out, and in all.
    fade_out: Option<(f32, f32)>,
}

impl Layer {
    fn new(joint: &str) -> Self {
        Self { joint: joint.to_owned(), entries: Vec::new(), fade_out: None }
    }

    /// What plays now (the newest motion), and seconds since it started.
    pub fn current(&self) -> Option<(&Motion, f32)> {
        self.entries.last().map(|e| (&e.motion, e.time))
    }

    /// Whether a fade is under way: in from what played before, or out of the whole layer.
    pub fn fading(&self) -> bool {
        self.fade_out.is_some() || self.entries.len() > 1
    }

    /// How much of the layer shows (a layer over the base fades out as it stops).
    fn strength(&self) -> f32 {
        self.fade_out.map_or(1.0, |(left, total)| if total > 0.0 { (left / total).clamp(0.0, 1.0) } else { 0.0 })
    }

    /// Each entry's share of the layer's pose: its weight times what the later ones leave.
    fn shares(&self) -> Vec<f32> {
        let mut shares = vec![0.0; self.entries.len()];
        let mut left = 1.0;
        for (i, e) in self.entries.iter().enumerate().rev() {
            let w = if i == 0 { 1.0 } else { e.weight() };
            shares[i] = left * w;
            left *= 1.0 - w;
        }
        shares
    }

    fn pose(&self, rig: &Rig<'_>) -> Option<Pose> {
        let mut pose: Option<Pose> = None;
        for e in &self.entries {
            let own = e.motion.pose(rig, e.time);
            pose = Some(match pose {
                None => own,
                Some(before) => before.mix(&own, e.weight()),
            });
        }
        pose
    }
}

/// What one step of a player gave.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Step {
    /// The clip events it passed, in order.
    pub events: Vec<String>,
    /// How far the root joint moved along the ground, in the model's space (with its import scale
    /// and axes), when the player follows root motion.
    pub root: Vec3,
}

/// The motions playing on one object, by layer (the base first), and the parameters they read.
#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    layers: Vec<Layer>,
    pub parameters: BTreeMap<String, Parameter>,
    /// The joint whose movement along the ground moves the object, drawn in place: root motion.
    /// `None` plays every motion where it was made.
    pub root: Option<usize>,
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}

impl Player {
    /// A player with nothing playing: every node at rest.
    pub fn new() -> Self {
        Self { layers: vec![Layer::new("")], parameters: BTreeMap::new(), root: None }
    }

    /// The base layer, then the layers over it.
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    /// The layer on `joint` (empty for the base).
    pub fn layer(&self, joint: &str) -> Option<&Layer> {
        self.layers.iter().find(|l| l.joint == joint)
    }

    /// Plays `motion` on the layer of `joint` (empty: the whole skeleton), fading it in over
    /// `fade` seconds from what played there; a new layer starts at full strength at once.
    pub fn play(&mut self, joint: &str, motion: Motion, fade: f32) -> Result<(), String> {
        let fade = if fade.is_finite() { fade.clamp(0.0, MAX_FADE) } else { 0.0 };
        let index = match self.layers.iter().position(|l| l.joint == joint) {
            Some(i) => i,
            None => {
                if self.layers.len() >= MAX_LAYERS {
                    return Err(format!("an object plays at most {MAX_LAYERS} layers"));
                }
                self.layers.push(Layer::new(joint));
                self.layers.len() - 1
            }
        };
        let Some(layer) = self.layers.get_mut(index) else { return Ok(()) };
        // A layer that was stopping comes back at once.
        layer.fade_out = None;
        let fade = if layer.entries.is_empty() { 0.0 } else { fade };
        layer.entries.push(Entry { motion, time: 0.0, fade });
        // A fade over many quick changes keeps the last few.
        if layer.entries.len() > 8 {
            layer.entries.remove(0);
        }
        Ok(())
    }

    /// Stops the layer of `joint` over `fade` seconds: the base layer fades to the rest pose, a
    /// layer over it fades out and goes.
    pub fn stop(&mut self, joint: &str, fade: f32) {
        let fade = if fade.is_finite() { fade.clamp(0.0, MAX_FADE) } else { 0.0 };
        if joint.is_empty() {
            if let Some(base) = self.layers.first_mut()
                && !base.entries.is_empty()
            {
                base.entries.push(Entry { motion: Motion::Rest, time: 0.0, fade });
            }
            return;
        }
        if fade <= 0.0 {
            self.layers.retain(|l| l.joint != joint);
        } else if let Some(layer) = self.layers.iter_mut().find(|l| l.joint == joint) {
            layer.fade_out = Some((fade, fade));
        }
    }

    /// Sets a parameter (a number, a true/false or a trigger).
    pub fn set_parameter(&mut self, name: &str, value: Parameter) -> Result<(), String> {
        if !self.parameters.contains_key(name) && self.parameters.len() >= MAX_PARAMETERS {
            return Err(format!("an object holds at most {MAX_PARAMETERS} animation parameters"));
        }
        self.parameters.insert(name.to_owned(), value);
        Ok(())
    }

    fn rig<'a>(skeleton: &'a Skeleton, clips: &'a Clips, parameters: &'a BTreeMap<String, Parameter>) -> Rig<'a> {
        Rig { skeleton, clips, parameters }
    }

    /// Moves every motion on by `dt` seconds: gives the clip events passed (of the base layer's
    /// main motion, and of layers over it) and, with root motion, how far the root joint moved.
    pub fn advance(&mut self, skeleton: &Skeleton, clips: &Clips, dt: f32) -> Step {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        let mut step = Step::default();
        let rig = Self::rig(skeleton, clips, &self.parameters);
        for (index, layer) in self.layers.iter_mut().enumerate() {
            let shares = layer.shares();
            let main = shares.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i);
            for (i, e) in layer.entries.iter().enumerate() {
                if Some(i) == main && layer.fade_out.is_none() {
                    e.motion.events(&rig, e.time, e.time + dt, &mut step.events);
                }
                if index == 0
                    && let Some(root) = self.root
                {
                    step.root += e.motion.root_motion(&rig, root, e.time, e.time + dt) * shares.get(i).copied().unwrap_or(0.0);
                }
            }
            for e in &mut layer.entries {
                e.time += dt;
            }
            // What a finished fade covers no longer shows.
            if let Some(last_full) = layer.entries.iter().rposition(|e| e.weight() >= 1.0) {
                layer.entries.drain(..last_full);
            }
            if let Some((left, _)) = &mut layer.fade_out {
                *left -= dt;
            }
        }
        self.layers.retain(|l| l.joint.is_empty() || l.fade_out.is_none_or(|(left, _)| left > 0.0));
        step
    }

    /// The pose the player gives now: the base layer's, with each layer over it on its joint and
    /// the nodes under it; with root motion, the root joint kept in place along the ground.
    pub fn pose(&self, skeleton: &Skeleton, clips: &Clips) -> Pose {
        let rig = Self::rig(skeleton, clips, &self.parameters);
        let mut pose = self.layers.first().and_then(|l| l.pose(&rig)).unwrap_or_else(|| skeleton.rest_pose());
        if let Some(root) = self.root {
            pin_root(skeleton, &mut pose, root);
        }
        for layer in self.layers.iter().skip(1) {
            let (Some(over), Some(mask)) = (layer.pose(&rig), skeleton.mask(&layer.joint)) else { continue };
            pose = pose.mix_masked(&over, layer.strength(), &mask);
        }
        pose
    }

    /// Whether anything plays (the base layer has a motion other than rest, or a layer is over it).
    pub fn is_playing(&self) -> bool {
        self.layers.len() > 1 || self.layers.first().and_then(Layer::current).is_some_and(|(m, _)| *m != Motion::Rest)
    }
}
