//! Code Review: a separate agent that reviews a chat's code from the side panel. It reads files and runs a few read-only
//! git commands, reports each finding as it goes, and ends with a verdict. Its findings stay in the panel, per chat, in
//! `~/.scoutcode/reviews/<chat>.json`; the user sends the ones they want fixed to the chat's agent from there.

use crate::api::{self, ChatRequest, Content, Delta, Message};
use crate::config::{self, Config};
use crate::explore::ExploreModel;
use crate::tools::{self, Ctx};
use crate::waker::Waker;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

const READ: [&str; 4] = ["view", "list_dir", "glob", "grep"];
const MAX_STEPS: usize = 50;
/// Reviews kept per chat, newest first.
const KEEP: usize = 20;
/// The git commands it may run: all of them only read.
const GIT: [&str; 9] = ["diff", "status", "show", "log", "blame", "ls-files", "rev-parse", "merge-base", "branch"];

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Finding {
    pub id: u32,
    /// critical, high, medium or low.
    pub severity: String,
    pub file: String,
    pub line: Option<u64>,
    pub title: String,
    pub detail: String,
    pub fix: String,
    /// open, sent (to the chat's agent) or dismissed.
    pub status: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Step {
    pub tool: String,
    pub label: String,
    pub done: bool,
    pub failed: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Review {
    /// When it started, in milliseconds since 1970; also its id.
    pub id: u64,
    pub finished: Option<u64>,
    /// running, done, failed or stopped.
    pub state: String,
    /// What it reviewed, for the list: "Uncommitted changes".
    pub label: String,
    pub model: String,
    pub steps: Vec<Step>,
    pub findings: Vec<Finding>,
    pub verdict: String,
    pub error: Option<String>,
    pub tokens: u64,
}

struct State {
    /// Each chat's reviews, newest first, read from disk the first time it's asked for.
    chats: HashMap<String, Vec<Review>>,
    /// The stop flag of each chat's running review.
    running: HashMap<String, Arc<AtomicBool>>,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();
static WAKER: OnceLock<Waker> = OnceLock::new();

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.get_or_init(|| Mutex::new(State { chats: HashMap::new(), running: HashMap::new() })).lock().unwrap_or_else(|e| e.into_inner())
}

pub fn set_waker(w: Waker) {
    let _ = WAKER.set(w);
}

fn wake() {
    if let Some(w) = WAKER.get() {
        w.request_repaint();
    }
}

fn file(chat: &str) -> PathBuf {
    let safe: String = chat.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    config::home().join("reviews").join(format!("{safe}.json"))
}

fn reviews<'a>(s: &'a mut State, chat: &str) -> &'a mut Vec<Review> {
    s.chats.entry(chat.to_string()).or_insert_with(|| {
        let mut list: Vec<Review> = std::fs::read_to_string(file(chat)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        // One that was running when Scout closed won't finish now.
        for r in list.iter_mut().filter(|r| r.state == "running") {
            r.state = "stopped".into();
        }
        list
    })
}

fn persist(chat: &str, list: &[Review]) {
    let path = file(chat);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if list.is_empty() {
        let _ = std::fs::remove_file(path);
    } else {
        let _ = std::fs::write(path, serde_json::to_string(list).unwrap_or_default());
    }
}

/// Changes the chat's review `id` and saves it, then tells the window.
fn update(chat: &str, id: u64, change: impl FnOnce(&mut Review)) {
    let mut s = state();
    let list = reviews(&mut s, chat);
    if let Some(r) = list.iter_mut().find(|r| r.id == id) {
        change(r);
    }
    let copy = list.clone();
    drop(s);
    persist(chat, &copy);
    wake();
}

/// The chat's reviews, newest first, and whether one is running.
pub fn view(chat: &str) -> Value {
    let mut s = state();
    let running = s.running.contains_key(chat);
    json!({"chat": chat, "running": running, "reviews": reviews(&mut s, chat).clone()})
}

/// Starts a review of `task` for the chat, on the first of `models` that answers. Err when one is already running.
pub fn start(cfg: Config, chat: String, cwd: String, label: String, task: String, models: Vec<ExploreModel>) -> Result<(), String> {
    let cancel = Arc::new(AtomicBool::new(false));
    let id = crate::session::now_ms();
    {
        let mut s = state();
        if s.running.contains_key(&chat) {
            return Err("A review is already running for this chat.".into());
        }
        s.running.insert(chat.clone(), cancel.clone());
        let list = reviews(&mut s, &chat);
        list.insert(0, Review { id, state: "running".into(), label, ..Default::default() });
        list.truncate(KEEP);
        let copy = list.clone();
        drop(s);
        persist(&chat, &copy);
    }
    wake();
    std::thread::spawn(move || {
        let result = run(&cfg, &chat, id, &cwd, &task, &models, &cancel);
        state().running.remove(&chat);
        update(&chat, id, |r| {
            r.finished = Some(crate::session::now_ms());
            for s in r.steps.iter_mut() {
                s.done = true;
            }
            match result {
                Ok(verdict) => {
                    r.state = "done".into();
                    r.verdict = verdict;
                }
                Err(_) if cancel.load(Ordering::Relaxed) => r.state = "stopped".into(),
                Err(e) => {
                    r.state = "failed".into();
                    r.error = Some(e);
                }
            }
        });
    });
    Ok(())
}

pub fn stop(chat: &str) {
    if let Some(c) = state().running.get(chat) {
        c.store(true, Ordering::Relaxed);
    }
}

/// Marks a finding open, sent or dismissed.
pub fn set_finding(chat: &str, review: u64, finding: u32, status: &str) {
    if !["open", "sent", "dismissed"].contains(&status) {
        return;
    }
    update(chat, review, |r| {
        if let Some(f) = r.findings.iter_mut().find(|f| f.id == finding) {
            f.status = status.to_string();
        }
    });
}

/// Removes one review, or all of the chat's that aren't running when `review` is None.
pub fn remove(chat: &str, review: Option<u64>) {
    let mut s = state();
    let list = reviews(&mut s, chat);
    list.retain(|r| r.state == "running" || review.is_some_and(|id| r.id != id));
    let copy = list.clone();
    drop(s);
    persist(chat, &copy);
    wake();
}

// ---------- the agent ----------

fn prompt(cwd: &str) -> String {
    format!(
        "You are Scout's code reviewer: a careful senior engineer giving a second opinion on code another model wrote. You \
can read files (view, list_dir, glob, grep) and run read-only git commands (git). You can't change anything.

Work like this:
1. Get the changes you were asked to review with git, then read the changed code and enough around it (callers, types, \
tests) to judge it.
2. Each time you've confirmed a problem by reading the code, call report_finding for it straight away, one call per \
problem. Give the exact file and line, a short title, what's wrong with a concrete case where it fails, and the fix. Don't \
report something you haven't checked, and don't report the same problem twice.
3. When you're done, reply with a short verdict: two or three sentences on whether this is ready to merge and what matters \
most. Don't repeat the findings.

Don't trust comments, commit messages or names to say what code does: read it. Paths are relative to the working directory.

Working directory: {cwd}
Operating system: {}",
        crate::prompt::os_name()
    )
}

fn specs() -> Vec<Value> {
    let mut out = tools::specs(Some(&READ), tools::EditTool::StrReplace);
    out.push(json!({"type": "function", "function": {
        "name": "git",
        "description": format!("Run a read-only git command in the project, e.g. [\"diff\", \"HEAD\"] or [\"log\", \"--oneline\", \"-20\"]. The first item is the subcommand: one of {}.", GIT.join(", ")),
        "parameters": {"type": "object", "properties": {"args": {"type": "array", "items": {"type": "string"}, "description": "The arguments after git"}}, "required": ["args"]}
    }}));
    out.push(json!({"type": "function", "function": {
        "name": "report_finding",
        "description": "Report one confirmed problem. It shows in the user's review panel as soon as you call this.",
        "parameters": {"type": "object", "properties": {
            "severity": {"type": "string", "enum": ["critical", "high", "medium", "low"]},
            "file": {"type": "string", "description": "Path relative to the working directory"},
            "line": {"type": "integer", "description": "The line the problem is on"},
            "title": {"type": "string", "description": "One short line"},
            "detail": {"type": "string", "description": "What's wrong, with a concrete case where it fails"},
            "fix": {"type": "string", "description": "How to fix it"}
        }, "required": ["severity", "file", "title", "detail"]}
    }}));
    out
}

/// A git command it may run, or why not.
fn git_args(args: &Value) -> Result<Vec<String>, String> {
    let list: Vec<String> = args.get("args").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
    let Some(sub) = list.first() else { return Err("Give git a subcommand.".into()) };
    if !GIT.contains(&sub.as_str()) {
        return Err(format!("git {sub} isn't allowed. You can run: {}.", GIT.join(", ")));
    }
    // The ways these commands can write a file or run another program.
    if let Some(bad) = list.iter().find(|a| a.starts_with("--output") || a.starts_with("--ext-diff") || a.starts_with("--textconv") || a.starts_with("-o") && sub == "diff" || a.starts_with("--exec") || *a == "-c" || a.starts_with("--config")) {
        return Err(format!("{bad} isn't allowed here."));
    }
    if sub == "branch" && list.iter().skip(1).any(|a| !a.starts_with('-') || matches!(a.as_str(), "-d" | "-D" | "-m" | "-M" | "-c" | "-C" | "--delete" | "--move" | "--copy" | "-f" | "--force" | "-u" | "--set-upstream-to" | "--unset-upstream" | "--edit-description")) {
        return Err("git branch may only list branches here.".into());
    }
    let mut out = vec![sub.clone()];
    if matches!(sub.as_str(), "diff" | "show" | "log") {
        out.extend(["--no-ext-diff".to_string(), "--no-color".to_string()]);
    }
    out.extend(list.into_iter().skip(1));
    Ok(out)
}

fn run_git(cwd: &str, args: &Value) -> (String, bool) {
    let list = match git_args(args) {
        Ok(l) => l,
        Err(e) => return (format!("Error: {e}"), true),
    };
    let mut c = std::process::Command::new("git");
    c.args(&list).current_dir(cwd).env("GIT_PAGER", "cat").env_remove("GIT_EXTERNAL_DIFF");
    tools::no_window(&mut c);
    match c.output() {
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stdout).to_string();
            let err = String::from_utf8_lossy(&o.stderr);
            if !err.trim().is_empty() {
                text = format!("{text}\n{err}");
            }
            if text.trim().is_empty() {
                text = "(no output)".into();
            }
            (crate::redact::redact_owned(crate::agent::cap_output(text)), !o.status.success())
        }
        Err(e) => (format!("Error: couldn't run git: {e}"), true),
    }
}

