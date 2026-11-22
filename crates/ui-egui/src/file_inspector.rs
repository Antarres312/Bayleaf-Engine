//! The Inspector for a file clicked in the Project panel: what it is, what it holds (from
//! `asset.info`), a picture of an image, the start of a text file, a model's or image's import
//! settings, and the actions that fit it.

use crate::i18n::tl;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use bayleaf_engine::asset_info::{self, Info};
use bayleaf_render::assets::import::{ColorSpace, Compression, ImportSettings, UpAxis};

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
    /// A Markdown file's contents, read and parsed, and its text.
    markdown: Option<(craft_flow::Flow, String)>,
    /// An Input Map being edited, or why it could not be read.
    input: Option<Result<crate::input_editor::Draft, String>>,
    /// Shows a Markdown file's text as written instead of formatted.
    show_source: bool,
    /// A lighting asset as read, or why it could not be.
    lighting: Option<Result<bayleaf_engine::scene::lighting::Lighting, String>>,
    /// A post-processing asset as read, or why it could not be.
    post: Option<Result<bayleaf_engine::scene::post::PostProcessing, String>>,
    /// A model's or image's import settings as read, or why they could not be.
    import: Option<Result<ImportSettings, String>>,
    /// An interface style file as read (and being edited), or why it could not be.
    style: Option<Result<bayleaf_interface::StyleSheet, String>>,
}

