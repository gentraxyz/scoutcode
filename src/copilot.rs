//! GitHub Copilot, on the user's own GitHub account. Models don't run behind an HTTP API here: GitHub's SDK drives a
//! runtime it ships separately, which loops over the model itself and calls Scout's tools back. The runtime is
//! downloaded at sign-in and checked against the hashes below, and only ever runs on this PC with the user's token.

use crate::api::{Message, http};
use crate::config::Config;
use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use github_copilot_sdk::tool::ToolHandler;
use github_copilot_sdk::types::{Attachment, MessageOptions, SessionConfig, SystemMessageConfig};
use github_copilot_sdk::{CliProgram, Client, ClientOptions, Error, Tool, ToolInvocation, ToolResult};
use serde_json::{Value, json};
use sha2::{Digest, Sha512};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

/// Scout's GitHub OAuth App. A client id isn't a secret.
const CLIENT_ID: &str = "Ov23liJvQotlxKI3corN";
/// The version of the runtime, the same as the `github-copilot-sdk` crate in Cargo.toml. It only changes with a Scout release.
const VERSION: &str = "1.0.28";
/// npm's integrity hash of each platform's runtime package at that version.
const PACKAGES: [(&str, &str); 8] = [
    ("win32-x64", "pzlzyuBHcF3TrgBue0w9cypmZ+1BpCKL1VdVO98SlXqOhOBJ2aHXAY42gI0ZVbZuCfkv4T+EUHAriyv0PJIidw=="),
    ("win32-arm64", "jG18fJN20P5fm8pWkpvKcFzH8XupSj8cPo0ZULFJkmYDnV4UcMceGAJrIA8L/iNFu9vabfIP3cmOmWDPNyMP+A=="),
    ("darwin-arm64", "R15W4SUS12DyGiWEX/hxsSWTtvyo+Cxrn4SAbU0NeB1kBb+IzkvKmc85gOefh7QzuRZnSn2Wnch7oOmfoYMuPQ=="),
    ("darwin-x64", "b4haZ9J0G1BDg2gtNHW8yAEyddulrUZkGcnP7Dm7Ue1uNiO35qAqS+Q5jdadYj+cwhlBnNqXscO5l19tpaPyrw=="),
    ("linux-x64", "yAsoTZfP//a1oirpdXc4oIX1m69ePP4ef3Yasmm3iT+hM7mpQLTxWWaDD6C2vuIJk1tPZLK0jndFVoFYr/d7yw=="),
    ("linux-arm64", "41G/pQajhEz6rCeUSDOooFPrKSpgE8aOetH0y4a3zIJNKybr+1QfJJkia9O49LQpzQBAgn0AkEF/h6qrYdUnQQ=="),
    ("linuxmusl-x64", "8vFhwdmy8mYswNBqkWqbuaKG833DbnZRctk7WL5cw5UjXqipEMMt6XKeadmdZihwLwPiDoZnG5ymmCslaFbpoA=="),
    ("linuxmusl-arm64", "ixerQJEpCDhsFKM/ct6JVUepczY+aISb9oygq4BWAoC1cxutsA3hIRY28UGH5yOGlCinw7Ig6MEoLA0pyEK+CQ=="),
];
pub const PLAN_URL: &str = "https://github.com/settings/copilot";

/// How signing in is going, for the window.
pub enum Update {
    Code { code: String, url: String },
    Progress { done: u64, total: u64 },
    Installed,
    Done { token: String, user: Option<String> },
    Failed(String),
}

fn platform() -> Result<&'static str, String> {
    let os = if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_env = "musl") {
        "linuxmusl"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
    let name = format!("{os}-{arch}");
    PACKAGES.iter().map(|p| p.0).find(|p| *p == name).ok_or_else(|| format!("Copilot isn't available for {name}."))
}

fn root() -> PathBuf {
    crate::config::home().join("runtime").join("copilot-sdk")
}

