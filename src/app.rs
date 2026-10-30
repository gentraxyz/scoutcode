use crate::agent::{Agent, Ev, Hooks, PermissionAnswer, PermissionRequest, PlanAnswer, Question, QuestionAnswer, Shared, Steer};
use crate::api::{Content, Message, ModelInfo};
use crate::attach;
use crate::config::{self, Config, EFFORTS, MODES};
use crate::diff::{self, DiffLine, Kind};
use crate::dock::{Dock, Tab};
use crate::markdown::{self, Block};
use crate::session::{self, Session, SessionInfo, Totals};
use crate::theme;
use crate::tools::{self, Display, Todo, ToolResult};
use crate::update::Status;
use eframe::egui::{self, Color32, FontId, Key, Modifiers, RichText, Sense, Stroke};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

enum Kind_ {
    /// What the user typed, and the files they attached.
    User { text: String, files: Vec<String> },
    Assistant { text: String, blocks: Vec<Block>, dirty: bool, reasoning: String, show_reasoning: bool },
    Tool { name: String, label: String, progress: String, result: Option<ToolResult>, open: bool },
    Notice(String),
    Error(String),
}

struct Item {
    kind: Kind_,
    /// Height and width last time it was drawn, so items outside the viewport can be skipped.
    h: f32,
    w: f32,
    /// On the first call of a folded run of tool calls: whether the run is expanded.
    fold_open: bool,
}

impl Item {
    fn new(kind: Kind_) -> Self {
        Item { kind, h: 0.0, w: 0.0, fold_open: false }
    }
}

/// A run of two or more tool calls the model has moved on from, shown under one header.
struct ToolGroup {
    start: usize,
    end: usize,
    open: bool,
    count: usize,
    /// What the run did, e.g. "Read 3, Run 2".
    summary: String,
    failed: usize,
}

/// The runs of tool calls to fold: those followed by another kind of item, or ending a turn that has finished.
fn tool_groups(items: &[Item], running: bool) -> Vec<ToolGroup> {
    let is_tool = |i: &Item| matches!(i.kind, Kind_::Tool { .. });
    let mut groups = vec![];
    let mut i = 0;
    while i < items.len() {
        if !is_tool(&items[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < items.len() && is_tool(&items[i]) {
            i += 1;
        }
        if i - start < 2 || (i == items.len() && running) {
            continue;
        }
        let mut counts: Vec<(&str, usize)> = vec![];
        let mut failed = 0;
        for item in &items[start..i] {
            if let Kind_::Tool { name, result, .. } = &item.kind {
                let title = tool_title(name);
                match counts.iter_mut().find(|(t, _)| *t == title) {
                    Some(c) => c.1 += 1,
                    None => counts.push((title, 1)),
                }
                failed += result.as_ref().is_some_and(|r| r.is_error) as usize;
            }
        }
        let summary = counts.iter().map(|(t, n)| format!("{t} {n}")).collect::<Vec<_>>().join(", ");
        groups.push(ToolGroup { start, end: i, open: items[start].fold_open, count: i - start, summary, failed });
    }
    groups
}

struct PendingPermission {
    req: PermissionRequest,
    reply: Sender<PermissionAnswer>,
    feedback: String,
}

struct PendingPlan {
    blocks: Vec<Block>,
    reply: Sender<PlanAnswer>,
    feedback: String,
}

/// Questions from `ask_questions`, with what the user has picked and typed so far, one entry per question.
struct PendingQuestions {
    questions: Vec<Question>,
    picks: Vec<Vec<usize>>,
    other: Vec<String>,
    reply: Sender<QuestionAnswer>,
}

/// A message on its way: what was typed and the files attached to it.
struct Outgoing {
    text: String,
    files: Vec<String>,
}

/// A tab in the model menu's rail.
#[derive(Clone, Copy, PartialEq)]
enum ModelTab {
    Starred,
    ScoutCode,
    OpenRouter,
}

/// The @ file picker under the cursor: where the "@word" starts and ends (in chars) and the text after the @.
#[derive(Clone, PartialEq)]
struct Mention {
    start: usize,
    end: usize,
    query: String,
}

/// Ctrl+F in the conversation.
#[derive(Default)]
struct Find {
    open: bool,
    query: String,
    /// Items containing the query, and which of them is current.
    hits: Vec<usize>,
    at: usize,
    /// The query and item count the hits were found for.
    searched: Option<(String, usize)>,
    /// Scroll to the current hit on the next frame.
    jump: bool,
    focus: bool,
}

/// The sections of Settings.
#[derive(Clone, Copy, PartialEq)]
enum SettingsTab {
    Notifications,
    Usage,
    Account,
    Updates,
}

impl SettingsTab {
    const ALL: [SettingsTab; 4] = [SettingsTab::Notifications, SettingsTab::Usage, SettingsTab::Account, SettingsTab::Updates];

    fn title(self) -> &'static str {
        match self {
            SettingsTab::Notifications => "Notifications",
            SettingsTab::Usage => "Usage",
            SettingsTab::Account => "Account",
            SettingsTab::Updates => "Updates",
        }
    }

    /// What the sidebar search matches, lower case.
    fn keywords(self) -> &'static str {
        match self {
            SettingsTab::Notifications => "notifications alerts taskbar flash toast windows attention",
            SettingsTab::Usage => "usage tokens cost spend daily monthly limits balance banked reset chart history",
            SettingsTab::Account => "account sign in sign out login key username billing",
            SettingsTab::Updates => "updates version upgrade download restart release notes about",
        }
    }
}

/// Settings > Usage: the account's limits from the server, and what's typed into the admin forms.
struct UsagePanel {
    info: Option<crate::api::UsageInfo>,
    busy: bool,
    /// The last result to show: whether it went well, and what happened.
    message: Option<(bool, String)>,
    /// The user the admin forms change, and the search that finds them.
    user: String,
    search: String,
    mode: String,
    monthly: String,
    cli: String,
    give_everyone: bool,
    give_banked: bool,
    amount: String,
    expiry_days: String,
    reset_days: String,
    /// The everyone-wide button waiting for its second click.
    confirm: Option<&'static str>,
}

impl Default for UsagePanel {
    fn default() -> Self {
        UsagePanel {
            info: None,
            busy: false,
            message: None,
            user: String::new(),
            search: String::new(),
            mode: "default".into(),
            monthly: String::new(),
            cli: String::new(),
            give_everyone: false,
            give_banked: false,
            amount: String::new(),
            expiry_days: "30".into(),
            reset_days: "30".into(),
            confirm: None,
        }
    }
}

impl UsagePanel {
    /// Puts the chosen user's saved limits in the form.
    fn fill_limits(&mut self) {
        let Some(u) = self.info.as_ref().and_then(|i| i.users.iter().find(|u| u.username == self.user)) else { return };
        self.mode = u.mode.clone();
        self.monthly = u.limits.as_ref().and_then(|l| l.monthly).map(|m| crate::api::money(m)).unwrap_or_default();
        self.cli = u.cli_limit.map(|c| crate::api::money(c)).unwrap_or_default();
    }
}

/// A USD amount as typed ("1.5", "$2"), or None when it isn't one.
fn parse_usd(s: &str) -> Option<f64> {
    s.trim().trim_start_matches('$').parse::<f64>().ok().filter(|n| n.is_finite() && *n >= 0.0)
}

/// "$0.50", "$12.00", "$0.0015".
fn usd(v: f64) -> String {
    let cents = format!("{v:.2}");
    if cents.parse::<f64>().is_ok_and(|c| (c - v).abs() < 1e-9) { format!("${cents}") } else { format!("${}", crate::api::money(v)) }
}

#[derive(Default)]
struct Login {
    key: String,
    /// The OpenRouter key being typed, and the name it's saved under.
    or_key: String,
    or_name: String,
    busy: bool,
    url: Option<String>,
    error: Option<String>,
}

pub struct App {
    cfg: Config,
    shared: Arc<Shared>,
    /// None while a worker thread owns it.
    agent: Option<Agent>,
    hooks: Hooks,
    rx: Receiver<Ev>,
    items: Vec<Item>,
    /// Index of the assistant item being streamed into.
    streaming: Option<usize>,
    partial: String,
    tool_index: HashMap<String, usize>,
    input: String,
    history: Vec<String>,
    history_pos: Option<usize>,
    running: bool,
    stopping: bool,
    status: String,
    todos: Vec<Todo>,
    totals: Totals,
    context: u64,
    models: Vec<ModelInfo>,
    /// The model id to send, for `provider` ("scoutcode" or "openrouter").
    model: String,
    provider: String,
    /// The model menu's tab: starred models, or one provider's. Picking a row in a provider's tab runs it there.
    model_tab: ModelTab,
    effort: String,
    permission: Option<PendingPermission>,
    plan: Option<PendingPlan>,
    questions: Option<PendingQuestions>,
    /// Every saved chat, from every folder; the sidebar groups them by project.
    sessions: Vec<SessionInfo>,
    projects: Vec<config::Project>,
    /// Paths of the projects whose chats are showing in the sidebar.
    expanded: HashSet<String>,
    /// The project being renamed, and the name typed so far.
    renaming: Option<(usize, String)>,
    /// The catchphrase typed out on the empty screen, and when it started typing (egui time). A new one is picked for
    /// each new chat.
    splash: usize,
    splash_start: Option<f64>,
    starred: Vec<String>,
    model_search: String,
    model_menu_open: bool,
    sidebar: bool,
    /// The panel on the right: terminal, changes and files.
    dock: Dock,
    login: Login,
    focus_input: bool,
    /// The open chat's folder, also while a worker has the agent.
    cwd: String,
    /// Files attached to the message being typed, as shown: relative inside the project, absolute outside it.
    attachments: Vec<String>,
    /// Messages sent while Scout worked, sent in order once it finishes.
    queue: Vec<Outgoing>,
    mention: Option<Mention>,
    /// The start of an @word whose picker was closed with Esc, so it stays closed.
    mention_closed: Option<usize>,
    mention_pick: usize,
    /// The project's files for the picker, which folder they're from, and when they were listed.
    file_index: Option<(String, Vec<String>, std::time::Instant)>,
    indexing: bool,
    /// Sidebar search: the query, the query last sent to the search thread, and its results.
    chat_search: String,
    chat_search_sent: String,
    chat_search_busy: bool,
    chat_hits: Option<(String, HashSet<String>)>,
    /// The chat whose deletion is being confirmed: id and title.
    deleting: Option<(String, String)>,
    find: Find,
    /// The message being edited: its item and the new text.
    editing: Option<(usize, String)>,
    settings_open: bool,
    settings_tab: SettingsTab,
    settings_search: String,
    /// Tokens per day, read when Settings opens.
    usage_log: crate::usage::Log,
    usage: UsagePanel,
    /// The first-run choice of notifications (flash, toast) while it's being made.
    notify_choice: (bool, bool),
    voice: Voice,
    updater: crate::update::Updater,
}

