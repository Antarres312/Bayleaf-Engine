//! The Profiler panel: while the game plays and it records, how long each frame took by part (each
//! part a colour and a name in the legend), the last frames as a graph, and one frame in detail:
//! the parts, the slowest scripts, and what was drawn. A click on the graph pauses on that frame.
//! It reads the engine's `profile.*` commands, so agents see the same numbers.

use bayleaf_engine::profiler::FrameProfile;
use serde_json::json;

use crate::Editor;
use crate::i18n::{tf, tl};
use crate::theme::{self, ROW_HEIGHT};

/// Frames the graph shows at most.
const SHOWN: usize = 300;

/// The panel's choice: the frame looked at, counted back from the newest (0 the newest).
#[derive(Default)]
pub(crate) struct ProfilerView {
    back: Option<usize>,
}

/// The timed parts of a frame: name, and its milliseconds.
fn parts(f: &FrameProfile) -> [(&'static str, f32); 7] {
    [
        ("Scripts", f.scripts),
        ("Physics", f.physics),
        ("Streaming", f.streaming),
        ("Sound", f.sound),
        ("Particles", f.particles),
        ("Gathering", f.extract.unwrap_or(0.0)),
        ("Drawing", f.draw.unwrap_or(0.0)),
    ]
}

/// The GPU's time for a frame, or why there is none.
fn gpu_text(gpu: Option<f32>, timed: Option<bool>) -> String {
    match (gpu, timed) {
        (Some(ms), _) => format!("{ms:.2} ms"),
        (None, Some(false)) => tl("not measured: this graphics device gives no timestamps").to_owned(),
        (None, _) => tl("not measured for this frame").to_owned(),
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    let t = theme::tokens(ui);
    let recording = editor.session.profiler().is_recording();
    let frames: Vec<FrameProfile> = editor.session.profiler().frames().iter().rev().take(SHOWN).rev().cloned().collect();
    let colors = [t.axis_z, t.pin_vector, t.pin_color, t.pin_text, t.warning, t.pin_bool, t.axis_x];
    ui.horizontal(|ui| {
        let (label, command, hint) = if recording {
            ("Stop", "profile.stop", "Stop recording; the frames stay to look at")
        } else {
            ("Record", "profile.start", "Record each frame while the game plays")
        };
        if ui.button(tl(label)).on_hover_text(tl(hint)).clicked() {
            editor.dispatch(command, &json!({}), None);
            editor.profiler_view.back = None;
        }
        if ui.button(tl("Clear")).on_hover_text(tl("Forget the frames recorded")).clicked() {
            editor.dispatch("profile.clear", &json!({}), None);
            editor.profiler_view.back = None;
        }
        let state = if recording {
            tf("Recording: {count} frames", &[("count", &editor.session.profiler().frames().len().to_string())])
        } else {
            tf("Not recording: {count} frames", &[("count", &editor.session.profiler().frames().len().to_string())])
        };
        ui.label(egui::RichText::new(state).color(t.text_muted));
        // Keyboard: step between frames.
        if let Some(newest) = frames.len().checked_sub(1) {
            let back = editor.profiler_view.back.unwrap_or(0).min(newest);
            if ui.add_enabled(back < newest, egui::Button::new(tl("Earlier Frame"))).clicked() {
                editor.profiler_view.back = Some(back + 1);
            }
            if ui.add_enabled(back > 0, egui::Button::new(tl("Later Frame"))).clicked() {
                editor.profiler_view.back = Some(back - 1);
            }
            if editor.profiler_view.back.is_some() && ui.button(tl("Newest")).clicked() {
                editor.profiler_view.back = None;
            }
        }
    });
    if frames.is_empty() {
        ui.add(egui::Label::new(egui::RichText::new(tl("Press Record, then play the game: each frame's time shows here by part.")).color(t.text_muted)).wrap());
        return;
    }
    // The legend: each part's colour and its name, so the graph does not rely on colour alone.
    ui.horizontal_wrapped(|ui| {
        for ((name, _), color) in parts(&frames[0]).iter().zip(colors) {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, color);
            ui.label(egui::RichText::new(tl(name)).small());
        }
    });
    // The graph and the chosen frame scroll in a short panel.
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        graph(editor, ui, &frames, &colors);
        let newest = frames.len() - 1;
        let chosen = &frames[newest - editor.profiler_view.back.unwrap_or(0).min(newest)];
        details(ui, chosen, &colors, editor.session.profiler().gpu_timed());
    });
}

