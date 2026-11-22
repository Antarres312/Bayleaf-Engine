//! L4: particles. Each object with a Particle Emitter gets particles that start on its shape, move
//! with their speed, gravity and drag, change size and colour over their life, and go. They are
//! simulated on the CPU, a step at a time, and drawn as camera-facing images (with a flipbook's
//! frames), images stretched along their motion, or small shapes.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::BTreeMap;
use std::sync::Arc;

use bayleaf_core::StableId;
use bayleaf_core::math::{DMat4, DVec3, Mat4, Vec3, Vec4};
use bayleaf_render::assets::Image;
use bayleaf_render::{AssetSource, CameraView, DrawItem};
use bayleaf_scene::Scene;
use bayleaf_scene::components::{ParticleEmitter, Shape};

/// Falling things' acceleration, units a second squared.
const GRAVITY: f32 = 9.81;

#[derive(Clone, Debug)]
struct Particle {
    /// In the world, or (local space) in the emitter object's own space.
    position: DVec3,
    velocity: Vec3,
    age: f32,
    life: f32,
    /// Degrees, and degrees a second.
    rotation: f32,
    spin: f32,
}

#[derive(Clone, Debug)]
struct Emitter {
    particles: Vec<Particle>,
    /// Seconds into the cycle.
    clock: f32,
    /// Particles owed by the rate but not yet started.
    owed: f32,
    playing: bool,
    /// The cycle's burst has started.
    burst_done: bool,
    random: u64,
}

impl Emitter {
    fn new(id: StableId, playing: bool) -> Self {
        // Seeded from the object, so the same scene plays the same way.
        let seed = id.as_u128() as u64 ^ (id.as_u128() >> 64) as u64;
        Self { particles: Vec::new(), clock: 0.0, owed: 0.0, playing, burst_done: false, random: seed | 1 }
    }

    /// The next number from 0 to 1 (xorshift).
    fn next(&mut self) -> f32 {
        let mut x = self.random;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.random = x;
        (x >> 40) as f32 / (1u64 << 24) as f32
    }

    fn between(&mut self, [low, high]: [f32; 2]) -> f32 {
        low + (high - low) * self.next()
    }

    /// A direction evenly spread over the sphere.
    fn direction(&mut self) -> Vec3 {
        let z = self.next() * 2.0 - 1.0;
        let angle = self.next() * std::f32::consts::TAU;
        let r = (1.0 - z * z).max(0.0).sqrt();
        Vec3::new(r * angle.cos(), z, r * angle.sin())
    }

    /// Starts one particle on the emitter's shape.
    fn spawn(&mut self, e: &ParticleEmitter, world: &DMat4) {
        let (offset, direction) = match e.shape.as_str() {
            "point" => (Vec3::ZERO, self.direction()),
            "sphere" => {
                let d = self.direction();
                (d * e.radius * self.next().cbrt(), d)
            }
            "box" => {
                let [w, h, d] = e.box_size;
                (Vec3::new((self.next() - 0.5) * w, (self.next() - 0.5) * h, (self.next() - 0.5) * d), Vec3::Y)
            }
            // A cone: from a disc of the radius, outwards up to the angle around +Y.
            _ => {
                let around = self.next() * std::f32::consts::TAU;
                let out = self.next().sqrt();
                let tilt = e.angle.to_radians() * out;
                let start = Vec3::new(around.cos(), 0.0, around.sin()) * e.radius * out;
                (start, Vec3::new(tilt.sin() * around.cos(), tilt.cos(), tilt.sin() * around.sin()))
            }
        };
        let speed = self.between(e.speed);
        let (position, velocity) = if e.space == "local" {
            (offset.as_dvec3(), direction * speed)
        } else {
            (world.transform_point3(offset.as_dvec3()), world.transform_vector3(direction.as_dvec3()).as_vec3().normalize_or_zero() * speed)
        };
        let life = self.between(e.lifetime).max(0.01);
        let rotation = self.next() * 360.0;
        let spin = self.between(e.spin);
        self.particles.push(Particle { position, velocity, age: 0.0, life, rotation, spin });
    }

