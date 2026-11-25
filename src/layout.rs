//! Canvas layout: turns the directory tree into nested rectangles.
//!
//! Every folder is a rectangle. Its files sit in a tile grid in the upper
//! part, its sub-folders are shelf-packed below them. Sizes are computed
//! bottom-up so that a folder is exactly as large as its contents need
//! (or larger, if the user resized it).
//!
//! The order of files and the rows of folders follow the user's custom
//! arrangement where there is one (see `arrange`), the sort order otherwise.

use crate::arrange::{Arrangement, DirOrder};
use crate::fsmodel::{NodeId, Tree};
use egui::{Pos2, Rect, Vec2, pos2, vec2};
use std::collections::{HashMap, HashSet};
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
    pub fn compute(tree: &Tree, custom: &HashMap<PathBuf, Vec2>, arrange: &Arrangement) -> Layout {
        let mut items = Vec::with_capacity(tree.nodes.len());
        let ctx = Ctx { tree, custom, arrange };
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
    arrange: &'a Arrangement,
}

/// A folder block to be shelf-packed.
struct Block {
    /// Range of its items in the output.
    start: usize,
    end: usize,
    size: Vec2,
    /// Starts a new row (custom arrangement).
    row_start: bool,
    /// May wrap to a new row when the row gets too wide (not arranged).
    auto: bool,
}

/// `ids` in the custom order `names`; unknown items keep their sort order
/// and follow the known ones.
fn ordered(tree: &Tree, ids: &[NodeId], names: &[String]) -> Vec<NodeId> {
    let mut v = ids.to_vec();
    if !names.is_empty() {
        let rank: HashMap<&str, usize> = names.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
        v.sort_by_key(|&id| rank.get(tree.nodes[id].name.as_str()).copied().unwrap_or(usize::MAX));
    }
    v
}

