//! 2D game-asset helpers: Tiled maps and atlas JSON (Aseprite / TexturePacker).
use crate::{Frame, ImageData, Opts, Preview};
use serde_json::Value;
use std::path::Path;

fn read_json(path: &Path) -> Option<Value> {
    let md = std::fs::metadata(path).ok()?;
    if md.len() > 16 * 1024 * 1024 { return None; }
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

pub fn looks_like_sheet(path: &Path) -> bool {
    read_json(path).map(|v| is_tiled(&v) || atlas_frames(&v).is_some()).unwrap_or(false)
}

fn is_tiled(v: &Value) -> bool {
    v.get("tilewidth").is_some() && v.get("layers").map(|l| l.is_array()).unwrap_or(false)
}

/// Frame rects from Aseprite/TexturePacker JSON (array or hash form).
pub fn atlas_frames(v: &Value) -> Option<Vec<(String, [i64; 4])>> {
    let fr = v.get("frames")?;
    let rect = |o: &Value| -> Option<[i64; 4]> {
        let f = o.get("frame")?;
        Some([f.get("x")?.as_i64()?, f.get("y")?.as_i64()?, f.get("w")?.as_i64()?, f.get("h")?.as_i64()?])
    };
    let mut out = vec![];
    match fr {
        Value::Array(a) => for (i, o) in a.iter().enumerate() { out.push((o.get("filename").and_then(|s| s.as_str()).map(String::from).unwrap_or_else(|| i.to_string()), rect(o)?)) },
        Value::Object(m) => for (k, o) in m { out.push((k.clone(), rect(o)?)) },
        _ => return None,
    }
    Some(out)
}

pub fn load(path: &Path, o: &Opts) -> Preview {
    let Some(v) = read_json(path) else { return Preview::Error("not valid JSON".into()) };
    if is_tiled(&v) { return tiled(&v) }
    if let Some(frames) = atlas_frames(&v) {
        let img_name = v.pointer("/meta/image").and_then(|s| s.as_str());
        let Some(img_name) = img_name else { return Preview::Info(vec![format!("Atlas with {} frames", frames.len()), "meta.image missing".into()]) };
        let ip = path.parent().unwrap_or(Path::new(".")).join(img_name);
        let mut d = match crate::image_load::decode(&ip, &Opts { max_dim: 100_000, ..o.clone() }) { Ok(d) => d, Err(e) => return Preview::Error(format!("{}: {e}", ip.display())) };
        let fr = &mut d.frames[0];
        for (_, [x, y, w, h]) in &frames { outline(fr, *x, *y, *w, *h, [255, 80, 80, 255]); }
        // now downscale for display
        let (w, h) = (fr.w, fr.h);
        if w.max(h) > o.max_dim {
            let img = image::RgbaImage::from_raw(w, h, std::mem::take(&mut fr.rgba)).unwrap();
            let s = o.max_dim as f32 / w.max(h) as f32;
            let r = image::imageops::resize(&img, (w as f32 * s) as u32, (h as f32 * s) as u32, image::imageops::FilterType::Nearest);
            *fr = Frame { w: r.width(), h: r.height(), rgba: r.into_raw(), delay_ms: 0 };
        }
        d.format = format!("Atlas, {} frames", frames.len());
        return Preview::Image(d);
    }
    Preview::Error("unrecognised JSON sheet".into())
}

fn outline(f: &mut Frame, x: i64, y: i64, w: i64, h: i64, c: [u8; 4]) {
    let mut put = |px: i64, py: i64| {
        if px >= 0 && py >= 0 && (px as u32) < f.w && (py as u32) < f.h {
            let i = ((py as u32 * f.w + px as u32) * 4) as usize;
            f.rgba[i..i + 4].copy_from_slice(&c);
        }
    };
    for i in 0..w { put(x + i, y); put(x + i, y + h - 1); }
    for j in 0..h { put(x, y + j); put(x + w - 1, y + j); }
}

/// Tiled map: draw each layer's tile ids as hash-coloured cells (first layer
/// with data wins per cell, later layers draw over non-empty ids).
fn tiled(v: &Value) -> Preview {
    let (mw, mh) = (v.get("width").and_then(|x| x.as_u64()).unwrap_or(0) as u32, v.get("height").and_then(|x| x.as_u64()).unwrap_or(0) as u32);
    let (tw, th) = (v.get("tilewidth").and_then(|x| x.as_u64()).unwrap_or(0), v.get("tileheight").and_then(|x| x.as_u64()).unwrap_or(0));
    if mw == 0 || mh == 0 || mw * mh > 4_000_000 { return Preview::Error("bad map size".into()) }
    let cell = (800 / mw.max(mh)).clamp(2, 24);
    let (iw, ih) = (mw * cell, mh * cell);
    let mut rgba = vec![0u8; (iw * ih * 4) as usize];
    for px in rgba.chunks_exact_mut(4) { px.copy_from_slice(&[24, 26, 32, 255]); }
    let layers = v["layers"].as_array().unwrap();
    let mut tile_layers = 0;
    for l in layers {
        if l.get("type").and_then(|t| t.as_str()) != Some("tilelayer") { continue }
        let Some(data) = l.get("data").and_then(|d| d.as_array()) else { continue };
        tile_layers += 1;
        for (i, id) in data.iter().enumerate() {
            let id = (id.as_u64().unwrap_or(0) & 0x0FFF_FFFF) as u32; // strip flip flags
            if id == 0 || i as u32 >= mw * mh { continue }
            let (cx, cy) = (i as u32 % mw, i as u32 / mw);
            let h = id.wrapping_mul(2654435761);
            let col = [(h >> 8) as u8 | 0x40, (h >> 16) as u8 | 0x40, (h >> 24) as u8 | 0x40, 255];
            for y in 0..cell { for x in 0..cell {
                let edge = x == cell - 1 || y == cell - 1;
                let o = (((cy * cell + y) * iw + cx * cell + x) * 4) as usize;
                rgba[o..o + 4].copy_from_slice(&if edge && cell > 5 { [0, 0, 0, 255] } else { col });
            } }
        }
    }
    Preview::Image(ImageData { frames: vec![Frame { w: iw, h: ih, rgba, delay_ms: 0 }], orig_w: mw * tw as u32, orig_h: mh * th as u32, format: format!("Tiled map {mw}x{mh}, tile {tw}x{th}, {tile_layers} tile layer(s)") })
}
