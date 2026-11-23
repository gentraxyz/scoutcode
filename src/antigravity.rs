//! Google Antigravity, on the user's own Google account, run the way T3 Code runs it. Google's own ACP server for
//! Antigravity, from the official ACP registry, runs here as an Agent Client Protocol server (see `acp.rs`) and does
//! everything that talks to Google: it does the browser sign-in itself and keeps the login. Scout never reads that login,
//! never calls Antigravity's backend and runs no proxy. Antigravity's terms don't allow using its login through other
//! software, so nothing here may ever take the token out of the CLI or send a request to Google on its own.
//!
//! The server is downloaded at sign-in and checked against the registry's hashes below. It runs its own tools, and like
//! Kiro, Scout shows them and answers its permission requests.

use crate::acp::{Run, Server, browser_sign_in};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

pub use crate::kiro::Update;

/// The server's version, from the ACP registry. It only changes with a Scout release.
const VERSION: &str = "1.5.2";
/// Each platform's package at that version, under Google's download site, and its SHA-256 from the registry.
const PACKAGES: [(&str, &str); 6] = [
    ("windows/agy-acp-server-1.5.2-windows-x86_64.zip", "5cbadd4f4df698572473d8890fc26ec2dc1724501f06e3a963941daec2bd21d6"),
    ("windows/agy-acp-server-1.5.2-windows-arm64.zip", "8a4b2ea8df2ffefae33309348752e9119d21a6d36090c0eb48631c2e8f5781ab"),
    ("macos/agy-acp-server-1.5.2-darwin-arm64.zip", "123a16d5c40a8ac99e20f01a62b68dd860fd96b1c528dfb8eaa37ca8733d4845"),
    ("macos/agy-acp-server-1.5.2-darwin-x86_64.zip", "92addb5580577b5356d35484d72d1a113d9c5312ad766bcb304983aa8e3fbb06"),
    ("linux/agy-acp-server-1.5.2-linux-x86_64.zip", "99dd8612450805011619d7b8447ce374c289e00549d8557f36790e1dd6256852"),
    ("linux/agy-acp-server-1.5.2-linux-arm64.zip", "e729e05ef54ef0763e48900032466b47fa7c9939cde257f93ba49410ab577acb"),
];
pub const PLAN_URL: &str = "https://antigravity.google/";
const NAME: &str = "Antigravity";

fn package() -> Result<(&'static str, &'static str), String> {
    let arm = cfg!(target_arch = "aarch64");
    let name = if cfg!(windows) {
        format!("windows/agy-acp-server-{VERSION}-windows-{}.zip", if arm { "arm64" } else { "x86_64" })
    } else if cfg!(target_os = "macos") {
        format!("macos/agy-acp-server-{VERSION}-darwin-{}.zip", if arm { "arm64" } else { "x86_64" })
    } else {
        format!("linux/agy-acp-server-{VERSION}-linux-{}.zip", if arm { "arm64" } else { "x86_64" })
    };
    PACKAGES.iter().find(|p| p.0 == name).copied().ok_or_else(|| format!("Antigravity isn't available as {name}."))
}

fn root() -> PathBuf {
    crate::config::home().join("runtime").join("antigravity")
}

/// A folder only gets this name once it's complete, so one that exists can be used.
fn install_dir() -> PathBuf {
    root().join(VERSION)
}

fn exe() -> PathBuf {
    install_dir().join(if cfg!(windows) { "agy_acp_server.exe" } else { "agy_acp_server.par" })
}

pub fn installed() -> bool {
    exe().is_file()
}

pub fn remove_runtime() {
    let _ = std::fs::remove_dir_all(root());
}

/// The archives are zips, which don't keep the files' permissions, so on Mac and Linux they're made runnable here.
fn unpack(file: &Path, name: &str, to: &Path) -> std::io::Result<()> {
    crate::devin::unpack(file, name, to)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for e in std::fs::read_dir(to)?.flatten() {
            if e.file_type().is_ok_and(|t| t.is_file()) {
                std::fs::set_permissions(e.path(), std::fs::Permissions::from_mode(0o755))?;
            }
        }
    }
    Ok(())
}

/// Downloads the server unless it's there. The download must match the registry's hash.
pub fn ensure_runtime(progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
    static INSTALLING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one_at_a_time = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    if installed() {
        return Ok(());
    }
    let (name, want) = package()?;
    let url = format!("https://dl.google.com/agy-extensions/releases/{name}");
    crate::acp::install(NAME, &url, want, &root(), VERSION, &|file, to| unpack(file, name, to), progress)
}

