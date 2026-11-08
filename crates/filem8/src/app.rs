//! Application state and UI. No windowing code here: `App::build` is a pure
//! function of state + input, so the headless test mode drives the same code.

use fm8_core::class::Class;
use fm8_core::fileops::{self, History};
use fm8_core::listing::{human_size, DirModel, Sort, SortKey};
use fm8_core::places::{self, Bookmark};
use fm8_core::settings::{FolderMemory, FolderView};
use fm8_core::watch::DirWatch;
use fm8_core::paths;
use fm8_ui::*;
use std::path::{Path, PathBuf};

pub struct Tab {
    pub uid: u64,
    pub model: DirModel,
    pub back: Vec<PathBuf>,
    pub fwd: Vec<PathBuf>,
    nav: u64,
    watch: Option<DirWatch>,
}

enum Place {
    Header(&'static str),
    Item { label: String, path: PathBuf, icon: Icon, sub: String },
}

enum Ctx {
    Entry(PathBuf, bool),
    Empty,
}

pub struct App {
    pub tabs: Vec<Tab>,
    pub active: usize,
    next_uid: u64,
    places: Vec<Place>,
    bookmarks: Vec<Bookmark>,
    pub path_edit: Option<String>,
    ctx: Option<(Vec2, Ctx)>,
    pub status: String,
    history: History,
    memory: FolderMemory,
    mem_file: Option<PathBuf>,
    pub quit: bool,
    pub open_external: Vec<PathBuf>,
    find: String,
    find_at: f64,
    last_list: Option<Id>,
}

fn key_name(k: SortKey) -> &'static str {
    match k { SortKey::Name => "name", SortKey::Size => "size", SortKey::Modified => "modified", SortKey::Type => "type" }
}
fn key_parse(s: &str) -> SortKey {
    match s { "size" => SortKey::Size, "modified" => SortKey::Modified, "type" => SortKey::Type, _ => SortKey::Name }
}

pub fn fmt_time(t: Option<std::time::SystemTime>) -> String {
    let Some(t) = t else { return String::new() };
    let Ok(d) = t.duration_since(std::time::UNIX_EPOCH) else { return String::new() };
    let secs = d.as_secs() as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs, &mut tm) };
    format!("{:04}-{:02}-{:02} {:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min)
}

