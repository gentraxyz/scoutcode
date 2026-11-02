//! Devin by Cognition, on the user's own account. Its CLI runs here as an Agent Client Protocol server: JSON-RPC over
//! stdio. It loops over the model itself and sends its reads, writes and commands back to Scout, which runs them with
//! its own permissions and undo. Its searches stay inside it, since ACP has no call for them. The CLI is downloaded at
//! sign-in and checked against the hashes below, and only ever runs on this PC. Scout never sees the account's token:
//! the CLI keeps it, in a place that can't be moved on Windows, so the sign-in is shared with any other Devin CLI here.

use crate::tools::{Todo, kill_tree, no_window};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The CLI's version. It only changes with a Scout release: see "Devin" in README.md.
const VERSION: &str = "3000.11.234";
/// The SHA-256 of each platform's archive at that version. The registry publishes none, so these are our own.
const ARCHIVES: [(&str, &str); 6] = [
    ("x86_64-pc-windows.zip", "de01a37364c3c09389e20b699e993962296e5b8e1bea60aa48c95cc57dd228e3"),
    ("aarch64-pc-windows.zip", "1fc2fcedfe0649a9ed370b122064d66f406c9a2e4a7622b4c6712cc098c01b22"),
    ("x86_64-apple-darwin.tar.gz", "55ef4f2d2b2c82b5258b7516eb1126549e6f10d6b418035676237bcc18e93e46"),
    ("aarch64-apple-darwin.tar.gz", "35f17a21e53a49435e42fb9627731726aa8ef7fa63e5c3d9d43d0cc5ca43edaf"),
    ("x86_64-unknown-linux.tar.gz", "201370cddd2924294e3ca6b46fd8ece49b5763953b644a1dd823fce5490cb41a"),
    ("aarch64-unknown-linux.tar.gz", "438ca7f8280b3ef0747afc406e827d9e182156000277a1660358ac4fbd04156e"),
];
pub const PLAN_URL: &str = "https://devin.ai/pricing";

/// How signing in is going, for the window.
pub enum Update {
    Progress { done: u64, total: u64 },
    /// The CLI is there, and the browser should be open.
    Waiting,
    Done,
    Failed(String),
}

/// The archive for this platform. Devin has no build for musl Linux.
fn archive() -> Result<(&'static str, &'static str), String> {
    let arch = if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" };
    let target = if cfg!(windows) {
        format!("{arch}-pc-windows.zip")
    } else if cfg!(target_os = "macos") {
        format!("{arch}-apple-darwin.tar.gz")
    } else if cfg!(target_env = "musl") {
        return Err("Devin isn't available for this Linux (musl).".into());
    } else {
        format!("{arch}-unknown-linux.tar.gz")
    };
    ARCHIVES.iter().find(|a| a.0 == target).copied().ok_or_else(|| format!("Devin isn't available for {target}."))
}

fn root() -> PathBuf {
    crate::config::home().join("runtime").join("devin")
}

/// Where the CLI lives. A folder only gets this name once it's complete, so one that exists can be used.
fn install_dir() -> PathBuf {
    root().join(VERSION)
}

fn exe() -> PathBuf {
    install_dir().join("bin").join(if cfg!(windows) { "devin.exe" } else { "devin" })
}

pub fn installed() -> bool {
    exe().is_file()
}

/// Deletes the CLI.
pub fn remove_runtime() {
    let _ = std::fs::remove_dir_all(root());
}

