//! Hermes, on Nous Research's Portal, with the user's own account: a browser sign-in or an API key. Unlike the other providers
//! with a sign-in, nothing is installed. Requests go straight to Nous' OpenAI-style API, so Scout's own tools, permissions and undo
//! all apply. Models come from the catalog in `api.rs`.
//!
//! A sign-in's refresh token is single-use, and Nous revokes the whole sign-in if one is used twice. So every refresh happens
//! under one lock with the file re-read first, and the new pair is on disk before its token goes anywhere. The sign-in is in its
//! own file, away from `config.json`, which other threads write whole.

use crate::api::{ApiError, http};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PORTAL: &str = "https://portal.nousresearch.com";
/// Hermes' own client: Nous offers no way to register another. The user agent still says it's Scout.
const CLIENT_ID: &str = "hermes-cli";
const SCOPE: &str = "inference:invoke";
/// A token this close to expiring is refreshed first, so it can't run out during a request.
const REFRESH_WITHIN: u64 = 120;
const POLL_CAP: u64 = 10;
pub const PLAN_URL: &str = "https://portal.nousresearch.com/manage-subscription";
/// Nous' 402 is about the user's Portal account, which ScoutCode's admins have nothing to do with.
pub const NO_CREDIT: &str = "This model needs credit on your Nous Portal account. Pick a model marked Free, or add credit on the Portal.";

/// How signing in is going, for the window.
pub enum Update {
    /// The code to approve on Nous' page, which should be open by now.
    Code { code: String, url: String },
    SignedIn,
    Failed(String),
}

#[derive(Serialize, Deserialize)]
struct Login {
    access: String,
    refresh: Option<String>,
    /// Seconds since 1970.
    expires: u64,
}

fn file() -> PathBuf {
    crate::config::home().join("hermes-login.json")
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn read(file: &Path) -> Option<Login> {
    serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()
}

/// Replaces the file in one step, so a crash can't lose the only copy of a refresh token.
fn write(file: &Path, login: &Login) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let staged = file.with_extension("tmp");
    std::fs::write(&staged, serde_json::to_vec_pretty(login)?)?;
    std::fs::rename(staged, file)
}

/// Held for a whole refresh, across chats.
static LOCK: Mutex<()> = Mutex::new(());

pub fn signed_in() -> bool {
    read(&file()).is_some()
}

pub fn sign_out() {
    let _one_at_a_time = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = std::fs::remove_file(file());
}

/// What a refresh came to.
#[derive(Debug, PartialEq)]
enum Refreshed {
    New(String, Option<String>, u64),
    /// Nous refused the refresh token, so only signing in again will help.
    Dead,
    /// Says nothing about the token: the network, an outage or Nous' firewall.
    Later(String),
}

fn refresh(portal: &str, token: &str) -> Refreshed {
    let res = http()
        .post(&format!("{portal}/api/oauth/token"))
        .config()
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .header("x-nous-refresh-token", token)
        .send_form([("grant_type", "refresh_token"), ("client_id", CLIENT_ID)]);
    let mut res = match res {
        Ok(r) => r,
        Err(e) => return Refreshed::Later(e.to_string()),
    };
    let status = res.status().as_u16();
    let firewall = res.headers().contains_key("x-vercel-mitigated");
    let body: Value = res.body_mut().read_json().unwrap_or_default();
    let text = |k: &str| body.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
    if status == 200 {
        return match text("access_token") {
            Some(access) => Refreshed::New(access, text("refresh_token"), body.get("expires_in").and_then(Value::as_u64).unwrap_or(0)),
            None => Refreshed::Dead,
        };
    }
    if firewall || status >= 500 {
        return Refreshed::Later(format!("Nous Portal answered HTTP {status}"));
    }
    // Only an explicit answer about the grant ends a sign-in: a 404 or 429 from a gateway says nothing about it.
    let code = text("error").or_else(|| matches!(status, 401 | 403).then(|| "invalid_grant".to_string()));
    let reused = text("error_description").is_some_and(|d| d.to_lowercase().contains("reuse"));
    if reused || matches!(code.as_deref(), Some("invalid_grant" | "invalid_token" | "refresh_token_reused")) {
        return Refreshed::Dead;
    }
    Refreshed::Later(format!("Nous Portal answered HTTP {status}"))
}

/// The token to send: the sign-in's if there is one, refreshed if it needs it, otherwise the key.
pub fn bearer(key: Option<&str>) -> Result<String, ApiError> {
    bearer_from(&file(), PORTAL, key)
}

