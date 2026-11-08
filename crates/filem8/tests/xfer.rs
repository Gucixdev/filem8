//! Async transfer worker: progress, completion, cancel. Temp dirs only.
use filem8::xfer::Transfer;
use std::time::{Duration, Instant};

fn finish(t: &Transfer) -> Result<fm8_core::fileops::Op, String> {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(r) = t.poll() { return r; }
        assert!(Instant::now() < end, "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn async_copy_progress_cancel_and_move() {
    let root = std::env::temp_dir().join(format!("fm8-xfer-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (src, dst) = (root.join("src"), root.join("dst"));
    std::fs::create_dir_all(src.join("d")).unwrap();
    std::fs::create_dir_all(&dst).unwrap();
    std::fs::write(src.join("a.bin"), vec![7u8; 3_000_000]).unwrap();
    std::fs::write(src.join("d/b.bin"), vec![8u8; 1_000_000]).unwrap();

    // cancelled before the first byte: error, nothing left behind
    let t = Transfer::start_held(vec![src.join("a.bin"), src.join("d")], dst.clone(), true);
    t.cancel();
    t.release();
    assert!(finish(&t).is_err());
    assert!(t.cancelled());
    assert_eq!(std::fs::read_dir(&dst).unwrap().count(), 0, "partial copy rolled back");

    // full copy: progress reaches total, content identical
    let t = Transfer::start(vec![src.join("a.bin"), src.join("d")], dst.clone(), true);
    let op = finish(&t).unwrap();
    assert!(matches!(op, fm8_core::fileops::Op::Copy { ref created } if created.len() == 2));
    assert_eq!(t.total.load(std::sync::atomic::Ordering::Relaxed), 4_000_000);
    assert_eq!(t.fraction(), 1.0);
    assert_eq!(std::fs::read(dst.join("d/b.bin")).unwrap().len(), 1_000_000);
    assert!(src.join("a.bin").exists(), "copy keeps the source");

    // move
    let dst2 = root.join("dst2");
    std::fs::create_dir_all(&dst2).unwrap();
    let t = Transfer::start(vec![src.join("a.bin")], dst2.clone(), false);
    finish(&t).unwrap();
    assert!(dst2.join("a.bin").exists() && !src.join("a.bin").exists());
    let _ = std::fs::remove_dir_all(&root);
}
