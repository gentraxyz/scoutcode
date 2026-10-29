//! Colors from the scoutcode.dev site, and fonts from the system when it has them.

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow, Stroke, Visuals};
use std::sync::Arc;

/// The transcript and composer.
pub const BG: Color32 = Color32::from_rgb(0x0e, 0x10, 0x14);
/// The sidebar, a step darker than the transcript.
pub const SIDEBAR: Color32 = Color32::from_rgb(0x0a, 0x0b, 0x0e);
pub const SURFACE: Color32 = Color32::from_rgb(0x15, 0x18, 0x1e);
pub const SURFACE_HI: Color32 = Color32::from_rgb(0x1c, 0x20, 0x27);
pub const HOVER: Color32 = Color32::from_rgb(0x19, 0x1c, 0x23);
pub const BORDER: Color32 = Color32::from_rgb(0x23, 0x27, 0x30);
pub const BORDER_HI: Color32 = Color32::from_rgb(0x30, 0x36, 0x41);
pub const TEXT: Color32 = Color32::from_rgb(0xd4, 0xd8, 0xdf);
pub const STRONG: Color32 = Color32::from_rgb(0xf3, 0xf4, 0xf6);
pub const MUTED: Color32 = Color32::from_rgb(0xa4, 0xab, 0xb6);
pub const DIM: Color32 = Color32::from_rgb(0x76, 0x7f, 0x8c);
pub const FAINT: Color32 = Color32::from_rgb(0x4d, 0x54, 0x5f);
pub const ACCENT: Color32 = Color32::from_rgb(0xf5, 0xa5, 0x24);
pub const ACCENT_HI: Color32 = Color32::from_rgb(0xff, 0xb8, 0x45);
pub const ACCENT_INK: Color32 = Color32::from_rgb(0x1a, 0x12, 0x03);
pub const USAGE: Color32 = Color32::from_rgb(0x4d, 0x8f, 0xff);
pub const CODE: Color32 = Color32::from_rgb(0xf0, 0xc0, 0x74);
pub const CODE_BG: Color32 = Color32::from_rgb(0x11, 0x13, 0x18);
pub const GREEN: Color32 = Color32::from_rgb(0x4a, 0xc2, 0x7d);
pub const RED: Color32 = Color32::from_rgb(0xec, 0x5f, 0x5f);
pub const ADD_BG: Color32 = Color32::from_rgb(0x10, 0x26, 0x1a);
pub const DEL_BG: Color32 = Color32::from_rgb(0x2e, 0x13, 0x15);
/// A row under the pointer in a dropdown, a step above the popup's own fill.
pub const MENU_HOVER: Color32 = Color32::from_rgb(0x22, 0x26, 0x2f);
/// The selected row in a dropdown: a faint wash of the accent.
pub const MENU_SELECTED: Color32 = Color32::from_rgba_premultiplied(0x0f, 0x0a, 0x02, 0x10);
pub const USER_BG: Color32 = Color32::from_rgb(0x1b, 0x1f, 0x27);

pub const BODY: f32 = 14.5;

const BOLD: &str = "bold";

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

fn shadow(blur: u8, alpha: u8) -> Shadow {
    Shadow { offset: [0, 6], blur, spread: 0, color: Color32::from_black_alpha(alpha) }
}

