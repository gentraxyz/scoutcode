//! A real terminal: the shell runs in a pseudo console (ConPTY on Windows) and vt100 keeps the screen.

use crate::theme;
use eframe::egui::{self, Color32, FontId, Key, Sense, Stroke};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub fn id() -> egui::Id {
    egui::Id::unique("dock_terminal")
}

const FONT_SIZE: f32 = 13.0;
const PAD: f32 = 10.0;
const SCROLLBACK: usize = 5000;

struct Session {
    parser: Arc<Mutex<vt100::Parser>>,
    /// Shared with the reader thread, which answers the shell's questions about the cursor.
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    done: Arc<AtomicBool>,
    size: (u16, u16),
    shell: String,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[derive(Default)]
pub struct Terminal {
    session: Option<Session>,
    error: Option<String>,
    /// Rows scrolled back into history; 0 is the live screen.
    scroll: usize,
    /// Leftover wheel movement smaller than a row.
    scroll_acc: f32,
    /// Selected cells, as (row, col) on the visible screen: where the drag started, and where it is now.
    selection: Option<((u16, u16), (u16, u16))>,
    /// Take the keyboard on the next frame.
    pub focus: bool,
}

/// PowerShell 7 when it's installed, else Windows PowerShell; $SHELL elsewhere. SCOUT_TERMINAL overrides both.
fn shell() -> CommandBuilder {
    if let Ok(s) = std::env::var("SCOUT_TERMINAL") {
        return CommandBuilder::new(s);
    }
    if cfg!(windows) {
        let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
        let pwsh = std::path::Path::new(&pf).join(r"PowerShell\7\pwsh.exe");
        let mut c = CommandBuilder::new(if pwsh.exists() { pwsh.to_string_lossy().to_string() } else { "powershell.exe".into() });
        c.arg("-NoLogo");
        c
    } else {
        CommandBuilder::new(std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into()))
    }
}

fn spawn(cwd: &str, rows: u16, cols: u16, ctx: egui::Context) -> Result<Session, String> {
    let pair = native_pty_system().openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }).map_err(|e| e.to_string())?;
    let mut cmd = shell();
    let shell = std::path::Path::new(cmd.get_argv()[0].to_str().unwrap_or("shell")).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    cmd.cwd(cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = Arc::new(Mutex::new(pair.master.take_writer().map_err(|e| e.to_string())?));
    let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, SCROLLBACK)));
    let done = Arc::new(AtomicBool::new(false));
    let (p, d, w) = (parser.clone(), done.clone(), writer.clone());
    std::thread::Builder::new()
        .name("scout-terminal".into())
        .spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let mut parser = p.lock().unwrap();
                parser.process(&buf[..n]);
                // ConPTY asks where the cursor is (DSR) when it starts, and draws nothing until it hears back.
                if buf[..n].windows(4).any(|s| s == b"[6n") {
                    let (row, col) = parser.screen().cursor_position();
                    let _ = w.lock().unwrap().write_all(format!("[{};{}R", row + 1, col + 1).as_bytes());
                }
                drop(parser);
                ctx.request_repaint();
            }
            d.store(true, Ordering::Relaxed);
            ctx.request_repaint();
        })
        .map_err(|e| e.to_string())?;
    Ok(Session { parser, writer, master: pair.master, child, done, size: (rows, cols), shell })
}

impl Terminal {
    fn exited(&mut self) -> bool {
        match &mut self.session {
            Some(s) => s.done.load(Ordering::Relaxed) || matches!(s.child.try_wait(), Ok(Some(_))),
            None => false,
        }
    }

    fn restart(&mut self) {
        self.session = None;
        self.error = None;
        self.scroll = 0;
        self.selection = None;
        self.focus = true;
    }

