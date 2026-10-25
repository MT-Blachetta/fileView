//! The infinite canvas: camera, hit testing, painting and pointer gestures
//! for both the main canvas and the clipboard canvas.

use crate::app::{App, ClipItem, ClipMode, DropTarget, MenuTarget, Pane, Tool};
use crate::fsmodel;
use crate::icons;
use crate::layout::{self, COLLAPSED, Item, Kind, TILE_H, TILE_W};
use crate::theme::{self, Palette, mix};
use crate::uicons;
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align, Align2, Color32, CursorIcon, FontId, Id, LayerId, Order, Painter, PointerButton, Pos2, Rect, Stroke,
    StrokeKind, Vec2, pos2, vec2,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub const MIN_ZOOM: f32 = 0.008;
pub const MAX_ZOOM: f32 = 5.0;

// ----------------------------------------------------------------------
// Camera

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// World coordinate shown at the top-left corner of the viewport.
    pub offset: Vec2,
    pub zoom: f32,
    /// Animation target: (world point at viewport centre, zoom).
    target: Option<(Pos2, f32)>,
}

impl Default for Camera {
    fn default() -> Self {
        Camera { offset: Vec2::ZERO, zoom: 1.0, target: None }
    }
}

impl Camera {
    pub fn to_screen(self, vp: Rect, p: Pos2) -> Pos2 {
        vp.min + (p.to_vec2() - self.offset) * self.zoom
    }
    pub fn to_world(self, vp: Rect, s: Pos2) -> Pos2 {
        (self.offset + (s - vp.min) / self.zoom).to_pos2()
    }
    pub fn rect_to_screen(&self, vp: Rect, r: Rect) -> Rect {
        Rect::from_min_max(self.to_screen(vp, r.min), self.to_screen(vp, r.max))
    }
    pub fn visible_world(&self, vp: Rect) -> Rect {
        Rect::from_min_max(self.to_world(vp, vp.min), self.to_world(vp, vp.max))
    }
    pub fn target_zoom(&self) -> f32 {
        self.target.map(|t| t.1).unwrap_or(self.zoom)
    }
    pub fn zoom_at(&mut self, vp: Rect, s: Pos2, factor: f32) {
        self.target = None;
        let w = self.to_world(vp, s);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.offset = w.to_vec2() - (s - vp.min) / self.zoom;
    }
    pub fn zoom_center(&mut self, vp: Rect, factor: f32) {
        let c = self.to_world(vp, vp.center());
        let z = (self.target_zoom() * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.target = Some((self.target.map(|t| t.0).unwrap_or(c), z));
    }
    pub fn animate_to(&mut self, center: Pos2, zoom: f32) {
        self.target = Some((center, zoom.clamp(MIN_ZOOM, MAX_ZOOM)));
    }
    pub fn center_on(&mut self, vp: Rect, w: Pos2) {
        self.target = None;
        self.offset = w.to_vec2() - vp.size() / 2.0 / self.zoom;
    }
    pub fn fit(&mut self, vp: Rect, r: Rect, animate: bool) {
        if !r.is_positive() || !vp.is_positive() {
            return;
        }
        let margin = 70.0;
        let z = ((vp.width() - 2.0 * margin).max(50.0) / r.width()).min((vp.height() - 2.0 * margin).max(50.0) / r.height());
        let z = z.clamp(MIN_ZOOM, 1.25);
        if animate {
            self.animate_to(r.center(), z);
        } else {
            self.zoom = z;
            self.offset = r.center().to_vec2() - vp.size() / 2.0 / z;
        }
    }
    /// Initial view: readable zoom, aligned to the top-left of the content.
    pub fn fit_top(&mut self, vp: Rect, r: Rect) {
        let margin = 70.0;
        let z = ((vp.width() - 2.0 * margin) / r.width()).min((vp.height() - 2.0 * margin) / r.height());
        if z >= 0.55 {
            self.fit(vp, r, false);
        } else {
            self.zoom = 0.75_f32.max(z.min(1.0));
            self.offset = r.min.to_vec2() - vec2(margin + 20.0, margin) / self.zoom;
            self.target = None;
        }
    }
    /// Advances the animation. Returns true while animating.
    pub fn step(&mut self, vp: Rect, dt: f32) -> bool {
        let Some((tc, tz)) = self.target else { return false };
        if !vp.is_positive() {
            self.target = None;
            return false;
        }
        let t = 1.0 - (-dt.max(0.001) * 14.0).exp();
        let c = self.to_world(vp, vp.center());
        let lz = self.zoom.ln() + (tz.ln() - self.zoom.ln()) * t;
        let nc = c + (tc - c) * t;
        self.zoom = lz.exp();
        self.offset = nc.to_vec2() - vp.size() / 2.0 / self.zoom;
        if (self.zoom - tz).abs() < 0.0005 * tz && (nc - tc).length() * self.zoom < 0.3 {
            self.zoom = tz;
            self.offset = tc.to_vec2() - vp.size() / 2.0 / self.zoom;
            self.target = None;
        }
        true
    }
}

// ----------------------------------------------------------------------
// Hit testing

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Edge {
    Right,
    Bottom,
    Corner,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Body,
    Name,
    Chevron,
    Edge(Edge),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hit {
    pub idx: usize,
    pub part: Part,
}

pub enum Gesture {
    None,
    Pan(Pane),
    /// Middle button pressed: a click toggles select/hand, a drag pans.
    MiddlePress { pane: Pane, origin: Pos2 },
    /// Pointer pressed on an item, not yet moved far enough to start a drag.
    Press { pane: Pane, origin: Pos2, path: PathBuf, modified: bool },
    Drag { pane: Pane, items: Vec<PathBuf>, last_world: Pos2 },
    Marquee { pane: Pane, start: Pos2, cur: Pos2, base: BTreeSet<PathBuf> },
    Resize { path: PathBuf, edge: Edge },
    Draw { parent: PathBuf, start: Pos2, cur: Pos2 },
}

const DRAG_THRESHOLD: f32 = 5.0;

/// World rectangle of an item on the clipboard canvas (label included for folders).
pub fn clip_item_rect(c: &ClipItem) -> Rect {
    if c.info.is_dir {
        Rect::from_min_size(c.pos, vec2(COLLAPSED.x, COLLAPSED.y + layout::LABEL_H))
    } else {
        Rect::from_min_size(c.pos, vec2(TILE_W, TILE_H))
    }
}

fn clip_body_rect(c: &ClipItem) -> Rect {
    let r = clip_item_rect(c);
    if c.info.is_dir { Rect::from_min_max(pos2(r.left(), r.top() + layout::LABEL_H), r.max) } else { r }
}

impl App {
    fn cam_of(&self, pane: Pane) -> (&crate::canvas::Camera, Rect) {
        match pane {
            Pane::Main => (&self.cam, self.main_rect),
            Pane::Clip => (&self.clip_cam, self.clip_rect),
        }
    }

    fn cam_mut(&mut self, pane: Pane) -> (&mut Camera, Rect) {
        match pane {
            Pane::Main => (&mut self.cam, self.main_rect),
            Pane::Clip => (&mut self.clip_cam, self.clip_rect),
        }
    }

    pub fn hit_main(&self, w: Pos2) -> Option<Hit> {
        let tol = 5.0 / self.cam.zoom;
        for (i, it) in self.layout.items.iter().enumerate().rev() {
            match it.kind {
                Kind::Dir => {
                    let r = it.rect;
                    if !it.collapsed {
                        let near_r = (w.x - r.right()).abs() <= tol && w.y >= r.top() && w.y <= r.bottom() + tol;
                        let near_b = (w.y - r.bottom()).abs() <= tol && w.x >= r.left() && w.x <= r.right() + tol;
                        if near_r && near_b || (near_r || near_b) && (w - r.right_bottom()).length() < 3.0 * tol {
                            return Some(Hit { idx: i, part: Part::Edge(Edge::Corner) });
                        } else if near_r {
                            return Some(Hit { idx: i, part: Part::Edge(Edge::Right) });
                        } else if near_b {
                            return Some(Hit { idx: i, part: Part::Edge(Edge::Bottom) });
                        }
                    }
                    if it.label.contains(w) {
                        let chev = 18.0 * label_scale(self.cam.zoom, it.label.width() * self.cam.zoom) / self.cam.zoom;
                        let part = if w.x < it.label.left() + chev { Part::Chevron } else { Part::Name };
                        return Some(Hit { idx: i, part });
                    }
                    if r.contains(w) {
                        return Some(Hit { idx: i, part: Part::Body });
                    }
                }
                Kind::File | Kind::More(_) => {
                    if it.rect.contains(w) {
                        let part = if it.label.contains(w) && it.kind == Kind::File { Part::Name } else { Part::Body };
                        return Some(Hit { idx: i, part });
                    }
                }
            }
        }
        None
    }

    fn item_path(&self, idx: usize) -> &Path {
        &self.tree.nodes[self.layout.items[idx].node].path
    }

    pub fn hit_clip(&self, w: Pos2) -> Option<usize> {
        self.clip.iter().enumerate().rev().find(|(_, c)| clip_item_rect(c).contains(w)).map(|(i, _)| i)
    }

    /// Deepest folder on the main canvas containing `w` that is not one of
    /// (or inside one of) `exclude`.
    fn dir_at(&self, w: Pos2, exclude: &[PathBuf]) -> Option<usize> {
        let mut i = 0;
        let mut best = None;
        let items = &self.layout.items;
        while i < items.len() {
            let it = &items[i];
            if it.kind != Kind::Dir || !it.outer().contains(w) {
                i = if it.kind == Kind::Dir { it.end } else { i + 1 };
                continue;
            }
            let p = self.item_path(i);
            if exclude.iter().any(|e| p.starts_with(e)) {
                i = it.end;
                continue;
            }
            best = Some(i);
            i += 1;
        }
        best
    }

    pub fn drop_target_at(&self, s: Pos2, exclude: &[PathBuf]) -> DropTarget {
        if self.clip_rect.is_positive() && self.clip_rect.contains(s) {
            return DropTarget::Clip(self.clip_cam.to_world(self.clip_rect, s));
        }
        if self.main_rect.contains(s) {
            let w = self.cam.to_world(self.main_rect, s);
            if exclude.iter().any(|e| e == &self.root) {
                return DropTarget::None;
            }
            return match self.dir_at(w, exclude) {
                Some(i) => DropTarget::Dir(self.item_path(i).to_path_buf()),
                None => DropTarget::Dir(self.root.clone()),
            };
        }
        DropTarget::None
    }

    pub fn main_menu_target(&mut self, s: Option<Pos2>) -> MenuTarget {
        self.focus = Pane::Main;
        let Some(s) = s else { return MenuTarget::Background };
        let w = self.cam.to_world(self.main_rect, s);
        match self.hit_main(w) {
            Some(h) if !matches!(self.layout.items[h.idx].kind, Kind::More(_)) => {
                let p = self.item_path(h.idx).to_path_buf();
                if !self.sel.contains(&p) {
                    self.sel.clear();
                    self.sel.insert(p.clone());
                }
                MenuTarget::Item(p)
            }
            _ => {
                self.sel.clear();
                MenuTarget::Background
            }
        }
    }

    pub fn clip_menu_target(&mut self, s: Option<Pos2>) -> MenuTarget {
        self.focus = Pane::Clip;
        let Some(s) = s else { return MenuTarget::ClipBackground };
        let w = self.clip_cam.to_world(self.clip_rect, s);
        match self.hit_clip(w) {
            Some(i) => {
                let p = self.clip[i].path.clone();
                if !self.clip_sel.contains(&p) {
                    self.clip_sel.clear();
                    self.clip_sel.insert(p.clone());
                }
                MenuTarget::ClipItem(p)
            }
            None => MenuTarget::ClipBackground,
        }
    }

    // ------------------------------------------------------------------
    // Input

    /// Wheel zoom / pan for a canvas pane.
    pub fn canvas_view_input(&mut self, ctx: &egui::Context, pane: Pane, resp: &egui::Response) {
        if !resp.contains_pointer() {
            return;
        }
        let (scroll, zoom, hover, mods) =
            ctx.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.pointer.hover_pos(), i.modifiers));
        let wheel_zooms = self.settings.wheel_zooms;
        let (cam, vp) = self.cam_mut(pane);
        let Some(hp) = hover else { return };
        if zoom != 1.0 {
            cam.zoom_at(vp, hp, zoom);
        }
        if scroll != Vec2::ZERO {
            if wheel_zooms && !mods.shift {
                let f = (scroll.y / 300.0).exp();
                cam.zoom_at(vp, hp, f);
                if scroll.x != 0.0 {
                    cam.offset.x -= scroll.x / cam.zoom;
                }
            } else {
                let d = if mods.shift && scroll.x == 0.0 { vec2(scroll.y, 0.0) } else { scroll };
                cam.target = None;
                cam.offset -= d / cam.zoom;
            }
        }
    }

