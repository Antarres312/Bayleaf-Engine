//! Built-in scene components.
//!
//! Axis convention: right-handed, +Y up. Cameras and lights look along their local −Z axis.

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use bayleaf_core::math::{DMat4, DQuat, DVec3, EulerRot};
use bayleaf_ecs::Component;

/// A component type the scene can store, show in the inspector and write to scene files.
pub trait SceneComponent: Component + Serialize + DeserializeOwned + Default {
    /// Key in scene files and command params.
    const NAME: &'static str;
    /// English label, used as the translation key.
    const LABEL: &'static str;
    /// False for components every object always has.
    const REMOVABLE: bool;
    /// False for components the inspector shows in its own way and the generic Add Component
    /// list leaves out.
    const LISTED: bool = true;
    /// Where Add Component lists it: folders, then its name (`Light/Point Light`); empty for
    /// the top level.
    const MENU: &'static str = "";

    /// Checks a value that came from a file or a command.
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }

    /// Allowed values of a text field, for fields that take one of a fixed set.
    fn options(_field: &str) -> &'static [&'static str] {
        &[]
    }
}

/// Display name of an object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Name(pub String);

impl Default for Name {
    fn default() -> Self {
        Self("Object".to_owned())
    }
}

impl SceneComponent for Name {
    const NAME: &'static str = "name";
    const LABEL: &'static str = "Name";
    const REMOVABLE: bool = false;
}

/// Position, rotation (Euler angles in degrees, applied X then Y then Z) and scale relative to the parent.
/// The position is double precision, so objects far from the origin keep millimetre accuracy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    pub position: [f64; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}

impl Default for Transform {
    fn default() -> Self {
        Self { position: [0.0; 3], rotation: [0.0; 3], scale: [1.0; 3] }
    }
}

impl Transform {
    /// Local-to-parent matrix, in double precision.
    pub fn matrix(&self) -> DMat4 {
        let [rx, ry, rz] = self.rotation.map(|d| f64::from(d).to_radians());
        DMat4::from_scale_rotation_translation(DVec3::from(self.scale.map(f64::from)), DQuat::from_euler(EulerRot::XYZ, rx, ry, rz), DVec3::from(self.position))
    }

    /// True when every value is a finite number.
    pub fn is_finite(&self) -> bool {
        self.position.iter().all(|v| v.is_finite()) && self.rotation.iter().chain(&self.scale).all(|v| v.is_finite())
    }

    /// The transform a local-to-parent matrix stands for. Values come out as the matrix holds them,
    /// with -0 made 0.
    pub fn from_matrix(matrix: DMat4) -> Self {
        let (scale, rotation, position) = matrix.to_scale_rotation_translation();
        let (rx, ry, rz) = rotation.to_euler(EulerRot::XYZ);
        let zero = |v: f64| if v == 0.0 { 0.0 } else { v };
        Self {
            position: position.to_array().map(zero),
            rotation: [rx, ry, rz].map(|a| zero(a.to_degrees()) as f32),
            scale: scale.to_array().map(|s| zero(s) as f32),
        }
    }
}

impl SceneComponent for Transform {
    const NAME: &'static str = "transform";
    const LABEL: &'static str = "Transform";
    const REMOVABLE: bool = false;

    fn validate(&self) -> Result<(), String> {
        if !self.position.iter().all(|v| v.is_finite()) {
            return Err("transform values must be finite numbers".to_owned());
        }
        finite("transform", self.rotation.iter().chain(&self.scale))
    }
}

/// Free-form labels for finding and grouping objects.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Tags(pub Vec<String>);

impl SceneComponent for Tags {
    const NAME: &'static str = "tags";
    const LABEL: &'static str = "Tags";
    const REMOVABLE: bool = true;
}

/// Built-in mesh shapes, all one unit across and centred on the origin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    #[default]
    Cube,
    Sphere,
    /// Flat square in the XZ plane, facing +Y.
    Plane,
    /// Flat square in the XY plane, facing +Z.
    Quad,
}

