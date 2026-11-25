//! Synthetic singing voice for accuracy tests and benchmarks.
//!
//! Glottal-like source (band-limited sawtooth + spectral tilt) through three
//! vowel formants, with vibrato, jitter, amplitude envelopes and optional
//! "laptop mic" degradation: weak lows (HPF ~200 Hz, which removes the
//! fundamental of low voices), small-room reflections and fan-like noise.

use crate::biquad::{Biquad, Coeffs};

#[derive(Clone, Copy, Debug)]
pub struct Segment {
    /// MIDI pitch, or `None` for silence.
    pub midi: Option<f32>,
    pub dur: f32,
}

impl Segment {
    pub fn note(midi: f32, dur: f32) -> Self {
        Segment { midi: Some(midi), dur }
    }
    pub fn rest(dur: f32) -> Self {
        Segment { midi: None, dur }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SynthOpts {
    pub vibrato_cents: f32,
    pub vibrato_hz: f32,
    /// Random pitch wobble in cents.
    pub jitter_cents: f32,
    /// Signal-to-noise ratio of added noise; `None` = clean.
    pub snr_db: Option<f32>,
    pub laptop_mic: bool,
    pub level: f32,
    pub seed: u32,
}

impl Default for SynthOpts {
    fn default() -> Self {
        SynthOpts { vibrato_cents: 25.0, vibrato_hz: 5.5, jitter_cents: 4.0, snr_db: None, laptop_mic: false, level: 0.3, seed: 1 }
    }
}

struct Rng(u32);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

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

/// Render the segments. Returns the signal and the true MIDI pitch per sample
/// (`None` while silent or within the attack/release ramps).
pub fn render(sr: f32, segs: &[Segment], opts: SynthOpts) -> (Vec<f32>, Vec<Option<f32>>) {
    let total: usize = segs.iter().map(|s| (s.dur * sr) as usize).sum();
    let mut sig = Vec::with_capacity(total);
    let mut truth = Vec::with_capacity(total);
    let mut rng = Rng(opts.seed.max(1));
    let srd = sr as f64;
    let mut formants = [
        (Biquad::new(Coeffs::bandpass(srd, 700.0, 6.0)), 1.0f32),
        (Biquad::new(Coeffs::bandpass(srd, 1220.0, 8.0)), 0.5),
        (Biquad::new(Coeffs::bandpass(srd, 2600.0, 10.0)), 0.25),
    ];
    let mut tilt = 0.0f32;
    let mut phase = 0.0f32;
    let mut vib_phase = 0.0f32;
    let mut jitter = 0.0f32;
    let ramp = (0.012 * sr) as usize;

    for seg in segs {
        let n = (seg.dur * sr) as usize;
        for i in 0..n {
            let Some(base) = seg.midi else {
                sig.push(0.0);
                truth.push(None);
                continue;
            };
            vib_phase += opts.vibrato_hz / sr;
            jitter = 0.999 * jitter + 0.001 * rng.next() * opts.jitter_cents * 30.0;
            let cents = opts.vibrato_cents * (2.0 * std::f32::consts::PI * vib_phase).sin() + jitter;
            let midi = base + cents / 100.0;
            let f0 = 440.0 * 2f32.powf((midi - 69.0) / 12.0);
            let dt = f0 / sr;
            phase += dt;
            if phase >= 1.0 {
                phase -= 1.0;
            }
            let saw = 2.0 * phase - 1.0 - poly_blep(phase, dt);
            tilt += 0.3 * (saw - tilt);
            let src = tilt + 0.02 * rng.next(); // breathiness
            let mut v = 0.0;
            for (f, g) in &mut formants {
                v += f.process(src as f64) as f32 * *g;
            }
            v += 0.3 * src; // keep some direct low end (fundamental)
            let env = (i.min(n - 1 - i) as f32 / ramp as f32).min(1.0);
            sig.push(v * env);
            truth.push((i > ramp && i + ramp < n).then_some(midi));
        }
    }

    // Normalise voiced RMS to `level`.
    let voiced: Vec<f32> = sig.iter().zip(&truth).filter(|(_, t)| t.is_some()).map(|(s, _)| *s).collect();
    let rms = (voiced.iter().map(|v| v * v).sum::<f32>() / voiced.len().max(1) as f32).sqrt().max(1e-9);
    sig.iter_mut().for_each(|s| *s *= opts.level / rms);

    if opts.laptop_mic {
        let mut hp = [Biquad::new(Coeffs::highpass(srd, 200.0, 0.7)), Biquad::new(Coeffs::highpass(srd, 200.0, 0.7))];
        let mut lp = Biquad::new(Coeffs::lowpass(srd, 7000.0, 0.7));
        let taps = [(0.007, 0.35f32), (0.013, 0.25), (0.023, 0.18), (0.041, 0.1)];
        let dry = sig.clone();
        for i in 0..sig.len() {
            let mut v = dry[i];
            for &(d, g) in &taps {
                let k = (d * sr) as usize;
                if i >= k {
                    v += dry[i - k] * g;
                }
            }
            let mut y = v as f64;
            for h in &mut hp {
                y = h.process(y);
            }
            sig[i] = lp.process(y) as f32;
        }
    }

    if let Some(snr) = opts.snr_db {
        let noise_rms = opts.level / 10f32.powf(snr / 20.0);
        // Fan-like noise: mostly low-passed hum-free rumble plus hiss.
        let mut lpn = Biquad::new(Coeffs::lowpass(srd, 900.0, 0.7));
        let mut buf: Vec<f32> = (0..sig.len()).map(|_| 0.7 * lpn.process(rng.next() as f64) as f32 * 3.0 + 0.3 * rng.next()).collect();
        let nr = (buf.iter().map(|v| v * v).sum::<f32>() / buf.len().max(1) as f32).sqrt().max(1e-9);
        buf.iter_mut().for_each(|v| *v *= noise_rms / nr);
        sig.iter_mut().zip(&buf).for_each(|(s, n)| *s += n);
    }
    (sig, truth)
}
