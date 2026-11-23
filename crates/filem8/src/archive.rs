//! Archive engine: extract and create zip / tar(.gz) with our own code, everything else
//! (rar, 7z, tar.xz/zst/bz2, xz/zst/bz2 streams, tar.zst, 7z creation) through tools found
//! in PATH, with a clear "install X" message when they are missing.
//!
//! Safety model (all tested with malicious archives):
//! * every member goes to a fresh staging directory inside the destination, never straight
//!   into it; only after validation are the top-level items moved into place, renamed
//!   `name (2)` when something already exists, so nothing is ever overwritten;
//! * `..`, absolute paths, NUL and backslash tricks are rejected (zip-slip); symlinks must stay
//!   inside the staging tree (lexically while extracting, physically afterwards);
//! * total bytes and file count are capped (zip bomb) and the cap is enforced on actual output
//!   (decoders are given the remaining budget; tools are killed when the staging dir grows past it).
use fm8_codec::{inflate, tar, zip};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind { Zip, Tar, TarGz, TarXz, TarZst, TarBz2, SevenZ, Rar, Gz, Xz, Zst, Bz2 }

impl Kind {
    pub fn label(self) -> &'static str {
        match self { Kind::Zip => "zip", Kind::Tar => "tar", Kind::TarGz => "tar.gz", Kind::TarXz => "tar.xz", Kind::TarZst => "tar.zst", Kind::TarBz2 => "tar.bz2", Kind::SevenZ => "7z", Kind::Rar => "rar", Kind::Gz => "gz", Kind::Xz => "xz", Kind::Zst => "zst", Kind::Bz2 => "bz2" }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Total uncompressed bytes.
    pub max_bytes: u64,
    pub max_files: u64,
}
impl Default for Limits { fn default() -> Self { Limits { max_bytes: 8 << 30, max_files: 200_000 } } }

/// Progress + cancel shared with the UI thread.
#[derive(Clone, Default)]
pub struct Prog { pub done: Arc<AtomicU64>, pub total: Arc<AtomicU64>, pub cancel: Arc<AtomicBool> }
impl Prog {
    pub fn new() -> Prog { Prog::default() }
    pub fn cancelled(&self) -> bool { self.cancel.load(Relaxed) }
    pub fn fraction(&self) -> f32 { let t = self.total.load(Relaxed); if t == 0 { 0.0 } else { (self.done.load(Relaxed) as f32 / t as f32).clamp(0.0, 1.0) } }
}

#[derive(Debug, Default)]
pub struct Report {
    pub files: u64,
    pub bytes: u64,
    /// What now exists in the destination (top level, after renames).
    pub top: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

const OWN_ARCHIVE_MAX: u64 = 128 << 20;
const OWN_UNPACKED_MAX: usize = 512 << 20;
const UNSUPPORTED: &str = "UNSUPPORTED:";

// ---------------------------------------------------------------- detection

pub fn kind_of_name(name: &str) -> Option<Kind> {
    let l = name.to_lowercase();
    let e = |s: &str| l.ends_with(s);
    Some(if e(".tar.gz") || e(".tgz") || e(".crate") { Kind::TarGz }
    else if e(".tar.xz") || e(".txz") { Kind::TarXz }
    else if e(".tar.zst") || e(".tzst") || e(".tar.zstd") { Kind::TarZst }
    else if e(".tar.bz2") || e(".tbz2") || e(".tbz") { Kind::TarBz2 }
    else if e(".tar") { Kind::Tar }
    else if e(".zip") || e(".jar") || e(".apk") || e(".whl") || e(".epub") || e(".cbz") { Kind::Zip }
    else if e(".7z") { Kind::SevenZ }
    else if e(".rar") || e(".cbr") { Kind::Rar }
    else if e(".gz") { Kind::Gz } else if e(".xz") { Kind::Xz } else if e(".zst") { Kind::Zst } else if e(".bz2") { Kind::Bz2 }
    else { return None })
}

/// By name, then by magic bytes (so `.tpz` and extension-less archives work).
pub fn detect(path: &Path) -> Option<Kind> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    if let Some(k) = kind_of_name(&name) { return Some(k); }
    use std::io::Read;
    let mut head = vec![0u8; 65536];
    let n = fs::File::open(path).ok()?.read(&mut head).ok()?;
    head.truncate(n);
    let tar_inside = |v: &[u8]| v.get(257..262) == Some(b"ustar");
    Some(match fm8_core::class::mime_sniff(&head)? {
        "application/zip" => Kind::Zip,
        "application/gzip" => if tar_inside(&inflate::gunzip_prefix(&head, 1024).unwrap_or_default()) { Kind::TarGz } else { Kind::Gz },
        "application/x-tar" => Kind::Tar,
        "application/x-7z-compressed" => Kind::SevenZ,
        "application/vnd.rar" => Kind::Rar,
        "application/x-xz" => Kind::Xz, "application/zstd" => Kind::Zst, "application/x-bzip2" => Kind::Bz2,
        _ => return None,
    })
}

