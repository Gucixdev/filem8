//! Box/bilinear downscale of RGBA8.
use crate::Image;

pub fn fit(w: u32, h: u32, max: u32) -> (u32, u32) {
    if w <= max && h <= max { return (w, h) }
    let s = max as f32 / w.max(h) as f32;
    (((w as f32 * s).round() as u32).max(1), ((h as f32 * s).round() as u32).max(1))
}

/// Area-averaged (premultiplied) resize; nearest when upscaling.
pub fn resize(src: &Image, nw: u32, nh: u32, nearest: bool) -> Image {
    if (nw, nh) == (src.w, src.h) { return src.clone() }
    let mut out = Image::new(nw, nh);
    let (sw, sh) = (src.w as usize, src.h as usize);
    for y in 0..nh as usize {
        let (y0, y1) = (y * sh / nh as usize, ((y + 1) * sh).div_ceil(nh as usize).min(sh).max(y * sh / nh as usize + 1));
        for x in 0..nw as usize {
            let (x0, x1) = (x * sw / nw as usize, ((x + 1) * sw).div_ceil(nw as usize).min(sw).max(x * sw / nw as usize + 1));
            let o = (y * nw as usize + x) * 4;
            if nearest || (x1 - x0 == 1 && y1 - y0 == 1) { out.rgba[o..o + 4].copy_from_slice(&src.rgba[(y0 * sw + x0) * 4..][..4]); continue }
            let (mut r, mut g, mut b, mut a, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
            for yy in y0..y1 { for xx in x0..x1 {
                let p = &src.rgba[(yy * sw + xx) * 4..][..4];
                let al = p[3] as u32;
                r += p[0] as u32 * al; g += p[1] as u32 * al; b += p[2] as u32 * al; a += al; n += 1;
            } }
            if a > 0 { out.rgba[o..o + 4].copy_from_slice(&[(r / a) as u8, (g / a) as u8, (b / a) as u8, (a / n) as u8]) }
        }
    }
    out
}
