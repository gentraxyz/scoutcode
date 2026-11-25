//! A log of each run of Scout, kept in `~/.scoutcode/logs`, so a crash or a failure can be looked at afterwards.
//!
//! Every start writes a new `session-<UTC time>-<pid>.log` and only the newest [`KEEP`] stay. Each line is written and
//! flushed at once, so the lines before a crash are on disk. A panic writes its message, place and backtrace; a session
//! that ends without its closing line (a crash the panic hook couldn't see, a kill) is named in the next one's header.
//! Settings > Logs lists them to copy or save. Text goes through `redact` first, so keys and tokens stay out.

use crate::config::home;
use serde::Serialize;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// How many session logs stay, this one included.
pub const KEEP: usize = 3;
/// A log stops growing here, so a loop that logs can't fill the disk.
const MAX_BYTES: u64 = 25 * 1024 * 1024;
/// The last line of a session that closed normally.
const END: &str = "--- Scout closed normally ---";

struct Log {
    file: File,
    name: String,
}

static LOG: OnceLock<Mutex<Log>> = OnceLock::new();
static WRITTEN: AtomicU64 = AtomicU64::new(0);

pub fn dir() -> PathBuf {
    home().join("logs")
}

fn is_log(name: &str) -> bool {
    name.starts_with("session-") && name.ends_with(".log") && !name.contains(['/', '\\']) && !name.contains("..")
}

/// The session logs there, oldest first (the names sort by time).
fn names() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir())
        .map(|r| r.flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| is_log(n)).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// Whether a log ends with the line a normal close writes.
fn closed_normally(name: &str) -> bool {
    let Ok(mut f) = File::open(dir().join(name)) else { return true };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(256)));
    let mut tail = String::new();
    let _ = f.read_to_string(&mut tail);
    tail.contains(END)
}

fn stamp(ms: u64) -> String {
    let (y, mo, d, h, mi, s) = crate::session::civil(ms / 1000);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}.{:03}Z", ms % 1000)
}

/// Starts this session's log. Called once, early in `main`, by the copy of Scout that opens the window.
pub fn init() {
    let _ = std::fs::create_dir_all(dir());
    let before = names();
    let unclean = before.last().filter(|n| !closed_normally(n)).cloned();
    let now = crate::session::now_ms();
    let (y, mo, d, h, mi, s) = crate::session::civil(now / 1000);
    let name = format!("session-{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}-{}.log", std::process::id());
    let Ok(file) = File::create(dir().join(&name)) else { return };
    let _ = LOG.set(Mutex::new(Log { file, name: name.clone() }));
    // Older ones go, leaving this one and the newest before it.
    for old in before.iter().rev().skip(KEEP - 1) {
        let _ = std::fs::remove_file(dir().join(old));
    }
    install_panic_hook();
    info("start", format!("Scout {} ({}) on {} {}, pid {}", crate::update::VERSION, crate::update::COMMIT, crate::prompt::os_name(), std::env::consts::ARCH, std::process::id()));
    info("start", format!("Executable: {}", std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default()));
    info("start", format!("Data folder: {}", home().display()));
    if let Ok(v) = tauri::webview_version() {
        info("start", format!("WebView: {v}"));
    }
    if let Some(prev) = unclean {
        warn("start", format!("The last session ({prev}) didn't close normally: Scout crashed, froze or was ended."));
    }
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let what = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_else(|| "(no message)".into());
        let place = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_default();
        let trace = std::backtrace::Backtrace::force_capture();
        error("panic", format!("Thread '{}' panicked at {place}: {what}\n{trace}", thread.name().unwrap_or("unnamed")));
        previous(info);
    }));
}

fn write(level: &str, area: &str, msg: &str) {
    let Some(log) = LOG.get() else { return };
    if WRITTEN.load(Ordering::Relaxed) > MAX_BYTES {
        return;
    }
    let msg = crate::redact::redact(msg);
    let mut lines = msg.lines();
    let mut out = format!("{} {level:<5} [{area}] {}\n", stamp(crate::session::now_ms()), lines.next().unwrap_or(""));
    for l in lines {
        out.push_str("    ");
        out.push_str(l);
        out.push('\n');
    }
    let total = WRITTEN.fetch_add(out.len() as u64, Ordering::Relaxed) + out.len() as u64;
    if total > MAX_BYTES {
        out.push_str("--- The log reached its size limit; nothing more is written this session. ---\n");
    }
    // A panic while the lock is held (here) mustn't stop the panic itself from being written.
    let mut g = match log.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let _ = g.file.write_all(out.as_bytes());
    let _ = g.file.flush();
}

pub fn info(area: &str, msg: impl AsRef<str>) {
    write("INFO", area, msg.as_ref());
}

pub fn warn(area: &str, msg: impl AsRef<str>) {
    write("WARN", area, msg.as_ref());
}

pub fn error(area: &str, msg: impl AsRef<str>) {
    write("ERROR", area, msg.as_ref());
}

/// A line from the window: its errors, warnings and crashes.
pub fn ui(level: &str, msg: &str) {
    let level = match level {
        "error" => "ERROR",
        "warn" => "WARN",
        _ => "INFO",
    };
    // Long stacks and dumped objects are cut, so one message can't take the whole log.
    let msg: String = msg.chars().take(8000).collect();
    write(level, "ui", &msg);
}

/// The closing line, which tells the next session this one didn't crash.
pub fn end() {
    info("exit", END);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    /// When it was last written, in Unix ms.
    pub modified: u64,
    pub bytes: u64,
    /// This run's log.
    pub current: bool,
    /// It ended with the closing line. This run's log hasn't yet.
    pub closed: bool,
}

/// The kept logs, newest first.
pub fn list() -> Vec<Entry> {
    let current = LOG.get().map(|l| l.lock().map(|g| g.name.clone()).unwrap_or_default());
    names()
        .into_iter()
        .rev()
        .take(KEEP)
        .map(|name| {
            let meta = std::fs::metadata(dir().join(&name)).ok();
            let modified = meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64);
            let is_current = current.as_deref() == Some(name.as_str());
            Entry { closed: !is_current && closed_normally(&name), current: is_current, bytes: meta.map_or(0, |m| m.len()), modified, name }
        })
        .collect()
}

/// A kept log's text, by its name from [`list`].
pub fn read(name: &str) -> Result<String, String> {
    if !is_log(name) {
        return Err("That isn't one of Scout's logs.".into());
    }
    let bytes = std::fs::read(dir().join(name)).map_err(|e| format!("Couldn't read the log: {e}"))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_log_names_are_read() {
        assert!(is_log("session-20261125-101500-42.log"));
        assert!(!is_log("../config.json"));
        assert!(!is_log("session-..\\x.log"));
        assert!(read("config.json").is_err());
    }
}
