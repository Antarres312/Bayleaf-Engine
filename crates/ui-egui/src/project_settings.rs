//! Project Settings: what applies to the whole project, such as physics, the project's Input
//! Map, how shadows are drawn and the editor's preferences for it. Reads the settings once per project and changes them only through `project.settings.set`.

use crate::i18n::tl;
use bayleaf_engine::project_cmds::{MAX_SHADOW_DISTANCE, PhysicsPrecision, PhysicsSettings, ProjectSettings, SHADOW_RESOLUTIONS, settings_of};
use serde_json::json;

use crate::Editor;
use crate::theme::{self, ROW_HEIGHT};

/// The settings shown, and the project folder they belong to.
#[derive(Default)]
pub(crate) struct Shown {
    project: Option<std::path::PathBuf>,
    settings: Option<ProjectSettings>,
    /// The category picked in the list on the left.
    category: Category,
}

/// The kinds of settings, listed on the left of the panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Category {
    #[default]
    Physics,
    Input,
    Graphics,
    Game,
    Preferences,
}

impl Category {
    const ALL: [Category; 5] = [Category::Physics, Category::Input, Category::Graphics, Category::Game, Category::Preferences];

    fn label(self) -> &'static str {
        match self {
            Category::Physics => "Physics",
            Category::Input => "Input",
            Category::Graphics => "Graphics",
            Category::Game => "Game",
            Category::Preferences => "Preferences",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Category::Physics => "Physics in play mode, gravity and precision",
            Category::Input => "The project's Input Map",
            Category::Graphics => "How sharp shadows are, how far they reach and how many lights cast them",
            Category::Game => "How the exported game shows itself",
            Category::Preferences => "How the editor shows this project",
        }
    }
}

/// The open project's settings: read once per project, then kept as this panel changes them.
pub(crate) fn current(editor: &mut Editor) -> ProjectSettings {
    let project = editor.session.project_dir().map(std::path::Path::to_path_buf);
    if editor.project_settings.settings.is_none() || editor.project_settings.project != project {
        let category = editor.project_settings.category;
        editor.project_settings = Shown { settings: Some(settings_of(&editor.session)), project, category };
    }
    editor.project_settings.settings.clone().unwrap_or_default()
}

pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    let t = theme::tokens(ui);
    let mut settings = current(editor);
    let project = editor.project_settings.project.clone();
    let before = settings.clone();
    let input_files = editor.project.input_map_files();
    let images = editor.project.image_files();
    let mut edit = None;
    let mut category = editor.project_settings.category;
    // The categories on the left; the chosen one's settings on the right.
    egui::Panel::left("project settings categories").resizable(true).default_size(150.0).min_size(100.0).frame(egui::Frame::new()).show(ui, |ui| {
        ui.add_space(4.0);
        for choice in Category::ALL {
            let button = egui::Button::selectable(category == choice, choice.label()).min_size(egui::vec2(ui.available_width(), ROW_HEIGHT));
            if ui.add(button).on_hover_text(choice.hint()).clicked() {
                category = choice;
            }
        }
    });
    egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(egui::Margin { left: 8, ..egui::Margin::ZERO })).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(4.0);
            if project.is_none() {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(tl("Open a project to change its settings. Scenes outside a project use these defaults.")).color(t.text_muted),
                    )
                    .wrap(),
                );
                ui.add_space(theme::GAP);
            }
            ui.add_enabled_ui(project.is_some(), |ui| {
                match category {
                    Category::Physics => physics_section(ui, t, &mut settings),
                    Category::Input => edit = input_section(ui, t, &input_files, &mut settings),
                    Category::Graphics => graphics_section(ui, t, &mut settings),
                    Category::Game => game_section(ui, t, &images, &mut settings),
                    Category::Preferences => preferences_section(ui, t, &mut settings),
                }
                if !matches!(category, Category::Preferences | Category::Game | Category::Graphics) {
                    ui.add_space(theme::GAP);
                    ui.label(egui::RichText::new(tl("Play mode uses these settings from its next start.")).small().color(t.text_muted));
                }
            });
        });
    });
    editor.project_settings.category = category;
    if settings != before {
        let params = json!({ "settings": settings });
        if editor.dispatch("project.settings.set", &params, None).is_some() {
            editor.project_settings.settings = Some(settings);
        }
    }
    if let Some(relative) = edit {
        let path = editor.session.script_dir().join(relative);
        editor.inspect_file(path);
    }
}

