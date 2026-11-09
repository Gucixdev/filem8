//! freedesktop.org Trash specification 1.0, std + libc only: `files/`,
//! `info/*.trashinfo`, `directorysizes`, the home trash and per-mount
//! `$topdir/.Trash/$uid` / `.Trash-$uid`, restore.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct TrashItem {
    pub original: PathBuf,
    /// Where the data lives now (`<trash>/files/<name>`).
    pub trashed: PathBuf,
    pub info: PathBuf,
    /// `YYYY-MM-DDThh:mm:ss` local time; sorts chronologically.
    pub deleted: String,
}

fn uid() -> u32 { unsafe { libc::getuid() } }

pub fn home_trash() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| crate::paths::home().join(".local/share")).join("Trash")
}

pub fn encode(p: &str) -> String {
    let mut s = String::new();
    for b in p.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~/".contains(&b) { s.push(b as char) } else { s.push_str(&format!("%{b:02X}")) }
    }
    s
}

pub fn decode(p: &str) -> String {
    let b = p.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 3 <= b.len() && p.is_char_boundary(i + 1) && p.is_char_boundary(i + 3) {
            if let Ok(v) = u8::from_str_radix(&p[i + 1..i + 3], 16) { out.push(v); i += 3; continue; }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn now_iso() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs, &mut tm) };
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
}

/// Mount point containing `p`: walk up while the device id stays the same.
pub fn topdir(p: &Path) -> PathBuf {
    let dev = fs::symlink_metadata(p).map(|m| m.dev()).unwrap_or(0);
    let mut cur = p.to_path_buf();
    while let Some(par) = cur.parent() {
        match fs::symlink_metadata(par) { Ok(m) if m.dev() == dev => cur = par.to_path_buf(), _ => break }
    }
    if cur == p { p.parent().map(|x| x.to_path_buf()).unwrap_or_else(|| PathBuf::from("/")) } else { cur }
}

fn ensure_dir(p: &Path, mode: u32) -> std::io::Result<()> {
    fs::create_dir_all(p)?;
    let _ = fs::set_permissions(p, fs::Permissions::from_mode(mode));
    Ok(())
}

/// Trash directory to use for `p` and the base that `Path=` is relative to
/// (None = absolute, used for the home trash).
fn trash_for(p: &Path) -> Result<(PathBuf, Option<PathBuf>), String> {
    let home = home_trash();
    let probe = home.parent().and_then(|d| if d.exists() { Some(d.to_path_buf()) } else { d.parent().map(|x| x.to_path_buf()) });
    let same_dev = |a: &Path, b: &Path| matches!((fs::metadata(a), fs::metadata(b)), (Ok(x), Ok(y)) if x.dev() == y.dev());
    let pdir = p.parent().unwrap_or(p);
    if probe.as_ref().map(|d| same_dev(d, pdir)).unwrap_or(false) { return Ok((home, None)); }
    let top = topdir(p);
    let shared = top.join(".Trash");
    if let Ok(m) = fs::symlink_metadata(&shared) {
        if m.is_dir() && !m.file_type().is_symlink() && m.permissions().mode() & 0o1000 != 0 {
            let t = shared.join(uid().to_string());
            if ensure_dir(&t, 0o700).is_ok() { return Ok((t, Some(top))); }
        }
    }
    let t = top.join(format!(".Trash-{}", uid()));
    ensure_dir(&t, 0o700).map_err(|e| format!("cannot create {}: {e}", t.display()))?;
    Ok((t, Some(top)))
}

fn dir_size(p: &Path) -> u64 {
    let Ok(m) = fs::symlink_metadata(p) else { return 0 };
    if !m.is_dir() { return m.len(); }
    fs::read_dir(p).map(|r| r.flatten().map(|e| dir_size(&e.path())).sum::<u64>()).unwrap_or(0) + m.len()
}

