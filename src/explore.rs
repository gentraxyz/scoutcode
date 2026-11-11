//! Explore: read-only codebase research on a free model, so a paid model's plan isn't spent on searching and reading.
//! Scout's own `task` tool runs here when it's on, and Claude Code and Codex get it as an MCP tool, served by
//! `Scout.exe explore-mcp` (see `serve_stdio`). Each run ends with the files the caller needs, each checked to exist.
//! The tokens runs use are added up in `~/.scoutcode/explore.json`, for the note on a new chat.

use crate::api::{self, ChatRequest, Content, Delta, Message};
use crate::config::{self, Config};
use crate::tools::{self, Ctx};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// The MCP server's name, and so the tool's name in Claude Code: `mcp__scout_explore__explore`.
pub const SERVER: &str = "scout_explore";
pub const TOOL: &str = "explore";
/// The note on a new chat shows from this many tokens.
pub const SHOW_FROM: u64 = 100_000;
/// What it can call: the codebase only, nothing that writes or leaves the computer.
const TOOLS: [&str; 4] = ["view", "list_dir", "glob", "grep"];
const MAX_STEPS: usize = 40;

/// One model it runs on: the model menu's key and provider, and the id sent for it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExploreModel {
    pub key: String,
    pub provider: String,
    pub model: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub on: bool,
    /// Tried in order. Empty is the default: Qwen 3.8 27B on ScoutCode, then ScoutCode's free models.
    pub models: Vec<ExploreModel>,
}

impl Settings {
    pub fn chain(&self) -> Vec<ExploreModel> {
        let own: Vec<ExploreModel> = self.models.iter().filter(|m| runs_here(&m.provider)).cloned().collect();
        if !own.is_empty() {
            return own;
        }
        let scout = |m: &str| ExploreModel { key: m.into(), provider: "scoutcode".into(), model: m.into() };
        vec![scout("qwen3.8-27b"), scout("auto")]
    }
}

/// Whether a provider's models can be called straight from here, rather than through another app's own agent.
pub fn runs_here(provider: &str) -> bool {
    matches!(provider, "scoutcode" | "openrouter" | "nvidia" | "kilo") || provider == crate::local::PROVIDER || config::additional_provider(provider).is_some()
}

static ON: AtomicBool = AtomicBool::new(false);

/// Whether it's on, for what starts Claude Code and Codex. Kept up to date by the config.
pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

