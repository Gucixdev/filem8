//! File operations with undo. Everything is reversible except permanent delete,
//! which is a separate, explicit call that is never put on the undo stack.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub enum Op {
    Rename { from: PathBuf, to: PathBuf },
    Move { pairs: Vec<(PathBuf, PathBuf)> },
    /// Created copies; undo = move them to trash.
    Copy { created: Vec<PathBuf> },
    Trash { originals: Vec<PathBuf> },
    NewDir { path: PathBuf },
    NewFile { path: PathBuf },
}

#[derive(Default)]
pub struct History {
    undo: Vec<Op>,
    redo: Vec<Op>,
}

impl History {
    pub fn push(&mut self, op: Op) {
        self.undo.push(op);
        self.redo.clear();
        if self.undo.len() > 200 { self.undo.remove(0); }
    }
    pub fn can_undo(&self) -> bool { !self.undo.is_empty() }
    pub fn can_redo(&self) -> bool { !self.redo.is_empty() }
    pub fn describe_undo(&self) -> Option<String> { self.undo.last().map(describe) }

    pub fn undo(&mut self) -> Result<String, String> {
        let op = self.undo.pop().ok_or("nothing to undo")?;
        match revert(&op) {
            Ok(redo_op) => { let d = describe(&op); self.redo.push(redo_op.unwrap_or(op)); Ok(format!("Undid: {d}")) }
            Err(e) => { self.undo.push(op); Err(e) }
        }
    }
    pub fn redo(&mut self) -> Result<String, String> {
        let op = self.redo.pop().ok_or("nothing to redo")?;
        let d = describe(&op);
        match apply(&op) {
            Ok(real) => { self.undo.push(real); Ok(format!("Redid: {d}")) }
            Err(e) => { self.redo.push(op); Err(e) }
        }
    }
}

pub fn describe(op: &Op) -> String {
    let n = |p: &Path| p.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
    match op {
        Op::Rename { from, to } => format!("rename {} to {}", n(from), n(to)),
        Op::Move { pairs } => format!("move {} item(s)", pairs.len()),
        Op::Copy { created } => format!("copy {} item(s)", created.len()),
        Op::Trash { originals } => format!("move {} item(s) to trash", originals.len()),
        Op::NewDir { path } => format!("create folder {}", n(path)),
        Op::NewFile { path } => format!("create file {}", n(path)),
    }
}

fn err(e: impl std::fmt::Display, p: &Path) -> String { format!("{}: {e}", p.display()) }

/// `name (copy).ext`, `name (copy 2).ext`... first free sibling name.
pub fn unique_name(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if fs::symlink_metadata(&first).is_err() { return first; }
    let (stem, ext) = match name.rfind('.') { Some(i) if i > 0 => (&name[..i], &name[i..]), _ => (name, "") };
    for n in 1.. {
        let cand = if n == 1 { format!("{stem} (copy){ext}") } else { format!("{stem} (copy {n}){ext}") };
        let p = dir.join(cand);
        if fs::symlink_metadata(&p).is_err() { return p; }
    }
    unreachable!()
}

pub fn rename(from: &Path, new_name: &str) -> Result<Op, String> {
    if new_name.is_empty() || new_name.contains('/') || new_name == "." || new_name == ".." { return Err("invalid name".into()); }
    let to = from.parent().ok_or("no parent")?.join(new_name);
    if to == from { return Err("same name".into()); }
    if fs::symlink_metadata(&to).is_ok() { return Err(format!("'{new_name}' already exists")); }
    fs::rename(from, &to).map_err(|e| err(e, from))?;
    Ok(Op::Rename { from: from.to_path_buf(), to })
}

pub fn new_dir(parent: &Path, name: &str) -> Result<Op, String> {
    if name.is_empty() || name.contains('/') { return Err("invalid name".into()); }
    let p = parent.join(name);
    fs::create_dir(&p).map_err(|e| err(e, &p))?;
    Ok(Op::NewDir { path: p })
}

