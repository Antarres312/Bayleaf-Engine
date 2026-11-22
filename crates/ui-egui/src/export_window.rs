//! The Export Game window: where a game goes (platform and folder), how its scripts run (the
//! script mode, saved per platform in the project's settings), and what `project.export.check`
//! says about the project for that choice, before `project.export` writes anything. It opens as a
//! window of its own, or inside the editor where a second window cannot open.

use bayleaf_engine::export::{Platform, ScriptMode};
use serde_json::{Value, json};

use crate::Editor;
use crate::i18n::{tf, tl};
use crate::theme::{self, ROW_HEIGHT};

/// The window's choices and what the last check and export said.
pub(crate) struct ExportWindow {
    platform: Platform,
    mode: ScriptMode,
    folder: String,
    /// The last check, for the platform and mode it was made for: its report, or why it failed.
    report: Option<((Platform, ScriptMode), Result<Value, String>)>,
    /// What the last export said: where it went, or why not.
    outcome: Option<Result<String, String>>,
    /// A native player being built in the background: the game's folder, and the build's answer.
    building: Option<(String, std::sync::mpsc::Receiver<Result<(), String>>)>,
}

impl ExportWindow {
    /// The window for the open project, on this computer's system and the mode its settings choose.
    pub(crate) fn new(editor: &Editor) -> Self {
        let platform = Platform::current();
        let mode = bayleaf_engine::export::chosen_mode(&editor.session, platform);
        Self { platform, mode, folder: String::new(), report: None, outcome: None, building: None }
    }
}

/// Runs the check again when the choices changed since the last one.
fn refresh(editor: &mut Editor, window: &mut ExportWindow) {
    let wanted = (window.platform, window.mode);
    if window.report.as_ref().is_some_and(|(made, _)| *made == wanted) {
        return;
    }
    let params = json!({ "platform": window.platform, "mode": window.mode });
    let result = editor.registry.execute(&mut editor.session, "project.export.check", &params).map_err(|e| e.to_string());
    window.report = Some((wanted, result));
}

/// Shows the window while it is open.
pub(crate) fn show(editor: &mut Editor, ctx: &egui::Context) {
    let Some(mut window) = editor.export_window.take() else { return };
    refresh(editor, &mut window);
    let mut keep = true;
    let builder = egui::ViewportBuilder::default().with_title(tl("Export Game")).with_inner_size([560.0, 660.0]).with_min_inner_size([420.0, 420.0]);
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("export game"), builder, |ui, _| {
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            keep = false;
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::tokens(ui).panel).inner_margin(egui::Margin::same(12))).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| body(editor, ui, &mut window, &mut keep));
        });
    });
    if keep {
        editor.export_window = Some(window);
    }
}

