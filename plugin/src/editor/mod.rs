//! Qimist editor. Fixed 1200×900 layout based on design/qimist-ui.html.

mod calibrate;
mod chords;
mod looper;
mod noteedit;
mod drums;
mod engine;
mod guide;
mod perform;
mod presets;
mod widgets;

use crate::params::{FilterChoice, FxTarget, KeyMode, Latency, Mode, Quantize, ScaleChoice, VelCurve, VoiceRange, VoxParams, WaveChoice};
use crate::shared::Shared;
use nih_plug::prelude::{Editor, ParamSetter};
use nih_plug_egui::egui::{self, pos2, vec2, Align2, Color32, Painter, Pos2, Rect, Shape, Stroke, Ui};
use nih_plug_egui::create_egui_editor;
use std::path::PathBuf;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use qimist_dsp::recorder::RecordedNote;
use qimist_dsp::{ScaleKind, NOTE_NAMES};
use widgets::*;

#[derive(Default)]
struct UiState {
    needle: f32,
    last_note: i32,
    last_midi: f32,
    message: String,
    last_export_dir: Option<PathBuf>,
    browser: presets::Browser,
    sound_tab: usize,
    drums: drums::DrumUi,
    pads: chords::PadUi,
    /// Centre screen: 0 = this mode's view, 1 = LOOPER.
    center_tab: usize,
    /// Piano-roll editing of the recorded take.
    take_ed: noteedit::NoteEditor,
    loop_ui: looper::LoopUi,
    calib: Option<calibrate::Calib>,
    guide: Option<guide::Guide>,
    /// When the input last had any signal (to warn when a DAW sends silence).
    last_input: f64,
}

pub fn create(params: Arc<VoxParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    create_egui_editor(
        params.editor_state.clone(),
        UiState { last_note: 69, last_midi: 69.0, guide: guide::first_run().then(guide::Guide::new), ..Default::default() },
        |ctx, _| {
            ctx.set_visuals(egui::Visuals::dark());
        },
        move |ctx, setter, state| {
            ctx.request_repaint();
            // UI scale: the window size (set by the header's UI button) drives the zoom.
            let zoom = (params.editor_state.size().0 as f32 / crate::params::EDITOR_W as f32).clamp(0.5, 2.0);
            if (ctx.zoom_factor() - zoom).abs() > 0.001 {
                ctx.set_zoom_factor(zoom);
            }
            egui::CentralPanel::default().frame(egui::Frame::default().fill(col::BG)).show(ctx, |ui| {
                draw(ui, setter, &params, &shared, state);
            });
        },
    )
}

fn draw(ui: &mut Ui, setter: &ParamSetter, params: &VoxParams, shared: &Shared, st: &mut UiState) {
    let o = Origin(ui.max_rect().min);
    let p = ui.painter().clone();

    wood(&p, o.rect(0.0, 0.0, 14.0, 900.0));
    wood(&p, o.rect(1186.0, 0.0, 14.0, 900.0));
    p.rect_filled(o.rect(14.0, 0.0_f32, 1172.0, 900.0), 0.0, col::BODY);

    header(ui, &p, o, setter, params, shared);
    if button(ui, o.rect(180.0, 14.0, 26.0, 26.0), "?", col::AMBER, st.guide.is_some()) {
        st.guide = Some(guide::Guide::new());
    }
    tuner(ui, &p, o.rect(28.0, 74.0, 400.0, 330.0), shared, st);
    center(ui, &p, o.rect(440.0, 74.0, 450.0, 330.0), setter, params, shared, st);
    key_panel(ui, &p, o.rect(902.0, 74.0, 270.0, 330.0), setter, params, shared);
    controls(ui, &p, o, setter, params, shared, st);
    perform::row(ui, &p, o, setter, params, shared);
    sound_engine(ui, &p, o, setter, params, shared, st);
    // Drawn last so the list/save overlays sit on top of everything.
    presets::bar(ui, &p, o.rect(486.0, 14.0, 360.0, 34.0), setter, params, &mut st.browser);
    if let Some(c) = st.calib.as_mut() {
        if !calibrate::show(ui.ctx(), c, setter, params, shared) {
            st.calib = None;
        }
    }
    if let Some(g) = st.guide.as_mut() {
        if !guide::show(ui.ctx(), g, o.0) {
            st.guide = None;
        }
    }
}

// ───────────────────────────── header ─────────────────────────────

fn header(ui: &mut Ui, p: &Painter, o: Origin, setter: &ParamSetter, params: &VoxParams, shared: &Shared) {
    p.rect_filled(o.rect(14.0, 0.0_f32, 1172.0, 60.0), 0.0, col::PANEL);
    p.line_segment([o.pos(14.0, 60.0), o.pos(1186.0, 60.0)], Stroke::new(1.0_f32, col::LINE));

    let r = text(p, o.pos(34.0, 26.0), Align2::LEFT_CENTER, "QI", 24.0, col::CYAN, false);
    text(p, pos2(r.max.x, r.center().y), Align2::LEFT_CENTER, "MIST", 24.0, col::CREAM, false);
    text(p, o.pos(35.0, 46.0), Align2::LEFT_CENTER, "VOICE · TO · MIDI · SYNTH", 9.0, col::MUTED, false);

    enum_pills(ui, setter, &params.mode, o.rect(220.0, 14.0, 250.0, 34.0), &[(Mode::Melody, "MELODY"), (Mode::Drums, "DRUMS"), (Mode::Chords, "CHORDS")], col::CYAN);

    // Mic level LEDs.
    label(p, o.pos(862.0, 20.0), "MIC IN", col::MUTED);
    let peak_db = 20.0 * shared.input_peak.load(Relaxed).max(1e-6).log10();
    let lit = (((peak_db + 60.0) / 60.0) * 12.0).clamp(0.0, 12.0) as usize;
    for i in 0..12 {
        let c = if i >= lit { col::KNOB } else if i < 8 { col::CYAN } else if i < 10 { col::AMBER } else { col::RED };
        p.rect_filled(o.rect(862.0 + i as f32 * 7.0, 32.0_f32, 5.0, 14.0), 1.0, c);
    }

    let bpm = shared.host_bpm.load(Relaxed);
    text(p, o.pos(1012.0, 24.0), Align2::RIGHT_CENTER, format!("{bpm:.0} BPM"), 13.0, col::CREAM, true);
    text(p, o.pos(1012.0, 42.0), Align2::RIGHT_CENTER, "HOST TEMPO", 9.0, col::MUTED, false);

    // UI scale: cycles 75 % … 150 % by resizing the window.
    let (w, _) = params.editor_state.size();
    let scale = w as f32 / crate::params::EDITOR_W as f32;
    const SCALES: [f32; 6] = [0.75, 0.9, 1.0, 1.15, 1.3, 1.5];
    let sr = o.rect(1020.0, 16.0, 58.0, 28.0);
    if button(ui, sr, &format!("UI {:.0}%", scale * 100.0), col::CYAN, false) {
        let next = SCALES.iter().copied().find(|&s| s > scale + 0.01).unwrap_or(SCALES[0]);
        params.editor_state.set_requested_size(((crate::params::EDITOR_W as f32 * next) as u32, (crate::params::EDITOR_H as f32 * next) as u32));
    }

    let midi_on = shared.sounding.load(Relaxed) >= 0;
    boxed(p, o.rect(1084.0, 16.0, 84.0, 28.0), Color32::from_rgb(0x0f, 0x0e, 0x11), col::LINE, 6.0);
    led(p, o.pos(1097.0, 30.0), midi_on, col::CYAN);
    text(p, o.pos(1108.0, 30.0), Align2::LEFT_CENTER, "MIDI OUT", 10.0, col::CREAM, false);
}

// ───────────────────────────── tuner ─────────────────────────────

