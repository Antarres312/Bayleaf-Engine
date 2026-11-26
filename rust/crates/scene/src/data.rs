//! Data files (`.bdata`) and their shapes (`.bshape`): plain data that scripts and scenes share
//! instead of copying the same values into every object, such as an enemy type's settings, a
//! weapon's numbers or a level's list of waves.
//!
//! A shape declares fields, each with a name, a type (the script property types, lists and maps of
//! them, and paths of prefabs, sounds, materials and other data files) and a default, plus how the
//! Inspector shows it (label, hint, range, slider, group). A data file names its shape and keeps
//! the values that differ from the defaults; a field it leaves out reads as the default, and a
//! value that no longer fits its field (the shape changed) reads as the default too, and is
//! reported. Scripts read data files as they were saved; play mode never writes them.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::SceneError;
use crate::components::{DeclaredProperty, PropertyShape, PropertyType, PropertyValue};
use crate::inspector::{InspectorItem, PropertyLayout};

/// File extension of data shapes.
pub const SHAPE_EXTENSION: &str = "bshape";
/// File extension of data files.
pub const DATA_EXTENSION: &str = "bdata";
/// `format` of shape files.
pub const SHAPE_FORMAT: &str = "bayleaf.shape";
/// `format` of data files.
pub const DATA_FORMAT: &str = "bayleaf.data";
/// Current version of both file kinds.
pub const VERSION: u32 = 1;
/// Most fields a shape declares.
pub const MAX_FIELDS: usize = 256;
/// Largest shape or data file read, in bytes.
pub const MAX_FILE_BYTES: u64 = 4 << 20;
/// Longest field name, label or group title, in bytes.
pub const MAX_NAME: usize = 64;
/// Longest hint, in bytes.
pub const MAX_HINT: usize = 512;

/// One field of a shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataField {
    /// 1 to 64 bytes, no `:`; unique in the shape.
    pub name: String,
    /// The type as scripts declare property types: `number`, `whole`, `bool`, `vector`, `color`,
    /// `text`, `object`, `prefab`, `sound`, `material`, `data`, or a list or map of one
    /// (`list:number`, `map:text:whole`).
    #[serde(rename = "type")]
    pub code: String,
    /// The value a data file reads when it does not set this field.
    pub default: PropertyValue,
    /// Shown instead of the name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Said on hover and to screen readers.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hint: String,
    /// The lowest and highest value a number may be set to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<[f64; 2]>,
    /// Shown as a slider across `range`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub slider: bool,
    /// The group the field shows in; fields of one group show together, in the order of the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

impl DataField {
    /// A field of this type and shape with the type's zero as its default.
    pub fn new(name: &str, ty: PropertyType, shape: PropertyShape) -> Self {
        Self {
            name: name.to_owned(),
            code: type_code(ty, shape),
            default: ty.zero_of(shape),
            label: None,
            hint: String::new(),
            range: None,
            slider: false,
            group: None,
        }
    }

    /// The field's type and shape; `None` for a code that names none.
    pub fn kind(&self) -> Option<(PropertyType, PropertyShape)> {
        PropertyShape::parse(&self.code)
    }

    /// The field as a declared property: its name, type, shape and default.
    pub fn declared(&self) -> Option<DeclaredProperty> {
        let (ty, shape) = self.kind()?;
        Some(DeclaredProperty { name: self.name.clone(), ty, shape, default: self.default.clone() })
    }

    fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() || self.name.len() > MAX_NAME || self.name.contains(':') || self.name.chars().any(char::is_control) {
            return Err(format!("field name {:?} must be 1 to {MAX_NAME} bytes without : or control characters", self.name));
        }
        let Some((ty, shape)) = self.kind() else {
            return Err(format!("field {} has an unknown type {:?}", self.name, self.code));
        };
        if !self.default.fits_shape(ty, shape) {
            return Err(format!("field {}'s default does not fit its type {}", self.name, self.code));
        }
        let short = |what: &str, text: &str, max: usize| {
            if text.len() <= max && !text.chars().any(char::is_control) {
                Ok(())
            } else {
                Err(format!("field {}'s {what} is longer than {max} bytes", self.name))
            }
        };
        short("label", self.label.as_deref().unwrap_or(""), MAX_NAME)?;
        short("hint", &self.hint, MAX_HINT)?;
        short("group", self.group.as_deref().unwrap_or(""), MAX_NAME)?;
        if let Some([low, high]) = self.range {
            if !matches!(ty, PropertyType::Number | PropertyType::WholeNumber) || shape != PropertyShape::Single {
                return Err(format!("field {} has a range, but only a number or whole number takes one", self.name));
            }
            if !(low.is_finite() && high.is_finite() && low < high) {
                return Err(format!("field {}'s range must be two finite numbers, the lower first", self.name));
            }
        }
        if self.slider && self.range.is_none() {
            return Err(format!("field {} is a slider without a range", self.name));
        }
        Ok(())
    }
}

