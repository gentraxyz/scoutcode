//! OpenCode, run through its own CLI as an Agent Client Protocol server: see `acp.rs`. Its free models need no account, and
//! signing in to an OpenCode account adds the rest. The CLI is downloaded when it's first wanted and checked against the
//! hashes below. Scout never sees the account's token: the CLI keeps it, in a folder only Scout's copy uses, apart from any
//! other OpenCode on this PC.

use crate::acp::{Msg, Run, Server, options_of};
use crate::tools::{kill_tree, no_window};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The CLI's version. It only changes with a Scout release: see "OpenCode" in README.md.
const VERSION: &str = "1.19.5";
/// The SHA-256 of each platform's package at that version, from the digests on OpenCode's GitHub release. The baseline
/// builds run on any x64 CPU, and the others need AVX2.
const PACKAGES: [(&str, &str); 8] = [
    ("opencode-windows-x64-baseline.zip", "e4dac5cd38a182a24e7b3e18853f01d08cdb137e50b3bd1397c1a89e54d28c63"),
    ("opencode-windows-arm64.zip", "5d49139bd917511ff35a3c06065c9f47918346668f05ea020e778286f302fbb7"),
    ("opencode-darwin-arm64.zip", "c1c9e62521192192ef4c3ed1355ada2144055bddb61682217e997b03c55203c1"),
    ("opencode-darwin-x64-baseline.zip", "e1235374cb7e712cdc15e21ba915a029a5d22312b460606d0783754739706775"),
    ("opencode-linux-x64-baseline.tar.gz", "17c200fd4b2289954cc01552559f9b6d03ec157b2ddfb5eb54144f491656333d"),
    ("opencode-linux-arm64.tar.gz", "fae5be262d693b7c5e562856cc473e59e741772936db0c0600ac0d5ed5bd088b"),
    ("opencode-linux-x64-baseline-musl.tar.gz", "e796ccff81070b527b2aea144741928e56bdbc6650cd90fb5564d5c114f0e08b"),
    ("opencode-linux-arm64-musl.tar.gz", "3522eccafd30d84a2d770b72a56d4877c576b9072b36a5a9ea0fa30ed82b896e"),
];
pub const PLAN_URL: &str = "https://opencode.ai/console";

/// What OpenCode asks Scout about. Scout's modes decide from there, so everything that isn't only looking gets asked. A
/// `.env` file is looked at only with permission, as OpenCode does by default.
const PERMISSION: &str = r#"{"*":"ask","read":{"*":"allow","*.env":"ask","*.env.*":"ask"},"glob":"allow","grep":"allow","list":"allow","todowrite":"allow"}"#;
/// Its settings: only its own models, even with another provider's key in the environment, and nothing shared or updated
/// behind Scout's back.
const SETTINGS: &str = r#"{"enabled_providers":["opencode","opencode-go"],"share":"disabled","autoupdate":false}"#;

/// How setting it up or signing in is going, for the window.
pub enum Update {
    Progress { done: u64, total: u64 },
    /// The code to enter on OpenCode's page, which should be open by now.
    Code { code: String, url: String },
    /// The CLI is there, for the free models.
    Ready,
    /// Signed in to an account too.
    SignedIn,
    Failed(String),
}

/// The package for this platform.
fn package() -> Result<(&'static str, &'static str), String> {
    let arm = cfg!(target_arch = "aarch64");
    let name = if cfg!(windows) {
        if arm { "opencode-windows-arm64.zip" } else { "opencode-windows-x64-baseline.zip" }
    } else if cfg!(target_os = "macos") {
        if arm { "opencode-darwin-arm64.zip" } else { "opencode-darwin-x64-baseline.zip" }
    } else if cfg!(target_env = "musl") {
        if arm { "opencode-linux-arm64-musl.tar.gz" } else { "opencode-linux-x64-baseline-musl.tar.gz" }
    } else if arm {
        "opencode-linux-arm64.tar.gz"
    } else {
        "opencode-linux-x64-baseline.tar.gz"
    };
    PACKAGES.iter().find(|p| p.0 == name).copied().ok_or_else(|| format!("OpenCode isn't available as {name}."))
}

fn root() -> PathBuf {
    crate::config::home().join("runtime").join("opencode")
}

/// Where the CLI lives. A folder only gets this name once it's complete, so one that exists can be used.
fn install_dir() -> PathBuf {
    root().join(VERSION)
}

