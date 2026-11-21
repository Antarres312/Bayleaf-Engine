//! Built-in scene components.

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
        let all = self.position.iter().chain(&self.rotation).chain(&self.scale);
        if all.copied().all(f32::is_finite) { Ok(()) } else { Err("transform values must be finite numbers".to_owned()) }
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
