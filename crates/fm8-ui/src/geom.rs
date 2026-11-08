//! Basic geometry and colour types. All UI coordinates are logical pixels.

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}
pub fn v2(x: f32, y: f32) -> Vec2 {
    Vec2 { x, y }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// Size of one child in a flex row/column.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    Fixed(f32),
    /// Share of the leftover space, proportional to the weight.
    Flex(f32),
}

impl Rect {
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.x && p.y >= self.y && p.x < self.right() && p.y < self.bottom()
    }
    pub fn inset(&self, d: f32) -> Rect {
        rect(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0))
    }
    pub fn inset_xy(&self, dx: f32, dy: f32) -> Rect {
        rect(self.x + dx, self.y + dy, (self.w - 2.0 * dx).max(0.0), (self.h - 2.0 * dy).max(0.0))
    }
    pub fn center(&self) -> Vec2 {
        v2(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    pub fn intersect(&self, o: Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        rect(x, y, (r - x).max(0.0), (b - y).max(0.0))
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }
    /// Cut `w` off the left side; returns (cut, rest).
    pub fn cut_left(&self, w: f32) -> (Rect, Rect) {
        let w = w.min(self.w).max(0.0);
        (rect(self.x, self.y, w, self.h), rect(self.x + w, self.y, self.w - w, self.h))
    }
    pub fn cut_right(&self, w: f32) -> (Rect, Rect) {
        let w = w.min(self.w).max(0.0);
        (rect(self.right() - w, self.y, w, self.h), rect(self.x, self.y, self.w - w, self.h))
    }
    pub fn cut_top(&self, h: f32) -> (Rect, Rect) {
        let h = h.min(self.h).max(0.0);
        (rect(self.x, self.y, self.w, h), rect(self.x, self.y + h, self.w, self.h - h))
    }
    pub fn cut_bottom(&self, h: f32) -> (Rect, Rect) {
        let h = h.min(self.h).max(0.0);
        (rect(self.x, self.bottom() - h, self.w, h), rect(self.x, self.y, self.w, self.h - h))
    }
    /// Flex layout: lay `items` out along `axis` with `gap` between them.
    pub fn flex(&self, axis: Axis, gap: f32, items: &[Size]) -> Vec<Rect> {
        let total = if axis == Axis::Horizontal { self.w } else { self.h };
        let gaps = gap * items.len().saturating_sub(1) as f32;
        let fixed: f32 = items.iter().map(|s| if let Size::Fixed(f) = s { *f } else { 0.0 }).sum();
        let weights: f32 = items.iter().map(|s| if let Size::Flex(f) = s { *f } else { 0.0 }).sum();
        let left = (total - gaps - fixed).max(0.0);
        let mut pos = 0.0;
        let mut out = Vec::with_capacity(items.len());
        for s in items {
            let len = match s {
                Size::Fixed(f) => *f,
                Size::Flex(f) => if weights > 0.0 { left * f / weights } else { 0.0 },
            };
            out.push(match axis {
                Axis::Horizontal => rect(self.x + pos, self.y, len, self.h),
                Axis::Vertical => rect(self.x, self.y + pos, self.w, len),
            });
            pos += len + gap;
        }
        out
    }
    pub fn row(&self, gap: f32, items: &[Size]) -> Vec<Rect> {
        self.flex(Axis::Horizontal, gap, items)
    }
    pub fn col(&self, gap: f32, items: &[Size]) -> Vec<Rect> {
        self.flex(Axis::Vertical, gap, items)
    }
    /// A `w`x`h` rect centred inside self.
    pub fn centered(&self, w: f32, h: f32) -> Rect {
        rect(self.x + (self.w - w) / 2.0, self.y + (self.h - h) / 2.0, w, h)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color { r, g, b, a: 255 }
}
pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color { r, g, b, a }
}

impl Color {
    pub const TRANSPARENT: Color = rgba(0, 0, 0, 0);
    /// Parse `#rrggbb`, `#rrggbbaa`, `#rgb`.
    pub fn parse(s: &str) -> Option<Color> {
        let h = s.trim().strip_prefix('#')?;
        let p = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
        match h.len() {
            6 => Some(rgb(p(0)?, p(2)?, p(4)?)),
            8 => Some(rgba(p(0)?, p(2)?, p(4)?, p(6)?)),
            3 => {
                let q = |i: usize| u8::from_str_radix(h.get(i..i + 1)?, 16).ok().map(|v| v * 17);
                Some(rgb(q(0)?, q(1)?, q(2)?))
            }
            _ => None,
        }
    }
    pub fn with_alpha(self, a: u8) -> Color {
        Color { a, ..self }
    }
    /// Mix toward `o` by `t` (0..1).
    pub fn mix(self, o: Color, t: f32) -> Color {
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round().clamp(0.0, 255.0) as u8;
        Color { r: m(self.r, o.r), g: m(self.g, o.g), b: m(self.b, o.b), a: m(self.a, o.a) }
    }
    /// Relative luminance (WCAG).
    pub fn luminance(&self) -> f32 {
        let f = |c: u8| {
            let c = c as f32 / 255.0;
            if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * f(self.r) + 0.7152 * f(self.g) + 0.0722 * f(self.b)
    }
}