/// The code a type and shape are declared with (`number`, `list:number`, `map:whole:number`).
pub fn type_code(ty: PropertyType, shape: PropertyShape) -> String {
    match shape {
        PropertyShape::Single => ty.code().to_owned(),
        PropertyShape::List => format!("list:{}", ty.code()),
        PropertyShape::Map(PropertyType::Text) => format!("map:{}", ty.code()),
        PropertyShape::Map(key) => format!("map:{}:{}", key.code(), ty.code()),
    }
}

/// The fields a data file has: what the Inspector shows and what scripts read.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataShape {
    #[serde(default)]
    pub fields: Vec<DataField>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShapeFile {
    format: String,
    version: u32,
    #[serde(default)]
    fields: Vec<DataField>,
}

impl DataShape {
    /// The shape a new shape file starts with: one number field.
    pub fn starter() -> Self {
        Self { fields: vec![DataField::new("value", PropertyType::Number, PropertyShape::Single)] }
    }

    /// Checks every field and that names are unique.
    pub fn validate(&self) -> Result<(), String> {
        if self.fields.len() > MAX_FIELDS {
            return Err(format!("a shape has at most {MAX_FIELDS} fields"));
        }
        for (i, field) in self.fields.iter().enumerate() {
            field.validate()?;
            if self.fields[..i].iter().any(|f| f.name == field.name) {
                return Err(format!("two fields are named {}", field.name));
            }
        }
        Ok(())
    }

    pub fn field(&self, name: &str) -> Option<&DataField> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Every field as a declared property, in order.
    pub fn declared(&self) -> Vec<DeclaredProperty> {
        self.fields.iter().filter_map(DataField::declared).collect()
    }

    /// How the Inspector lays the fields out: one row each, those of a group together in a group
    /// placed where its first field is.
    pub fn layout(&self) -> Vec<InspectorItem> {
        let row = |f: &DataField| {
            InspectorItem::Property(PropertyLayout {
                name: f.name.clone(),
                label: f.label.clone(),
                hint: f.hint.clone(),
                range: f.range.map(|[low, high]| (low, high)),
                slider: f.slider,
                ..PropertyLayout::default()
            })
        };
        let mut items: Vec<InspectorItem> = Vec::new();
        let mut placed: Vec<&str> = Vec::new();
        for field in &self.fields {
            match field.group.as_deref().filter(|g| !g.is_empty()) {
                None => items.push(row(field)),
                Some(group) if placed.contains(&group) => {}
                Some(group) => {
                    placed.push(group);
                    let members = self.fields.iter().filter(|f| f.group.as_deref() == Some(group)).map(row).collect();
                    items.push(InspectorItem::Group { title: group.to_owned(), closed: false, items: members, show_if: None });
                }
            }
        }
        items
    }

    pub fn to_text(&self) -> String {
        let file = ShapeFile { format: SHAPE_FORMAT.to_owned(), version: VERSION, fields: self.fields.clone() };
        let mut text = serde_json::to_string_pretty(&file).unwrap_or_default();
        text.push('\n');
        text
    }

    pub fn from_text(text: &str) -> Result<Self, SceneError> {
        let file: ShapeFile = serde_json::from_str(text).map_err(|e| SceneError::Format(e.to_string()))?;
        if file.format != SHAPE_FORMAT {
            return Err(SceneError::Format(format!("not a data shape (format \"{}\")", file.format)));
        }
        if !(1..=VERSION).contains(&file.version) {
            return Err(SceneError::Format(format!("unsupported data shape version {}", file.version)));
        }
        let shape = Self { fields: file.fields };
        shape.validate().map_err(SceneError::Format)?;
        Ok(shape)
    }

    pub fn load(path: &Path) -> Result<Self, SceneError> {
        Self::from_text(&read_limited(path)?)
    }

    /// Writes the file through a temporary one, so a failed write keeps the old file.
    pub fn save(&self, path: &Path) -> Result<(), SceneError> {
        self.validate().map_err(SceneError::Format)?;
        write_replacing(path, &self.to_text())
    }
}

/// A data file: its shape and the values it sets.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataFile {
    /// The shape file, by its path in the project; empty until one is chosen.
    #[serde(default)]
    pub shape: String,
    /// The values that differ from the shape's defaults, by field name.
    #[serde(default)]
    pub values: BTreeMap<String, PropertyValue>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DataText {
    format: String,
    version: u32,
    shape: String,
    #[serde(default)]
    values: BTreeMap<String, PropertyValue>,
}

