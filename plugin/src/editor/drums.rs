//! Drums mode: three flashing pads (training, MIDI note, custom sample), a hit
//! timeline + hit map, and the drum-kit panel that replaces the oscillators.

use super::widgets::*;
use crate::params::{KitChoice, VoxParams};
use crate::samples;
use crate::shared::{Shared, TRAIN_HITS};
use nih_plug::prelude::ParamSetter;
use nih_plug_egui::egui::{self, pos2, vec2, Align2, Color32, Painter, Rect, Stroke, Ui};
use std::path::PathBuf;
use std::sync::atomic::Ordering::Relaxed;
use qimist_dsp::drums::{brightness_voicing, DrumClass};

const TIMELINE_S: f32 = 4.0;

/// Editor-side drum state: the sample-folder listing, refreshed every 2 s.
#[derive(Default)]
pub struct DrumUi {
    files: Vec<PathBuf>,
    listed_at: f64,
}

impl DrumUi {
    fn files(&mut self, now: f64) -> &[PathBuf] {
        if self.listed_at == 0.0 || now - self.listed_at > 2.0 {
            self.files = samples::list();
            self.listed_at = now.max(1e-3);
        }
        &self.files
    }
}

fn class_color(c: DrumClass) -> Color32 {
    match c {
        DrumClass::Kick => col::AMBER,
        DrumClass::Snare => col::RED,
        DrumClass::HiHat => col::CYAN,
    }
}

fn hint(c: DrumClass) -> &'static str {
    match c {
        DrumClass::Kick => "say \"boom\" / \"b\"",
        DrumClass::Snare => "say \"pf\" / \"k\"",
        DrumClass::HiHat => "say \"ts\" / \"t\"",
    }
}

/// ◀ name ▶ selector over "built-in" + the WAVs in the samples folder.
fn sample_picker(ui: &mut Ui, r: Rect, pad: usize, params: &VoxParams, shared: &Shared, st: &mut DrumUi) {
    let p = ui.painter().clone();
    boxed(&p, r, col::SCREEN, col::LINE, 4.0);
    let current = params.drum_samples.read()[pad].clone();
    let now = ui.input(|i| i.time);
    let files = st.files(now).to_vec();
    let idx = files.iter().position(|f| f.to_string_lossy() == current).map(|i| i + 1).unwrap_or(0);
    let n = files.len() + 1;
    let bw = r.height();
    for (dir, rect, glyph) in [(-1i32, Rect::from_min_size(r.min, vec2(bw, r.height())), "◀"), (1, Rect::from_min_size(pos2(r.max.x - bw, r.min.y), vec2(bw, r.height())), "▶")] {
        let resp = ui.interact(rect, ui.id().with(("sample-step", pad, dir)), egui::Sense::click());
        text(&p, rect.center(), Align2::CENTER_CENTER, glyph, 9.0, if resp.hovered() { col::CYAN } else { col::MUTED }, false);
        if resp.clicked() {
            let next = (idx as i32 + dir).rem_euclid(n as i32) as usize;
            samples::set(params, shared, pad, if next == 0 { None } else { Some(&files[next - 1]) });
        }
    }
    let mut name = samples::display_name(&current);
    if name.chars().count() > 13 {
        name = name.chars().take(12).collect::<String>() + "…";
    }
    let c = if current.is_empty() { col::MUTED } else { col::CREAM };
    let resp = ui.interact(r.shrink2(vec2(bw, 0.0)), ui.id().with(("sample-name", pad)), egui::Sense::hover());
    text(&p, r.center(), Align2::CENTER_CENTER, name, 9.0, c, true);
    resp.on_hover_text(if files.is_empty() {
        format!("Put .wav files in {} to use your own sounds", samples::samples_dir().display())
    } else {
        format!("Sound for this pad: built-in or one of {} WAVs in the samples folder", files.len())
    });
}