impl Shape {
    pub const ALL: [Shape; 4] = [Shape::Cube, Shape::Sphere, Shape::Plane, Shape::Quad];
    const NAMES: &'static [&'static str] = &["cube", "sphere", "plane", "quad"];
}

/// A lit shape with a simple physically based material.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mesh {
    pub shape: Shape,
    /// Linear RGBA, each 0 to 1. Alpha below 1 draws the mesh as transparent.
    pub color: [f32; 4],
    /// 0 for dielectrics such as plastic or wood, 1 for bare metal.
    pub metallic: f32,
    /// 0 for a mirror finish, 1 for fully rough.
    pub roughness: f32,
    /// Material graph file (`.bmaterial`), relative to the project folder (or the scene's folder outside a project). Empty: colour, metallic
    /// and roughness above. With a material, its Object Color node reads `color`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub material: String,
    /// A glTF 2.0 model (`.glb`, or `.gltf` with its buffers beside it), relative to the project
    /// folder; drawn instead of `shape`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
}

impl Default for Mesh {
    fn default() -> Self {
        Self { shape: Shape::Cube, color: [0.8, 0.8, 0.8, 1.0], metallic: 0.0, roughness: 0.5, material: String::new(), model: String::new() }
    }
}

impl SceneComponent for Mesh {
    const NAME: &'static str = "mesh";
    const LABEL: &'static str = "Mesh";
    const MENU: &'static str = "Rendering/Mesh";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        unit_range("color", &self.color)?;
        unit_range("metallic", &[self.metallic])?;
        unit_range("roughness", &[self.roughness])?;
        file_field("material", &self.material, &["bmaterial"])?;
        file_field("model", &self.model, &["glb", "gltf"])
    }

    fn options(field: &str) -> &'static [&'static str] {
        if field == "shape" { Shape::NAMES } else { &[] }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Projection {
    #[default]
    Perspective,
    Orthographic,
}

/// A viewpoint the scene can be rendered from. It looks along its local −Z axis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Camera {
    pub projection: Projection,
    /// Vertical field of view in degrees, for perspective cameras.
    pub fov: f32,
    /// Half the visible height in scene units, for orthographic cameras.
    pub size: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self { projection: Projection::Perspective, fov: 60.0, size: 5.0, near: 0.1, far: 1000.0 }
    }
}

impl SceneComponent for Camera {
    const NAME: &'static str = "camera";
    const LABEL: &'static str = "Camera";
    const MENU: &'static str = "Rendering/Camera";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        finite("camera", [self.fov, self.size, self.near, self.far].iter())?;
        if !(1.0..=179.0).contains(&self.fov) {
            return Err("fov must be between 1 and 179 degrees".to_owned());
        }
        if self.size <= 0.0 {
            return Err("size must be greater than 0".to_owned());
        }
        if self.near <= 0.0 || self.far <= self.near {
            return Err("near must be greater than 0 and far greater than near".to_owned());
        }
        Ok(())
    }

    fn options(field: &str) -> &'static [&'static str] {
        if field == "projection" { &["perspective", "orthographic"] } else { &[] }
    }
}

/// Light from a distant source, such as the sun. It shines along the object's local −Z axis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectionalLight {
    /// Linear RGB, each 0 to 1.
    pub color: [f32; 3],
    pub intensity: f32,
    /// Objects block this light and cast shadows. Only the first directional light with shadows on casts them.
    #[serde(default = "yes")]
    pub shadows: bool,
}

fn yes() -> bool {
    true
}

impl Default for DirectionalLight {
    fn default() -> Self {
        Self { color: [1.0, 1.0, 1.0], intensity: 3.0, shadows: true }
    }
}

impl SceneComponent for DirectionalLight {
    const NAME: &'static str = "directionalLight";
    const LABEL: &'static str = "Directional Light";
    const MENU: &'static str = "Light/Directional Light";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        unit_range("color", &self.color)?;
        if self.intensity.is_finite() && self.intensity >= 0.0 { Ok(()) } else { Err("intensity must be 0 or more".to_owned()) }
    }
}