/// One field of a data file as it reads: its type, shape and value.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedField {
    pub ty: PropertyType,
    pub shape: PropertyShape,
    pub value: PropertyValue,
}

impl DataFile {
    pub fn new(shape: &str) -> Self {
        Self { shape: shape.to_owned(), values: BTreeMap::new() }
    }

    /// Checks the shape's path and the values' names and sizes (not that they fit the shape, which
    /// may change after the file is written: see [`DataFile::problems`]).
    pub fn validate(&self) -> Result<(), String> {
        if !self.shape.is_empty() && !(self.shape.len() <= 512 && self.shape.to_ascii_lowercase().ends_with(&format!(".{SHAPE_EXTENSION}"))) {
            return Err(format!("shape must be the path of a .{SHAPE_EXTENSION} file (at most 512 characters)"));
        }
        if self.values.len() > MAX_FIELDS {
            return Err(format!("a data file sets at most {MAX_FIELDS} values"));
        }
        for (name, value) in &self.values {
            if name.is_empty() || name.len() > MAX_NAME || name.contains(':') {
                return Err(format!("value name {name:?} must be 1 to {MAX_NAME} bytes without :"));
            }
            let fits_any = PropertyShape::all().any(|shape| PropertyType::ALL.iter().any(|ty| value.fits_shape(*ty, shape)));
            if !fits_any {
                return Err(format!("value {name} is not a value any field holds"));
            }
        }
        Ok(())
    }

    /// Every field of `shape` as this file reads: its own value where it sets one that fits, else
    /// the default.
    pub fn resolved(&self, shape: &DataShape) -> BTreeMap<String, ResolvedField> {
        shape
            .fields
            .iter()
            .filter_map(|f| {
                let (ty, kind) = f.kind()?;
                let value = self.values.get(&f.name).filter(|v| v.fits_shape(ty, kind)).unwrap_or(&f.default).clone();
                Some((f.name.clone(), ResolvedField { ty, shape: kind, value }))
            })
            .collect()
    }

    /// What does not read as written against `shape`: values of fields the shape lacks, and values
    /// that no longer fit their field (those read as the default).
    pub fn problems(&self, shape: &DataShape) -> Vec<String> {
        let mut out = Vec::new();
        for (name, value) in &self.values {
            match shape.field(name).and_then(|f| Some((f, f.kind()?))) {
                None => out.push(format!("{name} is not a field of the shape; it is kept but not read")),
                Some((f, (ty, kind))) if !value.fits_shape(ty, kind) => {
                    out.push(format!("{name} does not fit its field's type {}; it reads as the default", f.code));
                }
                Some(_) => {}
            }
        }
        out
    }

    pub fn to_text(&self) -> String {
        let file = DataText { format: DATA_FORMAT.to_owned(), version: VERSION, shape: self.shape.clone(), values: self.values.clone() };
        let mut text = serde_json::to_string_pretty(&file).unwrap_or_default();
        text.push('\n');
        text
    }

    pub fn from_text(text: &str) -> Result<Self, SceneError> {
        let file: DataText = serde_json::from_str(text).map_err(|e| SceneError::Format(e.to_string()))?;
        if file.format != DATA_FORMAT {
            return Err(SceneError::Format(format!("not a data file (format \"{}\")", file.format)));
        }
        if !(1..=VERSION).contains(&file.version) {
            return Err(SceneError::Format(format!("unsupported data file version {}", file.version)));
        }
        let data = Self { shape: file.shape, values: file.values };
        data.validate().map_err(SceneError::Format)?;
        Ok(data)
    }

    pub fn load(path: &Path) -> Result<Self, SceneError> {
        Self::from_text(&read_limited(path)?)
    }

    /// Writes the file through a temporary one, so a failed write keeps the old file.
    pub fn save(&self, path: &Path) -> Result<(), SceneError> {
        self.validate().map_err(SceneError::Format)?;
        write_replacing(path, &self.to_text())
    }
}

fn read_limited(path: &Path) -> Result<String, SceneError> {
    let len = bayleaf_core::files::len(path).map_err(|e| SceneError::Io(format!("cannot read {}: {e}", path.display())))?;
    if len > MAX_FILE_BYTES {
        return Err(SceneError::Format(format!("{} is larger than {} MiB", path.display(), MAX_FILE_BYTES >> 20)));
    }
    bayleaf_core::files::read_to_string(path).map_err(|e| SceneError::Io(format!("cannot read {}: {e}", path.display())))
}

fn write_replacing(path: &Path, text: &str) -> Result<(), SceneError> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, text).map_err(|e| SceneError::Io(format!("cannot write {}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, path).map_err(|e| SceneError::Io(format!("cannot replace {}: {e}", path.display())))
}
