use crate::diff::{self, DiffLine};
use regex::{Regex, RegexBuilder};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Todo {
    pub content: String,
    pub status: String,
}

/// What the UI shows under a tool call. The model only sees `output`.
#[derive(Clone, Debug)]
pub enum Display {
    Text(String),
    Diff { path: String, lines: Vec<DiffLine>, created: bool },
    /// One patch that changed several files: a Diff for each.
    Diffs(Vec<Display>),
    Todos(Vec<Todo>),
    /// An image a tool made: where it was saved, and a small copy as a data: URL to show.
    Image { path: String, preview: String },
}

#[derive(Clone, Debug, Default)]
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
    pub display: Option<Display>,
    pub summary: Option<String>,
}

impl ToolResult {
    pub fn ok(output: impl Into<String>, summary: impl Into<String>) -> Self {
        ToolResult { output: output.into(), summary: Some(summary.into()), ..Default::default() }
    }
    pub fn err(output: impl Into<String>, summary: impl Into<String>) -> Self {
        ToolResult { output: output.into(), is_error: true, summary: Some(summary.into()), ..Default::default() }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Permission {
    Read,
    Edit,
    Exec,
}

pub struct Ctx<'a> {
    pub cwd: &'a str,
    pub cancel: &'a AtomicBool,
    /// Absolute paths read this session, so edits of files the model never looked at are refused.
    pub read_files: &'a mut HashSet<PathBuf>,
    pub on_progress: &'a mut dyn FnMut(&str),
}

type R = Result<ToolResult, String>;

const IGNORED_DIRS: [&str; 12] = ["node_modules", ".git", "dist", "build", ".next", ".astro", ".cache", "__pycache__", ".venv", "venv", "target", "coverage"];
const MAX_LINE: usize = 2000;
const VIEW_LINES: usize = 1000;
const MAX_OUTPUT: usize = 30_000;

pub const READ_ONLY: [&str; 6] = ["view", "list_dir", "glob", "grep", "web_fetch", "web_search"];

/// What the names of tools from the user's MCP servers start with (see mcp_client.rs).
pub const MCP_PREFIX: &str = "mcp__";

pub fn is_mcp(name: &str) -> bool {
    name.starts_with(MCP_PREFIX)
}

/// How a model edits files, as /v1/models says for each (admins pick it): the edit tool it was trained on.
/// StrReplace is Anthropic's exact old→new text (edit_file, multi_edit); ApplyPatch is OpenAI's diff-style patch.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum EditTool {
    #[default]
    StrReplace,
    ApplyPatch,
}

impl EditTool {
    pub fn parse(s: &str) -> EditTool {
        if s == "apply_patch" { EditTool::ApplyPatch } else { EditTool::StrReplace }
    }
}

