//! Qimist: sing or hum into the mic, get MIDI notes out.

use nih_plug::prelude::*;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use qimist_dsp::aec::EchoCanceller;
use qimist_dsp::arp::{Arp, ArpEvent};
use qimist_dsp::autotune::AutoTune;
use qimist_dsp::chords::MAX_CHORD;
use qimist_dsp::follow::{ChordEvent, ChordFollower};
use qimist_dsp::looper::{LoopOut, LoopState, Looper};
use qimist_dsp::drums::{brightness_voicing, rule_classify, DrumClass, DrumDetector, DrumKit, Features};
use qimist_dsp::fx::{FxChain, Limiter};
use qimist_dsp::synth::Synth;
use qimist_dsp::{EngineEvent, Scale, VoiceEngine};

mod editor;
pub mod params;
mod presets;
mod samples;
mod shared;

use params::{FxTarget, Mode, VoxParams};
use shared::{Shared, HISTORY_DECIMATE};

/// Drum MIDI notes are short triggers.
const DRUM_GATE_S: f32 = 0.06;

/// Recorder events that couldn't be written because the editor held the lock.
const PENDING_CAP: usize = 256;

#[derive(Clone, Copy)]
enum RecEvent {
    On { t: f64, note: u8, vel: u8, ch: u8 },
    Off { t: f64, note: u8 },
}

pub struct Qimist {
    params: Arc<VoxParams>,
    shared: Arc<Shared>,
    engine: Option<VoiceEngine>,
    synth: Synth,
    fx: FxChain,
    sr: f32,
    events: Vec<EngineEvent>,
    pending: Vec<RecEvent>,
    rec_clock: f64,
    was_recording: bool,
    frame_count: u32,
    peak: f32,
    wobble: [f32; 16],
    wobble_pos: usize,
    drums: DrumDetector,
    kit: DrumKit,
    /// Per drum pad: samples until its MIDI note-off (0 = not sounding) and the note/channel it used.
    drum_offs: [(u32, u8, u8); 3],
    sample_version: u32,
    limiter: Limiter,
    out_peak: f32,
    last_mode: Mode,
    clock: f64,
    /// Notes started for the current sung note (the note, or its chord).
    held: [u8; MAX_CHORD],
    n_held: usize,
    arp: Arp,
    arp_active: bool,
    arp_events: Vec<ArpEvent>,
    /// Scale chords and auto-tune use this block (locked, or detected and confident).
    scale: Option<Scale>,
    autotune: AutoTune,
    expression: f32,
    last_cc: i32,
    follower: ChordFollower,
    chord_events: Vec<ChordEvent>,
    /// Chord pad the GUI last asked for (−1 none) and the key the chord was built in.
    pad_state: i32,
    follow_scale: Option<Scale>,
    looper: Looper,
    loop_events: Vec<LoopOut>,
    /// Metronome click: envelope and oscillator phase/step.
    click_env: f32,
    click_phase: f32,
    click_step: f32,
    /// Removes our own output from the mic before the detectors.
    aec: EchoCanceller,
    /// Last output sample (mono): the echo canceller's reference.
    last_out: f32,
    was_loop_recording: bool,
}

impl Default for Qimist {
    fn default() -> Self {
        Qimist {
            params: Arc::new(VoxParams::default()),
            shared: Arc::new(Shared::default()),
            engine: None,
            synth: Synth::new(48_000.0),
            fx: FxChain::new(48_000.0),
            sr: 48_000.0,
            events: Vec::with_capacity(64),
            pending: Vec::with_capacity(PENDING_CAP),
            rec_clock: 0.0,
            was_recording: false,
            frame_count: 0,
            peak: 0.0,
            wobble: [0.0; 16],
            wobble_pos: 0,
            drums: DrumDetector::new(48_000.0),
            kit: DrumKit::new(48_000.0),
            drum_offs: [(0, 0, 0); 3],
            sample_version: u32::MAX,
            limiter: Limiter::new(48_000.0),
            out_peak: 0.0,
            last_mode: Mode::Melody,
            clock: 0.0,
            held: [0; MAX_CHORD],
            n_held: 0,
            arp: Arp::new(48_000.0),
            arp_active: false,
            arp_events: Vec::with_capacity(16),
            scale: None,
            autotune: AutoTune::new(48_000.0),
            expression: 0.0,
            last_cc: -1,
            follower: ChordFollower::new(48_000.0),
            chord_events: Vec::with_capacity(8),
            pad_state: -1,
            follow_scale: None,
            looper: Looper::new(),
            loop_events: Vec::with_capacity(64),
            click_env: 0.0,
            click_phase: 0.0,
            click_step: 0.0,
            aec: EchoCanceller::new(48_000.0),
            last_out: 0.0,
            was_loop_recording: false,
        }
    }
}

