//! `cargo xtask web-player`: builds the web player (bayleaf-player for `wasm32-unknown-unknown`,
//! then wasm-bindgen for the script that loads it) into `target/web-player`, where Export Game
//! finds it for the web. `cargo xtask serve <folder> [port]`: serves a folder (a game exported for
//! the web) on this computer only, to try it in a browser, which does not run it from a file.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, bail};

const TARGET: &str = "wasm32-unknown-unknown";
/// What Export Game looks for (`bayleaf_engine::web_export::PLAYER_FOLDER`).
const OUT_FOLDER: &str = "web-player";

/// The wasm-bindgen version the workspace uses, from Cargo.lock.
fn bindgen_version(root: &Path) -> anyhow::Result<String> {
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).context("reading Cargo.lock")?;
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line == "name = \"wasm-bindgen\""
            && let Some(version) = lines.next().and_then(|l| l.strip_prefix("version = \"")).and_then(|v| v.strip_suffix('"'))
        {
            return Ok(version.to_owned());
        }
    }
    bail!("Cargo.lock has no wasm-bindgen")
}

/// `dev`: the dev profile, quicker to build and with overflow checks, for finding a fault.
pub fn build_player(root: &Path, dev: bool) -> anyhow::Result<()> {
    let version = bindgen_version(root)?;
    let install = format!("cargo install wasm-bindgen-cli --version {version} --locked");
    let found = std::process::Command::new("wasm-bindgen").arg("--version").output();
    match found {
        Ok(out) if String::from_utf8_lossy(&out.stdout).split_whitespace().nth(1) == Some(version.as_str()) => {}
        Ok(out) => bail!("wasm-bindgen {version} is needed, found {}; run: {install}", String::from_utf8_lossy(&out.stdout).trim()),
        Err(_) => bail!("wasm-bindgen is not installed; run: {install}"),
    }

    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = std::process::Command::new(cargo)
        .args(["rustc", "-p", "bayleaf-player", "--lib", "--target", TARGET, "--crate-type", "cdylib"])
        .args(if dev { None } else { Some("--release") })
        .current_dir(root)
        .status()
        .context("could not run cargo")?;
    if !status.success() {
        bail!("building the web player failed; if the target is missing, run: rustup target add {TARGET}");
    }
    let target = std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from);
    let module = target.join(TARGET).join(if dev { "debug" } else { "release" }).join("bayleaf_player.wasm");
    let out = target.join(OUT_FOLDER);
    let status = std::process::Command::new("wasm-bindgen")
        .args(["--target", "web", "--no-typescript", "--out-dir"])
        .arg(&out)
        .arg(&module)
        .status()
        .context("could not run wasm-bindgen")?;
    if !status.success() {
        bail!("wasm-bindgen could not prepare {}", module.display());
    }
    let size = std::fs::metadata(out.join("bayleaf_player_bg.wasm")).map(|m| m.len() >> 20).unwrap_or(0);
    println!("web-player: {} ({size} MiB)", out.display());
    Ok(())
}

/// The media type a browser needs for a file.
fn media_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript",
        "wasm" => "application/wasm",
        "png" => "image/png",
        _ => "application/octet-stream",
    }
}

/// The file a request names inside `folder`; nothing outside it.
fn requested(folder: &Path, target: &str) -> Option<PathBuf> {
    let path = target.split(['?', '#']).next()?.trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let relative = Path::new(path);
    relative.components().all(|c| matches!(c, Component::Normal(_))).then(|| folder.join(relative))
}

fn answer(folder: &Path, stream: &mut TcpStream) -> std::io::Result<()> {
    let mut line = String::new();
    BufReader::new(&*stream).read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
    let file = requested(folder, target).filter(|f| f.is_file());
    match (method, file) {
        ("GET", Some(file)) => {
            let body = std::fs::read(&file)?;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", media_type(&file), body.len())?;
            stream.write_all(&body)
        }
        _ => stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
    }
}

pub fn serve(folder: &Path, port: u16) -> anyhow::Result<()> {
    if !folder.join("index.html").is_file() {
        bail!("{} has no index.html; give the folder a game was exported into for the web", folder.display());
    }
    let listener = TcpListener::bind(("127.0.0.1", port)).with_context(|| format!("port {port} is taken; give another"))?;
    println!("serve: http://127.0.0.1:{port}/ (this computer only; Ctrl+C stops)");
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let folder = folder.to_path_buf();
        std::thread::spawn(move || {
            if let Err(e) = answer(&folder, &mut stream) {
                eprintln!("serve: {e}");
            }
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::requested;
    use std::path::Path;

    #[test]
    fn only_files_inside_the_folder_are_served() {
        let folder = Path::new("out");
        assert_eq!(requested(folder, "/"), Some(folder.join("index.html")));
        assert_eq!(requested(folder, "/game.pack?x=1"), Some(folder.join("game.pack")));
        for outside in ["/../secret", "/a/../../b", "//c:/windows", "/./x"] {
            assert_eq!(requested(folder, outside), None, "{outside}");
        }
    }
}
