//! File operations (move, copy, rename, create, delete) and a tiny background
//! job runner so that long copies never block the UI thread.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    Move,
    Copy,
    Trash,
    Delete,
}

impl JobKind {
    pub fn verb(self) -> &'static str {
        match self {
            JobKind::Move => "Moving",
            JobKind::Copy => "Copying",
            JobKind::Trash => "Moving to trash",
            JobKind::Delete => "Deleting",
        }
    }
}

/// A batch of operations. For move/copy every pair is `(source, full destination path)`,
/// for trash/delete only the source is used.
pub struct Job {
    pub kind: JobKind,
    pub pairs: Vec<(PathBuf, PathBuf)>,
    /// True if this job reverts an earlier one (it is then not undoable itself).
    pub is_undo: bool,
}

pub struct JobResult {
    pub kind: JobKind,
    pub done: Vec<(PathBuf, PathBuf)>,
    pub errors: Vec<String>,
    pub is_undo: bool,
}

pub fn spawn(job: Job, repaint: egui::Context) -> Receiver<JobResult> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut done = vec![];
        let mut errors = vec![];
        match job.kind {
            JobKind::Trash => {
                for (src, _) in &job.pairs {
                    match trash::delete(src) {
                        Ok(()) => done.push((src.clone(), src.clone())),
                        Err(e) => errors.push(format!("{}: {e}", src.display())),
                    }
                }
            }
            _ => {
                for (src, dst) in &job.pairs {
                    let r = match job.kind {
                        JobKind::Move => move_path(src, dst),
                        JobKind::Copy => copy_recursive(src, dst),
                        JobKind::Delete => remove_path(src),
                        JobKind::Trash => unreachable!(),
                    };
                    match r {
                        Ok(()) => done.push((src.clone(), dst.clone())),
                        Err(e) => errors.push(format!("{}: {e}", crate::fsmodel::display_name(src))),
                    }
                }
            }
        }
        let _ = tx.send(JobResult { kind: job.kind, done, errors, is_undo: job.is_undo });
        repaint.request_repaint();
    });
    rx
}

/// Checks whether `src` may be moved/copied into folder `dest_dir`.
pub fn check_transfer(src: &Path, dest_dir: &Path) -> Result<(), String> {
    if dest_dir.starts_with(src) {
        return Err(format!("Cannot put \u{201c}{}\u{201d} inside itself", crate::fsmodel::display_name(src)));
    }
    Ok(())
}

pub fn move_path(src: &Path, dst: &Path) -> io::Result<()> {
    if dst.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "an item with this name already exists"));
    }
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices || is_cross_device(&e) => {
            copy_recursive(src, dst)?;
            remove_path(src)
        }
        Err(e) => Err(e),
    }
}

fn is_cross_device(e: &io::Error) -> bool {
    // EXDEV on Unix, ERROR_NOT_SAME_DEVICE on Windows.
    let code = if cfg!(windows) { 17 } else { 18 };
    e.raw_os_error() == Some(code)
}

pub fn copy_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    let md = fs::symlink_metadata(src)?;
    if md.file_type().is_symlink() {
        #[cfg(unix)]
        {
            let target = fs::read_link(src)?;
            return std::os::unix::fs::symlink(target, dst);
        }
    }
    if fs::metadata(src)?.is_dir() {
        fs::create_dir(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(src, dst).map(|_| ())
    }
}

pub fn remove_path(p: &Path) -> io::Result<()> {
    let md = fs::symlink_metadata(p)?;
    if md.is_dir() { fs::remove_dir_all(p) } else { fs::remove_file(p) }
}

/// Destination path for a copy: keeps the name if free, otherwise
/// "name - Copy.ext", "name - Copy (2).ext", ... (like Windows Explorer).
pub fn unique_copy_target(dest_dir: &Path, name: &str) -> PathBuf {
    let direct = dest_dir.join(name);
    if !direct.exists() {
        return direct;
    }
    let (stem, ext) = split_name(name);
    for i in 1.. {
        let candidate = if i == 1 { format!("{stem} - Copy{ext}") } else { format!("{stem} - Copy ({i}){ext}") };
        let p = dest_dir.join(candidate);
        if !p.exists() {
            return p;
        }
    }
    unreachable!()
}

