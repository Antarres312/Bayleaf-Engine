//! `asset.create`: new folders, scenes, material graphs, visual scripts and Rust scripts in the
//! project, for the Project panel's create menu and for agents. Never writes over a file.
//! `asset.delete`: a file or folder to the Recycle Bin (or gone for good, when asked).

use std::path::{Path, PathBuf};

use bayleaf_scene::Scene;
use bayleaf_scene::components::{Camera, DirectionalLight, Transform};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::confine::Access;
use crate::{CommandError, CommandSpec, Mutates, Prompt, Session, always, parse};

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "asset.create",
            label: "Create Asset",
            menu: "",
            shortcut: "",
            params: "{kind: \"folder\" | \"scene\" | \"material\" | \"visualScript\" | \"script\" | \"inputMap\" | \"lighting\" | \"postProcessing\" | \"interface\" | \"interfaceStyle\", folder, name}: makes it in the folder (relative to the project folder or absolute); returns {path}, and for a script {path, module}",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: always,
            run: asset_create,
        },
        CommandSpec {
            id: "asset.list",
            label: "List Assets",
            menu: "",
            shortcut: "",
            params: "{path?, recursive?}: what a folder of the project holds (the project folder when left out), as the Project panel lists it: hidden and build folders left out. Returns {entries: [{path, kind}], truncated}, paths relative to the project folder",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: always,
            run: asset_list,
        },
        CommandSpec {
            id: "asset.delete",
            label: "Delete Asset",
            menu: "",
            shortcut: "",
            params: "{path, permanent?}: moves a file or folder of the project (relative to the project folder or absolute) to the Recycle Bin; permanent: true deletes it for good, for where there is no Recycle Bin. Never the project folder itself. A Rust script leaves the project's workspace with it",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: always,
            run: asset_delete,
        },
        CommandSpec {
            id: "asset.rename",
            label: "Rename Asset",
            menu: "",
            shortcut: "",
            params: "{path, name}: renames a file or folder of the project (relative to the project folder or absolute); a file keeps its extension when the name leaves it out; a Rust script's folder renames its crate, its built module and its workspace entry too. Never over another file. Returns {path}",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: always,
            run: asset_rename,
        },
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameParams {
    path: PathBuf,
    name: String,
}

