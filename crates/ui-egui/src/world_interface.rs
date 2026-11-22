//! Interfaces standing in the world: an Interface with `space: world` is a flat panel at its
//! object, turned with it (or toward the camera), `size` world units wide and high. Each panel's
//! interface is drawn by an egui context of its own into a picture on the GPU, and the scene draws
//! the picture on the panel, so the panel hides behind what stands in front of it. The pointer
//! reaches it through the scene: a ray from the pointer finds the nearest panel, and the spot it
//! hits is where the panel's own pointer is, so its buttons, toggles, sliders and On Click
//! elements answer as on the screen and the scripts hear the same messages.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use bayleaf_core::StableId;
use bayleaf_core::math::{DMat4, DVec3, Vec2, Vec3, Vec4};
use bayleaf_engine::Session;
use bayleaf_engine::scene::components::{Interface, MAX_PANEL_POINTS};
use bayleaf_render::wgpu;
use bayleaf_render::{CameraView, DrawItem, Renderer};

use crate::game_interface::GameInterfaces;

/// The pictures are sRGB with alpha, as images are.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// One panel's own egui context and the picture it draws into.
struct Panel {
    ctx: egui::Context,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// The picture's size, in points (one pixel each).
    points: [u32; 2],
    /// Stands for the picture in a draw item: the renderer draws the texture of this id.
    image: Arc<bayleaf_render::assets::Image>,
    /// The pointer was over it last frame.
    hovered: bool,
}

/// The world panels of the scene a view shows, and the renderer that draws their interfaces.
#[derive(Default)]
pub struct WorldInterfaces {
    painter: Option<egui_wgpu::Renderer>,
    panels: HashMap<StableId, Panel>,
    /// The panel the keyboard and the gamepad drive: the one last pressed on, until a press lands
    /// elsewhere.
    keyboard: Option<StableId>,
}

impl WorldInterfaces {
    /// Whether a world panel has the keyboard and the gamepad now, so the screen's interface does not.
    pub fn has_keyboard(&self) -> bool {
        self.keyboard.is_some()
    }
}

/// The pointer as the view saw it this frame: where, as -1 to 1 across the view, and its button
/// presses and releases.
pub struct PointerInput {
    pub ndc: Option<Vec2>,
    pub buttons: Vec<(egui::PointerButton, bool)>,
    /// Keys and typed text this frame, for the panel that has the keyboard.
    pub keys: Vec<egui::Event>,
}

impl PointerInput {
    /// The pointer over `rect` (a view's picture) as `ui` saw it this frame.
    pub fn of(ui: &egui::Ui, rect: egui::Rect) -> Self {
        let ndc = ui.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p)).map(|p| {
            let local = (p - rect.min) / rect.size();
            Vec2::new(local.x * 2.0 - 1.0, 1.0 - local.y * 2.0)
        });
        let buttons = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::PointerButton { button, pressed, .. } => Some((*button, *pressed)),
                    _ => None,
                })
                .collect()
        });
        let keys = ui.input(|i| i.events.iter().filter(|e| matches!(e, egui::Event::Key { .. } | egui::Event::Text(_))).cloned().collect());
        Self { ndc, buttons, keys }
    }
}

/// What a view gives to draw the world panels with.
pub struct View<'a> {
    pub gpu: &'a egui_wgpu::RenderState,
    pub renderer: &'a mut Renderer,
    pub camera: &'a CameraView,
    pub pointer: &'a PointerInput,
    /// The panels answer the pointer (while the game plays).
    pub interactive: bool,
}

/// A panel's place in the world: at its object, turned with it or toward the eye, sized.
fn panel_matrix(world: DMat4, component: &Interface, eye: DVec3) -> DMat4 {
    let size = DMat4::from_scale(DVec3::new(f64::from(component.size[0]), f64::from(component.size[1]), 1.0));
    if !component.face_camera {
        return world * size;
    }
    let at = world.w_axis.truncate();
    let toward = (eye - at).try_normalize().unwrap_or(DVec3::Z);
    let side = DVec3::Y.cross(toward).try_normalize().unwrap_or(DVec3::X);
    let up = toward.cross(side);
    DMat4::from_cols(side.extend(0.0), up.extend(0.0), toward.extend(0.0), at.extend(1.0)) * size
}

