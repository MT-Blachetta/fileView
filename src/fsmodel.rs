//! File-system model: a snapshot of the directory tree below the canvas root.
//!
//! The tree is scanned breadth-first with a node budget so that huge roots
//! (e.g. `/` or `C:\`) stay responsive: shallow levels are always loaded,
//! deeper folders are loaded only while the budget lasts and otherwise shown
//! as collapsed folders that can be expanded on demand.

use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub type NodeId = usize;

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum SortKey {
    Name,
    Type,
    Size,
    Modified,
}

impl SortKey {
    pub const ALL: [SortKey; 4] = [SortKey::Name, SortKey::Type, SortKey::Size, SortKey::Modified];
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => "Name",
            SortKey::Type => "Type",
            SortKey::Size => "Size",
            SortKey::Modified => "Date modified",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub hidden: bool,
    pub readonly: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub depth: u16,
    /// Child folders (sorted).
    pub dirs: Vec<NodeId>,
    /// Child files (sorted).
    pub files: Vec<NodeId>,
    /// Whether the children of this folder were read.
    pub loaded: bool,
    /// Number of visible entries, if known (also known for collapsed folders).
    pub entry_count: Option<usize>,
    /// Entries that exist but were not loaded because of per-folder limits.
    pub omitted: usize,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ScanOptions {
    pub show_hidden: bool,
    /// Folders up to this depth (root's children = 1) are expanded by default.
    pub auto_depth: u16,
    /// Explicit expand (true) / collapse (false) choices of the user.
    pub expand: HashMap<PathBuf, bool>,
    pub sort: SortKey,
    pub sort_desc: bool,
    /// Maximum number of entries in the whole tree.
    pub budget: usize,
    /// Maximum number of files shown per folder.
    pub max_files_per_dir: usize,
}

impl ScanOptions {
    pub fn wants_expanded(&self, path: &Path, depth: u16) -> bool {
        match self.expand.get(path) {
            Some(v) => *v,
            None => depth <= self.auto_depth,
        }
    }
}

pub struct Tree {
    pub nodes: Vec<Node>,
    pub root: NodeId,
    /// Directory modification times of all loaded folders (for change polling).
    pub dir_mtimes: Vec<(PathBuf, Option<SystemTime>)>,
    pub budget_exhausted: bool,
    pub total_entries: usize,
}

impl Tree {
    pub fn empty(root: &Path) -> Self {
        let node = Node {
            path: root.to_path_buf(),
            name: display_name(root),
            is_dir: true,
            is_symlink: false,
            hidden: false,
            readonly: false,
            size: 0,
            modified: None,
            depth: 0,
            dirs: vec![],
            files: vec![],
            loaded: false,
            entry_count: None,
            omitted: 0,
            error: None,
        };
        Tree { nodes: vec![node], root: 0, dir_mtimes: vec![], budget_exhausted: false, total_entries: 0 }
    }

    pub fn root_node(&self) -> &Node {
        &self.nodes[self.root]
    }