pub fn set_enabled(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

// ---------- the run ----------

/// What the caller hears as it goes.
pub enum Step {
    /// A tool call starting, as (tool, label).
    Call(String, String),
    /// That call finished: its summary and whether it failed.
    Done(Option<String>, bool),
    /// Its own words between calls.
    Note(String),
    /// Moving to the next model, and why.
    Fallback(String),
}

pub struct Report {
    /// The findings, then the files.
    pub text: String,
    /// The files it named or read that exist, relative to the folder, with any line range it gave.
    pub files: Vec<String>,
    pub tokens: u64,
    /// The model that wrote the report.
    pub model: String,
    pub calls: usize,
}

fn prompt(cwd: &str) -> String {
    format!(
        "You are Explore, a read-only research sub-agent for a coding agent that pays for every token it reads. Your job is \
to do the searching and reading for it, so it only has to open the files that matter.

You have view, list_dir, glob and grep. Search first (grep for names, glob for files), then view only the relevant line \
ranges. Read several files per view call with paths. Don't repeat a search or reread an unchanged range. Stop as soon as \
you can answer.

Then reply with one report, in this shape:
## Findings
Concrete answers to the question: how it works, where, and what to change, with path:line references. Say what you \
confirmed by reading and what you're inferring. No filler, no restating the question.
## Files
Every file the agent will need to read or change for this task, one per line, most important first, as \
`path:start-end - why` with paths relative to the working directory. Include the tests and config that matter. Only \
list files you actually saw.

Working directory: {cwd}
Operating system: {}",
        crate::prompt::os_name()
    )
}

/// The config to call `m` with, or why it can't be.
pub(crate) fn config_for(cfg: &Config, m: &ExploreModel, cancel: &AtomicBool) -> Result<Config, String> {
    let mut c = cfg.for_provider(&m.provider);
    if m.provider == crate::local::PROVIDER {
        let (port, key) = crate::local::ensure(&cfg.local, &m.model, cancel, &mut |_| {})?;
        c.base_url = format!("http://127.0.0.1:{port}/v1");
        c.api_key = Some(key);
    }
    if m.provider == "openrouter" && c.api_key.is_none() {
        return Err("OpenRouter has no key here".into());
    }
    Ok(c)
}

/// Researches `question` in `cwd` on the first model of the chain that answers, carrying the work so far on to the next
/// when one fails. Adds the tokens it used to the total.
pub fn run(cfg: &Config, cwd: &str, question: &str, cancel: &Arc<AtomicBool>, on: &mut dyn FnMut(Step)) -> Result<Report, String> {
    let chain = cfg.explore.chain();
    let specs = tools::specs(Some(&TOOLS), tools::EditTool::StrReplace);
    let mut messages = vec![Message::System { content: prompt(cwd) }, Message::User { content: Content::Text(question.into()) }];
    let (mut at, mut tokens, mut calls) = (0, 0u64, 0usize);
    let mut read_files: HashSet<PathBuf> = HashSet::new();
    let mut seen: Vec<String> = vec![];
    let mut last_error = String::new();
    let mut steps = 0;
    let finish = |text: String, tokens: u64, model: &str, calls: usize, seen: &[String]| -> Result<Report, String> {
        record(tokens);
        let files = files_in(cwd, &text, seen);
        Ok(Report { text: with_files(&text, &files), files, tokens, model: model.to_string(), calls })
    };
    while steps < MAX_STEPS {
        if cancel.load(Ordering::Relaxed) {
            record(tokens);
            return Err("Stopped.".into());
        }
        let Some(m) = chain.get(at) else {
            record(tokens);
            return Err(format!("No explore model answered. The last said: {last_error}"));
        };
        let c = match config_for(cfg, m, cancel) {
            Ok(c) => c,
            Err(e) => {
                last_error = e;
                at += 1;
                continue;
            }
        };
        let req = ChatRequest { model: &m.model, messages: &messages, tools: Some(&specs), tool_choice_none: false, temperature: None, effort: None, parallel_tool_calls: None };
        let r = match api::stream_chat(&c, req, &mut |_: Delta| {}, cancel) {
            Ok(r) => r,
            Err(e) if e.aborted => {
                record(tokens);
                return Err("Stopped.".into());
            }
            Err(e) => {
                last_error = e.message.chars().take(200).collect();
                at += 1;
                if let Some(next) = chain.get(at) {
                    on(Step::Fallback(format!("{} didn't answer ({}), trying {}", m.model, last_error.chars().take(80).collect::<String>(), next.model)));
                }
                continue;
            }
        };
        steps += 1;
        tokens += r.usage.as_ref().map(|u| u.prompt + u.completion).unwrap_or_else(|| estimate(&messages, &r.content));
        let tool_calls = r.tool_calls.clone();
        messages.push(Message::Assistant { content: (!r.content.is_empty()).then(|| r.content.clone()), tool_calls: (!tool_calls.is_empty()).then(|| tool_calls.clone()), reasoning_content: None, model: None, provider: None });
        if tool_calls.is_empty() {
            if r.content.trim().is_empty() {
                last_error = "an empty reply".into();
                at += 1;
                messages.pop();
                continue;
            }
            return finish(r.content, tokens, &m.model, calls, &seen);
        }
        if !r.content.trim().is_empty() {
            on(Step::Note(r.content.trim().to_string()));
        }
        for call in tool_calls {
            let args: Value = serde_json::from_str(if call.function.arguments.is_empty() { "{}" } else { &call.function.arguments }).unwrap_or(json!({}));
            let (name, args) = tools::canonical(&call.function.name, args);
            let out = if !TOOLS.contains(&name.as_str()) {
                format!("Error: Unknown tool {name}. You have {}.", TOOLS.join(", "))
            } else if let Some(p) = outside(cwd, &args) {
                format!("Error: {p} is outside the project ({cwd}). Explore only reads the project's own files.")
            } else {
                calls += 1;
                on(Step::Call(name.clone(), tools::label(&name, &args)));
                if name == "view" {
                    seen.extend(viewed(&args));
                }
                let mut noop = |_: &str| {};
                let mut ctx = Ctx { cwd, cancel, read_files: &mut read_files, on_progress: &mut noop };
                let (out, summary, failed) = match tools::run(&name, &args, &mut ctx) {
                    Ok(r) => (r.output, r.summary, r.is_error),
                    Err(e) => (format!("Error: {e}"), Some(e.lines().next().unwrap_or("").chars().take(100).collect()), true),
                };
                on(Step::Done(summary, failed));
                crate::redact::redact_owned(crate::agent::cap_output(out))
            };
            messages.push(Message::Tool { tool_call_id: call.id, content: out });
        }
    }
    // Out of steps: one last answer, without tools.
    if let Some(m) = chain.get(at) {
        messages.push(Message::User { content: Content::Text("You're out of steps. Write the report now from what you've found.".into()) });
        if let Ok(c) = config_for(cfg, m, cancel) {
            let req = ChatRequest { model: &m.model, messages: &messages, tools: Some(&specs), tool_choice_none: true, temperature: None, effort: None, parallel_tool_calls: None };
            if let Ok(r) = api::stream_chat(&c, req, &mut |_: Delta| {}, cancel) {
                tokens += r.usage.as_ref().map(|u| u.prompt + u.completion).unwrap_or_else(|| estimate(&messages, &r.content));
                if !r.content.trim().is_empty() {
                    return finish(r.content, tokens, &m.model, calls, &seen);
                }
            }
        }
    }
    record(tokens);
    Err("Explore ran out of steps without a report.".into())
}

pub(crate) fn estimate(messages: &[Message], reply: &str) -> u64 {
    let chars: usize = messages.iter().map(|m| serde_json::to_string(m).map(|s| s.len()).unwrap_or(0)).sum::<usize>() + reply.len();
    (chars / 4) as u64
}

/// A path in a call that leads out of the project, such as Scout's saved chats in the home folder. view checks this
/// itself; list_dir, glob and grep don't, so explore does for all of them.
pub(crate) fn outside(cwd: &str, args: &Value) -> Option<String> {
    let mut paths = viewed(args);
    paths.retain(|p| !p.is_empty() && p != ".");
    paths.into_iter().find(|p| !tools::within_project(cwd, p))
}

/// The files a view call read.
fn viewed(args: &Value) -> Vec<String> {
    let mut out: Vec<String> = args.get("paths").and_then(Value::as_array).into_iter().flatten().filter_map(|p| p.as_str().map(String::from)).collect();
    if let Some(p) = args.get("path").and_then(Value::as_str) {
        out.push(p.to_string());
    }
    out
}

/// A path as the caller should see it: relative to `cwd`, with forward slashes.
fn relative(cwd: &str, p: &Path) -> String {
    let rel = p.strip_prefix(cwd).unwrap_or(p);
    rel.to_string_lossy().replace('\\', "/").trim_start_matches('/').to_string()
}

/// The files the report names (with line ranges) that exist, then the files it read but didn't name, without repeats.
fn files_in(cwd: &str, report: &str, seen: &[String]) -> Vec<String> {
    let re = regex::Regex::new(r"(?:^|[\s`(\[*])((?:[A-Za-z]:)?[\w.@+\-/\\]*[\w\-]\.[A-Za-z0-9]{1,8})(?::(\d+)(?:\s*-\s*(\d+))?)?").unwrap();
    // path → ranges, in the order they first came up.
    let mut order: Vec<String> = vec![];
    let mut ranges: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut add = |path: &str, range: Option<String>| {
        let Ok(full) = tools::resolve(cwd, path) else { return };
        if !full.is_file() || !tools::within_project(cwd, path) {
            return;
        }
        let rel = relative(cwd, &full);
        if !ranges.contains_key(&rel) {
            order.push(rel.clone());
        }
        let list = ranges.entry(rel).or_default();
        if let Some(r) = range.filter(|r| !list.contains(r)) {
            list.push(r);
        }
    };
    for cap in re.captures_iter(report) {
        let path = &cap[1];
        let range = cap.get(2).map(|s| match cap.get(3) {
            Some(e) => format!("{}-{}", s.as_str(), e.as_str()),
            None => s.as_str().to_string(),
        });
        add(path, range);
    }
    for p in seen {
        add(p, None);
    }
    order.into_iter().map(|p| match ranges.get(&p).map(|r| merged(r)).filter(|r| !r.is_empty()) {
        Some(r) => format!("{p}:{}", r.join(",")),
        None => p,
    }).collect()
}

/// Line ranges without the ones another already covers, in the order they came.
fn merged(ranges: &[String]) -> Vec<String> {
    let span = |r: &str| -> Option<(u64, u64)> {
        let (a, b) = r.split_once('-').unwrap_or((r, r));
        Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
    };
    let spans: Vec<Option<(u64, u64)>> = ranges.iter().map(|r| span(r)).collect();
    ranges
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            let Some((a, b)) = spans[*i] else { return true };
            // Covered by another range, or the same range seen earlier.
            !spans.iter().enumerate().any(|(j, s)| j != *i && s.is_some_and(|(c, d)| c <= a && b <= d && ((c, d) != (a, b) || j < *i)))
        })
        .map(|(_, r)| r.clone())
        .collect()
}

