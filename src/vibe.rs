//! Mistral Vibe, on the user's own Mistral account. Mistral's `vibe-acp`, from the official ACP registry, runs here as an
//! Agent Client Protocol server (see `acp.rs`). It signs in through Mistral AI Studio in the browser by itself and keeps
//! the key it gets in the system keyring (or ~/.vibe), shared with any other Vibe on this PC; Scout never sees it. A
//! MISTRAL_API_KEY in the environment works too. Its settings, agents and skills in ~/.vibe apply here as in the CLI.
//!
//! The server is downloaded at sign-in and checked against the registry's hashes below. It runs its own tools: Scout
//! shows them and answers its permission requests.

use crate::acp::{Run, Server, browser_sign_in};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

pub use crate::kiro::Update;

/// The server's version, from the ACP registry. It only changes with a Scout release.
const VERSION: &str = "3.1.4";
/// Each platform's package at that version on Mistral's GitHub releases, and its SHA-256 from the registry. There's no
/// Windows on Arm build, so the x64 one runs there through emulation.
const PACKAGES: [(&str, &str); 5] = [
    ("vibe-acp-windows-x86_64-3.1.4.zip", "4f8681d6e0496a258ba62acf57188961ebdfdc5799f26fe218adb43c637c7b79"),
    ("vibe-acp-darwin-aarch64-3.1.4.tar.gz", "4492735d66b615cb321e19e762199df957744ff9f8e7cccaf39fbebd55cad8a0"),
    ("vibe-acp-darwin-x86_64-3.1.4.tar.gz", "a5e2fac0ec4ea09c7c8fd218ae2acbb9b830ecd071668fe7f4293e793a104940"),
    ("vibe-acp-linux-aarch64-3.1.4.tar.gz", "3db34e9f64920ddc819687e848ac3ebb730112538a56b0b64633bce6beccaf74"),
    ("vibe-acp-linux-x86_64-3.1.4.tar.gz", "934a79356755821a31e6ce1aee7f7f1eacb6b1e6c78901acf1eb7fc4650aece7"),
];
pub const PLAN_URL: &str = "https://console.mistral.ai/";
const NAME: &str = "Mistral Vibe";

fn package() -> Result<(&'static str, &'static str), String> {
    let arch = if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" };
    let name = if cfg!(windows) {
        format!("vibe-acp-windows-x86_64-{VERSION}.zip")
    } else if cfg!(target_os = "macos") {
        format!("vibe-acp-darwin-{arch}-{VERSION}.tar.gz")
    } else {
        format!("vibe-acp-linux-{arch}-{VERSION}.tar.gz")
    };
    PACKAGES.iter().find(|p| p.0 == name).copied().ok_or_else(|| format!("Mistral Vibe isn't available as {name}."))
}

fn root() -> PathBuf {
    crate::config::home().join("runtime").join("vibe")
}

/// A folder only gets this name once it's complete, so one that exists can be used.
fn install_dir() -> PathBuf {
    root().join(VERSION)
}

fn exe() -> PathBuf {
    install_dir().join(if cfg!(windows) { "vibe-acp.exe" } else { "vibe-acp" })
}

pub fn installed() -> bool {
    exe().is_file()
}

pub fn remove_runtime() {
    let _ = std::fs::remove_dir_all(root());
}

/// Downloads the server unless it's there. The download must match the registry's hash.
pub fn ensure_runtime(progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
    static INSTALLING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one_at_a_time = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    if installed() {
        return Ok(());
    }
    let (name, want) = package()?;
    let url = format!("https://github.com/mistralai/mistral-vibe/releases/download/v{VERSION}/{name}");
    crate::acp::install(NAME, &url, want, &root(), VERSION, &|file, to| crate::devin::unpack(file, name, to), progress)
}

