use egui::{Color32, CornerRadius, FontFamily, RichText, Sense, Stroke, vec2};

use super::{
    Action, App,
    grid::status_color,
    theme::{self, ICON_FILL, caption, ph, semibold, with_icon},
};
use crate::{model::Status, worker::Command};

pub fn parse_hex(s: &str) -> Option<Color32> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

impl App {
    pub(super) fn detail_ui(&mut self, ui: &mut egui::Ui) {
        let Some(wp) = self.selected.clone() else { return };
        let p = self.pal;
        let key = wp.key();
        let entry = self.entry(&key);
        // Long names, links and tags get cut to fit, unless they ask to wrap.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);

        // Header
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_max_width(ui.available_width() - 34.0);
                ui.add(egui::Label::new(semibold(wp.display_title()).size(15.5)).wrap());
                ui.horizontal(|ui| {
                    theme::pill(ui, p.accent, self.provider_name(&wp.provider));
                    theme::pill(ui, p.weak, wp.resolution());
                    if entry.status != Status::None {
                        theme::pill(ui, status_color(entry.status), entry.status.label());
                    }
                });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                if theme::icon_button(ui, ph::X, "Close (Esc)").clicked() {
                    self.selected = None;
                }
            });
        });
        ui.add_space(8.0);

        // Preview
        let w = ui.available_width();
        let aspect = if wp.width > 0 { wp.height as f32 / wp.width as f32 } else { 10.0 / 16.0 };
        let size = vec2(w, (w * aspect).clamp(w * 0.4, w * 1.4));
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let prev_key = format!("prev:{key}");
        let url = wp.preview_url.clone().unwrap_or_else(|| wp.thumb_url.clone());
        let worker = &self.worker;
        let fallback = entry.local_path.clone();
        let tex = self
            .textures
            .get(&prev_key, || worker.send(Command::LoadImage { tex_key: prev_key.clone(), url, max_dim: 1600, fallback }))
            .or_else(|| self.textures.peek(&format!("thumb:{key}")));
        let radius = CornerRadius::same(10);
        ui.painter().rect_filled(rect, radius, p.surface2);
        if let Some(t) = tex {
            egui::Image::from_texture((t.id(), t.size_vec2()))
                .uv(super::grid::cover_uv(t.size_vec2(), rect.size()))
                .corner_radius(radius)
                .paint_at(ui, rect);
        }
        if resp.hovered() {
            ui.painter().rect_stroke(rect, radius, Stroke::new(1.5, p.accent), egui::StrokeKind::Inside);
        }
        if resp.on_hover_text("Click for full-size preview (Space)").on_hover_cursor(egui::CursorIcon::ZoomIn).clicked() {
            self.actions.push(Action::Lightbox(wp.clone()));
        }

        // Primary actions
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let downloading = self.downloading.contains(&key);
            let label = if downloading { "Downloading…".to_string() } else { with_icon(ph::MONITOR, "Set as wallpaper") };
            let width = ui.available_width() - 76.0 - ui.spacing().item_spacing.x;
            let set = ui.add_sized(
                [width, 32.0],
                egui::Button::new(semibold(label).color(p.on_accent)).fill(p.accent).corner_radius(CornerRadius::same(theme::RADIUS)),
            );
            if set.clicked() && !downloading {
                self.actions.push(Action::SetWallpaper(wp.clone()));
            }
            let combo = egui::ComboBox::from_id_salt("fit_detail")
                .selected_text(self.config.fit_mode.label())
                .width(76.0)
                .show_ui(ui, |ui| {
                    for m in crate::model::FitMode::ALL {
                        ui.selectable_value(&mut self.config.fit_mode, m, m.label());
                    }
                })
                .response;
            theme::name_widget(&combo, egui::Role::ComboBox, "Fit mode");
        });
        if self.displays.len() > 1 {
            // One display only; the button above sets all of them.
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Only on").color(p.weak));
                for d in self.displays.clone() {
                    let hint = format!("Set as the wallpaper of {} ({}x{}) only", d.name, d.width, d.height);
                    if ui.button(&d.name).on_hover_text(hint).clicked() && !self.downloading.contains(&key) {
                        self.actions.push(Action::SetWallpaperOn(wp.clone(), d.name));
                    }
                }
            });
        }
        let similar = self.info(&wp.provider).is_some_and(|i| i.caps.similar);
        let has_tools = !wp.is_local() || entry.local_path.is_some() || wp.page_url.is_some() || similar;
        if has_tools {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if !wp.is_local() {
                    let (icon, tip) = if entry.local_path.is_some() { (ph::CHECK_CIRCLE, "Downloaded") } else { (ph::DOWNLOAD_SIMPLE, "Download (D)") };
                    if theme::icon_button(ui, icon, tip).clicked() && entry.local_path.is_none() {
                        self.actions.push(Action::Download(wp.clone()));
                    }
                }
                if let Some(path) = &entry.local_path
                    && theme::icon_button(ui, ph::FOLDER_OPEN, "Show file").clicked()
                {
                    self.actions.push(Action::Reveal(path.clone()));
                }
                if let Some(u) = &wp.page_url
                    && theme::icon_button(ui, ph::GLOBE, "Open page").clicked()
                {
                    self.actions.push(Action::OpenUrl(u.clone()));
                }
                if similar && theme::icon_button(ui, ph::APPROXIMATE_EQUALS, "Find similar").clicked() {
                    self.actions.push(Action::Similar(wp.clone()));
                }
            });
        }

        // Curation
        ui.add_space(10.0);
        caption(ui, &p, "Curate");
        ui.horizontal(|ui| {
            let spacing = ui.spacing().item_spacing.x;
            let width = (ui.available_width() - spacing * 2.0) / 3.0;
            for (st, name) in [(Status::Considering, "Consider"), (Status::Kept, "Keep"), (Status::Rejected, "Reject")] {
                let on = entry.status == st;
                let c = status_color(st);
                let label = with_icon(super::grid::status_icon(st), name);
                let btn = egui::Button::new(if on { semibold(label).color(Color32::WHITE) } else { RichText::new(label) })
                    .fill(if on { c } else { p.surface2 })
                    .corner_radius(CornerRadius::same(theme::RADIUS));
                if ui.add_sized([width, 30.0], btn).clicked() {
                    self.actions.push(Action::SetStatus(wp.clone(), st));
                }
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for r in 1..=5u8 {
                let filled = r <= entry.rating;
                let star = RichText::new(ph::STAR).size(21.0);
                let star = if filled { star.family(FontFamily::Name(ICON_FILL.into())).color(Color32::from_rgb(255, 200, 50)) } else { star.color(p.weak) };
                if ui.add(egui::Button::new(star).frame(false)).on_hover_text(format!("{r} star(s) — key {r}")).clicked() {
                    self.actions.push(Action::Rate(wp.clone(), r));
                }
            }
        });

        // Local tags
        ui.add_space(8.0);
        caption(ui, &p, "My tags");
        if !entry.tags.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for t in &entry.tags {
                    if theme::chip(ui, &p, true, format!("{t}  {}", ph::X)).on_hover_text("Remove tag").clicked() {
                        self.actions.push(Action::RemoveTag(wp.clone(), t.clone()));
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.tag_input)
                    .hint_text("add tags, comma separated")
                    // Room for the button, and for the field's own margins around this width.
                    .desired_width(ui.available_width() - 60.0),
            );
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button("Add").clicked() || enter) && !self.tag_input.trim().is_empty() {
                self.actions.push(Action::AddTag(wp.clone(), std::mem::take(&mut self.tag_input)));
                if enter {
                    resp.request_focus();
                }
            }
        });
        let prefix = self.tag_input.trim().to_lowercase();
        if !prefix.is_empty() {
            let suggestions: Vec<String> = self
                .all_tags
                .iter()
                .map(|(t, _)| t)
                .filter(|t| t.to_lowercase().starts_with(&prefix) && !entry.tags.contains(t))
                .take(8)
                .cloned()
                .collect();
            ui.horizontal_wrapped(|ui| {
                for s in suggestions {
                    if theme::chip(ui, &p, false, &s).clicked() {
                        self.tag_input.clear();
                        self.actions.push(Action::AddTag(wp.clone(), s));
                    }
                }
            });
        }

        // Notes
        ui.add_space(8.0);
        caption(ui, &p, "Notes");
        if self.notes_buf.0 != key {
            self.notes_buf = (key.clone(), entry.notes.clone());
        }
        let resp = ui.add(
            egui::TextEdit::multiline(&mut self.notes_buf.1)
                .hint_text("Why do you like it? Where would it fit?")
                .desired_rows(2)
                .desired_width(f32::INFINITY),
        );
        if resp.lost_focus() && self.notes_buf.1 != entry.notes {
            self.actions.push(Action::SetNotes(wp.clone(), self.notes_buf.1.clone()));
        }

        // Provider tags
        let searchable = self.info(&wp.provider).is_some_and(|i| i.caps.text_search);
        if !wp.tags.is_empty() {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                caption(ui, &p, "Source tags");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("adopt all").on_hover_text("Copy all source tags to my tags").clicked() {
                        self.actions.push(Action::AddTag(wp.clone(), wp.tags.join(",")));
                    }
                });
            });
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(5.0, 5.0);
                for t in &wp.tags {
                    let r = theme::chip(ui, &p, false, t).on_hover_text("Click: search · Right-click: add to my tags");
                    if r.clicked() && searchable {
                        self.provider = wp.provider.clone();
                        self.actions.push(Action::SearchText(t.clone()));
                    }
                    if r.secondary_clicked() {
                        self.actions.push(Action::AddTag(wp.clone(), t.clone()));
                    }
                }
            });
        } else if self.details_requested.contains(&key) && !self.details.contains_key(&key) {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("loading tags…").color(p.weak));
            });
        }

        if let Some(desc) = wp.extra.get("description") {
            ui.add_space(8.0);
            caption(ui, &p, "About");
            ui.add(egui::Label::new(RichText::new(desc).color(p.weak)).wrap());
        }

        // Metadata
        ui.add_space(10.0);
        caption(ui, &p, "Details");
        egui::Grid::new("meta").num_columns(2).spacing([14.0, 5.0]).show(ui, |ui| {
            let row = |ui: &mut egui::Ui, k: &str, v: String| {
                ui.label(RichText::new(k).color(p.weak));
                ui.add(egui::Label::new(v).truncate());
                ui.end_row();
            };
            if let Some(s) = wp.file_size {
                row(ui, "Size", theme::human_size(s));
            }
            if let Some(c) = &wp.category {
                row(ui, "Category", c.clone());
            }
            if let Some(pu) = &wp.purity {
                row(ui, "Purity", pu.clone());
            }
            if let Some(l) = &wp.license {
                row(ui, "License", l.clone());
            }
            if let Some(d) = &wp.created_at {
                row(ui, "Published", d.clone());
            }
            if entry.added_at > 0
                && let Some(d) = chrono::DateTime::from_timestamp(entry.added_at, 0)
            {
                row(ui, "In library since", d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string());
            }
            if let Some(a) = &wp.author {
                ui.label(RichText::new("Author").color(p.weak));
                match &wp.author_url {
                    Some(u) => {
                        if ui.link(a).clicked() {
                            self.actions.push(Action::OpenUrl(u.clone()));
                        }
                    }
                    None => {
                        ui.add(egui::Label::new(a).truncate());
                    }
                }
                ui.end_row();
            }
            if let Some(s) = &wp.source
                && s.starts_with("http")
            {
                ui.label(RichText::new("Source").color(p.weak));
                if ui.add(egui::Link::new(s.as_str())).on_hover_text(s).clicked() {
                    self.actions.push(Action::OpenUrl(s.clone()));
                }
                ui.end_row();
            }
            if let Some(path) = &entry.local_path {
                ui.label(RichText::new("File").color(p.weak));
                let name = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                if ui.link(name).on_hover_text(path.display().to_string()).clicked() {
                    self.actions.push(Action::Reveal(path.clone()));
                }
                ui.end_row();
            }
        });

        if !wp.colors.is_empty() {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let searchable_colors = self.info(&wp.provider).is_some_and(|i| i.caps.colors);
                for c in &wp.colors {
                    if let Some(col) = parse_hex(c) {
                        let (r, resp) = ui.allocate_exact_size(vec2(26.0, 18.0), Sense::click());
                        ui.painter().rect_filled(r, CornerRadius::same(5), col);
                        let tip = if searchable_colors { format!("{c} — search this colour") } else { c.clone() };
                        if resp.on_hover_text(tip).clicked() && searchable_colors {
                            self.query.color = Some(c.trim_start_matches('#').to_string());
                            self.provider = wp.provider.clone();
                            self.actions.push(Action::SearchText(String::new()));
                        }
                    }
                }
            });
        }

        if entry.added_at > 0 {
            ui.add_space(14.0);
            if ui
                .add(egui::Button::new(RichText::new(with_icon(ph::TRASH, "Remove from library")).color(p.danger)).frame(false))
                .on_hover_text("Forget status, rating, tags and notes; deletes the downloaded file")
                .clicked()
            {
                self.actions.push(Action::Forget(wp.clone()));
            }
        }
    }
}
