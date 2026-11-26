//! Control channel of a running editor: newline-delimited JSON over a loopback TCP socket.
//!
//! Request `{"id": 1, "method": "engine.execute", "params": {"command": "...", "params": {...}}}`,
//! reply `{"id": 1, "ok": true, "result": ...}` or `{"id": 1, "ok": false, "error": "..."}`.
//! The first request on every connection must be `{"method": "auth", "params": {"token": "..."}}`
//! with the token from the token file; anything else closes the connection.
//! Methods besides `auth`: `engine.commands` and `engine.execute`, answered by the editor's UI thread.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};
use std::time::Duration;

use serde_json::{Value, json};

/// Longest request line.
pub const MAX_LINE: usize = 1 << 20;
/// Open connections at most.
pub const MAX_CONNECTIONS: usize = 16;
/// A connection that sends nothing for this long is closed.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
/// Longest wait for the editor to answer one request.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(60);

/// One request for the editor to answer on its UI thread.
pub struct ControlRequest {
    pub method: String,
    pub params: Value,
    reply: SyncSender<Result<Value, String>>,
}

impl ControlRequest {
    pub fn respond(self, result: Result<Value, String>) {
        // The connection may have closed while the editor worked; nothing to do then.
        let _ = self.reply.send(result);
    }
}

/// A running control channel. The editor drains [`ControlServer::pending`] every frame.
pub struct ControlServer {
    requests: Receiver<ControlRequest>,
    addr: SocketAddr,
}

impl ControlServer {
    /// Listens on `127.0.0.1:port` (0 picks a free port). `wake` is called when a request arrives,
    /// so an idle editor redraws and answers it.
    pub fn start(port: u16, token: String, wake: impl Fn() + Send + Sync + 'static) -> std::io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
        let addr = listener.local_addr()?;
        let (sender, requests) = channel();
        let token: Arc<str> = token.into();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let open = Arc::new(AtomicUsize::new(0));
        std::thread::Builder::new().name("bayleaf control".into()).spawn(move || {
            for stream in listener.incoming().flatten() {
                if open.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
                    let _ = writeln!(&stream, "{}", json!({ "id": null, "ok": false, "error": "too many connections" }));
                    continue;
                }
                open.fetch_add(1, Ordering::SeqCst);
                let (token, sender, wake, open) = (Arc::clone(&token), sender.clone(), Arc::clone(&wake), Arc::clone(&open));
                let _ = std::thread::Builder::new().name("bayleaf control connection".into()).spawn(move || {
                    let _ = serve_connection(stream, &token, &sender, &*wake);
                    open.fetch_sub(1, Ordering::SeqCst);
                });
            }
        })?;
        Ok(Self { requests, addr })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Requests waiting for an answer.
    pub fn pending(&self) -> impl Iterator<Item = ControlRequest> + '_ {
        self.requests.try_iter()
    }

    /// Answers every waiting request with `handler`: `engine.commands` and `engine.execute`.
    /// Returns how many were answered.
    pub fn answer(&self, handler: &mut dyn Handler) -> usize {
        let requests: Vec<ControlRequest> = self.pending().collect();
        let count = requests.len();
        for request in requests {
            let result = match request.method.as_str() {
                "engine.commands" => Ok(handler.commands()),
                "engine.execute" => match request.params.get("command").and_then(Value::as_str) {
                    Some(command) => {
                        let params = request.params.get("params").cloned().unwrap_or_else(|| json!({}));
                        handler.execute(command, &params)
                    }
                    None => Err("engine.execute needs params.command".to_owned()),
                },
                other => Err(format!("unknown method {other}; use engine.commands or engine.execute")),
            };
            request.respond(result);
        }
        count
    }
}

/// What answers control requests: the editor, on its UI thread.
pub trait Handler {
    /// The command list, as `engine.commands` returns it.
    fn commands(&mut self) -> Value;
    /// Runs one command for an agent, inside the agent's allowed folders.
    fn execute(&mut self, command: &str, params: &Value) -> Result<Value, String>;
}

