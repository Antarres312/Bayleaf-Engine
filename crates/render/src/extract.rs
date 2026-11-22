//! CPU side of a frame: what to draw, from where, and which object a screen point hits.

use bayleaf_core::StableId;
use bayleaf_core::math::{DMat4, DVec3, Mat3, Mat4, Vec2, Vec3, Vec4};
use bayleaf_scene::Scene;
use std::sync::Arc;

use bayleaf_scene::components::{Camera, DirectionalLight, Environment, Mesh, PointLight, Projection, Shape, SpotLight, Sprite};

use bayleaf_assets::{Image, MeshData};
use bayleaf_scene::lighting::Lighting;

use crate::material::CompiledMaterial;

/// Where a frame gets the files its scene names: materials, images and models, by the path
/// written in the scene. `None` draws the object without it.
pub trait AssetSource {
    fn material(&mut self, path: &str) -> Option<Arc<CompiledMaterial>>;
    fn image(&mut self, path: &str) -> Option<Arc<Image>>;
    fn model(&mut self, path: &str) -> Option<Arc<MeshData>>;
    /// A lighting asset (`.blight`) the scene's settings name.
    fn lighting(&mut self, _path: &str) -> Option<Arc<Lighting>> {
        None
    }
    /// A post-processing asset (`.bpost`) the scene's settings or a camera name.
    fn post(&mut self, _path: &str) -> Option<Arc<bayleaf_scene::post::PostProcessing>> {
        None
    }
}

/// Reads materials, images and models from files, relative to `base`, through caches that
/// notice when a file changes. Every path passes `allow` first (the automation folders).
/// Problems are kept in `warnings`, once each.
pub struct FileAssets<'a> {
    pub base: std::path::PathBuf,
    pub materials: &'a mut crate::MaterialLibrary,
    pub cache: &'a mut bayleaf_assets::AssetCache,
    pub allow: &'a dyn Fn(&std::path::Path) -> Result<std::path::PathBuf, String>,
    pub warnings: Vec<String>,
}

impl FileAssets<'_> {
    fn load<T>(&mut self, path: &str, load: impl FnOnce(&mut Self, &std::path::Path) -> Result<T, String>) -> Option<T> {
        let result = (self.allow)(&self.base.join(path)).and_then(|full| load(self, &full));
        match result {
            Ok(value) => Some(value),
            Err(error) => {
                let message = format!("{path}: {error}");
                if !self.warnings.contains(&message) {
                    self.warnings.push(message);
                }
                None
            }
        }
    }
}

impl AssetSource for FileAssets<'_> {
    fn material(&mut self, path: &str) -> Option<Arc<CompiledMaterial>> {
        self.load(path, |s, full| s.materials.get(full))
    }
    fn image(&mut self, path: &str) -> Option<Arc<Image>> {
        self.load(path, |s, full| s.cache.image(full))
    }
    fn model(&mut self, path: &str) -> Option<Arc<MeshData>> {
        self.load(path, |s, full| s.cache.model(full))
    }
    fn lighting(&mut self, path: &str) -> Option<Arc<Lighting>> {
        self.load(path, |s, full| s.materials.lighting(full))
    }
    fn post(&mut self, path: &str) -> Option<Arc<bayleaf_scene::post::PostProcessing>> {
        self.load(path, |s, full| s.materials.post(full))
    }
}

/// Draws every object without its files.
pub struct NoAssets;

impl AssetSource for NoAssets {
    fn material(&mut self, _: &str) -> Option<Arc<CompiledMaterial>> {
        None
    }
    fn image(&mut self, _: &str) -> Option<Arc<Image>> {
        None
    }
    fn model(&mut self, _: &str) -> Option<Arc<MeshData>> {
        None
    }
}

/// Lights beyond this count are ignored.
pub const MAX_LIGHTS: usize = 8;

