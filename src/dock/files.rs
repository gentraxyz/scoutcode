//! The project's files as a tree, and an editor for the one that's open. With room for both they sit side by
//! side; in a narrow panel the editor takes the tree's place and a back arrow returns to it.

use super::{highlight, placeholder, rule, small_button};
use crate::theme;
use eframe::egui::{self, Color32, FontId, Key, Modifiers, RichText, Sense};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

pub fn editor_id() -> egui::Id {
    egui::Id::unique("dock_editor")
}

/// The panel width from which the tree and the editor fit side by side.
const SPLIT_WIDTH: f32 = 620.0;
const MAX_EDIT_BYTES: u64 = 5_000_000;
const FONT_SIZE: f32 = 13.0;
/// Folders that are never worth browsing.
const HIDDEN: [&str; 1] = [".git"];

struct Node {
    name: String,
    path: PathBuf,
    dir: bool,
    open: bool,
    /// Read the first time the folder is opened.
    children: Option<Vec<Node>>,
}

fn read_dir(dir: &Path) -> Vec<Node> {
    let mut out: Vec<Node> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    if HIDDEN.contains(&name.as_str()) {
                        return None;
                    }
                    let dir = e.file_type().ok()?.is_dir();
                    Some(Node { name, path: e.path(), dir, open: false, children: None })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

/// Reads every folder that was read before again, keeping what was open.
fn refresh(dir: &Path, old: Vec<Node>) -> Vec<Node> {
    let mut old: std::collections::HashMap<String, Node> = old.into_iter().map(|n| (n.name.clone(), n)).collect();
    read_dir(dir)
        .into_iter()
        .map(|mut n| {
            if let Some(o) = old.remove(&n.name).filter(|o| o.dir && n.dir) {
                n.open = o.open;
                n.children = o.children.map(|c| refresh(&n.path, c));
            }
            n
        })
        .collect()
}

struct Editor {
    path: PathBuf,
    text: String,
    /// The text as it is on disk, to tell whether there's anything to save.
    saved: String,
    crlf: bool,
    mtime: Option<SystemTime>,
    ext: String,
    /// Why the file can't be edited here: binary, too big, unreadable.
    blocked: Option<String>,
    changed_on_disk: bool,
    error: Option<String>,
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

impl Editor {
    fn load(path: PathBuf) -> Editor {
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default());
        let mut ed = Editor { mtime: mtime(&path), path, text: String::new(), saved: String::new(), crlf: false, ext, blocked: None, changed_on_disk: false, error: None };
        ed.read();
        ed
    }

    fn read(&mut self) {
        self.blocked = None;
        self.changed_on_disk = false;
        self.mtime = mtime(&self.path);
        match std::fs::metadata(&self.path) {
            Ok(m) if m.len() > MAX_EDIT_BYTES => {
                self.blocked = Some(format!("Too large to edit here ({:.1} MB).", m.len() as f64 / 1e6));
                return;
            }
            Err(e) => {
                self.blocked = Some(e.to_string());
                return;
            }
            _ => {}
        }
        match std::fs::read(&self.path) {
            Ok(b) if b.iter().take(8000).any(|c| *c == 0) => self.blocked = Some("Binary file.".into()),
            Ok(b) => match String::from_utf8(b) {
                Ok(t) => {
                    self.crlf = t.contains("\r\n");
                    self.text = if self.crlf { t.replace("\r\n", "\n") } else { t };
                    self.saved = self.text.clone();
                }
                Err(_) => self.blocked = Some("Not UTF-8 text.".into()),
            },
            Err(e) => self.blocked = Some(e.to_string()),
        }
    }

    fn dirty(&self) -> bool {
        self.blocked.is_none() && self.text != self.saved
    }

    fn save(&mut self) {
        let out = if self.crlf { self.text.replace('\n', "\r\n") } else { self.text.clone() };
        match std::fs::write(&self.path, out) {
            Ok(()) => {
                self.saved = self.text.clone();
                self.mtime = mtime(&self.path);
                self.changed_on_disk = false;
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Could not save: {e}")),
        }
    }
}

/// What to do once the user has decided about unsaved changes.
enum Pending {
    Close,
    Open(PathBuf),
}

#[derive(Default)]
pub struct Files {
    root: String,
    tree: Option<Vec<Node>>,
    stale: bool,
    editor: Option<Editor>,
    pending: Option<Pending>,
    /// In a narrow panel with a file open: showing the tree instead of the editor.
    show_tree: bool,
    request: Option<PathBuf>,
    last_check: f64,
}

impl Files {
    pub fn request_open(&mut self, path: PathBuf) {
        self.request = Some(path);
    }

    /// Files may have changed under us: re-read the tree, and the open file if it has no unsaved edits.
    pub fn disk_changed(&mut self) {
        self.stale = true;
        self.check_editor();
    }

    fn check_editor(&mut self) {
        let Some(ed) = &mut self.editor else { return };
        if ed.blocked.is_some() || mtime(&ed.path) != ed.mtime {
            if ed.dirty() {
                ed.changed_on_disk = true;
                ed.mtime = mtime(&ed.path);
            } else {
                ed.read();
            }
        }
    }

    fn open(&mut self, path: PathBuf) {
        self.show_tree = false;
        if self.editor.as_ref().is_some_and(|e| e.path == path) {
            return;
        }
        if self.editor.as_ref().is_some_and(Editor::dirty) {
            self.pending = Some(Pending::Open(path));
        } else {
            self.editor = Some(Editor::load(path));
        }
    }

    fn close(&mut self) {
        if self.editor.as_ref().is_some_and(Editor::dirty) {
            self.pending = Some(Pending::Close);
        } else {
            self.editor = None;
        }
    }

    /// Carries out what was waiting on unsaved changes, after saving them or not.
    fn resolve(&mut self, save: bool) {
        if save {
            if let Some(ed) = &mut self.editor {
                ed.save();
                if ed.error.is_some() {
                    self.pending = None;
                    return;
                }
            }
        }
        match self.pending.take() {
            Some(Pending::Close) => self.editor = None,
            Some(Pending::Open(p)) => self.editor = Some(Editor::load(p)),
            None => {}
        }
    }

    pub fn actions(&mut self, ui: &mut egui::Ui) {
        if theme::icon_button(ui, 26.0, "Refresh", theme::icon::refresh).clicked() {
            self.stale = true;
            self.check_editor();
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cwd: &str) {
        if self.root != cwd {
            self.root = cwd.to_string();
            self.tree = None;
        }
        if let Some(p) = self.request.take() {
            self.open(p);
        }
        if self.stale {
            self.stale = false;
            if let Some(t) = self.tree.take() {
                self.tree = Some(refresh(Path::new(&self.root), t));
            }
        }
        if self.tree.is_none() {
            self.tree = Some(read_dir(Path::new(&self.root)));
        }
        // Notice edits made outside the app (or by Scout) while the file is open.
        let now = ui.input(|i| i.time);
        if now - self.last_check > 1.5 {
            self.last_check = now;
            self.check_editor();
        }
        if self.editor.is_some() && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S)) {
            if let Some(ed) = self.editor.as_mut().filter(|e| e.blocked.is_none()) {
                ed.save();
            }
        }

        let wide = ui.available_width() >= SPLIT_WIDTH;
        if self.editor.is_none() {
            self.tree_ui(ui);
        } else if wide {
            let max = (ui.available_width() * 0.5).max(180.0);
            egui::Panel::left("dock_tree")
                .resizable(true)
                .default_size(240.0)
                .size_range(160.0..=max)
                .frame(egui::Frame::NONE.fill(theme::SIDEBAR))
                .show(ui, |ui| self.tree_ui(ui));
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(theme::CODE_BG)).show(ui, |ui| self.editor_ui(ui, false));
        } else if self.show_tree {
            self.tree_ui(ui);
        } else {
            self.editor_ui(ui, true);
        }
    }

    fn tree_ui(&mut self, ui: &mut egui::Ui) {
        let selected = self.editor.as_ref().map(|e| e.path.clone());
        let mut clicked = None;
        let root = PathBuf::from(&self.root);
        let Some(tree) = &mut self.tree else { return };
        if tree.is_empty() {
            return placeholder(ui, "Empty folder", "");
        }
        egui::ScrollArea::both().id_salt("dock_tree_scroll").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(6.0);
            nodes_ui(ui, tree, 0, selected.as_deref(), &root, &mut clicked);
            ui.add_space(8.0);
        });
        if let Some(p) = clicked {
            self.open(p);
        }
    }

    fn editor_ui(&mut self, ui: &mut egui::Ui, narrow: bool) {
        let root = self.root.clone();
        let Some(ed) = &mut self.editor else { return };
        let mut close = false;
        let mut back = false;
        let rel = ed.path.strip_prefix(&root).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_else(|_| ed.path.to_string_lossy().to_string());
        let dirty = ed.dirty();
        egui::Frame::NONE.fill(theme::BG).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_height(34.0);
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.add_space(6.0);
                if narrow && theme::icon_button(ui, 26.0, "Back to files", theme::icon::arrow_left).clicked() {
                    back = true;
                }
                let (dir, name) = rel.rsplit_once('/').map_or(("", rel.as_str()), |(d, n)| (d, n));
                ui.label(RichText::new(name).font(theme::bold(13.0)).color(theme::STRONG)).on_hover_text(ed.path.to_string_lossy());
                if dirty {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 3.5, theme::ACCENT);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
                    if theme::icon_button(ui, 26.0, "Close file", theme::icon::cross).clicked() {
                        close = true;
                    }
                    if small_button(ui, dirty, "Save").on_hover_text("Ctrl+S").clicked() {
                        ed.save();
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        if !dir.is_empty() {
                            ui.add(egui::Label::new(RichText::new(dir).size(12.0).color(theme::DIM)).truncate());
                        }
                    });
                });
            });
        });
        rule(ui);
        if let Some(p) = &self.pending {
            let name = ed.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let mut choice = None;
            banner(ui, theme::ACCENT, &format!("Save changes to {name}{}?", if matches!(p, Pending::Open(_)) { " before opening another file" } else { "" }), |ui| {
                if small_button(ui, true, "Cancel").clicked() {
                    choice = Some(None);
                }
                if small_button(ui, true, "Don't save").clicked() {
                    choice = Some(Some(false));
                }
                if theme::primary_button(ui, "Save").clicked() {
                    choice = Some(Some(true));
                }
            });
            match choice {
                Some(None) => self.pending = None,
                Some(Some(save)) => return self.resolve(save),
                None => {}
            }
        }
        let Some(ed) = &mut self.editor else { return };
        if ed.changed_on_disk {
            let mut reload = false;
            banner(ui, theme::ACCENT, "Changed on disk since you started editing.", |ui| {
                if small_button(ui, true, "Keep mine").clicked() {
                    ed.changed_on_disk = false;
                }
                if small_button(ui, true, "Reload").clicked() {
                    reload = true;
                }
            });
            if reload {
                ed.read();
            }
        }
        if let Some(e) = ed.error.clone() {
            banner(ui, theme::RED, &e, |ui| {
                if small_button(ui, true, "Dismiss").clicked() {
                    ed.error = None;
                }
            });
        }
        if let Some(b) = &ed.blocked {
            placeholder(ui, "Can't open this file here", b);
        } else {
            code_editor(ui, ed);
        }
        if back {
            self.show_tree = true;
        }
        if close {
            self.close();
        }
    }
}