/// Move `p` into `trash` (a trash directory). `rel_base` makes `Path=` relative.
pub fn trash_in(p: &Path, trash: &Path, rel_base: Option<&Path>) -> Result<TrashItem, String> {
    let abs = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().map_err(|e| e.to_string())?.join(p) };
    fs::symlink_metadata(&abs).map_err(|e| format!("{}: {e}", abs.display()))?;
    let (files, info) = (trash.join("files"), trash.join("info"));
    ensure_dir(trash, 0o700).and_then(|_| ensure_dir(&files, 0o700)).and_then(|_| ensure_dir(&info, 0o700)).map_err(|e| e.to_string())?;
    let base = abs.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or("cannot trash a root")?;
    let stored = match rel_base { Some(b) => abs.strip_prefix(b).unwrap_or(&abs).to_string_lossy().into_owned(), None => abs.to_string_lossy().into_owned() };
    let date = now_iso();
    // reserve a unique name by creating its .trashinfo with O_EXCL
    let mut n = 1;
    let (name, info_path) = loop {
        let name = if n == 1 { base.clone() } else { format!("{base}.{n}") };
        let ip = info.join(format!("{name}.trashinfo"));
        match fs::OpenOptions::new().write(true).create_new(true).open(&ip) {
            Ok(mut f) => {
                f.write_all(format!("[Trash Info]\nPath={}\nDeletionDate={}\n", encode(&stored), date).as_bytes()).map_err(|e| e.to_string())?;
                if files.join(&name).symlink_metadata().is_ok() { let _ = fs::remove_file(&ip); n += 1; continue; }
                break (name, ip);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e.to_string()),
        }
    };
    let dest = files.join(&name);
    let is_dir = fs::symlink_metadata(&abs).map(|m| m.is_dir()).unwrap_or(false);
    let size = if is_dir { dir_size(&abs) } else { 0 };
    if let Err(e) = fs::rename(&abs, &dest) {
        let _ = fs::remove_file(&info_path);
        return Err(format!("{}: {e}", abs.display()));
    }
    if is_dir {
        let mtime = fs::metadata(&info_path).map(|m| m.mtime()).unwrap_or(0);
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(trash.join("directorysizes")) { let _ = writeln!(f, "{size} {mtime} {}", encode(&name)); }
    }
    Ok(TrashItem { original: abs, trashed: dest, info: info_path, deleted: date })
}

pub fn trash(p: &Path) -> Result<TrashItem, String> {
    let (t, base) = trash_for(p)?;
    trash_in(p, &t, base.as_deref())
}

/// Items of one trash directory.
pub fn list_in(trash: &Path, rel_base: Option<&Path>) -> Vec<TrashItem> {
    let mut v = vec![];
    let Ok(rd) = fs::read_dir(trash.join("info")) else { return v };
    for e in rd.flatten() {
        let ip = e.path();
        let Some(stem) = ip.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".trashinfo")) else { continue };
        let Ok(txt) = fs::read_to_string(&ip) else { continue };
        let (mut path, mut date) = (None, String::new());
        for l in txt.lines() {
            if let Some(x) = l.strip_prefix("Path=") { path = Some(decode(x)); }
            if let Some(x) = l.strip_prefix("DeletionDate=") { date = x.to_string(); }
        }
        let Some(path) = path else { continue };
        let original = match rel_base { Some(b) if !path.starts_with('/') => b.join(path), _ => PathBuf::from(path) };
        v.push(TrashItem { original, trashed: trash.join("files").join(stem), info: ip, deleted: date });
    }
    v
}

/// Items from the home trash and every mounted filesystem's trash.
pub fn list() -> Vec<TrashItem> {
    let mut v = list_in(&home_trash(), None);
    for m in crate::places::mounts() {
        for t in [m.path.join(".Trash").join(uid().to_string()), m.path.join(format!(".Trash-{}", uid()))] { v.extend(list_in(&t, Some(&m.path))); }
    }
    v
}

/// Put a trashed item back (fails when something already sits at the original path).
pub fn restore(it: &TrashItem) -> Result<(), String> {
    if it.original.symlink_metadata().is_ok() { return Err(format!("{} already exists", it.original.display())); }
    if let Some(par) = it.original.parent() { fs::create_dir_all(par).map_err(|e| e.to_string())?; }
    fs::rename(&it.trashed, &it.original).map_err(|e| format!("{}: {e}", it.original.display()))?;
    let _ = fs::remove_file(&it.info);
    Ok(())
}

