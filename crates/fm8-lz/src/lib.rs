//! filem8's folder view on lazyUI-rs: a path bar, the entries of one folder as a list, a status line. `App` holds the
//! folder and what the list shows; `handle` turns the `Ui`'s events into navigation. The window loop is in `main.rs`.
//! Keys: enter on a folder goes in, alt-up goes up, ctrl-l puts the cursor in the path bar, tab moves between the bar and the list.
pub mod thumbs;

use fm8_core::class::Class;
use fm8_core::listing::{compare, human_size, list_blocking, Entry, Sort};
use lazyui::prelude::*;
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
}

/// The one line a row shows: a folder mark, the name, the size.
pub fn row(e: &Entry) -> String {
    let (mark, size) = if e.is_dir { ("/", format!("{} items", e.items)) } else { (" ", human_size(e.size)) };
    format!("{mark}{}\t{size}", e.name)
}

const SPEC: &str = "col\n  pathbar id=bar bind=path dirs=1 size=1\n  pages bind=mode\n    list id=files\n    grid id=grid count=@count tile=16x8 prefix=t bind=cur\n  text id=status bind=status size=1\n  floatwin id=props bind=props title=Properties from=info w=44 h=10\n";

/// The `Ui` of the folder view, with its keys. ctrl-g switches between the list and the thumbnail grid.
pub fn ui() -> Ui {
    let mut ui = Ui::from_spec(SPEC).expect("the folder view spec is valid");
    ui.hotkey("alt-up", "up");
    ui.hotkey("ctrl-l", "bar");
    ui.hotkey("ctrl-g", "grid");
    ui.hotkey("ctrl-i", "info");
    ui.every(0.2, "poll");
    ui
}

/// The text of the properties window for one entry.
pub fn describe(e: &Entry) -> String {
    let kind = if e.is_dir { "folder".to_string() } else { format!("{:?}", e.class).to_lowercase() };
    let size = if e.is_dir { format!("{} items", e.items) } else { format!("{} ({} bytes)", human_size(e.size), e.size) };
    format!("{}\nkind: {kind}\nsize: {size}\npath: {}", e.name, e.path.display())
}

impl App {
    pub fn open(dir: &Path) -> App {
        let mut a = App { dir: PathBuf::new(), entries: Vec::new(), error: None, thumbs: thumbs::Thumbs::default(), gen: 0, asked: HashSet::new(), shown: 0 };
        a.go(dir);
        a
    }

    /// Read `dir` (folders first, then by name; hidden entries last) and make it the current folder. A folder that
    /// cannot be read leaves the old one and says why in the status line.
    pub fn go(&mut self, dir: &Path) {
        match list_blocking(dir) {
            Ok(mut v) => {
                v.retain(|e| !e.hidden);
                v.sort_by(|a, b| compare(a, b, Sort::default()));
                self.dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
                self.entries = v;
                self.error = None;
                self.gen += 1;
                self.asked.clear();
            }
            Err(e) => self.error = Some(e),
        }
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
    }

    /// An event of the `Ui`: Enter in the path bar goes to that folder; Enter on a list row goes into the folder or
    /// ignores a file; alt-up goes up and puts the cursor on the folder just left.
    pub fn handle(&mut self, ui: &mut Ui, e: &UiEvent) {
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
                    }
                    None => ui.set("props", 0.0),
                }
            }
            ("app", "bar") => {
                ui.focus("bar");
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
        let app = App::open(&d);
        let rows: Vec<String> = app.entries.iter().map(row).collect();
        assert_eq!(rows, vec!["/docs\t0 items", "/music\t1 items", " notes.txt\t5 B"], "{rows:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn enter_goes_into_a_folder_and_alt_up_comes_back_onto_it() {
        let d = tree("nav");
        let mut app = App::open(&d);
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
        let mut app = App::open(&d);
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
        let mut app = App::open(&d);
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
        let mut app = App::open(&d);
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
        let mut app = App::open(&d);
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
}
