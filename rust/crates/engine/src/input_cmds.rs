//! Input Map assets (`.binput`): a game's input, as a list of maps (Player, Menu…), each holding
//! its actions: a name, a kind (trigger, held or value) and its bindings. One map is active at a
//! time. Code and visual scripts name an action `map/action`, or just `action` in the active map.
//! The project's asset is chosen in Project Settings; an object can carry its own with an Input
//! Map component.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bayleaf_script::actions::{ActionKind, InputMaps};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::confine::Access;
use crate::{CommandError, CommandSpec, Mutates, Prompt, Session, always, parse};

/// File extension of Input Map assets.
pub const INPUT_EXTENSION: &str = "binput";
const FORMAT: &str = "bayleaf.input";
/// Version 2 lists maps, each with its actions; version 1 listed actions named `map/action`.
/// Version 1 files still read.
const VERSION: u32 = 2;
/// The largest Input Map file read.
const MAX_FILE_BYTES: u64 = 1 << 20;

/// An Input Map asset as stored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputAsset {
    pub format: String,
    pub version: u32,
    /// The map active when play starts; empty for the first map.
    #[serde(default)]
    pub start: String,
    pub maps: Vec<InputMapDef>,
}

/// One map: its name and its actions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputMapDef {
    pub name: String,
    pub actions: Vec<InputAction>,
}

/// One action of a map: its name, its kind, and what presses it (`key:W`, `mouse:Left`,
/// `pad:South`, `axis:LeftX+`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputAction {
    pub name: String,
    #[serde(default)]
    pub kind: ActionKind,
    pub bindings: Vec<String>,
}

impl InputAction {
    pub fn new(name: &str, kind: ActionKind, bindings: &[&str]) -> Self {
        Self { name: name.to_owned(), kind, bindings: bindings.iter().map(|b| (*b).to_owned()).collect() }
    }
}

/// A version 1 file: actions named `map/action` in one list.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FlatAsset {
    #[serde(default)]
    start: String,
    actions: Vec<InputAction>,
}

impl InputAsset {
    pub fn new(start: &str, maps: Vec<InputMapDef>) -> Self {
        Self { format: FORMAT.to_owned(), version: VERSION, start: start.to_owned(), maps }
    }

    /// The maps, checked, for play mode.
    pub fn maps(&self) -> Result<InputMaps, String> {
        let list: Vec<bayleaf_script::actions::MapSpec> =
            self.maps.iter().map(|m| (m.name.clone(), m.actions.iter().map(|a| (a.name.clone(), a.kind, a.bindings.clone())).collect())).collect();
        InputMaps::new(&list, &self.start)
    }

    /// How many actions all the maps hold.
    pub fn action_count(&self) -> usize {
        self.maps.iter().map(|m| m.actions.len()).sum()
    }

    /// A game's usual controls: Player (walking, jumping, firing) and Menu (moving and choosing).
    pub fn starter() -> Self {
        let map = |name: &str, actions| InputMapDef { name: name.to_owned(), actions };
        Self::new(
            "Player",
            vec![
                map(
                    "Player",
                    vec![
                        InputAction::new("MoveForward", ActionKind::Value, &["key:W", "key:ArrowUp", "axis:LeftY+"]),
                        InputAction::new("MoveBack", ActionKind::Value, &["key:S", "key:ArrowDown", "axis:LeftY-"]),
                        InputAction::new("MoveLeft", ActionKind::Value, &["key:A", "key:ArrowLeft", "axis:LeftX-"]),
                        InputAction::new("MoveRight", ActionKind::Value, &["key:D", "key:ArrowRight", "axis:LeftX+"]),
                        InputAction::new("Jump", ActionKind::Trigger, &["key:Space", "pad:South"]),
                        InputAction::new("Fire", ActionKind::Held, &["mouse:Left", "axis:RightTrigger+"]),
                    ],
                ),
                map(
                    "Menu",
                    vec![
                        InputAction::new("Up", ActionKind::Trigger, &["key:ArrowUp", "key:W", "pad:Up"]),
                        InputAction::new("Down", ActionKind::Trigger, &["key:ArrowDown", "key:S", "pad:Down"]),
                        InputAction::new("Confirm", ActionKind::Trigger, &["key:Enter", "key:Space", "pad:South"]),
                        InputAction::new("Back", ActionKind::Trigger, &["key:Escape", "pad:East"]),
                    ],
                ),
            ],
        )
    }

    /// Reads the file text: version 2, or version 1 with its `map/action` names sorted into maps.
    fn from_text(text: &str) -> Result<Self, String> {
        let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if value["format"] != FORMAT {
            return Err("not an Input Map".to_owned());
        }
        match value["version"].as_u64() {
            Some(2) => serde_json::from_value(value).map_err(|e| e.to_string()),
            Some(1) => {
                let mut body = value;
                if let Some(fields) = body.as_object_mut() {
                    fields.remove("format");
                    fields.remove("version");
                }
                let flat: FlatAsset = serde_json::from_value(body).map_err(|e| e.to_string())?;
                let mut maps: Vec<InputMapDef> = Vec::new();
                for action in flat.actions {
                    let (map, name) = action.name.split_once('/').ok_or_else(|| format!("{:?} is not named map/action", action.name))?;
                    let entry = InputAction { name: name.to_owned(), kind: action.kind, bindings: action.bindings.clone() };
                    match maps.iter_mut().find(|m| m.name == map) {
                        Some(found) => found.actions.push(entry),
                        None => maps.push(InputMapDef { name: map.to_owned(), actions: vec![entry] }),
                    }
                }
                Ok(Self::new(&flat.start, maps))
            }
            _ => Err(format!("not a version {VERSION} Input Map")),
        }
    }

