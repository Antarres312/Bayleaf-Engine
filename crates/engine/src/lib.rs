//! L5: editing session, command registry, undo history and play mode.
//!
//! Every action is a [`CommandSpec`] in the [`CommandRegistry`]. The editor, `bayleaf-cli`, the
//! JSON control channel and the MCP server all dispatch the same command ids (AA 14).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod builtin;
mod scene_cmds;

/// Optional world streaming, present only with the `streaming` feature.
#[cfg(feature = "streaming")]
pub use bayleaf_streaming as streaming;

pub use bayleaf_scene as scene;

use std::path::{Path, PathBuf};

use bayleaf_scene::Scene;
use bayleaf_scene::doc::SceneDoc;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Errors returned when running a command.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("unknown command: {0}")]
    Unknown(String),
    #[error("command {id} is disabled: {reason}")]
    Disabled { id: String, reason: String },
    #[error("params must be a JSON object, null or omitted")]
    ParamsNotObject,
    #[error("invalid params: {0}")]
    BadParams(String),
    #[error("{0}")]
    Failed(String),
}

impl From<bayleaf_scene::SceneError> for CommandError {
    fn from(error: bayleaf_scene::SceneError) -> Self {
        Self::Failed(error.to_string())
    }
}

struct HistoryStep {
    command: String,
    before: SceneDoc,
    merge_key: Option<String>,
}

/// State that commands read and change: the open scene, its file and the undo history.
#[derive(Default)]
pub struct Session {
    scene: Scene,
    scene_path: Option<PathBuf>,
    dirty: bool,
    undo: Vec<HistoryStep>,
    redo: Vec<HistoryStep>,
}

impl Session {
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// File the scene was opened from or last saved to.
    pub fn scene_path(&self) -> Option<&Path> {
        self.scene_path.as_deref()
    }

    /// True when the scene has changes that are not saved.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Ids of the commands that can be undone, oldest first.
    pub fn history(&self) -> Vec<&str> {
        self.undo.iter().map(|s| s.command.as_str()).collect()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn replace_scene(&mut self, scene: Scene, path: Option<PathBuf>) {
        self.scene = scene;
        self.scene_path = path;
        self.dirty = false;
        self.undo.clear();
        self.redo.clear();
    }
}

type RunFn = fn(&mut Session, &Value) -> Result<Value, CommandError>;
type EnabledFn = fn(&Session) -> Result<(), String>;

/// What a front end has to ask the user for before running a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Prompt {
    /// Params can be null.
    None,
    /// `{"path"}` of an existing scene file.
    OpenScene,
    /// `{"path"}` to write a scene file to.
    SaveScene,
    /// `{"id"}` of the selected object.
    SelectedObject,
}

/// One registered command.
pub struct CommandSpec {
    /// Dotted id with camelCase segments, e.g. `scene.object.create`.
    pub id: &'static str,
    /// English label. It is the translation key.
    pub label: &'static str,
    /// Menu path, empty when the command has no menu entry.
    pub menu: &'static str,
    /// Default shortcut such as `Ctrl+Shift+S`, empty for none.
    pub shortcut: &'static str,
    /// Human-readable description of the params object.
    pub params: &'static str,
    pub prompt: Prompt,
    /// True when the command changes the scene and records one undo step.
    pub mutates: bool,
    pub enabled: EnabledFn,
    pub run: RunFn,
}

/// One row of `engine.commands`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub menu: &'static str,
    pub shortcut: &'static str,
    pub params: &'static str,
    pub prompt: Prompt,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

/// Registry of every command the engine knows.
pub struct CommandRegistry {
    commands: Vec<CommandSpec>,
}

impl CommandRegistry {
    /// Registry with the built-in commands.
    pub fn with_builtins() -> Self {
        let mut commands = builtin::commands();
        commands.extend(scene_cmds::commands());
        Self { commands }
    }

    pub fn get(&self, id: &str) -> Option<&CommandSpec> {
        self.commands.iter().find(|c| c.id == id)
    }

    pub fn list(&self, session: &Session) -> Vec<CommandInfo> {
        self.commands
            .iter()
            .map(|c| {
                let state = (c.enabled)(session);
                CommandInfo {
                    id: c.id,
                    label: c.label,
                    menu: c.menu,
                    shortcut: c.shortcut,
                    params: c.params,
                    prompt: c.prompt,
                    enabled: state.is_ok(),
                    disabled_reason: state.err(),
                }
            })
            .collect()
    }

    /// Runs `id` with `params`, which must be a JSON object or null.
    pub fn execute(&self, session: &mut Session, id: &str, params: &Value) -> Result<Value, CommandError> {
        self.execute_merging(session, id, params, None)
    }

    /// Like [`Self::execute`], but consecutive mutating runs with the same `merge_key` share one
    /// undo step. The editor uses this while a value is being dragged or typed.
    pub fn execute_merging(&self, session: &mut Session, id: &str, params: &Value, merge_key: Option<&str>) -> Result<Value, CommandError> {
        if !(params.is_object() || params.is_null()) {
            return Err(CommandError::ParamsNotObject);
        }
        let spec = self.get(id).ok_or_else(|| CommandError::Unknown(id.to_owned()))?;
        (spec.enabled)(session).map_err(|reason| CommandError::Disabled { id: id.to_owned(), reason })?;
        if !spec.mutates {
            return (spec.run)(session, params);
        }
        let before = session.scene.to_doc();
        match (spec.run)(session, params) {
            Ok(result) => {
                let merges = merge_key.is_some() && session.undo.last().is_some_and(|s| s.merge_key.as_deref() == merge_key);
                if !merges {
                    session.undo.push(HistoryStep { command: id.to_owned(), before, merge_key: merge_key.map(str::to_owned) });
                }
                session.redo.clear();
                session.dirty = true;
                Ok(result)
            }
            Err(error) => {
                // A failed command leaves the scene as it was.
                if let Ok(scene) = Scene::from_doc(&before) {
                    session.scene = scene;
                }
                Err(error)
            }
        }
    }
}

/// Parses a params object (null counts as `{}`) into `T`.
fn parse<T: DeserializeOwned>(params: &Value) -> Result<T, CommandError> {
    let value = if params.is_null() { Value::Object(serde_json::Map::new()) } else { params.clone() };
    serde_json::from_value(value).map_err(|e| CommandError::BadParams(e.to_string()))
}

fn always(_: &Session) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests;
