//! What changed in the project's git repository since the last commit, file by file, with each diff a click away.

use super::{placeholder, rule};
use crate::diff::{self, DiffLine};
use crate::theme;
use eframe::egui::{self, Color32, FontId, RichText, Sense};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

/// Diffs of files bigger than this aren't worked out.
const MAX_DIFF_BYTES: u64 = 2_000_000;

struct Change {
    /// Relative to the repository root, with forward slashes.
    path: String,
    /// Where a renamed file came from.
    from: Option<String>,
    status: char,
    counts: Option<(usize, usize)>,
    open: bool,
    diff: Option<Result<(Vec<DiffLine>, bool), String>>,
}

type Loaded = Result<(String, Vec<Change>), String>;

#[derive(Default)]
pub struct Changes {
    pub stale: bool,
    /// The folder the list was loaded for.
    cwd: String,
    repo: String,
    files: Vec<Change>,
    loading: Option<Receiver<Loaded>>,
    error: Option<String>,
}

/// Runs git and returns its raw output (prompt::git trims, which would eat porcelain's leading spaces).
fn git(cwd: &str, args: &[&str]) -> Option<Vec<u8>> {
    let mut c = std::process::Command::new("git");
    c.args(args).current_dir(cwd);
    crate::tools::no_window(&mut c);
    let out = c.output().ok()?;
    out.status.success().then_some(out.stdout)
}

fn load(cwd: &str) -> Loaded {
    let top = git(cwd, &["rev-parse", "--show-toplevel"]).map(|o| String::from_utf8_lossy(&o).trim().to_string()).ok_or("This folder isn't in a git repository.")?;
    let status = git(&top, &["status", "--porcelain=v1", "-z", "--untracked-files=all"]).ok_or("git status failed.")?;
    let status = String::from_utf8_lossy(&status).to_string();
    let mut files = vec![];
    let mut parts = status.split('\0').filter(|p| !p.is_empty());
    while let Some(e) = parts.next() {
        if e.len() < 4 {
            continue;
        }
        let (x, y) = (e.as_bytes()[0] as char, e.as_bytes()[1] as char);
        let from = if matches!(x, 'R' | 'C') { parts.next().map(String::from) } else { None };
        let status = match (x, y) {
            ('?', _) => 'U',
            ('R', _) => 'R',
            ('A', _) => 'A',
            (_, 'D') | ('D', _) => 'D',
            _ => 'M',
        };
        files.push(Change { path: e[3..].to_string(), from, status, counts: None, open: false, diff: None });
    }
    // Line counts for tracked files in one go; renames show as "a => b", which simply won't match.
    if let Some(ns) = git(&top, &["diff", "HEAD", "--numstat"]) {
        for line in String::from_utf8_lossy(&ns).lines() {
            let mut it = line.splitn(3, '\t');
            let (Some(a), Some(d), Some(p)) = (it.next(), it.next(), it.next()) else { continue };
            if let (Ok(a), Ok(d), Some(f)) = (a.parse(), d.parse(), files.iter_mut().find(|f| f.path == p)) {
                f.counts = Some((a, d));
            }
        }
    }
    for f in files.iter_mut().filter(|f| f.status == 'U') {
        let p = Path::new(&top).join(&f.path);
        if std::fs::metadata(&p).is_ok_and(|m| m.len() < MAX_DIFF_BYTES) {
            if let Ok(t) = std::fs::read_to_string(&p) {
                f.counts = Some((t.lines().count(), 0));
            }
        }
    }
    files.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));
    Ok((top, files))
}

fn text_of(bytes: Vec<u8>) -> Result<String, String> {
    if bytes.len() as u64 > MAX_DIFF_BYTES {
        return Err("Too large to show.".into());
    }
    if bytes.iter().take(8000).any(|b| *b == 0) {
        return Err("Binary file.".into());
    }
    Ok(String::from_utf8_lossy(&bytes).replace("\r\n", "\n"))
}

/// The file at the last commit against the one on disk.
fn file_diff(repo: &str, c: &Change) -> Result<(Vec<DiffLine>, bool), String> {
    let created = matches!(c.status, 'U' | 'A');
    let before = if created { String::new() } else { text_of(git(repo, &["show", &format!("HEAD:{}", c.from.as_ref().unwrap_or(&c.path))]).unwrap_or_default())? };
    let after = if c.status == 'D' { String::new() } else { text_of(std::fs::read(Path::new(repo).join(&c.path)).map_err(|e| e.to_string())?)? };
    // The final newline would otherwise show up as an extra empty line.
    let trim = |s: &str| s.strip_suffix('\n').unwrap_or(s).to_string();
    Ok((diff::diff_lines(&trim(&before), &trim(&after)), created))
}

