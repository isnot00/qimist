//! Chords built on the sung note. With a key, chords are diatonic (sing A in
//! C major → A minor); without one, triads are major. Seven-note scales stack
//! thirds from the scale; pentatonic/blues/chromatic pick each chord tone from
//! what the scale contains.

use crate::scale::Scale;
use crate::NOTE_NAMES;

pub const MAX_CHORD: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChordType {
    Triad,
    Seventh,
    Ninth,
    Sus2,
    Sus4,
    Power,
    Octaves,
}

impl ChordType {
    pub const ALL: [ChordType; 7] =
        [ChordType::Triad, ChordType::Seventh, ChordType::Ninth, ChordType::Sus2, ChordType::Sus4, ChordType::Power, ChordType::Octaves];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub notes: [u8; MAX_CHORD],
    pub len: usize,
    /// The sung (root) note.
    pub root: u8,
    /// Intervals of the chord tones above the root, before voicing.
    pub intervals: [u8; 5],
    pub n_intervals: usize,
}

impl Chord {
    pub fn notes(&self) -> &[u8] {
        &self.notes[..self.len]
    }

    /// "Am7", "Csus4", "G5", …
    pub fn name(&self) -> String {
        let iv = &self.intervals[..self.n_intervals];
        let has = |x: u8| iv.contains(&x);
        let root = NOTE_NAMES[(self.root % 12) as usize];
        let suffix = if iv == [12] {
            "8va".to_string()
        } else if iv == [7] {
            "5".to_string()
        } else if has(2) && !has(3) && !has(4) {
            "sus2".to_string()
        } else if has(5) && !has(3) && !has(4) {
            "sus4".to_string()
        } else {
            let minor = has(3);
            let dim = minor && has(6);
            let aug = has(4) && has(8);
            let mut s = match (minor, dim, aug, has(10), has(11), has(9) && dim) {
                (_, true, _, _, _, true) => "dim7".to_string(),
                (_, true, _, true, _, _) => "m7♭5".to_string(),
                (_, true, _, _, _, _) => "dim".to_string(),
                (_, _, true, _, _, _) => "aug".to_string(),
                (true, _, _, true, _, _) => "m7".to_string(),
                (true, _, _, _, true, _) => "m(maj7)".to_string(),
                (true, _, _, _, _, _) => "m".to_string(),
                (false, _, _, true, _, _) => "7".to_string(),
                (false, _, _, _, true, _) => "maj7".to_string(),
                _ => String::new(),
            };
            if has(14) {
                s = if s.contains('7') { s.replace('7', "9") } else { format!("{s}add9") };
            } else if has(13) {
                s.push_str("(♭9)");
            }
            s
        };
        format!("{root}{suffix}")
    }
}

/// Interval above `root` for chord tone `degree` (0 = root, 1 = third, 2 = fifth, 3 = seventh, 4 = ninth).
fn tone(root: i32, degree: i32, scale: Option<Scale>) -> i32 {
    let fallback = [0, 4, 7, 10, 14][degree as usize]; // major triad, dominant 7, 9
    let Some(sc) = scale else { return fallback };
    if sc.kind.intervals().len() == 7 && sc.contains(root) {
        return sc.step_from(root, degree * 2) - root;
    }
    // Pick what the scale offers, preferring the usual chord tones.
    let options: &[i32] = match degree {
        0 => &[0],
        1 => &[4, 3],
        2 => &[7, 6, 8],
        3 => &[10, 11, 9],
        _ => &[14, 13],
    };
    options.iter().copied().find(|&i| sc.contains(root + i)).unwrap_or(fallback)
}

/// Build a chord on `root`. `inversion` moves the lowest tones up an octave;
/// `spread` opens the voicing (second tone up an octave); `bass` adds the root an
/// octave below.
pub fn build(root: u8, kind: ChordType, scale: Option<Scale>, inversion: u8, spread: bool, bass: bool) -> Chord {
    let r = root as i32;
    let degrees: &[i32] = match kind {
        ChordType::Triad => &[0, 1, 2],
        ChordType::Seventh => &[0, 1, 2, 3],
        ChordType::Ninth => &[0, 1, 2, 3, 4],
        ChordType::Sus2 | ChordType::Sus4 | ChordType::Power | ChordType::Octaves => &[],
    };
    let mut iv: Vec<i32> = match kind {
        ChordType::Sus2 => vec![0, 2, tone(r, 2, scale)],
        ChordType::Sus4 => vec![0, 5, tone(r, 2, scale)],
        ChordType::Power => vec![0, 7],
        ChordType::Octaves => vec![0, 12],
        _ => degrees.iter().map(|&d| tone(r, d, scale)).collect(),
    };
    let mut intervals = [0u8; 5];
    let n_intervals = (iv.len() - 1).min(5);
    for (k, i) in iv.iter().skip(1).take(5).enumerate() {
        intervals[k] = *i as u8;
    }

    // Voicing.
    for _ in 0..inversion.min(iv.len() as u8 - 1) {
        let low = iv.remove(0);
        iv.push(low + 12);
    }
    if spread && iv.len() >= 3 {
        iv[1] += 12;
        iv.sort_unstable();
    }
    if bass {
        iv.insert(0, -12);
    }
    let mut notes = [0u8; MAX_CHORD];
    let mut len = 0;
    for i in iv {
        let n = r + i;
        if (0..=127).contains(&n) && len < MAX_CHORD && !notes[..len].contains(&(n as u8)) {
            notes[len] = n as u8;
            len += 1;
        }
    }
    Chord { notes, len, root, intervals, n_intervals }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scale::ScaleKind;

    #[test]
    fn diatonic_chords_in_c_major() {
        let c = Some(Scale::new(0, ScaleKind::Major));
        assert_eq!(build(57, ChordType::Triad, c, 0, false, false).notes(), &[57, 60, 64]); // Am
        assert_eq!(build(57, ChordType::Triad, c, 0, false, false).name(), "Am");
        assert_eq!(build(67, ChordType::Seventh, c, 0, false, false).name(), "G7");
        assert_eq!(build(60, ChordType::Seventh, c, 0, false, false).name(), "Cmaj7");
        assert_eq!(build(71, ChordType::Seventh, c, 0, false, false).name(), "Bm7♭5");
        assert_eq!(build(62, ChordType::Ninth, c, 0, false, false).name(), "Dm9");
    }

    #[test]
    fn voicings() {
        let c = Some(Scale::new(0, ScaleKind::Major));
        assert_eq!(build(60, ChordType::Triad, c, 1, false, false).notes(), &[64, 67, 72]);
        assert_eq!(build(60, ChordType::Triad, c, 0, true, false).notes(), &[60, 67, 76]);
        assert_eq!(build(60, ChordType::Triad, None, 0, false, true).notes(), &[48, 60, 64, 67]);
        assert_eq!(build(60, ChordType::Power, None, 0, false, false).name(), "C5");
        assert_eq!(build(60, ChordType::Sus4, None, 0, false, false).name(), "Csus4");
    }

    #[test]
    fn pentatonic_uses_scale_tones() {
        let am = Some(Scale::new(9, ScaleKind::MinorPentatonic));
        assert_eq!(build(57, ChordType::Triad, am, 0, false, false).name(), "Am");
    }
}
