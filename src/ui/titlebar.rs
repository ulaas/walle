//! The custom window frame: titlebar with logo, navigation and window buttons, plus the
//! resize edges a frameless window has to provide itself.
use egui::{
    Color32, CornerRadius, CursorIcon, Frame, Layout, Rect, ResizeDirection, RichText, Sense, Stroke, UiBuilder, Vec2,
    ViewportCommand, emath::GuiRounding, pos2, vec2,
};

use super::{
    Action, App, View,
    theme::{self, Palette, fill_icon_font, ph, semibold_font},
};

pub const HEIGHT: f32 = 44.0;
const WINDOW_BUTTON_WIDTH: f32 = 46.0;
/// Below this window width the navigation shows icons only, so that everything still fits.
const COMPACT_BELOW: f32 = 1000.0;
/// How far from the window edge resizing grabs, and how long the corners' grab areas are.
const RESIZE_EDGE: f32 = 5.0;
const RESIZE_CORNER: f32 = 12.0;

#[derive(Clone, Copy, PartialEq)]
enum WindowButton {
    Minimize,
    Maximize,
    Restore,
    Close,
}

impl WindowButton {
    fn label(self) -> &'static str {
        match self {
            Self::Minimize => "Minimize",
            Self::Maximize => "Maximize",
            Self::Restore => "Restore",
            Self::Close => "Close",
        }
    }
}

/// Draws its glyph as lines rather than from a font, to stay pixel-exact at any scale.
fn window_button(ui: &mut egui::Ui, p: &Palette, kind: WindowButton, focused: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(WINDOW_BUTTON_WIDTH, HEIGHT), Sense::click());
    theme::name_widget(&resp, egui::Role::Button, kind.label());
    let hot = resp.hovered() || resp.is_pointer_button_down_on();
    let painter = ui.painter();
    let close_red = Color32::from_rgb(0xe8, 0x11, 0x23);
    if hot {
        painter.rect_filled(rect, CornerRadius::ZERO, if kind == WindowButton::Close { close_red } else { p.hover });
    }
    let color = match (hot, kind) {
        (true, WindowButton::Close) => Color32::WHITE,
        (true, _) => p.text,
        (false, _) if focused => p.text,
        _ => p.weak,
    };
    let stroke = Stroke::new(1.0, color);
    let ppp = ui.pixels_per_point();
    let c = rect.center().round_to_pixel_center(ppp);
    let box_at = |center, half: f32| Rect::from_center_size(center, Vec2::splat(half * 2.0));
    match kind {
        WindowButton::Minimize => {
            painter.hline((c.x - 5.0)..=(c.x + 5.0), c.y, stroke);
        }
        WindowButton::Maximize => {
            painter.rect_stroke(box_at(c, 5.0), CornerRadius::same(1), stroke, egui::StrokeKind::Middle);
        }
        WindowButton::Restore => {
            // The front window, and the corner of the one behind it.
            painter.rect_stroke(box_at(c + vec2(-1.0, 1.0), 4.0), CornerRadius::same(1), stroke, egui::StrokeKind::Middle);
            painter.line(vec![c + vec2(-3.0, -3.0), c + vec2(-3.0, -5.0), c + vec2(5.0, -5.0), c + vec2(5.0, 3.0), c + vec2(3.0, 3.0)], stroke);
        }
        WindowButton::Close => {
            painter.line_segment([c + vec2(-5.0, -5.0), c + vec2(5.0, 5.0)], stroke);
            painter.line_segment([c + vec2(-5.0, 5.0), c + vec2(5.0, -5.0)], stroke);
        }
    }
    resp
}