/// A strip across the editor with a message and buttons on the right.
fn banner(ui: &mut egui::Ui, color: Color32, text: &str, buttons: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE.fill(theme::SURFACE).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 18.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 3.5, color);
            ui.add(egui::Label::new(RichText::new(text).size(12.5).color(theme::TEXT)).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                buttons(ui);
            });
        });
    });
    rule(ui);
}

fn code_editor(ui: &mut egui::Ui, ed: &mut Editor) {
    let font = FontId::monospace(FONT_SIZE);
    let (cw, rh) = ui.fonts_mut(|f| (f.glyph_width(&font, '0'), f.row_height(&font)));
    let lines = ed.text.split('\n').count();
    let gutter_w = (lines.to_string().len().max(3) as f32) * cw + 22.0;
    let view_h = ui.available_height();
    let ext = ed.ext.clone();
    // The line the cursor is on, from last frame, to light up its number.
    let current = egui::TextEdit::load_state(ui.ctx(), editor_id())
        .and_then(|s| s.cursor.char_range())
        .map(|r| ed.text.chars().take(r.primary.index.0).filter(|c| *c == '\n').count());
    egui::ScrollArea::both().id_salt("dock_editor_scroll").auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let (gutter, _) = ui.allocate_exact_size(egui::vec2(gutter_w, (lines as f32 * rh).max(view_h)), Sense::hover());
            let first = ((viewport.top() / rh).floor().max(0.0)) as usize;
            let last = (((viewport.bottom() / rh).ceil()) as usize + 1).min(lines);
            let p = ui.painter();
            p.rect_filled(gutter.with_max_y(gutter.top() + viewport.bottom() + rh), 0, theme::CODE_BG);
            for n in first..last {
                let color = if Some(n) == current { theme::MUTED } else { theme::FAINT };
                p.text(egui::pos2(gutter.right() - 12.0, gutter.top() + n as f32 * rh), egui::Align2::RIGHT_TOP, (n + 1).to_string(), font.clone(), color);
            }
            let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, _wrap: f32| -> Arc<egui::Galley> {
                let mut job = highlight::job(buf.as_str(), &ext, &font);
                job.wrap.max_width = f32::INFINITY;
                ui.fonts_mut(|f| f.layout_job(job))
            };
            ui.add(
                egui::TextEdit::multiline(&mut ed.text)
                    .id(editor_id())
                    .code_editor()
                    .frame(egui::Frame::NONE)
                    .margin(egui::Margin { left: 4, right: 24, top: 0, bottom: 0 })
                    .desired_width(f32::INFINITY)
                    .desired_rows(((view_h / rh) as usize).max(1))
                    .layouter(&mut layouter),
            );
        });
    });
}

