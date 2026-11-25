//! Scout's built-in browser: a real webview inside the window's right-hand panel (the Dock's Browser tab), which the
//! user can browse in themselves and, with Settings > Browser on, the agent can use too.
//!
//! The agent starts with one small tool, `browser`. Calling it turns the browser on for that chat, and from the next
//! request the `browser_*` tools are offered too. They're added at the very end of the tool list and stay for the rest of
//! the chat (whether they're on is read from the chat's own history), so the prompt cache misses once, when they're
//! turned on, and never again.
//!
//! Pages are read as text, never as screenshots: a compact outline of the page where each element the agent can act on
//! has a ref such as [e12]. After an action, the agent gets only what changed on the page, not the whole outline again.
//!
//! The page is driven with scripts run in it (see `LIB`), which works the same on WebView2, WebKit and WebKitGTK. The
//! webview has no IPC: it isn't in any capability, so a site in it can't reach Scout's commands.

use crate::api::Message;
use crate::tools::ToolResult;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard, OnceLock, mpsc};
use std::time::{Duration, Instant};
use tauri::webview::{NewWindowResponse, PageLoadEvent};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewBuilder, WebviewUrl};

/// Each tab is a webview of its own, labelled with this and its id.
const LABEL: &str = "browser-";
/// The most of a page outline one result holds, in characters (about 4k tokens).
const MAX_SNAPSHOT: usize = 16_000;
/// The most lines an action's "what changed" holds.
const MAX_CHANGED: usize = 80;
/// The most tabs the browser keeps open; opening another closes the oldest one.
const MAX_TABS: usize = 24;

static APP: OnceLock<AppHandle> = OnceLock::new();
/// The Browse page is on screen: what the user says there is about the browser (see `browse_note`).
static BROWSE: AtomicBool = AtomicBool::new(false);

#[derive(Clone)]
struct Tab {
    id: u64,
    url: String,
    title: String,
    started: u64,
    finished: u64,
    zoom: f64,
}

#[derive(Default)]
struct State {
    /// Where the browser's space is (the Dock's Browser tab or the Browse page), in the window's CSS pixels.
    bounds: Option<[f64; 4]>,
    /// That space is on screen.
    shown: bool,
    /// The user took over: the agent's browser tools wait until they hand it back.
    user: bool,
    /// One of the agent's browser tools is running.
    agent: bool,
    /// The open tabs, in the order the tab strip shows them.
    tabs: Vec<Tab>,
    /// The tab in front, which the agent uses too.
    active: u64,
    next: u64,
    /// Addresses of closed tabs, newest last, for Ctrl+Shift+T.
    closed: Vec<String>,
    /// The page outline the agent saw last, to say what an action changed.
    last: Vec<String>,
    last_url: String,
}

impl State {
    fn tab(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }
    fn front(&self) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == self.active)
    }
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

/// Tells the window where the browser is and who has it.
fn emit() {
    let Some(app) = APP.get() else { return };
    let _ = app.emit("browser", status());
}

fn tab_label(id: u64) -> String {
    format!("{LABEL}{id}")
}

fn tab_view(id: u64) -> Option<Webview> {
    APP.get()?.get_webview(&tab_label(id))
}

/// The tab in front's webview.
fn view() -> Option<Webview> {
    let id = state().active;
    tab_view(id)
}

/// The tab in front's page loads: how many started, and how many finished.
fn loads() -> (u64, u64) {
    state().front().map(|t| (t.started, t.finished)).unwrap_or((0, 0))
}

/// The tab in front's webview, with a first tab made if there's none. Tabs share Scout's WebView2 profile, so sign-ins
/// stay between restarts.
fn ensure() -> Result<Webview, String> {
    if let Some(w) = view() {
        return Ok(w);
    }
    open_tab(None).map(|(_, w)| w)
}

/// Opens a tab in front, on an address or a blank page.
fn open_tab(url: Option<Url>) -> Result<(u64, Webview), String> {
    let app = APP.get().ok_or("The browser isn't ready yet.")?;
    let window = app.get_window("main").ok_or("Scout's window isn't open.")?;
    let id = {
        let mut s = state();
        s.next += 1;
        let id = s.next;
        s.tabs.push(Tab { id, url: url.as_ref().map(|u| u.to_string()).unwrap_or_default(), title: String::new(), started: 0, finished: 0, zoom: 1.0 });
        id
    };
    let handle = app.clone();
    let builder = WebviewBuilder::new(tab_label(id), WebviewUrl::External(url.unwrap_or_else(|| "about:blank".parse().unwrap())))
        // A link that opens a new window opens in a new tab, unless the agent is working: then it stays in this one,
        // so the agent sees where its click went.
        .on_new_window(move |url, _| {
            let agent = state().agent;
            let h = handle.clone();
            std::thread::spawn(move || {
                if agent {
                    if let Some(w) = h.get_webview(&tab_label(id)) {
                        let _ = w.navigate(url);
                    }
                } else {
                    let _ = open_tab(Some(url));
                }
            });
            NewWindowResponse::Deny
        })
        .on_page_load(move |_, p| {
            let mut s = state();
            if let Some(t) = s.tab(id) {
                match p.event() {
                    PageLoadEvent::Started => t.started += 1,
                    PageLoadEvent::Finished => t.finished = t.started,
                }
                t.url = p.url().to_string();
            }
            drop(s);
            emit();
        })
        .on_document_title_changed(move |_, title| {
            if let Some(t) = state().tab(id) {
                t.title = title;
            }
            emit();
        });
    let bounds = state().bounds.unwrap_or([0.0, 0.0, 1024.0, 768.0]);
    let wv = match window.add_child(builder, LogicalPosition::new(bounds[0], bounds[1]), LogicalSize::new(bounds[2].max(1.0), bounds[3].max(1.0))) {
        Ok(w) => w,
        Err(e) => {
            state().tabs.retain(|t| t.id != id);
            return Err(format!("Could not open the browser: {e}"));
        }
    };
    // Too many tabs: the oldest one goes.
    let extra = {
        let mut s = state();
        s.active = id;
        if s.tabs.len() > MAX_TABS { Some(s.tabs.remove(0)) } else { None }
    };
    if let Some(t) = extra {
        if let Some(w) = tab_view(t.id) {
            let _ = w.close();
        }
    }
    place();
    emit();
    Ok((id, wv))
}

