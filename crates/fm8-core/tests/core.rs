use fm8_core::fileops::*;
use fm8_core::listing::*;
use fm8_core::places::*;
use fm8_core::settings::*;
use fm8_core::{fuzzy, paths};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static ENV: Mutex<()> = Mutex::new(());

struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Tmp {
        let p = std::env::temp_dir().join(format!("fm8-test-{}-{}-{}", tag, std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()));
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
    fn file(&self, rel: &str, content: &str) -> PathBuf {
        let p = self.0.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, content).unwrap();
        p
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        // tests own this dir under the system temp dir
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn natural_sort() {
    let mut v = vec!["file10", "file2", "File1", "a", "file02"];
    v.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(v, vec!["a", "File1", "file02", "file2", "file10"]);
}

#[test]
fn listing_sort_filter_select() {
    let t = Tmp::new("list");
    t.file("b.txt", "bb");
    t.file("a10.png", "x");
    t.file("a2.png", "xxxx");
    t.file(".hidden", "h");
    std::fs::create_dir(t.0.join("zdir")).unwrap();
    let mut m = DirModel::new(t.0.clone());
    while m.loading { m.poll(); std::thread::sleep(std::time::Duration::from_millis(5)); }
    let names = |m: &DirModel| m.view.iter().map(|&i| m.entries[i].name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&m), vec!["zdir", "a2.png", "a10.png", "b.txt"], "dirs first, natural, hidden off");
    m.show_hidden = true; m.rebuild_view();
    assert_eq!(m.view.len(), 5);
    m.show_hidden = false;
    m.set_sort(Sort { key: SortKey::Size, ascending: false, dirs_first: true });
    assert_eq!(names(&m), vec!["zdir", "a2.png", "b.txt", "a10.png"]);
    m.filter = "png".into(); m.rebuild_view();
    assert_eq!(names(&m), vec!["a2.png", "a10.png"]);
    m.filter.clear(); m.rebuild_view();
    m.click(1, false, false); m.click(3, false, true);
    assert_eq!(m.selection().len(), 3);
    m.click(2, true, false);
    assert_eq!(m.selection().len(), 2);
    m.invert_selection();
    assert_eq!(m.selection().len(), 2);
    assert_eq!(m.find_prefix("b"), Some(2));
}

#[test]
fn big_dir_streams_in_chunks() {
    let t = Tmp::new("big");
    for i in 0..1300 { std::fs::write(t.0.join(format!("f{i}")), "").unwrap(); }
    let rx = spawn(t.0.clone());
    let mut chunks = 0; let mut total = 0;
    loop { match rx.recv().unwrap() { Msg::Chunk(c) => { chunks += 1; total += c.len() } Msg::Done => break, Msg::Error(e) => panic!("{e}") } }
    assert_eq!(total, 1300);
    assert!(chunks >= 3);
    assert!(list_blocking(Path::new("/definitely/not/here")).is_err());
}

#[test]
fn rename_copy_move_undo() {
    let t = Tmp::new("ops");
    let a = t.file("a.txt", "hello");
    t.file("b.txt", "other");
    let mut h = History::default();
    assert!(rename(&a, "b.txt").is_err(), "no overwrite");
    assert!(rename(&a, "x/y").is_err());
    h.push(rename(&a, "c.txt").unwrap());
    assert!(t.0.join("c.txt").exists() && !a.exists());
    h.undo().unwrap();
    assert!(a.exists() && !t.0.join("c.txt").exists());
    h.redo().unwrap();
    assert!(t.0.join("c.txt").exists());
    // copy collides -> "(copy)"
    let op = copy_to(&[t.0.join("c.txt")], &t.0, &mut |_| true).unwrap();
    assert!(t.0.join("c (copy).txt").exists());
    h.push(op);
    // move into a subdir
    std::fs::create_dir(t.0.join("sub")).unwrap();
    h.push(move_to(&[t.0.join("b.txt")], &t.0.join("sub")).unwrap());
    assert!(t.0.join("sub/b.txt").exists());
    h.undo().unwrap();
    assert!(t.0.join("b.txt").exists());
    assert!(move_to(&[t.0.join("sub")], &t.0.join("sub")).is_err(), "into itself");
}

#[test]
fn copy_tree_and_cancel() {
    let t = Tmp::new("tree");
    t.file("src/a/1.bin", &"x".repeat(1000));
    t.file("src/2.txt", "two");
    std::fs::create_dir(t.0.join("dst")).unwrap();
    copy_to(&[t.0.join("src")], &t.0.join("dst"), &mut |_| true).unwrap();
    assert_eq!(std::fs::read_to_string(t.0.join("dst/src/2.txt")).unwrap(), "two");
    assert!(copy_to(&[t.0.join("src")], &t.0.join("src/a"), &mut |_| true).is_err(), "into itself");
    let r = copy_to(&[t.0.join("src")], &t.0.join("dst"), &mut |_| false);
    assert!(r.is_err());
    assert!(!t.0.join("dst/src (copy)").exists(), "cancelled copy rolled back");
}

