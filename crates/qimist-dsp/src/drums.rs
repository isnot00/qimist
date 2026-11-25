//! Beatbox → drums.
//!
//! 1. **Onset detection** on the raw mic signal: a fast peak envelope jumping
//!    well above a slow average (and above the gate) marks a hit; a refractory
//!    period stops one "ts" from firing twice.
//! 2. **Features** from a ~21 ms window starting just before the hit: energy
//!    share in five bands, spectral centroid, flatness and zero-crossing rate.
//!    All are level-independent, so loud and soft hits classify the same.
//! 3. **Classification**: built-in rules work out of the box ("boom" is dark,
//!    "ts" is bright, "pf/ka" sits in between). After the user trains a few
//!    examples per sound, a k-nearest-neighbour classifier on standardised
//!    features takes over — it learns *your* mouth and *your* mic.

use crate::biquad::{Biquad, Coeffs};
use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const N_FEATURES: usize = 12;
pub const MAX_EXAMPLES_PER_CLASS: usize = 24;
/// Attack (≈ 10 ms incl. pre-roll) and body (≈ 21 ms) segments at 48 kHz, scaled for other rates.
const EARLY: f32 = 512.0;
const LATE: f32 = 1024.0;
const PRE_ROLL: usize = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DrumClass {
    Kick,
    Snare,
    HiHat,
}

impl DrumClass {
    pub const ALL: [DrumClass; 3] = [DrumClass::Kick, DrumClass::Snare, DrumClass::HiHat];

    /// General MIDI drum note.
    pub fn gm_note(self) -> u8 {
        match self {
            DrumClass::Kick => 36,
            DrumClass::Snare => 38,
            DrumClass::HiHat => 42,
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            DrumClass::Kick => "KICK",
            DrumClass::Snare => "SNARE",
            DrumClass::HiHat => "HI-HAT",
        }
    }

    pub fn from_index(i: usize) -> Self {
        DrumClass::ALL[i.min(2)]
    }
}

pub type Features = [f32; N_FEATURES];

/// User training examples. Serializable so the plugin can save it with the project.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DrumModel {
    pub examples: Vec<(DrumClass, Features)>,
}

impl DrumModel {
    pub fn count(&self, class: DrumClass) -> usize {
        self.examples.iter().filter(|(c, _)| *c == class).count()
    }

    pub fn add(&mut self, class: DrumClass, f: Features) {
        if self.count(class) >= MAX_EXAMPLES_PER_CLASS {
            // Replace the oldest example of this class.
            if let Some(i) = self.examples.iter().position(|(c, _)| *c == class) {
                self.examples.remove(i);
            }
        }
        self.examples.push((class, f));
    }

    pub fn clear(&mut self) {
        self.examples.clear();
    }

    /// Trained = at least 2 examples for at least 2 classes.
    pub fn is_trained(&self) -> bool {
        DrumClass::ALL.iter().filter(|&&c| self.count(c) >= 2).count() >= 2
    }

    /// kNN (k = 3) on standardised features; falls back to the rules when the
    /// model isn't trained. Untrained classes can still be chosen by the rules
    /// if the hit is clearly unlike every trained class.
    pub fn classify(&self, f: &Features) -> DrumClass {
        if !self.is_trained() {
            return rule_classify(f);
        }
        // Per-feature standard deviation over all examples (floored).
        let n = self.examples.len() as f32;
        let mut mean = [0.0f32; N_FEATURES];
        for (_, e) in &self.examples {
            for i in 0..N_FEATURES {
                mean[i] += e[i] / n;
            }
        }
        let mut sd = [0.0f32; N_FEATURES];
        for (_, e) in &self.examples {
            for i in 0..N_FEATURES {
                sd[i] += (e[i] - mean[i]).powi(2) / n;
            }
        }
        sd.iter_mut().for_each(|s| *s = s.sqrt().max(0.05));

        let mut best = [(f32::MAX, DrumClass::Kick); 3];
        for (c, e) in &self.examples {
            let d: f32 = (0..N_FEATURES).map(|i| ((f[i] - e[i]) / sd[i]).powi(2)).sum();
            if d < best[2].0 {
                best[2] = (d, *c);
                best.sort_by(|a, b| a.0.total_cmp(&b.0));
            }
        }
        // Distance-weighted vote.
        let mut votes = [0.0f32; 3];
        for (d, c) in best.iter().filter(|b| b.0 < f32::MAX) {
            votes[c.index()] += 1.0 / (d + 0.1);
        }
        let winner = DrumClass::from_index((0..3).max_by(|&a, &b| votes[a].total_cmp(&votes[b])).unwrap());

        // If the nearest example is very far and the rules pick an untrained class, trust the rules.
        let rule = rule_classify(f);
        if best[0].0 > 40.0 && self.count(rule) == 0 {
            return rule;
        }
        winner
    }
}

