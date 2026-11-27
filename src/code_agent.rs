//! The Code agent: the built-in agent at the top of the Agents list that starts and keeps track of Code Agents (the
//! app's coding chats) for the user.
//!
//! Its loop runs here in the app, on its own thread, so it can start, message, read and stop the Code Agents directly.
//! Its model calls go to the ScoutCode site (`code_turn` on /api/account/agents), which answers with the admin-set
//! Agent model, billed like every Agent. The model picker in its message box is for the Code Agents it starts, not for
//! it. Its chat is kept on this PC, in `code_agent.json` next to the sessions.
//!
//! Tool calls are run by the core (core.rs, `code_tool`), which owns the chats: the thread sends each one as a
//! `CodeEv::Tool` and waits for the answer.

use crate::agent::{Ev, Hooks};
use crate::config::{Config, home};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::Arc;
use std::time::Duration;

/// Model steps in one turn before it stops and says so.
const MAX_STEPS: usize = 24;
/// Model messages kept for the next turn: older ones are dropped, from a user message on.
const KEEP_HISTORY: usize = 120;
/// Chat messages kept on screen and on disk.
const KEEP_MSGS: usize = 600;

/// One message in the Code agent's chat, as the window shows it.
#[derive(Serialize, Deserialize, Clone)]
pub struct Msg {
    pub id: u64,
    /// "user", "agent", "started" (a Code Agent it started: the widget), "notice" or "error".
    pub role: String,
    pub text: String,
    /// For "started": the Code Agent's chat id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<String>,
    pub at: u64,
}

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    #[serde(default)]
    msgs: Vec<Msg>,
    #[serde(default)]
    history: Vec<Value>,
    #[serde(default)]
    started: Vec<String>,
    #[serde(default)]
    next_id: u64,
    /// The intro and the chats already there were shown once.
    #[serde(default)]
    welcomed: bool,
}

/// What the Code agent's thread tells the core.
pub enum CodeEv {
    /// Something it said, to show now.
    Said(String),
    /// What it's doing, in a few words.
    Status(String),
    /// A tool call for the core to run, and where to send what it returned.
    Tool { name: String, args: Value, reply: Sender<String> },
    /// The turn is over: the model messages it added, and why it stopped early if it did.
    Done(Vec<Value>, Result<(), String>),
}

pub struct CodeAgent {
    s: Saved,
    pub busy: bool,
    pub status: String,
    cancel: Option<Arc<AtomicBool>>,
    /// Code Agents that finished while a turn ran: the next turn hears about them.
    events: Vec<String>,
}

fn file() -> PathBuf {
    home().join("code_agent.json")
}

fn now_ms() -> u64 {
    crate::session::now_ms()
}

impl CodeAgent {
    pub fn load() -> Self {
        let s = std::fs::read_to_string(file()).ok().and_then(|t| serde_json::from_str::<Saved>(&t).ok()).unwrap_or_default();
        CodeAgent { s, busy: false, status: String::new(), cancel: None, events: vec![] }
    }

    fn save(&self) {
        if let Ok(t) = serde_json::to_string(&self.s) {
            let _ = std::fs::write(file(), t);
        }
    }

    fn push(&mut self, role: &str, text: String, chat: Option<String>) {
        self.s.next_id += 1;
        self.s.msgs.push(Msg { id: self.s.next_id, role: role.into(), text, chat, at: now_ms() });
        if self.s.msgs.len() > KEEP_MSGS {
            let cut = self.s.msgs.len() - KEEP_MSGS;
            self.s.msgs.drain(..cut);
        }
    }

    /// The first time it's opened: what it does, and that the chats already here are its Code Agents now (the
    /// migration: every existing chat shows on the right, nothing is moved on disk).
    pub fn welcome(&mut self, existing: usize) {
        if self.s.welcomed {
            return;
        }
        self.s.welcomed = true;
        let mut text = "Hi! I'm your Code agent. Tell me what you want built or fixed, and I'll start Code Agents to do it, \
                        in the folder and with the model you pick below. You'll see each one on the right: press it to \
                        watch it work or talk to it directly."
            .to_string();
        if existing > 0 {
            text.push_str(&format!(
                "\n\nYour {} chat{} from before {} here too, on the right, as Code Agents.",
                existing,
                if existing == 1 { "" } else { "s" },
                if existing == 1 { "is" } else { "are" }
            ));
        }
        self.push("agent", text, None);
        self.save();
    }

    pub fn started(&self) -> &[String] {
        &self.s.started
    }

    pub fn note_started(&mut self, chat: &str, title: &str) {
        if !self.s.started.iter().any(|c| c == chat) {
            self.s.started.push(chat.to_string());
        }
        self.push("started", title.to_string(), Some(chat.to_string()));
        self.save();
    }