fn status_color(s: char) -> Color32 {
    match s {
        'A' | 'U' => theme::GREEN,
        'D' => theme::RED,
        'R' => theme::USAGE,
        _ => theme::ACCENT,
    }
}

impl Changes {
    pub fn actions(&mut self, ui: &mut egui::Ui) {
        if self.loading.is_some() {
            ui.add_space(6.0);
            ui.add(egui::Spinner::new().size(13.0).color(theme::DIM));
        } else if theme::icon_button(ui, 26.0, "Refresh", theme::icon::refresh).clicked() {
            self.stale = true;
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, cwd: &str, open_file: &mut Option<PathBuf>) {
        if self.cwd != cwd {
            self.cwd = cwd.to_string();
            self.files.clear();
            self.stale = true;
        }
        if let Some(rx) = &self.loading {
            if let Ok(r) = rx.try_recv() {
                self.loading = None;
                match r {
                    Ok((repo, files)) => {
                        // Keep open diffs open across a refresh.
                        let open: Vec<String> = self.files.iter().filter(|f| f.open).map(|f| f.path.clone()).collect();
                        self.files = files;
                        for f in self.files.iter_mut() {
                            f.open = open.contains(&f.path);
                        }
                        self.repo = repo;
                        self.error = None;
                    }
                    Err(e) => {
                        self.files.clear();
                        self.error = Some(e);
                    }
                }
            }
        }
        if self.stale && self.loading.is_none() {
            self.stale = false;
            let (tx, rx) = channel();
            let (cwd, ctx) = (cwd.to_string(), ui.ctx().clone());
            std::thread::spawn(move || {
                let _ = tx.send(load(&cwd));
                ctx.request_repaint();
            });
            self.loading = Some(rx);
        }

        if let Some(e) = &self.error {
            return placeholder(ui, "No changes to show", e);
        }
        if self.files.is_empty() {
            if self.loading.is_none() {
                placeholder(ui, "No changes", "The working tree matches the last commit.");
            }
            return;
        }
        let (add, del) = self.files.iter().filter_map(|f| f.counts).fold((0, 0), |(a, d), (x, y)| (a + x, d + y));
        ui.horizontal(|ui| {
            ui.set_height(34.0);
            ui.add_space(14.0);
            let n = self.files.len();
            ui.label(RichText::new(format!("{n} file{} changed", if n == 1 { "" } else { "s" })).size(12.5).color(theme::MUTED));
            ui.add_space(8.0);
            ui.label(RichText::new(format!("+{add}")).font(FontId::monospace(12.0)).color(theme::GREEN));
            ui.add_space(6.0);
            ui.label(RichText::new(format!("-{del}")).font(FontId::monospace(12.0)).color(theme::RED));
        });
        rule(ui);
        let repo = self.repo.clone();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(4.0);
            for f in self.files.iter_mut() {
                let r = change_row(ui, f).on_hover_text(format!("{}\nDouble-click to open", f.path));
                if r.double_clicked() && f.status != 'D' {
                    *open_file = Some(Path::new(&repo).join(&f.path));
                } else if r.clicked() {
                    f.open = !f.open;
                }
                r.context_menu(|ui| {
                    if f.status != 'D' && ui.button("Open file").clicked() {
                        *open_file = Some(Path::new(&repo).join(&f.path));
                    }
                    if ui.button("Copy path").clicked() {
                        ui.ctx().copy_text(f.path.clone());
                    }
                });
                if f.open {
                    if f.diff.is_none() {
                        f.diff = Some(file_diff(&repo, f));
                    }
                    let d = f.diff.as_ref().unwrap();
                    egui::Frame::NONE.inner_margin(egui::Margin { left: 10, right: 10, top: 2, bottom: 10 }).show(ui, |ui| match d {
                        Ok((lines, created)) if lines.iter().any(|l| l.kind != diff::Kind::Same) || *created => crate::app::diff_rows(ui, lines, *created),
                        Ok(_) => {
                            ui.label(RichText::new("No line changes (only mode or line endings).").size(12.5).color(theme::DIM));
                        }
                        Err(e) => {
                            ui.label(RichText::new(e.as_str()).size(12.5).color(theme::DIM));
                        }
                    });
                }
            }
            ui.add_space(8.0);
        });
    }
}

