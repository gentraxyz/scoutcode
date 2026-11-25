//! The commands the web UI calls. `act` changes the core's state, which comes back through its `view` events; the
//! rest ask for something and answer directly. Commands that wait (dialogs, the network) run off the main thread.

use crate::agent::{PermissionAnswer, PlanAnswer, QuestionAnswer};
use crate::config::{AgentRouter, ModelRef};
use crate::core::Core;
use crate::{attach, dock, mcp_client, music, session, skills};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Runtime, State};

type Shared<'a> = State<'a, Arc<Mutex<Core>>>;

/// Commands too frequent to log each call: they'd bury everything else.
const UNLOGGED: &[&str] = &[
    "snapshot", "provider_icon", "window_ready", "background_shells", "display_path", "mention_search", "search_chats", "folder_list",
    "usage_log", "dir_read", "file_read", "file_mtime", "files_list", "files_search", "terminal_suggestions", "terminal_list",
    "terminal_write", "terminal_resize", "browser_bounds", "browser_status", "browser_find", "claude_setup_status", "changes_load",
    "changes_diff", "delta_local_list", "delta_local_read", "music_list", "log_ui", "logs_list", "logs_read",
];

/// What the window asked for, in the log. Only the command's name, and an action's type: their arguments can hold
/// messages, keys and file contents.
fn log_invoke(command: &str, payload: &tauri::ipc::InvokeBody) {
    if UNLOGGED.contains(&command) {
        return;
    }
    if command == "act" {
        if let tauri::ipc::InvokeBody::Json(v) = payload {
            let kind = v.pointer("/action/type").and_then(Value::as_str).unwrap_or("?");
            crate::logs::info("action", kind);
            return;
        }
    }
    crate::logs::info("command", command);
}

pub fn handler<R: Runtime>() -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    let inner = generated();
    move |invoke| {
        log_invoke(invoke.message.command(), invoke.message.payload());
        inner(invoke)
    }
}

fn generated<R: Runtime>() -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        log_ui,
        logs_list,
        logs_read,
        logs_save,
        logs_open_folder,
        snapshot,
        act,
        provider_icon,
        send,
        unqueue,
        restart_to_update,
        window_ready,
        background_shells,
        stop_background_shell,
        paste_image,
        display_path,
        add_project,
        detect_external_chats,
        import_external_chats,
        folder_places,
        folder_list,
        folder_create,
        search_chats,
        mention_search,
        open_url,
        set_material,
        set_app_icon,
        reveal,
        usage_log,
        image_models,
        instructions_load,
        instructions_save,
        memory_load,
        memory_save,
        skill_upload,
        skill_remove,
        skill_search,
        skill_install,
        mcp_save,
        uncensored_accept,
        mcp_show_file,
        claude_setup_extension,
        claude_setup_start,
        claude_setup_status,
        claude_setup_stop,
        claude_setup_pause,
        claude_setup_browsers,
        changes_load,
        changes_diff,
        dir_read,
        file_read,
        file_save,
        file_mtime,
        files_list,
        files_search,
        terminal_prewarm,
        terminal_suggestions,
        terminal_list,
        terminal_start,
        terminal_write,
        terminal_resize,
        terminal_stop,
        delta_local_list,
        delta_local_read,
        delta_local_write,
        delta_local_delete,
        delta_cloud,
        epsilon_cloud,
        agent_cloud,
        music_list,
        music_upload,
        music_remove,
        music_soundcloud_add,
        local_add_file,
        local_hf_files,
        browser_bounds,
        browser_go,
        browser_take_over,
        browser_status,
        browser_tab_new,
        browser_tab_select,
        browser_tab_close,
        browser_tab_reopen,
        browser_tab_move,
        browser_zoom,
        browser_find,
        browser_browse_page,
    ]
}

// Session logs (logs.rs), for Settings > Logs.

/// Errors, warnings and crashes from the window, sent in batches.
#[tauri::command(async)]
fn log_ui(entries: Vec<(String, String)>) {
    for (level, msg) in entries.iter().take(200) {
        crate::logs::ui(level, msg);
    }
}

#[tauri::command(async)]
fn logs_list() -> Vec<crate::logs::Entry> {
    crate::logs::list()
}

#[tauri::command(async)]
fn logs_read(name: String) -> Result<String, String> {
    crate::logs::read(&name)
}

/// Asks where to save a log, and copies it there. False when the dialog was closed.
#[tauri::command(async)]
fn logs_save(name: String) -> Result<bool, String> {
    let text = crate::logs::read(&name)?;
    let Some(path) = rfd::FileDialog::new().set_title("Save Scout's log").set_file_name(format!("scout-{name}")).add_filter("Log", &["log", "txt"]).save_file() else {
        return Ok(false);
    };
    std::fs::write(&path, text).map_err(|e| format!("Couldn't save the log: {e}"))?;
    crate::logs::info("logs", format!("Saved {name} to {}", path.display()));
    Ok(true)
}

#[tauri::command(async)]
fn logs_open_folder() {
    let dir = crate::logs::dir();
    let _ = std::fs::create_dir_all(&dir);
    // Explorer opens the folder with the newest log selected.
    match crate::logs::list().first() {
        Some(e) => reveal_path(&dir.join(&e.name)),
        None => reveal_path(&dir),
    }
}

// The built-in browser (browser.rs). Async, so they run off the main thread: making its webview waits on the main thread.

#[tauri::command]
async fn browser_bounds(x: f64, y: f64, width: f64, height: f64, shown: bool) {
    crate::browser::set_bounds(x, y, width, height, shown);
}

/// An address, or "back", "forward" or "reload".
#[tauri::command]
async fn browser_go(to: String) -> Result<(), String> {
    crate::browser::user_go(&to)
}

#[tauri::command]
async fn browser_take_over(on: bool) {
    crate::browser::set_user(on);
}

#[tauri::command]
async fn browser_status() -> Value {
    crate::browser::status()
}

#[tauri::command]
async fn browser_tab_new(url: Option<String>) -> Result<u64, String> {
    crate::browser::new_tab(url.as_deref())
}

#[tauri::command]
async fn browser_tab_select(id: u64) {
    crate::browser::select_tab(id);
}

#[tauri::command]
async fn browser_tab_close(id: u64) {
    crate::browser::close_tab(id);
}

#[tauri::command]
async fn browser_tab_reopen() -> Result<(), String> {
    crate::browser::reopen_tab()
}