    fn step(&mut self, e: &ParticleEmitter, world: &DMat4, dt: f32) {
        // The particles alive move on and age; the old ones go.
        let pull = Vec3::new(0.0, -GRAVITY * e.gravity, 0.0);
        let slow = (1.0 - e.drag * dt).max(0.0);
        for p in &mut self.particles {
            p.age += dt;
            p.velocity = (p.velocity + pull * dt) * slow;
            p.position += (p.velocity * dt).as_dvec3();
            p.rotation += p.spin * dt;
        }
        self.particles.retain(|p| p.age < p.life);
        if !self.playing {
            return;
        }
        let room = |s: &Self| (s.particles.len() as u32) < e.max_particles;
        if !self.burst_done {
            self.burst_done = true;
            for _ in 0..e.burst {
                if !room(self) {
                    break;
                }
                self.spawn(e, world);
            }
        }
        self.owed += e.rate * dt;
        while self.owed >= 1.0 {
            self.owed -= 1.0;
            if room(self) {
                self.spawn(e, world);
            }
        }
        self.clock += dt;
        if self.clock >= e.duration {
            if e.looping {
                self.clock -= e.duration;
                self.burst_done = false;
            } else {
                self.playing = false;
            }
        }
    }
}

/// The particles of a scene's emitters, by emitter object.
#[derive(Clone, Debug, Default)]
pub struct Particles {
    emitters: BTreeMap<StableId, Emitter>,
}

impl Particles {
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves every emitter's particles on by `dt` seconds and starts new ones. Emitters new to the
    /// scene start as their `playing` says; emitters gone from it take their particles with them.
    pub fn step(&mut self, scene: &Scene, dt: f32) {
        let dt = if dt.is_finite() { dt.clamp(0.0, 0.25) } else { 0.0 };
        let mut seen = Vec::new();
        for (id, world) in scene.world_matrices() {
            let Some(e) = scene.get::<ParticleEmitter>(id) else { continue };
            seen.push(id);
            self.emitters.entry(id).or_insert_with(|| Emitter::new(id, e.playing)).step(e, &world, dt);
        }
        self.emitters.retain(|id, _| seen.contains(id));
    }

    /// Steps only the emitter of object `id` (the Scene view's preview of the selected emitter).
    pub fn step_one(&mut self, scene: &Scene, id: StableId, dt: f32) {
        let (Some(e), Some(world)) = (scene.get::<ParticleEmitter>(id), scene.world_matrix(id)) else {
            self.emitters.remove(&id);
            return;
        };
        let dt = if dt.is_finite() { dt.clamp(0.0, 0.25) } else { 0.0 };
        self.emitters.entry(id).or_insert_with(|| Emitter::new(id, true)).step(e, &world, dt);
        self.emitters.retain(|other, _| *other == id);
    }

    /// Starts (or goes on) emitting from object `id`'s emitter: a new cycle with its burst.
    pub fn play(&mut self, id: StableId) {
        let emitter = self.emitters.entry(id).or_insert_with(|| Emitter::new(id, true));
        if !emitter.playing {
            emitter.playing = true;
            emitter.clock = 0.0;
            emitter.burst_done = false;
        }
    }

    /// Stops starting particles at `id`; those alive live out their lives.
    pub fn stop(&mut self, id: StableId) {
        if let Some(emitter) = self.emitters.get_mut(&id) {
            emitter.playing = false;
        }
    }

    /// Takes every particle of `id` away and starts it again from the beginning.
    pub fn restart(&mut self, id: StableId) {
        self.emitters.insert(id, Emitter::new(id, true));
    }

    /// Starts `count` particles at once from `id`'s emitter (as many as it has room for).
    pub fn burst(&mut self, scene: &Scene, id: StableId, count: u32) {
        let (Some(e), Some(world)) = (scene.get::<ParticleEmitter>(id), scene.world_matrix(id)) else { return };
        let emitter = self.emitters.entry(id).or_insert_with(|| Emitter::new(id, e.playing));
        for _ in 0..count.min(e.max_particles) {
            if emitter.particles.len() as u32 >= e.max_particles {
                break;
            }
            emitter.spawn(e, &world);
        }
    }

    /// Whether `id`'s emitter is starting particles.
    pub fn is_playing(&self, id: StableId) -> bool {
        self.emitters.get(&id).is_some_and(|e| e.playing)
    }

    /// Particles alive in every emitter, or in `id`'s.
    pub fn count(&self, id: Option<StableId>) -> usize {
        match id {
            Some(id) => self.emitters.get(&id).map_or(0, |e| e.particles.len()),
            None => self.emitters.values().map(|e| e.particles.len()).sum(),
        }
    }

