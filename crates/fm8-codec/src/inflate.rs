//! RFC 1950/1951 inflate, plus a small deflate encoder (fixed Huffman + LZ77).

struct Bits<'a> { d: &'a [u8], pos: usize, buf: u32, n: u32 }

impl Bits<'_> {
    fn need(&mut self, k: u32) -> Result<(), String> {
        while self.n < k {
            let b = *self.d.get(self.pos).ok_or("deflate: unexpected end")?;
            self.pos += 1;
            self.buf |= (b as u32) << self.n;
            self.n += 8;
        }
        Ok(())
    }
    fn bits(&mut self, k: u32) -> Result<u32, String> {
        if k == 0 { return Ok(0) }
        self.need(k)?;
        let v = self.buf & ((1u64 << k) - 1) as u32;
        self.buf >>= k;
        self.n -= k;
        Ok(v)
    }
}

struct Huff { counts: [u16; 16], syms: Vec<u16> }

impl Huff {
    fn new(lens: &[u8]) -> Huff {
        let mut counts = [0u16; 16];
        for &l in lens { counts[l as usize] += 1 }
        counts[0] = 0;
        let mut offs = [0u16; 16];
        for i in 1..16 { offs[i] = offs[i - 1] + counts[i - 1] }
        let mut syms = vec![0u16; lens.len()];
        for (s, &l) in lens.iter().enumerate() { if l != 0 { syms[offs[l as usize] as usize] = s as u16; offs[l as usize] += 1 } }
        Huff { counts, syms }
    }
    fn decode(&self, b: &mut Bits) -> Result<u16, String> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= b.bits(1)? as i32;
            let count = self.counts[len] as i32;
            if code - count < first { return Ok(self.syms[(index + (code - first)) as usize]) }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("deflate: bad code".into())
    }
}

const LBASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEXT: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DBASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DEXT: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

/// Raw deflate stream. `limit` caps the output size (zip-bomb guard).
pub fn inflate(d: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    let mut b = Bits { d, pos: 0, buf: 0, n: 0 };
    let mut out: Vec<u8> = Vec::new();
    loop {
        let last = b.bits(1)?;
        match b.bits(2)? {
            0 => {
                b.buf = 0; b.n = 0;
                let p = b.pos;
                let h = d.get(p..p + 4).ok_or("deflate: truncated")?;
                let len = u16::from_le_bytes([h[0], h[1]]) as usize;
                out.extend_from_slice(d.get(p + 4..p + 4 + len).ok_or("deflate: truncated")?);
                b.pos = p + 4 + len;
            }
            t @ (1 | 2) => {
                let (lit, dist);
                if t == 1 {
                    let mut l = [8u8; 288];
                    for x in &mut l[144..256] { *x = 9 }
                    for x in &mut l[256..280] { *x = 7 }
                    lit = Huff::new(&l);
                    dist = Huff::new(&[5u8; 30]);
                } else {
                    let (hlit, hdist, hclen) = (b.bits(5)? as usize + 257, b.bits(5)? as usize + 1, b.bits(4)? as usize + 4);
                    const ORD: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
                    let mut cl = [0u8; 19];
                    for &o in ORD.iter().take(hclen) { cl[o] = b.bits(3)? as u8 }
                    let ch = Huff::new(&cl);
                    let mut lens = vec![0u8; hlit + hdist];
                    let mut i = 0;
                    while i < lens.len() {
                        let s = ch.decode(&mut b)?;
                        let (v, rep) = match s {
                            0..=15 => (s as u8, 1),
                            16 => (*lens.get(i.wrapping_sub(1)).ok_or("deflate: bad repeat")?, 3 + b.bits(2)? as usize),
                            17 => (0, 3 + b.bits(3)? as usize),
                            _ => (0, 11 + b.bits(7)? as usize),
                        };
                        if i + rep > lens.len() { return Err("deflate: bad lengths".into()) }
                        for _ in 0..rep { lens[i] = v; i += 1 }
                    }
                    lit = Huff::new(&lens[..hlit]);
                    dist = Huff::new(&lens[hlit..]);
                }
                loop {
                    let s = lit.decode(&mut b)? as usize;
                    if s < 256 { out.push(s as u8) }
                    else if s == 256 { break }
                    else {
                        let s = s - 257;
                        if s >= 29 { return Err("deflate: bad length".into()) }
                        let len = LBASE[s] as usize + b.bits(LEXT[s] as u32)? as usize;
                        let ds = dist.decode(&mut b)? as usize;
                        if ds >= 30 { return Err("deflate: bad distance".into()) }
                        let dd = DBASE[ds] as usize + b.bits(DEXT[ds] as u32)? as usize;
                        if dd > out.len() { return Err("deflate: distance too far".into()) }
                        for _ in 0..len { out.push(out[out.len() - dd]) }
                    }
                    if out.len() > limit { return Err("deflate: output too large".into()) }
                }
            }
            _ => return Err("deflate: bad block".into()),
        }
        if out.len() > limit { return Err("deflate: output too large".into()) }
        if last == 1 { break }
    }
    Ok(out)
}

