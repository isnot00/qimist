//! RBJ-cookbook biquads (transposed direct form II, f64 state).
//!
//! f64 state matters for the 65 Hz high-pass at 48 kHz+, where f32 TDF2 gets noisy.

use std::f64::consts::PI;

/// Q values for a 4th-order Butterworth built from two cascaded biquads.
pub const BUTTERWORTH4_Q: [f64; 2] = [0.541_196_1, 1.306_563];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coeffs {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Coeffs {
    pub const IDENTITY: Coeffs = Coeffs { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 };

    fn norm(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Coeffs { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }

    fn prep(sr: f64, f: f64, q: f64) -> (f64, f64, f64) {
        let f = f.clamp(1.0, sr * 0.49);
        let w = 2.0 * PI * f / sr;
        let (s, c) = w.sin_cos();
        (s, c, s / (2.0 * q.max(1e-3)))
    }

    pub fn lowpass(sr: f64, f: f64, q: f64) -> Self {
        let (_, c, a) = Self::prep(sr, f, q);
        Self::norm((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + a, -2.0 * c, 1.0 - a)
    }

    pub fn highpass(sr: f64, f: f64, q: f64) -> Self {
        let (_, c, a) = Self::prep(sr, f, q);
        Self::norm((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + a, -2.0 * c, 1.0 - a)
    }

    pub fn notch(sr: f64, f: f64, q: f64) -> Self {
        let (_, c, a) = Self::prep(sr, f, q);
        Self::norm(1.0, -2.0 * c, 1.0, 1.0 + a, -2.0 * c, 1.0 - a)
    }

    pub fn bandpass(sr: f64, f: f64, q: f64) -> Self {
        let (_, c, a) = Self::prep(sr, f, q);
        Self::norm(a, 0.0, -a, 1.0 + a, -2.0 * c, 1.0 - a)
    }

    pub fn peaking(sr: f64, f: f64, q: f64, gain_db: f64) -> Self {
        let (_, c, a) = Self::prep(sr, f, q);
        let g = 10f64.powf(gain_db / 40.0);
        Self::norm(1.0 + a * g, -2.0 * c, 1.0 - a * g, 1.0 + a / g, -2.0 * c, 1.0 - a / g)
    }

    pub fn low_shelf(sr: f64, f: f64, q: f64, gain_db: f64) -> Self {
        let (_, c, a) = Self::prep(sr, f, q);
        let g = 10f64.powf(gain_db / 40.0);
        let k = 2.0 * g.sqrt() * a;
        Self::norm(
            g * ((g + 1.0) - (g - 1.0) * c + k),
            2.0 * g * ((g - 1.0) - (g + 1.0) * c),
            g * ((g + 1.0) - (g - 1.0) * c - k),
            (g + 1.0) + (g - 1.0) * c + k,
            -2.0 * ((g - 1.0) + (g + 1.0) * c),
            (g + 1.0) + (g - 1.0) * c - k,
        )
    }

    pub fn high_shelf(sr: f64, f: f64, q: f64, gain_db: f64) -> Self {
        let (_, c, a) = Self::prep(sr, f, q);
        let g = 10f64.powf(gain_db / 40.0);
        let k = 2.0 * g.sqrt() * a;
        Self::norm(
            g * ((g + 1.0) + (g - 1.0) * c + k),
            -2.0 * g * ((g - 1.0) + (g + 1.0) * c),
            g * ((g + 1.0) + (g - 1.0) * c - k),
            (g + 1.0) - (g - 1.0) * c + k,
            2.0 * ((g - 1.0) - (g + 1.0) * c),
            (g + 1.0) - (g - 1.0) * c - k,
        )
    }

    /// Magnitude response in dB at `f` Hz (for drawing EQ curves).
    pub fn magnitude_db(&self, sr: f64, f: f64) -> f64 {
        let w = 2.0 * PI * f / sr;
        let (s1, c1) = w.sin_cos();
        let (s2, c2) = (2.0 * w).sin_cos();
        let nr = self.b0 + self.b1 * c1 + self.b2 * c2;
        let ni = -(self.b1 * s1 + self.b2 * s2);
        let dr = 1.0 + self.a1 * c1 + self.a2 * c2;
        let di = -(self.a1 * s1 + self.a2 * s2);
        10.0 * ((nr * nr + ni * ni) / (dr * dr + di * di)).max(1e-30).log10()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    c: Coeffs,
    z1: f64,
    z2: f64,
}

impl Default for Biquad {
    fn default() -> Self {
        Biquad { c: Coeffs::IDENTITY, z1: 0.0, z2: 0.0 }
    }
}

impl Biquad {
    pub fn new(c: Coeffs) -> Self {
        Biquad { c, z1: 0.0, z2: 0.0 }
    }

    pub fn set(&mut self, c: Coeffs) {
        self.c = c;
    }

    pub fn coeffs(&self) -> Coeffs {
        self.c
    }

    pub fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let c = &self.c;
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highpass_attenuates_hum() {
        let sr = 48_000.0;
        let c = Coeffs::highpass(sr, 65.0, BUTTERWORTH4_Q[0]);
        assert!(c.magnitude_db(sr, 1000.0).abs() < 0.1);
        assert!(c.magnitude_db(sr, 20.0) < -15.0);
    }

    #[test]
    fn peaking_gain_at_center() {
        let sr = 48_000.0;
        let c = Coeffs::peaking(sr, 1000.0, 1.0, 6.0);
        assert!((c.magnitude_db(sr, 1000.0) - 6.0).abs() < 0.05);
    }
}
