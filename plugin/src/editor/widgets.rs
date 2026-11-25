//! Hand-drawn widgets in the Qimist look: neon on dark, vintage hardware details.
//! Everything is painted with the egui painter and made interactive with
//! `ui.interact`, so it stays independent of egui's built-in widget styling.

use nih_plug::prelude::{BoolParam, Enum, EnumParam, IntParam, Param, ParamSetter};
use nih_plug_egui::egui::{self, pos2, vec2, Align2, Color32, FontFamily, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui};

pub mod col {
    use super::Color32;
    pub const BG: Color32 = Color32::from_rgb(0x0b, 0x0a, 0x0c);
    pub const BODY: Color32 = Color32::from_rgb(0x15, 0x13, 0x17);
    pub const PANEL: Color32 = Color32::from_rgb(0x1d, 0x1a, 0x20);
    pub const SCREEN: Color32 = Color32::from_rgb(0x0a, 0x0a, 0x0c);
    pub const LINE: Color32 = Color32::from_rgb(0x2c, 0x28, 0x30);
    pub const LINE_SOFT: Color32 = Color32::from_rgb(0x22, 0x1f, 0x25);
    pub const KNOB: Color32 = Color32::from_rgb(0x2a, 0x26, 0x2d);
    pub const KNOB_RIM: Color32 = Color32::from_rgb(0x3a, 0x35, 0x3e);
    pub const CREAM: Color32 = Color32::from_rgb(0xe9, 0xdf, 0xc8);
    pub const MUTED: Color32 = Color32::from_rgb(0xa8, 0x9c, 0x86);
    pub const DIM: Color32 = Color32::from_rgb(0x6b, 0x63, 0x58);
    pub const CYAN: Color32 = Color32::from_rgb(0x3e, 0xe8, 0xff);
    pub const AMBER: Color32 = Color32::from_rgb(0xff, 0xb3, 0x40);
    pub const RED: Color32 = Color32::from_rgb(0xff, 0x5a, 0x4f);
    pub const WOOD: Color32 = Color32::from_rgb(0x3a, 0x24, 0x18);
    pub const INK: Color32 = Color32::from_rgb(0x2a, 0x24, 0x20);
    pub const FACE: Color32 = Color32::from_rgb(0xef, 0xe2, 0xc2);
}

pub fn alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}

pub fn font(size: f32, mono: bool) -> FontId {
    FontId::new(size, if mono { FontFamily::Monospace } else { FontFamily::Proportional })
}

pub fn text(p: &Painter, pos: Pos2, align: Align2, s: impl ToString, size: f32, color: Color32, mono: bool) -> Rect {
    p.text(pos, align, s.to_string(), font(size, mono), color)
}

/// Small uppercase section label.
pub fn label(p: &Painter, pos: Pos2, s: &str, color: Color32) -> Rect {
    text(p, pos, Align2::LEFT_CENTER, s, 10.0, color, false)
}

/// Rounded panel with a 1px border.
pub fn panel(p: &Painter, r: Rect, fill: Color32) {
    p.rect_filled(r, 10.0_f32, col::LINE);
    p.rect_filled(r.shrink(1.0), 9.0_f32, fill);
}

/// Filled rounded box with a 1px border. Opaque fills paint border-then-fill
/// (crisp); translucent fills get a real outline so the border colour doesn't
/// show through them.
pub fn boxed(p: &Painter, r: Rect, fill: Color32, border: Color32, radius: f32) {
    if fill.a() == 255 {
        p.rect_filled(r, radius, border);
        p.rect_filled(r.shrink(1.0), (radius - 1.0).max(0.0), fill);
    } else {
        p.rect_filled(r.shrink(1.0), (radius - 1.0).max(0.0), fill);
        p.rect_stroke(r.shrink(0.5), radius, Stroke::new(1.0_f32, border), egui::StrokeKind::Inside);
    }
}