pub fn which(tool: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(tool)).find(|p| fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false))
}

fn missing(what: &str, tools: &str, hint: &str) -> String { format!("Cannot {what}: none of {tools} found in PATH. Install {hint} and try again.") }

// ---------------------------------------------------------------- path safety

/// A member name as a safe relative path. Empty result = the root itself.
pub fn safe_rel(name: &str) -> Result<PathBuf, String> {
    if name.contains('\0') { return Err(format!("unsafe name (NUL): {name:?}")); }
    let n = name.replace('\\', "/");
    let mut out = PathBuf::new();
    for c in Path::new(&n).components() {
        match c {
            Component::Normal(x) => { if x.len() > 255 { return Err(format!("name component too long in {name:?}")); } out.push(x) }
            Component::CurDir => {}
            Component::ParentDir => return Err(format!("unsafe path with '..': {name}")),
            Component::RootDir | Component::Prefix(_) => return Err(format!("unsafe absolute path: {name}")),
        }
    }
    Ok(out)
}

/// Lexical check: does a symlink at `rel` pointing to `target` stay inside the tree?
fn link_ok(rel: &Path, target: &str) -> bool {
    if target.is_empty() || target.starts_with('/') || target.contains('\0') { return false; }
    let mut depth = rel.components().count() as i64 - 1;
    for c in Path::new(target).components() {
        match c { Component::Normal(_) => depth += 1, Component::ParentDir => { depth -= 1; if depth < 0 { return false; } } Component::CurDir => {} _ => return false }
    }
    true
}

fn ensure_parent(stage: &Path, canon_stage: &Path, dst: &Path) -> Result<(), String> {
    let par = dst.parent().ok_or("bad path")?;
    fs::create_dir_all(par).map_err(|e| format!("{}: {e}", par.display()))?;
    let c = fs::canonicalize(par).map_err(|e| e.to_string())?;
    if !c.starts_with(canon_stage) { return Err(format!("member would be written outside the target ({})", dst.strip_prefix(stage).unwrap_or(dst).display())); }
    Ok(())
}

pub fn unique(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    if p.symlink_metadata().is_err() { return p; }
    let (stem, ext) = match name.rfind('.') { Some(i) if i > 0 && !name[i..].contains(' ') && name.len() - i <= 8 => (&name[..i], &name[i..]), _ => (name, "") };
    let mut n = 2;
    loop {
        let c = dir.join(format!("{stem} ({n}){ext}"));
        if c.symlink_metadata().is_err() { return c; }
        n += 1;
    }
}

fn bomb(l: &Limits) -> String {
    format!("Extraction stopped: the archive would expand beyond {} bytes or {} files (possible zip bomb). Nothing was extracted.", fm8_core::listing::human_size(l.max_bytes), l.max_files)
}

/// (bytes, files) of a tree, not following symlinks; stops early past the caps.
fn tree_stats(dir: &Path, l: &Limits) -> (u64, u64) {
    let (mut b, mut n) = (0u64, 0u64);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(m) = e.metadata() else { continue };
            n += 1;
            if m.is_dir() { stack.push(e.path()); } else { b += m.len(); }
            if b > l.max_bytes || n > l.max_files { return (b, n); }
        }
    }
    (b, n)
}

/// Final physical pass over a staged tree: drop escaping/absolute symlinks and special files,
/// enforce the caps. Returns (files, bytes).
fn validate_tree(stage: &Path, l: &Limits, warn: &mut Vec<String>) -> Result<(u64, u64), String> {
    let canon = fs::canonicalize(stage).map_err(|e| e.to_string())?;
    let (mut files, mut bytes) = (0u64, 0u64);
    let mut stack = vec![stage.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).map_err(|e| e.to_string())?.flatten() {
            let p = e.path();
            let m = fs::symlink_metadata(&p).map_err(|e| e.to_string())?;
            let rel = p.strip_prefix(stage).unwrap_or(&p).to_path_buf();
            files += 1;
            if files > l.max_files { return Err(bomb(l)); }
            let ft = m.file_type();
            if ft.is_symlink() {
                let t = fs::read_link(&p).map(|t| t.to_string_lossy().into_owned()).unwrap_or_default();
                let bad = !link_ok(&rel, &t) || fs::canonicalize(&p).map(|c| !c.starts_with(&canon)).unwrap_or(false);
                if bad { let _ = fs::remove_file(&p); warn.push(format!("skipped symlink {} -> {t} (points outside the extracted folder)", rel.display())); }
            } else if ft.is_dir() {
                stack.push(p);
            } else if ft.is_file() {
                bytes += m.len();
                if bytes > l.max_bytes { return Err(bomb(l)); }
                if m.mode() & 0o6000 != 0 { let _ = fs::set_permissions(&p, fs::Permissions::from_mode(m.mode() & 0o777)); }
            } else {
                let _ = fs::remove_file(&p);
                warn.push(format!("skipped special file {}", rel.display()));
            }
        }
    }
    Ok((files, bytes))
}

