//! Special files in a folder must not tie up the thumbnail workers.
use filem8::thumbs::Thumbs;
use std::time::{Duration, Instant};

extern "C" { fn mkfifo(path: *const std::ffi::c_char, mode: u32) -> i32; }

#[test]
fn fifos_named_like_images_do_not_block_the_workers() {
    let root = std::env::temp_dir().join(format!("fm8-thumbs-special-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::env::set_var("XDG_CACHE_HOME", root.join("cache"));
    let mut t = Thumbs::new();
    // more pipes than workers: each one used to block a worker in open() for good
    for i in 0..4 {
        let p = root.join(format!("pipe{i}.png"));
        let _ = std::fs::remove_file(&p);
        let c = std::ffi::CString::new(p.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { mkfifo(c.as_ptr(), 0o600) }, 0);
        assert!(t.get(&p).is_none());
    }
    let png = root.join("real.png");
    std::fs::write(&png, fm8_codec::png::encode(&fm8_codec::Image::new(8, 8))).unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    let th = loop {
        t.poll();
        if let Some(th) = t.get(&png) { break th; }
        assert!(Instant::now() < end, "thumbnail workers are stuck");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!((th.img.w, th.img.h), (8, 8));
    while t.busy() {
        assert!(Instant::now() < end, "still busy: a job never came back");
        std::thread::sleep(Duration::from_millis(10));
        t.poll();
    }
}