/// A folder for sessions that aren't about a project, such as listing models.
fn data_dir() -> Result<PathBuf, String> {
    let d = crate::config::home().join("runtime").join("vibe-home");
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

fn open(cwd: &Path, tx: Sender<crate::acp::Msg>) -> Result<Server, String> {
    let mut c = Command::new(exe());
    c.current_dir(cwd);
    Server::open(c, NAME, tx)
}

/// What Vibe's `auth/status` says, or failing that, whether it can start a session.
fn authenticated(server: &Server, cwd: &Path) -> bool {
    if let Ok(status) = server.ext("auth/status", json!({}), Duration::from_secs(15)) {
        if let Some(on) = status.get("authenticated").and_then(Value::as_bool) {
            return on;
        }
    }
    match server.new_session(&cwd.to_string_lossy()) {
        Ok(s) => {
            server.delete_session(&s);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
fn signed_in() -> bool {
    let Ok(cwd) = data_dir() else { return false };
    let (tx, _rx) = crate::acp::messages();
    let Ok(mut server) = open(&cwd, tx) else { return false };
    let on = authenticated(&server, &cwd);
    server.stop();
    on
}

/// Removes the key Vibe saved, here and for any other Vibe on this PC: they share it.
pub fn sign_out() {
    if !installed() {
        return;
    }
    let (Ok(cwd), (tx, _rx)) = (data_dir(), crate::acp::messages()) else { return };
    if let Ok(mut server) = open(&cwd, tx) {
        let _ = server.ext("auth/signOut", json!({}), Duration::from_secs(15));
        server.stop();
    }
}

/// Downloads the server, then has it sign in through Mistral AI Studio in the browser, unless it's signed in already.
pub fn sign_in(cancel: Arc<AtomicBool>, send: impl Fn(Update) + Send + 'static) {
    std::thread::spawn(move || {
        let run = || -> Result<(), String> {
            ensure_runtime(&mut |done, total| send(Update::Progress { done, total }))?;
            if cancel.load(Ordering::Relaxed) {
                return Err(String::new());
            }
            send(Update::Waiting);
            let cwd = data_dir()?;
            let (tx, _rx) = crate::acp::messages();
            let mut server = open(&cwd, tx)?;
            let result = (|| {
                if authenticated(&server, &cwd) {
                    return Ok(());
                }
                let method = browser_sign_in(&server.hello).ok_or("Mistral Vibe didn't offer a sign-in through the browser.")?;
                server.authenticate(&method, &cancel)?;
                if authenticated(&server, &cwd) { Ok(()) } else { Err("Mistral Vibe didn't accept the sign-in. Try again.".to_string()) }
            })();
            server.stop();
            result
        };
        match run() {
            Ok(()) => send(Update::Done),
            Err(e) => send(Update::Failed(e)),
        }
    });
}

/// The models this account may pick, or why they can't be told.
pub fn list_models() -> Result<Vec<(String, String)>, String> {
    if !installed() {
        return Ok(vec![]);
    }
    let cwd = data_dir()?;
    let (tx, _rx) = crate::acp::messages();
    crate::acp::list_models_on(open(&cwd, tx)?, NAME, &cwd.to_string_lossy())
}

/// Starts a session and sends `prompt`. `quiet` is a one-off answer, which runs as Vibe's plan agent.
pub fn start(model: &str, cwd: &str, mode: &str, effort: &str, prompt: String, images: Vec<(String, String)>, quiet: bool) -> Result<Run, String> {
    let (tx, rx) = crate::acp::messages();
    let server = open(Path::new(cwd), tx.clone())?;
    crate::acp::start_with(server, tx, rx, NAME, model, cwd, mode, effort, prompt, images, quiet)
}

/// What Vibe's error means for the user.
pub fn explain(message: &str) -> String {
    let m = message.to_lowercase();
    if m.contains("missing api key") || m.contains("invalid api key") || m.contains("unauthorized") || m.contains("authentication required") {
        return "Mistral Vibe isn't signed in, or its key no longer works. Sign in under Settings > Providers.".into();
    }
    if ["rate limit", "quota", "429", "payment", "billing", "credit", "usage limit"].iter().any(|w| m.contains(w)) {
        return format!("Your Mistral limits may have run out ({message}). Plans are at {PLAN_URL}. This chat is saved, so you can carry on by picking another provider in the model menu.");
    }
    format!("Mistral Vibe: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_platform_has_a_full_sha256() {
        for (name, hash) in PACKAGES {
            assert!(name.contains(VERSION));
            assert_eq!(hash.len(), 64);
            assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        }
        assert!(package().is_ok());
    }

    #[test]
    fn errors_say_what_to_do() {
        assert!(explain("Missing API key for mistral provider.").contains("Settings > Providers"));
        assert!(explain("429 rate limit exceeded").contains(PLAN_URL));
        assert_eq!(explain("boom"), "Mistral Vibe: boom");
    }

    /// Downloads the real server into SCOUT_HOME and shows what it offers:
    /// `SCOUT_HOME=<temp> cargo test -- --ignored live_vibe --nocapture`.
    #[test]
    #[ignore]
    fn live_vibe() {
        assert!(std::env::var("SCOUT_HOME").is_ok(), "set SCOUT_HOME so this doesn't touch the real install");
        ensure_runtime(&mut |_, _| {}).unwrap();
        assert!(installed());
        let cwd = data_dir().unwrap();
        let (tx, _rx) = crate::acp::messages();
        let mut server = open(&cwd, tx).unwrap();
        println!("initialize: {}", server.hello);
        assert_eq!(browser_sign_in(&server.hello).as_deref(), Some("browser-auth"));
        server.stop();
        if signed_in() {
            println!("models: {:?}", list_models());
        }
    }
}
