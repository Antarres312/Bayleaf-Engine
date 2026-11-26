//! A game's files in one file (`game.pack`): what an exported game ships instead of its project's
//! files, read into memory when it starts (`bayleaf_core::files`), so the files are not left in its
//! folder to read or change one by one. It is not encryption: the pack's format is plain.

use std::path::Path;

/// The pack's file name, beside the game's program or `index.html`.
pub const PACK_FILE: &str = "game.pack";

/// The first bytes of a pack, with its version.
const MAGIC: &[u8; 8] = b"BLPACK1\n";
/// Most files and the longest path a pack holds; a pack is read whole, so its size is the browser's limit.
pub const MAX_FILES: usize = 100_000;
const MAX_PATH: usize = 1024;

/// Writes the files (paths relative to the game, with `/`) into one pack.
pub fn pack(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let size = files.iter().map(|(p, b)| 12 + p.len() + b.len()).sum::<usize>();
    let mut out = Vec::with_capacity(MAGIC.len() + 4 + size);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for (path, bytes) in files {
        out.extend_from_slice(&(path.len() as u32).to_le_bytes());
        out.extend_from_slice(path.as_bytes());
        out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        out.extend_from_slice(bytes);
    }
    out
}

/// The files in a pack. Refuses a pack that is cut short, too large, or names a path outside the
/// game (absolute, or with `..`).
pub fn unpack(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut rest = bytes.strip_prefix(MAGIC.as_slice()).ok_or("this is not a Bayleaf game pack (or one from another version)")?;
    let mut take = |n: usize| -> Result<&[u8], String> {
        if rest.len() < n {
            return Err("the game pack is cut short".to_owned());
        }
        let (head, tail) = rest.split_at(n);
        rest = tail;
        Ok(head)
    };
    let number = |b: &[u8]| b.iter().rev().fold(0u64, |n, byte| (n << 8) | u64::from(*byte));
    let count = number(take(4)?) as usize;
    if count > MAX_FILES {
        return Err(format!("the game pack holds more than {MAX_FILES} files"));
    }
    let mut files = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        let length = number(take(4)?) as usize;
        if length == 0 || length > MAX_PATH {
            return Err("the game pack names a file with an empty or too long path".to_owned());
        }
        let path = std::str::from_utf8(take(length)?).map_err(|_| "the game pack names a file that is not UTF-8")?.to_owned();
        if path.starts_with('/') || path.contains('\\') || path.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
            return Err(format!("the game pack names {path:?}, which is not a path inside the game"));
        }
        let size = usize::try_from(number(take(8)?)).map_err(|_| "the game pack holds a file too large for this browser")?;
        files.push((path, take(size)?.to_vec()));
    }
    Ok(files)
}

/// Puts the pack's files in the game's file store under `root`, where the game reads them;
/// returns how many.
pub fn mount(root: &Path, bytes: &[u8]) -> Result<usize, String> {
    let files = unpack(bytes)?;
    let count = files.len();
    for (path, bytes) in files {
        bayleaf_core::files::mount(&path.split('/').fold(root.to_path_buf(), |at, part| at.join(part)), bytes);
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::{pack, unpack};

    #[test]
    fn a_pack_gives_back_the_files_it_was_written_with() {
        let files =
            vec![("game.bproject".to_owned(), b"{}".to_vec()), ("scenes/main.bscene".to_owned(), vec![0, 255, 7]), ("empty.bin".to_owned(), Vec::new())];
        assert_eq!(unpack(&pack(&files)).unwrap(), files);
    }

    #[test]
    fn a_hostile_pack_is_refused_without_a_panic() {
        let good = pack(&[("a.bscene".to_owned(), vec![1, 2, 3])]);
        for cut in 0..good.len() {
            assert!(unpack(&good[..cut]).is_err(), "cut at {cut}");
        }
        for path in ["/etc/passwd", "../outside", "a/../../b", "a//b", "a\\b", ""] {
            assert!(unpack(&pack(&[(path.to_owned(), Vec::new())])).is_err(), "{path:?} is outside the game");
        }
        let mut huge = good.clone();
        huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(unpack(&huge).is_err(), "a count past the limit");
        let mut long = good;
        let at = 8 + 4 + 4 + 8;
        long[at..at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(unpack(&long).is_err(), "a size past the end");
    }
}