/// Polyline with a soft neon glow underneath.
pub fn glow_line(p: &Painter, pts: Vec<Pos2>, color: Color32, width: f32) {
    if pts.len() < 2 {
        return;
    }
    p.add(Shape::line(pts.clone(), Stroke::new(width + 5.0, alpha(color, 0.10))));
    p.add(Shape::line(pts.clone(), Stroke::new(width + 2.5, alpha(color, 0.22))));
    p.add(Shape::line(pts, Stroke::new(width, color)));
}

pub fn glow_dot(p: &Painter, c: Pos2, r: f32, color: Color32) {
    p.circle_filled(c, r * 3.0, alpha(color, 0.10));
    p.circle_filled(c, r * 1.8, alpha(color, 0.25));
    p.circle_filled(c, r, color);
}

pub fn led(p: &Painter, c: Pos2, on: bool, color: Color32) {
    if on {
        glow_dot(p, c, 4.0, color);
    } else {
        p.circle_filled(c, 4.0, col::KNOB);
    }
}

pub fn bar(p: &Painter, r: Rect, frac: f32, color: Color32) {
    p.rect_filled(r, r.height() / 2.0, col::SCREEN);
    let w = r.width() * frac.clamp(0.0, 1.0);
    if w > 0.5 {
        p.rect_filled(Rect::from_min_size(r.min, vec2(w, r.height())), r.height() / 2.0, color);
    }
}

/// Points along a knob arc. Angle 0 = up, clockwise positive, degrees.
pub fn arc_points(c: Pos2, r: f32, a0: f32, a1: f32) -> Vec<Pos2> {
    let steps = (((a1 - a0).abs() / 4.0).ceil() as usize).max(2);
    (0..=steps)
        .map(|i| {
            let a = (a0 + (a1 - a0) * i as f32 / steps as f32).to_radians();
            pos2(c.x + r * a.sin(), c.y - r * a.cos())
        })
        .collect()
}

/// Rotary knob bound to a parameter: drag vertically, double-click resets.
#[allow(clippy::too_many_arguments)]
pub fn knob<P: Param>(ui: &mut Ui, setter: &ParamSetter, param: &P, center: Pos2, radius: f32, name: &str, color: Color32) {
    let hit = Rect::from_center_size(center + vec2(0.0, 8.0), vec2(radius * 2.6, radius * 2.0 + 30.0));
    let id = ui.id().with(("knob", name, center.x as i32, center.y as i32));
    let resp = ui.interact(hit, id, Sense::click_and_drag());

    if resp.double_clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter_normalized(param, param.default_normalized_value());
        setter.end_set_parameter(param);
    } else {
        if resp.drag_started() {
            setter.begin_set_parameter(param);
        }
        if resp.dragged() {
            let fine = ui.input(|i| i.modifiers.shift);
            let speed = if fine { 1.0 / 900.0 } else { 1.0 / 180.0 };
            let v = (param.unmodulated_normalized_value() - resp.drag_delta().y * speed).clamp(0.0, 1.0);
            setter.set_parameter_normalized(param, v);
        }
        if resp.drag_stopped() {
            setter.end_set_parameter(param);
        }
    }

    // Right-click: type an exact value (Enter applies, Esc cancels).
    let edit_id = id.with("edit");
    if resp.secondary_clicked() {
        ui.ctx().data_mut(|d| d.insert_temp(edit_id, String::new()));
    }
    if let Some(mut typed) = ui.ctx().data(|d| d.get_temp::<String>(edit_id)) {
        let r = Rect::from_center_size(center + vec2(0.0, radius + 9.0), vec2((radius * 2.0 + 30.0).max(64.0), 20.0));
        let (mut close, mut commit) = (false, false);
        let shown = param.normalized_value_to_string(param.unmodulated_normalized_value(), true);
        egui::Area::new(edit_id.with("area")).order(egui::Order::Foreground).fixed_pos(r.min).show(ui.ctx(), |ui| {
            let te = ui.add(egui::TextEdit::singleline(&mut typed).desired_width(r.width()).hint_text(shown).font(egui::TextStyle::Monospace));
            te.request_focus();
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                commit = true;
                close = true;
            } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) || te.clicked_elsewhere() {
                close = true;
            }
        });
        if commit {
            if let Some(v) = param.string_to_normalized_value(typed.trim()) {
                setter.begin_set_parameter(param);
                setter.set_parameter_normalized(param, v);
                setter.end_set_parameter(param);
            }
        }
        ui.ctx().data_mut(|d| if close { d.remove::<String>(edit_id) } else { d.insert_temp(edit_id, typed) });
    }
    if !resp.dragged() {
        let value = param.normalized_value_to_string(param.unmodulated_normalized_value(), true);
        resp.clone().on_hover_text(format!("{}: {value}\nDrag to change · Shift = fine\nDouble-click = reset · Right-click = type a value", param.name()));
    }

    let v = param.unmodulated_normalized_value();
    let p = ui.painter();
    let ang = -135.0 + 270.0 * v;
    p.add(Shape::line(arc_points(center, radius, -135.0, 135.0), Stroke::new(4.0_f32, col::SCREEN)));
    if v > 0.002 {
        glow_line(p, arc_points(center, radius, -135.0, ang), color, 3.0);
    }
    let cap = radius * 0.68;
    p.circle_filled(center, cap + 1.0, col::KNOB_RIM);
    p.circle_filled(center, cap, if resp.hovered() || resp.dragged() { Color32::from_rgb(0x33, 0x2e, 0x37) } else { col::KNOB });
    let a = ang.to_radians();
    let tip = pos2(center.x + cap * 0.85 * a.sin(), center.y - cap * 0.85 * a.cos());
    let base = pos2(center.x + cap * 0.25 * a.sin(), center.y - cap * 0.25 * a.cos());
    p.line_segment([base, tip], Stroke::new(2.0_f32, col::CREAM));

    let value = param.normalized_value_to_string(v, true);
    text(p, center + vec2(0.0, radius + 9.0), Align2::CENTER_CENTER, value, 11.0, col::CREAM, true);
    text(p, center + vec2(0.0, radius + 22.0), Align2::CENTER_CENTER, name, 9.0, col::MUTED, false);
}

