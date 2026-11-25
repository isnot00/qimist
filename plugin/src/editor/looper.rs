//! LOOPER screen: four layers that loop over 1–8 bars. Press REC on a layer
//! and sing, beatbox or play chords for one loop; it keeps playing while you
//! add the next layer on top. Click a lane to fix its notes in the piano roll.
//! Scenes A–D hold four different loops (verse, chorus…); SONG chains them
//! into an arrangement that plays by itself and exports as one file.

use super::noteedit::{EdCfg, EdNote, NoteEditor};
use super::widgets::*;
use crate::params::{LoopBars, LoopQuant, VoxParams};
use crate::shared::Shared;
use nih_plug::prelude::ParamSetter;
use nih_plug_egui::egui::{pos2, vec2, Align2, Color32, CursorIcon, Painter, Rect, Sense, Stroke, Ui};
use qimist_dsp::looper::{LoopCmd, LoopData, LoopLayer, LoopNote, Song, SongData, SongStep, LAYERS, SCENES, SONG_MAX};
use qimist_dsp::recorder::{encode_smf, RecordedNote};
use std::path::PathBuf;
use std::sync::atomic::Ordering::Relaxed;

const SCENE_NAMES: [&str; SCENES] = ["A", "B", "C", "D"];
const COLORS: [Color32; LAYERS] = [col::CYAN, col::AMBER, Color32::from_rgb(0xff, 0x7a, 0xc8), Color32::from_rgb(0x8c, 0xf2, 0x8a)];

/// Which layer is open in the piano roll, and the editor's working copy.
#[derive(Default)]
pub struct LoopUi {
    editing: Option<usize>,
    ed: NoteEditor,
    notes: Vec<EdNote>,
    /// Showing the song arrangement instead of the layers.
    song_view: bool,
}

fn to_ed(layer: &LoopLayer) -> Vec<EdNote> {
    layer.notes.iter().map(|n| EdNote { start: n.start, len: n.len, note: n.note, vel: n.vel, ch: n.ch, drum: n.drum }).collect()
}

/// Hand the edited layer to the audio thread (sorted, as the looper needs).
fn push_edit(shared: &Shared, scene: usize, layer: usize, notes: &[EdNote]) {
    let mut edit = shared.loop_edit.lock();
    edit.scene = scene;
    edit.layer = layer;
    edit.notes.clear();
    edit.notes.extend(notes.iter().map(|n| LoopNote { start: n.start, len: n.len, note: n.note, vel: n.vel, ch: n.ch, drum: n.drum }));
    edit.notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.note.cmp(&b.note)));
    edit.pending = true;
}

fn send(shared: &Shared, cmd: LoopCmd) {
    shared.loop_cmds.lock().push(cmd);
}