/// Where the scene is seen from. A frame is drawn in coordinates measured from `origin`, a world
/// point at or near the camera, so the GPU's single precision stays exact however far from the
/// world's centre the camera is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraView {
    pub view: Mat4,
    pub proj: Mat4,
    /// The eye, measured from `origin`.
    pub position: Vec3,
    /// The world point the frame's coordinates start from.
    pub origin: DVec3,
}

impl CameraView {
    /// Perspective camera at `eye` looking at `target`, +Y up, measured from the world's centre.
    pub fn look_at(eye: Vec3, target: Vec3, fov_degrees: f32, aspect: f32) -> Self {
        let up = if (target - eye).normalize_or_zero().cross(Vec3::Y).length_squared() < 1e-6 { Vec3::Z } else { Vec3::Y };
        Self {
            view: Mat4::look_at_rh(eye, target, up),
            proj: Mat4::perspective_rh(fov_degrees.to_radians(), aspect.max(1e-3), 0.05, 5000.0),
            position: eye,
            origin: DVec3::ZERO,
        }
    }

    /// Like [`Self::look_at`] for world points in double precision: the frame is measured from the eye.
    pub fn look_at_world(eye: DVec3, target: DVec3, fov_degrees: f32, aspect: f32) -> Self {
        Self { origin: eye, ..Self::look_at(Vec3::ZERO, (target - eye).as_vec3(), fov_degrees, aspect) }
    }

    /// The view through a camera component placed by `world`, measured from the camera.
    pub fn from_camera(world: DMat4, camera: &Camera, aspect: f32) -> Self {
        let aspect = aspect.max(1e-3);
        // Scale on a camera object does not stretch the view.
        let (_, rotation, position) = world.to_scale_rotation_translation();
        let view = Mat4::from_quat(rotation.as_quat()).inverse();
        let proj = match camera.projection {
            Projection::Perspective => Mat4::perspective_rh(camera.fov.to_radians(), aspect, camera.near, camera.far),
            Projection::Orthographic => {
                let h = camera.size;
                Mat4::orthographic_rh(-h * aspect, h * aspect, -h, h, camera.near, camera.far)
            }
        };
        Self { view, proj, position: Vec3::ZERO, origin: position }
    }

    /// A perspective view that shows a sphere of `radius` around `center` from a three-quarter angle.
    pub fn framing(center: DVec3, radius: f32, aspect: f32) -> Self {
        let distance = f64::from(radius.max(0.5)) * 2.6 + 1.0;
        let eye = center + DVec3::new(0.6, 0.45, 0.65).normalize() * distance;
        Self::look_at_world(eye, center, 50.0, aspect)
    }

    /// `world` (a world matrix in double precision) measured from this view's origin.
    pub fn relative(&self, world: DMat4) -> Mat4 {
        let mut m = world;
        m.w_axis -= self.origin.extend(0.0);
        m.as_mat4()
    }

    /// A world point measured from this view's origin.
    pub fn relative_point(&self, point: DVec3) -> Vec3 {
        (point - self.origin).as_vec3()
    }

