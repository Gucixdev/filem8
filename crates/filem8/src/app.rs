//! Application state and UI. No windowing code here: `App::build` is a pure
//! function of state + input, so the headless test mode drives the same code.
//!
//! Panes: the focused pane's tabs always live in `App::tabs`/`active`; the
//! second pane (dual layout) is parked in `other` and swapped in while it is
//! drawn, so all navigation code only ever deals with "the current pane".

use crate::preview_pane::PreviewPane;
use crate::settings_ui::{bind_matches, SettingsState};
use crate::thumbs::Thumbs;
use crate::views::*;
use fm8_core::class::Class;
use fm8_core::fileops::{self, History, Op};
use fm8_core::listing::{human_size, DirModel, Sort, SortKey};
use fm8_core::paths;
use fm8_core::places::{self, Bookmark};
use fm8_core::settings::{FolderMemory, FolderView, Prefs, ViewMode};
use fm8_core::watch::DirWatch;
use fm8_ui::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct Tab {
    pub uid: u64,
    pub model: DirModel,
    pub back: Vec<PathBuf>,
    pub fwd: Vec<PathBuf>,
    pub mode: ViewMode,
    pub(crate) nav: u64,
    pub(crate) last_list: Option<Id>,
    watch: Option<DirWatch>,
}

pub struct Pane {
    pub tabs: Vec<Tab>,
    pub active: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout { Single, Dual, Preview }

pub(crate) enum Place {
    Header(&'static str),
    Item { label: String, path: PathBuf, icon: Icon, sub: String, bm: Option<usize> },
}

pub(crate) enum Ctx {
    Entry(PathBuf, bool),
    Empty,
    Columns,
    Bookmark(usize),
}

#[derive(Clone, Copy)]
enum Act { Open, NewTab, AddBookmark, Rename, Copy, Cut, Paste, PasteInto, Trash, CopyOther, MoveOther, NewFolder, Reload, SelectAll, Col(usize), BmRename, BmUp, BmDown, BmRemove, Nop }

pub(crate) struct Drag { pub paths: Vec<PathBuf> }

pub struct App {
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub(crate) other: Option<Pane>,
    pub(crate) side: usize,
    pub layout: Layout,
    next_uid: u64,
    pub(crate) places: Vec<Place>,
    pub bookmarks: Vec<Bookmark>,
    pub path_edit: Option<String>,
    pub(crate) ctx: Option<(Vec2, Ctx)>,
    pub status: String,
    pub(crate) history: History,
    pub(crate) memory: FolderMemory,
    pub prefs: Prefs,
    pub(crate) cfg_dir: Option<PathBuf>,
    pub(crate) bm_file: Option<PathBuf>,
    real: bool,
    pub quit: bool,
    pub open_external: Vec<PathBuf>,
    pub(crate) find: String,
    pub(crate) find_at: f64,
    pub cols: Vec<Col>,
    pub(crate) cols_dirty: bool,
    pub(crate) owners: RefCell<HashMap<u32, String>>,
    pub(crate) thumbs: Thumbs,
    pub icons: crate::iconset::IconSet,
    pub preview: PreviewPane,
    pub clip: Option<(Vec<PathBuf>, bool)>,
    pub(crate) drag: Option<Drag>,
    pub(crate) drops: Vec<(Rect, PathBuf)>,
    pub rename: Option<(PathBuf, String)>,
    pub(crate) pending_open: bool,
    pub settings: Option<SettingsState>,
    pub(crate) confirm: Option<Vec<PathBuf>>,
    pub theme_request: Option<Theme>,
    pub(crate) bm_rename: Option<(usize, String)>,
    bm_press: Option<(usize, Vec2)>,
    bm_drag: Option<usize>,
    pub(crate) xfers: Vec<crate::xfer::Transfer>,
}

pub fn fmt_time(t: Option<std::time::SystemTime>) -> String {
    let Some(t) = t else { return String::new() };
    let Ok(d) = t.duration_since(std::time::UNIX_EPOCH) else { return String::new() };
    let secs = d.as_secs() as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs, &mut tm) };
    format!("{:04}-{:02}-{:02} {:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min)
}

pub fn icon_for(c: Class) -> Icon {
    match c {
        Class::Dir => Icon::Folder,
        Class::Image => Icon::Image,
        Class::Audio => Icon::Audio,
        Class::Video => Icon::Video,
        Class::Text | Class::Document => Icon::Text,
        Class::Code => Icon::Code,
        Class::Archive => Icon::Archive,
        Class::Pdf => Icon::Pdf,
        Class::Model3d => Icon::Cube,
        _ => Icon::File,
    }
}

fn key_name(k: SortKey) -> &'static str {
    match k { SortKey::Name => "name", SortKey::Size => "size", SortKey::Modified => "modified", SortKey::Type => "type", SortKey::Perms => "perms", SortKey::Owner => "owner", SortKey::Items => "items" }
}
fn key_parse(s: &str) -> SortKey {
    match s { "size" => SortKey::Size, "modified" => SortKey::Modified, "type" => SortKey::Type, "perms" => SortKey::Perms, "owner" => SortKey::Owner, "items" => SortKey::Items, _ => SortKey::Name }
}

fn defaults_from(p: &Prefs) -> FolderView {
    FolderView { mode: ViewMode::parse(&p.get("view")).unwrap_or(ViewMode::List), sort_key: p.get("sort"), ascending: p.flag("sort_ascending"), show_hidden: p.flag("show_hidden") }
}

impl App {
    /// `persist`: use the real config dir, GTK bookmarks, mounts and XDG dirs.
    pub fn new(start: PathBuf, persist: bool) -> App {
        if persist { App::new_in(start, Some(fm8_core::settings::config_dir()), Some(places::gtk_bookmarks_file()), true) } else { App::new_in(start, None, None, false) }
    }

