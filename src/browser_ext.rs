//! The Scout browser extension, for the Claude connector setup (claude_setup.rs): it works claude.ai and Gemini in the
//! user's own Chrome or Edge, with the user's own account.
//!
//! The extension reaches Scout through a small server on 127.0.0.1 that only answers requests with this PC's token.
//! Scout writes the extension's files to `~/.scoutcode/scout-extension`, with the port and token in `scout.json`, for
//! Load unpacked.

use crate::claude_setup;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// The extension's local server tries this port first.
const PORT: u16 = 47615;

const EXTENSION: [(&str, &str); 3] = [
    ("manifest.json", include_str!("browser_ext/manifest.json")),
    ("claude.js", include_str!("browser_ext/claude.js")),
    ("background.js", include_str!("browser_ext/background.js")),
];

static SERVING: Mutex<Option<u16>> = Mutex::new(None);
static TOKEN: OnceLock<String> = OnceLock::new();

fn token_file() -> PathBuf {
    crate::config::home().join("browser-extension.json")
}

/// This PC's token, kept across runs so a loaded extension stays valid. Made once if there isn't one.
fn token() -> &'static str {
    TOKEN.get_or_init(|| {
        let saved: Value = std::fs::read_to_string(token_file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
        if let Some(t) = saved.get("token").and_then(Value::as_str).filter(|t| t.len() >= 32) {
            return t.to_string();
        }
        let mut b = [0u8; 24];
        let _ = getrandom::fill(&mut b);
        let t: String = b.iter().map(|x| format!("{x:02x}")).collect();
        let _ = std::fs::write(token_file(), json!({ "token": t }).to_string());
        t
    })
}

fn extension_dir() -> PathBuf {
    crate::config::home().join("scout-extension")
}

pub fn extension_path() -> String {
    extension_dir().to_string_lossy().to_string()
}

/// Writes the extension's folder, with this PC's port and token, and starts the local server. Returns the manifest,
/// for Load unpacked.
pub fn extension_folder() -> Result<PathBuf, String> {
    serve();
    let dir = extension_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (name, body) in EXTENSION {
        std::fs::write(dir.join(name), body).map_err(|e| format!("Couldn't write the extension: {e}"))?;
    }
    let port = SERVING.lock().unwrap().unwrap_or(PORT);
    std::fs::write(dir.join("scout.json"), json!({ "port": port, "token": token() }).to_string()).map_err(|e| e.to_string())?;
    Ok(dir.join("manifest.json"))
}

/// Starts the local server, once. Only 127.0.0.1, and only for requests with the token.
fn serve() {
    let mut serving = SERVING.lock().unwrap();
    if serving.is_some() {
        return;
    }
    let Ok(listener) = TcpListener::bind(("127.0.0.1", PORT)).or_else(|_| TcpListener::bind(("127.0.0.1", 0))) else { return };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(PORT);
    *serving = Some(port);
    let _ = std::thread::Builder::new().name("browser-ext-bridge".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let _ = answer(stream);
            });
        }
    });
}

fn answer(mut stream: TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let (mut length, mut token_header, mut origin) = (0usize, String::new(), String::new());
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "content-length" => length = v.trim().parse().unwrap_or(0),
                "x-scout-token" => token_header = v.trim().to_string(),
                "origin" => origin = v.trim().to_string(),
                _ => {}
            }
        }
    }
    let reply = |stream: &mut TcpStream, status: &str, body: &str| {
        let head = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
        stream.write_all(head.as_bytes()).and_then(|_| stream.write_all(body.as_bytes()))
    };
    // Only the extension: web pages can't send the token header without a preflight, which is never answered, and the
    // extension's own requests come from chrome-extension:// or with no origin.
    let from_page = !origin.is_empty() && !origin.starts_with("chrome-extension://") && !origin.starts_with("extension://");
    if from_page || method != "POST" || path != "/claude/sync" || length > 8 << 20 || !same(&token_header, token()) {
        return reply(&mut stream, "403 Forbidden", "{}");
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let msg: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    reply(&mut stream, "200 OK", &claude_setup::sync(&msg).to_string())
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}
