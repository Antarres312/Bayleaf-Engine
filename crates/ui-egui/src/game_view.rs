//! Game view: the game as players see it, through the scene's first camera (or a view of everything
//! when the scene has none), with the game's state beside it. In play mode the pointer and mouse
//! buttons over the view go to the scripts.
use crate::i18n::tl;

use std::collections::BTreeMap;

use bayleaf_render::{FrameOptions, RenderTarget, Renderer};
use serde_json::Value;

use crate::Editor;
use crate::theme::{self, ROW_HEIGHT};

/// How often saved values are read from the project while the game is not running.
const SAVED_REFRESH: std::time::Duration = std::time::Duration::from_secs(1);
/// Width of the state panel beside the view.
const STATE_WIDTH: f32 = 240.0;

/// Saved values as read from the project, or why they could not be.
type SavedRead = Result<BTreeMap<String, f64>, String>;

struct Gpu {
    renderer: Renderer,
    target: RenderTarget,
    texture: egui::TextureId,
}

pub(crate) struct GameView {
    gpu: Option<Gpu>,
    /// Shows the game's state beside the view.
    pub show_state: bool,
    /// Saved values as the project held them when last read, outside play mode.
    saved: Option<(std::time::Instant, SavedRead)>,
}

impl Default for GameView {
    fn default() -> Self {
        Self { gpu: None, show_state: true, saved: None }
    }
}

pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui, gpu: Option<&egui_wgpu::RenderState>) {
    let mut view = std::mem::take(&mut editor.game_view);
    let mut command = None;
    ui.horizontal(|ui| {
        command = crate::scene_view::play_controls(ui, &editor.session);
        ui.separator();
        ui.checkbox(&mut view.show_state, tl("State")).on_hover_text(tl("Show the game's values, each player's and the saved ones"));
        if let Some(won) = editor.session.game_state().and_then(|s| s.outcome) {
            ui.separator();
            let text = if won { "Game won" } else { "Game lost" };
            let label = ui.label(egui::RichText::new(text).strong());
            // Said by screen readers when it appears, wherever the focus is.
            ui.ctx().accesskit_node_builder(label.id, |node| node.set_live(egui::accesskit::Live::Polite));
        }
    });
    if let Some(command) = command {
        editor.dispatch(command, &Value::Null, None);
    }
    let area = ui.available_rect_before_wrap();
    let side = if view.show_state { STATE_WIDTH.min(area.width() * 0.45) } else { 0.0 };
    let view_rect = egui::Rect::from_min_max(area.min, egui::pos2(area.max.x - side, area.max.y));
    ui.scope_builder(egui::UiBuilder::new().max_rect(view_rect), |ui| match gpu {
        Some(gpu) => viewport(&mut view, editor, ui, gpu),
        None => {
            ui.label(tl("The game view needs the wgpu renderer, which is not available here."));
        }
    });
    if view.show_state {
        let side_rect = egui::Rect::from_min_max(egui::pos2(view_rect.max.x + theme::GAP, area.min.y), area.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(side_rect), |ui| state_panel(&mut view, editor, ui));
    }
    ui.allocate_rect(area, egui::Sense::hover());
    editor.game_view = view;
}

fn viewport(view: &mut GameView, editor: &mut Editor, ui: &mut egui::Ui, gpu: &egui_wgpu::RenderState) {
    let size = ui.available_size().max(egui::vec2(16.0, 16.0));
    let ppp = ui.ctx().pixels_per_point();
    let (width, height) = ((size.x * ppp).round().max(1.0) as u32, (size.y * ppp).round().max(1.0) as u32);
    let target = view.gpu.get_or_insert_with(|| {
        let target = RenderTarget::new(&gpu.device, width, height);
        let texture = gpu.renderer.write().register_native_texture(&gpu.device, target.view(), bayleaf_render::wgpu::FilterMode::Linear);
        Gpu { renderer: Renderer::new(&gpu.device), target, texture }
    });
    if target.target.size() != (width, height) {
        target.target = RenderTarget::new(&gpu.device, width, height);
        gpu.renderer.write().update_egui_texture_from_wgpu_texture(&gpu.device, target.target.view(), bayleaf_render::wgpu::FilterMode::Linear, target.texture);
    }
    let graphics = crate::project_settings::current(editor).graphics;
    let shadows = graphics.shadows();
    let effects = bayleaf_render::Effects { show: true, allowed: graphics.effects.render(), camera: bayleaf_render::camera_object(editor.session.scene()) };
    let scene = editor.session.scene();
    let camera = bayleaf_render::game_camera(scene, size.x / size.y);
    let base = editor.session.script_dir();
    let allow = |path: &std::path::Path| Ok(path.to_path_buf());
    // The scene view's caches: both views draw the same files.
    let shared = &mut editor.scene_view;
    let mut assets = bayleaf_render::FileAssets { base, materials: &mut shared.materials, cache: &mut shared.assets, allow: &allow, warnings: Vec::new() };
    let options = FrameOptions { show_grid: false, selected: None, time: editor.session.play_time() as f32, shadows, effects };
    let frame = bayleaf_render::extract_with(scene, camera, options, &mut assets);
    target.renderer.render(&gpu.device, &gpu.queue, &target.target, &frame);

    let image = egui::Image::new(egui::load::SizedTexture::new(target.texture, size)).sense(egui::Sense::click_and_drag());
    let response = ui.add(image);
    let playing = editor.session.is_playing();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Canvas, true, if playing { "Game view, playing" } else { "Game view" }));
    if response.clicked() {
        response.request_focus();
    }
    let t = theme::tokens(ui);
    if playing {
        let pointer = response.hover_pos().map(|p| {
            let local = (p - response.rect.min) / response.rect.size();
            [local.x * 2.0 - 1.0, 1.0 - local.y * 2.0]
        });
        let buttons = ui.input(|i| u8::from(i.pointer.primary_down()) | u8::from(i.pointer.secondary_down()) << 1 | u8::from(i.pointer.middle_down()) << 2);
        // Buttons count only while the pointer is over the view.
        editor.session.set_pointer(pointer, if pointer.is_some() { buttons } else { 0 });
        ui.ctx().request_repaint();
    } else {
        // Before play the view shows where the game starts; say how to run it.
        let hint = "Press Play (Ctrl+P) to run the game here";
        let galley = ui.painter().layout_no_wrap(hint.to_owned(), egui::FontId::proportional(14.0), t.text);
        let at = response.rect.center_bottom() - egui::vec2(galley.size().x / 2.0, galley.size().y + 24.0);
        let back = egui::Rect::from_min_size(at, galley.size()).expand2(egui::vec2(10.0, 6.0));
        ui.painter().rect_filled(back, f32::from(theme::RADIUS), t.panel);
        ui.painter().galley(at, galley, t.text);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(response.rect, 0.0, egui::Stroke::new(2.0, t.focus), egui::StrokeKind::Inside);
    }
}

