//! Sign in through the browser instead of pasting an API key (OAuth with PKCE, on a loopback port).
//! The token spends the account's desktop credit and can only use the models admins approved for the desktop app.
//! The loopback and PKCE parts are shared with signing in to MCP servers (see mcp_auth.rs).

use crate::config::Config;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

const CLIENT_ID: &str = "scoutcode-cli";

fn random(n: usize) -> String {
    let mut b = vec![0u8; n];
    let _ = getrandom::fill(&mut b);
    URL_SAFE_NO_PAD.encode(b)
}

pub fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(v);
                    i += 2;
                } else {
                    out.push(b'%');
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn open_browser(url: &str) {
    let mut c = if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    crate::tools::no_window(&mut c);
    let _ = c.spawn();
}

fn page(title: &str, text: &str) -> String {
    // The site's dark palette (see the sign-in pages on scoutcode), so this tab matches the page that came before it.
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{title} · Scout</title>\
        <style>:root{{color-scheme:dark}}html,body{{margin:0;min-height:100vh;background:#14120b;color:#edecec;font:14px/1.5 system-ui,-apple-system,'Segoe UI',Roboto,sans-serif;-webkit-font-smoothing:antialiased}}\
        main{{min-height:100vh;display:grid;place-items:center;padding:24px}}\
        .card{{box-sizing:border-box;width:100%;max-width:440px;padding:32px 30px 28px;border:1px solid rgba(237,236,236,.1);border-radius:14px;background:linear-gradient(180deg,#1d1b14 0%,#1b1913 100%);box-shadow:0 24px 60px rgba(0,0,0,.45)}}\
        .mark{{width:40px;height:40px;border-radius:10px;background:#edecec;color:#14120b;display:grid;place-items:center;margin-bottom:18px}}\
        .mark svg{{width:24px;height:24px}}\
        h2{{margin:0 0 .6rem;font-size:1.3rem;font-weight:500;letter-spacing:-.02em}}\
        p{{margin:0;color:#a09e97}}</style>\
        <main><div class=\"card\"><div class=\"mark\" aria-hidden=\"true\"><svg viewBox=\"0 0 32 32\"><path d=\"M18.65 4.84V8.79L10.74 13.32 16.03 16.57V20.21L7.48 15.07V11.37ZM13.35 27.16V23.21L21.26 18.68 15.97 15.43V11.79L24.52 16.93V20.63Z\" fill=\"currentColor\" stroke=\"currentColor\" stroke-width=\"1\" stroke-linejoin=\"round\"/></svg></div>\
        <h2>{title}</h2><p>{text}</p></div></main>"
    );
    format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
}

/// A PKCE verifier and its S256 challenge.
pub fn pkce() -> (String, String) {
    let verifier = random(32);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

pub fn new_state() -> String {
    random(16)
}

/// A loopback port for the browser to come back to, and the redirect address on it. `preferred` is tried first, so a
/// client registered for that exact address can be used again.
pub fn loopback(preferred: Option<u16>) -> Result<(TcpListener, String), String> {
    let listener = match preferred.and_then(|p| TcpListener::bind(("127.0.0.1", p)).ok()) {
        Some(l) => l,
        None => TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?,
    };
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    Ok((listener, format!("http://127.0.0.1:{port}/callback")))
}

/// Waits up to five minutes for the browser to come back to `/callback` with a code for `state`, and tells the tab how
/// that went. `cancelled` stops it early.
pub fn wait_for_code(listener: &TcpListener, state: &str, cancelled: &dyn Fn() -> bool) -> Result<String, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        if Instant::now() > deadline {
            return Err("Timed out waiting for the browser sign-in.".into());
        }
        if cancelled() {
            return Err("Sign-in was cancelled.".into());
        }
        let (mut stream, _) = match listener.accept() {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut line = String::new();
        let _ = BufReader::new(&stream).read_line(&mut line);
        let target = line.split_whitespace().nth(1).unwrap_or("/");
        let Some(query) = target.strip_prefix("/callback?").or_else(|| (target == "/callback").then_some("")) else {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            continue;
        };
        let get = |k: &str| query.split('&').filter_map(|p| p.split_once('=')).find(|(n, _)| *n == k).map(|(_, v)| decode(v));
        let fail = |mut stream: std::net::TcpStream, msg: &str| {
            let _ = stream.write_all(page("Not signed in", msg).as_bytes());
            Err(msg.to_string())
        };
        if get("state").as_deref() != Some(state) {
            return fail(stream, "The sign-in answer did not match this request. Try again.");
        }
        if get("error").is_some() {
            return fail(stream, "Sign-in was cancelled in the browser.");
        }
        let Some(c) = get("code") else { return fail(stream, "The browser sent no code.") };
        let _ = stream.write_all(page("Signed in", "You can close this tab and go back to Scout.").as_bytes());
        return Ok(c);
    }
}

/// Opens the browser at the sign-in page and waits for the user to approve. `on_url` gets the address in case the browser didn't open.
/// Returns the token and the username.
pub fn browser_login(cfg: &Config, on_url: impl FnOnce(&str)) -> Result<(String, Option<String>), String> {
    let (verifier, challenge) = pkce();
    let state = new_state();
    let (listener, redirect) = loopback(None)?;
    let host = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_default();
    let site = cfg.base_url.trim_end_matches("/v1");
    let url = format!(
        "{site}/oauth/authorize?response_type=code&client_id={CLIENT_ID}&redirect_uri={}&code_challenge={challenge}&code_challenge_method=S256&state={state}&device={}",
        encode(&redirect),
        encode(&host.chars().take(40).collect::<String>())
    );
    on_url(&url);
    open_browser(&url);
    let code = wait_for_code(&listener, &state, &|| false)?;

    let form = format!(
        "grant_type=authorization_code&client_id={CLIENT_ID}&code={}&code_verifier={verifier}&redirect_uri={}",
        encode(&code),
        encode(&redirect)
    );
    let res = crate::api::http()
        .post(&format!("{}/oauth/token", cfg.base_url))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(form)
        .map_err(|e| e.to_string())?;
    let status = res.status().as_u16();
    let body: serde_json::Value = res.into_body().read_json().unwrap_or_default();
    match body.get("access_token").and_then(|t| t.as_str()) {
        Some(t) if status < 400 => Ok((t.to_string(), body.get("username").and_then(|u| u.as_str()).map(String::from))),
        _ => Err(body
            .get("error_description")
            .or_else(|| body.get("error"))
            .and_then(|e| e.as_str())
            .map(String::from)
            .unwrap_or_else(|| format!("Sign-in failed: HTTP {status}"))),
    }
}
