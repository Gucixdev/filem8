//! Asynchronous copy/move: one worker thread per paste/drop with a byte
//! counter the UI polls for the progress bar, and a cancel flag.
use fm8_core::fileops::{self, Op};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

pub struct Transfer {
    pub copy: bool,
    pub n: usize,
    pub dest: PathBuf,
    pub done: Arc<AtomicU64>,
    pub total: Arc<AtomicU64>,
    pub cancel: Arc<AtomicBool>,
    hold: Arc<AtomicBool>,
    rx: Receiver<Result<Op, String>>,
}

impl Transfer {
    pub fn start(paths: Vec<PathBuf>, dest: PathBuf, copy: bool) -> Transfer { Self::start_inner(paths, dest, copy, None) }

    /// Test hook: the worker idles until `release()`, so a cancel can be
    /// requested deterministically before any byte is copied.
    pub fn start_held(paths: Vec<PathBuf>, dest: PathBuf, copy: bool) -> Transfer { Self::start_inner(paths, dest, copy, Some(())) }

    fn start_inner(paths: Vec<PathBuf>, dest: PathBuf, copy: bool, held: Option<()>) -> Transfer {
        let (tx, rx) = channel();
        let (done, total, cancel) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)), Arc::new(AtomicBool::new(false)));
        let t = Transfer { copy, n: paths.len(), dest: dest.clone(), done: done.clone(), total: total.clone(), cancel: cancel.clone(), hold: Arc::new(AtomicBool::new(held.is_some())), rx };
        let hold = t.hold.clone();
        std::thread::spawn(move || {
            while hold.load(Relaxed) { std::thread::sleep(std::time::Duration::from_millis(2)); }
            total.store(fileops::tree_size(&paths), Relaxed);
            let mut prog = |b: u64| { done.store(b, Relaxed); !cancel.load(Relaxed) };
            let r = if copy { fileops::copy_to(&paths, &dest, &mut prog) } else { fileops::move_to_with(&paths, &dest, &mut prog) };
            let _ = tx.send(r);
        });
        t
    }
    pub fn fraction(&self) -> f32 {
        let t = self.total.load(Relaxed);
        if t == 0 { 0.0 } else { (self.done.load(Relaxed) as f32 / t as f32).clamp(0.0, 1.0) }
    }
    pub fn release(&self) { self.hold.store(false, Relaxed); }
    pub fn cancel(&self) { self.cancel.store(true, Relaxed); }
    pub fn cancelled(&self) -> bool { self.cancel.load(Relaxed) }
    pub fn poll(&self) -> Option<Result<Op, String>> { self.rx.try_recv().ok() }
}
