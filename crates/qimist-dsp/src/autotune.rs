//! Voice auto-tune and harmonizer.
//!
//! The voice is re-pitched with TD-PSOLA: it is cut into two-period grains at
//! pitch marks one period apart (placed by cross-correlation so neighbouring
//! grains line up) and the grains are overlap-added at the new period. Each
//! grain keeps its spectral envelope, so the formants — what makes a voice
//! sound like *that* person — stay where they are instead of going chipmunk.
//! Resampling the grains moves the formants on purpose (the FORMANT control).
//! Unvoiced sounds (breaths, "s", "t") bypass the shifter.
//!
//! Two pitch sources:
//! - **Low latency** (default): a short-window YIN on the live voice gives a
//!   pitch every ~2.7 ms, and the shifter runs with a 10 ms look-ahead. The
//!   main detector only decides the octave. About 10 ms in, 10 ms out.
//! - **Smooth**: the voice is delayed by the main detector's latency so each
//!   HMM-smoothed pitch frame lines up with the audio it describes (the PSOLA
//!   look-ahead is taken out of that delay). Steadier, but ~50 ms late.
//!
//! Retune speed 0 gives the hard "robot" effect; slower speeds keep
//! slides and vibrato natural. Harmony voices are extra synthesis streams tuned
//! to chord tones above the corrected note.

use crate::biquad::{Biquad, Coeffs};
use crate::chords::{self, ChordType};
use crate::pitch::PitchFrame;
use crate::scale::Scale;
use std::f32::consts::PI;

const RING: usize = 1 << 16;
const ACC: usize = 1 << 13;
const MARKS: usize = 32;
/// Synthesis streams: the tuned voice plus two harmonies.
pub const STREAMS: usize = 3;

/// One overlap-add output stream.
struct Stream {
    acc: Vec<f32>,
    wsum: Vec<f32>,
    /// Next synthesis mark (analysis-timeline sample index).
    next: f64,
}

/// Streaming TD-PSOLA pitch shifter with formant control. All the streams
/// share one set of analysis pitch marks.
pub struct Psola {
    sr: f32,
    ring: Vec<f32>,
    /// Absolute index of the next input sample.
    t: i64,
    lookahead: i64,
    /// Longest grain half-width (keeps the look-ahead short for low voices).
    wmax: f32,
    period: f32,
    marks: [(i64, f32); MARKS],
    n_marks: usize,
    streams: [Stream; STREAMS],
}

#[inline]
fn at(ring: &[f32], i: i64) -> f32 {
    if i < 0 {
        0.0
    } else {
        ring[i as usize & (RING - 1)]
    }
}

#[inline]
fn at_frac(ring: &[f32], p: f64) -> f32 {
    let i = p.floor();
    let f = (p - i) as f32;
    let i = i as i64;
    let a = at(ring, i);
    a + (at(ring, i + 1) - a) * f
}

impl Psola {
    pub fn new(sr: f32) -> Self {
        let stream = || Stream { acc: vec![0.0; ACC], wsum: vec![0.0; ACC], next: 0.0 };
        Psola {
            sr,
            ring: vec![0.0; RING],
            t: 0,
            lookahead: (0.022 * sr) as i64,
            wmax: 0.0125 * sr,
            period: sr / 200.0,
            marks: [(0, 0.0); MARKS],
            n_marks: 0,
            streams: [stream(), stream(), stream()],
        }
    }

    /// Samples between input and output.
    pub fn latency(&self) -> usize {
        self.lookahead as usize
    }

    /// Short look-ahead and grains (low latency) or long ones (smoothest).
    pub fn set_low_latency(&mut self, low: bool) {
        let (lookahead, wmax) = if low { (0.010, 0.006) } else { (0.022, 0.0125) };
        let lookahead = (lookahead * self.sr) as i64;
        if lookahead == self.lookahead {
            return;
        }
        self.lookahead = lookahead;
        self.wmax = wmax * self.sr;
        for s in self.streams.iter_mut() {
            s.acc.fill(0.0);
            s.wsum.fill(0.0);
            s.next = 0.0;
        }
    }

    pub fn reset(&mut self) {
        self.ring.fill(0.0);
        for s in self.streams.iter_mut() {
            s.acc.fill(0.0);
            s.wsum.fill(0.0);
            s.next = 0.0;
        }
        self.t = 0;
        self.n_marks = 0;
    }

