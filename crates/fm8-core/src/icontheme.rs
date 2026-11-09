//! freedesktop Icon Theme Specification 0.13 (std only): index.theme parsing,
//! Inherits, Directories with Size/Scale/Type/MinSize/MaxSize/Threshold,
//! hicolor fallback, name-dash fallback, MIME and place name mapping.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirType { Fixed, Scalable, Threshold }

#[derive(Clone, Debug)]
pub struct Dir {
    pub path: String,
    pub size: u32,
    pub scale: u32,
    pub min: u32,
    pub max: u32,
    pub threshold: u32,
    pub ty: DirType,
}

impl Dir {
    fn matches(&self, size: u32, scale: u32) -> bool {
        if self.scale != scale { return false }
        match self.ty {
            DirType::Fixed => self.size == size,
            DirType::Scalable => self.min <= size && size <= self.max,
            DirType::Threshold => self.size.saturating_sub(self.threshold) <= size && size <= self.size + self.threshold,
        }
    }
    fn distance(&self, size: u32, scale: u32) -> u32 {
        let (s, want) = (size * scale, 0);
        let _ = want;
        match self.ty {
            DirType::Fixed => (self.size * self.scale).abs_diff(s),
            DirType::Scalable => { let (lo, hi) = (self.min * self.scale, self.max * self.scale); if s < lo { lo - s } else { s.saturating_sub(hi) } }
            DirType::Threshold => { let (lo, hi) = ((self.size - self.threshold.min(self.size)) * self.scale, (self.size + self.threshold) * self.scale); if s < lo { lo - s } else { s.saturating_sub(hi) } }
        }
    }
}

#[derive(Debug, Default)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub root: PathBuf,
    pub hidden: bool,
    pub inherits: Vec<String>,
    pub dirs: Vec<Dir>,
    /// icon name -> [(dir index, is_svg)]
    index: HashMap<String, Vec<(usize, bool)>>,
}

fn parse_ini(text: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut out: Vec<(String, Vec<(String, String)>)> = vec![];
    for l in text.lines() {
        let l = l.trim();
        if l.is_empty() || l.starts_with('#') { continue }
        if let Some(s) = l.strip_prefix('[').and_then(|x| x.strip_suffix(']')) { out.push((s.to_string(), vec![])) }
        else if let (Some((k, v)), Some(last)) = (l.split_once('='), out.last_mut()) { last.1.push((k.trim().to_string(), v.trim().to_string())) }
    }
    out
}

impl Theme {
    /// Load `<root>/index.theme` and index the icon files of its directories.
    pub fn load(root: &Path) -> Option<Theme> {
        let text = std::fs::read_to_string(root.join("index.theme")).ok()?;
        let ini = parse_ini(&text);
        let head = &ini.iter().find(|(s, _)| s == "Icon Theme")?.1;
        let get = |k: &str| head.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
        let list = |k: &str| get(k).map(|v| v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect::<Vec<_>>()).unwrap_or_default();
        let mut dir_names = list("Directories");
        dir_names.extend(list("ScaledDirectories"));
        let mut t = Theme {
            id: root.file_name()?.to_string_lossy().into_owned(),
            name: get("Name").unwrap_or("").to_string(),
            root: root.to_path_buf(),
            hidden: get("Hidden") == Some("true"),
            inherits: list("Inherits"),
            ..Default::default()
        };
        for dn in dir_names {
            let Some((_, kv)) = ini.iter().find(|(s, _)| *s == dn) else { continue };
            let g = |k: &str| kv.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
            let n = |k: &str| g(k).and_then(|v| v.parse::<u32>().ok());
            let Some(size) = n("Size") else { continue };
            let ty = match g("Type") { Some("Fixed") => DirType::Fixed, Some("Scalable") => DirType::Scalable, _ => DirType::Threshold };
            t.dirs.push(Dir { path: dn, size, scale: n("Scale").unwrap_or(1), min: n("MinSize").unwrap_or(size), max: n("MaxSize").unwrap_or(size), threshold: n("Threshold").unwrap_or(2), ty });
        }
        for (i, d) in t.dirs.iter().enumerate() {
            let Ok(rd) = std::fs::read_dir(root.join(&d.path)) else { continue };
            for e in rd.flatten() {
                let f = e.file_name().to_string_lossy().into_owned();
                if let Some(stem) = f.strip_suffix(".png") { t.index.entry(stem.to_string()).or_default().push((i, false)) }
                else if let Some(stem) = f.strip_suffix(".svg") { t.index.entry(stem.to_string()).or_default().push((i, true)) }
            }
        }
        Some(t)
    }