/// The world around the scene: a sky that fades from the horizon up to the top and down to the
/// ground, and the soft light that sky and ground give to every shape. The first object with one,
/// in hierarchy order, sets it for the scene.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Environment {
    /// Linear RGB straight up, each 0 to 1.
    pub sky_color: [f32; 3],
    /// Linear RGB at the horizon.
    pub horizon_color: [f32; 3],
    /// Linear RGB below the horizon.
    pub ground_color: [f32; 3],
    /// How strongly the sky and ground light shapes, 0 to 4.
    pub ambient: f32,
    /// Draw the sky behind the scene; off, the background is the horizon colour and only the light remains.
    pub show_sky: bool,
    /// Distance in world units at which fog in the horizon colour hides half of what is behind it;
    /// 0 for no fog.
    #[serde(default)]
    pub fog_distance: f32,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            sky_color: [0.18, 0.36, 0.72],
            horizon_color: [0.62, 0.72, 0.82],
            ground_color: [0.22, 0.2, 0.18],
            ambient: 1.0,
            show_sky: true,
            fog_distance: 0.0,
        }
    }
}

impl SceneComponent for Environment {
    const NAME: &'static str = "environment";
    const LABEL: &'static str = "Environment";
    const MENU: &'static str = "Rendering/Environment";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        unit_range("skyColor", &self.sky_color)?;
        unit_range("horizonColor", &self.horizon_color)?;
        unit_range("groundColor", &self.ground_color)?;
        if !(0.0..=4.0).contains(&self.ambient) {
            return Err("ambient must be between 0 and 4".to_owned());
        }
        if self.fog_distance == 0.0 || (0.1..=100_000.0).contains(&self.fog_distance) {
            Ok(())
        } else {
            Err("fogDistance must be 0 (no fog) or between 0.1 and 100000".to_owned())
        }
    }
}

/// Light from a point that shines in every direction and fades out by `range`, like a bulb.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointLight {
    /// Linear RGB, each 0 to 1.
    pub color: [f32; 3],
    pub intensity: f32,
    /// Distance in world units at which the light has faded out.
    pub range: f32,
    /// Shapes around it cast shadows (see `SpotLight::shadows`).
    #[serde(default)]
    pub shadows: bool,
}

impl Default for PointLight {
    fn default() -> Self {
        Self { color: [1.0, 0.95, 0.85], intensity: 8.0, range: 8.0, shadows: false }
    }
}

impl SceneComponent for PointLight {
    const NAME: &'static str = "pointLight";
    const LABEL: &'static str = "Point Light";
    const MENU: &'static str = "Light/Point Light";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        unit_range("color", &self.color)?;
        light_amounts(self.intensity, self.range)
    }
}

/// Light from a point that shines in a cone along the object's local −Z axis, like a torch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpotLight {
    /// Linear RGB, each 0 to 1.
    pub color: [f32; 3],
    pub intensity: f32,
    /// Distance in world units at which the light has faded out.
    pub range: f32,
    /// Full width of the cone in degrees.
    pub angle: f32,
    /// How much of the cone's edge fades: 0 for a hard edge, 1 for a fade from the centre.
    pub softness: f32,
    /// Shapes in the cone cast shadows. One light casts shadows at a time: the first directional
    /// light that does, else the spot or point light nearest the camera that does.
    #[serde(default)]
    pub shadows: bool,
}

impl Default for SpotLight {
    fn default() -> Self {
        Self { color: [1.0, 1.0, 1.0], intensity: 12.0, range: 12.0, angle: 45.0, softness: 0.3, shadows: false }
    }
}

impl SceneComponent for SpotLight {
    const NAME: &'static str = "spotLight";
    const LABEL: &'static str = "Spot Light";
    const MENU: &'static str = "Light/Spot Light";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        unit_range("color", &self.color)?;
        unit_range("softness", &[self.softness])?;
        light_amounts(self.intensity, self.range)?;
        if self.angle.is_finite() && (1.0..=179.0).contains(&self.angle) { Ok(()) } else { Err("angle must be between 1 and 179 degrees".to_owned()) }
    }
}

