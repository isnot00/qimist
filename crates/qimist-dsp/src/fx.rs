//! Stereo effects: chorus → 5-band EQ → reverb.
//! All buffers are allocated in `new()`; processing never allocates.

use crate::biquad::{Biquad, Coeffs};
use std::f32::consts::TAU;

// ───────────────────────────── chorus ─────────────────────────────

const CHORUS_MAX_S: f32 = 0.05;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChorusSettings {
    pub rate_hz: f32,
    /// 0..1 → up to 8 ms of delay modulation.
    pub depth: f32,
    pub mix: f32,
}

pub struct Chorus {
    sr: f32,
    buf: [Vec<f32>; 2],
    pos: usize,
    phase: f32,
    s: ChorusSettings,
}

impl Chorus {
    pub fn new(sr: f32) -> Self {
        let n = (CHORUS_MAX_S * sr) as usize + 4;
        Chorus { sr, buf: [vec![0.0; n], vec![0.0; n]], pos: 0, phase: 0.0, s: ChorusSettings { rate_hz: 0.8, depth: 0.55, mix: 0.4 } }
    }

    pub fn set(&mut self, s: ChorusSettings) {
        self.s = s;
    }

    pub fn reset(&mut self) {
        self.buf.iter_mut().for_each(|b| b.fill(0.0));
    }

    #[inline]
    fn read(buf: &[f32], pos: usize, delay: f32) -> f32 {
        let n = buf.len();
        let d = delay.clamp(1.0, (n - 3) as f32);
        let i = d.floor() as usize;
        let frac = d - i as f32;
        let a = buf[(pos + n - i) % n];
        let b = buf[(pos + n - i - 1) % n];
        a + (b - a) * frac
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.s.mix <= 0.0001 {
            return (l, r);
        }
        let n = self.buf[0].len();
        self.buf[0][self.pos] = l;
        self.buf[1][self.pos] = r;
        self.phase = (self.phase + self.s.rate_hz / self.sr) % 1.0;
        let base = 0.012 * self.sr;
        let depth = 0.008 * self.sr * self.s.depth;
        let ml = (TAU * self.phase).sin();
        let mr = (TAU * (self.phase + 0.25)).sin();
        let wl = Self::read(&self.buf[0], self.pos, base + depth * 0.5 * (1.0 + ml));
        let wr = Self::read(&self.buf[1], self.pos, base + depth * 0.5 * (1.0 + mr));
        self.pos = (self.pos + 1) % n;
        let m = self.s.mix;
        (l * (1.0 - 0.5 * m) + wl * m, r * (1.0 - 0.5 * m) + wr * m)
    }
}

// ───────────────────────────── EQ ─────────────────────────────

pub const EQ_FREQS: [f32; 5] = [80.0, 250.0, 1000.0, 4000.0, 10_000.0];

pub fn eq_coeffs(sr: f64, band: usize, gain_db: f32) -> Coeffs {
    let f = EQ_FREQS[band] as f64;
    match band {
        0 => Coeffs::low_shelf(sr, f, 0.707, gain_db as f64),
        4 => Coeffs::high_shelf(sr, f, 0.707, gain_db as f64),
        _ => Coeffs::peaking(sr, f, 1.0, gain_db as f64),
    }
}

pub struct Eq5 {
    sr: f64,
    gains: [f32; 5],
    bands: [[Biquad; 5]; 2],
}

impl Eq5 {
    pub fn new(sr: f32) -> Self {
        let mut eq = Eq5 { sr: sr as f64, gains: [f32::NAN; 5], bands: [[Biquad::default(); 5]; 2] };
        eq.set([0.0; 5]);
        eq
    }

    pub fn set(&mut self, gains: [f32; 5]) {
        for b in 0..5 {
            if gains[b] != self.gains[b] {
                let c = eq_coeffs(self.sr, b, gains[b]);
                self.bands[0][b].set(c);
                self.bands[1][b].set(c);
            }
        }
        self.gains = gains;
    }

    pub fn reset(&mut self) {
        self.bands.iter_mut().flatten().for_each(Biquad::reset);
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let mut out = [l as f64, r as f64];
        for (ch, x) in out.iter_mut().enumerate() {
            for b in 0..5 {
                if self.gains[b] != 0.0 {
                    *x = self.bands[ch][b].process(*x);
                }
            }
        }
        (out[0] as f32, out[1] as f32)
    }
}

