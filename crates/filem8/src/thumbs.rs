//! Background thumbnail loader for the tile view. Worker threads produce a
//! small picture plus a one-line info string ("1024x1024 PNG", "12k tris").
//! Results are cached on disk in the freedesktop thumbnail directory
//! (`~/.cache/thumbnails/{normal,large}`, MD5 of the URI, `Thumb::MTime`
//! checked), shared with Thunar/Dolphin. The UI asks `get()` every frame and
//! gets `None` until ready (the caller draws the theme icon meanwhile); the
//! first request of a frame is the visible-first order because only visible
//! cells ask. `anim()` lazily loads extra frames (gif/webp, rotating 3D).
use fm8_core::class::{classify_name, Class};
use fm8_core::thumbcache;
use fm8_ui::Image;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

#[derive(Clone)]
pub struct Thumb {
    pub img: Arc<Image>,
    pub info: String,
    /// Has transparent pixels: the tile shows a checkerboard behind it.
    pub alpha: bool,
}

fn has_alpha(rgba: &[u8]) -> bool { rgba.chunks(4).any(|p| p[3] < 250) }

pub type Frames = Arc<Vec<(Arc<Image>, u32)>>;

enum Slot {
    Pending,
    Ready(Thumb),
    Failed,
}

enum Job {
    Still(PathBuf, u32),
    Anim(PathBuf),
}

enum Done {
    Still(PathBuf, Option<Thumb>),
    Anim(PathBuf, Option<Frames>),
}

pub struct Thumbs {
    map: HashMap<PathBuf, Slot>,
    anims: HashMap<PathBuf, Option<Frames>>,
    anim_wait: usize,
    jobs: Sender<Job>,
    done: Receiver<Done>,
    pending: usize,
    px: u32,
}

impl Default for Thumbs {
    fn default() -> Self { Thumbs::new() }
}

