//! Colours, fonts and egui style of the "lucid" light theme.

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Visuals};
use std::sync::Arc;

pub const ACCENT: Color32 = Color32::from_rgb(0x2F, 0x6F, 0xEB);
pub const INK: Color32 = Color32::from_rgb(0x1F, 0x23, 0x28);
pub const TEXT: Color32 = Color32::from_rgb(0x24, 0x29, 0x2F);
pub const MUTED: Color32 = Color32::from_rgb(0x6B, 0x72, 0x80);
pub const FAINT: Color32 = Color32::from_rgb(0x9C, 0xA3, 0xAF);
pub const LINE: Color32 = Color32::from_rgb(0xE3, 0xE6, 0xEA);
pub const PANEL: Color32 = Color32::from_rgb(0xF7, 0xF8, 0xFA);
pub const SURFACE: Color32 = Color32::WHITE;
pub const CLIP_BG: Color32 = Color32::from_rgb(0xF1, 0xF4, 0xF8);
pub const DANGER: Color32 = Color32::from_rgb(0xD9, 0x30, 0x25);
pub const SUCCESS: Color32 = Color32::from_rgb(0x1D, 0x7A, 0x46);
pub const MATCH: Color32 = Color32::from_rgb(0xFF, 0xE0, 0x66);
pub const CANVAS_DEFAULT: Color32 = Color32::from_rgb(0xFB, 0xFB, 0xFA);

/// Background presets offered in the View menu.
pub const CANVAS_PRESETS: [(&str, Color32); 7] = [
    ("Paper", CANVAS_DEFAULT),
    ("White", Color32::WHITE),
    ("Mist", Color32::from_rgb(0xF0, 0xF3, 0xF7)),
    ("Sand", Color32::from_rgb(0xF7, 0xF2, 0xE8)),
    ("Mint", Color32::from_rgb(0xEE, 0xF6, 0xF1)),
    ("Blush", Color32::from_rgb(0xF9, 0xEF, 0xF1)),
    ("Slate", Color32::from_rgb(0x2B, 0x30, 0x38)),
];

/// Colours used on a canvas, derived from its (user-chosen) background so that
/// borders and text stay readable on light as well as dark backgrounds.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    /// Background of the canvas and of all folder rectangles.
    pub bg: Color32,
    /// Folder borders and folder names.
    pub ink: Color32,
    /// File names and other body text.
    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,
    /// Translucent plate behind labels that are drawn larger than the canvas scale.
    pub backdrop: Color32,
    /// Background dots.
    pub dot: Color32,
}

impl Palette {
    pub fn for_bg(bg: Color32) -> Self {
        let bg = Color32::from_rgb(bg.r(), bg.g(), bg.b());
        let backdrop = Color32::from_rgba_unmultiplied(bg.r(), bg.g(), bg.b(), 225);
        if is_dark(bg) {
            Palette {
                bg,
                ink: Color32::from_rgb(0xEE, 0xF0, 0xF3),
                text: Color32::from_rgb(0xE4, 0xE7, 0xEB),
                muted: Color32::from_rgb(0xB4, 0xBA, 0xC2),
                faint: Color32::from_rgb(0x8C, 0x93, 0x9C),
                backdrop,
                dot: Color32::from_white_alpha(40),
            }
        } else {
            Palette { bg, ink: INK, text: TEXT, muted: MUTED, faint: FAINT, backdrop, dot: Color32::from_black_alpha(34) }
        }
    }
}

/// Perceived brightness below the middle grey.
pub fn is_dark(c: Color32) -> bool {
    (c.r() as f32 * 0.299 + c.g() as f32 * 0.587 + c.b() as f32 * 0.114) < 140.0
}

/// Linear blend of two colours (`t` = 0 gives `a`).
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