/// Physics in play mode: on or off, gravity, and how precisely positions count.
fn physics_section(ui: &mut egui::Ui, t: &theme::Tokens, settings: &mut ProjectSettings) {
    theme::headed(t, "Physics").show(ui, |ui| {
        ui.checkbox(&mut settings.physics.enabled, tl("Physics in play mode"))
            .on_hover_text(tl("Off: Rigid Bodies and Colliders do nothing when the game runs"));
        ui.add_enabled_ui(settings.physics.enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label(tl("Gravity"));
                for (axis, value) in ["X", "Y", "Z"].iter().zip(settings.physics.gravity.iter_mut()) {
                    let field = ui.add_sized(
                        [64.0, ROW_HEIGHT - 4.0],
                        egui::DragValue::new(value).speed(0.05).range(-1000.0..=1000.0).max_decimals(2).prefix(format!("{axis} ")),
                    );
                    ui.ctx().accesskit_node_builder(field.id, |node| {
                        node.set_label(crate::i18n::tf("Gravity {axis}, metres per second squared", &[("axis", axis)]))
                    });
                }
                ui.label(egui::RichText::new(tl("m/s²")).color(t.text_muted));
            });
            if ui.button(tl("Earth Gravity")).on_hover_text(tl("9.81 m/s² downwards")).clicked() {
                settings.physics.gravity = PhysicsSettings::default().gravity;
            }
            ui.horizontal(|ui| {
                ui.label(tl("Precision"));
                let choices = [
                    (PhysicsPrecision::Single, "Single", "Faster; exact to the millimetre within about 10 km of the world's centre"),
                    (PhysicsPrecision::Double, "Double", "About a third slower physics; exact far from the centre, for large worlds"),
                ];
                for (value, label, hint) in choices {
                    let button = ui.add(egui::Button::selectable(settings.physics.precision == value, tl(label))).on_hover_text(tl(hint));
                    ui.ctx().accesskit_node_builder(button.id, |node| node.set_label(crate::i18n::tf("{label} physics precision", &[("label", tl(label))])));
                    if button.clicked() {
                        settings.physics.precision = value;
                    }
                }
            });
        });
    });
}