/// Returns a status message when something was exported.
pub fn view(ui: &mut Ui, p: &Painter, r: Rect, setter: &ParamSetter, params: &VoxParams, shared: &Shared, st: &mut LoopUi) -> Option<String> {
    let scene = (shared.loop_scene.load(Relaxed).clamp(0, SCENES as i32 - 1)) as usize;
    let song = params.song.read().clone();
    let data = song.scenes[scene].clone();
    let state = shared.loop_state.load(Relaxed);
    let rec_layer = shared.loop_layer.load(Relaxed);
    let pos = shared.loop_pos.load(Relaxed) as f64;
    let beats = data.beats;
    let t = ui.input(|i| i.time);
    let blink = (t * 3.0).fract() < 0.5;
    let mut message = None;

    // ── Row 1: transport, length, tempo, metronome ──
    let y = r.min.y;
    let running = state != 0;
    if button(ui, Rect::from_min_size(pos2(r.min.x, y), vec2(70.0, 24.0)), if running { "STOP" } else { "▶ PLAY" }, col::CYAN, running) {
        send(shared, if running { LoopCmd::Stop } else { LoopCmd::Play });
    }
    let locked = !data.is_empty();
    enum_pills(
        ui,
        setter,
        &params.loop_bars,
        Rect::from_min_size(pos2(r.min.x + 80.0, y), vec2(150.0, 24.0)),
        &[(LoopBars::B1, "1"), (LoopBars::B2, "2"), (LoopBars::B4, "4"), (LoopBars::B8, "8")],
        if locked { col::DIM } else { col::AMBER },
    );
    text(p, pos2(r.min.x + 155.0, y + 32.0), Align2::CENTER_CENTER, if locked { "BARS · clear all to change" } else { "BARS" }, 8.5, col::DIM, false);
    let bpm = shared.host_bpm.load(Relaxed);
    let br = Rect::from_min_size(pos2(r.min.x + 240.0, y), vec2(92.0, 24.0));
    if shared.standalone.load(Relaxed) {
        stepper(ui, setter, &params.tempo, br, &format!("{bpm:.0}"));
        text(p, pos2(br.center().x, y + 32.0), Align2::CENTER_CENTER, "BPM", 8.5, col::DIM, false);
    } else {
        boxed(p, br, col::SCREEN, col::LINE, 5.0);
        text(p, br.center(), Align2::CENTER_CENTER, format!("{bpm:.0} BPM"), 11.0, col::CYAN, true);
        text(p, pos2(br.center().x, y + 32.0), Align2::CENTER_CENTER, "HOST TEMPO", 8.5, col::DIM, false);
    }
    label(p, pos2(r.max.x - 64.0, y + 12.0), "CLICK", col::MUTED);
    toggle(ui, setter, &params.loop_metronome, Rect::from_min_size(pos2(r.max.x - 30.0, y + 4.0), vec2(30.0, 16.0)));

    // ── Row 2: scenes and the song ──
    let sy = y + 42.0;
    label(p, pos2(r.min.x, sy + 11.0), "SCENE", col::MUTED);
    let queued = shared.loop_next_scene.load(Relaxed);
    let song_step = shared.loop_song_step.load(Relaxed);
    for i in 0..SCENES {
        let sr = Rect::from_min_size(pos2(r.min.x + 44.0 + i as f32 * 34.0, sy), vec2(30.0, 22.0));
        let waiting = queued == i as i32;
        let lit = i == scene || (waiting && blink);
        if pill(ui, sr, SCENE_NAMES[i], lit, if waiting { col::AMBER } else { col::CYAN }) {
            send(shared, LoopCmd::Scene(i));
        }
        // A dot under scenes that have something in them.
        if !song.scenes[i].is_empty() {
            p.circle_filled(pos2(sr.center().x, sr.max.y + 3.0), 1.5, col::MUTED);
        }
    }
    let cr = Rect::from_min_size(pos2(r.min.x + 182.0, sy), vec2(74.0, 22.0));
    if button(ui, cr, &format!("COPY {} ▶", SCENE_NAMES[scene]), col::CREAM, false) {
        message = Some(match (0..SCENES).find(|&i| i != scene && song.scenes[i].is_empty()) {
            Some(to) if !data.is_empty() => {
                send(shared, LoopCmd::CopyScene { from: scene, to });
                format!("copied {} to {}", SCENE_NAMES[scene], SCENE_NAMES[to])
            }
            Some(_) => "this scene is empty".to_string(),
            None => "no empty scene — clear one first".to_string(),
        });
    }
    let song_r = Rect::from_min_size(pos2(r.max.x - 96.0, sy), vec2(96.0, 22.0));
    let song_label = if song_step >= 0 { format!("SONG ▶ {}", song_step + 1) } else { "SONG".to_string() };
    if button(ui, song_r, &song_label, col::AMBER, st.song_view || song_step >= 0) {
        st.song_view = !st.song_view;
        st.editing = None;
    }

    // ── Ruler with playhead ──
    let lane_x = r.min.x + 128.0;
    let lane_w = r.max.x - lane_x;
    let ruler = Rect::from_min_size(pos2(lane_x, y + 72.0), vec2(lane_w, 12.0));
    let full = st.editing.is_some() || st.song_view;
    let bx = |b: f64| lane_x + (b / beats) as f32 * lane_w;
    for b in 0..=beats as usize {
        if full {
            break;
        }
        let bar = b % 4 == 0;
        let x = bx(b as f64);
        p.line_segment([pos2(x, ruler.max.y - if bar { 12.0 } else { 5.0 }), pos2(x, ruler.max.y)], Stroke::new(1.0_f32, if bar { col::MUTED } else { col::LINE }));
        if bar && b < beats as usize {
            text(p, pos2(x + 3.0, ruler.min.y + 3.0), Align2::LEFT_CENTER, b / 4 + 1, 8.0, col::DIM, true);
        }
    }
    let status = match state {
        1 => format!("COUNT-IN  {}", (-pos).ceil().max(1.0) as i32),
        2 => "PLAYING".to_string(),
        3 => format!("RECORDING LAYER {}", rec_layer + 1),
        _ => "STOPPED".to_string(),
    };
    let sc = match state {
        3 => col::RED,
        1 => col::AMBER,
        2 => col::CYAN,
        _ => col::DIM,
    };
    if !full {
        text(p, pos2(r.min.x, ruler.center().y), Align2::LEFT_CENTER, status, 9.5, sc, true);
    }

    // ── Layers (or one layer in the piano roll) ──
    let row_h = 32.0;
    let top = y + 90.0;
    let by = r.max.y - 26.0;
    let body = Rect::from_min_max(pos2(r.min.x, y + 72.0), pos2(r.max.x, by - 8.0));
    if st.song_view {
        if let Some(m) = song_view(ui, p, body, params, shared, &song, scene, state, song_step, bpm) {
            message = Some(m);
        }
    } else if let Some(l) = st.editing {
        edit_layer(ui, p, body, params, shared, &data, scene, st, l, state, rec_layer, pos);
    }
    for l in 0..LAYERS {
        if full {
            break;
        }
        let ry = top + l as f32 * (row_h + 4.0);
        let c = COLORS[l];
        let layer = &data.layers[l];
        let recording = state == 3 && rec_layer == l as i32;
        let armed = state == 1 && rec_layer == l as i32;
        text(p, pos2(r.min.x + 6.0, ry + row_h / 2.0), Align2::CENTER_CENTER, l + 1, 13.0, if layer.muted { col::DIM } else { c }, true);
        let rec_r = Rect::from_min_size(pos2(r.min.x + 16.0, ry + 4.0), vec2(48.0, 24.0));
        let rec_label = if recording { "DONE" } else { "REC" };
        if button(ui, rec_r, rec_label, col::RED, recording || (armed && blink)) {
            send(shared, LoopCmd::Record(l));
        }
        if button(ui, Rect::from_min_size(pos2(r.min.x + 68.0, ry + 4.0), vec2(24.0, 24.0)), "M", col::AMBER, layer.muted) {
            send(shared, LoopCmd::ToggleMute(l));
        }
        if button(ui, Rect::from_min_size(pos2(r.min.x + 96.0, ry + 4.0), vec2(26.0, 24.0)), "CLR", col::CREAM, false) {
            send(shared, LoopCmd::Clear(l));
        }

        // Lane: notes by pitch, beatbox hits in three rows.
        let lane = Rect::from_min_size(pos2(lane_x, ry), vec2(lane_w, row_h));
        boxed(p, lane, if recording { alpha(col::RED, 0.08) } else { Color32::from_rgb(0x12, 0x11, 0x14) }, if recording { col::RED } else { col::LINE_SOFT }, 4.0);
        let ink = if layer.muted { col::DIM } else { c };
        let (lo, hi) = layer.notes.iter().filter(|n| n.drum < 0).fold((127u8, 0u8), |(a, b), n| (a.min(n.note), b.max(n.note)));
        let span = (hi.saturating_sub(lo)).max(6) as f32;
        for n in &layer.notes {
            let x0 = bx(n.start);
            let x1 = bx((n.start + n.len).min(beats)).max(x0 + 2.0);
            let yy = if n.drum >= 0 {
                lane.min.y + 5.0 + n.drum as f32 * 11.0
            } else {
                lane.max.y - 5.0 - (n.note.saturating_sub(lo)) as f32 / span * (row_h - 10.0)
            };
            let h = if n.drum >= 0 { 5.0 } else { 3.0 };
            p.rect_filled(Rect::from_min_max(pos2(x0, yy - h / 2.0), pos2(x1, yy + h / 2.0)), 1.0, alpha(ink, 0.25 + 0.75 * n.vel as f32 / 127.0));
        }
        if layer.notes.is_empty() {
            let hint = if recording { "listening… sing, beatbox or play chords" } else { "empty" };
            text(p, lane.center(), Align2::CENTER_CENTER, hint, 9.0, col::DIM, false);
        }
        // Click a lane to edit it.
        if !recording {
            let resp = ui.interact(lane, ui.id().with(("looplane", l)), Sense::click());
            if resp.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                p.rect_stroke(lane, 4.0, Stroke::new(1.0_f32, alpha(c, 0.7)), nih_plug_egui::egui::StrokeKind::Inside);
                text(p, pos2(lane.max.x - 6.0, lane.min.y + 8.0), Align2::RIGHT_CENTER, "✎ EDIT", 8.5, c, true);
            }
            if resp.clicked() {
                st.editing = Some(l);
                st.ed = NoteEditor::default();
                st.notes = to_ed(layer);
            }
        }
    }
    // Playhead across all lanes.
    if state >= 2 && !full {
        let x = bx(pos);
        p.line_segment([pos2(x, ruler.min.y), pos2(x, top + LAYERS as f32 * (row_h + 4.0) - 4.0)], Stroke::new(1.5_f32, col::CREAM));
    }

    // ── Bottom row: quantize, count-in, undo, export ──
    label(p, pos2(r.min.x, by + 12.0), "SNAP", col::MUTED);
    enum_pills(ui, setter, &params.loop_quantize, Rect::from_min_size(pos2(r.min.x + 34.0, by), vec2(116.0, 24.0)), &[(LoopQuant::Off, "OFF"), (LoopQuant::Q8, "1/8"), (LoopQuant::Q16, "1/16")], col::CYAN);
    label(p, pos2(r.min.x + 160.0, by + 12.0), "COUNT-IN", col::MUTED);
    toggle(ui, setter, &params.loop_count_in, Rect::from_min_size(pos2(r.min.x + 212.0, by + 4.0), vec2(30.0, 16.0)));
    if !full && button(ui, Rect::from_min_size(pos2(r.max.x - 152.0, by), vec2(50.0, 24.0)), "UNDO", col::CREAM, false) {
        send(shared, LoopCmd::Undo);
    }
    let what = if st.song_view { "EXPORT SONG" } else { "EXPORT .MID" };
    if button(ui, Rect::from_min_size(pos2(r.max.x - 96.0, by), vec2(96.0, 24.0)), what, col::AMBER, false) {
        message = Some(if st.song_view { export_song(&song, bpm) } else { export(&data, bpm) });
    }
    message
}

