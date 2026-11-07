use crate::api::{Content, Message};
use crate::config::home;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
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

/// A saved conversation. The old CLI used the same format, so its chats can still be resumed.
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
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
    Session { id, cwd: cwd.to_string(), created: now_ms(), updated: now_ms(), title: String::new(), model: model.to_string(), messages: vec![], totals: Totals::default(), mode: None, provider: None }
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

/// A conversation found in a standalone Claude Code (`~/.claude`) or Codex (`~/.codex`) install on this PC.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DetectedChat {
    pub id: String,
    /// `"claude"` or `"codex"`.
    pub source: String,
    pub cwd: String,
    pub project: String,
    pub title: String,
    pub messages: usize,
    pub updated: u64,
    pub ago: String,
}

fn user_home() -> PathBuf {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default()
}

fn claude_home() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| user_home().join(".claude"))
}

fn codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| user_home().join(".codex"))
}

fn walk_jsonl(root: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 || !root.is_dir() {
        return;
    }
    let Ok(rd) = std::fs::read_dir(root) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name();
        let name_str = name.to_string_lossy();
        if p.is_dir() {
            if name_str != "subagents" {
                walk_jsonl(&p, depth - 1, out);
            }
        } else if p.extension().is_some_and(|x| x == "jsonl") {
            out.push(p);
        }
    }
}

fn extract_text_blocks(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|b| {
                let kind = b.get("type").and_then(Value::as_str).unwrap_or("text");
                if matches!(kind, "text" | "input_text" | "output_text") {
                    b.get("text").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn is_system_injection(t: &str) -> bool {
    let t = t.trim_start();
    t.starts_with("<environment_context>")
        || t.starts_with("<user_instructions>")
        || t.starts_with("<permissions")
        || t.starts_with("<INSTRUCTIONS>")
        || t.starts_with("# AGENTS.md")
        || t.starts_with("<command-name>")
        || t.starts_with("<local-command-")
        || t.starts_with("[Request interrupted")
}

fn push_turn(messages: &mut Vec<Message>, role: &str, text: String) {
    let text = text.trim();
    if text.is_empty() || is_system_injection(text) {
        return;
    }
    match (role, messages.last_mut()) {
        ("user", Some(Message::User { content: Content::Text(prev) })) => {
            if prev != text {
                prev.push_str("\n\n");
                prev.push_str(text);
            }
        }
        ("assistant", Some(Message::Assistant { content: Some(prev), .. })) => {
            if prev != text {
                prev.push_str("\n\n");
                prev.push_str(text);
            }
        }
        ("user", _) => messages.push(Message::User { content: Content::Text(text.to_string()) }),
        ("assistant", _) => messages.push(Message::Assistant { content: Some(text.to_string()), reasoning_content: None, tool_calls: None, model: None, provider: None }),
        _ => {}
    }
}

fn parse_external_file(source: &str, file: &Path) -> Option<Session> {
    let meta = std::fs::metadata(file).ok()?;
    if meta.len() == 0 || meta.len() > 16 * 1024 * 1024 {
        return None;
    }
    let updated = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or_else(now_ms);
    let raw = std::fs::read_to_string(file).ok()?;

    let mut cwd = String::new();
    let mut summary = String::new();
    let mut messages = Vec::new();
    let mut event_messages = Vec::new();

    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if cwd.is_empty() {
            if let Some(c) = v.get("cwd").and_then(Value::as_str).or_else(|| v.pointer("/payload/cwd").and_then(Value::as_str)).filter(|s| !s.trim().is_empty()) {
                cwd = c.to_string();
            }
        }
        let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
        if source == "claude" {
            if kind == "summary" {
                if let Some(s) = v.get("summary").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
                    summary = s.to_string();
                }
                continue;
            }
            let msg = v.get("message").unwrap_or(&v);
            let role = msg.get("role").and_then(Value::as_str).unwrap_or(kind);
            if matches!(role, "user" | "assistant") {
                let text = extract_text_blocks(msg.get("content"));
                push_turn(&mut messages, role, text);
            }
        } else {
            if kind == "event_msg" {
                if let Some(p) = v.get("payload") {
                    match p.get("type").and_then(Value::as_str).unwrap_or("") {
                        "user_message" => {
                            if let Some(m) = p.get("message").or_else(|| p.get("text")).and_then(Value::as_str) {
                                push_turn(&mut event_messages, "user", m.to_string());
                            }
                        }
                        "agent_message" => {
                            if let Some(m) = p.get("message").or_else(|| p.get("text")).and_then(Value::as_str) {
                                push_turn(&mut event_messages, "assistant", m.to_string());
                            }
                        }
                        _ => {}
                    }
                }
                continue;
            }
            let item = if kind == "response_item" { v.get("payload").unwrap_or(&v) } else { &v };
            let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
            if item_type == "message" || matches!(kind, "user" | "assistant") {
                let role = item.get("role").and_then(Value::as_str).unwrap_or(kind);
                if matches!(role, "user" | "assistant") {
                    let text = extract_text_blocks(item.get("content"));
                    push_turn(&mut messages, role, text);
                }
            }
        }
    }

    if !event_messages.is_empty() {
        messages = event_messages;
    }
    if !messages.iter().any(|m| matches!(m, Message::User { .. })) {
        return None;
    }
    if cwd.is_empty() {
        cwd = user_home().to_string_lossy().to_string();
    }
    let first_user = messages.iter().find_map(|m| match m {
        Message::User { content } => Some(content.text()),
        _ => None,
    }).unwrap_or_default();
    let fallback_title: String = first_user.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect();
    let title: String = if !summary.is_empty() { summary.chars().take(80).collect() } else if !fallback_title.is_empty() { fallback_title } else { "Imported chat".into() };

    let mut hash = Sha256::new();
    hash.update(source.as_bytes());
    hash.update(b":");
    hash.update(file.to_string_lossy().as_bytes());
    let digest = hash.finalize();
    let hex: String = digest[..6].iter().map(|b| format!("{b:02x}")).collect();
    let id = format!("{source}-{hex}");
    let model = if source == "claude" { "claude/auto".into() } else { "codex/auto".into() };

    Some(Session {
        id,
        cwd,
        created: updated,
        updated,
        title,
        model,
        messages,
        totals: Totals::default(),
        mode: None,
        provider: if source == "claude" { Some("claude".into()) } else { Some("codex".into()) },
    })
}

pub fn collect_external_sessions(claude_root: &Path, codex_root: &Path, sessions_dir: &Path) -> Vec<(String, Session)> {
    let mut claude_files = Vec::new();
    walk_jsonl(&claude_root.join("projects"), 3, &mut claude_files);
    let mut codex_files = Vec::new();
    walk_jsonl(&codex_root.join("sessions"), 4, &mut codex_files);
    walk_jsonl(&codex_root.join("archived_sessions"), 2, &mut codex_files);

    let mut found = Vec::new();
    for f in claude_files {
        if let Some(s) = parse_external_file("claude", &f) {
            if !sessions_dir.join(format!("{}.json", s.id)).exists() {
                found.push(("claude".to_string(), s));
            }
        }
    }
    for f in codex_files {
        if let Some(s) = parse_external_file("codex", &f) {
            if !sessions_dir.join(format!("{}.json", s.id)).exists() {
                found.push(("codex".to_string(), s));
            }
        }
    }
    found.sort_by(|a, b| b.1.updated.cmp(&a.1.updated));
    found.truncate(40);
    found
}

pub fn detect_external_chats() -> Vec<DetectedChat> {
    collect_external_sessions(&claude_home(), &codex_home(), &dir())
        .into_iter()
        .map(|(source, s)| {
            let project = Path::new(&s.cwd).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| s.cwd.clone());
            DetectedChat {
                id: s.id,
                source,
                cwd: s.cwd,
                project,
                title: s.title,
                messages: s.messages.len(),
                ago: ago(s.updated),
                updated: s.updated,
            }
        })
        .collect()
}

