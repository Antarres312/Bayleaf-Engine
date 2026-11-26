//! Files dropped on the editor from the system: brought into the project (`asset.import`) and,
//! depending on where they land, placed. On the Project panel they go into the folder under the
//! pointer (a folder row or tile, else the open folder). On the Scene view a model or a sound
//! becomes an object where the pointer meets the ground, and an image on an object becomes its
//! look. On the hierarchy, models and sounds become objects at the top. Windows gives no pointer
//! position with a drop until the pointer moves again, so a drop without one waits a moment for
//! it; with none by then, the files go into the Project panel's open folder.
//!
//! The dropped files wait in the frame's memory: the first part of the editor under the pointer
//! takes them, and whatever none took goes to the Project panel at the end of the frame.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::Editor;
use crate::i18n::{tf, tl};
use crate::theme;

/// How long a drop without a pointer position waits for one, in seconds.
const WAIT_FOR_POINTER: f64 = 0.4;

/// This frame's dropped files, and whether a part of the editor already showed it would take them.
#[derive(Clone, Default)]
struct Pending {
    files: Vec<PathBuf>,
    /// Dropped without a pointer position: held until one comes, or the wait is over.
    waiting: Vec<PathBuf>,
    /// When the held files were dropped (input time, seconds).
    since: Option<f64>,
    /// Files hover over the window (not dropped yet).
    hovering: bool,
    /// A part of the editor already ringed itself for the hovering files.
    ringed: bool,
}

fn id() -> egui::Id {
    egui::Id::unique("bayleaf dropped files")
}

/// At the start of the frame: what the system dropped or holds over the window.
pub(crate) fn begin(ctx: &egui::Context) {
    let (dropped, hovering, has_pointer, now) = ctx.input(|i| {
        let files: Vec<PathBuf> = i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect();
        (files, !i.raw.hovered_files.is_empty(), i.pointer.latest_pos().is_some(), i.time)
    });
    let before: Pending = ctx.data(|d| d.get_temp(id())).unwrap_or_default();
    let mut files = before.waiting;
    files.extend(dropped);
    let mut since = before.since;
    let waited_out = since.is_some_and(|at| now - at >= WAIT_FOR_POINTER);
    let (files, waiting) = if files.is_empty() || has_pointer || waited_out {
        since = None;
        (files, Vec::new())
    } else {
        since.get_or_insert(now);
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
        (Vec::new(), files)
    };
    ctx.data_mut(|d| d.insert_temp(id(), Pending { files, waiting, since, hovering, ringed: false }));
}

/// The files dropped this frame onto `rect`, taken so no other part takes them too. While files
/// hover over `rect`, it is ringed (only the first part asking, so nested parts ring once).
pub(crate) fn take(ui: &egui::Ui, rect: egui::Rect) -> Option<Vec<PathBuf>> {
    let ctx = ui.ctx();
    let over = ctx.input(|i| i.pointer.latest_pos()).is_some_and(|p| ui.clip_rect().intersect(rect).contains(p));
    if !over {
        return None;
    }
    let mut pending: Pending = ctx.data(|d| d.get_temp(id())).unwrap_or_default();
    if pending.hovering && !pending.ringed {
        pending.ringed = true;
        let t = theme::tokens(ui);
        ui.painter().rect_stroke(rect, theme::RADIUS, egui::Stroke::new(2.0, t.focus), egui::StrokeKind::Inside);
    }
    let files = (!pending.files.is_empty()).then(|| std::mem::take(&mut pending.files));
    ctx.data_mut(|d| d.insert_temp(id(), pending));
    files
}

/// Where dropped files go once they are in the project.
pub(crate) enum Place {
    /// Only into the project.
    Project,
    /// Into the scene: at `at` (world units), onto `target` for an image.
    Scene { at: Option<[f64; 3]>, target: Option<bayleaf_core::StableId> },
    /// Into the scene at the top of the hierarchy.
    Top,
}

/// At the end of the frame: files no part took go into the Project panel's open folder, and a
/// hint says what a drop does while files hover.
pub(crate) fn finish(editor: &mut Editor, ctx: &egui::Context) {
    let pending: Pending = ctx.data(|d| d.get_temp(id())).unwrap_or_default();
    if !pending.files.is_empty() {
        bring_in(editor, pending.files, None, Place::Project);
    }
    if pending.hovering {
        let t = theme::tokens_for(ctx);
        let hint = tl("Drop to bring the files into the project: onto a folder, the Scene view or the hierarchy");
        egui::Area::new(egui::Id::unique("drop hint")).order(egui::Order::Tooltip).anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -40.0)).show(
            ctx,
            |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.label(egui::RichText::new(hint).color(t.text));
                });
            },
        );
    }
}

/// The folder files go into when no folder is named: the Project panel's open folder when it is in
/// the project, else the project folder.
fn default_folder(editor: &Editor) -> PathBuf {
    let current = &editor.project.current;
    match editor.session.project_dir() {
        Some(project) if !current.as_os_str().is_empty() && current.starts_with(project) => current.clone(),
        _ => editor.session.script_dir(),
    }
}

/// Brings `files` into `folder` (or the default one) and places them as `place` says.
pub(crate) fn bring_in(editor: &mut Editor, files: Vec<PathBuf>, folder: Option<PathBuf>, place: Place) {
    let folder = folder.unwrap_or_else(|| default_folder(editor));
    let Some(result) = editor.dispatch("asset.import", &json!({ "files": files, "folder": folder }), None) else { return };
    let imported: Vec<(String, String)> = result["imported"]
        .as_array()
        .map(|list| list.iter().filter_map(|i| Some((i["path"].as_str()?.to_owned(), i["kind"].as_str()?.to_owned()))).collect())
        .unwrap_or_default();
    let base = editor.session.script_dir();
    if let Some((first, _)) = imported.first() {
        // The Project panel shows where they went.
        let first = base.join(first);
        if let Some(parent) = first.parent() {
            editor.project.current = parent.to_path_buf();
        }
        editor.project.selected = Some(first);
    }
    let mut placed = Vec::new();
    for (path, kind) in &imported {
        let made = match (&place, kind.as_str()) {
            (Place::Project, _) => None,
            (Place::Scene { at, .. }, "model" | "sound") => editor.dispatch("scene.file.place", &json!({ "path": path, "position": at }), None),
            (Place::Top, "model" | "sound") => editor.dispatch("scene.file.place", &json!({ "path": path }), None),
            (Place::Scene { target: Some(target), .. }, "image") => {
                editor.dispatch("scene.image.apply", &json!({ "id": target, "image": path }), None);
                None
            }
            _ => None,
        };
        if let Some(id) = made.as_ref().and_then(|v| v.get("id")).and_then(Value::as_str).and_then(|s| bayleaf_core::StableId::parse_hex(s).ok()) {
            placed.push(id);
        }
    }
    if let Some(last) = placed.last() {
        editor.select_only(Some(*last));
    }
    editor.status = tf("Brought in: {files}", &[("files", &crate::i18n::tn(imported.len() as u64, "{n} file", "{n} files"))]);
}