// ---------------------------------------------------------------- extraction

fn write_new(dst: &Path, data: &[u8], mode: u32) -> Result<(), String> {
    let mut f = fs::OpenOptions::new().write(true).create_new(true).mode(mode & 0o777).open(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    f.write_all(data).map_err(|e| format!("{}: {e}", dst.display()))
}

fn fill_zip(b: &[u8], st: &Path, l: &Limits, prog: &Prog, rep: &mut Report) -> Result<(), String> {
    let (es, more) = zip::list(b, l.max_files as usize + 1)?;
    if more || es.len() as u64 > l.max_files { return Err(bomb(l)); }
    let declared: u64 = es.iter().map(|e| e.size).sum();
    if declared > l.max_bytes { return Err(bomb(l)); }
    if es.iter().any(|e| !e.is_dir && e.method != 0 && e.method != 8) { return Err(format!("{UNSUPPORTED}zip method")); }
    prog.total.store(declared, Relaxed);
    let canon = fs::canonicalize(st).map_err(|e| e.to_string())?;
    let mut left = l.max_bytes;
    for e in &es {
        if prog.cancelled() { return Err("Cancelled".into()); }
        let rel = safe_rel(&e.name)?;
        if rel.as_os_str().is_empty() { continue; }
        let dst = st.join(&rel);
        if e.is_dir { ensure_parent(st, &canon, &dst.join("x"))?; continue; }
        ensure_parent(st, &canon, &dst)?;
        let data = zip::read(b, e, (left as usize).min(OWN_UNPACKED_MAX))?;
        left = left.checked_sub(data.len() as u64).ok_or_else(|| bomb(l))?;
        if dst.symlink_metadata().is_ok() { rep.warnings.push(format!("duplicate member {} ignored", rel.display())); continue; }
        write_new(&dst, &data, 0o644)?;
        prog.done.fetch_add(data.len() as u64, Relaxed);
    }
    Ok(())
}

fn fill_tar(buf: &[u8], st: &Path, l: &Limits, prog: &Prog, rep: &mut Report) -> Result<(), String> {
    let (es, more) = tar::list(buf, l.max_files as usize + 1);
    if more || es.len() as u64 > l.max_files { return Err(bomb(l)); }
    let declared: u64 = es.iter().map(|e| e.size).sum();
    if declared > l.max_bytes { return Err(bomb(l)); }
    prog.total.store(declared, Relaxed);
    let canon = fs::canonicalize(st).map_err(|e| e.to_string())?;
    for e in &es {
        if prog.cancelled() { return Err("Cancelled".into()); }
        let rel = safe_rel(&e.name)?;
        if rel.as_os_str().is_empty() { continue; }
        let dst = st.join(&rel);
        match e.ty {
            b'5' => { ensure_parent(st, &canon, &dst.join("x"))?; let _ = fs::set_permissions(&dst, fs::Permissions::from_mode((e.mode & 0o777) | 0o700)); }
            b'0' | b'7' => {
                ensure_parent(st, &canon, &dst)?;
                let data = tar::read(buf, e).ok_or("tar: truncated member")?;
                if dst.symlink_metadata().is_ok() { rep.warnings.push(format!("duplicate member {} ignored", rel.display())); continue; }
                write_new(&dst, data, if e.mode == 0 { 0o644 } else { e.mode })?;
                prog.done.fetch_add(data.len() as u64, Relaxed);
            }
            b'2' => {
                if !link_ok(&rel, &e.link) { return Err(format!("unsafe symlink in archive: {} -> {}", rel.display(), e.link)); }
                ensure_parent(st, &canon, &dst)?;
                if dst.symlink_metadata().is_ok() { continue; }
                std::os::unix::fs::symlink(&e.link, &dst).map_err(|er| format!("{}: {er}", dst.display()))?;
            }
            b'1' => rep.warnings.push(format!("skipped hard link {}", rel.display())),
            _ => rep.warnings.push(format!("skipped special entry {}", rel.display())),
        }
    }
    Ok(())
}

fn tool_log() -> (PathBuf, fs::File) {
    static N: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!("fm8-tool-{}-{}.log", std::process::id(), N.fetch_add(1, Relaxed)));
    let f = fs::File::create(&p).unwrap_or_else(|_| fs::File::open("/dev/null").unwrap());
    (p, f)
}

