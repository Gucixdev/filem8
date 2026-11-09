//! Trash view: listing (home + mount trash), restore (with name conflict), permanent
//! delete, empty trash, golden. Temp dirs only; trash roots are injected, the real
//! trash is never read.
use filem8::app::App;
use fm8_core::trashbin;
use fm8_ui::*;
use std::path::{Path, PathBuf};

struct Env { root: PathBuf, ui: Ui, t: f64 }
fn env(tag: &str) -> Env {
    let root = std::env::temp_dir().join(format!("fm8-trash-{tag}"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    Env { root, ui: Ui::new(Theme::dark()), t: 0.0 }
}
impl Env {
    fn frame(&mut self, app: &mut App, ev: &[Event]) -> soft::Framebuffer {
        self.t += 0.1;
        let t = self.t;
        render_frame(&mut self.ui, 1000, 600, 1.0, t, ev, |ui| app.build(ui))
    }
    fn key(&mut self, app: &mut App, k: Key, m: Mods) { self.frame(app, &[Event::Key(k, m)]); self.frame(app, &[]); }
}
impl Drop for Env { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); } }
fn quiet(app: &mut App) { for _ in 0..200 { app.tick(); if !app.tabs[app.active].model.loading { break; } std::thread::sleep(std::time::Duration::from_millis(5)); } }
fn none() -> Mods { Mods::default() }

fn golden(name: &str, f: &soft::Framebuffer) {
    let g = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(format!("{name}.png"));
    if std::env::var("UPDATE_GOLDEN").is_ok() || !g.exists() { std::fs::create_dir_all(g.parent().unwrap()).unwrap(); f.save_png(&g).unwrap(); }
    let gold = soft::Framebuffer::load_png(&g).unwrap();
    assert_eq!((gold.w, gold.h), (f.w, f.h));
    assert_eq!(f.diff(&gold, 3), 0, "{name}: differs from golden (UPDATE_GOLDEN=1 to refresh)");
}

fn fix_date(info: &Path, date: &str) {
    let t = std::fs::read_to_string(info).unwrap();
    let t: Vec<String> = t.lines().map(|l| if l.starts_with("DeletionDate=") { format!("DeletionDate={date}") } else { l.to_string() }).collect();
    std::fs::write(info, t.join("\n") + "\n").unwrap();
}

struct Setup { app: App, home_trash: PathBuf, mnt_trash: PathBuf, work: PathBuf }
fn setup(e: &Env) -> Setup {
    let work = e.root.join("work");
    let mnt = e.root.join("mnt");
    std::fs::create_dir_all(work.join("docs")).unwrap();
    std::fs::create_dir_all(mnt.join("data")).unwrap();
    std::fs::write(work.join("docs/report.txt"), "quarterly report").unwrap();
    std::fs::write(work.join("old.log"), vec![b'x'; 2048]).unwrap();
    std::fs::write(mnt.join("data/big.bin"), vec![1u8; 10_000]).unwrap();
    let (home_trash, mnt_trash) = (e.root.join("Trash"), mnt.join(".Trash-1000"));
    let a = trashbin::trash_in(&work.join("docs/report.txt"), &home_trash, None).unwrap();
    fix_date(&a.info, "2026-11-09T10:30:00");
    let b = trashbin::trash_in(&work.join("old.log"), &home_trash, None).unwrap();
    fix_date(&b.info, "2026-11-08T08:00:00");
    let c = trashbin::trash_in(&mnt.join("data/big.bin"), &mnt_trash, Some(&mnt)).unwrap();
    fix_date(&c.info, "2026-11-07T23:59:00");
    let mut app = App::new_in(work.clone(), Some(e.root.join("cfg")), Some(e.root.join("bm")), false);
    app.set_trash_roots(vec![(home_trash.clone(), None), (mnt_trash.clone(), Some(mnt))]);
    for _ in 0..200 { app.tick(); if !app.tabs[app.active].model.loading { break; } std::thread::sleep(std::time::Duration::from_millis(5)); }
    Setup { app, home_trash, mnt_trash, work }
}

#[test]
fn view_lists_both_trashes_and_golden() {
    let mut e = env("view");
    let mut s = setup(&e);
    s.app.goto(PathBuf::from("trash:///"), true);
    let f = e.frame(&mut s.app, &[]);
    e.frame(&mut s.app, &[]);
    assert_eq!(s.app.trash_rows(), 3, "home trash (2) + mount trash (1)");
    golden("trash_view", &f);
    // select the first row: highlighted state
    s.app.trash_select(&[0]);
    let f = e.frame(&mut s.app, &[]);
    golden("trash_selected", &f);
}

