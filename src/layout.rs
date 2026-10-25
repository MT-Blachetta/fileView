//! Canvas layout: turns the directory tree into nested rectangles.
//!
//! Every folder is a rectangle. Its files sit in a tile grid in the upper
//! part, its sub-folders are shelf-packed below them. Sizes are computed
//! bottom-up so that a folder is exactly as large as its contents need
//! (or larger, if the user resized it).

use crate::fsmodel::{NodeId, Tree};
use egui::{Pos2, Rect, Vec2, pos2, vec2};
use std::collections::HashMap;
use std::path::PathBuf;

pub const TILE_W: f32 = 88.0;
pub const TILE_H: f32 = 98.0;
pub const ICON: f32 = 44.0;
pub const PAD: f32 = 14.0;
pub const LABEL_H: f32 = 26.0;
pub const GAP_X: f32 = 26.0;
pub const GAP_Y: f32 = 18.0;
pub const SECTION_GAP: f32 = 14.0;
pub const COLLAPSED: Vec2 = vec2(200.0, 64.0);
pub const EMPTY: Vec2 = vec2(150.0, 46.0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    File,
    Dir,
    /// "+N more" tile for folders with more files than shown.
    More(usize),
}

#[derive(Clone, Debug)]
pub struct Item {
    pub node: NodeId,
    pub kind: Kind,
    /// Folder body or file tile.
    pub rect: Rect,
    /// Folder: name strip above the body. File: name text area below the icon.
    pub label: Rect,
    pub depth: u16,
    /// Index one past the last descendant (folders); `self + 1` for files.
    pub end: usize,
    pub collapsed: bool,
    pub custom_size: bool,
}

impl Item {
    pub fn icon_rect(&self) -> Rect {
        Rect::from_center_size(pos2(self.rect.center().x, self.rect.top() + 6.0 + ICON / 2.0), Vec2::splat(ICON))
    }
    /// Union of body and label.
    pub fn outer(&self) -> Rect {
        self.rect.union(self.label)
    }
}

pub struct Layout {
    pub items: Vec<Item>,
    pub bounds: Rect,
    pub by_path: HashMap<PathBuf, usize>,
}

impl Default for Layout {
    fn default() -> Self {
        Layout { items: vec![], bounds: Rect::from_min_size(Pos2::ZERO, Vec2::splat(1.0)), by_path: HashMap::new() }
    }
}

impl Layout {
    pub fn compute(tree: &Tree, custom: &HashMap<PathBuf, Vec2>) -> Layout {
        let mut items = Vec::with_capacity(tree.nodes.len());
        let ctx = Ctx { tree, custom };
        let size = ctx.contents(tree.root, true, None, &mut items);
        let mut bounds = Rect::from_min_size(Pos2::ZERO, size.max(Vec2::splat(1.0)));
        for it in &items {
            bounds = bounds.union(it.outer());
        }
        let by_path = items
            .iter()
            .enumerate()
            .filter(|(_, it)| !matches!(it.kind, Kind::More(_)))
            .map(|(i, it)| (tree.nodes[it.node].path.clone(), i))
            .collect();
        Layout { items, bounds, by_path }
    }

    pub fn get(&self, path: &std::path::Path) -> Option<&Item> {
        self.by_path.get(path).map(|&i| &self.items[i])
    }
}

struct Ctx<'a> {
    tree: &'a Tree,
    custom: &'a HashMap<PathBuf, Vec2>,
}

fn translate(items: &mut [Item], d: Vec2) {
    for it in items {
        it.rect = it.rect.translate(d);
        it.label = it.label.translate(d);
    }
}

