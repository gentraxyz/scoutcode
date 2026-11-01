//! OpenAI's apply_patch format, which GPT-4.1, GPT-5 and Codex are trained on:
//!
//! ```text
//! *** Begin Patch
//! *** Update File: src/app.ts
//! @@ function main() {
//!  unchanged line
//! -old line
//! +new line
//! *** Add File: notes.md
//! +content
//! *** Delete File: old.ts
//! *** End Patch
//! ```
//!
//! A hunk's old lines (context and -) are looked for after the previous hunk, first exactly, then ignoring trailing
//! and then all surrounding whitespace, the way OpenAI's own tool does.

use crate::diff;
use crate::tools::{Ctx, Display, ToolResult, read_text, rel, resolve, write_text};
use serde_json::Value;
use std::path::PathBuf;

pub const DESCRIPTION: &str = "Edit files with a patch. Use it to change, create, delete and move files. Read a file before changing it.
The input is the whole patch:
*** Begin Patch
*** Update File: path/to/file.ts
@@ an optional line above the change, such as the enclosing function
 unchanged context line
-line to remove
+line to add
 unchanged context line
*** Add File: path/to/new.ts
+every line of the new file, each starting with +
*** Delete File: path/to/old.ts
*** End Patch
Give about 3 unchanged lines around each change so the place is unambiguous; start another @@ hunk for another place in the same file. Add \"*** Move to: new/path\" right after an Update File line to rename the file.";

#[derive(Debug, Default, PartialEq)]
pub struct Hunk {
    header: String,
    old: Vec<String>,
    new: Vec<String>,
    eof: bool,
}

#[derive(Debug, PartialEq)]
pub enum Op {
    Add { path: String, content: String },
    Delete { path: String },
    Update { path: String, move_to: Option<String>, hunks: Vec<Hunk> },
}

impl Op {
    fn path(&self) -> &str {
        match self {
            Op::Add { path, .. } | Op::Delete { path } | Op::Update { path, .. } => path,
        }
    }
}

/// "*** Add File: x" → ("Add", "x").
fn file_line(l: &str) -> Option<(&str, &str)> {
    let rest = l.trim().strip_prefix("*** ")?;
    for verb in ["Add", "Delete", "Update"] {
        if let Some(p) = rest.strip_prefix(verb).and_then(|r| r.strip_prefix(" File: ")) {
            return Some((verb, p.trim()));
        }
    }
    None
}

pub fn parse(input: &str) -> Result<Vec<Op>, String> {
    let text = input.replace("\r\n", "\n");
    let mut lines: Vec<&str> = text.split('\n').collect();
    // Some models wrap it as a shell heredoc or in a code fence; only what's between the markers counts.
    let begin = lines.iter().position(|l| l.trim() == "*** Begin Patch");
    if let Some(b) = begin {
        lines.drain(..=b);
    }
    match lines.iter().position(|l| l.trim() == "*** End Patch") {
        Some(e) => lines.truncate(e),
        None if begin.is_none() && lines.last() == Some(&"") => {
            lines.pop();
        }
        None => {}
    }
    let mut ops = vec![];
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let Some((verb, path)) = file_line(line) else {
            if line.trim().is_empty() {
                i += 1;
                continue;
            }
            return Err(format!("Invalid patch line {}: \"{line}\". Each file starts with \"*** Add File: path\", \"*** Update File: path\" or \"*** Delete File: path\".", i + 1));
        };
        let path = path.to_string();
        i += 1;
        match verb {
            "Delete" => ops.push(Op::Delete { path }),
            "Add" => {
                let mut body = vec![];
                while i < lines.len() && file_line(lines[i]).is_none() {
                    let l = lines[i];
                    i += 1;
                    if let Some(t) = l.strip_prefix('+') {
                        body.push(t);
                    } else if !l.trim().is_empty() {
                        return Err(format!("In \"*** Add File: {path}\", every line must start with \"+\" (got \"{l}\")."));
                    }
                }
                let content = if body.is_empty() { String::new() } else { body.join("\n") + "\n" };
                ops.push(Op::Add { path, content });
            }
            _ => {
                let mut move_to = None;
                if let Some(m) = lines.get(i).and_then(|l| l.strip_prefix("*** Move to: ")) {
                    move_to = Some(m.trim().to_string());
                    i += 1;
                }
                let mut hunks: Vec<Hunk> = vec![];
                while i < lines.len() && file_line(lines[i]).is_none() {
                    let l = lines[i];
                    i += 1;
                    if let Some(h) = l.strip_prefix("@@") {
                        let h = h.trim();
                        hunks.push(Hunk { header: h.strip_suffix("@@").unwrap_or(h).trim().to_string(), ..Default::default() });
                        continue;
                    }
                    if l.trim() == "*** End of File" {
                        if let Some(h) = hunks.last_mut() {
                            h.eof = true;
                        }
                        continue;
                    }
                    if hunks.is_empty() {
                        hunks.push(Hunk::default());
                    }
                    let h = hunks.last_mut().unwrap();
                    if let Some(t) = l.strip_prefix('+') {
                        h.new.push(t.to_string());
                    } else if let Some(t) = l.strip_prefix('-') {
                        h.old.push(t.to_string());
                    } else {
                        // Context; models often drop the leading space on blank lines.
                        let t = l.strip_prefix(' ').unwrap_or(l).to_string();
                        h.old.push(t.clone());
                        h.new.push(t);
                    }
                }
                if !hunks.iter().any(|h| !h.old.is_empty() || !h.new.is_empty()) && move_to.is_none() {
                    return Err(format!("\"*** Update File: {path}\" has no changes."));
                }
                ops.push(Op::Update { path, move_to, hunks });
            }
        }
    }
    if ops.is_empty() {
        return Err("The patch changes no files. Start it with \"*** Begin Patch\" and list files with \"*** Update File: path\".".into());
    }
    Ok(ops)
}