/// An optional file path field: empty, or at most 512 characters ending in one of `extensions`.
fn file_field(name: &str, value: &str, extensions: &[&str]) -> Result<(), String> {
    if value.is_empty() || (value.len() <= 512 && extensions.iter().any(|e| value.to_ascii_lowercase().ends_with(&format!(".{e}")))) {
        Ok(())
    } else {
        Err(format!("{name} must be the path of a .{} file (at most 512 characters)", extensions.join(" or .")))
    }
}

fn light_amounts(intensity: f32, range: f32) -> Result<(), String> {
    if !(intensity.is_finite() && intensity >= 0.0) {
        return Err("intensity must be 0 or more".to_owned());
    }
    if range.is_finite() && range > 0.0 && range <= 100_000.0 { Ok(()) } else { Err("range must be more than 0 and at most 100000".to_owned()) }
}

/// An unlit flat rectangle in the object's XY plane, for 2D games and markers.
/// Images on sprites come with the asset pipeline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sprite {
    /// Linear RGBA, each 0 to 1. With an image, it tints the image.
    pub color: [f32; 4],
    /// Width and height in scene units.
    pub size: [f32; 2],
    /// A PNG image, relative to the project folder (or the scene's folder outside a project).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image: String,
}

impl Default for Sprite {
    fn default() -> Self {
        Self { color: [1.0, 1.0, 1.0, 1.0], size: [1.0, 1.0], image: String::new() }
    }
}

impl SceneComponent for Sprite {
    const NAME: &'static str = "sprite";
    const LABEL: &'static str = "Sprite";
    const MENU: &'static str = "Rendering/Sprite";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        unit_range("color", &self.color)?;
        file_field("image", &self.image, &["png", "jpg", "jpeg"])?;
        if self.size.iter().all(|s| s.is_finite() && *s > 0.0) { Ok(()) } else { Err("size must be greater than 0".to_owned()) }
    }
}

/// One WebAssembly script on an object; see `docs/script-abi.md`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptRef {
    /// Path to the `.wasm` module or `.bscript` visual script, relative to the project folder (or the scene file's folder outside a project).
    pub module: String,
    /// Property values changed from the script's declared defaults.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub properties: std::collections::BTreeMap<String, f32>,
}

