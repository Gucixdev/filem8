//! The viewer, built into the window, in two shapes of the same thing:
//!   split  (F3 until "list + preview"): the right half follows the cursor and is a real viewer, with a button row
//!   full   (Space on a file): the whole window, walks the list with n / p, Esc goes back
//! Any format, never edits. The maths and loaders are the `filem8_view` library (also the standalone `filem8_view`);
//! this file is only the toolkit side. Rotation and flips are baked into the pixels (`render::orient`) because the
//! toolkit draws axis-aligned images.
use super::*;
use filem8_view::{anim::Anim, doc::{self, Doc}, nav::{self, Nav}, render, xform::Xform};
use fm8_ui::geom::{rect, rgb, Color};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

pub(crate) struct Viewer {
    path: PathBuf,
    /// full window: the files to walk; split: None, the cursor decides
    nav: Option<Nav>,
    /// split: the file the cursor moved to and since when, so fast scrolling does not decode every file it passes
    want: Option<(PathBuf, f64)>,
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

pub(crate) enum Out { Stay, Maximize }

enum Go { Next, Prev, First, Last }

impl App {
    /// Space: the file under the cursor (or the first selected), full window, walking the list in its own order.
    pub(crate) fn open_viewer(&mut self) {
        if let Some(p) = self.cursor_path() { self.open_viewer_at(p); }
    }

    pub(crate) fn open_viewer_at(&mut self, p: PathBuf) {
        if p.is_dir() { return; }
        let t = &self.tabs[self.active];
        let files: Vec<PathBuf> = t.model.view.iter().map(|&i| &t.model.entries[i]).filter(|e| !e.is_dir).map(|e| e.path.clone()).collect();
        self.viewer = Some(Viewer::new(p.clone(), Some(nav::from_files(files, &p))));
    }

    /// The right half of the "list + preview" layout. Audio keeps the old preview (it has the player); the trash too.
    pub(crate) fn split_view(&mut self, ui: &mut Ui, area: Rect) {
        let target = { let tb = &self.tabs[self.active]; tb.model.cursor.and_then(|c| tb.model.entry(c)).filter(|e| !e.is_dir).map(|e| (e.path.clone(), e.class)) };
        let old_pane = self.trash.is_some() || target.as_ref().is_some_and(|(_, c)| *c == Class::Audio);
        if old_pane {
            let t = if let Some(v) = &self.trash { v.rows.get(v.cursor).map(|r| r.it.trashed.clone()) } else { target.map(|(p, _)| p) };
            self.preview.set(t);
            self.preview.draw(ui, area);
            return;
        }
        // not used: stop its worker decoding the same file a second time
        self.preview.set(None);
        let v = self.split_viewer.get_or_insert_with(|| Viewer::new(PathBuf::new(), None));
        v.follow(ui, target.map(|(p, _)| p));
        let ui_rect = area;
        let hovered = ui_rect.contains(ui.mouse);
        match v.pane(ui, ui_rect, hovered, true) {
            Out::Maximize => self.open_viewer(),
            _ => {}
        }
    }

    /// True when plain keys are for the split viewer (the mouse is over it): the list must not type-search with them.
    /// What the split viewer shows: "text", "image", "anim" or "" (nothing loaded). For tests and scripts.
    pub fn split_kind(&self) -> &'static str {
        match self.split_viewer.as_ref().and_then(|v| v.doc.as_ref().map(|d| (d.text, v.anim.animated()))) {
            Some((true, _)) => "text",
            Some((false, true)) => "anim",
            Some((false, false)) => "image",
            None => "",
        }
    }

    pub(crate) fn split_hovered(&self, ui: &Ui, area: Rect) -> bool {
        self.layout == Layout::Preview && self.trash.is_none() && area.contains(ui.mouse) && self.split_viewer.as_ref().is_some_and(|v| !v.path.as_os_str().is_empty())
    }
}