pub fn import_external_sessions(ids: &[String]) -> Vec<Session> {
    import_external_from(&claude_home(), &codex_home(), &dir(), ids)
}

pub fn import_external_from(claude_root: &Path, codex_root: &Path, sessions_dir: &Path, ids: &[String]) -> Vec<Session> {
    let _ = std::fs::create_dir_all(sessions_dir);
    let mut imported = Vec::new();
    for (_, s) in collect_external_sessions(claude_root, codex_root, sessions_dir) {
        if !ids.iter().any(|id| id == &s.id) {
            continue;
        }
        let file = sessions_dir.join(format!("{}.json", s.id));
        if let Ok(bytes) = serde_json::to_vec(&s) {
            if std::fs::write(&file, bytes).is_ok() {
                imported.push(s);
            }
        }
    }
    imported
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_and_imports_claude_and_codex_chats() {
        let base = std::env::temp_dir().join(format!("scout-ext-chats-{}", now_ms()));
        let claude = base.join("claude");
        let codex = base.join("codex");
        let sessions = base.join("sessions");
        std::fs::create_dir_all(claude.join("projects").join("proj-a")).unwrap();
        std::fs::create_dir_all(codex.join("sessions").join("2026").join("11").join("05")).unwrap();

        let claude_jsonl = claude.join("projects").join("proj-a").join("chat1.jsonl");
        std::fs::write(
            &claude_jsonl,
            concat!(
                r#"{"type":"user","cwd":"C:\\dev\\alpha","message":{"role":"user","content":"Fix the auth middleware bug"}}"#, "\n",
                r#"{"type":"assistant","cwd":"C:\\dev\\alpha","message":{"role":"assistant","content":[{"type":"text","text":"I fixed the token check in auth.ts."}]}}"#, "\n"
            ),
        ).unwrap();

        let codex_jsonl = codex.join("sessions").join("2026").join("11").join("05").join("rollout-1.jsonl");
        std::fs::write(
            &codex_jsonl,
            concat!(
                r#"{"type":"session_meta","payload":{"id":"1","cwd":"C:\\dev\\beta"}}"#, "\n",
                r#"{"type":"event_msg","payload":{"type":"user_message","message":"Add dark mode support"}}"#, "\n",
                r#"{"type":"event_msg","payload":{"type":"agent_message","message":"Added dark theme variables."}}"#, "\n"
            ),
        ).unwrap();

        let found = collect_external_sessions(&claude, &codex, &sessions);
        assert_eq!(found.len(), 2);
        let ids: Vec<String> = found.iter().map(|(_, s)| s.id.clone()).collect();
        let imported = import_external_from(&claude, &codex, &sessions, &ids);
        assert_eq!(imported.len(), 2);
        assert!(collect_external_sessions(&claude, &codex, &sessions).is_empty(), "already imported chats are skipped");
        let _ = std::fs::remove_dir_all(&base);
    }
}
