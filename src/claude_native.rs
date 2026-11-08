//! Claude connector setup without the browser extension, on Windows: Scout finds the user's browser on claude.ai and
//! works it with the real mouse and keyboard, the way a person would.
//!
//! Windows UI Automation (what screen readers use) says where each button and field is on screen, by the name the page
//! gives it. Scout's own cursor (`setup-cursor`, a click-through window) moves there, and the click is made with the
//! real mouse, which hops there for an instant and back to where the user left it. Keys are typed one at a time. A small
//! always-on-top Scout bar (`setup-bar`) shows each step, with Pause and Stop.
//!
//! While it runs, the browser is Scout's: the user's clicks, wheel and keys over it are held back (`guard`), so they
//! can't knock a step off course. Their mouse still moves, to reach the bar. Esc or Pause gives the browser back,
//! as does switching away from it, which pauses the setup until Resume. Anything it can't find stops it with a reason,
//! and when no browser with claude.ai comes up at all, claude_setup.rs falls back to the extension.

use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, HWND, POINT, RECT};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PWSTR;

use crate::claude_setup::Site;

/// Why a run ended early.
pub enum Fail {
    /// The user pressed Stop, or another Start replaced this job.
    Stopped,
    /// No browser with claude.ai came to the front: the extension may still do it.
    NoBrowser,
    Error(String),
}

/// What the run needs from claude_setup.rs: whether to carry on, and where to report.
pub trait Host {
    /// Waits while paused. Err when the job was stopped.
    fn gate(&self) -> Result<(), Fail> {
        self.gate_was_paused().map(|_| ())
    }
    /// The same, saying whether it had to wait.
    fn gate_was_paused(&self) -> Result<bool, Fail>;
    /// Pauses the job, saying why, so the user can sort it out and press Resume.
    fn pause(&self, why: &str);
    fn step(&self, step: u64, label: &str);
    /// Puts the Scout bar at the bottom middle of the browser window, and Scout's cursor on the screen.
    fn place_bar(&self, browser: RECT);
    /// Moves Scout's cursor so its tip is at (x, y), in screen pixels.
    fn cursor_to(&self, x: i32, y: i32);
    /// Shows Scout's cursor pressed, for a click.
    fn cursor_press(&self, down: bool);
}

/// One thing on the page.
struct El {
    e: IUIAutomationElement,
    name: String,
    rect: RECT,
    enabled: bool,
}

struct Ua {
    auto: IUIAutomation,
    cache: IUIAutomationCacheRequest,
}

impl Ua {
    fn new() -> windows::core::Result<Self> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let auto: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
            let cache = auto.CreateCacheRequest()?;
            for p in [UIA_NamePropertyId, UIA_BoundingRectanglePropertyId, UIA_IsEnabledPropertyId] {
                cache.AddProperty(p)?;
            }
            Ok(Self { auto, cache })
        }
    }

    fn kind_is(&self, kind: UIA_CONTROLTYPE_ID) -> windows::core::Result<IUIAutomationCondition> {
        unsafe { self.auto.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(kind.0)) }
    }

    /// The page in the window's front tab.
    fn document(&self, hwnd: HWND) -> Option<IUIAutomationElement> {
        unsafe {
            let w = self.auto.ElementFromHandle(hwnd).ok()?;
            w.FindFirst(TreeScope_Descendants, &self.kind_is(UIA_DocumentControlTypeId).ok()?).ok()
        }
    }

    /// The front tab's address: Chromium gives it as the page's value.
    fn address(&self, doc: &IUIAutomationElement) -> String {
        unsafe {
            doc.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .and_then(|v| v.CurrentValue())
                .map(|b| b.to_string())
                .unwrap_or_default()
        }
    }

    /// Everything of these kinds under `root`, with their names, places and whether they're enabled.
    fn all(&self, root: &IUIAutomationElement, kinds: &[UIA_CONTROLTYPE_ID]) -> Vec<El> {
        let found = (|| unsafe {
            let mut cond = self.kind_is(kinds[0])?;
            for k in &kinds[1..] {
                cond = self.auto.CreateOrCondition(&cond, &self.kind_is(*k)?)?;
            }
            let arr = root.FindAllBuildCache(TreeScope_Descendants, &cond, &self.cache)?;
            let mut out = Vec::new();
            for i in 0..arr.Length()? {
                let e = arr.GetElement(i)?;
                out.push(El {
                    name: e.CachedName().map(|b| b.to_string()).unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" "),
                    rect: e.CachedBoundingRectangle().unwrap_or_default(),
                    enabled: e.CachedIsEnabled().map(|b| b.as_bool()).unwrap_or(false),
                    e,
                });
            }
            windows::core::Result::Ok(out)
        })();
        found.unwrap_or_default()
    }

    /// Whether a checkbox is ticked.
    fn checked(&self, e: &IUIAutomationElement) -> bool {
        unsafe {
            e.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
                .and_then(|t| t.CurrentToggleState())
                .is_ok_and(|s| s == ToggleState_On)
        }
    }

    fn value(&self, e: &IUIAutomationElement) -> String {
        unsafe {
            e.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .and_then(|v| v.CurrentValue())
                .map(|b| b.to_string())
                .unwrap_or_default()
        }
    }
}

fn shown(r: &RECT) -> bool {
    r.right - r.left > 2 && r.bottom - r.top > 2
}

// ---------- the browser window ----------

fn class_of(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// The full path of the program that owns the window, lowercased.
fn exe_of(hwnd: HWND) -> String {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..len as usize]).to_lowercase()
    }
}

/// A browser window in front with the chatbot (or its sign-in) in its front tab: Chrome, Edge, Brave, Helium and the
/// rest of Chromium, or Firefox. Claude's own desktop app shows claude.ai too, so it's left out. With `want` (the
/// chosen browser's program), only that browser's windows count: ones whose program is in its folder, which also
/// covers launchers such as Opera's, whose windows belong to a program a folder further in.
fn browser_in_front(ua: &Ua, want: Option<&str>, site: &Site) -> Option<(HWND, String)> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return None;
    }
    let class = class_of(hwnd);
    if !class.starts_with("Chrome_WidgetWin") && class != "MozillaWindowClass" {
        return None;
    }
    let exe = exe_of(hwnd);
    let file = exe.rsplit('\\').next().unwrap_or("");
    if file == "claude.exe" || file == "scout.exe" || file.starts_with("scout-desktop") {
        return None;
    }
    if let Some(want) = want {
        let folder = want.to_lowercase();
        let folder = folder.rsplit_once('\\').map(|(dir, _)| format!("{dir}\\")).unwrap_or_default();
        if folder.is_empty() || !exe.starts_with(&folder) {
            return None;
        }
    }
    let doc = ua.document(hwnd)?;
    let at = ua.address(&doc);
    site.owns(&at).then_some((hwnd, at))
}

