//! Game interfaces drawn over the game, in the Game view and in the player: every visible
//! Interface component's layout, lowest order first. While the game plays, buttons, toggles,
//! sliders and text fields answer the pointer and the keyboard (Tab moves the focus, Space or
//! Enter presses, arrow keys move a slider), and what the player does goes to the object's
//! scripts. Each element is a node in the accessibility tree with its role and its text, so the
//! game's interface reads like the editor's. Outside play mode the interface shows as it is
//! written, as a preview.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use bayleaf_core::StableId;
use bayleaf_engine::Session;
use bayleaf_engine::interface::{Input, Loaded};
use bayleaf_engine::scene::components::Interface;
use bayleaf_interface::{Color, Kind, Placed, TextAlign, TextMeasure};

/// Layout and style files read, images on the GPU, and what is being typed into text fields.
#[derive(Default)]
pub struct GameInterfaces {
    /// By layout path: when the layout and its style files last changed, and what they read as.
    files: HashMap<PathBuf, Cached>,
    /// By image path: when it changed, and its picture (or `None` when it could not be read).
    images: HashMap<PathBuf, (Option<SystemTime>, Option<egui::TextureHandle>)>,
    /// A text field's text while the player types, by object and element name.
    typing: HashMap<(StableId, String), String>,
}

struct Cached {
    stamps: Vec<Option<SystemTime>>,
    styles: Vec<PathBuf>,
    loaded: Result<Arc<Loaded>, String>,
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl GameInterfaces {
    /// A layout and its styles, read again when any of the files changed.
    fn load(&mut self, base: &Path, layout: &str) -> Result<Arc<Loaded>, String> {
        let path = base.join(layout);
        if let Some(cached) = self.files.get(&path) {
            let now: Vec<Option<SystemTime>> = std::iter::once(&path).chain(&cached.styles).map(|p| modified(p)).collect();
            if now == cached.stamps && now.iter().all(Option::is_some) {
                return cached.loaded.clone();
            }
        }
        let loaded = bayleaf_engine::interface::load(base, layout).map(Arc::new);
        // Its style and part files: a change to any of them reads it again.
        let styles: Vec<PathBuf> = loaded.as_ref().map(|l| l.files.iter().map(|s| base.join(s)).collect()).unwrap_or_default();
        let stamps = std::iter::once(&path).chain(&styles).map(|p| modified(p)).collect();
        self.files.insert(path, Cached { stamps, styles, loaded: loaded.clone() });
        loaded
    }

    /// Puts every image the tree shows on the GPU, once each (again when the file changes).
    fn load_images(&mut self, ctx: &egui::Context, base: &Path, root: &bayleaf_interface::Element) {
        for element in root.walk().into_iter().filter(|e| !e.image.is_empty()) {
            let path = base.join(&element.image);
            let stamp = modified(&path);
            if self.images.get(&path).is_some_and(|(seen, _)| *seen == stamp) {
                continue;
            }
            let texture = bayleaf_render::assets::load_image(&path).ok().map(|image| {
                let picture = egui::ColorImage::from_rgba_unmultiplied([image.width as usize, image.height as usize], &image.rgba);
                ctx.load_texture(path.display().to_string(), picture, egui::TextureOptions::LINEAR)
            });
            self.images.insert(path, (stamp, texture));
        }
    }
}

/// Text sizes from egui's fonts, image sizes from the pictures loaded.
struct Measure<'a> {
    painter: &'a egui::Painter,
    images: &'a HashMap<PathBuf, (Option<SystemTime>, Option<egui::TextureHandle>)>,
    base: &'a Path,
}

impl TextMeasure for Measure<'_> {
    fn text(&self, text: &str, font_size: f32, wrap: Option<f32>) -> [f32; 2] {
        if text.is_empty() {
            return [0.0, 0.0];
        }
        let galley = self.painter.layout(text.to_owned(), egui::FontId::proportional(font_size), egui::Color32::WHITE, wrap.unwrap_or(f32::INFINITY));
        [galley.size().x.ceil(), galley.size().y.ceil()]
    }

    fn image(&self, path: &str) -> Option<[f32; 2]> {
        let texture = self.images.get(&self.base.join(path))?.1.as_ref()?;
        let size = texture.size_vec2();
        Some([size.x, size.y])
    }
}

