//! What script modules say about themselves (their properties and where Add Component lists
//! them), read for the Inspector and cached until the file changes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use bayleaf_engine::script::{Description, ScriptHost};

type Declared = Result<Vec<bayleaf_engine::scene::components::DeclaredProperty>, String>;

#[derive(Default)]
pub(crate) struct ScriptInfo {
    host: ScriptHost,
    cache: HashMap<PathBuf, (Option<SystemTime>, Result<Description, String>)>,
}

impl ScriptInfo {
    /// The module's description, read again when the file changed.
    fn description(&mut self, path: &Path) -> Result<Description, String> {
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if let Some((seen, described)) = self.cache.get(path)
            && *seen == modified
        {
            return described.clone();
        }
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        // Visual scripts are compiled first, exactly as play mode loads them.
        let described = if path.exists() {
            bayleaf_engine::script::module_bytes(path).and_then(|bytes| self.host.describe_all(&name, &bytes)).map_err(|e| e.to_string())
        } else {
            Err(crate::i18n::tf("Script file not found: {path}", &[("path", &path.display().to_string())]))
        };
        self.cache.insert(path.to_path_buf(), (modified, described.clone()));
        described
    }

    /// Declared properties of the module at `path`, or a message saying why they cannot be read.
    pub(crate) fn properties(&mut self, path: &Path) -> Declared {
        self.description(path).map(|d| d.properties)
    }

    /// Where Add Component lists the module, if it says.
    pub(crate) fn menu(&mut self, path: &Path) -> Option<String> {
        self.description(path).ok().and_then(|d| d.menu)
    }
}
