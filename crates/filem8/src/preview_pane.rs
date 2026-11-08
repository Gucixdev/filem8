//! Preview panel: part of the window layout (no extra window). Loading runs on
//! a worker thread; stale results are dropped via a generation counter.
use fm8_core::listing::human_size;
use fm8_preview::text::Tok;
use fm8_preview::{Opts, Preview};
use fm8_ui::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

pub enum Content {
    Empty,
    Loading,
    Anim { frames: Vec<(Arc<Image>, u32)>, total: u32, meta: String },
    Text { lines: Vec<Vec<(Tok, String)>>, meta: String },
    Lines { lines: Vec<String>, meta: String },
    Audio { peaks: Vec<f32>, meta: String, duration: f64 },
    Model { img: Arc<Image>, meta: String },
    Error(String),
}

pub struct PreviewPane {
    pub path: Option<PathBuf>,
    pub content: Content,
    gen: Arc<AtomicU64>,
    tx: Sender<(u64, Content)>,
    rx: Receiver<(u64, Content)>,
    #[cfg(feature = "full")]
    player: Option<(PathBuf, fm8_preview::audio::Player)>,
}

impl Default for PreviewPane {
    fn default() -> Self { PreviewPane::new() }
}

fn convert(p: Preview) -> Content {
    match p {
        Preview::Image(d) => {
            let total: u32 = d.frames.iter().map(|f| f.delay_ms.max(20)).sum();
            let meta = format!("{} - {} x {}{}", d.format, d.orig_w, d.orig_h, if d.frames.len() > 1 { format!(" - {} frames", d.frames.len()) } else { String::new() });
            let frames = d.frames.into_iter().map(|f| (Image::new(f.w, f.h, f.rgba), f.delay_ms.max(20))).collect();
            Content::Anim { frames, total, meta }
        }
        Preview::Text(t) => {
            let meta = format!("{} - {}{}", t.lang, human_size(t.total_bytes), if t.truncated { " (start only)" } else { "" });
            Content::Text { lines: t.lines, meta }
        }
        Preview::Archive { kind, entries, truncated } => {
            let mut lines: Vec<String> = entries.iter().map(|e| if e.is_dir { format!("{}/", e.name) } else { format!("{}   ({})", e.name, human_size(e.size)) }).collect();
            if truncated { lines.push("...".into()); }
            Content::Lines { meta: format!("{kind} - {} entries", entries.len()), lines }
        }
        Preview::Audio(a) => Content::Audio { meta: format!("{} - {} Hz - {} ch - {}:{:02}", a.codec, a.sample_rate, a.channels, a.duration_s as u64 / 60, a.duration_s as u64 % 60), peaks: a.peaks, duration: a.duration_s },
        Preview::Model(m) => {
            let rgba = fm8_preview::model::render(&m, 0.6, 512);
            Content::Model { meta: format!("{} - {} triangles", m.source, m.triangles()), img: Image::new(512, 512, rgba) }
        }
        Preview::Info(l) => Content::Lines { lines: l, meta: String::new() },
        Preview::Error(e) => Content::Error(e),
    }
}