#[test]
fn restore_conflict_delete_and_empty() {
    let mut e = env("ops");
    let mut s = setup(&e);
    s.app.goto(PathBuf::from("trash:///"), true);
    e.frame(&mut s.app, &[]);
    // newest first: row 0 = docs/report.txt
    s.app.trash_select(&[0]);
    e.key(&mut s.app, Key::Char('r'), none());
    assert_eq!(std::fs::read_to_string(s.work.join("docs/report.txt")).unwrap(), "quarterly report");
    assert_eq!(s.app.trash_rows(), 2);

    // conflict: something new sits at the original path -> dialog, nothing overwritten
    std::fs::write(s.work.join("old.log"), "NEW").unwrap();
    s.app.trash_select(&[0]); // old.log
    e.key(&mut s.app, Key::Char('r'), none());
    assert!(s.app.trash_dialog_open(), "conflict must ask");
    assert_eq!(std::fs::read_to_string(s.work.join("old.log")).unwrap(), "NEW");
    e.key(&mut s.app, Key::Enter, none()); // restore under a new name
    assert!(!s.app.trash_dialog_open());
    assert_eq!(std::fs::read_to_string(s.work.join("old.log")).unwrap(), "NEW");
    assert_eq!(std::fs::read(s.work.join("old (restored).log")).unwrap().len(), 2048);
    assert_eq!(s.app.trash_rows(), 1);

    // delete permanently asks first; Escape keeps it, Enter deletes
    s.app.trash_select(&[0]);
    e.key(&mut s.app, Key::Delete, none());
    assert!(s.app.trash_dialog_open());
    e.key(&mut s.app, Key::Escape, none());
    assert_eq!(s.app.trash_rows(), 1);
    e.key(&mut s.app, Key::Delete, none());
    quiet(&mut s.app);
    e.frame(&mut s.app, &[]);
    let dlg = e.frame(&mut s.app, &[]);
    golden("trash_delete_dialog", &dlg);
    e.key(&mut s.app, Key::Enter, none());
    assert_eq!(s.app.trash_rows(), 0);
    assert!(std::fs::read_dir(s.mnt_trash.join("files")).unwrap().next().is_none());
    assert!(std::fs::read_dir(s.mnt_trash.join("info")).unwrap().next().is_none());

    // empty trash with confirmation (re-fill first)
    let g = trashbin::trash_in(&s.work.join("docs/report.txt"), &s.home_trash, None).unwrap();
    assert!(g.trashed.exists());
    fix_date(&g.info, "2026-11-09T12:00:00");
    s.app.trash_reload();
    assert_eq!(s.app.trash_rows(), 1);
    e.key(&mut s.app, Key::Delete, Mods { ctrl: true, shift: true, ..none() });
    assert!(s.app.trash_dialog_open());
    quiet(&mut s.app);
    e.frame(&mut s.app, &[]);
    let dlg = e.frame(&mut s.app, &[]);
    golden("trash_empty_dialog", &dlg);
    e.key(&mut s.app, Key::Enter, none());
    assert_eq!(s.app.trash_rows(), 0);
    assert!(trashbin::list_roots(&[(s.home_trash.clone(), None)]).is_empty());
    quiet(&mut s.app);
    e.frame(&mut s.app, &[]);
    let f = e.frame(&mut s.app, &[]);
    golden("trash_empty", &f);
}

#[test]
fn purge_cleans_directorysizes() {
    let e = env("purge");
    let tr = e.root.join("Trash");
    let d = e.root.join("dir");
    std::fs::create_dir_all(d.join("in")).unwrap();
    std::fs::write(d.join("in/f"), "1234").unwrap();
    let it = trashbin::trash_in(&d, &tr, None).unwrap();
    assert!(std::fs::read_to_string(tr.join("directorysizes")).unwrap().contains("dir"));
    trashbin::purge(&it).unwrap();
    assert!(!it.trashed.exists() && !it.info.exists());
    assert!(std::fs::read_to_string(tr.join("directorysizes")).unwrap().trim().is_empty());
    assert_eq!(trashbin::free_name(&e.root.join("a.txt")), e.root.join("a (restored).txt"));
}
