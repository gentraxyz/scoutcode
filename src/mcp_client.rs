//! MCP servers the user adds in Settings > MCP servers, whose tools Scout's model can then call. This is the other way
//! round from mcp.rs, which serves Scout's own tools to chatbots. They're kept in ~/.scoutcode/mcp.json, in the same
//! `mcpServers` format other apps use, and spoken to over stdio or Streamable HTTP. There's one set of servers for the
//! whole app, and only the window connects them, so `Scout.exe mcp` never hands them on to a chatbot.

use crate::api::http;
use crate::tools::ToolResult;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Long enough for `npx` to download a server the first time.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(120);
const CALL_TIMEOUT: Duration = Duration::from_secs(600);
/// What a provider accepts as a tool's name.
const MAX_NAME: usize = 64;
const MAX_DESCRIPTION: usize = 2000;
/// A session the server has forgotten, which a new `initialize` fixes.
const EXPIRED: &str = "The server ended the session.";
const CANCELLED: &str = "Interrupted by the user.";
/// The server wants a sign-in that Scout doesn't have, or that ran out.
pub const NEEDS_SIGN_IN: &str = "This server needs you to sign in. Open Settings > MCP servers and press Sign in.";
/// A 401 on a server Scout may sign in to, before a refresh is tried.
const UNAUTHORIZED: &str = "unauthorized";

/// One entry of mcp.json. Fields Scout doesn't use, like `type`, are kept when it saves the file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub command: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cwd: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl ServerConfig {
    /// What the server is, for Settings: never its env or headers, or a URL's query, which can hold keys.
    pub fn summary(&self) -> String {
        if !self.url.is_empty() {
            return self.url.split(['?', '#']).next().unwrap_or("").to_string();
        }
        std::iter::once(self.command.as_str()).chain(self.args.iter().map(String::as_str)).collect::<Vec<_>>().join(" ")
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct File {
    #[serde(rename = "mcpServers")]
    servers: BTreeMap<String, ServerConfig>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

pub fn file() -> PathBuf {
    crate::config::home().join("mcp.json")
}

fn load_from(path: &Path) -> Result<File, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("mcp.json isn't valid: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(File::default()),
        Err(e) => Err(format!("Could not read mcp.json: {e}")),
    }
}

fn save_to(path: &Path, f: &File) -> Result<(), String> {
    let parent = path.parent().ok_or("Invalid path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temp = path.with_extension(format!("{}.tmp", crate::session::now_ms()));
    let text = serde_json::to_string_pretty(f).map_err(|e| e.to_string())?;
    std::fs::write(&temp, text).and_then(|_| std::fs::rename(&temp, path)).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("Could not save mcp.json: {e}")
    })
}

/// Adds, replaces (dropping `old` if it was renamed) or, with None, removes a server. A file that doesn't parse is left alone.
pub fn set(old: Option<&str>, name: &str, cfg: Option<ServerConfig>) -> Result<(), String> {
    let path = file();
    let mut f = load_from(&path)?;
    if let Some(o) = old {
        f.servers.remove(o);
    }
    match cfg {
        Some(c) => _ = f.servers.insert(name.to_string(), c),
        None => _ = f.servers.remove(name),
    }
    save_to(&path, &f)
}

/// Writes an empty file if there's none, so there's something to open.
pub fn ensure_file() -> PathBuf {
    let path = file();
    if !path.exists() {
        let _ = save_to(&path, &File::default());
    }
    path
}

/// Why a server's name is no good, if it isn't.
pub fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Give the server a name.".into());
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("A server's name can only have letters, numbers, - and _.".into());
    }
    Ok(())
}

/// Splits a line of arguments on spaces, keeping quoted parts together.
pub fn split_args(s: &str) -> Vec<String> {
    let (mut out, mut word, mut quote, mut any) = (vec![], String::new(), None, false);
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if any {
                    out.push(std::mem::take(&mut word));
                    any = false;
                }
            }
            (None, c) => {
                word.push(c);
                any = true;
            }
        }
    }
    if any {
        out.push(word);
    }
    out
}

/// The opposite of `split_args`, for editing.
pub fn join_args(args: &[String]) -> String {
    args.iter().map(|a| if a.is_empty() || a.contains(char::is_whitespace) { format!("\"{a}\"") } else { a.clone() }).collect::<Vec<_>>().join(" ")
}

/// `KEY<sep>value` lines. Blank lines are skipped, and a line without the separator is an error.
pub fn parse_pairs(text: &str, sep: char) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Some((k, v)) = line.split_once(sep) else {
            // Not the line itself, which is likely to be a secret.
            return Err(format!("Each line needs a name, then {sep}, then a value."));
        };
        out.insert(k.trim().to_string(), v.trim().to_string());
    }
    Ok(out)
}

pub fn join_pairs(pairs: &BTreeMap<String, String>, sep: &str) -> String {
    pairs.iter().map(|(k, v)| format!("{k}{sep}{v}")).collect::<Vec<_>>().join("\n")
}

/// The name a tool goes by to the model: `mcp__server__tool`, in the characters and length providers allow. A name
/// that had to be cut keeps a hash of the whole, so two long names don't collide.
fn tool_id(server: &str, tool: &str) -> String {
    let full = format!("{}{server}__{tool}", crate::tools::MCP_PREFIX);
    let clean: String = full.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if clean.len() <= MAX_NAME {
        return clean;
    }
    let hash: String = Sha256::digest(full.as_bytes()).iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("{}_{hash}", &clean[..MAX_NAME - 9])
}