/// Run a tool, watching the staging dir (progress, zip-bomb cap) and the cancel flag.
fn run_tool(mut cmd: Command, watch: Option<&Path>, l: &Limits, prog: &Prog, ok_codes: &[i32]) -> Result<(), String> {
    let (log, f) = tool_log();
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::from(f));
    let mut c = cmd.spawn().map_err(|e| format!("cannot start {:?}: {e}", cmd.get_program()))?;
    let res = loop {
        match c.try_wait() {
            Ok(Some(st)) => {
                break if st.code().map(|x| ok_codes.contains(&x)).unwrap_or(false) { Ok(()) } else {
                    let msg = fs::read_to_string(&log).unwrap_or_default();
                    let last: Vec<&str> = msg.lines().filter(|x| !x.trim().is_empty()).collect();
                    Err(format!("{} failed ({}){}", cmd.get_program().to_string_lossy(), st, last.last().map(|x| format!(": {x}")).unwrap_or_default()))
                };
            }
            Ok(None) => {
                if prog.cancelled() { let _ = c.kill(); let _ = c.wait(); break Err("Cancelled".into()); }
                if let Some(w) = watch {
                    let (b, n) = tree_stats(w, l);
                    prog.done.store(b, Relaxed);
                    if b > l.max_bytes || n > l.max_files { let _ = c.kill(); let _ = c.wait(); break Err(bomb(l)); }
                }
                std::thread::sleep(std::time::Duration::from_millis(60));
            }
            Err(e) => break Err(e.to_string()),
        }
    };
    let _ = fs::remove_file(&log);
    res
}

fn tool_total(archive: &Path) -> u64 {
    let Some(z) = which("7z").or_else(|| which("7za")) else { return 0 };
    let Ok(o) = Command::new(z).args(["l", "-slt", "-ba"]).arg(archive).stdin(Stdio::null()).stderr(Stdio::null()).output() else { return 0 };
    String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| l.strip_prefix("Size = ")?.trim().parse::<u64>().ok()).sum()
}

fn fill_tool(kind: Kind, archive: &Path, st: &Path, l: &Limits, prog: &Prog) -> Result<(), String> {
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tot = tool_total(archive);
    if tot > l.max_bytes { return Err(bomb(l)); }
    prog.total.store(tot, Relaxed);
    let sevenz = which("7z").or_else(|| which("7za")).or_else(|| which("7zr"));
    let bsdtar = which("bsdtar");
    let mut tried = vec![];
    let mut last_err = None;
    macro_rules! attempt { ($tool:expr, $cmd:expr, $codes:expr) => {{
        tried.push($tool);
        match run_tool($cmd, Some(st), l, prog, $codes) { Ok(()) => return Ok(()), Err(e) if e == "Cancelled" || e.starts_with("Extraction stopped") => return Err(e), Err(e) => last_err = Some(e) }
    }}; }
    let sevenz_cmd = |z: &PathBuf| { let mut c = Command::new(z); c.args(["x", "-y", "-bd", "-p-"]).arg(format!("-o{}", st.display())).arg(archive); c };
    match kind {
        Kind::Rar => {
            if let Some(u) = which("unrar") { let mut c = Command::new(u); c.args(["x", "-y", "-idq", "-p-", "-o-"]).arg(archive).arg(format!("{}/", st.display())); attempt!("unrar", c, &[0]); }
            if let Some(z) = &sevenz { attempt!("7z", sevenz_cmd(z), &[0, 1]); }
            if let Some(b) = &bsdtar { let mut c = Command::new(b); c.arg("-xf").arg(archive).arg("-C").arg(st); attempt!("bsdtar", c, &[0]); }
            if tried.is_empty() { return Err(missing("extract .rar", "unrar, 7z, bsdtar", "unrar (pacman -S unrar / apt install unrar) or 7zip")); }
        }
        Kind::SevenZ | Kind::Zip => {
            if let Some(z) = &sevenz { attempt!("7z", sevenz_cmd(z), &[0, 1]); }
            if let Some(b) = &bsdtar { let mut c = Command::new(b); c.arg("-xf").arg(archive).arg("-C").arg(st); attempt!("bsdtar", c, &[0]); }
            if tried.is_empty() { return Err(missing(&format!("extract .{}", kind.label()), "7z, 7za, bsdtar", "7zip (pacman -S 7zip / apt install p7zip-full) or libarchive-tools")); }
        }
        Kind::Tar | Kind::TarGz | Kind::TarXz | Kind::TarZst | Kind::TarBz2 => {
            if let Some(b) = &bsdtar { let mut c = Command::new(b); c.arg("-xf").arg(archive).arg("-C").arg(st); attempt!("bsdtar", c, &[0]); }
            if let Some(t) = which("tar") { let mut c = Command::new(t); c.arg("-xf").arg(archive).arg("-C").arg(st); attempt!("tar", c, &[0]); }
            if tried.is_empty() { return Err(missing(&format!("extract .{}", kind.label()), "bsdtar, tar", "tar (and xz/zstd/bzip2 for those compressions)")); }
        }
        Kind::Gz | Kind::Xz | Kind::Zst | Kind::Bz2 => {
            let (tool, hint) = match kind { Kind::Gz => ("gzip", "gzip"), Kind::Xz => ("xz", "xz (pacman -S xz)"), Kind::Zst => ("zstd", "zstd (pacman -S zstd)"), _ => ("bzip2", "bzip2") };
            let out_name = name.rsplit_once('.').map(|x| x.0.to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("{name}.out"));
            let dst = st.join(safe_rel(&out_name)?);
            if let Some(t) = which(tool) {
                let out = fs::File::create(&dst).map_err(|e| e.to_string())?;
                let mut c = Command::new(t); c.arg("-dc").arg(archive);
                let (log, f) = tool_log();
                c.stdin(Stdio::null()).stdout(Stdio::from(out)).stderr(Stdio::from(f));
                let mut ch = c.spawn().map_err(|e| e.to_string())?;
                let res = loop {
                    match ch.try_wait() {
                        Ok(Some(s)) => break if s.success() { Ok(()) } else { Err(format!("{tool} failed ({s})")) },
                        Ok(None) => {
                            if prog.cancelled() { let _ = ch.kill(); let _ = ch.wait(); break Err("Cancelled".to_string()); }
                            let sz = fs::metadata(&dst).map(|m| m.len()).unwrap_or(0);
                            prog.done.store(sz, Relaxed);
                            if sz > l.max_bytes { let _ = ch.kill(); let _ = ch.wait(); break Err(bomb(l)); }
                            std::thread::sleep(std::time::Duration::from_millis(40));
                        }
                        Err(e) => break Err(e.to_string()),
                    }
                };
                let _ = fs::remove_file(&log);
                return res;
            }
            if let Some(z) = &sevenz {
                let mut c = Command::new(z); c.args(["e", "-y", "-bd", "-p-"]).arg(format!("-o{}", st.display())).arg(archive);
                return run_tool(c, Some(st), l, prog, &[0, 1]);
            }
            return Err(missing(&format!("extract .{}", kind.label()), &format!("{tool}, 7z"), hint));
        }
    }
    Err(last_err.unwrap_or_else(|| "extraction failed".into()))
}