fn bearer_from(file: &Path, portal: &str, key: Option<&str>) -> Result<String, ApiError> {
    let _one_at_a_time = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let key_or = |message: &str| key.map(String::from).ok_or_else(|| ApiError::new(message, 401));
    let Some(login) = read(file) else {
        return key_or("Not signed in to Hermes. Sign in or add a key under Settings > Providers.");
    };
    let at = now();
    if login.expires > at + REFRESH_WITHIN {
        return Ok(login.access);
    }
    let ended = "Your Hermes sign-in has ended. Sign in again under Settings > Providers.";
    let Some(token) = login.refresh.as_deref() else {
        let _ = std::fs::remove_file(file);
        return key_or(ended);
    };
    match refresh(portal, token) {
        Refreshed::New(access, rotated, expires_in) => {
            let next = Login { access: access.clone(), refresh: rotated.or(login.refresh), expires: at + expires_in };
            // The old pair is spent, so what's left is to say so if the new one can't be kept.
            write(file, &next).map_err(|e| ApiError::new(format!("Couldn't save your Hermes sign-in: {e}"), 500))?;
            Ok(access)
        }
        Refreshed::Dead => {
            let _ = std::fs::remove_file(file);
            key_or(ended)
        }
        // The token still works for the seconds it has left. Otherwise a retry may get through.
        Refreshed::Later(_) if login.expires > at => Ok(login.access),
        Refreshed::Later(e) => Err(ApiError::new(format!("Couldn't renew your Hermes sign-in: {e}"), 503)),
    }
}

// ---------- signing in ----------

fn post_form(url: &str, form: &[(&str, &str)]) -> Result<Value, String> {
    let mut res = http().post(url).config().timeout_global(Some(Duration::from_secs(20))).build().send_form(form.iter().copied()).map_err(|e| e.to_string())?;
    res.body_mut().read_json().map_err(|e| e.to_string())
}

