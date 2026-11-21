//! Projects: a folder with a `.bproject` file. Paths in the project's scenes (script modules,
//! materials) start at that folder, so scenes can live in subfolders. `project.new` makes a new
//! project folder with a small starting point to build on.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::confine::Access;
use crate::{CommandError, CommandSpec, Mutates, Prompt, Session, always, parse};

/// `format` of project files.
pub const PROJECT_FORMAT: &str = "bayleaf.project";
/// File extension of project files.
pub const PROJECT_EXTENSION: &str = "bproject";
/// Version 2 added `settings`; version 3 moved the input actions into an Input Map asset
/// (`settings.input.map`). Older files still open; opening one moves its actions.
const PROJECT_VERSION: u32 = 3;

/// A project file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectFile {
    pub format: String,
    pub version: u32,
    pub name: String,
    /// The scene the project opens with, relative to the project folder.
    pub main_scene: String,
    #[serde(default)]
    pub settings: ProjectSettings,
}

/// Settings for the whole project. Play mode reads them when it starts.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectSettings {
    #[serde(default)]
    pub physics: PhysicsSettings,
    #[serde(default)]
    pub input: InputSettings,
}

/// The project's input: an Input Map asset, the one map set every script and Player Control read
/// (an object's own Input Map component comes first for that object).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputSettings {
    /// A `.binput` file, relative to the project folder; empty for none.
    #[serde(default)]
    pub map: String,
    /// Actions written in the project file before format 3. Opening the project moves them into
    /// an Input Map asset; until then play mode uses them as the map `Default`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<ActionSetting>>,
}

/// An action as project files before format 3 kept it, with its bindings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionSetting {
    pub name: String,
    pub bindings: Vec<String>,
}

/// The map old inline actions are moved into.
const LEGACY_MAP: &str = "Default";

impl InputSettings {
    /// Old inline actions as an Input Map asset: map `Default`, every action a value, as they read before.
    pub fn legacy_asset(&self) -> Option<crate::input_cmds::InputAsset> {
        let actions = self.actions.as_ref()?;
        let list = actions
            .iter()
            .map(|a| crate::input_cmds::InputAction {
                name: format!("{LEGACY_MAP}/{}", a.name),
                kind: bayleaf_script::actions::ActionKind::Value,
                bindings: a.bindings.clone(),
            })
            .collect();
        Some(crate::input_cmds::InputAsset::new(LEGACY_MAP, list))
    }
}

/// Physics in play mode.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PhysicsSettings {
    /// Off: Rigid Bodies and Colliders do nothing in play mode.
    pub enabled: bool,
    /// World units per second squared.
    pub gravity: [f32; 3],
    /// Single is faster; Double stays exact far from the world's centre (beyond about 10 km).
    #[serde(default)]
    pub precision: PhysicsPrecision,
}

/// How precisely physics counts positions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PhysicsPrecision {
    #[default]
    Single,
    Double,
}

impl Default for PhysicsSettings {
    fn default() -> Self {
        Self { enabled: true, gravity: [0.0, -9.81, 0.0], precision: PhysicsPrecision::Single }
    }
}

/// Largest gravity along any axis.
pub const MAX_GRAVITY: f32 = 1000.0;

impl ProjectSettings {
    fn check(&self) -> Result<(), CommandError> {
        if !self.physics.gravity.iter().all(|g| g.is_finite() && g.abs() <= MAX_GRAVITY) {
            return Err(CommandError::BadParams(format!("gravity values must be finite and at most {MAX_GRAVITY} in size")));
        }
        let map = &self.input.map;
        if !(map.is_empty() || (map.len() <= 512 && map.ends_with(&format!(".{}", crate::input_cmds::INPUT_EXTENSION)))) {
            return Err(CommandError::BadParams(format!("input.map must be the path of a .{} file", crate::input_cmds::INPUT_EXTENSION)));
        }
        Ok(())
    }
}

/// The open project's settings, or the defaults for a scene outside any project (or an unreadable file).
pub fn settings_of(session: &Session) -> ProjectSettings {
    session.project_dir().and_then(project_file_in).and_then(|file| ProjectFile::read(&file).ok()).map(|p| p.settings).unwrap_or_default()
}

impl ProjectFile {
    fn read(path: &Path) -> Result<Self, CommandError> {
        let text = std::fs::read_to_string(path).map_err(|e| CommandError::Failed(format!("cannot read {}: {e}", path.display())))?;
        let file: ProjectFile = serde_json::from_str(&text).map_err(|e| CommandError::Failed(format!("{} is not a project file: {e}", path.display())))?;
        if file.format != PROJECT_FORMAT || !(1..=PROJECT_VERSION).contains(&file.version) {
            return Err(CommandError::Failed(format!("{} is not a version {PROJECT_VERSION} Bayleaf project", path.display())));
        }
        Ok(file)
    }
}