fn asset_rename(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: RenameParams = parse(params)?;
    let path = session.allowed(&session.script_dir().join(&p.path), Access::Write)?;
    if !path.exists() {
        return Err(CommandError::BadParams(format!("{} does not exist", path.display())));
    }
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    if let Some(project) = session.project_dir()
        && canonical(project).starts_with(canonical(&path))
    {
        return Err(CommandError::BadParams("the project folder itself, or a folder holding it, is not renamed here".to_owned()));
    }
    let name = p.name.trim();
    let renamed = if path.join("Cargo.toml").is_file() {
        // A Rust script: its crate, module and workspace entry follow the folder.
        let parent = path.parent().unwrap_or(&path);
        if let Some(new_name) = crate::code_scripts::crate_name(name) {
            session.allowed(&parent.join(&new_name), Access::Write)?;
        }
        let renamed = crate::code_scripts::rename_script(&path, name).map_err(CommandError::BadParams)?;
        if let Some(manifest) = session.project_dir().map(|p| p.join(crate::code_scripts::WORKSPACE_MANIFEST)).filter(|m| m.is_file()) {
            crate::code_scripts::move_workspace_member(&manifest, &path, &renamed).map_err(CommandError::Failed)?;
        }
        renamed
    } else {
        if !valid_file_name(name) {
            return Err(CommandError::BadParams(r#"a name has 1 to 100 characters and none of / \ : * ? " < > |"#.to_owned()));
        }
        let extension = if path.is_file() { path.extension().map(|e| e.to_string_lossy().into_owned()) } else { None };
        let file = match extension {
            Some(e) if !name.to_ascii_lowercase().ends_with(&format!(".{}", e.to_ascii_lowercase())) => format!("{name}.{e}"),
            _ => name.to_owned(),
        };
        let renamed = path.with_file_name(file);
        if renamed == path {
            return Ok(json!({ "path": path }));
        }
        // A name that differs only in case is a rename on any file system; anything else must be new.
        if renamed.exists() && !canonical(&renamed).eq(&canonical(&path)) {
            return Err(CommandError::Failed(format!("{} already exists; pick another name", renamed.display())));
        }
        let renamed = session.allowed(&renamed, Access::Write)?;
        std::fs::rename(&path, &renamed).map_err(|e| CommandError::Failed(format!("cannot rename {}: {e}", path.display())))?;
        // Its import settings go with it.
        let settings = bayleaf_render::assets::import::settings_path(&path);
        if path.is_file() || renamed.is_file() {
            let moved = session.allowed(&bayleaf_render::assets::import::settings_path(&renamed), Access::Write)?;
            if settings.is_file() {
                std::fs::rename(&settings, &moved).map_err(|e| CommandError::Failed(format!("cannot rename {}: {e}", settings.display())))?;
            }
        }
        renamed
    };
    // The open scene and graph follow their files.
    let follow = |open: &Path| open.strip_prefix(&path).ok().map(|rest| if rest.as_os_str().is_empty() { renamed.clone() } else { renamed.join(rest) });
    if let Some(scene) = session.scene_path.clone().and_then(|s| follow(&s)) {
        session.set_scene_path(Some(scene));
    }
    if let Some(doc) = session.graph.as_mut()
        && let Some(moved) = doc.path.as_deref().and_then(follow)
    {
        doc.path = Some(moved);
    }
    Ok(json!({ "path": renamed }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteParams {
    path: PathBuf,
    #[serde(default)]
    permanent: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListParams {
    #[serde(default)]
    path: String,
    #[serde(default)]
    recursive: bool,
}

/// Most entries one `asset.list` returns.
const MAX_LISTED: usize = 5000;

/// Folders never listed: build output and package caches.
const NOT_LISTED: &[&str] = &["target", "node_modules"];

/// What a file or folder is, as the Project panel names it.
fn asset_kind(path: &Path) -> &'static str {
    if path.is_dir() {
        return if path.join("Cargo.toml").is_file() { "Rust Script" } else { "Folder" };
    }
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("bscene") => "Scene",
        Some("bprefab") => "Prefab",
        Some("bmaterial") => "Material",
        Some("bscript") => "Visual Script",
        Some("wasm") => "Script",
        Some("binput") => "Input Map",
        Some("blight") => "Lighting",
        Some("bpost") => "Post-processing",
        Some("bui") => "Interface",
        Some("bstyle") => "Interface Style",
        Some("bproject") => "Project",
        Some(e) if bayleaf_render::assets::MODEL_EXTENSIONS.contains(&e) => "Model",
        Some(e) if bayleaf_render::assets::IMAGE_EXTENSIONS.contains(&e) => "Image",
        _ => "File",
    }
}

fn asset_list(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: ListParams = parse(params)?;
    let base = session.script_dir();
    let top = session.allowed(&base.join(&p.path), Access::Read)?;
    if !top.is_dir() {
        return Err(CommandError::BadParams(format!("{} is not a folder", top.display())));
    }
    let shown = |path: &Path| path.strip_prefix(&base).unwrap_or(path).to_string_lossy().replace('\\', "/");
    let (mut entries, mut truncated) = (Vec::new(), false);
    let mut pending = vec![top];
    while let Some(dir) = pending.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        let mut here: Vec<PathBuf> = read
            .flatten()
            .filter(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                !name.starts_with('.') && !(e.file_type().is_ok_and(|t| t.is_dir()) && NOT_LISTED.contains(&name.as_str()))
            })
            .map(|e| e.path())
            .filter(|p| !bayleaf_render::assets::import::is_settings_file(p))
            .collect();
        here.sort();
        for path in here {
            if entries.len() >= MAX_LISTED {
                truncated = true;
                break;
            }
            let kind = asset_kind(&path);
            if p.recursive && kind == "Folder" {
                pending.push(path.clone());
            }
            entries.push(json!({ "path": shown(&path), "kind": kind }));
        }
    }
    Ok(json!({ "entries": entries, "truncated": truncated }))
}

fn asset_delete(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: DeleteParams = parse(params)?;
    let path = session.allowed(&session.script_dir().join(&p.path), Access::Write)?;
    if !path.exists() {
        return Err(CommandError::BadParams(format!("{} does not exist", path.display())));
    }
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    if let Some(project) = session.project_dir()
        && canonical(project).starts_with(canonical(&path))
    {
        return Err(CommandError::BadParams("the project folder itself, or a folder holding it, is never deleted".to_owned()));
    }
    // Scripts in the deleted path leave the workspace first, so the others still build.
    let workspace = session.project_dir().map(|p| p.join(crate::code_scripts::WORKSPACE_MANIFEST)).filter(|m| m.is_file());
    if let Some(manifest) = &workspace {
        session.allowed(manifest, Access::Write)?;
        crate::code_scripts::leave_workspace(manifest, &path).map_err(CommandError::Failed)?;
    }
    let removed = if p.permanent {
        if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) }.map_err(|e| e.to_string())
    } else {
        to_bin(&path).map_err(|e| format!("{e}; with no Recycle Bin, delete with permanent: true"))
    };
    removed.map_err(|e| CommandError::Failed(format!("cannot delete {}: {e}", path.display())))?;
    // Its import settings go with it.
    let settings = bayleaf_render::assets::import::settings_path(&path);
    if settings.is_file() && session.allowed(&settings, Access::Write).is_ok() {
        let _ = if p.permanent { std::fs::remove_file(&settings).map_err(|e| e.to_string()) } else { to_bin(&settings) };
    }
    Ok(json!({ "path": path, "recycled": !p.permanent }))
}

