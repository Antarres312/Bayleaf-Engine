//! The game's files. Everything a running game reads or writes goes through here: on a desktop
//! system that is the file system, and in a browser, which has none, a store in memory that the web
//! player fills from the game's pack before the game starts. Editor-only code (importing,
//! exporting, building scripts) may use `std::fs` directly.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::{Path, PathBuf, SystemTime, io};

    pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        let path = path.as_ref();
        std::fs::read(path)
    }

    pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
        std::fs::write(path, bytes)
    }

    pub fn len(path: impl AsRef<Path>) -> io::Result<u64> {
        let path = path.as_ref();
        std::fs::metadata(path).map(|m| m.len())
    }

    pub fn modified(path: impl AsRef<Path>) -> Option<SystemTime> {
        let path = path.as_ref();
        std::fs::metadata(path).and_then(|m| m.modified()).ok()
    }

    pub fn is_file(path: impl AsRef<Path>) -> bool {
        let path = path.as_ref();
        path.is_file()
    }

    pub fn is_dir(path: impl AsRef<Path>) -> bool {
        let path = path.as_ref();
        path.is_dir()
    }

    pub fn read_dir(path: impl AsRef<Path>) -> io::Result<Vec<PathBuf>> {
        let path = path.as_ref();
        std::fs::read_dir(path)?.map(|entry| entry.map(|e| e.path())).collect()
    }

    pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        std::fs::create_dir_all(path)
    }

    pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        std::fs::remove_file(path)
    }
}

/// The browser's store: what the game's pack holds, and what the game wrote since.
#[cfg(any(target_arch = "wasm32", test))]
mod memory {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::{Path, PathBuf, io, plain};

    #[derive(Default)]
    pub struct Store {
        files: BTreeMap<PathBuf, Arc<[u8]>>,
        /// Files written since [`Store::take_written`] was last asked.
        written: Vec<PathBuf>,
    }

    fn missing(path: &Path) -> io::Error {
        io::Error::new(io::ErrorKind::NotFound, format!("the game has no file {}", path.display()))
    }

    impl Store {
        pub const fn new() -> Self {
            Self { files: BTreeMap::new(), written: Vec::new() }
        }

        pub fn mount(&mut self, path: &Path, bytes: Vec<u8>) {
            self.files.insert(plain(path), bytes.into());
        }

        pub fn take_written(&mut self) -> Vec<PathBuf> {
            std::mem::take(&mut self.written)
        }

        pub fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            self.files.get(&plain(path)).map(|b| b.to_vec()).ok_or_else(|| missing(path))
        }

        pub fn write(&mut self, path: &Path, bytes: &[u8]) {
            let path = plain(path);
            self.files.insert(path.clone(), bytes.into());
            if !self.written.contains(&path) {
                self.written.push(path);
            }
        }

        pub fn len(&self, path: &Path) -> io::Result<u64> {
            self.files.get(&plain(path)).map(|b| b.len() as u64).ok_or_else(|| missing(path))
        }

        pub fn is_file(&self, path: &Path) -> bool {
            self.files.contains_key(&plain(path))
        }

        /// A folder is there when a file is below it.
        pub fn is_dir(&self, path: &Path) -> bool {
            let folder = plain(path);
            self.files.keys().any(|f| f != &folder && f.starts_with(&folder))
        }

