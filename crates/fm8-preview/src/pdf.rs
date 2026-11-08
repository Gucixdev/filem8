//! First page of a PDF via poppler's `pdftoppm`, when installed.
use crate::{Opts, Preview};
use std::path::Path;
use std::process::Command;

pub fn available() -> bool {
    Command::new("pdftoppm").arg("-v").output().is_ok()
}

pub fn load(path: &Path, o: &Opts) -> Preview {
    if !available() {
        return Preview::Info(vec!["PDF".into(), "Install poppler (pdftoppm) for a first-page preview".into()]);
    }
    let out = Command::new("pdftoppm")
        .args(["-png", "-f", "1", "-l", "1", "-singlefile", "-scale-to", &o.max_dim.to_string()])
        .arg(path)
        .arg("-")
        .output();
    match out {
        Ok(r) if r.status.success() && !r.stdout.is_empty() => match image::load_from_memory(&r.stdout) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let (w, h) = (rgba.width(), rgba.height());
                Preview::Image(crate::ImageData { frames: vec![crate::Frame { w, h, rgba: rgba.into_raw(), delay_ms: 0 }], orig_w: w, orig_h: h, format: "PDF p.1".into() })
            }
            Err(e) => Preview::Error(e.to_string()),
        },
        Ok(r) => Preview::Error(String::from_utf8_lossy(&r.stderr).lines().next().unwrap_or("pdftoppm failed").to_string()),
        Err(e) => Preview::Error(e.to_string()),
    }
}
