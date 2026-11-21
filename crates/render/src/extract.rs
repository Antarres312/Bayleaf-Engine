//! CPU side of a frame: what to draw, from where, and which object a screen point hits.

use bayleaf_core::StableId;
use bayleaf_core::math::{Mat3, Mat4, Vec2, Vec3, Vec4};
use bayleaf_scene::Scene;
use bayleaf_scene::components::{Camera, DirectionalLight, Mesh, Projection, Shape, Sprite};

/// Lights beyond this count are ignored.
pub const MAX_LIGHTS: usize = 4;

/// Where the scene is seen from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraView {
    pub view: Mat4,
    pub proj: Mat4,
    pub position: Vec3,
}

impl CameraView {
    /// Perspective camera at `eye` looking at `target`, +Y up.
    pub fn look_at(eye: Vec3, target: Vec3, fov_degrees: f32, aspect: f32) -> Self {
        let up = if (target - eye).normalize_or_zero().cross(Vec3::Y).length_squared() < 1e-6 { Vec3::Z } else { Vec3::Y };
        Self { view: Mat4::look_at_rh(eye, target, up), proj: Mat4::perspective_rh(fov_degrees.to_radians(), aspect.max(1e-3), 0.05, 5000.0), position: eye }
    }

    /// The view through a camera component placed by `world`.
    pub fn from_camera(world: Mat4, camera: &Camera, aspect: f32) -> Self {
        let aspect = aspect.max(1e-3);
        // Scale on a camera object does not stretch the view.
        let (_, rotation, position) = world.to_scale_rotation_translation();
        let view = Mat4::from_rotation_translation(rotation, position).inverse();
        let proj = match camera.projection {
            Projection::Perspective => Mat4::perspective_rh(camera.fov.to_radians(), aspect, camera.near, camera.far),
            Projection::Orthographic => {
                let h = camera.size;
                Mat4::orthographic_rh(-h * aspect, h * aspect, -h, h, camera.near, camera.far)
            }
        };
        Self { view, proj, position }
    }

    /// A perspective view that shows a sphere of `radius` around `center` from a three-quarter angle.
    pub fn framing(center: Vec3, radius: f32, aspect: f32) -> Self {
        let distance = radius.max(0.5) * 2.6 + 1.0;
        let eye = center + Vec3::new(0.6, 0.45, 0.65).normalize() * distance;
        Self::look_at(eye, center, 50.0, aspect)
    }

    pub fn view_proj(&self) -> Mat4 {
        self.proj * self.view
    }

