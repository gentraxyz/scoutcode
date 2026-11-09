//! Keeps the installed app current. A worker thread reads the release manifest at startup and every few hours, and
//! downloads anything newer next to the exe. The new build takes over when the user clicks "Restart to update", or when
//! the window closes. Windows lets a running exe be renamed, so it's moved aside and the new one put in its place.
//!
//! Builds are signed with a key kept off the servers (see scripts/release-desktop.ts), so whoever controls the site or
//! the bucket still can't push code to installed copies. The signature covers the version too, which stops an old
//! build being passed off as a new one.
//!
//! The same exe doubles as the installer: downloaded as ScoutSetup.exe, it installs itself and starts the real copy.

use crate::api::http;
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

const MANIFEST_URL: &str = "https://scoutcode.dev/download/latest.json";
/// The public half of the release signing key.
const PUBLIC_KEY: &str = "FkAxNq1jjgh/jX3njTG6VaAYarAfh9vnOhemGeXcZ7I=";
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(5);
const CHECK_EVERY: Duration = Duration::from_secs(30);
const MANIFEST_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Deserialize, Clone)]
struct Manifest {
    version: String,
    /// The summary and changelog together, from releases before they were separate.
    #[serde(default)]
    notes: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    changelog: String,
    platforms: HashMap<String, Build>,
}

#[derive(Deserialize, Clone)]
struct Build {
    url: String,
    sha256: String,
    size: u64,
    signature: String,
}

#[derive(Clone)]
struct Release {
    version: String,
    summary: String,
    changelog: String,
    build: Build,
}

/// One release's notes, from releases.json.
#[derive(Deserialize, serde::Serialize, Clone, Debug)]
pub struct ReleaseNote {
    pub version: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub changelog: String,
}

#[derive(Clone, PartialEq)]
pub enum Status {
    Idle,
    Checking,
    UpToDate,
    /// Newer than this build, and waiting for the user because automatic downloads are off.
    Available { version: String, summary: String, changelog: String },
    Downloading { version: String, got: u64, total: u64 },
    /// Downloaded and verified: it takes over at the next restart.
    Ready { version: String, summary: String, changelog: String },
    Failed(String),
}

enum Command {
    Check,
    Download,
}

pub struct Updater {
    status: Arc<Mutex<Status>>,
    auto: Arc<AtomicBool>,
    /// None when this copy doesn't update itself.
    worker: Option<Sender<Command>>,
}

/// The exe's path, read before any swap, because the name Windows reports for a running exe follows the rename.
fn exe_path() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| std::env::current_exe().unwrap_or_default())
}

fn exe_stem() -> String {
    exe_path().file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Only an installed release build updates itself, never `cargo run` or a copy under target/.
pub fn self_updating() -> bool {
    cfg!(windows) && !cfg!(debug_assertions) && exe_stem() == "scout" && std::env::var_os("SCOUT_NO_UPDATE").is_none()
}

fn pending_path(exe: &Path, version: &str) -> PathBuf {
    exe.with_file_name(format!("update-{version}.exe"))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    fn parts(v: &str) -> Option<[u64; 3]> {
        let mut it = v.split('.').map(|p| p.parse::<u64>().ok());
        let parsed = [it.next()??, it.next()??, it.next()??];
        it.next().is_none().then_some(parsed)
    }
    matches!((parts(candidate), parts(current)), (Some(a), Some(b)) if a > b)
}

/// What a release's signature covers.
fn signed_message(platform: &str, version: &str, sha256: &str) -> String {
    format!("scout-desktop:{platform}:{version}:{sha256}")
}

fn verify_signature(public_key: &str, message: &str, signature: &str) -> Result<(), String> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let key: [u8; 32] = b64.decode(public_key).ok().and_then(|k| k.try_into().ok()).ok_or("The built-in release key is invalid.")?;
    let sig: [u8; 64] = b64.decode(signature).ok().and_then(|s| s.try_into().ok()).ok_or("The release has a malformed signature.")?;
    let key = VerifyingKey::from_bytes(&key).map_err(|_| "The built-in release key is invalid.")?;
    key.verify(message.as_bytes(), &Signature::from_bytes(&sig)).map_err(|_| "The release isn't signed by ScoutCode, so it wasn't installed.".to_string())
}

