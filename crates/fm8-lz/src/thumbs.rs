//! Thumbnails for the grid: a worker thread decodes the images the grid asks for (`fm8-codec`, files up to 16 MB),
//! shrinks them to fit 128 px and sends back RGBA. Results of an earlier folder are dropped by generation number.
use fm8_codec::{decode, resize};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

const MAX_BYTES: u64 = 16 << 20;
const SIDE: u32 = 128;

pub struct Done {
    pub gen: u64,
    pub index: usize,
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

pub struct Thumbs {
    tx: Sender<(u64, usize, PathBuf)>,
    rx: Receiver<Done>,
}

/// Decode one file into a thumbnail; None when it is too large, unreadable or not an image the codecs know.
pub fn thumb(path: &std::path::Path) -> Option<(usize, usize, Vec<u8>)> {
    if std::fs::metadata(path).ok()?.len() > MAX_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let (_, img) = decode(&bytes, &ext).ok()?;
    let (w, h) = resize::fit(img.w, img.h, SIDE);
    let small = if (w, h) == (img.w, img.h) { img } else { resize::resize(&img, w, h, false) };
    Some((small.w as usize, small.h as usize, small.rgba))
}

impl Default for Thumbs {
    fn default() -> Thumbs {
        let (tx, jobs) = channel::<(u64, usize, PathBuf)>();
        let (out, rx) = channel();
        std::thread::spawn(move || {
            for (gen, index, path) in jobs {
                if let Some((w, h, rgba)) = thumb(&path) {
                    if out.send(Done { gen, index, w, h, rgba }).is_err() {
                        break;
                    }
                }
            }
        });
        Thumbs { tx, rx }
    }
}

impl Thumbs {
    pub fn ask(&self, gen: u64, index: usize, path: PathBuf) {
        let _ = self.tx.send((gen, index, path));
    }

    pub fn finished(&self) -> Vec<Done> {
        self.rx.try_iter().collect()
    }
}