/// The mic button's state.
enum Voice {
    Idle,
    Recording(crate::voice::Recording),
    /// Downloading the model or transcribing: what it's doing.
    Busy(&'static str),
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

impl App {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        theme::install(&cc.egui_ctx);
        let cfg = config::load_config();
        let (tx, rx) = channel();
        let hooks = Hooks { tx, ctx: cc.egui_ctx.clone() };
        let shared = Arc::new(Shared {
            mode: Mutex::new(if MODES.contains(&cfg.mode.as_str()) { cfg.mode.clone() } else { "default".into() }),
            model: Mutex::new(cfg.model.clone()),
            provider: Mutex::new(cfg.provider.clone()),
            effort: Mutex::new(cfg.effort.clone()),
            cancel: Default::default(),
            edit_tools: Default::default(),
            steer: Default::default(),
        });
        let cwd = initial_cwd();
        let agent = Agent::new(cfg.clone(), session::new_session(&cwd, &cfg.model), shared.clone());
        let updater = crate::update::Updater::start(&cc.egui_ctx, cfg.auto_update);
        let mut app = App {
            model: cfg.model.clone(),
            provider: cfg.provider.clone(),
            model_tab: ModelTab::ScoutCode,
            effort: cfg.effort.clone(),
            projects: cfg.projects.clone(),
            starred: cfg.starred_models.clone(),
            cfg,
            shared,
            agent: Some(agent),
            hooks,
            rx,
            items: vec![],
            streaming: None,
            partial: String::new(),
            tool_index: HashMap::new(),
            input: String::new(),
            history: vec![],
            history_pos: None,
            running: false,
            stopping: false,
            status: String::new(),
            todos: vec![],
            totals: Totals::default(),
            context: 0,
            models: vec![],
            permission: None,
            plan: None,
            questions: None,
            sessions: session::list_sessions(),
            expanded: HashSet::new(),
            renaming: None,
            splash: next_splash(usize::MAX),
            splash_start: None,
            model_search: String::new(),
            model_menu_open: false,
            sidebar: true,
            dock: Dock::new(),
            login: Login::default(),
            focus_input: true,
            cwd: cwd.clone(),
            attachments: vec![],
            queue: vec![],
            mention: None,
            mention_closed: None,
            mention_pick: 0,
            file_index: None,
            indexing: false,
            chat_search: String::new(),
            chat_search_sent: String::new(),
            chat_search_busy: false,
            chat_hits: None,
            deleting: None,
            find: Find::default(),
            editing: None,
            settings_open: false,
            settings_tab: SettingsTab::Notifications,
            settings_search: String::new(),
            usage_log: Default::default(),
            usage: Default::default(),
            notify_choice: (true, false),
            voice: Voice::Idle,
            updater,
        };
        app.enter_project(&cwd);
        app.fetch_models();
        // `scout-desktop [folder] --resume <id>` reopens a saved conversation, including one from the CLI.
        let args: Vec<String> = std::env::args().collect();
        if let Some(s) = args.iter().position(|a| a == "--resume").and_then(|i| args.get(i + 1)).and_then(|id| session::load_session(id)) {
            app.open_session(s);
        }
        app
    }

    fn cwd(&self) -> String {
        self.cwd.clone()
    }

    /// Loads the ScoutCode and OpenRouter lists, whichever keys are saved, and merges them.
    fn fetch_models(&self) {
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
            let asked = cfg.api_key.is_some() as usize + (!cfg.openrouter_keys.is_empty()) as usize;
            if asked > 0 && errors.len() == asked {
                h.send(Ev::Models(Err(errors.join("; "))));
            } else {
                h.send(Ev::Models(Ok(crate::api::merge_models(scout, openrouter))));
            }
        });
    }

    // ---------- events from the worker ----------

    fn handle(&mut self, ev: Ev) {
        match ev {
            Ev::RequestStart => {
                self.streaming = None;
                self.partial.clear();
                self.status.clear();
            }
            Ev::Text(t) => {
                self.partial.push_str(&t);
                if let Kind_::Assistant { text, dirty, .. } = &mut self.assistant_item().kind {
                    text.push_str(&t);
                    *dirty = true;
                }
            }
            Ev::Reasoning(t) => {
                if let Kind_::Assistant { reasoning, .. } = &mut self.assistant_item().kind {
                    reasoning.push_str(&t);
                }
            }
            Ev::AssistantDone => {
                self.streaming = None;
                self.partial.clear();
            }
            Ev::ToolCall { id, name, label } => {
                self.streaming = None;
                let open = matches!(name.as_str(), "write_file" | "edit_file" | "multi_edit" | "apply_patch" | "todo_write");
                self.tool_index.insert(id.clone(), self.items.len());
                self.items.push(Item::new(Kind_::Tool { name, label, progress: String::new(), result: None, open }));
            }
            Ev::ToolProgress { id, text } => {
                if let Some(Kind_::Tool { progress, .. }) = self.tool_index.get(&id).and_then(|&i| self.items.get_mut(i)).map(|i| &mut i.kind) {
                    *progress = text;
                }
            }
            Ev::ToolResult { id, result } => {
                if let Some(item) = self.tool_index.get(&id).and_then(|&i| self.items.get_mut(i)) {
                    if let Kind_::Tool { name, result: r, .. } = &mut item.kind {
                        if matches!(name.as_str(), "write_file" | "edit_file" | "multi_edit" | "apply_patch" | "bash") {
                            self.dock.files_changed();
                        }
                        *r = Some(result);
                    }
                    item.w = 0.0;
                }
            }
            Ev::Status(s) => self.status = s,
            Ev::Notice(s) => self.items.push(Item::new(Kind_::Notice(s))),
            Ev::Todos(t) => self.todos = t,
            Ev::Totals(t, c) => {
                self.totals = t;
                self.context = c;
            }
            Ev::AskPermission(req, reply) => {
                let verb = if req.tool == "bash" { "run a command" } else { "change a file" };
                self.alert(&format!("Scout wants to {verb}"), req.label.lines().next().unwrap_or(""));
                self.permission = Some(PendingPermission { req, reply, feedback: String::new() });
            }
            Ev::AskPlan(plan, reply) => {
                self.alert("Scout's plan is ready", &self.chat_title());
                self.plan = Some(PendingPlan { blocks: markdown::parse(&plan), reply, feedback: String::new() });
            }
            Ev::AskQuestions(questions, reply) => {
                self.alert("Scout has a question", &self.chat_title());
                let n = questions.len();
                self.questions = Some(PendingQuestions { questions, picks: vec![vec![]; n], other: vec![String::new(); n], reply });
            }
            Ev::Steered(msgs) => {
                self.streaming = None;
                for (text, files) in msgs {
                    self.items.push(Item::new(Kind_::User { text, files }));
                }
            }
            Ev::FileIndex(cwd, files) => {
                self.indexing = false;
                self.file_index = Some((cwd, files, std::time::Instant::now()));
            }
            Ev::ChatSearch(q, ids) => {
                self.chat_search_busy = false;
                self.chat_hits = Some((q, ids.into_iter().collect()));
            }
            Ev::Finished(mut agent, result) => {
                let stopped = self.stopping || matches!(&result, Err(e) if e == "Interrupted");
                let error = result.as_ref().err().filter(|_| !stopped).cloned();
                if let Err(e) = result {
                    if stopped {
                        agent.keep_partial(&self.partial);
                        self.items.push(Item::new(Kind_::Notice("Interrupted.".into())));
                    } else {
                        self.items.push(Item::new(Kind_::Error(e)));
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
                self.focus_input = true;
                self.dock.files_changed();
                // The queue carries on unless the user stopped Scout, who then decides what's next.
                if !stopped && !self.queue.is_empty() {
                    let next = self.queue.remove(0);
                    self.submit(next);
                } else if let Some(e) = error {
                    self.alert("Scout stopped with an error", &e);
                } else if !stopped {
                    self.alert("Scout is done", &self.chat_title());
                }
            }
            Ev::Models(Ok(m)) => {
                // The agent picks each model's edit tools from these, under the id it sends on either provider.
                *self.shared.edit_tools.lock().unwrap() = m.iter().flat_map(|m| std::iter::once(m.id.clone()).chain(m.openrouter_id.clone()).map(move |id| (id, m.edit_tool))).collect();
                self.models = m;
            }
            Ev::Models(Err(e)) => self.status = format!("Could not load models: {e}"),
            Ev::LoginUrl(u) => self.login.url = Some(u),
            Ev::User(u) => self.set_user(Some(u)),
            Ev::Usage(r, said) => {
                self.usage.busy = false;
                match r {
                    Ok(info) => {
                        if self.usage.user.is_empty() {
                            self.usage.user = info.username.clone();
                        }
                        self.usage.info = Some(info);
                        self.usage.fill_limits();
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
            Ev::Voice(crate::voice::Update::Busy(s)) => self.voice = Voice::Busy(s),
            Ev::Voice(crate::voice::Update::Done(r)) => {
                self.voice = Voice::Idle;
                match r {
                    Ok(text) if text.is_empty() => self.status = "Didn't catch anything. Try again a little closer to the mic.".into(),
                    Ok(text) => {
                        self.status.clear();
                        if !self.input.is_empty() && !self.input.ends_with(char::is_whitespace) {
                            self.input.push(' ');
                        }
                        self.input.push_str(&text);
                        self.focus_input = true;
                    }
                    Err(e) => self.status = e,
                }
            }
        }
    }

    /// The mic button: starts a recording, or stops it and transcribes what was said into the box.
    fn toggle_voice(&mut self) {
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

    fn assistant_item(&mut self) -> &mut Item {
        if self.streaming.is_none() {
            self.streaming = Some(self.items.len());
            self.items.push(Item::new(Kind_::Assistant { text: String::new(), blocks: vec![], dirty: false, reasoning: String::new(), show_reasoning: false }));
        }
        &mut self.items[self.streaming.unwrap()]
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
        if let Some(a) = &mut self.agent {
            a.cfg.take_keys(&self.cfg);
        }
        self.fetch_models();
    }

    /// Adds a key at the end of the order (so it's tried last). An empty name gets a number.
    fn add_openrouter_key(&mut self, name: String, key: String) {
        let mut keys = self.cfg.openrouter_keys.clone();
        let name = if name.is_empty() { format!("Key {}", keys.len() + 1) } else { name };
        keys.push(config::OpenRouterKey { name, key });
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
        if let Some(a) = &mut self.agent {
            a.cfg.take_keys(&self.cfg);
        }
        self.fetch_models();
    }

    /// Makes `key` (a model's id in the list) the one sent to `provider`, for the next message and the next launch.
    fn choose_model(&mut self, key: &str, provider: &str) {
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

    // ---------- actions ----------

    /// Sends what's typed. While Scout works it waits in the queue, or with `steer` goes into the running turn.
    fn send(&mut self, steer: bool) {
        let text = self.input.trim().to_string();
        if text.is_empty() && self.attachments.is_empty() {
            return;
        }
        if self.running && text.starts_with('/') {
            self.status = "Commands wait until Scout is done. Esc stops it.".into();
            return;
        }
        self.input.clear();
        self.history_pos = None;
        self.mention = None;
        if !text.is_empty() && self.history.last() != Some(&text) {
            self.history.push(text.clone());
        }
        let files = std::mem::take(&mut self.attachments);
        if self.running {
            let o = Outgoing { text, files };
            if steer { self.steer(o) } else { self.queue.push(o) }
            return;
        }
        if let Some(cmd) = text.strip_prefix('/').filter(|_| files.is_empty()) {
            let (name, arg) = cmd.split_once(' ').unwrap_or((cmd, ""));
            match name {
                "new" | "clear" => return self.new_chat(),
                "undo" => return self.undo(),
                "prune" => return self.prune(),
                "compact" => return self.compact(arg.trim().to_string()),
                _ => {}
            }
        }
        self.submit(Outgoing { text, files });
    }

    /// Starts a turn with the message.
    fn submit(&mut self, o: Outgoing) {
        let Some(agent) = &mut self.agent else { return };
        let (content, read) = attach::build(&self.cwd, &o.text, &o.files);
        agent.read_files.extend(read);
        self.items.push(Item::new(Kind_::User { text: o.text, files: o.files }));
        self.start(move |agent, h| agent.turn(content, h).map_err(|e| e.message));
    }

    /// Hands a message to the running turn, which adds it after the step it's on.
    fn steer(&mut self, o: Outgoing) {
        let (content, read) = attach::build(&self.cwd, &o.text, &o.files);
        self.shared.steer.lock().unwrap().push(Steer { text: o.text, files: o.files, content, read });
    }

    /// Flashes the taskbar and/or shows a Windows notification, as the user chose, when the window isn't in front.
    fn alert(&self, title: &str, body: &str) {
        let ctx = &self.hooks.ctx;
        if ctx.input(|i| i.viewport().focused).unwrap_or(true) {
            return;
        }
        if self.cfg.notify_flash.unwrap_or(true) {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
        }
        if self.cfg.notify_toast.unwrap_or(false) {
            crate::notify::toast(title, &body.chars().take(200).collect::<String>());
        }
    }

    fn attach_path(&mut self, abs: &std::path::Path) {
        let p = attach::display_path(&self.cwd, abs);
        if !self.attachments.contains(&p) {
            self.attachments.push(p);
        }
        self.focus_input = true;
    }

    /// Opens the @ picker when the word before the cursor starts with @.
    fn update_mention(&mut self, focused: bool, cursor: Option<usize>) {
        let found = cursor.filter(|_| focused).and_then(|c| {
            let chars: Vec<char> = self.input.chars().collect();
            let c = c.min(chars.len());
            let start = chars[..c].iter().rposition(|ch| ch.is_whitespace()).map_or(0, |i| i + 1);
            (chars.get(start) == Some(&'@') && start < c).then(|| Mention { start, end: c, query: chars[start + 1..c].iter().collect() })
        });
        if found.is_none() {
            self.mention_closed = None;
        }
        let found = found.filter(|m| self.mention_closed != Some(m.start));
        if found.as_ref().map(|m| &m.query) != self.mention.as_ref().map(|m| &m.query) {
            self.mention_pick = 0;
        }
        if found.is_some() {
            self.index_files();
        }
        self.mention = found;
    }

    /// Replaces the @word with the file's path and attaches the file.
    fn pick_mention(&mut self, ctx: &egui::Context, id: egui::Id, path: &str) {
        let Some(m) = self.mention.take() else { return };
        let chars: Vec<char> = self.input.chars().collect();
        let (start, end) = (m.start.min(chars.len()), m.end.min(chars.len()));
        let insert = format!("@{path} ");
        self.input = format!("{}{insert}{}", chars[..start].iter().collect::<String>(), chars[end..].iter().collect::<String>());
        if let Some(mut s) = egui::text_edit::TextEditState::load(ctx, id) {
            s.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(start + insert.chars().count()))));
            s.store(ctx, id);
        }
        if !self.attachments.iter().any(|a| a == path) {
            self.attachments.push(path.to_string());
        }
    }

    fn pick_files(&mut self) {
        if let Some(files) = rfd::FileDialog::new().set_directory(&self.cwd).pick_files() {
            for f in files {
                self.attach_path(&f);
            }
        }
    }

    /// Lists the project's files for the @ picker on a worker thread, at most every few seconds.
    fn index_files(&mut self) {
        let fresh = self.file_index.as_ref().is_some_and(|(cwd, _, at)| *cwd == self.cwd && at.elapsed().as_secs() < 10);
        if fresh || self.indexing {
            return;
        }
        self.indexing = true;
        let cwd = self.cwd.clone();
        let h = self.hooks.clone();
        std::thread::spawn(move || {
            let files = attach::index(&cwd, 50_000);
            h.send(Ev::FileIndex(cwd, files));
        });
    }

    /// Searches every saved chat for the sidebar query on a worker thread, one search at a time.
    fn search_chats(&mut self) {
        let q = self.chat_search.trim().to_string();
        if q.is_empty() || self.chat_search_busy || q == self.chat_search_sent {
            return;
        }
        self.chat_search_sent = q.clone();
        self.chat_search_busy = true;
        let h = self.hooks.clone();
        std::thread::spawn(move || {
            let ids = session::search_sessions(&q);
            h.send(Ev::ChatSearch(q, ids));
        });
    }

    fn delete_chat(&mut self, id: &str) {
        let current = self.agent.as_ref().is_some_and(|a| a.session.id == id);
        if let Err(e) = session::delete_session(id) {
            self.status = format!("Could not delete the chat: {e}");
            return;
        }
        if let Some((_, hits)) = &mut self.chat_hits {
            hits.remove(id);
        }
        self.sessions = session::list_sessions();
        if current {
            self.new_chat();
        }
    }

    /// Rewinds the chat to before item `n`, a message of the user's, restoring the files changed since, and sends
    /// `text` in its place with the same attachments.
    fn resend(&mut self, n: usize, text: String) {
        if self.running {
            return;
        }
        let Some(Kind_::User { text: old, files }) = self.items.get(n).map(|i| &i.kind) else { return };
        let (old, files) = (old.clone(), files.clone());
        // The saved message with the same text, counted from the end, since later ones may repeat it.
        let later = self.items[n + 1..].iter().filter(|i| matches!(&i.kind, Kind_::User { text, .. } if *text == old)).count();
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
            self.items.push(Item::new(Kind_::Notice("That message was summarised by compaction, so it can't be edited.".into())));
            return;
        };
        let (restored, all) = agent.rewind(at);
        let s = agent.session.clone();
        self.context = agent.context_tokens();
        self.items = rebuild(&s);
        self.tool_index.clear();
        self.todos.clear();
        self.dock.files_changed();
        let mut note = String::new();
        if !restored.is_empty() {
            note = format!("Restored {}.", restored.join(", "));
        }
        if !all {
            note.push_str(if note.is_empty() { "" } else { " " });
            note.push_str("Some files were changed before this chat was opened in this window, so they weren't restored.");
        }
        if !note.is_empty() {
            self.items.push(Item::new(Kind_::Notice(note)));
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

    fn stop(&mut self) {
        if !self.running {
            return;
        }
        self.stopping = true;
        self.shared.cancel.store(true, Ordering::Relaxed);
        // Dropping the reply channels unblocks a worker waiting on the user.
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

    fn new_chat(&mut self) {
        if self.running {
            return;
        }
        let cwd = self.cwd();
        self.open_session(session::new_session(&cwd, &self.model));
    }

    fn open_session(&mut self, s: Session) {
        if self.running {
            return;
        }
        self.items = rebuild(&s);
        if self.items.is_empty() {
            self.splash = next_splash(self.splash);
            self.splash_start = None;
        }
        self.tool_index.clear();
        self.todos.clear();
        self.queue.clear();
        self.editing = None;
        self.find.searched = None;
        self.totals = s.totals.clone();
        self.sessions = session::list_sessions();
        if !session::same_dir(&self.cwd, &s.cwd) {
            // Attached paths are relative to the old folder.
            self.attachments.clear();
        }
        self.cwd = s.cwd.clone();
        self.enter_project(&s.cwd);
        let mut agent = Agent::new(self.cfg.clone(), s, self.shared.clone());
        agent.last_prompt_tokens = 0;
        self.context = agent.context_tokens();
        self.agent = Some(agent);
        self.status.clear();
        self.focus_input = true;
    }

    /// Starts a new chat in the folder.
    fn change_project(&mut self, dir: String) {
        self.open_session(session::new_session(&dir, &self.model));
    }

    // ---------- projects: named folders that chats belong to (by the folder they ran in) ----------

    fn project_index(&self, dir: &str) -> Option<usize> {
        self.projects.iter().position(|p| session::same_dir(&p.path, dir))
    }

    /// The current folder's project name.
    fn project_label(&self) -> String {
        let cwd = self.cwd();
        self.project_index(&cwd).map(|i| self.projects[i].name.clone()).unwrap_or_else(|| project_name(&cwd))
    }

    fn save_projects(&self) {
        config::save_one("desktopProjects", json!(self.projects));
    }

    /// Every folder Scout works in is a project; the first visit names it after the folder.
    /// Also remembers it as the folder to open next launch, and shows its chats.
    fn enter_project(&mut self, dir: &str) {
        let i = self.project_index(dir).unwrap_or_else(|| {
            self.projects.push(config::Project { name: project_name(dir), path: dir.to_string() });
            self.save_projects();
            self.projects.len() - 1
        });
        self.expanded.insert(self.projects[i].path.clone());
        config::save_one("desktopProject", json!(dir));
    }

    /// Picks a folder, adds it as a project, starts a chat there and lets the user name it.
    fn add_project(&mut self) {
        if self.running {
            return;
        }
        let Some(dir) = rfd::FileDialog::new().set_directory(self.cwd()).pick_folder() else { return };
        let dir = dir.to_string_lossy().to_string();
        let is_new = self.project_index(&dir).is_none();
        self.change_project(dir.clone());
        if let Some(i) = self.project_index(&dir).filter(|_| is_new) {
            self.renaming = Some((i, self.projects[i].name.clone()));
        }
    }

    fn undo(&mut self) {
        let Some(agent) = &mut self.agent else { return };
        let msg = match agent.undo() {
            None => "Nothing to undo.".to_string(),
            Some((label, files, conv)) => {
                let s = agent.session.clone();
                self.items = rebuild(&s);
                format!(
                    "Undid \"{}\"{}{}",
                    label.chars().take(60).collect::<String>(),
                    if conv { "" } else { " (files only: the conversation was compacted since)" },
                    if files.is_empty() { String::new() } else { format!("; restored {}", files.join(", ")) }
                )
            }
        };
        self.dock.files_changed();
        self.items.push(Item::new(Kind_::Notice(msg)));
    }

    fn prune(&mut self) {
        let Some(agent) = &mut self.agent else { return };
        let msg = match agent.prune(0, 0) {
            Some((n, freed)) => format!("Cleared {n} old tool output{} (~{}k tokens).", if n == 1 { "" } else { "s" }, freed / 1000),
            None => "No old tool output to clear.".into(),
        };
        self.context = agent.context_tokens();
        self.items.push(Item::new(Kind_::Notice(msg)));
    }

    fn compact(&mut self, focus: String) {
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

    fn answer_permission(&mut self, a: PermissionAnswer) {
        if let Some(p) = self.permission.take() {
            let _ = p.reply.send(a);
        }
    }

    fn answer_plan(&mut self, a: PlanAnswer) {
        if let Some(p) = self.plan.take() {
            let _ = p.reply.send(a);
        }
    }

    fn answer_questions(&mut self, a: QuestionAnswer) {
        if let Some(p) = self.questions.take() {
            let _ = p.reply.send(a);
        }
    }

    // ---------- drawing ----------
    /// The first thing asked in this chat, as its title.
    fn chat_title(&self) -> String {
        let first = self.items.iter().find_map(|i| if let Kind_::User { text, .. } = &i.kind { Some(text.lines().next().unwrap_or("").trim().to_string()) } else { None });
        first.filter(|t| !t.is_empty()).unwrap_or_else(|| "New chat".into())
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.set_height(32.0);
        // The whole bar, margins included, doubles as the window's title bar.
        let r = ui.max_rect();
        drag_region(ui, egui::Rect::from_min_max(r.min - egui::vec2(16.0, 5.0), r.max + egui::vec2(14.0, 13.0)));
        ui.horizontal_centered(|ui| {
            if !self.sidebar {
                if theme::icon_button(ui, 28.0, "Show sidebar", theme::icon::sidebar).clicked() {
                    self.sidebar = true;
                }
            }
            let title = self.chat_title();
            let cwd = self.cwd();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Room for the window buttons, which sit over the bar's right end.
                ui.add_space(3.0 * WIN_BTN.x - 2.0);
                // The right panel's tabs, then its toggle; right to left, so they read toggle, terminal, changes, files.
                ui.spacing_mut().item_spacing.x = 2.0;
                for tab in [Tab::Files, Tab::Changes, Tab::Terminal] {
                    let tip = format!("{}{}", tab.title(), if tab == Tab::Terminal { " (Ctrl+`)" } else { "" });
                    if bar_button(ui, self.dock.open && self.dock.tab == tab, &tip, tab.icon()).clicked() {
                        self.dock.toggle(tab);
                    }
                }
                let tip = if self.dock.open { "Hide panel" } else { "Show panel" };
                if bar_button(ui, self.dock.open, tip, theme::icon::panel_right).clicked() {
                    self.dock.open = !self.dock.open;
                }
                ui.add_space(8.0);
                self.update_pill(ui);
                // The sidebar shows the folder; without it, the bar does.
                if !self.sidebar {
                    let r = chip(ui, &self.project_label(), theme::MUTED, Some(&theme::icon::folder), !self.running).on_hover_text(format!("{cwd}\nClick to add another project folder"));
                    if r.clicked() {
                        self.add_project();
                    }
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(title).font(theme::bold(14.0)).color(theme::STRONG)).truncate());
                });
            });
        });
    }

    /// Shows in the title bar once an update is downloaded and waiting.
    fn update_pill(&mut self, ui: &mut egui::Ui) {
        let Status::Ready { version, .. } = self.updater.status() else { return };
        let tip = match self.restart_blocker() {
            Some(why) => why.to_string(),
            None => format!("Scout {version} is ready. Your chat reopens after the restart."),
        };
        let pill = egui::Button::new(RichText::new("Restart to update").font(theme::bold(12.5)).color(theme::ACCENT_INK)).fill(theme::ACCENT).corner_radius(7);
        let enabled = self.restart_blocker().is_none();
        if ui.add_enabled(enabled, pill).on_hover_text(tip.as_str()).on_disabled_hover_text(tip.as_str()).clicked() {
            self.restart_to_update(ui.ctx());
        }
        ui.add_space(8.0);
    }

    /// Why restarting now would lose something, if it would.
    fn restart_blocker(&self) -> Option<&'static str> {
        if self.running {
            Some("Scout is working. You can restart when it finishes.")
        } else if self.dock.has_unsaved_edits() {
            Some("Save the file open in the editor first.")
        } else {
            None
        }
    }

    /// Starts the new build on the same folder and chat, then closes this window.
    fn restart_to_update(&mut self, ctx: &egui::Context) {
        if self.restart_blocker().is_some() {
            return;
        }
        let mut args = vec![self.cwd()];
        if let Some(agent) = &self.agent {
            args.extend(["--resume".to_string(), agent.session.id.clone()]);
        }
        match self.updater.restart(&args) {
            Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(e) => self.status = e,
        }
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        // The header strip is part of the title bar too.
        let r = ui.max_rect();
        drag_region(ui, egui::Rect::from_min_max(r.min - egui::vec2(10.0, 0.0), egui::pos2(r.right() + 10.0, r.top() + 50.0)));
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.set_height(30.0);
            ui.add_space(4.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), Sense::hover());
            theme::icon::logo(ui.painter(), rect);
            ui.add_space(2.0);
            ui.label(RichText::new("Scout").font(theme::bold(16.0)).color(theme::STRONG));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::icon_button(ui, 28.0, "Hide sidebar", theme::icon::sidebar).clicked() {
                    self.sidebar = false;
                }
            });
        });
        ui.add_space(12.0);

        let r = nav_button(ui, !self.running, "New chat", "Ctrl+N", &theme::icon::plus, true);
        if r.clicked() {
            self.new_chat();
        }
        ui.add_space(8.0);
        egui::Frame::NONE.fill(theme::SURFACE).stroke(Stroke::new(1.0, theme::BORDER)).corner_radius(9).inner_margin(egui::Margin { left: 8, right: 4, top: 0, bottom: 0 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 30.0), Sense::hover());
                theme::icon::search(ui.painter(), egui::Rect::from_center_size(rect.center(), egui::vec2(18.0, 18.0)), theme::DIM);
                let clear = !self.chat_search.is_empty();
                let w = ui.available_width() - if clear { 26.0 } else { 0.0 };
                // As tall as the magnifier's row, with the text centred on it.
                let r = ui.add(egui::TextEdit::singleline(&mut self.chat_search).hint_text(RichText::new("Search chats").color(theme::DIM)).frame(egui::Frame::NONE).desired_width(w).min_size(egui::vec2(w, 30.0)).align(egui::Align2::LEFT_CENTER).font(FontId::proportional(13.5)));
                if r.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.chat_search.clear();
                }
                if clear && theme::icon_button(ui, 24.0, "Clear", theme::icon::close).clicked() {
                    self.chat_search.clear();
                }
            });
        });
        if self.chat_search.trim().is_empty() {
            self.chat_search_sent.clear();
            self.chat_hits = None;
        } else {
            self.search_chats();
        }
        ui.add_space(14.0);
        if !self.chat_search.trim().is_empty() {
            self.search_results(ui);
            self.profile(ui);
            return;
        }
        ui.horizontal(|ui| {
            ui.set_height(24.0);
            ui.add_space(10.0);
            theme::section_label(ui, "Projects");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(4.0);
                if theme::icon_button(ui, 24.0, "Add a project folder", theme::icon::plus).clicked() {
                    self.add_project();
                }
            });
        });
        ui.add_space(4.0);
        let current = self.agent.as_ref().map(|a| a.session.id.clone());
        let cwd = self.cwd();
        let enabled = !self.running;
        let mut open = None;
        let mut new_chat_in = None;
        let mut remove = None;
        let mut commit_rename = false;
        egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(ui.available_height() - 56.0).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for (i, p) in self.projects.iter().enumerate() {
                let here = session::same_dir(&p.path, &cwd);
                if let Some((_, name)) = self.renaming.as_mut().filter(|(r, _)| *r == i) {
                    let r = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 7)).font(FontId::proportional(13.5)));
                    if !r.has_focus() && !r.lost_focus() {
                        r.request_focus();
                    }
                    if r.lost_focus() {
                        commit_rename = !ui.input(|i| i.key_pressed(Key::Escape));
                        if !commit_rename {
                            self.renaming = None;
                        }
                    }
                } else {
                    let expanded = self.expanded.contains(&p.path);
                    let (row, plus, more) = project_row(ui, here, expanded, enabled, &p.name);
                    let row = row.on_hover_text(p.path.as_str());
                    if plus.clicked() {
                        new_chat_in = Some(p.path.clone());
                    } else if row.clicked() {
                        if expanded {
                            self.expanded.remove(&p.path);
                        } else {
                            self.expanded.insert(p.path.clone());
                        }
                    }
                    egui::Popup::menu(&more).width(170.0).show(|ui| {
                        if menu_item(ui, false, "New chat").clicked() {
                            new_chat_in = Some(p.path.clone());
                        }
                        if menu_item(ui, false, "Rename").clicked() {
                            self.renaming = Some((i, p.name.clone()));
                        }
                        // The open project stays, so the chat on screen always has one.
                        if !here && menu_item(ui, false, "Remove from list").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if !self.expanded.contains(&p.path) {
                    continue;
                }
                let mut any = false;
                for s in self.sessions.iter().filter(|s| session::same_dir(&s.cwd, &p.path)) {
                    any = true;
                    let selected = current.as_deref() == Some(s.id.as_str());
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        let (row, more) = session_row(ui, selected, enabled, &s.title, &session::ago(s.updated));
                        if row.clicked() {
                            open = Some(s.id.clone());
                        }
                        egui::Popup::menu(&more).width(150.0).show(|ui| {
                            // The chat Scout is working in stays until it's done.
                            let busy = selected && self.running;
                            if ui.add_enabled_ui(!busy, |ui| menu_item(ui, false, "Delete…")).inner.clicked() {
                                self.deleting = Some((s.id.clone(), s.title.clone()));
                            }
                        });
                    });
                }
                if !any {
                    ui.horizontal(|ui| {
                        ui.set_height(28.0);
                        ui.add_space(26.0);
                        ui.label(RichText::new("No chats yet").size(12.5).color(theme::DIM));
                    });
                }
                ui.add_space(4.0);
            }
        });
        if commit_rename {
            if let Some((i, name)) = self.renaming.take() {
                let name = name.trim();
                if !name.is_empty() {
                    self.projects[i].name = name.to_string();
                    self.save_projects();
                }
            }
        }
        if let Some(i) = remove {
            self.expanded.remove(&self.projects[i].path);
            self.projects.remove(i);
            self.renaming = None;
            self.save_projects();
        }
        if let Some(dir) = new_chat_in {
            self.change_project(dir);
        } else if let Some(s) = open.and_then(|id| session::load_session(&id)) {
            self.open_session(s);
        }
        self.profile(ui);
    }

    /// Chats matching the sidebar search, across every project: titles at once, then text once the search ends.
    fn search_results(&mut self, ui: &mut egui::Ui) {
        let q = self.chat_search.trim().to_lowercase();
        let hits = self.chat_hits.as_ref().filter(|(hq, _)| hq.to_lowercase() == q).map(|(_, h)| h);
        let current = self.agent.as_ref().map(|a| a.session.id.clone());
        let enabled = !self.running;
        let running = self.running;
        let mut open = None;
        let mut delete = None;
        // Each match with its project's name, for the tooltip.
        let rows: Vec<(SessionInfo, String)> = self
            .sessions
            .iter()
            .filter(|s| hits.map_or(s.title.to_lowercase().contains(&q), |h| h.contains(&s.id)))
            .map(|s| (s.clone(), self.project_index(&s.cwd).map(|i| self.projects[i].name.clone()).unwrap_or_else(|| project_name(&s.cwd))))
            .collect();
        let searching = hits.is_none();
        ui.horizontal(|ui| {
            ui.set_height(24.0);
            ui.add_space(10.0);
            theme::section_label(ui, "Results");
            if searching {
                ui.add(egui::Spinner::new().size(11.0).color(theme::DIM));
            }
        });
        ui.add_space(4.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(ui.available_height() - 56.0).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for (s, project) in &rows {
                let selected = current.as_deref() == Some(s.id.as_str());
                let (row, more) = session_row(ui, selected, enabled, &s.title, &session::ago(s.updated));
                if row.on_hover_text(project).clicked() {
                    open = Some(s.id.clone());
                }
                egui::Popup::menu(&more).width(150.0).show(|ui| {
                    if ui.add_enabled_ui(!(selected && running), |ui| menu_item(ui, false, "Delete…")).inner.clicked() {
                        delete = Some((s.id.clone(), s.title.clone()));
                    }
                });
            }
            if rows.is_empty() && !searching {
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(RichText::new("No chats match.").size(12.5).color(theme::DIM));
                });
            }
        });
        if delete.is_some() {
            self.deleting = delete;
        }
        if let Some(s) = open.and_then(|id| session::load_session(&id)) {
            self.open_session(s);
        }
    }

    /// The account row at the foot of the sidebar and its menu.
    fn profile(&mut self, ui: &mut egui::Ui) {
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(10.0);
            let browser = config::is_sign_in(self.cfg.api_key.as_deref());
            let name = self.cfg.username.clone().unwrap_or_else(|| if browser { "Signed in" } else if self.cfg.api_key.is_none() { "OpenRouter key" } else { "API key" }.to_string());
            let r = profile_row(ui, &name);
            egui::Popup::menu(&r).width(r.rect.width()).align(egui::RectAlign::TOP_START).show(|ui| {
                if menu_icon_item(ui, "Settings", theme::icon::gear).clicked() {
                    self.open_settings(SettingsTab::Notifications);
                }
                if menu_icon_item(ui, "Usage", theme::icon::chart).clicked() {
                    self.open_settings(SettingsTab::Usage);
                }
                if menu_icon_item(ui, "Check for updates", theme::icon::download).clicked() {
                    self.open_settings(SettingsTab::Updates);
                    self.updater.check_now();
                }
                if menu_icon_item(ui, "Account page", theme::icon::user).clicked() {
                    let site = self.cfg.base_url.trim_end_matches("/v1");
                    ui.ctx().open_url(egui::OpenUrl::new_tab(format!("{site}/account")));
                }
            });
            ui.add_space(4.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
            ui.painter().rect_filled(rect, 0, theme::BORDER);
            ui.add_space(6.0);
        });
    }

    /// The permission mode, as a menu in the footer (Shift+Tab cycles it too).
    fn mode_chip(&mut self, ui: &mut egui::Ui) {
        let r = chip(ui, config::mode_label(&self.shared.mode()), theme::MUTED, None, true).on_hover_text("Permission mode (Shift+Tab)");
        egui::Popup::menu(&r).width(200.0).show(|ui| {
            for m in MODES {
                let selected = self.shared.mode() == m;
                if menu_item(ui, selected, config::mode_label(m)).clicked() {
                    *self.shared.mode.lock().unwrap() = m.to_string();
                }
            }
        });
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.set_height(18.0);
            ui.spacing_mut().item_spacing.x = 6.0;
            let small = |s: String| RichText::new(s).size(11.5).color(theme::DIM);
            ui.add_space(6.0);
            self.mode_chip(ui);
            if self.running {
                ui.add(egui::Spinner::new().size(11.0).color(theme::ACCENT));
                let waiting = self.permission.is_some() || self.plan.is_some() || self.questions.is_some();
                let s = if self.status.is_empty() { if waiting { "Waiting for you" } else { "Working…" }.to_string() } else { self.status.clone() };
                ui.label(RichText::new(s).size(11.5).color(theme::MUTED));
                let typing = !self.input.trim().is_empty() || !self.attachments.is_empty();
                ui.label(small(if typing { "· Enter to queue · Ctrl+Enter to send now" } else { "· Esc to stop" }.into()));
            } else if !self.status.is_empty() {
                ui.label(small(self.status.clone()));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                ui.spacing_mut().item_spacing.x = 10.0;
                // Share of the funds this session has spent. Stays grey until the server reports a balance.
                let spent = self.totals.cost;
                let frac = match self.totals.balance {
                    Some(b) if b.is_finite() && b >= 0.0 && spent + b > 0.0 => (spent / (spent + b)).clamp(0.0, 1.0) as f32,
                    _ => 0.0,
                };
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), Sense::click());
                let c = rect.center();
                let r = 6.0;
                let p = ui.painter();
                p.circle_stroke(c, r, Stroke::new(2.5, if resp.hovered() { theme::MUTED } else { theme::FAINT }));
                if frac > 0.0 {
                    let n = ((frac * 48.0).ceil() as usize).max(2);
                    let pts: Vec<egui::Pos2> = (0..=n)
                        .map(|i| {
                            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * frac * i as f32 / n as f32;
                            c + egui::vec2(a.cos(), a.sin()) * r
                        })
                        .collect();
                    p.add(egui::Shape::line(pts, Stroke::new(2.5, theme::USAGE)));
                }
                egui::Popup::menu(&resp).width(260.0).align(egui::RectAlign::TOP_END).show(|ui| {
                    ui.label(RichText::new("Usage this session").font(theme::bold(13.0)).color(theme::STRONG));
                    ui.add_space(6.0);
                    let row = |ui: &mut egui::Ui, k: &str, v: String| {
                        ui.label(RichText::new(k).size(12.0).color(theme::DIM));
                        ui.label(RichText::new(v).size(12.5).color(theme::TEXT));
                        ui.end_row();
                    };
                    egui::Grid::new("usage_grid").num_columns(2).spacing([18.0, 6.0]).show(ui, |ui| {
                        row(ui, "Cost", format!("${:.4}", self.totals.cost));
                        row(ui, "Requests", self.totals.requests.to_string());
                        row(ui, "Prompt tokens", format!("{} ({} cached)", self.totals.prompt_tokens, self.totals.cached_tokens));
                        row(ui, "Completion tokens", self.totals.completion_tokens.to_string());
                        let limit = self.cfg.auto_compact_tokens.max(1);
                        let ctx_frac = (self.context as f64 / limit as f64 * 100.0).min(100.0);
                        row(ui, "Context", format!("{:.1}k tokens · {ctx_frac:.0}% of auto-compact", self.context as f64 / 1000.0));
                    });
                });
                if let Some(b) = self.totals.balance {
                    ui.label(small(format!("balance ${b:.2}")));
                }
                self.effort_chip(ui);
                self.model_chip(ui);
            });
        });
    }

    fn todos(&mut self, ui: &mut egui::Ui) {
        if self.todos.is_empty() || self.todos.iter().all(|t| t.status == "completed") && !self.running {
            return;
        }
        let done = self.todos.iter().filter(|t| t.status == "completed").count();
        let total = self.todos.len();
        egui::Frame::NONE.fill(theme::SURFACE).stroke(Stroke::new(1.0, theme::BORDER)).corner_radius(12).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Tasks").font(theme::bold(13.0)).color(theme::STRONG));
                ui.label(RichText::new(format!("{done} of {total}")).size(12.0).color(theme::DIM));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(80.0, 4.0), Sense::hover());
                    ui.painter().rect_filled(rect, 2, theme::SURFACE_HI);
                    ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * done as f32 / total.max(1) as f32, 4.0)), 2, theme::GREEN);
                });
            });
            ui.add_space(4.0);
            todo_list(ui, &self.todos);
        });
        ui.add_space(8.0);
    }

    fn permission_card(&mut self, ui: &mut egui::Ui) {
        // With text in the prompt, Ctrl+Enter sends it instead.
        let shortcut = self.input.trim().is_empty();
        let Some(p) = &mut self.permission else { return };
        let mut answer = None;
        card(ui, |ui| {
            let verb = match p.req.tool.as_str() {
                "bash" => "Run this command?",
                "write_file" => "Write this file?",
                _ => "Make this edit?",
            };
            card_header(ui, verb, "Ctrl+Enter to allow");
            ui.add_space(8.0);
            if let Some(r) = &p.req.reason {
                ui.label(RichText::new(format!("Held for review. {r}")).color(theme::RED));
                ui.add_space(6.0);
            }
            if p.req.tool == "bash" {
                markdown::code_block(ui, &p.req.label);
            } else {
                ui.label(RichText::new(&p.req.label).font(FontId::monospace(13.0)).color(theme::CODE));
            }
            if let Some(d) = &p.req.preview {
                ui.add_space(6.0);
                egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show(ui, |ui| show_display(ui, d));
            }
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if theme::primary_button(ui, "Allow").clicked() || shortcut && ui.input(|i| i.modifiers.command && i.key_pressed(Key::Enter)) {
                    answer = Some(PermissionAnswer::Allow);
                }
                let always = match (&p.req.prefix, p.req.tool.as_str()) {
                    (Some(pre), _) => Some(format!("Always allow `{pre}` here")),
                    (None, "bash") => None,
                    _ => Some("Allow all edits this session".into()),
                };
                if let Some(a) = always {
                    if theme::secondary_button(ui, &a.replace('`', "")).clicked() {
                        answer = Some(PermissionAnswer::Always);
                    }
                }
                ui.add(egui::TextEdit::singleline(&mut p.feedback).hint_text("or tell Scout what to do instead").desired_width(240.0).margin(egui::Margin::symmetric(10, 4)));
                if theme::ghost_button(ui, if p.feedback.trim().is_empty() { "Deny" } else { "Deny and send" }).clicked() {
                    answer = Some(PermissionAnswer::Deny(p.feedback.trim().to_string()));
                }
            });
        });
        if let Some(a) = answer {
            self.answer_permission(a);
        }
    }

    fn plan_card(&mut self, ui: &mut egui::Ui) {
        let Some(p) = &mut self.plan else { return };
        let mut answer = None;
        let max = ui.ctx().content_rect().height() * 0.5;
        card(ui, |ui| {
            card_header(ui, "Scout's plan", "Review before it starts");
            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(max).auto_shrink([false, true]).show(ui, |ui| markdown::show(ui, &p.blocks));
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if theme::primary_button(ui, "Approve").clicked() {
                    answer = Some(PlanAnswer::Approve("default".into()));
                }
                if theme::secondary_button(ui, "Approve, accept edits").clicked() {
                    answer = Some(PlanAnswer::Approve("acceptEdits".into()));
                }
                ui.add(egui::TextEdit::singleline(&mut p.feedback).hint_text("what to change").desired_width(220.0).margin(egui::Margin::symmetric(10, 4)));
                if theme::ghost_button(ui, "Keep planning").clicked() {
                    answer = Some(PlanAnswer::Revise(p.feedback.trim().to_string()));
                }
            });
        });
        if let Some(a) = answer {
            self.answer_plan(a);
        }
    }

    /// The questions from `ask_questions`: options to pick from, or a typed answer, for each question.
    fn questions_card(&mut self, ui: &mut egui::Ui) {
        let Some(p) = &mut self.questions else { return };
        let PendingQuestions { questions, picks, other, .. } = p;
        let answers: Vec<Option<String>> = questions
            .iter()
            .enumerate()
            .map(|(i, q)| {
                let mut parts: Vec<String> = picks[i].iter().filter_map(|&j| q.options.get(j)).map(|o| o.label.clone()).collect();
                let typed = other[i].trim();
                if !typed.is_empty() {
                    parts.push(typed.to_string());
                }
                (!parts.is_empty()).then(|| parts.join(", "))
            })
            .collect();
        let ready = answers.iter().all(Option::is_some);
        let mut answer = None;
        card(ui, |ui| {
            card_header(ui, if questions.len() == 1 { "Scout has a question" } else { "Scout has some questions" }, "Pick an option or type your own");
            ui.add_space(8.0);
            let max = ui.ctx().content_rect().height() * 0.5;
            egui::ScrollArea::vertical().max_height(max).auto_shrink([false, true]).show(ui, |ui| {
                for (i, q) in questions.iter().enumerate() {
                    if i > 0 {
                        ui.add_space(12.0);
                    }
                    if !q.header.is_empty() {
                        ui.label(RichText::new(q.header.to_uppercase()).size(11.0).color(theme::DIM));
                    }
                    ui.label(RichText::new(&q.question).size(theme::BODY).color(theme::STRONG));
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                        for (j, opt) in q.options.iter().enumerate() {
                            let on = picks[i].contains(&j);
                            let r = if on { theme::primary_button(ui, &opt.label) } else { theme::secondary_button(ui, &opt.label) };
                            let r = if opt.description.is_empty() { r } else { r.on_hover_text(&opt.description) };
                            if r.clicked() {
                                if on {
                                    picks[i].retain(|&k| k != j);
                                } else if q.multi_select {
                                    picks[i].push(j);
                                } else {
                                    picks[i] = vec![j];
                                }
                            }
                        }
                    });
                    ui.add_space(6.0);
                    ui.add(egui::TextEdit::singleline(&mut other[i]).hint_text("Or type your own answer").desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 6)));
                }
            });
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(ready, egui::Button::new("Send answers")).clicked() || ready && ui.input(|i| i.modifiers.command && i.key_pressed(Key::Enter)) {
                    answer = Some(QuestionAnswer::Answered(answers.iter().flatten().cloned().collect()));
                }
                if theme::ghost_button(ui, "Skip").clicked() {
                    answer = Some(QuestionAnswer::Skipped);
                }
            });
        });
        if let Some(a) = answer {
            self.answer_questions(a);
        }
    }

    /// Asked once, on the first run: how Scout should get the user's attention.
    fn notify_setup_card(&mut self, ui: &mut egui::Ui) {
        if self.cfg.notify_flash.is_some() || self.cfg.notify_toast.is_some() {
            return;
        }
        let mut save = false;
        card(ui, |ui| {
            card_header(ui, "How should Scout get your attention?", "Change it later in Settings");
            ui.add_space(4.0);
            ui.label(RichText::new("When Scout finishes or needs you while its window is in the background:").size(13.0).color(theme::MUTED));
            ui.add_space(6.0);
            ui.checkbox(&mut self.notify_choice.0, "Flash the taskbar button");
            ui.checkbox(&mut self.notify_choice.1, "Show a Windows notification");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                save = theme::primary_button(ui, "Save").clicked();
                if theme::ghost_button(ui, "Send a test").clicked() {
                    crate::notify::toast("Scout", "This is how Scout will tell you it's done.");
                }
            });
        });
        if save {
            let (flash, toast) = self.notify_choice;
            self.set_notify(flash, toast);
        }
    }

    fn set_notify(&mut self, flash: bool, toast: bool) {
        self.cfg.notify_flash = Some(flash);
        self.cfg.notify_toast = Some(toast);
        let mut m = serde_json::Map::new();
        m.insert("desktopFlash".into(), json!(flash));
        m.insert("desktopToast".into(), json!(toast));
        config::save_config(m);
    }

    fn open_settings(&mut self, tab: SettingsTab) {
        self.settings_open = true;
        self.settings_tab = tab;
        self.settings_search.clear();
        self.usage_log = crate::usage::load();
        self.usage_request(None);
    }

    /// Asks the server for the account's limits, after making a change when `change` says what to send and say.
    fn usage_request(&mut self, change: Option<(Value, String)>) {
        if self.cfg.api_key.is_none() || self.usage.busy {
            return;
        }
        self.usage.busy = true;
        self.usage.confirm = None;
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

    /// Settings: a large panel in the middle of the window, sections on the left and the open one on the right.
    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let screen = ctx.content_rect();
        let size = egui::vec2((screen.width() - 80.0).clamp(560.0, 1040.0), (screen.height() - 80.0).clamp(400.0, 720.0));
        let mut close = false;
        let frame = egui::Frame::NONE.fill(theme::BG).stroke(Stroke::new(1.0, theme::BORDER_HI)).corner_radius(14).shadow(egui::Shadow { offset: [0, 16], blur: 48, spread: 0, color: Color32::from_black_alpha(140) });
        let m = egui::Modal::new(egui::Id::unique("settings")).frame(frame).show(ctx, |ui| {
            ui.set_min_size(size);
            ui.set_max_size(size);
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
            // Title bar.
            ui.horizontal(|ui| {
                ui.set_height(52.0);
                ui.add_space(20.0);
                ui.label(RichText::new("Settings").font(theme::bold(16.0)).color(theme::STRONG));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(12.0);
                    close = theme::icon_button(ui, 30.0, "Close (Esc)", theme::icon::close).clicked();
                });
            });
            let (sep, _) = ui.allocate_exact_size(egui::vec2(size.x, 1.0), Sense::hover());
            ui.painter().rect_filled(sep, 0, theme::BORDER);
            let body = size.y - 53.0;
            ui.horizontal_top(|ui| {
                // Sections.
                egui::Frame::NONE.fill(theme::SIDEBAR).corner_radius(egui::CornerRadius { sw: 14, ..Default::default() }).inner_margin(egui::Margin::symmetric(14, 14)).show(ui, |ui| {
                    ui.set_width(220.0);
                    ui.set_height(body - 28.0);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    // This frame sits in a left-to-right row, so stack the search and the list in their own column.
                    ui.vertical(|ui| {
                        egui::Frame::NONE.fill(theme::SURFACE).stroke(Stroke::new(1.0, theme::BORDER)).corner_radius(9).inner_margin(egui::Margin { left: 8, right: 8, top: 0, bottom: 0 }).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 30.0), Sense::hover());
                                theme::icon::search(ui.painter(), egui::Rect::from_center_size(rect.center(), egui::vec2(18.0, 18.0)), theme::DIM);
                                ui.add_space(4.0);
                                ui.add(egui::TextEdit::singleline(&mut self.settings_search).hint_text(RichText::new("Search").color(theme::DIM)).frame(egui::Frame::NONE).desired_width(f32::INFINITY));
                            });
                        });
                        ui.add_space(14.0);
                        theme::section_label(ui, "Preferences");
                        ui.add_space(4.0);
                        let q = self.settings_search.trim().to_lowercase();
                        let mut shown = 0;
                        for tab in SettingsTab::ALL {
                            // Every word typed has to appear in the section's title or keywords, in any order.
                            let text = format!("{} {}", tab.title().to_lowercase(), tab.keywords());
                            if !q.split_whitespace().all(|word| text.contains(word)) {
                                continue;
                            }
                            shown += 1;
                            let icon: fn(&egui::Painter, egui::Rect, Color32) = match tab {
                                SettingsTab::Notifications => theme::icon::bell,
                                SettingsTab::Usage => theme::icon::chart,
                                SettingsTab::Account => theme::icon::user,
                                SettingsTab::Updates => theme::icon::download,
                            };
                            if settings_nav(ui, self.settings_tab == tab, tab.title(), icon).clicked() {
                                self.settings_tab = tab;
                            }
                        }
                        if shown == 0 {
                            ui.add_space(4.0);
                            ui.label(RichText::new("Nothing matches.").size(12.5).color(theme::DIM));
                        }
                    });
                });
                let (sep, _) = ui.allocate_exact_size(egui::vec2(1.0, body), Sense::hover());
                ui.painter().rect_filled(sep, 0, theme::BORDER);
                // The open section.
                ui.vertical(|ui| {
                    ui.set_height(body);
                    egui::ScrollArea::vertical().id_salt("settings_body").auto_shrink([false, false]).show(ui, |ui| {
                        egui::Frame::NONE.inner_margin(egui::Margin { left: 28, right: 28, top: 22, bottom: 28 }).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 0.0;
                            match self.settings_tab {
                                SettingsTab::Notifications => self.settings_notifications(ui),
                                SettingsTab::Usage => self.settings_usage(ui),
                                SettingsTab::Account => self.settings_account(ui),
                                SettingsTab::Updates => self.settings_updates(ui),
                            }
                        });
                    });
                });
            });
        });
        if close || m.should_close() {
            self.settings_open = false;
        }
    }

    fn settings_notifications(&mut self, ui: &mut egui::Ui) {
        settings_heading(ui, "Notifications", "When Scout finishes, or needs an answer, while its window is in the background.");
        let (mut flash, mut toast) = (self.cfg.notify_flash.unwrap_or(true), self.cfg.notify_toast.unwrap_or(false));
        settings_row(ui, "Flash the taskbar button", "Scout's button blinks until you switch back to it.", |ui| {
            toggle(ui, &mut flash);
        });
        settings_row(ui, "Windows notification", "A notification in the corner of the screen says what Scout needs.", |ui| {
            toggle(ui, &mut toast);
        });
        settings_row(ui, "Try it", "Shows a notification now, whatever is chosen above.", |ui| {
            if theme::secondary_button(ui, "Send a test").clicked() {
                crate::notify::toast("Scout", "This is how Scout will tell you it's done.");
            }
        });
        if (flash, toast) != (self.cfg.notify_flash.unwrap_or(true), self.cfg.notify_toast.unwrap_or(false)) {
            self.set_notify(flash, toast);
        }
    }

    /// What the account has left this month, the resets it can use, and for admins the controls.
    fn usage_limits(&mut self, ui: &mut egui::Ui) {
        if self.cfg.api_key.is_none() {
            ui.label(RichText::new("Sign in to ScoutCode to see your monthly limits.").size(13.0).color(theme::MUTED));
            ui.add_space(18.0);
            return;
        }
        let mut change: Option<(Value, String)> = None;
        if let Some(info) = self.usage.info.clone() {
            let resets = format!("Resets {} UTC", short_date(info.resets_at.get(..10).unwrap_or("")));
            let banked: f64 = info.banked_money.iter().map(|b| b.amount).sum();
            let plan = |p: &Option<crate::api::Plan>| p.as_ref().map_or(("Unlimited".to_string(), "Requests are free".to_string()), |p| (usd(p.left), format!("left of {}", usd(p.limit))));
            let (money, money_sub) = plan(&info.money);
            let (cli, cli_sub) = plan(&info.cli);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let w = (ui.available_width() - 30.0) / 4.0;
                stat_tile(ui, w, "CLI credit (browser sign-in)", &cli, &cli_sub);
                stat_tile(ui, w, "Money (API keys)", &money, &money_sub);
                stat_tile(ui, w, "Banked", &usd(banked), "Spent next, runs out");
                stat_tile(ui, w, "Balance", &usd(info.balance), "Spent last, doesn't reset");
            });
            ui.add_space(8.0);
            ui.label(RichText::new(resets).size(12.0).color(theme::DIM));
            ui.add_space(8.0);
            for b in &info.banked_money {
                settings_row(ui, &format!("{} banked", usd(b.amount)), &format!("Expires {} UTC", short_date(b.expires_at.get(..10).unwrap_or(""))), |_| {});
            }
            let busy = self.usage.busy;
            for (i, r) in info.banked_resets.iter().enumerate() {
                settings_row(ui, "Banked reset", &format!("Clears your spending this month. Expires {} UTC.", short_date(r.expires_at.get(..10).unwrap_or(""))), |ui| {
                    // The one that runs out soonest goes first, so only the first row has the button.
                    if i == 0 && ui.add_enabled_ui(!busy, |ui| theme::primary_button(ui, "Use")).inner.clicked() {
                        change = Some((json!({ "action": "use-banked-reset" }), "Your usage this month is back to zero.".into()));
                    }
                });
            }
            if info.admin {
                self.usage_admin(ui, &info, &mut change);
            }
        } else if self.usage.busy {
            ui.add(egui::Spinner::new().size(16.0).color(theme::ACCENT));
        }
        if let Some((ok, text)) = &self.usage.message {
            ui.add_space(8.0);
            ui.add(egui::Label::new(RichText::new(text).size(13.0).color(if *ok { theme::GREEN } else { theme::RED })).wrap());
        }
        ui.add_space(22.0);
        if change.is_some() {
            self.usage_request(change);
        }
    }

    /// Admin controls: anyone's limits, giving money, and resetting everyone.
    fn usage_admin(&mut self, ui: &mut egui::Ui, info: &crate::api::UsageInfo, change: &mut Option<(Value, String)>) {
        let u = &mut self.usage;
        let field = |ui: &mut egui::Ui, text: &mut String, hint: &str, w: f32| ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(w).margin(egui::Margin::symmetric(12, 9)));
        let days = |s: &str| s.trim().parse::<u32>().ok().filter(|d| (1..=3650).contains(d));
        ui.add_space(14.0);
        theme::section_label(ui, "Admin");
        ui.add_space(6.0);
        ui.add_enabled_ui(!u.busy, |ui| {
            // Who the limits and a single gift are for.
            ui.label(RichText::new(format!("Changing {}", u.user)).font(theme::bold(13.5)).color(theme::STRONG));
            ui.add_space(6.0);
            field(ui, &mut u.search, "Find a user", 260.0);
            let q = u.search.trim().to_lowercase();
            if !q.is_empty() {
                ui.add_space(4.0);
                let found: Vec<_> = info.users.iter().filter(|m| m.username.contains(&q)).collect();
                let mut picked = None;
                for m in found.iter().take(6) {
                    let line = format!("{}   {} spent, {} CLI, {} balance, {} banked", m.username, usd(m.spent_this_month), usd(m.harness_spent_this_month), usd(m.balance), usd(m.banked));
                    if ui.selectable_label(m.username == u.user, line).clicked() {
                        picked = Some(m.username.clone());
                    }
                }
                if found.is_empty() {
                    ui.label(RichText::new("No matching users.").size(12.5).color(theme::DIM));
                } else if found.len() > 6 {
                    ui.label(RichText::new(format!("{} more. Keep typing to narrow it down.", found.len() - 6)).size(12.5).color(theme::DIM));
                }
                if let Some(name) = picked {
                    u.user = name;
                    u.search.clear();
                    u.fill_limits();
                }
            }
            ui.add_space(12.0);

            ui.label(RichText::new("Limits per month").size(13.0).color(theme::MUTED));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                for (mode, label) in [("default", "Default"), ("custom", "Custom"), ("unlimited", "Unlimited")] {
                    if ui.radio(u.mode == mode, label).clicked() {
                        u.mode = mode.into();
                    }
                }
            });
            ui.add_space(6.0);
            let plan_cli = info.users.iter().find(|m| m.username == u.user).map_or(0.0, |m| m.cli_monthly);
            ui.horizontal(|ui| {
                if u.mode == "custom" {
                    field(ui, &mut u.monthly, "Money per month, USD", 180.0);
                }
                field(ui, &mut u.cli, &format!("CLI credit (now {})", usd(plan_cli)), 180.0);
                if theme::primary_button(ui, "Save limits").clicked() {
                    let monthly = parse_usd(&u.monthly);
                    let cli = u.cli.trim();
                    let cli_value = parse_usd(cli);
                    if u.mode == "custom" && monthly.is_none() {
                        u.message = Some((false, "Money per month is an amount in USD, like 5 (0 = none).".into()));
                    } else if !cli.is_empty() && cli_value.is_none() {
                        u.message = Some((false, "CLI credit is an amount in USD, like 5. Leave it empty for what the plan gets.".into()));
                    } else {
                        let limits = if u.mode == "custom" { json!({ "monthly": monthly }) } else { json!(u.mode) };
                        *change = Some((json!({ "action": "set-limits", "username": u.user, "limits": limits, "cliLimit": cli_value }), format!("Saved the limits for {}.", u.user)));
                    }
                }
            });
            ui.add_space(14.0);

            ui.label(RichText::new("Give money").size(13.0).color(theme::MUTED));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.radio(!u.give_banked, "Balance").clicked() {
                    u.give_banked = false;
                }
                if ui.radio(u.give_banked, "Banked balance").clicked() {
                    u.give_banked = true;
                }
                ui.add_space(16.0);
                if ui.radio(!u.give_everyone, format!("To {}", u.user)).clicked() {
                    u.give_everyone = false;
                    u.confirm = None;
                }
                if ui.radio(u.give_everyone, "To everyone").clicked() {
                    u.give_everyone = true;
                    u.confirm = None;
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                field(ui, &mut u.amount, "Amount, USD", 140.0);
                if u.give_banked {
                    field(ui, &mut u.expiry_days, "Expires in days", 140.0);
                }
                let label = if u.confirm == Some("give") { "Confirm: give to everyone" } else { "Give" };
                if theme::primary_button(ui, label).clicked() {
                    match (parse_usd(&u.amount).filter(|a| *a > 0.0), days(&u.expiry_days)) {
                        (None, _) => u.message = Some((false, "The amount is in USD and more than $0, like 5.".into())),
                        (_, None) if u.give_banked => u.message = Some((false, "The expiry is a whole number of days, from 1 to 3650.".into())),
                        _ if u.give_everyone && u.confirm != Some("give") => u.confirm = Some("give"),
                        (Some(amount), d) => {
                            let who = if u.give_everyone { "everyone".to_string() } else { u.user.clone() };
                            let what = if u.give_banked { "banked balance" } else { "balance" };
                            let mut body = json!({ "action": if u.give_banked { "give-banked" } else { "give-balance" }, "amount": amount, "expiresInDays": d });
                            if u.give_everyone {
                                body["everyone"] = json!(true);
                            } else {
                                body["username"] = json!(u.user);
                            }
                            *change = Some((body, format!("Gave {who} {} of {what}.", usd(amount))));
                            u.amount.clear();
                        }
                    }
                }
            });
            ui.add_space(10.0);

            let reset_label = if u.confirm == Some("reset") { "Confirm: reset everyone" } else { "Reset usage" };
            settings_row(ui, "Reset usage", "Everyone's spending this month and free requests today go back to zero. Banked money and balances stay.", |ui| {
                if theme::secondary_button(ui, reset_label).clicked() {
                    if u.confirm == Some("reset") {
                        *change = Some((json!({ "action": "reset-all" }), "Everyone's usage is back to zero.".into()));
                    } else {
                        u.confirm = Some("reset");
                    }
                }
            });
            let banked_label = if u.confirm == Some("banked") { "Confirm: give to everyone" } else { "Banked Reset" };
            settings_row(ui, "Banked Reset", "Everyone gets one reset of their own to use when they want, before it expires.", |ui| {
                if theme::secondary_button(ui, banked_label).clicked() {
                    match days(&u.reset_days) {
                        None => u.message = Some((false, "The expiry is a whole number of days, from 1 to 3650.".into())),
                        Some(d) if u.confirm == Some("banked") => {
                            *change = Some((json!({ "action": "give-banked-reset", "everyone": true, "expiresInDays": d }), format!("Everyone got a banked reset that expires in {d} days.")));
                        }
                        Some(_) => u.confirm = Some("banked"),
                    }
                }
                ui.add_space(6.0);
                field(ui, &mut u.reset_days, "Days", 70.0);
                ui.label(RichText::new("Expires in days").size(12.5).color(theme::DIM));
            });
        });
    }

    fn settings_usage(&mut self, ui: &mut egui::Ui) {
        settings_heading(ui, "Usage", "What you have left this month, and the tokens Scout used each day on this PC, from the desktop app and the CLI. Days and months are UTC.");
        self.usage_limits(ui);
        let log = &self.usage_log;
        let sum = |n: usize| crate::usage::last_days(log, n).iter().fold((0u64, 0f64), |(t, c), (_, d)| (t + d.tokens, c + d.cost));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let w = (ui.available_width() - 20.0) / 3.0;
            for (label, (t, c)) in [("Today", sum(1)), ("Last 7 days", sum(7)), ("Last 30 days", sum(30))] {
                stat_tile(ui, w, label, &format!("{} tokens", short_tokens(t)), &format!("${c:.2}"));
            }
        });
        ui.add_space(14.0);
        usage_chart(ui, log);
        ui.add_space(10.0);
        let note = match log.seeded_on.as_deref() {
            Some(d) if log.days.keys().any(|k| k.as_str() < d) => format!("Days before {} are estimated from saved chats: each chat counts on the day it was last used.", short_date(d)),
            _ => "Counted from each request as it's answered.".to_string(),
        };
        ui.label(RichText::new(note).size(12.0).color(theme::DIM));
    }

    fn settings_account(&mut self, ui: &mut egui::Ui) {
        settings_heading(ui, "Account", "The ScoutCode account this app and the CLI use.");
        let browser = config::is_sign_in(self.cfg.api_key.as_deref());
        let name = self.cfg.username.clone().unwrap_or_else(|| if self.cfg.api_key.is_some() { "Unknown until the model list loads" } else { "Not signed in" }.into());
        settings_row(ui, "Signed in as", &name, |_| {});
        let (how, pays) = if browser { ("Browser sign-in", "Uses your monthly CLI credit, then any banked money and balance.") } else { ("API key", "Pays from your account's money.") };
        settings_row(ui, how, pays, |_| {});
        settings_row(ui, "Account page", "Keys, balance and billing on scoutcode.dev.", |ui| {
            if theme::secondary_button(ui, "Open").clicked() {
                let site = self.cfg.base_url.trim_end_matches("/v1");
                ui.ctx().open_url(egui::OpenUrl::new_tab(format!("{site}/account")));
            }
        });
        let mut sign_out = false;
        settings_row(ui, "Sign out", "Forgets the login on this PC, for the CLI too.", |ui| {
            sign_out = theme::secondary_button(ui, "Sign out").clicked();
        });
        if sign_out {
            config::save_one("apiKey", Value::Null);
            self.set_key(None);
            self.settings_open = false;
        }
        ui.add_space(22.0);
        theme::section_label(ui, "OpenRouter");
        ui.add_space(6.0);
        let how = "Tried from the top. If one fails, the next one is used. Saved on this PC, for the CLI too. Models on OpenRouter bill your OpenRouter account.";
        ui.label(RichText::new(how).size(12.5).color(theme::DIM));
        ui.add_space(8.0);
        let mut keys = self.cfg.openrouter_keys.clone();
        let count = keys.len();
        let mut edit: Option<(usize, KeyEdit)> = None;
        for (i, k) in keys.iter().enumerate() {
            settings_row(ui, &format!("{}. {}", i + 1, k.name), &mask_key(&k.key), |ui| {
                if i > 0 && theme::secondary_button(ui, "Up").clicked() {
                    edit = Some((i, KeyEdit::Up));
                }
                if i + 1 < count && theme::secondary_button(ui, "Down").clicked() {
                    edit = Some((i, KeyEdit::Down));
                }
                if theme::secondary_button(ui, "Remove").clicked() {
                    edit = Some((i, KeyEdit::Remove));
                }
            });
        }
        if let Some((i, op)) = edit {
            match op {
                KeyEdit::Up => keys.swap(i, i - 1),
                KeyEdit::Down => keys.swap(i, i + 1),
                KeyEdit::Remove => {
                    keys.remove(i);
                }
            }
            self.save_openrouter_keys(keys);
        }
        ui.add_space(12.0);
        let width = ui.available_width().min(420.0);
        ui.add(egui::TextEdit::singleline(&mut self.login.or_name).hint_text("Name, e.g. Work").desired_width(width).margin(egui::Margin::symmetric(12, 9)));
        ui.add_space(6.0);
        ui.add(egui::TextEdit::singleline(&mut self.login.or_key).password(true).hint_text("sk-or-…").desired_width(width).margin(egui::Margin::symmetric(12, 9)));
        ui.add_space(6.0);
        let ready = !self.login.or_key.trim().is_empty();
        if ui.add_enabled(ready, egui::Button::new(RichText::new("Add key").size(13.5)).corner_radius(10).min_size(egui::vec2(width, 36.0))).clicked() {
            let key = std::mem::take(&mut self.login.or_key).trim().to_string();
            let name = std::mem::take(&mut self.login.or_name).trim().to_string();
            self.add_openrouter_key(name, key);
        }
    }

    fn settings_updates(&mut self, ui: &mut egui::Ui) {
        settings_heading(ui, "Updates", "Scout checks for new versions when it starts and every few hours. They're signed by ScoutCode and checked before they're installed.");
        let version = format!("Scout {}", crate::update::VERSION);
        if !self.updater.enabled() {
            settings_row(ui, &version, "This copy doesn't update itself. Only the installed app does, not a development build.", |_| {});
            return;
        }
        let status = self.updater.status();
        let (line, notes): (String, Option<String>) = match &status {
            Status::Idle => ("Hasn't checked yet.".into(), None),
            Status::Checking => ("Checking for updates…".into(), None),
            Status::UpToDate => ("You're on the latest version.".into(), None),
            Status::Available { version, notes } => (format!("Version {version} is available."), Some(notes.clone())),
            Status::Downloading { version, got, total } => (format!("Downloading version {version}: {}%", got * 100 / (*total).max(1)), None),
            Status::Ready { version, notes } => (format!("Version {version} is ready. It installs when you restart or close Scout."), Some(notes.clone())),
            Status::Failed(e) => (e.clone(), None),
        };
        settings_row(ui, &version, &line, |ui| match &status {
            Status::Ready { .. } => {
                let blocker = self.restart_blocker();
                let restart = ui.add_enabled_ui(blocker.is_none(), |ui| theme::primary_button(ui, "Restart to update")).inner;
                if restart.on_disabled_hover_text(blocker.unwrap_or_default()).clicked() {
                    self.restart_to_update(ui.ctx());
                }
            }
            Status::Available { .. } => {
                if theme::primary_button(ui, "Download").clicked() {
                    self.updater.download_now();
                }
            }
            Status::Checking | Status::Downloading { .. } => {
                ui.add(egui::Spinner::new().size(16.0).color(theme::ACCENT));
            }
            Status::Idle | Status::UpToDate | Status::Failed(_) => {
                if theme::secondary_button(ui, "Check now").clicked() {
                    self.updater.check_now();
                }
            }
        });
        let mut auto = self.cfg.auto_update;
        settings_row(ui, "Download automatically", "New versions download in the background and wait for your next restart.", |ui| {
            toggle(ui, &mut auto);
        });
        if auto != self.cfg.auto_update {
            self.cfg.auto_update = auto;
            config::save_one("desktopAutoUpdate", json!(auto));
            self.updater.set_auto(auto);
        }
        if let Some(notes) = notes.filter(|n| !n.trim().is_empty()) {
            ui.add_space(22.0);
            theme::section_label(ui, "What's new");
            ui.add_space(6.0);
            ui.add(egui::Label::new(RichText::new(notes).size(13.0).color(theme::MUTED)).wrap());
        }
    }

    fn delete_dialog(&mut self, ctx: &egui::Context) {
        let Some((id, title)) = self.deleting.clone() else { return };
        let mut answer = None;
        let m = egui::Modal::new(egui::Id::unique("delete_chat")).frame(modal_frame()).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new("Delete this chat?").font(theme::bold(17.0)).color(theme::STRONG));
            ui.add_space(6.0);
            ui.add(egui::Label::new(RichText::new(&title).size(13.5).color(theme::TEXT)).truncate());
            ui.add_space(4.0);
            ui.label(RichText::new("It's removed from this PC for good, for the CLI too. Files Scout changed stay as they are.").size(12.5).color(theme::DIM));
            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let del = egui::Button::new(RichText::new("Delete").font(theme::bold(13.5)).color(Color32::WHITE)).fill(theme::RED).corner_radius(8);
                if ui.add(del).clicked() || ui.input(|i| i.key_pressed(Key::Enter)) {
                    answer = Some(true);
                }
                if theme::secondary_button(ui, "Cancel").clicked() {
                    answer = Some(false);
                }
            });
        });
        if m.should_close() && answer.is_none() {
            answer = Some(false);
        }
        match answer {
            Some(true) => {
                self.deleting = None;
                self.delete_chat(&id);
            }
            Some(false) => self.deleting = None,
            None => {}
        }
    }

    /// Messages waiting for Scout to finish, each of which can be sent into the running turn instead, and the ones
    /// already handed to it.
    fn queue_card(&mut self, ui: &mut egui::Ui) {
        let steering: Vec<String> = self.shared.steer.lock().unwrap().iter().map(|s| s.text.clone()).collect();
        if self.queue.is_empty() && steering.is_empty() {
            return;
        }
        let mut steer = None;
        let mut remove = None;
        egui::Frame::NONE.fill(theme::SURFACE).stroke(Stroke::new(1.0, theme::BORDER)).corner_radius(12).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 2.0;
            for t in &steering {
                queued_row(ui, t, 0, "Sending now · Scout reads it after this step", |_| {});
            }
            for (i, q) in self.queue.iter().enumerate() {
                let label = if self.running { "Queued · sent when Scout is done" } else { "Queued" };
                queued_row(ui, &q.text, q.files.len(), label, |ui| {
                    if theme::icon_button(ui, 24.0, "Take it out of the queue (back into the box when that's empty)", theme::icon::close).clicked() {
                        remove = Some(i);
                    }
                    if self.running {
                        if ui.add(egui::Button::new(RichText::new("Steer").size(12.5).color(theme::ACCENT)).frame_when_inactive(false).corner_radius(6)).on_hover_text("Send it now: Scout reads it after the step it's on").clicked() {
                            steer = Some(i);
                        }
                    } else if ui.add(egui::Button::new(RichText::new("Send").size(12.5).color(theme::ACCENT)).frame_when_inactive(false).corner_radius(6)).clicked() {
                        steer = Some(i);
                    }
                });
            }
        });
        ui.add_space(8.0);
        if let Some(i) = remove {
            let o = self.queue.remove(i);
            // Back into the box, in case it was removed to be changed.
            if self.input.trim().is_empty() {
                self.input = o.text;
                self.attachments = o.files;
            }
        } else if let Some(i) = steer {
            let o = self.queue.remove(i);
            if self.running { self.steer(o) } else { self.submit(o) }
        }
    }

    /// Mic: click to start talking, click again to put what you said in the box. Ctrl+Shift+Space does the same.
    fn mic_button(&mut self, ui: &mut egui::Ui) {
        let shortcut = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Space));
        let clicked = match &self.voice {
            Voice::Idle => icon_box(ui, "Dictate (Ctrl+Shift+Space). Transcribed by Deepgram through ScoutCode", theme::icon::mic).clicked(),
            Voice::Recording(r) => {
                let (rect, b) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), Sense::click());
                let p = ui.painter();
                p.rect(rect, 8, theme::RED.gamma_multiply(0.14), Stroke::new(1.0, theme::RED.gamma_multiply(0.6)), egui::StrokeKind::Inside);
                // A ring that swells with the voice, around the mic.
                p.circle_stroke(rect.center(), 6.0 + 3.0 * r.level().sqrt(), Stroke::new(1.5, theme::RED));
                theme::icon::mic(p, icon_rect(rect), theme::RED);
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
                if b.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                b.on_hover_text("Listening. Click (or Ctrl+Shift+Space) to stop and transcribe").clicked()
            }
            Voice::Busy(s) => {
                let s = *s;
                let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), Sense::hover());
                ui.painter().rect(rect, 8, theme::SURFACE_HI, Stroke::new(1.0, theme::BORDER_HI), egui::StrokeKind::Inside);
                ui.put(rect.shrink(7.0), egui::Spinner::new().size(14.0).color(theme::ACCENT)).on_hover_text(s);
                false
            }
        };
        if clicked || shortcut {
            self.toggle_voice();
        }
    }

    /// Attach and mic, as the composer's buttons.
    fn composer_controls(&mut self, ui: &mut egui::Ui) {
        if icon_box(ui, "Attach files (or drop them on the window, or type @)", theme::icon::paperclip).clicked() {
            self.pick_files();
        }
        self.mic_button(ui);
    }

    /// The reasoning effort, as a compact menu in the footer, beside the model.
    fn effort_chip(&mut self, ui: &mut egui::Ui) {
        let effort_title = format!("{}{}", self.effort[..1].to_uppercase(), &self.effort[1..]);
        let effort_chip = chip(ui, &effort_title, theme::MUTED, None, true).on_hover_text("Reasoning effort");
        let before_effort = self.effort.clone();
        // The reasoning effort has its own menu, from its own chip.
        egui::Popup::menu(&effort_chip).width(200.0).show(|ui| {
            for e in EFFORTS {
                let selected = self.effort == e;
                let text = format!("{}{}", e[..1].to_uppercase(), &e[1..]);
                if menu_item(ui, selected, &text).clicked() {
                    self.effort = e.to_string();
                }
            }
        });
        if before_effort != self.effort {
            *self.shared.effort.lock().unwrap() = self.effort.clone();
            config::save_one("effort", json!(self.effort));
        }
    }

    /// The model, as a menu with search and where it runs. It sits in the footer, beside the usage ring.
    fn model_chip(&mut self, ui: &mut egui::Ui) {
        let current = self.current_key();
        let label = self.models.iter().find(|m| m.id == current).map(|m| m.label()).unwrap_or_else(|| crate::api::short_id(&self.model));
        let model: String = if label.chars().count() > 26 { format!("{}…", label.chars().take(25).collect::<String>()) } else { label };
        let hint = if self.provider == "openrouter" { "Model · running on OpenRouter" } else { "Model" };
        let r = chip(ui, &model, theme::MUTED, None, true).on_hover_text(hint);
        // The model row under the pointer, and where it is, for the stats card beside the menu.
        let mut hovered: Option<(String, egui::Rect)> = None;
        // A row click picks a model and where it runs: the provider its row is listed under.
        let mut pick: Option<(String, &'static str)> = None;
        let just_opened = !self.model_menu_open;
        if just_opened {
            self.model_search.clear();
            self.model_tab = if self.provider == "openrouter" { ModelTab::OpenRouter } else { ModelTab::ScoutCode };
        }
        let has_openrouter = self.models.iter().any(|m| m.openrouter_id.is_some());
        // Pinned to the chip's right edge, which stays put; its left edge moves with the model name's length.
        let shown = egui::Popup::menu(&r)
            .align(egui::RectAlign::TOP_END)
            .align_alternatives(&[egui::RectAlign::BOTTOM_END])
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
            ui.horizontal_top(|ui| {
                // The rail: starred models, then a tab per provider.
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    let tabs: &[(ModelTab, &str)] = &[(ModelTab::Starred, "Starred"), (ModelTab::ScoutCode, "ScoutCode"), (ModelTab::OpenRouter, "OpenRouter")];
                    for &(tab, tip) in tabs {
                        if tab == ModelTab::OpenRouter && !has_openrouter {
                            continue;
                        }
                        let on = self.model_tab == tab && self.model_search.trim().is_empty();
                        if rail_tab(ui, on, tip, tab).clicked() {
                            self.model_tab = tab;
                            self.model_search.clear();
                        }
                    }
                });
                let sep = ui.available_rect_before_wrap();
                ui.painter().vline(sep.left(), sep.top()..=sep.top() + 440.0, Stroke::new(1.0, theme::BORDER));
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.set_width(300.0);
                    let search = ui.add(
                        egui::TextEdit::singleline(&mut self.model_search)
                            .hint_text(RichText::new("Search models").color(theme::DIM))
                            .desired_width(f32::INFINITY)
                            .margin(egui::Margin::symmetric(10, 6)),
                    );
                    if just_opened {
                        search.request_focus();
                    }
                    ui.add_space(4.0);
                    let q = self.model_search.trim().to_lowercase();
                    // One row per model and place it runs: (list key, provider, name, star key). A model both serve
                    // is listed under each, and its star key is the id that provider knows it by.
                    let mut scout: Vec<(String, &'static str, String, String)> = vec![];
                    scout.extend(self.models.iter().filter(|m| m.hosted).map(|m| (m.id.clone(), "scoutcode", m.label(), m.id.clone())));
                    let openrouter = self.models.iter().filter_map(|m| m.openrouter_id.as_ref().map(|or| (m.id.clone(), "openrouter", m.label(), or.clone())));
                    let all: Vec<_> = scout.iter().cloned().chain(openrouter).collect();
                    let mut rows: Vec<_> = if !q.is_empty() {
                        // A search looks through every tab.
                        all.into_iter().filter(|(id, _, name, star)| [id, name, star].iter().any(|s| s.to_lowercase().contains(&q))).collect()
                    } else {
                        match self.model_tab {
                            ModelTab::Starred => {
                                let mut v: Vec<_> = all.into_iter().filter(|r| self.starred.contains(&r.3)).collect();
                                v.sort_by_key(|r| self.starred.iter().position(|s| *s == r.3));
                                v
                            }
                            ModelTab::ScoutCode => scout,
                            ModelTab::OpenRouter => all.into_iter().filter(|r| r.1 == "openrouter").collect(),
                        }
                    };
                    // Starred first within a provider's tab too.
                    if !q.is_empty() || self.model_tab != ModelTab::Starred {
                        rows.sort_by_key(|r| self.starred.iter().position(|s| *s == r.3).unwrap_or(usize::MAX));
                    }
                    let mut toggle = None;
                    // Always full height, so the menu doesn't grow and shrink between tabs and searches.
                    egui::ScrollArea::vertical().id_salt("models").max_height(400.0).auto_shrink([false, false]).show(ui, |ui| {
                        for (id, provider, text, star_key) in &rows {
                            let starred = self.starred.contains(star_key);
                            let free = self.models.iter().any(|m| m.id == *id && m.free) || star_key.ends_with(":free");
                            let selected = current == *id && (self.provider == "openrouter") == (*provider == "openrouter");
                            let (row, star) = model_item(ui, selected, starred, free, text, provider);
                            if row.hovered() || star.hovered() {
                                hovered = Some((id.clone(), row.rect));
                            }
                            if star.clicked() {
                                toggle = Some(star_key.clone());
                            } else if row.clicked() {
                                pick = Some((id.clone(), provider));
                            }
                        }
                        if rows.is_empty() {
                            let empty = if !q.is_empty() { "No models match." } else if self.model_tab == ModelTab::Starred { "Star a model to keep it here." } else { "No models here yet." };
                            ui.add_space(6.0);
                            ui.label(RichText::new(empty).size(12.5).color(theme::DIM));
                        }
                    });
                    if let Some(id) = toggle {
                        if let Some(i) = self.starred.iter().position(|s| *s == id) {
                            self.starred.remove(i);
                        } else {
                            self.starred.push(id);
                        }
                        config::save_one("starredModels", json!(self.starred));
                    }
                });
            });
        });
        self.model_menu_open = shown.is_some();
        if let (Some(menu), Some((id, row))) = (&shown, hovered) {
            if let Some(m) = self.models.iter().find(|m| m.id == id) {
                model_card(ui.ctx(), m, menu.response.rect, row);
            }
        }
        if let Some((key, provider)) = pick {
            self.choose_model(&key, provider);
        }
    }

    fn input_box(&mut self, ui: &mut egui::Ui) {
        let id = egui::Id::unique("prompt");
        let focused = ui.ctx().memory(|m| m.has_focus(id));
        // The @ picker's rows, from the keys and text of the last frame.
        let picks: Vec<String> = match (&self.mention, &self.file_index) {
            (Some(m), Some((cwd, files, _))) if session::same_dir(cwd, &self.cwd) => attach::matches(files, &m.query, 8).into_iter().cloned().collect(),
            _ => vec![],
        };
        let mut picked = None;
        if focused && self.mention.is_some() {
            if !picks.is_empty() {
                self.mention_pick = self.mention_pick.min(picks.len() - 1);
                if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
                    self.mention_pick = (self.mention_pick + 1) % picks.len();
                }
                if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
                    self.mention_pick = (self.mention_pick + picks.len() - 1) % picks.len();
                }
                if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Tab)) {
                    picked = Some(picks[self.mention_pick].clone());
                }
            }
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                self.mention_closed = self.mention.as_ref().map(|m| m.start);
                self.mention = None;
            }
        }
        if focused {
            // Enter sends (or queues while Scout works), Ctrl+Enter sends into the running turn, Shift+Enter makes a
            // new line; ↑/↓ walk history when the box is empty or unchanged.
            if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
                self.send(true);
            }
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
                self.send(false);
            }
            let at_history = self.input.is_empty() || self.history_pos.is_some_and(|p| self.history.get(p) == Some(&self.input));
            if at_history && !self.history.is_empty() {
                if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
                    let p = self.history_pos.map_or(self.history.len() - 1, |p| p.saturating_sub(1));
                    self.history_pos = Some(p);
                    self.input = self.history[p].clone();
                } else if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
                    match self.history_pos {
                        Some(p) if p + 1 < self.history.len() => {
                            self.history_pos = Some(p + 1);
                            self.input = self.history[p + 1].clone();
                        }
                        _ => {
                            self.history_pos = None;
                            self.input.clear();
                        }
                    }
                }
            }
        }
        let stroke = if focused { Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.35)) } else { Stroke::new(1.0, theme::BORDER_HI) };
        // While Scout works, the shadow turns into a slowly breathing amber glow.
        let now = ui.input(|i| i.time);
        let shadow = if self.running {
            let breath = 0.5 + 0.5 * (now * 1.6).sin() as f32;
            egui::Shadow { offset: [0, 0], blur: 30, spread: 1, color: theme::ACCENT.gamma_multiply(0.10 + 0.08 * breath) }
        } else {
            egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(90) }
        };
        let framed = egui::Frame::NONE
            .fill(theme::SURFACE)
            .stroke(stroke)
            .corner_radius(16)
            .inner_margin(egui::Margin { left: 8, right: 10, top: 12, bottom: 10 })
            .shadow(shadow)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if !self.attachments.is_empty() {
                    let mut remove = None;
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(6.0);
                        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                        for (i, f) in self.attachments.iter().enumerate() {
                            if file_chip(ui, f, true) {
                                remove = Some(i);
                            }
                        }
                    });
                    if let Some(i) = remove {
                        self.attachments.remove(i);
                    }
                    ui.add_space(8.0);
                }
                let hint = if self.running { "Scout is working… type to queue a message" } else { "Ask Scout to build, fix or explain something" };
                let out = egui::TextEdit::multiline(&mut self.input)
                    .id(id)
                    .hint_text(RichText::new(hint).color(theme::DIM))
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .frame(egui::Frame::NONE)
                    .margin(egui::Margin::symmetric(10, 0))
                    .font(FontId::proportional(theme::BODY + 0.5))
                    .show(ui);
                let r = out.response.response.clone();
                if let Some(path) = picked {
                    self.pick_mention(ui.ctx(), id, &path);
                } else {
                    let cursor = out.cursor_range.map(|c| c.primary.index.0);
                    self.update_mention(r.has_focus(), cursor);
                }
                if !picks.is_empty() && self.mention.is_some() {
                    let mut clicked = None;
                    egui::Area::new(id.with("mention")).order(egui::Order::Foreground).pivot(egui::Align2::LEFT_BOTTOM).fixed_pos(r.rect.left_top() + egui::vec2(0.0, -14.0)).show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                            ui.set_width(420.0);
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for (i, p) in picks.iter().enumerate() {
                                if mention_row(ui, i == self.mention_pick, p).clicked() {
                                    clicked = Some(p.clone());
                                }
                            }
                            ui.add_space(2.0);
                            ui.label(RichText::new("↑↓ to choose · Enter or Tab to attach · Esc to close").size(11.0).color(theme::FAINT));
                        });
                    });
                    if let Some(p) = clicked {
                        self.pick_mention(ui.ctx(), id, &p);
                    }
                }
                if self.focus_input {
                    // Not while someone is typing in the terminal or the editor.
                    if !self.dock.has_keyboard(ui.ctx()) {
                        r.request_focus();
                    }
                    self.focus_input = false;
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    self.composer_controls(ui);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let ready = !self.input.trim().is_empty() || !self.attachments.is_empty();
                        if self.running {
                            let (rect, b) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), Sense::click());
                            let p = ui.painter();
                            p.circle_filled(rect.center(), 16.0, if b.hovered() { theme::BORDER_HI } else { theme::SURFACE_HI });
                            theme::icon::stop(p, rect, theme::STRONG);
                            if b.clicked() {
                                self.stop();
                            }
                            b.on_hover_text(if self.stopping { "Stopping" } else { "Stop (Esc)" });
                        }
                        // While Scout works this queues; Ctrl+Enter sends into the running turn instead.
                        if ready || !self.running {
                            let (rect, b) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), Sense::click());
                            let p = ui.painter();
                            let fill = if !ready { theme::SURFACE_HI } else if b.hovered() { theme::ACCENT_HI } else { theme::ACCENT };
                            p.circle_filled(rect.center(), 16.0, fill);
                            theme::icon::arrow_up(p, rect, if ready { theme::ACCENT_INK } else { theme::DIM });
                            if ready && b.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            if ready && b.clicked() {
                                self.send(ui.input(|i| i.modifiers.command));
                            }
                            if self.running {
                                b.on_hover_text("Queue (Enter) · Ctrl+click or Ctrl+Enter to send it now");
                            }
                        }
                    });
                });
            });
        if self.running {
            // Above the panels so it can spill past the composer's edges, but under popups like the @ picker.
            let layer = egui::LayerId::new(egui::Order::Middle, egui::Id::unique("code_aura"));
            theme::code_aura(&ui.ctx().layer_painter(layer), framed.response.rect, now);
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
        }
    }

    fn empty_state(&mut self, ui: &mut egui::Ui) {
        let cwd = self.cwd();
        let name = self.project_label();
        let enabled = !self.running;
        let mut picked = None;
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.5 - 90.0).max(24.0));
            let (rect, _) = ui.allocate_exact_size(egui::vec2(48.0, 48.0), Sense::hover());
            theme::icon::logo(ui.painter(), rect);
            ui.add_space(18.0);
            // "< Building in project? >": the whole line, brackets included, types itself out with a caret. Clicking the
            // project's name opens a dropdown of the saved projects to switch to.
            let now = ui.input(|i| i.time);
            let start = *self.splash_start.get_or_insert(now);
            let font = theme::bold(26.0);
            let parts = [
                ("<".to_string(), theme::ACCENT),
                (format!(" {} ", SPLASH[self.splash]), theme::STRONG),
                (name.clone(), theme::ACCENT),
                ("?".to_string(), theme::STRONG),
                (">".to_string(), theme::ACCENT),
            ];
            let total: usize = parts.iter().map(|(t, _)| t.chars().count()).sum();
            // A short pause, then about 18 characters a second.
            let shown = (((now - start - 0.3).max(0.0) / 0.055) as usize).min(total);
            let job = |n: usize| {
                let mut job = egui::text::LayoutJob::default();
                let mut left = n;
                for (t, c) in &parts {
                    let s: String = t.chars().take(left).collect();
                    left -= s.chars().count();
                    job.append(&s, 0.0, egui::TextFormat { font_id: font.clone(), color: *c, ..Default::default() });
                }
                job
            };
            let full = ui.painter().layout_job(job(total));
            let typed = ui.painter().layout_job(job(shown));
            // The caret's width, and room for it past the full line.
            let caret_w = 3.0;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0 + full.size().x + caret_w + 12.0, 48.0), Sense::hover());
            let y = rect.center().y;
            let text_x = rect.left() + 12.0;
            let text_top = y - full.size().y / 2.0;
            // Only the project's name is the hover target: it gets an underline and opens the switcher.
            let prefix_w = ui.painter().layout_no_wrap(format!("< {} ", SPLASH[self.splash]), font.clone(), theme::STRONG).size().x;
            let name_g = ui.painter().layout_no_wrap(name.clone(), font.clone(), theme::ACCENT);
            let name_x = text_x + prefix_w;
            let name_rect = egui::Rect::from_min_size(egui::pos2(name_x, text_top), name_g.size());
            let sense = if enabled { Sense::click() } else { Sense::hover() };
            let r = ui.interact(name_rect, ui.scope_id().with("switch_project"), sense).on_hover_text("Switch project");
            let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&r));
            let hot = enabled && (r.hovered() || open);
            if hot {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let p = ui.painter();
            let typed_w = typed.size().x;
            p.galley(egui::pos2(text_x, text_top), typed, theme::STRONG);
            // The caret stays solid while typing, then blinks.
            let typing = shown < total;
            if typing || (now * 1000.0) as u64 / 530 % 2 == 0 {
                let caret = egui::Rect::from_min_size(egui::pos2(text_x + typed_w + 2.0, y - 13.0), egui::vec2(caret_w, 26.0));
                p.rect_filled(caret, 1, theme::ACCENT);
            }
            if hot {
                p.hline(name_x..=name_x + name_g.size().x, text_top + name_g.size().y - 2.0, egui::Stroke::new(1.2, theme::ACCENT));
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(if typing { 30 } else { 530 }));
            egui::Popup::menu(&r).width(240.0).show(|ui| {
                for p in &self.projects {
                    if menu_item(ui, session::same_dir(&p.path, &cwd), &p.name).clicked() {
                        picked = Some(p.path.clone());
                    }
                }
            });
        });
        if let Some(dir) = picked.filter(|d| !session::same_dir(d, &cwd)) {
            self.change_project(dir);
        }
    }

    fn transcript(&mut self, ui: &mut egui::Ui) {
        if self.items.is_empty() {
            self.empty_state(ui);
            return;
        }
        let max_w = MAX_W;
        self.find_bar(ui);
        let current_hit = self.find.hits.get(self.find.at).copied().filter(|_| self.find.open);
        let mut area = egui::ScrollArea::vertical().stick_to_bottom(!self.find.jump).auto_shrink([false, false]);
        if let Some(hit) = current_hit.filter(|_| self.find.jump) {
            // Items keep their last height, so the offset of the hit is the sum of the ones above it.
            let y = 20.0 + self.items[..hit].iter().map(|i| i.h).sum::<f32>();
            area = area.vertical_scroll_offset((y - 80.0).max(0.0));
            self.find.jump = false;
        }
        let editable = !self.running;
        let groups = tool_groups(&self.items, self.running);
        let mut edit = None;
        let mut resend = None;
        let mut cancel_edit = false;
        let mut toggle = None;
        let editing = &mut self.editing;
        area.show_viewport(ui, |ui, viewport| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let full = ui.available_width();
            let w = full.min(max_w);
            let pad = ((full - w) / 2.0).max(0.0);
            let origin = ui.min_rect().top();
            ui.add_space(20.0);
            let last = self.items.len().saturating_sub(1);
            let streaming = self.streaming;
            for (n, item) in self.items.iter_mut().enumerate() {
                let group = groups.iter().find(|g| (g.start..g.end).contains(&n));
                let folded = group.is_some_and(|g| !g.open);
                if group.is_some_and(|g| !g.open && n != g.start) {
                    // Inside a folded run: only its header is shown.
                    item.h = 0.0;
                    item.w = w;
                    continue;
                }
                let y = ui.cursor().top() - origin;
                let being_edited = editing.as_ref().is_some_and(|(i, _)| *i == n);
                let live = n == last || Some(n) == streaming || being_edited || matches!(&item.kind, Kind_::Tool { result: None, .. });
                let off = y + item.h < viewport.top() - 400.0 || y > viewport.bottom() + 400.0;
                if !live && off && item.h > 0.0 && (item.w - w).abs() < 0.5 {
                    // Not on screen and unchanged: keep its space without laying it out.
                    ui.allocate_exact_size(egui::vec2(full, item.h), Sense::hover());
                    continue;
                }
                let r = ui.horizontal(|ui| {
                    ui.add_space(pad);
                    ui.vertical(|ui| {
                        ui.set_width(w);
                        match editing.as_mut().filter(|(i, _)| *i == n) {
                            Some((_, text)) => match edit_box(ui, text) {
                                Some(true) => resend = Some((n, text.trim().to_string())),
                                Some(false) => cancel_edit = true,
                                None => {}
                            },
                            None => {
                                if let Some(g) = group.filter(|g| g.start == n) {
                                    if tool_group_header(ui, g) {
                                        toggle = Some(n);
                                    }
                                }
                                if !folded && draw_item(ui, item, n, editable) {
                                    edit = Some(n);
                                }
                            }
                        }
                    });
                });
                if current_hit == Some(n) {
                    let left = r.response.rect.left() + pad;
                    let hr = egui::Rect::from_x_y_ranges(left - 8.0..=left + w + 8.0, r.response.rect.y_range());
                    ui.painter().rect_stroke(hr, 10, Stroke::new(1.5, theme::ACCENT.gamma_multiply(0.7)), egui::StrokeKind::Outside);
                }
                item.h = r.response.rect.height();
                item.w = w;
            }
            ui.add_space(40.0);
        });
        if let Some(n) = toggle {
            self.items[n].fold_open = !self.items[n].fold_open;
        }
        if let Some(n) = edit {
            if let Kind_::User { text, .. } = &self.items[n].kind {
                self.editing = Some((n, text.clone()));
            }
        }
        if cancel_edit {
            self.editing = None;
        }
        if let Some((n, text)) = resend {
            self.editing = None;
            if !text.is_empty() {
                self.resend(n, text);
            }
        }
    }

    /// Ctrl+F: a search box over the conversation that steps through the messages containing the text.
    fn find_bar(&mut self, ui: &mut egui::Ui) {
        if !self.find.open {
            return;
        }
        let q = self.find.query.trim().to_lowercase();
        if self.find.searched.as_ref() != Some(&(q.clone(), self.items.len())) {
            self.find.hits = if q.is_empty() { vec![] } else { self.items.iter().enumerate().filter(|(_, i)| item_text(&i.kind).to_lowercase().contains(&q)).map(|(n, _)| n).collect() };
            // Start from the newest match, where the reader most likely is.
            self.find.at = self.find.hits.len().saturating_sub(1);
            self.find.jump = !self.find.hits.is_empty();
            self.find.searched = Some((q, self.items.len()));
        }
        let mut close = false;
        let mut step: i32 = 0;
        ui.add_space(8.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            egui::Frame::NONE.fill(theme::SURFACE).stroke(Stroke::new(1.0, theme::BORDER_HI)).corner_radius(9).inner_margin(egui::Margin { left: 8, right: 4, top: 2, bottom: 2 }).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 28.0), Sense::hover());
                    theme::icon::search(ui.painter(), egui::Rect::from_center_size(rect.center(), egui::vec2(18.0, 18.0)), theme::DIM);
                    let r = ui.add(egui::TextEdit::singleline(&mut self.find.query).id(egui::Id::unique("find")).hint_text(RichText::new("Find in chat").color(theme::DIM)).frame(egui::Frame::NONE).desired_width(200.0));
                    if self.find.focus {
                        r.request_focus();
                        self.find.focus = false;
                    }
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        step = if ui.input(|i| i.modifiers.shift) { -1 } else { 1 };
                        r.request_focus();
                    }
                    if r.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                        close = true;
                    }
                    let n = self.find.hits.len();
                    let count = if self.find.query.trim().is_empty() { String::new() } else if n == 0 { "No matches".into() } else { format!("{} of {n}", self.find.at + 1) };
                    ui.add_sized([70.0, 20.0], egui::Label::new(RichText::new(count).size(12.0).color(theme::DIM)));
                    if theme::icon_button(ui, 24.0, "Previous (Shift+Enter)", theme::icon::chevron_up).clicked() {
                        step = -1;
                    }
                    if theme::icon_button(ui, 24.0, "Next (Enter)", theme::icon::chevron_down).clicked() {
                        step = 1;
                    }
                    if theme::icon_button(ui, 24.0, "Close (Esc)", theme::icon::close).clicked() {
                        close = true;
                    }
                });
            });
        });
        let n = self.find.hits.len();
        if step != 0 && n > 0 {
            self.find.at = (self.find.at as i32 + step).rem_euclid(n as i32) as usize;
            self.find.jump = true;
        }
        if close {
            self.find.open = false;
            self.focus_input = true;
        }
    }

    fn login_screen(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.5 - 250.0).max(24.0));
            let (rect, _) = ui.allocate_exact_size(egui::vec2(52.0, 52.0), Sense::hover());
            theme::icon::logo(ui.painter(), rect);
            ui.add_space(18.0);
            ui.label(RichText::new("Welcome to Scout").font(theme::bold(26.0)).color(theme::STRONG));
            ui.add_space(4.0);
            ui.label(RichText::new("A coding agent for your projects, powered by ScoutCode.").size(13.5).color(theme::DIM));
            ui.add_space(26.0);
            egui::Frame::NONE.fill(theme::SURFACE).stroke(Stroke::new(1.0, theme::BORDER)).corner_radius(16).inner_margin(egui::Margin::same(24)).show(ui, |ui| {
                ui.set_width(360.0);
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.add_enabled_ui(!self.login.busy, |ui| {
                    let b = egui::Button::new(RichText::new("Sign in with your browser").font(theme::bold(14.0)).color(theme::ACCENT_INK)).fill(theme::ACCENT).corner_radius(10).min_size(egui::vec2(360.0, 40.0));
                    if ui.add(b).clicked() {
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
                });
                ui.label(RichText::new("No key needed: uses your account's monthly CLI credit.").size(12.0).color(theme::DIM));
                if self.login.busy {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(12.0).color(theme::ACCENT));
                        ui.label(RichText::new("Waiting for the browser…").size(13.0).color(theme::MUTED));
                    });
                    if let Some(u) = &self.login.url {
                        ui.hyperlink_to(RichText::new("Open the sign-in page again").size(12.0).color(theme::ACCENT), u);
                    }
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let lw = (360.0 - 110.0) / 2.0;
                    let (a, _) = ui.allocate_exact_size(egui::vec2(lw, 12.0), Sense::hover());
                    ui.painter().hline(a.x_range(), a.center().y, Stroke::new(1.0, theme::BORDER));
                    ui.add_sized([94.0, 12.0], egui::Label::new(RichText::new("or use an API key").size(11.5).color(theme::DIM)));
                    let (b, _) = ui.allocate_exact_size(egui::vec2(lw, 12.0), Sense::hover());
                    ui.painter().hline(b.x_range(), b.center().y, Stroke::new(1.0, theme::BORDER));
                });
                ui.add_space(6.0);
                ui.add(egui::TextEdit::singleline(&mut self.login.key).password(true).hint_text("sk-…").desired_width(360.0).margin(egui::Margin::symmetric(12, 9)));
                if ui.add_enabled(!self.login.key.trim().is_empty(), egui::Button::new(RichText::new("Save key").size(13.5)).corner_radius(10).min_size(egui::vec2(360.0, 36.0))).clicked() {
                    let k = self.login.key.trim().to_string();
                    config::save_one("apiKey", json!(k));
                    self.set_key(Some(k));
                    self.login = Login::default();
                }
                ui.hyperlink_to(RichText::new("Make a key on your account page").size(12.5).color(theme::ACCENT), "https://scoutcode.dev/account");
                if let Some(e) = &self.login.error {
                    ui.add_space(4.0);
                    ui.colored_label(theme::RED, e);
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    let lw = (360.0 - 150.0) / 2.0;
                    let (a, _) = ui.allocate_exact_size(egui::vec2(lw, 12.0), Sense::hover());
                    ui.painter().hline(a.x_range(), a.center().y, Stroke::new(1.0, theme::BORDER));
                    ui.add_sized([130.0, 12.0], egui::Label::new(RichText::new("or use your OpenRouter key").size(11.5).color(theme::DIM)));
                    let (b, _) = ui.allocate_exact_size(egui::vec2(lw, 12.0), Sense::hover());
                    ui.painter().hline(b.x_range(), b.center().y, Stroke::new(1.0, theme::BORDER));
                });
                ui.add_space(6.0);
                if let Some(k) = self.openrouter_entry(ui, 360.0) {
                    self.add_openrouter_key("OpenRouter".into(), k);
                }
            });
        });
    }

    /// The OpenRouter key field and its save button, under the how-to animation. Returns the key when it's saved.
    fn openrouter_entry(&mut self, ui: &mut egui::Ui, width: f32) -> Option<String> {
        key_guide(ui, width);
        ui.add_space(8.0);
        ui.add(egui::TextEdit::singleline(&mut self.login.or_key).password(true).hint_text("sk-or-…").desired_width(width).margin(egui::Margin::symmetric(12, 9)));
        let ready = !self.login.or_key.trim().is_empty();
        let save = ui.add_enabled(ready, egui::Button::new(RichText::new("Save OpenRouter key").size(13.5)).corner_radius(10).min_size(egui::vec2(width, 36.0)));
        if save.clicked() {
            let k = self.login.or_key.trim().to_string();
            self.login.or_key.clear();
            return Some(k);
        }
        None
    }

}

