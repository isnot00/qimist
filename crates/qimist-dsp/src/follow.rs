//! Chord follower: turns sung notes into steady chord changes.
//!
//! Raw "every sung note = a chord" jumps around with every passing note and
//! wobble. The follower instead:
//! - picks a chord for the sung note: as its root (ROOT), or a chord from the
//!   key that *contains* it, preferring the current chord and the strong
//!   degrees I, IV, V, vi (MELODY, i.e. harmonising a tune);
//! - only changes chord once a new note has been held for `hold_s`, so passing
//!   notes don't count;
//! - optionally waits for the next beat or bar to change;
//! - lets the chord ring through short gaps (or until 2 s of silence);
//! - voices each chord close to the previous one (smooth voice leading);
//! - can be played directly from the seven chord pads.

use crate::chords::{self, Chord, ChordType};
use crate::scale::Scale;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FollowMode {
    /// Harmonise the tune: a chord from the key that contains the sung note.
    Melody,
    /// The sung note is the chord's root.
    Root,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeSync {
    Free,
    Beat,
    Bar,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FollowSettings {
    pub mode: FollowMode,
    /// How long a new note must be held before the chord changes.
    pub hold_s: f32,
    pub sync: ChangeSync,
    /// Keep the chord sounding through silences of up to 2 s.
    pub let_ring: bool,
    /// Choose inversions/octaves that move the least from the last chord.
    pub smooth: bool,
    pub kind: ChordType,
    pub inversion: u8,
    pub spread: bool,
    pub bass: bool,
}

impl Default for FollowSettings {
    fn default() -> Self {
        FollowSettings {
            mode: FollowMode::Melody,
            hold_s: 0.2,
            sync: ChangeSync::Free,
            let_ring: true,
            smooth: true,
            kind: ChordType::Triad,
            inversion: 0,
            spread: false,
            bass: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChordEvent {
    Play { chord: Chord, velocity: u8 },
    Release,
}

/// Silence that ends a chord: a short gap without LET RING, 2 s with it.
const GAP_S: f32 = 0.12;
const RING_S: f32 = 2.0;

/// Root note (around middle C) of the chord on scale `degree`.
pub fn pad_root(scale: Scale, degree: usize) -> u8 {
    let iv = scale.kind.intervals();
    let pc = (scale.root as usize + iv[degree % iv.len()] as usize) % 12;
    // Keep roots between G3 and F#4 so the pads sit in one register.
    let n = 48 + pc as i32;
    (if n < 55 { n + 12 } else { n }) as u8
}

/// Whether `scale` has the seven degrees the pads and MELODY mode need.
pub fn has_degrees(scale: Option<Scale>) -> bool {
    scale.is_some_and(|s| s.kind.intervals().len() == 7)
}

/// "I", "ii", "vii°", … for a built chord on scale `degree`.
pub fn roman(degree: usize, chord: &Chord) -> String {
    const R: [&str; 7] = ["I", "II", "III", "IV", "V", "VI", "VII"];
    let iv = &chord.intervals[..chord.n_intervals];
    let minor = iv.contains(&3);
    let dim = minor && iv.contains(&6);
    let base = R[degree % 7];
    if dim {
        format!("{}°", base.to_lowercase())
    } else if minor {
        base.to_lowercase()
    } else {
        base.to_string()
    }
}

pub struct ChordFollower {
    sr: f32,
    pub s: FollowSettings,
    /// What's sounding (root note, chord).
    current: Option<(u8, Chord)>,
    /// A different chord the singer moved to, and for how long they've held it.
    candidate: Option<(u8, u8)>,
    cand_age: usize,
    /// A change that is due, waiting for the beat/bar.
    pending: Option<(u8, u8)>,
    singing: bool,
    silent: usize,
    pad: Option<usize>,
    free_beat: f64,
    last_grid: i64,
}

impl ChordFollower {
    pub fn new(sr: f32) -> Self {
        ChordFollower {
            sr,
            s: FollowSettings::default(),
            current: None,
            candidate: None,
            cand_age: 0,
            pending: None,
            singing: false,
            silent: 0,
            pad: None,
            free_beat: 0.0,
            last_grid: i64::MIN,
        }
    }

    pub fn reset(&mut self) {
        *self = ChordFollower { s: self.s, ..ChordFollower::new(self.sr) };
    }

    /// The sounding chord's root note.
    pub fn current_root(&self) -> Option<u8> {
        self.current.map(|(r, _)| r)
    }

    pub fn current_chord(&self) -> Option<Chord> {
        self.current.map(|(_, c)| c)
    }

    /// Root of a change that's waiting for the beat.
    pub fn pending_root(&self) -> Option<u8> {
        self.pending.map(|(r, _)| r)
    }

    pub fn pad(&self) -> Option<usize> {
        self.pad
    }

    /// Root note of the chord for a sung `note`.
    pub fn choose_root(&self, note: u8, scale: Option<Scale>) -> u8 {
        let (FollowMode::Melody, Some(sc)) = (self.s.mode, scale.filter(|_| has_degrees(scale))) else {
            return note;
        };
        if !sc.contains(note as i32) {
            // An outside note: stay where we are (it's colour, not a new chord).
            return self.current_root().unwrap_or(note);
        }
        let minor_key = sc.kind.intervals()[2] == 3;
        let prior: [f32; 7] = if minor_key { [1.0, 0.0, 0.6, 0.8, 0.6, 0.8, 0.6] } else { [1.0, 0.4, 0.2, 0.8, 0.8, 0.6, 0.0] };
        let cur_pc = self.current_root().map(|r| r % 12);
        let mut best = (f32::MIN, note);
        for (d, p) in prior.iter().enumerate() {
            let root = pad_root(sc, d);
            let triad = chords::build(root, ChordType::Triad, Some(sc), 0, false, false);
            if !triad.notes().iter().any(|&n| n % 12 == note % 12) {
                continue;
            }
            let mut score = *p;
            if cur_pc == Some(root % 12) {
                score += 1.5;
            }
            // Sung note as the chord's root or fifth sounds most stable.
            if root % 12 == note % 12 {
                score += 0.3;
            }
            if score > best.0 {
                // Put the root at or below the sung note so the tune sits on top.
                let below = note as i32 - ((note as i32 - root as i32).rem_euclid(12));
                best = (score, below.clamp(0, 127) as u8);
            }
        }
        best.1
    }

    /// Build the chord on `root`, voiced close to what's sounding when SMOOTH is on.
    pub fn voice(&self, root: u8, scale: Option<Scale>) -> Chord {
        let s = &self.s;
        let plain = chords::build(root, s.kind, scale, s.inversion, s.spread, s.bass);
        let (true, Some((_, prev))) = (s.smooth, self.current) else { return plain };
        let prev: Vec<f32> = prev.notes().iter().map(|&n| n as f32).collect();
        let prev_mean = prev.iter().sum::<f32>() / prev.len() as f32;
        let tones = plain.n_intervals + 1;
        let mut best: Option<(f32, Chord)> = None;
        for oct in [-12i32, 0, 12] {
            let r = root as i32 + oct;
            if !(12..=115).contains(&r) {
                continue;
            }
            for inv in 0..tones as u8 {
                let c = chords::build(r as u8, s.kind, scale, inv, s.spread, s.bass);
                // Movement: each new note to its nearest old note; plus drift from the register.
                let moved: f32 = c.notes().iter().map(|&n| prev.iter().map(|&p| (n as f32 - p).abs()).fold(f32::MAX, f32::min)).sum();
                let mean = c.notes().iter().map(|&n| n as f32).sum::<f32>() / c.len as f32;
                let anchor = root as f32 + 4.0;
                let cost = moved + 0.5 * (mean - anchor).abs() + 0.3 * (mean - prev_mean).abs();
                if best.as_ref().is_none_or(|(b, _)| cost < *b) {
                    best = Some((cost, c));
                }
            }
        }
        let mut c = best.map(|(_, c)| c).unwrap_or(plain);
        c.root = root;
        c
    }

    fn play(&mut self, root: u8, velocity: u8, scale: Option<Scale>, out: &mut impl FnMut(ChordEvent)) {
        let chord = self.voice(root, scale);
        self.current = Some((root, chord));
        self.candidate = None;
        self.pending = None;
        out(ChordEvent::Play { chord, velocity });
    }

    fn release(&mut self, out: &mut impl FnMut(ChordEvent)) {
        if self.current.take().is_some() {
            out(ChordEvent::Release);
        }
        self.candidate = None;
        self.pending = None;
    }

    pub fn note_on(&mut self, note: u8, velocity: u8, scale: Option<Scale>, out: &mut impl FnMut(ChordEvent)) {
        self.singing = true;
        self.silent = 0;
        if self.pad.is_some() {
            return;
        }
        let root = self.choose_root(note, scale);
        match self.current {
            // Nothing sounding: start right away, no waiting.
            None => self.play(root, velocity, scale, out),
            Some((r, _)) if r % 12 == root % 12 => {
                self.candidate = None;
                self.pending = None;
            }
            Some(_) => {
                if self.candidate.map(|(r, _)| r % 12) != Some(root % 12) {
                    self.candidate = Some((root, velocity));
                    self.cand_age = 0;
                }
            }
        }
    }

    pub fn note_off(&mut self) {
        self.singing = false;
        self.silent = 0;
        // A note that ended before the hold time was a passing note.
        self.candidate = None;
    }

    pub fn pad_press(&mut self, degree: usize, velocity: u8, scale: Option<Scale>, out: &mut impl FnMut(ChordEvent)) {
        let Some(sc) = scale.filter(|_| has_degrees(scale)) else { return };
        self.pad = Some(degree);
        self.play(pad_root(sc, degree), velocity, scale, out);
    }

    pub fn pad_release(&mut self, out: &mut impl FnMut(ChordEvent)) {
        if self.pad.take().is_some() {
            self.release(out);
        }
    }

    /// Stop everything (mode change, transport stop).
    pub fn stop(&mut self, out: &mut impl FnMut(ChordEvent)) {
        self.pad = None;
        self.singing = false;
        self.release(out);
    }

    /// Re-voice the sounding chord after a settings change (style, key, …).
    pub fn refresh(&mut self, scale: Option<Scale>, out: &mut impl FnMut(ChordEvent)) {
        if let Some((root, chord)) = self.current {
            let new = self.voice(root, scale);
            if new.notes() != chord.notes() {
                self.current = Some((root, new));
                out(ChordEvent::Play { chord: new, velocity: 100 });
            }
        }
    }

    /// Advance one sample. `beat` is the host's beat position when it's
    /// playing; otherwise an internal clock at `bpm` keeps the grid.
    pub fn tick(&mut self, beat: Option<f64>, bpm: f32, scale: Option<Scale>, out: &mut impl FnMut(ChordEvent)) {
        self.free_beat += bpm as f64 / 60.0 / self.sr as f64;
        let beat = beat.unwrap_or(self.free_beat);
        let grid = match self.s.sync {
            ChangeSync::Free => 0.0,
            ChangeSync::Beat => 1.0,
            ChangeSync::Bar => 4.0,
        };
        let cell = if grid > 0.0 { (beat / grid).floor() as i64 } else { 0 };
        let crossed = cell != self.last_grid;
        self.last_grid = cell;

        if self.pad.is_some() {
            return;
        }
        if let Some(c) = self.candidate {
            self.cand_age += 1;
            if self.cand_age as f32 >= self.s.hold_s * self.sr {
                self.pending = Some(c);
                self.candidate = None;
            }
        }
        if let Some((root, vel)) = self.pending {
            if grid == 0.0 || crossed {
                self.play(root, vel, scale, out);
            }
        }
        if !self.singing && self.current.is_some() {
            self.silent += 1;
            let limit = if self.s.let_ring { RING_S } else { GAP_S };
            if self.silent as f32 >= limit * self.sr {
                self.release(out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scale::ScaleKind;

    const SR: f32 = 1000.0;

    fn run(f: &mut ChordFollower, ms: usize, scale: Option<Scale>, log: &mut Vec<ChordEvent>) {
        for _ in 0..ms {
            f.tick(None, 120.0, scale, &mut |e| log.push(e));
        }
    }

    fn names(log: &[ChordEvent]) -> Vec<String> {
        log.iter()
            .map(|e| match e {
                ChordEvent::Play { chord, .. } => chord.name(),
                ChordEvent::Release => "-".into(),
            })
            .collect()
    }

    #[test]
    fn passing_notes_dont_change_the_chord() {
        let c = Some(Scale::new(0, ScaleKind::Major));
        let mut f = ChordFollower::new(SR);
        f.s.mode = FollowMode::Root;
        let mut log = vec![];
        f.note_on(60, 100, c, &mut |e| log.push(e)); // C
        run(&mut f, 300, c, &mut log);
        f.note_off();
        f.note_on(62, 100, c, &mut |e| log.push(e)); // D for 80 ms: passing
        run(&mut f, 80, c, &mut log);
        f.note_off();
        f.note_on(67, 100, c, &mut |e| log.push(e)); // G held
        run(&mut f, 400, c, &mut log);
        assert_eq!(names(&log), ["C", "G"]);
    }

    #[test]
    fn melody_mode_harmonises_a_tune() {
        let c = Some(Scale::new(0, ScaleKind::Major));
        let mut f = ChordFollower::new(SR);
        let mut log = vec![];
        // E then G stay on C (both in C major); F moves to F; B goes to G.
        for (note, ms) in [(64, 300), (67, 300), (65, 400), (71, 400)] {
            f.note_on(note, 100, c, &mut |e| log.push(e));
            run(&mut f, ms, c, &mut log);
            f.note_off();
        }
        assert_eq!(names(&log), ["C", "F", "G"]);
        // Out-of-key note keeps the chord.
        f.note_on(66, 100, c, &mut |e| log.push(e));
        run(&mut f, 400, c, &mut log);
        assert_eq!(names(&log).len(), 3);
    }

    #[test]
    fn waits_for_the_bar_and_rings_then_releases() {
        let c = Some(Scale::new(0, ScaleKind::Major));
        let mut f = ChordFollower::new(SR);
        f.s.sync = ChangeSync::Bar; // 120 BPM: a bar = 2 s
        f.s.mode = FollowMode::Root;
        let mut log = vec![];
        f.note_on(60, 100, c, &mut |e| log.push(e));
        run(&mut f, 100, c, &mut log);
        f.note_on(65, 100, c, &mut |e| log.push(e));
        run(&mut f, 1500, c, &mut log);
        assert_eq!(names(&log), ["C"], "change waits for the bar line");
        run(&mut f, 500, c, &mut log);
        assert_eq!(names(&log), ["C", "F"]);
        f.note_off();
        run(&mut f, 1900, c, &mut log);
        assert_eq!(names(&log).len(), 2, "rings through a short silence");
        run(&mut f, 200, c, &mut log);
        assert_eq!(names(&log), ["C", "F", "-"]);
    }

    #[test]
    fn smooth_voicing_moves_little() {
        let c = Some(Scale::new(0, ScaleKind::Major));
        let mut f = ChordFollower::new(SR);
        f.s.mode = FollowMode::Root;
        f.s.hold_s = 0.0;
        let mut log = vec![];
        let mut last: Vec<u8> = vec![];
        for root in [60u8, 65, 67, 57, 62, 67, 60] {
            f.note_on(root, 100, c, &mut |e| log.push(e));
            run(&mut f, 5, c, &mut log);
            let now = f.current_chord().unwrap().notes().to_vec();
            if !last.is_empty() {
                let moved: i32 = now.iter().map(|&n| last.iter().map(|&p| (n as i32 - p as i32).abs()).min().unwrap()).sum();
                assert!(moved <= 7, "{last:?} → {now:?} moves {moved}");
            }
            assert!(now.iter().all(|&n| (50..=80).contains(&n)), "register drifted: {now:?}");
            last = now;
        }
    }

    #[test]
    fn pads_play_diatonic_chords() {
        let a_minor = Some(Scale::new(9, ScaleKind::NaturalMinor));
        let mut f = ChordFollower::new(SR);
        let mut log = vec![];
        for d in 0..7 {
            f.pad_press(d, 100, a_minor, &mut |e| log.push(e));
            f.pad_release(&mut |e| log.push(e));
        }
        let played: Vec<String> = names(&log).into_iter().filter(|n| n != "-").collect();
        assert_eq!(played, ["Am", "Bdim", "C", "Dm", "Em", "F", "G"]);
        let romans: Vec<String> = (0..7).map(|d| roman(d, &chords::build(pad_root(a_minor.unwrap(), d), ChordType::Triad, a_minor, 0, false, false))).collect();
        assert_eq!(romans, ["i", "ii°", "III", "iv", "v", "VI", "VII"]);
    }
}
