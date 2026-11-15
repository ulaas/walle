use egui::{Align2, Color32, CornerRadius, FontId, Mesh, Rect, RichText, Sense, Shadow, Stroke, Vec2, pos2, vec2};

use super::{
    Action, App, View,
    theme::{self, ph, semibold, semibold_font, with_icon},
};
use crate::{
    model::{Status, Wallpaper},
    worker::Command,
};

const SPACING: f32 = 12.0;
const RADIUS: u8 = 10;
/// Diameter of a quick action on a card, and the distance to the next one.
const QUICK_SIZE: f32 = 26.0;
const QUICK_STEP: f32 = 30.0;
const QUICK_COUNT: usize = 4;

pub fn status_color(s: Status) -> Color32 {
    match s {
        Status::None => Color32::GRAY,
        Status::Considering => Color32::from_rgb(240, 180, 40),
        Status::Kept => Color32::from_rgb(70, 195, 110),
        Status::Rejected => Color32::from_rgb(235, 85, 85),
    }
}

pub fn status_icon(s: Status) -> &'static str {
    match s {
        Status::None => "",
        Status::Considering => ph::PUSH_PIN,
        Status::Kept => ph::CHECK,
        Status::Rejected => ph::X,
    }
}

/// UV rect that crops an image of `img` size to cover a cell of `cell` size.
pub fn cover_uv(img: Vec2, cell: Vec2) -> Rect {
    let ia = img.x / img.y.max(1.0);
    let ca = cell.x / cell.y.max(1.0);
    if ia > ca {
        let f = ca / ia;
        Rect::from_min_max(pos2((1.0 - f) / 2.0, 0.0), pos2((1.0 + f) / 2.0, 1.0))
    } else {
        let f = ia / ca;
        Rect::from_min_max(pos2(0.0, (1.0 - f) / 2.0), pos2(1.0, (1.0 + f) / 2.0))
    }
}

/// Text on a translucent dark pill (readable on any image). `None` if it would pass `max_right`.
fn overlay_pill(painter: &egui::Painter, anchor: Align2, pos: egui::Pos2, text: String, color: Color32, size: f32, max_right: f32) -> Option<Rect> {
    let galley = painter.layout_no_wrap(text, semibold_font(size), color);
    let r = anchor.anchor_size(pos, galley.size() + vec2(12.0, 6.0));
    if r.right() > max_right {
        return None;
    }
    painter.rect_filled(r, CornerRadius::same(255), Color32::from_black_alpha(175));
    painter.galley(r.min + vec2(6.0, 3.0), galley, color);
    Some(r)
}

/// A dark fade towards the bottom edge of a card, following its rounded corners.
fn bottom_scrim(painter: &egui::Painter, card: Rect, height: f32, radius: f32, strength: f32) {
    let top = card.bottom() - height;
    let corner_top = card.bottom() - radius;
    let mut mesh = Mesh::default();
    let mut add_row = |y: f32| {
        let dy = (y - corner_top).clamp(0.0, radius);
        let inset = radius - (radius * radius - dy * dy).sqrt();
        let t = ((y - top) / height).clamp(0.0, 1.0);
        let color = Color32::from_black_alpha((t * t * 215.0 * strength) as u8);
        let i = mesh.vertices.len() as u32;
        mesh.colored_vertex(pos2(card.left() + inset, y), color);
        mesh.colored_vertex(pos2(card.right() - inset, y), color);
        if i >= 2 {
            mesh.add_triangle(i - 2, i - 1, i);
            mesh.add_triangle(i - 1, i, i + 1);
        }
    };
    const BODY_ROWS: usize = 8;
    const CORNER_ROWS: usize = 6;
    for r in 0..BODY_ROWS {
        add_row(top + (corner_top - top) * r as f32 / BODY_ROWS as f32);
    }
    for r in 0..=CORNER_ROWS {
        add_row(corner_top + radius * r as f32 / CORNER_ROWS as f32);
    }
    painter.add(mesh);
}

