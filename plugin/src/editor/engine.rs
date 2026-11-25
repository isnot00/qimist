//! Sound-engine panels added in phase 6: unison/drive, LFO, delay, master.

use super::widgets::*;
use crate::params::{DelayDiv, LfoShapeChoice, LfoTargetChoice, VoxParams};
use crate::shared::Shared;
use nih_plug::prelude::ParamSetter;
use nih_plug_egui::egui::{pos2, vec2, Align2, Color32, Painter, Rect, Ui};
use std::sync::atomic::Ordering::Relaxed;

pub fn unison_panel(ui: &mut Ui, p: &Painter, r: Rect, setter: &ParamSetter, params: &VoxParams) {
    panel(p, r, col::PANEL);
    text(p, r.min + vec2(14.0, 18.0), Align2::LEFT_CENTER, "UNISON · DRIVE", 12.0, col::CREAM, false);
    let n = params.unison.value();
    let shown = if n == 1 { "MONO".to_string() } else { format!("{n} VOICES") };
    stepper(ui, setter, &params.unison, Rect::from_min_size(r.min + vec2(14.0, 32.0), vec2(r.width() - 28.0, 22.0)), &shown);
    knob(ui, setter, &params.unison_detune, pos2(r.min.x + r.width() * 0.28, r.min.y + 88.0), 14.0, "DETUNE", col::CYAN);
    knob(ui, setter, &params.unison_width, pos2(r.min.x + r.width() * 0.72, r.min.y + 88.0), 14.0, "WIDTH", col::CYAN);
    knob(ui, setter, &params.drive, pos2(r.min.x + r.width() * 0.28, r.min.y + 150.0), 14.0, "DRIVE", col::AMBER);
    knob(ui, setter, &params.voice_dynamics, pos2(r.min.x + r.width() * 0.72, r.min.y + 150.0), 14.0, "DYNAMICS", col::AMBER);
}

/// Live loudness bar under the DYNAMICS knob.
pub fn dynamics_meter(p: &Painter, r: Rect, params: &VoxParams, shared: &Shared) {
    if params.voice_dynamics.value() <= 0.0 {
        return;
    }
    let m = Rect::from_min_size(pos2(r.min.x + r.width() * 0.72 - 20.0, r.max.y - 10.0), vec2(40.0, 3.0));
    bar(p, m, shared.expression.load(Relaxed), col::AMBER);
}

pub fn lfo_panel(ui: &mut Ui, p: &Painter, r: Rect, setter: &ParamSetter, params: &VoxParams) {
    panel(p, r, col::PANEL);
    text(p, r.min + vec2(14.0, 18.0), Align2::LEFT_CENTER, "LFO", 12.0, col::CREAM, false);
    // The LED pulses at the LFO rate when it's doing something.
    let depth = params.lfo_depth.value();
    let t = ui.input(|i| i.time);
    let blink = depth > 0.0 && ((t * params.lfo_rate.value() as f64).fract() < 0.5);
    led(p, r.min + vec2(r.width() - 16.0, 18.0), blink, col::CYAN);
    let w = r.width() - 28.0;
    enum_pills(
        ui,
        setter,
        &params.lfo_shape,
        Rect::from_min_size(r.min + vec2(14.0, 32.0), vec2(w, 22.0)),
        &[(LfoShapeChoice::Sine, "SINE"), (LfoShapeChoice::Triangle, "TRI"), (LfoShapeChoice::Square, "SQR"), (LfoShapeChoice::SampleHold, "S&H")],
        col::CYAN,
    );
    label(p, r.min + vec2(14.0, 70.0), "TARGET", col::MUTED);
    enum_pills(
        ui,
        setter,
        &params.lfo_target,
        Rect::from_min_size(r.min + vec2(64.0, 60.0), vec2(w - 50.0, 22.0)),
        &[(LfoTargetChoice::Pitch, "PITCH"), (LfoTargetChoice::Filter, "FILTER"), (LfoTargetChoice::Amp, "AMP")],
        col::AMBER,
    );
    knob(ui, setter, &params.lfo_rate, pos2(r.min.x + r.width() * 0.3, r.min.y + 128.0), 15.0, "RATE", col::CYAN);
    knob(ui, setter, &params.lfo_depth, pos2(r.min.x + r.width() * 0.7, r.min.y + 128.0), 15.0, "DEPTH", col::AMBER);
}