/// The report with its file list replaced by the checked one.
fn with_files(report: &str, files: &[String]) -> String {
    let body = match report.find("## Files") {
        Some(i) => report[..i].trim_end(),
        None => report.trim_end(),
    };
    if files.is_empty() {
        return format!("{body}\n\n## Files\n(none of the files it named were found)");
    }
    // Its own reasons, where it gave them, by path.
    let reasons: Vec<(String, String)> = report
        .find("## Files")
        .map(|i| &report[i..])
        .unwrap_or("")
        .lines()
        .filter_map(|l| {
            let l = l.trim().trim_start_matches(['-', '*', ' ']).trim();
            let (path, why) = l.split_once(" - ").or_else(|| l.split_once(" — "))?;
            Some((path.trim_matches('`').split(':').next()?.replace('\\', "/").to_string(), why.trim().to_string()))
        })
        .collect();
    let lines: Vec<String> = files
        .iter()
        .map(|f| {
            let path = f.split(':').next().unwrap_or(f);
            match reasons.iter().find(|(p, _)| p.ends_with(path) || path.ends_with(p.as_str())) {
                Some((_, why)) => format!("- {f} - {why}"),
                None => format!("- {f}"),
            }
        })
        .collect();
    format!("{body}\n\n## Files\nChecked to exist, relative to the working directory:\n{}", lines.join("\n"))
}

