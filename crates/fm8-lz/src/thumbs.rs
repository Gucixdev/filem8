//! Thumbnails for the grid: a worker thread decodes the images the grid asks for (`fm8-codec`, files up to 16 MB),
//! shrinks them to fit 128 px and sends back RGBA. Results of an earlier folder are dropped by generation number.
use fm8_codec::{decode, resize};
use fm8_core::class::Class;
use fm8_core::icontheme::IconThemes;
use std::collections::HashMap;
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

/// What the worker is asked for: a picture shrunk to a thumbnail, or the icon of a kind of file from the icon theme.
pub enum Job {
    Thumb(PathBuf),
    Icon(Class, bool),
}

pub struct Thumbs {
    tx: Sender<(u64, usize, Job)>,
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

/// Icon names to try for a kind of file, most specific first.
pub fn icon_names(class: Class, dir: bool) -> Vec<String> {
    let n: &[&str] = if dir {
        &["folder"]
    } else {
        match class {
            Class::Pdf => &["application-pdf", "x-office-document"],
            Class::Image => &["image-x-generic"],
            Class::Audio => &["audio-x-generic"],
            Class::Video => &["video-x-generic"],
            Class::Archive => &["package-x-generic", "application-x-archive"],
            Class::Code => &["text-x-script", "text-x-generic"],
            Class::Document => &["x-office-document", "text-x-generic"],
            Class::Font => &["font-x-generic"],
            Class::Model3d => &["application-x-blender", "model"],
            _ => &["text-x-generic"],
        }
    };
    n.iter().map(|s| s.to_string()).collect()
}

/// `auto` only offers themes the index lists as visible; Adwaita and its legacy PNG set are often hidden yet complete.
pub fn pick_theme(themes: &mut IconThemes) -> Option<String> {
    themes.auto(true, true).or_else(|| ["AdwaitaLegacy", "Adwaita"].into_iter().find(|t| themes.usable(t, true)).map(String::from))
}

fn icon(themes: &mut IconThemes, theme: &Option<String>, class: Class, dir: bool) -> Option<(usize, usize, Vec<u8>)> {
    let theme = theme.as_deref()?;
    let path = themes.find_any(theme, &icon_names(class, dir), 64, 1, true)?;
    icon_file(&path)
}

/// One icon file as RGBA fitted to the thumbnail size: a PNG is decoded, an SVG is drawn at that size.
pub fn icon_file(path: &std::path::Path) -> Option<(usize, usize, Vec<u8>)> {
    let bytes = std::fs::read(path).ok()?;
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg") || e.eq_ignore_ascii_case("svgz")) {
        let img = fm8_preview::image_load::render_svg(&bytes, SIDE).ok()?;
        let f = img.frames.into_iter().next()?;
        return Some((f.w as usize, f.h as usize, f.rgba));
    }
    let (_, img) = decode(&bytes, "png").ok()?;
    let (w, h) = resize::fit(img.w, img.h, SIDE);
    let small = if (w, h) == (img.w, img.h) { img } else { resize::resize(&img, w, h, false) };
    Some((small.w as usize, small.h as usize, small.rgba))
}

impl Default for Thumbs {
    fn default() -> Thumbs {
        let (tx, jobs) = channel::<(u64, usize, Job)>();
        let (out, rx) = channel();
        std::thread::spawn(move || {
            let mut themes = IconThemes::system();
            let theme = pick_theme(&mut themes);
            let mut cache: HashMap<(Class, bool), Option<(usize, usize, Vec<u8>)>> = HashMap::new();
            for (gen, index, job) in jobs {
                let got = match job {
                    Job::Thumb(path) => thumb(&path),
                    Job::Icon(class, dir) => cache.entry((class, dir)).or_insert_with(|| icon(&mut themes, &theme, class, dir)).clone(),
                };
                if let Some((w, h, rgba)) = got {
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
    pub fn ask(&self, gen: u64, index: usize, job: Job) {
        let _ = self.tx.send((gen, index, job));
    }

    pub fn finished(&self) -> Vec<Done> {
        self.rx.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48"><rect width="48" height="48" fill="#ff0000"/></svg>"##;

    #[test]
    fn an_svg_only_theme_gives_icons() {
        let root = std::env::temp_dir().join(format!("fm8lz-svgtheme-{}", std::process::id()));
        let theme = root.join("svgonly");
        for d in ["places", "mimetypes"] {
            std::fs::create_dir_all(theme.join("scalable").join(d)).unwrap();
        }
        std::fs::write(theme.join("index.theme"), "[Icon Theme]\nName=svgonly\nDirectories=scalable/places,scalable/mimetypes\n\n[scalable/places]\nSize=64\nType=Scalable\nMinSize=16\nMaxSize=512\nContext=Places\n\n[scalable/mimetypes]\nSize=64\nType=Scalable\nMinSize=16\nMaxSize=512\nContext=MimeTypes\n").unwrap();
        std::fs::write(theme.join("scalable/places/folder.svg"), SVG).unwrap();
        std::fs::write(theme.join("scalable/mimetypes/text-x-generic.svg"), SVG).unwrap();
        let mut themes = IconThemes::new(vec![root.clone()]);
        let picked = pick_theme(&mut themes);
        assert_eq!(picked.as_deref(), Some("svgonly"));
        let (w, h, rgba) = icon(&mut themes, &picked, Class::Text, false).expect("the generic icon is drawn");
        assert_eq!((w, h, rgba.len()), (SIDE as usize, SIDE as usize, (SIDE * SIDE * 4) as usize));
        assert_eq!(&rgba[..4], &[255, 0, 0, 255]);
        assert!(icon(&mut themes, &picked, Class::Text, true).is_some());
        let _ = std::fs::remove_dir_all(&root);
    }
}
