//! Codex, OpenAI's coding agent, on the user's own ChatGPT sign-in. It runs through its Agent Client Protocol adapter
//! (`@agentclientprotocol/codex-acp`), which starts Codex's own app server: see `acp.rs`. Both come from npm, pinned by
//! `src/npm/codex/package-lock.json`, and run on Scout's own Node: see `node.rs`. Scout never sees the sign-in's token: Codex
//! keeps it, in a folder only Scout's copy uses, apart from any other Codex on this PC.

use crate::acp::{Msg, Run, Server, modes_of, options_of};
use crate::node::Update;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::time::Duration;

pub const PLAN_URL: &str = "https://chatgpt.com/codex";

static AGENT: crate::node::Agent = crate::node::Agent {
    name: "Codex",
    folder: "codex",
    // The adapter's version. It only changes with a Scout release: see "Codex and Claude Code" in docs/providers.md.
    version: "2.5.0",
    package_json: include_str!("npm/codex/package.json"),
    lock: include_str!("npm/codex/package-lock.json"),
    entry: "node_modules/@agentclientprotocol/codex-acp/dist/index.js",
};

/// Its settings, in a folder that's Scout's alone: no update checks, and no history of its own, since the chat is Scout's.
const SETTINGS: &str = "check_for_update_on_startup = false\n\n[history]\npersistence = \"none\"\n";

/// Its sign-in, settings and sessions: Scout's alone.
fn own_home() -> PathBuf {
    crate::config::home().join("runtime").join("codex-home")
}

pub fn installed() -> bool {
    AGENT.installed()
}

pub fn ensure_runtime(progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
    AGENT.ensure(progress)
}

/// Deletes the adapter and Codex, with the sign-in and history.
pub fn remove_runtime() {
    AGENT.remove();
    let _ = std::fs::remove_dir_all(own_home());
}

/// Another copy's settings and keys stay with it. `CODEX_PATH` alone would run some other program as Codex, and an
/// OpenAI key would bill per token instead of the sign-in.
fn is_outside_setting(name: &str) -> bool {
    let name = name.to_uppercase();
    name.starts_with("CODEX_") || name.starts_with("OPENAI_") || name == "MODEL_PROVIDER" || name == "DEFAULT_AUTH_REQUEST"
}

/// The adapter, with its settings written first, so it never runs on anything else.
fn command() -> Command {
    own_env(AGENT.command())
}

/// `c` with Codex's own settings and folder, and nothing from another copy.
fn own_env(mut c: Command) -> Command {
    let home = own_home();
    let _ = std::fs::create_dir_all(&home).and_then(|_| std::fs::write(home.join("config.toml"), SETTINGS));
    for (k, _) in std::env::vars_os() {
        if is_outside_setting(&k.to_string_lossy()) {
            c.env_remove(k);
        }
    }
    c.env("CODEX_HOME", &home);
    c
}

// ---------- signing in ----------

pub fn signed_in() -> bool {
    own_home().join("auth.json").is_file()
}

pub fn sign_out() {
    if installed() {
        let mut c = command();
        c.args(["cli", "logout"]);
        crate::node::quietly(c);
    }
    let _ = std::fs::remove_file(own_home().join("auth.json"));
}

/// Installs Codex, then has it open ChatGPT's sign-in page, unless it's signed in already.
pub fn sign_in(cancel: Arc<AtomicBool>, send: impl Fn(Update) + Clone + Send + 'static) {
    crate::node::sign_in(
        "Codex",
        ensure_runtime,
        signed_in,
        || {
            let mut c = command();
            c.args(["cli", "login"]);
            c
        },
        cancel,
        send,
    );
}

// ---------- limits ----------

/// What's left of the plan's limits: Codex's own app server, asked directly, as the adapter does for its `/status`. It
/// uses none of the plan up. The answer goes to `quota`.
pub fn fetch_limits() -> Result<(), String> {
    if !installed() || !signed_in() {
        return Ok(());
    }
    let mut c = own_env(crate::node::command());
    c.arg(AGENT.dir().join("node_modules/@openai/codex/bin/codex.js")).arg("app-server");
    c.current_dir(crate::node::folder("codex-work")?);
    let hello = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"clientInfo": {"name": "scout-desktop", "version": env!("CARGO_PKG_VERSION")}}});
    let lines = [hello, json!({"jsonrpc": "2.0", "method": "initialized"}), json!({"jsonrpc": "2.0", "id": 2, "method": "account/rateLimits/read"})];
    let answer = crate::acp::exchange(c, "Codex", &lines, |v| v.get("id").and_then(Value::as_u64) == Some(2), Duration::from_secs(30))?;
    let result = answer.get("result").ok_or_else(|| answer.pointer("/error/message").and_then(Value::as_str).unwrap_or("Codex didn't say its limits.").to_string())?;
    crate::quota::record_codex(result);
    Ok(())
}

// ---------- the protocol ----------

fn open(cwd: &Path, tx: Sender<Msg>) -> Result<Server, String> {
    let mut c = command();
    c.current_dir(cwd);
    Server::open(c, "Codex", tx)
}