/// Moves a file or folder to the system's Recycle Bin (or Trash).
#[cfg(not(target_arch = "wasm32"))]
fn to_bin(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|e| e.to_string())
}

/// A browser has no Recycle Bin.
#[cfg(target_arch = "wasm32")]
fn to_bin(_path: &Path) -> Result<(), String> {
    Err("a browser has no Recycle Bin".to_owned())
}

/// What `asset.create` makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AssetKind {
    Folder,
    Scene,
    Material,
    VisualScript,
    /// A Rust script crate, built to WebAssembly.
    Script,
    /// An Input Map: input actions in maps (`.binput`), starting with a game's usual controls.
    InputMap,
    /// A lighting asset (`.blight`): the sky, the light around everything, fog and a sun.
    Lighting,
    /// A post-processing asset (`.bpost`): what a camera does to the drawn picture.
    PostProcessing,
    /// A game interface layout (`.bui`): a title and a button to start from.
    Interface,
    /// A style file for game interfaces (`.bstyle`).
    InterfaceStyle,
}

impl AssetKind {
    /// The file extension, or `None` for a folder or a script's crate folder.
    pub fn extension(self) -> Option<&'static str> {
        match self {
            AssetKind::Folder | AssetKind::Script => None,
            AssetKind::Scene => Some(bayleaf_scene::doc::EXTENSION),
            AssetKind::Material => Some(bayleaf_render::material::CATALOG.extension),
            AssetKind::VisualScript => Some(bayleaf_script::visual::CATALOG.extension),
            AssetKind::InputMap => Some(crate::input_cmds::INPUT_EXTENSION),
            AssetKind::Lighting => Some(bayleaf_scene::lighting::EXTENSION),
            AssetKind::PostProcessing => Some(bayleaf_scene::post::EXTENSION),
            AssetKind::Interface => Some(bayleaf_interface::LAYOUT_EXTENSION),
            AssetKind::InterfaceStyle => Some(bayleaf_interface::STYLE_EXTENSION),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Params {
    kind: AssetKind,
    folder: PathBuf,
    name: String,
}

/// Whether `name` can name a file on every system: 1 to 100 characters, none of `/ \ : * ? " < > |`
/// or control characters, and not only dots or spaces.
pub fn valid_file_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && name.chars().count() <= 100
        && !name.chars().all(|c| c == '.' || c == ' ')
        && !name.chars().any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control())
}

