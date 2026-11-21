//! Scene view: the scene rendered with wgpu from an orbiting editor camera.
//!
//! Mouse: drag to orbit, Shift+drag or middle-drag to pan, scroll to zoom, click to select.
//! Keyboard, with the view focused: arrows orbit, Shift+arrows pan, +/− zoom, F frames the selection.

use bayleaf_core::math::{Vec2, Vec3};
use bayleaf_render::{CameraView, FrameOptions, RenderTarget, Renderer};

use crate::Editor;

/// Camera that circles a target point. The editor's own view, not saved in the scene.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Orbit {
    pub target: Vec3,
    /// Radians around +Y.
    pub yaw: f32,
    /// Radians above the horizon.
    pub pitch: f32,
    pub distance: f32,
}

impl Default for Orbit {
    fn default() -> Self {
        Self { target: Vec3::ZERO, yaw: 0.75, pitch: 0.45, distance: 8.0 }
    }
}

const PITCH_LIMIT: f32 = 1.55;

impl Orbit {
    pub fn eye(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.target + Vec3::new(cp * sy, sp, cp * cy) * self.distance
    }

    pub fn view(&self, aspect: f32) -> CameraView {
        CameraView::look_at(self.eye(), self.target, 50.0, aspect)
    }

    pub fn rotate(&mut self, yaw: f32, pitch: f32) {
        self.yaw = (self.yaw + yaw).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + pitch).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    /// Moves the target across the screen; `delta` is in fractions of the view height.
    pub fn pan(&mut self, delta: Vec2) {
        let forward = (self.target - self.eye()).normalize_or(Vec3::NEG_Z);
        let right = forward.cross(Vec3::Y).normalize_or(Vec3::X);
        let up = right.cross(forward);
        let scale = self.distance * 0.9;
        self.target += (-right * delta.x + up * delta.y) * scale;
    }

    pub fn zoom(&mut self, factor: f32) {
        self.distance = (self.distance * factor).clamp(0.05, 10_000.0);
    }

    /// Looks at a sphere of `radius` around `center` from the current direction.
    pub fn frame(&mut self, center: Vec3, radius: f32) {
        self.target = center;
        self.distance = radius.max(0.5) * 2.6 + 1.0;
    }
}

struct GpuView {
    renderer: Renderer,
    target: RenderTarget,
    texture: egui::TextureId,
}

#[derive(Default)]
pub(crate) struct SceneView {
    pub orbit: Orbit,
    pub show_grid: bool,
    gpu: Option<GpuView>,
    framed_once: bool,
    /// Compiled material graphs; the one open in the graph editor shows unsaved changes live.
    pub materials: bayleaf_render::MaterialLibrary,
    /// When the view first drew, for the Time node of animated materials.
    started: Option<std::time::Instant>,
}

impl SceneView {
    pub fn new() -> Self {
        Self { show_grid: true, ..Self::default() }
    }

    pub fn frame_selection(&mut self, editor: &Editor) {
        let scene = editor.session.scene();
        let bounds = match editor.selected {
            Some(id) => bayleaf_render::object_bounds(scene, id),
            None => bayleaf_render::bounds(scene),
        };
        if let Some((center, radius)) = bounds {
            self.orbit.frame(center, radius);
        }
    }
}

pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui, gpu: Option<&egui_wgpu::RenderState>) {
    let mut view = std::mem::take(&mut editor.scene_view);
    if let Some(command) = toolbar(&mut view, editor, ui) {
        editor.dispatch(command, &serde_json::Value::Null, None);
    }
    if !view.framed_once && editor.session.scene().object_count() > 0 {
        view.frame_selection(editor);
        view.framed_once = true;
    }
    match gpu {
        Some(gpu) => viewport(&mut view, editor, ui, gpu),
        None => {
            ui.label("The scene view needs the wgpu renderer, which is not available here.");
        }
    }
    editor.scene_view = view;
}