/// zlib container (2-byte header, adler32 ignored).
pub fn zlib(d: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    if d.len() < 2 || d[0] & 0x0f != 8 { return Err("zlib: bad header".into()) }
    inflate(&d[2..], limit)
}

pub fn adler32(d: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for c in d.chunks(5552) { for &x in c { a += x as u32; b += a } a %= 65521; b %= 65521 }
    (b << 16) | a
}

pub fn crc32(parts: &[&[u8]]) -> u32 {
    static mut T: [u32; 256] = [0; 256];
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| unsafe { for n in 0..256u32 { let mut c = n; for _ in 0..8 { c = if c & 1 != 0 { 0xEDB88320 ^ (c >> 1) } else { c >> 1 } } T[n as usize] = c } });
    let mut c = !0u32;
    for p in parts { for &x in *p { c = unsafe { T[((c ^ x as u32) & 0xff) as usize] } ^ (c >> 8) } }
    !c
}

struct Out { v: Vec<u8>, acc: u64, n: u32 }

impl Out {
    fn put(&mut self, val: u32, k: u32) { self.acc |= (val as u64) << self.n; self.n += k; while self.n >= 8 { self.v.push(self.acc as u8); self.acc >>= 8; self.n -= 8 } }
    /// Huffman codes are sent MSB first.
    fn code(&mut self, c: u32, k: u32) { let mut r = 0; for i in 0..k { r |= ((c >> i) & 1) << (k - 1 - i) } self.put(r, k) }
    fn lit(&mut self, s: u32) {
        match s { 0..=143 => self.code(0x30 + s, 8), 144..=255 => self.code(0x190 + s - 144, 9), 256..=279 => self.code(s - 256, 7), _ => self.code(0xC0 + s - 280, 8) }
    }
}

/// Fixed-Huffman deflate with a hash-chain LZ77 matcher.
pub fn deflate(d: &[u8]) -> Vec<u8> {
    let mut o = Out { v: vec![], acc: 0, n: 0 };
    o.put(1, 1);
    o.put(1, 2);
    let mut head = vec![usize::MAX; 1 << 15];
    let mut prev = vec![usize::MAX; d.len()];
    let hash = |i: usize| ((d[i] as usize) << 10 ^ (d[i + 1] as usize) << 5 ^ d[i + 2] as usize) & 0x7fff;
    let mut i = 0;
    while i < d.len() {
        let (mut bl, mut bd) = (0usize, 0usize);
        if i + 3 <= d.len() {
            let h = hash(i);
            let mut c = head[h];
            let mut tries = 32;
            while c != usize::MAX && i - c <= 32768 && tries > 0 {
                let mut l = 0;
                while l < 258 && i + l < d.len() && d[c + l] == d[i + l] { l += 1 }
                if l > bl { bl = l; bd = i - c; if l == 258 { break } }
                c = prev[c];
                tries -= 1;
            }
            prev[i] = head[h];
            head[h] = i;
        }
        if bl >= 3 {
            let ls = LBASE.iter().rposition(|&b| b as usize <= bl).unwrap();
            o.lit(257 + ls as u32);
            o.put((bl - LBASE[ls] as usize) as u32, LEXT[ls] as u32);
            let ds = DBASE.iter().rposition(|&b| b as usize <= bd).unwrap();
            o.code(ds as u32, 5);
            o.put((bd - DBASE[ds] as usize) as u32, DEXT[ds] as u32);
            for k in 1..bl { let j = i + k; if j + 3 <= d.len() { let h = hash(j); prev[j] = head[h]; head[h] = j; } }
            i += bl;
        } else { o.lit(d[i] as u32); i += 1 }
    }
    o.lit(256);
    o.put(0, 7);
    o.v
}
