//! Scene view: the scene rendered with wgpu from an orbiting editor camera.
//!
//! Mouse: drag to orbit, Shift+drag or middle-drag to pan, scroll to zoom, click to select.
//! Keyboard, with the view focused: arrows orbit, Shift+arrows pan, +/− zoom, F frames the selection.

use bayleaf_core::math::{Vec2, Vec3};
use bayleaf_render::{CameraView, FrameOptions, RenderTarget, Renderer};

use crate::{Editor, gizmo};

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
    /// The selected camera's view, shown in a corner of the scene view.
    preview: Option<(RenderTarget, egui::TextureId)>,
}

#[derive(Default)]
pub(crate) struct SceneView {
    pub orbit: Orbit,
    pub show_grid: bool,
    gpu: Option<GpuView>,
    framed_once: bool,
    /// Compiled material graphs; the one open in the graph editor shows unsaved changes live.
    pub materials: bayleaf_render::MaterialLibrary,
    /// Loaded images and models.
    pub assets: bayleaf_render::assets::AssetCache,
    /// When the view first drew, for the Time node of animated materials.
    started: Option<std::time::Instant>,
    /// What dragging a handle on the selected object does.
    pub tool: gizmo::Tool,
    /// A handle being dragged.
    gizmo_drag: Option<gizmo::Drag>,
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
        for tool in gizmo::Tool::ALL {
            let key = tool.key().name();
            let button = ui
                .add(egui::Button::selectable(view.tool == tool, tool.label()))
                .on_hover_text(format!("Drag the handles on the selected object to {} it ({key})", tool.label().to_lowercase()));
            if button.clicked() {
                view.tool = tool;
            }
        }
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
        GpuView { renderer: Renderer::new(&gpu.device), target, texture, preview: None }
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
    let allow = |path: &std::path::Path| Ok(path.to_path_buf());
    let mut assets = bayleaf_render::FileAssets { base, materials: &mut view.materials, cache: &mut view.assets, allow: &allow, warnings: Vec::new() };
    let options = FrameOptions { show_grid: view.show_grid, selected: editor.selected, time };
    let mut frame = bayleaf_render::extract_with(editor.session.scene(), view.orbit.view(aspect), options, &mut assets);
    frame.highlighted = editor.also_selected.clone();
    if frame.items.iter().any(|i| i.material.as_ref().is_some_and(|m| m.animated)) {
        ui.ctx().request_repaint();
    }
    gpu_view.renderer.render(&gpu.device, &gpu.queue, &gpu_view.target, &frame);

    // What the selected camera sees, rendered small.
    let preview_camera = editor.selected.and_then(|id| bayleaf_render::object_camera(editor.session.scene(), id, aspect));
    let preview_size = egui::vec2((size.x * 0.28).clamp(120.0, 360.0), (size.x * 0.28).clamp(120.0, 360.0) / aspect);
    let preview = preview_camera.map(|camera| {
        let (pw, ph) = ((preview_size.x * ppp).round().max(1.0) as u32, (preview_size.y * ppp).round().max(1.0) as u32);
        if gpu_view.preview.as_ref().is_none_or(|(target, _)| target.size() != (pw, ph)) {
            let target = RenderTarget::new(&gpu.device, pw, ph);
            let mut renderer = gpu.renderer.write();
            let texture = match gpu_view.preview.take() {
                Some((_, id)) => {
                    renderer.update_egui_texture_from_wgpu_texture(&gpu.device, target.view(), bayleaf_render::wgpu::FilterMode::Linear, id);
                    id
                }
                None => renderer.register_native_texture(&gpu.device, target.view(), bayleaf_render::wgpu::FilterMode::Linear),
            };
            gpu_view.preview = Some((target, texture));
        }
        let options = FrameOptions { show_grid: false, selected: None, time };
        let preview_frame = bayleaf_render::extract_with(editor.session.scene(), camera, options, &mut assets);
        if let Some((target, texture)) = &gpu_view.preview {
            gpu_view.renderer.render(&gpu.device, &gpu.queue, target, &preview_frame);
            *texture
        } else {
            gpu_view.texture
        }
    });