/// How shadows are drawn in the Scene and Game views, scene renders and the exported game.
fn graphics_section(ui: &mut egui::Ui, t: &theme::Tokens, settings: &mut ProjectSettings) {
    let graphics = &mut settings.graphics;
    let note = |ui: &mut egui::Ui, text: &str| {
        ui.add(egui::Label::new(egui::RichText::new(tl(text)).small().color(t.text_muted)).wrap());
    };
    theme::headed(t, "Shadows").show(ui, |ui| {
        crate::value_editor::row(ui, t, "Resolution", |ui, label, width| {
            craft_ui::ComboBox::from_id_salt("shadow resolution")
                .width((width - 1.0).max(16.0))
                .selected_text(crate::i18n::tf("{size} × {size}", &[("size", &graphics.shadow_resolution.to_string())]))
                .show_ui(ui, |ui| {
                    for size in SHADOW_RESOLUTIONS {
                        let text = crate::i18n::tf("{size} × {size}", &[("size", &size.to_string())]);
                        if ui.selectable_label(graphics.shadow_resolution == size, text).clicked() {
                            graphics.shadow_resolution = size;
                        }
                    }
                })
                .response
                .labelled_by(label)
                .on_hover_text(tl("Texels per side of each shadow map: larger is sharper and uses more graphics memory"));
        });
        crate::value_editor::row(ui, t, "Shadow Lights", |ui, label, width| {
            ui.add_sized([width, ROW_HEIGHT], egui::DragValue::new(&mut graphics.shadow_lights).range(1..=4).speed(0.05))
                .labelled_by(label)
                .on_hover_text(tl("How many lights cast shadows at once, 1 to 4; sun-like lights come first, then the nearest"));
        });
        note(ui, "Each light that casts shadows draws the scene again from where it stands (six times for a Point Light), so more lights cost more.");
    });
    ui.add_space(theme::SECTION_GAP);
    theme::headed(t, "Sun Shadows").show(ui, |ui| {
        crate::value_editor::row(ui, t, "Cascades", |ui, label, width| {
            ui.add_sized([width, ROW_HEIGHT], egui::DragValue::new(&mut graphics.shadow_cascades).range(1..=4).speed(0.05))
                .labelled_by(label)
                .on_hover_text(tl("How many maps a sun's shadow is split into by distance, 1 to 4: near shadows stay sharp in a large scene"));
        });
        crate::value_editor::row(ui, t, "Distance", |ui, label, width| {
            let speed = (f64::from(graphics.shadow_distance) * 0.01).max(0.5);
            ui.add_sized(
                [width, ROW_HEIGHT],
                egui::DragValue::new(&mut graphics.shadow_distance).range(0.0..=MAX_SHADOW_DISTANCE).speed(speed).max_decimals(1),
            )
            .labelled_by(label)
            .on_hover_text(tl("How far from the camera a sun's shadow reaches, in world units; 0 for the whole scene"));
        });
        let reach = if graphics.shadow_distance > 0.0 {
            "Past this distance from the camera, shapes cast no sun shadow; a shorter distance gives sharper shadows nearby."
        } else {
            "0: the shadow covers the whole scene."
        };
        note(ui, reach);
        note(
            ui,
            "Cascades apply to scenes wider than about 80 units, and to any scene when the distance is shorter than it; otherwise a sun shadow is one map.",
        );
    });
    ui.add_space(theme::GAP);
    ui.add_space(theme::SECTION_GAP);
    theme::headed(t, "Edges").show(ui, |ui| {
        ui.checkbox(&mut graphics.multisampling, tl("Multisampling (4 samples a pixel)"))
            .on_hover_text(tl("Smooth edges of every shape as it is drawn; off saves GPU memory and time"));
        note(ui, "A post-processing asset's Antialiasing smooths what is left, or does the work alone with multisampling off.");
    });
    ui.add_space(theme::SECTION_GAP);
    theme::headed(t, "Post-processing Effects").show(ui, |ui| {
        let effects = &mut graphics.effects;
        for (on, label) in [
            (&mut effects.tone_mapping, "Tone Mapping"),
            (&mut effects.bloom, "Bloom"),
            (&mut effects.color, "Color Adjustment"),
            (&mut effects.vignette, "Vignette"),
            (&mut effects.depth_fog, "Depth Fog"),
            (&mut effects.occlusion, "Ambient Occlusion"),
            (&mut effects.antialiasing, "Antialiasing"),
        ] {
            ui.checkbox(on, tl(label));
        }
        note(ui, "The effects scenes may use at all: an effect turned off here stays off whatever a post-processing asset says.");
    });
    ui.add_space(theme::GAP);
    note(ui, "The Scene and Game views show changes at once; the exported game and scene renders use them too.");
}

/// How the editor shows this project.
fn preferences_section(ui: &mut egui::Ui, t: &theme::Tokens, settings: &mut ProjectSettings) {
    theme::headed(t, "Project Panel").show(ui, |ui| {
        ui.checkbox(&mut settings.editor.hide_extensions, tl("Hide file extensions"));
        ui.add(
            egui::Label::new(
                egui::RichText::new(tl(
                    "Show names without .bmaterial, .png and the like; the kind of each file still shows, and renaming keeps the extension.",
                ))
                .small()
                .color(t.text_muted),
            )
            .wrap(),
        );
    });
}

