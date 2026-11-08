//! Text measuring and glyph rasterisation (ab_glyph). Fonts are embedded
//! (Noto Sans, SIL OFL) so rendering is identical on every machine.

use ab_glyph::{Font, FontRef, ScaleFont};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

static REGULAR: &[u8] = include_bytes!("../assets/NotoSans-Regular.ttf");
static BOLD: &[u8] = include_bytes!("../assets/NotoSans-Bold.ttf");

fn font(bold: bool) -> &'static FontRef<'static> {
    static F: OnceLock<(FontRef<'static>, FontRef<'static>)> = OnceLock::new();
    let f = F.get_or_init(|| {
        (FontRef::try_from_slice(REGULAR).expect("regular font"), FontRef::try_from_slice(BOLD).expect("bold font"))
    });
    if bold { &f.1 } else { &f.0 }
}

pub struct Glyph {
    pub w: u32,
    pub h: u32,
    /// Offset of bitmap top-left from the pen origin / baseline.
    pub left: i32,
    pub top: i32,
    pub cov: Vec<u8>,
}

type Key = (char, u32, bool);
fn cache() -> &'static Mutex<HashMap<Key, std::sync::Arc<Glyph>>> {
    static C: OnceLock<Mutex<HashMap<Key, std::sync::Arc<Glyph>>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// `size` is in device pixels here (callers multiply by the DPI scale).
pub fn glyph(c: char, size: f32, bold: bool) -> std::sync::Arc<Glyph> {
    let key = (c, (size * 4.0) as u32, bold);
    if let Some(g) = cache().lock().unwrap().get(&key) {
        return g.clone();
    }
    let f = font(bold);
    let sg = f.glyph_id(c).with_scale(size);
    let g = match f.outline_glyph(sg) {
        Some(o) => {
            let b = o.px_bounds();
            let (w, h) = (b.width().ceil() as u32 + 1, b.height().ceil() as u32 + 1);
            let mut cov = vec![0u8; (w * h) as usize];
            o.draw(|x, y, c| {
                if x < w && y < h {
                    cov[(y * w + x) as usize] = (c * 255.0).round() as u8;
                }
            });
            Glyph { w, h, left: b.min.x.floor() as i32, top: b.min.y.floor() as i32, cov }
        }
        None => Glyph { w: 0, h: 0, left: 0, top: 0, cov: vec![] },
    };
    let g = std::sync::Arc::new(g);
    let mut c = cache().lock().unwrap();
    if c.len() > 8000 {
        c.clear();
    }
    c.insert(key, g.clone());
    g
}

pub fn advance(c: char, size: f32, bold: bool) -> f32 {
    let f = font(bold).as_scaled(size);
    f.h_advance(f.glyph_id(c))
}

pub fn ascent(size: f32, bold: bool) -> f32 {
    font(bold).as_scaled(size).ascent()
}

pub fn line_height(size: f32, bold: bool) -> f32 {
    let f = font(bold).as_scaled(size);
    f.ascent() - f.descent() + f.line_gap()
}

/// Width of `text` at `size` (same units as size).
pub fn measure(text: &str, size: f32, bold: bool) -> f32 {
    let f = font(bold).as_scaled(size);
    let mut w = 0.0;
    let mut prev = None;
    for c in text.chars() {
        let id = f.glyph_id(c);
        if let Some(p) = prev {
            w += f.kern(p, id);
        }
        w += f.h_advance(id);
        prev = Some(id);
    }
    w
}

/// Shorten `text` with an ellipsis in the middle-preserving-extension style:
/// keeps the end (extension) visible, like good file managers do.
pub fn ellipsize(text: &str, size: f32, bold: bool, max_w: f32) -> String {
    if measure(text, size, bold) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let ell = '…';
    let ew = advance(ell, size, bold);
    // keep up to the last 6 chars (extension) when the name is long enough
    let tail_n = if chars.len() > 12 { 5 } else { 0 };
    let tail: String = chars[chars.len() - tail_n..].iter().collect();
    let tw = measure(&tail, size, bold);
    let mut head = String::new();
    let mut w = ew + tw;
    for &c in &chars[..chars.len() - tail_n] {
        let a = advance(c, size, bold);
        if w + a > max_w {
            break;
        }
        w += a;
        head.push(c);
    }
    format!("{head}{ell}{tail}")
}

/// Character index at x offset (for caret placement).
pub fn index_at(text: &str, size: f32, x: f32) -> usize {
    let mut acc = 0.0;
    for (i, c) in text.chars().enumerate() {
        let a = advance(c, size, false);
        if x < acc + a / 2.0 {
            return i;
        }
        acc += a;
    }
    text.chars().count()
}