/// The models this account may pick, or why they can't be told.
pub fn list_models() -> Result<Vec<(String, String)>, String> {
    if !installed() {
        return Ok(vec![]);
    }
    let cwd = crate::node::folder("codex-work")?;
    let (tx, _rx) = crate::acp::messages();
    let mut server = open(&cwd, tx)?;
    let session = server.new_session(&cwd.to_string_lossy());
    if let Ok(s) = &session {
        server.delete_session(s);
    }
    server.stop();
    let models = options_of(&session?, "model");
    if models.is_empty() {
        return Err("Codex didn't list its models.".into());
    }
    Ok(models)
}

/// Codex's mode for Scout's: each asks about what Scout's would. Read-only asks before any edit, workspace access edits
/// the project without asking, auto review only asks about what looks unsafe, and full access never asks.
fn mode_for(mode: &str, quiet: bool) -> &'static str {
    match mode {
        _ if quiet => "read-only",
        "acceptEdits" => "workspace-write",
        "auto" => "agent",
        "yolo" => "agent-full-access",
        _ => "read-only",
    }
}

/// Starts a session and sends `prompt`. `quiet` is a one-off answer, which runs read-only in Codex's plan mode.
pub fn start(model: &str, cwd: &str, mode: &str, effort: &str, prompt: String, images: Vec<(String, String)>, quiet: bool) -> Result<Run, String> {
    let (tx, rx) = crate::acp::messages();
    let mut server = open(Path::new(cwd), tx.clone())?;
    let started = (|| -> Result<Value, String> {
        let wait = Duration::from_secs(60);
        // It takes explore's tool as an MCP server; when to use it goes in the prompt (see `Agent::acp_run`).
        let explore = crate::explore::acp_server(cwd).filter(|_| !quiet);
        let new = server.new_session_full(cwd, Value::Null, explore.into_iter().collect())?;
        let session = new.get("sessionId").cloned().ok_or("Codex didn't start a session.")?;
        let set = |option: &str, value: &str| server.conn.call("session/set_config_option", json!({"sessionId": session, "configId": option, "value": value}), wait);
        let mut options = new.clone();
        // "auto" is what a session starts on. Another model must not quietly run on that.
        if model != "auto" {
            if !options_of(&new, "model").iter().any(|o| o.0 == model) {
                server.delete_session(&new);
                return Err(format!("Codex isn't offering {model} right now, so nothing was sent. Try again, or pick another model."));
            }
            // Each model has its own efforts, which the answer lists.
            let answer = set("model", model)?;
            if answer.get("configOptions").is_some() {
                options = answer;
            }
        }
        if options_of(&options, "reasoning_effort").iter().any(|o| o.0 == effort) {
            set("reasoning_effort", effort)?;
        }
        // It must not run with more access than Scout's mode gives, so a mode it doesn't offer stops the turn.
        let want = mode_for(mode, quiet);
        if modes_of(&new).iter().any(|m| m == want) {
            server.conn.call("session/set_mode", json!({"sessionId": session, "modeId": want}), wait)?;
        } else if options_of(&new, "mode").iter().any(|o| o.0 == want) {
            set("mode", want)?;
        } else {
            server.delete_session(&new);
            return Err(format!("Codex doesn't offer its {want} mode, so nothing was sent."));
        }
        if (quiet || mode == "plan") && options_of(&new, "collaboration_mode").iter().any(|o| o.0 == "plan") {
            set("collaboration_mode", "plan")?;
        }
        Ok(session)
    })();
    match started {
        Ok(session) => Ok(Run::begin(server, session, tx, rx, prompt, images)),
        Err(e) => {
            server.stop();
            Err(e)
        }
    }
}

/// What Codex's error means for the user.
pub fn explain(message: &str) -> String {
    let m = message.to_lowercase();
    if ["authentication", "not signed in", "not logged in", "login", "unauthorized", "401"].iter().any(|w| m.contains(w)) {
        return "Codex isn't signed in, or its sign-in has expired. Sign in under Settings > Providers.".into();
    }
    if ["usage limit", "rate limit", "quota", "credit", "upgrade", "429"].iter().any(|w| m.contains(w)) {
        return format!("Your ChatGPT plan's Codex limit may have been reached ({message}). Your plan is at {PLAN_URL}. This chat is saved, so you can carry on by picking another provider in the model menu.");
    }
    format!("Codex: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_comes_in_from_outside() {
        assert!(is_outside_setting("CODEX_PATH") && is_outside_setting("codex_home") && is_outside_setting("OPENAI_API_KEY") && is_outside_setting("DEFAULT_AUTH_REQUEST"));
        assert!(!is_outside_setting("PATH"));
        assert!(SETTINGS.contains("check_for_update_on_startup = false"));
    }

    /// Never more access than Scout's mode gives, and only reading for a one-off answer.
    #[test]
    fn modes_ask_what_scouts_would() {
        assert_eq!(mode_for("default", false), "read-only");
        assert_eq!(mode_for("plan", false), "read-only");
        assert_eq!(mode_for("acceptEdits", false), "workspace-write");
        assert_eq!(mode_for("auto", false), "agent");
        assert_eq!(mode_for("yolo", false), "agent-full-access");
        assert_eq!(mode_for("yolo", true), "read-only");
    }

    #[test]
    fn errors_say_what_to_do() {
        assert!(explain("Authentication required").contains("Settings > Providers"));
        assert!(explain("You've hit your usage limit.").contains(PLAN_URL));
        assert_eq!(explain("boom"), "Codex: boom");
    }
}