/// Downloads the CLI unless it's there. The download must match the built-in hash, and the folder only appears once
/// it's fully unpacked. Older versions are deleted afterwards.
pub fn ensure_runtime(progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
    static INSTALLING: Mutex<()> = Mutex::new(());
    let _one_at_a_time = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    if installed() {
        return Ok(());
    }
    let (name, want) = archive()?;
    let url = format!("https://static.devin.ai/cli/{VERSION}/devin-{VERSION}-{name}");
    let res = crate::api::http().get(&url).call().map_err(|e| format!("Couldn't download Devin: {e}"))?;
    if res.status().as_u16() != 200 {
        return Err(format!("Couldn't download Devin: HTTP {}", res.status().as_u16()));
    }
    let total = res.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(0);

    let (tmp, file_path) = (root().join(format!("{VERSION}.partial")), root().join(format!("{VERSION}.download")));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(root()).map_err(|e| e.to_string())?;
    let mut file = std::fs::File::create(&file_path).map_err(|e| e.to_string())?;
    let (mut body, mut hash, mut buf, mut done) = (res.into_body().into_reader(), Sha256::new(), vec![0u8; 256 * 1024], 0u64);
    loop {
        let n = body.read(&mut buf).map_err(|e| format!("The Devin download stopped: {e}"))?;
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
            return Err("The Devin download didn't match its expected hash, so it wasn't installed.".to_string());
        }
        unpack(&file_path, name, &tmp).map_err(|e| format!("Couldn't unpack Devin: {e}"))?;
        let _ = std::fs::remove_dir_all(install_dir());
        std::fs::rename(&tmp, install_dir()).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_file(&file_path);
    if finish.is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    finish?;
    for e in std::fs::read_dir(root()).into_iter().flatten().flatten() {
        if e.file_name() != VERSION {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    Ok(())
}

/// Unpacks a zip or a tar.gz. `enclosed_name` and `unpack_in` refuse paths that leave the folder.
pub fn unpack(archive: &Path, name: &str, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    let file = std::fs::File::open(archive)?;
    if name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(file).map_err(std::io::Error::other)?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(std::io::Error::other)?;
            let Some(rel) = entry.enclosed_name() else { continue };
            let path = to.join(rel);
            if entry.is_dir() {
                std::fs::create_dir_all(&path)?;
            } else {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::io::copy(&mut entry, &mut std::fs::File::create(&path)?)?;
            }
        }
    } else {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
        for entry in tar.entries()? {
            entry?.unpack_in(to)?;
        }
    }
    Ok(())
}

// ---------- signing in ----------

fn cli(args: &[&str]) -> Command {
    let mut c = Command::new(exe());
    c.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    no_window(&mut c);
    c
}

pub fn signed_in() -> bool {
    cli(&["auth", "status"]).output().is_ok_and(|o| {
        let text = String::from_utf8_lossy(&o.stdout);
        text.contains("Logged in") && !text.contains("Not logged in")
    })
}

/// Signs the CLI out, here and for any other copy on this PC: they share the sign-in.
pub fn sign_out() {
    if installed() {
        let _ = cli(&["auth", "logout"]).output();
    }
}

/// Downloads the CLI, then has it open the browser, unless it's signed in already.
pub fn sign_in(cancel: Arc<AtomicBool>, send: impl Fn(Update) + Send + 'static) {
    std::thread::spawn(move || {
        let run = || -> Result<(), String> {
            ensure_runtime(&mut |done, total| send(Update::Progress { done, total }))?;
            if cancel.load(Ordering::Relaxed) {
                return Err(String::new());
            }
            send(Update::Waiting);
            if signed_in() {
                return Ok(());
            }
            let (tx, _rx) = channel();
            let mut server = Server::open(&data_dir()?, &["acp"], tx)?;
            let answer = server.conn.ask("authenticate", json!({"methodId": "devin-browser"}));
            let deadline = Instant::now() + Duration::from_secs(600);
            let result = loop {
                if cancel.load(Ordering::Relaxed) {
                    break Err(String::new());
                }
                if Instant::now() > deadline {
                    break Err("The sign-in timed out. Try again.".to_string());
                }
                match answer.recv_timeout(Duration::from_millis(100)) {
                    Ok(r) => break r.map(|_| ()),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break Err("Devin stopped during the sign-in.".to_string()),
                }
            };
            server.stop();
            result?;
            if signed_in() { Ok(()) } else { Err("Devin didn't accept the sign-in. Try again.".into()) }
        };
        match run() {
            Ok(()) => send(Update::Done),
            Err(e) => send(Update::Failed(e)),
        }
    });
}

