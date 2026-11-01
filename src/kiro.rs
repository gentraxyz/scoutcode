//! Kiro by AWS, on the user's own account. Its CLI runs here as an Agent Client Protocol server (the "V3" engine): see
//! `acp.rs`. Unlike Devin, it runs its own tools and doesn't offer to send them back. The CLI is downloaded at sign-in and
//! checked against the hashes below. Scout never sees the account's token: the CLI keeps it, in a place shared with any
//! other Kiro CLI on this PC.

use crate::acp::{Msg, Run, Server, options_of};
use crate::tools::{kill_tree, no_window};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The CLI's version. It only changes with a Scout release: see "Kiro" in README.md.
const VERSION: &str = "2.37.0";
/// The SHA-256 of each platform's package at that version, from Kiro's release manifest.
const PACKAGES: [(&str, &str); 6] = [
    ("kiro-cli-x86_64-pc-windows-msvc.msi", "17dc0d87420c5e5d2c3ecce54a1931354f92c7490389c3919ca30180b9ee0156"),
    ("kirocli-universal-darwin.tar.gz", "c6e3f5cd4131145a097b1bd55f5247e88f9096cc2fe176c200d52ef37511859c"),
    ("kirocli-x86_64-linux.tar.gz", "02c49873ba64285f34048aac377ccf0b546b5627f39dd775bae9fa306112ff19"),
    ("kirocli-aarch64-linux.tar.gz", "3588100df2addfbb2c6953698ea2d6e7d2b88c5ef664ec3235e08fa454219f68"),
    ("kirocli-x86_64-linux-musl.tar.gz", "1bd7d51f940c7ffc0d682016e3c1d64bc0a81d851ba0df1f55619fca7a9f50ab"),
    ("kirocli-aarch64-linux-musl.tar.gz", "945b334790d5a12c9953f1ba01846fa1274c2d28e90a66ee023a9972a063e574"),
];
pub const PLAN_URL: &str = "https://kiro.dev/pricing/";

/// How signing in is going, for the window.
pub enum Update {
    Progress { done: u64, total: u64 },
    /// The CLI is there, and the browser should be open.
    Waiting,
    Done,
    Failed(String),
}

/// The package for this platform. Windows has one, which also runs on Arm through emulation.
fn package() -> Result<(&'static str, &'static str), String> {
    let arch = if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" };
    let name = if cfg!(windows) {
        "kiro-cli-x86_64-pc-windows-msvc.msi".to_string()
    } else if cfg!(target_os = "macos") {
        "kirocli-universal-darwin.tar.gz".to_string()
    } else if cfg!(target_env = "musl") {
        format!("kirocli-{arch}-linux-musl.tar.gz")
    } else {
        format!("kirocli-{arch}-linux.tar.gz")
    };
    PACKAGES.iter().find(|p| p.0 == name).copied().ok_or_else(|| format!("Kiro isn't available as {name}."))
}

fn root() -> PathBuf {
    crate::config::home().join("runtime").join("kiro")
}

/// Where the CLI lives. A folder only gets this name once it's complete, so one that exists can be used.
fn install_dir() -> PathBuf {
    root().join(VERSION)
}

fn exe() -> PathBuf {
    install_dir().join("kirocli").join("bin").join(if cfg!(windows) { "kiro-cli.exe" } else { "kiro-cli" })
}

pub fn installed() -> bool {
    exe().is_file()
}

/// Deletes the CLI.
pub fn remove_runtime() {
    let _ = std::fs::remove_dir_all(root());
}

/// Unpacks the Windows package without installing it: an administrative install only copies its files, and needs no
/// administrator. The result is laid out like the other platforms' archives.
#[cfg(windows)]
fn unpack(file: &Path, _name: &str, to: &Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    std::fs::create_dir_all(to)?;
    let mut c = Command::new("msiexec");
    // Raw, since msiexec wants the quotes around the value only. It can't open a path with forward slashes either.
    let windows_path = |p: &Path| p.to_string_lossy().replace('/', "\\");
    c.raw_arg(format!("/a \"{}\" /qn TARGETDIR=\"{}\"", windows_path(file), windows_path(to)));
    no_window(&mut c);
    let status = c.status()?;
    if !status.success() {
        return Err(std::io::Error::other(format!("msiexec failed with {status}")));
    }
    let bin = to.join("kirocli").join("bin");
    std::fs::create_dir_all(&bin)?;
    std::fs::rename(to.join("LocalApp").join("Kiro-Cli").join("kiro-cli.exe"), bin.join("kiro-cli.exe"))?;
    let _ = std::fs::remove_dir_all(to.join("LocalApp"));
    // It leaves a small copy of the package behind too.
    if let Some(name) = file.file_name() {
        let _ = std::fs::remove_file(to.join(name));
    }
    Ok(())
}

