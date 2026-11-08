//! 3D model preview: loaders (OBJ, STL, glTF/GLB) and a small CPU z-buffer
//! renderer that draws a flat-shaded, slowly turning model into an RGBA image.
//! (A wgpu renderer is planned; the CPU one is deterministic and works with no GPU.)

use crate::Preview;
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    /// Base colour (glTF material) or neutral grey.
    pub color: [f32; 3],
    pub source: String,
}

impl Mesh {
    pub fn triangles(&self) -> usize { self.indices.len() / 3 }
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for p in &self.positions { for k in 0..3 { lo[k] = lo[k].min(p[k]); hi[k] = hi[k].max(p[k]); } }
        (lo, hi)
    }
}

pub fn load(path: &Path) -> Preview {
    match load_mesh(path) {
        Ok(m) if m.triangles() > 0 => Preview::Model(m),
        Ok(_) => Preview::Info(vec!["3D model has no triangles".into()]),
        Err(e) => Preview::Error(e),
    }
}

pub fn load_mesh(path: &Path) -> Result<Mesh, String> {
    let ext = fm8_core::class::ext_of(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let mut m = match ext.as_str() {
        "obj" => parse_obj(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)?,
        "stl" => parse_stl(&std::fs::read(path).map_err(|e| e.to_string())?)?,
        "gltf" | "glb" => crate::gltf::parse(path)?,
        _ => return Err(format!("no loader for .{ext} (supported: obj, stl, gltf, glb)")),
    };
    if m.color == [0.0; 3] { m.color = [0.72, 0.74, 0.8]; }
    m.source = ext.to_uppercase();
    Ok(m)
}

pub fn parse_obj(s: &str) -> Result<Mesh, String> {
    let mut m = Mesh::default();
    let mut v: Vec<[f32; 3]> = vec![];
    let mut tris: Vec<[u32; 3]> = vec![];
    for l in s.lines() {
        let mut it = l.split_whitespace();
        match it.next() {
            Some("v") => {
                let c: Vec<f32> = it.take(3).filter_map(|x| x.parse().ok()).collect();
                if c.len() == 3 { v.push([c[0], c[1], c[2]]) }
            }
            Some("f") => {
                let idx: Vec<u32> = it
                    .filter_map(|t| t.split('/').next()?.parse::<i64>().ok())
                    .map(|i| if i < 0 { (v.len() as i64 + i) as u32 } else { (i - 1) as u32 })
                    .collect();
                for k in 1..idx.len().saturating_sub(1) { tris.push([idx[0], idx[k], idx[k + 1]]); }
            }
            _ => {}
        }
    }
    if tris.iter().flatten().any(|&i| i as usize >= v.len()) { return Err("OBJ face index out of range".into()); }
    m.positions = v;
    m.indices = tris.into_iter().flatten().collect();
    Ok(m)
}

pub fn parse_stl(b: &[u8]) -> Result<Mesh, String> {
    let mut m = Mesh::default();
    let binary_len = if b.len() >= 84 { 84 + 50 * u32::from_le_bytes([b[80], b[81], b[82], b[83]]) as usize } else { usize::MAX };
    if b.len() == binary_len {
        let n = (b.len() - 84) / 50;
        let f = |o: usize| f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        for t in 0..n {
            let base = 84 + t * 50 + 12;
            for k in 0..3 {
                let o = base + k * 12;
                m.indices.push(m.positions.len() as u32);
                m.positions.push([f(o), f(o + 4), f(o + 8)]);
            }
        }
    } else {
        let s = std::str::from_utf8(b).map_err(|_| "not a valid STL")?;
        for l in s.lines() {
            let mut it = l.split_whitespace();
            if it.next() == Some("vertex") {
                let c: Vec<f32> = it.filter_map(|x| x.parse().ok()).collect();
                if c.len() == 3 {
                    m.indices.push(m.positions.len() as u32);
                    m.positions.push([c[0], c[1], c[2]]);
                }
            }
        }
        m.indices.truncate(m.indices.len() / 3 * 3);
    }
    Ok(m)
}

/// Render the mesh turned by `yaw` radians into a `size`x`size` RGBA image
/// (transparent background, 2x supersampled). Triangle count is capped.
pub fn render(m: &Mesh, yaw: f32, size: u32) -> Vec<u8> {
    let ss = 2u32;
    let n = size * ss;
    let mut depth = vec![f32::MAX; (n * n) as usize];
    let mut img = vec![0f32; (n * n * 4) as usize];
    let (lo, hi) = m.bounds();
    let c = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let radius = (0..3).map(|k| (hi[k] - lo[k]) / 2.0).fold(0.0f32, |a, b| a.hypot(b)).max(1e-6);
    let (sy, cy) = yaw.sin_cos();
    let pitch = 0.35f32;
    let (sp, cp) = pitch.sin_cos();
    let tf = |p: [f32; 3]| -> [f32; 3] {
        let (x, y, z) = ((p[0] - c[0]) / radius, (p[1] - c[1]) / radius, (p[2] - c[2]) / radius);
        let (x, z) = (x * cy + z * sy, -x * sy + z * cy);
        let (y, z) = (y * cp - z * sp, y * sp + z * cp);
        [x, y, z]
    };
    let cam = 3.2f32;
    let scale = n as f32 * 0.5 * cam / 1.25;
    let proj = |p: [f32; 3]| -> (f32, f32, f32) {
        let d = cam - p[2];
        (n as f32 / 2.0 + p[0] / d * scale, n as f32 / 2.0 - p[1] / d * scale, d)
    };
    let step = (m.triangles() / 300_000).max(1);
    let light = { let l = [0.4f32, 0.6, 0.7]; let k = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt(); [l[0] / k, l[1] / k, l[2] / k] };
    for t in (0..m.triangles()).step_by(step) {
        let v: Vec<[f32; 3]> = (0..3).map(|k| tf(m.positions[m.indices[t * 3 + k] as usize])).collect();
        let e1 = [v[1][0] - v[0][0], v[1][1] - v[0][1], v[1][2] - v[0][2]];
        let e2 = [v[2][0] - v[0][0], v[2][1] - v[0][1], v[2][2] - v[0][2]];
        let mut nr = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        let nl = (nr[0] * nr[0] + nr[1] * nr[1] + nr[2] * nr[2]).sqrt();
        if nl < 1e-12 { continue; }
        for x in &mut nr { *x /= nl; }
        let lam = (nr[0] * light[0] + nr[1] * light[1] + nr[2] * light[2]).abs();
        let shade = 0.25 + 0.75 * lam;
        let p: Vec<(f32, f32, f32)> = v.iter().map(|&q| proj(q)).collect();
        let (minx, maxx) = (p.iter().map(|q| q.0).fold(f32::MAX, f32::min).floor().max(0.0), p.iter().map(|q| q.0).fold(f32::MIN, f32::max).ceil().min(n as f32 - 1.0));
        let (miny, maxy) = (p.iter().map(|q| q.1).fold(f32::MAX, f32::min).floor().max(0.0), p.iter().map(|q| q.1).fold(f32::MIN, f32::max).ceil().min(n as f32 - 1.0));
        let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
        if area.abs() < 1e-9 { continue; }
        for y in miny as i32..=maxy as i32 {
            for x in minx as i32..=maxx as i32 {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((p[1].0 - fx) * (p[2].1 - fy) - (p[2].0 - fx) * (p[1].1 - fy)) / area;
                let w1 = ((p[2].0 - fx) * (p[0].1 - fy) - (p[0].0 - fx) * (p[2].1 - fy)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 { continue; }
                let z = w0 * p[0].2 + w1 * p[1].2 + w2 * p[2].2;
                let i = (y as u32 * n + x as u32) as usize;
                if z < depth[i] {
                    depth[i] = z;
                    for k in 0..3 { img[i * 4 + k] = m.color[k] * shade; }
                    img[i * 4 + 3] = 1.0;
                }
            }
        }
    }
    // box downsample
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let (mut a, mut rgb) = (0.0, [0.0f32; 3]);
            for dy in 0..ss { for dx in 0..ss {
                let i = (((y * ss + dy) * n + x * ss + dx) * 4) as usize;
                a += img[i + 3];
                for k in 0..3 { rgb[k] += img[i + k]; }
            } }
            let o = ((y * size + x) * 4) as usize;
            if a > 0.0 { for k in 0..3 { out[o + k] = (rgb[k] / a * 255.0).clamp(0.0, 255.0) as u8; } }
            out[o + 3] = (a / (ss * ss) as f32 * 255.0) as u8;
        }
    }
    out
}
