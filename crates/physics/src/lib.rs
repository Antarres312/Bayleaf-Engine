//! L4: rigid-body physics for play mode, on Rapier.
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
use bayleaf_core::math::{EulerRot, Mat4, Quat, Vec3};
use bayleaf_scene::Scene;
use bayleaf_scene::components::{BodyKind, Collider, ColliderShape, RigidBody, Transform};
use rapier3d::prelude as rp;

/// Length of one physics step, in seconds.
pub const STEP: f32 = 1.0 / 60.0;
/// Most steps one frame may take; a slower frame makes the game run slower instead of stalling.
pub const MAX_STEPS_PER_FRAME: u32 = 5;
/// Earth's gravity, in metres per second squared, pointing down.
pub const GRAVITY: [f32; 3] = [0.0, -9.81, 0.0];

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

struct Body {
    handle: rp::RigidBodyHandle,
    kind: BodyKind,
    /// The object's world scale when the body was made; physics moves and turns, it does not scale.
    scale: Vec3,
}

/// The physics world of one play session.
pub struct Physics {
    world: rp::PhysicsWorld,
    bodies: HashMap<StableId, Body>,
    owners: HashMap<rp::ColliderHandle, StableId>,
    events: Receiver<rp::CollisionEvent>,
    collector: rp::ChannelEventCollector,
    /// Time not yet stepped.
    pending: f32,
}

fn to_rapier(v: Vec3) -> rp::Vector {
    rp::Vector::new(v.x, v.y, v.z)
}

fn from_rapier(v: rp::Vector) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

fn pose_of(world: Mat4) -> (rp::Pose, Vec3) {
    let (scale, rotation, translation) = world.to_scale_rotation_translation();
    let rotation = rp::Rotation::from_xyzw(rotation.x, rotation.y, rotation.z, rotation.w);
    (rp::Pose::from_parts(to_rapier(translation), rotation), scale)
}

impl Physics {
    /// A world with `gravity` and a body for every object that has a Rigid Body or a Collider.
    pub fn build(scene: &Scene, gravity: [f32; 3]) -> Self {
        let (collision_send, events) = channel();
        let (force_send, _) = channel();
        let (tear_send, _) = channel();
        let mut world = rp::PhysicsWorld::new();
        world.gravity = rp::Vector::new(gravity[0], gravity[1], gravity[2]);
        world.integration_parameters.dt = STEP;
        let mut physics = Self {
            world,
            bodies: HashMap::new(),
            owners: HashMap::new(),
            events,
            collector: rp::ChannelEventCollector::new(collision_send, force_send, tear_send),
            pending: 0.0,
        };
        physics.sync_objects(scene);
        physics
    }

    /// Bodies for objects that gained physics, none for objects that are gone.
    fn sync_objects(&mut self, scene: &Scene) {
        let gone: Vec<StableId> = self.bodies.keys().filter(|id| !scene.contains(**id)).copied().collect();
        for id in gone {
            if let Some(body) = self.bodies.remove(&id) {
                // Bodies that touched it wake up and fall.
                self.world.remove_body_with_colliders(body.handle, true);
            }
        }
        self.owners.retain(|handle, _| self.world.colliders.get(*handle).is_some());
        for (id, world) in scene.world_matrices() {
            if self.bodies.contains_key(&id) {
                continue;
            }
            let body = scene.get::<RigidBody>(id);
            let collider = scene.get::<Collider>(id);
            if body.is_none() && collider.is_none() {
                continue;
            }
            let settings = body.cloned().unwrap_or(RigidBody { kind: BodyKind::Static, ..RigidBody::default() });
            let (pose, scale) = pose_of(world);
            let builder = match settings.kind {
                BodyKind::Dynamic => rp::RigidBodyBuilder::dynamic(),
                BodyKind::Kinematic => rp::RigidBodyBuilder::kinematic_position_based(),
                BodyKind::Static => rp::RigidBodyBuilder::fixed(),
            }
            .pose(pose)
            .gravity_scale(settings.gravity_scale)
            .ccd_enabled(settings.kind == BodyKind::Dynamic);
            let builder = if settings.lock_rotation { builder.lock_rotations() } else { builder };
            let handle = self.world.bodies.insert(builder);
            let shape = collider.cloned().unwrap_or_default();
            let made = collider_for(&shape, scale.abs()).mass(settings.mass);
            let collider_handle = self.world.insert_collider(made, Some(handle));
            self.owners.insert(collider_handle, id);
            self.bodies.insert(id, Body { handle, kind: settings.kind, scale });
        }
    }

