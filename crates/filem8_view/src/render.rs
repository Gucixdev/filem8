//! Pages -> pixels: one inverse-mapped blit does zoom, rotate, flip and pan together.
//! Magnified pictures are drawn with hard pixels (pixel art stays crisp), shrunk ones are smoothed.

use fm8_ui::{draw::DrawList, geom::{rect, Color}, soft::Framebuffer, Theme};

use crate::{doc::Page, xform::Xform};

/// Where the picture goes in the framebuffer, in pixels.
#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

fn checker(x: u32, y: u32) -> u8 {
    if ((x / 8) + (y / 8)) % 2 == 0 { 58 } else { 84 }
}

fn texel(p: &Page, x: i64, y: i64) -> [u8; 4] {
    let (x, y) = (x.clamp(0, p.w as i64 - 1) as usize, y.clamp(0, p.h as i64 - 1) as usize);
    let i = (y * p.w as usize + x) * 4;
    [p.rgba[i], p.rgba[i + 1], p.rgba[i + 2], p.rgba[i + 3]]
}

fn bilinear(p: &Page, sx: f32, sy: f32) -> [u8; 4] {
    let (fx, fy) = (sx - 0.5, sy - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let (x0, y0) = (x0 as i64, y0 as i64);
    let (a, b, c, d) = (texel(p, x0, y0), texel(p, x0 + 1, y0), texel(p, x0, y0 + 1), texel(p, x0 + 1, y0 + 1));
    let mut out = [0u8; 4];
    for k in 0..4 {
        let top = a[k] as f32 * (1.0 - tx) + b[k] as f32 * tx;
        let bot = c[k] as f32 * (1.0 - tx) + d[k] as f32 * tx;
        out[k] = (top * (1.0 - ty) + bot * ty + 0.5) as u8;
    }
    out
}

pub fn draw_page(fb: &mut Framebuffer, vp: Viewport, page: &Page, xf: &Xform, bg: Color) {
    let view = (vp.w as f32, vp.h as f32);
    let img = (page.w as f32, page.h as f32);
    let m = xf.matrix(view, img);
    let magnified = xf.scale(view, img) >= 1.5;
    for y in 0..vp.h {
        if vp.y + y >= fb.h {
            break;
        }
        for x in 0..vp.w {
            if vp.x + x >= fb.w {
                break;
            }
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let (sx, sy) = (m[0] * px + m[1] * py + m[2], m[3] * px + m[4] * py + m[5]);
            let inside = sx >= 0.0 && sy >= 0.0 && sx < img.0 && sy < img.1;
            let rgba = if !inside {
                [bg.r, bg.g, bg.b, 255]
            } else if magnified {
                texel(page, sx as i64, sy as i64)
            } else {
                bilinear(page, sx, sy)
            };
            let o = (((vp.y + y) * fb.w + vp.x + x) * 4) as usize;
            if rgba[3] == 255 {
                fb.px[o..o + 3].copy_from_slice(&rgba[..3]);
            } else {
                let under = checker(x, y) as u32;
                let a = rgba[3] as u32;
                for k in 0..3 {
                    fb.px[o + k] = ((rgba[k] as u32 * a + under * (255 - a) + 127) / 255) as u8;
                }
            }
            fb.px[o + 3] = 255;
        }
    }
}

/// The picture with rotation and flips baked into its pixels: `quarter` clockwise turns, then mirrors in screen space
/// (same meaning as `Xform`). For toolkits that can only draw axis-aligned images. Returns (w, h, rgba).
pub fn orient(w: u32, h: u32, rgba: &[u8], quarter: u8, flip_x: bool, flip_y: bool) -> (u32, u32, Vec<u8>) {
    let q = quarter % 4;
    let (ow, oh) = if q % 2 == 1 { (h, w) } else { (w, h) };
    if q == 0 && !flip_x && !flip_y {
        return (w, h, rgba.to_vec());
    }
    let mut out = vec![0u8; (ow * oh * 4) as usize];
    for y in 0..oh {
        for x in 0..ow {
            let (fx, fy) = (if flip_x { ow - 1 - x } else { x }, if flip_y { oh - 1 - y } else { y });
            let (sx, sy) = match q {
                0 => (fx, fy),
                1 => (fy, h - 1 - fx),
                2 => (w - 1 - fx, h - 1 - fy),
                _ => (w - 1 - fy, fx),
            };
            let (i, o) = (((sy * w + sx) * 4) as usize, ((y * ow + x) * 4) as usize);
            out[o..o + 4].copy_from_slice(&rgba[i..i + 4]);
        }
    }
    (ow, oh, out)
}

/// A one-line bar along the bottom with `left` and `right` text.
pub fn status_bar(fb: &mut Framebuffer, t: &Theme, scale: f32, left: &str, right: &str) -> u32 {
    let bar_h = (22.0 * scale).ceil() as u32;
    let top = fb.h.saturating_sub(bar_h);
    let mut dl = DrawList::default();
    let (w, h) = (fb.w as f32 / scale, bar_h as f32 / scale);
    dl.rect(rect(0.0, 0.0, w, h), t.bg_alt);
    dl.text(8.0, 4.0, left, 12.0, t.fg, false);
    let rw = fm8_ui::text::measure(right, 12.0, false);
    dl.text((w - rw - 8.0).max(8.0), 4.0, right, 12.0, t.fg_dim, false);
    // render the bar into its own strip, then copy it down (the rasteriser draws from the top-left)
    let mut strip = Framebuffer::new(fb.w, bar_h, t.bg_alt);
    strip.render(&dl, scale);
    let from = (top * fb.w * 4) as usize;
    fb.px[from..from + strip.px.len()].copy_from_slice(&strip.px);
    bar_h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page() -> Page {
        // 2x1: red, blue
        Page { w: 2, h: 1, rgba: vec![255, 0, 0, 255, 0, 0, 255, 255], delay_ms: 100 }
    }

    fn px(fb: &Framebuffer, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * fb.w + x) * 4) as usize;
        [fb.px[i], fb.px[i + 1], fb.px[i + 2]]
    }

    #[test]
    fn zoomed_pixels_stay_hard_and_rotation_moves_them() {
        let bg = Color { r: 0, g: 0, b: 0, a: 255 };
        let vp = Viewport { x: 0, y: 0, w: 40, h: 40 };
        let mut xf = Xform::default();
        xf.zoom = 10.0; // 20 x 10 window pixels, centred
        let mut fb = Framebuffer::new(40, 40, bg);
        draw_page(&mut fb, vp, &page(), &xf, bg);
        assert_eq!(px(&fb, 12, 20), [255, 0, 0]);
        assert_eq!(px(&fb, 28, 20), [0, 0, 255]);
        assert_eq!(px(&fb, 2, 2), [0, 0, 0], "outside is the background");
        // a clockwise quarter turn puts the left (red) pixel on top
        xf.rotate(true);
        let mut fb = Framebuffer::new(40, 40, bg);
        draw_page(&mut fb, vp, &page(), &xf, bg);
        assert_eq!(px(&fb, 20, 12), [255, 0, 0]);
        assert_eq!(px(&fb, 20, 28), [0, 0, 255]);
    }

    #[test]
    fn orient_agrees_with_the_inverse_map_for_every_orientation() {
        // 3x2 picture with a distinct colour per pixel
        let rgba: Vec<u8> = (0..6u8).flat_map(|i| [i * 40, 255 - i * 40, i, 255]).collect();
        let page = Page { w: 3, h: 2, rgba: rgba.clone(), delay_ms: 100 };
        for q in 0..4u8 {
            for (fx, fy) in [(false, false), (true, false), (false, true), (true, true)] {
                let (ow, oh, out) = orient(3, 2, &rgba, q, fx, fy);
                let mut xf = Xform::default();
                xf.zoom = 1.0;
                xf.quarter = q;
                xf.flip_x = fx;
                xf.flip_y = fy;
                let view = (ow as f32, oh as f32);
                for y in 0..oh {
                    for x in 0..ow {
                        let (sx, sy) = xf.to_source(view, (3.0, 2.0), (x as f32 + 0.5, y as f32 + 0.5));
                        let i = ((sy.floor() as u32 * page.w + sx.floor() as u32) * 4) as usize;
                        let o = ((y * ow + x) * 4) as usize;
                        assert_eq!(out[o..o + 4], page.rgba[i..i + 4], "q={q} fx={fx} fy={fy} at {x},{y}");
                    }
                }
            }
        }
    }

    #[test]
    fn transparency_shows_a_checkerboard() {
        let bg = Color { r: 0, g: 0, b: 0, a: 255 };
        let p = Page { w: 1, h: 1, rgba: vec![255, 255, 255, 0], delay_ms: 100 };
        let mut xf = Xform::default();
        xf.zoom = 40.0;
        let mut fb = Framebuffer::new(40, 40, bg);
        draw_page(&mut fb, Viewport { x: 0, y: 0, w: 40, h: 40 }, &p, &xf, bg);
        assert_ne!(px(&fb, 2, 2), px(&fb, 10, 2));
    }
}
