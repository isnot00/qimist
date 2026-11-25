//! Echo canceller for the analysis path.
//!
//! On laptop speakers the mic hears Qimist's own output (loops, chords,
//! drums), which drowns the voice for the pitch and beatbox detectors. We know
//! exactly what we played, so an adaptive filter (NLMS) learns the path from
//! speaker to mic and subtracts the echo. It runs at ~16 kHz (all the
//! detectors need) over a 64 ms window, which covers the audio buffers plus
//! the room. It's a two-path canceller: a background filter adapts all the
//! time, and a frozen foreground copy does the cancelling. Each block a frozen
//! snapshot of the background is put on trial; it replaces the foreground only
//! if, frozen, it removes clearly more. A frozen filter can only produce what
//! is in the playback, so it can't learn the singer away (a constantly adapting
//! filter can, by wobbling its weights in step with the voice). With no output
//! playing it steps aside and the mic passes straight through.

use crate::biquad::{Biquad, Coeffs};

const TAPS: usize = 1024;
const MU: f32 = 0.5;
/// Compare the two filters over blocks of this many (16 kHz) samples.
const BLOCK: usize = 512;

#[inline]
fn cascade(f: &mut [Biquad; 2], x: f32) -> f32 {
    let y = f[0].process(x as f64);
    f[1].process(y) as f32
}

pub struct EchoCanceller {
    dec: usize,
    phase: usize,
    lp_mic: [Biquad; 2],
    lp_ref: [Biquad; 2],
    up: [Biquad; 2],
    /// Reference history, stored twice so `x[pos..pos + TAPS]` is contiguous (newest first).
    x: Vec<f32>,
    pos: usize,
    w: Vec<f32>,
    /// Frozen copy that produces the output.
    wf: Vec<f32>,
    /// Frozen snapshot of the background on trial this block.
    wc: Vec<f32>,
    block_n: usize,
    e_cand: f32,
    e_fg: f32,
    e_mic: f32,
    x_energy: f32,
    echo_pow: f32,
    err_pow: f32,
    /// Samples since the output last had signal; the canceller fades out after ~1 s.
    idle: usize,
    mix: f32,
    hold: f32,
    sr: f32,
}

impl EchoCanceller {
    pub fn new(sr: f32) -> Self {
        let dec = (sr / 16_000.0).round().max(1.0) as usize;
        let sr_d = sr / dec as f32;
        let lp = Coeffs::lowpass(sr as f64, (0.42 * sr_d) as f64, 0.7071);
        EchoCanceller {
            dec,
            phase: 0,
            lp_mic: [Biquad::new(lp); 2],
            lp_ref: [Biquad::new(lp); 2],
            up: [Biquad::new(lp); 2],
            x: vec![0.0; 2 * TAPS],
            pos: 0,
            w: vec![0.0; TAPS],
            wf: vec![0.0; TAPS],
            wc: vec![0.0; TAPS],
            block_n: 0,
            e_cand: 0.0,
            e_fg: 0.0,
            e_mic: 0.0,
            x_energy: 0.0,
            echo_pow: 0.0,
            err_pow: 0.0,
            idle: usize::MAX / 2,
            mix: 0.0,
            hold: 0.0,
            sr,
        }
    }

    pub fn reset(&mut self) {
        self.x.fill(0.0);
        self.w.fill(0.0);
        self.wf.fill(0.0);
        self.wc.fill(0.0);
        self.x_energy = 0.0;
        self.echo_pow = 0.0;
        self.err_pow = 0.0;
    }

    /// How much echo is being removed right now, in dB (for display).
    pub fn reduction_db(&self) -> f32 {
        if self.mix < 0.5 || self.err_pow <= 0.0 {
            return 0.0;
        }
        (10.0 * ((self.echo_pow + self.err_pow) / self.err_pow).log10()).clamp(0.0, 40.0)
    }