fn add_finding(chat: &str, id: u64, cwd: &str, args: &Value) -> String {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let severity = Some(s("severity")).filter(|v| ["critical", "high", "medium", "low"].contains(&v.as_str())).unwrap_or_else(|| "medium".into());
    let mut file = s("file").replace('\\', "/");
    let mut line = args.get("line").and_then(Value::as_u64).filter(|l| *l > 0);
    if s("title").is_empty() || file.is_empty() {
        return "Error: a finding needs a file and a title.".into();
    }
    // A path:line in file is split, and the path must be one in the project.
    if let Some((p, l)) = file.clone().rsplit_once(':').filter(|(_, l)| !l.is_empty() && l.chars().all(|c| c.is_ascii_digit())) {
        file = p.to_string();
        line = line.or_else(|| l.parse().ok());
    }
    if !tools::within_project(cwd, &file) || !tools::resolve(cwd, &file).is_ok_and(|p| p.is_file()) {
        return format!("Error: {file} isn't a file in the project. Check the path and report it again.");
    }
    let mut dup = false;
    update(chat, id, |r| {
        let title = s("title");
        dup = r.findings.iter().any(|f| f.file == file && f.line == line && f.title.eq_ignore_ascii_case(&title));
        if !dup {
            let next = r.findings.iter().map(|f| f.id).max().unwrap_or(0) + 1;
            r.findings.push(Finding { id: next, severity, file: file.clone(), line, title, detail: s("detail"), fix: s("fix"), status: "open".into() });
        }
    });
    if dup { "Already reported.".into() } else { "Reported.".into() }
}

