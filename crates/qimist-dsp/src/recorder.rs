//! Note recorder and Standard MIDI File export.

use midly::num::{u15, u24, u28, u4, u7};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};

pub const PPQ: u16 = 480;
pub const DRUM_CHANNEL: u8 = 9;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecordedNote {
    /// Seconds from the start of the take.
    pub start: f64,
    pub end: f64,
    pub note: u8,
    pub velocity: u8,
    pub channel: u8,
}

/// Collects notes. Pre-allocates so recording never allocates on the audio
/// thread until `capacity` notes have been captured.
pub struct Recorder {
    notes: Vec<RecordedNote>,
    open: [Option<(f64, u8, u8)>; 128],
}

impl Default for Recorder {
    fn default() -> Self {
        Self::with_capacity(20_000)
    }
}

impl Recorder {
    pub fn with_capacity(cap: usize) -> Self {
        Recorder { notes: Vec::with_capacity(cap), open: [None; 128] }
    }

    pub fn clear(&mut self) {
        self.notes.clear();
        self.open = [None; 128];
    }

    pub fn notes(&self) -> &[RecordedNote] {
        &self.notes
    }

    /// Replace the finished notes (hand edits from the GUI; not while recording).
    pub fn replace_notes(&mut self, notes: &[RecordedNote]) {
        self.notes.clear();
        let room = self.notes.capacity().max(notes.len());
        self.notes.extend(notes.iter().take(room).copied());
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty() && self.open.iter().all(Option::is_none)
    }

    pub fn note_on(&mut self, t: f64, note: u8, velocity: u8, channel: u8) {
        if self.open[note as usize].is_some() {
            self.note_off(t, note);
        }
        self.open[note as usize] = Some((t.max(0.0), velocity, channel));
    }

    pub fn note_off(&mut self, t: f64, note: u8) {
        if let Some((start, velocity, channel)) = self.open[note as usize].take() {
            if self.notes.len() < self.notes.capacity() {
                self.notes.push(RecordedNote { start, end: t.max(start + 0.01), note, velocity, channel });
            }
        }
    }

    /// Close all hanging notes at time `t`.
    pub fn close_all(&mut self, t: f64) {
        for n in 0..128u8 {
            self.note_off(t, n);
        }
    }

    /// Notes still sounding (start time, note), for live display.
    pub fn open_notes(&self) -> impl Iterator<Item = (f64, u8)> + '_ {
        self.open.iter().enumerate().filter_map(|(n, o)| o.map(|(t, _, _)| (t, n as u8)))
    }

    /// Encode as a type-0 Standard MIDI File. `trim` shifts the take so the
    /// first note starts on beat 1; `quantize` = grid division (4 = quarter
    /// notes, 16 = sixteenths, 0 = off).
    pub fn to_smf(&self, bpm: f64, trim: bool, quantize: u32) -> Vec<u8> {
        encode_smf(&self.notes, bpm, trim, quantize)
    }
}

pub fn encode_smf(notes: &[RecordedNote], bpm: f64, trim: bool, quantize: u32) -> Vec<u8> {
    let bpm = bpm.clamp(20.0, 400.0);
    let ticks_per_sec = bpm / 60.0 * PPQ as f64;
    let offset = if trim { notes.iter().map(|n| n.start).fold(f64::MAX, f64::min) } else { 0.0 };
    let offset = if offset.is_finite() { offset } else { 0.0 };
    let grid = if quantize > 0 { (PPQ as u64 * 4 / quantize as u64).max(1) } else { 1 };
    let tick = |t: f64| {
        let raw = ((t - offset).max(0.0) * ticks_per_sec).round() as u64;
        (raw + grid / 2) / grid * grid
    };

    // (tick, order, kind): note-offs sort before note-ons on the same tick.
    let mut evs: Vec<(u64, u8, TrackEventKind<'static>)> = Vec::with_capacity(notes.len() * 2);
    for n in notes {
        let ch = u4::new(n.channel.min(15));
        let key = u7::new(n.note.min(127));
        evs.push((tick(n.start), 1, TrackEventKind::Midi { channel: ch, message: MidiMessage::NoteOn { key, vel: u7::new(n.velocity.clamp(1, 127)) } }));
        evs.push((tick(n.end).max(tick(n.start) + grid.max(2) / 2), 0, TrackEventKind::Midi { channel: ch, message: MidiMessage::NoteOff { key, vel: u7::new(64) } }));
    }
    evs.sort_by_key(|e| (e.0, e.1));

    let mut track: Vec<TrackEvent<'static>> = Vec::with_capacity(evs.len() + 3);
    track.push(TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::TrackName(b"Qimist")) });
    track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::Tempo(u24::new((60_000_000.0 / bpm).round() as u32))),
    });
    let mut last = 0u64;
    for (t, _, kind) in evs {
        track.push(TrackEvent { delta: u28::new((t - last).min(0x0FFF_FFFF) as u32), kind });
        last = t;
    }
    track.push(TrackEvent { delta: u28::new(0), kind: TrackEventKind::Meta(MetaMessage::EndOfTrack) });

    let smf = Smf { header: Header::new(Format::SingleTrack, Timing::Metrical(u15::new(PPQ))), tracks: vec![track] };
    let mut out = Vec::new();
    smf.write_std(&mut out).expect("writing to a Vec cannot fail");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smf_roundtrip() {
        let mut r = Recorder::default();
        r.note_on(1.0, 60, 100, 0);
        r.note_off(1.5, 60);
        r.note_on(1.5, 64, 90, 0);
        r.close_all(2.0);
        let bytes = r.to_smf(120.0, true, 16);
        let smf = Smf::parse(&bytes).unwrap();
        let ons = smf.tracks[0]
            .iter()
            .filter(|e| matches!(e.kind, TrackEventKind::Midi { message: MidiMessage::NoteOn { .. }, .. }))
            .count();
        assert_eq!(ons, 2);
    }
}