/// Tool specs, with the edit tools that suit the model. The list doesn't change with
/// the mode, since a change would void the prompt cache (switching models voids it anyway).
pub fn specs(only: Option<&[&str]>, edit: EditTool) -> Vec<Value> {
    let all = [
        ("view", "Show a file in the project, with line numbers. Long files come 1000 lines at a time; pass start to see further. A folder path lists the folder. To read several files in one call, pass paths instead of path (up to 20).",
            json!({"type":"object","properties":{"path":{"type":"string","description":"Path inside the project, e.g. src/index.ts"},"paths":{"type":"array","items":{"type":"string"},"description":"Several files to show at once, instead of path"},"start":{"type":"integer","description":"Optional line to start from (with path)"}}})),
        ("write_file", "Create a file, or overwrite one entirely. Prefer edit_file for changes to existing files. Existing files must be read first. Parent directories are created.",
            json!({"type":"object","properties":{"path":{"type":"string","description":"File path"},"content":{"type":"string","description":"The full file content"}},"required":["path","content"]})),
        ("apply_patch", crate::patch::DESCRIPTION,
            json!({"type":"object","properties":{"input":{"type":"string","description":"The patch, from *** Begin Patch to *** End Patch"}},"required":["input"]})),
        ("edit_file", "Replace an exact string in a file. old_string must match the file exactly (including indentation) and be unique unless replace_all is set. Read the file first. Line-number prefixes from view are not part of the file.",
            json!({"type":"object","properties":{"path":{"type":"string"},"old_string":{"type":"string","description":"Exact text to replace"},"new_string":{"type":"string","description":"Replacement text"},"replace_all":{"type":"boolean","description":"Replace every occurrence"}},"required":["path","old_string","new_string"]})),
        ("multi_edit", "Apply several exact-string edits to one file in order, all or nothing. Same rules as edit_file for each edit.",
            json!({"type":"object","properties":{"path":{"type":"string"},"edits":{"type":"array","items":{"type":"object","properties":{"old_string":{"type":"string"},"new_string":{"type":"string"},"replace_all":{"type":"boolean"}},"required":["old_string","new_string"]}}},"required":["path","edits"]})),
        ("list_dir", "List a directory: files with sizes and subdirectories (with a trailing /). Skips node_modules, .git and build output.",
            json!({"type":"object","properties":{"path":{"type":"string","description":"Directory, default the working directory"}}})),
        ("glob", "Find files by glob pattern, e.g. \"src/**/*.ts\" or \"*.json\". Returns paths sorted by most recently modified. Skips node_modules, .git and build output.",
            json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string","description":"Directory to search in, default the working directory"}},"required":["pattern"]})),
        ("grep", "Search file contents with a regular expression. output_mode \"files\" (default) lists matching files; \"content\" shows matching lines with line numbers; \"count\" shows counts per file. Filter files with glob (e.g. \"*.ts\").",
            json!({"type":"object","properties":{"pattern":{"type":"string","description":"Regular expression"},"path":{"type":"string","description":"File or directory, default the working directory"},"glob":{"type":"string","description":"Only search files matching this glob"},"output_mode":{"type":"string","enum":["files","content","count"]},"ignore_case":{"type":"boolean"},"context":{"type":"number","description":"Lines of context around matches (content mode)"},"max_results":{"type":"number","description":"Default 200"}},"required":["pattern"]})),
        ("bash", "Run a shell command in the working directory and return its output. Each call starts a fresh shell (cd does not carry over; chain with &&). Default timeout 2 minutes, max 10. Use run_in_background for servers and watchers, then read them with shell_output. Prefer the file tools over cat/sed/grep/find for reading and searching files.",
            json!({"type":"object","properties":{"command":{"type":"string"},"description":{"type":"string","description":"A few words on what the command does"},"timeout_ms":{"type":"number"},"run_in_background":{"type":"boolean"}},"required":["command"]})),
        ("shell_output", "Read new output from a background shell started with bash run_in_background, and whether it is still running.",
            json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"]})),
        ("kill_shell", "Stop a background shell.", json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"]})),
        ("web_fetch", "Fetch a URL (http/https) and return its text. HTML is converted to plain text. Use for docs, issues and API references.",
            json!({"type":"object","properties":{"url":{"type":"string"}},"required":["url"]})),
        ("web_search", "Search the web. Returns titles, URLs, dates and the sentences that best match the query; read a page with web_fetch. Use it for current information, docs you don't have a URL for, and error messages.",
            json!({"type":"object","properties":{"query":{"type":"string","description":"What to search for, in plain words"},"num_results":{"type":"integer","description":"1-10, default 5"},"include_domains":{"type":"array","items":{"type":"string"},"description":"Only these sites, e.g. [\"docs.rs\"]"},"exclude_domains":{"type":"array","items":{"type":"string"}},"start_published_date":{"type":"string","description":"Only pages published after this ISO date"},"category":{"type":"string","enum":["news","research paper","github","pdf","company","personal site"]}},"required":["query"]})),
        ("todo_write", "Keep a todo list for multi-step work. Send the whole list each time. Exactly one item should be in_progress while working. Use it for tasks with 3+ steps; skip it for simple ones.",
            json!({"type":"object","properties":{"todos":{"type":"array","items":{"type":"object","properties":{"content":{"type":"string"},"status":{"type":"string","enum":["pending","in_progress","completed"]}},"required":["content","status"]}}},"required":["todos"]})),
        ("task", "Hand a self-contained research task to a sub-agent with read-only tools (read, list, glob, grep, web_fetch, web_search). It returns one final report. Use it for broad searches across many files so your own context stays small. Give it a complete prompt: it cannot see this conversation.",
            json!({"type":"object","properties":{"description":{"type":"string","description":"3-5 words"},"prompt":{"type":"string","description":"The full task and what to report back"}},"required":["description","prompt"]})),
        ("present_plan", "Plan mode only: show the user your implementation plan (Markdown, concrete steps and files) and ask them to approve it. If they approve, you can start making changes.",
            json!({"type":"object","properties":{"plan":{"type":"string"}},"required":["plan"]})),
        ("ask_questions", "Ask the user one to four questions and wait for their answers. Use it when a decision or a detail only the user can give would otherwise be a guess. Give each question 2-4 short options; the user can always type their own answer instead. Ask only what you need.",
            json!({"type":"object","properties":{"questions":{"type":"array","maxItems":4,"items":{"type":"object","properties":{"question":{"type":"string","description":"The full question, ending in a question mark"},"header":{"type":"string","description":"A very short label, 16 characters or fewer"},"options":{"type":"array","maxItems":6,"items":{"type":"object","properties":{"label":{"type":"string","description":"1-5 words"},"description":{"type":"string","description":"What picking it means"}},"required":["label"]}},"multiSelect":{"type":"boolean","description":"Whether the user may pick several options"}},"required":["question","options"]}}},"required":["questions"]})),
        ("update_memory", "Change a memory that carries over between chats (its text is in the system prompt). add puts text at the end of a section, creating it if needed. replace and remove need old_text, copied exactly. Keep entries short and specific.",
            json!({"type":"object","properties":{"memory":{"type":"string","enum":["project","user"],"description":"project: facts about this repo, nothing personal. user: facts about the person."},"action":{"type":"string","enum":["add","replace","remove"]},"section":{"type":"string","description":"For add: the section name, e.g. Pitfalls"},"text":{"type":"string","description":"For add and replace: the new text, usually one bullet"},"old_text":{"type":"string","description":"For replace and remove: the exact text to change"}},"required":["memory","action"]})),
    ];
    all.into_iter()
        .filter(|(n, ..)| only.is_none_or(|o| o.contains(n)))
        .filter(|(n, ..)| match edit {
            EditTool::StrReplace => *n != "apply_patch",
            EditTool::ApplyPatch => !matches!(*n, "edit_file" | "multi_edit"),
        })
        .map(|(name, description, parameters)| json!({"type":"function","function":{"name":name,"description":description,"parameters":parameters}}))
        .collect()
}

/// Names models call grep by when they guess at a search tool.
const SEARCH_NAMES: [&str; 6] = ["search", "search_files", "search_code", "code_search", "grep_search", "find_in_files"];

/// The tool a call means and its arguments in that tool's terms. Models often call a code search "search" with a
/// `query`; that runs grep for the text as written (not a regex), ignoring case and showing the matching lines.
pub fn canonical(name: &str, args: Value) -> (String, Value) {
    if !SEARCH_NAMES.contains(&name) {
        return (name.to_string(), args);
    }
    let mut a = args.as_object().cloned().unwrap_or_default();
    if a.get("pattern").and_then(Value::as_str).is_none_or(str::is_empty) {
        let query = ["query", "q", "text", "search", "term", "regex"].iter().find_map(|k| a.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from));
        if let Some(q) = query {
            let literal = a.get("regex").and_then(Value::as_str) != Some(q.as_str());
            a.insert("pattern".into(), json!(if literal { regex::escape(&q) } else { q }));
            a.entry("ignore_case").or_insert(json!(true));
        }
    }
    a.entry("output_mode").or_insert(json!("content"));
    if a.get("path").is_none() {
        if let Some(d) = ["directory", "dir", "folder"].iter().find_map(|k| a.get(*k).cloned()) {
            a.insert("path".into(), d);
        }
    }
    ("grep".to_string(), Value::Object(a))
}

pub fn known(name: &str) -> bool {
    matches!(name, "view" | "write_file" | "edit_file" | "multi_edit" | "apply_patch" | "list_dir" | "glob" | "grep" | "bash" | "shell_output" | "kill_shell" | "web_fetch" | "web_search" | "todo_write" | "task" | "present_plan" | "ask_questions" | "update_memory" | "generate_image")
}

/// Experimental, only offered when Settings > Experimental turns it on: makes an image on the ScoutCode server.
pub fn image_spec() -> Value {
    json!({"type":"function","function":{"name":"generate_image","description":"Make an image from a text description and save it in the project, e.g. an icon, illustration, mockup or placeholder art. Describe the subject, style, colors and composition in detail. Each call makes one image and costs money, so make what's asked for, not variations nobody requested. It can't edit an existing image or write exact text reliably.",
        "parameters":{"type":"object","properties":{
            "prompt":{"type":"string","description":"What the image shows, in detail"},
            "path":{"type":"string","description":"Where to save it, a new file in the project, e.g. assets/hero.png. .png, .jpg or .webp"},
            "aspect_ratio":{"type":"string","enum":["1:1","2:3","3:2","3:4","4:3","4:5","5:4","9:16","16:9","21:9"],"description":"Default 1:1"}
        },"required":["prompt","path"]}}})
}

pub fn permission(name: &str) -> Permission {
    match name {
        "write_file" | "edit_file" | "multi_edit" | "apply_patch" | "update_memory" | "generate_image" => Permission::Edit,
        "bash" => Permission::Exec,
        // Anything could be behind one, so it's asked about like a command.
        n if is_mcp(n) => Permission::Exec,
        _ => Permission::Read,
    }
}

/// The files an edit tool will change, for undo.
pub fn edit_paths(name: &str, a: &Value) -> Vec<String> {
    match name {
        "apply_patch" => crate::patch::paths(a),
        _ => a.get("path").and_then(Value::as_str).map(|p| vec![p.to_string()]).unwrap_or_default(),
    }
}

fn s<'a>(a: &'a Value, k: &str) -> &'a str {
    a.get(k).and_then(Value::as_str).unwrap_or("")
}

/// One line describing the call, e.g. "src/app.ts" or "npm test".
pub fn label(name: &str, a: &Value) -> String {
    match name {
        "view" => match view_paths(a) {
            Some(paths) => paths.join(", "),
            None => view_args(a).map(|v| format!("{}{}", v.path, v.start.map(|s| format!(":{s}")).unwrap_or_default())).unwrap_or_default(),
        },
        "write_file" | "edit_file" => s(a, "path").into(),
        "multi_edit" => format!("{} ({} edits)", s(a, "path"), a.get("edits").and_then(Value::as_array).map_or(0, Vec::len)),
        "apply_patch" => crate::patch::label(a),
        "list_dir" => if s(a, "path").is_empty() { ".".into() } else { s(a, "path").into() },
        "glob" => format!("{}{}", s(a, "pattern"), if s(a, "path").is_empty() { String::new() } else { format!(" in {}", s(a, "path")) }),
        "grep" => format!(
            "/{}/{}{}",
            s(a, "pattern"),
            if s(a, "glob").is_empty() { String::new() } else { format!(" {}", s(a, "glob")) },
            if s(a, "path").is_empty() { String::new() } else { format!(" in {}", s(a, "path")) }
        ),
        "bash" => s(a, "command").into(),
        "shell_output" | "kill_shell" => s(a, "id").into(),
        "web_fetch" => s(a, "url").into(),
        "web_search" => s(a, "query").into(),
        "generate_image" => s(a, "path").into(),
        "todo_write" => format!("{} items", a.get("todos").and_then(Value::as_array).map_or(0, Vec::len)),
        "task" => s(a, "description").into(),
        "present_plan" => "plan".into(),
        "update_memory" => format!("{} memory", s(a, "memory")),
        "ask_questions" => {
            let qs = a.get("questions").and_then(Value::as_array);
            let first = qs
                .and_then(|q| q.first())
                .map(|q| s(q, "question"))
                .unwrap_or_default();
            match qs.map_or(0, Vec::len) {
                0 | 1 => first.to_string(),
                n => format!("{first} (+{} more)", n - 1),
            }
        }
        n if is_mcp(n) => a.to_string().chars().take(160).collect(),
        _ => String::new(),
    }
}

