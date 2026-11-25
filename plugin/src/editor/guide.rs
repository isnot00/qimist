//! First-run guide: a short spotlight tour of the screen. Opens by itself the
//! first time Qimist runs (a marker file in the config folder remembers it was
//! seen) and again from the "?" button in the header.

use super::widgets::*;
use nih_plug_egui::egui::{self, pos2, vec2, Align2, Color32, Pos2, Rect, Sense, Stroke};
use std::path::PathBuf;

/// (x, y, w, h) in editor coordinates (None = no spotlight), title, text.
type Step = (Option<(f32, f32, f32, f32)>, &'static str, &'static str);

const STEPS: [Step; 10] = [
    (None, "WELCOME TO QIMIST", "Qimist turns your voice into music. Sing, hum or beatbox into the mic and it plays a synth, sends MIDI to your DAW, and records what you did.\n\nThis tour takes a minute. Tip: headphones give the cleanest results."),
    (Some((220.0, 14.0, 250.0, 34.0)), "1 · PICK A MODE", "MELODY: your voice plays notes.\nDRUMS: beatbox kick, snare and hat.\nCHORDS: sing a tune and Qimist plays chords that fit it."),
    (Some((28.0, 74.0, 400.0, 330.0)), "2 · WHAT YOU SING", "The meter shows the note you're singing and how in tune you are. The strip below it is your pitch over the last few seconds."),
    (Some((440.0, 74.0, 450.0, 330.0)), "3 · RECORD, EDIT, LOOP", "Press RECORD, sing, then STOP. Click and drag notes to fix them, then EXPORT .MID.\n\nThe LOOPER tab layers up to four loops; click a lane to edit it."),
    (Some((902.0, 74.0, 270.0, 330.0)), "4 · KEY & SCALE", "Qimist finds the key you're singing in. LOCK it when you know the key, and turn on SCALE SNAP to keep every note in it."),
    (Some((28.0, 416.0, 290.0, 140.0)), "5 · YOUR MIC", "The level should say YOU when you sing. If it picks up noise, raise GATE. ECHO removes the sound from your speakers when you aren't on headphones."),
    (Some((330.0, 416.0, 260.0, 140.0)), "6 · CALIBRATE", "Press CALIBRATE once: stay quiet for a moment, then sing low and high. Qimist sets the noise gate and voice range for you."),
    (Some((28.0, 568.0, 1144.0, 98.0)), "7 · PERFORMANCE", "CHORDS under your melody, an ARPEGGIATOR, and VOICE AUTO-TUNE that corrects your own voice live (turn on MONITOR in INPUT to hear it)."),
    (Some((28.0, 672.0, 1144.0, 214.0)), "8 · THE SOUND", "Shape the synth here: three oscillators, filter, envelopes and LFO. The EFFECTS tab has chorus, delay, EQ and reverb."),
    (Some((486.0, 14.0, 360.0, 34.0)), "9 · PRESETS", "Browse ready-made sounds with the arrows or open the list. SAVE keeps your own.\n\nThe ? button at the top left opens this tour again any time."),
];

fn marker() -> Option<PathBuf> {
    crate::presets::user_dir().and_then(|d| d.parent().map(|p| p.join("guide-seen")))
}

/// True the first time Qimist runs on this computer.
pub fn first_run() -> bool {
    marker().is_some_and(|m| !m.exists())
}

fn mark_seen() {
    if let Some(m) = marker() {
        if let Some(d) = m.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(m, b"1");
    }
}

pub struct Guide {
    step: usize,
}

impl Guide {
    pub fn new() -> Self {
        Guide { step: 0 }
    }
}