// ───────────────────────────── reverb (Freeverb-style) ─────────────────────────────

const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
const SPREAD: usize = 23;
const MAX_PREDELAY_S: f32 = 0.1;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbSettings {
    /// 0..1, scales the room (delay lengths).
    pub size: f32,
    /// Decay time in seconds (RT60-ish).
    pub decay_s: f32,
    /// 0..1 high-frequency damping.
    pub damp: f32,
    pub predelay_s: f32,
    /// 0..1 stereo width.
    pub width: f32,
    pub mix: f32,
}

struct Comb {
    buf: Vec<f32>,
    pos: usize,
    len: usize,
    store: f32,
}

impl Comb {
    #[inline]
    fn process(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let y = self.buf[self.pos];
        self.store = y * (1.0 - damp) + self.store * damp;
        self.buf[self.pos] = x + self.store * feedback;
        self.pos += 1;
        if self.pos >= self.len {
            self.pos = 0;
        }
        y
    }
}

struct Allpass {
    buf: Vec<f32>,
    pos: usize,
}

impl Allpass {
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let b = self.buf[self.pos];
        let y = b - x;
        self.buf[self.pos] = x + b * 0.5;
        self.pos = (self.pos + 1) % self.buf.len();
        y
    }
}

pub struct Reverb {
    sr: f32,
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
    pre: Vec<f32>,
    pre_pos: usize,
    s: ReverbSettings,
    feedback: f32,
}

impl Reverb {
    pub fn new(sr: f32) -> Self {
        let scale = sr / 44_100.0;
        let mk_combs = |extra: usize| {
            COMBS
                .iter()
                .map(|&n| {
                    let len = (((n + extra) as f32) * scale) as usize;
                    Comb { buf: vec![0.0; len.max(1)], pos: 0, len: len.max(1), store: 0.0 }
                })
                .collect::<Vec<_>>()
        };
        let mk_ap = |extra: usize| {
            ALLPASSES.iter().map(|&n| Allpass { buf: vec![0.0; (((n + extra) as f32) * scale) as usize + 1], pos: 0 }).collect::<Vec<_>>()
        };
        let mut r = Reverb {
            sr,
            combs: [mk_combs(0), mk_combs(SPREAD)],
            allpasses: [mk_ap(0), mk_ap(SPREAD)],
            pre: vec![0.0; (MAX_PREDELAY_S * sr) as usize + 2],
            pre_pos: 0,
            s: ReverbSettings { size: 0.7, decay_s: 2.8, damp: 0.4, predelay_s: 0.02, width: 0.9, mix: 0.3 },
            feedback: 0.84,
        };
        r.set(r.s);
        r
    }

    pub fn set(&mut self, s: ReverbSettings) {
        self.s = s;
        let room = 0.45 + 0.55 * s.size.clamp(0.0, 1.0);
        for ch in &mut self.combs {
            for c in ch.iter_mut() {
                c.len = ((c.buf.len() as f32 * room) as usize).clamp(1, c.buf.len());
                if c.pos >= c.len {
                    c.pos = 0;
                }
            }
        }
        // Feedback from decay time: each pass through the mean comb loses 60 dB over decay_s.
        let mean_len = self.combs[0].iter().map(|c| c.len).sum::<usize>() as f32 / 8.0 / self.sr;
        self.feedback = 10f32.powf(-3.0 * mean_len / s.decay_s.max(0.1)).clamp(0.0, 0.98);
    }

