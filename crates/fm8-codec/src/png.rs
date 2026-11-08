//! PNG decoder (all colour types, 1-16 bit, Adam7, tRNS) and RGBA8 encoder.

use crate::inflate;
use crate::Image;

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (ia, ib, ic) = (a as i32, b as i32, c as i32);
    let p = ia + ib - ic;
    let (pa, pb, pc) = ((p - ia).abs(), (p - ib).abs(), (p - ic).abs());
    if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
}

/// Undo scanline filters in place; returns the raw rows concatenated.
fn unfilter(d: &[u8], h: usize, rowbytes: usize, bpp: usize) -> Result<Vec<u8>, String> {
    if d.len() < h * (rowbytes + 1) { return Err("png: short data".into()) }
    let mut out = vec![0u8; h * rowbytes];
    for y in 0..h {
        let f = d[y * (rowbytes + 1)];
        let src = &d[y * (rowbytes + 1) + 1..(y + 1) * (rowbytes + 1)];
        let (done, cur) = out.split_at_mut(y * rowbytes);
        let cur = &mut cur[..rowbytes];
        let up: &[u8] = if y > 0 { &done[(y - 1) * rowbytes..] } else { &[] };
        for x in 0..rowbytes {
            let a = if x >= bpp { cur[x - bpp] } else { 0 };
            let b = if y > 0 { up[x] } else { 0 };
            let c = if x >= bpp && y > 0 { up[x - bpp] } else { 0 };
            cur[x] = src[x].wrapping_add(match f { 0 => 0, 1 => a, 2 => b, 3 => ((a as u16 + b as u16) / 2) as u8, 4 => paeth(a, b, c), _ => return Err("png: bad filter".into()) });
        }
    }
    Ok(out)
}

