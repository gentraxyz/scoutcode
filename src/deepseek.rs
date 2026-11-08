//! DeepSeek Web: the user's own chat.deepseek.com account as a provider, with no API key.
//!
//! A driver script (`deepseek/driver.js`) runs inside chat.deepseek.com and sends each request through the page the way
//! the user would, then streams the answer back. It runs only in the **Scout for DeepSeek extension**, in the user's own
//! Chrome or Edge, on an open chat.deepseek.com tab. The extension reaches Scout through a small server on 127.0.0.1
//! that only answers requests with this PC's token. Scout waits a random few seconds before each request, so the
//! account isn't driven at a steady rhythm.
//!
//! The website has no tool calling, so Scout runs the tool loop: the prompt lists Scout's tools (with the user's MCP
//! servers) and asks for `<tool_call>` blocks, which `tools::recover_calls` reads back. Each Scout chat keeps one
//! DeepSeek chat while the history matches, so only what's new is sent each step.

use crate::api::{ApiError, Delta, FunctionCall, Message, ModelInfo, StreamResult, ToolCall};
use serde_json::{Value, json};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const PROVIDER: &str = "deepseek-web";
/// The extension's local server tries this port first.
const PORT: u16 = 47615;

pub const DRIVER: &str = include_str!("deepseek/driver.js");
/// The Scout browser extension: DeepSeek Web, and the Claude connector setup (claude_setup.rs).
const EXTENSION: [(&str, &str); 8] = [
    ("manifest.json", include_str!("deepseek/extension/manifest.json")),
    ("claude.js", include_str!("deepseek/extension/claude.js")),
    ("driver.js", DRIVER),
    ("main.js", include_str!("deepseek/extension/main.js")),
    ("bridge.js", include_str!("deepseek/extension/bridge.js")),
    ("background.js", include_str!("deepseek/extension/background.js")),
    ("popup.html", include_str!("deepseek/extension/popup.html")),
    ("popup.js", include_str!("deepseek/extension/popup.js")),
];

pub const WARNING: &str = "DeepSeek Web drives your own chat.deepseek.com account the way a browser would. DeepSeek's terms don't allow automated use of the website, so DeepSeek could limit or ban the account. It also breaks when DeepSeek changes the site, until Scout is updated. For dependable use, link the DeepSeek API with a key instead.";

/// The ways to run it, which the model menu lists as models.
struct Mode {
    id: &'static str,
    name: &'static str,
    thinking: bool,
    search: bool,
    description: &'static str,
}

const MODES: [Mode; 4] = [
    Mode { id: "chat", name: "DeepSeek Web", thinking: false, search: false, description: "DeepSeek's default chat model on chat.deepseek.com, with your account." },
    Mode { id: "think", name: "DeepSeek Web DeepThink", thinking: true, search: false, description: "DeepThink on: reasons before it answers. Slower, better on hard problems." },
    Mode { id: "search", name: "DeepSeek Web + Search", thinking: false, search: true, description: "DeepSeek's web search on, for questions about recent things." },
    Mode { id: "think-search", name: "DeepSeek Web DeepThink + Search", thinking: true, search: true, description: "DeepThink and web search both on." },
];

fn mode(model: &str) -> &'static Mode {
    let id = model.strip_prefix("deepseek-web/").unwrap_or(model);
    MODES.iter().find(|m| m.id == id).unwrap_or(&MODES[0])
}

/// The model menu's entries.
pub fn models() -> Vec<ModelInfo> {
    MODES
        .iter()
        .map(|m| ModelInfo {
            id: format!("{PROVIDER}/{}", m.id),
            name: Some(m.name.into()),
            description: Some(m.description.into()),
            tools: Some(true),
            vision: Some(false),
            cost: Some("Free".into()),
            free: true,
            intelligence: Some(if m.thinking { 3 } else { 2 }),
            ..Default::default()
        })
        .collect()
}

// ---------- settings ----------

/// The pacing presets: the wait before each request is a random time between the two, in seconds.
pub const PACINGS: [(&str, u32, u32); 3] = [("passive", 5, 9), ("moderate", 4, 6), ("aggressive", 2, 3)];
const PACING_DEFAULT: &str = "passive";

/// What's saved in ~/.scoutcode/deepseek-web.json.
#[derive(Clone)]
pub struct Settings {
    pub enabled: bool,
    /// What the extension has to send to the local server.
    pub token: String,
    /// One of PACINGS' names.
    pub pacing: String,
}

/// The wait range of a pacing preset, in seconds.
fn pacing_range(name: &str) -> (u32, u32) {
    PACINGS.iter().find(|p| p.0 == name).map_or((5, 9), |p| (p.1, p.2))
}

fn settings_file() -> PathBuf {
    crate::config::home().join("deepseek-web.json")
}

