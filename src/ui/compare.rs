//! Compare ("duel") mode: winner-stays-on head-to-head picking over a set of wallpapers.
use std::collections::VecDeque;

use egui::{Align2, CornerRadius, FontId, Key, Rect, RichText, Sense, Stroke, pos2, vec2};

use super::{
    Action, App,
    grid::status_color,
    theme::{self, ph, semibold, semibold_font, with_icon},
};
use crate::{model::Status, model::Wallpaper, worker::Command};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LoserAction {
    /// Loser goes to Rejected (hidden from future searches).
    #[default]
    Reject,
    /// Loser loses its Considering status.
    Unconsider,
    /// Nothing happens to the loser.
    Leave,
}

impl LoserAction {
    const ALL: [LoserAction; 3] = [LoserAction::Reject, LoserAction::Unconsider, LoserAction::Leave];
    fn label(self) -> &'static str {
        match self {
            LoserAction::Reject => "Reject loser",
            LoserAction::Unconsider => "Unshortlist loser",
            LoserAction::Leave => "Leave loser",
        }
    }
}

/// One decision, kept for undo.
struct Step {
    champion: Wallpaper,
    challenger: Wallpaper,
    /// (loser, its status before the decision) when a loser action was applied.
    loser: Option<(Wallpaper, Status)>,
    tie: bool,
}

pub struct Duel {
    pub champion: Wallpaper,
    pub challenger: Option<Wallpaper>,
    queue: VecDeque<Wallpaper>,
    ties: Vec<Wallpaper>,
    steps: Vec<Step>,
    pub loser_action: LoserAction,
    total: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Left,
    Right,
    Both,
}

impl Duel {
    pub fn new(mut items: Vec<Wallpaper>) -> Option<Self> {
        if items.len() < 2 {
            return None;
        }
        let total = items.len() - 1;
        let champion = items.remove(0);
        let mut queue: VecDeque<Wallpaper> = items.into();
        let challenger = queue.pop_front();
        Some(Self { champion, challenger, queue, ties: vec![], steps: vec![], loser_action: LoserAction::default(), total })
    }

    pub fn finished(&self) -> bool {
        self.challenger.is_none()
    }

    pub fn finalists(&self) -> Vec<Wallpaper> {
        let mut v = vec![self.champion.clone()];
        v.extend(self.ties.iter().cloned());
        v
    }

    /// Apply a pick. Returns the loser (if any) so the caller can update its status.
    pub fn pick(&mut self, pick: Pick, loser_prev: impl FnOnce(&Wallpaper) -> Status) -> Option<Wallpaper> {
        let challenger = self.challenger.take()?;
        let champion = self.champion.clone();
        let (winner, loser) = match pick {
            Pick::Left => (champion.clone(), Some(challenger.clone())),
            Pick::Right => (challenger.clone(), Some(champion.clone())),
            Pick::Both => (champion.clone(), None),
        };
        if pick == Pick::Both {
            self.ties.push(challenger.clone());
        }
        let loser_rec = match (&loser, self.loser_action) {
            (Some(l), LoserAction::Reject | LoserAction::Unconsider) => Some((l.clone(), loser_prev(l))),
            _ => None,
        };
        self.steps.push(Step { champion, challenger, loser: loser_rec, tie: pick == Pick::Both });
        self.champion = winner;
        self.challenger = self.queue.pop_front();
        match self.loser_action {
            LoserAction::Leave => None,
            _ => loser,
        }
    }

    /// Undo the last pick. Returns the loser and the status to restore.
    pub fn undo(&mut self) -> Option<Option<(Wallpaper, Status)>> {
        let step = self.steps.pop()?;
        if let Some(c) = self.challenger.take() {
            self.queue.push_front(c);
        }
        if step.tie {
            self.ties.pop();
        }
        self.champion = step.champion;
        self.challenger = Some(step.challenger);
        Some(step.loser)
    }

    pub fn progress(&self) -> (usize, usize) {
        (self.steps.len(), self.total)
    }
}

impl App {
    pub(super) fn start_duel(&mut self, items: Vec<Wallpaper>) {
        match Duel::new(items) {
            Some(d) => {
                self.lightbox = None;
                self.duel = Some(d);
            }
            None => self.toast(theme::ToastKind::Info, "Compare needs at least two wallpapers"),
        }
    }

