//! Performance row: chords, arpeggiator and voice auto-tune / harmony.

use super::widgets::*;
use super::hslider;
use crate::params::{ArpPatternChoice, ArpRate, ChordChoice, Mode, VoxParams};
use crate::shared::Shared;
use nih_plug::prelude::ParamSetter;
use nih_plug_egui::egui::{vec2, Align2, Painter, Rect, Stroke, Ui};
use std::sync::atomic::Ordering::Relaxed;
use qimist_dsp::{note_name, NOTE_NAMES};

pub const ROW_Y: f32 = 568.0;

pub fn row(ui: &mut Ui, p: &Painter, o: Origin, setter: &ParamSetter, params: &VoxParams, shared: &Shared) {
    let y = ROW_Y;
    label(p, o.pos(28.0, y + 8.0), "PERFORMANCE", col::MUTED);
    p.line_segment([o.pos(112.0, y + 8.0), o.pos(1172.0, y + 8.0)], Stroke::new(1.0_f32, col::LINE));
    let top = y + 22.0;
    let h = 72.0;
    chords_panel(ui, p, o.rect(28.0, top, 430.0, h), setter, params, shared);
    arp_panel(ui, p, o.rect(470.0, top, 330.0, h), setter, params, shared);
    autotune_panel(ui, p, o.rect(812.0, top, 360.0, h), setter, params, shared);
}

fn chords_panel(ui: &mut Ui, p: &Painter, r: Rect, setter: &ParamSetter, params: &VoxParams, shared: &Shared) {
    panel(p, r, col::PANEL);
    let active = params.mode.value() == Mode::Chords;
    text(p, r.min + vec2(14.0, 16.0), Align2::LEFT_CENTER, "CHORDS", 12.0, col::CREAM, false);
    led(p, r.min + vec2(72.0, 16.0), active && shared.chord_root.load(Relaxed) >= 0, col::AMBER);

    if !active {
        // One click into chords mode, right here.
        if button(ui, Rect::from_min_size(r.min + vec2(86.0, 6.0), vec2(118.0, 20.0)), "USE CHORDS ▶", col::AMBER, false) {
            setter.begin_set_parameter(&params.mode);
            setter.set_parameter(&params.mode, Mode::Chords);
            setter.end_set_parameter(&params.mode);
        }
    } else {
        let root = shared.chord_root.load(Relaxed);
        let (readout, c) = if root >= 0 {
            let notes: Vec<&str> = shared.chord_notes.iter().map(|n| n.load(Relaxed)).filter(|&n| n >= 0).map(|n| NOTE_NAMES[(n % 12) as usize]).collect();
            (format!("{} · {}", params.chord_for(root as u8, shared.active_scale()).name(), notes.join(" ")), col::AMBER)
        } else {
            ("sing a note".to_string(), col::MUTED)
        };
        text(p, r.min + vec2(86.0, 16.0), Align2::LEFT_CENTER, readout, 10.0, c, true);
    }

    label(p, r.min + vec2(218.0, 16.0), "SMOOTH", col::MUTED);
    toggle(ui, setter, &params.chord_smooth, Rect::from_min_size(r.min + vec2(258.0, 8.0), vec2(28.0, 16.0)));
    label(p, r.min + vec2(294.0, 16.0), "OPEN", col::MUTED);
    toggle(ui, setter, &params.chord_spread, Rect::from_min_size(r.min + vec2(322.0, 8.0), vec2(28.0, 16.0)));
    label(p, r.min + vec2(358.0, 16.0), "BASS", col::MUTED);
    toggle(ui, setter, &params.chord_bass, Rect::from_min_size(r.min + vec2(388.0, 8.0), vec2(28.0, 16.0)));

    // Styles, in plain words.
    enum_pills(
        ui,
        setter,
        &params.chord_type,
        Rect::from_min_size(r.min + vec2(14.0, 30.0), vec2(306.0, 22.0)),
        &[
            (ChordChoice::Triad, "POP"),
            (ChordChoice::Seventh, "SOUL"),
            (ChordChoice::Ninth, "JAZZ"),
            (ChordChoice::Sus2, "DREAMY"),
            (ChordChoice::Sus4, "FOLK"),
            (ChordChoice::Power, "ROCK"),
            (ChordChoice::Octaves, "OCTAVE"),
        ],
        col::AMBER,
    );
    if params.chord_smooth.value() {
        text(p, r.min + vec2(372.0, 41.0), Align2::CENTER_CENTER, "auto voicing", 9.0, col::DIM, false);
    } else {
        let inv = params.chord_inversion.value();
        let shown = if inv == 0 { "ROOT".to_string() } else { format!("INV {inv}") };
        stepper(ui, setter, &params.chord_inversion, Rect::from_min_size(r.min + vec2(330.0, 30.0), vec2(86.0, 22.0)), &shown);
    }

    let style = match params.chord_type.value() {
        ChordChoice::Triad => "simple 3-note chords",
        ChordChoice::Seventh => "warm 7th chords",
        ChordChoice::Ninth => "rich 9th chords",
        ChordChoice::Sus2 => "open sus2 chords, no 3rd",
        ChordChoice::Sus4 => "bright sus4 chords",
        ChordChoice::Power => "power chords: root + 5th",
        ChordChoice::Octaves => "just the note, in octaves",
    };
    let key = match shared.active_scale() {
        Some(s) => format!("in {}", s.name()),
        None => "no key yet (major chords)".to_string(),
    };
    text(p, r.min + vec2(14.0, 63.0), Align2::LEFT_CENTER, format!("{style} · {key}"), 9.0, col::MUTED, false);
}

