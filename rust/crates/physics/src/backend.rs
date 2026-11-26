// One physics world on one Rapier build. Included twice by lib.rs: in `single` (rapier3d, f32)
// and in `double` (rapier3d-f64, f64), each of which defines `rp`, `Real`, `real` and `wide`.

struct Body {
    handle: rp::RigidBodyHandle,
    kind: BodyKind,
    /// The object's world scale when the body was made; physics moves and turns, it does not scale.
    scale: DVec3,
    /// A walking character, moved by its controller rather than by forces.
    character: Option<Walker>,
}

/// A character's controller and what scripts asked of it.
struct Walker {
    controller: ctl::KinematicCharacterController,
    shape: rp::SharedShape,
    /// Walking velocity scripts set, world units per second.
    walk: DVec3,
    /// Upward speed from jumping and falling.
    vertical: f64,
    grounded: bool,
}

pub(crate) struct World {
    world: rp::PhysicsWorld,
    bodies: HashMap<StableId, Body>,
    owners: HashMap<rp::ColliderHandle, StableId>,
    events: Receiver<rp::CollisionEvent>,
    collector: rp::ChannelEventCollector,
    /// Time not yet stepped.
    pending: f32,
    /// Joints made, by the object whose Joint asked for them, with the settings they were made with.
    joints: HashMap<StableId, (rp::ImpulseJointHandle, Joint)>,
    /// A fixed body at the origin that joints to the world hold on to, once one does.
    ground: Option<rp::RigidBodyHandle>,
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
            joints: HashMap::new(),
            ground: None,
        };
        #[cfg(not(target_arch = "wasm32"))]
        physics.world.set_thread_pool(crate::pool::shared());
        physics.sync_objects(scene);
        // Scene queries (raycasts) work before the first step: a step of no time fills them in.
        physics.world.integration_parameters.dt = real(0.0);
        physics.world.step_with_events(&(), &physics.collector);
        physics.world.integration_parameters.dt = real(f64::from(STEP));
        physics
    }

    /// Bodies for objects that gained physics, none for objects that are gone.
    fn sync_objects(&mut self, scene: &Scene) {
        // In id order, here and for joints: a HashMap's order changes from run to run, and the
        // order bodies and joints come and go in decides Rapier's handles and so its solving order.
        let mut gone: Vec<StableId> = self.bodies.keys().filter(|id| !scene.contains(**id)).copied().collect();
        gone.sort_unstable();
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
            let (pose, scale) = pose_of(world);
            if let Some(character) = scene.get::<CharacterBody>(id) {
                self.add_character(id, character, pose, scale);
                continue;
            }
            let body = scene.get::<RigidBody>(id);
            let collider = scene.get::<Collider>(id);
            if body.is_none() && collider.is_none() {
                continue;
            }
            let settings = body.cloned().unwrap_or(RigidBody { kind: BodyKind::Static, ..RigidBody::default() });
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
            self.bodies.insert(id, Body { handle, kind: settings.kind, scale, character: None });
        }
        self.sync_joints(scene);
    }

    pub(crate) fn step(&mut self, scene: &mut Scene, dt: f32) -> Vec<Contact> {
        self.sync_objects(scene);
        for (id, body) in &self.bodies {
            if body.kind == BodyKind::Kinematic
                && body.character.is_none()
                && let Some(world) = scene.world_matrix(*id)
                && let Some(rb) = self.world.bodies.get_mut(body.handle)
            {
                rb.set_next_kinematic_position(pose_of(world).0);
            }
        }
        self.pending = (self.pending + dt.max(0.0)).min(STEP * MAX_STEPS_PER_FRAME as f32);
        while self.pending >= STEP {
            self.walk_characters();
            self.world.step_with_events(&(), &self.collector);
            self.pending -= STEP;
        }
        for (id, body) in &self.bodies {
            if body.kind != BodyKind::Dynamic && body.character.is_none() {
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
            let mut transform = Transform::from_matrix(local);
            // A character only moves: its turn and size stay as the scene (and Player Control) set them.
            if body.character.is_some()
                && let Some(own) = scene.get::<Transform>(*id)
            {
                transform.rotation = own.rotation;
                transform.scale = own.scale;
            }
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

    /// Gives this world its own pool of worker threads, or the shared one again with `None`.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn set_pool(&mut self, pool: Option<std::sync::Arc<crate::pool::ThreadPool>>) {
        self.world.set_thread_pool(pool.or_else(crate::pool::shared));
    }

    /// The worker threads a step runs on.
    pub(crate) fn threads(&self) -> usize {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(threads) = self.world.num_threads() {
            return threads;
        }
        1
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

    fn add_character(&mut self, id: StableId, character: &CharacterBody, pose: rp::Pose, scale: DVec3) {
        let radius = f64::from(character.radius);
        let half = (f64::from(character.height) * 0.5 - radius).max(0.0);
        let shape = rp::SharedShape::capsule_y(real(half), real(radius));
        let handle = self.world.bodies.insert(rp::RigidBodyBuilder::kinematic_position_based().pose(pose));
        let collider = rp::ColliderBuilder::new(shape.clone()).active_events(rp::ActiveEvents::COLLISION_EVENTS);
        let collider_handle = self.world.insert_collider(collider, Some(handle));
        self.owners.insert(collider_handle, id);
        let slope = real(f64::from(character.max_slope).to_radians());
        let controller = ctl::KinematicCharacterController {
            up: rp::Vector::new(real(0.0), real(1.0), real(0.0)),
            max_slope_climb_angle: slope,
            min_slope_slide_angle: slope,
            autostep: Some(ctl::CharacterAutostep {
                max_height: ctl::CharacterLength::Absolute(real(f64::from(character.step_height))),
                min_width: ctl::CharacterLength::Absolute(real(radius * 0.5)),
                include_dynamic_bodies: false,
            }),
            snap_to_ground: Some(ctl::CharacterLength::Absolute(real(0.1))),
            ..ctl::KinematicCharacterController::default()
        };
        let walker = Walker { controller, shape, walk: DVec3::ZERO, vertical: 0.0, grounded: false };
        self.bodies.insert(id, Body { handle, kind: BodyKind::Kinematic, scale, character: Some(walker) });
    }

    /// Moves every character by one step: its walking velocity, its jump and gravity, stopped and
    /// slid by what it meets.
    fn walk_characters(&mut self) {
        let dt = f64::from(STEP);
        let gravity = from_vector(self.world.gravity);
        let mut moves = Vec::new();
        for body in self.bodies.values_mut() {
            let Some(walker) = body.character.as_mut() else { continue };
            let Some(rb) = self.world.bodies.get(body.handle) else { continue };
            walker.vertical += gravity.y * dt;
            let desired = DVec3::new(walker.walk.x, walker.walk.y + walker.vertical, walker.walk.z) * dt;
            let filter = rp::QueryFilter::default().exclude_rigid_body(body.handle).exclude_sensors();
            let queries = self.world.query_pipeline_with_filter(filter);
            let pose = *rb.position();
            let moved = walker.controller.move_shape(real(dt), &queries, &*walker.shape, &pose, vector(desired), |_| {});
            walker.grounded = moved.grounded;
            if moved.grounded && walker.vertical < 0.0 {
                walker.vertical = 0.0;
            }
            moves.push((body.handle, rb.translation() + moved.translation));
        }
        for (handle, target) in moves {
            if let Some(rb) = self.world.bodies.get_mut(handle) {
                rb.set_next_kinematic_translation(target);
            }
        }
    }

    /// Sets the walking velocity of a character. False when the object is not one.
    pub(crate) fn walk(&mut self, id: StableId, velocity: [f32; 3]) -> bool {
        let Some(walker) = self.bodies.get_mut(&id).and_then(|b| b.character.as_mut()) else { return false };
        walker.walk = DVec3::from(velocity.map(f64::from));
        true
    }

    /// Makes a character on the ground jump with an upward speed. False when it is not a character
    /// or is in the air.
    pub(crate) fn jump(&mut self, id: StableId, speed: f32) -> bool {
        let Some(walker) = self.bodies.get_mut(&id).and_then(|b| b.character.as_mut()) else { return false };
        if !walker.grounded {
            return false;
        }
        walker.vertical = f64::from(speed);
        walker.grounded = false;
        true
    }

    /// Moves a body to a world place at once, stopping it (a character also stops falling).
    pub(crate) fn teleport(&mut self, id: StableId, world: DMat4) -> bool {
        let Some(body) = self.bodies.get_mut(&id) else { return false };
        if let Some(walker) = body.character.as_mut() {
            walker.vertical = 0.0;
        }
        let Some(rb) = self.world.bodies.get_mut(body.handle) else { return false };
        let pose = pose_of(world).0;
        rb.set_position(pose, true);
        rb.set_next_kinematic_position(pose);
        rb.set_linvel(vector(DVec3::ZERO), true);
        true
    }

    pub(crate) fn is_character(&self, id: StableId) -> bool {
        self.bodies.get(&id).is_some_and(|b| b.character.is_some())
    }

    /// Characters standing on something.
    pub(crate) fn grounded(&self) -> Vec<StableId> {
        self.bodies.iter().filter(|(_, b)| b.character.as_ref().is_some_and(|w| w.grounded)).map(|(id, _)| *id).collect()
    }

    /// The first collider a ray from `origin` along `direction` (unit length) hits within `max`,
    /// leaving out `skip`'s own body and triggers: the object, the distance and the surface normal.
    pub(crate) fn raycast(&self, origin: DVec3, direction: DVec3, max: f64, skip: Option<StableId>) -> Option<(StableId, f64, DVec3)> {
        let mut filter = rp::QueryFilter::default().exclude_sensors();
        if let Some(body) = skip.and_then(|id| self.bodies.get(&id)) {
            filter = filter.exclude_rigid_body(body.handle);
        }
        let ray = rp::Ray::new(vector(origin), vector(direction));
        let (collider, hit) = self.world.cast_ray_and_get_normal(&ray, real(max), true, filter)?;
        Some((*self.owners.get(&collider)?, wide(hit.time_of_impact), from_vector(hit.normal)))
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

/// The joint a Joint component asks for, between the object's body (world matrix `world1`) and
/// what it holds on to (`world2`; `None` for the world, a fixed body at the origin).
fn joint_data(settings: &Joint, world1: DMat4, world2: Option<DMat4>) -> rp::GenericJoint {
    let (_, rotation1, position1) = world1.to_scale_rotation_translation();
    let anchor_world = world1.transform_point3(DVec3::from(settings.anchor.map(f64::from)));
    let anchor1 = rotation1.inverse() * (anchor_world - position1);
    let axis1 = DVec3::from(settings.axis.map(f64::from)).normalize_or(DVec3::Y);
    let (rotation2, position2) = world2.map_or((DQuat::IDENTITY, DVec3::ZERO), |w| {
        let (_, r, t) = w.to_scale_rotation_translation();
        (r, t)
    });
    let anchor2 = rotation2.inverse() * (anchor_world - position2);
    let axis2 = rotation2.inverse() * (rotation1 * axis1);
    // Rapier measures the second body against the first: the object is the second, so a slider's
    // limits and a motor's speed are the object's own movement.
    let mut data = match settings.kind {
        JointKind::Fixed => rp::FixedJointBuilder::new().build().data,
        JointKind::Hinge => rp::RevoluteJointBuilder::new(vector(axis2)).build().data,
        JointKind::Ball => rp::SphericalJointBuilder::new().build().data,
        JointKind::Slider => rp::PrismaticJointBuilder::new(vector(axis2)).build().data,
    };
    match settings.kind {
        JointKind::Fixed => {
            // Held in the turn they have now, too.
            let turn = rotation2.inverse() * rotation1;
            let rotation = rp::Rotation::from_xyzw(real(turn.x), real(turn.y), real(turn.z), real(turn.w));
            data.set_local_frame1(rp::Pose::from_parts(vector(anchor2), rotation));
            data.set_local_frame2(rp::Pose::from_parts(vector(anchor1), rp::Rotation::IDENTITY));
        }
        JointKind::Hinge | JointKind::Slider => {
            data.set_local_axis1(vector(axis2));
            data.set_local_axis2(vector(axis1));
            data.set_local_anchor1(vector(anchor2));
            data.set_local_anchor2(vector(anchor1));
        }
        JointKind::Ball => {
            data.set_local_anchor1(vector(anchor2));
            data.set_local_anchor2(vector(anchor1));
        }
    }
    drive(&mut data, settings);
    data
}

/// How hard a motor pulls toward its speed (Rapier's velocity factor), within its force.
const MOTOR_STRENGTH: f64 = 10.0;

/// A hinge's or slider's limits and motor, from the component: degrees for a hinge, units for a slider.
fn drive(data: &mut rp::GenericJoint, settings: &Joint) {
    let (axis, unit) = match settings.kind {
        JointKind::Hinge => (rp::JointAxis::AngX, std::f64::consts::PI / 180.0),
        JointKind::Slider => (rp::JointAxis::LinX, 1.0),
        JointKind::Fixed | JointKind::Ball => return,
    };
    // Without limits or a motor the axis is left free: Rapier counts a limit or motor once it is set.
    if settings.limited {
        data.set_limits(axis, [real(f64::from(settings.lower) * unit), real(f64::from(settings.upper) * unit)]);
    } else {
        data.limit_axes.remove(axis.into());
    }
    if settings.motor_force == 0.0 {
        data.motor_axes.remove(axis.into());
    } else {
        data.set_motor_velocity(axis, real(f64::from(settings.motor_speed) * unit), real(MOTOR_STRENGTH));
        data.set_motor_max_force(axis, real(f64::from(settings.motor_force)));
    }
}

/// Whether two Joint components make the same joint, differing at most in limits and motor.
fn same_joint(a: &Joint, b: &Joint) -> bool {
    a.kind == b.kind && a.connected == b.connected && a.anchor == b.anchor && a.axis == b.axis
}

impl World {
    /// Joints for objects that have a Joint and a body, once what they hold on to has a body (or
    /// at once, for the world). Changed limits and motors apply to the joint; other changes make
    /// it again where the bodies are now.
    fn sync_joints(&mut self, scene: &Scene) {
        let mut stale: Vec<StableId> = self.joints.keys().filter(|id| !self.bodies.contains_key(id) || scene.get::<Joint>(**id).is_none()).copied().collect();
        stale.sort_unstable();
        for id in stale {
            if let Some((handle, _)) = self.joints.remove(&id) {
                self.world.impulse_joints.remove(handle, true);
            }
        }
        let mut wanted: Vec<(StableId, Joint)> = self.bodies.keys().filter_map(|id| scene.get::<Joint>(*id).map(|j| (*id, j.clone()))).collect();
        if wanted.is_empty() {
            return;
        }
        wanted.sort_unstable_by_key(|(id, _)| *id);
        let objects = scene.world_matrices();
        for (id, settings) in wanted {
            if let Some((handle, made)) = self.joints.get(&id).cloned() {
                if made == settings {
                    continue;
                }
                if same_joint(&made, &settings) {
                    if let Some(joint) = self.world.impulse_joints.get_mut(handle, true) {
                        drive(&mut joint.data, &settings);
                    }
                    self.joints.insert(id, (handle, settings));
                    continue;
                }
                self.world.impulse_joints.remove(handle, true);
                self.joints.remove(&id);
            }
            let Some(&(_, world1)) = objects.iter().find(|(o, _)| *o == id) else { continue };
            let Some(body1) = self.bodies.get(&id).map(|b| b.handle) else { continue };
            let (body2, world2) = if settings.connected.trim().is_empty() {
                (self.ground(), None)
            } else {
                // The first other object with that name, once it has a body.
                let Some(&(other, world2)) = objects.iter().find(|(o, _)| *o != id && scene.name(*o) == Some(settings.connected.trim())) else { continue };
                let Some(body2) = self.bodies.get(&other).map(|b| b.handle) else { continue };
                (body2, Some(world2))
            };
            let handle = self.world.impulse_joints.insert(body2, body1, joint_data(&settings, world1, world2), true);
            self.joints.insert(id, (handle, settings));
        }
    }

    /// A fixed body at the origin that joints to the world hold on to.
    fn ground(&mut self) -> rp::RigidBodyHandle {
        *self.ground.get_or_insert_with(|| self.world.bodies.insert(rp::RigidBodyBuilder::fixed()))
    }
}
