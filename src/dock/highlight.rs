//! A small, language-agnostic highlighter for the editor: comments, strings, numbers, keywords, types and calls.
//! It knows each language's comment and string syntax from the file extension and nothing more.

use crate::theme;
use eframe::egui::{Color32, FontId, text::LayoutJob, text::TextFormat};

const KEYWORD: Color32 = Color32::from_rgb(0xf5, 0xa5, 0x24);
const STRING: Color32 = Color32::from_rgb(0x9e, 0xce, 0x6a);
const NUMBER: Color32 = Color32::from_rgb(0xf7, 0x8c, 0x6c);
const COMMENT: Color32 = Color32::from_rgb(0x6a, 0x73, 0x80);
const TYPE: Color32 = Color32::from_rgb(0x7d, 0xcf, 0xff);
const CALL: Color32 = Color32::from_rgb(0x9d, 0xb4, 0xff);

/// Past this size the file is shown in one color, so typing stays quick.
const MAX_HIGHLIGHT: usize = 400_000;

const KEYWORDS: &[&str] = &[
    "abstract", "and", "as", "async", "await", "break", "case", "catch", "class", "const", "continue", "crate", "def", "default", "defer", "del",
    "do", "dyn", "elif", "else", "enum", "export", "extends", "extern", "false", "fi", "finally", "fn", "for", "from", "func", "function", "go",
    "if", "impl", "implements", "import", "in", "instanceof", "interface", "is", "lambda", "let", "loop", "match", "mod", "module", "move", "mut",
    "namespace", "new", "nil", "none", "None", "not", "null", "or", "package", "pass", "private", "protected", "pub", "public", "raise", "ref",
    "return", "self", "Self", "static", "struct", "super", "switch", "then", "this", "throw", "throws", "trait", "true", "True", "False", "try",
    "type", "typeof", "undefined", "unsafe", "use", "using", "var", "void", "where", "while", "with", "yield", "readonly", "satisfies", "keyof",
    "goto", "union", "sizeof", "typedef", "select", "chan", "range", "foreach", "elseif", "end", "local", "begin", "until", "unless", "esac",
];

struct Syntax {
    line: &'static [&'static str],
    block: Option<(&'static str, &'static str)>,
    /// Characters that open a string.
    quotes: &'static str,
}

fn syntax(ext: &str) -> Syntax {
    let c_like = Syntax { line: &["//"], block: Some(("/*", "*/")), quotes: "\"'`" };
    match ext {
        "rs" => Syntax { quotes: "\"", ..c_like },
        "py" | "pyw" | "rb" | "sh" | "bash" | "zsh" | "fish" | "toml" | "yaml" | "yml" | "r" | "pl" | "conf" | "ini" | "cfg" | "dockerfile"
        | "makefile" | "mk" | "cmake" | "nix" | "gitignore" | "env" => Syntax { line: &["#"], block: None, quotes: "\"'" },
        "ps1" | "psm1" => Syntax { line: &["#"], block: Some(("<#", "#>")), quotes: "\"'" },
        "sql" | "lua" | "hs" => Syntax { line: &["--"], block: Some(("/*", "*/")), quotes: "\"'" },
        "css" | "scss" | "less" => Syntax { line: &[], block: Some(("/*", "*/")), quotes: "\"'" },
        "html" | "htm" | "xml" | "svg" | "vue" | "md" | "markdown" => Syntax { line: &[], block: Some(("<!--", "-->")), quotes: "\"" },
        "json" | "jsonc" => Syntax { line: &["//"], block: Some(("/*", "*/")), quotes: "\"" },
        "bat" | "cmd" => Syntax { line: &["::", "REM ", "rem "], block: None, quotes: "\"" },
        "txt" | "log" | "" => Syntax { line: &[], block: None, quotes: "" },
        _ => c_like,
    }
}

