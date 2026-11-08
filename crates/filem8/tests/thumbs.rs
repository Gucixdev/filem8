//! Thumbnail loader: generation, info line, freedesktop disk cache, hover
//! frames. Uses a temp dir and a temp XDG_CACHE_HOME only.
use filem8::thumbs::Thumbs;
use fm8_core::thumbcache;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn wait<T>(t: &mut Thumbs, mut f: impl FnMut(&mut Thumbs) -> Option<T>) -> T {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(v) = f(t) { return v; }
        assert!(Instant::now() < end, "timed out");
        std::thread::sleep(Duration::from_millis(10));
        t.poll();
    }
}

#[test]
fn thumbs_generate_cache_and_animate() {
    let root = std::env::temp_dir().join(format!("fm8-thumbs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::env::set_var("XDG_CACHE_HOME", root.join("cache"));
    let png = root.join("pic one.png");
    let mut im = fm8_codec::Image::new(16, 8);
    for p in im.rgba.chunks_mut(4) { p.copy_from_slice(&[200, 30, 30, 255]); }
    std::fs::write(&png, fm8_codec::png::encode(&im)).unwrap();
    let stl = root.join("tri.stl");
    std::fs::write(&stl, "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid t\n").unwrap();

    let mut t = Thumbs::new();
    let p: PathBuf = png.clone();
    let th = wait(&mut t, |t| { t.poll(); t.get(&p) });
    assert_eq!(th.info, "16x8 PNG");
    assert_eq!((th.img.w, th.img.h), (16, 8));
    // disk cache written, spec naming, tEXt keys present
    let cf = thumbcache::cache_file(&png, false);
    assert!(cf.starts_with(root.join("cache/thumbnails/normal")), "{cf:?}");
    let b = std::fs::read(&cf).unwrap();
    let txt = fm8_codec::png::text_chunks(&b);
    assert!(txt.iter().any(|(k, v)| k == "Thumb::URI" && *v == thumbcache::uri(&png)));
    assert!(txt.iter().any(|(k, _)| k == "Thumb::MTime"));

    // a fresh loader is served from the cache (poison the cache pixel data
    // by replacing info text to prove the cached file is what is read)
    let mut cached = fm8_codec::png::decode(&b).unwrap();
    cached.rgba[0] = 7;
    let (uri, mt) = (thumbcache::uri(&png), txt.iter().find(|(k, _)| k == "Thumb::MTime").unwrap().1.clone());
    std::fs::write(&cf, fm8_codec::png::encode_text(&cached, &[("Thumb::URI", &uri), ("Thumb::MTime", &mt), ("Fm8::Info", "from cache")])).unwrap();
    let mut t2 = Thumbs::new();
    let th2 = wait(&mut t2, |t| { t.poll(); t.get(&p) });
    assert_eq!(th2.info, "from cache");
    // stale mtime -> regenerated
    std::fs::write(&cf, fm8_codec::png::encode_text(&cached, &[("Thumb::URI", &uri), ("Thumb::MTime", "1"), ("Fm8::Info", "stale")])).unwrap();
    let mut t3 = Thumbs::new();
    assert_eq!(wait(&mut t3, |t| { t.poll(); t.get(&p) }).info, "16x8 PNG");

    // 3D model: info line + 16 rotation frames on hover
    let s: PathBuf = stl.clone();
    assert_eq!(wait(&mut t, |t| { t.poll(); t.get(&s) }).info, "1 tris");
    let fr = wait(&mut t, |t| { t.poll(); t.anim(&s) });
    assert_eq!(fr.len(), 16);
    let _ = std::fs::remove_dir_all(&root);
}
