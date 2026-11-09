//! Theme icons for files, folders and places: resolved on a worker thread via
//! the freedesktop icon theme lookup in fm8-core, rasterised (PNG with our own
//! decoder, SVG with resvg, the `svg-icons` feature, on by default) and cached by (name, size).
//! `get()` returns None until an icon is ready; callers draw the procedural icon meanwhile.
use fm8_core::icontheme::{names_for_mime, names_for_place, IconThemes};
use fm8_ui::{Icon, Image};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

pub const CAN_SVG: bool = cfg!(feature = "svg-icons");

enum Msg {
    Setup { search: Vec<PathBuf>, theme: String, dark: bool },
    Load { key: (String, u32), names: Vec<String>, tint: Option<[u8; 3]> },
}

enum Slot { Pending, Ready(Arc<Image>), Missing }

pub struct IconSet {
    tx: Option<Sender<Msg>>,
    rx: Option<Receiver<((String, u32), Option<Arc<Image>>)>>,
    map: HashMap<(String, u32), Slot>,
    pending: usize,
    pub theme: String,
}

/// Sizes we rasterise at; a request snaps up to the next one so scaling stays gentle.
pub fn bucket(px: f32) -> u32 {
    [16u32, 22, 24, 32, 48, 64, 96, 128, 256].into_iter().find(|&b| b as f32 >= px - 0.5).unwrap_or(256)
}

pub fn names_for_icon(i: Icon) -> Option<Vec<String>> {
    Some(names_for_place(match i {
        Icon::Folder => "folder", Icon::Home => "home", Icon::Desktop => "desktop", Icon::Document => "documents", Icon::Download => "downloads",
        Icon::Music => "music", Icon::Picture => "pictures", Icon::Trash => "trash", Icon::Disk => "disk", Icon::Star => "bookmark", Icon::Clock => "recent",
        _ => return None,
    }))
}

impl IconSet {
    /// A disabled set (headless tests, "builtin" theme): `get()` is always None.
    pub fn off() -> IconSet { IconSet { tx: None, rx: None, map: HashMap::new(), pending: 0, theme: "builtin".into() } }

    pub fn new(search: Vec<PathBuf>, theme: &str, dark: bool) -> IconSet {
        let mut s = IconSet::off();
        s.configure(search, theme, dark);
        s
    }

    /// (Re)start the worker for `theme` ("auto" picks the best installed one, "builtin" disables).
    pub fn configure(&mut self, search: Vec<PathBuf>, theme: &str, dark: bool) {
        self.map.clear();
        self.pending = 0;
        self.theme = theme.to_string();
        if theme == "builtin" { self.tx = None; self.rx = None; return }
        let (tx, wrx) = channel::<Msg>();
        let (dtx, rx) = channel();
        std::thread::spawn(move || worker(wrx, dtx));
        let _ = tx.send(Msg::Setup { search, theme: theme.to_string(), dark });
        self.tx = Some(tx);
        self.rx = Some(rx);
    }

    pub fn enabled(&self) -> bool { self.tx.is_some() }

    pub fn get(&mut self, names: impl FnOnce() -> Vec<String>, key: &str, px: f32) -> Option<Arc<Image>> { self.get_tinted(names, key, px, None) }

    pub fn get_tinted(&mut self, names: impl FnOnce() -> Vec<String>, key: &str, px: f32, tint: Option<[u8; 3]>) -> Option<Arc<Image>> {
        let tx = self.tx.as_ref()?;
        let k = (match tint { Some(c) => format!("{key}#{:02x}{:02x}{:02x}", c[0], c[1], c[2]), None => key.to_string() }, bucket(px));
        match self.map.get(&k) {
            Some(Slot::Ready(i)) => Some(i.clone()),
            Some(_) => None,
            None => {
                self.map.insert(k.clone(), Slot::Pending);
                self.pending += 1;
                let _ = tx.send(Msg::Load { key: k, names: names(), tint });
                None
            }
        }
    }

    pub fn for_mime(&mut self, mime: &str, px: f32) -> Option<Arc<Image>> { let m = mime.to_string(); self.get(move || names_for_mime(&m), mime, px) }

    pub fn for_icon(&mut self, i: Icon, px: f32) -> Option<Arc<Image>> {
        self.get(|| names_for_icon(i).unwrap_or_default(), &format!("icon:{i:?}"), px)
    }

    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        let mut ch = false;
        while let Ok((k, img)) = rx.try_recv() {
            self.pending = self.pending.saturating_sub(1);
            self.map.insert(k, match img { Some(i) => Slot::Ready(i), None => Slot::Missing });
            ch = true;
        }
        ch
    }

    pub fn busy(&self) -> bool { self.pending > 0 }
}

fn worker(rx: Receiver<Msg>, tx: Sender<((String, u32), Option<Arc<Image>>)>) {
    let mut themes = IconThemes::system();
    let mut theme = String::from("hicolor");
    while let Ok(m) = rx.recv() {
        match m {
            Msg::Setup { search, theme: t, dark } => {
                themes = IconThemes::new(search);
                theme = if t == "auto" { themes.auto(CAN_SVG, dark).unwrap_or_else(|| "hicolor".into()) } else { t };
            }
            Msg::Load { key, names, tint } => {
                let img = themes.find_any(&theme, &names, key.1, 1, CAN_SVG).and_then(|p| rasterise(&p, key.1)).map(|i| match tint { Some(c) => tinted(&i, c), None => i });
                if tx.send((key, img)).is_err() { return }
            }
        }
    }
}