/// Runs a file, shell or web tool. todo_write, task, present_plan, ask_questions and web_search belong to the agent.
pub fn run(name: &str, a: &Value, ctx: &mut Ctx) -> R {
    match name {
        "view" => view(a, ctx),
        "write_file" => write_file(a, ctx),
        "edit_file" => run_edits(s(a, "path"), &[a.clone()], ctx),
        "multi_edit" => {
            let edits = a.get("edits").and_then(Value::as_array).cloned().unwrap_or_default();
            if edits.is_empty() {
                return Err("edits must be a non-empty array.".into());
            }
            run_edits(s(a, "path"), &edits, ctx)
        }
        "apply_patch" => crate::patch::run(a, ctx),
        "list_dir" => list_dir(a, ctx),
        "glob" => glob(a, ctx),
        "grep" => grep(a, ctx),
        "bash" => bash(a, ctx),
        "shell_output" => shell_output(a),
        "kill_shell" => kill_shell(a),
        "web_fetch" => web_fetch(a, ctx),
        "update_memory" => crate::memory::run(a, ctx),
        _ => Err(format!("Unknown tool {name}. To search the code, use grep.")),
    }
}

/// The diff an edit would make, for the permission prompt.
pub fn preview(name: &str, a: &Value, ctx: &Ctx) -> Option<Display> {
    match name {
        "write_file" => {
            let abs = resolve(ctx.cwd, s(a, "path")).ok()?;
            let before = read_text(&abs).map(|(t, _)| t).unwrap_or_default();
            let content = s(a, "content").replace("\r\n", "\n");
            Some(Display::Diff { path: rel(ctx.cwd, &abs), lines: diff::diff_lines(&before, &content), created: before.is_empty() })
        }
        "edit_file" | "multi_edit" => {
            let edits = if name == "edit_file" { vec![a.clone()] } else { a.get("edits").and_then(Value::as_array).cloned().unwrap_or_default() };
            Some(match edit_plan(ctx.cwd, s(a, "path"), &edits) {
                Ok((abs, text, next, _)) => Display::Diff { path: rel(ctx.cwd, &abs), lines: diff::diff_lines(&text, &next), created: false },
                Err(e) => Display::Text(format!("This edit will fail: {e}")),
            })
        }
        "apply_patch" => Some(crate::patch::preview(a, ctx)),
        "update_memory" => Some(crate::memory::preview(a, ctx)),
        "generate_image" => Some(Display::Text(format!("Make an image (this costs money) and save it to {}:\n\n{}", s(a, "path"), s(a, "prompt")))),
        _ => None,
    }
}

// ---------- paths and files ----------

/// Joins and normalises without touching the disk, like Node's path.resolve.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

/// The path a model sent, in this machine's terms: ~ is the home folder, and on Windows Git Bash's /c/... (what the bash
/// tool prints) is C:\..., while a bare /src means the project's src rather than the root of the drive.
fn native(p: &str) -> PathBuf {
    if p == "~" || p.starts_with("~/") || p.starts_with("~\\") {
        if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
            return PathBuf::from(home).join(p[1..].trim_start_matches(['/', '\\']));
        }
    }
    if cfg!(windows) {
        static DRIVE: OnceLock<Regex> = OnceLock::new();
        if let Some(c) = DRIVE.get_or_init(|| Regex::new(r"^/(?:mnt/|cygdrive/)?([A-Za-z])(?:/|$)").unwrap()).captures(p) {
            return PathBuf::from(format!("{}:\\{}", c[1].to_uppercase(), &p[c[0].len()..]));
        }
        if p.starts_with(['/', '\\']) && !p.starts_with("//") && !p.starts_with(r"\\") {
            return PathBuf::from(p.trim_start_matches(['/', '\\']));
        }
    }
    PathBuf::from(p)
}

pub fn resolve(cwd: &str, p: &str) -> Result<PathBuf, String> {
    if p.is_empty() {
        return Err("A path is required.".into());
    }
    let abs = normalize(&Path::new(cwd).join(native(p)));
    // Spelled as on disk below the project, so src/App.ts and src/app.ts are one file to the view-before-edit check.
    Ok(match canon(&abs).strip_prefix(canon(Path::new(cwd))) {
        Ok(r) if r.as_os_str().is_empty() => PathBuf::from(cwd),
        Ok(r) => Path::new(cwd).join(r),
        Err(_) => abs,
    })
}

pub fn rel(cwd: &str, abs: &Path) -> String {
    match abs.strip_prefix(cwd) {
        Ok(r) if !r.as_os_str().is_empty() => r.to_string_lossy().replace('\\', "/"),
        _ => abs.to_string_lossy().to_string(),
    }
}

fn is_binary(b: &[u8]) -> bool {
    b.iter().take(8000).any(|&c| c == 0)
}

/// Normalises CRLF so models can match strings, and remembers whether the file used it.
pub(crate) fn read_text(abs: &Path) -> Option<(String, bool)> {
    let raw = std::fs::read_to_string(abs).ok()?;
    let crlf = raw.contains("\r\n");
    Some((if crlf { raw.replace("\r\n", "\n") } else { raw }, crlf))
}

pub(crate) fn write_text(abs: &Path, text: &str, crlf: bool) -> Result<(), String> {
    if let Some(d) = abs.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(abs, if crlf { text.replace('\n', "\r\n") } else { text.to_string() }).map_err(|e| e.to_string())
}

/// Files view refuses even inside the project: env files, keys, credentials and git internals.
pub fn is_secret(rel_path: &str) -> bool {
    let parts: Vec<&str> = rel_path.split('/').collect();
    let name = parts.last().unwrap_or(&"").to_lowercase();
    if parts.contains(&".git") {
        return true;
    }
    if (name == ".env" || name.starts_with(".env.")) && !["example", "sample", "template", "dist"].iter().any(|x| name.ends_with(&format!(".{x}"))) {
        return true;
    }
    if [".pem", ".key", ".p12", ".pfx", ".jks", ".keystore"].iter().any(|x| name.ends_with(x)) {
        return true;
    }
    if ["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519"].iter().any(|x| name.starts_with(x)) {
        return true;
    }
    [".npmrc", ".netrc", ".pypirc", ".htpasswd", "credentials.json", "secrets.json"].contains(&name.as_str())
}

struct ViewArgs {
    path: String,
    start: Option<usize>,
    line: Option<usize>,
}

/// Takes the path however a model sends it: other key names, quotes, an @ from a mention, file://, or a :line suffix.
fn view_args(a: &Value) -> Result<ViewArgs, String> {
    let raw = a.as_str().or_else(|| ["path", "file_path", "filepath", "file", "filename"].iter().find_map(|k| a.get(*k).and_then(Value::as_str)));
    let raw = raw.map(str::trim).filter(|r| !r.is_empty()).ok_or("Give the file to view as path, e.g. {\"path\": \"src/index.ts\"}.")?;
    let p = raw.trim_matches(['"', '\'', '`']).trim_start_matches('@').trim_start_matches("file://").to_string();
    let start = ["start", "offset", "line"].iter().find_map(|k| a.get(*k).and_then(Value::as_f64)).filter(|n| *n >= 1.0).map(|n| n as usize);
    static LINE: OnceLock<Regex> = OnceLock::new();
    let line = LINE.get_or_init(|| Regex::new(r"^(.+?):(\d+)(?::\d+)?$").unwrap()).captures(&p).and_then(|c| c[2].parse().ok());
    Ok(ViewArgs { path: p, start, line })
}

/// The file a view call showed, for pruning.
pub fn viewed_path(name: &str, args: &Value, cwd: &str) -> Option<PathBuf> {
    if name != "view" {
        return None;
    }
    let v = view_args(args).ok()?;
    static SUFFIX: OnceLock<Regex> = OnceLock::new();
    let p = SUFFIX.get_or_init(|| Regex::new(r"(?::\d+){1,2}$").unwrap()).replace(&v.path, "");
    resolve(cwd, &p).ok()
}

