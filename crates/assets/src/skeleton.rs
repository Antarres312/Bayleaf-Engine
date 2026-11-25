//! A model's skeleton and animations (glTF): every node of the model's scene can move, and each
//! vertex follows up to four of them. A part without a skin follows its own node entirely; a
//! skinned part follows its skin's joints by its weights. An animation moves nodes by keys of
//! translation, rotation and scale over time; [`Skeleton::posed`] gives the model's vertices at a
//! moment of one. Cubic spline keys are read as their values, joined in straight lines.
//!
//! A [`Pose`] is every node's place against its parent. Poses mix ([`Pose::mix`]), also on part of
//! the skeleton only ([`Pose::mix_masked`], with a mask from [`Skeleton::mask`]: a joint and its
//! children). [`Skeleton::world`] turns a pose into each node's matrix in the model, and
//! [`Skeleton::skin_matrices`] into the matrices vertices follow: the renderer sends those to the
//! GPU, which moves the vertices there; [`Skeleton::skin_on_cpu`] does the same on the CPU (tests,
//! and drawing without GPU skinning).

use glam::{Mat3, Mat4, Quat, Vec3, Vec4};

use crate::MeshData;

/// Most keys one model's animations may hold in all.
pub const MAX_KEYS: usize = 4_000_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Skeleton {
    /// Every node of the model, by its index in the file.
    pub nodes: Vec<Node>,
    /// The nodes in an order with each parent before its children.
    pub order: Vec<usize>,
    /// What vertices are bound to: a node, and the matrix from the bound pose into its space.
    pub bindings: Vec<Binding>,
    /// For each vertex of the model: up to four bindings and their weights (adding up to 1).
    pub vertex_bindings: Vec<[u16; 4]>,
    pub vertex_weights: Vec<[f32; 4]>,
    /// The vertices in the bound pose, before the import scale and axes.
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// The import scale and axes, applied after the pose.
    pub root: Mat4,
    pub animations: Vec<Animation>,
    /// Each node's name in the file, by index (`Node 4` for the fourth when it has none).
    pub names: Vec<String>,
    /// For each binding, the box (smallest and largest corner) round the bound-pose vertices that
    /// follow it, so a posed model's box is found without moving every vertex. See
    /// [`Skeleton::bounds_of_bindings`].
    pub binding_bounds: Vec<([f32; 3], [f32; 3])>,
}

/// A node at rest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    pub parent: Option<usize>,
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Node {
    pub fn transform(&self) -> Transform {
        Transform { translation: self.translation, rotation: self.rotation, scale: self.scale }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Binding {
    pub node: usize,
    pub inverse_bind: Mat4,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Animation {
    pub name: String,
    /// Seconds, to its last key.
    pub duration: f32,
    pub channels: Vec<Channel>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Property {
    Translation,
    Rotation,
    Scale,
}

/// Keys of one property of one node.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub node: usize,
    pub property: Property,
    /// Holds each key until the next instead of moving between them.
    pub step: bool,
    /// Seconds, rising.
    pub times: Vec<f32>,
    /// x, y, z (and w for a rotation) of each key.
    pub values: Vec<[f32; 4]>,
}

/// A node's place against its parent: scaled, turned, then moved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Transform {
    pub const IDENTITY: Transform = Transform { translation: Vec3::ZERO, rotation: Quat::IDENTITY, scale: Vec3::ONE };

    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// From `self` (0) to `other` (1) by `t`: places and scales in a straight line, the turn along
    /// the shorter arc.
    pub fn mix(&self, other: &Transform, t: f32) -> Transform {
        Transform {
            translation: self.translation.lerp(other.translation, t),
            rotation: self.rotation.slerp(other.rotation, t).normalize(),
            scale: self.scale.lerp(other.scale, t),
        }
    }
}

/// Every node's place against its parent, by node index.
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    pub locals: Vec<Transform>,
}

/// `t` as a mixing factor: 0 to 1, and 0 for a number that is not one.
fn factor(t: f32) -> f32 {
    if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 }
}

impl Pose {
    /// `self` mixed towards `other` by `t` (0 all `self`, 1 all `other`), node by node.
    pub fn mix(&self, other: &Pose, t: f32) -> Pose {
        let t = factor(t);
        let locals = self.locals.iter().enumerate().map(|(i, a)| other.locals.get(i).map_or(*a, |b| a.mix(b, t))).collect();
        Pose { locals }
    }

    /// Like [`Pose::mix`], each node by `t` times its weight in `mask` (a node the mask leaves at 0
    /// keeps `self`'s place).
    pub fn mix_masked(&self, other: &Pose, t: f32, mask: &[f32]) -> Pose {
        let t = factor(t);
        let locals = self
            .locals
            .iter()
            .enumerate()
            .map(|(i, a)| match (other.locals.get(i), mask.get(i)) {
                (Some(b), Some(w)) if *w > 0.0 => a.mix(b, t * w.min(1.0)),
                _ => *a,
            })
            .collect();
        Pose { locals }
    }
}

