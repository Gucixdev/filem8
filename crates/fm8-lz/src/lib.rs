//! filem8's folder view on lazyUI-rs: a path bar, the entries of one folder as a list, a status line. `App` holds the
//! folder and what the list shows; `handle` turns the `Ui`'s events into navigation. The window loop is in `main.rs`.
//! Keys: enter on a folder goes in, alt-up goes up, ctrl-l puts the cursor in the path bar, tab moves between the bar and the list.
pub mod preview;
pub mod thumbs;

use fm8_core::class::Class;
use fm8_core::listing::{compare, human_size, list_blocking, Entry, Sort, SortKey};
use fm8_core::places::{self, Bookmark};
use fm8_core::settings::{config_dir, Prefs};
use lazyui::prelude::*;
use fm8_core::fileops;
use fm8_core::xfer::Transfer;
use lazyui::media::Image;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub struct App {
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    pub error: Option<String>,
    thumbs: thumbs::Thumbs,
    /// bumped on every folder change so late thumbnails of the old one are dropped
    gen: u64,
    asked: HashSet<usize>,
    shown: usize,
    history: fileops::History,
    /// what the open trash question is about
    pending: Option<Vec<PathBuf>>,
    /// what the open name prompt is for
    prompt: Option<Prompt>,
    /// what ctrl-c or ctrl-x took, and whether it is to be moved
    clip: Option<(Vec<PathBuf>, bool)>,
    /// the folder tabs: each remembers its folder and the row the cursor was on; `tab` is the one shown
    tabs: Vec<Tab>,
    tab: usize,
    /// the places sidebar: what each row of it opens
    places: Vec<Target>,
    bookmarks: Vec<Bookmark>,
    bookmarks_file: PathBuf,
    /// the trash view: while it is Some the list shows these items and the folder commands are off
    trash: Option<Vec<fm8_core::trashbin::TrashItem>>,
    /// the settings view: the list shows the preferences and Enter changes the one under the cursor
    settings: bool,
    prefs: Prefs,
    prefs_file: PathBuf,
    previewer: preview::Previewer,
    /// request number of the newest preview asked for, and what it was for
    pv_seq: u64,
    pv_path: Option<PathBuf>,
    /// the copy or move running on its own thread
    job: Option<Paste>,
}

/// One running paste: the transfer, what it carries, and where it came from (a cut that fails goes back on the clipboard).
struct Paste {
    transfer: Transfer,
    name: String,
    from: Vec<PathBuf>,
}

impl Paste {
    /// What the status line says when the transfer is over, and the change to remember for undo.
    fn finish(self, done: Result<fileops::Op, String>) -> (Option<fileops::Op>, String) {
        let (verb, stopped) = (if self.transfer.copy { "copied" } else { "moved" }, self.transfer.cancelled());
        let past = if self.transfer.copy { "pasted" } else { "moved" };
        match done {
            // a stopped move keeps what had already arrived; one that got through before the stop is just moved
            Ok(op) if stopped && !self.transfer.copy && self.from.iter().any(|p| p.exists()) => (Some(op), format!("stopped, part of {} was {verb} (ctrl-z takes it back)", self.name)),
            Ok(op) => (Some(op), format!("{past} {}", self.name)),
            Err(_) if stopped => (None, format!("stopped, nothing of {} was {verb}", self.name)),
            Err(e) => (None, e),
        }
    }
}

enum Prompt {
    Rename(PathBuf),
    NewDir,
}

/// The one line a row shows: a folder mark, the name, the size.
pub fn row(e: &Entry) -> String {
    let (mark, size) = if e.is_dir { ("/", format!("{} items", e.items)) } else { (" ", human_size(e.size)) };
    format!("{mark}{}\t{size}", e.name)
}

const SPEC: &str = "col\n  pathbar id=bar bind=path dirs=1 size=1\n  buttonbar id=tabs items=@tabnames active=tab size=@tabh:0\n  row\n    list id=places size=@sw:0\n    pages bind=mode\n      list id=files multi=1\n      grid id=grid count=@count tile=16x8 prefix=t bind=cur\n    col size=@pw:0\n      pages bind=pvmode\n        list id=pv\n        image bind=pvimg\n  row size=@xh:0\n    text bind=xtext size=44\n    progressbar bind=xfrac max=1\n  text id=status bind=status size=1\n  floatwin id=props bind=props title=Properties from=info w=44 h=10\n  dialog id=purge bind=purge title=\"Delete for good\" from=purgetext buttons=\"Delete,Cancel\" default=1\n  dialog id=ask bind=ask title=\"Move to trash\" from=asktext buttons=\"Trash,Cancel\" default=1\n  dialog id=prompt bind=prompt title=Name from=ptext field=pname buttons=\"OK,Cancel\"\n";

/// What a row of the places sidebar does.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    Dir(PathBuf),
    Trash,
    Header,
}

/// One folder tab.
struct Tab {
    dir: PathBuf,
    at: usize,
}

/// The `Ui` of the folder view, with its keys. ctrl-g switches between the list and the thumbnail grid.
pub fn ui() -> Ui {
    let mut ui = Ui::from_spec(SPEC).expect("the folder view spec is valid");
    ui.hotkey("alt-up", "up");
    ui.hotkey("ctrl-l", "bar");
    ui.hotkey("ctrl-g", "grid");
    ui.hotkey("ctrl-i", "info");
    ui.hotkey("delete", "trash");
    ui.hotkey("ctrl-z", "undo");
    ui.hotkey("ctrl-c", "copy");
    ui.hotkey("ctrl-x", "cut");
    ui.hotkey("ctrl-v", "paste");
    ui.hotkey("ctrl-p", "pane");
    ui.hotkey("f9", "side");
    ui.hotkey("ctrl-d", "bookmark");
    ui.hotkey("alt-t", "bin");
    ui.hotkey("ctrl-t", "newtab");
    ui.hotkey("ctrl-w", "closetab");
    ui.hotkey("ctrl-tab", "nexttab");
    ui.hotkey("ctrl-shift-tab", "prevtab");
    ui.hotkey("ctrl-,", "settings");
    ui.hotkey("ctrl-r", "restore");
    ui.hotkey("f2", "rename");
    ui.hotkey("ctrl-n", "newdir");
    ui.every(0.2, "poll");
    ui
}

struct Setting {
    key: &'static str,
    label: &'static str,
    /// empty: an on/off switch
    choices: &'static [&'static str],
}

/// The preferences this window offers; the names and values are those of `filem8`'s settings file.
const SETTINGS: &[Setting] = &[
    Setting { key: "show_hidden", label: "Show hidden files", choices: &[] },
    Setting { key: "sort", label: "Sort by", choices: &["name", "size", "modified", "type", "perms", "owner", "items"] },
    Setting { key: "sort_ascending", label: "Ascending order", choices: &[] },
    Setting { key: "confirm_trash", label: "Confirm before moving to trash", choices: &[] },
];

/// The text of the properties window for one entry.
pub fn describe(e: &Entry) -> String {
    let kind = if e.is_dir { "folder".to_string() } else { format!("{:?}", e.class).to_lowercase() };
    let size = if e.is_dir { format!("{} items", e.items) } else { format!("{} ({} bytes)", human_size(e.size), e.size) };
    format!("{}\nkind: {kind}\nsize: {size}\npath: {}", e.name, e.path.display())
}

impl App {
    pub fn open(dir: &Path) -> App {
        App::open_with(dir, config_dir().join("settings"), places::gtk_bookmarks_file())
    }