/// The text the model gets from a tools/call result.
fn render(result: &Value) -> String {
    let mut parts = vec![];
    for c in result["content"].as_array().map(Vec::as_slice).unwrap_or_default() {
        parts.push(match c["type"].as_str().unwrap_or("") {
            "text" => c["text"].as_str().unwrap_or("").to_string(),
            kind @ ("image" | "audio") => format!("[{kind} left out: {}]", c["mimeType"].as_str().unwrap_or("unknown type")),
            "resource" => c["resource"]["text"].as_str().map(String::from).unwrap_or_else(|| format!("[resource {}]", c["resource"]["uri"].as_str().unwrap_or(""))),
            "resource_link" => format!("[{}]", c["uri"].as_str().unwrap_or("")),
            _ => c.to_string(),
        });
    }
    if parts.is_empty() {
        if let Some(s) = result.get("structuredContent") {
            return serde_json::to_string_pretty(s).unwrap_or_default();
        }
        return "(no output)".into();
    }
    parts.join("\n")
}

// ---------- talking to a server ----------

type Pending = Arc<Mutex<HashMap<u64, Sender<Result<Value, String>>>>>;

struct StdioLink {
    /// None once it's being shut down, which closes the pipe.
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    child: Mutex<Child>,
    pending: Pending,
    stderr: Arc<Mutex<VecDeque<String>>>,
    gone: Arc<AtomicBool>,
}

fn write_line(stdin: &Mutex<Option<ChildStdin>>, msg: &Value) -> std::io::Result<()> {
    let mut guard = stdin.lock().unwrap();
    let Some(pipe) = guard.as_mut() else { return Err(std::io::ErrorKind::BrokenPipe.into()) };
    writeln!(pipe, "{msg}").and_then(|_| pipe.flush())
}

impl StdioLink {
    fn start(cfg: &ServerConfig) -> Result<StdioLink, String> {
        let mut c = Command::new(find_program(&cfg.command, &cfg.env));
        c.args(&cfg.args).envs(&cfg.env).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        if !cfg.cwd.is_empty() {
            c.current_dir(&cfg.cwd);
        } else if let Some(h) = crate::config::home().parent() {
            c.current_dir(h);
        }
        crate::tools::no_window(&mut c);
        let mut child = c.spawn().map_err(|e| format!("Could not start {}: {e}", cfg.command))?;
        let stdout = child.stdout.take().ok_or("No output pipe")?;
        let errors = child.stderr.take().ok_or("No error pipe")?;
        let stdin = Arc::new(Mutex::new(child.stdin.take()));
        let pending: Pending = Default::default();
        let stderr: Arc<Mutex<VecDeque<String>>> = Default::default();
        let gone = Arc::new(AtomicBool::new(false));

        let tail = stderr.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(errors).lines().map_while(Result::ok) {
                let mut t = tail.lock().unwrap();
                t.push_back(line);
                if t.len() > 20 {
                    t.pop_front();
                }
            }
        });

        let (input, waiting, ended) = (stdin.clone(), pending.clone(), gone.clone());
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
                match (msg.get("id").filter(|i| !i.is_null()), msg["method"].as_str()) {
                    // Its own requests. Scout offers no capabilities, so only ping is answered.
                    (Some(id), Some(method)) => {
                        let reply = if method == "ping" {
                            json!({"jsonrpc": "2.0", "id": id, "result": {}})
                        } else {
                            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Method not found"}})
                        };
                        let _ = write_line(&input, &reply);
                    }
                    (Some(id), None) => {
                        if let Some(tx) = id.as_u64().and_then(|i| waiting.lock().unwrap().remove(&i)) {
                            let _ = tx.send(Ok(msg));
                        }
                    }
                    _ => {}
                }
            }
            ended.store(true, Ordering::Relaxed);
            waiting.lock().unwrap().clear();
        });
        Ok(StdioLink { stdin, child: Mutex::new(child), pending, stderr, gone })
    }

    fn why_gone(&self) -> String {
        let tail: Vec<String> = self.stderr.lock().unwrap().iter().rev().take(4).rev().cloned().collect();
        if tail.is_empty() { "The server stopped.".into() } else { format!("The server stopped: {}", tail.join(" ")) }
    }
}

