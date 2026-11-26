//! The Keyframes panel: the open keyframe animation's tracks under a time ruler.
//!
//! Each animated part of the prefab has a row, and each of its tracks a lane of keys; a key's
//! shape says how it eases to the next (a square steps, a diamond goes evenly, a circle goes
//! smoothly). The ruler moves the playhead, which poses the prefab without play mode. Every edit
//! goes through the `timeline.*` commands, so it undoes as one step.
//!
//! Keyboard: Tab reaches the ruler, the track rows and the keys. On the ruler, Left and Right move
//! the playhead (Shift: by a second), Home and End to the start and end. On a track row, Up and
//! Down go between tracks, Right goes to its keys, Enter or K places a key at the playhead. On a
//! key, Left and Right go to the next keys, Alt+Left and Alt+Right move the selected keys in time
//! (Alt+Shift: finely), Space adds or drops it from the selection, E changes how it eases, Delete
//! deletes the selected keys and Escape goes back to its track row. Ctrl+C and Ctrl+V copy and
//! paste keys (at the playhead).

use bayleaf_core::StableId;
use bayleaf_engine::scene::timeline::{Ease, Target, Timeline};
use bayleaf_engine::timeline_cmds::{KeyRef, TimelineDoc};
use egui::{Rect, Sense, pos2, vec2};
use serde_json::{Value, json};

use crate::Editor;
use crate::i18n::{tf, tl};
use crate::theme::{self, ROW_HEIGHT};

/// Width of the track names on the left.
const NAMES: f32 = 200.0;
/// Height of the time ruler.
const RULER: f32 = 24.0;
/// Size of a key's shape.
const KEY: f32 = 11.0;
/// Seconds an arrow nudges a key (Alt), and finely (Alt+Shift).
const NUDGE: f32 = 0.1;
const FINE: f32 = 0.01;
/// Seconds an arrow moves the playhead on the ruler (Shift: one second).
const STEP: f32 = 0.1;

/// What the keyboard works on next frame.
#[derive(Clone, Debug, PartialEq)]
enum Focus {
    Ruler,
    Track(StableId, Target),
    Key(StableId, Target, f32),
}

/// The panel's own state.
#[derive(Default)]
pub(crate) struct KeyframesView {
    /// Selected keys.
    selected: Vec<KeyRef>,
    /// The track the keyboard and paste work on.
    track: Option<(StableId, Target)>,
    /// A widget of the panel has the keyboard: plain keys belong to the panel.
    pub(crate) focused: bool,
    /// A key has the keyboard.
    on_key: bool,
    /// Where the focus goes next frame.
    focus: Option<Focus>,
    /// Seconds the selected keys are being dragged by.
    dragging: Option<f32>,
    /// The playhead runs with time, showing the animation as it plays.
    previewing: bool,
    /// The pose stays on show: it comes back after other edits put the values back.
    show_pose: bool,
    /// The last change, said to screen readers.
    announcement: String,
}

/// One row of the panel.
enum Row {
    Part { id: StableId, name: String },
    Track { part: StableId, target: Target, label: String },
}