fn window_rect(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut r);
    }
    r
}

// ---------- mouse and keyboard ----------

/// A small random number source for the pace of moves and keys; it only changes timing.
struct Jitter(u64);
impl Jitter {
    fn new() -> Self {
        Self(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7) | 1)
    }
    /// 0.0 to 1.0.
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % 10_000) as f64 / 10_000.0
    }
    fn ms(&mut self, lo: u64, hi: u64) -> Duration {
        Duration::from_millis(lo + ((hi - lo) as f64 * self.next()) as u64)
    }
}

fn cursor() -> POINT {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    p
}

fn send(inputs: &[INPUT]) {
    unsafe {
        SendInput(inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

fn mouse(flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dwFlags: flags, ..Default::default() } } }
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, ..Default::default() } },
    }
}

fn unicode(unit: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wScan: unit, dwFlags: if up { KEYEVENTF_UNICODE | KEYEVENTF_KEYUP } else { KEYEVENTF_UNICODE }, ..Default::default() },
        },
    }
}

fn tap(vk: VIRTUAL_KEY) {
    send(&[key(vk, false), key(vk, true)]);
}

fn chord(modifier: VIRTUAL_KEY, vk: VIRTUAL_KEY) {
    send(&[key(modifier, false), key(vk, false), key(vk, true), key(modifier, true)]);
}

// ---------- the run ----------

struct Run<'a> {
    ua: Ua,
    host: &'a dyn Host,
    site: &'static Site,
    /// The names of the chatbot's connector forms (dialogs), lowercase, by which the open one is found.
    forms: &'static [&'static str],
    browser: HWND,
    rng: Jitter,
    /// Where Scout's own cursor is.
    at: POINT,
}

/// How long to look for something on the page before giving up on it.
const FIND: Duration = Duration::from_secs(12);

