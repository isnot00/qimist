//! Header preset browser: ◀ [name ▾] ▶ SAVE, with a drop-down list and a save box.

use super::widgets::*;
use crate::params::VoxParams;
use crate::presets::{self as store, Preset};
use nih_plug::prelude::ParamSetter;
use nih_plug_egui::egui::{self, pos2, vec2, Align2, Color32, Key, Painter, Rect, Sense, Stroke, Ui};

#[derive(Default)]
pub struct Browser {
    list: Vec<Preset>,
    loaded: bool,
    open: bool,
    saving: bool,
    save_name: String,
    status: String,
}

impl Browser {
    fn reload(&mut self, params: &VoxParams) {
        self.list = store::list(params);
        self.loaded = true;
    }
}

fn load(params: &VoxParams, setter: &ParamSetter, p: &Preset) {
    store::apply(params, setter.raw_context, p);
    *params.preset_name.write() = p.name.clone();
}

pub fn bar(ui: &mut Ui, p: &Painter, r: Rect, setter: &ParamSetter, params: &VoxParams, b: &mut Browser) {
    if !b.loaded {
        b.reload(params);
    }
    let name = params.preset_name.read().clone();
    let cur = b.list.iter().position(|x| x.name == name);
    let modified = cur.is_some_and(|i| store::is_modified(params, &b.list[i]));

    let prev = Rect::from_min_size(r.min, vec2(28.0, r.height()));
    let save = Rect::from_min_size(pos2(r.max.x - 56.0, r.min.y), vec2(56.0, r.height()));
    let next = Rect::from_min_size(pos2(save.min.x - 34.0, r.min.y), vec2(28.0, r.height()));
    let field = Rect::from_min_max(pos2(prev.max.x + 6.0, r.min.y), pos2(next.min.x - 6.0, r.max.y));

    let step = |d: i32, b: &mut Browser| {
        if b.list.is_empty() {
            return;
        }
        let n = b.list.len() as i32;
        let i = cur.map(|i| (i as i32 + d).rem_euclid(n)).unwrap_or(0) as usize;
        load(params, setter, &b.list[i]);
        b.open = false;
    };
    if button(ui, prev, "◀", col::CYAN, false) {
        step(-1, b);
    }
    if button(ui, next, "▶", col::CYAN, false) {
        step(1, b);
    }

    // Name field (click to open the list).
    let resp = ui.interact(field, ui.id().with("preset-field"), Sense::click());
    if modified && !b.open {
        if let Some(i) = cur {
            let changed = store::modified_params(params, &b.list[i]);
            let more = if changed.len() > 8 { format!(" +{} more", changed.len() - 8) } else { String::new() };
            resp.clone().on_hover_text(format!("Changed: {}{more}", changed.iter().take(8).cloned().collect::<Vec<_>>().join(", ")));
        }
    }
    if resp.clicked() {
        b.open = !b.open;
        b.saving = false;
        if b.open {
            b.reload(params);
        }
    }
    let border = if b.open { col::CYAN } else if resp.hovered() { col::MUTED } else { col::LINE };
    boxed(p, field, col::SCREEN, border, 6.0);
    text(p, pos2(field.min.x + 10.0, field.min.y + 11.0), Align2::LEFT_CENTER, "PRESET", 8.0, col::DIM, false);
    let shown = if modified { format!("{name} *") } else { name.clone() };
    text(p, pos2(field.min.x + 10.0, field.min.y + 24.0), Align2::LEFT_CENTER, shown, 12.0, col::CREAM, false);
    let cat = cur.map(|i| b.list[i].category.to_uppercase()).unwrap_or_default();
    text(p, pos2(field.max.x - 22.0, field.center().y), Align2::RIGHT_CENTER, cat, 8.0, col::AMBER, true);
    // Drawn arrow (the UI font has no ▼ glyph).
    let c = pos2(field.max.x - 11.0, field.center().y);
    let (a, d) = if b.open { (-3.0, 3.0) } else { (3.0, -3.0) };
    p.add(egui::Shape::line(vec![pos2(c.x - 4.0, c.y - d), pos2(c.x, c.y + a), pos2(c.x + 4.0, c.y - d)], Stroke::new(1.5_f32, col::MUTED)));

    if button(ui, save, "SAVE", col::AMBER, b.saving) {
        b.saving = !b.saving;
        b.open = false;
        b.status.clear();
        b.save_name = if cur.is_some_and(|i| b.list[i].factory) { format!("{name} (mine)") } else { name.clone() };
    }

    if b.open {
        list_overlay(ui, field, params, setter, b, &name);
    }
    if b.saving {
        save_overlay(ui, r, params, b);
    }
}

