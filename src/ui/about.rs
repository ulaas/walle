//! The About box: what this build is, where its data lives, and who made the parts.
use egui::{Frame, Margin, RichText, Vec2, vec2};

use super::{
    Action, App,
    theme::{self, ph, semibold},
};
use crate::build_info;

/// Width of the first column of the info rows.
const LABEL_WIDTH: f32 = 96.0;

const SHORTCUTS: &[(&str, &str)] = &[
    ("Arrow keys", "Move the selection"),
    ("Space", "Full-size preview"),
    ("Enter", "Set as wallpaper"),
    ("C / K / X", "Consider / keep / reject"),
    ("1 – 5, 0", "Rate / clear rating"),
    ("D", "Download"),
    ("Ctrl+click, Shift+click", "Add to the selection"),
    ("Ctrl+A", "Select everything"),
    ("Esc", "Close / clear the selection"),
    ("F1", "This window"),
];

const CREDITS: &[(&str, &str)] = &[
    ("egui", "UI toolkit, MIT or Apache 2.0"),
    ("Inter", "Typeface, SIL Open Font License 1.1"),
    ("Phosphor Icons", "Icon set, MIT"),
];

impl App {
    pub(super) fn about_ui(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        let p = self.pal;
        let frame = Frame::default()
            .fill(p.surface)
            .stroke(egui::Stroke::new(1.0, p.border))
            .corner_radius(egui::CornerRadius::same(14))
            .inner_margin(Margin::same(22))
            .shadow(ctx.global_style().visuals.window_shadow);
        // The buttons stay in view; what doesn't fit above them scrolls.
        let max_height = (ctx.content_rect().height() - 150.0).max(160.0);
        let id = egui::Id::unique("walle-about");
        // Areas assume 400 points of height unless told otherwise, which would cut the box off.
        let area = egui::Modal::default_area(id).default_size(vec2(420.0, max_height + 60.0));
        let modal = egui::Modal::new(id).area(area).frame(frame).show(ctx, |ui| {
            ui.set_width(420.0);
            egui::ScrollArea::vertical().max_height(max_height).auto_shrink([false, true]).show(ui, |ui| self.about_contents(ui));
            ui.add_space(14.0);
            self.about_buttons(ui);
        });
        if modal.should_close() {
            self.show_about = false;
        }
    }