fn find_by_name(root: &Path, name: &str) -> Vec<String> {
    let want = name.to_lowercase();
    let mut hits = vec![];
    let mut stack = vec![root.to_path_buf()];
    let mut budget = 8000i64;
    while let Some(dir) = stack.pop() {
        if hits.len() >= 5 || budget <= 0 {
            break;
        }
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            budget -= 1;
            let n = e.file_name().to_string_lossy().to_string();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                if !IGNORED_DIRS.contains(&n.as_str()) {
                    stack.push(e.path());
                }
            } else if n.to_lowercase() == want {
                hits.push(rel(&root.to_string_lossy(), &e.path()));
            }
        }
    }
    hits
}

fn canon(p: &Path) -> PathBuf {
    // A path that doesn't exist yet is resolved through its nearest folder that does, so a short (8.3) or linked
    // project path still contains it.
    let c = std::fs::canonicalize(p).unwrap_or_else(|_| match (p.parent(), p.file_name()) {
        (Some(dir), Some(name)) if dir != p => canon(dir).join(name),
        _ => p.to_path_buf(),
    });
    // Strip Windows' \\?\ prefix so it compares with plain paths.
    let s = c.to_string_lossy();
    PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s))
}

/// The files of a view call that passed paths.
fn view_paths(a: &Value) -> Option<Vec<String>> {
    let paths = a.get("paths")?.as_array()?;
    Some(paths.iter().filter_map(Value::as_str).map(String::from).collect())
}

/// Several files in one call, each under its own heading. Only an error when none of them could be shown.
fn view_many(paths: &[String], ctx: &mut Ctx) -> R {
    if paths.is_empty() {
        return Err("paths is empty. Give the files, e.g. {\"paths\": [\"src/a.ts\", \"src/b.ts\"]}.".into());
    }
    let mut out = vec![];
    let mut shown = 0;
    for p in paths.iter().take(20) {
        match view(&json!({ "path": p }), ctx) {
            Ok(r) => {
                shown += 1;
                out.push(format!("==> {p} <==\n{}", r.output));
            }
            Err(e) => out.push(format!("==> {p} <==\nError: {e}")),
        }
    }
    if paths.len() > 20 {
        out.push(format!("({} more not shown: view takes 20 files at a time.)", paths.len() - 20));
    }
    let out = out.join("\n\n");
    if shown == 0 { Err(out) } else { Ok(ToolResult::ok(out, format!("{shown} files"))) }
}

fn view(a: &Value, ctx: &mut Ctx) -> R {
    if let Some(paths) = view_paths(a) {
        return view_many(&paths, ctx);
    }
    let v = view_args(a)?;
    let mut abs = resolve(ctx.cwd, &v.path)?;
    let mut meta = std::fs::metadata(&abs).ok();
    let mut start = v.start;
    if meta.is_none() && v.line.is_some() {
        // "src/app.ts:120" means line 120 of src/app.ts.
        abs = viewed_path("view", &json!({ "path": v.path }), ctx.cwd).unwrap_or(abs);
        meta = std::fs::metadata(&abs).ok();
        start = start.or(v.line);
    }
    // Symlinks are resolved first, so a link can't lead out of the project.
    let root = canon(Path::new(ctx.cwd));
    let real = canon(&abs);
    let inside = match real.strip_prefix(&root) {
        Ok(r) => r.to_string_lossy().replace('\\', "/"),
        // The user attached it, which is permission to read it.
        Err(_) if ctx.read_files.contains(&abs) => abs.to_string_lossy().to_string(),
        Err(_) => return Err(format!("{} is outside the project ({}). view only shows files inside it.", v.path, ctx.cwd)),
    };
    let shown = if inside.is_empty() { ".".to_string() } else { inside };
    if is_secret(&shown) {
        return Err(format!("{shown} may hold secrets, so view won't show it."));
    }
    let Some(meta) = meta else {
        let hits = find_by_name(&root, &abs.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        return Err(if hits.is_empty() {
            format!("No file at {shown}. Check the folder with a folder path, e.g. {{\"path\": \".\"}}.")
        } else {
            format!("No file at {shown}. Did you mean: {}?", hits.join(", "))
        });
    };
    if meta.is_dir() {
        let mut entries: Vec<(bool, String)> = std::fs::read_dir(&abs)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| (e.file_type().is_ok_and(|t| t.is_dir()), e.file_name().to_string_lossy().to_string()))
            .filter(|(_, n)| !is_secret(&format!("{shown}/{n}")))
            .collect();
        entries.sort_by(|x, y| y.0.cmp(&x.0).then_with(|| x.1.cmp(&y.1)));
        let mut list: Vec<String> = entries.iter().take(300).map(|(d, n)| if *d { format!("{n}/") } else { n.clone() }).collect();
        if entries.len() > 300 {
            list.push(format!("… {} more", entries.len() - 300));
        }
        let out = if list.is_empty() { "(empty folder)".into() } else { list.join("\n") };
        return Ok(ToolResult::ok(out, format!("{} entries", entries.len())));
    }
    if meta.len() > 20 * 1024 * 1024 {
        return Err(format!("{shown} is over 20 MB. Use grep to find the part you need."));
    }
    let buf = std::fs::read(&abs).map_err(|e| e.to_string())?;
    if is_binary(&buf) {
        return Err(format!("{shown} is a binary file ({} bytes), so there is no text to show.", meta.len()));
    }
    let text = String::from_utf8_lossy(&buf).replace("\r\n", "\n");
    let lines: Vec<&str> = text.split('\n').collect();
    let from = start.unwrap_or(1).min(lines.len()).max(1);
    let slice = &lines[from - 1..(from - 1 + VIEW_LINES).min(lines.len())];
    ctx.read_files.insert(abs.clone());
    let end = from + slice.len() - 1;
    let width = end.to_string().len();
    let mut out = String::with_capacity(text.len() + slice.len() * 8);
    for (i, l) in slice.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("{:>width$}\t", from + i));
        if l.len() > MAX_LINE {
            out.push_str(&l.chars().take(MAX_LINE).collect::<String>());
            out.push_str("…[line truncated]");
        } else {
            out.push_str(l);
        }
    }
    if end < lines.len() {
        out.push_str(&format!("\n\n(lines {from}-{end} of {}; view again with start: {} for more)", lines.len(), end + 1));
    }
    Ok(ToolResult::ok(out, format!("{} lines", slice.len())))
}

/// A file's exact text for Devin, which keeps what it reads to write it back: unlike `view`, no line numbers and no
/// cut-off long lines. The same files are off limits. `line` is 1-based, and with `limit` picks a range of lines.
pub fn read_raw(cwd: &str, path: &str, line: Option<usize>, limit: Option<usize>) -> Result<(PathBuf, String), String> {
    let abs = resolve(cwd, path)?;
    let meta = std::fs::metadata(&abs).map_err(|_| format!("No file at {path}."))?;
    // Symlinks are resolved first, so a link can't lead out of the project.
    let Ok(inside) = canon(&abs).strip_prefix(canon(Path::new(cwd))).map(|r| r.to_string_lossy().replace('\\', "/")) else {
        return Err(format!("{path} is outside the project ({cwd}), so Scout won't read it."));
    };
    if is_secret(&inside) {
        return Err(format!("{inside} may hold secrets, so Scout won't read it."));
    }
    if meta.len() > 20 * 1024 * 1024 {
        return Err(format!("{inside} is over 20 MB."));
    }
    let buf = std::fs::read(&abs).map_err(|e| e.to_string())?;
    if is_binary(&buf) {
        return Err(format!("{inside} is a binary file ({} bytes), so there is no text to show.", meta.len()));
    }
    let text = String::from_utf8_lossy(&buf).to_string();
    if line.is_none() && limit.is_none() {
        return Ok((abs, text));
    }
    let lines = text.split_inclusive('\n').skip(line.unwrap_or(1).saturating_sub(1)).take(limit.unwrap_or(usize::MAX));
    Ok((abs, lines.collect()))
}