    fn duel_pick(&mut self, pick: Pick) {
        let Some(mut duel) = self.duel.take() else { return };
        let action = duel.loser_action;
        let loser = duel.pick(pick, |w| self.lib.entry(&w.key()).ok().flatten().map(|e| e.status).unwrap_or_default());
        if let Some(l) = loser {
            let st = match action {
                LoserAction::Reject => Status::Rejected,
                _ => Status::None,
            };
            self.apply_status(&l, st);
        }
        self.duel = Some(duel);
    }

    fn duel_undo(&mut self) {
        let Some(mut duel) = self.duel.take() else { return };
        if let Some(Some((loser, prev))) = duel.undo() {
            self.apply_status(&loser, prev);
        }
        self.duel = Some(duel);
    }

    /// Keyboard handling while a duel is open. Returns true if the duel consumed input.
    pub(super) fn duel_keys(&mut self, keys: &[Key]) {
        for k in keys {
            let finished = self.duel.as_ref().is_some_and(|d| d.finished());
            match k {
                Key::Escape => self.duel = None,
                Key::ArrowLeft | Key::A | Key::Num1 if !finished => self.duel_pick(Pick::Left),
                Key::ArrowRight | Key::D | Key::Num2 if !finished => self.duel_pick(Pick::Right),
                Key::ArrowUp | Key::T if !finished => self.duel_pick(Pick::Both),
                Key::Backspace | Key::Z => self.duel_undo(),
                Key::Enter if finished => self.duel_finish(true),
                _ => {}
            }
        }
    }

    fn duel_finish(&mut self, keep: bool) {
        if let Some(d) = self.duel.take() {
            if keep {
                let finalists = d.finalists();
                for w in &finalists {
                    self.apply_status(w, Status::Kept);
                }
                self.toast(theme::ToastKind::Success, format!("Kept {} finalist(s)", finalists.len()));
            }
            self.lib_dirty = true;
        }
    }

    fn duel_texture(&mut self, wp: &Wallpaper) -> Option<egui::TextureHandle> {
        let key = wp.key();
        let prev_key = format!("prev:{key}");
        let url = wp.preview_url.clone().unwrap_or_else(|| wp.thumb_url.clone());
        let fallback = self.entry(&key).local_path.clone();
        let worker = &self.worker;
        self.textures
            .get(&prev_key, || worker.send(Command::LoadImage { tex_key: prev_key.clone(), url, max_dim: 1600, fallback }))
            .or_else(|| self.textures.peek(&format!("thumb:{key}")))
    }

    /// Draw one contestant; returns true when clicked.
    fn duel_side(&mut self, ui: &mut egui::Ui, rect: Rect, wp: &Wallpaper, hint: &str, id: &str) -> bool {
        let p = self.pal;
        let resp = ui.interact(rect, ui.scope_id().with(id), Sense::click());
        let hover = ui.ctx().animate_bool_with_time(ui.scope_id().with((id, "h")), resp.hovered(), 0.12);
        let painter = ui.painter().clone();
        let frame = rect.shrink(10.0);
        painter.rect_filled(frame, CornerRadius::same(14), p.surface);
        painter.rect_stroke(frame, CornerRadius::same(14), Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
        let img_area = Rect::from_min_max(frame.min + vec2(12.0, 12.0), frame.max - vec2(12.0, 64.0));
        if let Some(t) = self.duel_texture(wp) {
            let size = t.size_vec2();
            let s = (img_area.width() / size.x).min(img_area.height() / size.y);
            let r = Rect::from_center_size(img_area.center(), size * s);
            egui::Image::from_texture((t.id(), size)).corner_radius(CornerRadius::same(10)).paint_at(ui, r);
        } else {
            let t = ui.input(|i| i.time) as f32;
            painter.rect_filled(img_area, CornerRadius::same(10), p.hover.gamma_multiply(0.45 + 0.25 * (t * 3.0).sin()));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(40));
        }
        if hover > 0.0 {
            painter.rect_stroke(frame, CornerRadius::same(14), Stroke::new(3.0 * hover, p.accent), egui::StrokeKind::Inside);
        }
        let entry = self.entry(&wp.key());
        let mut details = format!("{}   ·   {}", wp.resolution(), self.provider_name(&wp.provider));
        if entry.rating > 0 {
            details += &format!("   ·   {} {}", ph::STAR, entry.rating);
        }
        let title = theme::truncated(&painter, wp.display_title(), semibold_font(14.0), p.text, frame.width() * 0.5);
        let details = painter.layout_no_wrap(details, FontId::proportional(13.0), p.weak);
        let line_w = title.size().x + 14.0 + details.size().x;
        let mut x = frame.center().x - line_w / 2.0;
        let y = frame.bottom() - 40.0;
        let title_w = title.size().x;
        painter.galley(pos2(x, y - title.size().y / 2.0), title, p.text);
        x += title_w + 14.0;
        painter.galley(pos2(x, y - details.size().y / 2.0), details, p.weak);
        painter.text(pos2(frame.center().x, frame.bottom() - 18.0), Align2::CENTER_CENTER, hint, FontId::proportional(12.0), p.weak);
        resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
    }