    /// The eye in world space.
    pub fn eye(&self) -> DVec3 {
        self.origin + self.position.as_dvec3()
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
#[derive(Clone, Debug)]
pub struct DrawItem {
    pub object: StableId,
    pub shape: Shape,
    pub model: Mat4,
    pub color: Vec4,
    pub metallic: f32,
    pub roughness: f32,
    /// Sprites ignore lighting.
    pub unlit: bool,
    /// A compiled material graph; `None` uses colour, metallic and roughness.
    pub material: Option<Arc<CompiledMaterial>>,
    /// A loaded model, drawn instead of `shape`.
    pub mesh: Option<Arc<MeshData>>,
    /// Images by texture slot: a sprite's image, or a material's Texture nodes in order. A
    /// missing image reads as white.
    pub textures: Vec<Option<Arc<Image>>>,
    /// The model's triangles this item draws (first index, count); None for all of them.
    pub part: Option<(u32, u32)>,
    /// Adds its light to what is behind instead of covering it (fire, sparks).
    pub additive: bool,
    /// The part of its images it shows: offset x and y, then width and height, in texture
    /// coordinates; `[0, 0, 1, 1]` for all (a flipbook's frame is less).
    pub uv_rect: [f32; 4],
}

/// The whole image, as most items show it.
pub const WHOLE_IMAGE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

impl DrawItem {
    /// The box around the item before its model matrix: the model's, or the unit shape's.
    pub fn local_bounds(&self) -> (Vec3, Vec3) {
        if let Some(mesh) = &self.mesh {
            return (Vec3::from(mesh.min), Vec3::from(mesh.max));
        }
        match self.shape {
            Shape::Plane => (Vec3::new(-0.5, -0.001, -0.5), Vec3::new(0.5, 0.001, 0.5)),
            Shape::Quad => (Vec3::new(-0.5, -0.5, -0.001), Vec3::new(0.5, 0.5, 0.001)),
            Shape::Cube | Shape::Sphere => (Vec3::splat(-0.5), Vec3::splat(0.5)),
        }
    }

    pub fn is_transparent(&self) -> bool {
        self.unlit
            || match &self.material {
                Some(material) => material.transparent,
                None => self.color.w < 1.0,
            }
    }
}

/// What kind of source a light is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightKind {
    Directional,
    Point,
    Spot,
}

#[derive(Clone, Copy, Debug)]
pub struct LightItem {
    pub kind: LightKind,
    /// World-space position (point and spot lights).
    pub position: Vec3,
    /// World-space direction the light travels in (directional and spot lights).
    pub direction: Vec3,
    /// Linear RGB times intensity.
    pub radiance: Vec3,
    /// Distance at which a point or spot light has faded out.
    pub range: f32,
    /// Cosines of the spot cone's outer edge and of where its fade starts.
    pub cos_outer: f32,
    pub cos_inner: f32,
    /// This light casts shadows (the first light that does; one at a time).
    pub shadows: bool,
}

impl LightItem {
    pub const fn directional(direction: Vec3, radiance: Vec3, shadows: bool) -> Self {
        Self { kind: LightKind::Directional, position: Vec3::ZERO, direction, radiance, range: 0.0, cos_outer: -1.0, cos_inner: -1.0, shadows }
    }
}

/// A sky: colours straight up, at the horizon and below it, in linear RGB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sky {
    pub zenith: Vec3,
    pub horizon: Vec3,
    pub ground: Vec3,
}

/// Ambient light and background for a scene without an environment.
const DEFAULT_AMBIENT: Vec3 = Vec3::new(0.22, 0.24, 0.28);
const DEFAULT_CLEAR: Vec4 = Vec4::new(0.035, 0.04, 0.05, 1.0);
/// Ambient light from an environment's colours at strength 1.
const ENVIRONMENT_AMBIENT: f32 = 0.5;

/// Everything one frame needs, independent of the GPU.
#[derive(Clone, Debug)]
pub struct Frame {
    pub camera: CameraView,
    pub items: Vec<DrawItem>,
    pub lights: Vec<LightItem>,
    /// Light from above (the sky) that every shape gets.
    pub ambient: Vec3,
    /// Light from below (the ground).
    pub ambient_ground: Vec3,
    pub clear_color: Vec4,
    /// The sky drawn behind the scene, if the scene has one.
    pub sky: Option<Sky>,
    /// Draws the sky (or the clear colour) behind the shapes; off, the background stays transparent
    /// (material previews over a panel) while the sky still lights and is reflected.
    pub background: bool,
    /// Fog colour (linear RGB) and density per world unit; density 0 is no fog.
    pub fog: (Vec3, f32),
    pub show_grid: bool,
    pub selected: Option<StableId>,
    /// How shadows are drawn.
    pub shadows: ShadowSettings,
    /// What post-processing makes of the drawn picture.
    pub post: crate::PostSettings,
    /// More selected objects, highlighted like `selected`.
    pub highlighted: Vec<StableId>,
    /// Seconds, for animated materials.
    pub time: f32,
}