fn nodes_ui(ui: &mut egui::Ui, nodes: &mut [Node], depth: usize, selected: Option<&Path>, root: &Path, clicked: &mut Option<PathBuf>) {
    for n in nodes.iter_mut() {
        let here = selected == Some(n.path.as_path());
        let r = node_row(ui, n, depth, here);
        if r.clicked() {
            if n.dir {
                n.open = !n.open;
                if n.open && n.children.is_none() {
                    n.children = Some(read_dir(&n.path));
                }
            } else {
                *clicked = Some(n.path.clone());
            }
        }
        r.context_menu(|ui| {
            if ui.button("Copy path").clicked() {
                ui.ctx().copy_text(n.path.to_string_lossy().to_string());
            }
            if ui.button("Copy relative path").clicked() {
                ui.ctx().copy_text(n.path.strip_prefix(root).unwrap_or(&n.path).to_string_lossy().replace('\\', "/"));
            }
        });
        if n.dir && n.open {
            if let Some(c) = &mut n.children {
                nodes_ui(ui, c, depth + 1, selected, root, clicked);
            }
        }
    }
}

fn node_row(ui: &mut egui::Ui, n: &Node, depth: usize, selected: bool) -> egui::Response {
    let w = ui.available_width().max(180.0);
    let (rect, r) = ui.allocate_exact_size(egui::vec2(w, 26.0), Sense::click());
    let p = ui.painter();
    let bg = rect.shrink2(egui::vec2(6.0, 1.0));
    if selected {
        p.rect_filled(bg, 6, theme::SURFACE_HI);
    } else if r.hovered() {
        p.rect_filled(bg, 6, theme::HOVER);
    }
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let x = rect.left() + 12.0 + depth as f32 * 14.0;
    let icon = |dx: f32| egui::Rect::from_center_size(egui::pos2(x + dx, rect.center().y), egui::vec2(16.0, 16.0));
    if n.dir {
        if n.open { theme::icon::chevron_down(p, icon(6.0), theme::DIM) } else { theme::icon::chevron_right(p, icon(6.0), theme::DIM) }
        theme::icon::folder(p, icon(22.0), if n.open { theme::ACCENT } else { theme::MUTED });
    } else {
        theme::icon::file(p, icon(22.0), if selected { theme::STRONG } else { theme::DIM });
    }
    let color = if selected { theme::STRONG } else if n.name.starts_with('.') { theme::DIM } else { theme::TEXT };
    let g = one_line(ui, &n.name, FontId::proportional(13.0), color, rect.right() - x - 40.0);
    ui.painter().galley(egui::pos2(x + 34.0, rect.center().y - g.size().y / 2.0), g, color);
    r
}

