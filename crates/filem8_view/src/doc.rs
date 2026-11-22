//! File -> pages. The loaders are filem8's own (fm8-preview), so the viewer opens exactly what the preview pane opens;
//! what has no decoder falls back to a hex dump, so every file shows *something* and nothing is ever written.

use std::path::Path;

use fm8_preview::{text::Tok, ImageData, Opts, Preview};
use fm8_ui::{draw::DrawList, geom::{rect, Color}, soft::Framebuffer, text, Theme};

pub struct Page {
    pub w: u32,
    pub h: u32,
    /// RGBA8; alpha below 255 is shown over a checkerboard
    pub rgba: Vec<u8>,
    pub delay_ms: u32,
}

pub struct Doc {
    pub pages: Vec<Page>,
    pub meta: String,
    /// kept so a model can be orbited (re-rendered at another angle)
    pub mesh: Option<fm8_preview::model::Mesh>,
    pub yaw: f32,
    /// a page of text (listing, hex, source): opens at 100% on its top and the wheel scrolls
    pub text: bool,
}

pub fn opts() -> Opts {
    Opts { max_dim: 2048, max_text_bytes: 1 << 20, max_frames: 500 }
}

const FONT: f32 = 13.0;
const LINE: f32 = 17.0;
const MARGIN: f32 = 12.0;
/// A text page is one tall bitmap; this keeps it under ~60 MB.
const MAX_PAGE_H: f32 = 16384.0;
const MAX_PAGE_W: f32 = 4096.0;

fn human(n: u64) -> String {
    match n {
        0..=1023 => format!("{n} B"),
        1024..=1_048_575 => format!("{:.1} KB", n as f64 / 1024.0),
        _ => format!("{:.1} MB", n as f64 / 1_048_576.0),
    }
}

fn col(t: &Theme, tok: Tok) -> Color {
    match tok {
        Tok::Plain => t.fg,
        Tok::Keyword => t.accent,
        Tok::Str => t.ok,
        Tok::Comment => t.fg_dim,
        Tok::Number => t.warn,
        Tok::Heading => t.link,
        Tok::Punct => t.fg_dim,
    }
}

/// Lines of coloured words drawn onto one page.
pub fn text_page(lines: &[Vec<(Tok, String)>], t: &Theme) -> Page {
    let max_lines = ((MAX_PAGE_H - 2.0 * MARGIN) / LINE) as usize;
    let shown = &lines[..lines.len().min(max_lines)];
    let width = shown
        .iter()
        .map(|l| l.iter().map(|(_, s)| text::measure(s, FONT, false)).sum::<f32>())
        .fold(0.0f32, f32::max);
    let w = (width + 2.0 * MARGIN).clamp(480.0, MAX_PAGE_W).ceil() as u32;
    let h = (shown.len() as f32 * LINE + 2.0 * MARGIN).ceil().max(64.0) as u32;
    let mut dl = DrawList::default();
    for (i, l) in shown.iter().enumerate() {
        let mut x = MARGIN;
        for (tok, s) in l {
            dl.text(x, MARGIN + i as f32 * LINE, s, FONT, col(t, *tok), matches!(tok, Tok::Heading));
            x += text::measure(s, FONT, matches!(tok, Tok::Heading));
        }
    }
    let mut fb = Framebuffer::new(w, h, t.bg);
    fb.render(&dl, 1.0);
    Page { w, h, rgba: fb.px, delay_ms: 100 }
}

fn plain(lines: Vec<String>) -> Vec<Vec<(Tok, String)>> {
    lines.into_iter().map(|l| vec![(Tok::Plain, l)]).collect()
}

/// `00000000  48 65 6c 6c ...  |Hello...|`
pub fn hex_lines(bytes: &[u8]) -> Vec<String> {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(i, c)| {
            let hex: String = (0..16).map(|j| c.get(j).map_or("   ".to_string(), |b| format!("{b:02x} "))).collect();
            let asc: String = c.iter().map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '.' }).collect();
            format!("{:08x}  {hex} |{asc}|", i * 16)
        })
        .collect()
}

fn waveform(peaks: &[f32], t: &Theme) -> Page {
    let (w, h) = (1200u32, 300u32);
    let mut dl = DrawList::default();
    let n = peaks.len().max(1) as f32;
    let bar = (w as f32 - 2.0 * MARGIN) / n;
    for (i, p) in peaks.iter().enumerate() {
        let half = (p.clamp(0.0, 1.0) * (h as f32 / 2.0 - MARGIN)).max(1.0);
        dl.rect(rect(MARGIN + i as f32 * bar, h as f32 / 2.0 - half, (bar - 0.5).max(1.0), half * 2.0), t.accent);
    }
    let mut fb = Framebuffer::new(w, h, t.bg);
    fb.render(&dl, 1.0);
    Page { w, h, rgba: fb.px, delay_ms: 100 }
}