fn sha256_of(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).ok()?;
    Some(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Puts `new` in `exe`'s place. The old exe is moved aside, not deleted, because it may be running. If the new one can't
/// be moved in, the old one goes back. Each old copy gets its own name, since one still in use can't be replaced.
fn swap(exe: &Path, new: &Path) -> std::io::Result<()> {
    let old = exe.with_file_name(format!("{}.{}.old", exe.file_name().unwrap_or_default().to_string_lossy(), std::process::id()));
    if exe.exists() {
        std::fs::rename(exe, &old)?;
    }
    if let Err(e) = std::fs::rename(new, exe) {
        let _ = std::fs::rename(&old, exe);
        return Err(e);
    }
    Ok(())
}

/// Clears what earlier updates left next to the exe: old copies (skipped while still running) and downloads other
/// than `keep`.
fn clean_folder(exe: &Path, keep: Option<&Path>) {
    let Some(dir) = exe.parent() else { return };
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let leftover = name.ends_with(".old") || (name.starts_with("update-") && Some(path.as_path()) != keep);
        if leftover {
            let _ = std::fs::remove_file(&path);
        }
    }
}

impl Updater {
    pub fn start(ctx: &crate::waker::Waker, auto: bool) -> Self {
        let status = Arc::new(Mutex::new(Status::Idle));
        let auto = Arc::new(AtomicBool::new(auto));
        let mut worker = None;
        if self_updating() {
            let (tx, rx) = channel();
            let w = Worker { status: status.clone(), auto: auto.clone(), ctx: ctx.clone(), found: None };
            std::thread::spawn(move || w.run(rx));
            worker = Some(tx);
        }
        Updater { status, auto, worker }
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    pub fn enabled(&self) -> bool {
        self.worker.is_some()
    }

    pub fn check_now(&self) {
        if let Some(w) = &self.worker {
            let _ = w.send(Command::Check);
        }
    }

    pub fn download_now(&self) {
        if let Some(w) = &self.worker {
            let _ = w.send(Command::Download);
        }
    }

    /// Turning this on also fetches an update that's already waiting.
    pub fn set_auto(&self, on: bool) {
        self.auto.store(on, Ordering::Relaxed);
        if on && matches!(self.status(), Status::Available { .. }) {
            self.download_now();
        }
    }

    /// Moves a downloaded update into place, for the next start. Does nothing when none is waiting, or another open
    /// window of the app already did it.
    pub fn install(&self) -> Result<(), String> {
        let Status::Ready { version, .. } = self.status() else { return Ok(()) };
        let new = pending_path(exe_path(), &version);
        if new.exists() {
            swap(exe_path(), &new).map_err(|e| format!("Couldn't install the update: {e}"))?;
            set_display_version(&version);
        }
        *self.status.lock().unwrap() = Status::Idle;
        Ok(())
    }

    /// Installs the update and starts the new build with `args`. The caller closes this window.
    pub fn restart(&self, args: &[String]) -> Result<(), String> {
        self.install()?;
        // The old copy is still closing, so the new one waits for it (see single_instance.rs) rather than handing off.
        std::process::Command::new(exe_path()).args(args).env(crate::single_instance::RESTART_ENV, "1").spawn().map(|_| ()).map_err(|e| format!("Installed, but couldn't restart Scout: {e}"))
    }
}

struct Worker {
    status: Arc<Mutex<Status>>,
    auto: Arc<AtomicBool>,
    ctx: crate::waker::Waker,
    /// What the last check found, for a download the user asks for.
    found: Option<Release>,
}

impl Worker {
    fn run(mut self, rx: Receiver<Command>) {
        clean_folder(exe_path(), None);
        let mut wait = FIRST_CHECK_AFTER;
        loop {
            match rx.recv_timeout(wait) {
                Ok(Command::Check) => {
                    self.check(false);
                    wait = CHECK_EVERY;
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.check(true);
                    wait = CHECK_EVERY;
                }
                Ok(Command::Download) => {
                    if let Some(release) = self.found.clone() {
                        self.fetch(&release);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn set(&self, status: Status) {
        *self.status.lock().unwrap() = status;
        self.ctx.request_repaint();
    }

    /// A `quiet` check is the timer's: it doesn't show "Checking", and a failed one leaves the status as it was.
    fn check(&mut self, quiet: bool) {
        let waiting = match &*self.status.lock().unwrap() {
            Status::Ready { version, .. } => Some(version.clone()),
            _ => None,
        };
        if waiting.is_none() && !quiet {
            self.set(Status::Checking);
        }
        match latest() {
            Ok(Some(release)) if is_newer(&release.version, VERSION) => {
                // Already downloaded and nothing newer has come out since.
                if waiting.as_deref() == Some(release.version.as_str()) {
                    return;
                }
                let pending = pending_path(exe_path(), &release.version);
                clean_folder(exe_path(), Some(&pending));
                self.found = Some(release.clone());
                if sha256_of(&pending).as_deref() == Some(release.build.sha256.as_str()) {
                    self.set(Status::Ready { version: release.version, summary: release.summary, changelog: release.changelog });
                } else if self.auto.load(Ordering::Relaxed) {
                    self.fetch(&release);
                } else {
                    self.set(Status::Available { version: release.version, summary: release.summary, changelog: release.changelog });
                }
            }
            Ok(_) => {
                // A release that's been withdrawn shouldn't be installed either.
                clean_folder(exe_path(), None);
                self.found = None;
                self.set(Status::UpToDate);
            }
            // An update that's ready doesn't stop being ready because the network dropped.
            Err(_) if waiting.is_some() || quiet => {}
            Err(e) => self.set(Status::Failed(e)),
        }
    }

    fn fetch(&self, release: &Release) {
        match self.download(release) {
            Ok(()) => self.set(Status::Ready { version: release.version.clone(), summary: release.summary.clone(), changelog: release.changelog.clone() }),
            Err(e) => self.set(Status::Failed(e)),
        }
    }

    fn download(&self, release: &Release) -> Result<(), String> {
        let total = release.build.size;
        self.set(Status::Downloading { version: release.version.clone(), got: 0, total });
        let pending = pending_path(exe_path(), &release.version);
        let part = pending.with_extension("part");
        let saved = (|| {
            let mut res = http().get(&release.build.url).config().timeout_global(Some(DOWNLOAD_TIMEOUT)).build().call().map_err(|e| e.to_string())?;
            if !res.status().is_success() {
                return Err(format!("The download answered {}.", res.status().as_u16()));
            }
            let mut file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
            let mut body = res.body_mut().as_reader();
            let mut hasher = Sha256::new();
            let mut buf = vec![0u8; 64 * 1024];
            let (mut got, mut shown) = (0u64, 0u64);
            loop {
                let n = body.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                got += n as u64;
                if got > total {
                    return Err("The download is bigger than the release says it should be.".to_string());
                }
                hasher.update(&buf[..n]);
                file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                // Redraws only as often as the bar moves.
                if got - shown >= total / 100 + 1 {
                    shown = got;
                    self.set(Status::Downloading { version: release.version.clone(), got, total });
                }
            }
            file.sync_all().map_err(|e| e.to_string())?;
            if hex(&hasher.finalize()) != release.build.sha256 {
                return Err("The download was damaged, so it wasn't installed.".to_string());
            }
            Ok(())
        })();
        if let Err(e) = saved {
            let _ = std::fs::remove_file(&part);
            return Err(format!("Couldn't download the update: {e}"));
        }
        std::fs::rename(&part, &pending).map_err(|e| format!("Couldn't save the update: {e}"))
    }
}

/// The release for this platform, checked against the signing key. None when there's nothing for this platform.
fn latest() -> Result<Option<Release>, String> {
    let url = std::env::var("SCOUT_UPDATE_URL").unwrap_or_else(|_| MANIFEST_URL.to_string());
    let failed = |e: String| format!("Couldn't check for updates: {e}");
    let mut res = http().get(&url).config().timeout_global(Some(MANIFEST_TIMEOUT)).build().call().map_err(|e| failed(e.to_string()))?;
    // The site answers 404 until the first release is out.
    if res.status().as_u16() == 404 {
        return Ok(None);
    }
    if !res.status().is_success() {
        return Err(failed(format!("the server answered {}", res.status().as_u16())));
    }
    let manifest: Manifest = res.body_mut().read_json().map_err(|e| failed(e.to_string()))?;
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let Some(build) = manifest.platforms.get(&platform).cloned() else { return Ok(None) };
    verify_signature(PUBLIC_KEY, &signed_message(&platform, &manifest.version, &build.sha256), &build.signature)?;
    let (summary, changelog) = if manifest.summary.is_empty() && manifest.changelog.is_empty() {
        (String::new(), manifest.notes)
    } else {
        (manifest.summary, manifest.changelog)
    };
    Ok(Some(Release { version: manifest.version, summary, changelog, build }))
}

/// Every release newer than `since`, newest first, for "What's new" after an update.
pub fn notes_since(since: &str) -> Result<Vec<ReleaseNote>, String> {
    let manifest = std::env::var("SCOUT_UPDATE_URL").unwrap_or_else(|_| MANIFEST_URL.to_string());
    let url = format!("{}?since={since}", manifest.replace("latest.json", "releases.json"));
    let mut res = http().get(&url).config().timeout_global(Some(MANIFEST_TIMEOUT)).build().call().map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("the server answered {}", res.status().as_u16()));
    }
    let notes: Vec<ReleaseNote> = res.body_mut().read_json().map_err(|e| e.to_string())?;
    // A folder release (--out) has no server to filter it.
    Ok(notes.into_iter().filter(|n| is_newer(&n.version, since) && !is_newer(&n.version, VERSION)).collect())
}

/// What Settings > Apps shows. Purely cosmetic, so a failure is ignored.
fn set_display_version(version: &str) {
    let mut c = std::process::Command::new("reg");
    c.args(["add", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\ScoutDesktop", "/v", "DisplayVersion", "/t", "REG_SZ", "/d", version, "/f"]);
    crate::tools::no_window(&mut c);
    let _ = c.output();
}

// ---------- ScoutSetup.exe ----------

/// Makes the Start menu entry and the Settings > Apps listing, like install.ps1. Paths come in through the environment
/// so nothing in them can be read as PowerShell.
#[cfg(windows)]
const REGISTER_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$dest = $env:SCOUT_DEST
$exe = Join-Path $dest 'Scout.exe'
$lnk = (New-Object -ComObject WScript.Shell).CreateShortcut((Join-Path ([Environment]::GetFolderPath('Programs')) 'Scout.lnk'))
$lnk.TargetPath = $exe
$lnk.WorkingDirectory = $dest
$lnk.IconLocation = "$exe,0"
$lnk.Description = 'Scout: a coding agent for your projects'
$lnk.Save()
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ScoutDesktop'
New-Item $key -Force | Out-Null
$props = @{
    DisplayName = 'Scout'; DisplayIcon = "$exe,0"; Publisher = 'ScoutCode'; DisplayVersion = $env:SCOUT_VERSION
    InstallLocation = $dest; NoModify = 1; NoRepair = 1
    UninstallString = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$dest\install.ps1`" -Uninstall"
}
foreach ($k in $props.Keys) { New-ItemProperty $key -Name $k -Value $props[$k] -Force | Out-Null }
"#;

/// When this exe is the downloaded installer, installs it, starts the installed copy and returns true.
pub fn run_setup_if_needed() -> bool {
    if !exe_stem().starts_with("scoutsetup") {
        return false;
    }
    #[cfg(windows)]
    if let Err(e) = install_from_setup() {
        rfd::MessageDialog::new().set_title("Scout").set_level(rfd::MessageLevel::Error).set_description(format!("Scout couldn't be installed: {e}")).show();
    }
    true
}

#[cfg(windows)]
fn install_from_setup() -> Result<(), String> {
    let base = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA isn't set")?;
    let dest = PathBuf::from(base).join("Programs").join("Scout");
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    let exe = dest.join("Scout.exe");
    // Over a running copy this has to be a swap too, so copy it next to the target first.
    let incoming = pending_path(&exe, VERSION);
    std::fs::copy(exe_path(), &incoming).map_err(|e| e.to_string())?;
    swap(&exe, &incoming).map_err(|e| e.to_string())?;
    // The uninstaller, which is what Settings > Apps runs.
    std::fs::write(dest.join("install.ps1"), include_str!("../install.ps1")).map_err(|e| e.to_string())?;

    let mut c = std::process::Command::new("powershell.exe");
    c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", REGISTER_SCRIPT]);
    c.env("SCOUT_DEST", &dest).env("SCOUT_VERSION", VERSION);
    crate::tools::no_window(&mut c);
    let out = c.output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    std::process::Command::new(&exe).current_dir(&dest).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn compares_versions_as_numbers() {
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
    }

    #[test]
    fn refuses_anything_that_is_not_a_version() {
        assert!(!is_newer("latest", "0.1.0"));
        assert!(!is_newer("1.2", "0.1.0"));
        assert!(!is_newer("1.2.3.4", "0.1.0"));
        assert!(!is_newer("1.2.3-beta", "0.1.0"));
    }

    fn sign(key: &SigningKey, message: &str) -> (String, String) {
        let b64 = base64::engine::general_purpose::STANDARD;
        (b64.encode(key.verifying_key().to_bytes()), b64.encode(key.sign(message.as_bytes()).to_bytes()))
    }

    #[test]
    fn accepts_a_signature_from_the_key() {
        let message = signed_message("windows-x86_64", "1.2.3", "abc");
        let (public, signature) = sign(&SigningKey::from_bytes(&[7; 32]), &message);
        assert!(verify_signature(&public, &message, &signature).is_ok());
    }

    #[test]
    fn refuses_another_key_and_a_changed_release() {
        let message = signed_message("windows-x86_64", "1.2.3", "abc");
        let (public, signature) = sign(&SigningKey::from_bytes(&[7; 32]), &message);
        let (other, _) = sign(&SigningKey::from_bytes(&[8; 32]), &message);
        assert!(verify_signature(&other, &message, &signature).is_err());
        // An old build presented as a newer version, or different bytes under the same version.
        assert!(verify_signature(&public, &signed_message("windows-x86_64", "9.9.9", "abc"), &signature).is_err());
        assert!(verify_signature(&public, &signed_message("windows-x86_64", "1.2.3", "def"), &signature).is_err());
        assert!(verify_signature(&public, &message, "not base64!").is_err());
    }

    #[test]
    fn the_built_in_key_is_well_formed() {
        let message = signed_message("windows-x86_64", "1.2.3", "abc");
        let err = verify_signature(PUBLIC_KEY, &message, &base64::engine::general_purpose::STANDARD.encode([0u8; 64])).unwrap_err();
        assert!(err.contains("isn't signed"), "{err}");
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("scout-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn swap_replaces_the_exe_and_keeps_the_old_one_aside() {
        let dir = temp_dir("swap");
        let (exe, new) = (dir.join("Scout.exe"), dir.join("update-1.2.3.exe"));
        std::fs::write(&exe, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        swap(&exe, &new).unwrap();
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "new");
        assert!(!new.exists());
        clean_folder(&exe, None);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn swap_puts_the_old_exe_back_when_the_new_one_is_missing() {
        let dir = temp_dir("rollback");
        let exe = dir.join("Scout.exe");
        std::fs::write(&exe, "old").unwrap();
        assert!(swap(&exe, &dir.join("missing.exe")).is_err());
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old");
    }

    #[test]
    fn clean_folder_keeps_the_exe_and_the_wanted_download() {
        let dir = temp_dir("clean");
        let exe = dir.join("Scout.exe");
        let keep = dir.join("update-2.0.0.exe");
        for name in ["Scout.exe", "install.ps1", "update-2.0.0.exe", "update-1.0.0.exe", "update-1.0.0.part", "Scout.exe.42.old"] {
            std::fs::write(dir.join(name), "x").unwrap();
        }
        clean_folder(&exe, Some(&keep));
        let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        left.sort();
        assert_eq!(left, ["Scout.exe", "install.ps1", "update-2.0.0.exe"]);
    }
}
