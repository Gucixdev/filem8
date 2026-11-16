//! ZIP reader via the central directory: listing and stored/deflate extraction.
use crate::inflate;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub size: u64,
    pub csize: u64,
    pub method: u16,
    pub is_dir: bool,
    pub crc: u32,
    local: usize,
}

fn u16le(b: &[u8], o: usize) -> Option<u16> { Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?)) }
fn u32le(b: &[u8], o: usize) -> Option<u32> { Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?)) }
fn u64le(b: &[u8], o: usize) -> Option<u64> { Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?)) }

/// List entries (at most `cap`); bool = more exist.
pub fn list(b: &[u8], cap: usize) -> Result<(Vec<Entry>, bool), String> {
    let lo = b.len().saturating_sub(65557);
    let eocd = (lo..b.len().saturating_sub(21)).rev().find(|&i| &b[i..i + 4] == b"PK\x05\x06").ok_or("zip: no end-of-central-directory")?;
    let (mut n, mut cd) = (u16le(b, eocd + 10).ok_or("zip: short")? as u64, u32le(b, eocd + 16).ok_or("zip: short")? as u64);
    if (n == 0xFFFF || cd == 0xFFFF_FFFF) && eocd >= 20 && &b[eocd - 20..eocd - 16] == b"PK\x06\x07" {
        let z64 = usize::try_from(u64le(b, eocd - 12).ok_or("zip64")?).ok().filter(|&p| p < b.len()).ok_or("zip64")?;
        if b.get(z64..z64 + 4) == Some(b"PK\x06\x06") { n = u64le(b, z64 + 32).ok_or("zip64")?; cd = u64le(b, z64 + 48).ok_or("zip64")?; }
    }
    let mut p = usize::try_from(cd).ok().filter(|&p| p < b.len()).ok_or("zip: bad central directory")?;
    let mut v = vec![];
    for i in 0..n {
        if i as usize >= cap { return Ok((v, true)) }
        if b.get(p..p + 4) != Some(b"PK\x01\x02") { return Err("zip: bad central directory".into()) }
        let flags = u16le(b, p + 8).ok_or("zip: short")?;
        let (method, crc) = (u16le(b, p + 10).ok_or("zip: short")?, u32le(b, p + 16).ok_or("zip: short")?);
        let (mut csize, mut size) = (u32le(b, p + 20).ok_or("zip: short")? as u64, u32le(b, p + 24).ok_or("zip: short")? as u64);
        let (nl, el, cl) = (u16le(b, p + 28).ok_or("zip: short")? as usize, u16le(b, p + 30).ok_or("zip: short")? as usize, u16le(b, p + 32).ok_or("zip: short")? as usize);
        let mut local = u32le(b, p + 42).ok_or("zip: short")? as u64;
        let raw = b.get(p + 46..p + 46 + nl).ok_or("zip: short name")?;
        let name = if flags & 0x800 != 0 { String::from_utf8_lossy(raw).into_owned() } else { raw.iter().map(|&c| c as char).collect::<String>() };
        // zip64 extra
        let mut q = p + 46 + nl;
        let end = q + el;
        while q + 4 <= end {
            let (id, len) = (u16le(b, q).unwrap_or(0), u16le(b, q + 2).unwrap_or(0) as usize);
            if id == 1 {
                let mut o = q + 4;
                if size == 0xFFFF_FFFF { size = u64le(b, o).unwrap_or(size); o += 8 }
                if csize == 0xFFFF_FFFF { csize = u64le(b, o).unwrap_or(csize); o += 8 }
                if local == 0xFFFF_FFFF { local = u64le(b, o).unwrap_or(local) }
            }
            q += 4 + len;
        }
        v.push(Entry { is_dir: name.ends_with('/'), name, size, csize, method, crc, local: local as usize });
        p = end + cl;
    }
    Ok((v, false))
}

/// Extract one member (stored or deflate), checked against its CRC-32.
pub fn read(b: &[u8], e: &Entry, limit: usize) -> Result<Vec<u8>, String> {
    let l = e.local;
    if l >= b.len() || e.csize > b.len() as u64 { return Err("zip: truncated member".into()) }
    if b.get(l..l + 4) != Some(b"PK\x03\x04") { return Err("zip: bad local header".into()) }
    let start = l + 30 + u16le(b, l + 26).ok_or("zip: short")? as usize + u16le(b, l + 28).ok_or("zip: short")? as usize;
    let raw = b.get(start..start + e.csize as usize).ok_or("zip: truncated member")?;
    let out = match e.method { 0 => raw.to_vec(), 8 => inflate::inflate(raw, limit.min(e.size as usize + 1))?, m => return Err(format!("zip: method {m} unsupported")) };
    if inflate::crc32(&[&out]) != e.crc { return Err("zip: crc mismatch".into()) }
    Ok(out)
}