impl Inspected {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            info: Info::default(),
            modified: None,
            texture: None,
            markdown: None,
            show_source: false,
            input: None,
            lighting: None,
            post: None,
            import: None,
            style: None,
        }
    }

    /// Reads the file again when it changed since it was last read.
    fn refresh(&mut self, editor_session: &bayleaf_engine::Session) {
        let modified = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        if self.info.kind.is_empty() || modified != self.modified {
            self.info = asset_info::describe(editor_session, &self.path);
            self.modified = modified;
            self.texture = None;
            self.markdown = if crate::markdown_view::is_markdown(&self.path) { crate::markdown_view::read(&self.path) } else { None };
            let is_input = self.path.extension().is_some_and(|e| e == bayleaf_engine::input_cmds::INPUT_EXTENSION);
            self.input = is_input.then(|| bayleaf_engine::input_cmds::read_asset(editor_session, &self.path).map(|a| crate::input_editor::Draft::of(&a)));
            let is_lighting = self.path.extension().is_some_and(|e| e == bayleaf_engine::scene::lighting::EXTENSION);
            self.lighting = is_lighting.then(|| bayleaf_engine::scene::lighting::Lighting::load(&self.path).map_err(|e| e.to_string()));
            let is_post = self.path.extension().is_some_and(|e| e == bayleaf_engine::scene::post::EXTENSION);
            self.post = is_post.then(|| bayleaf_engine::scene::post::PostProcessing::load(&self.path).map_err(|e| e.to_string()));
            self.import = ImportSettings::defaults_for(&self.path).map(|defaults| ImportSettings::read_for(&self.path).map(|s| s.unwrap_or(defaults)));
            let is_style = self.path.extension().is_some_and(|e| e == bayleaf_interface::STYLE_EXTENSION);
            self.style = is_style.then(|| bayleaf_interface::StyleSheet::load(&self.path));
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
    let mut save_input = None;
    let mut changed_lighting = None;
    let mut changed_post = None;
    let mut make_lookup = false;
    let mut changed_style = None;
    let mut changed_import = None;

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(file.info.kind).small().color(t.text_muted));
        ui.add(egui::Label::new(egui::RichText::new(&name).heading()).truncate()).on_hover_text(shown_path.display().to_string());
        ui.add(egui::Label::new(egui::RichText::new(shown_path.display().to_string()).small().color(t.text_muted)).truncate());
        ui.add_space(6.0);

        if file.info.kind == "Image" {
            image_preview(ui, &mut file, &name);
        }
        if file.info.kind == "Material" {
            material_picture(ui, &mut editor.thumbnails, &file.path, &name);
        }
        if file.info.kind == "Interface" {
            interface_preview(ui, editor, &file.path);
        }
        // The open scene's own settings, under its file.
        if editor.session.scene_path() == Some(file.path.as_path()) {
            crate::scene_settings::show(editor, ui);
            ui.add_space(theme::SECTION_GAP);
        }
        // A lighting asset is edited here; every scene that names it changes.
        if let Some(Ok(lighting)) = file.lighting.as_mut()
            && crate::scene_settings::lighting_editor(ui, lighting)
        {
            changed_lighting = Some(lighting.clone());
        }
        // A post-processing asset is edited here; every scene and camera that names it changes.
        if let Some(Ok(post)) = file.post.as_mut() {
            if crate::scene_settings::post_editor(ui, post, &mut make_lookup) {
                changed_post = Some(post.clone());
            }
            if make_lookup {
                changed_post = Some(post.clone());
            }
        }

        // An interface style file is edited here; every layout using it changes.
        if let Some(Ok(style)) = file.style.as_mut()
            && crate::style_editor::show(ui, style)
        {
            changed_style = Some(style.clone());
        }

        // How a model or image is read; saved beside it, and every scene using it reads it again.
        match file.import.as_mut() {
            Some(Ok(settings)) => {
                if import_settings(ui, settings) {
                    changed_import = Some(settings.clone());
                }
                ui.add_space(theme::SECTION_GAP);
            }
            Some(Err(problem)) => {
                ui.add(egui::Label::new(crate::i18n::tf("Its import settings do not read: {problem}", &[("problem", problem)])).wrap());
                ui.add_space(theme::SECTION_GAP);
            }
            None => {}
        }

        theme::section(t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Grid::new("file rows").num_columns(2).spacing([12.0, 4.0]).min_row_height(ROW_HEIGHT - 4.0).show(ui, |ui| {
                if file.info.kind != "Folder" && file.info.kind != "Script Source" {
                    ui.label(egui::RichText::new(tl("Size on disk")).color(t.text_muted));
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
                ui.label(egui::RichText::new(tl("Problems")).strong().color(t.on_warning));
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

        if let Some(draft) = file.input.as_mut() {
            ui.add_space(6.0);
            match draft {
                Ok(draft) => save_input = crate::input_editor::show(ui, draft),
                Err(problem) => {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(crate::i18n::tf("This Input Map does not read: {problem}", &[("problem", &problem.to_string())])).color(t.text),
                        )
                        .wrap(),
                    );
                }
            }
        } else if file.markdown.is_some() {
            markdown_contents(ui, &mut file);
        } else if let Some(text) = &file.info.text {
            ui.add_space(6.0);
            let label = ui.label(egui::RichText::new(tl("Contents")).small().color(t.text_muted));
            theme::section(t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::both().id_salt("file text").max_height(TEXT_HEIGHT).auto_shrink([false, true]).show(ui, |ui| {
                    let response = ui.add(egui::Label::new(egui::RichText::new(text).monospace()).extend().selectable(true));
                    response.labelled_by(label.id);
                });
            });
        }
    });

    // An edited Input Map is saved; its new time on disk is taken, so the edits are not read back.
    if let Some(asset) = save_input
        && editor.dispatch("input.asset.set", &serde_json::json!({ "path": file.path, "asset": { "start": asset.start, "maps": asset.maps } }), None).is_some()
    {
        file.modified = std::fs::metadata(&file.path).and_then(|m| m.modified()).ok();
    }
    // An edited lighting asset is written; its new time on disk is taken, so it is not read back.
    if let Some(lighting) = changed_lighting
        && editor.dispatch("lighting.set", &serde_json::json!({ "path": file.path, "lighting": lighting }), None).is_some()
    {
        file.modified = std::fs::metadata(&file.path).and_then(|m| m.modified()).ok();
    }
    // A style that does not read is not written; the status bar says why, and the edit stays to fix.
    if let Some(style) = changed_style
        && editor.dispatch("ui.style.set", &serde_json::json!({ "path": file.path, "style": style }), None).is_some()
    {
        file.modified = std::fs::metadata(&file.path).and_then(|m| m.modified()).ok();
    }
    if make_lookup && let Some(mut post) = changed_post.take() {
        // A free name beside the asset: "<name> lookup.png", then "<name> lookup 2.png" and on.
        let stem = file.path.file_stem().map_or_else(|| "post".to_owned(), |s| s.to_string_lossy().into_owned());
        let folder = file.path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
        let free = (1..1000).map(|i| folder.join(if i == 1 { format!("{stem} lookup.png") } else { format!("{stem} lookup {i}.png") })).find(|p| !p.exists());
        if let Some(target) = free
            && let Some(made) = editor.dispatch("post.lookup.neutral", &serde_json::json!({ "path": target }), None)
        {
            post.color.lookup = made["relative"].as_str().unwrap_or_default().to_owned();
            post.color.enabled = true;
            if let Some(Ok(shown)) = file.post.as_mut() {
                *shown = post.clone();
            }
            changed_post = Some(post);
        }
    }
    if let Some(post) = changed_post
        && editor.dispatch("post.set", &serde_json::json!({ "path": file.path, "post": post }), None).is_some()
    {
        file.modified = std::fs::metadata(&file.path).and_then(|m| m.modified()).ok();
    }
    if let Some(settings) = changed_import {
        let mut given = serde_json::Map::new();
        if let Some(model) = &settings.model {
            given.insert("model".to_owned(), serde_json::json!(model));
        }
        if let Some(image) = &settings.image {
            given.insert("image".to_owned(), serde_json::json!(image));
        }
        if let Some(sound) = &settings.sound {
            given.insert("sound".to_owned(), serde_json::json!(sound));
        }
        if editor.dispatch("asset.importSettings.set", &serde_json::json!({ "path": file.path, "settings": given }), None).is_none() {
            // Refused (the status line says why): show what is saved again.
            file.import = ImportSettings::defaults_for(&file.path).map(|defaults| ImportSettings::read_for(&file.path).map(|s| s.unwrap_or(defaults)));
        }
    }
    let path = file.path.clone();
    // Something on the card (Edit on the scene's lighting) may have chosen another file to show.
    if editor.inspected_file.is_none() {
        editor.inspected_file = Some(file);
    }
    match action {
        Some(Action::OpenScene) => editor.open_scene(&path),
        Some(Action::OpenGraph) => editor.open_graph(&path),
        Some(Action::PlacePrefab) => editor.place_prefab(&path),
        Some(Action::BuildScript) => editor.build_script(&path),
        None => {}
    }
}