/// Shows the tab in front where the window has room for it, and hides the rest.
fn place() {
    let (ids, active, bounds, shown) = {
        let s = state();
        (s.tabs.iter().map(|t| t.id).collect::<Vec<_>>(), s.active, s.bounds, s.shown)
    };
    for id in ids {
        let Some(wv) = tab_view(id) else { continue };
        match bounds {
            Some([x, y, w, h]) if id == active && shown => {
                let _ = wv.set_position(LogicalPosition::new(x, y));
                let _ = wv.set_size(LogicalSize::new(w, h));
                let _ = wv.show();
            }
            _ => {
                let _ = wv.hide();
            }
        }
    }
}

// ---------- what the window asks for ----------

/// The browser's space in the window moved, resized, showed or hid.
pub fn set_bounds(x: f64, y: f64, w: f64, h: f64, shown: bool) {
    {
        let mut s = state();
        if w > 1.0 && h > 1.0 {
            s.bounds = Some([x, y, w, h]);
        }
        s.shown = shown && w > 1.0 && h > 1.0;
    }
    place();
}

/// The user typed an address or pressed back, forward, reload or stop.
pub fn user_go(to: &str) -> Result<(), String> {
    let wv = ensure()?;
    match to {
        "back" => wv.eval("history.back()").map_err(|e| e.to_string()),
        "forward" => wv.eval("history.forward()").map_err(|e| e.to_string()),
        "reload" => wv.reload().map_err(|e| e.to_string()),
        "stop" => wv.eval("window.stop()").map_err(|e| e.to_string()),
        "" => Ok(()),
        url => {
            let url = parse_url(url)?;
            // Shown in the address bar at once, before the page starts loading.
            let id = state().active;
            if let Some(t) = state().tab(id) {
                t.url = url.to_string();
            }
            emit();
            wv.navigate(url).map_err(|e| e.to_string())
        }
    }
}

/// A new tab in front, on an address if there is one.
pub fn new_tab(url: Option<&str>) -> Result<u64, String> {
    let url = match url.map(str::trim).filter(|u| !u.is_empty()) {
        Some(u) => Some(parse_url(u)?),
        None => None,
    };
    open_tab(url).map(|(id, _)| id)
}

/// Brings a tab to the front.
pub fn select_tab(id: u64) {
    {
        let mut s = state();
        if s.tabs.iter().any(|t| t.id == id) {
            s.active = id;
        }
    }
    place();
    emit();
}

/// Closes a tab. The one beside it comes to the front if it was in front.
pub fn close_tab(id: u64) {
    {
        let mut s = state();
        let Some(i) = s.tabs.iter().position(|t| t.id == id) else { return };
        let t = s.tabs.remove(i);
        if !t.url.is_empty() && t.url != "about:blank" {
            s.closed.push(t.url);
            if s.closed.len() > 20 {
                s.closed.remove(0);
            }
        }
        if s.active == id {
            s.active = s.tabs.get(i).or_else(|| i.checked_sub(1).and_then(|j| s.tabs.get(j))).map(|t| t.id).unwrap_or(0);
        }
    }
    if let Some(w) = tab_view(id) {
        let _ = w.close();
    }
    place();
    emit();
}

/// Opens the tab closed last again.
pub fn reopen_tab() -> Result<(), String> {
    let Some(url) = state().closed.pop() else { return Ok(()) };
    new_tab(Some(&url)).map(|_| ())
}

/// Moves a tab to another place in the strip.
pub fn move_tab(id: u64, to: usize) {
    {
        let mut s = state();
        let Some(i) = s.tabs.iter().position(|t| t.id == id) else { return };
        let t = s.tabs.remove(i);
        let to = to.min(s.tabs.len());
        s.tabs.insert(to, t);
    }
    emit();
}

/// Zooms the tab in front, where 1 is 100%.
pub fn set_zoom(factor: f64) -> Result<(), String> {
    let factor = factor.clamp(0.25, 5.0);
    let wv = ensure()?;
    wv.set_zoom(factor).map_err(|e| e.to_string())?;
    let id = state().active;
    if let Some(t) = state().tab(id) {
        t.zoom = factor;
    }
    emit();
    Ok(())
}

/// Finds text in the page in front, forwards or backwards, and says whether it's there.
pub fn find(text: &str, back: bool) -> Result<bool, String> {
    let wv = view().ok_or("No page is open.")?;
    if text.is_empty() {
        let _ = wv.eval("getSelection&&getSelection().removeAllRanges()");
        return Ok(false);
    }
    let v = eval_raw(&wv, format!("JSON.stringify(window.find({},false,{back},true,false,false,false))", json!(text)), Duration::from_secs(3))?;
    Ok(v == Value::Bool(true))
}

/// The user takes the browser over, or hands it back to the agent.
pub fn set_user(on: bool) {
    state().user = on;
    emit();
}

/// The Browse page came on screen or went away.
pub fn set_browse(on: bool) {
    BROWSE.store(on, Ordering::Relaxed);
}

/// Starts the note that says a message came from the Browse page.
pub const BROWSE_MARK: &str = "\n\n[Browse page: ";

/// What the agent is told when a message comes from the Browse page: the user is looking at the browser, so web tasks
/// go there. Appended to the message like the permission mode, and kept out of the transcript the same way.
pub fn browse_note() -> Option<String> {
    if !BROWSE.load(Ordering::Relaxed) {
        return None;
    }
    let s = state();
    let page = match s.front() {
        Some(t) if !t.url.is_empty() && t.url != "about:blank" => format!(" The tab in front shows {}{}.", if t.title.is_empty() { String::new() } else { format!("\"{}\" — ", t.title) }, t.url),
        _ => " No page is open yet.".to_string(),
    };
    let n = s.tabs.len();
    Some(format!(
        "{BROWSE_MARK}the user sent this from Scout's Browse page, where the built-in browser fills the window and this chat sits beside it. It has {n} tab{}.{page} Use the browser_* tools for anything on the web (they're on for this chat), not web_fetch; \"this page\" or \"this site\" means the tab in front.]",
        if n == 1 { "" } else { "s" }
    ))
}

