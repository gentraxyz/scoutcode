//! Web chatbot connector setup: adds Scout's MCP address to claude.ai or Gemini (`Site`) as a custom connector, in the
//! user's own browser, on their own account. It started with Claude, hence the names.
//!
//! Two ways to do it. On Windows Scout first works the browser itself with the real mouse and keyboard
//! (claude_native.rs), with Scout's own bar (`setup-bar`) showing each step and Pause. When no browser with claude.ai
//! comes to the front, or on other systems, the Scout browser extension does it instead.
//!
//! It only does the setup, never a chat. Once the user presses Start in Scout, it runs the steps by itself, showing a
//! cursor that moves to each button or field it uses, and a Pause button. Signing in to ScoutCode, and the Link that
//! lets Claude reach this computer, stay with the user.
//!
//! Scout keeps one job at a time here. In extension mode, the extension's content script on claude.ai asks for it through the extension's
//! local server (`/claude/sync`, see browser_ext.rs), and reports each step back, which Scout's UI shows.

use serde_json::{Value, json};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A web chatbot Scout can add itself to.
pub struct Site {
    /// What Scout's UI and the extension call it: claude or gemini.
    pub bot: &'static str,
    /// Its name in what Scout says.
    pub name: &'static str,
    /// Its connector settings, which Scout opens.
    pub connectors: &'static str,
    /// The start of its pages' addresses.
    pub home: &'static str,
    /// Its sign-in pages, which are the user's.
    pub logins: &'static [&'static str],
}

impl Site {
    pub fn is_login(&self, at: &str) -> bool {
        self.logins.iter().any(|l| at.starts_with(l))
    }
    /// Whether a tab at `at` is this chatbot, or its sign-in on the way there.
    pub fn owns(&self, at: &str) -> bool {
        at.starts_with(self.home) || self.is_login(at)
    }
}

/// Claude's connector settings are a view over the new-chat page now (they used to be /settings/connectors).
pub const CLAUDE: Site = Site {
    bot: "claude",
    name: "Claude",
    connectors: "https://claude.ai/new#customize/connectors",
    home: "https://claude.ai",
    logins: &["https://claude.ai/login", "https://claude.ai/logout", "https://claude.ai/magic-link"],
};

/// Gemini's connectors: Customize → Connectors, where custom MCP servers are "custom apps". Signed out, Gemini sends
/// the tab to Google's sign-in first.
pub const GEMINI: Site = Site {
    bot: "gemini",
    name: "Gemini",
    connectors: "https://gemini.google.com/customize/connectors",
    home: "https://gemini.google.com",
    logins: &["https://accounts.google.com/"],
};

pub fn site(bot: &str) -> Option<&'static Site> {
    [&CLAUDE, &GEMINI].into_iter().find(|s| s.bot == bot)
}
/// A job nobody picked up for this long is dropped, so a later visit to claude.ai doesn't start an old one.
const FRESH: Duration = Duration::from_secs(15 * 60);
/// A tab that has said nothing for this long gives the job up to another tab.
const OWNER_GONE: Duration = Duration::from_secs(30);

struct Job {
    id: u64,
    site: &'static Site,
    name: String,
    url: String,
    started: Instant,
    /// waiting (for the extension), running, done, failed or stopped.
    state: &'static str,
    /// native (Scout drives the browser) or extension.
    mode: &'static str,
    /// Native mode only: held by the user or by Scout, with why.
    paused: bool,
    note: Option<String>,
    step: u64,
    label: String,
    error: Option<String>,
    /// The tab doing it, and when it last reported.
    owner: Option<String>,
    seen: Option<Instant>,
}

static JOB: Mutex<Option<Job>> = Mutex::new(None);

/// The Scout bar's window, shown over the browser while Scout drives it.
const BAR: &str = "setup-bar";
const BAR_SIZE: (f64, f64) = (420.0, 64.0);
/// Scout's own cursor while it works the browser, so the user's stays theirs.
const CURSOR: &str = "setup-cursor";
const CURSOR_SIZE: (f64, f64) = (86.0, 44.0);

