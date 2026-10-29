use crate::api::Message;
use crate::config::home;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost: f64,
    pub requests: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<f64>,
    #[serde(default)]
    pub cached_tokens: u64,
}

/// The CLI's session format, so a conversation started in one can be resumed in the other.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Session {
    pub id: String,
    pub cwd: String,
    pub created: u64,
    pub updated: u64,
    pub title: String,
    pub model: String,
    /// Without the system prompt, which is rebuilt each run.
    pub messages: Vec<Message>,
    pub totals: Totals,
    /// The permission mode last told to the model in the messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn dir() -> PathBuf {
    home().join("sessions")
}

pub fn new_session(cwd: &str, model: &str) -> Session {
    let secs = now_ms() / 1000;
    let (y, mo, d, h, mi, s) = civil(secs);
    let mut rnd = [0u8; 2];
    let _ = getrandom::fill(&mut rnd);
    let id = format!("{y:04}{mo:02}{d:02}{h:02}{mi:02}{s:02}-{:04x}", u16::from_le_bytes(rnd));
    Session { id, cwd: cwd.to_string(), created: now_ms(), updated: now_ms(), title: String::new(), model: model.to_string(), messages: vec![], totals: Totals::default(), mode: None }
}

/// UTC date and time from Unix seconds.
pub fn civil(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d, (rem / 3600) as u32, ((rem % 3600) / 60) as u32, (rem % 60) as u32)
}

pub fn save_session(s: &mut Session) {
    if s.messages.is_empty() {
        return;
    }
    s.updated = now_ms();
    if s.title.is_empty() {
        let first = s.messages.iter().find_map(|m| match m {
            Message::User { content } => Some(crate::attach::split(&content.text()).0),
            _ => None,
        });
        let t: String = first.unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect();
        s.title = if t.is_empty() { "untitled".into() } else { t };
    }
    let _ = std::fs::create_dir_all(dir());
    let file = dir().join(format!("{}.json", s.id));
    let tmp = file.with_extension("json.tmp");
    if let Ok(text) = serde_json::to_vec(s) {
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &file);
        }
    }
}

pub fn load_session(id: &str) -> Option<Session> {
    serde_json::from_slice(&std::fs::read(dir().join(format!("{id}.json"))).ok()?).ok()
}

pub fn delete_session(id: &str) -> std::io::Result<()> {
    std::fs::remove_file(dir().join(format!("{id}.json")))
}

/// Ids of the saved chats whose title or text contains `q`, ignoring case. Reads every file, so run it off the UI
/// thread.
pub fn search_sessions(q: &str) -> Vec<String> {
    let q = q.to_lowercase();
    let mut out = vec![];
    for e in std::fs::read_dir(dir()).into_iter().flatten().flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let Ok(s) = std::fs::read(&p).map(|b| serde_json::from_slice::<Session>(&b)) else { continue };
        let Ok(s) = s else { continue };
        let hit = s.title.to_lowercase().contains(&q)
            || s.messages.iter().any(|m| match m {
                Message::User { content } => crate::attach::split(&content.text()).0.to_lowercase().contains(&q),
                Message::Assistant { content: Some(c), .. } => c.to_lowercase().contains(&q),
                _ => false,
            });
        if hit {
            out.push(s.id);
        }
    }
    out
}

#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub id: String,
    pub cwd: String,
    pub title: String,
    pub updated: u64,
}

#[derive(Deserialize)]
struct Head {
    id: String,
    cwd: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    updated: u64,
}

pub fn same_dir(a: &str, b: &str) -> bool {
    let n = |p: &str| {
        let p = Path::new(p).components().collect::<PathBuf>();
        let s = p.to_string_lossy().to_string();
        if cfg!(windows) { s.to_lowercase() } else { s }
    };
    n(a) == n(b)
}

/// Every saved session in every folder, newest first.
pub fn list_sessions() -> Vec<SessionInfo> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir()).into_iter().flatten().flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let Ok(bytes) = std::fs::read(&p) else { continue };
        let Ok(h) = serde_json::from_slice::<Head>(&bytes) else { continue };
        out.push(SessionInfo { id: h.id, cwd: h.cwd, title: h.title, updated: h.updated });
    }
    out.sort_by(|a, b| b.updated.cmp(&a.updated));
    out
}

pub fn ago(ts: u64) -> String {
    let s = now_ms().saturating_sub(ts) / 1000;
    match s {
        0..60 => format!("{s}s ago"),
        60..3600 => format!("{}m ago", s / 60),
        3600..86_400 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86_400),
    }
}