impl App {
    pub(super) fn title_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        let ctx = ui.ctx().clone();
        let (maximized, focused) = ctx.input(|i| {
            let v = i.viewport();
            (v.maximized.unwrap_or(false), v.focused.unwrap_or(true))
        });
        egui::Panel::top("titlebar")
            .exact_size(HEIGHT)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::default().fill(p.surface))
            .show(ui, |ui| {
                let bar = ui.max_rect();
                // Registered first, so that everything drawn on the bar takes the input over it.
                let empty_area = ui.interact(bar, ui.scope_id().with("drag"), Sense::click_and_drag());
                if empty_area.drag_started_by(egui::PointerButton::Primary) {
                    ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                }
                if empty_area.double_clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                }
                empty_area.context_menu(|ui| self.window_menu(ui, maximized));
                ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, p.border));

                let compact = bar.width() < COMPACT_BELOW;
                ui.scope_builder(UiBuilder::new().max_rect(bar).layout(Layout::left_to_right(egui::Align::Center)), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    ui.add_space(10.0);
                    self.brand(ui, focused);
                    ui.add_space(14.0);
                    for (view, count) in [
                        (View::Browse, None),
                        (View::Considering, Some(self.counts[1])),
                        (View::Library, Some(self.counts[2])),
                        (View::Rejected, Some(self.counts[3])),
                        (View::History, None),
                    ] {
                        self.nav_tab(ui, view, count, compact);
                    }
                    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        if window_button(ui, &p, WindowButton::Close, focused).clicked() {
                            ctx.send_viewport_cmd(ViewportCommand::Close);
                        }
                        let kind = if maximized { WindowButton::Restore } else { WindowButton::Maximize };
                        if window_button(ui, &p, kind, focused).clicked() {
                            ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                        }
                        if window_button(ui, &p, WindowButton::Minimize, focused).clicked() {
                            ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                        }
                        ui.add_space(8.0);
                        ui.spacing_mut().item_spacing.x = 2.0;
                        self.nav_tab(ui, View::Settings, None, true);
                        let (icon, tip) = if p.dark { (ph::SUN, "Light theme") } else { (ph::MOON, "Dark theme") };
                        if theme::icon_button(ui, icon, tip).clicked() {
                            let mut cfg = self.config.clone();
                            cfg.theme = if p.dark { crate::config::Theme::Light } else { crate::config::Theme::Dark };
                            self.apply_config(cfg);
                        }
                    });
                });
            });
    }

    /// Logo and name; a click opens the About box.
    fn brand(&mut self, ui: &mut egui::Ui, focused: bool) {
        let p = self.pal;
        let name_color = if focused { p.text } else { p.weak };
        let name = ui.painter().layout_no_wrap("walle".into(), semibold_font(15.0), name_color);
        let size = vec2(6.0 + 24.0 + 8.0 + name.size().x + 8.0, 34.0);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        theme::name_widget(&resp, egui::Role::Button, "About walle");
        if resp.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(theme::RADIUS), p.hover);
        }
        let logo = Rect::from_center_size(pos2(rect.left() + 6.0 + 12.0, rect.center().y), Vec2::splat(24.0));
        crate::icon::paint(ui.painter(), logo, self.config.accent);
        let name_pos = pos2(logo.right() + 8.0, rect.center().y - name.size().y / 2.0);
        ui.painter().galley(name_pos, name, name_color);
        if resp.on_hover_text("About walle").clicked() {
            self.show_about = true;
        }
    }

    /// A navigation entry: icon (filled when active), name unless compact, count if any.
    fn nav_tab(&mut self, ui: &mut egui::Ui, view: View, count: Option<u32>, compact: bool) {
        let p = self.pal;
        let selected = self.view == view;
        let (fg, count_color) = if selected { (p.accent_text(), p.accent_text()) } else { (p.weak, p.weak) };
        let icon_font = if selected { fill_icon_font(17.0) } else { egui::FontId::proportional(17.0) };
        let painter = ui.painter().clone();
        let icon = painter.layout_no_wrap(view.icon().into(), icon_font, fg);
        let label = (!compact).then(|| {
            let font = if selected { semibold_font(13.5) } else { egui::FontId::proportional(13.5) };
            painter.layout_no_wrap(view.label().into(), font, fg)
        });
        let count_text = count.filter(|c| *c > 0).map(|c| painter.layout_no_wrap(c.to_string(), egui::FontId::proportional(11.5), count_color));

        let mut width = 10.0 + icon.size().x + 10.0;
        if let Some(l) = &label {
            width += 7.0 + l.size().x;
        }
        if let Some(c) = &count_text {
            width += 6.0 + c.size().x;
        }
        let (rect, resp) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, view.label()));
        if selected {
            painter.rect_filled(rect, CornerRadius::same(theme::RADIUS), p.accent_soft());
        } else if resp.hovered() {
            painter.rect_filled(rect, CornerRadius::same(theme::RADIUS), p.hover);
        }
        let hover_fg = if resp.hovered() && !selected { p.text } else { fg };
        let mut x = rect.left() + 10.0;
        let y = rect.center().y;
        let icon_w = icon.size().x;
        painter.galley(pos2(x, y - icon.size().y / 2.0), icon, hover_fg);
        x += icon_w + 7.0;
        if let Some(l) = label {
            let w = l.size().x;
            painter.galley(pos2(x, y - l.size().y / 2.0), l, hover_fg);
            x += w + 6.0;
        } else {
            x -= 1.0;
        }
        if let Some(c) = count_text {
            painter.galley(pos2(x, y - c.size().y / 2.0), c, count_color);
        }
        let mut resp = resp;
        if compact {
            resp = resp.on_hover_text(view.label());
        }
        if resp.clicked() && !selected {
            self.actions.push(Action::SwitchView(view));
        }
    }

    /// What a native titlebar offers on right-click.
    fn window_menu(&mut self, ui: &mut egui::Ui, maximized: bool) {
        ui.set_min_width(170.0);
        let ctx = ui.ctx().clone();
        let item = |ui: &mut egui::Ui, text: &str| {
            let clicked = ui.add(egui::Button::new(RichText::new(text)).frame_when_inactive(false)).clicked();
            if clicked {
                ui.close();
            }
            clicked
        };
        if item(ui, if maximized { "Restore" } else { "Maximize" }) {
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        }
        if item(ui, "Minimize") {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
        }
        ui.separator();
        if item(ui, "About walle") {
            self.show_about = true;
        }
        ui.separator();
        if item(ui, "Close") {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    /// Resizing from the window edges. Handled on raw input, drawn nowhere, and called after
    /// everything else so that its cursor wins.
    pub(super) fn window_resize_edges(&self, ctx: &egui::Context) {
        let (pos, pressed, any_down, maximized) = ctx.input(|i| {
            let v = i.viewport();
            (
                i.pointer.hover_pos(),
                i.pointer.primary_pressed(),
                i.pointer.any_down(),
                v.maximized.unwrap_or(false) || v.fullscreen.unwrap_or(false),
            )
        });
        let Some(pos) = pos else { return };
        if maximized {
            return;
        }
        let Some(direction) = resize_direction(ctx.content_rect(), pos) else { return };
        if !any_down || pressed {
            ctx.set_cursor_icon(resize_cursor(direction));
        }
        if pressed {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
        }
    }

    /// The area below the titlebar, for full-window overlays: the frame stays usable above them.
    pub(super) fn below_title_bar(ctx: &egui::Context) -> Rect {
        let mut r = ctx.content_rect();
        r.min.y += HEIGHT;
        r
    }

}

fn resize_direction(window: Rect, pos: egui::Pos2) -> Option<ResizeDirection> {
    if !window.contains(pos) {
        return None;
    }
    let near = |d: f32, within: f32| d <= within;
    let (left, right) = (pos.x - window.left(), window.right() - pos.x);
    let (top, bottom) = (pos.y - window.top(), window.bottom() - pos.y);
    // On an edge, a stretch near the corner resizes both ways: exact corners are hard to hit.
    let on_edge = near(left, RESIZE_EDGE) || near(right, RESIZE_EDGE) || near(top, RESIZE_EDGE) || near(bottom, RESIZE_EDGE);
    if !on_edge {
        return None;
    }
    let horizontal = if near(left, RESIZE_CORNER) {
        -1
    } else if near(right, RESIZE_CORNER) {
        1
    } else {
        0
    };
    let vertical = if near(top, RESIZE_CORNER) {
        -1
    } else if near(bottom, RESIZE_CORNER) {
        1
    } else {
        0
    };
    Some(match (horizontal, vertical) {
        (-1, -1) => ResizeDirection::NorthWest,
        (1, -1) => ResizeDirection::NorthEast,
        (-1, 1) => ResizeDirection::SouthWest,
        (1, 1) => ResizeDirection::SouthEast,
        (-1, _) => ResizeDirection::West,
        (1, _) => ResizeDirection::East,
        (_, -1) => ResizeDirection::North,
        _ => ResizeDirection::South,
    })
}

fn resize_cursor(direction: ResizeDirection) -> CursorIcon {
    match direction {
        ResizeDirection::North => CursorIcon::ResizeNorth,
        ResizeDirection::South => CursorIcon::ResizeSouth,
        ResizeDirection::East => CursorIcon::ResizeEast,
        ResizeDirection::West => CursorIcon::ResizeWest,
        ResizeDirection::NorthEast => CursorIcon::ResizeNorthEast,
        ResizeDirection::SouthEast => CursorIcon::ResizeSouthEast,
        ResizeDirection::NorthWest => CursorIcon::ResizeNorthWest,
        ResizeDirection::SouthWest => CursorIcon::ResizeSouthWest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_and_corners_map_to_directions() {
        let w = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 800.0));
        let at = |x, y| resize_direction(w, pos2(x, y));
        assert_eq!(at(500.0, 400.0), None, "the middle is not an edge");
        assert_eq!(at(500.0, 30.0), None, "the titlebar below the edge drags instead");
        assert_eq!(at(2.0, 400.0), Some(ResizeDirection::West));
        assert_eq!(at(998.0, 400.0), Some(ResizeDirection::East));
        assert_eq!(at(500.0, 2.0), Some(ResizeDirection::North));
        assert_eq!(at(500.0, 798.0), Some(ResizeDirection::South));
        assert_eq!(at(2.0, 2.0), Some(ResizeDirection::NorthWest));
        assert_eq!(at(998.0, 798.0), Some(ResizeDirection::SouthEast));
        assert_eq!(at(2.0, 10.0), Some(ResizeDirection::NorthWest), "corners are forgiving");
        assert_eq!(at(1200.0, 400.0), None, "outside the window");
    }
}