/// `relative` (with `/` separators) inside `dir`, spelled with the platform's separators.
fn inside(dir: &Path, relative: &str) -> PathBuf {
    relative.split('/').filter(|part| !part.is_empty()).fold(dir.to_path_buf(), |path, part| path.join(part))
}

/// The project file in `dir`, if it has one.
pub fn project_file_in(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.is_file() && p.extension().is_some_and(|e| e == PROJECT_EXTENSION))
}

/// The project folder a scene file belongs to: the nearest folder above it with a project file.
pub fn project_dir_of(scene: &Path) -> Option<PathBuf> {
    scene.parent()?.ancestors().find(|dir| project_file_in(dir).is_some()).map(Path::to_path_buf)
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "project.new",
            label: "New Project…",
            menu: "File/New Project…",
            shortcut: "",
            params: "{path, name?}: makes the folder `path` (it must not exist or be empty) with a starting scene, a material and a visual script, then opens the scene",
            prompt: Prompt::NewProject,
            mutates: Mutates::No,
            enabled: crate::not_playing,
            run: project_new,
        },
        CommandSpec {
            id: "project.open",
            label: "Open Project…",
            menu: "File/Open Project…",
            shortcut: "",
            params: "{path}: a .bproject file or its folder; opens the project's main scene",
            prompt: Prompt::OpenProject,
            mutates: Mutates::No,
            enabled: crate::not_playing,
            run: project_open,
        },
        CommandSpec {
            id: "project.info",
            label: "Project Info",
            menu: "",
            shortcut: "",
            params: "none; the open project's folder, name and main scene, or null",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: always,
            run: project_info,
        },
        CommandSpec {
            id: "project.settings",
            label: "Project Settings",
            menu: "",
            shortcut: "",
            params: "none; returns {project, settings}: the open project's settings (physics: {enabled, gravity, precision: \"single\" | \"double\"}, input: {map: the project's Input Map, a .binput file (see input.asset)}), or the defaults and project: false outside a project",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: always,
            run: project_settings,
        },
        CommandSpec {
            id: "project.settings.set",
            label: "Change Project Settings",
            menu: "",
            shortcut: "",
            params: "{settings}: the whole settings object, written to the project file; play mode uses it from its next start",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: |s| {
                if s.project_dir().is_some() { Ok(()) } else { Err("open a project first; scenes outside a project use the default settings".to_owned()) }
            },
            run: project_settings_set,
        },
    ]
}