    /// Starts gestures on primary / middle press.
    pub fn canvas_press(&mut self, ctx: &egui::Context, pane: Pane, resp: &egui::Response) {
        let (pressed, middle, pos, mods, space) = ctx.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.button_pressed(PointerButton::Middle),
                i.pointer.interact_pos(),
                i.modifiers,
                i.key_down(egui::Key::Space) && self.settings.toggle_key != "Space",
            )
        });
        let hovered = resp.contains_pointer() && resp.hovered();
        // Cursor feedback while hovering.
        if hovered && matches!(self.gesture, Gesture::None) {
            self.update_hover(ctx, pane, pos, space);
        }
        if !hovered || !matches!(self.gesture, Gesture::None) {
            return;
        }
        let Some(s) = pos else { return };
        if middle {
            self.gesture = Gesture::MiddlePress { pane, origin: s };
            return;
        }
        if pressed && (space || self.tool == Tool::Pan) {
            self.gesture = Gesture::Pan(pane);
            return;
        }
        if !pressed {
            return;
        }
        if self.rename.is_some() {
            return; // the rename field commits on focus loss
        }
        self.focus = pane;
        let additive = mods.command || mods.shift;
        let (cam, vp) = self.cam_of(pane);
        let w = cam.to_world(vp, s);
        match pane {
            Pane::Main => {
                let hit = self.hit_main(w);
                if self.tool == Tool::DrawFolder && !matches!(hit.map(|h| h.part), Some(Part::Edge(_))) {
                    let parent = self.dir_at(w, &[]).map(|i| self.item_path(i).to_path_buf()).unwrap_or(self.root.clone());
                    self.gesture = Gesture::Draw { parent, start: w, cur: w };
                    return;
                }
                match hit {
                    Some(Hit { idx, part: Part::Edge(edge) }) => {
                        let p = self.item_path(idx).to_path_buf();
                        self.gesture = Gesture::Resize { path: p, edge };
                    }
                    Some(Hit { idx, part: Part::Chevron }) => {
                        let p = self.item_path(idx).to_path_buf();
                        self.toggle_expand(&p);
                    }
                    Some(Hit { idx, .. }) if matches!(self.layout.items[idx].kind, Kind::More(_)) => {
                        let p = self.item_path(idx).to_path_buf();
                        self.toast(format!("Open \u{201c}{}\u{201d} as root to see all files", fsmodel::display_name(&p)), crate::app::ToastKind::Info);
                    }
                    Some(Hit { idx, part }) => {
                        let p = self.item_path(idx).to_path_buf();
                        let is_dir = self.layout.items[idx].kind == Kind::Dir;
                        // Pressing on the empty body of a folder starts a rubber band
                        // inside it (like on the desktop), unless the folder is selected.
                        if is_dir && part == Part::Body && !self.sel.contains(&p) && !additive {
                            self.start_marquee(pane, w, additive);
                            return;
                        }
                        if additive {
                            if !self.sel.remove(&p) {
                                self.sel.insert(p.clone());
                            }
                        } else if !self.sel.contains(&p) {
                            self.sel.clear();
                            self.sel.insert(p.clone());
                        }
                        self.clip_sel.clear();
                        self.gesture = Gesture::Press { pane, origin: s, path: p, modified: additive };
                    }
                    None => self.start_marquee(pane, w, additive),
                }
            }
            Pane::Clip => match self.hit_clip(w) {
                Some(i) => {
                    let p = self.clip[i].path.clone();
                    if additive {
                        if !self.clip_sel.remove(&p) {
                            self.clip_sel.insert(p.clone());
                        }
                    } else if !self.clip_sel.contains(&p) {
                        self.clip_sel.clear();
                        self.clip_sel.insert(p.clone());
                    }
                    // Bring to front.
                    let item = self.clip.remove(i);
                    self.clip.push(item);
                    self.gesture = Gesture::Press { pane, origin: s, path: p, modified: additive };
                }
                None => self.start_marquee(pane, w, additive),
            },
        }
    }

    fn start_marquee(&mut self, pane: Pane, w: Pos2, additive: bool) {
        let base = match (pane, additive) {
            (_, false) => BTreeSet::new(),
            (Pane::Main, true) => self.sel.clone(),
            (Pane::Clip, true) => self.clip_sel.clone(),
        };
        if !additive {
            match pane {
                Pane::Main => self.sel.clear(),
                Pane::Clip => self.clip_sel.clear(),
            }
        }
        self.gesture = Gesture::Marquee { pane, start: w, cur: w, base };
    }

    fn update_hover(&mut self, ctx: &egui::Context, pane: Pane, pos: Option<Pos2>, space: bool) {
        self.hover = None;
        self.hover_clip = None;
        let Some(s) = pos else { return };
        if space || self.tool == Tool::Pan {
            ctx.set_cursor_icon(CursorIcon::Grab);
            return;
        }
        match pane {
            Pane::Main => {
                let w = self.cam.to_world(self.main_rect, s);
                self.hover = self.hit_main(w);
                if self.tool == Tool::DrawFolder && !matches!(self.hover.map(|h| h.part), Some(Part::Edge(_))) {
                    ctx.set_cursor_icon(CursorIcon::Crosshair);
                    return;
                }
                match self.hover.map(|h| h.part) {
                    Some(Part::Edge(Edge::Right)) => ctx.set_cursor_icon(CursorIcon::ResizeHorizontal),
                    Some(Part::Edge(Edge::Bottom)) => ctx.set_cursor_icon(CursorIcon::ResizeVertical),
                    Some(Part::Edge(Edge::Corner)) => ctx.set_cursor_icon(CursorIcon::ResizeNwSe),
                    Some(Part::Chevron) => ctx.set_cursor_icon(CursorIcon::PointingHand),
                    _ => {}
                }
            }
            Pane::Clip => {
                let w = self.clip_cam.to_world(self.clip_rect, s);
                self.hover_clip = self.hit_clip(w);
            }
        }
    }

    pub fn canvas_double_click(&mut self, ctx: &egui::Context, resp: &egui::Response) {
        if !resp.double_clicked() || self.tool != Tool::Select {
            return;
        }
        let Some(s) = resp.interact_pointer_pos() else { return };
        let w = self.cam.to_world(self.main_rect, s);
        let Some(hit) = self.hit_main(w) else { return };
        let it = &self.layout.items[hit.idx];
        if matches!(it.kind, Kind::More(_)) {
            return;
        }
        let p = self.item_path(hit.idx).to_path_buf();
        match (it.kind, hit.part) {
            (_, Part::Name) => {
                self.sel = [p.clone()].into_iter().collect();
                self.start_rename(&p);
            }
            (Kind::File, _) => self.open_item(ctx, &p),
            (Kind::Dir, Part::Body) => self.set_root(ctx, p, true),
            (Kind::Dir, Part::Edge(_)) => {
                self.custom_sizes.remove(&p);
                self.layout_dirty = true;
            }
            _ => {}
        }
    }

    /// Updates / finishes the active gesture. Runs at the start of the frame.
    pub fn update_gesture(&mut self, ctx: &egui::Context) {
        if matches!(self.gesture, Gesture::None) {
            self.drop = DropTarget::None;
            return;
        }
        let (pos, primary, middle, delta, mods) = ctx.input(|i| {
            (i.pointer.interact_pos(), i.pointer.primary_down(), i.pointer.button_down(PointerButton::Middle), i.pointer.delta(), i.modifiers)
        });
        let down = primary || middle;
        let gesture = std::mem::replace(&mut self.gesture, Gesture::None);
        self.gesture = match gesture {
            Gesture::Pan(pane) => {
                if down {
                    ctx.set_cursor_icon(CursorIcon::Grabbing);
                    let (cam, _) = self.cam_mut(pane);
                    cam.target = None;
                    cam.offset -= delta / cam.zoom;
                    Gesture::Pan(pane)
                } else {
                    Gesture::None
                }
            }
            Gesture::MiddlePress { pane, origin } => {
                if !middle {
                    self.toggle_mode();
                    Gesture::None
                } else if pos.is_some_and(|p| p.distance(origin) > DRAG_THRESHOLD) {
                    Gesture::Pan(pane)
                } else {
                    Gesture::MiddlePress { pane, origin }
                }
            }
            Gesture::Press { pane, origin, path, modified } => {
                if !down {
                    // A plain click on an item of a multi-selection selects only it.
                    if !modified {
                        match pane {
                            Pane::Main => self.sel = [path].into_iter().collect(),
                            Pane::Clip => self.clip_sel = [path].into_iter().collect(),
                        }
                    }
                    Gesture::None
                } else if pos.is_some_and(|p| p.distance(origin) > DRAG_THRESHOLD) {
                    let items: Vec<PathBuf> = match pane {
                        Pane::Main => top_level(&self.sel),
                        Pane::Clip => self.clip_sel.iter().cloned().collect(),
                    };
                    let (cam, vp) = self.cam_of(pane);
                    let last_world = cam.to_world(vp, pos.unwrap());
                    Gesture::Drag { pane, items, last_world }
                } else {
                    Gesture::Press { pane, origin, path, modified }
                }
            }
            Gesture::Drag { pane, items, last_world } => {
                let s = pos.unwrap_or(Pos2::ZERO);
                let target = self.drop_target_at(s, if pane == Pane::Main { &items } else { &[] });
                // Dragging within the clipboard canvas just repositions items.
                let within_clip = pane == Pane::Clip && matches!(target, DropTarget::Clip(_));
                if down {
                    ctx.set_cursor_icon(if within_clip { CursorIcon::Grabbing } else if mods.command { CursorIcon::Copy } else { CursorIcon::Grabbing });
                    let mut lw = last_world;
                    if within_clip {
                        let w = self.clip_cam.to_world(self.clip_rect, s);
                        let d = w - last_world;
                        for c in &mut self.clip {
                            if items.contains(&c.path) {
                                c.pos += d;
                            }
                        }
                        lw = w;
                    }
                    self.autopan(s);
                    self.drop = if within_clip { DropTarget::None } else { target };
                    Gesture::Drag { pane, items, last_world: lw }
                } else {
                    self.drop = DropTarget::None;
                    if !within_clip {
                        self.perform_drop(ctx, pane, &items, target, mods.command);
                    }
                    Gesture::None
                }
            }
            Gesture::Marquee { pane, start, cur, base } => {
                if down {
                    let (cam, vp) = self.cam_of(pane);
                    let cur = pos.map(|p| cam.to_world(vp, p)).unwrap_or(cur);
                    let r = Rect::from_two_pos(start, cur);
                    let mut set = base.clone();
                    match pane {
                        Pane::Main => {
                            set.extend(self.items_in_rect(r));
                            self.sel = set;
                        }
                        Pane::Clip => {
                            set.extend(self.clip.iter().filter(|c| r.contains_rect(clip_item_rect(c))).map(|c| c.path.clone()));
                            self.clip_sel = set;
                        }
                    }
                    if pane == Pane::Main
                        && let Some(p) = pos {
                            self.autopan(p);
                        }
                    Gesture::Marquee { pane, start, cur, base }
                } else {
                    Gesture::None
                }
            }
            Gesture::Resize { path, edge } => {
                if down {
                    ctx.set_cursor_icon(match edge {
                        Edge::Right => CursorIcon::ResizeHorizontal,
                        Edge::Bottom => CursorIcon::ResizeVertical,
                        Edge::Corner => CursorIcon::ResizeNwSe,
                    });
                    if let (Some(p), Some(it)) = (pos, self.layout.get(&path)) {
                        let w = self.cam.to_world(self.main_rect, p);
                        let body = it.rect;
                        let old = self.custom_sizes.get(&path).copied().unwrap_or(Vec2::ZERO);
                        let min_w = TILE_W + 2.0 * layout::PAD;
                        let nw = (w.x - body.left()).max(min_w);
                        let nh = (w.y - body.top()).max(40.0);
                        let new = match edge {
                            Edge::Right => vec2(nw, old.y),
                            Edge::Bottom => vec2(if old.x > 0.0 { old.x } else { body.width() }, nh),
                            Edge::Corner => vec2(nw, nh),
                        };
                        if new != old {
                            self.custom_sizes.insert(path.clone(), new);
                            self.layout_dirty = true;
                        }
                        self.autopan(p);
                    }
                    Gesture::Resize { path, edge }
                } else {
                    Gesture::None
                }
            }
            Gesture::Draw { parent, start, cur } => {
                let cur = pos.map(|p| self.cam.to_world(self.main_rect, p)).unwrap_or(cur);
                if down {
                    ctx.set_cursor_icon(CursorIcon::Crosshair);
                    Gesture::Draw { parent, start, cur }
                } else {
                    let r = Rect::from_two_pos(start, cur);
                    let size = if r.width() > 60.0 && r.height() > 40.0 { Some(r.size()) } else { None };
                    self.new_folder(ctx, &parent, size);
                    self.tool = Tool::Select;
                    Gesture::None
                }
            }
            Gesture::None => Gesture::None,
        };
    }

    /// Topmost items fully inside world rect `r` (folders count as a whole).
    fn items_in_rect(&self, r: Rect) -> Vec<PathBuf> {
        let mut out = vec![];
        let items = &self.layout.items;
        let mut i = 0;
        while i < items.len() {
            let it = &items[i];
            let outer = if it.kind == Kind::Dir { it.outer() } else { it.rect.shrink2(vec2(14.0, 8.0)) };
            if !matches!(it.kind, Kind::More(_)) && r.contains_rect(outer) {
                out.push(self.item_path(i).to_path_buf());
                i = it.end;
            } else if it.kind == Kind::Dir && !r.intersects(it.outer()) {
                i = it.end;
            } else {
                i += 1;
            }
        }
        out
    }

    fn autopan(&mut self, s: Pos2) {
        let vp = self.main_rect;
        if !vp.contains(s) {
            return;
        }
        let edge = 36.0;
        let mut d = Vec2::ZERO;
        if s.x < vp.left() + edge {
            d.x = -(vp.left() + edge - s.x);
        } else if s.x > vp.right() - edge {
            d.x = s.x - (vp.right() - edge);
        }
        if s.y < vp.top() + edge {
            d.y = -(vp.top() + edge - s.y);
        } else if s.y > vp.bottom() - edge {
            d.y = s.y - (vp.bottom() - edge);
        }
        if d != Vec2::ZERO {
            self.cam.offset += d * 0.25 / self.cam.zoom;
        }
    }

    fn perform_drop(&mut self, ctx: &egui::Context, pane: Pane, items: &[PathBuf], target: DropTarget, ctrl: bool) {
        match (pane, target) {
            (Pane::Main, DropTarget::Clip(w)) => {
                let mode = if ctrl { ClipMode::Copy } else { ClipMode::Move };
                self.add_to_clip(items, mode, Some(w));
                self.focus = Pane::Clip;
            }
            (Pane::Main, DropTarget::Dir(d)) => self.transfer(ctx, items, &d, ctrl),
            (Pane::Clip, DropTarget::Dir(d)) => {
                let (copies, moves): (Vec<PathBuf>, Vec<PathBuf>) = items.iter().cloned().partition(|p| {
                    ctrl || self.clip.iter().any(|c| &c.path == p && c.mode == ClipMode::Copy)
                });
                self.transfer(ctx, &moves, &d, false);
                self.transfer(ctx, &copies, &d, true);
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------
    // Painting: main canvas

    pub fn paint_main(&self, p: &Painter, vp: Rect) {
        let pal = Palette::for_bg(self.settings.canvas_bg);
        p.rect_filled(vp, 0.0, pal.bg);
        if self.settings.dot_grid {
            paint_dots(p, vp, &self.cam, pal.dot);
        }
        let z = self.cam.zoom;
        let vis = self.cam.visible_world(vp).expand(30.0 / z);
        let items = &self.layout.items;
        let searching = !self.search.trim().is_empty();
        let mut labels: Vec<usize> = vec![];
        let mut i = 0;
        while i < items.len() {
            let it = &items[i];
            if !vis.intersects(it.outer()) {
                i = if it.kind == Kind::Dir { it.end } else { i + 1 };
                continue;
            }
            let node = &self.tree.nodes[it.node];
            let mut look = self.look_for(i, &node.path, searching);
            look.hidden = node.hidden;
            match it.kind {
                Kind::Dir => {
                    let body = self.cam.rect_to_screen(vp, it.rect);
                    if body.width() < 5.0 || body.height() < 5.0 {
                        p.rect_filled(body, 0.0, pal.ink.gamma_multiply(0.35 * look.alpha()));
                        i = it.end;
                        continue;
                    }
                    let content = if !node.loaded {
                        DirContent::Collapsed(node.entry_count, node.error.is_some())
                    } else if let Some(e) = &node.error {
                        DirContent::Error(e.clone())
                    } else if node.dirs.is_empty() && node.files.is_empty() {
                        DirContent::Empty
                    } else {
                        DirContent::Normal
                    };
                    paint_dir_body(p, body, z, it.depth, &look, &content, it.custom_size);
                    labels.push(i);
                }
                Kind::File => {
                    let hide = self.rename.as_ref().is_some_and(|r| r.path == node.path);
                    paint_file(p, &self.cam, vp, it, &node.name, &look, hide);
                    if node.is_symlink && z > 0.3 {
                        paint_link_badge(p, self.cam.rect_to_screen(vp, it.icon_rect()));
                    }
                }
                Kind::More(n) => {
                    let r = self.cam.rect_to_screen(vp, it.rect.shrink(8.0));
                    icons::paint_more_tile(p, r, n, z);
                }
            }
            i += 1;
        }
        for i in labels {
            let it = &items[i];
            let node = &self.tree.nodes[it.node];
            if self.rename.as_ref().is_some_and(|r| r.path == node.path) {
                continue;
            }
            let look = self.look_for(i, &node.path, searching);
            let lab = self.cam.rect_to_screen(vp, it.label);
            paint_dir_label(p, lab, z, &node.name, node.entry_count, !it.collapsed, &look);
        }

        // Drop highlight for the root (the canvas itself).
        if let DropTarget::Dir(d) = &self.drop
            && d == &self.root {
                p.rect_stroke(vp.shrink(3.0), 6.0, Stroke::new(3.0, theme::accent_soft(0.6)), StrokeKind::Inside);
            }
        match &self.gesture {
            Gesture::Marquee { pane: Pane::Main, start, cur, .. } => {
                let r = self.cam.rect_to_screen(vp, Rect::from_two_pos(*start, *cur));
                p.rect(r, 2.0, theme::accent_soft(0.08), Stroke::new(1.0, theme::ACCENT), StrokeKind::Middle);
            }
            Gesture::Draw { start, cur, .. } => {
                let r = self.cam.rect_to_screen(vp, Rect::from_two_pos(*start, *cur));
                p.rect_filled(r, 6.0 * z.min(1.0), pal.backdrop);
                uicons::dashed_rect(p, r, Stroke::new(1.5, pal.ink), 6.0, 4.0);
                p.text(r.left_top() + vec2(0.0, -4.0), Align2::LEFT_BOTTOM, "New folder", theme::semibold(12.0), pal.ink);
                if r.width() > 60.0 && r.height() > 30.0 {
                    let wz = r.size() / z;
                    p.text(r.center(), Align2::CENTER_CENTER, format!("{:.0} \u{00D7} {:.0}", wz.x, wz.y), FontId::proportional(11.0), pal.muted);
                }
            }
            _ => {}
        }
        if self.layout.items.is_empty() && self.is_scanning() {
            p.text(vp.center(), Align2::CENTER_CENTER, "Reading folder\u{2026}", FontId::proportional(14.0), pal.muted);
        }
    }

    fn look_for(&self, idx: usize, path: &Path, searching: bool) -> Look {
        let in_clip_move = self.clip.iter().any(|c| c.mode == ClipMode::Move && path.starts_with(&c.path));
        let matched = searching && self.matches.contains(path);
        let dragging = matches!(&self.gesture, Gesture::Drag { pane: Pane::Main, items, .. } if items.iter().any(|d| path.starts_with(d)));
        Look {
            selected: self.sel.contains(path),
            hovered: self.hover.map(|h| h.idx) == Some(idx) && matches!(self.gesture, Gesture::None),
            drop: matches!(&self.drop, DropTarget::Dir(d) if d == path),
            ghost: in_clip_move || dragging,
            dim: searching && !matched && !self.match_ancestors.contains(path),
            hidden: false,
            matched,
            mode_badge: None,
            missing: false,
            pal: Palette::for_bg(self.settings.canvas_bg),
        }
    }

    // ------------------------------------------------------------------
    // Painting: clipboard canvas

    pub fn paint_clip(&self, p: &Painter, vp: Rect) {
        let pal = Palette::for_bg(self.settings.canvas_bg);
        p.rect_filled(vp, 0.0, pal.bg);
        if self.settings.dot_grid {
            paint_dots(p, vp, &self.clip_cam, pal.dot);
        }
        let z = self.clip_cam.zoom;
        if self.clip.is_empty() {
            let c = vp.center();
            let card = Rect::from_center_size(c, vec2((vp.width() - 40.0).clamp(120.0, 240.0), 150.0));
            uicons::dashed_rect(p, card, Stroke::new(1.2, pal.faint), 6.0, 4.0);
            let r = Rect::from_center_size(card.center() - vec2(0.0, 34.0), Vec2::splat(30.0));
            uicons::paint(p, r, uicons::Ico::Clipboard, pal.faint);
            let wrap = card.width() - 24.0;
            let g = p.layout(
                "Drag files and folders here to stage them.\nThen drag them onto any folder on the canvas.".into(),
                FontId::proportional(12.0),
                pal.muted,
                wrap,
            );
            let gs = g.size();
            p.galley(pos2(card.center().x - gs.x / 2.0, card.center().y - 6.0), g, pal.muted);
        }
        for (i, c) in self.clip.iter().enumerate() {
            let selected = self.clip_sel.contains(&c.path);
            let look = Look {
                selected,
                hovered: self.hover_clip == Some(i),
                drop: false,
                ghost: false,
                dim: false,
                matched: false,
                mode_badge: Some(c.mode),
                missing: !c.info.exists,
                hidden: false,
                pal,
            };
            if c.info.is_dir {
                let body = self.clip_cam.rect_to_screen(vp, clip_body_rect(c));
                let lab = Rect::from_min_max(pos2(body.left(), body.top() - layout::LABEL_H * z), pos2(body.right(), body.top()));
                paint_dir_body(p, body, z, 1, &look, &DirContent::Collapsed(c.info.entries, false), false);
                paint_dir_label(p, lab, z, &c.info.name, c.info.entries, false, &look);
                paint_mode_badge(p, body.right_top() + vec2(-6.0, 6.0), c.mode, z, !c.info.exists);
            } else {
                let r = clip_item_rect(c);
                let it = Item {
                    node: 0,
                    kind: Kind::File,
                    rect: r,
                    label: Rect::from_min_max(pos2(r.left() + 3.0, r.top() + 6.0 + layout::ICON + 5.0), pos2(r.right() - 3.0, r.bottom() - 2.0)),
                    depth: 1,
                    end: 0,
                    collapsed: false,
                    custom_size: false,
                };
                paint_file(p, &self.clip_cam, vp, &it, &c.info.name, &look, false);
                let sr = self.clip_cam.rect_to_screen(vp, r);
                paint_mode_badge(p, sr.right_top() + vec2(-4.0, 4.0), c.mode, z, !c.info.exists);
            }
        }
        if let Gesture::Marquee { pane: Pane::Clip, start, cur, .. } = &self.gesture {
            let r = self.clip_cam.rect_to_screen(vp, Rect::from_two_pos(*start, *cur));
            p.rect(r, 2.0, theme::accent_soft(0.08), Stroke::new(1.0, theme::ACCENT), StrokeKind::Middle);
        }
        if let DropTarget::Clip(_) = self.drop {
            p.rect_stroke(vp.shrink(3.0), 8.0, Stroke::new(2.5, theme::accent_soft(0.7)), StrokeKind::Inside);
            p.rect_filled(vp.shrink(3.0), 8.0, theme::accent_soft(0.05));
        }
    }

    /// Floating preview of dragged items next to the cursor.
    pub fn paint_drag_ghost(&self, ctx: &egui::Context) {
        let Gesture::Drag { pane, items, .. } = &self.gesture else { return };
        let Some(s) = ctx.input(|i| i.pointer.interact_pos()) else { return };
        if *pane == Pane::Clip && self.clip_rect.contains(s) {
            return;
        }
        let p = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::unique("drag-ghost")));
        let ctrl = ctx.input(|i| i.modifiers.command);
        let origin = s + vec2(14.0, 14.0);
        for (k, path) in items.iter().take(3).enumerate().rev() {
            let r = Rect::from_min_size(origin + vec2(k as f32 * 5.0, k as f32 * 5.0), vec2(56.0, 56.0));
            p.rect(r, 8.0, Color32::from_white_alpha(235), Stroke::new(1.0, theme::LINE), StrokeKind::Inside);
            if path.is_dir() {
                icons::paint_folder_icon(&p, r.shrink(10.0), 1.0, false);
            } else {
                icons::paint_file_icon(&p, r.shrink(8.0), &fsmodel::display_name(path), 1.0);
            }
        }
        if items.len() > 1 {
            let c = origin + vec2(56.0, 0.0);
            p.circle_filled(c, 10.0, theme::ACCENT);
            p.text(c, Align2::CENTER_CENTER, items.len().to_string(), theme::semibold(11.0), Color32::WHITE);
        }
        let copy = ctrl || (*pane == Pane::Clip && items.iter().all(|i| self.clip.iter().any(|c| &c.path == i && c.mode == ClipMode::Copy)));
        let text = match &self.drop {
            DropTarget::Dir(d) => {
                let verb = if copy { "Copy to" } else { "Move to" };
                format!("{verb} {}", fsmodel::display_name(d))
            }
            DropTarget::Clip(_) => if ctrl { "Stage copy in clipboard".into() } else { "Stage in clipboard".into() },
            DropTarget::None => "Cannot drop here".into(),
        };
        let g = p.layout_no_wrap(text, FontId::proportional(12.0), Color32::WHITE);
        let pill = Rect::from_min_size(origin + vec2(0.0, 72.0), g.size() + vec2(16.0, 8.0));
        let bg = if matches!(self.drop, DropTarget::None) { theme::DANGER } else { theme::INK };
        p.rect_filled(pill, 6.0, bg);
        p.galley(pill.min + vec2(8.0, 4.0), g, Color32::WHITE);
    }
}

