//! The viewer, built into the window: Space / F3 on a file shows it full window, every format, never edits it.
//! The maths and the loaders are the `filem8_view` library (the same ones behind the standalone `filem8_view`);
//! this file is only the toolkit side: draw the page, read keys and the mouse.
//! Rotation and flips are baked into the pixels (`render::orient`) because the toolkit draws axis-aligned images.
use super::*;
use filem8_view::{anim::Anim, doc::{self, Doc}, nav::{self, Nav}, render, xform::Xform};
use fm8_ui::geom::{rect, rgb, Color};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

pub(crate) struct Viewer {
    nav: Nav,
    doc: Option<Doc>,
    /// pages as the loader made them; `cache` holds them rotated and flipped
    raw: Vec<Arc<Image>>,
    cache: Vec<Option<Arc<Image>>>,
    cache_key: (u8, bool, bool),
    alpha: bool,
    rx: Option<Receiver<Doc>>,
    xf: Xform,
    anim: Anim,
    view: (f32, f32),
    drag: Option<Vec2>,
    help: bool,
}

impl App {
    /// Space / F3: the file under the cursor (or the first selected), walking the folder in the list's own order.
    pub(crate) fn open_viewer(&mut self) {
        if let Some(p) = self.cursor_path() { self.open_viewer_at(p); }
    }

    pub(crate) fn open_viewer_at(&mut self, p: PathBuf) {
        if p.is_dir() { return; }
        let t = &self.tabs[self.active];
        let files: Vec<PathBuf> = t.model.view.iter().map(|&i| &t.model.entries[i]).filter(|e| !e.is_dir).map(|e| e.path.clone()).collect();
        self.viewer = Some(Viewer::new(nav::from_files(files, &p)));
    }
}

impl Viewer {
    fn new(nav: Nav) -> Viewer {
        Viewer { nav, doc: None, raw: vec![], cache: vec![], cache_key: (0, false, false), alpha: false, rx: None, xf: Xform::default(), anim: Anim::new(vec![100]), view: (800.0, 600.0), drag: None, help: false }
    }

    fn load(&mut self, theme: &Theme) {
        let path = self.nav.files[self.nav.idx].clone();
        let theme = theme.clone();
        let (tx, rx) = channel();
        // decoders can be slow on big files: never on the UI thread; a panic in one must not take the window down
        std::thread::spawn(move || {
            if let Ok(d) = std::panic::catch_unwind(|| doc::load(&path, &theme)) { let _ = tx.send(d); }
        });
        // replacing the receiver drops the previous one: a late result from the file we left is discarded
        self.rx = Some(rx);
        self.doc = None;
        self.raw.clear();
        self.xf.reset();
    }

    fn adopt(&mut self, mut d: Doc) {
        self.anim = Anim::new(d.pages.iter().map(|p| p.delay_ms).collect());
        self.alpha = d.pages.iter().any(|p| p.has_alpha());
        self.take_pages(&mut d);
        self.xf.reset();
        if d.text { let img = (self.raw[0].w as f32, self.raw[0].h as f32); self.xf.top_left(self.view, img); }
        self.doc = Some(d);
    }

    fn take_pages(&mut self, d: &mut Doc) {
        self.raw = std::mem::take(&mut d.pages).into_iter().map(|p| Image::new(p.w, p.h, p.rgba)).collect();
        self.cache = vec![None; self.raw.len()];
    }

    fn frame_idx(&self) -> usize { self.anim.idx.min(self.raw.len().saturating_sub(1)) }

    fn oriented(&mut self, i: usize) -> Arc<Image> {
        let key = (self.xf.quarter, self.xf.flip_x, self.xf.flip_y);
        if key != self.cache_key { self.cache_key = key; self.cache.iter_mut().for_each(|c| *c = None); }
        if let Some(c) = &self.cache[i] { return c.clone(); }
        let r = &self.raw[i];
        let img = if key == (0, false, false) { r.clone() } else {
            let (w, h, px) = render::orient(r.w, r.h, &r.rgba, key.0, key.1, key.2);
            Image::new(w, h, px)
        };
        self.cache[i] = Some(img.clone());
        img
    }

    fn img_size(&self) -> (f32, f32) {
        self.raw.get(self.frame_idx()).map_or((1.0, 1.0), |i| (i.w as f32, i.h as f32))
    }

