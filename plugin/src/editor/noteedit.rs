//! Piano-roll note editor, shared by the recorded take (seconds) and the loop
//! layers (beats).
//!
//! Mouse: click a note to select it (Shift adds), drag it to move (time snaps
//! to the grid, up/down changes pitch), drag its right edge to resize, drag on
//! empty space to box-select, double-click empty space to add a note.
//! Keys (while the pointer is over the editor): Delete/Backspace, ↑/↓
//! semitone (Shift = octave), ←/→ one grid step, Q quantize, Ctrl+A select
//! all, Ctrl+Z undo, Ctrl+Y / Ctrl+Shift+Z redo.

use super::widgets::*;
use nih_plug_egui::egui::{self, pos2, vec2, Align2, Color32, CursorIcon, Key, Painter, Pos2, Rect, Sense, Stroke, Ui};
use qimist_dsp::NOTE_NAMES;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdNote {
    pub start: f64,
    pub len: f64,
    pub note: u8,
    pub vel: u8,
    pub ch: u8,
    /// Beatbox class for loop drum hits (−1 = normal note). Hits keep their pitch.
    pub drum: i8,
}

/// Grid and view settings from the caller.
pub struct EdCfg {
    /// Time shown (at least this long).
    pub total: f64,
    /// Snap step (also the nudge step and a new note's length).
    pub grid: f64,
    /// One beat, for the grid lines.
    pub beat: f64,
    /// Where the grid starts (a take's grid starts at its first note, like the export).
    pub origin: f64,
    pub playhead: Option<f64>,
    pub channel: u8,
    pub color: Color32,
}

#[derive(Clone)]
enum Drag {
    Move { origin: Pos2, before: Vec<EdNote> },
    Resize { origin: Pos2, before: Vec<EdNote> },
    Select { origin: Pos2 },
}

#[derive(Default)]
pub struct NoteEditor {
    pub sel: Vec<bool>,
    drag: Option<Drag>,
    undo: Vec<Vec<EdNote>>,
    redo: Vec<Vec<EdNote>>,
    /// Pitch rows shown; frozen while dragging so the grid doesn't jump.
    rows: Option<(i32, i32)>,
}

const MAX_UNDO: usize = 50;

fn snap(t: f64, grid: f64) -> f64 {
    if grid > 0.0 {
        (t / grid).round() * grid
    } else {
        t
    }
}

fn snap_at(t: f64, grid: f64, origin: f64) -> f64 {
    origin + snap(t - origin, grid)
}

impl NoteEditor {
    /// A drag is in progress (don't swap the notes underneath it).
    pub fn busy(&self) -> bool {
        self.drag.is_some()
    }

    pub fn selected_count(&self) -> usize {
        self.sel.iter().filter(|&&s| s).count()
    }

    fn fit_sel(&mut self, n: usize) {
        self.sel.resize(n, false);
    }

    /// Remember the state before an edit.
    pub fn checkpoint(&mut self, notes: &[EdNote]) {
        self.undo.push(notes.to_vec());
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn undo(&mut self, notes: &mut Vec<EdNote>) -> bool {
        let Some(prev) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(notes, prev));
        self.sel = vec![false; notes.len()];
        true
    }

