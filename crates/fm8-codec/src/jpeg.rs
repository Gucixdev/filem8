//! JPEG baseline / extended-sequential Huffman decoder (grey and YCbCr, any
//! sampling, restart markers). Progressive and arithmetic coding return Err.
use crate::Image;

const ZZ: [usize; 64] = [0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63];

#[derive(Default, Clone)]
struct Table { maxcode: [i32; 18], valptr: [i32; 17], mincode: [i32; 17], vals: Vec<u8> }

impl Table {
    fn new(counts: &[u8; 16], vals: Vec<u8>) -> Table {
        let mut t = Table { vals, ..Default::default() };
        let (mut code, mut k) = (0i32, 0i32);
        for l in 1..=16 {
            t.valptr[l] = k;
            t.mincode[l] = code;
            code += counts[l - 1] as i32;
            k += counts[l - 1] as i32;
            t.maxcode[l] = if counts[l - 1] > 0 { code - 1 } else { -1 };
            code <<= 1;
        }
        t.maxcode[17] = i32::MAX;
        t
    }
}

#[derive(Clone, Default)]
struct Comp { id: u8, h: usize, v: usize, tq: usize, td: usize, ta: usize, pred: i32, bw: usize, bh: usize, data: Vec<u8> }

struct Bits<'a> { d: &'a [u8], p: usize, acc: u32, n: u32, marker: Option<u8> }

impl Bits<'_> {
    fn bit(&mut self) -> Result<u32, String> {
        if self.n == 0 {
            let b = if self.marker.is_some() { 0 } else {
                let b = *self.d.get(self.p).ok_or("jpeg: truncated")?;
                self.p += 1;
                if b == 0xFF {
                    let m = *self.d.get(self.p).ok_or("jpeg: truncated")?;
                    if m == 0 { self.p += 1; 0xFF } else { self.marker = Some(m); 0 }
                } else { b }
            };
            self.acc = b as u32;
            self.n = 8;
        }
        self.n -= 1;
        Ok((self.acc >> self.n) & 1)
    }
    fn bits(&mut self, k: u32) -> Result<i32, String> { let mut v = 0; for _ in 0..k { v = (v << 1) | self.bit()? as i32 } Ok(v) }
    fn huff(&mut self, t: &Table) -> Result<u8, String> {
        let mut code = 0i32;
        for l in 1..=16 {
            code = (code << 1) | self.bit()? as i32;
            if t.maxcode[l] >= 0 && code <= t.maxcode[l] && code >= t.mincode[l] {
                return t.vals.get((t.valptr[l] + code - t.mincode[l]) as usize).copied().ok_or_else(|| "jpeg: bad huffman".to_string());
            }
        }
        Err("jpeg: bad huffman code".into())
    }
    fn restart(&mut self) { self.n = 0; if self.marker.is_none() { while self.p + 1 < self.d.len() && !(self.d[self.p] == 0xFF && (0xD0..=0xD7).contains(&self.d[self.p + 1])) { self.p += 1 } if self.p + 1 < self.d.len() { self.p += 2 } } else { self.p += 1; self.marker = None; } }
}

fn extend(v: i32, t: u32) -> i32 { if t == 0 { 0 } else if v < (1 << (t - 1)) { v - (1 << t) + 1 } else { v } }

fn idct(blk: &[i32; 64], out: &mut [u8; 64]) {
    use std::sync::OnceLock;
    static C: OnceLock<[[f32; 8]; 8]> = OnceLock::new();
    let c = C.get_or_init(|| { let mut c = [[0f32; 8]; 8]; for x in 0..8 { for u in 0..8 { let a = if u == 0 { (0.125f32).sqrt() } else { 0.5 }; c[x][u] = a * ((2 * x + 1) as f32 * u as f32 * std::f32::consts::PI / 16.0).cos() } } c });
    let mut tmp = [0f32; 64];
    for y in 0..8 { for x in 0..8 { let mut s = 0.0; for u in 0..8 { s += c[x][u] * blk[y * 8 + u] as f32 } tmp[y * 8 + x] = s } }
    for x in 0..8 { for y in 0..8 { let mut s = 0.0; for v in 0..8 { s += c[y][v] * tmp[v * 8 + x] } out[y * 8 + x] = (s + 128.0).round().clamp(0.0, 255.0) as u8 } }
}