pub fn decode(b: &[u8]) -> Result<Image, String> {
    if !b.starts_with(b"\x89PNG\r\n\x1a\n") { return Err("not a png".into()) }
    let (mut w, mut h, mut depth, mut ct, mut inter) = (0usize, 0usize, 0u8, 0u8, 0u8);
    let (mut plte, mut trns, mut idat) = (vec![], vec![], vec![]);
    let mut p = 8;
    while p + 8 <= b.len() {
        let len = u32::from_be_bytes(b[p..p + 4].try_into().unwrap()) as usize;
        let ty = &b[p + 4..p + 8];
        let data = b.get(p + 8..p + 8 + len).ok_or("png: truncated chunk")?;
        match ty {
            b"IHDR" if len >= 13 => { w = u32::from_be_bytes(data[0..4].try_into().unwrap()) as usize; h = u32::from_be_bytes(data[4..8].try_into().unwrap()) as usize; depth = data[8]; ct = data[9]; inter = data[12]; }
            b"PLTE" => plte = data.to_vec(),
            b"tRNS" => trns = data.to_vec(),
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        p += 12 + len;
    }
    if w == 0 || h == 0 || w * h > 1 << 28 { return Err("png: bad size".into()) }
    let ch = match ct { 0 => 1, 2 => 3, 3 => 1, 4 => 2, 6 => 4, _ => return Err("png: bad colour type".into()) };
    if ![1, 2, 4, 8, 16].contains(&depth) { return Err("png: bad depth".into()) }
    let bits = ch * depth as usize;
    let bpp = bits.div_ceil(8);
    let raw = inflate::zlib(&idat, w * h * 8 + h * 8 + 4096)?;
    let mut img = Image::new(w as u32, h as u32);
    let maxv = ((1u32 << depth.min(16)) - 1) as f32;
    let sample = |row: &[u8], i: usize| -> u32 {
        match depth {
            16 => ((row[i * 2] as u32) << 8) | row[i * 2 + 1] as u32,
            8 => row[i] as u32,
            _ => { let per = 8 / depth as usize; let byte = row[i / per]; let sh = 8 - depth as usize * (i % per + 1); ((byte >> sh) & ((1 << depth) - 1)) as u32 }
        }
    };
    let to8 = |v: u32| -> u8 { if depth == 8 { v as u8 } else if depth == 16 { (v >> 8) as u8 } else { (v as f32 * 255.0 / maxv).round() as u8 } };
    let tkey: Option<Vec<u32>> = if ct == 0 && trns.len() >= 2 { Some(vec![u16::from_be_bytes([trns[0], trns[1]]) as u32]) } else if ct == 2 && trns.len() >= 6 { Some((0..3).map(|i| u16::from_be_bytes([trns[i * 2], trns[i * 2 + 1]]) as u32).collect()) } else { None };
    let mut put = |row: &[u8], lx: usize, ox: usize, oy: usize| {
        let o = (oy * w + ox) * 4;
        let px = &mut img.rgba[o..o + 4];
        match ct {
            0 => { let v = sample(row, lx); let g = to8(v); px.copy_from_slice(&[g, g, g, if tkey.as_ref().map(|k| k[0] == v).unwrap_or(false) { 0 } else { 255 }]) }
            2 => { let v: Vec<u32> = (0..3).map(|c| sample(row, lx * 3 + c)).collect(); px.copy_from_slice(&[to8(v[0]), to8(v[1]), to8(v[2]), if tkey.as_ref().map(|k| *k == v).unwrap_or(false) { 0 } else { 255 }]) }
            3 => { let i = sample(row, lx) as usize; let c = plte.get(i * 3..i * 3 + 3).unwrap_or(&[0, 0, 0]); px.copy_from_slice(&[c[0], c[1], c[2], trns.get(i).copied().unwrap_or(255)]) }
            4 => { let g = to8(sample(row, lx * 2)); px.copy_from_slice(&[g, g, g, to8(sample(row, lx * 2 + 1))]) }
            _ => px.copy_from_slice(&[to8(sample(row, lx * 4)), to8(sample(row, lx * 4 + 1)), to8(sample(row, lx * 4 + 2)), to8(sample(row, lx * 4 + 3))]),
        }
    };
    if inter == 0 {
        let rows = unfilter(&raw, h, (w * bits).div_ceil(8), bpp)?;
        let rb = (w * bits).div_ceil(8);
        for y in 0..h { let row = &rows[y * rb..(y + 1) * rb]; for x in 0..w { put(row, x, x, y) } }
    } else {
        const P: [(usize, usize, usize, usize); 7] = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];
        let mut off = 0;
        for (x0, y0, dx, dy) in P {
            let pw = (w + dx - 1 - x0) / dx;
            let ph = (h + dy - 1 - y0) / dy;
            if pw == 0 || ph == 0 { continue }
            let rb = (pw * bits).div_ceil(8);
            let rows = unfilter(raw.get(off..).ok_or("png: short")?, ph, rb, bpp)?;
            off += ph * (rb + 1);
            for py in 0..ph {
                let row = &rows[py * rb..(py + 1) * rb];
                for px in 0..pw {
                    // sample from pass-local index, store at full-image position
                    let (ox, oy) = (x0 + px * dx, y0 + py * dy);
                    put(row, px, ox, oy);
                }
            }
        }
    }
    Ok(img)
}

fn chunk(out: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(ty);
    out.extend_from_slice(data);
    out.extend_from_slice(&inflate::crc32(&[ty, data]).to_be_bytes());
}

/// RGBA8 encoder: per-row Sub filter, fixed-Huffman LZ77.
pub fn encode(img: &Image) -> Vec<u8> {
    let (w, h) = (img.w as usize, img.h as usize);
    let mut raw = Vec::with_capacity(h * (w * 4 + 1));
    for y in 0..h {
        raw.push(1);
        let row = &img.rgba[y * w * 4..(y + 1) * w * 4];
        for x in 0..w * 4 { raw.push(row[x].wrapping_sub(if x >= 4 { row[x - 4] } else { 0 })) }
    }
    let mut z = vec![0x78, 0x01];
    z.extend(inflate::deflate(&raw));
    z.extend_from_slice(&inflate::adler32(&raw).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = vec![];
    ihdr.extend_from_slice(&img.w.to_be_bytes());
    ihdr.extend_from_slice(&img.h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}
