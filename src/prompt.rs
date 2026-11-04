use crate::config::home;
use crate::tools::shell_name;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const INSTRUCTION_FILES: [&str; 3] = ["SCOUT.md", "AGENTS.md", "CLAUDE.md"];

pub fn git(cwd: &str, args: &[&str]) -> Option<String> {
    let mut c = Command::new("git");
    c.args(args).current_dir(cwd);
    crate::tools::no_window(&mut c);
    let out = c.output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Instruction files from ~/.scoutcode and from the git root (or filesystem root) down to cwd.
pub fn load_instructions(cwd: &str) -> Vec<(PathBuf, String)> {
    let mut out: Vec<(PathBuf, String)> = vec![];
    let mut read = |f: PathBuf| -> bool {
        match std::fs::read_to_string(&f) {
            Ok(t) => {
                let t = t.trim();
                if !t.is_empty() {
                    out.push((f, t.chars().take(40_000).collect()));
                }
                true
            }
            Err(_) => false,
        }
    };
    read(home().join("SCOUT.md"));
    let root = git(cwd, &["rev-parse", "--show-toplevel"]).map(PathBuf::from);
    let mut dirs = vec![];
    let mut d = Some(Path::new(cwd));
    while let Some(p) = d {
        dirs.insert(0, p.to_path_buf());
        if root.as_deref().is_some_and(|r| crate::session::same_dir(&r.to_string_lossy(), &p.to_string_lossy())) {
            break;
        }
        d = p.parent();
    }
    for dir in dirs {
        for name in INSTRUCTION_FILES {
            read(dir.join(name));
        }
        read(dir.join("SCOUT.local.md"));
    }
    // CLAUDE.md is often a copy of (or link to) AGENTS.md: keep each text once.
    let mut seen = std::collections::HashSet::new();
    out.retain(|(_, t)| seen.insert(t.clone()));
    out
}

/// The working directory's top level, like `ls .`: folders first with a trailing /, then files. Secrets and git
/// internals are left out.
pub fn listing(cwd: &str) -> String {
    let mut entries: Vec<(bool, String)> = std::fs::read_dir(cwd)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| (e.file_type().is_ok_and(|t| t.is_dir()), e.file_name().to_string_lossy().to_string()))
        .filter(|(_, n)| n != ".git" && !crate::tools::is_secret(n))
        .collect();
    entries.sort_by(|x, y| y.0.cmp(&x.0).then_with(|| x.1.to_lowercase().cmp(&y.1.to_lowercase())));
    let mut list: Vec<String> = entries.iter().take(200).map(|(d, n)| if *d { format!("{n}/") } else { n.clone() }).collect();
    if entries.len() > 200 {
        list.push(format!("… {} more", entries.len() - 200));
    }
    if list.is_empty() { "(empty folder)".into() } else { list.join("\n") }
}

/// The user's operating system by name, e.g. "Windows 11 Home 25H2 (build 26200, aarch64)" or "macOS 15.2 (aarch64)".
pub fn os_name() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        let arch = std::env::consts::ARCH;
        let run = |cmd: &str, args: &[&str]| {
            let mut c = Command::new(cmd);
            c.args(args);
            crate::tools::no_window(&mut c);
            c.output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        };
        match std::env::consts::OS {
            "windows" => {
                let out = run("reg", &["query", r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion"]).unwrap_or_default();
                let value = |name: &str| {
                    out.lines()
                        .map(str::trim)
                        .find(|l| l.split_whitespace().next() == Some(name))
                        .and_then(|l| l.split_once("REG_SZ").map(|(_, v)| v.trim().to_string()))
                        .unwrap_or_default()
                };
                let build: u32 = value("CurrentBuild").parse().unwrap_or(0);
                let mut product = value("ProductName");
                if product.is_empty() {
                    product = "Windows".into();
                }
                // Windows 11 still calls itself Windows 10 in the registry; build 22000 and up is 11.
                if build >= 22000 {
                    product = product.replacen("Windows 10", "Windows 11", 1);
                }
                let release = value("DisplayVersion");
                let release = if release.is_empty() { String::new() } else { format!(" {release}") };
                format!("{product}{release} (build {build}, {arch})")
            }
            "macos" => format!("macOS {} ({arch})", run("sw_vers", &["-productVersion"]).map(|v| v.trim().to_string()).unwrap_or_default()),
            "linux" => {
                let pretty = std::fs::read_to_string("/etc/os-release")
                    .ok()
                    .and_then(|t| t.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string())));
                format!("{} ({arch})", pretty.unwrap_or_else(|| "Linux".into()))
            }
            os => format!("{os} ({arch})"),
        }
    })
}

