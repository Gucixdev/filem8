//! The preview pane's worker: `fm8-preview` loads what the cursor is on (text, archive listing, picture, metadata) off the
//! UI thread. Only the newest request matters: answers carry the request number and the app drops old ones.
use fm8_preview::{Opts, Preview};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

/// What the pane shows: lines of text, or one picture.
pub enum Shown {
    Lines(Vec<String>),
    Picture { w: usize, h: usize, rgba: Vec<u8> },
}

const MAX_LINES: usize = 400;

/// Turn a loaded preview into what the pane can draw.
pub fn shown(p: Preview) -> Shown {
    match p {
        Preview::Image(img) => match img.frames.into_iter().next() {
            Some(f) => Shown::Picture { w: f.w as usize, h: f.h as usize, rgba: f.rgba },
            None => Shown::Lines(vec!["empty image".into()]),
        },
        Preview::Text(t) => {
            let mut lines: Vec<String> = t.lines.iter().take(MAX_LINES).map(|l| l.iter().map(|(_, s)| s.as_str()).collect()).collect();
            if t.truncated || t.lines.len() > MAX_LINES {
                lines.push(format!("... ({} bytes in all)", t.total_bytes));
            }
            Shown::Lines(lines)
        }
        Preview::Archive { kind, entries, truncated } => {
            let mut lines = vec![format!("{kind} archive, {} entries", entries.len())];
            lines.extend(entries.iter().take(MAX_LINES).map(|e| format!("{}{}", e.name, if e.is_dir { "/" } else { "" })));
            if truncated {
                lines.push("...".into());
            }
            Shown::Lines(lines)
        }
        Preview::Audio(a) => Shown::Lines(vec![format!("{} audio", a.codec), format!("{:.1} s, {} Hz, {} ch", a.duration_s, a.sample_rate, a.channels)]),
        Preview::Model(m) => Shown::Lines(vec![format!("3D model, {} triangles", m.triangles())]),
        Preview::Info(v) => Shown::Lines(v),
        Preview::Error(e) => Shown::Lines(vec![e]),
    }
}

pub struct Previewer {
    tx: Sender<(u64, PathBuf)>,
    rx: Receiver<(u64, Shown)>,
}

impl Default for Previewer {
    fn default() -> Previewer {
        let (tx, jobs) = channel::<(u64, PathBuf)>();
        let (out, rx) = channel();
        std::thread::spawn(move || {
            let opts = Opts { max_dim: 512, max_text_bytes: 16 * 1024, max_frames: 1 };
            for (seq, path) in jobs {
                let _ = out.send((seq, shown(fm8_preview::load(&path, &opts))));
            }
        });
        Previewer { tx, rx }
    }
}

impl Previewer {
    pub fn ask(&self, seq: u64, path: PathBuf) {
        let _ = self.tx.send((seq, path));
    }

    /// The newest finished answer, older ones dropped.
    pub fn latest(&self) -> Option<(u64, Shown)> {
        self.rx.try_iter().last()
    }
}
