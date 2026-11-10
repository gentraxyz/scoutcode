//! Secrets taken out of text before it enters a conversation, which goes to whatever provider the chat runs on.
//!
//! `view` refuses secret files by name, but the same bytes can arrive another way: `cat .env` in the shell, a file the
//! user attaches, a command that prints a token. Every tool result and attached file goes through `redact` first.
//!
//! It matches only shapes that are almost never anything else (a provider's key format, a private key block, a
//! credential in a URL, an env-style `SOME_TOKEN=value`), so the source code the agent reads and edits stays as it is.
//! It can't catch every secret; the permission modes remain the main defence.

use regex::{Captures, Regex};
use std::borrow::Cow;
use std::sync::OnceLock;

/// What a secret is replaced with. The model is told what it was, so it can say where the value is kept.
pub const MARK: &str = "[REDACTED by Scout]";

fn tokens() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(concat!(
            // OpenAI, Anthropic, OpenRouter and others: sk-…, sk-ant-…, sk-or-…
            r"\bsk-[A-Za-z0-9_-]{20,}",
            // Stripe live and restricted keys.
            r"|\b[sr]k_live_[A-Za-z0-9]{16,}",
            // GitHub tokens.
            r"|\bgh[pousr]_[A-Za-z0-9]{20,}|\bgithub_pat_[A-Za-z0-9_]{20,}",
            // AWS access key ids.
            r"|\b(?:AKIA|ASIA)[0-9A-Z]{16}\b",
            // Slack, Google, npm, Hugging Face, GitLab.
            r"|\bxox[abprs]-[A-Za-z0-9-]{10,}|\bAIza[0-9A-Za-z_-]{30,}|\bnpm_[A-Za-z0-9]{30,}|\bhf_[A-Za-z0-9]{30,}|\bglpat-[A-Za-z0-9_-]{20,}",
            // JWTs, which include Supabase's service-role key.
            r"|\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]+",
        ))
        .unwrap()
    })
}

fn private_keys() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?s)-----BEGIN ([A-Z0-9 ]*)PRIVATE KEY-----.*?(?:-----END [A-Z0-9 ]*PRIVATE KEY-----|\z)").unwrap())
}

/// `scheme://user:password@host`: the password goes, the rest stays.
fn url_credentials() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\b([a-z][a-z0-9+.-]*://[^\s:/@]+:)([^\s@/]{3,})@").unwrap())
}

/// An env-style assignment whose name says it's a secret: `API_KEY=…`, `export GITHUB_TOKEN="…"`, `DB_PASSWORD: …`.
fn assignments() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r##"(?m)^(\s*(?:export\s+|set\s+|\$env:)?[A-Z][A-Z0-9_]*(?:KEY|TOKEN|SECRET|PASSWORD|PASSWD|PWD|CREDENTIALS?|DSN|PRIVATE)[A-Z0-9_]*\s*[=:]\s*['"]?)([^\s'"#]{6,})"##).unwrap()
    })
}

/// `text` with any secrets it holds replaced by [`MARK`]. Borrowed back unchanged when there were none.
pub fn redact(text: &str) -> Cow<'_, str> {
    let mut out = Cow::Borrowed(text);
    let mut apply = |re: &Regex, with: &dyn Fn(&Captures) -> String| {
        if re.is_match(&out) {
            out = Cow::Owned(re.replace_all(&out, |c: &Captures| with(c)).into_owned());
        }
    };
    apply(private_keys(), &|_| format!("{MARK} (a private key)"));
    apply(tokens(), &|_| MARK.to_string());
    apply(url_credentials(), &|c| format!("{}{MARK}@", &c[1]));
    apply(assignments(), &|c| {
        // Already taken out by a pattern above.
        if c[2].contains(MARK) { c[0].to_string() } else { format!("{}{MARK}", &c[1]) }
    });
    out
}

/// Like [`redact`], for an owned string.
pub fn redact_owned(text: String) -> String {
    match redact(&text) {
        Cow::Borrowed(_) => text,
        Cow::Owned(s) => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_keys_are_taken_out() {
        let samples = [
            "OPENAI=sk-proj-abcdefghijklmnopqrstuvwxyz012345",
            "token ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "aws AKIAIOSFODNN7EXAMPLE here",
            "sk_live_abcdefghijklmnop1234",
            "key=AIzaSyA-abcdefghijklmnopqrstuvwxyz12345",
            "eyJhbGciOiJIUzI1NiJ9.eyJyb2xlIjoic2VydmljZV9yb2xlIn0.c2lnbmF0dXJl",
        ];
        for s in samples {
            let r = redact(s);
            assert!(r.contains(MARK), "{s} -> {r}");
        }
        assert!(!redact("ghp_abcdefghijklmnopqrstuvwxyz0123456789").contains("ghp_"));
    }

    #[test]
    fn env_files_lose_their_values_but_keep_their_names() {
        let env = "SUPABASE_URL=https://x.supabase.co\nSUPABASE_SERVICE_ROLE_KEY=abcdef123456\nexport GITHUB_TOKEN=\"zzzzzzzz\"\nDB_PASSWORD: hunter2hunter2\nPORT=3000\n";
        let r = redact(env);
        assert!(r.contains("SUPABASE_URL=https://x.supabase.co"), "{r}");
        assert!(r.contains(&format!("SUPABASE_SERVICE_ROLE_KEY={MARK}")), "{r}");
        assert!(r.contains(&format!("export GITHUB_TOKEN=\"{MARK}")), "{r}");
        assert!(r.contains(&format!("DB_PASSWORD: {MARK}")), "{r}");
        assert!(r.contains("PORT=3000"));
        assert!(!r.contains("hunter2") && !r.contains("abcdef123456") && !r.contains("zzzzzzzz"));
    }

    #[test]
    fn private_keys_and_url_passwords_go() {
        let pem = "before\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk\nAAAA\n-----END OPENSSH PRIVATE KEY-----\nafter";
        let r = redact(pem);
        assert!(r.contains("before") && r.contains("after") && !r.contains("b3BlbnNzaC1rZXk"), "{r}");
        let url = redact("postgres://admin:s3cretpass@db.example.com:5432/app");
        assert_eq!(url, format!("postgres://admin:{MARK}@db.example.com:5432/app"));
    }

    #[test]
    fn ordinary_code_is_left_alone() {
        let code = [
            "let password = read_password()?;",
            "fn api_key(cfg: &Config) -> Result<&str, ApiError> {",
            "const MAX_TOKENS: usize = 4096;",
            "password: String::from(input),",
            "if token.is_empty() { return Err(\"no token\") }",
            "https://example.com/path@v2",
        ];
        for c in code {
            assert!(matches!(redact(c), Cow::Borrowed(_)), "{c} -> {}", redact(c));
        }
    }
}
