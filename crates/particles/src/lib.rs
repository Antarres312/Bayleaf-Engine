//! L4: particles. Each object with a Particle Emitter gets particles that start on its shape (or on
//! the surface of its own mesh), move with their speed, gravity and drag, change size and colour over
//! their life (start, middle and end, or a curve of points), and go. They are
//! simulated on the CPU, a step at a time, and drawn as camera-facing images (with a flipbook's
//! frames), images stretched along their motion, or small shapes.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use bayleaf_core::StableId;
use bayleaf_core::math::{DMat4, DVec3, Mat4, Vec3, Vec4};
use bayleaf_render::assets::Image;
use bayleaf_render::{AssetSource, CameraView, DrawItem};
use bayleaf_scene::Scene;
use bayleaf_scene::components::{ColorKey, Mesh, ParticleEmitter, Shape, SizeKey};

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

/// A mesh's surface to start particles on: its triangles (in the object's own space) and how much
/// of the area lies up to each, so a point is picked evenly over the area.
#[derive(Debug)]
pub struct Surface {
    triangles: Vec<[Vec3; 3]>,
    /// Running total of the triangles' areas.
    area_to: Vec<f32>,
    /// A sphere of this radius, picked as a sphere (no triangles).
    sphere: Option<f32>,
}

impl Surface {
    fn from_triangles(triangles: Vec<[Vec3; 3]>) -> Option<Self> {
        let mut total = 0.0;
        let area_to: Vec<f32> = triangles
            .iter()
            .map(|[a, b, c]| {
                total += (*b - *a).cross(*c - *a).length() * 0.5;
                total
            })
            .collect();
        (total > 0.0).then_some(Self { triangles, area_to, sphere: None })
    }

    /// The surface of one of the built-in shapes, as unit shapes are drawn.
    fn of_shape(shape: Shape) -> Option<Self> {
        let square = |corners: [Vec3; 4]| [[corners[0], corners[1], corners[2]], [corners[0], corners[2], corners[3]]];
        let h = 0.5;
        match shape {
            Shape::Sphere => Some(Self { triangles: Vec::new(), area_to: Vec::new(), sphere: Some(0.5) }),
            Shape::Plane => Self::from_triangles(square([Vec3::new(-h, 0.0, -h), Vec3::new(-h, 0.0, h), Vec3::new(h, 0.0, h), Vec3::new(h, 0.0, -h)]).to_vec()),
            Shape::Quad => Self::from_triangles(square([Vec3::new(-h, -h, 0.0), Vec3::new(h, -h, 0.0), Vec3::new(h, h, 0.0), Vec3::new(-h, h, 0.0)]).to_vec()),
            Shape::Cube => {
                let mut triangles = Vec::new();
                // Each face: its normal axis and the two axes across it, wound outward.
                for (normal, u, v) in [(Vec3::X, Vec3::Y, Vec3::Z), (Vec3::Y, Vec3::Z, Vec3::X), (Vec3::Z, Vec3::X, Vec3::Y)] {
                    for sign in [1.0f32, -1.0] {
                        let c = normal * h * sign;
                        let (u, v) = if sign > 0.0 { (u * h, v * h) } else { (v * h, u * h) };
                        triangles.extend(square([c - u - v, c + u - v, c + u + v, c - u + v]));
                    }
                }
                Self::from_triangles(triangles)
            }
        }
    }

    fn of_model(mesh: &bayleaf_render::assets::MeshData) -> Option<Self> {
        let triangles = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter_map(|t| {
                let p = |i: u32| mesh.positions.get(i as usize).map(|p| Vec3::from(*p));
                Some([p(t[0])?, p(t[1])?, p(t[2])?])
            })
            .collect();
        Self::from_triangles(triangles)
    }

