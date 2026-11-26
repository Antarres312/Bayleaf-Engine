//! Keyframe animation commands (`timeline.*`): make, edit, save and preview `.btimeline` files.
//!
//! One timeline is open at a time. Its edits are undo steps in the same history as scene edits.
//! The playhead poses the prefab the timeline is made for without play mode: the prefab open on
//! its own (`prefab.open`), or else the first copy of it in the scene. The pose is shown, not
//! kept: it is never an undo step, never dirties the file, and the values it replaced come back
//! before any other command runs. While recording, edits of a posed part's components and script
//! properties (`scene.component.set`, `scene.script.setProperty`) become keys at the playhead.

use std::path::PathBuf;

use bayleaf_core::StableId;
use bayleaf_scene::components::{Prefab, Scripts};
use bayleaf_scene::timeline::{self, Binding, Ease, Key, Part, Target, Timeline, Track};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::confine::Access;
use crate::{CommandError, CommandSpec, Mutates, Prompt, Session, Snapshot, always, parse};

/// Why timeline edits are disabled with no timeline open.
pub(crate) const NO_TIMELINE: &str = "open or create a timeline first (timeline.open, timeline.new)";
/// `format` of copied keys.
pub const KEYS_FORMAT: &str = "bayleaf.timeline.keys";
/// Most keys one copy or paste may hold.
const MAX_COPIED: usize = 10_000;

/// The timeline open in the editor.
#[derive(Clone, Debug, PartialEq)]
pub struct TimelineDoc {
    pub timeline: Timeline,
    /// File it was opened from or last saved to.
    pub path: Option<PathBuf>,
    /// True when it has changes that are not saved.
    pub dirty: bool,
    /// The moment shown, in seconds.
    pub playhead: f32,
    /// Edits of posed parts become keys at the playhead.
    pub recording: bool,
    /// The values the pose on show replaced (object, component, value), put back when it ends;
    /// `None` while nothing is posed.
    rest: Option<Vec<(StableId, String, Value)>>,
}

impl TimelineDoc {
    fn new(timeline: Timeline, path: Option<PathBuf>) -> Self {
        Self { timeline, path, dirty: false, playhead: 0.0, recording: false, rest: None }
    }

    /// Whether the prefab shows the timeline's pose at the playhead.
    pub fn is_posed(&self) -> bool {
        self.rest.is_some()
    }
}

impl Snapshot {
    pub(crate) fn timeline(doc: &TimelineDoc) -> Self {
        Self::Timeline(serde_json::to_vec(&doc.timeline).unwrap_or_default())
    }

    pub(crate) fn to_timeline(&self) -> Result<Timeline, CommandError> {
        let Self::Timeline(bytes) = self else { return Err(CommandError::Failed("this undo step is not a timeline".to_owned())) };
        serde_json::from_slice(bytes).map_err(|e| CommandError::Failed(format!("the undo step cannot be read: {e}")))
    }
}

impl Session {
    /// The open timeline.
    pub fn timeline(&self) -> Option<&TimelineDoc> {
        self.timeline.as_ref()
    }

    /// Which objects stand for the open timeline's parts, when its prefab is open or placed.
    pub fn timeline_binding(&self) -> Option<Binding> {
        self.timeline.as_ref().and_then(|doc| binding(self, &doc.timeline).ok())
    }

    /// Every object of the open timeline's prefab in hierarchy order, animated or not: its part id,
    /// the scene object that stands for it (while the prefab is open or placed) and its name.
    pub fn timeline_prefab_objects(&self) -> Vec<(StableId, Option<StableId>, String)> {
        let Some(doc) = self.timeline.as_ref() else { return Vec::new() };
        let prefab = &doc.timeline.prefab;
        let Ok(ids) = prefab_ids(self, prefab) else { return Vec::new() };
        let file = if prefab_is_open(self, prefab) { None } else { crate::prefab_cmds::read_prefab(self, prefab).ok() };
        ids.into_iter()
            .map(|part| {
                let object = object_for(self, part);
                let name = object
                    .and_then(|o| self.scene.name(o))
                    .map(str::to_owned)
                    .or_else(|| file.as_ref()?.iter().find(|o| o.id == part)?.components.get("name")?.as_str().map(str::to_owned));
                (part, object, name.unwrap_or_else(|| "Object".to_owned()))
            })
            .collect()
    }
}

fn has_timeline(session: &Session) -> Result<(), String> {
    if session.timeline.is_some() { Ok(()) } else { Err(NO_TIMELINE.to_owned()) }
}

