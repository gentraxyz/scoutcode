use crate::api::{self, ApiError, ChatRequest, Content, Delta, Message, ModelInfo, StreamResult, ToolCall};
use crate::config::{self, Config};
use crate::prompt;
use crate::session::{Session, Totals, save_session};
use crate::tools::{self, Ctx, Display, EditTool, Permission, Todo, ToolResult};
use serde_json::{Value, json};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct PermissionRequest {
    pub tool: String,
    pub label: String,
    pub preview: Option<Display>,
    /// The command prefix "always" would allow, for bash.
    pub prefix: Option<String>,
    /// Why auto mode held it for review.
    pub reason: Option<String>,
}

pub enum PermissionAnswer {
    Allow,
    Always,
    Deny(String),
}

pub enum PlanAnswer {
    Approve(String),
    Revise(String),
}

/// One option of an `ask_questions` question. `description` says what picking it means.
#[derive(Clone)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

/// A question the model asks the user with `ask_questions`.
#[derive(Clone)]
pub struct Question {
    pub question: String,
    /// A very short label shown above the question.
    pub header: String,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
}

pub enum QuestionAnswer {
    /// One answer per question, in order.
    Answered(Vec<String>),
    Skipped,
}

/// Everything the worker thread tells the window. Each send wakes the UI.
pub enum Ev {
    RequestStart,
    Text(String),
    Reasoning(String),
    AssistantDone,
    ToolCall { id: String, name: String, label: String },
    ToolProgress { id: String, text: String },
    ToolResult { id: String, result: ToolResult },
    Status(String),
    Notice(String),
    Todos(Vec<Todo>),
    Totals(Totals, u64),
    AskPermission(PermissionRequest, Sender<PermissionAnswer>),
    AskPlan(String, Sender<PlanAnswer>),
    AskQuestions(Vec<Question>, Sender<QuestionAnswer>),
    /// The project's files for the @ picker: the folder, and its files relative to it.
    FileIndex(String, Vec<String>),
    /// Chats matching a sidebar search: the query, and their ids.
    ChatSearch(String, Vec<String>),
    /// Messages sent while Scout worked, now in the conversation: each one's text and attached files.
    Steered(Vec<(String, Vec<String>)>),
    Finished(Box<Agent>, Result<(), String>),
    Models(Result<Vec<ModelInfo>, String>),
    /// The account the key belongs to, from the model list.
    User(String),
    LoginUrl(String),
    /// The token and, when the server sent it, the username.
    Login(Result<(String, Option<String>), String>),
    Voice(crate::voice::Update),
}

#[derive(Clone)]
pub struct Hooks {
    pub tx: Sender<Ev>,
    pub ctx: eframe::egui::Context,
}

impl Hooks {
    pub fn send(&self, ev: Ev) {
        let _ = self.tx.send(ev);
        self.ctx.request_repaint();
    }
}

/// Settings the window can change while a turn runs.
pub struct Shared {
    pub mode: Mutex<String>,
    /// The id to send to `provider`.
    pub model: Mutex<String>,
    /// "scoutcode" or "openrouter".
    pub provider: Mutex<String>,
    pub effort: Mutex<String>,
    pub cancel: AtomicBool,
    /// Each model's edit tool, from /v1/models once the window has the list.
    pub edit_tools: Mutex<HashMap<String, EditTool>>,
    /// Messages to add to the running turn at its next step.
    pub steer: Mutex<Vec<Steer>>,
}

/// A message sent into a running turn.
pub struct Steer {
    pub text: String,
    pub files: Vec<String>,
    /// The text with the files' contents, as the model gets it.
    pub content: String,
    pub read: Vec<PathBuf>,
}

impl Shared {
    /// The edit tools that suit `model`; str_replace until the server says otherwise.
    pub fn edit_tool(&self, model: &str) -> EditTool {
        self.edit_tools.lock().unwrap().get(model).copied().unwrap_or_default()
    }

    pub fn mode(&self) -> String {
        self.mode.lock().unwrap().clone()
    }
}

pub struct Checkpoint {
    pub at: usize,
    pub label: String,
    pub files: HashMap<PathBuf, Option<String>>,
}

const SAFE_PREFIXES: [&str; 13] = ["git status", "git diff", "git log", "git show", "git branch", "git remote -v", "git rev-parse", "ls", "pwd", "echo", "which", "whoami", "date"];
const PREFIX_TOOLS: [&str; 17] = ["npm", "npx", "pnpm", "yarn", "bun", "git", "cargo", "go", "python", "python3", "pip", "uv", "make", "docker", "deno", "dotnet", "gh"];

/// Commands chained with operators never match an allow rule, so "npm test && rm -rf x" needs its own approval.
fn has_operators(cmd: &str) -> bool {
    cmd.contains([';', '&', '|', '`', '<', '>']) || cmd.contains("$(")
}

/// The files an edit call writes: its path, or every file an apply_patch names.
fn edit_targets(args: &Value) -> Vec<String> {
    if let Some(p) = args.get("path").and_then(Value::as_str) {
        return vec![p.to_string()];
    }
    let patch = args.get("input").or_else(|| args.get("patch")).and_then(Value::as_str).unwrap_or("");
    let heads = ["*** Add File: ", "*** Update File: ", "*** Delete File: ", "*** Move to: "];
    patch.lines().filter_map(|l| heads.iter().find_map(|h| l.strip_prefix(h)).map(|p| p.trim().to_string())).collect()
}