/// Devin's own files for sessions that aren't about a project, such as listing models.
fn data_dir() -> Result<PathBuf, String> {
    let d = crate::config::home().join("runtime").join("devin-home");
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

// ---------- the protocol ----------

/// What a run tells the agent's thread. The ones with an `id` wait for a reply through the run.
pub enum Msg {
    Text(String),
    Reasoning(String),
    Usage { prompt: u64, completion: u64, cached: u64 },
    ToolStart { id: String, kind: String, title: String, input: Value },
    ToolEnd { id: String, failed: bool, text: String },
    Todos(Vec<Todo>),
    Read { id: Value, path: String, line: Option<usize>, limit: Option<usize> },
    Write { id: Value, path: String, content: String, tool: Option<String> },
    Exec { id: Value, command: String, cwd: String },
    Permission { id: Value, tool: String, is_command: bool, options: Vec<(String, String)> },
    /// The prompt is finished, and why.
    Idle(String),
    Failed(String),
}

/// One tool_call or other session/update, or None for those Scout has no use for.
fn update_of(u: &Value) -> Option<Msg> {
    let text = |v: &Value| v.pointer("/content/text").and_then(Value::as_str).unwrap_or("").to_string();
    let s = |k: &str| u.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    match u.get("sessionUpdate")?.as_str()? {
        "agent_message_chunk" => Some(Msg::Text(text(u))),
        "agent_thought_chunk" => Some(Msg::Reasoning(text(u))),
        "usage_update" => {
            let meta = u.get("_meta")?;
            // Each request is reported again for its sub-agent context; once is enough.
            if meta.get("cognition.ai/subagent_context").is_some() {
                return None;
            }
            let num = |k: &str| meta.get(k).and_then(Value::as_u64).unwrap_or(0);
            Some(Msg::Usage { prompt: num("cognition.ai/inputTokens"), completion: num("cognition.ai/outputTokens"), cached: num("cognition.ai/cachedReadTokens") })
        }
        "tool_call" => Some(Msg::ToolStart { id: s("toolCallId"), kind: s("kind"), title: s("title"), input: u.get("rawInput").cloned().unwrap_or(json!({})) }),
        "tool_call_update" => {
            let status = s("status");
            if status != "completed" && status != "failed" {
                return None;
            }
            let parts = u.get("content").and_then(Value::as_array).map(|a| a.iter().filter_map(|c| c.pointer("/content/text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"));
            Some(Msg::ToolEnd { id: s("toolCallId"), failed: status == "failed", text: parts.unwrap_or_default() })
        }
        "plan" => {
            let list = u.get("entries")?.as_array()?;
            Some(Msg::Todos(list.iter().map(|e| Todo { content: e.get("content").and_then(Value::as_str).unwrap_or("").to_string(), status: e.get("status").and_then(Value::as_str).unwrap_or("pending").to_string() }).collect()))
        }
        _ => None,
    }
}

/// The exit code Scout's shell tool put at the end of a failed command's output.
pub fn exit_code(output: &str) -> i32 {
    output.rsplit_once("[exit code ").and_then(|(_, r)| r.split(']').next()).and_then(|c| c.parse().ok()).unwrap_or(1)
}

/// Commands have finished by the time Devin hears of their terminal, so these are just what they left.
type Terminals = Arc<Mutex<HashMap<String, (String, i32)>>>;

struct Conn {
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

    fn call(&self, method: &str, params: Value, wait: Duration) -> Result<Value, String> {
        match self.ask(method, params).recv_timeout(wait) {
            Ok(r) => r,
            Err(RecvTimeoutError::Timeout) => Err(format!("Devin didn't answer {method} in time.")),
            Err(RecvTimeoutError::Disconnected) => Err("The Devin CLI stopped unexpectedly.".into()),
        }
    }

    fn reply(&self, id: &Value, result: Value) {
        self.write(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }

    fn reply_err(&self, id: &Value, code: i64, message: &str) {
        self.write(&json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}));
    }
}

/// Handles a request from Devin: the terminal calls here, and anything needing the agent goes to it.
fn request(method: &str, id: Value, p: &Value, conn: &Conn, terminals: &Terminals, tx: &Sender<Msg>) {
    let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let n = |k: &str| p.get(k).and_then(Value::as_u64).map(|v| v as usize);
    let term = || terminals.lock().unwrap().get(p.get("terminalId").and_then(Value::as_str).unwrap_or("")).cloned();
    let msg = match method {
        "fs/read_text_file" => Msg::Read { id, path: s("path"), line: n("line"), limit: n("limit") },
        "fs/write_text_file" => Msg::Write { id, path: s("path"), content: s("content"), tool: p.pointer("/_meta/cognition.ai~1toolCallId").and_then(Value::as_str).map(String::from) },
        "terminal/create" => {
            let args: Vec<&str> = p.get("args").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            Msg::Exec { id, command: std::iter::once(s("command").as_str()).chain(args).collect::<Vec<_>>().join(" "), cwd: s("cwd") }
        }
        "terminal/output" => {
            let (output, code) = term().unwrap_or_default();
            return conn.reply(&id, json!({"output": output, "truncated": false, "exitStatus": {"exitCode": code, "signal": null}}));
        }
        "terminal/wait_for_exit" => return conn.reply(&id, json!({"exitCode": term().map_or(0, |t| t.1), "signal": null})),
        "terminal/release" | "terminal/kill" => return conn.reply(&id, json!({})),
        "session/request_permission" => {
            let options = p.get("options").and_then(Value::as_array).map(|a| a.iter().filter_map(|o| Some((o.get("optionId")?.as_str()?.to_string(), o.get("kind")?.as_str()?.to_string()))).collect()).unwrap_or_default();
            Msg::Permission { id, tool: p.pointer("/toolCall/toolCallId").and_then(Value::as_str).unwrap_or("").to_string(), is_command: p.pointer("/toolCall/_meta/cognition.ai~1editableCommand").is_some(), options }
        }
        _ => return conn.reply_err(&id, -32601, "Scout doesn't support this call."),
    };
    let _ = tx.send(msg);
}

/// The CLI running as an ACP server, after its handshake.
struct Server {
    child: Child,
    conn: Arc<Conn>,
    terminals: Terminals,
    /// It said it takes images in a prompt.
    images: bool,
}

impl Server {
    fn open(cwd: &Path, args: &[&str], tx: Sender<Msg>) -> Result<Server, String> {
        let mut c = Command::new(exe());
        c.args(args).current_dir(cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        no_window(&mut c);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            c.process_group(0);
        }
        let mut child = c.spawn().map_err(|e| format!("Couldn't start Devin: {e}"))?;
        let (stdin, stdout) = (child.stdin.take().ok_or("Devin has no input.")?, child.stdout.take().ok_or("Devin has no output.")?);
        let conn = Arc::new(Conn { stdin: Mutex::new(stdin), next: AtomicU64::new(1), waiting: Default::default() });
        let terminals: Terminals = Default::default();
        let (reader_conn, reader_terms) = (conn.clone(), terminals.clone());
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(m) = serde_json::from_str::<Value>(&line) else { continue };
                let method = m.get("method").and_then(Value::as_str);
                match (method, m.get("id")) {
                    (None, Some(id)) => {
                        let Some(waiter) = id.as_u64().and_then(|id| reader_conn.waiting.lock().unwrap().remove(&id)) else { continue };
                        let answer = match m.get("error") {
                            Some(e) => Err(e.get("message").and_then(Value::as_str).unwrap_or("Devin refused the request.").to_string()),
                            None => Ok(m.get("result").cloned().unwrap_or(Value::Null)),
                        };
                        let _ = waiter.send(answer);
                    }
                    (Some(method), Some(id)) => request(method, id.clone(), m.get("params").unwrap_or(&Value::Null), &reader_conn, &reader_terms, &tx),
                    (Some("session/update"), None) => {
                        if let Some(msg) = m.pointer("/params/update").and_then(update_of) {
                            let _ = tx.send(msg);
                        }
                    }
                    _ => {}
                }
            }
            // Whoever is still waiting hears it stopped.
            reader_conn.waiting.lock().unwrap().clear();
        });
        let mut server = Server { child, conn, terminals, images: false };
        // Offering files and a terminal is what makes Devin send its reads, writes and commands here.
        let hello = json!({"protocolVersion": 1, "clientCapabilities": {"fs": {"readTextFile": true, "writeTextFile": true}, "terminal": true}, "clientInfo": {"name": "scout-desktop", "version": env!("CARGO_PKG_VERSION")}});
        match server.conn.call("initialize", hello, Duration::from_secs(30)) {
            Ok(r) => {
                server.images = crate::acp::takes_images(&r);
                Ok(server)
            }
            Err(e) => {
                server.stop();
                Err(e)
            }
        }
    }

    fn stop(&mut self) {
        kill_tree(&mut self.child);
        let _ = self.child.wait();
    }
}