    fn go(&mut self, how: Go, theme: &Theme) {
        match how {
            Go::Next => { self.nav.step(true); }
            Go::Prev => { self.nav.step(false); }
            Go::First => { self.nav.go(true); }
            Go::Last => { self.nav.go(false); }
        }
        self.load(theme);
    }

    /// One frame of the viewer. false = close it.
    pub(crate) fn frame(&mut self, ui: &mut Ui) -> bool {
        let t = ui.theme.clone();
        let full = rect(0.0, 0.0, ui.size.x, ui.size.y);
        let (bar, main) = full.cut_bottom(26.0);
        self.view = (main.w, main.h);
        if self.rx.is_none() && self.doc.is_none() { self.load(&t); }
        if let Some(rx) = &self.rx {
            if let Ok(d) = rx.try_recv() { self.rx = None; self.adopt(d); }
        }
        let loading = self.rx.is_some();
        if self.anim.advance((ui.dt * 1000.0) as u32) || self.anim.next_in().is_some() || loading { ui.request_redraw(); }

        // ---- input
        let (view, img) = (self.view, self.img_size());
        let centre = (view.0 / 2.0, view.1 / 2.0);
        let step = if ui.mods.shift { 240.0 } else { 60.0 };
        let take = |ui: &mut Ui, c: char| ui.take_key(|k, m| k == Key::Char(c) && !m.ctrl && !m.alt);
        if ui.key_pressed(Key::Escape) || take(ui, 'q') { return false; }
        let text = self.doc.as_ref().is_some_and(|d| d.text);
        for (k, dx, dy) in [(Key::Left, step, 0.0), (Key::Right, -step, 0.0), (Key::Up, 0.0, step), (Key::Down, 0.0, -step)] {
            if ui.take_key(|kk, m| kk == k && !m.ctrl && !m.alt) { self.xf.pan_by(dx, dy, view, img); }
        }
        if ui.key_pressed(Key::PageDown) || take(ui, 'n') { self.go(Go::Next, &t); return true; }
        if ui.key_pressed(Key::PageUp) || take(ui, 'p') { self.go(Go::Prev, &t); return true; }
        if ui.key_pressed(Key::Home) { self.go(Go::First, &t); return true; }
        if ui.key_pressed(Key::End) { self.go(Go::Last, &t); return true; }
        if ui.key_pressed(Key::Space) { self.anim.toggle_play(); }
        if take(ui, '+') || take(ui, '=') { self.xf.zoom_by(1.25, centre, view, img); }
        if take(ui, '-') { self.xf.zoom_by(0.8, centre, view, img); }
        if take(ui, '0') { if text { self.xf.top_left(view, img) } else { self.xf.reset_zoom() } }
        if take(ui, '1') { self.xf.actual(); }
        if take(ui, 'r') { self.xf.rotate(true); }
        if take(ui, 'R') { self.xf.rotate(false); }
        if take(ui, 'h') { self.xf.flip_x = !self.xf.flip_x; }
        if take(ui, 'v') { self.xf.flip_y = !self.xf.flip_y; }
        if take(ui, 'l') { self.anim.toggle_loop(); }
        if take(ui, ',') { self.anim.step(false); }
        if take(ui, '.') { self.anim.step(true); }
        if take(ui, '?') || ui.key_pressed(Key::F(1)) { self.help = !self.help; }
        let orbit = if take(ui, '[') { -0.3 } else if take(ui, ']') { 0.3 } else { 0.0 };
        if orbit != 0.0 {
            if let Some(mut d) = self.doc.take() {
                if d.orbit(orbit) { self.take_pages(&mut d); }
                self.doc = Some(d);
            }
        }
        let w = ui.frame.wheel;
        if w != 0.0 && main.contains(ui.mouse) {
            if text && !ui.mods.ctrl { self.xf.pan_by(0.0, -w * 80.0, view, img); }
            else { self.xf.zoom_by(1.1f32.powf(-w), (ui.mouse.x - main.x, ui.mouse.y - main.y), view, img); }
        }
        if ui.frame.press[0] && main.contains(ui.mouse) {
            self.drag = Some(ui.mouse);
            if ui.frame.double { if self.xf.zoom == 0.0 { self.xf.actual() } else { self.xf.reset_zoom() } }
        }
        if let Some(from) = self.drag {
            if ui.mouse_down[0] {
                self.xf.pan_by(ui.mouse.x - from.x, ui.mouse.y - from.y, view, img);
                self.drag = Some(ui.mouse);
            } else { self.drag = None; }
        }
        let (view, img) = (self.view, self.img_size());
        self.xf.clamp_pan(view, img);

        // ---- draw
        ui.draw().rect(full, t.bg);
        if !self.raw.is_empty() {
            let i = self.frame_idx();
            let (x, y, w, h) = self.xf.dest(view, img);
            let r = rect(main.x + x, main.y + y, w, h);
            let smooth = self.xf.scale(view, img) < 1.5;
            let im = self.oriented(i);
            ui.push_clip(main);
            if self.alpha { checker(ui, r.intersect(main)); }
            ui.draw().cmds.push(Cmd::Image { r, img: im, smooth });
            ui.pop_clip();
        } else if loading {
            ui.label_ex(main, "Loading...", t.fg_dim, false, 0.5);
        }
        self.status(ui, bar, loading);
        if self.help { self.help_box(ui, main); }
        true
    }