fn spec(id: &'static str, label: &'static str, params: &'static str, mutates: Mutates, run: crate::RunFn) -> CommandSpec {
    let enabled: crate::EnabledFn = if matches!(id, "timeline.new" | "timeline.open") { always } else { has_timeline };
    CommandSpec { id, label, menu: "", shortcut: "", params, prompt: Prompt::None, mutates, enabled, run }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        spec(
            "timeline.new",
            "New Keyframe Animation",
            "{prefab, length?, path?}: opens a new timeline for the .bprefab file (length in seconds, 2 if left out); with path it is written there at once",
            Mutates::No,
            timeline_new,
        ),
        spec("timeline.open", "Open Keyframe Animation", "{path} (.btimeline)", Mutates::No, timeline_open),
        spec("timeline.save", "Save Keyframe Animation", "{path?} (needed for a timeline that has no file yet)", Mutates::No, timeline_save),
        spec(
            "timeline.close",
            "Close Keyframe Animation",
            "{discard?}: closes the timeline with its undo steps; refuses unsaved changes unless discard",
            Mutates::No,
            timeline_close,
        ),
        spec(
            "timeline.get",
            "Show Keyframe Animation",
            "none; returns {path, dirty, playhead, recording, posed, timeline, parts: [{id, name, object}]}; object is the scene object that stands for the part, when the prefab is open or placed",
            Mutates::No,
            timeline_get,
        ),
        spec(
            "timeline.set",
            "Change Keyframe Animation",
            "{length?, looping?}: a shorter length drops the keys after it; returns {dropped}",
            Mutates::Timeline,
            timeline_set,
        ),
        spec("timeline.part.add", "Add Animated Part", "{part}: an object of the prefab, by its id in the prefab file", Mutates::Timeline, part_add),
        spec("timeline.part.remove", "Remove Animated Part", "{part}: with its tracks", Mutates::Timeline, part_remove),
        spec(
            "timeline.track.add",
            "Add Track",
            "{part, track}: track is {component, field} (transform position, rotation or scale; a field of another component, dotted inside an object field) or {script, property} (a property of the part's script with that module); the part is added too",
            Mutates::Timeline,
            track_add,
        ),
        spec("timeline.track.remove", "Remove Track", "{part, track}: with its keys", Mutates::Timeline, track_remove),
        spec(
            "timeline.key.set",
            "Set Key",
            "{part, track, time?, value?, ease?}: places a key (at the playhead when time is left out) or changes the one there; value left out keeps the key's or reads the part's value now; ease \"step\", \"linear\" or \"smooth\" (the default) says how it goes to the next key; the track and part are added too",
            Mutates::Timeline,
            key_set,
        ),
        spec(
            "timeline.key.move",
            "Move Keys",
            "{keys: [{part, track, time}], by}: moves the keys by seconds, inside the length; a key moved onto another's moment replaces it; returns {keys}, where they are now",
            Mutates::Timeline,
            key_move,
        ),
        spec("timeline.key.remove", "Delete Keys", "{keys: [{part, track, time}]}", Mutates::Timeline, key_remove),
        spec("timeline.key.copy", "Copy Keys", "{keys: [{part, track, time}]}: returns {text}, the keys as clipboard text", Mutates::No, key_copy),
        spec(
            "timeline.key.paste",
            "Paste Keys",
            "{text, time?, part?, track?}: adds copied keys, the earliest at time (else the playhead), on their own tracks, or all on part and track when given; returns {keys}",
            Mutates::Timeline,
            key_paste,
        ),
        spec(
            "timeline.key.record",
            "Record Key",
            "{id, component, value} or {id, index, name, value}: what scene.component.set and scene.script.setProperty take; keys the changed values of the posed part at the playhead (recording turns this on for those commands); returns {keys}",
            Mutates::Timeline,
            key_record,
        ),
        spec(
            "timeline.seek",
            "Move Playhead",
            "{time}: shows the prefab's pose at that moment, without play mode or an undo step; any other command puts the values back first; returns {time, problems}",
            Mutates::No,
            seek,
        ),
        spec("timeline.pose.end", "Show Without Animation", "none: puts back the values the pose replaced", Mutates::No, pose_end),
        spec(
            "timeline.record",
            "Record Keys",
            "{on}: while on, edits of a posed part (scene.component.set, scene.script.setProperty) become keys at the playhead",
            Mutates::No,
            record,
        ),
    ]
}

fn failed(message: impl Into<String>) -> CommandError {
    CommandError::Failed(message.into())
}

fn doc(session: &Session) -> Result<&TimelineDoc, CommandError> {
    session.timeline.as_ref().ok_or_else(|| failed(NO_TIMELINE))
}

fn doc_mut(session: &mut Session) -> Result<&mut TimelineDoc, CommandError> {
    session.timeline.as_mut().ok_or_else(|| failed(NO_TIMELINE))
}