impl ScriptRef {
    /// Display name: the module's file name without `.wasm`, in title case (`spinner.wasm` -> `Spinner`).
    pub fn display_name(&self) -> String {
        let file = self.module.rsplit(['/', '\\']).next().unwrap_or(&self.module);
        // Any script file: `.wasm` modules and `.bscript` visual scripts.
        let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
        stem.split(['_', '-', ' '])
            .filter(|w| !w.is_empty())
            .map(|w| {
                let mut chars = w.chars();
                chars.next().map(|first| first.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The scripts on an object, in the order they run. The inspector shows each one as its own
/// component, named after its module.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Scripts(pub Vec<ScriptRef>);

/// Most scripts one object may carry.
pub const MAX_SCRIPTS: usize = 64;

impl SceneComponent for Scripts {
    const NAME: &'static str = "scripts";
    const LABEL: &'static str = "Scripts";
    const REMOVABLE: bool = true;
    const LISTED: bool = false;

    fn validate(&self) -> Result<(), String> {
        if self.0.len() > MAX_SCRIPTS {
            return Err(format!("an object may carry at most {MAX_SCRIPTS} scripts"));
        }
        for script in &self.0 {
            if script.module.is_empty() || script.module.contains('\0') {
                return Err("a script needs a module path without NUL characters".to_owned());
            }
            finite("script property", script.properties.values())?;
        }
        Ok(())
    }
}

fn finite<'a>(what: &str, mut values: impl Iterator<Item = &'a f32>) -> Result<(), String> {
    if values.all(|v| v.is_finite()) { Ok(()) } else { Err(format!("{what} values must be finite numbers")) }
}

fn unit_range(what: &str, values: &[f32]) -> Result<(), String> {
    if values.iter().all(|v| (0.0..=1.0).contains(v)) { Ok(()) } else { Err(format!("{what} values must be between 0 and 1")) }
}

/// Marks an object as a placed copy of a prefab file. The object and its children came from
/// the file; Apply writes them back to it, Revert reads them again.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prefab {
    /// The `.bprefab` file, relative to the project folder (or the scene's folder outside a project).
    pub source: String,
    /// What this copy changed and keeps when it is updated from the file: `"<index>/<component>"`,
    /// where index counts the copy's objects in hierarchy order (the copy itself is 0).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<String>,
}

/// Most overrides one prefab copy keeps.
pub const MAX_PREFAB_OVERRIDES: usize = 1000;

impl Prefab {
    /// A link to `source` with no overrides.
    pub fn new(source: impl Into<String>) -> Self {
        Self { source: source.into(), overrides: Vec::new() }
    }

    /// The object index and component name of each override.
    pub fn parsed_overrides(&self) -> impl Iterator<Item = (usize, &str)> {
        self.overrides.iter().filter_map(|o| o.split_once('/')).filter_map(|(i, c)| Some((i.parse().ok()?, c)))
    }
}

impl SceneComponent for Prefab {
    const NAME: &'static str = "prefab";
    const LABEL: &'static str = "Prefab";
    const REMOVABLE: bool = true;
    const LISTED: bool = false;

    fn validate(&self) -> Result<(), String> {
        if self.source.is_empty() || self.source.len() > 512 || !self.source.ends_with(".bprefab") {
            return Err("source must be the path of a .bprefab file (at most 512 characters)".to_owned());
        }
        let well_formed = |o: &String| {
            o.len() <= 80
                && o.split_once('/').is_some_and(|(i, c)| i.parse::<usize>().is_ok() && !c.is_empty() && c.chars().all(|ch| ch.is_ascii_alphanumeric()))
        };
        if self.overrides.len() > MAX_PREFAB_OVERRIDES || !self.overrides.iter().all(well_formed) {
            return Err(format!("overrides must be at most {MAX_PREFAB_OVERRIDES} entries like \"2/mesh\""));
        }
        Ok(())
    }
}

/// How a body moves in play mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BodyKind {
    /// Moved by gravity, collisions and impulses.
    #[default]
    Dynamic,
    /// Moved only by its transform (scripts, animation); pushes dynamic bodies.
    Kinematic,
    /// Never moves.
    Static,
}

/// Makes the object a physics body in play mode. Its Collider gives it a shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RigidBody {
    pub kind: BodyKind,
    /// Kilograms, more than 0.
    pub mass: f32,
    /// How strongly gravity pulls it: 1 normal, 0 floating, negative upwards.
    pub gravity_scale: f32,
    /// Keeps it upright: collisions move it but do not turn it.
    pub lock_rotation: bool,
}

impl Default for RigidBody {
    fn default() -> Self {
        Self { kind: BodyKind::Dynamic, mass: 1.0, gravity_scale: 1.0, lock_rotation: false }
    }
}

impl SceneComponent for RigidBody {
    const NAME: &'static str = "rigidBody";
    const LABEL: &'static str = "Rigid Body";
    const MENU: &'static str = "Physics/Rigid Body";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        if !(self.mass.is_finite() && self.mass > 0.0 && self.mass <= 1.0e6) {
            return Err("mass must be more than 0 and at most 1000000".to_owned());
        }
        if !(self.gravity_scale.is_finite() && (-100.0..=100.0).contains(&self.gravity_scale)) {
            return Err("gravityScale must be between -100 and 100".to_owned());
        }
        Ok(())
    }

    fn options(field: &str) -> &'static [&'static str] {
        if field == "kind" { &["dynamic", "kinematic", "static"] } else { &[] }
    }
}

/// A collider's shape, in the object's own space before its scale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColliderShape {
    /// `size` is the box's width, height and depth.
    #[default]
    Box,
    /// `size[0]` is the diameter.
    Sphere,
    /// Upright: `size[0]` is the diameter, `size[1]` the whole height.
    Capsule,
}