/// Restore the most recently trashed item whose original path is `orig`.
pub fn restore_path(orig: &Path) -> Result<(), String> {
    let mut c: Vec<TrashItem> = list().into_iter().filter(|i| i.original == orig).collect();
    c.sort_by(|a, b| b.deleted.cmp(&a.deleted));
    restore(c.first().ok_or_else(|| format!("{} not found in trash", orig.display()))?)
}

/// Every trash directory to look at, with the base its `Path=` entries are relative to:
/// the home trash plus `$topdir/.Trash/$uid` and `$topdir/.Trash-$uid` of each mount.
pub fn roots() -> Vec<(PathBuf, Option<PathBuf>)> {
    let mut v = vec![(home_trash(), None)];
    for m in crate::places::mounts() {
        for t in [m.path.join(".Trash").join(uid().to_string()), m.path.join(format!(".Trash-{}", uid()))] {
            if t.join("info").is_dir() && !v.iter().any(|(x, _)| *x == t) { v.push((t, Some(m.path.clone()))); }
        }
    }
    v
}

/// Items of the given trash directories, newest deletion first.
pub fn list_roots(roots: &[(PathBuf, Option<PathBuf>)]) -> Vec<TrashItem> {
    let mut v: Vec<TrashItem> = roots.iter().flat_map(|(t, b)| list_in(t, b.as_deref())).collect();
    v.sort_by(|a, b| b.deleted.cmp(&a.deleted).then_with(|| a.original.cmp(&b.original)));
    v
}

/// Forget a trashed item for good: its data, its `.trashinfo` and its `directorysizes` line.
pub fn purge(it: &TrashItem) -> Result<(), String> {
    match fs::symlink_metadata(&it.trashed) {
        Ok(m) if m.is_dir() => fs::remove_dir_all(&it.trashed),
        Ok(_) => fs::remove_file(&it.trashed),
        Err(_) => Ok(()),
    }
    .map_err(|e| format!("{}: {e}", it.trashed.display()))?;
    let _ = fs::remove_file(&it.info);
    if let (Some(files), Some(name)) = (it.trashed.parent(), it.trashed.file_name().and_then(|n| n.to_str())) {
        if let Some(trash) = files.parent() {
            let f = trash.join("directorysizes");
            if let Ok(txt) = fs::read_to_string(&f) {
                let enc = encode(name);
                let keep: Vec<&str> = txt.lines().filter(|l| l.splitn(3, ' ').nth(2) != Some(enc.as_str())).collect();
                if keep.len() != txt.lines().count() { let _ = fs::write(&f, keep.iter().map(|l| format!("{l}\n")).collect::<String>()); }
            }
        }
    }
    Ok(())
}

/// Restore to a different path than the recorded one (used on name conflicts).
pub fn restore_to(it: &TrashItem, dest: &Path) -> Result<(), String> {
    if dest.symlink_metadata().is_ok() { return Err(format!("{} already exists", dest.display())); }
    if let Some(par) = dest.parent() { fs::create_dir_all(par).map_err(|e| e.to_string())?; }
    fs::rename(&it.trashed, dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let _ = purge(it);
    Ok(())
}

/// `dir/name` or `dir/name (restored)`, `dir/name (restored 2)` ... that does not exist yet.
pub fn free_name(p: &Path) -> PathBuf {
    let (dir, name) = (p.parent().unwrap_or(Path::new("/")), p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let (stem, ext) = match name.rfind('.') { Some(i) if i > 0 => (&name[..i], &name[i..]), _ => (name.as_str(), "") };
    let mut n = 1;
    loop {
        let c = dir.join(if n == 1 { format!("{stem} (restored){ext}") } else { format!("{stem} (restored {n}){ext}") });
        if c.symlink_metadata().is_err() { return c; }
        n += 1;
    }
}
