//! ScoutCode Delta, the issue tracker. The UI (ui/src/delta) keeps the logic; this file only stores local workspaces
//! and carries cloud requests.
//!
//! A local workspace is one JSON file in ~/.scoutcode/delta, named by its id. A cloud workspace lives on the ScoutCode
//! server, and the UI reaches it through `delta_cloud`, which adds the account's key so the webview never sees it.

use crate::api::http;
use crate::config::Config;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;

fn dir() -> PathBuf {
    crate::config::home().join("delta")
}

/// Ids are made by the UI; anything else could reach outside the folder.
fn file(id: &str) -> Result<PathBuf, String> {
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("Not a workspace id".into());
    }
    Ok(dir().join(format!("{id}.json")))
}

/// Every local workspace: its file's `meta`, which holds the id and name.
pub fn list() -> Vec<Value> {
    let Ok(entries) = std::fs::read_dir(dir()) else { return vec![] };
    let mut out: Vec<Value> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|t| serde_json::from_str::<Value>(&t).ok())
        .filter_map(|v| v.get("meta").cloned())
        .collect();
    out.sort_by(|a, b| a["name"].as_str().unwrap_or("").to_lowercase().cmp(&b["name"].as_str().unwrap_or("").to_lowercase()));
    out
}

pub fn read(id: &str) -> Result<Option<String>, String> {
    match std::fs::read_to_string(file(id)?) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Written beside the old file and then moved over it, so a crash never leaves half a workspace.
pub fn write(id: &str, text: &str) -> Result<(), String> {
    serde_json::from_str::<Value>(text).map_err(|e| format!("Not JSON: {e}"))?;
    let path = file(id)?;
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

pub fn delete(id: &str) -> Result<(), String> {
    match std::fs::remove_file(file(id)?) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

/// A request to the server's Delta API (/api/account/delta…) as the signed-in account.
pub fn cloud(cfg: &Config, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    if !path.starts_with("/api/account/delta") || path.contains("..") {
        return Err("Not a Delta path".into());
    }
    let Some(key) = cfg.api_key.as_deref().filter(|k| !k.is_empty()) else {
        return Err("Sign in to your ScoutCode account to use cloud workspaces.".into());
    };
    let site = cfg.base_url.trim_end_matches('/').trim_end_matches("/v1");
    let url = format!("{site}{path}");
    let agent = http();
    let res = match method {
        "GET" => agent
            .get(&url)
            .config()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .header("Authorization", &format!("Bearer {key}"))
            .call(),
        "POST" => agent
            .post(&url)
            .config()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .header("Authorization", &format!("Bearer {key}"))
            .header("x-scoutcode", "1")
            .send_json(body.unwrap_or(Value::Null)),
        _ => return Err("GET or POST only".into()),
    }
    .map_err(|e| format!("Could not reach ScoutCode: {e}"))?;
    let status = res.status().as_u16();
    let data: Value = res.into_body().read_json().unwrap_or(Value::Null);
    if status >= 400 {
        let msg = data["error"].as_str().map(String::from).unwrap_or_else(|| format!("HTTP {status}"));
        return Err(msg);
    }
    Ok(data)
}
