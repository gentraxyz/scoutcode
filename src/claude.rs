//! Claude Code, Anthropic's coding agent, on the user's own Claude sign-in. It runs through its Agent Client Protocol
//! adapter (`@agentclientprotocol/claude-agent-acp`), which drives the Claude Code CLI bundled with Anthropic's Agent SDK:
//! see `acp.rs`. Both come from npm, pinned by `src/npm/claude/package-lock.json`, and run on Scout's own Node: see
//! `node.rs`. Scout never sees the sign-in's token: Claude Code keeps it, in a folder only Scout's copy uses, apart from any
//! other Claude Code on this PC.

use crate::acp::{Msg, Run, Server, modes_of, options_of};
use crate::node::Update;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::time::Duration;

pub const PLAN_URL: &str = "https://claude.ai/upgrade";

static AGENT: crate::node::Agent = crate::node::Agent {
    name: "Claude Code",
    folder: "claude",
    // The adapter's version. It only changes with a Scout release: see "Codex and Claude Code" in docs/providers.md.
    version: "0.100.0",
    package_json: include_str!("npm/claude/package.json"),
    lock: include_str!("npm/claude/package-lock.json"),
    entry: "node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js",
};

/// Its sign-in, settings and sessions: Scout's alone.
fn own_home() -> PathBuf {
    crate::config::home().join("runtime").join("claude-home")
}

pub fn installed() -> bool {
    AGENT.installed()
}

pub fn ensure_runtime(progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
    AGENT.ensure(progress)
}

/// Deletes the adapter and Claude Code, with the sign-in.
pub fn remove_runtime() {
    AGENT.remove();
    let _ = std::fs::remove_dir_all(own_home());
}

/// Another copy's settings and keys stay with it. An Anthropic key or another endpoint from outside would bill or send
/// the chat somewhere other than the sign-in.
fn is_outside_setting(name: &str) -> bool {
    let name = name.to_uppercase();
    name.starts_with("CLAUDE_") || name.starts_with("ANTHROPIC_")
}

/// What it runs with, on top of the environment: its own folder, and nothing sent that the chat doesn't need.
fn scout_env(home: &Path) -> Vec<(&'static str, String)> {
    vec![("CLAUDE_CONFIG_DIR", home.to_string_lossy().into_owned()), ("DISABLE_AUTOUPDATER", "1".into()), ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1".into())]
}

fn command() -> Command {
    let home = own_home();
    let _ = std::fs::create_dir_all(&home);
    let mut c = AGENT.command();
    for (k, _) in std::env::vars_os() {
        if is_outside_setting(&k.to_string_lossy()) {
            c.env_remove(k);
        }
    }
    c.envs(scout_env(&home));
    c
}

/// The bundled Claude Code CLI itself, through the adapter.
fn cli(args: &[&str]) -> Command {
    let mut c = command();
    c.arg("--cli").args(args);
    c
}

// ---------- signing in ----------

pub fn signed_in() -> bool {
    installed() && crate::node::quietly(cli(&["auth", "status"]))
}

pub fn sign_out() {
    if installed() {
        crate::node::quietly(cli(&["auth", "logout"]));
    }
}

/// Installs Claude Code, then has it open Claude's sign-in page for a Pro or Max plan, unless it's signed in already.
pub fn sign_in(cancel: Arc<AtomicBool>, send: impl Fn(Update) + Clone + Send + 'static) {
    crate::node::sign_in("Claude Code", ensure_runtime, signed_in, || cli(&["auth", "login", "--claudeai"]), cancel, send);
}

// ---------- the protocol ----------

/// What every session is made with: no settings, hooks, plugins or MCP servers from the project or anywhere else, so a
/// repo can't run its own code or turn off the questions, and nothing kept in Claude Code's history, since the chat is
/// Scout's.
fn session_meta() -> Value {
    json!({"claudeCode": {"options": {"settingSources": [], "strictMcpConfig": true, "persistSession": false}}})
}

fn open(cwd: &Path, tx: Sender<Msg>) -> Result<Server, String> {
    let mut c = command();
    c.current_dir(cwd);
    Server::open(c, "Claude Code", tx)
}

/// The models this account may pick, or why they can't be told.
pub fn list_models() -> Result<Vec<(String, String)>, String> {
    if !installed() {
        return Ok(vec![]);
    }
    let cwd = crate::node::folder("claude-work")?;
    let (tx, _rx) = crate::acp::messages();
    let mut server = open(&cwd, tx)?;
    let session = server.new_session_with(&cwd.to_string_lossy(), session_meta());
    if let Ok(s) = &session {
        server.delete_session(s);
    }
    server.stop();
    let models = options_of(&session?, "model");
    if models.is_empty() {
        return Err("Claude Code didn't list its models.".into());
    }
    Ok(models)
}

/// Claude Code's permission mode for Scout's. Its auto mode only exists on some models and plans, and without it, it asks
/// as it does by default, which never gives more than Scout's mode would.
fn mode_for(mode: &str, quiet: bool, offered: &[String]) -> &'static str {
    let want = match mode {
        _ if quiet => "plan",
        "plan" => "plan",
        "acceptEdits" => "acceptEdits",
        "auto" => "auto",
        "yolo" => "bypassPermissions",
        _ => "default",
    };
    if offered.iter().any(|m| m == want) { want } else { "default" }
}

