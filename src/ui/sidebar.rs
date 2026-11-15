use egui::{CornerRadius, RichText, Sense, Stroke, vec2};

use super::{
    Action, App, View,
    detail::parse_hex,
    theme::{self, caption, chip, ph, with_icon},
};
use crate::{
    library::{LibraryFilter, LibrarySort, OrientationFilter},
    model::{Orientation, Sorting},
};

const FILTER_DEBOUNCE_SECS: f64 = 0.18;

/// Wallhaven's fixed colour palette.
const PALETTE: &[&str] = &[
    "660000", "990000", "cc0000", "cc3333", "ea4c88", "993399", "663399", "333399", "0066cc", "0099cc",
    "66cccc", "77cc33", "669900", "336600", "666600", "999900", "cccc33", "ffff00", "ffcc33", "ff9900",
    "ff6600", "cc6633", "996633", "663300", "000000", "999999", "cccccc", "ffffff", "424153",
];

const RESOLUTIONS: &[(u32, u32)] = &[(1920, 1080), (2560, 1440), (3440, 1440), (3840, 2160), (5120, 2880)];
const RATIOS: &[&str] = &["16x9", "16x10", "21x9", "32x9", "4x3", "9x16", "10x16"];
const RANGES: &[(&str, &str)] =
    &[("1d", "Day"), ("3d", "3 days"), ("1w", "Week"), ("1M", "Month"), ("3M", "3 months"), ("6M", "6 months"), ("1y", "Year")];

