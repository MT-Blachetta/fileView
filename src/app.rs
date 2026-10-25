//! Application state, actions and the surrounding UI (top bar, panels,
//! menus, dialogs). Canvas rendering and pointer interaction live in
//! `canvas.rs`.

use crate::canvas::{Camera, Gesture, Hit};
use crate::fsmodel::{self, SortKey, Tree};
use crate::layout::{Kind, Layout};
use crate::ops::{self, Job, JobKind, JobResult};
use crate::theme;
use crate::uicons::{self, Ico};
use egui::{
    Align, Align2, Button, Color32, CornerRadius, Event, Frame, Id, Key, Layout as ELayout, Margin, Order, Pos2,
    Rect, RichText, Sense, Stroke, TextEdit, Ui, Vec2, pos2, vec2,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    Main,
    Clip,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    Pan,
    DrawFolder,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ClipMode {
    Move,
    Copy,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ClipItem {
    pub path: PathBuf,
    pub mode: ClipMode,
    pub pos: Pos2,
    #[serde(skip)]
    pub info: ClipInfo,
}

#[derive(Clone, Default)]
pub struct ClipInfo {
    pub name: String,
    pub is_dir: bool,
    pub entries: Option<usize>,
    pub exists: bool,
}

impl ClipItem {
    pub fn refresh(&mut self) {
        let md = std::fs::metadata(&self.path).ok();
        self.info = ClipInfo {
            name: fsmodel::display_name(&self.path),
            is_dir: md.as_ref().map(|m| m.is_dir()).unwrap_or(false),
            entries: if md.as_ref().map(|m| m.is_dir()).unwrap_or(false) {
                std::fs::read_dir(&self.path).ok().map(|rd| rd.count())
            } else {
                None
            },
            exists: md.is_some(),
        };
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub canvas_bg: Color32,
    pub dot_grid: bool,
    pub show_hidden: bool,
    pub auto_depth: u16,
    pub sort: SortKey,
    pub sort_desc: bool,
    pub minimap: bool,
    pub show_props: bool,
    pub show_clip: bool,
    /// Mouse wheel zooms (miro style). If false the wheel pans and Ctrl+wheel zooms.
    pub wheel_zooms: bool,
    /// Key that toggles between the select and the hand tool: "Alt", "Ctrl",
    /// "Shift" (toggle when tapped alone) or an egui key name such as "Tab".
    pub toggle_key: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            canvas_bg: theme::CANVAS_DEFAULT,
            dot_grid: true,
            show_hidden: false,
            auto_depth: 2,
            sort: SortKey::Name,
            sort_desc: false,
            minimap: true,
            show_props: true,
            show_clip: true,
            wheel_zooms: true,
            toggle_key: "Alt".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Persisted {
    root: Option<PathBuf>,
    settings: Settings,
    clip: Vec<ClipItem>,
    custom_sizes: Vec<(PathBuf, Vec2)>,
    expand: Vec<(PathBuf, bool)>,
}

#[derive(Clone, Debug)]
pub enum MenuTarget {
    /// An item on the main canvas (file or folder).
    Item(PathBuf),
    /// Empty canvas area (the root folder).
    Background,
    ClipItem(PathBuf),
    ClipBackground,
}

pub struct RenameState {
    pub path: PathBuf,
    pub text: String,
    pub just_started: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub until: f64,
    pub undo: bool,
}

pub enum Undo {
    Move(Vec<(PathBuf, PathBuf)>),
    Rename(PathBuf, PathBuf),
    Created(Vec<PathBuf>),
}

pub enum Confirm {
    DeletePermanently(Vec<PathBuf>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum DropTarget {
    None,
    Dir(PathBuf),
    Clip(Pos2),
}

pub struct App {
    pub root: PathBuf,
    pub path_input: String,
    pub editing_path: bool,
    pub back: Vec<PathBuf>,
    pub fwd: Vec<PathBuf>,

    pub tree: Tree,
    pub layout: Layout,
    pub layout_dirty: bool,
    scan_rx: Option<Receiver<Tree>>,
    scan_again: bool,
    pub expand: HashMap<PathBuf, bool>,
    pub custom_sizes: HashMap<PathBuf, Vec2>,
    pub settings: Settings,

    pub cam: Camera,
    pub clip_cam: Camera,
    pub main_rect: Rect,
    pub clip_rect: Rect,
    pub tool: Tool,
    pub sel: BTreeSet<PathBuf>,
    pub clip_sel: BTreeSet<PathBuf>,
    pub focus: Pane,
    pub clip: Vec<ClipItem>,

    pub gesture: Gesture,
    pub hover: Option<Hit>,
    pub hover_clip: Option<usize>,
    pub drop: DropTarget,
    pub menu: Option<MenuTarget>,
    pub rename: Option<RenameState>,

    pub search: String,
    /// Search results in canvas order.
    pub match_list: Vec<PathBuf>,
    pub matches: std::collections::HashSet<PathBuf>,
    /// Folders containing a search result (not dimmed while searching).
    pub match_ancestors: std::collections::HashSet<PathBuf>,
    pub match_cursor: usize,
    focus_search: bool,

    jobs: Vec<(Receiver<JobResult>, String)>,
    undo: Vec<Undo>,
    pub toasts: Vec<Toast>,
    dir_stats: HashMap<PathBuf, Option<DirStats>>,
    stat_rx: Vec<Receiver<(PathBuf, DirStats)>>,
    stat_cancel: Arc<AtomicBool>,
    confirm: Option<Confirm>,
    show_help: bool,

    pending_rename: Option<PathBuf>,
    /// The toggle modifier went down and nothing else happened since.
    toggle_armed: bool,
    prev_mods: egui::Modifiers,
    /// The View menu waits for the new toggle key.
    pub capturing_key: bool,
    /// Pending result of the system folder dialog.
    folder_pick_rx: Option<Receiver<Option<PathBuf>>>,
    pending_select: Vec<PathBuf>,
    pub needs_fit: bool,
    last_poll: f64,
    pub last_pointer: Option<Pos2>,
    script: Option<crate::script::Script>,
}

const MAX_UNDO: usize = 50;

/// Recursive folder size: (bytes, files, folders).
type DirStats = (u64, usize, usize);

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, root_arg: Option<PathBuf>) -> Self {
        theme::install(&cc.egui_ctx);
        let persisted: Persisted = cc.storage.and_then(|s| eframe::get_value(s, eframe::APP_KEY)).unwrap_or_default();
        let root = root_arg
            .or(persisted.root.clone())
            .filter(|p| p.is_dir())
            .or_else(home_dir)
            .unwrap_or_else(|| PathBuf::from("/"));
        let root = std::fs::canonicalize(&root).map(clean_path).unwrap_or(root);
        let mut clip = persisted.clip;
        for c in &mut clip {
            c.refresh();
        }
        let mut app = App {
            path_input: root.display().to_string(),
            editing_path: false,
            back: vec![],
            fwd: vec![],
            tree: Tree::empty(&root),
            layout: Layout::default(),
            layout_dirty: true,
            scan_rx: None,
            scan_again: false,
            expand: persisted.expand.into_iter().filter(|(p, _)| p.exists()).collect(),
            custom_sizes: persisted.custom_sizes.into_iter().filter(|(p, _)| p.exists()).collect(),
            settings: persisted.settings,
            cam: Camera::default(),
            clip_cam: Camera::default(),
            main_rect: Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)),
            clip_rect: Rect::NOTHING,
            tool: Tool::Select,
            sel: BTreeSet::new(),
            clip_sel: BTreeSet::new(),
            focus: Pane::Main,
            clip,
            gesture: Gesture::None,
            hover: None,
            hover_clip: None,
            drop: DropTarget::None,
            menu: None,
            rename: None,
            search: String::new(),
            match_list: vec![],
            matches: Default::default(),
            match_ancestors: Default::default(),
            match_cursor: 0,
            focus_search: false,
            jobs: vec![],
            undo: vec![],
            toasts: vec![],
            dir_stats: HashMap::new(),
            stat_rx: vec![],
            stat_cancel: Arc::new(AtomicBool::new(false)),
            confirm: None,
            show_help: false,
            pending_rename: None,
            toggle_armed: false,
            prev_mods: egui::Modifiers::NONE,
            capturing_key: false,
            folder_pick_rx: None,
            pending_select: vec![],
            needs_fit: true,
            last_poll: 0.0,
            last_pointer: None,
            script: std::env::var("PLANAR_SCRIPT").ok().and_then(|p| crate::script::Script::load(&p)),
            root,
        };
        app.clip_cam.offset = vec2(-16.0, -16.0);
        cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("{} \u{2014} Planar", fsmodel::display_name(&app.root))));
        app.request_scan(&cc.egui_ctx);
        app
    }

    // ------------------------------------------------------------------
    // Scanning & layout

    pub fn scan_options(&self) -> fsmodel::ScanOptions {
        fsmodel::ScanOptions {
            show_hidden: self.settings.show_hidden,
            auto_depth: self.settings.auto_depth,
            expand: self.expand.clone(),
            sort: self.settings.sort,
            sort_desc: self.settings.sort_desc,
            budget: 40_000,
            max_files_per_dir: 400,
        }
    }

    pub fn request_scan(&mut self, ctx: &egui::Context) {
        if self.scan_rx.is_some() {
            self.scan_again = true;
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let root = self.root.clone();
        let opts = self.scan_options();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let tree = Tree::scan(&root, &opts);
            let _ = tx.send(tree);
            ctx.request_repaint();
        });
        self.scan_rx = Some(rx);
    }

    pub fn is_scanning(&self) -> bool {
        self.scan_rx.is_some()
    }

    fn poll_scan(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.scan_rx else { return };
        match rx.try_recv() {
            Ok(tree) => {
                self.scan_rx = None;
                if tree.root_node().path == self.root {
                    if let Some(e) = &tree.root_node().error {
                        self.toast(format!("Cannot read folder: {e}"), ToastKind::Error);
                    }
                    self.tree = tree;
                    self.layout_dirty = true;
                    self.dir_stats.clear();
                }
                if self.scan_again {
                    self.scan_again = false;
                    self.request_scan(ctx);
                }
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.scan_rx = None,
            Err(_) => {}
        }
    }

    /// Expands or collapses a folder in place (without a full rescan).
    pub fn toggle_expand(&mut self, path: &Path) {
        let Some(&idx) = self.layout.by_path.get(path) else { return };
        let id = self.layout.items[idx].node;
        let loaded = self.tree.nodes[id].loaded;
        self.expand.insert(path.to_path_buf(), !loaded);
        if loaded {
            let n = &mut self.tree.nodes[id];
            n.loaded = false;
            n.dirs.clear();
            n.files.clear();
            n.omitted = 0;
        } else {
            let mut opts = self.scan_options();
            opts.auto_depth = 0;
            opts.budget = 20_000;
            let sub = Tree::scan(path, &opts);
            graft(&mut self.tree, id, sub);
        }
        self.layout_dirty = true;
    }

    fn relayout(&mut self) {
        self.layout = Layout::compute(&self.tree, &self.custom_sizes);
        self.layout_dirty = false;
        self.sel.retain(|p| p.exists());
        self.update_matches();
    }

    // ------------------------------------------------------------------
    // Navigation

    pub fn set_root(&mut self, ctx: &egui::Context, path: PathBuf, record_history: bool) {
        let path = std::fs::canonicalize(&path).map(clean_path).unwrap_or(path);
        if !path.is_dir() {
            self.toast(format!("\u{201c}{}\u{201d} is not a folder", path.display()), ToastKind::Error);
            self.path_input = self.root.display().to_string();
            return;
        }
        if path == self.root {
            return;
        }
        if record_history {
            self.back.push(self.root.clone());
            self.fwd.clear();
        }
        self.root = path;
        self.path_input = self.root.display().to_string();
        self.tree = Tree::empty(&self.root);
        self.layout = Layout::default();
        self.sel.clear();
        self.hover = None;
        self.rename = None;
        self.gesture = Gesture::None;
        self.needs_fit = true;
        self.scan_rx = None;
        self.scan_again = false;
        self.request_scan(ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("{} \u{2014} Planar", fsmodel::display_name(&self.root))));
    }

    pub fn go_up(&mut self, ctx: &egui::Context) {
        if let Some(p) = self.root.parent() {
            let child = self.root.clone();
            self.set_root(ctx, p.to_path_buf(), true);
            self.pending_select = vec![child];
        }
    }

    pub fn go_back(&mut self, ctx: &egui::Context) {
        if let Some(p) = self.back.pop() {
            self.fwd.push(self.root.clone());
            self.set_root(ctx, p, false);
        }
    }

    pub fn go_forward(&mut self, ctx: &egui::Context) {
        if let Some(p) = self.fwd.pop() {
            self.back.push(self.root.clone());
            self.set_root(ctx, p, false);
        }
    }

    pub fn open_item(&mut self, ctx: &egui::Context, path: &Path) {
        if path.is_dir() {
            self.set_root(ctx, path.to_path_buf(), true);
        } else if let Err(e) = ops::open_default(path) {
            self.toast(format!("Could not open: {e}"), ToastKind::Error);
        }
    }

    // ------------------------------------------------------------------
    // Clipboard canvas

    /// Adds items to the clipboard canvas. `at` is a world position on the
    /// clipboard canvas; if `None` the items are placed below the existing ones.
    pub fn add_to_clip(&mut self, paths: &[PathBuf], mode: ClipMode, at: Option<Pos2>) {
        let mut pos = at.unwrap_or_else(|| {
            let bottom = self.clip.iter().map(|c| crate::canvas::clip_item_rect(c).bottom()).fold(0.0f32, f32::max);
            pos2(0.0, if self.clip.is_empty() { 0.0 } else { bottom + 24.0 })
        });
        self.clip_sel.clear();
        for p in paths {
            if let Some(existing) = self.clip.iter_mut().find(|c| &c.path == p) {
                existing.mode = mode;
                if at.is_some() {
                    existing.pos = pos;
                }
            } else {
                let mut item = ClipItem { path: p.clone(), mode, pos, info: ClipInfo::default() };
                item.refresh();
                let r = crate::canvas::clip_item_rect(&item);
                pos.x += r.width() + 20.0;
                if pos.x > 600.0 {
                    pos.x = at.map(|a| a.x).unwrap_or(0.0);
                    pos.y += r.height() + 24.0;
                }
                self.clip.push(item);
            }
            self.clip_sel.insert(p.clone());
        }
        self.settings.show_clip = true;
    }

    pub fn arrange_clip(&mut self) {
        let mut x = 0.0f32;
        let mut y = 0.0f32;
        let mut row_h = 0.0f32;
        let width = (self.clip_rect.width() / self.clip_cam.zoom - 40.0).max(200.0);
        for c in &mut self.clip {
            let r = crate::canvas::clip_item_rect(c);
            if x > 0.0 && x + r.width() > width {
                x = 0.0;
                y += row_h + 20.0;
                row_h = 0.0;
            }
            c.pos = pos2(x, y);
            x += r.width() + 16.0;
            row_h = row_h.max(r.height());
        }
    }

    pub fn cut_or_copy(&mut self, mode: ClipMode, ctx: &egui::Context) {
        let items: Vec<PathBuf> = self.sel.iter().cloned().collect();
        if items.is_empty() {
            return;
        }
        self.add_to_clip(&items, mode, None);
        // Also put the paths on the system clipboard as text.
        ctx.copy_text(items.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"));
        let verb = if mode == ClipMode::Move { "Cut" } else { "Copied" };
        self.toast(format!("{verb} {} \u{2192} clipboard area", plural(items.len(), "item")), ToastKind::Info);
    }

    /// Pastes the selected clipboard items (or all of them) into `dest`.
    pub fn paste_into(&mut self, ctx: &egui::Context, dest: &Path) {
        let use_sel = !self.clip_sel.is_empty();
        let items: Vec<(PathBuf, ClipMode)> = self
            .clip
            .iter()
            .filter(|c| !use_sel || self.clip_sel.contains(&c.path))
            .map(|c| (c.path.clone(), c.mode))
            .collect();
        if items.is_empty() {
            self.toast("The clipboard is empty".into(), ToastKind::Info);
            return;
        }
        let moves: Vec<PathBuf> = items.iter().filter(|i| i.1 == ClipMode::Move).map(|i| i.0.clone()).collect();
        let copies: Vec<PathBuf> = items.iter().filter(|i| i.1 == ClipMode::Copy).map(|i| i.0.clone()).collect();
        self.transfer(ctx, &moves, dest, false);
        self.transfer(ctx, &copies, dest, true);
    }

    /// Moves (or copies) `items` into the folder `dest` in a background job.
    pub fn transfer(&mut self, ctx: &egui::Context, items: &[PathBuf], dest: &Path, copy: bool) {
        let mut pairs = vec![];
        for src in items {
            if let Err(e) = ops::check_transfer(src, dest) {
                self.toast(e, ToastKind::Error);
                continue;
            }
            let name = src.file_name().map(|n| n.to_os_string()).unwrap_or_default();
            if copy {
                pairs.push((src.clone(), ops::unique_copy_target(dest, &name.to_string_lossy())));
            } else {
                if src.parent() == Some(dest) {
                    continue; // already there
                }
                let target = dest.join(&name);
                if target.exists() {
                    self.toast(format!("\u{201c}{}\u{201d} already exists in the destination", name.to_string_lossy()), ToastKind::Error);
                    continue;
                }
                pairs.push((src.clone(), target));
            }
        }
        if pairs.is_empty() {
            return;
        }
        let kind = if copy { JobKind::Copy } else { JobKind::Move };
        self.spawn_job(ctx, Job { kind, pairs, is_undo: false });
    }

    fn spawn_job(&mut self, ctx: &egui::Context, job: Job) {
        let desc = format!("{} {}\u{2026}", job.kind.verb(), plural(job.pairs.len(), "item"));
        let rx = ops::spawn(job, ctx.clone());
        self.jobs.push((rx, desc));
    }

    fn poll_jobs(&mut self, ctx: &egui::Context) {
        let mut finished = vec![];
        self.jobs.retain(|(rx, _)| match rx.try_recv() {
            Ok(r) => {
                finished.push(r);
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(_) => false,
        });
        for r in finished {
            self.job_finished(ctx, r);
        }
    }

    fn job_finished(&mut self, ctx: &egui::Context, r: JobResult) {
        let n = r.done.len();
        if !r.errors.is_empty() {
            let more = if r.errors.len() > 1 { format!(" (+{} more)", r.errors.len() - 1) } else { String::new() };
            self.toast(format!("{}{more}", r.errors[0]), ToastKind::Error);
        }
        match r.kind {
            JobKind::Move => {
                for (from, to) in &r.done {
                    self.rekey(from, to);
                    self.clip.retain(|c| &c.path != from);
                    self.clip_sel.remove(from);
                }
                if n > 0 {
                    self.pending_select = r.done.iter().map(|d| d.1.clone()).collect();
                    if !r.is_undo {
                        self.push_undo(Undo::Move(r.done.clone()));
                        let dest = r.done[0].1.parent().map(fsmodel::display_name).unwrap_or_default();
                        self.toast_undo(format!("Moved {} to \u{201c}{dest}\u{201d}", plural(n, "item")));
                    } else {
                        self.toast(format!("Undone \u{2014} moved {} back", plural(n, "item")), ToastKind::Info);
                    }
                }
            }
            JobKind::Copy => {
                if n > 0 {
                    self.pending_select = r.done.iter().map(|d| d.1.clone()).collect();
                    self.push_undo(Undo::Created(r.done.iter().map(|d| d.1.clone()).collect()));
                    let dest = r.done[0].1.parent().map(fsmodel::display_name).unwrap_or_default();
                    self.toast_undo(format!("Copied {} to \u{201c}{dest}\u{201d}", plural(n, "item")));
                }
            }
            JobKind::Trash | JobKind::Delete => {
                for (p, _) in &r.done {
                    self.clip.retain(|c| &c.path != p);
                    self.sel.remove(p);
                }
                if n > 0 && !r.is_undo {
                    let what = if r.kind == JobKind::Trash { "Moved to trash:" } else { "Deleted" };
                    self.toast(format!("{what} {}", plural(n, "item")), ToastKind::Success);
                } else if n > 0 {
                    self.toast(format!("Undone \u{2014} removed {}", plural(n, "item")), ToastKind::Info);
                }
            }
        }
        for c in &mut self.clip {
            c.refresh();
        }
        self.dir_stats.clear();
        self.request_scan(ctx);
    }

    /// Updates path-keyed state after `from` was moved/renamed to `to`.
    fn rekey(&mut self, from: &Path, to: &Path) {
        fn remap<V: Clone>(map: &mut HashMap<PathBuf, V>, from: &Path, to: &Path) {
            let keys: Vec<PathBuf> = map.keys().filter(|k| k.starts_with(from)).cloned().collect();
            for k in keys {
                let v = map.remove(&k).unwrap();
                let rest = k.strip_prefix(from).unwrap();
                map.insert(if rest.as_os_str().is_empty() { to.to_path_buf() } else { to.join(rest) }, v);
            }
        }
        remap(&mut self.custom_sizes, from, to);
        remap(&mut self.expand, from, to);
        if self.sel.remove(from) {
            self.sel.insert(to.to_path_buf());
        }
        for c in &mut self.clip {
            if c.path.starts_with(from) && c.path != from {
                c.path = to.join(c.path.strip_prefix(from).unwrap());
            }
        }
    }

    fn push_undo(&mut self, u: Undo) {
        self.undo.push(u);
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn undo(&mut self, ctx: &egui::Context) {
        let Some(u) = self.undo.pop() else {
            self.toast("Nothing to undo".into(), ToastKind::Info);
            return;
        };
        match u {
            Undo::Move(pairs) => {
                let pairs = pairs.into_iter().map(|(a, b)| (b, a)).collect();
                self.spawn_job(ctx, Job { kind: JobKind::Move, pairs, is_undo: true });
            }
            Undo::Rename(from, to) => match ops::rename(&to, &fsmodel::display_name(&from)) {
                Ok(p) => {
                    self.rekey(&to, &p);
                    self.toast("Undone \u{2014} rename reverted".into(), ToastKind::Info);
                    self.request_scan(ctx);
                }
                Err(e) => self.toast(e, ToastKind::Error),
            },
            Undo::Created(paths) => {
                let pairs = paths.into_iter().map(|p| (p.clone(), p)).collect();
                self.spawn_job(ctx, Job { kind: JobKind::Trash, pairs, is_undo: true });
            }
        }
    }

    pub fn trash_selected(&mut self, ctx: &egui::Context) {
        let items: Vec<PathBuf> = self.sel.iter().cloned().collect();
        if items.is_empty() {
            return;
        }
        let pairs = items.into_iter().map(|p| (p.clone(), p)).collect();
        self.spawn_job(ctx, Job { kind: JobKind::Trash, pairs, is_undo: false });
    }

    pub fn confirm_delete_selected(&mut self) {
        let items: Vec<PathBuf> = self.sel.iter().cloned().collect();
        if !items.is_empty() {
            self.confirm = Some(Confirm::DeletePermanently(items));
        }
    }

    pub fn new_folder(&mut self, ctx: &egui::Context, parent: &Path, size: Option<Vec2>) {
        match ops::create_dir(parent, "New folder") {
            Ok(p) => {
                if let Some(s) = size {
                    self.custom_sizes.insert(p.clone(), s);
                }
                self.expand.insert(p.clone(), true);
                self.push_undo(Undo::Created(vec![p.clone()]));
                self.pending_rename = Some(p);
                self.request_scan(ctx);
            }
            Err(e) => self.toast(format!("Could not create folder: {e}"), ToastKind::Error),
        }
    }

    pub fn new_file(&mut self, ctx: &egui::Context, parent: &Path) {
        match ops::create_file(parent, "New text document.txt") {
            Ok(p) => {
                self.push_undo(Undo::Created(vec![p.clone()]));
                self.pending_rename = Some(p);
                self.request_scan(ctx);
            }
            Err(e) => self.toast(format!("Could not create file: {e}"), ToastKind::Error),
        }
    }

    /// Folder that "New folder"/"Paste" without an explicit target act on:
    /// the single selected folder, the parent of a single selected file, or the root.
    pub fn target_dir(&self) -> PathBuf {
        if self.sel.len() == 1 {
            let p = self.sel.iter().next().unwrap();
            if p.is_dir() {
                return p.clone();
            }
            if let Some(parent) = p.parent()
                && parent.starts_with(&self.root) {
                    return parent.to_path_buf();
                }
        }
        self.root.clone()
    }

    pub fn start_rename(&mut self, path: &Path) {
        if path == self.root {
            return;
        }
        self.rename = Some(RenameState { path: path.to_path_buf(), text: fsmodel::display_name(path), just_started: true });
    }

    fn commit_rename(&mut self, ctx: &egui::Context) {
        let Some(r) = self.rename.take() else { return };
        if r.text.trim() == fsmodel::display_name(&r.path) {
            return;
        }
        match ops::rename(&r.path, &r.text) {
            Ok(newp) => {
                self.rekey(&r.path, &newp);
                self.sel.clear();
                self.sel.insert(newp.clone());
                self.push_undo(Undo::Rename(r.path.clone(), newp));
                self.request_scan(ctx);
            }
            Err(e) => self.toast(e, ToastKind::Error),
        }
    }

    pub fn toast(&mut self, text: String, kind: ToastKind) {
        let secs = if kind == ToastKind::Error { 6.0 } else { 3.5 };
        self.toasts.push(Toast { text, kind, until: now() + secs, undo: false });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    fn toast_undo(&mut self, text: String) {
        self.toasts.push(Toast { text, kind: ToastKind::Success, until: now() + 6.0, undo: true });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    // ------------------------------------------------------------------
    // Search

    pub fn update_matches(&mut self) {
        self.match_list.clear();
        self.matches.clear();
        self.match_ancestors.clear();
        let q = self.search.trim().to_lowercase();
        if q.is_empty() {
            return;
        }
        for it in &self.layout.items {
            if matches!(it.kind, Kind::More(_)) {
                continue;
            }
            let n = &self.tree.nodes[it.node];
            if n.name.to_lowercase().contains(&q) {
                self.match_list.push(n.path.clone());
                self.matches.insert(n.path.clone());
                let mut a = n.path.parent();
                while let Some(p) = a {
                    if !p.starts_with(&self.root) || !self.match_ancestors.insert(p.to_path_buf()) {
                        break;
                    }
                    a = p.parent();
                }
            }
        }
        if self.match_cursor >= self.match_list.len() {
            self.match_cursor = 0;
        }
    }

    fn jump_to_match(&mut self, delta: isize) {
        if self.match_list.is_empty() {
            return;
        }
        let n = self.match_list.len() as isize;
        self.match_cursor = ((self.match_cursor as isize + delta).rem_euclid(n)) as usize;
        let p = self.match_list[self.match_cursor].clone();
        self.sel.clear();
        self.sel.insert(p.clone());
        self.focus = Pane::Main;
        self.reveal_in_canvas(&p);
    }

    /// Pans (and if necessary zooms) so that the item is visible. Does nothing
    /// if it already is.
    pub fn reveal_in_canvas(&mut self, path: &Path) {
        if let Some(it) = self.layout.get(path) {
            let r = it.outer();
            let vp = self.main_rect.shrink(40.0);
            let screen = self.cam.rect_to_screen(self.main_rect, r);
            if vp.contains_rect(screen) {
                return;
            }
            let zoom = self.cam.target_zoom();
            let fits = r.width() * zoom < vp.width() && r.height() * zoom < vp.height();
            if fits && zoom > 0.25 {
                self.cam.animate_to(r.center(), zoom);
            } else {
                self.cam.fit(self.main_rect, r, true);
            }
        }
    }

    // ------------------------------------------------------------------
    // Per-frame bookkeeping

    pub fn toggle_mode(&mut self) {
        self.tool = if self.tool == Tool::Pan { Tool::Select } else { Tool::Pan };
    }

    /// Human readable name of the toggle key, e.g. "Alt (tap)".
    pub fn toggle_key_label(&self) -> String {
        let k = &self.settings.toggle_key;
        if modifier_state(k, egui::Modifiers::NONE).is_some() { format!("{k} (tap)") } else { k.clone() }
    }

    /// Switches between the select and the hand tool with the configured key.
    /// Modifier keys toggle when tapped alone (pressed and released with no
    /// other key, click or scroll in between); other keys toggle on press.
    fn mode_toggle_input(&mut self, ctx: &egui::Context) {
        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));
        if self.capturing_key {
            if egui::Popup::is_any_open(ctx) {
                self.capture_toggle_key(&events, mods);
            } else {
                self.capturing_key = false;
            }
            self.prev_mods = mods;
            return;
        }
        let typing = ctx.egui_wants_keyboard_input() || self.rename.is_some();
        let key = self.settings.toggle_key.clone();
        if let Some(held) = modifier_state(&key, mods) {
            let was = modifier_state(&key, self.prev_mods).unwrap_or(false);
            let interrupted = events.iter().any(|e| {
                matches!(
                    e,
                    Event::Key { pressed: true, .. }
                        | Event::PointerButton { pressed: true, .. }
                        | Event::MouseWheel { .. }
                        | Event::Text(_)
                        | Event::Copy
                        | Event::Cut
                        | Event::Paste(_)
                )
            });
            if held && !was {
                self.toggle_armed = !typing;
            }
            if held && interrupted {
                self.toggle_armed = false;
            }
            if !held {
                if was && self.toggle_armed && !typing {
                    self.toggle_mode();
                }
                self.toggle_armed = false;
            }
        } else if let Some(k) = Key::from_name(&key) {
            let pressed = events.iter().any(|e| {
                matches!(e, Event::Key { key, pressed: true, repeat: false, modifiers, .. }
                    if *key == k && !modifiers.command && !modifiers.alt)
            });
            if pressed && !typing && !egui::Popup::is_any_open(ctx) {
                self.toggle_mode();
            }
        }
        self.prev_mods = mods;
    }

    /// Takes the next key (or modifier) pressed as the new toggle key; Escape cancels.
    fn capture_toggle_key(&mut self, events: &[Event], mods: egui::Modifiers) {
        for e in events {
            if let Event::Key { key, pressed: true, .. } = e {
                if *key != Key::Escape {
                    self.settings.toggle_key = key.name().to_string();
                }
                self.capturing_key = false;
                return;
            }
        }
        let prev = self.prev_mods;
        for (name, now, was) in [("Alt", mods.alt, prev.alt), ("Ctrl", mods.ctrl, prev.ctrl), ("Shift", mods.shift, prev.shift)] {
            if now && !was {
                self.settings.toggle_key = name.to_string();
                self.capturing_key = false;
                self.toggle_armed = false;
                return;
            }
        }
    }

    /// Opens the operating system's folder dialog (in a thread, so the canvas
    /// keeps painting); the chosen folder becomes the canvas root.
    pub fn pick_root_folder(&mut self, ctx: &egui::Context) {
        if self.folder_pick_rx.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let start = self.root.clone();
        // Scripted UI checks answer the dialog without showing it.
        let stub = std::env::var_os("PLANAR_SCRIPT_PICK").filter(|_| self.script.is_some()).map(PathBuf::from);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let picked = match stub {
                Some(p) => Some(p),
                None => rfd::FileDialog::new().set_title("Choose the canvas root folder").set_directory(&start).pick_folder(),
            };
            let _ = tx.send(picked);
            ctx.request_repaint();
        });
        self.folder_pick_rx = Some(rx);
    }

    fn poll_folder_pick(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.folder_pick_rx else { return };
        match rx.try_recv() {
            Ok(picked) => {
                self.folder_pick_rx = None;
                if let Some(p) = picked {
                    self.set_root(ctx, p, true);
                }
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.folder_pick_rx = None,
            Err(_) => {}
        }
    }

    fn tick(&mut self, ctx: &egui::Context) {
        self.poll_folder_pick(ctx);
        self.poll_scan(ctx);
        self.poll_jobs(ctx);
        for rx in &self.stat_rx {
            while let Ok((p, s)) = rx.try_recv() {
                self.dir_stats.insert(p, Some(s));
            }
        }
        if self.layout_dirty {
            self.relayout();
        }
        if self.needs_fit && !self.layout.items.is_empty() && self.main_rect.width() > 10.0 {
            self.needs_fit = false;
            let bounds = self.layout.bounds;
            self.cam.fit_top(self.main_rect, bounds);
        } else if self.needs_fit && !self.is_scanning() && self.layout.items.is_empty() {
            self.needs_fit = false;
            self.cam = Camera::default();
            self.cam.offset = vec2(-40.0, -40.0);
        }
        if !self.pending_select.is_empty() && !self.is_scanning() {
            let found: Vec<PathBuf> = self.pending_select.iter().filter(|p| self.layout.by_path.contains_key(*p)).cloned().collect();
            if !found.is_empty() {
                self.sel = found.into_iter().collect();
                self.focus = Pane::Main;
            }
            self.pending_select.clear();
        }
        if let Some(p) = self.pending_rename.clone() {
            if self.layout.by_path.contains_key(&p) {
                self.pending_rename = None;
                self.sel.clear();
                self.sel.insert(p.clone());
                self.reveal_in_canvas(&p);
                self.start_rename(&p);
            } else if !self.is_scanning() && !p.exists() {
                self.pending_rename = None;
            }
        }
        // Poll for external changes of the loaded folders.
        let t = now();
        if t - self.last_poll > 2.5 && !self.is_scanning() && self.jobs.is_empty() && matches!(self.gesture, Gesture::None) {
            self.last_poll = t;
            let changed = self
                .tree
                .dir_mtimes
                .iter()
                .take(3000)
                .any(|(p, m)| std::fs::metadata(p).and_then(|md| md.modified()).ok() != *m);
            if changed {
                self.request_scan(ctx);
                for c in &mut self.clip {
                    c.refresh();
                }
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(2600));
        self.toasts.retain(|t| t.until > now());
        if !self.toasts.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    fn request_dir_stats(&mut self, path: &Path) {
        if self.dir_stats.contains_key(path) {
            return;
        }
        self.dir_stats.insert(path.to_path_buf(), None);
        self.stat_cancel.store(true, Ordering::Relaxed);
        self.stat_cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.stat_cancel.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let p = path.to_path_buf();
        std::thread::spawn(move || {
            let s = fsmodel::dir_stats(&p, &cancel);
            if !cancel.load(Ordering::Relaxed) {
                let _ = tx.send((p, s));
            }
        });
        self.stat_rx.push(rx);
        if self.stat_rx.len() > 8 {
            self.stat_rx.remove(0);
        }
    }

    // ------------------------------------------------------------------
    // UI: top bar

    fn top_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            // Brand
            let (r, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
            paint_logo(ui.painter(), r);
            ui.label(RichText::new("Planar").font(theme::semibold(15.0)).color(theme::INK));
            ui.add_space(14.0);
            if uicons::button(ui, Ico::Back, false, !self.back.is_empty(), "Back (Alt+\u{2190})").clicked() {
                self.go_back(&ctx);
            }
            if uicons::button(ui, Ico::Forward, false, !self.fwd.is_empty(), "Forward (Alt+\u{2192})").clicked() {
                self.go_forward(&ctx);
            }
            if uicons::button(ui, Ico::Up, false, self.root.parent().is_some(), "Parent folder (Alt+\u{2191})").clicked() {
                self.go_up(&ctx);
            }
            if uicons::button(ui, Ico::Refresh, false, true, "Refresh (F5)").clicked() {
                self.request_scan(&ctx);
            }
            ui.add_space(6.0);

            // Right side is laid out from the right edge.
            ui.with_layout(ELayout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if uicons::button(ui, Ico::Help, self.show_help, true, "Shortcuts & help (F1)").clicked() {
                    self.show_help = !self.show_help;
                }
                let view_btn = uicons::button(ui, Ico::Gear, false, true, "View options");
                egui::Popup::menu(&view_btn).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                    self.view_menu(ui, &ctx);
                });
                if uicons::button(ui, Ico::Info, self.settings.show_props, true, "Properties panel (Ctrl+I)").clicked() {
                    self.settings.show_props = !self.settings.show_props;
                }
                if uicons::button(ui, Ico::Clipboard, self.settings.show_clip, true, "Clipboard area (Ctrl+B)").clicked() {
                    self.settings.show_clip = !self.settings.show_clip;
                }
                ui.add_space(6.0);
                self.search_box(ui);
                ui.add_space(6.0);
                let places = ui.add(Button::new("Places     ").corner_radius(6));
                uicons::chevron(ui.painter(), pos2(places.rect.right() - 14.0, places.rect.center().y), 8.0, true, theme::MUTED);
                egui::Popup::menu(&places).show(|ui| self.places_menu(ui, &ctx));
                ui.add_space(4.0);
                let picking = self.folder_pick_rx.is_some();
                if folder_button(ui, !picking).on_hover_text("Choose the root folder with the system file browser (Ctrl+O)").clicked() {
                    self.pick_root_folder(&ctx);
                }
                ui.add_space(2.0);
                // Path / breadcrumb field fills the remaining space.
                let w = ui.available_width();
                ui.allocate_ui_with_layout(vec2(w, 32.0), ELayout::left_to_right(Align::Center), |ui| {
                    self.path_field(ui, &ctx);
                });
            });
        });
    }

    fn path_field(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let w = ui.available_width();
        if self.editing_path {
            let id = Id::unique("path-edit");
            let resp = ui.add(
                TextEdit::singleline(&mut self.path_input)
                    .id(id)
                    .desired_width(w)
                    .margin(Margin::symmetric(10, 6))
                    .hint_text("Type a folder path and press Enter"),
            );
            if !resp.has_focus() && !resp.lost_focus() {
                resp.request_focus();
            }
            if resp.lost_focus() {
                self.editing_path = false;
                if ui.input(|i| i.key_pressed(Key::Enter)) {
                    let p = expand_tilde(self.path_input.trim());
                    self.set_root(ctx, p, true);
                } else {
                    self.path_input = self.root.display().to_string();
                }
            }
            return;
        }
        // Breadcrumbs
        let frame = Frame::new()
            .fill(theme::SURFACE)
            .stroke(Stroke::new(1.0, theme::LINE))
            .corner_radius(7)
            .inner_margin(Margin::symmetric(6, 2));
        let resp = frame
            .show(ui, |ui| {
                ui.set_width(w - 14.0);
                ui.set_height(26.0);
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                    crate::icons::paint_folder_icon(ui.painter(), r.shrink(2.0), 1.0, true);
                    ui.add_space(4.0);
                    let comps = path_components(&self.root);
                    // Show as many trailing components as fit.
                    let avail = ui.available_width() - 30.0;
                    let mut used = 0.0;
                    let mut first = comps.len();
                    for (i, (name, _)) in comps.iter().enumerate().rev() {
                        let est = name.chars().count() as f32 * 7.4 + 26.0;
                        if used + est > avail && i + 1 < comps.len() {
                            break;
                        }
                        used += est;
                        first = i;
                    }
                    if first > 0 {
                        ui.label(RichText::new("\u{2026}").color(theme::MUTED));
                        ui.label(RichText::new(" \u{203A} ").color(theme::FAINT));
                    }
                    for (i, (name, path)) in comps.iter().enumerate().skip(first) {
                        let last = i + 1 == comps.len();
                        let text = if last { RichText::new(name).font(theme::semibold(13.0)) } else { RichText::new(name).color(theme::MUTED) };
                        let b = ui.add(Button::new(text).frame(false).min_size(vec2(0.0, 22.0)));
                        if b.clicked() && !last {
                            self.set_root(ctx, path.clone(), true);
                        }
                        if !last {
                            ui.label(RichText::new(" \u{203A} ").color(theme::FAINT));
                        }
                    }
                });
            })
            .response;
        let click = ui.interact(resp.rect, Id::unique("crumb-bg"), Sense::click());
        if click.clicked() {
            self.editing_path = true;
            self.path_input = self.root.display().to_string();
        }
        click.on_hover_text("Click to type a path (Ctrl+L)");
    }

    fn search_box(&mut self, ui: &mut Ui) {
        let id = Id::unique("search");
        let frame = Frame::new().fill(theme::SURFACE).stroke(Stroke::new(1.0, theme::LINE)).corner_radius(7).inner_margin(Margin::symmetric(6, 0));
        frame.show(ui, |ui| {
            ui.set_height(30.0);
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if !self.search.is_empty() {
                    let clear = uicons::button(ui, Ico::Close, false, true, "Clear search");
                    if clear.clicked() {
                        self.search.clear();
                        self.update_matches();
                    }
                    let label = if self.match_list.is_empty() { "0".to_string() } else { format!("{}/{}", self.match_cursor + 1, self.match_list.len()) };
                    ui.label(RichText::new(label).small().color(theme::MUTED));
                }
                let resp = ui.add(TextEdit::singleline(&mut self.search).id(id).frame(Frame::NONE).desired_width(150.0).hint_text("Search canvas\u{2026}"));
                if self.focus_search {
                    self.focus_search = false;
                    resp.request_focus();
                }
                if resp.changed() {
                    self.match_cursor = 0;
                    self.update_matches();
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    let back = ui.input(|i| i.modifiers.shift);
                    self.jump_to_match(if back { -1 } else { 1 });
                    resp.request_focus();
                }
                let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                uicons::paint(ui.painter(), r, Ico::Search, theme::MUTED);
            });
        });
    }

    fn places_menu(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        ui.set_min_width(220.0);
        ui.label(RichText::new("PLACES").small().color(theme::FAINT));
        for (name, path) in places() {
            if ui.add(Button::new(name).right_text(RichText::new(short_path(&path)).small().color(theme::FAINT))).clicked() {
                self.set_root(ctx, path, true);
                ui.close();
            }
        }
        ui.separator();
        ui.label(RichText::new("THIS COMPUTER").small().color(theme::FAINT));
        for (name, path) in drives() {
            if ui.button(name).clicked() {
                self.set_root(ctx, path, true);
                ui.close();
            }
        }
    }

    fn view_menu(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        ui.set_min_width(260.0);
        let mut rescan = false;
        ui.label(RichText::new("CANVAS BACKGROUND").small().color(theme::FAINT));
        ui.horizontal(|ui| {
            for (name, c) in theme::CANVAS_PRESETS {
                let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
                let selected = self.settings.canvas_bg == c;
                ui.painter().rect(r.shrink(2.0), 6.0, c, Stroke::new(if selected { 2.0 } else { 1.0 }, if selected { theme::ACCENT } else { theme::LINE }), egui::StrokeKind::Inside);
                if resp.on_hover_text(name).clicked() {
                    self.settings.canvas_bg = c;
                }
            }
            ui.color_edit_button_srgba(&mut self.settings.canvas_bg).on_hover_text("Custom colour");
        });
        ui.checkbox(&mut self.settings.dot_grid, "Dot grid");
        ui.checkbox(&mut self.settings.minimap, "Minimap");
        ui.checkbox(&mut self.settings.wheel_zooms, "Mouse wheel zooms (otherwise pans)");
        ui.separator();
        ui.label(RichText::new("TOOLS").small().color(theme::FAINT));
        ui.horizontal(|ui| {
            ui.label("Select / Hand toggle key");
            let text = if self.capturing_key { "Press a key\u{2026}".to_string() } else { self.toggle_key_label() };
            let b = Button::new(RichText::new(text).monospace())
                .selected(self.capturing_key)
                .fill(theme::SURFACE)
                .stroke(Stroke::new(1.0, if self.capturing_key { theme::ACCENT } else { theme::LINE }))
                .corner_radius(6);
            if ui
                .add(b)
                .on_hover_text("Click, then press the key to use (Escape cancels). Alt, Ctrl and Shift toggle when tapped alone.")
                .clicked()
            {
                self.capturing_key = !self.capturing_key;
            }
            if self.settings.toggle_key != "Alt" && !self.capturing_key && pill(ui, "Reset").clicked() {
                self.settings.toggle_key = "Alt".into();
            }
        });
        ui.label(RichText::new("A middle-button click toggles too; dragging with it pans.").small().color(theme::MUTED));
        ui.separator();
        ui.label(RichText::new("CONTENT").small().color(theme::FAINT));
        rescan |= ui.checkbox(&mut self.settings.show_hidden, "Show hidden items").changed();
        ui.horizontal(|ui| {
            ui.label("Auto-expand depth");
            rescan |= ui.add(egui::Slider::new(&mut self.settings.auto_depth, 0..=8)).changed();
        });
        ui.horizontal(|ui| {
            ui.label("Sort by");
            egui::ComboBox::from_id_salt("sort").selected_text(self.settings.sort.label()).show_ui(ui, |ui| {
                for k in SortKey::ALL {
                    rescan |= ui.selectable_value(&mut self.settings.sort, k, k.label()).changed();
                }
            });
            let label = if self.settings.sort_desc { "\u{2193} Desc" } else { "\u{2191} Asc" };
            rescan |= ui.toggle_value(&mut self.settings.sort_desc, label).changed();
        });
        ui.separator();
        if pill(ui, "Reset all folder sizes").clicked() {
            self.custom_sizes.clear();
            self.layout_dirty = true;
        }
        if pill(ui, "Collapse / expand: reset to defaults").clicked() {
            self.expand.clear();
            rescan = true;
        }
        if rescan {
            self.request_scan(ctx);
        }
    }

    // ------------------------------------------------------------------
    // UI: status bar

    fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            let (mut dirs, mut files) = (0usize, 0usize);
            for it in &self.layout.items {
                match it.kind {
                    Kind::Dir => dirs += 1,
                    Kind::File => files += 1,
                    Kind::More(n) => files += n,
                }
            }
            ui.label(RichText::new(format!("{} \u{00B7} {}", plural(dirs, "folder"), plural(files, "file"))).small().color(theme::MUTED));
            if !self.sel.is_empty() {
                let size: u64 = self.sel.iter().filter_map(|p| self.layout.get(p)).map(|it| self.tree.nodes[it.node].size).sum();
                ui.label(RichText::new(format!("{} selected \u{00B7} {}", self.sel.len(), fsmodel::format_size(size))).small().color(theme::ACCENT));
            }
            if self.tree.budget_exhausted {
                ui.label(RichText::new("Large tree \u{2014} deep folders load on expand").small().color(theme::FAINT))
                    .on_hover_text("To keep the canvas fast, only the first 40,000 entries are loaded automatically. Click the arrow next to a folder name to load it.");
            }
            if self.is_scanning() {
                ui.add(egui::Spinner::new().size(12.0));
                ui.label(RichText::new("Scanning\u{2026}").small().color(theme::MUTED));
            }
            for (_, desc) in &self.jobs {
                ui.add(egui::Spinner::new().size(12.0));
                ui.label(RichText::new(desc).small().color(theme::MUTED));
            }
            ui.with_layout(ELayout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(format!("{:.0}%", self.cam.zoom * 100.0)).small().color(theme::MUTED));
                let key = self.toggle_key_label();
                let hint = match (&self.gesture, self.tool) {
                    (Gesture::Drag { .. }, _) => "Drop on a folder to move \u{00B7} hold Ctrl to copy \u{00B7} drop on the left area to stage in clipboard".to_string(),
                    (_, Tool::Pan) => format!("Drag to move the canvas \u{00B7} {key} or middle click: back to select"),
                    (_, Tool::DrawFolder) => "Drag a rectangle inside a folder to create a new folder of that size".to_string(),
                    _ => format!("Drag to select \u{00B7} {key} or middle click: hand tool \u{00B7} wheel to zoom \u{00B7} double-click a folder to open it"),
                };
                ui.label(RichText::new(hint).small().color(theme::FAINT));
            });
        });
    }

    // ------------------------------------------------------------------
    // UI: properties panel

    fn props_panel(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        ui.add_space(14.0);
        let paths: Vec<PathBuf> = match self.focus {
            Pane::Clip if !self.clip_sel.is_empty() => self.clip_sel.iter().cloned().collect(),
            _ => self.sel.iter().cloned().collect(),
        };
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if paths.len() > 1 {
                self.props_multi(ui, &paths);
            } else {
                let p = paths.first().cloned().unwrap_or_else(|| self.root.clone());
                self.props_single(ui, &ctx, &p, paths.is_empty());
            }
        });
    }

    fn props_single(&mut self, ui: &mut Ui, ctx: &egui::Context, path: &Path, is_root: bool) {
        let md = std::fs::metadata(path).ok();
        let lmd = std::fs::symlink_metadata(path).ok();
        let is_dir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let name = fsmodel::display_name(path);
        ui.vertical_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
            if is_dir {
                crate::icons::paint_folder_icon(ui.painter(), r, 1.0, false);
            } else {
                crate::icons::paint_file_icon(ui.painter(), r, &name, 1.0);
            }
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(&name).font(theme::semibold(15.0)).color(theme::INK)).wrap());
            let kind = if is_root { "Canvas root folder".to_string() } else { crate::icons::type_label(&name, is_dir) };
            ui.label(RichText::new(kind).color(theme::MUTED));
        });
        ui.add_space(10.0);
        if md.is_none() {
            ui.colored_label(theme::DANGER, "This item no longer exists.");
            return;
        }
        let md = md.unwrap();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            if !is_root && pill(ui, if is_dir { "Open on canvas" } else { "Open" }).clicked() {
                self.open_item(ctx, path);
            }
            if pill(ui, "Show in system").clicked()
                && let Err(e) = ops::reveal(path) {
                    self.toast(e, ToastKind::Error);
                }
            if pill(ui, "Copy path").clicked() {
                ctx.copy_text(path.display().to_string());
                self.toast("Path copied".into(), ToastKind::Info);
            }
        });
        ui.add_space(8.0);
        section(ui, "DETAILS");
        let size_text = if is_dir {
            self.request_dir_stats(path);
            match self.dir_stats.get(path).cloned().flatten() {
                Some((s, f, d)) => format!("{}  ({} B)\n{}, {}", fsmodel::format_size(s), group_digits(s), plural(f, "file"), plural(d, "folder")),
                None => "Calculating\u{2026}".into(),
            }
        } else {
            format!("{}  ({} B)", fsmodel::format_size(md.len()), group_digits(md.len()))
        };
        prop_row(ui, "Location", &path.parent().map(|p| p.display().to_string()).unwrap_or_else(|| "\u{2014}".into()));
        prop_row(ui, if is_dir { "Size / contents" } else { "Size" }, &size_text);
        prop_row(ui, "Modified", &fsmodel::format_time(md.modified().ok()));
        prop_row(ui, "Created", &fsmodel::format_time(md.created().ok()));
        prop_row(ui, "Accessed", &fsmodel::format_time(md.accessed().ok()));
        let mut attrs = vec![];
        if md.permissions().readonly() {
            attrs.push("Read-only");
        }
        if fsmodel::is_hidden(&name, Some(&md)) {
            attrs.push("Hidden");
        }
        if lmd.map(|m| m.file_type().is_symlink()).unwrap_or(false) {
            attrs.push("Symbolic link");
        }
        prop_row(ui, "Attributes", &if attrs.is_empty() { "\u{2014}".to_string() } else { attrs.join(", ") });
        if let Ok(t) = std::fs::read_link(path) {
            prop_row(ui, "Link target", &t.display().to_string());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            use std::os::unix::fs::PermissionsExt;
            prop_row(ui, "Permissions", &unix_mode(md.permissions().mode()));
            prop_row(ui, "Owner (uid:gid)", &format!("{}:{}", md.uid(), md.gid()));
        }
        if !is_root {
            ui.add_space(10.0);
            section(ui, "ACTIONS");
            ui.horizontal_wrapped(|ui| {
                if pill(ui, "Rename").clicked() {
                    self.sel = [path.to_path_buf()].into_iter().collect();
                    self.start_rename(path);
                }
                if pill(ui, "Cut").clicked() {
                    self.add_to_clip(&[path.to_path_buf()], ClipMode::Move, None);
                }
                if pill(ui, "Copy").clicked() {
                    self.add_to_clip(&[path.to_path_buf()], ClipMode::Copy, None);
                }
                if pill(ui, RichText::new("Move to trash").color(theme::DANGER)).clicked() {
                    self.sel = [path.to_path_buf()].into_iter().collect();
                    self.trash_selected(ctx);
                }
            });
        } else {
            ui.add_space(14.0);
            hint_box(ui, "Tip: click any file or folder on the canvas to see its properties here. Drag items to the clipboard area on the left to stage them for moving.");
        }
    }

    fn props_multi(&mut self, ui: &mut Ui, paths: &[PathBuf]) {
        ui.vertical_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
            let p = ui.painter();
            for k in (0..3).rev() {
                let off = vec2(k as f32 * 6.0 - 6.0, -(k as f32) * 5.0 + 5.0);
                crate::icons::paint_file_icon(p, r.translate(off).shrink(6.0), "", 1.0 - k as f32 * 0.2);
            }
            ui.label(RichText::new(format!("{} items selected", paths.len())).font(theme::semibold(15.0)).color(theme::INK));
        });
        ui.add_space(10.0);
        let mut files = 0;
        let mut dirs = 0;
        let mut size = 0u64;
        for p in paths {
            if let Ok(m) = std::fs::metadata(p) {
                if m.is_dir() {
                    dirs += 1;
                } else {
                    files += 1;
                    size += m.len();
                }
            }
        }
        section(ui, "DETAILS");
        prop_row(ui, "Contains", &format!("{}, {}", plural(files, "file"), plural(dirs, "folder")));
        prop_row(ui, "Size of files", &fsmodel::format_size(size));
        ui.add_space(8.0);
        section(ui, "ITEMS");
        for p in paths.iter().take(14) {
            ui.label(RichText::new(fsmodel::display_name(p)).color(theme::TEXT));
        }
        if paths.len() > 14 {
            ui.label(RichText::new(format!("\u{2026} and {} more", paths.len() - 14)).color(theme::MUTED));
        }
    }

    // ------------------------------------------------------------------
    // Context menus

    pub fn context_menu_ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        ui.set_min_width(230.0);
        let Some(target) = self.menu.clone() else {
            ui.close();
            return;
        };
        let clip_n = self.clip.len();
        let paste_label = if clip_n == 0 { "Paste".to_string() } else { format!("Paste {} here", plural(if self.clip_sel.is_empty() { clip_n } else { self.clip_sel.len() }, "item")) };
        match target {
            MenuTarget::Item(path) => {
                let is_dir = path.is_dir();
                let multi = self.sel.len() > 1;
                if is_dir {
                    if menu_item(ui, "Open on canvas", "Double-click", !multi) {
                        self.set_root(&ctx, path.clone(), true);
                    }
                    let expanded = self.layout.get(&path).map(|it| !it.collapsed).unwrap_or(false);
                    if menu_item(ui, if expanded { "Collapse" } else { "Expand" }, "", true) {
                        self.toggle_expand(&path);
                    }
                    if menu_item(ui, &paste_label, "Ctrl+V", clip_n > 0) {
                        self.paste_into(&ctx, &path);
                    }
                    ui.separator();
                    if menu_item(ui, "New folder", "Ctrl+Shift+N", true) {
                        self.new_folder(&ctx, &path, None);
                    }
                    if menu_item(ui, "New text file", "Ctrl+N", true) {
                        self.new_file(&ctx, &path);
                    }
                } else if menu_item(ui, "Open", "Enter", !multi) {
                    self.open_item(&ctx, &path);
                }
                ui.separator();
                if menu_item(ui, "Cut", "Ctrl+X", true) {
                    self.cut_or_copy(ClipMode::Move, &ctx);
                }
                if menu_item(ui, "Copy", "Ctrl+C", true) {
                    self.cut_or_copy(ClipMode::Copy, &ctx);
                }
                if menu_item(ui, "Rename", "F2", !multi) {
                    self.start_rename(&path);
                }
                ui.separator();
                if is_dir && menu_item(ui, "Set as canvas root", "", !multi) {
                    self.set_root(&ctx, path.clone(), true);
                }
                if is_dir && self.custom_sizes.contains_key(&path) && menu_item(ui, "Reset size", "", true) {
                    self.custom_sizes.remove(&path);
                    self.layout_dirty = true;
                }
                if menu_item(ui, "Show in system file manager", "", !multi)
                    && let Err(e) = ops::reveal(&path) {
                        self.toast(e, ToastKind::Error);
                    }
                if menu_item(ui, "Copy path", "", true) {
                    ctx.copy_text(self.sel.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"));
                }
                ui.separator();
                if menu_item_danger(ui, "Move to trash", "Del") {
                    self.trash_selected(&ctx);
                }
                if menu_item_danger(ui, "Delete permanently\u{2026}", "Shift+Del") {
                    self.confirm_delete_selected();
                }
            }
            MenuTarget::Background => {
                let root = self.root.clone();
                if menu_item(ui, &paste_label, "Ctrl+V", clip_n > 0) {
                    self.paste_into(&ctx, &root);
                }
                ui.separator();
                if menu_item(ui, "New folder", "Ctrl+Shift+N", true) {
                    self.new_folder(&ctx, &root, None);
                }
                if menu_item(ui, "New text file", "Ctrl+N", true) {
                    self.new_file(&ctx, &root);
                }
                ui.separator();
                if menu_item(ui, "Select all", "Ctrl+A", true) {
                    self.select_all();
                }
                if menu_item(ui, "Fit canvas to window", "Ctrl+0", true) {
                    self.cam.fit(self.main_rect, self.layout.bounds, true);
                }
                if menu_item(ui, "Refresh", "F5", true) {
                    self.request_scan(&ctx);
                }
                if menu_item(ui, "Go to parent folder", "Alt+\u{2191}", self.root.parent().is_some()) {
                    self.go_up(&ctx);
                }
                ui.separator();
                if menu_item(ui, "Show in system file manager", "", true) {
                    let _ = ops::open_default(&root);
                }
            }
            MenuTarget::ClipItem(path) => {
                let root = self.root.clone();
                let target_dir = self.target_dir();
                let tname = fsmodel::display_name(&target_dir);
                if menu_item(ui, &format!("Paste into \u{201c}{tname}\u{201d}"), "", true) {
                    self.paste_into(&ctx, &target_dir);
                }
                if target_dir != root && menu_item(ui, "Paste into canvas root", "", true) {
                    self.paste_into(&ctx, &root);
                }
                ui.separator();
                let mode = self.clip.iter().find(|c| c.path == path).map(|c| c.mode).unwrap_or(ClipMode::Move);
                let (label, new_mode) = if mode == ClipMode::Move { ("Switch to copy", ClipMode::Copy) } else { ("Switch to move", ClipMode::Move) };
                if menu_item(ui, label, "", true) {
                    for c in &mut self.clip {
                        if self.clip_sel.contains(&c.path) {
                            c.mode = new_mode;
                        }
                    }
                }
                if menu_item(ui, "Show on canvas", "", self.layout.by_path.contains_key(&path)) {
                    self.sel = [path.clone()].into_iter().collect();
                    self.reveal_in_canvas(&path);
                }
                if menu_item(ui, "Open", "", true) {
                    self.open_item(&ctx, &path);
                }
                ui.separator();
                if menu_item(ui, "Remove from clipboard", "Del", true) {
                    self.remove_clip_selected();
                }
            }
            MenuTarget::ClipBackground => {
                if menu_item(ui, "Arrange items", "", clip_n > 0) {
                    self.arrange_clip();
                }
                if menu_item(ui, "Select all", "Ctrl+A", clip_n > 0) {
                    self.clip_sel = self.clip.iter().map(|c| c.path.clone()).collect();
                }
                if menu_item(ui, "Clear clipboard", "", clip_n > 0) {
                    self.clip.clear();
                    self.clip_sel.clear();
                }
            }
        }
    }

    pub fn remove_clip_selected(&mut self) {
        let sel = std::mem::take(&mut self.clip_sel);
        self.clip.retain(|c| !sel.contains(&c.path));
    }

    pub fn select_all(&mut self) {
        let root_id = self.tree.root;
        self.sel = self
            .layout
            .items
            .iter()
            .filter(|it| !matches!(it.kind, Kind::More(_)) && it.depth == self.tree.nodes[root_id].depth + 1)
            .map(|it| self.tree.nodes[it.node].path.clone())
            .collect();
        self.focus = Pane::Main;
    }

    // ------------------------------------------------------------------
    // Overlays: rename editor, toasts, dialogs, help

    fn rename_overlay(&mut self, ctx: &egui::Context) {
        let Some(r) = &mut self.rename else { return };
        let Some(it) = self.layout.get(&r.path) else {
            if !self.is_scanning() {
                self.rename = None;
            }
            return;
        };
        let is_dir = it.kind == Kind::Dir;
        let z = self.cam.zoom;
        let vp = self.main_rect;
        let (pos, width) = if is_dir {
            let lr = self.cam.rect_to_screen(vp, it.label);
            let s = if lr.width() >= 56.0 { z.max(0.78) } else { z };
            (pos2(lr.left() + 33.0 * s, lr.bottom() - 6.0 * s.min(1.0) - 7.5 * s - 13.0), (lr.width() - 30.0).clamp(160.0, 420.0))
        } else {
            let lr = self.cam.rect_to_screen(vp, it.label);
            let w = (lr.width() + 40.0).clamp(140.0, 300.0);
            (pos2(lr.center().x - w / 2.0, lr.top() - 2.0), w)
        };
        let mut commit = false;
        let mut cancel = false;
        egui::Area::new(Id::unique("rename-area")).order(Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
            let id = Id::unique("rename-edit");
            let resp = ui.add(
                TextEdit::singleline(&mut r.text)
                    .id(id)
                    .desired_width(width)
                    .margin(Margin::symmetric(6, 4))
                    .font(egui::FontId::proportional(13.0)),
            );
            if r.just_started {
                r.just_started = false;
                resp.request_focus();
                // Select the name without its extension, like Explorer.
                let stem_len = if is_dir { r.text.chars().count() } else { ops::split_name(&r.text).0.chars().count() };
                if let Some(mut state) = TextEdit::load_state(ui.ctx(), id) {
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(stem_len),
                    )));
                    state.store(ui.ctx(), id);
                }
            } else if resp.lost_focus() {
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    cancel = true;
                } else {
                    commit = true;
                }
            }
        });
        if cancel {
            self.rename = None;
        } else if commit {
            self.commit_rename(ctx);
        }
    }

    fn toasts_overlay(&mut self, ctx: &egui::Context) {
        if self.toasts.is_empty() {
            return;
        }
        let mut do_undo = false;
        let anchor = pos2(self.main_rect.center().x, self.main_rect.bottom() - 20.0);
        egui::Area::new(Id::unique("toasts")).order(Order::Foreground).pivot(Align2::CENTER_BOTTOM).fixed_pos(anchor).interactable(true).show(ctx, |ui| {
            for t in &self.toasts {
                let (bg, fg) = match t.kind {
                    ToastKind::Error => (Color32::from_rgb(0xFD, 0xEC, 0xEA), theme::DANGER),
                    ToastKind::Success => (theme::INK, Color32::WHITE),
                    ToastKind::Info => (theme::INK, Color32::WHITE),
                };
                Frame::new()
                    .fill(bg)
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(14, 9))
                    .shadow(egui::Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(40) })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&t.text).color(fg));
                            if t.undo {
                                ui.add_space(10.0);
                                if ui.add(Button::new(RichText::new("Undo").font(theme::semibold(13.0)).color(Color32::from_rgb(0x8F, 0xB5, 0xFF))).frame(false)).clicked() {
                                    do_undo = true;
                                }
                            }
                        });
                    });
                ui.add_space(6.0);
            }
        });
        if do_undo {
            self.toasts.retain(|t| !t.undo);
            self.undo(ctx);
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(Confirm::DeletePermanently(items)) = &self.confirm {
            let items = items.clone();
            let mut close = false;
            let mut go = false;
            egui::Modal::new(Id::unique("confirm-delete")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(RichText::new("Delete permanently?").font(theme::semibold(16.0)));
                ui.add_space(6.0);
                let what = if items.len() == 1 { format!("\u{201c}{}\u{201d}", fsmodel::display_name(&items[0])) } else { plural(items.len(), "item") };
                ui.label(format!("{what} will be deleted permanently. This cannot be undone."));
                ui.add_space(12.0);
                ui.with_layout(ELayout::right_to_left(Align::Center), |ui| {
                    let del = Button::new(RichText::new("Delete").color(Color32::WHITE)).fill(theme::DANGER);
                    if ui.add(del).clicked() {
                        go = true;
                    }
                    if pill(ui, "Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        close = true;
                    }
                });
            });
            if go {
                let pairs = items.into_iter().map(|p| (p.clone(), p)).collect();
                self.spawn_job(ctx, Job { kind: JobKind::Delete, pairs, is_undo: false });
                close = true;
            }
            if close {
                self.confirm = None;
            }
        }
        if self.show_help {
            let toggle_keys = format!("{} / middle click", self.toggle_key_label());
            let resp = egui::Modal::new(Id::unique("help")).show(ctx, |ui| {
                ui.set_width(560.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Planar \u{2014} keyboard & mouse").font(theme::semibold(17.0)));
                    ui.with_layout(ELayout::right_to_left(Align::Center), |ui| {
                        if uicons::button(ui, Ico::Close, false, true, "").clicked() {
                            self.show_help = false;
                        }
                    });
                });
                ui.add_space(8.0);
                ui.columns(2, |cols| {
                    help_group(&mut cols[0], "Canvas", &[
                        ("Wheel", "Zoom at cursor"),
                        ("Space + drag", "Pan"),
                        ("Middle drag", "Pan"),
                        (toggle_keys.as_str(), "Toggle Select / Hand"),
                        ("V / H / F", "Select / Hand / Draw folder"),
                        ("Ctrl+O", "Choose root folder (system dialog)"),
                        ("Ctrl+0", "Fit to window"),
                        ("Ctrl+1", "Zoom 100%"),
                        ("Ctrl + / \u{2212}", "Zoom in / out"),
                        ("Ctrl+F", "Search"),
                        ("Ctrl+L", "Edit path"),
                    ]);
                    help_group(&mut cols[1], "Items", &[
                        ("Click / Ctrl+click", "Select / toggle"),
                        ("Drag on empty", "Rubber-band select"),
                        ("Drag items", "Move \u{00B7} Ctrl = copy"),
                        ("Double-click", "Open file / folder as root"),
                        ("Double-click name", "Rename (also F2)"),
                        ("Ctrl+X / C / V", "Cut / copy / paste"),
                        ("Del / Shift+Del", "Trash / delete"),
                        ("Ctrl+Z", "Undo"),
                        ("Alt+\u{2191} / \u{2190} / \u{2192}", "Parent / back / forward"),
                    ]);
                });
                ui.add_space(8.0);
                hint_box(ui, "The clipboard area on the left is a staging canvas: drop items there, then drag them onto any folder. Items marked MOVE are moved, items marked COPY are copied. Resize a folder by dragging its right or bottom edge.");
            });
            if resp.should_close() {
                self.show_help = false;
            }
        }
    }

    // ------------------------------------------------------------------
    // Keyboard shortcuts

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || self.rename.is_some() || self.confirm.is_some() || egui::Popup::is_any_open(ctx) {
            return;
        }
        let ctx2 = ctx.clone();
        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));
        let mut actions: Vec<&'static str> = vec![];
        for e in &events {
            match e {
                Event::Cut if mods.shift && !mods.command => actions.push("delete-perm"),
                Event::Cut => actions.push("cut"),
                Event::Copy => actions.push("copy"),
                Event::Paste(_) | Event::PasteImage(_) => actions.push("paste"),
                Event::Key { key, pressed: true, modifiers, .. } => {
                    let m = *modifiers;
                    let cmd = m.command;
                    if key.name() == self.settings.toggle_key && !cmd && !m.alt {
                        continue; // handled by mode_toggle_input
                    }
                    let a = match key {
                        Key::O if cmd => "pick-root",
                        Key::V if !cmd && !m.alt => "tool-select",
                        Key::H if !cmd => "tool-pan",
                        Key::F if !cmd => "tool-draw",
                        Key::F if cmd => "search",
                        Key::L if cmd => "path",
                        Key::Delete if m.shift => "delete-perm",
                        Key::Delete => "trash",
                        Key::F2 => "rename",
                        Key::F5 => "refresh",
                        Key::F1 => "help",
                        Key::Questionmark => "help",
                        Key::Enter => "open",
                        Key::Escape => "escape",
                        Key::A if cmd => "select-all",
                        Key::Z if cmd => "undo",
                        Key::N if cmd && m.shift => "new-folder",
                        Key::N if cmd => "new-file",
                        Key::B if cmd => "toggle-clip",
                        Key::I if cmd => "toggle-props",
                        Key::M if cmd => "toggle-map",
                        Key::Num0 if cmd => "fit",
                        Key::Num1 if cmd => "zoom-100",
                        Key::Plus | Key::Equals if cmd => "zoom-in",
                        Key::Minus if cmd => "zoom-out",
                        Key::ArrowUp if m.alt => "up",
                        Key::Backspace => "up",
                        Key::ArrowLeft if m.alt => "back",
                        Key::ArrowRight if m.alt => "forward",
                        _ => "",
                    };
                    if !a.is_empty() {
                        actions.push(a);
                    }
                }
                _ => {}
            }
        }
        for a in actions {
            let ctx = &ctx2;
            match a {
                "pick-root" => self.pick_root_folder(ctx),
                "tool-select" => self.tool = Tool::Select,
                "tool-pan" => self.tool = Tool::Pan,
                "tool-draw" => self.tool = Tool::DrawFolder,
                "search" => self.focus_search = true,
                "path" => {
                    self.editing_path = true;
                    self.path_input = self.root.display().to_string();
                }
                "cut" => self.cut_or_copy(ClipMode::Move, ctx),
                "copy" => self.cut_or_copy(ClipMode::Copy, ctx),
                "paste" => {
                    let d = self.target_dir();
                    self.paste_into(ctx, &d);
                }
                "trash" => {
                    if self.focus == Pane::Clip {
                        self.remove_clip_selected();
                    } else {
                        self.trash_selected(ctx);
                    }
                }
                "delete-perm" => self.confirm_delete_selected(),
                "rename" => {
                    if self.sel.len() == 1 {
                        let p = self.sel.iter().next().unwrap().clone();
                        self.start_rename(&p);
                    }
                }
                "refresh" => self.request_scan(ctx),
                "help" => self.show_help = !self.show_help,
                "open" => {
                    if self.sel.len() == 1 {
                        let p = self.sel.iter().next().unwrap().clone();
                        self.open_item(ctx, &p);
                    }
                }
                "escape" => {
                    if !matches!(self.gesture, Gesture::None) {
                        self.gesture = Gesture::None;
                        self.drop = DropTarget::None;
                    } else {
                        self.sel.clear();
                        self.clip_sel.clear();
                        self.tool = Tool::Select;
                    }
                }
                "select-all" => {
                    if self.focus == Pane::Clip {
                        self.clip_sel = self.clip.iter().map(|c| c.path.clone()).collect();
                    } else {
                        self.select_all();
                    }
                }
                "undo" => self.undo(ctx),
                "new-folder" => {
                    let d = self.target_dir();
                    self.new_folder(ctx, &d, None);
                }
                "new-file" => {
                    let d = self.target_dir();
                    self.new_file(ctx, &d);
                }
                "toggle-clip" => self.settings.show_clip = !self.settings.show_clip,
                "toggle-props" => self.settings.show_props = !self.settings.show_props,
                "toggle-map" => self.settings.minimap = !self.settings.minimap,
                "fit" => self.cam.fit(self.main_rect, self.layout.bounds, true),
                "zoom-100" => {
                    let c = self.cam.to_world(self.main_rect, self.main_rect.center());
                    self.cam.animate_to(c, 1.0);
                }
                "zoom-in" => self.cam.zoom_center(self.main_rect, 1.25),
                "zoom-out" => self.cam.zoom_center(self.main_rect, 0.8),
                "up" => self.go_up(ctx),
                "back" => self.go_back(ctx),
                "forward" => self.go_forward(ctx),
                _ => {}
            }
        }
    }

    // ------------------------------------------------------------------
    // Clipboard pane (left split)

    fn clip_pane(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        // Header
        Frame::new().inner_margin(Margin { left: 14, right: 8, top: 10, bottom: 8 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                uicons::paint(ui.painter(), r, Ico::Clipboard, theme::INK);
                ui.label(RichText::new("Clipboard").font(theme::semibold(14.0)).color(theme::INK));
                if !self.clip.is_empty() {
                    ui.label(RichText::new(self.clip.len().to_string()).small().color(theme::MUTED));
                }
                ui.with_layout(ELayout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if uicons::button(ui, Ico::Trash, false, !self.clip.is_empty(), "Clear clipboard (files are not deleted)").clicked() {
                        self.clip.clear();
                        self.clip_sel.clear();
                    }
                    if uicons::button(ui, Ico::Grid, false, !self.clip.is_empty(), "Arrange items").clicked() {
                        self.arrange_clip();
                    }
                    if uicons::button(ui, Ico::Fit, false, !self.clip.is_empty(), "Fit items").clicked() {
                        let b = self.clip.iter().map(crate::canvas::clip_item_rect).fold(Rect::NOTHING, |a, r| a.union(r));
                        if b.is_positive() {
                            self.clip_cam.fit(self.clip_rect, b, true);
                        }
                    }
                });
            });
        });
        let line_y = ui.cursor().top();
        ui.painter().hline(ui.max_rect().x_range(), line_y, Stroke::new(1.0, theme::LINE));
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        self.clip_rect = rect;
        self.canvas_view_input(&ctx, Pane::Clip, &resp);
        self.canvas_press(&ctx, Pane::Clip, &resp);
        let painter = ui.painter_at(rect);
        self.paint_clip(&painter, rect);
        if resp.secondary_clicked() {
            self.menu = Some(self.clip_menu_target(resp.interact_pointer_pos()));
        }
        resp.context_menu(|ui| self.context_menu_ui(ui));
    }

    // ------------------------------------------------------------------
    // Main canvas pane

    fn main_pane(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        self.main_rect = rect;
        self.canvas_view_input(&ctx, Pane::Main, &resp);
        self.canvas_press(&ctx, Pane::Main, &resp);
        self.canvas_double_click(&ctx, &resp);
        let painter = ui.painter_at(rect);
        self.paint_main(&painter, rect);
        if resp.secondary_clicked() {
            self.menu = Some(self.main_menu_target(resp.interact_pointer_pos()));
        }
        resp.context_menu(|ui| self.context_menu_ui(ui));
        self.floating_tools(&ctx, rect);
        self.zoom_widget(&ctx, rect);
        if self.settings.minimap && !self.layout.items.is_empty() {
            self.minimap(&ctx, rect);
        }
        if self.layout.items.is_empty() && !self.is_scanning() {
            self.empty_state(&ctx, rect);
        }
    }

    fn empty_state(&mut self, ctx: &egui::Context, rect: Rect) {
        egui::Area::new(Id::unique("empty-state")).order(Order::Middle).pivot(Align2::CENTER_CENTER).fixed_pos(rect.center()).show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
                crate::icons::paint_folder_icon(ui.painter(), r, 0.9, true);
                ui.label(RichText::new("This folder is empty").font(theme::semibold(16.0)).color(theme::INK));
                ui.label(RichText::new("Create something, or drop files here from your clipboard area.").color(theme::MUTED));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let root = self.root.clone();
                    if pill(ui, "New folder").clicked() {
                        self.new_folder(ctx, &root, None);
                    }
                    if pill(ui, "New text file").clicked() {
                        self.new_file(ctx, &root);
                    }
                });
            });
        });
    }

    fn floating_tools(&mut self, ctx: &egui::Context, rect: Rect) {
        egui::Area::new(Id::unique("tools")).order(Order::Middle).fixed_pos(rect.min + vec2(14.0, 14.0)).show(ctx, |ui| {
            theme::floating_frame().show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.vertical(|ui| {
                    if uicons::button(ui, Ico::Cursor, self.tool == Tool::Select, true, "Select & move (V)").clicked() {
                        self.tool = Tool::Select;
                    }
                    if uicons::button(ui, Ico::Hand, self.tool == Tool::Pan, true, "Hand \u{2014} drag the canvas (H)").clicked() {
                        self.tool = Tool::Pan;
                    }
                    if uicons::button(ui, Ico::DrawRect, self.tool == Tool::DrawFolder, true, "Draw a new folder (F)").clicked() {
                        self.tool = Tool::DrawFolder;
                    }
                    separator_h(ui);
                    let d = self.target_dir();
                    let dn = fsmodel::display_name(&d);
                    if uicons::button(ui, Ico::NewFolder, false, true, &format!("New folder in \u{201c}{dn}\u{201d} (Ctrl+Shift+N)")).clicked() {
                        self.new_folder(ctx, &d, None);
                    }
                    if uicons::button(ui, Ico::NewFile, false, true, &format!("New text file in \u{201c}{dn}\u{201d} (Ctrl+N)")).clicked() {
                        self.new_file(ctx, &d);
                    }
                    separator_h(ui);
                    if uicons::button(ui, Ico::Undo, false, self.can_undo(), "Undo (Ctrl+Z)").clicked() {
                        self.undo(ctx);
                    }
                    if uicons::button(ui, Ico::Trash, false, !self.sel.is_empty(), "Move selection to trash (Del)").clicked() {
                        self.trash_selected(ctx);
                    }
                });
            });
        });
    }

    fn zoom_widget(&mut self, ctx: &egui::Context, rect: Rect) {
        egui::Area::new(Id::unique("zoom")).order(Order::Middle).pivot(Align2::RIGHT_BOTTOM).fixed_pos(rect.right_bottom() - vec2(14.0, 14.0)).show(ctx, |ui| {
            theme::floating_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if uicons::button(ui, Ico::Minus, false, true, "Zoom out (Ctrl+\u{2212})").clicked() {
                        self.cam.zoom_center(rect, 0.8);
                    }
                    let pct = format!("{:.0}%", self.cam.target_zoom() * 100.0);
                    if ui.add(Button::new(RichText::new(pct).font(theme::semibold(12.0))).frame(false).min_size(vec2(48.0, 30.0))).on_hover_text("Reset to 100% (Ctrl+1)").clicked() {
                        let c = self.cam.to_world(rect, rect.center());
                        self.cam.animate_to(c, 1.0);
                    }
                    if uicons::button(ui, Ico::Plus, false, true, "Zoom in (Ctrl++)").clicked() {
                        self.cam.zoom_center(rect, 1.25);
                    }
                    separator_v(ui);
                    if uicons::button(ui, Ico::Fit, false, true, "Fit to window (Ctrl+0)").clicked() {
                        self.cam.fit(rect, self.layout.bounds, true);
                    }
                    if uicons::button(ui, Ico::Map, self.settings.minimap, true, "Minimap (Ctrl+M)").clicked() {
                        self.settings.minimap = !self.settings.minimap;
                    }
                });
            });
        });
    }

    fn minimap(&mut self, ctx: &egui::Context, rect: Rect) {
        let size = vec2(200.0, 130.0);
        egui::Area::new(Id::unique("minimap")).order(Order::Middle).pivot(Align2::RIGHT_BOTTOM).fixed_pos(rect.right_bottom() - vec2(14.0, 66.0)).show(ctx, |ui| {
            theme::floating_frame().inner_margin(Margin::same(6)).show(ui, |ui| {
                let (r, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
                let b = self.layout.bounds.expand(40.0);
                let s = (r.width() / b.width()).min(r.height() / b.height());
                let off = r.center() - b.center().to_vec2() * s;
                let to_mini = |p: Pos2| pos2(p.x * s + off.x, p.y * s + off.y);
                let p = ui.painter_at(r);
                let pal = theme::Palette::for_bg(self.settings.canvas_bg);
                p.rect_filled(r, 4.0, pal.bg);
                for it in &self.layout.items {
                    if it.kind == Kind::Dir && it.depth <= 2 {
                        let mr = Rect::from_min_max(to_mini(it.rect.min), to_mini(it.rect.max));
                        if mr.width() > 1.5 {
                            p.rect(mr, 1.0, pal.bg, Stroke::new(if it.depth == 1 { 1.0 } else { 0.5 }, pal.ink.gamma_multiply(0.6)), egui::StrokeKind::Middle);
                        }
                    } else if it.kind == Kind::File && it.depth == 1 {
                        let mr = Rect::from_min_max(to_mini(it.rect.min), to_mini(it.rect.max)).shrink(1.0);
                        p.rect_filled(mr, 0.0, pal.faint.gamma_multiply(0.5));
                    }
                }
                let vis = self.cam.visible_world(rect);
                let vr = Rect::from_min_max(to_mini(vis.min), to_mini(vis.max)).intersect(r);
                p.rect(vr, 2.0, theme::accent_soft(0.08), Stroke::new(1.5, theme::ACCENT), egui::StrokeKind::Middle);
                if resp.is_pointer_button_down_on()
                    && let Some(pp) = resp.interact_pointer_pos() {
                        let w = pos2((pp.x - off.x) / s, (pp.y - off.y) / s);
                        self.cam.center_on(rect, w);
                    }
            });
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.tick(&ctx);
        self.mode_toggle_input(&ctx);
        self.update_gesture(&ctx);
        self.external_drops(&ctx);

        egui::Panel::top("topbar")
            .exact_size(52.0)
            .frame(Frame::new().fill(theme::SURFACE).inner_margin(Margin::symmetric(12, 0)).stroke(Stroke::new(1.0, theme::LINE)))
            .show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("status")
            .exact_size(26.0)
            .frame(Frame::new().fill(theme::PANEL).inner_margin(Margin::symmetric(12, 0)).stroke(Stroke::new(1.0, theme::LINE)))
            .show(ui, |ui| self.status_bar(ui));
        if self.settings.show_clip {
            egui::Panel::left("clipboard")
                .resizable(true)
                .default_size(280.0)
                .size_range(170.0..=1200.0)
                .frame(Frame::new().fill(theme::CLIP_BG).stroke(Stroke::new(1.0, theme::LINE)))
                .show(ui, |ui| self.clip_pane(ui));
        } else {
            self.clip_rect = Rect::NOTHING;
        }
        if self.settings.show_props {
            egui::Panel::right("props")
                .resizable(true)
                .default_size(290.0)
                .size_range(230.0..=560.0)
                .frame(Frame::new().fill(theme::SURFACE).inner_margin(Margin::symmetric(16, 0)).stroke(Stroke::new(1.0, theme::LINE)))
                .show(ui, |ui| self.props_panel(ui));
        }
        egui::CentralPanel::no_frame().show(ui, |ui| self.main_pane(ui));

        // Shortcuts run before the overlays so that e.g. the Enter which
        // commits a rename is not also treated as "open".
        self.shortcuts(&ctx);
        self.paint_drag_ghost(&ctx);
        self.rename_overlay(&ctx);
        self.toasts_overlay(&ctx);
        self.dialogs(&ctx);
        if self.cam.step(self.main_rect, ui.input(|i| i.stable_dt)) | self.clip_cam.step(self.clip_rect, ui.input(|i| i.stable_dt)) {
            ctx.request_repaint();
        }
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.script_logic(ctx);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if let Some(script) = &mut self.script
            && !script.feed(raw) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let p = Persisted {
            root: Some(self.root.clone()),
            settings: self.settings.clone(),
            clip: self.clip.clone(),
            custom_sizes: self.custom_sizes.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            expand: self.expand.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        };
        eframe::set_value(storage, eframe::APP_KEY, &p);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::PANEL.to_normalized_gamma_f32()
    }
}

impl App {
    /// Files dragged in from the operating system are copied into the folder
    /// under the cursor (or staged on the clipboard canvas).
    fn external_drops(&mut self, ctx: &egui::Context) {
        let (dropped, hover): (Vec<PathBuf>, Option<Pos2>) =
            ctx.input(|i| (i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect(), i.pointer.hover_pos()));
        if let Some(p) = hover {
            self.last_pointer = Some(p);
        }
        if dropped.is_empty() {
            return;
        }
        let pos = hover.or(self.last_pointer).unwrap_or(self.main_rect.center());
        match self.drop_target_at(pos, &[]) {
            DropTarget::Clip(w) => self.add_to_clip(&dropped, ClipMode::Copy, Some(w)),
            DropTarget::Dir(d) => self.transfer(ctx, &dropped, &d, true),
            DropTarget::None => {
                let root = self.root.clone();
                self.transfer(ctx, &dropped, &root, true);
            }
        }
    }

    /// Scripted UI checks (see `script.rs`): screenshots and state dumps.
    fn script_logic(&mut self, ctx: &egui::Context) {
        let Some(script) = &mut self.script else { return };
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
        if let Some(path) = script.pending_shot.take() {
            ctx.request_screenshot(move |img| {
                let mut out = format!("P6\n{} {}\n255\n", img.width(), img.height()).into_bytes();
                for px in &img.pixels {
                    out.extend_from_slice(&[px.r(), px.g(), px.b()]);
                }
                let _ = std::fs::write(&path, out);
            });
        }
        if std::mem::take(&mut script.want_print) {
            eprintln!("--- state ---");
            eprintln!("root: {}", self.root.display());
            eprintln!("items: {}  sel: {:?}", self.layout.items.len(), self.sel);
            eprintln!("clip: {:?}", self.clip.iter().map(|c| (c.path.display().to_string(), c.mode)).collect::<Vec<_>>());
            eprintln!("clip_sel: {:?}", self.clip_sel);
            eprintln!("toasts: {:?}", self.toasts.iter().map(|t| t.text.clone()).collect::<Vec<_>>());
            eprintln!("zoom: {:.3}  offset: {:?}  tool: {:?}", self.cam.zoom, self.cam.offset, self.tool);
            eprintln!("rename: {:?}  custom: {:?}", self.rename.as_ref().map(|r| (&r.path, &r.text)), self.custom_sizes);
            eprintln!("toggle_key: {}  canvas_bg: {:?}  auto_depth: {}", self.settings.toggle_key, self.settings.canvas_bg, self.settings.auto_depth);
        }
    }
}

// ----------------------------------------------------------------------
// Helpers

/// Attaches the scanned subtree `sub` (rooted at the same path) to node `at` of `tree`.
fn graft(tree: &mut Tree, at: usize, sub: Tree) {
    let base = tree.nodes.len();
    let depth0 = tree.nodes[at].depth;
    let map = |id: usize| if id == sub.root { at } else { base + id - 1 };
    let mut sub_nodes = sub.nodes.into_iter();
    let sub_root = sub_nodes.next().unwrap();
    for mut n in sub_nodes {
        n.depth += depth0;
        n.dirs = n.dirs.iter().map(|&d| map(d)).collect();
        n.files = n.files.iter().map(|&f| map(f)).collect();
        tree.nodes.push(n);
    }
    let target = &mut tree.nodes[at];
    target.dirs = sub_root.dirs.iter().map(|&d| map(d)).collect();
    target.files = sub_root.files.iter().map(|&f| map(f)).collect();
    target.loaded = true;
    target.entry_count = sub_root.entry_count;
    target.omitted = sub_root.omitted;
    target.error = sub_root.error;
    tree.dir_mtimes.extend(sub.dir_mtimes);
}

pub fn now() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

pub fn plural(n: usize, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{} {word}s", group_digits(n as u64)) }
}

fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).filter(|p| p.is_dir())
}

fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix('~')
        && let Some(h) = home_dir() {
            return h.join(rest.trim_start_matches(['/', '\\']));
        }
    PathBuf::from(s)
}

/// Removes the `\\?\` prefix that `canonicalize` adds on Windows.
pub fn clean_path(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\")
        && !rest.starts_with("UNC") {
            return PathBuf::from(rest.to_string());
        }
    p
}

fn path_components(p: &Path) -> Vec<(String, PathBuf)> {
    let mut out = vec![];
    let mut acc = PathBuf::new();
    for c in p.components() {
        acc.push(c.as_os_str());
        let name = match c {
            std::path::Component::RootDir => {
                if out.is_empty() {
                    "/".to_string()
                } else {
                    continue;
                }
            }
            std::path::Component::Prefix(px) => px.as_os_str().to_string_lossy().into_owned(),
            other => other.as_os_str().to_string_lossy().into_owned(),
        };
        out.push((name, acc.clone()));
    }
    // On Windows "C:" + "\" -> make the prefix point at the drive root.
    if cfg!(windows)
        && let Some(first) = out.first_mut()
            && !first.1.to_string_lossy().ends_with('\\') {
                first.1 = PathBuf::from(format!("{}\\", first.1.display()));
            }
    out
}

fn places() -> Vec<(String, PathBuf)> {
    let mut v = vec![];
    if let Some(h) = home_dir() {
        v.push(("Home".to_string(), h.clone()));
        for name in ["Desktop", "Documents", "Downloads", "Pictures", "Music", "Videos"] {
            let p = h.join(name);
            if p.is_dir() {
                v.push((name.to_string(), p));
            }
        }
    }
    v
}

