//! State shared between the audio thread (writer) and the editor (reader).
//! Everything the GUI reads every frame is an atomic; the recorder sits behind
//! a mutex the audio thread only ever `try_lock`s.

use nih_plug::prelude::AtomicF32;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering::Relaxed};
use qimist_dsp::drums::Sample;
use qimist_dsp::looper::{LoopCmd, LoopNote};
use qimist_dsp::{KeyEstimate, PitchFrame, Recorder, Scale, ScaleKind};

/// Pitch history length: 3 s at one sample every 4 frames (~16 ms).
pub const HISTORY: usize = 192;
pub const HISTORY_DECIMATE: u32 = 4;
/// Drum hit timeline length.
pub const HITS: usize = 64;
/// Hits recorded per TRAIN press.
pub const TRAIN_HITS: u32 = 6;

/// A hand-edited loop layer waiting for the audio thread. The GUI owns the
/// Vec (the audio thread only copies out of it), so nothing is freed there.
#[derive(Default)]
pub struct LoopEdit {
    pub scene: usize,
    pub layer: usize,
    pub notes: Vec<LoopNote>,
    pub pending: bool,
}

pub struct Shared {
    // Live detector values.
    pub voiced: AtomicBool,
    pub f0: AtomicF32,
    pub midi: AtomicF32,
    pub confidence: AtomicF32,
    pub rms_db: AtomicF32,
    pub noise_db: AtomicF32,
    pub input_peak: AtomicF32,
    pub sounding: AtomicI32,
    pub velocity: AtomicI32,
    /// Spread of recent pitch around its mean, in cents (vibrato depth / wobble).
    pub wobble_cents: AtomicF32,
    pub stability: AtomicF32,
    pub latency_ms: AtomicF32,
    /// Singing level learned by focus mode (dBFS, −120 = not learned).
    pub voice_db: AtomicF32,

    // Key detection.
    pub key_root: AtomicI32, // -1 = not enough material yet
    pub key_minor: AtomicBool,
    pub key_conf: AtomicF32,
    pub alt: [(AtomicI32, AtomicBool, AtomicF32); 2],
    pub histogram: [AtomicF32; 12],
    /// Active snap scale as `root * 16 + kind index`, -1 = none.
    pub active_scale: AtomicI32,

    // Pitch history ring (float MIDI, NaN = unvoiced).
    pub history: [AtomicF32; HISTORY],
    pub history_pos: AtomicUsize,

    // Recording.
    pub recording: AtomicBool,
    pub rec_time: AtomicF32,
    pub recorder: Mutex<Recorder>,
    pub clear_request: AtomicBool,
    pub clear_key_request: AtomicBool,
    pub host_bpm: AtomicF32,
    pub notes_sent: AtomicU32,

    // Drums.
    /// Seconds since the plugin started processing (timeline clock).
    pub clock: AtomicF32,
    /// Clock time of the last hit per class (for pad flashes), and its velocity.
    pub last_hit: [AtomicF32; 3],
    pub last_vel: [AtomicU32; 3],
    /// Ring of recent hits: time + class (NaN time = empty).
    pub hit_time: [AtomicF32; HITS],
    pub hit_class: [AtomicU32; HITS],
    /// Per hit: body brightness (Hz) and voicing (0..1) for the hit map.
    pub hit_bright: [AtomicF32; HITS],
    pub hit_voicing: [AtomicF32; HITS],
    pub hit_pos: AtomicUsize,
    /// Class being trained (-1 = none) and hits still to capture.
    pub train_class: AtomicI32,
    pub train_left: AtomicU32,

    // Phase 5.
    /// Note the arpeggiator is playing (-1 = none).
    pub arp_note: AtomicI32,
    /// Auto-tune target note (-1 = none) and the correction being applied, in cents.
    pub at_note: AtomicI32,
    pub at_cents: AtomicF32,