    /// The header's buttons for this tab.
    pub fn actions(&mut self, ui: &mut egui::Ui) {
        if theme::icon_button(ui, 26.0, "Restart the shell in this project", theme::icon::refresh).clicked() {
            self.restart();
        }
        if let Some(s) = &self.session {
            ui.label(egui::RichText::new(&s.shell).size(12.0).color(theme::DIM));
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cwd: &str) {
        let font = FontId::monospace(FONT_SIZE);
        let (cw, rh) = ui.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let resp = ui.interact(rect, id(), Sense::click_and_drag()).union(resp);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0, theme::CODE_BG);
        let inner = rect.shrink(PAD);
        let cols = ((inner.width() / cw).floor() as u16).max(20);
        let rows = ((inner.height() / rh).floor() as u16).max(4);

        if self.session.is_none() && self.error.is_none() {
            match spawn(cwd, rows, cols, ui.ctx().clone()) {
                Ok(s) => self.session = Some(s),
                Err(e) => self.error = Some(e),
            }
        }
        if let Some(e) = &self.error {
            painter.text(inner.min, egui::Align2::LEFT_TOP, format!("Could not start a shell: {e}"), FontId::proportional(13.0), theme::RED);
            return;
        }
        let exited = self.exited();
        let s = self.session.as_mut().unwrap();
        if s.size != (rows, cols) {
            let _ = s.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
            s.parser.lock().unwrap().screen_mut().set_size(rows, cols);
            s.size = (rows, cols);
            self.selection = None;
        }

        if resp.clicked() || resp.drag_started() || std::mem::take(&mut self.focus) {
            resp.request_focus();
        }
        let focused = resp.has_focus();
        if focused {
            ui.memory_mut(|m| m.set_focus_lock_filter(id(), egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true }));
        }

        // Where the pointer is, as a cell.
        let cell_at = |p: egui::Pos2| -> (u16, u16) {
            let col = ((p.x - inner.left()) / cw).floor().clamp(0.0, cols as f32 - 1.0) as u16;
            let row = ((p.y - inner.top()) / rh).floor().clamp(0.0, rows as f32 - 1.0) as u16;
            (row, col)
        };
        if resp.drag_started_by(egui::PointerButton::Primary) {
            if let Some(p) = resp.interact_pointer_pos() {
                let c = cell_at(p);
                self.selection = Some((c, c));
            }
        } else if resp.dragged_by(egui::PointerButton::Primary) {
            if let (Some(p), Some((a, _))) = (resp.interact_pointer_pos(), self.selection) {
                self.selection = Some((a, cell_at(p)));
            }
        } else if resp.clicked() {
            self.selection = None;
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }

        let mut parser = s.parser.lock().unwrap();
        // The wheel walks the scrollback (the full-screen apps that use the alternate screen get arrow keys instead).
        if resp.hovered() {
            let dy = ui.input(|i| i.smooth_scroll_delta.y);
            if dy != 0.0 {
                self.scroll_acc += dy / rh;
                let steps = self.scroll_acc.trunc();
                self.scroll_acc -= steps;
                if steps != 0.0 {
                    if parser.screen().alternate_screen() {
                        let key = if steps > 0.0 { b"\x1b[A" } else { b"\x1b[B" };
                        for _ in 0..(steps.abs() as usize) {
                            let _ = s.writer.lock().unwrap().write_all(key);
                        }
                    } else {
                        self.scroll = (self.scroll as i64 + steps as i64).max(0) as usize;
                    }
                }
            }
        }

        let mut out: Vec<u8> = vec![];
        if focused && !exited {
            let screen = parser.screen();
            let app_cursor = screen.application_cursor();
            let bracketed = screen.bracketed_paste();
            let events = ui.input(|i| i.events.clone());
            for ev in events {
                match ev {
                    egui::Event::Text(t) => out.extend(t.as_bytes()),
                    egui::Event::Paste(t) => {
                        let t = t.replace("\r\n", "\n").replace('\n', "\r");
                        if bracketed {
                            out.extend(b"\x1b[200~");
                            out.extend(t.as_bytes());
                            out.extend(b"\x1b[201~");
                        } else {
                            out.extend(t.as_bytes());
                        }
                    }
                    egui::Event::Copy => {
                        if !copy_selection(&mut self.selection, ui, screen) {
                            out.push(0x03);
                        }
                    }
                    egui::Event::Cut => out.push(0x18),
                    egui::Event::Key { key, pressed: true, modifiers, .. } => {
                        if modifiers.ctrl && modifiers.shift && key == Key::C {
                            copy_selection(&mut self.selection, ui, screen);
                        } else if let Some(b) = key_bytes(key, modifiers, app_cursor) {
                            out.extend(b);
                        }
                    }
                    _ => {}
                }
            }
        }
        if !out.is_empty() {
            let mut w = s.writer.lock().unwrap();
            let _ = w.write_all(&out);
            let _ = w.flush();
            drop(w);
            self.scroll = 0;
            self.selection = None;
        }
        parser.screen_mut().set_scrollback(self.scroll);
        self.scroll = parser.screen().scrollback();
        let screen = parser.screen();