/// Options for [`extract`].
#[derive(Clone, Copy, Debug)]
pub struct FrameOptions {
    pub show_grid: bool,
    pub selected: Option<StableId>,
    /// Seconds, for animated materials.
    pub time: f32,
    /// How shadows are drawn (Project Settings > Graphics).
    pub shadows: ShadowSettings,
    /// Which post-processing applies.
    pub effects: Effects,
}

/// Which post-processing a view shows: whether at all, which effects the project allows, and the
/// camera object whose own asset comes before the scene's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Effects {
    /// Off, the plain picture (the Scene view's switch).
    pub show: bool,
    pub allowed: EffectsAllowed,
    pub camera: Option<StableId>,
}

impl Effects {
    pub const DEFAULT: Effects = Effects { show: true, allowed: EffectsAllowed::ALL, camera: None };
}

/// The effects a project lets its scenes use (Project Settings > Graphics).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectsAllowed {
    pub tone_mapping: bool,
    pub bloom: bool,
    pub color: bool,
    pub vignette: bool,
    pub depth_fog: bool,
    pub occlusion: bool,
    pub antialiasing: bool,
}

impl EffectsAllowed {
    pub const ALL: EffectsAllowed =
        EffectsAllowed { tone_mapping: true, bloom: true, color: true, vignette: true, depth_fog: true, occlusion: true, antialiasing: true };
}

/// `model` as the object's Model Animation poses it at `time` seconds, or as it is when the object
/// has none playing or the model has no such animation.
fn animated(scene: &Scene, id: StableId, model: Arc<MeshData>, time: f32) -> Arc<MeshData> {
    let Some(animation) = scene.get::<bayleaf_scene::components::ModelAnimation>(id).filter(|a| a.playing) else { return model };
    let Some(skeleton) = model.skeleton.clone() else { return model };
    let Some(index) = skeleton.animation(&animation.animation) else { return model };
    let at = time * animation.speed;
    // Its own id, so each object's pose is its own mesh, and a version for each moment.
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&(model.id, id), &mut hasher);
    let own = std::hash::Hasher::finish(&hasher) | 1;
    Arc::new(skeleton.posed(&model, Some(index), at, animation.looping, own, u64::from(at.to_bits()) + 1))
}

/// What a post-processing asset asks for, as far as `allowed` lets it; its lookup image is for the
/// caller to load (see [`lookup_image`]).
pub fn post_settings(post: &bayleaf_scene::post::PostProcessing, allowed: &EffectsAllowed) -> crate::PostSettings {
    let mut s = crate::PostSettings::default();
    if post.tone_mapping.enabled && allowed.tone_mapping {
        s.tone_mapping = true;
        s.exposure = post.tone_mapping.exposure;
    }
    if post.bloom.enabled && allowed.bloom {
        s.bloom = post.bloom.strength;
        s.bloom_threshold = post.bloom.threshold;
    }
    if post.color.enabled && allowed.color {
        s.contrast = post.color.contrast;
        s.saturation = post.color.saturation;
        s.lookup_strength = post.color.lookup_strength;
    }
    if post.vignette.enabled && allowed.vignette {
        s.vignette = post.vignette.strength;
        s.vignette_smoothness = post.vignette.smoothness;
    }
    if post.depth_fog.enabled && allowed.depth_fog {
        s.fog = Some((post.depth_fog.color, post.depth_fog.start, post.depth_fog.end));
    }
    if post.occlusion.enabled && allowed.occlusion {
        s.occlusion = post.occlusion.strength;
        s.occlusion_radius = post.occlusion.radius;
    }
    s.antialiasing = post.antialiasing && allowed.antialiasing;
    s
}

