//! Lets a chatbot use Scout's tools over MCP: the chatbot's model does the thinking, and the files and shell are the ones
//! on this computer, under Scout's permission modes. `Scout.exe mcp` serves an app on this computer over stdio. The link
//! in Settings > Chatbots serves web chatbots through the site, which it long-polls for calls (see src/lib/mcp-relay.ts
//! on the server), so this computer takes no incoming connections.

use crate::agent::{Agent, Ev, Hooks, PermissionAnswer, PermissionRequest, Shared, cap_output};
use crate::api::{Content, FunctionCall, Message, ToolCall, http};
use crate::config::{self, Config};
use crate::session::{self, save_session};
use crate::tools::{self, EditTool, ToolResult};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Everything but what needs Scout's own model or conversation.
const NOT_SERVED: [&str; 5] = ["todo_write", "task", "present_plan", "ask_questions", "update_memory"];
pub const PROTOCOLS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// What the chatbot is told first: its work is saved as a Scout chat, which it has to start or resume.
fn instructions(cwd: &str) -> String {
    format!(
        "These tools run on the user's computer through Scout, in {cwd}. Scout saves your work as a chat the user can \
         rewind, or carry on with another AI. Before anything else, call start_chat with the user's message, or \
         resume_chat to carry on a chat from list_chats (one another AI started, say). Pass the chat_id it gives you with \
         every other call. For each further message from the user, call resume_chat with that chat_id and their message, \
         and when you've answered, call save_reply with your answer. Read a file before you edit it."
    )
}

fn chat_id_schema() -> Value {
    json!({"type": "string", "description": "The chat this call belongs to, from start_chat or resume_chat."})
}

/// The tools that start, carry on and fill in a chat.
fn chat_tools() -> Vec<Value> {
    let request = json!({"type": "string", "description": "The user's message, word for word."});
    vec![
        json!({"name": "start_chat", "description": "Starts a Scout chat for what the user just asked, so your work is saved and can be rewound or carried on in another AI. Call it before any other tool, then pass the chat_id it returns with every call.", "inputSchema": {"type": "object", "properties": {"request": request, "title": {"type": "string", "description": "A short title for the chat. Optional: the request is used if you leave it out."}}, "required": ["request"]}}),
        json!({"name": "resume_chat", "description": "Carries on a saved Scout chat with the user's new message, and returns what happened in it so far. Use it for each further message in a chat you started, and to pick up a chat from list_chats, including one another AI or the Scout app worked in.", "inputSchema": {"type": "object", "properties": {"chat_id": chat_id_schema(), "request": request}, "required": ["chat_id", "request"]}}),
        json!({"name": "list_chats", "description": "The saved Scout chats in this folder, newest first, with their ids and titles.", "inputSchema": {"type": "object", "properties": {"limit": {"type": "integer", "description": "How many to list. Default 20."}}}}),
        json!({"name": "save_reply", "description": "Saves your answer to the user's message into the chat, so it shows in Scout and another AI can read it. Call it when you've answered.", "inputSchema": {"type": "object", "properties": {"chat_id": chat_id_schema(), "text": {"type": "string", "description": "Your answer, as you gave it to the user."}}, "required": ["chat_id", "text"]}}),
    ]
}

fn mcp_tools() -> Vec<Value> {
    let mut out = chat_tools();
    out.extend(tools::specs(None, EditTool::StrReplace).iter().map(|s| &s["function"]).filter(|f| !NOT_SERVED.contains(&f["name"].as_str().unwrap_or(""))).map(|f| {
        let mut schema = f["parameters"].clone();
        if let Some(o) = schema.as_object_mut() {
            o.entry("properties").or_insert_with(|| json!({}))["chat_id"] = chat_id_schema();
            let req = o.entry("required").or_insert_with(|| json!([]));
            if let Some(r) = req.as_array_mut() {
                r.push(json!("chat_id"));
            }
        }
        json!({"name": f["name"], "description": f["description"], "inputSchema": schema})
    }));
    out
}