/// The how-to animation for the OpenRouter key. A placeholder for now: three dots taking turns to bounce.
fn key_guide(ui: &mut egui::Ui, width: f32) {
    egui::Frame::NONE.fill(theme::SURFACE).stroke(Stroke::new(1.0, theme::BORDER)).corner_radius(12).show(ui, |ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 84.0), Sense::hover());
        let t = ui.input(|i| i.time) as f32;
        for i in 0..3 {
            let lift = (t * 3.0 - i as f32 * 0.7).sin().max(0.0) * 12.0;
            let c = rect.center() + egui::vec2((i as f32 - 1.0) * 22.0, -lift);
            ui.painter().circle_filled(c, 6.0, theme::ACCENT);
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    });
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(ev) = self.rx.try_recv() {
            self.handle(ev);
        }
        // Files dropped anywhere on the window are attached to the message being typed.
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        for p in dropped {
            self.attach_path(&p);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Backtick)) {
            self.dock.toggle(Tab::Terminal);
        }
        // The terminal and the editor want these keys for themselves.
        if self.dock.has_keyboard(ctx) {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::F)) {
            self.find.open = true;
            self.find.focus = true;
            self.find.searched = None;
        }
        // Esc closes whatever is open on top before it stops Scout.
        let esc_taken = self.mention.is_some() || self.editing.is_some() || self.deleting.is_some() || self.settings_open || ctx.memory(|m| m.has_focus(egui::Id::unique("find")));
        if ctx.input(|i| i.key_pressed(Key::Escape)) && !esc_taken {
            self.stop();
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(Key::N)) {
            self.new_chat();
        }
        if ctx.input(|i| i.modifiers.shift && i.key_pressed(Key::Tab)) && !self.running {
            let m = self.shared.mode();
            let next = MODES[(MODES.iter().position(|x| *x == m).unwrap_or(0) + 1) % MODES.len()];
            *self.shared.mode.lock().unwrap() = next.into();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.cfg.api_key.is_none() && self.cfg.openrouter_keys.is_empty() {
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(theme::BG)).show(ui, |ui| {
                drag_region(ui, egui::Rect::from_min_size(ui.max_rect().min, egui::vec2(ui.max_rect().width(), WIN_BTN.y)));
                self.login_screen(ui)
            });
            window_chrome(ui.ctx());
            return;
        }
        // The sidebar goes first so it runs the full height of the window.
        if self.sidebar {
            egui::Panel::left("sessions")
                .resizable(true)
                .default_size(264.0)
                .size_range(200.0..=420.0)
                .frame(egui::Frame::NONE.fill(theme::SIDEBAR).inner_margin(egui::Margin::symmetric(10, 0)))
                .show(ui, |ui| self.side_panel(ui));
        }
        egui::Panel::top("top")
            .show_separator_line(false)
            .frame(egui::Frame::NONE.fill(theme::BG).inner_margin(egui::Margin { left: 16, right: 14, top: 5, bottom: 13 }))
            .show(ui, |ui| self.top_bar(ui));
        if self.dock.open {
            // Leave the chat at least a readable column.
            let max = (ui.available_width() - 380.0).max(320.0);
            let cwd = self.cwd();
            egui::Panel::right("dock")
                .resizable(true)
                .default_size(560.0_f32.min(max))
                .size_range(320.0..=max)
                .frame(egui::Frame::NONE.fill(theme::SIDEBAR).stroke(Stroke::new(1.0, theme::BORDER)))
                .show(ui, |ui| self.dock.ui(ui, &cwd));
        }
        egui::Panel::bottom("composer").show_separator_line(false).frame(egui::Frame::NONE.fill(theme::BG).inner_margin(egui::Margin { left: 20, right: 20, top: 4, bottom: 12 })).show(ui, |ui| {
            // A centered column that still lays out left to right inside.
            let full = ui.available_width();
            let w = full.min(MAX_W + 16.0);
            ui.horizontal(|ui| {
                ui.add_space(((full - w) / 2.0).max(0.0));
                ui.vertical(|ui| {
                    ui.set_width(w);
                    self.todos(ui);
                    self.notify_setup_card(ui);
                    self.permission_card(ui);
                    self.plan_card(ui);
                    self.questions_card(ui);
                    self.queue_card(ui);
                    self.input_box(ui);
                    ui.add_space(6.0);
                    self.footer(ui);
                });
            });
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(theme::BG).inner_margin(egui::Margin::symmetric(20, 0))).show(ui, |ui| self.transcript(ui));
        self.settings_window(ui.ctx());
        self.delete_dialog(ui.ctx());
        drop_overlay(ui.ctx());
        window_chrome(ui.ctx());
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        tools::kill_background_shells();
        // A waiting update goes in now. If it can't, it's found again at the next start.
        let _ = self.updater.install();
    }
}


