//! How a picture sits in the window. `zoom == 0.0` means "fit" (shrink to the window, never grow past 100%).
//! Rotation is in quarter turns clockwise, flips are applied after rotating, in screen space.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xform {
    pub zoom: f32,
    pub quarter: u8,
    pub flip_x: bool,
    pub flip_y: bool,
    /// shift of the picture's centre from the window's centre, in window pixels
    pub pan: (f32, f32),
}

impl Default for Xform {
    fn default() -> Self {
        Xform { zoom: 0.0, quarter: 0, flip_x: false, flip_y: false, pan: (0.0, 0.0) }
    }
}

pub const MIN_ZOOM: f32 = 0.02;
pub const MAX_ZOOM: f32 = 64.0;

impl Xform {
    /// Size of the picture after rotating.
    pub fn oriented(&self, img: (f32, f32)) -> (f32, f32) {
        if self.quarter % 2 == 1 { (img.1, img.0) } else { img }
    }

    /// Window pixels per picture pixel.
    pub fn scale(&self, view: (f32, f32), img: (f32, f32)) -> f32 {
        if self.zoom > 0.0 {
            return self.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        }
        let (ow, oh) = self.oriented(img);
        (view.0 / ow.max(1.0)).min(view.1 / oh.max(1.0)).clamp(MIN_ZOOM, 1.0)
    }

    pub fn rotate(&mut self, clockwise: bool) {
        self.quarter = if clockwise { (self.quarter + 1) % 4 } else { (self.quarter + 3) % 4 };
        self.pan = (0.0, 0.0);
    }

    pub fn reset(&mut self) {
        *self = Xform::default();
    }

    /// Back to fit, keeping the rotation and flips.
    pub fn reset_zoom(&mut self) {
        self.zoom = 0.0;
        self.pan = (0.0, 0.0);
    }

    pub fn actual(&mut self) {
        self.zoom = 1.0;
        self.pan = (0.0, 0.0);
    }

    /// Reading position: 100%, left edge and top of the page at the window's edges (fit-width when the page is wider).
    pub fn top_left(&mut self, view: (f32, f32), img: (f32, f32)) {
        self.quarter = 0;
        self.zoom = (view.0 / img.0.max(1.0)).clamp(MIN_ZOOM, 1.0);
        self.pan = (0.0, (img.1 * self.zoom - view.1) / 2.0);
        self.clamp_pan(view, img);
    }

    pub fn pan_by(&mut self, dx: f32, dy: f32, view: (f32, f32), img: (f32, f32)) {
        self.pan = (self.pan.0 + dx, self.pan.1 + dy);
        self.clamp_pan(view, img);
    }

    /// Keep some of the picture on screen.
    pub fn clamp_pan(&mut self, view: (f32, f32), img: (f32, f32)) {
        let s = self.scale(view, img);
        let (ow, oh) = self.oriented(img);
        let lim = |o: f32, v: f32| ((o * s + v) / 2.0 - 48.0).max(0.0);
        self.pan.0 = self.pan.0.clamp(-lim(ow, view.0), lim(ow, view.0));
        self.pan.1 = self.pan.1.clamp(-lim(oh, view.1), lim(oh, view.1));
    }

