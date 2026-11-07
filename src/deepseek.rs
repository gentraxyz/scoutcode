//! DeepSeek Web: the user's own chat.deepseek.com account as a provider, with no API key.
//!
//! A driver script (`deepseek/driver.js`) runs inside chat.deepseek.com and sends each request through the page the way
//! the user would, then streams the answer back. It runs in one of two places:
//!
//! - **Scout's DeepSeek window**, a hidden window of Scout's own, where the user signs in once. Scout starts jobs and
//!   reads what came back by running small scripts in the page (`eval`), so the page gets no access to Scout's IPC.
//! - **The Scout for DeepSeek extension**, in the user's own Chrome or Edge, on an open chat.deepseek.com tab. It reaches
//!   Scout through a small server on 127.0.0.1 that only answers requests with this PC's token.
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
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Wry};

pub const PROVIDER: &str = "deepseek-web";
const SITE: &str = "https://chat.deepseek.com/";
const WINDOW: &str = "deepseek";
/// The extension's local server tries this port first.
const PORT: u16 = 47615;

pub const DRIVER: &str = include_str!("deepseek/driver.js");
const EXTENSION: [(&str, &str); 7] = [
    ("manifest.json", include_str!("deepseek/extension/manifest.json")),
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

/// What's saved in ~/.scoutcode/deepseek-web.json.
#[derive(Clone)]
pub struct Settings {
    pub enabled: bool,
    /// "window" (Scout's own DeepSeek window) or "extension".
    pub via: String,
    /// What the extension has to send to the local server.
    pub token: String,
}

fn settings_file() -> PathBuf {
    crate::config::home().join("deepseek-web.json")
}

pub fn settings() -> Settings {
    let s: Value = std::fs::read_to_string(settings_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    let token = s.get("token").and_then(Value::as_str).filter(|t| t.len() >= 32).map(String::from).unwrap_or_else(new_token);
    Settings {
        enabled: s.get("enabled").and_then(Value::as_bool).unwrap_or(false),
        via: if s.get("via").and_then(Value::as_str) == Some("extension") { "extension".into() } else { "window".into() },
        token,
    }
}

fn save(s: &Settings) -> Result<(), String> {
    let body = json!({ "enabled": s.enabled, "via": s.via, "token": s.token });
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

/// Turns DeepSeek Web on or off, or changes where it runs.
pub fn set(enabled: Option<bool>, via: Option<String>) -> Result<(), String> {
    let mut s = settings();
    if let Some(e) = enabled {
        s.enabled = e;
    }
    if let Some(v) = via {
        s.via = if v == "extension" { "extension".into() } else { "window".into() };
    }
    save(&s)?;
    if s.enabled && s.via == "extension" {
        serve();
        write_extension(&s)?;
    }
    if !s.enabled {
        if let Some(w) = app().and_then(|a| a.get_webview_window(WINDOW)) {
            let _ = w.destroy();
        }
    }
    Ok(())
}

// ---------- the app ----------

static APP: OnceLock<AppHandle<Wry>> = OnceLock::new();

fn app() -> Option<&'static AppHandle<Wry>> {
    APP.get()
}

/// Called once the app is up.
pub fn init(app: AppHandle<Wry>) {
    let _ = APP.set(app);
    let s = settings();
    if s.enabled && s.via == "extension" {
        serve();
        let _ = write_extension(&s);
    }
}

/// What Settings shows.
pub fn status() -> Value {
    let s = settings();
    // Asks the page itself, so a sign-in made in the window shows at once. Without the window, it's opened hidden to look.
    if s.enabled && s.via == "window" {
        match app().and_then(|a| a.get_webview_window(WINDOW)) {
            Some(w) => {
                if let Some(signed) = eval(&w, "window.__scoutDS ? JSON.stringify(window.__scoutDS.status()) : 'null'").and_then(|v| v.get("signedIn").and_then(Value::as_bool)) {
                    *WINDOW_SIGNED_IN.lock().unwrap() = signed;
                }
            }
            None => check_window(),
        }
    }
    let ext = EXT.lock().unwrap();
    let connected = ext.seen.is_some_and(|t| t.elapsed() < Duration::from_secs(20));
    let window = app().and_then(|a| a.get_webview_window(WINDOW)).is_some();
    let signed_in = if s.via == "extension" { connected && ext.status.get("signedIn").and_then(Value::as_bool).unwrap_or(false) } else { *WINDOW_SIGNED_IN.lock().unwrap() };
    json!({
        "enabled": s.enabled,
        "via": s.via,
        "signedIn": signed_in,
        "windowOpen": window,
        "extensionConnected": connected,
        "extensionTab": connected && ext.status.get("path").is_some(),
        "extensionPath": extension_dir().to_string_lossy(),
        "port": *SERVING.lock().unwrap(),
        "warning": WARNING,
    })
}

/// Last thing the window said about signing in, so Settings can show it without asking the page.
static WINDOW_SIGNED_IN: Mutex<bool> = Mutex::new(false);

// ---------- Scout's DeepSeek window ----------

fn window(show: bool) -> Result<WebviewWindow<Wry>, String> {
    let app = app().ok_or("Scout isn't ready yet.")?;
    if let Some(w) = app.get_webview_window(WINDOW) {
        if show {
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
        }
        return Ok(w);
    }
    WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::External(SITE.parse().unwrap()))
        .title("DeepSeek · Scout")
        .inner_size(1060.0, 820.0)
        .center()
        .visible(show)
        .initialization_script(DRIVER)
        // A hidden page's timers and streams slow down, which would stall the answer.
        .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Disabled)
        // DeepSeek and its sign-in providers. Everything is https.
        .on_navigation(|url| url.scheme() == "https")
        .build()
        .map_err(|e| format!("Couldn't open DeepSeek's window: {e}"))
}

