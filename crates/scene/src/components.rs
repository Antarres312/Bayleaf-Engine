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
    /// A post-processing asset (`.bpost`) for what this camera shows; empty for the scene's.
    #[serde(default, rename = "postProcessing", skip_serializing_if = "String::is_empty")]
    pub post_processing: String,
}

impl Default for Camera {
    fn default() -> Self {
        Self { projection: Projection::Perspective, fov: 60.0, size: 5.0, near: 0.1, far: 1000.0, post_processing: String::new() }
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
        crate::lighting::file_path("postProcessing", &self.post_processing, crate::post::EXTENSION)?;
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
    // Older scenes only: a lighting asset in the scene's settings holds this now.
    const LISTED: bool = false;

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

/// What kind of value a script property holds: what the Inspector edits it with and how the script
/// reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PropertyType {
    #[default]
    Number,
    /// A 32-bit whole number.
    WholeNumber,
    TrueFalse,
    Vector,
    Color,
    /// Text up to [`MAX_PROPERTY_TEXT`] bytes.
    Text,
    /// An object of the scene, by its name; empty for none.
    Object,
}

/// Longest text (and object name) a script property holds, in bytes.
pub const MAX_PROPERTY_TEXT: usize = 1024;

impl PropertyType {
    pub const ALL: [PropertyType; 7] = [
        PropertyType::Number,
        PropertyType::WholeNumber,
        PropertyType::TrueFalse,
        PropertyType::Vector,
        PropertyType::Color,
        PropertyType::Text,
        PropertyType::Object,
    ];

    /// English name shown to people.
    pub fn label(self) -> &'static str {
        match self {
            PropertyType::Number => "Number",
            PropertyType::WholeNumber => "Whole Number",
            PropertyType::TrueFalse => "True/False",
            PropertyType::Vector => "Vector",
            PropertyType::Color => "Color",
            PropertyType::Text => "Text",
            PropertyType::Object => "Object",
        }
    }

    /// The word scripts declare it with (`bayleaf_describe`): `number`, `whole`, `bool`, `vector`,
    /// `color`, `text` or `object`.
    pub fn code(self) -> &'static str {
        match self {
            PropertyType::Number => "number",
            PropertyType::WholeNumber => "whole",
            PropertyType::TrueFalse => "bool",
            PropertyType::Vector => "vector",
            PropertyType::Color => "color",
            PropertyType::Text => "text",
            PropertyType::Object => "object",
        }
    }

    pub fn from_code(code: &str) -> Option<PropertyType> {
        PropertyType::ALL.into_iter().find(|t| t.code() == code)
    }

    /// The value a property of this type has when nothing sets it.
    pub fn zero(self) -> PropertyValue {
        match self {
            PropertyType::Number | PropertyType::WholeNumber => PropertyValue::Number(0.0),
            PropertyType::TrueFalse => PropertyValue::Flag(false),
            PropertyType::Vector => PropertyValue::Triple([0.0; 3]),
            PropertyType::Color => PropertyValue::Triple([1.0; 3]),
            PropertyType::Text | PropertyType::Object => PropertyValue::Text(String::new()),
        }
    }
}

/// A script property's value, stored in the scene as plain JSON: a number, `true`/`false`, three
/// numbers (a vector or a colour) or text (text, or an object's name).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PropertyValue {
    Flag(bool),
    Number(f64),
    Triple([f32; 3]),
    Text(String),
}