/// Brightness (body centroid in Hz) and voicing (0..1): the two numbers the
/// GUI's hit map shows.
pub fn brightness_voicing(f: &Features) -> (f32, f32) {
    (2f32.powf(f[feat::LATE_CENTROID]), f[feat::VOICING])
}

/// Out-of-the-box classification. A hi-hat ("ts") is bright hiss; a kick
/// ("boom", "b") is voiced and dark; a snare ("pf", "k", "psh") is unvoiced,
/// mid-bright noise. Scores rather than hard cut-offs so one odd feature
/// (e.g. a laptop mic with no bass) doesn't flip the result.
pub fn rule_classify(f: &Features) -> DrumClass {
    let body = 2f32.powf(f[feat::LATE_CENTROID]);
    let attack = 2f32.powf(f[feat::EARLY_CENTROID]);
    let v = f[feat::VOICING];
    let high = f[feat::LATE_BANDS + 4];
    let low = 10.0 * (10f32.powf(f[feat::LATE_BANDS] / 10.0) + 10f32.powf(f[feat::LATE_BANDS + 1] / 10.0)).log10();
    let flat = f[feat::LATE_FLATNESS];

    if v < 0.55 && (body > 6000.0 || high > -3.0) {
        return DrumClass::HiHat;
    }
    let mut kick = 0.0;
    let mut snare = 0.0;
    kick += if v > 0.6 { 2.0 } else if v > 0.42 { 1.0 } else { 0.0 };
    snare += if v < 0.3 { 1.5 } else { 0.0 };
    kick += if body < 1100.0 { 1.5 } else if body < 1600.0 { 0.5 } else { 0.0 };
    snare += if body > 1800.0 { 1.0 } else { 0.0 };
    kick += if low > -5.0 { 1.0 } else { 0.0 };
    snare += if flat > -1.4 { 1.0 } else { 0.0 };
    kick += if attack < 1500.0 { 0.5 } else { 0.0 };
    snare += if attack > 2500.0 { 0.5 } else { 0.0 };
    if kick > snare {
        DrumClass::Kick
    } else {
        DrumClass::Snare
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrumHit {
    pub class: DrumClass,
    pub velocity: u8,
    pub features: Features,
    /// Samples between the actual onset and when the hit was reported.
    pub delay_samples: u32,
}

pub struct DrumDetector {
    sr: f32,
    hp: Biquad,
    fast: f32,
    slow: f32,
    fast_rel: f32,
    /// Peak envelope with a 30 ms release: smooth enough to ignore pitch-cycle ripple.
    held: f32,
    held_rel: f32,
    /// Slow peak envelope: 25 ms attack, 250 ms release. A hit is a jump of the
    /// fast envelope above it; a decaying sound never rises above its own peak.
    slow_att: f32,
    slow_rel: f32,
    refractory: u32,
    since_onset: u32,
    /// Re-armed once the envelope falls back toward the average (hysteresis).
    armed: bool,
    /// Loudest envelope since the last onset.
    hit_peak_db: f32,
    ring: Vec<f32>,
    ring_pos: usize,
    capture: Option<usize>,
    /// Attack segment (includes the pre-roll) and body segment lengths.
    early: Segment,
    late: Segment,
    peak: f32,
    dec: Vec<f32>,
    /// dB the fast envelope must jump above the slow one.
    pub threshold_db: f32,
    pub gate_db: f32,
}

/// One analysis segment with its own FFT.
struct Segment {
    len: usize,
    fft: Arc<dyn Fft<f32>>,
    buf: Vec<Complex32>,
    scratch: Vec<Complex32>,
    window: Vec<f32>,
}

struct SpecStats {
    /// dB share of energy in <150, 150–600, 600–2k, 2k–5k, >5k Hz.
    bands: [f32; 5],
    centroid: f32,
    flatness: f32,
    power: f32,
}

impl Segment {
    fn new(len: usize, planner: &mut FftPlanner<f32>) -> Self {
        let fft = planner.plan_fft_forward(len);
        let scratch = vec![Complex32::new(0.0, 0.0); fft.get_inplace_scratch_len()];
        Segment {
            len,
            fft,
            buf: vec![Complex32::new(0.0, 0.0); len],
            scratch,
            window: (0..len).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / len as f32).cos()).collect(),
        }
    }

    fn analyse(&mut self, x: impl Iterator<Item = f32>, sr: f32) -> SpecStats {
        let mut power = 0.0;
        for (i, v) in x.take(self.len).enumerate() {
            power += v * v;
            self.buf[i] = Complex32::new(v * self.window[i], 0.0);
        }
        self.fft.process_with_scratch(&mut self.buf, &mut self.scratch);
        let bin_hz = sr / self.len as f32;
        let edges = [150.0, 600.0, 2000.0, 5000.0];
        let mut bands = [0.0f32; 5];
        let (mut total, mut wsum, mut log_sum) = (0.0f32, 0.0f32, 0.0f32);
        let half = self.len / 2;
        for k in 1..half {
            let p = self.buf[k].norm_sqr() + 1e-12;
            let f = k as f32 * bin_hz;
            bands[edges.iter().position(|&e| f < e).unwrap_or(4)] += p;
            total += p;
            wsum += p * f;
            log_sum += p.ln();
        }
        let flatness = ((log_sum / (half - 1) as f32).exp() / (total / (half - 1) as f32)).clamp(1e-6, 1.0);
        SpecStats {
            bands: bands.map(|e| 10.0 * (e / total).max(1e-6).log10()),
            centroid: (wsum / total).max(20.0),
            flatness,
            power: power / self.len as f32,
        }
    }
}

