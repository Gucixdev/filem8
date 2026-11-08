use crate::{ArchiveEntry, Preview};
use fm8_codec::{inflate, tar, zip};
use std::path::Path;

const CAP: usize = 5000;
/// Decompressed bytes inspected when listing a tar.gz (keeps huge archives cheap).
const TAR_HEAD: usize = 64 << 20;

pub fn load(path: &Path, name: &str) -> Preview {
    let lower = name.to_lowercase();
    let data = match read_head(path) { Ok(d) => d, Err(e) => return Preview::Error(e) };
    let r = if data.starts_with(b"PK\x03\x04") || data.starts_with(b"PK\x05\x06") {
        zip_list(path).map(|(e, t)| ("ZIP", e, t))
    } else if data.starts_with(&[0x1f, 0x8b]) && (lower.ends_with(".tar.gz") || lower.ends_with(".tgz") || lower.ends_with(".crate")) {
        std::fs::read(path).map_err(|e| e.to_string()).and_then(|b| inflate::gunzip_prefix(&b, TAR_HEAD)).map(|t| tar_list(&t, "TAR.GZ"))
    } else if lower.ends_with(".tar") || data.get(257..262) == Some(b"ustar") {
        std::fs::read(path).map_err(|e| e.to_string()).map(|b| tar_list(&b, "TAR"))
    } else {
        return Preview::Info(vec![name.to_string(), "Archive format not listable (read-only listing supports zip, tar, tar.gz)".into()]);
    };
    match r {
        Ok((kind, entries, truncated)) => Preview::Archive { kind: kind.into(), entries, truncated },
        Err(e) => Preview::Error(e),
    }
}

fn read_head(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut v = vec![0u8; 512];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut v)).map_err(|e| e.to_string())?;
    v.truncate(n);
    Ok(v)
}

fn zip_list(path: &Path) -> Result<(Vec<ArchiveEntry>, bool), String> {
    let b = std::fs::read(path).map_err(|e| e.to_string())?;
    let (es, more) = zip::list(&b, CAP)?;
    Ok((es.into_iter().map(|e| ArchiveEntry { name: e.name, size: e.size, is_dir: e.is_dir }).collect(), more))
}

fn tar_list(buf: &[u8], kind: &'static str) -> (&'static str, Vec<ArchiveEntry>, bool) {
    let (es, more) = tar::list(buf, CAP);
    (kind, es.into_iter().map(|e| ArchiveEntry { name: e.name, size: e.size, is_dir: e.is_dir }).collect(), more)
}