    let image = egui::Image::new(egui::load::SizedTexture::new(gpu_view.texture, size)).sense(egui::Sense::click_and_drag());
    let response = ui.add(image);
    if editor.session.is_playing() {
        // A frame in the accent colour, plus the toolbar text, shows play mode is on.
        let t = crate::theme::tokens(ui);
        ui.painter().rect_stroke(response.rect, 0.0, egui::Stroke::new(3.0, t.accent), egui::StrokeKind::Inside);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Canvas, true, "Scene view"));
    // A prefab dragged from the Project panel is placed in the scene.
    if let Some(asset) = response.dnd_release_payload::<crate::project::DraggedAsset>()
        && asset.relative.ends_with(".bprefab")
    {
        let path = editor.session.script_dir().join(&asset.relative);
        editor.place_prefab(&path);
    }
    if let Some(texture) = preview {
        paint_preview(ui, texture, response.rect, preview_size);
    }

    // Handles on the selected object. Not during play: play-mode changes are thrown away on stop.
    let projector = gizmo::Projector { view_proj: frame.camera.view_proj(), rect: response.rect, eye: frame.camera.position };
    let target = editor.selected.filter(|_| !editor.session.is_playing()).and_then(|id| {
        let scene = editor.session.scene();
        let world = scene.world_matrix(id)?;
        let parent = scene.parent(id).and_then(|p| scene.world_matrix(p)).unwrap_or(bayleaf_core::math::Mat4::IDENTITY);
        let local = scene.get::<bayleaf_engine::scene::components::Transform>(id)?.clone();
        Some((id, world, parent, local))
    });
    let hovered = target.as_ref().and_then(|(_, world, _, _)| {
        // A drag starts once the pointer has moved, maybe off the handle: test where it was pressed.
        let pointer = if response.drag_started() { ui.input(|i| i.pointer.press_origin()) } else { response.hover_pos() }?;
        gizmo::hit(&projector, *world, view.tool, pointer, 7.0)
    });
    if response.drag_started_by(egui::PointerButton::Primary)
        && let (Some(handle), Some((id, world, parent, local))) = (hovered, target.clone())
        && let Some(start) = ui.input(|i| i.pointer.press_origin())
    {
        // The other selected objects follow the primary one (objects inside a selected object move with it).
        let mut objects = vec![gizmo::Start { object: id, local, world, parent }];
        let scene = editor.session.scene();
        for other in editor.selection_roots().into_iter().filter(|o| *o != id) {
            if let (Some(world), Some(local)) = (scene.world_matrix(other), scene.get::<bayleaf_engine::scene::components::Transform>(other)) {
                let parent = scene.parent(other).and_then(|p| scene.world_matrix(p)).unwrap_or(bayleaf_core::math::Mat4::IDENTITY);
                objects.push(gizmo::Start { object: other, local: local.clone(), world, parent });
            }
        }
        view.gizmo_drag = Some(gizmo::Drag { handle, objects, start });
    }
    if let Some(drag) = view.gizmo_drag.clone() {
        if let Some(pointer) = response.interact_pointer_pos().or_else(|| response.hover_pos()) {
            // Every changed object, under one merge key: the whole drag is one undo step.
            let key = format!("gizmo {}", editor.gesture);
            for (object, moved) in gizmo::drag_to(&projector, &drag, pointer) {
                let current = editor.session.scene().get::<bayleaf_engine::scene::components::Transform>(object).cloned();
                if current.as_ref() != Some(&moved) {
                    editor.dispatch("scene.component.set", &serde_json::json!({ "id": object, "component": "transform", "value": moved }), Some(&key));
                }
            }
        }
        if !response.dragged() {
            view.gizmo_drag = None;
            editor.gesture += 1;
        }
    }
    let icons = scene_icons(editor, &projector);
    paint_icons(ui, &icons, editor.selected, response.rect);
    if let Some((_, world, _, _)) = &target {
        let active = view.gizmo_drag.as_ref().map(|d| d.handle).or(hovered);
        paint_handles(ui, &projector, *world, view.tool, active, response.rect);
    }

    // Mouse. In play mode the view is the game's: the pointer and buttons go to the scripts, and
    // the left button no longer orbits or selects (the right and middle buttons still move the view).
    let playing = editor.session.is_playing();
    if playing {
        send_pointer(editor, ui, &response);
    }
    // While a handle is dragged, the view stays put.
    if view.gizmo_drag.is_none() {
        move_view(view, ui, &response, size, playing);
    }
    // A press and release that barely moved is a click, even if egui already counted it as a drag.
    let tiny_drag = response.drag_stopped_by(egui::PointerButton::Primary)
        && ui.input(|i| i.pointer.press_origin().zip(i.pointer.interact_pos()).is_some_and(|(a, b)| a.distance(b) < 5.0));
    if playing && (response.clicked() || tiny_drag) {
        response.request_focus();
    } else if response.clicked() || tiny_drag {
        response.request_focus();
        if let Some(pos) = response.interact_pointer_pos() {
            let local = (pos - response.rect.min) / response.rect.size();
            let ndc = Vec2::new(local.x * 2.0 - 1.0, 1.0 - local.y * 2.0);
            // Camera and light icons sit in front of the shapes.
            let icon = icons.iter().filter(|i| i.center.distance(pos) <= ICON_REACH).min_by(|a, b| a.center.distance(pos).total_cmp(&b.center.distance(pos)));
            let picked = icon.map(|i| i.id).or_else(|| bayleaf_render::pick(&frame, ndc));
            // Ctrl + click adds to the selection or takes out of it.
            match (picked, ui.input(|i| i.modifiers.command)) {
                (Some(id), true) => editor.toggle_selected(id),
                (_, true) => {}
                (picked, false) => editor.select_only(picked),
            }
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
        for tool in gizmo::Tool::ALL {
            if ui.input(|i| i.modifiers.is_none() && i.key_pressed(tool.key())) {
                view.tool = tool;
            }
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

/// Gives the scripts the pointer over the view and the mouse buttons held there.
fn send_pointer(editor: &mut Editor, ui: &egui::Ui, response: &egui::Response) {
    let pointer = response.hover_pos().map(|p| {
        let local = (p - response.rect.min) / response.rect.size();
        [local.x * 2.0 - 1.0, 1.0 - local.y * 2.0]
    });
    let buttons = ui.input(|i| u8::from(i.pointer.primary_down()) | u8::from(i.pointer.secondary_down()) << 1 | u8::from(i.pointer.middle_down()) << 2);
    // Buttons count only while the pointer is over the view.
    editor.session.set_pointer(pointer, if pointer.is_some() { buttons } else { 0 });
}

/// Orbit, pan and zoom with the mouse. In play mode the left button belongs to the game.
fn move_view(view: &mut SceneView, ui: &egui::Ui, response: &egui::Response, size: egui::Vec2, playing: bool) {
    let shift = ui.input(|i| i.modifiers.shift);
    let delta = response.drag_delta() / size.y;
    if response.dragged_by(egui::PointerButton::Middle) || (response.dragged_by(egui::PointerButton::Primary) && shift && !playing) {
        view.orbit.pan(Vec2::new(delta.x, delta.y));
    } else if (response.dragged_by(egui::PointerButton::Primary) && !playing) || response.dragged_by(egui::PointerButton::Secondary) {
        view.orbit.rotate(-delta.x * 3.0, delta.y * 3.0);
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            view.orbit.zoom((-scroll * 0.0025).exp());
        }
    }
}

/// The selected camera's view in the bottom-right corner, framed and labelled.
fn paint_preview(ui: &egui::Ui, texture: egui::TextureId, view: egui::Rect, size: egui::Vec2) {
    let t = crate::theme::tokens(ui);
    let rect = egui::Rect::from_min_size(view.right_bottom() - size - egui::vec2(12.0, 12.0), size);
    let painter = ui.painter_at(view);
    painter.rect_filled(rect.expand(3.0), crate::theme::RADIUS, t.backdrop);
    painter.image(texture, rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    let label = egui::Rect::from_min_size(rect.min, egui::vec2(112.0, 18.0));
    painter.rect_filled(label, egui::CornerRadius { nw: 2, ne: 0, sw: 0, se: crate::theme::RADIUS }, t.backdrop);
    painter.text(label.left_center() + egui::vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, "Camera preview", egui::FontId::proportional(12.0), t.text);
}

/// How close a click must be to a camera or light icon to pick it, in points.
const ICON_REACH: f32 = 12.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum IconKind {
    Camera,
    Light,
    /// Point and spot lights.
    Bulb,
}

/// An object without a shape, drawn as an icon so it can be seen and clicked.
struct Icon {
    id: bayleaf_core::StableId,
    kind: IconKind,
    center: egui::Pos2,
    /// Where the object looks or shines, on screen, when that point is in front of the camera.
    facing: Option<egui::Pos2>,
}

fn scene_icons(editor: &Editor, projector: &gizmo::Projector) -> Vec<Icon> {
    use bayleaf_engine::scene::components::{Camera, DirectionalLight, PointLight, SpotLight};
    let scene = editor.session.scene();
    scene
        .world_matrices()
        .into_iter()
        .filter_map(|(id, world)| {
            let kind = if scene.get::<Camera>(id).is_some() {
                IconKind::Camera
            } else if scene.get::<DirectionalLight>(id).is_some() {
                IconKind::Light
            } else if scene.get::<PointLight>(id).is_some() || scene.get::<SpotLight>(id).is_some() {
                IconKind::Bulb
            } else {
                return None;
            };
            let origin = world.w_axis.truncate();
            let center = projector.project(origin)?;
            let forward = world.transform_vector3(Vec3::NEG_Z).normalize_or(Vec3::NEG_Z);
            let facing = projector.project(origin + forward * gizmo::handle_length(projector, origin) * 1.2);
            Some(Icon { id, kind, center, facing })
        })
        .collect()
}

/// Cameras as a body with a lens, lights as a sun; the selected one in the accent colour with a
/// line toward where it looks or shines.
fn paint_icons(ui: &egui::Ui, icons: &[Icon], selected: Option<bayleaf_core::StableId>, clip: egui::Rect) {
    let t = crate::theme::tokens(ui);
    let painter = ui.painter_at(clip);
    let edge = egui::Stroke::new(1.5, t.backdrop);
    for icon in icons {
        let is_selected = selected == Some(icon.id);
        let color = if is_selected { t.accent } else { t.text };
        let c = icon.center;
        if is_selected && let Some(facing) = icon.facing {
            painter.line_segment([c, facing], egui::Stroke::new(3.0, t.backdrop));
            painter.line_segment([c, facing], egui::Stroke::new(1.5, color));
        }
        match icon.kind {
            IconKind::Camera => {
                let body = egui::Rect::from_center_size(c - egui::vec2(3.0, 0.0), egui::vec2(16.0, 11.0));
                painter.rect(body, 2.0, color, edge, egui::StrokeKind::Outside);
                let lens = vec![body.right_center(), body.right_center() + egui::vec2(7.0, -6.0), body.right_center() + egui::vec2(7.0, 6.0)];
                painter.add(egui::Shape::convex_polygon(lens, color, edge));
            }
            IconKind::Bulb => {
                // A bulb: a round glass on a small base.
                painter.circle(c - egui::vec2(0.0, 2.0), 6.5, color, edge);
                painter.rect(egui::Rect::from_center_size(c + egui::vec2(0.0, 6.5), egui::vec2(6.0, 4.0)), 1.0, color, edge, egui::StrokeKind::Outside);
            }
            IconKind::Light => {
                for i in 0..8 {
                    let a = i as f32 / 8.0 * std::f32::consts::TAU;
                    let d = egui::vec2(a.cos(), a.sin());
                    painter.line_segment([c + d * 8.0, c + d * 12.0], egui::Stroke::new(3.5, t.backdrop));
                    painter.line_segment([c + d * 8.0, c + d * 12.0], egui::Stroke::new(2.0, color));
                }
                painter.circle(c, 5.5, color, edge);
            }
        }
    }
}

/// Draws the handles of `tool` for an object at `world`; `active` is drawn thicker.
fn paint_handles(
    ui: &egui::Ui,
    projector: &gizmo::Projector,
    world: bayleaf_core::math::Mat4,
    tool: gizmo::Tool,
    active: Option<gizmo::Handle>,
    clip: egui::Rect,
) {
    let t = crate::theme::tokens(ui);
    let painter = ui.painter_at(clip);
    for (axis, color) in [t.axis_x, t.axis_y, t.axis_z].into_iter().enumerate() {
        let handle = gizmo::Handle { tool, axis };
        let points = gizmo::handle_points(projector, world, handle);
        if points.len() < 2 {
            continue;
        }
        let hot = active == Some(handle);
        let stroke = egui::Stroke::new(if hot { 4.5 } else { 2.5 }, color);
        // A dark edge under each handle keeps it visible on light and dark surfaces alike.
        painter.add(egui::Shape::line(points.clone(), egui::Stroke::new(stroke.width + 2.0, t.backdrop.gamma_multiply(0.7))));
        painter.add(egui::Shape::line(points.clone(), stroke));
        let (Some(&end), Some(&before)) = (points.last(), points.get(points.len().saturating_sub(2))) else { continue };
        let dir = (end - before).normalized();
        let side = egui::vec2(-dir.y, dir.x);
        let size = if hot { 9.0 } else { 7.0 };
        match tool {
            gizmo::Tool::Move => {
                let tip = end + dir * size * 1.6;
                painter.add(egui::Shape::convex_polygon(vec![tip, end + side * size, end - side * size], color, egui::Stroke::new(1.0, t.backdrop)));
            }
            gizmo::Tool::Scale => {
                painter.rect(
                    egui::Rect::from_center_size(end, egui::vec2(size, size) * 1.4),
                    1.0,
                    color,
                    egui::Stroke::new(1.0, t.backdrop),
                    egui::StrokeKind::Outside,
                );
            }
            gizmo::Tool::Rotate => {}
        }
    }
    if let Some(center) = projector.project(world.w_axis.truncate()) {
        painter.circle(center, 3.5, t.text, egui::Stroke::new(1.0, t.backdrop));
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
