//! Files attached to a message. They go in after the text as `<file path="…">` blocks, the same as the old CLI's, so
//! its chats read back the same way. Skills named with @ go in the same way, as `<skill name="…">` blocks; those aren't
//! attachments, since the @name stays in the text and brings the skill back if the message is edited and sent again.

use crate::tools;
use std::path::PathBuf;

/// Files this size or larger go in by name only; the model can read them with the view tool.
const MAX_BYTES: u64 = 300_000;
/// Entries listed for an attached folder.
const MAX_ENTRIES: usize = 200;

/// The message to send, and the files whose contents it carries (so the agent may edit them without a view first).
pub fn build(cwd: &str, text: &str, files: &[String]) -> (String, Vec<PathBuf>) {
    let mut blocks = crate::skills::blocks(text);
    if files.is_empty() && blocks.is_empty() {
        return (text.to_string(), vec![]);
    }
    let mut read = vec![];
    for f in files {
        let Ok(abs) = tools::resolve(cwd, f) else { continue };
        if abs.is_dir() {
            let mut names: Vec<String> = std::fs::read_dir(&abs)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    if e.file_type().is_ok_and(|t| t.is_dir()) { format!("{n}/") } else { n }
                })
                .collect();
            names.sort_by_key(|n| n.to_lowercase());
            let more = names.len().saturating_sub(MAX_ENTRIES);
            names.truncate(MAX_ENTRIES);
            if more > 0 {
                names.push(format!("… {more} more"));
            }
            blocks.push(format!("<folder path=\"{f}\">\n{}\n</folder>", names.join("\n")));
            continue;
        }
        let size = std::fs::metadata(&abs).map(|m| m.len()).unwrap_or(0);
        let body = if size >= MAX_BYTES {
            format!("(Not included: {} KB is too large to attach. Read the parts you need with the view tool.)", size / 1000)
        } else {
            match std::fs::read(&abs) {
                Err(e) => format!("(Could not read it: {e})"),
                Ok(b) if b.iter().take(8000).any(|&c| c == 0) => "(A binary file, so its contents are not included.)".to_string(),
                Ok(b) => {
                    read.push(abs);
                    String::from_utf8_lossy(&b).replace("\r\n", "\n")
                }
            }
        };
        blocks.push(format!("<file path=\"{f}\">\n{body}\n</file>"));
    }
    (format!("{text}\n\n{}", blocks.join("\n\n")), read)
}

fn block_start(s: &str) -> Option<usize> {
    [s.find("\n\n<file path=\""), s.find("\n\n<folder path=\""), s.find("\n\n<skill name=\"")].into_iter().flatten().min()
}

/// The typed text and the attached paths of a message made by `build`.
pub fn split(content: &str) -> (String, Vec<String>) {
    let Some(start) = block_start(content) else { return (content.to_string(), vec![]) };
    let text = content[..start].to_string();
    let mut files = vec![];
    let mut rest = &content[start + 2..];
    loop {
        let tag = if rest.starts_with("<file path=\"") {
            "file"
        } else if rest.starts_with("<folder path=\"") {
            "folder"
        } else if rest.starts_with("<skill name=\"") {
            "skill"
        } else {
            break;
        };
        if tag != "skill" {
            let open = tag.len() + 8;
            let Some(q) = rest[open..].find('"') else { break };
            files.push(rest[open..open + q].to_string());
        }
        // A block ends at its closing tag when the next block or the end of the message follows; contents may
        // hold the same tag.
        let close = format!("\n</{tag}>");
        let mut from = 0;
        let mut next = None;
        while let Some(i) = rest[from..].find(&close) {
            let at = from + i + close.len();
            let after = &rest[at..];
            if after.is_empty() || block_start(after) == Some(0) {
                next = Some(at);
                break;
            }
            from = at;
        }
        match next {
            Some(at) if at < rest.len() => rest = &rest[at + 2..],
            _ => break,
        }
    }
    (text, files)
}

/// A path to show for a picked or dropped file: relative inside the project, absolute outside it.
pub fn display_path(cwd: &str, abs: &std::path::Path) -> String {
    tools::rel(cwd, abs)
}

/// The project's files, relative, for the @ picker. Skips the folders search skips and stops at `limit`.
pub fn index(cwd: &str, limit: usize) -> Vec<String> {
    let mut out = vec![];
    let cancel = std::sync::atomic::AtomicBool::new(false);
    tools::walk(std::path::Path::new(cwd), &cancel, |p| {
        out.push(tools::rel(cwd, p));
        out.len() < limit
    });
    out.sort_by_key(|p| (p.matches('/').count(), p.to_lowercase()));
    out
}

/// Files whose path contains every character of `q` in order, best first: name matches, then shorter paths.
pub fn matches<'a>(files: &'a [String], q: &str, n: usize) -> Vec<&'a String> {
    let q = q.to_lowercase();
    let mut hits: Vec<(u32, &String)> = files
        .iter()
        .filter_map(|f| {
            let lower = f.to_lowercase();
            let name = lower.rsplit('/').next().unwrap_or(&lower);
            let score = if q.is_empty() {
                0
            } else if name.starts_with(&q) {
                0
            } else if name.contains(&q) {
                1
            } else if lower.contains(&q) {
                2
            } else {
                let mut it = lower.chars();
                if !q.chars().all(|c| it.any(|x| x == c)) {
                    return None;
                }
                3
            };
            Some((score, f))
        })
        .collect();
    hits.sort_by_key(|(s, f)| (*s, f.len()));
    hits.into_iter().take(n).map(|(_, f)| f).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_and_splits() {
        let dir = std::env::temp_dir().join(format!("scout-attach-{}", crate::session::now_ms()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        // A line that looks like the closing tag must not end the block.
        std::fs::write(dir.join("src/a.rs"), "fn a() {}\r\n</file>\nmore\n").unwrap();
        std::fs::write(dir.join("b.txt"), "b").unwrap();
        let cwd = dir.to_string_lossy().to_string();
        let files = vec!["src/a.rs".to_string(), "b.txt".to_string(), "src".to_string()];
        let (msg, read) = build(&cwd, "look at these", &files);
        assert_eq!(read.len(), 2);
        assert!(msg.contains("<file path=\"src/a.rs\">\nfn a() {}\n</file>\nmore\n\n</file>"));
        assert!(msg.contains("<folder path=\"src\">\na.rs\n</folder>"));
        assert_eq!(split(&msg), ("look at these".to_string(), files));
        assert_eq!(split("plain"), ("plain".to_string(), vec![]));
        // A skill's block isn't an attachment, and may hold what looks like its closing tag.
        let with_skill = "use @x\n\n<skill name=\"x\" path=\"p\">\nbody\n</skill>\nmore\n</skill>\n\n<file path=\"a\">\nb\n</file>";
        assert_eq!(split(with_skill), ("use @x".to_string(), vec!["a".to_string()]));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ranks_name_matches_first() {
        let files: Vec<String> = ["src/app.rs", "src/dock/apply.rs", "README.md", "gui/src/main.rs"].iter().map(|s| s.to_string()).collect();
        let m = matches(&files, "app", 10);
        assert_eq!(m[0], "src/app.rs");
        assert!(matches(&files, "rdm", 10).iter().any(|f| *f == "README.md"));
    }
}