fn color32(color: Color, opacity: f32) -> egui::Color32 {
    let [r, g, b, a] = color.0;
    egui::Color32::from_rgba_unmultiplied(r, g, b, (f32::from(a) * opacity.clamp(0.0, 1.0)).round() as u8)
}

/// Draws every visible Interface of the scene inside `area`. With `interactive`, its controls
/// answer the player and the session hears about it. Returns whether the pointer is over an
/// element that takes it, so the game does not also get its clicks.
pub fn show(ui: &mut egui::Ui, area: egui::Rect, session: &mut Session, cache: &mut GameInterfaces, interactive: bool) -> bool {
    let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|p| area.contains(*p));
    let mut blocked = false;
    let base = session.script_dir();
    let scene = session.scene();
    let mut list: Vec<(StableId, Interface)> = scene
        .world_matrices()
        .into_iter()
        .filter_map(|(id, _)| scene.get::<Interface>(id).cloned().map(|c| (id, c)))
        .filter(|(_, c)| !c.layout.is_empty())
        .collect();
    list.sort_by_key(|(_, c)| c.order);
    let game = session.game_state();
    let mut inputs: Vec<(StableId, String, Input)> = Vec::new();
    let mut problems: Vec<String> = Vec::new();
    let painter = ui.painter_at(area);
    for (object, component) in list {
        let state = session.interface_state(object);
        if !state.and_then(|s| s.visible).unwrap_or(component.visible) {
            continue;
        }
        let loaded = match cache.load(&base, &component.layout) {
            Ok(loaded) => loaded,
            Err(problem) => {
                problems.push(problem);
                continue;
            }
        };
        let root = bayleaf_engine::interface::shown(&loaded.root, state, game.as_ref());
        cache.load_images(ui.ctx(), &base, &root);
        let placed = {
            let measure = Measure { painter: &painter, images: &cache.images, base: &base };
            bayleaf_interface::layout(&root, &loaded.styles, [area.width(), area.height()], 1.0, &measure)
        };
        for (index, item) in placed.iter().enumerate() {
            if let Some(p) = pointer
                && !blocked
                && to_screen(item.clip, area).contains(p)
                && bayleaf_interface::blocks_pointer(item.element, &item.style)
            {
                blocked = true;
            }
            let drawing = Drawing { painter: &painter, area, object, base: &base, interactive };
            draw(ui, &drawing, index, item, cache, &mut inputs);
        }
    }
    for (object, element, input) in inputs {
        if let Err(problem) = session.interface_input(object, &element, input) {
            problems.push(problem);
        }
    }
    // A layout or style that does not read says so over the game, instead of showing nothing.
    if !problems.is_empty() {
        let t = crate::theme::tokens(ui);
        let text = crate::i18n::tf("Interface: {problem}", &[("problem", &problems.join("; "))]);
        let galley = painter.layout(text, egui::FontId::proportional(13.0), t.on_warning, area.width() - 16.0);
        let at = area.left_top() + egui::vec2(8.0, 8.0);
        painter.rect_filled(egui::Rect::from_min_size(at, galley.size()).expand(4.0), 4.0, t.warning);
        painter.galley(at, galley, t.on_warning);
    }
    blocked
}

/// Screen sizes the preview offers: a name and width by height in points.
pub(crate) const PREVIEW_SCREENS: [(&str, [f32; 2]); 4] =
    [("1280 x 720", [1280.0, 720.0]), ("1920 x 1080", [1920.0, 1080.0]), ("800 x 600", [800.0, 600.0]), ("Phone, upright (390 x 844)", [390.0, 844.0])];

/// A preview drawn: its box, and where each element landed with its place in the tree (child
/// indices from the root).
pub(crate) struct Previewed {
    pub area: egui::Rect,
    pub elements: Vec<(egui::Rect, Vec<usize>)>,
}

/// The layout file `layout` (relative to `base`) read, again when it or its style files change.
pub(crate) fn loaded(cache: &mut GameInterfaces, base: &Path, layout: &str) -> Result<Arc<Loaded>, String> {
    cache.load(base, layout)
}

