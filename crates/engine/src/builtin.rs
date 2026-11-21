//! Built-in commands that do not belong to a feature area: about, undo and redo.

use bayleaf_scene::Scene;
use serde_json::{Value, json};

use crate::{CommandError, CommandSpec, HistoryStep, Prompt, Session, always};

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "app.version",
            label: "About Bayleaf Engine",
            menu: "Help/About Bayleaf Engine",
            shortcut: "",
            params: "none",
            prompt: Prompt::None,
            mutates: false,
            enabled: always,
            run: app_version,
        },
        CommandSpec {
            id: "edit.undo",
            label: "Undo",
            menu: "Edit/Undo",
            shortcut: "Ctrl+Z",
            params: "none",
            prompt: Prompt::None,
            mutates: false,
            enabled: |s| if s.can_undo() { Ok(()) } else { Err("nothing to undo".to_owned()) },
            run: undo,
        },
        CommandSpec {
            id: "edit.redo",
            label: "Redo",
            menu: "Edit/Redo",
            shortcut: "Ctrl+Y",
            params: "none",
            prompt: Prompt::None,
            mutates: false,
            enabled: |s| if s.can_redo() { Ok(()) } else { Err("nothing to redo".to_owned()) },
            run: redo,
        },
    ]
}

fn app_version(_: &mut Session, _: &Value) -> Result<Value, CommandError> {
    Ok(json!({ "name": "Bayleaf Engine", "version": env!("CARGO_PKG_VERSION") }))
}

fn undo(session: &mut Session, _: &Value) -> Result<Value, CommandError> {
    let step = session.undo.pop().ok_or_else(|| CommandError::Failed("nothing to undo".to_owned()))?;
    let current = session.scene.to_doc();
    session.scene = Scene::from_doc(&step.before)?;
    session.dirty = true;
    let command = step.command.clone();
    session.redo.push(HistoryStep { command: step.command, before: current, merge_key: None });
    Ok(json!({ "undone": command }))
}

fn redo(session: &mut Session, _: &Value) -> Result<Value, CommandError> {
    let step = session.redo.pop().ok_or_else(|| CommandError::Failed("nothing to redo".to_owned()))?;
    let current = session.scene.to_doc();
    session.scene = Scene::from_doc(&step.before)?;
    session.dirty = true;
    let command = step.command.clone();
    session.undo.push(HistoryStep { command: step.command, before: current, merge_key: None });
    Ok(json!({ "redone": command }))
}