fn mtime_s(p: &Path) -> Option<u64> {
    std::fs::metadata(p).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

fn kfmt(n: usize) -> String { if n >= 1000 { format!("{}k", n / 1000) } else { n.to_string() } }

fn waveform(peaks: &[f32], w: u32, h: u32) -> fm8_codec::Image {
    let mut im = fm8_codec::Image::new(w, h);
    for x in 0..w {
        let p = peaks.get(x as usize * peaks.len() / w as usize).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        let half = ((p * h as f32 * 0.45) as u32).max(1);
        for y in (h / 2 - half.min(h / 2))..(h / 2 + half).min(h) {
            let o = (y * w + x) as usize * 4;
            im.rgba[o..o + 4].copy_from_slice(&[90, 200, 140, 255]);
        }
    }
    im
}

fn from_png(bytes: &[u8]) -> Option<fm8_codec::Image> { fm8_codec::png::decode(bytes).ok() }

fn external_png(cmd: &str, args: &[&str]) -> Option<fm8_codec::Image> {
    let out = std::process::Command::new(cmd).args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
    if out.status.success() { from_png(&out.stdout) } else { None }
}

fn generate(p: &Path, px: u32) -> Option<Thumb> {
    let name = p.file_name()?.to_string_lossy().into_owned();
    let mut class = classify_name(&name, false);
    if class == Class::Other {
        let mut head = [0u8; 512];
        if let Ok(mut f) = std::fs::File::open(p) { let n = std::io::Read::read(&mut f, &mut head).unwrap_or(0); class = fm8_core::class::sniff(&head[..n]); }
    }
    let o = fm8_preview::Opts { max_dim: px, max_frames: 1, ..Default::default() };
    match class {
        Class::Image | Class::Pdf => match fm8_preview::load(p, &o) {
            fm8_preview::Preview::Image(d) => {
                let f = d.frames.into_iter().next()?;
                let info = if class == Class::Pdf { "PDF".to_string() } else { format!("{}x{} {}", d.orig_w, d.orig_h, d.format.to_uppercase()) };
                { let alpha = has_alpha(&f.rgba); Some(Thumb { img: Image::new(f.w, f.h, f.rgba), info, alpha }) }
            }
            _ => None,
        },
        Class::Model3d => match fm8_preview::model::load_mesh(p) {
            Ok(m) => Some(Thumb { img: Image::new(px, px, fm8_preview::model::render(&m, 0.6, px)), info: format!("{} tris", kfmt(m.triangles())), alpha: true }),
            Err(_) => None,
        },
        Class::Video => {
            let im = external_png("ffmpeg", &["-v", "error", "-ss", "1", "-i", &p.to_string_lossy(), "-frames:v", "1", "-vf", &format!("scale={px}:-2"), "-f", "image2pipe", "-vcodec", "png", "-"])
                .or_else(|| external_png("ffmpeg", &["-v", "error", "-i", &p.to_string_lossy(), "-frames:v", "1", "-vf", &format!("scale={px}:-2"), "-f", "image2pipe", "-vcodec", "png", "-"]))?;
            let ext = fm8_core::class::ext_of(&name).to_uppercase();
            Some(Thumb { img: Image::new(im.w, im.h, im.rgba), info: format!("Video {ext}"), alpha: false })
        }
        Class::Audio => match fm8_preview::load(p, &o) {
            fm8_preview::Preview::Audio(a) => {
                let s = a.duration_s as u64;
                Some(Thumb { alpha: true, img: { let w = waveform(&a.peaks, px, px / 2); Image::new(w.w, w.h, w.rgba) }, info: format!("{}:{:02} {}", s / 60, s % 60, a.codec.to_uppercase()) })
            }
            _ => None,
        },
        _ => None,
    }
}

/// Cache hit -> Thumb; miss -> generate and store.
fn make(p: &Path, px: u32) -> Option<Thumb> {
    if p.is_dir() { return make_dir(p); }
    let mt = mtime_s(p)?;
    let cf = thumbcache::cache_file(p, px);
    if let Ok(b) = std::fs::read(&cf) {
        let t = fm8_codec::png::text_chunks(&b);
        let get = |k: &str| t.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
        if get("Thumb::URI").as_deref() == Some(&thumbcache::uri(p)) && get("Thumb::MTime").and_then(|v| v.parse::<u64>().ok()) == Some(mt) {
            if let Some(im) = from_png(&b) {
                let alpha = has_alpha(&im.rgba);
                return Some(Thumb { img: Image::new(im.w, im.h, im.rgba), info: get("Fm8::Info").unwrap_or_default(), alpha });
            }
        }
    }
    let th = generate(p, px)?;
    let im = fm8_codec::Image { w: th.img.w, h: th.img.h, rgba: th.img.rgba.clone() };
    let png = fm8_codec::png::encode_text(&im, &[("Thumb::URI", &thumbcache::uri(p)), ("Thumb::MTime", &mt.to_string()), ("Fm8::Info", &th.info)]);
    if let Some(dir) = cf.parent() {
        if std::fs::create_dir_all(dir).is_ok() {
            let tmp = cf.with_extension(format!("tmp{}", std::process::id()));
            if std::fs::write(&tmp, png).is_ok() && std::fs::rename(&tmp, &cf).is_err() { let _ = std::fs::remove_file(&tmp); }
        }
    }
    Some(th)
}

/// Folder collage: the first up to four images inside, 2x2 on transparent.
fn make_dir(p: &Path) -> Option<Thumb> {
    let mut names: Vec<PathBuf> = std::fs::read_dir(p).ok()?.flatten().take(400).map(|e| e.path()).filter(|q| q.file_name().map(|n| classify_name(&n.to_string_lossy(), false) == Class::Image).unwrap_or(false) && std::fs::metadata(q).map(|m| m.is_file() && m.len() < 12_000_000).unwrap_or(false)).collect();
    names.sort();
    let mut cells = vec![];
    for q in names { if cells.len() == 4 { break; } if let Some(t) = make(&q, 128) { cells.push(t.img); } }
    if cells.is_empty() { return None; }
    let n = if cells.len() == 1 { 1 } else { 2 };
    let side = 128u32;
    let cell = side / n;
    let mut out = fm8_codec::Image::new(side, side);
    for (i, im) in cells.iter().enumerate() {
        let src = fm8_codec::Image { w: im.w, h: im.h, rgba: im.rgba.clone() };
        let (w, h) = fm8_codec::resize::fit(im.w, im.h, cell - 4);
        let r = fm8_codec::resize::resize(&src, w, h, false);
        let rows = (cells.len() as u32).div_ceil(n);
        let (ox, oy) = ((i as u32 % n) * cell + (cell - w) / 2, (side - rows * cell) / 2 + (i as u32 / n) * cell + (cell - h) / 2);
        for y in 0..h { for x in 0..w {
            let (s, d) = (((y * w + x) * 4) as usize, (((oy + y) * side + ox + x) * 4) as usize);
            out.rgba[d..d + 4].copy_from_slice(&r.rgba[s..s + 4]);
        } }
    }
    Some(Thumb { img: Image::new(out.w, out.h, out.rgba), info: String::new(), alpha: false })
}

fn make_anim(p: &Path) -> Option<Frames> {
    let name = p.file_name()?.to_string_lossy().into_owned();
    match classify_name(&name, false) {
        Class::Image => {
            let o = fm8_preview::Opts { max_dim: 256, max_frames: 60, ..Default::default() };
            match fm8_preview::image_load::load(p, &o) {
                fm8_preview::Preview::Image(d) if d.frames.len() > 1 => Some(Arc::new(d.frames.into_iter().map(|f| (Image::new(f.w, f.h, f.rgba), f.delay_ms.max(20))).collect())),
                _ => None,
            }
        }
        Class::Model3d => {
            let m = fm8_preview::model::load_mesh(p).ok()?;
            Some(Arc::new((0..16).map(|i| (Image::new(128, 128, fm8_preview::model::render(&m, i as f32 * std::f32::consts::TAU / 16.0, 128)), 90)).collect()))
        }
        _ => None,
    }
}

impl Thumbs {
    pub fn new() -> Thumbs {
        let (jtx, jrx) = channel::<Job>();
        let (dtx, drx) = channel();
        let jrx = Arc::new(Mutex::new(jrx));
        for _ in 0..3 {
            let (jrx, dtx) = (jrx.clone(), dtx.clone());
            std::thread::spawn(move || loop {
                let job = { jrx.lock().unwrap().recv() };
                let Ok(job) = job else { return };
                let r = match job {
                    Job::Still(p, px) => { let t = make(&p, px); Done::Still(p, t) }
                    Job::Anim(p) => { let f = make_anim(&p); Done::Anim(p, f) }
                };
                if dtx.send(r).is_err() { return; }
            });
        }
        Thumbs { map: HashMap::new(), anims: HashMap::new(), anim_wait: 0, jobs: jtx, done: drx, pending: 0, px: 128 }
    }

    /// Cache size class follows the tile size (128 / 256 / 512).
    pub fn set_px(&mut self, want: u32) {
        let px = thumbcache::class_px(want);
        if px != self.px { self.px = px; self.map.clear(); }
    }

    pub fn get(&mut self, p: &PathBuf) -> Option<Thumb> {
        match self.map.get(p) {
            Some(Slot::Ready(t)) => Some(t.clone()),
            Some(_) => None,
            None => {
                if self.map.len() > 800 { self.map.retain(|_, s| matches!(s, Slot::Ready(_))); if self.map.len() > 800 { self.map.clear(); } }
                self.map.insert(p.clone(), Slot::Pending);
                self.pending += 1;
                let _ = self.jobs.send(Job::Still(p.clone(), self.px));
                None
            }
        }
    }

    /// Extra frames for hover animation; starts loading on first call.
    pub fn anim(&mut self, p: &PathBuf) -> Option<Frames> {
        match self.anims.get(p) {
            Some(f) => f.clone(),
            None => {
                if self.anims.len() > 40 { self.anims.clear(); }
                self.anims.insert(p.clone(), None);
                self.anim_wait += 1;
                let _ = self.jobs.send(Job::Anim(p.clone()));
                None
            }
        }
    }

    pub fn forget(&mut self) { self.map.retain(|_, s| matches!(s, Slot::Ready(_))); }

    /// Collect finished work. True when something new arrived.
    pub fn poll(&mut self) -> bool {
        let mut ch = false;
        while let Ok(d) = self.done.try_recv() {
            match d {
                Done::Still(p, t) => {
                    self.pending = self.pending.saturating_sub(1);
                    self.map.insert(p, match t { Some(t) => Slot::Ready(t), None => Slot::Failed });
                }
                Done::Anim(p, f) => { self.anim_wait = self.anim_wait.saturating_sub(1); self.anims.insert(p, f); }
            }
            ch = true;
        }
        ch
    }

    pub fn busy(&self) -> bool { self.pending > 0 || self.anim_wait > 0 }
}