/// Something the panel asks for, run once it is drawn.
enum Ask {
    Seek(f32),
    Run(&'static str, Value),
    /// A command whose runs share one undo step.
    Merged(&'static str, Value, String),
    Select(Vec<KeyRef>),
    Copy,
    TogglePreview,
}

fn same_key(a: &KeyRef, part: StableId, target: &Target, time: f32) -> bool {
    a.part == part && &a.track == target && (a.time - time).abs() <= bayleaf_engine::scene::timeline::SAME_TIME
}

/// A track's name: Position, Rotation and Scale for the transform, else the component and field.
fn track_label(editor: &Editor, target: &Target) -> String {
    match target {
        Target::Field { component, field } if component == "transform" => match field.as_str() {
            "position" => tl("Position").to_owned(),
            "rotation" => tl("Rotation").to_owned(),
            "scale" => tl("Scale").to_owned(),
            other => other.to_owned(),
        },
        Target::Field { component, field } => {
            let label = editor.session.scene().registry().find(component).map_or(component.as_str(), |k| k.label);
            tf("{component}: {field}", &[("component", tl(label)), ("field", field)])
        }
        Target::Property { script, property } => {
            let name = bayleaf_engine::scene::components::ScriptRef { module: script.clone(), properties: Default::default() }.display_name();
            tf("{component}: {field}", &[("component", &name), ("field", property)])
        }
    }
}

fn rows(editor: &Editor, timeline: &Timeline) -> Vec<Row> {
    let names = editor.session.timeline_prefab_objects();
    let mut rows = Vec::new();
    for part in &timeline.parts {
        let name = names.iter().find(|(id, _, _)| *id == part.id).map_or_else(|| tl("Missing part").to_owned(), |(_, _, n)| n.clone());
        rows.push(Row::Part { id: part.id, name });
        for track in &part.tracks {
            rows.push(Row::Track { part: part.id, target: track.target.clone(), label: track_label(editor, &track.target) });
        }
    }
    rows
}

/// The tracks a part could get: its transform, the number and true/false fields of its other
/// components, and its scripts' properties set on it.
fn track_choices(editor: &Editor, object: Option<StableId>) -> Vec<Target> {
    let mut out: Vec<Target> = ["position", "rotation", "scale"].into_iter().map(|f| Target::field("transform", f)).collect();
    let Some(object) = object else { return out };
    let scene = editor.session.scene();
    for (component, value) in scene.components(object).unwrap_or_default() {
        if matches!(component, "name" | "transform" | "prefab" | "scripts" | "tags") {
            continue;
        }
        let Some(fields) = value.as_object() else { continue };
        for (field, v) in fields {
            let keyable = v.is_number() || v.is_boolean() || v.as_array().is_some_and(|a| !a.is_empty() && a.len() <= 16 && a.iter().all(Value::is_number));
            if keyable {
                out.push(Target::field(component, field));
            }
        }
    }
    if let Some(scripts) = scene.get::<bayleaf_engine::scene::components::Scripts>(object) {
        for script in &scripts.0 {
            for name in script.properties.keys() {
                out.push(Target::property(&script.module, name));
            }
        }
    }
    out
}

/// Makes a keyframe animation beside the prefab at `prefab` (a path in the project) and opens it.
pub(crate) fn new_for_prefab(editor: &mut Editor, prefab: &std::path::Path) {
    let folder = prefab.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    let stem = prefab.file_stem().map_or_else(|| "Animation".to_owned(), |s| s.to_string_lossy().into_owned());
    let mut path = folder.join(format!("{stem}.{}", bayleaf_engine::scene::timeline::EXTENSION));
    let mut n = 2;
    while path.exists() {
        path = folder.join(format!("{stem} {n}.{}", bayleaf_engine::scene::timeline::EXTENSION));
        n += 1;
    }
    if editor.dispatch("timeline.new", &json!({ "prefab": prefab, "path": path }), None).is_some() {
        editor.keyframes_view = KeyframesView { show_pose: true, ..KeyframesView::default() };
        editor.project.rescan_soon();
    }
}

/// Opens the keyframe animation at `path` in the panel.
pub(crate) fn open(editor: &mut Editor, path: &std::path::Path) {
    if editor.dispatch("timeline.open", &json!({ "path": path }), None).is_some() {
        editor.keyframes_view = KeyframesView { show_pose: true, ..KeyframesView::default() };
    }
}

/// Saves the open keyframe animation (Ctrl+S while the panel has the keyboard).
pub(crate) fn save(editor: &mut Editor) {
    if editor.session.timeline().is_some() {
        editor.dispatch("timeline.save", &json!({}), None);
    }
}

/// Ctrl+C and Ctrl+V while the panel has the keyboard.
pub(crate) fn clipboard_keys(editor: &mut Editor, copy: bool, paste: Option<String>) {
    if copy {
        copy_selected(editor);
    }
    if let Some(text) = paste {
        paste_text(editor, &text);
    }
}

fn copy_selected(editor: &mut Editor) {
    let keys = editor.keyframes_view.selected.clone();
    if keys.is_empty() {
        return;
    }
    if let Some(text) = editor.dispatch("timeline.key.copy", &json!({ "keys": keys }), None).and_then(|r| r["text"].as_str().map(str::to_owned)) {
        editor.copy_out = Some(text);
        editor.keyframes_view.announcement = tf("{count} copied", &[("count", &crate::i18n::tn(keys.len() as u64, "1 key", "{n} keys"))]);
    }
}

fn paste_text(editor: &mut Editor, text: &str) {
    // Keys copied from one track go onto the track in use; from several, onto their own.
    let one_track = serde_json::from_str::<Value>(text).ok().and_then(|v| {
        let keys = v["keys"].as_array()?.clone();
        let first = keys.first()?.clone();
        keys.iter().all(|k| k["part"] == first["part"] && k["track"] == first["track"]).then_some(())
    });
    let mut params = json!({ "text": text });
    if let (Some(()), Some((part, target))) = (one_track, editor.keyframes_view.track.clone()) {
        params["part"] = json!(part);
        params["track"] = json!(target);
    }
    if let Some(result) = editor.dispatch("timeline.key.paste", &params, None) {
        let keys: Vec<KeyRef> = serde_json::from_value(result["keys"].clone()).unwrap_or_default();
        editor.keyframes_view.announcement = tf("{count} pasted", &[("count", &crate::i18n::tn(keys.len() as u64, "1 key", "{n} keys"))]);
        editor.keyframes_view.selected = keys;
    }
}

pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    let t = theme::tokens(ui);
    editor.keyframes_view.focused = false;
    editor.keyframes_view.on_key = false;
    let Some(doc) = editor.session.timeline().cloned() else {
        empty(editor, ui);
        return;
    };
    let posable = editor.session.timeline_binding().is_some();
    // The pose comes back after another edit put the values back.
    if editor.keyframes_view.show_pose && posable && !doc.is_posed() && !editor.session.is_playing() {
        let _ = editor.registry.execute(&mut editor.session, "timeline.seek", &json!({ "time": doc.playhead }));
    }
    let mut asks: Vec<Ask> = Vec::new();
    toolbar(editor, ui, &doc, &mut asks);
    if !posable {
        ui.add(
            egui::Label::new(
                egui::RichText::new(tf(
                    "Open {prefab} (double-click it in the Project panel) or place a copy of it in the scene to see the pose and record keys.",
                    &[("prefab", &doc.timeline.prefab)],
                ))
                .color(t.warning),
            )
            .wrap(),
        );
    }
    ui.separator();
    let rows = rows(editor, &doc.timeline);
    let choices: Vec<(StableId, Option<StableId>, String)> = editor.session.timeline_prefab_objects();
    if rows.is_empty() {
        ui.add(
            egui::Label::new(
                egui::RichText::new(tl("No parts yet: add one with Add Part, or turn on Record and move a part in the scene view.")).color(t.text_muted),
            )
            .wrap(),
        );
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        lanes(editor, ui, &doc, &rows, &choices, &mut asks);
    });
    // The focused key, and how it eases.
    selected_bar(editor, ui, &mut asks);
    let said = ui.add(egui::Label::new(egui::RichText::new(editor.keyframes_view.announcement.clone()).small().color(t.text_muted)));
    ui.ctx().accesskit_node_builder(said.id, |node| node.set_live(egui::accesskit::Live::Polite));
    preview(editor, ui, &doc, &mut asks);
    run(editor, asks);
}