fn asset_create(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: Params = parse(params)?;
    if p.kind == AssetKind::Script {
        return crate::code_scripts::script_new(session, &json!({ "folder": p.folder, "name": p.name }));
    }
    if !valid_file_name(&p.name) {
        return Err(CommandError::BadParams(r#"a name has 1 to 100 characters and none of / \ : * ? " < > |"#.to_owned()));
    }
    let folder = session.allowed(&session.script_dir().join(&p.folder), Access::Write)?;
    if !folder.is_dir() {
        return Err(CommandError::BadParams(format!("{} is not a folder", folder.display())));
    }
    let name = p.name.trim();
    let file = match p.kind.extension() {
        Some(extension) if !name.to_ascii_lowercase().ends_with(&format!(".{extension}")) => format!("{name}.{extension}"),
        _ => name.to_owned(),
    };
    let path = folder.join(file);
    if path.exists() {
        return Err(CommandError::Failed(format!("{} already exists; pick another name", path.display())));
    }
    let failed = |e: std::io::Error| CommandError::Failed(format!("cannot write {}: {e}", path.display()));
    match p.kind {
        // Scripts were made above, by `script.new`.
        AssetKind::Folder | AssetKind::Script => std::fs::create_dir(&path).map_err(failed)?,
        AssetKind::Scene => starter_scene()?.save(&path)?,
        AssetKind::Material => std::fs::write(&path, bayleaf_graph::Graph::new(&bayleaf_render::material::CATALOG).to_json()).map_err(failed)?,
        AssetKind::VisualScript => std::fs::write(&path, bayleaf_graph::Graph::new(&bayleaf_script::visual::CATALOG).to_json()).map_err(failed)?,
        AssetKind::InputMap => std::fs::write(&path, crate::input_cmds::InputAsset::starter().to_text()).map_err(failed)?,
        AssetKind::Lighting => std::fs::write(&path, bayleaf_scene::lighting::Lighting::default().to_text()).map_err(failed)?,
        AssetKind::PostProcessing => std::fs::write(&path, bayleaf_scene::post::PostProcessing::default().to_text()).map_err(failed)?,
        AssetKind::Interface => std::fs::write(&path, bayleaf_interface::Layout::starter().to_text()).map_err(failed)?,
        AssetKind::InterfaceStyle => std::fs::write(&path, starter_style().to_text()).map_err(failed)?,
    }
    Ok(json!({ "path": path }))
}

/// A new style file: one rule for buttons, to change from.
fn starter_style() -> bayleaf_interface::StyleSheet {
    let mut style = bayleaf_interface::StyleSheet::new();
    style.rules.push(bayleaf_interface::Rule {
        kind: Some(bayleaf_interface::Kind::Button),
        class: None,
        name: None,
        set: [("background", "#2d3440e6"), ("radius", "6px"), ("padding", "8px 16px")].into_iter().map(|(k, v)| (k.to_owned(), v.to_owned())).collect(),
    });
    style
}

/// A new scene: a camera looking at the centre and a sun, so it shows something when played.
fn starter_scene() -> Result<Scene, CommandError> {
    let mut scene = Scene::new();
    let camera = scene.create_object(None, "Camera")?;
    scene.set(camera, Transform { position: [0.0, 2.0, 6.0], rotation: [-15.0, 0.0, 0.0], ..Transform::default() })?;
    scene.set(camera, Camera::default())?;
    let sun = scene.create_object(None, "Sun")?;
    scene.set(sun, Transform { rotation: [-50.0, 30.0, 0.0], ..Transform::default() })?;
    scene.set(sun, DirectionalLight::default())?;
    Ok(scene)
}
