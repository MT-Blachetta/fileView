//! Small line icons for toolbars, drawn with shapes (no icon font needed).

use crate::theme;
use egui::{Color32, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, vec2};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ico {
    Cursor,
    Hand,
    DrawRect,
    NewFolder,
    NewFile,
    Back,
    Forward,
    Up,
    Refresh,
    Search,
    Plus,
    Minus,
    Fit,
    Gear,
    Help,
    Undo,
    Clipboard,
    Trash,
    Grid,
    Map,
    Info,
    Close,
}

pub fn paint(p: &egui::Painter, r: Rect, ico: Ico, color: Color32) {
    let c = r.center();
    let s = r.width() / 18.0;
    let st = Stroke::new(1.5 * s.max(0.8), color);
    let at = |x: f32, y: f32| -> Pos2 { c + vec2(x * s, y * s) };
    let line = |pts: &[(f32, f32)]| Shape::line(pts.iter().map(|&(x, y)| at(x, y)).collect(), st);
    let mut shapes: Vec<Shape> = vec![];
    match ico {
        Ico::Cursor => {
            shapes.push(Shape::convex_polygon(
                vec![at(-4.5, -7.0), at(5.0, 1.5), at(0.6, 2.0), at(-4.5, 6.0)],
                color,
                Stroke::NONE,
            ));
            shapes.push(Shape::line_segment([at(0.2, 1.5), at(3.0, 6.5)], Stroke::new(2.0 * s, color)));
        }
        Ico::Hand => {
            for (i, x) in [-4.0f32, -1.3, 1.4, 4.1].iter().enumerate() {
                let top = if i == 0 { -2.5 } else { -6.0 + (i as f32 - 1.5).abs() };
                shapes.push(line(&[(*x, 1.0), (*x, top)]));
            }
            shapes.push(line(&[(-4.0, 1.0), (-6.0, -1.0)]));
            shapes.push(Shape::closed_line(
                vec![at(-5.0, 1.0), at(5.0, 1.0), at(4.0, 6.5), at(-3.0, 6.5)],
                st,
            ));
        }
        Ico::DrawRect => {
            shapes.push(Shape::rect_stroke(
                Rect::from_min_max(at(-7.0, -5.0), at(5.0, 5.0)),
                1.5 * s,
                st,
                StrokeKind::Middle,
            ));
            shapes.push(Shape::rect_filled(Rect::from_center_size(at(5.0, 5.0), Vec2::splat(4.0 * s)), 1.0, color));
            shapes.push(Shape::rect_filled(Rect::from_center_size(at(-7.0, -5.0), Vec2::splat(4.0 * s)), 1.0, color));
        }
        Ico::NewFolder | Ico::NewFile => {
            if ico == Ico::NewFolder {
                shapes.push(Shape::closed_line(
                    vec![at(-8.0, -5.0), at(-3.0, -5.0), at(-1.5, -3.0), at(7.0, -3.0), at(7.0, 6.0), at(-8.0, 6.0)],
                    st,
                ));
            } else {
                shapes.push(Shape::closed_line(
                    vec![at(-6.0, -7.5), at(1.5, -7.5), at(6.0, -3.0), at(6.0, 7.5), at(-6.0, 7.5)],
                    st,
                ));
            }
            shapes.push(line(&[(-0.5, -1.0), (-0.5, 5.0)]));
            shapes.push(line(&[(-3.5, 2.0), (2.5, 2.0)]));
        }
        Ico::Back => shapes.push(line(&[(2.5, -6.0), (-3.5, 0.0), (2.5, 6.0)])),
        Ico::Forward => shapes.push(line(&[(-2.5, -6.0), (3.5, 0.0), (-2.5, 6.0)])),
        Ico::Up => {
            shapes.push(line(&[(-5.5, -1.0), (0.0, -6.5), (5.5, -1.0)]));
            shapes.push(line(&[(0.0, -6.0), (0.0, 7.0)]));
        }
        Ico::Refresh => {
            let pts: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let t = -0.6 + i as f32 / 24.0 * 5.0;
                    c + vec2(t.cos(), t.sin()) * 6.0 * s
                })
                .collect();
            let end = *pts.last().unwrap();
            shapes.push(Shape::line(pts, st));
            shapes.push(Shape::convex_polygon(
                vec![end + vec2(-3.0, -2.5) * s, end + vec2(3.2, -1.2) * s, end + vec2(-0.2, 3.5) * s],
                color,
                Stroke::NONE,
            ));
        }
        Ico::Search => {
            shapes.push(Shape::circle_stroke(at(-1.5, -1.5), 5.0 * s, st));
            shapes.push(line(&[(2.2, 2.2), (6.5, 6.5)]));
        }
        Ico::Plus => {
            shapes.push(line(&[(-6.0, 0.0), (6.0, 0.0)]));
            shapes.push(line(&[(0.0, -6.0), (0.0, 6.0)]));
        }
        Ico::Minus => shapes.push(line(&[(-6.0, 0.0), (6.0, 0.0)])),
        Ico::Fit => {
            for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                shapes.push(line(&[(7.0 * sx, 3.0 * sy), (7.0 * sx, 7.0 * sy), (3.0 * sx, 7.0 * sy)]));
            }
            shapes.push(Shape::rect_stroke(Rect::from_center_size(c, Vec2::splat(5.0 * s)), 1.0, st, StrokeKind::Middle));
        }
        Ico::Gear => {
            shapes.push(Shape::circle_stroke(c, 2.6 * s, st));
            let pts: Vec<Pos2> = (0..=48)
                .map(|i| {
                    let t = i as f32 / 48.0 * std::f32::consts::TAU;
                    let rr = if (i / 3) % 2 == 0 { 7.2 } else { 5.4 };
                    c + vec2(t.cos(), t.sin()) * rr * s
                })
                .collect();
            shapes.push(Shape::line(pts, st));
        }
        Ico::Help => {
            shapes.push(Shape::circle_stroke(c, 7.5 * s, st));
            shapes.push(line(&[(-2.5, -2.5), (-1.5, -4.3), (1.5, -4.3), (2.8, -2.5), (2.0, -0.8), (0.0, 0.5), (0.0, 2.0)]));
            shapes.push(Shape::circle_filled(at(0.0, 4.6), 1.1 * s, color));
        }
        Ico::Undo => {
            shapes.push(line(&[(-3.0, -7.0), (-7.0, -3.0), (-3.0, 1.0)]));
            let mut pts = vec![at(-7.0, -3.0), at(1.0, -3.0)];
            for i in 0..=12 {
                let t = -std::f32::consts::FRAC_PI_2 + i as f32 / 12.0 * std::f32::consts::PI;
                pts.push(at(1.0 + t.cos() * 5.0, 2.0 + t.sin() * 5.0));
            }
            pts.push(at(-4.0, 7.0));
            shapes.push(Shape::line(pts, st));
        }
        Ico::Clipboard => {
            shapes.push(Shape::rect_stroke(Rect::from_min_max(at(-6.0, -5.5), at(6.0, 7.5)), 2.0 * s, st, StrokeKind::Middle));
            shapes.push(Shape::rect_filled(Rect::from_min_max(at(-3.0, -7.5), at(3.0, -3.5)), 1.5 * s, color));
        }
        Ico::Trash => {
            shapes.push(line(&[(-7.0, -4.5), (7.0, -4.5)]));
            shapes.push(line(&[(-2.5, -4.5), (-2.0, -7.0), (2.0, -7.0), (2.5, -4.5)]));
            shapes.push(line(&[(-5.0, -4.5), (-4.0, 7.0), (4.0, 7.0), (5.0, -4.5)]));
        }
        Ico::Grid => {
            for (x, y) in [(-4.0f32, -4.0f32), (4.0, -4.0), (-4.0, 4.0), (4.0, 4.0)] {
                shapes.push(Shape::rect_stroke(Rect::from_center_size(at(x, y), Vec2::splat(5.5 * s)), 1.0 * s, st, StrokeKind::Middle));
            }
        }
        Ico::Map => {
            shapes.push(Shape::closed_line(
                vec![at(-7.5, -5.0), at(-2.5, -7.0), at(2.5, -5.0), at(7.5, -7.0), at(7.5, 5.0), at(2.5, 7.0), at(-2.5, 5.0), at(-7.5, 7.0)],
                st,
            ));
            shapes.push(line(&[(-2.5, -7.0), (-2.5, 5.0)]));
            shapes.push(line(&[(2.5, -5.0), (2.5, 7.0)]));
        }
        Ico::Info => {
            shapes.push(Shape::circle_stroke(c, 7.5 * s, st));
            shapes.push(line(&[(0.0, -0.5), (0.0, 4.5)]));
            shapes.push(Shape::circle_filled(at(0.0, -3.5), 1.1 * s, color));
        }
        Ico::Close => {
            shapes.push(line(&[(-5.0, -5.0), (5.0, 5.0)]));
            shapes.push(line(&[(5.0, -5.0), (-5.0, 5.0)]));
        }
    }
    p.extend(shapes);
}

