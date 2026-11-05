//! A copy of Node.js that's Scout's alone, for the agents that only ship as npm packages: Codex and Claude Code. Node is
//! downloaded when one is first wanted and checked against the hashes below. Each agent's packages come from a lockfile
//! in `src/npm`, which pins every package with its SHA-512, and npm refuses anything that doesn't match. Nothing in them
//! runs while they install.

use crate::tools::{kill_tree, no_window};
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Node's version. It only changes with a Scout release: see "Codex and Claude Code" in docs/providers.md.
const VERSION: &str = "24.22.2";
/// The SHA-256 of each platform's archive at that version, from nodejs.org's SHASUMS256.txt.
const PACKAGES: [(&str, &str); 6] = [
    ("win-x64.zip", "ca1bff5d8ead13c9f2ea404a176db46bd430704d5b5113734049fa365e7799e2"),
    ("win-arm64.zip", "f47968c213105077f64c6510f007632c0ca2574802a0b5067c327c49abe1c468"),
    ("darwin-arm64.tar.gz", "aa9549349e47dffaa42dcf95d6e06ade8c4bdafc980979883b88767670df385a"),
    ("darwin-x64.tar.gz", "d5e336d588c45be0c37dbd52cc29895593009ca4997a77a1fd02815131ed0e52"),
    ("linux-x64.tar.gz", "361718c444e49af2ca5a110c83ba6a8901143f23ab285ebbef0341f391f482a9"),
    ("linux-arm64.tar.gz", "95c264150aceb7f3437a29fd6984924f5f8d1511ea3cbd6c508b83f6c555dae2"),
];

/// How setting an agent up or signing in to it is going, for the window.
pub enum Update {
    Progress { done: u64, total: u64 },
    /// It's installed, and the browser should be open on this page.
    Waiting(Option<String>),
    Done,
    Failed(String),
}

/// The archive for this platform.
fn package() -> Result<(&'static str, &'static str), String> {
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
    let name = if cfg!(windows) { format!("win-{arch}.zip") } else if cfg!(target_os = "macos") { format!("darwin-{arch}.tar.gz") } else { format!("linux-{arch}.tar.gz") };
    PACKAGES.iter().find(|p| p.0 == name).copied().ok_or_else(|| format!("Node.js isn't available as {name}."))
}

fn root() -> PathBuf {
    crate::config::home().join("runtime").join("node")
}

/// Where the archive unpacks to: one folder, named after it.
fn node_dir() -> Result<PathBuf, String> {
    let (name, _) = package()?;
    let folder = name.trim_end_matches(".zip").trim_end_matches(".tar.gz");
    Ok(root().join(VERSION).join(format!("node-v{VERSION}-{folder}")))
}

pub fn node_exe() -> PathBuf {
    let dir = node_dir().unwrap_or_default();
    if cfg!(windows) { dir.join("node.exe") } else { dir.join("bin").join("node") }
}

fn npm_cli() -> PathBuf {
    let dir = node_dir().unwrap_or_default();
    let modules = if cfg!(windows) { dir.join("node_modules") } else { dir.join("lib").join("node_modules") };
    modules.join("npm").join("bin").join("npm-cli.js")
}

/// Downloads Node unless it's there.
fn ensure_node(progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
    static INSTALLING: Mutex<()> = Mutex::new(());
    let _one_at_a_time = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    if node_exe().is_file() && npm_cli().is_file() {
        return Ok(());
    }
    let (name, want) = package()?;
    let url = format!("https://nodejs.org/dist/v{VERSION}/node-v{VERSION}-{name}");
    crate::acp::install("Node.js", &url, want, &root(), VERSION, &|file, to| crate::devin::unpack(file, name, to), progress)
}

/// Settings from outside that would change what Node or npm load: a `--require` in `NODE_OPTIONS`, another registry, or
/// lifecycle scripts turned back on.
fn is_outside_setting(name: &str) -> bool {
    let name = name.to_uppercase();
    name.starts_with("NODE_") || name.starts_with("NPM_CONFIG_")
}

/// Node, with nothing from outside changing how it starts.
pub fn command() -> Command {
    let mut c = Command::new(node_exe());
    for (k, _) in std::env::vars_os() {
        if is_outside_setting(&k.to_string_lossy()) {
            c.env_remove(k);
        }
    }
    c
}

/// An agent's npm packages, pinned by a lockfile.
pub struct Agent {
    pub name: &'static str,
    /// Its folder under `~/.scoutcode/runtime`.
    pub folder: &'static str,
    /// Its adapter's version, which names the folder it's installed in.
    pub version: &'static str,
    pub package_json: &'static str,
    pub lock: &'static str,
    /// The script to run, inside the folder.
    pub entry: &'static str,
}