impl Run<'_> {
    fn doc(&self) -> Option<IUIAutomationElement> {
        self.ua.document(self.browser)
    }

    fn address(&self) -> String {
        self.doc().map(|d| self.ua.address(&d)).unwrap_or_default()
    }

    fn sleep(&self, d: Duration) -> Result<(), Fail> {
        std::thread::sleep(d);
        self.host.gate()
    }

    /// Waits up to `limit` for `find` to give something, checking for Pause and Stop between looks.
    fn until<T>(&self, limit: Duration, mut find: impl FnMut(&Self) -> Option<T>) -> Result<Option<T>, Fail> {
        let end = Instant::now() + limit;
        loop {
            self.host.gate()?;
            if let Some(v) = find(self) {
                return Ok(Some(v));
            }
            if Instant::now() > end {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    fn all(&self, kinds: &[UIA_CONTROLTYPE_ID]) -> Vec<El> {
        self.doc().map(|d| self.ua.all(&d, kinds)).unwrap_or_default()
    }

    /// The chatbot's connector form (Claude's Add custom connector, Gemini's MCP server dialogs), while it's open.
    fn form(&self) -> Option<El> {
        self.all(&[UIA_WindowControlTypeId]).into_iter().find(|w| {
            let name = w.name.to_lowercase();
            self.forms.iter().any(|f| name.contains(f)) && shown(&w.rect)
        })
    }

    fn in_form(&self, kinds: &[UIA_CONTROLTYPE_ID]) -> Vec<El> {
        self.form().map(|f| self.ua.all(&f.e, kinds)).unwrap_or_default()
    }

    /// Before each click or key: the browser has to be in front and the user's hands off, or it pauses until Resume.
    fn ready(&self) -> Result<(), Fail> {
        loop {
            if self.host.gate_was_paused()? {
                // Pressing Resume put Scout's bar in front; hand it back to the browser.
                self.refocus();
                std::thread::sleep(Duration::from_millis(350));
            }
            if unsafe { GetForegroundWindow() } == self.browser {
                return Ok(());
            }
            self.host.pause("Paused because the browser isn't in front. Bring it back and press Resume.");
        }
    }

    /// Puts the browser back in front after a Resume (clicking Resume put Scout's bar in front).
    fn refocus(&self) {
        unsafe {
            if IsIconic(self.browser).as_bool() {
                let _ = ShowWindow(self.browser, SW_RESTORE);
            }
            let _ = SetForegroundWindow(self.browser);
        }
    }

    /// Moves Scout's own cursor (not the user's) to (x, y) along a gentle curve, easing in and out.
    fn move_to(&mut self, x: i32, y: i32) -> Result<(), Fail> {
        let from = self.at;
        let (fx, fy, tx, ty) = (from.x as f64, from.y as f64, x as f64, y as f64);
        let dist = ((tx - fx).powi(2) + (ty - fy).powi(2)).sqrt();
        if dist < 2.0 {
            return Ok(());
        }
        let bend = (self.rng.next() - 0.5) * 0.3 * dist;
        let (mx, my) = ((fx + tx) / 2.0 - (ty - fy) / dist * bend, (fy + ty) / 2.0 + (tx - fx) / dist * bend);
        let ms = (300.0 + dist * (0.5 + self.rng.next() * 0.2)).min(1100.0);
        let steps = (ms / 16.0).max(8.0) as i32;
        for i in 1..=steps {
            self.host.gate()?;
            let t = i as f64 / steps as f64;
            let e = if t < 0.5 { 2.0 * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(2) / 2.0 };
            let u = 1.0 - e;
            let px = (u * u * fx + 2.0 * u * e * mx + e * e * tx).round() as i32;
            let py = (u * u * fy + 2.0 * u * e * my + e * e * ty).round() as i32;
            self.host.cursor_to(px, py);
            self.at = POINT { x: px, y: py };
            std::thread::sleep(Duration::from_millis(16));
        }
        Ok(())
    }

    /// The real click (or wheel turn) at (x, y): Windows only clicks where the real cursor is, so it hops there for a
    /// moment and straight back to where the user left it. The user's own mouse is held still meanwhile.
    fn real_at(&self, x: i32, y: i32, inputs: &[INPUT]) {
        let back = cursor();
        guard::hold_mouse(true);
        unsafe {
            let _ = SetCursorPos(x, y);
        }
        std::thread::sleep(Duration::from_millis(30));
        for i in inputs {
            send(std::slice::from_ref(i));
            std::thread::sleep(Duration::from_millis(45));
        }
        unsafe {
            let _ = SetCursorPos(back.x, back.y);
        }
        guard::hold_mouse(false);
    }

    /// Scrolls until the element is in view: first by asking the page, then with the mouse wheel over the form.
    fn bring_into_view(&mut self, el: &El) -> Result<RECT, Fail> {
        let now = |el: &El| unsafe { el.e.CurrentBoundingRectangle() }.unwrap_or(el.rect);
        let mut rect = now(el);
        if self.on_screen(&rect) {
            return Ok(rect);
        }
        unsafe {
            if let Ok(s) = el.e.GetCurrentPatternAs::<IUIAutomationScrollItemPattern>(UIA_ScrollItemPatternId) {
                let _ = s.ScrollIntoView();
            }
        }
        self.sleep(Duration::from_millis(400))?;
        rect = now(el);
        for _ in 0..15 {
            if self.on_screen(&rect) {
                return Ok(rect);
            }
            self.ready()?;
            let area = self.form().map(|f| f.rect).or_else(|| self.doc().and_then(|d| unsafe { d.CurrentBoundingRectangle() }.ok())).unwrap_or_default();
            let (cx, cy) = ((area.left + area.right) / 2, (area.top + area.bottom) / 2);
            // Down unless it's above what's shown.
            let up = shown(&rect) && rect.bottom < area.top;
            let turn = if up { 120i32 } else { -120i32 };
            self.move_to(cx, cy)?;
            let wheel = INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { mouseData: turn as u32, dwFlags: MOUSEEVENTF_WHEEL, ..Default::default() } } };
            self.real_at(cx, cy, &[wheel]);
            self.sleep(Duration::from_millis(300))?;
            rect = now(el);
        }
        if self.on_screen(&rect) { Ok(rect) } else { Err(Fail::Error("offscreen".into())) }
    }

    /// Turns the mouse wheel down over the open form (or the page), a notch at a time, to reach its foot.
    fn wheel_form(&mut self, notches: u32) -> Result<(), Fail> {
        self.ready()?;
        let area = self.form().map(|f| f.rect).or_else(|| self.doc().and_then(|d| unsafe { d.CurrentBoundingRectangle() }.ok())).unwrap_or_default();
        if !shown(&area) {
            return Ok(());
        }
        let (cx, cy) = ((area.left + area.right) / 2, (area.top + area.bottom) / 2);
        self.move_to(cx, cy)?;
        for _ in 0..notches {
            self.ready()?;
            let turn = -120i32;
            let wheel = INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { mouseData: turn as u32, dwFlags: MOUSEEVENTF_WHEEL, ..Default::default() } } };
            self.real_at(cx, cy, &[wheel]);
            std::thread::sleep(Duration::from_millis(60));
        }
        self.sleep(Duration::from_millis(350))
    }

    /// Scrolls the element into view, moves Scout's cursor there and clicks it.
    fn click(&mut self, el: &El) -> Result<(), Fail> {
        self.ready()?;
        let rect = self.bring_into_view(el)?;
        let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
        let x = rect.left + w / 2 + ((self.rng.next() - 0.5) * (w as f64 / 5.0).min(10.0)) as i32;
        let y = rect.top + h / 2 + ((self.rng.next() - 0.5) * (h as f64 / 5.0).min(5.0)) as i32;
        self.move_to(x, y)?;
        let wait = self.rng.ms(120, 320);
        self.sleep(wait)?;
        self.ready()?;
        // Whatever is on top there gets the click, so make sure it's the browser and not something over it.
        let mut top = unsafe { GetAncestor(WindowFromPoint(POINT { x, y }), GA_ROOT) };
        if top != self.browser && is_ours(top) {
            // Scout's own bar, over a button at the bottom of the page: it moves to the top for the rest of the run.
            BAR_AT_TOP.store(true, std::sync::atomic::Ordering::SeqCst);
            self.host.place_bar(window_rect(self.browser));
            std::thread::sleep(Duration::from_millis(250));
            self.refocus();
            std::thread::sleep(Duration::from_millis(150));
            top = unsafe { GetAncestor(WindowFromPoint(POINT { x, y }), GA_ROOT) };
        }
        if top != self.browser {
            self.host.pause("Paused because a window is covering the browser where Scout needs to click. Move it and press Resume.");
            return Err(Fail::Error("covered".into()));
        }
        self.host.cursor_press(true);
        self.real_at(x, y, &[mouse(MOUSEEVENTF_LEFTDOWN), mouse(MOUSEEVENTF_LEFTUP)]);
        self.host.cursor_press(false);
        Ok(())
    }

    /// Whether the rectangle is inside the browser's page area, and inside the form when one is open.
    fn on_screen(&self, r: &RECT) -> bool {
        let Some(doc) = self.doc() else { return true };
        let page = unsafe { doc.CurrentBoundingRectangle() }.unwrap_or_default();
        let inside = |a: &RECT| r.top >= a.top && r.bottom <= a.bottom && r.left >= a.left && r.right <= a.right;
        shown(r) && inside(&page) && self.form().is_none_or(|f| inside(&f.rect))
    }

    /// Clicks into the field, selects what's there and types over it a key at a time.
    fn type_into(&mut self, el: &El, text: &str) -> Result<(), Fail> {
        self.click(el)?;
        self.sleep(Duration::from_millis(150))?;
        self.ready()?;
        chord(VK_CONTROL, VK_A);
        self.sleep(Duration::from_millis(80))?;
        for ch in text.chars() {
            self.ready()?;
            let mut buf = [0u16; 2];
            for unit in ch.encode_utf16(&mut buf) {
                send(&[unicode(*unit, false), unicode(*unit, true)]);
            }
            let mut gap = self.rng.ms(45, 120);
            if " ./:@-".contains(ch) {
                gap += self.rng.ms(30, 120);
            }
            std::thread::sleep(gap);
        }
        Ok(())
    }

    /// Clicks, retrying when a pause or a covering window got in the way, finding the element again each time.
    fn click_found(&mut self, what: &str, mut find: impl FnMut(&Self) -> Option<El>) -> Result<(), Fail> {
        for _ in 0..4 {
            let Some(el) = self.until(FIND, &mut find)? else {
                return Err(Fail::Error(format!("Scout couldn't find {what} on {}.", self.site.name)));
            };
            match self.click(&el) {
                Ok(()) => return Ok(()),
                Err(Fail::Error(_)) => continue,
                Err(f) => return Err(f),
            }
        }
        Err(Fail::Error(format!("Scout couldn't click {what}.")))
    }

    fn type_found(&mut self, what: &str, text: &str, mut find: impl FnMut(&Self) -> Option<El>) -> Result<(), Fail> {
        for _ in 0..4 {
            let Some(el) = self.until(FIND, &mut find)? else {
                return Err(Fail::Error(format!("Scout couldn't find {what} in {}'s form.", self.site.name)));
            };
            match self.type_into(&el, text) {
                Ok(()) => {}
                Err(Fail::Error(_)) => continue,
                Err(f) => return Err(f),
            }
            // Done when the field says exactly that.
            let ok = self.until(Duration::from_secs(3), |r| (r.ua.value(&el.e).trim() == text.trim()).then_some(()))?;
            if ok.is_some() {
                return Ok(());
            }
        }
        Err(Fail::Error(format!("Scout couldn't fill in {what}.")))
    }

    /// Goes to an address in the front tab: the address bar, the address, Enter.
    fn navigate(&mut self, url: &str) -> Result<(), Fail> {
        self.ready()?;
        chord(VK_CONTROL, VK_L);
        self.sleep(Duration::from_millis(250))?;
        for ch in url.chars() {
            self.ready()?;
            let mut buf = [0u16; 2];
            for unit in ch.encode_utf16(&mut buf) {
                send(&[unicode(*unit, false), unicode(*unit, true)]);
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        self.sleep(Duration::from_millis(200))?;
        self.ready()?;
        tap(VK_RETURN);
        Ok(())
    }
}

fn on_connectors(at: &str) -> bool {
    at.contains("customize/connectors") || at.contains("/settings/connectors")
}

/// Adds the connector to the chatbot. Its connector settings were already opened in `browser` (a program path; None
/// for the default browser). Returns what to tell the user.
pub fn run(host: &dyn Host, site: &'static Site, name: &str, url: &str, browser: Option<&str>) -> Result<String, Fail> {
    let ua = Ua::new().map_err(|_| Fail::NoBrowser)?;
    host.step(0, &format!("Looking for your browser on {}", site.name));
    // The browser comes to the front with the page Scout just opened.
    let end = Instant::now() + Duration::from_secs(25);
    let (browser, _) = loop {
        host.gate()?;
        if let Some(b) = browser_in_front(&ua, browser, site) {
            break b;
        }
        if Instant::now() > end {
            return Err(Fail::NoBrowser);
        }
        std::thread::sleep(Duration::from_millis(400));
    };
    let forms: &'static [&'static str] =
        if site.bot == "gemini" { &["connect to an mcp server", "connect this mcp server", "save your custom app"] } else { &["add custom connector"] };
    // Scout's cursor starts where the user's is, and goes its own way from there.
    let mut r = Run { ua, host, site, forms, browser, rng: Jitter::new(), at: cursor() };
    // Gemini's dialogs keep their checkbox and buttons at the bottom, where the bar would sit: it goes at the top there.
    BAR_AT_TOP.store(site.bot == "gemini", std::sync::atomic::Ordering::SeqCst);
    host.place_bar(window_rect(browser));
    host.cursor_to(r.at.x, r.at.y);
    // Showing the bar can bring Scout to the front; the browser goes back there.
    std::thread::sleep(Duration::from_millis(500));
    r.refocus();

    // Signing in is the user's; carry on once they have.
    if site.is_login(&r.address()) {
        host.step(0, &format!("Sign in to {} in your browser. Scout carries on after.", site.name));
        let signed_in = r.until(Duration::from_secs(600), |r| {
            let at = r.address();
            at.starts_with(site.home).then_some(())
        })?;
        if signed_in.is_none() {
            return Err(Fail::Error(format!("{} wasn't signed in.", site.name)));
        }
        r.sleep(Duration::from_secs(2))?;
    }
    // From here the browser is Scout's until the run ends, however it ends.
    let _guard = guard::arm(browser);
    if site.bot == "gemini" { gemini(&mut r, name, url) } else { claude(&mut r, name, url) }
}

