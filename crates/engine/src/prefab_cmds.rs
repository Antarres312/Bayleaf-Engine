//! Prefabs: an object and its children saved to a `.bprefab` file, to place again and again.
//!
//! A placed copy keeps a `prefab` link to its file. Nothing updates on its own: Apply writes a
//! copy's objects to the file and updates every other copy in the scene, Revert reads the file
//! again. Each copy keeps its own name and transform.

use std::path::{Path, PathBuf};

use bayleaf_core::StableId;
use bayleaf_scene::Scene;
use bayleaf_scene::components::Prefab;
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
    ]
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

fn read_prefab(session: &Session, source: &str) -> Result<Vec<ObjectDoc>, CommandError> {
    let path = session.allowed(&session.script_dir().join(source), Access::Read)?;
    let text = std::fs::read_to_string(&path).map_err(|e| CommandError::Failed(format!("cannot read {}: {e}", path.display())))?;
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
    let written = write_prefab(session, &path, &session.scene, p.id)?;
    let source = link_path(session, &written);
    session.scene.set(p.id, Prefab { source: source.clone() })?;
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
    session.scene.set(id, Prefab { source })?;
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
    let path = session.script_dir().join(&source);
    write_prefab(session, &path, &session.scene, p.id)?;
    let objects = read_prefab(session, &source)?;
    let others: Vec<StableId> = session
        .scene
        .world_matrices()
        .into_iter()
        .map(|(id, _)| id)
        .filter(|id| *id != p.id && session.scene.get::<Prefab>(*id).is_some_and(|l| l.source == source))
        .collect();
    for other in &others {
        replace_contents(&mut session.scene, *other, &objects)?;
    }
    Ok(json!({ "updated": others.len() }))
}

fn prefab_revert(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: IdParams = parse(params)?;
    let source = source_of(session, p.id)?;
    let objects = read_prefab(session, &source)?;
    replace_contents(&mut session.scene, p.id, &objects)?;
    Ok(json!({}))
}

/// Makes `id` and its children match the prefab `objects`, keeping `id`'s own name, transform,
/// link and place in the hierarchy, so the copy stays where it was and keeps its id.
fn replace_contents(scene: &mut Scene, id: StableId, objects: &[ObjectDoc]) -> Result<(), CommandError> {
    let Some((root, rest)) = objects.split_first() else { return Ok(()) };
    for child in scene.children(Some(id)).to_vec() {
        scene.delete_object(child)?;
    }
    let keep = ["name", "transform", "prefab"];
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