fn rasterise(p: &std::path::Path, size: u32) -> Option<Arc<Image>> {
    let data = std::fs::read(p).ok()?;
    if p.extension().and_then(|e| e.to_str()) == Some("svg") {
        #[cfg(feature = "svg-icons")]
        {
            let d = fm8_preview::image_load::render_svg(&data, size).ok()?;
            let f = d.frames.into_iter().next()?;
            return Some(Image::new(f.w, f.h, f.rgba));
        }
        #[cfg(not(feature = "svg-icons"))]
        return None;
    }
    let img = fm8_codec::png::decode(&data).ok()?;
    let (w, h) = fm8_codec::resize::fit(img.w, img.h, size);
    let img = if (w, h) != (img.w, img.h) { fm8_codec::resize::resize(&img, w, h, false) } else { img };
    Some(Image::new(img.w, img.h, img.rgba))
}

/// Monochrome (symbolic) icon recoloured: keep the alpha mask, replace RGB.
fn tinted(i: &Image, c: [u8; 3]) -> Arc<Image> {
    let mut v = i.rgba.clone();
    for p in v.chunks_mut(4) { p[0] = c[0]; p[1] = c[1]; p[2] = c[2]; }
    Image::new(i.w, i.h, v)
}

/// Toolbar glyph -> symbolic theme names (action icons).
pub fn symbolic_names(i: Icon) -> Option<Vec<String>> {
    let n: &[&str] = match i {
        Icon::Back => &["go-previous"], Icon::Forward => &["go-next"], Icon::Up => &["go-up"], Icon::Refresh => &["view-refresh"],
        Icon::List => &["view-list-details", "view-list-text"], Icon::Grid => &["view-list-icons", "view-grid"], Icon::Compact => &["view-list-text", "view-list-compact"],
        Icon::Split => &["view-split-left-right", "view-dual", "view-right-new"], Icon::Eye => &["view-preview", "document-preview", "view-visible"],
        Icon::Search => &["edit-find", "system-search"], Icon::Menu => &["open-menu", "application-menu", "configure"],
        Icon::Plus => &["list-add", "tab-new"], Icon::Close => &["window-close", "tab-close"],
        _ => return None,
    };
    Some(n.iter().flat_map(|s| [format!("{s}-symbolic"), s.to_string()]).collect())
}

impl IconSet {
    /// Symbolic action icon tinted to `rgb` (None until rasterised or when the theme lacks it).
    pub fn symbolic(&mut self, i: Icon, px: f32, rgb: [u8; 3]) -> Option<Arc<Image>> {
        let names = symbolic_names(i)?;
        self.get_tinted(move || names, &format!("sym:{i:?}"), px, Some(rgb))
    }
}

/// Ids offered by the settings panel: "auto", "builtin", then every installed theme.
pub fn theme_choices() -> &'static [&'static str] {
    static C: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    C.get_or_init(|| {
        let mut v: Vec<&'static str> = vec!["auto", "builtin"];
        for t in IconThemes::system().installed() { v.push(Box::leak(t.into_boxed_str())) }
        v
    })
}

impl IconSet {
    /// Draw a theme icon fitted in `r`; false when none is ready (caller draws the procedural one).
    pub fn draw(&mut self, ui: &mut fm8_ui::Ui, img: Option<Arc<Image>>, r: fm8_ui::geom::Rect) -> bool {
        let Some(img) = img else { return false };
        let s = (r.w / img.w as f32).min(r.h / img.h as f32);
        let (w, h) = (img.w as f32 * s, img.h as f32 * s);
        ui.draw().cmds.push(fm8_ui::Cmd::Image { r: fm8_ui::geom::rect(r.x + (r.w - w) / 2.0, r.y + (r.h - h) / 2.0, w, h), img, smooth: true });
        true
    }
}

/// Well-known folder under $HOME -> place kind, so ~/Music gets the music folder icon.
pub fn place_kind(path: &std::path::Path) -> Option<&'static str> {
    if path.parent()? != fm8_core::paths::home() { return None }
    Some(match path.file_name()?.to_str()? { "Desktop" => "desktop", "Documents" => "documents", "Downloads" => "downloads", "Music" => "music", "Pictures" => "pictures", "Videos" => "videos", _ => return None })
}

impl IconSet {
    /// Theme icon for a file/folder by MIME (or place kind for ~/Music etc.); false = draw the procedural one.
    pub fn entry(&mut self, ui: &mut fm8_ui::Ui, e: &fm8_core::listing::Entry, r: fm8_ui::geom::Rect) -> bool {
        if !self.enabled() { return false }
        let (key, place) = match (e.is_dir, place_kind(&e.path)) {
            (true, Some(k)) => (format!("place:{k}"), Some(k)),
            _ => (fm8_core::class::mime_of(&e.name, e.is_dir).to_string(), None),
        };
        let k2 = key.clone();
        let img = self.get(move || match place { Some(p) => names_for_place(p), None => names_for_mime(&k2) }, &key, r.w);
        self.draw(ui, img, r)
    }
}
