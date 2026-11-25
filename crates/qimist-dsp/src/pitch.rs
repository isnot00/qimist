//! Streaming pitch tracker: front end → denoiser → pYIN → HMM.

use crate::denoise::Denoiser;
use crate::frontend::FrontEnd;
use crate::hmm::PitchHmm;
use crate::yin::{Candidate, Yin, MAX_CANDIDATES};
use crate::{db, hz_to_midi};

/// Analysis hop at ~16 kHz: 64 samples = 4 ms.
pub const HOP: usize = 64;
/// Max per-frame pitch jump allowed by the HMM, in semitones.
const MAX_JUMP_SEMITONES: f32 = 6.0;
/// Noise floor may only rise this fast (dB per second).
const FLOOR_RISE_DB_S: f32 = 1.0;
/// Level history for the crowd floor: 1024 frames ≈ 4 s.
const LEVEL_HISTORY: usize = 1024;
/// The learned voice level fades this fast while you're not singing (dB/s).
const VOICE_DECAY_DB_S: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LatencyMode {
    /// ~35 ms. Snappy for live playing; less reliable below ~80 Hz.
    Fast,
    /// ~55 ms. Default.
    Balanced,
    /// ~90 ms. Most robust; best for recording.
    Accurate,
}

impl LatencyMode {
    fn window(self) -> usize {
        match self {
            LatencyMode::Fast => 256,
            LatencyMode::Balanced => 384,
            LatencyMode::Accurate => 512,
        }
    }