fn from_image(d: ImageData) -> Doc {
    let meta = format!("{} - {} x {}{}", d.format, d.orig_w, d.orig_h, if d.frames.len() > 1 { format!(" - {} frames", d.frames.len()) } else { String::new() });
    let pages = d.frames.into_iter().map(|f| Page { w: f.w, h: f.h, rgba: f.rgba, delay_ms: f.delay_ms.max(20) }).collect();
    Doc { pages, meta, mesh: None, yaw: 0.6, text: false }
}

fn model_page(m: &fm8_preview::model::Mesh, yaw: f32) -> Page {
    Page { w: 1024, h: 1024, rgba: fm8_preview::model::render(m, yaw, 1024), delay_ms: 100 }
}

fn one(pages: Vec<Page>, meta: String) -> Doc {
    Doc { pages, meta, mesh: None, yaw: 0.6, text: false }
}

fn text_doc(page: Page, meta: String) -> Doc {
    Doc { pages: vec![page], meta, mesh: None, yaw: 0.6, text: true }
}

pub fn load(path: &Path, t: &Theme) -> Doc {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    match fm8_preview::load(path, &opts()) {
        Preview::Image(d) => from_image(d),
        Preview::Text(tx) => {
            let meta = format!("{} - {}{}", tx.lang, human(tx.total_bytes), if tx.truncated { " (start only)" } else { "" });
            text_doc(text_page(&tx.lines, t), meta)
        }
        Preview::Archive { kind, entries, truncated } => {
            let mut lines: Vec<String> = entries.iter().map(|e| if e.is_dir { format!("{}/", e.name) } else { format!("{}   ({})", e.name, human(e.size)) }).collect();
            if truncated {
                lines.push("...".into());
            }
            text_doc(text_page(&plain(lines), t), format!("{kind} - {} entries", entries.len()))
        }
        Preview::Audio(a) => {
            let meta = format!("{} - {} Hz - {} ch - {}:{:02}", a.codec, a.sample_rate, a.channels, a.duration_s as u64 / 60, a.duration_s as u64 % 60);
            one(vec![waveform(&a.peaks, t)], meta)
        }
        Preview::Model(m) => {
            let meta = format!("{} - {} triangles ([ and ] orbit)", m.source, m.triangles());
            let page = model_page(&m, 0.6);
            Doc { pages: vec![page], meta, mesh: Some(m), yaw: 0.6, text: false }
        }
        Preview::Error(e) => text_doc(text_page(&plain(vec![format!("{name}: {e}")]), t), "error".into()),
        Preview::Info(info) => {
            // no decoder: the file's own bytes, so it is never a blank page
            let mut lines = vec![vec![(Tok::Heading, name.clone())]];
            lines.extend(plain(info));
            let mut head = vec![0u8; 4096];
            let n = std::fs::File::open(path).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or(0);
            if n > 0 {
                lines.push(vec![(Tok::Plain, String::new())]);
                lines.push(vec![(Tok::Heading, format!("first {n} of {} bytes", size))]);
                lines.extend(plain(hex_lines(&head[..n])));
            }
            text_doc(text_page(&lines, t), format!("no decoder - {}", human(size)))
        }
    }
}

impl Doc {
    /// Orbit a model: render it again at another angle.
    pub fn orbit(&mut self, dyaw: f32) -> bool {
        let Some(m) = &self.mesh else { return false };
        self.yaw += dyaw;
        self.pages = vec![model_page(m, self.yaw)];
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_dump_lines_up() {
        let l = hex_lines(b"Hello, world! 0123456789");
        assert_eq!(l.len(), 2);
        assert!(l[0].starts_with("00000000  48 65 6c 6c 6f 2c 20 77"));
        assert!(l[0].ends_with("|Hello, world! 01|"));
        // a short last row keeps the columns
        assert_eq!(l[1].find('|'), l[0].find('|'));
    }

    #[test]
    fn a_text_page_has_the_right_size_and_is_opaque() {
        let t = Theme::dark();
        let p = text_page(&plain(vec!["one".into(), "two".into(), "three".into()]), &t);
        assert_eq!(p.rgba.len(), (p.w * p.h * 4) as usize);
        assert!(p.rgba.chunks_exact(4).all(|px| px[3] == 255));
        assert!(p.h >= 64 && p.w >= 480);
    }

    #[test]
    fn a_huge_text_is_cut_not_exploded() {
        let t = Theme::dark();
        let lines = plain((0..5000).map(|i| format!("line {i}")).collect());
        let p = text_page(&lines, &t);
        assert!(p.h as f32 <= MAX_PAGE_H);
    }
}