    /// The sung period in samples (from the pitch detector).
    pub fn set_period(&mut self, samples: f32) {
        self.period = samples.clamp(24.0, self.sr / 45.0);
    }

    fn push_mark(&mut self, pos: i64, p: f32) {
        self.marks[self.n_marks % MARKS] = (pos, p);
        self.n_marks += 1;
    }

    /// Place the next analysis mark one period after the last, nudged to where
    /// the waveform best matches the previous grain.
    fn place_marks(&mut self) {
        let p = self.period;
        let now = self.t - 1;
        let half = (p * 0.5) as i64;
        let r = ((p / 8.0) as i64).max(1);
        if self.n_marks == 0 {
            if now >= (2.0 * p) as i64 {
                self.push_mark(now - half - r, p);
            }
            return;
        }
        let last = self.marks[(self.n_marks - 1) % MARKS].0;
        let cand = last + p.round() as i64;
        if now < cand + r + half {
            return;
        }
        let ring = &self.ring;
        let el: f32 = (-half..=half).map(|j| at(ring, last + j).powi(2)).sum();
        let step = if p > 400.0 { 2 } else { 1 };
        let mut best = (f32::MIN, cand);
        for k in (-r..=r).step_by(step) {
            let m = cand + k;
            let (mut num, mut em) = (0.0f32, 0.0f32);
            for j in -half..=half {
                let a = at(ring, m + j);
                num += a * at(ring, last + j);
                em += a * a;
            }
            let score = num / (em * el).sqrt().max(1e-9);
            if score > best.0 {
                best = (score, m);
            }
        }
        self.push_mark(best.1, p);
    }

    /// The analysis mark nearest `s` whose whole grain has arrived.
    fn nearest(&self, s: f64) -> Option<(i64, f32)> {
        let now = self.t - 1;
        let mut best = None;
        let mut bd = f64::MAX;
        for &(a, p) in self.marks.iter().take(self.n_marks.min(MARKS)) {
            if a + p.min(self.wmax) as i64 + 1 > now {
                continue;
            }
            let d = (a as f64 - s).abs();
            if d < bd {
                bd = d;
                best = Some((a, p));
            }
        }
        best
    }

    /// Feed one sample. `ratios[k]` is stream k's pitch ratio (≤ 0 = silent,
    /// skipped); `formant` scales the formants (1 = natural). Returns each
    /// stream's output and the dry signal, all `latency()` samples late.
    pub fn process(&mut self, x: f32, ratios: [f32; STREAMS], formant: f32) -> ([f32; STREAMS], f32) {
        self.ring[self.t as usize & (RING - 1)] = x;
        self.t += 1;
        self.place_marks();
        let o = self.t - 1 - self.lookahead;
        let mut out = [0.0; STREAMS];
        if o < 0 {
            return (out, 0.0);
        }
        let fs = formant.clamp(0.5, 2.0);
        let w_est = (self.period.min(self.wmax) / fs) as f64;
        let i = o as usize & (ACC - 1);
        for (k, &ratio) in ratios.iter().enumerate() {
            if ratio <= 0.0 {
                let st = &mut self.streams[k];
                st.acc[i] = 0.0;
                st.wsum[i] = 0.0;
                st.next = o as f64;
                continue;
            }
            let ratio = ratio.clamp(0.25, 4.0);
            if self.streams[k].next < o as f64 {
                self.streams[k].next = o as f64;
            }
            while self.streams[k].next - w_est <= o as f64 {
                let s = self.streams[k].next;
                let step = match self.nearest(s) {
                    Some((a, p)) => {
                        let wo = p.min(self.wmax) / fs;
                        let n = wo as i64;
                        let sc = s.round() as i64;
                        let st = &mut self.streams[k];
                        for j in (-n).max(o - sc)..=n {
                            let jf = j as f32;
                            let win = 0.5 + 0.5 * (PI * jf / wo).cos();
                            let idx = (sc + j) as usize & (ACC - 1);
                            st.acc[idx] += win * at_frac(&self.ring, a as f64 + (jf * fs) as f64);
                            st.wsum[idx] += win;
                        }
                        p / ratio
                    }
                    None => self.period / ratio,
                };
                self.streams[k].next += step as f64;
            }
            let st = &mut self.streams[k];
            // Normalise where grains pile up; leave gaps between sparse grains alone.
            out[k] = st.acc[i] / st.wsum[i].max(1.0);
            st.acc[i] = 0.0;
            st.wsum[i] = 0.0;
        }
        (out, at(&self.ring, o))
    }
}