/// Claude: Add → Add custom connector, a name and the address, Continue, Add, then Connect.
fn claude(r: &mut Run, name: &str, url: &str) -> Result<String, Fail> {
    let host = r.host;
    let connectors = r.site.connectors;
    if !on_connectors(&r.address()) {
        host.step(0, "Opening Customize → Connectors");
        r.navigate(connectors)?;
        r.until(Duration::from_secs(15), |r| on_connectors(&r.address()).then_some(()))?;
    }

    // Wait for the connectors view, and see whether this connector is already there.
    host.step(0, "Waiting for Claude's connectors");
    let add_button = |r: &Run| r.all(&[UIA_ButtonControlTypeId]).into_iter().find(|b| b.name.eq_ignore_ascii_case("Add connector") && shown(&b.rect));
    if r.until(Duration::from_secs(20), add_button)?.is_none() {
        return Err(Fail::Error("Scout couldn't find Claude's Add connector button. Is the connectors page open?".into()));
    }
    r.sleep(Duration::from_millis(600))?;
    // A connector in Claude's list is a link with its name; plain text could be a chat called that.
    let listed = r.all(&[UIA_HyperlinkControlTypeId, UIA_ListItemControlTypeId]).iter().any(|e| e.name == name);

    if !listed {
        host.step(1, "Clicking Add, then Add custom connector");
        let custom = |r: &Run| r.all(&[UIA_MenuItemControlTypeId]).into_iter().find(|m| m.name.to_lowercase().contains("add custom connector") && shown(&m.rect));
        let mut opened = false;
        for _ in 0..3 {
            r.click_found("the Add connector button", add_button)?;
            if r.until(Duration::from_secs(4), custom)?.is_some() {
                opened = true;
                break;
            }
        }
        if !opened {
            return Err(Fail::Error("Claude's Add menu didn't show Add custom connector.".into()));
        }
        let wait = r.rng.ms(200, 450);
        r.sleep(wait)?;
        r.click_found("Add custom connector", custom)?;
        if r.until(FIND, |r| r.form())?.is_none() {
            return Err(Fail::Error("Claude's Add custom connector form didn't open.".into()));
        }

        // The form: a name, then the server address. By label, or else by order.
        let edit = |r: &Run, label: &str, nth: usize| {
            let edits: Vec<El> = r.in_form(&[UIA_EditControlTypeId]).into_iter().filter(|e| shown(&e.rect)).collect();
            let i = edits.iter().position(|e| e.name.to_lowercase().contains(label)).unwrap_or(nth);
            edits.into_iter().nth(i)
        };
        host.step(2, &format!("Typing {name} as the name"));
        r.type_found("the Name box", name, |r| edit(r, "name", 0))?;
        host.step(3, "Typing the address");
        r.type_found("the MCP server URL box", url, |r| edit(r, "url", 1))?;
        // Leave the field: Claude checks it then, and keeps Continue greyed out until it has.
        r.ready()?;
        tap(VK_TAB);
        r.sleep(Duration::from_millis(400))?;

        host.step(4, "Clicking Continue");
        // The form's own name keeps saying Step 1 after Continue; its heading and a Back button say Step 2.
        let second_page = |r: &Run| {
            r.in_form(&[UIA_TextControlTypeId, UIA_ButtonControlTypeId]).iter().any(|e| e.name.to_lowercase().contains("step 2") || e.name.eq_ignore_ascii_case("back"))
        };
        let cont = |r: &Run| {
            if second_page(r) {
                return None;
            }
            r.in_form(&[UIA_ButtonControlTypeId]).into_iter().find(|b| matches!(b.name.to_lowercase().as_str(), "continue" | "next") && b.enabled)
        };
        if r.until(Duration::from_secs(8), cont)?.is_none() {
            return Err(Fail::Error("Claude's Continue stayed greyed out. Check the name and address in the form.".into()));
        }
        r.click_found("Continue", cont)?;
        r.until(Duration::from_secs(10), |r| (second_page(r) || r.form().is_none()).then_some(()))?;

        // The second page: how users sign in. Scout leaves what Claude detected from the server, and saves.
        if r.form().is_some() {
            host.step(5, "Clicking Add to save the connector");
            let save = |r: &Run| {
                if !second_page(r) {
                    return None;
                }
                // The form's own Add sits at its foot, below the fold: it's the last one, whether in view or not
                // (an Add for request headers can come before it). Clicking scrolls it into view.
                r.in_form(&[UIA_ButtonControlTypeId]).into_iter().filter(|b| matches!(b.name.to_lowercase().as_str(), "add" | "save" | "add connector") && b.enabled).last()
            };
            r.click_found("Claude's Add button", save)?;
            if r.until(Duration::from_secs(20), |r| r.form().is_none().then_some(()))?.is_none() {
                return Err(Fail::Error("Claude didn't save the connector. Its form is still open in the browser.".into()));
            }
        }
        r.sleep(Duration::from_millis(900))?;
    }

    // Connect, in the connector's own row: a button that names it, or the only Connect there is.
    host.step(6, &format!("Clicking Connect on {name}"));
    let lower = name.to_lowercase();
    let connect = |r: &Run| {
        let all: Vec<El> = r.all(&[UIA_ButtonControlTypeId]).into_iter().filter(|b| b.enabled && b.name.to_lowercase().starts_with("connect")).collect();
        let named = all.iter().position(|b| b.name.to_lowercase().split_whitespace().any(|w| w == lower));
        let plain: Vec<usize> = all.iter().enumerate().filter(|(_, b)| b.name.eq_ignore_ascii_case("connect")).map(|(i, _)| i).collect();
        let i = named.or(if plain.len() == 1 { Some(plain[0]) } else { None })?;
        all.into_iter().nth(i)
    };
    if r.until(Duration::from_secs(8), connect)?.is_some() && r.click_found("Connect", connect).is_ok() {
        return Ok("Added to Claude".into());
    }
    Ok("Added to Claude, click Connect".into())
}