/// Segoe UI and Cascadia/Consolas on Windows, egui's built-in fonts elsewhere or as fallback.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let dir = std::path::PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into())).join("Fonts");
    let load = |names: &[&str]| names.iter().find_map(|n| std::fs::read(dir.join(n)).ok());
    fn add(fonts: &mut FontDefinitions, name: &str, family: FontFamily, bytes: Option<Vec<u8>>) {
        if let Some(b) = bytes {
            fonts.font_data.insert(name.into(), Arc::new(FontData::from_owned(b)));
            fonts.families.entry(family).or_default().insert(0, name.into());
        }
    }
    if cfg!(windows) {
        add(&mut fonts, "ui", FontFamily::Proportional, load(&["segoeui.ttf"]));
        add(&mut fonts, "mono", FontFamily::Monospace, load(&["CascadiaMono.ttf", "consola.ttf"]));
    }
    let proportional = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    fonts.families.insert(FontFamily::Name(BOLD.into()), proportional);
    if cfg!(windows) {
        add(&mut fonts, "ui-bold", FontFamily::Name(BOLD.into()), load(&["seguisb.ttf", "segoeuib.ttf"]));
        // Arrows, dots and checkmarks that Segoe UI lacks; after the main font so it only fills gaps.
        if let Some(b) = load(&["seguisym.ttf"]) {
            fonts.font_data.insert("symbols".into(), Arc::new(FontData::from_owned(b)));
            for fam in [FontFamily::Proportional, FontFamily::Name(BOLD.into()), FontFamily::Monospace] {
                let list = fonts.families.entry(fam).or_default();
                list.insert(1.min(list.len()), "symbols".into());
            }
        }
    }
    ctx.set_fonts(fonts);

    let mut v = Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.extreme_bg_color = CODE_BG;
    v.faint_bg_color = SURFACE;
    v.code_bg_color = CODE_BG;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(0xf5, 0xa5, 0x24, 48);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.window_stroke = Stroke::new(1.0, BORDER_HI);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(12);
    v.window_shadow = shadow(24, 120);
    v.popup_shadow = Shadow { offset: [0, 10], blur: 32, spread: 2, color: Color32::from_black_alpha(150) };
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.weak_bg_fill = SURFACE_HI;
    v.widgets.inactive.bg_fill = SURFACE_HI;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x23, 0x28, 0x31);
    v.widgets.hovered.bg_fill = Color32::from_rgb(0x23, 0x28, 0x31);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, BORDER_HI);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, STRONG);
    v.widgets.hovered.expansion = 0.0;
    v.widgets.active.weak_bg_fill = Color32::from_rgb(0x2a, 0x30, 0x3a);
    v.widgets.active.bg_fill = Color32::from_rgb(0x2a, 0x30, 0x3a);
    v.widgets.active.bg_stroke = Stroke::new(1.0, BORDER_HI);
    v.widgets.active.expansion = 0.0;
    v.widgets.open.weak_bg_fill = SURFACE_HI;
    v.widgets.open.bg_stroke = Stroke::new(1.0, BORDER_HI);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = 8.into();
    }
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.interact_size.y = 26.0;
        s.spacing.menu_margin = Margin::same(5);
        s.spacing.window_margin = Margin::same(10);
        s.spacing.scroll = egui::style::ScrollStyle::floating();
        s.spacing.scroll.floating_width = 6.0;
        s.text_styles.insert(egui::TextStyle::Body, FontId::proportional(BODY));
        s.text_styles.insert(egui::TextStyle::Button, FontId::proportional(BODY - 1.0));
        s.text_styles.insert(egui::TextStyle::Monospace, FontId::monospace(BODY - 1.0));
        s.text_styles.insert(egui::TextStyle::Small, FontId::proportional(BODY - 2.5));
    });
}

// ---------- shared widgets ----------

/// A filled amber button, the one primary action in a group.
pub fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let r = ui.add(egui::Button::new(egui::RichText::new(text).font(bold(13.5)).color(ACCENT_INK)).fill(ACCENT).stroke(Stroke::NONE).corner_radius(8));
    if r.hovered() {
        ui.painter().rect_filled(r.rect, 8, Color32::from_white_alpha(18));
    }
    r
}

/// A quiet bordered button for secondary actions.
pub fn secondary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(egui::RichText::new(text).size(13.5).color(TEXT)).corner_radius(8))
}

/// A frameless button that only shows a background on hover.
pub fn ghost_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(egui::RichText::new(text).size(13.5).color(MUTED)).frame_when_inactive(false).corner_radius(8))
}

/// A square hover-highlighted button with a painted icon.
pub fn icon_button(ui: &mut egui::Ui, size: f32, tip: &str, paint: impl FnOnce(&egui::Painter, egui::Rect, Color32)) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
    if r.hovered() {
        ui.painter().rect_filled(rect, 7, SURFACE_HI);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    paint(ui.painter(), rect, if r.hovered() { STRONG } else { MUTED });
    r.on_hover_text(tip)
}