// ---------------------------------------------------------------------------------------------
// The prefab a timeline is made for, and the objects that stand for its parts.

/// Whether the prefab open on its own is `prefab`.
fn prefab_is_open(session: &Session, prefab: &str) -> bool {
    session.prefab_edits.last().is_some_and(|e| e.source == prefab)
}

/// The ids of the prefab's objects in hierarchy order: the scene's own while the prefab is open,
/// else its file's.
fn prefab_ids(session: &Session, prefab: &str) -> Result<Vec<StableId>, CommandError> {
    if prefab_is_open(session, prefab) {
        let roots = session.scene.children(None).to_vec();
        return Ok(session.scene.export(&roots)?.into_iter().map(|o| o.id).collect());
    }
    Ok(crate::prefab_cmds::read_prefab(session, prefab)?.into_iter().map(|o| o.id).collect())
}

/// The objects that stand for `timeline`'s parts: the prefab's own while it is open, else the
/// first copy of it in the scene.
pub(crate) fn binding(session: &Session, timeline: &Timeline) -> Result<Binding, CommandError> {
    if prefab_is_open(session, &timeline.prefab) {
        return Ok(Binding::same_ids(&session.scene, timeline));
    }
    let copy = first_copy(session, &timeline.prefab)
        .ok_or_else(|| failed(format!("open {} (prefab.open) or place a copy of it in the scene to see the timeline's pose", timeline.prefab)))?;
    let file_ids = prefab_ids(session, &timeline.prefab)?;
    Ok(Binding::by_order(&session.scene, copy, &file_ids, timeline))
}

/// The component a track writes.
fn component_of(target: &Target) -> &str {
    match target {
        Target::Field { component, .. } => component,
        Target::Property { .. } => "scripts",
    }
}

// ---------------------------------------------------------------------------------------------
// The pose at the playhead.

/// Puts back the values the pose replaced.
pub(crate) fn end_pose(session: &mut Session) {
    let Some(rest) = session.timeline.as_mut().and_then(|d| d.rest.take()) else { return };
    for (object, component, value) in rest {
        if session.scene.contains(object) {
            let _ = session.scene.set_component(object, &component, value);
        }
    }
}

/// Shows the pose at the playhead, from the values the scene holds without it. Returns what could
/// not be shown.
fn pose(session: &mut Session) -> Result<Vec<String>, CommandError> {
    end_pose(session);
    let binding = binding(session, &doc(session)?.timeline)?;
    let Some(doc) = session.timeline.as_mut() else { return Err(failed(NO_TIMELINE)) };
    let samples = doc.timeline.sample(doc.playhead);
    let mut rest: Vec<(StableId, String, Value)> = Vec::new();
    for sample in &samples {
        let Some(object) = binding.object(sample.part) else { continue };
        let component = component_of(sample.target);
        if !rest.iter().any(|(o, c, _)| *o == object && c == component)
            && let Ok(value) = session.scene.component(object, component)
        {
            rest.push((object, component.to_owned(), value));
        }
    }
    let problems = timeline::apply(&mut session.scene, &binding, &samples);
    doc.rest = Some(rest);
    Ok(problems)
}

/// Before any command but the timeline's own (and undo or redo of a timeline step), the pose ends.
pub(crate) fn before_command(session: &mut Session, id: &str) {
    if id.starts_with("timeline.") {
        return;
    }
    let step = match id {
        "edit.undo" => session.undo.last(),
        "edit.redo" => session.redo.last(),
        _ => None,
    };
    if step.is_some_and(|s| matches!(s.before, Snapshot::Timeline(_))) {
        return;
    }
    end_pose(session);
}

/// After the timeline changed: the pose on show follows it.
pub(crate) fn after_change(session: &mut Session) {
    if session.timeline.as_ref().is_some_and(TimelineDoc::is_posed) {
        let _ = pose(session);
    }
}

/// While recording, the commands that change a posed part's values become `timeline.key.record`.
pub(crate) fn redirect(session: &Session, id: &str, params: &Value) -> bool {
    if !matches!(id, "scene.component.set" | "scene.script.setProperty") || session.is_playing() {
        return false;
    }
    let Some(doc) = session.timeline.as_ref().filter(|d| d.recording) else { return false };
    if id == "scene.component.set" && matches!(params.get("component").and_then(Value::as_str), Some("name" | "prefab")) {
        return false;
    }
    if id == "scene.script.setProperty" && params.get("value").is_none_or(Value::is_null) {
        return false;
    }
    let Some(object) = params.get("id").and_then(|v| serde_json::from_value::<StableId>(v.clone()).ok()) else { return false };
    let Ok(binding) = binding(session, &doc.timeline) else { return false };
    if binding.part_of(object).is_some() {
        return true;
    }
    // A part not animated yet is one of the prefab's objects.
    prefab_ids(session, &doc.timeline.prefab).is_ok_and(|ids| part_for(session, &binding, &ids, object).is_some())
}