#[cfg(not(windows))]
fn unpack(file: &Path, name: &str, to: &Path) -> std::io::Result<()> {
    crate::devin::unpack(file, name, to)
}

/// Downloads the CLI unless it's there. The download must match the built-in hash, and the folder only appears once
/// it's fully unpacked. Older versions are deleted afterwards.
pub fn ensure_runtime(progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
    static INSTALLING: Mutex<()> = Mutex::new(());
    let _one_at_a_time = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    if installed() {
        return Ok(());
    }
    let (name, want) = package()?;
    let url = format!("https://prod.download.cli.kiro.dev/stable/{VERSION}/{name}");
    crate::acp::install("Kiro", &url, want, &root(), VERSION, &|file, to| unpack(file, name, to), progress)
}

// ---------- signing in ----------

fn cli(args: &[&str]) -> Command {
    let mut c = Command::new(exe());
    // Nothing reads its output: the login draws a spinner, which would fill a pipe and stop it.
    c.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    no_window(&mut c);
    c
}

pub fn signed_in() -> bool {
    cli(&["whoami"]).status().is_ok_and(|s| s.success())
}

/// Signs the CLI out, here and for any other copy on this PC: they share the sign-in.
pub fn sign_out() {
    if installed() {
        let _ = cli(&["logout"]).status();
    }
}

/// Downloads the CLI, then has it open Kiro's sign-in page, unless it's signed in already.
pub fn sign_in(cancel: Arc<AtomicBool>, send: impl Fn(Update) + Send + 'static) {
    std::thread::spawn(move || {
        let run = || -> Result<(), String> {
            ensure_runtime(&mut |done, total| send(Update::Progress { done, total }))?;
            if cancel.load(Ordering::Relaxed) {
                return Err(String::new());
            }
            send(Update::Waiting);
            if signed_in() {
                return Ok(());
            }
            let mut login = cli(&["login"]).spawn().map_err(|e| format!("Couldn't start Kiro: {e}"))?;
            let deadline = Instant::now() + Duration::from_secs(600);
            let result = loop {
                if cancel.load(Ordering::Relaxed) {
                    break Err(String::new());
                }
                if Instant::now() > deadline {
                    break Err("The sign-in timed out. Try again.".to_string());
                }
                match login.try_wait() {
                    Ok(Some(_)) => break Ok(()),
                    Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                    Err(e) => break Err(e.to_string()),
                }
            };
            kill_tree(&mut login);
            let _ = login.wait();
            result?;
            if signed_in() { Ok(()) } else { Err("Kiro didn't accept the sign-in. Try again.".into()) }
        };
        match run() {
            Ok(()) => send(Update::Done),
            Err(e) => send(Update::Failed(e)),
        }
    });
}

