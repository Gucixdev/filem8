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
