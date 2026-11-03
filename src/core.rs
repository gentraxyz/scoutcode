//! The app's state behind the window: the agent, the transcript, sign-ins and settings. The web UI in `ui/` draws what
//! `view` says and calls these methods through `bridge.rs`; what's only about drawing (menus, drafts) lives over there.

use crate::agent::{Agent, Ev, Hooks, PermissionAnswer, PermissionRequest, PlanAnswer, Question, QuestionAnswer, Shared, Steer};
use crate::api::{Content, Message, ModelInfo};
use crate::attach;
use crate::config::{self, Config, EFFORTS, MODES, ModelRef};
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
    /// A model failed and no fallbacks are set up yet, so the window offers to set some. Says why, and which model.
    FallbackSetup(String, ModelRef),
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

/// Signing in to Copilot, Devin, Kiro, Codex, Claude Code, OpenCode, Grok or Hermes, while it's going on.
#[derive(Default)]
struct SignIn {
    /// The code to enter, and where. Copilot, OpenCode, Grok and Hermes only.
    code: Option<(String, String)>,
    /// Bytes downloaded so far, of how many.
    progress: (u64, u64),
    installed: bool,
    /// The CLI is there and the browser should be open.
    waiting: bool,
    /// The page the browser should be open on, when the CLI said. Codex and Claude Code only.
    page: Option<String>,
    cancel: Arc<AtomicBool>,
}