fn drives() -> Vec<(String, PathBuf)> {
    if cfg!(windows) {
        (b'A'..=b'Z')
            .map(|c| format!("{}:\\", c as char))
            .filter(|d| Path::new(d).exists())
            .map(|d| (format!("Drive {d}"), PathBuf::from(d)))
            .collect()
    } else {
        let mut v = vec![("File system  /".to_string(), PathBuf::from("/"))];
        for base in ["/media", "/mnt", "/run/media"] {
            if let Ok(rd) = std::fs::read_dir(base) {
                for e in rd.flatten() {
                    let p = e.path();
                    // /media/<user>/<volume>
                    if let Ok(inner) = std::fs::read_dir(&p) {
                        let vols: Vec<PathBuf> = inner.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
                        if base != "/mnt" && !vols.is_empty() {
                            for vp in vols {
                                v.push((fsmodel::display_name(&vp), vp));
                            }
                            continue;
                        }
                    }
                    if p.is_dir() {
                        v.push((fsmodel::display_name(&p), p));
                    }
                }
            }
        }
        v
    }
}

fn short_path(p: &Path) -> String {
    if let Some(h) = home_dir()
        && let Ok(rest) = p.strip_prefix(&h) {
            return if rest.as_os_str().is_empty() { "~".into() } else { format!("~/{}", rest.display()) };
        }
    p.display().to_string()
}

