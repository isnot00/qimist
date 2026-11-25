//! Loop station: four layers of notes that loop over 1–8 bars.
//!
//! Recording captures whatever Qimist sends out (melody notes, chords, arp,
//! beatbox hits) at its position in the loop. A take always lasts exactly one
//! loop: press REC while playing and it records from here round to here;
//! press it while stopped and it counts in one bar first. Recording into a
//! layer that already has notes overdubs. Everything runs on a beat clock:
//! the host's when its transport plays, otherwise an internal one.
//!
//! Song mode: there are four scenes (A–D), each its own set of layers and
//! length. Picking a scene while the loop plays switches at the end of the
//! current pass; PLAY SONG runs through an arrangement (e.g. A×2 B×2 A×2 C)
//! by itself.
//!
//! Real-time safety: layers reserve room for `MAX_NOTES` up front and refuse
//! notes beyond it, so recording never allocates on the audio thread.

use serde::{Deserialize, Serialize};

pub const LAYERS: usize = 4;
pub const SCENES: usize = 4;
/// Longest arrangement.
pub const SONG_MAX: usize = 16;
pub const MAX_NOTES: usize = 1024;
const MAX_SOUNDING: usize = 64;
/// Beatbox hits are stored as short notes.
pub const HIT_BEATS: f64 = 0.125;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoopNote {
    /// Position in the loop, in beats.
    pub start: f64,
    /// Length in beats.
    pub len: f64,
    pub note: u8,
    pub vel: u8,
    pub ch: u8,
    /// Drum class index for beatbox hits, −1 for synth notes.
    pub drum: i8,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LoopLayer {
    /// Sorted by `start`.
    pub notes: Vec<LoopNote>,
    pub muted: bool,
}

/// The loop contents (saved with the project).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoopData {
    pub beats: f64,
    pub layers: [LoopLayer; LAYERS],
}

impl Default for LoopData {
    fn default() -> Self {
        LoopData { beats: 8.0, layers: Default::default() }
    }
}

impl LoopData {
    /// Make room so the audio thread can record without allocating.
    pub fn reserve(&mut self) {
        for l in self.layers.iter_mut() {
            let n = l.notes.len();
            l.notes.reserve(MAX_NOTES.saturating_sub(n));
        }
    }

    pub fn is_empty(&self) -> bool {
        self.layers.iter().all(|l| l.notes.is_empty())
    }

    /// Copy `other` in, reusing this copy's reserved room (no allocation
    /// once both have reserved `MAX_NOTES`).
    pub fn copy_from(&mut self, other: &LoopData) {
        self.beats = other.beats;
        for (dst, src) in self.layers.iter_mut().zip(&other.layers) {
            dst.notes.clear();
            dst.notes.extend_from_slice(&src.notes);
            dst.muted = src.muted;
        }
    }
}

/// One step of the song: play `scene` this many times.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SongStep {
    pub scene: u8,
    pub repeats: u8,
}

impl Default for SongStep {
    fn default() -> Self {
        SongStep { scene: 0, repeats: 1 }
    }
}

/// Everything the loop station saves with the project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SongData {
    pub scenes: [LoopData; SCENES],
    /// The scene that's playing / shown.
    pub scene: usize,
    pub steps: Vec<SongStep>,
    /// Start the song over at the end instead of stopping.
    pub repeat: bool,
}

impl Default for SongData {
    fn default() -> Self {
        SongData { scenes: Default::default(), scene: 0, steps: vec![SongStep { scene: 0, repeats: 2 }], repeat: false }
    }
}

impl SongData {
    pub fn reserve(&mut self) {
        self.scenes.iter_mut().for_each(LoopData::reserve);
    }

    /// Total length of the arrangement in beats.
    pub fn song_beats(&self) -> f64 {
        self.steps.iter().map(|s| self.scenes[s.scene as usize % SCENES].beats * s.repeats as f64).sum()
    }
}

/// The arrangement as a fixed array, so it can travel in a (Copy) command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Song {
    pub steps: [SongStep; SONG_MAX],
    pub len: u8,
    pub repeat: bool,
}