impl Drop for StdioLink {
    fn drop(&mut self) {
        // Closing the pipe is what ends a server started through a wrapper such as npx.cmd, which kill alone wouldn't reach.
        self.stdin.lock().unwrap().take();
        let mut child = self.child.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// On Windows only an .exe is found by name, and `npx` is really npx.cmd.
fn find_program(cmd: &str, env: &BTreeMap<String, String>) -> PathBuf {
    let plain = PathBuf::from(cmd);
    if !cfg!(windows) || plain.extension().is_some() || plain.components().count() > 1 {
        return plain;
    }
    let path = env.get("PATH").cloned().or_else(|| std::env::var("PATH").ok()).unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        for ext in ["exe", "cmd", "bat", "com"] {
            let f = dir.join(format!("{cmd}.{ext}"));
            if f.is_file() {
                return f;
            }
        }
    }
    plain
}

struct HttpLink {
    /// The server's name in mcp.json, which its sign-in is kept under.
    name: String,
    url: String,
    /// Whether Scout signs in itself: the user gave no Authorization header.
    oauth: bool,
    headers: Vec<(String, String)>,
    session: Mutex<Option<String>>,
    protocol: Mutex<Option<String>>,
}

impl HttpLink {
    /// Sends with the kept sign-in, refreshing it once if the server turns it down.
    fn post(&self, msg: &Value, timeout: Duration) -> Result<Value, String> {
        if !self.oauth {
            return self.post_once(msg, timeout, None);
        }
        let token = crate::mcp_auth::token(&self.name, &self.url);
        let unauthorized = |r: Result<Value, String>| r.map_err(|e| if e == UNAUTHORIZED { NEEDS_SIGN_IN.to_string() } else { e });
        match self.post_once(msg, timeout, token.as_deref()) {
            Err(e) if e == UNAUTHORIZED && token.is_some() => match crate::mcp_auth::refresh(&self.name, &self.url) {
                Ok(t) => unauthorized(self.post_once(msg, timeout, Some(&t))),
                Err(_) => Err(NEEDS_SIGN_IN.into()),
            },
            r => unauthorized(r),
        }
    }

    fn post_once(&self, msg: &Value, timeout: Duration, token: Option<&str>) -> Result<Value, String> {
        let mut req = http().post(&self.url).config().timeout_global(Some(timeout)).build().header("Accept", "application/json, text/event-stream");
        for (k, v) in &self.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        if let Some(t) = token {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        let session = self.session.lock().unwrap().clone();
        if let Some(s) = &session {
            req = req.header("Mcp-Session-Id", s.as_str());
        }
        if let Some(p) = self.protocol.lock().unwrap().as_deref() {
            req = req.header("MCP-Protocol-Version", p);
        }
        let res = req.send_json(msg).map_err(|e| format!("Can't reach the server: {e}"))?;
        let status = res.status().as_u16();
        if let Some(s) = res.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            *self.session.lock().unwrap() = Some(s.to_string());
        }
        match status {
            401 if self.oauth => return Err(UNAUTHORIZED.into()),
            401 | 403 if !self.oauth => return Err(format!("The server refused the token under Headers (HTTP {status}). Check it, or take the Authorization header out to sign in through the browser instead.")),
            403 => return Err("The server refused access (HTTP 403). Your account may not be allowed to use it; try signing out and in again.".into()),
            404 if session.is_some() => {
                *self.session.lock().unwrap() = None;
                return Err(EXPIRED.into());
            }
            404 | 405 => return Err(format!("HTTP {status}. Check the address: Scout speaks Streamable HTTP, not the older SSE transport.")),
            202 | 204 => return Ok(Value::Null),
            200..=299 => {}
            s => return Err(format!("The server answered HTTP {s}.")),
        }
        let is_stream = res.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|t| t.contains("text/event-stream"));
        if !is_stream {
            return res.into_body().read_json().map_err(|e| format!("The server's answer wasn't JSON: {e}"));
        }
        // The answer comes as one event among any others it sends first.
        let want = msg["id"].clone();
        let mut data = String::new();
        for line in BufReader::new(res.into_body().into_reader()).lines() {
            let line = line.map_err(|e| format!("The connection dropped: {e}"))?;
            if let Some(d) = line.strip_prefix("data:") {
                data.push_str(d.trim_start());
                continue;
            }
            if line.is_empty() && !data.is_empty() {
                if let Ok(v) = serde_json::from_str::<Value>(&std::mem::take(&mut data)) {
                    if v["id"] == want && v.get("method").is_none() {
                        return Ok(v);
                    }
                }
            }
        }
        Err("The server closed the stream without answering.".into())
    }
}

enum Link {
    Stdio(StdioLink),
    Http(Arc<HttpLink>),
}

struct Conn {
    link: Link,
    next_id: AtomicU64,
}

impl Conn {
    fn notify(&self, method: &str, params: Value) {
        let msg = json!({"jsonrpc": "2.0", "method": method, "params": params});
        match &self.link {
            Link::Stdio(s) => _ = write_line(&s.stdin, &msg),
            Link::Http(h) => _ = h.post(&msg, Duration::from_secs(15)),
        }
    }

    /// Tells the server to stop on a request Scout has given up on. Nothing waits for it.
    fn give_up(&self, id: u64, reason: &str) {
        let msg = json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": id, "reason": reason}});
        match &self.link {
            Link::Stdio(s) => {
                s.pending.lock().unwrap().remove(&id);
                let _ = write_line(&s.stdin, &msg);
            }
            Link::Http(h) => {
                let h = h.clone();
                std::thread::spawn(move || _ = h.post(&msg, Duration::from_secs(15)));
            }
        }
    }

