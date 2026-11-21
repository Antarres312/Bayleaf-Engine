//! Built-in scene components.
//!
//! Axis convention: right-handed, +Y up. Cameras and lights look along their local −Z axis.

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
}

impl Default for Mesh {
    fn default() -> Self {
        Self { shape: Shape::Cube, color: [0.8, 0.8, 0.8, 1.0], metallic: 0.0, roughness: 0.5 }
    }
}

impl SceneComponent for Mesh {
    const NAME: &'static str = "mesh";
    const LABEL: &'static str = "Mesh";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        unit_range("color", &self.color)?;
        unit_range("metallic", &[self.metallic])?;
        unit_range("roughness", &[self.roughness])
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

/// A WebAssembly script attached to the object; see `docs/script-abi.md`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Script {
    /// Path to the `.wasm` module, relative to the scene file's folder.
    pub module: String,
    /// Numbers the script reads with `property(name)`.
    #[serde(default)]
    pub properties: std::collections::BTreeMap<String, f32>,
}

impl SceneComponent for Script {
    const NAME: &'static str = "script";
    const LABEL: &'static str = "Script";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        if self.module.contains('\0') {
            return Err("module path contains a NUL character".to_owned());
        }
        finite("script property", self.properties.values())
    }
}

fn finite<'a>(what: &str, mut values: impl Iterator<Item = &'a f32>) -> Result<(), String> {
    if values.all(|v| v.is_finite()) { Ok(()) } else { Err(format!("{what} values must be finite numbers")) }
}

fn unit_range(what: &str, values: &[f32]) -> Result<(), String> {
    if values.iter().all(|v| (0.0..=1.0).contains(v)) { Ok(()) } else { Err(format!("{what} values must be between 0 and 1")) }
}