    pub fn scan(root: &Path, opts: &ScanOptions) -> Tree {
        let mut tree = Tree::empty(root);
        if let Ok(md) = fs::metadata(root) {
            tree.nodes[0].modified = md.modified().ok();
            tree.nodes[0].readonly = md.permissions().readonly();
        }
        let mut queue = VecDeque::new();
        queue.push_back(tree.root);
        let mut used = 0usize;
        while let Some(id) = queue.pop_front() {
            let depth = tree.nodes[id].depth;
            let path = tree.nodes[id].path.clone();
            let is_root = id == tree.root;
            let expand = is_root || opts.wants_expanded(&path, depth);
            // Explicitly expanded folders get loaded even beyond the budget
            // (with a smaller per-folder cap), auto-expanded ones only within it.
            let explicit = opts.expand.get(&path) == Some(&true);
            if !expand || (!is_root && !explicit && used >= opts.budget) {
                tree.budget_exhausted |= expand;
                tree.nodes[id].entry_count = count_entries(&path, opts.show_hidden);
                continue;
            }
            tree.dir_mtimes.push((path.clone(), fs::metadata(&path).and_then(|m| m.modified()).ok()));
            match read_children(&path, opts) {
                Ok(mut entries) => {
                    sort_entries(&mut entries, opts.sort, opts.sort_desc);
                    let total = entries.len();
                    let mut dirs = vec![];
                    let mut files = vec![];
                    let mut shown_files = 0usize;
                    let mut omitted = 0usize;
                    for mut e in entries {
                        if !e.is_dir {
                            if shown_files >= opts.max_files_per_dir {
                                omitted += 1;
                                continue;
                            }
                            shown_files += 1;
                        }
                        e.depth = depth + 1;
                        let nid = tree.nodes.len();
                        let is_dir = e.is_dir;
                        tree.nodes.push(e);
                        if is_dir {
                            dirs.push(nid);
                            queue.push_back(nid);
                        } else {
                            files.push(nid);
                        }
                    }
                    used += total;
                    let n = &mut tree.nodes[id];
                    n.dirs = dirs;
                    n.files = files;
                    n.loaded = true;
                    n.entry_count = Some(total);
                    n.omitted = omitted;
                }
                Err(e) => {
                    tree.nodes[id].error = Some(e.to_string());
                    tree.nodes[id].loaded = true;
                    tree.nodes[id].entry_count = Some(0);
                }
            }
        }
        tree.total_entries = used;
        tree
    }
}

fn read_children(dir: &Path, opts: &ScanOptions) -> std::io::Result<Vec<Node>> {
    let mut out = vec![];
    for entry in fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let ftype = entry.file_type().ok();
        let is_symlink = ftype.map(|t| t.is_symlink()).unwrap_or(false);
        // Follow symlinks for type/size, but fall back to the link itself.
        let md = fs::metadata(&path).or_else(|_| entry.metadata()).ok();
        let is_dir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let hidden = is_hidden(&name, md.as_ref());
        if hidden && !opts.show_hidden {
            continue;
        }
        out.push(Node {
            path,
            name,
            is_dir,
            is_symlink,
            hidden,
            readonly: md.as_ref().map(|m| m.permissions().readonly()).unwrap_or(false),
            size: if is_dir { 0 } else { md.as_ref().map(|m| m.len()).unwrap_or(0) },
            modified: md.as_ref().and_then(|m| m.modified().ok()),
            depth: 0,
            dirs: vec![],
            files: vec![],
            loaded: false,
            entry_count: None,
            omitted: 0,
            error: None,
        });
    }
    Ok(out)
}

fn count_entries(dir: &Path, show_hidden: bool) -> Option<usize> {
    let rd = fs::read_dir(dir).ok()?;
    let mut n = 0;
    for e in rd.flatten().take(100_000) {
        if !show_hidden && e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        n += 1;
    }
    Some(n)
}

#[allow(unused_variables)]
pub fn is_hidden(name: &str, md: Option<&fs::Metadata>) -> bool {
    if name.starts_with('.') {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
        if let Some(md) = md {
            return md.file_attributes() & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0;
        }
    }
    false
}

fn sort_entries(entries: &mut [Node], key: SortKey, desc: bool) {
    entries.sort_by(|a, b| {
        let ord = match key {
            SortKey::Name => natural_cmp(&a.name, &b.name),
            SortKey::Type => extension(&a.name)
                .cmp(&extension(&b.name))
                .then_with(|| natural_cmp(&a.name, &b.name)),
            SortKey::Size => a.size.cmp(&b.size).then_with(|| natural_cmp(&a.name, &b.name)),
            SortKey::Modified => a.modified.cmp(&b.modified).then_with(|| natural_cmp(&a.name, &b.name)),
        };
        if desc { ord.reverse() } else { ord }
    });
}

/// Case-insensitive comparison that orders embedded numbers numerically
/// ("file2" < "file10"), like Windows Explorer.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) if ca.is_ascii_digit() && cb.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ai.peek().copied().filter(|c| c.is_ascii_digit()) {
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(c) = bi.peek().copied().filter(|c| c.is_ascii_digit()) {
                    nb.push(c);
                    bi.next();
                }
                let ta = na.trim_start_matches('0');
                let tb = nb.trim_start_matches('0');
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(ca), Some(cb)) => {
                let la = ca.to_lowercase().next().unwrap_or(ca);
                let lb = cb.to_lowercase().next().unwrap_or(cb);
                if la != lb {
                    return la.cmp(&lb);
                }
                ai.next();
                bi.next();
            }
        }
    }
}

