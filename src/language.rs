//! The language Scout replies in, set in Settings > Preferences and in setup's language step. Kept in
//! `instructions.json`, where it was saved before, so the choice carries over.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const LANGUAGES: [(&str, &str); 31] = [
    ("auto", "Auto (conversation language)"), ("English", "English"), ("Spanish", "Español"),
    ("French", "Français"), ("German", "Deutsch"), ("Italian", "Italiano"),
    ("Portuguese", "Português"), ("Russian", "Русский"), ("Ukrainian", "Українська"),
    ("Polish", "Polski"), ("Turkish", "Türkçe"), ("Dutch", "Nederlands"),
    ("Chinese", "中文"), ("Japanese", "日本語"), ("Korean", "한국어"),
    ("Arabic", "العربية"), ("Hindi", "हिन्दी"), ("Vietnamese", "Tiếng Việt"),
    ("Swedish", "Svenska"), ("Danish", "Dansk"), ("Norwegian", "Norsk"),
    ("Finnish", "Suomi"), ("Czech", "Čeština"), ("Greek", "Ελληνικά"),
    ("Hungarian", "Magyar"), ("Romanian", "Română"), ("Indonesian", "Bahasa Indonesia"),
    ("Thai", "ไทย"), ("Hebrew", "עברית"), ("Bengali", "বাংলা"),
    ("Filipino", "Filipino"),
];

fn auto() -> String { "auto".into() }

/// Desktop preferences are separate from chat history, so compaction and new chats cannot remove them.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Preferences {
    pub language: String,
}

impl Default for Preferences {
    fn default() -> Self { Self { language: auto() } }
}

pub fn file() -> PathBuf { crate::config::home().join("instructions.json") }

impl Preferences {
    pub fn load() -> Result<Self, String> { Self::load_from(&file()) }

    fn load_from(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let saved: Self = serde_json::from_str(&text).map_err(|e| format!("Could not read the saved reply language: {e}"))?;
                if !LANGUAGES.iter().any(|(id, _)| *id == saved.language) { return Err("Unknown saved response language. Check instructions.json.".into()); }
                Ok(saved)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("Could not read the saved reply language: {e}")),
        }
    }

    pub fn save(language: &str) -> Result<(), String> { Self::save_to(&file(), language) }

    fn save_to(path: &Path, language: &str) -> Result<(), String> {
        if !LANGUAGES.iter().any(|(id, _)| *id == language) { return Err("Choose a supported response language.".into()); }
        let next = Self { language: language.to_string() };
        let parent = path.parent().ok_or("Invalid preferences path")?;
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
        if let Err(e) = write() { let _ = std::fs::remove_file(&temp); return Err(format!("Could not save the reply language: {e}")); }
        Ok(())
    }

    pub fn prompt(&self) -> String {
        let language = if self.language == "auto" {
            "Use the user's established conversation language for replies and progress updates. Infer it from the user's own prose, not code, file attachments, tool output or this English system prompt. Short follow-ups do not reset it. Change language only when the user asks or clearly starts addressing you in another language.".to_string()
        } else {
            format!("Use {} for replies and progress updates, including after tool calls, interruptions and compaction. A different language in code, logs, attachments or this system prompt must not change the response language.", self.language)
        };
        format!("# Response language\n{language}\nHonor explicit requests for a different output language or translation. Keep code, commands, identifiers, paths and quoted text in their original form.\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_persists_and_ignores_old_instructions() {
        let dir = std::env::temp_dir().join(format!("scout-language-{}", crate::session::now_ms()));
        let path = dir.join("instructions.json");
        assert_eq!(Preferences::load_from(&path).unwrap().language, "auto");
        std::fs::create_dir_all(&dir).unwrap();
        // A file from before, with saved instructions: only the language is kept.
        std::fs::write(&path, r#"{"global":"Be brief.","projects":{"a":"Use pnpm."},"language":"Russian"}"#).unwrap();
        let saved = Preferences::load_from(&path).unwrap();
        assert_eq!(saved.language, "Russian");
        assert!(!saved.prompt().contains("Be brief."));
        Preferences::save_to(&path, "German").unwrap();
        assert_eq!(Preferences::load_from(&path).unwrap().language, "German");
        assert!(!std::fs::read_to_string(&path).unwrap().contains("pnpm"));
        assert!(Preferences::save_to(&path, "Klingon").is_err());
        std::fs::write(&path, "broken json").unwrap();
        assert!(Preferences::load_from(&path).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