pub fn new_file(parent: &Path, name: &str) -> Result<Op, String> {
    if name.is_empty() || name.contains('/') { return Err("invalid name".into()); }
    let p = parent.join(name);
    fs::OpenOptions::new().write(true).create_new(true).open(&p).map_err(|e| err(e, &p))?;
    Ok(Op::NewFile { path: p })
}

/// Copy a file or tree. Never overwrites: collisions get a "(copy)" name.
/// `progress` receives bytes copied so far; return false to cancel.
pub fn copy_tree(src: &Path, dst: &Path, progress: &mut dyn FnMut(u64) -> bool, done: &mut u64) -> io::Result<()> {
    let md = fs::symlink_metadata(src)?;
    if md.file_type().is_symlink() {
        std::os::unix::fs::symlink(fs::read_link(src)?, dst)?;
    } else if md.is_dir() {
        if dst.starts_with(src) { return Err(io::Error::other("cannot copy a folder into itself")); }
        fs::create_dir(dst)?;
        for e in fs::read_dir(src)? {
            let e = e?;
            copy_tree(&e.path(), &dst.join(e.file_name()), progress, done)?;
        }
        let _ = fs::set_permissions(dst, md.permissions());
    } else if !md.is_file() {
        // opening a FIFO for reading blocks until a writer shows up: recreate it instead; sockets and devices are not copied
        use std::os::unix::fs::{FileTypeExt, PermissionsExt};
        if !md.file_type().is_fifo() { return Err(io::Error::other("special file (socket or device) cannot be copied")); }
        let c = std::ffi::CString::new(std::os::unix::ffi::OsStrExt::as_bytes(dst.as_os_str()))?;
        if unsafe { libc::mkfifo(c.as_ptr(), (md.permissions().mode() & 0o7777) as libc::mode_t) } != 0 { return Err(io::Error::last_os_error()); }
    } else {
        let mut r = fs::File::open(src)?;
        let mut w = fs::OpenOptions::new().write(true).create_new(true).open(dst)?;
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let n = io::Read::read(&mut r, &mut buf)?;
            if n == 0 { break; }
            io::Write::write_all(&mut w, &buf[..n])?;
            *done += n as u64;
            if !progress(*done) { drop(w); let _ = fs::remove_file(dst); return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled")); }
        }
        let _ = fs::set_permissions(dst, md.permissions());
    }
    Ok(())
}

pub fn copy_to(sources: &[PathBuf], dest_dir: &Path, progress: &mut dyn FnMut(u64) -> bool) -> Result<Op, String> {
    let mut created: Vec<PathBuf> = vec![];
    let mut done = 0;
    for s in sources {
        let name = s.file_name().ok_or("bad source")?.to_string_lossy().into_owned();
        let d = unique_name(dest_dir, &name);
        if let Err(e) = copy_tree(s, &d, progress, &mut done) {
            // roll back what this call made
            if d.exists() { let _ = remove_permanent(&d); }
            for c in &created { let _ = remove_permanent(c); }
            return Err(err(e, s));
        }
        created.push(d);
    }
    Ok(Op::Copy { created })
}

/// Move; same-filesystem renames are atomic, cross-device falls back to copy+delete.
/// Collisions get "(copy)" names rather than overwriting.
pub fn move_to(sources: &[PathBuf], dest_dir: &Path) -> Result<Op, String> {
    move_to_with(sources, dest_dir, &mut |_| true)
}

/// Total bytes under `paths` (symlinks not followed); for progress bars.
pub fn tree_size(paths: &[PathBuf]) -> u64 {
    fn go(p: &Path, n: &mut u64) {
        let Ok(md) = fs::symlink_metadata(p) else { return };
        if md.is_dir() { if let Ok(rd) = fs::read_dir(p) { for e in rd.flatten() { go(&e.path(), n); } } } else { *n += md.len(); }
    }
    let mut n = 0;
    for p in paths { go(p, &mut n); }
    n
}

