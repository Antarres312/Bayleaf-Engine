//! Built-in scene components.
//!
//! Axis convention: right-handed, +Y up. Cameras and lights look along their local â’Z axis.

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use bayleaf_core::math::{EulerRot, Mat4, Quat, Vec3};
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}

impl Default for Transform {
    fn default() -> Self {
        Self { position: [0.0; 3], rotation: [0.0; 3], scale: [1.0; 3] }
    }
}

impl Transform {
    /// Local-to-parent matrix.
    pub fn matrix(&self) -> Mat4 {
        let [rx, ry, rz] = self.rotation.map(f32::to_radians);
        Mat4::from_scale_rotation_translation(Vec3::from(self.scale), Quat::from_euler(EulerRot::XYZ, rx, ry, rz), Vec3::from(self.position))
    }
}

impl SceneComponent for Transform {
    const NAME: &'static str = "transform";
    const LABEL: &'static str = "Transform";
    const REMOVABLE: bool = false;

    fn validate(&self) -> Result<(), String> {
        finite("transform", self.position.iter().chain(&self.rotation).chain(&self.scale))
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
}

impl Default for Mesh {
    fn default() -> Self {
        Self { shape: Shape::Cube, color: [0.8, 0.8, 0.8, 1.0], metallic: 0.0, roughness: 0.5, material: String::new() }
    }
}

impl SceneComponent for Mesh {
    const NAME: &'static str = "mesh";
    const LABEL: &'static str = "Mesh";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        unit_range("color", &self.color)?;
        unit_range("metallic", &[self.metallic])?;
        unit_range("roughness", &[self.roughness])?;
        if !self.material.is_empty() && (self.material.len() > 512 || !self.material.ends_with(".bmaterial")) {
            return Err("material must be the path of a .bmaterial file (at most 512 characters)".to_owned());
        }
        Ok(())
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

/// A viewpoint the scene can be rendered from. It looks along its local â’Z axis.
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

/// Light from a distant source, such as the sun. It shines along the object's local â’Z axis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectionalLight {
    /// Linear RGB, each 0 to 1.
    pub color: [f32; 3],
    pub intensity: f32,
}

impl Default for DirectionalLight {
    fn default() -> Self {
        Self { color: [1.0, 1.0, 1.0], intensity: 3.0 }
    }
}

impl SceneComponent for DirectionalLight {
    const NAME: &'static str = "directionalLight";
    const LABEL: &'static str = "Directional Light";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        unit_range("color", &self.color)?;
        if self.intensity.is_finite() && self.intensity >= 0.0 { Ok(()) } else { Err("intensity must be 0 or more".to_owned()) }
    }
}

/// An unlit flat rectangle in the object's XY plane, for 2D games and markers.
/// Images on sprites come with the asset pipeline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sprite {
    /// Linear RGBA, each 0 to 1.
    pub color: [f32; 4],
    /// Width and height in scene units.
    pub size: [f32; 2],
}

impl Default for Sprite {
    fn default() -> Self {
        Self { color: [1.0, 1.0, 1.0, 1.0], size: [1.0, 1.0] }
    }
}

impl SceneComponent for Sprite {
    const NAME: &'static str = "sprite";
    const LABEL: &'static str = "Sprite";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        unit_range("color", &self.color)?;
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