    pub fn view(&self) -> Value {
        json!({ "messages": self.s.msgs, "busy": self.busy, "status": self.status, "started": self.s.started })
    }

    pub fn clear(&mut self) {
        self.stop();
        self.s.msgs.clear();
        self.s.history.clear();
        self.events.clear();
        self.save();
    }

    pub fn stop(&mut self) {
        if let Some(c) = &self.cancel {
            c.store(true, Ordering::SeqCst);
        }
    }

    /// The user sent a message. False when there's nothing to send.
    pub fn send(&mut self, text: String, system: String, cfg: &Config, hooks: &Hooks) -> bool {
        let text = text.trim().to_string();
        if text.is_empty() {
            return false;
        }
        self.push("user", text.clone(), None);
        self.save();
        if self.busy {
            // It hears it once the turn that's running is over (see `done`).
            self.events.push(text);
            return true;
        }
        self.s.history.push(json!({ "role": "user", "content": text }));
        self.run(system, cfg, hooks);
        true
    }

    /// A Code Agent it started finished its turn: it tells the user how it went.
    pub fn chat_finished(&mut self, summary: String, system: String, cfg: &Config, hooks: &Hooks) {
        let summary = format!("[Event] {summary}");
        if self.busy {
            self.events.push(summary);
            return;
        }
        self.s.history.push(json!({ "role": "user", "content": summary }));
        self.run(system, cfg, hooks);
    }

    fn run(&mut self, system: String, cfg: &Config, hooks: &Hooks) {
        trim_history(&mut self.s.history);
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        self.busy = true;
        self.status = "Thinking…".into();
        let history = self.s.history.clone();
        let cfg = cfg.clone();
        let hooks = hooks.clone();
        std::thread::spawn(move || {
            let mut added = vec![];
            let r = turn(&cfg, &system, &history, &mut added, &hooks, &cancel);
            hooks.send(Ev::CodeAgent(CodeEv::Done(added, r)));
        });
    }

    pub fn said(&mut self, text: String) {
        self.push("agent", text, None);
        self.save();
    }

    pub fn done(&mut self, added: Vec<Value>, r: Result<(), String>) -> Option<Vec<String>> {
        // A turn the model never answered leaves no question behind: otherwise the next turn answers it instead.
        if r.is_err() && added.is_empty() {
            while self.s.history.last().is_some_and(|m| m["role"] == "user") {
                self.s.history.pop();
            }
        }
        self.s.history.extend(added);
        self.busy = false;
        self.cancel = None;
        self.status.clear();
        if let Err(e) = r {
            if e != "Stopped." {
                self.push("error", e, None);
            }
        }
        self.save();
        // Anything that came in after its last step starts the next turn.
        let left = std::mem::take(&mut self.events);
        (!left.is_empty()).then_some(left)
    }

    pub fn resume_with(&mut self, events: Vec<String>, system: String, cfg: &Config, hooks: &Hooks) {
        self.s.history.push(json!({ "role": "user", "content": events.join("\n\n") }));
        self.run(system, cfg, hooks);
    }
}

/// Keeps the last messages, starting from a user message so no tool result is left without its call.
fn trim_history(h: &mut Vec<Value>) {
    if h.len() <= KEEP_HISTORY {
        return;
    }
    let mut cut = h.len() - KEEP_HISTORY;
    while cut < h.len() && h[cut]["role"] != "user" {
        cut += 1;
    }
    h.drain(..cut);
}

fn tool(name: &str, description: &str, props: Value, required: &[&str]) -> Value {
    json!({ "type": "function", "function": { "name": name, "description": description, "parameters": { "type": "object", "properties": props, "required": required } } })
}

pub fn tools() -> Vec<Value> {
    let id = json!({ "type": "string", "description": "The Code Agent's id, from start_code_agent or list_code_agents." });
    vec![
        tool(
            "start_code_agent",
            "Starts a new Code Agent: a coding agent that works on the user's computer, in the folder and with the model the user picked. Give it the whole task, with everything it needs to know, as if briefing a developer who hasn't seen this chat. Returns its id.",
            json!({ "task": { "type": "string", "description": "The full task for the Code Agent." } }),
            &["task"],
        ),
        tool("list_code_agents", "Lists the user's Code Agents, newest first, with their ids, folders and whether they're working.", json!({ "limit": { "type": "integer", "description": "How many. Default 15." } }), &[]),
        tool("read_code_agent", "What a Code Agent is doing or did: its state, the last thing it was asked and its latest messages.", json!({ "id": id }), &["id"]),
        tool(
            "message_code_agent",
            "Sends a Code Agent a message: a follow-up, a correction or an answer to its question. It's queued if the agent is working.",
            json!({ "id": id, "message": { "type": "string" } }),
            &["id", "message"],
        ),
        tool("stop_code_agent", "Stops a Code Agent that's working.", json!({ "id": id }), &["id"]),
    ]
}

