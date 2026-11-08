//! CPU rasteriser for the display list. This is the software fallback (works
//! with no GPU / no Vulkan ICD), and what the headless golden tests use.

use crate::draw::{Cmd, DrawList, Image};
use crate::geom::{Color, Rect};
use crate::text;

pub struct Framebuffer {
    pub w: u32,
    pub h: u32,
    /// RGBA8, always opaque after `clear`.
    pub px: Vec<u8>,
}

#[derive(Clone, Copy)]
struct Clip {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl Framebuffer {
    pub fn new(w: u32, h: u32, bg: Color) -> Framebuffer {
        let mut f = Framebuffer { w, h, px: vec![0; (w * h * 4) as usize] };
        f.clear(bg);
        f
    }
    pub fn clear(&mut self, c: Color) {
        for p in self.px.chunks_exact_mut(4) {
            p.copy_from_slice(&[c.r, c.g, c.b, 255]);
        }
    }
    pub fn pixel(&self, x: u32, y: u32) -> Color {
        let i = ((y * self.w + x) * 4) as usize;
        Color { r: self.px[i], g: self.px[i + 1], b: self.px[i + 2], a: self.px[i + 3] }
    }
    #[inline]
    fn blend(&mut self, x: i32, y: i32, c: Color, cov: f32) {
        let a = cov * c.a as f32 / 255.0;
        if a <= 0.0 {
            return;
        }
        let i = ((y as u32 * self.w + x as u32) * 4) as usize;
        let d = &mut self.px[i..i + 3];
        d[0] = (d[0] as f32 + (c.r as f32 - d[0] as f32) * a) as u8;
        d[1] = (d[1] as f32 + (c.g as f32 - d[1] as f32) * a) as u8;
        d[2] = (d[2] as f32 + (c.b as f32 - d[2] as f32) * a) as u8;
    }

    /// Render `list` (logical coordinates) with DPI `scale`.
    pub fn render(&mut self, list: &DrawList, scale: f32) {
        let full = Clip { x0: 0, y0: 0, x1: self.w as i32, y1: self.h as i32 };
        let mut stack = vec![full];
        for cmd in &list.cmds {
            let clip = *stack.last().unwrap();
            match cmd {
                Cmd::PushClip(r) => {
                    let n = Clip {
                        x0: ((r.x * scale).floor() as i32).max(clip.x0),
                        y0: ((r.y * scale).floor() as i32).max(clip.y0),
                        x1: ((r.right() * scale).ceil() as i32).min(clip.x1),
                        y1: ((r.bottom() * scale).ceil() as i32).min(clip.y1),
                    };
                    stack.push(n);
                }
                Cmd::PopClip => {
                    if stack.len() > 1 {
                        stack.pop();
                    }
                }
                Cmd::Rect { r, color, radius, border } => {
                    self.rrect(*r, *color, *radius, *border, scale, clip)
                }
                Cmd::Shadow { r, radius, blur, color } => self.shadow(*r, *radius, *blur, *color, scale, clip),
                Cmd::Text { x, y, text: t, size, color, bold } => {
                    self.text(*x, *y, t, *size, *color, *bold, scale, clip)
                }
                Cmd::Line { a, b, width, color } => self.line(*a, *b, *width, *color, scale, clip),
                Cmd::Image { r, img, smooth } => self.image(*r, img, *smooth, scale, clip),
            }
        }
    }

    fn rrect(&mut self, r: Rect, color: Color, radius: f32, border: Option<(f32, Color)>, s: f32, clip: Clip) {
        let (x, y, w, h) = (r.x * s, r.y * s, r.w * s, r.h * s);
        let rad = (radius * s).min(w / 2.0).min(h / 2.0).max(0.0);
        let x0 = (x.floor() as i32).max(clip.x0);
        let y0 = (y.floor() as i32).max(clip.y0);
        let x1 = ((x + w).ceil() as i32).min(clip.x1);
        let y1 = ((y + h).ceil() as i32).min(clip.y1);
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        let (hx, hy) = (w / 2.0 - rad, h / 2.0 - rad);
        for py in y0..y1 {
            for px in x0..x1 {
                let (dx, dy) = ((px as f32 + 0.5 - cx).abs() - hx, (py as f32 + 0.5 - cy).abs() - hy);
                let d = dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - rad;
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov <= 0.0 {
                    continue;
                }
                if let Some((bw, bc)) = border {
                    let bw = bw * s;
                    let inner = (0.5 - (d + bw)).clamp(0.0, 1.0);
                    if color.a > 0 {
                        self.blend(px, py, color, inner);
                    }
                    self.blend(px, py, bc, cov - inner);
                } else {
                    self.blend(px, py, color, cov);
                }
            }
        }
    }

    fn line(&mut self, a: (f32, f32), b: (f32, f32), width: f32, color: Color, s: f32, clip: Clip) {
        let (ax, ay, bx, by) = (a.0 * s, a.1 * s, b.0 * s, b.1 * s);
        let hw = width * s / 2.0;
        let x0 = ((ax.min(bx) - hw - 1.0).floor() as i32).max(clip.x0);
        let y0 = ((ay.min(by) - hw - 1.0).floor() as i32).max(clip.y0);
        let x1 = ((ax.max(bx) + hw + 1.0).ceil() as i32).min(clip.x1);
        let y1 = ((ay.max(by) + hw + 1.0).ceil() as i32).min(clip.y1);
        let (dx, dy) = (bx - ax, by - ay);
        let l2 = (dx * dx + dy * dy).max(1e-6);
        for py in y0..y1 {
            for px in x0..x1 {
                let (qx, qy) = (px as f32 + 0.5 - ax, py as f32 + 0.5 - ay);
                let t = ((qx * dx + qy * dy) / l2).clamp(0.0, 1.0);
                let d = (qx - t * dx).hypot(qy - t * dy) - hw;
                self.blend(px, py, color, (0.5 - d).clamp(0.0, 1.0));
            }
        }
    }

