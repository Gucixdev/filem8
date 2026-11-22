//! filem8_view [FILE]  - look at a file; nothing is ever written.
//!   + - wheel zoom (scroll on text, Ctrl+wheel zoom)   0 fit   1 100%   arrows / drag pan   r R rotate   h v flip   l loop   space play/pause   , . step frame
//!   n p PageDown PageUp  next / previous file in the folder   Home End first / last   [ ]  orbit a 3D model
//!   i status bar   f fullscreen   q Esc quit
//! filem8_view --shot OUT.png FILE [WxH] [ROT 0-3] [ZOOM 0=fit] [FRAME]   headless render (tests, screenshots)

use std::{
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

use filem8_view::{anim::Anim, doc::{self, Doc}, nav::{self, Nav}, render, xform::Xform};
use fm8_ui::{soft::Framebuffer, Theme};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, ModifiersState, NamedKey},
    window::{Fullscreen, Window, WindowId},
};

struct Shell {
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    theme: Theme,
    nav: Nav,
    doc: Option<Doc>,
    loading: Option<mpsc::Receiver<Doc>>,
    xf: Xform,
    anim: Anim,
    mods: ModifiersState,
    cursor: (f32, f32),
    drag: Option<(f32, f32)>,
    last_click: Option<Instant>,
    last_tick: Instant,
    bar: bool,
    dirty: bool,
}

impl Shell {
    fn open(&mut self) {
        let path = self.nav.files[self.nav.idx].clone();
        let theme = self.theme.clone();
        let (tx, rx) = mpsc::channel();
        // decoders can take a while on big files: never on the UI thread
        std::thread::spawn(move || {
            let d = std::panic::catch_unwind(|| doc::load(&path, &theme)).ok();
            if let Some(d) = d {
                let _ = tx.send(d);
            }
        });
        self.loading = Some(rx);
        self.doc = None;
        self.xf.reset();
        self.dirty = true;
        self.set_title();
    }

    fn set_title(&self) {
        if let Some(w) = &self.window {
            let name = self.nav.files[self.nav.idx].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            w.set_title(&format!("{name} - filem8_view"));
        }
    }

    fn view_size(&self) -> (f32, f32) {
        let Some(w) = &self.window else { return (1.0, 1.0) };
        let s = w.inner_size();
        let bar = if self.bar { (22.0 * w.scale_factor() as f32).ceil() } else { 0.0 };
        (s.width as f32, (s.height as f32 - bar).max(1.0))
    }

    fn img_size(&self) -> (f32, f32) {
        self.doc.as_ref().and_then(|d| d.pages.get(self.anim.idx.min(d.pages.len().saturating_sub(1)))).map_or((1.0, 1.0), |p| (p.w as f32, p.h as f32))
    }

    fn poll(&mut self) {
        let Some(rx) = &self.loading else { return };
        if let Ok(d) = rx.try_recv() {
            self.anim = Anim::new(d.pages.iter().map(|p| p.delay_ms).collect());
            self.xf.reset();
            if d.text {
                let (view, img) = (self.view_size(), (d.pages[0].w as f32, d.pages[0].h as f32));
                self.xf.top_left(view, img);
            }
            self.doc = Some(d);
            self.loading = None;
            self.last_tick = Instant::now();
            self.dirty = true;
        }
    }

    fn goto(&mut self, forward: bool) {
        self.nav.step(forward);
        self.open();
    }

