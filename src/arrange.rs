//! Custom arrangement of the items on the canvas.
//!
//! By default the items of a folder follow the sort order (see `fsmodel`).
//! The user can rearrange them by dragging: files change their position in
//! the file grid, folders their position inside a row or the row they sit in.
//! The result is stored in a hidden file in the canvas root folder,
//! `.planar-layout.json`, whose `tree` mirrors the directory tree:
//!
//! ```json
//! {
//!   "format": "planar-layout",
//!   "version": 1,
//!   "tree": {
//!     "files": ["b.txt", "a.txt"],
//!     "rows": [["src", "docs"], ["assets"]],
//!     "children": {
//!       "src": { "files": ["main.rs", "app.rs"] }
//!     }
//!   }
//! }
//! ```
//!
//! * `files` – file names in display order (left to right, row by row).
//! * `rows` – folder rectangles by row, top to bottom, each row left to right.
//! * `children` – the same structure for sub-folders, keyed by folder name.
//!
//! Only folders the user rearranged get an entry. Items that have none (never
//! moved, or created later) follow the sort order after the arranged ones.
//!
//! The file is read before the first layout of a root. If the root has no file
//! of its own, the entry of the nearest ancestor that has one is used. Layout
//! files of sub-folders (written when such a folder was the root) fill in
//! folders the root file knows nothing about. Saving updates all of these
//! files, so every root shows the same arrangement.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = ".planar-layout.json";
const FORMAT: &str = "planar-layout";
const VERSION: u32 = 1;

/// Arrangement of one folder and (via `children`) of its sub-folders.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DirOrder {
    /// File names in display order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// Folder names by row (top to bottom), each row left to right.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<Vec<String>>,
    /// Arrangements of sub-folders, by folder name.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub children: BTreeMap<String, DirOrder>,
}

impl DirOrder {
    /// Neither this folder nor any sub-folder has a custom order.
    pub fn is_empty(&self) -> bool {
        !self.has_own() && self.children.values().all(DirOrder::is_empty)
    }

    /// This folder itself has a custom order.
    pub fn has_own(&self) -> bool {
        !self.files.is_empty() || !self.rows.is_empty()
    }

    fn prune(&mut self) {
        self.rows.retain(|r| !r.is_empty());
        self.children.retain(|_, c| {
            c.prune();
            !c.is_empty()
        });
    }

    fn len(&self) -> usize {
        self.files.len() + self.rows.iter().map(Vec::len).sum::<usize>()
    }

    /// Removes `name` from the file list and the rows.
    fn forget_name(&mut self, name: &str) -> bool {
        let n = self.len();
        self.files.retain(|f| f != name);
        for r in &mut self.rows {
            r.retain(|d| d != name);
        }
        self.rows.retain(|r| !r.is_empty());
        n != self.len()
    }

    /// Renames `from` in place, keeping its position.
    fn rename_name(&mut self, from: &str, to: &str) -> bool {
        let mut changed = false;
        for n in self.files.iter_mut().chain(self.rows.iter_mut().flatten()) {
            if n == from {
                *n = to.to_string();
                changed = true;
            }
        }
        changed
    }

    fn at(&self, rel: &Path) -> Option<&DirOrder> {
        let mut cur = self;
        for c in rel.components() {
            cur = cur.children.get(c.as_os_str().to_string_lossy().as_ref())?;
        }
        Some(cur)
    }

    fn at_mut(&mut self, rel: &Path) -> Option<&mut DirOrder> {
        let mut cur = self;
        for c in rel.components() {
            cur = cur.children.get_mut(c.as_os_str().to_string_lossy().as_ref())?;
        }
        Some(cur)
    }

    fn entry(&mut self, rel: &Path) -> &mut DirOrder {
        let mut cur = self;
        for c in rel.components() {
            cur = cur.children.entry(c.as_os_str().to_string_lossy().into_owned()).or_default();
        }
        cur
    }
}

/// On-disk format of the layout file.
#[derive(Serialize, Deserialize)]
struct LayoutFile {
    format: String,
    version: u32,
    #[serde(default)]
    tree: DirOrder,
}