    pub(super) fn duel_ui(&mut self, ctx: &egui::Context) {
        if self.duel.is_none() {
            return;
        }
        let p = self.pal;
        let screen = Self::below_title_bar(ctx);
        egui::Area::new(egui::Id::unique("walle-duel")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::hover());
            ui.painter().rect_filled(rect, 0.0, p.bg);

            // Toolbar.
            let bar = Rect::from_min_size(rect.min, vec2(rect.width(), 56.0));
            let mut top = ui.new_child(
                egui::UiBuilder::new().max_rect(bar.shrink2(vec2(18.0, 12.0))).layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            let (done, total) = self.duel.as_ref().map(|d| d.progress()).unwrap_or((0, 0));
            top.label(semibold(with_icon(ph::SCALES, "Compare")).size(17.0));
            top.add_space(10.0);
            top.label(RichText::new(format!("{done} of {total} decided")).color(p.weak));
            top.add_space(16.0);
            let mut la = self.duel.as_ref().map(|d| d.loser_action).unwrap_or_default();
            for a in LoserAction::ALL {
                if theme::chip(&mut top, &p, la == a, a.label()).clicked() {
                    la = a;
                }
            }
            if let Some(d) = &mut self.duel {
                d.loser_action = la;
            }
            top.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::icon_button(ui, ph::X, "Exit compare (Esc)").clicked() {
                    self.duel = None;
                }
                if ui.add_enabled(done > 0, egui::Button::new(with_icon(ph::ARROW_U_UP_LEFT, "Undo"))).on_hover_text("Backspace").clicked() {
                    self.duel_undo();
                }
            });
            // Progress line.
            if total > 0 {
                let w = rect.width() * done as f32 / total as f32;
                ui.painter().rect_filled(Rect::from_min_size(pos2(rect.left(), bar.bottom()), vec2(w, 3.0)), 0.0, p.accent);
            }
            let Some(duel) = &self.duel else { return };

