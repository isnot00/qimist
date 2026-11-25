//! Turns a frame-by-frame pitch track into clean note on/off events.
//!
//! - Median filter over the last 5 voiced frames removes leftover spikes.
//! - A new note must be stable for `min_note_ms` before it is emitted.
//! - Hysteresis: once a note sounds, the pitch must move past the half-way
//!   point by `hysteresis_cents` before we switch, so vibrato and drifting
//!   between two notes don't cause flutter.
//! - Re-articulation: a sharp level rise during a held note (e.g. "da-da-da"
//!   on one pitch) retriggers the note.
//! - Short unvoiced gaps (consonants) shorter than `release_ms` are bridged.

use crate::pitch::PitchFrame;
use crate::scale::Scale;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VelocityCurve {
    Fixed,
    Linear,
    Soft,
    Hard,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteConfig {
    pub min_note_ms: f32,
    pub release_ms: f32,
    pub hysteresis_cents: f32,
    /// Semitones added to every note (octave shift folded in).
    pub transpose: i32,
    pub snap: Option<Scale>,
    pub pitch_bend: bool,
    pub bend_range: f32,
    pub velocity_curve: VelocityCurve,
    pub fixed_velocity: u8,
    /// Level jump (dB) that retriggers a held note.
    pub retrigger_db: f32,
}

impl Default for NoteConfig {
    fn default() -> Self {
        NoteConfig {
            min_note_ms: 45.0,
            release_ms: 50.0,
            hysteresis_cents: 30.0,
            transpose: 0,
            snap: None,
            pitch_bend: false,
            bend_range: 2.0,
            velocity_curve: VelocityCurve::Soft,
            fixed_velocity: 100,
            retrigger_db: 9.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NoteEvent {
    /// `frames_ago`: how many analysis frames ago the note actually began.
    On { note: u8, velocity: u8, frames_ago: u32 },
    Off { note: u8, frames_ago: u32 },
    /// Pitch bend in semitones relative to the sounding note.
    Bend { semitones: f32 },
}

pub struct NoteTracker {
    cfg: NoteConfig,
    dt_ms: f32,
    /// Detected (untransposed) note currently sounding.
    current: Option<u8>,
    /// Note actually sent (after transpose) — used for the matching Off.
    sent: Option<u8>,
    pending: Option<u8>,
    pending_frames: u32,
    pending_peak_db: f32,
    unvoiced_frames: u32,
    frames_on: u32,
    med: [f32; 5],
    med_len: usize,
    med_pos: usize,
    valley_db: f32,
    last_bend: f32,
    smoothed: f32,
}

impl NoteTracker {
    pub fn new(frame_period_s: f32, cfg: NoteConfig) -> Self {
        NoteTracker {
            cfg,
            dt_ms: frame_period_s * 1000.0,
            current: None,
            sent: None,
            pending: None,
            pending_frames: 0,
            pending_peak_db: -120.0,
            unvoiced_frames: 0,
            frames_on: 0,
            med: [0.0; 5],
            med_len: 0,
            med_pos: 0,
            valley_db: 0.0,
            last_bend: 0.0,
            smoothed: 0.0,
        }
    }

    pub fn set_config(&mut self, cfg: NoteConfig) {
        self.cfg = cfg;
    }

    pub fn config(&self) -> &NoteConfig {
        &self.cfg
    }

    /// Note currently sent to the output (after transpose).
    pub fn sounding(&self) -> Option<u8> {
        self.sent
    }

    /// Detected note before transpose (used for key detection).
    pub fn detected(&self) -> Option<u8> {
        self.current
    }

    /// Median-smoothed pitch of the current voiced segment.
    pub fn smoothed_midi(&self) -> f32 {
        self.smoothed
    }

    fn frames(&self, ms: f32) -> u32 {
        (ms / self.dt_ms).ceil().max(1.0) as u32
    }

    fn velocity(&self, db: f32) -> u8 {
        let x = ((db + 50.0) / 38.0).clamp(0.0, 1.0);
        let y = match self.cfg.velocity_curve {
            VelocityCurve::Fixed => return self.cfg.fixed_velocity.clamp(1, 127),
            VelocityCurve::Linear => x,
            VelocityCurve::Soft => x.powf(0.6),
            VelocityCurve::Hard => x.powf(1.6),
        };
        (1.0 + y * 126.0).round() as u8
    }

    fn out_note(&self, n: u8) -> u8 {
        (n as i32 + self.cfg.transpose).clamp(0, 127) as u8
    }

    fn quantize(&self, m: f32) -> u8 {
        match self.cfg.snap {
            Some(s) => s.snap(m),
            None => m.round().clamp(0.0, 127.0) as u8,
        }
    }

    fn start(&mut self, n: u8, db: f32, frames_ago: u32, out: &mut impl FnMut(NoteEvent)) {
        let o = self.out_note(n);
        out(NoteEvent::On { note: o, velocity: self.velocity(db), frames_ago });
        self.current = Some(n);
        self.sent = Some(o);
        self.frames_on = 0;
        self.valley_db = db;
        self.last_bend = 0.0;
    }

    fn stop(&mut self, frames_ago: u32, out: &mut impl FnMut(NoteEvent)) {
        if let Some(o) = self.sent.take() {
            out(NoteEvent::Off { note: o, frames_ago });
        }
        self.current = None;
    }

    /// Release any sounding note (transport stop, bypass, reset).
    pub fn flush(&mut self, out: &mut impl FnMut(NoteEvent)) {
        self.stop(0, out);
        self.pending = None;
        self.med_len = 0;
    }

    pub fn push(&mut self, f: &PitchFrame, out: &mut impl FnMut(NoteEvent)) {
        if !f.voiced {
            self.med_len = 0;
            self.pending = None;
            self.unvoiced_frames += 1;
            if self.current.is_some() && self.unvoiced_frames >= self.frames(self.cfg.release_ms) {
                let ago = self.unvoiced_frames;
                self.stop(ago, out);
            }
            return;
        }
        self.unvoiced_frames = 0;

        // Median of the last voiced frames.
        self.med[self.med_pos] = f.midi;
        self.med_pos = (self.med_pos + 1) % self.med.len();
        self.med_len = (self.med_len + 1).min(self.med.len());
        let mut tmp = [0.0f32; 5];
        tmp[..self.med_len].copy_from_slice(&self.med[..self.med_len]);
        tmp[..self.med_len].sort_unstable_by(f32::total_cmp);
        let m = tmp[self.med_len / 2];
        self.smoothed = m;
        let target = self.quantize(m);
        let min_frames = self.frames(self.cfg.min_note_ms);

        match self.current {
            None => {
                if self.pending == Some(target) {
                    self.pending_frames += 1;
                    self.pending_peak_db = self.pending_peak_db.max(f.rms_db);
                } else {
                    self.pending = Some(target);
                    self.pending_frames = 1;
                    self.pending_peak_db = f.rms_db;
                }
                if self.pending_frames >= min_frames {
                    let (db, ago) = (self.pending_peak_db, self.pending_frames);
                    self.pending = None;
                    self.start(target, db, ago, out);
                }
            }
            Some(n) => {
                self.frames_on += 1;
                // Re-articulation on a sharp level rise.
                self.valley_db = f.rms_db.min(self.valley_db + 0.25);
                if f.rms_db - self.valley_db > self.cfg.retrigger_db && self.frames_on > min_frames {
                    self.stop(0, out);
                    self.start(n, f.rms_db, 0, out);
                    return;
                }

                let threshold = 0.5 + self.cfg.hysteresis_cents / 100.0;
                if target != n && (m - n as f32).abs() > threshold {
                    if self.pending == Some(target) {
                        self.pending_frames += 1;
                    } else {
                        self.pending = Some(target);
                        self.pending_frames = 1;
                    }
                    if self.pending_frames >= min_frames {
                        let ago = self.pending_frames;
                        self.pending = None;
                        self.stop(ago, out);
                        self.start(target, f.rms_db, ago, out);
                    }
                } else {
                    self.pending = None;
                }

                if self.cfg.pitch_bend {
                    if let Some(cur) = self.current {
                        let semis = (m - cur as f32).clamp(-self.cfg.bend_range, self.cfg.bend_range);
                        if (semis - self.last_bend).abs() > 0.02 {
                            self.last_bend = semis;
                            out(NoteEvent::Bend { semitones: semis });
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(midi: f32) -> PitchFrame {
        PitchFrame { voiced: true, f0: 0.0, midi, confidence: 0.9, rms_db: -20.0, noise_db: -70.0 }
    }

    fn run(track: &[PitchFrame], cfg: NoteConfig) -> Vec<NoteEvent> {
        let mut nt = NoteTracker::new(0.004, cfg);
        let mut ev = Vec::new();
        for f in track {
            nt.push(f, &mut |e| ev.push(e));
        }
        ev
    }

    #[test]
    fn vibrato_does_not_flutter() {
        let track: Vec<_> = (0..500).map(|i| frame(69.0 + 0.45 * (i as f32 * 0.1).sin())).collect();
        let ev = run(&track, NoteConfig::default());
        assert_eq!(ev.len(), 1, "{ev:?}");
        assert!(matches!(ev[0], NoteEvent::On { note: 69, .. }));
    }

    #[test]
    fn note_change_and_release() {
        let mut track: Vec<_> = (0..100).map(|_| frame(60.0)).collect();
        track.extend((0..100).map(|_| frame(64.1)));
        track.extend((0..50).map(|_| PitchFrame::default()));
        let ev = run(&track, NoteConfig::default());
        let ons: Vec<u8> = ev.iter().filter_map(|e| if let NoteEvent::On { note, .. } = e { Some(*note) } else { None }).collect();
        assert_eq!(ons, vec![60, 64]);
        assert!(matches!(ev.last(), Some(NoteEvent::Off { note: 64, .. })));
    }

    #[test]
    fn short_glitch_is_ignored() {
        let mut track: Vec<_> = (0..100).map(|_| frame(60.0)).collect();
        track.extend((0..5).map(|_| frame(72.0))); // 20 ms octave glitch
        track.extend((0..100).map(|_| frame(60.0)));
        let ev = run(&track, NoteConfig::default());
        assert_eq!(ev.len(), 1, "{ev:?}");
    }
}