/// Small uppercase label above a group.
pub fn section_label(ui: &mut egui::Ui, text: &str) {
    let job = egui::text::LayoutJob::single_section(
        text.to_uppercase(),
        egui::TextFormat { font_id: bold(10.5), color: DIM, extra_letter_spacing: 0.8, ..Default::default() },
    );
    ui.label(job);
}

/// Faint bits of code that rise off the edges of `rect` and fade, like heat: the composer while Scout works.
/// Each glyph runs its own loop from a fixed spot on the edge, drifting outward, and is a new token every loop.
pub fn code_aura(p: &egui::Painter, rect: egui::Rect, time: f64) {
    const TOKENS: [&str; 32] = [
        "{", "}", "(", ")", "[", "]", "<", "/>", "=>", "::", ";", "&&", "||", "fn", "let", "if", "0x", "1", "0", "==", "!=", "+=",
        "->", "//", "*", "%", "$", "#", "λ", "…", "</>", "?",
    ];
    // A cheap integer hash, so every glyph's spot, speed and token are fixed without storing anything.
    fn hash(mut x: u32) -> u32 {
        x ^= x >> 16;
        x = x.wrapping_mul(0x7feb_352d);
        x ^= x >> 15;
        x = x.wrapping_mul(0x846c_a68b);
        x ^ (x >> 16)
    }
    let unit = |x: u32| (hash(x) % 10_000) as f32 / 10_000.0;
    let (w, h) = (rect.width(), rect.height());
    let perimeter = 2.0 * (w + h);
    let count = (perimeter / 16.0) as u32;
    let font = egui::FontId::monospace(10.5);
    for i in 0..count {
        let speed = 0.25 + 0.35 * unit(i * 3 + 1);
        let phase = time as f32 * speed + unit(i * 3 + 2);
        let (cycle, f) = (phase.floor() as u32, phase.fract());
        // A spot on the edge (top, right, bottom, left) and the way out from it, nudged a little each loop.
        let u = (unit(i * 3) + 0.03 * unit(i.wrapping_mul(97) + cycle)) % 1.0 * perimeter;
        let (edge, out) = if u < w {
            (egui::pos2(rect.left() + u, rect.top()), egui::vec2(0.0, -1.0))
        } else if u < w + h {
            (egui::pos2(rect.right(), rect.top() + u - w), egui::vec2(1.0, 0.0))
        } else if u < 2.0 * w + h {
            (egui::pos2(rect.right() - (u - w - h), rect.bottom()), egui::vec2(0.0, 1.0))
        } else {
            (egui::pos2(rect.left(), rect.bottom() - (u - 2.0 * w - h)), egui::vec2(-1.0, 0.0))
        };
        // Not far below, where the footer is.
        let reach = if out.y > 0.0 { 8.0 } else { 22.0 };
        let pos = edge + out * (5.0 + reach * f);
        // Fades in quickly and out slowly.
        let alpha = (f * 6.0).min(1.0) * (1.0 - f).powf(1.5) * 0.55;
        let token = TOKENS[(hash(i * 31 + cycle * 7) as usize) % TOKENS.len()];
        let color = if hash(i + cycle) % 5 == 0 { MUTED } else { ACCENT };
        p.text(pos, egui::Align2::CENTER_CENTER, token, font.clone(), color.gamma_multiply(alpha));
    }
}

// ---------- painted icons, so they look the same with any font ----------

pub mod icon {
    use super::*;
    use egui::{Painter, Pos2, Rect, pos2, vec2};

    fn line(p: &Painter, pts: &[Pos2], c: Color32, w: f32) {
        p.add(egui::Shape::line(pts.to_vec(), Stroke::new(w, c)));
    }