#[cfg(unix)]
fn unix_mode(mode: u32) -> String {
    let mut s = String::new();
    for shift in [6, 3, 0] {
        let m = (mode >> shift) & 7;
        s.push(if m & 4 != 0 { 'r' } else { '-' });
        s.push(if m & 2 != 0 { 'w' } else { '-' });
        s.push(if m & 1 != 0 { 'x' } else { '-' });
    }
    format!("{s}  ({:o})", mode & 0o777)
}

/// Square button showing a folder icon (opens the system folder dialog).
fn folder_button(ui: &mut Ui, enabled: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 32.0), if enabled { Sense::click() } else { Sense::hover() });
    let p = ui.painter();
    let fill = if enabled && resp.hovered() { Color32::from_rgb(0xEC, 0xEF, 0xF3) } else { theme::SURFACE };
    p.rect(rect, 7.0, fill, Stroke::new(1.0, theme::LINE), egui::StrokeKind::Inside);
    let icon = Rect::from_center_size(rect.center(), vec2(20.0, 20.0));
    crate::icons::paint_folder_icon(p, icon, if enabled { 1.0 } else { 0.45 }, true);
    resp
}

/// A secondary button with a subtle outline.
fn pill(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> egui::Response {
    ui.add(Button::new(text).fill(theme::SURFACE).stroke(Stroke::new(1.0, theme::LINE)).corner_radius(6))
}

fn section(ui: &mut Ui, title: &str) {
    ui.label(RichText::new(title).small().color(theme::FAINT));
    ui.add_space(2.0);
}

fn prop_row(ui: &mut Ui, key: &str, value: &str) {
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(vec2(96.0, 18.0), ELayout::left_to_right(Align::Min), |ui| {
            ui.label(RichText::new(format!("{key}:")).color(theme::MUTED));
        });
        ui.add(egui::Label::new(RichText::new(value).color(theme::TEXT)).wrap());
    });
    ui.add_space(2.0);
}