/// No keyframe animation is open: say how to make one.
fn empty(editor: &mut Editor, ui: &mut egui::Ui) {
    let t = theme::tokens(ui);
    ui.add(
        egui::Label::new(
            egui::RichText::new(tl("No keyframe animation is open. Right-click a prefab in the Project panel and choose New Keyframe Animation, or double-click a .btimeline file."))
                .color(t.text_muted),
        )
        .wrap(),
    );
    if let Some(prefab) = editor.session.open_prefabs().last().cloned()
        && ui.button(tf("New Keyframe Animation for {prefab}", &[("prefab", &prefab)])).clicked()
    {
        let path = editor.session.script_dir().join(&prefab);
        new_for_prefab(editor, &path);
    }
}

fn toolbar(editor: &mut Editor, ui: &mut egui::Ui, doc: &TimelineDoc, asks: &mut Vec<Ask>) {
    let t = theme::tokens(ui);
    ui.horizontal_wrapped(|ui| {
        let name = doc.path.as_deref().and_then(|p| p.file_name()).map_or_else(|| tl("Untitled").to_owned(), |n| n.to_string_lossy().into_owned());
        let shown = if doc.dirty { format!("{name} •") } else { name.clone() };
        let title = ui.label(egui::RichText::new(shown).strong());
        if doc.dirty {
            ui.ctx().accesskit_node_builder(title.id, |node| node.set_label(tf("{file}, not saved", &[("file", &name)])));
        }
        if ui.button(tl("Save")).on_hover_text(tl("Save the keyframe animation (Ctrl+S)")).clicked() {
            asks.push(Ask::Run("timeline.save", json!({})));
        }
        if doc.dirty {
            ui.menu_button(tl("Close"), |ui| {
                if ui.button(tl("Save and Close")).clicked() {
                    asks.push(Ask::Run("timeline.save", json!({})));
                    asks.push(Ask::Run("timeline.close", json!({})));
                    ui.close();
                }
                if ui.button(tl("Discard Changes")).clicked() {
                    asks.push(Ask::Run("timeline.close", json!({ "discard": true })));
                    ui.close();
                }
            });
        } else if ui.button(tl("Close")).clicked() {
            asks.push(Ask::Run("timeline.close", json!({})));
        }
        ui.separator();
        let mut previewing = editor.keyframes_view.previewing;
        if ui.toggle_value(&mut previewing, tl("Play Preview")).on_hover_text(tl("Run the playhead to show the animation, without play mode (Space)")).changed()
        {
            asks.push(Ask::TogglePreview);
        }
        let mut recording = doc.recording;
        if ui
            .toggle_value(&mut recording, tl("Record"))
            .on_hover_text(tl("While on, changing a part in the scene view or the Inspector places a key at the playhead"))
            .changed()
        {
            asks.push(Ask::Run("timeline.record", json!({ "on": recording })));
        }
        if recording {
            ui.label(egui::RichText::new(tl("Recording")).color(t.error).strong());
        }
        ui.separator();
        ui.label(tl("Length"));
        let mut length = doc.timeline.length;
        let drag = ui.add(egui::DragValue::new(&mut length).range(0.05..=bayleaf_engine::scene::timeline::MAX_LENGTH).speed(0.05).suffix(" s").max_decimals(2));
        ui.ctx().accesskit_node_builder(drag.id, |node| node.set_label(tl("Length in seconds")));
        if drag.changed() && (length - doc.timeline.length).abs() > f32::EPSILON {
            asks.push(Ask::Merged("timeline.set", json!({ "length": length }), format!("keyframes length {}", editor.gesture)));
        }
        let mut looping = doc.timeline.looping;
        if ui.checkbox(&mut looping, tl("Loops")).on_hover_text(tl("Starts over at its end; otherwise it plays once and stays at its end")).changed() {
            asks.push(Ask::Run("timeline.set", json!({ "looping": looping })));
        }
        ui.separator();
        let objects = editor.session.timeline_prefab_objects();
        ui.menu_button(tl("Add Part"), |ui| {
            let mut any = false;
            for (part, _, name) in objects.iter().filter(|(id, _, _)| doc.timeline.part(*id).is_none()) {
                any = true;
                if ui.button(name).clicked() {
                    asks.push(Ask::Run("timeline.part.add", json!({ "part": part })));
                    ui.close();
                }
            }
            if !any {
                ui.label(egui::RichText::new(tl("Every object of the prefab takes part")).color(t.text_muted));
            }
        });
        if doc.is_posed() && ui.button(tl("Show Without Animation")).on_hover_text(tl("Put back the values the pose replaced")).clicked() {
            editor.keyframes_view.show_pose = false;
            editor.keyframes_view.previewing = false;
            asks.push(Ask::Run("timeline.pose.end", json!({})));
        }
    });
}

