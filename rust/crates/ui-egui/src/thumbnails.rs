//! Material pictures for the Project panel and the Inspector: each material drawn on a sphere off
//! screen, a few per frame so the editor stays responsive, and kept until its file or an image it
//! uses changes. A material that does not compile keeps its reason instead of a picture.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use bayleaf_render::{MaterialLibrary, RenderTarget, Renderer};

/// Pixels per side of a picture.
pub(crate) const PIXELS: u32 = 192;
/// Pictures drawn in one frame at most.
const PER_FRAME: usize = 2;
/// How often a shown picture's files are looked at for changes.
const CHECK_EVERY: Duration = Duration::from_secs(1);
/// A picture not asked for in this long is let go.
const FORGET_AFTER: Duration = Duration::from_secs(120);
/// Most pictures kept at once; the longest unused go first.
const MAX_KEPT: usize = 256;

/// What a panel can show for a material.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Thumb {
    /// Not drawn yet: show the kind's icon.
    Waiting,
    /// It compiles, but there is no graphics adapter to draw it with: show the kind's icon.
    Unavailable,
    Ready(egui::TextureId),
    /// Why the material does not compile (or its shader was refused).
    Failed(String),
}

/// The files a picture was drawn from, with their change times.
type Stamp = Vec<(PathBuf, Option<SystemTime>)>;

struct Entry {
    thumb: Thumb,
    /// What the picture was drawn from; `None` until it is.
    stamp: Option<Stamp>,
    /// The files changed since, so it is drawn again (the old picture shows meanwhile).
    stale: bool,
    checked: Instant,
    asked: Instant,
}

/// Every material picture, by file.
#[derive(Default)]
pub(crate) struct Thumbnails {
    entries: HashMap<PathBuf, Entry>,
    materials: MaterialLibrary,
    gpu: Option<(Renderer, RenderTarget)>,
}

impl Thumbnails {
    /// The picture of the material at `path`, asking for it to be drawn when there is none yet.
    pub(crate) fn get(&mut self, path: &Path) -> Thumb {
        let now = Instant::now();
        let entry =
            self.entries.entry(path.to_path_buf()).or_insert_with(|| Entry { thumb: Thumb::Waiting, stamp: None, stale: true, checked: now, asked: now });
        entry.asked = now;
        entry.thumb.clone()
    }

    /// Draws the pictures asked for that are missing or out of date, a few at a time. Paths in a
    /// material are relative to `base`; `images` caches the images they name.
    pub(crate) fn update(&mut self, ctx: &egui::Context, gpu: Option<&egui_wgpu::RenderState>, base: &Path, images: &mut bayleaf_render::assets::AssetCache) {
        let now = Instant::now();
        self.forget_unused(gpu, now);
        for entry in self.entries.values_mut() {
            if !entry.stale && now.duration_since(entry.checked) >= CHECK_EVERY {
                entry.checked = now;
                entry.stale = entry.stamp.as_ref().is_none_or(|stamp| stamp.iter().any(|(path, seen)| modified(path) != *seen));
            }
        }
        let mut due: Vec<PathBuf> = self.entries.iter().filter(|(_, e)| e.stale).map(|(p, _)| p.clone()).collect();
        // The most recently asked for first.
        due.sort_by_key(|p| std::cmp::Reverse(self.entries[p].asked));
        let more = due.len() > PER_FRAME;
        for path in due.into_iter().take(PER_FRAME) {
            self.draw(&path, gpu, base, images);
        }
        if more {
            ctx.request_repaint();
        } else if !self.entries.is_empty() {
            ctx.request_repaint_after(CHECK_EVERY);
        }
    }