fn serve_connection(stream: TcpStream, token: &str, sender: &Sender<ControlRequest>, wake: &(dyn Fn() + Send + Sync)) -> std::io::Result<()> {
    stream.set_read_timeout(Some(IDLE_TIMEOUT))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let mut authenticated = false;
    loop {
        let Some(line) = read_line(&mut reader)? else { return Ok(()) };
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                writeln!(writer, "{}", json!({ "id": null, "ok": false, "error": format!("not JSON: {e}") }))?;
                continue;
            }
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request.get("method").and_then(Value::as_str).unwrap_or_default().to_owned();
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        if !authenticated {
            let given = params.get("token").and_then(Value::as_str).unwrap_or_default();
            if method == "auth" && constant_time_eq(given.as_bytes(), token.as_bytes()) {
                authenticated = true;
                writeln!(writer, "{}", json!({ "id": id, "ok": true, "result": {} }))?;
                continue;
            }
            writeln!(writer, "{}", json!({ "id": id, "ok": false, "error": "authentication required" }))?;
            return Ok(());
        }
        if method == "auth" {
            writeln!(writer, "{}", json!({ "id": id, "ok": false, "error": "already authenticated" }))?;
            continue;
        }
        let (reply, answer) = sync_channel(1);
        if sender.send(ControlRequest { method, params, reply }).is_err() {
            writeln!(writer, "{}", json!({ "id": id, "ok": false, "error": "the editor is closing" }))?;
            return Ok(());
        }
        wake();
        let line = match answer.recv_timeout(REPLY_TIMEOUT) {
            Ok(Ok(result)) => json!({ "id": id, "ok": true, "result": result }),
            Ok(Err(error)) => json!({ "id": id, "ok": false, "error": error }),
            // The command may still run; the client should check before sending it again.
            Err(_) => json!({ "id": id, "ok": false, "error": format!("timeout: no reply in {}s", REPLY_TIMEOUT.as_secs()) }),
        };
        writeln!(writer, "{line}")?;
    }
}

/// One line of at most [`MAX_LINE`] bytes, without the newline; `None` at the end of the stream.
fn read_line(reader: &mut impl BufRead) -> std::io::Result<Option<String>> {
    let mut buffer = Vec::new();
    let read = reader.by_ref().take(MAX_LINE as u64 + 1).read_until(b'\n', &mut buffer)?;
    if read == 0 {
        return Ok(None);
    }
    if buffer.len() > MAX_LINE {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "request line too long"));
    }
    while buffer.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
        buffer.pop();
    }
    String::from_utf8(buffer).map(Some).map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "request is not UTF-8"))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The token in `path`, creating the file with a new random 256-bit token if it does not exist.
pub fn load_or_create_token(path: &Path) -> std::io::Result<String> {
    if let Ok(text) = std::fs::read_to_string(path) {
        let token = text.trim().to_owned();
        if token.len() >= 32 {
            // A file from an older version may still be readable by others.
            restrict_to_owner(path)?;
            return Ok(token);
        }
    }
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &token)?;
    restrict_to_owner(path)?;
    Ok(token)
}

#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

