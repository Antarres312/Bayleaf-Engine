//! Prefabs: an object and its children saved to a `.bprefab` file, to place again and again.
//!
//! A placed copy keeps a `prefab` link to its file. Apply writes a copy's objects to the file and
//! updates every other copy in the scene; opening a scene updates every copy from its file; Revert
//! reads the file again and drops the copy's own changes. Each copy keeps its own name and
//! transform, and the component changes made to it (its overrides) survive updates. A prefab may
//! hold copies of other prefabs: they update from their own files, inside the copy.

use std::path::{Path, PathBuf};

use bayleaf_core::StableId;
use bayleaf_scene::Scene;
use bayleaf_scene::components::{Prefab, Transform};
use bayleaf_scene::doc::ObjectDoc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::confine::Access;
use crate::{CommandError, CommandSpec, Mutates, Prompt, Session, always, parse};

/// `format` of prefab files.
pub const PREFAB_FORMAT: &str = "bayleaf.prefab";
/// File extension of prefab files.
pub const PREFAB_EXTENSION: &str = "bprefab";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrefabFile {
    format: String,
    /// The scene format version the components are written in.
    version: u32,
    /// The prefab's root first, then its children, parents before their children.
    objects: Vec<ObjectDoc>,
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    let spec =
        |id, label, params, run| CommandSpec { id, label, menu: "", shortcut: "", params, prompt: Prompt::None, mutates: Mutates::Scene, enabled: always, run };
    vec![
        spec(
            "scene.prefab.create",
            "Save as Prefab",
            "{id, path}: saves the object and its children to a .bprefab file and links the object to it",
            prefab_create,
        ),
        spec("scene.prefab.place", "Place Prefab", "{path, parent?, index?}: adds a linked copy of a .bprefab file; returns {id}", prefab_place),
        spec(
            "scene.prefab.apply",
            "Apply to Prefab",
            "{id}: writes this copy to its prefab file and updates the other copies in the scene; returns {updated}",
            prefab_apply,
        ),
        spec(
            "scene.prefab.revert",
            "Revert to Prefab",
            "{id}: replaces this copy's objects with the prefab file's, keeping its name and transform",
            prefab_revert,
        ),
        CommandSpec {
            id: "prefab.open",
            label: "Open Prefab",
            menu: "",
            shortcut: "",
            params: "{path}: edits the prefab on its own: the scene and its undo history wait underneath, and prefab.close goes back; returns {source, depth}",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: |s| if s.is_playing() { Err("stop play mode first".to_owned()) } else { Ok(()) },
            run: prefab_open,
        },
        CommandSpec {
            id: "prefab.save",
            label: "Save Prefab",
            menu: "",
            shortcut: "",
            params: "none: writes the open prefab to its file; returns {path}",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: |s| if s.prefab_edits.is_empty() { Err("no prefab is open".to_owned()) } else { Ok(()) },
            run: prefab_save,
        },
        CommandSpec {
            id: "prefab.close",
            label: "Back from Prefab",
            menu: "",
            shortcut: "",
            params: "{discard?}: goes back to the scene (or the prefab) underneath, whose copies update from the file; refuses unsaved changes unless discard",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: |s| if s.prefab_edits.is_empty() { Err("no prefab is open".to_owned()) } else { Ok(()) },
            run: prefab_close,
        },
    ]
}