/// Gemini: Create custom app, the address, Next, then Gemini's notice about custom apps, which the user accepts
/// themselves, and Connect. Gemini asks for no name: it names the app from the server.
fn gemini(r: &mut Run, name: &str, url: &str) -> Result<String, Fail> {
    let host = r.host;
    let connectors = r.site.connectors;
    let on_connectors = |at: &str| at.contains("gemini.google.com/customize/connectors");
    if !on_connectors(&r.address()) {
        host.step(0, "Opening Customize → Connectors");
        r.navigate(connectors)?;
        r.until(Duration::from_secs(15), |r| on_connectors(&r.address()).then_some(()))?;
    }

    host.step(0, "Waiting for Gemini's connected apps");
    // There are two: the + at the top, and Custom beside the search box. Either opens the form.
    let create = |r: &Run| r.all(&[UIA_ButtonControlTypeId]).into_iter().find(|b| b.name.eq_ignore_ascii_case("Create custom app") && b.enabled && shown(&b.rect));
    if r.until(Duration::from_secs(20), create)?.is_none() {
        return Err(Fail::Error("Scout couldn't find Gemini's Create custom app button. Is the Connectors page open?".into()));
    }
    r.sleep(Duration::from_millis(600))?;
    // An installed app is a button named "<name>, <description>"; Gemini may also go by the server's address.
    let lower = name.to_lowercase();
    let host_of = url.trim_start_matches("https://").split('/').next().unwrap_or("").to_lowercase();
    let listed = r.all(&[UIA_ButtonControlTypeId]).iter().any(|b| {
        let n = b.name.to_lowercase();
        n == lower || n.starts_with(&format!("{lower},")) || (!host_of.is_empty() && n.contains(&host_of))
    });
    if listed {
        return Ok("Already added to Gemini".into());
    }

    host.step(1, "Clicking Create custom app");
    let mut opened = false;
    for _ in 0..3 {
        r.click_found("the Create custom app button", create)?;
        if r.until(Duration::from_secs(4), |r| r.form())?.is_some() {
            opened = true;
            break;
        }
    }
    if !opened {
        return Err(Fail::Error("Gemini's Connect to an MCP server form didn't open.".into()));
    }

    host.step(2, "Typing the address");
    // The address box, by its label or placeholder, or else the first box (Client ID and secret stay folded away).
    let field = |r: &Run| {
        let edits: Vec<El> = r.in_form(&[UIA_EditControlTypeId]).into_iter().filter(|e| shown(&e.rect)).collect();
        let i = edits.iter().position(|e| {
            let n = e.name.to_lowercase();
            n.contains("url") || n.contains("https") || n.contains("mcp")
        });
        edits.into_iter().nth(i.unwrap_or(0))
    };
    r.type_found("the MCP Server URL box", url, field)?;
    r.ready()?;
    tap(VK_TAB);
    r.sleep(Duration::from_millis(400))?;

    host.step(3, "Clicking Next");
    // The second dialog: Gemini's notice, with its checkbox.
    let notice = |r: &Run| {
        r.form().is_some_and(|f| f.name.to_lowercase().contains("connect this")) || !r.in_form(&[UIA_CheckBoxControlTypeId]).is_empty()
    };
    let next = |r: &Run| {
        if notice(r) {
            return None;
        }
        r.in_form(&[UIA_ButtonControlTypeId]).into_iter().find(|b| b.name.eq_ignore_ascii_case("next") && b.enabled)
    };
    if r.until(Duration::from_secs(8), next)?.is_none() {
        return Err(Fail::Error("Gemini's Next stayed greyed out. Check the address in the form.".into()));
    }
    // What Gemini says went wrong, in red under the address: the server refused it, or Gemini couldn't reach it.
    let complaint = |r: &Run| {
        r.in_form(&[UIA_TextControlTypeId]).into_iter().map(|t| t.name).find(|n| {
            let n = n.to_lowercase();
            ["rejected", "couldn't", "could not", "unable", "error", "failed", "invalid"].iter().any(|w| n.contains(w))
        })
    };
    let refused = |what: String| Fail::Error(format!("Gemini says: {what}"));
    r.click_found("Next", next)?;
    let moved = r.until(Duration::from_secs(20), |r| if notice(r) { Some(None) } else { complaint(r).map(Some) })?;
    match moved {
        Some(None) => {}
        Some(Some(what)) => return Err(refused(what)),
        None => return Err(Fail::Error("Gemini didn't take the address. Its form is still open in the browser.".into())),
    }

    // Gemini's notice about custom apps: Scout scrolls to its foot and ticks the box (Scout's setup says so before
    // Start). The box sits at the very bottom, half under the Cancel and Connect row, so Scout scrolls all the way
    // down and clicks the box's words, which tick it too. Ticked shows as the box, or as Connect turning on.
    host.step(4, "Ticking the box on Gemini's notice");
    let tick = |r: &Run| r.in_form(&[UIA_CheckBoxControlTypeId]).into_iter().next();
    let words = |r: &Run| r.in_form(&[UIA_TextControlTypeId]).into_iter().find(|t| t.name.to_lowercase().starts_with("i understand and accept") && shown(&t.rect));
    let connect_on = |r: &Run| r.in_form(&[UIA_ButtonControlTypeId]).iter().any(|b| b.name.eq_ignore_ascii_case("connect") && b.enabled);
    let accepted = |r: &Run| connect_on(r) || tick(r).is_some_and(|c| r.ua.checked(&c.e));
    if r.until(FIND, tick)?.is_none() {
        return Err(Fail::Error("Scout couldn't find the checkbox on Gemini's notice.".into()));
    }
    let mut ticked = accepted(r);
    for attempt in 0..3 {
        if ticked {
            break;
        }
        r.wheel_form(8)?;
        // First the words, then the box itself, then the box through its accessibility toggle.
        let done = match attempt {
            0 if words(r).is_some() => r.click_found("the words beside Gemini's checkbox", words),
            0 | 1 => r.click_found("the checkbox on Gemini's notice", tick),
            _ => {
                if let Some(c) = tick(r) {
                    unsafe {
                        if let Ok(t) = c.e.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId) {
                            let _ = t.Toggle();
                        }
                    }
                }
                Ok(())
            }
        };
        done?;
        ticked = r.until(Duration::from_secs(3), |r| accepted(r).then_some(()))?.is_some();
    }
    if !ticked {
        return Err(Fail::Error("Scout couldn't tick the box on Gemini's notice.".into()));
    }
    r.sleep(Duration::from_millis(400))?;

    host.step(5, "Clicking Connect");
    let connect = |r: &Run| r.in_form(&[UIA_ButtonControlTypeId]).into_iter().find(|b| b.name.eq_ignore_ascii_case("connect") && b.enabled);
    if r.until(Duration::from_secs(8), connect)?.is_none() {
        return Err(Fail::Error("Gemini's Connect stayed greyed out. Tick the box on its notice.".into()));
    }
    r.click_found("Connect", connect)?;
    // Connect registers Scout with ScoutCode, then Google asks to link the user's Google Account to it ("Agree and
    // continue"), on Gemini's page or in a window of its own, before ScoutCode's sign-in. When registering fails,
    // Gemini goes back to the first form with a reason.
    host.step(6, "Waiting for Google to ask about linking your account");
    let agree = |r: &Run| r.all(&[UIA_ButtonControlTypeId]).into_iter().find(|b| b.name.eq_ignore_ascii_case("agree and continue") && b.enabled && shown(&b.rect));
    let main = r.browser;
    let program = exe_of(main);
    let end = Instant::now() + Duration::from_secs(30);
    let found = loop {
        host.gate()?;
        if let Some(what) = complaint(r) {
            return Err(refused(what));
        }
        if agree(r).is_some() {
            break true;
        }
        // A window of its own: the same browser, in front, on Google's sign-in.
        let front = unsafe { GetForegroundWindow() };
        if front != main && !front.is_invalid() && exe_of(front) == program {
            let at = r.ua.document(front).map(|d| r.ua.address(&d)).unwrap_or_default();
            if at.starts_with("https://accounts.google.com") {
                r.browser = front;
                if r.until(Duration::from_secs(10), agree)?.is_some() {
                    break true;
                }
                r.browser = main;
            }
            // Straight to ScoutCode: nothing for Google to ask.
            if at.starts_with("https://scoutcode.dev") {
                break false;
            }
        }
        if Instant::now() > end {
            break false;
        }
        std::thread::sleep(Duration::from_millis(400));
    };
    if found {
        // Scout scrolls down to Agree and continue and clicks it; ScoutCode's sign-in comes next.
        host.step(7, "Clicking Agree and continue to link your Google Account");
        let window = r.browser;
        if window != main {
            // Guard and focus follow Google's window while Scout works it.
            guard::follow(window);
        }
        let clicked = r.click_found("Agree and continue", agree);
        let gone = clicked.is_ok() && r.until(Duration::from_secs(15), |r| (!unsafe { IsWindow(Some(window)) }.as_bool() || agree(r).is_none()).then_some(()))?.is_some();
        guard::follow(main);
        r.browser = main;
        if !gone {
            clicked?;
            return Err(Fail::Error("Google's Link your Google Account page didn't go on after Agree and continue.".into()));
        }
    }

    // ScoutCode's sign-in comes next, in its own window, where the user approves Link. Then Gemini asks to save the
    // app ("Save your custom app", its name filled in from the server) with one more Connect.
    host.step(8, "Waiting for you to approve Link in ScoutCode's window");
    let save_form = |r: &Run| r.form().filter(|f| f.name.to_lowercase().contains("save your custom app"));
    // By its buttons too, in case the dialog goes by no name: a Connect beside a box holding the app's name.
    let save_connect = |r: &Run| {
        let buttons = match save_form(r) {
            Some(f) => r.ua.all(&f.e, &[UIA_ButtonControlTypeId]),
            None => {
                let named = r.all(&[UIA_EditControlTypeId]).iter().any(|e| shown(&e.rect) && r.ua.value(&e.e).trim().eq_ignore_ascii_case(name));
                if !named {
                    return None;
                }
                r.all(&[UIA_ButtonControlTypeId])
            }
        };
        buttons.into_iter().find(|b| b.name.eq_ignore_ascii_case("connect") && b.enabled && shown(&b.rect))
    };
    // The browser is the user's meanwhile, wherever ScoutCode opens (a window, or a tab of this one).
    guard::follow(HWND::default());
    let end = Instant::now() + Duration::from_secs(600);
    let shown_up = loop {
        if host.gate().is_err() {
            break Err(Fail::Stopped);
        }
        if save_connect(r).is_some() {
            break Ok(true);
        }
        if Instant::now() > end {
            break Ok(false);
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    guard::follow(main);
    if !shown_up? {
        return Ok("Added to Gemini, approve Link in ScoutCode, then Connect".into());
    }
    // ScoutCode's window has just closed; Gemini's comes back to the front for the last click.
    r.refocus();
    r.sleep(Duration::from_millis(400))?;
    host.step(9, "Clicking Connect to save the app");
    let name_box = |r: &Run| save_form(r).and_then(|f| r.ua.all(&f.e, &[UIA_EditControlTypeId]).into_iter().find(|e| shown(&e.rect)));
    if let Some(b) = name_box(r) {
        if r.ua.value(&b.e).trim().is_empty() {
            r.type_found("the app's name", name, name_box)?;
        }
    }
    r.click_found("Connect", save_connect)?;
    r.until(Duration::from_secs(15), |r| save_connect(r).is_none().then_some(()))?;

    // Done: a new chat, ready to ask Gemini to work on the project.
    host.step(10, "Opening a new chat in Gemini");
    r.sleep(Duration::from_millis(800))?;
    r.navigate("https://gemini.google.com/app")?;
    r.until(Duration::from_secs(15), |r| r.address().starts_with("https://gemini.google.com/app").then_some(()))?;
    Ok("Gemini is linked to Scout and completely ready. Ask it to work on your project in the new chat.".into())
}

/// Holds back the user's own clicks, wheel and keys over the browser while Scout works it, with low-level input hooks
/// (no admin needed). Scout's own input is marked injected and goes through. The user's mouse still moves, so they can
/// reach Scout's bar, and clicks anywhere else work. Esc pauses the setup; Alt and Windows-key combinations pass, so
/// Alt+Tab still switches away (which pauses it too). Nothing is held while the setup is paused.
mod guard {
    use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_LWIN, VK_RWIN};
    use windows::Win32::UI::WindowsAndMessaging::*;

    static ARMED: AtomicBool = AtomicBool::new(false);
    /// Set while the real cursor hops for one of Scout's clicks: the user's mouse is held still for it.
    static STILL: AtomicBool = AtomicBool::new(false);
    static BROWSER: AtomicIsize = AtomicIsize::new(0);
    static THREAD: AtomicU32 = AtomicU32::new(0);

    pub fn hold_mouse(on: bool) {
        STILL.store(on, Ordering::SeqCst);
    }

    fn holding() -> bool {
        ARMED.load(Ordering::SeqCst) && crate::claude_setup::blocking()
    }

    fn browser() -> HWND {
        HWND(BROWSER.load(Ordering::SeqCst) as *mut _)
    }

    unsafe extern "system" fn on_mouse(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe {
            if code == HC_ACTION as i32 && holding() {
                let m = &*(l.0 as *const MSLLHOOKSTRUCT);
                if m.flags & LLMHF_INJECTED == 0 {
                    if w.0 as u32 == WM_MOUSEMOVE {
                        if STILL.load(Ordering::SeqCst) {
                            return LRESULT(1);
                        }
                    } else if GetAncestor(WindowFromPoint(POINT { x: m.pt.x, y: m.pt.y }), GA_ROOT) == browser() {
                        return LRESULT(1);
                    }
                }
            }
            CallNextHookEx(None, code, w, l)
        }
    }

    unsafe extern "system" fn on_key(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        unsafe {
            if code == HC_ACTION as i32 && holding() {
                let k = &*(l.0 as *const KBDLLHOOKSTRUCT);
                let combo = k.flags.contains(LLKHF_ALTDOWN) || k.vkCode == VK_LWIN.0 as u32 || k.vkCode == VK_RWIN.0 as u32;
                if !k.flags.contains(LLKHF_INJECTED) && !combo && GetForegroundWindow() == browser() {
                    let down = matches!(w.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
                    if down && k.vkCode == VK_ESCAPE.0 as u32 {
                        crate::claude_setup::pause_with("Paused (Esc). The browser is yours; press Resume when you're done.");
                    }
                    return LRESULT(1);
                }
            }
            CallNextHookEx(None, code, w, l)
        }
    }

    /// Holds input over another window of the browser instead (Google's own window during Gemini's setup).
    pub fn follow(window: HWND) {
        BROWSER.store(window.0 as isize, Ordering::SeqCst);
    }

    /// Lets go of the browser when dropped.
    pub struct Armed;

    impl Drop for Armed {
        fn drop(&mut self) {
            ARMED.store(false, Ordering::SeqCst);
            STILL.store(false, Ordering::SeqCst);
            let t = THREAD.swap(0, Ordering::SeqCst);
            if t != 0 {
                unsafe {
                    let _ = PostThreadMessageW(t, WM_QUIT, WPARAM(0), LPARAM(0));
                }
            }
        }
    }

    /// Starts holding the user's input over `browser`. The hooks live on their own thread, which pumps their messages.
    pub fn arm(browser: HWND) -> Armed {
        BROWSER.store(browser.0 as isize, Ordering::SeqCst);
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new().name("claude-setup-guard".into()).spawn(move || unsafe {
            let module = GetModuleHandleW(None).ok().map(|m| m.into());
            let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(on_mouse), module, 0);
            let keys = SetWindowsHookExW(WH_KEYBOARD_LL, Some(on_key), module, 0);
            let _ = tx.send(GetCurrentThreadId());
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if let Ok(h) = mouse {
                let _ = UnhookWindowsHookEx(h);
            }
            if let Ok(h) = keys {
                let _ = UnhookWindowsHookEx(h);
            }
        });
        if let Ok(t) = rx.recv_timeout(std::time::Duration::from_secs(2)) {
            THREAD.store(t, Ordering::SeqCst);
            ARMED.store(true, Ordering::SeqCst);
        }
        Armed
    }
}

