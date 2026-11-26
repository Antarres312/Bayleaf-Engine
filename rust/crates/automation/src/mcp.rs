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
const READ_ONLY: &[&str] =
    &["app.version", "scene.objects", "scene.document", "asset.info", "graph.get", "scene.components", "scene.object.inspect", "play.status"];

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

/// How often the server looks for changes to subscribed resources while no message arrives.
pub const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// What the server can show as resources: URI, name, title, MIME type and description.
const RESOURCES: &[(&str, &str, &str, &str, &str)] = &[
    ("bayleaf://scene", "scene", "Scene", "application/json", "The open scene: every object and component, as the scene file holds them"),
    ("bayleaf://scene/render", "scene-render", "Scene picture", "image/png", "The scene from its first camera (or framing every object), 640 x 360"),
    ("bayleaf://graph", "graph", "Open graph", "application/json", "The graph open in the graph editor, its file and its problems"),
    ("bayleaf://play", "play", "Play mode", "application/json", "Whether play mode runs, its time, and the scripts' errors"),
];

/// The command a resource reads, its params, and the command whose result shows it changed.
fn resource_command(uri: &str) -> Option<(&'static str, Value, &'static str)> {
    Some(match uri {
        "bayleaf://scene" => ("scene.document", Value::Null, "scene.document"),
        "bayleaf://scene/render" => ("scene.render", json!({ "width": 640, "height": 360 }), "scene.document"),
        "bayleaf://graph" => ("graph.get", Value::Null, "graph.get"),
        "bayleaf://play" => ("play.status", Value::Null, "play.status"),
        _ => return None,
    })
}

/// One client's connection: the resources it subscribed to, with a fingerprint of what each
/// showed when it was last reported.
#[derive(Default)]
pub struct Server {
    subscriptions: std::collections::BTreeMap<String, u64>,
}

impl Server {
    pub fn new() -> Self {
        Self::default()
    }

    /// Handles one JSON-RPC message; returns the reply line, or `None` for notifications.
    pub fn handle(&mut self, backend: &mut dyn Backend, line: &str) -> Option<String> {
        let message: Value = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(e) => return Some(error_reply(&Value::Null, -32700, &format!("parse error: {e}"))),
        };
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str).unwrap_or_default();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        // A message without an id is a notification: never answered.
        let id = id?;
        let uri = params.get("uri").and_then(Value::as_str).unwrap_or_default().to_owned();
        let result = match method {
            "initialize" => Ok(initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => tools_list(backend),
            "tools/call" => return Some(reply(&id, tools_call(backend, &params))),
            "resources/list" => Ok(resources_list()),
            "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
            "resources/read" => resources_read(backend, &uri),
            "resources/subscribe" => match fingerprint(backend, &uri) {
                Some(print) => {
                    self.subscriptions.insert(uri, print);
                    Ok(json!({}))
                }
                None => Err(not_found(&uri)),
            },
            "resources/unsubscribe" => {
                self.subscriptions.remove(&uri);
                Ok(json!({}))
            }
            "" => Err((-32600, "invalid request: no method".to_owned())),
            other => Err((-32601, format!("method not found: {other}"))),
        };
        Some(match result {
            Ok(result) => reply(&id, result),
            Err((code, message)) => error_reply(&id, code, &message),
        })
    }

    /// `notifications/resources/updated` for each subscribed resource that changed since it was
    /// last reported.
    pub fn changes(&mut self, backend: &mut dyn Backend) -> Vec<String> {
        let mut out = Vec::new();
        for (uri, last) in &mut self.subscriptions {
            if let Some(print) = fingerprint(backend, uri)
                && print != *last
            {
                *last = print;
                out.push(json!({ "jsonrpc": "2.0", "method": "notifications/resources/updated", "params": { "uri": uri } }).to_string());
            }
        }
        out
    }
}

/// Handles one message without subscriptions (they need a [`Server`]).
pub fn handle(backend: &mut dyn Backend, line: &str) -> Option<String> {
    Server::new().handle(backend, line)
}