    /// Explicit config locations (tests point these at temp dirs).
    pub fn new_in(start: PathBuf, cfg_dir: Option<PathBuf>, bm_file: Option<PathBuf>, real: bool) -> App {
        let prefs = cfg_dir.as_ref().map(|d| Prefs::load(&d.join("settings"))).unwrap_or_default();
        let mut memory = cfg_dir.as_ref().map(|d| FolderMemory::load(&d.join("folders.tsv"))).unwrap_or_default();
        memory.default = defaults_from(&prefs);
        let cols = cfg_dir.as_ref().and_then(|d| std::fs::read_to_string(d.join("columns.tsv")).ok()).map(|s| cols_from_text(&s)).unwrap_or_else(default_cols);
        let bookmarks = bm_file.as_ref().map(|f| places::load_bookmarks(f)).unwrap_or_default();
        let mut a = App {
            tabs: vec![], active: 0, other: None, side: 0, layout: Layout::Single, next_uid: 1, places: vec![], bookmarks, path_edit: None, ctx: None,
            status: String::new(), history: History::default(), memory, prefs, cfg_dir, bm_file, real, quit: false, open_external: vec![], find: String::new(), find_at: 0.0,
            cols, cols_dirty: false, owners: RefCell::new(HashMap::new()), thumbs: Thumbs::new(), xfers: vec![], icons: crate::iconset::IconSet::off(), preview: PreviewPane::new(), clip: None, drag: None, drops: vec![],
            rename: None, pending_open: false, settings: None, confirm: None, theme_request: None, bm_rename: None, bm_press: None, bm_drag: None,
        };
        if real { let th = a.prefs.get("icon_theme"); a.icons.configure(fm8_core::icontheme::default_search_dirs(), &th); }
        a.refresh_places();
        a.new_tab(start);
        a
    }

    pub(crate) fn refresh_places(&mut self) {
        let mut v = vec![Place::Header("PLACES")];
        v.push(Place::Item { label: "Home".into(), path: paths::home(), icon: Icon::Home, sub: String::new(), bm: None });
        if self.real {
            for b in places::user_dirs() {
                let icon = match b.label.as_str() { "Desktop" => Icon::Desktop, "Documents" => Icon::Document, "Downloads" => Icon::Download, "Music" => Icon::Music, "Pictures" => Icon::Picture, _ => Icon::Folder };
                if b.path.is_dir() { v.push(Place::Item { label: b.label, path: b.path, icon, sub: String::new(), bm: None }); }
            }
        }
        v.push(Place::Header("BOOKMARKS"));
        let shown: Vec<PathBuf> = v.iter().filter_map(|p| if let Place::Item { path, .. } = p { Some(path.clone()) } else { None }).collect();
        for (i, b) in self.bookmarks.iter().enumerate().filter(|(_, b)| !shown.contains(&b.path)) { v.push(Place::Item { label: b.label.clone(), path: b.path.clone(), icon: Icon::Star, sub: String::new(), bm: Some(i) }); }
        if self.real {
            v.push(Place::Header("DEVICES"));
            for m in places::mounts() {
                let sub = places::space(&m.path).map(|(t, f)| format!("{} free of {}", human_size(f), human_size(t))).unwrap_or_default();
                v.push(Place::Item { label: m.label, path: m.path, icon: Icon::Disk, sub, bm: None });
            }
        }
        self.places = v;
    }

    pub(crate) fn save_bookmarks(&mut self) {
        if let Some(f) = &self.bm_file { let _ = places::save_bookmarks(f, &self.bookmarks); }
        self.refresh_places();
    }

    pub fn toggle_bookmark(&mut self, dir: &Path) {
        if let Some(i) = self.bookmarks.iter().position(|b| b.path == dir) {
            self.bookmarks.remove(i);
            self.status = "Bookmark removed".into();
        } else if places::add_bookmark(&mut self.bookmarks, dir) {
            self.status = "Bookmark added".into();
        }
        self.save_bookmarks();
    }

    fn make_tab(&mut self, dir: PathBuf) -> Tab {
        let mut model = DirModel::new(dir.clone());
        let v = self.memory.get(&dir);
        model.show_hidden = v.show_hidden;
        model.sort = Sort { key: key_parse(&v.sort_key), ascending: v.ascending, dirs_first: true };
        let uid = self.next_uid;
        self.next_uid += 1;
        Tab { uid, model, back: vec![], fwd: vec![], mode: v.mode, nav: 0, last_list: None, watch: DirWatch::new(&dir).ok() }
    }

    pub fn new_tab(&mut self, dir: PathBuf) {
        let t = self.make_tab(dir);
        self.tabs.push(t);
        self.active = self.tabs.len() - 1;
    }

    pub(crate) fn list_id(&self) -> Id {
        let t = &self.tabs[self.active];
        Id::new("list").with(t.uid).with(t.nav)
    }

    fn focus_swap(&mut self) {
        if let Some(o) = self.other.as_mut() {
            std::mem::swap(&mut self.tabs, &mut o.tabs);
            std::mem::swap(&mut self.active, &mut o.active);
            self.side ^= 1;
        }
    }

    pub fn set_layout(&mut self, l: Layout) {
        if l == Layout::Dual && self.other.is_none() {
            let d = self.tabs[self.active].model.dir.clone();
            let t = self.make_tab(d);
            self.other = Some(Pane { tabs: vec![t], active: 0 });
        }
        self.layout = l;
    }

    fn cycle_layout(&mut self) {
        let n = match self.layout { Layout::Single => Layout::Dual, Layout::Dual => if self.prefs.flag("preview") { Layout::Preview } else { Layout::Single }, Layout::Preview => Layout::Single };
        self.set_layout(n);
    }

    fn other_dir(&self) -> Option<PathBuf> {
        if self.layout != Layout::Dual { return None; }
        self.other.as_ref().map(|p| p.tabs[p.active].model.dir.clone())
    }

    pub(crate) fn remember(&mut self) {
        let t = &self.tabs[self.active];
        let v = FolderView { mode: t.mode, sort_key: key_name(t.model.sort.key).into(), ascending: t.model.sort.ascending, show_hidden: t.model.show_hidden };
        let dir = t.model.dir.clone();
        self.memory.set(&dir, v);
        if let Some(d) = &self.cfg_dir { let _ = self.memory.save(&d.join("folders.tsv")); }
    }

    pub fn set_mode(&mut self, m: ViewMode) {
        self.tabs[self.active].mode = m;
        self.remember();
    }

    fn apply_view(&mut self, dir: &Path) {
        let v = self.memory.get(dir);
        let t = &mut self.tabs[self.active];
        t.mode = v.mode;
        t.model.show_hidden = v.show_hidden;
        t.model.sort = Sort { key: key_parse(&v.sort_key), ascending: v.ascending, dirs_first: true };
    }

    pub fn goto(&mut self, dir: PathBuf, record: bool) {
        let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
        if !dir.is_dir() { self.status = format!("Not a folder: {}", dir.display()); return; }
        let t = &mut self.tabs[self.active];
        if record && t.model.dir != dir { t.back.push(t.model.dir.clone()); t.fwd.clear(); }
        self.enter(dir);
        self.status.clear();
    }

    fn enter(&mut self, dir: PathBuf) {
        let t = &mut self.tabs[self.active];
        t.model.navigate(dir.clone());
        t.nav += 1;
        t.watch = DirWatch::new(&dir).ok();
        self.apply_view(&dir);
        self.rename = None;
    }