/// The ruler, the names on the left and the key lanes.
fn lanes(editor: &mut Editor, ui: &mut egui::Ui, doc: &TimelineDoc, rows: &[Row], choices: &[(StableId, Option<StableId>, String)], asks: &mut Vec<Ask>) {
    let t = theme::tokens(ui);
    let width = ui.available_width().max(NAMES + 120.0);
    let height = RULER + ROW_HEIGHT * rows.len() as f32;
    let (area, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let lane_left = area.left() + NAMES;
    let lane_right = area.right() - KEY;
    let length = doc.timeline.length.max(0.05);
    let per_second = (lane_right - lane_left) / length;
    let axis = Axis { left: lane_left, per_second, length };
    let x_of = |time: f32| axis.x_of(time);
    let time_of = |x: f32| axis.time_of(x);
    let painter = ui.painter_at(area);

    // The ruler.
    let ruler = Rect::from_min_max(pos2(lane_left, area.top()), pos2(area.right(), area.top() + RULER));
    let ruler_id = ui.scope_id().with("keyframes ruler");
    let response = ui.interact(ruler, ruler_id, Sense::click_and_drag());
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::Role::Slider, true, tl("Playhead"));
        info.value = Some(f64::from(doc.playhead));
        info
    });
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_label(tl("Playhead"));
        node.set_numeric_value(f64::from(doc.playhead));
        node.set_min_numeric_value(0.0);
        node.set_max_numeric_value(f64::from(length));
        node.set_value(tf("{time} seconds", &[("time", &format!("{:.2}", doc.playhead))]));
    });
    if editor.keyframes_view.focus == Some(Focus::Ruler) {
        response.request_focus();
        editor.keyframes_view.focus = None;
    }
    if let Some(at) = response.interact_pointer_pos().filter(|_| response.clicked() || response.dragged()) {
        editor.keyframes_view.show_pose = true;
        asks.push(Ask::Seek(time_of(at.x)));
    }
    if response.has_focus() {
        editor.keyframes_view.focused = true;
        ui.memory_mut(|m| m.set_focus_lock_filter(response.id, egui::EventFilter { horizontal_arrows: true, ..Default::default() }));
        let (left, right, home, end, shift) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::Home),
                i.key_pressed(egui::Key::End),
                i.modifiers.shift,
            )
        });
        let step = if shift { 1.0 } else { STEP };
        let to = if home {
            Some(0.0)
        } else if end {
            Some(length)
        } else if left {
            Some((doc.playhead - step).max(0.0))
        } else if right {
            Some((doc.playhead + step).min(length))
        } else {
            None
        };
        if let Some(to) = to {
            editor.keyframes_view.show_pose = true;
            asks.push(Ask::Seek(to));
        }
    }
    painter.rect_filled(ruler, theme::RADIUS, t.surface);
    let marks = ruler_step(length, per_second);
    let mut mark = 0.0;
    while mark <= length + 1e-4 {
        let x = x_of(mark);
        painter.vline(x, ruler.y_range(), egui::Stroke::new(1.0, t.divider));
        painter.text(pos2(x + 3.0, ruler.center().y), egui::Align2::LEFT_CENTER, format!("{mark:.2}"), egui::FontId::proportional(11.0), t.text_muted);
        mark += marks;
    }
    if response.has_focus() {
        painter.rect_stroke(ruler, theme::RADIUS, egui::Stroke::new(2.0, t.focus), egui::StrokeKind::Inside);
    }

    // The rows.
    let tracks: Vec<(StableId, Target)> = rows
        .iter()
        .filter_map(|r| match r {
            Row::Track { part, target, .. } => Some((*part, target.clone())),
            Row::Part { .. } => None,
        })
        .collect();
    for (i, row) in rows.iter().enumerate() {
        let top = area.top() + RULER + ROW_HEIGHT * i as f32;
        let names = Rect::from_min_size(pos2(area.left(), top), vec2(NAMES - 4.0, ROW_HEIGHT));
        let lane = Rect::from_min_max(pos2(lane_left, top), pos2(area.right(), top + ROW_HEIGHT));
        if i % 2 == 1 {
            painter.rect_filled(Rect::from_min_max(names.min, lane.max), 0.0, t.stripe);
        }
        match row {
            Row::Part { id, name } => part_row(editor, ui, doc, (*id, name), names, choices, asks),
            Row::Track { part, target, label } => {
                let track = doc.timeline.part(*part).and_then(|p| p.track(target));
                let keys = track.map(|t| t.keys.clone()).unwrap_or_default();
                let at = tracks.iter().position(|(p, tg)| p == part && tg == target).unwrap_or(0);
                let row = TrackRow { part: *part, target, label };
                track_row(editor, ui, doc, &row, names, Nav { at, tracks: &tracks, count: keys.len() }, asks);
                lane_row(editor, ui, doc, &row, (lane, &keys), &axis, asks);
            }
        }
    }
    // The playhead, over the lanes.
    let x = x_of(doc.playhead);
    painter.vline(x, area.top()..=area.bottom(), egui::Stroke::new(2.0, t.accent));
    painter.add(egui::Shape::convex_polygon(
        vec![pos2(x - 5.0, area.top()), pos2(x + 5.0, area.top()), pos2(x, area.top() + 7.0)],
        t.accent,
        egui::Stroke::NONE,
    ));
}

