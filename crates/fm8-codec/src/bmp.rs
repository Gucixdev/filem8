//! BMP: 1/4/8/16/24/32 bit, RLE4/RLE8, BITFIELDS, top-down and bottom-up.
use crate::Image;

fn u16le(b: &[u8], o: usize) -> usize { b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]) as usize).unwrap_or(0) }
fn u32le(b: &[u8], o: usize) -> u32 { b.get(o..o + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap())).unwrap_or(0) }

pub fn decode(b: &[u8]) -> Result<Image, String> {
    if b.len() < 26 || &b[..2] != b"BM" { return Err("not bmp".into()) }
    let off = u32le(b, 10) as usize;
    decode_dib(&b[14..], off.saturating_sub(14), false)
}

/// `dib` starts at the BITMAPINFOHEADER. `data_off` is relative to it. `ico` = height is doubled and an AND mask follows.
pub fn decode_dib(dib: &[u8], data_off: usize, ico: bool) -> Result<Image, String> {
    let hs = u32le(dib, 0) as usize;
    let (w, mut h, bpp, comp, ncol);
    if hs == 12 { w = u16le(dib, 4) as i64; h = u16le(dib, 6) as i64; bpp = u16le(dib, 10); comp = 0; ncol = 0; }
    else if hs >= 40 { w = u32le(dib, 4) as i32 as i64; h = u32le(dib, 8) as i32 as i64; bpp = u16le(dib, 14); comp = u32le(dib, 16); ncol = u32le(dib, 32) as usize; }
    else { return Err("bmp: bad header".into()) }
    let top_down = h < 0;
    h = h.abs();
    if ico { h /= 2 }
    if w <= 0 || h <= 0 || w * h > 1 << 28 { return Err("bmp: bad size".into()) }
    let (w, h) = (w as usize, h as usize);
    let pal_n = if bpp <= 8 { if ncol > 0 { ncol } else { 1 << bpp } } else { 0 };
    let pe = if hs == 12 { 3 } else { 4 };
    let mut pal = vec![];
    for i in 0..pal_n { let o = hs + i * pe; let s = dib.get(o..o + 3).ok_or("bmp: palette")?; pal.push([s[2], s[1], s[0], 255]) }
    let mut masks = [0u32; 4];
    if comp == 3 || comp == 6 { for i in 0..4 { masks[i] = if i < 3 || hs >= 56 { u32le(dib, 40 + i * 4) } else { 0 } } }
    else if bpp == 16 { masks = [0x7C00, 0x03E0, 0x001F, 0] } else if bpp == 32 { masks = [0xFF0000, 0xFF00, 0xFF, 0] }
    let data = dib.get(data_off..).ok_or("bmp: data offset")?;
    let mut img = Image::new(w as u32, h as u32);
    let oy = |y: usize| if top_down { y } else { h - 1 - y };
    if comp == 1 || comp == 2 {
        // RLE8 / RLE4
        let (mut x, mut y, mut p) = (0usize, 0usize, 0usize);
        let mut idx = vec![0u8; w * h];
        while p + 1 < data.len() && y < h {
            let (n, v) = (data[p] as usize, data[p + 1]);
            p += 2;
            if n > 0 {
                for k in 0..n { if x < w { idx[y * w + x] = if comp == 1 { v } else if k % 2 == 0 { v >> 4 } else { v & 15 }; x += 1 } }
            } else {
                match v {
                    0 => { x = 0; y += 1 } 1 => break,
                    2 => { x += *data.get(p).unwrap_or(&0) as usize; y += *data.get(p + 1).unwrap_or(&0) as usize; p += 2 }
                    c => {
                        let c = c as usize;
                        for k in 0..c { let byte = *data.get(p + if comp == 1 { k } else { k / 2 }).unwrap_or(&0); if x < w { idx[y * w + x] = if comp == 1 { byte } else if k % 2 == 0 { byte >> 4 } else { byte & 15 }; x += 1 } }
                        let used = if comp == 1 { c } else { c.div_ceil(2) };
                        p += used + (used & 1);
                    }
                }
            }
        }
        for y in 0..h { for x in 0..w { let c = pal.get(idx[y * w + x] as usize).copied().unwrap_or([0, 0, 0, 255]); img.rgba[(oy(y) * w + x) * 4..][..4].copy_from_slice(&c) } }
        return Ok(img);
    }
    let rb = (w * bpp).div_ceil(32) * 4;
    if data.len() < rb * h { return Err("bmp: short pixel data".into()) }
    let shift = |m: u32| -> (u32, u32) { if m == 0 { (0, 0) } else { let s = m.trailing_zeros(); (s, (m >> s).count_ones()) } };
    let ch = |v: u32, m: u32| -> u8 { let (s, n) = shift(m); if n == 0 { 255 } else { let (x, max) = (((v & m) >> s) as u64, (1u64 << n) - 1); (x.min(max) * 255 / max) as u8 } };
    let mut any_alpha = false;
    for y in 0..h {
        let row = &data[y * rb..(y + 1) * rb];
        for x in 0..w {
            let c = match bpp {
                1 | 2 | 4 | 8 => { let per = 8 / bpp; let sh = 8 - bpp * (x % per + 1); pal.get(((row[x / per] >> sh) as usize) & ((1 << bpp) - 1)).copied().unwrap_or([0, 0, 0, 255]) }
                16 => { let v = u16::from_le_bytes([row[x * 2], row[x * 2 + 1]]) as u32; [ch(v, masks[0]), ch(v, masks[1]), ch(v, masks[2]), 255] }
                24 => [row[x * 3 + 2], row[x * 3 + 1], row[x * 3], 255],
                32 => { let v = u32::from_le_bytes(row[x * 4..x * 4 + 4].try_into().unwrap()); let a = if masks[3] != 0 { ch(v, masks[3]) } else { row[x * 4 + 3] }; if a != 0 { any_alpha = true } [ch(v, masks[0]), ch(v, masks[1]), ch(v, masks[2]), a] }
                _ => return Err("bmp: unsupported depth".into()),
            };
            img.rgba[(oy(y) * w + x) * 4..][..4].copy_from_slice(&c);
        }
    }
    if bpp == 32 && !any_alpha { for p in img.rgba.as_chunks_mut::<4>().0 { p[3] = 255 } }
    if ico {
        // 1-bit AND mask follows the colour data (only when no alpha was present)
        let mrb = w.div_ceil(32) * 4;
        let mo = rb * h;
        if bpp != 32 || !any_alpha {
            if let Some(m) = data.get(mo..mo + mrb * h) {
                for y in 0..h { for x in 0..w { if (m[y * mrb + x / 8] >> (7 - x % 8)) & 1 == 1 { img.rgba[(oy(y) * w + x) * 4 + 3] = 0 } } }
            }
        }
    }
    Ok(img)
}
