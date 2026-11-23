//! Lighting assets (`.blight`): what a scene's light looks like, kept in a file of its own so
//! several scenes can share it. A scene names one in its settings (`SceneSettings::lighting`).
//! It holds the sky's colours, the light around everything, fog, and a sun that lights the scene
//! where it has no Directional Light of its own.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::SceneError;
use crate::components::{DirectionalLight, Environment};

/// File extension of lighting assets.
pub const EXTENSION: &str = "blight";
/// `format` of lighting files.
pub const FORMAT: &str = "bayleaf.lighting";
/// Current lighting file version.
pub const VERSION: u32 = 1;

/// A scene's light.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Lighting {
    /// Linear RGB straight up, each 0 to 1.
    pub sky_color: [f32; 3],
    /// Linear RGB at the horizon; also the fog's colour.
    pub horizon_color: [f32; 3],
    /// Linear RGB below the horizon.
    pub ground_color: [f32; 3],
    /// How strongly the sky and the ground light shapes, 0 to 4.
    pub ambient: f32,
    /// Draw the sky behind the scene; off, the background is the horizon colour.
    pub show_sky: bool,
    /// Distance at which fog hides half of what is behind it; 0 for no fog.
    #[serde(default)]
    pub fog_distance: f32,
    #[serde(default)]
    pub sun: Sun,
}

/// The sun of a lighting asset: a light from far away, as a Directional Light gives.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Sun {
    /// Lights scenes that have no Directional Light of their own.
    pub enabled: bool,
    /// Its direction as an object's rotation in degrees: the light travels along the object's −Z.
    pub rotation: [f32; 3],
    /// Linear RGB, each 0 to 1.
    pub color: [f32; 3],
    /// 0 to 100.
    pub intensity: f32,
    pub shadows: bool,
}

impl Default for Sun {
    fn default() -> Self {
        let light = DirectionalLight::default();
        Self { enabled: true, rotation: [-50.0, 30.0, 0.0], color: light.color, intensity: light.intensity, shadows: light.shadows }
    }
}

impl Default for Lighting {
    fn default() -> Self {
        Self::from_environment(&Environment::default(), Sun::default())
    }
}

/// A lighting file: its format and version around the lighting itself.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LightingFile {
    format: String,
    version: u32,
    lighting: Lighting,
}

fn unit(name: &str, values: &[f32; 3]) -> Result<(), String> {
    if values.iter().all(|v| (0.0..=1.0).contains(v)) { Ok(()) } else { Err(format!("{name} values must be between 0 and 1")) }
}

impl Lighting {
    /// The light an Environment component gave, with `sun`; how older scenes move to lighting assets.
    pub fn from_environment(env: &Environment, sun: Sun) -> Self {
        Self {
            sky_color: env.sky_color,
            horizon_color: env.horizon_color,
            ground_color: env.ground_color,
            ambient: env.ambient,
            show_sky: env.show_sky,
            fog_distance: env.fog_distance,
            sun,
        }
    }

    /// The same light as an Environment component, for drawing (the sun aside).
    pub fn environment(&self) -> Environment {
        Environment {
            sky_color: self.sky_color,
            horizon_color: self.horizon_color,
            ground_color: self.ground_color,
            ambient: self.ambient,
            show_sky: self.show_sky,
            fog_distance: self.fog_distance,
        }
    }

    /// Checks every value is in its range.
    pub fn validate(&self) -> Result<(), String> {
        crate::components::SceneComponent::validate(&self.environment())?;
        unit("sun.color", &self.sun.color)?;
        if !self.sun.rotation.iter().all(|r| r.is_finite()) {
            return Err("sun.rotation values must be finite".to_owned());
        }
        if !(0.0..=100.0).contains(&self.sun.intensity) {
            return Err("sun.intensity must be between 0 and 100".to_owned());
        }
        unit("skyColor", &self.sky_color)
    }

    /// The file's text.
    pub fn to_text(&self) -> String {
        let file = LightingFile { format: FORMAT.to_owned(), version: VERSION, lighting: self.clone() };
        let mut text = serde_json::to_string_pretty(&file).unwrap_or_default();
        text.push('\n');
        text
    }

    /// Reads a lighting file's text.
    pub fn from_text(text: &str) -> Result<Self, SceneError> {
        let file: LightingFile = serde_json::from_str(text).map_err(|e| SceneError::Format(e.to_string()))?;
        if file.format != FORMAT {
            return Err(SceneError::Format(format!("not a lighting file (format \"{}\")", file.format)));
        }
        if !(1..=VERSION).contains(&file.version) {
            return Err(SceneError::Format(format!("unsupported lighting version {}", file.version)));
        }
        file.lighting.validate().map_err(SceneError::Format)?;
        Ok(file.lighting)
    }

    pub fn load(path: &Path) -> Result<Self, SceneError> {
        let text = bayleaf_core::files::read_to_string(path).map_err(|e| SceneError::Io(format!("cannot read {}: {e}", path.display())))?;
        Self::from_text(&text)
    }

    /// Writes the file through a temporary one, so a failed write keeps the old file.
    pub fn save(&self, path: &Path) -> Result<(), SceneError> {
        self.validate().map_err(SceneError::Format)?;
        let tmp = path.with_extension("blight.tmp");
        std::fs::write(&tmp, self.to_text()).map_err(|e| SceneError::Io(format!("cannot write {}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, path).map_err(|e| SceneError::Io(format!("cannot replace {}: {e}", path.display())))
    }
}

/// A scene's own settings, kept in its file beside its objects.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SceneSettings {
    /// A lighting asset (`.blight`), relative to the project folder; empty for the engine's light.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub lighting: String,
    /// A post-processing asset (`.bpost`), relative to the project folder; empty for none. A
    /// camera's own comes first.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub post_processing: String,
}

/// Whether `path` is empty or a short path of a file with `extension`.
pub(crate) fn file_path(field: &str, path: &str, extension: &str) -> Result<(), String> {
    if path.is_empty() || (path.len() <= 512 && path.to_ascii_lowercase().ends_with(&format!(".{extension}"))) {
        Ok(())
    } else {
        Err(format!("{field} must be the path of a .{extension} file"))
    }
}

impl SceneSettings {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn validate(&self) -> Result<(), String> {
        file_path("lighting", &self.lighting, EXTENSION)?;
        file_path("postProcessing", &self.post_processing, crate::post::EXTENSION)
    }
}
