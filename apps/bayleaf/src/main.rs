//! Bayleaf Engine editor (desktop).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::PathBuf;

use anyhow::Context;
use bayleaf_automation::control::{self, ControlServer};
use bayleaf_engine::confine::Confinement;
use bayleaf_ui_egui::Editor;
use clap::Parser;

/// Storage key of the panel layout. The value carries its own format version.
const LAYOUT_KEY: &str = "bayleaf.layout";

#[derive(Parser)]
#[command(name = "bayleaf", version, about = "Bayleaf Engine editor")]
struct Args {
    /// Scene file to open.
    scene: Option<PathBuf>,
    /// Let agents control this editor through a loopback control channel on this port
    /// (see docs/automation.md). Needs both automation roots.
    #[arg(long)]
    control: Option<u16>,
    /// Token file of the control channel; created with a new token when missing.
    /// Default: bayleaf-control.token in the system temp folder.
    #[arg(long = "control-token-file", requires = "control")]
    token_file: Option<PathBuf>,
    /// A folder agents may read. Repeat for more.
    #[arg(long = "automation-read-root", requires = "control")]
    read_roots: Vec<PathBuf>,
    /// A folder agents may write (and read). Repeat for more.
    #[arg(long = "automation-write-root", requires = "control")]
    write_roots: Vec<PathBuf>,
}

/// What the control channel needs once the window exists.
struct ControlSetup {
    port: u16,
    token: String,
    confinement: Confinement,
}

/// The running control channel and the folders its requests may touch.
struct Control {
    server: ControlServer,
    confinement: Confinement,
}

struct App {
    editor: Editor,
    control: Option<Control>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, scene: Option<PathBuf>, setup: Option<ControlSetup>) -> anyhow::Result<Self> {
        bayleaf_ui_egui::install_fonts(&cc.egui_ctx);
        let mut editor = Editor::default();
        if let Some(text) = cc.storage.and_then(|s| s.get_string(LAYOUT_KEY)) {
            // A layout from another version is ignored; the editor keeps its default layout.
            let _restored = editor.restore_layout(&text);
        }
        if let Some(scene) = &scene {
            editor.open_scene(scene);
        }
        let control = match setup {
            Some(ControlSetup { port, token, confinement }) => {
                let ctx = cc.egui_ctx.clone();
                let server = ControlServer::start(port, token, move || ctx.request_repaint())
                    .with_context(|| format!("cannot open the control channel on port {port}"))?;
                eprintln!("bayleaf: control channel listening on {}", server.addr());
                editor.set_automation_note(Some(server.addr().to_string()));
                Some(Control { server, confinement })
            }
            None => None,
        };
        Ok(Self { editor, control })
    }

    /// Answers the agents' requests on the UI thread, before the frame is drawn.
    fn serve_agents(&mut self) {
        let Some(control) = &self.control else { return };
        control.server.answer(&mut AgentHandler { editor: &mut self.editor, confinement: &control.confinement });
    }
}
/// The editor answering agents, inside their allowed folders.
struct AgentHandler<'a> {
    editor: &'a mut Editor,
    confinement: &'a Confinement,
}

impl control::Handler for AgentHandler<'_> {
    fn commands(&mut self) -> serde_json::Value {
        self.editor.automation_commands()
    }

    fn execute(&mut self, command: &str, params: &serde_json::Value) -> Result<serde_json::Value, String> {
        self.editor.execute_automation(command, params, self.confinement)
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, frame: &mut eframe::Frame) {
        self.serve_agents();
        self.editor.ui(ui, frame.wgpu_render_state());
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some(text) = self.editor.saved_layout() {
            storage.set_string(LAYOUT_KEY, text);
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    // Checked before the window opens, so a bad setup fails at once with a clear message (AA 18).
    let setup = match args.control {
        Some(port) => {
            let confinement = Confinement::new(&args.read_roots, &args.write_roots).map_err(anyhow::Error::msg)?;
            let path = args.token_file.clone().unwrap_or_else(|| std::env::temp_dir().join("bayleaf-control.token"));
            let token = control::load_or_create_token(&path).with_context(|| format!("cannot read or create {}", path.display()))?;
            eprintln!("bayleaf: control token in {}", path.display());
            Some(ControlSetup { port, token, confinement })
        }
        None => None,
    };
    let scene = args.scene;

    let mut viewport =
        eframe::egui::ViewportBuilder::default().with_title("Bayleaf Engine").with_inner_size([1440.0, 900.0]).with_min_inner_size([960.0, 600.0]);
    if let Some(icon) = bayleaf_ui_egui::brand::window_icon() {
        viewport = viewport.with_icon(eframe::egui::IconData { rgba: icon.rgba, width: icon.size, height: icon.size });
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "Bayleaf Engine",
        options,
        Box::new(move |cc| match App::new(cc, scene, setup) {
            Ok(app) => Ok(Box::new(app) as Box<dyn eframe::App>),
            Err(e) => Err(format!("{e:#}").into()),
        }),
    )
    .map_err(|e| anyhow::anyhow!("could not start the editor window: {e}"))
}