impl Agent {
    fn root(&self) -> PathBuf {
        crate::config::home().join("runtime").join(self.folder)
    }

    /// The installed version's folder, with its `node_modules`.
    pub fn dir(&self) -> PathBuf {
        self.root().join(self.version)
    }

    pub fn entry(&self) -> PathBuf {
        self.root().join(self.version).join(self.entry)
    }

    pub fn installed(&self) -> bool {
        node_exe().is_file() && self.entry().is_file()
    }

    /// Deletes its packages. Node stays while another agent might use it.
    pub fn remove(&self) {
        let _ = std::fs::remove_dir_all(self.root());
    }

    /// Node running the agent's script, with nothing from outside changing how Node starts.
    pub fn command(&self) -> Command {
        let mut c = command();
        c.arg(self.entry());
        c
    }

    /// Downloads Node, then the packages, unless they're there. The folder only gets its name once npm has finished, and
    /// older versions are deleted afterwards. npm only gets what the lockfile names, checks each against its SHA-512, and
    /// runs no install scripts.
    pub fn ensure(&self, progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
        static INSTALLING: Mutex<()> = Mutex::new(());
        ensure_node(progress)?;
        let _one_at_a_time = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
        if self.installed() {
            return Ok(());
        }
        let name = self.name;
        let root = self.root();
        let tmp = root.join(format!("{}.partial", self.version));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        let finish = (|| {
            std::fs::write(tmp.join("package.json"), self.package_json).and_then(|_| std::fs::write(tmp.join("package-lock.json"), self.lock)).map_err(|e| e.to_string())?;
            let cache = crate::config::home().join("runtime").join("npm-cache");
            let mut c = command();
            c.arg(npm_cli())
                .args(["ci", "--ignore-scripts", "--no-audit", "--no-fund", "--omit=dev", "--loglevel=error"])
                .env("npm_config_cache", &cache)
                // No .npmrc from the user or the machine: the lockfile says where everything comes from.
                .env("npm_config_userconfig", tmp.join("no-user-npmrc"))
                .env("npm_config_globalconfig", tmp.join("no-global-npmrc"))
                .env("npm_config_update_notifier", "false")
                .current_dir(&tmp)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped());
            no_window(&mut c);
            let mut child = c.spawn().map_err(|e| format!("Couldn't install {name}: {e}"))?;
            let mut said = String::new();
            if let Some(mut err) = child.stderr.take() {
                let _ = err.read_to_string(&mut said);
            }
            let status = child.wait().map_err(|e| e.to_string())?;
            let _ = std::fs::remove_dir_all(&cache);
            if !status.success() {
                let last = said.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
                return Err(format!("Couldn't install {name}: npm failed ({status}). {last}"));
            }
            if !tmp.join(self.entry).is_file() {
                return Err(format!("Couldn't install {name}: npm didn't install it."));
            }
            let _ = std::fs::remove_dir_all(root.join(self.version));
            std::fs::rename(&tmp, root.join(self.version)).map_err(|e| e.to_string())
        })();
        if finish.is_err() {
            let _ = std::fs::remove_dir_all(&tmp);
        }
        finish?;
        for e in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            if e.file_name() != self.version {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
        Ok(())
    }
}

/// The first web address in a line of a login's output.
fn url_in(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let url: String = line[start..].chars().take_while(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | ')' | '>' | '<')).collect();
    Some(url.trim_end_matches(['.', ',']).to_string())
}