/// The shape physics uses for the object: what it collides with, or, as a trigger, what it notices
/// entering and leaving it. Without a Rigid Body the object is a static obstacle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Collider {
    pub shape: ColliderShape,
    pub size: [f32; 3],
    /// 0 slippery to 1 grippy (up to 10).
    pub friction: f32,
    /// Bounciness: 0 none, 1 keeps all its speed.
    pub restitution: f32,
    /// Notices other colliders passing through instead of blocking them.
    pub trigger: bool,
}

impl Default for Collider {
    fn default() -> Self {
        Self { shape: ColliderShape::Box, size: [1.0, 1.0, 1.0], friction: 0.5, restitution: 0.0, trigger: false }
    }
}

impl SceneComponent for Collider {
    const NAME: &'static str = "collider";
    const LABEL: &'static str = "Collider";
    const MENU: &'static str = "Physics/Collider";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        if !self.size.iter().all(|s| s.is_finite() && *s > 0.0 && *s <= 10_000.0) {
            return Err("size values must be more than 0 and at most 10000".to_owned());
        }
        if !(self.friction.is_finite() && (0.0..=10.0).contains(&self.friction)) {
            return Err("friction must be between 0 and 10".to_owned());
        }
        unit_range("restitution", &[self.restitution])
    }

    fn options(field: &str) -> &'static [&'static str] {
        if field == "shape" { &["box", "sphere", "capsule"] } else { &[] }
    }
}

/// The file extensions an Audio Source's clip may have.
pub const SOUND_EXTENSIONS: &[&str] = &["wav", "ogg", "mp3", "aif", "aiff", "opus", "m4a", "aac"];

/// A sound the object plays in play mode: from its own place when spatial, heard from the first
/// camera (quieter with distance, from the side it is on), or the same everywhere.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioSource {
    /// A sound file (WAV, Ogg Vorbis, MP3, Opus, AIFF, AAC), relative to the project folder; empty
    /// for none.
    pub clip: String,
    /// 0 silent, 1 as recorded, up to 4.
    pub volume: f32,
    pub looping: bool,
    /// Starts when play starts, or when the object appears during play.
    pub play_on_start: bool,
    /// Heard from where the object is, rather than the same everywhere.
    pub spatial: bool,
    /// How far a spatial sound is heard, in world units; it fades out toward it.
    pub range: f32,
}

impl Default for AudioSource {
    fn default() -> Self {
        Self { clip: String::new(), volume: 1.0, looping: false, play_on_start: true, spatial: true, range: 30.0 }
    }
}

impl SceneComponent for AudioSource {
    const NAME: &'static str = "audioSource";
    const LABEL: &'static str = "Audio Source";
    const MENU: &'static str = "Audio/Audio Source";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        file_field("clip", &self.clip, SOUND_EXTENSIONS)?;
        if !(self.volume.is_finite() && (0.0..=4.0).contains(&self.volume)) {
            return Err("volume must be between 0 and 4".to_owned());
        }
        if !(self.range.is_finite() && self.range > 0.0 && self.range <= 100_000.0) {
            return Err("range must be more than 0 and at most 100000".to_owned());
        }
        Ok(())
    }
}

/// How a joint lets its object move against what it holds on to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JointKind {
    /// Held fast, as if glued.
    #[default]
    Fixed,
    /// Turns about the axis only, like a door.
    Hinge,
    /// Turns every way about the anchor, like a shoulder.
    Ball,
    /// Slides along the axis only, like a drawer.
    Slider,
}

/// Holds the object's physics body to another object's, or to the world, at the anchor. Physics
/// makes it when play starts (or when the object or the other one appears) and updates its motor
/// and limits when they change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Joint {
    pub kind: JointKind,
    /// The object it holds on to, by name (the first with that name); empty for the world.
    pub connected: String,
    /// Where it holds, in the object's own space.
    pub anchor: [f32; 3],
    /// The hinge's turning axis or the slider's direction, in the object's own space.
    pub axis: [f32; 3],
    /// Whether movement stops at Lower and Upper: degrees for a hinge, world units for a slider.
    pub limited: bool,
    pub lower: f32,
    pub upper: f32,
    /// A hinge or slider driven at this speed: degrees or units per second, the object's own turning
    /// or sliding along the axis (0 holds it still, as a brake).
    pub motor_speed: f32,
    /// The most force (or turning force) the motor uses; 0 for no motor.
    pub motor_force: f32,
}

