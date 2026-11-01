//! The app's state behind the window: the agent, the transcript, sign-ins and settings. The web UI in `ui/` draws what
//! `view` says and calls these methods through `bridge.rs`; what's only about drawing (menus, drafts) lives over there.

use crate::agent::{Agent, Ev, Hooks, PermissionAnswer, PermissionRequest, PlanAnswer, Question, QuestionAnswer, Shared, Steer};
use crate::api::{Content, Message, ModelInfo};
use crate::attach;
use crate::config::{self, Config, EFFORTS, MODES};
use crate::diff::{self, DiffLine, Kind};
use crate::session::{self, Session, SessionInfo, Totals};
use crate::tools::{self, Display, Todo, ToolResult};
use crate::update::Status;
use crate::waker::Waker;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

pub enum ItemKind {
    /// What the user typed, and the files they attached.
    User { text: String, files: Vec<String> },
    Assistant { text: String, reasoning: String },
    Tool { name: String, label: String, progress: String, result: Option<ToolResult> },
    Notice(String),
    Error(String),
    /// The account can't pay for the model, so the turn stopped and the window asks what to do. Says why.
    OutOfCredit(String),
}

pub struct Item {
    /// Stays the same for as long as the item exists, so the window can keep what's expanded.
    id: u64,
    kind: ItemKind,
}

struct PendingPermission {
    req: PermissionRequest,
    reply: Sender<PermissionAnswer>,
}

struct PendingPlan {
    text: String,
    reply: Sender<PlanAnswer>,
}

struct PendingQuestions {
    questions: Vec<Question>,
    reply: Sender<QuestionAnswer>,
}

/// A message on its way: what was typed and the files attached to it.
#[derive(Clone)]
pub struct Outgoing {
    pub text: String,
    pub files: Vec<String>,
}

/// Signing in to Copilot, Devin or Kiro, while it's going on.
#[derive(Default)]
struct SignIn {
    /// The code to enter on GitHub, and where. Copilot only.
    code: Option<(String, String)>,
    /// Bytes downloaded so far, of how many.
    progress: (u64, u64),
    installed: bool,
    /// The CLI is there and the browser should be open.
    waiting: bool,
    cancel: Arc<AtomicBool>,
}

impl SignIn {
    fn json(&self) -> Value {
        json!({ "code": self.code.as_ref().map(|c| &c.0), "url": self.code.as_ref().map(|c| &c.1), "done": self.progress.0, "total": self.progress.1, "installed": self.installed, "waiting": self.waiting })
    }
}

#[derive(Default)]
struct Login {
    kilo_check: Option<Result<String, String>>,
    copilot: Option<SignIn>,
    copilot_error: Option<String>,
    devin: Option<SignIn>,
    devin_error: Option<String>,
    kiro: Option<SignIn>,
    kiro_error: Option<String>,
    busy: bool,
    url: Option<String>,
    error: Option<String>,
}

#[derive(Default)]
struct Usage {
    info: Option<crate::api::UsageInfo>,
    busy: bool,
    /// The last result to show: whether it went well, and what happened.
    message: Option<(bool, String)>,
}

enum Voice {
    Idle,
    Recording(crate::voice::Recording),
    /// Transcribing: what it's doing.
    Busy(&'static str),
}

pub struct Core {
    handle: AppHandle,
    cfg: Config,
    shared: Arc<Shared>,
    /// None while a worker thread owns it.
    agent: Option<Agent>,
    hooks: Hooks,
    items: Vec<Item>,
    next_item: u64,
    /// Items changed since the window last heard, or all of them after a reset.
    changed: BTreeSet<usize>,
    reset: bool,
    /// Index of the assistant item being streamed into.
    streaming: Option<usize>,
    partial: String,
    tool_index: HashMap<String, usize>,
    running: bool,
    stopping: bool,
    status: String,
    todos: Vec<Todo>,
    totals: Totals,
    context: u64,
    models: Vec<ModelInfo>,
    models_changed: bool,
    /// The model id to send, for `provider` ("scoutcode", "openrouter", "nvidia" or "kilo").
    model: String,
    provider: String,
    additional_errors: Vec<String>,
    /// The additional provider to choose a model on once its list is in: it was linked with nothing else to run on.
    start_additional: Option<&'static str>,
    kiro_models_error: Option<String>,
    effort: String,
    permission: Option<PendingPermission>,
    plan: Option<PendingPlan>,
    questions: Option<PendingQuestions>,
    /// Every saved chat, from every folder; the sidebar groups them by project.
    sessions: Vec<SessionInfo>,
    projects: Vec<config::Project>,
    starred: Vec<String>,
    /// How many model lists are still being fetched.
    models_loading: u32,
    login: Login,
    /// The open chat's folder, also while a worker has the agent.
    cwd: String,
    /// Messages sent while Scout worked, sent in order once it finishes.
    queue: Vec<Outgoing>,
    skills: Vec<crate::skills::Skill>,
    /// Goes up when Scout may have changed files, so the right panel reloads.
    files_epoch: u64,
    /// Goes up when the composer should take the keyboard.
    focus_epoch: u64,
    /// Goes up with each chat opened, so the window drops what belongs to the last one.
    chat_epoch: u64,
    /// The link that lets web chatbots use Scout, while it's on. It stays off until turned on each run.
    link: Option<crate::mcp::Link>,
    link_status: String,
    /// The chatbot's latest tool calls.
    link_log: Vec<String>,
    link_permission: Option<PendingPermission>,
    usage: Usage,
    voice: Voice,
    updater: crate::update::Updater,
    /// What the window was last told, by key, so only what changed is sent.
    sent: HashMap<&'static str, String>,
    /// Counts the sends, so that a window starting up can tell which came after its snapshot.
    seq: u64,
}

fn project_name(cwd: &str) -> String {
    std::path::Path::new(cwd).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| cwd.to_string())
}

/// The folder to open: the first argument, the current directory, or the last project when launched from a shortcut.
fn initial_cwd() -> String {
    if let Some(a) = std::env::args().nth(1).filter(|a| std::path::Path::new(a).is_dir()) {
        return std::fs::canonicalize(&a).map(|p| p.to_string_lossy().trim_start_matches(r"\\?\").to_string()).unwrap_or(a);
    }
    let cwd = std::env::current_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
    let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_string_lossy().to_string())).unwrap_or_default();
    let sys = std::env::var("WINDIR").unwrap_or_default();
    let launched_oddly = cwd.is_empty() || session::same_dir(&cwd, &exe_dir) || (!sys.is_empty() && cwd.to_lowercase().starts_with(&sys.to_lowercase()));
    if launched_oddly {
        let saved = std::fs::read_to_string(config::home().join("config.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.get("desktopProject").and_then(Value::as_str).map(String::from));
        if let Some(p) = saved.filter(|p| std::path::Path::new(p).is_dir()) {
            return p;
        }
        // Started from the Start menu for the first time: the home folder beats the install folder.
        if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
            return home.to_string_lossy().to_string();
        }
    }
    cwd
}

fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 12 {
        return "…".into();
    }
    format!("{}…{}", chars[..8].iter().collect::<String>(), chars[chars.len() - 4..].iter().collect::<String>())
}

// ---------- what's sent to the window ----------

/// Numbered diff lines with context, or all of a new file; null is a gap between hunks.
pub fn diff_rows(lines: &[DiffLine], created: bool) -> Value {
    const MAX: usize = 600;
    let shown: Vec<Option<&DiffLine>> = if created { lines.iter().map(Some).collect() } else { diff::hunks(lines, 3) };
    let rows: Vec<Value> = shown
        .iter()
        .take(MAX)
        .map(|l| match l {
            Some(l) => json!({ "kind": match l.kind { Kind::Add => "add", Kind::Del => "del", Kind::Same => "same" }, "text": l.text, "no": l.new_no.or(l.old_no) }),
            None => Value::Null,
        })
        .collect();
    json!({ "rows": rows, "more": shown.len().saturating_sub(MAX) })
}

fn display_json(d: &Display) -> Value {
    match d {
        Display::Text(t) => {
            let lines: Vec<&str> = t.lines().collect();
            let text = if lines.len() > 400 { format!("{}\n… {} more lines", lines[..400].join("\n"), lines.len() - 400) } else { t.clone() };
            json!({ "type": "text", "text": text })
        }
        Display::Diff { path, lines, created } => {
            let (add, del) = diff::stats(lines);
            json!({ "type": "diff", "path": path, "created": created, "add": add, "del": del, "lines": diff_rows(lines, *created) })
        }
        Display::Diffs(all) => json!({ "type": "diffs", "diffs": all.iter().map(display_json).collect::<Vec<_>>() }),
        Display::Todos(t) => json!({ "type": "todos", "todos": todos_json(t) }),
    }
}