/// Models mix up the two path styles on Windows: Git Bash prints /c/Users/..., and they pass that to the file tools.
fn paths_note() -> &'static str {
    match (cfg!(windows), shell_name()) {
        (true, "bash") => "give the file tools paths relative to the working directory (e.g. src/main.rs). The bash tool is Git Bash: it takes forward slashes and shows C:\\Users as /c/Users.",
        (true, _) => "give the file tools paths relative to the working directory (e.g. src\\main.rs). The bash tool is PowerShell and takes Windows paths.",
        _ => "give the file tools paths relative to the working directory (e.g. src/main.rs).",
    }
}

pub fn environment(cwd: &str) -> String {
    let branch = git(cwd, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let (y, m, d, ..) = crate::session::civil(crate::session::now_ms() / 1000);
    [
        format!("Working directory: {cwd}"),
        format!("Operating system: {}", os_name()),
        format!("Shell for the bash tool: {}", shell_name()),
        format!("Paths: {}", paths_note()),
        format!("Date: {y:04}-{m:02}-{d:02}"),
        format!("Git repository: {}", branch.map(|b| format!("yes, branch {b}")).unwrap_or_else(|| "no".into())),
    ]
    .join("\n")
}

fn mode_text(mode: &str) -> &'static str {
    match mode {
        "acceptEdits" => "File edits are applied without asking; the user approves each shell command.",
        "auto" => "File edits and shell commands run without asking, but a safety check (rules plus Jev) holds dangerous commands and edits outside the project for the user to review. If one is denied, do not retry it or a workaround; ask or adjust.",
        "yolo" => "Every tool runs without asking. Be careful with destructive commands: never delete data, force-push or touch files outside the project unless asked.",
        "plan" => "PLAN MODE: you can only read and search. Investigate, then call present_plan with a concrete step-by-step plan. Do not try to edit files or run commands until the user approves the plan.",
        _ => "The user approves each file edit and shell command. If they deny one, do not retry it unchanged; ask or adjust.",
    }
}

/// Sent at the end of the newest message when the mode changes, so the system prompt never does.
pub fn mode_note(mode: &str) -> String {
    format!("\n\n[Permission mode: {mode}. {}]", mode_text(mode))
}

/// The user's text without a trailing mode note, for display.
pub fn strip_mode_note(s: &str) -> &str {
    match s.rfind("\n\n[Permission mode: ") {
        Some(i) if s.ends_with(']') => &s[..i],
        _ => s,
    }
}

const PLAN_NOTE: &str = "\n\n[Command /plan: before changing anything, look at what's involved and write a short, numbered plan for the request above. Then carry it out.]";

/// What the model is sent for a message: a `/plan` gets its instruction added after the user's words.
pub fn command_prompt(text: &str) -> String {
    match text.strip_prefix("/plan") {
        Some(rest) if rest.starts_with(char::is_whitespace) => format!("{text}{PLAN_NOTE}"),
        _ => text.to_string(),
    }
}

/// The user's text without a command's note, for display.
pub fn strip_command_note(s: &str) -> &str {
    s.strip_suffix(PLAN_NOTE).unwrap_or(s)
}