        // Cells, a run of same-styled ASCII at a time; anything else on its own, pinned to its column.
        let sel = self.selection.map(|(a, b)| if a <= b { (a, b) } else { (b, a) });
        let in_sel = |r: u16, c: u16| sel.is_some_and(|(a, b)| a != b && (r, c) >= a && (r, c) <= b);
        for r in 0..rows {
            let y = inner.top() + r as f32 * rh;
            let mut c = 0;
            while c < cols {
                let Some(cell) = screen.cell(r, c) else { break };
                let style = Style::of(cell, in_sel(r, c));
                let start = c;
                let mut text = String::new();
                let wide = cell.is_wide();
                let ascii = cell.contents().is_ascii();
                text.push_str(if cell.has_contents() { cell.contents() } else { " " });
                c += if wide { 2 } else { 1 };
                if ascii && !wide {
                    while c < cols {
                        let Some(n) = screen.cell(r, c) else { break };
                        if !n.contents().is_ascii() || n.is_wide() || Style::of(n, in_sel(r, c)) != style {
                            break;
                        }
                        text.push_str(if n.has_contents() { n.contents() } else { " " });
                        c += 1;
                    }
                }
                let x = inner.left() + start as f32 * cw;
                let w = (c - start) as f32 * cw;
                if style.bg != theme::CODE_BG {
                    painter.rect_filled(egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, rh)), 0, style.bg);
                }
                if !text.trim().is_empty() {
                    // No bold monospace font is loaded, so bold is the same text drawn twice, half a pixel apart.
                    if style.bold {
                        painter.text(egui::pos2(x + 0.5, y), egui::Align2::LEFT_TOP, &text, font.clone(), style.fg);
                    }
                    painter.text(egui::pos2(x, y), egui::Align2::LEFT_TOP, text, font.clone(), style.fg);
                }
                if style.underline {
                    painter.hline(x..=x + w, y + rh - 1.5, Stroke::new(1.0, style.fg));
                }
            }
        }
        if self.scroll == 0 && !screen.hide_cursor() && !exited {
            let (cr, cc) = screen.cursor_position();
            let cur = egui::Rect::from_min_size(egui::pos2(inner.left() + cc as f32 * cw, inner.top() + cr as f32 * rh), egui::vec2(cw, rh));
            if focused {
                painter.rect_filled(cur, 1, theme::ACCENT.gamma_multiply(0.85));
                if let Some(cell) = screen.cell(cr, cc).filter(|c| c.has_contents()) {
                    painter.text(cur.min, egui::Align2::LEFT_TOP, cell.contents(), font.clone(), theme::ACCENT_INK);
                }
            } else {
                painter.rect_stroke(cur, 1, Stroke::new(1.0, theme::DIM), egui::StrokeKind::Inside);
            }
        }
        if self.scroll > 0 {
            let tag = egui::Rect::from_min_size(egui::pos2(rect.right() - 128.0, rect.top() + 8.0), egui::vec2(120.0, 22.0));
            painter.rect_filled(tag, 6, theme::SURFACE_HI);
            painter.text(tag.center(), egui::Align2::CENTER_CENTER, format!("{} lines up", self.scroll), FontId::proportional(11.5), theme::MUTED);
        }
        drop(parser);
        if exited {
            let bar = egui::Rect::from_min_size(egui::pos2(rect.left(), rect.bottom() - 34.0), egui::vec2(rect.width(), 34.0));
            painter.rect_filled(bar, 0, theme::SURFACE);
            painter.text(bar.left_center() + egui::vec2(PAD, 0.0), egui::Align2::LEFT_CENTER, "The shell exited. Click or press Enter to start a new one.", FontId::proportional(12.5), theme::MUTED);
            if resp.clicked() || (focused && ui.input(|i| i.key_pressed(Key::Enter))) {
                self.restart();
            }
        }
    }

}