    /// The particles as draw items for `camera` (measured from its eye, as frames are). Images and
    /// materials come from `assets`.
    pub fn draw_items(&self, scene: &Scene, camera: &CameraView, assets: &mut dyn AssetSource) -> Vec<DrawItem> {
        let mut items = Vec::new();
        let view = camera.view;
        // The camera's own axes in the world: particles facing it lie in its right-up plane.
        let right = Vec3::new(view.x_axis.x, view.y_axis.x, view.z_axis.x);
        let up = Vec3::new(view.x_axis.y, view.y_axis.y, view.z_axis.y);
        let back = Vec3::new(view.x_axis.z, view.y_axis.z, view.z_axis.z);
        for (id, emitter) in &self.emitters {
            let (Some(e), Some(world)) = (scene.get::<ParticleEmitter>(*id), scene.world_matrix(*id)) else { continue };
            let image = if e.image.is_empty() { Some(soft_dot()) } else { assets.image(&e.image) };
            let material = Some(e.material.as_str()).filter(|m| !m.is_empty()).and_then(|m| assets.material(m));
            let frames = (e.flipbook_columns.max(1), e.flipbook_rows.max(1));
            for p in &emitter.particles {
                let t = (p.age / p.life).clamp(0.0, 1.0);
                let size = three(e.start_size, e.middle_size, e.end_size, t);
                let color = Vec4::from(e.start_color).lerp(Vec4::from(e.middle_color), (t * 2.0).min(1.0));
                let color = if t > 0.5 { Vec4::from(e.middle_color).lerp(Vec4::from(e.end_color), t * 2.0 - 1.0) } else { color };
                let (position, velocity) = if e.space == "local" {
                    (world.transform_point3(p.position), world.transform_vector3(p.velocity.as_dvec3()).as_vec3())
                } else {
                    (p.position, p.velocity)
                };
                let at = (position - camera.origin).as_vec3();
                let (sin, cos) = p.rotation.to_radians().sin_cos();
                let (model, shape, unlit) = match e.render.as_str() {
                    "mesh" => {
                        let spin = Mat4::from_rotation_y(p.rotation.to_radians());
                        (Mat4::from_translation(at) * spin * Mat4::from_scale(Vec3::splat(size)), e.mesh_shape, false)
                    }
                    "stretched" if velocity.length_squared() > 1e-8 => {
                        // Long along the motion (more the faster), facing the camera across it.
                        let along = velocity.normalize();
                        let across = along.cross(back).normalize_or(right);
                        let length = size + velocity.length() * size * 0.5;
                        (Mat4::from_cols((across * size).extend(0.0), (along * length).extend(0.0), back.extend(0.0), at.extend(1.0)), Shape::Quad, true)
                    }
                    _ => {
                        let x = (right * cos + up * sin) * size;
                        let y = (up * cos - right * sin) * size;
                        (Mat4::from_cols(x.extend(0.0), y.extend(0.0), back.extend(0.0), at.extend(1.0)), Shape::Quad, true)
                    }
                };
                // A flipbook's frame for this point of the particle's life, left to right, top down.
                let count = frames.0 * frames.1;
                let frame = ((t * count as f32) as u32).min(count - 1);
                let (w, h) = (1.0 / frames.0 as f32, 1.0 / frames.1 as f32);
                let uv_rect = [(frame % frames.0) as f32 * w, (frame / frames.0) as f32 * h, w, h];
                items.push(DrawItem {
                    // No object: particles are not picked or outlined.
                    object: StableId::from_u128(0),
                    shape,
                    model,
                    color,
                    metallic: 0.0,
                    roughness: 0.8,
                    unlit,
                    material: material.clone(),
                    mesh: None,
                    textures: if unlit || material.is_none() { vec![image.clone()] } else { Vec::new() },
                    part: None,
                    additive: e.blend == "additive",
                    uv_rect,
                });
            }
        }
        items
    }
}

/// A value from three keys: at the start, the middle and the end of a life.
fn three(start: f32, middle: f32, end: f32, t: f32) -> f32 {
    if t < 0.5 { start + (middle - start) * t * 2.0 } else { middle + (end - middle) * (t * 2.0 - 1.0) }
}

/// The image of a particle without one: a round dot, soft at its edge.
fn soft_dot() -> Arc<Image> {
    static DOT: std::sync::OnceLock<Arc<Image>> = std::sync::OnceLock::new();
    Arc::clone(DOT.get_or_init(|| {
        const SIZE: u32 = 32;
        let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let dx = (x as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0;
                let dy = (y as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0;
                let fade = (1.0 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
                rgba.extend_from_slice(&[255, 255, 255, (fade * fade * 255.0) as u8]);
            }
        }
        // An id of its own, far from the ids images read from files get.
        Arc::new(Image { id: u64::MAX - 0x5d07, width: SIZE, height: SIZE, rgba, mipmaps: true })
    }))
}

#[cfg(test)]
mod tests;