/// The game's values, each player's and the saved ones, as name and number rows.
fn state_panel(view: &mut GameView, editor: &mut Editor, ui: &mut egui::Ui) {
    let t = theme::tokens(ui);
    let state = editor.session.game_state();
    let saved = match &state {
        Some(state) => Ok(state.saved.clone()),
        None => {
            if view.saved.as_ref().is_none_or(|(read, _)| read.elapsed() >= SAVED_REFRESH) {
                view.saved = Some((std::time::Instant::now(), bayleaf_engine::saved::read_saved(&editor.session)));
            }
            view.saved.as_ref().map_or_else(|| Ok(BTreeMap::new()), |(_, saved)| saved.clone())
        }
    };
    let mut clear = false;
    egui::ScrollArea::vertical().id_salt("game state").auto_shrink([false, false]).show(ui, |ui| {
        theme::headed(t, "Game").show(ui, |ui| match &state {
            Some(state) => values(ui, t, "Game", &state.game),
            None => muted(ui, t, "The game's values appear here while it plays."),
        });
        ui.add_space(theme::SECTION_GAP);
        theme::headed(t, "Players").show(ui, |ui| match &state {
            Some(state) if state.players.is_empty() => muted(ui, t, "No player values yet."),
            Some(state) => {
                for (id, player) in &state.players {
                    let name = editor.session.scene().name(*id).unwrap_or("Player").to_owned();
                    ui.label(egui::RichText::new(&name).color(t.text_muted));
                    values(ui, t, &name, player);
                }
            }
            None => muted(ui, t, "Each player's values appear here while the game plays."),
        });
        ui.add_space(theme::SECTION_GAP);
        theme::headed(t, "Saved").show(ui, |ui| {
            match &saved {
                Ok(saved) => values(ui, t, "Saved", saved),
                Err(problem) => {
                    ui.add(
                        egui::Label::new(egui::RichText::new(crate::i18n::tf("Not read: {problem}", &[("problem", &problem.to_string())])).color(t.text))
                            .wrap(),
                    );
                }
            }
            if editor.session.project_dir().is_none() {
                muted(ui, t, "Saved values are kept only in a project.");
            }
            let any = saved.as_ref().is_ok_and(|s| !s.is_empty());
            let button = ui.add_enabled(any, egui::Button::new(tl("Clear Saved Values"))).on_hover_text(tl("Forget what the game saved in this project"));
            clear = button.clicked();
        });
    });
    if clear && editor.dispatch("game.saved.clear", &Value::Null, None).is_some() {
        view.saved = None;
    }
}

/// One row per value: the name, then the number. Rows say both to screen readers.
fn values(ui: &mut egui::Ui, t: &theme::Tokens, group: &str, values: &BTreeMap<String, f64>) {
    if values.is_empty() {
        muted(ui, t, "Nothing set yet.");
        return;
    }
    for (name, value) in values {
        let text = format_value(*value);
        let row = ui.horizontal(|ui| {
            ui.set_min_height(ROW_HEIGHT - 6.0);
            ui.label(name);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(egui::RichText::new(&text).monospace()));
        });
        let label = format!("{name} {text}, {group}");
        ui.ctx().accesskit_node_builder(row.response.id, |node| node.set_label(label));
    }
}

fn muted(ui: &mut egui::Ui, t: &theme::Tokens, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).color(t.text_muted)).wrap());
}

/// A number as people write it: whole numbers without a fraction, others to three places.
fn format_value(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 { format!("{value:.0}") } else { format!("{value:.3}") }
}
