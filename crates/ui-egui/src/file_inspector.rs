//! The Inspector for a file clicked in the Project panel: what it is, what it holds (from
//! `asset.info`), a picture of an image, the start of a text file, and the actions that fit it.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use bayleaf_engine::asset_info::{self, Info};

use crate::Editor;
use crate::theme::{self, ROW_HEIGHT};

/// Largest preview, in points.
const PREVIEW_HEIGHT: f32 = 240.0;
/// How tall the text preview gets before it scrolls.
const TEXT_HEIGHT: f32 = 320.0;

/// The file the Inspector shows, read once and again when it changes on disk.
pub(crate) struct Inspected {
    pub path: PathBuf,
    info: Info,
    modified: Option<SystemTime>,
    /// An image's picture, made the first time it is shown.
    texture: Option<egui::TextureHandle>,
}

impl Inspected {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path, info: Info::default(), modified: None, texture: None }
    }

    /// Reads the file again when it changed since it was last read.
    fn refresh(&mut self, editor_session: &bayleaf_engine::Session) {
        let modified = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        if self.info.kind.is_empty() || modified != self.modified {
            self.info = asset_info::describe(editor_session, &self.path);
            self.modified = modified;
            self.texture = None;
        }
    }
}

/// What a button on the file card asks for.
enum Action {
    OpenScene,
    OpenGraph,
    PlacePrefab,
    BuildScript,
}

pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    let Some(mut file) = editor.inspected_file.take() else { return };
    file.refresh(&editor.session);
    let t = theme::tokens(ui);
    let name = file.path.file_name().map_or_else(|| file.path.display().to_string(), |n| n.to_string_lossy().into_owned());
    let shown_path = file.path.strip_prefix(editor.session.script_dir()).map_or_else(|_| file.path.clone(), Path::to_path_buf);
    let mut action = None;

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(file.info.kind).small().color(t.text_muted));
        ui.add(egui::Label::new(egui::RichText::new(&name).heading()).truncate()).on_hover_text(shown_path.display().to_string());
        ui.add(egui::Label::new(egui::RichText::new(shown_path.display().to_string()).small().color(t.text_muted)).truncate());
        ui.add_space(6.0);

        if file.info.kind == "Image" {
            image_preview(ui, &mut file, &name);
        }

        theme::section(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Grid::new("file rows").num_columns(2).spacing([12.0, 4.0]).min_row_height(ROW_HEIGHT - 4.0).show(ui, |ui| {
                if file.info.kind != "Folder" && file.info.kind != "Script Source" {
                    ui.label(egui::RichText::new("Size on disk").color(t.text_muted));
                    ui.label(file_size(file.info.bytes));
                    ui.end_row();
                }
                for (label, value) in &file.info.rows {
                    ui.label(egui::RichText::new(label).color(t.text_muted));
                    ui.add(egui::Label::new(value).wrap());
                    ui.end_row();
                }
            });
        });
        ui.add_space(theme::SECTION_GAP);

        if !file.info.problems.is_empty() {
            theme::section(t).fill(t.warning).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(egui::RichText::new("Problems").strong().color(t.on_warning));
                for problem in &file.info.problems {
                    ui.add(egui::Label::new(egui::RichText::new(problem).color(t.on_warning)).wrap());
                }
            });
            ui.add_space(theme::SECTION_GAP);
        }

        let button = match file.info.kind {
            "Scene" | "Project" => Some(("Open Scene", "Open it in place of the current scene", Action::OpenScene)),
            "Material" | "Visual Script" => Some(("Open in Graph Editor", "Edit its nodes in the Graph panel", Action::OpenGraph)),
            "Prefab" => Some(("Place in Scene", "Adds a linked copy to the scene", Action::PlacePrefab)),
            "Script Source" => Some(("Build Script", "Build it to WebAssembly, next to its folder", Action::BuildScript)),
            _ => None,
        };
        if let Some((label, hint, chosen)) = button
            && ui.button(label).on_hover_text(hint).clicked()
        {
            action = Some(chosen);
        }

        if let Some(text) = &file.info.text {
            ui.add_space(6.0);
            let label = ui.label(egui::RichText::new("Contents").small().color(t.text_muted));
            theme::section(t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::both().id_salt("file text").max_height(TEXT_HEIGHT).auto_shrink([false, true]).show(ui, |ui| {
                    let response = ui.add(egui::Label::new(egui::RichText::new(text).monospace()).extend().selectable(true));
                    response.labelled_by(label.id);
                });
            });
        }
    });

    let path = file.path.clone();
    editor.inspected_file = Some(file);
    match action {
        Some(Action::OpenScene) => editor.open_scene(&path),
        Some(Action::OpenGraph) => editor.open_graph(&path),
        Some(Action::PlacePrefab) => editor.place_prefab(&path),
        Some(Action::BuildScript) => editor.build_script(&path),
        None => {}
    }
}

/// The image, fitted to the panel's width and the preview height, over a checkerboard so
/// transparent parts show.
fn image_preview(ui: &mut egui::Ui, file: &mut Inspected, name: &str) {
    if file.texture.is_none()
        && let Ok(image) = bayleaf_engine::render::assets::load_image(&file.path)
    {
        let pixels = egui::ColorImage::from_rgba_unmultiplied([image.width as usize, image.height as usize], &image.rgba);
        file.texture = Some(ui.ctx().load_texture(format!("file preview {}", file.path.display()), pixels, egui::TextureOptions::LINEAR));
    }
    let Some(texture) = &file.texture else { return };
    let size = texture.size_vec2();
    let scale = (ui.available_width() / size.x).min(PREVIEW_HEIGHT / size.y).clamp(f32::MIN_POSITIVE, 1.0);
    let shown = size * scale;
    let t = theme::tokens(ui);
    ui.vertical_centered(|ui| {
        let (rect, response) = ui.allocate_exact_size(shown, egui::Sense::hover());
        checkerboard(ui.painter(), rect, t.surface, t.surface_hover);
        ui.painter().image(texture.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        let label = format!("Preview of {name}, {} × {} pixels", size.x, size.y);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &label));
    });
    ui.add_space(6.0);
}

fn checkerboard(painter: &egui::Painter, rect: egui::Rect, a: egui::Color32, b: egui::Color32) {
    const CELL: f32 = 8.0;
    painter.rect_filled(rect, 0.0, a);
    let (columns, rows) = ((rect.width() / CELL).ceil() as usize, (rect.height() / CELL).ceil() as usize);
    for row in 0..rows {
        for column in (row % 2..columns).step_by(2) {
            let min = rect.min + egui::vec2(column as f32 * CELL, row as f32 * CELL);
            painter.rect_filled(egui::Rect::from_min_size(min, egui::vec2(CELL, CELL)).intersect(rect), 0.0, b);
        }
    }
}

/// A byte count in the unit that reads best.
fn file_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}