fn write_file(a: &Value, ctx: &mut Ctx) -> R {
    let abs = resolve(ctx.cwd, s(a, "path"))?;
    let had = abs.exists();
    if had && !ctx.read_files.contains(&abs) {
        return Err(format!("{} already exists. Open it with the view tool before overwriting it.", s(a, "path")));
    }
    let (before, crlf) = if had { read_text(&abs).unwrap_or_default() } else { (String::new(), false) };
    let content = s(a, "content").replace("\r\n", "\n");
    write_text(&abs, &content, crlf)?;
    ctx.read_files.insert(abs.clone());
    let lines = diff::diff_lines(&before, &content);
    let (add, del) = diff::stats(&lines);
    let n = content.split('\n').count();
    let r = rel(ctx.cwd, &abs);
    Ok(ToolResult {
        output: if had { format!("Overwrote {r} (+{add} -{del}).") } else { format!("Created {r} ({n} lines).") },
        is_error: false,
        summary: Some(if had { format!("+{add} -{del}") } else { format!("created, {n} lines") }),
        display: Some(Display::Diff { path: r, lines, created: !had }),
    })
}

fn apply_edit(text: &str, old: &str, new: &str, all: bool, file: &str) -> Result<String, String> {
    if old == new {
        return Err("old_string and new_string are the same.".into());
    }
    if old.is_empty() {
        return Err("old_string is empty. Use write_file to create a file.".into());
    }
    let count = text.matches(old).count();
    if count == 0 {
        let t = old.trim();
        let hint = if !t.is_empty() && text.contains(t) { " (it matches if you ignore surrounding whitespace: check indentation)" } else { "" };
        return Err(format!("old_string was not found in {file}{hint}. Read the file again and copy the text exactly."));
    }
    if count > 1 && !all {
        return Err(format!("old_string appears {count} times in {file}. Add surrounding lines to make it unique, or set replace_all."));
    }
    Ok(if all { text.replace(old, new) } else { text.replacen(old, new, 1) })
}

fn edit_plan(cwd: &str, path: &str, edits: &[Value]) -> Result<(PathBuf, String, String, bool), String> {
    let abs = resolve(cwd, path)?;
    let Some((text, crlf)) = read_text(&abs) else {
        return Err(format!("File not found: {path}. Use write_file to create it."));
    };
    let mut next = text.clone();
    for e in edits {
        let old = s(e, "old_string").replace("\r\n", "\n");
        let new = s(e, "new_string").replace("\r\n", "\n");
        next = apply_edit(&next, &old, &new, e.get("replace_all").and_then(Value::as_bool).unwrap_or(false), path)?;
    }
    Ok((abs, text, next, crlf))
}

fn run_edits(path: &str, edits: &[Value], ctx: &mut Ctx) -> R {
    let (abs, text, next, crlf) = edit_plan(ctx.cwd, path, edits)?;
    if !ctx.read_files.contains(&abs) {
        return Err(format!("Open {path} with the view tool before editing it."));
    }
    write_text(&abs, &next, crlf)?;
    let lines = diff::diff_lines(&text, &next);
    let (add, del) = diff::stats(&lines);
    let r = rel(ctx.cwd, &abs);
    Ok(ToolResult {
        output: format!("Edited {r} (+{add} -{del})."),
        is_error: false,
        summary: Some(format!("+{add} -{del}")),
        display: Some(Display::Diff { path: r, lines, created: false }),
    })
}

fn list_dir(a: &Value, ctx: &mut Ctx) -> R {
    let p = if s(a, "path").is_empty() { "." } else { s(a, "path") };
    let abs = resolve(ctx.cwd, p)?;
    let mut entries: Vec<_> = std::fs::read_dir(&abs).map_err(|_| format!("Not a directory: {p}"))?.flatten().collect();
    entries.sort_by_key(|e| (!e.file_type().is_ok_and(|t| t.is_dir()), e.file_name()));
    let mut lines = vec![];
    for e in entries.iter().take(500) {
        let n = e.file_name().to_string_lossy().to_string();
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            lines.push(format!("{n}/{}", if IGNORED_DIRS.contains(&n.as_str()) { "  (skipped by search)" } else { "" }));
        } else {
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            lines.push(if size < 1024 { format!("{n}  {size} B") } else { format!("{n}  {:.1} KB", size as f64 / 1024.0) });
        }
    }
    if entries.len() > 500 {
        lines.push(format!("… {} more", entries.len() - 500));
    }
    let out = if lines.is_empty() { "(empty directory)".into() } else { lines.join("\n") };
    Ok(ToolResult::ok(out, format!("{} entries", entries.len())))
}

/// Walks files under root, skipping ignored directories.
pub fn walk(root: &Path, cancel: &AtomicBool, mut f: impl FnMut(&Path) -> bool) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() {
                if !IGNORED_DIRS.contains(&e.file_name().to_string_lossy().as_ref()) {
                    stack.push(e.path());
                }
            } else if t.is_file() && !f(&e.path()) {
                return;
            }
        }
    }
}

/// Glob to regex: **, *, ?, {a,b} and [abc]. Patterns without a slash match the file name anywhere.
pub fn glob_to_regex(glob: &str) -> Result<Regex, String> {
    let mut g = glob.replace('\\', "/");
    if !g.contains('/') {
        g = format!("**/{g}");
    }
    let c: Vec<char> = g.chars().collect();
    let mut re = String::from("^");
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '*' if c.get(i + 1) == Some(&'*') => {
                i += 1;
                if c.get(i + 1) == Some(&'/') {
                    i += 1;
                    re += "(?:.*/)?";
                } else {
                    re += ".*";
                }
            }
            '*' => re += "[^/]*",
            '?' => re += "[^/]",
            '{' => match c[i..].iter().position(|&x| x == '}') {
                Some(len) => {
                    let inner: String = c[i + 1..i + len].iter().collect();
                    let alts: Vec<String> = inner.split(',').map(|s| regex::escape(s).replace(r"\*", "[^/]*")).collect();
                    re += &format!("(?:{})", alts.join("|"));
                    i += len;
                }
                None => re += r"\{",
            },
            '[' => match c[i..].iter().position(|&x| x == ']') {
                Some(len) => {
                    re.extend(&c[i..=i + len]);
                    i += len;
                }
                None => re += r"\[",
            },
            ch => re += &regex::escape(&ch.to_string()),
        }
        i += 1;
    }
    re += "$";
    RegexBuilder::new(&re).case_insensitive(cfg!(windows)).build().map_err(|e| e.to_string())
}

fn glob(a: &Value, ctx: &mut Ctx) -> R {
    let root = resolve(ctx.cwd, if s(a, "path").is_empty() { "." } else { s(a, "path") })?;
    let re = glob_to_regex(s(a, "pattern"))?;
    let mut hits = vec![];
    walk(&root, ctx.cancel, |f| {
        let r = f.strip_prefix(&root).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
        if re.is_match(&r) {
            let m = std::fs::metadata(f).and_then(|m| m.modified()).ok();
            hits.push((f.to_path_buf(), m));
        }
        hits.len() < 5000
    });
    hits.sort_by(|x, y| y.1.cmp(&x.1));
    let mut shown: Vec<String> = hits.iter().take(200).map(|(p, _)| rel(ctx.cwd, p)).collect();
    if hits.len() > 200 {
        shown.push(format!("… {} more; narrow the pattern", hits.len() - 200));
    }
    let out = if shown.is_empty() { "No files matched.".into() } else { shown.join("\n") };
    Ok(ToolResult::ok(out, format!("{} files", hits.len())))
}