pub fn job(text: &str, ext: &str, font: &FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    let fmt = |color: Color32| TextFormat { font_id: font.clone(), color, ..Default::default() };
    if text.len() > MAX_HIGHLIGHT {
        job.append(text, 0.0, fmt(theme::TEXT));
        return job;
    }
    let syn = syntax(ext);
    let b = text.as_bytes();
    let mut i = 0;
    // Plain text is gathered and appended in one section, so the job stays small.
    let mut plain = 0;
    let push = |job: &mut LayoutJob, plain: &mut usize, start: usize, end: usize, color: Color32| {
        if *plain < start {
            job.append(&text[*plain..start], 0.0, fmt(theme::TEXT));
        }
        job.append(&text[start..end], 0.0, fmt(color));
        *plain = end;
    };
    let starts = |i: usize, s: &str| b[i..].starts_with(s.as_bytes());
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80;
    while i < b.len() {
        let c = b[i];
        if syn.line.iter().any(|t| starts(i, t)) {
            let end = text[i..].find('\n').map_or(b.len(), |n| i + n);
            push(&mut job, &mut plain, i, end, COMMENT);
            i = end;
        } else if let Some((open, close)) = syn.block.filter(|(o, _)| starts(i, o)) {
            let end = text[i + open.len()..].find(close).map_or(b.len(), |n| i + open.len() + n + close.len());
            push(&mut job, &mut plain, i, end, COMMENT);
            i = end;
        } else if syn.quotes.as_bytes().contains(&c) {
            // To the closing quote, skipping escapes; only backticks run past the end of the line.
            let mut j = i + 1;
            while j < b.len() && b[j] != c && (c == b'`' || b[j] != b'\n') {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            let mut end = (j + 1).min(b.len());
            while !text.is_char_boundary(end) {
                end += 1;
            }
            push(&mut job, &mut plain, i, end, STRING);
            i = end;
        } else if c.is_ascii_digit() && (i == 0 || !ident(b[i - 1])) {
            let mut j = i + 1;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'.' || b[j] == b'_') {
                j += 1;
            }
            push(&mut job, &mut plain, i, j, NUMBER);
            i = j;
        } else if ident(c) && (i == 0 || !ident(b[i - 1])) {
            let mut j = i + 1;
            while j < b.len() && ident(b[j]) {
                j += 1;
            }
            // Stay on a char boundary: identifiers may hold multi-byte letters.
            while !text.is_char_boundary(j) {
                j += 1;
            }
            let word = &text[i..j];
            let next = b[j..].iter().find(|c| **c != b' ').copied();
            let color = if KEYWORDS.contains(&word) {
                Some(KEYWORD)
            } else if next == Some(b'(') || (next == Some(b'!') && ext == "rs") {
                Some(CALL)
            } else if word.as_bytes()[0].is_ascii_uppercase() && word.len() > 1 {
                Some(TYPE)
            } else {
                None
            };
            if let Some(color) = color {
                push(&mut job, &mut plain, i, j, color);
            }
            i = j;
        } else {
            i += 1;
            while i < b.len() && !text.is_char_boundary(i) {
                i += 1;
            }
        }
    }
    if plain < b.len() {
        job.append(&text[plain..], 0.0, fmt(theme::TEXT));
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every byte ends up in exactly one section, whatever the input.
    #[test]
    fn covers_all_text() {
        let font = FontId::monospace(13.0);
        for (text, ext) in [
            ("fn main() { let s = \"héllo \\\"x\\\"\"; // ünï\n    Vec::new(); 0x1f }", "rs"),
            ("const a = `multi\nline ${x}`; /* open comment", "ts"),
            ("x = 'unterminated\n# comment é\nprint(1.5e3)", "py"),
            ("\"\\é", "js"),
            ("<!-- c --> <a href=\"x\">ö</a>", "html"),
            ("", "rs"),
        ] {
            let job = job(text, ext, &font);
            assert_eq!(job.text, text);
            let covered: usize = job.sections.iter().map(|s| s.byte_range.end.0 - s.byte_range.start.0).sum();
            assert_eq!(covered, text.len(), "{text:?}");
        }
    }

    #[test]
    fn colors_tokens() {
        let font = FontId::monospace(13.0);
        let job = job("let x = \"s\"; // c", "rs", &font);
        let color_of = |needle: &str| {
            let at = job.text.find(needle).unwrap();
            job.sections.iter().find(|s| (s.byte_range.start.0..s.byte_range.end.0).contains(&at)).unwrap().format.color
        };
        assert_eq!(color_of("let"), KEYWORD);
        assert_eq!(color_of("\"s\""), STRING);
        assert_eq!(color_of("// c"), COMMENT);
        assert_eq!(color_of("x"), theme::TEXT);
    }
}