impl PropertyValue {
    /// Whether the value can be a property of type `ty`: the right shape, finite numbers, a whole
    /// number in 32 bits, colours from 0 to 1, text not too long.
    pub fn fits(&self, ty: PropertyType) -> bool {
        match (ty, self) {
            (PropertyType::Number, PropertyValue::Number(v)) => v.is_finite() && v.abs() <= f64::from(f32::MAX),
            (PropertyType::WholeNumber, PropertyValue::Number(v)) => v.fract() == 0.0 && (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(v),
            (PropertyType::TrueFalse, PropertyValue::Flag(_)) => true,
            (PropertyType::Vector, PropertyValue::Triple(v)) => v.iter().all(|n| n.is_finite()),
            (PropertyType::Color, PropertyValue::Triple(v)) => v.iter().all(|n| (0.0..=1.0).contains(n)),
            (PropertyType::Text | PropertyType::Object, PropertyValue::Text(t)) => t.len() <= MAX_PROPERTY_TEXT && !t.contains('\0'),
            _ => false,
        }
    }

    /// The type a value of this shape has when no script declares it: a number, true/false, a
    /// vector or text.
    pub fn shape_type(&self) -> PropertyType {
        match self {
            PropertyValue::Flag(_) => PropertyType::TrueFalse,
            PropertyValue::Number(_) => PropertyType::Number,
            PropertyValue::Triple(_) => PropertyType::Vector,
            PropertyValue::Text(_) => PropertyType::Text,
        }
    }

    /// The value as one number: a number, a whole number, or 1 and 0 for true and false.
    pub fn number(&self) -> Option<f32> {
        match self {
            PropertyValue::Number(v) => Some(*v as f32),
            PropertyValue::Flag(b) => Some(f32::from(u8::from(*b))),
            _ => None,
        }
    }

    /// The value as three numbers, as Get Component reads fields: one number in the first place,
    /// or a vector or colour.
    pub fn numbers(&self) -> Option<[f32; 3]> {
        match self {
            PropertyValue::Triple(v) => Some(*v),
            other => other.number().map(|n| [n, 0.0, 0.0]),
        }
    }

    pub fn text(&self) -> Option<&str> {
        if let PropertyValue::Text(t) = self { Some(t) } else { None }
    }

    /// A value of type `ty` from three numbers, as Set Component writes fields; `None` for text.
    pub fn from_numbers(ty: PropertyType, [x, y, z]: [f32; 3]) -> Option<PropertyValue> {
        let value = match ty {
            PropertyType::Number => PropertyValue::Number(f64::from(x)),
            PropertyType::WholeNumber => PropertyValue::Number(f64::from(x).round().clamp(f64::from(i32::MIN), f64::from(i32::MAX))),
            PropertyType::TrueFalse => PropertyValue::Flag(x != 0.0),
            PropertyType::Vector => PropertyValue::Triple([x, y, z]),
            PropertyType::Color => PropertyValue::Triple([x, y, z].map(|c| c.clamp(0.0, 1.0))),
            PropertyType::Text | PropertyType::Object => return None,
        };
        value.fits(ty).then_some(value)
    }
}

/// A property a script declares: its name, type and default.
#[derive(Clone, Debug, PartialEq)]
pub struct DeclaredProperty {
    pub name: String,
    pub ty: PropertyType,
    pub default: PropertyValue,
}

/// One WebAssembly script on an object; see `docs/script-abi.md`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptRef {
    /// Path to the `.wasm` module or `.bscript` visual script, relative to the project folder (or the scene file's folder outside a project).
    pub module: String,
    /// Property values changed from the script's declared defaults. Each must fit the type the script
    /// declares; one that does not is ignored when the script runs.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub properties: std::collections::BTreeMap<String, PropertyValue>,
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
            if script.properties.values().any(|v| !PropertyType::ALL.iter().any(|ty| v.fits(*ty))) {
                return Err("a script property must be a finite number, true or false, three finite numbers, or text up to 1024 bytes".to_owned());
            }
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

/// A game interface over the game: a layout file (`.bui`) of panels, text, buttons and other
/// elements. Its buttons, toggles, sliders and text fields send the object's scripts messages
/// (`pressed:<name>`, `changed:<name>`), and scripts change its elements while the game plays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Interface {
    /// A layout file (`.bui`), relative to the project folder; empty for none.
    pub layout: String,
    /// Drawn and answering the player; scripts show and hide it.
    pub visible: bool,
    /// Interfaces with a higher order are drawn over those with a lower one.
    pub order: i32,
}

impl Default for Interface {
    fn default() -> Self {
        Self { layout: String::new(), visible: true, order: 0 }
    }
}