/// Where the runtime lives. A folder only gets this name once it's complete, so one that exists can be used.
fn install_dir() -> PathBuf {
    root().join(VERSION)
}

/// Copilot's own files (sessions, logs, caches), kept apart from the user's ~/.copilot.
fn data_dir() -> PathBuf {
    crate::config::home().join("runtime").join("copilot-home")
}

fn runtime_exe() -> Result<PathBuf, String> {
    let p = platform()?;
    Ok(install_dir().join("prebuilds").join(p).join(if cfg!(windows) { "copilot-runtime.exe" } else { "copilot-runtime" }))
}

pub fn installed() -> bool {
    runtime_exe().is_ok_and(|e| e.is_file())
}

/// Deletes the runtime and Copilot's files.
pub fn remove_runtime() {
    let _ = std::fs::remove_dir_all(root());
    let _ = std::fs::remove_dir_all(data_dir());
}

/// Downloads the runtime unless it's there. The download must match the built-in hash, and the folder only appears
/// once it's fully unpacked. Older versions are deleted afterwards.
pub fn ensure_runtime(progress: &mut dyn FnMut(u64, u64)) -> Result<(), String> {
    static INSTALLING: Mutex<()> = Mutex::new(());
    let _one_at_a_time = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    if installed() {
        return Ok(());
    }
    let p = platform()?;
    let want = PACKAGES.iter().find(|x| x.0 == p).map(|x| x.1).unwrap_or_default();
    let url = format!("https://registry.npmjs.org/@github/copilot-sdk-{p}/-/copilot-sdk-{p}-{VERSION}.tgz");
    let res = http().get(&url).call().map_err(|e| format!("Couldn't download Copilot: {e}"))?;
    if res.status().as_u16() != 200 {
        return Err(format!("Couldn't download Copilot: HTTP {}", res.status().as_u16()));
    }
    let total = res.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(0);

    let (tmp, archive) = (root().join(format!("{VERSION}.partial")), root().join(format!("{VERSION}.tgz")));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(root()).map_err(|e| e.to_string())?;
    let mut file = std::fs::File::create(&archive).map_err(|e| e.to_string())?;
    let (mut body, mut hash, mut buf, mut done) = (res.into_body().into_reader(), Sha512::new(), vec![0u8; 256 * 1024], 0u64);
    loop {
        let n = body.read(&mut buf).map_err(|e| format!("The Copilot download stopped: {e}"))?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        progress(done, total);
    }
    drop(file);
    let finish = (|| {
        if STANDARD.encode(hash.finalize()) != want {
            return Err("The Copilot download didn't match its expected hash, so it wasn't installed.".to_string());
        }
        unpack(&archive, &tmp).map_err(|e| format!("Couldn't unpack Copilot: {e}"))?;
        let _ = std::fs::remove_dir_all(install_dir());
        std::fs::rename(&tmp, install_dir()).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_file(&archive);
    if finish.is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    finish?;
    for e in std::fs::read_dir(root()).into_iter().flatten().flatten() {
        if e.file_name() != VERSION {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    Ok(())
}

/// Unpacks an npm package's files, which sit under "package/". `unpack_in` refuses paths that leave the folder.
fn unpack(archive: &Path, to: &Path) -> std::io::Result<()> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(std::fs::File::open(archive)?));
    let staged = to.with_extension("unpacking");
    let _ = std::fs::remove_dir_all(&staged);
    std::fs::create_dir_all(&staged)?;
    for entry in tar.entries()? {
        entry?.unpack_in(&staged)?;
    }
    std::fs::rename(staged.join("package"), to)?;
    std::fs::remove_dir_all(&staged)
}

// ---------- sign-in ----------

fn post_github(url: &str, body: Value) -> Result<Value, String> {
    let mut res = http().post(url).header("Accept", "application/json").send_json(body).map_err(|e| e.to_string())?;
    res.body_mut().read_json().map_err(|e| e.to_string())
}

/// The token's owner, for Settings. Nothing depends on it.
fn github_login(token: &str) -> Option<String> {
    let mut res = http().get("https://api.github.com/user").header("Authorization", &format!("Bearer {token}")).header("Accept", "application/vnd.github+json").call().ok()?;
    let body: Value = res.body_mut().read_json().ok()?;
    body.get("login").and_then(Value::as_str).map(String::from)
}

/// Waits for the user to approve `device_code`, for as long as GitHub says it's good.
fn poll_token(device_code: &str, mut wait: u64, expires_in: u64, cancel: &AtomicBool) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(expires_in);
    while Instant::now() < deadline {
        let next = Instant::now() + Duration::from_secs(wait);
        while Instant::now() < next {
            if cancel.load(Ordering::Relaxed) {
                return Err(String::new());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let r = post_github("https://github.com/login/oauth/access_token", json!({"client_id": CLIENT_ID, "device_code": device_code, "grant_type": "urn:ietf:params:oauth:grant-type:device_code"}));
        // A failed poll is usually the network, so the next one may work.
        let Ok(r) = r else { continue };
        if let Some(t) = r.get("access_token").and_then(Value::as_str) {
            return Ok(t.to_string());
        }
        match r.get("error").and_then(Value::as_str) {
            Some("authorization_pending") | None => {}
            Some("slow_down") => wait += 5,
            Some("access_denied") => return Err("Sign-in was cancelled in the browser.".into()),
            Some(e) => return Err(format!("GitHub refused the sign-in: {e}")),
        }
    }
    Err("The sign-in code expired. Try again.".into())
}

/// Signs in with GitHub's device flow while the runtime downloads. Nothing to set up beyond approving the code.
pub fn sign_in(cancel: Arc<AtomicBool>, send: impl Fn(Update) + Clone + Send + 'static) {
    std::thread::spawn(move || {
        let run = || -> Result<(String, Option<String>), String> {
            let d = post_github("https://github.com/login/device/code", json!({"client_id": CLIENT_ID, "scope": "read:user"})).map_err(|e| format!("Couldn't reach GitHub: {e}"))?;
            let (Some(code), Some(device_code)) = (d.get("user_code").and_then(Value::as_str), d.get("device_code").and_then(Value::as_str)) else {
                return Err(format!("GitHub refused the sign-in: {}", d.get("error_description").or_else(|| d.get("error")).and_then(Value::as_str).unwrap_or("no code came back")));
            };
            let url = d.get("verification_uri").and_then(Value::as_str).unwrap_or("https://github.com/login/device").to_string();
            send(Update::Code { code: code.to_string(), url: url.clone() });
            crate::oauth::open_browser(&url);
            let s = send.clone();
            let installing = std::thread::spawn(move || {
                let r = ensure_runtime(&mut |done, total| s(Update::Progress { done, total }));
                if r.is_ok() {
                    s(Update::Installed);
                }
                r
            });
            let token = poll_token(device_code, d.get("interval").and_then(Value::as_u64).unwrap_or(5), d.get("expires_in").and_then(Value::as_u64).unwrap_or(900), &cancel)?;
            installing.join().map_err(|_| "The Copilot install crashed.".to_string())??;
            let user = github_login(&token);
            Ok((token, user))
        };
        match run() {
            Ok((token, user)) => send(Update::Done { token, user }),
            Err(e) => send(Update::Failed(e)),
        }
    });
}

/// Whether Copilot takes this kind of token. Classic personal tokens are the one kind it refuses, and it only says "Not authenticated".
pub fn check_token(token: &str) -> Result<(), String> {
    if token.starts_with("ghp_") {
        return Err("Copilot doesn't accept classic GitHub tokens (ghp_). Sign in under Settings > Providers, or use a fine-grained token with the Copilot Requests permission.".into());
    }
    Ok(())
}

// ---------- running ----------

/// What a run tells the agent's thread.
pub enum Msg {
    Text(String),
    Reasoning(String),
    RequestStart,
    RequestEnd,
    Usage { prompt: u64, completion: u64, cached: u64, resets: Option<String> },
    Tool { id: String, name: String, arguments: String, reply: oneshot::Sender<String> },
    Idle,
    Failed { message: String, kind: String },
}

enum Cmd {
    Send(String),
    Abort,
}

/// One Copilot session, on its own runtime process: started for a turn and gone after it, so the conversation always
/// comes from Scout's messages and undo, compaction and switching provider have nothing to keep in step.
pub struct Run {
    rx: Receiver<Msg>,
    cmd: mpsc::UnboundedSender<Cmd>,
    rt: tokio::runtime::Runtime,
    task: Option<tokio::task::JoinHandle<()>>,
}

struct Bridge(Sender<Msg>);

#[async_trait]
impl ToolHandler for Bridge {
    async fn call(&self, inv: ToolInvocation) -> Result<ToolResult, Error> {
        let (reply, answer) = oneshot::channel();
        let msg = Msg::Tool { id: inv.tool_call_id, name: inv.tool_name, arguments: inv.arguments.to_string(), reply };
        let out = if self.0.send(msg).is_ok() { answer.await.ok() } else { None };
        Ok(ToolResult::Text(out.unwrap_or_else(|| "Interrupted by the user.".into())))
    }
}

/// The options that keep the runtime out of the user's own Copilot setup: its own folders and only the token it's given.
fn client_options(token: &str, cwd: &str) -> Result<ClientOptions, String> {
    let (data, exe) = (data_dir(), runtime_exe()?);
    std::fs::create_dir_all(&data).map_err(|e| e.to_string())?;
    let d = data.to_string_lossy().to_string();
    let env = [("USERPROFILE", d.clone()), ("HOME", d.clone()), ("LOCALAPPDATA", format!("{d}/local")), ("APPDATA", format!("{d}/roaming")), ("XDG_CONFIG_HOME", format!("{d}/config")), ("XDG_STATE_HOME", format!("{d}/state")), ("XDG_CACHE_HOME", format!("{d}/cache")), ("COPILOT_GITHUB_TELEMETRY_DISABLED", "true".to_string())];
    Ok(ClientOptions::default().with_program(CliProgram::Path(exe)).with_cwd(cwd).with_base_directory(&data).with_env(env).with_env_remove(["COPILOT_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN", "COPILOT_CLI_PATH"]).with_github_token(token))
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().map_err(|e| e.to_string())
}

impl Run {
    /// Starts a session with `system` as its whole system prompt and Scout's tools (OpenAI-format `specs`) as its only
    /// ones, and sends `prompt`. Scout asks the user about each tool itself, so the tools skip the runtime's permission
    /// step and anything else the runtime might try is refused.
    pub fn start(cfg: &Config, model: &str, cwd: &str, system: String, prompt: String, images: Vec<(String, String)>, specs: &[Value]) -> Result<Run, String> {
        let token = cfg.copilot_token.clone().ok_or("Sign in to GitHub Copilot under Settings > Providers first.")?;
        check_token(&token)?;
        let options = client_options(&token, cwd)?;
        let (tx, rx) = channel();
        let (cmd, commands) = mpsc::unbounded_channel();
        let rt = runtime()?;
        let (name_list, tools): (Vec<String>, Vec<Tool>) = specs
            .iter()
            .filter_map(|s| {
                let f = s.get("function")?;
                let name = f.get("name")?.as_str()?.to_string();
                let tool = Tool::new(name.clone())
                    .with_description(f.get("description").and_then(Value::as_str).unwrap_or(""))
                    .with_parameters(f.get("parameters").cloned().unwrap_or(json!({"type": "object", "properties": {}})))
                    .with_handler(Arc::new(Bridge(tx.clone())))
                    .with_overrides_built_in_tool(true)
                    .with_skip_permission(true);
                Some((name, tool))
            })
            .unzip();
        let session = SessionConfig::default()
            .deny_all_permissions()
            .with_model(model)
            .with_streaming(true)
            .with_working_directory(cwd)
            .with_system_message(SystemMessageConfig::new().with_mode("replace").with_content(system))
            .with_available_tools(name_list)
            .with_tools(tools);
        let task = rt.spawn(async move {
            if let Err(e) = drive(options, session, prompt, images, tx.clone(), commands).await {
                let _ = tx.send(Msg::Failed { message: e.to_string(), kind: String::new() });
            }
        });
        Ok(Run { rx, cmd, rt, task: Some(task) })
    }

    pub fn next(&self, wait: Duration) -> Option<Msg> {
        self.rx.recv_timeout(wait).ok()
    }

    pub fn send(&self, text: String) {
        let _ = self.cmd.send(Cmd::Send(text));
    }

    pub fn abort(&self) {
        let _ = self.cmd.send(Cmd::Abort);
    }

    /// Closes the session and its process, giving them a moment before dropping them.
    pub fn finish(mut self) {
        let (cmd, _) = mpsc::unbounded_channel();
        drop(std::mem::replace(&mut self.cmd, cmd));
        if let Some(task) = self.task.take() {
            let _ = self.rt.block_on(async { tokio::time::timeout(Duration::from_secs(5), task).await });
        }
    }
}

/// Runs the session until the commands stop, forwarding its events.
async fn drive(options: ClientOptions, config: SessionConfig, prompt: String, images: Vec<(String, String)>, tx: Sender<Msg>, mut commands: mpsc::UnboundedReceiver<Cmd>) -> Result<(), Error> {
    let client = Client::start(options).await?;
    let session = client.create_session(config).await?;
    let mut events = session.subscribe();
    let forward = tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(e) => {
                    if let Some(m) = message_of(&e.event_type, &e.data) {
                        let _ = tx.send(m);
                    }
                }
                Err(e) if matches!(e.kind(), github_copilot_sdk::subscription::RecvErrorKind::Lagged(_)) => {}
                Err(_) => {
                    // The runtime went away. Harmless if the turn is over: nobody's listening by then.
                    let _ = tx.send(Msg::Failed { message: "The Copilot runtime stopped unexpectedly.".into(), kind: String::new() });
                    return;
                }
            }
        }
    });
    let attachments = images.into_iter().map(|(mime_type, data)| Attachment::Blob { data, mime_type, display_name: None }).collect();
    session.send(MessageOptions::new(prompt).with_attachments(attachments)).await?;
    while let Some(c) = commands.recv().await {
        match c {
            Cmd::Send(text) => {
                session.send(text.as_str()).await?;
            }
            Cmd::Abort => {
                let _ = session.abort().await;
            }
        }
    }
    forward.abort();
    let id = session.id().clone();
    let _ = session.disconnect().await;
    let _ = client.delete_session(&id).await;
    let _ = client.stop().await;
    Ok(())
}