    /// The app's mark: an amber tile with a white < and > interlocking into an S, like the window icon.
    pub fn logo(p: &Painter, rect: Rect) {
        // The < in a 32x32 tile, split into two convex pieces since egui only fills convex shapes.
        // The > is the same shape turned half a turn about the centre.
        const PIECES: [&[[f32; 2]]; 2] = [
            &[[18.65, 4.84], [18.65, 8.79], [10.74, 13.32], [7.48, 15.07], [7.48, 11.37]],
            &[[10.74, 13.32], [16.03, 16.57], [16.03, 20.21], [7.48, 15.07]],
        ];
        let r = (rect.height() * 0.28) as u8;
        p.rect_filled(rect, r, ACCENT);
        let s = rect.width() / 32.0;
        for flip in [false, true] {
            for piece in PIECES {
                let pts = piece
                    .iter()
                    .map(|&[x, y]| if flip { [32.0 - x, 32.0 - y] } else { [x, y] })
                    .map(|[x, y]| rect.min + vec2(x, y) * s)
                    .collect();
                // The thin outline hides the seam between the two pieces and softens the corners.
                p.add(egui::Shape::convex_polygon(pts, Color32::WHITE, Stroke::new(s * 0.6, Color32::WHITE)));
            }
        }
    }

    pub fn plus(p: &Painter, rect: Rect, c: Color32) {
        let (m, s) = (rect.center(), rect.height() * 0.22);
        line(p, &[m - vec2(s, 0.0), m + vec2(s, 0.0)], c, 1.6);
        line(p, &[m - vec2(0.0, s), m + vec2(0.0, s)], c, 1.6);
    }

    pub fn chevron_down(p: &Painter, rect: Rect, c: Color32) {
        let (m, s) = (rect.center(), rect.height() * 0.16);
        line(p, &[m + vec2(-s, -s * 0.5), m + vec2(0.0, s * 0.5), m + vec2(s, -s * 0.5)], c, 1.4);
    }

    pub fn chevron_up(p: &Painter, rect: Rect, c: Color32) {
        let (m, s) = (rect.center(), rect.height() * 0.16);
        line(p, &[m + vec2(-s, s * 0.5), m + vec2(0.0, -s * 0.5), m + vec2(s, s * 0.5)], c, 1.4);
    }

    pub fn chevron_right(p: &Painter, rect: Rect, c: Color32) {
        let (m, s) = (rect.center(), rect.height() * 0.16);
        line(p, &[m + vec2(-s * 0.5, -s), m + vec2(s * 0.5, 0.0), m + vec2(-s * 0.5, s)], c, 1.4);
    }

    pub fn arrow_up(p: &Painter, rect: Rect, c: Color32) {
        let (m, s) = (rect.center(), rect.height() * 0.24);
        line(p, &[m + vec2(0.0, s), m - vec2(0.0, s)], c, 2.0);
        line(p, &[m + vec2(-s * 0.8, -s * 0.2), m - vec2(0.0, s), m + vec2(s * 0.8, -s * 0.2)], c, 2.0);
    }

    pub fn stop(p: &Painter, rect: Rect, c: Color32) {
        let s = rect.height() * 0.3;
        p.rect_filled(Rect::from_center_size(rect.center(), vec2(s, s)), 2, c);
    }