/// Pill button; returns true when clicked.
pub fn pill(ui: &mut Ui, r: Rect, s: &str, active: bool, color: Color32) -> bool {
    let resp = ui.interact(r, ui.id().with(("pill", s, r.min.x as i32, r.min.y as i32)), Sense::click());
    let p = ui.painter();
    if active {
        p.rect_filled(r.expand(2.0), 7.0_f32, alpha(color, 0.10));
        boxed(p, r, alpha(color, 0.10), color, 5.0);
        text(p, r.center(), Align2::CENTER_CENTER, s, 11.0, color, false);
    } else {
        let fill = if resp.hovered() { Color32::from_rgb(0x1f, 0x1c, 0x22) } else { Color32::TRANSPARENT };
        p.rect_filled(r, 5.0_f32, fill);
        text(p, r.center(), Align2::CENTER_CENTER, s, 11.0, col::MUTED, false);
    }
    resp.clicked()
}

/// Row of pills selecting an enum parameter's value.
pub fn enum_pills<T: Enum + PartialEq + Copy + 'static>(ui: &mut Ui, setter: &ParamSetter, param: &EnumParam<T>, r: Rect, options: &[(T, &str)], color: Color32) {
    ui.painter().rect_filled(r, 7.0_f32, col::LINE);
    ui.painter().rect_filled(r.shrink(1.0), 6.0_f32, Color32::from_rgb(0x0f, 0x0e, 0x11));
    let inner = r.shrink(3.0);
    let w = inner.width() / options.len() as f32;
    let current = param.value();
    for (i, (v, name)) in options.iter().enumerate() {
        let pr = Rect::from_min_size(pos2(inner.min.x + w * i as f32, inner.min.y), vec2(w - 2.0, inner.height()));
        if pill(ui, pr, name, current == *v, color) && current != *v {
            setter.begin_set_parameter(param);
            setter.set_parameter(param, *v);
            setter.end_set_parameter(param);
        }
    }
}