    fn status(&self, ui: &mut Ui, bar: Rect, loading: bool) {
        let t = ui.theme.clone();
        ui.draw().rect(bar, t.bg_alt);
        let name = self.nav.files[self.nav.idx].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let meta = self.doc.as_ref().map(|d| d.meta.clone()).unwrap_or_default();
        let left = if loading { format!("{name}   loading...") } else { format!("{name}   {meta}") };
        let (l, r) = (rect(bar.x + 10.0, bar.y, bar.w * 0.6 - 10.0, bar.h), rect(bar.x + bar.w * 0.6, bar.y, bar.w * 0.4 - 10.0, bar.h));
        ui.label_ex(l, &fm8_ui::text::ellipsize(&left, t.font_size, false, l.w), t.fg, false, 0.0);
        let mut right = String::new();
        if !self.raw.is_empty() {
            let img = self.img_size();
            right.push_str(&self.xf.label(self.view, img));
            if self.anim.animated() {
                right.push_str(&format!("   frame {}/{}{}{}", self.frame_idx() + 1, self.raw.len(), if self.anim.looping { " loop" } else { "" }, if self.anim.playing { "" } else { " paused" }));
            }
        }
        right.push_str(&format!("   {}   ? help", self.nav.label()));
        ui.label_ex(r, &right, t.fg_dim, false, 1.0);
    }

    fn help_box(&self, ui: &mut Ui, main: Rect) {
        let t = ui.theme.clone();
        const KEYS: [&str; 11] = [
            "Esc q          back to the files",
            "n p PgDn PgUp  next / previous file",
            "+ -  wheel     zoom (text: wheel scrolls, Ctrl+wheel zooms)",
            "0  1           fit  /  100%   (double click toggles)",
            "arrows  drag   pan (Shift = bigger steps)",
            "r R            rotate clockwise / counter-clockwise",
            "h v            flip horizontally / vertically",
            "Space  l       play / pause,  loop on / off",
            ", .            previous / next frame",
            "[ ]            orbit a 3D model",
            "Home End       first / last file",
        ];
        let (w, h) = (470.0, 40.0 + KEYS.len() as f32 * 20.0);
        let r = rect(main.x + (main.w - w) / 2.0, main.y + (main.h - h) / 2.0, w, h);
        ui.draw().rounded(r, Color { a: 235, ..t.bg_alt }, 8.0);
        for (i, k) in KEYS.iter().enumerate() { ui.draw().text(r.x + 18.0, r.y + 20.0 + i as f32 * 20.0, k, 13.0, t.fg, false); }
    }
}

enum Go { Next, Prev, First, Last }

/// Two-tone squares behind pictures with transparency (only what is visible).
fn checker(ui: &mut Ui, r: Rect) {
    if r.is_empty() { return; }
    ui.draw().rect(r, rgb(58, 58, 62));
    let dark = rgb(84, 84, 88);
    let (cols, rows) = ((r.w / 16.0).ceil() as i32, (r.h / 16.0).ceil() as i32);
    for cy in 0..rows {
        for cx in 0..cols {
            if (cx + cy) % 2 == 1 { ui.draw().rect(rect(r.x + cx as f32 * 16.0, r.y + cy as f32 * 16.0, 16.0, 16.0).intersect(r), dark); }
        }
    }
}
