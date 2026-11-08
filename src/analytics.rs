//! Anonymous usage analytics, sent to PostHog: which version runs, and that a message was sent on which provider and
//! model. Never the text of a message, a file, a folder or a key. Settings > Privacy turns it off, as does
//! DO_NOT_TRACK=1, and debug builds never send anything.
//!
//! Each install has a random id. Once the user signs in, events go under their ScoutCode username instead, so the same
//! person counts once across this app and scoutcode.dev.

use serde_json::{Map, Value, json};
use std::sync::Mutex;

/// PostHog's project key is public by design: it can only send events, never read them.
const KEY: &str = "phc_Cny9mBXrY9fnm7TPaECVK9NDfUCWEosjVJUHHcmHcvV7";
const URL: &str = "https://us.i.posthog.com/i/v0/e/";

struct State {
    on: bool,
    install_id: String,
    user: Option<String>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn allowed() -> bool {
    !cfg!(debug_assertions) && std::env::var("DO_NOT_TRACK").map_or(true, |v| v.is_empty() || v == "0")
}

/// Starts analytics with the saved choice and account. Makes the install's random id on first run.
pub fn init(on: bool, user: Option<String>) {
    let saved = crate::config::read_saved();
    let install_id = match saved.get("analyticsId").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        Some(id) => id.to_string(),
        None => {
            let mut b = [0u8; 16];
            let _ = getrandom::fill(&mut b);
            let id = b.iter().map(|x| format!("{x:02x}")).collect::<String>();
            crate::config::save_one("analyticsId", json!(id));
            id
        }
    };
    *STATE.lock().unwrap() = Some(State { on, install_id, user });
}

pub fn set_enabled(on: bool) {
    if let Some(s) = STATE.lock().unwrap().as_mut() {
        s.on = on;
    }
}

/// Signing in links this install's earlier events to the account; signing out goes back to the install's id.
pub fn set_user(user: Option<String>) {
    let anon = {
        let mut guard = STATE.lock().unwrap();
        let Some(s) = guard.as_mut() else { return };
        if s.user == user {
            return;
        }
        s.user = user.clone();
        s.install_id.clone()
    };
    if user.is_some() {
        track("$identify", json!({ "$anon_distinct_id": anon }));
    }
}

/// Sends one event in the background. Failures are dropped: analytics never gets in the way.
pub fn track(event: &str, props: Value) {
    if !allowed() {
        return;
    }
    let distinct_id = {
        let guard = STATE.lock().unwrap();
        let Some(s) = guard.as_ref().filter(|s| s.on) else { return };
        s.user.clone().unwrap_or_else(|| s.install_id.clone())
    };
    let mut properties = match props {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    properties.insert("platform".into(), json!("desktop"));
    properties.insert("app_version".into(), json!(crate::update::VERSION));
    properties.insert("$os".into(), json!(os_name()));
    properties.insert("$lib".into(), json!("scout-desktop"));
    let body = json!({ "api_key": KEY, "event": event, "distinct_id": distinct_id, "properties": properties });
    std::thread::spawn(move || {
        let _ = crate::api::http().post(URL).send_json(body);
    });
}

fn os_name() -> &'static str {
    match std::env::consts::OS {
        "windows" => "Windows",
        "macos" => "Mac OS X",
        "linux" => "Linux",
        other => other,
    }
}
