//! Project Settings: what applies to the whole project, such as physics. Reads the settings once
//! per project and changes them only through `project.settings.set`.

use bayleaf_engine::project_cmds::{PhysicsPrecision, PhysicsSettings, ProjectSettings, settings_of};
use serde_json::json;

use crate::Editor;
use crate::theme::{self, ROW_HEIGHT};

/// The settings shown, and the project folder they belong to.
#[derive(Default)]
pub(crate) struct Shown {
    project: Option<std::path::PathBuf>,
    settings: Option<ProjectSettings>,
}

pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    let t = theme::tokens(ui);
    let project = editor.session.project_dir().map(std::path::Path::to_path_buf);
    if editor.project_settings.settings.is_none() || editor.project_settings.project != project {
        editor.project_settings = Shown { settings: Some(settings_of(&editor.session)), project: project.clone() };
    }
    let Some(mut settings) = editor.project_settings.settings.clone() else { return };
    let before = settings.clone();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(4.0);
        if project.is_none() {
            ui.add(
                egui::Label::new(
                    egui::RichText::new("Open a project to change its settings. Scenes outside a project use these defaults.").color(t.text_muted),
                )
                .wrap(),
            );
            ui.add_space(theme::GAP);
        }
        ui.add_enabled_ui(project.is_some(), |ui| {
            theme::section(t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(egui::RichText::new("Physics").strong());
                ui.checkbox(&mut settings.physics.enabled, "Physics in play mode")
                    .on_hover_text("Off: Rigid Bodies and Colliders do nothing when the game runs");
                ui.add_enabled_ui(settings.physics.enabled, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Gravity");
                        for (axis, value) in ["X", "Y", "Z"].iter().zip(settings.physics.gravity.iter_mut()) {
                            let field = ui.add_sized(
                                [64.0, ROW_HEIGHT - 4.0],
                                egui::DragValue::new(value).speed(0.05).range(-1000.0..=1000.0).max_decimals(2).prefix(format!("{axis} ")),
                            );
                            ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(format!("Gravity {axis}, metres per second squared")));
                        }
                        ui.label(egui::RichText::new("m/s²").color(t.text_muted));
                    });
                    if ui.button("Earth Gravity").on_hover_text("9.81 m/s² downwards").clicked() {
                        settings.physics.gravity = PhysicsSettings::default().gravity;
                    }
                    ui.horizontal(|ui| {
                        ui.label("Precision");
                        let choices = [
                            (PhysicsPrecision::Single, "Single", "Faster; exact to the millimetre within about 10 km of the world's centre"),
                            (PhysicsPrecision::Double, "Double", "About a third slower physics; exact far from the centre, for large worlds"),
                        ];
                        for (value, label, hint) in choices {
                            let button = ui.add(egui::Button::selectable(settings.physics.precision == value, label)).on_hover_text(hint);
                            ui.ctx().accesskit_node_builder(button.id, |node| node.set_label(format!("{label} physics precision")));
                            if button.clicked() {
                                settings.physics.precision = value;
                            }
                        }
                    });
                });
            });
            ui.add_space(theme::GAP);
            ui.label(egui::RichText::new("Play mode uses these settings from its next start.").small().color(t.text_muted));
        });
    });
    if settings != before {
        let params = json!({ "settings": settings });
        if editor.dispatch("project.settings.set", &params, None).is_some() {
            editor.project_settings.settings = Some(settings);
        }
    }
}