fn message_of(kind: &str, d: &Value) -> Option<Msg> {
    let text = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let num = |k: &str| d.get(k).and_then(Value::as_u64).unwrap_or(0);
    Some(match kind {
        "assistant.message_delta" => Msg::Text(text("deltaContent")),
        "assistant.reasoning_delta" => Msg::Reasoning(text("deltaContent")),
        "assistant.turn_start" => Msg::RequestStart,
        "assistant.turn_end" => Msg::RequestEnd,
        "assistant.usage" => Msg::Usage { prompt: num("inputTokens"), completion: num("outputTokens"), cached: num("cacheReadTokens"), resets: d.pointer("/quotaSnapshots/chat/resetDate").and_then(Value::as_str).map(String::from) },
        "session.idle" => Msg::Idle,
        "session.error" => Msg::Failed { message: text("message"), kind: text("errorType") },
        _ => return None,
    })
}

/// What Copilot's error means for the user. `resets` is when the allowance renews, if a request reported it.
pub fn explain(message: &str, kind: &str, resets: Option<&str>) -> String {
    let m = format!("{kind} {message}").to_lowercase();
    if ["quota", "credit", "allowance", "exhausted"].iter().any(|w| m.contains(w)) {
        let when = resets.and_then(|r| r.get(..10)).map(|d| format!("It resets on {d}.")).unwrap_or_else(|| "It resets at the start of the month.".into());
        return format!("Your Copilot allowance has run out. {when} This chat is saved, so you can carry on by picking another provider in the model menu.");
    }
    if m.contains("not authenticated") || m.contains("authentication info") || m.contains("401") || m.contains("bad credentials") {
        return "GitHub didn't accept your Copilot sign-in. Sign in again under Settings > Providers.".into();
    }
    if ["subscription", "not entitled", "no access", "not enabled", "403"].iter().any(|w| m.contains(w)) {
        return format!("This GitHub account can't use Copilot yet ({message}). Turn on the free plan at {PLAN_URL}, then try again.");
    }
    format!("Copilot: {message}")
}

