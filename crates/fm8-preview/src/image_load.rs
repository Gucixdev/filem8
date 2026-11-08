use crate::{Frame, ImageData, Opts, Preview};
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat};
use std::io::BufReader;
use std::path::Path;

fn fit(w: u32, h: u32, max: u32) -> (u32, u32) {
    if w <= max && h <= max { return (w, h); }
    let s = max as f32 / w.max(h) as f32;
    (((w as f32 * s).round() as u32).max(1), ((h as f32 * s).round() as u32).max(1))
}

fn downscale(img: DynamicImage, max: u32) -> (u32, u32, Vec<u8>) {
    let (w, h) = (img.width(), img.height());
    let (nw, nh) = fit(w, h, max);
    let img = if (nw, nh) != (w, h) { img.resize_exact(nw, nh, image::imageops::FilterType::Triangle) } else { img };
    let rgba = img.to_rgba8();
    (rgba.width(), rgba.height(), rgba.into_raw())
}

pub fn load(path: &Path, o: &Opts) -> Preview {
    let ext = fm8_core::class::ext_of(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    if ext == "svg" || ext == "svgz" {
        return load_svg(path, o);
    }
    match decode(path, o) {
        Ok(d) => Preview::Image(d),
        Err(e) => Preview::Error(e),
    }
}

pub fn decode(path: &Path, o: &Opts) -> Result<ImageData, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let reader = image::ImageReader::new(BufReader::new(f)).with_guessed_format().map_err(|e| e.to_string())?;
    let fmt = reader.format();
    let fmt_name = fmt.map(|f| format!("{f:?}").to_uppercase()).unwrap_or_else(|| "?".into());
    // animated formats first
    if matches!(fmt, Some(ImageFormat::Gif) | Some(ImageFormat::WebP)) {
        let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let br = BufReader::new(f);
        let frames_res: Result<(Vec<image::Frame>, u32, u32), String> = match fmt {
            Some(ImageFormat::Gif) => {
                let d = image::codecs::gif::GifDecoder::new(br).map_err(|e| e.to_string())?;
                let (w, h) = d.dimensions();
                let mut v = vec![];
                for fr in d.into_frames().take(o.max_frames) { v.push(fr.map_err(|e| e.to_string())?); }
                Ok((v, w, h))
            }
            _ => {
                let d = image::codecs::webp::WebPDecoder::new(br).map_err(|e| e.to_string())?;
                let (w, h) = d.dimensions();
                if d.has_animation() {
                    let mut v = vec![];
                    for fr in d.into_frames().take(o.max_frames) { v.push(fr.map_err(|e| e.to_string())?); }
                    Ok((v, w, h))
                } else {
                    Ok((vec![], w, h))
                }
            }
        };
        let (frames, w, h) = frames_res?;
        if !frames.is_empty() {
            let out: Vec<Frame> = frames
                .into_iter()
                .map(|fr| {
                    let (n, d) = fr.delay().numer_denom_ms();
                    let delay = if d == 0 { 100 } else { (n / d).max(20) };
                    let (fw, fh, rgba) = downscale(DynamicImage::ImageRgba8(fr.into_buffer()), o.max_dim);
                    Frame { w: fw, h: fh, rgba, delay_ms: delay }
                })
                .collect();
            return Ok(ImageData { frames: out, orig_w: w, orig_h: h, format: fmt_name });
        }
    }
    let img = reader.decode().map_err(|e| e.to_string())?;
    let (ow, oh) = (img.width(), img.height());
    let (w, h, rgba) = downscale(img, o.max_dim);
    Ok(ImageData { frames: vec![Frame { w, h, rgba, delay_ms: 0 }], orig_w: ow, orig_h: oh, format: fmt_name })
}

fn load_svg(path: &Path, o: &Opts) -> Preview {
    let data = match std::fs::read(path) { Ok(d) => d, Err(e) => return Preview::Error(e.to_string()) };
    match render_svg(&data, o.max_dim) {
        Ok(d) => Preview::Image(d),
        Err(e) => Preview::Error(e),
    }
}

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