fn hint_box(ui: &mut Ui, text: &str) {
    Frame::new().fill(theme::accent_soft(0.07)).corner_radius(8).inner_margin(Margin::same(10)).show(ui, |ui| {
        ui.add(egui::Label::new(RichText::new(text).color(theme::TEXT).size(12.0)).wrap());
    });
}

fn help_group(ui: &mut Ui, title: &str, rows: &[(&str, &str)]) {
    ui.label(RichText::new(title).font(theme::semibold(13.0)).color(theme::INK));
    ui.add_space(4.0);
    egui::Grid::new(title).num_columns(2).spacing(vec2(12.0, 5.0)).show(ui, |ui| {
        for (k, v) in rows {
            Frame::new().fill(theme::PANEL).stroke(Stroke::new(1.0, theme::LINE)).corner_radius(4).inner_margin(Margin::symmetric(5, 1)).show(ui, |ui| {
                ui.label(RichText::new(*k).monospace().size(11.0));
            });
            ui.label(RichText::new(*v).color(theme::MUTED));
            ui.end_row();
        }
    });
}

fn menu_item(ui: &mut Ui, text: &str, shortcut: &str, enabled: bool) -> bool {
    let b = Button::new(text).shortcut_text(RichText::new(shortcut).color(theme::FAINT));
    let r = ui.add_enabled(enabled, b);
    if r.clicked() {
        ui.close();
        true
    } else {
        false
    }
}

