//! Key / scale detection with Krumhansl-Schmuckler profiles.
//!
//! We accumulate a pitch-class histogram weighted by how long (and how
//! confidently) each note was sung, with exponential forgetting so the key can
//! follow a song that modulates. Every candidate key's profile is correlated
//! with the histogram; the best correlation wins.

use crate::scale::{Scale, ScaleKind};

const MAJOR: [f32; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const MINOR: [f32; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub root: u8,
    pub minor: bool,
}

impl Key {
    pub fn scale(&self) -> Scale {
        Scale::new(self.root, if self.minor { ScaleKind::NaturalMinor } else { ScaleKind::Major })
    }

    pub fn name(&self) -> String {
        format!("{} {}", crate::NOTE_NAMES[self.root as usize], if self.minor { "minor" } else { "major" })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyEstimate {
    pub key: Key,
    /// 0..1, combining correlation strength and amount of material heard.
    pub confidence: f32,
    pub alternatives: [(Key, f32); 2],
    /// Normalised pitch-class histogram (max = 1).
    pub histogram: [f32; 12],
}

pub struct KeyDetector {
    hist: [f32; 12],
    half_life_s: f32,
    /// Seconds of sung material in the (decayed) histogram.
    amount: f32,
}

impl Default for KeyDetector {
    fn default() -> Self {
        Self::new(30.0)
    }
}

fn pearson(a: &[f32; 12], b: &[f32; 12], rot: usize) -> f32 {
    let ma = a.iter().sum::<f32>() / 12.0;
    let mb = b.iter().sum::<f32>() / 12.0;
    let (mut num, mut da, mut db) = (0.0, 0.0, 0.0);
    for i in 0..12 {
        let x = a[(i + rot) % 12] - ma;
        let y = b[i] - mb;
        num += x * y;
        da += x * x;
        db += y * y;
    }
    if da <= 0.0 || db <= 0.0 {
        0.0
    } else {
        num / (da * db).sqrt()
    }
}

impl KeyDetector {
    pub fn new(half_life_s: f32) -> Self {
        KeyDetector { hist: [0.0; 12], half_life_s, amount: 0.0 }
    }

    pub fn clear(&mut self) {
        self.hist = [0.0; 12];
        self.amount = 0.0;
    }

    /// Advance time by `dt` seconds; if a note is sounding, add its pitch class.
    pub fn update(&mut self, dt: f32, note: Option<u8>, weight: f32) {
        let decay = 0.5f32.powf(dt / self.half_life_s);
        self.hist.iter_mut().for_each(|h| *h *= decay);
        self.amount *= decay;
        if let Some(n) = note {
            self.hist[(n % 12) as usize] += dt * weight;
            self.amount += dt;
        }
    }

    pub fn estimate(&self) -> Option<KeyEstimate> {
        if self.amount < 0.5 {
            return None;
        }
        let mut scores = [(Key { root: 0, minor: false }, f32::MIN); 24];
        for root in 0..12u8 {
            scores[root as usize] = (Key { root, minor: false }, pearson(&self.hist, &MAJOR, root as usize));
            scores[12 + root as usize] = (Key { root, minor: true }, pearson(&self.hist, &MINOR, root as usize));
        }
        scores.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
        let max = self.hist.iter().cloned().fold(0.0, f32::max).max(1e-9);
        let material = (self.amount / 6.0).min(1.0);
        let strength = ((scores[0].1 - 0.3) / 0.6).clamp(0.0, 1.0);
        let to_conf = |r: f32| ((r - 0.3) / 0.6).clamp(0.0, 1.0) * material;
        Some(KeyEstimate {
            key: scores[0].0,
            confidence: strength * material,
            alternatives: [(scores[1].0, to_conf(scores[1].1)), (scores[2].0, to_conf(scores[2].1))],
            histogram: self.hist.map(|h| h / max),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_a_minor_melody() {
        let mut k = KeyDetector::default();
        // A minor-ish melody, tonic and fifth emphasised.
        let melody = [69, 72, 71, 69, 67, 64, 65, 64, 62, 60, 69, 64, 69, 72, 76, 69];
        for _ in 0..4 {
            for &n in &melody {
                for _ in 0..50 {
                    k.update(0.004 * 2.0, Some(n), 1.0);
                }
            }
        }
        let e = k.estimate().unwrap();
        assert_eq!(e.key, Key { root: 9, minor: true }, "{:?}", e);
    }
}