/// A readable step between the ruler's marks: at least 60 points apart.
fn ruler_step(length: f32, per_second: f32) -> f32 {
    [0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 300.0, 600.0].into_iter().find(|s| s * per_second >= 60.0).unwrap_or(length)
}

/// The time axis of the lanes.
struct Axis {
    left: f32,
    per_second: f32,
    length: f32,
}

impl Axis {
    fn x_of(&self, time: f32) -> f32 {
        self.left + time * self.per_second
    }

    fn time_of(&self, x: f32) -> f32 {
        ((x - self.left) / self.per_second).clamp(0.0, self.length)
    }
}

/// A track's row: whose, what it animates and its name.
struct TrackRow<'a> {
    part: StableId,
    target: &'a Target,
    label: &'a str,
}

/// Where a track row stands among the tracks, for Up and Down, and how many keys it has.
struct Nav<'a> {
    at: usize,
    tracks: &'a [(StableId, Target)],
    count: usize,
}

fn part_row(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    doc: &TimelineDoc,
    (part, name): (StableId, &str),
    rect: Rect,
    choices: &[(StableId, Option<StableId>, String)],
    asks: &mut Vec<Ask>,
) {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.label(egui::RichText::new(name).strong());
    let object = choices.iter().find(|(id, _, _)| *id == part).and_then(|(_, o, _)| *o);
    let have: Vec<Target> = doc.timeline.part(part).map(|p| p.tracks.iter().map(|t| t.target.clone()).collect()).unwrap_or_default();
    let options: Vec<(Target, String)> =
        track_choices(editor, object).into_iter().filter(|c| !have.contains(c)).map(|c| (c.clone(), track_label(editor, &c))).collect();
    let menu = child.menu_button("+", |ui| {
        for (target, label) in &options {
            if ui.button(label).clicked() {
                asks.push(Ask::Run("timeline.track.add", json!({ "part": part, "track": target })));
                ui.close();
            }
        }
        if options.is_empty() {
            ui.label(tl("Nothing more to animate"));
        }
        ui.separator();
        if ui.button(tl("Remove Part")).clicked() {
            asks.push(Ask::Run("timeline.part.remove", json!({ "part": part })));
            ui.close();
        }
    });
    let label = tf("Add Track to {part}", &[("part", name)]);
    ui.ctx().accesskit_node_builder(menu.response.id, |node| node.set_label(label.clone()));
    menu.response.on_hover_text(label);
}