// ---------- which browsers there are ----------

/// A browser on this PC, as Windows lists it for Default apps.
pub struct Browser {
    pub name: String,
    /// Its program, which Scout starts with the address.
    pub path: String,
    pub default: bool,
}

mod reg {
    use windows::Win32::System::Registry::*;
    use windows::core::{HSTRING, PCWSTR};

    /// A key's default value, as text.
    pub fn default_value(root: HKEY, path: &str) -> Option<String> {
        let mut buf = vec![0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        let ok = unsafe {
            RegGetValueW(root, &HSTRING::from(path), PCWSTR::null(), RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut size))
        };
        ok.is_ok().then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]))
    }

    /// A named value, as text.
    pub fn value(root: HKEY, path: &str, name: &str) -> Option<String> {
        let mut buf = vec![0u16; 512];
        let mut size = (buf.len() * 2) as u32;
        let ok = unsafe { RegGetValueW(root, &HSTRING::from(path), &HSTRING::from(name), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut size)) };
        ok.is_ok().then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]))
    }

    /// The names of a key's subkeys.
    pub fn subkeys(root: HKEY, path: &str) -> Vec<String> {
        let mut out = Vec::new();
        unsafe {
            let mut key = HKEY::default();
            if RegOpenKeyExW(root, &HSTRING::from(path), None, KEY_READ, &mut key).is_err() {
                return out;
            }
            for i in 0.. {
                let mut name = [0u16; 256];
                let mut len = name.len() as u32;
                if RegEnumKeyExW(key, i, Some(windows::core::PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None).is_err() {
                    break;
                }
                out.push(String::from_utf16_lossy(&name[..len as usize]));
            }
            let _ = RegCloseKey(key);
        }
        out
    }
}