/// Where a ray (measured from the view's origin) meets a panel whose matrix is `model`: how far
/// along the ray, and the spot as 0 to 1 across and down the panel.
fn hit(model: bayleaf_core::math::Mat4, origin: Vec3, direction: Vec3) -> Option<(f32, Vec2)> {
    let inverse = model.inverse();
    let (o, d) = (inverse.transform_point3(origin), inverse.transform_vector3(direction));
    if d.z.abs() < 1e-9 {
        return None;
    }
    let t = -o.z / d.z;
    let p = o + d * t;
    (t > 0.0 && p.x.abs() <= 0.5 && p.y.abs() <= 0.5).then(|| (t, Vec2::new(p.x + 0.5, 0.5 - p.y)))
}

/// The picture's id, apart from images read from files (theirs are hashes of their paths).
fn picture_id(object: StableId, points: [u32; 2]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    ("world interface", object.as_u128(), points).hash(&mut hasher);
    hasher.finish() | 1
}

impl WorldInterfaces {
    /// Draws every world interface of the scene into its picture and adds a panel showing it to
    /// `items`. With `interactive`, the pointer reaches the panel it points at. Returns whether the
    /// pointer is over an element that takes it, so the game does not also get its clicks.
    pub fn prepare(&mut self, view: View<'_>, session: &mut Session, cache: &mut GameInterfaces, items: &mut Vec<DrawItem>) -> bool {
        let View { gpu, renderer, camera, pointer, interactive } = view;
        let scene = session.scene();
        let list: Vec<(StableId, Interface, DMat4)> = scene
            .world_matrices()
            .into_iter()
            .filter_map(|(id, world)| scene.get::<Interface>(id).filter(|c| c.space == "world" && !c.layout.is_empty()).map(|c| (id, c.clone(), world)))
            .filter(|(id, c, _)| session.interface_state(*id).and_then(|s| s.visible).unwrap_or(c.visible))
            .collect();
        self.panels.retain(|id, _| list.iter().any(|(l, _, _)| l == id));
        let eye = camera.origin + camera.position.as_dvec3();
        let ray = pointer.ndc.map(|ndc| camera.ray(ndc));
        // Each panel's matrix, picture size, and where the pointer's ray meets it.
        let mut placed = Vec::new();
        let mut nearest: Option<(f32, StableId, Vec2)> = None;
        for (id, component, world) in list {
            let model = camera.relative(panel_matrix(world, &component, eye));
            let points = component.size.map(|s| (s * component.points_per_unit).clamp(16.0, MAX_PANEL_POINTS).round() as u32);
            if let Some(ray) = ray.filter(|_| interactive)
                && let Some((t, spot)) = hit(model, ray.origin, ray.direction)
                && nearest.is_none_or(|(best, _, _)| t < best)
            {
                nearest = Some((t, id, spot));
            }
            placed.push((id, component, model, points));
        }
        // Something in the scene in front of the panel keeps the pointer from it.
        if let (Some((t, id, _)), Some(ray)) = (nearest, ray) {
            let others: Vec<DrawItem> = items.iter().filter(|i| i.object != id).cloned().collect();
            if bayleaf_render::nearest_hit(&others, ray).is_some_and(|(_, wall)| wall < t) {
                nearest = None;
            }
        }
        // A press lands on a panel (it then has the keyboard and the gamepad) or elsewhere.
        if interactive && pointer.ndc.is_some() && pointer.buttons.iter().any(|(_, pressed)| *pressed) {
            self.keyboard = nearest.map(|(_, id, _)| id);
        }
        if self.keyboard.is_some_and(|id| !self.panels.contains_key(&id) && !placed.iter().any(|(p, ..)| *p == id)) {
            self.keyboard = None;
        }
        let keyboard = self.keyboard;
        let painter = self.painter.get_or_insert_with(|| egui_wgpu::Renderer::new(&gpu.device, FORMAT, egui_wgpu::RendererOptions::default()));
        let mut blocked = false;
        for (id, component, model, points) in placed {
            let panel = self.panels.entry(id).or_insert_with(|| new_panel(&gpu.device, id, points));
            if panel.points != points {
                *panel = new_panel(&gpu.device, id, points);
            }
            // The pointer where the ray meets the panel, else gone from it.
            let mut events = Vec::new();
            let over = nearest.filter(|(_, at, _)| *at == id).map(|(_, _, spot)| egui::pos2(spot.x * points[0] as f32, spot.y * points[1] as f32));
            match over {
                Some(at) => {
                    events.push(egui::Event::PointerMoved(at));
                    for (button, pressed) in &pointer.buttons {
                        events.push(egui::Event::PointerButton { pos: at, button: *button, pressed: *pressed, modifiers: egui::Modifiers::NONE });
                    }
                }
                None if panel.hovered => events.push(egui::Event::PointerGone),
                None => {}
            }
            let has_keyboard = interactive && keyboard == Some(id);
            if has_keyboard {
                events.extend(pointer.keys.iter().cloned());
            }
            panel.hovered = over.is_some();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(points[0] as f32, points[1] as f32));
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let mut blocks_here = false;
            let output = panel.ctx.run_ui(input, |ui| {
                let one = vec![(id, component.clone())];
                // Only the panel that has the keyboard follows the gamepad.
                blocks_here = if has_keyboard {
                    crate::game_interface::show_list(ui, screen, session, cache, interactive, one)
                } else {
                    cache.without_pad(|cache| crate::game_interface::show_list(ui, screen, session, cache, interactive, one))
                };
            });
            blocked |= blocks_here && over.is_some();
            paint(gpu, painter, panel, output);
            renderer.set_texture(panel.image.id, panel.texture.clone(), panel.view.clone());
            items.push(DrawItem {
                object: id,
                shape: bayleaf_engine::scene::components::Shape::Quad,
                model,
                color: Vec4::ONE,
                metallic: 0.0,
                roughness: 1.0,
                unlit: true,
                material: None,
                mesh: None,
                textures: vec![Some(Arc::clone(&panel.image))],
                part: None,
                additive: false,
                uv_rect: bayleaf_render::WHOLE_IMAGE,
            });
        }
        blocked
    }
}