/// The part `object` stands for, among the prefab's ids, whether or not it is animated yet.
fn part_for(session: &Session, binding: &Binding, prefab_ids: &[StableId], object: StableId) -> Option<StableId> {
    if let Some(part) = binding.part_of(object) {
        return Some(part);
    }
    let timeline = &session.timeline.as_ref()?.timeline;
    if prefab_is_open(session, &timeline.prefab) {
        return prefab_ids.contains(&object).then_some(object);
    }
    // A copy: the object's place in the copy's hierarchy order is the part's in the file.
    let root = first_copy(session, &timeline.prefab)?;
    let order: Vec<StableId> = session.scene.export(&[root]).ok()?.into_iter().map(|o| o.id).collect();
    let at = order.iter().position(|o| *o == object)?;
    prefab_ids.get(at).copied()
}

// ---------------------------------------------------------------------------------------------
// Files.

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewParams {
    prefab: PathBuf,
    #[serde(default)]
    length: Option<f32>,
    #[serde(default)]
    path: Option<PathBuf>,
}

fn refuse_unsaved(session: &Session) -> Result<(), CommandError> {
    match &session.timeline {
        Some(doc) if doc.dirty => Err(failed("the open timeline has unsaved changes: save it (timeline.save) or close it with discard (timeline.close)")),
        _ => Ok(()),
    }
}

fn check_extension(path: &std::path::Path) -> Result<(), CommandError> {
    if path.extension().is_some_and(|e| e == timeline::EXTENSION) {
        Ok(())
    } else {
        Err(CommandError::BadParams(format!("timeline files end in .{}", timeline::EXTENSION)))
    }
}

fn write(session: &Session, path: &std::path::Path, timeline: &Timeline) -> Result<PathBuf, CommandError> {
    check_extension(path)?;
    let path = session.allowed(path, Access::Write)?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| failed(format!("cannot make {}: {e}", parent.display())))?;
    }
    std::fs::write(&path, timeline.to_json()).map_err(|e| failed(format!("cannot write {}: {e}", path.display())))?;
    Ok(path)
}

/// Puts `doc` in place of the open timeline, whose undo steps go.
fn replace(session: &mut Session, doc: TimelineDoc) {
    end_pose(session);
    session.forget(|s| matches!(s, Snapshot::Timeline(_)));
    session.timeline = Some(doc);
}

