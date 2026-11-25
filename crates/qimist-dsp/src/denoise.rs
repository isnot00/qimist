//! STFT spectral-subtraction denoiser for the analysis path.
//!
//! Laptop mics pick up steady fan/hiss noise that flattens the YIN dip. We keep a
//! per-bin noise profile using minimum tracking (fast down, slow up, frozen while
//! the frame is clearly louder than the noise), then apply an over-subtracting
//! gain with a spectral floor and per-bin release smoothing to avoid
//! "musical noise". This only feeds the detector, never the audio output.
//!
//! Sample in / sample out with a fixed latency of `N` samples.

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

pub const N: usize = 256;
pub const HOP: usize = N / 4;
const BINS: usize = N / 2 + 1;
const GAIN_FLOOR: f32 = 0.08;

pub struct Denoiser {
    fft: Arc<dyn Fft<f32>>,
    ifft: Arc<dyn Fft<f32>>,
    window: [f32; N],
    frame: [f32; N],
    ola: [f32; N],
    inq: [f32; HOP],
    outq: [f32; HOP],
    idx: usize,
    buf: Vec<Complex32>,
    scratch: Vec<Complex32>,
    noise: [f32; BINS],
    gain: [f32; BINS],
    noise_energy: f32,
    frames_seen: u32,
    /// Per-frame multiplicative rise for the noise estimate (≈3 dB/s).
    rise: f32,
    strength: f32,
}

impl Denoiser {
    pub fn new(sr: f64) -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(N);
        let ifft = planner.plan_fft_inverse(N);
        let scratch_len = fft.get_inplace_scratch_len().max(ifft.get_inplace_scratch_len());
        let mut window = [0.0; N];
        for (i, w) in window.iter_mut().enumerate() {
            // sqrt of periodic Hann → analysis·synthesis = Hann, which sums to 2 at 75% overlap.
            let h = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N as f32).cos();
            *w = h.sqrt();
        }
        let frames_per_sec = sr as f32 / HOP as f32;
        Denoiser {
            fft,
            ifft,
            window,
            frame: [0.0; N],
            ola: [0.0; N],
            inq: [0.0; HOP],
            outq: [0.0; HOP],
            idx: 0,
            buf: vec![Complex32::new(0.0, 0.0); N],
            scratch: vec![Complex32::new(0.0, 0.0); scratch_len],
            noise: [0.0; BINS],
            gain: [1.0; BINS],
            noise_energy: 0.0,
            frames_seen: 0,
            rise: 2f32.powf(1.0 / frames_per_sec),
            strength: 0.6,
        }
    }

    /// 0 = bypass, 1 = aggressive.
    pub fn set_strength(&mut self, s: f32) {
        self.strength = s.clamp(0.0, 1.0);
    }

    pub const fn latency() -> usize {
        N
    }

    pub fn reset(&mut self) {
        self.frame = [0.0; N];
        self.ola = [0.0; N];
        self.inq = [0.0; HOP];
        self.outq = [0.0; HOP];
        self.idx = 0;
        self.noise = [0.0; BINS];
        self.gain = [1.0; BINS];
        self.noise_energy = 0.0;
        self.frames_seen = 0;
    }

    #[inline]
    pub fn push(&mut self, x: f32) -> f32 {
        let y = self.outq[self.idx];
        self.inq[self.idx] = x;
        self.idx += 1;
        if self.idx == HOP {
            self.idx = 0;
            self.run_frame();
        }
        y
    }

    fn run_frame(&mut self) {
        self.frame.copy_within(HOP.., 0);
        self.frame[N - HOP..].copy_from_slice(&self.inq);

        for i in 0..N {
            self.buf[i] = Complex32::new(self.frame[i] * self.window[i], 0.0);
        }
        self.fft.process_with_scratch(&mut self.buf, &mut self.scratch);

        let mut energy = 0.0;
        for k in 0..BINS {
            energy += self.buf[k].norm_sqr();
        }

        // Noise profile: initialise from the first frames, then minimum-track.
        self.frames_seen = self.frames_seen.saturating_add(1);
        let learning = self.frames_seen <= 16;
        let loud = energy > 4.0 * self.noise_energy.max(1e-12);
        for k in 0..BINS {
            let p = self.buf[k].norm_sqr();
            let n = &mut self.noise[k];
            if learning {
                *n += (p - *n) / self.frames_seen as f32;
            } else if p < *n {
                *n = 0.7 * *n + 0.3 * p;
            } else if !loud {
                *n = (*n * self.rise).min(p);
            }
        }
        self.noise_energy = if learning || energy < self.noise_energy {
            0.7 * self.noise_energy + 0.3 * energy
        } else if !loud {
            self.noise_energy * self.rise
        } else {
            self.noise_energy
        };

        let alpha = 1.0 + 3.0 * self.strength;
        let bypass = self.strength <= 0.001;
        for k in 0..BINS {
            let g = if bypass {
                1.0
            } else {
                let p = self.buf[k].norm_sqr().max(1e-20);
                (1.0 - alpha * self.noise[k] / p).max(GAIN_FLOOR).sqrt()
            };
            // Instant attack, smoothed release reduces musical noise.
            let gs = &mut self.gain[k];
            *gs = if g > *gs { g } else { 0.5 * *gs + 0.5 * g };
            let gain = *gs;
            self.buf[k] *= gain;
            if k != 0 && k != N / 2 {
                self.buf[N - k] *= gain;
            }
        }

        self.ifft.process_with_scratch(&mut self.buf, &mut self.scratch);
        let scale = 0.5 / N as f32;
        for i in 0..N {
            self.ola[i] += self.buf[i].re * self.window[i] * scale;
        }
        self.outq.copy_from_slice(&self.ola[..HOP]);
        self.ola.copy_within(HOP.., 0);
        self.ola[N - HOP..].fill(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bypass_reconstructs_signal() {
        let mut d = Denoiser::new(16_000.0);
        d.set_strength(0.0);
        let sig: Vec<f32> = (0..4000).map(|i| (i as f32 * 0.05).sin()).collect();
        let out: Vec<f32> = sig.iter().map(|&x| d.push(x)).collect();
        let lat = Denoiser::latency();
        let mut err = 0.0f32;
        for i in 1000..3000 {
            err = err.max((out[i] - sig[i - lat]).abs());
        }
        assert!(err < 1e-3, "max reconstruction error {err}");
    }
}