fn fill_own_or_tool(kind: Kind, archive: &Path, st: &Path, l: &Limits, prog: &Prog, rep: &mut Report) -> Result<(), String> {
    let size = fs::metadata(archive).map(|m| m.len()).unwrap_or(0);
    let own: Option<Result<(), String>> = if size <= OWN_ARCHIVE_MAX {
        match kind {
            Kind::Zip => Some(fs::read(archive).map_err(|e| e.to_string()).and_then(|b| fill_zip(&b, st, l, prog, rep))),
            Kind::Tar => Some(fs::read(archive).map_err(|e| e.to_string()).and_then(|b| fill_tar(&b, st, l, prog, rep))),
            Kind::TarGz => Some(fs::read(archive).map_err(|e| e.to_string()).and_then(|b| {
                let cap = OWN_UNPACKED_MAX.min(l.max_bytes as usize + 1024);
                let t = inflate::gunzip_prefix(&b, cap)?;
                if t.len() >= cap { return Err(format!("{UNSUPPORTED}too big for the built-in decoder")); }
                fill_tar(&t, st, l, prog, rep)
            })),
            Kind::Gz => Some(fs::read(archive).map_err(|e| e.to_string()).and_then(|b| {
                let cap = OWN_UNPACKED_MAX.min(l.max_bytes as usize + 1);
                let d = inflate::gunzip_prefix(&b, cap)?;
                if d.len() >= cap { return Err(format!("{UNSUPPORTED}too big")); }
                let n = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let out = n.rsplit_once('.').map(|x| x.0.to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("{n}.out"));
                write_new(&st.join(safe_rel(&out)?), &d, 0o644)?;
                prog.done.store(d.len() as u64, Relaxed);
                Ok(())
            })),
            _ => None,
        }
    } else { None };
    match own {
        Some(Err(e)) if e.starts_with(UNSUPPORTED) => {
            // partial output from the attempt must not leak into the tool run
            if let Ok(rd) = fs::read_dir(st) { for x in rd.flatten() { let p = x.path(); if p.is_dir() { let _ = fs::remove_dir_all(&p); } else { let _ = fs::remove_file(&p); } } }
            prog.done.store(0, Relaxed);
            fill_tool(kind, archive, st, l, prog)
        }
        Some(r) => r,
        None => fill_tool(kind, archive, st, l, prog),
    }
}