fn todos_json(todos: &[Todo]) -> Value {
    Value::Array(todos.iter().map(|t| json!({ "content": t.content, "status": t.status })).collect())
}

fn item_json(item: &Item) -> Value {
    match &item.kind {
        ItemKind::User { text, files } => json!({ "id": item.id, "kind": "user", "text": text, "files": files }),
        ItemKind::Assistant { text, reasoning } => json!({ "id": item.id, "kind": "assistant", "text": text, "reasoning": reasoning }),
        ItemKind::Tool { name, label, progress, result } => {
            let result = result.as_ref().map(|r| {
                let display = r.display.clone().unwrap_or_else(|| Display::Text(r.output.clone()));
                json!({ "isError": r.is_error, "summary": r.summary, "display": display_json(&display) })
            });
            json!({ "id": item.id, "kind": "tool", "name": name, "label": label, "progress": progress, "result": result })
        }
        ItemKind::Notice(text) => json!({ "id": item.id, "kind": "notice", "text": text }),
        ItemKind::Error(text) => json!({ "id": item.id, "kind": "error", "text": text }),
        ItemKind::OutOfCredit(text) => json!({ "id": item.id, "kind": "outOfCredit", "text": text }),
    }
}

fn permission_json(p: &PendingPermission) -> Value {
    json!({ "tool": p.req.tool, "label": p.req.label, "preview": p.req.preview.as_ref().map(display_json), "prefix": p.req.prefix, "reason": p.req.reason })
}

fn model_json(m: &ModelInfo) -> Value {
    json!({
        "id": m.id, "label": m.label(), "description": m.description, "context": m.context, "tools": m.tools, "vision": m.vision, "cost": m.cost,
        "providers": m.providers, "routesTo": m.routes_to, "available": m.available, "openrouterId": m.openrouter_id, "nvidiaId": m.nvidia_id,
        "kiloId": m.kilo_id, "kiloFree": m.kilo_free, "kiloTier": m.kilo_tier, "copilotId": m.copilot_id, "devinId": m.devin_id, "kiroId": m.kiro_id,
        "additional": m.additional, "hosted": m.hosted, "free": m.free, "tier": m.tier, "openrouterTier": m.openrouter_tier,
    })
}

fn status_json(s: &Status) -> Value {
    match s {
        Status::Idle => json!({ "state": "idle" }),
        Status::Checking => json!({ "state": "checking" }),
        Status::UpToDate => json!({ "state": "upToDate" }),
        Status::Available { version, notes } => json!({ "state": "available", "version": version, "notes": notes }),
        Status::Downloading { version, got, total } => json!({ "state": "downloading", "version": version, "got": got, "total": total }),
        Status::Ready { version, notes } => json!({ "state": "ready", "version": version, "notes": notes }),
        Status::Failed(e) => json!({ "state": "failed", "error": e }),
    }
}

/// Transcript items for a saved conversation. Tool results show their text output; diffs aren't stored.
fn rebuild(s: &Session, next_id: &mut u64) -> Vec<Item> {
    let results: HashMap<&str, &str> = s
        .messages
        .iter()
        .filter_map(|m| if let Message::Tool { tool_call_id, content } = m { Some((tool_call_id.as_str(), content.as_str())) } else { None })
        .collect();
    let mut kinds = vec![];
    for m in &s.messages {
        match m {
            Message::User { content } => {
                let (text, files) = user_parts(content);
                kinds.push(ItemKind::User { text, files });
            }
            Message::Assistant { content, tool_calls, .. } => {
                if let Some(c) = content.as_ref().filter(|c| !c.is_empty()) {
                    kinds.push(ItemKind::Assistant { text: c.clone(), reasoning: String::new() });
                }
                for c in tool_calls.iter().flatten() {
                    let args: Value = serde_json::from_str(&c.function.arguments).unwrap_or_default();
                    let out = results.get(c.id.as_str()).map(|o| crate::prompt::strip_mode_note(o).to_string());
                    kinds.push(ItemKind::Tool {
                        name: c.function.name.clone(),
                        label: tools::label(&c.function.name, &args),
                        progress: String::new(),
                        result: Some(ToolResult { is_error: out.as_deref().is_some_and(|o| o.starts_with("Error") || o.starts_with("The user denied")), output: out.unwrap_or_default(), ..Default::default() }),
                    });
                }
            }
            _ => {}
        }
    }
    kinds
        .into_iter()
        .map(|kind| {
            *next_id += 1;
            Item { id: *next_id, kind }
        })
        .collect()
}

fn user_parts(content: &Content) -> (String, Vec<String>) {
    let t = content.text();
    attach::split(crate::prompt::strip_mode_note(&t))
}

impl Core {
    pub fn new(handle: AppHandle, tx: Sender<Ev>, waker: Waker) -> Self {
        let cfg = config::load_config();
        let hooks = Hooks { tx, ctx: waker.clone() };
        let shared = Arc::new(Shared {
            mode: Mutex::new(if MODES.contains(&cfg.mode.as_str()) { cfg.mode.clone() } else { "default".into() }),
            model: Mutex::new(cfg.model.clone()),
            provider: Mutex::new(cfg.provider.clone()),
            effort: Mutex::new(cfg.effort.clone()),
            cancel: Default::default(),
            edit_tools: Default::default(),
            model_names: Default::default(),
            steer: Default::default(),
        });
        let cwd = initial_cwd();
        let agent = Agent::new(cfg.clone(), session::new_session(&cwd, &cfg.model), shared.clone());
        let updater = crate::update::Updater::start(&waker, cfg.auto_update);
        crate::mcp_client::hub().reload(&waker, false);
        let mut core = Core {
            handle,
            model: cfg.model.clone(),
            provider: cfg.provider.clone(),
            additional_errors: vec![],
            start_additional: None,
            kiro_models_error: None,
            effort: cfg.effort.clone(),
            projects: cfg.projects.clone(),
            starred: cfg.starred_models.clone(),
            cfg,
            shared,
            agent: Some(agent),
            hooks,
            items: vec![],
            next_item: 0,
            changed: BTreeSet::new(),
            reset: true,
            streaming: None,
            partial: String::new(),
            tool_index: HashMap::new(),
            running: false,
            stopping: false,
            status: String::new(),
            todos: vec![],
            totals: Totals::default(),
            context: 0,
            models: vec![],
            models_changed: true,
            permission: None,
            plan: None,
            questions: None,
            sessions: session::list_sessions(),
            models_loading: 0,
            login: Login::default(),
            cwd: cwd.clone(),
            queue: vec![],
            link: None,
            skills: crate::skills::list(),
            files_epoch: 0,
            focus_epoch: 0,
            chat_epoch: 0,
            link_status: String::new(),
            link_log: vec![],
            link_permission: None,
            usage: Usage::default(),
            voice: Voice::Idle,
            updater,
            sent: HashMap::new(),
            seq: 0,
        };
        core.enter_project(&cwd);
        core.fetch_models();
        // `scout-desktop [folder] --resume <id>` reopens a saved conversation.
        let args: Vec<String> = std::env::args().collect();
        if let Some(s) = args.iter().position(|a| a == "--resume").and_then(|i| args.get(i + 1)).and_then(|id| session::load_session(id)) {
            core.open_session(s);
        }
        core
    }

    pub fn wake(&self) {
        self.hooks.ctx.request_repaint();
    }

    pub fn waker(&self) -> Waker {
        self.hooks.ctx.clone()
    }

    pub fn is_recording(&self) -> bool {
        matches!(self.voice, Voice::Recording(_))
    }

    pub fn cwd(&self) -> String {
        self.cwd.clone()
    }

    fn push(&mut self, kind: ItemKind) {
        self.next_item += 1;
        self.changed.insert(self.items.len());
        self.items.push(Item { id: self.next_item, kind });
    }

    fn replace_items(&mut self, items: Vec<Item>) {
        self.items = items;
        self.reset = true;
        self.changed.clear();
    }