    pub fn to_text(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }
}

/// Reads an Input Map asset; `path` is relative to the project folder or absolute.
pub fn read_asset(session: &Session, path: &Path) -> Result<InputAsset, String> {
    let path = session.allowed(&session.script_dir().join(path), Access::Read).map_err(|e| e.to_string())?;
    let size = bayleaf_core::files::len(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if size > MAX_FILE_BYTES {
        return Err(format!("{} is larger than {} KiB", path.display(), MAX_FILE_BYTES / 1024));
    }
    let text = bayleaf_core::files::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    InputAsset::from_text(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Reads and checks an Input Map asset, for play mode.
pub fn load_maps(session: &Session, path: &Path) -> Result<Arc<InputMaps>, String> {
    let asset = read_asset(session, path)?;
    asset.maps().map(Arc::new).map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes an Input Map asset after checking it.
pub fn write_asset(session: &Session, path: &Path, asset: &InputAsset) -> Result<PathBuf, CommandError> {
    if path.extension().is_none_or(|e| e != INPUT_EXTENSION) {
        return Err(CommandError::BadParams(format!("Input Map files end in .{INPUT_EXTENSION}")));
    }
    asset.maps().map_err(CommandError::BadParams)?;
    let path = session.allowed(&session.script_dir().join(path), Access::Write)?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|e| CommandError::Failed(format!("cannot make {}: {e}", folder.display())))?;
    }
    std::fs::write(&path, asset.to_text()).map_err(|e| CommandError::Failed(format!("cannot write {}: {e}", path.display())))?;
    Ok(path)
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "input.asset",
            label: "Read Input Map",
            menu: "",
            shortcut: "",
            params: "{path}: a .binput file; returns {asset: {start, maps: [{name, actions: [{name, kind: \"trigger\" | \"held\" | \"value\", bindings: [key:W | mouse:Left | pad:South | axis:LeftX+]}]}]}, maps, problem}; scripts name an action map/action",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: always,
            run: input_asset,
        },
        CommandSpec {
            id: "input.asset.set",
            label: "Change Input Map",
            menu: "",
            shortcut: "",
            params: "{path, asset: {start, maps: [{name, actions: [{name, kind, bindings}]}]}}: checks the maps and actions (names of letters, digits and _, unique; known bindings) and writes the file; play mode reads it from its next start",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: always,
            run: input_asset_set,
        },
        CommandSpec {
            id: "input.map.set",
            label: "Set Input Map",
            menu: "",
            shortcut: "",
            params: "{map, object?}: makes a map active, in the project's Input Map or an object's own (Input Map component); play mode only",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: crate::play::playing,
            run: input_map_set,
        },
        CommandSpec {
            id: "input.state",
            label: "Input State",
            menu: "",
            shortcut: "",
            params: "none; in play mode the active map of the project's Input Map and of each object's own, with their maps",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: always,
            run: input_state,
        },
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathParams {
    path: PathBuf,
}

fn input_asset(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PathParams = parse(params)?;
    let asset = read_asset(session, &p.path).map_err(CommandError::Failed)?;
    let (maps, problem) = match asset.maps() {
        Ok(maps) => (maps.maps().to_vec(), None),
        Err(problem) => (Vec::new(), Some(problem)),
    };
    Ok(json!({ "asset": { "start": asset.start, "maps": asset.maps }, "maps": maps, "problem": problem }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetBody {
    #[serde(default)]
    start: String,
    maps: Vec<InputMapDef>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetParams {
    path: PathBuf,
    asset: AssetBody,
}

fn input_asset_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: SetParams = parse(params)?;
    let asset = InputAsset::new(&p.asset.start, p.asset.maps);
    let path = write_asset(session, &p.path, &asset)?;
    Ok(json!({ "path": path }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MapParams {
    map: String,
    #[serde(default)]
    object: Option<bayleaf_core::StableId>,
}

fn input_map_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: MapParams = parse(params)?;
    let play = session.play.as_mut().ok_or_else(|| CommandError::Failed("maps switch only in play mode".to_owned()))?;
    let result = play.host.with_input(|set| match set.get_mut(p.object) {
        None => Err(match p.object {
            Some(id) => format!("{id} has no Input Map of its own"),
            None => "the project has no Input Map; choose one in Project Settings".to_owned(),
        }),
        Some(state) => {
            if state.set_active(&p.map) {
                Ok(())
            } else {
                Err(format!("there is no map {}; the maps are {}", p.map, state.maps().maps().join(", ")))
            }
        }
    });
    result.map_err(CommandError::BadParams)?;
    input_state(session, &Value::Null)
}

fn input_state(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    parse::<crate::play::NoParams>(params)?;
    let Some(play) = session.play.as_ref() else { return Ok(json!({ "playing": false })) };
    let describe = |state: &bayleaf_script::actions::InputState| json!({ "active": state.active(), "maps": state.maps().maps() });
    let (project, objects) = play.host.with_input(|set| {
        let objects: Vec<Value> = set
            .objects
            .iter()
            .map(|(id, state)| {
                let mut entry = describe(state);
                entry["object"] = json!(id);
                entry["name"] = json!(session.scene.name(*id));
                entry
            })
            .collect();
        (set.project.as_ref().map(describe), objects)
    });
    Ok(json!({ "playing": true, "project": project, "objects": objects }))
}