    /// World-space ray through a point in normalized device coordinates (−1 to 1, +Y up).
    pub fn ray(&self, ndc: Vec2) -> Ray {
        let inverse = self.view_proj().inverse();
        let near = inverse.project_point3(Vec3::new(ndc.x, ndc.y, 0.0));
        let far = inverse.project_point3(Vec3::new(ndc.x, ndc.y, 1.0));
        Ray { origin: near, direction: far - near }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3,
}

/// One drawn shape.
#[derive(Clone, Copy, Debug)]
pub struct DrawItem {
    pub object: StableId,
    pub shape: Shape,
    pub model: Mat4,
    pub color: Vec4,
    pub metallic: f32,
    pub roughness: f32,
    /// Sprites ignore lighting.
    pub unlit: bool,
}

impl DrawItem {
    pub fn is_transparent(&self) -> bool {
        self.unlit || self.color.w < 1.0
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LightItem {
    /// World-space direction the light travels in.
    pub direction: Vec3,
    /// Linear RGB times intensity.
    pub radiance: Vec3,
}

/// Everything one frame needs, independent of the GPU.
#[derive(Clone, Debug)]
pub struct Frame {
    pub camera: CameraView,
    pub items: Vec<DrawItem>,
    pub lights: Vec<LightItem>,
    pub ambient: Vec3,
    pub clear_color: Vec4,
    pub show_grid: bool,
    pub selected: Option<StableId>,
}

/// Options for [`extract`].
#[derive(Clone, Copy, Debug)]
pub struct FrameOptions {
    pub show_grid: bool,
    pub selected: Option<StableId>,
}

/// Light used when the scene has none, so a new scene is not black.
const FALLBACK_LIGHT: LightItem = LightItem { direction: Vec3::new(-0.4, -1.0, -0.6), radiance: Vec3::new(2.8, 2.8, 2.8) };

/// Collects the draw items and lights of `scene` as seen by `camera`.
pub fn extract(scene: &Scene, camera: CameraView, options: FrameOptions) -> Frame {
    let mut items = Vec::new();
    let mut lights = Vec::new();
    for (id, world) in scene.world_matrices() {
        if let Some(mesh) = scene.get::<Mesh>(id) {
            items.push(DrawItem {
                object: id,
                shape: mesh.shape,
                model: world,
                color: Vec4::from(mesh.color),
                metallic: mesh.metallic,
                roughness: mesh.roughness,
                unlit: false,
            });
        }
        if let Some(sprite) = scene.get::<Sprite>(id) {
            let model = world * Mat4::from_scale(Vec3::new(sprite.size[0], sprite.size[1], 1.0));
            items.push(DrawItem { object: id, shape: Shape::Quad, model, color: Vec4::from(sprite.color), metallic: 0.0, roughness: 1.0, unlit: true });
        }
        if let Some(light) = scene.get::<DirectionalLight>(id)
            && lights.len() < MAX_LIGHTS
        {
            let direction = world.transform_vector3(Vec3::NEG_Z).normalize_or(Vec3::NEG_Y);
            lights.push(LightItem { direction, radiance: Vec3::from(light.color) * light.intensity });
        }
    }
    if lights.is_empty() {
        lights.push(FALLBACK_LIGHT);
    }
    Frame {
        camera,
        items,
        lights,
        ambient: Vec3::new(0.22, 0.24, 0.28),
        clear_color: Vec4::new(0.035, 0.04, 0.05, 1.0),
        show_grid: options.show_grid,
        selected: options.selected,
    }
}

/// The view of the first camera object in hierarchy order, if the scene has one.
pub fn scene_camera(scene: &Scene, aspect: f32) -> Option<CameraView> {
    scene.world_matrices().into_iter().find_map(|(id, world)| scene.get::<Camera>(id).map(|camera| CameraView::from_camera(world, camera, aspect)))
}

/// The view of camera object `id`, if it has a camera component.
pub fn object_camera(scene: &Scene, id: StableId, aspect: f32) -> Option<CameraView> {
    let camera = scene.get::<Camera>(id)?;
    Some(CameraView::from_camera(scene.world_matrix(id)?, camera, aspect))
}

/// Centre and radius of a sphere around every drawn shape, or `None` for a scene with nothing to draw.
pub fn bounds(scene: &Scene) -> Option<(Vec3, f32)> {
    let frame = extract(scene, CameraView::look_at(Vec3::ONE, Vec3::ZERO, 60.0, 1.0), FrameOptions { show_grid: false, selected: None });
    item_bounds(frame.items.iter())
}

/// Bounds of one object's shapes.
pub fn object_bounds(scene: &Scene, id: StableId) -> Option<(Vec3, f32)> {
    let frame = extract(scene, CameraView::look_at(Vec3::ONE, Vec3::ZERO, 60.0, 1.0), FrameOptions { show_grid: false, selected: None });
    item_bounds(frame.items.iter().filter(|i| i.object == id)).or_else(|| scene.world_matrix(id).map(|m| (m.w_axis.truncate(), 0.5)))
}

fn item_bounds<'a>(items: impl Iterator<Item = &'a DrawItem>) -> Option<(Vec3, f32)> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut any = false;
    for item in items {
        for corner in unit_box_corners() {
            let p = item.model.transform_point3(corner);
            min = min.min(p);
            max = max.max(p);
            any = true;
        }
    }
    any.then(|| ((min + max) * 0.5, ((max - min) * 0.5).length()))
}

fn unit_box_corners() -> impl Iterator<Item = Vec3> {
    (0..8).map(|i| Vec3::new(if i & 1 == 0 { -0.5 } else { 0.5 }, if i & 2 == 0 { -0.5 } else { 0.5 }, if i & 4 == 0 { -0.5 } else { 0.5 }))
}

/// The nearest drawn object under a point in normalized device coordinates.
pub fn pick(frame: &Frame, ndc: Vec2) -> Option<StableId> {
    let ray = frame.camera.ray(ndc);
    let mut best: Option<(f32, StableId)> = None;
    for item in &frame.items {
        let inverse = item.model.inverse();
        if !inverse.is_finite() {
            continue;
        }
        let local = Ray { origin: inverse.transform_point3(ray.origin), direction: inverse.transform_vector3(ray.direction) };
        // `t` is shared between the world and local rays because the transform is affine.
        let hit = match item.shape {
            Shape::Sphere => hit_sphere(local, 0.5),
            Shape::Cube => hit_box(local, Vec3::splat(0.5)),
            Shape::Plane => hit_box(local, Vec3::new(0.5, 0.001, 0.5)),
            Shape::Quad => hit_box(local, Vec3::new(0.5, 0.5, 0.001)),
        };
        if let Some(t) = hit
            && best.is_none_or(|(best_t, _)| t < best_t)
        {
            best = Some((t, item.object));
        }
    }
    best.map(|(_, id)| id)
}

fn hit_sphere(ray: Ray, radius: f32) -> Option<f32> {
    let a = ray.direction.length_squared();
    let b = ray.origin.dot(ray.direction);
    let c = ray.origin.length_squared() - radius * radius;
    let disc = b * b - a * c;
    if a <= 0.0 || disc < 0.0 {
        return None;
    }
    let sq = disc.sqrt();
    [(-b - sq) / a, (-b + sq) / a].into_iter().find(|t| *t >= 0.0)
}

fn hit_box(ray: Ray, half: Vec3) -> Option<f32> {
    let mut t_min = 0.0_f32;
    let mut t_max = f32::INFINITY;
    for axis in 0..3 {
        let (o, d, h) = (ray.origin[axis], ray.direction[axis], half[axis]);
        if d.abs() < 1e-12 {
            if o.abs() > h {
                return None;
            }
            continue;
        }
        let (t0, t1) = ((-h - o) / d, (h - o) / d);
        t_min = t_min.max(t0.min(t1));
        t_max = t_max.min(t0.max(t1));
        if t_min > t_max {
            return None;
        }
    }
    Some(t_min)
}

/// Inverse transpose of the upper 3×3 of `model`, for transforming normals.
pub(crate) fn normal_matrix(model: Mat4) -> Mat3 {
    let m = Mat3::from_mat4(model);
    if m.determinant().abs() < 1e-12 { m } else { m.inverse().transpose() }
}