fn tuner(ui: &mut Ui, p: &Painter, r: Rect, shared: &Shared, st: &mut UiState) {
    let _ = ui;
    panel(p, r, col::PANEL);
    let x = r.min.x;
    let y = r.min.y;

    let voiced = shared.voiced.load(Relaxed);
    let conf = shared.confidence.load(Relaxed).min(1.0);
    let midi = if voiced { shared.midi.load(Relaxed) } else { st.last_midi };
    if voiced {
        st.last_midi = midi;
    }
    let note = midi.round() as i32;
    let cents = (midi - note as f32) * 100.0;
    let target = if voiced { cents } else { 0.0 };
    st.needle += (target - st.needle) * 0.35;
    if voiced && note != st.last_note {
        st.last_note = note;
    }

    // Header.
    text(p, pos2(x + 14.0, y + 18.0), Align2::LEFT_CENTER, "PITCH DETECTOR", 13.0, col::CREAM, false);
    text(p, pos2(x + 196.0, y + 18.0), Align2::LEFT_CENTER, "pYIN · HMM", 10.0, col::MUTED, true);
    led(p, pos2(x + 276.0, y + 18.0), voiced, col::CYAN);
    text(p, pos2(x + 285.0, y + 18.0), Align2::LEFT_CENTER, if voiced { "VOICED" } else { "SILENT" }, 10.0, col::CREAM, false);
    let locked = voiced && conf > 0.8;
    let badge = Rect::from_min_size(pos2(x + 336.0, y + 9.0), vec2(52.0, 18.0));
    let (bc, bt) = if locked { (col::CYAN, "LOCKED") } else if voiced { (col::AMBER, "SEARCH") } else { (col::DIM, "IDLE") };
    boxed(p, badge, col::PANEL, bc, 4.0);
    text(p, badge.center(), Align2::CENTER_CENTER, bt, 10.0, bc, true);

    // Vintage needle meter.
    let mr = Rect::from_min_size(pos2(x + 14.0, y + 34.0), vec2(372.0, 124.0));
    meter(p, mr, st.needle, voiced && cents.abs() <= 5.0);

    // Note name + pitch readouts.
    let name = NOTE_NAMES[note.rem_euclid(12) as usize];
    let octave = note.div_euclid(12) - 1;
    let ny = y + 168.0;
    let nc = if voiced { col::CYAN } else { alpha(col::CYAN, 0.35) };
    let letter = &name[..1];
    let lr = text(p, pos2(x + 16.0, ny + 30.0), Align2::LEFT_CENTER, letter, 56.0, nc, true);
    if voiced {
        p.circle_filled(lr.center(), 26.0, alpha(col::CYAN, 0.05));
    }
    if name.len() > 1 {
        text(p, pos2(lr.max.x + 2.0, ny + 12.0), Align2::LEFT_CENTER, "#", 22.0, nc, true);
    }
    text(p, pos2(lr.max.x + 2.0, ny + 40.0), Align2::LEFT_CENTER, octave, 20.0, col::CREAM, true);
    let f0 = shared.f0.load(Relaxed);
    let sign = if cents >= 0.0 { "+" } else { "" };
    text(p, pos2(x + 16.0, ny + 64.0), Align2::LEFT_CENTER, format!("{f0:.1} Hz · {sign}{cents:.0}¢"), 11.0, col::CREAM, true);
    let vel = shared.velocity.load(Relaxed);
    text(p, pos2(x + 16.0, ny + 78.0), Align2::LEFT_CENTER, format!("MIDI {note} · VEL {vel}"), 10.0, col::MUTED, true);

    // Pitch trace.
    let tr = Rect::from_min_size(pos2(x + 146.0, ny), vec2(240.0, 82.0));
    trace(p, tr, shared, if voiced { note } else { st.last_note });

    // One-octave keyboard.
    keyboard(p, Rect::from_min_size(pos2(x + 14.0, y + 258.0), vec2(372.0, 26.0)), voiced.then_some(note.rem_euclid(12) as usize));

    // Meters.
    let stab = shared.stability.load(Relaxed);
    let wob = shared.wobble_cents.load(Relaxed);
    let noise = shared.noise_db.load(Relaxed);
    let row = |yy: f32| y + yy;
    label(p, pos2(x + 14.0, row(298.0)), "CONFIDENCE", col::MUTED);
    bar(p, Rect::from_min_size(pos2(x + 86.0, row(296.0)), vec2(70.0, 5.0)), conf, col::CYAN);
    text(p, pos2(x + 190.0, row(298.0)), Align2::RIGHT_CENTER, format!("{:.0}%", conf * 100.0), 11.0, col::CREAM, true);
    label(p, pos2(x + 206.0, row(298.0)), "STABILITY", col::MUTED);
    bar(p, Rect::from_min_size(pos2(x + 272.0, row(296.0)), vec2(78.0, 5.0)), stab, col::AMBER);
    text(p, pos2(x + 386.0, row(298.0)), Align2::RIGHT_CENTER, format!("{:.0}%", stab * 100.0), 11.0, col::CREAM, true);
    label(p, pos2(x + 14.0, row(316.0)), "VIBRATO", col::MUTED);
    let vib = if voiced { format!("±{wob:.0}¢") } else { "—".into() };
    text(p, pos2(x + 86.0, row(316.0)), Align2::LEFT_CENTER, vib, 11.0, col::CREAM, true);
    label(p, pos2(x + 206.0, row(316.0)), "NOISE FLOOR", col::MUTED);
    text(p, pos2(x + 290.0, row(316.0)), Align2::LEFT_CENTER, format!("{noise:.0} dB"), 11.0, col::CREAM, true);
}

/// Cream analog meter: ±50 cents over ±47.5°, pivot below the face.
fn meter(p: &Painter, r: Rect, cents: f32, in_tune: bool) {
    p.rect_filled(r.expand(3.0), 8.0_f32, col::SCREEN);
    p.rect_filled(r, 6.0_f32, col::FACE);
    // Vignette: warmer toward the edges.
    for i in 0..6 {
        let rr = r.shrink(i as f32 * 2.0);
        p.rect_stroke_compat(rr, Color32::from_rgba_unmultiplied(90, 60, 15, 10));
    }
    let s = r.width() / 392.0;
    let pivot = pos2(r.center().x, r.min.y + 206.0 * s);
    let rad = 172.0 * s;
    let at = |c: f32, rr: f32| {
        let a = (c * 0.95 - 90.0).to_radians();
        pos2(pivot.x + rr * a.cos(), pivot.y + rr * a.sin())
    };
    let arc = |c0: f32, c1: f32, rr: f32| -> Vec<Pos2> { (0..=40).map(|i| at(c0 + (c1 - c0) * i as f32 / 40.0, rr)).collect() };

    p.add(Shape::line(arc(-50.0, 50.0, rad), Stroke::new(1.2_f32, col::INK)));
    p.add(Shape::line(arc(-50.0, -30.0, rad - 6.0), Stroke::new(6.0_f32, Color32::from_rgb(0xc2, 0x41, 0x0c))));
    p.add(Shape::line(arc(30.0, 50.0, rad - 6.0), Stroke::new(6.0_f32, Color32::from_rgb(0xc2, 0x41, 0x0c))));
    p.add(Shape::line(arc(-5.0, 5.0, rad - 6.0), Stroke::new(6.0_f32, Color32::from_rgb(0x0e, 0x74, 0x90))));
    let mut c = -50.0f32;
    while c <= 50.01 {
        let major = (c % 25.0).abs() < 0.01;
        let mid = (c % 5.0).abs() < 0.01;
        let len = if major { 18.0 } else if mid { 12.0 } else { 7.0 };
        let w: f32 = if major { 2.0 } else if mid { 1.2 } else { 0.7 };
        p.line_segment([at(c, rad), at(c, rad - len * s)], Stroke::new(w, col::INK));
        if major {
            let label = if c > 0.0 { format!("+{c:.0}") } else { format!("{c:.0}") };
            text(p, at(c, rad - 30.0 * s), Align2::CENTER_CENTER, label, 11.0, col::INK, true);
        }
        c += 2.5;
    }
    text(p, pos2(r.min.x + 22.0, r.max.y - 20.0), Align2::LEFT_CENTER, "♭", 16.0, Color32::from_rgb(0x5a, 0x4f, 0x44), false);
    text(p, pos2(r.max.x - 22.0, r.max.y - 20.0), Align2::RIGHT_CENTER, "♯", 15.0, Color32::from_rgb(0x5a, 0x4f, 0x44), false);
    text(p, pos2(r.center().x, r.min.y + 86.0 * s), Align2::CENTER_CENTER, "CENTS", 9.0, Color32::from_rgb(0x5a, 0x4f, 0x44), false);
    text(p, pos2(r.center().x, r.min.y + 99.0 * s), Align2::CENTER_CENTER, "QIMIST · MODEL VT-1", 8.0, Color32::from_rgb(0x8a, 0x7a, 0x62), false);

    // Needle with shadow, then the hub covering the pivot.
    let c = cents.clamp(-55.0, 55.0);
    let tip = at(c, rad - 2.0);
    p.line_segment([pivot + vec2(2.0, 3.0), tip + vec2(2.0, 3.0)], Stroke::new(2.5_f32, Color32::from_rgba_unmultiplied(60, 40, 10, 60)));
    p.line_segment([pivot, tip], Stroke::new(2.0_f32, Color32::from_rgb(0xb9, 0x1c, 0x1c)));
    let hub = pos2(r.center().x, r.max.y);
    p.circle_filled(hub, 22.0, col::PANEL);
    p.circle_filled(hub, 15.0, col::KNOB);
    p.circle_stroke(hub, 15.0, Stroke::new(1.0_f32, col::KNOB_RIM));

    // Glass reflection.
    let glass = vec![r.left_top() + vec2(4.0, 4.0), pos2(r.min.x + r.width() * 0.55, r.min.y + 4.0), pos2(r.min.x + r.width() * 0.25, r.max.y - 30.0), pos2(r.min.x + 4.0, r.max.y - 30.0)];
    p.add(Shape::convex_polygon(glass, Color32::from_rgba_unmultiplied(255, 255, 255, 28), Stroke::NONE));

    for (dx, dy) in [(6.0, 6.0), (r.width() - 6.0, 6.0), (6.0, r.height() - 6.0), (r.width() - 6.0, r.height() - 6.0)] {
        screw(p, r.min + vec2(dx, dy));
    }
    let lamp = pos2(r.max.x - 22.0, r.max.y - 36.0);
    if in_tune {
        glow_dot(p, lamp, 5.0, col::CYAN);
    } else {
        p.circle_filled(lamp, 5.0, Color32::from_rgb(0x6b, 0x5d, 0x47));
    }
    text(p, lamp + vec2(0.0, 12.0), Align2::CENTER_CENTER, "IN TUNE", 7.0, Color32::from_rgb(0x5a, 0x4f, 0x44), false);
}