/// Removes items whose ancestor is also in the set (moving a folder moves its contents).
fn top_level(set: &BTreeSet<PathBuf>) -> Vec<PathBuf> {
    set.iter().filter(|p| !set.iter().any(|q| q != *p && p.starts_with(q))).cloned().collect()
}

// ----------------------------------------------------------------------
// Primitive painters shared by both canvases

pub struct Look {
    pub selected: bool,
    pub hovered: bool,
    pub drop: bool,
    /// Cut (pending move) or currently being dragged.
    pub ghost: bool,
    pub dim: bool,
    pub matched: bool,
    pub mode_badge: Option<ClipMode>,
    pub missing: bool,
    /// Hidden file (drawn faded, like in Explorer).
    pub hidden: bool,
    pub pal: Palette,
}

impl Look {
    fn alpha(&self) -> f32 {
        if self.dim {
            0.28
        } else if self.ghost || self.missing {
            0.5
        } else if self.hidden {
            0.6
        } else {
            1.0
        }
    }
}

pub enum DirContent {
    Normal,
    Empty,
    Collapsed(Option<usize>, bool),
    Error(String),
}

fn quant(px: f32) -> f32 {
    (px * 2.0).round() / 2.0
}

/// Scale factor of folder labels: labels of folders that are still reasonably
/// large on screen never get smaller than ~10px, so folder names stay readable
/// when zoomed far out. Labels of tiny folders simply scale with the canvas.
fn label_scale(z: f32, screen_w: f32) -> f32 {
    if screen_w >= 56.0 { z.max(0.78) } else { z }
}

