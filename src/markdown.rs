//! A small Markdown renderer for chat replies: headings, lists, quotes, code blocks, tables (as monospace), and inline
//! code, bold, italic and links. Parsed once per change; drawing reuses egui's galley cache.

use crate::theme;
use eframe::egui::{self, Color32, FontId, RichText, text::LayoutJob, text::TextFormat};

#[derive(Clone, Debug)]
pub enum Block {
    Heading(u8, String),
    Para(String),
    Code(String),
    Item { indent: usize, marker: String, text: String },
    Quote(String),
    Mono(String),
    Rule,
}

pub fn parse(src: &str) -> Vec<Block> {
    let mut out = vec![];
    let mut para: Vec<&str> = vec![];
    let lines: Vec<&str> = src.lines().collect();
    let flush = |para: &mut Vec<&str>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::Para(para.join("\n")));
            para.clear();
        }
    };
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim_start();
        let indent = line.len() - t.len();
        if t.starts_with("```") || t.starts_with("~~~") {
            flush(&mut para, &mut out);
            let fence = &t[..3];
            let mut code = vec![];
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with(fence) {
                code.push(lines[i]);
                i += 1;
            }
            out.push(Block::Code(code.join("\n")));
        } else if t.is_empty() {
            flush(&mut para, &mut out);
        } else if let Some(level) = heading(t) {
            flush(&mut para, &mut out);
            out.push(Block::Heading(level, t[level as usize..].trim().trim_end_matches('#').trim().to_string()));
        } else if t.len() >= 3 && (t.chars().all(|c| c == '-' || c == ' ') || t.chars().all(|c| c == '*' || c == ' ') || t.chars().all(|c| c == '_')) && t.chars().filter(|c| !c.is_whitespace()).count() >= 3 {
            flush(&mut para, &mut out);
            out.push(Block::Rule);
        } else if let Some(rest) = t.strip_prefix("> ").or_else(|| (t == ">").then_some("")) {
            flush(&mut para, &mut out);
            let mut q = vec![rest];
            while i + 1 < lines.len() && lines[i + 1].trim_start().starts_with('>') {
                i += 1;
                q.push(lines[i].trim_start().trim_start_matches('>').trim_start());
            }
            out.push(Block::Quote(q.join("\n")));
        } else if t.starts_with('|') {
            flush(&mut para, &mut out);
            let mut rows = vec![t];
            while i + 1 < lines.len() && lines[i + 1].trim_start().starts_with('|') {
                i += 1;
                rows.push(lines[i].trim());
            }
            out.push(Block::Mono(table(&rows)));
        } else if let Some((marker, text)) = list_item(t) {
            flush(&mut para, &mut out);
            out.push(Block::Item { indent: indent / 2, marker, text: text.to_string() });
        } else {
            para.push(line.trim_end());
        }
        i += 1;
    }
    flush(&mut para, &mut out);
    out
}

fn heading(t: &str) -> Option<u8> {
    let n = t.bytes().take_while(|&b| b == b'#').count();
    ((1..=6).contains(&n) && t.as_bytes().get(n) == Some(&b' ')).then_some(n as u8)
}

fn list_item(t: &str) -> Option<(String, &str)> {
    for m in ["- [ ] ", "- [x] ", "- ", "* ", "+ "] {
        if let Some(rest) = t.strip_prefix(m) {
            let marker = match m {
                "- [ ] " => "☐",
                "- [x] " => "☑",
                _ => "•",
            };
            return Some((marker.to_string(), rest));
        }
    }
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && digits < 4 {
        let rest = &t[digits..];
        if let Some(r) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return Some((format!("{}.", &t[..digits]), r));
        }
    }
    None
}