impl Viewer {
    fn new(path: PathBuf, nav: Option<Nav>) -> Viewer {
        Viewer { path, nav, want: None, doc: None, raw: vec![], cache: vec![], cache_key: (0, false, false), alpha: false, rx: None, xf: Xform::default(), anim: Anim::new(vec![100]), view: (800.0, 600.0), drag: None, help: false }
    }

    fn clear(&mut self) {
        self.path = PathBuf::new();
        self.doc = None;
        self.raw.clear();
        self.cache.clear();
        self.rx = None;
    }

    fn load(&mut self, theme: &Theme) {
        let path = self.path.clone();
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
        self.cache.clear();
        self.xf.reset();
    }

    /// Split: show whatever the cursor is on, after it has rested for a moment.
    fn follow(&mut self, ui: &mut Ui, want: Option<PathBuf>) {
        let Some(w) = want else { if !self.path.as_os_str().is_empty() { self.clear(); } self.want = None; return };
        if w == self.path { self.want = None; return; }
        let since = match &self.want { Some((p, t)) if *p == w => *t, _ => { self.want = Some((w.clone(), ui.time)); ui.time } };
        if ui.time - since >= 0.1 || self.path.as_os_str().is_empty() {
            self.path = w;
            self.want = None;
            let t = ui.theme.clone();
            self.load(&t);
        } else {
            ui.request_redraw();
        }
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

    fn is_text(&self) -> bool { self.doc.as_ref().is_some_and(|d| d.text) }

    fn fit(&mut self) {
        if self.is_text() { let img = self.img_size(); self.xf.top_left(self.view, img) } else { self.xf.reset_zoom() }
    }

    fn orbit(&mut self, d: f32) {
        if let Some(mut doc) = self.doc.take() {
            if doc.orbit(d) { self.take_pages(&mut doc); }
            self.doc = Some(doc);
        }
    }

    /// Keys that mean the same in both shapes. `take` is false-safe: it only consumes what it uses.
    fn keys(&mut self, ui: &mut Ui) {
        let (view, img) = (self.view, self.img_size());
        let centre = (view.0 / 2.0, view.1 / 2.0);
        let step = if ui.mods.shift { 240.0 } else { 60.0 };
        let take = |ui: &mut Ui, c: char| ui.take_key(|k, m| k == Key::Char(c) && !m.ctrl && !m.alt);
        for (k, dx, dy) in [(Key::Left, step, 0.0), (Key::Right, -step, 0.0), (Key::Up, 0.0, step), (Key::Down, 0.0, -step)] {
            if ui.take_key(|kk, m| kk == k && !m.ctrl && !m.alt) { self.xf.pan_by(dx, dy, view, img); }
        }
        if take(ui, '+') || take(ui, '=') { self.xf.zoom_by(1.25, centre, view, img); }
        if take(ui, '-') { self.xf.zoom_by(0.8, centre, view, img); }
        if take(ui, '0') { self.fit(); }
        if take(ui, '1') { self.xf.actual(); }
        if take(ui, 'r') { self.xf.rotate(true); }
        if take(ui, 'R') { self.xf.rotate(false); }
        if take(ui, 'h') { self.xf.flip_x = !self.xf.flip_x; }
        if take(ui, 'v') { self.xf.flip_y = !self.xf.flip_y; }
        if take(ui, 'l') { self.anim.toggle_loop(); }
        if take(ui, ',') { self.anim.step(false); }
        if take(ui, '.') { self.anim.step(true); }
        if take(ui, '[') { self.orbit(-0.3); }
        if take(ui, ']') { self.orbit(0.3); }
    }

    /// Draw into `r` and handle the mouse over it; `keys` = plain keys are ours. Split shape adds a button row.
    pub(crate) fn pane(&mut self, ui: &mut Ui, r: Rect, keys: bool, split: bool) -> Out {
        let t = ui.theme.clone();
        let mut out = Out::Stay;
        let (row, area) = if split { r.cut_bottom(34.0) } else { (rect(0.0, 0.0, 0.0, 0.0), r) };
        self.view = (area.w, area.h);
        if let Some(rx) = &self.rx {
            if let Ok(d) = rx.try_recv() { self.rx = None; self.adopt(d); }
        }
        let loading = self.rx.is_some();
        if self.anim.advance((ui.dt * 1000.0) as u32) || self.anim.next_in().is_some() || loading { ui.request_redraw(); }

        // input
        if keys { self.keys(ui); }
        let (view, img) = (self.view, self.img_size());
        let over = area.contains(ui.mouse);
        let w = ui.frame.wheel;
        if w != 0.0 && over {
            if self.is_text() && !ui.mods.ctrl { self.xf.pan_by(0.0, -w * 80.0, view, img); }
            else { self.xf.zoom_by(1.1f32.powf(-w), (ui.mouse.x - area.x, ui.mouse.y - area.y), view, img); }
            ui.frame.wheel = 0.0;
        }
        if ui.frame.press[0] && over {
            self.drag = Some(ui.mouse);
            if ui.frame.double { if self.xf.zoom == 0.0 { self.xf.actual() } else { self.fit() } }
        }
        if let Some(from) = self.drag {
            if ui.mouse_down[0] {
                self.xf.pan_by(ui.mouse.x - from.x, ui.mouse.y - from.y, view, img);
                self.drag = Some(ui.mouse);
            } else { self.drag = None; }
        }

        // draw
        ui.draw().rect(area, t.bg);
        if !self.raw.is_empty() {
            let (view, img) = (self.view, self.img_size());
            self.xf.clamp_pan(view, img);
            let i = self.frame_idx();
            let (x, y, w, h) = self.xf.dest(view, img);
            let ir = rect(area.x + x, area.y + y, w, h);
            let smooth = self.xf.scale(view, img) < 1.5;
            let im = self.oriented(i);
            ui.push_clip(area);
            if self.alpha { checker(ui, ir.intersect(area)); }
            ui.draw().cmds.push(Cmd::Image { r: ir, img: im, smooth });
            ui.pop_clip();
        } else if loading {
            ui.label_ex(area, "Loading...", t.fg_dim, false, 0.5);
        } else if self.path.as_os_str().is_empty() {
            ui.label_ex(area, "Select a file to view it here", t.fg_dim, false, 0.5);
        }
        if split { if let Some(o) = self.buttons(ui, row) { out = o; } }
        out
    }

    fn buttons(&mut self, ui: &mut Ui, row: Rect) -> Option<Out> {
        let t = ui.theme.clone();
        ui.draw().rect(row, t.bg_alt);
        let (view, img) = (self.view, self.img_size());
        let centre = (view.0 / 2.0, view.1 / 2.0);
        // most useful first: a narrow pane cuts the row from the right
        let mut labels: Vec<(&str, u8)> = vec![("Full", 12), ("Fit", 6), ("1:1", 7), ("-", 4), ("+", 5), ("R-", 0), ("R+", 1), ("H", 2), ("V", 3)];
        if self.anim.animated() { labels.push((if self.anim.playing { "Pause" } else { "Play" }, 8)); labels.push((if self.anim.looping { "Loop" } else { "Once" }, 9)); }
        if self.doc.as_ref().is_some_and(|d| d.mesh.is_some()) { labels.push(("<", 10)); labels.push((">", 11)); }
        let mut x = row.x + 6.0;
        let mut out = None;
        for (label, code) in labels {
            let w = (fm8_ui::text::measure(label, t.font_size, false) + 14.0).max(28.0);
            if x + w > row.right() - 4.0 { break; }
            if ui.button(Id::new("vbtn").with(code as u64), rect(x, row.y + 4.0, w, row.h - 8.0), label, BtnStyle::Ghost) {
                match code {
                    0 => self.xf.rotate(false),
                    1 => self.xf.rotate(true),
                    2 => self.xf.flip_x = !self.xf.flip_x,
                    3 => self.xf.flip_y = !self.xf.flip_y,
                    4 => self.xf.zoom_by(0.8, centre, view, img),
                    5 => self.xf.zoom_by(1.25, centre, view, img),
                    6 => self.fit(),
                    7 => self.xf.actual(),
                    8 => self.anim.toggle_play(),
                    9 => self.anim.toggle_loop(),
                    10 => self.orbit(-0.3),
                    11 => self.orbit(0.3),
                    _ => out = Some(Out::Maximize),
                }
            }
            x += w + 2.0;
        }
        // what you are looking at, right-aligned in whatever room is left
        let mut info = String::new();
        if !self.raw.is_empty() {
            info = self.xf.label(view, img);
            if self.anim.animated() { info.push_str(&format!("  {}/{}", self.frame_idx() + 1, self.raw.len())); }
        }
        if x + 60.0 < row.right() { ui.label_ex(rect(x, row.y, row.right() - x - 8.0, row.h), &info, t.fg_dim, false, 1.0); }
        out
    }

    fn go(&mut self, how: Go, theme: &Theme) {
        let Some(n) = self.nav.as_mut() else { return };
        match how {
            Go::Next => { n.step(true); }
            Go::Prev => { n.step(false); }
            Go::First => { n.go(true); }
            Go::Last => { n.go(false); }
        }
        self.path = n.files[n.idx].clone();
        self.load(theme);
    }

    /// Full-window shape, one frame. false = close it.
    pub(crate) fn frame(&mut self, ui: &mut Ui) -> bool {
        let t = ui.theme.clone();
        let full = rect(0.0, 0.0, ui.size.x, ui.size.y);
        let (bar, main) = full.cut_bottom(26.0);
        if self.rx.is_none() && self.doc.is_none() { self.load(&t); }
        let take = |ui: &mut Ui, c: char| ui.take_key(|k, m| k == Key::Char(c) && !m.ctrl && !m.alt);
        if ui.key_pressed(Key::Escape) || take(ui, 'q') { return false; }
        if ui.key_pressed(Key::PageDown) || take(ui, 'n') { self.go(Go::Next, &t); return true; }
        if ui.key_pressed(Key::PageUp) || take(ui, 'p') { self.go(Go::Prev, &t); return true; }
        if ui.key_pressed(Key::Home) { self.go(Go::First, &t); return true; }
        if ui.key_pressed(Key::End) { self.go(Go::Last, &t); return true; }
        if ui.key_pressed(Key::Space) { self.anim.toggle_play(); }
        if take(ui, '?') || ui.key_pressed(Key::F(1)) { self.help = !self.help; }
        ui.draw().rect(full, t.bg);
        self.pane(ui, main, true, false);
        self.status(ui, bar);
        if self.help { self.help_box(ui, main); }
        true
    }

    fn status(&self, ui: &mut Ui, bar: Rect) {
        let t = ui.theme.clone();
        ui.draw().rect(bar, t.bg_alt);
        let name = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let meta = self.doc.as_ref().map(|d| d.meta.clone()).unwrap_or_default();
        let left = if self.rx.is_some() { format!("{name}   loading...") } else { format!("{name}   {meta}") };
        let (l, r) = (rect(bar.x + 10.0, bar.y, bar.w * 0.6 - 10.0, bar.h), rect(bar.x + bar.w * 0.6, bar.y, bar.w * 0.4 - 10.0, bar.h));
        ui.label_ex(l, &fm8_ui::text::ellipsize(&left, t.font_size, false, l.w), t.fg, false, 0.0);
        let mut right = String::new();
        if !self.raw.is_empty() {
            right.push_str(&self.xf.label(self.view, self.img_size()));
            if self.anim.animated() {
                right.push_str(&format!("   frame {}/{}{}{}", self.frame_idx() + 1, self.raw.len(), if self.anim.looping { " loop" } else { "" }, if self.anim.playing { "" } else { " paused" }));
            }
        }
        if let Some(n) = &self.nav { right.push_str(&format!("   {}", n.label())); }
        right.push_str("   ? help");
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