/// The last frames as stacked bars, a line at 60 frames a second. A click pauses on a frame.
fn graph(editor: &mut Editor, ui: &mut egui::Ui, frames: &[FrameProfile], colors: &[egui::Color32; 7]) {
    let t = theme::tokens(ui);
    let width = ui.available_width().max(60.0);
    let height = 120.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    let total = |f: &FrameProfile| parts(f).iter().map(|(_, v)| v).sum::<f32>().max(f.step);
    let top = frames.iter().map(total).fold(1000.0 / 60.0, f32::max) * 1.1;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, theme::RADIUS, t.surface);
    let bar = rect.width() / SHOWN as f32;
    let start = rect.right() - bar * frames.len() as f32;
    let chosen = frames.len() - 1 - editor.profiler_view.back.unwrap_or(0).min(frames.len() - 1);
    for (i, frame) in frames.iter().enumerate() {
        let x = start + bar * i as f32;
        let mut y = rect.bottom();
        for ((_, ms), color) in parts(frame).iter().zip(colors) {
            let h = ms / top * rect.height();
            painter.rect_filled(egui::Rect::from_min_max(egui::pos2(x, y - h), egui::pos2(x + bar.max(1.0), y)), 0.0, *color);
            y -= h;
        }
        if i == chosen && editor.profiler_view.back.is_some() {
            painter.rect_stroke(
                egui::Rect::from_min_max(egui::pos2(x - 1.0, rect.top()), egui::pos2(x + bar + 1.0, rect.bottom())),
                0.0,
                egui::Stroke::new(1.5, t.focus),
                egui::StrokeKind::Outside,
            );
        }
    }
    let sixty = rect.bottom() - (1000.0 / 60.0) / top * rect.height();
    painter.hline(rect.x_range(), sixty, egui::Stroke::new(1.0, t.text_muted));
    painter.text(
        egui::pos2(rect.left() + 4.0, sixty - 2.0),
        egui::Align2::LEFT_BOTTOM,
        tl("60 frames a second"),
        egui::FontId::proportional(10.0),
        t.text_muted,
    );
    let average = frames.iter().map(total).sum::<f32>() / frames.len() as f32;
    let summary = tf("Frame times: {count} frames, {ms} ms on average", &[("count", &frames.len().to_string()), ("ms", &format!("{average:.2}"))]);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &summary));
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let i = ((pos.x - start) / bar).floor();
        if i >= 0.0 && (i as usize) < frames.len() {
            editor.profiler_view.back = Some(frames.len() - 1 - i as usize);
            // Looking at a frame pauses recording, so it stays in view.
            if editor.session.profiler().is_recording() {
                editor.dispatch("profile.stop", &json!({}), None);
            }
        }
    }
}

/// One frame: each part's time, the slowest scripts, and what was drawn.
fn details(ui: &mut egui::Ui, f: &FrameProfile, colors: &[egui::Color32; 7], gpu_timed: Option<bool>) {
    let t = theme::tokens(ui);
    let row = |ui: &mut egui::Ui, name: &str, value: String| {
        crate::value_editor::row(ui, t, name, |ui, _, _| {
            ui.label(value);
        });
    };
    theme::headed(t, "Frame").show(ui, |ui| {
        row(ui, "Play Time", format!("{:.2} s", f.time));
        row(ui, "Step", format!("{:.2} ms", f.step));
        for ((name, ms), color) in parts(f).iter().zip(colors) {
            crate::value_editor::row(ui, t, name, |ui, _, _| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, ROW_HEIGHT * 0.5), egui::Sense::hover());
                ui.painter().rect_filled(rect, 2.0, *color);
                ui.label(format!("{ms:.2} ms"));
            });
        }
        row(ui, "GPU", gpu_text(f.gpu, gpu_timed));
        row(ui, "Draw Calls", f.draw_calls.to_string());
        row(ui, "Triangles", f.triangles.to_string());
        row(ui, "Meshes", megabytes(f.mesh_bytes));
        row(ui, "Images", megabytes(f.image_bytes));
    });
    if !f.by_script.is_empty() {
        ui.add_space(theme::SECTION_GAP);
        theme::headed(t, "Scripts, Slowest First").show(ui, |ui| {
            for (name, ms) in f.by_script.iter().take(20) {
                row(ui, name, format!("{ms:.3} ms"));
            }
        });
    }
}
