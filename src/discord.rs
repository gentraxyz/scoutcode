//! Discord Rich Presence: shows "Using Scout" on the user's Discord profile while Scout is open.
//!
//! Off by default. Nothing is sent unless the user turns it on in Settings, and the folder name is only sent when they
//! turn that on as well. Paths, chats, prompts and file names are never sent.

use std::fs::File;
use std::io::{Read, Write};
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

/// The Discord application that Scout's presence belongs to. Empty until one is registered at
/// discord.com/developers, which keeps presence off without any other change.
const CLIENT_ID: &str = "";

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
    let mut conn: Option<File> = None;
    loop {
        match rx.recv_timeout(RETRY_EVERY) {
            Ok(activity) => last = activity,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let Some(activity) = last.clone() else {
            conn = None;
            continue;
        };
        if CLIENT_ID.is_empty() {
            continue;
        }
        if conn.is_none() {
            conn = connect();
        }
        if let Some(file) = conn.as_mut() {
            if set_activity(file, &activity).is_err() {
                conn = None;
            }
        }
    }
}

/// Opens the first Discord IPC pipe that answers the handshake.
fn connect() -> Option<File> {
    for i in 0..10 {
        let Ok(mut file) = std::fs::OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\discord-ipc-{i}")) else {
            continue;
        };
        let hello = serde_json::json!({ "v": 1, "client_id": CLIENT_ID });
        if send_frame(&mut file, 0, &hello).is_ok() && read_frame(&mut file).is_ok() {
            return Some(file);
        }
    }
    None
}

fn set_activity(file: &mut File, activity: &Activity) -> std::io::Result<()> {
    let mut body = serde_json::json!({ "details": "Using Scout", "assets": { "large_image": "scout", "large_text": "Scout" } });
    if let Some(state) = activity.state() {
        body["state"] = serde_json::json!(state);
    }
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let cmd = serde_json::json!({ "cmd": "SET_ACTIVITY", "args": { "pid": std::process::id(), "activity": body }, "nonce": nonce.to_string() });
    send_frame(file, 1, &cmd)?;
    read_frame(file).map(drop)
}

/// Discord's IPC frame: an opcode and a length, both little-endian u32, then the JSON body.
fn send_frame(file: &mut File, op: u32, body: &serde_json::Value) -> std::io::Result<()> {
    let data = body.to_string();
    let mut frame = Vec::with_capacity(8 + data.len());
    frame.extend_from_slice(&op.to_le_bytes());
    frame.extend_from_slice(&(data.len() as u32).to_le_bytes());
    frame.extend_from_slice(data.as_bytes());
    file.write_all(&frame)
}

fn read_frame(file: &mut File) -> std::io::Result<Vec<u8>> {
    let mut header = [0u8; 8];
    file.read_exact(&mut header)?;
    let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    let mut body = vec![0u8; len];
    file.read_exact(&mut body)?;
    Ok(body)
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