    // Phase 6.
    pub sample_rate: AtomicF32,
    /// Drum samples handed to the audio thread; bump `sample_version` after changing.
    pub drum_sample_slots: Mutex<[Option<Sample>; 3]>,
    pub sample_version: AtomicU32,
    /// Every sample ever loaded, so dropping one on the audio thread never frees memory.
    pub sample_cache: Mutex<Vec<Sample>>,
    pub sample_status: Mutex<[String; 3]>,
    /// Output peak (linear) and limiter gain reduction (dB).
    pub out_peak: AtomicF32,
    pub limiter_gr: AtomicF32,
    /// Singer's loudness 0..1 (drives VOICE DYNAMICS and the expression CC).
    pub expression: AtomicF32,
    /// Tuned-voice delay (auto-tune / harmony), ms.
    pub monitor_ms: AtomicF32,
    /// Chord pads: the pad the mouse holds (−1 none), GUI → audio.
    pub chord_pad: AtomicI32,
    /// Sounding chord's root and a change waiting for the beat (−1 none).
    pub chord_root: AtomicI32,
    pub chord_pending: AtomicI32,
    /// Sounding chord notes (−1 = unused slot).
    pub chord_notes: [AtomicI32; 6],
    /// Loop station: commands GUI → audio, and its state for the GUI.
    pub loop_cmds: Mutex<Vec<LoopCmd>>,
    /// A layer edited in the piano roll, GUI → audio (the GUI owns the Vec).
    pub loop_edit: Mutex<LoopEdit>,
    /// Loop position in beats (negative during the count-in).
    pub loop_pos: AtomicF32,
    /// 0 stopped, 1 waiting/count-in, 2 playing, 3 recording.
    pub loop_state: AtomicI32,
    /// Layer being recorded (or armed), −1 none.
    pub loop_layer: AtomicI32,
    /// Scene playing, scene queued for the end of the pass (−1 none), song step (−1 none).
    pub loop_scene: AtomicI32,
    pub loop_next_scene: AtomicI32,
    pub loop_song_step: AtomicI32,
    /// Running as the standalone app (tempo comes from our own TEMPO).
    pub standalone: AtomicBool,
    /// Speaker echo being removed from the mic, dB.
    pub echo_db: AtomicF32,
}

impl Default for Shared {
    fn default() -> Self {
        Shared {
            voiced: AtomicBool::new(false),
            f0: AtomicF32::new(0.0),
            midi: AtomicF32::new(0.0),
            confidence: AtomicF32::new(0.0),
            rms_db: AtomicF32::new(-120.0),
            noise_db: AtomicF32::new(-120.0),
            input_peak: AtomicF32::new(0.0),
            sounding: AtomicI32::new(-1),
            velocity: AtomicI32::new(0),
            wobble_cents: AtomicF32::new(0.0),
            stability: AtomicF32::new(0.0),
            latency_ms: AtomicF32::new(0.0),
            voice_db: AtomicF32::new(-120.0),
            key_root: AtomicI32::new(-1),
            key_minor: AtomicBool::new(false),
            key_conf: AtomicF32::new(0.0),
            alt: std::array::from_fn(|_| (AtomicI32::new(-1), AtomicBool::new(false), AtomicF32::new(0.0))),
            histogram: std::array::from_fn(|_| AtomicF32::new(0.0)),
            active_scale: AtomicI32::new(-1),
            history: std::array::from_fn(|_| AtomicF32::new(f32::NAN)),
            history_pos: AtomicUsize::new(0),
            recording: AtomicBool::new(false),
            rec_time: AtomicF32::new(0.0),
            recorder: Mutex::new(Recorder::default()),
            clear_request: AtomicBool::new(false),
            clear_key_request: AtomicBool::new(false),
            host_bpm: AtomicF32::new(120.0),
            notes_sent: AtomicU32::new(0),
            clock: AtomicF32::new(0.0),
            last_hit: std::array::from_fn(|_| AtomicF32::new(-100.0)),
            last_vel: std::array::from_fn(|_| AtomicU32::new(0)),
            hit_time: std::array::from_fn(|_| AtomicF32::new(f32::NAN)),
            hit_class: std::array::from_fn(|_| AtomicU32::new(0)),
            hit_bright: std::array::from_fn(|_| AtomicF32::new(0.0)),
            hit_voicing: std::array::from_fn(|_| AtomicF32::new(0.0)),
            hit_pos: AtomicUsize::new(0),
            train_class: AtomicI32::new(-1),
            train_left: AtomicU32::new(0),
            arp_note: AtomicI32::new(-1),
            at_note: AtomicI32::new(-1),
            at_cents: AtomicF32::new(0.0),
            sample_rate: AtomicF32::new(48_000.0),
            drum_sample_slots: Mutex::new([None, None, None]),
            sample_version: AtomicU32::new(0),
            sample_cache: Mutex::new(Vec::new()),
            sample_status: Mutex::new(Default::default()),
            out_peak: AtomicF32::new(0.0),
            limiter_gr: AtomicF32::new(0.0),
            expression: AtomicF32::new(0.0),
            monitor_ms: AtomicF32::new(0.0),
            chord_pad: AtomicI32::new(-1),
            chord_root: AtomicI32::new(-1),
            chord_pending: AtomicI32::new(-1),
            chord_notes: std::array::from_fn(|_| AtomicI32::new(-1)),
            loop_cmds: Mutex::new(Vec::with_capacity(32)),
            loop_edit: Mutex::new(LoopEdit::default()),
            loop_pos: AtomicF32::new(0.0),
            loop_state: AtomicI32::new(0),
            loop_layer: AtomicI32::new(-1),
            loop_scene: AtomicI32::new(0),
            loop_next_scene: AtomicI32::new(-1),
            loop_song_step: AtomicI32::new(-1),
            standalone: AtomicBool::new(false),
            echo_db: AtomicF32::new(0.0),
        }
    }
}

