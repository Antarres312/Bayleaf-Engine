//! Motions: what plays on a skeleton (a clip, a mix of motions, a layer on part of it), and how
//! they pose it, which clip events they pass and how far they carry the root joint.

use std::collections::BTreeMap;

use bayleaf_assets::skeleton::{Pose, Skeleton};
use bayleaf_core::math::{Mat4, Vec3};

use crate::clip::Clips;

/// Most loops of a clip one step looks through for events and root motion.
const MAX_LOOPS: i64 = 64;

/// A value scripts set on an object's animation: what mixes, layers and animation states read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Parameter {
    Number(f32),
    Flag(bool),
    /// True from when it is set until a transition it lets through uses it up.
    Trigger(bool),
}

impl Parameter {
    /// As a number: a true/false or a trigger reads 1 when set, else 0.
    pub fn number(&self) -> f32 {
        match *self {
            Parameter::Number(n) => n,
            Parameter::Flag(b) | Parameter::Trigger(b) => f32::from(u8::from(b)),
        }
    }

    /// As true/false: a number is true when it is not 0.
    pub fn flag(&self) -> bool {
        match *self {
            Parameter::Number(n) => n != 0.0,
            Parameter::Flag(b) | Parameter::Trigger(b) => b,
        }
    }
}

/// What a number in a motion reads.
#[derive(Clone, Debug, PartialEq)]
pub enum Number {
    Value(f32),
    /// A parameter by name; 0 while none of that name is set.
    Parameter(String),
}

impl Number {
    pub fn read(&self, parameters: &BTreeMap<String, Parameter>) -> f32 {
        let n = match self {
            Number::Value(v) => *v,
            Number::Parameter(name) => parameters.get(name).map_or(0.0, Parameter::number),
        };
        if n.is_finite() { n } else { 0.0 }
    }
}

/// What plays on a skeleton.
#[derive(Clone, Debug, PartialEq)]
pub enum Motion {
    /// Every node at rest.
    Rest,
    /// A clip by name, `speed` times as fast as made, from its end round to its start when
    /// `looping`, else holding its last moment.
    Clip { clip: String, speed: f32, looping: bool },
    /// Motions by where the number `by` lies among their points: between two points, the two
    /// round it, each by how near it is; before the first or past the last, that one alone (walk
    /// at 1 and run at 3, by speed). They play in step: each reaches its end when the mix does.
    Mix { items: Vec<(Motion, f32)>, by: Number },
    /// `top` over `base` on the joint named `joint` and every node under it (the whole skeleton
    /// when it is empty), by `weight` from 0 to 1.
    Layer { base: Box<Motion>, top: Box<Motion>, joint: String, weight: Number },
}

/// What a motion plays on: the skeleton, the clips it names, and the parameters its numbers read.
#[derive(Clone, Copy, Debug)]
pub struct Rig<'a> {
    pub skeleton: &'a Skeleton,
    pub clips: &'a Clips,
    pub parameters: &'a BTreeMap<String, Parameter>,
}

/// The weight of each point in `points` for `value`: see [`Motion::Mix`].
fn mix_weights(points: &[f32], value: f32) -> Vec<f32> {
    let mut weights = vec![0.0; points.len()];
    let below = points.iter().enumerate().filter(|(_, p)| **p <= value).max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i);
    let above = points.iter().enumerate().filter(|(_, p)| **p >= value).min_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i);
    match (below, above) {
        (Some(b), Some(a)) if a != b && points[a] > points[b] => {
            let t = (value - points[b]) / (points[a] - points[b]);
            weights[b] = 1.0 - t;
            weights[a] = t;
        }
        (Some(i), _) | (None, Some(i)) => weights[i] = 1.0,
        (None, None) => {}
    }
    weights
}

