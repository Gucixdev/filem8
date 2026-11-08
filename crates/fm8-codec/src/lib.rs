//! fm8-codec: std-only image decoders (and a PNG encoder), one file per format.
//! Every decoder returns straight (non-premultiplied) RGBA8.

pub mod bmp;
pub mod gif;
pub mod hdr;
pub mod ico;
pub mod inflate;
pub mod jpeg;
pub mod pnm;
pub mod png;
pub mod qoi;
pub mod resize;
pub mod tga;
pub mod tiff;

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn new(w: u32, h: u32) -> Image { Image { w, h, rgba: vec![0; w as usize * h as usize * 4] } }
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub img: Image,
    pub delay_ms: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format { Png, Jpeg, Gif, Bmp, Qoi, Tga, Pnm, Tiff, Ico, Hdr }

impl Format {
    pub fn name(self) -> &'static str {
        match self { Format::Png => "PNG", Format::Jpeg => "JPEG", Format::Gif => "GIF", Format::Bmp => "BMP", Format::Qoi => "QOI", Format::Tga => "TGA", Format::Pnm => "PNM", Format::Tiff => "TIFF", Format::Ico => "ICO", Format::Hdr => "HDR" }
    }
}

/// Identify by magic bytes; `ext` (lowercase, no dot) only breaks ties for headerless formats (TGA).
pub fn sniff(b: &[u8], ext: &str) -> Option<Format> {
    let s = |m: &[u8]| b.starts_with(m);
    Some(if s(b"\x89PNG\r\n\x1a\n") { Format::Png }
    else if s(&[0xFF, 0xD8, 0xFF]) { Format::Jpeg }
    else if s(b"GIF87a") || s(b"GIF89a") { Format::Gif }
    else if s(b"BM") { Format::Bmp }
    else if s(b"qoif") { Format::Qoi }
    else if s(b"II*\0") || s(b"MM\0*") { Format::Tiff }
    else if s(&[0, 0, 1, 0]) && b.len() > 6 { Format::Ico }
    else if s(b"#?RADIANCE") || s(b"#?RGBE") { Format::Hdr }
    else if b.len() > 2 && b[0] == b'P' && (b'1'..=b'7').contains(&b[1]) && (b[2] as char).is_ascii_whitespace() { Format::Pnm }
    else if ext == "tga" { Format::Tga }
    else { return None })
}

/// Decode the first (or only) frame.
pub fn decode(b: &[u8], ext: &str) -> Result<(Format, Image), String> {
    let f = sniff(b, ext).ok_or("unknown image format")?;
    let img = match f {
        Format::Png => png::decode(b)?,
        Format::Jpeg => jpeg::decode(b)?,
        Format::Gif => gif::decode(b, 1)?.into_iter().next().ok_or("empty gif")?.img,
        Format::Bmp => bmp::decode(b)?,
        Format::Qoi => qoi::decode(b)?,
        Format::Tga => tga::decode(b)?,
        Format::Pnm => pnm::decode(b)?,
        Format::Tiff => tiff::decode(b)?,
        Format::Ico => ico::decode(b)?,
        Format::Hdr => hdr::decode(b)?,
    };
    Ok((f, img))
}

/// Header-only dimensions for the formats where that is cheap; otherwise None.
pub fn dimensions(b: &[u8], ext: &str) -> Option<(Format, u32, u32)> {
    let f = sniff(b, ext)?;
    let be = |o: usize| Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?));
    let le = |o: usize| Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?));
    let (w, h) = match f {
        Format::Png => (be(16)?, be(20)?),
        Format::Gif => (u16::from_le_bytes([*b.get(6)?, *b.get(7)?]) as u32, u16::from_le_bytes([*b.get(8)?, *b.get(9)?]) as u32),
        Format::Bmp => (le(18)?, (le(22)? as i32).unsigned_abs()),
        Format::Qoi => (be(4)?, be(8)?),
        _ => { let (_, i) = decode(b, ext).ok()?; (i.w, i.h) }
    };
    Some((f, w, h))
}