/// Whether `p` (relative to `cwd` or absolute) is one of the project's own files: inside `cwd` and not in .git.
fn inside_project(cwd: &str, p: &str) -> bool {
    let joined = std::path::Path::new(cwd).join(p);
    let mut parts = Vec::new();
    for c in joined.components() {
        match c {
            std::path::Component::ParentDir => _ = parts.pop(),
            std::path::Component::CurDir => {}
            c => parts.push(c),
        }
    }
    let full: PathBuf = parts.iter().collect();
    match full.strip_prefix(cwd) {
        Ok(rel) => rel.components().next().is_none_or(|c| c.as_os_str() != ".git"),
        Err(_) => false,
    }
}

fn is_safe(cmd: &str) -> bool {
    SAFE_PREFIXES.iter().any(|p| cmd == *p || cmd.starts_with(&format!("{p} ")))
}

pub fn command_prefix(cmd: &str) -> String {
    let w: Vec<&str> = cmd.split_whitespace().collect();
    if w.len() > 1 && PREFIX_TOOLS.contains(&w[0]) && !w[1].starts_with('-') {
        if w[0] == "npm" && w[1] == "run" && w.len() > 2 {
            return w[..3].join(" ");
        }
        return w[..2].join(" ");
    }
    w.first().copied().unwrap_or("").to_string()
}

fn estimate_tokens(m: &[Message]) -> u64 {
    serde_json::to_string(m).map(|s| s.len() as u64 / 4).unwrap_or(0)
}

/// Tool output above this many characters is cut in the middle before it enters the conversation.
const MAX_TOOL_CHARS: usize = 40_000;
/// Tool results shorter than this are never cleared: the stub would save next to nothing.
const MIN_PRUNE_CHARS: usize = 600;
/// The newest tool results are never cleared, since the model is usually still working with them.
const KEEP_RECENT_RESULTS: usize = 6;
/// Providers drop a cached prefix after a few idle minutes; editing history after that costs no extra cache miss.
const CACHE_TTL: Duration = Duration::from_secs(300);
const STUB: &str = "[Output cleared to save context";

fn cap_output(s: String) -> String {
    let n = s.chars().count();
    if n <= MAX_TOOL_CHARS {
        return s;
    }
    let half = MAX_TOOL_CHARS / 2;
    let head: String = s.chars().take(half).collect();
    let tail: String = s.chars().skip(n - half).collect();
    format!("{head}\n\n… [{} characters cut to save context; narrow the request to see this part] …\n\n{tail}", n - MAX_TOOL_CHARS)
}

fn args_of(c: &ToolCall) -> Value {
    serde_json::from_str(&c.function.arguments).unwrap_or(json!({}))
}

pub struct Agent {
    pub cfg: Config,
    pub session: Session,
    pub shared: Arc<Shared>,
    pub read_files: HashSet<PathBuf>,
    /// One per user message: where the conversation was and each edited file's content before the edit.
    pub checkpoints: Vec<Checkpoint>,
    pub todos: Vec<Todo>,
    pub last_prompt_tokens: u64,
    last_request_at: Option<Instant>,
    sys: Option<(String, EditTool, Message)>,
    /// The tool list for the edit tool it was built for.
    specs: (EditTool, Vec<Value>),
    read_only_specs: Vec<Value>,
}

impl Agent {
    pub fn new(cfg: Config, session: Session, shared: Arc<Shared>) -> Self {
        Agent {
            cfg,
            session,
            shared,
            read_files: HashSet::new(),
            checkpoints: vec![],
            todos: vec![],
            last_prompt_tokens: 0,
            last_request_at: None,
            sys: None,
            specs: (EditTool::StrReplace, tools::specs(None, EditTool::StrReplace)),
            read_only_specs: tools::specs(Some(&tools::READ_ONLY), EditTool::StrReplace),
        }
    }

    fn cancelled(&self) -> bool {
        self.shared.cancel.load(Ordering::Relaxed)
    }

    /// Built once and then sent unchanged, so providers that cache prompt prefixes bill the repeat as cached input.
    fn system(&mut self) -> Message {
        let edit = self.shared.edit_tool(&self.shared.model.lock().unwrap());
        if self.sys.as_ref().is_none_or(|(cwd, e, _)| *cwd != self.session.cwd || *e != edit) {
            let msg = Message::System { content: prompt::system_prompt(&self.session.cwd, edit) };
            self.sys = Some((self.session.cwd.clone(), edit, msg));
        }
        self.sys.as_ref().unwrap().2.clone()
    }

    fn add_usage(&mut self, r: &StreamResult, main: bool, h: &Hooks) {
        crate::usage::record(r.usage.as_ref().map_or(0, |u| u.prompt + u.completion), r.meta.cost.unwrap_or(0.0));
        let t = &mut self.session.totals;
        t.requests += 1;
        if let Some(u) = &r.usage {
            t.prompt_tokens += u.prompt;
            t.completion_tokens += u.completion;
            t.cached_tokens += u.cached;
            // Only the main conversation's requests say how full its context is.
            if main {
                self.last_prompt_tokens = u.prompt + u.completion;
            }
        }
        if let Some(c) = r.meta.cost.filter(|c| c.is_finite()) {
            t.cost += c;
        }
        if let Some(b) = r.meta.balance.filter(|b| b.is_finite()) {
            t.balance = Some(b);
        }
        h.send(Ev::Totals(self.session.totals.clone(), self.context_tokens()));
    }