// ---------- widgets ----------

/// Width of the transcript and composer column.
const MAX_W: f32 = 780.0;

/// The verb on the empty screen: "Working in <project>" or "Vibing in <project>".
/// Catchphrases for the empty screen: each is followed by the project's name and a "?".
const SPLASH: [&str; 12] = [
    "Building in",
    "Shipping from",
    "Cooking in",
    "Hacking on",
    "Debugging",
    "Vibing in",
    "What's next in",
    "Tinkering with",
    "Refactoring",
    "Fixing bugs in",
    "Polishing",
    "Dreaming up features for",
];

/// A random catchphrase other than `last`.
fn next_splash(last: usize) -> usize {
    let mut b = [0u8; 2];
    let _ = getrandom::fill(&mut b);
    let n = u16::from_le_bytes(b) as usize % (SPLASH.len() - 1);
    if last < SPLASH.len() && n >= last { n + 1 } else { n }
}

type IconFn<'a> = &'a dyn Fn(&egui::Painter, egui::Rect, Color32);

// ---------- the frameless window's own title bar ----------

const WIN_BTN: egui::Vec2 = egui::vec2(46.0, 42.0);

fn maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

/// The edge or corner under the pointer, for resizing the window by hand now that Windows' frame is gone.
fn resize_edge(ctx: &egui::Context) -> Option<egui::ResizeDirection> {
    use egui::ResizeDirection::*;
    if maximized(ctx) {
        return None;
    }
    let pos = ctx.input(|i| i.pointer.hover_pos())?;
    let r = ctx.content_rect();
    let m = 5.0;
    let (w, e, n, s) = (pos.x - r.left() < m, r.right() - pos.x < m, pos.y - r.top() < m, r.bottom() - pos.y < m);
    Some(match (w, e, n, s) {
        (true, _, true, _) => NorthWest,
        (_, true, true, _) => NorthEast,
        (true, _, _, true) => SouthWest,
        (_, true, _, true) => SouthEast,
        (true, ..) => West,
        (_, true, ..) => East,
        (_, _, true, _) => North,
        (_, _, _, true) => South,
        _ => return None,
    })
}