            let body = Rect::from_min_max(pos2(rect.left() + 8.0, bar.bottom() + 8.0), rect.max - vec2(8.0, 8.0));
            if duel.finished() {
                let finalists = duel.finalists();
                self.duel_summary(ui, body, finalists);
                return;
            }
            let champion = duel.champion.clone();
            let challenger = duel.challenger.clone().unwrap();
            // Prefetch the next challenger.
            if let Some(next) = duel.queue.front().cloned() {
                let _ = self.duel_texture(&next);
            }
            let half = body.width() / 2.0;
            let left = Rect::from_min_size(body.min, vec2(half, body.height()));
            let right = Rect::from_min_size(pos2(body.left() + half, body.top()), vec2(half, body.height()));
            let l = self.duel_side(ui, left, &champion, &format!("{}  or  1  ·  click to pick  ·  current favourite", ph::ARROW_LEFT), "left");
            let r = self.duel_side(ui, right, &challenger, &format!("{}  or  2  ·  click to pick  ·  challenger", ph::ARROW_RIGHT), "right");
            // "Keep both" in the middle.
            let mid = Rect::from_center_size(pos2(body.center().x, body.bottom() - 90.0), vec2(124.0, 32.0));
            let both = ui.put(mid, egui::Button::new(semibold(with_icon(ph::ARROW_UP, "Keep both"))).corner_radius(CornerRadius::same(255)));
            if l {
                self.duel_pick(Pick::Left);
            } else if r {
                self.duel_pick(Pick::Right);
            } else if both.clicked() {
                self.duel_pick(Pick::Both);
            }
        });
    }

    fn duel_summary(&mut self, ui: &mut egui::Ui, body: Rect, finalists: Vec<Wallpaper>) {
        let p = self.pal;
        let champion = finalists[0].clone();
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body.shrink(24.0)).layout(egui::Layout::top_down(egui::Align::Center)));
        child.label(semibold(with_icon(ph::TROPHY, "Winner")).size(22.0));
        child.add_space(10.0);
        let avail = child.available_size();
        let img_h = (avail.y - 200.0).max(160.0);
        if let Some(t) = self.duel_texture(&champion) {
            let size = t.size_vec2();
            let s = (avail.x * 0.8 / size.x).min(img_h / size.y);
            child.add(egui::Image::from_texture((t.id(), size)).fit_to_exact_size(size * s).corner_radius(CornerRadius::same(12)));
        }
        child.add_space(8.0);
        child.label(RichText::new(format!("{}  ·  {}", champion.display_title(), champion.resolution())).color(p.text));
        if finalists.len() > 1 {
            child.label(
                RichText::new(format!("+ {} other finalist(s) you kept alongside it", finalists.len() - 1)).color(p.weak),
            );
        }
        child.add_space(14.0);
        child.horizontal(|ui| {
            let w = 520.0;
            ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
            if theme::primary_button(ui, &p, with_icon(ph::CHECK, "Keep finalists (Enter)")).clicked() {
                self.duel_finish(true);
            }
            if ui.button(with_icon(ph::MONITOR, "Set winner as wallpaper")).clicked() {
                self.actions.push(Action::SetWallpaper(champion.clone()));
            }
            if ui.button(RichText::new(with_icon(ph::STAR, "Rate 5")).color(status_color(Status::Considering))).clicked() {
                let cur = self.entry(&champion.key()).rating;
                if cur != 5 {
                    self.actions.push(Action::Rate(champion.clone(), 5));
                }
            }
            if ui.button("Close").clicked() {
                self.duel_finish(false);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wp(id: &str) -> Wallpaper {
        Wallpaper { provider: "t".into(), remote_id: id.into(), ..Default::default() }
    }

    #[test]
    fn winner_stays_on_and_undo_restores() {
        let mut d = Duel::new(vec![wp("a"), wp("b"), wp("c"), wp("d")]).unwrap();
        assert_eq!(d.progress(), (0, 3));
        // b beats a
        let loser = d.pick(Pick::Right, |_| Status::Considering).unwrap();
        assert_eq!(loser.remote_id, "a");
        assert_eq!(d.champion.remote_id, "b");
        assert_eq!(d.challenger.as_ref().unwrap().remote_id, "c");
        // tie with c
        assert!(d.pick(Pick::Both, |_| Status::Considering).is_none());
        // b beats d
        d.pick(Pick::Left, |_| Status::Considering);
        assert!(d.finished());
        let f: Vec<_> = d.finalists().into_iter().map(|w| w.remote_id).collect();
        assert_eq!(f, vec!["b", "c"]);

        // Undo twice: back to b vs c with a as loser of the first round recorded.
        let last = d.undo().unwrap().unwrap();
        assert_eq!(last.0.remote_id, "d");
        assert!(d.undo().unwrap().is_none(), "tie had no loser");
        assert_eq!(d.champion.remote_id, "b");
        assert_eq!(d.challenger.as_ref().unwrap().remote_id, "c");
        assert_eq!(d.finalists().len(), 1);
        let first = d.undo().unwrap().unwrap();
        assert_eq!((first.0.remote_id.as_str(), first.1), ("a", Status::Considering));
        assert_eq!(d.champion.remote_id, "a");
        assert!(d.undo().is_none());
    }

    #[test]
    fn leave_action_returns_no_loser() {
        let mut d = Duel::new(vec![wp("a"), wp("b")]).unwrap();
        d.loser_action = LoserAction::Leave;
        assert!(d.pick(Pick::Left, |_| Status::None).is_none());
        assert!(d.finished());
        assert!(Duel::new(vec![wp("a")]).is_none());
    }
}
