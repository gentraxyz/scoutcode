//! Sign in through the browser instead of pasting an API key (OAuth with PKCE, on a loopback port), the same flow as
//! `scout login`. The token spends the account's CLI credit and can only use the models admins approved for the CLI.

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
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>{title}</title><body style=\"font-family:system-ui;max-width:32rem;margin:4rem auto;padding:0 1rem\"><h2>{title}</h2><p>{text}</p></body>"
    );
    format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
}

/// Opens the browser at the sign-in page and waits for the user to approve. `on_url` gets the address in case the browser didn't open.
/// Returns the token and the username.
pub fn browser_login(cfg: &Config, on_url: impl FnOnce(&str)) -> Result<(String, Option<String>), String> {
    let verifier = random(32);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random(16);
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}/callback");
    let host = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_default();
    let site = cfg.base_url.trim_end_matches("/v1");
    let url = format!(
        "{site}/oauth/authorize?response_type=code&client_id={CLIENT_ID}&redirect_uri={}&code_challenge={challenge}&code_challenge_method=S256&state={state}&device={}",
        encode(&redirect),
        encode(&host.chars().take(40).collect::<String>())
    );
    on_url(&url);
    open_browser(&url);

    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(300);
    let code = loop {
        if Instant::now() > deadline {
            return Err("Timed out waiting for the browser sign-in.".into());
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
        if get("state").as_deref() != Some(state.as_str()) {
            return fail(stream, "The sign-in answer did not match this request. Try again.");
        }
        if get("error").is_some() {
            return fail(stream, "Sign-in was cancelled in the browser.");
        }
        let Some(c) = get("code") else { return fail(stream, "The browser sent no code.") };
        let _ = stream.write_all(page("Signed in", "You can close this tab and go back to Scout.").as_bytes());
        break c;
    };

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
