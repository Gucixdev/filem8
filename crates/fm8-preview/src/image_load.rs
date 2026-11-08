use crate::{Frame, ImageData, Opts, Preview};
use std::path::Path;

fn downscale(img: fm8_codec::Image, max: u32) -> (u32, u32, Vec<u8>) {
    let (nw, nh) = fm8_codec::resize::fit(img.w, img.h, max);
    let img = fm8_codec::resize::resize(&img, nw, nh, false);
    (img.w, img.h, img.rgba)
}

pub fn load(path: &Path, o: &Opts) -> Preview {
    let ext = fm8_core::class::ext_of(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    #[cfg(feature = "svg")]
    if ext == "svg" || ext == "svgz" {
        return load_svg(path, o);
    }
    match decode(path, o) {
        Ok(d) => Preview::Image(d),
        Err(e) => Preview::Error(e),
    }
}

pub fn decode(path: &Path, o: &Opts) -> Result<ImageData, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let ext = fm8_core::class::ext_of(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    match fm8_codec::sniff(&bytes, &ext) {
        Some(fm8_codec::Format::Gif) => {
            let frames = fm8_codec::gif::decode(&bytes, o.max_frames.max(1))?;
            let (ow, oh) = (frames[0].img.w, frames[0].img.h);
            let out = frames.into_iter().map(|f| { let (w, h, rgba) = downscale(f.img, o.max_dim); Frame { w, h, rgba, delay_ms: f.delay_ms } }).collect();
            Ok(ImageData { frames: out, orig_w: ow, orig_h: oh, format: "GIF".into() })
        }
        Some(fmt) => match fm8_codec::decode(&bytes, &ext) {
            Ok((_, img)) => {
                let (ow, oh) = (img.w, img.h);
                let (w, h, rgba) = downscale(img, o.max_dim);
                Ok(ImageData { frames: vec![Frame { w, h, rgba, delay_ms: 0 }], orig_w: ow, orig_h: oh, format: fmt.name().into() })
            }
            #[cfg(feature = "extra-codecs")]
            Err(_) => extra::decode(&bytes, o),
            #[cfg(not(feature = "extra-codecs"))]
            Err(e) => Err(e),
        },
        #[cfg(feature = "extra-codecs")]
        None => extra::decode(&bytes, o),
        #[cfg(not(feature = "extra-codecs"))]
        None => Err("unsupported image format".into()),
    }
}

/// WebP, AVIF and progressive JPEG still go through the `image` crate (feature `extra-codecs`).
#[cfg(feature = "extra-codecs")]
mod extra {
    use super::*;
    use image::{AnimationDecoder, ImageDecoder};
    pub fn decode(bytes: &[u8], o: &Opts) -> Result<ImageData, String> {
        let fmt = image::guess_format(bytes).map_err(|e| e.to_string())?;
        let name = format!("{fmt:?}").to_uppercase();
        if fmt == image::ImageFormat::WebP {
            let d = image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
            let (w, h) = d.dimensions();
            if d.has_animation() {
                let mut out = vec![];
                for fr in d.into_frames().take(o.max_frames) {
                    let fr = fr.map_err(|e| e.to_string())?;
                    let (n, dd) = fr.delay().numer_denom_ms();
                    let delay = if dd == 0 { 100 } else { (n / dd).max(20) };
                    let b = fr.into_buffer();
                    let (fw, fh, rgba) = downscale(fm8_codec::Image { w: b.width(), h: b.height(), rgba: b.into_raw() }, o.max_dim);
                    out.push(Frame { w: fw, h: fh, rgba, delay_ms: delay });
                }
                return Ok(ImageData { frames: out, orig_w: w, orig_h: h, format: name });
            }
        }
        let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgba8();
        let (ow, oh) = (img.width(), img.height());
        let (w, h, rgba) = downscale(fm8_codec::Image { w: ow, h: oh, rgba: img.into_raw() }, o.max_dim);
        Ok(ImageData { frames: vec![Frame { w, h, rgba, delay_ms: 0 }], orig_w: ow, orig_h: oh, format: name })
    }
}

#[cfg(feature = "svg")]
fn load_svg(path: &Path, o: &Opts) -> Preview {
    let data = match std::fs::read(path) { Ok(d) => d, Err(e) => return Preview::Error(e.to_string()) };
    match render_svg(&data, o.max_dim) {
        Ok(d) => Preview::Image(d),
        Err(e) => Preview::Error(e),
    }
}

#[cfg(feature = "svg")]
pub fn render_svg(data: &[u8], max: u32) -> Result<ImageData, String> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_data(data, &usvg::Options::default()).map_err(|e| e.to_string())?;
    let sz = tree.size();
    let (ow, oh) = (sz.width().ceil().max(1.0) as u32, sz.height().ceil().max(1.0) as u32);
    // vector: render at the target size, not the nominal size
    let scale = max as f32 / sz.width().max(sz.height()).max(1.0);
    let (w, h) = (((sz.width() * scale).round() as u32).max(1), ((sz.height() * scale).round() as u32).max(1));
    let mut pm = tiny_skia::Pixmap::new(w, h).ok_or("bad svg size")?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pm.as_mut());
    // tiny-skia is premultiplied; demultiply
    let mut rgba = pm.take();
    for p in rgba.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a > 0 && a < 255 {
            for c in &mut p[..3] { *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8; }
        }
    }
    Ok(ImageData { frames: vec![Frame { w, h, rgba, delay_ms: 0 }], orig_w: ow, orig_h: oh, format: "SVG".into() })
}