/// Where `want` sits in `lines` at or after `from`: exact first, then ignoring trailing whitespace, then all of it.
fn seek(lines: &[String], want: &[String], from: usize, eof: bool) -> Option<usize> {
    if want.is_empty() {
        return Some(if eof { lines.len() } else { from });
    }
    let norms: [fn(&str) -> &str; 3] = [|s| s, str::trim_end, str::trim];
    for norm in norms {
        let at = |i: usize| want.iter().enumerate().all(|(k, w)| norm(&lines[i + k]) == norm(w));
        if lines.len() < want.len() {
            return None;
        }
        let last = lines.len() - want.len();
        // A hunk marked *** End of File belongs at the end, so try there first.
        if eof && last >= from && at(last) {
            return Some(last);
        }
        if let Some(i) = (from..=last).find(|&i| at(i)) {
            return Some(i);
        }
    }
    None
}

pub fn apply_hunks(text: &str, hunks: &[Hunk], file: &str) -> Result<String, String> {
    let trailing = text.is_empty() || text.ends_with('\n');
    let body = if trailing && !text.is_empty() { &text[..text.len() - 1] } else { text };
    let mut lines: Vec<String> = if text.is_empty() { vec![] } else { body.split('\n').map(String::from).collect() };
    let mut cursor = 0;
    for h in hunks {
        if !h.header.is_empty() {
            if let Some(at) = seek(&lines, std::slice::from_ref(&h.header), cursor, false) {
                cursor = at + 1;
            }
        }
        // Hunks out of order: look from the top before giving up.
        let at = seek(&lines, &h.old, cursor, h.eof).or_else(|| if cursor > 0 { seek(&lines, &h.old, 0, h.eof) } else { None });
        let Some(at) = at else {
            let after = if h.header.is_empty() { String::new() } else { format!(" (after \"{}\")", h.header) };
            return Err(format!("These lines were not found in {file}{after}:\n{}\nRead the file again and copy the context and - lines exactly.", h.old.join("\n")));
        };
        lines.splice(at..at + h.old.len(), h.new.iter().cloned());
        cursor = at + h.new.len();
    }
    Ok(if lines.is_empty() { String::new() } else { lines.join("\n") + if trailing { "\n" } else { "" } })
}

struct Change {
    abs: PathBuf,
    before: Option<String>,
    /// None deletes the file.
    after: Option<String>,
    crlf: bool,
    /// A move's old path, removed once the new one is written.
    removes: Option<PathBuf>,
}

fn input(a: &Value) -> &str {
    a.get("input").or_else(|| a.get("patch")).and_then(Value::as_str).unwrap_or("")
}