    /// Advances by `dt` seconds in fixed steps. Kinematic bodies follow their objects' transforms;
    /// dynamic bodies' places are written back to the scene. Returns the contacts that began or ended.
    pub fn step(&mut self, scene: &mut Scene, dt: f32) -> Vec<Contact> {
        self.sync_objects(scene);
        for (id, body) in &self.bodies {
            if body.kind == BodyKind::Kinematic
                && let Some(world) = scene.world_matrix(*id)
                && let Some(rb) = self.world.bodies.get_mut(body.handle)
            {
                rb.set_next_kinematic_position(pose_of(world).0);
            }
        }
        self.pending = (self.pending + dt.max(0.0)).min(STEP * MAX_STEPS_PER_FRAME as f32);
        while self.pending >= STEP {
            self.world.step_with_events(&(), &self.collector);
            self.pending -= STEP;
        }
        for (id, body) in &self.bodies {
            if body.kind != BodyKind::Dynamic {
                continue;
            }
            let Some(rb) = self.world.bodies.get(body.handle) else { continue };
            let r = rb.rotation();
            let rotation = Quat::from_xyzw(r.x, r.y, r.z, r.w);
            let world = Mat4::from_scale_rotation_translation(body.scale, rotation, from_rapier(rb.translation()));
            let local = match scene.parent(*id).and_then(|p| scene.world_matrix(p)) {
                Some(parent) => parent.inverse() * world,
                None => world,
            };
            let (scale, rotation, position) = local.to_scale_rotation_translation();
            let (rx, ry, rz) = rotation.to_euler(EulerRot::XYZ);
            let transform = Transform { position: position.to_array(), rotation: [rx.to_degrees(), ry.to_degrees(), rz.to_degrees()], scale: scale.to_array() };
            if transform.position.iter().chain(&transform.rotation).all(|v| v.is_finite()) {
                let _ = scene.set(*id, transform);
            }
        }
        let mut contacts = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            let (a, b, started) = match event {
                rp::CollisionEvent::Started(a, b, _) => (a, b, true),
                rp::CollisionEvent::Stopped(a, b, _) => (a, b, false),
            };
            let (Some(&first), Some(&second)) = (self.owners.get(&a), self.owners.get(&b)) else { continue };
            let sensor = [a, b].iter().any(|h| self.world.colliders.get(*h).is_some_and(rp::Collider::is_sensor));
            let kind = match (sensor, started) {
                (false, true) => ContactKind::CollisionStart,
                (false, false) => ContactKind::CollisionEnd,
                (true, true) => ContactKind::TriggerEnter,
                (true, false) => ContactKind::TriggerExit,
            };
            contacts.push(Contact { object: first, other: second, kind });
            contacts.push(Contact { object: second, other: first, kind });
        }
        contacts
    }

    /// Pushes a dynamic body by an impulse (mass times change of velocity), in world space.
    pub fn apply_impulse(&mut self, id: StableId, impulse: [f32; 3]) -> bool {
        self.dynamic_body(id).map(|rb| rb.apply_impulse(rp::Vector::new(impulse[0], impulse[1], impulse[2]), true)).is_some()
    }

    /// Sets a dynamic body's velocity, in world units per second.
    pub fn set_velocity(&mut self, id: StableId, velocity: [f32; 3]) -> bool {
        self.dynamic_body(id).map(|rb| rb.set_linvel(rp::Vector::new(velocity[0], velocity[1], velocity[2]), true)).is_some()
    }

    /// A body's velocity, if the object has one.
    pub fn velocity(&self, id: StableId) -> Option<[f32; 3]> {
        let rb = self.world.bodies.get(self.bodies.get(&id)?.handle)?;
        Some(from_rapier(rb.linvel()).to_array())
    }

    /// Every body's velocity, for scripts to read during a step.
    pub fn velocities(&self) -> HashMap<StableId, [f32; 3]> {
        self.bodies.keys().filter_map(|id| Some((*id, self.velocity(*id)?))).collect()
    }

    fn dynamic_body(&mut self, id: StableId) -> Option<&mut rp::RigidBody> {
        let body = self.bodies.get(&id).filter(|b| b.kind == BodyKind::Dynamic)?;
        self.world.bodies.get_mut(body.handle)
    }

    /// Number of bodies.
    pub fn len(&self) -> usize {
        self.bodies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bodies.is_empty()
    }
}

/// The Rapier collider for a Collider component, scaled by the object's world scale.
fn collider_for(shape: &Collider, scale: Vec3) -> rp::ColliderBuilder {
    let size = Vec3::from(shape.size) * scale;
    let builder = match shape.shape {
        ColliderShape::Box => rp::ColliderBuilder::cuboid(size.x * 0.5, size.y * 0.5, size.z * 0.5),
        ColliderShape::Sphere => rp::ColliderBuilder::ball(size.x.max(size.y).max(size.z) * 0.5),
        ColliderShape::Capsule => {
            let radius = size.x.max(size.z) * 0.5;
            rp::ColliderBuilder::capsule_y((size.y * 0.5 - radius).max(0.0), radius)
        }
    };
    builder.friction(shape.friction).restitution(shape.restitution).sensor(shape.trigger).active_events(rp::ActiveEvents::COLLISION_EVENTS)
}

#[cfg(test)]
mod tests;