    fn exchange(&self, method: &str, params: &Value, cancel: &AtomicBool, timeout: Duration) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let (tx, rx) = channel();
        match &self.link {
            Link::Stdio(s) => {
                s.pending.lock().unwrap().insert(id, tx);
                if s.gone.load(Ordering::Relaxed) || write_line(&s.stdin, &msg).is_err() {
                    s.pending.lock().unwrap().remove(&id);
                    return Err(s.why_gone());
                }
            }
            // On its own thread so a stop isn't held up by the network.
            Link::Http(h) => {
                let h = h.clone();
                std::thread::spawn(move || _ = tx.send(h.post(&msg, timeout)));
            }
        }
        let until = Instant::now() + timeout;
        let reply = loop {
            if cancel.load(Ordering::Relaxed) {
                self.give_up(id, "The user stopped it.");
                return Err(CANCELLED.into());
            }
            if Instant::now() >= until {
                self.give_up(id, "Timed out.");
                return Err(format!("The server didn't answer {method} within {} seconds.", timeout.as_secs()));
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(r) => break r?,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(match &self.link {
                        Link::Stdio(s) => s.why_gone(),
                        Link::Http(_) => "The connection dropped.".into(),
                    });
                }
            }
        };
        if let Some(e) = reply.get("error").filter(|e| !e.is_null()) {
            return Err(e["message"].as_str().map(String::from).unwrap_or_else(|| e.to_string()));
        }
        Ok(reply["result"].clone())
    }

    fn rpc(&self, method: &str, params: Value, cancel: &AtomicBool, timeout: Duration) -> Result<Value, String> {
        match self.exchange(method, &params, cancel, timeout) {
            Err(e) if e == EXPIRED => {
                self.initialize(cancel)?;
                self.exchange(method, &params, cancel, timeout)
            }
            r => r,
        }
    }

    fn initialize(&self, cancel: &AtomicBool) -> Result<(), String> {
        if let Link::Http(h) = &self.link {
            *h.protocol.lock().unwrap() = None;
        }
        let params = json!({
            "protocolVersion": crate::mcp::PROTOCOLS[0],
            "capabilities": {},
            "clientInfo": {"name": "scout", "title": "Scout", "version": crate::update::VERSION},
        });
        let init = self.exchange("initialize", &params, cancel, CONNECT_TIMEOUT)?;
        if let (Link::Http(h), Some(v)) = (&self.link, init["protocolVersion"].as_str()) {
            *h.protocol.lock().unwrap() = Some(v.to_string());
        }
        self.notify("notifications/initialized", json!({}));
        Ok(())
    }

    fn list_tools(&self, server: &str, cancel: &AtomicBool) -> Result<Vec<Tool>, String> {
        let mut tools: Vec<Tool> = vec![];
        let mut cursor: Option<String> = None;
        // A server that keeps handing out cursors doesn't get to hang this.
        for _ in 0..20 {
            let params = cursor.as_ref().map_or(json!({}), |c| json!({"cursor": c}));
            let page = self.rpc("tools/list", params, cancel, CONNECT_TIMEOUT)?;
            for t in page["tools"].as_array().map(Vec::as_slice).unwrap_or_default() {
                let Some(name) = t["name"].as_str() else { continue };
                let id = tool_id(server, name);
                if tools.iter().any(|x| x.id == id) {
                    continue;
                }
                let mut parameters = t.get("inputSchema").filter(|s| s.is_object()).cloned().unwrap_or(json!({"properties": {}}));
                parameters.as_object_mut().unwrap().entry("type").or_insert(json!("object"));
                let description: String = t["description"].as_str().unwrap_or("").chars().take(MAX_DESCRIPTION).collect();
                let spec = json!({"type": "function", "function": {"name": id, "description": format!("From the MCP server \"{server}\". {description}"), "parameters": parameters}});
                tools.push(Tool { id, name: name.to_string(), spec });
            }
            cursor = page["nextCursor"].as_str().filter(|c| !c.is_empty()).map(String::from);
            if cursor.is_none() {
                break;
            }
        }
        Ok(tools)
    }
}

fn connect(name: &str, cfg: &ServerConfig) -> Result<(Conn, Vec<Tool>), String> {
    if cfg.extra.get("type").and_then(Value::as_str) == Some("sse") {
        return Err("This server uses the older SSE transport, which Scout doesn't support. Look for a Streamable HTTP address.".into());
    }
    let link = if !cfg.url.is_empty() {
        let oauth = !cfg.headers.keys().any(|k| k.eq_ignore_ascii_case("authorization"));
        Link::Http(Arc::new(HttpLink { name: name.to_string(), url: cfg.url.clone(), oauth, headers: cfg.headers.iter().map(|(k, v)| (k.clone(), v.clone())).collect(), session: Mutex::new(None), protocol: Mutex::new(None) }))
    } else if !cfg.command.is_empty() {
        Link::Stdio(StdioLink::start(cfg)?)
    } else {
        return Err("It needs a command or a URL.".into());
    };
    let conn = Conn { link, next_id: AtomicU64::new(1) };
    let cancel = AtomicBool::new(false);
    conn.initialize(&cancel)?;
    let tools = conn.list_tools(name, &cancel)?;
    Ok((conn, tools))
}

// ---------- the app's servers ----------

