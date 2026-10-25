//! Vector file-type icons in the spirit of the Windows Explorer icon set:
//! a sheet of paper with a folded corner, a category glyph and a coloured
//! extension badge. Drawn with shapes so they stay crisp at every zoom level.

use egui::{Align2, Color32, FontFamily, FontId, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2, pos2, vec2};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileKind {
    Text,
    Document,
    Spreadsheet,
    Presentation,
    Pdf,
    Image,
    Audio,
    Video,
    Archive,
    Code,
    Executable,
    Font,
    Disk,
    Generic,
}

pub fn classify(ext: &str) -> FileKind {
    use FileKind::*;
    match ext {
        "txt" | "md" | "markdown" | "log" | "rtf" | "nfo" | "readme" | "ini" | "cfg" | "conf" => Text,
        "doc" | "docx" | "odt" | "pages" | "dot" | "dotx" | "wpd" => Document,
        "xls" | "xlsx" | "xlsm" | "ods" | "csv" | "tsv" | "numbers" => Spreadsheet,
        "ppt" | "pptx" | "odp" | "key" | "pps" | "ppsx" => Presentation,
        "pdf" | "xps" | "epub" => Pdf,
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "svg" | "webp" | "ico" | "tif" | "tiff" | "heic" | "heif" | "raw"
        | "cr2" | "nef" | "psd" | "xcf" | "avif" => Image,
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "wma" | "opus" | "mid" | "midi" => Audio,
        "mp4" | "mkv" | "avi" | "mov" | "webm" | "wmv" | "flv" | "m4v" | "mpg" | "mpeg" => Video,
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "cab" | "deb" | "rpm" | "jar" | "apk" => Archive,
        "rs" | "py" | "js" | "ts" | "jsx" | "tsx" | "c" | "h" | "cpp" | "hpp" | "cc" | "cs" | "java" | "kt" | "go"
        | "rb" | "php" | "swift" | "html" | "htm" | "css" | "scss" | "json" | "xml" | "yaml" | "yml" | "toml"
        | "sh" | "bash" | "zsh" | "ps1" | "bat" | "cmd" | "sql" | "lua" | "pl" | "r" | "dart" | "vue" | "ipynb" => Code,
        "exe" | "msi" | "appimage" | "app" | "com" | "bin" | "run" | "dll" | "so" | "dylib" | "sys" => Executable,
        "ttf" | "otf" | "woff" | "woff2" | "fon" => Font,
        "iso" | "img" | "dmg" | "vhd" | "vhdx" | "vmdk" | "qcow2" => Disk,
        _ => Generic,
    }
}

impl FileKind {
    /// Accent colour of the category.
    pub fn color(self) -> Color32 {
        use FileKind::*;
        match self {
            Text => Color32::from_rgb(0x6B, 0x77, 0x85),
            Document => Color32::from_rgb(0x2B, 0x59, 0xC3),
            Spreadsheet => Color32::from_rgb(0x1D, 0x7A, 0x46),
            Presentation => Color32::from_rgb(0xD2, 0x4A, 0x26),
            Pdf => Color32::from_rgb(0xD9, 0x30, 0x25),
            Image => Color32::from_rgb(0x0E, 0x9F, 0x9A),
            Audio => Color32::from_rgb(0xC2, 0x3B, 0x8A),
            Video => Color32::from_rgb(0x6D, 0x3F, 0xC9),
            Archive => Color32::from_rgb(0xB7, 0x7B, 0x0B),
            Code => Color32::from_rgb(0x34, 0x4D, 0x7A),
            Executable => Color32::from_rgb(0x2F, 0x6F, 0xEB),
            Font => Color32::from_rgb(0x4A, 0x4A, 0x55),
            Disk => Color32::from_rgb(0x52, 0x60, 0x70),
            Generic => Color32::from_rgb(0x8A, 0x93, 0x9E),
        }
    }

