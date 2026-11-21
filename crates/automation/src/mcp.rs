//! Model Context Protocol server: JSON-RPC 2.0 over stdio, one message per line.
//!
//! Every engine command is a tool; dots in command ids become underscores in tool names
//! (`scene.object.create` -> `scene_object_create`). The backend is either a local session
//! ([`LocalBackend`]) or a running editor reached through its control channel ([`BridgeBackend`]).

use std::io::{BufRead, Write};

use bayleaf_engine::confine::Confinement;
use bayleaf_engine::{CommandRegistry, Session};
use serde_json::{Value, json};

use crate::control::ControlClient;

/// Protocol versions this server speaks, newest first.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Commands that only read: their tools carry `readOnlyHint`.
const READ_ONLY: &[&str] = &["app.version", "scene.objects", "scene.components", "scene.object.inspect", "play.status"];

/// Where tool calls go.
pub trait Backend {
    /// The command list, as `engine.commands` returns it.
    fn commands(&mut self) -> Result<Vec<Value>, String>;
    fn execute(&mut self, command: &str, params: &Value) -> Result<Value, String>;
}

/// A session of its own, confined to the given folders.
pub struct LocalBackend {
    registry: CommandRegistry,
    session: Session,
}

impl LocalBackend {
    pub fn new(confinement: Confinement) -> Self {
        let mut session = Session::default();
        session.set_confinement(Some(confinement));
        Self { registry: CommandRegistry::with_builtins(), session }
    }
}

impl Backend for LocalBackend {
    fn commands(&mut self) -> Result<Vec<Value>, String> {
        self.registry.list(&self.session).into_iter().map(|c| serde_json::to_value(c).map_err(|e| e.to_string())).collect()
    }

    fn execute(&mut self, command: &str, params: &Value) -> Result<Value, String> {
        self.registry.execute(&mut self.session, command, params).map_err(|e| e.to_string())
    }
}

/// A running editor, through its control channel.
pub struct BridgeBackend {
    client: ControlClient,
}

impl BridgeBackend {
    pub fn new(client: ControlClient) -> Self {
        Self { client }
    }
}

impl Backend for BridgeBackend {
    fn commands(&mut self) -> Result<Vec<Value>, String> {
        match self.client.call("engine.commands", &Value::Null)? {
            Value::Array(items) => Ok(items),
            other => Err(format!("engine.commands returned {other}")),
        }
    }

    fn execute(&mut self, command: &str, params: &Value) -> Result<Value, String> {
        self.client.call("engine.execute", &json!({ "command": command, "params": params }))
    }
}

/// Tool name for a command id.
pub fn tool_name(command: &str) -> String {
    command.replace('.', "_")
}

/// Handles one JSON-RPC message; returns the reply line, or `None` for notifications.
pub fn handle(backend: &mut dyn Backend, line: &str) -> Option<String> {
    let message: Value = match serde_json::from_str(line) {
        Ok(message) => message,
        Err(e) => return Some(error_reply(&Value::Null, -32700, &format!("parse error: {e}"))),
    };
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    // A message without an id is a notification: never answered.
    let id = id?;
    let result = match method {
        "initialize" => Ok(initialize(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => tools_list(backend),
        "tools/call" => return Some(reply(&id, tools_call(backend, &params))),
        "" => Err((-32600, "invalid request: no method".to_owned())),
        other => Err((-32601, format!("method not found: {other}"))),
    };
    Some(match result {
        Ok(result) => reply(&id, result),
        Err((code, message)) => error_reply(&id, code, &message),
    })
}

/// Reads messages from `input` and writes replies to `output` until the input ends.
pub fn serve(backend: &mut dyn Backend, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle(backend, &line) {
            writeln!(output, "{reply}")?;
            output.flush()?;
        }
    }
    Ok(())
}

fn initialize(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
    let version = PROTOCOL_VERSIONS.iter().find(|v| **v == requested).copied().unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "bayleaf", "title": "Bayleaf Engine", "version": env!("CARGO_PKG_VERSION") },
        "instructions": "Every Bayleaf Engine command is a tool. Start with scene_objects to see the scene. \
            Object ids are 32-character hex strings. File paths are relative to the first write root, and files \
            outside the read and write roots are refused.",
    })
}

