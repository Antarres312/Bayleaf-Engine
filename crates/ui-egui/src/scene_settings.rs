//! The scene's own settings in the Inspector (its lighting asset, through `scene.settings.set`),
//! and the editor for a lighting asset (`lighting.set`), shown when its file is selected.

use bayleaf_engine::scene::lighting::Lighting;
use serde_json::json;

use crate::Editor;
use crate::i18n::{tf, tl};
use crate::theme::{self, ROW_HEIGHT};
use crate::value_editor::row;

/// The scene's settings: which lighting asset lights it, with buttons to edit it or make a new one.
pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    let t = theme::tokens(ui);
    let current = editor.session.scene().settings().lighting.clone();
    let files = editor.project.lighting_files();
    let base = editor.session.script_dir();
    let mut chosen: Option<String> = None;
    let mut edit = false;
    let mut make = false;
    let post_current = editor.session.scene().settings().post_processing.clone();
    let post_files = editor.project.post_files();
    let (mut post_chosen, mut post_edit, mut post_make) = (None, false, false);
    theme::headed(t, "Scene Settings").show(ui, |ui| {
        row(ui, t, "Lighting", |ui, label, width| {
            ui.allocate_ui_with_layout(egui::vec2(width, ROW_HEIGHT), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if current.is_empty() {
                    let button = ui.button(tl("New")).on_hover_text(tl("Make a lighting asset beside the scene and use it"));
                    ui.ctx().accesskit_node_builder(button.id, |node| node.set_label(tl("New lighting")));
                    make = button.clicked();
                } else {
                    let button = ui.button(tl("Edit")).on_hover_text(tl("Show it in the Inspector to change it"));
                    ui.ctx().accesskit_node_builder(button.id, |node| node.set_label(tl("Edit lighting")));
                    edit = button.clicked();
                }
                let shown = if current.is_empty() { tl("The engine's").to_owned() } else { current.clone() };
                let list = craft_ui::ComboBox::from_id_salt("scene lighting")
                    .width((ui.available_width() - 1.0).max(16.0))
                    .truncate()
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(current.is_empty(), tl("The engine's")).clicked() {
                            chosen = Some(String::new());
                        }
                        for file in &files {
                            if ui.selectable_label(current == *file, file).clicked() {
                                chosen = Some(file.clone());
                            }
                        }
                        if files.is_empty() {
                            ui.label(egui::RichText::new(tl("No lighting assets yet: New makes one")).color(t.text_muted));
                        }
                    })
                    .response
                    .labelled_by(label);
                // A lighting asset dragged from the Project panel is chosen too.
                if let Some(dropped) = crate::value_editor::file_drop(ui, &list, |path| path.ends_with(".blight")) {
                    chosen = Some(dropped);
                }
            });
        });
        post_row(ui, t, &post_current, &post_files, &mut post_chosen, &mut post_edit, &mut post_make);
        if !current.is_empty() && !base.join(&current).is_file() {
            ui.label(egui::RichText::new(tf("Warning: {file} is not there, so the scene shows the engine's light.", &[("file", &current)])).color(t.warning));
        }
        ui.add(
            egui::Label::new(
                egui::RichText::new(tl("The sky, the light around everything, fog and a sun for scenes without one. Scenes that name the same file share it."))
                    .small()
                    .color(t.text_muted),
            )
            .wrap(),
        );
    });
    if let Some(lighting) = chosen {
        editor.dispatch("scene.settings.set", &json!({ "lighting": lighting }), None);
    }
    if let Some(post) = post_chosen {
        editor.dispatch("scene.settings.set", &json!({ "postProcessing": post }), None);
    }
    if post_edit {
        editor.inspect_file(base.join(&post_current));
    }
    if post_make {
        new_post(editor);
    }
    if edit {
        editor.inspect_file(base.join(&current));
    }
    if make {
        new_lighting(editor);
    }
}