    pub fn description(self) -> &'static str {
        use FileKind::*;
        match self {
            Text => "Text document",
            Document => "Word processing document",
            Spreadsheet => "Spreadsheet",
            Presentation => "Presentation",
            Pdf => "PDF / e-book document",
            Image => "Image",
            Audio => "Audio file",
            Video => "Video file",
            Archive => "Compressed archive",
            Code => "Source code / markup",
            Executable => "Application / binary",
            Font => "Font file",
            Disk => "Disk image",
            Generic => "File",
        }
    }
}

pub fn type_label(name: &str, is_dir: bool) -> String {
    if is_dir {
        return "File folder".into();
    }
    let ext = crate::fsmodel::extension(name);
    if ext.is_empty() {
        return "File".into();
    }
    format!("{} ({})", classify(&ext).description(), ext.to_uppercase())
}

fn with_alpha(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a)
}

/// Draws a file icon filling (roughly) the square `r`.
pub fn paint_file_icon(p: &Painter, r: Rect, name: &str, alpha: f32) {
    let ext = crate::fsmodel::extension(name);
    let kind = classify(&ext);
    let s = r.width() / 44.0; // design grid is 44x44
    let accent = with_alpha(kind.color(), alpha);

    // Sheet of paper.
    let page = Rect::from_center_size(r.center(), vec2(34.0 * s, 42.0 * s));
    let fold = 10.0 * s;
    let outline = with_alpha(Color32::from_rgb(0x9A, 0xA3, 0xAE), alpha);
    let paper = with_alpha(Color32::WHITE, alpha);
    let stroke_w = (1.0 * s).clamp(0.5, 1.5);
    let pts = vec![
        page.left_top(),
        pos2(page.right() - fold, page.top()),
        pos2(page.right(), page.top() + fold),
        page.right_bottom(),
        page.left_bottom(),
    ];
    if s > 0.35 {
        // soft drop shadow
        let sh: Vec<Pos2> = pts.iter().map(|p| *p + vec2(0.0, 1.2 * s)).collect();
        p.add(Shape::convex_polygon(sh, with_alpha(Color32::from_black_alpha(28), alpha), Stroke::NONE));
    }
    p.add(Shape::convex_polygon(pts, paper, Stroke::new(stroke_w, outline)));
    p.add(Shape::convex_polygon(
        vec![
            pos2(page.right() - fold, page.top()),
            pos2(page.right() - fold, page.top() + fold),
            pos2(page.right(), page.top() + fold),
        ],
        with_alpha(Color32::from_rgb(0xE3, 0xE7, 0xEC), alpha),
        Stroke::new(stroke_w, outline),
    ));
    if s < 0.22 {
        p.rect_filled(Rect::from_min_size(pos2(page.left(), page.bottom() - 12.0 * s), vec2(page.width(), 8.0 * s)), 0.0, accent);
        return;
    }

    // Category glyph in the upper part.
    let g = Rect::from_center_size(pos2(page.center().x, page.top() + 15.5 * s), vec2(20.0 * s, 16.0 * s));
    paint_glyph(p, kind, g, s, accent, alpha);

    // Extension badge.
    let label = if ext.is_empty() { String::new() } else { ext.to_uppercase().chars().take(4).collect() };
    if !label.is_empty() {
        let font = FontId::new(7.4 * s, FontFamily::Name("semibold".into()));
        let badge_h = 10.5 * s;
        let galley = p.layout_no_wrap(label, font, Color32::WHITE);
        let bw = (galley.size().x + 6.0 * s).max(16.0 * s);
        let badge = Rect::from_min_size(pos2(page.left() - 3.0 * s, page.bottom() - badge_h - 5.0 * s), vec2(bw, badge_h));
        p.rect_filled(badge, 2.0 * s, accent);
        if s > 0.45 {
            p.galley(badge.center() - galley.size() / 2.0, galley, Color32::WHITE);
        }
    } else {
        for i in 0..3 {
            let y = page.bottom() - (7.0 + i as f32 * 4.5) * s;
            p.line_segment([pos2(page.left() + 6.0 * s, y), pos2(page.right() - 6.0 * s, y)], Stroke::new(1.2 * s, with_alpha(Color32::from_gray(200), alpha)));
        }
    }
}