    /// Loads the ScoutCode, OpenRouter, NIM, Kilo and additional providers' lists, whichever are on, and merges them.
    fn fetch_models(&mut self) {
        self.models_loading += 1;
        let cfg = self.cfg.clone();
        let h = self.hooks.clone();
        std::thread::spawn(move || {
            let (mut scout, mut openrouter, mut errors) = (vec![], vec![], vec![]);
            if cfg.api_key.is_some() {
                match crate::api::list_models(&cfg) {
                    Ok((models, user)) => {
                        if let Some(u) = user {
                            h.send(Ev::User(u));
                        }
                        scout = models;
                    }
                    Err(e) => errors.push(e.message),
                }
            }
            // The first key, in order, that lists the models is used.
            let mut failed = None;
            for k in &cfg.openrouter_keys {
                let mut only = cfg.clone();
                only.openrouter_keys = vec![k.clone()];
                match crate::api::list_openrouter(&only) {
                    Ok(models) => {
                        openrouter = models;
                        failed = None;
                        break;
                    }
                    Err(e) => failed = Some(e.message),
                }
            }
            if let Some(e) = failed {
                errors.push(format!("OpenRouter: {e}"));
            }
            let mut nvidia = vec![];
            if cfg.nvidia_key.is_some() {
                match crate::api::list_nvidia(&cfg) {
                    Ok(models) => nvidia = models,
                    Err(e) => errors.push(format!("NVIDIA NIM: {}", e.message)),
                }
            }
            // Always asked for: Kilo's free models need no key.
            let mut kilo = vec![];
            match crate::api::list_kilo(&cfg) {
                Ok(models) => kilo = models,
                Err(e) => errors.push(format!("Kilo Code: {}", e.message)),
            }
            let (mut additional, mut additional_errors) = (vec![], vec![]);
            for a in config::ADDITIONAL.iter().filter(|a| cfg.linked(a.id).is_some()) {
                match crate::api::list_additional(&cfg, a) {
                    Ok(models) => additional.extend(models),
                    Err(e) => additional_errors.push(format!("{}: {}", a.name, e.message)),
                }
            }
            errors.extend(additional_errors.iter().cloned());
            h.send(Ev::AdditionalErrors(additional_errors));
            let asked = cfg.api_key.is_some() as usize + (!cfg.openrouter_keys.is_empty()) as usize + cfg.nvidia_key.is_some() as usize + cfg.additional.len() + 1;
            if errors.len() == asked && cfg.copilot_token.is_none() && !cfg.devin_enabled && !cfg.kiro_enabled {
                h.send(Ev::Models(Err(errors.join("; "))));
            } else {
                let mut models = crate::api::merge_models(scout, openrouter, nvidia, kilo);
                models.extend(additional);
                // Auto is always there: it's all a free plan gets, and its list comes back empty.
                if let Some(token) = &cfg.copilot_token {
                    let listed = crate::copilot::list_models(token);
                    let copilot = |id: String, name: String| ModelInfo { id: format!("copilot/{id}"), name: Some(name), copilot_id: Some(id), ..Default::default() };
                    models.push(copilot("auto".into(), "Copilot Auto".into()));
                    models.extend(listed.into_iter().filter(|(id, _)| id != "auto").map(|(id, name)| copilot(id, name)));
                }
                // The account's default is always there too: it needs no list, and lets Devin pick for the plan.
                if cfg.devin_enabled {
                    let devin = |id: String, name: String| ModelInfo { id: format!("devin/{id}"), name: Some(name), devin_id: Some(id), ..Default::default() };
                    models.push(devin("auto".into(), "Devin Default".into()));
                    models.extend(crate::devin::list_models().into_iter().map(|(id, name)| devin(id, name)));
                }
                // Auto is Kiro's default, so it's there even if the list can't be read.
                if cfg.kiro_enabled {
                    let kiro = |id: String, name: String| ModelInfo { id: format!("kiro/{id}"), name: Some(name), kiro_id: Some(id), ..Default::default() };
                    models.push(kiro("auto".into(), "Kiro Auto".into()));
                    let (listed, error) = match crate::kiro::list_models() {
                        Ok(listed) => (listed, None),
                        Err(e) => (vec![], Some(crate::kiro::explain(&e))),
                    };
                    h.send(Ev::KiroModelsError(error));
                    models.extend(listed.into_iter().filter(|(id, _)| id != "auto").map(|(id, name)| kiro(id, name)));
                }
                h.send(Ev::Models(Ok(models)));
            }
        });
    }

    // ---------- events from the workers ----------

    pub fn handle(&mut self, ev: Ev) {
        match ev {
            Ev::RequestStart => {
                self.streaming = None;
                self.partial.clear();
                self.status.clear();
            }
            Ev::Text(t) => {
                self.partial.push_str(&t);
                if let ItemKind::Assistant { text, .. } = self.assistant_item() {
                    text.push_str(&t);
                }
            }
            Ev::Reasoning(t) => {
                if let ItemKind::Assistant { reasoning, .. } = self.assistant_item() {
                    reasoning.push_str(&t);
                }
            }
            Ev::AssistantDone => {
                self.streaming = None;
                self.partial.clear();
            }
            Ev::ToolCall { id, name, label } => {
                self.streaming = None;
                self.tool_index.insert(id, self.items.len());
                self.push(ItemKind::Tool { name, label, progress: String::new(), result: None });
            }
            Ev::ToolProgress { id, text } => {
                if let Some(&i) = self.tool_index.get(&id) {
                    if let Some(ItemKind::Tool { progress, .. }) = self.items.get_mut(i).map(|i| &mut i.kind) {
                        *progress = text;
                        self.changed.insert(i);
                    }
                }
            }
            Ev::ToolResult { id, result } => {
                if let Some(&i) = self.tool_index.get(&id) {
                    if let Some(ItemKind::Tool { name, result: r, .. }) = self.items.get_mut(i).map(|i| &mut i.kind) {
                        if matches!(name.as_str(), "write_file" | "edit_file" | "multi_edit" | "apply_patch" | "bash" | "update_memory") {
                            self.files_epoch += 1;
                        }
                        *r = Some(result);
                        self.changed.insert(i);
                    }
                }
            }
            Ev::Status(s) => self.status = s,
            Ev::Notice(s) => self.push(ItemKind::Notice(s)),
            Ev::Todos(t) => self.todos = t,
            Ev::Totals(t, c) => {
                self.totals = t;
                self.context = c;
            }
            Ev::AskPermission(req, reply) => {
                let verb = match req.tool.as_str() {
                    "bash" => "run a command",
                    "devin" => "use a Devin tool",
                    "kiro" => "use a Kiro tool",
                    t if tools::is_mcp(t) => "use an MCP tool",
                    _ => "change a file",
                };
                self.alert(&format!("Scout wants to {verb}"), req.label.lines().next().unwrap_or(""));
                self.permission = Some(PendingPermission { req, reply });
            }
            Ev::AskPlan(text, reply) => {
                self.alert("Scout's plan is ready", &self.chat_title());
                self.plan = Some(PendingPlan { text, reply });
            }
            Ev::AskQuestions(questions, reply) => {
                self.alert("Scout has a question", &self.chat_title());
                self.questions = Some(PendingQuestions { questions, reply });
            }
            Ev::Steered(msgs) => {
                self.streaming = None;
                for (text, files) in msgs {
                    self.push(ItemKind::User { text, files });
                }
            }
            Ev::Finished(mut agent, result) => {
                let stopped = self.stopping || matches!(&result, Err(e) if e == "Interrupted");
                let error = result.as_ref().err().filter(|_| !stopped).cloned();
                let out_of_credit = std::mem::take(&mut agent.out_of_credit) && !stopped;
                if let Err(e) = result {
                    if stopped {
                        agent.keep_partial(&self.partial);
                        self.push(ItemKind::Notice("Interrupted.".into()));
                    } else if out_of_credit {
                        self.push(ItemKind::OutOfCredit(e));
                    } else {
                        self.push(ItemKind::Error(e));
                    }
                }
                // Messages sent to steer that the turn ended before taking go first in the queue.
                let left: Vec<Steer> = std::mem::take(&mut *self.shared.steer.lock().unwrap());
                self.queue.splice(0..0, left.into_iter().map(|s| Outgoing { text: s.text, files: s.files }));
                self.totals = agent.session.totals.clone();
                self.context = agent.context_tokens();
                self.sessions = session::list_sessions();
                self.agent = Some(*agent);
                self.running = false;
                self.stopping = false;
                self.streaming = None;
                self.partial.clear();
                self.status.clear();
                self.permission = None;
                self.plan = None;
                self.questions = None;
                self.focus_epoch += 1;
                self.files_epoch += 1;
                // The queue carries on unless the user stopped Scout or it ran out of credit: they then decide what's next.
                if !stopped && !out_of_credit && !self.queue.is_empty() {
                    let next = self.queue.remove(0);
                    self.submit(next);
                } else if let Some(e) = error {
                    self.alert("Scout stopped with an error", &e);
                } else if !stopped {
                    self.alert("Scout is done", &self.chat_title());
                }
            }
            Ev::Models(Ok(m)) => {
                // The agent picks each model's edit tools from these, under the id it sends on any provider.
                *self.shared.edit_tools.lock().unwrap() = m.iter().flat_map(|m| std::iter::once(m.id.clone()).chain(m.openrouter_id.clone()).chain(m.nvidia_id.clone()).chain(m.copilot_id.clone()).chain(m.devin_id.clone()).chain(m.kiro_id.clone()).chain(m.additional.as_ref().map(|(_, id)| id.clone())).map(move |id| (id, m.edit_tool))).collect();
                *self.shared.model_names.lock().unwrap() = m.iter().flat_map(|m| std::iter::once(m.id.clone()).chain(m.openrouter_id.clone()).chain(m.nvidia_id.clone()).chain(m.copilot_id.clone()).chain(m.devin_id.clone()).chain(m.kiro_id.clone()).chain(m.additional.as_ref().map(|(_, id)| id.clone())).map(move |id| (id, m.label()))).collect();
                self.models = m;
                self.models_changed = true;
                self.models_loading -= 1;
                self.start_on_additional();
            }
            Ev::Models(Err(e)) => {
                self.status = format!("Could not load models: {e}");
                self.models_loading -= 1;
            }
            Ev::KiloCheck(r) => self.login.kilo_check = Some(r),
            Ev::AdditionalErrors(e) => self.additional_errors = e,
            Ev::Copilot(u) => self.copilot_update(u),
            Ev::Devin(u) => self.devin_update(u),
            Ev::Kiro(u) => self.kiro_update(u),
            Ev::KiroModelsError(e) => self.kiro_models_error = e,
            Ev::LoginUrl(u) => self.login.url = Some(u),
            Ev::User(u) => self.set_user(Some(u)),
            Ev::Usage(r, said) => {
                self.usage.busy = false;
                match r {
                    Ok(info) => {
                        self.usage.info = Some(info);
                        self.usage.message = said.map(|s| (true, s));
                    }
                    Err(e) => self.usage.message = Some((false, e)),
                }
            }
            Ev::Login(Ok((token, user))) => {
                config::save_one("apiKey", json!(token));
                self.set_key(Some(token));
                self.set_user(user);
                self.login = Login::default();
            }
            Ev::Login(Err(e)) => {
                self.login.busy = false;
                self.login.error = Some(e);
            }
            Ev::Link(crate::mcp::LinkEvent::Status(s)) => self.link_status = s,
            Ev::Link(crate::mcp::LinkEvent::Call(c)) => {
                self.link_log.push(c);
                if self.link_log.len() > 8 {
                    self.link_log.remove(0);
                }
            }
            Ev::Link(crate::mcp::LinkEvent::Ask(req, reply)) => {
                self.alert("A chatbot wants to use Scout", req.label.lines().next().unwrap_or(""));
                self.link_permission = Some(PendingPermission { req, reply });
            }
            Ev::Link(crate::mcp::LinkEvent::Ended(s)) => {
                self.link = None;
                self.link_permission = None;
                self.link_status = s;
            }
            Ev::Voice(crate::voice::Update::Busy(s)) => self.voice = Voice::Busy(s),
            Ev::Voice(crate::voice::Update::Done(r)) => {
                self.voice = Voice::Idle;
                match r {
                    Ok(text) if text.is_empty() => self.status = "Didn't catch anything. Try again a little closer to the mic.".into(),
                    Ok(text) => {
                        self.status.clear();
                        let _ = self.handle.emit("dictated", text);
                        self.focus_epoch += 1;
                    }
                    Err(e) => self.status = e,
                }
            }
        }
    }