impl SceneComponent for Interface {
    const NAME: &'static str = "interface";
    const LABEL: &'static str = "Interface";
    const MENU: &'static str = "Interface/Interface";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        file_field("layout", &self.layout, &["bui"])?;
        if (-1000..=1000).contains(&self.order) { Ok(()) } else { Err("order must be between -1000 and 1000".to_owned()) }
    }
}

/// A part of the world kept in its own scene file and added under the object while playing, in
/// the background, when a camera comes within Load Distance; it goes again once every camera is
/// further than Load Distance and Keep Margin. Its objects are placed relative to the object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StreamedArea {
    /// A `.bscene` file, relative to the project folder; empty for none.
    pub scene: String,
    /// How near a camera comes before the scene is added, in world units.
    pub load_distance: f64,
    /// How much further the cameras go before it is removed again, so it does not come and go at the edge.
    pub keep_margin: f64,
}

impl Default for StreamedArea {
    fn default() -> Self {
        Self { scene: String::new(), load_distance: 100.0, keep_margin: 20.0 }
    }
}

impl SceneComponent for StreamedArea {
    const NAME: &'static str = "streamedArea";
    const LABEL: &'static str = "Streamed Area";
    const MENU: &'static str = "World/Streamed Area";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        file_field("scene", &self.scene, &["bscene"])?;
        if !(self.load_distance.is_finite() && self.load_distance > 0.0 && self.load_distance <= 1.0e7) {
            return Err("load distance must be more than 0 and at most 10000000".to_owned());
        }
        if !(self.keep_margin.is_finite() && (0.0..=1.0e7).contains(&self.keep_margin)) {
            return Err("keep margin must be between 0 and 10000000".to_owned());
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

/// Particles: many small images (or shapes) that start from the object, move, change size and
/// colour over their life, and go. The engine simulates them while the game plays, and the Scene
/// view plays the selected emitter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParticleEmitter {
    /// Starts emitting when the game starts; scripts start and stop it otherwise.
    pub playing: bool,
    /// Starts again at the end of each cycle; off, it emits for one cycle.
    pub looping: bool,
    /// Seconds in a cycle.
    pub duration: f32,
    /// Particles a second.
    pub rate: f32,
    /// Particles at once at the start of each cycle.
    pub burst: u32,
    /// Most particles alive at once.
    pub max_particles: u32,
    /// Where particles start: `point`, `sphere`, `cone` or `box`.
    pub shape: String,
    /// The sphere's radius, or the cone's at its base.
    pub radius: f32,
    /// How wide the cone opens, degrees from its axis (the object's +Y).
    pub angle: f32,
    /// The box's size.
    pub box_size: [f32; 3],
    /// Shortest and longest life, seconds.
    pub lifetime: [f32; 2],
    /// Slowest and fastest start, units a second.
    pub speed: [f32; 2],
    /// How much gravity pulls them: 1 as falling things do, 0 not at all, less than 0 up.
    pub gravity: f32,
    /// How fast air slows them, 0 for not at all.
    pub drag: f32,
    pub start_size: f32,
    pub middle_size: f32,
    pub end_size: f32,
    pub start_color: [f32; 4],
    pub middle_color: [f32; 4],
    pub end_color: [f32; 4],
    /// Slowest and fastest spin, degrees a second.
    pub spin: [f32; 2],
    /// How they draw: `sprite` (facing the camera), `stretched` (along their motion) or `mesh`.
    pub render: String,
    /// The image of a sprite, relative to the project folder; empty for a soft dot.
    pub image: String,
    /// The image as a grid of frames shown one after another over each particle's life.
    pub flipbook_columns: u32,
    pub flipbook_rows: u32,
    /// The shape a `mesh` particle is.
    pub mesh_shape: Shape,
    /// A material graph (`.bmaterial`) for the particles; empty for their colour and image.
    pub material: String,
    /// `alpha` blends them over what is behind; `additive` adds their light (fire, sparks).
    pub blend: String,
    /// `world`: particles stay where they started as the object moves; `local`: they move with it.
    pub space: String,
}

impl Default for ParticleEmitter {
    fn default() -> Self {
        Self {
            playing: true,
            looping: true,
            duration: 5.0,
            rate: 10.0,
            burst: 0,
            max_particles: 500,
            shape: "cone".to_owned(),
            radius: 0.2,
            angle: 25.0,
            box_size: [1.0, 1.0, 1.0],
            lifetime: [1.5, 2.5],
            speed: [1.0, 2.0],
            gravity: 0.0,
            drag: 0.0,
            start_size: 0.2,
            middle_size: 0.25,
            end_size: 0.1,
            start_color: [1.0, 1.0, 1.0, 1.0],
            middle_color: [1.0, 1.0, 1.0, 0.8],
            end_color: [1.0, 1.0, 1.0, 0.0],
            spin: [0.0, 0.0],
            render: "sprite".to_owned(),
            image: String::new(),
            flipbook_columns: 1,
            flipbook_rows: 1,
            mesh_shape: Shape::Cube,
            material: String::new(),
            blend: "alpha".to_owned(),
            space: "world".to_owned(),
        }
    }
}

/// Most particles one emitter may keep alive.
pub const MAX_PARTICLES: u32 = 10_000;

impl SceneComponent for ParticleEmitter {
    const NAME: &'static str = "particleEmitter";
    const LABEL: &'static str = "Particle Emitter";
    const MENU: &'static str = "Effects/Particle Emitter";
    const REMOVABLE: bool = true;

