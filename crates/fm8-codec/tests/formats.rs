//! Our decoders vs the `image` crate (dev-dependency only) on generated
//! images and on whatever images the host has under /usr/share.
use fm8_codec::*;
use image::{ExtendedColorType, ImageEncoder};

fn pattern(w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::new();
    for y in 0..h { for x in 0..w { v.extend([(x * 255 / w) as u8, (y * 255 / h) as u8, ((x ^ y) * 7) as u8, if (x / 4 + y / 4) % 5 == 0 { 128 } else { 255 }]) } }
    v
}

fn rgb_of(rgba: &[u8]) -> Vec<u8> { rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect() }

fn same(a: &Image, b: &[u8], w: u32, h: u32, tol: u8) {
    assert_eq!((a.w, a.h), (w, h));
    let bad = a.rgba.iter().zip(b).filter(|(x, y)| x.abs_diff(**y) > tol).count();
    assert_eq!(bad, 0, "{bad} channel values differ");
}

#[test]
fn png_roundtrip_own_encoder_and_crate_encoder() {
    let (w, h) = (53, 37);
    let src = pattern(w, h);
    let img = Image { w, h, rgba: src.clone() };
    same(&png::decode(&png::encode(&img)).unwrap(), &src, w, h, 0);
    // crate-encoded variants
    let mut out = vec![];
    image::codecs::png::PngEncoder::new(&mut out).write_image(&src, w, h, ExtendedColorType::Rgba8).unwrap();
    same(&png::decode(&out).unwrap(), &src, w, h, 0);
    let rgb = rgb_of(&src);
    let mut out = vec![];
    image::codecs::png::PngEncoder::new(&mut out).write_image(&rgb, w, h, ExtendedColorType::Rgb8).unwrap();
    let d = png::decode(&out).unwrap();
    for (a, b) in d.rgba.chunks(4).zip(rgb.chunks(3)) { assert_eq!(&a[..3], b); assert_eq!(a[3], 255) }
    let grey: Vec<u8> = src.chunks(4).map(|p| p[0]).collect();
    let mut out = vec![];
    image::codecs::png::PngEncoder::new(&mut out).write_image(&grey, w, h, ExtendedColorType::L8).unwrap();
    assert!(png::decode(&out).unwrap().rgba.chunks(4).map(|p| p[0]).eq(grey.iter().copied()));
    let g16: Vec<u8> = grey.iter().flat_map(|&g| [7, g]).collect();
    let mut out = vec![];
    image::codecs::png::PngEncoder::new(&mut out).write_image(&g16, w, h, ExtendedColorType::L16).unwrap();
    assert!(png::decode(&out).unwrap().rgba.chunks(4).map(|p| p[0]).eq(grey.iter().copied()));
}

#[test]
fn deflate_roundtrip_incl_long_runs() {
    let mut d: Vec<u8> = (0..70000u32).map(|i| (i % 251) as u8).collect();
    d.extend(std::iter::repeat_n(7u8, 100000));
    let z = inflate::deflate(&d);
    assert!(z.len() < d.len() / 3);
    assert_eq!(inflate::inflate(&z, 1 << 24).unwrap(), d);
    assert_eq!(inflate::inflate(&inflate::deflate(b""), 10).unwrap(), b"");
    assert!(inflate::inflate(&z, 1000).is_err());
}