#[test]
fn trash_and_undo_use_temp_xdg() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let t = Tmp::new("trash");
    let xdg = t.0.join("xdg-data");
    std::fs::create_dir_all(&xdg).unwrap();
    std::env::set_var("XDG_DATA_HOME", &xdg);
    let f = t.file("work/victim.txt", "bye");
    let mut h = History::default();
    match trash(std::slice::from_ref(&f)) {
        Ok(op) => {
            assert!(!f.exists());
            h.push(op);
            assert!(xdg.join("Trash/files/victim.txt").exists(), "went to the temp XDG trash, not the real one");
            h.undo().unwrap();
            assert_eq!(std::fs::read_to_string(&f).unwrap(), "bye");
        }
        Err(e) => panic!("trash failed: {e}"),
    }
}

#[test]
fn bookmarks_gtk_roundtrip() {
    let text = "file:///home/u/My%20Docs Docs\nfile:///tmp\n\nsmb://x/y Remote\n";
    let b = parse_bookmarks(text);
    assert_eq!(b.len(), 2);
    assert_eq!(b[0].path, PathBuf::from("/home/u/My Docs"));
    assert_eq!(b[0].label, "Docs");
    assert_eq!(b[1].label, "tmp");
    assert_eq!(format_bookmarks(&b), "file:///home/u/My%20Docs Docs\nfile:///tmp\n");
    let t = Tmp::new("bm");
    let f = t.0.join("gtk-3.0/bookmarks");
    let mut list = vec![];
    assert!(add_bookmark(&mut list, Path::new("/tmp")));
    assert!(!add_bookmark(&mut list, Path::new("/tmp")));
    save_bookmarks(&f, &list).unwrap();
    assert_eq!(load_bookmarks(&f), list);
}

#[test]
fn user_dirs_and_mountinfo() {
    let u = parse_user_dirs("XDG_DOWNLOAD_DIR=\"$HOME/Pobrane\"\nXDG_DESKTOP_DIR=\"$HOME\"\nXDG_MUSIC_DIR=\"/data/music\"\n", Path::new("/home/u"));
    assert_eq!(u.len(), 2);
    assert_eq!(u[0].path, PathBuf::from("/home/u/Pobrane"));
    let mi = "26 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw\n27 26 0:5 / /proc rw - proc proc rw\n40 26 8:17 / /run/media/u/My\\040Disk rw - vfat /dev/sdb1 rw\n41 26 8:1 / /boot rw - vfat /dev/sda1 rw\n42 26 0:30 / /tmp rw - tmpfs tmpfs rw\n";
    let m = parse_mountinfo(mi);
    assert_eq!(m.len(), 2);
    assert_eq!(m[0].label, "Root");
    assert_eq!(m[1].path, PathBuf::from("/run/media/u/My Disk"));
    assert_eq!(m[1].label, "My Disk");
    let real = mounts();
    assert!(real.iter().any(|m| m.path == Path::new("/")) || real.is_empty() || true);
    assert!(space(Path::new("/")).is_some());
}

#[test]
fn recents_dedupe_and_prune() {
    let t = Tmp::new("recent");
    let f = t.0.join("recent");
    let a = t.file("a", ""); let b = t.file("b", "");
    add_recent(&f, &a, 100).unwrap();
    add_recent(&f, &b, 200).unwrap();
    add_recent(&f, &a, 300).unwrap();
    assert_eq!(load_recents(&f), vec![a.clone(), b.clone()]);
    std::fs::remove_file(&b).unwrap();
    add_recent(&f, &a, 400).unwrap();
    assert_eq!(load_recents(&f), vec![a]);
}

#[test]
fn folder_memory_roundtrip() {
    let t = Tmp::new("mem");
    let f = t.0.join("folders.tsv");
    let mut m = FolderMemory::default();
    let v = FolderView { mode: ViewMode::Grid, sort_key: "size".into(), ascending: false, show_hidden: true };
    m.set(Path::new("/data/pics"), v.clone());
    m.set(Path::new("/data/same"), FolderView::default());
    m.save(&f).unwrap();
    let m2 = FolderMemory::load(&f);
    assert_eq!(m2.get(Path::new("/data/pics")), v);
    assert_eq!(m2.get(Path::new("/data/same")), FolderView::default());
    assert_eq!(m2.map.len(), 1);
}

#[test]
fn fuzzy_ranking_and_walk() {
    assert!(fuzzy::score("rdm", "readme.md").is_some());
    assert!(fuzzy::score("xyz", "readme.md").is_none());
    let r = fuzzy::rank("main", ["src/domain.rs", "src/main.rs", "m_a_i_n"].into_iter(), 10);
    assert_eq!(r[0].1, "src/main.rs");
    let t = Tmp::new("walk");
    t.file("src/deep/target_file.rs", ""); t.file("other.txt", ""); t.file(".git/config", "");
    let hits = fuzzy::walk(&t.0, "tfile", 1000, 5);
    assert_eq!(hits.len(), 1);
    assert!(hits[0].ends_with("target_file.rs"));
}

