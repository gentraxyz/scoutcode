//! The Agent Client Protocol, for CLIs that loop over the model and run their own tools: JSON-RPC over stdio. Kiro, Codex,
//! Claude Code, OpenCode and Grok Build run this way. Scout shows what they do and answers their permission requests, but their edits are outside
//! Scout's undo. Each turn is one process, so the conversation always comes from Scout's messages and compaction and
//! switching provider have nothing to keep in step.

use crate::tools::{Todo, kill_tree, no_window};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Downloads a CLI's package into `root`/`version`. The download must match `want`, and the folder only appears once it's
/// fully unpacked. Older versions are deleted afterwards.
pub fn install(name: &str, url: &str, want: &str, root: &Path, version: &str, unpack: &dyn Fn(&Path, &Path) -> std::io::Result<()>, progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
    let res = crate::api::http().get(url).call().map_err(|e| format!("Couldn't download {name}: {e}"))?;
    if res.status().as_u16() != 200 {
        return Err(format!("Couldn't download {name}: HTTP {}", res.status().as_u16()));
    }
    let total = res.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(0);

    let (tmp, file_path) = (root.join(format!("{version}.partial")), root.join(format!("{version}.download")));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let mut file = std::fs::File::create(&file_path).map_err(|e| e.to_string())?;
    let (mut body, mut hash, mut buf, mut done) = (res.into_body().into_reader(), Sha256::new(), vec![0u8; 256 * 1024], 0u64);
    loop {
        let n = body.read(&mut buf).map_err(|e| format!("The {name} download stopped: {e}"))?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        progress(done, total);
    }
    drop(file);
    let finish = (|| {
        if hash.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>() != want {
            return Err(format!("The {name} download didn't match its expected hash, so it wasn't installed."));
        }
        unpack(&file_path, &tmp).map_err(|e| format!("Couldn't unpack {name}: {e}"))?;
        let _ = std::fs::remove_dir_all(root.join(version));
        std::fs::rename(&tmp, root.join(version)).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_file(&file_path);
    if finish.is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    finish?;
    for e in std::fs::read_dir(root).into_iter().flatten().flatten() {
        if e.file_name() != version {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    Ok(())
}

/// Whether the answer to `initialize` says the CLI takes images in a prompt.
pub fn takes_images(hello: &Value) -> bool {
    hello.pointer("/agentCapabilities/promptCapabilities/image").and_then(Value::as_bool).unwrap_or(false)
}

/// A prompt's content: the text, then the images as (media type, base64) when the CLI takes them.
pub fn prompt_blocks(text: String, images: &[(String, String)]) -> Value {
    let mut blocks = vec![json!({"type": "text", "text": text})];
    blocks.extend(images.iter().map(|(mime, data)| json!({"type": "image", "mimeType": mime, "data": data})));
    Value::Array(blocks)
}

/// What a run tells the agent's thread. Permission waits for a reply through the run.
pub enum Msg {
    Text(String),
    Reasoning(String),
    ToolStart { id: String, kind: String, title: String, input: Value },
    ToolEnd { id: String, failed: bool, text: String },
    Todos(Vec<Todo>),
    Permission { id: Value, kind: String, title: String, options: Vec<(String, String)> },
    /// The prompt is finished, and why.
    Idle(String),
    Failed(String),
}

/// What a tool call has said so far: its text, or failing that its raw output.
fn tool_output(u: &Value) -> String {
    let parts = u.get("content").and_then(Value::as_array).map_or(vec![], |a| a.iter().filter_map(|c| c.pointer("/content/text").and_then(Value::as_str)).collect::<Vec<_>>());
    if !parts.is_empty() {
        return parts.join("\n");
    }
    match u.get("rawOutput") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(v) => v.to_string(),
    }
}

/// What a session/update means for Scout: nothing for most, and both ends of a tool call that arrives finished.
pub fn updates_of(u: &Value) -> Vec<Msg> {
    let text = |v: &Value| v.pointer("/content/text").and_then(Value::as_str).unwrap_or("").to_string();
    let s = |k: &str| u.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let end = |u: &Value| {
        let status = u.get("status").and_then(Value::as_str).unwrap_or("");
        (status == "completed" || status == "failed").then(|| Msg::ToolEnd { id: u.get("toolCallId").and_then(Value::as_str).unwrap_or("").to_string(), failed: status == "failed", text: tool_output(u) })
    };
    match u.get("sessionUpdate").and_then(Value::as_str).unwrap_or("") {
        "agent_message_chunk" => vec![Msg::Text(text(u))],
        "agent_thought_chunk" => vec![Msg::Reasoning(text(u))],
        "tool_call" => std::iter::once(Msg::ToolStart { id: s("toolCallId"), kind: s("kind"), title: s("title"), input: u.get("rawInput").cloned().unwrap_or(json!({})) }).chain(end(u)).collect(),
        "tool_call_update" => end(u).into_iter().collect(),
        "plan" => {
            let list = u.get("entries").and_then(Value::as_array).map_or(&[][..], |a| a.as_slice());
            vec![Msg::Todos(list.iter().map(|e| Todo { content: e.get("content").and_then(Value::as_str).unwrap_or("").to_string(), status: e.get("status").and_then(Value::as_str).unwrap_or("pending").to_string() }).collect())]
        }
        _ => vec![],
    }
}

pub struct Conn {
    name: &'static str,
    stdin: Mutex<ChildStdin>,
    next: AtomicU64,
    waiting: Mutex<HashMap<u64, Sender<Result<Value, String>>>>,
}

impl Conn {
    fn write(&self, m: &Value) {
        let mut stdin = self.stdin.lock().unwrap();
        let _ = writeln!(stdin, "{m}").and_then(|_| stdin.flush());
    }

    fn ask(&self, method: &str, params: Value) -> Receiver<Result<Value, String>> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = channel();
        self.waiting.lock().unwrap().insert(id, tx);
        self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        rx
    }

    pub fn call(&self, method: &str, params: Value, wait: Duration) -> Result<Value, String> {
        match self.ask(method, params).recv_timeout(wait) {
            Ok(r) => r,
            Err(RecvTimeoutError::Timeout) => Err(format!("{} didn't answer {method} in time.", self.name)),
            Err(RecvTimeoutError::Disconnected) => Err(format!("The {} CLI stopped unexpectedly.", self.name)),
        }
    }

    fn reply(&self, id: &Value, result: Value) {
        self.write(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }
}

/// What a JSON-RPC error says. Its details are often the only useful part, as with Grok's "Internal error".
fn error_text(e: &Value, name: &str) -> String {
    let Some(message) = e.get("message").and_then(Value::as_str) else { return format!("{name} refused the request.") };
    match e.get("data").and_then(Value::as_str) {
        Some(data) => format!("{message}: {data}"),
        None => message.to_string(),
    }
}

/// Handles a request from the CLI. Only permissions need the agent. Any other must get an answer, or the CLI waits for it.
fn request(method: &str, id: Value, p: &Value, conn: &Conn, tx: &Sender<Msg>) {
    if method != "session/request_permission" {
        return conn.write(&json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Scout doesn't support this call."}}));
    }
    let call = |k: &str| p.pointer(&format!("/toolCall/{k}")).and_then(Value::as_str).unwrap_or("").to_string();
    let options = p.get("options").and_then(Value::as_array).map(|a| a.iter().filter_map(|o| Some((o.get("optionId")?.as_str()?.to_string(), o.get("kind")?.as_str()?.to_string()))).collect()).unwrap_or_default();
    let _ = tx.send(Msg::Permission { id, kind: call("kind"), title: call("title"), options });
}

/// The newest settings the CLI has sent for a session, as (session, configOptions).
pub type Latest = Arc<Mutex<Option<(String, Value)>>>;

/// The CLI running as an ACP server, after its handshake.
pub struct Server {
    name: &'static str,
    child: Child,
    pub conn: Arc<Conn>,
    pub latest: Latest,
    /// It said it takes images in a prompt.
    images: bool,
}

impl Server {
    /// Starts `c`, which only needs its program, arguments, environment and folder set.
    pub fn open(mut c: Command, name: &'static str, tx: Sender<Msg>) -> Result<Server, String> {
        c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        no_window(&mut c);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            c.process_group(0);
        }
        let mut child = c.spawn().map_err(|e| format!("Couldn't start {name}: {e}"))?;
        let (stdin, stdout) = (child.stdin.take().ok_or(format!("{name} has no input."))?, child.stdout.take().ok_or(format!("{name} has no output."))?);
        let conn = Arc::new(Conn { name, stdin: Mutex::new(stdin), next: AtomicU64::new(1), waiting: Default::default() });
        let reader_conn = conn.clone();
        let latest: Latest = Default::default();
        let reader_latest = latest.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(m) = serde_json::from_str::<Value>(&line) else { continue };
                let method = m.get("method").and_then(Value::as_str);
                match (method, m.get("id")) {
                    (None, Some(id)) => {
                        let Some(waiter) = id.as_u64().and_then(|id| reader_conn.waiting.lock().unwrap().remove(&id)) else { continue };
                        let answer = match m.get("error") {
                            Some(e) => Err(error_text(e, name)),
                            None => Ok(m.get("result").cloned().unwrap_or(Value::Null)),
                        };
                        let _ = waiter.send(answer);
                    }
                    (Some(method), Some(id)) => request(method, id.clone(), m.get("params").unwrap_or(&Value::Null), &reader_conn, &tx),
                    (Some("session/update"), None) => {
                        // Claude Code's adapter passes its plan's limits along with each answer.
                        if let Some(info) = m.pointer("/params/update/_meta/_claude~1rateLimit") {
                            crate::quota::record_claude_rate_limit(info);
                        }
                        if let (Some("config_option_update"), Some(session), Some(options)) = (m.pointer("/params/update/sessionUpdate").and_then(Value::as_str), m.pointer("/params/sessionId").and_then(Value::as_str), m.pointer("/params/update/configOptions")) {
                            *reader_latest.lock().unwrap() = Some((session.to_string(), options.clone()));
                        }
                        for msg in m.pointer("/params/update").map(updates_of).unwrap_or_default() {
                            let _ = tx.send(msg);
                        }
                    }
                    _ => {}
                }
            }
            // Whoever is still waiting hears it stopped.
            reader_conn.waiting.lock().unwrap().clear();
        });
        let mut server = Server { name, child, conn, latest, images: false };
        // No file or terminal calls are offered, so the CLI runs its own tools.
        let hello = json!({"protocolVersion": 1, "clientCapabilities": {}, "clientInfo": {"name": "scout-desktop", "version": env!("CARGO_PKG_VERSION")}});
        match server.conn.call("initialize", hello, Duration::from_secs(30)) {
            Ok(r) => {
                server.images = takes_images(&r);
                Ok(server)
            }
            Err(e) => {
                server.stop();
                Err(e)
            }
        }
    }

    pub fn new_session(&self, cwd: &str) -> Result<Value, String> {
        self.new_session_with(cwd, Value::Null)
    }

    /// A new session, with `meta` for the CLI's own extensions when it isn't null.
    pub fn new_session_with(&self, cwd: &str, meta: Value) -> Result<Value, String> {
        let mut params = json!({"cwd": cwd, "mcpServers": []});
        if !meta.is_null() {
            params["_meta"] = meta;
        }
        self.conn.call("session/new", params, Duration::from_secs(60))
    }

    pub fn delete_session(&self, session: &Value) {
        if let Some(id) = session.get("sessionId") {
            let _ = self.conn.call("session/delete", json!({"sessionId": id}), Duration::from_secs(3));
        }
    }

    pub fn stop(&mut self) {
        kill_tree(&mut self.child);
        let _ = self.child.wait();
    }
}

