//! Freedesktop thumbnail cache naming (Thumbnail Managing Standard): the file
//! name is the MD5 of the canonical `file://` URI. Std-only MD5 (RFC 1321).
use std::path::{Path, PathBuf};

pub fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20,
        4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64).map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32).collect();
    let (mut a0, mut b0, mut c0, mut d0) = (0x67452301u32, 0xefcdab89u32, 0x98badcfeu32, 0x10325476u32);
    let mut m = data.to_vec();
    m.push(0x80);
    while m.len() % 64 != 56 { m.push(0) }
    m.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    for ch in m.chunks(64) {
        let w: Vec<u32> = ch.chunks(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f2 = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(w[g]);
            a = d; d = c; c = b;
            b = b.wrapping_add(f2.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a); b0 = b0.wrapping_add(b); c0 = c0.wrapping_add(c); d0 = d0.wrapping_add(d);
    }
    let mut o = [0u8; 16];
    for (i, v) in [a0, b0, c0, d0].iter().enumerate() { o[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes()) }
    o
}

pub fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{:02x}", x)).collect() }

/// `file://` URI with the characters glib escapes percent-encoded.
pub fn uri(p: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut s = String::from("file://");
    for &c in p.as_os_str().as_bytes() {
        if c.is_ascii_alphanumeric() || b"/-_.~!*'()@&=+$,:".contains(&c) { s.push(c as char) } else { s.push_str(&format!("%{:02X}", c)) }
    }
    s
}

pub fn cache_root() -> PathBuf {
    if let Some(x) = std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()) { return PathBuf::from(x).join("thumbnails") }
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache/thumbnails")
}

/// Cache size class for a wanted edge in px: 128 `normal`, 256 `large`, 512 `x-large`.
pub fn class_px(want: u32) -> u32 { if want <= 128 { 128 } else if want <= 256 { 256 } else { 512 } }

/// Cache file for `p` in the class `px` (128/256/512, see `class_px`).
pub fn cache_file(p: &Path, px: u32) -> PathBuf {
    cache_root().join(match px { 128 => "normal", 256 => "large", _ => "x-large" }).join(format!("{}.png", hex(&md5(uri(p).as_bytes()))))
}
