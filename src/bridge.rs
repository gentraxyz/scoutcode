//! The commands the web UI calls. `act` changes the core's state, which comes back through its `view` events; the
//! rest ask for something and answer directly. Commands that wait (dialogs, the network) run off the main thread.

use crate::agent::{PermissionAnswer, PlanAnswer, QuestionAnswer};
use crate::config::{Fallbacks, ModelRef};
use crate::core::Core;
use crate::{attach, dock, mcp_client, session, skills};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Runtime, State};

type Shared<'a> = State<'a, Arc<Mutex<Core>>>;

pub fn handler<R: Runtime>() -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        snapshot,
        act,
        send,
        unqueue,
        restart_to_update,
        pick_attachments,
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
        mcp_show_file,
        changes_load,
        changes_diff,
        dir_read,
        file_read,
        file_save,
        file_mtime,
        terminal_start,
        terminal_write,
        terminal_resize,
        terminal_stop,
        delta_local_list,
        delta_local_read,
        delta_local_write,
        delta_local_delete,
        delta_cloud,
    ]
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
    NewChat,
    OpenChat { id: String },
    DeleteChat { id: String },
    Resend { at: usize, text: String },
    SendQueued { at: usize },
    RetryTurn { key: Option<String>, provider: Option<String> },
    SetupFallbacks { failed: ModelRef, all: Vec<ModelRef>, own: Vec<ModelRef>, off: bool },
    SetFallbacks { fallbacks: Fallbacks },
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
    CycleMode,
    ToggleVoice,
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
    LinkAdditional { id: String, key: Option<String> },
    UnlinkAdditional { id: String },
    SignIn { which: String },
    CancelSignIn { which: String },
    SignOutOf { which: String, uninstall: bool },
    SetNotify { flash: bool, toast: bool },
    TestNotification,
    SetDiscord { presence: bool, project: bool },
    SetImages { on: bool, model: Option<String> },
    SetGames { games: crate::config::Games },
    UsageRefresh,
    QuotaRefresh,
    UsageChange { body: Value, said: String },
    ToggleLink { mode: String },
    SetScoutMote { on: bool },
    McpSetEnabled { name: String, on: bool },
    McpRemove { name: String },
    McpReconnect,
    CheckUpdates,
    DismissWhatsNew,
    DownloadUpdate,
    SetAutoUpdate { on: bool },
    RefreshSkills,
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
    let mut c = core.lock().unwrap();
    // A Scout Enterprise team's members use the team's providers: they can't add their own, and can only sign in on this PC
    // to the ones the team allows.
    if let Some(t) = c.team() {
        let own = match &action {
            Action::AddOpenrouterKey { .. } | Action::SaveKilo { .. } | Action::LinkAdditional { .. } | Action::SetGuest { on: true } => true,
            Action::SaveNvidiaKey { key } | Action::SaveGrokKey { key } | Action::SaveHermesKey { key } => key.is_some(),
            Action::SetOpencode { on, .. } => *on && !t.acp.iter().any(|a| a == "opencode"),
            Action::SignIn { which } => !t.acp.iter().any(|a| a == which),
            _ => false,
        };
        if own {
            return Err(format!("{} manages your providers. Ask a team admin to add it.", t.name));
        }
    }
    match action {
        Action::Stop => c.stop(),
        Action::NewChat => c.new_chat(),
        Action::OpenChat { id } => c.open_chat(&id),
        Action::DeleteChat { id } => c.delete_chat(&id),
        Action::Resend { at, text } => c.resend(at, text),
        Action::SendQueued { at } => c.send_queued(at),
        Action::RetryTurn { key, provider } => c.retry(key.zip(provider)),
        Action::SetupFallbacks { failed, all, own, off } => c.setup_fallbacks(failed, all, own, off),
        Action::SetFallbacks { fallbacks } => c.set_fallbacks(fallbacks),
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
        Action::CycleMode => c.cycle_mode(),
        Action::ToggleVoice => c.toggle_voice(),
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
        Action::LinkAdditional { id, key } => c.link_additional(&id, key),
        Action::UnlinkAdditional { id } => c.unlink_additional(&id),
        Action::SignIn { which } => match which.as_str() {
            "copilot" => c.copilot_sign_in(),
            "devin" => c.devin_sign_in(),
            "kiro" => c.kiro_sign_in(),
            "codex" | "claude" => c.agent_sign_in(&which),
            "opencode" => c.opencode_sign_in(),
            "grok" => c.grok_sign_in(),
            "hermes" => c.hermes_sign_in(),
            _ => {}
        },
        Action::CancelSignIn { which } => c.cancel_sign_in(&which),
        Action::SignOutOf { which, uninstall } => match which.as_str() {
            "copilot" => c.copilot_sign_out(uninstall),
            "devin" => c.devin_sign_out(uninstall),
            "kiro" => c.kiro_sign_out(uninstall),
            "codex" | "claude" => c.agent_sign_out(&which, uninstall),
            "opencode" => c.opencode_sign_out(),
            "grok" => c.grok_sign_out(uninstall),
            "hermes" => c.hermes_sign_out(),
            _ => {}
        },
        Action::SetNotify { flash, toast } => c.set_notify(flash, toast),
        Action::SetDiscord { presence, project } => c.set_discord(presence, project),
        Action::SetImages { on, model } => c.set_images(on, model),
        Action::SetGames { games } => c.set_games(games),
        Action::TestNotification => crate::notify::toast("Scout", "This is how Scout will tell you it's done."),
        Action::UsageRefresh => c.usage_request(None),
        Action::QuotaRefresh => c.quota_refresh(),
        Action::UsageChange { body, said } => c.usage_request(Some((body, said))),
        Action::ToggleLink { mode } => c.toggle_link(&mode),
        Action::SetScoutMote { on } => c.set_scout_mote(on),
        Action::McpSetEnabled { name, on } => {
            let found = mcp_client::hub().views().into_iter().find(|s| s.name == name);
            if let Some(mut cfg) = found.map(|s| s.cfg) {
                cfg.disabled = !on;
                if let Err(e) = mcp_client::set(None, &name, Some(cfg)) {
                    c.set_status(e);
                }
            }
            mcp_client::hub().reload(&c.waker(), false);
        }
        Action::McpRemove { name } => {
            if let Err(e) = mcp_client::set(None, &name, None) {
                c.set_status(e);
            }
            mcp_client::hub().reload(&c.waker(), false);
        }
        Action::McpReconnect => mcp_client::hub().reload(&c.waker(), true),
        Action::CheckUpdates => c.updater().check_now(),
        Action::DismissWhatsNew => c.dismiss_whats_new(),
        Action::DownloadUpdate => c.updater().download_now(),
        Action::SetAutoUpdate { on } => c.set_auto_update(on),
        Action::RefreshSkills => c.refresh_skills(),
    }
    c.wake();
    Ok(())
}

