//! Signing in to a remote MCP server through the browser, as the MCP authorization spec has it: OAuth 2.1 with PKCE,
//! the server's protected-resource metadata pointing at its authorization server, and dynamic client registration, so
//! the user doesn't have to find a token and paste it under Headers.
//!
//! What a sign-in gets (the client Scout registered and its tokens) is kept per server, in Windows Credential Manager
//! where it fits and in ~/.scoutcode/mcp-auth.json where it doesn't, and only used for the URL it was made for.

use crate::api::http;
use crate::oauth::encode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use url::Url;

/// Refreshed this long before it runs out, so a call doesn't start with a token that dies halfway.
const MARGIN_MS: u64 = 60_000;
/// The loopback port Scout asks for first, so the client it registered for that address can sign in again.
const PORT: u16 = 33418;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Auth {
    /// The server's URL this was made for. A server whose URL changed signs in again.
    pub url: String,
    pub client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    /// The address the client was registered with.
    pub redirect: String,
    pub token_endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// ms since 1970, if the server said.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}

// ---------- keeping it ----------

fn file() -> PathBuf {
    crate::config::home().join("mcp-auth.json")
}

fn field(name: &str) -> String {
    format!("mcpAuth/{name}")
}

/// One lock for the file and the store, so a refresh and a sign-in don't write over each other.
fn lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn read_file() -> BTreeMap<String, Auth> {
    std::fs::read_to_string(file()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write_file(all: &BTreeMap<String, Auth>) {
    let path = file();
    if all.is_empty() {
        let _ = std::fs::remove_file(path);
        return;
    }
    if let Some(p) = path.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let _ = std::fs::write(path, serde_json::to_string_pretty(all).unwrap_or_default());
}

fn load(name: &str) -> Option<Auth> {
    let _g = lock();
    crate::secrets::get(&field(name)).and_then(|v| serde_json::from_value(v).ok()).or_else(|| read_file().remove(name))
}

fn store(name: &str, auth: &Auth) {
    let _g = lock();
    let mut all = read_file();
    if crate::secrets::set(&field(name), &serde_json::to_value(auth).unwrap_or_default()) {
        if all.remove(name).is_some() {
            write_file(&all);
        }
    } else {
        all.insert(name.to_string(), auth.clone());
        write_file(&all);
    }
}

/// Forgets a server's sign-in (signing out, or the server being removed).
pub fn forget(name: &str) {
    let _g = lock();
    crate::secrets::delete(&field(name));
    let mut all = read_file();
    if all.remove(name).is_some() {
        write_file(&all);
    }
}

/// The sign-in kept for this server at this URL.
pub fn saved(name: &str, url: &str) -> Option<Auth> {
    load(name).filter(|a| a.url == url && !a.access_token.is_empty())
}

pub fn signed_in(name: &str, url: &str) -> bool {
    saved(name, url).is_some()
}

/// A token to send, refreshed first if it's about to run out. None when there's no sign-in, or it can't be refreshed.
pub fn token(name: &str, url: &str) -> Option<String> {
    let a = saved(name, url)?;
    let now = crate::session::now_ms();
    match a.expires_at {
        Some(t) if t <= now + MARGIN_MS => refresh(name, url).ok(),
        _ => Some(a.access_token),
    }
}

/// Swaps the refresh token for a new access token. On failure the old tokens are dropped if the server refused them,
/// so the server shows as needing a sign-in.
pub fn refresh(name: &str, url: &str) -> Result<String, String> {
    let mut a = saved(name, url).ok_or("Not signed in.")?;
    let Some(rt) = a.refresh_token.clone() else { return Err("The sign-in ran out.".into()) };
    let mut form = format!("grant_type=refresh_token&refresh_token={}&client_id={}", encode(&rt), encode(&a.client_id));
    if let Some(r) = &a.resource {
        form.push_str(&format!("&resource={}", encode(r)));
    }
    if let Some(s) = &a.client_secret {
        form.push_str(&format!("&client_secret={}", encode(s)));
    }
    match post_token(&a.token_endpoint, form) {
        Ok(t) => {
            apply(&mut a, &t);
            store(name, &a);
            Ok(a.access_token)
        }
        Err((status, e)) => {
            if (400..500).contains(&status) {
                a.access_token.clear();
                a.refresh_token = None;
                store(name, &a);
            }
            Err(e)
        }
    }
}

fn apply(a: &mut Auth, t: &Value) {
    a.access_token = t["access_token"].as_str().unwrap_or("").to_string();
    if let Some(r) = t["refresh_token"].as_str() {
        a.refresh_token = Some(r.to_string());
    }
    a.expires_at = t["expires_in"].as_u64().or_else(|| t["expires_in"].as_str().and_then(|s| s.parse().ok())).map(|s| crate::session::now_ms() + s * 1000);
}

/// The token endpoint's answer, or its status and what went wrong.
fn post_token(endpoint: &str, form: String) -> Result<Value, (u16, String)> {
    let res = http()
        .post(endpoint)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .send(form)
        .map_err(|e| (0, format!("Can't reach the sign-in server: {e}")))?;
    let status = res.status().as_u16();
    let body: Value = res.into_body().read_json().unwrap_or_default();
    match body["access_token"].as_str() {
        Some(t) if status < 400 && !t.is_empty() => Ok(body),
        _ => Err((
            status,
            body["error_description"].as_str().or(body["error"].as_str()).map(String::from).unwrap_or_else(|| format!("The sign-in server answered HTTP {status}.")),
        )),
    }
}

// ---------- finding out how to sign in ----------

fn get_json(url: &str) -> Option<Value> {
    let res = http().get(url).header("Accept", "application/json").header("MCP-Protocol-Version", crate::mcp::PROTOCOLS[0]).call().ok()?;
    if res.status().as_u16() >= 400 {
        return None;
    }
    res.into_body().read_json::<Value>().ok().filter(Value::is_object)
}

/// A parameter of a WWW-Authenticate header, like `resource_metadata="https://..."`.
fn auth_param(header: &str, key: &str) -> Option<String> {
    let lower = header.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(key).map(|i| i + from) {
        let before_ok = i == 0 || matches!(lower.as_bytes()[i - 1], b' ' | b',' | b'\t');
        let rest = header[i + key.len()..].trim_start();
        if before_ok && let Some(v) = rest.strip_prefix('=') {
            let v = v.trim_start();
            return Some(if let Some(q) = v.strip_prefix('"') { q.split('"').next().unwrap_or("").to_string() } else { v.split([',', ' ']).next().unwrap_or("").to_string() });
        }
        from = i + key.len();
    }
    None
}

/// Asks the server, without a token, what it wants: its WWW-Authenticate header.
fn challenge(url: &str) -> Option<String> {
    let msg = json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": crate::mcp::PROTOCOLS[0], "capabilities": {}, "clientInfo": {"name": "scout", "version": crate::update::VERSION}}});
    let res = http().post(url).header("Accept", "application/json, text/event-stream").send_json(&msg).ok()?;
    res.headers().get("www-authenticate").and_then(|v| v.to_str().ok()).map(String::from)
}

/// `https://host/.well-known/<what><path>`, then without the path.
fn well_known(base: &Url, what: &str) -> Vec<String> {
    let origin = base.origin().ascii_serialization();
    let path = base.path().trim_end_matches('/');
    let mut out = vec![];
    if !path.is_empty() {
        out.push(format!("{origin}/.well-known/{what}{path}"));
    }
    out.push(format!("{origin}/.well-known/{what}"));
    out
}

struct Plan {
    authorize: String,
    token: String,
    register: Option<String>,
    scope: Option<String>,
    resource: Option<String>,
    pkce_ok: bool,
}

fn discover(server: &str) -> Result<Plan, String> {
    let server_url = Url::parse(server).map_err(|_| "The server's URL isn't valid.".to_string())?;
    let header = challenge(server).unwrap_or_default();
    let mut scope = auth_param(&header, "scope");

    // The protected resource's metadata names its authorization server.
    let mut prm_urls = vec![];
    if let Some(u) = auth_param(&header, "resource_metadata") {
        prm_urls.push(u);
    }
    prm_urls.extend(well_known(&server_url, "oauth-protected-resource"));
    let prm = prm_urls.iter().find_map(|u| get_json(u));
    let mut resource = Some(server.split(['#']).next().unwrap_or(server).to_string());
    let issuer = match &prm {
        Some(p) => {
            if let Some(r) = p["resource"].as_str() {
                resource = Some(r.to_string());
            }
            if scope.is_none() {
                scope = p["scopes_supported"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")).filter(|s| !s.is_empty());
            }
            p["authorization_servers"].as_array().and_then(|a| a.first()).and_then(Value::as_str).map(String::from)
        }
        None => None,
    }
    .unwrap_or_else(|| server_url.origin().ascii_serialization());

    let issuer_url = Url::parse(&issuer).map_err(|_| "The server named a sign-in address that isn't valid.".to_string())?;
    let mut meta_urls = well_known(&issuer_url, "oauth-authorization-server");
    meta_urls.extend(well_known(&issuer_url, "openid-configuration"));
    meta_urls.push(format!("{}/.well-known/openid-configuration", issuer.trim_end_matches('/')));
    let meta = meta_urls.iter().find_map(|u| get_json(u));

    let origin = issuer_url.origin().ascii_serialization();
    let plan = match meta {
        Some(m) => Plan {
            authorize: m["authorization_endpoint"].as_str().ok_or("The sign-in server doesn't say where to sign in.")?.to_string(),
            token: m["token_endpoint"].as_str().ok_or("The sign-in server doesn't say where to get a token.")?.to_string(),
            register: m["registration_endpoint"].as_str().map(String::from),
            scope,
            resource,
            pkce_ok: m["code_challenge_methods_supported"].as_array().is_none_or(|a| a.iter().any(|x| x == "S256")),
        },
        // Servers from before discovery was required keep the endpoints at fixed places.
        None if prm.is_none() && header.is_empty() => return Err("The server didn't ask for a sign-in. If it takes a token, add it under Headers.".into()),
        None => Plan { authorize: format!("{origin}/authorize"), token: format!("{origin}/token"), register: Some(format!("{origin}/register")), scope, resource, pkce_ok: true },
    };
    if !plan.pkce_ok {
        return Err("The server's sign-in doesn't support PKCE, which Scout needs. Add a token under Headers instead.".into());
    }
    Ok(plan)
}

fn register(endpoint: &str, redirect: &str) -> Result<(String, Option<String>), String> {
    let body = json!({
        "client_name": "Scout",
        "client_uri": "https://scoutcode.dev",
        "redirect_uris": [redirect],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    });
    let res = http().post(endpoint).header("Accept", "application/json").send_json(&body).map_err(|e| format!("Can't reach the sign-in server: {e}"))?;
    let status = res.status().as_u16();
    let v: Value = res.into_body().read_json().unwrap_or_default();
    match v["client_id"].as_str() {
        Some(id) if status < 400 => Ok((id.to_string(), v["client_secret"].as_str().filter(|s| !s.is_empty()).map(String::from))),
        _ => Err(v["error_description"].as_str().map(String::from).unwrap_or_else(|| format!("The server wouldn't register Scout to sign in (HTTP {status}). Add a token under Headers instead."))),
    }
}

/// Signs in to the server through the browser and keeps the tokens. `on_url` gets the address in case the browser
/// doesn't open; `cancelled` stops waiting.
pub fn sign_in(name: &str, server: &str, on_url: impl FnOnce(&str), cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    let plan = discover(server)?;
    let old = load(name).filter(|a| a.url == server && a.token_endpoint == plan.token && !a.client_id.is_empty());
    let preferred = old.as_ref().and_then(|a| Url::parse(&a.redirect).ok()).and_then(|u| u.port()).or(Some(PORT));
    let (listener, redirect) = crate::oauth::loopback(preferred)?;
    // A client registered for this exact address is used again; otherwise Scout registers itself.
    let (client_id, client_secret) = match old.filter(|a| a.redirect == redirect) {
        Some(a) => (a.client_id, a.client_secret),
        None => match &plan.register {
            Some(r) => register(r, &redirect)?,
            None => return Err("The server's sign-in doesn't let apps register themselves, so Scout can't sign in. Add a token under Headers instead.".into()),
        },
    };

    let (verifier, challenge) = crate::oauth::pkce();
    let state = crate::oauth::new_state();
    let mut url = format!(
        "{}{}response_type=code&client_id={}&redirect_uri={}&code_challenge={challenge}&code_challenge_method=S256&state={state}",
        plan.authorize,
        if plan.authorize.contains('?') { "&" } else { "?" },
        encode(&client_id),
        encode(&redirect)
    );
    if let Some(s) = &plan.scope {
        url.push_str(&format!("&scope={}", encode(s)));
    }
    if let Some(r) = &plan.resource {
        url.push_str(&format!("&resource={}", encode(r)));
    }
    on_url(&url);
    crate::oauth::open_browser(&url);
    let code = crate::oauth::wait_for_code(&listener, &state, cancelled)?;
    drop(listener);

    let mut form = format!("grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={verifier}", encode(&code), encode(&redirect), encode(&client_id));
    if let Some(r) = &plan.resource {
        form.push_str(&format!("&resource={}", encode(r)));
    }
    if let Some(s) = &client_secret {
        form.push_str(&format!("&client_secret={}", encode(s)));
    }
    let t = post_token(&plan.token, form).map_err(|(_, e)| e)?;
    let mut auth = Auth { url: server.to_string(), client_id, client_secret, redirect, token_endpoint: plan.token, resource: plan.resource, ..Default::default() };
    apply(&mut auth, &t);
    store(name, &auth);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_www_authenticate() {
        let h = r#"Bearer error="invalid_token", resource_metadata="https://x.dev/.well-known/oauth-protected-resource/mcp", scope="read write""#;
        assert_eq!(auth_param(h, "resource_metadata").as_deref(), Some("https://x.dev/.well-known/oauth-protected-resource/mcp"));
        assert_eq!(auth_param(h, "scope").as_deref(), Some("read write"));
        assert_eq!(auth_param(h, "realm"), None);
        assert_eq!(auth_param("Bearer realm=x, scope=a", "scope").as_deref(), Some("a"));
    }

    /// A server that wants a sign-in: 401 with resource metadata, an authorization server, and a token endpoint that
    /// takes a refresh token. Answers every request with what its path asks for.
    fn fake_server() -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let b = base.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut s = stream.unwrap();
                let (mut all, mut buf) = (vec![], [0u8; 8192]);
                let req = loop {
                    let n = s.read(&mut buf).unwrap_or(0);
                    all.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&all).to_string();
                    let Some(i) = text.find("\r\n\r\n") else {
                        if n == 0 { break text } else { continue }
                    };
                    let len: usize = text[..i].to_lowercase().lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                    if n == 0 || text.len() >= i + 4 + len {
                        break text;
                    }
                };
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let (status, extra, body) = match path.as_str() {
                    "/mcp" if req.to_lowercase().contains("authorization: bearer good") => ("200 OK", String::new(), json!({"jsonrpc": "2.0", "id": 1, "result": {}}).to_string()),
                    "/mcp" => ("401 Unauthorized", format!("WWW-Authenticate: Bearer resource_metadata=\"{b}/.well-known/oauth-protected-resource/mcp\", scope=\"tools\"\r\n"), String::new()),
                    "/.well-known/oauth-protected-resource/mcp" => ("200 OK", String::new(), json!({"resource": format!("{b}/mcp"), "authorization_servers": [format!("{b}/auth")]}).to_string()),
                    "/.well-known/oauth-authorization-server/auth" => ("200 OK", String::new(), json!({"issuer": format!("{b}/auth"), "authorization_endpoint": format!("{b}/auth/authorize"), "token_endpoint": format!("{b}/auth/token"), "registration_endpoint": format!("{b}/auth/register"), "code_challenge_methods_supported": ["S256"]}).to_string()),
                    "/auth/token" if req.contains("grant_type=refresh_token") && req.contains("refresh_token=r1") => ("200 OK", String::new(), json!({"access_token": "good", "expires_in": 3600}).to_string()),
                    "/auth/token" => ("400 Bad Request", String::new(), json!({"error": "invalid_grant"}).to_string()),
                    _ => ("404 Not Found", String::new(), String::new()),
                };
                let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        base
    }

    #[test]
    fn finds_the_sign_in_and_refreshes_it() {
        let base = fake_server();
        let server = format!("{base}/mcp");
        let plan = discover(&server).unwrap();
        assert_eq!(plan.authorize, format!("{base}/auth/authorize"));
        assert_eq!(plan.token, format!("{base}/auth/token"));
        assert_eq!(plan.register.as_deref(), Some(format!("{base}/auth/register").as_str()));
        assert_eq!(plan.scope.as_deref(), Some("tools"));
        assert_eq!(plan.resource.as_deref(), Some(server.as_str()));

        // No sign-in yet: the link says the server needs one.
        let dir = std::env::temp_dir().join(format!("scout-mcpauth-{}", crate::session::now_ms()));
        unsafe { std::env::set_var("SCOUT_HOME", &dir) };
        let name = format!("t{}", crate::session::now_ms());
        let cfg = crate::mcp_client::ServerConfig { url: server.clone(), ..Default::default() };
        let err = crate::mcp_client::connect_for_test(&name, &cfg).unwrap_err();
        assert_eq!(err, crate::mcp_client::NEEDS_SIGN_IN);

        // A token that ran out is refreshed before it's sent.
        store(&name, &Auth { url: server.clone(), client_id: "c".into(), token_endpoint: plan.token.clone(), access_token: "old".into(), refresh_token: Some("r1".into()), expires_at: Some(1), ..Default::default() });
        assert_eq!(token(&name, &server).as_deref(), Some("good"));
        assert!(saved(&name, &format!("{base}/other")).is_none());
        // A refresh the server refuses drops the sign-in.
        store(&name, &Auth { url: server.clone(), client_id: "c".into(), token_endpoint: plan.token, access_token: "old".into(), refresh_token: Some("bad".into()), expires_at: Some(1), ..Default::default() });
        assert!(token(&name, &server).is_none());
        assert!(!signed_in(&name, &server));
        forget(&name);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn well_known_paths() {
        let u = Url::parse("https://x.dev/v1/mcp").unwrap();
        assert_eq!(well_known(&u, "oauth-protected-resource"), vec!["https://x.dev/.well-known/oauth-protected-resource/v1/mcp", "https://x.dev/.well-known/oauth-protected-resource"]);
        let u = Url::parse("https://x.dev").unwrap();
        assert_eq!(well_known(&u, "oauth-authorization-server"), vec!["https://x.dev/.well-known/oauth-authorization-server"]);
    }
}