/// Installs an agent, then runs its login, which opens the browser, unless it's signed in already. The page it opens is
/// passed on too, in case the browser didn't open.
pub fn sign_in(name: &'static str, ensure: fn(&mut dyn FnMut(u64, u64)) -> Result<(), String>, signed_in: fn() -> bool, login: fn() -> Command, cancel: Arc<AtomicBool>, send: impl Fn(Update) + Clone + Send + 'static) {
    std::thread::spawn(move || {
        let run = || -> Result<(), String> {
            ensure(&mut |done, total| send(Update::Progress { done, total }))?;
            if cancel.load(Ordering::Relaxed) {
                return Err(String::new());
            }
            send(Update::Waiting(None));
            if signed_in() {
                return Ok(());
            }
            let mut c = login();
            // Its input stays open and empty: a login can take an end of input as the user giving up.
            c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
            no_window(&mut c);
            let mut child = c.spawn().map_err(|e| format!("Couldn't start {name}: {e}"))?;
            let _input = child.stdin.take();
            let found = Arc::new(AtomicBool::new(false));
            let readers: Vec<Box<dyn Read + Send>> = [child.stdout.take().map(|o| Box::new(o) as Box<dyn Read + Send>), child.stderr.take().map(|e| Box::new(e) as Box<dyn Read + Send>)].into_iter().flatten().collect();
            for out in readers {
                let (told, found) = (send.clone(), found.clone());
                std::thread::spawn(move || {
                    for line in BufReader::new(out).lines() {
                        let Ok(line) = line else { break };
                        if let Some(url) = url_in(&line) {
                            if !found.swap(true, Ordering::Relaxed) {
                                told(Update::Waiting(Some(url)));
                            }
                        }
                    }
                });
            }
            let deadline = Instant::now() + Duration::from_secs(600);
            let result = loop {
                if cancel.load(Ordering::Relaxed) {
                    break Err(String::new());
                }
                if Instant::now() > deadline {
                    break Err("The sign-in timed out. Try again.".to_string());
                }
                match child.try_wait() {
                    Ok(Some(_)) => break Ok(()),
                    Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                    Err(e) => break Err(e.to_string()),
                }
            };
            kill_tree(&mut child);
            let _ = child.wait();
            result?;
            if signed_in() { Ok(()) } else { Err(format!("{name} didn't finish signing in. Try again.")) }
        };
        match run() {
            Ok(()) => send(Update::Done),
            Err(e) => send(Update::Failed(e)),
        }
    });
}

/// Runs a short command of an agent's to the end, with nothing read from it.
pub fn quietly(mut c: Command) -> bool {
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    no_window(&mut c);
    c.status().is_ok_and(|s| s.success())
}

/// A folder of an agent's own, made if it's missing.
pub fn folder(name: &str) -> Result<PathBuf, String> {
    let d = crate::config::home().join("runtime").join(name);
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
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
        assert!(node_exe().to_string_lossy().contains(VERSION));
    }

    #[test]
    fn nothing_from_outside_changes_node() {
        assert!(is_outside_setting("NODE_OPTIONS") && is_outside_setting("npm_config_registry") && is_outside_setting("NPM_CONFIG_IGNORE_SCRIPTS"));
        assert!(!is_outside_setting("PATH"));
    }

    #[test]
    fn the_login_page_is_found() {
        assert_eq!(url_in("If your browser did not open, navigate to this URL to authenticate: https://auth.openai.com/oauth/authorize?a=1&b=2").as_deref(), Some("https://auth.openai.com/oauth/authorize?a=1&b=2"));
        assert_eq!(url_in("Browser didn't open? Use the url below to sign in (c to copy)\n").as_deref(), None);
        assert_eq!(url_in("  https://claude.ai/oauth/authorize?code=true.").as_deref(), Some("https://claude.ai/oauth/authorize?code=true"));
    }

    /// Installs Node, Codex and Claude Code in a throwaway home: `SCOUT_HOME=<temp> cargo test -- --ignored live_node_agents
    /// --nocapture`. Downloads about 300 MB. Signing in and real turns need accounts, so they aren't covered.
    #[test]
    #[ignore]
    fn live_node_agents() {
        assert!(std::env::var("SCOUT_HOME").is_ok(), "set SCOUT_HOME so this doesn't touch the real install");
        for (name, ensure, installed) in [("Codex", crate::codex::ensure_runtime as fn(&mut dyn FnMut(u64, u64)) -> Result<(), String>, crate::codex::installed as fn() -> bool), ("Claude Code", crate::claude::ensure_runtime, crate::claude::installed)] {
            ensure(&mut |_, _| {}).unwrap();
            assert!(installed(), "{name}");
            ensure(&mut |_, _| panic!("{name} installed twice")).unwrap();
            println!("{name} installed");
        }
        assert!(!crate::codex::signed_in(), "a new home has no account");
    }

    #[test]
    fn the_lockfiles_match_their_packages() {
        for (package, lock) in [(include_str!("npm/codex/package.json"), include_str!("npm/codex/package-lock.json")), (include_str!("npm/claude/package.json"), include_str!("npm/claude/package-lock.json"))] {
            let package: serde_json::Value = serde_json::from_str(package).unwrap();
            let lock: serde_json::Value = serde_json::from_str(lock).unwrap();
            assert_eq!(package["dependencies"], lock["packages"][""]["dependencies"]);
            for (path, p) in lock["packages"].as_object().unwrap().iter().filter(|(k, _)| !k.is_empty()) {
                assert!(p["integrity"].as_str().is_some_and(|i| i.starts_with("sha512-")), "{path} has no SHA-512");
                assert!(p["resolved"].as_str().is_some_and(|r| r.starts_with("https://registry.npmjs.org/")), "{path} comes from elsewhere");
                assert!(p.get("hasInstallScript").is_none(), "{path} has an install script");
            }
        }
    }
}