pub fn settings() -> Settings {
    let s: Value = std::fs::read_to_string(settings_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    let token = s.get("token").and_then(Value::as_str).filter(|t| t.len() >= 32).map(String::from).unwrap_or_else(new_token);
    let pacing = s.get("pacing").and_then(Value::as_str).filter(|p| PACINGS.iter().any(|x| x.0 == *p)).unwrap_or(PACING_DEFAULT).to_string();
    Settings { enabled: s.get("enabled").and_then(Value::as_bool).unwrap_or(false), token, pacing }
}

fn save(s: &Settings) -> Result<(), String> {
    let body = json!({ "enabled": s.enabled, "token": s.token, "pacing": s.pacing });
    std::fs::write(settings_file(), serde_json::to_string_pretty(&body).unwrap()).map_err(|e| format!("Couldn't save DeepSeek Web's settings: {e}"))
}

fn new_token() -> String {
    let mut b = [0u8; 24];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn enabled() -> bool {
    settings().enabled
}

/// Turns DeepSeek Web on or off. Turning it on starts the extension's server and writes the extension.
pub fn set(enabled: Option<bool>) -> Result<(), String> {
    let mut s = settings();
    if let Some(e) = enabled {
        s.enabled = e;
    }
    save(&s)?;
    if s.enabled {
        serve();
        write_extension(&s)?;
    }
    Ok(())
}

/// Sets how hard Scout paces DeepSeek Web: passive, moderate or aggressive.
pub fn set_pacing(name: &str) -> Result<(), String> {
    if !PACINGS.iter().any(|p| p.0 == name) {
        return Err("Pick passive, moderate or aggressive.".into());
    }
    let mut s = settings();
    s.pacing = name.to_string();
    save(&s)
}

/// Called once the app is up.
pub fn init() {
    let s = settings();
    if s.enabled {
        serve();
        let _ = write_extension(&s);
    }
}

/// What Settings shows.
pub fn status() -> Value {
    let s = settings();
    let ext = EXT.lock().unwrap();
    let connected = ext.seen.is_some_and(|t| t.elapsed() < Duration::from_secs(20));
    json!({
        "enabled": s.enabled,
        "signedIn": connected && ext.status.get("signedIn").and_then(Value::as_bool).unwrap_or(false),
        "extensionConnected": connected,
        "extensionTab": connected && ext.status.get("path").is_some(),
        "extensionPath": extension_dir().to_string_lossy(),
        "port": *SERVING.lock().unwrap(),
        "pacing": s.pacing,
        "challenge": ext.status.get("challenge").and_then(Value::as_bool).unwrap_or(false),
        "sessionsToday": sessions_today().len(),
        "sessionsPerDay": SESSIONS_PER_DAY,
        "warning": WARNING,
    })
}

// ---------- the extension's server ----------

#[derive(Default)]
struct Ext {
    /// What to tell the extension next.
    commands: Vec<Value>,
    /// When it last asked.
    seen: Option<Instant>,
    /// What the page last said about itself.
    status: Value,
}

static EXT: Mutex<Ext> = Mutex::new(Ext { commands: Vec::new(), seen: None, status: Value::Null });
static EXT_WAKE: Condvar = Condvar::new();
static SERVING: Mutex<Option<u16>> = Mutex::new(None);

fn extension_dir() -> PathBuf {
    crate::config::home().join("deepseek-extension")
}

pub fn extension_path() -> String {
    extension_dir().to_string_lossy().to_string()
}

/// Writes the extension's files where the user loads it from, with this PC's port and token.
pub fn write_extension(s: &Settings) -> Result<PathBuf, String> {
    let dir = extension_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (name, body) in EXTENSION {
        std::fs::write(dir.join(name), body).map_err(|e| format!("Couldn't write the extension: {e}"))?;
    }
    let port = SERVING.lock().unwrap().unwrap_or(PORT);
    std::fs::write(dir.join("scout.json"), json!({ "port": port, "token": s.token }).to_string()).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Sets up the extension's folder and shows it, for Load unpacked.
pub fn extension_folder() -> Result<PathBuf, String> {
    serve();
    let dir = write_extension(&settings())?;
    Ok(dir.join("manifest.json"))
}

/// Starts the local server, once. Only 127.0.0.1, and only for requests with the token.
fn serve() {
    let mut serving = SERVING.lock().unwrap();
    if serving.is_some() {
        return;
    }
    let Ok(listener) = TcpListener::bind(("127.0.0.1", PORT)).or_else(|_| TcpListener::bind(("127.0.0.1", 0))) else { return };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(PORT);
    *serving = Some(port);
    let _ = std::thread::Builder::new().name("deepseek-bridge".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let _ = answer(stream);
            });
        }
    });
}

