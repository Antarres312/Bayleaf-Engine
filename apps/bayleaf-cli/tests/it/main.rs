//! Integration tests: the real `bayleaf-cli` binary, driven like an MCP client would.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("bayleaf-cli-it-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn mcp_refuses_to_start_without_both_roots() {
    let output = Command::new(env!("CARGO_BIN_EXE_bayleaf-cli")).arg("mcp").output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("automation-read-root"));
}

#[test]
fn mcp_over_stdio_creates_saves_and_stays_inside_the_roots() {
    let dir = temp_dir("mcp");
    let mut child = Command::new(env!("CARGO_BIN_EXE_bayleaf-cli"))
        .args(["mcp", "--automation-read-root"])
        .arg(&dir)
        .arg("--automation-write-root")
        .arg(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut send = |message: Value| -> Value {
        writeln!(stdin, "{message}").unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let init = send(
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "it", "version": "1"}}}),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "bayleaf");
    let tools = send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    assert!(tools["result"]["tools"].as_array().unwrap().len() > 20);
    let created = send(
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "scene_object_create", "arguments": {"name": "Agent Cube", "components": {"mesh": null}}}}),
    );
    assert_eq!(created["result"]["isError"], false);
    let saved = send(json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "scene_saveAs", "arguments": {"path": "agent.bscene"}}}));
    assert_eq!(saved["result"]["isError"], false, "{saved}");
    let escaped =
        send(json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "scene_saveAs", "arguments": {"path": "../escaped.bscene"}}}));
    assert_eq!(escaped["result"]["isError"], true);
    drop(stdin);
    assert!(child.wait().unwrap().success(), "the server exits when its input ends");
    let file = std::fs::read_to_string(dir.join("agent.bscene")).unwrap();
    assert!(file.contains("Agent Cube"));
    assert!(!dir.parent().unwrap().join("escaped.bscene").exists());
    std::fs::remove_dir_all(&dir).ok();
}