/// Extract `archive` into `dest` (created if needed). See the module docs for the guarantees.
pub fn extract(archive: &Path, dest: &Path, l: &Limits, prog: &Prog) -> Result<Report, String> {
    let kind = detect(archive).ok_or_else(|| format!("{}: not a supported archive", archive.display()))?;
    fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    static N: AtomicU64 = AtomicU64::new(0);
    let st = dest.join(format!(".fm8-extract-{}-{}", std::process::id(), N.fetch_add(1, Relaxed)));
    fs::create_dir(&st).map_err(|e| format!("{}: {e}", st.display()))?;
    let mut rep = Report::default();
    let cleanup = |st: &Path| { let _ = fs::remove_dir_all(st); };
    if let Err(e) = fill_own_or_tool(kind, archive, &st, l, prog, &mut rep) { cleanup(&st); return Err(e); }
    match validate_tree(&st, l, &mut rep.warnings) { Ok((f, b)) => { rep.files = f; rep.bytes = b; } Err(e) => { cleanup(&st); return Err(e); } }
    let mut moved = 0;
    let mut renamed = 0;
    let kids: Vec<_> = fs::read_dir(&st).map_err(|e| e.to_string())?.flatten().collect();
    for k in kids {
        let name = k.file_name().to_string_lossy().into_owned();
        let to = unique(dest, &name);
        if to.file_name().map(|n| n.to_string_lossy() != name).unwrap_or(false) { renamed += 1; }
        match fs::rename(k.path(), &to) { Ok(()) => { rep.top.push(to); moved += 1; } Err(e) => rep.warnings.push(format!("{name}: {e}")) }
    }
    cleanup(&st);
    if renamed > 0 { rep.warnings.push(format!("{renamed} item(s) already existed and were extracted under a new name (nothing was overwritten)")); }
    if moved == 0 && rep.warnings.is_empty() { rep.warnings.push("the archive is empty".into()); }
    Ok(rep)
}

// ---------------------------------------------------------------- creation

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format { Zip, TarGz, TarZst, SevenZ }
impl Format {
    pub fn ext(self) -> &'static str { match self { Format::Zip => "zip", Format::TarGz => "tar.gz", Format::TarZst => "tar.zst", Format::SevenZ => "7z" } }
}

struct Item { name: String, path: PathBuf, is_dir: bool, mode: u32, mtime: i64, size: u64 }

fn gather(sel: &[PathBuf], warn: &mut Vec<String>, cancel: &Prog) -> Result<Vec<Item>, String> {
    let mut v = vec![];
    for s in sel {
        let base = s.file_name().ok_or("cannot compress a root")?.to_string_lossy().into_owned();
        let mut stack = vec![(s.clone(), base)];
        while let Some((p, name)) = stack.pop() {
            if cancel.cancelled() { return Err("Cancelled".into()); }
            let m = match fs::symlink_metadata(&p) { Ok(m) => m, Err(e) => { warn.push(format!("{}: {e}", p.display())); continue; } };
            if m.file_type().is_symlink() { warn.push(format!("skipped symlink {}", p.display())); continue; }
            if m.is_dir() {
                v.push(Item { name: format!("{name}/"), path: p.clone(), is_dir: true, mode: m.mode() & 0o777, mtime: m.mtime(), size: 0 });
                if let Ok(rd) = fs::read_dir(&p) {
                    let mut kids: Vec<_> = rd.flatten().map(|e| (e.path(), format!("{name}/{}", e.file_name().to_string_lossy()))).collect();
                    kids.sort_by(|a, b| b.1.cmp(&a.1)); // stack pops last first
                    stack.extend(kids);
                }
            } else if m.is_file() {
                v.push(Item { name, path: p, is_dir: false, mode: m.mode() & 0o777, mtime: m.mtime(), size: m.len() });
            }
        }
    }
    Ok(v)
}

fn dos_time(secs: i64) -> (u16, u16) {
    let t = secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    let y = (tm.tm_year + 1900).clamp(1980, 2107) as u16;
    (((tm.tm_hour as u16) << 11) | ((tm.tm_min as u16) << 5) | (tm.tm_sec as u16 / 2), ((y - 1980) << 9) | (((tm.tm_mon + 1) as u16) << 5) | tm.tm_mday as u16)
}