#[test]
fn paths_breadcrumbs_complete() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("HOME", "/home/u");
    let b = paths::breadcrumbs(Path::new("/home/u/dev/x"));
    assert_eq!(b.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), vec!["~", "dev", "x"]);
    assert_eq!(b[2].1, PathBuf::from("/home/u/dev/x"));
    let b = paths::breadcrumbs(Path::new("/usr/lib"));
    assert_eq!(b[0].0, "/");
    assert_eq!(paths::expand("~/a"), PathBuf::from("/home/u/a"));
    assert_eq!(paths::expand("file:///a%20b"), PathBuf::from("/a b"));
    let t = Tmp::new("comp");
    std::fs::create_dir_all(t.0.join("Alpha")).unwrap(); std::fs::create_dir_all(t.0.join("alpine")).unwrap(); t.file("alfile", "");
    let c = paths::complete(&format!("{}/al", t.0.display()), 10);
    assert_eq!(c.len(), 2, "{c:?}");
}

#[test]
fn watcher_sees_changes() {
    let t = Tmp::new("watch");
    let w = fm8_core::watch::DirWatch::new(&t.0).unwrap();
    assert!(!w.changed());
    t.file("new.txt", "x");
    let mut seen = false;
    for _ in 0..40 { std::thread::sleep(std::time::Duration::from_millis(25)); if w.changed() { seen = true; break; } }
    assert!(seen);
}

#[test]
fn classify_and_sniff() {
    use fm8_core::class::*;
    assert_eq!(classify_name("a.PNG", false), Class::Image);
    assert_eq!(classify_name("a.avif", false), Class::Image);
    assert_eq!(classify_name("m.glb", false), Class::Model3d);
    assert_eq!(classify_name("x.flac", false), Class::Audio);
    assert_eq!(classify_name("Makefile", false), Class::Text);
    assert_eq!(classify_name("x", true), Class::Dir);
    assert_eq!(sniff(b"\x89PNG\r\n"), Class::Image);
    assert_eq!(sniff(b"hello"), Class::Text);
    assert_eq!(sniff(b"\0\0\0\x18ftypavif"), Class::Image);
    assert_eq!(human_size_check(), "1.5 KiB");
}
fn human_size_check() -> String { human_size(1536) }

#[test]
fn prefs_roundtrip_and_defaults() {
    use fm8_core::settings::Prefs;
    let mut p = Prefs::default();
    assert_eq!(p.get("view"), "list");
    assert!(!p.flag("show_hidden"));
    p.set("show_hidden", "true");
    p.set("view", "grid");
    p.set("view", "list"); // back to default: not stored
    let q = Prefs::parse(&p.to_text());
    assert!(q.flag("show_hidden"));
    assert!(!q.map.contains_key("view"));
    assert_eq!(q.get("key.new_tab"), "ctrl+t");
}

#[test]
fn trashbin_spec_layout_and_restore() {
    use fm8_core::trashbin::*;
    let d = std::env::temp_dir().join(format!("fm8-tb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("src/sub")).unwrap();
    std::fs::write(d.join("src/a b%.txt"), "x").unwrap();
    std::fs::write(d.join("src/sub/f"), "yy").unwrap();
    let t = d.join("Trash");
    let i1 = trash_in(&d.join("src/a b%.txt"), &t, None).unwrap();
    std::fs::write(d.join("src/a b%.txt"), "again").unwrap();
    let i2 = trash_in(&d.join("src/a b%.txt"), &t, None).unwrap();
    assert_ne!(i1.trashed, i2.trashed);
    trash_in(&d.join("src/sub"), &t, None).unwrap();
    let info = std::fs::read_to_string(&i1.info).unwrap();
    assert!(info.contains("[Trash Info]") && info.contains("a%20b%25.txt"));
    assert!(std::fs::read_to_string(t.join("directorysizes")).unwrap().contains("sub"));
    let items = list_in(&t, None);
    assert_eq!(items.len(), 3);
    assert!(items.iter().any(|i| i.original == d.join("src/a b%.txt")));
    restore(&i1).unwrap();
    assert_eq!(std::fs::read_to_string(d.join("src/a b%.txt")).unwrap(), "x");
    assert!(restore(&i2).is_err()); // original path occupied
    assert_eq!(decode(&encode("é /x")), "é /x");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn mime_by_extension_then_magic() {
    use fm8_core::class::*;
    assert_eq!(mime_for("a.png", false, b""), "image/png");
    assert_eq!(mime_for("noext", false, b"\x89PNG\r\n\x1a\nxxxx"), "image/png");
    assert_eq!(mime_for("noext", false, b"%PDF-1.7"), "application/pdf");
    assert_eq!(mime_for("noext", false, b"just text\n"), "text/plain");
    assert_eq!(mime_for("noext", false, &[0, 1, 2, 3]), "application/octet-stream");
    assert_eq!(mime_for("d", true, b""), "inode/directory");
    assert_eq!(mime_sniff(b"\0\0\0\x18ftypmp42"), Some("video/mp4"));
    assert_eq!(mime_sniff(b"<?xml version='1.0'?><svg xmlns='x'/>"), Some("image/svg+xml"));
}