/// `move_to` with progress (bytes copied across devices) and cancel (return
/// false). Cancelling after some items were moved returns those as an `Ok`
/// partial op so history stays truthful; the caller knows it asked to cancel.
pub fn move_to_with(sources: &[PathBuf], dest_dir: &Path, progress: &mut dyn FnMut(u64) -> bool) -> Result<Op, String> {
    let mut done = 0u64;
    let mut pairs: Vec<(PathBuf, PathBuf)> = vec![];
    for s in sources {
        let name = s.file_name().ok_or("bad source")?.to_string_lossy().into_owned();
        if s.is_dir() && dest_dir.starts_with(s) { return Err("cannot move a folder into itself".into()); }
        if s.parent() == Some(dest_dir) { continue; }
        let d = unique_name(dest_dir, &name);
        match fs::rename(s, &d) {
            Ok(()) => {}
            Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
                if let Err(e) = copy_tree(s, &d, progress, &mut done) {
                    if d.exists() { let _ = remove_permanent(&d); }
                    if e.kind() == io::ErrorKind::Interrupted && !pairs.is_empty() { break; }
                    return Err(err(e, s));
                }
                remove_permanent(s).map_err(|e| err(e, s))?;
            }
            Err(e) => return Err(err(e, s)),
        }
        pairs.push((s.clone(), d));
    }
    if pairs.is_empty() { return Err("nothing to move".into()); }
    Ok(Op::Move { pairs })
}

pub fn trash(paths: &[PathBuf]) -> Result<Op, String> {
    for p in paths { crate::trashbin::trash(p)?; }
    Ok(Op::Trash { originals: paths.to_vec() })
}

pub fn remove_permanent(p: &Path) -> io::Result<()> {
    let md = fs::symlink_metadata(p)?;
    if md.is_dir() { fs::remove_dir_all(p) } else { fs::remove_file(p) }
}

fn restore(originals: &[PathBuf]) -> Result<(), String> {
    for o in originals { crate::trashbin::restore_path(o)?; }
    Ok(())
}

/// Re-apply (used by redo). Returns the op as actually applied.
/// Like the first run it never overwrites: a target that appeared in the meantime is an error.
fn apply(op: &Op) -> Result<Op, String> {
    let free = |p: &Path| if fs::symlink_metadata(p).is_ok() { Err(format!("{} exists", p.display())) } else { Ok(()) };
    match op {
        Op::Rename { from, to } => { free(to)?; fs::rename(from, to).map_err(|e| err(e, from))?; Ok(op.clone()) }
        Op::Move { pairs } => { for (_, b) in pairs { free(b)?; } for (a, b) in pairs { fs::rename(a, b).map_err(|e| err(e, a))?; } Ok(op.clone()) }
        Op::Trash { originals } => trash(originals),
        Op::NewDir { path } => { fs::create_dir(path).map_err(|e| err(e, path))?; Ok(op.clone()) }
        Op::NewFile { path } => { fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|e| err(e, path))?; Ok(op.clone()) }
        Op::Copy { .. } => Err("redo of copy is not supported".into()),
    }
}

/// Undo `op`. Returns a replacement op for the redo stack when it changed.
fn revert(op: &Op) -> Result<Option<Op>, String> {
    match op {
        Op::Rename { from, to } => {
            if fs::symlink_metadata(from).is_ok() { return Err(format!("{} exists", from.display())); }
            fs::rename(to, from).map_err(|e| err(e, to))?;
            Ok(None)
        }
        Op::Move { pairs } => {
            for (a, b) in pairs.iter().rev() {
                if fs::symlink_metadata(a).is_ok() { return Err(format!("{} exists", a.display())); }
                fs::rename(b, a).map_err(|e| err(e, b))?;
            }
            Ok(None)
        }
        Op::Copy { created } => {
            // undo of a copy is reversible through the trash, never a hard delete
            trash(created)?;
            Ok(Some(Op::Copy { created: created.clone() }))
        }
        Op::Trash { originals } => { restore(originals)?; Ok(None) }
        Op::NewDir { path } => { fs::remove_dir(path).map_err(|e| err(e, path))?; Ok(None) }
        Op::NewFile { path } => {
            if fs::metadata(path).map(|m| m.len() > 0).unwrap_or(false) { return Err("file has content; not removing".into()); }
            fs::remove_file(path).map_err(|e| err(e, path))?;
            Ok(None)
        }
    }
}
