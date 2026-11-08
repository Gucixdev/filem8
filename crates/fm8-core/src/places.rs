//! Sidebar sources: GTK bookmarks, XDG user dirs, mounts, recents.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Bookmark {
    pub path: PathBuf,
    pub label: String,
}

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| crate::paths::home().join(".config"))
}
fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| crate::paths::home().join(".local/share"))
}

pub fn gtk_bookmarks_file() -> PathBuf {
    config_home().join("gtk-3.0/bookmarks")
}

/// Parse GTK bookmarks text: `file:///path/with%20space Optional Label`.
pub fn parse_bookmarks(text: &str) -> Vec<Bookmark> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() { return None; }
            let (url, label) = match l.split_once(' ') { Some((u, lb)) => (u, Some(lb.trim())), None => (l, None) };
            let rest = url.strip_prefix("file://")?;
            let path = PathBuf::from(crate::paths::percent_decode(rest));
            let label = label.filter(|s| !s.is_empty()).map(|s| s.to_string()).unwrap_or_else(|| path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into()));
            Some(Bookmark { path, label })
        })
        .collect()
}

pub fn format_bookmarks(b: &[Bookmark]) -> String {
    b.iter()
        .map(|b| {
            let url = format!("file://{}", crate::paths::percent_encode_path(&b.path.to_string_lossy()));
            let default = b.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if b.label == default { format!("{url}\n") } else { format!("{url} {}\n", b.label) }
        })
        .collect()
}

pub fn load_bookmarks(file: &Path) -> Vec<Bookmark> {
    std::fs::read_to_string(file).map(|s| parse_bookmarks(&s)).unwrap_or_default()
}

pub fn save_bookmarks(file: &Path, b: &[Bookmark]) -> std::io::Result<()> {
    if let Some(d) = file.parent() { std::fs::create_dir_all(d)?; }
    let tmp = file.with_extension("tmp");
    std::fs::write(&tmp, format_bookmarks(b))?;
    std::fs::rename(tmp, file)
}

/// Add (no duplicates).
pub fn add_bookmark(list: &mut Vec<Bookmark>, path: &Path) -> bool {
    if list.iter().any(|b| b.path == path) { return false; }
    list.push(Bookmark { path: path.to_path_buf(), label: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into()) });
    true
}

/// XDG user dirs (`user-dirs.dirs`): Desktop, Documents, Downloads, Music, Pictures, Videos.
pub fn user_dirs() -> Vec<Bookmark> {
    let home = crate::paths::home();
    let text = std::fs::read_to_string(config_home().join("user-dirs.dirs")).unwrap_or_default();
    parse_user_dirs(&text, &home)
}

pub fn parse_user_dirs(text: &str, home: &Path) -> Vec<Bookmark> {
    let mut out = vec![];
    for l in text.lines() {
        let l = l.trim();
        let Some(rest) = l.strip_prefix("XDG_") else { continue };
        let Some((k, v)) = rest.split_once("_DIR=") else { continue };
        let v = v.trim().trim_matches('"').replace("$HOME", &home.to_string_lossy());
        let label = match k { "DESKTOP" => "Desktop", "DOCUMENTS" => "Documents", "DOWNLOAD" => "Downloads", "MUSIC" => "Music", "PICTURES" => "Pictures", "VIDEOS" => "Videos", _ => continue };
        let p = PathBuf::from(v);
        if p != home { out.push(Bookmark { path: p, label: label.into() }); }
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mount {
    pub device: String,
    pub path: PathBuf,
    pub fstype: String,
    pub label: String,
}

const REAL_FS: &[&str] = &["ext2", "ext3", "ext4", "btrfs", "xfs", "f2fs", "vfat", "exfat", "ntfs", "ntfs3", "fuseblk", "nfs", "nfs4", "cifs", "zfs", "reiserfs", "jfs", "iso9660", "udf", "hfsplus", "bcachefs", "erofs", "squashfs"];

/// Parse /proc/self/mountinfo, keeping only real, user-relevant filesystems.
pub fn parse_mountinfo(text: &str) -> Vec<Mount> {
    let mut out: Vec<Mount> = vec![];
    for l in text.lines() {
        let Some((pre, post)) = l.split_once(" - ") else { continue };
        let f: Vec<&str> = pre.split(' ').collect();
        let p: Vec<&str> = post.split(' ').collect();
        if f.len() < 5 || p.len() < 2 { continue; }
        let mp = unescape_mount(f[4]);
        let (fstype, dev) = (p[0], p[1]);
        if !REAL_FS.contains(&fstype) { continue; }
        if mp.starts_with("/boot") || mp.starts_with("/var/lib") || mp.starts_with("/snap") || mp.starts_with("/sys") || mp.starts_with("/proc") { continue; }
        if squash_snap(fstype, &mp) { continue; }
        if out.iter().any(|m| m.path == Path::new(&mp)) { continue; }
        // one entry per device: btrfs subvolumes and bind mounts share it
        if let Some(i) = out.iter().position(|m| m.device == dev) {
            if mp.len() < out[i].path.as_os_str().len() { let l = if mp == "/" { "Root".to_string() } else { Path::new(&mp).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(mp.clone()) }; out[i].path = PathBuf::from(&mp); out[i].label = l; }
            continue;
        }
        let label = if mp == "/" { "Root".to_string() } else { Path::new(&mp).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(mp.clone()) };
        out.push(Mount { device: dev.to_string(), path: PathBuf::from(mp), fstype: fstype.to_string(), label });
    }
    out
}
fn squash_snap(fs: &str, mp: &str) -> bool {
    fs == "squashfs" && !mp.starts_with("/media") && !mp.starts_with("/run/media")
}
fn unescape_mount(s: &str) -> String {
    s.replace("\\040", " ").replace("\\011", "\t").replace("\\012", "\n").replace("\\134", "\\")
}

pub fn mounts() -> Vec<Mount> {
    parse_mountinfo(&std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default())
}

/// (total, free) bytes of the filesystem holding `p`.
pub fn space(p: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(p.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 { return None; }
    Some((s.f_blocks as u64 * s.f_frsize as u64, s.f_bavail as u64 * s.f_frsize as u64))
}

// ---- recents (own file: `unix_time<TAB>path` newest first) ----

pub fn recents_file() -> PathBuf {
    data_home().join("filem8/recent")
}

pub fn load_recents(file: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(file).unwrap_or_default().lines().filter_map(|l| l.split_once('\t').map(|(_, p)| PathBuf::from(p))).collect()
}

/// Record `p` as most recent; keep at most 50 and drop vanished entries.
pub fn add_recent(file: &Path, p: &Path, now: u64) -> std::io::Result<()> {
    let mut list: Vec<PathBuf> = load_recents(file).into_iter().filter(|x| x != p && x.exists()).collect();
    list.insert(0, p.to_path_buf());
    list.truncate(50);
    if let Some(d) = file.parent() { std::fs::create_dir_all(d)?; }
    let body: String = list.iter().enumerate().map(|(i, q)| format!("{}\t{}\n", now.saturating_sub(i as u64), q.display())).collect();
    std::fs::write(file, body)
}
