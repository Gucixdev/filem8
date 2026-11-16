//! Hostile input: decoders and archive readers must return Err (or a short
//! result), never panic, hang or ask for absurd memory. Seeds come from the
//! `image` crate, mutations from a fixed xorshift so runs are repeatable.
use fm8_codec::*;
use image::{ExtendedColorType, ImageEncoder};
use std::panic::catch_unwind;

fn pattern(w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::new();
    for y in 0..h { for x in 0..w { v.extend([(x * 255 / w) as u8, (y * 255 / h) as u8, ((x ^ y) * 7) as u8, if (x + y) % 5 == 0 { 128 } else { 255 }]) } }
    v
}

fn seeds() -> Vec<(&'static str, Vec<u8>)> {
    let (w, h) = (13, 9);
    let src = pattern(w, h);
    let rgb: Vec<u8> = src.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let enc = |f: &dyn Fn(&mut Vec<u8>)| { let mut v = vec![]; f(&mut v); v };
    let mut s = vec![
        ("png", png::encode(&Image { w, h, rgba: src.clone() })),
        ("qoi", enc(&|o| image::codecs::qoi::QoiEncoder::new(&mut *o).write_image(&src, w, h, ExtendedColorType::Rgba8).unwrap())),
        ("bmp", enc(&|o| image::codecs::bmp::BmpEncoder::new(&mut std::io::Cursor::new(&mut *o)).write_image(&src, w, h, ExtendedColorType::Rgba8).unwrap())),
        ("tga", enc(&|o| image::codecs::tga::TgaEncoder::new(&mut std::io::Cursor::new(&mut *o)).write_image(&src, w, h, ExtendedColorType::Rgba8).unwrap())),
        ("tiff", enc(&|o| image::codecs::tiff::TiffEncoder::new(std::io::Cursor::new(&mut *o)).write_image(&rgb, w, h, ExtendedColorType::Rgb8).unwrap())),
        ("ico", enc(&|o| image::codecs::ico::IcoEncoder::new(&mut *o).write_image(&src, w, h, ExtendedColorType::Rgba8).unwrap())),
        ("hdr", enc(&|o| image::codecs::hdr::HdrEncoder::new(&mut *o).encode(&vec![image::Rgb([0.5f32, 0.25, 1.0]); (w * h) as usize], w as usize, h as usize).unwrap())),
        ("gif", enc(&|o| { let mut e = image::codecs::gif::GifEncoder::new(&mut *o); e.encode(&src, w, h, ExtendedColorType::Rgba8).unwrap(); })),
        ("jpg", enc(&|o| image::codecs::jpeg::JpegEncoder::new_with_quality(&mut *o, 80).write_image(&rgb, w, h, ExtendedColorType::Rgb8).unwrap())),
    ];
    let mut p6 = format!("P6\n{w} {h}\n255\n").into_bytes();
    p6.extend(&rgb);
    s.push(("pnm", p6));
    s.push(("pnm", b"P3 2 1 255 255 0 0 0 0 255".to_vec()));
    // palette TGA, RLE BMP-ish and a tar/zip so the archive readers get the same treatment
    let mut tga = vec![0u8, 1, 1, 0, 0, 4, 0, 24, 0, 0, 0, 0, 2, 0, 2, 0, 8, 0x20];
    tga.extend([0u8; 12]);
    tga.extend([0, 1, 2, 3]);
    s.push(("tga", tga));
    s
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0 }
}

fn mutate(seed: &[u8], r: &mut Rng) -> Vec<u8> {
    let mut b = seed.to_vec();
    for _ in 0..1 + r.next() % 4 {
        if b.is_empty() { break }
        let i = (r.next() % b.len() as u64) as usize;
        match r.next() % 6 {
            0 => b[i] = r.next() as u8,
            1 => b[i] ^= 1 << (r.next() % 8),
            2 => b[i] = [0, 0xFF, 0x7F, 0x80, 1][(r.next() % 5) as usize],
            3 => b.truncate(i),
            4 => { let n = (r.next() % 4) as usize + 1; for k in 0..n { if i + k < b.len() { b[i + k] = 0xFF } } }
            _ => { b.insert(i, r.next() as u8); }
        }
    }
    b
}

/// Dimensions stay small so a mutated header cannot make the test itself eat gigabytes:
/// what is checked here is "no panic", the size caps have their own test below.
fn small(b: &[u8], ext: &str) -> bool {
    match dimensions_cheap(b, ext) { Some((w, h)) => (w as u64) * (h as u64) <= 1 << 16, None => true }
}

fn dimensions_cheap(b: &[u8], ext: &str) -> Option<(u32, u32)> {
    let le16 = |o: usize| Some(u16::from_le_bytes([*b.get(o)?, *b.get(o + 1)?]) as u32);
    let be16 = |o: usize| Some(u16::from_be_bytes([*b.get(o)?, *b.get(o + 1)?]) as u32);
    match sniff(b, ext)? {
        Format::Png | Format::Gif | Format::Bmp | Format::Qoi => dimensions(b, ext).map(|x| (x.1, x.2)),
        Format::Tga => Some((le16(12)?, le16(14)?)),
        Format::Jpeg => { let i = b.windows(2).position(|w| w == [0xFF, 0xC0] || w == [0xFF, 0xC1])?; Some((be16(i + 7)?, be16(i + 5)?)) }
        _ => None,
    }
}