/// Where a clip of `duration` seconds is `time` seconds after it started at `speed`.
fn clip_time(duration: f32, speed: f32, looping: bool, time: f32) -> f32 {
    let t = time * speed;
    if duration <= 0.0 || !t.is_finite() {
        return 0.0;
    }
    if looping { t.rem_euclid(duration) } else { t.clamp(0.0, duration) }
}

/// The matrix of `node` into the model (before the import scale and axes) in `pose`, by its chain
/// of parents only.
fn node_matrix(skeleton: &Skeleton, pose: &Pose, node: usize) -> Mat4 {
    let mut m = pose.locals.get(node).map_or(Mat4::IDENTITY, |t| t.matrix());
    let mut parent = skeleton.nodes.get(node).and_then(|n| n.parent);
    let mut steps = 0;
    while let Some(p) = parent {
        m = pose.locals.get(p).map_or(Mat4::IDENTITY, |t| t.matrix()) * m;
        parent = skeleton.nodes.get(p).and_then(|n| n.parent);
        steps += 1;
        if steps > skeleton.nodes.len() {
            break;
        }
    }
    m
}

/// Where `node` is in the model, with the import scale and axes (so Y is up), in `pose`.
fn model_position(skeleton: &Skeleton, pose: &Pose, node: usize) -> Vec3 {
    (skeleton.root * node_matrix(skeleton, pose, node)).w_axis.truncate()
}

/// The joint root motion follows when none is named: the first node, parents first, that one of
/// the model's animations moves.
pub fn default_root(skeleton: &Skeleton) -> Option<usize> {
    let moved = |node: usize| {
        skeleton.animations.iter().any(|a| a.channels.iter().any(|c| c.node == node && c.property == bayleaf_assets::skeleton::Property::Translation))
    };
    skeleton.order.iter().copied().find(|n| moved(*n))
}

/// Keeps joint `root` in place along the ground in `pose`: moves it back by how far it is, along
/// X and Z of the model, from where it stands at rest. Its height stays.
pub fn pin_root(skeleton: &Skeleton, pose: &mut Pose, root: usize) {
    let rest = skeleton.rest_pose();
    let mut offset = model_position(skeleton, pose, root) - model_position(skeleton, &rest, root);
    offset.y = 0.0;
    let parent = skeleton.nodes.get(root).and_then(|n| n.parent).map_or(Mat4::IDENTITY, |p| node_matrix(skeleton, pose, p));
    let into_parent = (skeleton.root * parent).inverse();
    if !into_parent.is_finite() {
        return;
    }
    if let Some(local) = pose.locals.get_mut(root) {
        local.translation -= into_parent.transform_vector3(offset);
    }
}

impl Motion {
    /// A clip at its own speed, looping.
    pub fn clip(name: &str) -> Motion {
        Motion::Clip { clip: name.to_owned(), speed: 1.0, looping: true }
    }

    /// Seconds from its start to its end as it plays now (a mix's by its weights).
    pub fn duration(&self, rig: &Rig<'_>) -> f32 {
        match self {
            Motion::Rest => 0.0,
            Motion::Clip { clip, speed, .. } => {
                let d = rig.clips.get(clip).map_or(0.0, |c| c.duration());
                if *speed > 1e-6 { d / speed } else { d }
            }
            Motion::Mix { items, by } => {
                let points: Vec<f32> = items.iter().map(|(_, p)| *p).collect();
                let weights = mix_weights(&points, by.read(rig.parameters));
                items.iter().zip(weights).map(|((m, _), w)| if w > 0.0 { m.duration(rig) * w } else { 0.0 }).sum()
            }
            Motion::Layer { base, .. } => base.duration(rig),
        }
    }

    /// Whether it starts over at its end (a mix or layer when any of its clips does).
    pub fn looping(&self) -> bool {
        match self {
            Motion::Rest => false,
            Motion::Clip { looping, .. } => *looping,
            Motion::Mix { items, .. } => items.iter().any(|(m, _)| m.looping()),
            Motion::Layer { base, top, .. } => base.looping() || top.looping(),
        }
    }