/// One layer in the piano roll, with its own toolbar.
#[allow(clippy::too_many_arguments)]
fn edit_layer(ui: &mut Ui, p: &Painter, r: Rect, params: &VoxParams, shared: &Shared, data: &LoopData, scene: usize, st: &mut LoopUi, l: usize, state: i32, rec_layer: i32, pos: f64) {
    let c = COLORS[l];
    let bar_h = 22.0;
    // Toolbar: back, layer tabs, snap / delete / undo.
    if button(ui, Rect::from_min_size(r.min, vec2(62.0, bar_h)), "◀ BACK", col::CREAM, false) {
        st.editing = None;
        return;
    }
    for k in 0..LAYERS {
        let pr = Rect::from_min_size(pos2(r.min.x + 70.0 + k as f32 * 28.0, r.min.y), vec2(24.0, bar_h));
        if pill(ui, pr, &(k + 1).to_string(), k == l, COLORS[k]) && k != l {
            st.editing = Some(k);
            st.ed = NoteEditor::default();
            st.notes = to_ed(&data.layers[k]);
            return;
        }
    }
    let q = params.loop_quantize.value().beats();
    let grid = if q > 0.0 { q } else { 1.0 / 32.0 };
    let wrap = Some(data.beats);
    let mut changed = false;
    let mut bx = r.max.x;
    for (name, w) in [("UNDO", 50.0), ("DELETE", 60.0), ("SNAP", 50.0)] {
        bx -= w;
        if button(ui, Rect::from_min_size(pos2(bx, r.min.y), vec2(w, bar_h)), name, if name == "SNAP" { col::CYAN } else { col::CREAM }, false) {
            changed |= match name {
                "UNDO" => st.ed.undo(&mut st.notes),
                "DELETE" => st.ed.delete(&mut st.notes),
                _ => st.ed.quantize(&mut st.notes, grid, 0.0, wrap),
            };
        }
        bx -= 4.0;
    }
    let n_sel = st.ed.selected_count();
    let status = match state {
        _ if n_sel > 0 => format!("{n_sel} SELECTED"),
        1 => "COUNT-IN".into(),
        2 => "PLAYING".into(),
        3 => "RECORDING".into(),
        _ => "STOPPED".into(),
    };
    let sc = if n_sel > 0 { col::CREAM } else if state == 3 { col::RED } else { col::DIM };
    text(p, pos2(r.min.x + 70.0 + LAYERS as f32 * 28.0 + 6.0, r.min.y + bar_h / 2.0), Align2::LEFT_CENTER, status, 9.0, sc, true);

    let roll = Rect::from_min_max(pos2(r.min.x, r.min.y + bar_h + 6.0), r.max);
    if state == 3 && rec_layer == l as i32 {
        boxed(p, roll, alpha(col::RED, 0.08), col::RED, 6.0);
        text(p, roll.center(), Align2::CENTER_CENTER, "recording this layer — edit it when the take is done", 11.0, col::MUTED, false);
        return;
    }
    // Follow the looper (takes, clears, undo) unless our own edit is on its way.
    let pending = shared.loop_edit.lock().pending;
    if !pending && !st.ed.busy() {
        let fresh = to_ed(&data.layers[l]);
        if fresh != st.notes {
            st.notes = fresh;
        }
    }
    let cfg = EdCfg { total: data.beats, grid, beat: 1.0, origin: 0.0, playhead: (state >= 2).then_some(pos), channel: shared_channel(params), color: c };
    changed |= st.ed.show(ui, p, roll, &mut st.notes, &cfg, true);
    if changed {
        push_edit(shared, scene, l, &st.notes);
    }
}