fn write_zip(items: &[Item], out: &mut impl Write, prog: &Prog) -> Result<(), String> {
    if items.len() > 65000 { return Err(format!("{UNSUPPORTED}too many files for the built-in zip writer")); }
    let (mut off, mut central) = (0u64, vec![]);
    let put = |out: &mut dyn Write, b: &[u8], off: &mut u64| -> Result<(), String> { out.write_all(b).map_err(|e| e.to_string())?; *off += b.len() as u64; Ok(()) };
    for it in items {
        if prog.cancelled() { return Err("Cancelled".into()); }
        let data = if it.is_dir { vec![] } else { fs::read(&it.path).map_err(|e| format!("{}: {e}", it.path.display()))? };
        if data.len() > (512 << 20) { return Err(format!("{UNSUPPORTED}file too large for the built-in zip writer")); }
        let crc = inflate::crc32(&[&data]);
        let (method, body) = if data.is_empty() || data.len() > (32 << 20) { (0u16, data.clone()) } else { let d = inflate::deflate(&data); if d.len() < data.len() { (8u16, d) } else { (0u16, data.clone()) } };
        if off + body.len() as u64 > 0xFFFF_0000 { return Err(format!("{UNSUPPORTED}over 4 GiB")); }
        let (dt, dd) = dos_time(it.mtime);
        let name = it.name.as_bytes();
        let mut lh = vec![];
        lh.extend(b"PK\x03\x04"); lh.extend(20u16.to_le_bytes()); lh.extend(0x800u16.to_le_bytes()); lh.extend(method.to_le_bytes());
        lh.extend(dt.to_le_bytes()); lh.extend(dd.to_le_bytes()); lh.extend(crc.to_le_bytes());
        lh.extend((body.len() as u32).to_le_bytes()); lh.extend((data.len() as u32).to_le_bytes()); lh.extend((name.len() as u16).to_le_bytes()); lh.extend(0u16.to_le_bytes()); lh.extend(name);
        let local = off;
        let mut c = vec![];
        c.extend(b"PK\x01\x02"); c.extend(((3u16 << 8) | 20).to_le_bytes()); c.extend(20u16.to_le_bytes()); c.extend(0x800u16.to_le_bytes()); c.extend(method.to_le_bytes());
        c.extend(dt.to_le_bytes()); c.extend(dd.to_le_bytes()); c.extend(crc.to_le_bytes()); c.extend((body.len() as u32).to_le_bytes()); c.extend((data.len() as u32).to_le_bytes());
        c.extend((name.len() as u16).to_le_bytes()); c.extend([0u8; 8]);
        let mode = (if it.is_dir { 0o040000 } else { 0o100000 }) | it.mode;
        c.extend((mode << 16 | if it.is_dir { 0x10 } else { 0 }).to_le_bytes()); c.extend((local as u32).to_le_bytes()); c.extend(name);
        central.push(c);
        put(out, &lh, &mut off)?;
        put(out, &body, &mut off)?;
        prog.done.fetch_add(data.len() as u64, Relaxed);
    }
    let cd_off = off;
    let mut cd_len = 0u64;
    for c in &central { put(out, c, &mut off)?; cd_len += c.len() as u64; }
    let mut e = vec![];
    e.extend(b"PK\x05\x06"); e.extend([0u8; 4]); e.extend((central.len() as u16).to_le_bytes()); e.extend((central.len() as u16).to_le_bytes());
    e.extend((cd_len as u32).to_le_bytes()); e.extend((cd_off as u32).to_le_bytes()); e.extend(0u16.to_le_bytes());
    put(out, &e, &mut off)
}

fn tar_header(name: &str, size: u64, mode: u32, mtime: i64, ty: u8) -> Vec<u8> {
    let mut out = vec![];
    let mut h = [0u8; 512];
    let oct = |h: &mut [u8], v: u64, w: usize| { let s = format!("{:0w$o}", v, w = w - 1); h[..w - 1].copy_from_slice(s.as_bytes()); };
    let nb = name.as_bytes();
    if nb.len() > 100 {
        // GNU long name member
        out.extend(tar_header("././@LongLink", nb.len() as u64 + 1, 0o644, 0, b'L'));
        out.extend(nb); out.push(0);
        out.extend(vec![0u8; (512 - (nb.len() + 1) % 512) % 512]);
    }
    h[..nb.len().min(100)].copy_from_slice(&nb[..nb.len().min(100)]);
    oct(&mut h[100..108], mode as u64, 8); oct(&mut h[108..116], 0, 8); oct(&mut h[116..124], 0, 8);
    oct(&mut h[124..136], size, 12); oct(&mut h[136..148], mtime.max(0) as u64, 12);
    h[148..156].copy_from_slice(b"        ");
    h[156] = ty;
    h[257..263].copy_from_slice(b"ustar\0"); h[263..265].copy_from_slice(b"00");
    let sum: u32 = h.iter().map(|&b| b as u32).sum();
    h[148..155].copy_from_slice(format!("{:06o}\0", sum).as_bytes()); h[155] = b' ';
    out.extend(h);
    out
}

fn build_tar(items: &[Item], prog: &Prog) -> Result<Vec<u8>, String> {
    let mut t = vec![];
    for it in items {
        if prog.cancelled() { return Err("Cancelled".into()); }
        if it.is_dir { t.extend(tar_header(&it.name, 0, it.mode | 0o700, it.mtime, b'5')); continue; }
        let data = fs::read(&it.path).map_err(|e| format!("{}: {e}", it.path.display()))?;
        t.extend(tar_header(&it.name, data.len() as u64, it.mode, it.mtime, b'0'));
        t.extend(&data);
        t.extend(vec![0u8; (512 - data.len() % 512) % 512]);
        prog.done.fetch_add(data.len() as u64, Relaxed);
        if t.len() > (32 << 20) { return Err(format!("{UNSUPPORTED}too big for the built-in tar.gz writer")); }
    }
    t.extend(vec![0u8; 1024]);
    Ok(t)
}