#[tauri::command]
async fn browser_tab_move(id: u64, to: usize) {
    crate::browser::move_tab(id, to);
}

/// 1 is 100%.
#[tauri::command]
async fn browser_zoom(factor: f64) -> Result<(), String> {
    crate::browser::set_zoom(factor)
}

/// Finds text in the page in front; says whether it's there.
#[tauri::command]
async fn browser_find(text: String, back: bool) -> Result<bool, String> {
    crate::browser::find(&text, back)
}

/// The Browse page came on screen or went away: messages sent meanwhile are about the browser.
#[tauri::command]
async fn browser_browse_page(on: bool) {
    crate::browser::set_browse(on);
}

#[tauri::command]
fn snapshot(core: Shared) -> Value {
    core.lock().unwrap().snapshot()
}

/// What the window can ask the core to do. One enum so that it reads as a list.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum Action {
    Stop,
    Resume,
    NewChat,
    OpenChat { id: String },
    DeleteChat { id: String },
    SetDone { id: String, done: bool },
    /// The chats in one list (Recent or Done) in the order they now show, top first.
    ReorderChats { ids: Vec<String> },
    Resend { at: usize, text: String },
    SendQueued { at: usize },
    RetryTurn { key: Option<String>, provider: Option<String> },
    SetAgentRouters { agent_routers: Vec<AgentRouter> },
    ChangeProject { dir: String },
    RenameProject { path: String, name: String },
    RemoveProject { path: String },
    AnswerPermission { answer: String, feedback: Option<String> },
    AnswerLink { answer: String, feedback: Option<String> },
    ApprovePlan { mode: String },
    RevisePlan { feedback: String },
    AnswerQuestions { answers: Option<Vec<String>> },
    ChooseModel { key: String, provider: String },
    ToggleStar { id: String },
    SetEffort { effort: String },
    SetMode { mode: String },
    SetWorktree { on: bool },
    CycleMode,
    BrowserLogin,
    SaveApiKey { key: String },
    SignOut,
    SetGuest { on: bool },
    AddOpenrouterKey { name: String, key: String },
    EditOpenrouterKey { at: usize, op: String },
    SaveNvidiaKey { key: Option<String> },
    SetOpencode { on: bool, uninstall: bool },
    SaveGrokKey { key: Option<String> },
    SaveHermesKey { key: Option<String> },
    SaveKilo { enabled: bool, key: Option<String> },
    RemoveKiloKey,
    CheckKilo,
    /// Loads every provider's model list again, as when a sign-in changes.
    ReloadModels,
    LinkAdditional { id: String, key: Option<String> },
    UnlinkAdditional { id: String },
    /// Adds a provider of the user's own, or changes one (`id` given). A missing `key` on a change keeps the saved one.
    SaveCustomProvider { id: Option<String>, name: String, kind: crate::config::ApiKind, url: String, key: Option<String>, icon: Option<String> },
    SignIn { which: String },
    CancelSignIn { which: String },
    SignOutOf { which: String, uninstall: bool },
    /// Codex, Claude Code, Grok Build, OpenCode or Copilot: another account, signed in to from here.
    AddAccount { which: String },
    /// One of those: which of its accounts is in use, by the id in `cfg.agentAccounts`.
    SwitchAccount { which: String, id: String },
    RemoveAccount { which: String, id: String },
    SetNotify { flash: bool, toast: bool },
    TestNotification,
    SetDiscord { presence: bool, project: bool },
    SetImages { on: bool, provider: Option<String>, model: Option<String> },
    SetBrowserAgent { on: bool },
    SaveImageKey { key: Option<String> },
    SetExplore { on: bool, models: Vec<ModelRef> },
    /// Code Review in the chat's side panel: `task` is what to review and how, `label` its name in the list.
    ReviewStart { label: String, task: String, models: Vec<ModelRef> },
    ReviewStop,
    ReviewFinding { review: u64, finding: u32, status: String },
    /// One review, or all of the chat's when `review` is missing.
    ReviewRemove { review: Option<u64> },
    SetGames { games: crate::config::Games },
    SetMusic { music: crate::config::Music },
    UsageRefresh,
    QuotaRefresh,
    UsageChange { body: Value, said: String },
    ToggleLink { mode: String },
    SetScoutMote { on: bool },
    /// The user touched the window, for Dreaming's idle timer. Sent at most about once a minute.
    Activity,
    SetDreaming { on: bool },
    DreamNow,
    DreamUndo { id: String },
    /// "all" for every dream in the log.
    DreamSeen { id: String },
    McpSetEnabled { name: String, on: bool },
    McpRemove { name: String },
    McpReconnect,
    /// The user said yes to a server in mcp.json that was waiting for it.
    McpAllow { name: String },
    McpSignIn { name: String },
    McpSignOut { name: String },
    McpCancelSignIn,
    CheckUpdates,
    DismissWhatsNew,
    DownloadUpdate,
    SetAutoUpdate { on: bool },
    SetAnalytics { on: bool },
    /// Whether closing the window asks first.
    SetConfirmClose { on: bool },
    RefreshSkills,
    /// Changes local model settings: any of backend, gpuLayers, context, reasoningBudget, tools, singleCall, shortPrompt.
    SetLocal { patch: Value },
    LocalInstall { backend: Option<String> },
    LocalCancelInstall,
    LocalRemoveBackend { backend: String },
    LocalDownload { id: String },
    LocalDownloadHf { repo: String, path: String, sha256: String, size: u64 },
    LocalCancelDownload { id: String },
    LocalSetup { model: Option<String> },
    LocalCancelSetup,
    LocalRemove { id: String },
    LocalUnload,
}

fn permission_answer(answer: &str, feedback: Option<String>) -> PermissionAnswer {
    match answer {
        "allow" => PermissionAnswer::Allow,
        "always" => PermissionAnswer::Always,
        _ => PermissionAnswer::Deny(feedback.unwrap_or_default()),
    }
}

#[tauri::command]
fn act(core: Shared, action: Action) -> Result<(), String> {
    let r = act_inner(core, action);
    if let Err(e) = &r {
        crate::logs::warn("action", format!("Refused: {e}"));
    }
    r
}