    pub fn reset(&mut self) {
        for ch in &mut self.combs {
            for c in ch.iter_mut() {
                c.buf.fill(0.0);
                c.store = 0.0;
            }
        }
        for ch in &mut self.allpasses {
            for a in ch.iter_mut() {
                a.buf.fill(0.0);
            }
        }
        self.pre.fill(0.0);
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.s.mix <= 0.0001 {
            return (l, r);
        }
        // Pre-delay on the mono send.
        let n = self.pre.len();
        self.pre[self.pre_pos] = (l + r) * 0.5;
        let d = ((self.s.predelay_s * self.sr) as usize).min(n - 1);
        let input = self.pre[(self.pre_pos + n - d) % n] * 0.015;
        self.pre_pos = (self.pre_pos + 1) % n;

        let damp = self.s.damp * 0.4;
        let mut out = [0.0f32; 2];
        for ch in 0..2 {
            let mut acc = 0.0;
            for c in self.combs[ch].iter_mut() {
                acc += c.process(input, self.feedback, damp);
            }
            for a in self.allpasses[ch].iter_mut() {
                acc = a.process(acc);
            }
            out[ch] = acc;
        }
        let w = self.s.width;
        let wet1 = 0.5 + w * 0.5;
        let wet2 = 0.5 - w * 0.5;
        let wl = out[0] * wet1 + out[1] * wet2;
        let wr = out[1] * wet1 + out[0] * wet2;
        let m = self.s.mix;
        (l * (1.0 - m) + wl * m * 3.0, r * (1.0 - m) + wr * m * 3.0)
    }
}

// ───────────────────────────── chain ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DelaySettings {
    /// Delay time in seconds (the plugin converts tempo divisions).
    pub time_s: f32,
    /// 0..0.95
    pub feedback: f32,
    pub mix: f32,
    /// Ping-pong: each repeat alternates sides.
    pub ping_pong: bool,
    /// Low-pass in the feedback loop (Hz) — darker repeats like tape.
    pub tone_hz: f32,
}

impl Default for DelaySettings {
    fn default() -> Self {
        DelaySettings { time_s: 0.375, feedback: 0.35, mix: 0.0, ping_pong: true, tone_hz: 5000.0 }
    }
}

/// Stereo feedback delay with smoothed time changes (no zipper/click when the
/// tempo or division changes) and a damped feedback path.
pub struct Delay {
    sr: f32,
    s: DelaySettings,
    buf: [Vec<f32>; 2],
    pos: usize,
    time: f32,
    lp: [f32; 2],
    lp_coef: f32,
}

impl Delay {
    pub fn new(sr: f32) -> Self {
        let n = (2.1 * sr) as usize;
        Delay { sr, s: DelaySettings::default(), buf: [vec![0.0; n], vec![0.0; n]], pos: 0, time: 0.375 * sr, lp: [0.0; 2], lp_coef: 0.5 }
    }

    pub fn set(&mut self, s: DelaySettings) {
        // Glide the time only while the delay is audible; otherwise jump.
        if self.s.mix <= 0.0001 {
            self.time = (s.time_s * self.sr).clamp(1.0, self.buf[0].len() as f32 - 2.0);
        }
        self.s = s;
        self.lp_coef = 1.0 - (-std::f32::consts::TAU * s.tone_hz.clamp(200.0, 18_000.0) / self.sr).exp();
    }

    pub fn reset(&mut self) {
        self.buf.iter_mut().for_each(|b| b.fill(0.0));
        self.lp = [0.0; 2];
    }

    #[inline]
    fn read(&self, ch: usize, d: f32) -> f32 {
        let n = self.buf[ch].len();
        let p = self.pos as f32 - d;
        let p = if p < 0.0 { p + n as f32 } else { p };
        let i = p as usize % n;
        let f = p - p.floor();
        self.buf[ch][i] * (1.0 - f) + self.buf[ch][(i + 1) % n] * f
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.s.mix <= 0.0001 {
            return (l, r);
        }
        let n = self.buf[0].len();
        let target = (self.s.time_s * self.sr).clamp(1.0, n as f32 - 2.0);
        self.time += (target - self.time) * 0.0005;
        let (dl, dr) = (self.read(0, self.time), self.read(1, self.time));
        self.lp[0] += (dl - self.lp[0]) * self.lp_coef;
        self.lp[1] += (dr - self.lp[1]) * self.lp_coef;
        let fb = self.s.feedback.clamp(0.0, 0.95);
        let (wl, wr) = if self.s.ping_pong {
            // Mono input enters the left line; repeats bounce L → R → L.
            (0.5 * (l + r) + self.lp[1] * fb, self.lp[0] * fb)
        } else {
            (l + self.lp[0] * fb, r + self.lp[1] * fb)
        };
        self.buf[0][self.pos] = wl.clamp(-4.0, 4.0);
        self.buf[1][self.pos] = wr.clamp(-4.0, 4.0);
        self.pos = (self.pos + 1) % n;
        let m = self.s.mix;
        (l + dl * m, r + dr * m)
    }
}