fn list_overlay(ui: &mut Ui, field: Rect, params: &VoxParams, setter: &ParamSetter, b: &mut Browser, current: &str) {
    let w = field.width() + 70.0;
    let mut pick: Option<usize> = None;
    let mut delete: Option<String> = None;
    let area = egui::Area::new(egui::Id::new("vf-preset-list"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(field.min.x - 34.0, field.max.y + 6.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::default()
                .fill(col::PANEL)
                .stroke(Stroke::new(1.0_f32, col::CYAN))
                .corner_radius(8.0)
                .inner_margin(6.0)
                .show(ui, |ui| {
                    ui.set_width(w);
                    egui::ScrollArea::vertical().max_height(470.0).show(ui, |ui| {
                        let mut last_cat = String::new();
                        for (i, pr) in b.list.iter().enumerate() {
                            let cat = if pr.factory { pr.category.clone() } else { "My Presets".to_string() };
                            if cat != last_cat {
                                let (rr, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
                                text(ui.painter(), pos2(rr.min.x + 8.0, rr.center().y + 3.0), Align2::LEFT_CENTER, cat.to_uppercase(), 9.0, col::AMBER, false);
                                last_cat = cat;
                            }
                            let (rr, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
                            let pp = ui.painter();
                            let active = pr.name == current;
                            let del = Rect::from_min_size(pos2(rr.max.x - 40.0, rr.min.y + 4.0), vec2(34.0, 18.0));
                            let over_del = !pr.factory && resp.hover_pos().is_some_and(|q| del.contains(q));
                            if active {
                                pp.rect_filled(rr, 5.0, alpha(col::CYAN, 0.12));
                            } else if resp.hovered() {
                                pp.rect_filled(rr, 5.0, Color32::from_rgb(0x26, 0x22, 0x29));
                            }
                            text(pp, pos2(rr.min.x + 16.0, rr.center().y), Align2::LEFT_CENTER, &pr.name, 12.0, if active { col::CYAN } else { col::CREAM }, false);
                            if !pr.factory {
                                boxed(pp, del, Color32::TRANSPARENT, if over_del { col::RED } else { col::LINE }, 4.0);
                                text(pp, del.center(), Align2::CENTER_CENTER, "DEL", 8.0, if over_del { col::RED } else { col::MUTED }, true);
                            }
                            if resp.clicked() {
                                if over_del {
                                    delete = Some(pr.name.clone());
                                } else {
                                    pick = Some(i);
                                }
                            }
                        }
                        if let Some(dir) = store::user_dir() {
                            let (rr, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
                            text(ui.painter(), pos2(rr.min.x + 8.0, rr.center().y), Align2::LEFT_CENTER, format!("saved in {}", dir.display()), 8.0, col::DIM, true);
                        }
                    });
                });
        });

    if let Some(i) = pick {
        let pr = b.list[i].clone();
        load(params, setter, &pr);
        b.open = false;
    }
    if let Some(name) = delete {
        let _ = store::delete(&name);
        b.reload(params);
    }
    // Click anywhere else closes the list.
    let clicked_outside = ui.ctx().input(|i| i.pointer.any_click() && i.pointer.interact_pos().is_some_and(|q| !area.response.rect.contains(q) && !field.contains(q)));
    if clicked_outside || ui.ctx().input(|i| i.key_pressed(Key::Escape)) {
        b.open = false;
    }
}

fn save_overlay(ui: &mut Ui, bar: Rect, params: &VoxParams, b: &mut Browser) {
    let mut done = false;
    egui::Area::new(egui::Id::new("vf-preset-save"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(bar.min.x, bar.max.y + 6.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::default().fill(col::PANEL).stroke(Stroke::new(1.0_f32, col::AMBER)).corner_radius(8.0).inner_margin(12.0).show(ui, |ui| {
                ui.set_width(bar.width() - 24.0);
                ui.label(egui::RichText::new("SAVE PRESET AS").color(col::AMBER).size(10.0));
                let edit = ui.add(egui::TextEdit::singleline(&mut b.save_name).desired_width(f32::INFINITY).hint_text("My sound"));
                if !edit.has_focus() && b.status.is_empty() {
                    edit.request_focus();
                }
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                let name = b.save_name.trim().to_string();
                let clash = b.list.iter().any(|x| x.factory && x.name.eq_ignore_ascii_case(&name));
                let exists = b.list.iter().any(|x| !x.factory && x.name.eq_ignore_ascii_case(&name));
                let note = if name.is_empty() {
                    "Type a name".to_string()
                } else if clash {
                    "That's a factory name — pick another".to_string()
                } else if exists {
                    "Will overwrite your preset".to_string()
                } else if !b.status.is_empty() {
                    b.status.clone()
                } else {
                    "Saves sound, FX, notes & drums (not mic setup)".to_string()
                };
                ui.label(egui::RichText::new(note).color(if clash { col::RED } else { col::MUTED }).size(10.0));
                ui.horizontal(|ui| {
                    let ok = !name.is_empty() && !clash;
                    let save = ui.add_enabled(ok, egui::Button::new(egui::RichText::new("SAVE").color(col::AMBER)));
                    if (save.clicked() || enter) && ok {
                        let preset = store::capture(params, &name);
                        match store::save(&preset) {
                            Ok(_) => {
                                *params.preset_name.write() = name.clone();
                                done = true;
                            }
                            Err(e) => b.status = format!("Save failed: {e}"),
                        }
                    }
                    if ui.button("CANCEL").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        done = true;
                    }
                });
            });
        });
    if done {
        b.saving = false;
        b.reload(params);
    }
}