fn shared(cfg: &Config, mode: &str) -> Arc<Shared> {
    Arc::new(Shared {
        mode: Mutex::new(mode.to_string()),
        model: Mutex::new(cfg.model.clone()),
        provider: Mutex::new(cfg.provider.clone()),
        effort: Mutex::new("none".into()),
        cancel: Arc::new(AtomicBool::new(false)),
        edit_tools: Default::default(),
        model_names: Default::default(),
        no_vision: Default::default(),
        steer: Default::default(),
    })
}

/// The chats a chatbot is working in, with the channel their events arrive on. Whoever holds the channel answers
/// their permission questions.
fn bridge(cfg: &Config, cwd: &str, mode: &str) -> (Chats, Hooks, Receiver<Ev>) {
    let (tx, rx) = channel();
    let chats = Chats { cfg: cfg.clone(), cwd: cwd.to_string(), shared: shared(cfg, mode), client: "Chatbot".into(), open: HashMap::new() };
    (chats, Hooks { chat: None, tx, ctx: crate::waker::Waker::default() }, rx)
}

fn call_tool(agent: &mut Agent, h: &Hooks, id: &str, name: &str, args: &Value) -> ToolResult {
    let call = ToolCall { id: id.to_string(), kind: "function".into(), extra_content: None, function: FunctionCall { name: name.to_string(), arguments: args.to_string() } };
    let mut r = agent.run_tool(&call, h);
    r.output = cap_output(r.output);
    r
}

/// A tool call's id in the saved chat. The chatbot's own ids start again with each connection.
fn call_id() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    format!("mcp_{:x}_{}", session::now_ms(), N.fetch_add(1, Ordering::Relaxed))
}

/// The saved chats a chatbot works in. Each tool call is saved in its chat with its result, and each message of the
/// user's starts a checkpoint, so the chat can be rewound in Scout, files and all.
pub struct Chats {
    cfg: Config,
    cwd: String,
    shared: Arc<Shared>,
    /// Who the chatbot is, shown on its replies.
    client: String,
    open: HashMap<String, Agent>,
}

/// What a chat tool call came to, and the chat it was in.
struct Served {
    result: ToolResult,
    chat: Option<String>,
}

impl Chats {
    fn agent(&self, s: session::Session) -> Agent {
        let mut agent = Agent::new(self.cfg.clone(), s, self.shared.clone());
        agent.read_only_note = "Scout is read-only here. To allow edits and commands, the user has to choose another permission mode.";
        agent
    }

    /// The chat `id`, read from disk unless it's open: it may have been worked in since, in Scout or another AI.
    fn chat(&mut self, args: &Value, reread: bool) -> Result<&mut Agent, String> {
        let id = args.get("chat_id").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if id.is_empty() {
            return Err("Start a chat first: call start_chat with the user's message (or resume_chat to carry on a chat from list_chats), then pass its chat_id with this call.".into());
        }
        let stale = reread || !self.open.contains_key(&id);
        if stale {
            if let Some(s) = session::load_session(&id) {
                if !session::same_dir(&s.cwd, &self.cwd) {
                    return Err(format!("Chat {id} is in {}, but Scout is serving {} here. Start a new chat instead.", s.cwd, self.cwd));
                }
                // Files read in it stay read, so an open chat isn't asked to read them again.
                let read = self.open.remove(&id).map(|a| a.read_files).unwrap_or_default();
                let mut a = self.agent(s);
                a.read_files = read;
                self.open.insert(id.clone(), a);
            }
        }
        self.open.get_mut(&id).ok_or_else(|| format!("There's no chat {id}. Call list_chats to see the chats here, or start_chat to start one."))
    }

