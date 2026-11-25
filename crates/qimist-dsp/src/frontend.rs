//! Analysis front end: cleans the mic signal and decimates it to ~16 kHz.
//!
//! - 4th-order Butterworth high-pass at 65 Hz removes rumble, desk thumps, DC.
//! - Optional notches at 50/60 Hz remove mains hum. (No notches at 100/120 Hz:
//!   those sit inside the singing range, and a 110 Hz voice makes a sharp
//!   notch ring for >100 ms after the note ends.)
//! - 4th-order Butterworth low-pass at 2.8 kHz: keeps the fundamental and the
//!   first several harmonics (enough for periodicity), drops hiss and fan noise,
//!   and doubles as the anti-alias filter for decimation.

use crate::biquad::{Biquad, Coeffs, BUTTERWORTH4_Q};

pub const TARGET_SR: f64 = 16_000.0;
const HPF_HZ: f64 = 65.0;
const LPF_HZ: f64 = 2_800.0;
const HUM_FREQS: [f64; 2] = [50.0, 60.0];
const HUM_Q: f64 = 6.0;

pub struct FrontEnd {
    hp: [Biquad; 2],
    hum: [Biquad; 2],
    lp: [Biquad; 2],
    hum_enabled: bool,
    decim: usize,
    phase: usize,
    out_sr: f64,
}

impl FrontEnd {
    pub fn new(sr: f64) -> Self {
        let decim = (sr / TARGET_SR).round().max(1.0) as usize;
        let out_sr = sr / decim as f64;
        let lp_hz = LPF_HZ.min(0.4 * out_sr);
        FrontEnd {
            hp: BUTTERWORTH4_Q.map(|q| Biquad::new(Coeffs::highpass(sr, HPF_HZ, q))),
            hum: HUM_FREQS.map(|f| Biquad::new(Coeffs::notch(sr, f, HUM_Q))),
            lp: BUTTERWORTH4_Q.map(|q| Biquad::new(Coeffs::lowpass(sr, lp_hz, q))),
            hum_enabled: true,
            decim,
            phase: 0,
            out_sr,
        }
    }

    pub fn set_hum_notch(&mut self, on: bool) {
        self.hum_enabled = on;
    }

    pub fn out_sr(&self) -> f64 {
        self.out_sr
    }

    pub fn decimation(&self) -> usize {
        self.decim
    }

    pub fn reset(&mut self) {
        self.hp.iter_mut().chain(self.hum.iter_mut()).chain(self.lp.iter_mut()).for_each(|b| b.reset());
        self.phase = 0;
    }

    /// Feed one host-rate sample; returns a decimated sample every `decim` calls.
    #[inline]
    pub fn push(&mut self, x: f32) -> Option<f32> {
        let mut y = x as f64;
        for b in &mut self.hp {
            y = b.process(y);
        }
        if self.hum_enabled {
            for b in &mut self.hum {
                y = b.process(y);
            }
        }
        for b in &mut self.lp {
            y = b.process(y);
        }
        self.phase += 1;
        if self.phase >= self.decim {
            self.phase = 0;
            Some(y as f32)
        } else {
            None
        }
    }
}