    /// For each part of a mix, its weight and the time it is at when the mix is at `time`.
    fn mix_parts(&self, items: &[(Motion, f32)], by: &Number, rig: &Rig<'_>, time: f32) -> Vec<(f32, f32)> {
        let points: Vec<f32> = items.iter().map(|(_, p)| *p).collect();
        let weights = mix_weights(&points, by.read(rig.parameters));
        let whole = self.duration(rig);
        items
            .iter()
            .zip(weights)
            .map(|((m, _), w)| {
                let own = m.duration(rig);
                (w, if whole > 0.0 { time * own / whole } else { time })
            })
            .collect()
    }

    /// Writes how it poses the skeleton `time` seconds after it started over `pose`: start from
    /// the rest pose.
    pub fn sample(&self, rig: &Rig<'_>, time: f32, pose: &mut Pose) {
        match self {
            Motion::Rest => {}
            Motion::Clip { clip, speed, looping } => {
                if let Some(c) = rig.clips.get(clip) {
                    c.sample(rig.skeleton, clip_time(c.duration(), *speed, *looping, time), pose);
                }
            }
            Motion::Mix { items, by } => {
                let parts = self.mix_parts(items, by, rig, time);
                let mut mixed: Option<(Pose, f32)> = None;
                for ((motion, _), (weight, at)) in items.iter().zip(parts) {
                    if weight <= 0.0 {
                        continue;
                    }
                    let mut own = rig.skeleton.rest_pose();
                    motion.sample(rig, at, &mut own);
                    mixed = Some(match mixed {
                        None => (own, weight),
                        // Each further pose by its share of the weight so far.
                        Some((so_far, total)) => (so_far.mix(&own, weight / (total + weight)), total + weight),
                    });
                }
                if let Some((p, _)) = mixed {
                    *pose = p;
                }
            }
            Motion::Layer { base, top, joint, weight } => {
                base.sample(rig, time, pose);
                let Some(mask) = rig.skeleton.mask(joint) else { return };
                let mut over = rig.skeleton.rest_pose();
                top.sample(rig, time, &mut over);
                *pose = pose.mix_masked(&over, weight.read(rig.parameters), &mask);
            }
        }
    }

    /// The pose `time` seconds after it started.
    pub fn pose(&self, rig: &Rig<'_>, time: f32) -> Pose {
        let mut pose = rig.skeleton.rest_pose();
        self.sample(rig, time, &mut pose);
        pose
    }

    /// The names of the clip events it passes from `from` (included) to `to` (not) seconds after
    /// it started, in order: of a mix, those of its heaviest part; of a layer, those of its base
    /// and, at a weight of a half or more, of its top.
    pub fn events(&self, rig: &Rig<'_>, from: f32, to: f32, out: &mut Vec<String>) {
        if to <= from {
            return;
        }
        match self {
            Motion::Rest => {}
            Motion::Clip { clip, speed, looping } => {
                let Some(c) = rig.clips.get(clip) else { return };
                let (d, events) = (c.duration(), c.events());
                if events.is_empty() || *speed <= 0.0 {
                    return;
                }
                let (a, b) = (from * speed, to * speed);
                if !*looping || d <= 0.0 {
                    out.extend(events.iter().filter(|e| e.time >= a && e.time < b).map(|e| e.name.clone()));
                    return;
                }
                let (first, last) = ((a / d).floor() as i64, (b / d).floor() as i64);
                for round in first..=last.min(first + MAX_LOOPS) {
                    for e in events {
                        let at = round as f32 * d + e.time;
                        if at >= a && at < b {
                            out.push(e.name.clone());
                        }
                    }
                }
            }
            Motion::Mix { items, by } => {
                let (start, end) = (self.mix_parts(items, by, rig, from), self.mix_parts(items, by, rig, to));
                let heaviest = start.iter().enumerate().max_by(|a, b| a.1.0.total_cmp(&b.1.0)).map(|(i, _)| i);
                if let (Some(i), Some((motion, _))) = (heaviest, heaviest.and_then(|i| items.get(i))) {
                    let (Some((_, a)), Some((_, b))) = (start.get(i), end.get(i)) else { return };
                    motion.events(rig, *a, *b, out);
                }
            }
            Motion::Layer { base, top, weight, .. } => {
                base.events(rig, from, to, out);
                if weight.read(rig.parameters) >= 0.5 {
                    top.events(rig, from, to, out);
                }
            }
        }
    }