    /// A point on the surface and the way out of it there, from three numbers 0 to 1.
    fn pick(&self, a: f32, b: f32, c: f32) -> (Vec3, Vec3) {
        if let Some(radius) = self.sphere {
            let z = a * 2.0 - 1.0;
            let angle = b * std::f32::consts::TAU;
            let r = (1.0 - z * z).max(0.0).sqrt();
            let out = Vec3::new(r * angle.cos(), z, r * angle.sin());
            return (out * radius, out);
        }
        let total = self.area_to.last().copied().unwrap_or(0.0);
        let at = self.area_to.partition_point(|to| *to < a * total).min(self.triangles.len().saturating_sub(1));
        let Some([p, q, r]) = self.triangles.get(at).copied() else { return (Vec3::ZERO, Vec3::Y) };
        // Evenly over the triangle.
        let (b, c) = if b + c > 1.0 { (1.0 - b, 1.0 - c) } else { (b, c) };
        let point = p + (q - p) * b + (r - p) * c;
        (point, (q - p).cross(r - p).normalize_or(Vec3::Y))
    }
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
    fn spawn(&mut self, e: &ParticleEmitter, world: &DMat4, surface: Option<&Surface>) {
        let (offset, direction) = match e.shape.as_str() {
            "point" => (Vec3::ZERO, self.direction()),
            "surface" => match surface {
                Some(surface) => {
                    let (a, b, c) = (self.next(), self.next(), self.next());
                    surface.pick(a, b, c)
                }
                // No mesh to start on: from the object's middle, every way.
                None => (Vec3::ZERO, self.direction()),
            },
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

    fn step(&mut self, e: &ParticleEmitter, world: &DMat4, dt: f32, surface: Option<&Surface>) {
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
                self.spawn(e, world, surface);
            }
        }
        self.owed += e.rate * dt;
        while self.owed >= 1.0 {
            self.owed -= 1.0;
            if room(self) {
                self.spawn(e, world, surface);
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
#[derive(Default)]
pub struct Particles {
    emitters: BTreeMap<StableId, Emitter>,
    /// The folder models are named from, for emitters that start on a model's surface.
    base: PathBuf,
    models: bayleaf_render::assets::AssetCache,
    /// Surfaces made from models (by model id) and from built-in shapes (by shape).
    surfaces: HashMap<u64, Arc<Surface>>,
}

impl std::fmt::Debug for Particles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Particles").field("emitters", &self.emitters.len()).finish_non_exhaustive()
    }
}

impl Particles {
    pub fn new() -> Self {
        Self::default()
    }

    /// The folder the scene's models are named from (the project's), for `surface` emitters.
    pub fn set_base(&mut self, base: PathBuf) {
        self.base = base;
    }

    /// The surface of object `id`'s own Mesh for a `surface` emitter: its model's triangles, or its
    /// shape's. `None` when the object has no mesh (or the model does not read).
    fn surface(&mut self, scene: &Scene, id: StableId, e: &ParticleEmitter) -> Option<Arc<Surface>> {
        if e.shape != "surface" {
            return None;
        }
        let mesh = scene.get::<Mesh>(id)?;
        if mesh.model.is_empty() {
            let key = u64::MAX - mesh.shape as u64;
            if let std::collections::hash_map::Entry::Vacant(slot) = self.surfaces.entry(key) {
                slot.insert(Arc::new(Surface::of_shape(mesh.shape)?));
            }
            return self.surfaces.get(&key).cloned();
        }
        let model = self.models.model(&self.base.join(&mesh.model)).ok()?;
        if let std::collections::hash_map::Entry::Vacant(slot) = self.surfaces.entry(model.id) {
            slot.insert(Arc::new(Surface::of_model(&model)?));
        }
        self.surfaces.get(&model.id).cloned()
    }

    /// Moves every emitter's particles on by `dt` seconds and starts new ones. Emitters new to the
    /// scene start as their `playing` says; emitters gone from it take their particles with them.
    pub fn step(&mut self, scene: &Scene, dt: f32) {
        let dt = if dt.is_finite() { dt.clamp(0.0, 0.25) } else { 0.0 };
        let mut seen = Vec::new();
        for (id, world) in scene.world_matrices() {
            let Some(e) = scene.get::<ParticleEmitter>(id) else { continue };
            seen.push(id);
            let surface = self.surface(scene, id, e);
            self.emitters.entry(id).or_insert_with(|| Emitter::new(id, e.playing)).step(e, &world, dt, surface.as_deref());
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
        let surface = self.surface(scene, id, e);
        self.emitters.entry(id).or_insert_with(|| Emitter::new(id, true)).step(e, &world, dt, surface.as_deref());
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
        let surface = self.surface(scene, id, e);
        let emitter = self.emitters.entry(id).or_insert_with(|| Emitter::new(id, e.playing));
        for _ in 0..count.min(e.max_particles) {
            if emitter.particles.len() as u32 >= e.max_particles {
                break;
            }
            emitter.spawn(e, &world, surface.as_deref());
        }
    }

    /// Where `id`'s particles are, for tests.
    #[cfg(test)]
    fn positions(&self, id: StableId) -> Vec<Vec3> {
        self.emitters.get(&id).map(|e| e.particles.iter().map(|p| p.position.as_vec3()).collect()).unwrap_or_default()
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
                let size = size_at(e, t);
                let color = color_at(e, t);
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
/// A particle's size `t` (0 to 1) through its life: its curve's points when it has any, else start,
/// middle and end.
fn size_at(e: &ParticleEmitter, t: f32) -> f32 {
    if e.size_keys.is_empty() {
        return three(e.start_size, e.middle_size, e.end_size, t);
    }
    keyed(&e.size_keys.iter().map(|k: &SizeKey| (k.at, Vec4::splat(k.size))).collect::<Vec<_>>(), t).x
}

/// A particle's colour `t` through its life, as [`size_at`].
fn color_at(e: &ParticleEmitter, t: f32) -> Vec4 {
    if e.color_keys.is_empty() {
        let color = Vec4::from(e.start_color).lerp(Vec4::from(e.middle_color), (t * 2.0).min(1.0));
        return if t > 0.5 { Vec4::from(e.middle_color).lerp(Vec4::from(e.end_color), t * 2.0 - 1.0) } else { color };
    }
    keyed(&e.color_keys.iter().map(|k: &ColorKey| (k.at, Vec4::from(k.color))).collect::<Vec<_>>(), t)
}

/// The value at `t` of points (`at`, value), straight between them, held before the first and after
/// the last; the points need not be in order.
fn keyed(points: &[(f32, Vec4)], t: f32) -> Vec4 {
    let mut sorted = points.to_vec();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let Some(first) = sorted.first() else { return Vec4::ONE };
    if t <= first.0 {
        return first.1;
    }
    for pair in sorted.windows(2) {
        let [(a, va), (b, vb)] = [pair[0], pair[1]];
        if t <= b {
            let span = (b - a).max(f32::EPSILON);
            return va.lerp(vb, (t - a) / span);
        }
    }
    sorted.last().map_or(Vec4::ONE, |last| last.1)
}

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