/// A folder for sessions that aren't about a project, such as listing models.
fn data_dir() -> Result<PathBuf, String> {
    let d = crate::config::home().join("runtime").join("kiro-home");
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

// ---------- the protocol ----------

fn open(cwd: &Path, tx: Sender<Msg>) -> Result<Server, String> {
    let mut c = Command::new(exe());
    // Kiro keeps two engines that aren't compatible, and the older is still the default.
    c.args(["acp", "--agent-engine=v3"]).current_dir(cwd);
    Server::open(c, "Kiro", tx)
}

/// A new session that has its models, however long they took: see `session_with_models`.
fn session_on(server: &Server, cwd: &str) -> Result<Value, String> {
    let latest = |id: &str| server.latest.lock().unwrap().as_ref().filter(|l| l.0 == id).map(|l| l.1.clone());
    session_with_models(|| server.new_session(cwd), |s| server.delete_session(s), latest, Duration::from_secs(15))
}

/// A new session with its models listed in it. Kiro fetches them while it answers session/new, and whichever finishes
/// first decides if the answer has them, so a session without them waits for its settings to be sent again. If those
/// don't come, the next new session finds the fetch done. After `patience`, it's whatever there is.
fn session_with_models(mut new_session: impl FnMut() -> Result<Value, String>, mut delete_session: impl FnMut(&Value), latest: impl Fn(&str) -> Option<Value>, patience: Duration) -> Result<Value, String> {
    let give_up = Instant::now() + patience;
    loop {
        let mut session = new_session()?;
        let id = session.get("sessionId").and_then(Value::as_str).ok_or("Kiro didn't start a session.")?.to_string();
        let try_again = Instant::now() + patience / 6;
        loop {
            if let Some(options) = latest(&id) {
                session["configOptions"] = options;
            }
            if !options_of(&session, "model").is_empty() || Instant::now() >= give_up {
                return Ok(session);
            }
            if Instant::now() >= try_again {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        delete_session(&session);
    }
}

/// The models this account may pick, from a new session, or why they can't be told.
pub fn list_models() -> Result<Vec<(String, String)>, String> {
    if !installed() {
        return Ok(vec![]);
    }
    let cwd = data_dir()?;
    let (tx, _rx) = crate::acp::messages();
    let mut server = open(&cwd, tx)?;
    let session = session_on(&server, &cwd.to_string_lossy());
    if let Ok(s) = &session {
        server.delete_session(s);
    }
    server.stop();
    let models = options_of(&session?, "model");
    if models.is_empty() {
        return Err("Kiro didn't list its models in time.".into());
    }
    Ok(models)
}

/// The settings a session starts with, as (option, value): only the ones it offers get set. Autopilot is off so that
/// Kiro asks first, as Scout's permissions need, unless Yolo says never to ask. Content collection is off because a
/// chat here shouldn't go to improving Kiro's service unless the user chooses that in Kiro.
fn settings_for(model: &str, mode: &str, effort: &str) -> Vec<(&'static str, String)> {
    let mut wanted = vec![("autopilot", if mode == "yolo" { "on" } else { "off" }.to_string()), ("contentCollection", "disabled".to_string()), ("model", model.to_string())];
    if effort != "none" {
        wanted.push(("effortLevel", effort.to_string()));
    }
    wanted
}

/// Starts a session and sends `prompt`. `quiet` is a one-off answer, which runs in Kiro's plan mode.
pub fn start(model: &str, cwd: &str, mode: &str, effort: &str, prompt: String, images: Vec<(String, String)>, quiet: bool) -> Result<Run, String> {
    let (tx, rx) = crate::acp::messages();
    let mut server = open(Path::new(cwd), tx.clone())?;
    let started = (|| -> Result<Value, String> {
        let wait = Duration::from_secs(60);
        // Auto is what a session starts on, so only another model has to wait for the list. It must not quietly run on Auto.
        let new = if model == "auto" { server.new_session(cwd)? } else { session_on(&server, cwd)? };
        let session = new.get("sessionId").cloned().ok_or("Kiro didn't start a session.")?;
        if model != "auto" && !options_of(&new, "model").iter().any(|o| o.0 == model) {
            server.delete_session(&new);
            return Err(format!("Kiro isn't offering {model} right now, so nothing was sent. Try again, or pick another model."));
        }
        for (option, value) in settings_for(model, mode, effort) {
            if options_of(&new, option).iter().any(|o| o.0 == value) {
                server.conn.call("session/set_config_option", json!({"sessionId": session, "configId": option, "value": value}), wait)?;
            }
        }
        if quiet || mode == "plan" {
            server.conn.call("session/set_mode", json!({"sessionId": session, "modeId": "plan"}), wait)?;
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

/// What Kiro's error means for the user.
pub fn explain(message: &str) -> String {
    let m = message.to_lowercase();
    if m.contains("not logged in") || m.contains("authentication required") || m.contains("does not have access") {
        return "Kiro isn't signed in, or this account can't use it. Sign in under Settings > Account.".into();
    }
    if ["quota", "credit", "usage limit", "rate limit", "throttl", "upgrade"].iter().any(|w| m.contains(w)) {
        return format!("Your Kiro credits may have run out ({message}). Plans are at {PLAN_URL}. This chat is saved, so you can carry on by picking another provider in the model menu.");
    }
    format!("Kiro: {message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_platform_has_a_full_sha256() {
        for (_, hash) in PACKAGES {
            assert_eq!(hash.len(), 64);
            assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        }
        assert!(package().is_ok());
    }

    /// Kiro's real race: its answer to session/new can beat its fetch of the models by a few milliseconds.
    #[test]
    fn a_session_waits_for_models_that_arrive_late() {
        use std::cell::Cell;
        let model = json!([{"id": "model", "options": [{"value": "claude-sonnet-4.5", "name": "Sonnet 4.5"}]}]);
        let bare = |id: &str| json!({"sessionId": id, "configOptions": [{"id": "mode", "options": []}]});
        let patience = Duration::from_millis(600);

        // Sent again as settings a moment later.
        let start = Instant::now();
        let late = |id: &str| (id == "s1" && start.elapsed() > Duration::from_millis(30)).then(|| model.clone());
        let s = session_with_models(|| Ok(bare("s1")), |_| panic!("no need for another session"), late, patience).unwrap();
        assert_eq!(options_of(&s, "model")[0].1, "Sonnet 4.5");

        // Never sent, but the next session has them. The first one doesn't stay behind.
        let (made, deleted) = (Cell::new(0), Cell::new(0));
        let second = || {
            made.set(made.get() + 1);
            Ok(if made.get() == 1 { bare("s1") } else { json!({"sessionId": "s2", "configOptions": model}) })
        };
        let s = session_with_models(second, |_| deleted.set(deleted.get() + 1), |_| None, patience).unwrap();
        assert_eq!((s["sessionId"].as_str(), made.get(), deleted.get()), (Some("s2"), 2, 1));

        // Never there at all: it gives up rather than waiting forever.
        let s = session_with_models(|| Ok(bare("s")), |_| {}, |_| None, patience).unwrap();
        assert!(options_of(&s, "model").is_empty());
        assert!(session_with_models(|| Err("Authentication required".into()), |_| {}, |_| None, patience).is_err());
    }

    #[test]
    fn kiro_asks_first_unless_yolo() {
        let autopilot = |mode| settings_for("auto", mode, "low").into_iter().find(|s| s.0 == "autopilot").unwrap().1;
        assert_eq!(autopilot("yolo"), "on");
        for mode in ["default", "acceptEdits", "auto", "plan"] {
            assert_eq!(autopilot(mode), "off");
        }
        assert!(settings_for("auto", "default", "none").iter().all(|s| s.0 != "effortLevel"));
        assert!(settings_for("auto", "default", "low").contains(&("contentCollection", "disabled".to_string())));
    }

    #[test]
    fn errors_say_what_to_do() {
        assert!(explain("Authentication required. Not logged in. Set the KIRO_API_KEY environment variable").contains("Settings > Account"));
        assert!(explain("Kiro could not load the available models because this account does not have access to them.").contains("Settings > Account"));
        assert!(explain("monthly credits used").contains(PLAN_URL));
        assert_eq!(explain("boom"), "Kiro: boom");
    }

    /// Downloads and unpacks the real CLI into SCOUT_HOME: `SCOUT_HOME=<temp> cargo test -- --ignored live_kiro_install --nocapture`.
    #[test]
    #[ignore]
    fn live_kiro_install() {
        assert!(std::env::var("SCOUT_HOME").is_ok(), "set SCOUT_HOME so this doesn't touch the real install");
        let mut last = 0;
        ensure_runtime(&mut |done, total| last = total.max(done)).unwrap();
        println!("downloaded {last} bytes to {}", install_dir().display());
        assert!(installed());
        ensure_runtime(&mut |_, _| panic!("downloaded twice")).unwrap();
        let out = Command::new(exe()).arg("--version").output().unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains(VERSION));
        if signed_in() {
            return;
        }
        // Signed out, a session can't start. A made-up key gets one, with the real settings, until the prompt is refused.
        let cwd = data_dir().unwrap();
        let cwd = cwd.to_string_lossy();
        let refused = start("auto", &cwd, "default", "low", "hi".into(), vec![], false).err().unwrap();
        println!("signed out: {refused}");
        assert!(explain(&refused).contains("Settings > Account"));
        unsafe { std::env::set_var("KIRO_API_KEY", "ksk_notarealkey") };
        let run = start("auto", &cwd, "plan", "low", "hi".into(), vec![], false).unwrap();
        let failed = loop {
            match run.next(Duration::from_secs(60)).expect("no answer from Kiro") {
                Msg::Failed(e) => break e,
                Msg::Idle(stop) => panic!("a made-up key got an answer: {stop}"),
                _ => {}
            }
        };
        println!("made-up key: {failed}");
        assert!(explain(&failed).contains("Settings > Account"));
        run.finish();
    }
}