/// The choices a session offers for a config option, as (value, name).
pub fn options_of(session: &Value, option: &str) -> Vec<(String, String)> {
    let Some(options) = session.get("configOptions").and_then(Value::as_array) else { return vec![] };
    let found = options.iter().find(|o| o.get("id").and_then(Value::as_str) == Some(option));
    found.and_then(|m| m.get("options")).and_then(Value::as_array).map_or(vec![], |list| list.iter().filter_map(|o| Some((o.get("value")?.as_str()?.to_string(), o.get("name")?.as_str()?.to_string()))).collect())
}

/// The ids of the modes a session offers.
pub fn modes_of(session: &Value) -> Vec<String> {
    session.pointer("/modes/availableModes").and_then(Value::as_array).map_or(vec![], |list| list.iter().filter_map(|m| Some(m.get("id")?.as_str()?.to_string())).collect())
}

/// One session, on its own process: started for a turn and gone after it.
pub struct Run {
    rx: Receiver<Msg>,
    tx: Sender<Msg>,
    server: Server,
    session: Value,
}

impl Run {
    /// Takes over a server with a session set up, and sends `prompt`. `rx` is the other end of the sender it was opened with.
    pub fn begin(server: Server, session: Value, tx: Sender<Msg>, rx: Receiver<Msg>, prompt: String, images: Vec<(String, String)>) -> Run {
        let images = if server.images { images } else { vec![] };
        let run = Run { rx, tx, server, session };
        run.send_prompt(prompt_blocks(prompt, &images));
        run
    }