    fn nlms(&mut self, mic: f32, reference: f32) -> f32 {
        // Push the newest reference sample (both copies) and keep the window energy.
        let old = self.x[self.pos + TAPS - 1];
        self.pos = if self.pos == 0 { TAPS - 1 } else { self.pos - 1 };
        self.x[self.pos] = reference;
        self.x[self.pos + TAPS] = reference;
        self.x_energy = (self.x_energy + reference * reference - old * old).max(0.0);

        let xs = &self.x[self.pos..self.pos + TAPS];
        let (mut yb, mut yf, mut yc) = (0.0f32, 0.0f32, 0.0f32);
        for (((a, b), f), c) in xs.iter().zip(&self.w).zip(&self.wf).zip(&self.wc) {
            yb += a * b;
            yf += a * f;
            yc += a * c;
        }
        let eb = mic - yb;
        let ef = mic - yf;
        let ec = mic - yc;

        // Background: plain NLMS, always adapting.
        let step = MU * eb / (self.x_energy + 1e-3);
        for (w, a) in self.w.iter_mut().zip(xs) {
            *w += step * a;
        }

        // Foreground: promote the frozen candidate when, over a block, it
        // removes clearly more than the foreground (frozen filters can't chase the voice).
        self.e_cand += ec * ec;
        self.e_fg += ef * ef;
        self.e_mic += mic * mic;
        self.block_n += 1;
        if self.block_n == BLOCK {
            if self.e_cand < 0.8 * self.e_fg && self.e_cand < self.e_mic {
                self.wf.copy_from_slice(&self.wc);
            } else if self.e_fg > 1.1 * self.e_mic {
                // The foreground is adding sound, not removing it: drop it.
                self.wf.fill(0.0);
            }
            self.wc.copy_from_slice(&self.w);
            self.block_n = 0;
            self.e_cand = 0.0;
            self.e_fg = 0.0;
            self.e_mic = 0.0;
        }

        self.echo_pow += (yf * yf - self.echo_pow) * 0.002;
        self.err_pow += (ef * ef - self.err_pow) * 0.002;
        ef
    }

    /// One host-rate sample: the mic and what we just played. Returns the mic
    /// with the echo removed, at the host rate (analysis use only).
    #[inline]
    pub fn process(&mut self, mic: f32, played: f32) -> f32 {
        if played.abs() > 1e-4 {
            self.idle = 0;
        } else {
            self.idle = self.idle.saturating_add(1);
        }
        let active = (self.idle as f32) < self.sr;
        self.mix += ((if active { 1.0 } else { 0.0 }) - self.mix) * 0.0005;
        if self.mix < 1e-3 && !active {
            return mic;
        }

        let m = cascade(&mut self.lp_mic, mic);
        let r = cascade(&mut self.lp_ref, played);
        self.phase += 1;
        let stuffed = if self.phase >= self.dec {
            self.phase = 0;
            self.hold = self.nlms(m, r);
            self.hold * self.dec as f32
        } else {
            0.0
        };
        let cleaned = cascade(&mut self.up, stuffed);
        mic + (cleaned - mic) * self.mix
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
    }