fn track_row(editor: &mut Editor, ui: &mut egui::Ui, doc: &TimelineDoc, row: &TrackRow<'_>, rect: Rect, nav: Nav<'_>, asks: &mut Vec<Ask>) {
    let (part, target, label) = (row.part, row.target, row.label);
    let Nav { at, tracks, count } = nav;
    let t = theme::tokens(ui);
    let id = ui.scope_id().with(("keyframes track", part, target.describe()));
    let response = ui.interact(rect, id, Sense::click());
    let in_use = editor.keyframes_view.track.as_ref().is_some_and(|(p, tg)| *p == part && tg == target);
    let name = tf("{track}, {count}", &[("track", label), ("count", &crate::i18n::tn(count as u64, "1 key", "{n} keys"))]);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, in_use, &name));
    if editor.keyframes_view.focus == Some(Focus::Track(part, target.clone())) {
        response.request_focus();
        editor.keyframes_view.focus = None;
    }
    if response.clicked() {
        response.request_focus();
    }
    if response.has_focus() {
        editor.keyframes_view.focused = true;
        editor.keyframes_view.track = Some((part, target.clone()));
        ui.memory_mut(|m| m.set_focus_lock_filter(response.id, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() }));
        let (up, down, right, place) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::K),
            )
        });
        if up && at > 0 {
            editor.keyframes_view.focus = Some(Focus::Track(tracks[at - 1].0, tracks[at - 1].1.clone()));
        } else if up {
            editor.keyframes_view.focus = Some(Focus::Ruler);
        }
        if down && at + 1 < tracks.len() {
            editor.keyframes_view.focus = Some(Focus::Track(tracks[at + 1].0, tracks[at + 1].1.clone()));
        }
        if right {
            let keys = doc.timeline.part(part).and_then(|p| p.track(target)).map(|t| t.keys.clone()).unwrap_or_default();
            if let Some(key) = keys.iter().find(|k| k.time >= doc.playhead - 1e-4).or(keys.last()) {
                let r = KeyRef { part, track: target.clone(), time: key.time };
                editor.keyframes_view.focus = Some(Focus::Key(part, target.clone(), key.time));
                asks.push(Ask::Select(vec![r]));
                asks.push(Ask::Seek(key.time));
            }
        }
        if place {
            asks.push(Ask::Run("timeline.key.set", json!({ "part": part, "track": target, "time": doc.playhead })));
            editor.keyframes_view.announcement = tf("Key placed at {time} seconds", &[("time", &format!("{:.2}", doc.playhead))]);
        }
    }
    let painter = ui.painter();
    if in_use || response.has_focus() {
        painter.rect_filled(rect, theme::RADIUS, t.surface_active);
    }
    painter.text(pos2(rect.left() + 14.0, rect.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(13.0), t.text);
    if response.has_focus() {
        painter.rect_stroke(rect, theme::RADIUS, egui::Stroke::new(2.0, t.focus), egui::StrokeKind::Inside);
    }
    response.context_menu(|ui| {
        if ui.button(tl("Add Key at Playhead")).clicked() {
            asks.push(Ask::Run("timeline.key.set", json!({ "part": part, "track": target, "time": doc.playhead })));
            ui.close();
        }
        if ui.button(tl("Remove Track")).clicked() {
            asks.push(Ask::Run("timeline.track.remove", json!({ "part": part, "track": target })));
            ui.close();
        }
    });
}

fn lane_row(
    editor: &mut Editor,
    ui: &mut egui::Ui,
    doc: &TimelineDoc,
    row: &TrackRow<'_>,
    (lane, keys): (Rect, &[bayleaf_engine::scene::timeline::Key]),
    axis: &Axis,
    asks: &mut Vec<Ask>,
) {
    let (part, target, label) = (row.part, row.target, row.label);
    let x_of = |time: f32| axis.x_of(time);
    let time_of = |x: f32| axis.time_of(x);
    let t = theme::tokens(ui);
    // The lane itself: a double-click places a key there.
    let lane_id = ui.scope_id().with(("keyframes lane", part, target.describe()));
    let lane_response = ui.interact(lane, lane_id, Sense::click());
    lane_response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Group, true, tf("Keys of {track}", &[("track", label)])));
    if lane_response.double_clicked()
        && let Some(at) = lane_response.interact_pointer_pos()
    {
        asks.push(Ask::Run("timeline.key.set", json!({ "part": part, "track": target, "time": time_of(at.x) })));
    }
    ui.painter().hline(lane.x_range(), lane.center().y, egui::Stroke::new(1.0, t.divider));
    let view = &editor.keyframes_view;
    let selected: Vec<bool> = keys.iter().map(|k| view.selected.iter().any(|s| same_key(s, part, target, k.time))).collect();
    let offset = view.dragging.unwrap_or(0.0);
    for (index, key) in keys.iter().enumerate() {
        let is_selected = selected[index];
        let shown = if is_selected { (key.time + offset).clamp(0.0, doc.timeline.length) } else { key.time };
        let center = pos2(x_of(shown), lane.center().y);
        let rect = Rect::from_center_size(center, vec2(KEY + 4.0, ROW_HEIGHT - 4.0));
        let id = ui.scope_id().with(("keyframes key", part, target.describe(), index));
        let response = ui.interact(rect, id, Sense::click_and_drag());
        let name = tf("Key of {track} at {time} seconds, {ease}", &[("track", label), ("time", &format!("{:.2}", key.time)), ("ease", tl(key.ease.label()))]);
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, is_selected, &name));
        let this = KeyRef { part, track: target.clone(), time: key.time };
        if editor.keyframes_view.focus == Some(Focus::Key(part, target.clone(), key.time)) {
            response.request_focus();
            editor.keyframes_view.focus = None;
        }
        // A click picks the key (Ctrl or Shift: adds it) and shows its moment.
        if response.clicked() {
            response.request_focus();
            let adding = ui.input(|i| i.modifiers.command || i.modifiers.shift);
            let mut picked = if adding { editor.keyframes_view.selected.clone() } else { Vec::new() };
            if adding && is_selected {
                picked.retain(|s| !same_key(s, part, target, key.time));
            } else {
                picked.push(this.clone());
            }
            asks.push(Ask::Select(picked));
            asks.push(Ask::Seek(key.time));
        }
        // A drag moves the selected keys together.
        if response.drag_started() && !is_selected {
            asks.push(Ask::Select(vec![this.clone()]));
        }
        if response.dragged() {
            let by = editor.keyframes_view.dragging.unwrap_or(0.0) + response.drag_delta().x / (x_of(1.0) - x_of(0.0));
            editor.keyframes_view.dragging = Some(by);
        }
        if response.drag_stopped()
            && let Some(by) = editor.keyframes_view.dragging.take()
        {
            let by = (by * 100.0).round() / 100.0;
            let moving = if is_selected { editor.keyframes_view.selected.clone() } else { vec![this.clone()] };
            if by != 0.0 {
                asks.push(Ask::Run("timeline.key.move", json!({ "keys": moving, "by": by })));
            }
        }
        if response.has_focus() {
            editor.keyframes_view.focused = true;
            editor.keyframes_view.on_key = true;
            editor.keyframes_view.track = Some((part, target.clone()));
            key_keys(editor, ui, &response, doc, (part, target), (keys, index), asks);
        }
        response.context_menu(|ui| {
            for ease in Ease::ALL {
                if ui.radio(key.ease == ease, tl(ease.label())).clicked() {
                    asks.push(Ask::Run("timeline.key.set", json!({ "part": part, "track": target, "time": key.time, "ease": ease })));
                    ui.close();
                }
            }
            ui.separator();
            if ui.button(tl("Copy")).clicked() {
                if !is_selected {
                    asks.push(Ask::Select(vec![this.clone()]));
                }
                asks.push(Ask::Copy);
                ui.close();
            }
            if ui.button(tl("Delete")).clicked() {
                let keys = if is_selected { editor.keyframes_view.selected.clone() } else { vec![this.clone()] };
                asks.push(Ask::Run("timeline.key.remove", json!({ "keys": keys })));
                ui.close();
            }
        });
        draw_key(ui.painter(), center, key.ease, is_selected, response.has_focus() || response.hovered(), t);
    }
}

