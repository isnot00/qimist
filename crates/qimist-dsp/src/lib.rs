//! Qimist DSP core.
//!
//! Everything here is plain Rust with no plugin-framework dependency, so it can be
//! unit-tested and run offline over WAV files. All `process`/`push` paths are
//! allocation-free after construction and safe to call from the audio thread.
//!
//! Signal path for voice analysis:
//!
//! ```text
//! mic ─► FrontEnd (HPF 65 Hz, hum notches, LPF, decimate to ~16 kHz)
//!     ─► Denoiser (STFT spectral subtraction, adaptive noise profile)
//!     ─► Yin (pYIN multi-threshold candidates)
//!     ─► PitchHmm (fixed-lag Viterbi over pitch × voicing)
//!     ─► NoteTracker (hysteresis, onsets, scale snap) ─► note events
//!     └► KeyDetector (Krumhansl-Schmuckler over note pitch classes)
//! ```

pub mod aec;
pub mod arp;
pub mod autotune;
pub mod biquad;
pub mod chords;
pub mod denoise;
pub mod drums;
pub mod engine;
pub mod follow;
pub mod frontend;
pub mod fx;
pub mod hmm;
pub mod key;
pub mod looper;
pub mod notes;
pub mod pitch;
pub mod recorder;
pub mod scale;
pub mod synth;
pub mod testsig;
pub mod yin;

pub use engine::{EngineConfig, EngineEvent, VoiceEngine};
pub use key::{Key, KeyDetector, KeyEstimate};
pub use notes::{NoteConfig, NoteEvent};
pub use pitch::{LatencyMode, PitchConfig, PitchFrame, PitchTracker};
pub use recorder::{RecordedNote, Recorder};
pub use scale::{Scale, ScaleKind};

/// MIDI note number (float) from frequency in Hz.
#[inline]
pub fn hz_to_midi(hz: f32) -> f32 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

/// Frequency in Hz from MIDI note number (float).
#[inline]
pub fn midi_to_hz(midi: f32) -> f32 {
    440.0 * 2f32.powf((midi - 69.0) / 12.0)
}

pub const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// "A4", "C#3", ...
pub fn note_name(note: u8) -> String {
    format!("{}{}", NOTE_NAMES[(note % 12) as usize], note as i32 / 12 - 1)
}

#[inline]
pub(crate) fn db(x: f32) -> f32 {
    20.0 * x.max(1e-10).log10()
}