/// Low-latency pitch tracker: YIN over a short window that ends at the newest
/// sample, on a 4×-decimated copy of the voice, every 32 decimated samples.
pub struct FastPitch {
    dec: usize,
    sr_d: f32,
    lp: [Biquad; 2],
    buf: Vec<f32>,
    pos: usize,
    phase: usize,
    since: usize,
    min_lag: usize,
    max_lag: usize,
    last_lag: f32,
    d: Vec<f32>,
}

const FP_BUF: usize = 1024;
const FP_HOP: usize = 32;

/// One fast pitch estimate.
#[derive(Clone, Copy, Debug)]
pub struct FastEstimate {
    pub voiced: bool,
    pub f0: f32,
    pub rms_db: f32,
}

impl FastPitch {
    pub fn new(sr: f32) -> Self {
        let dec = (sr / 12_000.0).round().max(1.0) as usize;
        let sr_d = sr / dec as f32;
        let lp = Coeffs::lowpass(sr as f64, (0.4 * sr_d) as f64, 0.7071);
        let mut fp = FastPitch {
            dec,
            sr_d,
            lp: [Biquad::new(lp), Biquad::new(lp)],
            buf: vec![0.0; FP_BUF],
            pos: 0,
            phase: 0,
            since: 0,
            min_lag: 2,
            max_lag: 2,
            last_lag: 0.0,
            d: vec![0.0; FP_BUF],
        };
        fp.set_range(60.0, 1000.0);
        fp
    }

    pub fn set_range(&mut self, fmin: f32, fmax: f32) {
        self.min_lag = ((self.sr_d / fmax).floor() as usize).max(2);
        self.max_lag = ((self.sr_d / fmin).ceil() as usize).min(FP_BUF / 2 - 2);
    }

    #[inline]
    fn at(&self, back: usize) -> f32 {
        self.buf[(self.pos + FP_BUF - 1 - back) % FP_BUF]
    }

    /// Feed one host-rate sample; returns an estimate every hop.
    #[inline]
    pub fn push(&mut self, x: f32) -> Option<FastEstimate> {
        let y = self.lp[0].process(x as f64);
        let y = self.lp[1].process(y) as f32;
        self.phase += 1;
        if self.phase < self.dec {
            return None;
        }
        self.phase = 0;
        self.buf[self.pos] = y;
        self.pos = (self.pos + 1) % FP_BUF;
        self.since += 1;
        if self.since < FP_HOP {
            return None;
        }
        self.since = 0;
        Some(self.estimate())
    }