fn act_inner(core: Shared, action: Action) -> Result<(), String> {
    let mut c = core.lock().unwrap();
    // A Scout Enterprise team's members use the team's providers: they can't add their own, and can only sign in on this PC
    // to the ones the team allows.
    if let Some(t) = c.team() {
        let own = match &action {
            Action::AddOpenrouterKey { .. } | Action::SaveKilo { .. } | Action::LinkAdditional { .. } | Action::SaveCustomProvider { .. } | Action::SetGuest { on: true } | Action::LocalInstall { .. } | Action::LocalDownload { .. } | Action::LocalDownloadHf { .. } | Action::LocalSetup { .. } => true,
            Action::SaveNvidiaKey { key } | Action::SaveGrokKey { key } | Action::SaveHermesKey { key } | Action::SaveImageKey { key } => key.is_some(),
            Action::SetOpencode { on, .. } => *on && !t.acp.iter().any(|a| a == "opencode"),
            Action::SignIn { which } | Action::AddAccount { which } => !t.acp.iter().any(|a| a == which),
            _ => false,
        };
        if own {
            return Err(format!("{} manages your providers. Ask a team admin to add it.", t.name));
        }
    }
    match action {
        Action::Stop => c.stop(),
        Action::Resume => c.resume(),
        Action::NewChat => c.new_chat(),
        Action::OpenChat { id } => c.open_chat(&id),
        Action::DeleteChat { id } => c.delete_chat(&id),
        Action::SetDone { id, done } => c.set_done(&id, done),
        Action::ReorderChats { ids } => c.reorder_chats(&ids),
        Action::Resend { at, text } => c.resend(at, text),
        Action::SendQueued { at } => c.send_queued(at),
        Action::RetryTurn { key, provider } => c.retry(key.zip(provider)),
        Action::SetAgentRouters { agent_routers } => c.set_agent_routers(agent_routers),
        Action::ChangeProject { dir } => c.change_project(dir),
        Action::RenameProject { path, name } => c.rename_project(&path, &name),
        Action::RemoveProject { path } => c.remove_project(&path),
        Action::AnswerPermission { answer, feedback } => c.answer_permission(permission_answer(&answer, feedback)),
        Action::AnswerLink { answer, feedback } => c.answer_link(permission_answer(&answer, feedback)),
        Action::ApprovePlan { mode } => c.answer_plan(PlanAnswer::Approve(mode)),
        Action::RevisePlan { feedback } => c.answer_plan(PlanAnswer::Revise(feedback)),
        Action::AnswerQuestions { answers } => c.answer_questions(match answers {
            Some(a) => QuestionAnswer::Answered(a),
            None => QuestionAnswer::Skipped,
        }),
        Action::ChooseModel { key, provider } => c.choose_model(&key, &provider),
        Action::ToggleStar { id } => c.toggle_star(id),
        Action::SetEffort { effort } => c.set_effort(&effort),
        Action::SetMode { mode } => c.set_mode(&mode),
        Action::SetWorktree { on } => c.set_worktree(on),
        Action::CycleMode => c.cycle_mode(),
        Action::BrowserLogin => c.browser_login(),
        Action::SaveApiKey { key } => c.save_api_key(key),
        Action::SignOut => c.sign_out(),
        Action::SetGuest { on } => c.set_guest(on),
        Action::AddOpenrouterKey { name, key } => c.add_openrouter_key(name, key),
        Action::EditOpenrouterKey { at, op } => c.edit_openrouter_key(at, &op),
        Action::SaveNvidiaKey { key } => c.save_nvidia_key(key),
        Action::SetOpencode { on, uninstall } => c.set_opencode(on, uninstall),
        Action::SaveGrokKey { key } => c.save_grok_key(key),
        Action::SaveHermesKey { key } => c.save_hermes_key(key),
        Action::SaveKilo { enabled, key } => c.save_kilo(enabled, key),
        Action::RemoveKiloKey => c.remove_kilo_key(),
        Action::CheckKilo => c.check_kilo(),
        Action::ReloadModels => c.fetch_models(),
        Action::LinkAdditional { id, key } => c.link_additional(&id, key),
        Action::UnlinkAdditional { id } => c.unlink_additional(&id),
        Action::SaveCustomProvider { id, name, kind, url, key, icon } => c.save_custom_provider(id, name, kind, url, key, icon)?,
        Action::SignIn { which } => match which.as_str() {
            "copilot" => c.copilot_sign_in(),
            "devin" => c.devin_sign_in(),
            "kiro" => c.kiro_sign_in(),
            "antigravity" | "vibe" => c.acp_sign_in(&which),
            "codex" | "claude" => c.agent_sign_in(&which),
            "opencode" => c.opencode_sign_in(),
            "grok" => c.grok_sign_in(),
            "hermes" => c.hermes_sign_in(),
            _ => {}
        },
        Action::CancelSignIn { which } => c.cancel_sign_in(&which),
        Action::AddAccount { which } if crate::profiles::can_have_several(&which) => c.add_account(&which),
        Action::SwitchAccount { which, id } if crate::profiles::can_have_several(&which) => c.switch_account(&which, &id),
        Action::RemoveAccount { which, id } if crate::profiles::can_have_several(&which) => c.remove_account(&which, &id),
        Action::AddAccount { .. } | Action::SwitchAccount { .. } | Action::RemoveAccount { .. } => {}
        Action::SignOutOf { which, uninstall } => match which.as_str() {
            "copilot" => c.copilot_sign_out(uninstall),
            "devin" => c.devin_sign_out(uninstall),
            "kiro" => c.kiro_sign_out(uninstall),
            "antigravity" | "vibe" => c.acp_sign_out(&which, uninstall),
            "codex" | "claude" => c.agent_sign_out(&which, uninstall),
            "opencode" => c.opencode_sign_out(),
            "grok" => c.grok_sign_out(uninstall),
            "hermes" => c.hermes_sign_out(),
            _ => {}
        },
        Action::SetNotify { flash, toast } => c.set_notify(flash, toast),
        Action::SetDiscord { presence, project } => c.set_discord(presence, project),
        Action::SetImages { on, provider, model } => c.set_images(on, provider, model),
        Action::SetBrowserAgent { on } => c.set_browser_agent(on),
        Action::SaveImageKey { key } => c.save_image_key(key),
        Action::SetExplore { on, models } => c.set_explore(on, models),
        Action::ReviewStart { label, task, models } => c.review_start(label, task, models)?,
        Action::ReviewStop => crate::review::stop(&c.review_chat()),
        Action::ReviewFinding { review, finding, status } => crate::review::set_finding(&c.review_chat(), review, finding, &status),
        Action::ReviewRemove { review } => crate::review::remove(&c.review_chat(), review),
        Action::SetGames { games } => c.set_games(games),
        Action::SetMusic { music } => c.set_music(music),
        Action::TestNotification => crate::notify::toast("Scout", "This is how Scout will tell you it's done."),
        Action::UsageRefresh => c.usage_request(None),
        Action::QuotaRefresh => c.quota_refresh(),
        Action::UsageChange { body, said } => c.usage_request(Some((body, said))),
        Action::ToggleLink { mode } => c.toggle_link(&mode),
        Action::SetScoutMote { on } => c.set_scout_mote(on),
        Action::Activity => c.activity(),
        Action::SetDreaming { on } => c.set_dreaming(on),
        Action::DreamNow => c.dream_now(),
        Action::DreamUndo { id } => c.dream_undo(&id),
        Action::DreamSeen { id } => c.dream_seen(&id),
        Action::McpSetEnabled { name, on } => {
            let found = mcp_client::hub().views().into_iter().find(|s| s.name == name);
            if let Some(mut cfg) = found.map(|s| s.cfg) {
                cfg.disabled = !on;
                // Turning a server on in Settings is the user's own OK for it.
                if on {
                    if let Err(e) = mcp_client::approve(&name, &cfg) {
                        c.set_status(e);
                    }
                }
                if let Err(e) = mcp_client::set(None, &name, Some(cfg)) {
                    c.set_status(e);
                }
            }
            mcp_client::hub().reload(&c.waker(), false);
        }
        Action::McpRemove { name } => {
            if let Err(e) = mcp_client::set(None, &name, None) {
                c.set_status(e);
            } else {
                crate::mcp_auth::forget(&name);
                mcp_client::unapprove(&name);
            }
            mcp_client::hub().reload(&c.waker(), false);
        }
        Action::McpReconnect => mcp_client::hub().reload(&c.waker(), true),
        Action::McpAllow { name } => {
            if let Err(e) = mcp_client::hub().allow(&name, &c.waker()) {
                c.set_status(e);
            }
        }
        Action::McpSignIn { name } => mcp_client::hub().sign_in(&name, &c.waker()),
        Action::McpSignOut { name } => mcp_client::hub().sign_out(&name, &c.waker()),
        Action::McpCancelSignIn => mcp_client::hub().cancel_sign_in(),
        Action::CheckUpdates => c.updater().check_now(),
        Action::DismissWhatsNew => c.dismiss_whats_new(),
        Action::DownloadUpdate => c.updater().download_now(),
        Action::SetAutoUpdate { on } => c.set_auto_update(on),
        Action::SetAnalytics { on } => c.set_analytics(on),
        Action::SetConfirmClose { on } => c.set_confirm_close(on),
        Action::RefreshSkills => c.refresh_skills(),
        Action::SetLocal { patch } => c.set_local(patch),
        Action::LocalInstall { backend } => c.local_install(backend),
        Action::LocalCancelInstall => crate::local::cancel_install(),
        Action::LocalRemoveBackend { backend } => c.local_remove_backend(&backend),
        Action::LocalDownload { id } => c.local_download(&id),
        Action::LocalDownloadHf { repo, path, sha256, size } => c.local_download_hf(&repo, &path, &sha256, size)?,
        Action::LocalCancelDownload { id } => crate::local::cancel_download(&id),
        Action::LocalSetup { model } => c.local_setup(model),
        Action::LocalCancelSetup => crate::local::cancel_setup(),
        Action::LocalRemove { id } => c.local_remove(&id),
        Action::LocalUnload => crate::local::stop(),
    }
    c.wake();
    Ok(())
}