fn answer(mut stream: TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let (mut length, mut token, mut origin) = (0usize, String::new(), String::new());
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "content-length" => length = v.trim().parse().unwrap_or(0),
                "x-scout-token" => token = v.trim().to_string(),
                "origin" => origin = v.trim().to_string(),
                _ => {}
            }
        }
    }
    let reply = |stream: &mut TcpStream, status: &str, body: &str| {
        let head = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
        stream.write_all(head.as_bytes()).and_then(|_| stream.write_all(body.as_bytes()))
    };
    // Only the extension: web pages can't send the token header without a preflight, which is never answered, and the
    // extension's own requests come from chrome-extension:// or with no origin.
    let from_page = !origin.is_empty() && !origin.starts_with("chrome-extension://") && !origin.starts_with("extension://");
    let known = path == "/deepseek/sync" || path == "/claude/sync";
    if from_page || method != "POST" || !known || length > 8 << 20 || !same(&token, &settings().token) {
        return reply(&mut stream, "403 Forbidden", "{}");
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let msg: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    if path == "/claude/sync" {
        return reply(&mut stream, "200 OK", &crate::claude_setup::sync(&msg).to_string());
    }
    let commands = sync(&msg);
    reply(&mut stream, "200 OK", &json!({ "commands": commands }).to_string())
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// One exchange with the extension: it brings the page's status and events and takes any commands. With nothing
/// happening it waits a while for one, so a request starts at once.
fn sync(msg: &Value) -> Vec<Value> {
    for e in msg.get("events").and_then(Value::as_array).into_iter().flatten() {
        if e.get("kind").and_then(Value::as_str) == Some("shutdown") {
            *SHUTDOWN.lock().unwrap_or_else(|e| e.into_inner()) = e.get("text").and_then(Value::as_str).map(str::to_string);
            continue;
        }
        deliver(e.clone());
    }
    let mut ext = EXT.lock().unwrap();
    ext.seen = Some(Instant::now());
    if let Some(s) = msg.get("status").filter(|s| !s.is_null()) {
        ext.status = s.clone();
    } else if msg.get("tabs").and_then(Value::as_u64) == Some(0) {
        ext.status = Value::Null;
    }
    let busy = CURRENT.lock().unwrap().is_some();
    if ext.commands.is_empty() && !busy && msg.get("events").and_then(Value::as_array).is_none_or(|e| e.is_empty()) {
        ext = EXT_WAKE.wait_timeout(ext, Duration::from_secs(10)).unwrap().0;
        ext.seen = Some(Instant::now());
    }
    std::mem::take(&mut ext.commands)
}

fn command(c: Value) {
    EXT.lock().unwrap().commands.push(c);
    EXT_WAKE.notify_all();
}

/// Whether the page is held under the overlay between requests.
static HOLDING: AtomicBool = AtomicBool::new(false);

/// Keeps the page's overlay up for the rest of the turn, through the pause and the tools, not just while a request runs.
fn hold() {
    HOLDING.store(true, Ordering::Relaxed);
    command(json!({ "type": "hold", "on": true }));
}

/// Lets the page go when the agent's turn is over. Does nothing if DeepSeek Web wasn't used.
pub fn release() {
    *SHUTDOWN.lock().unwrap_or_else(|e| e.into_inner()) = None;
    if HOLDING.swap(false, Ordering::Relaxed) {
        command(json!({ "type": "hold", "on": false }));
    }
}

/// Why the page gave itself back mid-turn, if it did: "override" (the user pressed Take over) or "signin" (the
/// account signed out). The rest of the turn's requests are refused until it ends.
static SHUTDOWN: Mutex<Option<String>> = Mutex::new(None);

/// The error that ends the turn, once the page has shut Scout out.
fn shut_out() -> Option<ApiError> {
    let why = SHUTDOWN.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
    HOLDING.store(false, Ordering::Relaxed);
    Some(shutdown_error(&why))
}

fn shutdown_error(why: &str) -> ApiError {
    match why {
        "signin" => ApiError::new("DeepSeek Web signed out, so Scout stopped this turn. Sign in again on chat.deepseek.com, then send a message to carry on.", 401),
        _ => ApiError::new("You took over the DeepSeek tab, so Scout stopped this turn. Send a message when you want it to carry on.", 0),
    }
}

// ---------- running a request ----------

/// The job whose events are wanted, and where they go.
static CURRENT: Mutex<Option<(u64, Sender<Value>)>> = Mutex::new(None);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// One request at a time: there's one page.
static BUSY: Mutex<()> = Mutex::new(());

fn deliver(e: Value) {
    let id = e.get("id").and_then(Value::as_u64);
    if let Some((current, tx)) = CURRENT.lock().unwrap().as_ref() {
        if id == Some(*current) {
            let _ = tx.send(e);
        }
    }
}

/// A DeepSeek chat that a Scout conversation is in, and how much of it has been sent.
struct Thread {
    chat: String,
    /// The hash of the conversation (without its system prompt) up to `len`, when the last request went out.
    hash: u64,
    len: usize,
}

static THREADS: Mutex<Vec<Thread>> = Mutex::new(Vec::new());

fn hash(messages: &[Message]) -> u64 {
    let mut h = DefaultHasher::new();
    for m in messages.iter().filter(|m| !matches!(m, Message::System { .. })) {
        serde_json::to_string(m).unwrap_or_default().hash(&mut h);
    }
    h.finish()
}

/// The DeepSeek chat this conversation continues, and the messages new since: the last request's, then its answer.
fn continuing(messages: &[Message]) -> Option<(String, usize)> {
    let threads = THREADS.lock().unwrap();
    threads.iter().rev().find_map(|t| {
        let fits = messages.len() > t.len + 1 && matches!(messages.get(t.len), Some(Message::Assistant { .. })) && hash(&messages[..t.len]) == t.hash;
        fits.then(|| (t.chat.clone(), t.len + 1))
    })
}

fn remember(chat: &str, messages: &[Message]) {
    let mut threads = THREADS.lock().unwrap();
    threads.retain(|t| t.chat != chat);
    threads.push(Thread { chat: chat.to_string(), hash: hash(messages), len: messages.len() });
    let extra = threads.len().saturating_sub(12);
    threads.drain(..extra);
}

const TOOL_RULES: &str = "# Tools

You're working through Scout, a coding agent on the user's computer, which runs tools for you. Call them in this XML format, with arguments that match each tool's parameters:

<action_calls>
<invoke name=\"tool_name\">
<parameter name=\"text_param\" string=\"true\">plain text, written as is</parameter>
<parameter name=\"other_param\" string=\"false\">a JSON value: number, true/false, array or object</parameter>
</invoke>
</action_calls>

- One action_calls block can hold several invokes. Stop writing after it: Scout runs them and replies with each result in a <tool_result> block.
- Never put the block in a code fence, and never write a <tool_result> yourself or guess what one will say.
- When the work is done, or you need the user, answer in plain text with no function calls.
- Use the tools to look at and change the user's files: you can't see them otherwise.

The tools, as JSON:";

/// The most MCP tools the first prompt lists, so a big MCP catalog doesn't crowd out the conversation.
const MAX_MCP_TOOLS: usize = 25;
/// The longest MCP tool description the prompt carries.
const MAX_MCP_DESCRIPTION: usize = 400;
/// The most of one tool result a DeepSeek Web turn carries, head and tail, to stay within what the page's composer takes.
const MAX_TOOL_RESULT: usize = 7_500;

fn tool_list(tools: &[Value]) -> String {
    let mut mcp = 0;
    let mut left_out = 0;
    let mut lines: Vec<String> = tools
        .iter()
        .filter_map(|t| {
            let f = t.get("function")?;
            let name = f.get("name")?;
            let mut description = f.get("description").cloned();
            if name.as_str().is_some_and(|n| n.starts_with("mcp__")) {
                mcp += 1;
                if mcp > MAX_MCP_TOOLS {
                    left_out += 1;
                    return None;
                }
                if let Some(d) = description.as_ref().and_then(Value::as_str).filter(|d| d.chars().count() > MAX_MCP_DESCRIPTION) {
                    description = Some(Value::String(format!("{}…", d.chars().take(MAX_MCP_DESCRIPTION).collect::<String>())));
                }
            }
            Some(json!({ "name": name, "description": description, "parameters": f.get("parameters") }).to_string())
        })
        .collect();
    if left_out > 0 {
        lines.push(format!("({left_out} more MCP tools aren't listed here.)"));
    }
    lines.join("\n")
}

/// A tool result cut to its head and tail when it's longer than a DeepSeek Web turn should carry.
fn clip(s: &str) -> String {
    let n = s.chars().count();
    if n <= MAX_TOOL_RESULT {
        return s.to_string();
    }
    let half = MAX_TOOL_RESULT / 2;
    let head: String = s.chars().take(half).collect();
    let tail: String = s.chars().skip(n - half).collect();
    format!("{head}\n\n… [{} characters cut] …\n\n{tail}", n - MAX_TOOL_RESULT)
}

fn user_text(c: &crate::api::Content) -> String {
    let n = c.images().len();
    let mut t = c.text();
    if n > 0 {
        t.push_str(&format!("\n\n[{n} image(s) were attached, but DeepSeek Web can't be sent images.]"));
    }
    t
}

/// Writes messages as one block of text, naming tool results by their call.
fn render(messages: &[Message], all: &[Message]) -> String {
    let names: std::collections::HashMap<&str, &str> = all
        .iter()
        .filter_map(|m| if let Message::Assistant { tool_calls: Some(c), .. } = m { Some(c) } else { None })
        .flatten()
        .map(|c| (c.id.as_str(), c.function.name.as_str()))
        .collect();
    let mut out = vec![];
    for m in messages {
        match m {
            Message::System { .. } => {}
            Message::User { content } => out.push(format!("[User]\n{}", user_text(content))),
            Message::Assistant { content, tool_calls, .. } => {
                let mut s = content.clone().unwrap_or_default();
                if let Some(calls) = tool_calls.as_ref().filter(|c| !c.is_empty()) {
                    s.push('\n');
                    s.push_str(&xml_calls(calls));
                }
                out.push(format!("[You]\n{}", s.trim()));
            }
            Message::Tool { tool_call_id, content } => {
                let name = names.get(tool_call_id.as_str()).copied().unwrap_or("tool");
                out.push(format!("<tool_result name=\"{name}\">\n{}\n</tool_result>", clip(content)));
            }
        }
    }
    out.join("\n\n")
}

/// The first message of a new DeepSeek chat: Scout's instructions, the tools, and the conversation so far.
fn first_prompt(messages: &[Message], tools: &[Value]) -> String {
    let system: String = messages.iter().filter_map(|m| if let Message::System { content } = m { Some(content.as_str()) } else { None }).collect::<Vec<_>>().join("\n\n");
    let mut p = String::new();
    if !system.is_empty() {
        p.push_str("[Instructions]\n");
        p.push_str(&system);
        p.push_str("\n\n");
    }
    if !tools.is_empty() {
        p.push_str(TOOL_RULES);
        p.push_str("\n<tools>\n");
        p.push_str(&tool_list(tools));
        p.push_str("\n</tools>\n\n");
    }
    p.push_str("[Conversation]\n\n");
    p.push_str(&render(messages, messages));
    p
}

fn next_prompt(new: &[Message], all: &[Message], tools: &[Value]) -> String {
    let mut p = render(new, all);
    if !tools.is_empty() && new.iter().any(|m| matches!(m, Message::Tool { .. })) {
        p.push_str("\n\nContinue. Use an <action_calls> block for any more tools, or answer in plain text when done.");
    }
    p
}

/// A short line for the page's own composer, which the full prompt replaces as it's sent.
fn display(new: &[Message]) -> String {
    let line = match new.iter().rev().find(|m| matches!(m, Message::User { .. })) {
        Some(Message::User { content }) => crate::prompt::strip_mode_note(&content.text()).to_string(),
        _ => format!("[Scout: {} tool result(s)]", new.iter().filter(|m| matches!(m, Message::Tool { .. })).count()),
    };
    let line: String = line.chars().take(300).collect();
    if line.trim().is_empty() { "Continue.".into() } else { line }
}

/// How much of the text can be shown yet: none of a tool call, nor what may be the start of one.
fn shown_len(s: &str) -> usize {
    let start = [s.find("<tool_call"), calls_start().find(s).map(|m| m.start())].into_iter().flatten().min();
    if let Some(i) = start {
        return i;
    }
    // A tag may be on its way: an unfinished one at the end is held back until it's clear what it is.
    match s.rfind('<') {
        Some(i) if s.len() - i < 24 && !s[i..].contains('>') && !s[i + 1..].starts_with(char::is_whitespace) => i,
        _ => s.len(),
    }
}

// ---------- tool calls as XML ----------
//
// The prompt asks for plain ASCII tags (`<action_calls>`, `<invoke>`, `<parameter>`). DeepSeek may still write its
// native DSML form (`<｜DSML｜function_calls>` …), so the parser reads both.

/// Where a block of tool calls starts: `<action_calls>`, or DSML with full-width or plain bars.
fn calls_start() -> &'static regex::Regex {
    static R: OnceLock<regex::Regex> = OnceLock::new();
    R.get_or_init(|| regex::Regex::new(r"<\s*action_calls\s*>|<[|｜\s]*DSML").unwrap())
}