fn arp_panel(ui: &mut Ui, p: &Painter, r: Rect, setter: &ParamSetter, params: &VoxParams, shared: &Shared) {
    panel(p, r, col::PANEL);
    let on = params.arp_on.value();
    text(p, r.min + vec2(14.0, 16.0), Align2::LEFT_CENTER, "ARPEGGIATOR", 12.0, col::CREAM, false);
    let n = shared.arp_note.load(Relaxed);
    led(p, r.min + vec2(108.0, 16.0), on && n >= 0, col::CYAN);
    let bpm = shared.host_bpm.load(Relaxed);
    let readout = if !on { format!("{bpm:.0} BPM") } else if n >= 0 { note_name(n as u8) } else { "sing…".to_string() };
    text(p, r.min + vec2(122.0, 16.0), Align2::LEFT_CENTER, readout, 10.0, if on { col::CYAN } else { col::DIM }, true);
    label(p, r.min + vec2(176.0, 16.0), "GATE", col::MUTED);
    hslider(ui, setter, &params.arp_gate, Rect::from_min_size(r.min + vec2(206.0, 13.0), vec2(36.0, 6.0)));
    toggle(ui, setter, &params.arp_on, Rect::from_min_size(r.min + vec2(r.width() - 52.0, 7.0), vec2(38.0, 18.0)));

    enum_pills(
        ui,
        setter,
        &params.arp_rate,
        Rect::from_min_size(r.min + vec2(14.0, 30.0), vec2(r.width() - 28.0, 20.0)),
        &[(ArpRate::Q4, "1/4"), (ArpRate::Q8, "1/8"), (ArpRate::Q8T, "1/8T"), (ArpRate::Q16, "1/16"), (ArpRate::Q16T, "1/16T"), (ArpRate::Q32, "1/32")],
        col::CYAN,
    );
    enum_pills(
        ui,
        setter,
        &params.arp_pattern,
        Rect::from_min_size(r.min + vec2(14.0, 52.0), vec2(206.0, 16.0)),
        &[(ArpPatternChoice::Up, "UP"), (ArpPatternChoice::Down, "DOWN"), (ArpPatternChoice::UpDown, "UP·DN"), (ArpPatternChoice::Random, "RAND")],
        col::CYAN,
    );
    let oct = params.arp_octaves.value();
    stepper(ui, setter, &params.arp_octaves, Rect::from_min_size(r.min + vec2(228.0, 51.0), vec2(88.0, 18.0)), &format!("{oct} OCT"));
}

fn autotune_panel(ui: &mut Ui, p: &Painter, r: Rect, setter: &ParamSetter, params: &VoxParams, shared: &Shared) {
    panel(p, r, col::PANEL);
    let on = params.autotune_on.value();
    let harm = params.harmony_on.value();
    text(p, r.min + vec2(14.0, 16.0), Align2::LEFT_CENTER, "VOICE AUTO-TUNE", 12.0, col::CREAM, false);
    toggle(ui, setter, &params.autotune_on, Rect::from_min_size(r.min + vec2(r.width() - 52.0, 7.0), vec2(38.0, 18.0)));
    // Pitch from the live voice (fast) or from the smoothed detector (steadier).
    let low = params.autotune_low_latency.value();
    label(p, r.min + vec2(226.0, 16.0), "LOW LAT", if low { col::CYAN } else { col::MUTED });
    toggle(ui, setter, &params.autotune_low_latency, Rect::from_min_size(r.min + vec2(272.0, 9.0), vec2(26.0, 14.0)));

    let n = shared.at_note.load(Relaxed);
    let (readout, c) = if !(on || harm) {
        ("OFF".to_string(), col::DIM)
    } else if params.monitor.value() <= 0.001 {
        ("turn up MONITOR".to_string(), col::AMBER)
    } else if n >= 0 {
        let cents = shared.at_cents.load(Relaxed);
        (format!("▶ {}  {:+.0}¢", note_name(n.clamp(0, 127) as u8), cents), col::CYAN)
    } else {
        (format!("{:.0} ms", shared.monitor_ms.load(Relaxed)), col::MUTED)
    };
    text(p, r.min + vec2(132.0, 16.0), Align2::LEFT_CENTER, readout, 10.0, c, true);

    label(p, r.min + vec2(14.0, 40.0), "SPEED", col::MUTED);
    hslider(ui, setter, &params.autotune_speed, Rect::from_min_size(r.min + vec2(56.0, 37.0), vec2(90.0, 6.0)));
    label(p, r.min + vec2(204.0, 40.0), "AMOUNT", col::MUTED);
    hslider(ui, setter, &params.autotune_amount, Rect::from_min_size(r.min + vec2(254.0, 37.0), vec2(60.0, 6.0)));

    label(p, r.min + vec2(14.0, 61.0), "HARMONY", col::MUTED);
    toggle(ui, setter, &params.harmony_on, Rect::from_min_size(r.min + vec2(66.0, 53.0), vec2(30.0, 16.0)));
    if harm {
        hslider(ui, setter, &params.harmony_level, Rect::from_min_size(r.min + vec2(106.0, 58.0), vec2(44.0, 6.0)));
    }
    // Formant: the voice's character (deeper ↔ smaller) without changing its pitch.
    let fm = params.formant.value();
    label(p, r.min + vec2(194.0, 61.0), "FORMANT", if fm.abs() > 0.05 { col::AMBER } else { col::MUTED });
    hslider(ui, setter, &params.formant, Rect::from_min_size(r.min + vec2(242.0, 58.0), vec2(54.0, 6.0)));
}
