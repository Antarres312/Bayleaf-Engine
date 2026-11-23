//! Post-processing assets (`.bpost`): what the camera does to the drawn picture. A scene names one in
//! its settings (`SceneSettings::post_processing`) and a camera may name its own; each effect is on
//! or off with its settings, applied in a fixed order: occlusion, depth fog, bloom, exposure and
//! tone mapping, colour, antialiasing, vignette.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::SceneError;

/// File extension of post-processing assets.
pub const EXTENSION: &str = "bpost";
/// `format` of post-processing files.
pub const FORMAT: &str = "bayleaf.post";
/// Current post-processing file version.
/// Version 2 adds the colour lookup image (`color.lookup`, `color.lookupStrength`); version 3 makes
/// `antialiasing` a choice (`off`, `fast`, `fine`; true and false still read as fast and off).
pub const VERSION: u32 = 3;

/// Every effect, each on or off with its settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PostProcessing {
    #[serde(default)]
    pub tone_mapping: ToneMapping,
    #[serde(default)]
    pub bloom: Bloom,
    #[serde(default)]
    pub color: ColorAdjustment,
    #[serde(default)]
    pub vignette: Vignette,
    #[serde(default)]
    pub depth_fog: DepthFog,
    #[serde(default)]
    pub occlusion: Occlusion,
    /// How the finished picture's jagged edges are smoothed, after the scene's own multisampling.
    #[serde(default)]
    pub antialiasing: Antialiasing,
}

/// How a picture's remaining jagged edges are smoothed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Antialiasing {
    #[default]
    Off,
    /// Each pixel blended with its four neighbours where they differ sharply: quick, a little soft.
    Fast,
    /// Along each edge's own direction, sub-pixel steps too: smoother edges and thin lines.
    Fine,
}

impl<'de> Deserialize<'de> for Antialiasing {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Files before version 3 kept it on or off.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Stored {
            OnOff(bool),
            Named(String),
        }
        match Stored::deserialize(deserializer)? {
            Stored::OnOff(true) => Ok(Antialiasing::Fast),
            Stored::OnOff(false) => Ok(Antialiasing::Off),
            Stored::Named(name) => match name.as_str() {
                "off" => Ok(Antialiasing::Off),
                "fast" => Ok(Antialiasing::Fast),
                "fine" => Ok(Antialiasing::Fine),
                other => Err(serde::de::Error::custom(format!("antialiasing is off, fast or fine, not {other:?}"))),
            },
        }
    }
}

/// High-range colour onto the screen with a filmic curve, after exposure.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToneMapping {
    pub enabled: bool,
    /// Brightness before the curve, 0 to 16; 1 as drawn.
    pub exposure: f32,
}

impl Default for ToneMapping {
    fn default() -> Self {
        Self { enabled: true, exposure: 1.0 }
    }
}

/// Bright parts glow.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Bloom {
    pub enabled: bool,
    /// How strongly, 0 to 4.
    pub strength: f32,
    /// Brightness from which parts glow, 0 to 16.
    pub threshold: f32,
}

impl Default for Bloom {
    fn default() -> Self {
        Self { enabled: false, strength: 0.6, threshold: 1.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColorAdjustment {
    pub enabled: bool,
    /// 0 to 2; 1 as drawn.
    pub contrast: f32,
    /// 0 (grey) to 2; 1 as drawn.
    pub saturation: f32,
    /// A lookup image (a project path; empty for none) that maps each colour to another: N squares of
    /// N by N pixels side by side (so N² wide and N high, N from 2 to 64), one square for each step
    /// of blue, red across each square and green down it, all as sRGB. Applied after contrast and
    /// saturation.
    #[serde(default)]
    pub lookup: String,
    /// How much of the lookup applies, 0 to 1.
    #[serde(default = "full")]
    pub lookup_strength: f32,
}

fn full() -> f32 {
    1.0
}

/// Most steps per side of a lookup image.
pub const MAX_LOOKUP_STEPS: u32 = 64;

/// Whether an image of this size is a lookup image: N squares of N by N side by side.
pub fn is_lookup_size(width: u32, height: u32) -> bool {
    (2..=MAX_LOOKUP_STEPS).contains(&height) && width == height * height
}

impl Default for ColorAdjustment {
    fn default() -> Self {
        Self { enabled: false, contrast: 1.0, saturation: 1.0, lookup: String::new(), lookup_strength: 1.0 }
    }
}

/// Darker corners.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Vignette {
    pub enabled: bool,
    /// 0 to 1.
    pub strength: f32,
    /// How gradually it sets in, 0 to 1.
    pub smoothness: f32,
}

impl Default for Vignette {
    fn default() -> Self {
        Self { enabled: false, strength: 0.4, smoothness: 0.5 }
    }
}

/// Fog by distance from the camera, from Start, hiding everything from End.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DepthFog {
    pub enabled: bool,
    /// Linear RGB, each 0 to 1.
    pub color: [f32; 3],
    pub start: f32,
    pub end: f32,
}

impl Default for DepthFog {
    fn default() -> Self {
        Self { enabled: false, color: [0.7, 0.75, 0.8], start: 20.0, end: 200.0 }
    }
}

/// Creases and corners go darker (ambient occlusion, from depth).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Occlusion {
    pub enabled: bool,
    /// 0 to 1.
    pub strength: f32,
    /// How far around a point it looks, in world units, 0.01 to 10.
    pub radius: f32,
}