/// View controls and the play controls. Returns a play command to run.
fn toolbar(view: &mut SceneView, editor: &Editor, ui: &mut egui::Ui) -> Option<&'static str> {
    let mut command = None;
    let session = &editor.session;
    ui.horizontal(|ui| {
        let t = crate::theme::tokens(ui);
        // Accessible names start with the visible label but say more, as the Play menu shares the word.
        let named = |response: egui::Response, name: &str| {
            response.ctx.accesskit_node_builder(response.id, |node| node.set_label(name));
            response
        };
        if session.is_playing() {
            let (label, name) = if session.is_paused() { ("Resume", "Resume playing") } else { ("Pause", "Pause playing") };
            if named(ui.button(label), name).on_hover_text("Ctrl+Shift+P").clicked() {
                command = Some("play.pause");
            }
            let stop = ui.add(egui::Button::new(egui::RichText::new("Stop").color(t.on_accent)).fill(t.accent));
            if named(stop, "Stop playing").on_hover_text("Stop and restore the scene (Ctrl+P)").clicked() {
                command = Some("play.stop");
            }
            let state = if session.is_paused() { "Paused" } else { "Playing" };
            ui.label(egui::RichText::new(format!("{state} · {:.1} s", session.play_time())).strong());
        } else {
            let play = ui.add(egui::Button::new(egui::RichText::new("Play").color(t.on_accent)).fill(t.accent));
            if named(play, "Play scene").on_hover_text("Run the scene's scripts (Ctrl+P)").clicked() {
                command = Some("play.start");
            }
        }
        ui.separator();
        let frame_label = if editor.selected.is_some() { "Frame Selection" } else { "Frame All" };
        if ui.button(frame_label).on_hover_text("Move the view to show it (F)").clicked() {
            view.frame_selection(editor);
        }
        ui.checkbox(&mut view.show_grid, "Grid");
        ui.separator();
        ui.weak("Drag: orbit · Shift+drag: pan · Scroll: zoom · Click: select");
    });
    command
}