/// Starts adding `url` to a web chatbot (`bot`, see `site`) as a connector named `name`: opens its connector settings
/// in the browser, then works it from here (Windows), or leaves it to the extension, which it writes and serves.
pub fn start<R: tauri::Runtime>(app: &tauri::AppHandle<R>, bot: &str, name: &str, url: &str, browser: Option<&str>) -> Result<Value, String> {
    let site = site(bot).ok_or_else(|| format!("Scout can't add itself to {bot}."))?;
    let name = name.trim();
    if !url.starts_with("https://") {
        return Err(format!("{} only takes https connector addresses.", site.name));
    }
    if name.is_empty() || name.len() > 60 {
        return Err("Give the connector a short name.".into());
    }
    // The extension stays ready either way: it's the fallback.
    crate::browser_ext::extension_folder()?;
    let native = cfg!(windows);
    let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(1);
    *JOB.lock().unwrap() = Some(Job {
        id,
        site,
        name: name.to_string(),
        url: url.to_string(),
        started: Instant::now(),
        state: if native { "running" } else { "waiting" },
        mode: if native { "native" } else { "extension" },
        paused: false,
        note: None,
        step: 0,
        label: String::new(),
        error: None,
        owner: None,
        seen: None,
    });
    let browser = open_in(site.connectors, browser)?;
    if native {
        let app = app.clone();
        let (name, url) = (name.to_string(), url.to_string());
        std::thread::Builder::new().name("claude-setup".into()).spawn(move || drive(app, id, site, &name, &url, browser.as_deref())).map_err(|e| e.to_string())?;
    }
    Ok(status())
}

/// Opens the chatbot's connector settings in the chosen browser (a program from `browsers`), or the default one. Gives
/// back the program it used, so the run works only that browser's windows.
fn open_in(connectors: &str, browser: Option<&str>) -> Result<Option<String>, String> {
    let Some(path) = browser.filter(|p| !p.is_empty()) else {
        crate::oauth::open_browser(connectors);
        return Ok(None);
    };
    // Only a browser Windows lists, so the UI can't have Scout start any program.
    let known = browsers().iter().any(|b| b.get("path").and_then(Value::as_str).is_some_and(|p| p.eq_ignore_ascii_case(path)));
    if !known {
        return Err("That browser isn't installed any more. Pick another one.".into());
    }
    let mut c = std::process::Command::new(path);
    c.arg(connectors);
    crate::tools::no_window(&mut c);
    c.spawn().map_err(|e| format!("Couldn't open the browser: {e}"))?;
    Ok(Some(path.to_string()))
}

