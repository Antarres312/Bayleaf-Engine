//! A game exported for the web: a folder a web server (or a game site) serves as it is. It holds
//! `index.html`, the web player (its WebAssembly module and the script the browser loads it with,
//! both written by `cargo xtask web-player`) and `game.pack`: every file of the game in one file,
//! which the player reads into memory before the game starts, since a browser has no file system.

use std::path::{Path, PathBuf};

pub use crate::pack::PACK_FILE;
/// The web player's files: what the browser loads, and the player itself.
pub const PLAYER_SCRIPT: &str = "bayleaf_player.js";
pub const PLAYER_MODULE: &str = "bayleaf_player_bg.wasm";
/// The folder the web player is built into (`cargo xtask web-player`), in the build's `target`
/// folder, which also holds the editor's program folders (`debug`, `release`).
pub const PLAYER_FOLDER: &str = "web-player";

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
    use super::index_html;

    #[test]
    fn the_page_cannot_be_broken_by_the_games_name() {
        let page = index_html("<script>alert(1)</script> & \"Co\"");
        assert!(!page.contains("<script>alert"), "the name is escaped");
        assert!(page.contains("&lt;script&gt;"));
    }
}