trait StrokeCompat {
    fn rect_stroke_compat(&self, r: Rect, c: Color32);
}

impl StrokeCompat for Painter {
    /// A 1px rectangle outline drawn with lines (avoids egui-version-specific APIs).
    fn rect_stroke_compat(&self, r: Rect, c: Color32) {
        let s = Stroke::new(1.0_f32, c);
        self.add(Shape::line(vec![r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()], s));
    }
}

fn trace(p: &Painter, r: Rect, shared: &Shared, center_note: i32) {
    boxed(p, r, col::SCREEN, col::LINE_SOFT, 6.0);
    let hist = shared.history_snapshot();
    let span = 2.5;
    let center = center_note as f32;
    let y = |m: f32| r.center().y - (m - center) / span * (r.height() / 2.0 - 2.0);
    for d in -2..=2 {
        let yy = y(center + d as f32);
        let c = if d == 0 { Color32::from_rgb(0x2c, 0x4a, 0x52) } else { Color32::from_rgb(0x1d, 0x1a, 0x20) };
        p.line_segment([pos2(r.min.x + 1.0, yy), pos2(r.max.x - 1.0, yy)], Stroke::new(1.0_f32, c));
        let n = center_note + d;
        let lc = if d == 0 { col::CYAN } else { col::MUTED };
        text(p, pos2(r.max.x - 4.0, yy), Align2::RIGHT_CENTER, format!("{}{}", NOTE_NAMES[n.rem_euclid(12) as usize], n.div_euclid(12) - 1), 8.0, lc, true);
    }
    text(p, r.min + vec2(6.0, 8.0), Align2::LEFT_CENTER, "PITCH · 3 S", 8.0, col::MUTED, false);
    let n = hist.len();
    let mut seg: Vec<Pos2> = Vec::new();
    let w = r.width() - 30.0;
    for (i, &m) in hist.iter().enumerate() {
        if m.is_nan() || (m - center).abs() > span + 0.5 {
            glow_line(p, std::mem::take(&mut seg), col::CYAN, 1.6);
            continue;
        }
        seg.push(pos2(r.min.x + 2.0 + w * i as f32 / (n - 1) as f32, y(m).clamp(r.min.y + 2.0, r.max.y - 2.0)));
    }
    glow_line(p, seg, col::CYAN, 1.6);
}

fn keyboard(p: &Painter, r: Rect, active: Option<usize>) {
    let whites = [0usize, 2, 4, 5, 7, 9, 11];
    let blacks = [(1usize, 0usize), (3, 1), (6, 3), (8, 4), (10, 5)];
    let kw = (r.width() - 12.0) / 7.0;
    for (i, &pc) in whites.iter().enumerate() {
        let kr = Rect::from_min_size(pos2(r.min.x + i as f32 * (kw + 2.0), r.min.y), vec2(kw, r.height()));
        let on = active == Some(pc);
        if on {
            p.rect_filled(kr.expand(3.0), 5.0_f32, alpha(col::CYAN, 0.2));
        }
        p.rect_filled(kr, 3.0_f32, if on { col::CYAN } else { Color32::from_rgb(0xd9, 0xcf, 0xb8) });
        text(p, pos2(kr.center().x, kr.max.y - 6.0), Align2::CENTER_CENTER, NOTE_NAMES[pc], 8.0, Color32::from_rgb(0x5a, 0x4f, 0x44), true);
    }
    for (pc, after) in blacks {
        let cx = r.min.x + (after + 1) as f32 * (kw + 2.0) - 1.0;
        let kr = Rect::from_center_size(pos2(cx, r.min.y + 8.0), vec2(kw * 0.55, 16.0));
        let on = active == Some(pc);
        p.rect_filled(kr, 2.0_f32, if on { col::CYAN } else { Color32::from_rgb(0x1a, 0x17, 0x1c) });
    }
}

// ───────────────────────────── center ─────────────────────────────