    fn shadow(&mut self, r: Rect, radius: f32, blur: f32, color: Color, s: f32, clip: Clip) {
        let (blur, rad) = (blur * s, radius * s);
        let (x, y, w, h) = (r.x * s, r.y * s, r.w * s, r.h * s);
        let x0 = ((x - blur).floor() as i32).max(clip.x0);
        let y0 = ((y - blur).floor() as i32).max(clip.y0);
        let x1 = ((x + w + blur).ceil() as i32).min(clip.x1);
        let y1 = ((y + h + blur).ceil() as i32).min(clip.y1);
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        let (hx, hy) = (w / 2.0 - rad, h / 2.0 - rad);
        for py in y0..y1 {
            for px in x0..x1 {
                let (dx, dy) = ((px as f32 + 0.5 - cx).abs() - hx, (py as f32 + 0.5 - cy).abs() - hy);
                let d = dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - rad;
                let t = (1.0 - d / blur).clamp(0.0, 1.0);
                self.blend(px, py, color, t * t * (3.0 - 2.0 * t));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, x: f32, y: f32, t: &str, size: f32, color: Color, bold: bool, s: f32, clip: Clip) {
        let size = size * s;
        let base = y * s + text::ascent(size, bold);
        let mut pen = x * s;
        let mut prev: Option<char> = None;
        for c in t.chars() {
            let _ = prev;
            let g = text::glyph(c, size, bold);
            let gx = pen.round() as i32 + g.left;
            let gy = base.round() as i32 + g.top;
            for yy in 0..g.h as i32 {
                let py = gy + yy;
                if py < clip.y0 || py >= clip.y1 {
                    continue;
                }
                for xx in 0..g.w as i32 {
                    let px = gx + xx;
                    if px < clip.x0 || px >= clip.x1 {
                        continue;
                    }
                    let cv = g.cov[(yy as u32 * g.w + xx as u32) as usize];
                    if cv > 0 {
                        self.blend(px, py, color, cv as f32 / 255.0);
                    }
                }
            }
            pen += text::advance(c, size, bold);
            prev = Some(c);
        }
    }

    fn image(&mut self, r: Rect, img: &Image, smooth: bool, s: f32, clip: Clip) {
        let (x, y, w, h) = (r.x * s, r.y * s, r.w * s, r.h * s);
        if w <= 0.0 || h <= 0.0 || img.w == 0 || img.h == 0 {
            return;
        }
        let x0 = (x.floor() as i32).max(clip.x0);
        let y0 = (y.floor() as i32).max(clip.y0);
        let x1 = ((x + w).ceil() as i32).min(clip.x1);
        let y1 = ((y + h).ceil() as i32).min(clip.y1);
        let get = |ix: i32, iy: i32| -> [f32; 4] {
            let ix = ix.clamp(0, img.w as i32 - 1) as usize;
            let iy = iy.clamp(0, img.h as i32 - 1) as usize;
            let i = (iy * img.w as usize + ix) * 4;
            [img.rgba[i] as f32, img.rgba[i + 1] as f32, img.rgba[i + 2] as f32, img.rgba[i + 3] as f32]
        };
        for py in y0..y1 {
            for px in x0..x1 {
                let u = ((px as f32 + 0.5 - x) / w * img.w as f32).clamp(0.0, img.w as f32);
                let v = ((py as f32 + 0.5 - y) / h * img.h as f32).clamp(0.0, img.h as f32);
                let p = if smooth {
                    let (fu, fv) = (u - 0.5, v - 0.5);
                    let (iu, iv) = (fu.floor(), fv.floor());
                    let (tu, tv) = (fu - iu, fv - iv);
                    let (a, b, c, d) = (get(iu as i32, iv as i32), get(iu as i32 + 1, iv as i32), get(iu as i32, iv as i32 + 1), get(iu as i32 + 1, iv as i32 + 1));
                    let mut o = [0.0; 4];
                    for k in 0..4 {
                        o[k] = (a[k] * (1.0 - tu) + b[k] * tu) * (1.0 - tv) + (c[k] * (1.0 - tu) + d[k] * tu) * tv;
                    }
                    o
                } else {
                    get(u as i32, v as i32)
                };
                self.blend(px, py, Color { r: p[0] as u8, g: p[1] as u8, b: p[2] as u8, a: 255 }, p[3] / 255.0);
            }
        }
    }

    pub fn save_png(&self, path: &std::path::Path) -> Result<(), String> {
        let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), self.w, self.h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(|e| e.to_string())?;
        w.write_image_data(&self.px).map_err(|e| e.to_string())
    }

    pub fn load_png(path: &std::path::Path) -> Result<Framebuffer, String> {
        let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).map_err(|e| e.to_string())?));
        let mut r = dec.read_info().map_err(|e| e.to_string())?;
        let mut buf = vec![0; r.output_buffer_size().ok_or("size")?];
        let info = r.next_frame(&mut buf).map_err(|e| e.to_string())?;
        if info.color_type != png::ColorType::Rgba {
            return Err("golden must be RGBA".into());
        }
        buf.truncate(info.buffer_size());
        Ok(Framebuffer { w: info.width, h: info.height, px: buf })
    }

    /// Count of pixels differing by more than `tol` in any channel.
    pub fn diff(&self, o: &Framebuffer, tol: u8) -> usize {
        if self.w != o.w || self.h != o.h {
            return usize::MAX;
        }
        self.px.chunks_exact(4).zip(o.px.chunks_exact(4)).filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > tol)).count()
    }
}