/// Every element of `shown` (the layout with its parts in place) by its address, with its place
/// in `file`'s tree (as the file holds it): everything a part put in place is the part's.
fn places(file: &bayleaf_interface::Element, shown: &bayleaf_interface::Element) -> HashMap<*const bayleaf_interface::Element, Vec<usize>> {
    let mut out = HashMap::new();
    let mut pending = vec![(file, shown, Vec::new())];
    while let Some((file, shown, at)) = pending.pop() {
        if file.kind == bayleaf_interface::Kind::Part {
            for inner in shown.walk() {
                out.insert(std::ptr::from_ref(inner), at.clone());
            }
            continue;
        }
        for (i, (f, s)) in file.children.iter().zip(&shown.children).enumerate() {
            let mut child_at = at.clone();
            child_at.push(i);
            pending.push((f, s, child_at));
        }
        out.insert(std::ptr::from_ref(shown), at);
    }
    out
}

/// The layout file `layout` (relative to `base`) as a game `screen` points wide and high shows it,
/// made smaller to fit `room`. It does not answer the pointer: it is a picture of the layout.
pub(crate) fn preview(
    ui: &mut egui::Ui,
    room: egui::Vec2,
    screen: [f32; 2],
    base: &Path,
    layout: &str,
    cache: &mut GameInterfaces,
) -> Result<Previewed, String> {
    let loaded = cache.load(base, layout)?;
    let fit = (room.x / screen[0]).min(room.y / screen[1]).min(1.0);
    // Centred across the room it has.
    let size = egui::vec2(screen[0] * fit, screen[1] * fit);
    let (room_rect, _) = ui.allocate_exact_size(egui::vec2(room.x.min(ui.available_width()).max(size.x), size.y), egui::Sense::hover());
    let area = egui::Rect::from_center_size(egui::pos2(room_rect.center().x, room_rect.center().y), size);
    let painter = ui.painter_at(area);
    // A dark backdrop stands for the game behind the interface.
    painter.rect_filled(area, 2.0, egui::Color32::from_rgb(28, 32, 40));
    // Its edge, so the screen's size shows against the panel.
    ui.painter().rect_stroke(area, 2.0, egui::Stroke::new(1.0, crate::theme::tokens(ui).text_muted), egui::StrokeKind::Outside);
    let root = loaded.root.clone();
    cache.load_images(ui.ctx(), base, &root);
    let placed = {
        let measure = Measure { painter: &painter, images: &cache.images, base };
        bayleaf_interface::layout(&root, &loaded.styles, [area.width(), area.height()], fit, &measure)
    };
    let drawing = Drawing { painter: &painter, area, object: StableId::from_u128(0), base, interactive: false };
    let mut inputs = Vec::new();
    for (index, item) in placed.iter().enumerate() {
        draw(ui, &drawing, index, item, cache, &mut inputs);
    }
    let at = places(&loaded.layout.root, &root);
    let elements = placed.iter().filter_map(|p| Some((to_screen(p.rect, area), at.get(&std::ptr::from_ref(p.element))?.clone()))).collect();
    Ok(Previewed { area, elements })
}

fn to_screen(rect: bayleaf_interface::Rect, area: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_size(area.min + egui::vec2(rect.x, rect.y), egui::vec2(rect.w, rect.h))
}

/// Where one interface is drawn, and whether it answers the player.
struct Drawing<'a> {
    painter: &'a egui::Painter,
    area: egui::Rect,
    object: StableId,
    base: &'a Path,
    interactive: bool,
}

