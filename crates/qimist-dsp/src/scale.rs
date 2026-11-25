//! Scales and scale snapping.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScaleKind {
    Major,
    NaturalMinor,
    HarmonicMinor,
    MelodicMinor,
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    Locrian,
    MajorPentatonic,
    MinorPentatonic,
    Blues,
    Chromatic,
}

impl ScaleKind {
    pub const ALL: [ScaleKind; 13] = [
        ScaleKind::Major,
        ScaleKind::NaturalMinor,
        ScaleKind::HarmonicMinor,
        ScaleKind::MelodicMinor,
        ScaleKind::Dorian,
        ScaleKind::Phrygian,
        ScaleKind::Lydian,
        ScaleKind::Mixolydian,
        ScaleKind::Locrian,
        ScaleKind::MajorPentatonic,
        ScaleKind::MinorPentatonic,
        ScaleKind::Blues,
        ScaleKind::Chromatic,
    ];

    pub fn intervals(self) -> &'static [u8] {
        match self {
            ScaleKind::Major => &[0, 2, 4, 5, 7, 9, 11],
            ScaleKind::NaturalMinor => &[0, 2, 3, 5, 7, 8, 10],
            ScaleKind::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            ScaleKind::MelodicMinor => &[0, 2, 3, 5, 7, 9, 11],
            ScaleKind::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            ScaleKind::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
            ScaleKind::Lydian => &[0, 2, 4, 6, 7, 9, 11],
            ScaleKind::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            ScaleKind::Locrian => &[0, 1, 3, 5, 6, 8, 10],
            ScaleKind::MajorPentatonic => &[0, 2, 4, 7, 9],
            ScaleKind::MinorPentatonic => &[0, 3, 5, 7, 10],
            ScaleKind::Blues => &[0, 3, 5, 6, 7, 10],
            ScaleKind::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ScaleKind::Major => "Major",
            ScaleKind::NaturalMinor => "Minor",
            ScaleKind::HarmonicMinor => "Harmonic Minor",
            ScaleKind::MelodicMinor => "Melodic Minor",
            ScaleKind::Dorian => "Dorian",
            ScaleKind::Phrygian => "Phrygian",
            ScaleKind::Lydian => "Lydian",
            ScaleKind::Mixolydian => "Mixolydian",
            ScaleKind::Locrian => "Locrian",
            ScaleKind::MajorPentatonic => "Major Pent.",
            ScaleKind::MinorPentatonic => "Minor Pent.",
            ScaleKind::Blues => "Blues",
            ScaleKind::Chromatic => "Chromatic",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Scale {
    /// Pitch class of the tonic, 0 = C.
    pub root: u8,
    pub kind: ScaleKind,
}

impl Scale {
    pub fn new(root: u8, kind: ScaleKind) -> Self {
        Scale { root: root % 12, kind }
    }

    /// Bit `pc` set when pitch class `pc` is in the scale.
    pub fn mask(&self) -> u16 {
        self.kind.intervals().iter().fold(0u16, |m, &i| m | 1 << ((self.root + i) % 12))
    }

    pub fn contains(&self, note: i32) -> bool {
        self.mask() & (1 << note.rem_euclid(12)) != 0
    }

    /// Nearest in-scale MIDI note to a float pitch. Ties go to the lower note.
    pub fn snap(&self, midi: f32) -> u8 {
        let base = midi.round() as i32;
        let mut best = base;
        let mut best_d = f32::MAX;
        for n in base - 2..=base + 2 {
            if self.contains(n) {
                let d = (midi - n as f32).abs();
                if d < best_d - 1e-4 {
                    best_d = d;
                    best = n;
                }
            }
        }
        best.clamp(0, 127) as u8
    }

    /// Scale degree (0-based) of `note`, or None if not in the scale.
    pub fn degree(&self, note: i32) -> Option<usize> {
        let pc = (note - self.root as i32).rem_euclid(12) as u8;
        self.kind.intervals().iter().position(|&i| i == pc)
    }

    /// The in-scale note `steps` scale degrees above `note` (which must be in scale).
    pub fn step_from(&self, note: i32, steps: i32) -> i32 {
        let iv = self.kind.intervals();
        let len = iv.len() as i32;
        let Some(d) = self.degree(note) else { return note + steps };
        let octave_base = note - iv[d] as i32;
        let idx = d as i32 + steps;
        octave_base + iv[idx.rem_euclid(len) as usize] as i32 + 12 * idx.div_euclid(len)
    }

    pub fn name(&self) -> String {
        format!("{} {}", crate::NOTE_NAMES[self.root as usize], self.kind.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_to_a_minor() {
        let s = Scale::new(9, ScaleKind::NaturalMinor);
        assert_eq!(s.snap(68.9), 69); // A4
        assert_eq!(s.snap(67.8), 67); // near G#4 (not in scale) → G4 is nearer than A4
        assert_eq!(s.snap(68.2), 69);
        assert_eq!(s.snap(70.0), 69); // A#4 → tie, lower wins
    }

    #[test]
    fn steps_build_triads() {
        let s = Scale::new(0, ScaleKind::Major);
        assert_eq!(s.step_from(60, 2), 64);
        assert_eq!(s.step_from(60, 4), 67);
        assert_eq!(s.step_from(71, 2), 74); // B → D (next octave)
    }
}
