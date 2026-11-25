//! CHORD PADS: the centre screen in chords mode. Seven pads for the chords of
//! the key light up as you sing; click and hold one to play it yourself.
//! Below them: how chords follow the voice and when they change.

use super::hslider;
use super::widgets::*;
use crate::params::{ChordFollowChoice, ChordSyncChoice, VoxParams};
use crate::shared::Shared;
use nih_plug::prelude::ParamSetter;
use nih_plug_egui::egui::{pos2, vec2, Align2, Color32, Painter, Rect, Sense, Ui};
use qimist_dsp::follow::{has_degrees, pad_root, roman};
use qimist_dsp::{note_name, NOTE_NAMES};
use std::sync::atomic::Ordering::Relaxed;

/// Pad the mouse is holding, so we only clear what we set.
#[derive(Default)]
pub struct PadUi {
    held: Option<usize>,
}

pub fn pads(ui: &mut Ui, p: &Painter, r: Rect, setter: &ParamSetter, params: &VoxParams, shared: &Shared, st: &mut PadUi) {
    boxed(p, r, col::SCREEN, col::LINE_SOFT, 6.0);
    let scale = shared.active_scale();
    let t = ui.input(|i| i.time);

    // Header: key and what you're singing.
    let key = match scale {
        Some(s) => format!("KEY  {}", s.name().to_uppercase()),
        None => "KEY  —".to_string(),
    };
    text(p, r.min + vec2(12.0, 14.0), Align2::LEFT_CENTER, key, 11.0, if scale.is_some() { col::AMBER } else { col::DIM }, true);
    let sung = shared.sounding.load(Relaxed);
    let root = shared.chord_root.load(Relaxed);
    let right = if sung >= 0 {
        format!("singing {}", note_name(sung as u8))
    } else if root >= 0 {
        "ringing…".to_string()
    } else {
        "sing, or hold a pad".to_string()
    };
    text(p, pos2(r.max.x - 12.0, r.min.y + 14.0), Align2::RIGHT_CENTER, right, 10.0, if sung >= 0 { col::CYAN } else { col::MUTED }, true);

    let gap = 6.0;
    let pad_w = (r.width() - 24.0 - gap * 6.0) / 7.0;
    let pads_r = Rect::from_min_size(r.min + vec2(12.0, 28.0), vec2(r.width() - 24.0, 104.0));
    let pending = shared.chord_pending.load(Relaxed);

    let mut down = None;
    match scale.filter(|_| has_degrees(scale)) {
        Some(sc) => {
            for d in 0..7 {
                let pr = Rect::from_min_size(pos2(pads_r.min.x + d as f32 * (pad_w + gap), pads_r.min.y), vec2(pad_w, pads_r.height()));
                let resp = ui.interact(pr, ui.id().with(("chordpad", d)), Sense::click_and_drag());
                if resp.is_pointer_button_down_on() {
                    down = Some(d);
                }
                let proot = pad_root(sc, d);
                let chord = params.chord_for(proot, scale);
                let lit = root >= 0 && root as u8 % 12 == proot % 12;
                let waiting = pending >= 0 && pending as u8 % 12 == proot % 12;
                let (fill, edge) = if lit {
                    (alpha(col::AMBER, 0.22), col::AMBER)
                } else if waiting && (t * 4.0).fract() < 0.5 {
                    (alpha(col::CYAN, 0.10), col::CYAN)
                } else if resp.hovered() {
                    (Color32::from_rgb(0x1f, 0x1c, 0x22), col::CREAM)
                } else {
                    (Color32::from_rgb(0x12, 0x11, 0x14), col::LINE)
                };
                boxed(p, pr, fill, edge, 6.0);
                let ink = if lit { col::AMBER } else { col::CREAM };
                text(p, pos2(pr.center().x, pr.min.y + 26.0), Align2::CENTER_CENTER, roman(d, &chord), 18.0, ink, false);
                text(p, pos2(pr.center().x, pr.min.y + 56.0), Align2::CENTER_CENTER, chord.name(), 12.0, if lit { col::AMBER } else { col::CYAN }, true);
                let tones: Vec<&str> = chord.notes().iter().map(|&n| NOTE_NAMES[(n % 12) as usize]).collect();
                text(p, pos2(pr.center().x, pr.min.y + 80.0), Align2::CENTER_CENTER, tones.join(" "), 8.5, col::MUTED, true);
                if lit {
                    p.rect_filled(Rect::from_min_size(pos2(pr.min.x + 8.0, pr.max.y - 9.0), vec2(pr.width() - 16.0, 3.0)), 1.5, col::AMBER);
                }
            }
        }
        None => {
            boxed(p, pads_r, Color32::from_rgb(0x12, 0x11, 0x14), col::LINE, 6.0);
            let msg = if scale.is_some() { "This scale has no 7 chords — pick a 7-note scale for pads" } else { "Sing a few phrases to find the key, or LOCK one (top right)" };
            text(p, pads_r.center() - vec2(0.0, 8.0), Align2::CENTER_CENTER, msg, 11.0, col::MUTED, false);
            text(p, pads_r.center() + vec2(0.0, 12.0), Align2::CENTER_CENTER, "the chord pads fill in once there is a key", 10.0, col::DIM, false);
        }
    }
    // Tell the audio thread which pad is held (only touch it when it changes).
    if down != st.held {
        shared.chord_pad.store(down.map_or(-1, |d| d as i32), Relaxed);
        st.held = down;
    }

    // How chords follow the voice.
    let y = r.min.y + 146.0;
    label(p, pos2(r.min.x + 12.0, y + 11.0), "FOLLOW", col::MUTED);
    enum_pills(ui, setter, &params.chord_follow, Rect::from_min_size(pos2(r.min.x + 62.0, y), vec2(150.0, 22.0)), &[(ChordFollowChoice::Melody, "MELODY"), (ChordFollowChoice::Root, "ROOT")], col::AMBER);
    label(p, pos2(r.min.x + 226.0, y + 11.0), "CHANGE", col::MUTED);
    enum_pills(ui, setter, &params.chord_sync, Rect::from_min_size(pos2(r.min.x + 276.0, y), vec2(r.width() - 288.0, 22.0)), &[(ChordSyncChoice::Free, "NOW"), (ChordSyncChoice::Beat, "BEAT"), (ChordSyncChoice::Bar, "BAR")], col::CYAN);

    let y2 = y + 34.0;
    label(p, pos2(r.min.x + 12.0, y2 + 4.0), "STEADY", col::MUTED);
    hslider(ui, setter, &params.chord_hold, Rect::from_min_size(pos2(r.min.x + 62.0, y2 + 1.0), vec2(100.0, 6.0)));
    label(p, pos2(r.min.x + 226.0, y2 + 4.0), "LET RING", col::MUTED);
    toggle(ui, setter, &params.chord_ring, Rect::from_min_size(pos2(r.min.x + 276.0, y2 - 4.0), vec2(30.0, 16.0)));

    let follow = match params.chord_follow.value() {
        ChordFollowChoice::Melody => "MELODY: sing a tune — Qimist picks chords from the key that fit it",
        ChordFollowChoice::Root => "ROOT: the note you sing is the bottom of the chord",
    };
    text(p, pos2(r.min.x + 12.0, y2 + 26.0), Align2::LEFT_CENTER, follow, 9.5, col::DIM, false);
}