fn paint_dots(p: &Painter, vp: Rect, cam: &Camera, dot: Color32) {
    let mut step = 24.0;
    while step * cam.zoom < 14.0 {
        step *= 2.0;
    }
    let vis = cam.visible_world(vp);
    let x0 = (vis.left() / step).floor() as i64;
    let x1 = (vis.right() / step).ceil() as i64;
    let y0 = (vis.top() / step).floor() as i64;
    let y1 = (vis.bottom() / step).ceil() as i64;
    if (x1 - x0) * (y1 - y0) > 40_000 {
        return;
    }
    let r = (1.1 * cam.zoom.sqrt()).clamp(0.7, 1.4);
    for gx in x0..=x1 {
        for gy in y0..=y1 {
            let s = cam.to_screen(vp, pos2(gx as f32 * step, gy as f32 * step));
            p.rect_filled(Rect::from_center_size(s, Vec2::splat(r * 2.0)), r, dot);
        }
    }
}

pub fn paint_dir_body(p: &Painter, r: Rect, z: f32, depth: u16, look: &Look, content: &DirContent, custom: bool) {
    let a = look.alpha();
    let pal = look.pal;
    let rad = (6.0 * z).clamp(1.0, 8.0);
    if depth <= 1 && z > 0.12 && !look.ghost {
        p.rect_filled(r.translate(vec2(0.0, 2.0 * z.min(1.0))), rad, Color32::from_black_alpha((14.0 * a) as u8));
    }
    // Folder areas share the canvas background colour.
    let mut fill = pal.bg;
    if look.drop {
        fill = mix(fill, theme::ACCENT, 0.09);
    } else if look.selected {
        fill = mix(fill, theme::ACCENT, 0.035);
    }
    let border_w = (1.3 * z.sqrt()).clamp(1.0, 1.7);
    let stroke = if look.drop {
        Stroke::new(2.2, theme::ACCENT)
    } else if look.selected {
        Stroke::new(2.0, theme::ACCENT)
    } else if look.hovered {
        Stroke::new(border_w + 0.6, pal.ink)
    } else {
        Stroke::new(border_w, pal.ink.gamma_multiply(a))
    };
    if look.selected || look.drop {
        p.rect_stroke(r.expand(2.5), rad + 2.5, Stroke::new(3.5, theme::accent_soft(0.16)), StrokeKind::Outside);
    }
    if look.ghost {
        p.rect_filled(r, rad, fill.gamma_multiply(0.75));
        uicons::dashed_rect(p, r, Stroke::new(1.3, pal.ink.gamma_multiply(0.55)), 6.0, 4.0);
    } else {
        p.rect(r, rad, fill.gamma_multiply(if look.dim { 0.6 } else { 1.0 }), stroke, StrokeKind::Middle);
    }
    let font = 12.0 * z;
    match content {
        DirContent::Normal => {}
        DirContent::Empty if font >= 6.0 => {
            p.text(r.center(), Align2::CENTER_CENTER, "Empty folder", FontId::proportional(quant(font)), pal.faint.gamma_multiply(a));
        }
        DirContent::Error(e) if font >= 6.0 => {
            let msg = if e.to_lowercase().contains("denied") { "Access denied" } else { "Cannot read folder" };
            p.text(r.center(), Align2::CENTER_CENTER, msg, FontId::proportional(quant(font)), theme::DANGER.gamma_multiply(a));
        }
        DirContent::Collapsed(n, err) => {
            let s = r.height() / COLLAPSED.y;
            if s * 12.0 >= 5.0 {
                let icon = Rect::from_min_size(r.left_top() + vec2(14.0, 14.0) * s, Vec2::splat(36.0 * s));
                icons::paint_folder_icon(p, icon, a, false);
                let tx = icon.right() + 10.0 * s;
                let main = match (n, err) {
                    (_, true) => "Access denied".to_string(),
                    (Some(0), _) => "Empty folder".to_string(),
                    (Some(n), _) => crate::app::plural(*n, "item"),
                    (None, _) => "Folder".to_string(),
                };
                p.text(pos2(tx, r.center().y - 8.0 * s), Align2::LEFT_CENTER, main, theme::semibold(quant(12.0 * s)), pal.text.gamma_multiply(a));
                p.text(pos2(tx, r.center().y + 9.0 * s), Align2::LEFT_CENTER, "Click the arrow to expand", FontId::proportional(quant(10.5 * s)), pal.faint.gamma_multiply(a));
            }
        }
        _ => {}
    }
    // Resize grip
    if (look.selected || look.hovered || custom) && z > 0.3 && !matches!(content, DirContent::Collapsed(..)) && look.mode_badge.is_none() {
        let c = r.right_bottom() - vec2(4.0, 4.0);
        let col = if look.selected { theme::ACCENT } else { pal.faint };
        for k in 1..=3 {
            let d = k as f32 * 3.5;
            p.line_segment([c - vec2(d, 0.0), c - vec2(0.0, d)], Stroke::new(1.2, col));
        }
    }
}

