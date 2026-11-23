//! Import settings: a small JSON file beside a model or image brought into a project
//! (`robot.glb.bimport` beside `robot.glb`), saying how to read it: a model's scale and which way
//! is up, whether an image gets smaller copies for far away, holds colour or plain numbers, and is
//! kept block-compressed on the GPU, and whether a sound is played straight from its file. The
//! loaders apply them, and read the
//! file again when the settings change. A file without settings is read with the defaults.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The settings file's extension, added after the file's own.
pub const SETTINGS_EXTENSION: &str = "bimport";
pub const FORMAT: &str = "bayleaf.import";
pub const VERSION: u32 = 1;
/// Largest settings file read, in bytes.
const MAX_SETTINGS_BYTES: u64 = 64 << 10;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportSettings {
    pub format: String,
    pub version: u32,
    /// For a model (`.glb`, `.gltf`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelImport>,
    /// For an image (`.png`, `.jpg`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageImport>,
    /// For a sound (`.wav`, `.ogg`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<SoundImport>,
}

/// How a sound is played.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SoundImport {
    /// Read from its file as it plays (long music), instead of decoded whole when first played.
    pub stream: bool,
}

/// File extensions of sounds, as the scene's Audio Source takes them.
pub const SOUND_EXTENSIONS: &[&str] = &["wav", "ogg", "flac", "mp3", "aif", "aiff", "opus", "m4a", "aac"];

/// How a model is read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelImport {
    /// Every position times this: 0.01 for a model made in centimetres.
    pub scale: f32,
    /// Which axis the model's maker used as up; the engine's up is Y.
    pub up_axis: UpAxis,
}

impl Default for ModelImport {
    fn default() -> Self {
        Self { scale: 1.0, up_axis: UpAxis::Y }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpAxis {
    #[default]
    Y,
    Z,
}

/// How an image is read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageImport {
    /// Smaller copies for far away, so it does not shimmer; off keeps every pixel sharp (pixel art).
    pub mipmaps: bool,
    /// Whether the pixels are colours (stored as sRGB) or plain numbers (a normal map, a mask).
    #[serde(default)]
    pub color_space: ColorSpace,
    /// How it is kept on the GPU: as it is, or in 4 x 4 blocks that take an eighth or a quarter
    /// of the memory and lose a little detail.
    #[serde(default)]
    pub compression: Compression,
}

impl Default for ImageImport {
    fn default() -> Self {
        Self { mipmaps: true, color_space: ColorSpace::Color, compression: Compression::None }
    }
}

/// What an image's pixels are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorSpace {
    /// Colours, stored as sRGB: shown as they look.
    #[default]
    Color,
    /// Plain numbers (normals, roughness, masks): read as stored.
    Data,
}

/// How an image is kept on the GPU.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Compression {
    /// Every pixel as it is: 4 bytes each.
    #[default]
    None,
    /// BC1: half a byte a pixel; transparency only on or off.
    Bc1,
    /// BC3: a byte a pixel, with smooth transparency.
    Bc3,
}

/// What kind of file settings are kept for, by its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Imported {
    Model,
    Image,
    Sound,
}

impl Imported {
    pub fn of(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            e if crate::MODEL_EXTENSIONS.contains(&e) => Some(Imported::Model),
            e if crate::IMAGE_EXTENSIONS.contains(&e) => Some(Imported::Image),
            e if SOUND_EXTENSIONS.contains(&e) => Some(Imported::Sound),
            _ => None,
        }
    }
}

/// The settings file beside `file`.
pub fn settings_path(file: &Path) -> PathBuf {
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(SETTINGS_EXTENSION);
    file.with_file_name(name)
}

/// Whether `path` is a settings file.
pub fn is_settings_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case(SETTINGS_EXTENSION))
}

impl ImportSettings {
    /// The defaults for `file`, or `None` for a kind of file without settings.
    pub fn defaults_for(file: &Path) -> Option<Self> {
        let kind = Imported::of(file)?;
        Some(Self {
            format: FORMAT.to_owned(),
            version: VERSION,
            model: (kind == Imported::Model).then(ModelImport::default),
            image: (kind == Imported::Image).then(ImageImport::default),
            sound: (kind == Imported::Sound).then(SoundImport::default),
        })
    }

    /// Checks the format and every value.
    pub fn validate(&self) -> Result<(), String> {
        if self.format != FORMAT {
            return Err(format!("not import settings: the format is {:?}, not {FORMAT:?}", self.format));
        }
        if self.version > VERSION {
            return Err(format!("import settings version {} is newer than this engine reads ({VERSION})", self.version));
        }
        if let Some(model) = &self.model
            && !(model.scale.is_finite() && (0.0001..=10_000.0).contains(&model.scale))
        {
            return Err("a model's scale must be between 0.0001 and 10000".to_owned());
        }
        Ok(())
    }

    pub fn from_text(text: &str) -> Result<Self, String> {
        let settings: Self = serde_json::from_str(text).map_err(|e| format!("broken import settings: {e}"))?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn to_text(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).unwrap_or_default();
        text.push('\n');
        text
    }

    /// The settings saved beside `file`, or its defaults when there are none. An error when the
    /// settings file is there but broken, so a mistake there is not silently ignored.
    pub fn read_for(file: &Path) -> Result<Option<Self>, String> {
        let path = settings_path(file);
        let Ok(size) = bayleaf_core::files::len(&path) else { return Ok(Self::defaults_for(file)) };
        if size > MAX_SETTINGS_BYTES {
            return Err(format!("{} is too large for import settings", path.display()));
        }
        let text = bayleaf_core::files::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Self::from_text(&text).map(Some).map_err(|e| format!("{}: {e}", path.display()))
    }
}
