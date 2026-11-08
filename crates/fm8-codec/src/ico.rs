//! Windows ICO/CUR: picks the largest image; PNG or DIB payloads.
use crate::Image;

pub fn decode(b: &[u8]) -> Result<Image, String> {
    let n = u16::from_le_bytes([*b.get(4).ok_or("ico: short")?, *b.get(5).ok_or("ico: short")?]) as usize;
    let mut best: Option<(usize, usize, usize)> = None; // (pixels, offset, size)
    for i in 0..n {
        let e = b.get(6 + i * 16..6 + i * 16 + 16).ok_or("ico: short dir")?;
        let (mut w, mut h) = (e[0] as usize, e[1] as usize);
        if w == 0 { w = 256 }
        if h == 0 { h = 256 }
        let size = u32::from_le_bytes(e[8..12].try_into().unwrap()) as usize;
        let off = u32::from_le_bytes(e[12..16].try_into().unwrap()) as usize;
        if best.map(|x| w * h > x.0).unwrap_or(true) { best = Some((w * h, off, size)) }
    }
    let (_, off, size) = best.ok_or("ico: empty")?;
    let d = b.get(off..off.saturating_add(size).min(b.len())).ok_or("ico: bad offset")?;
    if d.starts_with(b"\x89PNG") { crate::png::decode(d) } else { crate::bmp::decode_dib(d, u32::from_le_bytes(d.get(0..4).ok_or("ico: short")?.try_into().unwrap()) as usize + palette_bytes(d), true) }
}

fn palette_bytes(d: &[u8]) -> usize {
    let g = |o: usize, n: usize| d.get(o..o + n).map(|s| s.iter().rev().fold(0usize, |a, &x| (a << 8) | x as usize)).unwrap_or(0);
    let bpp = g(14, 2);
    let ncol = g(32, 4);
    if bpp <= 8 { 4 * if ncol > 0 { ncol } else { 1 << bpp } } else { 0 }
}
