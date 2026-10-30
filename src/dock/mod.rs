//! The panel on the right, opened from the title bar: a terminal, the project's git changes, and a file browser
//! with an editor.

mod changes;
mod files;
mod highlight;
mod terminal;

use crate::theme;
use eframe::egui::{self, Color32, FontId, RichText, Sense};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Terminal,
    Changes,
    Files,
}

pub const TABS: [Tab; 3] = [Tab::Terminal, Tab::Changes, Tab::Files];

impl Tab {
    pub fn title(self) -> &'static str {
        match self {
            Tab::Terminal => "Terminal",
            Tab::Changes => "Changes",
            Tab::Files => "Files",
        }
    }

    pub fn icon(self) -> fn(&egui::Painter, egui::Rect, Color32) {
        match self {
            Tab::Terminal => theme::icon::terminal,
            Tab::Changes => theme::icon::diff,
            Tab::Files => theme::icon::folder,
        }
    }
}

pub struct Dock {
    pub open: bool,
    pub tab: Tab,
    terminal: terminal::Terminal,
    changes: changes::Changes,
    files: files::Files,
}

impl Dock {
    pub fn new() -> Self {
        Dock { open: false, tab: Tab::Terminal, terminal: terminal::Terminal::default(), changes: changes::Changes::default(), files: files::Files::default() }
    }

    /// Opens the panel on `tab`, or closes it when that tab is already showing.
    pub fn toggle(&mut self, tab: Tab) {
        if self.open && self.tab == tab {
            self.open = false;
        } else {
            self.open = true;
            self.select(tab);
        }
    }

    fn select(&mut self, tab: Tab) {
        if tab == Tab::Changes && self.tab != Tab::Changes {
            self.changes.stale = true;
        }
        self.tab = tab;
        if tab == Tab::Terminal {
            self.terminal.focus = true;
        }
    }

    /// True while the terminal or the editor has the keyboard, so the window's own shortcuts stay out of the way.
    pub fn has_keyboard(&self, ctx: &egui::Context) -> bool {
        self.open && ctx.memory(|m| m.focused().is_some_and(|id| id == terminal::id() || id == files::editor_id()))
    }

    pub fn has_unsaved_edits(&self) -> bool {
        self.files.has_unsaved_edits()
    }

    /// Scout (or /undo) touched files: refresh what's on screen.
    pub fn files_changed(&mut self) {
        self.changes.stale = true;
        self.files.disk_changed();
    }

    /// Opens a file in the editor, from anywhere in the app.
    pub fn open_file(&mut self, path: PathBuf) {
        self.open = true;
        self.tab = Tab::Files;
        self.files.request_open(path);
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cwd: &str) {
        let mut open_file = None;
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
        self.header(ui);
        let (sep, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter().rect_filled(sep, 0, theme::BORDER);
        match self.tab {
            Tab::Terminal => self.terminal.ui(ui, cwd),
            Tab::Changes => self.changes.ui(ui, cwd, &mut open_file),
            Tab::Files => self.files.ui(ui, cwd),
        }
        if let Some(p) = open_file {
            self.open_file(p);
        }
    }

    /// The tab strip, with the open tab's own actions on the right.
    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.set_height(38.0);
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(6.0);
            for t in TABS {
                if tab_button(ui, self.tab == t, t).clicked() {
                    self.select(t);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                if theme::icon_button(ui, 26.0, "Close panel", theme::icon::cross).clicked() {
                    self.open = false;
                }
                match self.tab {
                    Tab::Terminal => self.terminal.actions(ui),
                    Tab::Changes => self.changes.actions(ui),
                    Tab::Files => self.files.actions(ui),
                }
            });
        });
    }
}

fn tab_button(ui: &mut egui::Ui, selected: bool, tab: Tab) -> egui::Response {
    let g = ui.painter().layout_no_wrap(tab.title().to_string(), if selected { theme::bold(13.0) } else { FontId::proportional(13.0) }, theme::TEXT);
    let (rect, r) = ui.allocate_exact_size(egui::vec2(g.size().x + 40.0, 28.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 7, theme::SURFACE_HI);
    } else if r.hovered() {
        p.rect_filled(rect, 7, theme::HOVER);
    }
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let fg = if selected { theme::STRONG } else if r.hovered() { theme::TEXT } else { theme::MUTED };
    tab.icon()(p, egui::Rect::from_center_size(egui::pos2(rect.left() + 17.0, rect.center().y), egui::vec2(18.0, 18.0)), fg);
    p.galley_with_override_text_color(egui::pos2(rect.left() + 30.0, rect.center().y - g.size().y / 2.0), g, fg);
    r
}

/// A small text button for a tab's header.
fn small_button(ui: &mut egui::Ui, enabled: bool, text: &str) -> egui::Response {
    ui.add_enabled(enabled, egui::Button::new(RichText::new(text).size(12.0).color(theme::MUTED)).frame_when_inactive(false).corner_radius(6))
}

/// Centered grey text for an empty or failed view.
fn placeholder(ui: &mut egui::Ui, title: &str, detail: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.35).max(24.0));
        ui.label(RichText::new(title).font(theme::bold(14.0)).color(theme::MUTED));
        if !detail.is_empty() {
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(detail).size(12.5).color(theme::DIM)).wrap());
        }
    });
}

/// A 1px line across the ui.
fn rule(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, 0, theme::BORDER);
}

/// Runs one headless frame of `f` in a 1000×700 window with these input events.
#[cfg(test)]
pub(crate) fn test_frame(ctx: &egui::Context, events: Vec<egui::Event>, mut f: impl FnMut(&mut egui::Ui)) {
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0))), events, ..Default::default() };
    ctx.run_ui(input, |ui| f(ui)).textures_delta.clear();
}

#[cfg(test)]
pub(crate) fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}

/// A fresh empty folder for a test.
#[cfg(test)]
pub(crate) fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("scout-dock-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