fn new_panel(device: &wgpu::Device, id: StableId, points: [u32; 2]) -> Panel {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bayleaf world interface"),
        size: wgpu::Extent3d { width: points[0], height: points[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let ctx = egui::Context::default();
    crate::fonts::install_fonts(&ctx);
    let image = Arc::new(bayleaf_render::assets::Image { id: picture_id(id, points), width: 1, height: 1, rgba: vec![0; 4], mipmaps: false });
    Panel { ctx, texture, view, points, image, hovered: false }
}

/// Puts what the panel's context drew into its picture.
fn paint(gpu: &egui_wgpu::RenderState, painter: &mut egui_wgpu::Renderer, panel: &Panel, mut output: egui::FullOutput) {
    // The texture changes are taken out and applied: none is left behind.
    let mut textures = std::mem::take(&mut output.textures_delta);
    let jobs = panel.ctx.tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point);
    let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: panel.points, pixels_per_point: output.pixels_per_point };
    for (id, deltas) in textures.set.drain() {
        for delta in &deltas {
            painter.update_texture(&gpu.device, &gpu.queue, id, delta);
        }
    }
    let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("bayleaf world interface") });
    let extra = painter.update_buffers(&gpu.device, &gpu.queue, &mut encoder, &jobs, &screen);
    {
        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("bayleaf world interface"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &panel.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let mut pass = pass.forget_lifetime();
        painter.render(&mut pass, &jobs, &screen);
    }
    gpu.queue.submit(extra.into_iter().chain(std::iter::once(encoder.finish())));
    for id in textures.free.drain() {
        painter.free_texture(&id);
    }
}