#[derive(Clone)]
struct Tool {
    id: String,
    /// What the server calls it.
    name: String,
    spec: Value,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Status {
    Off,
    Connecting,
    Ready,
    /// A remote server that wants the user to sign in through the browser.
    NeedsSignIn,
    /// Waiting on the browser.
    SigningIn,
    Failed(String),
}

struct Server {
    name: String,
    cfg: ServerConfig,
    status: Status,
    tools: Vec<Tool>,
    conn: Option<Arc<Conn>>,
    /// Which connection attempt it's waiting for, so one that was replaced can't report in.
    attempt: u64,
    /// Whether a browser sign-in is kept for it, as of its last connection.
    signed_in: bool,
}

/// A server as Settings shows it.
pub struct ServerView {
    pub name: String,
    pub cfg: ServerConfig,
    pub status: Status,
    pub tools: Vec<String>,
    /// Scout has a browser sign-in kept for it.
    pub signed_in: bool,
}

#[derive(Default)]
struct Inner {
    servers: Vec<Server>,
    error: Option<String>,
    attempts: u64,
    /// Servers just saved from Settings, whose sign-in starts by itself if they turn out to need one.
    sign_in_on_connect: std::collections::HashSet<String>,
    /// Bumped to stop the sign-in that's waiting on the browser.
    sign_in_gen: Arc<AtomicU64>,
}

#[derive(Default)]
pub struct Hub {
    inner: Mutex<Inner>,
}

pub fn hub() -> &'static Hub {
    static HUB: OnceLock<Hub> = OnceLock::new();
    HUB.get_or_init(Hub::default)
}

impl Hub {
    /// Matches the servers to mcp.json: unchanged ones stay connected unless `force`, and the rest connect in the
    /// background. If the file doesn't parse, says so and leaves things as they are.
    pub fn reload(&'static self, ctx: &crate::waker::Waker, force: bool) {
        let file = match load_from(&file()) {
            Ok(f) => f,
            Err(e) => {
                self.inner.lock().unwrap().error = Some(e);
                return;
            }
        };
        let mut inner = self.inner.lock().unwrap();
        inner.error = None;
        let old = std::mem::take(&mut inner.servers);
        for (name, cfg) in file.servers {
            if let Some(s) = old.iter().find(|s| !force && s.name == name && s.cfg == cfg && s.status != Status::Connecting && !matches!(s.status, Status::Failed(_))) {
                inner.servers.push(Server { name, cfg, status: s.status.clone(), tools: s.tools.clone(), conn: s.conn.clone(), attempt: s.attempt, signed_in: s.signed_in });
                continue;
            }
            inner.servers.push(Server { name, cfg, status: Status::Off, tools: vec![], conn: None, attempt: 0, signed_in: false });
            let i = inner.servers.len() - 1;
            self.start(&mut inner, i, ctx);
        }
    }

    /// Connects the server at `i` in the background, unless it's turned off.
    fn start(&'static self, inner: &mut Inner, i: usize, ctx: &crate::waker::Waker) {
        inner.attempts += 1;
        let attempt = inner.attempts;
        let s = &mut inner.servers[i];
        s.attempt = attempt;
        s.tools.clear();
        s.conn = None;
        s.status = if s.cfg.disabled { Status::Off } else { Status::Connecting };
        if !s.cfg.disabled {
            let (n, c, ctx) = (s.name.clone(), s.cfg.clone(), ctx.clone());
            std::thread::spawn(move || {
                let result = connect(&n, &c);
                let signed_in = !c.url.is_empty() && crate::mcp_auth::signed_in(&n, &c.url);
                self.connected(&n, attempt, result, signed_in, &ctx);
                ctx.request_repaint();
            });
        }
    }

    fn connected(&'static self, name: &str, attempt: u64, result: Result<(Conn, Vec<Tool>), String>, signed_in: bool, ctx: &crate::waker::Waker) {
        let mut inner = self.inner.lock().unwrap();
        let Some(s) = inner.servers.iter_mut().find(|s| s.name == name && s.attempt == attempt) else { return };
        s.signed_in = signed_in;
        match result {
            Ok((conn, tools)) => {
                s.conn = Some(Arc::new(conn));
                s.tools = tools;
                s.status = Status::Ready;
            }
            Err(e) if e == NEEDS_SIGN_IN => s.status = Status::NeedsSignIn,
            Err(e) => s.status = Status::Failed(e.chars().take(400).collect()),
        }
        let needs = s.status == Status::NeedsSignIn;
        let auto = inner.sign_in_on_connect.remove(name);
        if needs && auto {
            drop(inner);
            self.sign_in(name, ctx);
        }
    }

    /// Starts the browser sign-in for this server if it needs one the moment Settings saves it.
    pub fn sign_in_when_needed(&self, name: &str) {
        self.inner.lock().unwrap().sign_in_on_connect.insert(name.to_string());
    }

    /// Opens the browser to sign in to a remote server, then connects it. Only one sign-in waits at a time.
    pub fn sign_in(&'static self, name: &str, ctx: &crate::waker::Waker) {
        let (url, my_gen, gen_now) = {
            let mut inner = self.inner.lock().unwrap();
            let gen_now = inner.sign_in_gen.clone();
            let my_gen = gen_now.fetch_add(1, Ordering::SeqCst) + 1;
            // A server left waiting by a sign-in this one replaces goes back to asking.
            for s in inner.servers.iter_mut().filter(|s| s.status == Status::SigningIn) {
                s.status = Status::NeedsSignIn;
            }
            let Some(s) = inner.servers.iter_mut().find(|s| s.name == name && !s.cfg.url.is_empty()) else { return };
            s.status = Status::SigningIn;
            (s.cfg.url.clone(), my_gen, gen_now)
        };
        let (name, ctx) = (name.to_string(), ctx.clone());
        ctx.request_repaint();
        std::thread::spawn(move || {
            let cancelled = || gen_now.load(Ordering::SeqCst) != my_gen;
            let result = crate::mcp_auth::sign_in(&name, &url, |_| {}, &cancelled);
            if cancelled() {
                return;
            }
            let mut inner = self.inner.lock().unwrap();
            let Some(i) = inner.servers.iter().position(|s| s.name == name && s.cfg.url == url) else { return };
            match result {
                Ok(()) => self.start(&mut inner, i, &ctx),
                Err(e) => inner.servers[i].status = Status::Failed(format!("Sign-in didn't work: {e}").chars().take(400).collect()),
            }
            drop(inner);
            ctx.request_repaint();
        });
    }