/// Sub-folders in display order with their row breaks: (id, row_start, auto).
fn dir_sequence(tree: &Tree, dirs: &[NodeId], order: Option<&DirOrder>) -> Vec<(NodeId, bool, bool)> {
    let rows = order.map(|o| o.rows.as_slice()).unwrap_or_default();
    if rows.is_empty() {
        return dirs.iter().map(|&d| (d, false, true)).collect();
    }
    let by_name: HashMap<&str, NodeId> = dirs.iter().map(|&d| (tree.nodes[d].name.as_str(), d)).collect();
    let mut used = HashSet::new();
    let mut seq = vec![];
    for row in rows {
        let mut first = true;
        for name in row {
            if let Some(&d) = by_name.get(name.as_str())
                && used.insert(d)
            {
                seq.push((d, first, false));
                first = false;
            }
        }
    }
    // Folders without a place (e.g. created later) go into new rows below.
    let mut first = true;
    for &d in dirs {
        if !used.contains(&d) {
            seq.push((d, first, true));
            first = false;
        }
    }
    seq
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
        let order = self.arrange.get(&node.path);
        let files = ordered(self.tree, &node.files, order.map(|o| o.files.as_slice()).unwrap_or_default());

        // 1. Sub-folders first, so that we know their sizes.
        let mut blocks: Vec<Block> = Vec::with_capacity(node.dirs.len());
        for (d, row_start, auto) in dir_sequence(self.tree, &node.dirs, order) {
            let start = out.len();
            let body = self.dir_block(d, depth, out);
            blocks.push(Block { start, end: out.len(), size: vec2(body.x, body.y + LABEL_H), row_start, auto });
        }

        let n_files = files.len() + usize::from(node.omitted > 0);
        if n_files == 0 && blocks.is_empty() {
            return min_w.map(|w| vec2(w, EMPTY.y)).unwrap_or(EMPTY).max(EMPTY);
        }

        // 2. Choose a content width giving a pleasant aspect ratio.
        let max_block_w = blocks.iter().map(|b| b.size.x).fold(0.0, f32::max);
        // Arranged rows are never wrapped, the folder grows to fit them.
        let mut widest_row = 0.0f32;
        let mut row_w = 0.0f32;
        for b in blocks.iter().filter(|b| !b.auto) {
            row_w = if b.row_start { b.size.x } else { row_w + GAP_X + b.size.x };
            widest_row = widest_row.max(row_w);
        }
        let aspect = if is_root { 1.9 } else { 1.45 };
        let mut cw = if blocks.is_empty() {
            let cols = ((n_files as f32 * aspect * TILE_H / TILE_W).sqrt().ceil() as usize).clamp(1, n_files.max(1));
            cols as f32 * TILE_W
        } else {
            let area = n_files as f32 * TILE_W * TILE_H
                + blocks.iter().map(|b| (b.size.x + GAP_X) * (b.size.y + GAP_Y)).sum::<f32>();
            (area * aspect).sqrt()
        };
        if let Some(w) = min_w {
            cw = w;
        }
        cw = cw.max(max_block_w).max(widest_row).max(TILE_W);

        // 3. File grid.
        let cols = ((cw / TILE_W).floor() as usize).max(1);
        let rows = n_files.div_ceil(cols);
        let files_h = rows as f32 * TILE_H;
        for (i, &f) in files.iter().enumerate() {
            let r = Rect::from_min_size(
                pos2((i % cols) as f32 * TILE_W, (i / cols) as f32 * TILE_H),
                vec2(TILE_W, TILE_H),
            );
            out.push(file_item(f, r, depth, out.len()));
        }
        if node.omitted > 0 {
            let i = files.len();
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
        for b in &blocks {
            let size = b.size;
            if x > 0.0 && (b.row_start || b.auto && x + size.x > cw) {
                x = 0.0;
                y += row_h + GAP_Y;
                row_h = 0.0;
            }
            translate(&mut out[b.start..b.end], vec2(x, y));
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
        let layout = Layout::compute(&tree, &HashMap::new(), &Arrangement::empty(&d));
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
        let layout2 = Layout::compute(&tree, &custom, &Arrangement::empty(&d));
        let c2 = layout2.get(&d.join("c")).unwrap();
        assert!((c2.rect.width() - 900.0).abs() < 0.1 && (c2.rect.height() - 500.0).abs() < 0.1);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn custom_arrangement() {
        let d = std::env::temp_dir().join(format!("planar-layout-arr-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        for n in ["d1", "d2", "d3", "d4"] {
            fs::create_dir_all(d.join(n)).unwrap();
        }
        for n in ["a.txt", "b.txt", "c.txt"] {
            fs::write(d.join(n), "").unwrap();
        }
        let opts = ScanOptions {
            show_hidden: false,
            auto_depth: 1,
            expand: Default::default(),
            sort: SortKey::Name,
            sort_desc: false,
            budget: 10_000,
            max_files_per_dir: 500,
        };
        let tree = Tree::scan(&d, &opts);
        let mut arr = Arrangement::empty(&d);
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        // c.txt first; d3 alone in the first row, d1 + d2 in the second, d4 (unknown) after them.
        arr.set_order(&d, Some(s(&["c.txt", "a.txt"])), Some(vec![s(&["d3"]), s(&["d2", "d1"])]));
        let l = Layout::compute(&tree, &HashMap::new(), &arr);
        let at = |n: &str| l.get(&d.join(n)).unwrap().clone();
        assert!(at("c.txt").rect.left() < at("a.txt").rect.left());
        assert!(at("a.txt").rect.left() < at("b.txt").rect.left(), "unknown files follow the arranged ones");
        assert!(at("d3").label.top() < at("d2").label.top(), "d3 has a row of its own");
        assert_eq!(at("d2").label.top(), at("d1").label.top());
        assert!(at("d2").rect.left() < at("d1").rect.left());
        assert!(at("d4").label.top() > at("d1").label.top(), "unknown folders go below the arranged rows");
        // Files stay above all folders.
        assert!(at("b.txt").rect.bottom() <= at("d3").label.top());
        fs::remove_dir_all(&d).unwrap();
    }
}