    pub fn sidebar(p: &Painter, rect: Rect, c: Color32) {
        let r = Rect::from_center_size(rect.center(), vec2(rect.width() * 0.56, rect.height() * 0.46));
        p.rect_stroke(r, 3, Stroke::new(1.4, c), egui::StrokeKind::Middle);
        let x = r.left() + r.width() * 0.36;
        p.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], Stroke::new(1.4, c));
    }

    /// A window with a column on the right: the side panel.
    pub fn panel_right(p: &Painter, rect: Rect, c: Color32) {
        let r = Rect::from_center_size(rect.center(), vec2(rect.width() * 0.72, rect.height() * 0.6));
        p.rect_stroke(r, 2, Stroke::new(1.4, c), egui::StrokeKind::Middle);
        let x = r.right() - r.width() * 0.36;
        p.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], Stroke::new(1.4, c));
    }

    /// A prompt: >_
    pub fn terminal(p: &Painter, rect: Rect, c: Color32) {
        let (m, s) = (rect.center(), rect.height() * 0.2);
        line(p, &[m + vec2(-s * 1.3, -s * 0.8), m + vec2(-s * 0.4, 0.0), m + vec2(-s * 1.3, s * 0.8)], c, 1.4);
        line(p, &[m + vec2(0.0, s * 0.9), m + vec2(s * 1.3, s * 0.9)], c, 1.4);
    }

    /// A page with a plus over a minus: changes.
    pub fn diff(p: &Painter, rect: Rect, c: Color32) {
        let r = Rect::from_center_size(rect.center(), vec2(rect.height() * 0.56, rect.height() * 0.68));
        p.rect_stroke(r, 2, Stroke::new(1.3, c), egui::StrokeKind::Middle);
        let (m, s) = (r.center() - vec2(0.0, r.height() * 0.1), r.width() * 0.22);
        line(p, &[m - vec2(s, 0.0), m + vec2(s, 0.0)], c, 1.3);
        line(p, &[m - vec2(0.0, s), m + vec2(0.0, s)], c, 1.3);
        let y = r.bottom() - r.height() * 0.22;
        line(p, &[pos2(m.x - s, y), pos2(m.x + s, y)], c, 1.3);
    }

    /// A page with a folded corner.
    pub fn file(p: &Painter, rect: Rect, c: Color32) {
        let r = Rect::from_center_size(rect.center(), vec2(rect.height() * 0.5, rect.height() * 0.64));
        let f = r.width() * 0.38;
        line(p, &[pos2(r.right() - f, r.top()), pos2(r.left(), r.top()), pos2(r.left(), r.bottom()), pos2(r.right(), r.bottom()), pos2(r.right(), r.top() + f), pos2(r.right() - f, r.top()), pos2(r.right() - f, r.top() + f), pos2(r.right(), r.top() + f)], c, 1.2);
    }

    /// A circular arrow, for reload.
    pub fn refresh(p: &Painter, rect: Rect, c: Color32) {
        // A clockwise arc with its gap at the top right, ending in a filled head that points along the arc.
        let (m, r) = (rect.center(), rect.height() * 0.23);
        let (from, to) = (-0.2_f32, 4.75_f32);
        let pts: Vec<Pos2> = (0..=32).map(|i| {
            let a = from + i as f32 / 32.0 * (to - from);
            m + vec2(a.cos(), a.sin()) * r
        }).collect();
        line(p, &pts, c, 1.6);
        let end = m + vec2(to.cos(), to.sin()) * r;
        let dir = vec2(-to.sin(), to.cos());
        let out = vec2(to.cos(), to.sin());
        let h = r * 0.62;
        p.add(egui::Shape::convex_polygon(vec![end + dir * h, end + out * h * 0.75, end - out * h * 0.75], c, Stroke::NONE));
    }

    pub fn arrow_left(p: &Painter, rect: Rect, c: Color32) {
        let (m, s) = (rect.center(), rect.height() * 0.22);
        line(p, &[m + vec2(s, 0.0), m - vec2(s, 0.0)], c, 1.5);
        line(p, &[m + vec2(-s * 0.2, -s * 0.8), m - vec2(s, 0.0), m + vec2(-s * 0.2, s * 0.8)], c, 1.5);
    }

    pub fn folder(p: &Painter, rect: Rect, c: Color32) {
        let r = Rect::from_center_size(rect.center(), vec2(rect.height() * 0.78, rect.height() * 0.58));
        let tab = r.left() + r.width() * 0.42;
        line(
            p,
            &[
                pos2(r.left(), r.bottom()),
                pos2(r.left(), r.top()),
                pos2(tab - 2.0, r.top()),
                pos2(tab, r.top() + 2.5),
                pos2(r.right(), r.top() + 2.5),
                pos2(r.right(), r.bottom()),
                pos2(r.left(), r.bottom()),
            ],
            c,
            1.3,
        );
    }

    pub fn check(p: &Painter, rect: Rect, c: Color32) {
        let (m, s) = (rect.center(), rect.height() * 0.22);
        line(p, &[m + vec2(-s, 0.0), m + vec2(-s * 0.3, s * 0.7), m + vec2(s, -s * 0.6)], c, 1.6);
    }

    pub fn cross(p: &Painter, rect: Rect, c: Color32) {
        let (m, s) = (rect.center(), rect.height() * 0.18);
        line(p, &[m + vec2(-s, -s), m + vec2(s, s)], c, 1.6);
        line(p, &[m + vec2(-s, s), m + vec2(s, -s)], c, 1.6);
    }

    /// A five-point star, filled when `filled`, else outlined.
    pub fn star(p: &Painter, rect: Rect, c: Color32, filled: bool) {
        let (m, r) = (rect.center() + vec2(0.0, rect.height() * 0.02), rect.height() * 0.3);
        let pt = |i: usize| {
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
            let d = if i % 2 == 0 { r } else { r * 0.45 };
            m + vec2(a.cos(), a.sin()) * d
        };
        if filled {
            // Not convex, so: the inner pentagon plus a triangle per point.
            p.add(egui::Shape::convex_polygon((0..5).map(|i| pt(2 * i + 1)).collect(), c, Stroke::new(0.5, c)));
            for i in 0..5 {
                p.add(egui::Shape::convex_polygon(vec![pt((2 * i + 9) % 10), pt(2 * i), pt(2 * i + 1)], c, Stroke::new(0.5, c)));
            }
        } else {
            p.add(egui::Shape::closed_line((0..10).map(pt).collect(), Stroke::new(1.2, c)));
        }
    }

    // Window buttons, drawn thin like Windows' own.

    pub fn minimize(p: &Painter, rect: Rect, c: Color32) {
        let (m, s) = (rect.center(), 5.0);
        p.line_segment([m - vec2(s, 0.0), m + vec2(s, 0.0)], Stroke::new(1.0, c));
    }

    pub fn maximize(p: &Painter, rect: Rect, c: Color32) {
        p.rect_stroke(Rect::from_center_size(rect.center(), vec2(10.0, 10.0)), 1, Stroke::new(1.0, c), egui::StrokeKind::Middle);
    }

    /// Two overlapping squares: un-maximize.
    pub fn restore(p: &Painter, rect: Rect, c: Color32) {
        let front = Rect::from_center_size(rect.center() + vec2(-1.0, 1.0), vec2(8.0, 8.0));
        p.rect_stroke(front, 1, Stroke::new(1.0, c), egui::StrokeKind::Middle);
        let (l, t, r, b) = (front.left() + 2.0, front.top() - 2.0, front.right() + 2.0, front.bottom() - 2.0);
        line(p, &[pos2(l, front.top()), pos2(l, t), pos2(r, t), pos2(r, b), pos2(front.right(), b)], c, 1.0);
    }

    pub fn close(p: &Painter, rect: Rect, c: Color32) {
        let (m, s) = (rect.center(), 5.0);
        line(p, &[m + vec2(-s, -s), m + vec2(s, s)], c, 1.0);
        line(p, &[m + vec2(-s, s), m + vec2(s, -s)], c, 1.0);
    }

    /// A paper clip, for attaching files.
    pub fn paperclip(p: &Painter, rect: Rect, c: Color32) {
        let (m, s) = (rect.center(), rect.height() / 20.0);
        let pt = |x: f32, y: f32| m + vec2(x, y) * s;
        let arc = |cx: f32, cy: f32, r: f32, from: f32, to: f32| -> Vec<Pos2> {
            (0..=10).map(|i| from + (to - from) * i as f32 / 10.0).map(|a| pt(cx + r * a.cos(), cy + r * a.sin())).collect()
        };
        use std::f32::consts::PI;
        // Outer loop: down the right side, round the bottom, up the left, round the top, then the inner prong.
        let mut pts = vec![pt(3.0, -2.0), pt(3.0, 4.0)];
        pts.extend(arc(0.0, 4.0, 3.0, 0.0, PI));
        pts.push(pt(-3.0, -5.0));
        pts.extend(arc(-0.75, -5.0, 2.25, PI, 2.0 * PI));
        pts.push(pt(1.5, 3.0));
        pts.extend(arc(0.0, 3.0, 1.5, 0.0, PI));
        pts.push(pt(-1.5, -2.5));
        line(p, &pts, c, 1.4);
    }

    /// A microphone on a stand, for dictation.
    pub fn mic(p: &Painter, rect: Rect, c: Color32) {
        let (m, s) = (rect.center(), rect.height() / 20.0);
        let pt = |x: f32, y: f32| m + vec2(x, y) * s;
        let head = Rect::from_min_max(pt(-2.4, -6.5), pt(2.4, 2.0));
        p.rect_stroke(head, 2.4 * s, Stroke::new(1.4, c), egui::StrokeKind::Middle);
        let cup: Vec<Pos2> = (0..=12).map(|i| std::f32::consts::PI * i as f32 / 12.0).map(|a| pt(4.5 * a.cos(), -0.5 + 4.5 * a.sin())).collect();
        line(p, &cup, c, 1.4);
        line(p, &[pt(0.0, 4.0), pt(0.0, 6.5)], c, 1.4);
    }

    /// A pencil, for editing.
    pub fn pencil(p: &Painter, rect: Rect, c: Color32) {
        let (m, s) = (rect.center(), rect.height() / 20.0);
        let pt = |x: f32, y: f32| m + vec2(x, y) * s;
        line(p, &[pt(-5.0, 5.0), pt(-4.2, 2.2), pt(3.2, -5.2), pt(5.2, -3.2), pt(-2.2, 4.2), pt(-5.0, 5.0)], c, 1.4);
        line(p, &[pt(1.6, -3.6), pt(3.6, -1.6)], c, 1.4);
    }

    /// A magnifying glass.
    pub fn search(p: &Painter, rect: Rect, c: Color32) {
        let (m, s) = (rect.center(), rect.height() / 20.0);
        p.circle_stroke(m + vec2(-1.2, -1.2) * s, 4.2 * s, Stroke::new(1.4, c));
        line(p, &[m + vec2(1.9, 1.9) * s, m + vec2(5.0, 5.0) * s], c, 1.6);
    }

    /// A bell, for notifications.
    pub fn bell(p: &Painter, rect: Rect, c: Color32) {
        let (m, s) = (rect.center(), rect.height() / 20.0);
        let pt = |x: f32, y: f32| m + vec2(x, y) * s;
        let mut pts = vec![pt(-5.5, 3.5), pt(-4.2, 2.0), pt(-4.2, -1.5)];
        pts.extend((0..=10).map(|i| std::f32::consts::PI * (1.0 + i as f32 / 10.0)).map(|a| pt(4.2 * a.cos(), -1.5 + 4.2 * a.sin())));
        pts.extend([pt(4.2, 2.0), pt(5.5, 3.5), pt(-5.5, 3.5)]);
        line(p, &pts, c, 1.4);
        line(p, &[pt(-1.6, 5.6), pt(1.6, 5.6)], c, 1.4);
    }

    /// Three rising bars, for usage.
    pub fn chart(p: &Painter, rect: Rect, c: Color32) {
        let (m, s) = (rect.center(), rect.height() / 20.0);
        for (x, h) in [(-4.0, 4.0), (0.0, 7.0), (4.0, 10.0)] {
            let r = Rect::from_min_max(m + vec2(x - 1.3, 5.5 - h) * s, m + vec2(x + 1.3, 5.5) * s);
            p.rect_stroke(r, 1, Stroke::new(1.3, c), egui::StrokeKind::Middle);
        }
    }

    /// A head and shoulders, for the account.
    pub fn user(p: &Painter, rect: Rect, c: Color32) {
        let (m, s) = (rect.center(), rect.height() / 20.0);
        p.circle_stroke(m + vec2(0.0, -2.8) * s, 3.0 * s, Stroke::new(1.4, c));
        let pts: Vec<Pos2> = (0..=12).map(|i| std::f32::consts::PI * (1.0 + i as f32 / 12.0)).map(|a| m + vec2(5.5 * a.cos(), 6.0 + 5.0 * a.sin()) * s).collect();
        line(p, &pts, c, 1.4);
    }

    /// Three dots, for a "more" menu.
    pub fn more(p: &Painter, rect: Rect, c: Color32) {
        for dx in [-5.0, 0.0, 5.0] {
            p.circle_filled(rect.center() + vec2(dx, 0.0), 1.4, c);
        }
    }
}
