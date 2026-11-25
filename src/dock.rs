//! What the right panel needs from the computer: the shell in a pseudo console, git's view of the project, and files.

use crate::diff::{self, DiffLine};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::Serialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::ipc::{Channel, InvokeResponseBody};

// ---------- terminal ----------

/// Where a shell's output goes: kept while it's a spare nobody sees yet, then streamed to the panel.
enum Sink {
    Held { bytes: Vec<u8>, exited: bool },
    Live { output: Channel<InvokeResponseBody>, exited: Channel<bool> },
}

struct Session {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    sink: Arc<Mutex<Sink>>,
    cwd: String,
    name: String,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

impl Session {
    fn exited(&self) -> bool {
        matches!(&*self.sink.lock().unwrap(), Sink::Held { exited: true, .. })
    }
}

#[derive(Default)]
struct Shells {
    /// One per terminal tab, by the tab's id.
    live: std::collections::HashMap<u32, Session>,
    /// Started ahead of time, so opening the panel (or Restart) shows a prompt at once instead of waiting most of a
    /// second for PowerShell to load.
    spare: Option<Session>,
}

#[derive(Clone, Default)]
pub struct Terminal(Arc<Mutex<Shells>>);

const INIT_PS1: &str = include_str!("terminal/init.ps1");
const INIT_SH: &str = include_str!("terminal/init.sh");

/// The panel's startup scripts (terminal/), written once per run to a folder of their own. None when that can't be
/// done: the shell then starts plain.
fn init_dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("scout-terminal-{}", env!("CARGO_PKG_VERSION")));
        std::fs::create_dir_all(&dir).ok()?;
        let sh = dir.join("init.sh").to_string_lossy().replace('\'', r"'\''");
        let files = [
            ("init.ps1", INIT_PS1.to_string()),
            ("init.sh", INIT_SH.to_string()),
            // bash reads --rcfile instead of ~/.bashrc, so this sources that first.
            ("bashrc", format!("[ -f ~/.bashrc ] && . ~/.bashrc\n. '{sh}'\n")),
            // zsh reads its startup files from $ZDOTDIR: these load the user's own (from their ZDOTDIR), then Scout's.
            (".zshenv", "__scout_zd=\"$ZDOTDIR\"; ZDOTDIR=\"${SCOUT_USER_ZDOTDIR:-$HOME}\"; [ -f \"$ZDOTDIR/.zshenv\" ] && . \"$ZDOTDIR/.zshenv\"; __scout_user_zd=\"$ZDOTDIR\"; ZDOTDIR=\"$__scout_zd\"\n".to_string()),
            (".zshrc", format!("ZDOTDIR=\"$__scout_user_zd\"; [ -f \"$ZDOTDIR/.zshrc\" ] && . \"$ZDOTDIR/.zshrc\"; . '{sh}'\n")),
        ];
        for (name, text) in files {
            let path = dir.join(name);
            if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
                std::fs::write(&path, text).ok()?;
            }
        }
        Some(dir)
    })
    .as_ref()
}

/// PowerShell 7 when it's installed, else Windows PowerShell; $SHELL elsewhere. SCOUT_TERMINAL overrides both, and
/// starts without Scout's additions.
fn shell() -> CommandBuilder {
    if let Ok(s) = std::env::var("SCOUT_TERMINAL") {
        return CommandBuilder::new(s);
    }
    let init = init_dir();
    if cfg!(windows) {
        let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
        let pwsh = Path::new(&pf).join(r"PowerShell\7\pwsh.exe");
        let mut c = CommandBuilder::new(if pwsh.exists() { pwsh.to_string_lossy().to_string() } else { "powershell.exe".into() });
        c.arg("-NoLogo");
        // Startup work nobody sees in a panel.
        c.env("POWERSHELL_UPDATECHECK", "Off");
        c.env("POWERSHELL_TELEMETRY_OPTOUT", "1");
        if let Some(dir) = init {
            let script = dir.join("init.ps1").to_string_lossy().replace('\'', "''");
            c.args(["-NoExit", "-Command", &format!(". '{script}'")]);
        }
        c
    } else {
        let sh = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
        let kind = Path::new(&sh).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let mut c = CommandBuilder::new(&sh);
        if let Some(dir) = init {
            if kind == "bash" {
                c.args(["--rcfile".to_string(), dir.join("bashrc").to_string_lossy().to_string(), "-i".to_string()]);
            } else if kind == "zsh" {
                if let Ok(z) = std::env::var("ZDOTDIR") {
                    c.env("SCOUT_USER_ZDOTDIR", z);
                }
                c.env("ZDOTDIR", dir);
            }
        }
        c
    }
}