fn paint_glyph(p: &Painter, kind: FileKind, g: Rect, s: f32, c: Color32, alpha: f32) {
    use FileKind::*;
    let st = Stroke::new((1.6 * s).max(0.6), c);
    let font = |size: f32| FontId::new(size * s, FontFamily::Name("semibold".into()));
    match kind {
        Text | Generic => {
            for i in 0..4 {
                let y = g.top() + 2.0 * s + i as f32 * 3.6 * s;
                let w = if i == 3 { 0.6 } else { 1.0 };
                p.line_segment([pos2(g.left(), y), pos2(g.left() + g.width() * w, y)], Stroke::new(1.3 * s, with_alpha(Color32::from_gray(170), alpha)));
            }
        }
        Document | Spreadsheet | Presentation => {
            let letter = match kind {
                Document => "W",
                Spreadsheet => "X",
                _ => "P",
            };
            let sq = Rect::from_center_size(g.center(), Vec2::splat(15.0 * s));
            p.rect_filled(sq, 2.5 * s, c);
            p.text(sq.center(), Align2::CENTER_CENTER, letter, font(10.0), with_alpha(Color32::WHITE, alpha));
        }
        Pdf => {
            // stylised "acrobat" loop
            let ctr = g.center();
            let pts: Vec<Pos2> = (0..=40)
                .map(|i| {
                    let t = i as f32 / 40.0 * std::f32::consts::TAU;
                    ctr + vec2((t * 1.0).sin() * 7.0 * s, (t * 2.0).sin() * 3.5 * s - (t).cos() * 3.0 * s)
                })
                .collect();
            p.add(Shape::line(pts, st));
        }
        Image => {
            let fr = g.shrink(1.0 * s);
            p.rect_stroke(fr, 1.5 * s, Stroke::new(1.2 * s, c), StrokeKind::Inside);
            p.add(Shape::convex_polygon(
                vec![pos2(fr.left() + 2.0 * s, fr.bottom() - 2.0 * s), pos2(fr.left() + 7.0 * s, fr.top() + 6.0 * s), pos2(fr.left() + 12.0 * s, fr.bottom() - 2.0 * s)],
                c,
                Stroke::NONE,
            ));
            p.add(Shape::convex_polygon(
                vec![pos2(fr.left() + 9.0 * s, fr.bottom() - 2.0 * s), pos2(fr.left() + 13.0 * s, fr.top() + 8.0 * s), pos2(fr.right() - 2.0 * s, fr.bottom() - 2.0 * s)],
                c.gamma_multiply(0.7),
                Stroke::NONE,
            ));
            p.circle_filled(pos2(fr.right() - 4.5 * s, fr.top() + 4.0 * s), 1.8 * s, Color32::from_rgb(0xF5, 0xB7, 0x31).gamma_multiply(alpha));
        }
        Audio => {
            let a = pos2(g.center().x - 5.0 * s, g.bottom() - 3.0 * s);
            let b = pos2(g.center().x + 5.0 * s, g.bottom() - 5.0 * s);
            p.circle_filled(a, 2.6 * s, c);
            p.circle_filled(b, 2.6 * s, c);
            p.line_segment([a + vec2(2.3 * s, 0.0), a + vec2(2.3 * s, -11.0 * s)], st);
            p.line_segment([b + vec2(2.3 * s, 0.0), b + vec2(2.3 * s, -11.0 * s)], st);
            p.line_segment([a + vec2(2.3 * s, -11.0 * s), b + vec2(2.3 * s, -11.0 * s)], Stroke::new(2.4 * s, c));
        }
        Video => {
            let fr = Rect::from_center_size(g.center(), vec2(18.0 * s, 13.0 * s));
            p.rect_filled(fr, 2.0 * s, c);
            let ctr = fr.center();
            p.add(Shape::convex_polygon(
                vec![ctr + vec2(-2.5 * s, -3.5 * s), ctr + vec2(4.0 * s, 0.0), ctr + vec2(-2.5 * s, 3.5 * s)],
                with_alpha(Color32::WHITE, alpha),
                Stroke::NONE,
            ));
        }
        Archive => {
            let x = g.center().x;
            for i in 0..5 {
                let y = g.top() + i as f32 * 3.0 * s;
                let dx = if i % 2 == 0 { -1.5 } else { 1.5 };
                p.rect_filled(Rect::from_center_size(pos2(x + dx * s, y), vec2(3.0 * s, 2.0 * s)), 0.0, c);
            }
            let lock = Rect::from_center_size(pos2(x, g.bottom() - 1.0 * s), vec2(6.0 * s, 5.0 * s));
            p.rect_filled(lock, 1.0 * s, c);
        }
        Code => {
            let ctr = g.center();
            p.add(Shape::line(vec![ctr + vec2(-4.0 * s, -5.0 * s), ctr + vec2(-9.0 * s, 0.0), ctr + vec2(-4.0 * s, 5.0 * s)], st));
            p.add(Shape::line(vec![ctr + vec2(4.0 * s, -5.0 * s), ctr + vec2(9.0 * s, 0.0), ctr + vec2(4.0 * s, 5.0 * s)], st));
            p.line_segment([ctr + vec2(1.8 * s, -6.0 * s), ctr + vec2(-1.8 * s, 6.0 * s)], st);
        }
        Executable => {
            let fr = Rect::from_center_size(g.center(), vec2(18.0 * s, 14.0 * s));
            p.rect_stroke(fr, 1.5 * s, Stroke::new(1.3 * s, c), StrokeKind::Inside);
            p.rect_filled(Rect::from_min_size(fr.min, vec2(fr.width(), 3.5 * s)), 1.5 * s, c);
            p.rect_filled(Rect::from_center_size(fr.center() + vec2(0.0, 2.0 * s), vec2(7.0 * s, 4.0 * s)), 1.0 * s, c.gamma_multiply(0.6));
        }
        Font => {
            p.text(g.center(), Align2::CENTER_CENTER, "Aa", font(11.0), c);
        }
        Disk => {
            p.circle_stroke(g.center(), 7.0 * s, Stroke::new(1.4 * s, c));
            p.circle_filled(g.center(), 2.0 * s, c);
        }
    }
}