/// Adds a GGUF model file the user picks. The dialog is open before the core is locked, so Scout keeps working behind it.
#[tauri::command(async)]
fn local_add_file(core: Shared<'_>) -> Result<(), String> {
    let Some(path) = rfd::FileDialog::new().set_title("Add a local model").add_filter("GGUF model", &["gguf"]).pick_file() else { return Ok(()) };
    let mut c = core.lock().unwrap();
    if let Some(t) = c.team() {
        return Err(format!("{} manages your providers. Ask a team admin to add it.", t.name));
    }
    let r = c.local_add_file(&path);
    c.wake();
    r
}

/// The icon to suggest for a custom provider at `address`: its site's favicon, as a data: URL. Off the core's lock.
#[tauri::command(async)]
fn provider_icon(address: String) -> Result<String, String> {
    crate::favicon::suggest(&address)
}

/// The GGUF files in a Hugging Face repo, from a link or "owner/name", for Get more models. Off the core's lock.
#[tauri::command(async)]
fn local_hf_files(input: String) -> Result<Value, String> {
    crate::local::hf_files(&input)
}

/// False when the message wasn't taken and should stay in the box.
#[tauri::command]
fn send(core: Shared, text: String, files: Vec<String>, steer: bool) -> bool {
    let mut c = core.lock().unwrap();
    let message = !text.trim_start().starts_with('/');
    let taken = c.send(text, files, steer);
    if taken && message {
        c.track_sent(steer);
    }
    c.wake();
    taken
}

/// Takes a message out of the queue and gives it back, to go back in the box.
#[tauri::command]
fn unqueue(core: Shared, at: usize) -> Option<Value> {
    let mut c = core.lock().unwrap();
    let o = c.unqueue(at);
    c.wake();
    o.map(|o| json!({ "text": o.text, "files": o.files }))
}

/// Starts the new build on the same folder and chat, then closes. The window has checked for unsaved edits.
#[tauri::command]
fn restart_to_update<R: Runtime>(app: AppHandle<R>, core: Shared) {
    let restarted = {
        let mut c = core.lock().unwrap();
        let r = c.restart_to_update();
        c.wake();
        r
    };
    if restarted {
        crate::logs::info("update", "Restarting into the new version.");
        // If something holds up the exit (a shell, a sidecar, the webview), the old copy must still go: the new one is
        // waiting for it, and users saw both stay open.
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(6));
            crate::logs::warn("exit", "Closing took over 6 seconds, so Scout ended itself.");
            crate::logs::end();
            std::process::exit(0);
        });
        app.exit(0);
    }
}

/// The agent's background shells, for their tabs in the panel. `lines` of output from each (0 for just the list).
#[tauri::command(async)]
fn background_shells(lines: usize) -> Vec<Value> {
    crate::tools::background_shells(lines.min(2000))
}

