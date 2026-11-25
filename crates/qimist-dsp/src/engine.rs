//! Voice → notes engine: pitch tracking, note segmentation and key detection
//! combined behind one sample-by-sample API.

use crate::key::{KeyDetector, KeyEstimate};
use crate::notes::{NoteConfig, NoteEvent, NoteTracker};
use crate::pitch::{PitchConfig, PitchFrame, PitchTracker};
use crate::scale::Scale;

/// Key confidence needed before auto scale-snap uses the detected key.
const AUTO_SNAP_MIN_CONFIDENCE: f32 = 0.35;
const KEY_ESTIMATE_EVERY_FRAMES: u32 = 25;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EngineConfig {
    pub pitch: PitchConfig,
    pub notes: NoteConfig,
    pub snap_enabled: bool,
    /// `Some` = key locked by the user; `None` = follow the detected key.
    pub locked_scale: Option<Scale>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig { pitch: PitchConfig::default(), notes: NoteConfig::default(), snap_enabled: false, locked_scale: None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EngineEvent {
    /// `delay_samples`: how long ago (host samples) the note really started —
    /// lets the recorder place notes exactly despite analysis latency.
    NoteOn { note: u8, velocity: u8, delay_samples: u32 },
    NoteOff { note: u8, delay_samples: u32 },
    PitchBend { semitones: f32 },
}

pub struct VoiceEngine {
    cfg: EngineConfig,
    pitch: PitchTracker,
    notes: NoteTracker,
    key: KeyDetector,
    key_est: Option<KeyEstimate>,
    last: PitchFrame,
    frames: u32,
    frame_period: f32,
    frame_samples: f32,
}

impl VoiceEngine {
    pub fn new(sr: f64, cfg: EngineConfig) -> Self {
        let pitch = PitchTracker::new(sr, cfg.pitch);
        let frame_period = pitch.frame_period();
        VoiceEngine {
            cfg,
            notes: NoteTracker::new(frame_period, cfg.notes),
            pitch,
            key: KeyDetector::default(),
            key_est: None,
            last: PitchFrame::default(),
            frames: 0,
            frame_period,
            frame_samples: frame_period * sr as f32,
        }
    }

    pub fn config(&self) -> &EngineConfig {
        &self.cfg
    }

    pub fn set_config(&mut self, cfg: EngineConfig) {
        if cfg == self.cfg {
            return;
        }
        if cfg.pitch != self.cfg.pitch {
            self.pitch.set_config(cfg.pitch);
        }
        self.cfg = cfg;
        // `snap` is managed per frame, keep whatever is active.
        let snap = self.notes.config().snap;
        self.notes.set_config(NoteConfig { snap, ..cfg.notes });
    }

    pub fn last_frame(&self) -> &PitchFrame {
        &self.last
    }

    pub fn key_estimate(&self) -> Option<&KeyEstimate> {
        self.key_est.as_ref()
    }

    pub fn clear_key(&mut self) {
        self.key.clear();
        self.key_est = None;
    }

    pub fn sounding_note(&self) -> Option<u8> {
        self.notes.sounding()
    }

    pub fn smoothed_midi(&self) -> f32 {
        self.notes.smoothed_midi()
    }

    /// Scale used for snapping right now (locked, or detected and confident).
    pub fn active_scale(&self) -> Option<Scale> {
        self.cfg.locked_scale.or_else(|| {
            self.key_est.filter(|k| k.confidence >= AUTO_SNAP_MIN_CONFIDENCE).map(|k| k.key.scale())
        })
    }

    /// Singing level learned by focus mode (dBFS, −120 = not learned yet).
    pub fn voice_level_db(&self) -> f32 {
        self.pitch.voice_level_db()
    }

    pub fn latency_samples(&self) -> usize {
        self.pitch.latency_samples()
    }

    pub fn frame_period(&self) -> f32 {
        self.frame_period
    }

    pub fn flush(&mut self, out: &mut impl FnMut(EngineEvent)) {
        let fs = self.frame_samples;
        self.notes.flush(&mut |e| emit(e, fs, 0, out));
    }

    pub fn reset(&mut self, out: &mut impl FnMut(EngineEvent)) {
        self.flush(out);
        self.pitch.reset();
        self.last = PitchFrame::default();
    }

    /// Feed one mono host-rate sample. Returns the analysis frame when one
    /// completed during this sample.
    #[inline]
    pub fn push(&mut self, x: f32, out: &mut impl FnMut(EngineEvent)) -> Option<PitchFrame> {
        let frame = self.pitch.push(x)?;
        self.last = frame;

        let snap = if self.cfg.snap_enabled { self.active_scale() } else { None };
        if self.notes.config().snap != snap {
            let c = NoteConfig { snap, ..*self.notes.config() };
            self.notes.set_config(c);
        }

        let fs = self.frame_samples;
        let base = self.pitch.latency_samples() as u32;
        self.notes.push(&frame, &mut |e| emit(e, fs, base, out));

        // Key detection uses the detected (untransposed, unsnapped) pitch.
        let pc = if frame.voiced { Some(frame.midi.round().clamp(0.0, 127.0) as u8) } else { None };
        self.key.update(self.frame_period, pc, frame.confidence);
        self.frames += 1;
        if self.frames % KEY_ESTIMATE_EVERY_FRAMES == 0 {
            self.key_est = self.key.estimate();
        }
        Some(frame)
    }
}

fn emit(e: NoteEvent, frame_samples: f32, base: u32, out: &mut impl FnMut(EngineEvent)) {
    let delay = |frames: u32| base + (frames as f32 * frame_samples) as u32;
    out(match e {
        NoteEvent::On { note, velocity, frames_ago } => EngineEvent::NoteOn { note, velocity, delay_samples: delay(frames_ago) },
        NoteEvent::Off { note, frames_ago } => EngineEvent::NoteOff { note, delay_samples: delay(frames_ago) },
        NoteEvent::Bend { semitones } => EngineEvent::PitchBend { semitones },
    });
}
