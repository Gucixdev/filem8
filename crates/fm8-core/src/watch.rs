//! Filesystem watching for the open directory (inotify via `notify`).
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

pub struct DirWatch {
    _w: RecommendedWatcher,
    rx: Receiver<notify::Result<notify::Event>>,
    pub dir: PathBuf,
}

impl DirWatch {
    pub fn new(dir: &Path) -> Result<DirWatch, String> {
        let (tx, rx) = channel();
        let mut w = notify::recommended_watcher(tx).map_err(|e| e.to_string())?;
        w.watch(dir, RecursiveMode::NonRecursive).map_err(|e| e.to_string())?;
        Ok(DirWatch { _w: w, rx, dir: dir.to_path_buf() })
    }
    /// True if anything relevant changed since the last call (events coalesced).
    pub fn changed(&self) -> bool {
        let mut any = false;
        while let Ok(ev) = self.rx.try_recv() {
            if let Ok(e) = ev {
                if !matches!(e.kind, notify::EventKind::Access(_)) { any = true; }
            }
        }
        any
    }
}