    /// The mic button: starts a recording, or stops it and transcribes what was said into the box.
    pub fn toggle_voice(&mut self) {
        match &self.voice {
            Voice::Idle => {
                self.status.clear();
                self.voice = Voice::Recording(crate::voice::start(self.cfg.clone(), self.hooks.clone()));
            }
            Voice::Recording(r) => {
                r.stop();
                self.voice = Voice::Busy("Transcribing…");
            }
            Voice::Busy(_) => {}
        }
    }

    fn assistant_item(&mut self) -> &mut ItemKind {
        if self.streaming.is_none() {
            self.streaming = Some(self.items.len());
            self.push(ItemKind::Assistant { text: String::new(), reasoning: String::new() });
        }
        let i = self.streaming.unwrap();
        self.changed.insert(i);
        &mut self.items[i].kind
    }

    fn set_user(&mut self, user: Option<String>) {
        if self.cfg.username != user {
            config::save_one("username", json!(user));
            self.cfg.username = user;
        }
    }

    /// A new key may be someone else's, so the username is dropped until the model list names it.
    fn set_key(&mut self, key: Option<String>) {
        self.set_user(None);
        self.usage = Default::default();
        self.cfg.api_key = key;
        if self.cfg.only_kilo() && self.provider == "scoutcode" {
            self.choose_model(config::KILO_DEFAULT_MODEL, "kilo");
        }
        self.keys_changed();
    }

    fn keys_changed(&mut self) {
        if let Some(a) = &mut self.agent {
            a.cfg.take_keys(&self.cfg);
        }
        self.fetch_models();
    }

    pub fn save_api_key(&mut self, key: String) {
        config::save_one("apiKey", json!(key));
        self.set_key(Some(key));
        self.login = Login::default();
    }

    pub fn browser_login(&mut self) {
        if self.login.busy {
            return;
        }
        self.login.busy = true;
        self.login.error = None;
        let cfg = self.cfg.clone();
        let h = self.hooks.clone();
        std::thread::spawn(move || {
            let h2 = h.clone();
            let r = crate::oauth::browser_login(&cfg, |u| h2.send(Ev::LoginUrl(u.to_string())));
            h.send(Ev::Login(r));
        });
    }

    pub fn sign_out(&mut self) {
        self.stop_link("Off.");
        config::save_one("apiKey", Value::Null);
        self.set_key(None);
    }

    /// Adds a key at the end of the order (so it's tried last). An empty name gets a number.
    pub fn add_openrouter_key(&mut self, name: String, key: String) {
        let mut keys = self.cfg.openrouter_keys.clone();
        let name = if name.is_empty() { format!("Key {}", keys.len() + 1) } else { name };
        keys.push(config::OpenRouterKey { name, key });
        self.save_openrouter_keys(keys);
    }

    /// Moves or removes a key: `op` is "up", "down" or "remove".
    pub fn edit_openrouter_key(&mut self, i: usize, op: &str) {
        let mut keys = self.cfg.openrouter_keys.clone();
        if i >= keys.len() {
            return;
        }
        match op {
            "up" if i > 0 => keys.swap(i, i - 1),
            "down" if i + 1 < keys.len() => keys.swap(i, i + 1),
            "remove" => {
                keys.remove(i);
            }
            _ => return,
        }
        self.save_openrouter_keys(keys);
    }

    /// Saves the OpenRouter keys in their order. With none left, a model that ran there moves back to ScoutCode.
    fn save_openrouter_keys(&mut self, keys: Vec<config::OpenRouterKey>) {
        config::save_openrouter_keys(&keys);
        let removed = keys.is_empty();
        self.cfg.openrouter_keys = keys;
        if removed && self.provider == "openrouter" {
            self.choose_model("auto", "scoutcode");
        }
        self.keys_changed();
    }

    /// Saves or forgets the NVIDIA key. Without one, a model that ran on NIM moves back to ScoutCode.
    pub fn save_nvidia_key(&mut self, key: Option<String>) {
        config::save_one("nvidiaKey", json!(key));
        self.cfg.nvidia_key = key;
        if self.cfg.nvidia_key.is_none() && self.provider == "nvidia" {
            self.choose_model("auto", "scoutcode");
        }
        self.keys_changed();
    }

    /// Turns Kilo on or off and saves its key. With nothing else to run on, models move there; without Kilo, a model
    /// that ran there moves back to ScoutCode.
    pub fn save_kilo(&mut self, enabled: bool, key: Option<String>) {
        config::save_one("kiloEnabled", json!(enabled.then_some(true)));
        config::save_one("kiloKey", json!(key));
        self.cfg.kilo_enabled = enabled;
        self.cfg.kilo_key = key;
        self.login.kilo_check = None;
        if !self.cfg.kilo_on() && self.provider == "kilo" {
            self.choose_model("auto", "scoutcode");
        } else if self.cfg.only_kilo() && self.provider == "scoutcode" {
            self.choose_model(config::KILO_DEFAULT_MODEL, "kilo");
        }
        self.keys_changed();
    }

    /// Keeps the on/off the same and takes the key away.
    pub fn remove_kilo_key(&mut self) {
        self.save_kilo(self.cfg.kilo_enabled, None);
    }

    pub fn check_kilo(&mut self) {
        self.login.kilo_check = None;
        let (cfg, h) = (self.cfg.clone(), self.hooks.clone());
        std::thread::spawn(move || h.send(Ev::KiloCheck(crate::api::check_kilo(&cfg))));
    }