pub fn accent_soft(a: f32) -> Color32 {
    ACCENT.gamma_multiply(a)
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("inter".into(), Arc::new(FontData::from_static(include_bytes!("../assets/Inter-Regular.ttf"))));
    fonts
        .font_data
        .insert("inter-semibold".into(), Arc::new(FontData::from_static(include_bytes!("../assets/Inter-SemiBold.ttf"))));
    let fallbacks: Vec<String> = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut prop = vec!["inter".to_string()];
    prop.extend(fallbacks.iter().cloned());
    fonts.families.insert(FontFamily::Proportional, prop);
    let mut semi = vec!["inter-semibold".to_string()];
    semi.extend(fallbacks);
    fonts.families.insert(FontFamily::Name("semibold".into()), semi);
    ctx.set_fonts(fonts);

    ctx.global_style_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(11.0)),
            (TextStyle::Body, FontId::proportional(13.0)),
            (TextStyle::Button, FontId::proportional(13.0)),
            (TextStyle::Heading, semibold(16.0)),
            (TextStyle::Monospace, FontId::monospace(12.0)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size.y = 28.0;
        style.spacing.menu_margin = Margin::same(6);
        style.spacing.window_margin = Margin::same(14);

        let mut v = Visuals::light();
        v.override_text_color = Some(TEXT);
        v.panel_fill = PANEL;
        v.window_fill = SURFACE;
        v.extreme_bg_color = SURFACE;
        v.faint_bg_color = Color32::from_rgb(0xF2, 0xF4, 0xF7);
        v.window_stroke = Stroke::new(1.0, LINE);
        v.window_corner_radius = CornerRadius::same(10);
        v.menu_corner_radius = CornerRadius::same(8);
        v.window_shadow = Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(30) };
        v.popup_shadow = Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(28) };
        v.selection.bg_fill = accent_soft(0.25);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.hyperlink_color = ACCENT;
        let r = CornerRadius::same(6);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
        v.widgets.noninteractive.corner_radius = r;
        v.widgets.inactive.bg_fill = Color32::from_rgb(0xE6, 0xE9, 0xEE);
        v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        v.widgets.inactive.bg_stroke = Stroke::NONE;
        v.widgets.inactive.corner_radius = r;
        v.widgets.hovered.bg_fill = Color32::from_rgb(0xEC, 0xEF, 0xF3);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0xEC, 0xEF, 0xF3);
        v.widgets.hovered.bg_stroke = Stroke::NONE;
        v.widgets.hovered.corner_radius = r;
        v.widgets.active.bg_fill = Color32::from_rgb(0xE1, 0xE6, 0xEE);
        v.widgets.active.weak_bg_fill = Color32::from_rgb(0xE1, 0xE6, 0xEE);
        v.widgets.active.bg_stroke = Stroke::NONE;
        v.widgets.active.corner_radius = r;
        v.widgets.open.weak_bg_fill = Color32::from_rgb(0xE1, 0xE6, 0xEE);
        v.widgets.open.corner_radius = r;
        style.visuals = v;
    });
}

/// Frame of the floating toolbars on the canvas (miro-like).
pub fn floating_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(10)
        .inner_margin(Margin::same(4))
        .shadow(Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(22) })
}

/// Window icon: a blue rounded square with nested folder outlines.
pub fn app_icon() -> egui::IconData {
    const N: usize = 64;
    let mut rgba = vec![0u8; N * N * 4];
    let mut put = |x: usize, y: usize, c: [u8; 4]| {
        let i = (y * N + x) * 4;
        rgba[i..i + 4].copy_from_slice(&c);
    };
    let inside_round = |x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32| {
        let cx = x.clamp(x0 + r, x1 - r);
        let cy = y.clamp(y0 + r, y1 - r);
        x >= x0 && x <= x1 && y >= y0 && y <= y1 && (x - cx).powi(2) + (y - cy).powi(2) <= r * r
    };
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            if !inside_round(fx, fy, 2.0, 2.0, 62.0, 62.0, 14.0) {
                continue;
            }
            let mut c = [0x2F, 0x6F, 0xEB, 255];
            let outline = |x0: f32, y0: f32, x1: f32, y1: f32| {
                inside_round(fx, fy, x0, y0, x1, y1, 3.0) && !inside_round(fx, fy, x0 + 3.0, y0 + 3.0, x1 - 3.0, y1 - 3.0, 1.0)
            };
            if outline(13.0, 16.0, 51.0, 49.0) || outline(30.0, 30.0, 45.0, 43.0) {
                c = [255, 255, 255, 255];
            }
            if inside_round(fx, fy, 13.0, 11.0, 28.0, 18.0, 2.0) || inside_round(fx, fy, 17.0, 30.0, 25.0, 38.0, 1.5) {
                c = [255, 255, 255, 255];
            }
            put(x, y, c);
        }
    }
    egui::IconData { rgba, width: N as u32, height: N as u32 }
}