impl Qimist {
    fn channel(&self) -> u8 {
        (self.params.midi_channel.value() - 1).clamp(0, 15) as u8
    }

    fn record(&mut self, ev: RecEvent) {
        if !self.shared.recording.load(Relaxed) {
            return;
        }
        if self.pending.len() < PENDING_CAP {
            self.pending.push(ev);
        }
        self.flush_pending();
    }

    fn flush_pending(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        if let Some(mut rec) = self.shared.recorder.try_lock() {
            for ev in self.pending.drain(..) {
                match ev {
                    RecEvent::On { t, note, vel, ch } => rec.note_on(t, note, vel, ch),
                    RecEvent::Off { t, note } => rec.note_off(t, note),
                }
            }
        }
    }

    /// A note going out: MIDI, the synth and the recorder.
    fn out_on(&mut self, timing: u32, note: u8, velocity: u8, delay: u32, context: &mut impl ProcessContext<Self>) {
        let channel = self.channel();
        context.send_event(NoteEvent::NoteOn { timing, voice_id: None, channel, note, velocity: velocity as f32 / 127.0 });
        self.synth.note_on(note, velocity);
        self.looper.note_on(note, velocity, channel);
        self.shared.notes_sent.fetch_add(1, Relaxed);
        let t = self.rec_clock + (timing as f64 - delay as f64) / self.sr as f64;
        self.record(RecEvent::On { t, note, vel: velocity, ch: channel });
    }

    fn out_off(&mut self, timing: u32, note: u8, delay: u32, context: &mut impl ProcessContext<Self>) {
        let channel = self.channel();
        context.send_event(NoteEvent::NoteOff { timing, voice_id: None, channel, note, velocity: 0.0 });
        self.synth.note_off(note);
        self.looper.note_off(note);
        let t = self.rec_clock + (timing as f64 - delay as f64) / self.sr as f64;
        self.record(RecEvent::Off { t, note });
    }

    fn arp_out(&mut self, timing: u32, context: &mut impl ProcessContext<Self>) {
        let evs = std::mem::take(&mut self.arp_events);
        for e in &evs {
            match *e {
                ArpEvent::On { note, velocity } => {
                    self.out_on(timing, note, velocity, 0, context);
                    self.shared.arp_note.store(note as i32, Relaxed);
                }
                ArpEvent::Off { note } => {
                    self.out_off(timing, note, 0, context);
                    if self.shared.arp_note.load(Relaxed) == note as i32 {
                        self.shared.arp_note.store(-1, Relaxed);
                    }
                }
            }
        }
        self.arp_events = evs;
        self.arp_events.clear();
    }

    /// End everything the sung note started (chord notes or the arp).
    fn end_held(&mut self, timing: u32, context: &mut impl ProcessContext<Self>) {
        if self.arp_active {
            let evs = &mut self.arp_events;
            self.arp.stop(&mut |e| evs.push(e));
            self.arp_out(timing, context);
        } else {
            for k in 0..self.n_held {
                self.out_off(timing, self.held[k], 0, context);
            }
        }
        self.n_held = 0;
    }

    /// Play what the chord follower decided. Notes shared by the old and new
    /// chord keep sounding; only the ones that change are re-struck.
    fn apply_chords(&mut self, timing: u32, delay: u32, context: &mut impl ProcessContext<Self>) {
        if self.chord_events.is_empty() {
            return;
        }
        let evs = std::mem::take(&mut self.chord_events);
        for e in &evs {
            match *e {
                ChordEvent::Play { chord, velocity } => {
                    let new = chord.notes();
                    if self.arp_active {
                        self.arp.set_held(new, velocity);
                    } else {
                        for k in 0..self.n_held {
                            let n = self.held[k];
                            if !new.contains(&n) {
                                self.out_off(timing, n, delay, context);
                            }
                        }
                        for &n in new {
                            if !self.held[..self.n_held].contains(&n) {
                                self.out_on(timing, n, velocity, delay, context);
                            }
                        }
                    }
                    self.n_held = new.len();
                    self.held[..self.n_held].copy_from_slice(new);
                }
                ChordEvent::Release => {
                    if self.arp_active {
                        self.arp.set_held(&[], 0);
                    } else {
                        for k in 0..self.n_held {
                            self.out_off(timing, self.held[k], delay, context);
                        }
                    }
                    self.n_held = 0;
                }
            }
        }
        self.chord_events = evs;
        self.chord_events.clear();
        self.publish_chord();
    }