// ---------- what it has saved ----------

fn stats_file() -> PathBuf {
    config::home().join("explore.json")
}

/// The total and when the file was last read, so a run in another process (Claude Code's server) shows up.
static TOTAL: AtomicU64 = AtomicU64::new(0);
static READ_AT: Mutex<Option<SystemTime>> = Mutex::new(None);

fn read_stats() -> (u64, u64) {
    let v: Value = std::fs::read_to_string(stats_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}));
    (v["tokens"].as_u64().unwrap_or(0), v["runs"].as_u64().unwrap_or(0))
}

/// Adds a run's tokens to the total.
pub fn record(tokens: u64) {
    if tokens == 0 {
        return;
    }
    let _guard = READ_AT.lock().unwrap();
    let (total, runs) = read_stats();
    let _ = std::fs::create_dir_all(config::home());
    let _ = std::fs::write(stats_file(), json!({"tokens": total + tokens, "runs": runs + 1}).to_string());
    TOTAL.store(total + tokens, Ordering::Relaxed);
}

/// The tokens explore runs have used so far, on every chat and app.
pub fn total() -> u64 {
    let mut read_at = READ_AT.lock().unwrap();
    let modified = std::fs::metadata(stats_file()).and_then(|m| m.modified()).ok();
    if modified.is_some() && *read_at != modified {
        *read_at = modified;
        TOTAL.store(read_stats().0, Ordering::Relaxed);
    }
    TOTAL.load(Ordering::Relaxed)
}