    fn call(&mut self, h: &Hooks, name: &str, args: &Value) -> Served {
        let chat_err = |e: String| Served { result: ToolResult::err(e, "no chat"), chat: None };
        let text = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
        match name {
            "list_chats" => {
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20).clamp(1, 100) as usize;
                let here: Vec<String> = session::list_sessions()
                    .into_iter()
                    .filter(|s| session::same_dir(&s.cwd, &self.cwd))
                    .take(limit)
                    .map(|s| format!("{}  {}  ({})", s.id, if s.title.is_empty() { "untitled" } else { &s.title }, session::ago(s.updated)))
                    .collect();
                let out = if here.is_empty() { "No saved chats in this folder yet. Call start_chat to start one.".to_string() } else { here.join("\n") };
                Served { result: ToolResult::ok(out, "listed"), chat: None }
            }
            "start_chat" => {
                let request = text("request");
                if request.is_empty() {
                    return chat_err("start_chat needs the user's message as request.".into());
                }
                let mut s = session::new_session(&self.cwd, &self.cfg.model);
                s.title = text("title").chars().take(80).collect();
                let mut a = self.agent(s);
                a.checkpoint(request.clone());
                a.session.messages.push(Message::User { content: Content::Text(request) });
                save_session(&mut a.session);
                let id = a.session.id.clone();
                self.open.insert(id.clone(), a);
                let out = format!("Started chat {id}. Pass chat_id \"{id}\" with every call for this chat, call resume_chat with it for each new message from the user, and save_reply with your answer.");
                Served { result: ToolResult::ok(out, "started"), chat: Some(id) }
            }
            "resume_chat" => {
                let request = text("request");
                if request.is_empty() {
                    return chat_err("resume_chat needs the user's new message as request.".into());
                }
                let client = self.client.clone();
                let a = match self.chat(args, true) {
                    Ok(a) => a,
                    Err(e) => return chat_err(e),
                };
                let history = transcript(&a.session.messages, &client);
                a.checkpoint(request.clone());
                a.session.messages.push(Message::User { content: Content::Text(request) });
                save_session(&mut a.session);
                let id = a.session.id.clone();
                let out = format!("Resumed chat {id} ({}). Keep passing chat_id \"{id}\".\n\nSo far:\n{history}", a.session.title);
                Served { result: ToolResult::ok(out, "resumed"), chat: Some(id) }
            }
            "save_reply" => {
                let reply = text("text");
                let client = self.client.clone();
                let a = match self.chat(args, false) {
                    Ok(a) => a,
                    Err(e) => return chat_err(e),
                };
                if !reply.is_empty() {
                    a.session.messages.push(Message::Assistant { content: Some(reply), tool_calls: None, reasoning_content: None, model: Some(client), provider: None });
                    save_session(&mut a.session);
                }
                let id = a.session.id.clone();
                Served { result: ToolResult::ok("Saved.", "saved"), chat: Some(id) }
            }
            _ => {
                let client = self.client.clone();
                let a = match self.chat(args, false) {
                    Ok(a) => a,
                    Err(e) => return chat_err(e),
                };
                let mut args = args.clone();
                if let Some(o) = args.as_object_mut() {
                    o.remove("chat_id");
                }
                let id = call_id();
                let r = call_tool(a, h, &id, name, &args);
                let call = ToolCall { id: id.clone(), kind: "function".into(), extra_content: None, function: FunctionCall { name: name.to_string(), arguments: args.to_string() } };
                a.session.messages.push(Message::Assistant { content: None, tool_calls: Some(vec![call]), reasoning_content: None, model: Some(client), provider: None });
                a.session.messages.push(Message::Tool { tool_call_id: id, content: r.output.clone() });
                save_session(&mut a.session);
                Served { result: r, chat: Some(a.session.id.clone()) }
            }
        }
    }
}

/// A chat so far, for a chatbot picking it up: what was asked, said and done, the latest last.
fn transcript(messages: &[Message], client: &str) -> String {
    const MAX: usize = 8000;
    let mut lines = vec![];
    for m in messages {
        match m {
            Message::User { content } => lines.push(format!("User: {}", crate::attach::split(&content.text()).0.trim())),
            Message::Assistant { content, tool_calls, model, .. } => {
                if let Some(c) = content.as_ref().map(|c| c.trim()).filter(|c| !c.is_empty()) {
                    lines.push(format!("{}: {c}", model.as_deref().unwrap_or(client)));
                }
                for c in tool_calls.iter().flatten() {
                    let args: Value = serde_json::from_str(&c.function.arguments).unwrap_or_default();
                    lines.push(format!("  [{} {}]", c.function.name, tools::label(&c.function.name, &args)).replace(" ]", "]"));
                }
            }
            _ => {}
        }
    }
    if lines.is_empty() {
        return "(nothing yet)".into();
    }
    // The latest matters most, so what doesn't fit is cut from the start.
    let mut out: Vec<String> = vec![];
    let mut len = 0;
    for l in lines.into_iter().rev() {
        let l: String = l.chars().take(1500).collect();
        if len + l.len() > MAX {
            out.push("(earlier messages left out)".into());
            break;
        }
        len += l.len() + 1;
        out.push(l);
    }
    out.reverse();
    out.join("\n")
}

