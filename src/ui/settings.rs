use egui::{Color32, CornerRadius, RichText, Sense, Stroke, vec2};

use super::{
    Action, App,
    theme::{self, ACCENTS, Palette, ph, semibold, with_icon},
};
use crate::{
    config::{Config, Theme, key_env_var},
    model::FitMode,
    providers, setter,
};

const NAV_WIDTH: f32 = 190.0;

/// The settings are split into sections, one shown at a time and picked on the left.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub(super) enum Section {
    #[default]
    Appearance,
    Wallpaper,
    Rotation,
    Sources,
    Library,
    System,
}

impl Section {
    pub(super) const ALL: [Section; 6] =
        [Section::Appearance, Section::Wallpaper, Section::Rotation, Section::Sources, Section::Library, Section::System];

    pub(super) fn label(self) -> &'static str {
        match self {
            Section::Appearance => "Appearance",
            Section::Wallpaper => "Wallpaper",
            Section::Rotation => "Rotation",
            Section::Sources => "Sources",
            Section::Library => "Library",
            Section::System => "System",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Section::Appearance => ph::PALETTE,
            Section::Wallpaper => ph::MONITOR,
            Section::Rotation => ph::ARROWS_CLOCKWISE,
            Section::Sources => ph::GLOBE,
            Section::Library => ph::IMAGES,
            Section::System => ph::POWER,
        }
    }
}

impl App {
    pub(super) fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        let mut draft = self.settings_draft.take().unwrap_or_else(|| self.config.clone());
        // The sections on the left, the chosen one in a centered column beside them.
        let pane = (ui.available_width() - NAV_WIDTH).max(0.0);
        let width = pane.min(820.0);
        let pad = ((pane - width) / 2.0).max(16.0);
        // Outside the scroll area: the way to save must stay in view, whichever section is open.
        let right_pad = pane - pad - (width - 32.0);
        let header = egui::Frame::default().fill(p.surface).inner_margin(egui::Margin::symmetric(0, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            self.settings_header(ui, &p, &mut draft, 20.0, right_pad);
        });
        let line = header.response.rect.bottom();
        ui.painter().hline(ui.max_rect().x_range(), line, Stroke::new(1.0, p.border));