/// A key: its shape says how it eases (square steps, diamond even, circle smooth); a selected key is
/// filled with the accent and ringed, so colour is not the only sign.
fn draw_key(painter: &egui::Painter, center: egui::Pos2, ease: Ease, selected: bool, hot: bool, t: &theme::Tokens) {
    let r = KEY / 2.0;
    let fill = if selected { t.accent } else { t.text };
    let stroke = egui::Stroke::new(1.0, t.panel);
    match ease {
        Ease::Step => {
            painter.rect(Rect::from_center_size(center, vec2(KEY - 2.0, KEY - 2.0)), 1.0, fill, stroke, egui::StrokeKind::Inside);
        }
        Ease::Linear => {
            painter.add(egui::Shape::convex_polygon(
                vec![pos2(center.x, center.y - r), pos2(center.x + r, center.y), pos2(center.x, center.y + r), pos2(center.x - r, center.y)],
                fill,
                stroke,
            ));
        }
        Ease::Smooth => {
            painter.circle(center, r - 0.5, fill, stroke);
        }
    }
    if selected || hot {
        painter.circle_stroke(center, r + 3.0, egui::Stroke::new(if hot { 2.0 } else { 1.5 }, t.focus));
    }
}

/// The keys a focused key answers.
fn key_keys(
    editor: &mut Editor,
    ui: &egui::Ui,
    response: &egui::Response,
    doc: &TimelineDoc,
    (part, target): (StableId, &Target),
    (keys, index): (&[bayleaf_engine::scene::timeline::Key], usize),
    asks: &mut Vec<Ask>,
) {
    ui.memory_mut(|m| {
        m.set_focus_lock_filter(response.id, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, escape: true, ..Default::default() })
    });
    let key = &keys[index];
    let this = KeyRef { part, track: target.clone(), time: key.time };
    let (left, right, alt, shift, space, ease, delete, escape) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::ArrowLeft),
            i.key_pressed(egui::Key::ArrowRight),
            i.modifiers.alt,
            i.modifiers.shift,
            i.key_pressed(egui::Key::Space),
            i.key_pressed(egui::Key::E),
            i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
            i.key_pressed(egui::Key::Escape),
        )
    });
    let view = &editor.keyframes_view;
    let chosen = if view.selected.iter().any(|s| same_key(s, part, target, key.time)) { view.selected.clone() } else { vec![this.clone()] };
    if (left || right) && alt {
        // Nudge the selected keys; the focus follows this key to its new moment.
        let by = if shift { FINE } else { NUDGE } * if left { -1.0 } else { 1.0 };
        let to = (key.time + by).clamp(0.0, doc.timeline.length);
        asks.push(Ask::Run("timeline.key.move", json!({ "keys": chosen, "by": by })));
        editor.keyframes_view.focus = Some(Focus::Key(part, target.clone(), to));
        editor.keyframes_view.announcement = tf("Key moved to {time} seconds", &[("time", &format!("{to:.2}"))]);
    } else if left || right {
        let next = if left { index.checked_sub(1) } else { Some(index + 1).filter(|n| *n < keys.len()) };
        if let Some(next) = next {
            let time = keys[next].time;
            editor.keyframes_view.focus = Some(Focus::Key(part, target.clone(), time));
            asks.push(Ask::Select(vec![KeyRef { part, track: target.clone(), time }]));
            asks.push(Ask::Seek(time));
        }
    }
    if space {
        let mut picked = editor.keyframes_view.selected.clone();
        if picked.iter().any(|s| same_key(s, part, target, key.time)) {
            picked.retain(|s| !same_key(s, part, target, key.time));
        } else {
            picked.push(this.clone());
        }
        asks.push(Ask::Select(picked));
    }
    if ease {
        let next = Ease::ALL[(Ease::ALL.iter().position(|e| *e == key.ease).unwrap_or(0) + 1) % Ease::ALL.len()];
        let merge = format!("keyframes ease {}", ui.ctx().cumulative_pass_nr());
        for k in &chosen {
            asks.push(Ask::Merged("timeline.key.set", json!({ "part": k.part, "track": k.track, "time": k.time, "ease": next }), merge.clone()));
        }
        editor.keyframes_view.announcement = tf("Eases: {ease}", &[("ease", tl(next.label()))]);
    }
    if delete {
        asks.push(Ask::Run("timeline.key.remove", json!({ "keys": chosen })));
        asks.push(Ask::Select(Vec::new()));
        editor.keyframes_view.focus = Some(Focus::Track(part, target.clone()));
        editor.keyframes_view.announcement = tf("{count} deleted", &[("count", &crate::i18n::tn(chosen.len() as u64, "1 key", "{n} keys"))]);
    }
    if escape {
        editor.keyframes_view.focus = Some(Focus::Track(part, target.clone()));
    }
}