fn center(ui: &mut Ui, p: &Painter, r: Rect, setter: &ParamSetter, params: &VoxParams, shared: &Shared, st: &mut UiState) {
    panel(p, r, col::SCREEN);
    let x = r.min.x;
    let y = r.min.y;
    let mode = params.mode.value();
    let title = match mode {
        Mode::Melody => "CAPTURED NOTES · PIANO ROLL",
        Mode::Drums => "BEATBOX ▶ DRUM KIT",
        Mode::Chords => "CHORD PADS",
    };
    // Tabs: this mode's screen or the loop station.
    let tab_name = match mode {
        Mode::Melody => "NOTES",
        Mode::Drums => "KIT",
        Mode::Chords => "PADS",
    };
    for (i, name) in [tab_name, "LOOPER"].into_iter().enumerate() {
        if pill(ui, Rect::from_min_size(pos2(r.max.x - 160.0 + i as f32 * 74.0, y + 9.0), vec2(70.0, 22.0)), name, st.center_tab == i, if i == 1 { col::RED } else { col::CYAN }) {
            st.center_tab = i;
        }
    }
    // A running loop shows on the LOOPER tab even from the other screen.
    let loop_state = shared.loop_state.load(Relaxed);
    if st.center_tab == 0 && loop_state != 0 {
        led(p, pos2(r.max.x - 76.0, y + 20.0), loop_state != 3 || (ui.input(|i| i.time) * 3.0).fract() < 0.5, if loop_state == 3 { col::RED } else { col::CYAN });
    }
    if st.center_tab == 1 {
        text(p, pos2(x + 14.0, y + 20.0), Align2::LEFT_CENTER, "LOOP STATION", 13.0, col::CREAM, false);
        let area = Rect::from_min_size(pos2(x + 14.0, y + 40.0), vec2(r.width() - 28.0, 280.0));
        if let Some(m) = looper::view(ui, p, area, setter, params, shared, &mut st.loop_ui) {
            st.message = m;
        }
        if !st.message.is_empty() {
            let m: String = st.message.chars().take(26).collect();
            text(p, pos2(r.max.x - 168.0, y + 20.0), Align2::RIGHT_CENTER, m, 9.0, col::MUTED, true);
        }
        return;
    }
    text(p, pos2(x + 14.0, y + 20.0), Align2::LEFT_CENTER, title, 13.0, col::CREAM, false);

    // Transport buttons.
    let recording = shared.recording.load(Relaxed);
    let br = Rect::from_min_size(pos2(x + 14.0, y + 290.0), vec2(104.0, 28.0));
    if button(ui, br, if recording { "STOP REC" } else { "RECORD" }, col::RED, recording) {
        if !recording && !shared.recorder.lock().is_empty() {
            // Start a fresh take.
            shared.recorder.lock().clear();
            shared.clear_request.store(true, Relaxed);
        }
        st.take_ed = Default::default();
        shared.recording.store(!recording, Relaxed);
        st.message.clear();
    }
    if button(ui, Rect::from_min_size(pos2(x + 124.0, y + 290.0), vec2(64.0, 28.0)), "CLEAR", col::CREAM, false) {
        shared.recorder.lock().clear();
        shared.clear_request.store(true, Relaxed);
        st.take_ed = Default::default();
        st.message.clear();
    }
    if button(ui, Rect::from_min_size(pos2(x + 194.0, y + 290.0), vec2(112.0, 28.0)), "⬇ EXPORT .MID", col::CREAM, false) {
        st.message = export(params, shared, st);
    }
    if button(ui, Rect::from_min_size(pos2(x + 312.0, y + 290.0), vec2(124.0, 28.0)), "OPEN FOLDER", col::AMBER, false) {
        let dir = st.last_export_dir.clone().unwrap_or_else(export_dir);
        let _ = std::fs::create_dir_all(&dir);
        open_folder(&dir);
    }

    let roll = Rect::from_min_size(pos2(x + 14.0, y + 38.0), vec2(r.width() - 28.0, 222.0));
    match mode {
        Mode::Melody if recording || shared.recorder.lock().is_empty() => piano_roll(p, roll, shared),
        Mode::Melody => take_editor(ui, p, roll, shared, &mut st.take_ed),
        Mode::Drums => drums::pads(ui, p, roll, setter, params, shared, &mut st.drums),
        Mode::Chords => chords::pads(ui, p, roll, setter, params, shared, &mut st.pads),
    }

    let count = shared.recorder.lock().notes().len();
    let t = shared.rec_time.load(Relaxed);
    let status = if st.message.is_empty() {
        let what = if mode == Mode::Drums { "hits" } else { "notes" };
        format!("{count} {what} · {:02}:{:04.1}{}", (t / 60.0) as u32, t % 60.0, if recording { " · recording" } else { "" })
    } else {
        st.message.clone()
    };
    text(p, pos2(x + 14.0, y + 272.0), Align2::LEFT_CENTER, status, 10.0, col::MUTED, true);
    if mode == Mode::Melody && !recording && count > 0 {
        let hint = match st.take_ed.selected_count() {
            0 => "click/drag notes · dbl-click adds · Q snaps all".to_string(),
            n => format!("{n} selected · drag · arrow keys move · Del · Q snap"),
        };
        text(p, pos2(r.max.x - 14.0, y + 272.0), Align2::RIGHT_CENTER, hint, 9.0, col::DIM, false);
        if r.max.x - 14.0 - (x + 442.0) >= 108.0 {
            let b = Rect::from_min_size(pos2(r.max.x - 14.0 - 108.0, y + 290.0), vec2(52.0, 28.0));
            if button(ui, b, "SNAP", col::CYAN, false) {
                let grid = 60.0 / shared.host_bpm.load(Relaxed).clamp(20.0, 400.0) as f64 / 4.0;
                edit_take(shared, |notes| {
                    let origin = take_origin(notes);
                    st.take_ed.quantize(notes, grid, origin, None)
                });
            }
            if button(ui, b.translate(vec2(56.0, 0.0)), "UNDO", col::CREAM, false) {
                edit_take(shared, |notes| st.take_ed.undo(notes));
            }
        }
    }
}

/// Where a take's grid starts: its first note (the export puts it on beat 1).
fn take_origin(notes: &[noteedit::EdNote]) -> f64 {
    notes.iter().map(|n| n.start).reduce(f64::min).unwrap_or(0.0).max(0.0)
}

/// Run an edit on the recorded take and write it back if it changed.
fn edit_take(shared: &Shared, f: impl FnOnce(&mut Vec<noteedit::EdNote>) -> bool) {
    let mut rec = shared.recorder.lock();
    let mut notes: Vec<noteedit::EdNote> = rec
        .notes()
        .iter()
        .map(|n| noteedit::EdNote { start: n.start, len: n.end - n.start, note: n.note, vel: n.velocity, ch: n.channel, drum: -1 })
        .collect();
    if f(&mut notes) {
        let back: Vec<RecordedNote> = notes.iter().map(|n| RecordedNote { start: n.start, end: n.start + n.len, note: n.note, velocity: n.vel, channel: n.ch }).collect();
        rec.replace_notes(&back);
    }
}

/// The finished take as an editable piano roll.
fn take_editor(ui: &mut Ui, p: &Painter, r: Rect, shared: &Shared, ed: &mut noteedit::NoteEditor) {
    let beat = 60.0 / shared.host_bpm.load(Relaxed).clamp(20.0, 400.0) as f64;
    edit_take(shared, |notes| {
        let origin = take_origin(notes);
        let cfg = noteedit::EdCfg { total: 8.0, grid: beat / 4.0, beat, origin, playhead: None, channel: notes.first().map_or(0, |n| n.ch), color: col::CYAN };
        ed.show(ui, p, r, notes, &cfg, false)
    });
}

fn piano_roll(p: &Painter, r: Rect, shared: &Shared) {
    boxed(p, r, col::SCREEN, col::LINE_SOFT, 6.0);
    let recording = shared.recording.load(Relaxed);
    let now = shared.rec_time.load(Relaxed) as f64;
    let window = 8.0f64;

    let (notes, open): (Vec<_>, Vec<_>) = {
        let rec = shared.recorder.lock();
        (rec.notes().iter().map(|n| (n.start, n.end, n.note)).collect(), rec.open_notes().collect())
    };
    let end_t = if recording { now } else { notes.iter().map(|n| n.1).fold(window, f64::max) };
    let t0 = end_t - window;
    let visible: Vec<(f64, f64, u8)> = notes
        .iter()
        .copied()
        .chain(open.iter().map(|&(s, n)| (s, end_t, n)))
        .filter(|n| n.1 >= t0)
        .collect();

    let (mut lo, mut hi) = visible.iter().fold((127u8, 0u8), |(l, h), n| (l.min(n.2), h.max(n.2)));
    if lo > hi {
        lo = 60;
        hi = 72;
    }
    let pad = (13i32 - (hi as i32 - lo as i32 + 1)).max(0);
    let lo = (lo as i32 - pad / 2 - 1).max(0);
    let hi = (hi as i32 + (pad - pad / 2) + 1).min(127);
    let rows = (hi - lo + 1) as f32;
    let keys_w = 36.0;
    let grid = Rect::from_min_max(pos2(r.min.x + keys_w, r.min.y + 1.0), pos2(r.max.x - 1.0, r.max.y - 1.0));
    let rh = grid.height() / rows;

    for n in lo..=hi {
        let yy = grid.max.y - (n - lo + 1) as f32 * rh;
        let black = NOTE_NAMES[n.rem_euclid(12) as usize].len() > 1;
        p.rect_filled(Rect::from_min_size(pos2(grid.min.x, yy), vec2(grid.width(), rh)), 0.0_f32, if black { Color32::from_rgb(0x0d, 0x0c, 0x0f) } else { Color32::from_rgb(0x12, 0x10, 0x14) });
        p.rect_filled(Rect::from_min_size(pos2(r.min.x + 1.0, yy), vec2(keys_w - 2.0, rh)), 0.0_f32, if black { Color32::from_rgb(0x0f, 0x0e, 0x11) } else { col::PANEL });
        if rh >= 9.0 {
            text(p, pos2(r.min.x + 5.0, yy + rh / 2.0), Align2::LEFT_CENTER, format!("{}{}", NOTE_NAMES[n.rem_euclid(12) as usize], n.div_euclid(12) - 1), 8.0, col::MUTED, true);
        }
    }
    // One line per second.
    let mut s = t0.ceil();
    while s < end_t {
        let xx = grid.min.x + ((s - t0) / window) as f32 * grid.width();
        p.line_segment([pos2(xx, grid.min.y), pos2(xx, grid.max.y)], Stroke::new(1.0_f32, if (s as i64) % 4 == 0 { col::LINE } else { Color32::from_rgb(0x18, 0x16, 0x1b) }));
        s += 1.0;
    }
    for (s, e, n) in visible {
        let x0 = grid.min.x + (((s - t0) / window) as f32 * grid.width()).max(0.0);
        let x1 = grid.min.x + ((e - t0) / window) as f32 * grid.width();
        let yy = grid.max.y - (n as i32 - lo + 1) as f32 * rh;
        let nr = Rect::from_min_max(pos2(x0 + 0.5, yy + 1.5), pos2(x1.max(x0 + 3.0) - 0.5, yy + rh - 1.5));
        p.rect_filled(nr.expand(2.0), 4.0_f32, alpha(col::CYAN, 0.15));
        p.rect_filled(nr, 3.0_f32, col::CYAN);
    }
    if recording {
        let xx = grid.max.x - 1.0;
        p.line_segment([pos2(xx, grid.min.y), pos2(xx, grid.max.y)], Stroke::new(2.0_f32, col::AMBER));
    }
    if notes.is_empty() && open.is_empty() {
        let hint = if recording { "Sing or hum — notes appear here" } else { "Press RECORD, then sing or hum" };
        text(p, grid.center(), Align2::CENTER_CENTER, hint, 12.0, col::DIM, false);
    }
}