    /// How far joint `root` moves along the ground (X and Z of the model, with its import scale
    /// and axes) from `from` to `to` seconds after it started: across each start over of a looping
    /// clip, the moves of the clip's runs added up. A mix moves it by its parts' weights; a layer
    /// as its base does.
    pub fn root_motion(&self, rig: &Rig<'_>, root: usize, from: f32, to: f32) -> Vec3 {
        if to <= from {
            return Vec3::ZERO;
        }
        let mut delta = match self {
            Motion::Rest => Vec3::ZERO,
            Motion::Clip { clip, speed, looping } => {
                let Some(c) = rig.clips.get(clip) else { return Vec3::ZERO };
                let d = c.duration();
                let at = |t: f32| {
                    let mut pose = rig.skeleton.rest_pose();
                    c.sample(rig.skeleton, t, &mut pose);
                    model_position(rig.skeleton, &pose, root)
                };
                let (a, b) = (from * speed, to * speed);
                if d <= 0.0 {
                    Vec3::ZERO
                } else if !*looping {
                    at(b.clamp(0.0, d)) - at(a.clamp(0.0, d))
                } else {
                    let (first, last) = ((a / d).floor() as i64, (b / d).floor() as i64);
                    if first == last {
                        at(b - last as f32 * d) - at(a - first as f32 * d)
                    } else {
                        let run = at(d) - at(0.0);
                        let whole = (last - first - 1).clamp(0, MAX_LOOPS) as f32;
                        (at(d) - at(a - first as f32 * d)) + run * whole + (at(b - last as f32 * d) - at(0.0))
                    }
                }
            }
            Motion::Mix { items, by } => {
                let (start, end) = (self.mix_parts(items, by, rig, from), self.mix_parts(items, by, rig, to));
                items
                    .iter()
                    .zip(start.iter().zip(&end))
                    .map(|((m, _), ((w, a), (_, b)))| if *w > 0.0 { m.root_motion(rig, root, *a, *b) * *w } else { Vec3::ZERO })
                    .sum()
            }
            Motion::Layer { base, .. } => base.root_motion(rig, root, from, to),
        };
        delta.y = 0.0;
        delta
    }

    /// What it names that `rig` does not have: clips and joints, one line each.
    pub fn problems(&self, skeleton: &Skeleton, clips: &Clips) -> Vec<String> {
        let mut out = Vec::new();
        self.gather_problems(skeleton, clips, &mut out);
        out.dedup();
        out
    }

    fn gather_problems(&self, skeleton: &Skeleton, clips: &Clips, out: &mut Vec<String>) {
        match self {
            Motion::Rest => {}
            Motion::Clip { clip, .. } => {
                if clips.get(clip).is_none() {
                    let names: Vec<&str> = clips.names().collect();
                    out.push(format!(
                        "there is no clip named \"{clip}\"; the model has {}",
                        if names.is_empty() { "none".to_owned() } else { names.join(", ") }
                    ));
                }
            }
            Motion::Mix { items, .. } => items.iter().for_each(|(m, _)| m.gather_problems(skeleton, clips, out)),
            Motion::Layer { base, top, joint, .. } => {
                if skeleton.mask(joint).is_none() {
                    out.push(format!("there is no joint named \"{joint}\""));
                }
                base.gather_problems(skeleton, clips, out);
                top.gather_problems(skeleton, clips, out);
            }
        }
    }
}