    fn publish_chord(&self) {
        let s = &self.shared;
        s.chord_root.store(self.follower.current_root().map_or(-1, |r| r as i32), Relaxed);
        s.chord_pending.store(self.follower.pending_root().map_or(-1, |r| r as i32), Relaxed);
        let chord = self.follower.current_chord();
        let notes = chord.as_ref().map_or(&[][..], |c| c.notes());
        for (k, slot) in s.chord_notes.iter().enumerate() {
            slot.store(notes.get(k).map_or(-1, |&n| n as i32), Relaxed);
        }
    }

    /// A take just started: notes already sounding (a ringing chord, a held
    /// note) belong in it too, from its first beat.
    fn seed_loop_take(&mut self) {
        let rec = self.looper.is_recording();
        if rec && !self.was_loop_recording && !self.arp_active {
            let ch = self.channel();
            let vel = self.shared.velocity.load(Relaxed).clamp(1, 127) as u8;
            for k in 0..self.n_held {
                self.looper.note_on(self.held[k], vel, ch);
            }
        }
        self.was_loop_recording = rec;
    }

    /// Play what the loop station sends (not recorded back into the loop).
    fn apply_loop(&mut self, timing: u32, context: &mut impl ProcessContext<Self>) {
        if self.loop_events.is_empty() {
            return;
        }
        let evs = std::mem::take(&mut self.loop_events);
        for e in &evs {
            match *e {
                LoopOut::On { note, vel, ch, drum } => {
                    context.send_event(NoteEvent::NoteOn { timing, voice_id: None, channel: ch, note, velocity: vel as f32 / 127.0 });
                    if drum >= 0 {
                        self.kit.trigger(DrumClass::from_index(drum as usize), vel);
                    } else if self.params.synth_on.value() {
                        self.synth.note_on(note, vel);
                    }
                }
                LoopOut::Off { note, ch } => {
                    context.send_event(NoteEvent::NoteOff { timing, voice_id: None, channel: ch, note, velocity: 0.0 });
                    self.synth.note_off(note);
                }
                LoopOut::Click { accent } => {
                    self.click_env = if accent { 0.5 } else { 0.3 };
                    self.click_phase = 0.0;
                    self.click_step = (if accent { 1760.0 } else { 1320.0 }) / self.sr;
                }
            }
        }
        self.loop_events = evs;
        self.loop_events.clear();
    }

    fn send(&mut self, timing: u32, e: EngineEvent, context: &mut impl ProcessContext<Self>) {
        let channel = self.channel();
        let chords_mode = self.last_mode == Mode::Chords;
        match e {
            EngineEvent::NoteOn { note, velocity, delay_samples } => {
                self.shared.sounding.store(note as i32, Relaxed);
                self.shared.velocity.store(velocity as i32, Relaxed);
                if chords_mode {
                    let (scale, evs) = (self.scale, &mut self.chord_events);
                    self.follower.note_on(note, velocity, scale, &mut |e| evs.push(e));
                    self.apply_chords(timing, delay_samples, context);
                    return;
                }
                // Melody mode plays just the note.
                self.n_held = 1;
                self.held[0] = note;
                if self.arp_active {
                    let held = self.held;
                    self.arp.set_held(&held[..self.n_held], velocity);
                } else {
                    for k in 0..self.n_held {
                        self.out_on(timing, self.held[k], velocity, delay_samples, context);
                    }
                }
            }
            EngineEvent::NoteOff { delay_samples, .. } => {
                self.shared.sounding.store(-1, Relaxed);
                if chords_mode {
                    // The follower decides when the chord ends (LET RING).
                    self.follower.note_off();
                    return;
                }
                if self.arp_active {
                    self.arp.set_held(&[], 0);
                } else {
                    for k in 0..self.n_held {
                        self.out_off(timing, self.held[k], delay_samples, context);
                    }
                }
                self.n_held = 0;
            }
            EngineEvent::PitchBend { semitones } => {
                let range = self.params.bend_range.value() as f32;
                let value = (0.5 + semitones / range * 0.5).clamp(0.0, 1.0);
                context.send_event(NoteEvent::MidiPitchBend { timing, channel, value });
                self.synth.set_bend(semitones);
            }
        }
    }