pub(super) fn export_dir() -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join("Music").join("Qimist")
}

fn export(params: &VoxParams, shared: &Shared, st: &mut UiState) -> String {
    let bytes = {
        let rec = shared.recorder.lock();
        if rec.notes().is_empty() {
            return "Nothing recorded yet".into();
        }
        rec.to_smf(shared.host_bpm.load(Relaxed) as f64, true, params.quantize.value().division())
    };
    let dir = export_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return format!("Export failed: {e}");
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let path = dir.join(format!("Qimist-take-{secs}.mid"));
    match std::fs::write(&path, bytes) {
        Ok(()) => {
            st.last_export_dir = Some(dir);
            format!("Saved {} — drag it into your DAW", path.display())
        }
        Err(e) => format!("Export failed: {e}"),
    }
}

fn open_folder(dir: &std::path::Path) {
    #[cfg(target_os = "linux")]
    let cmd = "xdg-open";
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "windows")]
    let cmd = "explorer";
    let _ = std::process::Command::new(cmd).arg(dir).spawn();
}

// ───────────────────────────── key & scale ─────────────────────────────

fn key_panel(ui: &mut Ui, p: &Painter, r: Rect, setter: &ParamSetter, params: &VoxParams, shared: &Shared) {
    panel(p, r, col::PANEL);
    let x = r.min.x;
    let y = r.min.y;
    text(p, pos2(x + 14.0, y + 20.0), Align2::LEFT_CENTER, "KEY & SCALE", 13.0, col::CREAM, false);
    enum_pills(ui, setter, &params.key_mode, Rect::from_min_size(pos2(x + 142.0, y + 8.0), vec2(114.0, 26.0)), &[(KeyMode::Auto, "AUTO"), (KeyMode::Lock, "LOCK")], col::AMBER);

    let lock = params.key_mode.value() == KeyMode::Lock;
    if lock {
        let root = params.key_root.value().clamp(0, 11) as usize;
        let kind = params.key_scale.value();
        text(p, pos2(x + 14.0, y + 56.0), Align2::LEFT_CENTER, format!("{} {}", NOTE_NAMES[root], kind.kind().name().to_uppercase()), 22.0, col::AMBER, true);
        stepper(ui, setter, &params.key_root, Rect::from_min_size(pos2(x + 14.0, y + 78.0), vec2(90.0, 24.0)), NOTE_NAMES[root]);
        scale_stepper(ui, setter, params, Rect::from_min_size(pos2(x + 110.0, y + 78.0), vec2(146.0, 24.0)));
    } else {
        let root = shared.key_root.load(Relaxed);
        if root >= 0 {
            let minor = shared.key_minor.load(Relaxed);
            let conf = shared.key_conf.load(Relaxed);
            let name = format!("{} {}", NOTE_NAMES[root as usize], if minor { "MINOR" } else { "MAJOR" });
            let nr = text(p, pos2(x + 14.0, y + 58.0), Align2::LEFT_CENTER, name, 26.0, col::AMBER, true);
            text(p, pos2(nr.max.x + 8.0, y + 60.0), Align2::LEFT_CENTER, format!("{:.0}%", conf * 100.0), 12.0, col::CREAM, true);
            let alts: Vec<String> = shared
                .alt
                .iter()
                .map(|(r, m, c)| format!("{} {} {:.0}%", NOTE_NAMES[r.load(Relaxed).clamp(0, 11) as usize], if m.load(Relaxed) { "MIN" } else { "MAJ" }, c.load(Relaxed) * 100.0))
                .collect();
            text(p, pos2(x + 14.0, y + 86.0), Align2::LEFT_CENTER, format!("ALT: {} · {}", alts[0], alts[1]), 10.0, col::MUTED, true);
        } else {
            text(p, pos2(x + 14.0, y + 58.0), Align2::LEFT_CENTER, "LISTENING…", 22.0, alpha(col::AMBER, 0.5), true);
            text(p, pos2(x + 14.0, y + 86.0), Align2::LEFT_CENTER, "Sing a few phrases to detect the key", 10.0, col::MUTED, false);
        }
    }

    // Pitch-class histogram, in-scale bars highlighted.
    let scale = if lock { Some(qimist_dsp::Scale::new(params.key_root.value() as u8, params.key_scale.value().kind())) } else { shared.active_scale().or_else(|| {
        let root = shared.key_root.load(Relaxed);
        (root >= 0).then(|| qimist_dsp::Scale::new(root as u8, if shared.key_minor.load(Relaxed) { ScaleKind::NaturalMinor } else { ScaleKind::Major }))
    }) };
    let hr = Rect::from_min_size(pos2(x + 14.0, y + 112.0), vec2(242.0, 150.0));
    let bw = (hr.width() - 11.0 * 4.0) / 12.0;
    for pc in 0..12 {
        let h = shared.histogram[pc].load(Relaxed).clamp(0.0, 1.0);
        let in_scale = scale.is_some_and(|s| s.contains(pc as i32));
        let tonic = scale.is_some_and(|s| s.root as usize == pc);
        let bx = hr.min.x + pc as f32 * (bw + 4.0);
        let bh = 6.0 + h * (hr.height() - 22.0);
        let c = if tonic { col::AMBER } else if in_scale { alpha(col::AMBER, 0.55) } else { col::KNOB };
        p.rect_filled(Rect::from_min_max(pos2(bx, hr.max.y - 16.0 - bh), pos2(bx + bw, hr.max.y - 16.0)), 2.0_f32, c);
        text(p, pos2(bx + bw / 2.0, hr.max.y - 6.0), Align2::CENTER_CENTER, NOTE_NAMES[pc], 9.0, if in_scale { col::CREAM } else { col::DIM }, true);
    }

    p.line_segment([pos2(x + 14.0, y + 274.0), pos2(r.max.x - 14.0, y + 274.0)], Stroke::new(1.0_f32, col::LINE));
    label(p, pos2(x + 14.0, y + 294.0), "SCALE SNAP", col::CREAM);
    toggle(ui, setter, &params.snap, Rect::from_min_size(pos2(x + 100.0, y + 282.0), vec2(44.0, 24.0)));
    if button(ui, Rect::from_min_size(pos2(x + 176.0, y + 282.0), vec2(80.0, 24.0)), "RESET KEY", col::CREAM, false) {
        shared.clear_key_request.store(true, Relaxed);
    }
    let snap_text = match (params.snap.value(), shared.active_scale()) {
        (true, Some(s)) => format!("snapping to {}", s.name()),
        (true, None) => "waiting for a confident key".to_string(),
        (false, _) => "notes are not forced to the scale".to_string(),
    };
    text(p, pos2(x + 14.0, y + 316.0), Align2::LEFT_CENTER, snap_text, 9.0, col::MUTED, false);
}