    fn about_contents(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        ui.horizontal(|ui| {
            let (logo, _) = ui.allocate_exact_size(Vec2::splat(56.0), egui::Sense::hover());
            crate::icon::paint(ui.painter(), logo, self.config.accent);
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.add_space(4.0);
                ui.label(semibold("walle").size(23.0));
                ui.label(RichText::new(build_info::DESCRIPTION).color(p.weak));
            });
        });
        ui.add_space(14.0);

        let row = |ui: &mut egui::Ui, label: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(LABEL_WIDTH, 20.0), egui::Sense::hover());
                ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(13.5), p.weak);
                add(ui);
            });
        };
        row(ui, "Version", &mut |ui| {
            ui.label(semibold(build_info::VERSION));
            if build_info::PROFILE != "release" {
                theme::pill(ui, p.warning, build_info::PROFILE);
            }
        });
        row(ui, "Commit", &mut |ui| {
            let commit = ui.label(RichText::new(build_info::commit_label()).monospace());
            if !build_info::COMMIT.is_empty() {
                commit.on_hover_text(build_info::COMMIT);
                if ui
                    .add(egui::Button::new(RichText::new(ph::COPY).size(14.0)).frame_when_inactive(false))
                    .on_hover_text("Copy the full commit hash")
                    .clicked()
                {
                    ui.ctx().copy_text(build_info::COMMIT.to_string());
                    self.toast(theme::ToastKind::Success, "Commit hash copied");
                }
            }
        });
        if !build_info::COMMIT_DATE.is_empty() {
            row(ui, "Committed", &mut |ui| {
                ui.label(build_info::COMMIT_DATE);
            });
        }
        row(ui, "Platform", &mut |ui| {
            ui.label(build_info::TARGET);
        });
        row(ui, "Compiler", &mut |ui| {
            ui.label(build_info::COMPILER);
        });
        row(ui, "License", &mut |ui| {
            ui.label(build_info::LICENSE);
        });
        row(ui, "Updates", &mut |ui| {
            ui.vertical(|ui| self.about_update(ui));
        });

        ui.add_space(6.0);
        ui.separator();
        ui.add_space(6.0);

        let [_, considering, kept, rejected] = self.counts;
        let enabled = self.enabled.len();
        let total = self.infos.len();
        row(ui, "Library", &mut |ui| {
            ui.label(format!("{kept} kept · {considering} shortlisted · {rejected} rejected"));
        });
        row(ui, "Sources", &mut |ui| {
            ui.label(format!("{enabled} of {total} enabled"));
        });
        let data_dir = self.paths.data_dir.clone();
        row(ui, "Data folder", &mut |ui| {
            let shown = data_dir.display().to_string();
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            let link = ui.link(shown.as_str()).on_hover_text("Open this folder");
            if link.clicked() {
                self.actions.push(Action::OpenPath(data_dir.clone()));
            }
        });

        ui.add_space(8.0);
        egui::CollapsingHeader::new(RichText::new(theme::with_icon(ph::KEYBOARD, "Keyboard shortcuts")).color(p.weak))
            .id_salt("about-shortcuts")
            .show(ui, |ui| {
                egui::Grid::new("about-shortcuts-grid").num_columns(2).spacing([16.0, 5.0]).show(ui, |ui| {
                    for (keys, what) in SHORTCUTS {
                        ui.label(RichText::new(*keys).size(12.5));
                        ui.label(RichText::new(*what).size(12.5).color(p.weak));
                        ui.end_row();
                    }
                });
            });
        egui::CollapsingHeader::new(RichText::new(theme::with_icon(ph::HEART, "Credits")).color(p.weak))
            .id_salt("about-credits")
            .show(ui, |ui| {
                egui::Grid::new("about-credits-grid").num_columns(2).spacing([16.0, 5.0]).show(ui, |ui| {
                    for (name, what) in CREDITS {
                        ui.label(RichText::new(*name).size(12.5));
                        ui.label(RichText::new(*what).size(12.5).color(p.weak));
                        ui.end_row();
                    }
                });
            });
    }

    /// The update check, the update on offer and its progress.
    fn about_update(&mut self, ui: &mut egui::Ui) {
        use crate::update::{Phase, Stage};
        let p = self.pal;
        let phase = self.updater.phase().clone();
        let version = self.updater.version().map(str::to_string);
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        match (&phase, version) {
            (Phase::Checking, _) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Checking…").color(p.weak));
                });
            }
            (Phase::Installing, _) => {
                let (stage, (done, total)) = self.updater.progress().map_or((Stage::Checking, (0, 0)), |pr| (pr.stage(), pr.bytes()));
                let what = match stage {
                    Stage::Checking => "Looking up the release…".to_string(),
                    Stage::Downloading if total > 0 => {
                        format!("Downloading: {} of {}", theme::human_size(done), theme::human_size(total))
                    }
                    Stage::Downloading => "Downloading…".to_string(),
                    Stage::Verifying => "Checking the download…".to_string(),
                    Stage::Installing => "Installing…".to_string(),
                };
                ui.label(what);
                let fraction = if total > 0 { done as f32 / total as f32 } else { 0.0 };
                ui.add(egui::ProgressBar::new(fraction).desired_width(220.0).desired_height(6.0));
                if ui.button("Cancel").clicked() {
                    self.updater.cancel();
                }
            }
            (Phase::Ready, _) => {
                ui.label("Restarting…");
            }
            (Phase::Available | Phase::Failed(_), Some(version)) => {
                let kind = self.updater.kind();
                ui.label(semibold(format!("walle {version} is available")));
                if let Phase::Failed(error) = &phase {
                    ui.label(RichText::new(format!("The update was not installed: {error}")).color(p.danger));
                }
                ui.label(RichText::new(kind.advice()).size(12.5).color(p.weak));
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    if let Some(action) = kind.action()
                        && theme::primary_button(ui, &p, action).clicked()
                    {
                        self.install_update();
                    }
                    if ui.link("Release page").clicked() {
                        self.actions.push(Action::OpenUrl(self.updater.page().to_string()));
                    }
                });
            }
            (phase, _) => {
                match phase {
                    Phase::UpToDate => {
                        ui.label(RichText::new("walle is up to date").color(p.weak));
                    }
                    Phase::CheckFailed(error) | Phase::Failed(error) => {
                        ui.label(RichText::new(format!("Could not check: {error}")).color(p.danger));
                    }
                    _ => {}
                }
                if ui.button(theme::with_icon(ph::ARROWS_CLOCKWISE, "Check for updates")).clicked() {
                    self.check_for_updates(true);
                }
            }
        }
    }

    fn about_buttons(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        ui.horizontal(|ui| {
            if ui.button(theme::with_icon(ph::COPY, "Copy details")).on_hover_text("Version information for a bug report").clicked() {
                ui.ctx().copy_text(build_info::details());
                self.toast(theme::ToastKind::Success, "Details copied");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::primary_button(ui, &p, "Close").clicked() {
                    self.show_about = false;
                }
            });
        });
    }
}
