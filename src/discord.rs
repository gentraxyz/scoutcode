//! Discord Rich Presence: shows "Using Scout" on the user's Discord profile while Scout is open.
//!
//! On by default; can be turned off in Settings. The folder name is only sent when the user turns that on as well.
//! Paths, chats, prompts and file names are never sent.

use std::io::{Read, Write};
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

/// The Discord application that Scout's presence belongs to (discord.com/developers). Its Rich Presence art asset
/// named "scout" is the profile's image.
const CLIENT_ID: &str = "1567289830969442344";

/// Discord's frames are small; anything bigger means the stream is out of step.
const MAX_FRAME: usize = 64 * 1024;

/// A connection to Discord: a named pipe on Windows, a Unix socket elsewhere.
trait Pipe: Read + Write + Send {}
impl<T: Read + Write + Send> Pipe for T {}
type Conn = Box<dyn Pipe>;

/// How often a missing connection to Discord is retried, so presence appears once Discord starts after Scout.
const RETRY_EVERY: Duration = Duration::from_secs(30);

/// What the profile shows. `project` is a folder's name, never its path, and is None unless the user allowed it.
#[derive(Clone, Debug, PartialEq)]
pub struct Activity {
    pub project: Option<String>,
}

impl Activity {
    fn state(&self) -> Option<String> {
        self.project.as_ref().map(|name| format!("Working on {name}"))
    }
}

static SENDER: OnceLock<Sender<Option<Activity>>> = OnceLock::new();

/// Sets what the profile shows, or clears it with None. Returns at once; Discord is reached on a background thread.
pub fn update(activity: Option<Activity>) {
    let tx = SENDER.get_or_init(|| {
        let (tx, rx) = channel();
        std::thread::Builder::new().name("scout-discord".into()).spawn(move || run(rx)).ok();
        tx
    });
    let _ = tx.send(activity);
}

fn run(rx: Receiver<Option<Activity>>) {
    let mut last: Option<Activity> = None;
    let mut conn: Option<Conn> = None;
    loop {
        match rx.recv_timeout(RETRY_EVERY) {
            Ok(activity) => last = activity,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        // Closing the connection is what clears the profile.
        let Some(activity) = last.clone() else {
            conn = None;
            continue;
        };
        if conn.is_none() {
            conn = connect();
        }
        if let Some(pipe) = conn.as_mut() {
            if set_activity(pipe, &activity).is_err() {
                conn = None;
            }
        }
    }
}

/// The places Discord listens: discord-ipc-0 to 9, under each folder the desktop, Flatpak and Snap builds use.
fn endpoints() -> Vec<String> {
    #[cfg(windows)]
    let dirs = vec![r"\\.\pipe".to_string()];
    #[cfg(not(windows))]
    let dirs = {
        let mut base: Vec<String> = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"].iter().filter_map(|v| std::env::var(v).ok()).collect();
        base.push("/tmp".into());
        let mut dirs = Vec::new();
        for b in base {
            let b = b.trim_end_matches('/').to_string();
            for sub in ["", "/app/com.discordapp.Discord", "/snap.discord", "/.flatpak/dev.vencord.Vesktop/xdg-run"] {
                let dir = format!("{b}{sub}");
                if !dirs.contains(&dir) {
                    dirs.push(dir);
                }
            }
        }
        dirs
    };
    dirs.iter().flat_map(|d| (0..10).map(move |i| format!("{d}/discord-ipc-{i}"))).collect()
}

fn open(path: &str) -> Option<Conn> {
    #[cfg(windows)]
    return std::fs::OpenOptions::new().read(true).write(true).open(path).ok().map(|f| Box::new(f) as Conn);
    #[cfg(not(windows))]
    return std::os::unix::net::UnixStream::connect(path).ok().map(|s| {
        let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
        Box::new(s) as Conn
    });
}

/// Opens the first Discord connection that accepts Scout's handshake with a READY.
fn connect() -> Option<Conn> {
    for path in endpoints() {
        let Some(mut pipe) = open(&path) else { continue };
        let hello = serde_json::json!({ "v": 1, "client_id": CLIENT_ID });
        if send_frame(&mut pipe, 0, &hello).is_err() {
            continue;
        }
        if matches!(read_reply(&mut pipe, None), Ok(reply) if reply["evt"] == "READY") {
            return Some(pipe);
        }
    }
    None
}

fn set_activity(pipe: &mut Conn, activity: &Activity) -> std::io::Result<()> {
    let mut body = serde_json::json!({ "details": "Using Scout", "assets": { "large_image": "scout", "large_text": "Scout" } });
    if let Some(state) = activity.state() {
        body["state"] = serde_json::json!(state);
    }
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0).to_string();
    let cmd = serde_json::json!({ "cmd": "SET_ACTIVITY", "args": { "pid": std::process::id(), "activity": body }, "nonce": nonce });
    send_frame(pipe, 1, &cmd)?;
    let reply = read_reply(pipe, Some(&nonce))?;
    if reply["evt"] == "ERROR" {
        return Err(std::io::Error::other(reply["data"]["message"].as_str().unwrap_or("Discord refused the activity").to_string()));
    }
    Ok(())
}

/// Reads frames until the reply: answering pings, failing on close, and skipping events that aren't for `nonce`.
fn read_reply(pipe: &mut Conn, nonce: Option<&str>) -> std::io::Result<serde_json::Value> {
    loop {
        let (op, body) = read_frame(pipe)?;
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
        match op {
            1 if nonce.is_none_or(|n| value["nonce"] == n) => return Ok(value),
            2 => return Err(std::io::Error::other(value["message"].as_str().unwrap_or("Discord closed the connection").to_string())),
            3 => send_frame(pipe, 4, &value)?,
            _ => {}
        }
    }
}

/// Discord's IPC frame: an opcode and a length, both little-endian u32, then the JSON body.
fn send_frame(pipe: &mut Conn, op: u32, body: &serde_json::Value) -> std::io::Result<()> {
    let data = body.to_string();
    let mut frame = Vec::with_capacity(8 + data.len());
    frame.extend_from_slice(&op.to_le_bytes());
    frame.extend_from_slice(&(data.len() as u32).to_le_bytes());
    frame.extend_from_slice(data.as_bytes());
    pipe.write_all(&frame)?;
    pipe.flush()
}

fn read_frame(pipe: &mut Conn) -> std::io::Result<(u32, Vec<u8>)> {
    let mut header = [0u8; 8];
    pipe.read_exact(&mut header)?;
    let op = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::other("Discord sent an oversized frame"));
    }
    let mut body = vec![0u8; len];
    pipe.read_exact(&mut body)?;
    Ok((op, body))
}

/// The folder's name for the profile, or None when the path has no usable name.
pub fn project_name(cwd: &str) -> Option<String> {
    std::path::Path::new(cwd).file_name().map(|n| n.to_string_lossy().to_string()).filter(|n| !n.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_shows_no_folder_unless_one_is_given() {
        assert_eq!(Activity { project: None }.state(), None);
        assert_eq!(Activity { project: Some("scout".into()) }.state().as_deref(), Some("Working on scout"));
    }

    #[test]
    fn a_project_name_is_the_folder_not_its_path() {
        assert_eq!(project_name(r"C:\Users\someone\work\secret-app").as_deref(), Some("secret-app"));
        assert_eq!(project_name(r"C:\"), None);
    }
}