#[tauri::command(async)]
fn stop_background_shell(id: String) {
    crate::tools::stop_background_shell(&id);
}

/// The UI has drawn its first frame: shows the window, which starts hidden so it never appears empty.
#[tauri::command]
fn window_ready<R: Runtime>(window: tauri::WebviewWindow<R>) {
    if !window.is_visible().unwrap_or(false) {
        crate::logs::info("window", "The UI drew its first frame.");
        reveal_window(&window);
    }
}

/// Shows the main window and brings it to the front. After an update the user has usually looked away while it
/// installed, so the new copy takes the foreground (the old one allowed it, see update::Updater::restart) to say it's back.
pub fn reveal_window<R: Runtime>(window: &tauri::WebviewWindow<R>) {
    let _ = window.show();
    let _ = window.unminimize();
    if std::env::var_os(crate::single_instance::RESTART_ENV).is_some() {
        // Windows only lets a window jump to the front this way; set_focus alone may just flash the taskbar.
        let _ = window.set_always_on_top(true);
        let _ = window.set_focus();
        let _ = window.set_always_on_top(false);
        let _ = window.request_user_attention(Some(tauri::UserAttentionType::Informational));
    } else {
        let _ = window.set_focus();
    }
}

/// Saves an image pasted into the box (base64, as the browser has it) to a temp file, to attach. Returns its path.
#[tauri::command(async)]
fn paste_image(data: String, ext: String) -> Result<String, String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| e.to_string())?;
    let dir = std::env::temp_dir().join("scout-pasted");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("pasted-{}.{ext}", crate::session::now_ms()));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
fn display_path(cwd: String, path: String) -> String {
    attach::display_path(&cwd, Path::new(&path))
}

/// Starts a chat in the folder the user picked in the window's folder picker. Returns the folder when it's a new
/// project, which the user then names.
#[tauri::command]
fn add_project(core: Shared<'_>, dir: String) -> Result<Option<String>, String> {
    if !Path::new(&dir).is_dir() {
        return Err("That folder doesn't exist.".into());
    }
    let mut c = core.lock().unwrap();
    let is_new = c.add_project(dir.clone());
    c.wake();
    Ok(is_new.then_some(dir))
}

#[tauri::command(async)]
fn folder_places() -> crate::folders::Places {
    crate::folders::places()
}

#[tauri::command(async)]
fn folder_list(dir: String, files: Option<bool>) -> crate::folders::Listing {
    crate::folders::list(&dir, files.unwrap_or(false))
}

#[tauri::command]
fn folder_create(parent: String, name: String) -> Result<String, String> {
    crate::folders::create(&parent, &name)
}

#[tauri::command(async)]
fn search_chats(q: String) -> Vec<String> {
    session::search_sessions(&q)
}

#[tauri::command(async)]
fn detect_external_chats() -> Vec<session::DetectedChat> {
    session::detect_external_chats()
}

#[tauri::command(async)]
fn import_external_chats(core: Shared<'_>, ids: Vec<String>) -> usize {
    let mut c = core.lock().unwrap();
    let n = c.import_external_chats(&ids);
    c.wake();
    n
}

/// The project's files for the @ picker, listed at most every few seconds.
static FILE_INDEX: Mutex<Option<(String, Vec<String>, std::time::Instant)>> = Mutex::new(None);

/// The @ picker's rows: matching skills first, leaving room for files.
#[tauri::command(async)]
fn mention_search(cwd: String, query: String) -> Value {
    let found = skills::list();
    let names: Vec<String> = found.iter().map(|s| s.name.clone()).collect();
    let mut rows: Vec<Value> = attach::matches(&names, &query, 4)
        .into_iter()
        .filter_map(|name| found.iter().find(|s| s.name == *name))
        .map(|s| json!({ "skill": true, "name": s.name, "about": s.short_description() }))
        .collect();
    let mut index = FILE_INDEX.lock().unwrap();
    let fresh = index.as_ref().is_some_and(|(c, _, at)| *c == cwd && at.elapsed().as_secs() < 10);
    if !fresh {
        *index = Some((cwd.clone(), attach::index(&cwd, 50_000), std::time::Instant::now()));
    }
    let files = &index.as_ref().unwrap().1;
    rows.extend(attach::matches(files, &query, 8 - rows.len()).into_iter().map(|f| json!({ "skill": false, "name": f })));
    Value::Array(rows)
}

/// The window's backdrop, from Settings > Appearance: "solid", or "mica", "acrylic" or "glass", which let Windows draw
/// the desktop through the window (blurred) while the UI's colors turn translucent. Liquid Glass is acrylic with a
/// lighter tint; the UI adds its glass highlights. Fails on Windows versions without the effect (Mica needs 11).
#[tauri::command]
fn set_material<R: Runtime>(window: tauri::WebviewWindow<R>, material: String) -> Result<(), String> {
    use tauri::window::{Effect, EffectsBuilder};
    use tauri::webview::Color;
    let effects = match material.as_str() {
        "mica" => Some(EffectsBuilder::new().effect(Effect::MicaDark).build()),
        "acrylic" => Some(EffectsBuilder::new().effect(Effect::Acrylic).color(Color(18, 18, 20, 120)).build()),
        "glass" => Some(EffectsBuilder::new().effect(Effect::Acrylic).color(Color(40, 44, 56, 60)).build()),
        _ => None,
    };
    match effects {
        Some(e) => {
            window.set_effects(e).map_err(|e| e.to_string())?;
            window.set_background_color(Some(Color(0, 0, 0, 0))).map_err(|e| e.to_string())
        }
        None => {
            window.set_effects(None).map_err(|e| e.to_string())?;
            window.set_background_color(Some(Color(0x1f, 0x1f, 0x1f, 0xff))).map_err(|e| e.to_string())
        }
    }
}

#[cfg(windows)]
static PREV_TASKBAR_ICON: std::sync::Mutex<Option<usize>> = std::sync::Mutex::new(None);