    fn lag(self) -> usize {
        match self {
            LatencyMode::Fast => 2,
            LatencyMode::Balanced => 4,
            LatencyMode::Accurate => 10,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PitchConfig {
    pub fmin: f32,
    pub fmax: f32,
    pub latency: LatencyMode,
    /// 0..1 spectral denoise strength (0 = off, also removes its latency).
    pub denoise: f32,
    /// 0..1: higher accepts quieter / breathier voice.
    pub sensitivity: f32,
    /// Absolute gate in dBFS; frames below are always unvoiced.
    pub gate_db: f32,
    pub hum_notch: bool,
    /// 0..1 crowd rejection. Learns how loud *you* sing (you're closest to the
    /// mic) and ignores quieter pitched sound — other people talking or singing.
    /// 0 = off.
    pub focus: f32,
}

impl Default for PitchConfig {
    fn default() -> Self {
        PitchConfig {
            fmin: 65.0,
            fmax: 1100.0,
            latency: LatencyMode::Balanced,
            denoise: 0.6,
            sensitivity: 0.7,
            gate_db: -55.0,
            hum_notch: true,
            focus: 0.0,
        }
    }
}

impl PitchConfig {
    /// Settings tuned for small built-in laptop mics: stronger denoise, more
    /// conservative voicing, higher floor (laptop mics barely capture < 100 Hz,
    /// but YIN still finds the period from the harmonics).
    pub fn laptop_mic() -> Self {
        PitchConfig { denoise: 0.75, sensitivity: 0.6, gate_db: -50.0, focus: 0.3, ..Default::default() }
    }

    /// True if a change requires rebuilding the analysers (not just retuning).
    pub fn needs_rebuild(&self, other: &PitchConfig) -> bool {
        self.fmin != other.fmin || self.fmax != other.fmax || self.latency != other.latency
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PitchFrame {
    pub voiced: bool,
    /// Detected fundamental in Hz (0 when unvoiced).
    pub f0: f32,
    /// Float MIDI note (0 when unvoiced).
    pub midi: f32,
    /// 0..1 periodicity / voicing probability.
    pub confidence: f32,
    pub rms_db: f32,
    pub noise_db: f32,
}

pub struct PitchTracker {
    cfg: PitchConfig,
    host_sr: f64,
    fe: FrontEnd,
    dn: Denoiser,
    yin: Yin,
    hmm: PitchHmm,
    ring: Vec<f32>,
    ring_pos: usize,
    filled: usize,
    frame: Vec<f32>,
    hop_count: usize,
    /// Per-frame data kept until the HMM decodes that frame.
    cands: Vec<([Candidate; MAX_CANDIDATES], usize, f32, f32)>,
    t: usize,
    noise_db: f32,
    floor_rise: f32,
    levels: Box<[f32; LEVEL_HISTORY]>,
    levels_scratch: Box<[f32; LEVEL_HISTORY]>,
    levels_pos: usize,
    levels_filled: usize,
    crowd_db: f32,
    voice_db: f32,
    voice_decay: f32,
}

impl PitchTracker {
    pub fn new(host_sr: f64, cfg: PitchConfig) -> Self {
        let fe = FrontEnd::new(host_sr);
        let sr = fe.out_sr();
        let yin = Yin::new(sr as f32, cfg.fmin, cfg.fmax, cfg.latency.window());
        let hmm = PitchHmm::new(cfg.fmin, cfg.fmax, cfg.latency.lag(), MAX_JUMP_SEMITONES);
        let frame_len = yin.frame_len();
        let lag = hmm.lag();
        let mut t = PitchTracker {
            cfg,
            host_sr,
            fe,
            dn: Denoiser::new(sr),
            yin,
            hmm,
            ring: vec![0.0; frame_len],
            ring_pos: 0,
            filled: 0,
            frame: vec![0.0; frame_len],
            hop_count: 0,
            cands: vec![([Candidate::default(); MAX_CANDIDATES], 0, -120.0, 0.0); lag + 1],
            t: 0,
            noise_db: -70.0,
            floor_rise: FLOOR_RISE_DB_S * HOP as f32 / sr as f32,
            levels: Box::new([-120.0; LEVEL_HISTORY]),
            levels_scratch: Box::new([-120.0; LEVEL_HISTORY]),
            levels_pos: 0,
            levels_filled: 0,
            crowd_db: -120.0,
            voice_db: -120.0,
            voice_decay: VOICE_DECAY_DB_S * HOP as f32 / sr as f32,
        };
        t.apply_tuning();
        t
    }

    fn apply_tuning(&mut self) {
        self.fe.set_hum_notch(self.cfg.hum_notch);
        self.dn.set_strength(self.cfg.denoise);
        self.yin.set_threshold_mean(0.1 + 0.1 * self.cfg.sensitivity);
    }

    pub fn config(&self) -> &PitchConfig {
        &self.cfg
    }

    /// Update settings. Range/latency changes rebuild the analysers (allocates:
    /// call from the audio thread only via a pre-built tracker swap, or accept a
    /// one-off allocation when the user changes these rarely-touched settings).
    pub fn set_config(&mut self, cfg: PitchConfig) {
        if cfg.needs_rebuild(&self.cfg) {
            *self = PitchTracker::new(self.host_sr, cfg);
        } else {
            self.cfg = cfg;
            self.apply_tuning();
        }
    }

    pub fn reset(&mut self) {
        *self = PitchTracker::new(self.host_sr, self.cfg);
    }

    /// Analysis frame period in seconds.
    pub fn frame_period(&self) -> f32 {
        (HOP * self.fe.decimation()) as f32 / self.host_sr as f32
    }

    /// Approximate delay between sound and its decoded frame, in host samples.
    pub fn latency_samples(&self) -> usize {
        let dn = if self.cfg.denoise > 0.001 { Denoiser::latency() } else { 0 };
        let analysis = dn + self.yin.frame_len() / 2 + self.hmm.lag() * HOP;
        analysis * self.fe.decimation()
    }

    /// Feed one host-rate sample. Returns a decoded frame every hop (4 ms).
    #[inline]
    pub fn push(&mut self, x: f32) -> Option<PitchFrame> {
        let y = self.fe.push(x)?;
        let y = if self.cfg.denoise > 0.001 { self.dn.push(y) } else { y };
        self.ring[self.ring_pos] = y;
        self.ring_pos = (self.ring_pos + 1) % self.ring.len();
        self.filled = (self.filled + 1).min(self.ring.len());
        self.hop_count += 1;
        if self.hop_count < HOP || self.filled < self.ring.len() {
            return None;
        }
        self.hop_count = 0;
        Some(self.analyze())
    }

    /// Allowed dB below the learned voice level before sound counts as background.
    fn focus_range_db(&self) -> f32 {
        30.0 - 16.0 * self.cfg.focus
    }

    /// Learned singing level in dBFS (−120 until you've sung something).
    pub fn voice_level_db(&self) -> f32 {
        self.voice_db
    }

    /// Effective floor in focus mode: the loudest of the steady-noise floor,
    /// the crowd floor (10th percentile of the last ~4 s of level — chatter
    /// fills the gaps between your notes), and "voice level − range". Never
    /// above the voice-relative floor, so a long sung passage can't gate itself.
    fn focus_floor(&mut self, rms_db: f32) -> f32 {
        self.levels[self.levels_pos] = rms_db;
        self.levels_pos = (self.levels_pos + 1) % LEVEL_HISTORY;
        self.levels_filled = (self.levels_filled + 1).min(LEVEL_HISTORY);
        // Recompute often while the history is still filling, then every 16 frames.
        if self.levels_filled < LEVEL_HISTORY || self.t % 16 == 0 {
            let n = self.levels_filled;
            let filled = if n < LEVEL_HISTORY { &self.levels[..n] } else { &self.levels[..] };
            self.levels_scratch[..n].copy_from_slice(filled);
            let k = n / 10;
            self.levels_scratch[..n].select_nth_unstable_by(k, f32::total_cmp);
            self.crowd_db = self.levels_scratch[k];
        }
        let voice_floor = if self.voice_db > -100.0 { self.voice_db - self.focus_range_db() } else { -120.0 };
        let crowd = if voice_floor > -100.0 { self.crowd_db.min(voice_floor) } else { self.crowd_db };
        self.noise_db.max(crowd).max(voice_floor)
    }

    fn analyze(&mut self) -> PitchFrame {
        // Unroll ring into a contiguous oldest→newest frame.
        let n = self.ring.len();
        let (a, b) = self.ring.split_at(self.ring_pos);
        self.frame[..n - self.ring_pos].copy_from_slice(b);
        self.frame[n - self.ring_pos..].copy_from_slice(a);

        // Level of the newest part of the frame.
        let recent = &self.frame[n - HOP * 2..];
        let rms = (recent.iter().map(|v| v * v).sum::<f32>() / recent.len() as f32).sqrt();
        let rms_db = db(rms);

        // Adaptive noise floor: falls fast, rises slowly.
        if rms_db < self.noise_db {
            self.noise_db += (rms_db - self.noise_db) * 0.2;
        } else {
            self.noise_db = (self.noise_db + self.floor_rise).min(rms_db);
        }
        self.noise_db = self.noise_db.clamp(-120.0, -30.0);
        let margin = 12.0 - 9.0 * self.cfg.sensitivity;

        let slot = self.t % self.cands.len();
        let entry = &mut self.cands[slot];
        entry.1 = self.yin.analyze(&self.frame, &mut entry.0);
        let (cands, count) = (entry.0, entry.1);
        let periodicity: f32 = cands[..count].iter().map(|c| c.prob).sum();

        let floor = if self.cfg.focus > 0.0 { self.focus_floor(rms_db) } else { self.noise_db };
        let mut energy_ok = rms_db > self.cfg.gate_db && rms_db > floor + margin;
        if self.cfg.focus > 0.0 {
            // Overlapping background voices blur the periodicity; demand a cleaner pitch.
            energy_ok &= periodicity > 0.1 + 0.15 * self.cfg.focus;
            // Learn the singer's level from loud, clean frames; fade slowly otherwise.
            if energy_ok && periodicity > 0.7 && rms_db > self.voice_db {
                self.voice_db += (rms_db - self.voice_db) * 0.3;
            } else {
                self.voice_db = (self.voice_db - self.voice_decay).max(-120.0);
            }
        }
        let entry = &mut self.cands[slot];
        entry.2 = rms_db;
        entry.3 = floor;
        self.t += 1;

        let Some(state) = self.hmm.step(&cands[..count], energy_ok) else {
            return PitchFrame { rms_db, noise_db: self.noise_db, ..Default::default() };
        };

        // Data for the decoded (lagged) frame.
        let lag_slot = (self.t - 1 - self.hmm.lag()) % self.cands.len();
        let (lc, ln, l_rms, l_noise) = self.cands[lag_slot];
        let voicing: f32 = lc[..ln].iter().map(|c| c.prob).sum::<f32>().min(1.0);
        if !state.voiced {
            return PitchFrame { voiced: false, confidence: voicing, rms_db: l_rms, noise_db: l_noise, ..Default::default() };
        }
        // Prefer the exact interpolated candidate near the decoded bin.
        let f0 = lc[..ln]
            .iter()
            .filter(|c| self.hmm.bin_of(c.freq).is_some_and(|b| b.abs_diff(state.bin) <= 1))
            .max_by(|a, b| a.prob.total_cmp(&b.prob))
            .map(|c| c.freq)
            .unwrap_or_else(|| self.hmm.bin_freq(state.bin));
        PitchFrame { voiced: true, f0, midi: hz_to_midi(f0), confidence: voicing, rms_db: l_rms, noise_db: l_noise }
    }
}