/// The project's Input Map: which `.binput` asset every script and Player Control read (an
/// object's own Input Map component comes first for it). Returns the asset to open for editing.
fn input_section(ui: &mut egui::Ui, t: &theme::Tokens, files: &[String], settings: &mut ProjectSettings) -> Option<String> {
    let mut edit = None;
    theme::headed(t, "Input").show(ui, |ui| {
        crate::value_editor::row(ui, t, "Input Map", |ui, label, width| {
            ui.allocate_ui_with_layout(egui::vec2(width, ROW_HEIGHT), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let current = settings.input.map.clone();
                if !current.is_empty() {
                    let button = ui.button(tl("Edit")).on_hover_text(tl("Open it in the Inspector to change its actions"));
                    ui.ctx().accesskit_node_builder(button.id, |node| node.set_label("Edit the Input Map"));
                    if button.clicked() {
                        edit = Some(current.clone());
                    }
                }
                let shown = if current.is_empty() { "None".to_owned() } else { current.clone() };
                let list = craft_ui::ComboBox::from_id_salt("project input map")
                    .width((ui.available_width() - 1.0).max(16.0))
                    .truncate()
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(current.is_empty(), tl("None")).clicked() {
                            settings.input.map.clear();
                        }
                        for file in files {
                            if ui.selectable_label(current == *file, file).clicked() {
                                settings.input.map = file.clone();
                            }
                        }
                        if files.is_empty() {
                            ui.label(egui::RichText::new(tl("No .binput files yet: create one in the Project panel")).color(t.text_muted));
                        }
                    })
                    .response
                    .labelled_by(label);
                // An Input Map can also be dragged here from the Project panel.
                if let Some(dropped) = crate::value_editor::file_drop(ui, &list, |path| files.iter().any(|f| f == path)) {
                    settings.input.map = dropped;
                }
            });
        });
        let note = if settings.input.actions.is_some() {
            "This project's actions are still in its project file from an older version; open the project again to move them into an Input Map."
        } else {
            "An Input Map holds maps, each with its actions; one map is active at a time. Code and visual scripts name an action map/action (Player/Jump). Objects can carry their own with an Input Map component."
        };
        ui.add(egui::Label::new(egui::RichText::new(note).small().color(t.text_muted)).wrap());
    });
    edit
}

/// How the exported game shows itself: its window's icon.
fn game_section(ui: &mut egui::Ui, t: &theme::Tokens, images: &[String], settings: &mut ProjectSettings) {
    theme::headed(t, "Game").show(ui, |ui| {
        crate::value_editor::row(ui, t, "Icon", |ui, label, width| {
            let current = settings.game.icon.clone();
            let pngs: Vec<&String> = images.iter().filter(|f| f.to_ascii_lowercase().ends_with(".png")).collect();
            let shown = if current.is_empty() { tl("The engine's").to_owned() } else { current.clone() };
            let list = craft_ui::ComboBox::from_id_salt("game icon")
                .width((width - 1.0).max(16.0))
                .truncate()
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(current.is_empty(), tl("The engine's")).clicked() {
                        settings.game.icon.clear();
                    }
                    for file in &pngs {
                        if ui.selectable_label(current == **file, file.as_str()).clicked() {
                            settings.game.icon = (*file).clone();
                        }
                    }
                    if pngs.is_empty() {
                        ui.label(egui::RichText::new(tl("No PNG images in the project folder")).color(t.text_muted));
                    }
                })
                .response
                .labelled_by(label);
            // A PNG can also be dragged here from the Project panel.
            if let Some(dropped) = crate::value_editor::file_drop(ui, &list, |path| path.to_ascii_lowercase().ends_with(".png")) {
                settings.game.icon = dropped;
            }
        });
        ui.add(
            egui::Label::new(
                egui::RichText::new(tl(
                    "The exported game's window shows this image (a square PNG). File > Export Game writes the game into a folder of its own.",
                ))
                .small()
                .color(t.text_muted),
            )
            .wrap(),
        );
    });
}

/// Reads the settings again the next time the panel shows, after something else changed them.
pub(crate) fn forget(editor: &mut Editor) {
    editor.project_settings.settings = None;
}
