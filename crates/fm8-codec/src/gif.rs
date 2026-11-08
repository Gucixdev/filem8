//! GIF87a/89a with LZW, interlace, transparency, disposal; composited full frames.
use crate::{Frame, Image};

fn lzw(data: &[u8], min: u8, n: usize) -> Vec<u8> {
    let clear = 1usize << min;
    let eoi = clear + 1;
    let mut table: Vec<(u16, u8)> = Vec::with_capacity(4096);
    let reset = |t: &mut Vec<(u16, u8)>| { t.clear(); for i in 0..clear + 2 { t.push((u16::MAX, i as u8)) } };
    reset(&mut table);
    let mut out = Vec::with_capacity(n);
    let (mut size, mut acc, mut nb, mut prev) = (min as usize + 1, 0u32, 0usize, None::<usize>);
    let mut buf = vec![];
    for &byte in data {
        acc |= (byte as u32) << nb;
        nb += 8;
        while nb >= size {
            let code = (acc & ((1 << size) - 1)) as usize;
            acc >>= size;
            nb -= size;
            if code == clear { reset(&mut table); size = min as usize + 1; prev = None; continue }
            if code == eoi { return out }
            let first;
            if code < table.len() {
                buf.clear();
                let mut c = code;
                loop { buf.push(table[c].1); if table[c].0 == u16::MAX { break } c = table[c].0 as usize }
                buf.reverse();
                first = buf[0];
            } else if let Some(p) = prev {
                buf.clear();
                let mut c = p;
                loop { buf.push(table[c].1); if table[c].0 == u16::MAX { break } c = table[c].0 as usize }
                buf.reverse();
                first = buf[0];
                buf.push(first);
            } else { return out }
            out.extend_from_slice(&buf);
            if let Some(p) = prev { if table.len() < 4096 { table.push((p as u16, first)) } }
            prev = Some(code.min(table.len() - 1));
            if table.len() == (1 << size) && size < 12 { size += 1 }
            if out.len() >= n { return out }
        }
    }
    out
}

/// Decode up to `max_frames` composited frames.
pub fn decode(b: &[u8], max_frames: usize) -> Result<Vec<Frame>, String> {
    if b.len() < 13 || !b.starts_with(b"GIF8") { return Err("not gif".into()) }
    let (w, h) = (u16::from_le_bytes([b[6], b[7]]) as usize, u16::from_le_bytes([b[8], b[9]]) as usize);
    if w == 0 || h == 0 { return Err("gif: bad size".into()) }
    let flags = b[10];
    let bg_index = b[11] as usize;
    let mut p = 13;
    let mut gct = vec![];
    if flags & 0x80 != 0 { let n = 3 << ((flags & 7) + 1); gct = b.get(p..p + n).ok_or("gif: short")?.to_vec(); p += n }
    let mut canvas = Image::new(w as u32, h as u32);
    let _ = bg_index;
    let mut frames = vec![];
    let (mut delay, mut transp, mut disposal) = (10u32, None::<u8>, 0u8);
    while p < b.len() && frames.len() < max_frames {
        match b[p] {
            0x21 => {
                let label = *b.get(p + 1).ok_or("gif: short")?;
                p += 2;
                if label == 0xF9 && b.get(p) == Some(&4) {
                    let g = b.get(p + 1..p + 5).ok_or("gif: short")?;
                    disposal = (g[0] >> 2) & 7;
                    delay = u16::from_le_bytes([g[1], g[2]]) as u32;
                    transp = if g[0] & 1 != 0 { Some(g[3]) } else { None };
                }
                while let Some(&n) = b.get(p) { p += 1 + n as usize; if n == 0 { break } }
            }
            0x2C => {
                let d = b.get(p + 1..p + 10).ok_or("gif: short")?;
                let (x0, y0, fw, fh) = (u16::from_le_bytes([d[0], d[1]]) as usize, u16::from_le_bytes([d[2], d[3]]) as usize, u16::from_le_bytes([d[4], d[5]]) as usize, u16::from_le_bytes([d[6], d[7]]) as usize);
                let lf = d[8];
                p += 10;
                let mut pal = gct.clone();
                if lf & 0x80 != 0 { let n = 3 << ((lf & 7) + 1); pal = b.get(p..p + n).ok_or("gif: short")?.to_vec(); p += n }
                let min = *b.get(p).ok_or("gif: short")?;
                p += 1;
                if !(1..=11).contains(&min) { return Err("gif: bad lzw".into()) }
                let mut data = vec![];
                while let Some(&n) = b.get(p) { if n == 0 { p += 1; break } data.extend_from_slice(b.get(p + 1..p + 1 + n as usize).ok_or("gif: truncated")?); p += 1 + n as usize; }
                let idx = lzw(&data, min, fw * fh);
                let saved = if disposal == 3 { Some(canvas.clone()) } else { None };
                let rows: Vec<usize> = if lf & 0x40 != 0 { [(0, 8), (4, 8), (2, 4), (1, 2)].iter().flat_map(|&(s, st)| (s..fh).step_by(st)).collect() } else { (0..fh).collect() };
                for (k, &ry) in rows.iter().enumerate() {
                    for x in 0..fw {
                        let Some(&ci) = idx.get(k * fw + x) else { continue };
                        if Some(ci) == transp { continue }
                        let (cx, cy) = (x0 + x, y0 + ry);
                        if cx >= w || cy >= h { continue }
                        let c = pal.get(ci as usize * 3..ci as usize * 3 + 3).unwrap_or(&[0, 0, 0]);
                        canvas.rgba[(cy * w + cx) * 4..][..4].copy_from_slice(&[c[0], c[1], c[2], 255]);
                    }
                }
                frames.push(Frame { img: canvas.clone(), delay_ms: if delay == 0 { 100 } else { (delay * 10).max(20) } });
                match disposal {
                    2 => for y in y0..(y0 + fh).min(h) { for x in x0..(x0 + fw).min(w) { canvas.rgba[(y * w + x) * 4..][..4].copy_from_slice(&[0, 0, 0, 0]) } },
                    3 => if let Some(s) = saved { canvas = s },
                    _ => {}
                }
                disposal = 0; transp = None; delay = 10;
            }
            0x3B => break,
            _ => return Err("gif: bad block".into()),
        }
    }
    if frames.is_empty() { Err("gif: no frames".into()) } else { Ok(frames) }
}