impl PreviewPane {
    pub fn new() -> PreviewPane {
        let (tx, rx) = channel();
        PreviewPane { path: None, content: Content::Empty, gen: Arc::new(AtomicU64::new(0)), tx, rx, #[cfg(feature = "full")] player: None }
    }

    /// Ask for `p` to be previewed (no-op when already shown).
    pub fn set(&mut self, p: Option<PathBuf>) {
        if self.path == p { return; }
        self.path = p.clone();
        let g = self.gen.fetch_add(1, Ordering::SeqCst) + 1;
        #[cfg(feature = "full")]
        { self.player = None; }
        let Some(p) = p else { self.content = Content::Empty; return };
        self.content = Content::Loading;
        let (tx, gen) = (self.tx.clone(), self.gen.clone());
        std::thread::spawn(move || {
            // debounce quick cursor movement
            std::thread::sleep(std::time::Duration::from_millis(60));
            if gen.load(Ordering::SeqCst) != g { return; }
            let o = Opts { max_dim: 640, max_frames: 120, ..Default::default() };
            let c = convert(fm8_preview::load(&p, &o));
            let _ = tx.send((g, c));
        });
    }

    pub fn poll(&mut self) -> bool {
        let mut ch = false;
        while let Ok((g, c)) = self.rx.try_recv() {
            if g == self.gen.load(Ordering::SeqCst) { self.content = c; ch = true; }
        }
        ch
    }

    pub fn draw(&mut self, ui: &mut Ui, r: Rect) {
        let t = ui.theme.clone();
        ui.draw().rect(r, t.bg_alt);
        let Some(path) = self.path.clone() else {
            ui.label_ex(r, "Select a file to preview it here", t.fg_dim, false, 0.5);
            return;
        };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let (head, area) = r.inset(14.0).cut_top(46.0);
        ui.label_ex(rect(head.x, head.y, head.w, 22.0), &fm8_ui::text::ellipsize(&name, t.font_size, true, head.w), t.fg, true, 0.0);
        let meta = match &self.content {
            Content::Anim { meta, .. } | Content::Text { meta, .. } | Content::Lines { meta, .. } | Content::Audio { meta, .. } | Content::Model { meta, .. } => meta.clone(),
            Content::Loading => "Loading...".into(),
            _ => String::new(),
        };
        ui.draw().text(head.x, head.y + 24.0, &fm8_ui::text::ellipsize(&meta, 12.0, false, head.w), 12.0, t.fg_dim, false);
        let id = Id::new("pv").with(self.gen.load(Ordering::SeqCst));
        match &self.content {
            Content::Empty | Content::Loading => {}
            Content::Error(e) => ui.label_ex(area, e, t.urgent, false, 0.0),
            Content::Anim { frames, total, .. } => {
                let mut idx = 0;
                if frames.len() > 1 {
                    let mut at = ((ui.time * 1000.0) as u32) % (*total).max(1);
                    for (i, (_, d)) in frames.iter().enumerate() { if at < *d { idx = i; break; } at -= d; }
                    ui.request_redraw();
                }
                let img = &frames[idx].0;
                ui.draw().cmds.push(Cmd::Image { r: fit(area, img.w, img.h), img: img.clone(), smooth: true });
            }
            Content::Model { img, .. } => ui.draw().cmds.push(Cmd::Image { r: fit(area, img.w, img.h), img: img.clone(), smooth: true }),
            Content::Text { lines, .. } => {
                let resp = ui.virt(id, area, lines.len(), None, 18.0);
                ui.push_clip(area);
                for i in resp.first..resp.last {
                    let cr = resp.cell_rect(i);
                    let mut x = cr.x;
                    for (tok, s) in &lines[i] {
                        let s = s.replace('\t', "    ");
                        let col = match tok { Tok::Plain => t.fg, Tok::Keyword => t.accent, Tok::Str => t.ok, Tok::Comment => t.fg_dim, Tok::Number => t.warn, Tok::Heading => t.link, Tok::Punct => t.fg_dim };
                        ui.draw().text(x, cr.y + 1.0, &s, 13.0, col, matches!(tok, Tok::Heading));
                        x += fm8_ui::text::measure(&s, 13.0, false);
                        if x > cr.right() { break; }
                    }
                }
                ui.pop_clip();
            }
            Content::Lines { lines, .. } => {
                let resp = ui.virt(id, area, lines.len(), None, 20.0);
                ui.push_clip(area);
                for i in resp.first..resp.last {
                    let cr = resp.cell_rect(i);
                    ui.draw().text(cr.x, cr.y + 2.0, &fm8_ui::text::ellipsize(&lines[i], 13.0, false, cr.w - 12.0), 13.0, t.fg, false);
                }
                ui.pop_clip();
            }
            Content::Audio { peaks, duration, .. } => {
                let (wave, ctl) = area.cut_top((area.h - 44.0).max(60.0));
                let mid = wave.y + wave.h / 2.0;
                let bw = wave.w / peaks.len().max(1) as f32;
                let pos_frac = 0.0f64;
                #[cfg(feature = "full")]
                if let Some((_, pl)) = &self.player { pos_frac = pl.position_s() / duration.max(0.001); }
                for (i, p) in peaks.iter().enumerate() {
                    let h = (p * wave.h * 0.9).max(2.0);
                    let played = (i as f64 / peaks.len() as f64) < pos_frac;
                    ui.draw().rounded(rect(wave.x + i as f32 * bw, mid - h / 2.0, (bw - 1.0).max(1.0), h), if played { t.accent } else { t.fg_dim.with_alpha(160) }, 1.0);
                }
                #[cfg(feature = "full")]
                {
                    let playing = self.player.as_ref().map(|(_, p)| p.is_playing()).unwrap_or(false);
                    if ui.icon_button(Id::new("pv-play"), rect(ctl.x, ctl.y + 6.0, 32.0, 32.0), if playing { Icon::Pause } else { Icon::Play }, "Play / pause", true, playing) {
                        if self.player.is_none() { self.player = fm8_preview::audio::Player::open(&path).ok().map(|p| (path.clone(), p)); }
                        if let Some((_, p)) = &self.player { if p.is_playing() { p.pause() } else { p.play() } }
                    }
                    if ui.hovered(wave) && ui.frame.press[0] {
                        if let Some((_, p)) = &self.player { p.seek_frac(((ui.mouse.x - wave.x) / wave.w) as f64); }
                    }
                    if playing { ui.request_redraw(); }
                }
                #[cfg(not(feature = "full"))]
                ui.draw().text(ctl.x, ctl.y + 12.0, "Playback needs the `full` build", 12.0, t.fg_dim, false);
                let _ = duration;
            }
        }
    }
}

fn fit(area: Rect, w: u32, h: u32) -> Rect {
    let s = (area.w / w as f32).min(area.h / h as f32).min(4.0);
    let (iw, ih) = (w as f32 * s, h as f32 * s);
    rect(area.x + (area.w - iw) / 2.0, area.y + (area.h - ih) / 2.0, iw, ih)
}