fn shared_channel(params: &VoxParams) -> u8 {
    (params.midi_channel.value() - 1).clamp(0, 15) as u8
}

/// The song arrangement: steps as chips (click = more repeats, right-click =
/// remove), buttons to add scenes, play / stop and repeat.
#[allow(clippy::too_many_arguments)]
fn song_view(ui: &mut Ui, p: &Painter, r: Rect, params: &VoxParams, shared: &Shared, song: &SongData, scene: usize, state: i32, song_step: i32, bpm: f32) -> Option<String> {
    let mut message = None;
    let mut steps = song.steps.clone();
    let mut repeat = song.repeat;
    let mut changed = false;

    // Header: play / stop, repeat, length.
    let playing = song_step >= 0;
    if button(ui, Rect::from_min_size(r.min, vec2(104.0, 24.0)), if playing { "STOP SONG" } else { "▶ PLAY SONG" }, col::AMBER, playing) {
        if playing {
            send(shared, LoopCmd::Stop);
        } else if steps.iter().any(|s| !song.scenes[s.scene as usize % SCENES].is_empty()) {
            send(shared, LoopCmd::PlaySong(Song::from_steps(&steps, repeat)));
        } else {
            message = Some("record something in the scenes first".to_string());
        }
    }
    if pill(ui, Rect::from_min_size(pos2(r.min.x + 112.0, r.min.y + 1.0), vec2(70.0, 22.0)), "REPEAT", repeat, col::CYAN) {
        repeat = !repeat;
        changed = true;
    }
    let beats = song.song_beats();
    let secs = beats * 60.0 / bpm.clamp(20.0, 400.0) as f64;
    text(p, pos2(r.max.x, r.min.y + 12.0), Align2::RIGHT_CENTER, format!("{:.0} bars · {}:{:02}", beats / 4.0, secs as u32 / 60, secs as u32 % 60), 10.0, col::MUTED, true);

    // Steps.
    let chip = vec2(46.0, 40.0);
    let gap = 6.0;
    let per_row = ((r.width() + gap) / (chip.x + gap)).floor().max(1.0) as usize;
    let top = r.min.y + 34.0;
    let mut remove = None;
    for (i, step) in steps.iter_mut().enumerate() {
        let cr = Rect::from_min_size(pos2(r.min.x + (i % per_row) as f32 * (chip.x + gap), top + (i / per_row) as f32 * (chip.y + gap)), chip);
        let resp = ui.interact(cr, ui.id().with(("songstep", i)), Sense::click());
        let now = song_step == i as i32;
        let empty = song.scenes[step.scene as usize % SCENES].is_empty();
        let edge = if now { col::AMBER } else if resp.hovered() { col::CREAM } else { col::LINE };
        boxed(p, cr, if now { alpha(col::AMBER, 0.18) } else { Color32::from_rgb(0x12, 0x11, 0x14) }, edge, 5.0);
        let ink = if empty { col::DIM } else if now { col::AMBER } else { col::CYAN };
        text(p, pos2(cr.center().x, cr.min.y + 15.0), Align2::CENTER_CENTER, SCENE_NAMES[step.scene as usize % SCENES], 15.0, ink, true);
        text(p, pos2(cr.center().x, cr.max.y - 9.0), Align2::CENTER_CENTER, format!("×{}", step.repeats), 9.5, col::MUTED, true);
        if resp.clicked() {
            step.repeats = match step.repeats {
                1 => 2,
                2 => 4,
                4 => 8,
                _ => 1,
            };
            changed = true;
        }
        if resp.secondary_clicked() {
            remove = Some(i);
        }
    }
    if let Some(i) = remove {
        steps.remove(i);
        changed = true;
    }
    // Add buttons right after the last chip.
    let n = steps.len();
    if n < SONG_MAX {
        let ar = Rect::from_min_size(pos2(r.min.x + (n % per_row) as f32 * (chip.x + gap), top + (n / per_row) as f32 * (chip.y + gap)), chip);
        let resp = ui.interact(ar, ui.id().with("songadd"), Sense::click());
        boxed(p, ar, Color32::TRANSPARENT, if resp.hovered() { col::CREAM } else { col::LINE_SOFT }, 5.0);
        text(p, pos2(ar.center().x, ar.min.y + 15.0), Align2::CENTER_CENTER, format!("+{}", SCENE_NAMES[scene]), 13.0, col::MUTED, true);
        text(p, pos2(ar.center().x, ar.max.y - 9.0), Align2::CENTER_CENTER, "add", 8.5, col::DIM, false);
        if resp.clicked() {
            steps.push(SongStep { scene: scene as u8, repeats: 2 });
            changed = true;
        }
    }
    let hint = if steps.is_empty() { "pick a scene above, then + to add it" } else { "click: ×2 ×4 ×8 · right-click: remove · + adds the scene picked above" };
    text(p, pos2(r.min.x, r.max.y - 4.0), Align2::LEFT_CENTER, hint, 8.5, col::DIM, false);

    if changed {
        let mut saved = params.song.write();
        saved.steps = steps;
        saved.repeat = repeat;
        // Changes take effect the next time the song starts.
        if playing && state != 0 {
            message = Some("song changed — press PLAY SONG again to hear it".to_string());
        }
    }
    message
}

