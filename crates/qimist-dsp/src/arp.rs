//! Arpeggiator: plays the held notes (the sung note, or its chord) one at a
//! time at a tempo-synced rate. While the host is playing, steps land on the
//! host's beat grid; otherwise it starts on the first note you sing, so it
//! locks to your phrasing.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArpPattern {
    Up,
    Down,
    UpDown,
    Random,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArpSettings {
    pub pattern: ArpPattern,
    /// Steps per whole note: 4 = quarters, 8 = eighths, 12 = eighth triplets, 16, 24, 32.
    pub steps_per_whole: f32,
    /// 1..=4 octaves.
    pub octaves: u8,
    /// Note length as a fraction of the step, 0.05..1.
    pub gate: f32,
}

impl Default for ArpSettings {
    fn default() -> Self {
        ArpSettings { pattern: ArpPattern::Up, steps_per_whole: 16.0, octaves: 1, gate: 0.6 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArpEvent {
    On { note: u8, velocity: u8 },
    Off { note: u8 },
}

const MAX_HELD: usize = 8;
const MAX_SEQ: usize = 64;

pub struct Arp {
    sr: f32,
    s: ArpSettings,
    held: [u8; MAX_HELD],
    n_held: usize,
    velocity: u8,
    seq: [u8; MAX_SEQ],
    seq_len: usize,
    idx: usize,
    counter: f64,
    sounding: Option<u8>,
    off_at: f64,
    rng: u32,
    /// Last host grid step seen (synced mode).
    last_step: i64,
}

impl Arp {
    pub fn new(sr: f32) -> Self {
        Arp {
            sr,
            s: ArpSettings::default(),
            held: [0; MAX_HELD],
            n_held: 0,
            velocity: 100,
            seq: [0; MAX_SEQ],
            seq_len: 0,
            idx: 0,
            counter: 0.0,
            sounding: None,
            off_at: 0.0,
            rng: 0x9E37_79B9,
            last_step: i64::MIN,
        }
    }

    pub fn set(&mut self, s: ArpSettings) {
        if s != self.s {
            self.s = s;
            self.rebuild();
        }
    }

    pub fn is_running(&self) -> bool {
        self.n_held > 0
    }

    /// Replace the held notes. An empty slice stops the arp (after the current note's gate).
    pub fn set_held(&mut self, notes: &[u8], velocity: u8) {
        let was_empty = self.n_held == 0;
        self.n_held = notes.len().min(MAX_HELD);
        self.held[..self.n_held].copy_from_slice(&notes[..self.n_held]);
        self.held[..self.n_held].sort_unstable();
        self.velocity = velocity;
        self.rebuild();
        if was_empty && self.n_held > 0 {
            // Start right away on the first step.
            self.idx = 0;
            self.counter = f64::MAX;
        }
    }

    fn rebuild(&mut self) {
        let mut up = [0u8; MAX_SEQ];
        let mut n = 0;
        for o in 0..self.s.octaves.clamp(1, 4) {
            for &h in &self.held[..self.n_held] {
                let note = h as u16 + 12 * o as u16;
                if note <= 127 && n < MAX_SEQ {
                    up[n] = note as u8;
                    n += 1;
                }
            }
        }
        self.seq_len = 0;
        let push = |x: u8, seq: &mut [u8; MAX_SEQ], len: &mut usize| {
            if *len < MAX_SEQ {
                seq[*len] = x;
                *len += 1;
            }
        };
        match self.s.pattern {
            ArpPattern::Up | ArpPattern::Random => up[..n].iter().for_each(|&x| push(x, &mut self.seq, &mut self.seq_len)),
            ArpPattern::Down => up[..n].iter().rev().for_each(|&x| push(x, &mut self.seq, &mut self.seq_len)),
            ArpPattern::UpDown => {
                up[..n].iter().for_each(|&x| push(x, &mut self.seq, &mut self.seq_len));
                if n > 2 {
                    up[1..n - 1].iter().rev().for_each(|&x| push(x, &mut self.seq, &mut self.seq_len));
                }
            }
        }
        if self.seq_len > 0 {
            self.idx %= self.seq_len;
        }
    }

    fn rand(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    /// Advance one sample at `bpm`. `beat` is the host's song position in
    /// quarter notes at this sample when the transport is playing.
    #[inline]
    pub fn tick(&mut self, bpm: f32, beat: Option<f64>, out: &mut impl FnMut(ArpEvent)) {
        let step = self.sr as f64 * 60.0 / bpm.clamp(20.0, 400.0) as f64 * 4.0 / self.s.steps_per_whole.max(1.0) as f64;
        if self.counter != f64::MAX {
            self.counter += 1.0;
        }
        if let Some(n) = self.sounding {
            if self.counter >= self.off_at {
                out(ArpEvent::Off { note: n });
                self.sounding = None;
            }
        }
        // Grid step crossings (tracked even while idle, so a new note waits for the next step).
        let grid_fire = match beat {
            Some(b) => {
                let idx = (b * self.s.steps_per_whole as f64 / 4.0).floor() as i64;
                let fire = self.last_step != i64::MIN && idx != self.last_step;
                self.last_step = idx;
                Some(fire)
            }
            None => {
                self.last_step = i64::MIN;
                None
            }
        };
        if self.n_held == 0 || self.seq_len == 0 {
            // Let the last note finish its gate, then idle.
            if self.sounding.is_none() {
                self.counter = 0.0;
            }
            return;
        }
        if grid_fire.unwrap_or(self.counter >= step) {
            if let Some(n) = self.sounding.take() {
                out(ArpEvent::Off { note: n });
            }
            let note = if self.s.pattern == ArpPattern::Random {
                self.seq[self.rand() as usize % self.seq_len]
            } else {
                let n = self.seq[self.idx % self.seq_len];
                self.idx = (self.idx + 1) % self.seq_len;
                n
            };
            out(ArpEvent::On { note, velocity: self.velocity });
            self.sounding = Some(note);
            self.counter = 0.0;
            self.off_at = (step * self.s.gate.clamp(0.05, 1.0) as f64).max(1.0);
        }
    }

    /// Stop immediately.
    pub fn stop(&mut self, out: &mut impl FnMut(ArpEvent)) {
        if let Some(n) = self.sounding.take() {
            out(ArpEvent::Off { note: n });
        }
        self.n_held = 0;
        self.seq_len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(arp: &mut Arp, samples: usize) -> Vec<u8> {
        let mut ons = vec![];
        for _ in 0..samples {
            arp.tick(120.0, None, &mut |e| {
                if let ArpEvent::On { note, .. } = e {
                    ons.push(note)
                }
            });
        }
        ons
    }

    #[test]
    fn up_down_over_two_octaves() {
        let mut a = Arp::new(48_000.0);
        a.set(ArpSettings { pattern: ArpPattern::UpDown, steps_per_whole: 16.0, octaves: 2, gate: 0.5 });
        a.set_held(&[64, 60, 67], 100);
        // 16ths at 120 BPM = 6000 samples per step.
        let ons = run(&mut a, 6000 * 10 - 1);
        assert_eq!(ons, vec![60, 64, 67, 72, 76, 79, 76, 72, 67, 64]);
    }

    #[test]
    fn stops_and_sends_offs() {
        let mut a = Arp::new(48_000.0);
        a.set_held(&[60], 100);
        let mut balance = 0i32;
        for i in 0..48_000 {
            if i == 20_000 {
                a.set_held(&[], 100);
            }
            a.tick(120.0, None, &mut |e| balance += if matches!(e, ArpEvent::On { .. }) { 1 } else { -1 });
        }
        assert_eq!(balance, 0);
    }

    #[test]
    fn follows_host_grid() {
        let mut a = Arp::new(48_000.0);
        a.set(ArpSettings { steps_per_whole: 8.0, ..Default::default() });
        // Start a third of the way into an eighth: first note waits for the next grid line.
        let spb = 24_000.0; // samples per beat at 120 BPM
        let start = 0.5 / 3.0;
        a.set_held(&[60, 64], 100);
        let mut ons = vec![];
        for i in 0..48_000 {
            let beat = start + i as f64 / spb;
            a.tick(120.0, Some(beat), &mut |e| {
                if let ArpEvent::On { note, .. } = e {
                    ons.push((i, note))
                }
            });
        }
        // Grid lines every 0.5 beat = 12 000 samples; first at beat 0.5.
        let first = ((0.5 - start) * spb).round() as usize;
        assert_eq!(ons.iter().map(|o| o.0).collect::<Vec<_>>(), vec![first, first + 12_000, first + 24_000, first + 36_000]);
    }
}