fn icon_for(c: Class) -> Icon {
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

impl App {
    /// `persist`: read/write bookmarks and per-folder view memory in the real config dir.
    pub fn new(start: PathBuf, persist: bool) -> App {
        let mem_file = persist.then(|| fm8_core::settings::config_dir().join("folders.tsv"));
        let memory = mem_file.as_ref().map(|f| FolderMemory::load(f)).unwrap_or_default();
        let bookmarks = if persist { places::load_bookmarks(&places::gtk_bookmarks_file()) } else { vec![] };
        let mut a = App { tabs: vec![], active: 0, next_uid: 1, places: vec![], bookmarks, path_edit: None, ctx: None, status: String::new(), history: History::default(), memory, mem_file, quit: false, open_external: vec![], find: String::new(), find_at: 0.0, last_list: None };
        a.refresh_places(persist);
        a.new_tab(start);
        a
    }

    fn refresh_places(&mut self, real: bool) {
        let mut v = vec![Place::Header("PLACES")];
        v.push(Place::Item { label: "Home".into(), path: paths::home(), icon: Icon::Home, sub: String::new() });
        if real {
            for b in places::user_dirs() {
                let icon = match b.label.as_str() { "Desktop" => Icon::Desktop, "Documents" => Icon::Document, "Downloads" => Icon::Download, "Music" => Icon::Music, "Pictures" => Icon::Picture, _ => Icon::Folder };
                if b.path.is_dir() { v.push(Place::Item { label: b.label, path: b.path, icon, sub: String::new() }); }
            }
        }
        if !self.bookmarks.is_empty() {
            v.push(Place::Header("BOOKMARKS"));
            for b in &self.bookmarks { v.push(Place::Item { label: b.label.clone(), path: b.path.clone(), icon: Icon::Star, sub: String::new() }); }
        }
        if real {
            v.push(Place::Header("DEVICES"));
            for m in places::mounts() {
                let sub = places::space(&m.path).map(|(t, f)| format!("{} free of {}", human_size(f), human_size(t))).unwrap_or_default();
                v.push(Place::Item { label: m.label, path: m.path, icon: Icon::Disk, sub });
            }
        }
        self.places = v;
    }

    pub fn new_tab(&mut self, dir: PathBuf) {
        let mut model = DirModel::new(dir.clone());
        self.apply_memory(&mut model);
        let uid = self.next_uid;
        self.next_uid += 1;
        let watch = DirWatch::new(&dir).ok();
        self.tabs.push(Tab { uid, model, back: vec![], fwd: vec![], nav: 0, watch });
        self.active = self.tabs.len() - 1;
    }

    fn apply_memory(&self, m: &mut DirModel) {
        let v = self.memory.get(&m.dir);
        m.show_hidden = v.show_hidden;
        m.sort = Sort { key: key_parse(&v.sort_key), ascending: v.ascending, dirs_first: true };
    }

    fn remember(&mut self) {
        let t = &self.tabs[self.active];
        let v = FolderView { mode: fm8_core::settings::ViewMode::List, sort_key: key_name(t.model.sort.key).into(), ascending: t.model.sort.ascending, show_hidden: t.model.show_hidden };
        let dir = t.model.dir.clone();
        self.memory.set(&dir, v);
        if let Some(f) = &self.mem_file { let _ = self.memory.save(f); }
    }

    pub fn goto(&mut self, dir: PathBuf, record: bool) {
        let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
        if !dir.is_dir() { self.status = format!("Not a folder: {}", dir.display()); return; }
        let mem = self.memory.get(&dir);
        let t = &mut self.tabs[self.active];
        if record && t.model.dir != dir { t.back.push(t.model.dir.clone()); t.fwd.clear(); }
        t.model.navigate(dir.clone());
        t.model.show_hidden = mem.show_hidden;
        t.model.sort = Sort { key: key_parse(&mem.sort_key), ascending: mem.ascending, dirs_first: true };
        t.nav += 1;
        t.watch = DirWatch::new(&dir).ok();
        self.status.clear();
    }

    fn go_back(&mut self) {
        let t = &mut self.tabs[self.active];
        if let Some(p) = t.back.pop() { t.fwd.push(t.model.dir.clone()); self.goto_raw(p); }
    }
    fn go_fwd(&mut self) {
        let t = &mut self.tabs[self.active];
        if let Some(p) = t.fwd.pop() { t.back.push(t.model.dir.clone()); self.goto_raw(p); }
    }
    fn goto_raw(&mut self, p: PathBuf) {
        let mem = self.memory.get(&p);
        let t = &mut self.tabs[self.active];
        t.model.navigate(p.clone());
        t.model.show_hidden = mem.show_hidden;
        t.nav += 1;
        t.watch = DirWatch::new(&p).ok();
    }
    fn go_up(&mut self) {
        if let Some(p) = paths::parent(&self.tabs[self.active].model.dir) { self.goto(p, true); }
    }

    fn open_path(&mut self, p: PathBuf, is_dir: bool, new_tab: bool) {
        if is_dir {
            if new_tab { self.new_tab(p); } else { self.goto(p, true); }
        } else {
            let _ = places::add_recent(&places::recents_file(), &p, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0));
            self.open_external.push(p);
        }
    }

    fn open_cursor(&mut self) {
        let t = &self.tabs[self.active];
        let sel = t.model.selection();
        let Some(p) = sel.first().cloned().or_else(|| t.model.cursor.and_then(|c| t.model.entry(c)).map(|e| e.path.clone())) else { return };
        let is_dir = p.is_dir();
        self.open_path(p, is_dir, false);
    }

    fn trash_selection(&mut self) {
        let sel = self.tabs[self.active].model.selection();
        if sel.is_empty() { return; }
        match fileops::trash(&sel) {
            Ok(op) => { self.status = format!("Moved {} item(s) to trash (Ctrl+Z undoes)", sel.len()); self.history.push(op); self.tabs[self.active].model.reload(); }
            Err(e) => self.status = format!("Trash failed: {e}"),
        }
    }

    fn undo(&mut self) {
        self.status = match self.history.undo() { Ok(s) | Err(s) => s };
        self.tabs[self.active].model.reload();
    }

    /// Poll loaders / watchers. Returns true when a redraw is needed.
    pub fn tick(&mut self) -> bool {
        let mut ch = false;
        for (i, t) in self.tabs.iter_mut().enumerate() {
            ch |= t.model.poll();
            if i == self.active && t.watch.as_ref().map(|w| w.changed()).unwrap_or(false) && !t.model.loading {
                t.model.reload();
                ch = true;
            }
        }
        ch || self.tabs[self.active].model.loading
    }

    pub fn build(&mut self, ui: &mut Ui) {
        let t = ui.theme.clone();
        let full = rect(0.0, 0.0, ui.size.x, ui.size.y);
        ui.draw().rect(full, t.bg);
        let typing = ui.typing;
        // ---- global shortcuts
        if !typing {
            if ui.ctrl_key('t') { let d = self.tabs[self.active].model.dir.clone(); self.new_tab(d); }
            if ui.ctrl_key('w') && self.tabs.len() > 1 { self.tabs.remove(self.active); self.active = self.active.min(self.tabs.len() - 1); }
            if ui.ctrl_key('l') || ui.take_key(|k, m| m.alt && k == Key::Char('d')) { self.path_edit = Some(self.tabs[self.active].model.dir.to_string_lossy().into_owned()); ui.focus = Some(Id::new("pathedit")); }
            if ui.ctrl_key('h') { let m = &mut self.tabs[self.active].model; m.show_hidden = !m.show_hidden; m.rebuild_view(); self.remember(); }
            if ui.ctrl_key('a') { self.tabs[self.active].model.select_all(); }
            if ui.ctrl_key('z') { self.undo(); }
            if ui.ctrl_key('q') { self.quit = true; }
            if ui.take_key(|k, m| k == Key::Tab && m.ctrl) { self.active = (self.active + 1) % self.tabs.len(); }
            if ui.take_key(|k, m| k == Key::Left && m.alt) || ui.key_pressed(Key::Backspace) { self.go_back_or_up(ui); }
            if ui.take_key(|k, m| k == Key::Right && m.alt) { self.go_fwd(); }
            if ui.take_key(|k, m| k == Key::Up && m.alt) { self.go_up(); }
            if ui.key_pressed(Key::F(5)) { self.tabs[self.active].model.reload(); }
            if ui.key_pressed(Key::Delete) { self.trash_selection(); }
            if ui.key_pressed(Key::Escape) { if self.ctx.is_some() { self.ctx = None } else { self.tabs[self.active].model.clear_selection(); } }
        }
        let (tabbar, rest) = full.cut_top(38.0);
        let (toolbar, rest) = rest.cut_top(46.0);
        let (status, rest) = rest.cut_bottom(26.0);
        let (sidebar, main) = rest.cut_left(214.0);
        // ---- tabs
        let titles: Vec<String> = self.tabs.iter().map(|t| t.model.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into())).collect();
        let tr = ui.tabs(Id::new("tabs"), tabbar, &titles, self.active, true);
        if let Some(i) = tr.select { self.active = i; }
        if let Some(i) = tr.close { if self.tabs.len() > 1 { self.tabs.remove(i); self.active = self.active.min(self.tabs.len() - 1); } }
        if tr.new { let d = self.tabs[self.active].model.dir.clone(); self.new_tab(d); }
        // ---- toolbar
        ui.draw().rect(toolbar, t.bg);
        let cells = toolbar.inset_xy(8.0, 7.0).row(4.0, &[Size::Fixed(32.0), Size::Fixed(32.0), Size::Fixed(32.0), Size::Fixed(32.0), Size::Fixed(6.0), Size::Flex(1.0), Size::Fixed(32.0)]);
        let (can_b, can_f) = (!self.tabs[self.active].back.is_empty(), !self.tabs[self.active].fwd.is_empty());
        if ui.icon_button(Id::new("back"), cells[0], Icon::Back, "Back (Alt+Left)", can_b, false) { self.go_back(); }
        if ui.icon_button(Id::new("fwd"), cells[1], Icon::Forward, "Forward (Alt+Right)", can_f, false) { self.go_fwd(); }
        if ui.icon_button(Id::new("up"), cells[2], Icon::Up, "Parent folder (Alt+Up)", true, false) { self.go_up(); }
        if ui.icon_button(Id::new("refresh"), cells[3], Icon::Refresh, "Reload (F5)", true, false) { self.tabs[self.active].model.reload(); }
        self.path_bar(ui, cells[5]);
        let hidden = self.tabs[self.active].model.show_hidden;
        if ui.icon_button(Id::new("hidden"), cells[6], Icon::Eye, "Show hidden files (Ctrl+H)", true, hidden) { let m = &mut self.tabs[self.active].model; m.show_hidden = !m.show_hidden; m.rebuild_view(); self.remember(); }
        ui.separator(rect(toolbar.x, toolbar.bottom() - 1.0, toolbar.w, 1.0));
        // ---- sidebar, list, status
        self.sidebar(ui, sidebar);
        self.list(ui, main);
        self.status_bar(ui, status);
        self.context_menu(ui);
    }

    fn go_back_or_up(&mut self, _ui: &mut Ui) {
        if self.tabs[self.active].back.is_empty() { self.go_up() } else { self.go_back() }
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
        let mut y = r.y + 8.0;
        let mut go: Option<(PathBuf, bool)> = None;
        let mut rm: Option<PathBuf> = None;
        for (n, p) in self.places.iter().enumerate() {
            match p {
                Place::Header(h) => { y += 8.0; ui.draw().text(r.x + 16.0, y, h, 11.0, t.fg_dim, true); y += 20.0; }
                Place::Item { label, path, icon, sub } => {
                    let h = if sub.is_empty() { 32.0 } else { 44.0 };
                    let ir = rect(r.x + 8.0, y, r.w - 16.0, h);
                    let id = Id::new("place").with(n as u64);
                    let hov = ui.hovered(ir);
                    let active = *path == cur;
                    if active { ui.draw().rounded(ir, t.selection_bg, t.radius); } else if hov { ui.draw().rounded(ir, t.hover(), t.radius); }
                    let fg = if active { t.selection_fg } else { t.fg };
                    ui.icon2(*icon, rect(ir.x + 10.0, ir.y + (32.0 - 18.0) / 2.0, 18.0, 18.0), if active { fg } else { t.fg_dim }, t.bg_alt);
                    ui.label(rect(ir.x + 38.0, ir.y, ir.w - 44.0, 32.0), label, fg);
                    if !sub.is_empty() { ui.draw().text(ir.x + 38.0, ir.y + 26.0, &fm8_ui::text::ellipsize(sub, 11.0, false, ir.w - 44.0), 11.0, t.fg_dim, false); }
                    let (clicked, _, _) = ui.interact(id, ir);
                    if clicked { go = Some((path.clone(), false)); }
                    if hov && ui.frame.press[2] { go = Some((path.clone(), true)); }
                    if hov && ui.frame.press[1] && matches!(icon, Icon::Star) { rm = Some(path.clone()); }
                    y += h + 2.0;
                }
            }
        }
        if let Some((p, nt)) = go { if nt { self.new_tab(p) } else { self.goto(p, true) } }
        if let Some(p) = rm {
            self.bookmarks.retain(|b| b.path != p);
            let _ = places::save_bookmarks(&places::gtk_bookmarks_file(), &self.bookmarks);
            self.refresh_places(true);
            self.status = "Bookmark removed".into();
        }
    }

    fn list(&mut self, ui: &mut Ui, r: Rect) {
        let t = ui.theme.clone();
        let (hdr, body) = r.cut_top(30.0);
        let ai = self.active;
        let tab_uid = self.tabs[ai].uid;
        let nav = self.tabs[ai].nav;
        // header with sortable columns
        ui.draw().rect(hdr, t.bg);
        let cols = hdr.inset_xy(12.0, 0.0).row(0.0, &[Size::Flex(1.0), Size::Fixed(96.0), Size::Fixed(150.0)]);
        let names = [("Name", SortKey::Name), ("Size", SortKey::Size), ("Modified", SortKey::Modified)];
        for (i, (label, key)) in names.iter().enumerate() {
            let c = cols[i];
            let sort = self.tabs[ai].model.sort;
            let (clicked, hov, _) = ui.interact(Id::new("col").with(i as u64), c);
            if hov { ui.draw().rounded(c.inset_xy(0.0, 3.0), t.hover(), 5.0); }
            let active = sort.key == *key;
            let lw = fm8_ui::text::measure(label, t.font_size, true);
            let tx = if i == 0 { c.x + 6.0 } else { c.right() - 6.0 - lw - if active { 16.0 } else { 0.0 } };
            ui.label_ex(rect(tx, c.y, lw + 2.0, c.h), label, if active { t.fg } else { t.fg_dim }, true, 0.0);
            if active {
                let (ax, ay) = (tx + lw + 9.0, c.y + c.h / 2.0);
                let d = if sort.ascending { -1.0 } else { 1.0 };
                ui.draw().line((ax - 4.0, ay - 2.0 * d), (ax, ay + 2.0 * d), 1.6, t.fg);
                ui.draw().line((ax, ay + 2.0 * d), (ax + 4.0, ay - 2.0 * d), 1.6, t.fg);
            }
            if clicked {
                let mut s = sort;
                if s.key == *key { s.ascending = !s.ascending } else { s.key = *key; s.ascending = true }
                self.tabs[ai].model.set_sort(s);
                self.remember();
            }
        }
        ui.separator(rect(hdr.x, hdr.bottom() - 1.0, hdr.w, 1.0));
        let id = Id::new("list").with(tab_uid).with(nav);
        let n = self.tabs[ai].model.view.len();
        if let Some(e) = &self.tabs[ai].model.error { ui.label_ex(body.inset(16.0), e, t.urgent, false, 0.0); }
        else if n == 0 && !self.tabs[ai].model.loading { ui.label_ex(body, "This folder is empty", t.fg_dim, false, 0.5); }
        // type-to-find (Thunar style)
        if !ui.typing && ui.focus == Some(id) && !ui.frame.text.is_empty() {
            if ui.time - self.find_at > 1.0 { self.find.clear(); }
            let txt: String = ui.frame.text.chars().filter(|c| !c.is_control()).collect();
            ui.frame.text.clear();
            self.find.push_str(&txt);
            self.find_at = ui.time;
            let f = self.find.clone();
            if let Some(pos) = self.tabs[ai].model.find_prefix(&f) { self.tabs[ai].model.click(pos, false, false); ui.reveal(id, pos); }
        }
        if !ui.typing && (ui.focus.is_none() || ui.focus == self.last_list) { ui.focus = Some(id); }
        self.last_list = Some(id);
        let resp = ui.virt(id, body, n, None, 30.0);
        let mut ctx = None;
        let mut open: Option<(PathBuf, bool)> = None;
        let mut newtab: Option<PathBuf> = None;
        for ev in &resp.events {
            let m = &mut self.tabs[ai].model;
            match ev {
                ListEvent::Click { i, mods } => m.click(*i, mods.ctrl, mods.shift),
                ListEvent::DoubleClick { i } => { if let Some(e) = m.entry(*i) { open = Some((e.path.clone(), e.is_dir)); } }
                ListEvent::MiddleClick { i } => { if let Some(e) = m.entry(*i) { if e.is_dir { newtab = Some(e.path.clone()); } } }
                ListEvent::RightClick { i } => {
                    if !m.is_selected(*i) { m.click(*i, false, false); }
                    if let Some(e) = m.entry(*i) { ctx = Some(Ctx::Entry(e.path.clone(), e.is_dir)); }
                }
                ListEvent::ClickEmpty => { m.clear_selection(); }
                ListEvent::Nav(k, mods) => {
                    let page = resp.page_rows() as isize;
                    let d = match k { Nav::Up => -1, Nav::Down => 1, Nav::PageUp => -page, Nav::PageDown => page, Nav::Home => -(n as isize), Nav::End => n as isize, _ => 0 };
                    if d != 0 { m.move_cursor(d, mods.shift); if let Some(c) = m.cursor { ui.reveal(id, c); } }
                }
                ListEvent::Activate => { open = None; self.open_cursor(); }
                ListEvent::DragStart { .. } => {}
            }
        }
        if ctx.is_none() && ui.hovered(body) && ui.frame.press[1] && resp.hovered.is_none() { ctx = Some(Ctx::Empty); }
        if let Some(c) = ctx { self.ctx = Some((ui.mouse, c)); }
        if let Some((p, d)) = open { self.open_path(p, d, false); }
        if let Some(p) = newtab { self.new_tab(p); }
        // rows
        let m = &self.tabs[ai].model;
        ui.push_clip(body);
        for pos in resp.first..resp.last {
            let Some(e) = m.entry(pos) else { break };
            let rr = resp.cell_rect(pos);
            let rr = rect(rr.x + 4.0, rr.y + 1.0, rr.w - 18.0, rr.h - 2.0);
            let sel = m.is_selected(pos);
            let hov = resp.hovered == Some(pos);
            if sel { ui.draw().rounded(rr, t.selection_bg, t.radius); } else if hov { ui.draw().rounded(rr, t.hover(), t.radius); }
            if m.cursor == Some(pos) && ui.focus == Some(id) { ui.draw().stroke(rr, t.border_focus, t.radius, 1.5); }
            let c = rr.inset_xy(8.0, 0.0).row(0.0, &[Size::Flex(1.0), Size::Fixed(96.0), Size::Fixed(150.0)]);
            let (fg, dim) = if sel { (t.selection_fg, t.selection_fg) } else { (t.fg, t.fg_dim) };
            let icol = if e.is_dir { t.accent } else { t.fg_dim };
            ui.icon2(icon_for(e.class), rect(c[0].x, rr.y + 5.0, 20.0, 20.0), if sel { t.selection_fg } else { icol }, if sel { t.selection_bg } else { t.bg });
            ui.label(rect(c[0].x + 30.0, rr.y, c[0].w - 36.0, rr.h), &e.name, if e.hidden { dim } else { fg });
            if !e.is_dir { ui.label_ex(rect(c[1].x, rr.y, c[1].w - 6.0, rr.h), &human_size(e.size), dim, false, 1.0); }
            ui.label_ex(rect(c[2].x, rr.y, c[2].w - 6.0, rr.h), &fmt_time(e.modified), dim, false, 1.0);
        }
        ui.pop_clip();
        if m.loading { ui.request_redraw(); }
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
        if m.loading { s.push_str("  (loading…)"); }
        ui.label(rect(r.x + 14.0, r.y, r.w * 0.5, r.h), &s, t.fg_dim);
        if !self.status.is_empty() {
            ui.label_ex(rect(r.x + r.w * 0.45, r.y, r.w * 0.55 - 14.0, r.h), &self.status, t.warn, false, 1.0);
        } else {
            ui.label_ex(rect(r.x + r.w * 0.45, r.y, r.w * 0.55 - 14.0, r.h), "Ctrl+L path   Ctrl+T tab   Del trash   Ctrl+Z undo", t.fg_dim, false, 1.0);
        }
    }

    fn context_menu(&mut self, ui: &mut Ui) {
        let Some((pos, c)) = self.ctx.take() else { return };
        let items: Vec<MenuItem> = match &c {
            Ctx::Entry(_, is_dir) => vec![
                MenuItem::new("Open", "Enter"),
                if *is_dir { MenuItem::new("Open in new tab", "Middle click") } else { MenuItem::new("Open in new tab", "").disabled() },
                if *is_dir { MenuItem::new("Add to bookmarks", "") } else { MenuItem::new("Add to bookmarks", "").disabled() },
                MenuItem::sep(),
                MenuItem::new("Move to trash", "Del"),
            ],
            Ctx::Empty => vec![MenuItem::new("New folder", ""), MenuItem::new("Reload", "F5"), MenuItem::new("Select all", "Ctrl+A")],
        };
        match ui.menu(Id::new("ctx"), pos, &items) {
            MenuResp::Open => self.ctx = Some((pos, c)),
            MenuResp::Dismiss => {}
            MenuResp::Chosen(i) => match (&c, i) {
                (Ctx::Entry(p, d), 0) => self.open_path(p.clone(), *d, false),
                (Ctx::Entry(p, _), 1) => self.new_tab(p.clone()),
                (Ctx::Entry(p, _), 2) => {
                    if places::add_bookmark(&mut self.bookmarks, p) {
                        let _ = places::save_bookmarks(&places::gtk_bookmarks_file(), &self.bookmarks);
                        self.refresh_places(true);
                        self.status = "Bookmark added".into();
                    }
                }
                (Ctx::Entry(..), 4) => self.trash_selection(),
                (Ctx::Empty, 0) => {
                    let dir = self.tabs[self.active].model.dir.clone();
                    match fileops::new_dir(&dir, &unique_folder_name(&dir)) { Ok(op) => { self.history.push(op); self.tabs[self.active].model.reload(); } Err(e) => self.status = e }
                }
                (Ctx::Empty, 1) => self.tabs[self.active].model.reload(),
                (Ctx::Empty, 2) => self.tabs[self.active].model.select_all(),
                _ => {}
            },
        }
    }
}

fn unique_folder_name(dir: &Path) -> String {
    let mut n = 1;
    loop {
        let name = if n == 1 { "New folder".to_string() } else { format!("New folder {n}") };
        if !dir.join(&name).exists() { return name; }
        n += 1;
    }
}