/// Puts the selected text on the clipboard. False when nothing is selected.
fn copy_selection(selection: &mut Option<((u16, u16), (u16, u16))>, ui: &egui::Ui, screen: &vt100::Screen) -> bool {
    let Some((a, b)) = selection.map(|(a, b)| if a <= b { (a, b) } else { (b, a) }).filter(|(a, b)| a != b) else { return false };
    let text = screen.contents_between(a.0, a.1, b.0, b.1 + 1);
    ui.ctx().copy_text(text.lines().map(str::trim_end).collect::<Vec<_>>().join("\n"));
    *selection = None;
    true
}

#[derive(PartialEq, Clone, Copy)]
struct Style {
    fg: Color32,
    bg: Color32,
    bold: bool,
    underline: bool,
}

impl Style {
    fn of(cell: &vt100::Cell, selected: bool) -> Style {
        let mut fg = color(cell.fgcolor(), theme::TEXT, cell.bold());
        let mut bg = color(cell.bgcolor(), theme::CODE_BG, false);
        if cell.inverse() {
            std::mem::swap(&mut fg, &mut bg);
        }
        if cell.dim() {
            fg = fg.gamma_multiply(0.6);
        }
        if selected {
            bg = Color32::from_rgb(0x3a, 0x33, 0x22);
        }
        Style { fg, bg, bold: cell.bold(), underline: cell.underline() }
    }
}

/// The 16 ANSI colors, tuned to sit on the app's dark background.
const ANSI: [Color32; 16] = [
    Color32::from_rgb(0x2a, 0x2f, 0x38),
    Color32::from_rgb(0xec, 0x5f, 0x5f),
    Color32::from_rgb(0x4a, 0xc2, 0x7d),
    Color32::from_rgb(0xf5, 0xa5, 0x24),
    Color32::from_rgb(0x4d, 0x8f, 0xff),
    Color32::from_rgb(0xc0, 0x7c, 0xf0),
    Color32::from_rgb(0x3e, 0xc5, 0xd0),
    Color32::from_rgb(0xd4, 0xd8, 0xdf),
    Color32::from_rgb(0x76, 0x7f, 0x8c),
    Color32::from_rgb(0xff, 0x80, 0x80),
    Color32::from_rgb(0x72, 0xe0, 0x9e),
    Color32::from_rgb(0xff, 0xc8, 0x5a),
    Color32::from_rgb(0x7f, 0xae, 0xff),
    Color32::from_rgb(0xd8, 0xa2, 0xff),
    Color32::from_rgb(0x72, 0xe4, 0xec),
    Color32::from_rgb(0xf3, 0xf4, 0xf6),
];

fn color(c: vt100::Color, default: Color32, bright: bool) -> Color32 {
    match c {
        vt100::Color::Default => default,
        vt100::Color::Idx(i) if i < 8 && bright => ANSI[i as usize + 8],
        vt100::Color::Idx(i) if i < 16 => ANSI[i as usize],
        vt100::Color::Idx(i) if i < 232 => {
            let i = i - 16;
            let v = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
            Color32::from_rgb(v(i / 36), v(i / 6 % 6), v(i % 6))
        }
        vt100::Color::Idx(i) => {
            let g = 8 + (i - 232) * 10;
            Color32::from_rgb(g, g, g)
        }
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    }
}