#[cfg(windows)]
fn create_hicon_from_rgba(
    rgba: &[u8],
    size: u32,
) -> Result<windows::Win32::UI::WindowsAndMessaging::HICON, String> {
    use windows::Win32::Graphics::Gdi::{
        CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        DIB_RGB_COLORS, HGDIOBJ, RGBQUAD,
    };
    use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, ICONINFO};

    if size == 0 || size > 512 || rgba.len() != (size * size * 4) as usize {
        return Err("The icon must be square RGBA.".into());
    }

    // Convert RGBA to BGRA for 32-bit DIB section
    let mut bgra = rgba.to_vec();
    for chunk in bgra.chunks_exact_mut(4) {
        chunk.swap(0, 2);
    }

    let bi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size as i32,
            biHeight: -(size as i32), // Negative height specifies a top-down DIB
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        bmiColors: [RGBQUAD::default()],
    };

    unsafe {
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let color_bmp = CreateDIBSection(None, &bi, DIB_RGB_COLORS, &mut bits, None, 0)
            .map_err(|e| format!("CreateDIBSection failed: {e}"))?;

        if !bits.is_null() {
            std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits as *mut u8, bgra.len());
        }

        let stride = ((size + 15) / 16 * 2) as usize;
        let mask_bytes = vec![0u8; stride * size as usize];
        let mask_bmp = CreateBitmap(
            size as i32,
            size as i32,
            1,
            1,
            Some(mask_bytes.as_ptr() as *const _),
        );

        if mask_bmp.is_invalid() {
            let _ = DeleteObject(HGDIOBJ::from(color_bmp));
            return Err("CreateBitmap for mask failed".into());
        }

        let icon_info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask_bmp,
            hbmColor: color_bmp,
        };

        let hicon = match CreateIconIndirect(&icon_info) {
            Ok(h) => h,
            Err(e) => {
                let _ = DeleteObject(HGDIOBJ::from(mask_bmp));
                let _ = DeleteObject(HGDIOBJ::from(color_bmp));
                return Err(format!("CreateIconIndirect failed: {e}"));
            }
        };

        let _ = DeleteObject(HGDIOBJ::from(mask_bmp));
        let _ = DeleteObject(HGDIOBJ::from(color_bmp));

        Ok(hicon)
    }
}

#[cfg(windows)]
fn set_windows_taskbar_icon<R: Runtime>(
    window: &tauri::WebviewWindow<R>,
    rgba: &[u8],
    size: u32,
) -> Result<(), String> {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        DestroyIcon, SendMessageW, SetWindowPos, HICON, ICON_BIG, ICON_SMALL, SWP_FRAMECHANGED,
        SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WM_SETICON,
    };

    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let hicon = create_hicon_from_rgba(rgba, size)?;

    unsafe {
        // Set both ICON_BIG (Windows taskbar, Alt+Tab) and ICON_SMALL (title bar)
        SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_BIG as usize)),
            Some(LPARAM(hicon.0 as isize)),
        );
        SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_SMALL as usize)),
            Some(LPARAM(hicon.0 as isize)),
        );

        // Notify shell/taskbar to redraw immediately
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
        );

        let mut prev = PREV_TASKBAR_ICON.lock().unwrap();
        let old = prev.replace(hicon.0 as usize);
        if let Some(old_hicon) = old {
            let _ = DestroyIcon(HICON(old_hicon as *mut _));
        }
    }

    Ok(())
}

/// The taskbar and title icon, drawn by the UI from the logo picked in Settings > Appearance: `size`×`size` RGBA.
#[tauri::command]
fn set_app_icon<R: Runtime>(window: tauri::WebviewWindow<R>, rgba: Vec<u8>, size: u32) -> Result<(), String> {
    if size == 0 || size > 512 || rgba.len() != (size * size * 4) as usize {
        return Err("The icon must be square RGBA.".into());
    }

    #[cfg(windows)]
    set_windows_taskbar_icon(&window, &rgba, size)?;

    window.set_icon(tauri::image::Image::new_owned(rgba, size, size)).map_err(|e| e.to_string())
}

/// Opens web pages in the browser. Anything else a page might link to stays closed.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("Only web links open from here.".into());
    }
    crate::oauth::open_browser(&url);
    Ok(())
}

/// Shows a file or folder in the system's file manager.
#[tauri::command]
fn reveal(path: String) {
    reveal_path(Path::new(&path));
}

fn reveal_path(path: &Path) {
    #[cfg(windows)]
    let mut c = {
        use std::os::windows::process::CommandExt;
        let mut c = std::process::Command::new("explorer");
        // Explorer wants the quotes around the path alone, which arg() wouldn't do.
        c.raw_arg(format!("/select,\"{}\"", path.display()));
        c
    };
    #[cfg(not(windows))]
    let mut c = {
        let mut c = std::process::Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" });
        c.arg(path.parent().unwrap_or(path));
        c
    };
    let _ = c.spawn();
}

// ---------- Delta ----------

#[tauri::command(async)]
fn delta_local_list() -> Vec<Value> {
    crate::delta::list()
}

#[tauri::command(async)]
fn delta_local_read(id: String) -> Result<Option<String>, String> {
    crate::delta::read(&id)
}

#[tauri::command(async)]
fn delta_local_write(id: String, text: String) -> Result<(), String> {
    crate::delta::write(&id, &text)
}

#[tauri::command(async)]
fn delta_local_delete(id: String) -> Result<(), String> {
    crate::delta::delete(&id)
}

/// The lock is let go before the request, which can take a while.
#[tauri::command(async)]
fn delta_cloud(core: Shared<'_>, method: String, path: String, body: Option<Value>) -> Result<Value, String> {
    let cfg = core.lock().unwrap().scout_cfg();
    crate::delta::cloud(&cfg, &method, &path, body)
}

/// Epsilon through the ScoutCode site (/api/epsilon/…): the account's cloud computers and the short-lived
/// tokens the framed computer page asks for. The key stays here; the window only sees the answers.
#[tauri::command(async)]
fn epsilon_cloud(core: Shared<'_>, method: String, path: String, body: Option<Value>) -> Result<Value, String> {
    let allowed = ["/api/epsilon/machines", "/api/epsilon/token"];
    if !allowed.iter().any(|p| path == *p || path.starts_with(&format!("{p}?"))) {
        return Err("Not an Epsilon path".into());
    }
    let cfg = core.lock().unwrap().scout_cfg();
    // Signing an agent in on the server can first download it there, and saving a computer as it stops takes a while.
    crate::delta::site_request(&cfg, &method, &path, body, 600, "Sign in to your ScoutCode account to use Epsilon.")
}