    /// One beatbox hit: classify (or train), send MIDI on channel 10, play the kit, record.
    fn drum_hit(&mut self, timing: u32, features: Features, velocity: u8, delay: u32, context: &mut impl ProcessContext<Self>) {
        let s = &self.shared;
        let training = s.train_class.load(Relaxed);
        let class = if training >= 0 {
            let class = DrumClass::from_index(training as usize);
            if let Some(mut model) = self.params.drum_model.try_write() {
                model.add(class, features);
                if s.train_left.fetch_sub(1, Relaxed) <= 1 {
                    s.train_class.store(-1, Relaxed);
                }
            }
            class
        } else {
            self.params.drum_model.try_read().map(|m| m.classify(&features)).unwrap_or_else(|| rule_classify(&features))
        };

        let idx = class.index();
        let note = self.params.drum_notes()[idx];
        let ch = (self.params.drum_channel.value() - 1).clamp(0, 15) as u8;
        let (left, old_note, old_ch) = self.drum_offs[idx];
        if left > 0 {
            context.send_event(NoteEvent::NoteOff { timing, voice_id: None, channel: old_ch, note: old_note, velocity: 0.0 });
        }
        context.send_event(NoteEvent::NoteOn { timing, voice_id: None, channel: ch, note, velocity: velocity as f32 / 127.0 });
        self.drum_offs[idx] = ((DRUM_GATE_S * self.sr) as u32, note, ch);
        self.kit.trigger(class, velocity);
        self.looper.hit(idx, note, velocity, ch);
        let (bright, voicing) = brightness_voicing(&features);
        s.push_hit(idx, velocity, (self.clock + timing as f64 / self.sr as f64) as f32, bright, voicing);
        s.notes_sent.fetch_add(1, Relaxed);

        let t = self.rec_clock + (timing as f64 - delay as f64) / self.sr as f64;
        self.record(RecEvent::On { t, note, vel: velocity, ch });
        self.record(RecEvent::Off { t: t + DRUM_GATE_S as f64, note });
    }

    /// Singer's loudness above the gate → synth dynamics and the expression CC.
    fn expression(&mut self, timing: u32, frame: &qimist_dsp::PitchFrame, context: &mut impl ProcessContext<Self>) {
        let target = ((frame.rms_db - self.params.gate.value()) / 30.0).clamp(0.0, 1.0);
        // Fast rise, slower fall, so note tails don't snap shut.
        let c = if target > self.expression { 0.35 } else { 0.08 };
        self.expression += (target - self.expression) * c;
        self.synth.set_expression(self.expression);
        self.shared.expression.store(self.expression, Relaxed);
        let cc = self.params.expr_cc.value();
        if cc == 0 {
            self.last_cc = -1;
            return;
        }
        let v = (self.expression * 127.0).round() as i32;
        if v != self.last_cc {
            self.last_cc = v;
            context.send_event(NoteEvent::MidiCC { timing, channel: self.channel(), cc: cc as u8, value: v as f32 / 127.0 });
        }
    }

    fn update_stats(&mut self, engine: &VoiceEngine, frame: &qimist_dsp::PitchFrame) {
        let s = &self.shared;
        s.publish_frame(frame);
        self.frame_count = self.frame_count.wrapping_add(1);

        // Short-term pitch wobble → vibrato depth and stability readouts.
        if frame.voiced {
            self.wobble[self.wobble_pos % self.wobble.len()] = frame.midi;
            self.wobble_pos += 1;
            let n = self.wobble_pos.min(self.wobble.len());
            let w = &self.wobble[..n];
            let mean = w.iter().sum::<f32>() / n as f32;
            let sd = (w.iter().map(|m| (m - mean).powi(2)).sum::<f32>() / n as f32).sqrt() * 100.0;
            s.wobble_cents.store(sd * std::f32::consts::SQRT_2, Relaxed);
            s.stability.store((1.0 - sd / 40.0).clamp(0.0, 1.0) * frame.confidence.min(1.0), Relaxed);
        } else {
            self.wobble_pos = 0;
            s.stability.store(0.0, Relaxed);
        }

        if self.frame_count % HISTORY_DECIMATE == 0 {
            s.push_history(if frame.voiced { frame.midi } else { f32::NAN });
        }
        if self.frame_count % 25 == 0 {
            s.publish_key(engine.key_estimate(), engine.active_scale());
            s.voice_db.store(engine.voice_level_db(), Relaxed);
        }
    }
}

