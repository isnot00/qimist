//! Pitch-tracking HMM with online fixed-lag Viterbi decoding.
//!
//! States: `nb` pitch bins (20 cents each) × {voiced, unvoiced}. Pitch moves
//! between neighbouring bins with a triangular transition prior; voicing flips
//! with a small probability. Decoding the most likely *path* instead of
//! trusting each frame alone removes octave jumps and single-frame glitches.
//!
//! Real-time: we run the Viterbi recursion every frame and backtrack `lag`
//! frames to emit the state at `t − lag`. A small lag (a few frames, ~10-30 ms)
//! gives almost all the benefit of offline decoding.

use crate::yin::Candidate;

pub const BINS_PER_SEMITONE: f32 = 5.0;
/// pYIN's "yinTrust": how much the candidate probabilities are believed.
const TRUST: f32 = 0.5;
const SWITCH_PROB: f32 = 0.01;
const OBS_FLOOR: f32 = 1e-12;

pub struct PitchHmm {
    nb: usize,
    fmin: f32,
    jump: usize,
    log_trans: Vec<f32>,
    log_stay: f32,
    log_switch: f32,
    delta: Vec<f32>,
    next: Vec<f32>,
    log_obs: Vec<f32>,
    /// Ring of backpointers, `(lag + 1) × 2nb`.
    back: Vec<u16>,
    lag: usize,
    t: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HmmState {
    pub voiced: bool,
    pub bin: usize,
}

impl PitchHmm {
    pub fn new(fmin: f32, fmax: f32, lag: usize, max_jump_semitones: f32) -> Self {
        let nb = ((fmax / fmin).log2() * 12.0 * BINS_PER_SEMITONE).ceil() as usize + 1;
        assert!(2 * nb < u16::MAX as usize);
        let jump = (max_jump_semitones * BINS_PER_SEMITONE).round() as usize;
        let mut log_trans = vec![0.0; jump + 1];
        let norm: f32 = (0..=2 * jump).map(|k| (jump + 1) as f32 - (k as f32 - jump as f32).abs()).sum();
        for (d, lt) in log_trans.iter_mut().enumerate() {
            *lt = (((jump + 1 - d) as f32) / norm).ln();
        }
        let uniform = -((2 * nb) as f32).ln();
        PitchHmm {
            nb,
            fmin,
            jump,
            log_trans,
            log_stay: (1.0 - SWITCH_PROB).ln(),
            log_switch: SWITCH_PROB.ln(),
            delta: vec![uniform; 2 * nb],
            next: vec![0.0; 2 * nb],
            log_obs: vec![0.0; 2 * nb],
            back: vec![0; (lag + 1) * 2 * nb],
            lag,
            t: 0,
        }
    }

    pub fn n_bins(&self) -> usize {
        self.nb
    }

    pub fn lag(&self) -> usize {
        self.lag
    }

    pub fn bin_freq(&self, bin: usize) -> f32 {
        self.fmin * 2f32.powf(bin as f32 / (12.0 * BINS_PER_SEMITONE))
    }

    pub fn bin_of(&self, f: f32) -> Option<usize> {
        if f <= 0.0 {
            return None;
        }
        let b = ((f / self.fmin).log2() * 12.0 * BINS_PER_SEMITONE).round();
        (b >= 0.0 && (b as usize) < self.nb).then_some(b as usize)
    }

    pub fn reset(&mut self) {
        let uniform = -((2 * self.nb) as f32).ln();
        self.delta.fill(uniform);
        self.t = 0;
    }

    /// Advance one frame. `energy_ok == false` forces the frame towards unvoiced
    /// (gate closed). Returns the decoded state for frame `t − lag` once enough
    /// frames have been seen.
    pub fn step(&mut self, cands: &[Candidate], energy_ok: bool) -> Option<HmmState> {
        let nb = self.nb;

        // Observation likelihoods.
        self.log_obs[..nb].fill(0.0);
        let mut sum = 0.0;
        for c in cands {
            if let Some(b) = self.bin_of(c.freq) {
                let p = c.prob * TRUST;
                self.log_obs[b] += p;
                sum += p;
            }
        }
        let gate = if energy_ok { 1.0 } else { 1e-4 };
        sum *= gate;
        let unvoiced_obs = ((1.0 - sum) / nb as f32).max(OBS_FLOOR).ln();
        for v in &mut self.log_obs[..nb] {
            *v = (*v * gate).max(OBS_FLOOR).ln();
        }
        self.log_obs[nb..].fill(unvoiced_obs);

        // Viterbi recursion with separable transitions.
        let slot = (self.t % (self.lag + 1)) * 2 * nb;
        let mut best_all = f32::NEG_INFINITY;
        for j in 0..nb {
            let lo = j.saturating_sub(self.jump);
            let hi = (j + self.jump).min(nb - 1);
            let (mut bv, mut iv) = (f32::NEG_INFINITY, j);
            let (mut bu, mut iu) = (f32::NEG_INFINITY, j);
            for i in lo..=hi {
                let lt = self.log_trans[i.abs_diff(j)];
                let v = self.delta[i] + lt;
                if v > bv {
                    bv = v;
                    iv = i;
                }
                let u = self.delta[nb + i] + lt;
                if u > bu {
                    bu = u;
                    iu = nb + i;
                }
            }
            // To voiced j.
            let (sv, su) = (bv + self.log_stay, bu + self.log_switch);
            let (val, src) = if sv >= su { (sv, iv) } else { (su, iu) };
            self.next[j] = val + self.log_obs[j];
            self.back[slot + j] = src as u16;
            // To unvoiced j.
            let (sv, su) = (bv + self.log_switch, bu + self.log_stay);
            let (val, src) = if su >= sv { (su, iu) } else { (sv, iv) };
            self.next[nb + j] = val + self.log_obs[nb + j];
            self.back[slot + nb + j] = src as u16;
            best_all = best_all.max(self.next[j]).max(self.next[nb + j]);
        }
        // Renormalise to keep numbers bounded.
        for (d, n) in self.delta.iter_mut().zip(&self.next) {
            *d = n - best_all;
        }
        self.t += 1;
        if self.t <= self.lag {
            return None;
        }

        // Backtrack `lag` frames from the current best state.
        let mut s = (0..2 * nb).max_by(|&a, &b| self.delta[a].total_cmp(&self.delta[b])).unwrap();
        for k in 0..self.lag {
            let frame = self.t - 1 - k;
            let slot = (frame % (self.lag + 1)) * 2 * nb;
            s = self.back[slot + s] as usize;
        }
        Some(HmmState { voiced: s < nb, bin: s % nb })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_single_frame_octave_glitch() {
        let mut hmm = PitchHmm::new(60.0, 1200.0, 4, 6.0);
        let mut decoded = Vec::new();
        for t in 0..40 {
            let f = if t == 20 { 440.0 } else { 220.0 }; // one-frame octave error
            let c = [Candidate { freq: f, prob: 0.9, dip: 0.05 }];
            if let Some(s) = hmm.step(&c, true) {
                decoded.push(hmm.bin_freq(s.bin));
            }
        }
        assert!(decoded.iter().skip(5).all(|f| (f / 220.0 - 1.0).abs() < 0.02), "{decoded:?}");
    }
}