/// Waits for the user to approve `device_code`, for as long as the code is good.
fn poll_token(portal: &str, device_code: &str, mut wait: u64, expires_in: u64, cancel: &AtomicBool) -> Result<Login, String> {
    let deadline = Instant::now() + Duration::from_secs(expires_in);
    let url = format!("{portal}/api/oauth/token");
    while Instant::now() < deadline {
        let next = Instant::now() + Duration::from_secs(wait);
        while Instant::now() < next {
            if cancel.load(Ordering::Relaxed) {
                return Err(String::new());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        // A failed poll is usually the network, so the next one may work.
        let Ok(r) = post_form(&url, &[("grant_type", "urn:ietf:params:oauth:grant-type:device_code"), ("client_id", CLIENT_ID), ("device_code", device_code)]) else { continue };
        if let Some(access) = r.get("access_token").and_then(Value::as_str) {
            return Ok(Login { access: access.to_string(), refresh: r.get("refresh_token").and_then(Value::as_str).map(String::from), expires: now() + r.get("expires_in").and_then(Value::as_u64).unwrap_or(0) });
        }
        match r.get("error").and_then(Value::as_str) {
            Some("authorization_pending") | None => {}
            Some("slow_down") => wait += 5,
            Some("access_denied") => return Err("Sign-in was cancelled in the browser.".into()),
            Some("expired_token") => break,
            Some(e) => return Err(format!("Nous refused the sign-in: {e}")),
        }
    }
    // Accounts with no plan or credit have been reported never to get the approval button (hermes-agent#47950).
    Err("The sign-in code expired. If the page never offered to approve it, your Nous account may need a plan first: choose one on the Portal, or use an API key.".into())
}

/// Signs in with Nous' device flow. Only Nous' own pages get opened, whatever the answer says.
pub fn sign_in(cancel: Arc<AtomicBool>, send: impl Fn(Update) + Send + 'static) {
    std::thread::spawn(move || {
        let run = || -> Result<(), String> {
            let d = post_form(&format!("{PORTAL}/api/oauth/device/code"), &[("client_id", CLIENT_ID), ("scope", SCOPE)]).map_err(|e| format!("Couldn't reach Nous Portal: {e}"))?;
            let text = |k: &str| d.get(k).and_then(Value::as_str);
            let (Some(code), Some(device_code)) = (text("user_code"), text("device_code")) else {
                return Err(format!("Nous refused the sign-in: {}", text("error_description").or(text("error")).unwrap_or("no code came back")));
            };
            let url = text("verification_uri_complete").or(text("verification_uri")).filter(|u| u.starts_with(&format!("{PORTAL}/"))).ok_or("Nous sent a sign-in page Scout doesn't recognize.")?;
            send(Update::Code { code: code.to_string(), url: url.to_string() });
            crate::oauth::open_browser(url);
            let wait = d.get("interval").and_then(Value::as_u64).unwrap_or(5).clamp(1, POLL_CAP);
            let login = poll_token(PORTAL, device_code, wait, d.get("expires_in").and_then(Value::as_u64).unwrap_or(600), &cancel)?;
            let _one_at_a_time = LOCK.lock().unwrap_or_else(|e| e.into_inner());
            write(&file(), &login).map_err(|e| format!("Couldn't save your sign-in: {e}"))
        };
        match run() {
            Ok(()) => send(Update::SignedIn),
            Err(e) => send(Update::Failed(e)),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Answers one request with `status` and `body`, and hands back what it was asked.
    fn portal(status: &str, headers: &str, body: &str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let reply = format!("HTTP/1.1 {status}\r\ncontent-type: application/json\r\n{headers}content-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
        let served = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            // The headers and the form can arrive apart, so it reads until the form is all there.
            let (mut got, mut chunk) = (vec![], [0; 4096]);
            loop {
                let n = s.read(&mut chunk).unwrap();
                got.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&got).to_lowercase();
                let whole = text.split_once("\r\n\r\n").is_some_and(|(head, form)| {
                    let length = head.lines().find_map(|l| l.strip_prefix("content-length: ")).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                    form.len() >= length
                });
                if n == 0 || whole {
                    break;
                }
            }
            s.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&got).to_lowercase()
        });
        (url, served)
    }

    fn saved(name: &str, expires: u64) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("scout-hermes-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let f = dir.join("login.json");
        write(&f, &Login { access: "old-access".into(), refresh: Some("old-refresh".into()), expires }).unwrap();
        f
    }

    #[test]
    fn a_fresh_token_is_used_without_asking() {
        let f = saved("fresh", now() + 3600);
        assert_eq!(bearer_from(&f, "http://127.0.0.1:9", None).unwrap(), "old-access");
    }

    #[test]
    fn a_refresh_saves_the_new_pair_before_using_it() {
        let f = saved("rotate", now() + 10);
        let (url, served) = portal("200 OK", "", r#"{"access_token":"new-access","refresh_token":"new-refresh","expires_in":900}"#);
        assert_eq!(bearer_from(&f, &url, None).unwrap(), "new-access");
        let asked = served.join().unwrap();
        assert!(asked.contains("x-nous-refresh-token: old-refresh") && asked.contains("grant_type=refresh_token"), "{asked}");
        let kept = read(&f).unwrap();
        assert_eq!((kept.access.as_str(), kept.refresh.as_deref()), ("new-access", Some("new-refresh")));
        assert!(kept.expires > now() + 800);
    }

    #[test]
    fn a_refused_refresh_ends_the_sign_in_and_falls_back_to_the_key() {
        let f = saved("dead", 0);
        let (url, served) = portal("400 Bad Request", "", r#"{"error":"invalid_grant"}"#);
        assert_eq!(bearer_from(&f, &url, Some("sk-key")).unwrap(), "sk-key");
        served.join().unwrap();
        assert!(read(&f).is_none());
    }

    #[test]
    fn a_firewall_or_outage_keeps_the_sign_in() {
        let f = saved("firewall", now() + 30);
        let (url, served) = portal("403 Forbidden", "x-vercel-mitigated: challenge\r\n", "<html>checkpoint</html>");
        assert_eq!(bearer_from(&f, &url, None).unwrap(), "old-access", "the old token still has seconds left");
        served.join().unwrap();
        assert!(read(&f).is_some());

        let f = saved("outage", 0);
        let (url, served) = portal("503 Service Unavailable", "", "");
        let e = bearer_from(&f, &url, None).unwrap_err();
        served.join().unwrap();
        assert!(e.retryable(), "{}", e.message);
        assert!(read(&f).is_some());
    }

    #[test]
    fn no_sign_in_uses_the_key_or_says_so() {
        let f = std::env::temp_dir().join("scout-hermes-nothing-here.json");
        assert_eq!(bearer_from(&f, "http://127.0.0.1:9", Some("sk-key")).unwrap(), "sk-key");
        assert_eq!(bearer_from(&f, "http://127.0.0.1:9", None).unwrap_err().status, 401);
    }
}