/// Works out every file's new content before anything is written, so a bad hunk changes nothing.
fn plan(a: &Value, ctx: &Ctx, check_read: bool) -> Result<Vec<Change>, String> {
    let input = input(a);
    if input.trim().is_empty() {
        return Err("apply_patch needs the patch text in \"input\".".into());
    }
    let must_have_read = |abs: &PathBuf, p: &str| {
        if check_read && !ctx.read_files.contains(abs) { Err(format!("Open {p} with the view tool before changing it.")) } else { Ok(()) }
    };
    let mut changes = vec![];
    for op in parse(input)? {
        let abs = resolve(ctx.cwd, op.path())?;
        let had = abs.exists();
        match op {
            Op::Add { path, content } => {
                if had {
                    must_have_read(&abs, &path)?;
                }
                let before = if had { read_text(&abs) } else { None };
                let crlf = before.as_ref().is_some_and(|b| b.1);
                changes.push(Change { abs, before: before.map(|b| b.0), after: Some(content), crlf, removes: None });
            }
            Op::Delete { path } => {
                if !had {
                    return Err(format!("Cannot delete {path}: it does not exist."));
                }
                must_have_read(&abs, &path)?;
                changes.push(Change { before: read_text(&abs).map(|b| b.0), abs, after: None, crlf: false, removes: None });
            }
            Op::Update { path, move_to, hunks } => {
                if !had {
                    return Err(format!("Cannot update {path}: it does not exist. Use \"*** Add File: {path}\" to create it."));
                }
                must_have_read(&abs, &path)?;
                let (text, crlf) = read_text(&abs).ok_or_else(|| format!("Could not read {path} as text."))?;
                let after = apply_hunks(&text, &hunks, &path)?;
                match move_to {
                    Some(m) => changes.push(Change { abs: resolve(ctx.cwd, &m)?, before: None, after: Some(after), crlf, removes: Some(abs) }),
                    None => changes.push(Change { abs, before: Some(text), after: Some(after), crlf, removes: None }),
                }
            }
        }
    }
    Ok(changes)
}

fn displays(ctx: &Ctx, changes: &[Change]) -> Vec<Display> {
    changes
        .iter()
        .map(|c| {
            let note = match (&c.removes, &c.after) {
                (Some(from), _) => format!(" (moved from {})", rel(ctx.cwd, from)),
                (None, None) => " (deleted)".into(),
                _ => String::new(),
            };
            Display::Diff {
                path: rel(ctx.cwd, &c.abs) + &note,
                lines: diff::diff_lines(c.before.as_deref().unwrap_or(""), c.after.as_deref().unwrap_or("")),
                created: c.before.is_none() && c.removes.is_none(),
            }
        })
        .collect()
}

fn one_or_many(mut d: Vec<Display>) -> Display {
    if d.len() == 1 { d.pop().unwrap() } else { Display::Diffs(d) }
}

