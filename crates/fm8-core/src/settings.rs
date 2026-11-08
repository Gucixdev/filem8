//! Persistent per-folder view memory and global settings (plain text files).
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    List,
    Grid,
    Compact,
}

impl ViewMode {
    pub fn name(self) -> &'static str { match self { ViewMode::List => "list", ViewMode::Grid => "grid", ViewMode::Compact => "compact" } }
    pub fn parse(s: &str) -> Option<ViewMode> { match s { "list" => Some(ViewMode::List), "grid" => Some(ViewMode::Grid), "compact" => Some(ViewMode::Compact), _ => None } }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FolderView {
    pub mode: ViewMode,
    pub sort_key: String,
    pub ascending: bool,
    pub show_hidden: bool,
}

impl Default for FolderView {
    fn default() -> Self { FolderView { mode: ViewMode::List, sort_key: "name".into(), ascending: true, show_hidden: false } }
}

/// `path<TAB>mode<TAB>sort<TAB>asc<TAB>hidden` per line.
#[derive(Default)]
pub struct FolderMemory {
    pub map: BTreeMap<PathBuf, FolderView>,
    pub default: FolderView,
}

impl FolderMemory {
    pub fn load(file: &Path) -> FolderMemory {
        let mut m = FolderMemory::default();
        for l in std::fs::read_to_string(file).unwrap_or_default().lines() {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() != 5 { continue; }
            let Some(mode) = ViewMode::parse(f[1]) else { continue };
            let v = FolderView { mode, sort_key: f[2].to_string(), ascending: f[3] == "1", show_hidden: f[4] == "1" };
            if f[0] == "*" { m.default = v } else { m.map.insert(PathBuf::from(f[0]), v); }
        }
        m
    }
    pub fn get(&self, p: &Path) -> FolderView { self.map.get(p).cloned().unwrap_or_else(|| self.default.clone()) }
    pub fn set(&mut self, p: &Path, v: FolderView) {
        if v == self.default { self.map.remove(p); } else { self.map.insert(p.to_path_buf(), v); }
    }
    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        if let Some(d) = file.parent() { std::fs::create_dir_all(d)?; }
        let line = |k: &str, v: &FolderView| format!("{}\t{}\t{}\t{}\t{}\n", k, v.mode.name(), v.sort_key, v.ascending as u8, v.show_hidden as u8);
        let mut s = line("*", &self.default);
        for (p, v) in &self.map {
            let ps = p.to_string_lossy();
            if ps.contains(['\t', '\n']) { continue; }
            s.push_str(&line(&ps, v));
        }
        let tmp = file.with_extension("tmp");
        std::fs::write(&tmp, s)?;
        std::fs::rename(tmp, file)
    }
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| crate::paths::home().join(".config")).join("filem8")
}

/// Application preferences: flat `key = value` text file (`~/.config/filem8/settings`).
/// Unknown keys are kept; missing keys fall back to `DEFAULTS`, so the file can
/// stay tiny and survive version changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prefs {
    pub map: BTreeMap<String, String>,
}

pub const DEFAULTS: &[(&str, &str)] = &[
    ("view", "list"),
    ("show_hidden", "false"),
    ("sort", "name"),
    ("sort_ascending", "true"),
    ("thumb_size", "medium"),
    ("thumbnails", "true"),
    ("preview", "true"),
    ("confirm_trash", "false"),
    ("backend", "auto"),
    ("theme", "file"),
    ("icon_theme", "auto"),
    ("key.new_tab", "ctrl+t"),
    ("key.close_tab", "ctrl+w"),
    ("key.path", "ctrl+l"),
    ("key.hidden", "ctrl+h"),
    ("key.layout", "f3"),
    ("key.rename", "f2"),
    ("key.copy", "ctrl+c"),
    ("key.cut", "ctrl+x"),
    ("key.paste", "ctrl+v"),
    ("key.bookmark", "ctrl+d"),
    ("key.settings", "ctrl+,"),
    ("key.reload", "ctrl+r"),
];

impl Prefs {
    pub fn parse(text: &str) -> Prefs {
        let mut map = BTreeMap::new();
        for l in text.lines() {
            let l = l.trim();
            if l.is_empty() || l.starts_with('#') { continue; }
            if let Some((k, v)) = l.split_once('=') { map.insert(k.trim().to_string(), v.trim().to_string()); }
        }
        Prefs { map }
    }
    pub fn load(file: &Path) -> Prefs { Prefs::parse(&std::fs::read_to_string(file).unwrap_or_default()) }
    pub fn get(&self, k: &str) -> String {
        self.map.get(k).cloned().or_else(|| DEFAULTS.iter().find(|(a, _)| *a == k).map(|(_, v)| v.to_string())).unwrap_or_default()
    }
    pub fn flag(&self, k: &str) -> bool { self.get(k) == "true" }
    pub fn set(&mut self, k: &str, v: &str) {
        if DEFAULTS.iter().any(|(a, b)| *a == k && *b == v) { self.map.remove(k); } else { self.map.insert(k.to_string(), v.to_string()); }
    }
    pub fn to_text(&self) -> String {
        let mut s = String::from("# filem8 settings (key = value). Edited live by the settings panel (Ctrl+,).\n");
        for (k, v) in &self.map { s.push_str(&format!("{k} = {v}\n")); }
        s
    }
    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        if let Some(d) = file.parent() { std::fs::create_dir_all(d)?; }
        let tmp = file.with_extension("tmp");
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(tmp, file)
    }
}