fn scale_stepper(ui: &mut Ui, setter: &ParamSetter, params: &VoxParams, r: Rect) {
    use nih_plug::prelude::{Enum, Param};
    boxed(ui.painter(), r, col::SCREEN, col::LINE, 5.0);
    let cur = params.key_scale.value();
    let n = ScaleChoice::variants().len();
    let idx = cur.to_index();
    let bw = r.height();
    for (dir, rect, glyph) in [(-1i32, Rect::from_min_size(r.min, vec2(bw, r.height())), "◀"), (1, Rect::from_min_size(pos2(r.max.x - bw, r.min.y), vec2(bw, r.height())), "▶")] {
        let resp = ui.interact(rect, ui.id().with(("scale-step", dir)), egui::Sense::click());
        text(ui.painter(), rect.center(), Align2::CENTER_CENTER, glyph, 10.0, if resp.hovered() { col::CYAN } else { col::MUTED }, false);
        if resp.clicked() {
            let next = ScaleChoice::from_index((idx as i32 + dir).rem_euclid(n as i32) as usize);
            setter.begin_set_parameter(&params.key_scale);
            setter.set_parameter(&params.key_scale, next);
            setter.end_set_parameter(&params.key_scale);
        }
    }
    let _ = params.key_scale.name();
    text(ui.painter(), r.center(), Align2::CENTER_CENTER, cur.kind().name(), 11.0, col::AMBER, true);
}

// ───────────────────────────── controls row ─────────────────────────────

fn controls(ui: &mut Ui, p: &Painter, o: Origin, setter: &ParamSetter, params: &VoxParams, shared: &Shared, st: &mut UiState) {
    let y = 416.0;
    let h = 140.0;
    let ky = y + 66.0;
    let kr = 20.0;

    // INPUT
    let r = o.rect(28.0, y, 290.0, h);
    panel(p, r, col::PANEL);
    text(p, r.min + vec2(14.0, 18.0), Align2::LEFT_CENTER, "INPUT", 12.0, col::CREAM, false);
    // ECHO: cancels Qimist's own sound coming back in from laptop speakers.
    let echo = shared.echo_db.load(Relaxed);
    label(p, r.min + vec2(150.0, 18.0), "ECHO", if echo > 3.0 { col::CYAN } else { col::MUTED });
    toggle(ui, setter, &params.echo_cancel, Rect::from_min_size(r.min + vec2(180.0, 10.0), vec2(30.0, 16.0)));
    label(p, r.min + vec2(220.0, 18.0), "HUM", col::MUTED);
    toggle(ui, setter, &params.hum_notch, Rect::from_min_size(r.min + vec2(246.0, 10.0), vec2(30.0, 16.0)));
    let voice = shared.voice_db.load(Relaxed);
    // In a DAW, pure silence for a while means the track isn't getting the mic.
    let now = ui.input(|i| i.time);
    if shared.input_peak.load(Relaxed) > 1e-6 {
        st.last_input = now;
    }
    let no_input = !shared.standalone.load(Relaxed) && now - st.last_input > 2.0;
    if no_input {
        let strip = o.rect(28.0, 61.0, 1144.0, 12.0);
        p.rect_filled(strip, 3.0, alpha(col::RED, 0.85));
        text(p, strip.center(), Align2::CENTER_CENTER, "NO SOUND IS REACHING QIMIST — set this track's audio input to your mic and turn monitoring on (Ableton: Audio From → your mic, Monitor → In)", 9.0, col::CREAM, false);
    }
    let (vt, vc) = if no_input {
        ("NO INPUT".to_string(), col::RED)
    } else if params.focus.value() <= 0.0 {
        ("FOCUS OFF".to_string(), col::DIM)
    } else if voice > -100.0 {
        (format!("YOU {voice:.0}dB"), col::CYAN)
    } else {
        ("SING!".to_string(), col::AMBER)
    };
    text(p, r.min + vec2(64.0, 18.0), Align2::LEFT_CENTER, vt, 10.0, vc, true);
    knob(ui, setter, &params.gate, o.pos(60.0, ky), 18.0, "GATE", col::CYAN);
    knob(ui, setter, &params.sensitivity, o.pos(116.0, ky), 18.0, "SENS", col::CYAN);
    knob(ui, setter, &params.denoise, o.pos(172.0, ky), 18.0, "DENOISE", col::CYAN);
    knob(ui, setter, &params.focus, o.pos(228.0, ky), 18.0, "FOCUS", col::AMBER);
    knob(ui, setter, &params.monitor, o.pos(284.0, ky), 18.0, "MONITOR", col::AMBER);

    // DETECTION
    let r = o.rect(330.0, y, 260.0, h);
    panel(p, r, col::PANEL);
    text(p, r.min + vec2(14.0, 18.0), Align2::LEFT_CENTER, "DETECTION", 12.0, col::CREAM, false);
    if button(ui, Rect::from_min_size(r.min + vec2(100.0, 7.0), vec2(96.0, 22.0)), "CALIBRATE", col::CYAN, st.calib.is_some()) && st.calib.is_none() {
        st.calib = Some(calibrate::Calib::start(ui.input(|i| i.time)));
    }
    let lat = shared.latency_ms.load(Relaxed);
    text(p, r.min + vec2(246.0, 18.0), Align2::RIGHT_CENTER, format!("{lat:.0} ms"), 10.0, col::CYAN, true);
    label(p, r.min + vec2(14.0, 44.0), "VOICE RANGE", col::MUTED);
    enum_pills(ui, setter, &params.range, Rect::from_min_size(r.min + vec2(14.0, 54.0), vec2(232.0, 28.0)), &[(VoiceRange::Low, "LOW"), (VoiceRange::Mid, "MID"), (VoiceRange::High, "HIGH")], col::CYAN);
    label(p, r.min + vec2(14.0, 96.0), "LATENCY", col::MUTED);
    enum_pills(ui, setter, &params.latency, Rect::from_min_size(r.min + vec2(14.0, 106.0), vec2(232.0, 28.0)), &[(Latency::Fast, "FAST"), (Latency::Balanced, "BALANCED"), (Latency::Accurate, "ACCURATE")], col::AMBER);

    // NOTE OUT
    let r = o.rect(602.0, y, 320.0, h);
    panel(p, r, col::PANEL);
    text(p, r.min + vec2(14.0, 18.0), Align2::LEFT_CENTER, "NOTE OUT", 12.0, col::CREAM, false);
    text(p, r.min + vec2(306.0, 18.0), Align2::RIGHT_CENTER, format!("{} NOTES SENT", shared.notes_sent.load(Relaxed)), 10.0, col::AMBER, true);
    knob(ui, setter, &params.min_note, o.pos(640.0, ky), kr, "MIN NOTE", col::CYAN);
    knob(ui, setter, &params.hold, o.pos(700.0, ky), kr, "HOLD", col::CYAN);
    knob(ui, setter, &params.hysteresis, o.pos(760.0, ky), kr, "HYSTERESIS", col::CYAN);
    knob(ui, setter, &params.octave, o.pos(820.0, ky), kr, "OCTAVE", col::AMBER);
    knob(ui, setter, &params.transpose, o.pos(880.0, ky), kr, "TRANSPOSE", col::AMBER);

    // MIDI
    let r = o.rect(934.0, y, 238.0, h);
    panel(p, r, col::PANEL);
    text(p, r.min + vec2(14.0, 18.0), Align2::LEFT_CENTER, "MIDI", 12.0, col::CREAM, false);
    label(p, r.min + vec2(128.0, 18.0), "PITCH BEND", col::MUTED);
    toggle(ui, setter, &params.pitch_bend, Rect::from_min_size(r.min + vec2(188.0, 8.0), vec2(38.0, 20.0)));
    knob(ui, setter, &params.midi_channel, o.pos(968.0, ky - 4.0), 17.0, "CHANNEL", col::CYAN);
    knob(ui, setter, &params.bend_range, o.pos(1024.0, ky - 4.0), 17.0, "BEND RNG", col::CYAN);
    knob(ui, setter, &params.quantize, o.pos(1080.0, ky - 4.0), 17.0, "EXPORT Q", col::AMBER);
    let _ = Quantize::Off;
    // Singer's loudness as a CC (for external synths' expression/filter).
    knob(ui, setter, &params.expr_cc, o.pos(1136.0, ky - 4.0), 17.0, "EXPR CC", col::AMBER);
    if params.expr_cc.value() > 0 {
        led(p, o.pos(1156.0, ky - 22.0), shared.expression.load(Relaxed) > 0.05, col::CYAN);
    }
    enum_pills(ui, setter, &params.vel_curve, Rect::from_min_size(r.min + vec2(14.0, 106.0), vec2(210.0, 26.0)), &[(VelCurve::Fixed, "FIX"), (VelCurve::Linear, "LIN"), (VelCurve::Soft, "SOFT"), (VelCurve::Hard, "HARD")], col::AMBER);
}