/// Whether a message in this chat came from the Browse page, which turns the browser's tools on for it, whatever
/// Settings > Browser says: chatting beside the browser is asking Scout to use it.
pub fn browsed(messages: &[Message]) -> bool {
    messages.iter().any(|m| matches!(m, Message::User { content } if content.text().contains(BROWSE_MARK)))
}

pub fn status() -> Value {
    let s = state();
    let tabs: Vec<Value> = s.tabs.iter().map(|t| json!({ "id": t.id, "url": t.url, "title": t.title, "loading": t.finished < t.started, "zoom": t.zoom })).collect();
    let (url, title, loading, zoom) = s.front().map(|t| (t.url.clone(), t.title.clone(), t.finished < t.started, t.zoom)).unwrap_or((String::new(), String::new(), false, 1.0));
    json!({ "url": url, "title": title, "loading": loading, "zoom": zoom, "agent": s.agent, "user": s.user, "open": !s.tabs.is_empty(), "tabs": tabs, "active": s.active, "canReopen": !s.closed.is_empty() })
}

fn parse_url(raw: &str) -> Result<Url, String> {
    let raw = raw.trim();
    // "mailto:x" or "javascript:x" has a scheme of its own; "localhost:5173" only a port.
    let own_scheme = raw.split_once(':').is_some_and(|(s, rest)| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphabetic()) && !rest.starts_with(|c: char| c.is_ascii_digit()));
    let with_scheme = if raw.contains("://") || own_scheme {
        raw.to_string()
    } else if raw.starts_with("localhost") || raw.starts_with("127.") || raw.starts_with("[::1]") {
        format!("http://{raw}")
    } else if !raw.contains('.') && !raw.contains(':') || raw.contains(' ') {
        // Not an address: search for it.
        format!("https://duckduckgo.com/?q={}", url::form_urlencoded::byte_serialize(raw.as_bytes()).collect::<String>())
    } else {
        format!("https://{raw}")
    };
    let url = Url::parse(&with_scheme).map_err(|e| format!("{raw} isn't a web address ({e})."))?;
    if !matches!(url.scheme(), "http" | "https" | "file" | "about") {
        return Err(format!("The browser only opens http, https and file addresses, not {}:.", url.scheme()));
    }
    Ok(url)
}

// ---------- the agent's tools ----------

/// The one browser tool every chat has (with Settings > Browser on). It's small, so it costs little in chats that never
/// use it, and it doesn't change, so the cache holds.
pub fn activation_spec() -> Value {
    json!({"type":"function","function":{"name":"browser","description":"Turn on Scout's built-in browser: a real web browser in the app's side panel that the user can see. Use it to open and use websites: click, fill forms, read pages that need JavaScript or a sign-in, or check a web app you're building (e.g. on localhost). After this call, the browser_* tools are available. To just read a page's text, web_fetch is cheaper.",
        "parameters":{"type":"object","properties":{
            "url":{"type":"string","description":"Optional: an address to open right away"}
        }}}})
}

const TOOLS: &[&str] = &["browser_navigate", "browser_snapshot", "browser_click", "browser_type", "browser_select", "browser_press", "browser_scroll", "browser_wait", "browser_eval", "browser_tabs"];

pub fn is_tool(name: &str) -> bool {
    name == "browser" || TOOLS.contains(&name)
}

/// The tools the agent gets once it has called `browser` in this chat, in a steady order.
pub fn specs() -> Vec<Value> {
    let f = |name: &str, description: &str, parameters: Value| json!({"type":"function","function":{"name":name,"description":description,"parameters":parameters}});
    let r = json!({"type":"string","description":"An element's ref from the page outline, e.g. e12"});
    vec![
        f("browser_navigate", "Open an address in the browser, or go back, forward or reload. Returns the page outline.",
            json!({"type":"object","properties":{"url":{"type":"string","description":"An address, or back, forward or reload"}},"required":["url"]})),
        f("browser_snapshot", "Read the page as an outline: headings, text, and the elements you can act on, each with a ref like [e12]. With query, only the lines that contain it.",
            json!({"type":"object","properties":{"query":{"type":"string","description":"Optional: only lines containing this text (case-insensitive)"}}})),
        f("browser_click", "Click an element. Returns what changed on the page, or the new page's outline if it navigated.",
            json!({"type":"object","properties":{"ref":r},"required":["ref"]})),
        f("browser_type", "Type into a text field, replacing what's in it (or adding to it with append). submit presses Enter afterwards.",
            json!({"type":"object","properties":{"ref":r,"text":{"type":"string"},"append":{"type":"boolean"},"submit":{"type":"boolean"}},"required":["ref","text"]})),
        f("browser_select", "Pick an option in a dropdown (select), by its value or its text.",
            json!({"type":"object","properties":{"ref":r,"option":{"type":"string"}},"required":["ref","option"]})),
        f("browser_press", "Press a key, e.g. Enter, Escape, Tab, ArrowDown, on an element or whatever has focus.",
            json!({"type":"object","properties":{"key":{"type":"string"},"ref":r},"required":["key"]})),
        f("browser_scroll", "Scroll an element into view, or the page up or down. The outline already holds the whole page, so this is for pages that load more as you scroll.",
            json!({"type":"object","properties":{"ref":r,"direction":{"type":"string","enum":["up","down","top","bottom"]}}})),
        f("browser_wait", "Wait until some text is on the page (up to 20 s), or for a number of seconds.",
            json!({"type":"object","properties":{"text":{"type":"string"},"seconds":{"type":"number"}}})),
        f("browser_eval", "Run JavaScript in the page and get its result as JSON (a promise is awaited, up to 10 s). For checking a web app you're building; prefer the other tools for using a site.",
            json!({"type":"object","properties":{"script":{"type":"string","description":"An expression, or statements ending in return inside an async function body"}},"required":["script"]})),
        f("browser_tabs", "The browser's tabs. list shows them; new opens one in front (on url if given); select brings one to the front; close closes one. The other browser tools act on the tab in front.",
            json!({"type":"object","properties":{"action":{"type":"string","enum":["list","new","select","close"]},"url":{"type":"string","description":"For new: an address to open"},"id":{"type":"integer","description":"For select and close: the tab's id from list"}},"required":["action"]})),
    ]
}

