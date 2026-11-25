//! pYIN candidate extraction (Mauch & Dixon, 2014).
//!
//! Classic YIN picks one period using one threshold on the cumulative mean
//! normalised difference function (CMNDF). pYIN instead sweeps 100 thresholds
//! weighted by a Beta prior, and every threshold "votes" for the first CMNDF dip
//! below it. The result is a small set of period candidates with probabilities,
//! which the HMM then tracks over time. This is far more robust against octave
//! errors and noise than single-threshold YIN.

pub const MAX_CANDIDATES: usize = 12;
const N_THRESH: usize = 100;
/// Mass given to the global minimum when no dip falls below a threshold.
const LOW_AMP_PRIOR: f32 = 0.01;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Candidate {
    pub freq: f32,
    pub prob: f32,
    /// CMNDF value at the dip (0 = perfectly periodic).
    pub dip: f32,
}

pub struct Yin {
    sr: f32,
    w: usize,
    tau_min: usize,
    tau_max: usize,
    diff: Vec<f32>,
    cmnd: Vec<f32>,
    troughs: Vec<usize>,
    trough_prob: Vec<f32>,
    thresh_prob: [f32; N_THRESH],
}

impl Yin {
    /// `w` is the integration window in samples; the frame passed to
    /// [`Yin::analyze`] must be at least [`Yin::frame_len`] long.
    pub fn new(sr: f32, fmin: f32, fmax: f32, w: usize) -> Self {
        let tau_min = ((sr / fmax).floor() as usize).max(2);
        let tau_max = ((sr / fmin).ceil() as usize).max(tau_min + 2);
        let mut y = Yin {
            sr,
            w,
            tau_min,
            tau_max,
            diff: vec![0.0; tau_max + 2],
            cmnd: vec![0.0; tau_max + 2],
            troughs: Vec::with_capacity(tau_max),
            trough_prob: Vec::with_capacity(tau_max),
            thresh_prob: [0.0; N_THRESH],
        };
        y.set_threshold_mean(0.15);
        y
    }

    pub fn frame_len(&self) -> usize {
        self.w + self.tau_max + 2
    }

    /// Mean of the Beta threshold prior (pYIN uses 0.1, 0.15 or 0.2).
    /// Higher means more permissive: more frames voiced in noise.
    pub fn set_threshold_mean(&mut self, mean: f32) {
        let mean = mean.clamp(0.05, 0.3);
        let a = 2.0f32;
        let b = a * (1.0 - mean) / mean;
        let mut sum = 0.0;
        for i in 0..N_THRESH {
            let x = (i as f32 + 0.5) / N_THRESH as f32;
            let p = x.powf(a - 1.0) * (1.0 - x).powf(b - 1.0);
            self.thresh_prob[i] = p;
            sum += p;
        }
        self.thresh_prob.iter_mut().for_each(|p| *p /= sum);
    }

    /// Analyse one frame. Writes candidates sorted by probability (desc) and
    /// returns how many were written.
    pub fn analyze(&mut self, x: &[f32], out: &mut [Candidate; MAX_CANDIDATES]) -> usize {
        let w = self.w;
        let tmax = self.tau_max + 1;
        debug_assert!(x.len() >= w + tmax);

        // Difference function, using d(τ) = e0 + eτ − 2·r(τ).
        let e0: f32 = x[..w].iter().map(|v| v * v).sum();
        let mut et = e0;
        self.diff[0] = 0.0;
        for tau in 1..=tmax {
            // Slide the energy of x[τ..τ+w].
            et += x[tau + w - 1] * x[tau + w - 1] - x[tau - 1] * x[tau - 1];
            let r: f32 = x[..w].iter().zip(&x[tau..tau + w]).map(|(a, b)| a * b).sum();
            self.diff[tau] = (e0 + et - 2.0 * r).max(0.0);
        }

        // Cumulative mean normalised difference.
        self.cmnd[0] = 1.0;
        let mut running = 0.0;
        for tau in 1..=tmax {
            running += self.diff[tau];
            self.cmnd[tau] = if running > 0.0 { self.diff[tau] * tau as f32 / running } else { 1.0 };
        }

        // Local minima within the allowed period range.
        self.troughs.clear();
        for tau in self.tau_min.max(1)..=self.tau_max {
            let c = self.cmnd[tau];
            if c < self.cmnd[tau - 1] && c <= self.cmnd[tau + 1] {
                self.troughs.push(tau);
            }
        }
        if self.troughs.is_empty() {
            return 0;
        }

        // Each threshold votes for the first dip below it.
        self.trough_prob.clear();
        self.trough_prob.resize(self.troughs.len(), 0.0);
        let global_min = (0..self.troughs.len())
            .min_by(|&a, &b| self.cmnd[self.troughs[a]].total_cmp(&self.cmnd[self.troughs[b]]))
            .unwrap();
        for i in 0..N_THRESH {
            let s = (i as f32 + 1.0) / N_THRESH as f32;
            match self.troughs.iter().position(|&t| self.cmnd[t] < s) {
                Some(f) => self.trough_prob[f] += self.thresh_prob[i],
                None => self.trough_prob[global_min] += self.thresh_prob[i] * LOW_AMP_PRIOR,
            }
        }

        // Collect, interpolate, sort.
        let mut n = 0;
        for (k, &tau) in self.troughs.iter().enumerate() {
            let p = self.trough_prob[k];
            if p <= 0.0 {
                continue;
            }
            let (a, b, c) = (self.cmnd[tau - 1], self.cmnd[tau], self.cmnd[tau + 1]);
            let den = a - 2.0 * b + c;
            let shift = if den.abs() > 1e-12 { (0.5 * (a - c) / den).clamp(-1.0, 1.0) } else { 0.0 };
            let cand = Candidate { freq: self.sr / (tau as f32 + shift), prob: p, dip: b };
            if n < MAX_CANDIDATES {
                out[n] = cand;
                n += 1;
            } else if let Some(min_i) = (0..n).min_by(|&i, &j| out[i].prob.total_cmp(&out[j].prob)) {
                if out[min_i].prob < p {
                    out[min_i] = cand;
                }
            }
        }
        out[..n].sort_unstable_by(|a, b| b.prob.total_cmp(&a.prob));
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saw(sr: f32, f: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| ((i as f32 * f / sr).fract() * 2.0 - 1.0) * 0.5).collect()
    }

    #[test]
    fn finds_pitch_of_sawtooth() {
        let sr = 16_000.0;
        let mut yin = Yin::new(sr, 60.0, 1200.0, 384);
        for f in [82.4f32, 110.0, 220.0, 440.0, 880.0] {
            let x = saw(sr, f, yin.frame_len());
            let mut c = [Candidate::default(); MAX_CANDIDATES];
            let n = yin.analyze(&x, &mut c);
            assert!(n > 0);
            let cents = 1200.0 * (c[0].freq / f).log2();
            assert!(cents.abs() < 10.0, "{f} Hz → {} Hz ({cents} cents)", c[0].freq);
        }
    }

    #[test]
    fn silence_gives_no_confident_candidate() {
        let mut yin = Yin::new(16_000.0, 60.0, 1200.0, 384);
        let x = vec![0.0; yin.frame_len()];
        let mut c = [Candidate::default(); MAX_CANDIDATES];
        let n = yin.analyze(&x, &mut c);
        let total: f32 = c[..n].iter().map(|c| c.prob).sum();
        assert!(total < 0.05);
    }
}