fn exe() -> PathBuf {
    install_dir().join(if cfg!(windows) { "opencode.exe" } else { "opencode" })
}

/// Its sign-in, settings and history: Scout's alone.
fn own_home() -> PathBuf {
    crate::config::home().join("runtime").join("opencode-home")
}

pub fn installed() -> bool {
    exe().is_file()
}

/// Deletes the CLI, with its sign-in and history.
pub fn remove_runtime() {
    let _ = std::fs::remove_dir_all(root());
    let _ = std::fs::remove_dir_all(own_home());
}

fn unpack(file: &Path, name: &str, to: &Path) -> std::io::Result<()> {
    crate::devin::unpack(file, name, to)?;
    // A zip doesn't keep it runnable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(to.join("opencode"), std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Downloads the CLI unless it's there.
pub fn ensure_runtime(progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
    static INSTALLING: Mutex<()> = Mutex::new(());
    let _one_at_a_time = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    if installed() {
        return Ok(());
    }
    let (name, want) = package()?;
    let url = format!("https://github.com/anomalyco/opencode/releases/download/v{VERSION}/{name}");
    crate::acp::install("OpenCode", &url, want, &root(), VERSION, &|file, to| unpack(file, name, to), progress)
}

/// What the CLI runs with, on top of the environment: its folders are Scout's alone, and a project can't change what's
/// asked. A repo's `opencode.json` would otherwise win over `OPENCODE_PERMISSION`, and could turn off every question.
fn scout_env(home: &Path) -> Vec<(&'static str, String)> {
    let dir = |d: &str| home.join(d).to_string_lossy().into_owned();
    let on = || "1".to_string();
    vec![
        ("XDG_DATA_HOME", dir("data")),
        ("XDG_CONFIG_HOME", dir("config")),
        ("XDG_CACHE_HOME", dir("cache")),
        ("XDG_STATE_HOME", dir("state")),
        ("OPENCODE_PERMISSION", PERMISSION.to_string()),
        ("OPENCODE_CONFIG_CONTENT", SETTINGS.to_string()),
        ("OPENCODE_DISABLE_PROJECT_CONFIG", on()),
        ("OPENCODE_PURE", on()),
        ("OPENCODE_DISABLE_AUTOUPDATE", on()),
        ("OPENCODE_DISABLE_CLAUDE_CODE", on()),
        ("OPENCODE_DISABLE_EXTERNAL_SKILLS", on()),
    ]
}

fn command() -> Command {
    let mut c = Command::new(exe());
    // Another copy's settings and keys stay with it.
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().to_uppercase().starts_with("OPENCODE_") {
            c.env_remove(k);
        }
    }
    c.envs(scout_env(&own_home()));
    c
}

// ---------- signing in ----------

fn cli(args: &[&str]) -> Command {
    let mut c = command();
    c.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    no_window(&mut c);
    c
}

/// Whether the account list has anyone on it. It says so with success either way, and both streams get a colour reset
/// even when there's nothing else, so that doesn't count.
fn lists_an_account(output: &str) -> bool {
    let clean = plain(output);
    !clean.trim().is_empty() && !clean.contains("No accounts found")
}

pub fn signed_in() -> bool {
    let mut c = cli(&["console", "orgs"]);
    // The list goes to stderr, not stdout.
    c.stdout(Stdio::piped()).stderr(Stdio::piped());
    c.output().is_ok_and(|o| {
        let both = [o.stdout, o.stderr].concat();
        o.status.success() && lists_an_account(&String::from_utf8_lossy(&both))
    })
}

pub fn sign_out() {
    if installed() {
        let _ = cli(&["console", "logout"]).status();
    }
}

/// Downloads the CLI, for its free models.
pub fn set_up(send: impl Fn(Update) + Send + 'static) {
    std::thread::spawn(move || match ensure_runtime(&mut |done, total| send(Update::Progress { done, total })) {
        Ok(()) => send(Update::Ready),
        Err(e) => send(Update::Failed(e)),
    });
}

/// The CLI's output without its colours.
fn plain(text: &str) -> String {
    let mut clean = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // An escape sequence ends with its first letter.
            for e in chars.by_ref() {
                if e.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            clean.push(c);
        }
    }
    clean
}