fn change_row(ui: &mut egui::Ui, f: &Change) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), Sense::click());
    let p = ui.painter();
    if r.hovered() {
        p.rect_filled(rect.shrink2(egui::vec2(6.0, 0.0)), 7, theme::HOVER);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let c = egui::Rect::from_center_size(egui::pos2(rect.left() + 20.0, rect.center().y), egui::vec2(16.0, 16.0));
    if f.open { theme::icon::chevron_down(p, c, theme::DIM) } else { theme::icon::chevron_right(p, c, theme::DIM) }
    p.text(egui::pos2(rect.left() + 38.0, rect.center().y), egui::Align2::CENTER_CENTER, f.status.to_string(), theme::bold(12.0), status_color(f.status));
    let counts = f.counts.map(|(a, d)| (p.layout_no_wrap(format!("+{a}"), FontId::monospace(11.5), theme::GREEN), p.layout_no_wrap(format!("-{d}"), FontId::monospace(11.5), theme::RED)));
    let counts_w = counts.as_ref().map_or(0.0, |(a, d)| a.size().x + d.size().x + 8.0);
    let (dir, name) = f.path.rsplit_once('/').map_or(("", f.path.as_str()), |(d, n)| (d, n));
    let x = rect.left() + 52.0;
    let max = rect.right() - 16.0 - counts_w - x;
    let name_g = super::files::one_line(ui, name, FontId::proportional(13.0), if f.status == 'D' { theme::DIM } else { theme::TEXT }, max);
    let name_w = name_g.size().x;
    ui.painter().galley(egui::pos2(x, rect.center().y - name_g.size().y / 2.0), name_g, theme::TEXT);
    if !dir.is_empty() && max - name_w > 40.0 {
        let dir_g = super::files::one_line(ui, dir, FontId::proportional(12.0), theme::DIM, max - name_w - 8.0);
        ui.painter().galley(egui::pos2(x + name_w + 8.0, rect.center().y - dir_g.size().y / 2.0), dir_g, theme::DIM);
    }
    if let Some((a, d)) = counts {
        let right = rect.right() - 16.0;
        let (aw, dw) = (a.size().x, d.size().x);
        ui.painter().galley(egui::pos2(right - dw, rect.center().y - d.size().y / 2.0), d, theme::RED);
        ui.painter().galley(egui::pos2(right - dw - 6.0 - aw, rect.center().y - a.size().y / 2.0), a, theme::GREEN);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dock::temp_dir;

    #[test]
    fn lists_changes_against_head() {
        let dir = temp_dir("changes");
        let root = dir.to_string_lossy().to_string();
        let run = |args: &[&str]| assert!(git(&root, args).is_some(), "git {args:?}");
        run(&["init", "-q"]);
        run(&["config", "user.email", "t@example.com"]);
        run(&["config", "user.name", "t"]);
        run(&["config", "core.autocrlf", "false"]);
        std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(dir.join("gone.txt"), "bye\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "init"]);
        std::fs::write(dir.join("a.txt"), "one\n2\nthree\nfour\n").unwrap();
        std::fs::remove_file(dir.join("gone.txt")).unwrap();
        std::fs::create_dir(dir.join("new dir")).unwrap();
        std::fs::write(dir.join("new dir").join("b.txt"), "x\ny\n").unwrap();

        let (repo, files) = load(&root).unwrap();
        let got: Vec<(&str, char, Option<(usize, usize)>)> = files.iter().map(|f| (f.path.as_str(), f.status, f.counts)).collect();
        assert_eq!(got, [("a.txt", 'M', Some((2, 1))), ("gone.txt", 'D', Some((0, 1))), ("new dir/b.txt", 'U', Some((2, 0)))]);
        let (lines, created) = file_diff(&repo, &files[0]).unwrap();
        assert!(!created);
        assert_eq!(diff::stats(&lines), (2, 1));
        let (lines, created) = file_diff(&repo, &files[2]).unwrap();
        assert!(created && diff::stats(&lines) == (2, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