/// The arrangement of the current canvas root.
#[derive(Debug)]
pub struct Arrangement {
    root: PathBuf,
    pub tree: DirOrder,
    /// Folders below the root that have a layout file of their own.
    nested: BTreeSet<PathBuf>,
    /// Nearest ancestor of the root that has a layout file.
    ancestor: Option<PathBuf>,
    /// Nested folders whose layout file was already read.
    seen: HashSet<PathBuf>,
}

impl Arrangement {
    pub fn empty(root: &Path) -> Self {
        Arrangement {
            root: root.to_path_buf(),
            tree: DirOrder::default(),
            nested: BTreeSet::new(),
            ancestor: None,
            seen: HashSet::new(),
        }
    }

    /// Reads the arrangement of `root` (its own layout file, else the entry
    /// in the nearest ancestor's file).
    pub fn load(root: &Path) -> Self {
        let mut a = Arrangement::empty(root);
        let own = read_file(root);
        a.ancestor = root.ancestors().skip(1).find(|d| d.join(FILE_NAME).is_file()).map(Path::to_path_buf);
        match (own, &a.ancestor) {
            (Some(t), _) => a.tree = t,
            (None, Some(anc)) => {
                if let (Some(t), Ok(rel)) = (read_file(anc), root.strip_prefix(anc))
                    && let Some(sub) = t.at(rel)
                {
                    a.tree = sub.clone();
                }
            }
            _ => {}
        }
        a
    }

    /// Takes note of layout files found in sub-folders while scanning. Their
    /// content is used for folders the root's file has no entry for.
    pub fn merge_nested(&mut self, dirs: &[PathBuf]) {
        for d in dirs {
            if d == &self.root || !d.starts_with(&self.root) {
                continue;
            }
            self.nested.insert(d.clone());
            if !self.seen.insert(d.clone()) {
                continue;
            }
            let rel = d.strip_prefix(&self.root).unwrap();
            if self.tree.at(rel).is_some_and(|o| !o.is_empty()) {
                continue; // the root's own file wins
            }
            if let Some(t) = read_file(d)
                && !t.is_empty()
            {
                *self.tree.entry(rel) = t;
            }
        }
    }

    /// Custom order of folder `dir`, if any.
    pub fn get(&self, dir: &Path) -> Option<&DirOrder> {
        self.tree.at(dir.strip_prefix(&self.root).ok()?)
    }

    fn node_mut(&mut self, dir: &Path) -> Option<&mut DirOrder> {
        self.tree.at_mut(dir.strip_prefix(&self.root).ok()?)
    }

    fn entry(&mut self, dir: &Path) -> Option<&mut DirOrder> {
        Some(self.tree.entry(dir.strip_prefix(&self.root).ok()?))
    }

    /// Replaces the order of folder `dir`. `files` / `rows` are the names
    /// currently shown in their new order; stored names that are not shown
    /// (e.g. hidden items) but still exist are kept at the end.
    pub fn set_order(&mut self, dir: &Path, files: Option<Vec<String>>, rows: Option<Vec<Vec<String>>>) {
        let Some(node) = self.entry(dir) else { return };
        if let Some(mut files) = files {
            let shown: HashSet<String> = files.iter().cloned().collect();
            files.extend(node.files.iter().filter(|n| !shown.contains(*n) && dir.join(n).exists()).cloned());
            node.files = files;
        }
        if let Some(mut rows) = rows {
            let shown: HashSet<String> = rows.iter().flatten().cloned().collect();
            let rest: Vec<String> =
                node.rows.iter().flatten().filter(|n| !shown.contains(*n) && dir.join(n).exists()).cloned().collect();
            if !rest.is_empty() {
                rows.push(rest);
            }
            rows.retain(|r| !r.is_empty());
            node.rows = rows;
        }
    }

    /// Drops the custom order of `dir` itself (sub-folders keep theirs).
    pub fn reset(&mut self, dir: &Path) -> bool {
        match self.node_mut(dir) {
            Some(n) if n.has_own() => {
                n.files.clear();
                n.rows.clear();
                true
            }
            _ => false,
        }
    }

