//! fm8-preview: turns a file into something the preview pane can show.
//! All loaders are pure (path in, data out) so they run on worker threads and
//! are testable without a window.
pub mod archive;
pub mod blend;
#[cfg(feature = "audio")]
pub mod audio;
pub mod json;
pub mod image_load;
pub mod model;
pub mod gltf;
pub mod pdf;
pub mod sheet;
pub mod text;

use fm8_core::class::{classify_name, sniff, Class};
use std::path::Path;

#[derive(Clone, Debug)]
pub struct Frame {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
    pub delay_ms: u32,
}

#[derive(Clone, Debug)]
pub struct ImageData {
    pub frames: Vec<Frame>,
    /// Original pixel size (frames may be downscaled).
    pub orig_w: u32,
    pub orig_h: u32,
    pub format: String,
}

#[derive(Clone, Debug)]
pub struct ArchiveEntry {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
}

#[derive(Clone, Debug)]
pub struct AudioData {
    /// Peak envelope 0..1, fixed number of bins.
    pub peaks: Vec<f32>,
    pub duration_s: f64,
    pub sample_rate: u32,
    pub channels: u16,
    pub codec: String,
}

#[derive(Clone, Debug)]
pub enum Preview {
    Image(ImageData),
    Text(text::HighlightedText),
    Archive { kind: String, entries: Vec<ArchiveEntry>, truncated: bool },
    Audio(AudioData),
    Model(model::Mesh),
    /// Plain metadata lines for everything we can't show.
    Info(Vec<String>),
    Error(String),
}

#[derive(Clone, Debug)]
pub struct Opts {
    pub max_dim: u32,
    pub max_text_bytes: usize,
    pub max_frames: usize,
}

impl Default for Opts {
    fn default() -> Self {
        Opts { max_dim: 1024, max_text_bytes: 64 * 1024, max_frames: 300 }
    }
}

pub fn load(path: &Path, o: &Opts) -> Preview {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let md = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) => return Preview::Error(e.to_string()),
    };
    if md.is_dir() {
        return Preview::Info(dir_info(path));
    }
    // a FIFO blocks on open and a device never ends: only regular files are read
    if !md.is_file() {
        return Preview::Info(vec![format!("{name} (special file)")]);
    }
    let mut class = classify_name(&name, false);
    if class == Class::Other {
        let mut head = [0u8; 512];
        if let Ok(mut f) = std::fs::File::open(path) {
            let n = std::io::Read::read(&mut f, &mut head).unwrap_or(0);
            class = sniff(&head[..n]);
        }
    }
    let ext = fm8_core::class::ext_of(&name);
    match class {
        Class::Image => image_load::load(path, o),
        Class::Text | Class::Code | Class::Data if ext == "json" && sheet::looks_like_sheet(path) => sheet::load(path, o),
        Class::Text | Class::Code => text::load(path, &ext, o),
        Class::Archive => archive::load(path, &name),
        #[cfg(feature = "audio")]
        Class::Audio => audio::load(path),
        Class::Model3d if ext == "blend" => blend::load(path),
        Class::Model3d => model::load(path),
        Class::Pdf => pdf::load(path, o),
        _ => {
            // JSON atlases are classified as text; others show metadata only
            Preview::Info(vec![format!("{} ({})", name, fm8_core::class::class_label(class)), format!("{} bytes", md.len())])
        }
    }
}

fn dir_info(p: &Path) -> Vec<String> {
    let mut n = 0;
    let mut dirs = 0;
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten().take(100_000) {
            n += 1;
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) { dirs += 1; }
        }
    }
    vec![format!("{} items ({} folders)", n, dirs)]
}