/// `Scout.exe mcp [--cwd <folder>] [--mode <mode>]`: serves MCP over stdin and stdout until the app that started it
/// closes them. Read-only unless --mode says otherwise. Nothing here can ask, so what the mode doesn't allow is refused.
pub fn serve_stdio(args: &[String]) {
    let flag = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).cloned();
    let cwd = flag("--cwd").or_else(|| std::env::current_dir().ok().map(|d| d.to_string_lossy().into_owned())).unwrap_or_default();
    let mode = flag("--mode").filter(|m| config::MODES.contains(&m.as_str())).unwrap_or_else(|| "plan".into());
    let (mut chats, h, rx) = bridge(&config::load_config(), &cwd, &mode);
    let refusal = format!("Scout is running in \"{mode}\" mode with no one to ask. To allow this, the user has to change --mode in this server's settings.");
    std::thread::spawn(move || {
        for ev in rx {
            if let Ev::AskPermission(_, reply) = ev {
                let _ = reply.send(PermissionAnswer::Deny(refusal.clone()));
            }
        }
    });
    let tools = mcp_tools();
    let mut out = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&msg, &cwd, &tools, &mut chats, &h),
            Err(_) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"}})),
        };
        if let Some(r) = reply {
            if writeln!(out, "{r}").and_then(|_| out.flush()).is_err() {
                break;
            }
        }
    }
}

/// The answer to one JSON-RPC message, or None for a notification.
fn handle(msg: &Value, cwd: &str, tools: &[Value], chats: &mut Chats, h: &Hooks) -> Option<Value> {
    let id = msg.get("id")?;
    let ok = |result: Value| Some(json!({"jsonrpc": "2.0", "id": id, "result": result}));
    let fail = |code: i32, message: String| Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}));
    let method = msg["method"].as_str().unwrap_or("");
    match method {
        "initialize" => {
            let asked = msg["params"]["protocolVersion"].as_str().unwrap_or("");
            let info = &msg["params"]["clientInfo"];
            if let Some(n) = info["title"].as_str().or(info["name"].as_str()).map(str::trim).filter(|n| !n.is_empty()) {
                chats.client = n.chars().take(60).collect();
            }
            ok(json!({
                "protocolVersion": if PROTOCOLS.contains(&asked) { asked } else { PROTOCOLS[0] },
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "scout", "title": "Scout", "version": crate::update::VERSION},
                "instructions": instructions(cwd),
            }))
        }
        "ping" => ok(json!({})),
        "tools/list" => ok(json!({"tools": tools})),
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or("");
            if !tools.iter().any(|t| t["name"] == name) {
                return fail(-32602, format!("Unknown tool {name}."));
            }
            let args = msg["params"].get("arguments").cloned().unwrap_or(json!({}));
            let r = chats.call(h, name, &args).result;
            ok(json!({"content": [{"type": "text", "text": r.output}], "isError": r.is_error}))
        }
        _ => fail(-32601, format!("Method not found: {method}")),
    }
}

/// What the link tells the window.
pub enum LinkEvent {
    /// How the link is doing, and whether ScoutCode is answering it.
    Status(String, bool),
    /// A tool call the chatbot made, as it starts.
    Running(String),
    /// That call, finished.
    Call(LinkCall),
    /// The chatbots linked to the account, from each poll.
    Chatbots(Vec<Chatbot>),
    Ask(PermissionRequest, std::sync::mpsc::Sender<PermissionAnswer>),
    /// It stopped by itself, and why.
    Ended(String),
}

pub struct LinkCall {
    pub tool: String,
    pub label: String,
    pub ok: bool,
    /// Milliseconds since 1970, when it finished.
    pub at: u64,
    pub edits: Vec<LinkEdit>,
    /// The saved chat it went into, if it was in one.
    pub chat: Option<String>,
}

/// A file a chatbot's call changed, with how many lines.
#[derive(Clone)]
pub struct LinkEdit {
    pub path: String,
    pub added: usize,
    pub removed: usize,
    pub created: bool,
}