/// The selected keys' ease, to change by mouse.
fn selected_bar(editor: &mut Editor, ui: &mut egui::Ui, asks: &mut Vec<Ask>) {
    let selected = editor.keyframes_view.selected.clone();
    let Some(doc) = editor.session.timeline() else { return };
    let eases: Vec<Ease> = selected
        .iter()
        .filter_map(|k| {
            let track = doc.timeline.part(k.part)?.track(&k.track)?;
            track.key_at(k.time).map(|i| track.keys[i].ease)
        })
        .collect();
    if eases.is_empty() {
        return;
    }
    ui.horizontal(|ui| {
        ui.label(crate::i18n::tn(eases.len() as u64, "1 key selected", "{n} keys selected"));
        ui.label(tl("Eases"));
        for ease in Ease::ALL {
            let all = eases.iter().all(|e| *e == ease);
            if ui.selectable_label(all, tl(ease.label())).clicked() {
                let merge = format!("keyframes ease {}", ui.ctx().cumulative_pass_nr());
                for k in &selected {
                    asks.push(Ask::Merged("timeline.key.set", json!({ "part": k.part, "track": k.track, "time": k.time, "ease": ease }), merge.clone()));
                }
            }
        }
    });
}

/// While the preview plays, the playhead runs with time.
fn preview(editor: &mut Editor, ui: &egui::Ui, doc: &TimelineDoc, asks: &mut Vec<Ask>) {
    // Space on a key selects it; on the ruler or a track row it plays and pauses.
    let view = &editor.keyframes_view;
    if view.focused && !view.on_key && !ui.ctx().text_edit_focused() && ui.input(|i| i.key_pressed(egui::Key::Space)) {
        asks.push(Ask::TogglePreview);
    }
    if !editor.keyframes_view.previewing || editor.session.is_playing() {
        return;
    }
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    let mut time = doc.playhead + dt;
    if time > doc.timeline.length {
        if doc.timeline.looping {
            time = time.rem_euclid(doc.timeline.length);
        } else {
            time = doc.timeline.length;
            editor.keyframes_view.previewing = false;
        }
    }
    asks.push(Ask::Seek(time));
    ui.ctx().request_repaint();
}

fn run(editor: &mut Editor, asks: Vec<Ask>) {
    for ask in asks {
        match ask {
            // Moving the playhead is not worth a status line each frame.
            Ask::Seek(time) => {
                let _ = editor.registry.execute(&mut editor.session, "timeline.seek", &json!({ "time": time }));
            }
            Ask::Run(id, params) => {
                let moved = editor.dispatch(id, &params, None);
                if id == "timeline.key.move"
                    && let Some(keys) = moved.and_then(|r| serde_json::from_value::<Vec<KeyRef>>(r["keys"].clone()).ok())
                {
                    editor.keyframes_view.selected = keys;
                }
                if id == "timeline.close" && editor.session.timeline().is_none() {
                    editor.keyframes_view = KeyframesView::default();
                }
                if id == "timeline.key.remove" {
                    editor.keyframes_view.selected.clear();
                }
            }
            Ask::Merged(id, params, key) => {
                editor.dispatch(id, &params, Some(&key));
            }
            Ask::Select(keys) => editor.keyframes_view.selected = keys,
            Ask::Copy => copy_selected(editor),
            Ask::TogglePreview => {
                let view = &mut editor.keyframes_view;
                view.previewing = !view.previewing;
                view.show_pose = true;
            }
        }
    }
}
