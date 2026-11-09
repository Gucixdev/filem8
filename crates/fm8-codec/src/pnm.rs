//! Netpbm P1-P6 (PBM/PGM/PPM, ASCII and binary) and PAM (P7).
use crate::Image;

struct T<'a> { b: &'a [u8], p: usize }

impl T<'_> {
    fn tok(&mut self) -> Option<&[u8]> {
        loop {
            while self.b.get(self.p)?.is_ascii_whitespace() { self.p += 1 }
            if self.b[self.p] == b'#' { while *self.b.get(self.p)? != b'\n' { self.p += 1 } } else { break }
        }
        let s = self.p;
        while self.p < self.b.len() && !self.b[self.p].is_ascii_whitespace() { self.p += 1 }
        Some(&self.b[s..self.p])
    }
    fn num(&mut self) -> Option<u32> { std::str::from_utf8(self.tok()?).ok()?.parse().ok() }
}

pub fn decode(b: &[u8]) -> Result<Image, String> {
    let mut t = T { b, p: 2 };
    let kind = *b.get(1).ok_or("pnm: short")?;
    let bad = || "pnm: bad header".to_string();
    let (w, h, maxv, depth);
    if kind == b'7' {
        let (mut ww, mut hh, mut dd, mut mm) = (0, 0, 0, 255);
        loop {
            let k = String::from_utf8_lossy(t.tok().ok_or_else(bad)?).into_owned();
            match k.as_str() {
                "WIDTH" => ww = t.num().ok_or_else(bad)?, "HEIGHT" => hh = t.num().ok_or_else(bad)?,
                "DEPTH" => dd = t.num().ok_or_else(bad)?, "MAXVAL" => mm = t.num().ok_or_else(bad)?,
                "TUPLTYPE" => { t.tok(); }
                "ENDHDR" => break,
                _ => return Err(bad()),
            }
        }
        (w, h, maxv, depth) = (ww, hh, mm, dd);
    } else {
        w = t.num().ok_or_else(bad)?; h = t.num().ok_or_else(bad)?;
        maxv = if kind == b'1' || kind == b'4' { 1 } else { t.num().ok_or_else(bad)? };
        depth = if kind == b'3' || kind == b'6' { 3 } else { 1 };
    }
    if w == 0 || h == 0 || maxv == 0 || maxv > 65535 || (w as u64) * (h as u64) > 1 << 28 || depth == 0 || depth > 4 { return Err("pnm: bad size".into()) }
    let n = (w * h) as usize;
    let mut img = Image::new(w, h);
    let sc = |v: u32| ((v.min(maxv) * 255 + maxv / 2) / maxv) as u8;
    let mut set = |i: usize, s: &[u32]| {
        let o = i * 4;
        let (r, g, bb, a) = match s.len() { 1 => (s[0], s[0], s[0], maxv), 2 => (s[0], s[0], s[0], s[1]), 3 => (s[0], s[1], s[2], maxv), _ => (s[0], s[1], s[2], s[3]) };
        img.rgba[o..o + 4].copy_from_slice(&[sc(r), sc(g), sc(bb), sc(a)]);
    };
    match kind {
        b'1' => for i in 0..n { let v = t.num().ok_or_else(bad)?; set(i, &[1 - v.min(1)]); }
        b'2' | b'3' => for i in 0..n { let mut s = [0; 3]; for c in 0..depth as usize { s[c] = t.num().ok_or_else(bad)?; } set(i, &s[..depth as usize]); }
        b'4' => {
            let d = &b[(t.p + 1).min(b.len())..];
            let rb = (w as usize).div_ceil(8);
            if d.len() < rb * h as usize { return Err("pnm: short".into()) }
            for y in 0..h as usize { for x in 0..w as usize { let bit = (d[y * rb + x / 8] >> (7 - x % 8)) & 1; set(y * w as usize + x, &[1 - bit as u32]); } }
        }
        b'5' | b'6' | b'7' => {
            let d = &b[(t.p + 1).min(b.len())..];
            let bps = if maxv > 255 { 2 } else { 1 };
            if d.len() < n * depth as usize * bps { return Err("pnm: short".into()) }
            for i in 0..n {
                let mut s = [0u32; 4];
                for c in 0..depth as usize { let o = (i * depth as usize + c) * bps; s[c] = if bps == 2 { ((d[o] as u32) << 8) | d[o + 1] as u32 } else { d[o] as u32 }; }
                set(i, &s[..depth as usize]);
            }
        }
        _ => return Err("pnm: unsupported".into()),
    }
    Ok(img)
}