/// The files a patch touches, for undo's snapshots.
pub fn paths(a: &Value) -> Vec<String> {
    parse(input(a))
        .map(|ops| {
            ops.into_iter()
                .flat_map(|op| match op {
                    Op::Update { path, move_to: Some(m), .. } => vec![path, m],
                    op => vec![op.path().to_string()],
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn label(a: &Value) -> String {
    match parse(input(a)) {
        Ok(ops) if ops.len() == 1 => ops[0].path().to_string(),
        Ok(ops) => format!("{} and {} more", ops[0].path(), ops.len() - 1),
        Err(_) => "patch".into(),
    }
}

pub fn preview(a: &Value, ctx: &Ctx) -> Display {
    match plan(a, ctx, false) {
        Ok(c) => one_or_many(displays(ctx, &c)),
        Err(e) => Display::Text(format!("This patch will fail: {e}")),
    }
}

pub fn run(a: &Value, ctx: &mut Ctx) -> Result<ToolResult, String> {
    let changes = plan(a, ctx, true)?;
    for c in &changes {
        match &c.after {
            Some(t) => write_text(&c.abs, t, c.crlf)?,
            None => std::fs::remove_file(&c.abs).map_err(|e| e.to_string())?,
        }
        if let Some(old) = &c.removes {
            let _ = std::fs::remove_file(old);
        }
        ctx.read_files.insert(c.abs.clone());
    }
    let d = displays(ctx, &changes);
    let (mut added, mut removed) = (0, 0);
    let parts: Vec<String> = changes
        .iter()
        .zip(&d)
        .map(|(c, d)| {
            let Display::Diff { lines, .. } = d else { unreachable!() };
            let (a, r) = diff::stats(lines);
            added += a;
            removed += r;
            let p = rel(ctx.cwd, &c.abs);
            match (&c.after, &c.removes, &c.before) {
                (None, ..) => format!("deleted {p}"),
                (_, Some(from), _) => format!("moved {} to {p} (+{a} -{r})", rel(ctx.cwd, from)),
                (_, _, None) => format!("created {p} ({a} lines)"),
                _ => format!("updated {p} (+{a} -{r})"),
            }
        })
        .collect();
    let summary = if changes.len() == 1 { format!("+{added} -{removed}") } else { format!("{} files, +{added} -{removed}", changes.len()) };
    Ok(ToolResult { output: format!("Patch applied: {}.", parts.join(", ")), is_error: false, display: Some(one_or_many(d)), summary: Some(summary) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::atomic::AtomicBool;

    fn hunks(p: &str) -> Vec<Hunk> {
        match parse(p).unwrap().remove(0) {
            Op::Update { hunks, .. } => hunks,
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn each_model_gets_its_edit_tools() {
        let names = |e| crate::tools::specs(None, e).iter().map(|s| s["function"]["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        let (patch, replace) = (names(crate::tools::EditTool::ApplyPatch), names(crate::tools::EditTool::StrReplace));
        assert_eq!(&patch[..3], ["view", "write_file", "apply_patch"]);
        assert!(!patch.iter().any(|n| n == "edit_file" || n == "multi_edit"));
        assert_eq!(&replace[..4], ["view", "write_file", "edit_file", "multi_edit"]);
        assert!(!replace.iter().any(|n| n == "apply_patch"));
        assert_eq!(crate::tools::EditTool::parse("apply_patch"), crate::tools::EditTool::ApplyPatch);
        assert_eq!(crate::tools::EditTool::parse("whatever"), crate::tools::EditTool::StrReplace);
    }

    #[test]
    fn header_picks_the_right_spot() {
        let h = hunks("*** Begin Patch\n*** Update File: f\n@@ fn b() {\n-    1\n+    2\n }\n*** End Patch");
        assert_eq!(apply_hunks("fn a() {\n    1\n}\nfn b() {\n    1\n}\n", &h, "f").unwrap(), "fn a() {\n    1\n}\nfn b() {\n    2\n}\n");
    }

    #[test]
    fn tolerant_matching_eof_and_wrappers() {
        let h = hunks("*** Begin Patch\n*** Update File: f\n a\n\n-b\n+c\n*** End Patch");
        assert_eq!(apply_hunks("a   \n\nb\n", &h, "f").unwrap(), "a\n\nc\n");
        let h = hunks("apply_patch <<'EOF'\n*** Begin Patch\n*** Update File: f\n@@\n x\n+end\n*** End of File\n*** End Patch\nEOF");
        assert_eq!(apply_hunks("x\nmid\nx\n", &h, "f").unwrap(), "x\nmid\nx\nend\n");
        let h = hunks("*** Begin Patch\n*** Update File: f\n@@\n-1\n+one\n@@\n-3\n+three\n*** End Patch");
        assert_eq!(apply_hunks("1\n2\n3", &h, "f").unwrap(), "one\n2\nthree");
        assert!(apply_hunks("1\n", &hunks("*** Update File: f\n-9\n+8"), "f").unwrap_err().contains("not found"));
        assert!(parse("hello").is_err() && parse("*** Begin Patch\n*** End Patch").is_err());
    }

    #[test]
    fn applies_all_or_nothing() {
        let dir = std::env::temp_dir().join(format!("scout-patch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "one\r\ntwo\r\n").unwrap();
        std::fs::write(dir.join("old.txt"), "bye\n").unwrap();
        let cwd = dir.to_string_lossy().to_string();
        let cancel = AtomicBool::new(false);
        let mut read: HashSet<PathBuf> = HashSet::new();
        let mut progress = |_: &str| {};
        let patch = serde_json::json!({"input": "*** Begin Patch\n*** Update File: a.txt\n one\n-two\n+2\n*** Add File: new/b.md\n+hi\n*** Delete File: old.txt\n*** End Patch"});
        {
            let mut ctx = Ctx { cwd: &cwd, cancel: &cancel, read_files: &mut read, on_progress: &mut progress };
            assert!(run(&patch, &mut ctx).unwrap_err().contains("view tool"), "files must be read first");
        }
        read.insert(dir.join("a.txt"));
        read.insert(dir.join("old.txt"));
        let mut ctx = Ctx { cwd: &cwd, cancel: &cancel, read_files: &mut read, on_progress: &mut progress };
        // One bad hunk and nothing is written.
        let bad = serde_json::json!({"input": "*** Begin Patch\n*** Delete File: old.txt\n*** Update File: a.txt\n-nope\n+x\n*** End Patch"});
        assert!(run(&bad, &mut ctx).is_err());
        assert!(dir.join("old.txt").exists());
        let r = run(&patch, &mut ctx).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "one\r\n2\r\n", "keeps CRLF");
        assert_eq!(std::fs::read_to_string(dir.join("new").join("b.md")).unwrap(), "hi\n");
        assert!(!dir.join("old.txt").exists());
        assert!(matches!(r.display, Some(Display::Diffs(ref d)) if d.len() == 3), "{:?}", r.display);
        assert_eq!(paths(&patch), ["a.txt", "new/b.md", "old.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