impl Skeleton {
    /// The animation called `name`, or the first when `name` is empty.
    pub fn animation(&self, name: &str) -> Option<usize> {
        if name.is_empty() {
            return (!self.animations.is_empty()).then_some(0);
        }
        self.animations.iter().position(|a| a.name == name)
    }

    /// Every node at rest.
    pub fn rest_pose(&self) -> Pose {
        Pose { locals: self.nodes.iter().map(Node::transform).collect() }
    }

    /// The node called `name`.
    pub fn node_named(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }

    /// 1 for the node called `joint` and every node under it, 0 for the others: what a layer that
    /// plays on that part of the skeleton moves. All 1 when `joint` is empty; `None` when no node
    /// has that name.
    pub fn mask(&self, joint: &str) -> Option<Vec<f32>> {
        if joint.is_empty() {
            return Some(vec![1.0; self.nodes.len()]);
        }
        let top = self.node_named(joint)?;
        let mut mask = vec![0.0; self.nodes.len()];
        // Parents come before their children in `order`, so one pass reaches every descendant.
        for &i in &self.order {
            let parent_inside = self.nodes.get(i).and_then(|n| n.parent).is_some_and(|p| mask.get(p).is_some_and(|w| *w > 0.0));
            if (i == top || parent_inside)
                && let Some(slot) = mask.get_mut(i)
            {
                *slot = 1.0;
            }
        }
        Some(mask)
    }

    /// The time `animation` is at after `time` seconds: wrapped round when `looping`, else held at
    /// its end; 0 for an animation of no length.
    pub fn clip_time(&self, animation: Option<usize>, time: f32, looping: bool) -> f32 {
        let duration = animation.and_then(|i| self.animations.get(i)).map_or(0.0, |a| a.duration);
        match (looping, duration > 0.0 && time.is_finite()) {
            (true, true) => time.rem_euclid(duration),
            (false, true) => time.clamp(0.0, duration),
            _ => 0.0,
        }
    }

    /// `animation`'s keys at `time` seconds written over `pose` (the nodes it does not move keep
    /// their place). The time is used as given: wrap or hold it first ([`Skeleton::clip_time`]).
    pub fn sample_into(&self, animation: usize, time: f32, pose: &mut Pose) {
        let Some(animation) = self.animations.get(animation) else { return };
        for channel in &animation.channels {
            let (Some(node), Some(v)) = (pose.locals.get_mut(channel.node), sample(channel, time)) else { continue };
            match channel.property {
                Property::Translation => node.translation = v.truncate(),
                Property::Rotation => node.rotation = Quat::from_vec4(v).normalize(),
                Property::Scale => node.scale = v.truncate(),
            }
        }
    }

    /// The pose `animation` gives at `time` seconds: the rest pose with its keys written over it.
    pub fn sampled(&self, animation: usize, time: f32) -> Pose {
        let mut pose = self.rest_pose();
        self.sample_into(animation, time, &mut pose);
        pose
    }

    /// Each node's matrix into the model's space (before the import scale and axes) in `pose`.
    pub fn world(&self, pose: &Pose) -> Vec<Mat4> {
        let mut world = vec![Mat4::IDENTITY; self.nodes.len()];
        for &i in &self.order {
            let own = pose.locals.get(i).map_or(Mat4::IDENTITY, Transform::matrix);
            let parent = self.nodes.get(i).and_then(|n| n.parent).and_then(|p| world.get(p)).copied();
            if let Some(slot) = world.get_mut(i) {
                *slot = parent.map_or(own, |parent| parent * own);
            }
        }
        world
    }

    /// Each node's matrix into the model's space (before the import scale and axes) at `time`
    /// seconds into `animation`, or at rest.
    pub fn pose(&self, animation: Option<usize>, time: f32) -> Vec<Mat4> {
        let pose = animation.map_or_else(|| self.rest_pose(), |a| self.sampled(a, time));
        self.world(&pose)
    }

    /// The matrix each binding moves its vertices by, from the bound pose to the posed model
    /// (with the import scale and axes), given `world` from [`Skeleton::world`]: what the vertex
    /// shader reads.
    pub fn skin_matrices(&self, world: &[Mat4]) -> Vec<Mat4> {
        self.bindings.iter().map(|b| self.root * world.get(b.node).copied().unwrap_or(Mat4::IDENTITY) * b.inverse_bind).collect()
    }

