//! Radiance RGBE (.hdr), tone-mapped to 8-bit sRGB-ish (clamp + gamma 2.2).
use crate::Image;

pub fn decode(b: &[u8]) -> Result<Image, String> {
    let mut p = 0;
    let mut line = || -> Option<String> { let s = p; while *b.get(p)? != b'\n' { p += 1 } p += 1; Some(String::from_utf8_lossy(&b[s..p - 1]).into_owned()) };
    loop { if line().ok_or("hdr: header")?.is_empty() { break } }
    let res = line().ok_or("hdr: resolution")?;
    let t: Vec<&str> = res.split_whitespace().collect();
    if t.len() != 4 || t[0].len() != 2 || t[2].len() != 2 { return Err("hdr: bad resolution".into()) }
    let (h, w): (usize, usize) = (t[1].parse().map_err(|_| "hdr: h")?, t[3].parse().map_err(|_| "hdr: w")?);
    if t[0] != "-Y" && t[0] != "+Y" || w == 0 || h == 0 || w * h > 1 << 28 { return Err("hdr: unsupported orientation/size".into()) }
    let flip_y = t[0] == "+Y";
    let flip_x = t[2] == "-X";
    let mut img = Image::new(w as u32, h as u32);
    let mut row = vec![[0u8; 4]; w];
    for y in 0..h {
        if (8..0x8000).contains(&w) && b.get(p..p + 4).map(|s| s[0] == 2 && s[1] == 2 && (((s[2] as usize) << 8) | s[3] as usize) == w).unwrap_or(false) {
            p += 4;
            for c in 0..4 {
                let mut x = 0;
                while x < w {
                    let n = *b.get(p).ok_or("hdr: truncated")? as usize;
                    p += 1;
                    if n > 128 { let v = *b.get(p).ok_or("hdr: truncated")?; p += 1; for _ in 0..n - 128 { if x < w { row[x][c] = v; x += 1 } } }
                    else { for _ in 0..n { if x < w { row[x][c] = *b.get(p).ok_or("hdr: truncated")?; p += 1; x += 1 } } }
                }
            }
        } else {
            for r in row.iter_mut() { r.copy_from_slice(b.get(p..p + 4).ok_or("hdr: truncated")?); p += 4 }
        }
        let oy = if flip_y { h - 1 - y } else { y };
        for (x, r) in row.iter().enumerate() {
            let ox = if flip_x { w - 1 - x } else { x };
            let e = if r[3] == 0 { 0.0 } else { 2f32.powi(r[3] as i32 - 136) };
            let m = |v: u8| ((v as f32 * e).clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0 + 0.5) as u8;
            img.rgba[(oy * w + ox) * 4..][..4].copy_from_slice(&[m(r[0]), m(r[1]), m(r[2]), 255]);
        }
    }
    Ok(img)
}
