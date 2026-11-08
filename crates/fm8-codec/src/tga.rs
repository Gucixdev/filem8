//! Truevision TGA: colour-mapped, true-colour, grey; raw and RLE; 8/15/16/24/32 bit.
use crate::Image;

pub fn decode(b: &[u8]) -> Result<Image, String> {
    if b.len() < 18 { return Err("tga: short".into()) }
    let idlen = b[0] as usize;
    let (cmtype, ty) = (b[1], b[2]);
    let (cm_first, cm_len, cm_bits) = (u16::from_le_bytes([b[3], b[4]]) as usize, u16::from_le_bytes([b[5], b[6]]) as usize, b[7] as usize);
    let (w, h) = (u16::from_le_bytes([b[12], b[13]]) as usize, u16::from_le_bytes([b[14], b[15]]) as usize);
    let (bpp, desc) = (b[16] as usize, b[17]);
    if w == 0 || h == 0 { return Err("tga: bad size".into()) }
    let rle = ty >= 8;
    let kind = ty & 7;
    if !matches!(kind, 1 | 2 | 3) || !matches!(bpp, 8 | 15 | 16 | 24 | 32) { return Err("tga: unsupported".into()) }
    let mut p = 18 + idlen;
    let conv = |d: &[u8], bits: usize| -> [u8; 4] {
        match bits {
            8 => [d[0], d[0], d[0], 255],
            15 | 16 => { let v = u16::from_le_bytes([d[0], d[1]]); let e = |x: u16| ((x << 3) | (x >> 2)) as u8; [e((v >> 10) & 31), e((v >> 5) & 31), e(v & 31), if bits == 16 && v & 0x8000 == 0 && false { 0 } else { 255 }] }
            24 => [d[2], d[1], d[0], 255],
            _ => [d[2], d[1], d[0], d[3]],
        }
    };
    let mut pal = vec![];
    if cmtype == 1 {
        let eb = cm_bits.div_ceil(8);
        let raw = b.get(p..p + cm_len * eb).ok_or("tga: short palette")?;
        for i in 0..cm_len { pal.push(conv(&raw[i * eb..], cm_bits)) }
        p += cm_len * eb;
    }
    let pb = bpp.div_ceil(8);
    let px = |d: &[u8]| -> [u8; 4] { if kind == 1 { let i = d[0] as usize; pal.get(i.wrapping_sub(cm_first)).copied().unwrap_or([0, 0, 0, 255]) } else { conv(d, bpp) } };
    let mut pix: Vec<[u8; 4]> = Vec::with_capacity(w * h);
    while pix.len() < w * h {
        if rle {
            let c = *b.get(p).ok_or("tga: truncated")?;
            p += 1;
            let n = (c & 0x7f) as usize + 1;
            if c & 0x80 != 0 { let v = px(b.get(p..p + pb).ok_or("tga: truncated")?); p += pb; for _ in 0..n { pix.push(v) } }
            else { for _ in 0..n { pix.push(px(b.get(p..p + pb).ok_or("tga: truncated")?)); p += pb; } }
        } else { pix.push(px(b.get(p..p + pb).ok_or("tga: truncated")?)); p += pb; }
    }
    pix.truncate(w * h);
    let mut img = Image::new(w as u32, h as u32);
    for y in 0..h {
        let sy = if desc & 0x20 != 0 { y } else { h - 1 - y };
        for x in 0..w {
            let sx = if desc & 0x10 != 0 { w - 1 - x } else { x };
            img.rgba[(y * w + x) * 4..][..4].copy_from_slice(&pix[sy * w + sx]);
        }
    }
    Ok(img)
}