/// Makes `rect` move the window when dragged and maximize or restore it on double-click.
/// Call it before drawing the widgets in that area, so they still get their own clicks.
fn drag_region(ui: &mut egui::Ui, rect: egui::Rect) {
    let r = ui.interact(rect, ui.scope_id().with("drag_region"), Sense::click_and_drag());
    if r.double_clicked() {
        let m = maximized(ui.ctx());
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!m));
    } else if r.drag_started_by(egui::PointerButton::Primary) && resize_edge(ui.ctx()).is_none() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
}

/// Minimize, maximize and close in the top-right corner, plus resizing from the edges. Drawn above everything else.
fn window_chrome(ctx: &egui::Context) {
    use egui::ResizeDirection::*;
    if let Some(dir) = resize_edge(ctx) {
        ctx.set_cursor_icon(match dir {
            North | South => egui::CursorIcon::ResizeVertical,
            East | West => egui::CursorIcon::ResizeHorizontal,
            NorthWest | SouthEast => egui::CursorIcon::ResizeNwSe,
            NorthEast | SouthWest => egui::CursorIcon::ResizeNeSw,
        });
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
        }
    }
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::unique("window_buttons")).order(egui::Order::Foreground).fixed_pos(egui::pos2(screen.right() - 3.0 * WIN_BTN.x, screen.top())).show(ctx, |ui| {
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        ui.horizontal(|ui| {
            let max = maximized(ctx);
            if window_button(ui, "Minimize", &theme::icon::minimize, false).clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
            let icon: IconFn = if max { &theme::icon::restore } else { &theme::icon::maximize };
            if window_button(ui, if max { "Restore" } else { "Maximize" }, icon, false).clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
            }
            if window_button(ui, "Close", &theme::icon::close, true).clicked() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    });
}