/// One line of text, cut with an ellipsis to fit `max` points.
pub fn one_line(ui: &egui::Ui, text: &str, font: FontId, color: Color32, max: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(text.to_string(), egui::TextFormat { font_id: font, color, ..Default::default() });
    job.wrap = egui::text::TextWrapping::truncate_at_width(max.max(10.0));
    ui.painter().layout_job(job)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dock::{key, temp_dir, test_frame};

    #[test]
    fn browses_opens_edits_and_saves() {
        let dir = temp_dir("files");
        std::fs::create_dir(dir.join("src")).unwrap();
        std::fs::write(dir.join("src").join("main.rs"), "fn main() {}\r\n").unwrap();
        std::fs::write(dir.join("README.md"), "hi").unwrap();
        std::fs::create_dir(dir.join(".git")).unwrap();
        let root = dir.to_string_lossy().to_string();
        let ctx = egui::Context::default();
        let mut f = Files::default();
        test_frame(&ctx, vec![], |ui| f.ui(ui, &root));
        let names: Vec<&str> = f.tree.as_ref().unwrap().iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["src", "README.md"], "folders first, .git hidden");

        let main = dir.join("src").join("main.rs");
        f.request_open(main.clone());
        test_frame(&ctx, vec![], |ui| f.ui(ui, &root));
        let ed = f.editor.as_mut().unwrap();
        assert_eq!(ed.text, "fn main() {}\n");
        assert!(ed.crlf);
        ed.text.push_str("// more\n");
        test_frame(&ctx, vec![key(Key::S, Modifiers::COMMAND)], |ui| f.ui(ui, &root));
        assert_eq!(std::fs::read_to_string(&main).unwrap(), "fn main() {}\r\n// more\r\n", "saved with its line endings");
        assert!(!f.editor.as_ref().unwrap().dirty());

        // Opening another file with unsaved edits asks first; "don't save" drops them.
        f.editor.as_mut().unwrap().text.push_str("unsaved");
        f.request_open(dir.join("README.md"));
        test_frame(&ctx, vec![], |ui| f.ui(ui, &root));
        assert!(matches!(f.pending, Some(Pending::Open(_))));
        f.resolve(false);
        assert_eq!(f.editor.as_ref().unwrap().text, "hi");

        // A change on disk reloads a clean file, and only flags a dirty one.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join("README.md"), "changed").unwrap();
        f.disk_changed();
        assert_eq!(f.editor.as_ref().unwrap().text, "changed");
        f.editor.as_mut().unwrap().text.push('!');
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join("README.md"), "again").unwrap();
        f.disk_changed();
        let ed = f.editor.as_ref().unwrap();
        assert!(ed.changed_on_disk && ed.text == "changed!");

        // New files show up on refresh, and open folders stay open.
        f.tree.as_mut().unwrap()[0].open = true;
        f.tree.as_mut().unwrap()[0].children = Some(read_dir(&dir.join("src")));
        std::fs::write(dir.join("src").join("lib.rs"), "").unwrap();
        f.disk_changed();
        test_frame(&ctx, vec![], |ui| f.ui(ui, &root));
        let src = &f.tree.as_ref().unwrap()[0];
        assert!(src.open);
        assert_eq!(src.children.as_ref().unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_binary_files() {
        let dir = temp_dir("binary");
        std::fs::write(dir.join("x.bin"), [0u8, 1, 2]).unwrap();
        let ed = Editor::load(dir.join("x.bin"));
        assert!(ed.blocked.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