/// Rebuilt at turn boundaries to pick up saved instructions, then reused throughout the turn. Unchanged content
/// remains cacheable. `model` is the chosen name; a router's says nothing about what's behind it.
pub fn system_prompt(cwd: &str, edit: crate::tools::EditTool, model: &str, preferences: &crate::instructions::Preferences) -> String {
    let edit_hint = match edit {
        crate::tools::EditTool::ApplyPatch => "Change files with apply_patch rather than rewriting them with write_file.",
        crate::tools::EditTool::StrReplace => "Prefer edit_file / multi_edit over rewriting whole files.",
    };
    let instructions = load_instructions(cwd);
    let project = if instructions.is_empty() {
        String::new()
    } else {
        let files: Vec<String> = instructions.iter().map(|(f, t)| format!("## {}\n{t}", f.display())).collect();
        format!("\n# Project instructions\nThese files were written by the user; follow them.\n\n{}", files.join("\n\n"))
    };
    let preferences = preferences.prompt(cwd);
    let memory = crate::memory::prompt(cwd);
    format!(
        "You are Scout, a coding agent running in the Scout desktop app through ScoutCode. You share a workspace with the user and help with software engineering: fixing bugs, adding features, refactoring, explaining code and running commands. Carry the user's intended task through to completion within the current permission mode.
You are running as the model \"{model}\". If asked which model you are, give that name.

# Completing the task
- Treat requests such as \"can you fix\" or \"help me add\" as instructions to do the work. For an implementation request, continue through investigation, changes and verification; do not stop at a plan, a partial fix or an offer to continue. In plan mode, investigate and use present_plan instead.
- Use the user's intent, project instructions and surrounding code to resolve routine implementation choices. State reasonable assumptions and proceed. Ask a concise question only when missing information materially affects correctness or the requested scope and cannot be found with the tools. Continue independent work while clarification is needed.
- If an approach fails, inspect the error and change the approach. Do not repeat the same unsuccessful tool call without new evidence. When blocked, finish any independent work, then explain the concrete blocker and what is needed to proceed.
- Before ending, check the result against the whole request. Complete required follow-up changes and checks, and report any remaining gaps honestly.

# Working efficiently
- Give each search or read a concrete purpose. Use glob to locate files and grep to find relevant symbols or text, then view the relevant ranges. Scope searches to likely directories and file types; narrow oversized results instead of dumping the whole repository.
- Read enough surrounding code to understand the change, then implement it. Do not keep exploring once you have the information needed for the next step. Reuse findings already in context instead of listing directories or reading unchanged files again, unless the earlier output was incomplete or cleared.
- Call independent read-only tools in one response when useful. Keep dependent operations and edits in order; inspect a result before issuing a call that depends on it.
- Use task only for a broad, self-contained read-only investigation whose report will save work or context. Give the sub-agent the relevant context and a specific question; it cannot see this conversation. Handle small searches directly and avoid duplicating delegated work.
- Use todo_write for work with several substantial steps, keep it current, and skip it for simple tasks. Planning should support progress, not replace implementation.
- Use bash for commands and the file tools for reading, searching and editing. Use the shell named in the environment, with its quoting and path syntax. Give bash a run_for_seconds that fits the command (default 30). A command still running then keeps going in the background and you get its last 50 lines: decide whether to keep waiting (shell_output wait_seconds), read more (shell_output lines), stop and change it (kill_shell), or work on something else meanwhile. Start servers, watchers and long builds with run_in_background, then use shell_output to check progress and completion. Avoid repeatedly polling when no new output is expected.

# Changes and verification
- Read existing files before editing them. Preserve the user's uncommitted work; never discard or overwrite unrelated changes.
- Make the smallest change that fully solves the task, matching the surrounding code's style, naming and comment density. Do not add unrelated refactors, files or comments.
- {edit_hint}
- Run the checks required by project instructions and those appropriate to the change, using the project's existing build, type checker, linter or tests. Fix failures caused by your changes and rerun the affected checks. Once checks pass, repeat or broaden them only when new changes or unresolved concerns justify it.
- Check tool results and exit codes before claiming success. Do not claim a build, test, installation or other action succeeded unless you observed it. If a check cannot run, say why and what remains unverified.

# Conversation continuity
- Preserve the original objective, user constraints, decisions and unfinished work across turns. Treat a new message during work as steering the current task unless the user clearly cancels or replaces it. Answer status questions briefly, then continue the work.
- After compaction, continue from the summary and retained messages. Reuse completed findings and checks; do not restart the investigation or repeat completed work. If important details were lost or files changed, retrieve only what is needed to proceed.

# Tone
- Be concise and direct. Answers are rendered as Markdown in a desktop window.
- Give a short update before substantial work and when a meaningful finding or blocker changes the approach. Do not narrate each tool call.
- When you mention a file, write it as a Markdown link to its path relative to the working directory, with :line when one place matters, like [config.rs:42](src/config.rs:42) or [README.md](README.md). The user clicks it to open the file in Scout. Use forward slashes in the link, and keep links out of code blocks. When done, lead with the outcome, then summarize what changed, what was verified and any remaining limitation.

# Permissions
- The permission mode is given in a [Permission mode: …] note at the end of a message whenever it changes; the latest note applies. Follow it even when the user has authorized the overall task. Tools that the mode does not allow are refused when called.
- Remember the user's authorization and preferences across turns. Within the current mode, proceed with already authorized work without asking for conversational confirmation again; tool approval requirements still apply. Do the allowed preparation before requesting any necessary approval, so the user can review a concrete result.
- Never bypass a denial or safety restriction. Explain the specific reason if progress requires the user to approve an action or supply information.
- Never commit, push, publish, deploy or send messages to others unless the user asks. Never expose secrets or write them into files.

# Environment
{}

# Working directory
The current top level of {cwd} (no need to list it again):
{}
{project}
{preferences}
{memory}",
        environment(cwd),
        listing(cwd)
    )
}