#[test]
fn mutated_images_never_panic() {
    let mut r = Rng(0x9E3779B97F4A7C15);
    let mut bad = vec![];
    for (ext, seed) in seeds() {
        for n in 0..4000 {
            let m = mutate(&seed, &mut r);
            if !small(&m, ext) { continue }
            let m2 = m.clone();
            if catch_unwind(move || { let _ = decode(&m2, ext); let _ = gif::decode(&m2, 4); }).is_err() {
                bad.push(format!("{ext} #{n}"));
                let dir = std::env::var("FM8_FUZZ_OUT").unwrap_or_default();
                if !dir.is_empty() { std::fs::write(format!("{dir}/{ext}-{n}.bin"), &m).unwrap() }
            }
        }
    }
    assert!(bad.is_empty(), "{} panics: {:?}", bad.len(), &bad[..bad.len().min(20)]);
}

fn archive_seeds() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    use std::io::Write;
    let files: [(&str, &[u8]); 3] = [("a.txt", b"hello hello hello hello"), ("dir/b.bin", &[7u8; 700]), ("empty", b"")];
    let mut tar = vec![];
    {
        let mut t = ::tar::Builder::new(&mut tar);
        for (n, d) in files { let mut h = ::tar::Header::new_gnu(); h.set_size(d.len() as u64); h.set_mode(0o644); t.append_data(&mut h, n, d).unwrap(); }
        t.finish().unwrap();
    }
    let mut zip = std::io::Cursor::new(vec![]);
    {
        let mut w = ::zip::ZipWriter::new(&mut zip);
        for (i, (n, d)) in files.iter().enumerate() {
            let m = if i % 2 == 0 { ::zip::CompressionMethod::Deflated } else { ::zip::CompressionMethod::Stored };
            w.start_file(*n, ::zip::write::SimpleFileOptions::default().compression_method(m)).unwrap();
            w.write_all(d).unwrap();
        }
        w.finish().unwrap();
    }
    let mut gz = flate2::write::GzEncoder::new(vec![], flate2::Compression::default());
    gz.write_all(&tar).unwrap();
    (tar, zip.into_inner(), gz.finish().unwrap())
}

#[test]
fn mutated_archives_never_panic() {
    let (tar, zip, gz) = archive_seeds();
    let mut r = Rng(0xD1B54A32D192ED03);
    let mut bad = 0;
    for _ in 0..4000 {
        let (t, z, g) = (mutate(&tar, &mut r), mutate(&zip, &mut r), mutate(&gz, &mut r));
        bad += catch_unwind(move || {
            let (es, _) = tar::list(&t, 50);
            for e in &es { let _ = tar::read(&t, e); }
            if let Ok((es, _)) = zip::list(&z, 50) { for e in &es { let _ = zip::read(&z, e, 1 << 20); } }
            let _ = inflate::gunzip_prefix(&g, 1 << 20);
            let _ = inflate::zlib(&g, 1 << 20);
            let _ = inflate::inflate(g.get(10..).unwrap_or(&[]), 1 << 20);
        }).is_err() as u32;
    }
    assert_eq!(bad, 0, "panics on mutated archives");
}

/// Headers that claim far more pixels than the file holds are refused before anything is allocated
/// (a 30-byte .gif or .tga used to ask for 17 GiB).
#[test]
fn absurd_sizes_are_refused() {
    let mut gif = b"GIF89a\xff\xff\xff\xff\x00\x00\x00".to_vec();
    gif.extend(b"\x2c\0\0\0\0\x01\0\x01\0\0\x02\x02\x44\x01\0\x3b");
    assert!(gif::decode(&gif, 4).is_err());
    let mut tga = vec![0u8; 18];
    tga[2] = 2; tga[12..16].copy_from_slice(&[0xff, 0xff, 0xff, 0xff]); tga[16] = 32;
    assert!(tga::decode(&tga).is_err());
    assert!(hdr::decode(b"#?RADIANCE\n\n-Y 4294967296 +X 4294967296\n\0\0\0\0").is_err());
    // TIFF 16384x16384 RGBA16 in 100 bytes: the 2 GiB row buffer is never filled
    assert!(tiff::decode(&tiff(&[(256, 16384), (257, 16384), (258, 16), (277, 4), (262, 2), (273, 8), (279, 1)])).is_err());
}