    /// Starts signing in to Copilot: the code shows while the runtime downloads behind it.
    pub fn copilot_sign_in(&mut self) {
        let sign_in = SignIn::default();
        let cancel = sign_in.cancel.clone();
        self.login.copilot = Some(sign_in);
        self.login.copilot_error = None;
        let h = self.hooks.clone();
        crate::copilot::sign_in(cancel, move |u| h.send(Ev::Copilot(u)));
    }

    fn copilot_update(&mut self, update: crate::copilot::Update) {
        use crate::copilot::Update;
        match update {
            Update::Done { token, user } => {
                self.login.copilot = None;
                self.save_copilot(Some(token), user);
                self.choose_model("auto", "copilot");
            }
            Update::Failed(e) => {
                // Cancelling fails with no message.
                self.login.copilot = None;
                self.login.copilot_error = (!e.is_empty()).then_some(e);
            }
            other => {
                let Some(s) = &mut self.login.copilot else { return };
                match other {
                    Update::Code { code, url } => s.code = Some((code, url)),
                    Update::Progress { done, total } => s.progress = (done, total),
                    _ => s.installed = true,
                }
            }
        }
    }

    /// Saves the linked additional providers. A model that ran on one no longer linked moves back to ScoutCode.
    fn save_additional(&mut self, linked: Vec<config::LinkedProvider>) {
        config::save_one("additionalProviders", json!(linked));
        self.cfg.additional = linked;
        if config::additional_provider(&self.provider).is_some() && self.cfg.linked(&self.provider).is_none() {
            self.choose_model("auto", "scoutcode");
        }
        if self.cfg.only_kilo() && self.provider == "scoutcode" {
            self.choose_model(config::KILO_DEFAULT_MODEL, "kilo");
        }
        self.keys_changed();
    }

    pub fn unlink_additional(&mut self, id: &str) {
        let mut linked = self.cfg.additional.clone();
        linked.retain(|o| o.id != id);
        self.save_additional(linked);
    }

    /// Links an additional provider, or changes its key. With nothing else to run on, models move there once it has
    /// listed them.
    pub fn link_additional(&mut self, id: &str, key: Option<String>) {
        let Some(a) = config::additional_provider(id) else { return };
        let mut linked = self.cfg.additional.clone();
        match linked.iter_mut().find(|l| l.id == a.id) {
            Some(l) => l.key = key,
            None => linked.push(config::LinkedProvider { id: a.id.into(), key }),
        }
        if self.provider == "scoutcode" && self.cfg.api_key.is_none() {
            self.start_additional = Some(a.id);
        }
        self.save_additional(linked);
    }

    /// Chooses a model on the additional provider just linked, once it's in the list.
    fn start_on_additional(&mut self) {
        let Some(a) = self.start_additional.and_then(config::additional_provider) else { return };
        let on_it: Vec<&ModelInfo> = self.models.iter().filter(|m| m.additional.as_ref().is_some_and(|(p, _)| *p == a.id)).collect();
        let Some(m) = on_it.iter().find(|m| m.request_id(a.id) == a.default_model).or(on_it.first()) else { return };
        let key = m.id.clone();
        self.start_additional = None;
        self.choose_model(&key, a.id);
    }

    /// Saves or forgets the Copilot sign-in. Without one, a model that ran on Copilot moves back to ScoutCode.
    fn save_copilot(&mut self, token: Option<String>, user: Option<String>) {
        config::save_one("copilotToken", json!(token));
        config::save_one("copilotUser", json!(user));
        self.cfg.copilot_token = token;
        self.cfg.copilot_user = user;
        if self.cfg.copilot_token.is_none() && self.provider == "copilot" {
            self.choose_model("auto", "scoutcode");
        }
        self.keys_changed();
    }

    pub fn copilot_sign_out(&mut self, uninstall: bool) {
        self.save_copilot(None, None);
        if uninstall {
            crate::copilot::remove_runtime();
        }
    }

    /// Starts signing in to Devin: the CLI downloads, then opens the browser.
    pub fn devin_sign_in(&mut self) {
        let sign_in = SignIn::default();
        let cancel = sign_in.cancel.clone();
        self.login.devin = Some(sign_in);
        self.login.devin_error = None;
        let h = self.hooks.clone();
        crate::devin::sign_in(cancel, move |u| h.send(Ev::Devin(u)));
    }

    fn devin_update(&mut self, update: crate::devin::Update) {
        use crate::devin::Update;
        match update {
            Update::Done => {
                self.login.devin = None;
                self.save_devin(true);
                self.choose_model("auto", "devin");
            }
            Update::Failed(e) => {
                // Cancelling fails with no message.
                self.login.devin = None;
                self.login.devin_error = (!e.is_empty()).then_some(e);
            }
            Update::Progress { done, total } => {
                if let Some(s) = &mut self.login.devin {
                    s.progress = (done, total);
                }
            }
            Update::Waiting => {
                if let Some(s) = &mut self.login.devin {
                    s.waiting = true;
                }
            }
        }
    }

    /// Turns Devin on or off. Without it, a model that ran on Devin moves back to ScoutCode.
    fn save_devin(&mut self, on: bool) {
        config::save_one("devinEnabled", json!(on));
        self.cfg.devin_enabled = on;
        if !on && self.provider == "devin" {
            self.choose_model("auto", "scoutcode");
        }
        self.keys_changed();
    }

    pub fn devin_sign_out(&mut self, uninstall: bool) {
        crate::devin::sign_out();
        self.save_devin(false);
        if uninstall {
            crate::devin::remove_runtime();
        }
    }

    /// Starts signing in to Kiro: the CLI downloads, then opens the browser.
    pub fn kiro_sign_in(&mut self) {
        let sign_in = SignIn::default();
        let cancel = sign_in.cancel.clone();
        self.login.kiro = Some(sign_in);
        self.login.kiro_error = None;
        let h = self.hooks.clone();
        crate::kiro::sign_in(cancel, move |u| h.send(Ev::Kiro(u)));
    }

    fn kiro_update(&mut self, update: crate::kiro::Update) {
        use crate::kiro::Update;
        match update {
            Update::Done => {
                self.login.kiro = None;
                self.save_kiro(true);
                self.choose_model("auto", "kiro");
            }
            Update::Failed(e) => {
                // Cancelling fails with no message.
                self.login.kiro = None;
                self.login.kiro_error = (!e.is_empty()).then_some(e);
            }
            Update::Progress { done, total } => {
                if let Some(s) = &mut self.login.kiro {
                    s.progress = (done, total);
                }
            }
            Update::Waiting => {
                if let Some(s) = &mut self.login.kiro {
                    s.waiting = true;
                }
            }
        }
    }

    /// Turns Kiro on or off. Without it, a model that ran on Kiro moves back to ScoutCode.
    fn save_kiro(&mut self, on: bool) {
        config::save_one("kiroEnabled", json!(on));
        self.cfg.kiro_enabled = on;
        if !on && self.provider == "kiro" {
            self.choose_model("auto", "scoutcode");
        }
        self.keys_changed();
    }

    pub fn kiro_sign_out(&mut self, uninstall: bool) {
        crate::kiro::sign_out();
        self.save_kiro(false);
        if uninstall {
            crate::kiro::remove_runtime();
        }
    }