pub fn extension(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => name[i + 1..].to_lowercase(),
        _ => String::new(),
    }
}

pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if v >= 100.0 { format!("{v:.0} {}", UNITS[u]) } else { format!("{v:.1} {}", UNITS[u]) }
}

pub fn format_time(t: Option<SystemTime>) -> String {
    let Some(t) = t else { return "—".into() };
    let Ok(d) = t.duration_since(SystemTime::UNIX_EPOCH) else { return "—".into() };
    let secs = d.as_secs() as i64 + local_utc_offset_secs();
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, dd) = civil_from_days(days);
    format!("{y:04}-{m:02}-{dd:02}  {:02}:{:02}", rem / 3600, (rem % 3600) / 60)
}

/// Local UTC offset in seconds. `PLANAR_UTC_OFFSET` (hours) overrides the
/// value queried from the operating system.
fn local_utc_offset_secs() -> i64 {
    std::env::var("PLANAR_UTC_OFFSET")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .map(|h| (h * 3600.0) as i64)
        .unwrap_or_else(system_offset_guess)
}

#[cfg(unix)]
fn system_offset_guess() -> i64 {
    // `date +%z` is available on every Linux / macOS system.
    use std::sync::OnceLock;
    static OFF: OnceLock<i64> = OnceLock::new();
    *OFF.get_or_init(|| {
        let out = std::process::Command::new("date").arg("+%z").output().ok();
        out.and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.len() == 5 {
                let sign = if s.starts_with('-') { -1 } else { 1 };
                let h: i64 = s[1..3].parse().ok()?;
                let m: i64 = s[3..5].parse().ok()?;
                Some(sign * (h * 3600 + m * 60))
            } else {
                None
            }
        })
        .unwrap_or(0)
    })
}

#[cfg(windows)]
fn system_offset_guess() -> i64 {
    #[repr(C)]
    struct SystemTimeW {
        y: u16,
        mo: u16,
        dow: u16,
        d: u16,
        h: u16,
        mi: u16,
        s: u16,
        ms: u16,
    }
    #[repr(C)]
    struct TimeZoneInformation {
        bias: i32,
        standard_name: [u16; 32],
        standard_date: SystemTimeW,
        standard_bias: i32,
        daylight_name: [u16; 32],
        daylight_date: SystemTimeW,
        daylight_bias: i32,
    }
    unsafe extern "system" {
        fn GetTimeZoneInformation(tzi: *mut TimeZoneInformation) -> u32;
    }
    unsafe {
        let mut tzi: TimeZoneInformation = std::mem::zeroed();
        let r = GetTimeZoneInformation(&mut tzi);
        let mut bias = tzi.bias;
        if r == 2 {
            bias += tzi.daylight_bias;
        } else if r == 1 {
            bias += tzi.standard_bias;
        }
        -(bias as i64) * 60
    }
}

#[cfg(not(any(unix, windows)))]
fn system_offset_guess() -> i64 {
    0
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Recursive size of a folder (used by the properties panel, in a thread).
pub fn dir_stats(path: &Path, cancel: &std::sync::atomic::AtomicBool) -> (u64, usize, usize) {
    let mut size = 0u64;
    let mut files = 0usize;
    let mut dirs = 0usize;
    let mut stack = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        let Ok(rd) = fs::read_dir(&p) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                dirs += 1;
                stack.push(e.path());
            } else {
                files += 1;
                size += e.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    (size, files, dirs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["file10", "File2", "file1", "a", "B"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["a", "B", "file1", "File2", "file10"]);
    }

    #[test]
    fn sizes() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(extension("archive.tar.GZ"), "gz");
        assert_eq!(extension(".bashrc"), "");
    }

    #[test]
    fn civil() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
    }
}