    pub fn next(&self, wait: Duration) -> Option<Msg> {
        self.rx.recv_timeout(wait).ok()
    }

    /// Sends a prompt; Msg::Idle says when it's done. Only one runs at a time, and the agent waits for that.
    pub fn send(&self, text: String) {
        self.send_prompt(prompt_blocks(text, &[]));
    }

    fn send_prompt(&self, prompt: Value) {
        let answer = self.server.conn.ask("session/prompt", json!({"sessionId": self.session, "prompt": prompt}));
        let tx = self.tx.clone();
        let name = self.server.name;
        std::thread::spawn(move || {
            let _ = tx.send(match answer.recv() {
                Ok(Ok(r)) => Msg::Idle(r.get("stopReason").and_then(Value::as_str).unwrap_or("end_turn").to_string()),
                Ok(Err(e)) => Msg::Failed(e),
                Err(_) => Msg::Failed(format!("The {name} CLI stopped unexpectedly.")),
            });
        });
    }

    pub fn abort(&self) {
        self.server.conn.write(&json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": self.session}}));
    }

    /// Answers a permission request for this once. The "always" options want a rule sent with them, and Scout's own mode
    /// already decides what's asked.
    pub fn reply_permission(&self, id: &Value, allow: bool, options: &[(String, String)]) {
        self.server.conn.reply(id, json!({"outcome": permission_outcome(allow, options)}));
    }

    /// Removes the session from the CLI's own history where it can, and stops the process. Returns the session's id, for
    /// a CLI that can only forget it from outside.
    pub fn finish(mut self) -> String {
        let _ = self.server.conn.call("session/delete", json!({"sessionId": self.session}), Duration::from_secs(3));
        self.server.stop();
        self.session.as_str().unwrap_or("").to_string()
    }
}

/// The option to pick: only ever a one-time one. With none of those, the request is cancelled, which refuses it.
pub fn permission_outcome(allow: bool, options: &[(String, String)]) -> Value {
    let want = if allow { "allow_once" } else { "reject_once" };
    match options.iter().find(|o| o.1 == want) {
        Some(o) => json!({"outcome": "selected", "optionId": o.0}),
        None => json!({"outcome": "cancelled"}),
    }
}

/// A channel for a run's messages.
pub fn messages() -> (Sender<Msg>, Receiver<Msg>) {
    channel()
}

/// Starts `c`, writes `lines` to it one JSON value per line, and gives back the first line it prints that `want` picks.
/// For a one-off question to a CLI that isn't an ACP server, such as Codex's app server. It's stopped afterwards.
pub fn exchange(mut c: Command, name: &str, lines: &[Value], want: impl Fn(&Value) -> bool, wait: Duration) -> Result<Value, String> {
    c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    no_window(&mut c);
    let mut child = c.spawn().map_err(|e| format!("Couldn't start {name}: {e}"))?;
    let stdout = child.stdout.take().ok_or(format!("{name} has no output."))?;
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                if tx.send(v).is_err() {
                    break;
                }
            }
        }
    });
    // Kept open until the end: some stop as soon as their input ends, before they answer.
    let mut stdin = child.stdin.take();
    if let Some(s) = stdin.as_mut() {
        for l in lines {
            let _ = writeln!(s, "{l}");
        }
        let _ = s.flush();
    }
    let until = std::time::Instant::now() + wait;
    let answer = loop {
        match rx.recv_timeout(until.saturating_duration_since(std::time::Instant::now())) {
            Ok(v) if want(&v) => break Ok(v),
            Ok(_) => continue,
            Err(RecvTimeoutError::Timeout) => break Err(format!("{name} didn't answer.")),
            Err(RecvTimeoutError::Disconnected) => break Err(format!("{name} stopped without answering.")),
        }
    };
    kill_tree(&mut child);
    drop(stdin);
    let _ = child.wait();
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_are_understood() {
        assert!(matches!(&updates_of(&json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "hi"}}))[..], [Msg::Text(t)] if t == "hi"));
        assert!(updates_of(&json!({"sessionUpdate": "session_info_update", "_meta": {"kiro": {"kind": "context_usage"}}})).is_empty());
        let call = json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "Read a", "kind": "read", "status": "pending"});
        assert!(matches!(&updates_of(&call)[..], [Msg::ToolStart { kind, .. }] if kind == "read"));
        let done_at_once = json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "Read a", "status": "completed", "rawOutput": "text"});
        assert!(matches!(&updates_of(&done_at_once)[..], [Msg::ToolStart { .. }, Msg::ToolEnd { failed: false, text, .. }] if text == "text"));
        assert!(updates_of(&json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "in_progress"})).is_empty());
        let end = json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "failed", "content": [{"type": "content", "content": {"type": "text", "text": "no"}}]});
        assert!(matches!(&updates_of(&end)[..], [Msg::ToolEnd { failed: true, text, .. }] if text == "no"));
    }

    #[test]
    fn errors_keep_their_details() {
        assert_eq!(error_text(&json!({"code": -32000, "message": "Authentication required", "data": "no auth method id provided"}), "X"), "Authentication required: no auth method id provided");
        assert_eq!(error_text(&json!({"message": "no"}), "X"), "no");
        assert_eq!(error_text(&json!({"code": 1}), "X"), "X refused the request.");
    }

    /// The shape the CLIs really send, with models that only show up once signed in.
    #[test]
    fn only_offered_settings_are_read() {
        let session = json!({"configOptions": [{"type": "select", "id": "autopilot", "currentValue": "on", "options": [{"value": "on", "name": "Autopilot"}, {"value": "off", "name": "Supervised"}]}]});
        assert_eq!(options_of(&session, "autopilot")[1], ("off".to_string(), "Supervised".to_string()));
        assert!(options_of(&session, "model").is_empty());
        assert!(options_of(&json!({}), "model").is_empty());
        let session = json!({"modes": {"currentModeId": "default", "availableModes": [{"id": "default", "name": "Default"}, {"id": "plan", "name": "Plan"}]}});
        assert_eq!(modes_of(&session), ["default", "plan"]);
        assert!(modes_of(&json!({})).is_empty());
    }

    #[test]
    fn permissions_are_only_ever_given_once() {
        let options = [("a".to_string(), "allow_always".to_string()), ("b".to_string(), "allow_once".to_string()), ("c".to_string(), "reject_once".to_string())];
        assert_eq!(permission_outcome(true, &options), json!({"outcome": "selected", "optionId": "b"}));
        assert_eq!(permission_outcome(false, &options), json!({"outcome": "selected", "optionId": "c"}));
        assert_eq!(permission_outcome(true, &options[..1]), json!({"outcome": "cancelled"}));
    }
}