/// The post-processing a frame gets: the camera's own asset, else the scene's, as the options allow.
fn frame_post(scene: &Scene, options: &FrameOptions, assets: &mut dyn AssetSource) -> crate::PostSettings {
    if !options.effects.show {
        return crate::PostSettings::default();
    }
    let camera = options.effects.camera.and_then(|id| scene.get::<Camera>(id)).map(|c| c.post_processing.clone()).filter(|p| !p.is_empty());
    let path = camera.or_else(|| Some(scene.settings().post_processing.clone()).filter(|p| !p.is_empty()));
    path.and_then(|p| assets.post(&p)).map_or_else(crate::PostSettings::default, |post| {
        let mut settings = post_settings(&post, &options.effects.allowed);
        if post.color.enabled && options.effects.allowed.color {
            settings.lookup = lookup_image(&post, assets);
        }
        settings
    })
}

/// The colour lookup image a post-processing asset names, when it loads and is one (N squares of
/// N by N side by side).
pub fn lookup_image(post: &bayleaf_scene::post::PostProcessing, assets: &mut dyn AssetSource) -> Option<Arc<Image>> {
    if post.color.lookup.is_empty() {
        return None;
    }
    assets.image(&post.color.lookup).filter(|i| bayleaf_scene::post::is_lookup_size(i.width, i.height))
}

/// How shadows are drawn: the same for every scene of a project.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowSettings {
    /// Texels per side of each shadow map: 512, 1024, 2048 or 4096.
    pub resolution: u32,
    /// How many maps a sun's shadow is split into by distance from the camera in a large scene, 1 to 4.
    pub cascades: u32,
    /// How far from the camera a sun's cascades reach, in world units; 0 for the whole scene.
    pub reach: f32,
    /// How many lights cast shadows at once, 1 to 4 (sun-like lights first, then the nearest).
    pub lights: u32,
}

impl ShadowSettings {
    pub const DEFAULT: ShadowSettings = ShadowSettings { resolution: 2048, cascades: 3, reach: 0.0, lights: 1 };

    /// The same settings within the ranges the renderer takes.
    pub fn clamped(self) -> ShadowSettings {
        ShadowSettings {
            resolution: self.resolution.clamp(256, 4096).next_power_of_two().min(4096),
            cascades: self.cascades.clamp(1, 4),
            reach: if self.reach.is_finite() { self.reach.max(0.0) } else { 0.0 },
            lights: self.lights.clamp(1, 4),
        }
    }
}