fn viewport(view: &mut SceneView, editor: &mut Editor, ui: &mut egui::Ui, gpu: &egui_wgpu::RenderState) {
    let size = ui.available_size().max(egui::vec2(16.0, 16.0));
    let ppp = ui.ctx().pixels_per_point();
    let (width, height) = ((size.x * ppp).round().max(1.0) as u32, (size.y * ppp).round().max(1.0) as u32);
    let aspect = size.x / size.y;

    let gpu_view = view.gpu.get_or_insert_with(|| {
        let target = RenderTarget::new(&gpu.device, width, height);
        let texture = gpu.renderer.write().register_native_texture(&gpu.device, target.view(), bayleaf_render::wgpu::FilterMode::Linear);
        GpuView { renderer: Renderer::new(&gpu.device), target, texture }
    });
    if gpu_view.target.size() != (width, height) {
        gpu_view.target = RenderTarget::new(&gpu.device, width, height);
        gpu.renderer.write().update_egui_texture_from_wgpu_texture(
            &gpu.device,
            gpu_view.target.view(),
            bayleaf_render::wgpu::FilterMode::Linear,
            gpu_view.texture,
        );
    }

    if let Some(doc) = editor.session.graph()
        && doc.graph.domain == bayleaf_render::material::CATALOG.domain
        && let Some(path) = &doc.path
    {
        view.materials.set_live(Some((path, &doc.graph)));
    } else {
        view.materials.set_live(None);
    }
    let started = *view.started.get_or_insert_with(std::time::Instant::now);
    let time = if editor.session.is_playing() { editor.session.play_time() as f32 } else { started.elapsed().as_secs_f32() };
    let base = editor.session.script_dir();
    let materials = &mut view.materials;
    let options = FrameOptions { show_grid: view.show_grid, selected: editor.selected, time };
    let frame = bayleaf_render::extract_with(editor.session.scene(), view.orbit.view(aspect), options, &mut |path| materials.get(&base.join(path)).ok());
    if frame.items.iter().any(|i| i.material.as_ref().is_some_and(|m| m.animated)) {
        ui.ctx().request_repaint();
    }
    gpu_view.renderer.render(&gpu.device, &gpu.queue, &gpu_view.target, &frame);

    let image = egui::Image::new(egui::load::SizedTexture::new(gpu_view.texture, size)).sense(egui::Sense::click_and_drag());
    let response = ui.add(image);
    if editor.session.is_playing() {
        // A frame in the accent colour, plus the toolbar text, shows play mode is on.
        let t = crate::theme::tokens(ui);
        ui.painter().rect_stroke(response.rect, 0.0, egui::Stroke::new(3.0, t.accent), egui::StrokeKind::Inside);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Canvas, true, "Scene view"));

    // Mouse.
    let shift = ui.input(|i| i.modifiers.shift);
    let delta = response.drag_delta() / size.y;
    if response.dragged_by(egui::PointerButton::Middle) || (response.dragged_by(egui::PointerButton::Primary) && shift) {
        view.orbit.pan(Vec2::new(delta.x, delta.y));
    } else if response.dragged_by(egui::PointerButton::Primary) || response.dragged_by(egui::PointerButton::Secondary) {
        view.orbit.rotate(-delta.x * 3.0, delta.y * 3.0);
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            view.orbit.zoom((-scroll * 0.0025).exp());
        }
    }
    // A press and release that barely moved is a click, even if egui already counted it as a drag.
    let tiny_drag = response.drag_stopped_by(egui::PointerButton::Primary)
        && ui.input(|i| i.pointer.press_origin().zip(i.pointer.interact_pos()).is_some_and(|(a, b)| a.distance(b) < 5.0));
    if response.clicked() || tiny_drag {
        response.request_focus();
        if let Some(pos) = response.interact_pointer_pos() {
            let local = (pos - response.rect.min) / response.rect.size();
            let ndc = Vec2::new(local.x * 2.0 - 1.0, 1.0 - local.y * 2.0);
            editor.selected = bayleaf_render::pick(&frame, ndc);
        }
    }

    // Keyboard, while the view has focus. During play the keys belong to the scripts.
    if response.has_focus() && !editor.session.is_playing() {
        let step = 0.08;
        ui.input(|i| {
            let held = |key| i.key_down(key);
            let x = f32::from(u8::from(held(egui::Key::ArrowRight))) - f32::from(u8::from(held(egui::Key::ArrowLeft)));
            let y = f32::from(u8::from(held(egui::Key::ArrowDown))) - f32::from(u8::from(held(egui::Key::ArrowUp)));
            if i.modifiers.shift {
                view.orbit.pan(Vec2::new(x, -y) * step * 0.3);
            } else {
                view.orbit.rotate(-x * step, -y * step);
            }
            if i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals) {
                view.orbit.zoom(0.85);
            }
            if i.key_pressed(egui::Key::Minus) {
                view.orbit.zoom(1.0 / 0.85);
            }
        });
        if ui.input(|i| i.key_pressed(egui::Key::F)) {
            view.frame_selection(editor);
        }
        if ui.input(|i| [egui::Key::ArrowLeft, egui::Key::ArrowRight, egui::Key::ArrowUp, egui::Key::ArrowDown].iter().any(|k| i.key_down(*k))) {
            ui.ctx().request_repaint();
        }
        // Arrows move the camera here instead of moving focus to the next control; Tab still leaves.
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(response.id, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, tab: false, escape: false })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::Orbit;
    use bayleaf_core::math::{Vec2, Vec3};

    #[test]
    fn orbit_keeps_its_distance_and_never_flips_over_the_pole() {
        let mut orbit = Orbit::default();
        let before = orbit.eye().distance(orbit.target);
        orbit.rotate(1.0, 10.0);
        assert!((orbit.eye().distance(orbit.target) - before).abs() < 1e-3);
        assert!(orbit.pitch < std::f32::consts::FRAC_PI_2);
    }

    #[test]
    fn panning_moves_target_and_eye_together() {
        let mut orbit = Orbit::default();
        let offset = orbit.eye() - orbit.target;
        orbit.pan(Vec2::new(0.3, 0.2));
        assert!(orbit.target.distance(Vec3::ZERO) > 0.1);
        assert!(((orbit.eye() - orbit.target) - offset).length() < 1e-4);
    }

    #[test]
    fn framing_a_larger_object_moves_further_away() {
        let mut small = Orbit::default();
        small.frame(Vec3::ZERO, 1.0);
        let mut large = Orbit::default();
        large.frame(Vec3::ZERO, 10.0);
        assert!(large.distance > small.distance);
    }
}
