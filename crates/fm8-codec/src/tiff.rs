//! Baseline TIFF: grey/RGB/RGBA/palette/CMYK-less, 1/8/16 bit, strips, tiles-less;
//! compression none, PackBits, LZW, Deflate; horizontal predictor.
use crate::{inflate, Image};

struct R<'a> { b: &'a [u8], le: bool }

impl R<'_> {
    fn u16(&self, o: usize) -> Option<u32> { let s = self.b.get(o..o + 2)?; Some(if self.le { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) } as u32) }
    fn u32(&self, o: usize) -> Option<u32> { let s: [u8; 4] = self.b.get(o..o + 4)?.try_into().ok()?; Some(if self.le { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) }) }
    /// All values of an IFD entry.
    fn vals(&self, e: usize) -> Option<Vec<u32>> {
        let (ty, n) = (self.u16(e + 2)?, self.u32(e + 4)? as usize);
        let sz = match ty { 3 => 2, 4 => 4, 1 | 6 | 7 => 1, _ => return None };
        let base = if n * sz <= 4 { e + 8 } else { self.u32(e + 8)? as usize };
        if n > 1 << 24 { return None }
        (0..n).map(|i| match sz { 1 => self.b.get(base + i).map(|&x| x as u32), 2 => self.u16(base + i * 2), _ => self.u32(base + i * 4) }).collect()
    }
}

fn tiff_lzw(d: &[u8], limit: usize) -> Vec<u8> {
    // MSB-first, early change
    let mut table: Vec<Vec<u8>> = vec![];
    let reset = |t: &mut Vec<Vec<u8>>| { t.clear(); for i in 0..256 { t.push(vec![i as u8]) } t.push(vec![]); t.push(vec![]) };
    reset(&mut table);
    let (mut out, mut size, mut acc, mut nb, mut prev) = (vec![], 9usize, 0u32, 0usize, None::<Vec<u8>>);
    for &byte in d {
        acc = (acc << 8) | byte as u32;
        nb += 8;
        while nb >= size {
            let code = ((acc >> (nb - size)) & ((1 << size) - 1)) as usize;
            nb -= size;
            if code == 256 { reset(&mut table); size = 9; prev = None; continue }
            if code == 257 { return out }
            let entry = if code < table.len() { table[code].clone() } else if let Some(p) = &prev { let mut e = p.clone(); e.push(p[0]); e } else { return out };
            out.extend_from_slice(&entry);
            if out.len() >= limit { return out }
            if let Some(p) = prev.take() { let mut e = p; e.push(entry[0]); if table.len() < 4096 { table.push(e) } }
            prev = Some(entry);
            size = match table.len() + 1 { n if n >= 2048 => 12, n if n >= 1024 => 11, n if n >= 512 => 10, _ => 9 };
        }
    }
    out
}

fn packbits(d: &[u8]) -> Vec<u8> {
    let (mut o, mut i) = (vec![], 0);
    while i < d.len() {
        let n = d[i] as i8;
        i += 1;
        if n >= 0 { let c = n as usize + 1; o.extend_from_slice(&d[i..(i + c).min(d.len())]); i += c } else if n != -128 { if let Some(&v) = d.get(i) { o.extend(std::iter::repeat_n(v, 1 - n as isize as usize)); } i += 1 }
    }
    o
}