pub fn pads(ui: &mut Ui, p: &Painter, r: Rect, setter: &ParamSetter, params: &VoxParams, shared: &Shared, st: &mut DrumUi) {
    boxed(p, r, col::SCREEN, col::LINE_SOFT, 6.0);
    let now = shared.clock.load(Relaxed);
    let training = shared.train_class.load(Relaxed);
    let counts = params.drum_model.try_read().map(|m| DrumClass::ALL.map(|c| m.count(c)));
    let status = shared.sample_status.lock().clone();
    let notes = [&params.kick_note, &params.snare_note, &params.hat_note];

    let gap = 10.0;
    let pw = (r.width() - 20.0 - 2.0 * gap) / 3.0;
    for (i, c) in DrumClass::ALL.into_iter().enumerate() {
        let pr = Rect::from_min_size(pos2(r.min.x + 10.0 + i as f32 * (pw + gap), r.min.y + 8.0), vec2(pw, 142.0));
        let color = class_color(c);
        let age = now - shared.last_hit[i].load(Relaxed);
        let flash = if age >= 0.0 { (-age / 0.14).exp() } else { 0.0 };
        let vel = shared.last_vel[i].load(Relaxed) as f32 / 127.0;
        let is_training = training == i as i32;

        // Pad body with a glow when hit.
        if flash > 0.02 {
            p.rect_filled(pr.expand(4.0 * flash), 9.0, alpha(color, 0.18 * flash));
        }
        let fill = Color32::from_rgb(0x14, 0x12, 0x16).lerp_to_gamma(alpha(color, 1.0), 0.35 * flash);
        let border = if is_training { col::CREAM } else { alpha(color, 0.35 + 0.65 * flash) };
        boxed(p, pr, fill, border, 7.0);

        text(p, pr.center_top() + vec2(0.0, 16.0), Align2::CENTER_CENTER, c.name(), 15.0, if flash > 0.3 { col::CREAM } else { color }, false);
        text(p, pr.center_top() + vec2(0.0, 32.0), Align2::CENTER_CENTER, hint(c), 9.0, col::MUTED, false);

        // MIDI note for this pad, then its sound.
        let note = notes[i].value();
        let nr = Rect::from_min_size(pos2(pr.min.x + 8.0, pr.min.y + 42.0), vec2(pw - 16.0, 18.0));
        stepper(ui, setter, notes[i], nr, &format!("{note} {}", qimist_dsp::note_name(note.clamp(0, 127) as u8)));
        sample_picker(ui, Rect::from_min_size(pos2(pr.min.x + 8.0, pr.min.y + 64.0), vec2(pw - 16.0, 18.0)), i, params, shared, st);

        // Velocity of the last hit, fading out.
        let vb = Rect::from_min_size(pos2(pr.min.x + 12.0, pr.min.y + 88.0), vec2(pw - 24.0, 4.0));
        bar(p, vb, vel * flash.max(0.15), color);

        let n = counts.map(|c| c[i]).unwrap_or(0);
        let (info, ic) = if !status[i].is_empty() {
            (status[i].clone(), col::RED)
        } else if is_training {
            (format!("{} MORE…", shared.train_left.load(Relaxed)), col::CREAM)
        } else if n > 0 {
            (format!("TRAINED ×{n}"), color)
        } else {
            ("BUILT-IN RULES".to_string(), col::DIM)
        };
        text(p, pr.center_top() + vec2(0.0, 101.0), Align2::CENTER_CENTER, info, 8.0, ic, true);

        let br = Rect::from_min_size(pos2(pr.min.x + 10.0, pr.max.y - 30.0), vec2(pw - 20.0, 22.0));
        let label = if is_training { "LISTENING…" } else { "TRAIN" };
        if button(ui, br, label, color, is_training) {
            if is_training {
                shared.train_class.store(-1, Relaxed);
            } else {
                shared.train_left.store(TRAIN_HITS, Relaxed);
                shared.train_class.store(i as i32, Relaxed);
            }
        }
    }

    // Hit timeline (three lanes, newest on the right) + hit map.
    let split = r.min.x + 10.0 + (r.width() - 20.0) * 0.6;
    let tl = Rect::from_min_max(pos2(r.min.x + 10.0, r.min.y + 158.0), pos2(split - 4.0, r.max.y - 6.0));
    hit_map(p, Rect::from_min_max(pos2(split + 4.0, r.min.y + 158.0), pos2(r.max.x - 10.0, r.max.y - 6.0)), params, shared, now);
    boxed(p, tl, Color32::from_rgb(0x0d, 0x0c, 0x0f), col::LINE_SOFT, 5.0);
    let lanes_x = tl.min.x + 46.0;
    let lane_h = tl.height() / 3.0;
    for (i, c) in DrumClass::ALL.into_iter().enumerate() {
        let yy = tl.min.y + (i as f32 + 0.5) * lane_h;
        text(p, pos2(tl.min.x + 6.0, yy), Align2::LEFT_CENTER, c.name(), 7.0, col::MUTED, true);
        p.line_segment([pos2(lanes_x, yy), pos2(tl.max.x - 6.0, yy)], Stroke::new(1.0_f32, col::LINE_SOFT));
    }
    let w = tl.max.x - 6.0 - lanes_x;
    let mut g = (now / 0.5).floor() * 0.5;
    while g > now - TIMELINE_S {
        let xx = lanes_x + (1.0 - (now - g) / TIMELINE_S) * w;
        p.line_segment([pos2(xx, tl.min.y + 3.0), pos2(xx, tl.max.y - 3.0)], Stroke::new(1.0_f32, Color32::from_rgb(0x18, 0x16, 0x1b)));
        g -= 0.5;
    }
    let mut any = false;
    for (t, c) in shared.hits() {
        let age = now - t;
        if !(0.0..TIMELINE_S).contains(&age) {
            continue;
        }
        any = true;
        let xx = lanes_x + (1.0 - age / TIMELINE_S) * w;
        let yy = tl.min.y + (c as f32 + 0.5) * lane_h;
        glow_dot(p, pos2(xx, yy), 3.5, class_color(DrumClass::from_index(c)));
    }
    if !any {
        let msg = if training >= 0 { "Make the sound, pause, repeat" } else { "Beatbox: boom · ts · pf" };
        text(p, pos2(lanes_x + w / 2.0, tl.center().y), Align2::CENTER_CENTER, msg, 10.0, col::DIM, false);
    }
}