pub struct Chatbot {
    pub name: String,
    /// When it last reached ScoutCode's MCP endpoint, in milliseconds since 1970, if it has since the site started.
    pub seen: Option<u64>,
}

fn edits(d: &Option<tools::Display>) -> Vec<LinkEdit> {
    use crate::diff::Kind;
    match d {
        Some(tools::Display::Diff { path, lines, created }) => vec![LinkEdit {
            path: path.clone(),
            added: lines.iter().filter(|l| l.kind == Kind::Add).count(),
            removed: lines.iter().filter(|l| l.kind == Kind::Del).count(),
            created: *created,
        }],
        Some(tools::Display::Diffs(all)) => all.iter().flat_map(|d| edits(&Some(d.clone()))).collect(),
        _ => vec![],
    }
}

/// A running link, for Settings > Chatbots.
pub struct Link {
    stop: Arc<AtomicBool>,
    shared: Arc<Shared>,
    pub cwd: String,
}

impl Link {
    pub fn mode(&self) -> String {
        self.shared.mode()
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.shared.cancel.store(true, Ordering::Relaxed);
    }
}

/// Links `cwd` so web chatbots on this account can use it, until it's stopped. Asks the window about what `mode` doesn't allow.
pub fn start_link(cfg: &Config, cwd: &str, mode: &str, tell: impl Fn(LinkEvent) + Send + Sync + 'static) -> Link {
    let (mut chats, h, rx) = bridge(cfg, cwd, mode);
    let stop = Arc::new(AtomicBool::new(false));
    let link = Link { stop: stop.clone(), shared: chats.shared.clone(), cwd: cwd.to_string() };
    let tell = Arc::new(tell);

    let events = tell.clone();
    std::thread::spawn(move || {
        for ev in rx {
            match ev {
                Ev::AskPermission(req, reply) => events(LinkEvent::Ask(req, reply)),
                Ev::ToolCall { name, label, .. } => events(LinkEvent::Running(format!("{name} {label}").trim().to_string())),
                _ => {}
            }
        }
    });

    let cfg = cfg.clone();
    let cwd = cwd.to_string();
    std::thread::spawn(move || {
        let device_id = format!("{:x}{:x}", crate::session::now_ms(), std::process::id());
        let name = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_default();
        let tools = mcp_tools();
        let mut results: Vec<Value> = vec![];
        let mut failures = 0u32;
        let post = |extra: Value, mode: &str| -> Result<Value, (u16, String)> {
            let mut body = json!({"deviceId": device_id, "name": name, "cwd": cwd, "mode": mode, "tools": tools});
            if let (Some(b), Some(e)) = (body.as_object_mut(), extra.as_object()) {
                b.extend(e.clone());
            }
            let res = http()
                .post(&format!("{}/mcp/device", cfg.base_url))
                .config()
                .timeout_global(Some(Duration::from_secs(60)))
                .build()
                .header("Authorization", &format!("Bearer {}", cfg.api_key.as_deref().unwrap_or("")))
                .send_json(&body)
                .map_err(|e| (0, e.to_string()))?;
            let status = res.status().as_u16();
            if status != 200 {
                return Err((status, format!("HTTP {status}")));
            }
            res.into_body().read_json().map_err(|e| (0, e.to_string()))
        };
        // The first poll shows the link is up at once, with the chatbots that use it.
        let mut up = false;
        if let Ok(r) = post(json!({}), &chats.shared.mode()) {
            up = true;
            tell(LinkEvent::Status("On. Chatbots on your account can use Scout here.".into(), true));
            if let Some(list) = r["chatbots"].as_array() {
                let bots = list.iter().map(|b| Chatbot { name: b["name"].as_str().unwrap_or("").to_string(), seen: b["seen"].as_u64() }).collect();
                tell(LinkEvent::Chatbots(bots));
            }
        }
        let ended = loop {
            if stop.load(Ordering::Relaxed) {
                break "Off.".to_string();
            }
            let mode = chats.shared.mode();
            // Quick polls with a pause the site picks (`next`), rather than one held open: on shared hosting a held request
            // takes one of the few connections the host lets through, and the website waits for them.
            let reply = match post(json!({"results": results}), &mode) {
                Ok(r) => r,
                Err((401, _)) => break "ScoutCode refused this sign-in. Sign in again.".to_string(),
                Err((_, e)) => {
                    // Results that didn't get through are sent again with the next poll.
                    failures += 1;
                    tell(LinkEvent::Status(format!("Can't reach ScoutCode ({e}). Trying again…"), false));
                    let wait = 2u64.saturating_pow(failures.min(5)).min(30);
                    for _ in 0..wait * 10 {
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    continue;
                }
            };
            results.clear();
            if failures > 0 || !up {
                failures = 0;
                up = true;
                tell(LinkEvent::Status("On. Chatbots on your account can use Scout here.".into(), true));
            }
            if let Some(list) = reply["chatbots"].as_array() {
                let bots = list.iter().map(|b| Chatbot { name: b["name"].as_str().unwrap_or("").to_string(), seen: b["seen"].as_u64() }).collect();
                tell(LinkEvent::Chatbots(bots));
            }
            if reply["replaced"] == true {
                break "Another Scout linked to this account, so this one has stopped.".to_string();
            }
            for c in reply["calls"].as_array().cloned().unwrap_or_default() {
                let id = c["id"].as_str().unwrap_or("").to_string();
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let tool = c["name"].as_str().unwrap_or("");
                let Served { result: r, chat } = chats.call(&h, tool, &c["arguments"]);
                tell(LinkEvent::Call(LinkCall {
                    tool: tool.to_string(),
                    label: tools::label(tool, &c["arguments"]),
                    ok: !r.is_error,
                    at: crate::session::now_ms(),
                    edits: if r.is_error { vec![] } else { edits(&r.display) },
                    chat,
                }));
                results.push(json!({"id": id, "output": r.output, "isError": r.is_error}));
            }
            // Results go back straight away. Otherwise wait as long as the site asks (`next`) before looking again.
            if results.is_empty() {
                let wait = crate::remote::next_poll(&reply);
                for _ in 0..wait / 100 {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        };
        let _ = post(json!({"done": true}), "");
        // Stopped from the window, which has moved on and may have started another.
        if !stop.load(Ordering::Relaxed) {
            tell(LinkEvent::Ended(ended));
        }
    });
    link
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One chat's agent on its own, as a chatbot's calls run in it.
    fn agent_in(cwd: &str, mode: &str) -> (Agent, Hooks, Receiver<Ev>) {
        let (chats, h, rx) = bridge(&config::load_config(), cwd, mode);
        (chats.agent(session::new_session(cwd, "m")), h, rx)
    }

    #[test]
    fn serves_the_tools_a_chatbot_can_use() {
        let tools = mcp_tools();
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        for n in ["view", "edit_file", "bash", "grep", "start_chat", "resume_chat", "list_chats", "save_reply"] {
            assert!(names.contains(&n), "{n} is missing");
        }
        for n in NOT_SERVED {
            assert!(!names.contains(&n), "{n} needs Scout's own model");
        }
        assert!(tools.iter().all(|t| t["inputSchema"].is_object()));
        let view = tools.iter().find(|t| t["name"] == "view").unwrap();
        assert!(view["inputSchema"]["required"].as_array().unwrap().contains(&json!("chat_id")));
    }

    fn started(chats: &mut Chats, h: &Hooks, request: &str) -> String {
        let r = chats.call(h, "start_chat", &json!({"request": request}));
        assert!(!r.result.is_error, "{}", r.result.output);
        r.chat.unwrap()
    }

    #[test]
    fn calls_are_saved_in_a_chat_that_can_be_resumed_and_rewound() {
        let dir = std::env::temp_dir().join(format!("scout-mcp-chat-{}", crate::session::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let cwd = dir.to_string_lossy().to_string();
        let (mut chats, h, _rx) = bridge(&config::load_config(), &cwd, "yolo");

        let no_chat = chats.call(&h, "write_file", &json!({"path": "a.txt", "content": "one"}));
        assert!(no_chat.result.is_error && no_chat.result.output.contains("start_chat"), "{}", no_chat.result.output);
        assert!(!dir.join("a.txt").exists());

        let id = started(&mut chats, &h, "make a.txt");
        let w = chats.call(&h, "write_file", &json!({"chat_id": id, "path": "a.txt", "content": "one"}));
        assert!(!w.result.is_error, "{}", w.result.output);
        chats.call(&h, "save_reply", &json!({"chat_id": id, "text": "Made it."}));

        // Another AI, starting afresh, picks it up and sees what happened.
        let (mut other, h2, _rx2) = bridge(&config::load_config(), &cwd, "yolo");
        let listed = other.call(&h2, "list_chats", &json!({})).result.output;
        assert!(listed.contains(&id), "{listed}");
        let r = other.call(&h2, "resume_chat", &json!({"chat_id": id, "request": "change it"}));
        assert!(r.result.output.contains("User: make a.txt") && r.result.output.contains("Made it."), "{}", r.result.output);
        other.call(&h2, "view", &json!({"chat_id": id, "path": "a.txt"}));
        let e = other.call(&h2, "write_file",&json!({"chat_id": id, "path": "a.txt", "content": "two"}));
        assert!(!e.result.is_error, "{}", e.result.output);

        // Opened again, as the app would, it rewinds to before the second message with the file as it was.
        let s = session::load_session(&id).unwrap();
        let at = s.messages.iter().rposition(|m| matches!(m, Message::User { .. })).unwrap();
        let mut agent = Agent::new(config::load_config(), s, shared(&config::load_config(), "default"));
        let (restored, all) = agent.rewind(at);
        assert!(all && restored == vec!["a.txt".to_string()], "{restored:?}");
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "one");
        let _ = session::delete_session(&id);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_only_refuses_edits_and_allows_reads() {
        let dir = std::env::temp_dir().join(format!("scout-mcp-{}", crate::session::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        let cwd = dir.to_string_lossy().to_string();
        let (mut agent, h, _rx) = agent_in(&cwd, "plan");
        let read = call_tool(&mut agent, &h, "1", "view", &json!({"path": "a.txt"}));
        assert!(!read.is_error && read.output.contains("hello"), "{}", read.output);
        let write = call_tool(&mut agent, &h, "2", "write_file", &json!({"path": "b.txt", "content": "no"}));
        assert!(write.is_error && write.output.contains("read-only"), "{}", write.output);
        let run = call_tool(&mut agent, &h, "3", "bash", &json!({"command": "echo no"}));
        assert!(run.is_error, "{}", run.output);
        assert!(!dir.join("b.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_memory_goes_through_permissions_like_an_edit() {
        let dir = std::env::temp_dir().join(format!("scout-memory-agent-{}", crate::session::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let cwd = dir.to_string_lossy().to_string();
        let file = dir.join(".scout").join("memory.md");
        let add = |text: &str| json!({"memory": "project", "action": "add", "section": "Commands", "text": text});

        let (mut agent, h, _rx) = agent_in(&cwd, "plan");
        let refused = call_tool(&mut agent, &h, "1", "update_memory", &add("- one"));
        assert!(refused.is_error && !file.exists(), "{}", refused.output);

        // Asked about with the diff, in the mode that asks about every edit.
        let (mut agent, h, rx) = agent_in(&cwd, "default");
        let answering = std::thread::spawn(move || {
            for ev in rx {
                if let Ev::AskPermission(req, answer) = ev {
                    let _ = answer.send(crate::agent::PermissionAnswer::Allow);
                    return Some((req.tool, matches!(req.preview, Some(tools::Display::Diff { .. }))));
                }
            }
            None
        });
        let saved = call_tool(&mut agent, &h, "2", "update_memory", &add("- two"));
        drop((agent, h));
        assert_eq!(answering.join().unwrap(), Some(("update_memory".to_string(), true)));
        assert!(!saved.is_error, "{}", saved.output);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "## Commands\n- two\n");

        // No question here, and the ordinary file tools can't get a secret in either.
        let (mut agent, h, _rx) = agent_in(&cwd, "acceptEdits");
        assert!(!call_tool(&mut agent, &h, "3", "update_memory", &add("- three")).is_error);
        let path = file.to_string_lossy().to_string();
        let secret = call_tool(&mut agent, &h, "4", "write_file", &json!({"path": path, "content": "ghp_abcdefghijklmnopqrstuvwxyz1234"}));
        assert!(secret.is_error && secret.output.contains("secret"), "{}", secret.output);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "## Commands\n- two\n- three\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