fn tools_list(backend: &mut dyn Backend) -> Result<Value, (i64, String)> {
    let commands = backend.commands().map_err(|e| (-32603, e))?;
    let tools: Vec<Value> = commands
        .iter()
        .filter_map(|c| {
            let id = c.get("id")?.as_str()?;
            let label = c.get("label").and_then(Value::as_str).unwrap_or(id);
            let params = c.get("params").and_then(Value::as_str).unwrap_or("none");
            let mut tool = json!({
                "name": tool_name(id),
                "title": label,
                "description": format!("{label}. Params: {params}. (Bayleaf command `{id}`.)"),
                "inputSchema": { "type": "object", "additionalProperties": true },
            });
            if READ_ONLY.contains(&id) {
                tool["annotations"] = json!({ "readOnlyHint": true });
            }
            Some(tool)
        })
        .collect();
    Ok(json!({ "tools": tools }))
}

/// Tool results report failures in the result (`isError`), so the model sees them.
fn tools_call(backend: &mut dyn Backend, params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
    let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
    let command = match backend.commands() {
        Ok(commands) => commands.iter().filter_map(|c| c.get("id").and_then(Value::as_str).map(str::to_owned)).find(|id| tool_name(id) == name),
        Err(e) => return tool_result(&e, true),
    };
    let Some(command) = command else {
        return tool_result(&format!("unknown tool: {name}"), true);
    };
    match backend.execute(&command, &arguments) {
        Ok(result) => tool_result(&serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string()), false),
        Err(e) => tool_result(&e, true),
    }
}

fn tool_result(text: &str, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

fn reply(id: &Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn error_reply(id: &Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

#[cfg(test)]
mod tests {
    use super::{LocalBackend, handle};
    use bayleaf_engine::confine::Confinement;
    use serde_json::{Value, json};

    fn backend(name: &str) -> (LocalBackend, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("bayleaf-mcp-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        (LocalBackend::new(Confinement::new(std::slice::from_ref(&dir), std::slice::from_ref(&dir)).unwrap()), dir)
    }

    fn call(backend: &mut LocalBackend, message: Value) -> Value {
        serde_json::from_str(&handle(backend, &message.to_string()).unwrap()).unwrap()
    }

    #[test]
    fn initialize_agrees_on_a_version_and_offers_tools() {
        let (mut b, dir) = backend("init");
        let reply = call(
            &mut b,
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}}),
        );
        assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");
        assert!(reply["result"]["capabilities"]["tools"].is_object());
        let unknown = call(&mut b, json!({"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {"protocolVersion": "1999-01-01"}}));
        assert_eq!(unknown["result"]["protocolVersion"], super::PROTOCOL_VERSIONS[0]);
        assert!(handle(&mut b, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none(), "notifications get no reply");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn every_command_is_a_tool_and_calls_reach_the_session() {
        let (mut b, dir) = backend("tools");
        let list = call(&mut b, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}));
        let tools = list["result"]["tools"].as_array().unwrap();
        let create = tools.iter().find(|t| t["name"] == "scene_object_create").unwrap();
        assert_eq!(create["inputSchema"]["type"], "object");
        assert!(tools.iter().find(|t| t["name"] == "scene_objects").unwrap()["annotations"]["readOnlyHint"].as_bool().unwrap());
        assert!(tools.iter().all(|t| t["name"].as_str().unwrap().chars().all(|c| c.is_ascii_alphanumeric() || c == '_')), "names suit every client");

        let created = call(
            &mut b,
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "scene_object_create", "arguments": {"name": "From MCP"}}}),
        );
        assert_eq!(created["result"]["isError"], false);
        let objects = call(&mut b, json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "scene_objects", "arguments": {}}}));
        assert!(objects["result"]["content"][0]["text"].as_str().unwrap().contains("From MCP"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn failures_are_reported_to_the_model_and_bad_messages_to_the_client() {
        let (mut b, dir) = backend("errors");
        let outside = call(
            &mut b,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "scene_saveAs", "arguments": {"path": "../../escape.bscene"}}}),
        );
        assert_eq!(outside["result"]["isError"], true);
        assert!(outside["result"]["content"][0]["text"].as_str().unwrap().contains("outside the allowed folders"));
        let unknown_tool = call(&mut b, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "format_disk"}}));
        assert_eq!(unknown_tool["result"]["isError"], true);
        assert_eq!(call(&mut b, json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"}))["error"]["code"], -32601);
        let parse: Value = serde_json::from_str(&handle(&mut b, "{not json").unwrap()).unwrap();
        assert_eq!(parse["error"]["code"], -32700);
        std::fs::remove_dir_all(&dir).ok();
    }
}
