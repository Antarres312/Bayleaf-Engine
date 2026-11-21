//! The properties script modules declare, read for the inspector and cached until the file changes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use bayleaf_engine::script::ScriptHost;

type Declared = Result<Vec<(String, f32)>, String>;

#[derive(Default)]
pub(crate) struct ScriptInfo {
    host: ScriptHost,
    cache: HashMap<PathBuf, (Option<SystemTime>, Declared)>,
}

impl ScriptInfo {
    /// Declared properties of the module at `path`, or a message saying why they cannot be read.
    pub(crate) fn properties(&mut self, path: &Path) -> Declared {
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if let Some((seen, declared)) = self.cache.get(path)
            && *seen == modified
        {
            return declared.clone();
        }
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        // Visual scripts are compiled first, exactly as play mode loads them.
        let declared = if path.exists() {
            bayleaf_engine::script::module_bytes(path).and_then(|bytes| self.host.describe(&name, &bytes)).map_err(|e| e.to_string())
        } else {
            Err(format!("Script file not found: {}", path.display()))
        };
        self.cache.insert(path.to_path_buf(), (modified, declared.clone()));
        declared
    }
}