/// Write every unmuted layer of the song's scenes, step after step.
fn export_song(song: &SongData, bpm: f32) -> String {
    let spb = 60.0 / bpm.clamp(20.0, 400.0) as f64;
    let mut notes = Vec::new();
    let mut at = 0.0;
    for step in &song.steps {
        let data = &song.scenes[step.scene as usize % SCENES];
        for _ in 0..step.repeats {
            notes.extend(loop_notes(data, at, spb));
            at += data.beats;
        }
    }
    if notes.is_empty() {
        return "song is empty — add scenes that have notes".into();
    }
    save_mid(&notes, bpm, "song")
}

/// A scene's unmuted notes, starting at beat `at`, in seconds.
fn loop_notes(data: &LoopData, at: f64, spb: f64) -> impl Iterator<Item = RecordedNote> + '_ {
    data.layers.iter().filter(|l| !l.muted).flat_map(|l| l.notes.iter()).map(move |n| RecordedNote {
        start: (at + n.start) * spb,
        end: (at + (n.start + n.len).min(data.beats)) * spb,
        note: n.note,
        velocity: n.vel,
        channel: n.ch,
    })
}

/// Write all unmuted layers as one loop to ~/Music/Qimist.
fn export(data: &LoopData, bpm: f32) -> String {
    if data.is_empty() {
        return "loop is empty — record a layer first".into();
    }
    let spb = 60.0 / bpm.clamp(20.0, 400.0) as f64;
    let notes: Vec<RecordedNote> = loop_notes(data, 0.0, spb).collect();
    save_mid(&notes, bpm, "loop")
}

fn save_mid(notes: &[RecordedNote], bpm: f32, what: &str) -> String {
    let mut notes = notes.to_vec();
    notes.sort_by(|a, b| a.start.total_cmp(&b.start));
    let bytes = encode_smf(&notes, bpm as f64, false, 0);
    let dir = super::export_dir();
    let _ = std::fs::create_dir_all(&dir);
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let path: PathBuf = dir.join(format!("Qimist-{what}-{secs}.mid"));
    match std::fs::write(&path, bytes) {
        Ok(()) => format!("saved {}", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
        Err(e) => format!("export failed: {e}"),
    }
}