/// Tool calls written in the prompt's XML format, for the conversation so far.
fn xml_calls(calls: &[ToolCall]) -> String {
    let mut s = String::from("<action_calls>\n");
    for c in calls {
        s.push_str(&format!("<invoke name=\"{}\">\n", c.function.name));
        let args: Value = serde_json::from_str(&c.function.arguments).unwrap_or(json!({}));
        for (k, v) in args.as_object().into_iter().flatten() {
            match v {
                Value::String(t) => s.push_str(&format!("<parameter name=\"{k}\" string=\"true\">{t}</parameter>\n")),
                v => s.push_str(&format!("<parameter name=\"{k}\" string=\"false\">{v}</parameter>\n")),
            }
        }
        s.push_str("</invoke>\n");
    }
    s.push_str("</action_calls>");
    s
}

/// Reads XML (or DSML) tool calls: `string="true"` values are text as written, the rest JSON. Returns the text before
/// the calls, and the calls.
///
/// Each call runs to its `</invoke>`. DeepSeek sometimes drops the `invoke name="…"` line (writing the opening tag
/// twice instead), so a call without a name goes to the one tool its parameters fit.
fn parse_calls(content: &str, tools: &[Value]) -> Option<(String, Vec<(String, Value)>)> {
    static NAME: OnceLock<regex::Regex> = OnceLock::new();
    static END: OnceLock<regex::Regex> = OnceLock::new();
    static PARAM: OnceLock<regex::Regex> = OnceLock::new();
    const BAR: &str = r"[|｜\s]*";
    // An optional DSML prefix, so `<invoke` and `<｜DSML｜invoke` both match.
    const PRE: &str = r"(?:[|｜\s]*DSML)?[|｜\s]*";
    let name = NAME.get_or_init(|| regex::Regex::new(&format!(r#"<{PRE}invoke\s+name\s*=\s*"([^"]+)"\s*>"#)).unwrap());
    let end = END.get_or_init(|| regex::Regex::new(&format!(r"<\s*/{PRE}invoke\s*>")).unwrap());
    let param = PARAM.get_or_init(|| regex::Regex::new(&format!(r#"(?s)<{PRE}parameter\s+name\s*=\s*"([^"]+)"(?:\s+string\s*=\s*"(true|false)")?\s*>(.*?)<\s*/(?:{BAR}DSML{BAR}(?:parameter)?|\s*parameter)\s*>"#)).unwrap());
    let start = calls_start().find(content)?.start();
    let mut calls = vec![];
    for part in end.split(&content[start..]) {
        let args: serde_json::Map<String, Value> = param
            .captures_iter(part)
            .map(|p| {
                let raw = &p[3];
                let value = match p.get(2).map(|m| m.as_str()) {
                    Some("true") => Value::String(raw.to_string()),
                    // Unmarked or JSON: a value that isn't valid JSON is kept as text.
                    _ => serde_json::from_str(raw.trim()).unwrap_or_else(|_| Value::String(raw.to_string())),
                };
                (p[1].to_string(), value)
            })
            .collect();
        let named = name.captures_iter(part).last().map(|c| c[1].trim().to_string());
        if named.is_none() && args.is_empty() {
            continue;
        }
        let Some(tool) = named.or_else(|| fitting_tool(&args, tools)) else { continue };
        calls.push(crate::tools::canonical(&tool, Value::Object(args)));
    }
    (!calls.is_empty()).then(|| (content[..start].trim().to_string(), calls))
}

/// The tool whose parameters these arguments fit: it takes all of them and has all it requires. With several, the one
/// with the fewest other parameters; with a tie, none, rather than a guess.
fn fitting_tool(args: &serde_json::Map<String, Value>, tools: &[Value]) -> Option<String> {
    let mut fits: Vec<(usize, String)> = tools
        .iter()
        .filter_map(|t| {
            let f = t.get("function")?;
            let props = f.pointer("/parameters/properties")?.as_object()?;
            let required = f.pointer("/parameters/required").and_then(Value::as_array).map(|r| r.iter().filter_map(Value::as_str).collect::<Vec<_>>()).unwrap_or_default();
            let fits = args.keys().all(|k| props.contains_key(k)) && required.iter().all(|r| args.contains_key(*r));
            let name = f.get("name")?.as_str()?.to_string();
            fits.then(|| (props.len() - args.len(), name))
        })
        .collect();
    fits.sort();
    match fits.as_slice() {
        [(_, only)] => Some(only.clone()),
        [(a, best), (b, _), ..] if a < b => Some(best.clone()),
        _ => None,
    }
}

enum Outcome {
    Done,
    ChatMissing,
}

/// When the last answer finished streaming.
static LAST_DONE: Mutex<Option<Instant>> = Mutex::new(None);
/// The shortest and longest gap after an answer before the next request, in milliseconds.
const AFTER_DONE_MS: (u64, u64) = (2_800, 5_200);

/// Waits a random time between the shortest and longest wait the user set, so requests don't come at a steady rhythm.
/// Stops early when the request is cancelled.
fn pause(s: &Settings, cancel: &AtomicBool) -> Result<(), ApiError> {
    let mut b = [0u8; 8];
    let seed = if getrandom::fill(&mut b).is_ok() {
        u64::from_le_bytes(b)
    } else {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| u64::from(d.subsec_nanos()))
    };
    let (min, max) = pacing_range(&s.pacing);
    let span_ms = u64::from(max - min) * 1000;
    let wait = Duration::from_millis(u64::from(min) * 1000 + seed % (span_ms + 1));
    let mut until = Instant::now() + wait;
    // Whatever the preset, the next request leaves at least 2.8-5.2 s after the last answer finished, so a fast tool
    // loop doesn't run into DeepSeek's rate limit.
    if let Some(done) = *LAST_DONE.lock().unwrap_or_else(|e| e.into_inner()) {
        let gap = Duration::from_millis(AFTER_DONE_MS.0 + (seed >> 32) % (AFTER_DONE_MS.1 - AFTER_DONE_MS.0 + 1));
        until = until.max(done + gap);
    }
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(ApiError::aborted());
        }
        if let Some(e) = shut_out() {
            return Err(e);
        }
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(());
        }
        std::thread::sleep(left.min(Duration::from_millis(100)));
    }
}

/// One request: sends the conversation and streams the answer, with tool calls read out of its text.
// ---------- the daily session limit ----------

/// How many Scout chats can use DeepSeek Web a day. chat.deepseek.com rate-limits accounts, and a coding agent's many
/// requests reach that quickly, so after this Scout asks the user to stop for the day.
pub const SESSIONS_PER_DAY: usize = 2;

fn sessions_file() -> PathBuf {
    crate::config::home().join("deepseek-web-sessions.json")
}

/// The chats counted today (UTC), by a hash of each one's first message.
fn sessions_today() -> Vec<String> {
    let v: Value = std::fs::read_to_string(sessions_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    if v.get("day").and_then(Value::as_str) != Some(crate::usage::today().as_str()) {
        return Vec::new();
    }
    v.get("chats").and_then(Value::as_array).into_iter().flatten().filter_map(|c| c.as_str().map(String::from)).collect()
}

/// Which chat this is: its first user message stays the same for the whole chat.
fn session_key(messages: &[Message]) -> String {
    let mut h = DefaultHasher::new();
    if let Some(first) = messages.iter().find(|m| matches!(m, Message::User { .. })) {
        serde_json::to_string(first).unwrap_or_default().hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

/// Counts this chat as one of today's sessions, or refuses it when the day's are used up. A chat already counted
/// carries on.
fn take_session(messages: &[Message]) -> Result<(), ApiError> {
    let key = session_key(messages);
    let mut chats = sessions_today();
    if chats.contains(&key) {
        return Ok(());
    }
    if chats.len() >= SESSIONS_PER_DAY {
        return Err(ApiError::new(
            format!(
                "That's {SESSIONS_PER_DAY} DeepSeek Web sessions today, so it's time to stop for the day. DeepSeek's website limits how much one account can do, and Scout's requests use that up fast. Give the account a rest: have a few normal chats with DeepSeek on chat.deepseek.com, then come back tomorrow. You can keep going in today's DeepSeek Web chats, or pick another model."
            ),
            403,
        ));
    }
    chats.push(key);
    let body = json!({ "day": crate::usage::today(), "chats": chats });
    let _ = std::fs::write(sessions_file(), body.to_string());
    Ok(())
}

/// `main` is the chat's own request, which counts toward the day's sessions; sub-agents and compaction ride along on it.
pub fn chat(model: &str, messages: &[Message], tools: &[Value], on: &mut dyn FnMut(Delta), cancel: &AtomicBool, main: bool) -> Result<StreamResult, ApiError> {
    let s = settings();
    if !s.enabled {
        return Err(ApiError::new("DeepSeek Web is off. Turn it on in Settings > Providers.", 400));
    }
    if main {
        take_session(messages)?;
    }
    let _one = BUSY.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(e) = shut_out() {
        return Err(e);
    }
    hold();
    pause(&s, cancel)?;
    let m = mode(model);
    let mut resume = continuing(messages);
    for _ in 0..2 {
        let (prompt, shown, chat) = match &resume {
            Some((chat, from)) => (next_prompt(&messages[*from..], messages, tools), display(&messages[*from..]), Some(chat.clone())),
            None => (first_prompt(messages, tools), display(messages), None),
        };
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let job = json!({ "id": id, "prompt": prompt, "display": shown, "thinking": m.thinking, "search": m.search, "chat": chat });
        let mut out = StreamResult::default();
        match run(id, &job, &mut out, on, cancel)? {
            (Outcome::ChatMissing, _) if resume.is_some() => {
                resume = None;
                continue;
            }
            (Outcome::ChatMissing, _) => return Err(ApiError::new("DeepSeek didn't open a new chat.", 0)),
            (Outcome::Done, chat_id) => {
                *LAST_DONE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
                if let Some(c) = chat_id.or(chat) {
                    remember(&c, messages);
                }
                return Ok(finish(out, tools, on));
            }
        }
    }
    Err(ApiError::new("DeepSeek didn't open the chat.", 0))
}

fn finish(mut out: StreamResult, tools: &[Value], on: &mut dyn FnMut(Delta)) -> StreamResult {
    let streamed = shown_len(&out.content);
    // The XML calls it's asked for (or DSML), or else Scout's own <tool_call> blocks.
    let found = parse_calls(&out.content, tools).or_else(|| crate::tools::recover_calls(&out.content, &out.reasoning, tools));
    if let Some((rest, calls)) = found {
        out.content = rest.trim().trim_end_matches("```xml").trim_end_matches("```").trim().to_string();
        let turn = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        out.tool_calls = calls
            .into_iter()
            .enumerate()
            .map(|(i, (name, args))| ToolCall { id: format!("ds_{turn}_{i}"), kind: "function".into(), extra_content: None, function: FunctionCall { name, arguments: args.to_string() } })
            .collect();
        out.finish_reason = Some("tool_calls".into());
    } else {
        // What was held back in case it was a tool call wasn't one.
        if out.content.len() > streamed {
            on(Delta::Text(&out.content[streamed..]));
        }
        out.finish_reason = Some("stop".into());
    }
    out
}

/// Sends a job and collects its answer. Returns how it ended and the DeepSeek chat it's in.
fn run(id: u64, job: &Value, out: &mut StreamResult, on: &mut dyn FnMut(Delta), cancel: &AtomicBool) -> Result<(Outcome, Option<String>), ApiError> {
    let (tx, rx): (Sender<Value>, Receiver<Value>) = channel();
    *CURRENT.lock().unwrap() = Some((id, tx));
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            *CURRENT.lock().unwrap() = None;
        }
    }
    let _clear = Clear;
    {
        let ext = EXT.lock().unwrap();
        if !ext.seen.is_some_and(|t| t.elapsed() < Duration::from_secs(25)) {
            return Err(ApiError::new("The Scout for DeepSeek extension isn't connected. Open chat.deepseek.com in the browser that has it, and keep the tab open.", 503));
        }
        if ext.status.is_null() {
            return Err(ApiError::new("Open chat.deepseek.com in the browser that has the Scout extension, and sign in.", 503));
        }
    }
    command(json!({ "type": "start", "job": job }));
    let stop = || command(json!({ "type": "stop" }));
    let mut chat = None;
    let mut confirmed = false;
    let mut last = Instant::now();
    let mut shown = 0;
    // While a CAPTCHA waits for the user, the request doesn't time out.
    let mut challenge = false;
    loop {
        if cancel.load(Ordering::Relaxed) {
            stop();
            return Err(ApiError::aborted());
        }
        if challenge {
            last = Instant::now();
        }
        let event = match rx.recv_timeout(Duration::from_millis(120)) {
            Ok(e) => e,
            Err(_) => {
                if last.elapsed() > Duration::from_secs(300) {
                    stop();
                    return Err(ApiError::new("DeepSeek stopped answering.", 0));
                }
                continue;
            }
        };
        last = Instant::now();
        let text = event.get("text").and_then(Value::as_str).unwrap_or_default();
        match event.get("kind").and_then(Value::as_str).unwrap_or_default() {
            "text" => {
                out.content.push_str(text);
                let upto = shown_len(&out.content);
                if upto > shown {
                    on(Delta::Text(&out.content[shown..upto]));
                    shown = upto;
                }
            }
            "reasoning" => {
                out.reasoning.push_str(text);
                on(Delta::Reasoning(text));
            }
            "chat" => chat = Some(text.to_string()),
            "sent" => confirmed = true,
            // hCaptcha wants a person. Scout pings them, and waits while they solve it on the page.
            "challenge" => {
                challenge = true;
                crate::notify::toast("Scout", "DeepSeek needs you to solve a CAPTCHA. Solve it on the chat.deepseek.com tab in your browser.");
            }
            "challenge-clear" => challenge = false,
            // An answer to the short line alone, without Scout's instructions and tools, would be no use.
            "done" if !confirmed => return Err(ApiError::new("DeepSeek Web: DeepSeek answered without Scout's prompt, so Scout didn't use the answer. The site may have changed; check for a Scout update.", 0)),
            "done" => return Ok((Outcome::Done, chat)),
            "error" if text == "chat-missing" => return Ok((Outcome::ChatMissing, None)),
            "error" if text == "override" => return Err(shutdown_error("override")),
            "error" if text == "signin" => return Err(signin()),
            "error" => return Err(ApiError::new(format!("DeepSeek Web: {text}"), if text.contains("rate") || text.contains("busy") { 429 } else { 0 })),
            _ => {}
        }
    }
}

fn signin() -> ApiError {
    ApiError::new("DeepSeek Web isn't signed in. Sign in on chat.deepseek.com in the browser that has the Scout extension.", 401)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Content;

    fn user(t: &str) -> Message {
        Message::User { content: Content::Text(t.into()) }
    }

    #[test]
    fn holds_back_what_may_be_a_tool_call() {
        assert_eq!(shown_len("Let me look."), 12);
        assert_eq!(shown_len("Let me look.\n<tool"), 13);
        assert_eq!(shown_len("Let me look.\n<tool_call>{\"name\""), 13);
        assert_eq!(shown_len("a < b"), 5);
    }

    #[test]
    fn a_conversation_continues_its_chat() {
        let first = vec![Message::System { content: "Be Scout.".into() }, user("hi")];
        remember("chat-1", &first);
        let mut next = first.clone();
        next.push(Message::Assistant { content: Some("Hello".into()), tool_calls: None, reasoning_content: None, model: None, provider: None });
        next.push(user("again"));
        assert_eq!(continuing(&next), Some(("chat-1".into(), 3)));
        // A changed system prompt doesn't matter; changed history does.
        next[0] = Message::System { content: "Be Scout today.".into() };
        assert_eq!(continuing(&next), Some(("chat-1".into(), 3)));
        next[1] = user("hello");
        assert_eq!(continuing(&next), None);
    }

    #[test]
    fn xml_calls_are_read() {
        let text = "On it.\n<action_calls>\n<invoke name=\"view\">\n<parameter name=\"paths\" string=\"false\">[\"README.md\"]</parameter>\n</invoke>\n<invoke name=\"bash\">\n<parameter name=\"command\" string=\"true\">echo \"a < b\"</parameter>\n</invoke>\n</action_calls>";
        let (rest, calls) = parse_calls(text, &[]).unwrap();
        assert_eq!(rest, "On it.");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].1["paths"], json!(["README.md"]));
        assert_eq!(calls[1].1["command"], "echo \"a < b\"");
        assert_eq!(shown_len(text), "On it.\n".len());
    }

    #[test]
    fn long_tool_results_and_mcp_catalogs_are_cut() {
        let long = "x".repeat(MAX_TOOL_RESULT * 3);
        assert!(clip(&long).chars().count() < MAX_TOOL_RESULT + 100);
        assert_eq!(clip("short"), "short");
        let mut tools: Vec<Value> = (0..MAX_MCP_TOOLS + 5).map(|i| json!({"type":"function","function":{"name": format!("mcp__s__t{i}"), "description": "d".repeat(2000)}})).collect();
        tools.push(json!({"type":"function","function":{"name":"bash","description":"run"}}));
        let list = tool_list(&tools);
        assert_eq!(list.lines().filter(|l| l.contains("mcp__")).count(), MAX_MCP_TOOLS);
        assert!(list.contains("\"bash\"") && list.contains("5 more MCP tools"));
        assert!(list.lines().all(|l| l.len() < 1000));
    }

    #[test]
    fn dsml_calls_are_read() {
        let text = "Let me look.\n<｜DSML｜function_calls>\n<｜DSML｜invoke name=\"view\">\n<｜DSML｜parameter name=\"paths\" string=\"false\">[\"README.md\", \"Cargo.toml\"]</｜DSML｜parameter>\n</｜DSML｜invoke>\n<｜DSML｜invoke name=\"list_dir\">\n<｜DSML｜parameter name=\"path\" string=\"true\">src</｜DSML｜parameter>\n</｜DSML｜invoke>\n</｜DSML｜function_calls>";
        let (rest, calls) = parse_calls(text, &[]).unwrap();
        assert_eq!(rest, "Let me look.");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1], ("list_dir".to_string(), json!({"path": "src"})));
        assert_eq!(shown_len(text), "Let me look.\n".len());
        // Plain bars, as some outputs have them.
        let plain = "<|DSML|function_calls><|DSML|invoke name=\"bash\"><|DSML|parameter name=\"command\" string=\"true\">npm test</|DSML|parameter></|DSML|invoke></|DSML|function_calls>";
        assert_eq!(parse_calls(plain, &[]).unwrap().1[0].1["command"], "npm test");
    }

    #[test]
    fn a_parameter_closed_without_its_word_is_still_read() {
        // As DeepSeek wrote it: the closing tag is `</｜DSML｜｜>`, with no `parameter` in it.
        let text = "<｜DSML｜function_calls><｜DSML｜invoke name=\"bash\"><｜DSML｜parameter name=\"command\" string=\"true\">cat src/deepseek/extension/manifest.json</｜DSML｜｜></｜DSML｜invoke></｜DSML｜function_calls>";
        let (_, calls) = parse_calls(text, &[]).unwrap();
        assert_eq!(calls[0].0, "bash");
        assert_eq!(calls[0].1["command"], "cat src/deepseek/extension/manifest.json");
    }

    #[test]
    fn a_call_without_its_name_goes_to_the_tool_it_fits() {
        // As DeepSeek wrote it: function_calls twice, and no invoke name.
        let text = "<｜DSML｜function_calls><｜DSML｜function_calls><｜DSML｜parameter name=\"command\" string=\"true\">git diff --stat</｜DSML｜parameter><｜DSML｜parameter name=\"description\" string=\"true\">Summarize in-progress changes</｜DSML｜parameter></｜DSML｜invoke></｜DSML｜function_calls>";
        let tools = crate::tools::specs(None, crate::tools::EditTool::StrReplace);
        let (_, calls) = parse_calls(text, &tools).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "bash");
        assert_eq!(calls[0].1["command"], "git diff --stat");
    }

    #[test]
    fn earlier_calls_are_written_as_xml_and_read_back() {
        let call = ToolCall { id: "1".into(), kind: "function".into(), extra_content: None, function: FunctionCall { name: "grep".into(), arguments: json!({"pattern": "fn main", "limit": 5}).to_string() } };
        let (_, calls) = parse_calls(&xml_calls(&[call]), &[]).unwrap();
        assert_eq!(calls[0].0, "grep");
        assert_eq!(calls[0].1["pattern"], "fn main");
        assert_eq!(calls[0].1["limit"], 5);
    }

    #[test]
    fn tool_calls_come_back_as_calls() {
        let tools = vec![json!({"type":"function","function":{"name":"view","parameters":{"type":"object","properties":{"path":{"type":"string"}}}}})];
        let out = StreamResult { content: "Looking.\n<tool_call>{\"name\": \"view\", \"arguments\": {\"path\": \"a.rs\"}}</tool_call>".into(), ..Default::default() };
        let r = finish(out, &tools, &mut |_| {});
        assert_eq!(r.content, "Looking.");
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.tool_calls[0].function.name, "view");
    }
}
