//! Data files and their shapes (`crates/scene/src/data.rs`): `data.shape.get`/`data.shape.set` read
//! and write a shape (`.bshape`), `data.get`/`data.set` a data file (`.bdata`), and `data.field.set`
//! one value of a data file. `asset.create` makes new ones (kinds `dataShape`, `dataFile`). Play mode
//! reads every data file of the project as it was saved when play starts and never writes them;
//! see [`load_all`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bayleaf_scene::components::PropertyValue;
use bayleaf_scene::data::{DATA_EXTENSION, DataFile, DataShape, SHAPE_EXTENSION};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::confine::Access;
use crate::{CommandError, CommandSpec, Mutates, Prompt, Session, always, parse};

pub(crate) fn commands() -> Vec<CommandSpec> {
    let spec =
        |id, label, params, run| CommandSpec { id, label, menu: "", shortcut: "", params, prompt: Prompt::None, mutates: Mutates::No, enabled: always, run };
    vec![
        spec(
            "data.shape.get",
            "Read Data Shape",
            "{path}: a data shape (.bshape, relative to the project folder or absolute); returns {shape: {fields: [{name, type, default, label?, hint?, range?, slider?, group?}]}}. type is as scripts declare property types: number, whole, bool, vector, color, text, object, prefab, sound, material, data, or list:<type>, map:<type> (text keys), map:<key type>:<type>",
            shape_get,
        ),
        spec(
            "data.shape.set",
            "Change Data Shape",
            "{path, shape}: writes a data shape (all of it, as data.shape.get returns it); every data file that names it reads its fields from then on: a value that no longer fits reads as the field's default and is reported by data.get",
            shape_set,
        ),
        spec(
            "data.get",
            "Read Data File",
            "{path}: a data file (.bdata); returns {data: {shape, values}, fields: [{name, type, value, set}] (every field of its shape as it reads, set when the file gives the value), problems: [text]}",
            data_get,
        ),
        spec(
            "data.set",
            "Change Data File",
            "{path, data: {shape, values}}: writes a data file (all of it): the shape it names (.bshape, relative to the project folder; empty for none) and the values that differ from the shape's defaults, by field name",
            data_set,
        ),
        spec(
            "data.field.set",
            "Change Data Field",
            "{path, name, value}: sets one field of a data file to a value that fits the field in its shape (as data.get shows values); value null returns it to the shape's default. Refuses names the shape lacks",
            field_set,
        ),
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathParams {
    path: PathBuf,
}

/// A data shape's or data file's path (by `extension`), relative to the project folder or absolute,
/// checked for `access`.
fn path_of(session: &Session, path: &Path, extension: &str, access: Access) -> Result<PathBuf, CommandError> {
    if !path.to_string_lossy().to_ascii_lowercase().ends_with(&format!(".{extension}")) {
        return Err(CommandError::BadParams(format!("give a .{extension} file")));
    }
    session.allowed(&session.script_dir().join(path), access)
}

fn shape_get(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PathParams = parse(params)?;
    let shape = DataShape::load(&path_of(session, &p.path, SHAPE_EXTENSION, Access::Read)?)?;
    Ok(json!({ "shape": shape }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShapeSetParams {
    path: PathBuf,
    shape: DataShape,
}

fn shape_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: ShapeSetParams = parse(params)?;
    p.shape.validate().map_err(CommandError::BadParams)?;
    let path = path_of(session, &p.path, SHAPE_EXTENSION, Access::Write)?;
    p.shape.save(&path)?;
    Ok(json!({ "path": path }))
}

/// The shape a data file names, read as `session` may read it; `Err` says why it cannot be.
pub(crate) fn shape_of(session: &Session, data: &DataFile) -> Result<DataShape, String> {
    if data.shape.is_empty() {
        return Err("no shape is chosen".to_owned());
    }
    let path = path_of(session, Path::new(&data.shape), SHAPE_EXTENSION, Access::Read).map_err(|e| e.to_string())?;
    DataShape::load(&path).map_err(|e| format!("its shape {} does not read: {e}", data.shape))
}

fn data_get(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PathParams = parse(params)?;
    let data = DataFile::load(&path_of(session, &p.path, DATA_EXTENSION, Access::Read)?)?;
    let (fields, problems) = match shape_of(session, &data) {
        Ok(shape) => {
            let read = data.resolved(&shape);
            let fields: Vec<Value> = shape
                .fields
                .iter()
                .filter_map(|f| {
                    let r = read.get(&f.name)?;
                    Some(json!({ "name": f.name, "type": f.code, "value": r.value, "set": data.values.get(&f.name) == Some(&r.value) }))
                })
                .collect();
            (fields, data.problems(&shape))
        }
        Err(why) => (Vec::new(), vec![why]),
    };
    Ok(json!({ "data": data, "fields": fields, "problems": problems }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DataSetParams {
    path: PathBuf,
    data: DataFile,
}

fn data_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: DataSetParams = parse(params)?;
    p.data.validate().map_err(CommandError::BadParams)?;
    let path = path_of(session, &p.path, DATA_EXTENSION, Access::Write)?;
    p.data.save(&path)?;
    Ok(json!({ "path": path }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FieldSetParams {
    path: PathBuf,
    name: String,
    value: Option<PropertyValue>,
}

fn field_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: FieldSetParams = parse(params)?;
    let path = path_of(session, &p.path, DATA_EXTENSION, Access::Write)?;
    let mut data = DataFile::load(&path)?;
    let shape = shape_of(session, &data).map_err(|why| CommandError::Failed(format!("cannot set a field: {why}")))?;
    let field = shape.field(&p.name).ok_or_else(|| CommandError::BadParams(format!("the shape {} has no field {}", data.shape, p.name)))?;
    match p.value {
        None => {
            data.values.remove(&p.name);
        }
        Some(value) => {
            let declared = field.declared().ok_or_else(|| CommandError::Failed(format!("field {} has an unknown type", p.name)))?;
            if !declared.accepts(&value) {
                return Err(CommandError::BadParams(format!("{} holds {}; that value is not one", p.name, field.code)));
            }
            data.values.insert(p.name, value);
        }
    }
    data.save(&path)?;
    Ok(json!({ "path": path }))
}

/// Most data files play mode reads.
const MAX_DATA_FILES: usize = 4096;

/// Every data file of the project as play reads it, by its path relative to the project folder
/// (with `/`): each field of its shape with its value. Files that cannot be read, or whose shape
/// cannot, are left out and said in the problems. Folders the session may not read are skipped.
pub(crate) fn load_all(session: &Session) -> (HashMap<String, bayleaf_script::DataValues>, Vec<String>) {
    let base = session.script_dir();
    let (mut out, mut problems) = (HashMap::new(), Vec::new());
    let mut pending = vec![base.clone()];
    while let Some(dir) = pending.pop() {
        if session.allowed(&dir, Access::Read).is_err() {
            continue;
        }
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "target" {
                continue;
            }
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                pending.push(path);
                continue;
            }
            if !name.to_ascii_lowercase().ends_with(&format!(".{DATA_EXTENSION}")) {
                continue;
            }
            if out.len() >= MAX_DATA_FILES {
                problems.push(format!("Only the first {MAX_DATA_FILES} data files are read."));
                return (out, problems);
            }
            let relative = path.strip_prefix(&base).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            let read = DataFile::load(&path).map_err(|e| e.to_string()).and_then(|data| Ok((shape_of(session, &data)?, data)));
            match read {
                Ok((shape, data)) => {
                    let values = data.resolved(&shape).into_iter().map(|(k, v)| (k, (v.ty, v.shape, v.value))).collect();
                    out.insert(relative, values);
                }
                Err(why) => problems.push(format!("The data file {relative} is not read: {why}.")),
            }
        }
    }
    (out, problems)
}