    fn key(&mut self, el: &ActiveEventLoop, k: &Key) {
        let (view, img) = (self.view_size(), self.img_size());
        let centre = (view.0 / 2.0, view.1 / 2.0);
        let step = if self.mods.shift_key() { 240.0 } else { 60.0 };
        match k {
            Key::Named(NamedKey::Escape) => el.exit(),
            Key::Named(NamedKey::ArrowLeft) => self.xf.pan_by(step, 0.0, view, img),
            Key::Named(NamedKey::ArrowRight) => self.xf.pan_by(-step, 0.0, view, img),
            Key::Named(NamedKey::ArrowUp) => self.xf.pan_by(0.0, step, view, img),
            Key::Named(NamedKey::ArrowDown) => self.xf.pan_by(0.0, -step, view, img),
            Key::Named(NamedKey::PageDown) => return self.goto(true),
            Key::Named(NamedKey::PageUp) => return self.goto(false),
            Key::Named(NamedKey::Home) => {
                self.nav.go(true);
                return self.open();
            }
            Key::Named(NamedKey::End) => {
                self.nav.go(false);
                return self.open();
            }
            Key::Named(NamedKey::Space) => self.anim.toggle_play(),
            Key::Character(c) => match c.as_str() {
                "q" => el.exit(),
                "+" | "=" => self.xf.zoom_by(1.25, centre, view, img),
                "-" => self.xf.zoom_by(0.8, centre, view, img),
                "0" => match self.doc.as_ref().filter(|d| d.text) {
                    Some(d) => {
                        let img = (d.pages[0].w as f32, d.pages[0].h as f32);
                        self.xf.top_left(view, img)
                    }
                    None => self.xf.reset_zoom(),
                },
                "1" => self.xf.actual(),
                "r" => self.xf.rotate(true),
                "R" => self.xf.rotate(false),
                "h" => self.xf.flip_x = !self.xf.flip_x,
                "v" => self.xf.flip_y = !self.xf.flip_y,
                "l" => self.anim.toggle_loop(),
                "," => self.anim.step(false),
                "." => self.anim.step(true),
                "n" => return self.goto(true),
                "p" => return self.goto(false),
                "i" => self.bar = !self.bar,
                "f" => {
                    if let Some(w) = &self.window {
                        w.set_fullscreen(if w.fullscreen().is_some() { None } else { Some(Fullscreen::Borderless(None)) });
                    }
                }
                "[" | "]" => {
                    if let Some(d) = self.doc.as_mut() {
                        d.orbit(if c == "[" { -0.3 } else { 0.3 });
                    }
                }
                _ => return,
            },
            _ => return,
        }
        let (view, img) = (self.view_size(), self.img_size());
        self.xf.clamp_pan(view, img);
        self.dirty = true;
    }

    fn redraw(&mut self) {
        let (Some(window), Some(surface)) = (self.window.clone(), self.surface.as_mut()) else { return };
        let size = window.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));
        let scale = window.scale_factor() as f32;
        let mut fb = Framebuffer::new(w, h, self.theme.bg);
        let bar_h = if self.bar { (22.0 * scale).ceil() as u32 } else { 0 };
        let vp = render::Viewport { x: 0, y: 0, w, h: h.saturating_sub(bar_h).max(1) };
        let (left, right);
        match &self.doc {
            Some(d) => {
                let i = self.anim.idx.min(d.pages.len() - 1);
                render::draw_page(&mut fb, vp, &d.pages[i], &self.xf, self.theme.bg);
                let name = self.nav.files[self.nav.idx].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                left = format!("{name}   {}", d.meta);
                let frame = if self.anim.animated() { format!("   frame {}/{}{}{}", i + 1, d.pages.len(), if self.anim.looping { " loop" } else { "" }, if self.anim.playing { "" } else { " paused" }) } else { String::new() };
                right = format!("{}{frame}   {}", self.xf.label((vp.w as f32, vp.h as f32), (d.pages[i].w as f32, d.pages[i].h as f32)), self.nav.label());
            }
            None => {
                left = "loading...".to_string();
                right = self.nav.label();
            }
        }
        if self.bar {
            render::status_bar(&mut fb, &self.theme, scale, &left, &right);
        }
        if surface.resize(std::num::NonZeroU32::new(w).unwrap(), std::num::NonZeroU32::new(h).unwrap()).is_ok() {
            if let Ok(mut buf) = surface.buffer_mut() {
                for (d, p) in buf.iter_mut().zip(fb.px.chunks_exact(4)) {
                    *d = (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32;
                }
                let _ = buf.present();
            }
        }
        self.dirty = false;
    }
}