impl Plugin for Qimist {
    const NAME: &'static str = "Qimist";
    const VENDOR: &'static str = "Qimist";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout { main_input_channels: NonZeroU32::new(2), main_output_channels: NonZeroU32::new(2), ..AudioIOLayout::const_default() },
        AudioIOLayout { main_input_channels: NonZeroU32::new(1), main_output_channels: NonZeroU32::new(2), ..AudioIOLayout::const_default() },
        AudioIOLayout { main_input_channels: NonZeroU32::new(1), main_output_channels: NonZeroU32::new(1), ..AudioIOLayout::const_default() },
    ];

    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone(), self.shared.clone())
    }

    fn initialize(&mut self, _layout: &AudioIOLayout, buffer_config: &BufferConfig, _context: &mut impl InitContext<Self>) -> bool {
        self.sr = buffer_config.sample_rate;
        let engine = VoiceEngine::new(self.sr as f64, self.params.engine_config());
        self.shared.latency_ms.store(engine.latency_samples() as f32 / self.sr * 1000.0, Relaxed);
        self.engine = Some(engine);
        self.synth = Synth::new(self.sr);
        self.fx = FxChain::new(self.sr);
        self.drums = DrumDetector::new(self.sr);
        self.kit = DrumKit::new(self.sr);
        self.arp = Arp::new(self.sr);
        self.follower = ChordFollower::new(self.sr);
        self.aec = EchoCanceller::new(self.sr);
        // Loops saved with the project; make room so recording never allocates.
        {
            let mut song = self.params.song.write();
            // Projects from before scenes: their loop becomes scene A.
            let mut old = self.params.loops.write();
            if !old.is_empty() && song.scenes.iter().all(|s| s.is_empty()) {
                song.scenes[0] = old.clone();
            }
            *old = Default::default();
            song.reserve();
        }
        self.looper = Looper::new();
        self.looper.load(&self.params.song.read());
        self.shared.standalone.store(_context.plugin_api() == PluginApi::Standalone, Relaxed);
        self.autotune = AutoTune::new(self.sr);
        self.limiter = Limiter::new(self.sr);
        // Custom drum samples are decoded here (not on the audio thread) at the new rate.
        self.shared.sample_rate.store(self.sr, Relaxed);
        samples::load_all(&self.params, &self.shared);
        self.sample_version = u32::MAX;
        if let Some(e) = &self.engine {
            self.autotune.set_alignment(e.latency_samples());
        }
        true
    }

    fn reset(&mut self) {
        if let Some(e) = &mut self.engine {
            e.reset(&mut |_| {});
        }
        self.shared.sounding.store(-1, Relaxed);
        self.synth.reset();
        self.fx.reset();
        self.limiter.reset();
        self.drums.reset();
        self.kit.reset();
        self.arp.stop(&mut |_| {});
        self.n_held = 0;
        self.follower.reset();
        self.aec.reset();
        self.pad_state = -1;
        self.publish_chord();
        self.autotune.reset();
    }

    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &mut impl ProcessContext<Self>) -> ProcessStatus {
        let Some(mut engine) = self.engine.take() else { return ProcessStatus::Normal };

        // Settings. Changing range/latency rebuilds the analyser (rare, user-driven).
        let cfg = self.params.engine_config();
        if cfg != *engine.config() {
            engine.set_config(cfg);
            self.shared.latency_ms.store(engine.latency_samples() as f32 / self.sr * 1000.0, Relaxed);
            self.autotune.set_alignment(engine.latency_samples());
        }
        self.scale = engine.active_scale();
        if self.shared.standalone.load(Relaxed) {
            self.shared.host_bpm.store(self.params.tempo.value() as f32, Relaxed);
        } else if let Some(bpm) = context.transport().tempo {
            self.shared.host_bpm.store(bpm as f32, Relaxed);
        }
        if self.shared.clear_key_request.swap(false, Relaxed) {
            engine.clear_key();
        }
        if self.shared.clear_request.swap(false, Relaxed) {
            self.pending.clear();
            self.rec_clock = 0.0;
        }

        // Start/stop of recording.
        let recording = self.shared.recording.load(Relaxed);
        if recording && !self.was_recording {
            self.rec_clock = 0.0;
            if !self.arp_active {
                let ch = self.channel();
                for &note in &self.held[..self.n_held] {
                    self.pending.push(RecEvent::On { t: 0.0, note, vel: 100, ch });
                }
            }
        }
        if !recording && self.was_recording {
            let t = self.rec_clock;
            if let Some(mut rec) = self.shared.recorder.try_lock() {
                rec.close_all(t);
            }
        }
        self.was_recording = recording;

        self.synth.set_settings(self.params.synth_settings());
        self.fx.chorus.set(self.params.chorus_settings());
        self.fx.eq.set(self.params.eq_gains());
        self.fx.reverb.set(self.params.reverb_settings());
        self.fx.delay.set(self.params.delay_settings(self.shared.host_bpm.load(Relaxed)));
        // New drum samples from the GUI / initialize().
        let v = self.shared.sample_version.load(Relaxed);
        if v != self.sample_version {
            if let Some(slots) = self.shared.drum_sample_slots.try_lock() {
                for c in DrumClass::ALL {
                    self.kit.set_sample(c, slots[c.index()].clone());
                }
                self.sample_version = v;
            }
        }
        let transport = context.transport();
        let beat0 = if transport.playing { transport.pos_beats() } else { None };
        // Mode switch: end whatever the melody engine had sounding.
        let mode = self.params.mode.value();
        let drums = mode == Mode::Drums;
        if mode != self.last_mode {
            let events = &mut self.events;
            engine.flush(&mut |e| events.push(e));
            if self.last_mode != Mode::Drums {
                let evs = std::mem::take(&mut self.events);
                for e in &evs {
                    self.send(0, *e, context);
                }
                self.events = evs;
            }
            self.events.clear();
            if self.last_mode == Mode::Chords {
                let evs = &mut self.chord_events;
                self.follower.stop(&mut |e| evs.push(e));
                self.apply_chords(0, 0, context);
            }
            self.synth.all_off();
            self.last_mode = mode;
        }
        // Arp switched on/off: end what's sounding; the next sung note starts fresh.
        let arp_on = self.params.arp_on.value() && !drums;
        if arp_on != self.arp_active {
            self.end_held(0, context);
            self.arp_active = arp_on;
        }
        self.arp.set(self.params.arp_settings());
        self.synth.set_legato(!arp_on);
        self.autotune.set_settings(self.params.autotune_settings());
        let (fmin, fmax) = self.params.range.value().hz();
        self.autotune.set_range(fmin, fmax);
        self.shared.monitor_ms.store(self.autotune.latency_samples() as f32 / self.sr * 1000.0, Relaxed);
        let bpm = self.shared.host_bpm.load(Relaxed);

        // Chord follower: settings, re-voicing after a style/key change, pads.
        let chords_mode = mode == Mode::Chords;
        if chords_mode {
            let fs = self.params.follow_settings();
            let old = self.follower.s;
            let restyle = (fs.kind, fs.inversion, fs.spread, fs.bass, fs.smooth) != (old.kind, old.inversion, old.spread, old.bass, old.smooth) || self.scale != self.follow_scale;
            self.follower.s = fs;
            self.follow_scale = self.scale;
            let (scale, evs) = (self.scale, &mut self.chord_events);
            if restyle {
                self.follower.refresh(scale, &mut |e| evs.push(e));
            }
            let pad = self.shared.chord_pad.load(Relaxed);
            if pad != self.pad_state {
                self.follower.pad_release(&mut |e| evs.push(e));
                if pad >= 0 {
                    self.follower.pad_press(pad as usize, 100, scale, &mut |e| evs.push(e));
                }
                self.pad_state = pad;
            }
            self.apply_chords(0, 0, context);
        }
        let synth_on = self.params.synth_on.value();
        // (In drums mode live melody notes are muted upstream; loop melodies still play.)
        if !synth_on {
            self.synth.all_off();
        }

        // Loop station: settings and commands from the GUI.
        self.looper.metronome = self.params.loop_metronome.value();
        self.looper.count_in = self.params.loop_count_in.value();
        self.looper.quantize = self.params.loop_quantize.value().beats();
        if self.looper.set_beats(self.params.loop_bars.value().beats()) {
            self.looper.dirty = true;
        }
        if let Some(mut cmds) = self.shared.loop_cmds.try_lock() {
            let evs = &mut self.loop_events;
            for c in cmds.drain(..) {
                self.looper.command(c, &mut |e| evs.push(e));
            }
        }
        if let Some(mut edit) = self.shared.loop_edit.try_lock() {
            if edit.pending {
                let evs = &mut self.loop_events;
                self.looper.replace_layer(edit.scene, edit.layer, &edit.notes, &mut |e| evs.push(e));
                edit.pending = false;
            }
        }
        self.apply_loop(0, context);
        self.seed_loop_take();
        let loop_dbeat = bpm as f64 / 60.0 / self.sr as f64;
        self.drums.set_sensitivity(self.params.drum_sens.value());
        self.drums.gate_db = self.params.gate.value() + 10.0;
        self.kit.set(self.params.kit_settings());
        let target = self.params.fx_target.value();
        let mut peak = self.peak * 0.9;
        let mut frames: [Option<qimist_dsp::PitchFrame>; 1] = [None];
        let echo_cancel = self.params.echo_cancel.value();

        for (i, mut ch) in buffer.iter_samples().enumerate() {
            let nch = ch.len();
            let mut ins = [0.0f32; 2];
            let mut sum = 0.0;
            for (c, s) in ch.iter_mut().enumerate() {
                if c < 2 {
                    ins[c] = *s;
                }
                sum += *s;
            }
            if nch == 1 {
                ins[1] = ins[0];
            }
            let mono = sum / nch.max(1) as f32;
            peak = peak.max(mono.abs());

            let events = &mut self.events;
            // Detectors hear the mic minus our own sound from the speakers.
            let heard = if echo_cancel { self.aec.process(mono, self.last_out) } else { mono };
            if let Some(f) = engine.push(heard, &mut |e| events.push(e)) {
                frames[0] = Some(f);
            }
            if drums {
                // Melody notes are muted in drums mode; the engine keeps running for the tuner.
                self.events.clear();
                if let Some((f, vel, delay)) = self.drums.push(heard) {
                    self.drum_hit(i as u32, f, vel, delay, context);
                }
                for (left, note, ch) in self.drum_offs.iter_mut() {
                    if *left > 0 {
                        *left -= 1;
                        if *left == 0 {
                            context.send_event(NoteEvent::NoteOff { timing: i as u32, voice_id: None, channel: *ch, note: *note, velocity: 0.0 });
                        }
                    }
                }
            }
            if !self.events.is_empty() {
                let evs = std::mem::take(&mut self.events);
                for e in &evs {
                    self.send(i as u32, *e, context);
                }
                self.events = evs;
                self.events.clear();
            }
            {
                let beat = beat0.map(|b| b + i as f64 * loop_dbeat);
                let evs = &mut self.loop_events;
                self.looper.tick(beat, loop_dbeat, &mut |e| evs.push(e));
                self.apply_loop(i as u32, context);
                self.seed_loop_take();
            }
            if chords_mode {
                let beat = beat0.map(|b| b + i as f64 * bpm as f64 / 60.0 / self.sr as f64);
                let (scale, evs) = (self.scale, &mut self.chord_events);
                self.follower.tick(beat, bpm, scale, &mut |e| evs.push(e));
                self.apply_chords(i as u32, 0, context);
            }
            if self.arp_active {
                let evs = &mut self.arp_events;
                let beat = beat0.map(|b| b + i as f64 * bpm as f64 / 60.0 / self.sr as f64);
                self.arp.tick(bpm, beat, &mut |e| evs.push(e));
                if !self.arp_events.is_empty() {
                    self.arp_out(i as u32, context);
                }
            }
            if let Some(f) = frames[0].take() {
                self.update_stats(&engine, &f);
                self.expression(i as u32, &f, context);
                if self.autotune.is_active() {
                    self.autotune.on_frame(&f, self.scale);
                    let s = &self.shared;
                    match self.autotune.target_note() {
                        Some(n) => {
                            s.at_note.store(n, Relaxed);
                            s.at_cents.store((n as f32 - self.autotune.sung_midi()) * 100.0, Relaxed);
                        }
                        _ => s.at_note.store(-1, Relaxed),
                    }
                }
            }

            // Sound: synth + monitored voice, effects on the chosen source.
            let vol = self.params.synth_volume.smoothed.next();
            let mon = self.params.monitor.smoothed.next();
            let dvol = self.params.drum_volume.smoothed.next();
            let (sl, sr) = self.synth.process();
            let drum = self.kit.process() * dvol;
            let (syn_l, syn_r) = (sl * vol + drum, sr * vol + drum);
            let (vl, vr) = if self.autotune.is_active() {
                let (tuned, harmony) = self.autotune.process(mono);
                let v = (tuned + harmony) * mon;
                (v, v)
            } else {
                (ins[0] * mon, ins[1] * mon)
            };
            // `ours` = what we play minus the monitored voice: the echo
            // canceller must never be told the voice is "our sound".
            let ((l, r), ours) = match target {
                FxTarget::Synth => {
                    let (l, r) = self.fx.process(syn_l, syn_r);
                    ((l + vl, r + vr), 0.5 * (l + r))
                }
                FxTarget::Voice => {
                    let (l, r) = self.fx.process(vl, vr);
                    ((l + syn_l, r + syn_r), 0.5 * (syn_l + syn_r))
                }
                FxTarget::Both => (self.fx.process(syn_l + vl, syn_r + vr), 0.5 * (syn_l + syn_r)),
            };
            // Metronome: a short sine blip, after the effects.
            let click = if self.click_env > 1e-4 {
                self.click_phase = (self.click_phase + self.click_step) % 1.0;
                self.click_env *= 0.998;
                (std::f32::consts::TAU * self.click_phase).sin() * self.click_env
            } else {
                0.0
            };
            let (l, r) = (l + click, r + click);
            let ours = ours + click;
            self.limiter.set_gain_db(self.params.output_gain.smoothed.next());
            let (l, r) = self.limiter.process(l, r);
            self.out_peak = self.out_peak.max(l.abs()).max(r.abs());
            // (Output gain and the limiter only scale it; the canceller adapts to that.)
            self.last_out = ours;
            for (c, s) in ch.iter_mut().enumerate() {
                *s = match (nch, c) {
                    (1, _) => 0.5 * (l + r),
                    (_, 0) => l,
                    (_, 1) => r,
                    _ => 0.0,
                };
            }
        }

        self.clock += buffer.samples() as f64 / self.sr as f64;
        self.shared.clock.store(self.clock as f32, Relaxed);
        self.peak = peak;
        self.shared.input_peak.store(peak, Relaxed);
        self.shared.out_peak.store(self.out_peak, Relaxed);
        self.out_peak *= 0.85;
        self.shared.limiter_gr.store(self.limiter.reduction_db, Relaxed);
        self.shared.echo_db.store(if echo_cancel { self.aec.reduction_db() } else { 0.0 }, Relaxed);

        // Loop station → GUI, and the saved copy (copy_from reuses the reserved room).
        let (st, layer) = match self.looper.state() {
            LoopState::Stopped => (0, -1),
            LoopState::Waiting { record } => (1, record.map_or(-1, |l| l as i32)),
            LoopState::Playing => (2, -1),
            LoopState::Recording { layer } => (3, layer as i32),
        };
        self.shared.loop_state.store(st, Relaxed);
        self.shared.loop_layer.store(layer, Relaxed);
        self.shared.loop_pos.store(self.looper.position() as f32, Relaxed);
        self.shared.loop_scene.store(self.looper.scene() as i32, Relaxed);
        self.shared.loop_next_scene.store(self.looper.next_scene().map_or(-1, |s| s as i32), Relaxed);
        self.shared.loop_song_step.store(self.looper.song_step().map_or(-1, |s| s as i32), Relaxed);
        if self.looper.dirty {
            if let Some(mut saved) = self.params.song.try_write() {
                self.looper.save(&mut saved);
                self.looper.dirty = false;
            }
        }
        if recording {
            self.rec_clock += buffer.samples() as f64 / self.sr as f64;
            self.shared.rec_time.store(self.rec_clock as f32, Relaxed);
        }
        self.flush_pending();
        self.engine = Some(engine);
        ProcessStatus::Normal
    }

    fn deactivate(&mut self) {
        if let Some(e) = &mut self.engine {
            e.flush(&mut |_| {});
        }
    }
}

impl ClapPlugin for Qimist {
    const CLAP_ID: &'static str = "com.qimist.qimist";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Voice to MIDI: pitch, notes and key from your mic");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] =
        &[ClapFeature::AudioEffect, ClapFeature::NoteEffect, ClapFeature::Analyzer, ClapFeature::Stereo, ClapFeature::Mono];
}

impl Vst3Plugin for Qimist {
    const VST3_CLASS_ID: [u8; 16] = *b"QimistVoiceMidi1";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Analyzer];
}

nih_export_clap!(Qimist);
nih_export_vst3!(Qimist);