/// The browsers on this PC that the setup can use, the default first: `{ name, path, default }`.
pub fn browsers() -> Vec<Value> {
    #[cfg(windows)]
    {
        crate::claude_native::browsers().into_iter().map(|b| json!({ "name": b.name, "path": b.path, "default": b.default })).collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Native mode's Pause and Resume, from Scout's bar or its Settings.
pub fn pause(paused: bool) -> Value {
    if paused {
        pause_with("Paused. The browser is yours; press Resume when you're done.");
    } else if let Some(j) = JOB.lock().unwrap().as_mut() {
        j.paused = false;
        j.note = None;
    }
    status()
}

/// Pauses a native run, saying why.
pub fn pause_with(why: &str) {
    if let Some(j) = JOB.lock().unwrap().as_mut() {
        if j.mode == "native" && j.state == "running" {
            j.paused = true;
            j.note = Some(why.to_string());
        }
    }
}

/// Whether Scout is working the browser right now, so the user's input over it is held back. Asked from input hooks,
/// so it never waits for the lock: busy counts as working.
pub fn blocking() -> bool {
    match JOB.try_lock() {
        Ok(job) => job.as_ref().is_some_and(|j| j.mode == "native" && j.state == "running" && !j.paused),
        Err(_) => true,
    }
}

#[cfg(windows)]
struct Driver<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
    id: u64,
}

#[cfg(windows)]
impl<R: tauri::Runtime> Driver<R> {
    fn with<T>(&self, f: impl FnOnce(&mut Job) -> T) -> Option<T> {
        let mut guard = JOB.lock().unwrap();
        guard.as_mut().filter(|j| j.id == self.id).map(f)
    }
}

#[cfg(windows)]
impl<R: tauri::Runtime> crate::claude_native::Host for Driver<R> {
    fn gate_was_paused(&self) -> Result<bool, crate::claude_native::Fail> {
        let mut waited = false;
        loop {
            match self.with(|j| (j.state, j.paused)) {
                Some(("running", false)) => return Ok(waited),
                Some(("running", true)) => {
                    waited = true;
                    std::thread::sleep(Duration::from_millis(150));
                }
                _ => return Err(crate::claude_native::Fail::Stopped),
            }
        }
    }
    fn pause(&self, why: &str) {
        self.with(|j| {
            j.paused = true;
            j.note = Some(why.to_string());
        });
    }
    fn step(&self, step: u64, label: &str) {
        self.with(|j| {
            j.step = step;
            j.label = label.to_string();
        });
    }
    fn place_bar(&self, browser: windows::Win32::Foundation::RECT) {
        use tauri::Manager;
        let scale = self.app.get_webview_window("main").and_then(|w| w.scale_factor().ok()).unwrap_or(1.0);
        let (w, h) = ((BAR_SIZE.0 * scale) as i32, (BAR_SIZE.1 * scale) as i32);
        let (x, y) = crate::claude_native::bar_spot(browser, w, h);
        let at = tauri::PhysicalPosition::new(x, y);
        if let Some(bar) = self.app.get_webview_window(BAR) {
            let _ = bar.set_position(at);
            let _ = bar.show();
            return;
        }
        let built = tauri::WebviewWindowBuilder::new(&self.app, BAR, tauri::WebviewUrl::App("index.html#setup-bar".into()))
            .title("Scout setup")
            .inner_size(BAR_SIZE.0, BAR_SIZE.1)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .background_color(tauri::webview::Color(0, 0, 0, 0))
            .build();
        if let Ok(bar) = built {
            let _ = bar.set_position(at);
        }
        // Scout's own cursor: a click-through window that never takes focus.
        if self.app.get_webview_window(CURSOR).is_none() {
            let built = tauri::WebviewWindowBuilder::new(&self.app, CURSOR, tauri::WebviewUrl::App("index.html#setup-cursor".into()))
                .title("Scout cursor")
                .inner_size(CURSOR_SIZE.0, CURSOR_SIZE.1)
                .resizable(false)
                .decorations(false)
                .transparent(true)
                .shadow(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .focused(false)
                .background_color(tauri::webview::Color(0, 0, 0, 0))
                .build();
            if let Ok(c) = built {
                let _ = c.set_ignore_cursor_events(true);
            }
        }
    }
    fn cursor_to(&self, x: i32, y: i32) {
        use tauri::Manager;
        // The tip is the window's top-left corner, placed just off the point so a click there lands on the page.
        if let Some(c) = self.app.get_webview_window(CURSOR) {
            let _ = c.set_position(tauri::PhysicalPosition::new(x + 1, y + 1));
        }
    }
    fn cursor_press(&self, down: bool) {
        use tauri::Emitter;
        let _ = self.app.emit_to(CURSOR, "setup-cursor-press", down);
    }
}

/// Native mode: Scout works the browser itself, and falls back to the extension when there's no browser to work.
#[cfg(windows)]
fn drive<R: tauri::Runtime>(app: tauri::AppHandle<R>, id: u64, site: &'static Site, name: &str, url: &str, browser: Option<&str>) {
    use crate::claude_native::Fail;
    use tauri::Manager;
    let d = Driver { app: app.clone(), id };
    let result = crate::claude_native::run(&d, site, name, url, browser);
    let mut linger = Duration::from_secs(6);
    d.with(|j| {
        j.paused = false;
        j.note = None;
        match result {
            Ok(label) => {
                j.state = "done";
                j.label = label;
                linger = Duration::from_secs(10);
            }
            Err(Fail::Stopped) => {
                if j.state == "running" {
                    j.state = "stopped";
                }
            }
            Err(Fail::Error(e)) => {
                j.state = "failed";
                j.error = Some(e);
            }
            Err(Fail::NoBrowser) => {
                // Over to the extension, if it's installed: it picks the job up on the page Scout opened.
                j.mode = "extension";
                j.state = "waiting";
                j.started = Instant::now();
                j.label = String::new();
                linger = Duration::ZERO;
            }
        }
    });
    // Scout's cursor goes at once; the bar stays a moment to say how it went.
    if let Some(c) = app.get_webview_window(CURSOR) {
        let _ = c.close();
    }
    std::thread::sleep(linger);
    if let Some(bar) = app.get_webview_window(BAR) {
        let _ = bar.close();
    }
}

#[cfg(not(windows))]
fn drive<R: tauri::Runtime>(_app: tauri::AppHandle<R>, _id: u64, _site: &'static Site, _name: &str, _url: &str, _browser: Option<&str>) {}

/// Stops the job. The page takes its overlay down on its next report.
pub fn stop() -> Value {
    if let Some(j) = JOB.lock().unwrap().as_mut() {
        if matches!(j.state, "waiting" | "running") {
            j.state = "stopped";
            j.paused = false;
        }
    }
    status()
}

/// What Scout's UI shows.
pub fn status() -> Value {
    let job = JOB.lock().unwrap();
    let Some(j) = job.as_ref() else { return json!({ "state": "idle" }) };
    let stale = j.state == "waiting" && j.started.elapsed() > FRESH;
    json!({
        "state": if stale { "failed" } else { j.state },
        "bot": j.site.bot,
        "step": j.step,
        "label": j.label,
        "error": if stale { Some("The extension never picked it up.".to_string()) } else { j.error.clone() },
        "mode": j.mode,
        "paused": j.paused,
        "note": j.note,
        "extensionSeen": j.seen.is_some(),
        "waitedSecs": j.started.elapsed().as_secs(),
        "extensionPath": crate::browser_ext::extension_path(),
    })
}

/// One exchange with a claude.ai tab: it brings what happened, and gets the job if it's the one doing it.
pub fn sync(msg: &Value) -> Value {
    let tab = msg.get("tab").and_then(Value::as_str).unwrap_or("").to_string();
    let mut guard = JOB.lock().unwrap();
    let Some(j) = guard.as_mut() else { return json!({ "job": null }) };
    // ScoutCode's sign-in and consent page, which Claude opens in its own window after Connect. It's only guided while
    // a setup the user started here is running or just finished, so a link from anywhere else gets no help.
    if msg.get("consent").and_then(Value::as_bool) == Some(true) {
        if !matches!(j.state, "running" | "done") || j.started.elapsed() > FRESH {
            return json!({ "job": null });
        }
        for e in msg.get("events").and_then(Value::as_array).into_iter().flatten() {
            if let Some(label) = e.get("label").and_then(Value::as_str) {
                j.label = label.chars().take(300).collect();
            }
        }
        return json!({ "job": { "id": j.id, "name": j.name, "bot": j.site.bot, "site": j.site.name } });
    }
    // While Scout drives the browser itself, the extension stays out of it. Only a tab on the job's chatbot takes it
    // (an older extension only runs on claude.ai, and doesn't say).
    let bot = msg.get("bot").and_then(Value::as_str).unwrap_or("claude");
    let live = j.mode == "extension" && bot == j.site.bot && matches!(j.state, "waiting" | "running") && j.started.elapsed() < FRESH;
    let owner_alive = j.owner.is_some() && j.seen.is_some_and(|t| t.elapsed() < OWNER_GONE);
    let mine = j.owner.as_deref() == Some(tab.as_str());
    if !live || tab.is_empty() || (owner_alive && !mine) {
        // A tab whose job was stopped from Scout hears so, and takes its overlay down.
        let stopped = mine && j.state == "stopped";
        return json!({ "job": null, "stopped": stopped });
    }
    j.owner = Some(tab);
    j.seen = Some(Instant::now());
    if j.state == "waiting" {
        j.state = "running";
    }
    for e in msg.get("events").and_then(Value::as_array).into_iter().flatten() {
        if e.get("id").and_then(Value::as_u64).is_some_and(|id| id != j.id) {
            continue;
        }
        let text = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or("").chars().take(300).collect::<String>();
        match e.get("kind").and_then(Value::as_str).unwrap_or("") {
            "step" => {
                j.step = e.get("step").and_then(Value::as_u64).unwrap_or(j.step);
                j.label = text("label");
            }
            "done" => {
                j.state = "done";
                j.label = text("label");
            }
            "error" => {
                j.state = "failed";
                j.error = Some(text("text"));
            }
            "stopped" => j.state = "stopped",
            _ => {}
        }
    }
    if j.state != "running" {
        return json!({ "job": null });
    }
    json!({ "job": { "id": j.id, "bot": j.site.bot, "name": j.name, "url": j.url, "step": j.step } })
}