/// The models this account may pick; a free plan lists none and gets `auto` alone. Empty when that can't be told.
pub fn list_models(token: &str) -> Vec<(String, String)> {
    if !installed() {
        return vec![];
    }
    let Ok(options) = client_options(token, &data_dir().to_string_lossy()) else { return vec![] };
    let Ok(rt) = runtime() else { return vec![] };
    rt.block_on(async move {
        let client = Client::start(options).await.ok()?;
        let models = client.list_models().await.ok();
        let _ = client.stop().await;
        models
    })
    .unwrap_or_default()
    .into_iter()
    .map(|m| (m.id.clone(), m.name.clone()))
    .collect()
}

/// The images of the newest user message, when nothing came after it: they go along with `as_prompt`'s message. Earlier
/// ones are only named in the text.
pub fn newest_images(messages: &[Message]) -> Vec<(String, String)> {
    match messages.last() {
        Some(Message::User { content }) => content.images(),
        _ => vec![],
    }
}

/// A conversation as the system prompt and one message: the SDK has no way to hand it earlier turns, so they go in as
/// text, tagged so the model tells them from what's new. Images can't go this way: see `newest_images`.
pub fn as_prompt(messages: &[Message]) -> (String, String) {
    let mut system = String::new();
    let mut lines = vec![];
    let mut names = std::collections::HashMap::new();
    let last_user = messages.iter().rposition(|m| matches!(m, Message::User { .. })).unwrap_or(messages.len());
    let tail = messages.iter().skip(last_user + 1).count();
    for m in &messages[..if tail == 0 { last_user } else { messages.len() }] {
        match m {
            Message::System { content } => system.push_str(content),
            Message::User { content } => lines.push(format!("[user]\n{}", content.text())),
            Message::Assistant { content, tool_calls, .. } => {
                if let Some(c) = content.as_deref().filter(|c| !c.is_empty()) {
                    lines.push(format!("[assistant]\n{c}"));
                }
                for c in tool_calls.iter().flatten() {
                    names.insert(c.id.clone(), c.function.name.clone());
                    lines.push(format!("[tool call: {}]\n{}", c.function.name, c.function.arguments));
                }
            }
            Message::Tool { tool_call_id, content } => lines.push(format!("[tool result: {}]\n{content}", names.get(tool_call_id).map_or("tool", String::as_str))),
        }
    }
    // The newest user message, when nothing came after it, is the one to answer.
    let newest = match messages.last() {
        Some(Message::User { content }) if tail == 0 => content.text(),
        _ => "Continue from where the conversation left off.".to_string(),
    };
    if lines.is_empty() {
        return (system, newest);
    }
    let history = lines.join("\n\n");
    (system, format!("<earlier_conversation>\n{history}\n</earlier_conversation>\n\nThat was this conversation so far, for context. Answer this new message from the user:\n\n{newest}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Content, FunctionCall, ToolCall};

    fn user(t: &str) -> Message {
        Message::User { content: Content::Text(t.into()) }
    }

    #[test]
    fn a_first_message_goes_alone() {
        let (system, prompt) = as_prompt(&[Message::System { content: "Be Scout.".into() }, user("hi")]);
        assert_eq!((system.as_str(), prompt.as_str()), ("Be Scout.", "hi"));
    }

    #[test]
    fn earlier_turns_go_in_as_tagged_text() {
        let call = ToolCall { id: "c1".into(), kind: "function".into(), extra_content: None, function: FunctionCall { name: "view".into(), arguments: "{\"path\":\"a\"}".into() } };
        let msgs = [
            Message::System { content: "S".into() },
            user("read a"),
            Message::Assistant { content: Some("ok".into()), tool_calls: Some(vec![call]), reasoning_content: None },
            Message::Tool { tool_call_id: "c1".into(), content: "text of a".into() },
            user("thanks"),
        ];
        let (_, prompt) = as_prompt(&msgs);
        assert!(prompt.contains("[user]\nread a") && prompt.contains("[tool call: view]") && prompt.contains("[tool result: view]\ntext of a"));
        assert!(prompt.ends_with("thanks") && !prompt.contains("[user]\nthanks"));
    }

    #[test]
    fn a_conversation_ending_on_a_tool_result_carries_on() {
        let (_, prompt) = as_prompt(&[user("go"), Message::Tool { tool_call_id: "x".into(), content: "done".into() }]);
        assert!(prompt.contains("[user]\ngo") && prompt.contains("done") && prompt.ends_with("Continue from where the conversation left off."));
    }

    #[test]
    fn errors_say_what_to_do() {
        assert!(explain("quota exceeded", "", Some("2026-11-01T00:00:00Z")).contains("resets on 2026-11-01"));
        assert!(explain("Not authenticated. Please authenticate first.", "", None).contains("Sign in again"));
        assert!(explain("no active subscription", "", None).contains(PLAN_URL));
        assert_eq!(explain("boom", "", None), "Copilot: boom");
    }

    #[test]
    fn classic_tokens_are_refused_up_front() {
        assert!(check_token("ghp_abc").is_err());
        for t in ["gho_abc", "ghu_abc", "github_pat_abc"] {
            assert!(check_token(t).is_ok());
        }
    }

    /// Downloads and unpacks the real runtime into SCOUT_HOME: `SCOUT_HOME=<temp> cargo test -- --ignored live_install --nocapture`.
    #[test]
    #[ignore]
    fn live_install() {
        assert!(std::env::var("SCOUT_HOME").is_ok(), "set SCOUT_HOME so this doesn't touch the real install");
        let mut last = 0;
        ensure_runtime(&mut |done, total| last = total.max(done)).unwrap();
        println!("downloaded {last} bytes to {}", install_dir().display());
        assert!(installed());
        let exe = runtime_exe().unwrap();
        assert!(exe.with_file_name("runtime.node").is_file());
        // A second call finds it, and a corrupted hash can't be reached through the public path.
        ensure_runtime(&mut |_, _| panic!("downloaded twice")).unwrap();
        let rt = runtime().unwrap();
        let options = ClientOptions::default().with_program(CliProgram::Path(exe)).with_cwd(data_dir_ready()).with_base_directory(data_dir_ready()).with_use_logged_in_user(false);
        let status = rt.block_on(async {
            let c = Client::start(options).await.unwrap();
            let s = c.get_auth_status().await.unwrap();
            let _ = c.stop().await;
            s
        });
        assert!(!status.is_authenticated);
    }

    fn data_dir_ready() -> PathBuf {
        std::fs::create_dir_all(data_dir()).unwrap();
        data_dir()
    }

    #[test]
    fn every_platform_has_a_full_sha512() {
        for (_, hash) in PACKAGES {
            assert_eq!(STANDARD.decode(hash).unwrap().len(), 64);
        }
        assert!(platform().is_ok());
    }
}