impl Default for Joint {
    fn default() -> Self {
        Self {
            kind: JointKind::Hinge,
            connected: String::new(),
            anchor: [0.0; 3],
            axis: [0.0, 1.0, 0.0],
            limited: false,
            lower: -45.0,
            upper: 45.0,
            motor_speed: 0.0,
            motor_force: 0.0,
        }
    }
}

impl SceneComponent for Joint {
    const NAME: &'static str = "joint";
    const LABEL: &'static str = "Joint";
    const MENU: &'static str = "Physics/Joint";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        let finite = |v: &[f32]| v.iter().all(|x| x.is_finite() && x.abs() <= 1.0e6);
        if !finite(&self.anchor) || !finite(&self.axis) || !finite(&[self.lower, self.upper, self.motor_speed, self.motor_force]) {
            return Err("joint values must be finite numbers of at most a million".to_owned());
        }
        if self.axis.iter().map(|a| a * a).sum::<f32>() < 1.0e-6 {
            return Err("the axis must not be zero".to_owned());
        }
        if self.lower > self.upper {
            return Err("lower must not be more than upper".to_owned());
        }
        if self.motor_force < 0.0 {
            return Err("motorForce must be 0 or more".to_owned());
        }
        if self.connected.chars().count() > 256 {
            return Err("connected must be a name of at most 256 characters".to_owned());
        }
        Ok(())
    }

    fn options(field: &str) -> &'static [&'static str] {
        if field == "kind" { &["fixed", "hinge", "ball", "slider"] } else { &[] }
    }
}

/// A walking character for play mode: an upright capsule that scripts steer and jump, which
/// climbs steps and gentle slopes, slides along walls and falls under gravity. It needs no Rigid
/// Body or Collider.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CharacterBody {
    /// Whole height of the capsule, feet to head.
    pub height: f32,
    pub radius: f32,
    /// Steepest slope it walks up, in degrees.
    pub max_slope: f32,
    /// Highest step it climbs without jumping.
    pub step_height: f32,
}

impl Default for CharacterBody {
    fn default() -> Self {
        Self { height: 1.8, radius: 0.4, max_slope: 45.0, step_height: 0.3 }
    }
}

impl SceneComponent for CharacterBody {
    const NAME: &'static str = "characterBody";
    const LABEL: &'static str = "Character";
    const MENU: &'static str = "Physics/Character";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        let ok = |v: f32| v.is_finite();
        if !(ok(self.radius) && self.radius > 0.0 && self.radius <= 100.0 && ok(self.height) && self.height >= self.radius * 2.0 && self.height <= 1000.0) {
            return Err("radius must be more than 0 (at most 100) and height at least twice the radius (at most 1000)".to_owned());
        }
        if !(ok(self.max_slope) && (0.0..=89.0).contains(&self.max_slope)) {
            return Err("maxSlope must be between 0 and 89 degrees".to_owned());
        }
        if !(ok(self.step_height) && self.step_height >= 0.0 && self.step_height <= self.height) {
            return Err("stepHeight must be between 0 and the height".to_owned());
        }
        Ok(())
    }
}

/// How a Player Control places the camera.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CameraFollow {
    /// The camera stays where the scene puts it.
    #[default]
    None,
    /// The scene's first camera trails the object at a distance and height, looking at it.
    Follow,
}

/// Lets the player steer this object's Character with the project's input actions (MoveForward,
/// MoveBack, MoveLeft, MoveRight, Jump), relative to where the camera looks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlayerControl {
    /// Walking speed, world units per second.
    pub speed: f32,
    /// Upward speed of a jump.
    pub jump_speed: f32,
    /// Turns the object to face where it walks.
    pub turn_to_move: bool,
    pub camera: CameraFollow,
    pub camera_distance: f32,
    pub camera_height: f32,
}