    /// Compiles the material at `path` and draws it, keeping what it was drawn from.
    fn draw(&mut self, path: &Path, gpu: Option<&egui_wgpu::RenderState>, base: &Path, images: &mut bayleaf_render::assets::AssetCache) {
        let mut stamp = vec![(path.to_path_buf(), modified(path))];
        let old = match self.entries.get(path).map(|e| &e.thumb) {
            Some(Thumb::Ready(id)) => Some(*id),
            _ => None,
        };
        let thumb = match self.materials.get(path) {
            Err(error) => Thumb::Failed(error),
            Ok(material) => {
                let files: Vec<PathBuf> = material.textures.iter().map(|t| base.join(t)).collect();
                stamp.extend(files.iter().map(|f| (f.clone(), modified(f))));
                match gpu {
                    None => Thumb::Unavailable,
                    Some(gpu) => {
                        let textures = files.iter().map(|f| images.image(f).ok()).collect();
                        self.render(gpu, material, textures, old)
                    }
                }
            }
        };
        // A picture that is no longer shown lets its texture go.
        if let (Some(old), Some(gpu)) = (old, gpu)
            && thumb != Thumb::Ready(old)
        {
            gpu.renderer.write().free_texture(&old);
        }
        if let Some(entry) = self.entries.get_mut(path) {
            entry.thumb = thumb;
            entry.stamp = Some(stamp);
            entry.stale = false;
            entry.checked = Instant::now();
        }
    }

    /// Draws `material` on a sphere and keeps the picture in `old`'s texture, or a new one.
    fn render(
        &mut self,
        gpu: &egui_wgpu::RenderState,
        material: std::sync::Arc<bayleaf_render::CompiledMaterial>,
        textures: Vec<Option<std::sync::Arc<bayleaf_render::assets::Image>>>,
        old: Option<egui::TextureId>,
    ) -> Thumb {
        let frame = bayleaf_render::material_preview(std::sync::Arc::clone(&material), textures, 1.0, 0.0);
        let (renderer, target) = self.gpu.get_or_insert_with(|| (Renderer::new(&gpu.device), RenderTarget::new(&gpu.device, PIXELS, PIXELS)));
        renderer.render(&gpu.device, &gpu.queue, target, &frame);
        if renderer.refused(&material) {
            return Thumb::Failed(format!("the graphics adapter refused its shader: {}", renderer.material_error().unwrap_or_default()));
        }
        let view = target.snapshot(&gpu.device, &gpu.queue);
        let filter = bayleaf_render::wgpu::FilterMode::Linear;
        let mut egui_renderer = gpu.renderer.write();
        match old {
            Some(id) => {
                egui_renderer.update_egui_texture_from_wgpu_texture(&gpu.device, &view, filter, id);
                Thumb::Ready(id)
            }
            None => Thumb::Ready(egui_renderer.register_native_texture(&gpu.device, &view, filter)),
        }
    }

    /// Lets go of pictures not asked for in a while, and of the oldest beyond the limit.
    fn forget_unused(&mut self, gpu: Option<&egui_wgpu::RenderState>, now: Instant) {
        let mut gone: Vec<PathBuf> = self.entries.iter().filter(|(_, e)| now.duration_since(e.asked) > FORGET_AFTER).map(|(p, _)| p.clone()).collect();
        if self.entries.len() - gone.len() > MAX_KEPT {
            let mut kept: Vec<(&PathBuf, Instant)> = self.entries.iter().filter(|(p, _)| !gone.contains(p)).map(|(p, e)| (p, e.asked)).collect();
            kept.sort_by_key(|(_, asked)| *asked);
            let extra = kept.len() - MAX_KEPT;
            gone.extend(kept.into_iter().take(extra).map(|(p, _)| p.clone()));
        }
        for path in gone {
            if let Some(Entry { thumb: Thumb::Ready(id), .. }) = self.entries.remove(&path)
                && let Some(gpu) = gpu
            {
                gpu.renderer.write().free_texture(&id);
            }
        }
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// A warning mark, a triangle with an exclamation mark, in the bottom-right corner of `rect`.
pub(crate) fn paint_warning(painter: &egui::Painter, rect: egui::Rect, t: &crate::theme::Tokens) {
    let s = (rect.width() * 0.42).clamp(8.0, 22.0);
    let corner = rect.right_bottom();
    let (left, right, top) = (corner + egui::vec2(-s, 0.0), corner, corner + egui::vec2(-s / 2.0, -s * 0.9));
    painter.add(egui::Shape::convex_polygon(vec![top, right, left], t.warning, egui::Stroke::new(1.0, t.on_warning)));
    let x = top.x;
    let stroke = egui::Stroke::new((s / 8.0).max(1.0), t.on_warning);
    painter.line_segment([egui::pos2(x, top.y + s * 0.3), egui::pos2(x, top.y + s * 0.6)], stroke);
    painter.circle_filled(egui::pos2(x, top.y + s * 0.75), (s / 14.0).max(0.8), t.on_warning);
}