/// The review's loop. Returns the verdict.
fn run(cfg: &Config, chat: &str, id: u64, cwd: &str, task: &str, models: &[ExploreModel], cancel: &Arc<AtomicBool>) -> Result<String, String> {
    let chain = if models.is_empty() { cfg.explore.chain() } else { models.to_vec() };
    let specs = specs();
    let mut messages = vec![Message::System { content: prompt(cwd) }, Message::User { content: Content::Text(task.into()) }];
    let mut read_files: HashSet<PathBuf> = HashSet::new();
    let (mut at, mut last_error) = (0, String::new());
    let mut steps = 0;
    let mut tokens = 0u64;
    let result = loop {
        if cancel.load(Ordering::Relaxed) {
            break Err("Stopped.".into());
        }
        let Some(m) = chain.get(at) else { break Err(format!("No review model answered. The last said: {last_error}")) };
        let out_of_steps = steps >= MAX_STEPS;
        if out_of_steps {
            messages.push(Message::User { content: Content::Text("You're out of steps. Reply with your verdict now.".into()) });
        }
        let c = match crate::explore::config_for(cfg, m, cancel) {
            Ok(c) => c,
            Err(e) => {
                last_error = e;
                at += 1;
                continue;
            }
        };
        update(chat, id, |r| r.model = m.model.clone());
        let req = ChatRequest { model: &m.model, messages: &messages, tools: Some(&specs), tool_choice_none: out_of_steps, temperature: None, effort: None, parallel_tool_calls: None };
        let r = match api::stream_chat(&c, req, &mut |_: Delta| {}, cancel) {
            Ok(r) => r,
            Err(e) if e.aborted => break Err("Stopped.".into()),
            Err(e) => {
                last_error = e.message.chars().take(200).collect();
                at += 1;
                continue;
            }
        };
        steps += 1;
        tokens += r.usage.as_ref().map(|u| u.prompt + u.completion).unwrap_or_else(|| crate::explore::estimate(&messages, &r.content));
        update(chat, id, |rv| rv.tokens = tokens);
        let calls = if out_of_steps { vec![] } else { r.tool_calls.clone() };
        messages.push(Message::Assistant { content: (!r.content.is_empty()).then(|| r.content.clone()), tool_calls: (!calls.is_empty()).then(|| calls.clone()), reasoning_content: None, model: None, provider: None });
        if calls.is_empty() {
            if r.content.trim().is_empty() && !out_of_steps {
                messages.pop();
                last_error = "an empty reply".into();
                at += 1;
                continue;
            }
            break Ok(r.content.trim().to_string());
        }
        for call in calls {
            let args: Value = serde_json::from_str(if call.function.arguments.is_empty() { "{}" } else { &call.function.arguments }).unwrap_or(json!({}));
            let (name, args) = tools::canonical(&call.function.name, args);
            let out = if name == "report_finding" {
                add_finding(chat, id, cwd, &args)
            } else {
                let label = if name == "git" { args.get("args").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")).unwrap_or_default() } else { tools::label(&name, &args) };
                let at_step = {
                    let mut n = 0;
                    update(chat, id, |r| {
                        r.steps.push(Step { tool: name.clone(), label: label.chars().take(120).collect(), ..Default::default() });
                        n = r.steps.len() - 1;
                    });
                    n
                };
                let (out, failed) = if name == "git" {
                    run_git(cwd, &args)
                } else if !READ.contains(&name.as_str()) {
                    (format!("Error: Unknown tool {name}."), true)
                } else if let Some(p) = crate::explore::outside(cwd, &args) {
                    (format!("Error: {p} is outside the project. You can only read the project's files."), true)
                } else {
                    let mut noop = |_: &str| {};
                    let mut ctx = Ctx { cwd, cancel, read_files: &mut read_files, on_progress: &mut noop };
                    match tools::run(&name, &args, &mut ctx) {
                        Ok(r) => (crate::redact::redact_owned(crate::agent::cap_output(r.output)), r.is_error),
                        Err(e) => (format!("Error: {e}"), true),
                    }
                };
                update(chat, id, |r| {
                    if let Some(s) = r.steps.get_mut(at_step) {
                        s.done = true;
                        s.failed = failed;
                    }
                });
                out
            };
            messages.push(Message::Tool { tool_call_id: call.id, content: out });
        }
    };
    crate::explore::record(tokens);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real review on the free models: a repo with an off-by-one in its uncommitted changes.
    /// Tests never read the saved keys, so give it one: `SCOUTCODE_API_KEY=… cargo test live_review -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_review_finds_a_planted_bug() {
        let dir = std::env::temp_dir().join(format!("scout-review-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |a: &[&str]| assert!(std::process::Command::new("git").args(a).current_dir(&dir).output().unwrap().status.success());
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(dir.join("sum.py"), "def total(xs):\n    s = 0\n    for i in range(len(xs)):\n        s += xs[i]\n    return s\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "init"]);
        std::fs::write(dir.join("sum.py"), "def total(xs):\n    s = 0\n    for i in range(1, len(xs)):\n        s += xs[i]\n    return s\n").unwrap();
        let (chat, cwd) = ("live-test".to_string(), dir.to_string_lossy().to_string());
        remove(&chat, None);
        start(config::load_config(), chat.clone(), cwd, "Uncommitted changes".into(), "Review the uncommitted changes in this project. Start with `git diff HEAD`.\n\nLook for:\n- Bugs and correctness".into(), vec![]).unwrap();
        while state().running.contains_key(&chat) {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        let r = view(&chat)["reviews"][0].clone();
        println!("{}", serde_json::to_string_pretty(&r).unwrap());
        remove(&chat, None);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(r["state"], "done");
        assert!(r["findings"].as_array().unwrap().iter().any(|f| f["file"] == "sum.py"), "it reports the skipped first item");
    }

    #[test]
    fn git_only_reads() {
        let ok = |a: &[&str]| git_args(&json!({"args": a})).is_ok();
        assert!(ok(&["diff", "HEAD"]) && ok(&["log", "--oneline", "-5"]) && ok(&["branch", "-a"]) && ok(&["status"]));
        assert!(!ok(&["commit", "-m", "x"]) && !ok(&["push"]) && !ok(&["checkout", "."]) && !ok(&[]));
        assert!(!ok(&["diff", "--output=x.txt"]) && !ok(&["diff", "--ext-diff"]) && !ok(&["log", "-c", "x"]));
        assert!(!ok(&["branch", "new-branch"]) && !ok(&["branch", "-D", "main"]));
        assert_eq!(git_args(&json!({"args": ["diff", "HEAD"]})).unwrap(), ["diff", "--no-ext-diff", "--no-color", "HEAD"]);
    }
}