// ---------- for Claude Code and Codex ----------

pub const GUIDE: &str = "## Explore (saves the user's plan)
You have an `explore` tool (from the scout_explore MCP server). It runs a read-only research sub-agent on a free model, \
so its searching and reading cost the user nothing from their plan, while every file you read yourself does. It returns \
findings and a checked list of the files that matter, with line ranges.

You MUST use explore:
- before any task where you'll need to find or read 5 or more files, or don't yet know which files are involved;
- instead of launching your own Explore or research sub-agent, and instead of a run of Grep, Glob and Read calls to find \
where something is or how it works.
For bigger tasks, use it a lot: one call per area or question, several in the same message when they're independent, \
rather than one vague call. Give each call a complete question, since it can't see this conversation. Then read only the \
files it lists that you need to change, at the line ranges it gives.
Use your own Read and Grep only for a file you already know, or a quick check of fewer than 5 files. Don't use explore \
for a one-file change or for something you've already read.";

/// What Scout's own agent is told when its task tool runs here.
pub const SCOUT_GUIDE: &str = "# Explore (task runs on a free model)
The task tool's sub-agent now runs on a free model, so its searching and reading don't cost the user's plan or credit, \
while everything you read yourself does. It reports findings with path:line references and a checked list of the files \
that matter.
You MUST use task:
- before any work where you'll need to find or read 5 or more files, or don't yet know which files are involved;
- instead of a run of glob, grep and view calls to find where something is or how it works.
For bigger tasks, use it a lot: one call per area or question, several in the same response when they're independent, \
rather than one vague call. Give each a complete question, since it can't see this conversation. Then view only the files \
it lists that you need to change, at the ranges it gives. Search and read yourself only for a file you already know, or a \
quick check of fewer than 5 files.";

fn tool_spec() -> Value {
    json!({
        "name": TOOL,
        "description": "Research the codebase with a read-only sub-agent that runs on a free model, so it doesn't use the user's plan. Use it for any task that needs 5 or more files, and instead of your own searching or research sub-agents. Returns findings with path:line references and the files you need, each checked to exist. Ask one complete, specific question per call (it can't see this conversation); make several calls at once for separate areas.",
        "inputSchema": {"type": "object", "properties": {
            "question": {"type": "string", "description": "What to find out, with everything it needs to know: the task, names you already know, and what to report back."}
        }, "required": ["question"]}
    })
}

/// The MCP server to give a Claude Code or Codex session in `cwd`, as ACP's `mcpServers` takes it.
pub fn acp_server(cwd: &str) -> Option<Value> {
    if !enabled() {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    Some(json!({"name": SERVER, "command": exe.to_string_lossy(), "args": ["explore-mcp", "--cwd", cwd], "env": []}))
}

/// `Scout.exe explore-mcp --cwd <folder>`: serves the explore tool over stdio until the app that started it closes it.
pub fn serve_stdio(args: &[String]) {
    use std::io::{BufRead, Write};
    let flag = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).cloned();
    let cwd = flag("--cwd").or_else(|| std::env::current_dir().ok().map(|d| d.to_string_lossy().into_owned())).unwrap_or_default();
    let cfg = config::load_config();
    let cancel = Arc::new(AtomicBool::new(false));
    let out = Arc::new(Mutex::new(std::io::stdout()));
    let send = |out: &Arc<Mutex<std::io::Stdout>>, v: Value| {
        let mut o = out.lock().unwrap();
        let _ = writeln!(o, "{v}").and_then(|_| o.flush());
    };
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = msg.get("id").cloned() else { continue };
        let method = msg["method"].as_str().unwrap_or("");
        let result = match method {
            "initialize" => {
                let asked = msg["params"]["protocolVersion"].as_str().unwrap_or("");
                json!({
                    "protocolVersion": if crate::mcp::PROTOCOLS.contains(&asked) { asked } else { crate::mcp::PROTOCOLS[0] },
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": SERVER, "title": "Scout Explore", "version": crate::update::VERSION},
                    "instructions": GUIDE,
                })
            }
            "ping" => json!({}),
            "tools/list" => json!({"tools": [tool_spec()]}),
            "tools/call" if msg["params"]["name"] == TOOL => {
                // Calls run side by side, so several at once finish together.
                let (cfg, cwd, cancel, out) = (cfg.clone(), cwd.clone(), cancel.clone(), out.clone());
                let question = msg["params"]["arguments"]["question"].as_str().unwrap_or("").to_string();
                std::thread::spawn(move || {
                    let result = answer(&cfg, &cwd, &question, &cancel);
                    send(&out, json!({"jsonrpc": "2.0", "id": id, "result": result}));
                });
                continue;
            }
            _ => {
                send(&out, json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("Method not found: {method}")}}));
                continue;
            }
        };
        send(&out, json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }
    cancel.store(true, Ordering::Relaxed);
}