/// Where each hit landed: brightness (x, log Hz) vs voicing (y). Rings are
/// your trained examples, dots the recent hits. If two sounds overlap here,
/// make them more different (e.g. a deeper "boom", a sharper "pf").
fn hit_map(p: &Painter, r: Rect, params: &VoxParams, shared: &Shared, now: f32) {
    boxed(p, r, Color32::from_rgb(0x0d, 0x0c, 0x0f), col::LINE_SOFT, 5.0);
    let plot = r.shrink2(vec2(8.0, 6.0));
    let (lo, hi) = (200f32.log2(), 16_000f32.log2());
    let at = |hz: f32, v: f32| {
        let x = ((hz.max(1.0).log2() - lo) / (hi - lo)).clamp(0.0, 1.0);
        pos2(plot.min.x + x * plot.width(), plot.max.y - v.clamp(0.0, 1.0) * plot.height())
    };
    for hz in [1000.0, 5000.0] {
        let x = at(hz, 0.0).x;
        p.line_segment([pos2(x, plot.min.y), pos2(x, plot.max.y)], Stroke::new(1.0_f32, col::LINE_SOFT));
    }
    text(p, pos2(plot.min.x, plot.min.y + 3.0), Align2::LEFT_CENTER, "VOICED", 7.0, col::DIM, true);
    text(p, pos2(plot.max.x, plot.max.y - 3.0), Align2::RIGHT_CENTER, "BRIGHT ▶", 7.0, col::DIM, true);

    if let Some(m) = params.drum_model.try_read() {
        for (c, f) in &m.examples {
            let (b, v) = brightness_voicing(f);
            p.circle_stroke(at(b, v), 3.0, Stroke::new(1.0_f32, alpha(class_color(*c), 0.6)));
        }
    }
    let mut newest: Option<(f32, usize, f32, f32)> = None;
    for h in shared.hits_detailed() {
        let age = now - h.0;
        if !(0.0..8.0).contains(&age) {
            continue;
        }
        let c = class_color(DrumClass::from_index(h.1));
        p.circle_filled(at(h.2, h.3), 2.5, alpha(c, 1.0 - age / 8.0));
        if newest.is_none_or(|n| h.0 > n.0) {
            newest = Some(h);
        }
    }
    if let Some((t, c, b, v)) = newest {
        if now - t < 2.0 {
            glow_dot(p, at(b, v), 4.0, class_color(DrumClass::from_index(c)));
        }
    }
}