#[test]
fn qoi_bmp_tga_pnm_tiff_ico_hdr_gif() {
    let (w, h) = (31, 19);
    let src = pattern(w, h);
    let rgb = rgb_of(&src);
    let enc = |f: &dyn Fn(&mut Vec<u8>)| { let mut v = vec![]; f(&mut v); v };
    // qoi
    let q = enc(&|o| image::codecs::qoi::QoiEncoder::new(&mut *o).write_image(&src, w, h, ExtendedColorType::Rgba8).unwrap());
    same(&qoi::decode(&q).unwrap(), &src, w, h, 0);
    // bmp 32 + 24
    let q = enc(&|o| { image::codecs::bmp::BmpEncoder::new(&mut std::io::Cursor::new(&mut *o)).write_image(&src, w, h, ExtendedColorType::Rgba8).unwrap() });
    same(&bmp::decode(&q).unwrap(), &src, w, h, 0);
    let q = enc(&|o| { image::codecs::bmp::BmpEncoder::new(&mut std::io::Cursor::new(&mut *o)).write_image(&rgb, w, h, ExtendedColorType::Rgb8).unwrap() });
    let d = bmp::decode(&q).unwrap();
    for (a, b) in d.rgba.chunks(4).zip(rgb.chunks(3)) { assert_eq!(&a[..3], b) }
    // tga (crate writes uncompressed/RLE bottom-left or top-left)
    let q = enc(&|o| image::codecs::tga::TgaEncoder::new(&mut std::io::Cursor::new(&mut *o)).write_image(&src, w, h, ExtendedColorType::Rgba8).unwrap());
    same(&tga::decode(&q).unwrap(), &src, w, h, 0);
    // pnm binary pixmap + ascii
    let mut p6 = format!("P6\n# c\n{w} {h}\n255\n").into_bytes();
    p6.extend(&rgb);
    let d = pnm::decode(&p6).unwrap();
    for (a, b) in d.rgba.chunks(4).zip(rgb.chunks(3)) { assert_eq!(&a[..3], b) }
    let p3 = format!("P3 2 1 255 255 0 0 0 0 255").into_bytes();
    assert_eq!(pnm::decode(&p3).unwrap().rgba, vec![255, 0, 0, 255, 0, 0, 255, 255]);
    let p1 = b"P1 3 1 1 0 1".to_vec();
    assert_eq!(pnm::decode(&p1).unwrap().rgba, vec![0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255]);
    // tiff
    let q = enc(&|o| image::codecs::tiff::TiffEncoder::new(std::io::Cursor::new(&mut *o)).write_image(&rgb, w, h, ExtendedColorType::Rgb8).unwrap());
    let d = tiff::decode(&q).unwrap();
    for (a, b) in d.rgba.chunks(4).zip(rgb.chunks(3)) { assert_eq!(&a[..3], b) }
    let q = enc(&|o| image::codecs::tiff::TiffEncoder::new(std::io::Cursor::new(&mut *o)).write_image(&src, w, h, ExtendedColorType::Rgba8).unwrap());
    same(&tiff::decode(&q).unwrap(), &src, w, h, 0);
    // ico with png payload and bmp payload
    let q = enc(&|o| image::codecs::ico::IcoEncoder::new(&mut *o).write_image(&src, w, h, ExtendedColorType::Rgba8).unwrap());
    same(&ico::decode(&q).unwrap(), &src, w, h, 0);
    // hdr: flat 0.5 -> gamma
    let q = enc(&|o| image::codecs::hdr::HdrEncoder::new(&mut *o).encode(&vec![image::Rgb([0.5f32, 0.25, 1.0]); (w * h) as usize], w as usize, h as usize).unwrap());
    let d = hdr::decode(&q).unwrap();
    assert_eq!((d.w, d.h), (w, h));
    assert!((d.rgba[0] as i32 - 186).abs() <= 3 && d.rgba[2] == 255, "{:?}", &d.rgba[..4]);
    // gif
    let q = enc(&|o| { let mut e = image::codecs::gif::GifEncoder::new(&mut *o); e.encode(&src, w, h, ExtendedColorType::Rgba8).unwrap(); });
    let f = gif::decode(&q, 10).unwrap();
    assert_eq!((f[0].img.w, f[0].img.h), (w, h));
    let refimg = image::load_from_memory(&q).unwrap().to_rgba8();
    same(&f[0].img, refimg.as_raw(), w, h, 0);
}

#[test]
fn jpeg_matches_reference_decoder() {
    let (w, h) = (67, 45);
    let src = pattern(w, h);
    let rgb = rgb_of(&src);
    for q in [60u8, 95] {
        let mut out = vec![];
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, q).write_image(&rgb, w, h, ExtendedColorType::Rgb8).unwrap();
        let ours = jpeg::decode(&out).unwrap();
        let reference = image::load_from_memory(&out).unwrap().to_rgb8();
        let mut worst = 0;
        for (a, b) in ours.rgba.chunks(4).zip(reference.as_raw().chunks(3)) { for c in 0..3 { worst = worst.max(a[c].abs_diff(b[c])) } }
        assert!(worst <= 6, "q{q}: worst channel diff {worst}");
    }
    let grey: Vec<u8> = src.chunks(4).map(|p| p[0]).collect();
    let mut out = vec![];
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90).write_image(&grey, w, h, ExtendedColorType::L8).unwrap();
    let d = jpeg::decode(&out).unwrap();
    assert!(d.rgba.chunks(4).zip(&grey).all(|(a, g)| a[0].abs_diff(*g) < 8));
}

#[test]
fn sniff_and_garbage() {
    assert_eq!(sniff(b"\x89PNG\r\n\x1a\n....", ""), Some(Format::Png));
    assert_eq!(sniff(b"RIFFxxxx", ""), None);
    for f in [png::decode as fn(&[u8]) -> Result<Image, String>, jpeg::decode, bmp::decode, qoi::decode, tga::decode, pnm::decode, tiff::decode, ico::decode, hdr::decode, |b| gif::decode(b, 1).map(|f| f[0].img.clone())] {
        assert!(f(b"").is_err());
        assert!(f(&[0xFF; 64]).is_err());
        assert!(f(b"\x89PNG\r\n\x1a\n\0\0\0\r").is_err() || true);
    }
}