/// Minimal little-endian TIFF with SHORT/LONG tags holding one value each.
fn tiff(tags: &[(u16, u32)]) -> Vec<u8> {
    let mut b = b"II*\0\x08\0\0\0".to_vec();
    b.extend((tags.len() as u16).to_le_bytes());
    for &(t, v) in tags { b.extend(t.to_le_bytes()); b.extend(4u16.to_le_bytes()); b.extend(1u32.to_le_bytes()); b.extend(v.to_le_bytes()); }
    b.extend([0; 8]);
    b
}

#[test]
fn malformed_headers_are_errors_not_panics() {
    // TGA palette entry sizes the decoder has no layout for
    for bits in [0u8, 4, 12, 20] {
        let mut t = vec![0u8, 1, 1, 0, 0, 2, 0, bits, 0, 0, 0, 0, 1, 0, 1, 0, 8, 0];
        t.extend([0u8; 16]);
        assert!(tga::decode(&t).is_err(), "palette bits {bits}");
    }
    // BMP BITFIELDS with a 32-bit wide mask (divided by zero in release, overflowed in debug)
    let mut bmp = vec![0u8; 14 + 56 + 4];
    bmp[0] = b'B'; bmp[1] = b'M'; bmp[10] = 70;
    bmp[14] = 56; bmp[18] = 1; bmp[22] = 1; bmp[26] = 1; bmp[28] = 32; bmp[30] = 3;
    bmp[54..58].copy_from_slice(&[0xff; 4]);
    bmp[70..74].copy_from_slice(&[0xff; 4]);
    let i = bmp::decode(&bmp).unwrap();
    assert_eq!(i.rgba[0], 255);
    // TIFF: zero samples per pixel, empty BitsPerSample
    assert!(tiff::decode(&tiff(&[(256, 1), (257, 1), (258, 8), (277, 0), (273, 8), (279, 1)])).is_err());
    let mut t = tiff(&[(256, 1), (257, 1), (258, 8), (273, 8), (279, 1)]);
    t[10 + 2 * 12 + 4] = 0; // BitsPerSample count = 0
    assert!(tiff::decode(&t).is_err());
    // JPEG segments shorter than their fixed fields
    for m in [0xDDu8, 0xC0, 0xC1] { assert!(jpeg::decode(&[0xFF, 0xD8, 0xFF, m, 0, 2, 0xFF, 0xD9]).is_err()); }
    let mut j = vec![0xFF, 0xD8, 0xFF, 0xC0, 0, 11, 8, 0, 1, 0, 1, 1, 1, 0x11, 0];
    j.extend([0xFF, 0xDA, 0, 2]);
    assert!(jpeg::decode(&j).is_err());
}

fn tar_header(name: &str, ty: u8, size_field: &[u8; 12]) -> Vec<u8> {
    let mut h = vec![0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    h[100..107].copy_from_slice(b"0000644");
    h[124..136].copy_from_slice(size_field);
    h[156] = ty;
    h[257..263].copy_from_slice(b"ustar\0");
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|&x| x as u32).sum();
    h[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
    h
}

#[test]
fn tar_size_that_wraps_ends_the_listing() {
    // base-256 size 2^64-512: header + size wrapped back to offset 0 and the listing never ended
    let mut size = [0xffu8; 12];
    size[0] = 0x80; size[10] = 0xfe; size[11] = 0;
    let mut t = tar_header("pax_global_header", b'g', &size);
    t.extend([0u8; 1024]);
    let (es, more) = tar::list(&t, 10);
    assert!(es.is_empty() && !more);
    let e = tar::Entry { size: u64::MAX, ..tar::list(&archive_seeds().0, 1).0.remove(0) };
    assert!(tar::read(&t, &e).is_none());
}

#[test]
fn tar_pax_size_moves_to_the_next_header() {
    // header size field 0, real size in the pax record (what tar writes for members over 8 GiB)
    let rec = b"13 size=600\n";
    let mut t = tar_header("PaxHeader", b'x', b"00000000014\0");
    t.extend(rec); t.resize(1024, 0);
    t.extend(tar_header("big", b'0', b"00000000000\0"));
    t.extend([b'A'; 600]); t.resize(1536 + 1024, 0);
    t.extend(tar_header("after", b'0', b"00000000000\0"));
    t.extend([0u8; 1024]);
    let (es, _) = tar::list(&t, 10);
    assert_eq!(es.iter().map(|e| (e.name.as_str(), e.size)).collect::<Vec<_>>(), [("big", 600), ("after", 0)]);
    assert_eq!(tar::read(&t, &es[0]).unwrap(), &[b'A'; 600][..]);
}

#[test]
fn zip64_offsets_past_the_file_are_errors() {
    let mut z = b"PK\x06\x06".to_vec();
    z.resize(56, 0);
    z[32..40].copy_from_slice(&1u64.to_le_bytes());
    z[48..56].copy_from_slice(&(u64::MAX - 1).to_le_bytes()); // central directory "at" 2^64-2
    z.extend(b"PK\x06\x07"); z.extend([0u8; 16]); // locator -> record at offset 0
    z.extend(b"PK\x05\x06"); z.extend([0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0, 0]);
    assert!(zip::list(&z, 10).is_err());
}
