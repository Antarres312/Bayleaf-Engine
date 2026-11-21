//! Project Settings: what applies to the whole project, such as physics. Reads the settings once
//! per project and changes them only through `project.settings.set`.

use bayleaf_engine::project_cmds::{ActionSetting, InputSettings, PhysicsPrecision, PhysicsSettings, ProjectSettings, settings_of};
use serde_json::json;

use crate::Editor;
use crate::theme::{self, ROW_HEIGHT};

/// The settings shown, and the project folder they belong to.
#[derive(Default)]
pub(crate) struct Shown {
    project: Option<std::path::PathBuf>,
    settings: Option<ProjectSettings>,
    /// Each action's name and bindings as typed (bindings separated by commas), kept while they
    /// are not valid yet.
    drafts: Vec<(String, String)>,
    /// Why the typed actions are not saved.
    problem: Option<String>,
}

fn drafts_of(settings: &ProjectSettings) -> Vec<(String, String)> {
    settings.input.actions.iter().map(|a| (a.name.clone(), a.bindings.join(", "))).collect()
}

pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    let t = theme::tokens(ui);
    let project = editor.session.project_dir().map(std::path::Path::to_path_buf);
    if editor.project_settings.settings.is_none() || editor.project_settings.project != project {
        let settings = settings_of(&editor.session);
        editor.project_settings = Shown { drafts: drafts_of(&settings), settings: Some(settings), project: project.clone(), problem: None };
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
            theme::headed(t, "Physics").show(ui, |ui| {
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
            ui.add_space(theme::SECTION_GAP);
            input_section(ui, t, &mut editor.project_settings, &mut settings);
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

/// Input actions: a name and its bindings per row. The typed text is saved once every row reads
/// right; until then the problem is shown under the list.
fn input_section(ui: &mut egui::Ui, t: &theme::Tokens, shown: &mut Shown, settings: &mut ProjectSettings) {
    theme::headed(t, "Input Actions").show(ui, |ui| {
        ui.label(egui::RichText::new("Bindings: key:W, mouse:Left, pad:South, axis:LeftX+ (separate with commas)").small().color(t.text_muted));
        let mut changed = false;
        let mut remove = None;
        for (index, (name, bindings)) in shown.drafts.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                let field = ui.add_sized([120.0, ROW_HEIGHT - 4.0], egui::TextEdit::singleline(name));
                ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(format!("Name of action {}", index + 1)));
                changed |= field.changed();
                let width = (ui.available_width() - 72.0).max(80.0);
                let list = ui.add_sized([width, ROW_HEIGHT - 4.0], egui::TextEdit::singleline(bindings));
                let label = format!("Bindings of {name}");
                ui.ctx().accesskit_node_builder(list.id, |node| node.set_label(label));
                changed |= list.changed();
                if ui.button("Remove").on_hover_text("Remove this action").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            shown.drafts.remove(index);
            changed = true;
        }
        if ui.button("Add Action").clicked() {
            shown.drafts.push((format!("Action{}", shown.drafts.len() + 1), String::new()));
            changed = true;
        }
        if changed {
            let actions = shown
                .drafts
                .iter()
                .map(|(name, bindings)| ActionSetting {
                    name: name.trim().to_owned(),
                    bindings: bindings.split(',').map(str::trim).filter(|b| !b.is_empty()).map(str::to_owned).collect(),
                })
                .collect();
            let candidate = InputSettings { actions };
            match candidate.actions() {
                Ok(_) => {
                    settings.input = candidate;
                    shown.problem = None;
                }
                Err(problem) => shown.problem = Some(problem),
            }
        }
        if let Some(problem) = &shown.problem {
            ui.add(egui::Label::new(egui::RichText::new(format!("Not saved: {problem}")).color(t.text)).wrap());
        }
    });
}