pub fn paint_dir_label(p: &Painter, lab: Rect, z: f32, name: &str, count: Option<usize>, expanded: bool, look: &Look) {
    let a = look.alpha();
    let pal = look.pal;
    let s = label_scale(z, lab.width());
    let font = quant(13.0 * s);
    if lab.width() < 24.0 || font < 5.0 {
        return;
    }
    let baseline_y = lab.bottom() - 6.0 * s.min(1.0) * if s > z { 0.6 } else { 1.0 };
    let cy = baseline_y - font * 0.55;
    let mut x = lab.left() + 2.0 * s;
    let max_x = lab.right();
    // chevron
    let col = if look.selected { theme::ACCENT } else { pal.muted };
    uicons::chevron(p, pos2(x + 6.0 * s, cy), 8.0 * s, expanded, col.gamma_multiply(a));
    x += 16.0 * s;
    let icon = Rect::from_min_size(pos2(x, cy - 7.5 * s), Vec2::splat(15.0 * s));
    icons::paint_folder_icon(p, icon, a, expanded);
    x += 19.0 * s;
    let text_col = if look.selected {
        theme::ACCENT
    } else if look.matched {
        theme::INK // on the yellow search plate
    } else {
        pal.ink
    };
    let mut job = LayoutJob::default();
    job.append(name, 0.0, TextFormat { font_id: theme::semibold(font), color: text_col.gamma_multiply(a), ..Default::default() });
    if let Some(n) = count {
        job.append(&format!("  {n}"), 0.0, TextFormat { font_id: FontId::proportional(quant(11.5 * s)), color: pal.faint.gamma_multiply(a), ..Default::default() });
    }
    job.wrap = TextWrapping { max_width: (max_x - x).max(10.0), max_rows: 1, break_anywhere: true, overflow_character: Some('\u{2026}') };
    let g = p.layout_job(job);
    let tr = Rect::from_min_size(pos2(x, cy - g.size().y / 2.0), g.size());
    // Readability backdrop when the label is drawn larger than the canvas scale.
    if s > z * 1.05 || look.matched || look.selected || look.drop {
        let bg = if look.matched {
            theme::MATCH
        } else if look.selected || look.drop {
            mix(pal.bg, theme::ACCENT, 0.12)
        } else {
            pal.backdrop
        };
        p.rect_filled(Rect::from_min_max(pos2(lab.left() - 2.0, tr.top() - 2.0), tr.max + vec2(5.0, 2.0)), 4.0, bg);
        uicons::chevron(p, pos2(lab.left() + 2.0 * s + 6.0 * s, cy), 8.0 * s, expanded, col.gamma_multiply(a));
        icons::paint_folder_icon(p, icon, a, expanded);
    }
    p.galley(tr.min, g, text_col);
}

