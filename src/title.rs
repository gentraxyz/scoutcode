//! Chat titles written by the model. It runs as its own one-shot request on the first message of a chat: the chat's
//! history is not sent and the answer is not added to it, so the chat itself never sees this step.

use crate::api::{ChatRequest, Content, Message};
use crate::config::Config;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

const SYSTEM: &str = "Write a short title for a chat that starts with the message you are given. Use 2 to 6 words, plain text, \
no quotes, no trailing punctuation, in the same language as the message. Reply with the title only.";

/// The model's title for a chat that starts with `first`, or None when the request fails or gives nothing usable.
pub fn generate(cfg: &Config, first: &str) -> Option<String> {
    let text: String = first.chars().take(2000).collect();
    let messages = [Message::System { content: SYSTEM.into() }, Message::User { content: Content::Text(text) }];
    let req = ChatRequest { model: &cfg.model, messages: &messages, tools: None, tool_choice_none: false, temperature: Some(0.2), effort: None, parallel_tool_calls: None };
    let cancel = Arc::new(AtomicBool::new(false));
    let reply = crate::api::stream_chat(cfg, req, &mut |_| {}, &cancel).ok()?;
    clean(&reply.content)
}

/// One line, no wrapping quotes, at most 80 characters.
fn clean(raw: &str) -> Option<String> {
    let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    let t = line.trim_matches(|c: char| c == '"' || c == '\'' || c == '`' || c == '*' || c == '#').trim().trim_end_matches(['.', ':', ';']).trim();
    let t: String = t.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect();
    (!t.is_empty()).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::clean;

    #[test]
    fn cleans_the_reply() {
        assert_eq!(clean("\"Fix the login bug.\"\n").as_deref(), Some("Fix the login bug"));
        assert_eq!(clean("  \n  **Build a snake game**  ").as_deref(), Some("Build a snake game"));
        assert_eq!(clean(" \n ").as_deref(), None);
    }
}