/// An interface layout as a game shows it, at a screen size chosen above it.
fn interface_preview(ui: &mut egui::Ui, editor: &mut Editor, path: &Path) {
    let t = theme::tokens(ui);
    let screens = crate::game_interface::PREVIEW_SCREENS;
    let chosen = editor.interface_preview_screen.min(screens.len() - 1);
    crate::value_editor::row(ui, t, "Screen", |ui, label, width| {
        craft_ui::ComboBox::from_id_salt("interface preview screen")
            .selected_text(tl(screens[chosen].0))
            .width(width)
            .show_ui(ui, |ui| {
                for (i, (name, _)) in screens.iter().enumerate() {
                    if ui.selectable_label(i == chosen, tl(name)).clicked() {
                        editor.interface_preview_screen = i;
                    }
                }
            })
            .response
            .labelled_by(label);
    });
    if ui.button(tl("Edit in Interface Panel")).on_hover_text(tl("Change its elements and their style, with the preview beside them")).clicked() {
        editor.open_interface(path);
    }
    let base = editor.session.script_dir();
    let relative = path.strip_prefix(&base).unwrap_or(path).to_string_lossy().into_owned();
    let screen = screens[editor.interface_preview_screen.min(screens.len() - 1)].1;
    if let Err(problem) =
        crate::game_interface::preview(ui, egui::vec2(ui.available_width(), f32::INFINITY), screen, &base, &relative, &mut editor.game_interfaces)
    {
        ui.add(
            egui::Label::new(egui::RichText::new(crate::i18n::tf("This interface does not read: {problem}", &[("problem", &problem)])).color(t.text)).wrap(),
        );
    }
    ui.add_space(theme::SECTION_GAP);
}

/// A model's scale and up axis, an image's mipmaps, colour space and compression, or whether a
/// sound streams. True when a change is finished (a drag let
/// go, a number typed in, a choice made), so the file is not written on every step of a drag.
fn import_settings(ui: &mut egui::Ui, settings: &mut ImportSettings) -> bool {
    let t = theme::tokens(ui);
    let mut done = false;
    theme::headed(t, "Import Settings").show(ui, |ui| {
        if let Some(model) = settings.model.as_mut() {
            crate::value_editor::row(ui, t, "Scale", |ui, label, width| {
                let response = ui
                    .add_sized(
                        [width.min(120.0), ROW_HEIGHT - 4.0],
                        egui::DragValue::new(&mut model.scale).speed(0.01).range(0.0001..=10_000.0).max_decimals(4),
                    )
                    .labelled_by(label)
                    .on_hover_text(tl("Every position times this: 0.01 for a model made in centimetres"));
                done |= response.drag_stopped() || response.lost_focus() || (response.changed() && !response.dragged() && !response.has_focus());
            });
            crate::value_editor::row(ui, t, "Up Axis", |ui, _, _| {
                for (axis, label, hint) in [
                    (UpAxis::Y, "Y Up", "The model was made with Y up, as the engine uses"),
                    (UpAxis::Z, "Z Up", "The model was made with Z up: it is turned to stand up"),
                ] {
                    if ui.radio_value(&mut model.up_axis, axis, tl(label)).on_hover_text(tl(hint)).changed() {
                        done = true;
                    }
                }
            });
        }
        if let Some(image) = settings.image.as_mut() {
            if ui
                .checkbox(&mut image.mipmaps, tl("Mipmaps"))
                .on_hover_text(tl("Smaller copies for far away, so it does not shimmer; off keeps every pixel sharp, as pixel art wants"))
                .changed()
            {
                done = true;
            }
            crate::value_editor::row(ui, t, "Pixels Are", |ui, _, _| {
                for (space, label, hint) in [
                    (ColorSpace::Color, "Colors", "Colours, stored as sRGB: shown as they look"),
                    (ColorSpace::Data, "Data", "Plain numbers, read as stored: normal maps, roughness, masks"),
                ] {
                    done |= ui.radio_value(&mut image.color_space, space, tl(label)).on_hover_text(tl(hint)).changed();
                }
            });
            crate::value_editor::row(ui, t, "Compression", |ui, _, _| {
                ui.vertical(|ui| {
                    for (kind, label, hint) in [
                        (Compression::None, "None", "Every pixel as it is: 4 bytes each"),
                        (Compression::Bc1, "BC1", "Half a byte a pixel; transparency only on or off. Sides must be multiples of 4"),
                        (Compression::Bc3, "BC3", "A byte a pixel, with smooth transparency. Sides must be multiples of 4"),
                    ] {
                        done |= ui.radio_value(&mut image.compression, kind, tl(label)).on_hover_text(tl(hint)).changed();
                    }
                });
            });
        }
        if let Some(sound) = settings.sound.as_mut() {
            done |= ui
                .checkbox(&mut sound.stream, tl("Stream"))
                .on_hover_text(tl("Read from the file as it plays, for long music: WAV, AIFF, MP3, Opus and AAC. Off decodes it whole the first time it plays"))
                .changed();
        }
    });
    done
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
        let label =
            crate::i18n::tf("Preview of {name}, {width} × {height} pixels", &[("name", name), ("width", &size.x.to_string()), ("height", &size.y.to_string())]);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &label));
    });
    ui.add_space(6.0);
}