/// Starts a session and sends `prompt`. `quiet` is a one-off answer, which runs in Claude Code's plan mode.
pub fn start(model: &str, cwd: &str, mode: &str, effort: &str, prompt: String, images: Vec<(String, String)>, quiet: bool) -> Result<Run, String> {
    let (tx, rx) = crate::acp::messages();
    let mut server = open(Path::new(cwd), tx.clone())?;
    let started = (|| -> Result<Value, String> {
        let wait = Duration::from_secs(60);
        let new = server.new_session_with(cwd, session_meta())?;
        let session = new.get("sessionId").cloned().ok_or("Claude Code didn't start a session.")?;
        let set = |option: &str, value: &str| server.conn.call("session/set_config_option", json!({"sessionId": session, "configId": option, "value": value}), wait);
        let mut options = new.clone();
        // "auto" is what a session starts on. Another model must not quietly run on that.
        if model != "auto" {
            if !options_of(&new, "model").iter().any(|o| o.0 == model) {
                server.delete_session(&new);
                return Err(format!("Claude Code isn't offering {model} right now, so nothing was sent. Try again, or pick another model."));
            }
            let answer = set("model", model)?;
            if answer.get("configOptions").is_some() {
                options = answer;
            }
        }
        // Only some models have this, and each its own levels.
        if options_of(&options, "effort").iter().any(|o| o.0 == effort) {
            set("effort", effort)?;
        }
        let want = mode_for(mode, quiet, &modes_of(&new));
        server.conn.call("session/set_mode", json!({"sessionId": session, "modeId": want}), wait)?;
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

/// What Claude Code's error means for the user.
pub fn explain(message: &str) -> String {
    let m = message.to_lowercase();
    if ["authentication", "/login", "not logged in", "login expired", "unauthorized", "401"].iter().any(|w| m.contains(w)) {
        return "Claude Code isn't signed in, or its sign-in has expired. Sign in under Settings > Providers.".into();
    }
    if ["usage limit", "rate limit", "limit reached", "quota", "credit", "upgrade", "429"].iter().any(|w| m.contains(w)) {
        return format!("Your Claude plan's limit may have been reached ({message}). Your plan is at {PLAN_URL}. This chat is saved, so you can carry on by picking another provider in the model menu.");
    }
    format!("Claude Code: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_comes_in_from_outside() {
        assert!(is_outside_setting("ANTHROPIC_API_KEY") && is_outside_setting("anthropic_base_url") && is_outside_setting("CLAUDE_CONFIG_DIR") && is_outside_setting("CLAUDE_CODE_EXECUTABLE"));
        assert!(!is_outside_setting("PATH"));
        let env = scout_env(Path::new("home"));
        assert!(env.contains(&("CLAUDE_CONFIG_DIR", "home".to_string())));
        assert_eq!(session_meta()["claudeCode"]["options"]["settingSources"], json!([]));
    }

    #[test]
    fn modes_never_give_more_than_scouts() {
        let all: Vec<String> = ["default", "acceptEdits", "plan", "auto", "bypassPermissions"].iter().map(|s| s.to_string()).collect();
        assert_eq!(mode_for("default", false, &all), "default");
        assert_eq!(mode_for("plan", false, &all), "plan");
        assert_eq!(mode_for("acceptEdits", false, &all), "acceptEdits");
        assert_eq!(mode_for("auto", false, &all), "auto");
        assert_eq!(mode_for("yolo", false, &all), "bypassPermissions");
        assert_eq!(mode_for("yolo", true, &all), "plan");
        assert_eq!(mode_for("auto", false, &all[..3]), "default", "without auto mode it asks");
    }

    #[test]
    fn errors_say_what_to_do() {
        assert!(explain("Not logged in · Please run /login").contains("Settings > Providers"));
        assert!(explain("Claude usage limit reached").contains(PLAN_URL));
        assert_eq!(explain("boom"), "Claude Code: boom");
    }
}
