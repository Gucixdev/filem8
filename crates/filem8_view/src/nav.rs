//! The files beside the one you opened, so n / p walk through a folder.

use std::path::{Path, PathBuf};

pub struct Nav {
    pub files: Vec<PathBuf>,
    pub idx: usize,
}

/// Regular, non-hidden files of `path`'s folder, sorted by name (digits compared as numbers: img2 before img10).
pub fn siblings(path: &Path) -> Nav {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter(|e| std::fs::metadata(e.path()).is_ok_and(|m| m.is_file()))
        .map(|e| e.path())
        .collect();
    files.sort_by_key(|p| natural(&p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default()));
    let idx = files.iter().position(|f| f == path).unwrap_or_else(|| {
        files.insert(0, path.to_path_buf());
        0
    });
    Nav { files, idx }
}

fn natural(s: &str) -> Vec<(u8, u64, String)> {
    let mut out = Vec::new();
    let mut it = s.chars().peekable();
    while let Some(&c) = it.peek() {
        if c.is_ascii_digit() {
            let mut n = 0u64;
            while let Some(d) = it.peek().and_then(|c| c.to_digit(10)) {
                n = n.saturating_mul(10).saturating_add(d as u64);
                it.next();
            }
            out.push((0, n, String::new()));
        } else {
            let mut t = String::new();
            while let Some(&c) = it.peek().filter(|c| !c.is_ascii_digit()) {
                t.push(c);
                it.next();
            }
            out.push((1, 0, t));
        }
    }
    out
}

impl Nav {
    pub fn step(&mut self, forward: bool) -> &Path {
        let n = self.files.len().max(1);
        self.idx = if forward { (self.idx + 1) % n } else { (self.idx + n - 1) % n };
        &self.files[self.idx]
    }

    pub fn go(&mut self, first: bool) -> &Path {
        self.idx = if first { 0 } else { self.files.len() - 1 };
        &self.files[self.idx]
    }

    pub fn label(&self) -> String {
        format!("{}/{}", self.idx + 1, self.files.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_sort_as_numbers() {
        assert!(natural("img2.png") < natural("img10.png"));
        assert!(natural("a1") < natural("b1"));
    }
}