pub fn paint_file(p: &Painter, cam: &Camera, vp: Rect, it: &Item, name: &str, look: &Look, hide_name: bool) {
    let z = cam.zoom;
    let tile = cam.rect_to_screen(vp, it.rect);
    let a = look.alpha();
    let pal = look.pal;
    if tile.width() < 7.0 {
        let c = icons::classify(&fsmodel::extension(name)).color();
        p.rect_filled(Rect::from_center_size(tile.center(), tile.size() * 0.5), 0.0, c.gamma_multiply(0.6 * a));
        return;
    }
    let hl = tile.shrink(2.0 * z.min(1.0));
    let rad = (6.0 * z).min(6.0);
    if look.selected {
        p.rect(hl, rad, mix(pal.bg, theme::ACCENT, 0.13), Stroke::new(1.0, theme::accent_soft(0.45)), StrokeKind::Inside);
    } else if look.matched {
        p.rect_filled(hl, rad, theme::MATCH.gamma_multiply(0.55));
    } else if look.hovered {
        p.rect_filled(hl, rad, if theme::is_dark(pal.bg) { Color32::from_white_alpha(18) } else { Color32::from_black_alpha(12) });
    }
    let icon = cam.rect_to_screen(vp, it.icon_rect());
    icons::paint_file_icon(p, icon, name, a);
    if look.missing {
        p.line_segment([icon.left_bottom(), icon.right_top()], Stroke::new(2.0, theme::DANGER));
    }
    let font = quant(11.5 * z);
    if font < 5.0 || hide_name {
        return;
    }
    let lab = cam.rect_to_screen(vp, it.label);
    let rows = if look.selected { 6 } else { 3 };
    // Search hits get a yellow plate, so they keep dark text even on dark canvases.
    let text_col = if look.matched && !look.selected { theme::TEXT } else { pal.text };
    let mut job = LayoutJob::single_section(name.to_string(), TextFormat { font_id: FontId::proportional(font), color: text_col.gamma_multiply(a), ..Default::default() });
    job.wrap = TextWrapping { max_width: lab.width(), max_rows: rows, break_anywhere: false, overflow_character: Some('\u{2026}') };
    job.halign = Align::Center;
    let g = p.layout_job(job);
    if look.selected && g.size().y > lab.height() + 1.0 {
        // Selected names expand below the tile, like on the Windows desktop.
        let bg = Rect::from_min_size(pos2(lab.center().x - g.size().x / 2.0 - 4.0, lab.top() - 1.0), g.size() + vec2(8.0, 3.0));
        p.rect(bg, 4.0, mix(pal.bg, theme::ACCENT, 0.13), Stroke::new(1.0, theme::accent_soft(0.45)), StrokeKind::Inside);
    }
    p.galley(pos2(lab.center().x, lab.top()), g, text_col);
}

