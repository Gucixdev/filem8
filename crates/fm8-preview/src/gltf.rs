//! Own glTF 2.0 / GLB triangle loader (std only, JSON via crate::json):
//! meshes in the default scene with node transforms (matrix or TRS), embedded
//! base64 / external / GLB-chunk buffers, any index width, byte strides, and
//! the base colour factor. No textures, skins, morph targets or sparse
//! accessors: this is for thumbnails and the preview pane.
use crate::json::{self, Value};
use crate::model::Mesh;
use std::path::Path;

type M4 = [[f32; 4]; 4];
const ID: M4 = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

fn mul(a: M4, b: M4) -> M4 {
    let mut o = [[0.0; 4]; 4];
    for c in 0..4 { for r in 0..4 { o[c][r] = (0..4).map(|k| a[k][r] * b[c][k]).sum(); } }
    o
}
fn xform(m: &M4, p: [f32; 3]) -> [f32; 3] {
    let f = |r: usize| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r];
    [f(0), f(1), f(2)]
}

pub fn base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes() {
        let v = match c { b'A'..=b'Z' => c - b'A', b'a'..=b'z' => c - b'a' + 26, b'0'..=b'9' => c - b'0' + 52, b'+' | b'-' => 62, b'/' | b'_' => 63, b'=' => break, b'\n' | b'\r' | b' ' => continue, _ => return None } as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 { bits -= 8; out.push((acc >> bits) as u8); acc &= (1 << bits) - 1; }
    }
    Some(out)
}

fn num(v: Option<&Value>, d: f64) -> f64 { v.and_then(|v| v.as_f64()).unwrap_or(d) }

fn node_matrix(n: &Value) -> M4 {
    if let Some(a) = n.get("matrix").and_then(|m| m.as_array()).filter(|a| a.len() == 16) {
        let mut m = ID;
        for i in 0..16 { m[i / 4][i % 4] = a[i].as_f64().unwrap_or(0.0) as f32; }
        return m;
    }
    let vec = |k: &str, d: &[f32]| -> Vec<f32> {
        match n.get(k).and_then(|v| v.as_array()) { Some(a) if a.len() == d.len() => a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect(), _ => d.to_vec() }
    };
    let (t, r, s) = (vec("translation", &[0.0; 3]), vec("rotation", &[0.0, 0.0, 0.0, 1.0]), vec("scale", &[1.0; 3]));
    let (x, y, z, w) = (r[0], r[1], r[2], r[3]);
    let rot = [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + z * w), 2.0 * (x * z - y * w)],
        [2.0 * (x * y - z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + x * w)],
        [2.0 * (x * z + y * w), 2.0 * (y * z - x * w), 1.0 - 2.0 * (x * x + y * y)],
    ];
    let mut m = ID;
    for c in 0..3 { for r in 0..3 { m[c][r] = rot[c][r] * s[c]; } m[3][c] = t[c]; }
    m
}

struct Doc<'a> { j: &'a Value, bufs: Vec<Vec<u8>> }