impl SignIn {
    fn json(&self) -> Value {
        json!({ "code": self.code.as_ref().map(|c| &c.0), "url": self.code.as_ref().map(|c| &c.1).or(self.page.as_ref()), "done": self.progress.0, "total": self.progress.1, "installed": self.installed, "waiting": self.waiting })
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
    codex: Option<SignIn>,
    codex_error: Option<String>,
    claude: Option<SignIn>,
    claude_error: Option<String>,
    opencode: Option<SignIn>,
    opencode_error: Option<String>,
    grok: Option<SignIn>,
    grok_error: Option<String>,
    hermes: Option<SignIn>,
    hermes_error: Option<String>,
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

/// One chat and whatever Scout is doing in it. Each runs on its own, on screen or not.
struct Chat {
    /// What its worker's events are marked with.
    key: u64,
    /// The saved chat's id, also while a worker has the agent.
    session_id: String,
    shared: Arc<Shared>,
    /// None while a worker thread owns it.
    agent: Option<Agent>,
    items: Vec<Item>,
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
    /// The model id to send, for `provider` ("scoutcode", "openrouter", "nvidia" or "kilo").
    model: String,
    provider: String,
    effort: String,
    permission: Option<PendingPermission>,
    plan: Option<PendingPlan>,
    questions: Option<PendingQuestions>,
    /// The chat's folder, also while a worker has the agent.
    cwd: String,
    /// Messages sent while Scout worked, sent in order once it finishes.
    queue: Vec<Outgoing>,
    /// It finished while another chat was on screen.
    unread: bool,
    /// The turn is running on a fallback because the chosen model failed.
    fallback: Option<FallbackRun>,
}

/// A turn handed from model to model as each fails.
struct FallbackRun {
    /// The model the user chose, to go back to once the turn ends.
    from: ModelRef,
    /// The fallbacks not tried yet.
    left: Vec<ModelRef>,
}

impl Chat {
    /// A chat opened on a saved or new conversation, with the settings it starts on.
    fn new(key: u64, s: Session, cfg: &Config, shared: Arc<Shared>) -> Self {
        let (model, provider, effort) = (shared.model.lock().unwrap().clone(), shared.provider.lock().unwrap().clone(), shared.effort.lock().unwrap().clone());
        let (session_id, cwd, totals) = (s.id.clone(), s.cwd.clone(), s.totals.clone());
        let mut agent = Agent::new(cfg.clone(), s, shared.clone());
        agent.last_prompt_tokens = 0;
        Chat {
            key,
            session_id,
            shared,
            context: agent.context_tokens(),
            agent: Some(agent),
            items: vec![],
            changed: BTreeSet::new(),
            reset: true,
            streaming: None,
            partial: String::new(),
            tool_index: HashMap::new(),
            running: false,
            stopping: false,
            status: String::new(),
            todos: vec![],
            totals,
            model,
            provider,
            effort,
            permission: None,
            plan: None,
            questions: None,
            cwd,
            queue: vec![],
            unread: false,
            fallback: None,
        }
    }

    /// Scout is held up until the user answers.
    fn waiting(&self) -> bool {
        self.permission.is_some() || self.plan.is_some() || self.questions.is_some()
    }

    /// The first thing asked in this chat, as its title.
    fn title(&self) -> String {
        let first = self.items.iter().find_map(|i| if let ItemKind::User { text, .. } = &i.kind { Some(text.lines().next().unwrap_or("").trim().to_string()) } else { None });
        first.filter(|t| !t.is_empty()).unwrap_or_else(|| "New chat".into())
    }
}

pub struct Core {
    handle: AppHandle,
    cfg: Config,
    /// The chat on screen.
    chat: Chat,
    /// Chats working behind it, and ones that finished there and haven't been opened since.
    background: Vec<Chat>,
    /// Handling an event for a chat in `background`, which is in `chat` for as long as that takes.
    behind: bool,
    next_chat: u64,
    hooks: Hooks,
    next_item: u64,
    models: Vec<ModelInfo>,
    models_changed: bool,
    additional_errors: Vec<String>,
    /// The additional provider to choose a model on once its list is in: it was linked with nothing else to run on.
    start_additional: Option<&'static str>,
    /// The user just chose to go without an account: move to a free model once the list has them.
    start_guest: bool,
    /// Hermes was just connected: choose its first free model once it's in the list.
    start_hermes: bool,
    hermes_models_error: Option<String>,
    kiro_models_error: Option<String>,
    codex_models_error: Option<String>,
    claude_models_error: Option<String>,
    opencode_models_error: Option<String>,
    grok_models_error: Option<String>,
    /// Every saved chat, from every folder; the sidebar groups them by project.
    sessions: Vec<SessionInfo>,
    projects: Vec<config::Project>,
    starred: Vec<String>,
    /// How many model lists are still being fetched.
    models_loading: u32,
    login: Login,
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
    /// Whether ScoutCode answered the link's last poll.
    link_online: bool,
    /// When the link was turned on, in milliseconds since 1970.
    link_since: u64,
    /// The chatbot's call being run now.
    link_running: Option<String>,
    /// The chatbot's latest tool calls, oldest first.
    link_log: Vec<crate::mcp::LinkCall>,
    /// Every file the chatbots changed since the link was turned on, with the lines added and removed in all.
    link_edits: Vec<crate::mcp::LinkEdit>,
    link_chatbots: Vec<crate::mcp::Chatbot>,
    link_permission: Option<PendingPermission>,
    usage: Usage,
    voice: Voice,
    updater: crate::update::Updater,
    /// The releases since the version that ran here before, shown once after an update.
    whats_new: Vec<crate::update::ReleaseNote>,
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

// ---------- model lists ----------

/// The providers whose models are listed, by the id `Config::provider` gives them, in the order the list shows them.
fn model_sources(cfg: &Config) -> Vec<&'static str> {
    let mut out = vec![];
    if cfg.api_key.is_some() || cfg.guest {
        out.push("scoutcode");
    }
    if !cfg.openrouter_keys.is_empty() {
        out.push("openrouter");
    }
    if cfg.nvidia_key.is_some() {
        out.push("nvidia");
    }
    // Always asked for: Kilo's free models need no key.
    out.push("kilo");
    out.extend(config::ADDITIONAL.iter().filter(|a| cfg.linked(a.id).is_some()).map(|a| a.id));
    let on = [("copilot", cfg.copilot_token.is_some()), ("devin", cfg.devin_enabled), ("kiro", cfg.kiro_enabled), ("codex", cfg.codex_enabled), ("claude", cfg.claude_enabled), ("opencode", cfg.opencode_enabled), ("grok", cfg.grok_on()), ("hermes", cfg.hermes_on())];
    out.extend(on.into_iter().filter(|(_, on)| *on).map(|(s, _)| s));
    out
}

/// An entry for a model a CLI provider runs. These are their own, under "copilot/…" and the like.
fn cli_model(source: &str, id: String, name: String) -> ModelInfo {
    let mut m = ModelInfo { id: format!("{source}/{id}"), name: Some(name), ..Default::default() };
    match source {
        "copilot" => m.copilot_id = Some(id),
        "devin" => m.devin_id = Some(id),
        "kiro" => m.kiro_id = Some(id),
        "codex" => m.codex_id = Some(id),
        "claude" => m.claude_id = Some(id),
        "opencode" => {
            m.free = crate::opencode::is_free(&id);
            m.opencode_id = Some(id);
        }
        _ => m.grok_id = Some(id),
    }
    m
}

/// What a CLI provider's list has even when it can't be read: its default, which needs no list. It's all Copilot's free
/// plan gets, lets Devin pick for the plan, and is what a new Codex, Claude Code, OpenCode or Grok session starts on.
fn default_models(source: &str) -> Vec<ModelInfo> {
    let name = match source {
        "copilot" => "Copilot Auto",
        "devin" => "Devin Default",
        "kiro" => "Kiro Auto",
        "codex" => "Codex Default",
        "claude" => "Claude Code Default",
        "opencode" => "OpenCode Default",
        "grok" => "Grok Default",
        _ => return vec![],
    };
    vec![cli_model(source, "auto".into(), name.into())]
}

/// Asks one provider for its models. The CLI providers tell the window why theirs couldn't be read.
fn fetch_source(cfg: &Config, source: &str, h: &Hooks) -> Result<Vec<ModelInfo>, String> {
    let with_default = |listed: Vec<(String, String)>| default_models(source).into_iter().chain(listed.into_iter().filter(|(id, _)| id != "auto").map(|(id, name)| cli_model(source, id, name))).collect();
    match source {
        "scoutcode" => {
            let (models, user) = crate::api::list_models(cfg).map_err(|e| e.message)?;
            if let Some(u) = user {
                h.send(Ev::User(u));
            }
            Ok(models)
        }
        // The first key, in order, that lists the models is used.
        "openrouter" => {
            let mut failed = String::new();
            for k in &cfg.openrouter_keys {
                let mut only = cfg.clone();
                only.openrouter_keys = vec![k.clone()];
                match crate::api::list_openrouter(&only) {
                    Ok(models) => return Ok(models),
                    Err(e) => failed = e.message,
                }
            }
            Err(format!("OpenRouter: {failed}"))
        }
        "nvidia" => crate::api::list_nvidia(cfg).map_err(|e| format!("NVIDIA NIM: {}", e.message)),
        "kilo" => crate::api::list_kilo(cfg).map_err(|e| format!("Kilo Code: {}", e.message)),
        "copilot" => Ok(with_default(crate::copilot::list_models(cfg.copilot_token.as_deref().unwrap_or_default()))),
        "devin" => Ok(with_default(crate::devin::list_models())),
        "codex" | "claude" => {
            let (which, listed, explain): (_, _, fn(&String) -> String) = match source {
                "codex" => ("codex", crate::codex::list_models(), |e| crate::codex::explain(e)),
                _ => ("claude", crate::claude::list_models(), |e| crate::claude::explain(e)),
            };
            let error = listed.as_ref().err().map(explain);
            h.send(Ev::AgentModelsError(which, error.clone()));
            listed.map(with_default).map_err(|_| error.unwrap_or_default())
        }
        "kiro" | "opencode" | "grok" => {
            let (listed, explain): (_, fn(&String) -> String) = match source {
                "kiro" => (crate::kiro::list_models(), |e| crate::kiro::explain(e)),
                "opencode" => (crate::opencode::list_models(), |e| crate::opencode::explain(e)),
                _ => (crate::grok::list_models(), |e| crate::grok::explain(e)),
            };
            let error = listed.as_ref().err().map(explain);
            h.send(match source {
                "kiro" => Ev::KiroModelsError(error.clone()),
                "opencode" => Ev::OpencodeModelsError(error.clone()),
                _ => Ev::GrokModelsError(error.clone()),
            });
            listed.map(with_default).map_err(|_| error.unwrap_or_default())
        }
        "hermes" => {
            let listed = crate::api::list_hermes().map_err(|e| format!("Could not load Hermes' models: {}", e.message));
            h.send(Ev::HermesModelsError(listed.as_ref().err().cloned()));
            listed
        }
        _ => {
            let a = config::additional_provider(source).ok_or_else(|| format!("{source}: not a provider"))?;
            crate::api::list_additional(cfg, a).map_err(|e| format!("{}: {}", a.name, e.message))
        }
    }
}

/// One list from each provider's that's on: the one just read, else the one saved last time, else its default.
fn assemble(sources: &[&str], lists: &HashMap<String, Vec<ModelInfo>>) -> Vec<ModelInfo> {
    let get = |s: &str| if sources.contains(&s) { lists.get(s).cloned().unwrap_or_else(|| default_models(s)) } else { vec![] };
    let mut models = crate::api::merge_models(get("scoutcode"), get("openrouter"), get("nvidia"), get("kilo"));
    for s in sources.iter().filter(|s| !["scoutcode", "openrouter", "nvidia", "kilo"].contains(s)) {
        models.extend(get(s));
    }
    models
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
        Display::Image { path, preview } => json!({ "type": "image", "path": path, "preview": preview }),
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
        ItemKind::FallbackSetup(text, m) => json!({ "id": item.id, "kind": "fallbackSetup", "text": text, "key": m.key, "provider": m.provider }),
    }
}

fn permission_json(p: &PendingPermission) -> Value {
    json!({ "tool": p.req.tool, "label": p.req.label, "preview": p.req.preview.as_ref().map(display_json), "prefix": p.req.prefix, "reason": p.req.reason })
}

fn model_json(m: &ModelInfo) -> Value {
    json!({
        "id": m.id, "label": m.label(), "description": m.description, "context": m.context, "tools": m.tools, "vision": m.vision, "cost": m.cost,
        "providers": m.providers, "routesTo": m.routes_to, "available": m.available, "openrouterId": m.openrouter_id, "nvidiaId": m.nvidia_id, "opencodeId": m.opencode_id,
        "kiloId": m.kilo_id, "kiloFree": m.kilo_free, "kiloTier": m.kilo_tier, "copilotId": m.copilot_id, "devinId": m.devin_id, "kiroId": m.kiro_id, "codexId": m.codex_id, "claudeId": m.claude_id, "grokId": m.grok_id, "hermesId": m.hermes_id,
        "additional": m.additional, "hosted": m.hosted, "free": m.free, "tier": m.tier, "openrouterTier": m.openrouter_tier,
    })
}

fn status_json(s: &Status) -> Value {
    match s {
        Status::Idle => json!({ "state": "idle" }),
        Status::Checking => json!({ "state": "checking" }),
        Status::UpToDate => json!({ "state": "upToDate" }),
        Status::Available { version, summary, changelog } => json!({ "state": "available", "version": version, "summary": summary, "changelog": changelog }),
        Status::Downloading { version, got, total } => json!({ "state": "downloading", "version": version, "got": got, "total": total }),
        Status::Ready { version, summary, changelog } => json!({ "state": "ready", "version": version, "summary": summary, "changelog": changelog }),
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
    let (text, files) = attach::split(crate::prompt::strip_mode_note(&t));
    (crate::prompt::strip_command_note(&text).to_string(), files)
}

impl Core {
    pub fn new(handle: AppHandle, tx: Sender<Ev>, waker: Waker) -> Self {
        let cfg = config::load_config();
        let hooks = Hooks { chat: None, tx, ctx: waker.clone() };
        let shared = Arc::new(Shared {
            mode: Mutex::new(if MODES.contains(&cfg.mode.as_str()) { cfg.mode.clone() } else { "default".into() }),
            model: Mutex::new(cfg.model.clone()),
            provider: Mutex::new(cfg.provider.clone()),
            effort: Mutex::new(cfg.effort.clone()),
            cancel: Default::default(),
            edit_tools: Default::default(),
            model_names: Default::default(),
            no_vision: Default::default(),
            steer: Default::default(),
        });
        let cwd = initial_cwd();
        let chat = Chat::new(0, session::new_session(&cwd, &cfg.model), &cfg, shared);
        let updater = crate::update::Updater::start(&waker, cfg.auto_update);
        crate::mcp_client::hub().reload(&waker, false);
        let mut core = Core {
            handle,
            chat,
            background: vec![],
            behind: false,
            next_chat: 0,
            additional_errors: vec![],
            start_additional: None,
            start_guest: false,
            start_hermes: false,
            hermes_models_error: None,
            opencode_models_error: None,
            grok_models_error: None,
            kiro_models_error: None,
            codex_models_error: None,
            claude_models_error: None,
            projects: cfg.projects.clone(),
            starred: cfg.starred_models.clone(),
            cfg,
            hooks,
            next_item: 0,
            models: vec![],
            models_changed: true,
            sessions: session::list_sessions(),
            models_loading: 0,
            login: Login::default(),
            link: None,
            skills: crate::skills::list(),
            files_epoch: 0,
            focus_epoch: 0,
            chat_epoch: 0,
            link_status: String::new(),
            link_online: false,
            link_since: 0,
            link_running: None,
            link_log: vec![],
            link_edits: vec![],
            link_chatbots: vec![],
            link_permission: None,
            usage: Usage::default(),
            voice: Voice::Idle,
            updater,
            whats_new: vec![],
            sent: HashMap::new(),
            seq: 0,
        };
        core.enter_project(&cwd);
        core.update_discord();
        // The lists saved last time show at once; they're asked for again straight after.
        let cached = crate::api::load_model_cache();
        if !cached.is_empty() {
            core.set_models(assemble(&model_sources(&core.cfg), &cached));
        }
        core.fetch_models();
        core.check_updated();
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
        self.chat.cwd.clone()
    }

    fn push(&mut self, kind: ItemKind) {
        self.next_item += 1;
        self.chat.changed.insert(self.chat.items.len());
        self.chat.items.push(Item { id: self.next_item, kind });
    }

    fn replace_items(&mut self, items: Vec<Item>) {
        self.chat.items = items;
        self.chat.reset = true;
        self.chat.changed.clear();
    }

    /// Asks every provider that's on for its models and merges them. The chosen model's provider goes first and the list
    /// is sent as soon as it's in; the rest are asked together after it. A list that can't be read is the one saved last
    /// time, and every list that's read is saved for the next start.
    fn fetch_models(&mut self) {
        self.models_loading += 1;
        let cfg = self.cfg.clone();
        let h = self.hooks.clone();
        std::thread::spawn(move || {
            let sources = model_sources(&cfg);
            let mut lists = crate::api::load_model_cache();
            let first = sources.iter().copied().find(|s| *s == cfg.provider);
            let mut results = vec![];
            if let Some(s) = first {
                let r = fetch_source(&cfg, s, &h);
                if let Ok(list) = &r {
                    lists.insert(s.to_string(), list.clone());
                }
                h.send(Ev::Models(Ok(assemble(&sources, &lists)), false));
                results.push((s, r));
            }
            let rest: Vec<&'static str> = sources.iter().copied().filter(|s| Some(*s) != first).collect();
            results.extend(std::thread::scope(|scope| {
                let running: Vec<_> = rest
                    .iter()
                    .map(|&s| {
                        let (cfg, h) = (&cfg, h.clone());
                        (s, scope.spawn(move || fetch_source(cfg, s, &h)))
                    })
                    .collect();
                running.into_iter().map(|(s, t)| (s, t.join().unwrap_or_else(|_| Err(format!("{s}: the list could not be read"))))).collect::<Vec<_>>()
            }));
            let (mut errors, mut additional_errors) = (vec![], vec![]);
            for (s, r) in results {
                match r {
                    Ok(list) => {
                        lists.insert(s.to_string(), list);
                    }
                    Err(e) if config::additional_provider(s).is_some() => additional_errors.push(e),
                    Err(e) => errors.push(e),
                }
            }
            crate::api::save_model_cache(&lists);
            errors.extend(additional_errors.iter().cloned());
            h.send(Ev::AdditionalErrors(additional_errors));
            let models = assemble(&sources, &lists);
            h.send(Ev::Models(if models.is_empty() && !errors.is_empty() { Err(errors.join("; ")) } else { Ok(models) }, true));
        });
    }

    /// Shows a model list, and tells the agent what it needs to know about each model.
    fn set_models(&mut self, m: Vec<ModelInfo>) {
        // The agent picks each model's edit tools from these, under the id it sends on any provider.
        *self.chat.shared.edit_tools.lock().unwrap() = m.iter().flat_map(|m| std::iter::once(m.id.clone()).chain(m.openrouter_id.clone()).chain(m.nvidia_id.clone()).chain(m.opencode_id.clone()).chain(m.copilot_id.clone()).chain(m.devin_id.clone()).chain(m.kiro_id.clone()).chain(m.codex_id.clone()).chain(m.claude_id.clone()).chain(m.grok_id.clone()).chain(m.hermes_id.clone()).chain(m.additional.as_ref().map(|(_, id)| id.clone())).map(move |id| (id, m.edit_tool))).collect();
        *self.chat.shared.model_names.lock().unwrap() = m.iter().flat_map(|m| std::iter::once(m.id.clone()).chain(m.openrouter_id.clone()).chain(m.nvidia_id.clone()).chain(m.opencode_id.clone()).chain(m.copilot_id.clone()).chain(m.devin_id.clone()).chain(m.kiro_id.clone()).chain(m.codex_id.clone()).chain(m.claude_id.clone()).chain(m.grok_id.clone()).chain(m.hermes_id.clone()).chain(m.additional.as_ref().map(|(_, id)| id.clone())).map(move |id| (id, m.label()))).collect();
        *self.chat.shared.no_vision.lock().unwrap() = m.iter().filter(|m| m.vision == Some(false)).flat_map(|m| std::iter::once(m.id.clone()).chain(m.openrouter_id.clone()).chain(m.nvidia_id.clone()).chain(m.opencode_id.clone()).chain(m.copilot_id.clone()).chain(m.devin_id.clone()).chain(m.kiro_id.clone()).chain(m.codex_id.clone()).chain(m.claude_id.clone()).chain(m.grok_id.clone()).chain(m.hermes_id.clone()).chain(m.additional.as_ref().map(|(_, id)| id.clone()))).collect();
        self.models = m;
        self.models_changed = true;
    }

    // ---------- events from the workers ----------

    /// Applies a worker's event to its chat. One behind the screen stands in as the current chat while it's handled.
    fn handle_for(&mut self, key: u64, ev: Ev) {
        if key == self.chat.key {
            return self.handle(ev);
        }
        let Some(i) = self.background.iter().position(|c| c.key == key) else { return };
        std::mem::swap(&mut self.chat, &mut self.background[i]);
        self.behind = true;
        self.handle(ev);
        self.behind = false;
        std::mem::swap(&mut self.chat, &mut self.background[i]);
    }

    pub fn handle(&mut self, ev: Ev) {
        match ev {
            Ev::ForChat(key, ev) => self.handle_for(key, *ev),
            Ev::RequestStart => {
                self.chat.streaming = None;
                self.chat.partial.clear();
                self.chat.status.clear();
            }
            Ev::Text(t) => {
                self.chat.partial.push_str(&t);
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
                self.chat.streaming = None;
                self.chat.partial.clear();
            }
            Ev::ToolCall { id, name, label } => {
                self.chat.streaming = None;
                self.chat.tool_index.insert(id, self.chat.items.len());
                self.push(ItemKind::Tool { name, label, progress: String::new(), result: None });
            }
            Ev::ToolProgress { id, text } => {
                if let Some(&i) = self.chat.tool_index.get(&id) {
                    if let Some(ItemKind::Tool { progress, .. }) = self.chat.items.get_mut(i).map(|i| &mut i.kind) {
                        *progress = text;
                        self.chat.changed.insert(i);
                    }
                }
            }
            Ev::ToolResult { id, result } => {
                if let Some(&i) = self.chat.tool_index.get(&id) {
                    if let Some(ItemKind::Tool { name, result: r, .. }) = self.chat.items.get_mut(i).map(|i| &mut i.kind) {
                        if matches!(name.as_str(), "write_file" | "edit_file" | "multi_edit" | "apply_patch" | "bash" | "update_memory") {
                            self.files_epoch += 1;
                        }
                        *r = Some(result);
                        self.chat.changed.insert(i);
                    }
                }
            }
            Ev::Status(s) => self.chat.status = s,
            Ev::Notice(s) => self.push(ItemKind::Notice(s)),
            Ev::Todos(t) => self.chat.todos = t,
            Ev::Totals(t, c) => {
                self.chat.totals = t;
                self.chat.context = c;
            }
            Ev::AskPermission(req, reply) => {
                let verb = match req.tool.as_str() {
                    "bash" => "run a command",
                    "devin" => "use a Devin tool",
                    "kiro" => "use a Kiro tool",
                    "codex" => "use a Codex tool",
                    "claude" => "use a Claude Code tool",
                    "opencode" => "use an OpenCode tool",
                    "grok" => "use a Grok Build tool",
                    t if tools::is_mcp(t) => "use an MCP tool",
                    _ => "change a file",
                };
                self.alert(&format!("Scout wants to {verb}"), req.label.lines().next().unwrap_or(""));
                self.chat.permission = Some(PendingPermission { req, reply });
            }
            Ev::AskPlan(text, reply) => {
                self.alert("Scout's plan is ready", &self.chat_title());
                self.chat.plan = Some(PendingPlan { text, reply });
            }
            Ev::AskQuestions(questions, reply) => {
                self.alert("Scout has a question", &self.chat_title());
                self.chat.questions = Some(PendingQuestions { questions, reply });
            }
            Ev::Steered(msgs) => {
                self.chat.streaming = None;
                for (text, files) in msgs {
                    self.push(ItemKind::User { text, files });
                }
            }
            Ev::Finished(mut agent, result) => {
                let stopped = self.chat.stopping || matches!(&result, Err(e) if e == "Interrupted");
                let error = result.as_ref().err().filter(|_| !stopped).cloned();
                let out_of_credit = std::mem::take(&mut agent.out_of_credit) && !stopped;
                // A sign-in Nous ended during the turn is gone from the file, and Settings should offer it again.
                self.cfg.hermes_signed_in = crate::hermes::signed_in();
                // Another model can't help with a turn that failed before asking one.
                let model_failed = !std::mem::take(&mut agent.before_model);
                let mut fall_back = None;
                if let Err(e) = result {
                    let failed = ModelRef { key: self.current_key(), provider: self.chat.provider.clone() };
                    let next = if stopped || !model_failed { None } else { self.next_fallback(&failed) };
                    if stopped {
                        agent.keep_partial(&self.chat.partial);
                        self.push(ItemKind::Notice("Interrupted.".into()));
                    } else if let Some(next) = next {
                        let short: String = e.chars().take(200).collect();
                        self.push(ItemKind::Notice(format!("{} failed: {short} Trying {}.", self.model_label(&failed), self.model_label(&next))));
                        fall_back = Some(next);
                    } else if self.chat.fallback.is_some() {
                        self.push(ItemKind::Error(format!("{e}\n\nThe fallback models failed too.")));
                    } else if out_of_credit {
                        self.push(ItemKind::OutOfCredit(e));
                    } else if model_failed && !self.cfg.fallbacks.chosen {
                        self.push(ItemKind::FallbackSetup(e, failed));
                    } else {
                        self.push(ItemKind::Error(e));
                    }
                }
                // Messages sent to steer that the turn ended before taking go first in the queue.
                let left: Vec<Steer> = std::mem::take(&mut *self.chat.shared.steer.lock().unwrap());
                self.chat.queue.splice(0..0, left.into_iter().map(|s| Outgoing { text: s.text, files: s.files }));
                self.chat.totals = agent.session.totals.clone();
                self.chat.context = agent.context_tokens();
                self.chat.agent = Some(*agent);
                self.chat.running = false;
                self.chat.stopping = false;
                self.chat.streaming = None;
                self.chat.partial.clear();
                self.chat.status.clear();
                self.chat.permission = None;
                self.chat.plan = None;
                self.chat.questions = None;
                self.refresh_sessions();
                // The keyboard stays where the user is typing.
                if self.behind {
                    self.chat.unread = true;
                } else {
                    self.focus_epoch += 1;
                }
                self.files_epoch += 1;
                if let Some(next) = fall_back {
                    self.retry_on(next);
                    return;
                }
                // The turn is over, so the model the user chose is back for the next one.
                if let Some(run) = self.chat.fallback.take() {
                    self.switch_model(&run.from.key, &run.from.provider, false);
                }
                // The queue carries on unless the user stopped Scout or it ran out of credit: they then decide what's next.
                if !stopped && !out_of_credit && !self.chat.queue.is_empty() {
                    let next = self.chat.queue.remove(0);
                    self.submit(next);
                } else if let Some(e) = error {
                    self.alert("Scout stopped with an error", &e);
                } else if !stopped {
                    self.alert("Scout is done", &self.chat_title());
                }
            }
            // The chosen model's list, ahead of the rest. A newer fetch under way has its own.
            Ev::Models(_, false) if self.models_loading > 1 => {}
            Ev::Models(Ok(m), done) => {
                self.set_models(m);
                if done {
                    self.models_loading -= 1;
                }
                self.start_on_additional();
                self.start_on_hermes();
                self.start_on_guest();
            }
            Ev::Models(Err(e), _) => {
                self.chat.status = format!("Could not load models: {e}");
                self.models_loading -= 1;
            }
            Ev::KiloCheck(r) => self.login.kilo_check = Some(r),
            Ev::WhatsNew(notes) => self.whats_new = notes,
            Ev::AdditionalErrors(e) => self.additional_errors = e,
            Ev::Copilot(u) => self.copilot_update(u),
            Ev::Devin(u) => self.devin_update(u),
            Ev::Kiro(u) => self.kiro_update(u),
            Ev::KiroModelsError(e) => self.kiro_models_error = e,
            Ev::AgentSignIn(which, u) => self.agent_update(which, u),
            Ev::AgentModelsError("codex", e) => self.codex_models_error = e,
            Ev::AgentModelsError(_, e) => self.claude_models_error = e,
            Ev::Opencode(u) => self.opencode_update(u),
            Ev::OpencodeModelsError(e) => self.opencode_models_error = e,
            Ev::Grok(u) => self.grok_update(u),
            Ev::GrokModelsError(e) => self.grok_models_error = e,
            Ev::Hermes(u) => self.hermes_update(u),
            Ev::HermesModelsError(e) => self.hermes_models_error = e,
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
            // A refresh in the background only ever updates the numbers.
            Ev::UsagePolled(r) => {
                self.usage.busy = false;
                if let Ok(info) = r {
                    self.usage.info = Some(info);
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
            Ev::Link(crate::mcp::LinkEvent::Status(s, online)) => {
                self.link_status = s;
                self.link_online = online;
            }
            Ev::Link(crate::mcp::LinkEvent::Running(c)) => self.link_running = Some(c),
            Ev::Link(crate::mcp::LinkEvent::Call(c)) => {
                self.link_running = None;
                if !c.edits.is_empty() {
                    self.files_epoch += 1;
                }
                for e in &c.edits {
                    match self.link_edits.iter_mut().find(|x| x.path == e.path) {
                        Some(x) => {
                            x.added += e.added;
                            x.removed += e.removed;
                        }
                        None => self.link_edits.push(e.clone()),
                    }
                }
                self.link_log.push(c);
                if self.link_log.len() > 20 {
                    self.link_log.remove(0);
                }
            }
            Ev::Link(crate::mcp::LinkEvent::Chatbots(b)) => self.link_chatbots = b,
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
                    Ok(text) if text.is_empty() => self.chat.status = "Didn't catch anything. Try again a little closer to the mic.".into(),
                    Ok(text) => {
                        self.chat.status.clear();
                        let _ = self.handle.emit("dictated", text);
                        self.focus_epoch += 1;
                    }
                    Err(e) => self.chat.status = e,
                }
            }
        }
    }

    /// The mic button: starts a recording, or stops it and transcribes what was said into the box.
    pub fn toggle_voice(&mut self) {
        match &self.voice {
            Voice::Idle => {
                self.chat.status.clear();
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
        if self.chat.streaming.is_none() {
            self.chat.streaming = Some(self.chat.items.len());
            self.push(ItemKind::Assistant { text: String::new(), reasoning: String::new() });
        }
        let i = self.chat.streaming.unwrap();
        self.chat.changed.insert(i);
        &mut self.chat.items[i].kind
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
        if self.cfg.only_kilo() && self.chat.provider == "scoutcode" {
            self.choose_model(config::KILO_DEFAULT_MODEL, "kilo");
        }
        self.keys_changed();
    }

    fn keys_changed(&mut self) {
        if let Some(a) = &mut self.chat.agent {
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
        // Back to the first-run choice, not on to using it without an account.
        config::save_one("desktopGuest", Value::Null);
        self.cfg.guest = false;
        self.set_key(None);
    }

    /// Starts or stops using ScoutCode's free models without an account. Starting moves to one of them once they're listed.
    pub fn set_guest(&mut self, on: bool) {
        config::save_one("desktopGuest", json!(on.then_some(true)));
        self.cfg.guest = on;
        self.start_guest = on;
        if self.cfg.only_kilo() && self.chat.provider == "scoutcode" {
            self.choose_model(config::KILO_DEFAULT_MODEL, "kilo");
        }
        self.keys_changed();
    }

    /// Chooses the first free model on offer after `set_guest`. The server's list has nothing else, and leaves out "auto".
    fn start_on_guest(&mut self) {
        if !self.start_guest {
            return;
        }
        let Some(key) = self.models.iter().find(|m| m.hosted && m.available != Some(false)).map(|m| m.id.clone()) else { return };
        self.start_guest = false;
        self.choose_model(&key, "scoutcode");
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
        if removed && self.chat.provider == "openrouter" {
            self.choose_model("auto", "scoutcode");
        }
        self.keys_changed();
    }

    /// Saves or forgets the NVIDIA key. Without one, a model that ran on NIM moves back to ScoutCode.
    pub fn save_nvidia_key(&mut self, key: Option<String>) {
        config::save_one("nvidiaKey", json!(key));
        self.cfg.nvidia_key = key;
        if self.cfg.nvidia_key.is_none() && self.chat.provider == "nvidia" {
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
        if !self.cfg.kilo_on() && self.chat.provider == "kilo" {
            self.choose_model("auto", "scoutcode");
        } else if self.cfg.only_kilo() && self.chat.provider == "scoutcode" {
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
        if config::additional_provider(&self.chat.provider).is_some() && self.cfg.linked(&self.chat.provider).is_none() {
            self.choose_model("auto", "scoutcode");
        }
        if self.cfg.only_kilo() && self.chat.provider == "scoutcode" {
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
        if self.chat.provider == "scoutcode" && self.cfg.api_key.is_none() {
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
        if self.cfg.copilot_token.is_none() && self.chat.provider == "copilot" {
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
        if !on && self.chat.provider == "devin" {
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
        if !on && self.chat.provider == "kiro" {
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

    /// Where signing in to Codex ("codex") or Claude Code ("claude") stands.
    fn agent_login(&mut self, which: &str) -> (&mut Option<SignIn>, &mut Option<String>) {
        if which == "codex" { (&mut self.login.codex, &mut self.login.codex_error) } else { (&mut self.login.claude, &mut self.login.claude_error) }
    }

    /// Starts signing in to Codex or Claude Code: Node and the agent download, then its login opens the browser.
    pub fn agent_sign_in(&mut self, which: &str) {
        let which = if which == "codex" { "codex" } else { "claude" };
        let sign_in = SignIn::default();
        let cancel = sign_in.cancel.clone();
        let (slot, error) = self.agent_login(which);
        *slot = Some(sign_in);
        *error = None;
        let h = self.hooks.clone();
        let send = move |u| h.send(Ev::AgentSignIn(which, u));
        if which == "codex" { crate::codex::sign_in(cancel, send) } else { crate::claude::sign_in(cancel, send) }
    }

    fn agent_update(&mut self, which: &'static str, update: crate::node::Update) {
        use crate::node::Update;
        match update {
            Update::Done => {
                *self.agent_login(which).0 = None;
                self.save_agent(which, true);
                self.choose_model("auto", which);
            }
            Update::Failed(e) => {
                // Cancelling fails with no message.
                let (slot, error) = self.agent_login(which);
                *slot = None;
                *error = (!e.is_empty()).then_some(e);
            }
            Update::Progress { done, total } => {
                if let Some(s) = self.agent_login(which).0 {
                    s.progress = (done, total);
                }
            }
            Update::Waiting(page) => {
                if let Some(s) = self.agent_login(which).0 {
                    s.waiting = true;
                    s.page = page.or(s.page.take());
                }
            }
        }
    }

    /// Turns Codex or Claude Code on or off. Without it, a model that ran there moves back to ScoutCode.
    fn save_agent(&mut self, which: &str, on: bool) {
        if which == "codex" {
            config::save_one("codexEnabled", json!(on));
            self.cfg.codex_enabled = on;
        } else {
            config::save_one("claudeEnabled", json!(on));
            self.cfg.claude_enabled = on;
        }
        if !on && self.chat.provider == which {
            self.choose_model("auto", "scoutcode");
        }
        self.keys_changed();
    }

    pub fn agent_sign_out(&mut self, which: &str, uninstall: bool) {
        if which == "codex" {
            crate::codex::sign_out();
            if uninstall {
                crate::codex::remove_runtime();
            }
        } else {
            crate::claude::sign_out();
            if uninstall {
                crate::claude::remove_runtime();
            }
        }
        self.save_agent(which, false);
    }

    /// Turns OpenCode on, which downloads its CLI for the free models, or off, which signs out of it too. Without it, a
    /// model that ran on OpenCode moves back to ScoutCode.
    pub fn set_opencode(&mut self, on: bool, uninstall: bool) {
        if on {
            self.login.opencode = Some(SignIn::default());
            self.login.opencode_error = None;
            let h = self.hooks.clone();
            crate::opencode::set_up(move |u| h.send(Ev::Opencode(u)));
            return;
        }
        self.opencode_sign_out();
        self.save_opencode(false);
        if uninstall {
            crate::opencode::remove_runtime();
        }
    }

    /// Starts signing in to an OpenCode account: the CLI downloads, then gives a code to enter in the browser.
    pub fn opencode_sign_in(&mut self) {
        let sign_in = SignIn::default();
        let cancel = sign_in.cancel.clone();
        self.login.opencode = Some(sign_in);
        self.login.opencode_error = None;
        let h = self.hooks.clone();
        crate::opencode::sign_in(cancel, move |u| h.send(Ev::Opencode(u)));
    }

    fn opencode_update(&mut self, update: crate::opencode::Update) {
        use crate::opencode::Update;
        match update {
            Update::Ready | Update::SignedIn => {
                let signed_in = matches!(update, Update::SignedIn);
                self.login.opencode = None;
                if signed_in {
                    config::save_one("opencodeSignedIn", json!(true));
                    self.cfg.opencode_signed_in = true;
                }
                self.save_opencode(true);
                if self.chat.provider != "opencode" {
                    self.choose_model("auto", "opencode");
                }
            }
            Update::Failed(e) => {
                // Cancelling fails with no message.
                self.login.opencode = None;
                self.login.opencode_error = (!e.is_empty()).then_some(e);
            }
            Update::Progress { done, total } => {
                if let Some(s) = &mut self.login.opencode {
                    s.progress = (done, total);
                }
            }
            Update::Code { code, url } => {
                if let Some(s) = &mut self.login.opencode {
                    s.code = Some((code, url));
                }
            }
        }
    }

    fn save_opencode(&mut self, on: bool) {
        config::save_one("opencodeEnabled", json!(on));
        self.cfg.opencode_enabled = on;
        if !on && self.chat.provider == "opencode" {
            self.choose_model("auto", "scoutcode");
        }
        self.keys_changed();
    }

    /// Signs out of the OpenCode account. Its free models stay.
    pub fn opencode_sign_out(&mut self) {
        crate::opencode::sign_out();
        config::save_one("opencodeSignedIn", json!(false));
        self.cfg.opencode_signed_in = false;
        self.keys_changed();
    }

    /// Starts signing in to Grok: the CLI downloads, then gives a code to confirm in the browser.
    pub fn grok_sign_in(&mut self) {
        let sign_in = SignIn::default();
        let cancel = sign_in.cancel.clone();
        self.login.grok = Some(sign_in);
        self.login.grok_error = None;
        let h = self.hooks.clone();
        crate::grok::sign_in(cancel, move |u| h.send(Ev::Grok(u)));
    }

    /// Saves or forgets the xAI key. A new one downloads the CLI, which is what uses it.
    pub fn save_grok_key(&mut self, key: Option<String>) {
        config::save_one("grokKey", json!(key));
        let adding = key.is_some();
        self.cfg.grok_key = key;
        if adding {
            self.login.grok = Some(SignIn::default());
            self.login.grok_error = None;
            let h = self.hooks.clone();
            crate::grok::set_up(move |u| h.send(Ev::Grok(u)));
        } else {
            self.grok_changed();
        }
    }

    fn grok_update(&mut self, update: crate::grok::Update) {
        use crate::grok::Update;
        match update {
            Update::Ready | Update::SignedIn => {
                let signed_in = matches!(update, Update::SignedIn);
                self.login.grok = None;
                if signed_in {
                    config::save_one("grokSignedIn", json!(true));
                    self.cfg.grok_signed_in = true;
                }
                if self.chat.provider != "grok" {
                    self.choose_model("auto", "grok");
                }
                self.grok_changed();
            }
            Update::Failed(e) => {
                // Cancelling fails with no message.
                self.login.grok = None;
                self.login.grok_error = (!e.is_empty()).then_some(e);
            }
            Update::Progress { done, total } => {
                if let Some(s) = &mut self.login.grok {
                    s.progress = (done, total);
                }
            }
            Update::Code { code, url } => {
                if let Some(s) = &mut self.login.grok {
                    s.code = Some((code, url));
                }
            }
        }
    }

    /// With no sign-in or key left, a model that ran on Grok Build moves back to ScoutCode.
    fn grok_changed(&mut self) {
        if !self.cfg.grok_on() && self.chat.provider == "grok" {
            self.choose_model("auto", "scoutcode");
        }
        self.keys_changed();
    }

    /// Signs out of Grok. Uninstalling forgets the key too, since nothing is left to use it.
    pub fn grok_sign_out(&mut self, uninstall: bool) {
        crate::grok::sign_out();
        config::save_one("grokSignedIn", json!(false));
        self.cfg.grok_signed_in = false;
        if uninstall {
            config::save_one("grokKey", Value::Null);
            self.cfg.grok_key = None;
            crate::grok::remove_runtime();
        }
        self.grok_changed();
    }

    /// Starts signing in to Nous Portal, which gives a code to approve in the browser.
    pub fn hermes_sign_in(&mut self) {
        let sign_in = SignIn::default();
        let cancel = sign_in.cancel.clone();
        self.login.hermes = Some(sign_in);
        self.login.hermes_error = None;
        let h = self.hooks.clone();
        crate::hermes::sign_in(cancel, move |u| h.send(Ev::Hermes(u)));
    }

    /// Saves or forgets the Nous Portal key. A new one moves to Hermes' first free model.
    pub fn save_hermes_key(&mut self, key: Option<String>) {
        config::save_one("hermesKey", json!(key));
        self.start_hermes = key.is_some() && self.chat.provider != "hermes";
        self.cfg.hermes_key = key;
        self.hermes_changed();
    }

    fn hermes_update(&mut self, update: crate::hermes::Update) {
        use crate::hermes::Update;
        match update {
            Update::SignedIn => {
                self.login.hermes = None;
                self.cfg.hermes_signed_in = true;
                self.start_hermes = self.chat.provider != "hermes";
                self.hermes_changed();
            }
            Update::Failed(e) => {
                // Cancelling fails with no message.
                self.login.hermes = None;
                self.login.hermes_error = (!e.is_empty()).then_some(e);
            }
            Update::Code { code, url } => {
                if let Some(s) = &mut self.login.hermes {
                    s.code = Some((code, url));
                }
            }
        }
    }

    /// With no sign-in or key left, a model that ran on Hermes moves back to ScoutCode.
    fn hermes_changed(&mut self) {
        if !self.cfg.hermes_on() && self.chat.provider == "hermes" {
            self.choose_model("auto", "scoutcode");
        }
        self.keys_changed();
    }

    /// Signs out of Nous Portal. A saved key stays.
    pub fn hermes_sign_out(&mut self) {
        crate::hermes::sign_out();
        self.cfg.hermes_signed_in = false;
        self.hermes_changed();
    }

    /// Chooses Hermes' first model, which is a free one while there are any, once it's in the list.
    fn start_on_hermes(&mut self) {
        if !self.start_hermes {
            return;
        }
        let Some(key) = self.models.iter().find(|m| m.hermes_id.is_some()).map(|m| m.id.clone()) else { return };
        self.start_hermes = false;
        self.choose_model(&key, "hermes");
    }

    /// Stops a sign-in that's under way: `which` is "copilot", "devin", "kiro", "codex", "claude", "opencode", "grok" or "hermes".
    pub fn cancel_sign_in(&mut self, which: &str) {
        let slot = match which {
            "copilot" => &mut self.login.copilot,
            "devin" => &mut self.login.devin,
            "codex" => &mut self.login.codex,
            "claude" => &mut self.login.claude,
            "opencode" => &mut self.login.opencode,
            "grok" => &mut self.login.grok,
            "hermes" => &mut self.login.hermes,
            _ => &mut self.login.kiro,
        };
        if let Some(s) = slot.take() {
            s.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Makes `key` (a model's id in the list) the one sent to `provider`, for the next message and the next launch.
    pub fn choose_model(&mut self, key: &str, provider: &str) {
        // The user's pick is the model to come back to, not the one a failure moved the turn off.
        self.chat.fallback = None;
        self.switch_model(key, provider, true);
    }

    /// Moves the chat to a model; `save` makes it the one new chats and the next launch start on.
    fn switch_model(&mut self, key: &str, provider: &str, save: bool) {
        if provider == "kilo" && !self.cfg.kilo_enabled {
            self.cfg.kilo_enabled = true;
            config::save_one("kiloEnabled", json!(true));
            if let Some(a) = &mut self.chat.agent {
                a.cfg.take_keys(&self.cfg);
            }
        }
        let model = self.models.iter().find(|m| m.id == key).map(|m| m.request_id(provider).to_string()).unwrap_or_else(|| key.to_string());
        self.chat.model = model.clone();
        self.chat.provider = provider.to_string();
        *self.chat.shared.model.lock().unwrap() = model.clone();
        *self.chat.shared.provider.lock().unwrap() = provider.to_string();
        if save {
            config::save_one("model", json!(model));
            config::save_one("provider", json!(provider));
        }
    }

    fn model_label(&self, m: &ModelRef) -> String {
        self.models.iter().find(|x| x.id == m.key).map(|x| x.label()).unwrap_or_else(|| crate::api::short_id(&m.key))
    }

    /// The next model to hand a failed turn to, starting a run of `failed`'s fallbacks if this is its first failure.
    fn next_fallback(&mut self, failed: &ModelRef) -> Option<ModelRef> {
        if self.chat.fallback.is_none() {
            let left = self.cfg.fallbacks.after(failed);
            if left.is_empty() {
                return None;
            }
            self.chat.fallback = Some(FallbackRun { from: failed.clone(), left });
        }
        let run = self.chat.fallback.as_mut()?;
        (!run.left.is_empty()).then(|| run.left.remove(0))
    }

    /// Whether a failure should end the turn at once rather than be retried: there's a fallback to move on to, or the
    /// user hasn't been asked about fallbacks yet. The last fallback, and a model left to show its errors, still retry.
    fn fails_fast(&self) -> bool {
        match &self.chat.fallback {
            Some(run) => !run.left.is_empty(),
            None => !self.cfg.fallbacks.chosen || !self.cfg.fallbacks.after(&ModelRef { key: self.current_key(), provider: self.chat.provider.clone() }).is_empty(),
        }
    }

    /// Carries on the failed turn on `next`, for this turn only.
    fn retry_on(&mut self, next: ModelRef) {
        self.switch_model(&next.key, &next.provider, false);
        self.start(|agent, h| agent.retry(h).map_err(|e| agent.failed(e)));
    }

    /// The answer to the card the first failure shows: the list for every model and `failed`'s own, which wins over
    /// it (empty to use that one), or `off` to just show errors. With a fallback to try, the failed turn carries on.
    pub fn setup_fallbacks(&mut self, failed: ModelRef, all: Vec<ModelRef>, own: Vec<ModelRef>, off: bool) {
        let f = &mut self.cfg.fallbacks;
        f.chosen = true;
        if !off {
            f.all = all;
            f.models.retain(|(m, _)| *m != failed);
            if !own.is_empty() {
                f.models.push((failed.clone(), own));
            }
        }
        config::save_one("modelFallbacks", json!(f));
        if off || self.chat.running || !matches!(self.chat.items.last().map(|i| &i.kind), Some(ItemKind::FallbackSetup(..))) {
            return;
        }
        self.chat.fallback = None;
        if let Some(next) = self.next_fallback(&failed) {
            self.push(ItemKind::Notice(format!("Trying {}.", self.model_label(&next))));
            self.retry_on(next);
        }
    }

    /// Settings' edits to the fallbacks.
    pub fn set_fallbacks(&mut self, f: config::Fallbacks) {
        config::save_one("modelFallbacks", json!(f));
        self.cfg.fallbacks = f;
    }

    /// The list key of the model being used: its entry's id, or the id itself when the list doesn't have it (auto).
    fn current_key(&self) -> String {
        self.models.iter().find(|m| m.request_id(&self.chat.provider) == self.chat.model).map(|m| m.id.clone()).unwrap_or_else(|| self.chat.model.clone())
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
            self.chat.effort = effort.to_string();
            *self.chat.shared.effort.lock().unwrap() = self.chat.effort.clone();
            config::save_one("effort", json!(self.chat.effort));
        }
    }

    pub fn set_mode(&mut self, mode: &str) {
        if MODES.contains(&mode) {
            *self.chat.shared.mode.lock().unwrap() = mode.to_string();
        }
    }

    pub fn cycle_mode(&mut self) {
        if !self.chat.running {
            let m = self.chat.shared.mode();
            let next = MODES[(MODES.iter().position(|x| *x == m).unwrap_or(0) + 1) % MODES.len()];
            *self.chat.shared.mode.lock().unwrap() = next.into();
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
        if self.chat.running && text.starts_with('/') {
            self.chat.status = "Commands wait until Scout is done. Esc stops it.".into();
            return false;
        }
        if self.chat.running {
            let o = Outgoing { text, files };
            if steer { self.steer(o) } else { self.chat.queue.push(o) }
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
                "plan" if arg.trim().is_empty() => {
                    self.chat.status = "Say what to plan after /plan.".into();
                    return false;
                }
                _ => {}
            }
        }
        self.submit(Outgoing { text, files });
        true
    }

    /// Starts a turn with the message.
    fn submit(&mut self, o: Outgoing) {
        if self.chat.agent.is_none() {
            return;
        }
        let (cwd, files) = (self.chat.cwd.clone(), o.files.clone());
        let text = crate::prompt::command_prompt(&o.text);
        self.push(ItemKind::User { text: o.text, files: o.files });
        // Off the window's thread: reading attachments takes a while when images have to be shrunk.
        self.start(move |agent, h| {
            let (content, read) = attach::build(&cwd, &text, &files);
            agent.read_files.extend(read);
            agent.turn(content, h).map_err(|e| agent.failed(e))
        });
    }

    /// Carries on a turn that ran out of credit, on `model` (its list key and provider) when one is picked.
    pub fn retry(&mut self, model: Option<(String, String)>) {
        if self.chat.running || !matches!(self.chat.items.last().map(|i| &i.kind), Some(ItemKind::OutOfCredit(_))) {
            return;
        }
        if let Some((key, provider)) = model {
            self.choose_model(&key, &provider);
        }
        self.start(|agent, h| agent.retry(h).map_err(|e| agent.failed(e)));
    }

    /// Hands a message to the running turn, which adds it after the step it's on.
    fn steer(&mut self, o: Outgoing) {
        let (content, read) = attach::build(&self.chat.cwd, &o.text, &o.files);
        self.chat.shared.steer.lock().unwrap().push(Steer { text: o.text, files: o.files, content, read });
    }

    /// Takes a message out of the queue and hands it back.
    pub fn unqueue(&mut self, i: usize) -> Option<Outgoing> {
        (i < self.chat.queue.len()).then(|| self.chat.queue.remove(i))
    }

    /// Sends a queued message now: into the running turn, or as the next one.
    pub fn send_queued(&mut self, i: usize) {
        if i < self.chat.queue.len() {
            let o = self.chat.queue.remove(i);
            if self.chat.running { self.steer(o) } else { self.submit(o) }
        }
    }

    /// Flashes the taskbar and/or shows a Windows notification, as the user chose, when the window isn't in front.
    fn alert(&self, title: &str, body: &str) {
        let Some(window) = self.handle.get_webview_window("main") else { return };
        // A chat behind the one on screen can't be seen even with the window in front.
        if !self.behind && window.is_focused().unwrap_or(true) {
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
        let current = self.chat.session_id == id;
        // The chat Scout is working in stays until it's done: it would only be saved again.
        if std::iter::once(&self.chat).chain(&self.background).any(|c| c.session_id == id && c.running) {
            return;
        }
        if let Err(e) = session::delete_session(id) {
            self.chat.status = format!("Could not delete the chat: {e}");
            return;
        }
        self.background.retain(|c| c.session_id != id);
        self.refresh_sessions();
        if current {
            self.new_chat();
        }
    }

    /// Rewinds the chat to before item `n`, a message of the user's, restoring the files changed since, and sends
    /// `text` in its place with the same attachments.
    pub fn resend(&mut self, n: usize, text: String) {
        if self.chat.running {
            return;
        }
        let Some(ItemKind::User { text: old, files }) = self.chat.items.get(n).map(|i| &i.kind) else { return };
        let (old, files) = (old.clone(), files.clone());
        // The saved message with the same text, counted from the end, since later ones may repeat it.
        let later = self.chat.items[n + 1..].iter().filter(|i| matches!(&i.kind, ItemKind::User { text, .. } if *text == old)).count();
        let Some(agent) = &mut self.chat.agent else { return };
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
        self.chat.context = agent.context_tokens();
        let items = rebuild(&s, &mut self.next_item);
        self.replace_items(items);
        self.chat.tool_index.clear();
        self.chat.todos.clear();
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
        let Some(mut agent) = self.chat.agent.take() else { return };
        agent.cfg.take_keys(&self.cfg);
        agent.fail_fast = self.fails_fast();
        self.chat.running = true;
        self.chat.stopping = false;
        self.chat.shared.cancel.store(false, Ordering::Relaxed);
        self.refresh_sessions();
        let mut h = self.hooks.clone();
        h.chat = Some(self.chat.key);
        std::thread::Builder::new()
            .name("scout-agent".into())
            .spawn(move || {
                let r = work(&mut agent, &h);
                h.send(Ev::Finished(Box::new(agent), r));
            })
            .expect("spawn agent thread");
    }

    pub fn stop(&mut self) {
        if !self.chat.running {
            return;
        }
        self.chat.stopping = true;
        self.chat.shared.cancel.store(true, Ordering::Relaxed);
        // Answering unblocks a worker waiting on the user.
        if let Some(p) = self.chat.permission.take() {
            let _ = p.reply.send(PermissionAnswer::Deny(String::new()));
        }
        if let Some(p) = self.chat.plan.take() {
            let _ = p.reply.send(PlanAnswer::Revise(String::new()));
        }
        if let Some(p) = self.chat.questions.take() {
            let _ = p.reply.send(QuestionAnswer::Skipped);
        }
        self.chat.status = "Stopping…".into();
    }

    pub fn new_chat(&mut self) {
        let s = session::new_session(&self.chat.cwd, &self.chat.model);
        self.open_session(s);
    }

    pub fn open_chat(&mut self, id: &str) {
        if self.chat.session_id == id {
            self.focus_epoch += 1;
        } else if let Some(i) = self.background.iter().position(|c| c.session_id == id) {
            let chat = self.background.remove(i);
            self.show(chat);
        } else if let Some(s) = session::load_session(id) {
            self.open_session(s);
        }
    }

    fn open_session(&mut self, s: Session) {
        let items = rebuild(&s, &mut self.next_item);
        self.next_chat += 1;
        let mut chat = Chat::new(self.next_chat, s, &self.cfg, self.chat.shared.for_new_chat());
        chat.items = items;
        self.show(chat);
    }

    /// Puts the chat on screen. The one there carries on behind it if Scout is working in it; if not, it's read back
    /// from disk when it's next opened.
    fn show(&mut self, mut chat: Chat) {
        chat.reset = true;
        chat.changed.clear();
        chat.unread = false;
        std::mem::swap(&mut self.chat, &mut chat);
        if chat.running {
            self.background.push(chat);
        }
        self.refresh_sessions();
        self.chat_epoch += 1;
        let cwd = self.chat.cwd.clone();
        self.enter_project(&cwd);
        self.update_discord();
        self.files_epoch += 1;
        self.focus_epoch += 1;
    }

    /// Reads the sidebar's chats. A chat that's just started may not be on disk yet, but it's there to go back to.
    fn refresh_sessions(&mut self) {
        self.sessions = session::list_sessions();
        for c in std::iter::once(&self.chat).chain(&self.background).filter(|c| c.running) {
            if !self.sessions.iter().any(|s| s.id == c.session_id) {
                self.sessions.insert(0, SessionInfo { id: c.session_id.clone(), cwd: c.cwd.clone(), title: c.title(), updated: session::now_ms() });
            }
        }
    }

    /// Starts a new chat in the folder.
    pub fn change_project(&mut self, dir: String) {
        let s = session::new_session(&dir, &self.chat.model);
        self.open_session(s);
    }

    // ---------- projects: named folders that chats belong to (by the folder they ran in) ----------

    fn project_index(&self, dir: &str) -> Option<usize> {
        self.projects.iter().position(|p| session::same_dir(&p.path, dir))
    }

    /// The current folder's project name.
    fn project_label(&self) -> String {
        self.project_index(&self.chat.cwd).map(|i| self.projects[i].name.clone()).unwrap_or_else(|| project_name(&self.chat.cwd))
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
        if session::same_dir(path, &self.chat.cwd) {
            return;
        }
        if let Some(i) = self.project_index(path) {
            self.projects.remove(i);
            self.save_projects();
        }
    }

    pub fn undo(&mut self) {
        let Some(agent) = &mut self.chat.agent else { return };
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
        let Some(agent) = &mut self.chat.agent else { return };
        let msg = match agent.prune(0, 0) {
            Some((n, freed)) => format!("Cleared {n} old tool output{} (~{}k tokens).", if n == 1 { "" } else { "s" }, freed / 1000),
            None => "No old tool output to clear.".into(),
        };
        self.chat.context = agent.context_tokens();
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
        if let Some(p) = self.chat.permission.take() {
            let _ = p.reply.send(a);
        }
    }

    pub fn answer_plan(&mut self, a: PlanAnswer) {
        if let Some(p) = self.chat.plan.take() {
            let _ = p.reply.send(a);
        }
    }

    pub fn answer_questions(&mut self, a: QuestionAnswer) {
        if let Some(p) = self.chat.questions.take() {
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
        self.chat.title()
    }

    fn any_running(&self) -> bool {
        self.chat.running || self.background.iter().any(|c| c.running)
    }

    /// Starts the new build on the same folder and chat. False when it can't: the window checks for unsaved edits first.
    pub fn restart_to_update(&mut self) -> bool {
        if self.any_running() {
            return false;
        }
        let args = vec![self.chat.cwd.clone(), "--resume".to_string(), self.chat.session_id.clone()];
        match self.updater.restart(&args) {
            Ok(()) => true,
            Err(e) => {
                self.chat.status = e;
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

    /// After an update, looks up what changed since the version that ran here before. A first install shows nothing.
    fn check_updated(&mut self) {
        if !crate::update::self_updating() {
            return;
        }
        let last = self.cfg.last_version.replace(crate::update::VERSION.to_string());
        if last.as_deref() == Some(crate::update::VERSION) {
            return;
        }
        config::save_one("desktopLastVersion", json!(crate::update::VERSION));
        let Some(last) = last.filter(|l| crate::update::is_newer(crate::update::VERSION, l)) else { return };
        let h = self.hooks.clone();
        std::thread::spawn(move || {
            if let Ok(notes) = crate::update::notes_since(&last) {
                if !notes.is_empty() {
                    h.send(Ev::WhatsNew(notes));
                }
            }
        });
    }

    pub fn dismiss_whats_new(&mut self) {
        self.whats_new.clear();
    }

    pub fn set_auto_update(&mut self, on: bool) {
        self.cfg.auto_update = on;
        config::save_one("desktopAutoUpdate", json!(on));
        self.updater.set_auto(on);
    }

    /// Turns the Discord profile status on or off, and whether it names the open folder.
    pub fn set_discord(&mut self, presence: bool, project: bool) {
        self.cfg.discord_presence = presence;
        self.cfg.discord_project = project;
        let mut m = serde_json::Map::new();
        m.insert("discordPresence".into(), json!(presence));
        m.insert("discordProject".into(), json!(project));
        config::save_config(m);
        self.update_discord();
    }

    /// Sends the current state to Discord: nothing when it's off, and the folder's name only when that's allowed.
    pub fn update_discord(&self) {
        let activity = self.cfg.discord_presence.then(|| crate::discord::Activity {
            project: self.cfg.discord_project.then(|| crate::discord::project_name(&self.chat.cwd)).flatten(),
        });
        crate::discord::update(activity);
    }

    /// Experimental image generation: whether the agent gets generate_image, and which server image model it uses.
    /// Applies from the next message, when the agent takes the settings.
    pub fn set_images(&mut self, on: bool, model: Option<String>) {
        self.cfg.experimental_images = on;
        self.cfg.image_model = model.filter(|m| !m.is_empty());
        let mut m = serde_json::Map::new();
        m.insert("experimentalImages".into(), json!(on));
        m.insert("imageModel".into(), json!(self.cfg.image_model));
        config::save_config(m);
    }

    /// Experimental games: where they show and when they open and close. Only the window reads these.
    pub fn set_games(&mut self, games: config::Games) {
        let mut m = serde_json::Map::new();
        m.insert("games".into(), json!(games));
        self.cfg.games = games;
        config::save_config(m);
    }

    /// The settings for ScoutCode's own server, for a request made outside the agent.
    pub fn scout_cfg(&self) -> Config {
        self.cfg.clone()
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

    /// Refreshes the account's balance and limits without a word, for the timer.
    pub fn usage_poll(&mut self) {
        if self.cfg.api_key.is_none() || self.usage.busy {
            return;
        }
        self.usage.busy = true;
        let cfg = self.cfg.clone();
        let h = self.hooks.clone();
        std::thread::spawn(move || h.send(Ev::UsagePolled(crate::api::usage_info(&cfg).map_err(|e| e.message))));
    }

    pub fn refresh_skills(&mut self) {
        self.skills = crate::skills::list();
    }

    fn stop_link(&mut self, status: &str) {
        if let Some(l) = self.link.take() {
            l.stop();
        }
        self.link_permission = None;
        self.link_online = false;
        self.link_running = None;
        self.link_status = status.to_string();
    }

    /// Turns the link for web chatbots on or off.
    pub fn toggle_link(&mut self, mode: &str) {
        if self.link.is_some() {
            self.stop_link("Off.");
            return;
        }
        if self.cfg.api_key.is_none() || self.chat.cwd.is_empty() {
            return;
        }
        let h = self.hooks.clone();
        self.link_log.clear();
        self.link_edits.clear();
        self.link_since = session::now_ms();
        self.link_status = "Connecting…".into();
        self.link = Some(crate::mcp::start_link(&self.cfg, &self.chat.cwd, mode, move |e| h.send(Ev::Link(e))));
    }

    pub fn set_status(&mut self, s: String) {
        self.chat.status = s;
    }

    // ---------- telling the window ----------

    /// Everything the window draws from, by key. Keys go only to the window, so no secrets: keys are masked here.
    fn view(&self) -> Vec<(&'static str, Value)> {
        let cfg = &self.cfg;
        let hub = crate::mcp_client::hub();
        let steering: Vec<String> = self.chat.shared.steer.lock().unwrap().iter().map(|s| s.text.clone()).collect();
        let login_needed = cfg.api_key.is_none() && !cfg.guest && cfg.openrouter_keys.is_empty() && cfg.nvidia_key.is_none() && !cfg.kilo_on() && cfg.copilot_token.is_none() && !cfg.devin_enabled && !cfg.kiro_enabled && !cfg.codex_enabled && !cfg.claude_enabled && !cfg.opencode_enabled && !cfg.grok_on() && !cfg.hermes_on() && cfg.additional.is_empty();
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
                "guest": cfg.is_guest(),
                "browserSignIn": config::is_sign_in(cfg.api_key.as_deref()),
                "username": cfg.username,
                "baseUrl": cfg.base_url,
                "openrouterKeys": cfg.openrouter_keys.iter().map(|k| json!({ "name": k.name, "masked": mask_key(&k.key) })).collect::<Vec<_>>(),
                "nvidiaKey": cfg.nvidia_key.as_deref().map(mask_key),
                "kiloKey": cfg.kilo_key.as_deref().map(mask_key),
                "kilo": cfg.kilo_on(),
                "copilot": cfg.copilot_token.is_some(),
                "copilotUser": cfg.copilot_user,
                "devin": cfg.devin_enabled,
                "kiro": cfg.kiro_enabled,
                "codex": cfg.codex_enabled,
                "claude": cfg.claude_enabled,
                "opencode": cfg.opencode_enabled,
                "opencodeSignedIn": cfg.opencode_signed_in,
                "grokSignedIn": cfg.grok_signed_in,
                "grokKey": cfg.grok_key.as_deref().map(mask_key),
                "hermesSignedIn": cfg.hermes_signed_in,
                "hermesKey": cfg.hermes_key.as_deref().map(mask_key),
                "additional": cfg.additional.iter().map(|l| json!({ "id": l.id, "masked": l.key.as_deref().map(mask_key) })).collect::<Vec<_>>(),
                "notifyFlash": cfg.notify_flash,
                "notifyToast": cfg.notify_toast,
                "discordPresence": cfg.discord_presence,
                "discordProject": cfg.discord_project,
                "autoUpdate": cfg.auto_update,
                "autoCompactTokens": cfg.auto_compact_tokens,
                "fallbacks": cfg.fallbacks,
                "experimentalImages": cfg.experimental_images,
                "imageModel": cfg.image_model,
                "games": cfg.games,
            })),
            ("mode", json!(self.chat.shared.mode())),
            ("model", json!({ "id": self.chat.model, "provider": self.chat.provider, "key": self.current_key(), "short": crate::api::short_id(&self.chat.model) })),
            ("effort", json!(self.chat.effort)),
            ("starred", json!(self.starred)),
            ("modelsLoading", json!(self.models_loading)),
            ("additionalErrors", json!(self.additional_errors)),
            ("kiroModelsError", json!(self.kiro_models_error)),
            ("codexModelsError", json!(self.codex_models_error)),
            ("claudeModelsError", json!(self.claude_models_error)),
            ("opencodeModelsError", json!(self.opencode_models_error)),
            ("grokModelsError", json!(self.grok_models_error)),
            ("hermesModelsError", json!(self.hermes_models_error)),
            ("run", json!({ "running": self.chat.running, "stopping": self.chat.stopping, "status": self.chat.status, "anyRunning": self.any_running() })),
            ("todos", todos_json(&self.chat.todos)),
            ("totals", json!({ "totals": self.chat.totals, "context": self.chat.context })),
            ("permission", json!(self.chat.permission.as_ref().map(permission_json))),
            ("plan", json!(self.chat.plan.as_ref().map(|p| &p.text))),
            ("questions", json!(self.chat.questions.as_ref().map(|p| p.questions.iter().map(|q| json!({
                "question": q.question, "header": q.header, "multiSelect": q.multi_select,
                "options": q.options.iter().map(|o| json!({ "label": o.label, "description": o.description })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>()))),
            ("queue", json!(self.chat.queue.iter().map(|q| json!({ "text": q.text, "files": q.files.len() })).collect::<Vec<_>>())),
            ("steering", json!(steering)),
            ("sessions", json!(self.sessions.iter().map(|s| {
                let live = std::iter::once(&self.chat).chain(&self.background).find(|c| c.session_id == s.id);
                json!({ "id": s.id, "cwd": s.cwd, "title": s.title, "ago": session::ago(s.updated), "running": live.is_some_and(|c| c.running), "waiting": live.is_some_and(Chat::waiting), "unread": live.is_some_and(|c| c.unread) })
            }).collect::<Vec<_>>())),
            ("projects", json!(self.projects)),
            ("chat", json!({ "id": self.chat.session_id, "cwd": self.chat.cwd, "project": self.project_label(), "title": self.chat_title(), "epoch": self.chat_epoch })),
            ("filesEpoch", json!(self.files_epoch)),
            ("focusEpoch", json!(self.focus_epoch)),
            ("login", json!({
                "busy": self.login.busy, "url": self.login.url, "error": self.login.error, "kiloCheck": kilo_check,
                "copilot": self.login.copilot.as_ref().map(SignIn::json), "copilotError": self.login.copilot_error,
                "devin": self.login.devin.as_ref().map(SignIn::json), "devinError": self.login.devin_error,
                "kiro": self.login.kiro.as_ref().map(SignIn::json), "kiroError": self.login.kiro_error,
                "codex": self.login.codex.as_ref().map(SignIn::json), "codexError": self.login.codex_error,
                "claude": self.login.claude.as_ref().map(SignIn::json), "claudeError": self.login.claude_error,
                "opencode": self.login.opencode.as_ref().map(SignIn::json), "opencodeError": self.login.opencode_error,
                "grok": self.login.grok.as_ref().map(SignIn::json), "grokError": self.login.grok_error,
                "hermes": self.login.hermes.as_ref().map(SignIn::json), "hermesError": self.login.hermes_error,
            })),
            ("skills", json!(self.skills.iter().map(|s| json!({ "name": s.name, "about": s.short_description(), "source": s.source, "dir": s.dir })).collect::<Vec<_>>())),
            ("link", json!({
                "on": self.link.is_some(),
                "cwd": self.link.as_ref().map(|l| l.cwd.clone()).unwrap_or_else(|| self.chat.cwd.clone()),
                "status": self.link_status,
                "online": self.link.is_some() && self.link_online,
                "mode": self.link.as_ref().map(|l| l.mode()),
                "since": self.link.as_ref().map(|_| self.link_since),
                "running": self.link_running,
                "log": self.link_log.iter().map(|c| json!({
                    "tool": c.tool, "label": c.label, "ok": c.ok, "at": c.at,
                    "edits": c.edits.iter().map(|e| json!({ "path": e.path, "added": e.added, "removed": e.removed, "created": e.created })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
                "edits": self.link_edits.iter().map(|e| json!({ "path": e.path, "added": e.added, "removed": e.removed, "created": e.created })).collect::<Vec<_>>(),
                "chatbots": self.link_chatbots.iter().map(|b| json!({ "name": b.name, "seen": b.seen })).collect::<Vec<_>>(),
                "permission": self.link_permission.as_ref().map(permission_json),
            })),
            ("usage", json!({ "info": self.usage.info, "busy": self.usage.busy, "message": self.usage.message })),
            ("voice", voice),
            ("updater", json!({ "enabled": self.updater.enabled(), "version": crate::update::VERSION, "status": status_json(&self.updater.status()) })),
            ("whatsNew", json!(self.whats_new)),
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
        if self.chat.reset {
            self.chat.reset = false;
            self.chat.changed.clear();
            let _ = self.handle.emit("items", json!({ "seq": seq, "reset": true, "items": self.chat.items.iter().map(item_json).collect::<Vec<_>>() }));
        } else if !self.chat.changed.is_empty() {
            let items: Vec<Value> = self.chat.changed.iter().filter_map(|&i| self.chat.items.get(i).map(|it| json!({ "at": i, "item": item_json(it) }))).collect();
            self.chat.changed.clear();
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
        self.chat.reset = false;
        self.chat.changed.clear();
        self.models_changed = false;
        json!({
            "seq": self.seq,
            "view": view,
            "models": self.models.iter().map(model_json).collect::<Vec<_>>(),
            "items": self.chat.items.iter().map(item_json).collect::<Vec<_>>(),
            "modes": MODES.iter().map(|m| json!({ "id": m, "label": config::mode_label(m) })).collect::<Vec<_>>(),
            "efforts": EFFORTS,
            "additionalProviders": config::ADDITIONAL.iter().map(|a| json!({ "id": a.id, "name": a.name, "needsKey": a.needs_key, "note": a.note, "keyUrl": a.key_url, "keyLink": a.key_link, "keyHint": a.key_hint })).collect::<Vec<_>>(),
            "kiloWarning": config::KILO_WARNING,
            "plans": { "copilot": crate::copilot::PLAN_URL, "devin": crate::devin::PLAN_URL, "kiro": crate::kiro::PLAN_URL, "codex": crate::codex::PLAN_URL, "claude": crate::claude::PLAN_URL, "opencode": crate::opencode::PLAN_URL, "grok": crate::grok::PLAN_URL, "hermes": crate::hermes::PLAN_URL },
            "languages": crate::instructions::LANGUAGES,
            "exe": std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "Scout.exe".into()),
        })
    }
}

/// Runs the workers' events into the core and tells the window, at most about 60 times a second.
pub fn pump(core: Arc<Mutex<Core>>, rx: Receiver<Ev>, wake: Receiver<()>) {
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::{Duration, Instant};
    // The balance is checked as the app starts and every 30 seconds after.
    const POLL_EVERY: Duration = Duration::from_secs(30);
    let mut polled: Option<Instant> = None;
    loop {
        if polled.is_none_or(|t| t.elapsed() >= POLL_EVERY) {
            polled = Some(Instant::now());
            core.lock().unwrap().usage_poll();
        }
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