/// What a line of the login says, without its colours and box drawing: where to go, or the code to enter there.
fn login_line(line: &str) -> (Option<String>, Option<String>) {
    let clean = plain(line);
    let after = |label: &str| clean.split_once(label).map(|(_, v)| v.trim().to_string()).filter(|v| !v.is_empty());
    (after("Go to:"), after("Enter code:"))
}

/// Downloads the CLI, then has it sign in with OpenCode's device flow, unless it's signed in already.
pub fn sign_in(cancel: Arc<AtomicBool>, send: impl Fn(Update) + Clone + Send + 'static) {
    std::thread::spawn(move || {
        let run = || -> Result<(), String> {
            ensure_runtime(&mut |done, total| send(Update::Progress { done, total }))?;
            if cancel.load(Ordering::Relaxed) {
                return Err(String::new());
            }
            if signed_in() {
                return Ok(());
            }
            let mut c = cli(&["console", "login"]);
            c.stdout(Stdio::piped());
            let mut login = c.spawn().map_err(|e| format!("Couldn't start OpenCode: {e}"))?;
            let stdout = login.stdout.take().ok_or("OpenCode has no output.")?;
            // It draws a spinner as well, so this reads to the end: a full pipe would stop it.
            let told = send.clone();
            std::thread::spawn(move || {
                let (mut url, mut reader, mut line) = (None, BufReader::new(stdout), Vec::new());
                loop {
                    line.clear();
                    match reader.read_until(b'\n', &mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    match login_line(&String::from_utf8_lossy(&line)) {
                        (Some(u), _) => url = Some(u),
                        (_, Some(code)) => {
                            if let Some(url) = url.take() {
                                told(Update::Code { code, url: url.clone() });
                                crate::oauth::open_browser(&url);
                            }
                        }
                        _ => {}
                    }
                }
            });
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
            if signed_in() { Ok(()) } else { Err("OpenCode didn't accept the sign-in. Try again.".into()) }
        };
        match run() {
            Ok(()) => send(Update::SignedIn),
            Err(e) => send(Update::Failed(e)),
        }
    });
}

// ---------- the protocol ----------

fn open(cwd: &Path, tx: Sender<Msg>) -> Result<Server, String> {
    let mut c = command();
    c.args(["acp", "--pure"]).current_dir(cwd);
    Server::open(c, "OpenCode", tx)
}

/// A folder for sessions that aren't about a project, such as listing models.
fn data_dir() -> Result<PathBuf, String> {
    let d = own_home().join("work");
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

/// Whether a model of OpenCode's costs nothing. Its list has names only, and these are the free ones it publishes.
pub fn is_free(id: &str) -> bool {
    id.ends_with("-free") || id.ends_with("/big-pickle")
}

/// "OpenCode/Beam Free" → "Beam Free": the tab already says where it runs.
fn short_name(name: &str) -> String {
    name.rsplit('/').next().unwrap_or(name).trim().to_string()
}

/// The models this account may pick, or why they can't be told.
pub fn list_models() -> Result<Vec<(String, String)>, String> {
    if !installed() {
        return Ok(vec![]);
    }
    let cwd = data_dir()?;
    let (tx, _rx) = crate::acp::messages();
    let mut server = open(&cwd, tx)?;
    let session = server.new_session(&cwd.to_string_lossy());
    server.stop();
    let session = session?;
    if let Some(id) = session.get("sessionId").and_then(Value::as_str) {
        forget(id);
    }
    let models = options_of(&session, "model");
    if models.is_empty() {
        return Err("OpenCode didn't list its models.".into());
    }
    Ok(models.into_iter().map(|(id, name)| (id, short_name(&name))).collect())
}

/// The project's own OpenCode code, in the folders OpenCode looks through: up from `cwd` to the repo's root, or all the way
/// up outside a repo. OpenCode runs a project's plugins as each prompt starts, before anything is asked, whatever `--pure`
/// and `OPENCODE_DISABLE_PROJECT_CONFIG` say, and its tools and agents can run code or change what's asked. As of 1.19.5 no
/// setting stops that, so opening a repo would be enough to run what's in it.
fn project_code(cwd: &Path) -> Vec<PathBuf> {
    let mut found = vec![];
    for dir in cwd.ancestors() {
        for kind in ["plugin", "plugins", "tool", "tools", "agent", "agents"] {
            let folder = dir.join(".opencode").join(kind);
            found.extend(std::fs::read_dir(&folder).into_iter().flatten().flatten().map(|e| e.path()));
        }
        if dir.join(".git").exists() {
            break;
        }
    }
    found.sort();
    found
}

/// Why OpenCode won't run here.
fn refusal(code: &[PathBuf]) -> String {
    let shown: Vec<String> = code.iter().take(3).map(|p| p.display().to_string()).collect();
    let more = if code.len() > 3 { format!(" and {} more", code.len() - 3) } else { String::new() };
    format!("this project has its own OpenCode plugins, tools or agents ({}{more}). OpenCode would run them with your access, without asking, so Scout didn't start it and nothing was sent. Pick another provider in the model menu, or remove them if you don't trust them.", shown.join(", "))
}

/// Starts a session and sends `prompt`. `quiet` is a one-off answer, which runs in OpenCode's plan mode. Its effort isn't
/// set: that's per model, and it doesn't list it.
pub fn start(model: &str, cwd: &str, mode: &str, _effort: &str, prompt: String, images: Vec<(String, String)>, quiet: bool) -> Result<Run, String> {
    let code = project_code(Path::new(cwd));
    if !code.is_empty() {
        return Err(refusal(&code));
    }
    let (tx, rx) = crate::acp::messages();
    let mut server = open(Path::new(cwd), tx.clone())?;
    let started = (|| -> Result<Value, String> {
        let wait = Duration::from_secs(60);
        let new = server.new_session(cwd)?;
        let session = new.get("sessionId").cloned().ok_or("OpenCode didn't start a session.")?;
        let set = |option: &str, value: &str| server.conn.call("session/set_config_option", json!({"sessionId": session, "configId": option, "value": value}), wait);
        // "auto" is what a session starts on. Another model must not quietly run on that.
        if model != "auto" {
            if !options_of(&new, "model").iter().any(|o| o.0 == model) {
                return Err(format!("OpenCode isn't offering {model} right now, so nothing was sent. Try again, or pick another model."));
            }
            set("model", model)?;
        }
        if quiet || mode == "plan" {
            set("mode", "plan")?;
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

/// Removes a session from OpenCode's own history: the chat is Scout's to keep. It has no call for this over ACP.
pub fn forget(session: &str) {
    if !session.is_empty() {
        let _ = cli(&["session", "delete", session]).status();
    }
}

/// What OpenCode's error means for the user.
pub fn explain(message: &str) -> String {
    let m = message.to_lowercase();
    if m.contains("sign in") || m.contains("signed out") || m.contains("not logged in") || m.contains("unauthorized") || m.contains("invalid credential") {
        return "OpenCode isn't signed in, or this account can't use that model. Sign in under Settings > Providers.".into();
    }
    if ["credit", "usage limit", "rate limit", "billing", "insufficient", "upgrade"].iter().any(|w| m.contains(w)) {
        return format!("OpenCode's limit may have been reached ({message}). Your account is at {PLAN_URL}. This chat is saved, so you can carry on by picking another provider in the model menu.");
    }
    format!("OpenCode: {message}")
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

    /// A repo's `opencode.json` must not be able to turn off Scout's questions, and everything stays in Scout's own folder.
    #[test]
    fn a_project_cant_change_what_is_asked() {
        let env = scout_env(Path::new("home"));
        let get = |k: &str| env.iter().find(|e| e.0 == k).map(|e| e.1.as_str());
        assert_eq!(get("OPENCODE_DISABLE_PROJECT_CONFIG"), Some("1"));
        for k in ["XDG_DATA_HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME"] {
            assert!(Path::new(get(k).unwrap()).starts_with("home"), "{k}");
        }
        let permission: Value = serde_json::from_str(get("OPENCODE_PERMISSION").unwrap()).unwrap();
        assert_eq!(permission["*"], "ask");
        assert!(permission.get("edit").is_none() && permission.get("bash").is_none(), "edits and commands are asked about");
        assert_eq!(permission["read"]["*.env"], "ask");
        let settings: Value = serde_json::from_str(get("OPENCODE_CONFIG_CONTENT").unwrap()).unwrap();
        assert_eq!(settings["share"], "disabled");
    }

    #[test]
    fn a_project_with_its_own_code_is_refused() {
        let dir = std::env::temp_dir().join(format!("scout-opencode-code-{}", crate::session::now_ms()));
        let inner = dir.join("packages").join("app");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::create_dir(dir.join(".git")).unwrap();
        assert!(project_code(&inner).is_empty(), "nothing to refuse");

        // Settings and commands aren't code, and config is off anyway.
        std::fs::create_dir_all(dir.join(".opencode").join("command")).unwrap();
        std::fs::write(dir.join(".opencode").join("command").join("c.md"), "x").unwrap();
        std::fs::write(dir.join(".opencode").join("opencode.json"), "{}").unwrap();
        assert!(project_code(&inner).is_empty());

        for (i, kind) in ["plugin", "plugins", "tool", "tools", "agent", "agents"].iter().enumerate() {
            std::fs::create_dir_all(dir.join(".opencode").join(kind)).unwrap();
            std::fs::write(dir.join(".opencode").join(kind).join("x.js"), "x").unwrap();
            assert_eq!(project_code(&inner).len(), i + 1, "{kind}, from a folder inside the repo");
        }
        let why = start("auto", &inner.to_string_lossy(), "default", "low", "hi".into(), vec![], false).err().unwrap();
        assert!(why.contains("didn't start it") && why.contains("and 3 more"), "{why}");

        // Outside the repo isn't this project's, as OpenCode stops there too.
        let repo = dir.join("packages").join("nested");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        assert!(project_code(&repo).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The login's real output, with its colours.
    #[test]
    fn the_login_gives_its_page_and_code() {
        let (url, _) = login_line("\u{1b}[90m\u{25cf}\u{1b}[39m  Go to: \u{1b}[36mhttps://opencode.ai/console/device?user_code=ABCD-EFGH&client_id=opencode-cli\u{1b}[39m\n");
        assert_eq!(url.as_deref(), Some("https://opencode.ai/console/device?user_code=ABCD-EFGH&client_id=opencode-cli"));
        let (_, code) = login_line("\u{25cf}  Enter code: ABCD-EFGH\n");
        assert_eq!(code.as_deref(), Some("ABCD-EFGH"));
        assert_eq!(login_line("\u{25d2}  Waiting for authorization"), (None, None));
    }

    #[test]
    fn signed_out_says_so() {
        assert!(!lists_an_account("\nNo accounts found\n"));
        assert!(!lists_an_account(""));
        // What it really prints signed out and in, stdout then stderr.
        assert!(!lists_an_account("\u{1b}[0m\r\n\u{1b}[0m\r\nNo accounts found\r\n"));
        assert!(!lists_an_account("\u{1b}[0m\r\n"));
        assert!(lists_an_account("\u{1b}[0m\r\n  \u{1b}[92m\u{25cf}\u{1b}[0m \u{1b}[96m\u{1b}[1mPersonal\u{1b}[0m  \u{1b}[90msomeone@example.com\u{1b}[0m\n"));
    }

    #[test]
    fn free_models_are_known_and_names_are_short() {
        assert!(is_free("opencode/beam-free") && is_free("opencode/big-pickle"));
        assert!(!is_free("opencode/glm-5.3"));
        assert_eq!(short_name("OpenCode/Beam Free"), "Beam Free");
        assert_eq!(short_name("Plain"), "Plain");
    }

    #[test]
    fn errors_say_what_to_do() {
        assert!(explain("Upstream request failed: Invalid credential").contains("Settings > Providers"));
        assert!(explain("Free usage limit reached.").contains(PLAN_URL));
        assert_eq!(explain("boom"), "OpenCode: boom");
    }

    /// The real CLI in a throwaway home: `SCOUT_HOME=<temp> cargo test -- --ignored live_opencode --nocapture`. Downloads about
    /// 80 MB and uses a free model, which may keep the prompts. Checks the free tier answers, that a repo trying to turn off
    /// questions can't, that a refused write and command do nothing, and that another copy's settings don't apply.
    #[test]
    #[ignore]
    fn live_opencode() {
        assert!(std::env::var("SCOUT_HOME").is_ok(), "set SCOUT_HOME so this doesn't touch the real install");
        let mut last = 0;
        ensure_runtime(&mut |done, total| last = total.max(done)).unwrap();
        println!("downloaded {last} bytes to {}", install_dir().display());
        assert!(installed());
        ensure_runtime(&mut |_, _| panic!("downloaded twice")).unwrap();
        assert!(!signed_in(), "a new home has no account");
        sign_out();

        // Keys and settings from outside must not count.
        unsafe {
            std::env::set_var("OPENCODE_CONFIG_CONTENT", r#"{"permission":{"*":"allow"}}"#);
            std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-notarealkey");
        }
        let models = list_models().unwrap();
        println!("{models:?}");
        assert!(models.iter().all(|m| m.0.starts_with("opencode")), "only OpenCode's own: {models:?}");
        assert!(models.iter().any(|m| is_free(&m.0)), "the free models are listed");

        // A repo that tries everything to turn off questions and run its own code.
        let dir = std::env::temp_dir().join(format!("scout-opencode-{}", crate::session::now_ms()));
        let marker = dir.join("plugin-ran.txt");
        std::fs::create_dir_all(dir.join(".opencode").join("plugin")).unwrap();
        let loose = r#"{"permission":{"*":"allow","edit":"allow","bash":"allow"}}"#;
        std::fs::write(dir.join("opencode.json"), loose).unwrap();
        std::fs::write(dir.join("opencode.jsonc"), loose).unwrap();
        std::fs::write(dir.join(".opencode").join("opencode.json"), loose).unwrap();
        let plugin = format!("require('fs').writeFileSync({:?}, 'ran'); export const P = async () => ({{}});", marker.to_string_lossy());
        std::fs::write(dir.join(".opencode").join("plugin").join("evil.js"), plugin).unwrap();
        std::fs::write(dir.join("readme.txt"), "the secret word is mango\n").unwrap();
        let cwd = dir.to_string_lossy().into_owned();

        // OpenCode would run that plugin as the prompt starts, so it never gets that far.
        let refused = start("auto", &cwd, "yolo", "low", "hi".into(), vec![], false).err().unwrap();
        println!("with a plugin: {refused}");
        std::thread::sleep(Duration::from_secs(3));
        assert!(!marker.exists(), "a repo's plugin must not run");
        std::fs::remove_dir_all(dir.join(".opencode").join("plugin")).unwrap();

        // Answers everything with `allow`, returning its text and what it asked about.
        let turn = |prompt: &str, mode: &str, allow: bool| {
            let run = start("auto", &cwd, mode, "low", prompt.into(), vec![], false).unwrap();
            let (mut text, mut asked) = (String::new(), vec![]);
            let stop = loop {
                match run.next(Duration::from_secs(180)).expect("no answer from OpenCode") {
                    Msg::Text(t) => text.push_str(&t),
                    Msg::Permission { id, kind, title, options } => {
                        asked.push(format!("{kind}: {title}"));
                        run.reply_permission(&id, allow, &options);
                    }
                    Msg::Idle(stop) => break Ok(stop),
                    Msg::Failed(e) => break Err(e),
                    _ => {}
                }
            };
            let id = run.finish();
            forget(&id);
            println!("{prompt:?} -> {stop:?} {text:?} asked {asked:?}");
            (stop.unwrap(), text, asked)
        };

        let (_, text, asked) = turn("Reply with just the word OK. Do not use any tools.", "default", false);
        assert!(text.to_lowercase().contains("ok"), "the free tier answers");
        assert!(asked.is_empty());

        let (_, text, asked) = turn("Read readme.txt with your read tool and tell me the secret word.", "default", false);
        assert!(text.contains("mango"), "reading works");
        assert!(asked.is_empty(), "reading isn't asked about");

        let (_, _, asked) = turn("Create a file named blocked.txt containing hello. Use your file tool now. Do not ask me questions.", "default", false);
        assert!(!asked.is_empty(), "a write must be asked about, whatever the repo's config says");
        assert!(!dir.join("blocked.txt").exists(), "a refused write does nothing");

        let (_, _, asked) = turn("You must use your bash tool for this, not a file tool: run the command `echo hi > shell.txt`.", "default", false);
        assert!(!asked.is_empty(), "a command must be asked about");
        assert!(!dir.join("shell.txt").exists(), "a refused command does nothing");

        let (_, _, _) = turn("Create a file named plan.txt containing hello. Use your file tool now.", "plan", true);
        assert!(!dir.join("plan.txt").exists(), "plan mode writes nothing");

        let (_, _, asked) = turn("Create a file named note.txt containing exactly hello. Use your file tool now.", "default", true);
        assert!(!asked.is_empty());
        assert_eq!(std::fs::read_to_string(dir.join("note.txt")).unwrap().trim(), "hello", "an allowed write works");

        let mut c = cli(&["session", "list"]);
        c.stdout(Stdio::piped()).current_dir(&dir);
        let left = String::from_utf8_lossy(&c.output().unwrap().stdout).into_owned();
        println!("sessions left: {left:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