        pub fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
            let folder = plain(path);
            let mut entries: Vec<PathBuf> = self
                .files
                .keys()
                .filter(|f| *f != &folder)
                .filter_map(|f| f.strip_prefix(&folder).ok().and_then(|rest| rest.components().next()).map(|first| folder.join(first)))
                .collect();
            entries.dedup();
            if entries.is_empty() { Err(missing(path)) } else { Ok(entries) }
        }

        pub fn remove_file(&mut self, path: &Path) -> io::Result<()> {
            self.files.remove(&plain(path)).map(|_| ()).ok_or_else(|| missing(path))
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::sync::{Mutex, MutexGuard, PoisonError};

    use super::memory::Store;
    use super::{Path, PathBuf, SystemTime, io};

    static STORE: Mutex<Store> = Mutex::new(Store::new());

    fn store() -> MutexGuard<'static, Store> {
        STORE.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Puts a file in the store, as the game's pack or the browser's saved data has it.
    pub fn mount(path: &Path, bytes: Vec<u8>) {
        store().mount(path, bytes);
    }

    /// The files written since this was last asked, so the player can keep them (saved values).
    pub fn take_written() -> Vec<PathBuf> {
        store().take_written()
    }

    pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        let path = path.as_ref();
        store().read(path)
    }

    pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
        store().write(path, bytes);
        Ok(())
    }

    pub fn len(path: impl AsRef<Path>) -> io::Result<u64> {
        let path = path.as_ref();
        store().len(path)
    }

    /// The pack's files never change while the game runs.
    pub fn modified(_path: &Path) -> Option<SystemTime> {
        None
    }

    pub fn is_file(path: impl AsRef<Path>) -> bool {
        let path = path.as_ref();
        store().is_file(path)
    }

    pub fn is_dir(path: impl AsRef<Path>) -> bool {
        let path = path.as_ref();
        store().is_dir(path)
    }

    pub fn read_dir(path: impl AsRef<Path>) -> io::Result<Vec<PathBuf>> {
        let path = path.as_ref();
        store().read_dir(path)
    }

    /// Folders need no making in the store.
    pub fn create_dir_all(_path: &Path) -> io::Result<()> {
        Ok(())
    }

    pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        store().remove_file(path)
    }
}

#[cfg(target_arch = "wasm32")]
pub use imp::{mount, take_written};

/// The whole file.
pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let path = path.as_ref();
    imp::read(path)
}

/// The whole file as UTF-8 text.
pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    let path = path.as_ref();
    String::from_utf8(imp::read(path)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Writes the whole file, replacing what it held.
pub fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    let path = path.as_ref();
    imp::write(path, bytes.as_ref())
}

/// The file's size in bytes.
pub fn len(path: impl AsRef<Path>) -> io::Result<u64> {
    let path = path.as_ref();
    imp::len(path)
}

/// When the file last changed, where the system says (never in a browser, where files stay as packed).
pub fn modified(path: impl AsRef<Path>) -> Option<SystemTime> {
    let path = path.as_ref();
    imp::modified(path)
}

pub fn is_file(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    imp::is_file(path)
}

pub fn is_dir(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    imp::is_dir(path)
}

/// The files and folders directly in a folder.
pub fn read_dir(path: impl AsRef<Path>) -> io::Result<Vec<PathBuf>> {
    let path = path.as_ref();
    imp::read_dir(path)
}

pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    imp::create_dir_all(path)
}

pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    imp::remove_file(path)
}

/// `path` with `.` and `..` worked out without asking the file system, so the same file has one name.
pub fn plain(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::memory::Store;
    use std::path::{Path, PathBuf};

    #[test]
    fn the_browser_store_finds_a_file_by_any_spelling_of_its_path_and_lists_its_folders() {
        let mut store = Store::new();
        store.mount(Path::new("/game/scenes/main.bscene"), b"{}".to_vec());
        store.mount(Path::new("/game/scenes/deep/other.bscene"), Vec::new());
        store.mount(Path::new("/game/game.bproject"), Vec::new());
        assert_eq!(store.read(Path::new("/game/./materials/../scenes/main.bscene")).unwrap(), b"{}");
        assert!(store.is_dir(Path::new("/game/scenes")) && !store.is_file(Path::new("/game/scenes")));
        assert!(!store.is_dir(Path::new("/game/game.bproject")), "a file is not a folder");
        let mut listed = store.read_dir(Path::new("/game")).unwrap();
        listed.sort();
        assert_eq!(listed, [PathBuf::from("/game/game.bproject"), PathBuf::from("/game/scenes")], "each entry once, folders by their own name");
        assert!(store.read(Path::new("/game/missing")).is_err());
    }

    #[test]
    fn writes_replace_the_file_are_reported_once_and_removing_forgets_it() {
        let mut store = Store::new();
        let saves = Path::new("/game/saves/progress.json");
        store.write(saves, b"1");
        store.write(saves, b"2");
        assert_eq!(store.read(saves).unwrap(), b"2");
        assert_eq!(store.take_written(), [saves.to_path_buf()]);
        assert!(store.take_written().is_empty(), "asked once, then forgotten");
        assert_eq!(store.len(saves).unwrap(), 1);
        store.remove_file(saves).unwrap();
        assert!(store.read(saves).is_err() && store.remove_file(saves).is_err(), "a removed file is gone");
    }
}