fn grep(a: &Value, ctx: &mut Ctx) -> R {
    let re = RegexBuilder::new(s(a, "pattern"))
        .case_insensitive(a.get("ignore_case").and_then(Value::as_bool).unwrap_or(false))
        .build()
        .map_err(|e| format!("Invalid regular expression: {e}"))?;
    let target = resolve(ctx.cwd, if s(a, "path").is_empty() { "." } else { s(a, "path") })?;
    let meta = std::fs::metadata(&target).map_err(|_| format!("Not found: {}", s(a, "path")))?;
    let file_re = if s(a, "glob").is_empty() { None } else { Some(glob_to_regex(s(a, "glob"))?) };
    let mode = if s(a, "output_mode").is_empty() { "files" } else { s(a, "output_mode") };
    let max = a.get("max_results").and_then(Value::as_f64).map(|n| n.max(1.0) as usize).unwrap_or(200);
    let ctx_lines = a.get("context").and_then(Value::as_f64).map(|n| n.clamp(0.0, 10.0) as usize).unwrap_or(0);
    let base = if meta.is_file() { target.parent().map(Path::to_path_buf).unwrap_or_default() } else { target.clone() };
    let (mut out, mut files, mut total) = (Vec::<String>::new(), 0, 0);
    let cwd = ctx.cwd;
    let mut search = |f: &Path| -> bool {
        if let Some(fr) = &file_re {
            let r = f.strip_prefix(&base).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            if !fr.is_match(&r) {
                return true;
            }
        }
        if std::fs::metadata(f).is_ok_and(|m| m.len() > 5 * 1024 * 1024) {
            return true;
        }
        let Ok(buf) = std::fs::read(f) else { return true };
        if is_binary(&buf) {
            return true;
        }
        let text = String::from_utf8_lossy(&buf);
        let lines: Vec<&str> = text.lines().collect();
        let matched: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| re.is_match(l)).map(|(i, _)| i).collect();
        if matched.is_empty() {
            return true;
        }
        files += 1;
        total += matched.len();
        let name = rel(cwd, f);
        match mode {
            "count" => out.push(format!("{name}: {}", matched.len())),
            "content" => {
                let mut shown = std::collections::BTreeSet::new();
                for &i in &matched {
                    for k in i.saturating_sub(ctx_lines)..=(i + ctx_lines).min(lines.len() - 1) {
                        shown.insert(k);
                    }
                }
                let hit: HashSet<usize> = matched.iter().copied().collect();
                let mut prev: Option<usize> = None;
                for k in shown {
                    if prev.is_some_and(|p| k != p + 1) {
                        out.push("--".into());
                    }
                    let l = lines[k];
                    let l = if l.len() > 300 { format!("{}…", l.chars().take(300).collect::<String>()) } else { l.to_string() };
                    out.push(format!("{name}:{}{}{l}", k + 1, if hit.contains(&k) { ':' } else { '-' }));
                    prev = Some(k);
                }
            }
            _ => out.push(name),
        }
        out.len() < max
    };
    if meta.is_file() {
        search(&target);
    } else {
        walk(&target, ctx.cancel, search);
    }
    let truncated = out.len() >= max;
    out.truncate(max);
    let mut body = if out.is_empty() { "No matches.".to_string() } else { out.join("\n") };
    if truncated {
        body += &format!("\n\n(stopped at {max} results; narrow the search)");
    }
    Ok(ToolResult::ok(body, format!("{total} matches in {files} files")))
}

// ---------- shell ----------

struct Shell {
    file: String,
    powershell: bool,
}

/// bash everywhere it exists (Git Bash on Windows), otherwise PowerShell on Windows.
fn shell() -> &'static Shell {
    static SHELL: OnceLock<Shell> = OnceLock::new();
    SHELL.get_or_init(|| {
        if let Ok(f) = std::env::var("SCOUT_SHELL") {
            let ps = f.to_lowercase().contains("powershell") || f.to_lowercase().contains("pwsh");
            return Shell { file: f, powershell: ps };
        }
        if !cfg!(windows) {
            let f = if Path::new("/bin/bash").exists() { "/bin/bash" } else { "/bin/sh" };
            return Shell { file: f.into(), powershell: false };
        }
        let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        let candidates = [
            Path::new(&env("ProgramFiles", r"C:\Program Files")).join(r"Git\bin\bash.exe"),
            Path::new(&env("ProgramFiles(x86)", r"C:\Program Files (x86)")).join(r"Git\bin\bash.exe"),
            Path::new(&env("LOCALAPPDATA", "")).join(r"Programs\Git\bin\bash.exe"),
        ];
        let found = candidates.iter().find(|c| c.exists()).map(|c| c.to_string_lossy().to_string()).or_else(|| {
            let mut c = Command::new("where");
            c.arg("bash");
            no_window(&mut c);
            let out = c.output().ok()?;
            // Skip WSL's bash.exe in System32: it runs in a different filesystem.
            String::from_utf8_lossy(&out.stdout).lines().map(str::trim).find(|l| !l.is_empty() && !l.to_lowercase().contains("system32")).map(String::from)
        });
        match found {
            Some(f) => Shell { file: f, powershell: false },
            None => Shell { file: "powershell.exe".into(), powershell: true },
        }
    })
}

pub fn shell_name() -> &'static str {
    if shell().powershell { "powershell" } else { "bash" }
}

/// Keeps child processes from flashing a console window, since the app has none.
pub fn no_window(c: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    let _ = c;
}

fn start(command: &str, cwd: &str) -> std::io::Result<Child> {
    let sh = shell();
    let mut c = Command::new(&sh.file);
    if sh.powershell {
        c.args(["-NoProfile", "-Command", command]);
    } else {
        c.args(["-c", command]);
    }
    c.current_dir(cwd)
        .env("FORCE_COLOR", "0")
        .env("NO_COLOR", "1")
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .env("SCOUT", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    no_window(&mut c);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    c.spawn()
}

pub fn kill_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        let mut c = Command::new("taskkill");
        c.args(["/pid", &child.id().to_string(), "/t", "/f"]).stdout(Stdio::null()).stderr(Stdio::null());
        no_window(&mut c);
        let _ = c.status();
    }
    #[cfg(unix)]
    {
        let _ = Command::new("kill").args(["-KILL", &format!("-{}", child.id())]).status();
    }
    let _ = child.kill();
}

/// Copies a child's stdout and stderr into one shared buffer on two threads.
fn pump(child: &mut Child) -> Arc<Mutex<String>> {
    let buf = Arc::new(Mutex::new(String::new()));
    let streams: [Option<Box<dyn Read + Send>>; 2] =
        [child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>), child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>)];
    for mut s in streams.into_iter().flatten() {
        let b = buf.clone();
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = s.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                let mut g = b.lock().unwrap();
                g.push_str(&String::from_utf8_lossy(&chunk[..n]));
                // Keep the head and the tail of runaway output.
                if g.len() > 5 * MAX_OUTPUT {
                    let head: String = g.chars().take(MAX_OUTPUT).collect();
                    let tail: String = g.chars().rev().take(MAX_OUTPUT * 2).collect::<Vec<_>>().into_iter().rev().collect();
                    *g = format!("{head}\n…\n{tail}");
                }
            }
        });
    }
    buf
}

pub fn strip_ansi(s: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").unwrap()).replace_all(s, "").into_owned()
}

fn truncate(s: &str) -> String {
    if s.len() <= MAX_OUTPUT {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= MAX_OUTPUT {
        return s.to_string();
    }
    let half = MAX_OUTPUT / 2;
    let head: String = chars[..half].iter().collect();
    let tail: String = chars[chars.len() - half..].iter().collect();
    format!("{head}\n\n… [{} characters truncated] …\n\n{tail}", chars.len() - MAX_OUTPUT)
}

struct Background {
    command: String,
    child: Child,
    output: Arc<Mutex<String>>,
    read: usize,
    exit: Option<i32>,
}

fn background() -> &'static Mutex<HashMap<String, Background>> {
    static BG: OnceLock<Mutex<HashMap<String, Background>>> = OnceLock::new();
    BG.get_or_init(Default::default)
}

pub fn kill_background_shells() {
    for b in background().lock().unwrap().values_mut() {
        if b.exit.is_none() {
            kill_tree(&mut b.child);
        }
    }
}

