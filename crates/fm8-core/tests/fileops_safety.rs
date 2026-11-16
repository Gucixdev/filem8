//! File operations must not lose data or hang: redo never overwrites, copy handles FIFOs.
use fm8_core::fileops::{self, History};
use std::path::PathBuf;

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("fm8-safe-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn redo_never_overwrites_what_appeared_meanwhile() {
    let d = tmp("redo");
    let mut h = History::default();
    // new file, undo, something else creates the same name with content, redo
    h.push(fileops::new_file(&d, "note.txt").unwrap());
    h.undo().unwrap();
    std::fs::write(d.join("note.txt"), "precious").unwrap();
    assert!(h.redo().is_err());
    assert_eq!(std::fs::read_to_string(d.join("note.txt")).unwrap(), "precious");
    // rename a -> b, undo, another b appears, redo
    let mut h = History::default();
    std::fs::write(d.join("a"), "A").unwrap();
    h.push(fileops::rename(&d.join("a"), "b").unwrap());
    h.undo().unwrap();
    std::fs::write(d.join("b"), "other").unwrap();
    assert!(h.redo().is_err());
    assert_eq!(std::fs::read_to_string(d.join("b")).unwrap(), "other");
    assert_eq!(std::fs::read_to_string(d.join("a")).unwrap(), "A");
    // move into a folder, undo, same name appears there, redo
    let mut h = History::default();
    std::fs::create_dir(d.join("dst")).unwrap();
    std::fs::write(d.join("m"), "M").unwrap();
    h.push(fileops::move_to(&[d.join("m")], &d.join("dst")).unwrap());
    h.undo().unwrap();
    std::fs::write(d.join("dst/m"), "other").unwrap();
    assert!(h.redo().is_err());
    assert_eq!(std::fs::read_to_string(d.join("dst/m")).unwrap(), "other");
    assert_eq!(std::fs::read_to_string(d.join("m")).unwrap(), "M");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn copying_a_folder_with_a_fifo_does_not_block() {
    let d = tmp("fifo");
    std::fs::create_dir_all(d.join("src")).unwrap();
    std::fs::write(d.join("src/f"), "x").unwrap();
    let fifo = std::ffi::CString::new(d.join("src/pipe").to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    std::fs::create_dir(d.join("out")).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let d2 = d.clone();
    std::thread::spawn(move || { let _ = tx.send(fileops::copy_to(&[d2.join("src")], &d2.join("out"), &mut |_| true)); });
    let r = rx.recv_timeout(std::time::Duration::from_secs(5)).expect("copy blocked on the FIFO");
    // unblock the old code's reader so the test process can end, then check the result
    let _ = std::fs::OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open(d.join("src/pipe"));
    r.unwrap();
    use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
    assert!(std::fs::symlink_metadata(d.join("out/src/pipe")).unwrap().file_type().is_fifo());
    assert_eq!(std::fs::read_to_string(d.join("out/src/f")).unwrap(), "x");
    let _ = std::fs::remove_dir_all(&d);
}