// ───────────────────────────── sound engine ─────────────────────────────

fn sound_engine(ui: &mut Ui, p: &Painter, o: Origin, setter: &ParamSetter, params: &VoxParams, shared: &Shared, st: &mut UiState) {
    let y = 672.0;
    label(p, o.pos(28.0, y + 8.0), "SOUND ENGINE", col::MUTED);
    for (i, name) in ["SYNTH", "EFFECTS"].into_iter().enumerate() {
        if pill(ui, o.rect(116.0 + i as f32 * 76.0, y - 2.0, 72.0, 20.0), name, st.sound_tab == i, col::CYAN) {
            st.sound_tab = i;
        }
    }
    p.line_segment([o.pos(276.0, y + 8.0), o.pos(640.0, y + 8.0)], Stroke::new(1.0_f32, col::LINE));
    label(p, o.pos(652.0, y + 8.0), "SYNTH", col::MUTED);
    toggle(ui, setter, &params.synth_on, o.rect(694.0, y - 2.0, 38.0, 20.0));
    label(p, o.pos(750.0, y + 8.0), "VOLUME", col::MUTED);
    hslider(ui, setter, &params.synth_volume, o.rect(800.0, y + 2.0, 96.0, 12.0));
    label(p, o.pos(942.0, y + 8.0), "FX TO", col::MUTED);
    enum_pills(ui, setter, &params.fx_target, o.rect(1010.0, y - 4.0, 162.0, 24.0), &[(FxTarget::Synth, "SYNTH"), (FxTarget::Voice, "VOICE"), (FxTarget::Both, "BOTH")], col::AMBER);

    let top = y + 22.0;
    let h = 196.0;

    if st.sound_tab == 1 {
    // CHORUS
    let r = o.rect(28.0, top, 120.0, h);
    panel(p, r, col::PANEL);
    text(p, r.min + vec2(14.0, 18.0), Align2::LEFT_CENTER, "CHORUS", 12.0, col::CREAM, false);
    led(p, r.min + vec2(r.width() - 16.0, 18.0), params.chorus_mix.value() > 0.0, col::AMBER);
    knob(ui, setter, &params.chorus_rate, pos2(r.min.x + 34.0, r.min.y + 62.0), 14.0, "RATE", col::CYAN);
    knob(ui, setter, &params.chorus_depth, pos2(r.min.x + 86.0, r.min.y + 62.0), 14.0, "DEPTH", col::CYAN);
    knob(ui, setter, &params.chorus_mix, pos2(r.min.x + 60.0, r.min.y + 140.0), 14.0, "MIX", col::AMBER);

    // EQ
    let r = o.rect(160.0, top, 250.0, h);
    panel(p, r, col::PANEL);
    text(p, r.min + vec2(14.0, 18.0), Align2::LEFT_CENTER, "EQ · 5 BAND", 12.0, col::CREAM, false);
    led(p, r.min + vec2(r.width() - 16.0, 18.0), params.eq_gains().iter().any(|g| *g != 0.0), col::AMBER);
    eq_curve(p, Rect::from_min_size(r.min + vec2(14.0, 34.0), vec2(222.0, 70.0)), params.eq_gains());
    let eq: [(&nih_plug::prelude::FloatParam, &str); 5] =
        [(&params.eq_low, "LOW"), (&params.eq_250, "250"), (&params.eq_1k, "1K"), (&params.eq_4k, "4K"), (&params.eq_high, "HIGH")];
    for (i, (param, name)) in eq.iter().enumerate() {
        knob(ui, setter, *param, pos2(r.min.x + 33.0 + i as f32 * 46.0, r.min.y + 138.0), 12.0, name, col::CYAN);
    }

    engine::delay_panel(ui, p, o.rect(422.0, top, 250.0, h), setter, params, shared);

    // REVERB
    let r = o.rect(684.0, top, 196.0, h);
    panel(p, r, col::PANEL);
    text(p, r.min + vec2(14.0, 18.0), Align2::LEFT_CENTER, "REVERB", 12.0, col::CREAM, false);
    led(p, r.min + vec2(r.width() - 16.0, 18.0), params.rv_mix.value() > 0.0, col::AMBER);
    let cx = [r.min.x + 38.0, r.min.x + 98.0, r.min.x + 158.0];
    knob(ui, setter, &params.rv_size, pos2(cx[0], r.min.y + 62.0), 14.0, "SIZE", col::CYAN);
    knob(ui, setter, &params.rv_decay, pos2(cx[1], r.min.y + 62.0), 14.0, "DECAY", col::CYAN);
    knob(ui, setter, &params.rv_damp, pos2(cx[2], r.min.y + 62.0), 14.0, "DAMP", col::CYAN);
    knob(ui, setter, &params.rv_predelay, pos2(cx[0], r.min.y + 140.0), 14.0, "PRE-DLY", col::CYAN);
    knob(ui, setter, &params.rv_width, pos2(cx[1], r.min.y + 140.0), 14.0, "WIDTH", col::CYAN);
    knob(ui, setter, &params.rv_mix, pos2(cx[2], r.min.y + 140.0), 14.0, "MIX", col::AMBER);

    engine::master_panel(ui, p, o.rect(892.0, top, 280.0, h), setter, params, shared);
        return;
    }

    // OSCILLATORS (the drum kit takes this slot in drums mode)
    let r = o.rect(28.0, top, 300.0, h);
    if params.mode.value() == Mode::Drums {
        drums::kit_panel(ui, p, r, setter, params);
    } else {
    panel(p, r, col::PANEL);
    text(p, r.min + vec2(14.0, 18.0), Align2::LEFT_CENTER, "OSCILLATORS", 12.0, col::CREAM, false);
    for (i, osc) in params.oscs.iter().enumerate() {
        let ry = r.min.y + 38.0 + i as f32 * 52.0;
        text(p, pos2(r.min.x + 14.0, ry + 14.0), Align2::LEFT_CENTER, i + 1, 12.0, col::CYAN, true);
        wave_pills(ui, setter, &osc.wave, Rect::from_min_size(pos2(r.min.x + 28.0, ry + 3.0), vec2(132.0, 24.0)), i);
        knob(ui, setter, &osc.level, pos2(r.min.x + 186.0, ry + 12.0), 10.0, "LEVEL", col::CYAN);
        knob(ui, setter, &osc.octave, pos2(r.min.x + 230.0, ry + 12.0), 10.0, "OCT", col::AMBER);
        knob(ui, setter, &osc.detune, pos2(r.min.x + 274.0, ry + 12.0), 10.0, "DETUNE", col::CYAN);
    }
    }

    // FILTER + ENV
    let r = o.rect(340.0, top, 230.0, h);
    panel(p, r, col::PANEL);
    text(p, r.min + vec2(14.0, 18.0), Align2::LEFT_CENTER, "FILTER", 12.0, col::CREAM, false);
    enum_pills(
        ui,
        setter,
        &params.filter_mode,
        Rect::from_min_size(r.min + vec2(62.0, 8.0), vec2(r.width() - 72.0, 20.0)),
        &[(FilterChoice::Lp12, "LP12"), (FilterChoice::Lp24, "LP24"), (FilterChoice::Bp, "BP"), (FilterChoice::Hp, "HP")],
        col::AMBER,
    );
    let cols = [r.min.x + 34.0, r.min.x + 88.0, r.min.x + 142.0, r.min.x + 196.0];
    knob(ui, setter, &params.cutoff, pos2(cols[0], r.min.y + 62.0), 14.0, "CUTOFF", col::CYAN);
    knob(ui, setter, &params.resonance, pos2(cols[1], r.min.y + 62.0), 14.0, "RESO", col::CYAN);
    knob(ui, setter, &params.env_amount, pos2(cols[2], r.min.y + 62.0), 14.0, "ENV AMT", col::AMBER);
    knob(ui, setter, &params.glide, pos2(cols[3], r.min.y + 62.0), 14.0, "GLIDE", col::CYAN);
    knob(ui, setter, &params.attack, pos2(cols[0], r.min.y + 140.0), 14.0, "ATTACK", col::AMBER);
    knob(ui, setter, &params.decay, pos2(cols[1], r.min.y + 140.0), 14.0, "DECAY", col::AMBER);
    knob(ui, setter, &params.sustain, pos2(cols[2], r.min.y + 140.0), 14.0, "SUSTAIN", col::AMBER);
    knob(ui, setter, &params.release, pos2(cols[3], r.min.y + 140.0), 14.0, "RELEASE", col::AMBER);

    engine::unison_panel(ui, p, o.rect(582.0, top, 160.0, h), setter, params);
    engine::dynamics_meter(p, o.rect(582.0, top, 160.0, h), params, shared);
    engine::lfo_panel(ui, p, o.rect(754.0, top, 250.0, h), setter, params);
    engine::master_panel(ui, p, o.rect(1016.0, top, 156.0, h), setter, params, shared);
}