fn draw(ui: &mut egui::Ui, drawing: &Drawing<'_>, index: usize, item: &Placed<'_>, cache: &mut GameInterfaces, inputs: &mut Vec<(StableId, String, Input)>) {
    let Drawing { painter, area, object, base, interactive } = *drawing;
    let style = &item.style;
    let element = item.element;
    let rect = to_screen(item.rect, area);
    let clip = to_screen(item.clip, area).intersect(area);
    if clip.width() <= 0.0 || clip.height() <= 0.0 {
        return;
    }
    let painter = painter.with_clip_rect(clip);
    let fade = |c: Color| color32(c, style.opacity);
    let id = ui.scope_id().with(("game interface", object, index));
    let label = if element.text.is_empty() { element.name.clone() } else { element.text.clone() };
    // Any element with On Click answers the pointer and the keyboard, as a button does.
    let clickable = interactive && !element.on_click.is_empty() && element.kind != Kind::TextField;
    let control = clickable || (interactive && element.kind.is_control() && element.kind != Kind::TextField);
    let response = if control {
        let sense = if element.kind == Kind::Slider { egui::Sense::click_and_drag() } else { egui::Sense::click() };
        Some(ui.interact(clip, id, sense))
    } else if matches!(element.kind, Kind::Text | Kind::Image | Kind::Progress) || (element.kind.is_control() && element.kind != Kind::TextField) {
        // Read by screen readers even when it does not answer the pointer.
        Some(ui.interact(clip, id, egui::Sense::hover()))
    } else {
        None
    };

    // The box: its fill, lighter while hovered and darker while held, and its border.
    let mut fill = fade(style.background);
    if let Some(r) = response.as_ref().filter(|_| control && (element.kind == Kind::Button || clickable)) {
        if r.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if r.is_pointer_button_down_on() {
            fill = fill.gamma_multiply(0.75);
        } else if r.hovered() || r.has_focus() {
            fill = egui::Color32::from_rgba_unmultiplied(fill.r().saturating_add(24), fill.g().saturating_add(24), fill.b().saturating_add(24), fill.a());
        }
    }
    painter.rect_filled(rect, style.radius, fill);
    if style.border_width > 0.0 {
        painter.rect_stroke(rect, style.radius, egui::Stroke::new(style.border_width, fade(style.border_color)), egui::StrokeKind::Inside);
    }
    let inset = style.border_width;
    let inner = egui::Rect::from_min_max(
        rect.min + egui::vec2(style.padding[3] + inset, style.padding[0] + inset),
        rect.max - egui::vec2(style.padding[1] + inset, style.padding[2] + inset),
    );
    let font = egui::FontId::proportional(style.font_size);
    let text_at = |galley: &egui::Galley, within: egui::Rect, centred: bool| {
        let x = match style.text_align {
            TextAlign::Start => within.left(),
            TextAlign::Center => within.center().x - galley.size().x / 2.0,
            TextAlign::End => within.right() - galley.size().x,
        };
        let y = if centred { within.center().y - galley.size().y / 2.0 } else { within.top() };
        egui::pos2(x, y)
    };
    let range = (element.max - element.min).max(f64::EPSILON);
    match element.kind {
        Kind::Text | Kind::Button => {
            // A little room over the measured width, so rounding never wraps a line that fits.
            let galley = painter.layout(element.text.clone(), font, fade(style.color), inner.width().max(1.0) + 1.0);
            let at = text_at(&galley, inner, element.kind == Kind::Button);
            painter.galley(at, galley, fade(style.color));
            if let Some(r) = response.as_ref().filter(|_| control)
                && r.clicked()
            {
                inputs.push((object, element.name.clone(), Input::Pressed));
            }
        }
        Kind::Toggle => {
            let side = inner.height().min(style.font_size * 1.25);
            let mark = egui::Rect::from_min_size(egui::pos2(inner.left(), inner.center().y - side / 2.0), egui::vec2(side, side));
            let on = element.value != 0.0;
            if on {
                painter.rect_filled(mark, 4.0, fade(style.accent));
                // A tick, so the state does not rest on colour alone.
                let tick = [
                    mark.left_center() + egui::vec2(side * 0.22, 0.0),
                    mark.center_bottom() + egui::vec2(-side * 0.08, -side * 0.25),
                    mark.right_top() + egui::vec2(-side * 0.2, side * 0.25),
                ];
                painter.line(tick.to_vec(), egui::Stroke::new(2.0, egui::Color32::WHITE));
            } else {
                painter.rect_stroke(mark, 4.0, egui::Stroke::new(2.0, fade(style.color)), egui::StrokeKind::Inside);
            }
            let text = egui::Rect::from_min_max(egui::pos2(mark.right() + style.gap, inner.top()), inner.max);
            let galley = painter.layout(element.text.clone(), font, fade(style.color), text.width().max(1.0) + 1.0);
            painter.galley(text_at(&galley, text, true), galley, fade(style.color));
            if let Some(r) = response.as_ref().filter(|_| control)
                && r.clicked()
            {
                inputs.push((object, element.name.clone(), Input::Changed(if on { 0.0 } else { 1.0 })));
            }
        }
        Kind::Slider => {
            let share = ((element.value - element.min) / range).clamp(0.0, 1.0) as f32;
            let fill_rect = egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + rect.width() * share, rect.bottom()));
            painter.rect_filled(fill_rect, style.radius, fade(style.accent));
            let knob = egui::pos2(rect.left() + rect.width() * share, rect.center().y);
            painter.circle(knob, rect.height() * 0.5, egui::Color32::WHITE, egui::Stroke::new(1.0, fade(style.accent)));
            if let Some(r) = response.as_ref().filter(|_| control) {
                let mut value = None;
                if (r.dragged() || r.clicked())
                    && let Some(pointer) = r.interact_pointer_pos()
                {
                    let share = ((pointer.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
                    value = Some(element.min + range * f64::from(share));
                }
                if r.has_focus() {
                    // The arrow keys move the slider, not the focus.
                    ui.memory_mut(|m| m.set_focus_lock_filter(r.id, egui::EventFilter { horizontal_arrows: true, ..Default::default() }));
                    let step = range / 20.0;
                    if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
                        value = Some((element.value + step).min(element.max));
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
                        value = Some((element.value - step).max(element.min));
                    }
                }
                if let Some(value) = value.filter(|v| (v - element.value).abs() > f64::EPSILON) {
                    inputs.push((object, element.name.clone(), Input::Changed(value)));
                }
            }
        }
        Kind::Progress => {
            let share = element.value.clamp(0.0, 1.0) as f32;
            let fill_rect = egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + rect.width() * share, rect.bottom()));
            painter.rect_filled(fill_rect, style.radius, fade(style.accent));
        }
        Kind::Image => {
            if let Some((_, Some(texture))) = cache.images.get(&base.join(&element.image)) {
                let tint = color32(Color([255, 255, 255, 255]), style.opacity);
                painter.image(texture.id(), inner, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), tint);
            }
        }
        Kind::TextField => {
            if interactive {
                let key = (object, element.name.clone());
                let text = cache.typing.entry(key).or_insert_with(|| element.text.clone());
                let edit = egui::TextEdit::singleline(text).frame(egui::Frame::NONE).font(font).text_color(fade(style.color)).desired_width(inner.width());
                let typed = ui.put(inner, edit);
                ui.ctx().accesskit_node_builder(typed.id, |node| node.set_label(label.clone()));
                if typed.changed() {
                    inputs.push((object, element.name.clone(), Input::Typed(text.clone())));
                }
                if typed.has_focus() {
                    painter.rect_stroke(rect.expand(2.0), style.radius, egui::Stroke::new(2.0, fade(style.accent)), egui::StrokeKind::Outside);
                }
            } else {
                let galley = painter.layout_no_wrap(element.text.clone(), font, fade(style.color));
                painter.galley(text_at(&galley, inner, true), galley, fade(style.color));
            }
        }
        Kind::Panel | Kind::Row | Kind::Column | Kind::List | Kind::Part => {}
    }

    // A click on an element with On Click: `pressed:<name>` for one with a name (a button already
    // sent it, a toggle sent its change), then its own message.
    if let Some(r) = response.as_ref().filter(|_| clickable)
        && r.clicked()
    {
        if !element.name.is_empty() && !matches!(element.kind, Kind::Button | Kind::Toggle) {
            inputs.push((object, element.name.clone(), Input::Pressed));
        }
        inputs.push((object, element.name.clone(), Input::Clicked(element.on_click.clone())));
    }
    let Some(response) = response else { return };
    if control && response.has_focus() {
        painter.rect_stroke(rect.expand(2.0), style.radius, egui::Stroke::new(2.0, fade(style.accent)), egui::StrokeKind::Outside);
    }
    let enabled = control;
    response.widget_info(|| match element.kind {
        Kind::Button => egui::WidgetInfo::labeled(egui::Role::Button, enabled, &label),
        // A clickable panel, image or text reads as the button it acts as.
        _ if clickable && !matches!(element.kind, Kind::Toggle | Kind::Slider) => egui::WidgetInfo::labeled(egui::Role::Button, true, &label),
        Kind::Toggle => egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, element.value != 0.0, &label),
        Kind::Slider => egui::WidgetInfo::slider(enabled, element.value, &label),
        Kind::Progress => egui::WidgetInfo::slider(false, element.value, &label),
        Kind::Image => egui::WidgetInfo::labeled(egui::Role::Image, true, &label),
        _ => egui::WidgetInfo::labeled(egui::Role::Label, true, &label),
    });
}