/// Makes a lighting asset beside the scene (or in the project folder) and lets the scene use it.
fn new_lighting(editor: &mut Editor) {
    let base = editor.session.script_dir();
    let scene = editor.session.scene_path().map(std::path::Path::to_path_buf);
    let folder = scene.as_ref().and_then(|p| p.parent()).map_or_else(|| base.clone(), std::path::Path::to_path_buf);
    let stem = scene.as_ref().and_then(|p| p.file_stem()).map_or_else(|| "Scene".to_owned(), |s| s.to_string_lossy().into_owned());
    let name = (1..)
        .map(|n| if n == 1 { format!("{stem} lighting") } else { format!("{stem} lighting {n}") })
        .find(|name| !folder.join(format!("{name}.blight")).exists())
        .unwrap_or_else(|| format!("{stem} lighting"));
    let Some(made) = editor.dispatch("asset.create", &json!({ "kind": "lighting", "folder": folder, "name": name }), None) else { return };
    let path = std::path::PathBuf::from(made["path"].as_str().unwrap_or_default());
    let relative = path.strip_prefix(&base).unwrap_or(&path).to_string_lossy().replace('\\', "/");
    editor.dispatch("scene.settings.set", &json!({ "lighting": relative }), None);
    editor.project.rescan_soon();
}

/// Fields for every value of a lighting asset. Returns true when one changed.
pub(crate) fn lighting_editor(ui: &mut egui::Ui, lighting: &mut Lighting) -> bool {
    let t = theme::tokens(ui);
    let mut changed = false;
    let color = |ui: &mut egui::Ui, label: egui::Id, value: &mut [f32; 3]| ui.color_edit_button_rgb(value).labelled_by(label).changed();
    let number = |ui: &mut egui::Ui, label: egui::Id, width: f32, value: &mut f32, range: std::ops::RangeInclusive<f32>, speed: f64| {
        ui.add_sized([width, ROW_HEIGHT], egui::DragValue::new(value).range(range).speed(speed).max_decimals(2)).labelled_by(label).changed()
    };
    theme::headed(t, "Sky").show(ui, |ui| {
        changed |= row(ui, t, "Sky Color", |ui, label, _| color(ui, label, &mut lighting.sky_color));
        changed |= row(ui, t, "Horizon Color", |ui, label, _| color(ui, label, &mut lighting.horizon_color));
        changed |= row(ui, t, "Ground Color", |ui, label, _| color(ui, label, &mut lighting.ground_color));
        changed |= row(ui, t, "Ambient", |ui, label, width| number(ui, label, width, &mut lighting.ambient, 0.0..=4.0, 0.01));
        changed |= row(ui, t, "Show Sky", |ui, label, _| ui.checkbox(&mut lighting.show_sky, "").labelled_by(label).changed());
        changed |= row(ui, t, "Fog Distance", |ui, label, width| number(ui, label, width, &mut lighting.fog_distance, 0.0..=100_000.0, 0.5));
    });
    ui.add_space(theme::SECTION_GAP);
    theme::headed(t, "Sun").show(ui, |ui| {
        changed |= row(ui, t, "Sun", |ui, label, _| ui.checkbox(&mut lighting.sun.enabled, "").labelled_by(label).changed());
        ui.add(egui::Label::new(egui::RichText::new(tl("Lights scenes that have no Directional Light of their own.")).small().color(t.text_muted)).wrap());
        changed |= row(ui, t, "Rotation", |ui, label, width| {
            let mut any = false;
            ui.horizontal(|ui| {
                for value in &mut lighting.sun.rotation {
                    any |= ui
                        .add_sized([(width / 3.0 - 4.0).max(30.0), ROW_HEIGHT], egui::DragValue::new(value).speed(0.5).suffix("°").max_decimals(1))
                        .labelled_by(label)
                        .changed();
                }
            });
            any
        });
        changed |= row(ui, t, "Color", |ui, label, _| color(ui, label, &mut lighting.sun.color));
        changed |= row(ui, t, "Intensity", |ui, label, width| number(ui, label, width, &mut lighting.sun.intensity, 0.0..=100.0, 0.05));
        changed |= row(ui, t, "Shadows", |ui, label, _| ui.checkbox(&mut lighting.sun.shadows, "").labelled_by(label).changed());
    });
    changed
}