/// Feature indices (see `DrumDetector::features`).
pub mod feat {
    pub const LATE_BANDS: usize = 0; // 5 values
    pub const LATE_CENTROID: usize = 5; // log2 Hz
    pub const LATE_FLATNESS: usize = 6; // log10
    pub const VOICING: usize = 7; // 0..1
    pub const EARLY_CENTROID: usize = 8; // log2 Hz
    pub const EARLY_HIGH: usize = 9; // dB share > 5 kHz
    pub const DECAY: usize = 10; // dB body vs attack
    pub const LATE_ZCR: usize = 11;
}

impl DrumDetector {
    pub fn new(sr: f32) -> Self {
        let scaled = |n: f32| ((n * sr / 48_000.0).round() as usize).next_power_of_two().clamp(128, 4096);
        let mut planner = FftPlanner::new();
        let early = Segment::new(scaled(EARLY), &mut planner);
        let late = Segment::new(scaled(LATE), &mut planner);
        let ring_len = early.len + late.len + 8;
        DrumDetector {
            sr,
            hp: Biquad::new(Coeffs::highpass(sr as f64, 30.0, 0.707)),
            fast: 0.0,
            slow: 0.0,
            fast_rel: (-1.0 / (0.004 * sr)).exp(),
            held: 0.0,
            held_rel: (-1.0 / (0.03 * sr)).exp(),
            slow_att: 1.0 - (-1.0 / (0.025 * sr)).exp(),
            slow_rel: (-1.0 / (0.25 * sr)).exp(),
            refractory: (0.07 * sr) as u32,
            since_onset: u32::MAX,
            armed: true,
            hit_peak_db: -120.0,
            ring: vec![0.0; ring_len],
            ring_pos: 0,
            capture: None,
            dec: Vec::with_capacity(late.len / 2),
            early,
            late,
            peak: 0.0,
            threshold_db: 9.0,
            gate_db: -45.0,
        }
    }

    fn pre_roll(&self) -> usize {
        (PRE_ROLL as f32 * self.sr / 48_000.0) as usize
    }

    /// Samples from the onset until the hit is classified.
    pub fn latency_samples(&self) -> u32 {
        (self.early.len + self.late.len - self.pre_roll()) as u32
    }

    /// 0..1 → onset threshold 15 dB (strict) … 5 dB (sensitive).
    pub fn set_sensitivity(&mut self, s: f32) {
        self.threshold_db = 15.0 - 10.0 * s.clamp(0.0, 1.0);
    }

    pub fn reset(&mut self) {
        self.fast = 0.0;
        self.held = 0.0;
        self.slow = 0.0;
        self.capture = None;
        self.since_onset = u32::MAX;
        self.armed = true;
        self.ring.fill(0.0);
    }

    /// Feed one sample. When a hit's analysis window completes, returns its
    /// features, velocity and the delay since the onset (classification is
    /// done by the caller with its model).
    #[inline]
    pub fn push(&mut self, x: f32) -> Option<(Features, u8, u32)> {
        let y = self.hp.process(x as f64) as f32;
        let n = self.ring.len();
        self.ring[self.ring_pos] = y;
        self.ring_pos = (self.ring_pos + 1) % n;

        let a = y.abs();
        self.fast = if a > self.fast { a } else { self.fast * self.fast_rel };
        self.held = if a > self.held { a } else { self.held * self.held_rel };
        self.slow = if a > self.slow { self.slow + (a - self.slow) * self.slow_att } else { self.slow * self.slow_rel };
        self.since_onset = self.since_onset.saturating_add(1);

        if let Some(left) = self.capture.as_mut() {
            self.peak = self.peak.max(a);
            *left -= 1;
            if *left == 0 {
                self.capture = None;
                let f = self.features();
                let vel_db = crate::db(self.peak);
                let vel = (((vel_db + 45.0) / 40.0).clamp(0.0, 1.0).powf(0.7) * 126.0 + 1.0) as u8;
                return Some((f, vel, self.latency_samples()));
            }
            return None;
        }

        let fast_db = crate::db(self.fast);
        let slow_db = crate::db(self.slow);
        if !self.armed {
            let held_db = crate::db(self.held);
            self.hit_peak_db = self.hit_peak_db.max(held_db);
            if held_db < self.hit_peak_db - 6.0 || held_db < self.gate_db {
                self.armed = true;
            }
        }
        if self.armed && self.since_onset > self.refractory && fast_db > self.gate_db && fast_db > slow_db + self.threshold_db {
            self.armed = false;
            self.hit_peak_db = fast_db;
            self.since_onset = 0;
            self.capture = Some(self.latency_samples() as usize);
            self.peak = a;
        }
        None
    }

