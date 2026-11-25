//! More than one account on a provider that signs in on this PC (Codex, Claude Code, Grok Build, OpenCode). Copilot has
//! several too, but it signs in with a token Scout keeps, so its accounts are a list in the settings instead (see
//! `Config::copilot_accounts`). Here each account is its own folder for
//! the CLI's sign-in and settings, and the one in use is the folder the CLI runs with. Every turn starts the CLI afresh,
//! so switching takes effect on the next message.
//!
//! The first account keeps the folder from before there could be several (`<base>`); the others get `<base>-<id>`.
//! Which accounts there are, and their names, are in `runtime/accounts.json`. No keys are kept there: each CLI keeps its
//! own sign-in in its folder, as before.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// The providers whose accounts are folders.
pub const PROVIDERS: [&str; 4] = ["codex", "claude", "grok", "opencode"];

/// Every provider that can have several accounts, Copilot's token list among them.
pub fn can_have_several(provider: &str) -> bool {
    provider == "copilot" || PROVIDERS.contains(&provider)
}

/// The first email in a sign-in file a CLI saved: a field called "email", or the "email" claim of a token in it.
pub fn email_in(path: &std::path::Path) -> Option<String> {
    fn find(v: &serde_json::Value) -> Option<String> {
        use base64::Engine;
        match v {
            serde_json::Value::Object(m) => {
                if let Some(e) = m.iter().find(|(k, _)| k.eq_ignore_ascii_case("email") || k.eq_ignore_ascii_case("emailAddress")).and_then(|(_, v)| v.as_str()) {
                    return Some(e.to_string());
                }
                m.values().find_map(find)
            }
            serde_json::Value::Array(a) => a.iter().find_map(find),
            // A JWT: its middle part is the claims.
            serde_json::Value::String(s) if s.matches('.').count() == 2 => {
                let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s.split('.').nth(1)?.trim_end_matches('=')).ok()?;
                serde_json::from_slice::<serde_json::Value>(&claims).ok()?.get("email")?.as_str().map(String::from)
            }
            _ => None,
        }
    }
    find(&serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?).filter(|e| e.contains('@'))
}

/// The id of the account whose folder is the one from before.
pub const FIRST: &str = "default";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Profile {
    pub id: String,
    /// Its email, once it's signed in and the CLI says.
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Profiles {
    pub active: String,
    pub list: Vec<Profile>,
}

impl Default for Profiles {
    fn default() -> Self {
        Profiles { active: FIRST.into(), list: vec![Profile { id: FIRST.into(), name: None }] }
    }
}

fn file() -> PathBuf {
    crate::config::home().join("runtime").join("accounts.json")
}

fn state() -> &'static Mutex<HashMap<String, Profiles>> {
    static S: OnceLock<Mutex<HashMap<String, Profiles>>> = OnceLock::new();
    S.get_or_init(|| {
        let saved: HashMap<String, Profiles> = std::fs::read_to_string(file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Mutex::new(saved.into_iter().filter(|(_, p)| !p.list.is_empty()).map(|(k, p)| (k, tidy(p))).collect())
    })
}

/// An active id that isn't in the list (a file edited by hand) falls back to the first account.
fn tidy(mut p: Profiles) -> Profiles {
    if !p.list.iter().any(|a| a.id == p.active) {
        p.active = p.list[0].id.clone();
    }
    p
}

fn save(all: &HashMap<String, Profiles>) {
    let _ = std::fs::create_dir_all(crate::config::home().join("runtime"));
    if let Ok(t) = serde_json::to_string_pretty(all) {
        let _ = std::fs::write(file(), t);
    }
}

fn edit<T>(provider: &str, f: impl FnOnce(&mut Profiles) -> T) -> T {
    let mut all = state().lock().unwrap();
    let p = all.entry(provider.to_string()).or_default();
    let out = f(p);
    *p = tidy(p.clone());
    save(&all);
    out
}

/// The accounts on `provider`, the one in use among them.
pub fn get(provider: &str) -> Profiles {
    state().lock().unwrap().get(provider).cloned().unwrap_or_default()
}

/// The folder of account `id` on a provider whose first folder is `base` (as "runtime/codex-home").
pub fn folder(base: &str, id: &str) -> PathBuf {
    let runtime = crate::config::home().join("runtime");
    if id == FIRST { runtime.join(base) } else { runtime.join(format!("{base}-{id}")) }
}

/// The folder the CLI runs with: the account in use.
pub fn home(provider: &str, base: &str) -> PathBuf {
    folder(base, &get(provider).active)
}

/// Adds an empty account and makes it the one in use, for signing in to. Says which was in use before.
pub fn add(provider: &str) -> (String, String) {
    let mut b = [0u8; 4];
    let _ = getrandom::fill(&mut b);
    let id: String = b.iter().map(|x| format!("{x:02x}")).collect();
    edit(provider, |p| {
        let before = std::mem::replace(&mut p.active, id.clone());
        p.list.push(Profile { id: id.clone(), name: None });
        (id.clone(), before)
    })
}

/// Makes account `id` the one in use. False when there's no such account.
pub fn switch(provider: &str, id: &str) -> bool {
    edit(provider, |p| {
        let known = p.list.iter().any(|a| a.id == id);
        if known {
            p.active = id.to_string();
        }
        known
    })
}

pub fn set_name(provider: &str, id: &str, name: Option<String>) {
    edit(provider, |p| {
        if let Some(a) = p.list.iter_mut().find(|a| a.id == id) {
            a.name = name;
        }
    });
}

/// Forgets account `id`, and deletes its folder unless it's the first one's (which another build may still use). The
/// first account left takes over if it was in use. Says whether any account is left.
pub fn remove(provider: &str, base: &str, id: &str) -> bool {
    if id != FIRST {
        let _ = std::fs::remove_dir_all(folder(base, id));
    }
    edit(provider, |p| {
        p.list.retain(|a| a.id != id);
        if p.list.is_empty() {
            *p = Profiles::default();
            return false;
        }
        true
    })
}

/// Every account's folder deleted and the list back to one empty account, as when the provider is uninstalled.
pub fn reset(provider: &str, base: &str) {
    for a in get(provider).list {
        let _ = std::fs::remove_dir_all(folder(base, &a.id));
    }
    edit(provider, |p| *p = Profiles::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_keep_the_first_account_where_it_was() {
        let first = folder("codex-home", FIRST);
        assert!(first.ends_with("codex-home"));
        assert!(folder("codex-home", "ab12").ends_with("codex-home-ab12"));
        let p = tidy(Profiles { active: "gone".into(), list: vec![Profile { id: "x".into(), name: None }] });
        assert_eq!(p.active, "x");
    }

    #[test]
    fn emails_are_found_in_sign_in_files() {
        use base64::Engine;
        let dir = std::env::temp_dir().join(format!("scout-profiles-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("auth.json");
        std::fs::write(&f, r#"{"user":{"name":"x","Email":"a@b.co"}}"#).unwrap();
        assert_eq!(email_in(&f).as_deref(), Some("a@b.co"));
        let jwt = format!("h.{}.s", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"email":"t@k.io"}"#));
        std::fs::write(&f, serde_json::json!({ "tokens": [{ "access": jwt }] }).to_string()).unwrap();
        assert_eq!(email_in(&f).as_deref(), Some("t@k.io"));
        std::fs::write(&f, r#"{"key":"sk-1","email":"not one"}"#).unwrap();
        assert_eq!(email_in(&f), None);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(can_have_several("copilot") && can_have_several("grok") && !can_have_several("kiro"));
    }
}