/// The session's model list.
fn models_of(session: &Value) -> Vec<(String, String)> {
    let Some(options) = session.get("configOptions").and_then(Value::as_array) else { return vec![] };
    let model = options.iter().find(|o| o.get("id").and_then(Value::as_str) == Some("model"));
    model.and_then(|m| m.get("options")).and_then(Value::as_array).map_or(vec![], |list| list.iter().filter_map(|o| Some((o.get("value")?.as_str()?.to_string(), o.get("name")?.as_str()?.to_string()))).collect())
}

/// The models this account may pick, from a new session: `devin models list` also shows ones the plan can't use. Empty
/// when it isn't signed in or that can't be told.
pub fn list_models() -> Vec<(String, String)> {
    if !installed() {
        return vec![];
    }
    let Ok(cwd) = data_dir() else { return vec![] };
    let (tx, _rx) = channel();
    let Ok(mut server) = Server::open(&cwd, &["acp"], tx) else { return vec![] };
    let session = server.conn.call("session/new", json!({"cwd": cwd.to_string_lossy(), "mcpServers": []}), Duration::from_secs(30));
    let models = session.as_ref().map(models_of).unwrap_or_default();
    if let Some(id) = session.ok().as_ref().and_then(|s| s.get("sessionId")) {
        let _ = server.conn.call("session/delete", json!({"sessionId": id}), Duration::from_secs(3));
    }
    server.stop();
    models
}

