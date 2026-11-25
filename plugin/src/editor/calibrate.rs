//! Voice calibration wizard: listens to the room, then to the singer's lowest
//! and highest notes, and sets GATE, VOICE RANGE and (in noisy rooms) FOCUS and
//! DENOISE from what it heard. Runs entirely on the GUI side using the
//! detector values the audio thread already publishes.

use super::widgets::*;
use crate::params::{VoiceRange, VoxParams};
use crate::shared::Shared;
use nih_plug::prelude::{Param, ParamSetter};
use nih_plug_egui::egui::{self, pos2, vec2, Align2, Color32, RichText, Sense, Stroke};
use std::sync::atomic::Ordering::Relaxed;

const STAGES: [(&str, &str, f64); 3] = [
    ("1 / 3 · ROOM", "Stay quiet for a moment — measuring your room's noise…", 3.0),
    ("2 / 3 · LOW", "Sing or hum your LOWEST comfortable note and hold it", 4.0),
    ("3 / 3 · HIGH", "Now your HIGHEST comfortable note — hold it", 4.0),
];

pub struct Calib {
    stage: usize,
    t0: f64,
    noise: Vec<f32>,
    low: Vec<f32>,
    high: Vec<f32>,
    levels: Vec<f32>,
    result: Vec<(String, Color32)>,
}

impl Calib {
    pub fn start(now: f64) -> Self {
        Calib { stage: 0, t0: now, noise: vec![], low: vec![], high: vec![], levels: vec![], result: vec![] }
    }
}

fn percentile(v: &[f32], q: f32) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(f32::total_cmp);
    Some(s[((s.len() - 1) as f32 * q).round() as usize])
}

fn set<P: Param>(setter: &ParamSetter, p: &P, v: P::Plain) {
    setter.begin_set_parameter(p);
    setter.set_parameter(p, v);
    setter.end_set_parameter(p);
}

/// MIDI span each voice-range setting covers.
fn range_span(r: VoiceRange) -> (f32, f32) {
    match r {
        VoiceRange::Low => (31.4, 76.9),
        VoiceRange::Mid => (36.0, 84.8),
        VoiceRange::High => (45.0, 90.2),
    }
}

fn finish(c: &mut Calib, setter: &ParamSetter, params: &VoxParams) {
    let mut out = vec![];
    let noise = percentile(&c.noise, 0.5).unwrap_or(-70.0);
    let voice = percentile(&c.levels, 0.5);
    let gate = params.gate.value();
    out.push((format!("Room noise {noise:.0} dB → GATE {gate:.0} dB"), col::CYAN));

    match (percentile(&c.low, 0.1), percentile(&c.high, 0.9)) {
        (Some(lo), Some(hi)) if c.low.len() > 10 && c.high.len() > 10 => {
            let fits = |r: VoiceRange| {
                let (a, b) = range_span(r);
                lo - 2.0 >= a && hi + 2.0 <= b
            };
            let pick = [VoiceRange::Mid, VoiceRange::Low, VoiceRange::High].into_iter().find(|&r| fits(r)).unwrap_or(if lo < 40.0 { VoiceRange::Low } else { VoiceRange::High });
            set(setter, &params.range, pick);
            let name = match pick {
                VoiceRange::Low => "LOW",
                VoiceRange::Mid => "MID",
                VoiceRange::High => "HIGH",
            };
            let (lo_n, hi_n) = (qimist_dsp::note_name(lo.round() as u8), qimist_dsp::note_name(hi.round() as u8));
            out.push((format!("Your range {lo_n} – {hi_n} → VOICE RANGE {name}"), col::CYAN));
        }
        _ => out.push(("Didn't hear enough singing — range unchanged (try again a bit louder)".into(), col::AMBER)),
    }

    // Noisy room: lean on the crowd/noise tools.
    if noise > -52.0 {
        if params.focus.value() < 0.5 {
            set(setter, &params.focus, 0.5);
        }
        if params.denoise.value() < 0.8 {
            set(setter, &params.denoise, 0.8);
        }
        out.push(("Noisy room → FOCUS 50%, DENOISE 80% (a headset mic helps most)".into(), col::AMBER));
    }
    if let Some(v) = voice {
        let margin = v - noise;
        let (msg, c) = if margin > 30.0 {
            ("excellent", col::CYAN)
        } else if margin > 18.0 {
            ("good", col::CYAN)
        } else {
            ("low — move closer to the mic", col::AMBER)
        };
        out.push((format!("Voice {v:.0} dB, {margin:.0} dB above the room: {msg}"), c));
    }
    c.result = out;
}