/// Five waveform buttons drawn as little oscilloscope glyphs.
fn wave_pills(ui: &mut Ui, setter: &ParamSetter, param: &nih_plug::prelude::EnumParam<WaveChoice>, r: Rect, osc: usize) {
    let waves = [WaveChoice::Sine, WaveChoice::Triangle, WaveChoice::Saw, WaveChoice::Square, WaveChoice::Noise];
    let w = (r.width() - 4.0 * 2.0) / 5.0;
    let current = param.value();
    for (i, wave) in waves.iter().enumerate() {
        let br = Rect::from_min_size(pos2(r.min.x + i as f32 * (w + 2.0), r.min.y), vec2(w, r.height()));
        let resp = ui.interact(br, ui.id().with(("wave", osc, i)), egui::Sense::click());
        let on = current == *wave;
        if resp.clicked() && !on {
            setter.begin_set_parameter(param);
            setter.set_parameter(param, *wave);
            setter.end_set_parameter(param);
        }
        let c = if on { col::CYAN } else if resp.hovered() { col::CREAM } else { col::MUTED };
        boxed(p_of(ui), br, if on { alpha(col::CYAN, 0.1) } else { Color32::from_rgb(0x0f, 0x0e, 0x11) }, if on { col::CYAN } else { col::LINE }, 4.0);
        let gx = |t: f32| br.min.x + 4.0 + t * (br.width() - 8.0);
        let gy = |v: f32| br.center().y - v * 5.0;
        let pts: Vec<Pos2> = match wave {
            WaveChoice::Sine => (0..=16).map(|k| { let t = k as f32 / 16.0; pos2(gx(t), gy((t * std::f32::consts::TAU).sin())) }).collect(),
            WaveChoice::Triangle => vec![pos2(gx(0.0), gy(0.0)), pos2(gx(0.25), gy(1.0)), pos2(gx(0.75), gy(-1.0)), pos2(gx(1.0), gy(0.0))],
            WaveChoice::Saw => vec![pos2(gx(0.0), gy(-1.0)), pos2(gx(0.5), gy(1.0)), pos2(gx(0.5), gy(-1.0)), pos2(gx(1.0), gy(1.0))],
            WaveChoice::Square => vec![pos2(gx(0.0), gy(1.0)), pos2(gx(0.5), gy(1.0)), pos2(gx(0.5), gy(-1.0)), pos2(gx(1.0), gy(-1.0))],
            WaveChoice::Noise => [0.2, -0.8, 0.6, -0.3, 0.9, -1.0, 0.4, -0.5, 0.7].iter().enumerate().map(|(k, v)| pos2(gx(k as f32 / 8.0), gy(*v))).collect(),
        };
        p_of(ui).add(Shape::line(pts, Stroke::new(1.4_f32, c)));
    }
}

fn p_of(ui: &Ui) -> &Painter {
    ui.painter()
}

/// Horizontal fader for a 0..1 parameter.
pub(super) fn hslider(ui: &mut Ui, setter: &ParamSetter, param: &nih_plug::prelude::FloatParam, r: Rect) {
    use nih_plug::prelude::Param;
    let resp = ui.interact(r.expand(4.0), ui.id().with(("hslider", param.name())), egui::Sense::click_and_drag());
    if resp.drag_started() {
        setter.begin_set_parameter(param);
    }
    if resp.dragged() || resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            if resp.clicked() {
                setter.begin_set_parameter(param);
            }
            setter.set_parameter_normalized(param, ((pos.x - r.min.x) / r.width()).clamp(0.0, 1.0));
            if resp.clicked() {
                setter.end_set_parameter(param);
            }
        }
    }
    if resp.drag_stopped() {
        setter.end_set_parameter(param);
    }
    let v = param.unmodulated_normalized_value();
    let p = ui.painter();
    bar(p, Rect::from_center_size(r.center(), vec2(r.width(), 5.0)), v, col::CYAN);
    glow_dot(p, pos2(r.min.x + r.width() * v, r.center().y), 5.0, col::CYAN);
    text(p, pos2(r.max.x + 8.0, r.center().y), Align2::LEFT_CENTER, param.normalized_value_to_string(v, true), 10.0, col::CREAM, true);
}

/// Combined magnitude response of the five EQ bands.
fn eq_curve(p: &Painter, r: Rect, gains: [f32; 5]) {
    boxed(p, r, col::SCREEN, col::LINE_SOFT, 5.0);
    let sr = 48_000.0;
    let coeffs: Vec<_> = (0..5).map(|b| qimist_dsp::fx::eq_coeffs(sr, b, gains[b])).collect();
    let fx = |f: f32| r.min.x + 2.0 + (f / 20.0).log10() / 3.0 * (r.width() - 4.0);
    let gy = |db: f32| r.center().y - db / 15.0 * (r.height() / 2.0 - 4.0);
    p.line_segment([pos2(r.min.x + 2.0, gy(0.0)), pos2(r.max.x - 2.0, gy(0.0))], Stroke::new(1.0_f32, col::LINE));
    for f in [100.0f32, 1000.0, 10_000.0] {
        p.line_segment([pos2(fx(f), r.min.y + 2.0), pos2(fx(f), r.max.y - 2.0)], Stroke::new(1.0_f32, Color32::from_rgb(0x18, 0x16, 0x1b)));
    }
    let pts: Vec<Pos2> = (0..=90)
        .map(|i| {
            let f = 20.0 * 1000f32.powf(i as f32 / 90.0);
            let db: f64 = coeffs.iter().map(|c| c.magnitude_db(sr, f as f64)).sum();
            pos2(fx(f), gy(db as f32).clamp(r.min.y + 2.0, r.max.y - 2.0))
        })
        .collect();
    glow_line(p, pts, col::CYAN, 1.8);
    for (b, &f) in qimist_dsp::fx::EQ_FREQS.iter().enumerate() {
        let db: f64 = coeffs.iter().map(|c| c.magnitude_db(sr, f as f64)).sum();
        p.circle_filled(pos2(fx(f), gy(db as f32).clamp(r.min.y + 4.0, r.max.y - 4.0)), 3.5, if gains[b] != 0.0 { col::AMBER } else { col::DIM });
    }
}