/// Devin's session mode for Scout's. Scout's own permissions still decide each write and command, so anything but plan
/// mode is Devin's "Code". Its "Plan" keeps the model from changing the project.
fn mode_for(mode: &str) -> &'static str {
    if mode == "plan" { "plan" } else { "accept-edits" }
}

/// One Devin session, on its own process: started for a turn and gone after it, so the conversation always comes from
/// Scout's messages and undo, compaction and switching provider have nothing to keep in step.
pub struct Run {
    rx: Receiver<Msg>,
    tx: Sender<Msg>,
    server: Server,
    session: Value,
}

impl Run {
    /// Starts a session and sends `prompt`. `quiet` is a one-off answer from Devin's summarizer, which has no tools.
    pub fn start(model: &str, cwd: &str, mode: &str, prompt: String, images: Vec<(String, String)>, quiet: bool) -> Result<Run, String> {
        let mut args = vec!["acp"];
        if model != "auto" {
            args.extend(["--model", model]);
        }
        if quiet {
            args.extend(["--agent-type", "summarizer"]);
        }
        let (tx, rx) = channel();
        let mut server = Server::open(Path::new(cwd), &args, tx.clone())?;
        let session = server.conn.call("session/new", json!({"cwd": cwd, "mcpServers": []}), Duration::from_secs(60)).and_then(|s| s.get("sessionId").cloned().ok_or_else(|| "Devin didn't start a session.".to_string()));
        let session = match session {
            Ok(s) => s,
            Err(e) => {
                server.stop();
                return Err(e);
            }
        };
        if !quiet && mode_for(mode) != "accept-edits" {
            let _ = server.conn.call("session/set_mode", json!({"sessionId": session, "modeId": mode_for(mode)}), Duration::from_secs(10));
        }
        let run = Run { rx, tx, server, session };
        run.send_prompt(crate::acp::prompt_blocks(prompt, if run.server.images { &images } else { &[] }));
        Ok(run)
    }

    pub fn next(&self, wait: Duration) -> Option<Msg> {
        self.rx.recv_timeout(wait).ok()
    }

    /// Sends a prompt; Msg::Idle says when it's done. Only one runs at a time, and the agent waits for that.
    pub fn send(&self, text: String) {
        self.send_prompt(crate::acp::prompt_blocks(text, &[]));
    }

    fn send_prompt(&self, prompt: Value) {
        let answer = self.server.conn.ask("session/prompt", json!({"sessionId": self.session, "prompt": prompt}));
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(match answer.recv() {
                Ok(Ok(r)) => Msg::Idle(r.get("stopReason").and_then(Value::as_str).unwrap_or("end_turn").to_string()),
                Ok(Err(e)) => Msg::Failed(e),
                Err(_) => Msg::Failed("The Devin CLI stopped unexpectedly.".into()),
            });
        });
    }

    pub fn abort(&self) {
        self.server.conn.write(&json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": self.session}}));
    }

    pub fn reply(&self, id: &Value, result: Value) {
        self.server.conn.reply(id, result);
    }

    pub fn reply_err(&self, id: &Value, message: &str) {
        self.server.conn.reply_err(id, -32000, message);
    }

    /// Answers terminal/create for a command Scout has already run. Devin only passes on an error from this call as
    /// "failed to create terminal", so a refused command comes back as one that ran and said why.
    pub fn reply_terminal(&self, id: &Value, output: String, code: i32) {
        let mut terminals = self.server.terminals.lock().unwrap();
        let terminal = format!("term{}", terminals.len() + 1);
        terminals.insert(terminal.clone(), (output, code));
        drop(terminals);
        self.reply(id, json!({"terminalId": terminal}));
    }

    /// Answers a permission request with the first option of that kind ("allow" or "reject").
    pub fn reply_permission(&self, id: &Value, allow: bool, options: &[(String, String)]) {
        let want = if allow { "allow" } else { "reject" };
        let outcome = match options.iter().find(|o| o.1.starts_with(want)) {
            Some(o) => json!({"outcome": "selected", "optionId": o.0}),
            None => json!({"outcome": "cancelled"}),
        };
        self.reply(id, json!({"outcome": outcome}));
    }

    /// Removes the session from Devin's own history and stops the process.
    pub fn finish(mut self) {
        let _ = self.server.conn.call("session/delete", json!({"sessionId": self.session}), Duration::from_secs(3));
        self.server.stop();
    }
}