pub fn decode(b: &[u8]) -> Result<Image, String> {
    if !b.starts_with(&[0xFF, 0xD8]) { return Err("not jpeg".into()) }
    let mut p = 2;
    let mut qt = [[0u16; 64]; 4];
    let mut dc: Vec<Table> = vec![Table::default(); 4];
    let mut ac = dc.clone();
    let (mut w, mut h) = (0usize, 0usize);
    let mut comps: Vec<Comp> = vec![];
    let mut ri = 0usize;
    let mut adobe = None::<u8>;
    loop {
        while p < b.len() && b[p] != 0xFF { p += 1 }
        while p < b.len() && b[p] == 0xFF { p += 1 }
        let m = *b.get(p).ok_or("jpeg: no scan")?;
        p += 1;
        if m == 0xD9 { return Err("jpeg: no scan".into()) }
        if m == 0x01 || (0xD0..=0xD8).contains(&m) { continue }
        let len = u16::from_be_bytes([*b.get(p).ok_or("jpeg: short")?, *b.get(p + 1).ok_or("jpeg: short")?]) as usize;
        let seg = b.get(p + 2..p + len).ok_or("jpeg: short segment")?;
        match m {
            0xDB => { let mut s = seg; while !s.is_empty() { let (pq, tq) = (s[0] >> 4, (s[0] & 15) as usize); if tq > 3 { return Err("jpeg: bad dqt".into()) } let n = if pq == 0 { 64 } else { 128 }; if s.len() < 1 + n { return Err("jpeg: short dqt".into()) } for i in 0..64 { qt[tq][ZZ[i]] = if pq == 0 { s[1 + i] as u16 } else { u16::from_be_bytes([s[1 + i * 2], s[2 + i * 2]]) } } s = &s[1 + n..] } }
            0xC4 => { let mut s = seg; while s.len() >= 17 { let (tc, th) = (s[0] >> 4, (s[0] & 15) as usize); let counts: [u8; 16] = s[1..17].try_into().unwrap(); let n: usize = counts.iter().map(|&c| c as usize).sum(); if th > 3 || s.len() < 17 + n { return Err("jpeg: bad dht".into()) } let t = Table::new(&counts, s[17..17 + n].to_vec()); if tc == 0 { dc[th] = t } else { ac[th] = t } s = &s[17 + n..] } }
            0xDD if seg.len() >= 2 => ri = u16::from_be_bytes([seg[0], seg[1]]) as usize,
            0xEE if seg.starts_with(b"Adobe") && seg.len() >= 12 => adobe = Some(seg[11]),
            0xC0 | 0xC1 => {
                if seg.len() < 6 { return Err("jpeg: short frame".into()) }
                if seg[0] != 8 { return Err("jpeg: only 8-bit".into()) }
                h = u16::from_be_bytes([seg[1], seg[2]]) as usize; w = u16::from_be_bytes([seg[3], seg[4]]) as usize;
                let n = seg[5] as usize;
                if w == 0 || h == 0 || w * h > 1 << 28 || !(n == 1 || n == 3) || seg.len() < 6 + n * 3 { return Err("jpeg: unsupported frame".into()) }
                comps = (0..n).map(|i| Comp { id: seg[6 + i * 3], h: (seg[7 + i * 3] >> 4) as usize, v: (seg[7 + i * 3] & 15) as usize, tq: (seg[8 + i * 3] & 3) as usize, ..Default::default() }).collect();
                if comps.iter().any(|c| c.h == 0 || c.v == 0 || c.h > 4 || c.v > 4) { return Err("jpeg: bad sampling".into()) }
            }
            0xC2 => return Err("jpeg: progressive not supported".into()),
            0xC3 | 0xC5..=0xC7 | 0xC9..=0xCF => return Err("jpeg: unsupported coding".into()),
            0xDA => {
                if comps.is_empty() { return Err("jpeg: scan before frame".into()) }
                let ns = *seg.first().ok_or("jpeg: short scan")? as usize;
                if seg.len() < 1 + ns * 2 { return Err("jpeg: short scan".into()) }
                if ns != comps.len() { return Err("jpeg: non-interleaved scans unsupported".into()) }
                for i in 0..ns { let cid = seg[1 + i * 2]; let t = seg[2 + i * 2]; let c = comps.iter_mut().find(|c| c.id == cid).ok_or("jpeg: bad scan comp")?; c.td = (t >> 4) as usize & 3; c.ta = (t & 15) as usize & 3; }
                p += len;
                return scan(b, p, w, h, comps, &qt, &dc, &ac, ri, adobe);
            }
            _ => {}
        }
        p += len;
    }
}