fn menu_item_danger(ui: &mut Ui, text: &str, shortcut: &str) -> bool {
    let b = Button::new(RichText::new(text).color(theme::DANGER)).shortcut_text(RichText::new(shortcut).color(theme::FAINT));
    if ui.add(b).clicked() {
        ui.close();
        true
    } else {
        false
    }
}

fn separator_h(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(30.0, 7.0), Sense::hover());
    ui.painter().hline(r.x_range().shrink(5.0), r.center().y, Stroke::new(1.0, theme::LINE));
}

fn separator_v(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(7.0, 30.0), Sense::hover());
    ui.painter().vline(r.center().x, r.y_range().shrink(5.0), Stroke::new(1.0, theme::LINE));
}

fn paint_logo(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, CornerRadius::same(7), theme::ACCENT);
    let inner = r.shrink(6.0);
    p.rect_stroke(inner.translate(vec2(0.0, 1.0)), 2.0, Stroke::new(1.6, Color32::WHITE), egui::StrokeKind::Middle);
    let small = Rect::from_min_max(inner.center() + vec2(0.0, 1.0), inner.max - vec2(2.5, 1.5));
    p.rect_stroke(small, 1.0, Stroke::new(1.4, Color32::WHITE), egui::StrokeKind::Middle);
}

/// `Some(is_held)` if `name` is a modifier toggle key, `None` for normal keys.
fn modifier_state(name: &str, m: egui::Modifiers) -> Option<bool> {
    match name {
        "Alt" => Some(m.alt),
        "Ctrl" => Some(m.ctrl),
        "Shift" => Some(m.shift),
        _ => None,
    }
}