fn project_settings(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let _: serde_json::Map<String, Value> = crate::parse(params)?;
    Ok(json!({ "project": session.project_dir().is_some(), "settings": settings_of(session) }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsParams {
    settings: ProjectSettings,
}

fn project_settings_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: SettingsParams = crate::parse(params)?;
    p.settings.check()?;
    let dir = session.project_dir().ok_or_else(|| CommandError::Failed("open a project first".to_owned()))?;
    let file = project_file_in(dir).ok_or_else(|| CommandError::Failed("the project file is gone".to_owned()))?;
    let path = session.allowed(&file, Access::Write)?;
    let mut project = ProjectFile::read(&path)?;
    project.settings = p.settings;
    project.version = PROJECT_VERSION;
    let text = serde_json::to_string_pretty(&project).map_err(|e| CommandError::Failed(e.to_string()))?;
    std::fs::write(&path, text).map_err(|e| CommandError::Failed(format!("cannot write {}: {e}", path.display())))?;
    Ok(json!({ "settings": project.settings }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewParams {
    path: PathBuf,
    #[serde(default)]
    name: Option<String>,
}

fn project_new(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: NewParams = parse(params)?;
    let dir = session.allowed(&p.path, Access::Write)?;
    if dir.exists() && std::fs::read_dir(&dir).map(|mut d| d.next().is_some()).unwrap_or(true) {
        return Err(CommandError::Failed(format!("{} already exists and is not empty; pick a new folder", dir.display())));
    }
    let name = p.name.unwrap_or_else(|| dir.file_name().map_or_else(|| "New Game".to_owned(), |n| n.to_string_lossy().into_owned()));
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(CommandError::BadParams("the project name must have 1 to 100 characters".to_owned()));
    }
    let file_stem: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let write = |relative: &str, text: &str| -> Result<(), CommandError> {
        let path = dir.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| CommandError::Failed(format!("cannot make {}: {e}", parent.display())))?;
        }
        std::fs::write(&path, text).map_err(|e| CommandError::Failed(format!("cannot write {}: {e}", path.display())))
    };
    let project = ProjectFile {
        format: PROJECT_FORMAT.to_owned(),
        version: PROJECT_VERSION,
        name: name.to_owned(),
        main_scene: "scenes/main.bscene".to_owned(),
        settings: ProjectSettings { input: InputSettings { map: STARTER_INPUT_MAP.to_owned(), actions: None }, ..ProjectSettings::default() },
    };
    write(&format!("{file_stem}.{PROJECT_EXTENSION}"), &serde_json::to_string_pretty(&project).unwrap_or_default())?;
    write("scenes/main.bscene", MAIN_SCENE)?;
    write("materials/ground.bmaterial", GROUND_MATERIAL)?;
    write("scripts/spin.bscript", SPIN_SCRIPT)?;
    write(STARTER_INPUT_MAP, &crate::input_cmds::InputAsset::starter().to_text())?;
    write("assets/README.md", "Images, sounds and other files the game uses go here.\n")?;
    write(".gitignore", "# Files the editor or build tools make again.\n/build/\n/.build/\n*.wasm\n")?;
    write(
        "README.md",
        &format!(
            "# {name}\n\nA Bayleaf Engine project. Open `{file_stem}.{PROJECT_EXTENSION}` in the editor.\n\n- `scenes/`: scenes; `main.bscene` opens first\n- `materials/`: material graphs\n- `scripts/`: visual scripts and WebAssembly scripts\n- `assets/`: images, sounds and other files\n- `saves/`: values the game keeps between plays, such as a best score (safe to delete)\n"
        ),
    )?;
    let scene = inside(&dir, &project.main_scene);
    open_scene_at(session, &scene)?;
    Ok(json!({ "path": dir, "name": name, "scene": scene }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathParams {
    path: PathBuf,
}

fn project_open(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PathParams = parse(params)?;
    let path = session.allowed(&p.path, Access::Read)?;
    let file = if path.is_dir() {
        project_file_in(&path).ok_or_else(|| CommandError::Failed(format!("{} has no .{PROJECT_EXTENSION} file", path.display())))?
    } else {
        path
    };
    let project = ProjectFile::read(&file)?;
    let dir = file.parent().map(Path::to_path_buf).unwrap_or_default();
    move_inline_actions(session, &file, &dir, &project);
    let scene = inside(&dir, &project.main_scene);
    open_scene_at(session, &scene)?;
    Ok(json!({ "path": dir, "name": project.name, "scene": scene }))
}

/// Where a new project keeps its Input Map, relative to the project folder.
const STARTER_INPUT_MAP: &str = "input/game.binput";

/// A project from before format 3 keeps its input actions in the project file: they move into an
/// Input Map asset (`input/actions.binput`, map `Default`) that the settings then name. When that
/// cannot be written, nothing changes and play mode still reads them from the project file.
fn move_inline_actions(session: &mut Session, file: &Path, dir: &Path, project: &ProjectFile) {
    let Some(asset) = project.settings.input.legacy_asset().filter(|_| project.settings.input.map.is_empty()) else { return };
    let relative = (1..100)
        .map(|n| if n == 1 { "input/actions.binput".to_owned() } else { format!("input/actions_{n}.binput") })
        .find(|r| !inside(dir, r).exists())
        .unwrap_or_else(|| "input/actions.binput".to_owned());
    let moved = (|| -> Result<(), CommandError> {
        let path = session.allowed(file, Access::Write)?;
        crate::input_cmds::write_asset(session, &inside(dir, &relative), &asset)?;
        let mut updated = project.clone();
        updated.settings.input = InputSettings { map: relative.clone(), actions: None };
        updated.version = PROJECT_VERSION;
        let text = serde_json::to_string_pretty(&updated).map_err(|e| CommandError::Failed(e.to_string()))?;
        std::fs::write(&path, text).map_err(|e| CommandError::Failed(format!("cannot write {}: {e}", path.display())))
    })();
    match moved {
        Ok(()) => session.log(crate::Level::Info, format!("The project's input actions moved into {relative} (map Default), chosen in Project Settings.")),
        Err(error) => session.log(crate::Level::Info, format!("The project's input actions stay in the project file for now: {error}")),
    }
}

fn open_scene_at(session: &mut Session, scene: &Path) -> Result<(), CommandError> {
    let path = session.allowed(scene, Access::Read)?;
    let loaded = bayleaf_scene::Scene::load(&path)?;
    // A prefab file that cannot be read leaves its copies as the scene holds them.
    let _ = session.replace_scene(loaded, Some(path));
    Ok(())
}

fn project_info(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    if !(params.is_null() || params.as_object().is_some_and(serde_json::Map::is_empty)) {
        return Err(CommandError::BadParams("project.info takes no params".to_owned()));
    }
    let Some(dir) = session.project_dir() else { return Ok(Value::Null) };
    let file = project_file_in(dir).ok_or_else(|| CommandError::Failed("the project file is gone".to_owned()))?;
    let project = ProjectFile::read(&file)?;
    Ok(json!({ "path": dir, "name": project.name, "mainScene": project.main_scene }))
}

/// The starting scene: a camera, the sun and sky, a checkered ground and a spinning cube.
const MAIN_SCENE: &str = r#"{
  "format": "bayleaf.scene",
  "version": 7,
  "objects": [
    {
      "id": "0b1f6e3a8c2d4e5f9a7b6c5d4e3f2a10",
      "parent": null,
      "components": {
        "name": "Camera",
        "camera": { "projection": "perspective", "fov": 60.0, "size": 5.0, "near": 0.1, "far": 500.0 },
        "transform": { "position": [0.0, 3.0, 8.0], "rotation": [-18.0, 0.0, 0.0], "scale": [1.0, 1.0, 1.0] }
      }
    },
    {
      "id": "0b1f6e3a8c2d4e5f9a7b6c5d4e3f2a11",
      "parent": null,
      "components": {
        "name": "Sun",
        "directionalLight": { "color": [1.0, 0.96, 0.9], "intensity": 3.0 },
        "environment": { "skyColor": [0.18, 0.36, 0.72], "horizonColor": [0.62, 0.72, 0.82], "groundColor": [0.22, 0.2, 0.18], "ambient": 1.0, "showSky": true },
        "transform": { "position": [0.0, 5.0, 0.0], "rotation": [-50.0, 30.0, 0.0], "scale": [1.0, 1.0, 1.0] }
      }
    },
    {
      "id": "0b1f6e3a8c2d4e5f9a7b6c5d4e3f2a12",
      "parent": null,
      "components": {
        "name": "Ground",
        "mesh": { "shape": "plane", "color": [0.3, 0.36, 0.32, 1.0], "metallic": 0.0, "roughness": 0.9, "material": "materials/ground.bmaterial" },
        "transform": { "position": [0.0, 0.0, 0.0], "rotation": [0.0, 0.0, 0.0], "scale": [20.0, 1.0, 20.0] }
      }
    },
    {
      "id": "0b1f6e3a8c2d4e5f9a7b6c5d4e3f2a13",
      "parent": null,
      "components": {
        "name": "Cube",
        "mesh": { "shape": "cube", "color": [0.85, 0.45, 0.25, 1.0], "metallic": 0.0, "roughness": 0.45 },
        "scripts": [ { "module": "scripts/spin.bscript" } ],
        "transform": { "position": [0.0, 0.5, 0.0], "rotation": [0.0, 0.0, 0.0], "scale": [1.0, 1.0, 1.0] }
      }
    }
  ]
}
"#;

/// A checkered ground in two greens.
const GROUND_MATERIAL: &str = r#"{
  "format": "bayleaf.graph",
  "version": 1,
  "domain": "material",
  "nodes": [
    { "id": 1, "kind": "output", "position": [560.0, 40.0], "values": { "roughness": 0.9 } },
    { "id": 2, "kind": "checker", "position": [0.0, 40.0] },
    { "id": 3, "kind": "blend", "position": [280.0, 40.0], "values": { "a": [0.07, 0.085, 0.08], "b": [0.13, 0.16, 0.14] } }
  ],
  "links": [
    { "from": { "node": 2, "pin": "value" }, "to": { "node": 3, "pin": "factor" } },
    { "from": { "node": 3, "pin": "result" }, "to": { "node": 1, "pin": "baseColor" } }
  ]
}
"#;

/// Turns its object around Y by its Speed property, in degrees per second.
const SPIN_SCRIPT: &str = r#"{
  "format": "bayleaf.graph",
  "version": 1,
  "domain": "script",
  "nodes": [
    { "id": 1, "kind": "update", "position": [40.0, 40.0] },
    { "id": 2, "kind": "property", "position": [40.0, 200.0], "values": { "name": "speed", "default": 45.0 } },
    { "id": 3, "kind": "multiply", "position": [300.0, 200.0] },
    { "id": 4, "kind": "vector", "position": [540.0, 200.0] },
    { "id": 5, "kind": "rotateBy", "position": [780.0, 40.0] }
  ],
  "links": [
    { "from": { "node": 2, "pin": "value" }, "to": { "node": 3, "pin": "a" } },
    { "from": { "node": 1, "pin": "dt" }, "to": { "node": 3, "pin": "b" } },
    { "from": { "node": 3, "pin": "result" }, "to": { "node": 4, "pin": "y" } },
    { "from": { "node": 1, "pin": "then" }, "to": { "node": 5, "pin": "run" } },
    { "from": { "node": 4, "pin": "vector" }, "to": { "node": 5, "pin": "degrees" } }
  ]
}
"#;