impl Shared {
    pub fn publish_frame(&self, f: &PitchFrame) {
        self.voiced.store(f.voiced, Relaxed);
        if f.voiced {
            self.f0.store(f.f0, Relaxed);
            self.midi.store(f.midi, Relaxed);
        }
        self.confidence.store(f.confidence, Relaxed);
        self.rms_db.store(f.rms_db, Relaxed);
        self.noise_db.store(f.noise_db, Relaxed);
    }

    pub fn push_hit(&self, class: usize, vel: u8, t: f32, bright: f32, voicing: f32) {
        self.last_hit[class].store(t, Relaxed);
        self.last_vel[class].store(vel as u32, Relaxed);
        let p = self.hit_pos.load(Relaxed);
        self.hit_class[p % HITS].store(class as u32, Relaxed);
        self.hit_bright[p % HITS].store(bright, Relaxed);
        self.hit_voicing[p % HITS].store(voicing, Relaxed);
        self.hit_time[p % HITS].store(t, Relaxed);
        self.hit_pos.store(p.wrapping_add(1), Relaxed);
    }

    /// Recent hits as (time, class), any order.
    pub fn hits(&self) -> impl Iterator<Item = (f32, usize)> + '_ {
        self.hits_detailed().map(|(t, c, _, _)| (t, c))
    }

    /// Recent hits as (time, class, brightness Hz, voicing), any order.
    pub fn hits_detailed(&self) -> impl Iterator<Item = (f32, usize, f32, f32)> + '_ {
        (0..HITS).filter_map(|i| {
            let t = self.hit_time[i].load(Relaxed);
            (!t.is_nan()).then(|| (t, self.hit_class[i].load(Relaxed) as usize, self.hit_bright[i].load(Relaxed), self.hit_voicing[i].load(Relaxed)))
        })
    }

    pub fn push_history(&self, midi: f32) {
        let p = self.history_pos.load(Relaxed);
        self.history[p % HISTORY].store(midi, Relaxed);
        self.history_pos.store(p.wrapping_add(1), Relaxed);
    }

    /// Oldest → newest.
    pub fn history_snapshot(&self) -> [f32; HISTORY] {
        let p = self.history_pos.load(Relaxed);
        std::array::from_fn(|i| self.history[(p + i) % HISTORY].load(Relaxed))
    }

    pub fn publish_key(&self, k: Option<&KeyEstimate>, active: Option<Scale>) {
        match k {
            Some(k) => {
                self.key_root.store(k.key.root as i32, Relaxed);
                self.key_minor.store(k.key.minor, Relaxed);
                self.key_conf.store(k.confidence, Relaxed);
                for (slot, (key, c)) in self.alt.iter().zip(k.alternatives) {
                    slot.0.store(key.root as i32, Relaxed);
                    slot.1.store(key.minor, Relaxed);
                    slot.2.store(c, Relaxed);
                }
                for (h, v) in self.histogram.iter().zip(k.histogram) {
                    h.store(v, Relaxed);
                }
            }
            None => self.key_root.store(-1, Relaxed),
        }
        let code = active
            .map(|s| s.root as i32 * 16 + ScaleKind::ALL.iter().position(|&k| k == s.kind).unwrap_or(0) as i32)
            .unwrap_or(-1);
        self.active_scale.store(code, Relaxed);
    }

    pub fn active_scale(&self) -> Option<Scale> {
        let c = self.active_scale.load(Relaxed);
        (c >= 0).then(|| Scale::new((c / 16) as u8, ScaleKind::ALL[(c % 16) as usize % ScaleKind::ALL.len()]))
    }
}