/// The material on a sphere, as its Project panel tile shows it, or why there is no picture.
fn material_picture(ui: &mut egui::Ui, thumbnails: &mut crate::thumbnails::Thumbnails, path: &Path, name: &str) {
    use crate::thumbnails::Thumb;
    let t = theme::tokens(ui);
    let thumb = thumbnails.get(path);
    let side = ui.available_width().min(PREVIEW_HEIGHT).min(crate::thumbnails::PIXELS as f32);
    ui.vertical_centered(|ui| {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
        let painter = ui.painter();
        let label = match &thumb {
            Thumb::Ready(texture) => {
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                painter.add(egui::epaint::RectShape::filled(rect, theme::RADIUS, egui::Color32::WHITE).with_texture(*texture, uv));
                crate::i18n::tf("Preview of {name}: the material on a sphere", &[("name", name)])
            }
            other => {
                painter.rect_filled(rect, theme::RADIUS, t.surface);
                let text = match other {
                    Thumb::Failed(_) => tl("Does not compile"),
                    Thumb::Unavailable => tl("Drawing previews needs a graphics adapter"),
                    _ => tl("Drawing the preview…"),
                };
                let color = if matches!(other, Thumb::Failed(_)) { t.warning } else { t.text_muted };
                let galley = painter.layout(text.to_owned(), egui::FontId::proportional(12.0), color, side - 16.0);
                painter.galley(rect.center() - galley.size() / 2.0, galley, color);
                if matches!(other, Thumb::Failed(_)) {
                    crate::thumbnails::paint_warning(painter, rect.shrink(6.0), t);
                }
                crate::i18n::tf("Preview of {name}: {text}", &[("name", name), ("text", text)])
            }
        };
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &label));
    });
    if let Thumb::Failed(error) = &thumb {
        ui.add(egui::Label::new(egui::RichText::new(error).small().color(t.warning)).wrap());
    }
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
        return crate::i18n::tf("{bytes} bytes", &[("bytes", &bytes.to_string())]);
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// A Markdown file's contents, formatted, or as written with Source.
fn markdown_contents(ui: &mut egui::Ui, file: &mut Inspected) {
    let t = theme::tokens(ui);
    let Some((flow, text)) = &file.markdown else { return };
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl("Contents")).small().color(t.text_muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let source = ui.add(egui::Button::selectable(file.show_source, "Source").small()).on_hover_text(tl("Show the Markdown as written"));
            if source.clicked() {
                file.show_source = !file.show_source;
            }
        });
    });
    theme::section(t).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if file.show_source {
            ui.add(egui::Label::new(egui::RichText::new(text).monospace()).wrap().selectable(true));
        } else {
            crate::markdown_view::show(ui, flow);
        }
        if file.info.bytes > crate::markdown_view::MAX_BYTES {
            ui.label(
                egui::RichText::new(crate::i18n::tf("Only the first {kb} KB are shown.", &[("kb", &(crate::markdown_view::MAX_BYTES / 1024).to_string())]))
                    .small()
                    .color(t.text_muted),
            );
        }
    });
}
