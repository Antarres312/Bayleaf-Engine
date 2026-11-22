//! Bayleaf Engine player: opens a project (or one scene, or the game exported beside it) and runs
//! its game in a window, with no editor around it. The game sees the keyboard and the mouse; F11 switches full screen.
//!
//! A library as well as the program: a game exported in native mode builds its own player that
//! calls [`run`] with the Rust scripts built into it.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::{Path, PathBuf};

use anyhow::Context;
use bayleaf_engine::render::{self, FrameOptions, RenderTarget, Renderer};
use bayleaf_engine::{CommandRegistry, Level, Session};
use clap::Parser;
use eframe::egui;
use eframe::egui_wgpu;
use serde_json::json;

#[derive(Parser)]
#[command(name = "bayleaf-player", version, about = "Runs a Bayleaf Engine game")]
struct Args {
    /// A project (.bproject file or its folder) or a scene (.bscene) to run. Without it, the game
    /// exported beside this program (its `game` folder).
    game: Option<PathBuf>,
    /// Start in full screen.
    #[arg(long)]
    fullscreen: bool,
    /// Record each frame's times by part and write them to this file (JSON) when the game closes.
    #[arg(long)]
    profile: Option<PathBuf>,
}

/// Script errors shown over the game at most.
const SHOWN_ERRORS: usize = 6;

struct Gpu {
    renderer: Renderer,
    target: RenderTarget,
    texture: egui::TextureId,
}

struct Player {
    session: Session,
    materials: render::MaterialLibrary,
    assets: render::assets::AssetCache,
    gpu: Option<Gpu>,
    /// Why the game cannot run, shown instead of it.
    failure: Option<String>,
    fullscreen: bool,
    gamepads: bayleaf_ui_egui::gamepad::Gamepads,
    /// The game's interfaces over the picture.
    interfaces: bayleaf_ui_egui::game_interface::GameInterfaces,
    /// The game's interfaces standing in the world.
    world: bayleaf_ui_egui::world_interface::WorldInterfaces,
    /// Where the profile goes when the game closes, if one is recorded.
    profile: Option<PathBuf>,
    /// How shadows are drawn, from the project's graphics settings.
    shadows: render::ShadowSettings,
    /// The post-processing effects the project lets the game use.
    effects: render::EffectsAllowed,
}

/// Opens `game` and starts play mode. Returns the window title.
fn open(registry: &CommandRegistry, session: &mut Session, game: &Path) -> Result<String, String> {
    let is_scene = game.extension().is_some_and(|e| e == "bscene");
    let opened = if is_scene {
        registry.execute(session, "scene.open", &json!({ "path": game }))
    } else {
        registry.execute(session, "project.open", &json!({ "path": game }))
    };
    let opened = opened.map_err(|e| format!("Cannot open {}: {e}", game.display()))?;
    // Scripts run the way the game was exported: interpreted, or compiled to machine code.
    let mode = bayleaf_engine::export::chosen_mode(session, bayleaf_engine::export::Platform::current());
    registry.execute(session, "play.start", &json!({ "scripts": mode.runtime() })).map_err(|e| format!("The game cannot start: {e}"))?;
    let title = opened.get("name").and_then(serde_json::Value::as_str).map(str::to_owned);
    Ok(title.unwrap_or_else(|| game.file_stem().map_or_else(|| "Bayleaf game".to_owned(), |s| s.to_string_lossy().into_owned())))
}