/// Classic yellow folder icon filling the square `r`.
pub fn paint_folder_icon(p: &Painter, r: Rect, alpha: f32, open: bool) {
    let s = r.width() / 44.0;
    let back = Color32::from_rgb(0xE9, 0xA8, 0x2B).gamma_multiply(alpha);
    let front = Color32::from_rgb(0xFF, 0xCB, 0x52).gamma_multiply(alpha);
    let body = Rect::from_center_size(r.center() + vec2(0.0, 2.0 * s), vec2(40.0 * s, 30.0 * s));
    // tab
    let tab = Rect::from_min_size(pos2(body.left(), body.top() - 4.0 * s), vec2(16.0 * s, 8.0 * s));
    p.rect_filled(tab, 2.0 * s, back);
    p.rect_filled(body, 3.0 * s, back);
    if s > 0.3 {
        // a sheet peeking out
        let sheet = Rect::from_min_max(pos2(body.left() + 4.0 * s, body.top() + 3.0 * s), pos2(body.right() - 4.0 * s, body.bottom() - 6.0 * s));
        p.rect_filled(sheet, 1.0 * s, Color32::WHITE.gamma_multiply(alpha));
    }
    let front_rect = if open {
        Rect::from_min_max(pos2(body.left() + 2.0 * s, body.top() + 9.0 * s), body.max)
    } else {
        Rect::from_min_max(pos2(body.left(), body.top() + 6.0 * s), body.max)
    };
    p.rect_filled(front_rect, 3.0 * s, front);
}

/// The little "+N more" tile.
pub fn paint_more_tile(p: &Painter, r: Rect, n: usize, s: f32) {
    let c = Color32::from_rgb(0x8A, 0x93, 0x9E);
    let st = Stroke::new(1.0, c);
    let shape = Shape::dashed_line(&[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()], st, 4.0, 3.0);
    p.extend(shape);
    if s > 0.3 {
        p.text(r.center(), Align2::CENTER_CENTER, format!("+{n} more"), FontId::proportional(11.0 * s), c);
    }
}
