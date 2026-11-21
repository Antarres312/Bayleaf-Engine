//! L4: rigid-body physics for play mode, on Rapier, in single or double precision.
//!
//! [`Physics::build`] makes a body for every object with a Rigid Body or a Collider (a Collider
//! alone is a static obstacle). Each [`Physics::step`] advances in fixed steps of 1/60 s, moves
//! kinematic bodies to their objects' transforms, writes dynamic bodies' places back to the scene,
//! and returns the collisions and trigger contacts that began or ended. Objects that appear or
//! disappear during play are picked up on the next step.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};

use bayleaf_core::StableId;
use bayleaf_core::math::{DMat4, DQuat, DVec3};
use bayleaf_scene::Scene;
use bayleaf_scene::components::{BodyKind, Collider, ColliderShape, RigidBody, Transform};

/// Length of one physics step, in seconds.
pub const STEP: f32 = 1.0 / 60.0;
/// Most steps one frame may take; a slower frame makes the game run slower instead of stalling.
pub const MAX_STEPS_PER_FRAME: u32 = 5;
/// Earth's gravity, in metres per second squared, pointing down.
pub const GRAVITY: [f64; 3] = [0.0, -9.81, 0.0];

/// What began or ended between two objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactKind {
    CollisionStart,
    CollisionEnd,
    TriggerEnter,
    TriggerExit,
}

/// A contact, reported once to each of the two objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Contact {
    pub object: StableId,
    pub other: StableId,
    pub kind: ContactKind,
}

/// How precisely physics counts positions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Precision {
    /// Single precision (f32): faster, exact to about a millimetre within 10 km of the centre.
    #[default]
    Single,
    /// Double precision (f64): about a third slower, exact far from the centre.
    Double,
}

/// Distance from the world's centre beyond which single-precision physics loses accuracy.
pub const SINGLE_PRECISION_RANGE: f64 = 10_000.0;

mod single {
    use super::{
        BodyKind, Collider, ColliderShape, Contact, ContactKind, DMat4, DQuat, DVec3, HashMap, MAX_STEPS_PER_FRAME, Receiver, RigidBody, STEP, Scene, StableId,
        Transform, channel,
    };
    use rapier3d::prelude as rp;

    type Real = f32;

    fn real(v: f64) -> Real {
        v as f32
    }

    fn wide(v: Real) -> f64 {
        f64::from(v)
    }

    include!("backend.rs");
}

mod double {
    use super::{
        BodyKind, Collider, ColliderShape, Contact, ContactKind, DMat4, DQuat, DVec3, HashMap, MAX_STEPS_PER_FRAME, Receiver, RigidBody, STEP, Scene, StableId,
        Transform, channel,
    };
    use rapier3d_f64::prelude as rp;

    type Real = f64;

    fn real(v: f64) -> Real {
        v
    }

    fn wide(v: Real) -> f64 {
        v
    }

    include!("backend.rs");
}

enum Backend {
    // Boxed: each world is several kilobytes.
    Single(Box<single::World>),
    Double(Box<double::World>),
}

/// The physics world of one play session.
pub struct Physics {
    backend: Backend,
}

macro_rules! both {
    ($self:expr, $world:ident => $body:expr) => {
        match &$self.backend {
            Backend::Single($world) => $body,
            Backend::Double($world) => $body,
        }
    };
    (mut $self:expr, $world:ident => $body:expr) => {
        match &mut $self.backend {
            Backend::Single($world) => $body,
            Backend::Double($world) => $body,
        }
    };
}

impl Physics {
    /// A world with `gravity` and a body for every object that has a Rigid Body or a Collider.
    pub fn build(scene: &Scene, gravity: [f64; 3], precision: Precision) -> Self {
        let backend = match precision {
            Precision::Single => Backend::Single(Box::new(single::World::build(scene, gravity))),
            Precision::Double => Backend::Double(Box::new(double::World::build(scene, gravity))),
        };
        Self { backend }
    }

    pub fn precision(&self) -> Precision {
        match self.backend {
            Backend::Single(_) => Precision::Single,
            Backend::Double(_) => Precision::Double,
        }
    }

    /// Advances by `dt` seconds in fixed steps. Kinematic bodies follow their objects' transforms;
    /// dynamic bodies' places are written back to the scene. Returns the contacts that began or ended.
    pub fn step(&mut self, scene: &mut Scene, dt: f32) -> Vec<Contact> {
        both!(mut self, w => w.step(scene, dt))
    }

    /// Pushes a dynamic body by an impulse (mass times change of velocity), in world space.
    pub fn apply_impulse(&mut self, id: StableId, impulse: [f32; 3]) -> bool {
        both!(mut self, w => w.apply_impulse(id, impulse))
    }

    /// Sets a dynamic body's velocity, in world units per second.
    pub fn set_velocity(&mut self, id: StableId, velocity: [f32; 3]) -> bool {
        both!(mut self, w => w.set_velocity(id, velocity))
    }

    /// A body's velocity, if the object has one.
    pub fn velocity(&self, id: StableId) -> Option<[f32; 3]> {
        both!(self, w => w.velocity(id))
    }

    /// Every body's velocity, for scripts to read during a step.
    pub fn velocities(&self) -> HashMap<StableId, [f32; 3]> {
        both!(self, w => w.ids().filter_map(|id| Some((id, w.velocity(id)?))).collect())
    }

    /// True when single precision is used and a moving body is farther than
    /// [`SINGLE_PRECISION_RANGE`] from the centre, where it starts to lose accuracy.
    pub fn out_of_range(&self) -> bool {
        self.precision() == Precision::Single && self.farthest() > SINGLE_PRECISION_RANGE
    }

    /// The largest distance of a moving body from the world's centre.
    pub fn farthest(&self) -> f64 {
        both!(self, w => w.farthest())
    }

    /// Number of bodies.
    pub fn len(&self) -> usize {
        both!(self, w => w.ids().count())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
#[cfg(test)]
mod tests;