/// A tools/call result for one question.
fn answer(cfg: &Config, cwd: &str, question: &str, cancel: &Arc<AtomicBool>) -> Value {
    if question.trim().is_empty() {
        return json!({"content": [{"type": "text", "text": "Give explore a question."}], "isError": true});
    }
    match run(cfg, cwd, question, cancel, &mut |_| {}) {
        Ok(r) => json!({"content": [{"type": "text", "text": format!("{}\n\n(Explore read this on {} with {} tool calls and {} tokens, none from your plan.)", r.text, r.model, r.calls, r.tokens)}], "isError": false}),
        Err(e) => json!({"content": [{"type": "text", "text": format!("Explore couldn't run: {e}. Use your own tools for this one.")}], "isError": true}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_chain_is_qwen_then_free_models() {
        let chain = Settings::default().chain();
        assert_eq!(chain.iter().map(|m| m.model.as_str()).collect::<Vec<_>>(), ["qwen3.8-27b", "auto"]);
        let own = Settings { on: true, models: vec![ExploreModel { key: "x".into(), provider: "claude".into(), model: "x".into() }] };
        assert_eq!(own.chain().len(), 2, "a model that only runs in another app's agent is skipped");
    }

    #[test]
    fn files_are_checked_and_keep_their_ranges() {
        let dir = std::env::temp_dir().join(format!("scout-explore-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/a.rs"), "fn a() {}").unwrap();
        std::fs::write(dir.join("b.ts"), "").unwrap();
        let cwd = dir.to_string_lossy().to_string();
        let report = "## Findings\nIt's in `src/a.rs:10-20` and src/a.rs:3, not in src/missing.rs:4.\n## Files\n- src/a.rs:10-20 - the entry point\n- src/missing.rs - made up";
        let files = files_in(&cwd, report, &["b.ts".into()]);
        assert_eq!(files, ["src/a.rs:10-20,3", "b.ts"]);
        let text = with_files(report, &files);
        assert!(text.contains("- src/a.rs:10-20,3 - the entry point"));
        let list = &text[text.find("## Files").unwrap()..];
        assert!(!list.contains("missing"), "a file that doesn't exist isn't listed");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn it_cannot_search_outside_the_project() {
        let cwd = std::env::temp_dir().join(format!("scout-explore-out-{}", std::process::id()));
        std::fs::create_dir_all(cwd.join("src")).unwrap();
        let cwd = cwd.to_string_lossy().to_string();
        let home = config::home().join("sessions").to_string_lossy().to_string();
        assert!(outside(&cwd, &json!({"pattern": "password", "path": home})).is_some(), "Scout's saved chats");
        assert!(outside(&cwd, &json!({"path": "../.."})).is_some());
        assert!(outside(&cwd, &json!({"paths": ["src", "../other.txt"]})).is_some());
        assert!(outside(&cwd, &json!({"pattern": "fn main"})).is_none(), "no path is the project itself");
        assert!(outside(&cwd, &json!({"path": "src"})).is_none());
        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[test]
    fn ranges_inside_others_are_dropped() {
        let r = |v: &[&str]| merged(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(r(&["178-215", "184", "122-124", "122-135", "300"]), ["178-215", "122-135", "300"]);
        assert_eq!(r(&["5-9", "5-9"]), ["5-9"]);
    }
}