/// Neon toggle switch for a bool parameter.
pub fn toggle(ui: &mut Ui, setter: &ParamSetter, param: &BoolParam, r: Rect) {
    let resp = ui.interact(r, ui.id().with(("toggle", param.name())), Sense::click());
    let on = param.value();
    if resp.clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, !on);
        setter.end_set_parameter(param);
    }
    let p = ui.painter();
    let rad = r.height() / 2.0;
    boxed(p, r, if on { alpha(col::CYAN, 0.15) } else { Color32::from_rgb(0x0f, 0x0e, 0x11) }, if on { col::CYAN } else { col::KNOB_RIM }, rad);
    let cx = if on { r.max.x - rad } else { r.min.x + rad };
    if on {
        glow_dot(p, pos2(cx, r.center().y), rad - 4.0, col::CYAN);
    } else {
        p.circle_filled(pos2(cx, r.center().y), rad - 4.0, col::DIM);
    }
}

/// `◄ value ►` stepper for an int parameter.
pub fn stepper(ui: &mut Ui, setter: &ParamSetter, param: &IntParam, r: Rect, shown: &str) {
    boxed(ui.painter(), r, col::SCREEN, col::LINE, 5.0);
    let bw = r.height();
    let left = Rect::from_min_size(r.min, vec2(bw, r.height()));
    let right = Rect::from_min_size(pos2(r.max.x - bw, r.min.y), vec2(bw, r.height()));
    for (rect, dir, glyph) in [(left, -1, "◀"), (right, 1, "▶")] {
        let resp = ui.interact(rect, ui.id().with(("step", param.name(), dir)), Sense::click());
        let c = if resp.hovered() { col::CYAN } else { col::MUTED };
        text(ui.painter(), rect.center(), Align2::CENTER_CENTER, glyph, 10.0, c, false);
        if resp.clicked() {
            let (lo, hi) = (param.preview_plain(0.0), param.preview_plain(1.0));
            let mut v = param.value() + dir;
            if v > hi {
                v = lo;
            }
            if v < lo {
                v = hi;
            }
            setter.begin_set_parameter(param);
            setter.set_parameter(param, v);
            setter.end_set_parameter(param);
        }
    }
    text(ui.painter(), r.center(), Align2::CENTER_CENTER, shown, 12.0, col::AMBER, true);
}

/// Plain action button.
pub fn button(ui: &mut Ui, r: Rect, s: &str, color: Color32, active: bool) -> bool {
    let resp = ui.interact(r, ui.id().with(("btn", s, r.min.x as i32, r.min.y as i32)), Sense::click());
    let p = ui.painter();
    let border = if active { color } else if resp.hovered() { col::MUTED } else { col::LINE };
    boxed(p, r, if active { alpha(color, 0.10) } else { col::BODY }, border, 6.0);
    text(p, r.center(), Align2::CENTER_CENTER, s, 11.0, if active { color } else { col::CREAM }, false);
    resp.clicked()
}

pub fn screw(p: &Painter, c: Pos2) {
    p.circle_filled(c, 3.5, Color32::from_rgb(0x8a, 0x7a, 0x62));
    p.circle_stroke(c, 3.5, Stroke::new(1.0_f32, Color32::from_rgb(0x5a, 0x4c, 0x38)));
    p.line_segment([c + vec2(-2.4, -1.4), c + vec2(2.4, 1.4)], Stroke::new(1.0_f32, Color32::from_rgb(0x4a, 0x3d, 0x2c)));
}

pub fn wood(p: &Painter, r: Rect) {
    p.rect_filled(r, 0.0_f32, col::WOOD);
    let mut x = r.min.x + 1.0;
    while x < r.max.x {
        p.line_segment([pos2(x, r.min.y), pos2(x, r.max.y)], Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(0, 0, 0, 46)));
        x += 4.0;
    }
}

/// Convenience to keep panels' coordinates relative to the window origin.
#[derive(Clone, Copy)]
pub struct Origin(pub Pos2);

impl Origin {
    pub fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(self.0 + vec2(x, y), vec2(w, h))
    }
    pub fn pos(&self, x: f32, y: f32) -> Pos2 {
        self.0 + vec2(x, y)
    }
}