    pub fn redo(&mut self, notes: &mut Vec<EdNote>) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(notes, next));
        self.sel = vec![false; notes.len()];
        true
    }

    /// Notes the next operation applies to: the selection, or everything.
    fn targets(&self, n: usize) -> Vec<usize> {
        let any = self.sel.iter().any(|&s| s);
        (0..n).filter(|&i| !any || self.sel[i]).collect()
    }

    /// Keep notes in time order, carrying the selection along.
    fn sort(&mut self, notes: &mut [EdNote]) {
        self.fit_sel(notes.len());
        let mut both: Vec<(EdNote, bool)> = notes.iter().copied().zip(self.sel.iter().copied()).collect();
        both.sort_by(|a, b| a.0.start.total_cmp(&b.0.start).then(a.0.note.cmp(&b.0.note)));
        for (i, (n, s)) in both.into_iter().enumerate() {
            notes[i] = n;
            self.sel[i] = s;
        }
    }

    pub fn delete(&mut self, notes: &mut Vec<EdNote>) -> bool {
        self.fit_sel(notes.len());
        if !self.sel.iter().any(|&s| s) {
            return false;
        }
        self.checkpoint(notes);
        let mut k = 0;
        notes.retain(|_| {
            let keep = !self.sel[k];
            k += 1;
            keep
        });
        self.sel = vec![false; notes.len()];
        true
    }

    pub fn transpose(&mut self, notes: &mut [EdNote], semis: i32) -> bool {
        self.fit_sel(notes.len());
        let t = self.targets(notes.len());
        if t.is_empty() {
            return false;
        }
        self.checkpoint(notes);
        for i in t {
            if notes[i].drum < 0 {
                notes[i].note = (notes[i].note as i32 + semis).clamp(0, 127) as u8;
            }
        }
        true
    }

    pub fn nudge(&mut self, notes: &mut [EdNote], dt: f64, total: Option<f64>) -> bool {
        self.fit_sel(notes.len());
        let t = self.targets(notes.len());
        if t.is_empty() {
            return false;
        }
        self.checkpoint(notes);
        for i in t {
            let mut s = (notes[i].start + dt).max(0.0);
            if let Some(total) = total {
                s = s.rem_euclid(total);
            }
            notes[i].start = s;
        }
        self.sort(notes);
        true
    }

    /// Snap starts (and lengths) to the grid.
    pub fn quantize(&mut self, notes: &mut [EdNote], grid: f64, origin: f64, total: Option<f64>) -> bool {
        self.fit_sel(notes.len());
        let t = self.targets(notes.len());
        if t.is_empty() || grid <= 0.0 {
            return false;
        }
        self.checkpoint(notes);
        for i in t {
            let mut s = snap_at(notes[i].start, grid, origin).max(0.0);
            if let Some(total) = total {
                s = s.rem_euclid(total);
            }
            notes[i].start = s;
            if notes[i].drum < 0 {
                notes[i].len = snap(notes[i].len, grid).max(grid);
            }
        }
        self.sort(notes);
        true
    }

    pub fn add(&mut self, notes: &mut Vec<EdNote>, n: EdNote) {
        self.checkpoint(notes);
        notes.push(n);
        self.sel = vec![false; notes.len() - 1];
        self.sel.push(true);
        self.sort(notes);
    }

    pub fn select_all(&mut self, n: usize) {
        self.sel = vec![true; n];
    }

    /// Draw and edit. Returns true when `notes` changed.
    /// `looped`: times wrap at `cfg.total` (loop layers).
    pub fn show(&mut self, ui: &mut Ui, p: &Painter, r: Rect, notes: &mut Vec<EdNote>, cfg: &EdCfg, looped: bool) -> bool {
        self.fit_sel(notes.len());
        let mut changed = false;
        let wrap = looped.then_some(cfg.total);
        boxed(p, r, col::SCREEN, col::LINE_SOFT, 6.0);

        // ── View ──
        let total = notes.iter().map(|n| n.start + n.len).fold(cfg.total, f64::max).max(cfg.grid * 4.0);
        if self.drag.is_none() || self.rows.is_none() {
            let (mut lo, mut hi) = notes.iter().fold((127i32, 0i32), |(l, h), n| (l.min(n.note as i32), h.max(n.note as i32)));
            if lo > hi {
                (lo, hi) = (60, 72);
            }
            let pad = (13 - (hi - lo + 1)).max(0);
            self.rows = Some(((lo - pad / 2 - 1).max(0), (hi + (pad - pad / 2) + 1).min(127)));
        }
        let (lo, hi) = self.rows.unwrap_or((60, 72));
        let keys_w = 34.0;
        let grid_r = Rect::from_min_max(pos2(r.min.x + keys_w, r.min.y + 1.0), pos2(r.max.x - 1.0, r.max.y - 1.0));
        let rows = (hi - lo + 1) as f32;
        let rh = grid_r.height() / rows;
        let px = grid_r.width() as f64 / total;
        let x_of = |t: f64| grid_r.min.x + (t * px) as f32;
        let t_of = |x: f32| ((x - grid_r.min.x) as f64 / px).max(0.0);
        let y_of = |n: i32| grid_r.max.y - (n - lo + 1) as f32 * rh;
        let n_of = |y: f32| (lo + ((grid_r.max.y - y) / rh).floor() as i32).clamp(0, 127);

        for n in lo..=hi {
            let yy = y_of(n);
            let black = NOTE_NAMES[n.rem_euclid(12) as usize].len() > 1;
            p.rect_filled(Rect::from_min_size(pos2(grid_r.min.x, yy), vec2(grid_r.width(), rh)), 0.0_f32, if black { Color32::from_rgb(0x0d, 0x0c, 0x0f) } else { Color32::from_rgb(0x12, 0x10, 0x14) });
            p.rect_filled(Rect::from_min_size(pos2(r.min.x + 1.0, yy), vec2(keys_w - 2.0, rh)), 0.0_f32, if black { Color32::from_rgb(0x0f, 0x0e, 0x11) } else { col::PANEL });
            if rh >= 9.0 {
                text(p, pos2(r.min.x + 4.0, yy + rh / 2.0), Align2::LEFT_CENTER, format!("{}{}", NOTE_NAMES[n.rem_euclid(12) as usize], n.div_euclid(12) - 1), 7.5, col::MUTED, true);
            }
        }
        // Grid: steps faint, beats stronger, bars strongest.
        let step = if cfg.grid * px >= 6.0 { cfg.grid } else { cfg.beat };
        let mut k = 0usize;
        loop {
            let t = cfg.origin.rem_euclid(step) + k as f64 * step;
            if t > total {
                break;
            }
            let rel = (t - cfg.origin) / cfg.beat;
            let on_beat = (rel - rel.round()).abs() < 1e-6;
            let beat_i = rel.round() as i64;
            let c = if on_beat && beat_i.rem_euclid(4) == 0 { col::LINE } else if on_beat { Color32::from_rgb(0x1e, 0x1b, 0x21) } else { Color32::from_rgb(0x15, 0x13, 0x18) };
            p.line_segment([pos2(x_of(t), grid_r.min.y), pos2(x_of(t), grid_r.max.y)], Stroke::new(1.0_f32, c));
            k += 1;
        }

        // ── Hit testing ──
        let resp = ui.interact(grid_r, ui.id().with("noteedit"), Sense::click_and_drag());
        let ptr = resp.hover_pos().or(resp.interact_pointer_pos());
        let note_rect = |n: &EdNote| {
            let x0 = x_of(n.start);
            let x1 = x_of(n.start + n.len).max(x0 + 4.0).min(grid_r.max.x);
            let yy = y_of(n.note as i32);
            Rect::from_min_max(pos2(x0, yy + 1.0), pos2(x1, yy + rh - 1.0))
        };
        let hit = |notes: &[EdNote], pos: Pos2| -> Option<(usize, bool)> {
            (0..notes.len()).rev().find_map(|i| {
                let nr = note_rect(&notes[i]);
                nr.expand2(vec2(2.0, 0.0)).contains(pos).then(|| (i, pos.x > nr.max.x - 5.0 && notes[i].drum < 0))
            })
        };
        if let Some(pos) = ptr {
            if self.drag.is_none() {
                match hit(notes, pos) {
                    Some((_, true)) => ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal),
                    Some((_, false)) => ui.ctx().set_cursor_icon(CursorIcon::Grab),
                    None => {}
                }
            }
        }
        let shift = ui.input(|i| i.modifiers.shift);

        if resp.drag_started() {
            if let Some(origin) = resp.interact_pointer_pos() {
                match hit(notes, origin) {
                    Some((i, edge)) => {
                        if !self.sel[i] {
                            if !shift {
                                self.sel.iter_mut().for_each(|s| *s = false);
                            }
                            self.sel[i] = true;
                        }
                        let before = notes.clone();
                        self.drag = Some(if edge { Drag::Resize { origin, before } } else { Drag::Move { origin, before } });
                    }
                    None => {
                        if !shift {
                            self.sel.iter_mut().for_each(|s| *s = false);
                        }
                        self.drag = Some(Drag::Select { origin });
                    }
                }
            }
        }
        if let (Some(drag), Some(pos)) = (self.drag.clone(), resp.interact_pointer_pos()) {
            match drag {
                Drag::Move { origin, before } => {
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                    let dt = snap((pos.x - origin.x) as f64 / px, cfg.grid);
                    let dp = ((origin.y - pos.y) / rh).round() as i32;
                    for (i, b) in before.iter().enumerate() {
                        if self.sel[i] {
                            let mut s = (b.start + dt).max(0.0);
                            if let Some(t) = wrap {
                                s = s.rem_euclid(t);
                            }
                            let note = if b.drum >= 0 { b.note } else { (b.note as i32 + dp).clamp(0, 127) as u8 };
                            if notes[i].start != s || notes[i].note != note {
                                notes[i].start = s;
                                notes[i].note = note;
                                changed = true;
                            }
                        }
                    }
                }
                Drag::Resize { origin, before } => {
                    let dl = snap((pos.x - origin.x) as f64 / px, cfg.grid);
                    for (i, b) in before.iter().enumerate() {
                        if self.sel[i] && b.drum < 0 {
                            let len = (b.len + dl).max(cfg.grid.max(0.01) * 0.5);
                            if notes[i].len != len {
                                notes[i].len = len;
                                changed = true;
                            }
                        }
                    }
                }
                Drag::Select { origin } => {
                    let sel_r = Rect::from_two_pos(origin, pos);
                    p.rect_filled(sel_r, 2.0, alpha(cfg.color, 0.08));
                    p.rect_stroke(sel_r, 2.0, Stroke::new(1.0_f32, alpha(cfg.color, 0.6)), egui::StrokeKind::Inside);
                    for (i, n) in notes.iter().enumerate() {
                        if note_rect(n).intersects(sel_r) {
                            self.sel[i] = true;
                        } else if !shift {
                            self.sel[i] = false;
                        }
                    }
                }
            }
        }
        if resp.drag_stopped() {
            match self.drag.take() {
                Some(Drag::Move { before, .. }) | Some(Drag::Resize { before, .. }) if before != *notes => {
                    self.undo.push(before);
                    if self.undo.len() > MAX_UNDO {
                        self.undo.remove(0);
                    }
                    self.redo.clear();
                    self.sort(notes);
                }
                _ => {}
            }
        }
        if resp.double_clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                if hit(notes, pos).is_none() {
                    let start = (cfg.origin + ((t_of(pos.x) - cfg.origin) / cfg.grid).floor() * cfg.grid).max(0.0);
                    let start = wrap.map_or(start, |t| start.rem_euclid(t));
                    self.add(notes, EdNote { start, len: cfg.beat.max(cfg.grid), note: n_of(pos.y) as u8, vel: 100, ch: cfg.channel, drum: -1 });
                    changed = true;
                }
            }
        } else if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                match hit(notes, pos) {
                    Some((i, _)) if shift => self.sel[i] = !self.sel[i],
                    Some((i, _)) => {
                        self.sel.iter_mut().for_each(|s| *s = false);
                        self.sel[i] = true;
                    }
                    None if !shift => self.sel.iter_mut().for_each(|s| *s = false),
                    None => {}
                }
            }
        }

        // ── Keys (pointer over the editor) ──
        if resp.hovered() || self.drag.is_some() {
            let (del, up, down, left, right, q, all, z, y, cmd, shift) = ui.input(|i| {
                (
                    i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace),
                    i.key_pressed(Key::ArrowUp),
                    i.key_pressed(Key::ArrowDown),
                    i.key_pressed(Key::ArrowLeft),
                    i.key_pressed(Key::ArrowRight),
                    i.key_pressed(Key::Q),
                    i.key_pressed(Key::A),
                    i.key_pressed(Key::Z),
                    i.key_pressed(Key::Y),
                    i.modifiers.command,
                    i.modifiers.shift,
                )
            });
            let oct = if shift { 12 } else { 1 };
            changed |= del && self.delete(notes);
            changed |= up && self.transpose(notes, oct);
            changed |= down && self.transpose(notes, -oct);
            changed |= left && self.nudge(notes, -cfg.grid, wrap);
            changed |= right && self.nudge(notes, cfg.grid, wrap);
            changed |= q && !cmd && self.quantize(notes, cfg.grid, cfg.origin, wrap);
            if all && cmd {
                self.select_all(notes.len());
            }
            if cmd && ((z && shift) || y) {
                changed |= self.redo(notes);
            } else if cmd && z {
                changed |= self.undo(notes);
            }
        }

        // ── Notes ──
        self.fit_sel(notes.len());
        for (i, n) in notes.iter().enumerate() {
            let nr = note_rect(n);
            let a = 0.35 + 0.65 * n.vel as f32 / 127.0;
            if self.sel[i] {
                p.rect_filled(nr.expand(2.0), 4.0_f32, alpha(col::CREAM, 0.18));
                p.rect_filled(nr, 3.0_f32, alpha(cfg.color, a));
                p.rect_stroke(nr, 3.0_f32, Stroke::new(1.5_f32, col::CREAM), egui::StrokeKind::Outside);
            } else {
                p.rect_filled(nr, 3.0_f32, alpha(cfg.color, a));
            }
            if n.drum < 0 && nr.width() > 10.0 {
                // Resize grip.
                p.line_segment([pos2(nr.max.x - 3.0, nr.min.y + 2.0), pos2(nr.max.x - 3.0, nr.max.y - 2.0)], Stroke::new(1.0_f32, alpha(col::SCREEN, 0.6)));
            }
        }
        if let Some(t) = cfg.playhead {
            let x = x_of(t);
            p.line_segment([pos2(x, grid_r.min.y), pos2(x, grid_r.max.y)], Stroke::new(1.5_f32, col::CREAM));
        }
        if notes.is_empty() {
            text(p, grid_r.center(), Align2::CENTER_CENTER, "double-click to add a note", 11.0, col::DIM, false);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(start: f64, note: u8) -> EdNote {
        EdNote { start, len: 0.5, note, vel: 100, ch: 0, drum: -1 }
    }

    #[test]
    fn edit_ops_and_undo() {
        let mut ed = NoteEditor::default();
        let mut notes = vec![n(0.1, 60), n(0.9, 64), n(2.2, 67)];
        // Quantize everything to 0.5.
        assert!(ed.quantize(&mut notes, 0.5, 0.0, None));
        assert_eq!(notes.iter().map(|n| n.start).collect::<Vec<_>>(), [0.0, 1.0, 2.0]);
        // Select the middle note and transpose an octave, then delete it.
        ed.sel = vec![false, true, false];
        assert!(ed.transpose(&mut notes, 12));
        assert_eq!(notes[1].note, 76);
        assert!(ed.delete(&mut notes));
        assert_eq!(notes.len(), 2);
        // Undo twice: note back, then back at 64.
        assert!(ed.undo(&mut notes));
        assert_eq!(notes.len(), 3);
        assert!(ed.undo(&mut notes));
        assert_eq!(notes[1].note, 64);
        assert!(ed.redo(&mut notes));
        assert_eq!(notes[1].note, 76);
    }

    #[test]
    fn nudge_wraps_in_loops_and_keeps_order() {
        let mut ed = NoteEditor::default();
        let mut notes = vec![n(0.0, 60), n(7.75, 62)];
        ed.sel = vec![false, true];
        assert!(ed.nudge(&mut notes, 0.25, Some(8.0)));
        // 7.75 + 0.25 wraps to 0.0 and sorts; the moved note stays selected.
        assert_eq!(notes.len(), 2);
        let moved = notes.iter().position(|n| n.note == 62).unwrap();
        assert_eq!(notes[moved].start, 0.0);
        assert!(ed.sel[moved]);
    }

    #[test]
    fn drum_hits_keep_their_pitch() {
        let mut ed = NoteEditor::default();
        let mut notes = vec![EdNote { drum: 0, note: 36, ..n(0.0, 36) }];
        ed.transpose(&mut notes, 5);
        assert_eq!(notes[0].note, 36);
    }
}