/// What a key sends to the shell, the way xterm does it.
fn key_bytes(key: Key, m: egui::Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    let csi = |s: &str| Some(format!("\x1b[{s}").into_bytes());
    // xterm's modifier parameter: 1 + shift + 2*alt + 4*ctrl.
    let modp = 1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.ctrl as u8;
    let arrow = |c: char| {
        if modp > 1 {
            csi(&format!("1;{modp}{c}"))
        } else if app_cursor {
            Some(format!("\x1bO{c}").into_bytes())
        } else {
            csi(&c.to_string())
        }
    };
    let tilde = |n: u8| if modp > 1 { csi(&format!("{n};{modp}~")) } else { csi(&format!("{n}~")) };
    match key {
        Key::Enter => Some(b"\r".to_vec()),
        Key::Backspace => Some(if m.ctrl { b"\x08".to_vec() } else if m.alt { b"\x1b\x7f".to_vec() } else { b"\x7f".to_vec() }),
        Key::Tab => Some(if m.shift { b"\x1b[Z".to_vec() } else { b"\t".to_vec() }),
        Key::Escape => Some(b"\x1b".to_vec()),
        Key::ArrowUp => arrow('A'),
        Key::ArrowDown => arrow('B'),
        Key::ArrowRight => arrow('C'),
        Key::ArrowLeft => arrow('D'),
        Key::Home => arrow('H'),
        Key::End => arrow('F'),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F1 => Some(b"\x1bOP".to_vec()),
        Key::F2 => Some(b"\x1bOQ".to_vec()),
        Key::F3 => Some(b"\x1bOR".to_vec()),
        Key::F4 => Some(b"\x1bOS".to_vec()),
        Key::F5 => tilde(15),
        Key::F6 => tilde(17),
        Key::F7 => tilde(18),
        Key::F8 => tilde(19),
        Key::F9 => tilde(20),
        Key::F10 => tilde(21),
        Key::F11 => tilde(23),
        Key::F12 => tilde(24),
        Key::Space if m.ctrl => Some(vec![0]),
        // Ctrl+letter is the control character; egui sends no text for it.
        k if m.ctrl && !m.alt => {
            let name = k.name();
            let b = name.as_bytes();
            match b {
                [c] if c.is_ascii_alphabetic() => Some(vec![c.to_ascii_uppercase() - b'A' + 1]),
                _ => match k {
                    Key::OpenBracket => Some(vec![0x1b]),
                    Key::Backslash => Some(vec![0x1c]),
                    Key::CloseBracket => Some(vec![0x1d]),
                    _ => None,
                },
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dock::{key, test_frame};

    #[test]
    fn keys_map_like_xterm() {
        let none = egui::Modifiers::NONE;
        assert_eq!(key_bytes(Key::C, egui::Modifiers::CTRL, false), Some(vec![3]));
        assert_eq!(key_bytes(Key::ArrowUp, none, false), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_bytes(Key::ArrowUp, none, true), Some(b"\x1bOA".to_vec()));
        assert_eq!(key_bytes(Key::ArrowLeft, egui::Modifiers::CTRL, false), Some(b"\x1b[1;5D".to_vec()));
        assert_eq!(key_bytes(Key::Delete, none, false), Some(b"\x1b[3~".to_vec()));
        assert_eq!(key_bytes(Key::A, none, false), None);
    }

    /// Starts the real shell in a pseudo console, types a command and waits for its output.
    #[test]
    fn runs_a_command() {
        let ctx = egui::Context::default();
        let mut t = Terminal { focus: true, ..Default::default() };
        let cwd = env!("CARGO_MANIFEST_DIR");
        test_frame(&ctx, vec![], |ui| t.ui(ui, cwd));
        test_frame(&ctx, vec![], |ui| t.ui(ui, cwd));
        assert!(t.error.is_none(), "{:?}", t.error);
        assert!(ctx.memory(|m| m.has_focus(id())), "the terminal should take the keyboard");
        let screen = |t: &Terminal| t.session.as_ref().unwrap().parser.lock().unwrap().screen().contents();
        // The output differs from what's typed, so finding it means the shell ran it.
        let cmd = if cfg!(windows) { "echo ('scout-' + 6*7)" } else { "echo scout-$((6*7))" };
        test_frame(&ctx, vec![egui::Event::Text(cmd.into()), key(Key::Enter, egui::Modifiers::NONE)], |ui| t.ui(ui, cwd));
        let start = std::time::Instant::now();
        while !screen(&t).contains("scout-42") && start.elapsed().as_secs() < 20 {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        let shown = screen(&t);
        assert!(shown.contains("scout-42"), "screen was:\n{shown}");
        let (rows, cols) = t.session.as_ref().unwrap().size;
        assert!(rows > 20 && cols > 80, "sized to the area: {rows}x{cols}");
    }
}