fn window_button(ui: &mut egui::Ui, tip: &str, icon: IconFn, close: bool) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(WIN_BTN, Sense::click());
    let hot = r.hovered();
    if hot {
        ui.painter().rect_filled(rect, 0, if close { Color32::from_rgb(0xc4, 0x2b, 0x1c) } else { theme::SURFACE_HI });
    }
    icon(ui.painter(), rect, if hot { theme::STRONG } else { theme::MUTED });
    r.on_hover_text(tip)
}

/// A title bar icon button that stays lit while what it opens is showing.
fn bar_button(ui: &mut egui::Ui, active: bool, tip: &str, icon: fn(&egui::Painter, egui::Rect, Color32)) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(30.0, 28.0), Sense::click());
    let p = ui.painter();
    if active {
        p.rect_filled(rect, 7, theme::SURFACE_HI);
    } else if r.hovered() {
        p.rect_filled(rect, 7, theme::HOVER);
    }
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    icon(p, egui::Rect::from_center_size(rect.center(), egui::vec2(20.0, 20.0)), if active || r.hovered() { theme::STRONG } else { theme::MUTED });
    r.on_hover_text(tip)
}

/// One line of text, cut with an ellipsis to fit `max` points.
fn one_line(ui: &egui::Ui, text: &str, font: FontId, color: Color32, max: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(text.to_string(), egui::TextFormat { font_id: font, color, ..Default::default() });
    job.wrap = egui::text::TextWrapping::truncate_at_width(max.max(10.0));
    ui.painter().layout_job(job)
}

/// A compact dropdown trigger: optional icon, text and a chevron.
/// The glyph's square inside a 28px box: small, so it sits level with the chips beside it.
fn icon_rect(rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_center_size(rect.center(), egui::vec2(14.0, 14.0))
}

/// A composer tool: a 28px box matching the chips beside it, with a border that brightens on hover.
fn icon_box(ui: &mut egui::Ui, tip: &str, paint: impl FnOnce(&egui::Painter, egui::Rect, Color32)) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), Sense::click());
    let hot = r.hovered();
    let p = ui.painter();
    if hot {
        p.rect(rect, 8, Color32::from_rgb(0x2a, 0x30, 0x3b), Stroke::new(1.0, Color32::from_rgb(0x4a, 0x52, 0x60)), egui::StrokeKind::Inside);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    } else {
        p.rect(rect, 8, theme::SURFACE_HI, Stroke::new(1.0, theme::BORDER_HI), egui::StrokeKind::Inside);
    }
    paint(p, icon_rect(rect), if hot { theme::STRONG } else { theme::TEXT });
    r.on_hover_text(tip)
}

fn chip(ui: &mut egui::Ui, text: &str, color: Color32, lead: Option<IconFn>, enabled: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(13.0), color);
    let lead_w = if lead.is_some() { 18.0 } else { 0.0 };
    let size = egui::vec2(10.0 + lead_w + galley.size().x + 18.0 + 4.0, 28.0);
    let (rect, r) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&r));
    let p = ui.painter();
    // Open, it reads as pressed: filled, outlined, and the chevron flips.
    if open {
        p.rect(rect, 8, theme::SURFACE_HI, Stroke::new(1.0, theme::BORDER_HI), egui::StrokeKind::Inside);
    } else if enabled && r.hovered() {
        p.rect_filled(rect, 8, theme::SURFACE_HI);
    }
    if enabled && r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let mut x = rect.left() + 10.0;
    let fg = if open || r.hovered() { theme::STRONG } else { color };
    if let Some(f) = lead {
        f(p, egui::Rect::from_center_size(egui::pos2(x + 7.0, rect.center().y), egui::vec2(16.0, 16.0)), fg);
        x += lead_w;
    }
    p.galley_with_override_text_color(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, fg);
    let chevron = egui::Rect::from_center_size(egui::pos2(rect.right() - 13.0, rect.center().y + 0.5), egui::vec2(16.0, 16.0));
    if open {
        theme::icon::chevron_up(p, chevron, theme::MUTED);
    } else {
        theme::icon::chevron_down(p, chevron, if enabled && r.hovered() { theme::MUTED } else { theme::DIM });
    }
    r
}

/// The background of a dropdown row: a wash of the accent for the selected one, a lift under the pointer.
/// Returns the color for the row's text.
fn menu_row_bg(ui: &egui::Ui, rect: egui::Rect, selected: bool, hot: bool) -> Color32 {
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 7, theme::MENU_SELECTED);
    }
    if hot {
        p.rect_filled(rect, 7, theme::MENU_HOVER);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if !ui.is_enabled() {
        theme::FAINT
    } else if selected || hot {
        theme::STRONG
    } else {
        theme::TEXT
    }
}

/// A row in a dropdown menu, with a check for the selected one.
fn menu_item(ui: &mut egui::Ui, selected: bool, text: &str) -> egui::Response {
    let w = ui.available_width().max(140.0);
    let (rect, r) = ui.allocate_exact_size(egui::vec2(w, 32.0), Sense::click());
    let color = menu_row_bg(ui, rect, selected, r.hovered());
    let x = rect.left() + 12.0;
    let g = one_line(ui, text, FontId::proportional(13.5), color, rect.right() - 30.0 - x);
    ui.painter().galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, theme::TEXT);
    if selected {
        theme::icon::check(ui.painter(), egui::Rect::from_center_size(egui::pos2(rect.right() - 16.0, rect.center().y), egui::vec2(18.0, 18.0)), theme::ACCENT);
    }
    r
}

/// A menu row with an icon before its text.
fn menu_icon_item(ui: &mut egui::Ui, text: &str, icon: fn(&egui::Painter, egui::Rect, Color32)) -> egui::Response {
    let w = ui.available_width().max(140.0);
    let (rect, r) = ui.allocate_exact_size(egui::vec2(w, 32.0), Sense::click());
    let color = menu_row_bg(ui, rect, false, r.hovered());
    icon(ui.painter(), egui::Rect::from_center_size(egui::pos2(rect.left() + 20.0, rect.center().y), egui::vec2(20.0, 20.0)), if r.hovered() { color } else { theme::MUTED });
    let x = rect.left() + 38.0;
    let g = one_line(ui, text, FontId::proportional(13.5), color, rect.right() - 12.0 - x);
    ui.painter().galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, theme::TEXT);
    r
}

/// A model row: the name over where it runs, a FREE tag if it costs nothing, and a star to keep it in the Starred tab.
/// Returns the row and the star.
fn model_item(ui: &mut egui::Ui, selected: bool, starred: bool, free: bool, text: &str, provider: &str) -> (egui::Response, egui::Response) {
    let w = ui.available_width().max(140.0);
    let (rect, r) = ui.allocate_exact_size(egui::vec2(w, 44.0), Sense::click());
    let star_rect = egui::Rect::from_center_size(egui::pos2(rect.right() - 16.0, rect.center().y), egui::vec2(24.0, 24.0));
    let star = ui.interact(star_rect, r.id.with("star"), Sense::click()).on_hover_text(if starred { "Unstar" } else { "Star to keep it in the Starred tab" });
    let color = menu_row_bg(ui, rect, selected, r.hovered() || star.hovered());
    if selected {
        // The bar on the left edge, like the rail's.
        ui.painter().rect_filled(egui::Rect::from_min_size(rect.left_top() + egui::vec2(0.0, 10.0), egui::vec2(2.5, rect.height() - 20.0)), 2, theme::ACCENT);
    }
    let star_color = if starred { theme::ACCENT } else if star.hovered() { theme::STRONG } else { theme::FAINT };
    theme::icon::star(ui.painter(), star_rect, star_color, starred);
    let x = rect.left() + 12.0;
    let mut text_right = star_rect.left() - 4.0;
    if free {
        let badge = egui::Rect::from_min_max(egui::pos2(text_right - 40.0, rect.center().y - 8.0), egui::pos2(text_right, rect.center().y + 8.0));
        let p = ui.painter();
        p.rect(badge, 4, theme::GREEN.gamma_multiply(0.14), Stroke::new(1.0, theme::GREEN.gamma_multiply(0.55)), egui::StrokeKind::Inside);
        p.text(badge.center(), egui::Align2::CENTER_CENTER, "FREE", theme::bold(9.5), theme::GREEN);
        text_right = badge.left() - 6.0;
    }
    let g = one_line(ui, text, theme::bold(13.5), color, text_right - x);
    ui.painter().galley(egui::pos2(x, rect.top() + 6.0), g, theme::TEXT);
    // Where it runs: the provider's mark and name, dimmed.
    let icon = egui::Rect::from_min_size(egui::pos2(x, rect.bottom() - 19.0), egui::vec2(13.0, 13.0));
    let name = if provider == "openrouter" {
        theme::icon::openrouter(ui.painter(), icon, theme::DIM);
        "OpenRouter"
    } else {
        theme::icon::logo(ui.painter(), icon.shrink(0.5));
        "ScoutCode"
    };
    ui.painter().text(egui::pos2(icon.right() + 6.0, icon.center().y), egui::Align2::LEFT_CENTER, name, FontId::proportional(11.5), theme::DIM);
    (r, star)
}

/// A tab in the model menu's rail: the provider's mark (or a star), with an accent bar when it's the open one.
fn rail_tab(ui: &mut egui::Ui, on: bool, tip: &str, tab: ModelTab) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), Sense::click());
    let r = r.on_hover_text(tip);
    let p = ui.painter();
    if on || r.hovered() {
        p.rect_filled(rect.shrink(2.0), 8, if on { theme::MENU_HOVER } else { theme::HOVER });
    }
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if on {
        p.rect_filled(egui::Rect::from_min_size(egui::pos2(rect.right() + 3.0, rect.top() + 8.0), egui::vec2(2.5, rect.height() - 16.0)), 2, theme::ACCENT);
    }
    let c = if on || r.hovered() { theme::STRONG } else { theme::MUTED };
    let icon = egui::Rect::from_center_size(rect.center(), egui::vec2(22.0, 22.0));
    match tab {
        ModelTab::Starred => theme::icon::star(p, egui::Rect::from_center_size(rect.center(), egui::vec2(30.0, 30.0)), c, true),
        ModelTab::ScoutCode => theme::icon::logo(p, icon),
        ModelTab::OpenRouter => theme::icon::openrouter(p, icon, c),
    }
    r
}