impl Ctx<'_> {
    /// Lays out the contents of folder `id` with the content origin at (0,0).
    /// Returns the content size.
    fn contents(&self, id: NodeId, is_root: bool, min_w: Option<f32>, out: &mut Vec<Item>) -> Vec2 {
        let node = &self.tree.nodes[id];
        let depth = node.depth + 1;

        // 1. Sub-folders first, so that we know their sizes.
        let mut blocks: Vec<(usize, usize, Vec2)> = Vec::with_capacity(node.dirs.len());
        for &d in &node.dirs {
            let start = out.len();
            let body = self.dir_block(d, depth, out);
            blocks.push((start, out.len(), vec2(body.x, body.y + LABEL_H)));
        }

        let n_files = node.files.len() + usize::from(node.omitted > 0);
        if n_files == 0 && blocks.is_empty() {
            return min_w.map(|w| vec2(w, EMPTY.y)).unwrap_or(EMPTY).max(EMPTY);
        }

        // 2. Choose a content width giving a pleasant aspect ratio.
        let max_block_w = blocks.iter().map(|b| b.2.x).fold(0.0, f32::max);
        let aspect = if is_root { 1.9 } else { 1.45 };
        let mut cw = if blocks.is_empty() {
            let cols = ((n_files as f32 * aspect * TILE_H / TILE_W).sqrt().ceil() as usize).clamp(1, n_files.max(1));
            cols as f32 * TILE_W
        } else {
            let area = n_files as f32 * TILE_W * TILE_H
                + blocks.iter().map(|b| (b.2.x + GAP_X) * (b.2.y + GAP_Y)).sum::<f32>();
            (area * aspect).sqrt()
        };
        if let Some(w) = min_w {
            cw = w;
        }
        cw = cw.max(max_block_w).max(TILE_W);

        // 3. File grid.
        let cols = ((cw / TILE_W).floor() as usize).max(1);
        let rows = n_files.div_ceil(cols);
        let files_h = rows as f32 * TILE_H;
        for (i, &f) in node.files.iter().enumerate() {
            let r = Rect::from_min_size(
                pos2((i % cols) as f32 * TILE_W, (i / cols) as f32 * TILE_H),
                vec2(TILE_W, TILE_H),
            );
            out.push(file_item(f, r, depth, out.len()));
        }
        if node.omitted > 0 {
            let i = node.files.len();
            let r = Rect::from_min_size(
                pos2((i % cols) as f32 * TILE_W, (i / cols) as f32 * TILE_H),
                vec2(TILE_W, TILE_H),
            );
            let mut it = file_item(id, r, depth, out.len());
            it.kind = Kind::More(node.omitted);
            out.push(it);
        }

        // 4. Shelf-pack the folder blocks below the files.
        let mut x = 0.0f32;
        let mut y = if n_files > 0 && !blocks.is_empty() { files_h + SECTION_GAP } else { files_h };
        let mut row_h = 0.0f32;
        let mut used_w = 0.0f32;
        for (start, end, size) in &blocks {
            if x > 0.0 && x + size.x > cw {
                x = 0.0;
                y += row_h + GAP_Y;
                row_h = 0.0;
            }
            translate(&mut out[*start..*end], vec2(x, y));
            used_w = used_w.max(x + size.x);
            x += size.x + GAP_X;
            row_h = row_h.max(size.y);
        }
        let total_h = if blocks.is_empty() { files_h } else { y + row_h };
        let files_w = cols.min(n_files) as f32 * TILE_W;
        let w = if min_w.is_some() { cw } else { used_w.max(files_w) };
        vec2(w, total_h)
    }

    /// Pushes folder `id` and its contents. The label starts at (0,0), the body
    /// directly below it. Returns the body size.
    fn dir_block(&self, id: NodeId, depth: u16, out: &mut Vec<Item>) -> Vec2 {
        let node = &self.tree.nodes[id];
        let idx = out.len();
        out.push(Item {
            node: id,
            kind: Kind::Dir,
            rect: Rect::NOTHING,
            label: Rect::NOTHING,
            depth,
            end: idx + 1,
            collapsed: !node.loaded,
            custom_size: false,
        });
        let custom = self.custom.get(&node.path).copied();
        let body = if !node.loaded {
            COLLAPSED
        } else {
            let min_w = custom.map(|c| (c.x - 2.0 * PAD).max(TILE_W));
            let c = self.contents(id, false, min_w, out);
            translate(&mut out[idx + 1..], vec2(PAD, LABEL_H + PAD));
            let mut size = c + Vec2::splat(2.0 * PAD);
            if let Some(cs) = custom {
                size.y = size.y.max(cs.y);
            }
            size
        };
        let end = out.len();
        let it = &mut out[idx];
        it.rect = Rect::from_min_size(pos2(0.0, LABEL_H), body);
        it.label = Rect::from_min_size(Pos2::ZERO, vec2(body.x, LABEL_H));
        it.end = end;
        it.custom_size = custom.is_some() && node.loaded;
        body
    }
}

fn file_item(node: NodeId, r: Rect, depth: u16, idx: usize) -> Item {
    let label = Rect::from_min_max(pos2(r.left() + 3.0, r.top() + 6.0 + ICON + 5.0), pos2(r.right() - 3.0, r.bottom() - 2.0));
    Item { node, kind: Kind::File, rect: r, label, depth, end: idx + 1, collapsed: false, custom_size: false }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsmodel::{ScanOptions, SortKey};
    use std::fs;

    #[test]
    fn nested_layout_contains_children() {
        let d = std::env::temp_dir().join(format!("planar-layout-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("a/b")).unwrap();
        fs::create_dir_all(d.join("c")).unwrap();
        for i in 0..7 {
            fs::write(d.join(format!("a/f{i}.txt")), "").unwrap();
        }
        fs::write(d.join("a/b/x.rs"), "").unwrap();
        fs::write(d.join("top.png"), "").unwrap();
        let opts = ScanOptions {
            show_hidden: false,
            auto_depth: 3,
            expand: Default::default(),
            sort: SortKey::Name,
            sort_desc: false,
            budget: 10_000,
            max_files_per_dir: 500,
        };
        let tree = Tree::scan(&d, &opts);
        let layout = Layout::compute(&tree, &HashMap::new());
        let a = layout.get(&d.join("a")).unwrap();
        let b = layout.get(&d.join("a/b")).unwrap();
        let x = layout.get(&d.join("a/b/x.rs")).unwrap();
        let top = layout.get(&d.join("top.png")).unwrap();
        assert!(a.rect.contains_rect(b.outer()));
        assert!(b.rect.contains_rect(x.rect));
        // Files are above the folders.
        assert!(top.rect.bottom() <= a.label.top());
        // The subtree range of `a` covers its descendants.
        let ai = layout.by_path[&d.join("a")];
        let xi = layout.by_path[&d.join("a/b/x.rs")];
        assert!(xi > ai && xi < layout.items[ai].end);
        // Folders don't overlap.
        let c = layout.get(&d.join("c")).unwrap();
        assert!(!a.outer().intersects(c.outer()));

        let mut custom = HashMap::new();
        custom.insert(d.join("c"), vec2(900.0, 500.0));
        let layout2 = Layout::compute(&tree, &custom);
        let c2 = layout2.get(&d.join("c")).unwrap();
        assert!((c2.rect.width() - 900.0).abs() < 0.1 && (c2.rect.height() - 500.0).abs() < 0.1);
        fs::remove_dir_all(&d).unwrap();
    }
}