impl Song {
    pub fn from_steps(steps: &[SongStep], repeat: bool) -> Self {
        let mut s = Song { steps: [SongStep::default(); SONG_MAX], len: 0, repeat };
        for (d, st) in s.steps.iter_mut().zip(steps.iter().filter(|st| st.repeats > 0)) {
            *d = SongStep { scene: st.scene % SCENES as u8, repeats: st.repeats };
            s.len += 1;
        }
        s
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopState {
    Stopped,
    /// Waiting for the loop to start (count-in); then play, or record `Some(layer)`.
    Waiting { record: Option<usize> },
    Playing,
    Recording { layer: usize },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoopOut {
    On { note: u8, vel: u8, ch: u8, drum: i8 },
    Off { note: u8, ch: u8 },
    /// Metronome tick (accent on the first beat of a bar).
    Click { accent: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopCmd {
    Play,
    Stop,
    Record(usize),
    ToggleMute(usize),
    Clear(usize),
    Undo,
    ClearAll,
    /// Go to a scene: now when stopped, at the end of the pass when playing.
    Scene(usize),
    /// Copy one scene's layers into another.
    CopyScene { from: usize, to: usize },
    /// Play the arrangement from its first step.
    PlaySong(Song),
}

#[derive(Clone, Copy, Default)]
struct Sounding {
    note: u8,
    ch: u8,
    layer: usize,
    /// Absolute beat at which it ends.
    end: f64,
}

pub struct Looper {
    /// The scene that's playing (the others wait in `scenes`).
    pub data: LoopData,
    /// Stored scenes; the active one's slot holds a spare (swapped out).
    pub scenes: [LoopData; SCENES],
    scene: usize,
    next_scene: Option<usize>,
    song: Song,
    /// Song position: step index and passes done in it.
    song_at: Option<(usize, u8)>,
    pub metronome: bool,
    pub count_in: bool,
    /// Record quantize grid in beats (0 = off).
    pub quantize: f64,
    state: LoopState,
    /// Beat clock (absolute) and where the loop's beat 0 is.
    beat: f64,
    origin: f64,
    start_at: f64,
    rec_from: f64,
    cursor: [usize; LAYERS],
    /// Notes being recorded: (note, ch, absolute start, vel).
    open: [Option<(f64, u8)>; 128],
    open_ch: [u8; 128],
    sounding: [Sounding; MAX_SOUNDING],
    n_sounding: usize,
    /// UNDO keeps the layer as it was before the last take/clear (swap to redo).
    undo: Option<usize>,
    undo_buf: Vec<LoopNote>,
    /// Contents changed (so the copy shown/saved can be refreshed).
    pub dirty: bool,
    started: bool,
    host_clock: bool,
}

impl Default for Looper {
    fn default() -> Self {
        Self::new()
    }
}

impl Looper {
    pub fn new() -> Self {
        let mut data = LoopData::default();
        data.reserve();
        let mut scenes: [LoopData; SCENES] = Default::default();
        scenes.iter_mut().for_each(LoopData::reserve);
        Looper {
            data,
            scenes,
            scene: 0,
            next_scene: None,
            song: Song::from_steps(&[], false),
            song_at: None,
            metronome: true,
            count_in: true,
            quantize: 0.25,
            state: LoopState::Stopped,
            beat: 0.0,
            origin: 0.0,
            start_at: 0.0,
            rec_from: 0.0,
            cursor: [0; LAYERS],
            open: [None; 128],
            open_ch: [0; 128],
            sounding: [Sounding::default(); MAX_SOUNDING],
            n_sounding: 0,
            undo: None,
            undo_buf: Vec::with_capacity(MAX_NOTES),
            dirty: false,
            started: false,
            host_clock: false,
        }
    }

    pub fn state(&self) -> LoopState {
        self.state
    }

    /// Position in the loop (beats), or the negative count-in while waiting.
    pub fn position(&self) -> f64 {
        match self.state {
            LoopState::Stopped => 0.0,
            LoopState::Waiting { .. } => self.beat - self.start_at,
            _ => (self.beat - self.origin).rem_euclid(self.data.beats),
        }
    }

    pub fn is_recording(&self) -> bool {
        matches!(self.state, LoopState::Recording { .. })
    }

    pub fn scene(&self) -> usize {
        self.scene
    }

    /// Scene waiting to start at the end of this pass.
    pub fn next_scene(&self) -> Option<usize> {
        self.next_scene
    }

    /// Song step playing (index into the arrangement).
    pub fn song_step(&self) -> Option<usize> {
        self.song_at.map(|(i, _)| i)
    }

    pub fn scene_data(&self, i: usize) -> &LoopData {
        if i == self.scene {
            &self.data
        } else {
            &self.scenes[i]
        }
    }

    fn scene_mut(&mut self, i: usize) -> &mut LoopData {
        if i == self.scene {
            &mut self.data
        } else {
            &mut self.scenes[i]
        }
    }

    /// Load saved scenes (call off the audio thread: it may allocate).
    pub fn load(&mut self, song: &SongData) {
        self.scene = song.scene.min(SCENES - 1);
        for i in 0..SCENES {
            self.scenes[i] = song.scenes[i].clone();
            self.scenes[i].reserve();
        }
        std::mem::swap(&mut self.data, &mut self.scenes[self.scene]);
        self.data.reserve();
        self.song = Song::from_steps(&song.steps, song.repeat);
    }

    /// Write every scene into `song` (no allocation once it has reserved room).
    pub fn save(&self, song: &mut SongData) {
        for i in 0..SCENES {
            song.scenes[i].copy_from(self.scene_data(i));
        }
        song.scene = self.scene;
    }

    /// Make `to` the playing scene, its loop starting at absolute beat `origin`.
    fn switch_scene(&mut self, to: usize, origin: f64, out: &mut impl FnMut(LoopOut)) {
        if let LoopState::Recording { layer } = self.state {
            self.finish_recording(layer);
        }
        self.all_off(None, out);
        if to != self.scene {
            std::mem::swap(&mut self.data, &mut self.scenes[self.scene]);
            std::mem::swap(&mut self.data, &mut self.scenes[to]);
            self.scene = to;
            // Undo belongs to the scene it was made in.
            self.undo = None;
        }
        self.origin = origin;
        self.cursor = [0; LAYERS];
        self.dirty = true;
    }

    /// End of a pass: move the song on, or go to a queued scene. Returns
    /// false when the song is over and playback stopped.
    fn at_loop_end(&mut self, origin: f64, out: &mut impl FnMut(LoopOut)) -> bool {
        if let Some((mut i, mut done)) = self.song_at {
            done += 1;
            if done >= self.song.steps[i].repeats {
                i += 1;
                done = 0;
                if i >= self.song.len as usize {
                    if !self.song.repeat {
                        if let LoopState::Recording { layer } = self.state {
                            self.finish_recording(layer);
                        }
                        self.all_off(None, out);
                        self.state = LoopState::Stopped;
                        self.song_at = None;
                        return false;
                    }
                    i = 0;
                }
            }
            self.song_at = Some((i, done));
            let to = self.song.steps[i].scene as usize;
            self.switch_scene(to, origin, out);
        } else if let Some(to) = self.next_scene.take() {
            self.switch_scene(to, origin, out);
        }
        true
    }

    /// Change the loop length (only while it's empty, so nothing gets cut).
    pub fn set_beats(&mut self, beats: f64) -> bool {
        if self.data.is_empty() && (beats - self.data.beats).abs() > 1e-9 {
            self.data.beats = beats.max(1.0);
            return true;
        }
        false
    }

    fn all_off(&mut self, layer: Option<usize>, out: &mut impl FnMut(LoopOut)) {
        let mut k = 0;
        while k < self.n_sounding {
            let s = self.sounding[k];
            if layer.is_none_or(|l| l == s.layer) {
                out(LoopOut::Off { note: s.note, ch: s.ch });
                self.n_sounding -= 1;
                self.sounding[k] = self.sounding[self.n_sounding];
            } else {
                k += 1;
            }
        }
    }

    fn reset_cursors(&mut self, pos: f64) {
        for (l, layer) in self.data.layers.iter().enumerate() {
            self.cursor[l] = layer.notes.partition_point(|n| n.start < pos);
        }
    }

    /// Where the loop starts. Internal clock: now, or after one bar of
    /// count-in. Host clock: the next bar line (a bar later if that's too
    /// close to count in).
    fn start_point(&self, beat: f64, count: bool) -> f64 {
        if !self.host_clock {
            return if count { beat + 4.0 } else { beat };
        }
        let next_bar = (beat / 4.0 - 1e-9).ceil() * 4.0;
        if count {
            if next_bar - beat < 2.0 {
                next_bar + 4.0
            } else {
                next_bar
            }
        } else {
            next_bar
        }
    }

    pub fn command(&mut self, cmd: LoopCmd, out: &mut impl FnMut(LoopOut)) {
        match cmd {
            LoopCmd::Play => {
                if self.state == LoopState::Stopped {
                    self.start_at = self.start_point(self.beat, false);
                    self.state = LoopState::Waiting { record: None };
                }
            }
            LoopCmd::Stop => {
                if let LoopState::Recording { layer } = self.state {
                    self.finish_recording(layer);
                }
                self.all_off(None, out);
                self.state = LoopState::Stopped;
                self.song_at = None;
                self.next_scene = None;
            }
            LoopCmd::Scene(to) if to < SCENES => {
                // Picking a scene by hand leaves the song.
                self.song_at = None;
                match self.state {
                    LoopState::Stopped | LoopState::Waiting { .. } => {
                        let origin = self.origin;
                        self.switch_scene(to, origin, out);
                        self.next_scene = None;
                    }
                    _ => self.next_scene = (to != self.scene).then_some(to),
                }
            }
            LoopCmd::Scene(_) => {}
            LoopCmd::CopyScene { from, to } if from < SCENES && to < SCENES && from != to => {
                if matches!(self.state, LoopState::Recording { .. }) && to == self.scene {
                    return;
                }
                if to == self.scene {
                    self.all_off(None, out);
                    self.data.copy_from(&self.scenes[from]);
                    let pos = self.position();
                    self.reset_cursors(pos);
                    self.undo = None;
                } else if from == self.scene {
                    self.scenes[to].copy_from(&self.data);
                } else {
                    let (a, b) = if from < to {
                        let (l, r) = self.scenes.split_at_mut(to);
                        (&l[from], &mut r[0])
                    } else {
                        let (l, r) = self.scenes.split_at_mut(from);
                        (&r[0], &mut l[to])
                    };
                    b.copy_from(a);
                }
                self.dirty = true;
            }
            LoopCmd::CopyScene { .. } => {}
            LoopCmd::PlaySong(song) => {
                if song.len == 0 {
                    return;
                }
                if let LoopState::Recording { layer } = self.state {
                    self.finish_recording(layer);
                }
                self.all_off(None, out);
                self.song = song;
                self.song_at = Some((0, 0));
                self.next_scene = None;
                self.state = LoopState::Stopped;
                let origin = self.origin;
                self.switch_scene(song.steps[0].scene as usize, origin, out);
                self.start_at = self.start_point(self.beat, self.count_in);
                self.state = LoopState::Waiting { record: None };
            }
            LoopCmd::Record(layer) if layer < LAYERS => match self.state {
                LoopState::Stopped => {
                    self.start_at = self.start_point(self.beat, self.count_in);
                    self.state = LoopState::Waiting { record: Some(layer) };
                }
                LoopState::Waiting { .. } => self.state = LoopState::Waiting { record: Some(layer) },
                LoopState::Playing => self.begin_recording(layer),
                LoopState::Recording { layer: cur } => {
                    self.finish_recording(cur);
                    if cur != layer {
                        self.begin_recording(layer);
                    }
                }
            },
            LoopCmd::Record(_) => {}
            LoopCmd::ToggleMute(l) if l < LAYERS => {
                self.data.layers[l].muted = !self.data.layers[l].muted;
                if self.data.layers[l].muted {
                    self.all_off(Some(l), out);
                }
                self.dirty = true;
            }
            LoopCmd::Clear(l) if l < LAYERS => {
                self.all_off(Some(l), out);
                self.save_undo(l);
                self.data.layers[l].notes.clear();
                self.cursor[l] = 0;
                self.dirty = true;
            }
            LoopCmd::ToggleMute(_) | LoopCmd::Clear(_) => {}
            LoopCmd::Undo => {
                if let Some(l) = self.undo {
                    if let LoopState::Recording { layer } = self.state {
                        self.finish_recording(layer);
                    }
                    self.all_off(Some(l), out);
                    std::mem::swap(&mut self.data.layers[l].notes, &mut self.undo_buf);
                    let pos = self.position();
                    self.reset_cursors(pos);
                    self.dirty = true;
                }
            }
            LoopCmd::ClearAll => {
                self.all_off(None, out);
                self.state = LoopState::Stopped;
                for l in 0..LAYERS {
                    self.data.layers[l].notes.clear();
                    self.data.layers[l].muted = false;
                }
                self.undo = None;
                self.undo_buf.clear();
                self.dirty = true;
            }
        }
    }

    /// Copy a layer's notes into the undo buffer (no allocation: both reserve MAX_NOTES).
    fn save_undo(&mut self, layer: usize) {
        self.undo_buf.clear();
        self.undo_buf.extend_from_slice(&self.data.layers[layer].notes);
        self.undo = Some(layer);
    }

    fn begin_recording(&mut self, layer: usize) {
        self.save_undo(layer);
        self.rec_from = self.beat;
        self.open = [None; 128];
        self.state = LoopState::Recording { layer };
    }

    fn finish_recording(&mut self, layer: usize) {
        let end = self.beat.min(self.rec_from + self.data.beats);
        for n in 0..128 {
            if let Some((start, vel)) = self.open[n].take() {
                self.store(layer, start, end, n as u8, vel, self.open_ch[n], -1);
            }
        }
        self.state = LoopState::Playing;
        self.dirty = true;
    }

    fn store(&mut self, layer: usize, start: f64, end: f64, note: u8, vel: u8, ch: u8, drum: i8) {
        let beats = self.data.beats;
        let mut pos = (start - self.origin).rem_euclid(beats);
        if self.quantize > 0.0 {
            pos = ((pos / self.quantize).round() * self.quantize).rem_euclid(beats);
        }
        let len = (end - start).clamp(0.02, beats - 0.01);
        let notes = &mut self.data.layers[layer].notes;
        if notes.len() >= notes.capacity().min(MAX_NOTES) {
            return;
        }
        let i = notes.partition_point(|n| n.start <= pos);
        notes.insert(i, LoopNote { start: pos, len, note, vel, ch, drum });
        // Count it as already played this pass (it was just heard live).
        if i <= self.cursor[layer] {
            self.cursor[layer] += 1;
        }
        self.dirty = true;
    }

    /// Put hand-edited notes into a layer of a scene (from the piano-roll
    /// editor). `notes` must be sorted by start. Refused while that layer is
    /// recording. Copies into the reserved capacity, so it doesn't allocate.
    pub fn replace_layer(&mut self, scene: usize, layer: usize, notes: &[LoopNote], out: &mut impl FnMut(LoopOut)) -> bool {
        let active = scene == self.scene;
        if layer >= LAYERS || scene >= SCENES || (active && matches!(self.state, LoopState::Recording { layer: l } if l == layer)) {
            return false;
        }
        if active {
            self.all_off(Some(layer), out);
        }
        let data = self.scene_mut(scene);
        let beats = data.beats;
        let dst = &mut data.layers[layer].notes;
        dst.clear();
        let room = dst.capacity().min(MAX_NOTES);
        dst.extend(notes.iter().take(room).map(|n| LoopNote { start: n.start.rem_euclid(beats), len: n.len.clamp(0.02, beats - 0.01), ..*n }));
        if active {
            let pos = match self.state {
                LoopState::Playing | LoopState::Recording { .. } => (self.beat - self.origin).rem_euclid(beats),
                _ => 0.0,
            };
            self.cursor[layer] = self.data.layers[layer].notes.partition_point(|n| n.start < pos);
        }
        self.dirty = true;
        true
    }

    /// A note Qimist sent out (recorded when a take is running).
    pub fn note_on(&mut self, note: u8, vel: u8, ch: u8) {
        if self.is_recording() {
            self.open[note as usize & 127] = Some((self.beat, vel));
            self.open_ch[note as usize & 127] = ch;
        }
    }

    pub fn note_off(&mut self, note: u8) {
        if let LoopState::Recording { layer } = self.state {
            if let Some((start, vel)) = self.open[note as usize & 127].take() {
                let ch = self.open_ch[note as usize & 127];
                self.store(layer, start, self.beat, note, vel, ch, -1);
            }
        }
    }

    /// A beatbox hit Qimist played.
    pub fn hit(&mut self, class: usize, note: u8, vel: u8, ch: u8) {
        if let LoopState::Recording { layer } = self.state {
            self.store(layer, self.beat, self.beat + HIT_BEATS, note, vel, ch, class as i8);
        }
    }

    /// Advance one sample. `host_beat` is the host's position while its
    /// transport plays; otherwise the internal clock moves `dbeat` per sample.
    pub fn tick(&mut self, host_beat: Option<f64>, dbeat: f64, out: &mut impl FnMut(LoopOut)) {
        let prev = self.beat;
        self.host_clock = host_beat.is_some();
        self.beat = match host_beat {
            Some(b) => b,
            None => self.beat + dbeat,
        };
        if !self.started {
            self.started = true;
            return;
        }
        // Clock jumped (host relocated): just re-sync the cursors.
        if (self.beat - prev).abs() > 0.5 {
            self.all_off(None, out);
            if !matches!(self.state, LoopState::Stopped | LoopState::Waiting { .. }) {
                let pos = self.position();
                self.reset_cursors(pos);
            }
            return;
        }

        let running = !matches!(self.state, LoopState::Stopped);
        if self.metronome && running {
            // Beats counted from where the loop starts (or will start).
            let reference = match self.state {
                LoopState::Waiting { .. } => self.start_at,
                _ => self.origin,
            };
            let (b0, b1) = ((prev - reference).floor(), (self.beat - reference).floor());
            if b0 != b1 {
                out(LoopOut::Click { accent: b1.rem_euclid(4.0) < 0.5 });
            }
        }

        match self.state {
            LoopState::Stopped => return,
            LoopState::Waiting { record } => {
                if self.beat >= self.start_at {
                    self.origin = self.start_at;
                    self.reset_cursors(0.0);
                    self.state = LoopState::Playing;
                    if let Some(l) = record {
                        self.begin_recording(l);
                        self.rec_from = self.start_at;
                    }
                } else {
                    return;
                }
            }
            LoopState::Recording { layer } => {
                if self.beat - self.rec_from >= self.data.beats {
                    self.finish_recording(layer);
                }
            }
            LoopState::Playing => {}
        }

        // Ends of sounding notes.
        let mut k = 0;
        while k < self.n_sounding {
            if self.beat >= self.sounding[k].end {
                let s = self.sounding[k];
                out(LoopOut::Off { note: s.note, ch: s.ch });
                self.n_sounding -= 1;
                self.sounding[k] = self.sounding[self.n_sounding];
            } else {
                k += 1;
            }
        }

        // Starts in (prev_pos, pos], wrapping at the loop end.
        let len = self.data.beats;
        let p0 = (prev - self.origin).rem_euclid(len);
        let p1 = (self.beat - self.origin).rem_euclid(len);
        if p1 < p0 {
            self.play_until(len + 1.0, out);
            self.cursor = [0; LAYERS];
            if self.song_at.is_some() || self.next_scene.is_some() {
                let wrap_at = self.beat - p1;
                if !self.at_loop_end(wrap_at, out) {
                    return;
                }
                let p1 = (self.beat - self.origin).rem_euclid(self.data.beats);
                self.play_until(p1, out);
                return;
            }
        }
        self.play_until(p1, out);
    }

    fn play_until(&mut self, pos: f64, out: &mut impl FnMut(LoopOut)) {
        for l in 0..LAYERS {
            let layer = &self.data.layers[l];
            while self.cursor[l] < layer.notes.len() && layer.notes[self.cursor[l]].start <= pos {
                let n = layer.notes[self.cursor[l]];
                self.cursor[l] += 1;
                // A layer being recorded into still plays (overdub) unless muted.
                if layer.muted {
                    continue;
                }
                if self.n_sounding < MAX_SOUNDING {
                    // Retrigger: end the same note if it's still ringing.
                    if let Some(k) = self.sounding[..self.n_sounding].iter().position(|s| s.note == n.note && s.ch == n.ch) {
                        out(LoopOut::Off { note: n.note, ch: n.ch });
                        self.n_sounding -= 1;
                        self.sounding[k] = self.sounding[self.n_sounding];
                    }
                    out(LoopOut::On { note: n.note, vel: n.vel, ch: n.ch, drum: n.drum });
                    self.sounding[self.n_sounding] = Sounding { note: n.note, ch: n.ch, layer: l, end: self.beat + n.len };
                    self.n_sounding += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1 kHz "sample rate" at 120 BPM: 500 samples per beat.
    const DB: f64 = 2.0 / 1000.0;

    struct Rig {
        lp: Looper,
        log: Vec<(f64, LoopOut)>,
    }

    impl Rig {
        fn new() -> Self {
            let mut lp = Looper::new();
            lp.metronome = false;
            lp.count_in = false;
            Rig { lp, log: vec![] }
        }
        fn run_beats(&mut self, beats: f64) {
            for _ in 0..(beats / DB).round() as usize {
                let log = &mut self.log;
                let b = self.lp.beat;
                self.lp.tick(None, DB, &mut |e| log.push((b, e)));
            }
        }
        fn cmd(&mut self, c: LoopCmd) {
            let log = &mut self.log;
            let b = self.lp.beat;
            self.lp.command(c, &mut |e| log.push((b, e)));
        }
        fn ons(&self, after: f64) -> Vec<(f64, u8)> {
            self.log.iter().filter(|(b, _)| *b >= after).filter_map(|(b, e)| if let LoopOut::On { note, .. } = e { Some((*b, *note)) } else { None }).collect()
        }
    }

    #[test]
    fn records_one_loop_and_plays_it_back() {
        let mut r = Rig::new();
        r.run_beats(0.5);
        r.cmd(LoopCmd::Record(0));
        r.run_beats(0.01);
        assert!(r.lp.is_recording());
        let t0 = r.lp.beat;
        r.run_beats(1.07);
        r.lp.note_on(60, 100, 0);
        r.run_beats(1.0);
        r.lp.note_off(60);
        r.run_beats(8.0 - 2.07 - 0.01 + 0.2);
        assert_eq!(r.lp.state(), LoopState::Playing);
        // Quantised to beat 1 (1/16 grid = 0.25 beats).
        assert_eq!(r.lp.data.layers[0].notes.len(), 1);
        assert!((r.lp.data.layers[0].notes[0].start - 1.0).abs() < 0.03, "{:?}", r.lp.data.layers[0].notes[0]);
        // Not echoed during the take, then played on the next pass.
        assert!(r.ons(0.0).iter().all(|(b, _)| *b > t0 + 8.0), "{:?}", r.ons(0.0));
        r.run_beats(8.0);
        let ons = r.ons(0.0);
        assert_eq!(ons.len(), 1, "{ons:?}");
        assert!(((ons[0].0 - t0) - 9.0).abs() < 0.05, "{ons:?} t0 {t0}");
    }

    #[test]
    fn overdub_mute_undo() {
        let mut r = Rig::new();
        r.lp.quantize = 0.0;
        r.cmd(LoopCmd::Record(0));
        r.run_beats(0.5);
        r.lp.hit(0, 36, 110, 9);
        r.run_beats(8.0);
        r.cmd(LoopCmd::Record(1));
        r.run_beats(2.0);
        r.lp.note_on(64, 90, 0);
        r.run_beats(0.5);
        r.lp.note_off(64);
        r.run_beats(6.0);
        assert_eq!(r.lp.data.layers[0].notes.len(), 1);
        assert_eq!(r.lp.data.layers[1].notes.len(), 1);
        assert_eq!(r.lp.data.layers[0].notes[0].drum, 0);
        let mark = r.lp.beat;
        r.run_beats(8.0);
        assert_eq!(r.ons(mark).len(), 2);
        // Mute layer 0: only the melody plays.
        r.cmd(LoopCmd::ToggleMute(0));
        let mark = r.lp.beat;
        r.run_beats(8.0);
        assert_eq!(r.ons(mark).iter().map(|x| x.1).collect::<Vec<_>>(), [64]);
        // Undo the last take (layer 1) and redo it.
        r.cmd(LoopCmd::Undo);
        assert!(r.lp.data.layers[1].notes.is_empty());
        r.cmd(LoopCmd::Undo);
        assert_eq!(r.lp.data.layers[1].notes.len(), 1);
        // Stop sends note-offs for anything ringing.
        r.cmd(LoopCmd::Stop);
        assert_eq!(r.lp.state(), LoopState::Stopped);
    }

    #[test]
    fn count_in_clicks_then_records() {
        let mut r = Rig::new();
        r.lp.metronome = true;
        r.lp.count_in = true;
        r.run_beats(0.3);
        r.cmd(LoopCmd::Record(2));
        assert!(matches!(r.lp.state(), LoopState::Waiting { record: Some(2) }));
        r.run_beats(4.05);
        assert_eq!(r.lp.state(), LoopState::Recording { layer: 2 });
        let clicks: Vec<bool> = r.log.iter().filter_map(|(_, e)| if let LoopOut::Click { accent } = e { Some(*accent) } else { None }).collect();
        assert!(clicks.len() >= 4, "{clicks:?}");
        assert!(clicks.last() == Some(&true), "downbeat accent: {clicks:?}");
    }

    #[test]
    fn follows_the_host_bar_grid() {
        let mut lp = Looper::new();
        lp.metronome = false;
        lp.count_in = false;
        let mut beat = 5.0;
        let mut log = vec![];
        lp.tick(Some(beat), 0.0, &mut |e| log.push(e));
        lp.command(LoopCmd::Record(0), &mut |e| log.push(e));
        // Starts on the next bar line (beat 8).
        while beat < 7.9 {
            beat += DB;
            lp.tick(Some(beat), 0.0, &mut |e| log.push(e));
        }
        assert!(matches!(lp.state(), LoopState::Waiting { .. }));
        while beat < 8.1 {
            beat += DB;
            lp.tick(Some(beat), 0.0, &mut |e| log.push(e));
        }
        assert!(lp.is_recording());
        assert!(lp.position() < 0.2);
    }
    #[test]
    fn replace_layer_plays_the_edit() {
        let mut lp = Looper::new();
        lp.count_in = false;
        lp.metronome = false;
        let edited = [LoopNote { start: 1.0, len: 0.5, note: 64, vel: 90, ch: 0, drum: -1 }];
        let mut outs = Vec::new();
        assert!(lp.replace_layer(0, 2, &edited, &mut |o| outs.push(o)));
        assert!(lp.dirty);
        lp.command(LoopCmd::Play, &mut |_| {});
        let mut ons = Vec::new();
        for _ in 0..400 {
            lp.tick(None, 0.01, &mut |o| if let LoopOut::On { note, .. } = o { ons.push(note) });
        }
        assert_eq!(ons, [64]);
    }

    fn layer_with(note: u8, beats: f64) -> LoopData {
        let mut d = LoopData { beats, ..Default::default() };
        d.layers[0].notes.push(LoopNote { start: 0.0, len: 0.5, note, vel: 100, ch: 0, drum: -1 });
        d
    }

    #[test]
    fn scene_switch_waits_for_the_end_of_the_pass() {
        let mut r = Rig::new();
        r.lp.data = layer_with(60, 4.0);
        r.lp.scenes[1] = layer_with(70, 4.0);
        r.cmd(LoopCmd::Play);
        r.run_beats(1.0);
        r.cmd(LoopCmd::Scene(1));
        assert_eq!(r.lp.next_scene(), Some(1));
        r.run_beats(6.0);
        assert_eq!(r.lp.scene(), 1);
        let ons: Vec<u8> = r.ons(0.0).iter().map(|o| o.1).collect();
        // A plays once, B starts right on the next bar.
        assert_eq!(ons, [60, 70]);
        let b_start = r.ons(0.0)[1].0;
        assert!((b_start - 4.0).abs() < 0.01, "B started at {b_start}");
    }

    #[test]
    fn song_plays_the_arrangement_then_stops() {
        let mut r = Rig::new();
        r.lp.data = layer_with(60, 4.0);
        r.lp.scenes[1] = layer_with(70, 8.0);
        let song = Song::from_steps(&[SongStep { scene: 0, repeats: 2 }, SongStep { scene: 1, repeats: 1 }, SongStep { scene: 0, repeats: 1 }], false);
        r.cmd(LoopCmd::PlaySong(song));
        r.run_beats(30.0);
        let ons = r.ons(0.0);
        let notes: Vec<u8> = ons.iter().map(|o| o.1).collect();
        assert_eq!(notes, [60, 60, 70, 60]);
        // A A (4 beats each), B (8 beats), A: starts at 0, 4, 8, 16.
        let starts: Vec<f64> = ons.iter().map(|o| (o.0 - ons[0].0).round()).collect();
        assert_eq!(starts, [0.0, 4.0, 8.0, 16.0]);
        assert_eq!(r.lp.state(), LoopState::Stopped);
        assert_eq!(r.lp.song_step(), None);
    }

    #[test]
    fn copy_scene_and_save_load() {
        let mut lp = Looper::new();
        lp.data = layer_with(60, 4.0);
        lp.command(LoopCmd::CopyScene { from: 0, to: 2 }, &mut |_| {});
        assert_eq!(lp.scene_data(2).layers[0].notes[0].note, 60);
        lp.command(LoopCmd::Scene(2), &mut |_| {});
        assert_eq!(lp.scene(), 2);
        let mut saved = SongData::default();
        saved.reserve();
        lp.save(&mut saved);
        assert_eq!(saved.scene, 2);
        assert_eq!(saved.scenes[0].layers[0].notes.len(), 1);
        let mut back = Looper::new();
        back.load(&saved);
        assert_eq!(back.scene(), 2);
        assert_eq!(back.data.layers[0].notes[0].note, 60);
        assert_eq!(back.scene_data(0).layers[0].notes.len(), 1);
    }
}
