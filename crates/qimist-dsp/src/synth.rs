//! Polyphonic synth (8 voices) played by the detected notes, chords and arps.
//!
//! Per voice: three oscillators (sine / triangle / PolyBLEP saw & square /
//! noise), each with up to 7 detuned unison copies spread across the stereo
//! field → drive → TPT state-variable filter (LP12 / LP24 / BP / HP) → ADSR amp
//! envelope. The cutoff follows the envelope (`env_amount`); one global LFO
//! can add vibrato, filter sweeps or tremolo. In legato mode a note that starts
//! within a few ms of another's release takes over that voice: the pitch glides
//! and the envelope continues, which suits a sung melody.

use std::f32::consts::{PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    Sine,
    Triangle,
    Saw,
    Square,
    Noise,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterMode {
    Lp12,
    Lp24,
    Bp,
    Hp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LfoShape {
    Sine,
    Triangle,
    Square,
    SampleHold,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LfoTarget {
    /// Vibrato, up to ±1 semitone.
    Pitch,
    /// Up to ±3 octaves of cutoff.
    Filter,
    /// Tremolo.
    Amp,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LfoSettings {
    pub shape: LfoShape,
    pub rate_hz: f32,
    /// 0..1
    pub depth: f32,
    pub target: LfoTarget,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OscSettings {
    pub wave: Wave,
    pub level: f32,
    pub octave: i32,
    pub detune_cents: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SynthSettings {
    pub oscs: [OscSettings; 3],
    pub cutoff_hz: f32,
    /// 0..1
    pub resonance: f32,
    /// 0..1, envelope opens the filter by up to 5 octaves.
    pub env_amount: f32,
    pub filter_mode: FilterMode,
    /// 0..1 saturation before the filter.
    pub drive: f32,
    pub attack_s: f32,
    pub decay_s: f32,
    pub sustain: f32,
    pub release_s: f32,
    pub glide_s: f32,
    pub volume: f32,
    /// 1..=7 copies per oscillator.
    pub unison: u8,
    pub unison_detune_cents: f32,
    /// 0..1 stereo spread of the unison copies.
    pub unison_width: f32,
    pub lfo: LfoSettings,
    /// 0..1: how much the singer's loudness (`set_expression`) opens the
    /// filter and swells the level.
    pub dynamics: f32,
}

impl Default for SynthSettings {
    fn default() -> Self {
        SynthSettings {
            oscs: [
                OscSettings { wave: Wave::Saw, level: 0.85, octave: 0, detune_cents: 0.0 },
                OscSettings { wave: Wave::Square, level: 0.6, octave: -1, detune_cents: 7.0 },
                OscSettings { wave: Wave::Sine, level: 0.3, octave: 1, detune_cents: -5.0 },
            ],
            cutoff_hz: 2400.0,
            resonance: 0.35,
            env_amount: 0.5,
            filter_mode: FilterMode::Lp12,
            drive: 0.0,
            attack_s: 0.008,
            decay_s: 0.32,
            sustain: 0.6,
            release_s: 0.45,
            glide_s: 0.03,
            volume: 0.8,
            unison: 1,
            unison_detune_cents: 12.0,
            unison_width: 0.7,
            lfo: LfoSettings { shape: LfoShape::Sine, rate_hz: 5.0, depth: 0.0, target: LfoTarget::Pitch },
            dynamics: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// ADSR: linear attack, exponential decay and release.
#[derive(Clone, Copy, Debug)]
pub struct Adsr {
    stage: Stage,
    level: f32,
}

impl Default for Adsr {
    fn default() -> Self {
        Adsr { stage: Stage::Idle, level: 0.0 }
    }
}

impl Adsr {
    pub fn gate_on(&mut self) {
        self.stage = Stage::Attack;
    }
    pub fn gate_off(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }
    pub fn is_active(&self) -> bool {
        self.stage != Stage::Idle
    }
    pub fn level(&self) -> f32 {
        self.level
    }

    #[inline]
    pub fn next(&mut self, sr: f32, s: &SynthSettings) -> f32 {
        let coef = |t: f32| 1.0 - (-1.0 / (t.max(0.001) * sr / 4.6)).exp(); // reach ~1% in t
        match self.stage {
            Stage::Idle => self.level = 0.0,
            Stage::Attack => {
                self.level += 1.0 / (s.attack_s.max(0.0005) * sr);
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level += (s.sustain - self.level) * coef(s.decay_s);
                if (self.level - s.sustain).abs() < 1e-4 {
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => self.level = s.sustain,
            Stage::Release => {
                self.level -= self.level * coef(s.release_s);
                if self.level < 1e-5 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }
}

/// Topology-preserving-transform state-variable filter (Simper).
#[derive(Clone, Copy, Debug, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

impl Svf {
    pub fn set(&mut self, sr: f32, cutoff: f32, resonance: f32) {
        let g = (PI * cutoff.clamp(20.0, sr * 0.45) / sr).tan();
        let k = 2.0 - 1.96 * resonance.clamp(0.0, 1.0);
        self.k = k;
        self.a1 = 1.0 / (1.0 + g * (g + k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    /// Copy coefficients (not state) from another filter.
    pub fn copy_coeffs(&mut self, o: &Svf) {
        (self.a1, self.a2, self.a3, self.k) = (o.a1, o.a2, o.a3, o.k);
    }

    pub fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }

    /// Returns (low, band, high).
    #[inline]
    pub fn tick(&mut self, x: f32) -> (f32, f32, f32) {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, v1, x - self.k * v1 - v2)
    }

    #[inline]
    pub fn lowpass(&mut self, x: f32) -> f32 {
        self.tick(x).0
    }
}

/// Two SVF stages for one channel; 12 dB modes use only the first.
#[derive(Clone, Copy, Debug, Default)]
struct FilterPair {
    a: Svf,
    b: Svf,
}

impl FilterPair {
    #[inline]
    fn process(&mut self, x: f32, mode: FilterMode) -> f32 {
        match mode {
            FilterMode::Lp12 => self.a.tick(x).0,
            FilterMode::Lp24 => {
                let y = self.a.tick(x).0;
                self.b.tick(y).0
            }
            FilterMode::Bp => self.a.tick(x).1,
            FilterMode::Hp => self.a.tick(x).2,
        }
    }
}

#[inline]
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

/// Soft saturation (rational tanh approximation).
#[inline]
fn soft_clip(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    x * (27.0 + x * x) / (27.0 + 9.0 * x * x)
}

const VOICES: usize = 8;
const MAX_UNISON: usize = 7;
/// Pitch-dependent values are refreshed every this many samples.
const CONTROL_RATE: u32 = 16;

#[derive(Clone, Copy)]
struct Voice {
    phase: [[f32; MAX_UNISON]; 3],
    /// Phase increment per oscillator / unison copy (control rate).
    dt: [[f32; MAX_UNISON]; 3],
    pitch: f32,
    target: f32,
    note: Option<u8>,
    velocity: f32,
    env: Adsr,
    filt: [FilterPair; 2],
    since_off: u32,
    level_smooth: f32,
    /// Note-on counter value when this voice started (for stealing the oldest).
    started: u64,
}

impl Default for Voice {
    fn default() -> Self {
        Voice {
            phase: [[0.0; MAX_UNISON]; 3],
            dt: [[0.0; MAX_UNISON]; 3],
            pitch: 60.0,
            target: 60.0,
            note: None,
            velocity: 0.0,
            env: Adsr::default(),
            filt: [FilterPair::default(); 2],
            since_off: u32::MAX,
            level_smooth: 0.0,
            started: 0,
        }
    }
}

pub struct Synth {
    sr: f32,
    settings: SynthSettings,
    voices: [Voice; VOICES],
    rng: u32,
    bend: f32,
    expression: f32,
    expr_smooth: f32,
    counter: u32,
    note_count: u64,
    legato: bool,
    lfo_phase: f32,
    lfo_hold: f32,
    lfo_value: f32,
}

/// Note-on within this time after a note-off plays legato.
const LEGATO_MS: f32 = 30.0;

impl Synth {
    pub fn new(sr: f32) -> Self {
        Synth {
            sr,
            settings: SynthSettings::default(),
            voices: [Voice::default(); VOICES],
            rng: 0x1234_5678,
            bend: 0.0,
            expression: 1.0,
            expr_smooth: 1.0,
            counter: 0,
            note_count: 0,
            legato: true,
            lfo_phase: 0.0,
            lfo_hold: 0.0,
            lfo_value: 0.0,
        }
    }

    pub fn set_settings(&mut self, s: SynthSettings) {
        self.settings = s;
    }

    /// Legato on: quick note changes glide without retriggering (sung lines).
    /// Off: every note retriggers its envelope (arpeggios).
    pub fn set_legato(&mut self, on: bool) {
        self.legato = on;
    }

    pub fn reset(&mut self) {
        self.voices = [Voice::default(); VOICES];
    }

    pub fn note_on(&mut self, note: u8, velocity: u8) {
        let legato_window = (LEGATO_MS * 0.001 * self.sr) as u32;
        let target = note as f32;
        // 1. Same note already sounding → retrigger it.
        // 2. Legato: a voice released moments ago → take it over (nearest pitch).
        // 3. An idle voice. 4. The longest-released voice. 5. Steal the oldest.
        let v = &self.voices;
        let idx = v
            .iter()
            .position(|x| x.note == Some(note))
            .or_else(|| {
                self.legato
                    .then(|| {
                        (0..VOICES)
                            .filter(|&i| v[i].note.is_none() && v[i].env.is_active() && v[i].since_off < legato_window)
                            .min_by(|&a, &b| (v[a].target - target).abs().total_cmp(&(v[b].target - target).abs()))
                    })
                    .flatten()
            })
            .or_else(|| v.iter().position(|x| !x.env.is_active()))
            .or_else(|| (0..VOICES).filter(|&i| v[i].note.is_none()).max_by_key(|&i| v[i].since_off))
            .unwrap_or_else(|| (0..VOICES).min_by_key(|&i| v[i].started).unwrap_or(0));

        let glide = self.settings.glide_s > 0.0;
        let legato = self.legato;
        self.note_count += 1;
        let vc = &mut self.voices[idx];
        let gliding = legato && vc.env.is_active() && vc.note.is_none() && vc.since_off < legato_window;
        vc.target = target;
        if !gliding || !glide {
            vc.pitch = target;
        }
        if !vc.env.is_active() {
            vc.filt = [FilterPair::default(); 2];
            vc.level_smooth = 0.0;
            // Spread unison phases so copies don't start in phase (avoids a flam).
            for o in 0..3 {
                for u in 0..MAX_UNISON {
                    vc.phase[o][u] = (u as f32 * 0.618_034 + o as f32 * 0.31) % 1.0;
                }
            }
        }
        vc.env.gate_on();
        vc.velocity = 0.25 + 0.75 * (velocity as f32 / 127.0);
        vc.note = Some(note);
        vc.started = self.note_count;
        // Make sure the new pitch is used from the first sample.
        self.counter = 0;
    }

    pub fn note_off(&mut self, note: u8) {
        for v in self.voices.iter_mut().filter(|v| v.note == Some(note)) {
            v.note = None;
            v.env.gate_off();
            v.since_off = 0;
        }
    }

    pub fn all_off(&mut self) {
        for v in self.voices.iter_mut().filter(|v| v.note.is_some()) {
            v.note = None;
            v.env.gate_off();
            v.since_off = 0;
        }
    }

    pub fn set_bend(&mut self, semitones: f32) {
        self.bend = semitones;
    }

    /// Voice loudness, 0 (quiet) ..1 (loud), used by `dynamics`.
    pub fn set_expression(&mut self, e: f32) {
        self.expression = e.clamp(0.0, 1.0);
    }

    pub fn is_active(&self) -> bool {
        self.voices.iter().any(|v| v.env.is_active())
    }

    #[inline]
    fn rand(rng: &mut u32) -> f32 {
        *rng ^= *rng << 13;
        *rng ^= *rng >> 17;
        *rng ^= *rng << 5;
        *rng as f32 / u32::MAX as f32 * 2.0 - 1.0
    }

    #[inline]
    fn lfo_tick(&mut self) -> f32 {
        let l = self.settings.lfo;
        let prev = self.lfo_phase;
        self.lfo_phase = (self.lfo_phase + l.rate_hz / self.sr) % 1.0;
        let t = self.lfo_phase;
        match l.shape {
            LfoShape::Sine => (TAU * t).sin(),
            LfoShape::Triangle => 1.0 - 4.0 * (t - 0.5).abs(),
            LfoShape::Square => {
                if t < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            LfoShape::SampleHold => {
                if self.lfo_phase < prev {
                    self.lfo_hold = Self::rand(&mut self.rng);
                }
                self.lfo_hold
            }
        }
    }

    /// One stereo sample.
    #[inline]
    pub fn process(&mut self) -> (f32, f32) {
        let s = self.settings;
        let sr = self.sr;
        let control = self.counter % CONTROL_RATE == 0;
        self.counter = self.counter.wrapping_add(1);
        let lfo_raw = self.lfo_tick();
        // Smooth stepped shapes a little so S&H / square don't click.
        self.lfo_value += (lfo_raw - self.lfo_value) * 0.02;
        let lfo = self.lfo_value * s.lfo.depth;
        let (lfo_pitch, lfo_filter, lfo_amp) = match s.lfo.target {
            LfoTarget::Pitch => (lfo, 0.0, 1.0),
            LfoTarget::Filter => (0.0, lfo * 3.0, 1.0),
            LfoTarget::Amp => (0.0, 0.0, 1.0 - s.lfo.depth * 0.5 * (1.0 - self.lfo_value)),
        };

        let glide_c = if s.glide_s > 0.0 { 1.0 - (-1.0 / (s.glide_s * sr / 3.0)).exp() } else { 1.0 };
        let n_uni = (s.unison as usize).clamp(1, MAX_UNISON);
        let uni_gain = 1.0 / (n_uni as f32).sqrt();
        // Unison copy k: detune offset and pan (-1..1), symmetric around the centre.
        let spread = |k: usize| if n_uni == 1 { 0.0 } else { k as f32 / (n_uni - 1) as f32 * 2.0 - 1.0 };
        // Equal-power pan gains per unison copy (fixed for the whole sample).
        let mut pan = [(1.0f32, 1.0f32); MAX_UNISON];
        for (k, g) in pan.iter_mut().enumerate().take(n_uni) {
            let a = (spread(k) * s.unison_width + 1.0) * PI / 4.0;
            *g = (a.cos() * uni_gain, a.sin() * uni_gain);
        }
        let drive_gain = 1.0 + s.drive * 8.0;
        let drive_comp = 1.0 / (1.0 + s.drive * 2.0);
        self.expr_smooth += (self.expression - self.expr_smooth) * 0.0015;
        // Loud = up to +1.5 octaves of cutoff and full level; quiet = darker and softer.
        let dyn_cutoff = s.dynamics * 3.0 * (self.expr_smooth - 0.5);
        let dyn_amp = 1.0 - s.dynamics * 0.6 * (1.0 - self.expr_smooth);

        let (mut out_l, mut out_r) = (0.0, 0.0);
        for v in self.voices.iter_mut() {
            v.since_off = v.since_off.saturating_add(1);
            if !v.env.is_active() {
                continue;
            }
            v.pitch += (v.target - v.pitch) * glide_c;
            let env = v.env.next(sr, &s);
            if control {
                let cutoff = s.cutoff_hz * 2f32.powf(s.env_amount * 5.0 * env + lfo_filter + dyn_cutoff);
                let mut f = Svf::default();
                f.set(sr, cutoff, s.resonance);
                for ch in v.filt.iter_mut() {
                    ch.a.copy_coeffs(&f);
                    ch.b.copy_coeffs(&f);
                }
                let base = v.pitch + self.bend + lfo_pitch;
                for (o, osc) in s.oscs.iter().enumerate() {
                    for k in 0..n_uni {
                        let midi = base + 12.0 * osc.octave as f32 + (osc.detune_cents + spread(k) * s.unison_detune_cents) / 100.0;
                        v.dt[o][k] = (440.0 * 2f32.powf((midi - 69.0) / 12.0) / sr).min(0.49);
                    }
                }
            }

            let (mut l, mut r) = (0.0, 0.0);
            for o in 0..3 {
                let osc = s.oscs[o];
                if osc.level <= 0.0001 {
                    continue;
                }
                for k in 0..n_uni {
                    let dt = v.dt[o][k];
                    let t = v.phase[o][k];
                    let w = match osc.wave {
                        Wave::Sine => (TAU * t).sin(),
                        Wave::Triangle => 1.0 - 4.0 * (t - 0.5).abs(),
                        Wave::Saw => 2.0 * t - 1.0 - poly_blep(t, dt),
                        Wave::Square => {
                            let sq = if t < 0.5 { 1.0 } else { -1.0 };
                            sq + poly_blep(t, dt) - poly_blep((t + 0.5) % 1.0, dt)
                        }
                        Wave::Noise => Self::rand(&mut self.rng),
                    };
                    let mut nt = t + dt;
                    if nt >= 1.0 {
                        nt -= 1.0;
                    }
                    v.phase[o][k] = nt;
                    let x = w * osc.level;
                    l += x * pan[k].0;
                    r += x * pan[k].1;
                }
            }
            let norm = std::f32::consts::SQRT_2 * 0.35;
            let (mut l, mut r) = (l * norm, r * norm);
            if s.drive > 0.001 {
                l = soft_clip(l * drive_gain) * drive_comp;
                r = soft_clip(r * drive_gain) * drive_comp;
            }
            // Smooth the velocity/volume to avoid zipper noise on legato level changes.
            v.level_smooth += (v.velocity * s.volume - v.level_smooth) * 0.002;
            let g = env * v.level_smooth * lfo_amp * dyn_amp;
            out_l += v.filt[0].process(l, s.filter_mode) * g;
            out_r += v.filt[1].process(r, s.filter_mode) * g;
        }
        (out_l, out_r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plays_and_releases() {
        let mut s = Synth::new(48_000.0);
        s.note_on(69, 100);
        let peak = (0..4800).map(|_| s.process().0.abs()).fold(0.0, f32::max);
        assert!(peak > 0.05, "peak {peak}");
        s.note_off(69);
        for _ in 0..48_000 * 3 {
            s.process();
        }
        assert!(!s.is_active());
    }

    #[test]
    fn plays_chords_and_legato() {
        let mut s = Synth::new(48_000.0);
        for n in [60, 64, 67] {
            s.note_on(n, 100);
        }
        assert_eq!(s.voices.iter().filter(|v| v.note.is_some()).count(), 3);
        for _ in 0..4800 {
            s.process();
        }
        // Legato: off + on reuses the released voice and doesn't restart its envelope.
        s.all_off();
        s.process();
        let before = s.voices.iter().map(|v| v.env.level()).fold(0.0, f32::max);
        s.note_on(62, 100);
        let v = s.voices.iter().find(|v| v.note == Some(62)).unwrap();
        assert!(v.env.level() >= before * 0.9);
        assert!((v.pitch - 62.0).abs() > 0.5, "should glide from the old pitch");
    }

    #[test]
    fn unison_filters_lfo_are_bounded_and_stereo() {
        let mut s = Synth::new(48_000.0);
        for mode in [FilterMode::Lp12, FilterMode::Lp24, FilterMode::Bp, FilterMode::Hp] {
            s.set_settings(SynthSettings {
                unison: 7,
                unison_width: 1.0,
                filter_mode: mode,
                drive: 1.0,
                resonance: 0.9,
                lfo: LfoSettings { shape: LfoShape::SampleHold, rate_hz: 8.0, depth: 1.0, target: LfoTarget::Filter },
                ..Default::default()
            });
            for n in [48, 60, 64, 67] {
                s.note_on(n, 127);
            }
            let (mut peak, mut diff) = (0.0f32, 0.0f32);
            for _ in 0..24_000 {
                let (l, r) = s.process();
                peak = peak.max(l.abs()).max(r.abs());
                diff = diff.max((l - r).abs());
            }
            assert!(peak.is_finite() && peak < 4.0 && peak > 0.01, "{mode:?} peak {peak}");
            assert!(diff > 0.001, "{mode:?} unison should be stereo");
            s.all_off();
        }
    }

    #[test]
    fn output_is_bounded() {
        let mut s = Synth::new(48_000.0);
        s.set_settings(SynthSettings { resonance: 1.0, env_amount: 1.0, ..Default::default() });
        s.note_on(40, 127);
        let peak = (0..48_000).map(|_| s.process().0.abs()).fold(0.0, f32::max);
        assert!(peak.is_finite() && peak < 4.0, "peak {peak}");
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    /// `cargo test --release synth_cpu -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn synth_cpu() {
        let sr = 48_000.0;
        let mut s = Synth::new(sr);
        s.set_settings(SynthSettings { unison: 7, filter_mode: FilterMode::Lp24, drive: 0.5, ..Default::default() });
        s.set_legato(false);
        for n in [48, 52, 55, 60, 64, 67, 71, 72] {
            s.note_on(n, 100);
        }
        let t = std::time::Instant::now();
        let mut acc = 0.0;
        for _ in 0..48_000 {
            acc += s.process().0;
        }
        let el = t.elapsed().as_secs_f64();
        println!("8 voices × 7 unison × 3 osc: {:.1}% of one core ({acc:.1})", el * 100.0);
    }
}