    /// Stops waiting on the browser.
    pub fn cancel_sign_in(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.sign_in_gen.fetch_add(1, Ordering::SeqCst);
        for s in inner.servers.iter_mut().filter(|s| s.status == Status::SigningIn) {
            s.status = Status::NeedsSignIn;
        }
    }

    /// Forgets the server's sign-in and connects it again, which then asks for one.
    pub fn sign_out(&'static self, name: &str, ctx: &crate::waker::Waker) {
        crate::mcp_auth::forget(name);
        let mut inner = self.inner.lock().unwrap();
        if let Some(i) = inner.servers.iter().position(|s| s.name == name) {
            inner.servers[i].signed_in = false;
            self.start(&mut inner, i, ctx);
        }
    }

    pub fn error(&self) -> Option<String> {
        self.inner.lock().unwrap().error.clone()
    }

    pub fn views(&self) -> Vec<ServerView> {
        let inner = self.inner.lock().unwrap();
        inner
            .servers
            .iter()
            .map(|s| ServerView {
                name: s.name.clone(),
                cfg: s.cfg.clone(),
                status: s.status.clone(),
                tools: s.tools.iter().map(|t| t.name.clone()).collect(),
                signed_in: s.signed_in,
            })
            .collect()
    }

    /// The tool specs of every connected server, in a steady order so the prompt cache holds.
    pub fn specs(&self) -> Vec<Value> {
        let inner = self.inner.lock().unwrap();
        inner.servers.iter().filter(|s| s.status == Status::Ready).flat_map(|s| s.tools.iter().map(|t| t.spec.clone())).collect()
    }