/// Master stage: output gain, then a peak limiter (1 ms attack, 120 ms
/// release, ceiling −0.3 dBFS, stereo-linked) with a soft clipper as the
/// last safety net. Replaces the old hard clamp.
pub struct Limiter {
    sr: f32,
    gain: f32,
    env: f32,
    att: f32,
    rel: f32,
    pub reduction_db: f32,
}

pub const LIMIT_CEILING: f32 = 0.966; // −0.3 dBFS

impl Limiter {
    pub fn new(sr: f32) -> Self {
        Limiter { sr, gain: 1.0, env: 1.0, att: 1.0 - (-1.0 / (0.001 * sr)).exp(), rel: 1.0 - (-1.0 / (0.12 * sr)).exp(), reduction_db: 0.0 }
    }

    pub fn set_gain_db(&mut self, db: f32) {
        self.gain = 10f32.powf(db / 20.0);
    }

    pub fn reset(&mut self) {
        self.env = 1.0;
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let (l, r) = (l * self.gain, r * self.gain);
        let peak = l.abs().max(r.abs());
        let want = if peak > LIMIT_CEILING { LIMIT_CEILING / peak } else { 1.0 };
        let c = if want < self.env { self.att } else { self.rel };
        self.env += (want - self.env) * c;
        self.reduction_db = -20.0 * self.env.max(1e-6).log10();
        let _ = self.sr;
        let soft = |x: f32| if x.abs() <= LIMIT_CEILING { x } else { x.signum() * (LIMIT_CEILING + (1.0 - LIMIT_CEILING) * ((x.abs() - LIMIT_CEILING) / (1.0 - LIMIT_CEILING)).tanh()) };
        (soft(l * self.env), soft(r * self.env))
    }
}

pub struct FxChain {
    pub chorus: Chorus,
    pub eq: Eq5,
    pub delay: Delay,
    pub reverb: Reverb,
}

impl FxChain {
    pub fn new(sr: f32) -> Self {
        FxChain { chorus: Chorus::new(sr), eq: Eq5::new(sr), delay: Delay::new(sr), reverb: Reverb::new(sr) }
    }

    pub fn reset(&mut self) {
        self.chorus.reset();
        self.eq.reset();
        self.delay.reset();
        self.reverb.reset();
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let (l, r) = self.chorus.process(l, r);
        let (l, r) = self.eq.process(l, r);
        let (l, r) = self.delay.process(l, r);
        self.reverb.process(l, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverb_tail_decays_and_is_finite() {
        let mut r = Reverb::new(48_000.0);
        r.set(ReverbSettings { size: 1.0, decay_s: 1.0, damp: 0.3, predelay_s: 0.0, width: 1.0, mix: 1.0 });
        r.process(1.0, 1.0);
        let early: f32 = (0..24_000).map(|_| r.process(0.0, 0.0).0.abs()).fold(0.0, f32::max);
        for _ in 0..48_000 * 2 {
            r.process(0.0, 0.0);
        }
        let late: f32 = (0..4800).map(|_| r.process(0.0, 0.0).0.abs()).fold(0.0, f32::max);
        assert!(early.is_finite() && early > 0.0);
        assert!(late < early * 0.01, "early {early} late {late}");
    }

    #[test]
    fn flat_eq_is_transparent() {
        let mut eq = Eq5::new(48_000.0);
        assert_eq!(eq.process(0.5, -0.25), (0.5, -0.25));
    }

    #[test]
    fn delay_repeats_and_limiter_holds_ceiling() {
        let sr = 48_000.0;
        let mut d = Delay::new(sr);
        d.set(DelaySettings { time_s: 0.1, feedback: 0.5, mix: 1.0, ping_pong: false, tone_hz: 18_000.0 });
        let out: Vec<f32> = (0..20_000).map(|i| d.process(if i == 0 { 1.0 } else { 0.0 }, 0.0).0).collect();
        let echo = out[4700..5100].iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(echo > 0.5, "first echo {echo}");

        let mut lim = Limiter::new(sr);
        lim.set_gain_db(12.0);
        let peak = (0..48_000).map(|i| lim.process((i as f32 * 0.05).sin(), 0.0).0.abs()).skip(480).fold(0.0f32, f32::max);
        assert!(peak <= 1.0, "peak {peak}");
    }
}