    /// Updates the arrangement after `from` was renamed or moved to `to`:
    /// a renamed item keeps its position, a moved folder takes the
    /// arrangement of its contents along.
    pub fn rekey(&mut self, from: &Path, to: &Path) -> bool {
        let (Some(fp), Some(fname)) = (from.parent(), from.file_name()) else { return false };
        let fname = fname.to_string_lossy().into_owned();
        let to_name = to.file_name().map(|n| n.to_string_lossy().into_owned());
        let mut changed = false;
        let sub = self.node_mut(fp).and_then(|n| n.children.remove(&fname));
        if let Some(n) = self.node_mut(fp) {
            changed |= match &to_name {
                Some(tn) if to.parent() == Some(fp) => n.rename_name(&fname, tn),
                _ => n.forget_name(&fname),
            };
        }
        if let Some(sub) = sub {
            changed = true;
            if let (Some(tp), Some(tn)) = (to.parent(), to_name)
                && let Some(n) = self.entry(tp)
            {
                n.children.insert(tn, sub);
            }
        }
        changed
    }

    /// Forgets a deleted item.
    pub fn forget(&mut self, path: &Path) -> bool {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else { return false };
        let name = name.to_string_lossy();
        let Some(n) = self.node_mut(parent) else { return false };
        let had_sub = n.children.remove(name.as_ref()).is_some();
        n.forget_name(&name) | had_sub
    }

    /// Writes the layout file of the root (removing it if nothing is
    /// arranged any more) and keeps the other known layout files in sync.
    /// Returns the folders whose layout file was written.
    pub fn save(&mut self) -> Result<Vec<PathBuf>, String> {
        self.tree.prune();
        let mut written = vec![];
        if write_or_remove(&self.root, &self.tree).map_err(|e| e.to_string())? {
            written.push(self.root.clone());
        }
        for d in &self.nested {
            if !d.join(FILE_NAME).is_file() {
                continue;
            }
            let rel = d.strip_prefix(&self.root).unwrap();
            let sub = self.tree.at(rel).cloned().unwrap_or_default();
            if let Ok(true) = write_or_remove(d, &sub) {
                written.push(d.clone());
            }
        }
        if let Some(anc) = &self.ancestor
            && let Ok(rel) = self.root.strip_prefix(anc)
            && let Some(mut t) = read_file(anc)
        {
            *t.entry(rel) = self.tree.clone();
            t.prune();
            if let Ok(true) = write_or_remove(anc, &t) {
                written.push(anc.clone());
            }
        }
        Ok(written)
    }
}

fn read_file(dir: &Path) -> Option<DirOrder> {
    let text = fs::read_to_string(dir.join(FILE_NAME)).ok()?;
    let f: LayoutFile = serde_json::from_str(&text).ok()?;
    (f.format == FORMAT).then_some(f.tree)
}

/// Writes `tree` to the layout file in `dir`, or removes the file when the
/// tree is empty. Returns whether anything on disk changed.
fn write_or_remove(dir: &Path, tree: &DirOrder) -> std::io::Result<bool> {
    let path = dir.join(FILE_NAME);
    if tree.is_empty() {
        return match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        };
    }
    let file = LayoutFile { format: FORMAT.into(), version: VERSION, tree: tree.clone() };
    let mut text = serde_json::to_string_pretty(&file).map_err(std::io::Error::other)?;
    text.push('\n');
    if fs::read_to_string(&path).is_ok_and(|old| old == text) {
        return Ok(false);
    }
    // Opened without truncation: on Windows a hidden file cannot be
    // re-created ("CREATE_ALWAYS"), but it can be opened and shortened.
    let mut f = fs::OpenOptions::new().write(true).create(true).truncate(false).open(&path)?;
    f.set_len(0)?;
    f.write_all(text.as_bytes())?;
    drop(f);
    hide(&path);
    Ok(true)
}

