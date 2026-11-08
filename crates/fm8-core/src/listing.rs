//! Directory listing: async reader, sorting, filtering.

use crate::class::{classify_name, Class};
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::SystemTime;

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub hidden: bool,
    pub class: Class,
    pub mode: u32,
}

impl Entry {
    pub fn from_path(path: &Path) -> std::io::Result<Entry> {
        use std::os::unix::fs::PermissionsExt;
        let lm = std::fs::symlink_metadata(path)?;
        let is_symlink = lm.file_type().is_symlink();
        let m = if is_symlink { std::fs::metadata(path).unwrap_or(lm) } else { lm };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into());
        let is_dir = m.is_dir();
        Ok(Entry {
            hidden: name.starts_with('.'),
            class: classify_name(&name, is_dir),
            name,
            path: path.to_path_buf(),
            is_dir,
            is_symlink,
            size: if is_dir { 0 } else { m.len() },
            modified: m.modified().ok(),
            mode: m.permissions().mode(),
        })
    }
}

pub enum Msg {
    Chunk(Vec<Entry>),
    Done,
    Error(String),
}

/// Start listing `dir` on a background thread. Entries arrive in chunks of
/// 512 so the UI shows the first rows of a huge folder immediately.
/// Dropping the receiver stops the thread early.
pub fn spawn(dir: PathBuf) -> Receiver<Msg> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(Msg::Error(format!("{}: {}", dir.display(), e)));
                return;
            }
        };
        let mut chunk = Vec::with_capacity(512);
        for de in rd.flatten() {
            if let Ok(e) = Entry::from_path(&de.path()) {
                chunk.push(e);
            } else {
                // broken symlink etc: still show it
                let name = de.file_name().to_string_lossy().into_owned();
                chunk.push(Entry { hidden: name.starts_with('.'), class: Class::Other, name, path: de.path(), is_dir: false, is_symlink: true, size: 0, modified: None, mode: 0 });
            }
            if chunk.len() >= 512 && tx.send(Msg::Chunk(std::mem::take(&mut chunk))).is_err() {
                return;
            }
        }
        if !chunk.is_empty() && tx.send(Msg::Chunk(chunk)).is_err() {
            return;
        }
        let _ = tx.send(Msg::Done);
    });
    rx
}

/// Blocking convenience for tests and small dirs.
pub fn list_blocking(dir: &Path) -> Result<Vec<Entry>, String> {
    let rx = spawn(dir.to_path_buf());
    let mut all = vec![];
    loop {
        match rx.recv() {
            Ok(Msg::Chunk(c)) => all.extend(c),
            Ok(Msg::Done) | Err(_) => return Ok(all),
            Ok(Msg::Error(e)) => return Err(e),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Modified,
    Type,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    pub key: SortKey,
    pub ascending: bool,
    pub dirs_first: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Sort { key: SortKey::Name, ascending: true, dirs_first: true }
    }
}

/// Natural, case-insensitive comparison: "file2" < "file10".
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut nx = String::new();
                while let Some(c) = ai.peek().copied().filter(|c| c.is_ascii_digit()) { nx.push(c); ai.next(); }
                let mut ny = String::new();
                while let Some(c) = bi.peek().copied().filter(|c| c.is_ascii_digit()) { ny.push(c); bi.next(); }
                let (tx, ty) = (nx.trim_start_matches('0'), ny.trim_start_matches('0'));
                let o = tx.len().cmp(&ty.len()).then_with(|| tx.cmp(ty));
                if o != Ordering::Equal { return o; }
            }
            (Some(x), Some(y)) => {
                let o = x.to_lowercase().cmp(y.to_lowercase());
                if o != Ordering::Equal { return o; }
                ai.next();
                bi.next();
            }
        }
    }
}

pub fn compare(a: &Entry, b: &Entry, s: Sort) -> Ordering {
    if s.dirs_first && a.is_dir != b.is_dir {
        return if a.is_dir { Ordering::Less } else { Ordering::Greater };
    }
    let o = match s.key {
        SortKey::Name => natural_cmp(&a.name, &b.name),
        SortKey::Size => a.size.cmp(&b.size).then_with(|| natural_cmp(&a.name, &b.name)),
        SortKey::Modified => a.modified.cmp(&b.modified).then_with(|| natural_cmp(&a.name, &b.name)),
        SortKey::Type => a.class.cmp(&b.class).then_with(|| crate::class::ext_of(&a.name).cmp(&crate::class::ext_of(&b.name))).then_with(|| natural_cmp(&a.name, &b.name)),
    };
    if s.ascending { o } else { o.reverse() }
}

/// Entries of one directory plus a sorted/filtered index; selection is kept
/// by path so it survives re-sorting and live refreshes.
#[derive(Default)]
pub struct DirModel {
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    /// Indexes into `entries`, in display order.
    pub view: Vec<usize>,
    pub sort: Sort,
    pub show_hidden: bool,
    pub filter: String,
    pub selected: std::collections::HashSet<PathBuf>,
    /// Cursor / anchor as position in `view`.
    pub cursor: Option<usize>,
    anchor: Option<usize>,
    pub loading: bool,
    pub error: Option<String>,
    rx: Option<Receiver<Msg>>,
}