impl App {
    pub(super) fn browse_sidebar(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        let Some(info) = self.info(&self.provider).cloned() else { return };
        let caps = info.caps;
        let before = self.query.clone();

        if caps.collections {
            self.collections_sidebar(ui, info.id);
        }

        if !caps.sortings.is_empty() && self.query.collection.is_none() {
            caption(ui, &p, "Sort");
            ui.horizontal_wrapped(|ui| {
                // A sorting this source doesn't have is ignored by it, leaving its default (first).
                let effective = if caps.sortings.contains(&self.query.sorting) { self.query.sorting } else { caps.sortings[0] };
                for s in &caps.sortings {
                    if chip(ui, &p, effective == *s, s.label()).clicked() {
                        self.query.sorting = *s;
                    }
                }
            });
            if caps.order {
                ui.horizontal_wrapped(|ui| {
                    if chip(ui, &p, self.query.descending, "Descending").clicked() {
                        self.query.descending = true;
                    }
                    if chip(ui, &p, !self.query.descending, "Ascending").clicked() {
                        self.query.descending = false;
                    }
                });
            }
            if caps.toplist_range && self.query.sorting == Sorting::Toplist {
                caption(ui, &p, "Toplist range");
                ui.horizontal_wrapped(|ui| {
                    for (v, l) in RANGES {
                        if chip(ui, &p, self.query.toplist_range == *v, *l).clicked() {
                            self.query.toplist_range = v.to_string();
                        }
                    }
                });
            }
            if self.query.sorting == Sorting::Random && ui.button(with_icon(ph::SHUFFLE, "Reshuffle")).clicked() {
                self.actions.push(Action::NewSearch);
            }
            ui.add_space(8.0);
        }

        if caps.categories {
            caption(ui, &p, "Categories");
            ui.horizontal_wrapped(|ui| {
                for (i, l) in ["General", "Anime", "People"].iter().enumerate() {
                    if chip(ui, &p, self.query.categories[i], *l).clicked() {
                        self.query.categories[i] = !self.query.categories[i];
                    }
                }
            });
            ui.add_space(8.0);
        }

        if caps.purity {
            caption(ui, &p, "Purity");
            ui.horizontal_wrapped(|ui| {
                for (i, l) in ["SFW", "Sketchy"].iter().enumerate() {
                    if chip(ui, &p, self.query.purity[i], *l).clicked() {
                        self.query.purity[i] = !self.query.purity[i];
                    }
                }
                let nsfw_ok = !caps.nsfw_needs_key || info.has_key;
                let r = ui.add_enabled_ui(nsfw_ok, |ui| chip(ui, &p, self.query.purity[2], "NSFW")).inner;
                if r.on_disabled_hover_text("Requires an API key (Settings)").clicked() {
                    self.query.purity[2] = !self.query.purity[2];
                }
                if !nsfw_ok {
                    self.query.purity[2] = false;
                }
            });
            ui.add_space(8.0);
        }

        if caps.orientation {
            caption(ui, &p, "Orientation");
            ui.horizontal_wrapped(|ui| {
                for o in Orientation::ALL {
                    if chip(ui, &p, self.query.orientation == o, o.label()).clicked() {
                        self.query.orientation = o;
                    }
                }
            });
            ui.add_space(8.0);
        }

        if caps.min_resolution {
            caption(ui, &p, "At least");
            ui.horizontal_wrapped(|ui| {
                if chip(ui, &p, self.query.min_resolution.is_none(), "Any").clicked() {
                    self.query.min_resolution = None;
                }
                for &(w, h) in RESOLUTIONS {
                    let label = match (w, h) {
                        (1920, 1080) => "1080p".to_string(),
                        (2560, 1440) => "1440p".to_string(),
                        (3840, 2160) => "4K".to_string(),
                        (5120, 2880) => "5K".to_string(),
                        _ => format!("{w}×{h}"),
                    };
                    if chip(ui, &p, self.query.min_resolution == Some((w, h)), label).on_hover_text(format!("{w}×{h}")).clicked() {
                        self.query.min_resolution = Some((w, h));
                    }
                }
            });
            ui.add_space(8.0);
        }

        if caps.ratios {
            caption(ui, &p, "Aspect ratio");
            ui.horizontal_wrapped(|ui| {
                for r in RATIOS {
                    let on = self.query.ratios.iter().any(|x| x == r);
                    if chip(ui, &p, on, r.replace('x', ":")).clicked() {
                        if on {
                            self.query.ratios.retain(|x| x != r);
                        } else {
                            self.query.ratios.push(r.to_string());
                        }
                    }
                }
            });
            ui.add_space(8.0);
        }

        if caps.colors {
            ui.horizontal(|ui| {
                caption(ui, &p, "Colour");
                if self.query.color.is_some() && ui.small_button("clear").clicked() {
                    self.query.color = None;
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                for hex in PALETTE {
                    let col = parse_hex(hex).unwrap_or_default();
                    let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
                    let selected = self.query.color.as_deref() == Some(*hex);
                    ui.painter().rect_filled(r, CornerRadius::same(5), col);
                    if selected {
                        ui.painter().rect_stroke(r.expand(2.0), CornerRadius::same(6), Stroke::new(2.0, p.accent), egui::StrokeKind::Outside);
                    } else if resp.hovered() {
                        ui.painter().rect_stroke(r, CornerRadius::same(5), Stroke::new(1.0, p.text), egui::StrokeKind::Outside);
                    }
                    if resp.on_hover_text(format!("#{hex}")).clicked() {
                        self.query.color = if selected { None } else { Some(hex.to_string()) };
                    }
                }
            });
            ui.add_space(8.0);
        }

        if !caps.text_search && caps.sortings.is_empty() && !caps.orientation {
            ui.label(RichText::new(format!("{} has no filters.", info.name)).color(p.weak));
            ui.add_space(8.0);
        }

        // Everything except the text box applies immediately.
        let mut after = self.query.clone();
        after.text = before.text.clone();
        after.seed = before.seed.clone();
        if after != before {
            self.actions.push(Action::NewSearch);
        }

        if !self.saved.is_empty() {
            ui.separator();
            caption(ui, &p, "Saved searches");
            let saved = std::mem::take(&mut self.saved);
            for s in &saved {
                ui.horizontal(|ui| {
                    let label = format!("{}  ·  {}", s.name, self.provider_name(&s.provider));
                    let w = ui.available_width() - 38.0;
                    if ui
                        .add_sized([w, 24.0], egui::Button::new(RichText::new(label).color(p.text)).truncate().frame_when_inactive(false))
                        .clicked()
                    {
                        self.actions.push(Action::ApplySaved(s.clone()));
                    }
                    if theme::icon_button(ui, ph::X, "Delete saved search").clicked() {
                        self.actions.push(Action::DeleteSaved(s.id));
                    }
                });
            }
            self.saved = saved;
            ui.add_space(8.0);
        }

        ui.separator();
        caption(ui, &p, "View");
        ui.checkbox(&mut self.config.show_rejected, "Show rejected");
        ui.checkbox(&mut self.config.hide_duplicates, "Hide duplicates of my library")
            .on_hover_text("Results whose thumbnail matches a wallpaper you already kept, shortlisted or rejected");
        ui.add(egui::Slider::new(&mut self.config.thumb_size, 140.0..=480.0).text("Size").show_value(false));
        if !info.homepage.is_empty() {
            ui.add_space(6.0);
            ui.hyperlink_to(RichText::new(with_icon(ph::ARROW_SQUARE_OUT, info.name)).color(p.weak), info.homepage);
        }
    }

    pub(super) fn library_sidebar(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        let before = self.lib_filter.clone();
        if self.view == View::History {
            caption(ui, &p, "History");
            ui.label(RichText::new("Wallpapers you have set, most recent first.").color(p.weak));
            ui.add_space(8.0);
            caption(ui, &p, "View");
            ui.add(egui::Slider::new(&mut self.config.thumb_size, 140.0..=480.0).text("Size").show_value(false));
            return;
        }
        caption(ui, &p, "Search");
        ui.add(
            egui::TextEdit::singleline(&mut self.lib_filter.text)
                .hint_text("title, tags, notes…")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(8.0);

        caption(ui, &p, "Sort");
        let combo = egui::ComboBox::from_id_salt("lib_sort").selected_text(self.lib_filter.sort.label()).width(ui.available_width()).show_ui(ui, |ui| {
            for s in LibrarySort::ALL {
                ui.selectable_value(&mut self.lib_filter.sort, s, s.label());
            }
        })
        .response;
        theme::name_widget(&combo, egui::Role::ComboBox, "Sort");
        ui.add_space(8.0);

        caption(ui, &p, "Minimum rating");
        ui.horizontal_wrapped(|ui| {
            for r in 0..=5u8 {
                let label = if r == 0 { "Any".to_string() } else { format!("{r}★") };
                if chip(ui, &p, self.lib_filter.min_rating == r, label).clicked() {
                    self.lib_filter.min_rating = r;
                }
            }
        });
        ui.add_space(8.0);

        caption(ui, &p, "Orientation");
        ui.horizontal_wrapped(|ui| {
            for (o, l) in [(OrientationFilter::Any, "Any"), (OrientationFilter::Landscape, "Landscape"), (OrientationFilter::Portrait, "Portrait")] {
                if chip(ui, &p, self.lib_filter.orientation == o, l).clicked() {
                    self.lib_filter.orientation = o;
                }
            }
        });
        ui.add_space(8.0);

        if self.lib_providers.len() > 1 {
            caption(ui, &p, "Source");
            ui.horizontal_wrapped(|ui| {
                if chip(ui, &p, self.lib_filter.provider.is_none(), "All").clicked() {
                    self.lib_filter.provider = None;
                }
                for (id, n) in self.lib_providers.clone() {
                    let on = self.lib_filter.provider.as_deref() == Some(id.as_str());
                    if chip(ui, &p, on, format!("{} {n}", self.provider_name(&id))).clicked() {
                        self.lib_filter.provider = if on { None } else { Some(id) };
                    }
                }
            });
            ui.add_space(8.0);
        }

        ui.horizontal(|ui| {
            caption(ui, &p, "Tags");
            if self.lib_filter.tag.is_some() && ui.small_button("clear").clicked() {
                self.lib_filter.tag = None;
            }
        });
        if self.all_tags.is_empty() {
            ui.label(RichText::new("Tag wallpapers in the detail panel to organise them.").color(p.weak).size(12.0));
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(5.0, 5.0);
            for (t, n) in &self.all_tags {
                let on = self.lib_filter.tag.as_deref() == Some(t.as_str());
                if chip(ui, &p, on, format!("{t} {n}")).clicked() {
                    self.lib_filter.tag = if on { None } else { Some(t.clone()) };
                }
            }
        });
        ui.add_space(8.0);
        ui.separator();
        caption(ui, &p, "View");
        ui.add(egui::Slider::new(&mut self.config.thumb_size, 140.0..=480.0).text("Size").show_value(false));
        // Filters apply at once; typing waits for a short pause so that a large library
        // isn't reloaded on every keystroke.
        if self.lib_filter.text != before.text {
            let now = ui.input(|i| i.time);
            self.filter_deadline = Some(now + FILTER_DEBOUNCE_SECS);
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(FILTER_DEBOUNCE_SECS));
        }
        let unchanged = LibraryFilter { text: self.lib_filter.text.clone(), ..before };
        if self.lib_filter != unchanged {
            self.lib_dirty = true;
        }
    }
}