impl Default for Occlusion {
    fn default() -> Self {
        Self { enabled: false, strength: 0.6, radius: 0.5 }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PostFile {
    format: String,
    version: u32,
    post: PostProcessing,
}

fn within(name: &str, value: f32, low: f32, high: f32) -> Result<(), String> {
    if value.is_finite() && (low..=high).contains(&value) { Ok(()) } else { Err(format!("{name} must be between {low} and {high}")) }
}

impl PostProcessing {
    /// Checks every value is in its range.
    pub fn validate(&self) -> Result<(), String> {
        within("toneMapping.exposure", self.tone_mapping.exposure, 0.0, 16.0)?;
        within("bloom.strength", self.bloom.strength, 0.0, 4.0)?;
        within("bloom.threshold", self.bloom.threshold, 0.0, 16.0)?;
        within("color.contrast", self.color.contrast, 0.0, 2.0)?;
        within("color.saturation", self.color.saturation, 0.0, 2.0)?;
        within("color.lookupStrength", self.color.lookup_strength, 0.0, 1.0)?;
        if self.color.lookup.len() > 1024 {
            return Err("color.lookup is longer than 1024 bytes".to_owned());
        }
        within("vignette.strength", self.vignette.strength, 0.0, 1.0)?;
        within("vignette.smoothness", self.vignette.smoothness, 0.0, 1.0)?;
        for c in self.depth_fog.color {
            within("depthFog.color", c, 0.0, 1.0)?;
        }
        within("depthFog.start", self.depth_fog.start, 0.0, 1_000_000.0)?;
        within("depthFog.end", self.depth_fog.end, 0.0, 1_000_000.0)?;
        if self.depth_fog.end <= self.depth_fog.start {
            return Err("depthFog.end must be past depthFog.start".to_owned());
        }
        within("occlusion.strength", self.occlusion.strength, 0.0, 1.0)?;
        within("occlusion.radius", self.occlusion.radius, 0.01, 10.0)
    }

    pub fn to_text(&self) -> String {
        let file = PostFile { format: FORMAT.to_owned(), version: VERSION, post: self.clone() };
        let mut text = serde_json::to_string_pretty(&file).unwrap_or_default();
        text.push('\n');
        text
    }

    pub fn from_text(text: &str) -> Result<Self, SceneError> {
        let file: PostFile = serde_json::from_str(text).map_err(|e| SceneError::Format(e.to_string()))?;
        if file.format != FORMAT {
            return Err(SceneError::Format(format!("not a post-processing file (format \"{}\")", file.format)));
        }
        if !(1..=VERSION).contains(&file.version) {
            return Err(SceneError::Format(format!("unsupported post-processing version {}", file.version)));
        }
        file.post.validate().map_err(SceneError::Format)?;
        Ok(file.post)
    }

    pub fn load(path: &Path) -> Result<Self, SceneError> {
        let text = bayleaf_core::files::read_to_string(path).map_err(|e| SceneError::Io(format!("cannot read {}: {e}", path.display())))?;
        Self::from_text(&text)
    }

    /// Writes the file through a temporary one, so a failed write keeps the old file.
    pub fn save(&self, path: &Path) -> Result<(), SceneError> {
        self.validate().map_err(SceneError::Format)?;
        let tmp = path.with_extension("bpost.tmp");
        std::fs::write(&tmp, self.to_text()).map_err(|e| SceneError::Io(format!("cannot write {}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, path).map_err(|e| SceneError::Io(format!("cannot replace {}: {e}", path.display())))
    }
}
