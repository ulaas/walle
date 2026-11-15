use egui::{Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, pos2, vec2};

use super::{
    Action, App,
    grid::{status_color, status_icon},
    theme::{ph, semibold, semibold_font, with_icon},
};
use crate::model::Status;

const TOOLBAR_HEIGHT: f32 = 52.0;

/// A button for the dark viewer chrome, which stays dark in the light theme as well.
fn chrome_button(ui: &mut egui::Ui, label: &str, fill: Option<Color32>, text: Color32) -> egui::Response {
    ui.add(
        egui::Button::new(semibold(label).color(text))
            .fill(fill.unwrap_or(Color32::from_white_alpha(26)))
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(8))
            .min_size(vec2(0.0, 30.0)),
    )
}

/// How far a zoomed image may move: it can't be dragged out of sight.
fn clamp_pan(pan: egui::Vec2, image: egui::Vec2, area: egui::Vec2) -> egui::Vec2 {
    let limit = ((image - area) / 2.0).max(egui::Vec2::ZERO);
    vec2(pan.x.clamp(-limit.x, limit.x), pan.y.clamp(-limit.y, limit.y))
}

impl App {
    pub(super) fn lightbox_ui(&mut self, ctx: &egui::Context) {
        let Some(wp) = self.lightbox.clone() else { return };
        let p = self.pal;
        let screen = Self::below_title_bar(ctx);
        let key = wp.key();
        let entry = self.entry(&key);
        let mut close = false;
        let mut nav: isize = 0;

        egui::Area::new(egui::Id::unique("walle-lightbox"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
                let painter = ui.painter().clone();
                painter.rect_filled(rect, 0.0, Color32::from_rgb(8, 9, 11));

                let full = self.textures.peek(&format!("full:{key}"));
                let tex = full
                    .clone()
                    .or_else(|| self.textures.peek(&format!("prev:{key}")))
                    .or_else(|| self.textures.peek(&format!("thumb:{key}")));

                // Zoom (scroll) around the pointer, pan by dragging, double-click to reset.
                let area = rect.shrink2(vec2(24.0, TOOLBAR_HEIGHT + 8.0));
                if resp.contains_pointer() {
                    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                    if scroll != 0.0 {
                        let old = self.lb_zoom;
                        self.lb_zoom = (self.lb_zoom * (1.0 + scroll * 0.0015)).clamp(1.0, 8.0);
                        if let Some(ptr) = ui.input(|i| i.pointer.hover_pos()) {
                            let rel = ptr - area.center() - self.lb_pan;
                            self.lb_pan -= rel * (self.lb_zoom / old - 1.0);
                        }
                    }
                }
                if resp.dragged() && self.lb_zoom > 1.0 {
                    self.lb_pan += resp.drag_delta();
                }
                if resp.double_clicked() {
                    self.lb_zoom = if self.lb_zoom > 1.0 { 1.0 } else { 2.5 };
                    self.lb_pan = egui::Vec2::ZERO;
                }
                if self.lb_zoom <= 1.0 {
                    self.lb_pan = egui::Vec2::ZERO;
                }

                let mut image_rect = None;
                if let Some(t) = &tex {
                    let size = t.size_vec2();
                    let fit = (area.width() / size.x).min(area.height() / size.y);
                    let shown = size * fit * self.lb_zoom;
                    self.lb_pan = clamp_pan(self.lb_pan, shown, rect.size());
                    let img_rect = Rect::from_center_size(area.center() + self.lb_pan, shown);
                    image_rect = Some(img_rect);
                    painter.with_clip_rect(rect).image(t.id(), img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                if full.is_none() {
                    let failed = self.textures.has_failed(&format!("full:{key}"));
                    let msg = if failed { "Could not load the full image" } else { "Loading full resolution…" };
                    let r = Align2::CENTER_BOTTOM.anchor_size(rect.center_bottom() - vec2(0.0, 18.0), vec2(240.0, 28.0));
                    painter.rect_filled(r, CornerRadius::same(255), Color32::from_black_alpha(190));
                    painter.text(r.center(), Align2::CENTER_CENTER, msg, FontId::proportional(13.0), Color32::LIGHT_GRAY);
                }

                // Previous / next.
                for (dir, x, icon) in [(-1isize, rect.left() + 36.0, ph::CARET_LEFT), (1, rect.right() - 36.0, ph::CARET_RIGHT)] {
                    let r = Rect::from_center_size(pos2(x, rect.center().y), vec2(48.0, 48.0));
                    let br = ui.interact(r, ui.scope_id().with(("lb_nav", dir)), Sense::click());
                    let a = if br.hovered() { 210 } else { 120 };
                    painter.circle_filled(r.center(), 22.0, Color32::from_black_alpha(a));
                    painter.text(r.center(), Align2::CENTER_CENTER, icon, FontId::proportional(20.0), Color32::WHITE);
                    if br.clicked() {
                        nav = dir;
                    }
                }

                // Toolbar.
                let bar = Rect::from_min_size(rect.min, vec2(rect.width(), TOOLBAR_HEIGHT));
                painter.rect_filled(bar, 0.0, Color32::from_black_alpha(165));
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(16.0, 10.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
                child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let hover_close = ui.add(
                        egui::Button::new(RichText::new(ph::X).size(16.0).color(Color32::WHITE))
                            .frame_when_inactive(false)
                            .min_size(egui::Vec2::splat(30.0)),
                    );
                    if hover_close.on_hover_text("Close (Esc)").clicked() {
                        close = true;
                    }
                    ui.add_space(8.0);
                    if chrome_button(ui, &with_icon(ph::MONITOR, "Set wallpaper"), Some(p.accent), p.on_accent).clicked() {
                        self.actions.push(Action::SetWallpaper(wp.clone()));
                    }
                    for st in [Status::Rejected, Status::Kept, Status::Considering] {
                        let name = match st {
                            Status::Rejected => "Reject",
                            Status::Kept => "Keep",
                            _ => "Consider",
                        };
                        let on = entry.status == st;
                        if chrome_button(ui, &with_icon(status_icon(st), name), on.then_some(status_color(st)), Color32::WHITE).clicked() {
                            self.actions.push(Action::SetStatus(wp.clone(), st));
                        }
                    }
                    ui.add_space(12.0);
                    // What remains on the left: the name, cut to fit, and the details.
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        let mut info = format!("{}  ·  {}", self.provider_name(&wp.provider), wp.resolution());
                        if entry.rating > 0 {
                            info += &format!("  ·  {} {}", ph::STAR, entry.rating);
                        }
                        if self.lb_zoom > 1.0 {
                            info += &format!("  ·  {:.0}%", self.lb_zoom * 100.0);
                        }
                        let info = ui.painter().layout_no_wrap(info, FontId::proportional(13.5), Color32::from_gray(185));
                        let title_w = (ui.available_width() - info.size().x - 16.0).max(40.0);
                        let title = super::theme::truncated(ui.painter(), wp.display_title(), semibold_font(15.0), Color32::WHITE, title_w);
                        let (r, _) = ui.allocate_exact_size(title.size(), Sense::hover());
                        ui.painter().galley(r.min, title, Color32::WHITE);
                        ui.add_space(8.0);
                        let (r, _) = ui.allocate_exact_size(info.size(), Sense::hover());
                        ui.painter().galley(r.min, info, Color32::from_gray(185));
                    });
                });

                // Clicking the backdrop (not the image or the toolbar) closes.
                if resp.clicked() && !resp.double_clicked() && self.lb_zoom <= 1.0 {
                    let on_something = resp.interact_pointer_pos().is_some_and(|pos| bar.contains(pos) || image_rect.is_some_and(|r| r.contains(pos)));
                    if !on_something {
                        close = true;
                    }
                }
            });

        if close {
            self.lightbox = None;
        } else if nav != 0 {
            self.move_selection(nav);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panning_keeps_the_image_in_view() {
        let area = vec2(1000.0, 800.0);
        // Twice the view: can move half the view each way, no further.
        assert_eq!(clamp_pan(vec2(5000.0, -5000.0), vec2(2000.0, 1600.0), area), vec2(500.0, -400.0));
        assert_eq!(clamp_pan(vec2(100.0, 100.0), vec2(2000.0, 1600.0), area), vec2(100.0, 100.0));
        // Smaller than the view in one direction: stays centred in that one.
        assert_eq!(clamp_pan(vec2(100.0, 100.0), vec2(2000.0, 600.0), area), vec2(100.0, 0.0));
    }
}
