//! Bayleaf Engine editor (desktop).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use bayleaf_ui_egui::Editor;

/// Storage key of the panel layout. The value carries its own format version.
const LAYOUT_KEY: &str = "bayleaf.layout";

struct App {
    editor: Editor,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut editor = Editor::default();
        if let Some(text) = cc.storage.and_then(|s| s.get_string(LAYOUT_KEY)) {
            // A layout from another version is ignored; the editor keeps its default layout.
            let _restored = editor.restore_layout(&text);
        }
        Self { editor }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, frame: &mut eframe::Frame) {
        self.editor.ui(ui, frame.wgpu_render_state());
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some(text) = self.editor.saved_layout() {
            storage.set_string(LAYOUT_KEY, text);
        }
    }
}

fn main() -> anyhow::Result<()> {
    let mut viewport =
        eframe::egui::ViewportBuilder::default().with_title("Bayleaf Engine").with_inner_size([1440.0, 900.0]).with_min_inner_size([960.0, 600.0]);
    if let Some(icon) = bayleaf_ui_egui::brand::window_icon() {
        viewport = viewport.with_icon(eframe::egui::IconData { rgba: icon.rgba, width: icon.size, height: icon.size });
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("Bayleaf Engine", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
        .map_err(|e| anyhow::anyhow!("could not start the editor window: {e}"))
}
