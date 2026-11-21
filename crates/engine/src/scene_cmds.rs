//! Scene commands: files, objects, hierarchy and components.

use std::path::PathBuf;

use bayleaf_core::StableId;
use bayleaf_scene::Scene;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{CommandError, CommandSpec, Prompt, Session, always, parse};

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "scene.new",
            label: "New Scene",
            menu: "File/New Scene",
            shortcut: "Ctrl+N",
            params: "none",
            prompt: Prompt::None,
            mutates: false,
            enabled: always,
            run: scene_new,
        },
        CommandSpec {
            id: "scene.open",
            label: "Open Scene…",
            menu: "File/Open Scene…",
            shortcut: "Ctrl+O",
            params: "{path}",
            prompt: Prompt::OpenScene,
            mutates: false,
            enabled: always,
            run: scene_open,
        },
        CommandSpec {
            id: "scene.save",
            label: "Save Scene",
            menu: "File/Save Scene",
            shortcut: "Ctrl+S",
            params: "none",
            prompt: Prompt::None,
            mutates: false,
            enabled: |s| if s.scene_path.is_some() { Ok(()) } else { Err("the scene has no file yet; use Save Scene As".to_owned()) },
            run: scene_save,
        },
        CommandSpec {
            id: "scene.saveAs",
            label: "Save Scene As…",
            menu: "File/Save Scene As…",
            shortcut: "Ctrl+Shift+S",
            params: "{path}",
            prompt: Prompt::SaveScene,
            mutates: false,
            enabled: always,
            run: scene_save_as,
        },
        CommandSpec {
            id: "scene.render",
            label: "Render Image…",
            menu: "",
            shortcut: "",
            params: "{path, width?, height?, camera?, grid?}; writes a PNG. camera: an object with a camera component; \
                     default: the first camera in the scene, else a view framing every object",
            prompt: Prompt::None,
            mutates: false,
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
            mutates: false,
            enabled: always,
            run: scene_objects,
        },
        CommandSpec {
            id: "scene.components",
            label: "List Component Types",
            menu: "",
            shortcut: "",
            params: "none",
            prompt: Prompt::None,
            mutates: false,
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
            mutates: true,
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
            mutates: true,
            enabled: always,
            run: object_delete,
        },
        CommandSpec {
            id: "scene.object.moveUp",
            label: "Move Up",
            menu: "Object/Move Up",
            shortcut: "Alt+ArrowUp",
            params: "{id}; moves before the previous sibling",
            prompt: Prompt::SelectedObject,
            mutates: true,
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
            mutates: true,
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
            mutates: true,
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
            mutates: true,
            enabled: always,
            run: |s, p| object_step(s, p, Step::Outdent),
        },
        CommandSpec {
            id: "scene.object.inspect",
            label: "Inspect Object",
            menu: "",
            shortcut: "",
            params: "{id}; returns {id, parent, children, components}",
            prompt: Prompt::SelectedObject,
            mutates: false,
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
            mutates: true,
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
            mutates: true,
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
            mutates: true,
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
            mutates: true,
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
    session.replace_scene(Scene::new(), None);
    Ok(json!({}))
}

fn scene_open(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PathParams = parse(params)?;
    let scene = Scene::load(&p.path)?;
    let objects = scene.object_count();
    session.replace_scene(scene, Some(p.path));
    Ok(json!({ "objects": objects }))
}

fn scene_save(session: &mut Session, _: &Value) -> Result<Value, CommandError> {
    let path = session.scene_path.clone().ok_or_else(|| CommandError::Failed("the scene has no file yet; use Save Scene As".to_owned()))?;
    session.scene.save(&path)?;
    session.dirty = false;
    Ok(json!({ "path": path }))
}

fn scene_save_as(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PathParams = parse(params)?;
    session.scene.save(&p.path)?;
    session.scene_path = Some(p.path.clone());
    session.dirty = false;
    Ok(json!({ "path": p.path }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenderParams {
    path: PathBuf,
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
    if !(1..=MAX_RENDER_SIDE).contains(&p.width) || !(1..=MAX_RENDER_SIDE).contains(&p.height) {
        return Err(CommandError::BadParams(format!("width and height must be between 1 and {MAX_RENDER_SIDE}")));
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
                let (center, radius) = bayleaf_render::bounds(scene).unwrap_or((bayleaf_core::math::Vec3::ZERO, 2.0));
                (bayleaf_render::CameraView::framing(center, radius, aspect), "framing")
            }
        },
    };
    let frame = bayleaf_render::extract(scene, camera, bayleaf_render::FrameOptions { show_grid: p.grid, selected: None });
    let gpu = bayleaf_render::HeadlessGpu::new().map_err(|e| CommandError::Failed(e.to_string()))?;
    let image = gpu.render(&frame, p.width, p.height).map_err(|e| CommandError::Failed(e.to_string()))?;
    image.write_png(&p.path).map_err(|e| CommandError::Failed(e.to_string()))?;
    Ok(json!({ "path": p.path, "width": p.width, "height": p.height, "camera": source }))
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
    Ok(json!({}))
}

fn component_add(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: ComponentParams = parse(params)?;
    session.scene.add_component(p.id, &p.component)?;
    Ok(json!({}))
}

fn component_remove(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: ComponentParams = parse(params)?;
    session.scene.remove_component(p.id, &p.component)?;
    Ok(json!({}))
}

fn yes() -> bool {
    true
}