/// A model's stats in a card beside the model menu, level with the `row` under the pointer.
fn model_card(ctx: &egui::Context, m: &ModelInfo, menu: egui::Rect, row: egui::Rect) {
    const W: f32 = 260.0;
    let screen = ctx.content_rect();
    // To the right of the menu, or to its left when there's no room.
    let x = if menu.right() + 8.0 + W + 24.0 <= screen.right() { menu.right() + 8.0 } else { menu.left() - 8.0 - W - 24.0 };
    let y = row.top().min(screen.bottom() - 220.0).max(screen.top() + 8.0);
    egui::Area::new(egui::Id::unique("model_card")).order(egui::Order::Tooltip).interactable(false).fixed_pos(egui::pos2(x, y)).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
            ui.set_width(W);
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(RichText::new(m.label()).font(theme::bold(15.0)).color(theme::STRONG));
            ui.label(RichText::new(&m.id).font(FontId::monospace(11.5)).color(theme::DIM));
            if let Some(d) = &m.description {
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(d).size(12.5).color(theme::MUTED)).wrap());
            }
            let caps: Vec<&str> = [(m.tools, "Tools"), (m.vision, "Vision")].into_iter().filter_map(|(on, s)| (on == Some(true)).then_some(s)).collect();
            let runs_on = match (m.hosted, m.openrouter_id.is_some()) {
                (true, true) => "ScoutCode or OpenRouter",
                (true, false) => "ScoutCode",
                _ => "OpenRouter",
            };
            let stats = [
                ("Context", m.context.map(|n| format!("{} tokens", tokens(n)))),
                ("Cost", m.cost.clone()),
                ("Supports", (!caps.is_empty()).then(|| caps.join(" · "))),
                ("Runs on", Some(runs_on.to_string())),
                ("Providers", (!m.providers.is_empty()).then(|| m.providers.join(", "))),
                ("Tries", (!m.routes_to.is_empty()).then(|| m.routes_to.join(" → "))),
            ];
            if stats.iter().any(|(_, v)| v.is_some()) || m.available == Some(false) {
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(2.0);
            }
            for (k, v) in stats.iter().filter_map(|(k, v)| v.as_ref().map(|v| (k, v))) {
                ui.horizontal_top(|ui| {
                    ui.add_sized(egui::vec2(66.0, 16.0), egui::Label::new(RichText::new(*k).size(12.0).color(theme::DIM)));
                    ui.add(egui::Label::new(RichText::new(v).size(12.5).color(theme::TEXT)).wrap());
                });
            }
            if m.available == Some(false) {
                ui.label(RichText::new("Unavailable right now").size(12.5).color(theme::RED));
            }
        });
    });
}

/// 131072 → "131K", 1048576 → "1M".
fn tokens(n: u64) -> String {
    if n >= 1_000_000 {
        let m = format!("{:.1}", n as f64 / 1e6);
        format!("{}M", m.trim_end_matches(".0"))
    } else {
        format!("{}K", (n as f64 / 1e3).round())
    }
}

/// A full-width sidebar button with an icon; `framed` gives it a border like a primary action.
fn nav_button(ui: &mut egui::Ui, enabled: bool, text: &str, hint: &str, icon: IconFn, framed: bool) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), if enabled { Sense::click() } else { Sense::hover() });
    let hot = enabled && r.hovered();
    let p = ui.painter();
    if framed {
        p.rect(rect, 9, if hot { theme::SURFACE_HI } else { theme::SURFACE }, Stroke::new(1.0, if hot { theme::BORDER_HI } else { theme::BORDER }), egui::StrokeKind::Inside);
    } else if hot {
        p.rect_filled(rect, 9, theme::HOVER);
    }
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let fg = if !enabled { theme::DIM } else if framed || hot { theme::STRONG } else { theme::TEXT };
    icon(p, egui::Rect::from_center_size(egui::pos2(rect.left() + 18.0, rect.center().y), egui::vec2(18.0, 18.0)), if framed { fg } else { theme::MUTED });
    let hint_g = ui.painter().layout_no_wrap(hint.to_string(), FontId::proportional(11.5), theme::FAINT);
    let g = one_line(ui, text, FontId::proportional(13.5), fg, rect.width() - 46.0 - hint_g.size().x);
    ui.painter().galley(egui::pos2(rect.left() + 34.0, rect.center().y - g.size().y / 2.0), g, fg);
    ui.painter().galley(egui::pos2(rect.right() - 10.0 - hint_g.size().x, rect.center().y - hint_g.size().y / 2.0), hint_g, theme::FAINT);
    r
}

/// A project in the sidebar: a chevron, its name, and on hover a new-chat button and a menu.
/// Returns the row, the new-chat button and the menu button.
fn project_row(ui: &mut egui::Ui, here: bool, expanded: bool, enabled: bool, name: &str) -> (egui::Response, egui::Response, egui::Response) {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), Sense::click());
    let btn = |dx: f32| egui::Rect::from_center_size(egui::pos2(rect.right() - dx, rect.center().y), egui::vec2(24.0, 24.0));
    let more = ui.interact(btn(16.0), r.id.with("more"), Sense::click()).on_hover_text("More");
    let plus = ui.interact(btn(42.0), r.id.with("new"), if enabled { Sense::click() } else { Sense::hover() }).on_hover_text("New chat in this project");
    let open_menu = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&more));
    let hot = r.hovered() || plus.hovered() || more.hovered() || open_menu;
    let p = ui.painter();
    if hot {
        p.rect_filled(rect, 8, theme::HOVER);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let c = egui::Rect::from_center_size(egui::pos2(rect.left() + 14.0, rect.center().y), egui::vec2(16.0, 16.0));
    if expanded { theme::icon::chevron_down(p, c, theme::DIM) } else { theme::icon::chevron_right(p, c, theme::DIM) }
    theme::icon::folder(p, egui::Rect::from_center_size(egui::pos2(rect.left() + 34.0, rect.center().y), egui::vec2(18.0, 18.0)), if here { theme::ACCENT } else { theme::MUTED });
    let right = if hot { 60.0 } else { 10.0 };
    let font = if here { theme::bold(13.5) } else { FontId::proportional(13.5) };
    let g = one_line(ui, name, font, if here { theme::STRONG } else { theme::TEXT }, rect.width() - 50.0 - right);
    ui.painter().galley(egui::pos2(rect.left() + 50.0, rect.center().y - g.size().y / 2.0), g, theme::TEXT);
    if hot {
        for (b, rr) in [(&plus, btn(42.0)), (&more, btn(16.0))] {
            if b.hovered() {
                ui.painter().rect_filled(rr, 6, theme::SURFACE_HI);
            }
        }
        theme::icon::plus(ui.painter(), btn(42.0), if plus.hovered() { theme::STRONG } else { theme::MUTED });
        theme::icon::more(ui.painter(), btn(16.0), if more.hovered() { theme::STRONG } else { theme::MUTED });
    }
    (r, plus, more)
}

/// The account at the foot of the sidebar: the username and a chevron. Clicking opens its menu above it.
fn profile_row(ui: &mut egui::Ui, name: &str) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&r));
    let hot = r.hovered() || open;
    let p = ui.painter();
    if open {
        p.rect_filled(rect, 9, theme::SURFACE_HI);
    } else if hot {
        p.rect_filled(rect, 9, theme::HOVER);
    }
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let g = one_line(ui, name, theme::bold(13.5), if hot { theme::STRONG } else { theme::TEXT }, rect.width() - 56.0);
    ui.painter().galley(egui::pos2(rect.left() + 14.0, rect.center().y - g.size().y / 2.0), g, theme::TEXT);
    let chev = egui::Rect::from_center_size(egui::pos2(rect.right() - 16.0, rect.center().y), egui::vec2(16.0, 16.0));
    if open { theme::icon::chevron_down(ui.painter(), chev, theme::MUTED) } else { theme::icon::chevron_up(ui.painter(), chev, theme::DIM) }
    r
}

/// A chat in the sidebar: its title, how long ago it was used, and on hover a menu button in place of the time.
/// Returns the row and the menu button.
fn session_row(ui: &mut egui::Ui, selected: bool, enabled: bool, title: &str, when: &str) -> (egui::Response, egui::Response) {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), if enabled { Sense::click() } else { Sense::hover() });
    let more_rect = egui::Rect::from_center_size(egui::pos2(rect.right() - 16.0, rect.center().y), egui::vec2(24.0, 24.0));
    let more = ui.interact(more_rect, r.id.with("more"), Sense::click()).on_hover_text("More");
    let menu_open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&more));
    let show_more = r.hovered() || more.hovered() || menu_open;
    let hot = enabled && r.hovered() || more.hovered();
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 8, theme::SURFACE_HI);
        p.rect_filled(egui::Rect::from_center_size(egui::pos2(rect.left() + 3.0, rect.center().y), egui::vec2(3.0, 14.0)), 2, theme::ACCENT);
    } else if hot {
        p.rect_filled(rect, 8, theme::HOVER);
    }
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let when_g = ui.painter().layout_no_wrap(when.to_string(), FontId::proportional(11.0), theme::FAINT);
    let g = one_line(ui, title, FontId::proportional(13.5), if selected { theme::STRONG } else if hot { theme::TEXT } else { theme::MUTED }, rect.width() - 30.0 - when_g.size().x);
    ui.painter().galley(egui::pos2(rect.left() + 12.0, rect.center().y - g.size().y / 2.0), g, theme::TEXT);
    if show_more {
        if more.hovered() || menu_open {
            ui.painter().rect_filled(more_rect, 6, theme::SURFACE_HI);
        }
        theme::icon::more(ui.painter(), more_rect, if more.hovered() { theme::STRONG } else { theme::MUTED });
    } else {
        ui.painter().galley(egui::pos2(rect.right() - 10.0 - when_g.size().x, rect.center().y - when_g.size().y / 2.0), when_g, theme::FAINT);
    }
    (r, more)
}

/// A message waiting to be sent: its first line, a status, and buttons drawn by `buttons` on the right.
fn queued_row(ui: &mut egui::Ui, text: &str, files: usize, status: &str, buttons: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.set_height(28.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            buttons(ui);
            ui.label(RichText::new(status).size(11.5).color(theme::DIM));
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let first = text.lines().next().unwrap_or("");
                let shown = match (first.is_empty(), files) {
                    (_, 0) => first.to_string(),
                    (true, n) => format!("{n} attached file{}", if n == 1 { "" } else { "s" }),
                    (false, n) => format!("{first}  (+{n} file{})", if n == 1 { "" } else { "s" }),
                };
                ui.add(egui::Label::new(RichText::new(shown).size(13.0).color(theme::TEXT)).truncate());
            });
        });
    });
}

/// An attached file: an icon and its name, the full path on hover. With `removable`, an × that returns true when
/// clicked.
fn file_chip(ui: &mut egui::Ui, path: &str, removable: bool) -> bool {
    let name = path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(path);
    let g = ui.painter().layout_no_wrap(name.to_string(), FontId::proportional(12.5), theme::TEXT);
    let x_w = if removable { 20.0 } else { 0.0 };
    let (rect, r) = ui.allocate_exact_size(egui::vec2(8.0 + 16.0 + 4.0 + g.size().x.min(220.0) + 8.0 + x_w, 26.0), Sense::hover());
    let p = ui.painter();
    p.rect(rect, 7, theme::SURFACE_HI, Stroke::new(1.0, theme::BORDER), egui::StrokeKind::Inside);
    theme::icon::file(p, egui::Rect::from_center_size(egui::pos2(rect.left() + 16.0, rect.center().y), egui::vec2(16.0, 16.0)), theme::MUTED);
    p.with_clip_rect(rect.shrink(2.0)).galley(egui::pos2(rect.left() + 28.0, rect.center().y - g.size().y / 2.0), g, theme::TEXT);
    r.on_hover_text(path);
    if !removable {
        return false;
    }
    let x_rect = egui::Rect::from_center_size(egui::pos2(rect.right() - 12.0, rect.center().y), egui::vec2(18.0, 18.0));
    let x = ui.interact(x_rect, ui.scope_id().with(("chip", path)), Sense::click()).on_hover_text("Remove");
    if x.hovered() {
        ui.painter().rect_filled(x_rect, 5, theme::BORDER_HI);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    theme::icon::close(ui.painter(), x_rect, if x.hovered() { theme::STRONG } else { theme::DIM });
    x.clicked()
}

/// A file in the @ picker: its name, then its folder dimmed.
fn mention_row(ui: &mut egui::Ui, selected: bool, path: &str) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::click());
    if selected || r.hovered() {
        ui.painter().rect_filled(rect, 6, theme::SURFACE_HI);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let (dir, name) = match path.rfind('/') {
        Some(i) => (&path[..i], &path[i + 1..]),
        None => ("", path),
    };
    theme::icon::file(ui.painter(), egui::Rect::from_center_size(egui::pos2(rect.left() + 14.0, rect.center().y), egui::vec2(16.0, 16.0)), theme::MUTED);
    let ng = ui.painter().layout_no_wrap(name.to_string(), FontId::proportional(13.5), if selected { theme::STRONG } else { theme::TEXT });
    let nw = ng.size().x;
    ui.painter().galley(egui::pos2(rect.left() + 30.0, rect.center().y - ng.size().y / 2.0), ng, theme::TEXT);
    if !dir.is_empty() {
        let dg = one_line(ui, dir, FontId::proportional(12.0), theme::DIM, rect.right() - 10.0 - (rect.left() + 40.0 + nw));
        ui.painter().galley(egui::pos2(rect.left() + 40.0 + nw, rect.center().y - dg.size().y / 2.0), dg, theme::DIM);
    }
    r
}

/// Editing a sent message in place. Some(true) to resend, Some(false) to cancel.
fn edit_box(ui: &mut egui::Ui, text: &mut String) -> Option<bool> {
    let mut answer = None;
    ui.add_space(18.0);
    egui::Frame::NONE.fill(theme::SURFACE).stroke(Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.4))).corner_radius(14).inner_margin(egui::Margin::symmetric(14, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let id = egui::Id::unique("edit_message");
        let r = ui.add(egui::TextEdit::multiline(text).id(id).desired_rows(2).desired_width(f32::INFINITY).frame(egui::Frame::NONE).font(FontId::proportional(theme::BODY)));
        if !r.has_focus() && ui.memory(|m| m.focused().is_none()) {
            r.request_focus();
        }
        if r.has_focus() {
            if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
                answer = Some(true);
            }
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                answer = Some(false);
            }
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Files Scout changed after this message are put back first.").size(11.5).color(theme::DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::primary_button(ui, "Send").on_hover_text("Ctrl+Enter").clicked() {
                    answer = Some(true);
                }
                if theme::ghost_button(ui, "Cancel").on_hover_text("Esc").clicked() {
                    answer = Some(false);
                }
            });
        });
    });
    ui.add_space(16.0);
    answer
}

/// A dim cover with a hint while files are dragged over the window.
fn drop_overlay(ctx: &egui::Context) {
    if !ctx.input(|i| !i.raw.hovered_files.is_empty()) {
        return;
    }
    let screen = ctx.content_rect();
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop_overlay")));
    p.rect_filled(screen, 0, Color32::from_black_alpha(170));
    let inner = screen.shrink(24.0);
    p.rect_stroke(inner, 16, Stroke::new(2.0, theme::ACCENT.gamma_multiply(0.7)), egui::StrokeKind::Inside);
    p.text(inner.center(), egui::Align2::CENTER_CENTER, "Drop to attach to your message", theme::bold(20.0), theme::STRONG);
}

/// A section in the Settings sidebar.
fn settings_nav(ui: &mut egui::Ui, selected: bool, text: &str, icon: fn(&egui::Painter, egui::Rect, Color32)) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect(rect, 8, theme::SURFACE_HI, Stroke::new(1.0, theme::BORDER_HI), egui::StrokeKind::Inside);
    } else if r.hovered() {
        p.rect_filled(rect, 8, theme::HOVER);
    }
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let fg = if selected || r.hovered() { theme::STRONG } else { theme::TEXT };
    icon(p, egui::Rect::from_center_size(egui::pos2(rect.left() + 18.0, rect.center().y), egui::vec2(20.0, 20.0)), if selected { theme::ACCENT } else { theme::MUTED });
    p.text(egui::pos2(rect.left() + 38.0, rect.center().y), egui::Align2::LEFT_CENTER, text, FontId::proportional(14.0), fg);
    r
}

fn settings_heading(ui: &mut egui::Ui, title: &str, sub: &str) {
    ui.label(RichText::new(title).font(theme::bold(18.0)).color(theme::STRONG));
    ui.add_space(6.0);
    ui.add(egui::Label::new(RichText::new(sub).size(13.0).color(theme::MUTED)).wrap());
    ui.add_space(14.0);
}

/// A setting: its name and a line about it on the left, `control` on the right, and a rule under it.
/// What a row's buttons do to an OpenRouter key.
#[derive(Clone, Copy)]
enum KeyEdit {
    Up,
    Down,
    Remove,
}

/// An OpenRouter key shown without its middle: the start and end are enough to tell keys apart.
fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 12 {
        return "…".into();
    }
    format!("{}…{}", chars[..8].iter().collect::<String>(), chars[chars.len() - 4..].iter().collect::<String>())
}

fn settings_row(ui: &mut egui::Ui, title: &str, sub: &str, control: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(60.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            control(ui);
            ui.add_space(16.0);
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.add_space(12.0);
                ui.label(RichText::new(title).size(14.5).color(theme::STRONG));
                ui.add_space(3.0);
                ui.add(egui::Label::new(RichText::new(sub).size(12.5).color(theme::DIM)).wrap());
                ui.add_space(12.0);
            });
        });
    });
    let (rule, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rule, 0, theme::BORDER);
}

/// Reasoning effort as a bar: a dot for each level, a thumb on the chosen one, and its name above.
/// An on/off switch.
fn toggle(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let (rect, mut r) = ui.allocate_exact_size(egui::vec2(38.0, 22.0), Sense::click());
    if r.clicked() {
        *on = !*on;
        r.mark_changed();
    }
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let t = ui.ctx().animate_bool(r.id, *on);
    let p = ui.painter();
    let track = if *on { theme::ACCENT } else { theme::SURFACE_HI };
    p.rect(rect, 11, track, Stroke::new(1.0, if *on { theme::ACCENT } else { theme::BORDER_HI }), egui::StrokeKind::Inside);
    let x = egui::lerp(rect.left() + 11.0..=rect.right() - 11.0, t);
    p.circle_filled(egui::pos2(x, rect.center().y), 8.0, if *on { theme::ACCENT_INK } else { theme::MUTED });
    r
}

/// A number with its label and a smaller line under it.
fn stat_tile(ui: &mut egui::Ui, w: f32, label: &str, value: &str, sub: &str) {
    egui::Frame::NONE.fill(theme::SURFACE).stroke(Stroke::new(1.0, theme::BORDER)).corner_radius(10).inner_margin(egui::Margin::symmetric(14, 12)).show(ui, |ui| {
        // The caller's row is left to right, so stack the lines in their own column.
        ui.vertical(|ui| {
            ui.set_width(w - 28.0);
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(RichText::new(label).size(12.0).color(theme::DIM));
            ui.add(egui::Label::new(RichText::new(value).font(theme::bold(18.0)).color(theme::STRONG)).wrap_mode(egui::TextWrapMode::Extend));
            ui.label(RichText::new(sub).size(12.0).color(theme::MUTED));
        });
    });
}

/// 950 → "950", 12_300 → "12.3K", 4_500_000 → "4.5M".
fn short_tokens(n: u64) -> String {
    let f = |v: f64, unit: &str| format!("{}{unit}", format!("{v:.1}").trim_end_matches(".0"));
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => f(n as f64 / 1e3, "K"),
        _ => f(n as f64 / 1e6, "M"),
    }
}

/// "2026-09-30" → "Sep 30".
fn short_date(day: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let mut parts = day.split('-').skip(1).map(|p| p.parse::<usize>().unwrap_or(0));
    let (m, d) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
    format!("{} {d}", MONTHS.get(m.wrapping_sub(1)).unwrap_or(&"?"))
}

/// Tokens per day as columns of squares, as many days as fit, ending today. Hover a column for its numbers.
fn usage_chart(ui: &mut egui::Ui, log: &crate::usage::Log) {
    const ROWS: usize = 16;
    const CELL: f32 = 14.0;
    const GAP: f32 = 4.0;
    egui::Frame::NONE.fill(theme::SIDEBAR).stroke(Stroke::new(1.0, theme::BORDER)).corner_radius(10).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
        // Header: a hatched strip with the title and the legend.
        let (head, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 38.0), Sense::hover());
        let p = ui.painter().with_clip_rect(head);
        let mut x = head.left() - head.height();
        while x < head.right() {
            p.line_segment([egui::pos2(x, head.bottom()), egui::pos2(x + head.height(), head.top())], Stroke::new(1.0, Color32::from_white_alpha(6)));
            x += 7.0;
        }
        p.line_segment([egui::pos2(head.left() + 16.0, head.center().y), egui::pos2(head.left() + 30.0, head.center().y)], Stroke::new(1.0, theme::DIM));
        p.text(egui::pos2(head.left() + 38.0, head.center().y), egui::Align2::LEFT_CENTER, "Tokens per day", FontId::monospace(12.5), theme::TEXT);
        let legend = egui::Rect::from_center_size(egui::pos2(head.right() - 62.0, head.center().y), egui::vec2(92.0, 24.0));
        p.rect(legend, 5, theme::SURFACE, Stroke::new(1.0, theme::BORDER), egui::StrokeKind::Inside);
        p.rect_filled(egui::Rect::from_center_size(egui::pos2(legend.left() + 12.0, legend.center().y), egui::vec2(8.0, 8.0)), 1, theme::ACCENT);
        p.text(egui::pos2(legend.left() + 22.0, legend.center().y), egui::Align2::LEFT_CENTER, "Tokens", FontId::monospace(12.0), theme::TEXT);
        ui.painter().hline(head.x_range(), head.bottom(), Stroke::new(1.0, theme::BORDER));

        let inner_w = ui.available_width() - 32.0;
        let n = (((inner_w + GAP) / (CELL + GAP)).floor() as usize).max(7);
        let days = crate::usage::last_days(log, n);
        let max = days.iter().map(|(_, d)| d.tokens).max().unwrap_or(0).max(1);
        let grid_w = n as f32 * (CELL + GAP) - GAP;
        let grid_h = ROWS as f32 * (CELL + GAP) - GAP;
        ui.add_space(14.0);
        let (area, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), grid_h), Sense::hover());
        let left = area.left() + (area.width() - grid_w) / 2.0;
        let pointer = ui.input(|i| i.pointer.hover_pos());
        for (i, (day, d)) in days.iter().enumerate() {
            let cx = left + i as f32 * (CELL + GAP);
            let col = egui::Rect::from_min_size(egui::pos2(cx, area.top()), egui::vec2(CELL, grid_h));
            let hot = pointer.is_some_and(|p| col.expand2(egui::vec2(GAP / 2.0, 0.0)).contains(p));
            // Any use at all shows at least one square.
            let filled = if d.tokens == 0 { 0 } else { ((d.tokens as f64 / max as f64 * ROWS as f64).round() as usize).clamp(1, ROWS) };
            for row in 0..ROWS {
                let y = area.bottom() - CELL - row as f32 * (CELL + GAP);
                let cell = egui::Rect::from_min_size(egui::pos2(cx, y), egui::vec2(CELL, CELL));
                let color = if row < filled { if hot { theme::ACCENT_HI } else { theme::ACCENT } } else if hot { theme::SURFACE_HI } else { theme::SURFACE };
                ui.painter().rect_filled(cell, 2, color);
            }
            if hot {
                let id = ui.make_persistent_id(("usage_day", i));
                egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), id, egui::PopupAnchor::Pointer).show(|ui| {
                    ui.label(RichText::new(short_date(day)).font(theme::bold(13.0)).color(theme::STRONG));
                    ui.label(RichText::new(format!("{} tokens", short_tokens(d.tokens))).size(12.5).color(theme::TEXT));
                    ui.label(RichText::new(format!("{} requests · ${:.2}", d.requests, d.cost)).size(12.0).color(theme::DIM));
                });
            }
        }
        // Dates under the first, middle and last columns.
        ui.add_space(10.0);
        let (labels, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 16.0), Sense::hover());
        let font = FontId::monospace(11.5);
        let p = ui.painter();
        p.text(egui::pos2(left, labels.center().y), egui::Align2::LEFT_CENTER, short_date(&days[0].0), font.clone(), theme::DIM);
        let mid = n / 2;
        p.text(egui::pos2(left + mid as f32 * (CELL + GAP) + CELL / 2.0, labels.center().y), egui::Align2::CENTER_CENTER, short_date(&days[mid].0), font.clone(), theme::DIM);
        p.text(egui::pos2(left + grid_w, labels.center().y), egui::Align2::RIGHT_CENTER, "today", font, theme::DIM);
        ui.add_space(12.0);
    });
}