    /// The box round the model posed by `matrices` (from [`Skeleton::skin_matrices`]): each
    /// binding's own box moved by its matrix, together. It may be larger than the model, never
    /// smaller.
    pub fn skinned_bounds(&self, matrices: &[Mat4]) -> Option<(Vec3, Vec3)> {
        let (mut min, mut max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for ((lo, hi), m) in self.binding_bounds.iter().zip(matrices) {
            if lo[0] > hi[0] {
                continue;
            }
            for corner in 0..8 {
                let pick = |bit: usize, axis: usize| if corner & bit == 0 { lo[axis] } else { hi[axis] };
                let p = m.transform_point3(Vec3::new(pick(1, 0), pick(2, 1), pick(4, 2)));
                min = min.min(p);
                max = max.max(p);
            }
        }
        (min.is_finite() && max.is_finite()).then_some((min, max))
    }

    /// The box round the vertices that follow each binding, in the bound pose; a binding no
    /// vertex follows gets an empty box (its smallest corner above its largest).
    pub fn bounds_of_bindings(&self) -> Vec<([f32; 3], [f32; 3])> {
        let mut bounds = vec![(Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)); self.bindings.len()];
        for (i, p) in self.positions.iter().enumerate() {
            let (bound, weights) = (self.vertex_bindings.get(i).copied().unwrap_or_default(), self.vertex_weights.get(i).copied().unwrap_or_default());
            for k in 0..4 {
                if weights[k] > 0.0
                    && let Some((lo, hi)) = bounds.get_mut(usize::from(bound[k]))
                {
                    *lo = lo.min(Vec3::from(*p));
                    *hi = hi.max(Vec3::from(*p));
                }
            }
        }
        bounds.into_iter().map(|(lo, hi)| (lo.to_array(), hi.to_array())).collect()
    }

    /// `mesh` with its vertices where `animation` puts them at `time` (wrapped round when
    /// `looping`, else held at the end), under the id `id` and the version `version`.
    pub fn posed(&self, mesh: &MeshData, animation: Option<usize>, time: f32, looping: bool, id: u64, version: u64) -> MeshData {
        let time = self.clip_time(animation, time, looping);
        let matrices = self.skin_matrices(&self.pose(animation, time));
        self.skin_on_cpu(mesh, &matrices, id, version)
    }

    /// `mesh` with each vertex moved by its bindings' `matrices` (from [`Skeleton::skin_matrices`])
    /// by their weights, under the id `id` and the version `version`: what the GPU does when it
    /// skins, done on the CPU.
    pub fn skin_on_cpu(&self, mesh: &MeshData, matrices: &[Mat4], id: u64, version: u64) -> MeshData {
        let mut positions = Vec::with_capacity(self.positions.len());
        let mut normals = Vec::with_capacity(self.positions.len());
        for (i, p) in self.positions.iter().enumerate() {
            let (bound, weights) =
                (self.vertex_bindings.get(i).copied().unwrap_or_default(), self.vertex_weights.get(i).copied().unwrap_or([1.0, 0.0, 0.0, 0.0]));
            let mut m = Mat4::ZERO;
            for k in 0..4 {
                if weights[k] > 0.0 {
                    m += matrices.get(usize::from(bound[k])).copied().unwrap_or(self.root) * weights[k];
                }
            }
            positions.push(m.transform_point3(Vec3::from(*p)).to_array());
            let n = self.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]);
            normals.push((Mat3::from_mat4(m).inverse().transpose() * Vec3::from(n)).normalize_or(Vec3::Y).to_array());
        }
        let (mut min, mut max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for p in &positions {
            min = min.min(Vec3::from(*p));
            max = max.max(Vec3::from(*p));
        }
        MeshData { id, version, positions, normals, min: min.to_array(), max: max.to_array(), ..mesh.clone() }
    }
}

/// A channel's value at `time`: the first key before it, the last after it, else between the two
/// keys round it (a rotation along the shorter arc).
fn sample(channel: &Channel, time: f32) -> Option<Vec4> {
    sample_keys(channel.property, channel.step, &channel.times, &channel.values, time)
}

/// The value keys of `property` at `times` (rising) with `values` give at `time`, as a channel's
/// are read: held at the ends, held from key to key when `step`, else between the two keys round
/// it (a rotation along the shorter arc).
pub fn sample_keys(property: Property, step: bool, times: &[f32], values: &[[f32; 4]], time: f32) -> Option<Vec4> {
    let (first, last) = (times.first()?, times.last()?);
    let value = |i: usize| values.get(i).copied().map(Vec4::from);
    if time <= *first {
        return value(0);
    }
    if time >= *last {
        return value(times.len() - 1);
    }
    let next = times.partition_point(|t| *t <= time);
    let before = next.checked_sub(1)?;
    let (a, b) = (value(before)?, value(next)?);
    if step {
        return Some(a);
    }
    let span = times.get(next)? - times.get(before)?;
    let t = if span > 0.0 { (time - times[before]) / span } else { 0.0 };
    Some(match property {
        Property::Rotation => Quat::from_vec4(a).normalize().slerp(Quat::from_vec4(b).normalize(), t).into(),
        _ => a.lerp(b, t),
    })
}
