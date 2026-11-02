//! Lets a chatbot use Scout's tools over MCP: the chatbot's model does the thinking, and the files and shell are the ones
//! on this computer, under Scout's permission modes. `Scout.exe mcp` serves an app on this computer over stdio. The link
//! in Settings > Chatbots serves web chatbots through the site, which it long-polls for calls (see src/lib/mcp-relay.ts
//! on the server), so this computer takes no incoming connections.

use crate::agent::{Agent, Ev, Hooks, PermissionAnswer, PermissionRequest, Shared, cap_output};
use crate::api::{FunctionCall, ToolCall, http};
use crate::config::{self, Config};
use crate::tools::{self, EditTool, ToolResult};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Everything but what needs Scout's own model or conversation.
const NOT_SERVED: [&str; 5] = ["todo_write", "task", "present_plan", "ask_questions", "update_memory"];
pub const PROTOCOLS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

fn mcp_tools() -> Vec<Value> {
    tools::specs(None, EditTool::StrReplace)
        .iter()
        .map(|s| &s["function"])
        .filter(|f| !NOT_SERVED.contains(&f["name"].as_str().unwrap_or("")))
        .map(|f| json!({"name": f["name"], "description": f["description"], "inputSchema": f["parameters"]}))
        .collect()
}

/// An agent that only runs tool calls, with the channel its events arrive on. Whoever holds the channel answers its
/// permission questions.
fn bridge(cfg: &Config, cwd: &str, mode: &str) -> (Agent, Hooks, Receiver<Ev>) {
    let shared = Arc::new(Shared {
        mode: Mutex::new(mode.to_string()),
        model: Mutex::new(cfg.model.clone()),
        provider: Mutex::new(cfg.provider.clone()),
        effort: Mutex::new("none".into()),
        cancel: AtomicBool::new(false),
        edit_tools: Default::default(),
        model_names: Default::default(), no_vision: Default::default(),
        steer: Default::default(),
    });
    let mut agent = Agent::new(cfg.clone(), crate::session::new_session(cwd, &cfg.model), shared);
    agent.read_only_note = "Scout is read-only here. To allow edits and commands, the user has to choose another permission mode.";
    let (tx, rx) = channel();
    (agent, Hooks { chat: None, tx, ctx: crate::waker::Waker::default() }, rx)
}

fn call_tool(agent: &mut Agent, h: &Hooks, id: &str, name: &str, args: &Value) -> ToolResult {
    let call = ToolCall { id: id.to_string(), kind: "function".into(), extra_content: None, function: FunctionCall { name: name.to_string(), arguments: args.to_string() } };
    let mut r = agent.run_tool(&call, h);
    r.output = cap_output(r.output);
    r
}

/// `Scout.exe mcp [--cwd <folder>] [--mode <mode>]`: serves MCP over stdin and stdout until the app that started it
/// closes them. Read-only unless --mode says otherwise. Nothing here can ask, so what the mode doesn't allow is refused.
pub fn serve_stdio(args: &[String]) {
    let flag = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).cloned();
    let cwd = flag("--cwd").or_else(|| std::env::current_dir().ok().map(|d| d.to_string_lossy().into_owned())).unwrap_or_default();
    let mode = flag("--mode").filter(|m| config::MODES.contains(&m.as_str())).unwrap_or_else(|| "plan".into());
    let (mut agent, h, rx) = bridge(&config::load_config(), &cwd, &mode);
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
            Ok(msg) => handle(&msg, &cwd, &tools, &mut agent, &h),
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
fn handle(msg: &Value, cwd: &str, tools: &[Value], agent: &mut Agent, h: &Hooks) -> Option<Value> {
    let id = msg.get("id")?;
    let ok = |result: Value| Some(json!({"jsonrpc": "2.0", "id": id, "result": result}));
    let fail = |code: i32, message: String| Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}));
    let method = msg["method"].as_str().unwrap_or("");
    match method {
        "initialize" => {
            let asked = msg["params"]["protocolVersion"].as_str().unwrap_or("");
            ok(json!({
                "protocolVersion": if PROTOCOLS.contains(&asked) { asked } else { PROTOCOLS[0] },
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "scout", "title": "Scout", "version": crate::update::VERSION},
                "instructions": format!("These tools run on the user's computer through Scout, in {cwd}. Read a file before you edit it."),
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
            let r = call_tool(agent, h, &id.to_string(), name, &args);
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
    let (mut agent, h, rx) = bridge(cfg, cwd, mode);
    let stop = Arc::new(AtomicBool::new(false));
    let link = Link { stop: stop.clone(), shared: agent.shared.clone(), cwd: cwd.to_string() };
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
        let ended = loop {
            if stop.load(Ordering::Relaxed) {
                break "Off.".to_string();
            }
            let mode = agent.shared.mode();
            let reply = match post(json!({"wait": true, "results": results}), &mode) {
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
            if failures > 0 || reply.get("calls").is_some() {
                failures = 0;
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
                let r = call_tool(&mut agent, &h, &id, tool, &c["arguments"]);
                tell(LinkEvent::Call(LinkCall {
                    tool: tool.to_string(),
                    label: tools::label(tool, &c["arguments"]),
                    ok: !r.is_error,
                    at: crate::session::now_ms(),
                    edits: if r.is_error { vec![] } else { edits(&r.display) },
                }));
                results.push(json!({"id": id, "output": r.output, "isError": r.is_error}));
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

    #[test]
    fn serves_the_tools_a_chatbot_can_use() {
        let tools = mcp_tools();
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        for n in ["view", "edit_file", "bash", "grep"] {
            assert!(names.contains(&n), "{n} is missing");
        }
        for n in NOT_SERVED {
            assert!(!names.contains(&n), "{n} needs Scout's own model");
        }
        assert!(tools.iter().all(|t| t["inputSchema"].is_object()));
    }

    #[test]
    fn read_only_refuses_edits_and_allows_reads() {
        let dir = std::env::temp_dir().join(format!("scout-mcp-{}", crate::session::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        let cwd = dir.to_string_lossy().to_string();
        let (mut agent, h, _rx) = bridge(&config::load_config(), &cwd, "plan");
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

        let (mut agent, h, _rx) = bridge(&config::load_config(), &cwd, "plan");
        let refused = call_tool(&mut agent, &h, "1", "update_memory", &add("- one"));
        assert!(refused.is_error && !file.exists(), "{}", refused.output);

        // Asked about with the diff, in the mode that asks about every edit.
        let (mut agent, h, rx) = bridge(&config::load_config(), &cwd, "default");
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
        let (mut agent, h, _rx) = bridge(&config::load_config(), &cwd, "acceptEdits");
        assert!(!call_tool(&mut agent, &h, "3", "update_memory", &add("- three")).is_error);
        let path = file.to_string_lossy().to_string();
        let secret = call_tool(&mut agent, &h, "4", "write_file", &json!({"path": path, "content": "ghp_abcdefghijklmnopqrstuvwxyz1234"}));
        assert!(secret.is_error && secret.output.contains("secret"), "{}", secret.output);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "## Commands\n- two\n- three\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