/// The leading dot hides the file on Unix; Windows needs the attribute.
#[cfg(windows)]
fn hide(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const NORMAL: u32 = 0x80;
    unsafe extern "system" {
        fn SetFileAttributesW(name: *const u16, attrs: u32) -> i32;
    }
    let Ok(md) = fs::metadata(path) else { return };
    let attrs = md.file_attributes();
    if attrs & HIDDEN != 0 {
        return;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        SetFileAttributesW(wide.as_ptr(), (attrs & !NORMAL) | HIDDEN);
    }
}

#[cfg(not(windows))]
fn hide(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("planar-arrange-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn save_load_roundtrip_and_sync() {
        let d = tmp("roundtrip");
        fs::create_dir_all(d.join("sub/deep")).unwrap();
        for f in ["a.txt", "b.txt", "sub/x.txt", "sub/y.txt"] {
            fs::write(d.join(f), "").unwrap();
        }
        let mut a = Arrangement::load(&d);
        assert!(a.tree.is_empty());
        a.set_order(&d, Some(vec!["b.txt".into(), "a.txt".into()]), Some(vec![vec!["sub".into()]]));
        a.set_order(&d.join("sub"), Some(vec!["y.txt".into(), "x.txt".into()]), None);
        assert_eq!(a.save().unwrap(), vec![d.clone()]);
        assert!(d.join(FILE_NAME).is_file());

        // Reopening the root reads the same tree.
        let b = Arrangement::load(&d);
        assert_eq!(b.tree, a.tree);
        assert_eq!(b.get(&d).unwrap().files, vec!["b.txt", "a.txt"]);
        // Opening the sub-folder as root uses the ancestor's entry ...
        let mut s = Arrangement::load(&d.join("sub"));
        assert_eq!(s.get(&d.join("sub")).unwrap().files, vec!["y.txt", "x.txt"]);
        // ... and changes made there show up in the ancestor's file.
        s.set_order(&d.join("sub"), Some(vec!["x.txt".into(), "y.txt".into()]), None);
        s.save().unwrap();
        assert!(d.join("sub").join(FILE_NAME).is_file());
        let b = Arrangement::load(&d);
        assert_eq!(b.get(&d.join("sub")).unwrap().files, vec!["x.txt", "y.txt"]);

        // Writing a hidden file twice works (Windows refuses CREATE_ALWAYS on hidden files).
        let mut b = b;
        b.set_order(&d, Some(vec!["a.txt".into(), "b.txt".into()]), None);
        b.save().unwrap();
        assert_eq!(Arrangement::load(&d).get(&d).unwrap().files, vec!["a.txt", "b.txt"]);

        // Resetting everything removes the file.
        b.tree = DirOrder::default();
        b.save().unwrap();
        assert!(!d.join(FILE_NAME).exists());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn rename_move_and_forget() {
        let root = PathBuf::from("/r");
        let mut a = Arrangement::empty(&root);
        a.set_order(&root, Some(vec!["b".into(), "a".into()]), Some(vec![vec!["d2".into(), "d1".into()]]));
        a.set_order(&root.join("d1"), Some(vec!["z".into(), "y".into()]), None);
        // Rename keeps the position and the sub-arrangement.
        assert!(a.rekey(&root.join("d1"), &root.join("dir1")));
        assert_eq!(a.get(&root).unwrap().rows, vec![vec!["d2".to_string(), "dir1".to_string()]]);
        assert_eq!(a.get(&root.join("dir1")).unwrap().files, vec!["z", "y"]);
        // Moving into another folder takes the sub-arrangement along.
        assert!(a.rekey(&root.join("dir1"), &root.join("d2/dir1")));
        assert_eq!(a.get(&root).unwrap().rows, vec![vec!["d2".to_string()]]);
        assert_eq!(a.get(&root.join("d2/dir1")).unwrap().files, vec!["z", "y"]);
        assert!(a.forget(&root.join("b")));
        assert_eq!(a.get(&root).unwrap().files, vec!["a"]);
        assert!(a.reset(&root));
        assert!(!a.get(&root).unwrap().has_own());
    }
}