/// A folder for sessions that aren't about a project, such as listing models.
fn data_dir() -> Result<PathBuf, String> {
    let d = crate::config::home().join("runtime").join("antigravity-home");
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

fn open(cwd: &Path, tx: Sender<crate::acp::Msg>) -> Result<Server, String> {
    let mut c = Command::new(exe());
    // As the registry runs it on Linux.
    if cfg!(target_os = "linux") {
        c.arg("--uid=");
    }
    c.current_dir(cwd);
    Server::open(c, NAME, tx)
}

/// Whether the server can start a session, which it only does once signed in.
fn can_start(server: &Server, cwd: &Path) -> bool {
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
    let on = can_start(&server, &cwd);
    server.stop();
    on
}

/// Asks the server to forget its Google login, where this version can. Scout turns Antigravity off either way.
pub fn sign_out() {
    if !installed() {
        return;
    }
    let (Ok(cwd), (tx, _rx)) = (data_dir(), crate::acp::messages()) else { return };
    if let Ok(mut server) = open(&cwd, tx) {
        let _ = server.ext("logout", serde_json::json!({}), Duration::from_secs(10));
        server.stop();
    }
}

/// Downloads the server, then has it sign in through the browser with Google, unless it's signed in already.
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
                if can_start(&server, &cwd) {
                    return Ok(());
                }
                let method = browser_sign_in(&server.hello).ok_or("Antigravity didn't offer a sign-in with Google.")?;
                server.authenticate(&method, &cancel)?;
                if can_start(&server, &cwd) { Ok(()) } else { Err("Antigravity didn't accept the sign-in. Try again.".to_string()) }
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

/// Starts a session and sends `prompt`. `quiet` is a one-off answer, which plans only where the server can.
pub fn start(model: &str, cwd: &str, mode: &str, effort: &str, prompt: String, images: Vec<(String, String)>, quiet: bool) -> Result<Run, String> {
    let (tx, rx) = crate::acp::messages();
    let server = open(Path::new(cwd), tx.clone())?;
    crate::acp::start_with(server, tx, rx, NAME, model, cwd, mode, effort, prompt, images, quiet)
}

/// What Antigravity's error means for the user.
pub fn explain(message: &str) -> String {
    let m = message.to_lowercase();
    if m.contains("terms of service") || m.contains("disabled in this account") {
        return format!("Google has turned Antigravity off for this account ({message}). Only Google can turn it back on. This chat is saved, so you can carry on by picking another provider in the model menu.");
    }
    if ["not logged in", "authentication required", "unauthenticated", "sign in", "log in", "login required", "credentials"].iter().any(|w| m.contains(w)) {
        return "Antigravity isn't signed in, or this Google account can't use it. Sign in under Settings > Providers.".into();
    }
    if ["quota", "resource_exhausted", "rate limit", "capacity", "exhausted", "usage limit"].iter().any(|w| m.contains(w)) {
        return format!("Antigravity's limits for this account may have run out ({message}). This chat is saved, so you can carry on by picking another provider in the model menu.");
    }
    format!("Antigravity: {message}")
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
        assert!(explain("Authentication required").contains("Settings > Providers"));
        assert!(explain("RESOURCE_EXHAUSTED: quota").contains("limits"));
        assert!(explain("This service has been disabled in this account for violation of Terms of Service.").contains("Only Google"));
        assert_eq!(explain("boom"), "Antigravity: boom");
    }

    /// Downloads the real server into SCOUT_HOME, and signs in if needed:
    /// `SCOUT_HOME=<temp> cargo test -- --ignored live_antigravity --nocapture`.
    #[test]
    #[ignore]
    fn live_antigravity() {
        assert!(std::env::var("SCOUT_HOME").is_ok(), "set SCOUT_HOME so this doesn't touch the real install");
        ensure_runtime(&mut |_, _| {}).unwrap();
        assert!(installed());
        let cwd = data_dir().unwrap();
        let (tx, _rx) = crate::acp::messages();
        let mut server = open(&cwd, tx).unwrap();
        println!("initialize: {}", server.hello);
        let session = server.new_session(&cwd.to_string_lossy());
        println!("session/new: {session:?}");
        server.stop();
        if signed_in() {
            println!("models: {:?}", list_models());
        }
    }
}