/// Agent, the home page: persistent agents kept on the ScoutCode site (src/pages/api/account/agents.ts).
#[tauri::command]
async fn agent_cloud(core: Shared<'_>, method: String, path: String, body: Option<Value>) -> Result<Value, String> {
    let base = "/api/account/agents";
    if !(path == base || path.starts_with(&format!("{base}?"))) {
        return Err("Not an Agent path".into());
    }
    let cfg = core.lock().unwrap().scout_cfg();
    // Off the main thread: attachments can take a while to upload.
    tauri::async_runtime::spawn_blocking(move || crate::delta::site_request(&cfg, &method, &path, body, 120, "Sign in to your ScoutCode account to use Agent."))
        .await
        .map_err(|e| e.to_string())?
}

// ---------- settings ----------

#[tauri::command]
fn usage_log() -> Value {
    let log = crate::usage::load();
    let sum = |n: usize| crate::usage::last_days(&log, n).iter().fold((0u64, 0f64), |(t, c), (_, d)| (t + d.tokens, c + d.cost));
    let seeded = log.seeded_on.as_deref().filter(|d| log.days.keys().any(|k| k.as_str() < *d));
    json!({
        "today": sum(1), "week": sum(7), "month": sum(30),
        "days": crate::usage::last_days(&log, 30).into_iter().map(|(day, d)| json!({ "day": day, "tokens": d.tokens, "cost": d.cost, "requests": d.requests })).collect::<Vec<_>>(),
        "seededOn": seeded,
    })
}

/// A provider's image models, for Settings > Image generation.
#[tauri::command(async)]
fn image_models(provider: String) -> Result<Value, String> {
    let models = crate::api::image_models(&provider).map_err(|e| e.to_string())?;
    Ok(Value::Array(models.into_iter().map(|(id, name, cost)| json!({ "id": id, "name": name, "cost": cost })).collect()))
}

#[tauri::command]
fn instructions_load(cwd: String) -> Result<Value, String> {
    let p = crate::instructions::Preferences::load()?;
    Ok(json!({ "global": p.global, "project": p.project(&cwd), "language": p.language }))
}

#[tauri::command]
fn instructions_save(cwd: String, global: String, project: String, language: String) -> Result<(), String> {
    crate::instructions::Preferences::load()?.save_changes(&cwd, &global, &project, &language).map(|_| ())
}

const MEMORY_SCOPES: [crate::memory::Scope; 2] = [crate::memory::Scope::Project, crate::memory::Scope::User];

/// The two memory files: where each is, and what's in it.
#[tauri::command]
fn memory_load(cwd: String) -> Value {
    Value::Array(
        MEMORY_SCOPES
            .into_iter()
            .map(|s| {
                let path = crate::memory::path(s, &cwd);
                let text = crate::tools::read_text(&path).map(|(t, _)| t).unwrap_or_default();
                json!({ "path": path, "text": text })
            })
            .collect(),
    )
}

#[tauri::command]
fn memory_save(cwd: String, saved: [String; 2], drafts: [String; 2]) -> Result<(), String> {
    for (i, scope) in MEMORY_SCOPES.into_iter().enumerate() {
        if saved[i] != drafts[i] {
            crate::memory::save_draft(&crate::memory::path(scope, &cwd), &saved[i], &drafts[i])?;
        }
    }
    Ok(())
}

/// Uploads a skill from a folder or a file the user picks. None when they cancelled; otherwise the names it added.
#[tauri::command(async)]
fn skill_upload(core: Shared<'_>, folder: bool) -> Result<Option<Vec<String>>, String> {
    let picked: Option<PathBuf> = if folder {
        rfd::FileDialog::new().set_title("Choose a skill's folder").pick_folder()
    } else {
        rfd::FileDialog::new().set_title("Choose a SKILL.md or a .zip").add_filter("Skill", &["md", "zip", "skill"]).pick_file()
    };
    let Some(path) = picked else { return Ok(None) };
    let names = skills::upload(&path);
    let mut c = core.lock().unwrap();
    c.refresh_skills();
    c.wake();
    names.map(Some)
}

#[tauri::command]
fn skill_remove(core: Shared, name: String) -> Result<(), String> {
    let r = skills::remove(&name);
    let mut c = core.lock().unwrap();
    c.refresh_skills();
    c.wake();
    r
}

#[tauri::command(async)]
fn skill_search(query: String) -> Result<Value, String> {
    let found = skills::search(query.trim())?;
    Ok(Value::Array(found.iter().map(|l| json!({ "key": l.key(), "name": l.name, "source": l.source, "id": l.id, "installs": l.installs, "url": l.url() })).collect()))
}

/// Installs a directory listing; only what's needed to find it comes from the window.
#[tauri::command(async)]
fn skill_install(core: Shared<'_>, source: String, id: String, name: String, installs: u64) -> Result<String, String> {
    let result = skills::install(&skills::Listing { source, id, name, installs });
    let mut c = core.lock().unwrap();
    c.refresh_skills();
    c.wake();
    result
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpForm {
    /// The name it's saved under, when changing one.
    editing: Option<String>,
    name: String,
    remote: bool,
    command: String,
    args: String,
    env: String,
    url: String,
    headers: String,
}

/// Saves the server as filled in, or says what's wrong with it.
#[tauri::command]
fn mcp_save(core: Shared, form: McpForm) -> Result<(), String> {
    use mcp_client::{check_name, hub, parse_pairs, split_args};
    let name = form.name.trim().to_string();
    let views = hub().views();
    if form.editing.as_deref() != Some(name.as_str()) && views.iter().any(|v| v.name == name) {
        return Err(format!("There's already a server called {name}."));
    }
    check_name(&name)?;
    // What's kept from the entry being changed: fields Scout doesn't use, like `type`.
    let mut c = form.editing.as_ref().and_then(|e| views.iter().find(|v| v.name == *e)).map(|v| v.cfg.clone()).unwrap_or_default();
    if form.remote {
        let url = form.url.trim();
        if !url.starts_with("https://") && !url.starts_with("http://") {
            return Err("The URL has to start with https://.".into());
        }
        c.url = url.to_string();
        c.headers = parse_pairs(&form.headers, ':')?;
        (c.command, c.args, c.env) = Default::default();
    } else {
        if form.command.trim().is_empty() {
            return Err("Say which command starts the server, like npx.".into());
        }
        c.command = form.command.trim().to_string();
        c.args = split_args(&form.args);
        c.env = parse_pairs(&form.env, '=')?;
        (c.url, c.headers) = Default::default();
    }
    let remote = !c.url.is_empty();
    // Saving the form, with what it runs or connects to in view, is the user's OK for this server.
    mcp_client::approve(&name, &c)?;
    mcp_client::set(form.editing.as_deref(), &name, Some(c))?;
    if let Some(old) = form.editing.as_deref().filter(|o| *o != name) {
        crate::mcp_auth::forget(old);
    }
    if remote {
        // Saved with no token: if the server wants a sign-in, the browser opens for it straight away.
        hub().sign_in_when_needed(&name);
    }
    let core = core.lock().unwrap();
    hub().reload(&core.waker(), false);
    core.wake();
    Ok(())
}

/// The account agrees to `version` of the uncensored model terms the popup showed. Off the main thread, as it waits
/// on the network.
#[tauri::command]
async fn uncensored_accept(core: Shared<'_>, version: String) -> Result<(), String> {
    let core = core.inner().clone();
    let cfg = core.lock().unwrap().scoutcode_cfg();
    tauri::async_runtime::spawn_blocking(move || crate::api::accept_uncensored(&cfg, &version))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.message)?;
    core.lock().unwrap().wake();
    Ok(())
}