pub fn subagent_prompt(cwd: &str, preferences: &crate::instructions::Preferences) -> String {
    let preferences = preferences.prompt(cwd);
    format!(
        "You are a research sub-agent for Scout, a coding agent. You have read-only tools. Complete the task you are given, then reply with one final report: concrete findings with file paths and line numbers, no filler. You cannot ask questions; do your best with what you have.
Search likely files first and view only relevant ranges. Reuse findings rather than repeating searches or reading unchanged files. If a tool fails, inspect the error and adjust; do not repeat the same failed call without new evidence. Stop when you can answer the assigned question. Distinguish confirmed findings from assumptions and report any unresolved gap.

Working directory: {cwd}
Operating system: {}

{preferences}
{}",
        os_name(),
        crate::memory::block(cwd)
    )
}

pub const COMPACT_PROMPT: &str = "Summarize this conversation so another instance of you can continue the work without it. Include:
1. The user's requests and goals, in their own words where it matters.
2. Key decisions, constraints, preferences, authorizations and denials the user stated, including the established conversation language and explicit language requests. Tool permission requirements still apply.
3. Files read or changed, with the important details (paths, functions, what changed).
4. Commands run and their results (errors, test outcomes), failed approaches to avoid repeating, and IDs of any background shells still needed.
5. The current state: what is done, what is in progress, any blocker or pending question, and the concrete next steps. Preserve enough findings to continue without repeating completed investigation.
Write the summary in the user's established conversation language. Preserve original code, commands and paths. Be specific and complete but compact. Reply with the summary only.";

#[cfg(test)]
mod tests {
    #[test]
    fn os_name_is_named() {
        let name = super::os_name();
        println!("{name}");
        assert!(!name.is_empty() && !name.starts_with(' '));
    }

    #[test]
    fn plan_command_adds_its_note_and_shows_without_it() {
        let sent = super::command_prompt("/plan add a dark theme");
        assert!(sent.starts_with("/plan add a dark theme\n\n[Command /plan:"));
        assert_eq!(super::strip_command_note(&sent), "/plan add a dark theme");
        assert_eq!(super::command_prompt("/planet facts"), "/planet facts");
        assert_eq!(super::command_prompt("plan it"), "plan it");
    }

    #[test]
    fn prompt_has_every_instruction_file_and_the_listing() {
        let dir = std::env::temp_dir().join(format!("scout-prompt-{}", crate::session::now_ms()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("SCOUT.md"), "scout rules").unwrap();
        std::fs::write(dir.join("AGENTS.md"), "agent rules").unwrap();
        std::fs::write(dir.join("CLAUDE.md"), "agent rules").unwrap();
        std::fs::write(dir.join(".env"), "SECRET=1").unwrap();
        let p = super::system_prompt(&dir.to_string_lossy(), crate::tools::EditTool::StrReplace, "Test Model", &Default::default());
        assert!(p.contains("running as the model \"Test Model\""), "{p}");
        assert!(p.contains("scout rules") && p.contains("agent rules"), "{p}");
        // CLAUDE.md is the same text as AGENTS.md, so it's in once.
        assert_eq!(p.matches("agent rules").count(), 1);
        assert!(p.contains("src/\nAGENTS.md\nCLAUDE.md\nSCOUT.md") && !p.contains(".env"), "{p}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