impl ApplicationHandler for Shell {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = Arc::new(el.create_window(Window::default_attributes().with_title("filem8_view").with_inner_size(LogicalSize::new(1000.0, 700.0))).expect("window"));
        let ctx = softbuffer::Context::new(window.clone()).expect("softbuffer context");
        self.surface = Some(softbuffer::Surface::new(&ctx, window.clone()).expect("softbuffer surface"));
        self.window = Some(window);
        self.open();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => self.dirty = true,
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => self.key(el, &event.logical_key),
            WindowEvent::CursorMoved { position, .. } => {
                let p = (position.x as f32, position.y as f32);
                if let Some(from) = self.drag {
                    let (view, img) = (self.view_size(), self.img_size());
                    self.xf.pan_by(p.0 - from.0, p.1 - from.1, view, img);
                    self.drag = Some(p);
                    self.dirty = true;
                }
                self.cursor = p;
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                if state == ElementState::Pressed {
                    self.drag = Some(self.cursor);
                    // double click: fit <-> 100%
                    let now = Instant::now();
                    if self.last_click.is_some_and(|t| now.duration_since(t) < Duration::from_millis(350)) {
                        if self.xf.zoom == 0.0 { self.xf.actual() } else { self.xf.reset_zoom() }
                        self.dirty = true;
                    }
                    self.last_click = Some(now);
                } else {
                    self.drag = None;
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let y = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                let (view, img) = (self.view_size(), self.img_size());
                if self.doc.as_ref().is_some_and(|d| d.text) && !self.mods.control_key() {
                    self.xf.pan_by(0.0, y * 80.0, view, img);
                } else {
                    self.xf.zoom_by(1.1f32.powf(y), self.cursor, view, img);
                }
                self.dirty = true;
            }
            _ => {}
        }
        if self.dirty {
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        self.poll();
        let now = Instant::now();
        let ms = now.duration_since(self.last_tick).as_millis().min(u32::MAX as u128) as u32;
        self.last_tick = now;
        if self.anim.advance(ms) {
            self.dirty = true;
        }
        if self.dirty {
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
        // wake for the next frame; while loading, look again soon; otherwise sleep until an event
        let wake = if self.loading.is_some() { Some(30) } else { self.anim.next_in() };
        el.set_control_flow(match wake {
            Some(ms) => ControlFlow::WaitUntil(now + Duration::from_millis(ms as u64)),
            None => ControlFlow::Wait,
        });
    }
}

fn parse_size(s: &str) -> (u32, u32) {
    s.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))).unwrap_or((1000, 700))
}

/// `--shot OUT FILE [WxH] [ROT] [ZOOM] [FRAME]`
fn shot(args: &[String]) -> Result<(), String> {
    let (Some(out), Some(file)) = (args.first(), args.get(1)) else { return Err("usage: --shot OUT.png FILE [WxH] [ROT 0-3] [ZOOM 0=fit] [FRAME]".into()) };
    let (w, h) = args.get(2).map(|s| parse_size(s)).unwrap_or((1000, 700));
    let theme = Theme::dark();
    let d = doc::load(Path::new(file), &theme);
    let mut xf = Xform::default();
    xf.quarter = args.get(3).and_then(|s| s.parse::<u8>().ok()).unwrap_or(0) % 4;
    xf.zoom = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    if d.text && args.get(4).is_none() {
        xf.top_left((w as f32, h as f32 - 22.0), (d.pages[0].w as f32, d.pages[0].h as f32));
    }
    let i = args.get(5).and_then(|s| s.parse::<usize>().ok()).unwrap_or(0).min(d.pages.len() - 1);
    let mut fb = Framebuffer::new(w, h, theme.bg);
    let bar = render::status_bar(&mut Framebuffer::new(w, h, theme.bg), &theme, 1.0, "", "");
    let vp = render::Viewport { x: 0, y: 0, w, h: h - bar };
    render::draw_page(&mut fb, vp, &d.pages[i], &xf, theme.bg);
    let name = Path::new(file).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let right = format!("{}   frame {}/{}", xf.label((vp.w as f32, vp.h as f32), (d.pages[i].w as f32, d.pages[i].h as f32)), i + 1, d.pages.len());
    render::status_bar(&mut fb, &theme, 1.0, &format!("{name}   {}", d.meta), &right);
    fb.save_png(Path::new(out))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--shot") => {
            if let Err(e) = shot(&args[1..]) {
                eprintln!("filem8_view: {e}");
                std::process::exit(1);
            }
            return;
        }
        Some("-h" | "--help") => {
            println!("filem8_view [FILE]\n  + - wheel zoom (scroll on text, Ctrl+wheel zoom), 0 fit, 1 100%, arrows/drag pan, r R rotate, h v flip, l loop, space play/pause, , . step\n  n p PageDown PageUp next/previous file, Home End first/last, [ ] orbit a 3D model, i bar, f fullscreen, q quit\n  --shot OUT.png FILE [WxH] [ROT 0-3] [ZOOM 0=fit] [FRAME]   headless render");
            return;
        }
        _ => {}
    }
    let first: PathBuf = args.first().map(|a| fm8_core::paths::expand(a)).unwrap_or_else(|| {
        eprintln!("filem8_view: give a file (filem8_view --help)");
        std::process::exit(2);
    });
    let nav = nav::siblings(&first);
    let mut shell = Shell {
        window: None,
        surface: None,
        theme: Theme::dark(),
        nav,
        doc: None,
        loading: None,
        xf: Xform::default(),
        anim: Anim::new(vec![100]),
        mods: ModifiersState::empty(),
        cursor: (0.0, 0.0),
        drag: None,
        last_click: None,
        last_tick: Instant::now(),
        bar: true,
        dirty: true,
    };
    let el = EventLoop::new().expect("event loop");
    el.run_app(&mut shell).expect("run");
}