    /// Runs a tool by the name the model knows it by.
    pub fn call(&self, id: &str, args: &Value, cancel: &AtomicBool) -> ToolResult {
        let found = {
            let inner = self.inner.lock().unwrap();
            inner.servers.iter().filter(|s| s.status == Status::Ready).find_map(|s| {
                let t = s.tools.iter().find(|t| t.id == id)?;
                Some((s.conn.clone()?, t.name.clone()))
            })
        };
        let Some((conn, name)) = found else {
            return ToolResult::err(format!("{id} isn't available: its MCP server isn't connected. Don't retry it; tell the user if you need it."), "not connected");
        };
        let arguments = if args.is_object() { args.clone() } else { json!({}) };
        match conn.rpc("tools/call", json!({"name": name, "arguments": arguments}), cancel, CALL_TIMEOUT) {
            Ok(r) => {
                let output = render(&r);
                let lines = output.lines().count();
                if r["isError"] == true {
                    let first: String = output.lines().next().unwrap_or("").chars().take(100).collect();
                    ToolResult::err(output, first)
                } else {
                    ToolResult::ok(output, format!("{lines} line{}", if lines == 1 { "" } else { "s" }))
                }
            }
            Err(e) => {
                if e == NEEDS_SIGN_IN {
                    let mut inner = self.inner.lock().unwrap();
                    if let Some(s) = inner.servers.iter_mut().find(|s| s.tools.iter().any(|t| t.id == id)) {
                        s.status = Status::NeedsSignIn;
                    }
                }
                let first: String = e.lines().next().unwrap_or("").chars().take(100).collect();
                ToolResult::err(e, first)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;

    #[test]
    fn config_round_trips_and_keeps_unknown_fields() {
        let dir = std::env::temp_dir().join(format!("scout-mcpc-{}", crate::session::now_ms()));
        let path = dir.join("mcp.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, r#"{"other":1,"mcpServers":{"a":{"type":"stdio","command":"npx","args":["-y","x"],"env":{"K":"v"}}}}"#).unwrap();
        let f = load_from(&path).unwrap();
        assert_eq!(f.servers["a"].command, "npx");
        save_to(&path, &f).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"other\"") && text.contains("\"type\": \"stdio\"") && !text.contains("disabled") && !text.contains("\"url\""), "{text}");
        std::fs::write(&path, "broken").unwrap();
        assert!(load_from(&path).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tool_names_suit_every_provider() {
        assert_eq!(tool_id("github", "create_issue"), "mcp__github__create_issue");
        assert_eq!(tool_id("a", "b.c/d"), "mcp__a__b_c_d");
        let (x, y) = (tool_id("s", &"long_".repeat(30)), tool_id("s", &format!("{}x", "long_".repeat(30))));
        assert!(x.len() <= MAX_NAME && y.len() <= MAX_NAME && x != y, "{x} {y}");
    }

    #[test]
    fn args_and_pairs_split() {
        assert_eq!(split_args(r#"-y "my pkg" --x='a b' """#), vec!["-y", "my pkg", "--x=a b", ""]);
        assert_eq!(split_args(&join_args(&["a b".to_string(), "c".to_string()])), vec!["a b", "c"]);
        let h = parse_pairs("Authorization: Bearer a:b\n\nX-Y: 1", ':').unwrap();
        assert_eq!(h["Authorization"], "Bearer a:b");
        // The message mustn't repeat the line, which may be a secret.
        assert!(!parse_pairs("sk-secret", ':').unwrap_err().contains("sk-secret"));
        assert_eq!(ServerConfig { url: "https://x.dev/mcp?key=sk".into(), ..Default::default() }.summary(), "https://x.dev/mcp");
    }

    #[test]
    fn renders_results() {
        assert_eq!(render(&json!({"content": [{"type": "text", "text": "a"}, {"type": "image", "mimeType": "image/png", "data": "xx"}]})), "a\n[image left out: image/png]");
        assert_eq!(render(&json!({})), "(no output)");
    }

    /// A Streamable HTTP server that answers initialize with JSON and everything else as an event stream, and forgets
    /// its session once, as a restarted server would.
    fn http_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let mut forgot = false;
            for stream in listener.incoming() {
                let mut s = stream.unwrap();
                let mut buf = vec![];
                let mut chunk = [0u8; 4096];
                let (head, body) = loop {
                    let n = s.read(&mut chunk).unwrap();
                    if n == 0 {
                        break (String::new(), String::new());
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(i) = text.find("\r\n\r\n") {
                        let head = text[..i].to_lowercase();
                        let len: usize = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                        if text.len() >= i + 4 + len {
                            break (head, text[i + 4..].to_string());
                        }
                    }
                };
                let msg: Value = serde_json::from_str(&body).unwrap_or(json!({}));
                let id = msg["id"].clone();
                let respond = |s: &mut std::net::TcpStream, status: &str, ty: &str, extra: &str, body: &str| {
                    let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Type: {ty}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                };
                match msg["method"].as_str().unwrap_or("") {
                    "initialize" => respond(&mut s, "200 OK", "application/json", "Mcp-Session-Id: sess\r\n", &json!({"jsonrpc": "2.0", "id": id, "result": {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "t", "version": "1"}}}).to_string()),
                    "tools/list" => respond(&mut s, "200 OK", "text/event-stream", "", &format!("data: {}\n\ndata: {}\n\n", json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": {}}), json!({"jsonrpc": "2.0", "id": id, "result": {"tools": [{"name": "add", "description": "Adds", "inputSchema": {"type": "object", "properties": {"a": {"type": "number"}}}}]}}))),
                    "tools/call" if !forgot && head.contains("mcp-session-id: sess") => {
                        forgot = true;
                        respond(&mut s, "404 Not Found", "text/plain", "", "gone");
                    }
                    "tools/call" => respond(&mut s, "200 OK", "text/event-stream", "", &format!("data: {}\n\n", json!({"jsonrpc": "2.0", "id": id, "result": {"content": [{"type": "text", "text": format!("got {}", msg["params"]["arguments"]["a"])}]}}))),
                    _ => respond(&mut s, "202 Accepted", "text/plain", "", ""),
                }
            }
        });
        url
    }

    #[test]
    fn speaks_streamable_http_and_recovers_a_lost_session() {
        let cfg = ServerConfig { url: http_server(), ..Default::default() };
        let (conn, tools) = connect("calc", &cfg).unwrap();
        assert_eq!(tools[0].id, "mcp__calc__add");
        assert_eq!(tools[0].spec["function"]["parameters"]["type"], "object");
        let cancel = AtomicBool::new(false);
        let r = conn.rpc("tools/call", json!({"name": "add", "arguments": {"a": 2}}), &cancel, Duration::from_secs(10)).unwrap();
        assert_eq!(render(&r), "got 2");
    }

    /// Stdio, against a little server in Node. Skipped where Node isn't installed.
    #[test]
    fn speaks_stdio_and_reports_a_dead_server() {
        let node_ok = Command::new(find_program("node", &BTreeMap::new())).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
        if !node_ok {
            return;
        }
        let script = r#"
            console.error('warming up');
            require('readline').createInterface({input: process.stdin}).on('line', l => {
                const m = JSON.parse(l);
                const send = o => console.log(JSON.stringify({jsonrpc: '2.0', ...o}));
                if (m.method === 'initialize') send({id: m.id, result: {protocolVersion: m.params.protocolVersion, capabilities: {tools: {}}, serverInfo: {name: 's', version: '1'}}});
                if (m.method === 'tools/list') {
                    send({id: 99, method: 'ping'});
                    send({id: m.id, result: {tools: [{name: 'boom', inputSchema: {type: 'object'}}, {name: 'die', inputSchema: {type: 'object'}}, {name: 'slow', inputSchema: {type: 'object'}}]}});
                }
                if (m.method === 'tools/call' && m.params.name === 'boom') send({id: m.id, result: {isError: true, content: [{type: 'text', text: 'bad ' + process.env.ANSWER}]}});
                if (m.method === 'tools/call' && m.params.name === 'die') { console.error('fatal: out of luck'); process.exit(1); }
            });
        "#;
        let cfg = ServerConfig { command: "node".into(), args: vec!["-e".into(), script.into()], env: BTreeMap::from([("ANSWER".to_string(), "42".to_string())]), ..Default::default() };
        let (conn, tools) = connect("local", &cfg).unwrap();
        assert_eq!(tools.len(), 3);
        let never = AtomicBool::new(false);
        let r = conn.rpc("tools/call", json!({"name": "boom", "arguments": {}}), &never, Duration::from_secs(10)).unwrap();
        assert_eq!(r["isError"], true);
        assert_eq!(render(&r), "bad 42");
        // Stopping a call that never answers comes back at once.
        let stop = AtomicBool::new(true);
        assert_eq!(conn.rpc("tools/call", json!({"name": "slow", "arguments": {}}), &stop, Duration::from_secs(10)).unwrap_err(), CANCELLED);
        let e = conn.rpc("tools/call", json!({"name": "die", "arguments": {}}), &never, Duration::from_secs(10)).unwrap_err();
        assert!(e.contains("out of luck"), "{e}");
    }

    /// `npx` on Windows is really npx.cmd, which has to be found by hand and still get its arguments intact.
    #[cfg(windows)]
    #[test]
    fn finds_a_cmd_wrapper_on_the_path() {
        let dir = std::env::temp_dir().join(format!("scout-mcpc-cmd-{}", crate::session::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("wrapped.cmd"), "@echo off\r\necho got %~1\r\n").unwrap();
        let env = BTreeMap::from([("PATH".to_string(), dir.to_string_lossy().to_string())]);
        let found = find_program("wrapped", &env);
        assert_eq!(found, dir.join("wrapped.cmd"));
        let out = Command::new(found).arg("two words").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "got two words");
        assert_eq!(find_program("no-such-program", &env), PathBuf::from("no-such-program"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A real server, in another process: Scout's own `mcp` command, from `cargo build`.
    /// `cargo test -- --ignored live_scout_as_server --nocapture`
    #[test]
    #[ignore]
    fn live_scout_as_server() {
        let exe = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join(if cfg!(windows) { "scout-desktop.exe" } else { "scout-desktop" });
        assert!(exe.exists(), "run cargo build first: {}", exe.display());
        let dir = std::env::temp_dir().join(format!("scout-mcpc-live-{}", crate::session::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("hello.txt"), "hello from a file").unwrap();
        let cfg = ServerConfig { command: exe.to_string_lossy().to_string(), args: vec!["mcp".into(), "--cwd".into(), dir.to_string_lossy().to_string()], ..Default::default() };
        let (conn, tools) = connect("scout", &cfg).unwrap();
        println!("{:?}", tools.iter().map(|t| &t.id).collect::<Vec<_>>());
        assert!(tools.iter().any(|t| t.id == "mcp__scout__view"));
        let never = AtomicBool::new(false);
        let r = conn.rpc("tools/call", json!({"name": "view", "arguments": {"path": "hello.txt"}}), &never, Duration::from_secs(30)).unwrap();
        assert!(render(&r).contains("hello from a file"), "{r}");
        // It's read-only unless told otherwise, and says so as a tool error.
        let r = conn.rpc("tools/call", json!({"name": "write_file", "arguments": {"path": "x.txt", "content": "no"}}), &never, Duration::from_secs(30)).unwrap();
        assert_eq!(r["isError"], true);
        assert!(!dir.join("x.txt").exists());
        drop(conn);

        // The way the window does it: from mcp.json, through the hub, and gone again when it's removed.
        unsafe { std::env::set_var("SCOUT_HOME", dir.join("home")) };
        let ctx = crate::waker::Waker::default();
        set(None, "scout", Some(cfg.clone())).unwrap();
        hub().reload(&ctx, false);
        let until = Instant::now() + Duration::from_secs(30);
        while hub().views()[0].status == Status::Connecting && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(hub().views()[0].status, Status::Ready);
        assert!(hub().specs().iter().any(|s| s["function"]["name"] == "mcp__scout__view"));
        let r = hub().call("mcp__scout__view", &json!({"path": "hello.txt"}), &never);
        assert!(!r.is_error && r.output.contains("hello from a file"), "{}", r.output);
        // Turned off: no tools, and calling one says why.
        set(None, "scout", Some(ServerConfig { disabled: true, ..cfg })).unwrap();
        hub().reload(&ctx, false);
        assert_eq!(hub().views()[0].status, Status::Off);
        assert!(hub().specs().is_empty());
        assert!(hub().call("mcp__scout__view", &json!({}), &never).is_error);
        set(None, "scout", None).unwrap();
        hub().reload(&ctx, false);
        assert!(hub().views().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A real server from npm, through npx (and npx.cmd on Windows): `cargo test -- --ignored live_mcp --nocapture`.
    #[test]
    #[ignore]
    fn live_mcp_server_from_npm() {
        let cfg = ServerConfig { command: "npx".into(), args: vec!["-y".into(), "@modelcontextprotocol/server-everything".into()], ..Default::default() };
        let (conn, tools) = connect("everything", &cfg).unwrap();
        println!("{:?}", tools.iter().map(|t| &t.id).collect::<Vec<_>>());
        let r = conn.rpc("tools/call", json!({"name": "echo", "arguments": {"message": "hello"}}), &AtomicBool::new(false), Duration::from_secs(60)).unwrap();
        assert!(render(&r).contains("hello"));
    }
}

#[cfg(test)]
pub fn connect_for_test(name: &str, cfg: &ServerConfig) -> Result<(), String> {
    connect(name, cfg).map(|_| ())
}
