//! Scene commands: files, objects, hierarchy and components.

use std::path::PathBuf;

use bayleaf_core::StableId;
use bayleaf_scene::Scene;
use bayleaf_scene::components::{ScriptRef, Scripts};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{CommandError, CommandSpec, Mutates, Prompt, Session, always, parse};

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "scene.new",
            label: "New Scene",
            menu: "File/New Scene",
            shortcut: "Ctrl+N",
            params: "none",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: crate::not_playing,
            run: scene_new,
        },
        CommandSpec {
            id: "scene.open",
            label: "Open Scene…",
            menu: "File/Open Scene…",
            shortcut: "Ctrl+O",
            params: "{path}",
            prompt: Prompt::OpenScene,
            mutates: Mutates::No,
            enabled: crate::not_playing,
            run: scene_open,
        },
        CommandSpec {
            id: "scene.save",
            label: "Save Scene",
            menu: "File/Save Scene",
            shortcut: "Ctrl+S",
            params: "none",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: |s| {
                crate::not_playing(s)?;
                if s.scene_path.is_some() { Ok(()) } else { Err("the scene has no file yet; use Save Scene As".to_owned()) }
            },
            run: scene_save,
        },
        CommandSpec {
            id: "scene.saveAs",
            label: "Save Scene As…",
            menu: "File/Save Scene As…",
            shortcut: "Ctrl+Shift+S",
            params: "{path}",
            prompt: Prompt::SaveScene,
            mutates: Mutates::No,
            enabled: crate::not_playing,
            run: scene_save_as,
        },
        CommandSpec {
            id: "scene.render",
            label: "Render Image…",
            menu: "",
            shortcut: "",
            params: "{path?, width?, height?, camera?, grid?}; writes a PNG, or without a path returns it as {png: base64} \
                     (at most 2048 on a side). camera: an object with a camera component; \
                     default: the first camera in the scene, else a view framing every object",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: always,
            run: scene_render,
        },
        CommandSpec {
            id: "scene.objects",
            label: "List Objects",
            menu: "",
            shortcut: "",
            params: "none; returns the object tree",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: always,
            run: scene_objects,
        },
        CommandSpec {
            id: "scene.document",
            label: "Show Scene Document",
            menu: "",
            shortcut: "",
            params: "none; returns {path, dirty, scene}: every object and component, as the scene file holds them",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: always,
            run: scene_document,
        },
        CommandSpec {
            id: "scene.components",
            label: "List Component Types",
            menu: "",
            shortcut: "",
            params: "none",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: always,
            run: scene_components,
        },
        CommandSpec {
            id: "scene.object.create",
            label: "Create Object",
            menu: "Object/Create Empty Object",
            shortcut: "Ctrl+Shift+N",
            params: "{parent?, name?, index?, components?: {name: value or null}}; returns {id}",
            prompt: Prompt::None,
            mutates: Mutates::Scene,
            enabled: always,
            run: object_create,
        },
        CommandSpec {
            id: "scene.object.delete",
            label: "Delete Object",
            menu: "Object/Delete Object",
            shortcut: "Delete",
            params: "{id}",
            prompt: Prompt::SelectedObject,
            mutates: Mutates::Scene,
            enabled: always,
            run: object_delete,
        },
        CommandSpec {
            id: "scene.object.duplicate",
            label: "Duplicate",
            menu: "Edit/Duplicate",
            shortcut: "Ctrl+D",
            params: "{id}; the copy, with its children, goes right after the original. Returns {id}",
            prompt: Prompt::SelectedObject,
            mutates: Mutates::Scene,
            enabled: always,
            run: object_duplicate,
        },
        CommandSpec {
            id: "scene.object.copy",
            label: "Copy",
            menu: "",
            shortcut: "",
            params: "{ids: [id, ...]}; returns the objects and their children as clipboard data for scene.object.paste",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: always,
            run: object_copy,
        },
        CommandSpec {
            id: "scene.object.paste",
            label: "Paste",
            menu: "",
            shortcut: "",
            params: "{objects: <data from scene.object.copy>, parent?: id, index?: n}; returns the new {ids}",
            prompt: Prompt::None,
            mutates: Mutates::Scene,
            enabled: always,
            run: object_paste,
        },
        CommandSpec {
            id: "scene.object.moveUp",
            label: "Move Up",
            menu: "Object/Move Up",
            shortcut: "Alt+ArrowUp",
            params: "{id}; moves before the previous sibling",
            prompt: Prompt::SelectedObject,
            mutates: Mutates::Scene,
            enabled: always,
            run: |s, p| object_step(s, p, Step::Up),
        },
        CommandSpec {
            id: "scene.object.moveDown",
            label: "Move Down",
            menu: "Object/Move Down",
            shortcut: "Alt+ArrowDown",
            params: "{id}; moves after the next sibling",
            prompt: Prompt::SelectedObject,
            mutates: Mutates::Scene,
            enabled: always,
            run: |s, p| object_step(s, p, Step::Down),
        },
        CommandSpec {
            id: "scene.object.indent",
            label: "Make Child of Previous",
            menu: "Object/Make Child of Previous",
            shortcut: "Alt+ArrowRight",
            params: "{id}; becomes the last child of the previous sibling",
            prompt: Prompt::SelectedObject,
            mutates: Mutates::Scene,
            enabled: always,
            run: |s, p| object_step(s, p, Step::Indent),
        },
        CommandSpec {
            id: "scene.object.outdent",
            label: "Move Out of Parent",
            menu: "Object/Move Out of Parent",
            shortcut: "Alt+ArrowLeft",
            params: "{id}; moves right after its parent",
            prompt: Prompt::SelectedObject,
            mutates: Mutates::Scene,
            enabled: always,
            run: |s, p| object_step(s, p, Step::Outdent),
        },
        CommandSpec {
            id: "scene.script.add",
            label: "Add Script",
            menu: "",
            shortcut: "",
            params: "{id, module}; module: .wasm or .bscript path relative to the project folder (or the scene file's folder outside a project); returns {index}",
            prompt: Prompt::SelectedObject,
            mutates: Mutates::Scene,
            enabled: always,
            run: script_add,
        },
        CommandSpec {
            id: "scene.script.remove",
            label: "Remove Script",
            menu: "",
            shortcut: "",
            params: "{id, index}",
            prompt: Prompt::SelectedObject,
            mutates: Mutates::Scene,
            enabled: always,
            run: script_remove,
        },
        CommandSpec {
            id: "scene.script.setProperty",
            label: "Set Script Property",
            menu: "",
            shortcut: "",
            params: "{id, index, name, value}: value is a number, true/false, three numbers or text, as the script declares the property (an object property: the object's name); null returns to the script's default",
            prompt: Prompt::SelectedObject,
            mutates: Mutates::Scene,
            enabled: always,
            run: script_set_property,
        },
        CommandSpec {
            id: "scene.object.inspect",
            label: "Inspect Object",
            menu: "",
            shortcut: "",
            params: "{id}; returns {id, parent, children, components}",
            prompt: Prompt::SelectedObject,
            mutates: Mutates::No,
            enabled: always,
            run: object_inspect,
        },
        CommandSpec {
            id: "scene.object.reparent",
            label: "Move Object",
            menu: "",
            shortcut: "",
            params: "{id, parent?, index?, keepWorld? = true}; parent null moves to the root",
            prompt: Prompt::SelectedObject,
            mutates: Mutates::Scene,
            enabled: always,
            run: object_reparent,
        },
        CommandSpec {
            id: "scene.component.set",
            label: "Set Component Value",
            menu: "",
            shortcut: "",
            params: "{id, component, value}",
            prompt: Prompt::SelectedObject,
            mutates: Mutates::Scene,
            enabled: always,
            run: component_set,
        },
        CommandSpec {
            id: "scene.component.add",
            label: "Add Component",
            menu: "",
            shortcut: "",
            params: "{id, component}",
            prompt: Prompt::SelectedObject,
            mutates: Mutates::Scene,
            enabled: always,
            run: component_add,
        },
        CommandSpec {
            id: "scene.component.remove",
            label: "Remove Component",
            menu: "",
            shortcut: "",
            params: "{id, component}",
            prompt: Prompt::SelectedObject,
            mutates: Mutates::Scene,
            enabled: always,
            run: component_remove,
        },
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathParams {
    path: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdParams {
    id: StableId,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateParams {
    #[serde(default)]
    parent: Option<StableId>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    index: Option<usize>,
    /// Components to add, by name; a null value keeps the component's default.
    #[serde(default)]
    components: std::collections::BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReparentParams {
    id: StableId,
    #[serde(default)]
    parent: Option<StableId>,
    #[serde(default)]
    index: Option<usize>,
    /// Keep the object where it is in the world (default) instead of keeping its local transform.
    #[serde(default = "yes", rename = "keepWorld")]
    keep_world: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentParams {
    id: StableId,
    component: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetParams {
    id: StableId,
    component: String,
    value: Value,
}

fn scene_new(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    parse::<serde_json::Map<String, Value>>(params)?;
    let _ = session.replace_scene(Scene::new(), None);
    Ok(json!({}))
}

fn scene_open(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PathParams = parse(params)?;
    let path = session.allowed(&p.path, crate::confine::Access::Read)?;
    let scene = Scene::load(&path)?;
    let objects = scene.object_count();
    let warnings = session.replace_scene(scene, Some(path));
    Ok(json!({ "objects": objects, "warnings": warnings }))
}

fn scene_save(session: &mut Session, _: &Value) -> Result<Value, CommandError> {
    let path = session.scene_path.clone().ok_or_else(|| CommandError::Failed("the scene has no file yet; use Save Scene As".to_owned()))?;
    let path = session.allowed(&path, crate::confine::Access::Write)?;
    session.scene.save(&path)?;
    session.dirty = false;
    Ok(json!({ "path": path }))
}

fn scene_save_as(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PathParams = parse(params)?;
    let path = session.allowed(&p.path, crate::confine::Access::Write)?;
    session.scene.save(&path)?;
    session.set_scene_path(Some(path.clone()));
    session.dirty = false;
    Ok(json!({ "path": path }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenderParams {
    #[serde(default)]
    path: Option<PathBuf>,
    #[serde(default = "default_width")]
    width: u32,
    #[serde(default = "default_height")]
    height: u32,
    #[serde(default)]
    camera: Option<StableId>,
    #[serde(default)]
    grid: bool,
}

fn default_width() -> u32 {
    1280
}

fn default_height() -> u32 {
    720
}

/// Largest image side `scene.render` accepts.
const MAX_RENDER_SIDE: u32 = 8192;

fn scene_render(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: RenderParams = parse(params)?;
    // Checked before any rendering work.
    let path = p.path.as_ref().map(|path| session.allowed(path, crate::confine::Access::Write)).transpose()?;
    let largest = if path.is_some() { MAX_RENDER_SIDE } else { MAX_INLINE_RENDER_SIDE };
    if !(1..=largest).contains(&p.width) || !(1..=largest).contains(&p.height) {
        return Err(CommandError::BadParams(format!("width and height must be between 1 and {largest}")));
    }
    let scene = &session.scene;
    let aspect = p.width as f32 / p.height as f32;
    let (camera, source) = match p.camera {
        Some(id) => {
            (bayleaf_render::object_camera(scene, id, aspect).ok_or_else(|| CommandError::Failed(format!("object {id} has no camera component")))?, "object")
        }
        None => match bayleaf_render::scene_camera(scene, aspect) {
            Some(view) => (view, "scene"),
            None => {
                let (center, radius) = bayleaf_render::bounds(scene).unwrap_or((bayleaf_core::math::DVec3::ZERO, 2.0));
                (bayleaf_render::CameraView::framing(center, radius, aspect), "framing")
            }
        },
    };
    let mut materials = session.material_library();
    let mut cache = bayleaf_render::assets::AssetCache::new();
    let allow = |path: &std::path::Path| session.allowed(path, crate::confine::Access::Read).map_err(|e| e.to_string());
    let mut assets =
        bayleaf_render::FileAssets { base: session.script_dir(), materials: &mut materials, cache: &mut cache, allow: &allow, warnings: Vec::new() };
    let options = bayleaf_render::FrameOptions { show_grid: p.grid, selected: None, time: 0.0 };
    let frame = bayleaf_render::extract_with(scene, camera, options, &mut assets);
    let warnings = assets.warnings;
    let gpu = bayleaf_render::HeadlessGpu::new().map_err(|e| CommandError::Failed(e.to_string()))?;
    let image = gpu.render(&frame, p.width, p.height).map_err(|e| CommandError::Failed(e.to_string()))?;
    match path {
        Some(path) => {
            image.write_png(&path).map_err(|e| CommandError::Failed(e.to_string()))?;
            Ok(json!({ "path": path, "width": p.width, "height": p.height, "camera": source, "warnings": warnings }))
        }
        None => {
            let png = image.png_bytes().map_err(|e| CommandError::Failed(e.to_string()))?;
            Ok(json!({ "png": base64(&png), "width": p.width, "height": p.height, "camera": source, "warnings": warnings }))
        }
    }
}

/// Largest side of a render returned inline, without a file.
const MAX_INLINE_RENDER_SIDE: u32 = 2048;

/// Standard base64 with padding (RFC 4648), for images returned in JSON.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            out.push(if i <= chunk.len() { char::from(ALPHABET[((n >> shift) & 63) as usize]) } else { '=' });
        }
    }
    out
}

fn scene_document(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let _: serde_json::Map<String, Value> = parse(params)?;
    Ok(json!({ "path": session.scene_path, "dirty": session.dirty, "scene": session.scene.to_doc() }))
}

fn tree(scene: &Scene, parent: Option<StableId>) -> Vec<Value> {
    scene.children(parent).iter().map(|id| json!({ "id": id, "name": scene.name(*id), "children": tree(scene, Some(*id)) })).collect()
}

fn scene_objects(session: &mut Session, _: &Value) -> Result<Value, CommandError> {
    Ok(Value::Array(tree(&session.scene, None)))
}

fn scene_components(session: &mut Session, _: &Value) -> Result<Value, CommandError> {
    let kinds: Vec<Value> = session.scene.registry().kinds().iter().map(|k| json!({ "name": k.name, "label": k.label, "removable": k.removable })).collect();
    Ok(Value::Array(kinds))
}

fn object_create(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: CreateParams = parse(params)?;
    let id = session.scene.create_object(p.parent, p.name.as_deref().unwrap_or("Object"))?;
    if p.index.is_some() {
        session.scene.reparent(id, p.parent, p.index)?;
    }
    for (component, value) in p.components {
        if session.scene.component(id, &component).is_err() {
            session.scene.add_component(id, &component)?;
        }
        if !value.is_null() {
            session.scene.set_component(id, &component, value)?;
        }
    }
    Ok(json!({ "id": id }))
}

fn object_delete(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: IdParams = parse(params)?;
    let deleted = session.scene.delete_object(p.id)?;
    Ok(json!({ "deleted": deleted }))
}

/// `format` of copied objects (clipboard data).
pub const CLIPBOARD_FORMAT: &str = "bayleaf.objects";

fn object_duplicate(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: IdParams = parse(params)?;
    let scene = &mut session.scene;
    let objects = scene.export(&[p.id])?;
    let parent = scene.parent(p.id);
    let index = scene.children(parent).iter().position(|c| *c == p.id).map(|i| i + 1);
    let copy = *scene.import(&objects, parent, index)?.first().ok_or_else(|| CommandError::Failed("nothing was copied".to_owned()))?;
    let name = next_name(scene, parent, scene.name(p.id).unwrap_or("Object"));
    scene.set_component(copy, "name", Value::String(name))?;
    Ok(json!({ "id": copy }))
}

/// `name` with the lowest number after it that no sibling uses: "Crate" becomes "Crate 2".
fn next_name(scene: &Scene, parent: Option<StableId>, name: &str) -> String {
    let base = match name.rsplit_once(' ') {
        Some((base, number)) if !base.is_empty() && number.parse::<u32>().is_ok() => base,
        _ => name,
    };
    let taken: Vec<&str> = scene.children(parent).iter().filter_map(|c| scene.name(*c)).collect();
    (2..).map(|n| format!("{base} {n}")).find(|candidate| !taken.contains(&candidate.as_str())).unwrap_or_else(|| base.to_owned())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CopyParams {
    ids: Vec<StableId>,
}

fn object_copy(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: CopyParams = parse(params)?;
    let objects = session.scene.export(&p.ids)?;
    Ok(json!({ "format": CLIPBOARD_FORMAT, "version": bayleaf_scene::doc::VERSION, "objects": objects }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasteParams {
    objects: Value,
    #[serde(default)]
    parent: Option<StableId>,
    #[serde(default)]
    index: Option<usize>,
}

fn object_paste(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PasteParams = parse(params)?;
    // Either the whole clipboard data or just its object list.
    let list = match &p.objects {
        Value::Object(data) if data.get("format").and_then(Value::as_str) == Some(CLIPBOARD_FORMAT) => data.get("objects").cloned().unwrap_or(Value::Null),
        Value::Object(_) => return Err(CommandError::BadParams(format!("objects must be a list or data with format \"{CLIPBOARD_FORMAT}\""))),
        other => other.clone(),
    };
    let objects: Vec<bayleaf_scene::doc::ObjectDoc> = serde_json::from_value(list).map_err(|e| CommandError::BadParams(format!("objects: {e}")))?;
    let ids = session.scene.import(&objects, p.parent, p.index)?;
    Ok(json!({ "ids": ids }))
}

fn object_inspect(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: IdParams = parse(params)?;
    let scene = &session.scene;
    let components: serde_json::Map<String, Value> = scene.components(p.id)?.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
    Ok(json!({ "id": p.id, "parent": scene.parent(p.id), "children": scene.children(Some(p.id)), "components": components }))
}

#[derive(Clone, Copy)]
enum Step {
    Up,
    Down,
    Indent,
    Outdent,
}

/// One-step moves in the hierarchy, for keyboards and menus. The object keeps its world placement.
fn object_step(session: &mut Session, params: &Value, step: Step) -> Result<Value, CommandError> {
    let p: IdParams = parse(params)?;
    let scene = &session.scene;
    if !scene.contains(p.id) {
        return Err(CommandError::Failed(format!("no object with id {}", p.id)));
    }
    let parent = scene.parent(p.id);
    let siblings = scene.children(parent);
    let index = siblings.iter().position(|s| *s == p.id).unwrap_or(0);
    let (new_parent, new_index) = match step {
        Step::Up => (parent, Some(index.checked_sub(1).ok_or_else(|| CommandError::Failed("the object is already first".to_owned()))?)),
        Step::Down => {
            if index + 1 >= siblings.len() {
                return Err(CommandError::Failed("the object is already last".to_owned()));
            }
            (parent, Some(index + 1))
        }
        Step::Indent => {
            let previous = index.checked_sub(1).and_then(|i| siblings.get(i)).copied();
            (Some(previous.ok_or_else(|| CommandError::Failed("there is no previous object to move into".to_owned()))?), None)
        }
        Step::Outdent => {
            let old_parent = parent.ok_or_else(|| CommandError::Failed("the object is already at the root".to_owned()))?;
            let grandparent = scene.parent(old_parent);
            let at = scene.children(grandparent).iter().position(|s| *s == old_parent).map_or(0, |i| i + 1);
            (grandparent, Some(at))
        }
    };
    session.scene.reparent_keeping_world(p.id, new_parent, new_index)?;
    Ok(json!({ "parent": new_parent }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptAddParams {
    id: StableId,
    /// Module path relative to the project folder, or the scene file's folder outside a project.
    module: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptIndexParams {
    id: StableId,
    index: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptPropertyParams {
    id: StableId,
    index: usize,
    name: String,
    /// A value (a number, true/false, three numbers or text, as the script declares the property)
    /// sets it; null goes back to the script's declared default.
    value: Option<bayleaf_scene::components::PropertyValue>,
}

fn scripts_of(session: &Session, id: StableId) -> Result<Scripts, CommandError> {
    if !session.scene.contains(id) {
        return Err(CommandError::Failed(format!("no object with id {id}")));
    }
    Ok(session.scene.get::<Scripts>(id).cloned().unwrap_or_default())
}

fn script_add(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: ScriptAddParams = parse(params)?;
    let mut scripts = scripts_of(session, p.id)?;
    let module = p.module.replace('\\', "/");
    scripts.0.push(ScriptRef { module, properties: Default::default() });
    let index = scripts.0.len() - 1;
    session.scene.set(p.id, scripts)?;
    crate::prefab_cmds::note_override(&mut session.scene, p.id, "scripts");
    Ok(json!({ "index": index }))
}

fn script_remove(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: ScriptIndexParams = parse(params)?;
    let mut scripts = scripts_of(session, p.id)?;
    if p.index >= scripts.0.len() {
        return Err(CommandError::BadParams(format!("the object has {} scripts; there is no index {}", scripts.0.len(), p.index)));
    }
    scripts.0.remove(p.index);
    if scripts.0.is_empty() {
        session.scene.remove_component(p.id, "scripts")?;
    } else {
        session.scene.set(p.id, scripts)?;
    }
    crate::prefab_cmds::note_override(&mut session.scene, p.id, "scripts");
    Ok(json!({}))
}

fn script_set_property(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: ScriptPropertyParams = parse(params)?;
    let mut scripts = scripts_of(session, p.id)?;
    let count = scripts.0.len();
    let script = scripts.0.get_mut(p.index).ok_or_else(|| CommandError::BadParams(format!("the object has {count} scripts; there is no index {}", p.index)))?;
    if p.name.is_empty() || p.name.len() > 64 {
        return Err(CommandError::BadParams("a property name has 1 to 64 bytes".to_owned()));
    }
    match p.value {
        Some(value) => {
            script.properties.insert(p.name, value);
        }
        None => {
            script.properties.remove(&p.name);
        }
    }
    session.scene.set(p.id, scripts)?;
    crate::prefab_cmds::note_override(&mut session.scene, p.id, "scripts");
    Ok(json!({}))
}

fn object_reparent(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: ReparentParams = parse(params)?;
    if p.keep_world {
        session.scene.reparent_keeping_world(p.id, p.parent, p.index)?;
    } else {
        session.scene.reparent(p.id, p.parent, p.index)?;
    }
    Ok(json!({}))
}

fn component_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: SetParams = parse(params)?;
    session.scene.set_component(p.id, &p.component, p.value)?;
    crate::prefab_cmds::note_override(&mut session.scene, p.id, &p.component);
    Ok(json!({}))
}

fn component_add(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: ComponentParams = parse(params)?;
    session.scene.add_component(p.id, &p.component)?;
    crate::prefab_cmds::note_override(&mut session.scene, p.id, &p.component);
    Ok(json!({}))
}

fn component_remove(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: ComponentParams = parse(params)?;
    session.scene.remove_component(p.id, &p.component)?;
    crate::prefab_cmds::note_override(&mut session.scene, p.id, &p.component);
    Ok(json!({}))
}

fn yes() -> bool {
    true
}