    /// Stops a sign-in that's under way: `which` is "copilot", "devin" or "kiro".
    pub fn cancel_sign_in(&mut self, which: &str) {
        let slot = match which {
            "copilot" => &mut self.login.copilot,
            "devin" => &mut self.login.devin,
            _ => &mut self.login.kiro,
        };
        if let Some(s) = slot.take() {
            s.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Makes `key` (a model's id in the list) the one sent to `provider`, for the next message and the next launch.
    pub fn choose_model(&mut self, key: &str, provider: &str) {
        if provider == "kilo" && !self.cfg.kilo_enabled {
            self.cfg.kilo_enabled = true;
            config::save_one("kiloEnabled", json!(true));
            if let Some(a) = &mut self.agent {
                a.cfg.take_keys(&self.cfg);
            }
        }
        let model = self.models.iter().find(|m| m.id == key).map(|m| m.request_id(provider).to_string()).unwrap_or_else(|| key.to_string());
        self.model = model.clone();
        self.provider = provider.to_string();
        *self.shared.model.lock().unwrap() = model.clone();
        *self.shared.provider.lock().unwrap() = provider.to_string();
        config::save_one("model", json!(model));
        config::save_one("provider", json!(provider));
    }

    /// The list key of the model being used: its entry's id, or the id itself when the list doesn't have it (auto).
    fn current_key(&self) -> String {
        self.models.iter().find(|m| m.request_id(&self.provider) == self.model).map(|m| m.id.clone()).unwrap_or_else(|| self.model.clone())
    }

    pub fn toggle_star(&mut self, id: String) {
        if let Some(i) = self.starred.iter().position(|s| *s == id) {
            self.starred.remove(i);
        } else {
            self.starred.push(id);
        }
        config::save_one("starredModels", json!(self.starred));
    }

    pub fn set_effort(&mut self, effort: &str) {
        if EFFORTS.contains(&effort) {
            self.effort = effort.to_string();
            *self.shared.effort.lock().unwrap() = self.effort.clone();
            config::save_one("effort", json!(self.effort));
        }
    }

    pub fn set_mode(&mut self, mode: &str) {
        if MODES.contains(&mode) {
            *self.shared.mode.lock().unwrap() = mode.to_string();
        }
    }

    pub fn cycle_mode(&mut self) {
        if !self.running {
            let m = self.shared.mode();
            let next = MODES[(MODES.iter().position(|x| *x == m).unwrap_or(0) + 1) % MODES.len()];
            *self.shared.mode.lock().unwrap() = next.into();
        }
    }

    // ---------- actions ----------

    /// Sends what's typed. While Scout works it waits in the queue, or with `steer` goes into the running turn. False
    /// when it wasn't taken, and should stay in the box.
    pub fn send(&mut self, text: String, files: Vec<String>, steer: bool) -> bool {
        let text = text.trim().to_string();
        if text.is_empty() && files.is_empty() {
            return false;
        }
        if self.running && text.starts_with('/') {
            self.status = "Commands wait until Scout is done. Esc stops it.".into();
            return false;
        }
        if self.running {
            let o = Outgoing { text, files };
            if steer { self.steer(o) } else { self.queue.push(o) }
            return true;
        }
        if let Some(cmd) = text.strip_prefix('/').filter(|_| files.is_empty()) {
            let (name, arg) = cmd.split_once(' ').unwrap_or((cmd, ""));
            match name {
                "new" | "clear" => {
                    self.new_chat();
                    return true;
                }
                "undo" => {
                    self.undo();
                    return true;
                }
                "prune" => {
                    self.prune();
                    return true;
                }
                "compact" => {
                    self.compact(arg.trim().to_string());
                    return true;
                }
                _ => {}
            }
        }
        self.submit(Outgoing { text, files });
        true
    }

    /// Starts a turn with the message.
    fn submit(&mut self, o: Outgoing) {
        let Some(agent) = &mut self.agent else { return };
        let (content, read) = attach::build(&self.cwd, &o.text, &o.files);
        agent.read_files.extend(read);
        self.push(ItemKind::User { text: o.text, files: o.files });
        self.start(move |agent, h| agent.turn(content, h).map_err(|e| agent.failed(e)));
    }

    /// Carries on a turn that ran out of credit, on `model` (its list key and provider) when one is picked.
    pub fn retry(&mut self, model: Option<(String, String)>) {
        if self.running || !matches!(self.items.last().map(|i| &i.kind), Some(ItemKind::OutOfCredit(_))) {
            return;
        }
        if let Some((key, provider)) = model {
            self.choose_model(&key, &provider);
        }
        self.start(|agent, h| agent.retry(h).map_err(|e| agent.failed(e)));
    }

    /// Hands a message to the running turn, which adds it after the step it's on.
    fn steer(&mut self, o: Outgoing) {
        let (content, read) = attach::build(&self.cwd, &o.text, &o.files);
        self.shared.steer.lock().unwrap().push(Steer { text: o.text, files: o.files, content, read });
    }

    /// Takes a message out of the queue and hands it back.
    pub fn unqueue(&mut self, i: usize) -> Option<Outgoing> {
        (i < self.queue.len()).then(|| self.queue.remove(i))
    }

    /// Sends a queued message now: into the running turn, or as the next one.
    pub fn send_queued(&mut self, i: usize) {
        if i < self.queue.len() {
            let o = self.queue.remove(i);
            if self.running { self.steer(o) } else { self.submit(o) }
        }
    }

    /// Flashes the taskbar and/or shows a Windows notification, as the user chose, when the window isn't in front.
    fn alert(&self, title: &str, body: &str) {
        let Some(window) = self.handle.get_webview_window("main") else { return };
        if window.is_focused().unwrap_or(true) {
            return;
        }
        if self.cfg.notify_flash.unwrap_or(true) {
            let _ = window.request_user_attention(Some(tauri::UserAttentionType::Informational));
        }
        if self.cfg.notify_toast.unwrap_or(false) {
            crate::notify::toast(title, &body.chars().take(200).collect::<String>());
        }
    }

    pub fn delete_chat(&mut self, id: &str) {
        let current = self.agent.as_ref().is_some_and(|a| a.session.id == id);
        if let Err(e) = session::delete_session(id) {
            self.status = format!("Could not delete the chat: {e}");
            return;
        }
        self.sessions = session::list_sessions();
        if current {
            self.new_chat();
        }
    }

    /// Rewinds the chat to before item `n`, a message of the user's, restoring the files changed since, and sends
    /// `text` in its place with the same attachments.
    pub fn resend(&mut self, n: usize, text: String) {
        if self.running {
            return;
        }
        let Some(ItemKind::User { text: old, files }) = self.items.get(n).map(|i| &i.kind) else { return };
        let (old, files) = (old.clone(), files.clone());
        // The saved message with the same text, counted from the end, since later ones may repeat it.
        let later = self.items[n + 1..].iter().filter(|i| matches!(&i.kind, ItemKind::User { text, .. } if *text == old)).count();
        let Some(agent) = &mut self.agent else { return };
        let at = agent
            .session
            .messages
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, m)| matches!(m, Message::User { content } if user_parts(content).0 == old))
            .nth(later)
            .map(|(i, _)| i);
        let Some(at) = at else {
            self.push(ItemKind::Notice("That message was summarised by compaction, so it can't be edited.".into()));
            return;
        };
        let (restored, all) = agent.rewind(at);
        let s = agent.session.clone();
        self.context = agent.context_tokens();
        let items = rebuild(&s, &mut self.next_item);
        self.replace_items(items);
        self.tool_index.clear();
        self.todos.clear();
        self.files_epoch += 1;
        let mut note = String::new();
        if !restored.is_empty() {
            note = format!("Restored {}.", restored.join(", "));
        }
        if !all {
            note.push_str(if note.is_empty() { "" } else { " " });
            note.push_str("Some files were changed before this chat was opened in this window, so they weren't restored.");
        }
        if !note.is_empty() {
            self.push(ItemKind::Notice(note));
        }
        self.submit(Outgoing { text, files });
    }

    /// Hands the agent to a worker thread; it comes back in Ev::Finished.
    fn start(&mut self, work: impl FnOnce(&mut Agent, &Hooks) -> Result<(), String> + Send + 'static) {
        let Some(mut agent) = self.agent.take() else { return };
        agent.cfg.take_keys(&self.cfg);
        self.running = true;
        self.stopping = false;
        self.shared.cancel.store(false, Ordering::Relaxed);
        let h = self.hooks.clone();
        std::thread::Builder::new()
            .name("scout-agent".into())
            .spawn(move || {
                let r = work(&mut agent, &h);
                h.send(Ev::Finished(Box::new(agent), r));
            })
            .expect("spawn agent thread");
    }

    pub fn stop(&mut self) {
        if !self.running {
            return;
        }
        self.stopping = true;
        self.shared.cancel.store(true, Ordering::Relaxed);
        // Answering unblocks a worker waiting on the user.
        if let Some(p) = self.permission.take() {
            let _ = p.reply.send(PermissionAnswer::Deny(String::new()));
        }
        if let Some(p) = self.plan.take() {
            let _ = p.reply.send(PlanAnswer::Revise(String::new()));
        }
        if let Some(p) = self.questions.take() {
            let _ = p.reply.send(QuestionAnswer::Skipped);
        }
        self.status = "Stopping…".into();
    }

    pub fn new_chat(&mut self) {
        if self.running {
            return;
        }
        let s = session::new_session(&self.cwd, &self.model);
        self.open_session(s);
    }

    pub fn open_chat(&mut self, id: &str) {
        if let Some(s) = session::load_session(id) {
            self.open_session(s);
        }
    }

    fn open_session(&mut self, s: Session) {
        if self.running {
            return;
        }
        let items = rebuild(&s, &mut self.next_item);
        self.replace_items(items);
        self.tool_index.clear();
        self.todos.clear();
        self.queue.clear();
        self.totals = s.totals.clone();
        self.sessions = session::list_sessions();
        self.chat_epoch += 1;
        self.cwd = s.cwd.clone();
        self.enter_project(&s.cwd);
        let mut agent = Agent::new(self.cfg.clone(), s, self.shared.clone());
        agent.last_prompt_tokens = 0;
        self.context = agent.context_tokens();
        self.agent = Some(agent);
        self.status.clear();
        self.focus_epoch += 1;
    }

    /// Starts a new chat in the folder.
    pub fn change_project(&mut self, dir: String) {
        let s = session::new_session(&dir, &self.model);
        self.open_session(s);
    }

    // ---------- projects: named folders that chats belong to (by the folder they ran in) ----------

    fn project_index(&self, dir: &str) -> Option<usize> {
        self.projects.iter().position(|p| session::same_dir(&p.path, dir))
    }

    /// The current folder's project name.
    fn project_label(&self) -> String {
        self.project_index(&self.cwd).map(|i| self.projects[i].name.clone()).unwrap_or_else(|| project_name(&self.cwd))
    }

    fn save_projects(&self) {
        config::save_one("desktopProjects", json!(self.projects));
    }

    /// Every folder Scout works in is a project; the first visit names it after the folder.
    /// Also remembers it as the folder to open next launch.
    fn enter_project(&mut self, dir: &str) {
        if self.project_index(dir).is_none() {
            self.projects.push(config::Project { name: project_name(dir), path: dir.to_string() });
            self.save_projects();
        }
        config::save_one("desktopProject", json!(dir));
    }

    /// Starts a chat in the folder the user picked. True when it's a new project, which they then get to name.
    pub fn add_project(&mut self, dir: String) -> bool {
        if self.running {
            return false;
        }
        let is_new = self.project_index(&dir).is_none();
        self.change_project(dir);
        is_new
    }

    pub fn rename_project(&mut self, path: &str, name: &str) {
        let name = name.trim();
        if let Some(i) = self.project_index(path).filter(|_| !name.is_empty()) {
            self.projects[i].name = name.to_string();
            self.save_projects();
        }
    }

    /// The open project stays, so the chat on screen always has one.
    pub fn remove_project(&mut self, path: &str) {
        if session::same_dir(path, &self.cwd) {
            return;
        }
        if let Some(i) = self.project_index(path) {
            self.projects.remove(i);
            self.save_projects();
        }
    }

    pub fn undo(&mut self) {
        let Some(agent) = &mut self.agent else { return };
        let (msg, rebuilt) = match agent.undo() {
            None => ("Nothing to undo.".to_string(), None),
            Some((label, files, conv)) => {
                let s = agent.session.clone();
                (
                    format!(
                        "Undid \"{}\"{}{}",
                        label.chars().take(60).collect::<String>(),
                        if conv { "" } else { " (files only: the conversation was compacted since)" },
                        if files.is_empty() { String::new() } else { format!("; restored {}", files.join(", ")) }
                    ),
                    Some(s),
                )
            }
        };
        if let Some(s) = rebuilt {
            let items = rebuild(&s, &mut self.next_item);
            self.replace_items(items);
        }
        self.files_epoch += 1;
        self.push(ItemKind::Notice(msg));
    }

    pub fn prune(&mut self) {
        let Some(agent) = &mut self.agent else { return };
        let msg = match agent.prune(0, 0) {
            Some((n, freed)) => format!("Cleared {n} old tool output{} (~{}k tokens).", if n == 1 { "" } else { "s" }, freed / 1000),
            None => "No old tool output to clear.".into(),
        };
        self.context = agent.context_tokens();
        self.push(ItemKind::Notice(msg));
    }

    pub fn compact(&mut self, focus: String) {
        self.start(move |agent, h| match agent.compact(&focus, h) {
            Ok(true) => {
                h.send(Ev::Notice("Compacted the conversation.".into()));
                Ok(())
            }
            Ok(false) => {
                h.send(Ev::Notice("Nothing to compact yet.".into()));
                Ok(())
            }
            Err(e) => Err(e.message),
        });
    }

    pub fn answer_permission(&mut self, a: PermissionAnswer) {
        if let Some(p) = self.permission.take() {
            let _ = p.reply.send(a);
        }
    }

    pub fn answer_plan(&mut self, a: PlanAnswer) {
        if let Some(p) = self.plan.take() {
            let _ = p.reply.send(a);
        }
    }

    pub fn answer_questions(&mut self, a: QuestionAnswer) {
        if let Some(p) = self.questions.take() {
            let _ = p.reply.send(a);
        }
    }

    pub fn answer_link(&mut self, a: PermissionAnswer) {
        if let Some(p) = self.link_permission.take() {
            let _ = p.reply.send(a);
        }
    }

    /// The first thing asked in this chat, as its title.
    fn chat_title(&self) -> String {
        let first = self.items.iter().find_map(|i| if let ItemKind::User { text, .. } = &i.kind { Some(text.lines().next().unwrap_or("").trim().to_string()) } else { None });
        first.filter(|t| !t.is_empty()).unwrap_or_else(|| "New chat".into())
    }

    /// Starts the new build on the same folder and chat. False when it can't: the window checks for unsaved edits first.
    pub fn restart_to_update(&mut self) -> bool {
        if self.running {
            return false;
        }
        let mut args = vec![self.cwd.clone()];
        if let Some(agent) = &self.agent {
            args.extend(["--resume".to_string(), agent.session.id.clone()]);
        }
        match self.updater.restart(&args) {
            Ok(()) => true,
            Err(e) => {
                self.status = e;
                false
            }
        }
    }

    pub fn updater(&self) -> &crate::update::Updater {
        &self.updater
    }

    /// Runs as the app closes. A waiting update goes in now; if it can't, it's found again at the next start.
    pub fn on_exit(&mut self) {
        tools::kill_background_shells();
        let _ = self.updater.install();
    }

    pub fn set_auto_update(&mut self, on: bool) {
        self.cfg.auto_update = on;
        config::save_one("desktopAutoUpdate", json!(on));
        self.updater.set_auto(on);
    }

    pub fn set_notify(&mut self, flash: bool, toast: bool) {
        self.cfg.notify_flash = Some(flash);
        self.cfg.notify_toast = Some(toast);
        let mut m = serde_json::Map::new();
        m.insert("desktopFlash".into(), json!(flash));
        m.insert("desktopToast".into(), json!(toast));
        config::save_config(m);
    }

    /// Asks the server for the account's limits, after making a change when `change` says what to send and say.
    pub fn usage_request(&mut self, change: Option<(Value, String)>) {
        if self.cfg.api_key.is_none() || self.usage.busy {
            return;
        }
        self.usage.busy = true;
        let cfg = self.cfg.clone();
        let h = self.hooks.clone();
        std::thread::spawn(move || {
            let (r, said) = match change {
                Some((body, said)) => (crate::api::usage_action(&cfg, &body), Some(said)),
                None => (crate::api::usage_info(&cfg), None),
            };
            h.send(Ev::Usage(r.map_err(|e| e.message), said));
        });
    }

    pub fn refresh_skills(&mut self) {
        self.skills = crate::skills::list();
    }

    fn stop_link(&mut self, status: &str) {
        if let Some(l) = self.link.take() {
            l.stop();
        }
        self.link_permission = None;
        self.link_status = status.to_string();
    }

    /// Turns the link for web chatbots on or off.
    pub fn toggle_link(&mut self, mode: &str) {
        if self.link.is_some() {
            self.stop_link("Off.");
            return;
        }
        if self.cfg.api_key.is_none() || self.cwd.is_empty() {
            return;
        }
        let h = self.hooks.clone();
        self.link_log.clear();
        self.link_status = "Connecting…".into();
        self.link = Some(crate::mcp::start_link(&self.cfg, &self.cwd, mode, move |e| h.send(Ev::Link(e))));
    }

    pub fn set_status(&mut self, s: String) {
        self.status = s;
    }

    // ---------- telling the window ----------

    /// Everything the window draws from, by key. Keys go only to the window, so no secrets: keys are masked here.
    fn view(&self) -> Vec<(&'static str, Value)> {
        let cfg = &self.cfg;
        let hub = crate::mcp_client::hub();
        let steering: Vec<String> = self.shared.steer.lock().unwrap().iter().map(|s| s.text.clone()).collect();
        let login_needed = cfg.api_key.is_none() && cfg.openrouter_keys.is_empty() && cfg.nvidia_key.is_none() && !cfg.kilo_on() && cfg.copilot_token.is_none() && !cfg.devin_enabled && !cfg.kiro_enabled && cfg.additional.is_empty();
        let voice = match &self.voice {
            Voice::Idle => json!({ "state": "idle" }),
            // Rounded so that it only goes out when it moves.
            Voice::Recording(r) => json!({ "state": "recording", "level": (r.level().sqrt() * 20.0).round() / 20.0 }),
            Voice::Busy(s) => json!({ "state": "busy", "label": s }),
        };
        let kilo_check = self.login.kilo_check.as_ref().map(|r| match r {
            Ok(m) => json!({ "ok": true, "message": m }),
            Err(m) => json!({ "ok": false, "message": m }),
        });
        let mcp_views: Vec<Value> = hub
            .views()
            .iter()
            .map(|s| {
                use crate::mcp_client::{Status as S, join_args, join_pairs};
                let (state, error) = match &s.status {
                    S::Off => ("off", None),
                    S::Connecting => ("connecting", None),
                    S::Ready => ("ready", None),
                    S::Failed(e) => ("failed", Some(e.clone())),
                };
                json!({ "name": s.name, "summary": s.cfg.summary(), "disabled": s.cfg.disabled, "state": state, "error": error, "tools": s.tools,
                    "form": { "remote": !s.cfg.url.is_empty(), "command": s.cfg.command, "args": join_args(&s.cfg.args), "env": join_pairs(&s.cfg.env, "="), "url": s.cfg.url, "headers": join_pairs(&s.cfg.headers, ": ") } })
            })
            .collect();
        vec![
            ("cfg", json!({
                "loginNeeded": login_needed,
                "hasKey": cfg.api_key.is_some(),
                "browserSignIn": config::is_sign_in(cfg.api_key.as_deref()),
                "username": cfg.username,
                "baseUrl": cfg.base_url,
                "openrouterKeys": cfg.openrouter_keys.iter().map(|k| json!({ "name": k.name, "masked": mask_key(&k.key) })).collect::<Vec<_>>(),
                "nvidiaKey": cfg.nvidia_key.as_deref().map(mask_key),
                "kiloKey": cfg.kilo_key.as_deref().map(mask_key),
                "copilot": cfg.copilot_token.is_some(),
                "copilotUser": cfg.copilot_user,
                "devin": cfg.devin_enabled,
                "kiro": cfg.kiro_enabled,
                "additional": cfg.additional.iter().map(|l| json!({ "id": l.id, "masked": l.key.as_deref().map(mask_key) })).collect::<Vec<_>>(),
                "notifyFlash": cfg.notify_flash,
                "notifyToast": cfg.notify_toast,
                "autoUpdate": cfg.auto_update,
                "autoCompactTokens": cfg.auto_compact_tokens,
            })),
            ("mode", json!(self.shared.mode())),
            ("model", json!({ "id": self.model, "provider": self.provider, "key": self.current_key(), "short": crate::api::short_id(&self.model) })),
            ("effort", json!(self.effort)),
            ("starred", json!(self.starred)),
            ("modelsLoading", json!(self.models_loading)),
            ("additionalErrors", json!(self.additional_errors)),
            ("kiroModelsError", json!(self.kiro_models_error)),
            ("run", json!({ "running": self.running, "stopping": self.stopping, "status": self.status })),
            ("todos", todos_json(&self.todos)),
            ("totals", json!({ "totals": self.totals, "context": self.context })),
            ("permission", json!(self.permission.as_ref().map(permission_json))),
            ("plan", json!(self.plan.as_ref().map(|p| &p.text))),
            ("questions", json!(self.questions.as_ref().map(|p| p.questions.iter().map(|q| json!({
                "question": q.question, "header": q.header, "multiSelect": q.multi_select,
                "options": q.options.iter().map(|o| json!({ "label": o.label, "description": o.description })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>()))),
            ("queue", json!(self.queue.iter().map(|q| json!({ "text": q.text, "files": q.files.len() })).collect::<Vec<_>>())),
            ("steering", json!(steering)),
            ("sessions", json!(self.sessions.iter().map(|s| json!({ "id": s.id, "cwd": s.cwd, "title": s.title, "ago": session::ago(s.updated) })).collect::<Vec<_>>())),
            ("projects", json!(self.projects)),
            ("chat", json!({ "id": self.agent.as_ref().map(|a| a.session.id.clone()), "cwd": self.cwd, "project": self.project_label(), "title": self.chat_title(), "epoch": self.chat_epoch })),
            ("filesEpoch", json!(self.files_epoch)),
            ("focusEpoch", json!(self.focus_epoch)),
            ("login", json!({
                "busy": self.login.busy, "url": self.login.url, "error": self.login.error, "kiloCheck": kilo_check,
                "copilot": self.login.copilot.as_ref().map(SignIn::json), "copilotError": self.login.copilot_error,
                "devin": self.login.devin.as_ref().map(SignIn::json), "devinError": self.login.devin_error,
                "kiro": self.login.kiro.as_ref().map(SignIn::json), "kiroError": self.login.kiro_error,
            })),
            ("skills", json!(self.skills.iter().map(|s| json!({ "name": s.name, "about": s.short_description(), "source": s.source, "dir": s.dir })).collect::<Vec<_>>())),
            ("link", json!({
                "on": self.link.is_some(),
                "cwd": self.link.as_ref().map(|l| l.cwd.clone()).unwrap_or_else(|| self.cwd.clone()),
                "status": self.link_status, "log": self.link_log,
                "permission": self.link_permission.as_ref().map(permission_json),
            })),
            ("usage", json!({ "info": self.usage.info, "busy": self.usage.busy, "message": self.usage.message })),
            ("voice", voice),
            ("updater", json!({ "enabled": self.updater.enabled(), "version": crate::update::VERSION, "status": status_json(&self.updater.status()) })),
            ("mcp", json!({ "error": hub.error(), "servers": mcp_views })),
        ]
    }

    /// Sends the window what changed: keys of the view, then models, then transcript items.
    pub fn flush(&mut self) {
        self.seq += 1;
        let seq = self.seq;
        let mut patch = serde_json::Map::new();
        for (key, value) in self.view() {
            let text = value.to_string();
            if self.sent.get(key) != Some(&text) {
                self.sent.insert(key, text);
                patch.insert(key.to_string(), value);
            }
        }
        if !patch.is_empty() {
            let _ = self.handle.emit("view", json!({ "seq": seq, "data": patch }));
        }
        if self.models_changed {
            self.models_changed = false;
            let _ = self.handle.emit("models", json!({ "seq": seq, "data": self.models.iter().map(model_json).collect::<Vec<_>>() }));
        }
        if self.reset {
            self.reset = false;
            self.changed.clear();
            let _ = self.handle.emit("items", json!({ "seq": seq, "reset": true, "items": self.items.iter().map(item_json).collect::<Vec<_>>() }));
        } else if !self.changed.is_empty() {
            let items: Vec<Value> = self.changed.iter().filter_map(|&i| self.items.get(i).map(|it| json!({ "at": i, "item": item_json(it) }))).collect();
            self.changed.clear();
            let _ = self.handle.emit("items", json!({ "seq": seq, "reset": false, "changes": items }));
        }
    }

    /// Everything, for a window that's just opened (or reloaded while developing).
    pub fn snapshot(&mut self) -> Value {
        let mut view = serde_json::Map::new();
        for (key, value) in self.view() {
            self.sent.insert(key, value.to_string());
            view.insert(key.to_string(), value);
        }
        self.reset = false;
        self.changed.clear();
        self.models_changed = false;
        json!({
            "seq": self.seq,
            "view": view,
            "models": self.models.iter().map(model_json).collect::<Vec<_>>(),
            "items": self.items.iter().map(item_json).collect::<Vec<_>>(),
            "modes": MODES.iter().map(|m| json!({ "id": m, "label": config::mode_label(m) })).collect::<Vec<_>>(),
            "efforts": EFFORTS,
            "additionalProviders": config::ADDITIONAL.iter().map(|a| json!({ "id": a.id, "name": a.name, "needsKey": a.needs_key, "note": a.note, "keyUrl": a.key_url, "keyLink": a.key_link, "keyHint": a.key_hint })).collect::<Vec<_>>(),
            "kiloWarning": config::KILO_WARNING,
            "plans": { "copilot": crate::copilot::PLAN_URL, "devin": crate::devin::PLAN_URL, "kiro": crate::kiro::PLAN_URL },
            "languages": crate::instructions::LANGUAGES,
            "exe": std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "Scout.exe".into()),
        })
    }
}

/// Runs the workers' events into the core and tells the window, at most about 60 times a second.
pub fn pump(core: Arc<Mutex<Core>>, rx: Receiver<Ev>, wake: Receiver<()>) {
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::Duration;
    loop {
        // The mic's level has no event of its own.
        let wait = if core.lock().unwrap().is_recording() { Duration::from_millis(50) } else { Duration::from_secs(1) };
        match wake.recv_timeout(wait) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        while wake.try_recv().is_ok() {}
        {
            let mut core = core.lock().unwrap();
            while let Ok(ev) = rx.try_recv() {
                core.handle(ev);
            }
            core.flush();
        }
        std::thread::sleep(Duration::from_millis(16));
    }
}