pub fn delay_panel(ui: &mut Ui, p: &Painter, r: Rect, setter: &ParamSetter, params: &VoxParams, shared: &Shared) {
    panel(p, r, col::PANEL);
    text(p, r.min + vec2(14.0, 18.0), Align2::LEFT_CENTER, "DELAY", 12.0, col::CREAM, false);
    led(p, r.min + vec2(64.0, 18.0), params.delay_mix.value() > 0.0, col::AMBER);
    label(p, r.min + vec2(r.width() - 98.0, 18.0), "PING-PONG", col::MUTED);
    toggle(ui, setter, &params.delay_ping_pong, Rect::from_min_size(r.min + vec2(r.width() - 44.0, 10.0), vec2(30.0, 16.0)));
    enum_pills(
        ui,
        setter,
        &params.delay_div,
        Rect::from_min_size(r.min + vec2(14.0, 32.0), vec2(r.width() - 28.0, 22.0)),
        &[
            (DelayDiv::Q16, "1/16"),
            (DelayDiv::Q8T, "1/8T"),
            (DelayDiv::Q8, "1/8"),
            (DelayDiv::Q4T, "1/4T"),
            (DelayDiv::Q8D, "1/8."),
            (DelayDiv::Q4, "1/4"),
            (DelayDiv::Q2, "1/2"),
        ],
        col::CYAN,
    );
    let ky = r.min.y + 104.0;
    knob(ui, setter, &params.delay_feedback, pos2(r.min.x + r.width() * 0.2, ky), 15.0, "FEEDBACK", col::CYAN);
    knob(ui, setter, &params.delay_tone, pos2(r.min.x + r.width() * 0.5, ky), 15.0, "TONE", col::CYAN);
    knob(ui, setter, &params.delay_mix, pos2(r.min.x + r.width() * 0.8, ky), 15.0, "MIX", col::AMBER);
    let bpm = shared.host_bpm.load(Relaxed);
    let ms = params.delay_settings(bpm).time_s * 1000.0;
    text(p, pos2(r.min.x + 14.0, r.max.y - 18.0), Align2::LEFT_CENTER, format!("{ms:.0} ms @ {bpm:.0} BPM · synced to tempo"), 9.0, col::MUTED, true);
}

pub fn master_panel(ui: &mut Ui, p: &Painter, r: Rect, setter: &ParamSetter, params: &VoxParams, shared: &Shared) {
    panel(p, r, col::PANEL);
    text(p, r.min + vec2(14.0, 18.0), Align2::LEFT_CENTER, "MASTER", 12.0, col::CREAM, false);
    let gr = shared.limiter_gr.load(Relaxed);
    led(p, r.min + vec2(r.width() - 16.0, 18.0), gr > 0.5, col::RED);
    knob(ui, setter, &params.output_gain, pos2(r.min.x + 44.0, r.min.y + 96.0), 18.0, "OUTPUT", col::AMBER);

    // Vertical meters: output level and limiter gain reduction.
    let mh = r.height() - 70.0;
    let out_db = 20.0 * shared.out_peak.load(Relaxed).max(1e-6).log10();
    let meters = [("OUT", ((out_db + 48.0) / 48.0).clamp(0.0, 1.0), col::CYAN), ("LIMIT", (gr / 12.0).clamp(0.0, 1.0), col::RED)];
    for (k, (name, frac, c)) in meters.iter().enumerate() {
        let x = r.min.x + 92.0 + k as f32 * 30.0;
        let m = Rect::from_min_size(pos2(x, r.min.y + 36.0), vec2(10.0, mh));
        boxed(p, m, col::SCREEN, col::LINE, 3.0);
        let fill = if k == 0 {
            Rect::from_min_max(pos2(m.min.x + 1.0, m.max.y - 1.0 - (mh - 2.0) * frac), pos2(m.max.x - 1.0, m.max.y - 1.0))
        } else {
            Rect::from_min_max(pos2(m.min.x + 1.0, m.min.y + 1.0), pos2(m.max.x - 1.0, m.min.y + 1.0 + (mh - 2.0) * frac))
        };
        let c = if k == 0 && out_db > -1.0 { col::AMBER } else { *c };
        p.rect_filled(fill, 2.0, c);
        text(p, pos2(m.center().x, m.max.y + 9.0), Align2::CENTER_CENTER, *name, 8.0, col::MUTED, false);
    }
    if r.width() > 200.0 {
        let tx = r.min.x + 160.0;
        text(p, pos2(tx, r.min.y + 52.0), Align2::LEFT_CENTER, format!("OUT {out_db:>5.1} dB"), 10.0, col::CREAM, true);
        text(p, pos2(tx, r.min.y + 70.0), Align2::LEFT_CENTER, format!("LIM {:>5.1} dB", -gr), 10.0, if gr > 0.5 { col::RED } else { col::MUTED }, true);
        text(p, pos2(tx, r.min.y + 96.0), Align2::LEFT_CENTER, "ceiling −0.3 dB", 9.0, col::DIM, false);
        text(p, pos2(tx, r.min.y + 110.0), Align2::LEFT_CENTER, "never clips", 9.0, col::DIM, false);
    }
    let _ = Color32::TRANSPARENT;
}
