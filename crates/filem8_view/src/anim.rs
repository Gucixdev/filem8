//! Frame timing. `looping` wraps at the end, otherwise the last frame stays and playing stops.

#[derive(Clone, Debug)]
pub struct Anim {
    pub delays: Vec<u32>,
    pub idx: usize,
    pub playing: bool,
    pub looping: bool,
    elapsed: u32,
}

impl Anim {
    pub fn new(delays: Vec<u32>) -> Anim {
        let playing = delays.len() > 1;
        Anim { delays, idx: 0, playing, looping: true, elapsed: 0 }
    }

    pub fn animated(&self) -> bool {
        self.delays.len() > 1
    }

    fn delay(&self) -> u32 {
        self.delays.get(self.idx).copied().unwrap_or(100).max(20)
    }

    /// Time passed; true when the frame changed.
    pub fn advance(&mut self, ms: u32) -> bool {
        if !self.playing || !self.animated() {
            return false;
        }
        self.elapsed += ms;
        let start = self.idx;
        while self.elapsed >= self.delay() {
            self.elapsed -= self.delay();
            if self.idx + 1 < self.delays.len() {
                self.idx += 1;
            } else if self.looping {
                self.idx = 0;
            } else {
                self.playing = false;
                self.elapsed = 0;
                break;
            }
        }
        self.idx != start
    }

    pub fn toggle_play(&mut self) {
        // pressing play after the end starts over
        if !self.playing && !self.looping && self.idx + 1 == self.delays.len() {
            self.idx = 0;
        }
        self.playing = !self.playing && self.animated();
        self.elapsed = 0;
    }

    pub fn toggle_loop(&mut self) {
        self.looping = !self.looping;
    }

    pub fn step(&mut self, forward: bool) {
        self.playing = false;
        let n = self.delays.len().max(1);
        self.idx = if forward { (self.idx + 1) % n } else { (self.idx + n - 1) % n };
    }

    /// Milliseconds until the frame changes (for the event loop's wake-up), None when still.
    pub fn next_in(&self) -> Option<u32> {
        (self.playing && self.animated()).then(|| self.delay().saturating_sub(self.elapsed).max(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loops_and_stops() {
        let mut a = Anim::new(vec![100, 100, 100]);
        assert!(a.advance(100) && a.idx == 1);
        assert!(!a.advance(50));
        assert!(a.advance(50) && a.idx == 2);
        assert!(a.advance(100) && a.idx == 0, "wraps while looping");
        a.looping = false;
        a.idx = 2;
        assert!(!a.advance(500) && a.idx == 2 && !a.playing, "stays on the last frame");
        a.toggle_play();
        assert!(a.playing && a.idx == 0, "play after the end starts over");
    }

    #[test]
    fn a_long_stall_catches_up_without_hanging() {
        let mut a = Anim::new(vec![20, 20]);
        a.advance(1_000_001);
        assert!(a.idx < 2);
    }

    #[test]
    fn a_still_picture_never_plays() {
        let mut a = Anim::new(vec![100]);
        assert!(!a.animated() && !a.advance(1000) && a.next_in().is_none());
        a.toggle_play();
        assert!(!a.playing);
    }

    #[test]
    fn stepping_pauses() {
        let mut a = Anim::new(vec![50, 50, 50]);
        a.step(false);
        assert!(!a.playing && a.idx == 2);
        assert_eq!(a.next_in(), None);
    }
}