/// False when the message wasn't taken and should stay in the box.
#[tauri::command]
fn send(core: Shared, text: String, files: Vec<String>, steer: bool) -> bool {
    let mut c = core.lock().unwrap();
    let taken = c.send(text, files, steer);
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
        app.exit(0);
    }
}

#[tauri::command(async)]
fn pick_attachments(cwd: String) -> Vec<String> {
    rfd::FileDialog::new().set_directory(&cwd).pick_files().unwrap_or_default().iter().map(|f| attach::display_path(&cwd, f)).collect()
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
fn folder_list(dir: String) -> crate::folders::Listing {
    crate::folders::list(&dir)
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

/// The taskbar and title icon, drawn by the UI from the logo picked in Settings > Appearance: `size`×`size` RGBA.
#[tauri::command]
fn set_app_icon<R: Runtime>(window: tauri::WebviewWindow<R>, rgba: Vec<u8>, size: u32) -> Result<(), String> {
    if size == 0 || size > 512 || rgba.len() != (size * size * 4) as usize {
        return Err("The icon must be square RGBA.".into());
    }
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

/// The ScoutCode server's image models, for Settings > Experimental.
#[tauri::command(async)]
fn image_models(core: Shared<'_>) -> Result<Value, String> {
    let cfg = core.lock().unwrap().scout_cfg();
    let models = crate::api::image_models(&cfg).map_err(|e| e.to_string())?;
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
    mcp_client::set(form.editing.as_deref(), &name, Some(c))?;
    let core = core.lock().unwrap();
    hub().reload(&core.waker(), false);
    core.wake();
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

#[tauri::command]
fn terminal_start(terminal: State<dock::Terminal>, cwd: String, rows: u16, cols: u16, output: Channel<InvokeResponseBody>, exited: Channel<bool>) -> Result<String, String> {
    terminal.start(&cwd, rows, cols, output, exited)
}

#[tauri::command]
fn terminal_write(terminal: State<dock::Terminal>, data: String) {
    terminal.write(&data);
}

#[tauri::command]
fn terminal_resize(terminal: State<dock::Terminal>, rows: u16, cols: u16) {
    terminal.resize(rows, cols);
}

#[tauri::command]
fn terminal_stop(terminal: State<dock::Terminal>) {
    terminal.stop();
}