impl DirModel {
    pub fn new(dir: PathBuf) -> DirModel {
        let mut m = DirModel { dir, ..Default::default() };
        m.reload();
        m
    }

    pub fn reload(&mut self) {
        self.entries.clear();
        self.view.clear();
        self.error = None;
        self.loading = true;
        self.rx = Some(spawn(self.dir.clone()));
    }

    pub fn navigate(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.selected.clear();
        self.cursor = None;
        self.anchor = None;
        self.filter.clear();
        self.reload();
    }

    /// Pump the loader; returns true if anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        let Some(rx) = &self.rx else { return false };
        loop {
            match rx.try_recv() {
                Ok(Msg::Chunk(c)) => {
                    self.entries.extend(c);
                    changed = true;
                }
                Ok(Msg::Done) => {
                    self.loading = false;
                    self.rx = None;
                    changed = true;
                    break;
                }
                Ok(Msg::Error(e)) => {
                    self.error = Some(e);
                    self.loading = false;
                    self.rx = None;
                    changed = true;
                    break;
                }
                Err(_) => break,
            }
        }
        if changed {
            self.rebuild_view();
        }
        changed
    }

    pub fn rebuild_view(&mut self) {
        let cur_path = self.cursor.and_then(|c| self.view.get(c)).map(|&i| self.entries[i].path.clone());
        let f = self.filter.to_lowercase();
        let mut v: Vec<usize> = (0..self.entries.len())
            .filter(|&i| {
                let e = &self.entries[i];
                (self.show_hidden || !e.hidden) && (f.is_empty() || crate::fuzzy::score(&f, &e.name.to_lowercase()).is_some())
            })
            .collect();
        let s = self.sort;
        let es = &self.entries;
        v.sort_by(|&a, &b| compare(&es[a], &es[b], s));
        self.view = v;
        if let Some(p) = cur_path {
            self.cursor = self.view.iter().position(|&i| self.entries[i].path == p);
        }
        let all: std::collections::HashSet<&PathBuf> = self.entries.iter().map(|e| &e.path).collect();
        self.selected.retain(|p| all.contains(p));
    }

    pub fn set_sort(&mut self, s: Sort) {
        self.sort = s;
        self.rebuild_view();
    }

    pub fn entry(&self, pos: usize) -> Option<&Entry> {
        self.view.get(pos).map(|&i| &self.entries[i])
    }

    pub fn is_selected(&self, pos: usize) -> bool {
        self.entry(pos).map(|e| self.selected.contains(&e.path)).unwrap_or(false)
    }

    /// Click semantics like every file manager: plain = select only,
    /// ctrl = toggle, shift = range from anchor.
    pub fn click(&mut self, pos: usize, ctrl: bool, shift: bool) {
        if pos >= self.view.len() { return; }
        if shift {
            let a = self.anchor.unwrap_or(pos);
            let (lo, hi) = (a.min(pos), a.max(pos));
            if !ctrl { self.selected.clear(); }
            for p in lo..=hi { if let Some(e) = self.entry(p) { let pp = e.path.clone(); self.selected.insert(pp); } }
        } else if ctrl {
            let p = self.entries[self.view[pos]].path.clone();
            if !self.selected.remove(&p) { self.selected.insert(p); }
            self.anchor = Some(pos);
        } else {
            self.selected.clear();
            self.selected.insert(self.entries[self.view[pos]].path.clone());
            self.anchor = Some(pos);
        }
        self.cursor = Some(pos);
    }

    pub fn select_all(&mut self) {
        self.selected = self.view.iter().map(|&i| self.entries[i].path.clone()).collect();
    }
    pub fn invert_selection(&mut self) {
        let all: Vec<PathBuf> = self.view.iter().map(|&i| self.entries[i].path.clone()).collect();
        for p in all { if !self.selected.remove(&p) { self.selected.insert(p); } }
    }
    pub fn clear_selection(&mut self) {
        self.selected.clear();
    }

    /// Move the cursor by `delta` positions (clamped) and select it, optionally extending.
    pub fn move_cursor(&mut self, delta: isize, extend: bool) {
        if self.view.is_empty() { return; }
        let cur = self.cursor.unwrap_or(0) as isize;
        let n = if self.cursor.is_none() { 0 } else { (cur + delta).clamp(0, self.view.len() as isize - 1) };
        self.click(n as usize, false, extend);
    }

    /// Selected paths in display order.
    pub fn selection(&self) -> Vec<PathBuf> {
        self.view.iter().map(|&i| &self.entries[i].path).filter(|p| self.selected.contains(*p)).cloned().collect()
    }

    /// Type-to-find: jump to the next entry whose name starts with `prefix` (case-insens).
    pub fn find_prefix(&mut self, prefix: &str) -> Option<usize> {
        let p = prefix.to_lowercase();
        let start = self.cursor.unwrap_or(0);
        let n = self.view.len();
        (0..n).map(|k| (start + k) % n.max(1)).find(|&pos| self.entries[self.view[pos]].name.to_lowercase().starts_with(&p))
    }
}

pub fn human_size(b: u64) -> String {
    const U: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    if b < 1024 { return format!("{b} B"); }
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < 4 { v /= 1024.0; i += 1; }
    if v >= 100.0 { format!("{:.0} {}", v, U[i]) } else { format!("{:.1} {}", v, U[i]) }
}
