//! The web player: the page's `index.html` (written by Export Game for the web) loads this module,
//! which reads `game.pack` into the game's files, opens the project from it and runs the game in
//! the page's canvas. Saved values are kept in the browser's storage for the site, by game.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use bayleaf_engine::web_export::{PACK_FILE, unpack};
use bayleaf_engine::{CommandRegistry, Session};
use eframe::egui;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen_futures::JsFuture;

/// Where the pack's files are in the game's file store.
const GAME_ROOT: &str = "/game";
/// The page's canvas and status line (see `web_export::index_html`).
const CANVAS_ID: &str = "bayleaf-game";
const STATUS_ID: &str = "bayleaf-status";
/// How often changed saved values are written to the browser's storage, in seconds.
const KEEP_EVERY: f64 = 2.0;

/// The start of the browser storage keys of this game's files: `bayleaf/<game>/`.
static STORAGE_PREFIX: OnceLock<String> = OnceLock::new();

/// Runs when the page has loaded the module.
#[wasm_bindgen(start)]
pub fn start() {
    // What eframe and the player say goes to the browser's console.
    let _ = eframe::WebLogger::init(log::LevelFilter::Info);
    wasm_bindgen_futures::spawn_local(async {
        if let Err(problem) = begin().await {
            status(Some(&problem));
        }
    });
}

/// What a browser call that failed said.
fn said(error: wasm_bindgen::JsValue) -> String {
    error.as_string().or_else(|| js_sys::JSON::stringify(&error).ok().and_then(|s| s.as_string())).unwrap_or_else(|| format!("{error:?}"))
}

/// Shows a message in the page's status line, or hides it.
fn status(message: Option<&str>) {
    let Some(element) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id(STATUS_ID)) else { return };
    match message {
        Some(text) => {
            element.set_text_content(Some(text));
            let _ = element.remove_attribute("hidden");
        }
        None => {
            let _ = element.set_attribute("hidden", "");
        }
    }
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

/// The game's files kept in the browser (its saved values), back into the game's file store.
fn restore_kept(prefix: &str) {
    let Some(storage) = storage() else { return };
    let count = storage.length().unwrap_or(0);
    for index in 0..count {
        let Some(key) = storage.key(index).ok().flatten() else { continue };
        let Some(relative) = key.strip_prefix(prefix) else { continue };
        if let Some(text) = storage.get_item(&key).ok().flatten() {
            bayleaf_core::files::mount(&Path::new(GAME_ROOT).join(relative), text.into_bytes());
        }
    }
}

async fn begin() -> Result<(), String> {
    let window = web_sys::window().ok_or("this page has no window")?;
    let document = window.document().ok_or("this page has no document")?;
    let canvas = document
        .get_element_by_id(CANVAS_ID)
        .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        .ok_or_else(|| format!("the page has no canvas with the id {CANVAS_ID}"))?;

    let response = JsFuture::from(window.fetch_with_str(PACK_FILE)).await.map_err(|e| format!("The game could not be loaded: {}", said(e)))?;
    let response: web_sys::Response = response.dyn_into().map_err(|_| "The game could not be loaded.")?;
    if !response.ok() {
        return Err(format!("The game could not be loaded: {PACK_FILE} answered {} {}.", response.status(), response.status_text()));
    }
    let buffer = JsFuture::from(response.array_buffer().map_err(said)?).await.map_err(|e| format!("The game could not be loaded: {}", said(e)))?;
    let files = unpack(&js_sys::Uint8Array::new(&buffer).to_vec()).map_err(|e| format!("The game could not be loaded: {e}."))?;
    log::info!("game pack read: {} files", files.len());
    for (path, bytes) in files {
        bayleaf_core::files::mount(&Path::new(GAME_ROOT).join(path), bytes);
    }

    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    // Sound starts quiet: the browser lets it play after the first click or key (`wake_sound`).
    session.set_sound(true);
    let game = PathBuf::from(GAME_ROOT);
    let name = bayleaf_engine::project_cmds::project_file_in(&game)
        .and_then(|f| bayleaf_core::files::read_to_string(f).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["name"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "game".to_owned());
    let prefix = STORAGE_PREFIX.get_or_init(|| format!("bayleaf/{name}/"));
    restore_kept(prefix);
    // A game that cannot start shows why over the canvas.
    let failure = match super::open(&registry, &mut session, &game) {
        Ok(title) => {
            document.set_title(&title);
            None
        }
        Err(e) if session.scene_path().is_some() => Some(e),
        Err(e) => return Err(e),
    };

    log::info!("game opened; starting the renderer");
    let options = eframe::WebOptions { wgpu_options: bayleaf_ui_egui::wgpu_configuration(), ..Default::default() };
    let player = super::Player::new(session, failure, false, None);
    eframe::WebRunner::new()
        .start(canvas, options, Box::new(move |_| Ok(Box::new(player) as Box<dyn eframe::App>)))
        .await
        .map_err(|e| format!("The game could not start in this browser: {}", said(e)))?;
    log::info!("game running");
    status(None);
    Ok(())
}

/// Every few seconds, saved values that scripts changed go to the browser's storage, since a page
/// can be closed at any moment without the game stopping.
pub(crate) fn keep_saved(session: &mut Session, ctx: &egui::Context) {
    let now = ctx.input(|i| i.time);
    let due = ctx.data_mut(|d| {
        let last = d.get_temp_mut_or(egui::Id::unique("bayleaf kept saved"), 0.0);
        let due = now - *last >= KEEP_EVERY;
        if due {
            *last = now;
        }
        due
    });
    if !due {
        return;
    }
    session.keep_saved_values();
    let (Some(prefix), Some(storage)) = (STORAGE_PREFIX.get(), storage()) else { return };
    for path in bayleaf_core::files::take_written() {
        let Ok(relative) = path.strip_prefix(GAME_ROOT) else { continue };
        let Ok(text) = bayleaf_core::files::read_to_string(&path) else { continue };
        let key = format!("{prefix}{}", relative.to_string_lossy());
        if storage.set_item(&key, &text).is_err() {
            web_sys::console::warn_1(&format!("{key} could not be kept: the browser's storage for this site is full or turned off").into());
        }
    }
}

/// A browser plays a page's sound only once it is used: each click, touch or key asks for it, which
/// does nothing once it plays.
pub(crate) fn wake_sound(session: &mut Session, ctx: &egui::Context) {
    let used = ctx.input(|i| {
        i.events
            .iter()
            .any(|e| matches!(e, egui::Event::PointerButton { pressed: true, .. } | egui::Event::Key { pressed: true, .. } | egui::Event::Touch { .. }))
    });
    if used {
        session.resume_sound();
    }
}

/// Says in the console when the first frame is drawn, so a page that stops before it shows where.
pub(crate) fn frame_done() {
    static DRAWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !DRAWN.swap(true, std::sync::atomic::Ordering::Relaxed) {
        log::info!("first frame drawn");
    }
}