    /// One streamed request with retries for rate limits, 5xx and network errors.
    fn request(&mut self, messages: &[Message], read_only: bool, quiet: bool, model: Option<String>, no_tools: bool, h: &Hooks) -> Result<StreamResult, ApiError> {
        let model = model.unwrap_or_else(|| self.shared.model.lock().unwrap().clone());
        let provider = self.shared.provider.lock().unwrap().clone();
        // OpenRouter tries the user's keys in order. A key that fails hands the request to the next one.
        let mut cfgs: Vec<Config> = if provider == "openrouter" {
            self.cfg
                .openrouter_keys
                .iter()
                .map(|k| self.cfg.for_openrouter_key(&k.key))
                .collect()
        } else {
            vec![]
        };
        if cfgs.is_empty() {
            cfgs.push(self.cfg.for_provider(&provider));
        }
        let mut key = 0;
        let effort = self.shared.effort.lock().unwrap().clone();
        let edit = self.shared.edit_tool(&model);
        if self.specs.0 != edit {
            self.specs = (edit, tools::specs(None, edit));
        }
        let tools = if read_only { &self.read_only_specs } else { &self.specs.1 };
        for attempt in 0.. {
            if !quiet {
                h.send(Ev::RequestStart);
            }
            let req = ChatRequest {
                model: &model,
                messages,
                tools: Some(tools),
                tool_choice_none: no_tools,
                temperature: self.cfg.temperature,
                effort: Some(effort.as_str()),
            };
            // Whether any of the answer has been shown: once it has, another key would repeat it, so it isn't retried.
            let shown = Cell::new(false);
            let mut on = |d: Delta| {
                shown.set(true);
                if quiet {
                    return;
                }
                match d {
                    Delta::Text(t) => h.send(Ev::Text(t.to_string())),
                    Delta::Reasoning(t) => h.send(Ev::Reasoning(t.to_string())),
                }
            };
            match api::stream_chat(&cfgs[key], req, &mut on, &self.shared.cancel) {
                Ok(r) => {
                    self.add_usage(&r, !quiet, h);
                    if !quiet {
                        self.last_request_at = Some(Instant::now());
                    }
                    return Ok(r);
                }
                Err(e) => {
                    if self.cancelled() {
                        return Err(ApiError::aborted());
                    }
                    if key + 1 < cfgs.len() && !shown.get() {
                        h.send(Ev::Status(format!("{} · trying the next OpenRouter key", e.message.chars().take(120).collect::<String>())));
                        key += 1;
                        continue;
                    }
                    if !e.retryable() || attempt >= 4 {
                        return Err(e);
                    }
                    let wait = e.retry_after.unwrap_or(2u64.pow(attempt + 1)).min(60);
                    h.send(Ev::Status(format!("{} · retrying in {wait}s ({}/4)", e.message.chars().take(120).collect::<String>(), attempt + 1)));
                    let until = Instant::now() + Duration::from_secs(wait);
                    while Instant::now() < until {
                        if self.cancelled() {
                            return Err(ApiError::aborted());
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            }
        }
        unreachable!()
    }

    fn set_mode(&self, mode: &str, h: &Hooks) {
        *self.shared.mode.lock().unwrap() = mode.to_string();
        h.ctx.request_repaint();
    }

    fn permit(&mut self, name: &str, args: &Value, h: &Hooks) -> PermissionAnswer {
        let mode = self.shared.mode();
        let perm = tools::permission(name);
        if perm == Permission::Read || mode == "yolo" {
            return PermissionAnswer::Allow;
        }
        if mode == "plan" {
            return PermissionAnswer::Deny("Plan mode is read-only. Call present_plan with your plan first.".into());
        }
        if perm == Permission::Edit && mode == "acceptEdits" {
            return PermissionAnswer::Allow;
        }
        let mut reason = None;
        if perm == Permission::Edit && mode == "auto" {
            match edit_targets(args).into_iter().find(|p| !inside_project(&self.session.cwd, p)) {
                None => return PermissionAnswer::Allow,
                Some(p) => reason = Some(format!("It writes {p}, outside the project's own files.")),
            }
        }
        let mut prefix = None;
        if name == "bash" {
            let cmd = args.get("command").and_then(Value::as_str).unwrap_or("").trim().to_string();
            if !has_operators(&cmd) {
                if is_safe(&cmd) {
                    return PermissionAnswer::Allow;
                }
                let rules = self.cfg.allow.get(&self.session.cwd).cloned().unwrap_or_default();
                if rules.iter().any(|r| cmd == *r || cmd.starts_with(&format!("{r} "))) {
                    return PermissionAnswer::Allow;
                }
                prefix = Some(command_prefix(&cmd));
            }
            if mode == "auto" {
                h.send(Ev::Status("Checking the command with Jev…".into()));
                match api::review_command(&self.cfg, &cmd, &self.session.cwd, tools::shell_name()) {
                    Ok(None) => return PermissionAnswer::Allow,
                    Ok(Some(r)) => reason = Some(r),
                    Err(e) => reason = Some(format!("The safety check failed ({}), so it needs your OK.", e.message.chars().take(120).collect::<String>())),
                }
            }
        }
        let mut noop = |_: &str| {};
        let ctx = Ctx { cwd: &self.session.cwd, cancel: &self.shared.cancel, read_files: &mut self.read_files, on_progress: &mut noop };
        let preview = tools::preview(name, args, &ctx);
        let (tx, rx) = channel();
        h.send(Ev::AskPermission(PermissionRequest { tool: name.to_string(), label: tools::label(name, args), preview, prefix: prefix.clone(), reason }, tx));
        // A dropped sender means the window was closed or the turn stopped.
        let answer = rx.recv().unwrap_or(PermissionAnswer::Deny(String::new()));
        if let PermissionAnswer::Always = answer {
            if perm == Permission::Edit {
                self.set_mode("acceptEdits", h);
            } else if let Some(p) = prefix {
                let rules = self.cfg.allow.entry(self.session.cwd.clone()).or_default();
                if !rules.contains(&p) {
                    rules.push(p);
                }
                config::save_one("allow", json!(self.cfg.allow));
            }
        }
        answer
    }

    fn run_tool(&mut self, call: &ToolCall, h: &Hooks) -> ToolResult {
        let name = call.function.name.as_str();
        let id = call.id.clone();
        let Ok(args) = serde_json::from_str::<Value>(if call.function.arguments.is_empty() { "{}" } else { &call.function.arguments }) else {
            h.send(Ev::ToolCall { id: id.clone(), name: name.into(), label: String::new() });
            let r = ToolResult::err(format!("The arguments for {name} were not valid JSON. Send the call again with valid JSON."), "invalid arguments");
            h.send(Ev::ToolResult { id, result: r.clone() });
            return r;
        };
        let (name, args) = tools::canonical(name, args);
        let name = name.as_str();
        h.send(Ev::ToolCall { id: id.clone(), name: name.into(), label: tools::label(name, &args) });
        let result = self.run_tool_inner(name, &args, &id, h);
        h.send(Ev::ToolResult { id, result: result.clone() });
        result
    }

    fn run_tool_inner(&mut self, name: &str, args: &Value, id: &str, h: &Hooks) -> ToolResult {
        if !tools::known(name) {
            let names: Vec<String> = self.specs.1.iter().filter_map(|s| s["function"]["name"].as_str().map(String::from)).collect();
            return ToolResult { output: format!("Unknown tool {name}. Available: {}", names.join(", ")), is_error: true, ..Default::default() };
        }
        if name == "present_plan" {
            if self.shared.mode() != "plan" {
                return ToolResult::err("present_plan is only for plan mode, which is off. Carry on with the task.", "not in plan mode");
            }
            let (tx, rx) = channel();
            h.send(Ev::AskPlan(args.get("plan").and_then(Value::as_str).unwrap_or("").to_string(), tx));
            return match rx.recv().unwrap_or(PlanAnswer::Revise(String::new())) {
                PlanAnswer::Approve(mode) => {
                    self.set_mode(&mode, h);
                    ToolResult::ok("The user approved the plan. Plan mode is off: carry it out now, keeping the todo list up to date.", "approved")
                }
                PlanAnswer::Revise(fb) => ToolResult::err(
                    format!("The user did not approve the plan.{} Stay in plan mode and revise it.", if fb.is_empty() { String::new() } else { format!(" Their feedback: {fb}") }),
                    "not approved",
                ),
            };
        }
        if name == "ask_questions" {
            let questions = parse_questions(args);
            if questions.is_empty() {
                return ToolResult::err("ask_questions needs at least one question with a text.", "invalid");
            }
            let (tx, rx) = channel();
            h.send(Ev::AskQuestions(questions.clone(), tx));
            return match rx.recv().unwrap_or(QuestionAnswer::Skipped) {
                QuestionAnswer::Answered(answers) => {
                    let lines: Vec<String> = questions.iter().zip(&answers).map(|(q, a)| format!("{}: {a}", q.question)).collect();
                    ToolResult::ok(format!("The user answered:\n{}", lines.join("\n")), "answered")
                }
                QuestionAnswer::Skipped => ToolResult::err("The user skipped the questions. Carry on with your best judgment and say what you assumed.", "skipped"),
            };
        }
        if name == "todo_write" {
            let Some(list) = args.get("todos").and_then(Value::as_array) else {
                return ToolResult::err("todos must be an array.", "invalid");
            };
            let todos: Vec<Todo> = list
                .iter()
                .map(|t| {
                    let st = t.get("status").and_then(Value::as_str).unwrap_or("pending");
                    Todo {
                        content: t.get("content").and_then(Value::as_str).unwrap_or("").to_string(),
                        status: if ["pending", "in_progress", "completed"].contains(&st) { st.into() } else { "pending".into() },
                    }
                })
                .collect();
            let done = todos.iter().filter(|t| t.status == "completed").count();
            self.todos = todos.clone();
            h.send(Ev::Todos(todos.clone()));
            return ToolResult { output: "Todo list updated.".into(), is_error: false, summary: Some(format!("{done}/{} done", todos.len())), display: Some(Display::Todos(todos)) };
        }
        if name == "task" {
            let d = args.get("description").and_then(Value::as_str).unwrap_or("task").to_string();
            let p = args.get("prompt").and_then(Value::as_str).unwrap_or("").to_string();
            return match self.run_subagent(&d, &p, h) {
                Ok(a) => ToolResult::ok(if a.is_empty() { "(the sub-agent returned nothing)".into() } else { a }, "done"),
                Err(e) => ToolResult::err(format!("Error: {e}"), e.message.chars().take(100).collect::<String>()),
            };
        }

        match self.permit(name, args, h) {
            PermissionAnswer::Deny(fb) => {
                return ToolResult::err(
                    format!(
                        "The user denied this {name} call.{}",
                        if fb.is_empty() { " Do not retry it unchanged; ask the user how to proceed if you need it.".to_string() } else { format!(" They said: {fb}") }
                    ),
                    "denied",
                );
            }
            _ => {}
        }
        if tools::permission(name) == Permission::Edit {
            let paths: Vec<PathBuf> = tools::edit_paths(name, args).iter().filter_map(|p| tools::resolve(&self.session.cwd, p).ok()).collect();
            for p in paths {
                self.snapshot(p);
            }
        }
        let cwd = self.session.cwd.clone();
        let mut progress = |t: &str| h.send(Ev::ToolProgress { id: id.to_string(), text: t.to_string() });
        let mut ctx = Ctx { cwd: &cwd, cancel: &self.shared.cancel, read_files: &mut self.read_files, on_progress: &mut progress };
        match tools::run(name, args, &mut ctx) {
            Ok(r) => r,
            Err(_) if self.cancelled() => ToolResult::err("Interrupted by the user.", "interrupted"),
            Err(e) => {
                let first = e.lines().next().unwrap_or("").chars().take(100).collect::<String>();
                ToolResult::err(e, first)
            }
        }
    }

    /// A read-only agent with its own context. Its tool calls show as status lines.
    fn run_subagent(&mut self, description: &str, task: &str, h: &Hooks) -> Result<String, ApiError> {
        let mut messages = vec![Message::System { content: prompt::subagent_prompt(&self.session.cwd) }, Message::User { content: Content::Text(task.into()) }];
        // A browser sign-in may only use the approved models, which the sub-agent model may not be, and OpenRouter has
        // none of ours; use the session's model for both.
        let on_scout = *self.shared.provider.lock().unwrap() == "scoutcode";
        let model = (on_scout && !config::is_sign_in(self.cfg.api_key.as_deref()) && !self.cfg.subagent_model.is_empty()).then(|| self.cfg.subagent_model.clone());
        let cwd = self.session.cwd.clone();
        for _ in 0..60 {
            let r = self.request(&messages, true, true, model.clone(), false, h)?;
            let calls = r.tool_calls.clone();
            messages.push(Message::Assistant { content: (!r.content.is_empty()).then(|| r.content.clone()), tool_calls: (!calls.is_empty()).then(|| calls.clone()), reasoning_content: None });
            if calls.is_empty() {
                return Ok(r.content);
            }
            for c in calls {
                let (n, args) = tools::canonical(&c.function.name, args_of(&c));
                let n = n.as_str();
                let out = if !tools::READ_ONLY.contains(&n) {
                    format!("Error: Unknown tool {n}.")
                } else {
                    h.send(Ev::Status(format!("task: {description} · {n} {}", tools::label(n, &args)).chars().take(140).collect()));
                    let mut noop = |_: &str| {};
                    let mut ctx = Ctx { cwd: &cwd, cancel: &self.shared.cancel, read_files: &mut self.read_files, on_progress: &mut noop };
                    match tools::run(n, &args, &mut ctx) {
                        Ok(r) => r.output,
                        Err(e) => format!("Error: {e}"),
                    }
                };
                messages.push(Message::Tool { tool_call_id: c.id, content: out });
            }
        }
        Ok("The sub-agent stopped after 60 steps without a final answer.".into())
    }

    /// Tells the model the mode when it changed, at the end of the newest message: earlier messages stay byte-identical.
    fn announce_mode(&mut self) {
        let mode = self.shared.mode();
        if self.session.mode.as_deref() == Some(mode.as_str()) {
            return;
        }
        match self.session.messages.last_mut() {
            Some(Message::User { content: Content::Text(t) }) | Some(Message::Tool { content: t, .. }) => t.push_str(&prompt::mode_note(&mode)),
            Some(Message::User { content: Content::Parts(p) }) => p.push(json!({"type":"text","text":prompt::mode_note(&mode).trim()})),
            _ => return,
        }
        self.session.mode = Some(mode);
    }

    /// Roughly what the next request will send.
    pub fn context_tokens(&self) -> u64 {
        self.last_prompt_tokens.max(estimate_tokens(&self.session.messages))
    }

    fn should_compact(&self) -> bool {
        self.cfg.auto_compact_tokens > 0 && self.context_tokens() > self.cfg.auto_compact_tokens
    }

    /// Replaces old tool output with a one-line stub, stale views first, then the oldest, until the context is down to `target`.
    /// Nothing changes unless that frees at least `min_gain` tokens, since any edit voids the provider's cached prefix.
    pub fn prune(&mut self, target: u64, min_gain: u64) -> Option<(usize, u64)> {
        let cwd = self.session.cwd.clone();
        let calls: HashMap<String, ToolCall> = self
            .session
            .messages
            .iter()
            .filter_map(|m| if let Message::Assistant { tool_calls: Some(c), .. } = m { Some(c.clone()) } else { None })
            .flatten()
            .map(|c| (c.id.clone(), c))
            .collect();
        // (message index, call)
        let results: Vec<(usize, Option<&ToolCall>)> = self
            .session
            .messages
            .iter()
            .enumerate()
            .filter_map(|(i, m)| if let Message::Tool { tool_call_id, .. } = m { Some((i, calls.get(tool_call_id))) } else { None })
            .collect();
        let content = |i: usize| match &self.session.messages[i] {
            Message::Tool { content, .. } => content.as_str(),
            _ => "",
        };
        let viewed = |c: Option<&ToolCall>| c.and_then(|c| tools::viewed_path(&c.function.name, &args_of(c), &cwd));

        let mut seen = HashSet::new();
        let mut stale = vec![];
        for &(i, c) in results.iter().rev() {
            let Some(file) = viewed(c) else { continue };
            if content(i).starts_with(STUB) {
                continue;
            }
            let a = c.map(args_of).unwrap_or_default();
            let start = ["start", "offset", "line"].iter().find_map(|k| a.get(*k).and_then(Value::as_u64)).unwrap_or(1);
            if !seen.insert((file, start)) {
                stale.push(i);
            }
        }
        let recent: HashSet<usize> = results.iter().rev().take(KEEP_RECENT_RESULTS).map(|r| r.0).collect();
        let mut candidates = stale.clone();
        candidates.extend(results.iter().map(|r| r.0).filter(|i| !stale.contains(i) && !recent.contains(i)));
        candidates.retain(|&i| content(i).len() >= MIN_PRUNE_CHARS && !content(i).starts_with(STUB));

        let used = self.context_tokens();
        let by_index: HashMap<usize, Option<&ToolCall>> = results.iter().copied().collect();
        let mut plan = vec![];
        let mut freed = 0u64;
        for i in candidates {
            if used.saturating_sub(freed) <= target {
                break;
            }
            let c = by_index[&i];
            let name = c.map(|c| c.function.name.clone()).unwrap_or_else(|| "tool".into());
            let label = c.map(|c| tools::label(&name, &args_of(c))).unwrap_or_default();
            let label = if label.is_empty() { String::new() } else { format!(" {}", label.chars().take(120).collect::<String>()) };
            let stub = format!("{STUB}: {name}{label}, {} lines. Run it again if you need it.]", content(i).lines().count());
            freed += (content(i).len().saturating_sub(stub.len()) / 4) as u64;
            plan.push((i, stub));
        }
        if plan.is_empty() || freed < min_gain {
            return None;
        }
        let pruned: Vec<(usize, Option<PathBuf>)> = plan.iter().map(|(i, _)| (*i, viewed(by_index[i]))).collect();
        let still: HashSet<PathBuf> = results.iter().filter(|(i, _)| !plan.iter().any(|(p, _)| p == i) && !content(*i).starts_with(STUB)).filter_map(|(_, c)| viewed(*c)).collect();
        let n = plan.len();
        for (i, stub) in plan {
            if let Message::Tool { content, .. } = &mut self.session.messages[i] {
                *content = stub;
            }
        }
        // A file whose contents are no longer in context must be viewed again before it can be edited.
        for (_, f) in pruned {
            if let Some(f) = f.filter(|f| !still.contains(f)) {
                self.read_files.remove(&f);
            }
        }
        self.last_prompt_tokens = used.saturating_sub(freed);
        save_session(&mut self.session);
        Some((n, freed))
    }

    /// Clears old tool output once the context passes cfg.prune_tokens, or sooner when the provider's cache has expired anyway.
    fn auto_prune(&mut self, h: &Hooks) {
        let limit = self.cfg.prune_tokens;
        if limit == 0 {
            return;
        }
        let used = self.context_tokens();
        let cold = self.last_request_at.is_some_and(|t| t.elapsed() > CACHE_TTL);
        let r = if used > limit {
            self.prune(limit / 2, 10_000)
        } else if cold && used > limit / 2 {
            self.prune(limit / 4, 1_000)
        } else {
            None
        };
        if let Some((n, freed)) = r {
            h.send(Ev::Notice(format!("Cleared {n} old tool output{} (~{}k tokens).", if n == 1 { "" } else { "s" }, freed / 1000)));
        }
    }

    /// Index from which a compaction keeps messages word for word: the latest user turns that fit in a quarter of the limit.
    fn compact_cut(&self) -> usize {
        let msgs = &self.session.messages;
        let budget = (if self.cfg.auto_compact_tokens > 0 { self.cfg.auto_compact_tokens } else { 80_000 }) / 4;
        let mut cut = msgs.len();
        for i in (1..msgs.len()).rev() {
            if !matches!(msgs[i], Message::User { .. }) {
                continue;
            }
            if estimate_tokens(&msgs[i..]) > budget {
                break;
            }
            cut = i;
        }
        cut
    }

    /// Replaces the older part of the conversation with a summary, keeping the latest turns as they were.
    pub fn compact(&mut self, focus: &str, h: &Hooks) -> Result<bool, ApiError> {
        if self.session.messages.len() < 2 {
            return Ok(false);
        }
        h.send(Ev::Status("Compacting the conversation…".into()));
        let cut = self.compact_cut();
        let tail = self.session.messages[cut..].to_vec();
        let mut req = vec![self.system()];
        req.extend_from_slice(&self.session.messages[..cut]);
        let ask = if focus.is_empty() { prompt::COMPACT_PROMPT.to_string() } else { format!("{}\n\nFocus especially on: {focus}", prompt::COMPACT_PROMPT) };
        req.push(Message::User { content: Content::Text(ask) });
        // Same system prompt, tools and leading messages as the conversation, so the summary reuses its cached prefix.
        let mut r = self.request(&req, false, true, None, true, h)?;
        if r.content.trim().is_empty() {
            r = self.request(&req, true, true, None, true, h)?;
        }
        let summary = r.content.trim().to_string();
        if summary.is_empty() {
            return Err(ApiError { message: "The model returned an empty summary; the conversation was not compacted.".into(), status: 500, retry_after: None, aborted: false });
        }
        let mut msgs = vec![
            Message::User { content: Content::Text(format!("This session continues an earlier conversation that was compacted. Summary of it:\n\n{summary}")) },
            Message::Assistant { content: Some("Understood. I have the context from the summary and will continue from there.".into()), tool_calls: None, reasoning_content: None },
        ];
        msgs.extend(tail.iter().cloned());
        self.session.messages = msgs;
        self.session.mode = None;
        self.sys = None;
        self.last_prompt_tokens = estimate_tokens(&self.session.messages);
        // Turns in the kept tail can still be undone; earlier ones only restore files.
        for cp in &mut self.checkpoints {
            cp.at = if cp.at >= cut { cp.at - cut + 2 } else { usize::MAX };
        }
        // Files must be read again before editing, unless the kept turns still show them.
        self.read_files.clear();
        let calls: HashMap<String, ToolCall> = tail
            .iter()
            .filter_map(|m| if let Message::Assistant { tool_calls: Some(c), .. } = m { Some(c.clone()) } else { None })
            .flatten()
            .map(|c| (c.id.clone(), c))
            .collect();
        for m in &tail {
            if let Message::Tool { tool_call_id, content } = m {
                if content.starts_with(STUB) {
                    continue;
                }
                if let Some(f) = calls.get(tool_call_id).and_then(|c| tools::viewed_path(&c.function.name, &args_of(c), &self.session.cwd)) {
                    self.read_files.insert(f);
                }
            }
        }
        save_session(&mut self.session);
        h.send(Ev::Totals(self.session.totals.clone(), self.context_tokens()));
        Ok(true)
    }

    fn snapshot(&mut self, abs: PathBuf) {
        let Some(cp) = self.checkpoints.last_mut() else { return };
        if cp.files.contains_key(&abs) {
            return;
        }
        let before = std::fs::read_to_string(&abs).ok();
        cp.files.insert(abs, before);
    }

    /// Puts back the files a checkpoint saw changed, returning the ones restored.
    fn restore(&mut self, cp: &Checkpoint) -> Vec<String> {
        let mut restored = vec![];
        for (abs, before) in &cp.files {
            let ok = match before {
                None => std::fs::remove_file(abs).is_ok() || !abs.exists(),
                Some(b) => std::fs::write(abs, b).is_ok(),
            };
            if ok {
                restored.push(tools::rel(&self.session.cwd, abs));
            }
            self.read_files.remove(abs);
        }
        restored
    }

    /// Undoes the last user message: restores the files it changed and drops it from the conversation.
    pub fn undo(&mut self) -> Option<(String, Vec<String>, bool)> {
        let cp = self.checkpoints.pop()?;
        let restored = self.restore(&cp);
        let in_conversation = cp.at <= self.session.messages.len();
        if in_conversation {
            self.session.messages.truncate(cp.at);
        }
        self.session.mode = None;
        save_session(&mut self.session);
        Some((cp.label, restored, in_conversation))
    }

    /// Drops the conversation from message `at` on and restores the files changed since, newest first, so each file
    /// ends up as it was before that message. Returns the files restored and whether every change since was known:
    /// changes made before this chat was opened in this window weren't recorded.
    pub fn rewind(&mut self, at: usize) -> (Vec<String>, bool) {
        let mut restored = std::collections::BTreeSet::new();
        // The popped checkpoints cover the messages from the earliest one's start on.
        let mut covered_from = self.session.messages.len();
        while self.checkpoints.last().is_some_and(|c| c.at >= at) {
            let cp = self.checkpoints.pop().unwrap();
            covered_from = cp.at;
            restored.extend(self.restore(&cp));
        }
        let end = covered_from.min(self.session.messages.len()).max(at);
        let changed_unknown = self.session.messages[at.min(end)..end]
            .iter()
            .any(|m| matches!(m, Message::Assistant { tool_calls: Some(c), .. } if c.iter().any(|c| tools::permission(&c.function.name) == Permission::Edit)));
        self.session.messages.truncate(at);
        self.session.mode = None;
        save_session(&mut self.session);
        (restored.into_iter().collect(), !changed_unknown)
    }

    /// Adds messages sent while the turn ran, one user message each, unless the user stopped it.
    fn take_steer(&mut self, h: &Hooks) -> bool {
        if self.cancelled() {
            return false;
        }
        let steer = std::mem::take(&mut *self.shared.steer.lock().unwrap());
        if steer.is_empty() {
            return false;
        }
        let mut shown = vec![];
        for s in steer {
            self.read_files.extend(s.read);
            self.session.messages.push(Message::User { content: Content::Text(s.content) });
            shown.push((s.text, s.files));
        }
        h.send(Ev::Steered(shown));
        true
    }

    /// Runs the agent loop for one user message until the model stops calling tools.
    pub fn turn(&mut self, input: String, h: &Hooks) -> Result<(), ApiError> {
        self.auto_prune(h);
        if self.should_compact() {
            self.compact("", h)?;
        }
        self.checkpoints.push(Checkpoint { at: self.session.messages.len(), label: input.clone(), files: HashMap::new() });
        self.session.messages.push(Message::User { content: Content::Text(input) });
        save_session(&mut self.session);

        for step in 0..200 {
            if step > 0 {
                self.auto_prune(h);
                if self.should_compact() {
                    self.compact("", h)?;
                    // When even the current turn was summarised, nothing in the kept messages says to carry on.
                    if self.session.messages.len() == 2 {
                        self.session.messages.push(Message::User { content: Content::Text("The conversation was compacted mid-task. Continue the task from where you left off.".into()) });
                    }
                }
            }
            self.announce_mode();
            self.session.model = self.shared.model.lock().unwrap().clone();
            let mut req = vec![self.system()];
            req.extend_from_slice(&self.session.messages);
            let r = match self.request(&req, false, false, None, false, h) {
                Ok(r) => r,
                Err(e) => {
                    save_session(&mut self.session);
                    return Err(e);
                }
            };
            let calls = r.tool_calls.clone();
            self.session.messages.push(Message::Assistant {
                content: (!r.content.is_empty()).then(|| r.content.clone()),
                tool_calls: (!calls.is_empty()).then(|| calls.clone()),
                reasoning_content: None,
            });
            h.send(Ev::AssistantDone);
            save_session(&mut self.session);

            if calls.is_empty() {
                if r.finish_reason.as_deref() == Some("length") {
                    h.send(Ev::Notice("The reply hit the output limit. Say \"continue\" to get the rest.".into()));
                }
                // A message sent just as the answer ended still gets one in this turn.
                if self.take_steer(h) {
                    save_session(&mut self.session);
                    continue;
                }
                return Ok(());
            }
            for (i, c) in calls.iter().enumerate() {
                if self.cancelled() {
                    for c in &calls[i..] {
                        self.session.messages.push(Message::Tool { tool_call_id: c.id.clone(), content: "Interrupted by the user.".into() });
                    }
                    break;
                }
                let res = self.run_tool(c, h);
                self.session.messages.push(Message::Tool { tool_call_id: c.id.clone(), content: cap_output(res.output) });
            }
            self.take_steer(h);
            save_session(&mut self.session);
            if self.cancelled() {
                return Err(ApiError::aborted());
            }
        }
        h.send(Ev::Notice("Stopped after 200 steps. Say \"continue\" to keep going.".into()));
        Ok(())
    }

    /// Keeps a partial answer when the user stops mid-stream, so the next turn knows what was said.
    pub fn keep_partial(&mut self, partial: &str) {
        if partial.is_empty() {
            return;
        }
        if let Some(Message::User { .. }) | Some(Message::Tool { .. }) = self.session.messages.last() {
            self.session.messages.push(Message::Assistant { content: Some(format!("{partial}\n\n[interrupted by the user]")), tool_calls: None, reasoning_content: None });
            save_session(&mut self.session);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_mode_edits_stay_in_the_project() {
        let cwd = if cfg!(windows) { r"C:\work\app" } else { "/work/app" };
        assert!(inside_project(cwd, "src/main.rs"));
        assert!(inside_project(cwd, "src/../README.md"));
        assert!(!inside_project(cwd, "../other/file.txt"));
        assert!(!inside_project(cwd, ".git/config"));
        assert!(!inside_project(cwd, if cfg!(windows) { r"C:\Windows\hosts" } else { "/etc/hosts" }));
        let patch = json!({ "input": "*** Begin Patch\n*** Update File: a.rs\n*** Add File: ../b.rs\n*** End Patch" });
        assert_eq!(edit_targets(&patch), ["a.rs", "../b.rs"]);
    }

    /// Runs one real turn against the configured account: `cargo test -- --ignored live_turn --nocapture`.
    #[test]
    #[ignore]
    fn live_turn() {
        let cfg = config::load_config();
        let dir = std::env::temp_dir().join(format!("scout-live-{}", crate::session::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let cwd = dir.to_string_lossy().to_string();
        let shared = Arc::new(Shared {
            mode: Mutex::new("yolo".into()),
            model: Mutex::new(cfg.model.clone()),
            provider: Mutex::new(cfg.provider.clone()),
            effort: Mutex::new("low".into()),
            cancel: AtomicBool::new(false),
            edit_tools: Default::default(),
            steer: Default::default(),
        });
        let mut agent = Agent::new(cfg.clone(), crate::session::new_session(&cwd, &cfg.model), shared);
        let (tx, rx) = channel();
        let h = Hooks { tx, ctx: eframe::egui::Context::default() };
        let r = agent.turn("Create a file hello.txt containing exactly `hi from scout`, then run a shell command that prints its contents. Reply in one short sentence.".into(), &h);
        let mut text = String::new();
        for ev in rx.try_iter() {
            match ev {
                Ev::Text(t) => text.push_str(&t),
                Ev::ToolCall { name, label, .. } => println!("tool: {name} {label}"),
                Ev::ToolResult { result, .. } => println!("  -> {:?} {}", result.summary, result.output.lines().next().unwrap_or("")),
                _ => {}
            }
        }
        println!("reply: {text}\ntotals: {:?}", agent.session.totals);
        r.unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("hello.txt")).unwrap().trim(), "hi from scout");
        let _ = std::fs::remove_file(crate::config::home().join("sessions").join(format!("{}.json", agent.session.id)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rewind_restores_files_changed_since() {
        let dir = std::env::temp_dir().join(format!("scout-rewind-{}", crate::session::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&a, "v2").unwrap();
        std::fs::write(&b, "made in turn two").unwrap();
        let cwd = dir.to_string_lossy().to_string();
        let shared = Arc::new(Shared { mode: Mutex::new("default".into()), model: Mutex::new("m".into()), provider: Mutex::new("scoutcode".into()), effort: Mutex::new("low".into()), cancel: AtomicBool::new(false), edit_tools: Default::default(), steer: Default::default() });
        let mut agent = Agent::new(config::load_config(), crate::session::new_session(&cwd, "m"), shared);
        let edit = |id: &str| Message::Assistant {
            content: None,
            tool_calls: Some(vec![ToolCall { id: id.into(), kind: "function".into(), function: api::FunctionCall { name: "write_file".into(), arguments: "{}".into() } }]),
            reasoning_content: None,
        };
        let user = |t: &str| Message::User { content: Content::Text(t.into()) };
        let tool = |id: &str| Message::Tool { tool_call_id: id.into(), content: "ok".into() };
        agent.session.messages = vec![user("one"), edit("1"), tool("1"), user("two"), edit("2"), tool("2")];
        agent.checkpoints = vec![
            Checkpoint { at: 0, label: "one".into(), files: HashMap::from([(a.clone(), Some("v0".to_string()))]) },
            Checkpoint { at: 3, label: "two".into(), files: HashMap::from([(a.clone(), Some("v1".to_string())), (b.clone(), None)]) },
        ];

        let (restored, all) = agent.rewind(3);
        assert!(all);
        assert_eq!(restored, vec!["a.txt".to_string(), "b.txt".to_string()]);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "v1");
        assert!(!b.exists(), "a file the turn created is removed");
        assert_eq!(agent.session.messages.len(), 3);

        agent.rewind(0);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "v0");

        // A reopened chat has no record of its earlier edits, and says so.
        agent.session.messages = vec![user("one"), edit("1"), tool("1")];
        assert!(!agent.rewind(0).1);

        let _ = std::fs::remove_file(crate::config::home().join("sessions").join(format!("{}.json", agent.session.id)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The questions in an `ask_questions` call. Options may be plain strings or {label, description}; up to four questions.
fn parse_questions(args: &Value) -> Vec<Question> {
    args.get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|q| {
            let question = q.get("question").and_then(Value::as_str)?.trim().to_string();
            if question.is_empty() {
                return None;
            }
            let options = q
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|o| match o {
                    Value::String(s) => Some(QuestionOption { label: s.clone(), description: String::new() }),
                    _ => {
                        let label = o.get("label").and_then(Value::as_str)?.to_string();
                        let description = o.get("description").and_then(Value::as_str).unwrap_or_default().to_string();
                        Some(QuestionOption { label, description })
                    }
                })
                .take(6)
                .collect();
            Some(Question {
                question,
                header: q.get("header").and_then(Value::as_str).unwrap_or_default().chars().take(16).collect(),
                options,
                multi_select: q.get("multiSelect").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .take(4)
        .collect()
}