        let rest = egui::Rect::from_min_max(egui::pos2(ui.max_rect().left(), line), ui.max_rect().max);
        let (nav, body) = rest.split_left_right_at_x(rest.left() + NAV_WIDTH);
        ui.painter().rect_filled(nav, 0, p.surface);
        ui.painter().vline(nav.right(), nav.y_range(), Stroke::new(1.0, p.border));
        ui.scope_builder(egui::UiBuilder::new().max_rect(nav.shrink(10.0)), |ui| {
            egui::ScrollArea::vertical().id_salt("settings-nav").show(ui, |ui| self.settings_nav(ui, &p));
        });
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            // Each section keeps its own scroll position.
            egui::ScrollArea::vertical().id_salt(self.settings_section).auto_shrink([false, false]).show(ui, |ui| {
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.add_space(pad);
                    ui.vertical(|ui| {
                        ui.set_width(width - 32.0);
                        self.settings_body(ui, &p, &mut draft);
                    });
                });
                ui.add_space(24.0);
            });
        });
        ui.advance_cursor_after_rect(rest);
        self.settings_draft = Some(draft);
    }

    fn settings_nav(&mut self, ui: &mut egui::Ui, p: &Palette) {
        ui.spacing_mut().item_spacing.y = 2.0;
        for section in Section::ALL {
            let selected = self.settings_section == section;
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, section.label()));
            let painter = ui.painter();
            if selected {
                painter.rect_filled(rect, CornerRadius::same(theme::RADIUS), p.accent_soft());
            } else if resp.hovered() || resp.has_focus() {
                painter.rect_filled(rect, CornerRadius::same(theme::RADIUS), p.hover);
            }
            let fg = if selected {
                p.accent_text()
            } else if resp.hovered() {
                p.text
            } else {
                p.weak
            };
            let (icon_font, font) = if selected {
                (theme::fill_icon_font(17.0), theme::semibold_font(13.5))
            } else {
                (egui::FontId::proportional(17.0), egui::FontId::proportional(13.5))
            };
            let y = rect.center().y;
            painter.text(egui::pos2(rect.left() + 12.0, y), egui::Align2::LEFT_CENTER, section.icon(), icon_font, fg);
            painter.text(egui::pos2(rect.left() + 38.0, y), egui::Align2::LEFT_CENTER, section.label(), font, fg);
            if resp.clicked() {
                self.settings_section = section;
            }
        }
    }

    fn settings_header(&mut self, ui: &mut egui::Ui, p: &Palette, draft: &mut Config, pad: f32, right_pad: f32) {
        ui.horizontal(|ui| {
            ui.add_space(pad);
            ui.label(semibold("Settings").size(21.0));
            let changed = *draft != self.config;
            if changed {
                ui.add_space(4.0);
                theme::pill(ui, p.warning, "Unsaved changes");
            }
            let shortcut = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(right_pad);
                if ui.add_enabled(changed, egui::Button::new(with_icon(ph::ARROW_U_UP_LEFT, "Revert"))).clicked() {
                    *draft = self.config.clone();
                }
                let save = ui.add_enabled_ui(changed, |ui| theme::primary_button(ui, p, with_icon(ph::FLOPPY_DISK, "Save changes"))).inner;
                let save = save.on_hover_text(theme::shortcut("Applies at once, no restart needed (Ctrl+S)"));
                if changed && (save.clicked() || shortcut) {
                    self.save_settings(draft.clone());
                }
            });
        });
    }

    /// Store edited settings and put them to use, saying what came of it.
    fn save_settings(&mut self, cfg: Config) {
        let before: Vec<&str> = self.enabled.iter().map(|i| i.id).collect();
        if !self.apply_config(cfg) {
            return; // apply_config said why.
        }
        let newly: Vec<&str> = self.enabled.iter().filter(|i| !before.contains(&i.id)).map(|i| i.name).collect();
        let msg = if newly.is_empty() {
            "Settings saved".to_string()
        } else {
            format!("Settings saved. Now available in Browse: {}", newly.join(", "))
        };
        self.toast(theme::ToastKind::Success, msg);
    }

    /// Going elsewhere keeps what was entered, rather than dropping it without a word.
    pub(super) fn commit_settings_draft(&mut self) {
        if let Some(draft) = self.settings_draft.clone()
            && draft != self.config
        {
            self.save_settings(draft);
        }
    }

    fn settings_body(&mut self, ui: &mut egui::Ui, p: &Palette, draft: &mut Config) {
        match self.settings_section {
            Section::Appearance => self.appearance_card(ui, p, draft),
            Section::Wallpaper => {
                self.wallpaper_card(ui, p, draft);
                ui.add_space(12.0);
                self.effects_card(ui, p, draft);
            }
            Section::Rotation => self.rotation_card(ui, p, draft),
            Section::Sources => {
                self.sources_card(ui, p, draft);
                ui.add_space(12.0);
                self.local_folders_card(ui, p, draft);
            }
            Section::Library => {
                self.library_card(ui, p, draft);
                ui.add_space(12.0);
                self.locations_card(ui, p, draft);
            }
            Section::System => {
                self.system_card(ui, p, draft);
                ui.add_space(12.0);
                self.about_card(ui, p);
            }
        }
    }

    fn appearance_card(&mut self, ui: &mut egui::Ui, p: &Palette, draft: &mut Config) {
        theme::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_title(ui, p, ph::PALETTE, "Appearance");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Theme");
                ui.add_space(20.0);
                for (t, icon, l) in [(Theme::Dark, ph::MOON, "Dark"), (Theme::Light, ph::SUN, "Light")] {
                    if theme::chip(ui, p, draft.theme == t, with_icon(icon, l)).clicked() {
                        draft.theme = t;
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Accent");
                ui.add_space(16.0);
                for a in ACCENTS {
                    let c = Color32::from_rgb(a[0], a[1], a[2]);
                    let (r, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
                    ui.painter().circle_filled(r.center(), 10.0, c);
                    if draft.accent == *a {
                        ui.painter().circle_stroke(r.center(), 12.0, Stroke::new(2.0, p.text));
                    }
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        draft.accent = *a;
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Thumbnail size");
                ui.add(egui::Slider::new(&mut draft.thumb_size, 140.0..=480.0).suffix(" px"));
            });
        });
    }

    fn wallpaper_card(&mut self, ui: &mut egui::Ui, p: &Palette, draft: &mut Config) {
        theme::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_title(ui, p, ph::MONITOR, "Desktop wallpaper");
            ui.add_space(6.0);
            egui::Grid::new("wp").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                ui.label("Fit mode");
                ui.horizontal_wrapped(|ui| {
                    for m in FitMode::ALL {
                        if theme::chip(ui, p, draft.fit_mode == m, m.label()).clicked() {
                            draft.fit_mode = m;
                        }
                    }
                });
                ui.end_row();
                ui.label("Backend");
                ui.label(RichText::new(setter::backend_name()).color(p.weak));
                ui.end_row();
                if !self.displays.is_empty() {
                    ui.label("Displays");
                    let list: Vec<String> = self.displays.iter().map(|d| format!("{} ({}x{})", d.name, d.width, d.height)).collect();
                    ui.label(RichText::new(list.join(" · ")).color(p.weak)).on_hover_text(
                        "Each display can have its own wallpaper: right-click a wallpaper, or use \"Only on\" in its details.",
                    );
                    ui.end_row();
                }
                if cfg!(target_os = "linux") {
                    ui.label("Custom command");
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.linux_set_command)
                            .hint_text("e.g. swww img {path}")
                            .desired_width(360.0),
                    );
                    ui.end_row();
                }
            });
        });
    }

    fn library_card(&mut self, ui: &mut egui::Ui, p: &Palette, draft: &mut Config) {
        theme::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_title(ui, p, ph::IMAGES, "Library");
            ui.add_space(6.0);
            ui.checkbox(&mut draft.auto_download_kept, "Download the full image when I keep a wallpaper (works offline later)");
            ui.horizontal(|ui| {
                ui.label(format!(
                    "{} kept · {} considering · {} rejected",
                    self.counts[2], self.counts[1], self.counts[3]
                ));
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button(with_icon(ph::EXPORT, "Export library…")).on_hover_text("Writes a JSON file with statuses, ratings, tags and notes").clicked() {
                    self.actions.push(Action::Pick(super::pickers::Pick::ExportTo));
                }
                if ui.button(with_icon(ph::DOWNLOAD_SIMPLE, "Download missing files")).clicked() {
                    self.actions.push(Action::DownloadMissing);
                }
                let dir = self.paths.wallpapers_dir(draft);
                if ui.button(with_icon(ph::FOLDER_OPEN, "Open wallpapers folder")).clicked() {
                    let _ = std::fs::create_dir_all(&dir);
                    self.actions.push(Action::OpenPath(dir));
                }
            });
            ui.horizontal(|ui| {
                ui.label("Import export file");
                ui.add(egui::TextEdit::singleline(&mut self.import_input).hint_text("path to walle-library-….json").desired_width(320.0));
                if ui.button("Import").clicked() && !self.import_input.trim().is_empty() {
                    self.actions.push(Action::ImportLibrary(std::mem::take(&mut self.import_input).trim().into()));
                }
                if ui.button("Browse…").clicked() {
                    self.actions.push(Action::Pick(super::pickers::Pick::ImportExport));
                }
            });
            ui.label(RichText::new("Tip: you can also drop an export file, images or folders onto the window.").color(p.weak).size(12.0));
        });
    }

    fn local_folders_card(&mut self, ui: &mut egui::Ui, p: &Palette, draft: &mut Config) {
        theme::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_title(ui, p, ph::FOLDER, "Local folders");
            ui.label(RichText::new("Browse your own wallpaper folders with the \"Local folders\" source.").color(p.weak));
            ui.add_space(6.0);
            let mut remove = None;
            for (i, f) in draft.local_folders.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(ph::FOLDER).color(p.weak));
                    if ui.link(f.display().to_string()).clicked() {
                        self.actions.push(Action::OpenPath(f.clone()));
                    }
                    if theme::icon_button(ui, ph::X, "Remove folder").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                draft.local_folders.remove(i);
            }
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.folder_input).hint_text("C:\\Users\\me\\Pictures\\Wallpapers").desired_width(360.0));
                let path = std::path::PathBuf::from(self.folder_input.trim());
                // A file system call, so only when the text changed (it can be a slow network path).
                if self.folder_check.0 != self.folder_input {
                    self.folder_check = (self.folder_input.clone(), path.is_dir());
                }
                let valid = self.folder_check.1;
                if ui.button("Browse…").clicked() {
                    self.actions.push(Action::Pick(super::pickers::Pick::LocalFolder));
                }
                if ui.add_enabled(valid, egui::Button::new("Add folder")).clicked() {
                    draft.local_folders.push(path);
                    draft.providers.entry("local".into()).or_default().enabled = true;
                    self.folder_input.clear();
                }
                if !self.folder_input.trim().is_empty() && !valid {
                    ui.label(RichText::new("not a folder").color(p.danger));
                }
            });
        });
    }

    fn sources_card(&mut self, ui: &mut egui::Ui, p: &Palette, draft: &mut Config) {
        theme::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_title(ui, p, ph::GLOBE, "Sources");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Default source");
                let infos = self.infos.clone();
                let cur = infos.iter().find(|i| i.id == draft.default_provider).map(|i| i.name).unwrap_or("?");
                let combo = egui::ComboBox::from_id_salt("defprov").selected_text(cur).show_ui(ui, |ui| {
                    for i in &infos {
                        ui.selectable_value(&mut draft.default_provider, i.id.to_string(), i.name);
                    }
                })
                .response;
                theme::name_widget(&combo, egui::Role::ComboBox, "Default source");
            });
            ui.add_space(6.0);
            for info in providers::all_infos(draft) {
                let id = info.id.to_string();
                let ps = draft.providers.entry(id.clone()).or_insert_with(|| crate::config::ProviderSettings {
                    enabled: crate::config::default_enabled(&id),
                    ..Default::default()
                });
                egui::Frame::default()
                    .fill(p.surface2)
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut ps.enabled, semibold(info.name));
                            let (c, label) = if !ps.enabled {
                                (p.weak, "Disabled")
                            } else if info.key_required && !info.has_key && ps.api_key.trim().is_empty() {
                                (p.warning, "Needs API key")
                            } else {
                                (p.success, "Ready")
                            };
                            theme::pill(ui, c, label);
                            if self.config.providers.get(&id) != Some(&*ps) {
                                theme::pill(ui, p.warning, "Not saved");
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if !info.homepage.is_empty() {
                                    ui.hyperlink_to(RichText::new(format!("site {}", ph::ARROW_SQUARE_OUT)).color(p.weak), info.homepage);
                                }
                            });
                        });
                        ui.label(RichText::new(info.key_help).color(p.weak).size(12.0));
                        if info.key_required || info.key_optional || !ps.api_key.is_empty() {
                            ui.horizontal(|ui| {
                                ui.label("API key");
                                let show = self.show_key.contains(&id);
                                ui.add(egui::TextEdit::singleline(&mut ps.api_key).password(!show).desired_width(330.0));
                                if ui.small_button(if show { "hide" } else { "show" }).clicked() {
                                    if show {
                                        self.show_key.remove(&id);
                                    } else {
                                        self.show_key.insert(id.clone());
                                    }
                                }
                            });
                            if ps.api_key.is_empty() && std::env::var(key_env_var(&id)).is_ok() {
                                ui.label(RichText::new(format!("Using key from ${}", key_env_var(&id))).color(p.weak).size(12.0));
                            }
                        }
                        if id == "wallhaven" {
                            let mut user = ps.options.get("username").cloned().unwrap_or_default();
                            ui.horizontal(|ui| {
                                ui.label("Username");
                                if ui.add(egui::TextEdit::singleline(&mut user).hint_text("for collections").desired_width(160.0)).changed() {
                                    ps.options.insert("username".into(), user);
                                }
                            });
                        }
                        if id == "bing" {
                            let mut market = ps.options.get("market").cloned().unwrap_or_else(|| "en-US".into());
                            ui.horizontal(|ui| {
                                ui.label("Market");
                                if ui.add(egui::TextEdit::singleline(&mut market).desired_width(80.0)).changed() {
                                    ps.options.insert("market".into(), market);
                                }
                            });
                        }
                    });
                ui.add_space(6.0);
            }
        });
    }

    fn locations_card(&mut self, ui: &mut egui::Ui, p: &Palette, draft: &mut Config) {
        theme::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_title(ui, p, ph::TRAY, "Locations");
            ui.add_space(6.0);
            egui::Grid::new("paths").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                let rows = [
                    ("Config", self.paths.config_file()),
                    ("Library database", self.paths.db_file()),
                    ("Wallpapers", self.paths.wallpapers_dir(draft)),
                    ("Thumbnail cache", self.paths.thumbs_dir()),
                ];
                for (label, path) in rows {
                    ui.label(RichText::new(label).color(p.weak));
                    if ui.link(path.display().to_string()).clicked() {
                        self.actions.push(if path.is_dir() { Action::OpenPath(path) } else { Action::Reveal(path) });
                    }
                    ui.end_row();
                }
            });
        });
    }

    fn about_card(&mut self, ui: &mut egui::Ui, p: &Palette) {
        theme::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    theme::card_title(ui, p, ph::INFO, "About walle");
                    let build = format!("Version {}  ·  {}", crate::build_info::VERSION, crate::build_info::commit_label());
                    ui.label(RichText::new(build).color(p.weak));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Details…").clicked() {
                        self.show_about = true;
                    }
                });
            });
        });
    }

    /// Put settings to use and write them to disk. Returns whether the write worked; if it
    /// didn't, the settings still apply for now and the periodic save tries again.
    pub(super) fn apply_config(&mut self, cfg: Config) -> bool {
        let written = cfg.save(&self.paths);
        let saved = written.is_ok();
        if let Err(e) = written {
            self.toast(theme::ToastKind::Error, format!("Could not save settings: {e:#}"));
        }
        let providers_changed = cfg.providers != self.config.providers || cfg.local_folders != self.config.local_folders;
        let look_changed = cfg.theme != self.config.theme || cfg.accent != self.config.accent;
        if saved {
            self.saved_config = cfg.clone();
        }
        self.config = cfg.clone();
        // Changes made outside the settings page (theme button, tray) must not read as unsaved
        // edits there, or be undone by saving the page.
        if let Some(d) = &mut self.settings_draft {
            d.theme = cfg.theme;
            d.accent = cfg.accent;
            d.rotation.enabled = cfg.rotation.enabled;
        }
        self.refresh_providers();
        self.worker.send(crate::worker::Command::Reconfigure(cfg));
        if look_changed {
            self.pal = Palette::new(self.config.theme, self.config.accent);
            self.theme_dirty = true;
        }
        if providers_changed {
            self.collections.clear();
            self.collections_requested.clear();
            if !self.enabled.iter().any(|i| i.id == self.provider) {
                self.provider = self.enabled.first().map(|i| i.id.to_string()).unwrap_or_default();
            }
            self.actions.push(Action::NewSearch);
        }
        saved
    }
}