    /// Like `open`, with the preferences kept in `prefs_file` (the file `filem8` itself uses by default).
    pub fn open_with(dir: &Path, prefs_file: PathBuf, bookmarks_file: PathBuf) -> App {
        let mut a = App { dir: PathBuf::new(), entries: Vec::new(), error: None, thumbs: thumbs::Thumbs::default(), gen: 0, asked: HashSet::new(), shown: 0, history: fileops::History::default(), pending: None, prompt: None, clip: None, job: None, tabs: Vec::new(), tab: 0, places: Vec::new(), bookmarks: places::load_bookmarks(&bookmarks_file), bookmarks_file, trash: None, settings: false, prefs: Prefs::load(&prefs_file), prefs_file, previewer: preview::Previewer::default(), pv_seq: 0, pv_path: None };
        a.go(dir);
        a.tabs = vec![Tab { dir: a.dir.clone(), at: 0 }];
        a
    }

    /// Read `dir` (folders first, then by name; hidden entries last) and make it the current folder. A folder that
    /// cannot be read leaves the old one and says why in the status line.
    pub fn go(&mut self, dir: &Path) {
        match list_blocking(dir) {
            Ok(mut v) => {
                if !self.prefs.flag("show_hidden") {
                    v.retain(|e| !e.hidden);
                }
                let sort = self.sort();
                v.sort_by(|a, b| compare(a, b, sort));
                self.dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
                self.entries = v;
                self.error = None;
                self.gen += 1;
                self.asked.clear();
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn sort(&self) -> Sort {
        let key = match self.prefs.get("sort").as_str() {
            "size" => SortKey::Size,
            "modified" => SortKey::Modified,
            "type" => SortKey::Type,
            "perms" => SortKey::Perms,
            "owner" => SortKey::Owner,
            "items" => SortKey::Items,
            _ => SortKey::Name,
        };
        Sort { key, ascending: self.prefs.flag("sort_ascending"), dirs_first: true }
    }

    /// Put what the app knows into the `Ui`: the path text, the rows, the status line.
    pub fn show(&mut self, ui: &mut Ui) {
        ui.set_text("path", &self.dir.display().to_string());
        ui.set_list("files", self.entries.iter().map(row).collect());
        let status = match &self.error {
            Some(e) => e.clone(),
            None => format!("{} entries", self.entries.len()),
        };
        ui.set_text("status", &status);
        for i in 0..self.shown {
            ui.remove_image(&format!("t{i}"));
        }
        for (i, e) in self.entries.iter().enumerate() {
            ui.set_text(&format!("t{i}"), &e.name);
        }
        self.shown = self.entries.len();
        ui.set("count", self.entries.len() as f64);
        self.show_tabs(ui);
        self.show_places(ui);
    }

    /// The sidebar: Home and the XDG folders, the trash, bookmarks (ctrl-d adds the folder shown), mounted disks.
    fn show_places(&mut self, ui: &mut Ui) {
        let mut rows: Vec<(String, Target)> = vec![("Home".into(), Target::Dir(fm8_core::paths::home()))];
        rows.extend(places::user_dirs().into_iter().filter(|b| b.path.is_dir()).map(|b| (b.label, Target::Dir(b.path))));
        rows.push(("Trash".into(), Target::Trash));
        rows.push(("-- Bookmarks".into(), Target::Header));
        rows.extend(self.bookmarks.iter().map(|b| (b.label.clone(), Target::Dir(b.path.clone()))));
        rows.push(("-- Devices".into(), Target::Header));
        for m in places::mounts() {
            let free = places::space(&m.path).map(|(_, f)| format!("\t{}", human_size(f))).unwrap_or_default();
            rows.push((format!("{}{free}", m.label), Target::Dir(m.path)));
        }
        let at = ui.list_selected("places").unwrap_or(0);
        ui.set_list("places", rows.iter().map(|(t, _)| t.clone()).collect());
        ui.select_list("places", at.min(rows.len() - 1));
        self.places = rows.into_iter().map(|(_, t)| t).collect();
    }

    /// The tab bar: hidden with one tab, else one label per folder (the folder's own name).
    fn show_tabs(&mut self, ui: &mut Ui) {
        if let Some(t) = self.tabs.get_mut(self.tab) {
            t.dir = self.dir.clone();
        }
        let names: Vec<String> = self.tabs.iter().map(|t| t.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into())).collect();
        ui.set_text("tabnames", &names.join("\n"));
        ui.set("tabh", if names.len() > 1 { 1.0 } else { 0.0 });
        ui.set("tab", self.tab as f64);
    }

    fn cursor(&self, ui: &Ui) -> usize {
        if ui.number("mode") == 0.0 { ui.list_selected("files").unwrap_or(0) } else { ui.number("cur") as usize }
    }

    /// Show tab `n`: read its folder again and put the cursor back where it was.
    fn switch_tab(&mut self, ui: &mut Ui, n: usize) {
        if n >= self.tabs.len() {
            return;
        }
        self.tabs[self.tab].at = self.cursor(ui);
        self.tab = n;
        let (dir, at) = (self.tabs[n].dir.clone(), self.tabs[n].at);
        self.go(&dir);
        self.show(ui);
        ui.select_list("files", at.min(self.entries.len().saturating_sub(1)));
    }

    /// What a command acts on: the marked rows of the list, else the entry under the cursor; with a name to show for it.
    fn targets(&self, ui: &Ui) -> Option<(Vec<PathBuf>, String)> {
        let grid = ui.number("mode") != 0.0;
        let marks = if grid { Vec::new() } else { ui.list_marks("files") };
        let idx: Vec<usize> = if !marks.is_empty() {
            marks
        } else if grid {
            vec![ui.number("cur") as usize]
        } else {
            ui.list_selected("files").into_iter().collect()
        };
        let list: Vec<&Entry> = idx.iter().filter_map(|&i| self.entries.get(i)).collect();
        let name = match list.as_slice() {
            [] => return None,
            [one] => one.name.clone(),
            many => format!("{} items", many.len()),
        };
        Some((list.iter().map(|e| e.path.clone()).collect(), name))
    }

    /// Read the folder again after a change, keep the cursor near where it was and say what happened.
    fn refresh(&mut self, ui: &mut Ui, msg: &str) {
        let at = ui.list_selected("files").unwrap_or(0);
        let dir = self.dir.clone();
        self.go(&dir);
        self.show(ui);
        ui.select_list("files", at.min(self.entries.len().saturating_sub(1)));
        ui.set_text("status", msg);
    }

    /// An event of the `Ui`: Enter in the path bar goes to that folder; Enter on a list row goes into the folder or
    /// ignores a file; alt-up goes up and puts the cursor on the folder just left.
    pub fn handle(&mut self, ui: &mut Ui, e: &UiEvent) {
        self.handle_event(ui, e);
        self.sync_preview(ui);
    }

    /// Keep the preview pane on the entry under the cursor: ask for it when it changed, show an answer when it arrives.
    fn sync_preview(&mut self, ui: &mut Ui) {
        if ui.number("pw") <= 0.0 || self.trash.is_some() || self.settings {
            if self.pv_path.take().is_some() {
                ui.set_list("pv", Vec::new());
                ui.set("pvmode", 0.0);
            }
            return;
        }
        let at = if ui.number("mode") == 0.0 { ui.list_selected("files") } else { Some(ui.number("cur") as usize) };
        let entry = at.and_then(|i| self.entries.get(i)).map(|e| (e.path.clone(), e.is_dir));
        if entry.as_ref().map(|(p, _)| p) != self.pv_path.as_ref() {
            self.pv_path = entry.as_ref().map(|(p, _)| p.clone());
            self.pv_seq += 1;
            match entry {
                Some((p, false)) => self.previewer.ask(self.pv_seq, p),
                Some((p, true)) => {
                    let n = std::fs::read_dir(&p).map(|d| d.count()).unwrap_or(0);
                    ui.set_list("pv", vec![format!("folder, {n} entries")]);
                    ui.set("pvmode", 0.0);
                }
                None => ui.set_list("pv", Vec::new()),
            }
        }
        if let Some((seq, shown)) = self.previewer.latest() {
            if seq == self.pv_seq {
                match shown {
                    preview::Shown::Lines(l) => {
                        ui.set_list("pv", l);
                        ui.set("pvmode", 0.0);
                    }
                    preview::Shown::Picture { w, h, rgba } => {
                        if let Some(img) = Image::new(w, h, rgba) {
                            ui.set_image("pvimg", img);
                            ui.set("pvmode", 1.0);
                        }
                    }
                }
            }
        }
    }

    /// The trash view, entered and left with alt-t: rows are `name<TAB>deleted`, ctrl-r restores the marked rows (or the
    /// cursor row) to where they came from, delete removes them for good after a question.
    fn handle_trash(&mut self, ui: &mut Ui, e: &UiEvent) {
        match (e.id.as_str(), e.kind.as_str()) {
            ("app", "poll") => self.handle_event(ui, e),
            ("app", "bin") => {
                self.trash = None;
                self.pending = None;
                self.refresh(ui, "back in the folder");
            }
            ("app", "restore") => {
                let picked = self.trash_picked(ui);
                let (mut done, mut failed) = (0, None);
                for it in &picked {
                    match fm8_core::trashbin::restore(it) {
                        Ok(()) => done += 1,
                        Err(e) => failed = Some(e),
                    }
                }
                self.load_trash(ui, &match failed {
                    Some(e) => format!("restored {done}, then: {e}"),
                    None => format!("restored {done}"),
                });
            }
            ("app", "trash") => {
                let picked = self.trash_picked(ui);
                if !picked.is_empty() {
                    let what = if picked.len() == 1 { picked[0].original.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() } else { format!("{} items", picked.len()) };
                    ui.set_text("purgetext", &format!("Delete {what} for good? This cannot be undone."));
                    self.pending = Some(picked.into_iter().map(|i| i.trashed).collect());
                    ui.set("purge", 1.0);
                }
            }
            ("purge", "activate") if e.index == 0 => {
                let gone = self.pending.take().unwrap_or_default();
                let items: Vec<_> = self.trash.iter().flatten().filter(|i| gone.contains(&i.trashed)).cloned().collect();
                let mut failed = None;
                for it in &items {
                    if let Err(e) = fm8_core::trashbin::purge(it) {
                        failed = Some(e);
                    }
                }
                self.load_trash(ui, &match failed {
                    Some(e) => format!("deleting failed: {e}"),
                    None => format!("deleted {} for good", items.len()),
                });
            }
            ("purge", _) => self.pending = None,
            _ => {}
        }
    }

    /// The settings view (ctrl-, in and out): one row per preference, Enter changes the one under the cursor, the change
    /// is applied to the folder at once and written to the preferences file.
    fn show_settings(&mut self, ui: &mut Ui, msg: &str) {
        let rows = SETTINGS.iter().map(|f| format!("{}\t{}", f.label, self.prefs.get(f.key))).collect();
        let at = ui.list_selected("files").unwrap_or(0);
        ui.set_list("files", rows);
        ui.select_list("files", at.min(SETTINGS.len() - 1));
        ui.set("mode", 0.0);
        ui.set_text("path", "settings");
        ui.set_text("status", msg);
        self.settings = true;
    }

    fn handle_settings(&mut self, ui: &mut Ui, e: &UiEvent) {
        match (e.id.as_str(), e.kind.as_str()) {
            ("app", "settings") => {
                self.settings = false;
                self.refresh(ui, "back in the folder");
            }
            ("files", "activate") => {
                let Some(f) = usize::try_from(e.index).ok().and_then(|i| SETTINGS.get(i)) else { return };
                let now = self.prefs.get(f.key);
                let next = match f.choices {
                    [] => if now == "true" { "false" } else { "true" }.to_string(),
                    list => list[(list.iter().position(|c| *c == now).unwrap_or(0) + 1) % list.len()].to_string(),
                };
                self.prefs.set(f.key, &next);
                let saved = self.prefs.save(&self.prefs_file);
                let dir = self.dir.clone();
                self.go(&dir);
                self.show_settings(ui, &match saved {
                    Ok(()) => format!("{} = {next}", f.key),
                    Err(e) => format!("not saved: {e}"),
                });
            }
            _ => {}
        }
    }

    fn trash_picked(&self, ui: &Ui) -> Vec<fm8_core::trashbin::TrashItem> {
        let marks = ui.list_marks("files");
        let idx: Vec<usize> = if marks.is_empty() { ui.list_selected("files").into_iter().collect() } else { marks };
        idx.iter().filter_map(|&i| self.trash.as_ref()?.get(i)).cloned().collect()
    }

    /// Read the trash again (newest first) and show it.
    fn load_trash(&mut self, ui: &mut Ui, msg: &str) {
        let at = ui.list_selected("files").unwrap_or(0);
        let mut items = fm8_core::trashbin::list_roots(&fm8_core::trashbin::roots());
        items.sort_by(|a, b| b.deleted.cmp(&a.deleted));
        let rows = items.iter().map(|i| format!("{}\t{}", i.original.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), i.deleted.replace('T', " "))).collect();
        ui.set_list("files", rows);
        ui.select_list("files", at.min(items.len().saturating_sub(1)));
        ui.set("mode", 0.0);
        ui.set_text("path", "trash");
        ui.set_text("status", &format!("{msg}; {} items in the trash (ctrl-r restores, delete removes, alt-t leaves)", items.len()));
        self.trash = Some(items);
    }

    /// Shows how far the running paste is, or takes its result. The bar appears only for a paste that outlives one poll.
    fn poll_paste(&mut self, ui: &mut Ui) {
        let Some(job) = &self.job else { return };
        let Some(done) = job.transfer.poll() else {
            let fraction = f64::from(job.transfer.fraction());
            let verb = if job.transfer.copy { "copying" } else { "moving" };
            ui.set_text("xtext", &format!("{verb} {}  {:.0}%  esc stops", job.name, fraction * 100.0));
            ui.set("xfrac", fraction);
            ui.set("xh", 1.0);
            return;
        };
        let Some(job) = self.job.take() else { return };
        ui.set("xh", 0.0);
        let (cut, from) = (!job.transfer.copy, job.from.clone());
        let (op, msg) = job.finish(done);
        match op {
            Some(op) => self.history.push(op),
            None if cut && self.clip.is_none() => self.clip = Some((from, true)),
            None => {}
        }
        self.refresh(ui, &msg);
    }

    fn handle_event(&mut self, ui: &mut Ui, e: &UiEvent) {
        if e.id == "places" && (self.trash.is_some() || self.settings) {
            self.trash = None;
            self.settings = false;
        }
        if self.settings && e.id != "bar" && !(e.id == "app" && e.kind == "poll") {
            return self.handle_settings(ui, e);
        }
        if e.id == "app" && e.kind == "settings" {
            return self.show_settings(ui, "Enter changes a setting, ctrl-, leaves");
        }
        if self.trash.is_some() && !(e.id == "app" && e.kind == "poll") && e.id != "bar" {
            return self.handle_trash(ui, e);
        }
        if e.id == "app" && e.kind == "bin" {
            return self.load_trash(ui, "trash opened");
        }
        match (e.id.as_str(), e.kind.as_str()) {
            ("bar", "activate") => {
                let p = PathBuf::from(&e.text);
                if p.is_dir() {
                    self.go(&p);
                } else {
                    self.error = Some(format!("{}: not a folder", e.text));
                }
                self.show(ui);
                ui.focus("files");
            }
            ("files", "activate") => {
                if let Some(en) = usize::try_from(e.index).ok().and_then(|i| self.entries.get(i)).filter(|en| en.is_dir) {
                    let p = en.path.clone();
                    self.go(&p);
                    self.show(ui);
                }
            }
            ("grid", "need") => {
                let first = e.index.max(0) as usize;
                for i in first..(first + e.number as usize).min(self.entries.len()) {
                    if self.asked.insert(i) {
                        let en = &self.entries[i];
                        let job = if en.class == Class::Image { thumbs::Job::Thumb(en.path.clone()) } else { thumbs::Job::Icon(en.class, en.is_dir) };
                        self.thumbs.ask(self.gen, i, job);
                    }
                }
            }
            ("grid", "activate") => {
                if let Some(en) = usize::try_from(e.index).ok().and_then(|i| self.entries.get(i)).filter(|en| en.is_dir) {
                    let p = en.path.clone();
                    self.go(&p);
                    self.show(ui);
                }
            }
            ("app", "poll") => {
                self.poll_paste(ui);
                for d in self.thumbs.finished().into_iter().filter(|d| d.gen == self.gen) {
                    if let Some(img) = Image::new(d.w, d.h, d.rgba) {
                        ui.set_image(&format!("t{}", d.index), img);
                    }
                }
            }
            ("app", "grid") => {
                let grid = ui.number("mode") == 0.0;
                ui.set("mode", if grid { 1.0 } else { 0.0 });
                ui.focus(if grid { "grid" } else { "files" });
            }
            ("app", "info") => {
                let i = if ui.number("mode") == 0.0 { ui.list_selected("files") } else { Some(ui.number("cur") as usize) };
                match i.and_then(|i| self.entries.get(i)) {
                    Some(en) => {
                        ui.set_text("info", &describe(en));
                        ui.set("props", 1.0);
                        ui.focus("props");
                    }
                    None => ui.set("props", 0.0),
                }
            }
            ("app", "bar") => {
                ui.focus("bar");
            }
            ("app", "trash") => {
                if let Some((paths, name)) = self.targets(ui) {
                    self.pending = Some(paths);
                    if self.prefs.flag("confirm_trash") {
                        ui.set_text("asktext", &format!("Move {name} to the trash?"));
                        ui.set("ask", 1.0);
                    } else {
                        let yes = UiEvent { id: "ask".into(), kind: "activate".into(), text: String::new(), index: 0, number: 0.0 };
                        self.handle_event(ui, &yes);
                    }
                }
            }
            ("ask", "activate") if e.index == 0 => {
                if let Some(paths) = self.pending.take() {
                    let msg = match fileops::trash(&paths) {
                        Ok(op) => {
                            self.history.push(op);
                            format!("{} moved to the trash (ctrl-z undoes)", if paths.len() == 1 { paths[0].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() } else { format!("{} items", paths.len()) })
                        }
                        Err(e) => format!("trash failed: {e}"),
                    };
                    ui.clear_list_marks("files");
                    self.refresh(ui, &msg);
                }
            }
            ("ask", _) => self.pending = None,
            ("app", "copy" | "cut") => {
                if let Some((paths, name)) = self.targets(ui) {
                    let cut = e.kind == "cut";
                    self.clip = Some((paths, cut));
                    ui.set_text("status", &format!("{} {name} (ctrl-v pastes)", if cut { "cut" } else { "copied" }));
                }
            }
            ("app", "paste") => {
                if self.job.is_some() {
                    ui.set_text("status", "still busy with the last paste (esc stops it)");
                } else if let Some((from, cut)) = self.clip.clone() {
                    let name = if from.len() == 1 { from[0].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() } else { format!("{} items", from.len()) };
                    let transfer = Transfer::start(from.clone(), self.dir.clone(), !cut);
                    if cut {
                        self.clip = None;
                    }
                    ui.clear_list_marks("files");
                    ui.set_text("status", &format!("pasting {name}..."));
                    self.job = Some(Paste { transfer, name, from });
                }
            }
            // esc reaches this only when no widget had a use for it
            ("ui", "key") if e.text == "esc" => {
                if let Some(job) = &self.job {
                    job.transfer.cancel();
                    ui.set_text("status", &format!("stopping the paste of {}...", job.name));
                }
            }
            ("props", "close") => {
                ui.focus(if ui.number("mode") == 0.0 { "files" } else { "grid" });
            }
            ("app", "pane") => {
                let open = ui.number("pw") > 0.0;
                ui.set("pw", if open { 0.0 } else { 44.0 });
                self.pv_path = None;
            }
            ("app", "rename") => {
                let i = if ui.number("mode") == 0.0 { ui.list_selected("files") } else { Some(ui.number("cur") as usize) };
                if let Some(en) = i.and_then(|i| self.entries.get(i)) {
                    ui.set_text("pname", &en.name);
                    ui.set_text("ptext", &format!("Rename {} to:", en.name));
                    self.prompt = Some(Prompt::Rename(en.path.clone()));
                    ui.set("prompt", 1.0);
                }
            }
            ("app", "newdir") => {
                ui.set_text("pname", "New folder");
                ui.set_text("ptext", "New folder in this directory:");
                self.prompt = Some(Prompt::NewDir);
                ui.set("prompt", 1.0);
            }
            ("prompt", "activate") if e.index == 0 => {
                let name = e.text.trim().to_string();
                let Some(what) = self.prompt.take() else { return };
                if name.is_empty() || name.contains('/') {
                    self.refresh(ui, "that is not a usable name");
                    return;
                }
                let done = match &what {
                    Prompt::Rename(p) => fileops::rename(p, &name).map(|op| (op, format!("renamed to {name}"))),
                    Prompt::NewDir => fileops::new_dir(&self.dir, &name).map(|op| (op, format!("created {name}"))),
                };
                match done {
                    Ok((op, msg)) => {
                        self.history.push(op);
                        self.refresh(ui, &msg);
                        if let Some(i) = self.entries.iter().position(|en| en.name == name) {
                            ui.select_list("files", i);
                        }
                    }
                    Err(e) => self.refresh(ui, &e),
                }
            }
            ("prompt", _) => self.prompt = None,
            ("app", "undo") => {
                let msg = match self.history.undo() {
                    Ok(s) | Err(s) => s,
                };
                self.refresh(ui, &msg);
            }
            ("app", "newtab") => {
                let at = self.cursor(ui);
                self.tabs[self.tab].at = at;
                self.tabs.insert(self.tab + 1, Tab { dir: self.dir.clone(), at });
                self.tab += 1;
                self.show_tabs(ui);
            }
            ("app", "closetab") if self.tabs.len() > 1 => {
                self.tabs.remove(self.tab);
                let n = self.tab.min(self.tabs.len() - 1);
                self.tab = n;
                let (dir, at) = (self.tabs[n].dir.clone(), self.tabs[n].at);
                self.go(&dir);
                self.show(ui);
                ui.select_list("files", at.min(self.entries.len().saturating_sub(1)));
            }
            ("app", "nexttab") => self.switch_tab(ui, (self.tab + 1) % self.tabs.len()),
            ("app", "prevtab") => self.switch_tab(ui, (self.tab + self.tabs.len() - 1) % self.tabs.len()),
            ("tabs", "activate" | "change") => {
                if let Ok(n) = usize::try_from(e.index) {
                    self.switch_tab(ui, n);
                }
            }
            ("places", "activate") => match usize::try_from(e.index).ok().and_then(|i| self.places.get(i)).cloned() {
                Some(Target::Dir(p)) => {
                    self.go(&p);
                    self.show(ui);
                    ui.focus("files");
                }
                Some(Target::Trash) => {
                    self.load_trash(ui, "trash opened");
                    ui.focus("files");
                }
                _ => {}
            },
            ("app", "side") => {
                let open = ui.number("sw") > 0.0;
                ui.set("sw", if open { 0.0 } else { 24.0 });
                ui.focus(if open { "files" } else { "places" });
            }
            ("app", "bookmark") => {
                let dir = self.dir.clone();
                let msg = if places::add_bookmark(&mut self.bookmarks, &dir) {
                    match places::save_bookmarks(&self.bookmarks_file, &self.bookmarks) {
                        Ok(()) => format!("bookmarked {}", dir.display()),
                        Err(e) => format!("bookmark not saved: {e}"),
                    }
                } else {
                    "already bookmarked".to_string()
                };
                self.show_places(ui);
                ui.set_text("status", &msg);
            }
            ("app", "up") => {
                if let Some(parent) = self.dir.parent().map(Path::to_path_buf) {
                    let from = self.dir.clone();
                    self.go(&parent);
                    self.show(ui);
                    if let Some(i) = self.entries.iter().position(|en| en.path == from) {
                        ui.select_list("files", i);
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lazyui::input::{Event, Key, Mods};

    /// Tests that point the trash at their own folder run one at a time.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// An app whose preferences live beside the test folder (not in it), with the trash question switched on.
    fn open_t(d: &Path) -> App {
        let file = d.with_extension("prefs");
        std::fs::write(&file, "confirm_trash = true\n").unwrap();
        App::open_with(d, file, d.with_extension("bookmarks"))
    }

    fn tree(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fm8-lz-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("music/rock")).unwrap();
        std::fs::create_dir_all(d.join("docs")).unwrap();
        std::fs::write(d.join("notes.txt"), "hello").unwrap();
        std::fs::write(d.join(".secret"), "x").unwrap();
        d
    }

    fn key(ui: &mut Ui, k: Key) {
        ui.dispatch(&Event::Key(k, Mods::default()));
    }

    fn handle_all(app: &mut App, ui: &mut Ui) {
        for e in ui.take_events() {
            app.handle(ui, &e);
        }
    }

    #[test]
    fn rows_are_folders_first_hidden_left_out_with_sizes() {
        let d = tree("rows");
        let app = open_t(&d);
        let rows: Vec<String> = app.entries.iter().map(row).collect();
        assert_eq!(rows, vec!["/docs\t0 items", "/music\t1 items", " notes.txt\t5 B"], "{rows:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn enter_goes_into_a_folder_and_alt_up_comes_back_onto_it() {
        let d = tree("nav");
        let mut app = open_t(&d);
        let mut ui = ui();
        app.show(&mut ui);
        let mut c = lazyui::canvas::Canvas::new(60, 10, lazyui::canvas::Caps::FULL);
        ui.render(&mut c);
        ui.focus("files");
        key(&mut ui, Key::Down);
        key(&mut ui, Key::Enter);
        handle_all(&mut app, &mut ui);
        assert!(app.dir.ends_with("music"), "{:?}", app.dir);
        assert_eq!(app.entries.len(), 1);
        ui.dispatch(&Event::Key(Key::Up, Mods { alt: true, ..Mods::default() }));
        handle_all(&mut app, &mut ui);
        assert_eq!(app.dir, d.canonicalize().unwrap());
        assert_eq!(ui.list_selected("files"), Some(1), "the cursor is on music again");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_path_typed_in_the_bar_is_opened_and_a_file_is_refused() {
        let d = tree("bar");
        let mut app = open_t(&d);
        let mut ui = ui();
        app.show(&mut ui);
        let mut c = lazyui::canvas::Canvas::new(60, 10, lazyui::canvas::Caps::FULL);
        ui.render(&mut c);
        ui.set_text("path", &d.join("docs").display().to_string());
        app.handle(&mut ui, &UiEvent::of("bar", "activate", &d.join("docs").display().to_string()));
        assert!(app.dir.ends_with("docs"));
        app.handle(&mut ui, &UiEvent::of("bar", "activate", &d.join("notes.txt").display().to_string()));
        assert!(app.error.as_deref().is_some_and(|e| e.contains("not a folder")), "{:?}", app.error);
        assert!(app.dir.ends_with("docs"), "the folder stays");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn grid_asks_for_image_thumbnails_and_they_arrive_as_pictures() {
        let d = tree("thumbs");
        let mut img = fm8_codec::Image::new(300, 200);
        for px in img.rgba.chunks_mut(4) {
            px.copy_from_slice(&[200, 30, 30, 255]);
        }
        std::fs::write(d.join("a.png"), fm8_codec::png::encode(&img)).unwrap();
        let mut app = open_t(&d);
        let mut ui = ui();
        app.show(&mut ui);
        let i = app.entries.iter().position(|e| e.name == "a.png").unwrap();
        app.handle(&mut ui, &UiEvent { id: "grid".into(), kind: "need".into(), text: String::new(), index: 0, number: app.entries.len() as f64 });
        let mut got = false;
        for _ in 0..100 {
            app.handle(&mut ui, &UiEvent::of("app", "poll", ""));
            if ui.has_image(&format!("t{i}")) {
                got = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(got, "the thumbnail of a.png never arrived");
        assert_eq!(thumbs::thumb(&d.join("a.png")).map(|t| (t.0, t.1)), Some((128, 85)));
        assert!(thumbs::thumb(&d.join("notes.txt")).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn ctrl_i_opens_the_properties_window_for_the_selected_entry() {
        let d = tree("props");
        let mut app = open_t(&d);
        let mut ui = ui();
        app.show(&mut ui);
        let mut c = lazyui::canvas::Canvas::new(80, 20, lazyui::canvas::Caps::FULL);
        ui.render(&mut c);
        ui.focus("files");
        key(&mut ui, Key::Down);
        key(&mut ui, Key::Down);
        ui.dispatch(&Event::Key(Key::Char('i'), Mods { ctrl: true, ..Mods::default() }));
        handle_all(&mut app, &mut ui);
        assert_eq!(ui.number("props"), 1.0);
        assert!(ui.text_of("info").starts_with("notes.txt\nkind: text\nsize: 5 B"), "{}", ui.text_of("info"));
        ui.render(&mut c);
        assert!(c.to_text().contains("Properties"), "{}", c.to_text());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn folders_get_an_icon_from_the_icon_theme_when_one_is_installed() {
        let mut themes = fm8_core::icontheme::IconThemes::system();
        if crate::thumbs::pick_theme(&mut themes).is_none() {
            eprintln!("no icon theme here: skipped");
            return;
        }
        let d = tree("icons");
        let mut app = open_t(&d);
        let mut ui = ui();
        app.show(&mut ui);
        app.handle(&mut ui, &UiEvent { id: "grid".into(), kind: "need".into(), text: String::new(), index: 0, number: 1.0 });
        let mut got = false;
        for _ in 0..200 {
            app.handle(&mut ui, &UiEvent::of("app", "poll", ""));
            if ui.has_image("t0") {
                got = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(got, "no icon for {:?}", app.entries[0].name);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn delete_asks_then_moves_to_the_trash_and_ctrl_z_brings_it_back() {
        let d = tree("trash");
        // the trash of this test lives inside the test tree, so nothing of the real one is touched
        let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("XDG_DATA_HOME", d.join(".data")) };
        let mut app = open_t(&d);
        let mut ui = ui();
        app.show(&mut ui);
        let mut c = lazyui::canvas::Canvas::new(80, 12, lazyui::canvas::Caps::FULL);
        ui.render(&mut c);
        ui.focus("files");
        key(&mut ui, Key::Down);
        key(&mut ui, Key::Down);
        assert_eq!(app.entries[2].name, "notes.txt");
        key(&mut ui, Key::Delete);
        handle_all(&mut app, &mut ui);
        ui.render(&mut c);
        assert!(c.to_text().contains("Move notes.txt to the trash?"), "{}", c.to_text());
        assert!(d.join("notes.txt").exists(), "nothing happens before the answer");
        key(&mut ui, Key::Esc);
        handle_all(&mut app, &mut ui);
        assert!(d.join("notes.txt").exists(), "cancel leaves it");
        key(&mut ui, Key::Delete);
        handle_all(&mut app, &mut ui);
        key(&mut ui, Key::Left);
        key(&mut ui, Key::Enter);
        handle_all(&mut app, &mut ui);
        assert!(!d.join("notes.txt").exists());
        assert!(app.entries.iter().all(|e| e.name != "notes.txt"));
        ui.render(&mut c);
        assert!(c.to_text().contains("moved to the trash"), "{}", c.to_text());
        ui.dispatch(&Event::Key(Key::Char('z'), Mods { ctrl: true, ..Mods::default() }));
        handle_all(&mut app, &mut ui);
        assert!(d.join("notes.txt").exists(), "undo restores it");
        assert!(app.entries.iter().any(|e| e.name == "notes.txt"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn f2_renames_and_ctrl_n_makes_a_folder_through_a_name_prompt() {
        let d = tree("prompt");
        let mut app = open_t(&d);
        let mut ui = ui();
        app.show(&mut ui);
        let mut c = lazyui::canvas::Canvas::new(80, 12, lazyui::canvas::Caps::FULL);
        ui.render(&mut c);
        ui.focus("files");
        key(&mut ui, Key::Down);
        key(&mut ui, Key::Down);
        key(&mut ui, Key::F(2));
        handle_all(&mut app, &mut ui);
        ui.render(&mut c);
        assert!(c.to_text().contains("> notes.txt"), "{}", c.to_text());
        for _ in 0..3 {
            key(&mut ui, Key::Backspace);
        }
        for ch in "md".chars() {
            key(&mut ui, Key::Char(ch));
        }
        key(&mut ui, Key::Enter);
        handle_all(&mut app, &mut ui);
        assert!(d.join("notes.md").exists() && !d.join("notes.txt").exists());
        assert_eq!(ui.list_selected("files").and_then(|i| app.entries.get(i)).map(|e| e.name.as_str()), Some("notes.md"));
        ui.dispatch(&Event::Key(Key::Char('n'), Mods { ctrl: true, ..Mods::default() }));
        handle_all(&mut app, &mut ui);
        key(&mut ui, Key::Enter);
        handle_all(&mut app, &mut ui);
        assert!(d.join("New folder").is_dir());
        key(&mut ui, Key::F(2));
        handle_all(&mut app, &mut ui);
        for _ in 0..12 {
            key(&mut ui, Key::Backspace);
        }
        for ch in "a/b".chars() {
            key(&mut ui, Key::Char(ch));
        }
        key(&mut ui, Key::Enter);
        handle_all(&mut app, &mut ui);
        ui.render(&mut c);
        assert!(c.to_text().contains("not a usable name"), "{}", c.to_text());
        ui.dispatch(&Event::Key(Key::Char('z'), Mods { ctrl: true, ..Mods::default() }));
        handle_all(&mut app, &mut ui);
        assert!(!d.join("New folder").exists(), "undo takes the new folder away again");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn copy_and_cut_paste_run_off_the_ui_thread_and_ctrl_z_takes_them_back() {
        let d = tree("paste");
        let mut app = open_t(&d);
        let mut ui = ui();
        app.show(&mut ui);
        let mut c = lazyui::canvas::Canvas::new(80, 12, lazyui::canvas::Caps::FULL);
        ui.render(&mut c);
        ui.focus("files");
        let ctrl = |ui: &mut Ui, ch: char| ui.dispatch(&Event::Key(Key::Char(ch), Mods { ctrl: true, ..Mods::default() }));
        let wait = |app: &mut App, ui: &mut Ui| {
            for _ in 0..200 {
                app.handle(ui, &UiEvent::of("app", "poll", ""));
                if app.job.is_none() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("the paste never finished");
        };
        key(&mut ui, Key::Down);
        key(&mut ui, Key::Down);
        assert_eq!(app.entries[2].name, "notes.txt");
        ctrl(&mut ui, 'c');
        handle_all(&mut app, &mut ui);
        key(&mut ui, Key::Up);
        key(&mut ui, Key::Up);
        key(&mut ui, Key::Enter);
        handle_all(&mut app, &mut ui);
        assert!(app.dir.ends_with("docs"));
        ctrl(&mut ui, 'v');
        handle_all(&mut app, &mut ui);
        wait(&mut app, &mut ui);
        assert!(d.join("docs/notes.txt").exists() && d.join("notes.txt").exists(), "a copy: both exist");
        assert!(app.entries.iter().any(|e| e.name == "notes.txt"), "the view shows it");
        ctrl(&mut ui, 'z');
        handle_all(&mut app, &mut ui);
        assert!(!d.join("docs/notes.txt").exists(), "undo removes the copy");
        app.go(&d);
        app.show(&mut ui);
        ui.render(&mut c);
        key(&mut ui, Key::Down);
        key(&mut ui, Key::Down);
        ctrl(&mut ui, 'x');
        handle_all(&mut app, &mut ui);
        key(&mut ui, Key::Up);
        key(&mut ui, Key::Up);
        key(&mut ui, Key::Enter);
        handle_all(&mut app, &mut ui);
        ctrl(&mut ui, 'v');
        handle_all(&mut app, &mut ui);
        wait(&mut app, &mut ui);
        assert!(d.join("docs/notes.txt").exists() && !d.join("notes.txt").exists(), "a move: only the new place");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_long_paste_shows_a_bar_and_esc_stops_it() {
        let d = tree("stop");
        let mut app = open_t(&d);
        let mut ui = ui();
        app.show(&mut ui);
        let mut c = lazyui::canvas::Canvas::new(80, 12, lazyui::canvas::Caps::FULL);
        ui.render(&mut c);
        ui.focus("files");
        for (copy, word) in [(true, "copying"), (false, "moving")] {
            let from = vec![d.join("notes.txt")];
            // held: the worker waits, so the stop is asked for before a byte moves
            app.job = Some(Paste { transfer: Transfer::start_held(from.clone(), d.join("docs"), copy), name: "notes.txt".into(), from: from.clone() });
            app.handle(&mut ui, &UiEvent::of("app", "poll", ""));
            assert_eq!(ui.number("xh"), 1.0);
            ui.render(&mut c);
            assert!(c.to_text().contains(&format!("{word} notes.txt  0%  esc stops")), "{}", c.to_text());
            key(&mut ui, Key::Esc);
            handle_all(&mut app, &mut ui);
            let job = app.job.as_ref().expect("still running until the worker answers");
            assert!(job.transfer.cancelled(), "esc on the list reaches the paste");
            job.transfer.release();
            for _ in 0..200 {
                app.handle(&mut ui, &UiEvent::of("app", "poll", ""));
                if app.job.is_none() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(app.job.is_none() && ui.number("xh") == 0.0);
            // a copy stops with nothing left behind; a move inside one disk is a rename and cannot be caught half way
            if copy {
                assert!(d.join("notes.txt").exists() && !d.join("docs/notes.txt").exists());
                assert_eq!(app.clip, None);
                assert_eq!(ui.text_of("status"), "stopped, nothing of notes.txt was copied");
            } else {
                assert!(!d.join("notes.txt").exists() && d.join("docs/notes.txt").exists());
                assert_eq!(ui.text_of("status"), "moved notes.txt", "it got through, so it is not called partial");
            }
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn marked_rows_are_trashed_and_pasted_together() {
        let d = tree("marks");
        std::fs::write(d.join("b.txt"), "b").unwrap();
        let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("XDG_DATA_HOME", d.join(".data")) };
        let mut app = open_t(&d);
        let mut ui = ui();
        app.show(&mut ui);
        let mut c = lazyui::canvas::Canvas::new(80, 12, lazyui::canvas::Caps::FULL);
        ui.render(&mut c);
        ui.focus("files");
        key(&mut ui, Key::Down);
        key(&mut ui, Key::Down);
        assert_eq!(app.entries[2].name, "b.txt");
        key(&mut ui, Key::Char(' '));
        key(&mut ui, Key::Char(' '));
        assert_eq!(ui.list_marks("files"), vec![2, 3]);
        ui.dispatch(&Event::Key(Key::Delete, Mods::default()));
        handle_all(&mut app, &mut ui);
        ui.render(&mut c);
        assert!(c.to_text().contains("Move 2 items to the trash?"), "{}", c.to_text());
        key(&mut ui, Key::Left);
        key(&mut ui, Key::Enter);
        handle_all(&mut app, &mut ui);
        assert!(!d.join("b.txt").exists() && !d.join("notes.txt").exists());
        assert!(ui.list_marks("files").is_empty());
        ui.dispatch(&Event::Key(Key::Char('z'), Mods { ctrl: true, ..Mods::default() }));
        handle_all(&mut app, &mut ui);
        assert!(d.join("b.txt").exists() && d.join("notes.txt").exists(), "one undo brings both back");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_preview_pane_follows_the_cursor_and_shows_text_then_a_folder() {
        let d = tree("pane");
        std::fs::write(d.join("notes.txt"), "hello preview\nsecond line\n").unwrap();
        let mut app = open_t(&d);
        let mut ui = ui();
        app.show(&mut ui);
        let mut c = lazyui::canvas::Canvas::new(100, 12, lazyui::canvas::Caps::FULL);
        ui.render(&mut c);
        ui.focus("files");
        key(&mut ui, Key::Down);
        key(&mut ui, Key::Down);
        assert!(!c.to_text().contains("hello preview"));
        ui.dispatch(&Event::Key(Key::Char('p'), Mods { ctrl: true, ..Mods::default() }));
        handle_all(&mut app, &mut ui);
        let mut seen = false;
        for _ in 0..200 {
            app.handle(&mut ui, &UiEvent::of("app", "poll", ""));
            ui.render(&mut c);
            if c.to_text().contains("second line") {
                seen = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(seen, "{}", c.to_text());
        key(&mut ui, Key::Up);
        handle_all(&mut app, &mut ui);
        ui.render(&mut c);
        assert!(c.to_text().contains("folder, 0 entries") || c.to_text().contains("folder, 1 entries"), "{}", c.to_text());
        assert!(!c.to_text().contains("second line"), "the old text is gone");
        ui.dispatch(&Event::Key(Key::Char('p'), Mods { ctrl: true, ..Mods::default() }));
        handle_all(&mut app, &mut ui);
        ui.render(&mut c);
        assert!(!c.to_text().contains("folder, "), "ctrl-p again closes the pane");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn previews_become_lines_or_a_picture() {
        use fm8_preview::{ArchiveEntry, Preview};
        match preview::shown(Preview::Archive { kind: "zip".into(), entries: vec![ArchiveEntry { name: "a".into(), size: 1, is_dir: true }], truncated: false }) {
            preview::Shown::Lines(l) => assert_eq!(l, vec!["zip archive, 1 entries", "a/"]),
            _ => panic!("lines expected"),
        }
        assert!(matches!(preview::shown(Preview::Error("no".into())), preview::Shown::Lines(l) if l == ["no"]));
    }

    #[test]
    fn the_trash_view_restores_and_deletes_for_good() {
        let d = tree("bin");
        std::fs::write(d.join("gone-a.txt"), "a").unwrap();
        std::fs::write(d.join("gone-b.txt"), "b").unwrap();
        let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("XDG_DATA_HOME", d.join(".data")) };
        let mut app = open_t(&d);
        let mut ui = ui();
        app.show(&mut ui);
        let mut c = lazyui::canvas::Canvas::new(100, 12, lazyui::canvas::Caps::FULL);
        ui.render(&mut c);
        ui.focus("files");
        let at = |app: &App, n: &str| app.entries.iter().position(|e| e.name == n).unwrap();
        for n in ["gone-a.txt", "gone-b.txt"] {
            ui.select_list("files", at(&app, n));
            ui.dispatch(&Event::Key(Key::Delete, Mods::default()));
            handle_all(&mut app, &mut ui);
            ui.render(&mut c);
            key(&mut ui, Key::Left);
            key(&mut ui, Key::Enter);
            handle_all(&mut app, &mut ui);
        }
        assert!(!d.join("gone-a.txt").exists() && !d.join("gone-b.txt").exists());
        let ctrl = |ui: &mut Ui, ch: char| ui.dispatch(&Event::Key(Key::Char(ch), Mods { ctrl: ch != 't', alt: ch == 't', ..Mods::default() }));
        ctrl(&mut ui, 't');
        handle_all(&mut app, &mut ui);
        ui.render(&mut c);
        assert!(c.to_text().contains("gone-a.txt") && c.to_text().contains("gone-b.txt"), "{}", c.to_text());
        let row = |app: &App, n: &str| app.trash.as_ref().unwrap().iter().position(|i| i.original.ends_with(n)).unwrap();
        ui.select_list("files", row(&app, "gone-b.txt"));
        ctrl(&mut ui, 'r');
        handle_all(&mut app, &mut ui);
        assert!(d.join("gone-b.txt").exists() && !d.join("gone-a.txt").exists());
        ui.select_list("files", row(&app, "gone-a.txt"));
        ui.dispatch(&Event::Key(Key::Delete, Mods::default()));
        handle_all(&mut app, &mut ui);
        ui.render(&mut c);
        assert!(c.to_text().contains("for good"), "{}", c.to_text());
        key(&mut ui, Key::Left);
        key(&mut ui, Key::Enter);
        handle_all(&mut app, &mut ui);
        ui.render(&mut c);
        assert!(!c.to_text().contains("gone-a.txt"), "{}", c.to_text());
        assert!(!d.join("gone-a.txt").exists());
        ctrl(&mut ui, 't');
        handle_all(&mut app, &mut ui);
        ui.render(&mut c);
        assert!(c.to_text().contains("gone-b.txt") && !c.to_text().contains("\t"), "{}", c.to_text());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn settings_change_the_folder_at_once_and_are_saved() {
        let d = tree("prefs");
        std::fs::write(d.join(".dot"), "x").unwrap();
        let mut app = open_t(&d);
        let file = d.with_extension("prefs");
        let mut ui = ui();
        app.show(&mut ui);
        let mut c = lazyui::canvas::Canvas::new(100, 12, lazyui::canvas::Caps::FULL);
        ui.render(&mut c);
        assert!(!app.entries.iter().any(|e| e.name == ".dot"), "hidden files are off by default");
        ui.focus("files");
        ui.dispatch(&Event::Key(Key::Char(','), Mods { ctrl: true, ..Mods::default() }));
        handle_all(&mut app, &mut ui);
        ui.render(&mut c);
        assert!(c.to_text().contains("Show hidden files") && c.to_text().contains("Sort by"), "{}", c.to_text());
        app.handle(&mut ui, &UiEvent::of("app", "poll", ""));
        key(&mut ui, Key::Enter);
        handle_all(&mut app, &mut ui);
        assert!(std::fs::read_to_string(&file).unwrap().contains("show_hidden = true"));
        key(&mut ui, Key::Down);
        key(&mut ui, Key::Enter);
        handle_all(&mut app, &mut ui);
        assert!(std::fs::read_to_string(&file).unwrap().contains("sort = size"));
        ui.dispatch(&Event::Key(Key::Char(','), Mods { ctrl: true, ..Mods::default() }));
        handle_all(&mut app, &mut ui);
        ui.render(&mut c);
        assert!(c.to_text().contains(".dot"), "{}", c.to_text());
        let again = App::open_with(&d, file.clone(), d.with_extension("bookmarks"));
        assert!(again.entries.iter().any(|e| e.name == ".dot"), "a new app reads the saved file");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn trash_goes_without_a_question_when_confirm_is_off() {
        let d = tree("noask");
        let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::set_var("XDG_DATA_HOME", d.join(".data")) };
        let mut app = App::open_with(&d, d.with_extension("prefs2"), d.with_extension("bookmarks2"));
        let mut ui = ui();
        app.show(&mut ui);
        let mut c = lazyui::canvas::Canvas::new(100, 12, lazyui::canvas::Caps::FULL);
        ui.render(&mut c);
        ui.focus("files");
        let i = app.entries.iter().position(|e| e.name == "notes.txt").unwrap();
        ui.select_list("files", i);
        ui.dispatch(&Event::Key(Key::Delete, Mods::default()));
        handle_all(&mut app, &mut ui);
        assert!(!d.join("notes.txt").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn tabs_keep_their_folder_and_cursor() {
        let d = tree("tabs");
        let mut app = open_t(&d);
        let mut ui = ui();
        app.show(&mut ui);
        let mut c = lazyui::canvas::Canvas::new(60, 8, lazyui::canvas::Caps::FULL);
        ui.render(&mut c);
        assert_eq!(ui.number("tabh"), 0.0, "one tab shows no bar");
        ui.focus("files");
        let ctrl = |ui: &mut Ui, k: Key, shift: bool| ui.dispatch(&Event::Key(k, Mods { ctrl: true, shift, ..Mods::default() }));
        ctrl(&mut ui, Key::Char('t'), false);
        handle_all(&mut app, &mut ui);
        ui.render(&mut c);
        assert_eq!((app.tabs.len(), app.tab, ui.number("tabh")), (2, 1, 1.0));
        assert!(c.to_text().lines().nth(1).unwrap().contains(d.file_name().unwrap().to_str().unwrap()), "{}", c.to_text());
        key(&mut ui, Key::Enter);
        handle_all(&mut app, &mut ui);
        let docs = d.join("docs").canonicalize().unwrap();
        assert_eq!(app.dir, docs, "the second tab went into docs");
        ctrl(&mut ui, Key::Tab, false);
        handle_all(&mut app, &mut ui);
        assert_eq!(app.tab, 0);
        assert_eq!(app.dir, d.canonicalize().unwrap(), "the first tab is where it was");
        ctrl(&mut ui, Key::Tab, false);
        handle_all(&mut app, &mut ui);
        assert_eq!(app.dir, docs);
        ctrl(&mut ui, Key::BackTab, false);
        handle_all(&mut app, &mut ui);
        assert_eq!(app.tab, 0);
        ctrl(&mut ui, Key::Char('w'), false);
        handle_all(&mut app, &mut ui);
        ui.render(&mut c);
        assert_eq!((app.tabs.len(), ui.number("tabh")), (1, 0.0));
        assert_eq!(app.dir, docs, "closing the first tab leaves the other one");
        ctrl(&mut ui, Key::Char('w'), false);
        handle_all(&mut app, &mut ui);
        assert_eq!(app.tabs.len(), 1, "the last tab stays");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_sidebar_opens_places_and_ctrl_d_bookmarks_the_folder() {
        let d = tree("places");
        let mut app = open_t(&d);
        let mut ui = ui();
        app.show(&mut ui);
        let mut c = lazyui::canvas::Canvas::new(100, 14, lazyui::canvas::Caps::FULL);
        ui.render(&mut c);
        ui.focus("files");
        assert_eq!(ui.number("sw"), 0.0, "the sidebar starts closed");
        key(&mut ui, Key::Enter);
        handle_all(&mut app, &mut ui);
        let docs = d.join("docs").canonicalize().unwrap();
        assert_eq!(app.dir, docs);
        ui.dispatch(&Event::Key(Key::Char('d'), Mods { ctrl: true, ..Mods::default() }));
        handle_all(&mut app, &mut ui);
        let saved = std::fs::read_to_string(d.with_extension("bookmarks")).unwrap();
        assert!(saved.contains("docs"), "{saved}");
        ui.dispatch(&Event::Key(Key::Char('d'), Mods { ctrl: true, ..Mods::default() }));
        handle_all(&mut app, &mut ui);
        ui.render(&mut c);
        assert!(c.to_text().contains("already bookmarked"), "{}", c.to_text());
        ui.dispatch(&Event::Key(Key::Up, Mods { alt: true, ..Mods::default() }));
        handle_all(&mut app, &mut ui);
        assert_eq!(app.dir, d.canonicalize().unwrap());
        ui.dispatch(&Event::Key(Key::F(9), Mods::default()));
        handle_all(&mut app, &mut ui);
        ui.render(&mut c);
        assert_eq!(ui.number("sw"), 24.0);
        assert!(c.to_text().contains("Home") && c.to_text().contains("Trash") && c.to_text().contains("Bookmarks"), "{}", c.to_text());
        let row = app.places.iter().position(|t| *t == Target::Dir(docs.clone())).expect("the bookmark is a row");
        ui.select_list("places", row);
        key(&mut ui, Key::Enter);
        handle_all(&mut app, &mut ui);
        assert_eq!(app.dir, docs, "Enter on the bookmark goes there");
        let again = App::open_with(&d, d.with_extension("prefs"), d.with_extension("bookmarks"));
        assert_eq!(again.bookmarks.len(), 1, "a new app reads the bookmark back");
        let _ = std::fs::remove_dir_all(&d);
    }
}