    pub fn has(&self, icon: &str) -> bool { self.index.contains_key(icon) }

    /// LookupIcon from the spec: exact directory match, else the closest one.
    pub fn lookup(&self, icon: &str, size: u32, scale: u32, svg: bool) -> Option<PathBuf> {
        let cands: Vec<&(usize, bool)> = self.index.get(icon)?.iter().filter(|(_, s)| svg || !*s).collect();
        let pick = |d: &(usize, bool)| self.root.join(&self.dirs[d.0].path).join(format!("{icon}.{}", if d.1 { "svg" } else { "png" }));
        // exact matches: prefer svg inside scalable dirs, png otherwise
        if let Some(d) = cands.iter().filter(|(i, _)| self.dirs[*i].matches(size, scale)).min_by_key(|(i, s)| (!*s as u8, self.dirs[*i].distance(size, scale))) { return Some(pick(d)) }
        let best = cands.iter().min_by_key(|(i, s)| (self.dirs[*i].distance(size, scale), !*s as u8))?;
        Some(pick(best))
    }
}

pub struct IconThemes {
    pub search: Vec<PathBuf>,
    loaded: HashMap<String, Option<Theme>>,
}

pub fn default_search_dirs() -> Vec<PathBuf> {
    let home = crate::paths::home();
    let mut v = vec![home.join(".icons")];
    v.push(std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share")).join("icons"));
    let xdg = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    for d in xdg.split(':').filter(|d| !d.is_empty()) { v.push(Path::new(d).join("icons")) }
    v
}

impl IconThemes {
    pub fn new(search: Vec<PathBuf>) -> IconThemes { IconThemes { search, loaded: HashMap::new() } }
    pub fn system() -> IconThemes { IconThemes::new(default_search_dirs()) }

    /// Ids of installed, non-hidden themes that have icons (cursor-only themes are skipped).
    pub fn installed(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut out = vec![];
        for d in &self.search {
            let Ok(rd) = std::fs::read_dir(d) else { continue };
            for e in rd.flatten() {
                let id = e.file_name().to_string_lossy().into_owned();
                let Ok(txt) = std::fs::read_to_string(e.path().join("index.theme")) else { continue };
                if !txt.contains("Directories=") && !txt.contains("Directories =") { continue }
                let hidden = txt.lines().any(|l| l.trim() == "Hidden=true");
                if !hidden && id != "default" && seen.insert(id.clone()) { out.push(id) }
            }
        }
        out.sort_by_key(|s| s.to_lowercase());
        out
    }

    fn theme(&mut self, id: &str) -> Option<&Theme> {
        if !self.loaded.contains_key(id) {
            let t = self.search.iter().map(|d| d.join(id)).find(|p| p.join("index.theme").is_file()).and_then(|p| Theme::load(&p));
            self.loaded.insert(id.to_string(), t);
        }
        self.loaded.get(id)?.as_ref()
    }

    /// Theme ids in lookup order: the theme, its Inherits (depth first), then hicolor.
    fn chain(&mut self, id: &str) -> Vec<String> {
        let mut order = vec![];
        let mut stack = vec![id.to_string()];
        while let Some(cur) = stack.pop() {
            if order.contains(&cur) { continue }
            let inh = self.theme(&cur).map(|t| t.inherits.clone()).unwrap_or_default();
            order.push(cur);
            for i in inh.into_iter().rev() { stack.push(i) }
        }
        if !order.iter().any(|t| t == "hicolor") { order.push("hicolor".into()) }
        order
    }

    /// FindIcon: theme chain, then hicolor, then `pixmaps`. `svg` = whether the caller can rasterise SVG.
    pub fn find(&mut self, theme: &str, icon: &str, size: u32, scale: u32, svg: bool) -> Option<PathBuf> {
        for t in self.chain(theme) {
            if let Some(p) = self.theme(&t).and_then(|th| th.lookup(icon, size, scale, svg)) { return Some(p) }
        }
        for ext in if svg { &["png", "svg"][..] } else { &["png"][..] } {
            let p = Path::new("/usr/share/pixmaps").join(format!("{icon}.{ext}"));
            if p.is_file() { return Some(p) }
        }
        None
    }

    /// First icon of `names` found, then each name with trailing `-segments` removed (spec fallback).
    pub fn find_any(&mut self, theme: &str, names: &[String], size: u32, scale: u32, svg: bool) -> Option<PathBuf> {
        for n in names { if let Some(p) = self.find(theme, n, size, scale, svg) { return Some(p) } }
        for n in names {
            let mut s = n.as_str();
            while let Some((head, _)) = s.rsplit_once('-') {
                s = head;
                if let Some(p) = self.find(theme, s, size, scale, svg) { return Some(p) }
            }
        }
        None
    }

    /// Whether the theme (or its parents) can supply a folder icon under the given constraints.
    pub fn usable(&mut self, theme: &str, svg: bool) -> bool { self.find(theme, "folder", 48, 1, svg).is_some() && self.find(theme, "text-x-generic", 48, 1, svg).is_some() }

    /// Best theme when the setting is "auto": first usable of the preference list, then any installed.
    pub fn auto(&mut self, svg: bool, dark: bool) -> Option<String> {
        let prefs: [&str; 7] = if dark { ["breeze-dark", "breeze", "Papirus-Dark", "Papirus", "Adwaita", "AdwaitaLegacy", "hicolor"] } else { ["breeze", "breeze-dark", "Papirus", "Papirus-Dark", "Adwaita", "AdwaitaLegacy", "hicolor"] };
        let have = self.installed();
        for p in prefs { if have.iter().any(|h| h == p) && self.usable(p, svg) { return Some(p.to_string()) } }
        for h in have { if self.usable(&h, svg) { return Some(h) } }
        None
    }
}

/// Icon names to try for a MIME type, most specific first (`application/pdf` ->
/// `application-pdf`, `x-office-document`, ..., generic fallbacks per the naming spec).
pub fn names_for_mime(mime: &str) -> Vec<String> {
    let mut v = vec![mime.replace('/', "-")];
    let (media, sub) = mime.split_once('/').unwrap_or((mime, ""));
    let sub = sub.split('+').next().unwrap_or(sub);
    if sub.starts_with("x-") { v.push(format!("{media}-{}", &sub[2..])) } else { v.push(format!("{media}-x-{sub}")) }
    let generic = match (media, sub) {
        ("inode", "directory") => "folder",
        ("text", "html") => "text-html",
        ("application", "pdf") => "x-office-document",
        ("application", "zip" | "gzip" | "x-tar" | "x-7z-compressed" | "x-xz" | "x-bzip2" | "zstd" | "vnd.rar" | "epub+zip") => "package-x-generic",
        ("application", "x-shellscript" | "x-executable" | "x-msdownload" | "vnd.appimage" | "x-sharedlib" | "x-ms-dos-executable" | "vnd.microsoft.portable-executable") => "application-x-executable",
        ("application", s) if s.contains("package") || s.contains("rpm") || s.contains("flatpak") || s.contains("snap") || s.contains("archive") || s.contains("java-archive") || s.contains("compressed") => "package-x-generic",
        ("application", s) if s.contains("msword") || s.contains("opendocument.text") => "x-office-document",
        ("application", s) if s.contains("ms-excel") || s.contains("spreadsheet") => "x-office-spreadsheet",
        ("model", _) => "model",
        ("font", _) => "font-x-generic",
        ("image", _) => "image-x-generic",
        ("audio", _) => "audio-x-generic",
        ("video", _) => "video-x-generic",
        ("text", _) => "text-x-generic",
        _ => "application-x-generic",
    };
    v.push(generic.into());
    if media == "text" { v.push("text-x-generic".into()) }
    // family fallbacks before the "?" icon
    v.push("application-x-generic".into());
    v.push("unknown".into());
    v.dedup();
    v
}

/// Icon names for well-known places.
pub fn names_for_place(kind: &str) -> Vec<String> {
    let l: &[&str] = match kind {
        "home" => &["user-home", "folder-home", "folder"],
        "desktop" => &["user-desktop", "folder-desktop", "folder"],
        "documents" => &["folder-documents", "folder"],
        "downloads" => &["folder-download", "folder-downloads", "folder"],
        "music" => &["folder-music", "folder"],
        "pictures" => &["folder-pictures", "folder"],
        "videos" => &["folder-videos", "folder"],
        "trash" => &["user-trash", "folder-trash", "folder"],
        "disk" => &["drive-harddisk", "drive-removable-media", "folder"],
        "root" => &["drive-harddisk", "folder"],
        "bookmark" => &["folder-bookmark", "bookmark-new", "folder"],
        "recent" => &["document-open-recent", "folder-recent", "folder"],
        _ => &["folder"],
    };
    l.iter().map(|s| s.to_string()).collect()
}
