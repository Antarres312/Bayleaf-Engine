//! A model's skeleton and animations (glTF): every node of the model's scene can move, and each
//! vertex follows up to four of them. A part without a skin follows its own node entirely; a
//! skinned part follows its skin's joints by its weights. An animation moves nodes by keys of
//! translation, rotation and scale over time; [`Skeleton::posed`] gives the model's vertices at a
//! moment of one. Cubic spline keys are read as their values, joined in straight lines.

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
}

/// A node at rest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    pub parent: Option<usize>,
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
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

impl Skeleton {
    /// The animation called `name`, or the first when `name` is empty.
    pub fn animation(&self, name: &str) -> Option<usize> {
        if name.is_empty() {
            return (!self.animations.is_empty()).then_some(0);
        }
        self.animations.iter().position(|a| a.name == name)
    }

    /// Each node's matrix into the model's space (before the import scale and axes) at `time`
    /// seconds into `animation`, or at rest.
    pub fn pose(&self, animation: Option<usize>, time: f32) -> Vec<Mat4> {
        let mut local: Vec<Node> = self.nodes.clone();
        if let Some(animation) = animation.and_then(|i| self.animations.get(i)) {
            for channel in &animation.channels {
                let (Some(node), Some(v)) = (local.get_mut(channel.node), sample(channel, time)) else { continue };
                match channel.property {
                    Property::Translation => node.translation = v.truncate(),
                    Property::Rotation => node.rotation = Quat::from_vec4(v).normalize(),
                    Property::Scale => node.scale = v.truncate(),
                }
            }
        }
        let mut world = vec![Mat4::IDENTITY; local.len()];
        for &i in &self.order {
            let Some(node) = local.get(i) else { continue };
            let own = Mat4::from_scale_rotation_translation(node.scale, node.rotation, node.translation);
            world[i] = node.parent.and_then(|p| world.get(p)).map_or(own, |parent| *parent * own);
        }
        world
    }

    /// `mesh` with its vertices where `animation` puts them at `time` (wrapped round when
    /// `looping`, else held at the end), under the id `id` and the version `version`.
    pub fn posed(&self, mesh: &MeshData, animation: Option<usize>, time: f32, looping: bool, id: u64, version: u64) -> MeshData {
        let duration = animation.and_then(|i| self.animations.get(i)).map_or(0.0, |a| a.duration);
        let time = match (looping, duration > 0.0) {
            (true, true) => time.rem_euclid(duration),
            (false, true) => time.clamp(0.0, duration),
            _ => 0.0,
        };
        let world = self.pose(animation, time);
        let matrices: Vec<Mat4> = self.bindings.iter().map(|b| world.get(b.node).copied().unwrap_or(Mat4::IDENTITY) * b.inverse_bind).collect();
        let mut positions = Vec::with_capacity(self.positions.len());
        let mut normals = Vec::with_capacity(self.positions.len());
        for (i, p) in self.positions.iter().enumerate() {
            let (bound, weights) =
                (self.vertex_bindings.get(i).copied().unwrap_or_default(), self.vertex_weights.get(i).copied().unwrap_or([1.0, 0.0, 0.0, 0.0]));
            let mut m = Mat4::ZERO;
            for k in 0..4 {
                if weights[k] > 0.0 {
                    m += matrices.get(usize::from(bound[k])).copied().unwrap_or(Mat4::IDENTITY) * weights[k];
                }
            }
            let m = self.root * m;
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
    let (first, last) = (channel.times.first()?, channel.times.last()?);
    let value = |i: usize| channel.values.get(i).copied().map(Vec4::from);
    if time <= *first {
        return value(0);
    }
    if time >= *last {
        return value(channel.times.len() - 1);
    }
    let next = channel.times.partition_point(|t| *t <= time);
    let before = next.checked_sub(1)?;
    let (a, b) = (value(before)?, value(next)?);
    if channel.step {
        return Some(a);
    }
    let span = channel.times[next] - channel.times[before];
    let t = if span > 0.0 { (time - channel.times[before]) / span } else { 0.0 };
    Some(match channel.property {
        Property::Rotation => Quat::from_vec4(a).normalize().slerp(Quat::from_vec4(b).normalize(), t).into(),
        _ => a.lerp(b, t),
    })
}