fn modal_frame() -> egui::Frame {
    egui::Frame::NONE.fill(theme::SURFACE).stroke(Stroke::new(1.0, theme::BORDER_HI)).corner_radius(14).inner_margin(egui::Margin::same(22))
}

fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE
        .fill(theme::SURFACE)
        .stroke(Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.4)))
        .corner_radius(14)
        .inner_margin(egui::Margin::symmetric(16, 14))
        .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(90) })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    ui.add_space(10.0);
}

fn card_header(ui: &mut egui::Ui, title: &str, hint: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 18.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 4.0, theme::ACCENT);
        ui.painter().circle_filled(rect.center(), 7.0, theme::ACCENT.gamma_multiply(0.18));
        ui.label(RichText::new(title).font(theme::bold(14.5)).color(theme::STRONG));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(hint).size(11.5).color(theme::DIM));
        });
    });
}

// ---------- items ----------

fn todo_list(ui: &mut egui::Ui, todos: &[Todo]) {
    ui.spacing_mut().item_spacing.y = 2.0;
    for t in todos {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 20.0), Sense::hover());
            let c = rect.center();
            let p = ui.painter();
            match t.status.as_str() {
                "completed" => {
                    p.circle_filled(c, 7.0, theme::GREEN.gamma_multiply(0.2));
                    theme::icon::check(p, egui::Rect::from_center_size(c, egui::vec2(16.0, 16.0)), theme::GREEN);
                }
                "in_progress" => {
                    p.circle_stroke(c, 6.5, Stroke::new(1.5, theme::ACCENT));
                    p.circle_filled(c, 3.0, theme::ACCENT);
                }
                _ => {
                    p.circle_stroke(c, 6.5, Stroke::new(1.2, theme::FAINT));
                }
            }
            let mut text = RichText::new(&t.content).size(13.5);
            text = match t.status.as_str() {
                "completed" => text.color(theme::DIM).strikethrough(),
                "in_progress" => text.color(theme::STRONG),
                _ => text.color(theme::MUTED),
            };
            ui.add(egui::Label::new(text).wrap());
        });
    }
}

fn show_display(ui: &mut egui::Ui, d: &Display) {
    match d {
        Display::Text(t) => {
            let lines: Vec<&str> = t.lines().collect();
            let shown = if lines.len() > 400 { format!("{}\n… {} more lines", lines[..400].join("\n"), lines.len() - 400) } else { t.clone() };
            egui::Frame::NONE.fill(theme::CODE_BG).stroke(Stroke::new(1.0, theme::BORDER)).corner_radius(8).inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(RichText::new(shown).font(FontId::monospace(12.5)).color(theme::MUTED).line_height(Some(18.0))).wrap());
            });
        }
        Display::Todos(t) => todo_list(ui, t),
        Display::Diffs(d) => {
            for (i, d) in d.iter().enumerate() {
                if i > 0 {
                    ui.add_space(6.0);
                }
                show_display(ui, d);
            }
        }
        Display::Diff { path, lines, created } => {
            let (add, del) = diff::stats(lines);
            let shown = if *created { lines.iter().map(Some).collect::<Vec<_>>() } else { diff::hunks(lines, 3) };
            egui::Frame::NONE.fill(theme::CODE_BG).stroke(Stroke::new(1.0, theme::BORDER)).corner_radius(8).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                // Header: file name and counts.
                egui::Frame::NONE.fill(theme::SURFACE).corner_radius(egui::CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }).inner_margin(egui::Margin::symmetric(12, 7)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(path).font(FontId::monospace(12.5)).color(theme::STRONG));
                        if *created {
                            ui.label(RichText::new("new file").size(11.5).color(theme::DIM));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(format!("-{del}")).font(FontId::monospace(12.0)).color(theme::RED));
                            ui.label(RichText::new(format!("+{add}")).font(FontId::monospace(12.0)).color(theme::GREEN));
                        });
                    });
                });
                let (sep, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
                ui.painter().rect_filled(sep, 0, theme::BORDER);
                diff_line_rows(ui, &shown);
            });
        }
    }
}

/// A diff on its own, without the file header: changed lines with context, or all of a new file.
pub fn diff_rows(ui: &mut egui::Ui, lines: &[DiffLine], created: bool) {
    let shown = if created { lines.iter().map(Some).collect::<Vec<_>>() } else { diff::hunks(lines, 3) };
    egui::Frame::NONE.fill(theme::CODE_BG).stroke(Stroke::new(1.0, theme::BORDER)).corner_radius(8).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        diff_line_rows(ui, &shown);
    });
}

/// Numbered, colored diff lines; None is a gap between hunks.
fn diff_line_rows(ui: &mut egui::Ui, shown: &[Option<&DiffLine>]) {
    ui.add_space(4.0);
    let font = FontId::monospace(12.5);
    let row_h = ui.fonts_mut(|f| f.row_height(&font)) + 3.0;
    for l in shown.iter().take(600) {
        let Some(l) = l else {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), row_h), Sense::hover());
            ui.painter().text(rect.left_center() + egui::vec2(14.0, 0.0), egui::Align2::LEFT_CENTER, "⋯", font.clone(), theme::FAINT);
            continue;
        };
        let (bg, sign, color, bar) = match l.kind {
            Kind::Add => (theme::ADD_BG, "+", Color32::from_rgb(0xb5, 0xf0, 0xc8), Some(theme::GREEN)),
            Kind::Del => (theme::DEL_BG, "-", Color32::from_rgb(0xf6, 0xb8, 0xb8), Some(theme::RED)),
            Kind::Same => (Color32::TRANSPARENT, " ", theme::MUTED, None),
        };
        let no = l.new_no.or(l.old_no).map(|n| format!("{n:>4}")).unwrap_or_else(|| "    ".into());
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), row_h), Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 0.0, bg);
        if let Some(b) = bar {
            p.rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())), 0.0, b);
        }
        let num = p.layout_no_wrap(no, font.clone(), theme::FAINT);
        p.galley(rect.min + egui::vec2(8.0, 1.5), num, theme::FAINT);
        let galley = p.layout_no_wrap(format!("{sign} {}", l.text.replace('\t', "    ")), font.clone(), color);
        p.with_clip_rect(rect).galley(rect.min + egui::vec2(52.0, 1.5), galley, color);
    }
    if shown.len() > 600 {
        ui.label(RichText::new(format!("   … {} more lines", shown.len() - 600)).font(font).color(theme::DIM));
    }
    ui.add_space(4.0);
}

/// A one-line tool call: status, verb, argument and summary, with a chevron when there's output to open.
fn tool_row(ui: &mut egui::Ui, name: &str, label: &str, result: &Option<ToolResult>, open: bool) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), Sense::click());
    let can_open = result.is_some();
    let hot = r.hovered() && can_open;
    let p = ui.painter();
    if hot || open {
        p.rect_filled(rect, 8, if hot { theme::HOVER } else { Color32::TRANSPARENT });
    }
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let icon = egui::Rect::from_center_size(egui::pos2(rect.left() + 13.0, rect.center().y), egui::vec2(18.0, 18.0));
    match result {
        None => egui::Spinner::new().size(12.0).color(theme::ACCENT).paint_at(ui, egui::Rect::from_center_size(icon.center(), egui::vec2(12.0, 12.0))),
        Some(res) if res.is_error => {
            p.circle_filled(icon.center(), 8.0, theme::RED.gamma_multiply(0.18));
            theme::icon::cross(p, icon, theme::RED);
        }
        Some(_) => {
            p.circle_filled(icon.center(), 8.0, theme::GREEN.gamma_multiply(0.15));
            theme::icon::check(p, icon, theme::GREEN);
        }
    }
    let short: String = label.lines().next().unwrap_or("").chars().take(160).collect();
    // Separate galleys on one baseline: a single job would line up the fonts' boxes, not their text.
    let baseline = |g: &egui::Galley| g.rows.first().map_or(g.size().y * 0.8, |r| r.pos.y + r.row.glyphs.first().map_or(r.row.size.y * 0.8, |gl| gl.pos.y));
    let name_g = ui.painter().layout_no_wrap(tool_title(name).to_string(), theme::bold(13.5), theme::STRONG);
    let base_y = rect.center().y - name_g.size().y / 2.0 + baseline(&name_g);
    let mut x = rect.left() + 30.0;
    let right = rect.right() - 30.0;
    let summary = result.as_ref().and_then(|r| r.summary.clone()).map(|s| (s, result.as_ref().is_some_and(|r| r.is_error)));
    let summary_g = summary.map(|(s, err)| one_line(ui, &format!("·  {s}"), FontId::proportional(12.5), if err { theme::RED } else { theme::DIM }, 260.0));
    let label_max = right - x - name_g.size().x - 8.0 - summary_g.as_ref().map_or(0.0, |g| g.size().x + 8.0);
    let label_g = one_line(ui, &short, FontId::monospace(12.5), theme::MUTED, label_max);
    for (g, gap) in [(Some(name_g), 8.0), (Some(label_g), 8.0), (summary_g, 0.0)] {
        let Some(g) = g else { continue };
        let w = g.size().x;
        let y = base_y - baseline(&g);
        ui.painter().galley(egui::pos2(x, y), g, theme::TEXT);
        x += w + gap;
    }
    if can_open && (hot || open) {
        let c = egui::Rect::from_center_size(egui::pos2(rect.right() - 14.0, rect.center().y), egui::vec2(18.0, 18.0));
        if open { theme::icon::chevron_down(ui.painter(), c, theme::MUTED) } else { theme::icon::chevron_right(ui.painter(), c, theme::MUTED) }
    }
    r
}

/// The header of a folded run of tool calls: a check, the count, what they did, and a chevron. Returns true when clicked.
fn tool_group_header(ui: &mut egui::Ui, g: &ToolGroup) -> bool {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), Sense::click());
    let hot = r.hovered();
    if hot {
        ui.painter().rect_filled(rect, 8, theme::HOVER);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let icon = egui::Rect::from_center_size(egui::pos2(rect.left() + 13.0, rect.center().y), egui::vec2(18.0, 18.0));
    let p = ui.painter();
    p.circle_filled(icon.center(), 8.0, theme::GREEN.gamma_multiply(0.15));
    theme::icon::check(p, icon, theme::GREEN);
    let title = p.layout_no_wrap(format!("{} tool calls", g.count), theme::bold(13.0), if hot { theme::STRONG } else { theme::TEXT });
    let title_h = title.size().y;
    let mut x = rect.left() + 30.0;
    let y = rect.center().y - title_h / 2.0;
    x += title.size().x + 8.0;
    p.galley(egui::pos2(rect.left() + 30.0, y), title, theme::TEXT);
    let right = rect.right() - 30.0;
    let summary = one_line(ui, &format!("·  {}", g.summary), FontId::proportional(12.5), theme::DIM, (right - x - 60.0).max(40.0));
    let summary_y = rect.center().y - summary.size().y / 2.0;
    ui.painter().galley(egui::pos2(x, summary_y), summary, theme::DIM);
    if g.failed > 0 {
        let failed = ui.painter().layout_no_wrap(format!("·  {} failed", g.failed), FontId::proportional(12.5), theme::RED);
        let failed_x = right - failed.size().x - 8.0;
        ui.painter().galley(egui::pos2(failed_x, summary_y), failed, theme::RED);
    }
    let c = egui::Rect::from_center_size(egui::pos2(rect.right() - 14.0, rect.center().y), egui::vec2(18.0, 18.0));
    if g.open { theme::icon::chevron_down(ui.painter(), c, theme::MUTED) } else { theme::icon::chevron_right(ui.painter(), c, theme::MUTED) }
    r.clicked()
}

/// Draws one transcript item. Returns true when the pencil on a message of the user's was clicked.
fn draw_item(ui: &mut egui::Ui, item: &mut Item, n: usize, editable: bool) -> bool {
    let mut edit = false;
    match &mut item.kind {
        Kind_::User { text, files } => {
            ui.add_space(18.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                let bubble = egui::Frame::NONE.fill(theme::USER_BG).stroke(Stroke::new(1.0, theme::BORDER)).corner_radius(egui::CornerRadius { nw: 16, ne: 16, sw: 16, se: 4 }).inner_margin(egui::Margin::symmetric(16, 11)).show(ui, |ui| {
                    ui.set_max_width(ui.available_width() * 0.8);
                    if !text.is_empty() {
                        ui.add(egui::Label::new(RichText::new(text.as_str()).size(theme::BODY).color(theme::STRONG)).wrap());
                    }
                    if !files.is_empty() {
                        if !text.is_empty() {
                            ui.add_space(8.0);
                        }
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                            for f in files.iter() {
                                file_chip(ui, f, false);
                            }
                        });
                    }
                });
                // A pencil to the bubble's left while the pointer is over the message.
                let b = bubble.response.rect;
                let zone = egui::Rect::from_min_max(egui::pos2(b.left() - 44.0, b.top()), b.max);
                if editable && ui.rect_contains_pointer(zone) {
                    let pr = egui::Rect::from_center_size(egui::pos2(b.left() - 20.0, b.bottom() - 14.0), egui::vec2(26.0, 26.0));
                    let r = ui.interact(pr, ui.scope_id().with(("edit", n)), Sense::click()).on_hover_text("Edit and send again");
                    if r.hovered() {
                        ui.painter().rect_filled(pr, 7, theme::SURFACE_HI);
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    theme::icon::pencil(ui.painter(), pr, if r.hovered() { theme::STRONG } else { theme::DIM });
                    edit = r.clicked();
                }
            });
            ui.add_space(16.0);
        }
        Kind_::Assistant { text, blocks, dirty, reasoning, show_reasoning } => {
            if *dirty || (blocks.is_empty() && !text.is_empty()) {
                *blocks = markdown::parse(text);
                *dirty = false;
            }
            ui.add_space(4.0);
            if !reasoning.is_empty() {
                // "Reasoning", then the model's latest thought beside it while it's collapsed.
                let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), Sense::click());
                let hot = r.hovered();
                if hot {
                    ui.painter().rect_filled(rect, 7, theme::HOVER);
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let c = egui::Rect::from_center_size(egui::pos2(rect.left() + 13.0, rect.center().y), egui::vec2(18.0, 18.0));
                let p = ui.painter();
                if *show_reasoning { theme::icon::chevron_down(p, c, theme::DIM) } else { theme::icon::chevron_right(p, c, theme::DIM) }
                let title = p.layout_no_wrap("Reasoning".into(), theme::bold(13.0), if hot { theme::STRONG } else { theme::TEXT });
                let mut x = rect.left() + 28.0;
                let title_y = rect.center().y - title.size().y / 2.0;
                x += title.size().x + 10.0;
                p.galley(egui::pos2(rect.left() + 28.0, title_y), title, theme::TEXT);
                if !*show_reasoning {
                    let latest = reasoning.trim().lines().rev().find(|l| !l.trim().is_empty()).unwrap_or_default().trim();
                    let preview = one_line(ui, latest, FontId::proportional(13.0), theme::DIM, rect.right() - x - 14.0);
                    let preview_y = rect.center().y - preview.size().y / 2.0;
                    ui.painter().galley(egui::pos2(x, preview_y), preview, theme::DIM);
                }
                if r.clicked() {
                    *show_reasoning = !*show_reasoning;
                }
                if *show_reasoning {
                    egui::Frame::NONE.inner_margin(egui::Margin { left: 26, right: 0, top: 2, bottom: 6 }).show(ui, |ui| {
                        let r = ui.add(egui::Label::new(RichText::new(reasoning.as_str()).size(13.0).color(theme::DIM)).wrap());
                        ui.painter().vline(r.rect.left() - 13.0, r.rect.y_range(), Stroke::new(1.0, theme::BORDER_HI));
                    });
                }
                ui.add_space(6.0);
            }
            markdown::show(ui, blocks);
            ui.add_space(10.0);
        }
        Kind_::Tool { name, label, progress, result, open, .. } => {
            if tool_row(ui, name, label, result, *open).clicked() && result.is_some() {
                *open = !*open;
            }
            if result.is_none() && !progress.is_empty() {
                egui::Frame::NONE.inner_margin(egui::Margin { left: 30, right: 0, top: 0, bottom: 4 }).show(ui, |ui| {
                    ui.label(RichText::new(progress.as_str()).font(FontId::monospace(12.0)).color(theme::DIM));
                });
            }
            if *open {
                if let Some(res) = result {
                    egui::Frame::NONE.inner_margin(egui::Margin { left: 30, right: 0, top: 2, bottom: 8 }).show(ui, |ui| match &res.display {
                        Some(d) => show_display(ui, d),
                        None => show_display(ui, &Display::Text(res.output.clone())),
                    });
                }
            }
            ui.add_space(1.0);
        }
        Kind_::Notice(t) => {
            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                egui::Frame::NONE.fill(theme::SURFACE).corner_radius(20).inner_margin(egui::Margin::symmetric(12, 5)).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(t.as_str()).size(12.5).color(theme::MUTED)).wrap());
                });
            });
            ui.add_space(8.0);
        }
        Kind_::Error(t) => {
            ui.add_space(8.0);
            egui::Frame::NONE.fill(theme::DEL_BG).stroke(Stroke::new(1.0, theme::RED.gamma_multiply(0.35))).corner_radius(10).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 20.0), Sense::hover());
                    theme::icon::cross(ui.painter(), rect, theme::RED);
                    ui.add(egui::Label::new(RichText::new(t.as_str()).color(Color32::from_rgb(0xff, 0xc9, 0xc9))).wrap());
                });
            });
            ui.add_space(8.0);
        }
    }
    edit
}

/// The text Ctrl+F looks through in an item.
fn item_text(k: &Kind_) -> String {
    match k {
        Kind_::User { text, files } => format!("{text}\n{}", files.join("\n")),
        Kind_::Assistant { text, .. } => text.clone(),
        Kind_::Tool { label, result, .. } => format!("{label}\n{}", result.as_ref().map(|r| r.output.as_str()).unwrap_or("")),
        Kind_::Notice(t) | Kind_::Error(t) => t.clone(),
    }
}

/// What the user typed in a saved message, and the files attached to it.
fn user_parts(content: &Content) -> (String, Vec<String>) {
    let t = content.text();
    attach::split(crate::prompt::strip_mode_note(&t))
}

fn tool_title(name: &str) -> &str {
    match name {
        "view" => "Read",
        "write_file" => "Write",
        "edit_file" | "multi_edit" => "Edit",
        "apply_patch" => "Patch",
        "list_dir" => "List",
        "glob" => "Find",
        "grep" => "Search",
        "bash" => "Run",
        "shell_output" => "Output",
        "kill_shell" => "Stop",
        "web_fetch" => "Fetch",
        "todo_write" => "Todos",
        "task" => "Task",
        "present_plan" => "Plan",
        "ask_questions" => "Ask",
        n => n,
    }
}

/// Transcript items for a saved conversation. Tool results show their text output; diffs aren't stored.
fn rebuild(s: &Session) -> Vec<Item> {
    let results: HashMap<&str, &str> = s
        .messages
        .iter()
        .filter_map(|m| if let Message::Tool { tool_call_id, content } = m { Some((tool_call_id.as_str(), content.as_str())) } else { None })
        .collect();
    let mut items = vec![];
    for m in &s.messages {
        match m {
            Message::User { content } => {
                let (text, files) = user_parts(content);
                items.push(Item::new(Kind_::User { text, files }));
            }
            Message::Assistant { content, tool_calls, .. } => {
                if let Some(c) = content.as_ref().filter(|c| !c.is_empty()) {
                    items.push(Item::new(Kind_::Assistant { text: c.clone(), blocks: markdown::parse(c), dirty: false, reasoning: String::new(), show_reasoning: false }));
                }
                for c in tool_calls.iter().flatten() {
                    let args: Value = serde_json::from_str(&c.function.arguments).unwrap_or_default();
                    let out = results.get(c.id.as_str()).map(|o| crate::prompt::strip_mode_note(o).to_string());
                    items.push(Item::new(Kind_::Tool {

                        name: c.function.name.clone(),
                        label: tools::label(&c.function.name, &args),
                        progress: String::new(),
                        result: Some(ToolResult { is_error: out.as_deref().is_some_and(|o| o.starts_with("Error") || o.starts_with("The user denied")), output: out.unwrap_or_default(), ..Default::default() }),
                        open: false,
                    }));
                }
            }
            _ => {}
        }
    }
    items
}