impl Default for PlayerControl {
    fn default() -> Self {
        Self { speed: 5.0, jump_speed: 5.0, turn_to_move: true, camera: CameraFollow::Follow, camera_distance: 6.0, camera_height: 3.0 }
    }
}

impl SceneComponent for PlayerControl {
    const NAME: &'static str = "playerControl";
    const LABEL: &'static str = "Player Control";
    const MENU: &'static str = "Gameplay/Player Control";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        let ranged = |v: f32, max: f32| v.is_finite() && (0.0..=max).contains(&v);
        if !(ranged(self.speed, 1000.0) && ranged(self.jump_speed, 1000.0)) {
            return Err("speed and jumpSpeed must be between 0 and 1000".to_owned());
        }
        if !(ranged(self.camera_distance, 10_000.0) && self.camera_height.is_finite() && self.camera_height.abs() <= 10_000.0) {
            return Err("cameraDistance must be between 0 and 10000 and cameraHeight at most 10000 either way".to_owned());
        }
        Ok(())
    }

    fn options(field: &str) -> &'static [&'static str] {
        if field == "camera" { &["none", "follow"] } else { &[] }
    }
}

/// What happens when the time limit runs out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimeUp {
    /// The game is lost.
    #[default]
    Lose,
    /// The game is won (survive until the time is up).
    Win,
}

/// The rules of the game in this scene: who the player is, where it starts, when it starts over,
/// and how long the game lasts. Put it on one object; the first one in the hierarchy counts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameRules {
    /// A `.bprefab` placed at the Player Start when play begins, if the scene has no object with
    /// Player Control yet; empty for none.
    pub player_prefab: String,
    /// Players that fall below this height go back to the Player Start.
    pub respawn: bool,
    pub respawn_height: f32,
    /// Seconds until the game ends; 0 for no limit.
    pub time_limit: f32,
    pub time_up: TimeUp,
}

impl Default for GameRules {
    fn default() -> Self {
        Self { player_prefab: String::new(), respawn: true, respawn_height: -20.0, time_limit: 0.0, time_up: TimeUp::Lose }
    }
}

impl SceneComponent for GameRules {
    const NAME: &'static str = "gameRules";
    const LABEL: &'static str = "Game Rules";
    const MENU: &'static str = "Gameplay/Game Rules";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        file_field("playerPrefab", &self.player_prefab, &["bprefab"])?;
        if !self.respawn_height.is_finite() {
            return Err("respawnHeight must be a finite number".to_owned());
        }
        if !(self.time_limit.is_finite() && (0.0..=1.0e7).contains(&self.time_limit)) {
            return Err("timeLimit must be 0 (none) or a number of seconds up to 10000000".to_owned());
        }
        Ok(())
    }

    fn options(field: &str) -> &'static [&'static str] {
        if field == "timeUp" { &["lose", "win"] } else { &[] }
    }
}

/// Where the player appears when play begins and when it starts over: this object's place and
/// turn. The first one in the hierarchy counts.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerStart {}

impl SceneComponent for PlayerStart {
    const NAME: &'static str = "playerStart";
    const LABEL: &'static str = "Player Start";
    const MENU: &'static str = "Gameplay/Player Start";
    const REMOVABLE: bool = true;
}

/// This object's own Input Map: its scripts and Player Control read their actions from this
/// asset instead of the project's (Project Settings), with its own active map.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputMap {
    /// A `.binput` file, relative to the project folder.
    pub asset: String,
    /// The map active when play starts; empty for the asset's own starting map.
    #[serde(default)]
    pub start: String,
}

impl SceneComponent for InputMap {
    const NAME: &'static str = "inputMap";
    const LABEL: &'static str = "Input Map";
    const MENU: &'static str = "Input/Input Map";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        file_field("asset", &self.asset, &["binput"])?;
        if self.start.len() > 64 || !self.start.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err("start must be a map name: up to 64 letters, digits or _".to_owned());
        }
        Ok(())
    }
}
