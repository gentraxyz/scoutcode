use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;

pub const MODES: [&str; 5] = ["default", "acceptEdits", "auto", "plan", "yolo"];
pub const EFFORTS: [&str; 5] = ["default", "none", "low", "medium", "high"];

pub fn mode_label(mode: &str) -> &'static str {
    match mode {
        "acceptEdits" => "Accept edits",
        "auto" => "Auto (Jev reviews)",
        "plan" => "Plan (read-only)",
        "yolo" => "Yolo (never ask)",
        _ => "Ask first",
    }
}

/// Same file and fields as the CLI (~/.scoutcode/config.json), so one login and one allow list serve both.
#[derive(Clone, Debug)]
pub struct Config {
    pub api_key: Option<String>,
    /// Who the key belongs to, for the sidebar. Saved at sign-in and refreshed from the model list.
    pub username: Option<String>,
    pub base_url: String,
    pub model: String,
    pub mode: String,
    pub effort: String,
    /// Shell command prefixes allowed without asking, per project directory.
    pub allow: HashMap<String, Vec<String>>,
    pub auto_compact_tokens: u64,
    pub prune_tokens: u64,
    pub subagent_model: String,
    pub temperature: Option<f64>,
    /// Desktop app only: models pinned to the top of the picker.
    pub starred_models: Vec<String>,
    /// Desktop app only: named folders that chats are grouped under.
    pub projects: Vec<Project>,
    /// Desktop app only: how to get the user's attention when a turn ends or needs them while the window is in the
    /// background. None until they choose, which the first run asks.
    pub notify_flash: Option<bool>,
    pub notify_toast: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Project {
    pub name: String,
    pub path: String,
}

pub fn home() -> PathBuf {
    if let Ok(h) = std::env::var("SCOUT_HOME") {
        return PathBuf::from(h);
    }
    let base = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).unwrap_or_default();
    PathBuf::from(base).join(".scoutcode")
}

fn file() -> PathBuf {
    home().join("config.json")
}

fn read_saved() -> Map<String, Value> {
    std::fs::read_to_string(file())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

pub fn load_config() -> Config {
    let s = read_saved();
    let str_ = |k: &str, d: &str| s.get(k).and_then(Value::as_str).unwrap_or(d).to_string();
    let num = |k: &str, d: u64| s.get(k).and_then(Value::as_f64).map(|n| n as u64).unwrap_or(d);
    let allow = s
        .get("allow")
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()))
                .collect()
        })
        .unwrap_or_default();
    let mut cfg = Config {
        api_key: s.get("apiKey").and_then(Value::as_str).filter(|k| !k.is_empty()).map(String::from),
        username: s.get("username").and_then(Value::as_str).filter(|u| !u.is_empty()).map(String::from),
        base_url: str_("baseUrl", "https://scoutcode.dev/v1"),
        model: str_("model", "auto"),
        mode: str_("mode", "default"),
        effort: str_("effort", "default"),
        allow,
        auto_compact_tokens: num("autoCompactTokens", 90_000),
        prune_tokens: num("pruneTokens", 60_000),
        subagent_model: str_("subagentModel", "qwen3.8-27b"),
        temperature: s.get("temperature").and_then(Value::as_f64),
        starred_models: s.get("starredModels").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default(),
        projects: s.get("desktopProjects").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default(),
        notify_flash: s.get("desktopFlash").and_then(Value::as_bool),
        notify_toast: s.get("desktopToast").and_then(Value::as_bool),
    };
    // Environment variables win over the file, so one-off runs need no login.
    if let Ok(k) = std::env::var("SCOUTCODE_API_KEY") {
        cfg.api_key = Some(k);
    }
    if let Ok(u) = std::env::var("SCOUTCODE_BASE_URL") {
        cfg.base_url = u;
    }
    cfg.base_url = cfg.base_url.trim_end_matches('/').to_string();
    cfg
}

/// Saves the given fields, keeping anything else already in the file. Null removes a field.
pub fn save_config(patch: Map<String, Value>) {
    let mut saved = read_saved();
    for (k, v) in patch {
        if v.is_null() {
            saved.remove(&k);
        } else {
            saved.insert(k, v);
        }
    }
    let _ = std::fs::create_dir_all(home());
    if let Ok(text) = serde_json::to_string_pretty(&Value::Object(saved)) {
        let _ = std::fs::write(file(), text);
    }
}

pub fn save_one(key: &str, value: Value) {
    let mut m = Map::new();
    m.insert(key.to_string(), value);
    save_config(m);
}

pub fn is_sign_in(token: Option<&str>) -> bool {
    token.is_some_and(|t| t.starts_with("scout-oauth-"))
}
