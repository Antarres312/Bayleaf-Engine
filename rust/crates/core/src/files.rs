//! The game's files. Everything a running game reads or writes goes through here: on a desktop
//! system that is the file system, and in a browser, which has none, a store in memory that the web
//! player fills from the game's pack before the game starts. Editor-only code (importing,
//! exporting, building scripts) may use `std::fs` directly.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// A desktop system: the file system, under a game's pack when the player mounted one (an exported
/// game's files, read from memory). Writes (saved values) go to the disk.
#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use std::sync::{PoisonError, RwLock, RwLockReadGuard};

    use super::memory::Store;
    use super::{Path, PathBuf, SystemTime, io};

    static PACK: RwLock<Store> = RwLock::new(Store::new());

    fn pack() -> RwLockReadGuard<'static, Store> {
        PACK.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Puts a file of a game's pack in memory, where reads find it before the disk.
    pub fn mount(path: &Path, bytes: Vec<u8>) {
        PACK.write().unwrap_or_else(PoisonError::into_inner).mount(path, bytes);
    }

    pub fn read(path: &Path) -> io::Result<Vec<u8>> {
        pack().read(path).or_else(|_| std::fs::read(path))
    }

    pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
        std::fs::write(path, bytes)
    }

    pub fn len(path: &Path) -> io::Result<u64> {
        pack().len(path).or_else(|_| std::fs::metadata(path).map(|m| m.len()))
    }

    /// A packed file never changes while the game runs.
    pub fn modified(path: &Path) -> Option<SystemTime> {
        if pack().is_file(path) {
            return None;
        }
        std::fs::metadata(path).and_then(|m| m.modified()).ok()
    }

    pub fn is_file(path: &Path) -> bool {
        pack().is_file(path) || path.is_file()
    }

    pub fn is_dir(path: &Path) -> bool {
        pack().is_dir(path) || path.is_dir()
    }

    pub fn read_dir(path: &Path) -> io::Result<Vec<PathBuf>> {
        let packed = pack().read_dir(path);
        let disk: io::Result<Vec<PathBuf>> = std::fs::read_dir(path).and_then(|d| d.map(|entry| entry.map(|e| e.path())).collect());
        match (packed, disk) {
            (Ok(mut packed), Ok(disk)) => {
                packed.extend(disk.into_iter().filter(|p| !pack().is_file(p) && !pack().is_dir(p)));
                Ok(packed)
            }
            (Ok(packed), Err(_)) => Ok(packed),
            (Err(_), disk) => disk,
        }
    }

    pub fn create_dir_all(path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    pub fn remove_file(path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }
}

/// A store in memory: what a game's pack holds, and (in a browser) what the game wrote since.
mod memory {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::{Path, PathBuf, io, plain};

    #[derive(Default)]
    pub struct Store {
        files: BTreeMap<PathBuf, Arc<[u8]>>,
        /// Files written since [`Store::take_written`] was last asked.
        #[cfg(any(target_arch = "wasm32", test))]
        written: Vec<PathBuf>,
    }

    fn missing(path: &Path) -> io::Error {
        io::Error::new(io::ErrorKind::NotFound, format!("the game has no file {}", path.display()))
    }

    impl Store {
        pub const fn new() -> Self {
            Self {
                files: BTreeMap::new(),
                #[cfg(any(target_arch = "wasm32", test))]
                written: Vec::new(),
            }
        }

        pub fn mount(&mut self, path: &Path, bytes: Vec<u8>) {
            self.files.insert(plain(path), bytes.into());
        }

        #[cfg(any(target_arch = "wasm32", test))]
        pub fn take_written(&mut self) -> Vec<PathBuf> {
            std::mem::take(&mut self.written)
        }

        pub fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            self.files.get(&plain(path)).map(|b| b.to_vec()).ok_or_else(|| missing(path))
        }

        #[cfg(any(target_arch = "wasm32", test))]
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

        #[cfg(any(target_arch = "wasm32", test))]
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

pub use imp::mount;
#[cfg(target_arch = "wasm32")]
pub use imp::take_written;

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
    fn on_a_desktop_a_mounted_pack_comes_before_the_disk_and_writes_go_to_the_disk() {
        let dir = std::env::temp_dir().join(format!("bayleaf-files-pack-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("saves")).unwrap();
        std::fs::write(dir.join("saves").join("progress.json"), "on disk").unwrap();
        super::mount(&dir.join("game.bproject"), b"packed".to_vec());
        assert_eq!(super::read(dir.join("game.bproject")).unwrap(), b"packed", "the pack is read from memory");
        assert!(!dir.join("game.bproject").exists() && super::is_file(dir.join("game.bproject")));
        assert_eq!(super::read(dir.join("saves").join("progress.json")).unwrap(), b"on disk", "what the pack lacks comes from the disk");
        let mut listed = super::read_dir(&dir).unwrap();
        listed.sort();
        assert_eq!(listed, [dir.join("game.bproject"), dir.join("saves")], "a folder lists both");
        super::write(dir.join("saves").join("new.json"), "kept").unwrap();
        assert!(dir.join("saves").join("new.json").is_file(), "saved values go to the disk");
        std::fs::remove_dir_all(&dir).ok();
    }

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
