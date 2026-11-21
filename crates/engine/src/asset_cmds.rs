//! `asset.create`: new folders, scenes, material graphs, visual scripts and Rust scripts in the
//! project, for the Project panel's create menu and for agents. Never writes over a file.

use std::path::PathBuf;

use bayleaf_scene::Scene;
use bayleaf_scene::components::{Camera, DirectionalLight, Transform};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::confine::Access;
use crate::{CommandError, CommandSpec, Mutates, Prompt, Session, always, parse};

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "asset.create",
        label: "Create Asset",
        menu: "",
        shortcut: "",
        params: "{kind: \"folder\" | \"scene\" | \"material\" | \"visualScript\" | \"script\", folder, name}: makes it in the folder (relative to the project folder or absolute); returns {path}, and for a script {path, module}",
        prompt: Prompt::None,
        mutates: Mutates::No,
        enabled: always,
        run: asset_create,
    }]
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
}

impl AssetKind {
    /// The file extension, or `None` for a folder or a script's crate folder.
    pub fn extension(self) -> Option<&'static str> {
        match self {
            AssetKind::Folder | AssetKind::Script => None,
            AssetKind::Scene => Some(bayleaf_scene::doc::EXTENSION),
            AssetKind::Material => Some(bayleaf_render::material::CATALOG.extension),
            AssetKind::VisualScript => Some(bayleaf_script::visual::CATALOG.extension),
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
    }
    Ok(json!({ "path": path }))
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