/// The program in a command line such as `"C:\…\chrome.exe" --single-argument %1`.
fn program_of(command: &str) -> String {
    let c = command.trim();
    if let Some(rest) = c.strip_prefix('"') {
        return rest.split('"').next().unwrap_or("").to_string();
    }
    match c.to_ascii_lowercase().find(".exe") {
        Some(i) => c[..i + 4].to_string(),
        None => c.to_string(),
    }
}

/// The browsers Windows knows about (what Default apps lists), with the default one marked. Internet Explorer is left
/// out: it can't open claude.ai.
pub fn browsers() -> Vec<Browser> {
    use windows::Win32::System::Registry::{HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    let https = r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https";
    let default = ["UserChoiceLatest", "UserChoice"]
        .iter()
        .find_map(|k| reg::value(HKEY_CURRENT_USER, &format!(r"{https}\{k}"), "ProgId").filter(|p| !p.is_empty()))
        .and_then(|prog| reg::default_value(HKEY_CLASSES_ROOT, &format!(r"{prog}\shell\open\command")))
        .map(|c| program_of(&c).to_lowercase());
    let mut out: Vec<Browser> = Vec::new();
    for (root, path) in [
        (HKEY_CURRENT_USER, r"SOFTWARE\Clients\StartMenuInternet"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Clients\StartMenuInternet"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Clients\StartMenuInternet"),
    ] {
        for key in reg::subkeys(root, path) {
            let Some(cmd) = reg::default_value(root, &format!(r"{path}\{key}\shell\open\command")) else { continue };
            let exe = program_of(&cmd);
            let file = exe.rsplit('\\').next().unwrap_or("").to_lowercase();
            if exe.is_empty() || file == "iexplore.exe" || out.iter().any(|b| b.path.eq_ignore_ascii_case(&exe)) || !std::path::Path::new(&exe).exists() {
                continue;
            }
            let name = reg::default_value(root, &format!(r"{path}\{key}")).filter(|n| !n.is_empty()).unwrap_or_else(|| key.clone());
            out.push(Browser { default: default.as_deref() == Some(exe.to_lowercase().as_str()), name, path: exe });
        }
    }
    // The default first, then by name.
    out.sort_by(|a, b| b.default.cmp(&a.default).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

/// Set once Scout's bar has been in the way of something Scout had to click (a dialog's buttons sit at the bottom
/// too): from then on, for the rest of the run, it goes at the top.
static BAR_AT_TOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Where Scout's bar goes for a browser window: the bottom middle, above the window's edge, or the top middle, below
/// the tabs and address bar, in physical pixels.
pub fn bar_spot(browser: RECT, w: i32, h: i32) -> (i32, i32) {
    let x = browser.left + ((browser.right - browser.left) - w) / 2;
    let y = if BAR_AT_TOP.load(std::sync::atomic::Ordering::SeqCst) { browser.top + 110 } else { browser.bottom - h - 28 };
    (x, y)
}

/// Whether the window belongs to Scout itself (its bar or cursor).
fn is_ours(hwnd: HWND) -> bool {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid == std::process::id()
}
