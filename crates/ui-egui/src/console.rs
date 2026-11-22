//! Console panel: script output and play-mode messages, newest at the bottom.

use crate::i18n::tl;
use bayleaf_engine::Level;

use crate::theme::{self, ROW_HEIGHT};
use crate::{Editor, inspector};

pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    let t = theme::tokens(ui);
    let mut clear = false;
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let errors = editor.session.console().iter().filter(|l| l.level == Level::Error).count();
        ui.label(
            egui::RichText::new(crate::i18n::tf(
                "{lines}, {errors}",
                &[
                    ("lines", &crate::i18n::tn(editor.session.console().len() as u64, "{n} line", "{n} lines")),
                    ("errors", &crate::i18n::tn(errors as u64, "{n} error", "{n} errors")),
                ],
            ))
            .color(t.text_muted),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            clear = ui.add_enabled(!editor.session.console().is_empty(), egui::Button::new(tl("Clear"))).clicked();
        });
    });
    ui.separator();
    if editor.session.console().is_empty() {
        inspector::empty_state(ui, "No messages", "Script output and play-mode messages appear here.");
    } else {
        egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show_rows(
            ui,
            ROW_HEIGHT - 6.0,
            editor.session.console().len(),
            |ui, rows| {
                for line in &editor.session.console()[rows] {
                    ui.horizontal(|ui| {
                        // The word, not only the colour, marks an error.
                        if line.level == Level::Error {
                            ui.label(egui::RichText::new(tl("Error")).strong().color(t.error));
                        }
                        ui.add(egui::Label::new(egui::RichText::new(&line.text).monospace()).truncate());
                    });
                }
            },
        );
    }
    if clear {
        editor.dispatch("console.clear", &serde_json::Value::Null, None);
    }
}