/// Draw the tour over everything. Returns false once it's closed.
pub fn show(ctx: &egui::Context, g: &mut Guide, origin: Pos2) -> bool {
    let screen = ctx.screen_rect();
    let mut open = true;
    egui::Area::new(egui::Id::new("qimist-guide")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        // Swallow clicks so the controls underneath don't react.
        let _ = ui.allocate_rect(screen, Sense::click_and_drag());
        let p = ui.painter();
        let (spot, title, body) = STEPS[g.step];
        let shade = Color32::from_rgba_unmultiplied(4, 3, 6, 215);
        let spot = spot.map(|(x, y, w, h)| Rect::from_min_size(origin + vec2(x, y), vec2(w, h)).expand(6.0));
        match spot {
            Some(s) => {
                // Darken around the spotlight.
                p.rect_filled(Rect::from_min_max(screen.min, pos2(screen.max.x, s.min.y)), 0.0, shade);
                p.rect_filled(Rect::from_min_max(pos2(screen.min.x, s.max.y), screen.max), 0.0, shade);
                p.rect_filled(Rect::from_min_max(pos2(screen.min.x, s.min.y), pos2(s.min.x, s.max.y)), 0.0, shade);
                p.rect_filled(Rect::from_min_max(pos2(s.max.x, s.min.y), pos2(screen.max.x, s.max.y)), 0.0, shade);
                p.rect_stroke(s, 8.0, Stroke::new(2.0_f32, col::AMBER), egui::StrokeKind::Outside);
            }
            None => {
                p.rect_filled(screen, 0.0, shade);
            }
        }

        // The card: under the spotlight if there's room, else above, else centred.
        let size = vec2(400.0, 190.0);
        let card_min = match spot {
            Some(s) if s.max.y + 12.0 + size.y < screen.max.y => pos2(s.center().x - size.x / 2.0, s.max.y + 12.0),
            Some(s) if s.min.y - 12.0 - size.y > screen.min.y => pos2(s.center().x - size.x / 2.0, s.min.y - 12.0 - size.y),
            _ => screen.center() - size / 2.0,
        };
        let card_min = pos2(card_min.x.clamp(screen.min.x + 12.0, screen.max.x - size.x - 12.0), card_min.y);
        let card = Rect::from_min_size(card_min, size);
        p.rect_filled(card.expand(1.0), 11.0, Color32::from_rgba_unmultiplied(0, 0, 0, 120));
        boxed(p, card, col::PANEL, col::AMBER, 10.0);
        text(p, card.min + vec2(18.0, 22.0), Align2::LEFT_CENTER, title, 13.0, col::AMBER, true);
        text(p, pos2(card.max.x - 18.0, card.min.y + 22.0), Align2::RIGHT_CENTER, format!("{}/{}", g.step + 1, STEPS.len()), 10.0, col::DIM, true);
        let galley = p.layout(body.to_string(), font(12.0, false), col::CREAM, size.x - 36.0);
        p.galley(card.min + vec2(18.0, 42.0), galley, col::CREAM);

        // Buttons.
        let by = card.max.y - 38.0;
        let last = g.step + 1 == STEPS.len();
        if button(ui, Rect::from_min_size(pos2(card.max.x - 98.0, by), vec2(80.0, 26.0)), if last { "DONE" } else { "NEXT ▶" }, col::CYAN, true) {
            if last {
                open = false;
            } else {
                g.step += 1;
            }
        }
        if g.step > 0 && button(ui, Rect::from_min_size(pos2(card.max.x - 184.0, by), vec2(80.0, 26.0)), "◀ BACK", col::CREAM, false) {
            g.step -= 1;
        }
        if !last && button(ui, Rect::from_min_size(pos2(card.min.x + 18.0, by), vec2(90.0, 26.0)), "SKIP TOUR", col::MUTED, false) {
            open = false;
        }
        let keys = ui.input(|i| (i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::ArrowLeft), i.key_pressed(egui::Key::Escape)));
        if keys.0 {
            if last {
                open = false;
            } else {
                g.step += 1;
            }
        }
        if keys.1 && g.step > 0 {
            g.step -= 1;
        }
        if keys.2 {
            open = false;
        }
    });
    if !open {
        mark_seen();
    }
    open
}
