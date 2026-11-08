//! QOI (https://qoiformat.org), decode.
use crate::Image;

pub fn decode(b: &[u8]) -> Result<Image, String> {
    if b.len() < 14 + 8 || &b[..4] != b"qoif" { return Err("not qoi".into()) }
    let w = u32::from_be_bytes(b[4..8].try_into().unwrap());
    let h = u32::from_be_bytes(b[8..12].try_into().unwrap());
    if w == 0 || h == 0 || (w as u64) * (h as u64) > 1 << 28 { return Err("qoi: bad size".into()) }
    let n = (w * h) as usize;
    let mut out = Vec::with_capacity(n * 4);
    let mut idx = [[0u8; 4]; 64];
    let mut px = [0u8, 0, 0, 255];
    let mut p = 14;
    let end = b.len() - 8;
    while out.len() < n * 4 {
        if p >= end { return Err("qoi: truncated".into()) }
        let c = b[p];
        p += 1;
        let mut run = 0;
        match c {
            0xFE => { px[..3].copy_from_slice(b.get(p..p + 3).ok_or("qoi: truncated")?); p += 3 }
            0xFF => { px.copy_from_slice(b.get(p..p + 4).ok_or("qoi: truncated")?); p += 4 }
            _ => match c >> 6 {
                0 => px = idx[c as usize],
                1 => { px[0] = px[0].wrapping_add(((c >> 4) & 3).wrapping_sub(2)); px[1] = px[1].wrapping_add(((c >> 2) & 3).wrapping_sub(2)); px[2] = px[2].wrapping_add((c & 3).wrapping_sub(2)); }
                2 => {
                    let c2 = *b.get(p).ok_or("qoi: truncated")?; p += 1;
                    let dg = (c & 0x3f).wrapping_sub(32);
                    px[0] = px[0].wrapping_add(dg.wrapping_sub(8).wrapping_add(c2 >> 4));
                    px[1] = px[1].wrapping_add(dg);
                    px[2] = px[2].wrapping_add(dg.wrapping_sub(8).wrapping_add(c2 & 15));
                }
                _ => run = c & 0x3f,
            },
        }
        idx[(px[0] as usize * 3 + px[1] as usize * 5 + px[2] as usize * 7 + px[3] as usize * 11) % 64] = px;
        for _ in 0..=run { if out.len() < n * 4 { out.extend_from_slice(&px) } }
    }
    Ok(Image { w, h, rgba: out })
}