/// A free name like "New folder", "New folder (2)", ...
pub fn unique_new_name(dir: &Path, base: &str) -> PathBuf {
    let (stem, ext) = split_name(base);
    for i in 1.. {
        let candidate = if i == 1 { base.to_string() } else { format!("{stem} ({i}){ext}") };
        let p = dir.join(candidate);
        if !p.exists() {
            return p;
        }
    }
    unreachable!()
}

/// Splits "report.final.pdf" into ("report.final", ".pdf"); dotfiles keep their name.
pub fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

pub fn validate_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("The name must not be empty".into());
    }
    if trimmed == "." || trimmed == ".." {
        return Err("This name is reserved".into());
    }
    let forbidden: &[char] = if cfg!(windows) { &['/', '\\', ':', '*', '?', '"', '<', '>', '|'] } else { &['/', '\0'] };
    if let Some(c) = trimmed.chars().find(|c| forbidden.contains(c)) {
        return Err(format!("A name cannot contain \u{201c}{c}\u{201d}"));
    }
    Ok(())
}

pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    validate_name(new_name)?;
    let parent = path.parent().ok_or("Cannot rename the file-system root")?;
    let target = parent.join(new_name.trim());
    if target == path {
        return Ok(target);
    }
    // Allow case-only renames on case-insensitive file systems.
    let case_only = target.to_string_lossy().to_lowercase() == path.to_string_lossy().to_lowercase();
    if target.exists() && !case_only {
        return Err(format!("\u{201c}{}\u{201d} already exists", new_name.trim()));
    }
    fs::rename(path, &target).map_err(|e| e.to_string())?;
    Ok(target)
}

pub fn create_dir(parent: &Path, base: &str) -> Result<PathBuf, String> {
    let p = unique_new_name(parent, base);
    fs::create_dir(&p).map_err(|e| e.to_string())?;
    Ok(p)
}

pub fn create_file(parent: &Path, base: &str) -> Result<PathBuf, String> {
    let p = unique_new_name(parent, base);
    fs::OpenOptions::new().write(true).create_new(true).open(&p).map_err(|e| e.to_string())?;
    Ok(p)
}

/// Open a file or folder with the operating system's default application.
pub fn open_default(path: &Path) -> Result<(), String> {
    open::that_detached(path).map_err(|e| e.to_string())
}

/// Show the item in the system file manager.
pub fn reveal(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let arg = format!("/select,{}", path.display());
        return std::process::Command::new("explorer").arg(arg).spawn().map(|_| ()).map_err(|e| e.to_string());
    }
    #[allow(unreachable_code)]
    {
        let dir = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
        open_default(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("planar-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn copy_names() {
        let d = tmp("names");
        fs::write(d.join("a.txt"), "x").unwrap();
        assert_eq!(unique_copy_target(&d, "a.txt"), d.join("a - Copy.txt"));
        fs::write(d.join("a - Copy.txt"), "x").unwrap();
        assert_eq!(unique_copy_target(&d, "a.txt"), d.join("a - Copy (2).txt"));
        assert_eq!(unique_new_name(&d, "b.txt"), d.join("b.txt"));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn move_and_copy_tree() {
        let d = tmp("tree");
        fs::create_dir_all(d.join("src/inner")).unwrap();
        fs::write(d.join("src/inner/f.txt"), "hello").unwrap();
        fs::create_dir(d.join("dst")).unwrap();
        copy_recursive(&d.join("src"), &d.join("dst/src")).unwrap();
        assert_eq!(fs::read_to_string(d.join("dst/src/inner/f.txt")).unwrap(), "hello");
        move_path(&d.join("src"), &d.join("dst/moved")).unwrap();
        assert!(!d.join("src").exists());
        assert!(d.join("dst/moved/inner/f.txt").exists());
        assert!(move_path(&d.join("dst/moved"), &d.join("dst/src")).is_err());
        assert!(check_transfer(&d.join("dst"), &d.join("dst/moved")).is_err());
        let r = rename(&d.join("dst/moved"), "renamed").unwrap();
        assert_eq!(r, d.join("dst/renamed"));
        assert!(validate_name("a/b").is_err());
        fs::remove_dir_all(&d).unwrap();
    }
}
