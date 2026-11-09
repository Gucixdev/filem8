//! `.blend` files without running Blender: the embedded thumbnail (block `TEST`) plus
//! version and object/scene counts. std only, except that zstd-compressed files go
//! through the `zstd` tool from PATH. Handles the legacy 32/64-bit, either-endian block
//! headers and the 5.x "large" header (`BLENDER17-01v0501`).
use crate::{Frame, ImageData, Preview};
use fm8_codec::inflate;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;

const MAX_UNPACKED: usize = 256 << 20;

#[derive(Clone, Debug, Default)]
pub struct BlendInfo {
    pub version: String,
    pub objects: usize,
    pub scenes: usize,
    /// Embedded thumbnail, top row first.
    pub thumb: Option<fm8_codec::Image>,
}

fn rd<R: Read>(r: &mut R, n: usize) -> Option<Vec<u8>> { let mut v = vec![0u8; n]; r.read_exact(&mut v).ok()?; Some(v) }

fn int(b: &[u8], le: bool) -> i64 {
    match (b.len(), le) {
        (4, true) => i32::from_le_bytes(b.try_into().unwrap()) as i64,
        (4, false) => i32::from_be_bytes(b.try_into().unwrap()) as i64,
        (8, true) => i64::from_le_bytes(b.try_into().unwrap()),
        (8, false) => i64::from_be_bytes(b.try_into().unwrap()),
        _ => 0,
    }
}

/// "404" -> "4.4", "293" -> "2.93", "0501" -> "5.1".
fn version_text(v: &str) -> String {
    let n = |s: &str| s.parse::<u32>().unwrap_or(0);
    if v.len() == 4 { format!("{}.{}", n(&v[..2]), n(&v[2..])) } else if v.len() == 3 { format!("{}.{}", n(&v[..1]), n(&v[1..])) } else { v.to_string() }
}

/// Parse a decompressed stream.
pub fn parse<R: Read + Seek>(r: &mut R) -> Result<BlendInfo, String> {
    let h = rd(r, 12).ok_or("too short")?;
    if &h[..7] != b"BLENDER" { return Err("not a .blend file".into()); }
    let (large, le, version);
    if h[7].is_ascii_digit() {
        // BLENDER17-01v0501: 2-digit header size, '-', 2-digit format version, endian, 4-digit version
        let rest = rd(r, 5).ok_or("too short")?;
        let hh: Vec<u8> = h[7..].iter().chain(rest.iter()).copied().collect();
        large = true;
        le = hh[5] == b'v';
        version = String::from_utf8_lossy(&hh[6..10]).into_owned();
    } else {
        large = false;
        le = h[8] == b'v';
        version = String::from_utf8_lossy(&h[9..12]).into_owned();
    }
    let ptr = if h[7] == b'-' { 8 } else { 4 };
    let mut info = BlendInfo { version: version_text(&version), ..Default::default() };
    let hdr = if large { 32 } else { 16 + ptr };
    for _ in 0..4_000_000 {
        let Some(b) = rd(r, hdr) else { break };
        let code = [b[0], b[1], b[2], b[3]];
        let size = if large { int(&b[16..24], le) } else { int(&b[4..8], le) };
        if &code == b"ENDB" { break; }
        if size < 0 { return Err("corrupt block size".into()); }
        match &code {
            b"TEST" if info.thumb.is_none() && size >= 8 && size < (64 << 20) => {
                let d = rd(r, size as usize).ok_or("truncated thumbnail")?;
                let (w, hh) = (int(&d[0..4], le), int(&d[4..8], le));
                if w > 0 && hh > 0 && w < 4096 && hh < 4096 && d.len() as i64 >= 8 + w * hh * 4 {
                    let (w, hh) = (w as usize, hh as usize);
                    let mut rgba = Vec::with_capacity(w * hh * 4);
                    for y in (0..hh).rev() { rgba.extend_from_slice(&d[8 + y * w * 4..8 + (y + 1) * w * 4]); }
                    info.thumb = Some(fm8_codec::Image { w: w as u32, h: hh as u32, rgba });
                }
                continue;
            }
            b"OB\0\0" => info.objects += 1,
            b"SC\0\0" => info.scenes += 1,
            _ => {}
        }
        r.seek(SeekFrom::Current(size)).map_err(|e| e.to_string())?;
    }
    Ok(info)
}

fn zstd_unpack(path: &Path) -> Result<Vec<u8>, String> {
    let mut c = std::process::Command::new("zstd").args(["-dc", "-q"]).arg(path).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn()
        .map_err(|_| "zstd-compressed .blend: install `zstd` to read it".to_string())?;
    let mut v = vec![];
    c.stdout.take().ok_or("no pipe")?.take(MAX_UNPACKED as u64).read_to_end(&mut v).map_err(|e| e.to_string())?;
    let _ = c.kill();
    let _ = c.wait();
    if v.is_empty() { Err("zstd could not decompress this file".into()) } else { Ok(v) }
}

pub fn read_file(path: &Path) -> Result<BlendInfo, String> {
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut m = [0u8; 4];
    let n = f.read(&mut m).map_err(|e| e.to_string())?;
    f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    if n >= 2 && m[..2] == [0x1f, 0x8b] {
        let b = std::fs::read(path).map_err(|e| e.to_string())?;
        let raw = inflate::gunzip_prefix(&b, MAX_UNPACKED)?;
        parse(&mut Cursor::new(raw))
    } else if n == 4 && m == [0x28, 0xb5, 0x2f, 0xfd] {
        parse(&mut Cursor::new(zstd_unpack(path)?))
    } else {
        parse(&mut std::io::BufReader::new(f))
    }
}

pub fn describe(i: &BlendInfo) -> String { format!("Blender {}, {} object(s), {} scene(s)", i.version, i.objects, i.scenes) }

pub fn load(path: &Path) -> Preview {
    match read_file(path) {
        Ok(i) => match i.thumb.clone() {
            Some(t) => Preview::Image(ImageData { orig_w: t.w, orig_h: t.h, format: describe(&i), frames: vec![Frame { w: t.w, h: t.h, rgba: t.rgba, delay_ms: 0 }] }),
            None => Preview::Info(vec![describe(&i), "No embedded thumbnail".into()]),
        },
        Err(e) => Preview::Error(e),
    }
}