impl App {
    fn empty_state(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        let no_sources = self.view == View::Browse && self.enabled.is_empty();
        let failed = self.view == View::Browse && self.search_error.is_some();
        let (icon, title, hint) = if self.loading {
            ("", "Loading…", String::new())
        } else if no_sources {
            (ph::PLUGS, "No sources enabled", "Turn on a source in Settings to start browsing.".to_string())
        } else if failed {
            (ph::WARNING, "Search failed", self.search_error.clone().unwrap_or_default())
        } else {
            let (icon, title, hint) = match self.view {
                View::Browse => (ph::MAGNIFYING_GLASS, "No results", "Try another search, source or fewer filters."),
                View::Considering => (ph::PUSH_PIN, "Your shortlist is empty", "Press C on a wallpaper to consider it."),
                View::Library => (ph::IMAGES, "Nothing here yet", "Press K to keep wallpapers, or drop images and folders onto the window."),
                View::Rejected => (ph::TRASH, "Nothing rejected", "Rejected wallpapers are hidden from future searches."),
                _ => (ph::CLOCK_COUNTER_CLOCKWISE, "No history yet", "Wallpapers you set will show up here."),
            };
            (icon, title, hint.to_string())
        };
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.26);
            if self.loading {
                ui.add(egui::Spinner::new().size(30.0).color(p.accent));
            } else {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(76.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 38.0, p.surface2);
                ui.painter().text(r.center(), Align2::CENTER_CENTER, icon, FontId::proportional(34.0), p.weak);
            }
            ui.add_space(10.0);
            ui.label(semibold(title).size(17.0));
            if !hint.is_empty() {
                ui.add_space(2.0);
                ui.add(egui::Label::new(RichText::new(hint).color(p.weak)).wrap());
            }
            if !self.loading && no_sources {
                ui.add_space(10.0);
                if theme::primary_button(ui, &p, "Open Settings").clicked() {
                    self.actions.push(Action::SwitchView(View::Settings));
                }
            } else if !self.loading && failed {
                ui.add_space(10.0);
                if theme::primary_button(ui, &p, with_icon(ph::ARROW_CLOCKWISE, "Try again")).clicked() {
                    self.actions.push(Action::NewSearch);
                }
            }
        });
    }

    pub(super) fn grid_ui(&mut self, ui: &mut egui::Ui) {
        self.refresh_visible();
        let items = self.visible.clone();

        if items.is_empty() {
            // A whole page can be filtered out (all rejected or duplicates): keep going.
            if self.view == View::Browse && self.has_more && !self.loading {
                self.actions.push(Action::LoadMore);
            }
            self.empty_state(ui);
            return;
        }

        ui.spacing_mut().item_spacing = vec2(SPACING, SPACING);
        let avail = ui.available_width() - 12.0; // room for the scrollbar
        let target = self.config.thumb_size.clamp(120.0, 600.0);
        let cols = (((avail + SPACING) / (target + SPACING)).floor() as usize).max(1);
        let cell_w = ((avail - SPACING * (cols as f32 - 1.0)) / cols as f32).floor();
        let cell = vec2(cell_w, (cell_w * 10.0 / 16.0).round());
        let rows = items.len().div_ceil(cols);
        self.grid_cols = cols;

        let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt(("grid", self.view));
        if std::mem::take(&mut self.scroll_to_selected)
            && let Some(sel) = self.selected.as_ref().map(|s| s.key())
            && let Some(i) = self.visible_order.iter().position(|k| *k == sel)
        {
            let row_h = cell.y + SPACING;
            let view_h = ui.available_height();
            let top = (i / cols) as f32 * row_h;
            let cur = self.grid_scroll;
            if top < cur || top + row_h > cur + view_h {
                area = area.vertical_scroll_offset((top - view_h / 2.0 + row_h / 2.0).max(0.0));
            }
        }

        let selected_key = self.selected.as_ref().map(|w| w.key());
        let out = area.show_rows(ui, cell.y, rows, |ui, range| {
            for row in range.clone() {
                ui.horizontal(|ui| {
                    for col in 0..cols {
                        let Some(wp) = items.get(row * cols + col) else { break };
                        let key = wp.key();
                        let sel = selected_key.as_deref() == Some(key.as_str());
                        let multi = self.multi.contains(&key);
                        self.thumb_cell(ui, wp, key, cell, sel, multi);
                    }
                });
            }
            if range.end == rows {
                self.grid_footer(ui);
            }
            range
        });
        self.grid_scroll = out.state.clamped_offset().y;
        let range = out.inner;
        if self.view == View::Browse && self.has_more && !self.loading && self.search_error.is_none() && range.end + 2 >= rows {
            self.actions.push(Action::LoadMore);
        }
    }

    /// Below the last row: more is coming, or a page failed and can be retried.
    fn grid_footer(&mut self, ui: &mut egui::Ui) {
        if self.view != View::Browse {
            return;
        }
        let p = self.pal;
        if self.loading {
            ui.vertical_centered(|ui| {
                ui.add_space(4.0);
                ui.add(egui::Spinner::new().size(20.0).color(p.accent));
            });
        } else if self.search_error.is_some() {
            ui.vertical_centered(|ui| {
                ui.add_space(4.0);
                ui.label(RichText::new("Could not load more").color(p.weak));
                if ui.button(with_icon(ph::ARROW_CLOCKWISE, "Try again")).clicked() {
                    self.search_error = None;
                    self.has_more = true;
                    self.actions.push(Action::LoadMore);
                }
            });
        }
    }

    fn thumb_cell(&mut self, ui: &mut egui::Ui, wp: &Wallpaper, key: String, size: Vec2, selected: bool, in_multi: bool) {
        let p = self.pal;
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let mut quick: Option<Action> = None;
        let entry = self.entry(&key);

        if ui.is_rect_visible(rect) {
            let tex_key = format!("thumb:{key}");
            let worker = &self.worker;
            let fallback = entry.local_path.clone();
            let tex = self.textures.get(&tex_key, || {
                worker.send(Command::LoadImage { tex_key: tex_key.clone(), url: wp.thumb_url.clone(), max_dim: 720, fallback })
            });
            let ctx = ui.ctx().clone();
            let id = ui.scope_id().with(("card", &key));
            // `contains_pointer`: over a quick action the card is not "hovered", which would
            // fade the actions out from under the pointer.
            let pointer_on_card = resp.contains_pointer();
            let hover_t = ctx.animate_bool_with_time(id.with("hover"), pointer_on_card, 0.12);
            let shown = ctx.animate_bool_with_time(id.with("fade"), tex.is_some(), 0.25);
            let radius = CornerRadius::same(RADIUS);
            let painter = ui.painter().clone();

            if hover_t > 0.0 {
                let shadow = Shadow { offset: [0, 8], blur: 22, spread: 0, color: Color32::from_black_alpha((120.0 * hover_t) as u8) };
                painter.add(shadow.as_shape(rect, radius));
            }
            painter.rect_filled(rect, radius, p.surface2);

            match &tex {
                Some(t) => {
                    let dim = if entry.status == Status::Rejected && self.view == View::Browse { 0.35 } else { 1.0 };
                    let grow = 1.0 + 0.025 * hover_t;
                    // Zoom slightly into the image on hover by shrinking the UV window.
                    let uv = cover_uv(t.size_vec2(), rect.size());
                    let uv = Rect::from_center_size(uv.center(), uv.size() / grow);
                    egui::Image::from_texture((t.id(), t.size_vec2()))
                        .uv(uv)
                        .corner_radius(radius)
                        .tint(Color32::WHITE.gamma_multiply(shown * dim))
                        .paint_at(ui, rect);
                }
                None if self.textures.has_failed(&tex_key) => {
                    painter.text(rect.center(), Align2::CENTER_CENTER, ph::IMAGE_BROKEN, FontId::proportional(26.0), p.weak);
                }
                None => {
                    let t = ctx.input(|i| i.time) as f32;
                    let phase = (rect.left() + rect.top()) * 0.004;
                    let pulse = 0.5 + 0.5 * (t * 3.0 + phase).sin();
                    painter.rect_filled(rect, radius, p.hover.gamma_multiply(0.3 + 0.4 * pulse));
                    ctx.request_repaint_after(std::time::Duration::from_millis(40));
                }
            }

            // Title and resolution over a dark fade, on hover.
            if hover_t > 0.0 {
                bottom_scrim(&painter, rect, (rect.height() * 0.5).min(72.0), f32::from(RADIUS), hover_t);
                let white = Color32::WHITE.gamma_multiply(hover_t);
                let dim_white = Color32::from_gray(205).gamma_multiply(hover_t);
                let res = if entry.local_path.is_some() { format!("{}  {}", ph::DOWNLOAD_SIMPLE, wp.resolution()) } else { wp.resolution() };
                let res = painter.layout_no_wrap(res, FontId::proportional(11.5), dim_white);
                let baseline = rect.bottom() - 10.0;
                let res_pos = pos2(rect.right() - 10.0 - res.size().x, baseline - res.size().y);
                let res_w = res.size().x;
                painter.galley(res_pos, res, dim_white);
                let title = theme::truncated(&painter, wp.display_title(), semibold_font(12.5), white, rect.width() - res_w - 32.0);
                painter.galley(pos2(rect.left() + 10.0, baseline - title.size().y), title, white);
            }

            // Status pills (top-left) and the multi-select check. While the quick actions show,
            // pills that would run under them make way.
            let pills_end = if hover_t > 0.0 { rect.right() - 8.0 - QUICK_STEP * QUICK_COUNT as f32 } else { rect.right() - 8.0 };
            let mut top_left = rect.left_top() + vec2(8.0, 8.0);
            if in_multi || (!self.multi.is_empty() && pointer_on_card) {
                let c = Rect::from_min_size(top_left, vec2(22.0, 22.0));
                painter.circle_filled(c.center(), 11.0, if in_multi { p.accent } else { Color32::from_black_alpha(150) });
                painter.circle_stroke(c.center(), 11.0, Stroke::new(1.5, Color32::WHITE));
                if in_multi {
                    painter.text(c.center(), Align2::CENTER_CENTER, ph::CHECK, FontId::proportional(13.0), p.on_accent);
                }
                top_left.x += 28.0;
            }
            if entry.status != Status::None {
                let label = format!("{} {}", status_icon(entry.status), entry.status.label().to_uppercase());
                if let Some(r) = overlay_pill(&painter, Align2::LEFT_TOP, top_left, label, status_color(entry.status), 10.5, pills_end) {
                    top_left.x = r.right() + 6.0;
                }
            }
            if self.current_keys.contains(&key) {
                // Which display, when they do not all show the same.
                let on: Vec<&str> = if self.current_keys.len() > 1 {
                    self.displays.iter().filter(|d| self.shown_on(&d.name) == Some(key.as_str())).map(|d| d.name.as_str()).collect()
                } else {
                    vec![]
                };
                let label = if on.is_empty() { format!("{} ON DESKTOP", ph::MONITOR) } else { format!("{} ON {}", ph::MONITOR, on.join(", ")) };
                if let Some(r) = overlay_pill(&painter, Align2::LEFT_TOP, top_left, label, p.accent, 10.5, pills_end) {
                    top_left.x = r.right() + 6.0;
                }
            }
            if self.view == View::Browse
                && entry.added_at == 0
                && let Some(Some((_, st))) = self.dup_of.get(&key)
            {
                let label = match st {
                    Status::Kept => "DUPLICATE OF KEPT",
                    Status::Rejected => "LOOKS LIKE REJECTED",
                    Status::Considering => "ON SHORTLIST",
                    Status::None => "IN LIBRARY",
                };
                overlay_pill(&painter, Align2::LEFT_TOP, top_left, label.into(), status_color(*st), 10.5, pills_end);
            }

            // Rating and tags (bottom-left); above the title while that shows.
            if entry.rating > 0 || !entry.tags.is_empty() {
                let mut s = String::new();
                if entry.rating > 0 {
                    s += &format!("{} {}", ph::STAR, entry.rating);
                }
                if !entry.tags.is_empty() {
                    if !s.is_empty() {
                        s += "   ";
                    }
                    s += &format!("{} {}", ph::TAG, entry.tags.len());
                }
                let lift = if hover_t > 0.5 { 30.0 } else { 8.0 };
                overlay_pill(&painter, Align2::LEFT_BOTTOM, rect.left_bottom() + vec2(8.0, -lift), s, Color32::from_rgb(255, 210, 70), 11.0, rect.right());
            }

            // Quick actions (top-right) on hover.
            if pointer_on_card || hover_t > 0.0 {
                let actions: [(&str, &str, Option<Status>); QUICK_COUNT] = [
                    (ph::MONITOR, "Set as wallpaper (Enter)", None),
                    (ph::X, "Reject (X)", Some(Status::Rejected)),
                    (ph::CHECK, "Keep (K)", Some(Status::Kept)),
                    (ph::PUSH_PIN, "Consider (C)", Some(Status::Considering)),
                ];
                let mut x = rect.right() - 8.0;
                for (i, (icon, tip, status)) in actions.into_iter().enumerate() {
                    let r = Rect::from_min_size(pos2(x - QUICK_SIZE, rect.top() + 8.0), Vec2::splat(QUICK_SIZE));
                    x -= QUICK_STEP;
                    let br = ui.interact(r, id.with(("quick", i)), Sense::click());
                    let active = status.is_some_and(|s| s == entry.status);
                    let fill = if br.hovered() {
                        p.accent
                    } else if active {
                        status_color(entry.status)
                    } else {
                        Color32::from_black_alpha(170)
                    };
                    let on_fill = if br.hovered() { p.on_accent } else { Color32::WHITE };
                    painter.circle_filled(r.center(), QUICK_SIZE / 2.0, fill.gamma_multiply(hover_t.max(0.2)));
                    painter.text(r.center(), Align2::CENTER_CENTER, icon, FontId::proportional(14.0), on_fill.gamma_multiply(hover_t));
                    if br.on_hover_text(tip).clicked() {
                        quick = Some(match status {
                            Some(s) => Action::SetStatus(wp.clone(), s),
                            None => Action::SetWallpaper(wp.clone()),
                        });
                    }
                }
            }

            if selected || in_multi {
                painter.rect_stroke(rect, radius, Stroke::new(2.5, p.accent), egui::StrokeKind::Inside);
            } else if hover_t > 0.0 {
                painter.rect_stroke(rect, radius, Stroke::new(1.0, Color32::WHITE.gamma_multiply(0.22 * hover_t)), egui::StrokeKind::Inside);
            }
        }

        if let Some(a) = quick {
            self.actions.push(a);
        } else if resp.double_clicked() {
            self.actions.push(Action::Lightbox(wp.clone()));
        } else if resp.clicked() {
            let mods = ui.input(|i| i.modifiers);
            self.actions.push(if mods.command {
                Action::ToggleMulti(wp.clone())
            } else if mods.shift {
                Action::RangeSelect(wp.clone())
            } else {
                Action::Select(wp.clone())
            });
        }

        let mut menu_actions = vec![];
        let similar = self.info(&wp.provider).is_some_and(|i| i.caps.similar);
        resp.context_menu(|ui| {
            ui.set_min_width(200.0);
            let mut item = |ui: &mut egui::Ui, icon: &str, label: &str, a: Action| {
                if ui.add(egui::Button::new(with_icon(icon, label)).frame_when_inactive(false)).clicked() {
                    menu_actions.push(a);
                    ui.close();
                }
            };
            let everywhere = if self.displays.len() > 1 { "Set as wallpaper on all displays" } else { "Set as wallpaper" };
            item(ui, ph::MONITOR, everywhere, Action::SetWallpaper(wp.clone()));
            if self.displays.len() > 1 {
                for d in &self.displays {
                    item(ui, ph::MONITOR, &format!("Set on {} only", d.name), Action::SetWallpaperOn(wp.clone(), d.name.clone()));
                }
            }
            item(ui, ph::ARROWS_OUT, "Preview", Action::Lightbox(wp.clone()));
            ui.separator();
            item(ui, ph::PUSH_PIN, "Consider", Action::SetStatus(wp.clone(), Status::Considering));
            item(ui, ph::CHECK, "Keep", Action::SetStatus(wp.clone(), Status::Kept));
            item(ui, ph::X, "Reject", Action::SetStatus(wp.clone(), Status::Rejected));
            ui.separator();
            if !wp.is_local() {
                item(ui, ph::DOWNLOAD_SIMPLE, "Download", Action::Download(wp.clone()));
            }
            if let Some(path) = &entry.local_path {
                item(ui, ph::FOLDER_OPEN, "Show file", Action::Reveal(path.clone()));
            }
            if let Some(url) = wp.page_url.clone() {
                item(ui, ph::GLOBE, "Open page", Action::OpenUrl(url));
            }
            if similar {
                item(ui, ph::APPROXIMATE_EQUALS, "Find similar", Action::Similar(wp.clone()));
            }
            if entry.added_at > 0 {
                ui.separator();
                item(ui, ph::TRASH, "Remove from library", Action::Forget(wp.clone()));
            }
        });
        self.actions.extend(menu_actions);
    }
}