/// A square icon button with hover/selected states and a tooltip.
pub fn button(ui: &mut Ui, ico: Ico, selected: bool, enabled: bool, tip: &str) -> Response {
    let size = Vec2::splat(30.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let p = ui.painter();
    let hovered = enabled && resp.hovered();
    if selected {
        p.rect_filled(rect, 7.0, theme::accent_soft(0.14));
    } else if hovered {
        p.rect_filled(rect, 7.0, Color32::from_rgb(0xEC, 0xEF, 0xF3));
    }
    let color = if !enabled {
        theme::FAINT.gamma_multiply(0.7)
    } else if selected {
        theme::ACCENT
    } else {
        theme::TEXT
    };
    paint(p, Rect::from_center_size(rect.center(), Vec2::splat(17.0)), ico, color);
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

/// Paints the small disclosure triangle used next to folder names.
pub fn chevron(p: &egui::Painter, center: Pos2, size: f32, open: bool, color: Color32) {
    let h = size * 0.5;
    let pts = if open {
        vec![center + vec2(-h, -h * 0.5), center + vec2(h, -h * 0.5), center + vec2(0.0, h * 0.6)]
    } else {
        vec![center + vec2(-h * 0.5, -h), center + vec2(h * 0.6, 0.0), center + vec2(-h * 0.5, h)]
    };
    p.add(Shape::convex_polygon(pts, color, Stroke::NONE));
}

pub fn dashed_rect(p: &egui::Painter, r: Rect, stroke: Stroke, dash: f32, gap: f32) {
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    p.extend(Shape::dashed_line(&pts, stroke, dash, gap));
}