fn bash(a: &Value, ctx: &mut Ctx) -> R {
    let command = s(a, "command").trim().to_string();
    if command.is_empty() {
        return Err("command is empty.".into());
    }
    let mut child = start(&command, ctx.cwd).map_err(|e| format!("Could not start {}: {e}", shell().file))?;
    let out = pump(&mut child);

    if a.get("run_in_background").and_then(Value::as_bool).unwrap_or(false) {
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        let id = format!("sh{}", NEXT.fetch_add(1, Ordering::Relaxed));
        let pid = child.id();
        background().lock().unwrap().insert(id.clone(), Background { command, child, output: out, read: 0, exit: None });
        return Ok(ToolResult::ok(
            format!("Started in the background as {id} (pid {pid}). Read its output with shell_output, stop it with kill_shell."),
            format!("background {id}"),
        ));
    }

    let timeout = Duration::from_millis(a.get("timeout_ms").and_then(Value::as_f64).unwrap_or(120_000.0).clamp(1000.0, 600_000.0) as u64);
    let started = Instant::now();
    let mut last_len = 0;
    let (code, why) = loop {
        if let Ok(Some(st)) = child.try_wait() {
            break (st.code().unwrap_or(-1), None);
        }
        if ctx.cancel.load(Ordering::Relaxed) {
            kill_tree(&mut child);
            break (-1, Some("interrupted"));
        }
        if started.elapsed() > timeout {
            kill_tree(&mut child);
            break (-1, Some("timed out"));
        }
        std::thread::sleep(Duration::from_millis(40));
        let g = out.lock().unwrap();
        if g.len() != last_len {
            last_len = g.len();
            let tail: Vec<&str> = g.lines().filter(|l| !l.trim().is_empty()).collect();
            let tail = tail[tail.len().saturating_sub(5)..].join("\n");
            drop(g);
            (ctx.on_progress)(&strip_ansi(&tail));
        }
    };
    // Let the reader threads take what is left in the pipes.
    std::thread::sleep(Duration::from_millis(30));
    let text = truncate(strip_ansi(&out.lock().unwrap()).trim_end());
    match why {
        Some("interrupted") => return Ok(ToolResult::err(format!("{text}\n\n[interrupted by the user]"), "interrupted")),
        Some(_) => return Ok(ToolResult::err(format!("{text}\n\n[timed out after {}s and was killed]", timeout.as_secs()), "timed out")),
        None => {}
    }
    let lines = if text.is_empty() { 0 } else { text.lines().count() };
    Ok(ToolResult {
        output: format!("{}{}", if text.is_empty() { "(no output)" } else { &text }, if code != 0 { format!("\n\n[exit code {code}]") } else { String::new() }),
        is_error: code != 0,
        summary: Some(if code != 0 { format!("exit {code}") } else { format!("{lines} lines") }),
        display: (!text.is_empty()).then(|| Display::Text(text)),
    })
}

fn shell_output(a: &Value) -> R {
    let mut bg = background().lock().unwrap();
    let keys: Vec<String> = bg.keys().cloned().collect();
    let b = bg.get_mut(s(a, "id")).ok_or_else(|| format!("No background shell {}. Running: {}", s(a, "id"), if keys.is_empty() { "none".into() } else { keys.join(", ") }))?;
    if b.exit.is_none() {
        if let Ok(Some(st)) = b.child.try_wait() {
            b.exit = Some(st.code().unwrap_or(-1));
        }
    }
    let all = b.output.lock().unwrap().clone();
    let fresh = strip_ansi(all.get(b.read.min(all.len())..).unwrap_or(""));
    b.read = all.len();
    let status = match b.exit {
        None => "running".to_string(),
        Some(c) => format!("exited with code {c}"),
    };
    let body = truncate(&fresh);
    Ok(ToolResult::ok(format!("[{} ({}): {status}]\n{}", s(a, "id"), b.command, if body.is_empty() { "(no new output)" } else { &body }), status))
}

fn kill_shell(a: &Value) -> R {
    let mut bg = background().lock().unwrap();
    let mut b = bg.remove(s(a, "id")).ok_or_else(|| format!("No background shell {}.", s(a, "id")))?;
    if b.exit.is_none() {
        kill_tree(&mut b.child);
    }
    Ok(ToolResult::ok(format!("Stopped {}.", s(a, "id")), "stopped"))
}

// ---------- web ----------

fn html_to_text(html: &str) -> String {
    static RES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let res = RES.get_or_init(|| {
        let mut v: Vec<(Regex, &str)> = ["script", "style", "noscript", "svg", "head"]
            .iter()
            .map(|t| (Regex::new(&format!(r"(?is)<{t}\b.*?</{t}>")).unwrap(), ""))
            .collect();
        v.extend([
            (Regex::new(r"(?i)<br\s*/?>").unwrap(), "\n"),
            (Regex::new(r"(?i)</(p|div|li|h[1-6]|tr|section|article|pre)>").unwrap(), "\n"),
            (Regex::new(r"(?i)<li[^>]*>").unwrap(), "- "),
            (Regex::new(r"(?i)<h1[^>]*>").unwrap(), "\n# "),
            (Regex::new(r"(?i)<h2[^>]*>").unwrap(), "\n## "),
            (Regex::new(r"(?i)<h[3-6][^>]*>").unwrap(), "\n### "),
            (Regex::new(r"<[^>]+>").unwrap(), ""),
        ]);
        v
    });
    let mut t = html.to_string();
    for (re, with) in res {
        t = re.replace_all(&t, *with).into_owned();
    }
    t = t.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&");
    static WS: OnceLock<(Regex, Regex)> = OnceLock::new();
    let (sp, nl) = WS.get_or_init(|| (Regex::new(r"[ \t]+").unwrap(), Regex::new(r"\n\s*\n\s*\n+").unwrap()));
    nl.replace_all(&sp.replace_all(&t, " "), "\n\n").trim().to_string()
}

fn web_fetch(a: &Value, _ctx: &mut Ctx) -> R {
    let url = s(a, "url");
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Only http and https URLs are supported.".into());
    }
    let res = crate::api::http()
        .get(url)
        .config()
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .header("Accept", "text/html,text/plain,application/json,*/*")
        .call()
        .map_err(|e| e.to_string())?;
    let status = res.status().as_u16();
    let kind = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    if !["text", "json", "xml", "javascript"].iter().any(|k| kind.contains(k)) {
        return Err(format!("Not a text response ({}).", if kind.is_empty() { "unknown type" } else { &kind }));
    }
    let mut text = res.into_body().with_config().limit(20 * 1024 * 1024).read_to_string().map_err(|e| e.to_string())?;
    if kind.contains("html") {
        text = html_to_text(&text);
    }
    if text.len() > 60_000 {
        text = text.chars().take(60_000).collect::<String>() + "\n\n[truncated]";
    }
    let kb = text.len() / 1024;
    Ok(ToolResult { output: format!("[{status} {url}]\n\n{text}"), is_error: status >= 400, summary: Some(format!("{status}, {kb} KB")), display: None })
}

/// Searches the web through ScoutCode's /search, which needs the ScoutCode settings, so the agent runs it rather than
/// `run`. Only the result list comes from the server; reading a page is web_fetch's job, on this computer.
pub fn web_search(cfg: &crate::config::Config, a: &Value) -> R {
    let query = s(a, "query").trim();
    if query.is_empty() {
        return Err("query is required.".into());
    }
    let mut body = json!({"query": query, "num_results": a.get("num_results").and_then(Value::as_u64).unwrap_or(5).clamp(1, 10), "highlights": true});
    for k in ["include_domains", "exclude_domains", "start_published_date", "category"] {
        if let Some(v) = a.get(k).filter(|v| !v.is_null()) {
            body[k] = v.clone();
        }
    }
    let res = crate::api::search(cfg, &body).map_err(|e| match e.status {
        404 => format!("Web search isn't available: the ScoutCode server at {} doesn't have it yet.", cfg.base_url),
        _ => format!("Search failed: {e}"),
    })?;
    let results = res.get("results").and_then(Value::as_array).cloned().unwrap_or_default();
    if results.is_empty() {
        return Ok(ToolResult::ok(format!("No results for \"{query}\"."), "no results"));
    }
    let mut out = format!("Results for \"{query}\". Use web_fetch to read a page.\n");
    for (i, r) in results.iter().enumerate() {
        let title = r.get("title").and_then(Value::as_str).filter(|t| !t.is_empty()).unwrap_or("(no title)");
        out += &format!("\n{}. {title}\n   {}\n", i + 1, s(r, "url"));
        if let Some(d) = r.get("published_date").and_then(Value::as_str) {
            out += &format!("   Published {}\n", d.get(..10).unwrap_or(d));
        }
        for h in r.get("highlights").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            out += &format!("   > {}\n", h.split_whitespace().collect::<Vec<_>>().join(" "));
        }
    }
    let n = results.len();
    Ok(ToolResult::ok(out, format!("{n} result{}", if n == 1 { "" } else { "s" })))
}