/// Reads messages from `input` and writes replies to `output` until the input ends. While no
/// message arrives, subscribed resources are checked every [`POLL_INTERVAL`], so changes made in
/// the editor are reported too.
pub fn serve(backend: &mut dyn Backend, input: impl BufRead + Send + 'static, mut output: impl Write) -> std::io::Result<()> {
    let (lines, incoming) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in input.lines() {
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    let mut server = Server::new();
    loop {
        let replies = match incoming.recv_timeout(POLL_INTERVAL) {
            Ok(line) => {
                let line = line?;
                let mut replies: Vec<String> = if line.trim().is_empty() { Vec::new() } else { server.handle(backend, &line).into_iter().collect() };
                replies.extend(server.changes(backend));
                replies
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => server.changes(backend),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        };
        for reply in replies {
            writeln!(output, "{reply}")?;
        }
        output.flush()?;
    }
}

fn not_found(uri: &str) -> (i64, String) {
    (-32002, format!("resource not found: {uri}"))
}

fn resources_list() -> Value {
    let resources: Vec<Value> = RESOURCES
        .iter()
        .map(|(uri, name, title, mime, description)| json!({ "uri": uri, "name": name, "title": title, "mimeType": mime, "description": description }))
        .collect();
    json!({ "resources": resources })
}

fn resources_read(backend: &mut dyn Backend, uri: &str) -> Result<Value, (i64, String)> {
    let (command, params, _) = resource_command(uri).ok_or_else(|| not_found(uri))?;
    let result = backend.execute(command, &params).map_err(|e| (-32603, e))?;
    let content = match result.get("png").and_then(Value::as_str) {
        Some(png) => json!({ "uri": uri, "mimeType": "image/png", "blob": png }),
        None => json!({ "uri": uri, "mimeType": "application/json", "text": serde_json::to_string_pretty(&result).unwrap_or_default() }),
    };
    Ok(json!({ "contents": [content] }))
}

/// A hash of what shows whether `uri` changed; `None` for an unknown resource. A command that
/// fails (no graph open) hashes its error, so opening one counts as a change.
fn fingerprint(backend: &mut dyn Backend, uri: &str) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let (_, _, watch) = resource_command(uri)?;
    let text = match backend.execute(watch, &Value::Null) {
        Ok(value) => value.to_string(),
        Err(e) => e,
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    Some(hasher.finish())
}
fn initialize(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
    let version = PROTOCOL_VERSIONS.iter().find(|v| **v == requested).copied().unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false }, "resources": { "subscribe": true, "listChanged": false } },
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
    use super::{LocalBackend, Server, handle};
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
        assert_eq!(call(&mut b, json!({"jsonrpc": "2.0", "id": 3, "method": "prompts/list"}))["error"]["code"], -32601);
        let parse: Value = serde_json::from_str(&handle(&mut b, "{not json").unwrap()).unwrap();
        assert_eq!(parse["error"]["code"], -32700);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resources_show_the_scene_and_subscribers_hear_when_it_changes() {
        let (mut b, dir) = backend("resources");
        let mut server = Server::new();
        let send = |b: &mut LocalBackend, server: &mut Server, message: Value| -> Value {
            serde_json::from_str(&server.handle(b, &message.to_string()).unwrap()).unwrap()
        };
        let list = send(&mut b, &mut server, json!({"jsonrpc": "2.0", "id": 1, "method": "resources/list"}));
        let uris: Vec<&str> = list["result"]["resources"].as_array().unwrap().iter().map(|r| r["uri"].as_str().unwrap()).collect();
        assert!(uris.contains(&"bayleaf://scene") && uris.contains(&"bayleaf://scene/render"), "{uris:?}");

        let subscribed = send(&mut b, &mut server, json!({"jsonrpc": "2.0", "id": 2, "method": "resources/subscribe", "params": {"uri": "bayleaf://scene"}}));
        assert!(subscribed["result"].is_object());
        assert!(server.changes(&mut b).is_empty(), "nothing changed yet");
        send(
            &mut b,
            &mut server,
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "scene_object_create", "arguments": {"name": "Watched"}}}),
        );
        let notes = server.changes(&mut b);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("notifications/resources/updated") && notes[0].contains("bayleaf://scene"));
        assert!(server.changes(&mut b).is_empty(), "each change is reported once");

        let read = send(&mut b, &mut server, json!({"jsonrpc": "2.0", "id": 4, "method": "resources/read", "params": {"uri": "bayleaf://scene"}}));
        assert!(read["result"]["contents"][0]["text"].as_str().unwrap().contains("Watched"));
        let missing = send(&mut b, &mut server, json!({"jsonrpc": "2.0", "id": 5, "method": "resources/read", "params": {"uri": "file:///etc/passwd"}}));
        assert_eq!(missing["error"]["code"], -32002);
        send(&mut b, &mut server, json!({"jsonrpc": "2.0", "id": 6, "method": "resources/unsubscribe", "params": {"uri": "bayleaf://scene"}}));
        send(&mut b, &mut server, json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": "scene_object_create", "arguments": {}}}));
        assert!(server.changes(&mut b).is_empty(), "no news after unsubscribing");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Answers control requests from a session of its own, like the editor does.
    struct SessionHandler {
        registry: bayleaf_engine::CommandRegistry,
        session: bayleaf_engine::Session,
    }

    impl crate::control::Handler for SessionHandler {
        fn commands(&mut self) -> Value {
            serde_json::to_value(self.registry.list(&self.session)).unwrap()
        }

        fn execute(&mut self, command: &str, params: &Value) -> Result<Value, String> {
            self.registry.execute(&mut self.session, command, params).map_err(|e| e.to_string())
        }
    }

    #[test]
    fn the_bridge_carries_tools_resources_and_refusals_to_the_running_session() {
        let dir = std::env::temp_dir().join(format!("bayleaf-mcp-bridge-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut session = bayleaf_engine::Session::default();
        session.set_confinement(Some(Confinement::new(std::slice::from_ref(&dir), std::slice::from_ref(&dir)).unwrap()));
        let mut handler = SessionHandler { registry: bayleaf_engine::CommandRegistry::with_builtins(), session };
        let token = "bridge-test-token-0123456789abcdef0123456789";
        let server = crate::control::ControlServer::start(0, token.to_owned(), || {}).unwrap();
        let addr = server.addr();

        // The MCP side, as `bayleaf-cli mcp --bridge` runs it.
        let agent = std::thread::spawn(move || {
            let mut backend = super::BridgeBackend::new(crate::control::ControlClient::connect(addr, token).unwrap());
            let mut mcp = Server::new();
            let mut send = |message: Value| -> Value { serde_json::from_str(&mcp.handle(&mut backend, &message.to_string()).unwrap()).unwrap() };
            let tools = send(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}));
            let created =
                send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "scene_object_create", "arguments": {"name": "Bridged"}}}));
            let refused = send(
                json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "scene_saveAs", "arguments": {"path": "../../escape.bscene"}}}),
            );
            let saved =
                send(json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "scene_saveAs", "arguments": {"path": "bridged.bscene"}}}));
            let read = send(json!({"jsonrpc": "2.0", "id": 5, "method": "resources/read", "params": {"uri": "bayleaf://scene"}}));
            (tools, created, refused, saved, read)
        });
        // The editor's side: answer between frames until the agent is done.
        while !agent.is_finished() {
            server.answer(&mut handler);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let (tools, created, refused, saved, read) = agent.join().unwrap();

        assert!(tools["result"]["tools"].as_array().unwrap().iter().any(|t| t["name"] == "scene_object_create"));
        assert_eq!(created["result"]["isError"], false);
        assert_eq!(refused["result"]["isError"], true, "the session's allowed folders hold over the bridge");
        assert_eq!(saved["result"]["isError"], false);
        assert!(dir.join("bridged.bscene").is_file());
        assert!(read["result"]["contents"][0]["text"].as_str().unwrap().contains("Bridged"));
        assert!(handler.session.scene().world_matrices().iter().any(|(id, _)| handler.session.scene().name(*id) == Some("Bridged")));
        std::fs::remove_dir_all(&dir).ok();
    }
}