fn body(editor: &mut Editor, ui: &mut egui::Ui, window: &mut ExportWindow, keep: &mut bool) {
    let t = theme::tokens(ui);
    // Everything fits the window's width: long text wraps instead of widening it.
    ui.set_max_width(ui.available_width());
    // Where the game goes.
    theme::headed(t, "Platform").show(ui, |ui| {
        crate::value_editor::row(ui, t, "Platform", |ui, label, width| {
            let shown =
                |p: Platform| if p == Platform::current() { tl(p.label()).to_owned() } else { tf("{platform} (not yet)", &[("platform", tl(p.label()))]) };
            craft_ui::ComboBox::from_id_salt("export platform")
                .width(width)
                .selected_text(shown(window.platform))
                .show_ui(ui, |ui| {
                    for platform in Platform::ALL {
                        if ui.selectable_label(window.platform == platform, shown(platform)).clicked() && window.platform != platform {
                            window.platform = platform;
                            window.mode = bayleaf_engine::export::chosen_mode(&editor.session, platform);
                        }
                    }
                })
                .response
                .labelled_by(label);
        });
    });
    ui.add_space(theme::SECTION_GAP);

    // How its scripts run, saved for the platform in the project's settings.
    theme::headed(t, "Scripts").show(ui, |ui| {
        for mode in ScriptMode::ALL {
            let title = if mode.available_for(window.platform) {
                tl(mode.label()).to_owned()
            } else {
                tf("{mode} (not available here)", &[("mode", tl(mode.label()))])
            };
            if ui.radio(window.mode == mode, title).clicked() && window.mode != mode {
                window.mode = mode;
                save_mode(editor, window.platform, mode);
            }
            ui.indent(("mode summary", mode.label()), |ui| {
                ui.add(egui::Label::new(egui::RichText::new(tl(mode.summary())).small().color(t.text_muted)).wrap());
            });
        }
    });
    ui.add_space(theme::SECTION_GAP);

    // What the check says about this choice.
    theme::headed(t, "Check").show(ui, |ui| match &window.report {
        Some((_, Ok(report))) => report_ui(ui, t, report),
        Some((_, Err(why))) => {
            ui.add(egui::Label::new(egui::RichText::new(tf("The check could not run: {problem}", &[("problem", why)])).color(t.error)).wrap());
        }
        None => {}
    });
    if ui.button(tl("Check Again")).on_hover_text(tl("Look at the project again, after changing it")).clicked() {
        window.report = None;
        refresh(editor, window);
    }
    ui.add_space(theme::SECTION_GAP);

    // A native build that finished says how it went.
    if let Some((path, answer)) = &window.building {
        match answer.try_recv() {
            Ok(result) => {
                window.outcome = Some(result.map(|()| path.clone()));
                window.building = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                window.outcome = Some(Err(tl("the build stopped without an answer").to_owned()));
                window.building = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(std::time::Duration::from_millis(250)),
        }
    }
    // Where to, and the export itself.
    let ready = matches!(&window.report, Some((_, Ok(report))) if report["ready"] == true) && window.building.is_none();
    theme::headed(t, "Export").show(ui, |ui| {
        crate::value_editor::row(ui, t, "Folder", |ui, label, width| {
            ui.allocate_ui_with_layout(egui::vec2(width, ROW_HEIGHT), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(tl("Choose…")).clicked()
                    && let Some(path) = rfd::FileDialog::new().set_title(tl("Choose the folder to export the game into")).pick_folder()
                {
                    window.folder = path.display().to_string();
                }
                ui.add_sized(
                    [ui.available_width(), ROW_HEIGHT],
                    egui::TextEdit::singleline(&mut window.folder).hint_text(tl("A folder the game's own folder goes into")),
                )
                .labelled_by(label);
            });
        });
        ui.add_space(theme::GAP);
        ui.horizontal(|ui| {
            let can = ready && !window.folder.trim().is_empty();
            let export = ui
                .add_enabled(can, egui::Button::new(egui::RichText::new(tl("Export")).color(t.on_accent)).fill(t.accent))
                .on_disabled_hover_text(tl("Choose a folder, and fix the errors the check lists"));
            if export.clicked() {
                // A native player builds for minutes: the export returns when it can start, and it
                // builds in the background while the editor stays in use.
                let params = json!({ "folder": window.folder.trim(), "platform": window.platform, "mode": window.mode, "wait": false });
                match editor.registry.execute(&mut editor.session, "project.export", &params) {
                    Ok(result) => {
                        let path = result["path"].as_str().unwrap_or_default().to_owned();
                        // Native mode: the player still to build (`build` is null otherwise).
                        match serde_json::from_value::<bayleaf_engine::native_export::NativeBuild>(result["build"].clone()) {
                            Ok(plan) => {
                                let (sender, receiver) = std::sync::mpsc::channel();
                                std::thread::spawn(move || {
                                    let _ = sender.send(bayleaf_engine::native_export::build(&plan));
                                });
                                window.building = Some((path, receiver));
                                window.outcome = None;
                            }
                            Err(_) => window.outcome = Some(Ok(path)),
                        }
                    }
                    Err(e) => window.outcome = Some(Err(e.to_string())),
                }
                // The project may have changed meanwhile; check it again.
                window.report = None;
            }
            if ui.button(tl("Close")).clicked() {
                *keep = false;
            }
        });
        if window.building.is_some() {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new());
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(tl("Building the game's player with its Rust scripts. The first build takes several minutes.")).color(t.text),
                    )
                    .wrap(),
                );
            });
        }
        match &window.outcome {
            Some(Ok(path)) => {
                ui.add(egui::Label::new(egui::RichText::new(tf("Exported to {path}", &[("path", path)])).color(t.text)).wrap());
            }
            Some(Err(why)) => {
                ui.add(egui::Label::new(egui::RichText::new(tf("Not exported: {problem}", &[("problem", why)])).color(t.error)).wrap());
            }
            None => {}
        }
    });
}

/// The check's verdict, its findings (each says what it is in words, not only in colour) and how
/// each script would run.
fn report_ui(ui: &mut egui::Ui, t: &theme::Tokens, report: &Value) {
    let findings = report["findings"].as_array().cloned().unwrap_or_default();
    let errors = findings.iter().filter(|f| f["severity"] == "error").count() as u64;
    let verdict = if report["ready"] == true {
        tl("Ready to export.").to_owned()
    } else {
        crate::i18n::tn(errors, "Not ready: 1 error to fix.", "Not ready: {n} errors to fix.")
    };
    ui.label(egui::RichText::new(verdict).strong());
    for finding in &findings {
        let (word, color) = match finding["severity"].as_str() {
            Some("error") => (tl("Error"), t.error),
            Some("warning") => (tl("Warning"), t.warning),
            _ => (tl("Note"), t.text_muted),
        };
        let message = finding["message"].as_str().unwrap_or_default();
        ui.add(egui::Label::new(egui::RichText::new(format!("{word}: {message}")).color(color)).wrap());
    }
    let scripts = report["scripts"].as_array().cloned().unwrap_or_default();
    if !scripts.is_empty() {
        ui.add_space(theme::GAP);
        ui.label(egui::RichText::new(tl("How each script would run")).small().color(t.text_muted));
        // One script a line or two: its path, then how it runs, wrapped to the window (a grid does not
        // wrap, and long notes would widen the window).
        for script in &scripts {
            let runs = serde_json::from_value::<ScriptMode>(script["runs"].clone()).map(|m| tl(m.label()).to_owned()).unwrap_or_default();
            let note = script["note"].as_str().unwrap_or_default();
            let how = if note.is_empty() { runs } else { format!("{runs}: {}", tl(note)) };
            let path = script["path"].as_str().unwrap_or_default();
            ui.add(egui::Label::new(egui::RichText::new(format!("{path}: {how}"))).wrap());
        }
    }
    ui.label(egui::RichText::new(tf("{files} files go with the game.", &[("files", &report["files"].to_string())])).small().color(t.text_muted));
}

/// Keeps the mode for the platform in the project's settings, so the next export starts with it.
fn save_mode(editor: &mut Editor, platform: Platform, mode: ScriptMode) {
    let mut settings = bayleaf_engine::project_cmds::settings_of(&editor.session);
    settings.game.script_modes.insert(platform, mode);
    if editor.dispatch("project.settings.set", &json!({ "settings": settings }), None).is_some() {
        // The Project Settings panel reads them again.
        crate::project_settings::forget(editor);
    }
}