    #[test]
    fn removes_speaker_echo_and_keeps_the_voice() {
        let sr = 48_000.0;
        let n = (sr * 6.0) as usize;
        // What Qimist plays: a sustained synth chord with some movement.
        let played: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / sr;
                let saw = |f: f32| 2.0 * (f * t).fract() - 1.0;
                0.2 * (saw(130.8) + saw(164.8) + saw(196.0)) * (0.7 + 0.3 * (TAU * 0.5 * t).sin())
            })
            .collect();
        // Speaker → mic: 9 ms late, quieter, a bit duller, plus a 23 ms reflection.
        let mut lp = Biquad::new(Coeffs::lowpass(sr as f64, 3000.0, 0.7));
        let echo: Vec<f32> = (0..n)
            .map(|i| {
                let d1 = i.checked_sub(432).map_or(0.0, |k| played[k]);
                let d2 = i.checked_sub(1100).map_or(0.0, |k| played[k]);
                lp.process((0.6 * d1 + 0.25 * d2) as f64) as f32
            })
            .collect();
        // The singer joins after 3 s.
        let voice: Vec<f32> = (0..n).map(|i| if i as f32 / sr > 3.0 { 0.08 * (TAU * 220.0 * i as f32 / sr).sin() } else { 0.0 }).collect();
        let mut aec = EchoCanceller::new(sr);
        let out: Vec<f32> = (0..n).map(|i| aec.process(echo[i] + voice[i], played[i])).collect();

        // Echo alone (2–3 s): well suppressed.
        let a = (2.0 * sr) as usize..(3.0 * sr) as usize;
        let erle = 20.0 * (rms(&echo[a.clone()]) / rms(&out[a.clone()])).log10();
        assert!(erle > 15.0, "echo reduced by only {erle:.1} dB");

        // With the voice (5–6 s): what's left is mostly voice, at about its level.
        let b = (5.0 * sr) as usize..n;
        // The filters delay the voice a little: best-fit delay and gain first.
        let (resid, gain) = (0..80)
            .map(|lag| {
                let v: Vec<f32> = b.clone().map(|i| voice[i - lag]).collect();
                let o = &out[b.clone()];
                let g = o.iter().zip(&v).map(|(a, b)| a * b).sum::<f32>() / v.iter().map(|x| x * x).sum::<f32>();
                (o.iter().zip(&v).map(|(a, b)| a - g * b).collect::<Vec<f32>>(), g)
            })
            .min_by(|a, b| rms(&a.0).total_cmp(&rms(&b.0)))
            .unwrap();
        assert!((0.7..1.3).contains(&gain), "voice level changed: gain {gain}");
        let voice_to_rest = 20.0 * (rms(&voice[b.clone()]) / rms(&resid)).log10();
        assert!(voice_to_rest > 8.0, "voice only {voice_to_rest:.1} dB above what's left");
        let before = 20.0 * (rms(&voice[b.clone()]) / rms(&echo[b.clone()])).log10();
        assert!(voice_to_rest > before + 12.0, "improved {before:.1} → {voice_to_rest:.1} dB");
    }

    #[test]
    fn never_learns_the_voice_away() {
        // A steady tone playing (no echo reaches the mic) while someone sings:
        // the voice must come through at full level.
        let sr = 48_000.0;
        let mut aec = EchoCanceller::new(sr);
        let (mut vin, mut vout) = (0.0, 0.0);
        for i in 0..(4.0 * sr) as usize {
            let t = i as f32 / sr;
            let v = 0.08 * (TAU * 220.0 * t).sin();
            let o = aec.process(v, 0.1 * (TAU * 131.0 * t).sin());
            if t > 2.0 {
                vin += v * v;
                vout += o * o;
            }
        }
        let db = 10.0 * (vout / vin).log10();
        assert!(db.abs() < 1.5, "voice changed by {db:.1} dB");
    }

    #[test]
    fn passes_mic_through_when_nothing_plays() {
        let mut aec = EchoCanceller::new(48_000.0);
        for i in 0..48_000 {
            let x = (i as f32 * 0.01).sin() * 0.3;
            let y = aec.process(x, 0.0);
            assert_eq!(x, y);
        }
    }
}

#[cfg(test)]
mod bench {
    /// Cost while cancelling (run with --ignored).
    #[test]
    #[ignore]
    fn aec_cpu() {
        let sr = 48_000.0;
        let mut aec = super::EchoCanceller::new(sr);
        let t = std::time::Instant::now();
        let mut acc = 0.0;
        for i in 0..480_000 {
            let x = (i as f32 * 0.0171).sin() * 0.2;
            acc += aec.process(x * 0.5 + (i as f32 * 0.029).sin() * 0.05, x);
        }
        println!("aec: {:.1}% of one core ({acc})", t.elapsed().as_secs_f32() / 10.0 * 100.0);
    }
}
