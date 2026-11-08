//! Path helpers: breadcrumbs, `~` expansion, completion.
use std::path::{Component, Path, PathBuf};

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

pub fn expand(s: &str) -> PathBuf {
    let s = s.trim();
    if s == "~" {
        home()
    } else if let Some(r) = s.strip_prefix("~/") {
        home().join(r)
    } else if let Some(r) = s.strip_prefix("file://") {
        PathBuf::from(percent_decode(r))
    } else {
        PathBuf::from(s)
    }
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn percent_encode_path(p: &str) -> String {
    let mut o = String::new();
    for &c in p.as_bytes() {
        if c.is_ascii_alphanumeric() || b"/-_.~".contains(&c) { o.push(c as char) } else { o.push_str(&format!("%{c:02X}")) }
    }
    o
}

/// Breadcrumb segments: (label, full path), root first. Home collapses to "~"
/// when the path is under $HOME.
pub fn breadcrumbs(p: &Path) -> Vec<(String, PathBuf)> {
    let mut out = vec![];
    // always the full path from "/" (not "~"), so it matches what Edit shows
    let start = PathBuf::from("/");
    out.push(("/".to_string(), start.clone()));
    let rel = p.strip_prefix(&start).unwrap_or(Path::new(""));
    let mut acc = start;
    for c in rel.components() {
        if let Component::Normal(n) = c {
            acc.push(n);
            out.push((n.to_string_lossy().into_owned(), acc.clone()));
        }
    }
    out
}

/// Directory name completions for the path bar: `input` may be partial.
/// Returns full path strings of matching directories (case-insensitive prefix).
pub fn complete(input: &str, limit: usize) -> Vec<String> {
    let exp = expand(input);
    let (dir, prefix) = if input.ends_with('/') || input == "~" { (exp.clone(), String::new()) } else { (exp.parent().map(|p| p.to_path_buf()).unwrap_or_default(), exp.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()) };
    let pl = prefix.to_lowercase();
    let Ok(rd) = std::fs::read_dir(&dir) else { return vec![] };
    let mut v: Vec<String> = rd
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false) || e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.to_lowercase().starts_with(&pl) && (prefix.starts_with('.') || !n.starts_with('.')))
        .collect();
    v.sort_by(|a, b| crate::listing::natural_cmp(a, b));
    v.truncate(limit);
    v.into_iter().map(|n| dir.join(n).to_string_lossy().into_owned()).collect()
}

pub fn parent(p: &Path) -> Option<PathBuf> {
    p.parent().map(|x| x.to_path_buf())
}