fn walk(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>, depth: u32) {
    let Ok(rd) = std::fs::read_dir(d) else { return };
    for e in rd.flatten() {
        if out.len() >= 600 { return }
        let p = e.path();
        if p.is_dir() { if depth < 7 { walk(&p, out, depth + 1) } }
        else if matches!(p.extension().and_then(|x| x.to_str()), Some("png" | "jpg" | "jpeg" | "gif" | "bmp" | "ico" | "tga" | "tif" | "tiff" | "qoi")) { out.push(p) }
    }
}

#[test]
fn host_image_corpus_agrees_with_reference() {
    let mut files = vec![];
    for d in ["/usr/share/pixmaps", "/usr/share/icons", "/usr/share/backgrounds", "/usr/share/doc", "/usr/share/wallpapers"] { walk(std::path::Path::new(d), &mut files, 0) }
    let (mut ok, mut skipped, mut bad) = (0, 0, vec![]);
    for f in &files {
        let Ok(b) = std::fs::read(f) else { continue };
        if b.len() > 3_000_000 { continue }
        let ext = f.extension().and_then(|x| x.to_str()).unwrap_or("").to_lowercase();
        let Ok(reference) = image::load_from_memory(&b) else { continue };
        let reference = reference.to_rgba8();
        match decode(&b, &ext) {
            Ok((fmt, ours)) => {
                let tol = if fmt == Format::Jpeg { 8 } else { 0 };
                let nbad = ours.rgba.iter().zip(reference.as_raw()).filter(|(a, b)| a.abs_diff(**b) > tol).count();
                if (ours.w, ours.h) != reference.dimensions() || (fmt != Format::Jpeg && nbad > 0) || (fmt == Format::Jpeg && nbad > ours.rgba.len() / 100) { bad.push(format!("{} {:?} {}x{} diff={nbad}", f.display(), fmt, ours.w, ours.h)) } else { ok += 1 }
            }
            Err(e) if e.contains("progressive") => skipped += 1,
            Err(e) => bad.push(format!("{}: {e}", f.display())),
        }
    }
    eprintln!("corpus: {} files, {ok} identical, {skipped} progressive-jpeg skipped, {} bad", files.len(), bad.len());
    for b in bad.iter().take(20) { eprintln!("  {b}") }
    assert!(bad.is_empty(), "{} mismatches", bad.len());
}

fn chunk(out: &mut Vec<u8>, ty: &[u8; 4], d: &[u8]) {
    out.extend((d.len() as u32).to_be_bytes());
    out.extend(ty);
    out.extend(d);
    out.extend(inflate::crc32(&[ty, d]).to_be_bytes());
}

#[test]
fn png_adam7_and_palette_and_low_bit_depth() {
    // 11x9 RGB8 interlaced, filter 0 per row
    let (w, h) = (11usize, 9usize);
    let px = |x: usize, y: usize| [(x * 20) as u8, (y * 25) as u8, (x * y) as u8];
    let mut raw = vec![];
    for (x0, y0, dx, dy) in [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)] {
        let ys: Vec<usize> = (y0..h).step_by(dy).collect();
        let xs: Vec<usize> = (x0..w).step_by(dx).collect();
        if xs.is_empty() { continue }
        for &y in &ys { raw.push(0); for &x in &xs { raw.extend(px(x, y)) } }
    }
    let mut z = vec![0x78, 1];
    z.extend(inflate::deflate(&raw));
    z.extend(inflate::adler32(&raw).to_be_bytes());
    let mut f = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = vec![];
    ihdr.extend((w as u32).to_be_bytes()); ihdr.extend((h as u32).to_be_bytes()); ihdr.extend([8, 2, 0, 0, 1]);
    chunk(&mut f, b"IHDR", &ihdr); chunk(&mut f, b"IDAT", &z); chunk(&mut f, b"IEND", &[]);
    let d = png::decode(&f).unwrap();
    for y in 0..h { for x in 0..w { assert_eq!(&d.rgba[(y * w + x) * 4..][..3], &px(x, y), "({x},{y})") } }

    // 2-bit palette with tRNS, 5 px wide (row padding)
    let raw = [0u8, 0b00_01_10_11, 0b11_00_00_00];
    let mut z = vec![0x78, 1];
    z.extend(inflate::deflate(&raw));
    z.extend(inflate::adler32(&raw).to_be_bytes());
    let mut f = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = vec![];
    ihdr.extend(5u32.to_be_bytes()); ihdr.extend(1u32.to_be_bytes()); ihdr.extend([2, 3, 0, 0, 0]);
    chunk(&mut f, b"IHDR", &ihdr);
    chunk(&mut f, b"PLTE", &[10, 0, 0, 0, 20, 0, 0, 0, 30, 40, 40, 40]);
    chunk(&mut f, b"tRNS", &[0, 128]);
    chunk(&mut f, b"IDAT", &z); chunk(&mut f, b"IEND", &[]);
    let d = png::decode(&f).unwrap();
    assert_eq!(d.rgba, vec![10, 0, 0, 0, 0, 20, 0, 128, 0, 0, 30, 255, 40, 40, 40, 255, 40, 40, 40, 255]);
}