/// Lines a table up in columns, dropping the separator row.
fn table(rows: &[&str]) -> String {
    let cells: Vec<Vec<String>> = rows
        .iter()
        .filter(|r| !r.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ')))
        .map(|r| r.trim_matches('|').split('|').map(|c| c.trim().replace("**", "").replace('`', "")).collect())
        .collect();
    let cols = cells.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols).map(|c| cells.iter().map(|r| r.get(c).map_or(0, |s| s.chars().count())).max().unwrap_or(0)).collect();
    let mut out = vec![];
    for (i, r) in cells.iter().enumerate() {
        let line: Vec<String> = (0..cols).map(|c| format!("{:w$}", r.get(c).map_or("", String::as_str), w = widths[c])).collect();
        out.push(line.join("  ").trim_end().to_string());
        if i == 0 && cells.len() > 1 {
            out.push(widths.iter().map(|w| "─".repeat(*w)).collect::<Vec<_>>().join("  "));
        }
    }
    out.join("\n")
}

/// Inline `code`, **bold**, *italic* and [links](url) as one wrapped layout.
pub fn inline(text: &str, size: f32, color: Color32, strong: bool) -> LayoutJob {
    let mut job = LayoutJob::default();
    let body = if strong { theme::bold(size) } else { FontId::proportional(size) };
    let plain = TextFormat { font_id: body.clone(), color, ..Default::default() };
    let b: Vec<char> = text.chars().collect();
    let mut buf = String::new();
    let mut bold = strong;
    let mut italic = false;
    let fmt = |bold: bool, italic: bool| TextFormat {
        font_id: if bold { theme::bold(size) } else { FontId::proportional(size) },
        color: if bold { theme::STRONG } else { color },
        italics: italic,
        ..Default::default()
    };
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == '`' {
            if let Some(end) = b[i + 1..].iter().position(|&x| x == '`') {
                job.append(&std::mem::take(&mut buf), 0.0, fmt(bold, italic));
                let code: String = b[i + 1..i + 1 + end].iter().collect();
                // Hair spaces outside the code so its padded background doesn't touch the words around it.
                job.append("\u{200A}", 0.0, fmt(bold, italic));
                job.append(&code, 0.0, TextFormat { font_id: FontId::monospace(size - 1.5), color: theme::CODE, background: theme::SURFACE_HI, expand_bg: 2.0, ..Default::default() });
                job.append("\u{200A}", 0.0, fmt(bold, italic));
                i += end + 2;
                continue;
            }
        }
        if c == '*' && b.get(i + 1) == Some(&'*') {
            job.append(&std::mem::take(&mut buf), 0.0, fmt(bold, italic));
            bold = !bold;
            i += 2;
            continue;
        }
        if (c == '*' || c == '_') && b.get(i + 1).is_some_and(|n| !n.is_whitespace()) && (i == 0 || !b[i - 1].is_alphanumeric()) && b[i + 1..].contains(&c) && !italic {
            job.append(&std::mem::take(&mut buf), 0.0, fmt(bold, italic));
            italic = true;
            i += 1;
            continue;
        }
        if (c == '*' || c == '_') && italic && b.get(i + 1).is_none_or(|n| !n.is_alphanumeric()) {
            job.append(&std::mem::take(&mut buf), 0.0, fmt(bold, italic));
            italic = false;
            i += 1;
            continue;
        }
        if c == '[' {
            if let Some(close) = b[i..].iter().position(|&x| x == ']') {
                if b.get(i + close + 1) == Some(&'(') {
                    if let Some(end) = b[i + close..].iter().position(|&x| x == ')') {
                        job.append(&std::mem::take(&mut buf), 0.0, fmt(bold, italic));
                        let label: String = b[i + 1..i + close].iter().collect();
                        job.append(&label, 0.0, TextFormat { font_id: body.clone(), color: theme::ACCENT, underline: egui::Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.5)), ..Default::default() });
                        i += close + end + 1;
                        continue;
                    }
                }
            }
        }
        buf.push(c);
        i += 1;
    }
    job.append(&buf, 0.0, fmt(bold, italic));
    if job.sections.is_empty() {
        job.append("", 0.0, plain);
    }
    job.wrap.max_width = f32::INFINITY;
    job
}