/// Small "shortcut" arrow in the lower-left corner of an icon (symbolic links).
fn paint_link_badge(p: &Painter, icon: Rect) {
    let s = icon.width() / 44.0;
    let r = Rect::from_min_size(pos2(icon.left() + 4.0 * s, icon.bottom() - 16.0 * s), Vec2::splat(13.0 * s));
    p.rect(r, 2.0 * s, Color32::WHITE, Stroke::new(1.0, theme::FAINT), StrokeKind::Inside);
    let a = r.left_bottom() + vec2(3.5, -3.5) * s;
    let b = r.right_top() + vec2(-3.5, 3.5) * s;
    let st = Stroke::new(1.5 * s, theme::ACCENT);
    p.line_segment([a, b], st);
    p.line_segment([b, b + vec2(-4.0, 0.0) * s], st);
    p.line_segment([b, b + vec2(0.0, 4.0) * s], st);
}

fn paint_mode_badge(p: &Painter, top_right: Pos2, mode: ClipMode, z: f32, missing: bool) {
    let s = z.clamp(0.6, 1.2);
    let (text, col) = match (missing, mode) {
        (true, _) => ("MISSING", theme::DANGER),
        (_, ClipMode::Move) => ("MOVE", theme::ACCENT),
        (_, ClipMode::Copy) => ("COPY", theme::SUCCESS),
    };
    let g = p.layout_no_wrap(text.into(), theme::semibold(9.0 * s), Color32::WHITE);
    let r = Rect::from_min_size(pos2(top_right.x - g.size().x - 10.0 * s, top_right.y), g.size() + vec2(10.0 * s, 4.0 * s));
    p.rect_filled(r, 3.0 * s, col);
    p.galley(r.min + vec2(5.0 * s, 2.0 * s), g, Color32::WHITE);
}