/// A prefab open on its own (`prefab.open`), and the scene it stands in front of.
pub(crate) struct PrefabEdit {
    /// The prefab's path as links name it.
    pub(crate) source: String,
    scene: Scene,
    dirty: bool,
    undo: Vec<crate::HistoryStep>,
    redo: Vec<crate::HistoryStep>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenParams {
    path: PathBuf,
}

fn prefab_open(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: OpenParams = parse(params)?;
    let source = link_path(session, &session.script_dir().join(&p.path));
    if session.prefab_edits.iter().any(|e| e.source == source) {
        return Err(CommandError::Failed(format!("{source} is already open")));
    }
    if session.prefab_edits.len() >= MAX_NESTING {
        return Err(CommandError::Failed(format!("prefabs open inside one another at most {MAX_NESTING} deep")));
    }
    let objects = read_prefab(session, &source)?;
    let mut scene = Scene::new();
    let root = *scene.import(&objects, None, None)?.first().ok_or_else(|| CommandError::Failed("the prefab is empty".to_owned()))?;
    // Around the origin: copies keep their own transform anyway.
    if let Some(mut transform) = scene.get::<Transform>(root).cloned() {
        transform.position = [0.0; 3];
        scene.set(root, transform)?;
    }
    let edit = PrefabEdit {
        source: source.clone(),
        scene: std::mem::replace(&mut session.scene, scene),
        dirty: std::mem::take(&mut session.dirty),
        undo: std::mem::take(&mut session.undo),
        redo: std::mem::take(&mut session.redo),
    };
    session.prefab_edits.push(edit);
    // Copies of other prefabs inside it come from their own files.
    let mut problems = Vec::new();
    for nested in nested_copies(&session.scene, root) {
        if let Err(e) = refresh(session, nested, true, &mut vec![source.clone()]) {
            problems.push(e.to_string());
        }
    }
    Ok(json!({ "source": source, "depth": session.prefab_edits.len(), "problems": problems }))
}

fn prefab_save(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let _: serde_json::Map<String, Value> = parse(params)?;
    let source = session.prefab_edits.last().map(|e| e.source.clone()).ok_or_else(|| CommandError::Failed("no prefab is open".to_owned()))?;
    let roots = session.scene.children(None).to_vec();
    let [root] = roots.as_slice() else {
        return Err(CommandError::Failed(format!("a prefab has one root object, and this one has {}: put the others under one", roots.len())));
    };
    let path = write_prefab(session, &session.script_dir().join(&source), &session.scene, *root)?;
    session.dirty = false;
    Ok(json!({ "path": path }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseParams {
    #[serde(default)]
    discard: bool,
}

fn prefab_close(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: CloseParams = parse(params)?;
    if session.dirty && !p.discard {
        return Err(CommandError::Failed("the prefab has unsaved changes: save it (prefab.save), or go back with discard".to_owned()));
    }
    let edit = session.prefab_edits.pop().ok_or_else(|| CommandError::Failed("no prefab is open".to_owned()))?;
    session.scene = edit.scene;
    session.dirty = edit.dirty;
    session.undo = edit.undo;
    session.redo = edit.redo;
    // The copies underneath follow the file, keeping their own changes.
    let problems = refresh_all(session);
    for problem in &problems {
        session.log(crate::Level::Error, problem.clone());
    }
    Ok(json!({ "depth": session.prefab_edits.len(), "problems": problems }))
}

/// `path` as stored in a link: relative to the project folder when inside it, with `/`.
fn link_path(session: &Session, path: &Path) -> String {
    let base = session.script_dir();
    let base = std::fs::canonicalize(&base).unwrap_or(base);
    let full = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    match full.strip_prefix(&base) {
        Ok(relative) => relative.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"),
        Err(_) => path.to_string_lossy().replace('\\', "/"),
    }
}

pub(crate) fn read_prefab(session: &Session, source: &str) -> Result<Vec<ObjectDoc>, CommandError> {
    let path = session.allowed(&session.script_dir().join(source), Access::Read)?;
    let text = bayleaf_core::files::read_to_string(&path).map_err(|e| CommandError::Failed(format!("cannot read {}: {e}", path.display())))?;
    let file: PrefabFile = serde_json::from_str(&text).map_err(|e| CommandError::Failed(format!("{} is not a prefab: {e}", path.display())))?;
    if file.format != PREFAB_FORMAT || !(1..=bayleaf_scene::doc::VERSION).contains(&file.version) {
        return Err(CommandError::Failed(format!("{} is not a prefab this engine can read", path.display())));
    }
    match file.objects.first() {
        Some(root) if root.parent.is_none() => Ok(file.objects),
        _ => Err(CommandError::Failed(format!("{} has no root object", path.display()))),
    }
}

fn write_prefab(session: &Session, path: &Path, scene: &Scene, id: StableId) -> Result<PathBuf, CommandError> {
    let path = session.allowed(path, Access::Write)?;
    let mut objects = scene.export(&[id])?;
    // The file holds the object itself, not its link to the file.
    if let Some(root) = objects.first_mut() {
        root.components.remove("prefab");
    }
    let file = PrefabFile { format: PREFAB_FORMAT.to_owned(), version: bayleaf_scene::doc::VERSION, objects };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CommandError::Failed(format!("cannot make {}: {e}", parent.display())))?;
    }
    let text = serde_json::to_string_pretty(&file).map_err(|e| CommandError::Failed(e.to_string()))?;
    std::fs::write(&path, text).map_err(|e| CommandError::Failed(format!("cannot write {}: {e}", path.display())))?;
    Ok(path)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateParams {
    id: StableId,
    path: PathBuf,
}

fn prefab_create(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: CreateParams = parse(params)?;
    if p.path.extension().is_none_or(|e| e != PREFAB_EXTENSION) {
        return Err(CommandError::BadParams(format!("prefab files end in .{PREFAB_EXTENSION}")));
    }
    let path = session.script_dir().join(&p.path);
    let source = link_path(session, &path);
    refuse_self_copy(session, p.id, &source)?;
    let written = write_prefab(session, &path, &session.scene, p.id)?;
    let source = link_path(session, &written);
    session.scene.set(p.id, Prefab::new(source.clone()))?;
    Ok(json!({ "path": written, "source": source }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaceParams {
    path: PathBuf,
    #[serde(default)]
    parent: Option<StableId>,
    #[serde(default)]
    index: Option<usize>,
}

fn prefab_place(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PlaceParams = parse(params)?;
    let full = session.script_dir().join(&p.path);
    let source = link_path(session, &full);
    let objects = read_prefab(session, &source)?;
    let id = *session.scene.import(&objects[..], p.parent, p.index)?.first().ok_or_else(|| CommandError::Failed("the prefab is empty".to_owned()))?;
    session.scene.set(id, Prefab::new(source.clone()))?;
    // Copies of other prefabs inside it come from their own files.
    for nested in nested_copies(&session.scene, id) {
        refresh(session, nested, true, &mut vec![source.clone()])?;
    }
    Ok(json!({ "id": id }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdParams {
    id: StableId,
}

fn source_of(session: &Session, id: StableId) -> Result<String, CommandError> {
    session.scene.get::<Prefab>(id).map(|p| p.source.clone()).ok_or_else(|| CommandError::Failed("this object is not a placed prefab".to_owned()))
}

fn prefab_apply(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: IdParams = parse(params)?;
    let source = source_of(session, p.id)?;
    refuse_self_copy(session, p.id, &source)?;
    let path = session.script_dir().join(&source);
    write_prefab(session, &path, &session.scene, p.id)?;
    // This copy now is the prefab: it has no changes of its own.
    session.scene.set(p.id, Prefab::new(source.clone()))?;
    let others: Vec<StableId> = session
        .scene
        .world_matrices()
        .into_iter()
        .map(|(id, _)| id)
        .filter(|id| *id != p.id && session.scene.get::<Prefab>(*id).is_some_and(|l| l.source == source))
        .collect();
    for other in &others {
        // An earlier update may have rebuilt the copy this one sat in.
        if session.scene.contains(*other) {
            refresh(session, *other, true, &mut Vec::new())?;
        }
    }
    Ok(json!({ "updated": others.len() }))
}

fn prefab_revert(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: IdParams = parse(params)?;
    let source = source_of(session, p.id)?;
    session.scene.set(p.id, Prefab::new(source))?;
    refresh(session, p.id, false, &mut Vec::new())?;
    Ok(json!({}))
}

/// A prefab may not hold a copy of itself.
fn refuse_self_copy(session: &Session, id: StableId, source: &str) -> Result<(), CommandError> {
    if order(&session.scene, id).iter().skip(1).any(|o| session.scene.get::<Prefab>(*o).is_some_and(|l| l.source == source)) {
        return Err(CommandError::Failed(format!("{source} would hold a copy of itself")));
    }
    Ok(())
}

/// Components a copy's own root keeps from the scene, not the file.
const KEPT: [&str; 3] = ["name", "transform", "prefab"];

/// `root` and everything below it, in hierarchy order.
fn order(scene: &Scene, root: StableId) -> Vec<StableId> {
    scene.export(&[root]).map(|objects| objects.into_iter().map(|o| o.id).collect()).unwrap_or_default()
}

/// The nearest copy at or above `id`, and `id`'s index among that copy's objects.
fn copy_of(scene: &Scene, id: StableId) -> Option<(StableId, usize)> {
    let mut at = Some(id);
    while let Some(object) = at {
        if scene.get::<Prefab>(object).is_some() {
            return order(scene, object).iter().position(|o| *o == id).map(|index| (object, index));
        }
        at = scene.parent(object);
    }
    None
}

/// Copies inside `root` that are not inside another copy inside `root`.
fn nested_copies(scene: &Scene, root: StableId) -> Vec<StableId> {
    order(scene, root)
        .into_iter()
        .skip(1)
        .filter(|id| scene.get::<Prefab>(*id).is_some() && copy_of(scene, scene.parent(*id).unwrap_or(root)).is_some_and(|(c, _)| c == root))
        .collect()
}

/// Records that component `component` of object `id` was changed in the scene, so the copy it
/// belongs to keeps the change when it is updated from its file. A copy's own name and transform
/// are always its own; inside another copy they are that copy's change.
pub(crate) fn note_override(scene: &mut Scene, id: StableId, component: &str) {
    if let Some((root, index)) = override_owner(scene, id, component) {
        add_override(scene, root, index, component);
    }
}

/// The copy that keeps component `component` of `id` as its own change, and `id`'s index among
/// its objects: the nearest copy, or for a copy's own name and transform the copy around it.
fn override_owner(scene: &Scene, id: StableId, component: &str) -> Option<(StableId, usize)> {
    if component == "prefab" {
        return None;
    }
    let (root, index) = copy_of(scene, id)?;
    if index == 0 && KEPT.contains(&component) {
        let (outer, _) = copy_of(scene, scene.parent(root)?)?;
        let at = order(scene, outer).iter().position(|o| *o == root)?;
        return Some((outer, at));
    }
    Some((root, index))
}

/// What the prefab file gives component `component` of `id`: the file and the value, or `None`
/// when `id` is not part of a copy, the component is the copy's own (its name and transform), or
/// the file has no such component there (or cannot be read).
fn prefab_value(session: &Session, id: StableId, component: &str) -> Option<(String, Value)> {
    let (owner, index) = override_owner(&session.scene, id, component)?;
    let source = session.scene.get::<Prefab>(owner)?.source.clone();
    let objects = read_prefab(session, &source).ok()?;
    let value = objects.get(index)?.components.get(component)?.clone();
    Some((source, value))
}

/// Where Reset takes component `component` of `id` from: the prefab file it names, or `None` for
/// the component's defaults. With `script`, one script of the object's list: the prefab's values
/// only when the file has the same script at that place.
pub fn reset_source(session: &Session, id: StableId, component: &str, script: Option<usize>) -> Option<String> {
    reset_values(session, id, component, script).map(|(source, _)| source)
}

/// The prefab's value for a Reset (see [`reset_source`]): the whole component, or with `script`
/// that script's properties as a `Scripts` value holding the file's script.
pub(crate) fn reset_values(session: &Session, id: StableId, component: &str, script: Option<usize>) -> Option<(String, Value)> {
    let (source, value) = prefab_value(session, id, component)?;
    let Some(index) = script else { return Some((source, value)) };
    let file: bayleaf_scene::components::Scripts = serde_json::from_value(value).ok()?;
    let current = session.scene.get::<bayleaf_scene::components::Scripts>(id)?.0.get(index)?;
    let theirs = file.0.get(index).filter(|s| s.module == current.module)?;
    Some((source, serde_json::to_value(theirs).ok()?))
}

/// Drops the override of component `component` of `id` once it matches the prefab file again,
/// so changes to the file reach it.
pub(crate) fn forget_override(session: &mut Session, id: StableId, component: &str) {
    let Some((root, index)) = override_owner(&session.scene, id, component) else { return };
    let theirs = prefab_value(session, id, component).and_then(|(_, value)| stored(component, value));
    let same = theirs.is_some() && session.scene.component(id, component).ok() == theirs;
    let Some(mut link) = session.scene.get::<Prefab>(root).cloned() else { return };
    let key = format!("{index}/{component}");
    if same && link.overrides.contains(&key) {
        link.overrides.retain(|k| *k != key);
        let _ = session.scene.set(root, link);
    }
}

/// `value` of component `component` as the scene stores it, so fields a file leaves out compare
/// as their defaults.
fn stored(component: &str, value: Value) -> Option<Value> {
    let mut probe = Scene::new();
    let object = probe.create_object(None, "probe").ok()?;
    if probe.component(object, component).is_err() {
        probe.add_component(object, component).ok()?;
    }
    probe.set_component(object, component, value).ok()?;
    probe.component(object, component).ok()
}
fn add_override(scene: &mut Scene, root: StableId, index: usize, component: &str) {
    let Some(mut link) = scene.get::<Prefab>(root).cloned() else { return };
    let key = format!("{index}/{component}");
    if !link.overrides.contains(&key) && link.overrides.len() < bayleaf_scene::components::MAX_PREFAB_OVERRIDES {
        link.overrides.push(key);
        let _ = scene.set(root, link);
    }
}

/// One copy's overrides with their values (`None`: the component was removed).
struct Kept {
    /// The copy's index inside the copy being updated (0 for that copy itself), and its file.
    at: usize,
    source: String,
    values: Vec<(usize, String, Option<Value>)>,
}

/// Updates copy `root` from its file. With `keep`, the overrides of it and of the copies inside it
/// are put back afterwards; the copies inside it update from their own files first. `visited`
/// holds the files being updated around it, so a prefab that holds itself is refused.
fn refresh(session: &mut Session, root: StableId, keep: bool, visited: &mut Vec<String>) -> Result<(), CommandError> {
    let source = source_of(session, root)?;
    if visited.contains(&source) || visited.len() > MAX_NESTING {
        return Err(CommandError::Failed(format!("{source} holds a copy of itself, or prefabs nest more than {MAX_NESTING} deep")));
    }
    let objects = read_prefab(session, &source)?;
    let kept: Vec<Kept> = if keep { capture(&session.scene, root) } else { Vec::new() };
    replace_contents(&mut session.scene, root, &objects)?;
    visited.push(source);
    for nested in nested_copies(&session.scene, root) {
        refresh(session, nested, true, visited)?;
    }
    visited.pop();
    let scene = &mut session.scene;
    let now = order(scene, root);
    for copy in kept {
        let Some(&target) = now.get(copy.at) else { continue };
        let Some(mut link) = scene.get::<Prefab>(target).cloned().filter(|l| l.source == copy.source) else { continue };
        let objects = order(scene, target);
        for (index, component, value) in copy.values {
            let Some(&object) = objects.get(index) else { continue };
            // Best effort: a change that no longer fits the file is dropped.
            let applied = match value {
                Some(value) => {
                    if scene.component(object, &component).is_err() {
                        let _ = scene.add_component(object, &component);
                    }
                    scene.set_component(object, &component, value).is_ok()
                }
                None => scene.component(object, &component).is_err() || scene.remove_component(object, &component).is_ok(),
            };
            let key = format!("{index}/{component}");
            if applied && !link.overrides.contains(&key) {
                link.overrides.push(key);
            }
        }
        let _ = scene.set(target, link);
    }
    Ok(())
}

/// Most prefabs inside one another.
const MAX_NESTING: usize = 16;

/// The overrides of copy `root` and of the copies inside it, with their current values.
fn capture(scene: &Scene, root: StableId) -> Vec<Kept> {
    let all = order(scene, root);
    all.iter()
        .enumerate()
        .filter_map(|(at, id)| {
            let link = scene.get::<Prefab>(*id)?;
            let objects = order(scene, *id);
            let values = link
                .parsed_overrides()
                .filter_map(|(index, component)| Some((index, component.to_owned(), scene.component(*objects.get(index)?, component).ok())))
                .collect();
            Some(Kept { at, source: link.source.clone(), values })
        })
        .collect()
}

/// Updates every copy in the scene from its file, keeping overrides. Copies whose file cannot be
/// read keep what the scene holds; the problems are returned.
pub(crate) fn refresh_all(session: &mut Session) -> Vec<String> {
    let outermost: Vec<StableId> = session
        .scene
        .world_matrices()
        .into_iter()
        .map(|(id, _)| id)
        .filter(|id| session.scene.get::<Prefab>(*id).is_some() && session.scene.parent(*id).is_none_or(|p| copy_of(&session.scene, p).is_none()))
        .collect();
    let mut problems = Vec::new();
    for id in outermost {
        if let Err(e) = refresh(session, id, true, &mut Vec::new()) {
            problems.push(e.to_string());
        }
    }
    problems
}
/// Makes `id` and its children match the prefab `objects`, keeping `id`'s own name, transform,
/// link and place in the hierarchy, so the copy stays where it was and keeps its id.
fn replace_contents(scene: &mut Scene, id: StableId, objects: &[ObjectDoc]) -> Result<(), CommandError> {
    let Some((root, rest)) = objects.split_first() else { return Ok(()) };
    for child in scene.children(Some(id)).to_vec() {
        scene.delete_object(child)?;
    }
    let keep = KEPT;
    let current: Vec<&'static str> = scene.components(id)?.into_iter().map(|(name, _)| name).collect();
    for name in current {
        if !keep.contains(&name) && !root.components.contains_key(name) {
            scene.remove_component(id, name)?;
        }
    }
    for (name, value) in root.components.iter().filter(|(name, _)| !keep.contains(&name.as_str())) {
        if scene.component(id, name).is_err() {
            scene.add_component(id, name)?;
        }
        scene.set_component(id, name, value.clone())?;
    }
    // The root's children become top-level objects of the import, under `id`.
    let children: Vec<ObjectDoc> = rest
        .iter()
        .cloned()
        .map(|mut o| {
            if o.parent == Some(root.id) {
                o.parent = None;
            }
            o
        })
        .collect();
    scene.import(&children, Some(id), None)?;
    Ok(())
}
