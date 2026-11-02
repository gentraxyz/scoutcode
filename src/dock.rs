//! What the right panel needs from the computer: the shell in a pseudo console, git's view of the project, and files.

use crate::diff::{self, DiffLine};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::Serialize;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use tauri::ipc::{Channel, InvokeResponseBody};

// ---------- terminal ----------

struct Session {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[derive(Clone, Default)]
pub struct Terminal(Arc<Mutex<Option<Session>>>);

/// PowerShell 7 when it's installed, else Windows PowerShell; $SHELL elsewhere. SCOUT_TERMINAL overrides both.
fn shell() -> CommandBuilder {
    if let Ok(s) = std::env::var("SCOUT_TERMINAL") {
        return CommandBuilder::new(s);
    }
    if cfg!(windows) {
        let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
        let pwsh = Path::new(&pf).join(r"PowerShell\7\pwsh.exe");
        let mut c = CommandBuilder::new(if pwsh.exists() { pwsh.to_string_lossy().to_string() } else { "powershell.exe".into() });
        c.arg("-NoLogo");
        c
    } else {
        CommandBuilder::new(std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into()))
    }
}

impl Terminal {
    /// Starts a shell, replacing any before it. Output arrives as raw bytes on `output`; `exited` fires when it ends.
    pub fn start(&self, cwd: &str, rows: u16, cols: u16, output: Channel<InvokeResponseBody>, exited: Channel<bool>) -> Result<String, String> {
        let pair = native_pty_system().openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }).map_err(|e| e.to_string())?;
        let mut cmd = shell();
        let name = Path::new(cmd.get_argv()[0].to_str().unwrap_or("shell")).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        cmd.cwd(cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        *self.0.lock().unwrap() = Some(Session { writer, master: pair.master, child });
        std::thread::Builder::new()
            .name("scout-terminal".into())
            .spawn(move || {
                let mut buf = [0u8; 16 * 1024];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 || output.send(InvokeResponseBody::Raw(buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
                let _ = exited.send(true);
            })
            .map_err(|e| e.to_string())?;
        Ok(name)
    }

    pub fn write(&self, data: &str) {
        if let Some(s) = &mut *self.0.lock().unwrap() {
            let _ = s.writer.write_all(data.as_bytes());
            let _ = s.writer.flush();
        }
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        if let Some(s) = &*self.0.lock().unwrap() {
            let _ = s.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        }
    }

    pub fn stop(&self) {
        *self.0.lock().unwrap() = None;
    }
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