pub fn system_prompt(folder: &str, model: &str, mode: &str) -> String {
    format!(
        "You are the Code agent in the Scout desktop app. You don't do the work yourself and you can't see the files: \
         Code Agents do. They are coding agents that work in the user's folder on this computer, read and change its \
         files and run commands. Your job is to hand each request to one and report back.\n\n\
         - Folder: {folder}\n\
         - Model the Code Agents use: {model}\n\
         - Their permission mode: {mode}\n\
         The user picks these in the message box; you can't change them.\n\
         The user is always asking about the current project, the folder above: read every request as being about it, \
         even when it doesn't say so. Don't ask which project they mean.\n\n\
         How to work:\n\
         - Act, don't explain. For almost every message (a task, a bug, a question about the project or its code, \
         pasted instructions, an error or log), call start_code_agent right away with a complete brief: the user's \
         words, plus anything from this chat it needs. \"This app\", \"the project\" and \"the code\" mean what's in \
         the folder, which only a Code Agent can look at: never answer about it from your own guesses.\n\
         - Don't describe Scout, yourself, or the folder, model and mode settings unless the user asks about Scout itself.\n\
         - Answer directly only for small talk or questions about what you just reported.\n\
         - Ask a short question first only when a Code Agent truly can't start without the answer.\n\
         - To follow up on, correct or continue the work of a Code Agent you started, use message_code_agent instead \
         of starting a new one.\n\
         - After starting or messaging one, say so in one line and stop. You'll get an [Event] message when it \
         finishes: then tell the user what it did or found, in a few lines (read_code_agent if you need more).\n\
         - Big work that splits cleanly into parts that don't touch the same files can go to several Code Agents.\n\
         - [Event] messages come from the app, not the user."
    )
}

fn turn(cfg: &Config, system: &str, history: &[Value], added: &mut Vec<Value>, hooks: &Hooks, cancel: &AtomicBool) -> Result<(), String> {
    let tools = tools();
    for _ in 0..MAX_STEPS {
        if cancel.load(Ordering::SeqCst) {
            return Err("Stopped.".into());
        }
        hooks.send(Ev::CodeAgent(CodeEv::Status("Thinking…".into())));
        let mut messages = vec![json!({ "role": "system", "content": system })];
        messages.extend(history.iter().cloned());
        messages.extend(added.iter().cloned());
        let body = json!({ "action": "code_turn", "messages": messages, "tools": tools });
        let r = crate::delta::site_request(cfg, "POST", "/api/account/agents", Some(body), 300, "Sign in to your ScoutCode account to use the Code agent.")?;
        if cancel.load(Ordering::SeqCst) {
            return Err("Stopped.".into());
        }
        let mut msg = r["message"].clone();
        if !msg.is_object() {
            return Err("The model didn't answer. Try again.".into());
        }
        msg["role"] = json!("assistant");
        let text = msg["content"].as_str().unwrap_or("").trim().to_string();
        if msg["content"].is_null() {
            msg["content"] = json!("");
        }
        let calls = msg["tool_calls"].as_array().cloned().unwrap_or_default();
        added.push(msg);
        if !text.is_empty() {
            hooks.send(Ev::CodeAgent(CodeEv::Said(text)));
        }
        if calls.is_empty() {
            return Ok(());
        }
        for c in calls {
            let id = c["id"].as_str().unwrap_or("").to_string();
            let name = c["function"]["name"].as_str().unwrap_or("").to_string();
            let args = c["function"]["arguments"].as_str().and_then(|a| serde_json::from_str::<Value>(a).ok()).unwrap_or_else(|| c["function"]["arguments"].clone());
            let out = if cancel.load(Ordering::SeqCst) {
                "Stopped by the user.".to_string()
            } else {
                let (tx, rx) = channel();
                hooks.send(Ev::CodeAgent(CodeEv::Tool { name, args, reply: tx }));
                rx.recv_timeout(Duration::from_secs(60)).unwrap_or_else(|_| "The app didn't answer.".into())
            };
            added.push(json!({ "role": "tool", "tool_call_id": id, "content": out }));
        }
    }
    Err(format!("I stopped after {MAX_STEPS} steps. Send me a message to carry on."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trimmed_history_starts_at_a_user_message() {
        let mut h: Vec<Value> = (0..200).map(|i| json!({ "role": if i % 3 == 0 { "user" } else { "tool" }, "content": i.to_string() })).collect();
        trim_history(&mut h);
        assert!(h.len() <= KEEP_HISTORY);
        assert_eq!(h[0]["role"], "user");
    }

    #[test]
    fn every_tool_has_a_name_and_parameters() {
        for t in tools() {
            assert!(t["function"]["name"].is_string());
            assert_eq!(t["function"]["parameters"]["type"], "object");
        }
    }
}