/// The scene's post-processing: which asset, with Edit or New beside it.
fn post_row(ui: &mut egui::Ui, t: &theme::Tokens, current: &str, files: &[String], chosen: &mut Option<String>, edit: &mut bool, make: &mut bool) {
    row(ui, t, "Post-processing", |ui, label, width| {
        ui.allocate_ui_with_layout(egui::vec2(width, ROW_HEIGHT), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if current.is_empty() {
                let button = ui.button(tl("New")).on_hover_text(tl("Make a post-processing asset beside the scene and use it"));
                ui.ctx().accesskit_node_builder(button.id, |node| node.set_label(tl("New post-processing")));
                *make = button.clicked();
            } else {
                let button = ui.button(tl("Edit")).on_hover_text(tl("Show it in the Inspector to change it"));
                ui.ctx().accesskit_node_builder(button.id, |node| node.set_label(tl("Edit post-processing")));
                *edit = button.clicked();
            }
            let shown = if current.is_empty() { tl("None").to_owned() } else { current.to_owned() };
            let list = craft_ui::ComboBox::from_id_salt("scene post-processing")
                .width((ui.available_width() - 1.0).max(16.0))
                .truncate()
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(current.is_empty(), tl("None")).clicked() {
                        *chosen = Some(String::new());
                    }
                    for file in files {
                        if ui.selectable_label(current == file, file).clicked() {
                            *chosen = Some(file.clone());
                        }
                    }
                })
                .response
                .labelled_by(label);
            if let Some(dropped) = crate::value_editor::file_drop(ui, &list, |path| path.ends_with(".bpost")) {
                *chosen = Some(dropped);
            }
        });
    });
}

/// Makes a post-processing asset beside the scene (or in the project folder) and lets the scene use it.
fn new_post(editor: &mut Editor) {
    let base = editor.session.script_dir();
    let scene = editor.session.scene_path().map(std::path::Path::to_path_buf);
    let folder = scene.as_ref().and_then(|p| p.parent()).map_or_else(|| base.clone(), std::path::Path::to_path_buf);
    let stem = scene.as_ref().and_then(|p| p.file_stem()).map_or_else(|| "Scene".to_owned(), |s| s.to_string_lossy().into_owned());
    let name = (1..)
        .map(|n| if n == 1 { format!("{stem} post") } else { format!("{stem} post {n}") })
        .find(|name| !folder.join(format!("{name}.bpost")).exists())
        .unwrap_or_else(|| format!("{stem} post"));
    let Some(made) = editor.dispatch("asset.create", &json!({ "kind": "postProcessing", "folder": folder, "name": name }), None) else { return };
    let path = std::path::PathBuf::from(made["path"].as_str().unwrap_or_default());
    let relative = path.strip_prefix(&base).unwrap_or(&path).to_string_lossy().replace('\\', "/");
    editor.dispatch("scene.settings.set", &json!({ "postProcessing": relative }), None);
    editor.project.rescan_soon();
}