#[tauri::command]
fn mcp_show_file() {
    reveal_path(&mcp_client::ensure_file());
}

// ---------- the right panel ----------

#[tauri::command(async)]
fn changes_load(cwd: String) -> Result<dock::Changes, String> {
    dock::changes(&cwd)
}

#[tauri::command(async)]
fn changes_diff(repo: String, path: String, from: Option<String>, status: char) -> Result<Value, String> {
    let (lines, created) = dock::file_diff(&repo, &path, from.as_deref(), status)?;
    Ok(crate::core::diff_rows(&lines, created))
}

#[tauri::command(async)]
fn dir_read(dir: String) -> Vec<dock::Node> {
    dock::read_dir(&dir)
}

#[tauri::command(async)]
fn file_read(path: String) -> dock::OpenFile {
    dock::read_file(&path)
}

#[tauri::command]
fn file_save(path: String, text: String, crlf: bool) -> Result<Option<u64>, String> {
    dock::save_file(&path, &text, crlf)
}

#[tauri::command]
fn file_mtime(path: String) -> Option<u64> {
    dock::mtime(&path)
}

/// Mission Control's quick open: the project's files, relative to `root`.
#[tauri::command(async)]
fn files_list(root: String) -> Vec<String> {
    dock::list_files(&root, 50_000)
}

/// Mission Control's search across the project's files.
#[tauri::command(async)]
fn files_search(root: String, query: String, case: bool, regex: bool) -> Result<Vec<dock::Hit>, String> {
    dock::search(&root, &query, case, regex, 2_000)
}

#[tauri::command]
fn terminal_list(dir: String) -> Vec<(String, bool)> {
    dock::list(&dir)
}

#[tauri::command]
fn terminal_suggestions(cwd: String) -> dock::Suggestions {
    dock::suggestions(&cwd)
}

#[tauri::command]
fn terminal_prewarm(terminal: State<dock::Terminal>, cwd: String, rows: u16, cols: u16) {
    terminal.prewarm(&cwd, rows, cols);
}

#[tauri::command]
fn terminal_start(terminal: State<dock::Terminal>, id: u32, cwd: String, rows: u16, cols: u16, output: Channel<InvokeResponseBody>, exited: Channel<bool>) -> Result<String, String> {
    terminal.start(id, &cwd, rows, cols, output, exited)
}

#[tauri::command]
fn terminal_write(terminal: State<dock::Terminal>, id: u32, data: String) {
    terminal.write(id, &data);
}

#[tauri::command]
fn terminal_resize(terminal: State<dock::Terminal>, id: u32, rows: u16, cols: u16) {
    terminal.resize(id, rows, cols);
}

#[tauri::command]
fn terminal_stop(terminal: State<dock::Terminal>, id: u32) {
    terminal.stop(id);
}

// ---------- music ----------

#[tauri::command]
fn music_list() -> Vec<music::MusicTrack> {
    music::list()
}

#[tauri::command(async)]
fn music_upload() -> Result<Option<Vec<music::MusicTrack>>, String> {
    music::upload_files()
}

#[tauri::command]
fn music_remove(id: String) -> Result<Vec<music::MusicTrack>, String> {
    music::remove_track(&id)
}

#[tauri::command(async)]
fn music_soundcloud_add(url: String) -> Result<music::Added, String> {
    music::soundcloud_add(&url)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn test_create_hicon_from_rgba() {
        use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;
        let size = 32u32;
        let mut rgba = vec![0u8; (size * size * 4) as usize];
        for pixel in rgba.chunks_exact_mut(4) {
            pixel[0] = 0xf5; // R
            pixel[1] = 0xa6; // G
            pixel[2] = 0x23; // B
            pixel[3] = 0xff; // A
        }
        let hicon = create_hicon_from_rgba(&rgba, size).expect("create HICON");
        assert!(!hicon.is_invalid());
        unsafe {
            let _ = DestroyIcon(hicon);
        }
    }

    #[test]
    #[cfg(windows)]
    fn test_create_hicon_invalid_size() {
        assert!(create_hicon_from_rgba(&[], 0).is_err());
        assert!(create_hicon_from_rgba(&[0; 10], 32).is_err());
    }
}


/// Adds Scout's MCP address to claude.ai or Gemini (`bot`, Claude when not given) as a connector: Scout works the
/// browser itself on Windows, with its own bar and Pause, or the Scout browser extension does. See claude_setup.rs.
#[tauri::command]
fn claude_setup_start<R: Runtime>(app: AppHandle<R>, bot: Option<String>, name: String, url: String, browser: Option<String>) -> Result<Value, String> {
    crate::claude_setup::start(&app, bot.as_deref().unwrap_or("claude"), &name, &url, browser.as_deref())
}

/// The browsers the Claude setup can open, the default first.
#[tauri::command]
fn claude_setup_browsers() -> Vec<Value> {
    crate::claude_setup::browsers()
}

#[tauri::command]
fn claude_setup_pause(paused: bool) -> Value {
    crate::claude_setup::pause(paused)
}

#[tauri::command]
fn claude_setup_status() -> Value {
    crate::claude_setup::status()
}

#[tauri::command]
fn claude_setup_stop() -> Value {
    crate::claude_setup::stop()
}

/// Writes the extension's folder and shows it, for Load unpacked.
#[tauri::command]
fn claude_setup_extension() -> Result<String, String> {
    let manifest = crate::browser_ext::extension_folder()?;
    reveal_path(&manifest);
    Ok(manifest.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default())
}