/// Shows the window, to sign in or to watch.
pub fn open_window() -> Result<(), String> {
    window(true).map(|_| ())
}

/// Runs a script in the window's page and returns what it returned.
fn eval(w: &WebviewWindow<Wry>, js: &str) -> Option<Value> {
    let (tx, rx) = channel();
    w.eval_with_callback(js, move |s| {
        let _ = tx.send(s);
    })
    .ok()?;
    let raw = rx.recv_timeout(Duration::from_secs(5)).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    // The script returns JSON text, which comes back as a JSON string.
    match v {
        Value::String(s) => serde_json::from_str(&s).ok(),
        v => Some(v),
    }
}

fn drain(w: &WebviewWindow<Wry>) -> Option<Value> {
    let v = eval(w, "window.__scoutDS ? JSON.stringify(window.__scoutDS.drain()) : 'null'")?;
    if let Some(s) = v.get("signedIn").and_then(Value::as_bool) {
        *WINDOW_SIGNED_IN.lock().unwrap() = s;
    }
    (!v.is_null()).then_some(v)
}

/// Checks whether the window is signed in, opening it hidden if it isn't open, for Settings.
pub fn check_window() {
    static CHECKING: AtomicBool = AtomicBool::new(false);
    if CHECKING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        struct Done;
        impl Drop for Done {
            fn drop(&mut self) {
                CHECKING.store(false, Ordering::SeqCst);
            }
        }
        let _done = Done;
        let Ok(w) = window(false) else { return };
        let until = Instant::now() + Duration::from_secs(30);
        while Instant::now() < until {
            if drain(&w).is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
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
    if from_page || method != "POST" || path != "/deepseek/sync" || length > 8 << 20 || !same(&token, &settings().token) {
        return reply(&mut stream, "403 Forbidden", "{}");
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let msg: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
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

You're working through Scout, a coding agent on the user's computer, which runs tools for you. Call them in your own DSML function-call format, with arguments that match each tool's parameters:

<｜DSML｜function_calls>
<｜DSML｜invoke name=\"tool_name\">
<｜DSML｜parameter name=\"text_param\" string=\"true\">plain text, written as is</｜DSML｜parameter>
<｜DSML｜parameter name=\"other_param\" string=\"false\">a JSON value: number, true/false, array or object</｜DSML｜parameter>
</｜DSML｜invoke>
</｜DSML｜function_calls>

- One function_calls block can hold several invokes. Stop writing after it: Scout runs them and replies with each result in a <tool_result> block.
- Never put the block in a code fence, and never write a <tool_result> yourself or guess what one will say.
- When the work is done, or you need the user, answer in plain text with no function calls.
- Use the tools to look at and change the user's files: you can't see them otherwise.

The tools, as JSON:";

fn tool_list(tools: &[Value]) -> String {
    tools
        .iter()
        .filter_map(|t| {
            let f = t.get("function")?;
            Some(json!({ "name": f.get("name")?, "description": f.get("description"), "parameters": f.get("parameters") }).to_string())
        })
        .collect::<Vec<_>>()
        .join("\n")
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
                    s.push_str(&dsml(calls));
                }
                out.push(format!("[You]\n{}", s.trim()));
            }
            Message::Tool { tool_call_id, content } => {
                let name = names.get(tool_call_id.as_str()).copied().unwrap_or("tool");
                out.push(format!("<tool_result name=\"{name}\">\n{content}\n</tool_result>"));
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
        p.push_str("\n\nContinue. Use DSML function calls for any more tools, or answer in plain text when done.");
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
    let start = [s.find("<tool_call"), dsml_start().find(s).map(|m| m.start())].into_iter().flatten().min();
    if let Some(i) = start {
        return i;
    }
    // A tag may be on its way: an unfinished one at the end is held back until it's clear what it is.
    match s.rfind('<') {
        Some(i) if s.len() - i < 24 && !s[i..].contains('>') && !s[i + 1..].starts_with(char::is_whitespace) => i,
        _ => s.len(),
    }
}

// ---------- DSML, DeepSeek's own tool-call format ----------

/// Where a DSML block starts: `<｜DSML｜function_calls>`, with full-width or plain bars.
fn dsml_start() -> &'static regex::Regex {
    static R: OnceLock<regex::Regex> = OnceLock::new();
    R.get_or_init(|| regex::Regex::new(r"<[|｜\s]*DSML").unwrap())
}

/// Tool calls written as DSML, for the conversation so far.
fn dsml(calls: &[ToolCall]) -> String {
    let mut s = String::from("<｜DSML｜function_calls>\n");
    for c in calls {
        s.push_str(&format!("<｜DSML｜invoke name=\"{}\">\n", c.function.name));
        let args: Value = serde_json::from_str(&c.function.arguments).unwrap_or(json!({}));
        for (k, v) in args.as_object().into_iter().flatten() {
            match v {
                Value::String(t) => s.push_str(&format!("<｜DSML｜parameter name=\"{k}\" string=\"true\">{t}</｜DSML｜parameter>\n")),
                v => s.push_str(&format!("<｜DSML｜parameter name=\"{k}\" string=\"false\">{v}</｜DSML｜parameter>\n")),
            }
        }
        s.push_str("</｜DSML｜invoke>\n");
    }
    s.push_str("</｜DSML｜function_calls>");
    s
}

/// Reads DSML tool calls: `string="true"` values are text as written, the rest JSON. Returns the text before the
/// calls, and the calls.
///
/// Each call runs to its `</｜DSML｜invoke>`. DeepSeek sometimes drops the `invoke name="…"` line (writing
/// `function_calls` twice instead), so a call without a name goes to the one tool its parameters fit.
fn dsml_calls(content: &str, tools: &[Value]) -> Option<(String, Vec<(String, Value)>)> {
    static NAME: OnceLock<regex::Regex> = OnceLock::new();
    static END: OnceLock<regex::Regex> = OnceLock::new();
    static PARAM: OnceLock<regex::Regex> = OnceLock::new();
    const BAR: &str = r"[|｜\s]*";
    let name = NAME.get_or_init(|| regex::Regex::new(&format!(r#"<{BAR}DSML{BAR}invoke\s+name\s*=\s*"([^"]+)"\s*>"#)).unwrap());
    let end = END.get_or_init(|| regex::Regex::new(&format!(r"<\s*/{BAR}DSML{BAR}invoke\s*>")).unwrap());
    let param = PARAM.get_or_init(|| regex::Regex::new(&format!(r#"(?s)<{BAR}DSML{BAR}parameter\s+name\s*=\s*"([^"]+)"(?:\s+string\s*=\s*"(true|false)")?\s*>(.*?)<\s*/{BAR}DSML{BAR}parameter\s*>"#)).unwrap());
    let start = dsml_start().find(content)?.start();
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

/// One request: sends the conversation and streams the answer, with tool calls read out of its text.
pub fn chat(model: &str, messages: &[Message], tools: &[Value], on: &mut dyn FnMut(Delta), cancel: &AtomicBool) -> Result<StreamResult, ApiError> {
    let s = settings();
    if !s.enabled {
        return Err(ApiError::new("DeepSeek Web is off. Turn it on in Settings > Providers.", 400));
    }
    let _one = BUSY.lock().unwrap_or_else(|e| e.into_inner());
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
        match run(&s, id, &job, &mut out, on, cancel)? {
            (Outcome::ChatMissing, _) if resume.is_some() => {
                resume = None;
                continue;
            }
            (Outcome::ChatMissing, _) => return Err(ApiError::new("DeepSeek didn't open a new chat.", 0)),
            (Outcome::Done, chat_id) => {
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
    // DSML, which it's asked for, or else Scout's own <tool_call> blocks.
    let found = dsml_calls(&out.content, tools).or_else(|| crate::tools::recover_calls(&out.content, &out.reasoning, tools));
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
fn run(s: &Settings, id: u64, job: &Value, out: &mut StreamResult, on: &mut dyn FnMut(Delta), cancel: &AtomicBool) -> Result<(Outcome, Option<String>), ApiError> {
    let (tx, rx): (Sender<Value>, Receiver<Value>) = channel();
    *CURRENT.lock().unwrap() = Some((id, tx));
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            *CURRENT.lock().unwrap() = None;
        }
    }
    let _clear = Clear;
    let w = if s.via == "extension" {
        let ext = EXT.lock().unwrap();
        if !ext.seen.is_some_and(|t| t.elapsed() < Duration::from_secs(25)) {
            return Err(ApiError::new("The Scout for DeepSeek extension isn't connected. Open chat.deepseek.com in the browser that has it, or switch DeepSeek Web to Scout's window in Settings > Providers.", 503));
        }
        if ext.status.is_null() {
            return Err(ApiError::new("Open chat.deepseek.com in the browser that has the Scout extension, and sign in.", 503));
        }
        drop(ext);
        command(json!({ "type": "start", "job": job }));
        None
    } else {
        let w = window(false).map_err(|e| ApiError::new(e, 0))?;
        let ready = Instant::now() + Duration::from_secs(40);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(ApiError::aborted());
            }
            if let Some(st) = drain(&w) {
                if st.get("signIn").and_then(Value::as_bool) == Some(true) {
                    return Err(signin());
                }
                if st.get("busy").and_then(Value::as_bool) != Some(true) {
                    break;
                }
                let _ = w.eval("window.__scoutDS && window.__scoutDS.stop()");
            }
            if Instant::now() > ready {
                return Err(ApiError::new("DeepSeek's page didn't load. Check your connection, or open it from Settings > Providers > DeepSeek Web.", 0));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        w.eval(format!("window.__scoutDS.start({job})")).map_err(|e| ApiError::new(e.to_string(), 0))?;
        Some(w)
    };
    let stop = || match &w {
        Some(w) => {
            let _ = w.eval("window.__scoutDS && window.__scoutDS.stop()");
        }
        None => command(json!({ "type": "stop" })),
    };
    let mut chat = None;
    let mut confirmed = false;
    let mut last = Instant::now();
    let mut shown = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            stop();
            return Err(ApiError::aborted());
        }
        if let Some(w) = &w {
            for e in drain(w).and_then(|v| v.get("events").and_then(Value::as_array).cloned()).into_iter().flatten() {
                deliver(e);
            }
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
            // An answer to the short line alone, without Scout's instructions and tools, would be no use.
            "done" if !confirmed => return Err(ApiError::new("DeepSeek Web: DeepSeek answered without Scout's prompt, so Scout didn't use the answer. The site may have changed; check for a Scout update.", 0)),
            "done" => return Ok((Outcome::Done, chat)),
            "error" if text == "chat-missing" => return Ok((Outcome::ChatMissing, None)),
            "error" if text == "signin" => return Err(signin()),
            "error" => return Err(ApiError::new(format!("DeepSeek Web: {text}"), if text.contains("rate") || text.contains("busy") { 429 } else { 0 })),
            _ => {}
        }
    }
}

fn signin() -> ApiError {
    ApiError::new(
        if settings().via == "extension" {
            "DeepSeek Web isn't signed in. Sign in on chat.deepseek.com in the browser that has the Scout extension."
        } else {
            "DeepSeek Web isn't signed in. Open Settings > Providers > DeepSeek Web and sign in."
        },
        401,
    )
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
    fn dsml_calls_are_read() {
        let text = "Let me look.\n<｜DSML｜function_calls>\n<｜DSML｜invoke name=\"view\">\n<｜DSML｜parameter name=\"paths\" string=\"false\">[\"README.md\", \"Cargo.toml\"]</｜DSML｜parameter>\n</｜DSML｜invoke>\n<｜DSML｜invoke name=\"list_dir\">\n<｜DSML｜parameter name=\"path\" string=\"true\">src</｜DSML｜parameter>\n</｜DSML｜invoke>\n</｜DSML｜function_calls>";
        let (rest, calls) = dsml_calls(text, &[]).unwrap();
        assert_eq!(rest, "Let me look.");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1], ("list_dir".to_string(), json!({"path": "src"})));
        assert_eq!(shown_len(text), "Let me look.\n".len());
        // Plain bars, as some outputs have them.
        let plain = "<|DSML|function_calls><|DSML|invoke name=\"bash\"><|DSML|parameter name=\"command\" string=\"true\">npm test</|DSML|parameter></|DSML|invoke></|DSML|function_calls>";
        assert_eq!(dsml_calls(plain, &[]).unwrap().1[0].1["command"], "npm test");
    }

    #[test]
    fn a_call_without_its_name_goes_to_the_tool_it_fits() {
        // As DeepSeek wrote it: function_calls twice, and no invoke name.
        let text = "<｜DSML｜function_calls><｜DSML｜function_calls><｜DSML｜parameter name=\"command\" string=\"true\">git diff --stat</｜DSML｜parameter><｜DSML｜parameter name=\"description\" string=\"true\">Summarize in-progress changes</｜DSML｜parameter></｜DSML｜invoke></｜DSML｜function_calls>";
        let tools = crate::tools::specs(None, crate::tools::EditTool::StrReplace);
        let (_, calls) = dsml_calls(text, &tools).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "bash");
        assert_eq!(calls[0].1["command"], "git diff --stat");
    }

    #[test]
    fn earlier_calls_are_written_as_dsml_and_read_back() {
        let call = ToolCall { id: "1".into(), kind: "function".into(), extra_content: None, function: FunctionCall { name: "grep".into(), arguments: json!({"pattern": "fn main", "limit": 5}).to_string() } };
        let (_, calls) = dsml_calls(&dsml(&[call]), &[]).unwrap();
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
