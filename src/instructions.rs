use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const MAX_CHARS: usize = 40_000;
pub const LANGUAGES: [(&str, &str); 18] = [
    ("auto", "Auto (conversation language)"), ("English", "English"), ("Russian", "Русский"),
    ("Ukrainian", "Українська"), ("Spanish", "Español"), ("French", "Français"),
    ("German", "Deutsch"), ("Portuguese", "Português"), ("Italian", "Italiano"),
    ("Polish", "Polski"), ("Turkish", "Türkçe"), ("Chinese", "中文"),
    ("Japanese", "日本語"), ("Korean", "한국어"), ("Arabic", "العربية"),
    ("Hindi", "हिन्दी"), ("Dutch", "Nederlands"), ("Vietnamese", "Tiếng Việt"),
];

fn auto() -> String { "auto".into() }

/// Desktop preferences are separate from chat history, so compaction and new chats cannot remove them.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Preferences {
    pub global: String,
    pub projects: BTreeMap<String, String>,
    pub language: String,
}

impl Default for Preferences {
    fn default() -> Self { Self { global: String::new(), projects: BTreeMap::new(), language: auto() } }
}

pub fn file() -> PathBuf { crate::config::home().join("instructions.json") }

fn project_key(cwd: &str) -> String {
    let path = std::fs::canonicalize(cwd).unwrap_or_else(|_| PathBuf::from(cwd));
    let key = path.to_string_lossy().trim_start_matches(r"\\?\").replace('\\', "/");
    if cfg!(windows) { key.to_lowercase() } else { key }
}

impl Preferences {
    pub fn load() -> Result<Self, String> { Self::load_from(&file()) }

    fn load_from(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let saved: Self = serde_json::from_str(&text).map_err(|e| format!("Could not read saved instructions: {e}"))?;
                if !LANGUAGES.iter().any(|(id, _)| *id == saved.language) { return Err("Unknown saved response language. Check instructions.json.".into()); }
                Ok(saved)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("Could not read saved instructions: {e}")),
        }
    }

    pub fn project(&self, cwd: &str) -> &str { self.projects.get(&project_key(cwd)).map(String::as_str).unwrap_or("") }

    pub fn save_changes(&self, cwd: &str, global: &str, project: &str, language: &str) -> Result<Self, String> {
        self.save_to(&file(), cwd, global, project, language)
    }

    fn save_to(&self, path: &Path, cwd: &str, global: &str, project: &str, language: &str) -> Result<Self, String> {
        if global.chars().count() > MAX_CHARS || project.chars().count() > MAX_CHARS {
            return Err(format!("Each instructions section is limited to {MAX_CHARS} characters."));
        }
        if !LANGUAGES.iter().any(|(id, _)| *id == language) { return Err("Choose a supported response language.".into()); }
        // Preserve other projects, and refuse to overwrite the same preferences edited in another window.
        let mut next = Self::load_from(path)?;
        if next.global != self.global || next.language != self.language || next.project(cwd) != self.project(cwd) {
            return Err("These instructions changed in another window. Reload before saving.".into());
        }
        next.global = global.to_string();
        next.language = language.to_string();
        let key = project_key(cwd);
        if project.trim().is_empty() { next.projects.remove(&key); } else { next.projects.insert(key, project.to_string()); }
        let parent = path.parent().ok_or("Invalid instructions path")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).map_err(|e| e.to_string())?;
        let temp = parent.join(format!("instructions-{:016x}.tmp", u64::from_le_bytes(random)));
        let write = || -> Result<(), String> {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e| e.to_string())?;
            f.write_all(serde_json::to_string_pretty(&next).map_err(|e| e.to_string())?.as_bytes()).map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())?;
            drop(f);
            std::fs::rename(&temp, path).map_err(|e| e.to_string())
        };
        if let Err(e) = write() { let _ = std::fs::remove_file(&temp); return Err(format!("Could not save instructions: {e}")); }
        Ok(next)
    }

    pub fn prompt(&self, cwd: &str) -> String {
        let language = if self.language == "auto" {
            "Use the user's established conversation language for replies and progress updates. Infer it from the user's own prose, not code, file attachments, tool output or this English system prompt. Short follow-ups do not reset it. Change language only when the user asks or clearly starts addressing you in another language.".to_string()
        } else {
            format!("Use {} for replies and progress updates, including after tool calls, interruptions and compaction. A different language in code, logs, attachments or this system prompt must not change the response language.", self.language)
        };
        let mut text = format!("# Response language\n{language}\nHonor explicit requests for a different output language or translation. Keep code, commands, identifiers, paths and quoted text in their original form.\n");
        if !self.global.trim().is_empty() { text.push_str(&format!("\n# Saved instructions for all projects\n{}\n", self.global.trim())); }
        if !self.project(cwd).trim().is_empty() { text.push_str(&format!("\n# Saved instructions for this project\n{}\n", self.project(cwd).trim())); }
        text.push_str("\nSaved instructions apply throughout this chat and after compaction. Project-specific instructions take precedence over general preferences; follow the user's current request when they explicitly change them.\n");
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_persist_and_keep_projects_separate() {
        let dir = std::env::temp_dir().join(format!("scout-instructions-{}", crate::session::now_ms()));
        let path = dir.join("instructions.json");
        let a = dir.join("project-a").to_string_lossy().to_string();
        let b = dir.join("project-b").to_string_lossy().to_string();
        let initial = Preferences::load_from(&path).unwrap();
        let first = initial.save_to(&path, &a, "Use concise explanations.", "Use pnpm.", "Russian").unwrap();
        // Another project's edit must survive saving a draft opened before that edit.
        first.save_to(&path, &b, &first.global, "Use cargo.", "Russian").unwrap();
        first.save_to(&path, &a, &first.global, "Use npm.", "Russian").unwrap();
        let saved = Preferences::load_from(&path).unwrap();
        assert_eq!(saved.project(&a), "Use npm.");
        assert_eq!(saved.project(&b), "Use cargo.");
        assert_eq!(saved.language, "Russian");
        assert!(saved.prompt(&a).contains("Use npm.") && !saved.prompt(&a).contains("Use cargo."));
        assert!(first.save_to(&path, &a, "overwrite", "Use pnpm.", "auto").is_err());
        let cleared = saved.save_to(&path, &a, "", "", "auto").unwrap();
        assert!(cleared.project(&a).is_empty());
        assert_eq!(cleared.project(&b), "Use cargo.");
        std::fs::write(&path, "broken json").unwrap();
        assert!(Preferences::load_from(&path).is_err());
        assert!(cleared.save_to(&path, &a, "", "", "auto").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "broken json");
        let _ = std::fs::remove_dir_all(dir);
    }
}
