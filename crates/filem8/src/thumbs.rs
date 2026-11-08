//! Background thumbnail loader for the grid view. Two worker threads decode
//! small versions of images; the UI asks `get()` every frame and gets `None`
//! until a thumbnail is ready (the caller draws the icon meanwhile).
use fm8_ui::Image;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

enum Slot {
    Pending,
    Ready(Arc<Image>),
    Failed,
}

pub struct Thumbs {
    map: HashMap<PathBuf, Slot>,
    jobs: Sender<PathBuf>,
    done: Receiver<(PathBuf, Option<Arc<Image>>)>,
    pending: usize,
}

impl Default for Thumbs {
    fn default() -> Self { Thumbs::new() }
}

impl Thumbs {
    pub fn new() -> Thumbs {
        let (jtx, jrx) = channel::<PathBuf>();
        let (dtx, drx) = channel();
        let jrx = Arc::new(Mutex::new(jrx));
        for _ in 0..2 {
            let (jrx, dtx) = (jrx.clone(), dtx.clone());
            std::thread::spawn(move || loop {
                let job = { jrx.lock().unwrap().recv() };
                let Ok(p) = job else { return };
                let o = fm8_preview::Opts { max_dim: 192, max_frames: 1, ..Default::default() };
                let img = match fm8_preview::image_load::load(&p, &o) {
                    fm8_preview::Preview::Image(d) => d.frames.into_iter().next().map(|f| Image::new(f.w, f.h, f.rgba)),
                    _ => None,
                };
                if dtx.send((p, img)).is_err() { return; }
            });
        }
        Thumbs { map: HashMap::new(), jobs: jtx, done: drx, pending: 0 }
    }

    pub fn get(&mut self, p: &PathBuf) -> Option<Arc<Image>> {
        match self.map.get(p) {
            Some(Slot::Ready(i)) => Some(i.clone()),
            Some(_) => None,
            None => {
                if self.map.len() > 800 { self.map.retain(|_, s| matches!(s, Slot::Ready(_))); if self.map.len() > 800 { self.map.clear(); } }
                self.map.insert(p.clone(), Slot::Pending);
                self.pending += 1;
                let _ = self.jobs.send(p.clone());
                None
            }
        }
    }

    pub fn forget(&mut self) { self.map.retain(|_, s| matches!(s, Slot::Ready(_))); }

    /// Collect finished thumbnails. True when something new arrived.
    pub fn poll(&mut self) -> bool {
        let mut ch = false;
        while let Ok((p, img)) = self.done.try_recv() {
            self.pending = self.pending.saturating_sub(1);
            self.map.insert(p, match img { Some(i) => Slot::Ready(i), None => Slot::Failed });
            ch = true;
        }
        ch
    }

    pub fn busy(&self) -> bool { self.pending > 0 }
}
