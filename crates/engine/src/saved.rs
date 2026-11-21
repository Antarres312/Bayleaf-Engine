//! Saved values: numbers a game keeps from one play to the next, such as a best score. They live in
//! the project, in `saves/progress.json`; outside a project they last for one play.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{Session, confine::Access};

/// Where saved values live, relative to the project folder.
pub const SAVED_FILE: &str = "saves/progress.json";
const FORMAT: &str = "bayleaf.saved";
const VERSION: u32 = 1;
/// The largest saved-values file read.
const MAX_FILE_BYTES: u64 = 1 << 20;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedFile {
    format: String,
    version: u32,
    values: BTreeMap<String, f64>,
}

/// The saved-values file of the session's project, if it has one.
pub fn saved_path(session: &Session) -> Option<PathBuf> {
    session.project_dir().map(|dir| dir.join(SAVED_FILE))
}

/// The values saved in the project: empty when there is no project or nothing saved yet.
pub fn read_saved(session: &Session) -> Result<BTreeMap<String, f64>, String> {
    let Some(path) = saved_path(session) else { return Ok(BTreeMap::new()) };
    let path = session.allowed(&path, Access::Read).map_err(|e| e.to_string())?;
    let size = match std::fs::metadata(&path) {
        Ok(meta) => meta.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    if size > MAX_FILE_BYTES {
        return Err(format!("{} is larger than {} KiB", path.display(), MAX_FILE_BYTES / 1024));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file: SavedFile = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    if file.format != FORMAT || file.version != VERSION {
        return Err(format!("{} is not a version {VERSION} saved-values file", path.display()));
    }
    if file.values.len() > bayleaf_script::MAX_STATE_VALUES {
        return Err(format!("{} holds more than {} values", path.display(), bayleaf_script::MAX_STATE_VALUES));
    }
    if let Some(name) = file.values.iter().find(|(name, value)| !bayleaf_script::valid_value(name, **value)).map(|(name, _)| name) {
        return Err(format!("{}: {name:?} needs a name of 1 to 64 bytes and a finite number", path.display()));
    }
    Ok(file.values)
}

/// Writes the values to the project; an error when there is no project or it may not be written.
pub fn write_saved(session: &Session, values: &BTreeMap<String, f64>) -> Result<PathBuf, String> {
    let path = saved_path(session).ok_or("saved values are kept only in a project")?;
    let path = session.allowed(&path, Access::Write).map_err(|e| e.to_string())?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    }
    let file = SavedFile { format: FORMAT.to_owned(), version: VERSION, values: values.clone() };
    let text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    std::fs::write(&path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Forgets every saved value of the project. True when there was a file.
pub fn clear_saved(session: &Session) -> Result<bool, String> {
    let Some(path) = saved_path(session) else { return Ok(false) };
    let path = session.allowed(&path, Access::Write).map_err(|e| e.to_string())?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}