    fn features(&mut self) -> Features {
        let n = self.ring.len();
        let total = self.early.len + self.late.len;
        let start = (self.ring_pos + n - total) % n;
        let ring = &self.ring;
        let at = move |i: usize| ring[(start + i) % n];
        let e = self.early.analyse((0..self.early.len).map(at), self.sr);
        let el = self.early.len;
        let l = self.late.analyse((el..total).map(at), self.sr);

        // Voicing: best normalised autocorrelation of the body for 60–400 Hz,
        // on a 2× decimated copy (cheap; voice harmonics survive a laptop mic's
        // missing bass, so this still tells "boom" from "pf").
        self.dec.clear();
        let mut zc = 0usize;
        let mut prev = 0.0f32;
        for i in el..total {
            let v = at(i);
            if (v >= 0.0) != (prev >= 0.0) {
                zc += 1;
            }
            prev = v;
            if (i - el) % 2 == 1 {
                self.dec.push(0.5 * (v + at(i - 1)));
            }
        }
        let dsr = self.sr / 2.0;
        let (lag_min, lag_max) = ((dsr / 400.0) as usize, ((dsr / 60.0) as usize).min(self.dec.len() / 2));
        let d = &self.dec;
        let mut voicing = 0.0f32;
        for lag in lag_min.max(1)..=lag_max {
            let m = d.len() - lag;
            let (mut xy, mut xx, mut yy) = (0.0f32, 0.0f32, 0.0f32);
            for i in 0..m {
                xy += d[i] * d[i + lag];
                xx += d[i] * d[i];
                yy += d[i + lag] * d[i + lag];
            }
            voicing = voicing.max(xy / (xx * yy).sqrt().max(1e-12));
        }

        let mut f = [0.0f32; N_FEATURES];
        f[feat::LATE_BANDS..feat::LATE_BANDS + 5].copy_from_slice(&l.bands);
        f[feat::LATE_CENTROID] = l.centroid.log2();
        f[feat::LATE_FLATNESS] = l.flatness.log10();
        f[feat::VOICING] = voicing.clamp(0.0, 1.0);
        f[feat::EARLY_CENTROID] = e.centroid.log2();
        f[feat::EARLY_HIGH] = e.bands[4];
        f[feat::DECAY] = 10.0 * (l.power.max(1e-12) / e.power.max(1e-12)).log10();
        f[feat::LATE_ZCR] = zc as f32 / self.late.len as f32 * 10.0;
        f
    }
}

// ───────────────────────────── built-in drum kit ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kit {
    /// Booming sine kick, long snare tail, metallic hats.
    Eight08,
    /// Short punchy kit.
    Tight,
    /// 808-ish, then crushed to 6 bits / 12 kHz and darkened.
    LoFi,
    /// Rounder kick with beater click, snare with body and rattle, airy hats.
    Acoustic,
    /// Long saturated 808 sub, crisp snare, very tight hats.
    Trap,
}

/// Synthesis recipe per kit.
#[derive(Clone, Copy)]
struct Recipe {
    k_start: f32,
    k_end: f32,
    k_decay: f32,
    k_click: f32,
    k_drive: f32,
    s_tone: f32,
    s_tone_decay: f32,
    s_noise_decay: f32,
    s_bp: f64,
    h_decay: f32,
    h_hp: f64,
}