/// What Devin's error means for the user.
pub fn explain(message: &str) -> String {
    let m = message.to_lowercase();
    if m.contains("log in") || m.contains("login") || m.contains("authenticate") {
        return "Devin isn't signed in. Sign in under Settings > Providers.".into();
    }
    if ["quota", "credit", "allowance", "exhausted", "usage limit", "rate limit", "upgrade"].iter().any(|w| m.contains(w)) {
        return format!("Your Devin allowance may have run out ({message}). Plans are at {PLAN_URL}. This chat is saved, so you can carry on by picking another provider in the model menu.");
    }
    format!("Devin: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_platform_has_a_full_sha256() {
        for (_, hash) in ARCHIVES {
            assert_eq!(hash.len(), 64);
            assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        }
        assert!(archive().is_ok());
    }

    #[test]
    fn the_updates_devin_really_sends_are_understood() {
        let m = update_of(&json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "hi"}}));
        assert!(matches!(m, Some(Msg::Text(t)) if t == "hi"));
        // Each request's usage comes twice, the second time for its sub-agent context.
        let usage = json!({"sessionUpdate": "usage_update", "used": 10, "_meta": {"cognition.ai/inputTokens": 7, "cognition.ai/outputTokens": 3, "cognition.ai/cachedReadTokens": 1}});
        assert!(matches!(update_of(&usage), Some(Msg::Usage { prompt: 7, completion: 3, cached: 1 })));
        let again = json!({"sessionUpdate": "usage_update", "_meta": {"cognition.ai/inputTokens": 7, "cognition.ai/subagent_context": {"parentAgentId": "root"}}});
        assert!(update_of(&again).is_none());
        let call = json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "Search for 'x'", "kind": "search", "rawInput": {"query": "x"}});
        assert!(matches!(update_of(&call), Some(Msg::ToolStart { kind, .. }) if kind == "search"));
        assert!(update_of(&json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "in_progress"})).is_none());
        let end = json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "failed", "content": [{"type": "content", "content": {"type": "text", "text": "no"}}]});
        assert!(matches!(update_of(&end), Some(Msg::ToolEnd { failed: true, text, .. }) if text == "no"));
    }

    #[test]
    fn the_model_list_comes_from_the_session() {
        let session = json!({"configOptions": [{"id": "mode", "options": [{"value": "ask", "name": "Ask"}]}, {"id": "model", "options": [{"value": "swe-1-6-slow", "name": "SWE-1.6 Slow"}]}]});
        assert_eq!(models_of(&session), vec![("swe-1-6-slow".to_string(), "SWE-1.6 Slow".to_string())]);
        assert!(models_of(&json!({})).is_empty());
    }

    #[test]
    fn exit_codes_come_from_the_output() {
        assert_eq!(exit_code("oops\n\n[exit code 2]"), 2);
        assert_eq!(exit_code("The user denied this bash call."), 1);
    }

    #[test]
    fn errors_say_what_to_do() {
        assert!(explain("Please log in to use Devin. Use `/login` to authenticate again.").contains("Settings > Providers"));
        assert!(explain("quota exceeded").contains(PLAN_URL));
        assert_eq!(explain("boom"), "Devin: boom");
    }

    #[test]
    fn plan_mode_is_the_only_special_one() {
        assert_eq!(mode_for("plan"), "plan");
        for m in ["default", "acceptEdits", "auto", "yolo"] {
            assert_eq!(mode_for(m), "accept-edits");
        }
    }

    /// Downloads and unpacks the real CLI into SCOUT_HOME: `SCOUT_HOME=<temp> cargo test -- --ignored live_devin_install --nocapture`.
    #[test]
    #[ignore]
    fn live_devin_install() {
        assert!(std::env::var("SCOUT_HOME").is_ok(), "set SCOUT_HOME so this doesn't touch the real install");
        let mut last = 0;
        ensure_runtime(&mut |done, total| last = total.max(done)).unwrap();
        println!("downloaded {last} bytes to {}", install_dir().display());
        assert!(installed());
        ensure_runtime(&mut |_, _| panic!("downloaded twice")).unwrap();
        let out = cli(&["--version"]).output().unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains(VERSION));
    }
}