    fn go_back(&mut self) {
        let t = &mut self.tabs[self.active];
        if let Some(p) = t.back.pop() { t.fwd.push(t.model.dir.clone()); self.enter(p); }
    }
    fn go_fwd(&mut self) {
        let t = &mut self.tabs[self.active];
        if let Some(p) = t.fwd.pop() { t.back.push(t.model.dir.clone()); self.enter(p); }
    }
    fn go_up(&mut self) {
        if let Some(p) = paths::parent(&self.tabs[self.active].model.dir) { self.goto(p, true); }
    }
    fn go_back_or_up(&mut self) {
        if self.tabs[self.active].back.is_empty() { self.go_up() } else { self.go_back() }
    }

    pub(crate) fn open_path(&mut self, p: PathBuf, is_dir: bool, new_tab: bool) {
        if is_dir {
            if new_tab { self.new_tab(p); } else { self.goto(p, true); }
        } else {
            let _ = places::add_recent(&places::recents_file(), &p, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
            self.open_external.push(p);
        }
    }

    fn cursor_path(&self) -> Option<PathBuf> {
        let t = &self.tabs[self.active];
        let sel = t.model.selection();
        sel.first().cloned().or_else(|| t.model.cursor.and_then(|c| t.model.entry(c)).map(|e| e.path.clone()))
    }

    fn open_cursor(&mut self) {
        let Some(p) = self.cursor_path() else { return };
        let is_dir = p.is_dir();
        self.open_path(p, is_dir, false);
    }

    pub fn reload_all(&mut self) {
        self.tabs[self.active].model.reload();
        if let Some(o) = self.other.as_mut() { let a = o.active; o.tabs[a].model.reload(); }
    }

    fn trash_selection(&mut self) {
        let sel = self.tabs[self.active].model.selection();
        if sel.is_empty() { return; }
        if self.prefs.flag("confirm_trash") { self.confirm = Some(sel); } else { self.do_trash(sel); }
    }

    fn do_trash(&mut self, sel: Vec<PathBuf>) {
        match fileops::trash(&sel) {
            Ok(op) => { self.status = format!("Moved {} item(s) to trash (Ctrl+Z undoes)", sel.len()); self.history.push(op); self.reload_all(); }
            Err(e) => self.status = format!("Trash failed: {e}"),
        }
    }

    fn undo(&mut self) {
        self.status = match self.history.undo() { Ok(s) | Err(s) => s };
        self.reload_all();
    }

    pub fn start_rename(&mut self) {
        let Some(p) = self.cursor_path() else { return };
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.rename = Some((p, name));
    }

    pub(crate) fn finish_rename(&mut self, ok: bool, name: String) {
        let Some((p, _)) = self.rename.take() else { return };
        if !ok || name.is_empty() || p.file_name().map(|n| n.to_string_lossy() == name).unwrap_or(false) { return; }
        match fileops::rename(&p, &name) {
            Ok(op) => {
                self.history.push(op);
                let m = &mut self.tabs[self.active].model;
                m.selected.clear();
                if let Some(par) = p.parent() { m.selected.insert(par.join(&name)); }
                self.status = format!("Renamed to {name}");
                self.reload_all();
            }
            Err(e) => self.status = e,
        }
    }

    pub fn copy_selection(&mut self, cut: bool) {
        let sel = self.tabs[self.active].model.selection();
        if sel.is_empty() { return; }
        self.status = format!("{} item(s) {}", sel.len(), if cut { "cut" } else { "copied" });
        self.clip = Some((sel, cut));
    }

    pub fn paste_into(&mut self, dir: &Path) {
        let Some((paths, cut)) = self.clip.clone() else { return };
        self.transfer(&paths, dir, !cut);
        if cut { self.clip = None; }
    }

    fn transfer(&mut self, paths: &[PathBuf], dir: &Path, copy: bool) {
        let paths: Vec<PathBuf> = paths.iter().filter(|p| !dir.starts_with(p) && (copy || p.parent() != Some(dir))).cloned().collect();
        if paths.is_empty() { return; }
        self.xfers.push(crate::xfer::Transfer::start(paths, dir.to_path_buf(), copy));
    }

    /// Test hook: queue a transfer that stays held (progress panel visible).
    pub fn hold_transfer_for_test(&mut self, paths: Vec<PathBuf>, dest: PathBuf) { self.xfers.push(crate::xfer::Transfer::start_held(paths, dest, true)); }

    pub fn transfers_active(&self) -> bool { !self.xfers.is_empty() }

    fn poll_xfers(&mut self) -> bool {
        let mut fin = vec![];
        for (i, x) in self.xfers.iter().enumerate() { if let Some(r) = x.poll() { fin.push((i, r)); } }
        let any = !fin.is_empty();
        for (i, r) in fin.into_iter().rev() {
            let x = self.xfers.remove(i);
            let verb = if x.copy { "Copied" } else { "Moved" };
            match r {
                Ok(op) => {
                    let n = match &op { Op::Copy { created } => created.len(), Op::Move { pairs } => pairs.len(), _ => x.n };
                    self.history.push(op);
                    self.status = if x.cancelled() { format!("Cancelled after {n} of {} item(s)", x.n) } else { format!("{verb} {n} item(s) to {}", x.dest.display()) };
                }
                Err(e) => self.status = if x.cancelled() { "Cancelled".into() } else { e },
            }
        }
        if any { self.reload_all(); }
        any
    }

    fn transfer_panel(&mut self, ui: &mut Ui) {
        if self.xfers.is_empty() { return; }
        let t = ui.theme.clone();
        ui.request_redraw();
        let mut cancel = None;
        for (i, x) in self.xfers.iter().enumerate() {
            let w = 340.0;
            let r = rect(ui.size.x - w - 14.0, ui.size.y - 36.0 - 62.0 * (i as f32 + 1.0), w, 54.0);
            ui.draw().rounded(r, t.bg_alt, t.radius);
            ui.draw().stroke(r, t.border, t.radius, 1.0);
            let what = format!("{} {} item(s) to {}", if x.copy { "Copying" } else { "Moving" }, x.n, x.dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            ui.label(rect(r.x + 10.0, r.y + 2.0, w - 90.0, 24.0), &what, t.fg);
            let f = x.fraction();
            let bar = rect(r.x + 10.0, r.y + 34.0, w - 100.0, 8.0);
            ui.draw().rounded(bar, t.border, 4.0);
            if f > 0.0 { ui.draw().rounded(rect(bar.x, bar.y, (bar.w * f).max(8.0), bar.h), t.accent, 4.0); }
            ui.label_ex(rect(bar.right() + 6.0, r.y + 28.0, 28.0, 20.0), &format!("{}%", (f * 100.0) as u32), t.fg_dim, false, 1.0);
            if ui.button(Id::new("xfer-cancel").with(i as u64), rect(r.right() - 58.0, r.y + 6.0, 50.0, 22.0), "Stop", BtnStyle::Normal) { cancel = Some(i); }
        }
        if let Some(i) = cancel { self.xfers[i].cancel(); }
    }

    fn drop_onto(&mut self, paths: Vec<PathBuf>, target: &Path, copy: bool) {
        if target.as_os_str().is_empty() {
            let mut n = 0;
            for p in paths.iter().filter(|p| p.is_dir()) { if places::add_bookmark(&mut self.bookmarks, p) { n += 1; } }
            if n > 0 { self.save_bookmarks(); self.status = format!("{n} bookmark(s) added"); }
            return;
        }
        self.transfer(&paths, target, copy);
    }

    pub(crate) fn prefs_changed(&mut self, key: &str) {
        if let Some(d) = &self.cfg_dir { let _ = self.prefs.save(&d.join("settings")); }
        match key {
            "view" | "show_hidden" | "sort" | "sort_ascending" => {
                self.memory.default = defaults_from(&self.prefs);
                let ai = self.active;
                let dir = self.tabs[ai].model.dir.clone();
                if !self.memory.map.contains_key(&dir) { self.apply_view(&dir); self.tabs[ai].model.rebuild_view(); }
            }
            "icon_theme" => self.icons.configure(fm8_core::icontheme::default_search_dirs(), &self.prefs.get("icon_theme")),
            "preview" => if !self.prefs.flag("preview") && self.layout == Layout::Preview { self.layout = Layout::Single; },
            "theme" => {
                let t = match self.prefs.get("theme").as_str() {
                    "dark" => Theme::dark(),
                    "light" => Theme::light(),
                    _ => self.cfg_dir.as_ref().and_then(|d| Theme::load(&d.join("theme")).ok()).unwrap_or_else(Theme::dark),
                };
                self.theme_request = Some(t);
            }
            _ => {}
        }
    }

    /// Poll loaders / watchers. Returns true when a redraw is needed.
    pub fn tick(&mut self) -> bool {
        let mut ch = false;
        let active = self.active;
        for (i, t) in self.tabs.iter_mut().enumerate() { ch |= tick_tab(t, i == active); }
        if let Some(o) = self.other.as_mut() { let a = o.active; for (i, t) in o.tabs.iter_mut().enumerate() { ch |= tick_tab(t, i == a); } }
        ch |= self.poll_xfers();
        ch |= self.thumbs.poll();
        ch |= self.icons.poll();
        ch |= self.preview.poll();
        ch || self.tabs[self.active].model.loading
    }

    fn kb(&self, ui: &mut Ui, action: &str) -> bool {
        let spec = self.prefs.get(&format!("key.{action}"));
        ui.take_key(|k, m| bind_matches(&spec, k, m))
    }

    fn shortcuts(&mut self, ui: &mut Ui) {
        if self.kb(ui, "new_tab") { let d = self.tabs[self.active].model.dir.clone(); self.new_tab(d); }
        if self.kb(ui, "close_tab") && self.tabs.len() > 1 { self.tabs.remove(self.active); self.active = self.active.min(self.tabs.len() - 1); }
        if self.kb(ui, "path") || ui.take_key(|k, m| m.alt && k == Key::Char('d')) { self.path_edit = Some(self.tabs[self.active].model.dir.to_string_lossy().into_owned()); ui.focus = Some(Id::new("pathedit")); }
        if self.kb(ui, "hidden") { let m = &mut self.tabs[self.active].model; m.show_hidden = !m.show_hidden; m.rebuild_view(); self.remember(); }
        if self.kb(ui, "layout") { self.cycle_layout(); }
        if self.kb(ui, "rename") { self.start_rename(); if self.rename.is_some() { ui.focus = Some(Id::new("rename")); } }
        if self.kb(ui, "copy") { self.copy_selection(false); }
        if self.kb(ui, "cut") { self.copy_selection(true); }
        if self.kb(ui, "paste") { let d = self.tabs[self.active].model.dir.clone(); self.paste_into(&d); }
        if self.kb(ui, "bookmark") { let d = self.tabs[self.active].model.dir.clone(); self.toggle_bookmark(&d); }
        if self.kb(ui, "settings") { self.open_settings(); }
        if self.kb(ui, "reload") { self.tabs[self.active].model.reload(); }
        if ui.ctrl_key('a') { self.tabs[self.active].model.select_all(); }
        if ui.ctrl_key('z') { self.undo(); }
        if ui.ctrl_key('q') { self.quit = true; }
        if ui.ctrl_key('1') { self.set_mode(ViewMode::List); }
        if ui.ctrl_key('2') { self.set_mode(ViewMode::Grid); }
        if ui.ctrl_key('3') { self.set_mode(ViewMode::Compact); }
        if ui.take_key(|k, m| k == Key::Tab && m.ctrl) { self.active = (self.active + 1) % self.tabs.len(); }
        if self.layout == Layout::Dual && ui.take_key(|k, m| k == Key::Tab && !m.ctrl && !m.alt) { self.focus_swap(); ui.focus = Some(self.list_id()); }
        if ui.take_key(|k, m| k == Key::Left && m.alt) || ui.key_pressed(Key::Backspace) { self.go_back_or_up(); }
        if ui.take_key(|k, m| k == Key::Right && m.alt) { self.go_fwd(); }
        if ui.take_key(|k, m| k == Key::Up && m.alt) { self.go_up(); }
        if ui.key_pressed(Key::F(5)) {
            if let Some(d) = self.other_dir() { let s = self.tabs[self.active].model.selection(); self.transfer(&s, &d, true); } else { self.tabs[self.active].model.reload(); }
        }
        if ui.key_pressed(Key::F(6)) { if let Some(d) = self.other_dir() { let s = self.tabs[self.active].model.selection(); self.transfer(&s, &d, false); } }
        if ui.key_pressed(Key::Delete) { self.trash_selection(); }
        if ui.key_pressed(Key::Escape) { if self.ctx.is_some() { self.ctx = None } else { self.tabs[self.active].model.clear_selection(); } }
    }

    pub fn build(&mut self, ui: &mut Ui) {
        let t = ui.theme.clone();
        self.drops.clear();
        let full = rect(0.0, 0.0, ui.size.x, ui.size.y);
        ui.draw().rect(full, t.bg);
        let modal = self.settings.is_some() || self.confirm.is_some();
        if self.settings.is_some() { self.settings_panel(ui); }
        if self.confirm.is_some() { self.confirm_dialog(ui); }
        if modal { ui.frame.keys.clear(); }
        else if !ui.typing { self.shortcuts(ui); }
        if std::mem::take(&mut self.pending_open) { self.open_cursor(); }
        let (toolbar, rest) = full.cut_top(46.0);
        let (status, rest) = rest.cut_bottom(26.0);
        let (sidebar, main) = rest.cut_left(214.0);
        self.toolbar(ui, toolbar);
        self.sidebar(ui, sidebar);
        match self.layout {
            Layout::Single => self.pane_at(ui, main, 0, 1),
            Layout::Dual => {
                let (a, b) = ui.split(Id::new("dualsplit"), main, Axis::Horizontal, 0.5, 280.0);
                let side0 = self.side;
                let mut switch = None;
                for (p, r) in [a, b].into_iter().enumerate() {
                    let clicked = ui.frame.press.iter().any(|&x| x) && ui.hovered(r);
                    if p == side0 { self.pane(ui, r, true, p as u64); } else {
                        self.focus_swap();
                        self.pane(ui, r, false, p as u64);
                        self.focus_swap();
                        if clicked { switch = Some(p); }
                    }
                }
                if switch.is_some() { self.focus_swap(); ui.focus = Some(self.list_id()); }
            }
            Layout::Preview => {
                let (a, b) = ui.split(Id::new("pvsplit"), main, Axis::Horizontal, 0.58, 280.0);
                self.pane_at(ui, a, 0, 1);
                let target = { let tb = &self.tabs[self.active]; tb.model.cursor.and_then(|c| tb.model.entry(c)).map(|e| e.path.clone()) };
                self.preview.set(target);
                self.preview.draw(ui, b);
            }
        }
        self.status_bar(ui, status);
        self.transfer_panel(ui);
        self.context_menu(ui);
        self.finish_drag(ui);
        if self.cols_dirty && !ui.mouse_down[0] {
            self.cols_dirty = false;
            if let Some(d) = &self.cfg_dir { let _ = std::fs::create_dir_all(d); let _ = std::fs::write(d.join("columns.tsv"), cols_to_text(&self.cols)); }
        }
    }

    fn pane_at(&mut self, ui: &mut Ui, r: Rect, p: u64, _n: u64) { self.pane(ui, r, true, p); }

    fn pane(&mut self, ui: &mut Ui, r: Rect, focused: bool, pid: u64) {
        let t = ui.theme.clone();
        let (tabbar, rest) = r.cut_top(36.0);
        let titles: Vec<String> = self.tabs.iter().map(|t| t.model.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into())).collect();
        let tr = ui.tabs(Id::new("tabs").with(pid), tabbar, &titles, self.active, true);
        if let Some(i) = tr.select { self.active = i; }
        if let Some(i) = tr.close { if self.tabs.len() > 1 { self.tabs.remove(i); self.active = self.active.min(self.tabs.len() - 1); } }
        if tr.new { let d = self.tabs[self.active].model.dir.clone(); self.new_tab(d); }
        if self.layout == Layout::Dual && focused { ui.draw().rect(rect(r.x, r.y, r.w, 2.0), t.accent); }
        self.items_view(ui, rest, focused);
    }

    fn toolbar(&mut self, ui: &mut Ui, toolbar: Rect) {
        let t = ui.theme.clone();
        ui.draw().rect(toolbar, t.bg);
        let f = Size::Fixed(32.0);
        let cells = toolbar.inset_xy(8.0, 7.0).row(4.0, &[f, f, f, f, Size::Fixed(6.0), Size::Flex(1.0), Size::Fixed(6.0), f, f, f, Size::Fixed(6.0), f, f, f]);
        let (can_b, can_f) = (!self.tabs[self.active].back.is_empty(), !self.tabs[self.active].fwd.is_empty());
        if ui.icon_button(Id::new("back"), cells[0], Icon::Back, "Back (Alt+Left)", can_b, false) { self.go_back(); }
        if ui.icon_button(Id::new("fwd"), cells[1], Icon::Forward, "Forward (Alt+Right)", can_f, false) { self.go_fwd(); }
        if ui.icon_button(Id::new("up"), cells[2], Icon::Up, "Parent folder (Alt+Up)", true, false) { self.go_up(); }
        if ui.icon_button(Id::new("refresh"), cells[3], Icon::Refresh, "Reload (Ctrl+R)", true, false) { self.tabs[self.active].model.reload(); }
        self.path_bar(ui, cells[5]);
        let mode = self.tabs[self.active].mode;
        if ui.icon_button(Id::new("vlist"), cells[7], Icon::List, "Detailed list (Ctrl+1)", true, mode == ViewMode::List) { self.set_mode(ViewMode::List); }
        if ui.icon_button(Id::new("vgrid"), cells[8], Icon::Grid, "Icons and thumbnails (Ctrl+2)", true, mode == ViewMode::Grid) { self.set_mode(ViewMode::Grid); }
        if ui.icon_button(Id::new("vcompact"), cells[9], Icon::Compact, "Compact (Ctrl+3)", true, mode == ViewMode::Compact) { self.set_mode(ViewMode::Compact); }
        let (tip, ic) = match self.layout { Layout::Single => ("Layout: single (F3 for dual pane)", Icon::Split), Layout::Dual => ("Layout: dual pane (F3 for preview)", Icon::Split), Layout::Preview => ("Layout: list + preview (F3 for single)", Icon::Eye) };
        if ui.icon_button(Id::new("layout"), cells[11], ic, tip, true, self.layout != Layout::Single) { self.cycle_layout(); }
        let hidden = self.tabs[self.active].model.show_hidden;
        if ui.icon_button(Id::new("hidden"), cells[12], Icon::Search, "Show hidden files (Ctrl+H)", true, hidden) { let m = &mut self.tabs[self.active].model; m.show_hidden = !m.show_hidden; m.rebuild_view(); self.remember(); }
        if ui.icon_button(Id::new("settings"), cells[13], Icon::Menu, "Settings (Ctrl+,)", true, self.settings.is_some()) { self.open_settings(); }
        ui.separator(rect(toolbar.x, toolbar.bottom() - 1.0, toolbar.w, 1.0));
    }

    fn path_bar(&mut self, ui: &mut Ui, r: Rect) {
        let t = ui.theme.clone();
        let id = Id::new("pathedit");
        if let Some(mut s) = self.path_edit.take() {
            if ui.take_key(|k, _| k == Key::Tab) {
                if let Some(c) = paths::complete(&s, 1).into_iter().next() { s = format!("{c}/"); }
            }
            let resp = ui.text_input(id, r, &mut s, "Type a path");
            if ui.focus != Some(id) && !resp.submitted { return; }
            if ui.focus.is_none() { ui.focus = Some(id); }
            if resp.submitted { let p = paths::expand(&s); self.goto(p, true); ui.focus = None; }
            else if resp.cancelled { ui.focus = None; }
            else { self.path_edit = Some(s); }
            // completion hints
            if let Some(s) = &self.path_edit {
                let sug = paths::complete(s, 6);
                if !sug.is_empty() && s.len() > 0 {
                    ui.begin_overlay();
                    let h = sug.len() as f32 * 26.0 + 8.0;
                    let pr = rect(r.x, r.bottom() + 4.0, r.w, h);
                    ui.draw().rounded(pr, t.bg_alt, 8.0);
                    ui.draw().stroke(pr, t.border, 8.0, 1.0);
                    for (i, p) in sug.iter().enumerate() { ui.label(rect(pr.x + 10.0, pr.y + 4.0 + i as f32 * 26.0, pr.w - 20.0, 26.0), p, t.fg_dim); }
                    ui.draw().text(pr.x + 10.0, pr.bottom() + 4.0, "Tab completes, Enter goes, Esc cancels", 11.0, t.fg_dim, false);
                    ui.end_overlay();
                }
            }
            return;
        }
        ui.draw().rounded(r, t.bg_alt, t.radius);
        let crumbs = paths::breadcrumbs(&self.tabs[self.active].model.dir);
        let mut x = r.x + 6.0;
        let last = crumbs.len() - 1;
        let mut go: Option<PathBuf> = None;
        // when too long, drop leading crumbs
        let widths: Vec<f32> = crumbs.iter().enumerate().map(|(i, (l, _))| fm8_ui::text::measure(l, t.font_size, i == last) + 18.0 + 12.0).collect();
        let total: f32 = widths.iter().sum();
        let mut skip = 0;
        let mut acc = total;
        while acc > r.w - 20.0 && skip < last { acc -= widths[skip]; skip += 1; }
        if skip > 0 { ui.label(rect(x, r.y, 14.0, r.h), "…", t.fg_dim); x += 18.0; }
        for (i, (label, path)) in crumbs.iter().enumerate().skip(skip) {
            let w = widths[i] - 12.0;
            let br = rect(x, r.y + 4.0, w, r.h - 8.0);
            let hov = ui.hovered(br);
            if hov { ui.draw().rounded(br, t.pressed(), 5.0); }
            ui.label_ex(br, label, if i == last { t.fg } else if hov { t.link } else { t.fg_dim }, i == last, 0.5);
            let (clicked, _, _) = ui.interact(Id::new("crumb").with(i as u64), br);
            if clicked { go = Some(path.clone()); }
            x += w;
            if i != last { ui.draw().text(x + 1.0, r.y + (r.h - 14.0) / 2.0, "›", 14.0, t.fg_dim, false); x += 12.0; }
        }
        // click on the empty part edits the path
        let empty = rect(x, r.y, (r.right() - x).max(0.0), r.h);
        if ui.hovered(empty) && ui.frame.press[0] { self.path_edit = Some(self.tabs[self.active].model.dir.to_string_lossy().into_owned()); ui.focus = Some(id); }
        if let Some(p) = go { self.goto(p, true); }
    }

    fn sidebar(&mut self, ui: &mut Ui, r: Rect) {
        let t = ui.theme.clone();
        ui.draw().rect(r, t.bg_alt);
        ui.draw().rect(rect(r.right() - 1.0, r.y, 1.0, r.h), t.border);
        let cur = self.tabs[self.active].model.dir.clone();
        let dragging = self.drag.is_some();
        if dragging { self.drops.push((r, PathBuf::new())); }
        let mut y = r.y + 8.0;
        let mut go: Option<(PathBuf, bool)> = None;
        let mut bm_rects: Vec<(usize, Rect)> = vec![];
        let mut ctx: Option<Ctx> = None;
        let mut rename_res: Option<(bool, String)> = None;
        for (n, p) in self.places.iter().enumerate() {
            match p {
                Place::Header(h) => {
                    y += 8.0;
                    ui.draw().text(r.x + 16.0, y, h, 11.0, t.fg_dim, true);
                    if *h == "BOOKMARKS" && self.bookmarks.is_empty() { ui.draw().text(r.x + 16.0, y + 20.0, "Ctrl+D adds this folder", 11.0, t.fg_dim, false); y += 22.0; }
                    y += 20.0;
                }
                Place::Item { label, path, icon, sub, bm } => {
                    let h = if sub.is_empty() { 32.0 } else { 44.0 };
                    let ir = rect(r.x + 8.0, y, r.w - 16.0, h);
                    let id = Id::new("place").with(n as u64);
                    let hov = ui.hovered(ir);
                    let active = *path == cur;
                    if active { ui.draw().rounded(ir, t.selection_bg, t.radius); } else if hov { ui.draw().rounded(ir, t.hover(), t.radius); }
                    let fg = if active { t.selection_fg } else { t.fg };
                    let ar = rect(ir.x + 10.0, ir.y + (32.0 - 18.0) / 2.0, 18.0, 18.0);
                    let themed = self.icons.for_icon(*icon, ar.w);
                    if !self.icons.draw(ui, themed, ar) { ui.icon2(*icon, ar, if active { fg } else { t.fg_dim }, t.bg_alt); }
                    let lr = rect(ir.x + 38.0, ir.y, ir.w - 44.0, 32.0);
                    let renaming = matches!((&self.bm_rename, bm), (Some((i, _)), Some(b)) if i == b);
                    if renaming {
                        if let Some((_, buf)) = self.bm_rename.as_mut() {
                            let rid = Id::new("bmrename");
                            let resp = ui.text_input(rid, lr.inset_xy(-4.0, 2.0), buf, "");
                            if resp.submitted { rename_res = Some((true, buf.clone())); } else if resp.cancelled || ui.focus != Some(rid) { rename_res = Some((false, String::new())); }
                        }
                    } else {
                        ui.label(lr, label, fg);
                        if !sub.is_empty() { ui.draw().text(ir.x + 38.0, ir.y + 26.0, &fm8_ui::text::ellipsize(sub, 11.0, false, ir.w - 44.0), 11.0, t.fg_dim, false); }
                        let (clicked, _, _) = ui.interact(id, ir);
                        if clicked { go = Some((path.clone(), false)); }
                    }
                    if hov && ui.frame.press[2] { go = Some((path.clone(), true)); }
                    if let Some(b) = bm {
                        bm_rects.push((*b, ir));
                        if hov && ui.frame.press[0] && !renaming { self.bm_press = Some((*b, ui.mouse)); }
                        if hov && ui.frame.press[1] { ctx = Some(Ctx::Bookmark(*b)); }
                    }
                    if dragging { self.drops.push((ir, path.clone())); }
                    y += h + 2.0;
                }
            }
        }
        // bookmark reorder by dragging
        if let Some((b, p0)) = self.bm_press {
            if ui.mouse_down[0] {
                if self.bm_drag.is_none() && (ui.mouse.x - p0.x).hypot(ui.mouse.y - p0.y) > 6.0 { self.bm_drag = Some(b); }
                if self.bm_drag.is_some() {
                    ui.cursor = Cursor::Pointer;
                    if let Some((to, rr)) = bm_rects.iter().find(|(_, rr)| rr.contains(ui.mouse)) {
                        let ly = if *to > b { rr.bottom() } else { rr.y };
                        ui.draw().rect(rect(rr.x, ly - 1.0, rr.w, 2.0), t.accent);
                    }
                }
            } else {
                if let Some(src) = self.bm_drag {
                    if let Some((to, _)) = bm_rects.iter().find(|(_, rr)| rr.contains(ui.mouse)) {
                        if *to != src { let b = self.bookmarks.remove(src); self.bookmarks.insert(*to, b); self.save_bookmarks(); }
                    }
                    go = None;
                }
                self.bm_press = None;
                self.bm_drag = None;
            }
        }
        if let Some((ok, name)) = rename_res {
            if let Some((i, _)) = self.bm_rename.take() { if ok && !name.is_empty() && i < self.bookmarks.len() { self.bookmarks[i].label = name; self.save_bookmarks(); } }
            ui.focus = None;
        }
        if let Some(c) = ctx { self.ctx = Some((ui.mouse, c)); }
        if self.bm_drag.is_none() { if let Some((p, nt)) = go { if nt { self.new_tab(p) } else { self.goto(p, true) } } }
    }

    fn status_bar(&mut self, ui: &mut Ui, r: Rect) {
        let t = ui.theme.clone();
        ui.draw().rect(r, t.bg_alt);
        ui.separator(rect(r.x, r.y, r.w, 1.0));
        let m = &self.tabs[self.active].model;
        let sel = m.selection();
        let mut s = format!("{} items", m.view.len());
        if !sel.is_empty() {
            let bytes: u64 = m.entries.iter().filter(|e| m.selected.contains(&e.path)).map(|e| e.size).sum();
            s = format!("{} of {} selected, {}", sel.len(), m.view.len(), human_size(bytes));
        }
        if m.loading { s.push_str("  (loading...)"); }
        ui.label(rect(r.x + 14.0, r.y, r.w * 0.4, r.h), &s, t.fg_dim);
        let hint = if self.layout == Layout::Dual { "Tab other pane   F5 copy   F6 move   F2 rename   Del trash" } else { "F2 rename   Ctrl+C/X/V   F3 layout   Ctrl+D bookmark   Ctrl+, settings" };
        let status = self.status.clone();
        let txt = if status.is_empty() { hint } else { &status };
        let mut right = r.w * 0.6 - 14.0;
        if self.tabs[self.active].mode == ViewMode::Grid {
            // tile size slider (56..256 px icon edge)
            let sr = rect(r.right() - 134.0, r.y + 4.0, 120.0, r.h - 8.0);
            let cur: f32 = crate::views::tile_dims(&self.prefs.get("thumb_size")).0 - 24.0;
            let id = Id::new("tile-slider");
            let ty = sr.y + sr.h / 2.0;
            ui.draw().rounded(rect(sr.x, ty - 1.5, sr.w, 3.0), t.border, 1.5);
            let f = (cur - 56.0) / 200.0;
            if let Some(nf) = ui.drag_frac(id, sr) {
                let px = (56.0 + nf * 200.0).round() as u32;
                if px as f32 != cur { self.prefs.set("thumb_size", &px.to_string()); self.prefs_changed("thumb_size"); }
            }
            let kx = sr.x + sr.w * f.clamp(0.0, 1.0);
            ui.draw().rounded(rect(kx - 6.0, ty - 6.0, 12.0, 12.0), t.accent, 6.0);
            right -= 140.0;
        }
        ui.label_ex(rect(r.x + r.w * 0.4, r.y, right, r.h), txt, if self.status.is_empty() { t.fg_dim } else { t.warn }, false, 1.0);
    }

    fn confirm_dialog(&mut self, ui: &mut Ui) {
        let Some(paths) = self.confirm.take() else { return };
        let t = ui.theme.clone();
        let body = ui.dialog_frame(440.0, 170.0, "Move to trash?");
        ui.begin_overlay();
        ui.label(rect(body.x, body.y, body.w, 28.0), &format!("{} item(s) will be moved to the trash.", paths.len()), t.fg);
        ui.draw().text(body.x, body.y + 30.0, "You can undo this with Ctrl+Z.", 12.0, t.fg_dim, false);
        let by = body.bottom() - 40.0;
        let yes = ui.button(Id::new("cf-yes"), rect(body.right() - 110.0, by, 110.0, 34.0), "Move to trash", BtnStyle::Primary);
        let no = ui.button(Id::new("cf-no"), rect(body.right() - 200.0, by, 80.0, 34.0), "Cancel", BtnStyle::Normal);
        ui.end_overlay();
        if yes || ui.take_key(|k, _| k == Key::Enter) { self.do_trash(paths); }
        else if no || ui.take_key(|k, _| k == Key::Escape) {}
        else { self.confirm = Some(paths); }
    }

    fn finish_drag(&mut self, ui: &mut Ui) {
        let Some(d) = &self.drag else { return };
        let t = ui.theme.clone();
        let n = d.paths.len();
        let target = self.drops.iter().rev().find(|(r, _)| r.contains(ui.mouse)).cloned();
        if let Some((r, _)) = &target { ui.draw().stroke(r.inset(-1.0), t.accent, t.radius, 2.0); }
        let copy = ui.mods.ctrl;
        let label = match &target { Some((_, p)) if p.as_os_str().is_empty() => format!("Add {n} bookmark(s)"), _ => format!("{} {} item(s)", if copy { "Copy" } else { "Move" }, n) };
        let w = fm8_ui::text::measure(&label, t.font_size, false) + 24.0;
        let gr = rect(ui.mouse.x + 14.0, ui.mouse.y + 14.0, w, 28.0);
        ui.draw().cmds.push(Cmd::Shadow { r: gr, radius: 8.0, blur: 10.0, color: rgba(0, 0, 0, 90) });
        ui.draw().rounded(gr, t.accent, 8.0);
        ui.label_ex(gr, &label, t.accent_fg, false, 0.5);
        ui.request_redraw();
        if !ui.mouse_down[0] || ui.frame.release[0] {
            let d = self.drag.take().unwrap();
            if let Some((_, p)) = target { self.drop_onto(d.paths, &p, copy); }
        }
    }

    fn context_menu(&mut self, ui: &mut Ui) {
        let Some((pos, c)) = self.ctx.take() else { return };
        let other = self.layout == Layout::Dual;
        let mut items: Vec<(MenuItem, Act)> = vec![];
        let paste_ok = self.clip.is_some();
        match &c {
            Ctx::Entry(_, is_dir) => {
                items.push((MenuItem::new("Open", "Enter"), Act::Open));
                items.push((if *is_dir { MenuItem::new("Open in new tab", "Middle click") } else { MenuItem::new("Open in new tab", "").disabled() }, Act::NewTab));
                items.push((if *is_dir { MenuItem::new("Add to bookmarks", "") } else { MenuItem::new("Add to bookmarks", "").disabled() }, Act::AddBookmark));
                items.push((MenuItem::sep(), Act::Nop));
                items.push((MenuItem::new("Rename", "F2"), Act::Rename));
                items.push((MenuItem::new("Copy", "Ctrl+C"), Act::Copy));
                items.push((MenuItem::new("Cut", "Ctrl+X"), Act::Cut));
                if *is_dir { items.push((if paste_ok { MenuItem::new("Paste into folder", "") } else { MenuItem::new("Paste into folder", "").disabled() }, Act::PasteInto)); }
                if other {
                    items.push((MenuItem::new("Copy to other pane", "F5"), Act::CopyOther));
                    items.push((MenuItem::new("Move to other pane", "F6"), Act::MoveOther));
                }
                items.push((MenuItem::sep(), Act::Nop));
                items.push((MenuItem::new("Move to trash", "Del"), Act::Trash));
            }
            Ctx::Empty => {
                items.push((MenuItem::new("New folder", ""), Act::NewFolder));
                items.push((if paste_ok { MenuItem::new("Paste", "Ctrl+V") } else { MenuItem::new("Paste", "Ctrl+V").disabled() }, Act::Paste));
                items.push((MenuItem::new("Reload", "Ctrl+R"), Act::Reload));
                items.push((MenuItem::new("Select all", "Ctrl+A"), Act::SelectAll));
            }
            Ctx::Columns => {
                for (i, c) in self.cols.iter().enumerate() { items.push((MenuItem::new(&format!("[{}] {}", if c.on { "x" } else { " " }, c.id.label()), ""), Act::Col(i))); }
            }
            Ctx::Bookmark(i) => {
                items.push((MenuItem::new("Open", ""), Act::Open));
                items.push((MenuItem::new("Open in new tab", ""), Act::NewTab));
                items.push((MenuItem::new("Rename", ""), Act::BmRename));
                items.push((if *i > 0 { MenuItem::new("Move up", "") } else { MenuItem::new("Move up", "").disabled() }, Act::BmUp));
                items.push((if i + 1 < self.bookmarks.len() { MenuItem::new("Move down", "") } else { MenuItem::new("Move down", "").disabled() }, Act::BmDown));
                items.push((MenuItem::new("Remove bookmark", ""), Act::BmRemove));
            }
        }
        let mi: Vec<MenuItem> = items.iter().map(|(m, _)| m.clone()).collect();
        match ui.menu(Id::new("ctx"), pos, &mi) {
            MenuResp::Open => self.ctx = Some((pos, c)),
            MenuResp::Dismiss => {}
            MenuResp::Chosen(i) => {
                let dir = self.tabs[self.active].model.dir.clone();
                let path = match &c { Ctx::Entry(p, _) => Some(p.clone()), Ctx::Bookmark(b) => self.bookmarks.get(*b).map(|b| b.path.clone()), _ => None };
                match items[i].1 {
                    Act::Open => if let Some(p) = path { let d = p.is_dir(); self.open_path(p, d, false); },
                    Act::NewTab => if let Some(p) = path { self.new_tab(p); },
                    Act::AddBookmark => if let Some(p) = path { if places::add_bookmark(&mut self.bookmarks, &p) { self.save_bookmarks(); self.status = "Bookmark added".into(); } },
                    Act::Rename => { self.start_rename(); if self.rename.is_some() { ui.focus = Some(Id::new("rename")); } }
                    Act::Copy => self.copy_selection(false),
                    Act::Cut => self.copy_selection(true),
                    Act::Paste => self.paste_into(&dir),
                    Act::PasteInto => if let Some(p) = path { self.paste_into(&p); },
                    Act::Trash => self.trash_selection(),
                    Act::CopyOther => if let Some(d) = self.other_dir() { let s = self.tabs[self.active].model.selection(); self.transfer(&s, &d, true); },
                    Act::MoveOther => if let Some(d) = self.other_dir() { let s = self.tabs[self.active].model.selection(); self.transfer(&s, &d, false); },
                    Act::NewFolder => match fileops::new_dir(&dir, &unique_folder_name(&dir)) { Ok(op) => { self.history.push(op); self.tabs[self.active].model.reload(); } Err(e) => self.status = e },
                    Act::Reload => self.tabs[self.active].model.reload(),
                    Act::SelectAll => self.tabs[self.active].model.select_all(),
                    Act::Col(k) => { self.cols[k].on = !self.cols[k].on; self.cols_dirty = true; }
                    Act::BmRename => if let Ctx::Bookmark(b) = c { self.bm_rename = Some((b, self.bookmarks[b].label.clone())); ui.focus = Some(Id::new("bmrename")); },
                    Act::BmUp => if let Ctx::Bookmark(b) = c { if b > 0 { self.bookmarks.swap(b, b - 1); self.save_bookmarks(); } },
                    Act::BmDown => if let Ctx::Bookmark(b) = c { if b + 1 < self.bookmarks.len() { self.bookmarks.swap(b, b + 1); self.save_bookmarks(); } },
                    Act::BmRemove => if let Ctx::Bookmark(b) = c { if b < self.bookmarks.len() { self.bookmarks.remove(b); self.save_bookmarks(); self.status = "Bookmark removed".into(); } },
                    Act::Nop => {}
                }
            }
        }
    }
}

fn tick_tab(t: &mut Tab, active: bool) -> bool {
    let mut ch = t.model.poll();
    if active && t.watch.as_ref().map(|w| w.changed()).unwrap_or(false) && !t.model.loading {
        t.model.reload();
        ch = true;
    }
    ch
}

fn unique_folder_name(dir: &Path) -> String {
    let mut n = 1;
    loop {
        let name = if n == 1 { "New folder".to_string() } else { format!("New folder {n}") };
        if !dir.join(&name).exists() { return name; }
        n += 1;
    }
}

impl App {
    /// Test hook: start renaming bookmark `i` (what the context menu does).
    pub fn bm_rename_for_test(&mut self, i: usize) { self.bm_rename = Some((i, self.bookmarks[i].label.clone())); }
}