impl Kit {
    fn recipe(self) -> Recipe {
        let base = Recipe {
            k_start: 160.0,
            k_end: 48.0,
            k_decay: 0.45,
            k_click: 0.5,
            k_drive: 0.0,
            s_tone: 185.0,
            s_tone_decay: 0.06,
            s_noise_decay: 0.22,
            s_bp: 2500.0,
            h_decay: 0.06,
            h_hp: 7000.0,
        };
        match self {
            Kit::Eight08 | Kit::LoFi => base,
            Kit::Tight => Recipe { k_start: 190.0, k_end: 62.0, k_decay: 0.17, s_noise_decay: 0.11, h_decay: 0.035, ..base },
            Kit::Acoustic => Recipe {
                k_start: 120.0,
                k_end: 58.0,
                k_decay: 0.22,
                k_click: 1.0,
                s_tone: 200.0,
                s_tone_decay: 0.08,
                s_noise_decay: 0.18,
                s_bp: 3200.0,
                h_decay: 0.05,
                h_hp: 6000.0,
                ..base
            },
            Kit::Trap => Recipe {
                k_start: 140.0,
                k_end: 42.0,
                k_decay: 0.9,
                k_drive: 1.0,
                s_tone: 220.0,
                s_noise_decay: 0.12,
                s_bp: 4200.0,
                h_decay: 0.028,
                h_hp: 8500.0,
                ..base
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KitSettings {
    pub kit: Kit,
    /// Semitones.
    pub tune: f32,
    /// Multiplier on all decay times (0.5..2).
    pub decay: f32,
    pub volume: f32,
    /// Per-pad level (kick, snare, hat).
    pub levels: [f32; 3],
}

impl Default for KitSettings {
    fn default() -> Self {
        KitSettings { kit: Kit::Eight08, tune: 0.0, decay: 1.0, volume: 0.8, levels: [1.0; 3] }
    }
}

#[derive(Clone, Copy, Default)]
struct OneShot {
    active: bool,
    t: f32,
    vel: f32,
    phase: f32,
}

/// A user sample: mono, already at the plugin's sample rate.
pub type Sample = Arc<Vec<f32>>;

pub struct DrumKit {
    sr: f32,
    s: KitSettings,
    shots: [OneShot; 3],
    hat_phases: [f32; 6],
    snare_bp: Biquad,
    hat_bp: Biquad,
    hat_hp: Biquad,
    lofi_lp: Biquad,
    rng: u32,
    hold: f32,
    hold_n: u32,
    samples: [Option<Sample>; 3],
    /// Fractional read position per pad when playing a sample.
    sample_pos: [f32; 3],
}

const HAT_FREQS: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];

impl DrumKit {
    pub fn new(sr: f32) -> Self {
        let srd = sr as f64;
        let r = Kit::Eight08.recipe();
        DrumKit {
            sr,
            s: KitSettings::default(),
            shots: [OneShot::default(); 3],
            hat_phases: [0.0; 6],
            snare_bp: Biquad::new(Coeffs::bandpass(srd, r.s_bp, 0.7)),
            hat_bp: Biquad::new(Coeffs::bandpass(srd, 10_000.0, 1.0)),
            hat_hp: Biquad::new(Coeffs::highpass(srd, r.h_hp, 0.7)),
            lofi_lp: Biquad::new(Coeffs::lowpass(srd, 6000.0, 0.7)),
            rng: 0xC0FF_EE11,
            hold: 0.0,
            hold_n: 0,
            samples: [None, None, None],
            sample_pos: [0.0; 3],
        }
    }

    pub fn set(&mut self, s: KitSettings) {
        if s.kit != self.s.kit {
            let r = s.kit.recipe();
            let srd = self.sr as f64;
            self.snare_bp = Biquad::new(Coeffs::bandpass(srd, r.s_bp, 0.7));
            self.hat_hp = Biquad::new(Coeffs::highpass(srd, r.h_hp, 0.7));
        }
        self.s = s;
    }

    /// Use a sample instead of the synthesized sound for one pad (`None` = built-in).
    pub fn set_sample(&mut self, class: DrumClass, sample: Option<Sample>) {
        self.samples[class.index()] = sample;
    }

    pub fn has_sample(&self, class: DrumClass) -> bool {
        self.samples[class.index()].is_some()
    }

    pub fn trigger(&mut self, class: DrumClass, velocity: u8) {
        let v = 0.2 + 0.8 * velocity as f32 / 127.0;
        self.shots[class.index()] = OneShot { active: true, t: 0.0, vel: v, phase: 0.0 };
        self.sample_pos[class.index()] = 0.0;
    }

    pub fn reset(&mut self) {
        self.shots.iter_mut().for_each(|s| s.active = false);
    }

    #[inline]
    fn noise(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32 * 2.0 - 1.0
    }

    /// Sample playback for pad `i`; `None` if that pad has no sample.
    #[inline]
    fn play_sample(&mut self, i: usize, rate: f32) -> Option<f32> {
        let smp = self.samples[i].as_ref()?;
        let shot = &mut self.shots[i];
        let p = self.sample_pos[i];
        let k = p as usize;
        if k + 1 >= smp.len() {
            shot.active = false;
            return Some(0.0);
        }
        let f = p - k as f32;
        let y = smp[k] * (1.0 - f) + smp[k + 1] * f;
        self.sample_pos[i] = p + rate;
        Some(y * shot.vel)
    }

    #[inline]
    pub fn process(&mut self) -> f32 {
        let dt = 1.0 / self.sr;
        let tune = 2f32.powf(self.s.tune / 12.0);
        let dm = self.s.decay.clamp(0.25, 4.0);
        let r = self.s.kit.recipe();
        let lv = self.s.levels;
        let mut synth_out = 0.0;
        let mut sample_out = 0.0;

        // Kick
        if self.shots[0].active {
            if let Some(y) = self.play_sample(0, tune) {
                sample_out += y * lv[0];
            } else {
                let t = self.shots[0].t;
                let f = (r.k_end + (r.k_start - r.k_end) * (-t / 0.035).exp()) * tune;
                self.shots[0].phase = (self.shots[0].phase + f * dt) % 1.0;
                let amp = (-t / (r.k_decay * dm)).exp();
                let click = if t < 0.002 { self.noise() * r.k_click * (1.0 - t / 0.002) } else { 0.0 };
                let mut body = (std::f32::consts::TAU * self.shots[0].phase).sin() * amp;
                if r.k_drive > 0.0 {
                    body = (body * (1.0 + 3.0 * r.k_drive)).tanh() * amp.sqrt();
                }
                synth_out += (body + click) * self.shots[0].vel * lv[0];
                self.shots[0].t += dt;
                self.shots[0].active = amp > 1e-4;
            }
        }
        // Snare
        if self.shots[1].active {
            if let Some(y) = self.play_sample(1, tune) {
                sample_out += y * lv[1];
            } else {
                let t = self.shots[1].t;
                self.shots[1].phase = (self.shots[1].phase + r.s_tone * tune * dt) % 1.0;
                let tone = (std::f32::consts::TAU * self.shots[1].phase).sin() * (-t / (r.s_tone_decay * dm)).exp() * 0.5;
                let n = self.noise();
                let noise = self.snare_bp.process(n as f64) as f32 * (-t / (r.s_noise_decay * dm)).exp() * 1.6;
                synth_out += (tone + noise) * self.shots[1].vel * 0.8 * lv[1];
                self.shots[1].t += dt;
                self.shots[1].active = t < 8.0 * r.s_noise_decay * dm;
            }
        }
        // Hi-hat
        if self.shots[2].active {
            if let Some(y) = self.play_sample(2, tune) {
                sample_out += y * lv[2];
            } else {
                let t = self.shots[2].t;
                let mut metal = 0.0;
                for (p, f) in self.hat_phases.iter_mut().zip(HAT_FREQS) {
                    *p = (*p + f * 2.0 * tune * dt) % 1.0;
                    metal += if *p < 0.5 { 1.0 } else { -1.0 };
                }
                let x = self.hat_bp.process(metal as f64 / 6.0);
                let x = self.hat_hp.process(x) as f32;
                synth_out += x * (-t / (r.h_decay * dm)).exp() * self.shots[2].vel * 1.8 * lv[2];
                self.shots[2].t += dt;
                self.shots[2].active = t < 8.0 * r.h_decay * dm;
            }
        }

        let mut out = synth_out + sample_out;
        if self.s.kit == Kit::LoFi {
            // Sample-rate reduction to ~12 kHz, 6-bit quantise, darken (samples too).
            let step = (self.sr / 12_000.0).round().max(1.0) as u32;
            if self.hold_n == 0 {
                self.hold = (out * 32.0).round() / 32.0;
            }
            self.hold_n = (self.hold_n + 1) % step;
            out = self.lofi_lp.process(self.hold as f64) as f32;
        }
        out * self.s.volume
    }
}

/// Decode a WAV file to mono at `sr` (linear resampling), peak-normalised to −1 dBFS
/// and trimmed of leading silence. Call this off the audio thread.
pub fn load_wav(path: &std::path::Path, sr: f32) -> Result<Vec<f32>, String> {
    let mut r = hound::WavReader::open(path).map_err(|e| e.to_string())?;
    let spec = r.spec();
    let ch = spec.channels.max(1) as usize;
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().map(|s| s.unwrap_or(0.0)).collect(),
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample.clamp(8, 32) - 1)) as f32;
            r.samples::<i32>().map(|s| s.unwrap_or(0) as f32 * scale).collect()
        }
    };
    let mono: Vec<f32> = raw.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect();
    if mono.is_empty() {
        return Err("empty file".into());
    }
    // At most 4 s of audio.
    let ratio = spec.sample_rate as f32 / sr;
    let n = ((mono.len() as f32 / ratio) as usize).min((4.0 * sr) as usize);
    let mut out: Vec<f32> = (0..n)
        .map(|i| {
            let p = i as f32 * ratio;
            let k = p as usize;
            let f = p - k as f32;
            mono[k.min(mono.len() - 1)] * (1.0 - f) + mono[(k + 1).min(mono.len() - 1)] * f
        })
        .collect();
    let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    if peak < 1e-5 {
        return Err("file is silent".into());
    }
    let start = out.iter().position(|x| x.abs() > peak * 0.02).unwrap_or(0);
    out.drain(..start.saturating_sub((0.001 * sr) as usize));
    let g = 0.89 / peak;
    out.iter_mut().for_each(|x| *x *= g);
    // 5 ms fade-out so cut-off samples don't click.
    let fade = ((0.005 * sr) as usize).min(out.len());
    let len = out.len();
    for i in 0..fade {
        out[len - 1 - i] *= i as f32 / fade as f32;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testsig;

    /// Synthetic beatbox sounds, modelled on mouth sounds rather than drums:
    /// kick = lip burst + voiced "oom"; snare = unvoiced "pf" or "k" burst +
    /// fricative; hi-hat = "t" click + "s" hiss.
    fn sound(class: DrumClass, sr: f32, seed: u32, vary: f32) -> Vec<f32> {
        let mut rng = seed.max(1);
        let mut rnd = move || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            rng as f32 / u32::MAX as f32 * 2.0 - 1.0
        };
        let srd = sr as f64;
        let v = 1.0 + 0.2 * vary;
        let n = (0.12 * sr) as usize;
        let mut burst_lp = Biquad::new(Coeffs::lowpass(srd, 700.0, 0.7));
        let mut f1 = Biquad::new(Coeffs::bandpass(srd, (330.0 * v) as f64, 3.0));
        let mut f2 = Biquad::new(Coeffs::bandpass(srd, (850.0 * v) as f64, 4.0));
        let k_style = seed % 2 == 0;
        let mut sn_burst = Biquad::new(Coeffs::bandpass(srd, if k_style { 2600.0 } else { 1200.0 } * v as f64, 1.2));
        let mut sn_fric = Biquad::new(Coeffs::bandpass(srd, if k_style { 3200.0 } else { 2200.0 } * v as f64, 0.7));
        let mut hat_hp = Biquad::new(Coeffs::highpass(srd, (6000.0 * v) as f64, 0.7));
        let mut hat_t = Biquad::new(Coeffs::highpass(srd, 3500.0, 0.7));
        let f0 = 105.0 * v;
        let mut phase = 0.0f32;
        (0..n)
            .map(|i| {
                let t = i as f32 / sr;
                match class {
                    DrumClass::Kick => {
                        let burst = burst_lp.process(rnd() as f64) as f32 * (-t / 0.004).exp() * 1.5;
                        phase = (phase + f0 * (1.0 + 0.3 * (-t / 0.02).exp()) / sr) % 1.0;
                        let glottal = 2.0 * phase - 1.0;
                        let env = (t / 0.006).min(1.0) * (-t / 0.07).exp();
                        let vowel = (f1.process(glottal as f64) * 1.0 + f2.process(glottal as f64) * 0.4) as f32 * env * 2.5;
                        0.5 * (burst + vowel)
                    }
                    DrumClass::Snare => {
                        let burst = sn_burst.process(rnd() as f64) as f32 * (-t / 0.004).exp() * 2.0;
                        let fric = sn_fric.process(rnd() as f64) as f32 * (t / 0.003).min(1.0) * (-t / 0.045).exp();
                        0.6 * (burst + fric)
                    }
                    DrumClass::HiHat => {
                        let click = hat_t.process(rnd() as f64) as f32 * (-t / 0.002).exp() * 1.5;
                        let hiss = hat_hp.process(rnd() as f64) as f32 * (t / 0.004).min(1.0) * (-t / 0.04).exp();
                        0.7 * (click + hiss)
                    }
                }
            })
            .collect()
    }

    fn pattern(sr: f32, classes: &[DrumClass], laptop: bool) -> Vec<f32> {
        let mut out = vec![0.0; (0.3 * sr) as usize];
        for (i, &c) in classes.iter().enumerate() {
            out.extend(sound(c, sr, 7 + i as u32 * 13, ((i * 7) % 5) as f32 / 4.0 - 0.5));
            out.extend(vec![0.0; (0.13 * sr) as usize]);
        }
        if laptop {
            // Tiny mic: no bass below ~250 Hz, dull top, hiss, a little room.
            let mut hp = [Biquad::new(Coeffs::highpass(sr as f64, 250.0, 0.7)), Biquad::new(Coeffs::highpass(sr as f64, 250.0, 0.7))];
            let mut lp = Biquad::new(Coeffs::lowpass(sr as f64, 9000.0, 0.7));
            let echo = (0.011 * sr) as usize;
            let dry = out.clone();
            for i in echo..out.len() {
                out[i] += 0.3 * dry[i - echo];
            }
            let mut rng = 99u32;
            for s in out.iter_mut() {
                let mut y = *s as f64;
                for h in &mut hp {
                    y = h.process(y);
                }
                y = lp.process(y);
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                *s = y as f32 + 0.003 * (rng as f32 / u32::MAX as f32 * 2.0 - 1.0);
            }
        }
        let _ = testsig::Segment::rest(0.0);
        out
    }

    fn detect(sig: &[f32], sr: f32, model: &DrumModel) -> Vec<DrumClass> {
        let mut d = DrumDetector::new(sr);
        sig.iter().filter_map(|&x| d.push(x)).map(|(f, _, _)| model.classify(&f)).collect()
    }

    fn feats(sig: &[f32], sr: f32) -> Vec<Features> {
        let mut d = DrumDetector::new(sr);
        sig.iter().filter_map(|&x| d.push(x)).map(|(f, _, _)| f).collect()
    }

    const SEQ: [DrumClass; 12] = [
        DrumClass::Kick,
        DrumClass::HiHat,
        DrumClass::Snare,
        DrumClass::HiHat,
        DrumClass::Kick,
        DrumClass::Kick,
        DrumClass::Snare,
        DrumClass::HiHat,
        DrumClass::HiHat,
        DrumClass::Kick,
        DrumClass::Snare,
        DrumClass::HiHat,
    ];

    #[test]
    fn rules_classify_clean_beatbox() {
        let sr = 48_000.0;
        let got = detect(&pattern(sr, &SEQ, false), sr, &DrumModel::default());
        assert_eq!(got, SEQ.to_vec());
    }

    #[test]
    fn rules_classify_laptop_beatbox() {
        let sr = 48_000.0;
        let got = detect(&pattern(sr, &SEQ, true), sr, &DrumModel::default());
        assert_eq!(got, SEQ.to_vec());
    }

    /// `cargo test print_features -- --ignored --nocapture` to see the numbers.
    #[test]
    #[ignore]
    fn print_features() {
        let sr = 48_000.0;
        for laptop in [false, true] {
            for (c, f) in SEQ.iter().zip(feats(&pattern(sr, &SEQ, laptop), sr)) {
                let (b, v) = brightness_voicing(&f);
                println!("{laptop:5} {c:?}: body {b:.0} Hz voicing {v:.2} attack {:.0} flat {:.2} low/high {:.1}/{:.1} -> {:?}", 2f32.powf(f[feat::EARLY_CENTROID]), f[feat::LATE_FLATNESS], f[0].max(f[1]), f[4], rule_classify(&f));
            }
        }
    }

    #[test]
    fn training_fixes_laptop_mic() {
        let sr = 48_000.0;
        // Train with 4 examples each (different variations than the test hits).
        let mut model = DrumModel::default();
        for c in DrumClass::ALL {
            let train: Vec<DrumClass> = vec![c; 4];
            for f in feats(&pattern(sr, &train, true), sr) {
                model.add(c, f);
            }
        }
        assert!(model.is_trained());
        let got = detect(&pattern(sr, &SEQ, true), sr, &model);
        assert_eq!(got, SEQ.to_vec());
    }

    #[test]
    fn no_hits_from_sustained_note() {
        let sr = 48_000.0;
        let (sig, _) = testsig::render(sr, &[testsig::Segment::rest(0.2), testsig::Segment::note(60.0, 1.5)], testsig::SynthOpts::default());
        let mut d = DrumDetector::new(sr);
        let at: Vec<f32> = sig.iter().enumerate().filter_map(|(i, &x)| d.push(x).map(|_| i as f32 / sr)).collect();
        let hits = at.len();
        assert!(hits <= 1, "hits at {at:?}");
    }

    #[test]
    fn samples_replace_pads_and_load_from_wav() {
        let sr = 48_000.0;
        let dir = std::env::temp_dir().join(format!("qimist-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("kick.wav");
        let spec = hound::WavSpec { channels: 2, sample_rate: 44_100, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..44_100 / 4 {
            let v = ((i as f32 * 0.05).sin() * 12_000.0 * (1.0 - i as f32 / 11_025.0)) as i16;
            w.write_sample(v).unwrap();
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
        let smp = load_wav(&path, sr).unwrap();
        assert!((smp.len() as f32 - 0.25 * sr).abs() < 200.0, "len {}", smp.len());
        let mut k = DrumKit::new(sr);
        k.set_sample(DrumClass::Kick, Some(Arc::new(smp)));
        k.trigger(DrumClass::Kick, 127);
        let peak = (0..24_000).map(|_| k.process().abs()).fold(0.0, f32::max);
        assert!(peak > 0.5, "sample peak {peak}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn kit_produces_bounded_sound() {
        let mut k = DrumKit::new(48_000.0);
        for kit in [Kit::Eight08, Kit::Tight, Kit::LoFi, Kit::Acoustic, Kit::Trap] {
            k.set(KitSettings { kit, tune: 0.0, decay: 1.0, volume: 1.0, levels: [1.0; 3] });
            for c in DrumClass::ALL {
                k.trigger(c, 127);
                let peak = (0..24_000).map(|_| k.process().abs()).fold(0.0, f32::max);
                assert!(peak > 0.05 && peak < 3.0, "{kit:?} {c:?} peak {peak}");
            }
        }
    }
}