const INTERVALS: &[(u32, &str)] =
    &[(15, "15 min"), (30, "30 min"), (60, "1 hour"), (180, "3 hours"), (360, "6 hours"), (720, "12 hours"), (1440, "Daily")];

impl App {
    fn rotation_card(&mut self, ui: &mut egui::Ui, p: &Palette, draft: &mut Config) {
        use crate::config::RotationSource;
        theme::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::card_title(ui, p, ph::ARROWS_CLOCKWISE, "Rotation");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(with_icon(ph::SKIP_FORWARD, "Next wallpaper now")).clicked() {
                        let _ = self.remote_tx.send(super::Remote::Next);
                    }
                });
            });
            ui.label(RichText::new("Change the desktop wallpaper automatically.").color(p.weak));
            ui.add_space(6.0);
            ui.checkbox(&mut draft.rotation.enabled, "Rotate automatically");
            ui.add_enabled_ui(draft.rotation.enabled, |ui| {
                egui::Grid::new("rotation").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                    ui.label("Every");
                    ui.horizontal_wrapped(|ui| {
                        for (m, l) in INTERVALS {
                            if theme::chip(ui, p, draft.rotation.interval_minutes == *m, *l).clicked() {
                                draft.rotation.interval_minutes = *m;
                            }
                        }
                        ui.add(egui::DragValue::new(&mut draft.rotation.interval_minutes).range(1..=10080).suffix(" min"));
                    });
                    ui.end_row();

                    ui.label("From");
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            let src = &mut draft.rotation.source;
                            if theme::chip(ui, p, *src == RotationSource::Kept, format!("Kept ({})", self.counts[2])).clicked() {
                                *src = RotationSource::Kept;
                            }
                            if theme::chip(ui, p, *src == RotationSource::Considering, format!("Shortlist ({})", self.counts[1]))
                                .clicked()
                            {
                                *src = RotationSource::Considering;
                            }
                            let is_tag = matches!(src, RotationSource::Tag(_));
                            if theme::chip(ui, p, is_tag, "Tag").clicked() && !is_tag {
                                *src = RotationSource::Tag(self.all_tags.first().map(|t| t.0.clone()).unwrap_or_default());
                            }
                            let is_saved = matches!(src, RotationSource::SavedSearch(_));
                            if theme::chip(ui, p, is_saved, "Saved search").clicked() && !is_saved {
                                *src = RotationSource::SavedSearch(self.saved.first().map(|s| s.id).unwrap_or(0));
                            }
                            let is_prov = matches!(src, RotationSource::Provider(_));
                            if theme::chip(ui, p, is_prov, "Online source").clicked() && !is_prov {
                                *src = RotationSource::Provider(self.enabled.first().map(|i| i.id.to_string()).unwrap_or_default());
                            }
                        });
                        match &mut draft.rotation.source {
                            RotationSource::Tag(t) => {
                                let resp = egui::ComboBox::from_id_salt("rot_tag")
                                    .selected_text(if t.is_empty() { "choose a tag".to_string() } else { t.clone() })
                                    .show_ui(ui, |ui| {
                                        for (name, n) in &self.all_tags {
                                            ui.selectable_value(t, name.clone(), format!("{name} ({n})"));
                                        }
                                    })
                                    .response;
                                theme::name_widget(&resp, egui::Role::ComboBox, "Rotation tag");
                            }
                            RotationSource::SavedSearch(id) => {
                                let cur = self
                                    .saved
                                    .iter()
                                    .find(|s| s.id == *id)
                                    .map(|s| s.name.clone())
                                    .unwrap_or("choose a saved search".into());
                                let resp = egui::ComboBox::from_id_salt("rot_saved")
                                    .selected_text(cur)
                                    .show_ui(ui, |ui| {
                                        for s in &self.saved {
                                            ui.selectable_value(id, s.id, &s.name);
                                        }
                                    })
                                    .response;
                                theme::name_widget(&resp, egui::Role::ComboBox, "Rotation saved search");
                                if self.saved.is_empty() {
                                    ui.label(RichText::new("Save a search in Browse first.").color(p.weak).size(12.0));
                                }
                            }
                            RotationSource::Provider(pid) => {
                                let cur = self.provider_name(pid);
                                let resp = egui::ComboBox::from_id_salt("rot_provider")
                                    .selected_text(cur)
                                    .show_ui(ui, |ui| {
                                        for i in &self.enabled {
                                            ui.selectable_value(pid, i.id.to_string(), i.name);
                                        }
                                    })
                                    .response;
                                theme::name_widget(&resp, egui::Role::ComboBox, "Rotation source");
                                ui.label(RichText::new("Picks from the source's default feed.").color(p.weak).size(12.0));
                            }
                            _ => {}
                        }
                    });
                    ui.end_row();

                    let library_source = crate::rotation::library_filter(&draft.rotation).is_some();
                    ui.label("Only");
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut draft.rotation.landscape_only, "landscape");
                        if library_source {
                            ui.label("· at least");
                            for r in 0..=5u8 {
                                let l = if r == 0 { "any".to_string() } else { format!("{r}★") };
                                if theme::chip(ui, p, draft.rotation.min_rating == r, l).clicked() {
                                    draft.rotation.min_rating = r;
                                }
                            }
                        }
                    });
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut draft.rotation.change_on_start, "Change when walle starts");
                    ui.end_row();
                    if self.displays.len() > 1 || draft.rotation.per_display {
                        ui.label("");
                        ui.checkbox(&mut draft.rotation.per_display, "A different wallpaper on each display")
                            .on_hover_text("Off: every display gets the same wallpaper (or one picture across all of them with the Span fit mode).");
                        ui.end_row();
                    }
                });
            });
            if let Some(t) = self.rotation_next {
                let local: chrono::DateTime<chrono::Local> = t.into();
                ui.label(RichText::new(format!("Next change at {}", local.format("%H:%M"))).color(p.weak));
            }
        });
    }

    /// Effects walle renders into the wallpaper on the desktop. They follow the draft only
    /// once it is saved: rendering every display on each slider step would lag behind.
    fn effects_card(&mut self, ui: &mut egui::Ui, p: &Palette, draft: &mut Config) {
        theme::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::card_title(ui, p, ph::SPARKLE, "Effects");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let plain = crate::effects::Effects::default();
                    if ui.add_enabled(draft.effects != plain, egui::Button::new("Reset")).on_hover_text("Back to the picture as it is").clicked() {
                        draft.effects = plain;
                    }
                });
            });
            ui.label(
                RichText::new(
                    "Rendered into a copy of the wallpaper when it is set; the original stays as it is. Saving applies them to what is on the desktop now.",
                )
                .color(p.weak),
            );
            ui.add_space(6.0);
            let fx = &mut draft.effects;
            egui::Grid::new("effects").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                let slider = |ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, hint: &str| {
                    ui.label(label);
                    let r = ui.add(egui::Slider::new(value, range).suffix(" %").fixed_decimals(0)).on_hover_text(hint);
                    theme::name_widget(&r, egui::Role::Slider, label);
                    ui.end_row();
                };
                slider(ui, "Blur", &mut fx.blur, 0.0..=100.0, "Softens the picture, so windows and text stand out in front of it");
                slider(ui, "Dim", &mut fx.dim, 0.0..=90.0, "Darkens the picture");
                slider(ui, "Saturation", &mut fx.saturation, 0.0..=200.0, "0 is black and white, 100 is the picture as it is");
                slider(ui, "Vignette", &mut fx.vignette, 0.0..=100.0, "Darkens the corners");
                ui.label("Tint");
                ui.horizontal(|ui| {
                    ui.color_edit_button_srgb(&mut fx.tint);
                    let r = ui.add(egui::Slider::new(&mut fx.tint_strength, 0.0..=100.0).suffix(" %").fixed_decimals(0));
                    theme::name_widget(&r, egui::Role::Slider, "Tint strength");
                });
                ui.end_row();
                ui.label("At night");
                ui.vertical(|ui| {
                    ui.checkbox(&mut fx.night.enabled, "Dimmer and warmer at night")
                        .on_hover_text("Comes in over an hour and goes out over an hour. walle changes the wallpaper as it does, so this one only moves while walle is running (the tray is enough).");
                    if fx.night.enabled {
                        ui.horizontal(|ui| {
                            ui.label("from");
                            ui.add(egui::DragValue::new(&mut fx.night.from_hour).range(0..=23).suffix(":00"));
                            ui.label("until");
                            ui.add(egui::DragValue::new(&mut fx.night.to_hour).range(0..=23).suffix(":00"));
                        });
                        ui.horizontal(|ui| {
                            ui.label("dimmer by");
                            ui.add(egui::DragValue::new(&mut fx.night.dim).range(0.0..=90.0).suffix(" %").fixed_decimals(0));
                            ui.label("warmer by");
                            ui.add(egui::DragValue::new(&mut fx.night.warmth).range(0.0..=100.0).suffix(" %").fixed_decimals(0));
                        });
                    }
                });
                ui.end_row();
            });
        });
    }

    fn system_card(&mut self, ui: &mut egui::Ui, p: &Palette, draft: &mut Config) {
        theme::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_title(ui, p, ph::POWER, "Startup & background");
            ui.add_space(6.0);
            let mut auto = self.autostart_on;
            let label =
                if cfg!(windows) {
                    "Start walle when I sign in (in the tray)"
                } else if cfg!(target_os = "macos") {
                    "Start walle when I log in (in the menu bar)"
                } else {
                    "Start walle when I log in (in the tray, without a window)"
                };
            if ui.checkbox(&mut auto, label).changed() {
                match crate::autostart::set_enabled(auto) {
                    Ok(()) => {
                        self.autostart_on = crate::autostart::is_enabled();
                        let msg = if auto { "walle will start with your session" } else { "Autostart disabled" };
                        self.toast(theme::ToastKind::Success, msg);
                    }
                    Err(e) => self.toast(theme::ToastKind::Error, format!("Autostart: {e:#}")),
                }
            }
            ui.add_enabled_ui(self.has_tray, |ui| {
                ui.checkbox(
                    &mut draft.close_to_tray,
                    if cfg!(target_os = "macos") {
                        "Closing the window keeps walle running in the menu bar"
                    } else {
                        "Closing the window keeps walle running in the tray"
                    },
                )
                    .on_disabled_hover_text("No panel in this session shows tray icons (walle offers a StatusNotifierItem)");
            });
            ui.checkbox(&mut draft.check_updates, "Check for a new version once a day").on_hover_text(
                "Asks GitHub for walle's latest release. The request carries nothing but walle's version, and nothing is downloaded until you click.",
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Thumbnail cache limit");
                ui.add(egui::DragValue::new(&mut draft.cache_limit_mb).range(50..=20_000).suffix(" MB"));
                if let Some(b) = self.cache_bytes {
                    ui.label(RichText::new(format!("using {}", theme::human_size(b))).color(p.weak));
                }
                if ui.button("Clear cache").clicked() {
                    self.worker.send(crate::worker::Command::PruneCache { clear: true });
                    self.textures.clear();
                }
            });
            ui.horizontal(|ui| {
                ui.label("Downloads of wallpapers you did not keep");
                if ui
                    .button("Clean up")
                    .on_hover_text("Deletes downloaded files of wallpapers that are not Kept (the current wallpaper is kept)")
                    .clicked()
                {
                    self.actions.push(Action::CleanDownloads);
                }
            });
        });
    }
}
