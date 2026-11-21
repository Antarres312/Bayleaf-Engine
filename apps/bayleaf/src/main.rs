//! Bayleaf Engine editor (desktop).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use bayleaf_ui_egui::Editor;

struct App {
    editor: Editor,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, frame: &mut eframe::Frame) {
        self.editor.ui(ui, frame.wgpu_render_state());
    }
}

fn main() -> anyhow::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native("Bayleaf Engine", options, Box::new(|_cc| Ok(Box::new(App { editor: Editor::default() }))))
        .map_err(|e| anyhow::anyhow!("could not start the editor window: {e}"))
}