    fn validate(&self) -> Result<(), String> {
        let range = |name: &str, v: f32, low: f32, high: f32| {
            if v.is_finite() && (low..=high).contains(&v) { Ok(()) } else { Err(format!("{name} must be between {low} and {high}")) }
        };
        range("duration", self.duration, 0.01, 3600.0)?;
        range("rate", self.rate, 0.0, 10_000.0)?;
        if self.burst > MAX_PARTICLES || self.max_particles > MAX_PARTICLES {
            return Err(format!("burst and maxParticles are at most {MAX_PARTICLES}"));
        }
        range("radius", self.radius, 0.0, 10_000.0)?;
        range("angle", self.angle, 0.0, 180.0)?;
        for v in self.box_size {
            range("boxSize", v, 0.0, 10_000.0)?;
        }
        for (name, [low, high], top) in [("lifetime", self.lifetime, 3600.0), ("speed", self.speed, 10_000.0), ("spin", self.spin, 100_000.0)] {
            let bottom = if name == "spin" { -top } else { 0.0 };
            range(name, low, bottom, top)?;
            range(name, high, bottom, top)?;
            if high < low {
                return Err(format!("{name}: the second number is the larger"));
            }
        }
        if self.lifetime[1] <= 0.0 {
            return Err("lifetime must be more than 0".to_owned());
        }
        range("gravity", self.gravity, -100.0, 100.0)?;
        range("drag", self.drag, 0.0, 100.0)?;
        for v in [self.start_size, self.middle_size, self.end_size] {
            range("size", v, 0.0, 10_000.0)?;
        }
        for c in [self.start_color, self.middle_color, self.end_color] {
            for v in c {
                range("color", v, 0.0, 1.0)?;
            }
        }
        if !(1..=64).contains(&self.flipbook_columns) || !(1..=64).contains(&self.flipbook_rows) {
            return Err("flipbookColumns and flipbookRows are 1 to 64".to_owned());
        }
        for (field, value) in [("shape", &self.shape), ("render", &self.render), ("blend", &self.blend), ("space", &self.space)] {
            if !Self::options(field).contains(&value.as_str()) {
                return Err(format!("{field} is one of {}", Self::options(field).join(", ")));
            }
        }
        file_field("image", &self.image, &["png", "jpg", "jpeg"])?;
        file_field("material", &self.material, &["bmaterial"])
    }

    fn options(field: &str) -> &'static [&'static str] {
        match field {
            "shape" => &["point", "sphere", "cone", "box"],
            "render" => &["sprite", "stretched", "mesh"],
            "blend" => &["alpha", "additive"],
            "space" => &["world", "local"],
            _ => &[],
        }
    }
}