/// Makes an image through ScoutCode's /images/generations and saves it at `path`, which must be a new file: undo can
/// only put back text, so an image never replaces one. Like web_search, the agent runs it, since it needs the settings.
pub fn generate_image(cfg: &crate::config::Config, a: &Value, cwd: &str) -> R {
    use base64::Engine;
    let prompt = s(a, "prompt").trim();
    if prompt.is_empty() {
        return Err("prompt is required.".into());
    }
    let file = resolve(cwd, s(a, "path"))?;
    if file.exists() {
        return Err(format!("{} already exists. Save the image as a new file.", rel(cwd, &file)));
    }
    let ext = file.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    let format = match ext.as_str() {
        "png" => image::ImageFormat::Png,
        "jpg" | "jpeg" => image::ImageFormat::Jpeg,
        "webp" => image::ImageFormat::WebP,
        _ => return Err("path must end in .png, .jpg or .webp.".into()),
    };
    let mut body = json!({"prompt": prompt, "n": 1});
    if let Some(r) = a.get("aspect_ratio").and_then(Value::as_str).filter(|r| !r.is_empty()) {
        body["aspect_ratio"] = json!(r);
    }
    if let Some(m) = &cfg.image_model {
        body["model"] = json!(m);
    }
    let res = crate::api::generate_image(cfg, &body).map_err(|e| match e.status {
        404 if e.message.contains("image model") => format!("{e} Pick another in Settings > Experimental."),
        404 => format!("Image generation isn't available: the ScoutCode server at {} doesn't have it yet.", cfg.base_url),
        503 => "Image generation isn't set up on the ScoutCode server yet (it has no image models).".into(),
        _ => format!("Image generation failed: {e}"),
    })?;
    let b64 = res.pointer("/data/0/b64_json").and_then(Value::as_str).ok_or("The server sent no image.")?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|e| format!("The server sent an unreadable image: {e}"))?;
    let img = image::load_from_memory(&bytes).map_err(|e| format!("The server sent an unreadable image: {e}"))?;
    // Saved in the format the path asks for. JPEG has no transparency, so it goes on white.
    let mut out = std::io::Cursor::new(Vec::new());
    let saved = if format == image::ImageFormat::Jpeg { image::DynamicImage::ImageRgb8(img.to_rgb8()) } else { img.clone() };
    saved.write_to(&mut out, format).map_err(|e| format!("Could not encode the image: {e}"))?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&file, out.into_inner()).map_err(|e| e.to_string())?;
    let mut thumb = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img.thumbnail(512, 512).to_rgb8()).write_to(&mut thumb, image::ImageFormat::Jpeg).map_err(|e| e.to_string())?;
    let preview = format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(thumb.into_inner()));
    let path = rel(cwd, &file);
    let (w, h) = (img.width(), img.height());
    let said = res.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty()).map(|t| format!("\nThe image model said: {}", t.trim())).unwrap_or_default();
    Ok(ToolResult {
        output: format!("Saved a {w}x{h} image to {path}.{said}"),
        is_error: false,
        summary: Some(format!("{w}x{h}")),
        display: Some(Display::Image { path, preview }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_takes_paths_however_models_write_them() {
        let dir = std::env::temp_dir().join(format!("scout-resolve-{}", crate::session::now_ms()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src").join("App.ts"), "").unwrap();
        let cwd = dir.to_string_lossy().to_string();
        let file = Path::new(&cwd).join("src").join("App.ts");
        assert_eq!(resolve(&cwd, "src/App.ts").unwrap(), file);
        assert_eq!(resolve(&cwd, "./src/../src/App.ts").unwrap(), file);
        assert_eq!(resolve(&cwd, &file.to_string_lossy()).unwrap(), file);
        assert_eq!(resolve(&cwd, ".").unwrap(), PathBuf::from(&cwd));
        assert_eq!(resolve(&cwd, "src/New.ts").unwrap(), Path::new(&cwd).join("src").join("New.ts"));
        if cfg!(windows) {
            assert_eq!(resolve(&cwd, r"src\App.ts").unwrap(), file);
            // Windows doesn't care about case, so neither does the view-before-edit check.
            assert_eq!(resolve(&cwd, "SRC/app.TS").unwrap(), file);
            assert_eq!(resolve(&cwd, "/src/App.ts").unwrap(), file);
            let drive = &cwd[..1];
            let bash = format!("/{}{}", drive.to_lowercase(), cwd[2..].replace('\\', "/"));
            assert_eq!(resolve(&cwd, &format!("{bash}/src/App.ts")).unwrap(), file);
            assert_eq!(resolve(&cwd, &format!("/mnt{bash}/src/App.ts")).unwrap(), file);
            assert_eq!(resolve(&cwd, &cwd.replace('\\', "/")).unwrap(), PathBuf::from(&cwd));
        }
        let home = PathBuf::from(std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap());
        assert_eq!(canon(&resolve(&cwd, "~/scout-nothing.txt").unwrap()), canon(&home.join("scout-nothing.txt")));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn generate_image_never_replaces_a_file_and_needs_an_image_extension() {
        let dir = std::env::temp_dir().join(format!("scout-image-{}", crate::session::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("logo.png"), b"old").unwrap();
        let cwd = dir.to_string_lossy().to_string();
        // Settings that point nowhere: both calls must fail before any request.
        let mut cfg = crate::config::load_config();
        cfg.base_url = "http://127.0.0.1:9".into();
        let e = generate_image(&cfg, &json!({"prompt": "a fox", "path": "logo.png"}), &cwd).unwrap_err();
        assert!(e.contains("already exists"), "{e}");
        let e = generate_image(&cfg, &json!({"prompt": "a fox", "path": "fox.gif"}), &cwd).unwrap_err();
        assert!(e.contains(".png"), "{e}");
        assert_eq!(std::fs::read(dir.join("logo.png")).unwrap(), b"old");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn search_runs_grep_for_the_query_as_written() {
        let (name, a) = canonical("search", json!({"query": "fn main(", "directory": "src"}));
        assert_eq!(name, "grep");
        assert_eq!(a["pattern"], r"fn main\(");
        assert_eq!(a["ignore_case"], true);
        assert_eq!(a["output_mode"], "content");
        assert_eq!(a["path"], "src");
        // A real pattern is kept as a regex, and other tools pass through untouched.
        assert_eq!(canonical("search_files", json!({"pattern": "a.*b"})).1["pattern"], "a.*b");
        assert_eq!(canonical("view", json!({"path": "x"})), ("view".to_string(), json!({"path": "x"})));

        let dir = std::env::temp_dir().join(format!("scout-search-{}", crate::session::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "fn Main() {}\n").unwrap();
        let cwd = dir.to_string_lossy().to_string();
        let (cancel, mut read) = (AtomicBool::new(false), HashSet::new());
        let mut noop = |_: &str| {};
        let mut ctx = Ctx { cwd: &cwd, cancel: &cancel, read_files: &mut read, on_progress: &mut noop };
        let (name, a) = canonical("search", json!({"query": "fn main("}));
        let out = run(&name, &a, &mut ctx).unwrap().output;
        assert!(out.contains("a.rs") && out.contains("fn Main()"), "{out}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn view_shows_several_files_at_once() {
        let dir = std::env::temp_dir().join(format!("scout-view-many-{}", crate::session::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "alpha\n").unwrap();
        std::fs::write(dir.join("b.txt"), "beta\n").unwrap();
        let cwd = dir.to_string_lossy().to_string();
        let (cancel, mut read) = (AtomicBool::new(false), HashSet::new());
        let mut noop = |_: &str| {};
        let mut ctx = Ctx { cwd: &cwd, cancel: &cancel, read_files: &mut read, on_progress: &mut noop };
        let r = run("view", &json!({"paths": ["a.txt", "b.txt", "missing.txt"]}), &mut ctx).unwrap();
        assert!(r.output.contains("==> a.txt <==\n1\talpha") && r.output.contains("==> b.txt <==\n1\tbeta"), "{}", r.output);
        assert!(r.output.contains("==> missing.txt <==\nError: No file"), "{}", r.output);
        // Both count as read, so they can be edited next.
        assert_eq!(read.len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}
