//! A game exported for the web: a folder a web server (or a game site) serves as it is. It holds
//! `index.html`, the web player (its WebAssembly module and the script the browser loads it with,
//! both written by `cargo xtask web-player`) and `game.pack`: every file of the game in one file,
//! which the player reads into memory before the game starts, since a browser has no file system.

use std::path::{Path, PathBuf};

/// The game's files in one file, beside `index.html`.
pub const PACK_FILE: &str = "game.pack";
/// The web player's files: what the browser loads, and the player itself.
pub const PLAYER_SCRIPT: &str = "bayleaf_player.js";
pub const PLAYER_MODULE: &str = "bayleaf_player_bg.wasm";
/// The folder the web player is built into (`cargo xtask web-player`), in the build's `target`
/// folder, which also holds the editor's program folders (`debug`, `release`).
pub const PLAYER_FOLDER: &str = "web-player";

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

/// `index.html`: the game full window, with what happens while it loads (or why it cannot run)
/// in a status line. The only script is the line that starts the web player; the rest is Rust.
pub fn index_html(name: &str) -> String {
    let name = escape(name);
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{name}</title>\n<style>\n\
         html, body {{ margin: 0; height: 100%; overflow: hidden; background: #000; }}\n\
         #bayleaf-game {{ display: block; width: 100%; height: 100%; }}\n\
         #bayleaf-status {{ position: fixed; inset: 0; margin: 0; display: flex; align-items: center; justify-content: center; \
         padding: 16px; text-align: center; color: #e8e8e8; font: 16px system-ui, sans-serif; pointer-events: none; }}\n\
         #bayleaf-status[hidden] {{ display: none; }}\n\
         </style>\n</head>\n<body>\n<canvas id=\"bayleaf-game\" aria-label=\"{name}\"></canvas>\n\
         <p id=\"bayleaf-status\" role=\"status\">Loading {name}…</p>\n\
         <noscript><p>{name} needs JavaScript to run in a browser.</p></noscript>\n\
         <script type=\"module\">import start from \"./{PLAYER_SCRIPT}\"; start();</script>\n</body>\n</html>\n"
    )
}

/// Text made safe to put in HTML.
fn escape(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '&' => "&amp;".to_owned(),
            '<' => "&lt;".to_owned(),
            '>' => "&gt;".to_owned(),
            '"' => "&quot;".to_owned(),
            '\'' => "&#39;".to_owned(),
            other => other.to_string(),
        })
        .collect()
}

/// The web player's folder: `given`, else the one built beside the editor (`target/web-player`
/// when the editor runs from `target/debug` or `target/release`, or a `web-player` folder beside it).
pub fn player_folder(given: Option<&Path>) -> Option<PathBuf> {
    if let Some(folder) = given {
        return Some(folder.to_path_buf());
    }
    let here = std::env::current_exe().ok()?;
    let dir = here.parent()?;
    [dir.join(PLAYER_FOLDER), dir.parent()?.join(PLAYER_FOLDER)].into_iter().find(|f| has_player(f))
}

/// Copies the web player's folder into `out`: its module, the script that loads it, and the
/// scripts wasm-bindgen keeps for it in `snippets`.
pub fn copy_player(folder: &Path, out: &Path) -> Result<(), String> {
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let from = folder.join(&relative);
        let to = out.join(&relative);
        std::fs::create_dir_all(&to).map_err(|e| format!("cannot make {}: {e}", to.display()))?;
        let entries = std::fs::read_dir(&from).map_err(|e| format!("cannot read {}: {e}", from.display()))?;
        for entry in entries.flatten() {
            let name = relative.join(entry.file_name());
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                pending.push(name);
            } else {
                std::fs::copy(folder.join(&name), out.join(&name)).map_err(|e| format!("cannot write {}: {e}", out.join(&name).display()))?;
            }
        }
    }
    Ok(())
}

/// Whether a folder holds a web player.
pub fn has_player(folder: &Path) -> bool {
    folder.join(PLAYER_SCRIPT).is_file() && folder.join(PLAYER_MODULE).is_file()
}

#[cfg(test)]
mod tests {
    use super::{index_html, pack, unpack};

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

    #[test]
    fn the_page_cannot_be_broken_by_the_games_name() {
        let page = index_html("<script>alert(1)</script> & \"Co\"");
        assert!(!page.contains("<script>alert"), "the name is escaped");
        assert!(page.contains("&lt;script&gt;"));
    }
}