fn spawn(cwd: &str, rows: u16, cols: u16) -> Result<Session, String> {
    let pair = native_pty_system().openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }).map_err(|e| e.to_string())?;
    let mut cmd = shell();
    let name = Path::new(cmd.get_argv()[0].to_str().unwrap_or("shell")).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    cmd.cwd(cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    if let Some(pid) = child.process_id() {
        crate::tools::contain(pid);
    }
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
    let sink = Arc::new(Mutex::new(Sink::Held { bytes: Vec::new(), exited: false }));
    let to = sink.clone();
    std::thread::Builder::new()
        .name("scout-terminal".into())
        .spawn(move || {
            let mut buf = [0u8; 64 * 1024];
            loop {
                let n = reader.read(&mut buf).unwrap_or(0);
                match &mut *to.lock().unwrap() {
                    Sink::Held { bytes, exited } => {
                        if n == 0 {
                            *exited = true;
                            return;
                        }
                        // A spare that's never shown shouldn't grow without end.
                        if bytes.len() < 4 << 20 {
                            bytes.extend_from_slice(&buf[..n]);
                        }
                    }
                    Sink::Live { output, exited } => {
                        if n == 0 || output.send(InvokeResponseBody::Raw(buf[..n].to_vec())).is_err() {
                            let _ = exited.send(true);
                            return;
                        }
                    }
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(Session { writer, master: pair.master, child, sink, cwd: cwd.to_string(), name })
}

impl Terminal {
    /// Starts a spare shell in `cwd` for the panel to take later, unless one is already waiting there.
    pub fn prewarm(&self, cwd: &str, rows: u16, cols: u16) {
        let ready = |s: &Option<Session>| s.as_ref().is_some_and(|s| s.cwd == cwd && !s.exited());
        if cwd.is_empty() || ready(&self.0.lock().unwrap().spare) {
            return;
        }
        let me = self.clone();
        let cwd = cwd.to_string();
        // Off the IPC thread: starting a process is the slow part.
        std::thread::spawn(move || {
            if let Ok(s) = spawn(&cwd, rows.max(2), cols.max(20)) {
                let mut shells = me.0.lock().unwrap();
                if !shells.spare.as_ref().is_some_and(|old| old.cwd == cwd && !old.exited()) {
                    shells.spare = Some(s);
                }
            }
        });
    }

    /// Shows a shell, replacing any before it: the spare when it's for this folder, else a new one. Output arrives as
    /// raw bytes on `output`; `exited` fires when it ends.
    pub fn start(&self, id: u32, cwd: &str, rows: u16, cols: u16, output: Channel<InvokeResponseBody>, exited: Channel<bool>) -> Result<String, String> {
        let (old, spare) = {
            let mut shells = self.0.lock().unwrap();
            (shells.live.remove(&id), shells.spare.take().filter(|s| s.cwd == cwd && !s.exited()))
        };
        drop(old);
        let session = match spare {
            Some(s) => {
                let _ = s.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
                s
            }
            None => spawn(cwd, rows, cols)?,
        };
        {
            // Under the lock, so nothing the shell prints falls between what was held and what streams.
            let mut sink = session.sink.lock().unwrap();
            if let Sink::Held { bytes, exited: done } = &mut *sink {
                if !bytes.is_empty() {
                    let _ = output.send(InvokeResponseBody::Raw(std::mem::take(bytes)));
                }
                if *done {
                    let _ = exited.send(true);
                }
            }
            *sink = Sink::Live { output, exited };
        }
        let name = session.name.clone();
        self.0.lock().unwrap().live.insert(id, session);
        Ok(name)
    }

    pub fn write(&self, id: u32, data: &str) {
        if let Some(s) = self.0.lock().unwrap().live.get_mut(&id) {
            let _ = s.writer.write_all(data.as_bytes());
            let _ = s.writer.flush();
        }
    }

    pub fn resize(&self, id: u32, rows: u16, cols: u16) {
        if let Some(s) = self.0.lock().unwrap().live.get(&id) {
            let _ = s.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        }
    }

    pub fn stop(&self, id: u32) {
        // Dropped outside the lock: killing a shell can take a moment.
        let gone = self.0.lock().unwrap().live.remove(&id);
        drop(gone);
    }
}

/// What the terminal's suggestions start from: the shell's own history (newest last) and the project's scripts.
#[derive(Serialize)]
pub struct Suggestions {
    history: Vec<String>,
    project: Vec<String>,
}

/// The shell's history file, as the panel's shell would write it.
fn history_file() -> Option<PathBuf> {
    if cfg!(windows) {
        // PSReadLine's, shared by Windows PowerShell and PowerShell 7.
        return Some(PathBuf::from(std::env::var("APPDATA").ok()?).join(r"Microsoft\Windows\PowerShell\PSReadLine\ConsoleHost_history.txt"));
    }
    if let Ok(f) = std::env::var("HISTFILE") {
        return Some(PathBuf::from(f));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let zsh = std::env::var("SHELL").unwrap_or_default().ends_with("zsh");
    Some(home.join(if zsh { ".zsh_history" } else { ".bash_history" }))
}

pub fn suggestions(cwd: &str) -> Suggestions {
    const KEEP: usize = 2000;
    let mut history = Vec::new();
    if let Some(text) = history_file().and_then(|f| std::fs::read(f).ok()) {
        let text = String::from_utf8_lossy(&text);
        let mut seen = std::collections::HashSet::new();
        // Newest first while de-duplicating, then flipped back.
        for line in text.lines().rev() {
            // zsh's extended format: ": 1700000000:0;command".
            let line = match line.strip_prefix(": ").and_then(|l| l.split_once(';')) {
                Some((_, cmd)) => cmd,
                None => line,
            };
            let line = line.trim();
            // PowerShell continues multi-line commands with a trailing backtick; those aren't one-liners.
            if line.is_empty() || line.len() > 300 || line.ends_with('`') || line.ends_with('\\') {
                continue;
            }
            if seen.insert(line.to_string()) {
                history.push(line.to_string());
                if history.len() >= KEEP {
                    break;
                }
            }
        }
        history.reverse();
    }

    let mut project = Vec::new();
    let dir = Path::new(cwd);
    if let Some(pkg) = std::fs::read_to_string(dir.join("package.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
        let runner = if dir.join("pnpm-lock.yaml").exists() {
            "pnpm"
        } else if dir.join("yarn.lock").exists() {
            "yarn"
        } else if dir.join("bun.lockb").exists() || dir.join("bun.lock").exists() {
            "bun"
        } else {
            "npm"
        };
        if let Some(scripts) = pkg.get("scripts").and_then(|s| s.as_object()) {
            for name in scripts.keys() {
                project.push(format!("{runner} run {name}"));
            }
        }
        project.push(format!("{runner} install"));
    }
    if dir.join("Cargo.toml").exists() {
        project.extend(["cargo build", "cargo run", "cargo test", "cargo check", "cargo clippy"].map(String::from));
    }
    if dir.join(".git").exists() {
        project.extend(["git status", "git diff", "git log --oneline", "git pull", "git push", "git add -A", "git commit -m \"\""].map(String::from));
    }
    Suggestions { history, project }
}

/// A folder's entries for path suggestions, as (name, is a folder). `~` at the start is the home folder.
pub fn list(dir: &str) -> Vec<(String, bool)> {
    let path = match dir.strip_prefix('~') {
        Some(rest) => PathBuf::from(std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).unwrap_or_default()).join(rest.trim_start_matches(['/', '\\'])),
        None => PathBuf::from(dir),
    };
    let Ok(entries) = std::fs::read_dir(path) else { return Vec::new() };
    let mut out: Vec<(String, bool)> = entries
        .flatten()
        .take(5000)
        .map(|e| (e.file_name().to_string_lossy().to_string(), e.file_type().map(|t| t.is_dir() || (t.is_symlink() && e.path().is_dir())).unwrap_or(false)))
        .collect();
    // Folders first, then by name: the first match is what's suggested.
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    out
}

// ---------- changes ----------

/// Diffs of files bigger than this aren't worked out.
const MAX_DIFF_BYTES: u64 = 2_000_000;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    /// Relative to the repository root, with forward slashes.
    path: String,
    /// Where a renamed file came from.
    from: Option<String>,
    status: char,
    counts: Option<(usize, usize)>,
}

#[derive(Serialize)]
pub struct Changes {
    repo: String,
    files: Vec<Change>,
}

/// Runs git and returns its raw output (prompt::git trims, which would eat porcelain's leading spaces).
fn git(cwd: &str, args: &[&str]) -> Option<Vec<u8>> {
    let mut c = std::process::Command::new("git");
    c.args(args).current_dir(cwd);
    crate::tools::no_window(&mut c);
    let out = c.output().ok()?;
    out.status.success().then_some(out.stdout)
}

pub fn changes(cwd: &str) -> Result<Changes, String> {
    let top = git(cwd, &["rev-parse", "--show-toplevel"]).map(|o| String::from_utf8_lossy(&o).trim().to_string()).ok_or("This folder isn't in a git repository.")?;
    let status = git(&top, &["status", "--porcelain=v1", "-z", "--untracked-files=all"]).ok_or("git status failed.")?;
    let status = String::from_utf8_lossy(&status).to_string();
    let mut files = vec![];
    let mut parts = status.split('\0').filter(|p| !p.is_empty());
    while let Some(e) = parts.next() {
        if e.len() < 4 {
            continue;
        }
        let (x, y) = (e.as_bytes()[0] as char, e.as_bytes()[1] as char);
        let from = if matches!(x, 'R' | 'C') { parts.next().map(String::from) } else { None };
        let status = match (x, y) {
            ('?', _) => 'U',
            ('R', _) => 'R',
            ('A', _) => 'A',
            (_, 'D') | ('D', _) => 'D',
            _ => 'M',
        };
        // Scout's own memory of the project isn't one of the user's changes.
        if e[3..].eq_ignore_ascii_case(".scout/memory.md") {
            continue;
        }
        files.push(Change { path: e[3..].to_string(), from, status, counts: None });
    }
    // Line counts for tracked files in one go; renames show as "a => b", which simply won't match.
    if let Some(ns) = git(&top, &["diff", "HEAD", "--numstat"]) {
        for line in String::from_utf8_lossy(&ns).lines() {
            let mut it = line.splitn(3, '\t');
            let (Some(a), Some(d), Some(p)) = (it.next(), it.next(), it.next()) else { continue };
            if let (Ok(a), Ok(d), Some(f)) = (a.parse(), d.parse(), files.iter_mut().find(|f| f.path == p)) {
                f.counts = Some((a, d));
            }
        }
    }
    for f in files.iter_mut().filter(|f| f.status == 'U') {
        let p = Path::new(&top).join(&f.path);
        if std::fs::metadata(&p).is_ok_and(|m| m.len() < MAX_DIFF_BYTES) {
            if let Ok(t) = std::fs::read_to_string(&p) {
                f.counts = Some((t.lines().count(), 0));
            }
        }
    }
    files.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));
    Ok(Changes { repo: top, files })
}

fn text_of(bytes: Vec<u8>) -> Result<String, String> {
    if bytes.len() as u64 > MAX_DIFF_BYTES {
        return Err("Too large to show.".into());
    }
    if bytes.iter().take(8000).any(|b| *b == 0) {
        return Err("Binary file.".into());
    }
    Ok(String::from_utf8_lossy(&bytes).replace("\r\n", "\n"))
}

/// The file at the last commit against the one on disk: its lines, and whether it's new.
pub fn file_diff(repo: &str, path: &str, from: Option<&str>, status: char) -> Result<(Vec<DiffLine>, bool), String> {
    let created = matches!(status, 'U' | 'A');
    let before = if created { String::new() } else { text_of(git(repo, &["show", &format!("HEAD:{}", from.unwrap_or(path))]).unwrap_or_default())? };
    let after = if status == 'D' { String::new() } else { text_of(std::fs::read(Path::new(repo).join(path)).map_err(|e| e.to_string())?)? };
    // The final newline would otherwise show up as an extra empty line.
    let trim = |s: &str| s.strip_suffix('\n').unwrap_or(s).to_string();
    Ok((diff::diff_lines(&trim(&before), &trim(&after)), created))
}

// ---------- files ----------

const MAX_EDIT_BYTES: u64 = 5_000_000;
/// Folders that are never worth browsing.
const HIDDEN: [&str; 1] = [".git"];

#[derive(Serialize)]
pub struct Node {
    name: String,
    path: String,
    dir: bool,
}

pub fn read_dir(dir: &str) -> Vec<Node> {
    let mut out: Vec<Node> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    if HIDDEN.contains(&name.as_str()) {
                        return None;
                    }
                    let dir = e.file_type().ok()?.is_dir();
                    Some(Node { name, path: e.path().to_string_lossy().to_string(), dir })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

/// A file for the editor. `blocked` says why it can't be edited here: binary, too big, unreadable.
#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OpenFile {
    text: String,
    crlf: bool,
    mtime: Option<u64>,
    blocked: Option<String>,
}

pub fn mtime(path: &str) -> Option<u64> {
    let t = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_millis() as u64)
}

pub fn read_file(path: &str) -> OpenFile {
    let mut f = OpenFile { mtime: mtime(path), ..Default::default() };
    match std::fs::metadata(path) {
        Ok(m) if m.len() > MAX_EDIT_BYTES => {
            f.blocked = Some(format!("Too large to edit here ({:.1} MB).", m.len() as f64 / 1e6));
            return f;
        }
        Err(e) => {
            f.blocked = Some(e.to_string());
            return f;
        }
        _ => {}
    }
    match std::fs::read(path) {
        Ok(b) if b.iter().take(8000).any(|c| *c == 0) => f.blocked = Some("Binary file.".into()),
        Ok(b) => match String::from_utf8(b) {
            Ok(t) => {
                f.crlf = t.contains("\r\n");
                f.text = if f.crlf { t.replace("\r\n", "\n") } else { t };
            }
            Err(_) => f.blocked = Some("Not UTF-8 text.".into()),
        },
        Err(e) => f.blocked = Some(e.to_string()),
    }
    f
}

/// Saves keeping the file's line endings, and returns its new modified time.
pub fn save_file(path: &str, text: &str, crlf: bool) -> Result<Option<u64>, String> {
    let out = if crlf { text.replace('\n', "\r\n") } else { text.to_string() };
    std::fs::write(path, out).map_err(|e| format!("Could not save: {e}"))?;
    Ok(mtime(path))
}

// ---------- Mission Control: quick open and search ----------

/// Every file under `root` (skipping node_modules, .git, build output and the like), as paths relative to it with `/`,
/// for Mission Control's quick open. Stops at `max`.
pub fn list_files(root: &str, max: usize) -> Vec<String> {
    let base = Path::new(root);
    let never = std::sync::atomic::AtomicBool::new(false);
    let mut out = Vec::new();
    crate::tools::walk(base, &never, |p| {
        if let Ok(r) = p.strip_prefix(base) {
            out.push(r.to_string_lossy().replace('\\', "/"));
        }
        out.len() < max
    });
    out.sort_unstable();
    out
}

/// One line that matched a search: the file (relative, with `/`), its 1-based line, and the line's text.
#[derive(Serialize)]
pub struct Hit {
    path: String,
    line: usize,
    text: String,
}

/// The lines under `root` that contain `query` (plain text, or a regular expression with `regex`), at most `max`.
pub fn search(root: &str, query: &str, case: bool, is_regex: bool, max: usize) -> Result<Vec<Hit>, String> {
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let pattern = if is_regex { query.to_string() } else { regex::escape(query) };
    let re = regex::RegexBuilder::new(&pattern).case_insensitive(!case).build().map_err(|e| format!("Invalid regular expression: {e}"))?;
    let base = Path::new(root);
    let never = std::sync::atomic::AtomicBool::new(false);
    let mut out = Vec::new();
    crate::tools::walk(base, &never, |p| {
        if std::fs::metadata(p).is_ok_and(|m| m.len() > 2 * 1024 * 1024) {
            return true;
        }
        let Ok(buf) = std::fs::read(p) else { return true };
        if crate::tools::is_binary(&buf) {
            return true;
        }
        let rel = p.strip_prefix(base).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
        for (i, l) in String::from_utf8_lossy(&buf).lines().enumerate() {
            if re.is_match(l) {
                let text: String = l.trim().chars().take(240).collect();
                out.push(Hit { path: rel.clone(), line: i + 1, text });
                if out.len() >= max {
                    return false;
                }
            }
        }
        true
    });
    Ok(out)
}