fn gzip(d: &[u8]) -> Vec<u8> {
    let mut o = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255];
    o.extend(inflate::deflate(d));
    o.extend(inflate::crc32(&[d]).to_le_bytes());
    o.extend((d.len() as u32).to_le_bytes());
    o
}

/// Create `out` (a path that must not exist) from `sel`. Own code for zip and small tar.gz,
/// tools for tar.zst, 7z and whatever is too big for the built-in writers.
pub fn compress(sel: &[PathBuf], out: &Path, fmt: Format, prog: &Prog) -> Result<Report, String> {
    if out.symlink_metadata().is_ok() { return Err(format!("{} already exists", out.display())); }
    if sel.is_empty() { return Err("nothing selected".into()); }
    let mut rep = Report::default();
    let part = out.with_file_name(format!(".{}.part", out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
    let items = gather(sel, &mut rep.warnings, prog)?;
    rep.files = items.iter().filter(|i| !i.is_dir).count() as u64;
    rep.bytes = items.iter().map(|i| i.size).sum();
    prog.total.store(rep.bytes, Relaxed);
    let cleanup = |p: &Path| { let _ = fs::remove_file(p); };
    let via_tool = |part: &Path| -> Result<(), String> {
        let parent = sel[0].parent().unwrap_or(Path::new("/")).to_path_buf();
        let names: Vec<_> = sel.iter().filter_map(|s| s.file_name().map(|n| n.to_os_string())).collect();
        let l = Limits { max_bytes: u64::MAX, max_files: u64::MAX };
        let cmd = match fmt {
            Format::SevenZ => {
                let z = which("7z").or_else(|| which("7za")).or_else(|| which("7zr")).ok_or_else(|| missing("create .7z", "7z, 7za", "7zip (pacman -S 7zip / apt install p7zip-full)"))?;
                let mut c = Command::new(z); c.args(["a", "-t7z", "-y", "-bd"]).arg(part).args(&names); c
            }
            Format::TarZst | Format::TarGz => {
                let z = if fmt == Format::TarZst { "--zstd" } else { "-z" };
                let t = which("bsdtar").or_else(|| which("tar")).ok_or_else(|| missing(&format!("create .{}", fmt.ext()), "bsdtar, tar", "tar"))?;
                if fmt == Format::TarZst && which("zstd").is_none() && !t.ends_with("bsdtar") { return Err(missing("create .tar.zst", "zstd", "zstd (pacman -S zstd)")); }
                let mut c = Command::new(t); c.arg(z).arg("-cf").arg(part).args(&names); c
            }
            Format::Zip => {
                let z = which("7z").or_else(|| which("7za")).ok_or_else(|| missing("create this .zip", "7z", "7zip"))?;
                let mut c = Command::new(z); c.args(["a", "-tzip", "-y", "-bd"]).arg(part).args(&names); c
            }
        };
        let mut cmd = cmd;
        cmd.current_dir(&parent);
        let watch_prog = prog.clone();
        let (p2, done) = (part.to_path_buf(), watch_prog.done.clone());
        let stop = Arc::new(AtomicBool::new(false));
        let st2 = stop.clone();
        let th = std::thread::spawn(move || while !st2.load(Relaxed) { done.store(fs::metadata(&p2).map(|m| m.len()).unwrap_or(0), Relaxed); std::thread::sleep(std::time::Duration::from_millis(80)); });
        let r = run_tool(cmd, None, &l, prog, &[0, 1]);
        stop.store(true, Relaxed);
        let _ = th.join();
        r
    };
    let built = match fmt {
        Format::Zip => {
            let r = fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display())).and_then(|f| { let mut w = std::io::BufWriter::new(f); let r = write_zip(&items, &mut w, prog); w.flush().map_err(|e| e.to_string()).and(r) });
            match r { Err(e) if e.starts_with(UNSUPPORTED) => { cleanup(&part); via_tool(&part) } r => r }
        }
        Format::TarGz => {
            let r = build_tar(&items, prog).map(|t| gzip(&t)).and_then(|g| fs::write(&part, g).map_err(|e| e.to_string()));
            match r { Err(e) if e.starts_with(UNSUPPORTED) => { prog.done.store(0, Relaxed); via_tool(&part) } r => r }
        }
        Format::TarZst | Format::SevenZ => via_tool(&part),
    };
    if let Err(e) = built { cleanup(&part); return Err(e); }
    fs::rename(&part, out).map_err(|e| { cleanup(&part); format!("{}: {e}", out.display()) })?;
    rep.top.push(out.to_path_buf());
    Ok(rep)
}