impl Doc<'_> {
    fn list(&self, k: &str) -> &[Value] { self.j.get(k).and_then(|v| v.as_array()).map(|v| v.as_slice()).unwrap_or(&[]) }

    /// Read accessor `ai` as rows of `comps` f64 values.
    fn read(&self, ai: usize, comps: usize) -> Option<Vec<f64>> {
        let acc = self.list("accessors").get(ai)?;
        let count = acc.get("count")?.as_u64()? as usize;
        let ct = acc.get("componentType")?.as_u64()?;
        let size = match ct { 5120 | 5121 => 1, 5122 | 5123 => 2, 5125 | 5126 => 4, _ => return None };
        let bv = self.list("bufferViews").get(acc.get("bufferView")?.as_u64()? as usize)?;
        let buf = self.bufs.get(bv.get("buffer")?.as_u64()? as usize)?;
        let start = num(bv.get("byteOffset"), 0.0) as usize + num(acc.get("byteOffset"), 0.0) as usize;
        let stride = match num(bv.get("byteStride"), 0.0) as usize { 0 => size * comps, s => s };
        // every row needs its bytes in the buffer: a count the buffer cannot hold is a broken file, not a reason to allocate
        if count.checked_mul(comps * size)? > buf.len() { return None }
        let mut out = Vec::with_capacity(count * comps);
        for i in 0..count {
            for c in 0..comps {
                let o = start + i * stride + c * size;
                let b = buf.get(o..o + size)?;
                out.push(match ct {
                    5120 => b[0] as i8 as f64, 5121 => b[0] as f64,
                    5122 => i16::from_le_bytes([b[0], b[1]]) as f64, 5123 => u16::from_le_bytes([b[0], b[1]]) as f64,
                    5125 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
                    _ => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
                });
            }
        }
        Some(out)
    }

    /// `seen` marks visited nodes: a glTF scene is a tree, so a node reached twice is a cycle or a shared child.
    fn walk(&self, ni: usize, parent: M4, depth: u32, m: &mut Mesh, seen: &mut Vec<bool>) {
        let Some(n) = self.list("nodes").get(ni) else { return };
        if depth > 64 || m.positions.len() > 4_000_000 || std::mem::replace(&mut seen[ni], true) { return; }
        let w = mul(parent, node_matrix(n));
        if let Some(mesh) = n.get("mesh").and_then(|v| v.as_u64()).and_then(|i| self.list("meshes").get(i as usize)) {
            for p in mesh.get("primitives").and_then(|v| v.as_array()).map(|v| v.as_slice()).unwrap_or(&[]) {
                if num(p.get("mode"), 4.0) as u32 != 4 { continue; }
                let Some(pa) = p.pointer("/attributes/POSITION").and_then(|v| v.as_u64()) else { continue };
                let Some(pos) = self.read(pa as usize, 3) else { continue };
                let base = m.positions.len() as u32;
                m.positions.extend(pos.chunks(3).map(|c| xform(&w, [c[0] as f32, c[1] as f32, c[2] as f32])));
                let nv = m.positions.len() as u32 - base;
                if nv == 0 { continue; }
                match p.get("indices").and_then(|v| v.as_u64()).and_then(|i| self.read(i as usize, 1)) {
                    Some(ix) => m.indices.extend(ix.iter().take(ix.len() / 3 * 3).map(|&i| (i as u32).min(nv.saturating_sub(1)) + base)),
                    None => m.indices.extend(base..base + nv / 3 * 3),
                }
                if m.color == [0.0; 3] {
                    if let Some(c) = p.get("material").and_then(|v| v.as_u64()).and_then(|i| self.list("materials").get(i as usize)).and_then(|mt| mt.pointer("/pbrMetallicRoughness/baseColorFactor")).and_then(|v| v.as_array()) {
                        if c.len() >= 3 { m.color = [c[0].as_f64().unwrap_or(0.7) as f32, c[1].as_f64().unwrap_or(0.7) as f32, c[2].as_f64().unwrap_or(0.7) as f32]; }
                    }
                }
            }
        }
        for c in n.get("children").and_then(|v| v.as_array()).map(|v| v.as_slice()).unwrap_or(&[]) { if let Some(ci) = c.as_u64() { self.walk(ci as usize, w, depth + 1, m, seen); } }
    }
}

pub fn parse(path: &Path) -> Result<Mesh, String> {
    let raw = std::fs::read(path).map_err(|e| e.to_string())?;
    let (jtxt, mut glb_bin): (Vec<u8>, Option<Vec<u8>>) = if raw.starts_with(b"glTF") {
        let (mut json, mut bin, mut i) = (None, None, 12);
        while i + 8 <= raw.len() {
            let n = u32::from_le_bytes([raw[i], raw[i + 1], raw[i + 2], raw[i + 3]]) as usize;
            let ty = &raw[i + 4..i + 8];
            let body = raw.get(i + 8..i + 8 + n).ok_or("glb: truncated chunk")?;
            if ty == b"JSON" { json = Some(body.to_vec()); } else if ty == b"BIN\0" && bin.is_none() { bin = Some(body.to_vec()); }
            i += 8 + n + (4 - n % 4) % 4;
        }
        (json.ok_or("glb: no JSON chunk")?, bin)
    } else { (raw, None) };
    let j = json::parse(&jtxt).ok_or("glTF: invalid JSON")?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut bufs = vec![];
    for b in j.get("buffers").and_then(|v| v.as_array()).map(|v| v.as_slice()).unwrap_or(&[]) {
        bufs.push(match b.get("uri").and_then(|u| u.as_str()) {
            Some(u) if u.starts_with("data:") => base64(u.split_once(',').ok_or("bad data uri")?.1).ok_or("bad base64")?,
            Some(u) => {
                // keep external buffers inside the model's folder
                if u.starts_with('/') || u.split('/').any(|s| s == "..") { return Err("glTF: external buffer outside folder".into()); }
                std::fs::read(dir.join(u.replace("%20", " "))).map_err(|e| format!("{u}: {e}"))?
            }
            None => glb_bin.take().ok_or("glTF: buffer without data")?,
        });
    }
    let d = Doc { j: &j, bufs };
    let mut m = Mesh::default();
    let mut seen = vec![false; d.list("nodes").len()];
    let scenes = d.list("scenes");
    let si = j.get("scene").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    match scenes.get(si) {
        Some(sc) => for n in sc.get("nodes").and_then(|v| v.as_array()).map(|v| v.as_slice()).unwrap_or(&[]) { if let Some(i) = n.as_u64() { d.walk(i as usize, ID, 0, &mut m, &mut seen); } },
        None => for i in 0..d.list("nodes").len() { d.walk(i, ID, 0, &mut m, &mut seen); },
    }
    Ok(m)
}
