//! Input Map assets (`.binput`): a game's input actions, named `map/action`, each with a kind
//! (trigger, held or value) and its bindings. One map is active at a time. The project's asset is
//! chosen in Project Settings; an object can carry its own with an Input Map component.

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
const VERSION: u32 = 1;
/// The largest Input Map file read.
const MAX_FILE_BYTES: u64 = 1 << 20;

/// An Input Map asset as stored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputAsset {
    pub format: String,
    pub version: u32,
    /// The map active when play starts; empty for the first map listed.
    #[serde(default)]
    pub start: String,
    pub actions: Vec<InputAction>,
}

/// One action: `map/action`, its kind, and what presses it (`key:W`, `mouse:Left`, `pad:South`,
/// `axis:LeftX+`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputAction {
    pub name: String,
    #[serde(default)]
    pub kind: ActionKind,
    pub bindings: Vec<String>,
}

impl InputAsset {
    pub fn new(start: &str, actions: Vec<InputAction>) -> Self {
        Self { format: FORMAT.to_owned(), version: VERSION, start: start.to_owned(), actions }
    }

    /// The maps, checked, for play mode.
    pub fn maps(&self) -> Result<InputMaps, String> {
        let list: Vec<(String, ActionKind, Vec<String>)> = self.actions.iter().map(|a| (a.name.clone(), a.kind, a.bindings.clone())).collect();
        InputMaps::new(&list, &self.start)
    }

    /// A game's usual controls: Player (walking, jumping, firing) and Menu (moving and choosing).
    pub fn starter() -> Self {
        let action =
            |name: &str, kind, bindings: &[&str]| InputAction { name: name.to_owned(), kind, bindings: bindings.iter().map(|b| (*b).to_owned()).collect() };
        Self::new(
            "Player",
            vec![
                action("Player/MoveForward", ActionKind::Value, &["key:W", "key:ArrowUp", "axis:LeftY+"]),
                action("Player/MoveBack", ActionKind::Value, &["key:S", "key:ArrowDown", "axis:LeftY-"]),
                action("Player/MoveLeft", ActionKind::Value, &["key:A", "key:ArrowLeft", "axis:LeftX-"]),
                action("Player/MoveRight", ActionKind::Value, &["key:D", "key:ArrowRight", "axis:LeftX+"]),
                action("Player/Jump", ActionKind::Trigger, &["key:Space", "pad:South"]),
                action("Player/Fire", ActionKind::Held, &["mouse:Left", "axis:RightTrigger+"]),
                action("Menu/Up", ActionKind::Trigger, &["key:ArrowUp", "key:W", "pad:Up"]),
                action("Menu/Down", ActionKind::Trigger, &["key:ArrowDown", "key:S", "pad:Down"]),
                action("Menu/Confirm", ActionKind::Trigger, &["key:Enter", "key:Space", "pad:South"]),
                action("Menu/Back", ActionKind::Trigger, &["key:Escape", "pad:East"]),
            ],
        )
    }

    pub fn to_text(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }
}

/// Reads an Input Map asset; `path` is relative to the project folder or absolute.
pub fn read_asset(session: &Session, path: &Path) -> Result<InputAsset, String> {
    let path = session.allowed(&session.script_dir().join(path), Access::Read).map_err(|e| e.to_string())?;
    let size = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?.len();
    if size > MAX_FILE_BYTES {
        return Err(format!("{} is larger than {} KiB", path.display(), MAX_FILE_BYTES / 1024));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let asset: InputAsset = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    if asset.format != FORMAT || asset.version != VERSION {
        return Err(format!("{} is not a version {VERSION} Input Map", path.display()));
    }
    Ok(asset)
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
            params: "{path}: a .binput file; returns {asset: {start, actions: [{name: \"map/action\", kind: \"trigger\" | \"held\" | \"value\", bindings: [key:W | mouse:Left | pad:South | axis:LeftX+]}]}, maps, problem}",
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
            params: "{path, asset: {start, actions}}: checks the actions (names map/action, known bindings) and writes the file; play mode reads it from its next start",
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
    Ok(json!({ "asset": { "start": asset.start, "actions": asset.actions }, "maps": maps, "problem": problem }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetBody {
    #[serde(default)]
    start: String,
    actions: Vec<InputAction>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetParams {
    path: PathBuf,
    asset: AssetBody,
}

fn input_asset_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: SetParams = parse(params)?;
    let asset = InputAsset::new(&p.asset.start, p.asset.actions);
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
