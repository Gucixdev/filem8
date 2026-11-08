//! Backend-independent display list. The widget layer only emits `Cmd`s;
//! `soft` (CPU) and the wgpu backend in the app turn them into pixels.

use crate::geom::{Color, Rect};
use std::sync::Arc;

#[derive(Debug)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    /// RGBA8, straight alpha.
    pub rgba: Vec<u8>,
    /// Unique id so GPU backends can cache textures.
    pub id: u64,
}

static NEXT_IMG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Image {
    pub fn new(w: u32, h: u32, rgba: Vec<u8>) -> Arc<Image> {
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        let id = NEXT_IMG.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Arc::new(Image { w, h, rgba, id })
    }
}

#[derive(Clone, Debug)]
pub enum Cmd {
    /// Rounded rect with optional border (width, colour).
    Rect { r: Rect, color: Color, radius: f32, border: Option<(f32, Color)> },
    Shadow { r: Rect, radius: f32, blur: f32, color: Color },
    Text { x: f32, y: f32, text: String, size: f32, color: Color, bold: bool },
    /// Image scaled to fill `r` (caller computes aspect fit). `tint` multiplies.
    Image { r: Rect, img: Arc<Image>, smooth: bool },
    Line { a: (f32, f32), b: (f32, f32), width: f32, color: Color },
    PushClip(Rect),
    PopClip,
}

#[derive(Default, Clone, Debug)]
pub struct DrawList {
    pub cmds: Vec<Cmd>,
}

impl DrawList {
    pub fn rect(&mut self, r: Rect, color: Color) {
        self.rounded(r, color, 0.0);
    }
    pub fn rounded(&mut self, r: Rect, color: Color, radius: f32) {
        if color.a > 0 && !r.is_empty() {
            self.cmds.push(Cmd::Rect { r, color, radius, border: None });
        }
    }
    pub fn stroke(&mut self, r: Rect, color: Color, radius: f32, width: f32) {
        self.cmds.push(Cmd::Rect { r, color: Color::TRANSPARENT, radius, border: Some((width, color)) });
    }
    pub fn text(&mut self, x: f32, y: f32, text: &str, size: f32, color: Color, bold: bool) {
        if !text.is_empty() {
            self.cmds.push(Cmd::Text { x, y, text: text.to_string(), size, color, bold });
        }
    }
    pub fn line(&mut self, a: (f32, f32), b: (f32, f32), width: f32, color: Color) {
        self.cmds.push(Cmd::Line { a, b, width, color });
    }
    pub fn push_clip(&mut self, r: Rect) {
        self.cmds.push(Cmd::PushClip(r));
    }
    pub fn pop_clip(&mut self) {
        self.cmds.push(Cmd::PopClip);
    }
    pub fn extend(&mut self, o: DrawList) {
        self.cmds.extend(o.cmds);
    }
}