    /// Zoom by `factor`, keeping the picture point under `at` (window pixels) where it is.
    pub fn zoom_by(&mut self, factor: f32, at: (f32, f32), view: (f32, f32), img: (f32, f32)) {
        let s0 = self.scale(view, img);
        let s1 = (s0 * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let r = s1 / s0;
        let d = (at.0 - view.0 / 2.0, at.1 - view.1 / 2.0);
        self.pan = (d.0 - (d.0 - self.pan.0) * r, d.1 - (d.1 - self.pan.1) * r);
        self.zoom = s1;
        self.clamp_pan(view, img);
    }

    /// Where the already-oriented picture lands in the window: (x, y, w, h). For callers that rotate and flip the pixels
    /// themselves (see `render::orient`) and only need an axis-aligned rectangle to draw.
    pub fn dest(&self, view: (f32, f32), img: (f32, f32)) -> (f32, f32, f32, f32) {
        let s = self.scale(view, img);
        let (ow, oh) = self.oriented(img);
        let (w, h) = (ow * s, oh * s);
        (view.0 / 2.0 + self.pan.0 - w / 2.0, view.1 / 2.0 + self.pan.1 - h / 2.0, w, h)
    }

    /// Linear part of "window pixel -> picture pixel": (m00, m01, m10, m11), already divided by the scale.
    fn linear(&self, view: (f32, f32), img: (f32, f32)) -> [f32; 4] {
        let k = 1.0 / self.scale(view, img);
        let (fx, fy) = (if self.flip_x { -1.0 } else { 1.0 }, if self.flip_y { -1.0 } else { 1.0 });
        // v = F * (d - c) / s, then undo the rotation: q=1 (x, y) -> (y, -x), q=2 -> (-x, -y), q=3 -> (-y, x)
        let f = [fx * k, 0.0, 0.0, fy * k];
        match self.quarter % 4 {
            0 => f,
            1 => [f[2], f[3], -f[0], -f[1]],
            2 => [-f[0], -f[1], -f[2], -f[3]],
            _ => [-f[2], -f[3], f[0], f[1]],
        }
    }

    /// Affine map window pixel (x, y) -> picture pixel: sx = a*x + b*y + e, sy = c*x + d*y + f.
    pub fn matrix(&self, view: (f32, f32), img: (f32, f32)) -> [f32; 6] {
        let [a, b, c, d] = self.linear(view, img);
        let centre = (view.0 / 2.0 + self.pan.0, view.1 / 2.0 + self.pan.1);
        [a, b, img.0 / 2.0 - (a * centre.0 + b * centre.1), c, d, img.1 / 2.0 - (c * centre.0 + d * centre.1)]
    }

    pub fn to_source(&self, view: (f32, f32), img: (f32, f32), p: (f32, f32)) -> (f32, f32) {
        let m = self.matrix(view, img);
        (m[0] * p.0 + m[1] * p.1 + m[2], m[3] * p.0 + m[4] * p.1 + m[5])
    }

    pub fn label(&self, view: (f32, f32), img: (f32, f32)) -> String {
        let mut s = format!("{:.0}%", self.scale(view, img) * 100.0);
        if self.zoom == 0.0 {
            s.push_str(" fit");
        }
        if self.quarter != 0 {
            s.push_str(&format!(" rot {}", self.quarter as u32 * 90));
        }
        if self.flip_x {
            s.push_str(" flip-h");
        }
        if self.flip_y {
            s.push_str(" flip-v");
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: (f32, f32) = (400.0, 300.0);
    const I: (f32, f32) = (200.0, 100.0);

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01
    }

    #[test]
    fn fit_never_grows_and_zoom_100_is_one_to_one() {
        let x = Xform::default();
        assert_eq!(x.scale(V, I), 1.0);
        assert!((x.scale(V, (800.0, 600.0)) - 0.5).abs() < 1e-6);
        let mut x = Xform::default();
        x.actual();
        // window centre shows the picture centre
        assert!(close(x.to_source(V, I, (200.0, 150.0)), (100.0, 50.0)));
    }

    #[test]
    fn four_turns_are_identity_and_one_turn_moves_top_left_to_top_right() {
        let mut x = Xform::default();
        x.actual();
        let before = x.to_source(V, I, (37.0, 61.0));
        for _ in 0..4 {
            x.rotate(true);
        }
        assert!(close(x.to_source(V, I, (37.0, 61.0)), before));
        x.rotate(true);
        // after a clockwise quarter turn the picture is 100 wide and 200 tall: its top-right corner (window pixels
        // centre + (50, -100)) is the source's top-left corner (0, 0)
        assert!(close(x.to_source(V, I, (250.0, 50.0)), (0.0, 0.0)), "{:?}", x.to_source(V, I, (250.0, 50.0)));
        x.rotate(false);
        assert_eq!(x.quarter, 0);
    }

    #[test]
    fn flip_mirrors_about_the_centre() {
        let mut x = Xform::default();
        x.actual();
        x.flip_x = true;
        assert!(close(x.to_source(V, I, (200.0 - 30.0, 150.0)), (100.0 + 30.0, 50.0)));
    }

    #[test]
    fn zoom_keeps_the_point_under_the_cursor() {
        let mut x = Xform::default();
        x.actual();
        let at = (260.0, 120.0);
        let before = x.to_source(V, I, at);
        x.zoom_by(2.0, at, V, I);
        assert_eq!(x.zoom, 2.0);
        assert!(close(x.to_source(V, I, at), before));
        for _ in 0..40 {
            x.zoom_by(2.0, at, V, I);
        }
        assert_eq!(x.zoom, MAX_ZOOM);
    }

    #[test]
    fn top_left_shows_the_top_of_a_tall_page() {
        let mut x = Xform::default();
        let img = (400.0, 3000.0);
        x.top_left(V, img);
        assert_eq!(x.zoom, 1.0);
        assert!(x.to_source(V, img, (200.0, 0.0)).1.abs() < 0.01);
    }

    #[test]
    fn dest_is_centred_and_scaled_by_the_oriented_size() {
        let mut x = Xform::default();
        x.zoom = 2.0;
        assert_eq!(x.dest(V, I), (0.0, 50.0, 400.0, 200.0));
        x.rotate(true);
        assert_eq!(x.dest(V, I), (100.0, -50.0, 200.0, 400.0));
        x.pan = (10.0, 20.0);
        assert_eq!(x.dest(V, I).0, 110.0);
    }

    #[test]
    fn pan_cannot_lose_the_picture() {
        let mut x = Xform::default();
        x.actual();
        x.pan_by(1.0e6, -1.0e6, V, I);
        assert!(x.pan.0 <= 400.0 && x.pan.1 >= -400.0);
        // still some of it in the window
        let s = x.to_source(V, I, (V.0 - 1.0, 1.0));
        assert!(s.0.is_finite() && s.1.is_finite());
    }
}