impl Default for ShadowSettings {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Light used when the scene has none, so a new scene is not black.
const FALLBACK_LIGHT: LightItem = LightItem::directional(Vec3::new(-0.4, -1.0, -0.6), Vec3::new(2.8, 2.8, 2.8), false);

/// Collects the draw items and lights of `scene` as seen by `camera`, without the files it names.
pub fn extract(scene: &Scene, camera: CameraView, options: FrameOptions) -> Frame {
    extract_with(scene, camera, options, &mut NoAssets)
}

/// Like [`extract`], with `assets` giving the materials, images and models the scene names.
pub fn extract_with(scene: &Scene, camera: CameraView, options: FrameOptions, assets: &mut dyn AssetSource) -> Frame {
    let mut items = Vec::new();
    let mut lights = Vec::new();
    // The scene's lighting asset, else (in scenes not yet moved to one) its first Environment.
    let lighting = Some(scene.settings().lighting.as_str()).filter(|p| !p.is_empty()).and_then(|path| assets.lighting(path));
    let mut environment: Option<Environment> = lighting.as_ref().map(|l| l.environment());
    let mut has_sun = false;
    for (id, world) in scene.world_matrices() {
        let world = camera.relative(world);
        if environment.is_none() {
            environment = scene.get::<Environment>(id).cloned();
        }
        has_sun |= scene.get::<DirectionalLight>(id).is_some();
        if let Some(mesh) = scene.get::<Mesh>(id) {
            let material = if mesh.material.is_empty() { None } else { assets.material(&mesh.material) };
            let model = if mesh.model.is_empty() { None } else { assets.model(&mesh.model) };
            let model = model.map(|model| animated(scene, id, model, options.time));
            let mut item = DrawItem {
                object: id,
                shape: mesh.shape,
                model: world,
                color: Vec4::from(mesh.color),
                metallic: mesh.metallic,
                roughness: mesh.roughness,
                unlit: false,
                textures: material.as_ref().map(|m| m.textures.iter().map(|path| assets.image(path)).collect()).unwrap_or_default(),
                material,
                mesh: model.clone(),
                part: None,
                additive: false,
                uv_rect: WHOLE_IMAGE,
            };
            // A model's own materials tint the mesh colour and set metallic and roughness, unless a
            // material graph is chosen; a model with several draws one item per part.
            match model.as_ref().filter(|_| item.material.is_none()) {
                Some(model) if model.parts.len() > 1 => {
                    for part in &model.parts {
                        let mut piece = item.clone();
                        piece.part = Some((part.first, part.count));
                        if let Some(own) = &part.material {
                            apply_model_material(&mut piece, own);
                        }
                        items.push(piece);
                    }
                }
                Some(model) => {
                    if let Some(own) = &model.material {
                        apply_model_material(&mut item, own);
                    }
                    items.push(item);
                }
                None => items.push(item),
            }
        }
        if let Some(sprite) = scene.get::<Sprite>(id) {
            let model = world * Mat4::from_scale(Vec3::new(sprite.size[0], sprite.size[1], 1.0));
            items.push(DrawItem {
                object: id,
                shape: Shape::Quad,
                model,
                color: Vec4::from(sprite.color),
                metallic: 0.0,
                roughness: 1.0,
                unlit: true,
                material: None,
                mesh: None,
                textures: if sprite.image.is_empty() { Vec::new() } else { vec![assets.image(&sprite.image)] },
                part: None,
                additive: false,
                uv_rect: WHOLE_IMAGE,
            });
        }
        let direction = world.transform_vector3(Vec3::NEG_Z).normalize_or(Vec3::NEG_Y);
        let position = world.w_axis.truncate();
        if let Some(light) = scene.get::<DirectionalLight>(id) {
            lights.push(LightItem::directional(direction, Vec3::from(light.color) * light.intensity, light.shadows));
        }
        if let Some(light) = scene.get::<PointLight>(id) {
            let radiance = Vec3::from(light.color) * light.intensity;
            lights.push(LightItem {
                kind: LightKind::Point,
                position,
                direction,
                radiance,
                range: light.range,
                cos_outer: -1.0,
                cos_inner: -1.0,
                shadows: light.shadows,
            });
        }
        if let Some(light) = scene.get::<SpotLight>(id) {
            let radiance = Vec3::from(light.color) * light.intensity;
            let half = (light.angle * 0.5).to_radians();
            let (cos_outer, cos_inner) = (half.cos(), (half * (1.0 - light.softness)).cos());
            lights.push(LightItem { kind: LightKind::Spot, position, direction, radiance, range: light.range, cos_outer, cos_inner, shadows: light.shadows });
        }
    }
    // The lighting asset's sun lights a scene that has no Directional Light of its own.
    if let Some(sun) = lighting.as_ref().map(|l| &l.sun).filter(|sun| sun.enabled && !has_sun) {
        let turn = bayleaf_scene::components::Transform { rotation: sun.rotation, ..Default::default() }.matrix().as_mat4();
        let direction = turn.transform_vector3(Vec3::NEG_Z).normalize_or(Vec3::NEG_Y);
        lights.push(LightItem::directional(direction, Vec3::from(sun.color) * sun.intensity, sun.shadows));
    }
    // Sun-like lights first, then the lights nearest the camera, as many as the shader takes.
    lights.sort_by(|a, b| {
        let rank = |l: &LightItem| if l.kind == LightKind::Directional { 0.0 } else { 1.0 + l.position.distance_squared(camera.position) };
        rank(a).total_cmp(&rank(b))
    });
    lights.truncate(MAX_LIGHTS);
    // A scene without lights gets a soft default light, so a new scene is not black.
    if lights.is_empty() {
        lights.push(FALLBACK_LIGHT);
    }
    let environment = environment.as_ref();
    let (ambient, ambient_ground, clear_color, sky) = match environment {
        Some(env) => environment_light(env),
        None => (DEFAULT_AMBIENT, DEFAULT_AMBIENT * 0.35, DEFAULT_CLEAR, None),
    };
    let post = frame_post(scene, &options, assets);
    // Half of what lies fogDistance away is hidden: 1 - exp(-density * distance) = 0.5.
    let fog = match environment {
        Some(env) if env.fog_distance > 0.0 => (Vec3::from(env.horizon_color), std::f32::consts::LN_2 / env.fog_distance),
        _ => (Vec3::ZERO, 0.0),
    };
    Frame {
        camera,
        items,
        lights,
        ambient,
        ambient_ground,
        clear_color,
        sky,
        background: true,
        fog,
        show_grid: options.show_grid,
        shadows: options.shadows,
        post,
        selected: options.selected,
        highlighted: Vec::new(),
        time: options.time,
    }
}

fn apply_model_material(item: &mut DrawItem, own: &bayleaf_assets::ModelMaterial) {
    item.color *= Vec4::from(own.base_color);
    item.metallic = own.metallic;
    item.roughness = own.roughness;
    item.textures = vec![own.image.clone(), own.metal_roughness.clone(), own.normal.clone()];
}

/// Ambient light from above and below, the background colour and the sky of an environment.
fn environment_light(env: &Environment) -> (Vec3, Vec3, Vec4, Option<Sky>) {
    let (zenith, horizon, ground) = (Vec3::from(env.sky_color), Vec3::from(env.horizon_color), Vec3::from(env.ground_color));
    let strength = env.ambient * ENVIRONMENT_AMBIENT;
    let sky = env.show_sky.then_some(Sky { zenith, horizon, ground });
    ((zenith + horizon) * 0.5 * strength, ground * strength, horizon.extend(1.0), sky)
}

/// A frame that shows `material` on a sphere under the default sky and a sun from the upper
/// left, with no background: only the sphere shows over the panel (the previews of the Graph and
/// Project panels and the Inspector). `textures` are its images by slot.
pub fn material_preview(material: Arc<CompiledMaterial>, textures: Vec<Option<Arc<Image>>>, aspect: f32, time: f32) -> Frame {
    let (ambient, ambient_ground, clear_color, sky) = environment_light(&Environment::default());
    let sun = LightItem::directional(Vec3::new(-0.5, -0.7, -0.6).normalize(), Vec3::splat(3.0), false);
    let sphere = DrawItem {
        object: StableId::from_u128(0),
        shape: Shape::Sphere,
        model: Mat4::IDENTITY,
        color: Vec4::ONE,
        metallic: 0.0,
        roughness: 0.5,
        unlit: false,
        material: Some(material),
        mesh: None,
        textures,
        part: None,
        additive: false,
        uv_rect: WHOLE_IMAGE,
    };
    Frame {
        camera: CameraView::look_at(Vec3::new(0.0, 0.3, 1.9), Vec3::ZERO, 36.0, aspect),
        items: vec![sphere],
        lights: vec![sun],
        ambient,
        ambient_ground,
        clear_color,
        sky,
        background: false,
        fog: (Vec3::ZERO, 0.0),
        show_grid: false,
        // Nothing casts a shadow here, so the smallest map.
        shadows: ShadowSettings { resolution: 256, cascades: 1, reach: 0.0, lights: 1 },
        post: crate::PostSettings::default(),
        selected: None,
        highlighted: Vec::new(),
        time,
    }
}

/// The first camera object in hierarchy order, if the scene has one: the one the game shows.
pub fn camera_object(scene: &Scene) -> Option<StableId> {
    scene.world_matrices().into_iter().map(|(id, _)| id).find(|id| scene.get::<Camera>(*id).is_some())
}

/// The view of the first camera object in hierarchy order, if the scene has one.
pub fn scene_camera(scene: &Scene, aspect: f32) -> Option<CameraView> {
    scene.world_matrices().into_iter().find_map(|(id, world)| scene.get::<Camera>(id).map(|camera| CameraView::from_camera(world, camera, aspect)))
}

/// What the game shows: the first camera's view, or for a scene without a camera a view of
/// everything in it.
pub fn game_camera(scene: &Scene, aspect: f32) -> CameraView {
    scene_camera(scene, aspect).unwrap_or_else(|| {
        let (center, radius) = bounds(scene).unwrap_or((DVec3::ZERO, 2.0));
        let eye = center + DVec3::new(0.6, 0.45, 1.0).normalize() * (f64::from(radius.max(0.5)) * 2.6 + 1.0);
        CameraView::look_at_world(eye, center, 60.0, aspect)
    })
}

/// The view of camera object `id`, if it has a camera component.
pub fn object_camera(scene: &Scene, id: StableId, aspect: f32) -> Option<CameraView> {
    let camera = scene.get::<Camera>(id)?;
    Some(CameraView::from_camera(scene.world_matrix(id)?, camera, aspect))
}

/// Centre and radius of a sphere around every drawn shape, or `None` for a scene with nothing to draw.
pub fn bounds(scene: &Scene) -> Option<(DVec3, f32)> {
    world_bounds(scene, None)
}

/// Bounds of one object's shapes.
pub fn object_bounds(scene: &Scene, id: StableId) -> Option<(DVec3, f32)> {
    world_bounds(scene, Some(id)).or_else(|| scene.world_matrix(id).map(|m| (m.w_axis.truncate(), 0.5)))
}

/// Bounds in world space, worked out near the shapes (measured from the first object) so far-away
/// scenes keep their precision.
fn world_bounds(scene: &Scene, only: Option<StableId>) -> Option<(DVec3, f32)> {
    let origin = scene.world_matrices().first().map_or(DVec3::ZERO, |(_, m)| m.w_axis.truncate());
    let camera = CameraView { origin, ..CameraView::look_at(Vec3::ONE, Vec3::ZERO, 60.0, 1.0) };
    let frame =
        extract(scene, camera, FrameOptions { show_grid: false, selected: None, time: 0.0, shadows: ShadowSettings::DEFAULT, effects: Effects::DEFAULT });
    let (center, radius) = item_bounds(frame.items.iter().filter(|i| only.is_none_or(|id| i.object == id)))?;
    Some((origin + center.as_dvec3(), radius))
}
pub(crate) fn item_bounds<'a>(items: impl Iterator<Item = &'a DrawItem>) -> Option<(Vec3, f32)> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut any = false;
    for item in items {
        let (lo, hi) = item.local_bounds();
        for corner in unit_box_corners() {
            // The unit box's corners moved onto the item's own box.
            let local = lo + (corner + Vec3::splat(0.5)) * (hi - lo);
            let p = item.model.transform_point3(local);
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
    nearest_hit(&frame.items, frame.camera.ray(ndc)).map(|(id, _)| id)
}

/// The nearest item `ray` (measured from the view's origin) meets, and how far along it: the
/// distance in lengths of the ray's direction.
pub fn nearest_hit(items: &[DrawItem], ray: Ray) -> Option<(StableId, f32)> {
    let mut best: Option<(f32, StableId)> = None;
    for item in items {
        let inverse = item.model.inverse();
        if !inverse.is_finite() {
            continue;
        }
        let local = Ray { origin: inverse.transform_point3(ray.origin), direction: inverse.transform_vector3(ray.direction) };
        // `t` is shared between the world and local rays because the transform is affine.
        let hit = if item.mesh.is_none() && item.shape == Shape::Sphere {
            hit_sphere(local, 0.5)
        } else {
            // Models are picked by the box around them.
            let (min, max) = item.local_bounds();
            let center = (min + max) * 0.5;
            hit_box(Ray { origin: local.origin - center, direction: local.direction }, (max - min) * 0.5)
        };
        if let Some(t) = hit
            && best.is_none_or(|(best_t, _)| t < best_t)
        {
            best = Some((t, item.object));
        }
    }
    best.map(|(t, id)| (id, t))
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