pub fn decode(b: &[u8]) -> Result<Image, String> {
    let le = match b.get(..2) { Some(b"II") => true, Some(b"MM") => false, _ => return Err("not tiff".into()) };
    let r = R { b, le };
    let ifd = r.u32(4).ok_or("tiff: short")? as usize;
    let n = r.u16(ifd).ok_or("tiff: bad ifd")? as usize;
    let get = |tag: u32| -> Option<Vec<u32>> { (0..n).find(|&i| r.u16(ifd + 2 + i * 12) == Some(tag)).and_then(|i| r.vals(ifd + 2 + i * 12)) };
    let one = |tag: u32, d: u32| get(tag).and_then(|v| v.first().copied()).unwrap_or(d);
    let (w, h) = (one(256, 0) as usize, one(257, 0) as usize);
    if w == 0 || h == 0 || w * h > 1 << 28 { return Err("tiff: bad size".into()) }
    let bits = get(258).unwrap_or(vec![1]);
    let bps = *bits.first().ok_or("tiff: no bit depth")? as usize;
    let comp = one(259, 1);
    let photo = one(262, 1);
    let spp = one(277, 1) as usize;
    let rps = one(278, h as u32) as usize;
    let pred = one(317, 1);
    let planar = one(284, 1);
    if planar != 1 || !matches!(bps, 1 | 8 | 16) || get(322).is_some() { return Err("tiff: unsupported layout (planar/tiled/bit depth)".into()) }
    let offs = get(273).ok_or("tiff: no strips")?;
    let cnts = get(279).unwrap_or_default();
    if spp == 0 || spp > 8 { return Err("tiff: unsupported samples per pixel".into()) }
    let rowbytes = (w * spp * bps).div_ceil(8);
    // no codec here expands more than about 1:1400 (LZW): a header that claims more than the file can hold is truncated
    if rowbytes * h > b.len().saturating_mul(2048) { return Err("tiff: truncated".into()) }
    let mut raw = Vec::with_capacity(rowbytes * h);
    for (i, &o) in offs.iter().enumerate() {
        let o = o as usize;
        let c = cnts.get(i).map(|&c| c as usize).unwrap_or(b.len().saturating_sub(o));
        let s = b.get(o..(o + c).min(b.len())).ok_or("tiff: bad strip")?;
        let rows = rps.min(h - (i * rps).min(h));
        let mut d = match comp { 1 => s.to_vec(), 5 => tiff_lzw(s, rowbytes * rows), 8 | 32946 => inflate::zlib(s, rowbytes * rows + 1024)?, 32773 => packbits(s), _ => return Err(format!("tiff: compression {comp} unsupported")) };
        d.resize(rowbytes * rows, 0);
        if pred == 2 && bps == 8 { for y in 0..rows { for x in spp..rowbytes { d[y * rowbytes + x] = d[y * rowbytes + x].wrapping_add(d[y * rowbytes + x - spp]) } } }
        raw.extend_from_slice(&d);
    }
    raw.resize(rowbytes * h, 0);
    let cmap = if photo == 3 { get(320) } else { None };
    let extra = get(338).and_then(|v| v.first().copied()).unwrap_or(0);
    let mut img = Image::new(w as u32, h as u32);
    for y in 0..h {
        let row = &raw[y * rowbytes..(y + 1) * rowbytes];
        for x in 0..w {
            let smp = |c: usize| -> u8 { match bps { 1 => { let v = (row[(x * spp + c) / 8] >> (7 - (x * spp + c) % 8)) & 1; v * 255 } 16 => { let o = (x * spp + c) * 2; if le { row[o + 1] } else { row[o] } } _ => row[x * spp + c] } };
            let inv = |v: u8| if photo == 0 { 255 - v } else { v };
            let px = match (photo, spp) {
                (3, _) => { let i = (if bps == 1 { smp(0) as usize / 255 } else { smp(0) as usize }).min(255); let m = cmap.as_ref().ok_or("tiff: no colormap")?; let g = |c: usize| m.get(c * (1 << bps.min(8)) + i).map(|&v| (v >> 8) as u8).unwrap_or(0); [g(0), g(1), g(2), 255] }
                (0 | 1, 1) => { let g = inv(smp(0)); [g, g, g, 255] }
                (0 | 1, _) => { let g = inv(smp(0)); [g, g, g, if extra != 0 { smp(1) } else { 255 }] }
                (2, 3) => [smp(0), smp(1), smp(2), 255],
                (2, s) if s >= 4 => [smp(0), smp(1), smp(2), smp(3)],
                _ => return Err("tiff: unsupported photometric".into()),
            };
            img.rgba[(y * w + x) * 4..][..4].copy_from_slice(&px);
        }
    }
    Ok(img)
}
