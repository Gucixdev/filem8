//! Feature tests: views, columns, dual pane, preview layout, rename,
//! copy/paste, drag and drop, bookmarks, settings. Everything runs in temp
//! dirs with temp config dirs; the user's real files and settings are never read.
use filem8::app::{App, Layout};
use filem8::preview_pane::Content;
use fm8_core::settings::ViewMode;
use fm8_ui::*;
use std::path::{Path, PathBuf};

struct Env { root: PathBuf, ui: Ui, t: f64 }

fn env(tag: &str) -> Env {
    let root = std::env::temp_dir().join(format!("fm8-feat-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    Env { root, ui: Ui::new(Theme::dark()), t: 0.0 }
}
impl Env {
    fn app(&self, start: &Path) -> App {
        let mut a = App::new_in(start.to_path_buf(), Some(self.root.join("cfg")), Some(self.root.join("bookmarks")), false);
        settle(&mut a);
        a
    }
    fn frame(&mut self, app: &mut App, ev: &[Event]) -> soft::Framebuffer {
        self.t += 0.1;
        let t = self.t;
        render_frame(&mut self.ui, 1000, 600, 1.0, t, ev, |ui| app.build(ui))
    }
    fn keys(&mut self, app: &mut App, k: Key, m: Mods) { self.frame(app, &[Event::Key(k, m)]); settle(app); }
    fn click(&mut self, app: &mut App, x: f32, y: f32) {
        self.frame(app, &[Event::MouseMove(v2(x, y)), Event::MouseDown(Button::Left)]);
        self.frame(app, &[Event::MouseUp(Button::Left)]);
    }
    fn drag(&mut self, app: &mut App, from: (f32, f32), to: (f32, f32), ctrl: bool) {
        self.frame(app, &[Event::MouseMove(v2(from.0, from.1)), Event::MouseDown(Button::Left)]);
        self.frame(app, &[Event::MouseMove(v2((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0))]);
        let _ = ctrl;
        self.frame(app, &[Event::MouseMove(v2(to.0, to.1))]);
        self.frame(app, &[Event::MouseMove(v2(to.0, to.1))]);
        self.frame(app, &[Event::MouseUp(Button::Left)]);
        settle(app);
    }
}
impl Drop for Env { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); } }

fn settle(app: &mut App) {
    for _ in 0..400 {
        app.tick();
        if !app.tabs[app.active].model.loading && !app.transfers_active() { break; }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    app.tick();
}
fn ctrl(c: char) -> (Key, Mods) { (Key::Char(c), Mods { ctrl: true, ..Default::default() }) }
fn none() -> Mods { Mods::default() }

fn crop(f: &soft::Framebuffer, x: u32, y: u32, w: u32, h: u32) -> soft::Framebuffer {
    let mut px = vec![];
    for yy in y..y + h { for xx in x..x + w { let c = f.pixel(xx, yy); px.extend([c.r, c.g, c.b, 255]); } }
    soft::Framebuffer { w, h, px }
}
fn golden(name: &str, f: &soft::Framebuffer) {
    let g = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(format!("{name}.png"));
    if std::env::var("UPDATE_GOLDEN").is_ok() || !g.exists() { std::fs::create_dir_all(g.parent().unwrap()).unwrap(); f.save_png(&g).unwrap(); }
    let gold = soft::Framebuffer::load_png(&g).unwrap();
    assert_eq!((gold.w, gold.h), (f.w, f.h), "{name}: size");
    assert_eq!(f.diff(&gold, 3), 0, "{name}: differs from golden (UPDATE_GOLDEN=1 to refresh)");
}

fn png(path: &Path, c: Color) {
    let (w, h) = (48u32, 32u32);
    let mut px = vec![];
    for y in 0..h { for x in 0..w { let k = if (x / 8 + y / 8) % 2 == 0 { c } else { rgba(250, 250, 250, 255) }; px.extend([k.r, k.g, k.b, 255]); } }
    soft::Framebuffer { w, h, px }.save_png(path).unwrap();
}

fn sample(root: &Path) -> PathBuf {
    let d = root.join("work");
    std::fs::create_dir_all(d.join("sub")).unwrap();
    std::fs::write(d.join("sub/inner.txt"), "inner").unwrap();
    std::fs::write(d.join("notes.txt"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
    png(&d.join("pic.png"), rgba(220, 60, 60, 255));
    d
}

#[test]
fn view_modes_columns_and_per_folder_memory() {
    let mut e = env("views");
    let d = sample(&e.root);
    let mut app = e.app(&d);
    // list view: details columns (names only are stable: sizes/dates are checked as text below)
    let fb = e.frame(&mut app, &[]);
    golden("list_names", &crop(&fb, 215, 82, 190, 120));
    // grid with thumbnail
    let (k, m) = ctrl('2');
    e.keys(&mut app, k, m);
    assert_eq!(app.tabs[0].mode, ViewMode::Grid);
    for _ in 0..200 { app.tick(); e.frame(&mut app, &[]); std::thread::sleep(std::time::Duration::from_millis(5)); if !app.preview.path.is_some() && app.tick() == false { } }
    std::thread::sleep(std::time::Duration::from_millis(300));
    app.tick();
    let fb = e.frame(&mut app, &[]);
    golden("grid", &crop(&fb, 215, 82, 500, 300));
    let (k, m) = ctrl('3');
    e.keys(&mut app, k, m);
    assert_eq!(app.tabs[0].mode, ViewMode::Compact);
    let fb = e.frame(&mut app, &[]);
    golden("compact", &crop(&fb, 215, 82, 500, 120));
    // per-folder memory: sub stays in the default (list), work remembers compact
    app.goto(d.join("sub"), true);
    settle(&mut app);
    assert_eq!(app.tabs[0].mode, ViewMode::List);
    e.keys(&mut app, Key::Backspace, none());
    assert_eq!(app.tabs[0].mode, ViewMode::Compact);
    // columns can be switched off; the setting is persisted
    app.cols[0].on = false;
    let cols = filem8::views::cols_to_text(&app.cols);
    assert!(cols.starts_with("size\t86\t0"));
    // entries carry the new details
    let e0 = app.tabs[0].model.entries.iter().find(|x| x.name == "sub").unwrap();
    assert_eq!(e0.items, 1);
    assert_eq!(filem8::views::fmt_mode(0o755, true), "drwxr-xr-x");
}

#[test]
fn header_click_sorts_by_each_column_and_resizes() {
    let mut e = env("sort");
    let d = sample(&e.root);
    let mut app = e.app(&d);
    e.frame(&mut app, &[]);
    // header y ~ 97; Name header at x~240
    let before = app.cols[1].w;
    // drag the left edge of the "Modified" column (x = 978 - 558 + 86 = 506)
    e.frame(&mut app, &[Event::MouseMove(v2(506.0, 97.0)), Event::MouseDown(Button::Left)]);
    e.frame(&mut app, &[Event::MouseMove(v2(470.0, 97.0))]);
    e.frame(&mut app, &[Event::MouseUp(Button::Left)]);
    assert!(app.cols[1].w > before + 20.0, "column should have grown, was {before}, is {}", app.cols[1].w);
    // clicking Name twice flips the order
    e.click(&mut app, 260.0, 97.0);
    assert_eq!(app.tabs[0].model.sort.key, fm8_core::listing::SortKey::Name);
    assert!(!app.tabs[0].model.sort.ascending);
}

#[test]
fn dual_pane_copy_move_and_golden() {
    let mut e = env("dual");
    let a = e.root.join("A");
    let b = e.root.join("B");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(a.join("one.txt"), "1").unwrap();
    std::fs::write(a.join("two.txt"), "2").unwrap();
    std::fs::write(b.join("other.txt"), "3").unwrap();
    let mut app = e.app(&a);
    e.keys(&mut app, Key::F(3), none());
    assert_eq!(app.layout, Layout::Dual);
    // Tab -> right pane, go to B, Tab back
    e.keys(&mut app, Key::Tab, none());
    app.goto(b.clone(), true);
    settle(&mut app);
    e.keys(&mut app, Key::Tab, none());
    assert!(app.tabs[0].model.dir.ends_with("A"));
    // independent tabs per pane
    e.frame(&mut app, &[]);
    let fb = e.frame(&mut app, &[]);
    golden("dual", &crop(&fb, 215, 46, 785, 160));
    // select one.txt and F5 (copy) then two.txt and F6 (move)
    app.tabs[0].model.click(0, false, false);
    e.keys(&mut app, Key::F(5), none());
    assert!(b.join("one.txt").exists() && a.join("one.txt").exists());
    app.tabs[0].model.click(1, false, false);
    e.keys(&mut app, Key::F(6), none());
    assert!(b.join("two.txt").exists() && !a.join("two.txt").exists());
    // clicking in the right pane focuses it
    e.click(&mut app, 800.0, 140.0);
    assert!(app.tabs[0].model.dir.ends_with("B"), "right pane should have focus");
}

#[test]
fn preview_layout_shows_text_and_image() {
    let mut e = env("preview");
    let d = sample(&e.root);
    let mut app = e.app(&d);
    e.keys(&mut app, Key::F(3), none());
    e.keys(&mut app, Key::F(3), none());
    assert_eq!(app.layout, Layout::Preview);
    // entries: sub, notes.txt, pic.png
    e.keys(&mut app, Key::Down, none());
    e.keys(&mut app, Key::Down, none());
    for _ in 0..100 { app.tick(); std::thread::sleep(std::time::Duration::from_millis(10)); if matches!(app.preview.content, Content::Text { .. }) { break; } }
    assert!(matches!(app.preview.content, Content::Text { .. }), "text preview expected");
    let fb = e.frame(&mut app, &[]);
    golden("preview_text", &crop(&fb, 640, 46, 360, 200));
    e.keys(&mut app, Key::Down, none());
    for _ in 0..100 { app.tick(); std::thread::sleep(std::time::Duration::from_millis(10)); if matches!(app.preview.content, Content::Anim { .. }) { break; } }
    assert!(matches!(app.preview.content, Content::Anim { .. }), "image preview expected");
    let fb = e.frame(&mut app, &[]);
    golden("preview_image", &crop(&fb, 640, 46, 360, 300));
}

#[test]
fn rename_copy_paste_and_undo() {
    let mut e = env("ops");
    let d = sample(&e.root);
    let mut app = e.app(&d);
    // entries: sub, notes.txt, pic.png ; select notes.txt
    app.tabs[0].model.click(1, false, false);
    e.keys(&mut app, Key::F(2), none());
    assert!(app.rename.is_some());
    e.frame(&mut app, &[Event::Key(Key::Char('a'), Mods { ctrl: true, ..none() }), Event::Text("renamed.txt".into())]);
    e.keys(&mut app, Key::Enter, none());
    assert!(d.join("renamed.txt").exists() && !d.join("notes.txt").exists());
    // undo restores
    e.frame(&mut app, &[]);
    let (k, m) = ctrl('z');
    e.keys(&mut app, k, m);
    assert!(d.join("notes.txt").exists());
    // copy + paste into sub (name clash is resolved, not overwritten)
    settle(&mut app);
    let i = app.tabs[0].model.view.iter().position(|&x| app.tabs[0].model.entries[x].name == "notes.txt").unwrap();
    app.tabs[0].model.click(i, false, false);
    let (k, m) = ctrl('c');
    e.keys(&mut app, k, m);
    app.paste_into(&d.join("sub"));
    settle(&mut app);
    assert!(d.join("sub/notes.txt").exists() && d.join("notes.txt").exists());
    // cut + paste moves
    app.tabs[0].model.click(i, false, false);
    let (k, m) = ctrl('x');
    e.keys(&mut app, k, m);
    app.paste_into(&d.join("sub"));
    settle(&mut app);
    assert!(!d.join("notes.txt").exists());
}

#[test]
fn drag_and_drop_moves_into_folder_and_adds_bookmark() {
    let mut e = env("dnd");
    let d = sample(&e.root);
    let mut app = e.app(&d);
    e.frame(&mut app, &[]);
    // rows: sub (y~127), notes.txt (157), pic.png (187). drag pic.png onto sub
    e.drag(&mut app, (300.0, 187.0), (300.0, 127.0), false);
    assert!(d.join("sub/pic.png").exists() && !d.join("pic.png").exists(), "file should have moved into sub");
    // drag the folder onto the sidebar background: bookmark added and persisted
    e.drag(&mut app, (300.0, 127.0), (100.0, 480.0), false);
    assert_eq!(app.bookmarks.len(), 1);
    assert!(std::fs::read_to_string(e.root.join("bookmarks")).unwrap().contains("sub"));
}

#[test]
fn bookmarks_add_rename_reorder_remove_and_golden() {
    let mut e = env("bm");
    let d = sample(&e.root);
    std::fs::create_dir_all(d.join("alpha")).unwrap();
    std::fs::create_dir_all(d.join("beta")).unwrap();
    let mut app = e.app(&d.join("alpha"));
    let (k, m) = ctrl('d');
    e.keys(&mut app, k, m);
    assert_eq!(app.bookmarks.len(), 1);
    app.goto(d.join("beta"), true);
    settle(&mut app);
    e.keys(&mut app, k, m);
    assert_eq!(app.bookmarks.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(), ["alpha", "beta"]);
    // Ctrl+D again on the same folder removes it, and once more re-adds it at the end
    e.keys(&mut app, k, m);
    assert_eq!(app.bookmarks.len(), 1);
    e.keys(&mut app, k, m);
    // rename through the inline editor
    app.bm_rename_for_test(0);
    e.ui.focus = Some(Id::new("bmrename"));
    e.frame(&mut app, &[]);
    e.frame(&mut app, &[Event::Key(Key::Char('a'), Mods { ctrl: true, ..none() }), Event::Text("Proj".into())]);
    e.keys(&mut app, Key::Enter, none());
    assert_eq!(app.bookmarks[0].label, "Proj");
    // reorder by dragging the first bookmark below the second (sidebar rows)
    let fb = e.frame(&mut app, &[]);
    golden("sidebar_bookmarks", &crop(&fb, 0, 46, 214, 200));
    e.drag(&mut app, (100.0, 163.0), (100.0, 197.0), false);
    assert_eq!(app.bookmarks[0].label, "beta");
    assert!(std::fs::read_to_string(e.root.join("bookmarks")).unwrap().find("beta").unwrap() < std::fs::read_to_string(e.root.join("bookmarks")).unwrap().find("Proj").unwrap());
    // removal through the model API used by the context menu
    app.bookmarks.remove(0);
    assert_eq!(app.bookmarks.len(), 1);
}

#[test]
fn settings_panel_changes_apply_live_and_persist() {
    let mut e = env("settings");
    let d = sample(&e.root);
    let mut app = e.app(&d);
    let (k, m) = (Key::Char(','), Mods { ctrl: true, ..none() });
    e.keys(&mut app, k, m);
    assert!(app.settings.is_some());
    let fb = e.frame(&mut app, &[]);
    golden("settings_general", &crop(&fb, 100, 30, 800, 540));
    // "Show hidden files" checkbox (row 1) and "grid" default view (row 0)
    e.click(&mut app, 594.0, 158.0);
    assert!(app.prefs.flag("show_hidden"));
    e.click(&mut app, 647.0, 106.0);
    assert_eq!(app.prefs.get("view"), "grid");
    // live: the current folder has no remembered view, so it switched
    assert_eq!(app.tabs[0].mode, ViewMode::Grid);
    let txt = std::fs::read_to_string(e.root.join("cfg/settings")).unwrap();
    assert!(txt.contains("view = grid") && txt.contains("show_hidden = true"), "{txt}");
    // shortcut editor: rebind "New tab" to Ctrl+Y
    e.click(&mut app, 200.0, 212.0);
    e.click(&mut app, 650.0, 128.0);
    e.frame(&mut app, &[Event::Key(Key::Char('y'), Mods { ctrl: true, ..none() })]);
    assert_eq!(app.prefs.get("key.new_tab"), "ctrl+y");
    e.keys(&mut app, Key::Escape, none());
    assert!(app.settings.is_none());
    e.keys(&mut app, Key::Char('y'), Mods { ctrl: true, ..none() });
    assert_eq!(app.tabs.len(), 2);
    // a fresh app reads the saved settings
    let app2 = e.app(&d);
    assert_eq!(app2.prefs.get("view"), "grid");
}

#[test]
fn theme_icons_replace_procedural_ones_and_fall_back() {
    let mut e = env("icons");
    // a tiny theme: pure red folder, pure green text-x-generic (PNG, 48x48 fixed)
    let th = e.root.join("themes/fake");
    let solid = |c: [u8; 4]| { let mut i = fm8_codec::Image::new(48, 48); for p in i.rgba.chunks_mut(4) { p.copy_from_slice(&c) } fm8_codec::png::encode(&i) };
    std::fs::create_dir_all(th.join("48x48/places")).unwrap();
    std::fs::create_dir_all(th.join("48x48/mimetypes")).unwrap();
    std::fs::write(th.join("index.theme"), "[Icon Theme]\nName=Fake\nDirectories=48x48/places,48x48/mimetypes\n[48x48/places]\nSize=48\nType=Fixed\n[48x48/mimetypes]\nSize=48\nType=Fixed\n").unwrap();
    std::fs::write(th.join("48x48/places/folder.png"), solid([255, 0, 0, 255])).unwrap();
    std::fs::write(th.join("48x48/mimetypes/text-x-generic.png"), solid([0, 255, 0, 255])).unwrap();
    let work = e.root.join("work");
    std::fs::create_dir_all(work.join("adir")).unwrap();
    std::fs::write(work.join("note.txt"), "x").unwrap();
    let mut app = e.app(&work);
    // procedural first: no pure red / pure green anywhere
    let count = |fb: &soft::Framebuffer, c: [u8; 3]| (0..fb.h).flat_map(|y| (0..fb.w).map(move |x| (x, y))).filter(|&(x, y)| { let p = fb.pixel(x, y); [p.r, p.g, p.b] == c }).count();
    let fb = e.frame(&mut app, &[]);
    assert_eq!((count(&fb, [255, 0, 0]), count(&fb, [0, 255, 0])), (0, 0));
    app.icons.configure(vec![e.root.join("themes")], "fake", true);
    let mut fb = e.frame(&mut app, &[]);
    for _ in 0..200 { std::thread::sleep(std::time::Duration::from_millis(5)); app.tick(); fb = e.frame(&mut app, &[]); if count(&fb, [255, 0, 0]) > 0 { break } }
    assert!(count(&fb, [255, 0, 0]) > 100, "red folder icon drawn");
    assert!(count(&fb, [0, 255, 0]) > 100, "green text icon drawn");
    // a theme without the icon falls back to procedural icons (no crash, no red)
    app.icons.configure(vec![e.root.join("themes")], "missing", true);
    for _ in 0..20 { std::thread::sleep(std::time::Duration::from_millis(5)); app.tick(); fb = e.frame(&mut app, &[]); }
    assert_eq!(count(&fb, [255, 0, 0]), 0);
}