fn timeline_new(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: NewParams = parse(params)?;
    refuse_unsaved(session)?;
    if p.prefab.extension().is_none_or(|e| e != crate::prefab_cmds::PREFAB_EXTENSION) {
        return Err(CommandError::BadParams(format!("a timeline is made for a .{} file", crate::prefab_cmds::PREFAB_EXTENSION)));
    }
    let length = p.length.unwrap_or(2.0);
    timeline::check_length(length).map_err(CommandError::BadParams)?;
    let source = crate::prefab_cmds::link_path(session, &session.script_dir().join(&p.prefab));
    // The prefab must be there to animate (and readable under automation).
    prefab_ids(session, &source)?;
    let timeline = Timeline::new(&source, length);
    let path = match &p.path {
        Some(path) => Some(write(session, path, &timeline)?),
        None => None,
    };
    replace(session, TimelineDoc::new(timeline, path));
    timeline_get(session, &Value::Null)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathParams {
    path: PathBuf,
}

fn timeline_open(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PathParams = parse(params)?;
    check_extension(&p.path)?;
    let path = session.allowed(&p.path, Access::Read)?;
    if session.timeline.as_ref().and_then(|d| d.path.as_deref()).is_some_and(|open| same_file(open, &path)) {
        return timeline_get(session, &Value::Null);
    }
    refuse_unsaved(session)?;
    let text = bayleaf_core::files::read_to_string(&path).map_err(|e| failed(format!("cannot read {}: {e}", path.display())))?;
    let timeline = Timeline::from_json(&text).map_err(|e| failed(format!("{}: {e}", path.display())))?;
    replace(session, TimelineDoc::new(timeline, Some(path)));
    timeline_get(session, &Value::Null)
}

fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    let canonical = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canonical(a) == canonical(b)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveParams {
    #[serde(default)]
    path: Option<PathBuf>,
}

fn timeline_save(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: SaveParams = parse(params)?;
    let doc = doc(session)?;
    let path = p.path.or_else(|| doc.path.clone()).ok_or_else(|| CommandError::BadParams("this timeline has no file yet; give a path".to_owned()))?;
    let written = write(session, &path, &doc.timeline)?;
    let doc = doc_mut(session)?;
    doc.path = Some(written.clone());
    doc.dirty = false;
    Ok(json!({ "path": written }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseParams {
    #[serde(default)]
    discard: bool,
}

fn timeline_close(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: CloseParams = parse(params)?;
    if doc(session)?.dirty && !p.discard {
        return Err(failed("the timeline has unsaved changes: save it (timeline.save), or close it with discard"));
    }
    end_pose(session);
    session.forget(|s| matches!(s, Snapshot::Timeline(_)));
    session.timeline = None;
    Ok(json!({}))
}

/// The prefab's objects' names by their file ids, from the scene where they stand for parts and
/// else from the file.
fn part_names(session: &Session, timeline: &Timeline, binding: Option<&Binding>) -> Vec<(StableId, String)> {
    let file = if prefab_is_open(session, &timeline.prefab) { None } else { crate::prefab_cmds::read_prefab(session, &timeline.prefab).ok() };
    timeline
        .parts
        .iter()
        .map(|part| {
            let from_scene = binding.and_then(|b| b.object(part.id)).and_then(|o| session.scene.name(o)).map(str::to_owned);
            let from_file = || {
                file.as_ref()
                    .and_then(|objects| objects.iter().find(|o| o.id == part.id))
                    .and_then(|o| o.components.get("name").and_then(Value::as_str).map(str::to_owned))
            };
            (part.id, from_scene.or_else(from_file).unwrap_or_else(|| "Missing part".to_owned()))
        })
        .collect()
}

fn timeline_get(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let _: crate::play::NoParams = parse(params)?;
    let doc = doc(session)?;
    let binding = binding(session, &doc.timeline).ok();
    let parts: Vec<Value> = part_names(session, &doc.timeline, binding.as_ref())
        .into_iter()
        .map(|(id, name)| json!({ "id": id, "name": name, "object": binding.as_ref().and_then(|b| b.object(id)) }))
        .collect();
    Ok(json!({
        "path": doc.path,
        "dirty": doc.dirty,
        "playhead": doc.playhead,
        "recording": doc.recording,
        "posed": doc.is_posed(),
        "timeline": doc.timeline,
        "parts": parts,
    }))
}

// ---------------------------------------------------------------------------------------------
// Edits.

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetParams {
    #[serde(default)]
    length: Option<f32>,
    #[serde(default)]
    looping: Option<bool>,
}

fn timeline_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: SetParams = parse(params)?;
    if let Some(length) = p.length {
        timeline::check_length(length).map_err(CommandError::BadParams)?;
    }
    let doc = doc_mut(session)?;
    let mut dropped = 0;
    if let Some(length) = p.length {
        doc.timeline.length = length;
        for track in doc.timeline.parts.iter_mut().flat_map(|p| p.tracks.iter_mut()) {
            let before = track.keys.len();
            track.keys.retain(|k| k.time <= length + timeline::SAME_TIME);
            dropped += before - track.keys.len();
        }
        doc.playhead = doc.playhead.min(length);
    }
    if let Some(looping) = p.looping {
        doc.timeline.looping = looping;
    }
    Ok(json!({ "dropped": dropped }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PartParams {
    part: StableId,
}

/// The part `id` of the open timeline, added when the prefab has that object.
fn ensure_part(session: &mut Session, id: StableId) -> Result<&mut Part, CommandError> {
    let prefab = doc(session)?.timeline.prefab.clone();
    if doc(session)?.timeline.part(id).is_none() {
        if !prefab_ids(session, &prefab)?.contains(&id) {
            return Err(CommandError::BadParams(format!("{prefab} has no object {id}")));
        }
        let parts = &mut doc_mut(session)?.timeline.parts;
        if parts.len() >= timeline::MAX_PARTS {
            return Err(failed(format!("a timeline animates at most {} parts", timeline::MAX_PARTS)));
        }
        parts.push(Part { id, tracks: Vec::new() });
    }
    doc_mut(session)?.timeline.part_mut(id).ok_or_else(|| failed("the part went missing"))
}

fn part_add(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PartParams = parse(params)?;
    if doc(session)?.timeline.part(p.part).is_some() {
        return Err(failed(format!("{} already takes part", p.part)));
    }
    ensure_part(session, p.part)?;
    Ok(json!({}))
}

fn part_remove(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PartParams = parse(params)?;
    let parts = &mut doc_mut(session)?.timeline.parts;
    let before = parts.len();
    parts.retain(|part| part.id != p.part);
    if parts.len() == before {
        return Err(failed(format!("{} does not take part", p.part)));
    }
    Ok(json!({}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackParams {
    part: StableId,
    track: Target,
}

/// Checks a track's target the way a file is checked.
fn check_target(target: &Target) -> Result<(), CommandError> {
    let mut probe = Timeline::new("probe.bprefab", 1.0);
    probe.parts.push(Part { id: StableId::from_u128(0), tracks: vec![Track { target: target.clone(), keys: Vec::new() }] });
    probe.validate().map_err(CommandError::BadParams)
}

fn ensure_track<'a>(session: &'a mut Session, part: StableId, target: &Target) -> Result<&'a mut Track, CommandError> {
    check_target(target)?;
    let part = ensure_part(session, part)?;
    if part.track(target).is_none() {
        if part.tracks.len() >= timeline::MAX_TRACKS {
            return Err(failed(format!("a part has at most {} tracks", timeline::MAX_TRACKS)));
        }
        part.tracks.push(Track { target: target.clone(), keys: Vec::new() });
    }
    part.track_mut(target).ok_or_else(|| failed("the track went missing"))
}

fn track_add(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: TrackParams = parse(params)?;
    if doc(session)?.timeline.part(p.part).and_then(|part| part.track(&p.track)).is_some() {
        return Err(failed(format!("the part already has the track {}", p.track.describe())));
    }
    ensure_track(session, p.part, &p.track)?;
    Ok(json!({}))
}

fn track_remove(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: TrackParams = parse(params)?;
    let part = doc_mut(session)?.timeline.part_mut(p.part).ok_or_else(|| failed(format!("{} does not take part", p.part)))?;
    let before = part.tracks.len();
    part.tracks.retain(|t| t.target != p.track);
    if part.tracks.len() == before {
        return Err(failed(format!("the part has no track {}", p.track.describe())));
    }
    Ok(json!({}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeySetParams {
    part: StableId,
    track: Target,
    #[serde(default)]
    time: Option<f32>,
    #[serde(default)]
    value: Option<Value>,
    #[serde(default)]
    ease: Option<Ease>,
}

/// The object that stands for `part` of the open timeline, whether or not it is animated yet.
fn object_for(session: &Session, part: StableId) -> Option<StableId> {
    let timeline = &session.timeline.as_ref()?.timeline;
    if prefab_is_open(session, &timeline.prefab) {
        return session.scene.contains(part).then_some(part);
    }
    // A copy: the part's place in the file is the object's in the copy.
    let ids = prefab_ids(session, &timeline.prefab).ok()?;
    let at = ids.iter().position(|id| *id == part)?;
    let root = first_copy(session, &timeline.prefab)?;
    session.scene.export(&[root]).ok()?.get(at).map(|o| o.id)
}

/// The first copy of `prefab` in the scene.
fn first_copy(session: &Session, prefab: &str) -> Option<StableId> {
    session.scene.world_matrices().into_iter().map(|(id, _)| id).find(|id| session.scene.get::<Prefab>(*id).is_some_and(|p| p.source == prefab))
}

/// The value of `target` on the object standing for `part` now (posed, if the pose is on show).
fn current_value(session: &Session, part: StableId, target: &Target) -> Option<Value> {
    timeline::read(&session.scene, object_for(session, part)?, target)
}

fn key_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: KeySetParams = parse(params)?;
    let (length, playhead) = doc(session).map(|d| (d.timeline.length, d.playhead))?;
    let time = p.time.unwrap_or(playhead);
    timeline::check_time(time, length).map_err(CommandError::BadParams)?;
    let time = time.min(length);
    let existing = doc(session)?.timeline.part(p.part).and_then(|part| part.track(&p.track)).and_then(|t| t.key_at(time).map(|i| t.keys[i].clone()));
    let value = match (p.value, &existing) {
        (Some(value), _) => value,
        (None, Some(key)) => key.value.clone(),
        (None, None) => current_value(session, p.part, &p.track)
            .or_else(|| doc(session).ok()?.timeline.part(p.part)?.track(&p.track)?.value_at(time))
            .ok_or_else(|| failed(format!("give a value: the part is not in the scene to read its {} from", p.track.describe())))?,
    };
    timeline::check_value(&value).map_err(CommandError::BadParams)?;
    let ease = p.ease.or(existing.map(|k| k.ease)).unwrap_or_default();
    let track = ensure_track(session, p.part, &p.track)?;
    if track.key_at(time).is_none() && track.keys.len() >= timeline::MAX_KEYS {
        return Err(failed(format!("a track has at most {} keys", timeline::MAX_KEYS)));
    }
    track.place(Key { time, value, ease });
    Ok(json!({ "time": time }))
}

/// One key, by its part, track and moment.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyRef {
    pub part: StableId,
    pub track: Target,
    pub time: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeysParams {
    keys: Vec<KeyRef>,
}

/// The keys `refs` name, taken off their tracks, with where they were. Refuses a key that is not there.
fn take_keys(timeline: &mut Timeline, refs: &[KeyRef]) -> Result<Vec<(KeyRef, Key)>, CommandError> {
    if refs.len() > MAX_COPIED {
        return Err(CommandError::BadParams(format!("at most {MAX_COPIED} keys at once")));
    }
    let mut taken = Vec::with_capacity(refs.len());
    for r in refs {
        let missing = || failed(format!("there is no key of {} at {} seconds", r.track.describe(), r.time));
        let track = timeline.part_mut(r.part).and_then(|p| p.track_mut(&r.track)).ok_or_else(missing)?;
        let at = track.key_at(r.time).ok_or_else(missing)?;
        taken.push((r.clone(), track.keys.remove(at)));
    }
    Ok(taken)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveParams {
    keys: Vec<KeyRef>,
    by: f32,
}

fn key_move(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: MoveParams = parse(params)?;
    if !p.by.is_finite() {
        return Err(CommandError::BadParams("by is a finite number of seconds".to_owned()));
    }
    let timeline = &mut doc_mut(session)?.timeline;
    let length = timeline.length;
    let taken = take_keys(timeline, &p.keys)?;
    let mut moved = Vec::with_capacity(taken.len());
    for (r, mut key) in taken {
        key.time = (key.time + p.by).clamp(0.0, length);
        let time = key.time;
        if let Some(track) = timeline.part_mut(r.part).and_then(|part| part.track_mut(&r.track)) {
            track.place(key);
        }
        moved.push(KeyRef { time, ..r });
    }
    Ok(json!({ "keys": moved }))
}

fn key_remove(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: KeysParams = parse(params)?;
    let taken = take_keys(&mut doc_mut(session)?.timeline, &p.keys)?;
    Ok(json!({ "removed": taken.len() }))
}

/// Copied keys as clipboard text.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Copied {
    format: String,
    keys: Vec<CopiedKey>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CopiedKey {
    part: StableId,
    track: Target,
    #[serde(flatten)]
    key: Key,
}

fn key_copy(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: KeysParams = parse(params)?;
    let mut timeline = doc(session)?.timeline.clone();
    let keys = take_keys(&mut timeline, &p.keys)?.into_iter().map(|(r, key)| CopiedKey { part: r.part, track: r.track, key }).collect();
    let text = serde_json::to_string(&Copied { format: KEYS_FORMAT.to_owned(), keys }).map_err(|e| failed(e.to_string()))?;
    Ok(json!({ "text": text }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasteParams {
    text: String,
    #[serde(default)]
    time: Option<f32>,
    #[serde(default)]
    part: Option<StableId>,
    #[serde(default)]
    track: Option<Target>,
}

fn key_paste(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PasteParams = parse(params)?;
    if p.part.is_some() != p.track.is_some() {
        return Err(CommandError::BadParams("give part and track together, or neither".to_owned()));
    }
    let copied: Copied = serde_json::from_str(&p.text).map_err(|e| CommandError::BadParams(format!("not copied keys: {e}")))?;
    if copied.format != KEYS_FORMAT || copied.keys.len() > MAX_COPIED {
        return Err(CommandError::BadParams("not copied keys".to_owned()));
    }
    let (length, playhead) = doc(session).map(|d| (d.timeline.length, d.playhead))?;
    let at = p.time.unwrap_or(playhead);
    timeline::check_time(at, length).map_err(CommandError::BadParams)?;
    let earliest = copied.keys.iter().map(|k| k.key.time).fold(f32::INFINITY, f32::min);
    let mut pasted = Vec::with_capacity(copied.keys.len());
    for copy in copied.keys {
        let (part, target) = match (&p.part, &p.track) {
            (Some(part), Some(track)) => (*part, track.clone()),
            _ => (copy.part, copy.track),
        };
        let mut key = copy.key;
        timeline::check_value(&key.value).map_err(CommandError::BadParams)?;
        key.time = (key.time - earliest + at).clamp(0.0, length);
        let time = key.time;
        let track = ensure_track(session, part, &target)?;
        if track.key_at(time).is_none() && track.keys.len() >= timeline::MAX_KEYS {
            return Err(failed(format!("a track has at most {} keys", timeline::MAX_KEYS)));
        }
        track.place(key);
        pasted.push(KeyRef { part, track: target, time });
    }
    Ok(json!({ "keys": pasted }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordParams {
    id: StableId,
    #[serde(default)]
    component: Option<String>,
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    name: Option<String>,
    value: Value,
}

/// The leaves of `new` that differ from `old`, by dotted path.
fn changed_fields(old: &Value, new: &Value, path: &str, out: &mut Vec<(String, Value)>) {
    match (old, new) {
        (Value::Object(old), Value::Object(new)) => {
            for (name, value) in new {
                let inner = if path.is_empty() { name.clone() } else { format!("{path}.{name}") };
                match old.get(name) {
                    Some(was) => changed_fields(was, value, &inner, out),
                    None => out.push((inner, value.clone())),
                }
            }
        }
        _ if !same_value(old, new) => out.push((path.to_owned(), new.clone())),
        _ => {}
    }
}

/// Equal values, with numbers compared by value (`1` and `1.0` are the same).
fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(xs), Value::Array(ys)) => xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| same_value(x, y)),
        _ => a == b,
    }
}

fn key_record(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: RecordParams = parse(params)?;
    let doc = doc(session)?;
    let binding = binding(session, &doc.timeline)?;
    let ids = prefab_ids(session, &doc.timeline.prefab)?;
    let part = part_for(session, &binding, &ids, p.id).ok_or_else(|| failed("this object is not a part of the timeline's prefab"))?;
    // The changed values, keyed at the playhead.
    let changes: Vec<(Target, Value)> = match (p.component, p.index, p.name) {
        (Some(component), None, None) => {
            if matches!(component.as_str(), "name" | "prefab") {
                return Err(CommandError::BadParams("an object's name and prefab link are not animated".to_owned()));
            }
            let now = session.scene.component(p.id, &component)?;
            if !now.is_object() || !p.value.is_object() {
                return Err(failed(format!("the {component} component's fields cannot be keyed one by one")));
            }
            let mut fields = Vec::new();
            changed_fields(&now, &p.value, "", &mut fields);
            fields.into_iter().map(|(field, value)| (Target::Field { component: component.clone(), field }, value)).collect()
        }
        (None, Some(index), Some(name)) => {
            let scripts = session.scene.get::<Scripts>(p.id).ok_or_else(|| failed("the object has no scripts"))?;
            let script = scripts.0.get(index).ok_or_else(|| CommandError::BadParams(format!("the object has no script {index}")))?;
            vec![(Target::Property { script: script.module.clone(), property: name }, p.value)]
        }
        _ => return Err(CommandError::BadParams("give {id, component, value} or {id, index, name, value}".to_owned())),
    };
    for (target, value) in &changes {
        check_target(target)?;
        timeline::check_value(value).map_err(|e| CommandError::BadParams(format!("{}: {e}", target.describe())))?;
    }
    let time = doc_mut(session)?.playhead;
    let mut keys = Vec::with_capacity(changes.len());
    for (target, value) in changes {
        let track = ensure_track(session, part, &target)?;
        let ease = track.key_at(time).map(|i| track.keys[i].ease).unwrap_or_default();
        if track.key_at(time).is_none() && track.keys.len() >= timeline::MAX_KEYS {
            return Err(failed(format!("a track has at most {} keys", timeline::MAX_KEYS)));
        }
        track.place(Key { time, value, ease });
        keys.push(KeyRef { part, track: target, time });
    }
    // The recorded values show at once.
    if let Some(doc) = session.timeline.as_mut()
        && doc.rest.is_none()
    {
        doc.rest = Some(Vec::new());
    }
    Ok(json!({ "keys": keys }))
}

// ---------------------------------------------------------------------------------------------
// The playhead and recording.

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SeekParams {
    time: f32,
}

fn seek(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: SeekParams = parse(params)?;
    let doc = doc_mut(session)?;
    timeline::check_time(p.time, doc.timeline.length).map_err(CommandError::BadParams)?;
    doc.playhead = p.time.min(doc.timeline.length);
    let time = doc.playhead;
    let problems = pose(session)?;
    Ok(json!({ "time": time, "problems": problems }))
}

fn pose_end(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let _: crate::play::NoParams = parse(params)?;
    end_pose(session);
    Ok(json!({}))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordOnParams {
    on: bool,
}

fn record(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: RecordOnParams = parse(params)?;
    if p.on {
        if session.is_playing() {
            return Err(failed("stop play mode first: play mode does not record keys"));
        }
        // Recording shows the pose it keys.
        pose(session)?;
    }
    doc_mut(session)?.recording = p.on;
    Ok(json!({ "recording": p.on }))
}

/// Restores a timeline undo step and returns the state it replaced.
pub(crate) fn restore(session: &mut Session, snapshot: &Snapshot) -> Result<Snapshot, CommandError> {
    let timeline = snapshot.to_timeline()?;
    let doc = session.timeline.as_mut().ok_or_else(|| failed("the timeline of this step is closed"))?;
    let current = Snapshot::timeline(doc);
    doc.timeline = timeline;
    doc.playhead = doc.playhead.min(doc.timeline.length);
    doc.dirty = true;
    after_change(session);
    Ok(current)
}

#[cfg(test)]
mod tests;