/// Draw the wizard; returns `false` once the user closes it.
pub fn show(ctx: &egui::Context, c: &mut Calib, setter: &ParamSetter, params: &VoxParams, shared: &Shared) -> bool {
    let now = ctx.input(|i| i.time);
    let rms = shared.rms_db.load(Relaxed);
    let voiced = shared.voiced.load(Relaxed);
    let midi = shared.midi.load(Relaxed);

    if c.stage < STAGES.len() {
        match c.stage {
            0 => c.noise.push(rms),
            1 if voiced => {
                c.low.push(midi);
                c.levels.push(rms);
            }
            2 if voiced => {
                c.high.push(midi);
                c.levels.push(rms);
            }
            _ => {}
        }
        if now - c.t0 >= STAGES[c.stage].2 {
            if c.stage == 0 {
                // Gate goes in right away so the singing stages already use it.
                let noise = percentile(&c.noise, 0.5).unwrap_or(-70.0);
                set(setter, &params.gate, (noise + 9.0).clamp(-75.0, -30.0));
            }
            c.stage += 1;
            c.t0 = now;
            if c.stage == STAGES.len() {
                finish(c, setter, params);
            }
        }
    }

    let mut open = true;
    let screen = ctx.screen_rect();
    let size = vec2(460.0, 230.0);
    egui::Area::new(egui::Id::new("vf-calibrate"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.center() - size / 2.0)
        .show(ctx, |ui| {
            egui::Frame::default().fill(col::PANEL).stroke(Stroke::new(1.0_f32, col::CYAN)).corner_radius(10.0).inner_margin(18.0).show(ui, |ui| {
                ui.set_width(size.x - 36.0);
                ui.set_min_height(size.y - 36.0);
                ui.label(RichText::new("VOICE CALIBRATION").color(col::CREAM).size(14.0));
                ui.add_space(6.0);
                if c.stage < STAGES.len() {
                    let (step, msg, dur) = STAGES[c.stage];
                    ui.label(RichText::new(step).color(col::AMBER).size(10.0).monospace());
                    ui.label(RichText::new(msg).color(col::CREAM).size(13.0));
                    ui.add_space(10.0);
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::hover());
                    bar(ui.painter(), r, ((now - c.t0) / dur).clamp(0.0, 1.0) as f32, col::CYAN);
                    ui.add_space(8.0);
                    // Live input level + what's being heard.
                    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
                    let p = ui.painter();
                    let lvl = ((rms + 80.0) / 70.0).clamp(0.0, 1.0);
                    bar(p, egui::Rect::from_min_size(r.min, vec2(200.0, 6.0)), lvl, if c.stage == 0 { col::MUTED } else { col::AMBER });
                    let heard = if voiced { format!("hearing {}", qimist_dsp::note_name(midi.round().clamp(0.0, 127.0) as u8)) } else { format!("{rms:.0} dB") };
                    text(p, pos2(r.min.x + 214.0, r.min.y + 3.0), Align2::LEFT_CENTER, heard, 11.0, if voiced { col::CYAN } else { col::MUTED }, true);
                    ui.add_space(12.0);
                    if ui.button("CANCEL").clicked() {
                        open = false;
                    }
                } else {
                    for (line, color) in &c.result {
                        ui.label(RichText::new(format!("•  {line}")).color(*color).size(12.0));
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("DONE").color(col::CYAN)).clicked() {
                            open = false;
                        }
                        if ui.button("AGAIN").clicked() {
                            *c = Calib::start(now);
                        }
                    });
                }
            });
        });
    open
}