pub fn kit_panel(ui: &mut Ui, p: &Painter, r: Rect, setter: &ParamSetter, params: &VoxParams) {
    panel(p, r, col::PANEL);
    text(p, r.min + vec2(14.0, 18.0), Align2::LEFT_CENTER, "DRUM KIT", 12.0, col::CREAM, false);
    let ch = params.drum_channel.value();
    label(p, r.min + vec2(r.width() - 130.0, 18.0), "MIDI CH", col::MUTED);
    stepper(ui, setter, &params.drum_channel, Rect::from_min_size(r.min + vec2(r.width() - 84.0, 8.0), vec2(70.0, 20.0)), &ch.to_string());
    enum_pills(
        ui,
        setter,
        &params.drum_kit,
        Rect::from_min_size(r.min + vec2(14.0, 34.0), vec2(r.width() - 28.0, 24.0)),
        &[(KitChoice::Eight08, "808"), (KitChoice::Tight, "TIGHT"), (KitChoice::LoFi, "LO-FI"), (KitChoice::Acoustic, "ACOUS"), (KitChoice::Trap, "TRAP")],
        col::AMBER,
    );
    let ky = r.min.y + 84.0;
    knob(ui, setter, &params.drum_volume, pos2(r.min.x + 42.0, ky), 13.0, "VOLUME", col::AMBER);
    knob(ui, setter, &params.drum_sens, pos2(r.min.x + 113.0, ky), 13.0, "HIT SENS", col::CYAN);
    knob(ui, setter, &params.drum_tune, pos2(r.min.x + 184.0, ky), 13.0, "TUNE", col::CYAN);
    knob(ui, setter, &params.drum_decay, pos2(r.min.x + 255.0, ky), 13.0, "DECAY", col::CYAN);

    let ly = r.min.y + 148.0;
    knob(ui, setter, &params.kick_level, pos2(r.min.x + 36.0, ly), 11.0, "KICK", col::AMBER);
    knob(ui, setter, &params.snare_level, pos2(r.min.x + 88.0, ly), 11.0, "SNARE", col::RED);
    knob(ui, setter, &params.hat_level, pos2(r.min.x + 140.0, ly), 11.0, "HAT", col::CYAN);

    if button(ui, Rect::from_min_size(pos2(r.max.x - 110.0, r.min.y + 128.0), vec2(96.0, 22.0)), "SAMPLES DIR", col::CREAM, false) {
        let dir = samples::samples_dir();
        let _ = std::fs::create_dir_all(&dir);
        super::open_folder(&dir);
    }
    if button(ui, Rect::from_min_size(pos2(r.max.x - 110.0, r.min.y + 158.0), vec2(96.0, 22.0)), "RESET TRAIN", col::RED, false) {
        params.drum_model.write().clear();
    }
}