impl Player {
    fn frame(&mut self, ui: &mut egui::Ui, gpu: Option<&egui_wgpu::RenderState>) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.key_pressed(egui::Key::F11)) {
            self.fullscreen = !self.fullscreen;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
        }
        if let Some(failure) = &self.failure {
            ui.centered_and_justified(|ui| ui.label(egui::RichText::new(failure).size(16.0)));
            return;
        }
        let Some(gpu) = gpu else {
            ui.centered_and_justified(|ui| ui.label("The player needs a graphics adapter that wgpu can use, and none was found."));
            return;
        };
        let size = ui.available_size().max(egui::vec2(16.0, 16.0));
        let ppp = ctx.pixels_per_point();
        let (width, height) = ((size.x * ppp).round().max(1.0) as u32, (size.y * ppp).round().max(1.0) as u32);

        // Input, then one step of the game.
        let keys = ctx.input(|i| i.keys_down.iter().filter(|k| **k != egui::Key::F11).map(|k| k.name().to_owned()).collect());
        self.session.set_keys(keys);
        let pad = self.gamepads.read();
        self.interfaces.set_pad(pad.as_ref());
        self.session.set_pad(pad);
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        self.session.tick(dt);
        if !self.session.is_playing() {
            // A script stopped the game, or play mode ended with an error.
            self.failure = Some("The game stopped.".to_owned());
        }

        let view = self.gpu.get_or_insert_with(|| {
            let target = RenderTarget::new(&gpu.device, width, height);
            let texture = gpu.renderer.write().register_native_texture(&gpu.device, target.view(), render::wgpu::FilterMode::Linear);
            Gpu { renderer: Renderer::new(&gpu.device), target, texture }
        });
        if view.target.size() != (width, height) {
            view.target = RenderTarget::new(&gpu.device, width, height);
            gpu.renderer.write().update_egui_texture_from_wgpu_texture(&gpu.device, view.target.view(), render::wgpu::FilterMode::Linear, view.texture);
        }
        let aspect = size.x / size.y;
        let scene = self.session.scene();
        let camera = render::game_camera(scene, aspect);
        let allow = |path: &Path| Ok(path.to_path_buf());
        let mut assets = render::FileAssets {
            base: self.session.script_dir(),
            materials: &mut self.materials,
            cache: &mut self.assets,
            allow: &allow,
            warnings: Vec::new(),
        };
        // The game camera's own post-processing, else the scene's, as the project allows.
        let effects = render::Effects { show: true, allowed: self.effects, camera: render::camera_object(scene) };
        let options = FrameOptions { show_grid: false, selected: None, time: self.session.play_time() as f32, shadows: self.shadows, effects };
        let gathering = std::time::Instant::now();
        let mut frame = render::extract_with(scene, camera, options, &mut assets);
        if let Some(particles) = self.session.particles() {
            frame.items.extend(particles.draw_items(scene, &frame.camera, &mut assets));
        }
        let extract = gathering.elapsed();
        // Interfaces standing in the world, answering the pointer where it meets them.
        let rect = egui::Rect::from_min_size(ui.cursor().min, size);
        let pointer = bayleaf_ui_egui::world_interface::PointerInput::of(ui, rect);
        let parts = bayleaf_ui_egui::world_interface::View { gpu, renderer: &mut view.renderer, camera: &frame.camera, pointer: &pointer, interactive: true };
        let over_world_panel = self.world.prepare(parts, &mut self.session, &mut self.interfaces, &mut frame.items);
        let drawing = std::time::Instant::now();
        view.renderer.render(&gpu.device, &gpu.queue, &view.target, &frame);
        let stats = view.renderer.stats();
        self.session.profile_drawn(bayleaf_engine::profiler::DrawnStats {
            extract,
            draw: drawing.elapsed(),
            draw_calls: stats.draw_calls,
            triangles: stats.triangles,
            mesh_bytes: stats.mesh_bytes,
            image_bytes: stats.image_bytes,
        });

        let image = egui::Image::new(egui::load::SizedTexture::new(view.texture, size)).sense(egui::Sense::click_and_drag());
        let response = ui.add(image);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Canvas, true, "Game view"));
        let over_interface = bayleaf_ui_egui::game_interface::show(ui, response.rect, &mut self.session, &mut self.interfaces, true) || over_world_panel;
        let pointer = response.hover_pos().map(|p| {
            let local = (p - response.rect.min) / response.rect.size();
            [local.x * 2.0 - 1.0, 1.0 - local.y * 2.0]
        });
        let buttons = ctx.input(|i| u8::from(i.pointer.primary_down()) | u8::from(i.pointer.secondary_down()) << 1 | u8::from(i.pointer.middle_down()) << 2);
        // Clicks on the interface are not the game's.
        self.session.set_pointer(pointer, if pointer.is_some() && !over_interface { buttons } else { 0 });
        self.session.set_pointer_over_interface(over_interface);
        self.paint_errors(ui, response.rect);
        ctx.request_repaint();
    }

    /// The latest script errors, over the bottom of the game, so a broken script is not silent.
    fn paint_errors(&self, ui: &egui::Ui, rect: egui::Rect) {
        let errors: Vec<&str> = self.session.console().iter().rev().filter(|l| l.level == Level::Error).take(SHOWN_ERRORS).map(|l| l.text.as_str()).collect();
        if errors.is_empty() {
            return;
        }
        let text = errors.into_iter().rev().collect::<Vec<_>>().join("\n");
        let font = egui::FontId::monospace(13.0);
        let galley = ui.painter().layout(text, font, egui::Color32::from_rgb(0xff, 0xd8, 0xd2), rect.width() - 32.0);
        let panel = egui::Rect::from_min_size(rect.left_bottom() + egui::vec2(8.0, -galley.size().y - 24.0), galley.size() + egui::vec2(16.0, 16.0));
        ui.painter().rect_filled(panel, 3.0, egui::Color32::from_rgba_unmultiplied(0x30, 0x08, 0x08, 0xe0));
        ui.painter().galley(panel.min + egui::vec2(8.0, 8.0), galley, egui::Color32::WHITE);
    }
}