pub fn show(ui: &mut egui::Ui, blocks: &[Block]) {
    let size = theme::BODY;
    ui.spacing_mut().item_spacing.y = 0.0;
    for (n, b) in blocks.iter().enumerate() {
        if n > 0 {
            let tight = matches!((&blocks[n - 1], b), (Block::Item { .. }, Block::Item { .. }));
            ui.add_space(if tight { 4.0 } else { 10.0 });
        }
        match b {
            Block::Heading(level, t) => {
                let s = match level {
                    1 => size + 6.0,
                    2 => size + 3.5,
                    3 => size + 1.5,
                    _ => size,
                };
                if n > 0 {
                    ui.add_space(6.0);
                }
                ui.add(egui::Label::new(inline(t, s, theme::STRONG, true)).wrap());
            }
            Block::Para(t) => {
                ui.add(egui::Label::new(inline(t, size, theme::TEXT, false)).wrap());
            }
            Block::Item { indent, marker, text } => {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.add_space(2.0 + *indent as f32 * 20.0);
                    // A fixed gutter so wrapped lines and numbered items line up.
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, size + 6.0), egui::Sense::hover());
                    if marker == "•" {
                        ui.painter().circle_filled(egui::pos2(rect.left() + 6.0, rect.center().y), 2.4, theme::DIM);
                    } else {
                        ui.painter().text(egui::pos2(rect.left() + 2.0, rect.center().y), egui::Align2::LEFT_CENTER, marker, FontId::proportional(size - 0.5), theme::DIM);
                    }
                    ui.add(egui::Label::new(inline(text, size, theme::TEXT, false)).wrap());
                });
            }
            Block::Quote(t) => {
                egui::Frame::NONE.inner_margin(egui::Margin { left: 14, right: 0, top: 2, bottom: 2 }).show(ui, |ui| {
                    let r = ui.add(egui::Label::new(inline(t, size, theme::MUTED, false)).wrap());
                    let x = r.rect.left() - 12.0;
                    ui.painter().vline(x, r.rect.y_range(), egui::Stroke::new(3.0, theme::BORDER_HI));
                });
            }
            Block::Code(code) => code_block(ui, code),
            Block::Mono(t) => {
                egui::Frame::NONE.fill(theme::CODE_BG).stroke(egui::Stroke::new(1.0, theme::BORDER)).corner_radius(8).inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::horizontal().id_salt(ui.next_auto_id()).show(ui, |ui| {
                        ui.label(RichText::new(t).font(FontId::monospace(size - 1.5)).color(theme::TEXT));
                    });
                });
            }
            Block::Rule => {
                ui.add_space(4.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 0, theme::BORDER);
                ui.add_space(4.0);
            }
        }
    }
}

pub fn code_block(ui: &mut egui::Ui, code: &str) {
    let r = egui::Frame::NONE.fill(theme::CODE_BG).stroke(egui::Stroke::new(1.0, theme::BORDER)).corner_radius(8).inner_margin(egui::Margin::symmetric(14, 11)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(egui::Label::new(RichText::new(code).font(FontId::monospace(theme::BODY - 1.5)).color(theme::TEXT).line_height(Some(19.0))).wrap());
    });
    let rect = r.response.rect;
    if ui.rect_contains_pointer(rect) {
        let btn = egui::Rect::from_min_size(egui::pos2(rect.right() - 58.0, rect.top() + 6.0), egui::vec2(52.0, 22.0));
        let b = ui.put(btn, egui::Button::new(RichText::new("Copy").size(11.5).color(theme::MUTED)).fill(theme::SURFACE_HI).corner_radius(6));
        if b.clicked() {
            ui.ctx().copy_text(code.to_string());
        }
    }
}