impl App {
    fn collections_sidebar(&mut self, ui: &mut egui::Ui, provider: &str) {
        let p = self.pal;
        if self.collections_requested.insert(provider.to_string()) {
            self.worker.send(crate::worker::Command::Collections { provider: provider.to_string() });
        }
        let list = match self.collections.get(provider) {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("loading collections…").color(p.weak));
                });
                return;
            }
            Some(Err(e)) => {
                caption(ui, &p, "Collections");
                ui.label(RichText::new(e).color(p.weak).size(12.0));
                if ui.small_button(with_icon(ph::ARROW_CLOCKWISE, "Try again")).clicked() {
                    self.collections_requested.remove(provider);
                    self.collections.remove(provider);
                }
                ui.add_space(8.0);
                return;
            }
            Some(Ok(list)) if list.is_empty() => return,
            Some(Ok(list)) => list.clone(),
        };
        ui.horizontal(|ui| {
            caption(ui, &p, "Collections");
            if theme::icon_button(ui, ph::ARROW_CLOCKWISE, "Reload collections").clicked() {
                self.collections_requested.remove(provider);
                self.collections.remove(provider);
            }
        });
        ui.horizontal_wrapped(|ui| {
            if chip(ui, &p, self.query.collection.is_none(), "Search").clicked() {
                self.query.collection = None;
            }
            for c in &list {
                let on = self.query.collection.as_deref() == Some(c.id.as_str());
                if chip(ui, &p, on, format!("{} {}", c.label, c.count)).clicked() {
                    self.query.collection = if on { None } else { Some(c.id.clone()) };
                }
            }
        });
        if let Some(id) = self.query.collection.clone()
            && let Some(c) = list.iter().find(|c| c.id == id)
        {
            ui.add_space(4.0);
            let busy = self.importing_collection.is_some();
            let label = if busy { "Importing…".to_string() } else { with_icon(ph::ARROW_SQUARE_IN, &format!("Import all {} into library", c.count)) };
            if ui
                .add_enabled(!busy, egui::Button::new(label))
                .on_hover_text("Keeps every wallpaper of this collection and tags it with the collection name")
                .clicked()
            {
                self.importing_collection = Some(c.label.clone());
                self.toast(theme::ToastKind::Info, format!("Importing \"{}\"…", c.label));
                self.worker.send(crate::worker::Command::ImportCollection {
                    provider: provider.to_string(),
                    query: self.query.clone(),
                    label: c.label.clone(),
                });
            }
        }
        ui.add_space(8.0);
    }
}