impl eframe::App for Player {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let gpu = frame.wgpu_render_state();
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(egui::Color32::BLACK)).show(ui, |ui| self.frame(ui, gpu));
    }

    /// Writes the recorded profile, if one was asked for.
    fn on_exit(&mut self) {
        let Some(path) = &self.profile else { return };
        let registry = CommandRegistry::with_builtins();
        let written = registry
            .execute(&mut self.session, "profile.frames", &json!({}))
            .map_err(|e| e.to_string())
            .and_then(|frames| serde_json::to_string_pretty(&frames).map_err(|e| e.to_string()))
            .and_then(|text| std::fs::write(path, text).map_err(|e| e.to_string()));
        if let Err(problem) = written {
            eprintln!("could not write the profile to {}: {problem}", path.display());
        }
    }
}

/// The window icon the project chooses (`settings.game.icon`), if it reads as an image.
fn game_icon(session: &Session) -> Option<egui::IconData> {
    let icon = bayleaf_engine::project_cmds::settings_of(session).game.icon;
    if icon.is_empty() {
        return None;
    }
    let image = render::assets::load_image(&session.script_dir().join(icon)).ok()?;
    Some(egui::IconData { rgba: image.rgba, width: image.width, height: image.height })
}

/// Runs the game given on the command line, or the one exported beside this program. `natives` are
/// the scripts built into this program (native mode), each by the module path it stands for in
/// the game (`scripts/mover.wasm`); the plain player has none.
pub fn run(natives: Vec<(PathBuf, bayleaf_engine::NativeScript)>) -> anyhow::Result<()> {
    let args = Args::parse();
    let game = match args.game.clone() {
        Some(game) => game,
        None => std::env::current_exe()
            .ok()
            .and_then(|program| bayleaf_engine::export::packaged_game(&program))
            .context("give a project or scene to run; no game was exported beside this program")?,
    };
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    session.set_sound(true);
    if args.profile.is_some() {
        let _ = registry.execute(&mut session, "profile.start", &json!({}));
    }
    session.set_native_scripts(natives);
    // A project that does not open fails here, before a window appears; a game that cannot start
    // shows why in the window.
    let (title, failure) = match open(&registry, &mut session, &game) {
        Ok(title) => (title, None),
        Err(e) if session.scene_path().is_some() => ("Bayleaf game".to_owned(), Some(e)),
        Err(e) => return Err(anyhow::anyhow!(e)),
    };

    let mut viewport = egui::ViewportBuilder::default().with_title(&title).with_inner_size([1280.0, 720.0]).with_fullscreen(args.fullscreen);
    // The game's own icon (Project Settings), else the engine's.
    if let Some(icon) = game_icon(&session) {
        viewport = viewport.with_icon(icon);
    } else if let Some(icon) = bayleaf_ui_egui::brand::window_icon() {
        viewport = viewport.with_icon(egui::IconData { rgba: icon.rgba, width: icon.size, height: icon.size });
    }
    let options = eframe::NativeOptions { viewport, persist_window: false, ..Default::default() };
    let graphics = bayleaf_engine::project_cmds::settings_of(&session).graphics;
    let (shadows, effects) = (graphics.shadows(), graphics.effects.render());
    let player = Player {
        session,
        materials: render::MaterialLibrary::default(),
        assets: render::assets::AssetCache::default(),
        gpu: None,
        gamepads: bayleaf_ui_egui::gamepad::Gamepads::default(),
        interfaces: bayleaf_ui_egui::game_interface::GameInterfaces::default(),
        world: bayleaf_ui_egui::world_interface::WorldInterfaces::default(),
        shadows,
        effects,
        failure,
        fullscreen: args.fullscreen,
        profile: args.profile.clone(),
    };
    eframe::run_native("Bayleaf Player", options, Box::new(move |_| Ok(Box::new(player) as Box<dyn eframe::App>)))
        .map_err(|e| anyhow::anyhow!("could not open the game window: {e}"))
        .context(title)
}
