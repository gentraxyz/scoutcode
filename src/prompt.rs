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

pub fn environment(cwd: &str) -> String {
    let branch = git(cwd, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let (y, m, d, ..) = crate::session::civil(crate::session::now_ms() / 1000);
    [
        format!("Working directory: {cwd}"),
        format!("Operating system: {}", os_name()),
        format!("Shell for the bash tool: {}", shell_name()),
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

/// Kept the same for a whole session so providers can reuse the cached prompt prefix.
pub fn system_prompt(cwd: &str, edit: crate::tools::EditTool) -> String {
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
    format!(
        "You are Scout, a coding agent running in the Scout desktop app through ScoutCode. You help with software engineering: fixing bugs, adding features, refactoring, explaining code and running commands.

# How to work
- Understand before changing: search with grep/glob and read the relevant files first. Never edit a file you have not read.
- Make the smallest change that fully solves the task, matching the surrounding code's style, naming and comment density. Do not add unrelated refactors, files or comments.
- {edit_hint}
- After changing code, verify it when you can: run the build, type checker, linter or tests that the project already uses.
- For tasks with several steps, keep a todo list with todo_write and update it as you go.
- Use task to delegate broad read-only searches to a sub-agent.
- Call several independent read-only tools in one response when you can.
- Never commit, push, publish or deploy unless the user asks. Never expose secrets or write them into files.
- If the request is ambiguous in a way that matters, ask one short question instead of guessing.

# Tone
- Be concise and direct. Answers are rendered as Markdown in a desktop window.
- Refer to code as path:line. Do not narrate each tool call; say what you found and what you changed.
- When you are done, give a short summary of what changed and anything left unverified.

# Permissions
The permission mode is given in a [Permission mode: …] note at the end of a message whenever it changes; the latest note applies. Tools that the mode does not allow are refused when called.

# Environment
{}

# Working directory
The top level of {cwd} when this conversation started (no need to list it again):
{}
{project}",
        environment(cwd),
        listing(cwd)
    )
}

pub fn subagent_prompt(cwd: &str) -> String {
    format!(
        "You are a research sub-agent for Scout, a coding agent. You have read-only tools. Complete the task you are given, then reply with one final report: concrete findings with file paths and line numbers, no filler. You cannot ask questions; do your best with what you have.\n\nWorking directory: {cwd}\nOperating system: {}",
        os_name()
    )
}

pub const COMPACT_PROMPT: &str = "Summarize this conversation so another instance of you can continue the work without it. Include:
1. The user's requests and goals, in their own words where it matters.
2. Key decisions, constraints and preferences the user stated.
3. Files read or changed, with the important details (paths, functions, what changed).
4. Commands run and their results (errors, test outcomes).
5. The current state: what is done, what is in progress, and the next steps.
Be specific and complete but compact. Reply with the summary only.";

#[cfg(test)]
mod tests {
    #[test]
    fn os_name_is_named() {
        let name = super::os_name();
        println!("{name}");
        assert!(!name.is_empty() && !name.starts_with(' '));
    }

    #[test]
    fn prompt_has_every_instruction_file_and_the_listing() {
        let dir = std::env::temp_dir().join(format!("scout-prompt-{}", crate::session::now_ms()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("SCOUT.md"), "scout rules").unwrap();
        std::fs::write(dir.join("AGENTS.md"), "agent rules").unwrap();
        std::fs::write(dir.join("CLAUDE.md"), "agent rules").unwrap();
        std::fs::write(dir.join(".env"), "SECRET=1").unwrap();
        let p = super::system_prompt(&dir.to_string_lossy(), crate::tools::EditTool::StrReplace);
        assert!(p.contains("scout rules") && p.contains("agent rules"), "{p}");
        // CLAUDE.md is the same text as AGENTS.md, so it's in once.
        assert_eq!(p.matches("agent rules").count(), 1);
        assert!(p.contains("src/\nAGENTS.md\nCLAUDE.md\nSCOUT.md") && !p.contains(".env"), "{p}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
