use crate::api::{self, ApiError, ChatRequest, Content, Delta, Message, ModelInfo, StreamResult, ToolCall};
use crate::config::{self, Config};
use crate::copilot::Msg as CopilotMsg;
use crate::devin::Msg as DevinMsg;
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
    /// Messages sent while Scout worked, now in the conversation: each one's text and attached files.
    Steered(Vec<(String, Vec<String>)>),
    /// How signing in to GitHub Copilot is going.
    Copilot(crate::copilot::Update),
    /// How signing in to Devin is going.
    Devin(crate::devin::Update),
    /// How signing in to Kiro is going.
    Kiro(crate::kiro::Update),
    /// Why Kiro's models couldn't be listed, from the last time the lists were loaded.
    KiroModelsError(Option<String>),
    /// How setting up Codex ("codex") or Claude Code ("claude") and signing in to it is going.
    AgentSignIn(&'static str, crate::node::Update),
    /// Why Codex's or Claude Code's models couldn't be listed, from the last time the lists were loaded.
    AgentModelsError(&'static str, Option<String>),
    /// How setting up OpenCode or signing in to it is going.
    Opencode(crate::opencode::Update),
    /// Why OpenCode's models couldn't be listed, from the last time the lists were loaded.
    OpencodeModelsError(Option<String>),
    /// How setting up Grok Build or signing in to it is going.
    Grok(crate::grok::Update),
    /// How signing in to Nous Portal is going.
    Hermes(crate::hermes::Update),
    /// Why Nous Portal's models couldn't be listed, from the last time the lists were loaded.
    HermesModelsError(Option<String>),
    /// Why Grok Build's models couldn't be listed, from the last time the lists were loaded.
    GrokModelsError(Option<String>),
    Finished(Box<Agent>, Result<(), String>),
    /// A merged model list, and whether it's the last of this fetch: the chosen model's provider is sent first.
    Models(Result<Vec<ModelInfo>, String>, bool),
    /// How Settings' test of the Kilo connection went.
    KiloCheck(Result<String, String>),
    /// What changed in the releases since the version that ran before this one.
    WhatsNew(Vec<crate::update::ReleaseNote>),
    /// Why linked additional providers didn't list their models, from the last time the lists were loaded.
    AdditionalErrors(Vec<String>),
    /// The account the key belongs to, from the model list.
    User(String),
    /// The Scout Enterprise team the account is in, read before the model list when it changed.
    Team(Option<crate::config::Team>),
    /// The account's limits for Settings > Usage, and what to say when it follows a change.
    Usage(Result<crate::api::UsageInfo, String>, Option<String>),
    /// The account's limits, from the 30-second refresh.
    UsagePolled(Result<crate::api::UsageInfo, String>),
    LoginUrl(String),
    /// The token and, when the server sent it, the username.
    Login(Result<(String, Option<String>), String>),
    Voice(crate::voice::Update),
    /// From the link that lets web chatbots use Scout.
    Link(crate::mcp::LinkEvent),
    /// From ScoutMote, which lets the website drive this Scout.
    Remote(crate::remote::RemoteEvent),
    /// An event from the worker of one chat, which may not be the one on screen.
    ForChat(u64, Box<Ev>),
}

#[derive(Clone)]
pub struct Hooks {
    /// The chat this worker runs, whose events are marked with it. None for what isn't about a chat.
    pub chat: Option<u64>,
    pub tx: Sender<Ev>,
    pub ctx: crate::waker::Waker,
}

impl Hooks {
    pub fn send(&self, ev: Ev) {
        let ev = match self.chat {
            Some(chat) => Ev::ForChat(chat, Box::new(ev)),
            None => ev,
        };
        let _ = self.tx.send(ev);
        self.ctx.request_repaint();
    }
}

/// Settings the window can change while a turn runs.
pub struct Shared {
    pub mode: Mutex<String>,
    /// The id to send to `provider`.
    pub model: Mutex<String>,
    /// "scoutcode", "openrouter", "nvidia", "kilo", "copilot", "devin", "kiro", "codex", "claude", "opencode", "grok", "hermes" or an additional provider's id.
    pub provider: Mutex<String>,
    pub effort: Mutex<String>,
    pub cancel: AtomicBool,
    /// Each model's edit tool, from /v1/models once the window has the list. The same for every chat.
    pub edit_tools: Arc<Mutex<HashMap<String, EditTool>>>,
    /// Each model's name, from the same list.
    pub model_names: Arc<Mutex<HashMap<String, String>>>,
    /// The models the same list says can't see images. One it doesn't mention is sent them and may refuse.
    pub no_vision: Arc<Mutex<HashSet<String>>>,
    /// Messages to add to the running turn at its next step.
    pub steer: Mutex<Vec<Steer>>,
}

/// A message sent into a running turn.
pub struct Steer {
    pub text: String,
    pub files: Vec<String>,
    /// The text with the files' contents, as the model gets it.
    pub content: Content,
    pub read: Vec<PathBuf>,
}

impl Shared {
    /// The settings for another chat: it starts on these, but stopping or changing one chat leaves the other alone.
    pub fn for_new_chat(&self) -> Arc<Shared> {
        Arc::new(Shared {
            mode: Mutex::new(self.mode()),
            model: Mutex::new(self.model.lock().unwrap().clone()),
            provider: Mutex::new(self.provider.lock().unwrap().clone()),
            effort: Mutex::new(self.effort.lock().unwrap().clone()),
            cancel: Default::default(),
            edit_tools: self.edit_tools.clone(),
            model_names: self.model_names.clone(),
            no_vision: self.no_vision.clone(),
            steer: Default::default(),
        })
    }

    /// The edit tools that suit `model`; str_replace until the server says otherwise.
    pub fn edit_tool(&self, model: &str) -> EditTool {
        self.edit_tools.lock().unwrap().get(model).copied().unwrap_or_default()
    }

    /// The name `model` goes by in the system prompt; its short id until the list is in, and for auto, which isn't listed.
    pub fn model_name(&self, model: &str) -> String {
        self.model_names.lock().unwrap().get(model).cloned().unwrap_or_else(|| crate::api::short_id(model))
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

/// Roughly what the messages cost. An image is priced by its size on screen, not by the length of its base64.
fn estimate_tokens(m: &[Message]) -> u64 {
    let chars = serde_json::to_string(m).map(|s| s.len() as u64).unwrap_or(0);
    let images: Vec<u64> = m.iter().filter_map(|m| if let Message::User { content } = m { Some(content.images()) } else { None }).flatten().map(|(_, data)| data.len() as u64).collect();
    chars.saturating_sub(images.iter().sum()) / 4 + images.len() as u64 * IMAGE_TOKENS
}

/// About what a provider charges for an image at the size attachments are sent.
const IMAGE_TOKENS: u64 = 1600;
/// Tool output above this many characters is cut in the middle before it enters the conversation.
const MAX_TOOL_CHARS: usize = 40_000;
/// Tool results shorter than this are never cleared: the stub would save next to nothing.
const MIN_PRUNE_CHARS: usize = 600;
/// The newest tool results are never cleared, since the model is usually still working with them.
const KEEP_RECENT_RESULTS: usize = 6;
/// Providers drop a cached prefix after a few idle minutes; editing history after that costs no extra cache miss.
const CACHE_TTL: Duration = Duration::from_secs(300);
const STUB: &str = "[Output cleared to save context";

pub fn cap_output(s: String) -> String {
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

/// The tool name and arguments one of Devin's own calls goes under, in the window and the session.
fn devin_shown(kind: &str, input: &Value, cwd: &str) -> (String, Value) {
    let s = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("");
    match kind {
        "read" => {
            let path = tools::resolve(cwd, s("file_path")).map_or_else(|_| s("file_path").to_string(), |abs| tools::rel(cwd, &abs));
            ("view".into(), json!({"path": path}))
        }
        "search" => ("grep".into(), json!({"pattern": s("query"), "path": s("path")})),
        "fetch" => ("web_fetch".into(), json!({"url": s("url")})),
        _ => ("devin".into(), input.clone()),
    }
}

/// What a Copilot session is for: the conversation itself, a sub-agent (by its description), or a one-off answer.
#[derive(Clone, Copy)]
enum CopilotRole<'a> {
    Main,
    Sub(&'a str),
    Quiet,
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
    /// What a refused call is told in plan mode.
    pub read_only_note: &'static str,
    /// The last turn stopped because the ScoutCode account can't pay for its model.
    pub out_of_credit: bool,
    /// The last turn failed before asking the model, so another model wouldn't help.
    pub before_model: bool,
    /// A failure goes straight to a fallback, or to the card that offers them, instead of being retried here.
    pub fail_fast: bool,
    /// A message whose turn failed before it joined the conversation, for a retry to send.
    unsent: Option<Content>,
    last_request_at: Option<Instant>,
    sys: Option<(String, EditTool, String, Message)>,
    preferences: crate::instructions::Preferences,
    /// The tool list for the edit tool it was built for.
    specs: (EditTool, Vec<Value>),
    read_only_specs: Vec<Value>,
}

/// A CLI that runs its own tools over the Agent Client Protocol: see `acp_run`.
struct AcpCli {
    /// What `Config::provider` says when a model runs here, and what its tool calls are called.
    tool: &'static str,
    name: &'static str,
    installed: fn() -> bool,
    ensure_runtime: fn(&mut dyn FnMut(u64, u64)) -> Result<(), String>,
    start: fn(&str, &str, &str, &str, String, Vec<(String, String)>, bool) -> Result<crate::acp::Run, String>,
    explain: fn(&str) -> String,
    /// What to say when it finishes having said nothing.
    no_reply: &'static str,
    /// Removes a finished session from its history, where `Run::finish` can't.
    forget: fn(&str),
}

static KIRO: AcpCli = AcpCli {
    tool: "kiro",
    name: "Kiro",
    installed: crate::kiro::installed,
    ensure_runtime: crate::kiro::ensure_runtime,
    start: crate::kiro::start,
    explain: crate::kiro::explain,
    no_reply: "Kiro finished without replying. Try rephrasing, or say \"continue\".",
    forget: |_| {},
};

static CODEX: AcpCli = AcpCli {
    tool: "codex",
    name: "Codex",
    installed: crate::codex::installed,
    ensure_runtime: crate::codex::ensure_runtime,
    start: crate::codex::start,
    explain: crate::codex::explain,
    no_reply: "Codex finished without replying. Try rephrasing, or say \"continue\".",
    forget: |_| {},
};

static CLAUDE: AcpCli = AcpCli {
    tool: "claude",
    name: "Claude Code",
    installed: crate::claude::installed,
    ensure_runtime: crate::claude::ensure_runtime,
    start: crate::claude::start,
    explain: crate::claude::explain,
    no_reply: "Claude Code finished without replying. Try rephrasing, or say \"continue\".",
    forget: |_| {},
};

static OPENCODE: AcpCli = AcpCli {
    tool: "opencode",
    name: "OpenCode",
    installed: crate::opencode::installed,
    ensure_runtime: crate::opencode::ensure_runtime,
    start: crate::opencode::start,
    explain: crate::opencode::explain,
    // It reports a refused model, like one past its free limit, only in its own log.
    no_reply: "OpenCode finished without replying. A free model may have hit its limit: try another, or sign in under Settings > Providers.",
    forget: crate::opencode::forget,
};

static GROK: AcpCli = AcpCli {
    tool: "grok",
    name: "Grok Build",
    installed: crate::grok::installed,
    ensure_runtime: crate::grok::ensure_runtime,
    start: crate::grok::start,
    explain: crate::grok::explain,
    no_reply: "Grok Build finished without replying. Try rephrasing, or say \"continue\".",
    forget: crate::grok::forget,
};

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
            read_only_note: "Plan mode is read-only. Call present_plan with your plan first.",
            out_of_credit: false,
            before_model: false,
            fail_fast: false,
            unsent: None,
            last_request_at: None,
            sys: None,
            preferences: Default::default(),
            specs: (EditTool::StrReplace, tools::specs(None, EditTool::StrReplace)),
            read_only_specs: tools::specs(Some(&tools::READ_ONLY), EditTool::StrReplace),
        }
    }

    fn cancelled(&self) -> bool {
        self.shared.cancel.load(Ordering::Relaxed)
    }

    /// Reload at turn boundaries; the prompt stays byte-identical within a turn until compaction or a model change.
    fn reload_instructions(&mut self) -> Result<(), ApiError> {
        self.preferences = crate::instructions::Preferences::load().map_err(|e| ApiError { message: format!("{e} Open Settings > Instructions to review it."), status: 0, retry_after: None, aborted: false })?;
        self.sys = None;
        Ok(())
    }

    fn system(&mut self) -> Message {
        let model = self.shared.model.lock().unwrap().clone();
        let edit = self.shared.edit_tool(&model);
        let name = self.shared.model_name(&model);
        if self.sys.as_ref().is_none_or(|(cwd, e, n, _)| *cwd != self.session.cwd || *e != edit || *n != name) {
            let msg = Message::System { content: prompt::system_prompt(&self.session.cwd, edit, &name, &self.preferences) };
            self.sys = Some((self.session.cwd.clone(), edit, name, msg));
        }
        self.sys.as_ref().unwrap().3.clone()
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
        // Only compaction gets here on Copilot; turns and sub-agents have their own way in, since it runs the tool loop.
        if provider == "copilot" {
            let content = self.copilot_run(messages, &[], CopilotRole::Quiet, h)?;
            return Ok(StreamResult { content, ..Default::default() });
        }
        // The same on Devin. Its summarizer has no tools.
        if provider == "devin" {
            let content = self.devin_run(messages, false, h)?;
            return Ok(StreamResult { content, ..Default::default() });
        }
        // The same on Kiro, in its plan mode.
        if provider == "kiro" {
            let content = self.acp_run(&KIRO, messages, false, h)?;
            return Ok(StreamResult { content, ..Default::default() });
        }
        for cli in [&CODEX, &CLAUDE] {
            if provider == cli.tool {
                let content = self.acp_run(cli, messages, false, h)?;
                return Ok(StreamResult { content, ..Default::default() });
            }
        }
        if provider == "opencode" {
            let content = self.acp_run(&OPENCODE, messages, false, h)?;
            return Ok(StreamResult { content, ..Default::default() });
        }
        if provider == "grok" {
            let content = self.acp_run(&GROK, messages, false, h)?;
            return Ok(StreamResult { content, ..Default::default() });
        }
        // A model that can't see images would refuse the whole request, so it gets a note in their place.
        let without_images: Vec<Message>;
        let messages = if self.shared.no_vision.lock().unwrap().contains(&model) && messages.iter().any(|m| matches!(m, Message::User { content } if !content.images().is_empty())) {
            if !quiet && matches!(messages.last(), Some(Message::User { content }) if !content.images().is_empty()) {
                h.send(Ev::Notice(format!("{} can't see images, so it only got the file names. Pick a model with vision to have it look at them.", self.shared.model_name(&model))));
            }
            without_images = messages.iter().map(|m| if let Message::User { content } = m { Message::User { content: content.without_images() } } else { m.clone() }).collect();
            &without_images[..]
        } else {
            messages
        };
        // OpenRouter tries the user's keys in order. A key that fails hands the request to the next one.
        // A team's OpenRouter goes through the server instead, with the team's key.
        let mut cfgs: Vec<Config> = if provider == "openrouter" && !self.cfg.team.as_ref().is_some_and(|t| t.runs("openrouter")) {
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
        // Only sub-agents and a retried summary are read-only, and neither gets the user's MCP tools.
        let tools = if read_only { self.read_only_specs.clone() } else { self.all_specs() };
        for attempt in 0.. {
            if !quiet {
                h.send(Ev::RequestStart);
            }
            let req = ChatRequest {
                model: &model,
                messages,
                tools: Some(&tools),
                tool_choice_none: no_tools,
                temperature: self.cfg.temperature,
                // ScoutCode and OpenRouter drop it for models without it; NIM isn't known to, and some of OVH's models refuse it.
                effort: (provider != "nvidia" && config::additional_provider(&provider).is_none()).then_some(effort.as_str()),
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
            // A sign-in's token can run out between attempts, so each one asks.
            let signed = if provider == "hermes" && !self.cfg.team.as_ref().is_some_and(|t| t.runs("hermes")) { crate::hermes::bearer(self.cfg.hermes_key.as_deref()).map(|t| cfgs[key].api_key = Some(t)) } else { Ok(()) };
            match signed.and_then(|()| api::stream_chat(&cfgs[key], req, &mut on, &self.shared.cancel)) {
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
                    if provider == "hermes" && e.status == 402 {
                        return Err(ApiError { message: crate::hermes::NO_CREDIT.into(), ..e });
                    }
                    if !e.retryable() || attempt >= 4 || self.fail_fast {
                        return Err(e);
                    }
                    // ScoutCode's free requests for the day are used up, so waiting a minute won't help.
                    if provider == "scoutcode" && e.status == 429 && e.retry_after.is_some_and(|s| s > 60) {
                        return Err(e);
                    }
                    // Kilo's limit is per hour, so a retry seconds later fails too and burns what's left. Only its own short wait gets one.
                    if provider == "kilo" && e.status == 429 && !(attempt == 0 && e.retry_after.is_some_and(|s| s <= 60)) {
                        return Err(ApiError { message: "Kilo's free models allow 200 requests an hour from your network, and that's used up. Switch provider in the model menu, use a paid model with a Kilo key (Settings > Providers), or try again later.".into(), ..e });
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

    /// Scout's own tools, then those of the user's MCP servers.
    fn all_specs(&self) -> Vec<Value> {
        let mut all = self.specs.1.clone();
        if self.cfg.experimental_images {
            all.push(tools::image_spec());
        }
        all.extend(crate::mcp_client::hub().specs());
        all
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
            return PermissionAnswer::Deny(self.read_only_note.into());
        }
        if perm == Permission::Edit && mode == "acceptEdits" {
            return PermissionAnswer::Allow;
        }
        let mut reason = None;
        if perm == Permission::Edit && mode == "auto" {
            let targets = if name == "update_memory" { crate::memory::target(&self.session.cwd, args).map(|p| p.to_string_lossy().to_string()).into_iter().collect() } else { edit_targets(args) };
            match targets.into_iter().find(|p| !inside_project(&self.session.cwd, p)) {
                None => return PermissionAnswer::Allow,
                Some(p) => reason = Some(format!("It writes {p}, outside the project's own files.")),
            }
        }
        let mut prefix = None;
        if tools::is_mcp(name) {
            if self.cfg.allow.get(&self.session.cwd).is_some_and(|rules| rules.iter().any(|r| r == name)) {
                return PermissionAnswer::Allow;
            }
            prefix = Some(name.to_string());
        }
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
        // What an MCP tool is about to do is all in its arguments.
        let (label, preview) = if tools::is_mcp(name) {
            (name.trim_start_matches(tools::MCP_PREFIX).replacen("__", " / ", 1), Some(Display::Text(serde_json::to_string_pretty(args).unwrap_or_default())))
        } else {
            (tools::label(name, args), tools::preview(name, args, &ctx))
        };
        let (tx, rx) = channel();
        h.send(Ev::AskPermission(PermissionRequest { tool: name.to_string(), label, preview, prefix: prefix.clone(), reason }, tx));
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

    pub fn run_tool(&mut self, call: &ToolCall, h: &Hooks) -> ToolResult {
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
        if !tools::known(name) && !tools::is_mcp(name) {
            let names: Vec<String> = self.all_specs().iter().filter_map(|s| s["function"]["name"].as_str().map(String::from)).collect();
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

        // Memory edited with the ordinary file tools gets update_memory's check too.
        if tools::permission(name) == Permission::Edit && name != "update_memory" {
            let cwd = &self.session.cwd;
            if edit_targets(args).iter().any(|p| tools::resolve(cwd, p).is_ok_and(|f| crate::memory::is_memory_file(cwd, &f))) {
                if let Some(why) = crate::memory::refuse_args(args) {
                    return ToolResult::err(format!("Not saved. {why}"), "refused");
                }
            }
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
            let paths: Vec<PathBuf> = if name == "update_memory" {
                crate::memory::target(&self.session.cwd, args).into_iter().collect()
            } else {
                tools::edit_paths(name, args).iter().filter_map(|p| tools::resolve(&self.session.cwd, p).ok()).collect()
            };
            for p in paths {
                self.snapshot(p);
            }
        }
        if tools::is_mcp(name) {
            return crate::mcp_client::hub().call(name, args, &self.shared.cancel);
        }
        if name == "web_search" {
            return tools::web_search(&self.cfg, args).unwrap_or_else(|e| ToolResult::err(e.clone(), e.chars().take(100).collect::<String>()));
        }
        if name == "generate_image" {
            if !self.cfg.experimental_images {
                return ToolResult::err("Image generation is off. The user can turn it on in Settings > Experimental.", "off");
            }
            return tools::generate_image(&self.cfg, args, &self.session.cwd).unwrap_or_else(|e| ToolResult::err(e.clone(), e.chars().take(100).collect::<String>()));
        }
        let cwd = self.session.cwd.clone();
        let mut progress = |t: &str| h.send(Ev::ToolProgress { id: id.to_string(), text: t.to_string() });
        let mut ctx = Ctx { cwd: &cwd, cancel: &self.shared.cancel, read_files: &mut self.read_files, on_progress: &mut progress };
        let mut result = match tools::run(name, args, &mut ctx) {
            Ok(r) => r,
            Err(_) if self.cancelled() => ToolResult::err("Interrupted by the user.", "interrupted"),
            Err(e) => {
                let first = e.lines().next().unwrap_or("").chars().take(100).collect::<String>();
                ToolResult::err(e, first)
            }
        };
        // Commands left running in the background say when they finish, on whatever the model does next.
        let notes = tools::finished_shell_notes();
        if !notes.is_empty() {
            result.output = format!("{}\n\n{notes}", result.output);
        }
        result
    }

    /// A read-only agent with its own context. Its tool calls show as status lines.
    fn run_subagent(&mut self, description: &str, task: &str, h: &Hooks) -> Result<String, ApiError> {
        let mut messages = vec![Message::System { content: prompt::subagent_prompt(&self.session.cwd, &self.preferences) }, Message::User { content: Content::Text(task.into()) }];
        if *self.shared.provider.lock().unwrap() == "copilot" {
            let specs = self.read_only_specs.clone();
            return self.copilot_run(&messages, &specs, CopilotRole::Sub(description), h);
        }
        // A browser sign-in may only use the approved models, which the sub-agent model may not be, and OpenRouter and
        // NIM have none of ours; use the session's model for those.
        let on_scout = *self.shared.provider.lock().unwrap() == "scoutcode";
        let model = (on_scout && self.cfg.api_key.is_some() && !config::is_sign_in(self.cfg.api_key.as_deref()) && !self.cfg.subagent_model.is_empty()).then(|| self.cfg.subagent_model.clone());
        let cwd = self.session.cwd.clone();
        for _ in 0..60 {
            let r = self.request(&messages, true, true, model.clone(), false, h)?;
            let calls = r.tool_calls.clone();
            messages.push(Message::Assistant { content: (!r.content.is_empty()).then(|| r.content.clone()), tool_calls: (!calls.is_empty()).then(|| calls.clone()), reasoning_content: None });
            if calls.is_empty() {
                return Ok(r.content);
            }
            for c in calls {
                let out = self.subagent_tool(description, &c, &cwd, h);
                messages.push(Message::Tool { tool_call_id: c.id, content: out });
            }
        }
        Ok("The sub-agent stopped after 60 steps without a final answer.".into())
    }

    /// One tool call from a sub-agent, which can only read.
    fn subagent_tool(&mut self, description: &str, c: &ToolCall, cwd: &str, h: &Hooks) -> String {
        let (n, args) = tools::canonical(&c.function.name, args_of(c));
        let n = n.as_str();
        if !tools::READ_ONLY.contains(&n) {
            return format!("Error: Unknown tool {n}.");
        }
        h.send(Ev::Status(format!("task: {description} · {n} {}", tools::label(n, &args)).chars().take(140).collect()));
        if n == "web_search" {
            return tools::web_search(&self.cfg, &args).map_or_else(|e| format!("Error: {e}"), |r| r.output);
        }
        let mut noop = |_: &str| {};
        let mut ctx = Ctx { cwd, cancel: &self.shared.cancel, read_files: &mut self.read_files, on_progress: &mut noop };
        match tools::run(n, &args, &mut ctx) {
            Ok(r) => r.output,
            Err(e) => format!("Error: {e}"),
        }
    }

    /// A conversation on Copilot, whose runtime loops over the model by itself: its tool calls come back here to run
    /// like any other, with the same permissions, and what they did goes in the session as usual. Returns the last text.
    fn copilot_run(&mut self, messages: &[Message], specs: &[Value], role: CopilotRole, h: &Hooks) -> Result<String, ApiError> {
        let fail = |message: String| ApiError { message, status: 400, retry_after: None, aborted: false };
        let main = matches!(role, CopilotRole::Main);
        if !crate::copilot::installed() {
            let (hooks, mut shown) = (h.clone(), 0);
            crate::copilot::ensure_runtime(&mut |done, total| {
                let mb = done / 1_000_000;
                if mb > shown {
                    shown = mb;
                    hooks.send(Ev::Status(if total > 0 { format!("Downloading Copilot… {}%", done * 100 / total) } else { format!("Downloading Copilot… {mb} MB") }));
                }
            })
            .map_err(fail)?;
        }
        let model = self.shared.model.lock().unwrap().clone();
        let (system, prompt) = crate::copilot::as_prompt(messages);
        let run = crate::copilot::Run::start(&self.cfg, &model, &self.session.cwd, system, prompt, crate::copilot::newest_images(messages), specs).map_err(fail)?;
        let (mut text, mut resets, mut said) = (String::new(), None, false);
        let result = loop {
            if self.cancelled() {
                run.abort();
                break Err(ApiError::aborted());
            }
            let Some(msg) = run.next(Duration::from_millis(100)) else { continue };
            match msg {
                CopilotMsg::Text(t) => {
                    said = true;
                    text.push_str(&t);
                    if main {
                        h.send(Ev::Text(t));
                    }
                }
                CopilotMsg::Reasoning(t) if main => h.send(Ev::Reasoning(t)),
                CopilotMsg::Reasoning(_) => {}
                CopilotMsg::RequestStart if main => h.send(Ev::RequestStart),
                CopilotMsg::RequestStart => {}
                CopilotMsg::RequestEnd => {}
                CopilotMsg::Usage { prompt, completion, cached, resets: r } => {
                    resets = r.or(resets);
                    self.add_usage(&StreamResult { usage: Some(api::Usage { prompt, completion, cached }), ..Default::default() }, main, h);
                    if main {
                        self.last_request_at = Some(Instant::now());
                    }
                }
                CopilotMsg::Tool { id, name, arguments, reply } => {
                    said = true;
                    let call = ToolCall { id, kind: "function".into(), extra_content: None, function: api::FunctionCall { name, arguments } };
                    let out = match role {
                        CopilotRole::Main => {
                            self.session.messages.push(Message::Assistant { content: (!text.is_empty()).then(|| std::mem::take(&mut text)), tool_calls: Some(vec![call.clone()]), reasoning_content: None });
                            h.send(Ev::AssistantDone);
                            let res = self.run_tool(&call, h);
                            let out = cap_output(res.output);
                            self.session.messages.push(Message::Tool { tool_call_id: call.id.clone(), content: out.clone() });
                            save_session(&mut self.session);
                            out
                        }
                        CopilotRole::Sub(d) => {
                            let cwd = self.session.cwd.clone();
                            self.subagent_tool(d, &call, &cwd, h)
                        }
                        CopilotRole::Quiet => "This request has no tools.".to_string(),
                    };
                    let _ = reply.send(out);
                }
                CopilotMsg::Idle => {
                    if !main {
                        break Ok(std::mem::take(&mut text));
                    }
                    if !text.is_empty() {
                        self.session.messages.push(Message::Assistant { content: Some(std::mem::take(&mut text)), tool_calls: None, reasoning_content: None });
                        h.send(Ev::AssistantDone);
                    }
                    save_session(&mut self.session);
                    // A message sent while it worked gets its answer now, in this turn.
                    let before = self.session.messages.len();
                    if self.take_steer(h) {
                        let new: Vec<String> = self.session.messages[before..].iter().filter_map(|m| if let Message::User { content } = m { Some(content.text()) } else { None }).collect();
                        save_session(&mut self.session);
                        run.send(new.join("\n\n"));
                        continue;
                    }
                    if !said {
                        h.send(Ev::Notice("Copilot finished without replying. Try rephrasing, or say \"continue\".".into()));
                    }
                    break Ok(String::new());
                }
                CopilotMsg::Failed { message, kind } => break Err(fail(crate::copilot::explain(&message, &kind, resets.as_deref()))),
            }
        };
        run.finish();
        result
    }

    /// One of Devin's writes or commands, run like any other tool, with what it did going in the session as usual.
    fn devin_tool(&mut self, call: ToolCall, text: &mut String, h: &Hooks) -> ToolResult {
        self.session.messages.push(Message::Assistant { content: (!text.is_empty()).then(|| std::mem::take(text)), tool_calls: Some(vec![call.clone()]), reasoning_content: None });
        h.send(Ev::AssistantDone);
        let res = self.run_tool(&call, h);
        self.session.messages.push(Message::Tool { tool_call_id: call.id, content: cap_output(res.output.clone()) });
        save_session(&mut self.session);
        res
    }

    /// A conversation on Devin, whose CLI loops over the model by itself. Its writes and commands come back here and run
    /// like any other tool, with the same permissions and undo. Its reads come back too, for their exact text. What it
    /// does on its own, such as searching, is only shown. With `main` false it's a one-off answer with no tools.
    fn devin_run(&mut self, messages: &[Message], main: bool, h: &Hooks) -> Result<String, ApiError> {
        let fail = |message: String| ApiError { message, status: 400, retry_after: None, aborted: false };
        if !crate::devin::installed() {
            let (hooks, mut shown) = (h.clone(), 0);
            crate::devin::ensure_runtime(&mut |done, total| {
                let mb = done / 1_000_000;
                if mb > shown {
                    shown = mb;
                    hooks.send(Ev::Status(if total > 0 { format!("Downloading Devin… {}%", done * 100 / total) } else { format!("Downloading Devin… {mb} MB") }));
                }
            })
            .map_err(fail)?;
        }
        let model = self.shared.model.lock().unwrap().clone();
        let cwd = self.session.cwd.clone();
        // Devin has its own system prompt, and Scout's describes tools it doesn't have. The user's saved instructions and memory still count.
        let (_, mut text_in) = crate::copilot::as_prompt(messages);
        if main {
            text_in = format!("<saved_instructions>\n{}</saved_instructions>\n\n{}\n{text_in}", self.preferences.prompt(&cwd), crate::memory::prompt_for_own_tools(&cwd));
        }
        let run = crate::devin::Run::start(&model, &cwd, &self.shared.mode(), text_in, crate::copilot::newest_images(messages), !main).map_err(fail)?;
        if main {
            h.send(Ev::RequestStart);
        }
        let (mut text, mut said) = (String::new(), false);
        // Its calls not settled yet: id → kind, title, input. Writes and commands wait here for their callback.
        let mut open: HashMap<String, (String, String, Value)> = HashMap::new();
        // Calls Scout ran itself, whose own updates are not to be shown as well.
        let mut handled: HashSet<String> = HashSet::new();
        // Commands that came back before Devin announced their call.
        let mut ran: Vec<String> = vec![];
        let mut next_id = 0;
        let result = loop {
            if self.cancelled() {
                run.abort();
                break Err(ApiError::aborted());
            }
            let Some(msg) = run.next(Duration::from_millis(100)) else { continue };
            match msg {
                DevinMsg::Text(t) => {
                    said = true;
                    text.push_str(&t);
                    if main {
                        h.send(Ev::Text(t));
                    }
                }
                DevinMsg::Reasoning(t) if main => h.send(Ev::Reasoning(t)),
                DevinMsg::Reasoning(_) => {}
                DevinMsg::Todos(t) if main => {
                    self.todos = t.clone();
                    h.send(Ev::Todos(t));
                }
                DevinMsg::Todos(_) => {}
                DevinMsg::Usage { prompt, completion, cached } => {
                    self.add_usage(&StreamResult { usage: Some(api::Usage { prompt, completion, cached }), ..Default::default() }, main, h);
                    if main {
                        self.last_request_at = Some(Instant::now());
                    }
                }
                DevinMsg::ToolStart { id, kind, title, input } => {
                    let command = input.get("command").and_then(Value::as_str);
                    if let Some(i) = command.filter(|_| kind == "execute").and_then(|c| ran.iter().position(|r| r == c)) {
                        ran.remove(i);
                        handled.insert(id);
                        continue;
                    }
                    if handled.contains(&id) || !main {
                        continue;
                    }
                    if kind != "edit" && kind != "execute" {
                        let (name, args) = devin_shown(&kind, &input, &cwd);
                        let label = Some(tools::label(&name, &args)).filter(|l| !l.is_empty()).unwrap_or_else(|| title.clone());
                        h.send(Ev::AssistantDone);
                        h.send(Ev::ToolCall { id: id.clone(), name, label });
                    }
                    open.insert(id, (kind, title, input));
                }
                DevinMsg::ToolEnd { id, failed, text: out } => {
                    if handled.remove(&id) {
                        continue;
                    }
                    let Some((kind, title, input)) = open.remove(&id) else { continue };
                    said = true;
                    let (name, args) = devin_shown(&kind, &input, &cwd);
                    // An edit or command that ends without coming back here wasn't run by Scout, so it hasn't been shown.
                    if kind == "edit" || kind == "execute" {
                        h.send(Ev::AssistantDone);
                        h.send(Ev::ToolCall { id: id.clone(), name: name.clone(), label: title.clone() });
                    }
                    let summary = out.lines().next().unwrap_or(if failed { "failed" } else { "done" }).chars().take(100).collect::<String>();
                    h.send(Ev::ToolResult { id: id.clone(), result: ToolResult { output: out.clone(), is_error: failed, summary: Some(summary), display: None } });
                    let call = ToolCall { id: id.clone(), kind: "function".into(), extra_content: None, function: api::FunctionCall { name, arguments: args.to_string() } };
                    self.session.messages.push(Message::Assistant { content: (!text.is_empty()).then(|| std::mem::take(&mut text)), tool_calls: Some(vec![call]), reasoning_content: None });
                    self.session.messages.push(Message::Tool { tool_call_id: id, content: cap_output(if out.is_empty() { title } else { out }) });
                    save_session(&mut self.session);
                }
                DevinMsg::Read { id, path, line, limit } => {
                    if !main {
                        run.reply_err(&id, "This request has no tools.");
                        continue;
                    }
                    match tools::read_raw(&cwd, &path, line, limit) {
                        Ok((abs, content)) => {
                            self.read_files.insert(abs);
                            run.reply(&id, json!({"content": content}));
                        }
                        Err(e) => run.reply_err(&id, &e),
                    }
                }
                DevinMsg::Write { id, path, content, tool } => {
                    if let Some(t) = &tool {
                        open.remove(t);
                        handled.insert(t.clone());
                    }
                    if !main {
                        run.reply_err(&id, "This request has no tools.");
                        continue;
                    }
                    // Devin saves its plan to a file by default, which plan mode has no prompt for: the reply is the plan.
                    if self.shared.mode() == "plan" {
                        run.reply_err(&id, "Plan mode is read-only here, so nothing can be saved. Give your plan as your reply.");
                        continue;
                    }
                    let Ok(abs) = tools::resolve(&cwd, &path) else {
                        run.reply_err(&id, "That path isn't valid.");
                        continue;
                    };
                    // Devin reads a file itself before changing it.
                    self.read_files.insert(abs.clone());
                    next_id += 1;
                    let call = ToolCall { id: tool.unwrap_or_else(|| format!("devin-{next_id}")), kind: "function".into(), extra_content: None, function: api::FunctionCall { name: "write_file".into(), arguments: json!({"path": tools::rel(&cwd, &abs), "content": content}).to_string() } };
                    let res = self.devin_tool(call, &mut text, h);
                    said = true;
                    if res.is_error {
                        run.reply_err(&id, &res.output);
                    } else {
                        run.reply(&id, json!({}));
                    }
                }
                DevinMsg::Exec { id, command, cwd: dir } => {
                    let known = open.iter().find(|(_, o)| o.0 == "execute" && o.2.get("command").and_then(Value::as_str) == Some(command.as_str())).map(|(k, _)| k.clone());
                    match &known {
                        Some(k) => {
                            open.remove(k);
                            handled.insert(k.clone());
                        }
                        None => ran.push(command.clone()),
                    }
                    let here = |p: &str| p.replace('\\', "/").trim_end_matches('/').to_lowercase();
                    if !main {
                        run.reply_terminal(&id, "This request has no tools.".into(), 1);
                    } else if self.shared.mode() == "plan" {
                        run.reply_terminal(&id, "Plan mode is read-only here, so commands don't run. Use your read and search tools.".into(), 1);
                    } else if !dir.is_empty() && here(&dir) != here(&cwd) {
                        run.reply_terminal(&id, format!("Scout runs commands in the project folder, {cwd}. To use another, put a cd in the command."), 1);
                    } else {
                        next_id += 1;
                        let call = ToolCall { id: known.unwrap_or_else(|| format!("devin-{next_id}")), kind: "function".into(), extra_content: None, function: api::FunctionCall { name: "bash".into(), arguments: json!({"command": command, "timeout_ms": 600_000}).to_string() } };
                        let res = self.devin_tool(call, &mut text, h);
                        said = true;
                        let code = if res.is_error { crate::devin::exit_code(&res.output) } else { 0 };
                        run.reply_terminal(&id, res.output, code);
                    }
                }
                DevinMsg::Permission { id, tool, is_command, options } => {
                    let call = open.get(&tool).cloned();
                    // A write or command comes back here to be asked about when it runs, so asking now would be asking twice.
                    let later = is_command || handled.contains(&tool) || call.as_ref().is_some_and(|c| c.0 == "edit" || c.0 == "execute");
                    let allow = later
                        || (main
                            && match self.shared.mode().as_str() {
                                "yolo" => true,
                                "plan" => false,
                                _ => {
                                    let label = call.map_or_else(|| tool.clone(), |c| c.1);
                                    let (tx, rx) = channel();
                                    h.send(Ev::AskPermission(PermissionRequest { tool: "devin".into(), label, preview: None, prefix: None, reason: None }, tx));
                                    !matches!(rx.recv().unwrap_or(PermissionAnswer::Deny(String::new())), PermissionAnswer::Deny(_))
                                }
                            });
                    run.reply_permission(&id, allow, &options);
                }
                DevinMsg::Idle(stop) => {
                    if stop == "cancelled" {
                        break Err(ApiError::aborted());
                    }
                    if !main {
                        break Ok(std::mem::take(&mut text));
                    }
                    if !text.is_empty() {
                        self.session.messages.push(Message::Assistant { content: Some(std::mem::take(&mut text)), tool_calls: None, reasoning_content: None });
                        h.send(Ev::AssistantDone);
                    }
                    save_session(&mut self.session);
                    // A message sent while it worked gets its answer now, in this turn.
                    let before = self.session.messages.len();
                    if self.take_steer(h) {
                        let new: Vec<String> = self.session.messages[before..].iter().filter_map(|m| if let Message::User { content } = m { Some(content.text()) } else { None }).collect();
                        save_session(&mut self.session);
                        run.send(new.join("\n\n"));
                        continue;
                    }
                    match stop.as_str() {
                        "max_tokens" | "max_turn_requests" => h.send(Ev::Notice("Devin hit its limit for one turn. Say \"continue\" to keep going.".into())),
                        "refusal" => h.send(Ev::Notice("Devin refused this request.".into())),
                        _ if !said => h.send(Ev::Notice("Devin finished without replying. Try rephrasing, or say \"continue\".".into())),
                        _ => {}
                    }
                    break Ok(String::new());
                }
                DevinMsg::Failed(e) => break Err(fail(crate::devin::explain(&e))),
            }
        };
        // Whatever it still had going stopped with it.
        for (id, (kind, ..)) in open {
            if kind != "edit" && kind != "execute" {
                h.send(Ev::ToolResult { id, result: ToolResult::err("Interrupted.", "interrupted") });
            }
        }
        run.finish();
        result
    }

    /// A conversation on a CLI that loops over the model and runs its own tools. Scout shows them, and answers what it asks
    /// permission for by the current mode, but they aren't Scout's to undo. With `main` false it's a one-off answer in its
    /// plan mode, with everything it asks for refused.
    fn acp_run(&mut self, cli: &AcpCli, messages: &[Message], main: bool, h: &Hooks) -> Result<String, ApiError> {
        use crate::acp::Msg as CliMsg;
        let (name, tool) = (cli.name, cli.tool);
        let fail = |message: String| ApiError { message, status: 400, retry_after: None, aborted: false };
        if !(cli.installed)() {
            let (hooks, mut shown) = (h.clone(), 0);
            (cli.ensure_runtime)(&mut |done, total| {
                let mb = done / 1_000_000;
                if mb > shown {
                    shown = mb;
                    hooks.send(Ev::Status(if total > 0 { format!("Downloading {name}… {}%", done * 100 / total) } else { format!("Downloading {name}… {mb} MB") }));
                }
            })
            .map_err(fail)?;
        }
        let model = self.shared.model.lock().unwrap().clone();
        let effort = self.shared.effort.lock().unwrap().clone();
        let cwd = self.session.cwd.clone();
        // It has its own system prompt, and Scout's describes tools it doesn't have. The user's saved instructions and memory still count.
        let (_, mut text_in) = crate::copilot::as_prompt(messages);
        if main {
            text_in = format!("<saved_instructions>\n{}</saved_instructions>\n\n{}\n{text_in}", self.preferences.prompt(&cwd), crate::memory::prompt_for_own_tools(&cwd));
        }
        let run = (cli.start)(&model, &cwd, &self.shared.mode(), &effort, text_in, crate::copilot::newest_images(messages), !main).map_err(|e| fail((cli.explain)(&e)))?;
        if main {
            h.send(Ev::RequestStart);
        }
        let (mut text, mut said, mut refused) = (String::new(), false, false);
        // Its calls not finished yet: id → title, input.
        let mut open: HashMap<String, (String, Value)> = HashMap::new();
        let result = loop {
            if self.cancelled() {
                run.abort();
                break Err(ApiError::aborted());
            }
            let Some(msg) = run.next(Duration::from_millis(100)) else { continue };
            match msg {
                CliMsg::Text(t) => {
                    said = true;
                    text.push_str(&t);
                    if main {
                        h.send(Ev::Text(t));
                    }
                }
                CliMsg::Reasoning(t) if main => h.send(Ev::Reasoning(t)),
                CliMsg::Reasoning(_) => {}
                CliMsg::Todos(t) if main => {
                    self.todos = t.clone();
                    h.send(Ev::Todos(t));
                }
                CliMsg::Todos(_) => {}
                // A call can be announced again as it fills in, and only the first shows.
                CliMsg::ToolStart { id, kind, title, input } => {
                    if main && !open.contains_key(&id) {
                        let title = if title.is_empty() { kind } else { title };
                        h.send(Ev::AssistantDone);
                        h.send(Ev::ToolCall { id: id.clone(), name: tool.into(), label: title.clone() });
                        open.insert(id, (title, input));
                    }
                }
                CliMsg::ToolEnd { id, failed, text: out } => {
                    let Some((title, input)) = open.remove(&id) else { continue };
                    said = true;
                    let summary = out.lines().next().unwrap_or(if failed { "failed" } else { "done" }).chars().take(100).collect::<String>();
                    h.send(Ev::ToolResult { id: id.clone(), result: ToolResult { output: out.clone(), is_error: failed, summary: Some(summary), display: None } });
                    let call = ToolCall { id: id.clone(), kind: "function".into(), extra_content: None, function: api::FunctionCall { name: tool.into(), arguments: input.to_string() } };
                    self.session.messages.push(Message::Assistant { content: (!text.is_empty()).then(|| std::mem::take(&mut text)), tool_calls: Some(vec![call]), reasoning_content: None });
                    self.session.messages.push(Message::Tool { tool_call_id: id, content: cap_output(if out.is_empty() { title } else { out }) });
                    save_session(&mut self.session);
                }
                CliMsg::Permission { id, kind, title, options } => {
                    let allow = main
                        && match self.shared.mode().as_str() {
                            "yolo" => true,
                            "plan" => false,
                            "acceptEdits" if kind == "edit" => true,
                            _ => {
                                let (tx, rx) = channel();
                                h.send(Ev::AskPermission(PermissionRequest { tool: tool.into(), label: title, preview: None, prefix: None, reason: None }, tx));
                                !matches!(rx.recv().unwrap_or(PermissionAnswer::Deny(String::new())), PermissionAnswer::Deny(_))
                            }
                        };
                    refused |= !allow;
                    run.reply_permission(&id, allow, &options);
                }
                CliMsg::Idle(stop) => {
                    // Grok ends its turn as cancelled when a tool is refused, which isn't the user stopping it.
                    if stop == "cancelled" && !refused {
                        break Err(ApiError::aborted());
                    }
                    if !main {
                        break Ok(std::mem::take(&mut text));
                    }
                    if !text.is_empty() {
                        self.session.messages.push(Message::Assistant { content: Some(std::mem::take(&mut text)), tool_calls: None, reasoning_content: None });
                        h.send(Ev::AssistantDone);
                    }
                    save_session(&mut self.session);
                    // A message sent while it worked gets its answer now, in this turn.
                    let before = self.session.messages.len();
                    if self.take_steer(h) {
                        let new: Vec<String> = self.session.messages[before..].iter().filter_map(|m| if let Message::User { content } = m { Some(content.text()) } else { None }).collect();
                        save_session(&mut self.session);
                        run.send(new.join("\n\n"));
                        continue;
                    }
                    match stop.as_str() {
                        "max_tokens" | "max_turn_requests" => h.send(Ev::Notice(format!("{name} hit its limit for one turn. Say \"continue\" to keep going."))),
                        "refusal" => h.send(Ev::Notice(format!("{name} refused this request."))),
                        "cancelled" => h.send(Ev::Notice(format!("{name} stopped, since it wasn't allowed to do that. Tell it what to do instead."))),
                        _ if !said => h.send(Ev::Notice(cli.no_reply.into())),
                        _ => {}
                    }
                    break Ok(String::new());
                }
                CliMsg::Failed(e) => break Err(fail((cli.explain)(&e))),
            }
        };
        // Whatever it still had going stopped with it.
        for id in open.into_keys() {
            h.send(Ev::ToolResult { id, result: ToolResult::err("Interrupted.", "interrupted") });
        }
        (cli.forget)(&run.finish());
        result
    }

    /// Runs one real Kiro turn on the CLI's sign-in from a test; see `live_kiro_turns`.
    #[cfg(test)]
    fn kiro_turn_for_test(&mut self, text: &str, h: &Hooks) -> Result<(), ApiError> {
        self.session.messages.push(Message::User { content: Content::Text(text.into()) });
        let req = self.session.messages.clone();
        self.acp_run(&KIRO, &req, true, h).map(|_| ())
    }

    /// Tells the model the mode when it changed, at the end of the newest message: earlier messages stay byte-identical.
    fn announce_mode(&mut self) {
        let mode = self.shared.mode();
        if self.session.mode.as_deref() == Some(mode.as_str()) {
            return;
        }
        match self.session.messages.last_mut() {
            Some(Message::User { content: Content::Text(t) }) | Some(Message::Tool { content: t, .. }) => t.push_str(&prompt::mode_note(&mode)),
            // Onto the text, so that it reads back the same as a plain message.
            Some(Message::User { content: Content::Parts(p) }) => match p.iter_mut().find_map(|x| x.get_mut("text")) {
                Some(Value::String(t)) => t.push_str(&prompt::mode_note(&mode)),
                _ => p.push(json!({"type":"text","text":prompt::mode_note(&mode)})),
            },
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
        self.reload_instructions()?;
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
            Message::Assistant { content: Some("✓".into()), tool_calls: None, reasoning_content: None },
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
            self.session.messages.push(Message::User { content: s.content });
            shown.push((s.text, s.files));
        }
        h.send(Ev::Steered(shown));
        true
    }

    /// Runs the agent loop for one user message until the model stops calling tools.
    pub fn turn(&mut self, input: Content, h: &Hooks) -> Result<(), ApiError> {
        self.unsent = Some(input.clone());
        self.before_model = false;
        self.reload_instructions().inspect_err(|_| self.before_model = true)?;
        self.auto_prune(h);
        if self.should_compact() {
            self.compact("", h)?;
        }
        self.checkpoints.push(Checkpoint { at: self.session.messages.len(), label: input.text(), files: HashMap::new() });
        self.session.messages.push(Message::User { content: input });
        self.unsent = None;
        save_session(&mut self.session);
        self.carry_on(h)
    }

    /// Picks a stopped turn up where it left off, on whichever model is chosen now.
    pub fn retry(&mut self, h: &Hooks) -> Result<(), ApiError> {
        self.before_model = false;
        match self.unsent.take() {
            Some(input) => self.turn(input, h),
            None => self.carry_on(h),
        }
    }

    /// What to show for a failed turn, noting whether it failed because the account can't pay.
    pub fn failed(&mut self, e: ApiError) -> String {
        self.out_of_credit = e.status == 402 && *self.shared.provider.lock().unwrap() == "scoutcode";
        e.message
    }

    /// Asks the model until it stops calling tools, from the conversation as it stands.
    fn carry_on(&mut self, h: &Hooks) -> Result<(), ApiError> {
        if *self.shared.provider.lock().unwrap() == "copilot" {
            self.announce_mode();
            self.session.model = self.shared.model.lock().unwrap().clone();
            let model = self.session.model.clone();
            let edit = self.shared.edit_tool(&model);
            if self.specs.0 != edit {
                self.specs = (edit, tools::specs(None, edit));
            }
            let (specs, mut req) = (self.all_specs(), vec![self.system()]);
            req.extend_from_slice(&self.session.messages);
            let r = self.copilot_run(&req, &specs, CopilotRole::Main, h).map(|_| ());
            save_session(&mut self.session);
            return r;
        }

        if *self.shared.provider.lock().unwrap() == "devin" {
            self.session.model = self.shared.model.lock().unwrap().clone();
            let req = self.session.messages.clone();
            let r = self.devin_run(&req, true, h).map(|_| ());
            save_session(&mut self.session);
            return r;
        }
        for cli in [&KIRO, &CODEX, &CLAUDE, &OPENCODE, &GROK] {
            if *self.shared.provider.lock().unwrap() == cli.tool {
                self.session.model = self.shared.model.lock().unwrap().clone();
                let req = self.session.messages.clone();
                let r = self.acp_run(cli, &req, true, h).map(|_| ());
                save_session(&mut self.session);
                return r;
            }
        }

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
    fn a_workers_events_say_which_chat_they_are_from() {
        let (tx, rx) = channel();
        let h = Hooks { chat: Some(7), tx, ctx: crate::waker::Waker::default() };
        h.clone().send(Ev::Status("working".into()));
        assert!(matches!(rx.try_recv(), Ok(Ev::ForChat(7, ev)) if matches!(*ev, Ev::Status(ref s) if s == "working")));
        Hooks { chat: None, ..h }.send(Ev::User("someone".into()));
        assert!(matches!(rx.try_recv(), Ok(Ev::User(_))));
    }

    #[test]
    fn chats_stop_and_change_settings_on_their_own() {
        let a = Arc::new(Shared { mode: Mutex::new("plan".into()), model: Mutex::new("m".into()), provider: Mutex::new("kilo".into()), effort: Mutex::new("low".into()), cancel: AtomicBool::new(true), edit_tools: Default::default(), model_names: Default::default(), no_vision: Default::default(), steer: Default::default() });
        a.steer.lock().unwrap().push(Steer { text: "t".into(), files: vec![], content: "t".into(), read: vec![] });
        let b = a.for_new_chat();
        assert_eq!((b.mode(), b.model.lock().unwrap().as_str(), b.provider.lock().unwrap().as_str()), ("plan".to_string(), "m", "kilo"));
        assert!(!b.cancel.load(Ordering::Relaxed));
        assert!(b.steer.lock().unwrap().is_empty());
        *b.model.lock().unwrap() = "other".into();
        assert_eq!(*a.model.lock().unwrap(), "m");
        // The model list comes once, for all of them.
        a.model_names.lock().unwrap().insert("m".into(), "M".into());
        assert_eq!(b.model_name("m"), "M");
    }

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

    /// Runs a real Kiro turn on the CLI's sign-in: `cargo test -- --ignored live_kiro_turns --nocapture`. Checks the models
    /// list, that a refused tool does nothing while Kiro asks first, and that an allowed one works. Uses up Kiro credits.
    #[test]
    #[ignore]
    fn live_kiro_turns() {
        crate::kiro::ensure_runtime(&mut |_, _| {}).unwrap();
        assert!(crate::kiro::signed_in(), "sign in to Kiro first");
        let models = crate::kiro::list_models().unwrap();
        println!("models: {models:?}");
        assert!(!models.is_empty());
        let dir = std::env::temp_dir().join(format!("scout-kiro-{}", crate::session::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let shared = Arc::new(Shared { mode: Mutex::new("default".into()), model: Mutex::new("auto".into()), provider: Mutex::new("kiro".into()), effort: Mutex::new("low".into()), cancel: AtomicBool::new(false), edit_tools: Default::default(), model_names: Default::default(), no_vision: Default::default(), steer: Default::default() });
        let mut agent = Agent::new(config::load_config(), crate::session::new_session(&dir.to_string_lossy(), "auto"), shared.clone());
        let (tx, rx) = channel();
        let h = Hooks { chat: None, tx, ctx: crate::waker::Waker::default() };
        // Answers what Kiro asks with `allow`, and prints the rest. Returns how many times it was asked.
        let turn = |agent: &mut Agent, text: &str, allow: bool| {
            let mut asked = 0;
            std::thread::scope(|s| {
                let run = s.spawn(|| agent.kiro_turn_for_test(text, &h));
                while !run.is_finished() {
                    for ev in rx.try_iter() {
                        match ev {
                            Ev::Text(t) => print!("{t}"),
                            Ev::ToolCall { label, .. } => println!("\ntool: {label}"),
                            Ev::ToolResult { result, .. } => println!("  -> {:?}", result.summary),
                            Ev::Notice(n) => println!("notice: {n}"),
                            Ev::AskPermission(req, reply) => {
                                asked += 1;
                                println!("\nasked: {} -> {allow}", req.label);
                                let _ = reply.send(if allow { PermissionAnswer::Allow } else { PermissionAnswer::Deny(String::new()) });
                            }
                            _ => {}
                        }
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                run.join().unwrap().unwrap();
            });
            asked
        };
        let target = dir.join("kiro-note.txt");
        let write = "Create a file named kiro-note.txt in the current folder containing exactly the word hello. Use your file tool, and don't ask me questions.";
        let asked = turn(&mut agent, write, false);
        assert!(asked > 0, "Kiro must ask first: autopilot should be off");
        assert!(!target.exists(), "a refused write must not happen");
        let asked = turn(&mut agent, write, true);
        assert!(asked > 0);
        assert!(target.exists(), "an allowed write happens");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Runs real Devin turns on the CLI's sign-in: `cargo test -- --ignored live_devin_turns --nocapture`. Checks that Scout's
    /// tools do the work, that a refused command doesn't run, that the next turn remembers the last, undo and compaction.
    #[test]
    #[ignore]
    fn live_devin_turns() {
        crate::devin::ensure_runtime(&mut |_, _| {}).unwrap();
        assert!(crate::devin::signed_in(), "sign in to Devin first");
        let models = crate::devin::list_models();
        println!("models: {models:?}");
        assert!(!models.is_empty());
        let dir = std::env::temp_dir().join(format!("scout-devin-{}", crate::session::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("seed.txt"), "first\nsecond\n").unwrap();
        let shared = Arc::new(Shared {
            mode: Mutex::new("yolo".into()),
            model: Mutex::new("auto".into()),
            provider: Mutex::new("devin".into()),
            effort: Mutex::new("low".into()),
            cancel: AtomicBool::new(false),
            edit_tools: Default::default(),
            model_names: Default::default(),
            no_vision: Default::default(),
            steer: Default::default(),
        });
        let mut agent = Agent::new(config::load_config(), crate::session::new_session(&dir.to_string_lossy(), "auto"), shared.clone());
        let (tx, rx) = channel();
        let h = Hooks { chat: None, tx, ctx: crate::waker::Waker::default() };
        let show = |rx: &std::sync::mpsc::Receiver<Ev>| {
            let mut text = String::new();
            let mut tools = vec![];
            for ev in rx.try_iter() {
                match ev {
                    Ev::Text(t) => text.push_str(&t),
                    Ev::ToolCall { name, label, .. } => {
                        println!("tool: {name} {label}");
                        tools.push(name);
                    }
                    Ev::ToolResult { result, .. } => println!("  -> {:?}", result.summary),
                    Ev::Notice(n) => println!("notice: {n}"),
                    _ => {}
                }
            }
            println!("reply: {text}");
            (text, tools)
        };

        agent.turn("Read seed.txt, change the word second in it to SECOND, create hello.txt containing exactly: hi from scout, then run a shell command that prints hello.txt. Reply in one short sentence.".into(), &h).unwrap();
        let (_, used) = show(&rx);
        assert_eq!(std::fs::read_to_string(dir.join("hello.txt")).unwrap().trim(), "hi from scout");
        assert_eq!(std::fs::read_to_string(dir.join("seed.txt")).unwrap().replace("\r\n", "\n"), "first\nSECOND\n");
        assert!(used.iter().any(|n| n == "write_file") && used.iter().any(|n| n == "bash"), "Scout's tools did the work: {used:?}");

        agent.turn("What was the name of the file you just created? Answer with the file name only, and don't use any tools.".into(), &h).unwrap();
        assert!(show(&rx).0.contains("hello.txt"), "the second turn remembers the first");

        // Refused by the user: the window's answer to the prompt.
        *shared.mode.lock().unwrap() = "default".into();
        let (answer_tx, answer_rx) = channel::<()>();
        let refused = {
            let dir = dir.clone();
            std::thread::spawn(move || {
                agent.turn("Run this shell command: rm hello.txt".into(), &h).unwrap();
                let _ = answer_tx.send(());
                (agent, dir, h)
            })
        };
        let mut denied = 0;
        while answer_rx.try_recv().is_err() {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Ev::AskPermission(req, reply)) => {
                    println!("asked: {} {}", req.tool, req.label);
                    denied += 1;
                    let _ = reply.send(PermissionAnswer::Deny("Not now.".into()));
                }
                Ok(Ev::Text(t)) => print!("{t}"),
                _ => {}
            }
        }
        let (mut agent, dir, h) = refused.join().unwrap();
        assert!(denied > 0, "Scout asked before running it");
        assert!(dir.join("hello.txt").exists(), "the refused command didn't run");

        *shared.mode.lock().unwrap() = "yolo".into();
        let (label, restored, _) = {
            let _ = agent.undo();
            agent.undo().unwrap()
        };
        println!("undid: {label} → {restored:?}");
        let _ = agent.undo();
        println!("files after undo: {:?}", std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect::<Vec<_>>());
        assert!(!dir.join("hello.txt").exists(), "undo removed the file Devin created");

        agent.turn("Reply with the word ok.".into(), &h).unwrap();
        assert!(agent.compact("", &h).is_ok());
        show(&rx);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Runs real Copilot turns on the saved sign-in: `cargo test -- --ignored live_copilot --nocapture`. Checks that Scout's
    /// tools do the work, that the next turn remembers the last, that a refused command doesn't run, and undo.
    #[test]
    #[ignore]
    fn live_copilot() {
        let cfg = config::load_config();
        assert!(cfg.copilot_token.is_some(), "sign in to Copilot in the app first");
        let dir = std::env::temp_dir().join(format!("scout-copilot-{}", crate::session::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let shared = Arc::new(Shared {
            mode: Mutex::new("yolo".into()),
            model: Mutex::new("auto".into()),
            provider: Mutex::new("copilot".into()),
            effort: Mutex::new("low".into()),
            cancel: AtomicBool::new(false),
            edit_tools: Default::default(),
            model_names: Default::default(),
            no_vision: Default::default(),
            steer: Default::default(),
        });
        let mut agent = Agent::new(cfg, crate::session::new_session(&dir.to_string_lossy(), "auto"), shared.clone());
        let (tx, rx) = channel();
        let h = Hooks { chat: None, tx, ctx: crate::waker::Waker::default() };
        let show = |rx: &std::sync::mpsc::Receiver<Ev>| {
            let mut text = String::new();
            for ev in rx.try_iter() {
                match ev {
                    Ev::Text(t) => text.push_str(&t),
                    Ev::ToolCall { name, label, .. } => println!("tool: {name} {label}"),
                    Ev::ToolResult { result, .. } => println!("  -> {:?}", result.summary),
                    Ev::Notice(n) => println!("notice: {n}"),
                    _ => {}
                }
            }
            println!("reply: {text}");
            text
        };

        agent.turn("Create a file hello.txt containing exactly `hi from scout`, then run a shell command that prints its contents. Reply in one short sentence.".into(), &h).unwrap();
        show(&rx);
        assert_eq!(std::fs::read_to_string(dir.join("hello.txt")).unwrap().trim(), "hi from scout");
        assert!(agent.session.messages.iter().any(|m| matches!(m, Message::Tool { .. })), "the tool calls are in the session");

        agent.turn("What was the name of the file you just created? Answer with the file name only, and don't use any tools.".into(), &h).unwrap();
        assert!(show(&rx).contains("hello.txt"), "the second turn remembers the first");

        *shared.mode.lock().unwrap() = "plan".into();
        agent.turn("Use the bash tool to run: echo refused > refused.txt".into(), &h).unwrap();
        show(&rx);
        assert!(!dir.join("refused.txt").exists(), "plan mode refused the command");
        *shared.mode.lock().unwrap() = "yolo".into();

        agent.compact("", &h).unwrap();
        assert!(matches!(&agent.session.messages[0], Message::User { content: Content::Text(t) } if t.contains("compacted")));

        println!("totals: {:?}", agent.session.totals);
        assert!(agent.session.totals.requests > 0 && agent.session.totals.prompt_tokens > 0);
        let _ = std::fs::remove_file(crate::config::home().join("sessions").join(format!("{}.json", agent.session.id)));
        let _ = std::fs::remove_dir_all(&dir);
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
            model_names: Default::default(),
            no_vision: Default::default(),
            steer: Default::default(),
        });
        let mut agent = Agent::new(cfg.clone(), crate::session::new_session(&cwd, &cfg.model), shared);
        let (tx, rx) = channel();
        let h = Hooks { chat: None, tx, ctx: crate::waker::Waker::default() };
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

    /// A stand-in for Kilo: lists two free models and a paid one, and answers every chat with a 429, with a short
    /// Retry-After when asked. Keeps each chat's headers.
    fn kilo_stub(retry_after: bool) -> (String, Arc<Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let chats = Arc::new(Mutex::new(vec![]));
        let seen = chats.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                // All of it: closing with some of it unread would look like a network failure to the client.
                let (mut req, mut buf) = (String::new(), [0; 65536]);
                loop {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    req.push_str(&String::from_utf8_lossy(&buf[..n]));
                    let Some((head, body)) = req.split_once("\r\n\r\n") else {
                        if n == 0 {
                            break;
                        }
                        continue;
                    };
                    let want = head.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
                    if n == 0 || body.len() >= want {
                        break;
                    }
                }
                let reply = if req.starts_with("GET") {
                    let body = r#"{"data":[{"id":"kilo-auto/free","isFree":true,"supported_parameters":["tools"]},{"id":"music/free","isFree":true,"supported_parameters":[]},{"id":"paid/one","isFree":false,"supported_parameters":["tools"]}]}"#;
                    format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                } else {
                    seen.lock().unwrap().push(req);
                    let extra = if retry_after { "Retry-After: 1\r\n" } else { "" };
                    let body = r#"{"error":{"message":"Rate limit exceeded for free models.","code":429}}"#;
                    format!("HTTP/1.1 429 Too Many Requests\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                };
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        (url, chats)
    }

    fn kilo_agent() -> (Agent, Hooks) {
        let shared = Arc::new(Shared { mode: Mutex::new("default".into()), model: Mutex::new("kilo-auto/free".into()), provider: Mutex::new("kilo".into()), effort: Mutex::new("low".into()), cancel: AtomicBool::new(false), edit_tools: Default::default(), model_names: Default::default(), no_vision: Default::default(), steer: Default::default() });
        let mut cfg = config::load_config();
        (cfg.kilo_enabled, cfg.kilo_key) = (true, None);
        let (tx, _) = std::sync::mpsc::channel();
        (Agent::new(cfg, crate::session::new_session(".", "kilo-auto/free"), shared), Hooks { chat: None, tx, ctx: Default::default() })
    }

    // One test, since the URL is in the environment and tests run side by side.
    #[test]
    fn kilo_needs_no_key_and_does_not_burn_its_limit_on_retries() {
        let say = [Message::User { content: Content::Text("hi".into()) }];

        let (url, chats) = kilo_stub(false);
        unsafe { std::env::set_var("KILO_BASE_URL", &url) };
        let (mut agent, h) = kilo_agent();
        let listed: Vec<String> = api::list_kilo(&agent.cfg).unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(listed, ["kilo-auto/free"], "no key: only free models that have tools");
        let e = agent.request(&say, false, true, None, false, &h).unwrap_err();
        assert!(e.message.contains("200 requests an hour"), "{}", e.message);
        assert_eq!(chats.lock().unwrap().len(), 1, "a 429 is not retried");
        assert!(!chats.lock().unwrap()[0].to_lowercase().contains("authorization"), "no key means no header");

        let (url, chats) = kilo_stub(true);
        unsafe { std::env::set_var("KILO_BASE_URL", &url) };
        agent.cfg.kilo_key = Some("k".into());
        assert_eq!(api::list_kilo(&agent.cfg).unwrap().len(), 2, "with a key: paid models too");
        agent.request(&say, false, true, None, false, &h).unwrap_err();
        assert_eq!(chats.lock().unwrap().len(), 2, "its own short wait gets one retry");
        assert!(chats.lock().unwrap()[0].to_lowercase().contains("authorization: bearer k"));
        unsafe { std::env::remove_var("KILO_BASE_URL") };
    }

    #[test]
    fn mcp_tools_need_permission() {
        let shared = Arc::new(Shared { mode: Mutex::new("plan".into()), model: Mutex::new("m".into()), provider: Mutex::new("scoutcode".into()), effort: Mutex::new("low".into()), cancel: AtomicBool::new(false), edit_tools: Default::default(), model_names: Default::default(), no_vision: Default::default(), steer: Default::default() });
        let mut agent = Agent::new(config::load_config(), crate::session::new_session(".", "m"), shared.clone());
        agent.cfg.allow.clear();
        let (tx, rx) = channel();
        let h = Hooks { chat: None, tx, ctx: crate::waker::Waker::default() };
        let call = ToolCall { id: "1".into(), kind: "function".into(), extra_content: None, function: api::FunctionCall { name: "mcp__srv__do_it".into(), arguments: r#"{"a":1}"#.into() } };
        // Plan mode is read-only, and a server's tool might do anything.
        assert!(agent.run_tool(&call, &h).output.contains("read-only"));

        for mode in ["default", "acceptEdits", "auto"] {
            *shared.mode.lock().unwrap() = mode.into();
            let (asked_tx, asked_rx) = channel();
            let waiter = std::thread::scope(|s| {
                let hooks = &h;
                let agent = &mut agent;
                let call = &call;
                let run = s.spawn(move || agent.run_tool(call, hooks));
                for ev in rx.iter() {
                    if let Ev::AskPermission(req, reply) = ev {
                        asked_tx.send((req.label.clone(), req.prefix.clone())).unwrap();
                        reply.send(PermissionAnswer::Deny(String::new())).unwrap();
                        break;
                    }
                }
                run.join().unwrap()
            });
            assert!(waiter.is_error && waiter.output.contains("denied"), "{mode}: {}", waiter.output);
            let (label, prefix) = asked_rx.recv().unwrap();
            assert_eq!((label.as_str(), prefix.as_deref()), ("srv / do_it", Some("mcp__srv__do_it")), "{mode}");
        }

        // Allowed for this project: no question, and with no such server it says so.
        *shared.mode.lock().unwrap() = "default".into();
        agent.cfg.allow.insert(agent.session.cwd.clone(), vec!["mcp__srv__do_it".into()]);
        let r = agent.run_tool(&call, &h);
        assert!(r.is_error && r.output.contains("isn't connected"), "{}", r.output);
        assert!(!rx.try_iter().any(|e| matches!(e, Ev::AskPermission(..))));
        assert!(!tools::READ_ONLY.contains(&"mcp__srv__do_it"));
    }

    #[test]
    fn rewind_restores_files_changed_since() {
        let dir = std::env::temp_dir().join(format!("scout-rewind-{}", crate::session::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&a, "v2").unwrap();
        std::fs::write(&b, "made in turn two").unwrap();
        let cwd = dir.to_string_lossy().to_string();
        let shared = Arc::new(Shared { mode: Mutex::new("default".into()), model: Mutex::new("m".into()), provider: Mutex::new("scoutcode".into()), effort: Mutex::new("low".into()), cancel: AtomicBool::new(false), edit_tools: Default::default(), model_names: Default::default(), no_vision: Default::default(), steer: Default::default() });
        let mut agent = Agent::new(config::load_config(), crate::session::new_session(&cwd, "m"), shared);
        let edit = |id: &str| Message::Assistant {
            content: None,
            tool_calls: Some(vec![ToolCall { id: id.into(), kind: "function".into(), extra_content: None, function: api::FunctionCall { name: "write_file".into(), arguments: "{}".into() } }]),
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
