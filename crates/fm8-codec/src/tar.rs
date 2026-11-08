//! tar (ustar, GNU long names, pax path/size) listing and member extraction.

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub mtime: u64,
    pub mode: u32,
    /// Offset of the member data within the (decompressed) buffer.
    pub data: usize,
}

fn oct(b: &[u8]) -> u64 {
    if b.first().map(|&x| x & 0x80 != 0).unwrap_or(false) { return b[1..].iter().fold(0u64, |a, &x| (a << 8) | x as u64) } // base-256
    b.iter().take_while(|&&c| c != 0 && c != b' ').fold(0, |a, &c| (a << 3) | (c.wrapping_sub(b'0') & 7) as u64)
}

fn cstr(b: &[u8]) -> String { String::from_utf8_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]).into_owned() }

/// Parse up to `cap` entries from decompressed tar bytes. Second value: more entries exist.
pub fn list(buf: &[u8], cap: usize) -> (Vec<Entry>, bool) {
    let mut v = vec![];
    let (mut p, mut long, mut pax_path, mut pax_size) = (0usize, None::<String>, None::<String>, None::<u64>);
    while p + 512 <= buf.len() {
        let h = &buf[p..p + 512];
        if h.iter().all(|&x| x == 0) { break }
        if &h[257..262] != b"ustar" && h[148..156].iter().all(|&x| x == 0) { break }
        let mut size = oct(&h[124..136]);
        let ty = h[156];
        let data = p + 512;
        let blocks = (size as usize).div_ceil(512) * 512;
        match ty {
            b'L' => { long = Some(cstr(buf.get(data..data + size as usize).unwrap_or(&[]))) }
            b'x' => {
                for rec in String::from_utf8_lossy(buf.get(data..data + size as usize).unwrap_or(&[])).split('\n') {
                    if let Some((_, kv)) = rec.split_once(' ') {
                        if let Some(x) = kv.strip_prefix("path=") { pax_path = Some(x.to_string()) }
                        if let Some(x) = kv.strip_prefix("size=") { pax_size = x.parse().ok() }
                    }
                }
            }
            b'g' | b'K' => {}
            _ => {
                if v.len() >= cap { return (v, true) }
                let mut name = cstr(&h[0..100]);
                if &h[257..262] == b"ustar" { let pre = cstr(&h[345..500]); if !pre.is_empty() { name = format!("{pre}/{name}") } }
                if let Some(l) = long.take() { name = l }
                if let Some(x) = pax_path.take() { name = x }
                if let Some(x) = pax_size.take() { size = x }
                let is_dir = ty == b'5' || name.ends_with('/');
                v.push(Entry { name, size, is_dir, mtime: oct(&h[136..148]), mode: oct(&h[100..108]) as u32, data });
            }
        }
        let skip = blocks;
        p = data + skip;
    }
    (v, false)
}

pub fn read<'a>(buf: &'a [u8], e: &Entry) -> Option<&'a [u8]> { buf.get(e.data..e.data + e.size as usize) }