#[allow(clippy::too_many_arguments)]
fn scan(b: &[u8], p: usize, w: usize, h: usize, mut comps: Vec<Comp>, qt: &[[u16; 64]; 4], dc: &[Table], ac: &[Table], ri: usize, adobe: Option<u8>) -> Result<Image, String> {
    let (hmax, vmax) = (comps.iter().map(|c| c.h).max().unwrap(), comps.iter().map(|c| c.v).max().unwrap());
    let (mw, mh) = (w.div_ceil(8 * hmax), h.div_ceil(8 * vmax));
    for c in &mut comps { c.bw = mw * c.h * 8; c.bh = mh * c.v * 8; c.data = vec![0; c.bw * c.bh]; }
    let mut br = Bits { d: b, p, acc: 0, n: 0, marker: None };
    let mut blk = [0i32; 64];
    let mut px = [0u8; 64];
    let mut cnt = 0;
    for my in 0..mh { for mx in 0..mw {
        if ri > 0 && cnt == ri { br.restart(); for c in &mut comps { c.pred = 0 } cnt = 0 }
        cnt += 1;
        for c in comps.iter_mut() { for by in 0..c.v { for bx in 0..c.h {
            blk.fill(0);
            let t = br.huff(&dc[c.td])? as u32;
            if t > 16 { return Err("jpeg: bad dc size".into()) }
            let diff = if t == 0 { 0 } else { extend(br.bits(t)?, t) };
            c.pred = c.pred.wrapping_add(diff);
            blk[0] = c.pred.wrapping_mul(qt[c.tq][0] as i32);
            let mut k = 1;
            while k < 64 {
                let rs = br.huff(&ac[c.ta])?;
                let (r, s) = ((rs >> 4) as usize, (rs & 15) as u32);
                if s == 0 { if r == 15 { k += 16; continue } break }
                k += r;
                if k > 63 { break }
                blk[ZZ[k]] = extend(br.bits(s)?, s) * qt[c.tq][ZZ[k]] as i32;
                k += 1;
            }
            idct(&blk, &mut px);
            let (ox, oy) = ((mx * c.h + bx) * 8, (my * c.v + by) * 8);
            for y in 0..8 { c.data[(oy + y) * c.bw + ox..][..8].copy_from_slice(&px[y * 8..y * 8 + 8]) }
        } } }
    } }
    let mut img = Image::new(w as u32, h as u32);
    let sample = |c: &Comp, x: usize, y: usize| c.data[(y * c.v / vmax) * c.bw + x * c.h / hmax];
    for y in 0..h { for x in 0..w {
        let o = (y * w + x) * 4;
        if comps.len() == 1 { let g = sample(&comps[0], x, y); img.rgba[o..o + 4].copy_from_slice(&[g, g, g, 255]) }
        else {
            let (yy, cb, cr) = (sample(&comps[0], x, y) as f32, sample(&comps[1], x, y) as f32 - 128.0, sample(&comps[2], x, y) as f32 - 128.0);
            let rgb = if adobe == Some(0) { [yy, cb + 128.0, cr + 128.0] } else { [yy + 1.402 * cr, yy - 0.344136 * cb - 0.714136 * cr, yy + 1.772 * cb] };
            img.rgba[o..o + 4].copy_from_slice(&[rgb[0].round().clamp(0.0, 255.0) as u8, rgb[1].round().clamp(0.0, 255.0) as u8, rgb[2].round().clamp(0.0, 255.0) as u8, 255]);
        }
    } }
    Ok(img)
}
