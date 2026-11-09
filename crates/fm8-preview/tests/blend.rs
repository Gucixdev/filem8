use fm8_preview::{blend, Preview};
use std::io::Cursor;

fn thumb_payload(w: i32, h: i32, le: bool) -> Vec<u8> {
    let f = |v: i32| if le { v.to_le_bytes() } else { v.to_be_bytes() };
    let mut d = vec![];
    d.extend(f(w)); d.extend(f(h));
    // bottom row first: row 0 (stored first) = red, last stored row = blue
    for y in 0..h { for _ in 0..w { d.extend(if y == 0 { [255, 0, 0, 255] } else if y == h - 1 { [0, 0, 255, 255] } else { [0, 255, 0, 255] }); } }
    d
}

/// Legacy header: code, size, old ptr (4 or 8), sdna, count.
fn legacy(ptr: usize, le: bool, with_thumb: bool) -> Vec<u8> {
    let f = |v: i32| if le { v.to_le_bytes() } else { v.to_be_bytes() };
    let mut b = format!("BLENDER{}{}404", if ptr == 8 { '-' } else { '_' }, if le { 'v' } else { 'V' }).into_bytes();
    let mut block = |b: &mut Vec<u8>, code: &[u8; 4], data: &[u8]| {
        b.extend(code); b.extend(f(data.len() as i32)); b.extend(vec![0u8; ptr]); b.extend(f(0)); b.extend(f(1)); b.extend(data);
    };
    block(&mut b, b"REND", &[0; 8]);
    if with_thumb { block(&mut b, b"TEST", &thumb_payload(4, 3, le)); }
    block(&mut b, b"SC\0\0", &[0; 16]);
    block(&mut b, b"OB\0\0", &[0; 16]);
    block(&mut b, b"OB\0\0", &[0; 16]);
    block(&mut b, b"ENDB", &[]);
    b
}

fn check(i: &blend::BlendInfo) {
    assert_eq!(i.version, "4.4");
    assert_eq!((i.objects, i.scenes), (2, 1));
    let t = i.thumb.as_ref().unwrap();
    assert_eq!((t.w, t.h), (4, 3));
    // flipped: the stored last row (blue) is now the top row
    assert_eq!(&t.rgba[..4], &[0, 0, 255, 255]);
    assert_eq!(&t.rgba[(2 * 4) * 4..(2 * 4) * 4 + 4], &[255, 0, 0, 255]);
}

#[test]
fn legacy_variants() {
    for ptr in [4, 8] { for le in [true, false] { check(&blend::parse(&mut Cursor::new(legacy(ptr, le, true))).unwrap()); } }
}

#[test]
fn large_header_v1() {
    let mut b = b"BLENDER17-01v0501".to_vec();
    let mut block = |b: &mut Vec<u8>, code: &[u8; 4], data: &[u8]| {
        b.extend(code); b.extend(0i32.to_le_bytes()); b.extend(0u64.to_le_bytes()); b.extend((data.len() as i64).to_le_bytes()); b.extend(1i64.to_le_bytes()); b.extend(data);
    };
    block(&mut b, b"TEST", &thumb_payload(4, 3, true));
    block(&mut b, b"OB\0\0", &[0; 8]);
    block(&mut b, b"ENDB", &[]);
    let i = blend::parse(&mut Cursor::new(b)).unwrap();
    assert_eq!(i.version, "5.1");
    assert_eq!(i.objects, 1);
    assert_eq!(i.thumb.unwrap().w, 4);
}

#[test]
fn no_thumbnail_and_garbage() {
    let i = blend::parse(&mut Cursor::new(legacy(8, true, false))).unwrap();
    assert!(i.thumb.is_none());
    assert_eq!(i.objects, 2);
    assert!(blend::parse(&mut Cursor::new(b"PK\x03\x04 not blender".to_vec())).is_err());
    // truncated mid-block must not panic
    let b = legacy(8, true, true);
    let _ = blend::parse(&mut Cursor::new(b[..40].to_vec()));
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("fm8-blend-{}-{name}", std::process::id()));
    let _ = std::fs::create_dir_all(&d);
    d
}

#[test]
fn compressed_files() {
    let d = tmp("c");
    let raw = legacy(8, true, true);
    // gzip: stored via our own deflate + crc32
    let mut gz = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255];
    gz.extend(fm8_codec::inflate::deflate(&raw));
    gz.extend(fm8_codec::inflate::crc32(&[&raw]).to_le_bytes());
    gz.extend((raw.len() as u32).to_le_bytes());
    let g = d.join("a.blend");
    std::fs::write(&g, gz).unwrap();
    check(&blend::read_file(&g).unwrap());
    // preview wraps it as an image with a description
    match blend::load(&g) { Preview::Image(im) => assert!(im.format.contains("Blender 4.4, 2 object")), o => panic!("{o:?}") }
    // zstd through the tool, only when installed
    if std::process::Command::new("zstd").arg("-V").output().is_ok() {
        let plain = d.join("raw.bin");
        std::fs::write(&plain, &raw).unwrap();
        let z = d.join("z.blend");
        assert!(std::process::Command::new("zstd").args(["-q", "-f", "-o"]).arg(&z).arg(&plain).status().unwrap().success());
        check(&blend::read_file(&z).unwrap());
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn real_blender_if_present() {
    if std::process::Command::new("blender").arg("--version").output().is_err() { return; }
    let d = tmp("real");
    let f = d.join("t.blend");
    let st = std::process::Command::new("blender").args(["-b", "--factory-startup", "--python-expr"])
        .arg(format!("import bpy; bpy.ops.wm.save_as_mainfile(filepath={:?})", f.to_string_lossy())).output().unwrap();
    if st.status.success() && f.exists() {
        let i = blend::read_file(&f).unwrap();
        assert!(i.objects >= 1 && i.scenes >= 1, "{i:?}");
    }
    let _ = std::fs::remove_dir_all(&d);
}
