use crate::{ArchiveEntry, Preview};
use std::io::Read;
use std::path::Path;

const CAP: usize = 5000;

pub fn load(path: &Path, name: &str) -> Preview {
    let lower = name.to_lowercase();
    let r = if lower.ends_with(".zip") || lower.ends_with(".jar") || lower.ends_with(".apk") || lower.ends_with(".whl") || lower.ends_with(".epub") || lower.ends_with(".crate_") {
        zip_list(path).map(|(e, t)| ("ZIP", e, t))
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        tar_list(path, true).map(|(e, t)| ("TAR.GZ", e, t))
    } else if lower.ends_with(".tar") {
        tar_list(path, false).map(|(e, t)| ("TAR", e, t))
    } else if lower.ends_with(".crate") {
        tar_list(path, true).map(|(e, t)| ("TAR.GZ", e, t))
    } else {
        // sniff zip magic for other container names
        let mut h = [0u8; 4];
        let is_zip = std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut h)).is_ok() && &h == b"PK\x03\x04";
        if is_zip { zip_list(path).map(|(e, t)| ("ZIP", e, t)) } else {
            return Preview::Info(vec![format!("{name}"), "Archive format not listable (read-only listing supports zip, tar, tar.gz)".into()]);
        }
    };
    match r {
        Ok((kind, entries, truncated)) => Preview::Archive { kind: kind.into(), entries, truncated },
        Err(e) => Preview::Error(e),
    }
}

fn zip_list(path: &Path) -> Result<(Vec<ArchiveEntry>, bool), String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|e| e.to_string())?;
    let n = z.len();
    let mut v = Vec::new();
    for i in 0..n.min(CAP) {
        let e = z.by_index_raw(i).map_err(|e| e.to_string())?;
        v.push(ArchiveEntry { name: e.name().to_string(), size: e.size(), is_dir: e.is_dir() });
    }
    Ok((v, n > CAP))
}

fn tar_list(path: &Path, gz: bool) -> Result<(Vec<ArchiveEntry>, bool), String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let r: Box<dyn Read> = if gz { Box::new(flate2::read::GzDecoder::new(f)) } else { Box::new(f) };
    let mut a = tar::Archive::new(r);
    let mut v = Vec::new();
    let mut trunc = false;
    for e in a.entries().map_err(|e| e.to_string())? {
        let e = e.map_err(|e| e.to_string())?;
        if v.len() >= CAP { trunc = true; break; }
        v.push(ArchiveEntry { name: e.path().map_err(|e| e.to_string())?.to_string_lossy().into_owned(), size: e.header().size().unwrap_or(0), is_dir: e.header().entry_type().is_dir() });
    }
    Ok((v, trunc))
}