/// Fields for every effect of a post-processing asset, each in its own section with its switch.
/// Returns true when one changed.
pub(crate) fn post_editor(ui: &mut egui::Ui, post: &mut bayleaf_engine::scene::post::PostProcessing) -> bool {
    let t = theme::tokens(ui);
    let mut changed = false;
    let number = |ui: &mut egui::Ui, label: egui::Id, width: f32, value: &mut f32, range: std::ops::RangeInclusive<f32>, speed: f64| {
        ui.add_sized([width, ROW_HEIGHT], egui::DragValue::new(value).range(range).speed(speed).max_decimals(2)).labelled_by(label).changed()
    };
    let switch = |ui: &mut egui::Ui, on: &mut bool| row(ui, t, "On", |ui, label, _| ui.checkbox(on, "").labelled_by(label).changed());
    let note = |ui: &mut egui::Ui, text: &str| {
        ui.add(egui::Label::new(egui::RichText::new(tl(text)).small().color(t.text_muted)).wrap());
    };
    theme::headed(t, "Tone Mapping").show(ui, |ui| {
        changed |= switch(ui, &mut post.tone_mapping.enabled);
        changed |= row(ui, t, "Exposure", |ui, label, width| number(ui, label, width, &mut post.tone_mapping.exposure, 0.0..=16.0, 0.02));
        note(ui, "Brightness, then a filmic curve onto the screen, so bright light keeps its colour instead of clipping.");
    });
    ui.add_space(theme::SECTION_GAP);
    theme::headed(t, "Bloom").show(ui, |ui| {
        changed |= switch(ui, &mut post.bloom.enabled);
        changed |= row(ui, t, "Strength", |ui, label, width| number(ui, label, width, &mut post.bloom.strength, 0.0..=4.0, 0.01));
        changed |= row(ui, t, "Threshold", |ui, label, width| number(ui, label, width, &mut post.bloom.threshold, 0.0..=16.0, 0.02));
    });
    ui.add_space(theme::SECTION_GAP);
    theme::headed(t, "Color Adjustment").show(ui, |ui| {
        changed |= switch(ui, &mut post.color.enabled);
        changed |= row(ui, t, "Contrast", |ui, label, width| number(ui, label, width, &mut post.color.contrast, 0.0..=2.0, 0.01));
        changed |= row(ui, t, "Saturation", |ui, label, width| number(ui, label, width, &mut post.color.saturation, 0.0..=2.0, 0.01));
    });
    ui.add_space(theme::SECTION_GAP);
    theme::headed(t, "Vignette").show(ui, |ui| {
        changed |= switch(ui, &mut post.vignette.enabled);
        changed |= row(ui, t, "Strength", |ui, label, width| number(ui, label, width, &mut post.vignette.strength, 0.0..=1.0, 0.01));
        changed |= row(ui, t, "Smoothness", |ui, label, width| number(ui, label, width, &mut post.vignette.smoothness, 0.0..=1.0, 0.01));
    });
    ui.add_space(theme::SECTION_GAP);
    theme::headed(t, "Depth Fog").show(ui, |ui| {
        changed |= switch(ui, &mut post.depth_fog.enabled);
        changed |= row(ui, t, "Color", |ui, label, _| ui.color_edit_button_rgb(&mut post.depth_fog.color).labelled_by(label).changed());
        changed |= row(ui, t, "Start", |ui, label, width| number(ui, label, width, &mut post.depth_fog.start, 0.0..=1_000_000.0, 0.5));
        changed |= row(ui, t, "End", |ui, label, width| number(ui, label, width, &mut post.depth_fog.end, 0.0..=1_000_000.0, 0.5));
    });
    ui.add_space(theme::SECTION_GAP);
    theme::headed(t, "Ambient Occlusion").show(ui, |ui| {
        changed |= switch(ui, &mut post.occlusion.enabled);
        changed |= row(ui, t, "Strength", |ui, label, width| number(ui, label, width, &mut post.occlusion.strength, 0.0..=1.0, 0.01));
        changed |= row(ui, t, "Radius", |ui, label, width| number(ui, label, width, &mut post.occlusion.radius, 0.01..=10.0, 0.01));
        note(ui, "Creases and corners go darker, worked out from depth.");
    });
    ui.add_space(theme::SECTION_GAP);
    theme::headed(t, "Antialiasing").show(ui, |ui| {
        changed |= row(ui, t, "Smooth Edges", |ui, label, _| ui.checkbox(&mut post.antialiasing, "").labelled_by(label).changed());
        note(ui, "Smooths the jagged edges the scene's own multisampling leaves.");
    });
    changed
}