/// The tabs as the agent sees them, one a line.
fn tab_list() -> String {
    let s = state();
    if s.tabs.is_empty() {
        return "No tabs are open.".into();
    }
    s.tabs
        .iter()
        .map(|t| {
            let url = if t.url.is_empty() { "about:blank" } else { &t.url };
            format!("[{}] {}{url}{}", t.id, if t.title.is_empty() { String::new() } else { format!("{} — ", t.title) }, if t.id == s.active { " (in front)" } else { "" })
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether this chat has turned the browser on: it called `browser` at some point. Read from the history, so it holds
/// across restarts and turns, and goes away with compaction like everything else that was summarised.
pub fn active(messages: &[Message]) -> bool {
    messages.iter().any(|m| matches!(m, Message::Assistant { tool_calls: Some(calls), .. } if calls.iter().any(|c| c.function.name == "browser")))
}

/// One line for the transcript.
pub fn label(name: &str, a: &Value) -> String {
    let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    match name {
        "browser" | "browser_navigate" => s("url"),
        "browser_snapshot" => s("query"),
        "browser_click" | "browser_scroll" => s("ref"),
        "browser_type" => format!("{} ← {}", s("ref"), s("text").chars().take(60).collect::<String>()),
        "browser_select" => format!("{} ← {}", s("ref"), s("option")),
        "browser_press" => s("key"),
        "browser_wait" => if s("text").is_empty() { a.get("seconds").map(|v| format!("{v} s")).unwrap_or_default() } else { s("text") },
        "browser_eval" => s("script").chars().take(120).collect(),
        "browser_tabs" => [s("action"), s("url"), a.get("id").map(|v| v.to_string()).unwrap_or_default()].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" "),
        _ => String::new(),
    }
}

/// An action that submits, buys, sends or deletes needs the user's OK first. Says what it would do, or None.
pub fn risky(name: &str, a: &Value) -> Option<String> {
    let wv = view()?;
    let r = a.get("ref").and_then(Value::as_str).unwrap_or("");
    let expr = match name {
        "browser_click" => format!("S.risky({})", json!(r)),
        "browser_type" if a.get("submit").and_then(Value::as_bool) == Some(true) => format!("S.submitsForm({})", json!(r)),
        "browser_press" if a.get("key").and_then(Value::as_str).is_some_and(|k| k.eq_ignore_ascii_case("enter")) => format!("S.submitsForm({})", json!(r)),
        _ => return None,
    };
    call(&wv, &expr).ok()?.as_str().map(String::from)
}

struct Busy;
impl Busy {
    fn new() -> Self {
        state().agent = true;
        emit();
        Busy
    }
}
impl Drop for Busy {
    fn drop(&mut self) {
        state().agent = false;
        emit();
    }
}

/// Runs one of the agent's browser tools.
pub fn run(name: &str, a: &Value, cancel: &AtomicBool) -> ToolResult {
    if state().user {
        return ToolResult::err("The user has taken over the browser and is using it themselves. Wait for them, or ask them in your reply to hand it back.", "user has it");
    }
    let _busy = Busy::new();
    match run_inner(name, a, cancel) {
        Ok((out, summary)) => ToolResult::ok(out, summary),
        Err(e) => ToolResult::err(e.clone(), e.chars().take(100).collect::<String>()),
    }
}

const GUIDE: &str = "The browser is on, in Scout's side panel or its Browse page, where the user can watch. Tools: browser_navigate, browser_snapshot, browser_click, browser_type, browser_select, browser_press, browser_scroll, browser_wait, browser_eval, browser_tabs. It has tabs; the tools act on the tab in front. Pages come as a text outline; [e12] marks an element you can act on by its ref, and refs stay the same while the element is on the page. Actions return only what changed. Clicks that submit, buy, send or delete ask the user first. Never type passwords or payment details: ask the user to do that in the panel.";

fn run_inner(name: &str, a: &Value, cancel: &AtomicBool) -> Result<(String, String), String> {
    let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let r = s("ref");
    // Opening a page brings the panel up, so the user sees what the agent does. Other actions leave the panel as the
    // user has it.
    if matches!(name, "browser" | "browser_navigate" | "browser_tabs") {
        if let Some(app) = APP.get() {
            let _ = app.emit("browser-show", ());
        }
    }
    if name == "browser_tabs" {
        let id = a.get("id").and_then(Value::as_u64);
        let has = |id: u64| state().tabs.iter().any(|t| t.id == id);
        return match s("action").as_str() {
            "" | "list" => Ok((tab_list(), "listed".into())),
            "new" => {
                new_tab(Some(&s("url")))?;
                let wv = ensure()?;
                let page = if s("url").trim().is_empty() {
                    String::new()
                } else {
                    settle(&wv, 0, Duration::from_secs(3), cancel);
                    let (head, lines) = snapshot(&wv)?;
                    format!("\n\n{head}\n{}", cap(&lines))
                };
                Ok((format!("{}{page}", tab_list()), "new tab".into()))
            }
            "select" => match id.filter(|&i| has(i)) {
                Some(i) => {
                    select_tab(i);
                    let wv = ensure()?;
                    let (head, lines) = snapshot(&wv)?;
                    Ok((format!("{}\n\n{head}\n{}", tab_list(), cap(&lines)), "switched".into()))
                }
                None => Err(format!("There's no tab with that id.\n{}", tab_list())),
            },
            "close" => match id.filter(|&i| has(i)) {
                Some(i) => {
                    close_tab(i);
                    Ok((tab_list(), "closed".into()))
                }
                None => Err(format!("There's no tab with that id.\n{}", tab_list())),
            },
            other => Err(format!("Unknown action {other}: use list, new, select or close.")),
        };
    }
    let wv = ensure()?;
    match name {
        "browser" => {
            if s("url").is_empty() {
                let page = page_line(&wv);
                Ok((format!("{GUIDE}\n\n{page}"), "on".into()))
            } else {
                let out = navigate(&wv, &s("url"), cancel)?;
                Ok((format!("{GUIDE}\n\n{out}"), "on".into()))
            }
        }
        "browser_navigate" => navigate(&wv, &s("url"), cancel).map(|o| (o, "opened".into())),
        "browser_snapshot" => {
            wait_loaded(&wv, Duration::from_secs(10), cancel);
            let (head, lines) = snapshot(&wv)?;
            let q = s("query").to_lowercase();
            if q.is_empty() {
                Ok((format!("{head}\n{}", cap(&lines)), format!("{} lines", lines.len())))
            } else {
                let hits: Vec<String> = lines.iter().filter(|l| l.to_lowercase().contains(&q)).cloned().collect();
                let n = hits.len();
                Ok((if hits.is_empty() { format!("{head}\nNothing on the page contains \"{}\".", s("query")) } else { format!("{head}\n{}", cap(&hits)) }, format!("{n} matches")))
            }
        }
        "browser_click" => act(&wv, &format!("S.click({})", json!(r)), "Clicked", cancel),
        "browser_type" => {
            let typed = act(&wv, &format!("S.type({},{},{})", json!(r), json!(s("text")), a.get("append").and_then(Value::as_bool).unwrap_or(false)), "Typed into", cancel)?;
            if a.get("submit").and_then(Value::as_bool) == Some(true) { act(&wv, &format!("S.key({},'Enter')", json!(r)), "Typed and pressed Enter in", cancel) } else { Ok(typed) }
        }
        "browser_select" => act(&wv, &format!("S.select({},{})", json!(r), json!(s("option"))), "Picked in", cancel),
        "browser_press" => act(&wv, &format!("S.key({},{})", json!(r), json!(s("key"))), "Pressed in", cancel),
        "browser_scroll" => {
            let dir = if s("direction").is_empty() { "down".to_string() } else { s("direction") };
            let v = call(&wv, &format!("S.scroll({},{})", json!(r), json!(dir)))?;
            std::thread::sleep(Duration::from_millis(400));
            let changed = changes(&wv)?;
            Ok((format!("Scrolled. {}\n{changed}", v.as_str().unwrap_or("")), "scrolled".into()))
        }
        "browser_wait" => {
            let text = s("text");
            if text.is_empty() {
                let secs = a.get("seconds").and_then(Value::as_f64).unwrap_or(1.0).clamp(0.0, 20.0);
                sleep(Duration::from_secs_f64(secs), cancel);
                return Ok((format!("Waited {secs} s.\n{}", changes(&wv)?), "waited".into()));
            }
            let until = Instant::now() + Duration::from_secs(20);
            while Instant::now() < until && !cancel.load(Ordering::Relaxed) {
                if call(&wv, &format!("S.has({})", json!(text))).ok() == Some(Value::Bool(true)) {
                    return Ok((format!("\"{text}\" is on the page.\n{}", changes(&wv)?), "found".into()));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(format!("\"{text}\" didn't show up on the page within 20 s."))
        }
        "browser_eval" => eval_script(&wv, &s("script"), cancel).map(|o| (o, "ran".into())),
        _ => Err(format!("Unknown browser tool {name}.")),
    }
}

fn sleep(d: Duration, cancel: &AtomicBool) {
    let until = Instant::now() + d;
    while Instant::now() < until && !cancel.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn page_line(wv: &Webview) -> String {
    let url = wv.url().map(|u| u.to_string()).unwrap_or_default();
    let title = state().front().map(|t| t.title.clone()).unwrap_or_default();
    format!("Page: {}{url}", if title.is_empty() { String::new() } else { format!("{title} — ") })
}

fn navigate(wv: &Webview, to: &str, cancel: &AtomicBool) -> Result<String, String> {
    let before = loads().0;
    match to.trim() {
        "back" => wv.eval("history.back()").map_err(|e| e.to_string())?,
        "forward" => wv.eval("history.forward()").map_err(|e| e.to_string())?,
        "reload" => wv.reload().map_err(|e| e.to_string())?,
        "" => return Err("url is empty.".into()),
        url => wv.navigate(parse_url(url)?).map_err(|e| e.to_string())?,
    }
    settle(wv, before, Duration::from_secs(3), cancel);
    let (head, lines) = snapshot(wv)?;
    Ok(format!("{head}\n{}", cap(&lines)))
}

/// An action on the page, then what it changed: the new page's outline if it navigated, or else only the lines that
/// are new since the outline the agent saw last.
fn act(wv: &Webview, expr: &str, verb: &str, cancel: &AtomicBool) -> Result<(String, String), String> {
    wait_loaded(wv, Duration::from_secs(10), cancel);
    let before = loads().0;
    let what = call(wv, expr)?;
    let what = what.as_str().unwrap_or("").to_string();
    settle(wv, before, Duration::from_millis(700), cancel);
    let navigated = loads().0 > before;
    let (head, lines) = snapshot_raw(wv)?;
    let url = current_url(wv);
    let moved = navigated || state().last_url != url;
    let out = if moved {
        remember(&lines, &url);
        format!("{verb} {what}. It went to another page.\n{head}\n{}", cap(&lines))
    } else {
        let diff = diff(&lines);
        remember(&lines, &url);
        format!("{verb} {what}.\n{diff}")
    };
    Ok((out, verb.to_lowercase()))
}

fn changes(wv: &Webview) -> Result<String, String> {
    let (head, lines) = snapshot_raw(wv)?;
    let url = current_url(wv);
    let out = if state().last_url != url { format!("{head}\n{}", cap(&lines)) } else { diff(&lines) };
    remember(&lines, &url);
    Ok(out)
}

fn current_url(wv: &Webview) -> String {
    call(wv, "location.href").ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| wv.url().map(|u| u.to_string()).unwrap_or_default())
}

fn remember(lines: &[String], url: &str) {
    let mut s = state();
    s.last = lines.to_vec();
    s.last_url = url.to_string();
}

/// The lines that weren't in the outline the agent saw last, and how many went away.
fn diff(lines: &[String]) -> String {
    let s = state();
    let old: HashSet<&String> = s.last.iter().collect();
    let new: HashSet<&String> = lines.iter().collect();
    let added: Vec<&String> = lines.iter().filter(|l| !old.contains(l)).collect();
    let gone = s.last.iter().filter(|l| !new.contains(l)).count();
    if added.is_empty() && gone == 0 {
        return "Nothing visible changed on the page.".into();
    }
    let mut out = String::new();
    if !added.is_empty() {
        out.push_str("New on the page:\n");
        for l in added.iter().take(MAX_CHANGED) {
            out.push_str(l);
            out.push('\n');
        }
        if added.len() > MAX_CHANGED {
            out.push_str(&format!("…and {} more lines (browser_snapshot shows all).\n", added.len() - MAX_CHANGED));
        }
    }
    if gone > 0 {
        out.push_str(&format!("{gone} line{} went away.", if gone == 1 { "" } else { "s" }));
    }
    out.trim_end().to_string()
}

fn cap(lines: &[String]) -> String {
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        if out.len() + l.len() + 1 > MAX_SNAPSHOT {
            out.push_str(&format!("…{} more lines. browser_snapshot with a query finds a part of the page.", lines.len() - i));
            break;
        }
        out.push_str(l);
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// The page outline, which then counts as what the agent saw last.
fn snapshot(wv: &Webview) -> Result<(String, Vec<String>), String> {
    let (head, lines) = snapshot_raw(wv)?;
    remember(&lines, &current_url(wv));
    Ok((head, lines))
}

fn snapshot_raw(wv: &Webview) -> Result<(String, Vec<String>), String> {
    let v = call(wv, "S.snapshot()")?;
    let title = v.get("title").and_then(Value::as_str).unwrap_or("");
    let url = v.get("url").and_then(Value::as_str).unwrap_or("");
    let text = v.get("text").and_then(Value::as_str).unwrap_or("");
    let lines: Vec<String> = text.lines().map(String::from).collect();
    let head = format!("Page: {}{url}", if title.is_empty() { String::new() } else { format!("{title} — ") });
    Ok((head, if lines.is_empty() { vec!["(the page is empty)".into()] } else { lines }))
}

/// Waits for a page load the last action may have started, then for it to finish.
fn settle(wv: &Webview, before: u64, start_within: Duration, cancel: &AtomicBool) {
    let t = Instant::now();
    while t.elapsed() < start_within && loads().0 == before && !cancel.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(50));
    }
    wait_loaded(wv, Duration::from_secs(20), cancel);
    // Let scripts that run after load draw.
    sleep(Duration::from_millis(250), cancel);
}

fn wait_loaded(wv: &Webview, max: Duration, cancel: &AtomicBool) {
    let t = Instant::now();
    while t.elapsed() < max && !cancel.load(Ordering::Relaxed) {
        let (started, finished) = loads();
        let loading = finished < started;
        if !loading && eval_raw(wv, "document.readyState".into(), Duration::from_secs(2)).ok().is_some_and(|v| v != "loading") {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn eval_raw(wv: &Webview, js: String, timeout: Duration) -> Result<Value, String> {
    let (tx, rx) = mpsc::channel();
    wv.eval_with_callback(js, move |s| {
        let _ = tx.send(s);
    })
    .map_err(|e| e.to_string())?;
    let s = rx.recv_timeout(timeout).map_err(|_| "The page didn't answer in time (it may be busy or still loading).".to_string())?;
    // The result arrives as JSON; a JSON string that holds JSON is unwrapped once more.
    let v: Value = serde_json::from_str(&s).unwrap_or(Value::String(s));
    Ok(match v {
        Value::String(inner) => serde_json::from_str(&inner).unwrap_or(Value::String(inner)),
        v => v,
    })
}

/// Runs one of LIB's functions in the page. Exceptions come back as the error.
fn call(wv: &Webview, expr: &str) -> Result<Value, String> {
    let js = format!("(()=>{{try{{{LIB}\nconst S=window.__scout;return JSON.stringify({{ok:({expr})}})}}catch(e){{return JSON.stringify({{error:String(e&&e.message||e)}})}}}})()");
    let v = eval_raw(wv, js, Duration::from_secs(10))?;
    if let Some(e) = v.get("error").and_then(Value::as_str) {
        return Err(e.to_string());
    }
    Ok(v.get("ok").cloned().unwrap_or(Value::Null))
}

fn eval_script(wv: &Webview, script: &str, cancel: &AtomicBool) -> Result<String, String> {
    let mut id = [0u8; 6];
    let _ = getrandom::fill(&mut id);
    let id: String = id.iter().map(|b| format!("{b:02x}")).collect();
    let body = if script.contains("return") { script.to_string() } else { format!("return ({script}\n)") };
    let start = format!(
        "(()=>{{const R=(window.__scoutEval=window.__scoutEval||{{}});R[{id:?}]=null;(async()=>{{{body}\n}})().then(v=>{{R[{id:?}]={{ok:v===undefined?null:v}}}},e=>{{R[{id:?}]={{error:String(e&&e.stack||e)}}}});return 'started'}})()"
    );
    eval_raw(wv, start, Duration::from_secs(10))?;
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until && !cancel.load(Ordering::Relaxed) {
        let v = eval_raw(wv, format!("(()=>{{const R=window.__scoutEval||{{}};const v=R[{id:?}];if(!v)return null;delete R[{id:?}];try{{return JSON.stringify(v)}}catch(e){{return JSON.stringify({{ok:String(v.ok)}})}}}})()"), Duration::from_secs(5))?;
        if !v.is_null() {
            if let Some(e) = v.get("error").and_then(Value::as_str) {
                return Err(e.chars().take(2000).collect());
            }
            let out = serde_json::to_string_pretty(v.get("ok").unwrap_or(&Value::Null)).unwrap_or_default();
            return Ok(if out.len() > 8000 { format!("{}… (cut at 8000 characters)", out.chars().take(8000).collect::<String>()) } else { out });
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("The script didn't finish within 10 s.".into())
}

/// What runs in the page: the outline, and the actions on its elements. Defined once per page.
const LIB: &str = r#"if(!window.__scout){(()=>{
const W=window,refs=new Map(),keys=new WeakMap();let n=0;
const clean=(t,max=120)=>{t=String(t||'').replace(/\s+/g,' ').trim();return t.length>max?t.slice(0,max-1)+'…':t};
const vis=(e)=>{if(e.checkVisibility)return e.checkVisibility({visibilityProperty:true});const s=getComputedStyle(e);return s.display!=='none'&&s.visibility!=='hidden'};
const INPUT={checkbox:'checkbox',radio:'radio',submit:'button',button:'button',reset:'button',image:'button',range:'slider',file:'file upload',color:'color'};
const ROLES=new Set(['button','link','checkbox','radio','tab','menuitem','menuitemcheckbox','menuitemradio','option','switch','combobox','textbox','searchbox','slider','spinbutton','treeitem']);
const SKIP=new Set(['SCRIPT','STYLE','NOSCRIPT','TEMPLATE','HEAD','META','LINK','SVG','CANVAS','VIDEO','AUDIO','OBJECT','PATH']);
const BLOCK=new Set(['DIV','P','LI','UL','OL','DL','DT','DD','TR','TABLE','TBODY','THEAD','SECTION','ARTICLE','HEADER','FOOTER','NAV','MAIN','ASIDE','FORM','FIELDSET','BLOCKQUOTE','PRE','FIGURE','FIGCAPTION','BR','HR','DIALOG','LEGEND','CAPTION']);
function role(e){const r=e.getAttribute('role');if(r&&ROLES.has(r))return r;const t=e.tagName;
if(t==='A')return e.hasAttribute('href')?'link':null;
if(t==='INPUT'){const ty=(e.type||'text').toLowerCase();if(ty==='hidden')return null;return INPUT[ty]||'textbox'}
if(t==='BUTTON'||t==='SUMMARY')return 'button';if(t==='SELECT')return 'select';if(t==='TEXTAREA')return 'textbox';
if(e.isContentEditable&&!(e.parentElement&&e.parentElement.isContentEditable))return 'textbox';
if(e.hasAttribute('onclick'))return 'clickable';return null}
function name(e){const a=e.getAttribute('aria-label');if(a)return clean(a);
const lb=e.getAttribute('aria-labelledby');if(lb){const t=lb.split(/\s+/).map(i=>{const x=document.getElementById(i);return x?x.innerText:''}).join(' ');if(t.trim())return clean(t)}
if(e.labels&&e.labels.length)return clean(e.labels[0].innerText);
if(e.tagName==='INPUT'&&['submit','button','reset'].includes(e.type))return clean(e.value);
if(e.tagName==='INPUT'||e.tagName==='TEXTAREA'||e.tagName==='SELECT')return clean(e.getAttribute('placeholder')||e.getAttribute('title')||e.name||'');
const t=clean(e.innerText||e.textContent);if(t)return t;
const i=e.querySelector&&e.querySelector('img[alt],[aria-label],[title]');
return clean(e.getAttribute('title')||(i&&(i.getAttribute('alt')||i.getAttribute('aria-label')||i.getAttribute('title')))||'')}
function ref(e){let k=keys.get(e);if(!k){k='e'+(++n);keys.set(e,k);refs.set(k,new WeakRef(e))}return k}
function href(h){try{const u=new URL(h,location.href);const s=u.origin===location.origin?u.pathname+u.search+u.hash:u.href;return clean(s,80)}catch(_){return clean(h,80)}}
function line(e,r){let s='['+ref(e)+'] '+r;const nm=name(e);if(nm)s+=' "'+nm+'"';
if(r==='link'){const h=e.getAttribute('href');if(h&&!h.startsWith('javascript:')&&h!=='#')s+=' → '+href(h)}
if(r==='textbox'||r==='searchbox'||r==='combobox'){const v=e.isContentEditable?e.innerText:e.value;if(v)s+=' = "'+clean(v,80)+'"';if(e.type==='password')s+=' (password)'}
if(r==='checkbox'||r==='radio'||r==='switch'){if(e.checked||e.getAttribute('aria-checked')==='true')s+=' (checked)'}
if(r==='select'){const o=e.selectedOptions&&e.selectedOptions[0];if(o)s+=' = "'+clean(o.text,40)+'"';const all=[...e.options];s+=' options: '+all.slice(0,8).map(o=>clean(o.text,30)).join(' | ')+(all.length>8?' …(+'+(all.length-8)+')':'')}
if(e.disabled||e.getAttribute('aria-disabled')==='true')s+=' (disabled)';
const x=e.getAttribute('aria-expanded');if(x)s+=x==='true'?' (expanded)':' (collapsed)';
if(e.getAttribute('aria-selected')==='true'||e.getAttribute('aria-current')&&e.getAttribute('aria-current')!=='false')s+=' (current)';
return s}
function snapshot(){const out=[];let buf='';
const push=(s)=>{if(s&&s!==out[out.length-1])out.push(s)};
const flush=()=>{const t=clean(buf,400);buf='';if(t)push(t)};
const text=(t)=>{buf+=(buf?' ':'')+t};
function kids(node){for(let c=node.firstChild;c;c=c.nextSibling)visit(c);if(node.shadowRoot)for(let c=node.shadowRoot.firstChild;c;c=c.nextSibling)visit(c)}
function visit(node){if(node.nodeType===3){const t=clean(node.nodeValue,300);if(t)text(t);return}
if(node.nodeType!==1)return;const e=node,t=e.tagName;
if(SKIP.has(t.toUpperCase())||e.getAttribute('aria-hidden')==='true'||!vis(e))return;
const r=role(e);if(r){flush();push(line(e,r));return}
if(t==='LABEL'&&e.control&&role(e.control)){if(e.contains(e.control))visit(e.control);return}
if(/^H[1-6]$/.test(t)){flush();push('#'.repeat(+t[1])+' '+clean(e.innerText,200));return}
if(t==='IMG'){const a=clean(e.alt,80);if(a)text('[image: '+a+']');return}
if(t==='IFRAME'){flush();let d=null;try{d=e.contentDocument}catch(_){}if(d&&d.body)kids(d.body);else push('[frame '+href(e.src||'')+']');return}
if(t==='TD'||t==='TH'){kids(e);text('|');return}
const b=BLOCK.has(t);if(b)flush();if(t==='LI')text('-');
const dl=e.getAttribute('role')==='dialog'||t==='DIALOG';if(dl){flush();push('[dialog'+(name(e)&&e.getAttribute('aria-label')?' "'+clean(e.getAttribute('aria-label'))+'"':'')+']')}
kids(e);if(b)flush();if(dl)push('[end of dialog]')}
if(document.body)kids(document.body);flush();
return{url:location.href,title:document.title,text:out.join('\n')}}
function el(r){const w=refs.get(r),e=w&&w.deref();if(!e||!e.isConnected)throw new Error('There is no element '+r+' on the page now. Take a new browser_snapshot.');return e}
function describe(e){return '['+ref(e)+'] '+(role(e)||e.tagName.toLowerCase())+(name(e)?' "'+name(e)+'"':'')}
const WORDS=/\b(submit|send|buy|pay|purchase|checkout|check out|place order|order now|confirm|delete|remove|publish|post|sign up|register|subscribe|transfer|donate|book now|reserve|unsubscribe)\b/i;
function risky(r){const e=el(r),t=e.tagName,ty=(e.getAttribute('type')||'').toLowerCase();const form=e.form||e.closest('form');
const sub=(t==='BUTTON'&&(ty==='submit'||(!ty&&form)))||(t==='INPUT'&&(ty==='submit'||ty==='image'));
return sub||WORDS.test(name(e))?'Click '+describe(e)+(form?' (submits a form)':''):null}
function submitsForm(r){const e=r?el(r):document.activeElement;if(!e)return null;const f=e.form||(e.closest&&e.closest('form'));return f&&e.tagName!=='TEXTAREA'?'Press Enter in '+describe(e)+' (submits a form)':null}
function point(e){e.scrollIntoView({block:'center',inline:'center'});const b=e.getBoundingClientRect();return{bubbles:true,cancelable:true,composed:true,view:W,clientX:b.x+b.width/2,clientY:b.y+b.height/2,button:0}}
function click(r){const e=el(r),o=point(e);e.dispatchEvent(new PointerEvent('pointerdown',o));e.dispatchEvent(new MouseEvent('mousedown',o));if(e.focus)e.focus({preventScroll:true});e.dispatchEvent(new PointerEvent('pointerup',o));e.dispatchEvent(new MouseEvent('mouseup',o));e.click();return describe(e)}
function setValue(e,v){const p=e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:e.tagName==='SELECT'?HTMLSelectElement.prototype:HTMLInputElement.prototype;
Object.getOwnPropertyDescriptor(p,'value').set.call(e,v);e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}))}
function type(r,text,append){const e=el(r);point(e);e.focus();
if(e.isContentEditable){if(!append){const s=getSelection(),g=document.createRange();g.selectNodeContents(e);s.removeAllRanges();s.addRange(g)}document.execCommand('insertText',false,text)}
else if('value' in e){setValue(e,append?e.value+text:text)}else throw new Error(describe(e)+' is not a text field.');
return describe(e)}
function select(r,v){const e=el(r);if(e.tagName!=='SELECT')throw new Error(describe(e)+' is not a dropdown. Click it, then click the option.');
const all=[...e.options],lo=String(v).toLowerCase();const o=all.find(o=>o.value===v||clean(o.text)===v)||all.find(o=>clean(o.text).toLowerCase().includes(lo));
if(!o)throw new Error('No option "'+v+'". Options: '+all.map(o=>clean(o.text,30)).join(' | '));setValue(e,o.value);return describe(e)+' = "'+clean(o.text)+'"'}
const CODES={Enter:13,Escape:27,Tab:9,Backspace:8,Delete:46,ArrowUp:38,ArrowDown:40,ArrowLeft:37,ArrowRight:39,Space:32,' ':32,Home:36,End:35,PageUp:33,PageDown:34};
function key(r,k){const e=r?el(r):(document.activeElement||document.body);if(k==='Esc')k='Escape';if(k==='Space')k=' ';
const o={key:k,code:k.length===1?'Key'+k.toUpperCase():k,keyCode:CODES[k]||k.toUpperCase().charCodeAt(0),which:CODES[k]||0,bubbles:true,cancelable:true,composed:true};
if(r&&e.focus)e.focus();const go=e.dispatchEvent(new KeyboardEvent('keydown',o));if(k.length===1||k==='Enter')e.dispatchEvent(new KeyboardEvent('keypress',o));e.dispatchEvent(new KeyboardEvent('keyup',o));
if(go&&k==='Enter'&&e.tagName==='INPUT'&&e.form){if(e.form.requestSubmit)e.form.requestSubmit();else e.form.submit()}
if(go&&k==='Tab'){const f=[...document.querySelectorAll('a[href],button,input,select,textarea,[tabindex]:not([tabindex="-1"])')].filter(x=>!x.disabled&&vis(x));const i=f.indexOf(e);const nx=f[i+1]||f[0];if(nx)nx.focus()}
if(go&&k===' '&&(e.tagName==='BUTTON'||e.getAttribute('role')==='button'||e.type==='checkbox'))e.click();
return describe(e)}
function scrollable(){let best=document.scrollingElement||document.documentElement,area=0;if(best.scrollHeight>innerHeight+10)return best;
for(const e of document.querySelectorAll('*')){const s=getComputedStyle(e);if((s.overflowY==='auto'||s.overflowY==='scroll')&&e.scrollHeight>e.clientHeight+10){const a=e.clientWidth*e.clientHeight;if(a>area){area=a;best=e}}}return best}
function scroll(r,d){if(r){el(r).scrollIntoView({block:'center'});return 'Brought '+describe(el(r))+' into view.'}
const s=scrollable(),h=s.clientHeight*0.85;if(d==='top')s.scrollTop=0;else if(d==='bottom')s.scrollTop=s.scrollHeight;else s.scrollTop+=d==='up'?-h:h;
const max=s.scrollHeight-s.clientHeight;return 'At '+(max>0?Math.round(100*s.scrollTop/max):100)+'% of the page.'}
function has(t){return !!document.body&&document.body.innerText.includes(t)}
W.__scout={snapshot,click,type,select,key,scroll,has,risky,submitsForm};
})()}"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(parse_url("localhost:5173").unwrap().as_str(), "http://localhost:5173/");
        assert_eq!(parse_url("example.com/a").unwrap().as_str(), "https://example.com/a");
        assert!(parse_url("rust borrow checker").unwrap().as_str().starts_with("https://duckduckgo.com/?q=rust+borrow"));
        assert!(parse_url("javascript:alert(1)").is_err());
    }

    #[test]
    fn specs_stay_put() {
        let names: Vec<String> = specs().iter().map(|s| s["function"]["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, TOOLS);
    }
}