    fn estimate(&mut self) -> FastEstimate {
        // Window: about one period of the last pitch (≥ 6 ms), so it stays short.
        let w_min = (0.006 * self.sr_d) as usize;
        let w = if self.last_lag > 0.0 { (self.last_lag * 1.2) as usize } else { self.max_lag };
        let w = w.clamp(w_min, self.max_lag);
        let energy: f32 = (0..w).map(|j| self.at(j).powi(2)).sum();
        let rms_db = 10.0 * (energy / w as f32 + 1e-12).log10();

        // Cumulative-mean-normalised difference (YIN steps 2–3).
        let mut run = 0.0f32;
        self.d[0] = 1.0;
        for tau in 1..=self.max_lag {
            let mut acc = 0.0f32;
            for j in 0..w {
                let e = self.at(j) - self.at(j + tau);
                acc += e * e;
            }
            run += acc;
            self.d[tau] = if run > 0.0 { acc * tau as f32 / run } else { 1.0 };
        }
        // First dip under the threshold, walked down to its minimum; else the global minimum.
        let mut pick = None;
        let mut tau = self.min_lag;
        while tau < self.max_lag {
            if self.d[tau] < 0.15 {
                while tau + 1 < self.max_lag && self.d[tau + 1] < self.d[tau] {
                    tau += 1;
                }
                pick = Some(tau);
                break;
            }
            tau += 1;
        }
        let tau = pick.unwrap_or_else(|| (self.min_lag..self.max_lag).min_by(|&a, &b| self.d[a].total_cmp(&self.d[b])).unwrap_or(self.min_lag));
        let voiced = self.d[tau] < 0.35;
        // Parabolic interpolation.
        let (a, b, c) = (self.d[tau - 1], self.d[tau], self.d[tau + 1]);
        let den = a - 2.0 * b + c;
        let lag = tau as f32 + if den.abs() > 1e-9 { (0.5 * (a - c) / den).clamp(-0.5, 0.5) } else { 0.0 };
        self.last_lag = if voiced { lag } else { 0.0 };
        FastEstimate { voiced, f0: if voiced { self.sr_d / lag } else { 0.0 }, rms_db }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutoTuneSettings {
    pub enabled: bool,
    /// Retune time constant in ms (0 = instant).
    pub speed_ms: f32,
    /// 0..1 correction strength.
    pub amount: f32,
    /// Formant shift in semitones (0 = natural voice; − deeper, + smaller).
    pub formant_st: f32,
    pub harmony: bool,
    pub harmony_level: f32,
    /// Track pitch on the live voice (~10 ms) instead of the smoothed detector (~50 ms).
    pub low_latency: bool,
    /// Below this level the fast tracker treats the voice as silent.
    pub gate_db: f32,
}

impl Default for AutoTuneSettings {
    fn default() -> Self {
        AutoTuneSettings { enabled: false, speed_ms: 20.0, amount: 1.0, formant_st: 0.0, harmony: false, harmony_level: 0.6, low_latency: true, gate_db: -55.0 }
    }
}

pub struct AutoTune {
    sr: f32,
    pub s: AutoTuneSettings,
    align: Vec<f32>,
    align_pos: usize,
    align_len: usize,
    psola: Psola,
    fast: FastPitch,
    det_latency: usize,
    scale: Option<Scale>,
    /// Last voiced f0 from the main detector and how many samples ago (octave guard).
    slow_f0: f32,
    slow_age: usize,
    /// Pitch currently being tuned (float MIDI).
    sung: f32,
    ratio: f32,
    target_ratio: f32,
    harm_ratio: [f32; 2],
    harm_target: [f32; 2],
    harm_gain: f32,
    harm_gain_target: f32,
    voiced: f32,
    voiced_target: f32,
    formant: f32,
    note: Option<i32>,
}

impl AutoTune {
    pub fn new(sr: f32) -> Self {
        AutoTune {
            sr,
            s: AutoTuneSettings::default(),
            align: vec![0.0; (0.25 * sr) as usize],
            align_pos: 0,
            align_len: 0,
            psola: Psola::new(sr),
            fast: FastPitch::new(sr),
            det_latency: 0,
            scale: None,
            slow_f0: 0.0,
            slow_age: usize::MAX,
            sung: 0.0,
            ratio: 1.0,
            target_ratio: 1.0,
            harm_ratio: [1.0; 2],
            harm_target: [1.0; 2],
            harm_gain: 0.0,
            harm_gain_target: 0.0,
            voiced: 0.0,
            voiced_target: 0.0,
            formant: 1.0,
            note: None,
        }
    }

    pub fn is_active(&self) -> bool {
        self.s.enabled || self.s.harmony
    }

    /// The main detector's latency. In smooth mode the voice is delayed to
    /// line up with its frames (the shifter's own look-ahead counts towards it).
    pub fn set_alignment(&mut self, samples: usize) {
        self.det_latency = samples;
        self.update_mode();
    }

    /// Voice range in Hz for the fast tracker.
    pub fn set_range(&mut self, fmin: f32, fmax: f32) {
        self.fast.set_range(fmin, fmax);
    }

    /// Apply new settings (switches the latency mode when it changed).
    pub fn set_settings(&mut self, s: AutoTuneSettings) {
        let mode_changed = s.low_latency != self.s.low_latency;
        self.s = s;
        if mode_changed {
            self.update_mode();
        }
    }

    fn update_mode(&mut self) {
        self.psola.set_low_latency(self.s.low_latency);
        self.align_len = if self.s.low_latency { 0 } else { self.det_latency.saturating_sub(self.psola.latency()).min(self.align.len() - 1) };
    }

    /// Delay from voice in to tuned voice out, in samples.
    pub fn latency_samples(&self) -> usize {
        self.align_len + self.psola.latency()
    }

    pub fn reset(&mut self) {
        self.align.fill(0.0);
        self.psola.reset();
        self.ratio = 1.0;
        self.voiced = 0.0;
        self.note = None;
    }

    /// The note the voice is being tuned to.
    pub fn target_note(&self) -> Option<i32> {
        self.note
    }

    /// The sung pitch (float MIDI) behind `target_note`.
    pub fn sung_midi(&self) -> f32 {
        self.sung
    }

    /// The (corrected) note for `midi`, with hysteresis so a voice hovering
    /// between two notes doesn't flip back and forth.
    pub fn choose_note(&self, midi: f32, scale: Option<Scale>) -> i32 {
        if let Some(n) = self.note {
            let keep = match scale {
                Some(s) => s.snap(midi) as i32 == n || (midi - n as f32).abs() < 0.65,
                None => (midi - n as f32).abs() < 0.65,
            };
            if keep {
                return n;
            }
        }
        scale.map(|s| s.snap(midi) as i32).unwrap_or(midi.round() as i32)
    }

    /// A frame from the main detector. It drives the tuning in smooth mode;
    /// in low-latency mode it only supplies the key and the octave guard.
    pub fn on_frame(&mut self, f: &PitchFrame, scale: Option<Scale>) {
        self.scale = scale;
        if f.voiced && f.f0 > 0.0 {
            self.slow_f0 = f.f0;
            self.slow_age = 0;
        }
        if !self.s.low_latency {
            self.retarget(f.voiced && f.f0 > 0.0, f.f0);
        }
    }

    /// Fix the fast tracker's octave against the (slower, steadier) detector.
    fn guard_octave(&self, f0: f32) -> f32 {
        if self.slow_age > (0.25 * self.sr) as usize || self.slow_f0 <= 0.0 {
            return f0;
        }
        let oct = (self.slow_f0 / f0).log2();
        let k = oct.round();
        if k != 0.0 && (oct - k).abs() < 0.12 {
            f0 * 2f32.powf(k)
        } else {
            f0
        }
    }

    /// New pitch → note choice, shift ratios and harmony targets.
    fn retarget(&mut self, voiced: bool, f0: f32) {
        if !voiced {
            self.target_ratio = 1.0;
            self.harm_gain_target = 0.0;
            self.voiced_target = 0.0;
            self.note = None;
            return;
        }
        let midi = crate::hz_to_midi(f0);
        self.sung = midi;
        self.voiced_target = 1.0;
        let n = self.choose_note(midi, self.scale);
        self.note = Some(n);
        self.psola.set_period(self.sr / f0);
        let amount = if self.s.enabled { self.s.amount } else { 0.0 };
        let corrected = midi + amount * (n as f32 - midi);
        self.target_ratio = 2f32.powf((corrected - midi) / 12.0);
        let c = chords::build(n.clamp(0, 127) as u8, ChordType::Triad, self.scale, 0, false, false);
        for k in 0..2 {
            // Harmonies follow the corrected line (keeping the singer's vibrato).
            self.harm_target[k] = 2f32.powf((c.notes[k + 1] as f32 + (corrected - n as f32) - midi) / 12.0);
        }
        self.harm_gain_target = if self.s.harmony { self.s.harmony_level } else { 0.0 };
    }

    /// Process one voice sample → (tuned voice, harmony voices).
    #[inline]
    pub fn process(&mut self, x: f32) -> (f32, f32) {
        let n = self.align.len();
        self.align[self.align_pos] = x;
        let aligned = self.align[(self.align_pos + n - self.align_len) % n];
        self.align_pos = (self.align_pos + 1) % n;

        self.slow_age = self.slow_age.saturating_add(1);
        if self.s.low_latency {
            if let Some(e) = self.fast.push(x) {
                let voiced = e.voiced && e.rms_db > self.s.gate_db;
                let f0 = if voiced { self.guard_octave(e.f0) } else { 0.0 };
                self.retarget(voiced, f0);
            }
        }

        let c = if self.s.speed_ms <= 0.5 { 1.0 } else { 1.0 - (-1000.0 / (self.s.speed_ms * self.sr)).exp() };
        self.ratio += (self.target_ratio - self.ratio) * c;
        for k in 0..2 {
            self.harm_ratio[k] += (self.harm_target[k] - self.harm_ratio[k]) * c.max(0.002);
        }
        self.harm_gain += (self.harm_gain_target - self.harm_gain) * 0.0015;
        self.voiced += (self.voiced_target - self.voiced) * 0.004;
        let formant_target = if self.s.enabled { 2f32.powf(self.s.formant_st / 12.0) } else { 1.0 };
        self.formant += (formant_target - self.formant) * 0.002;

        let harm_on = self.harm_gain > 1e-4 || self.harm_gain_target > 0.0;
        let ratios = [
            if self.s.enabled { self.ratio } else { 0.0 },
            if harm_on { self.harm_ratio[0] } else { 0.0 },
            if harm_on { self.harm_ratio[1] } else { 0.0 },
        ];
        let (out, dry) = self.psola.process(aligned, ratios, self.formant);
        let main = if self.s.enabled { dry + self.voiced * (out[0] - dry) } else { dry };
        (main, (out[1] + out[2]) * self.harm_gain * self.voiced * 0.7)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    /// Fundamental from the first strong autocorrelation peak.
    fn freq(sig: &[f32], sr: f32) -> f32 {
        let (lo, hi) = ((sr / 1000.0) as usize, (sr / 60.0) as usize);
        let acf: Vec<f32> = (0..=hi)
            .map(|lag| {
                let n = sig.len() - hi;
                let (mut num, mut e0, mut e1) = (0.0, 0.0, 0.0);
                for i in 0..n {
                    num += sig[i] * sig[i + lag];
                    e0 += sig[i] * sig[i];
                    e1 += sig[i + lag] * sig[i + lag];
                }
                num / (e0 * e1 as f32).sqrt().max(1e-9)
            })
            .collect();
        let max = acf[lo..hi].iter().cloned().fold(f32::MIN, f32::max);
        let lag = (lo..hi - 1).find(|&l| acf[l] >= 0.9 * max && acf[l] >= acf[l - 1] && acf[l] >= acf[l + 1]).unwrap();
        // Parabolic interpolation around the peak.
        let (a, b, c) = (acf[lag - 1], acf[lag], acf[lag + 1]);
        let d = 0.5 * (a - c) / (a - 2.0 * b + c);
        sr / (lag as f32 + d)
    }

    /// Spectral centroid below 4 kHz.
    fn centroid(sig: &[f32], sr: f32) -> f32 {
        let (mut num, mut den) = (0.0f64, 0.0f64);
        let n = sig.len();
        for k in 1..160 {
            let f = k as f32 * 25.0;
            let w = std::f32::consts::TAU * f / sr;
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (i, &x) in sig.iter().enumerate() {
                let win = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos();
                re += (x * win * (w * i as f32).cos()) as f64;
                im += (x * win * (w * i as f32).sin()) as f64;
            }
            let m = re * re + im * im;
            num += f as f64 * m;
            den += m;
        }
        (num / den) as f32
    }

    /// A crude vowel: glottal pulse train through two formant resonators.
    fn vowel(f0: f32, sr: f32, n: usize) -> Vec<f32> {
        let mut out = vec![0.0; n];
        let mut phase = 0.0;
        let res = |fc: f32, bw: f32| {
            let r = (-PI * bw / sr).exp();
            (2.0 * r * (std::f32::consts::TAU * fc / sr).cos(), -r * r)
        };
        let (f1, f2) = (res(700.0, 90.0), res(1200.0, 110.0));
        let (mut y1, mut y2, mut z1, mut z2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for o in out.iter_mut() {
            phase += f0 / sr;
            let x = if phase >= 1.0 {
                phase -= 1.0;
                1.0
            } else {
                0.0
            };
            let a = x + f1.0 * y1 + f1.1 * y2;
            y2 = y1;
            y1 = a;
            let b = a + f2.0 * z1 + f2.1 * z2;
            z2 = z1;
            z1 = b;
            *o = b * 0.01;
        }
        out
    }

    fn run(input: &[f32], sr: f32, period: f32, ratio: f32, formant: f32) -> Vec<f32> {
        let mut ps = Psola::new(sr);
        ps.set_period(period);
        input.iter().map(|&x| ps.process(x, [ratio, 0.0, 0.0], formant).0[0]).collect()
    }

    #[test]
    fn shifter_shifts_pitch() {
        let sr = 48_000.0;
        let sine: Vec<f32> = (0..48_000).map(|i| (std::f32::consts::TAU * 220.0 * i as f32 / sr).sin()).collect();
        for ratio in [0.8f32, 1.25, 1.5] {
            let out = run(&sine, sr, sr / 220.0, ratio, 1.0);
            let f = freq(&out[9600..], sr);
            assert!((f / (220.0 * ratio) - 1.0).abs() < 0.02, "ratio {ratio}: {f} Hz");
            let rms = |s: &[f32]| (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt();
            let g = rms(&out[9600..]) / rms(&sine);
            assert!((0.5..1.6).contains(&g), "ratio {ratio}: level {g}");
        }
    }

    #[test]
    fn formants_stay_put() {
        let sr = 48_000.0;
        let v = vowel(150.0, sr, 48_000);
        let c_in = centroid(&v[24_000..32_192], sr);
        for ratio in [0.75f32, 1.4] {
            let out = run(&v, sr, sr / 150.0, ratio, 1.0);
            let f = freq(&out[12_000..], sr);
            assert!((f / (150.0 * ratio) - 1.0).abs() < 0.03, "ratio {ratio}: {f} Hz");
            let c = centroid(&out[24_000..32_192], sr);
            // A plain resampling shifter would move the centroid by the full ratio.
            assert!((c / c_in - 1.0).abs() < 0.12, "ratio {ratio}: centroid {c_in:.0} → {c:.0}");
        }
        // FORMANT moves them on purpose without changing the pitch.
        let up = run(&v, sr, sr / 150.0, 1.0, 1.3);
        let c = centroid(&up[24_000..32_192], sr);
        assert!(c / c_in > 1.18, "formant +: centroid {c_in:.0} → {c:.0}");
        let f = freq(&up[12_000..], sr);
        assert!((f / 150.0 - 1.0).abs() < 0.03, "formant +: {f} Hz");
    }

    #[test]
    fn autotune_corrects_flat_note() {
        let sr = 48_000.0;
        let mut at = AutoTune::new(sr);
        at.set_settings(AutoTuneSettings { enabled: true, speed_ms: 0.0, amount: 1.0, low_latency: false, ..Default::default() });
        // Sung 40 cents flat of A3 (220 Hz).
        let f0 = 220.0 * 2f32.powf(-0.4 / 12.0);
        let frame = PitchFrame { voiced: true, f0, midi: crate::hz_to_midi(f0), confidence: 1.0, rms_db: -20.0, noise_db: -80.0 };
        at.on_frame(&frame, None);
        assert_eq!(at.target_note(), Some(57));
        let out: Vec<f32> = (0..48_000).map(|i| at.process((std::f32::consts::TAU * f0 * i as f32 / sr).sin()).0).collect();
        let f = freq(&out[9600..], sr);
        assert!((f - 220.0).abs() < 220.0 * 0.01, "{f} Hz");
    }

    #[test]
    fn survives_glides_silence_and_harmony() {
        for low in [false, true] {
        let sr = 48_000.0;
        let mut at = AutoTune::new(sr);
        at.set_settings(AutoTuneSettings { enabled: true, speed_ms: 10.0, amount: 1.0, formant_st: -3.0, harmony: true, harmony_level: 0.8, low_latency: low, gate_db: -60.0 });
        at.set_alignment(1500);
        let (mut phase, mut peak_in, mut peak_out) = (0.0f32, 0.0f32, 0.0f32);
        for i in 0..96_000 {
            let t = i as f32 / sr;
            // 0.5 s silence, then a slide 110 → 440 Hz with vibrato, then silence.
            let voiced = (0.5..1.6).contains(&t);
            let f0 = 110.0 * 4f32.powf(((t - 0.5) / 1.1).clamp(0.0, 1.0)) * (1.0 + 0.01 * (TAU * 5.5 * t).sin());
            phase = (phase + f0 / sr) % 1.0;
            let x = if voiced { 0.5 * (TAU * phase).sin() + 0.2 * (2.0 * TAU * phase).sin() } else { 0.0 };
            if i % 192 == 0 {
                let frame = PitchFrame { voiced, f0: if voiced { f0 } else { 0.0 }, midi: if voiced { crate::hz_to_midi(f0) } else { 0.0 }, confidence: 1.0, rms_db: -12.0, noise_db: -80.0 };
                at.on_frame(&frame, None);
            }
            let (a, b) = at.process(x);
            assert!(a.is_finite() && b.is_finite());
            peak_in = peak_in.max(x.abs());
            peak_out = peak_out.max((a + b).abs());
        }
        assert!(peak_out < 2.5 * peak_in, "low {low}: peak {peak_out} vs {peak_in}");
        }
    }

    #[test]
    fn fast_tracker_follows_voice() {
        let sr = 48_000.0;
        for f0 in [90.0f32, 140.0, 220.0, 330.0, 520.0] {
            let v = vowel(f0, sr, 24_000);
            let mut fp = FastPitch::new(sr);
            fp.set_range(65.0, 1000.0);
            let est: Vec<FastEstimate> = v.iter().filter_map(|&x| fp.push(x)).collect();
            let tail = &est[est.len() / 2..];
            let good = tail.iter().filter(|e| e.voiced && (crate::hz_to_midi(e.f0) - crate::hz_to_midi(f0)).abs() < 0.3).count();
            assert!(good as f32 / tail.len() as f32 > 0.95, "{f0} Hz: {good}/{} good, e.g. {:?}", tail.len(), tail[0]);
        }
        // Same vowel with fan-like noise 20 dB down.
        let mut rng = 1u32;
        let noisy: Vec<f32> = vowel(180.0, sr, 24_000)
            .iter()
            .map(|&x| {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                x + 0.004 * (rng as f32 / u32::MAX as f32 * 2.0 - 1.0)
            })
            .collect();
        let rms = (noisy.iter().map(|x| x * x).sum::<f32>() / noisy.len() as f32).sqrt();
        let mut fp = FastPitch::new(sr);
        fp.set_range(65.0, 1000.0);
        let est: Vec<FastEstimate> = noisy.iter().filter_map(|&x| fp.push(x)).collect();
        let tail = &est[est.len() / 2..];
        let good = tail.iter().filter(|e| e.voiced && (crate::hz_to_midi(e.f0) - crate::hz_to_midi(180.0)).abs() < 0.3).count();
        assert!(good as f32 / tail.len() as f32 > 0.9, "noisy (rms {rms}): {good}/{}", tail.len());
        // Silence is unvoiced or under the gate.
        let mut fp = FastPitch::new(sr);
        let est: Vec<FastEstimate> = (0..4800).filter_map(|_| fp.push(0.0)).collect();
        assert!(est.iter().all(|e| !e.voiced || e.rms_db < -100.0));
    }

    #[test]
    fn low_latency_mode_tunes_without_detector_frames() {
        let sr = 48_000.0;
        let mut at = AutoTune::new(sr);
        at.set_alignment(2500);
        at.set_settings(AutoTuneSettings { enabled: true, speed_ms: 0.0, amount: 1.0, low_latency: true, gate_db: -60.0, ..Default::default() });
        assert!(at.latency_samples() <= (0.011 * sr) as usize, "latency {}", at.latency_samples());
        // A vowel 40 cents flat of A3; no frames from the main detector at all.
        let f0 = 220.0 * 2f32.powf(-0.4 / 12.0);
        let v = vowel(f0, sr, 48_000);
        let out: Vec<f32> = v.iter().map(|&x| at.process(x).0).collect();
        assert_eq!(at.target_note(), Some(57));
        let f = freq(&out[12_000..], sr);
        assert!((f - 220.0).abs() < 220.0 * 0.01, "{f} Hz");
        // Tuning reaches the output within ~25 ms of the voice starting.
        let early = freq(&out[(0.025 * sr) as usize..(0.06 * sr) as usize], sr);
        assert!((early - 220.0).abs() < 220.0 * 0.012, "early {early} Hz");
    }

    /// Cost of the tuned voice plus both harmonies (run with --ignored).
    #[test]
    #[ignore]
    fn psola_cpu() {
        let sr = 48_000.0;
        let v = vowel(110.0, sr, 480_000);
        let mut ps = Psola::new(sr);
        ps.set_period(sr / 110.0);
        let t = std::time::Instant::now();
        let mut acc = 0.0;
        for &x in &v {
            acc += ps.process(x, [1.06, 1.26, 1.5], 0.8).0[2];
        }
        let el = t.elapsed().as_secs_f32();
        println!("psola: {:.1}% of one core ({acc})", el / 10.0 * 100.0);
    }
}