/// On Windows: drops the rules the file inherits from its folder and gives only the current user
/// access, with the system's `icacls` tool.
#[cfg(windows)]
fn restrict_to_owner(path: &Path) -> std::io::Result<()> {
    let user = std::env::var("USERNAME").map_err(|_| std::io::Error::other("USERNAME is not set, so the token file cannot be limited to you"))?;
    let account = match std::env::var("USERDOMAIN") {
        Ok(domain) if !domain.is_empty() => format!("{domain}\\{user}"),
        _ => user,
    };
    let status = std::process::Command::new("icacls")
        .arg(path)
        .args(["/inheritance:r", "/grant:r"])
        .arg(format!("{account}:F"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        // A token others may read is not kept.
        let _ = std::fs::remove_file(path);
        Err(std::io::Error::other(format!("icacls could not limit {} to {account}", path.display())))
    }
}

/// Elsewhere the file keeps its folder's access rules; keep it in a folder only you can read.
#[cfg(not(any(unix, windows)))]
fn restrict_to_owner(_: &Path) -> std::io::Result<()> {
    Ok(())
}

/// A client of a running editor's control channel.
#[derive(Debug)]
pub struct ControlClient {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
    next_id: u64,
}

impl ControlClient {
    /// Connects to `addr` (loopback only) and authenticates with `token`.
    pub fn connect(addr: SocketAddr, token: &str) -> Result<Self, String> {
        if !addr.ip().is_loopback() {
            return Err(format!("the control channel only listens on loopback addresses, not {addr}"));
        }
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map_err(|e| format!("cannot reach the editor at {addr}: {e}"))?;
        stream.set_read_timeout(Some(REPLY_TIMEOUT + Duration::from_secs(5))).map_err(|e| e.to_string())?;
        let reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        let mut client = Self { writer: stream, reader, next_id: 1 };
        client.call("auth", &json!({ "token": token }))?;
        Ok(client)
    }

    /// Sends one request and waits for its reply.
    pub fn call(&mut self, method: &str, params: &Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(self.writer, "{}", json!({ "id": id, "method": method, "params": params })).map_err(|e| format!("control channel: {e}"))?;
        let line = read_line(&mut self.reader).map_err(|e| format!("control channel: {e}"))?.ok_or("the editor closed the control channel")?;
        let reply: Value = serde_json::from_str(&line).map_err(|e| format!("control channel sent invalid JSON: {e}"))?;
        if reply.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(reply.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err(reply.get("error").and_then(Value::as_str).unwrap_or("unknown error").to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ControlClient, ControlServer, load_or_create_token};
    use serde_json::{Value, json};

    /// A server whose requests a background thread answers like the editor would.
    fn server(token: &str) -> std::net::SocketAddr {
        let server = ControlServer::start(0, token.to_owned(), || {}).unwrap();
        let addr = server.addr();
        std::thread::spawn(move || {
            loop {
                for request in server.pending() {
                    let result = match request.method.as_str() {
                        "engine.commands" => Ok(json!([{ "id": "app.version" }])),
                        "engine.execute" => Ok(json!({ "ran": request.params["command"] })),
                        other => Err(format!("unknown method {other}")),
                    };
                    request.respond(result);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        });
        addr
    }

    #[test]
    fn a_wrong_token_is_refused_and_the_right_one_works() {
        let addr = server("secret-token-of-enough-length-0123456789");
        assert!(ControlClient::connect(addr, "guess").unwrap_err().contains("authentication required"));
        let mut client = ControlClient::connect(addr, "secret-token-of-enough-length-0123456789").unwrap();
        assert_eq!(client.call("engine.commands", &Value::Null).unwrap()[0]["id"], "app.version");
        assert_eq!(client.call("engine.execute", &json!({ "command": "app.version" })).unwrap()["ran"], "app.version");
        assert!(client.call("auth", &json!({ "token": "x" })).is_err(), "no second auth");
        assert!(client.call("format.disk", &Value::Null).is_err());
    }

    #[test]
    fn only_loopback_addresses_are_accepted_by_the_client() {
        let remote: std::net::SocketAddr = "192.0.2.1:7878".parse().unwrap();
        assert!(ControlClient::connect(remote, "x").unwrap_err().contains("loopback"));
    }

    #[test]
    fn tokens_are_created_once_and_then_reused() {
        let path = std::env::temp_dir().join(format!("bayleaf-token-{}", std::process::id())).join("control.token");
        let first = load_or_create_token(&path).unwrap();
        assert_eq!(first.len(), 64);
        // On Windows only the current user is left on the file's access list.
        #[cfg(windows)]
        {
            let listing = std::process::Command::new("icacls").arg(&path).output().unwrap();
            let text = String::from_utf8_lossy(&listing.stdout);
            assert_eq!(text.matches(":(").count(), 1, "one access rule: {text}");
        }
        assert_eq!(load_or_create_token(&path).unwrap(), first);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}
