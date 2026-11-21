// One physics world on one Rapier build. Included twice by lib.rs: in `single` (rapier3d, f32)
// and in `double` (rapier3d-f64, f64), each of which defines `rp`, `Real`, `real` and `wide`.

struct Body {
    handle: rp::RigidBodyHandle,
    kind: BodyKind,
    /// The object's world scale when the body was made; physics moves and turns, it does not scale.
    scale: DVec3,
}

pub(crate) struct World {
    world: rp::PhysicsWorld,
    bodies: HashMap<StableId, Body>,
    owners: HashMap<rp::ColliderHandle, StableId>,
    events: Receiver<rp::CollisionEvent>,
    collector: rp::ChannelEventCollector,
    /// Time not yet stepped.
    pending: f32,
}

fn vector(v: DVec3) -> rp::Vector {
    rp::Vector::new(real(v.x), real(v.y), real(v.z))
}

fn from_vector(v: rp::Vector) -> DVec3 {
    DVec3::new(wide(v.x), wide(v.y), wide(v.z))
}

/// A world matrix as a Rapier pose (translation and rotation) and the scale it leaves out.
fn pose_of(world: DMat4) -> (rp::Pose, DVec3) {
    let (scale, rotation, translation) = world.to_scale_rotation_translation();
    let rotation = rp::Rotation::from_xyzw(real(rotation.x), real(rotation.y), real(rotation.z), real(rotation.w));
    (rp::Pose::from_parts(vector(translation), rotation), scale)
}

impl World {
    pub(crate) fn build(scene: &Scene, gravity: [f64; 3]) -> Self {
        let (collision_send, events) = channel();
        let (force_send, _) = channel();
        let (tear_send, _) = channel();
        let mut world = rp::PhysicsWorld::new();
        world.gravity = vector(DVec3::from(gravity));
        world.integration_parameters.dt = real(f64::from(STEP));
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
            .gravity_scale(real(f64::from(settings.gravity_scale)))
            .ccd_enabled(settings.kind == BodyKind::Dynamic);
            let builder = if settings.lock_rotation { builder.lock_rotations() } else { builder };
            let handle = self.world.bodies.insert(builder);
            let shape = collider.cloned().unwrap_or_default();
            let made = collider_for(&shape, scale.abs()).mass(real(f64::from(settings.mass)));
            let collider_handle = self.world.insert_collider(made, Some(handle));
            self.owners.insert(collider_handle, id);
            self.bodies.insert(id, Body { handle, kind: settings.kind, scale });
        }
    }

    pub(crate) fn step(&mut self, scene: &mut Scene, dt: f32) -> Vec<Contact> {
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
            let rotation = DQuat::from_xyzw(wide(r.x), wide(r.y), wide(r.z), wide(r.w));
            let world = DMat4::from_scale_rotation_translation(body.scale, rotation, from_vector(rb.translation()));
            let local = match scene.parent(*id).and_then(|p| scene.world_matrix(p)) {
                Some(parent) => parent.inverse() * world,
                None => world,
            };
            let transform = Transform::from_matrix(local);
            if transform.is_finite() {
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

    pub(crate) fn apply_impulse(&mut self, id: StableId, impulse: [f32; 3]) -> bool {
        let v = vector(DVec3::from(impulse.map(f64::from)));
        self.dynamic_body(id).map(|rb| rb.apply_impulse(v, true)).is_some()
    }

    pub(crate) fn set_velocity(&mut self, id: StableId, velocity: [f32; 3]) -> bool {
        let v = vector(DVec3::from(velocity.map(f64::from)));
        self.dynamic_body(id).map(|rb| rb.set_linvel(v, true)).is_some()
    }

    pub(crate) fn velocity(&self, id: StableId) -> Option<[f32; 3]> {
        let rb = self.world.bodies.get(self.bodies.get(&id)?.handle)?;
        Some(from_vector(rb.linvel()).as_vec3().to_array())
    }

    pub(crate) fn ids(&self) -> impl Iterator<Item = StableId> + '_ {
        self.bodies.keys().copied()
    }

    /// The largest distance of a moving body from the world's centre.
    pub(crate) fn farthest(&self) -> f64 {
        self.bodies
            .values()
            .filter(|b| b.kind != BodyKind::Static)
            .filter_map(|b| self.world.bodies.get(b.handle))
            .map(|rb| from_vector(rb.translation()).length())
            .fold(0.0, f64::max)
    }

    fn dynamic_body(&mut self, id: StableId) -> Option<&mut rp::RigidBody> {
        let body = self.bodies.get(&id).filter(|b| b.kind == BodyKind::Dynamic)?;
        self.world.bodies.get_mut(body.handle)
    }
}

/// The Rapier collider for a Collider component, scaled by the object's world scale.
fn collider_for(shape: &Collider, scale: DVec3) -> rp::ColliderBuilder {
    let size = DVec3::from(shape.size.map(f64::from)) * scale;
    let builder = match shape.shape {
        ColliderShape::Box => rp::ColliderBuilder::cuboid(real(size.x * 0.5), real(size.y * 0.5), real(size.z * 0.5)),
        ColliderShape::Sphere => rp::ColliderBuilder::ball(real(size.x.max(size.y).max(size.z) * 0.5)),
        ColliderShape::Capsule => {
            let radius = size.x.max(size.z) * 0.5;
            rp::ColliderBuilder::capsule_y(real((size.y * 0.5 - radius).max(0.0)), real(radius))
        }
    };
    builder
        .friction(real(f64::from(shape.friction)))
        .restitution(real(f64::from(shape.restitution)))
        .sensor(shape.trigger)
        .active_events(rp::ActiveEvents::COLLISION_EVENTS)
}
