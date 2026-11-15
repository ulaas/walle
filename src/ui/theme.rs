//! Visual identity: fonts, palette, widget styling, small reusable widgets, toasts.
use std::sync::Arc;

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Response, RichText, Shadow,
    Stroke, TextStyle, Ui, Vec2, vec2,
};
pub use super::icons as ph;

use crate::config::Theme;

pub const SEMIBOLD: &str = "semibold";
/// Filled variant of the icon font; the code points are the same as `ph`.
pub const ICON_FILL: &str = "phosphor-fill";

pub const RADIUS: u8 = 8;

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: Color32,
    pub surface: Color32,
    pub surface2: Color32,
    pub hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub weak: Color32,
    pub sunken: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub dark: bool,
}

/// sRGB channel to linear light, for contrast decisions.
fn linear(c: u8) -> f32 {
    let c = f32::from(c) / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn on_colour(c: Color32) -> Color32 {
    let luminance = 0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b());
    // Only the really light accents (amber) need dark text; white reads better on the others.
    if luminance > 0.42 { Color32::from_rgb(0x1a, 0x14, 0x05) } else { Color32::WHITE }
}

impl Palette {
    pub fn new(theme: Theme, accent: [u8; 3]) -> Self {
        let accent = Color32::from_rgb(accent[0], accent[1], accent[2]);
        let on_accent = on_colour(accent);
        match theme {
            Theme::Dark => Self {
                bg: Color32::from_rgb(0x0f, 0x11, 0x15),
                surface: Color32::from_rgb(0x16, 0x19, 0x1f),
                surface2: Color32::from_rgb(0x20, 0x24, 0x2c),
                hover: Color32::from_rgb(0x2b, 0x30, 0x3a),
                border: Color32::from_rgb(0x28, 0x2c, 0x35),
                text: Color32::from_rgb(0xe8, 0xea, 0xf0),
                weak: Color32::from_rgb(0x8f, 0x96, 0xa6),
                sunken: Color32::from_rgb(0x0c, 0x0e, 0x12),
                accent,
                on_accent,
                success: Color32::from_rgb(0x4c, 0xc7, 0x7b),
                warning: Color32::from_rgb(0xf0, 0xb4, 0x2c),
                danger: Color32::from_rgb(0xee, 0x5f, 0x5f),
                dark: true,
            },
            Theme::Light => Self {
                bg: Color32::from_rgb(0xf4, 0xf5, 0xf8),
                surface: Color32::WHITE,
                surface2: Color32::from_rgb(0xec, 0xee, 0xf2),
                hover: Color32::from_rgb(0xe0, 0xe3, 0xea),
                border: Color32::from_rgb(0xdc, 0xdf, 0xe6),
                text: Color32::from_rgb(0x1a, 0x1d, 0x25),
                weak: Color32::from_rgb(0x62, 0x69, 0x78),
                sunken: Color32::from_rgb(0xee, 0xf0, 0xf4),
                accent,
                on_accent,
                success: Color32::from_rgb(0x1f, 0x9d, 0x57),
                warning: Color32::from_rgb(0xc9, 0x85, 0x0c),
                danger: Color32::from_rgb(0xd6, 0x3b, 0x3b),
                dark: false,
            },
        }
    }

    pub fn accent_soft(&self) -> Color32 {
        self.accent.gamma_multiply(if self.dark { 0.26 } else { 0.16 })
    }

    /// The accent for text and icons: darker on light backgrounds, where it would wash out.
    pub fn accent_text(&self) -> Color32 {
        if self.dark {
            self.accent
        } else {
            let d = |c: u8| (f32::from(c) * 0.72) as u8;
            Color32::from_rgb(d(self.accent.r()), d(self.accent.g()), d(self.accent.b()))
        }
    }
}

pub const ACCENTS: &[[u8; 3]] = &[
    [139, 124, 246], // violet
    [96, 140, 255],  // blue
    [45, 180, 170],  // teal
    [80, 190, 110],  // green
    [240, 170, 50],  // amber
    [240, 110, 90],  // coral
    [230, 90, 150],  // pink
];

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("inter".into(), Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Inter-400.ttf"))));
    fonts.font_data.insert(
        "inter-semibold".into(),
        Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Inter-600.ttf"))),
    );
    let fallbacks = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    fonts.font_data.insert("phosphor".into(), Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Phosphor.ttf"))));
    fonts.font_data.insert(ICON_FILL.into(), Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Phosphor-Fill.ttf"))));
    // Icons mix into any text, ahead of the emoji fallbacks.
    let family = |first: &str, icons: &str| -> Vec<String> {
        [first.to_string(), icons.to_string()].into_iter().chain(fallbacks.iter().cloned()).collect()
    };
    fonts.families.insert(FontFamily::Proportional, family("inter", "phosphor"));
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), family("inter-semibold", "phosphor"));
    fonts.families.insert(FontFamily::Name(ICON_FILL.into()), family("inter", ICON_FILL));
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context, p: &Palette) {
    let theme = if p.dark { egui::Theme::Dark } else { egui::Theme::Light };
    ctx.set_theme(theme);
    ctx.style_mut_of(theme, |style| {
        use TextStyle::*;
        style.text_styles = [
            (Small, FontId::proportional(11.5)),
            (Body, FontId::proportional(13.5)),
            (Button, FontId::proportional(13.5)),
            (Monospace, FontId::monospace(12.5)),
            (Heading, FontId::new(19.0, FontFamily::Name(SEMIBOLD.into()))),
        ]
        .into();

        // This is an app, not a page: dragging across it shouldn't select text.
        style.interaction.selectable_labels = false;
        style.interaction.tooltip_delay = 0.35;

        let s = &mut style.spacing;
        s.item_spacing = vec2(8.0, 7.0);
        s.button_padding = vec2(10.0, 5.0);
        s.interact_size = vec2(36.0, 26.0);
        s.window_margin = Margin::same(14);
        s.menu_margin = Margin::same(6);
        s.combo_width = 140.0;
        s.slider_width = 140.0;
        s.scroll.bar_width = 8.0;
        s.scroll.floating_width = 3.0;

        let v = &mut style.visuals;
        v.dark_mode = p.dark;
        v.panel_fill = p.bg;
        v.window_fill = p.surface;
        v.faint_bg_color = p.surface;
        v.extreme_bg_color = p.sunken;
        v.text_edit_bg_color = Some(p.sunken);
        v.code_bg_color = p.surface2;
        v.hyperlink_color = p.accent_text();
        v.override_text_color = None;
        v.window_corner_radius = CornerRadius::same(14);
        v.menu_corner_radius = CornerRadius::same(10);
        v.window_stroke = Stroke::new(1.0, p.border);
        let shadow = Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha(if p.dark { 130 } else { 45 }) };
        v.window_shadow = shadow;
        v.popup_shadow = Shadow { offset: [0, 6], blur: 18, spread: 0, color: shadow.color };
        v.selection.bg_fill = p.accent_soft();
        v.selection.stroke = Stroke::new(1.5, p.accent_text());
        v.slider_trailing_fill = true;

        let r = CornerRadius::same(RADIUS);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = p.surface;
        w.noninteractive.weak_bg_fill = p.surface;
        w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
        w.noninteractive.corner_radius = r;

        // bg_fill is for small things like check boxes, weak_bg_fill for buttons.
        w.inactive.bg_fill = p.hover;
        w.inactive.weak_bg_fill = p.surface2;
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0, p.text);
        w.inactive.corner_radius = r;

        w.hovered.bg_fill = p.hover;
        w.hovered.weak_bg_fill = p.hover;
        w.hovered.bg_stroke = Stroke::new(1.0, p.accent.gamma_multiply(0.45));
        w.hovered.fg_stroke = Stroke::new(1.5, p.text);
        w.hovered.corner_radius = r;
        w.hovered.expansion = 0.0;

        w.active.bg_fill = p.accent;
        w.active.weak_bg_fill = p.accent;
        w.active.bg_stroke = Stroke::new(1.0, p.accent);
        w.active.fg_stroke = Stroke::new(1.5, p.on_accent);
        w.active.corner_radius = r;
        w.active.expansion = 0.0;

        w.open.bg_fill = p.hover;
        w.open.weak_bg_fill = p.hover;
        w.open.bg_stroke = Stroke::new(1.0, p.border);
        w.open.fg_stroke = Stroke::new(1.0, p.text);
        w.open.corner_radius = r;
    });
}

// ---------- small widgets ----------

pub fn semibold(text: impl Into<String>) -> RichText {
    RichText::new(text).family(FontFamily::Name(SEMIBOLD.into()))
}

pub fn semibold_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

pub fn fill_icon_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(ICON_FILL.into()))
}

/// `icon  text`, for buttons and menu items.
pub fn with_icon(icon: &str, text: &str) -> String {
    format!("{icon}  {text}")
}

/// Small, weak, upper-case section caption.
pub fn caption(ui: &mut Ui, p: &Palette, text: &str) {
    ui.add_space(2.0);
    ui.label(semibold(text.to_uppercase()).size(10.5).color(p.weak));
}

/// Card heading: accent icon + title.
pub fn card_title(ui: &mut Ui, p: &Palette, icon: &str, title: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(RichText::new(icon).size(17.0).color(p.accent_text()));
        ui.label(semibold(title).size(15.0));
    });
}

/// Filled accent call-to-action button.
pub fn primary_button(ui: &mut Ui, p: &Palette, text: impl Into<String>) -> Response {
    ui.add(
        egui::Button::new(semibold(text).color(p.on_accent))
            .fill(p.accent)
            .corner_radius(CornerRadius::same(RADIUS))
            .min_size(vec2(0.0, 30.0)),
    )
}

/// Rounded toggle chip.
pub fn chip(ui: &mut Ui, p: &Palette, selected: bool, text: impl Into<String>) -> Response {
    let text = text.into();
    let rt = if selected { semibold(text).color(p.accent_text()) } else { RichText::new(text).color(p.text) };
    let fill = if selected { p.accent_soft() } else { p.surface2 };
    let stroke = if selected { Stroke::new(1.0, p.accent.gamma_multiply(0.6)) } else { Stroke::NONE };
    ui.add(
        egui::Button::new(rt)
            .fill(fill)
            .stroke(stroke)
            .corner_radius(CornerRadius::same(255))
            .min_size(vec2(0.0, 25.0)),
    )
}

/// Non-interactive coloured pill label.
pub fn pill(ui: &mut Ui, color: Color32, text: impl Into<String>) -> Response {
    let text = text.into();
    let galley = ui.painter().layout_no_wrap(text.clone(), semibold_font(11.0), color);
    let size = galley.size() + vec2(14.0, 6.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &text));
    ui.painter().rect_filled(rect, CornerRadius::same(255), color.gamma_multiply(0.16));
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    resp
}

/// Frameless icon button with hover background.
pub fn icon_button(ui: &mut Ui, icon: &str, tooltip: &str) -> Response {
    let resp = ui
        .add(egui::Button::new(RichText::new(icon).size(16.0)).frame_when_inactive(false).min_size(Vec2::splat(30.0)))
        .on_hover_text(tooltip);
    name_widget(&resp, egui::Role::Button, tooltip);
    resp
}

pub fn card(p: &Palette) -> egui::Frame {
    egui::Frame::default()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(16))
}

/// A one-line galley cut to `max_width` with an ellipsis.
pub fn truncated(painter: &egui::Painter, text: String, font: FontId, color: Color32, max_width: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text, font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width.max(1.0));
    painter.layout_job(job)
}

// ---------- toasts ----------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

struct Toast {
    text: String,
    kind: ToastKind,
    born: f64,
}

#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
}

const TOAST_SECS: f64 = 4.0;
const TOAST_FADE_IN: f64 = 0.15;
const TOAST_FADE_OUT: f64 = 0.6;

impl Toasts {
    pub fn push(&mut self, kind: ToastKind, text: impl Into<String>) {
        let text = text.into();
        // Collapse repeats.
        self.items.retain(|t| t.text != text);
        self.items.push(Toast { text, kind, born: f64::NAN });
        if self.items.len() > 5 {
            self.items.remove(0);
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette) {
        let now = ctx.input(|i| i.time);
        for t in &mut self.items {
            if t.born.is_nan() {
                t.born = now;
            }
        }
        let life = |t: &Toast| if t.kind == ToastKind::Error { TOAST_SECS * 2.0 } else { TOAST_SECS };
        self.items.retain(|t| now - t.born < life(t));
        if self.items.is_empty() {
            return;
        }
        // Repaint smoothly while something fades, otherwise only when the next fade starts.
        let next_change = self
            .items
            .iter()
            .map(|t| {
                let age = now - t.born;
                let fade_out_at = life(t) - TOAST_FADE_OUT;
                if age < TOAST_FADE_IN || age >= fade_out_at { 0.0 } else { fade_out_at - age }
            })
            .fold(f64::INFINITY, f64::min);
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(next_change.max(0.033)));
        egui::Area::new(egui::Id::unique("walle-toasts"))
            .order(egui::Order::Tooltip)
            .anchor(egui::Align2::RIGHT_BOTTOM, vec2(-16.0, -42.0))
            .interactable(false)
            .show(ctx, |ui| {
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Max), |ui| {
                    for t in self.items.iter().rev() {
                        let age = now - t.born;
                        let fade = (1.0 - ((age - (life(t) - TOAST_FADE_OUT)) / TOAST_FADE_OUT).clamp(0.0, 1.0)) as f32;
                        let fade_in = (age / TOAST_FADE_IN).clamp(0.0, 1.0) as f32;
                        let a = fade * fade_in;
                        let (icon, color) = match t.kind {
                            ToastKind::Info => (ph::INFO, p.accent_text()),
                            ToastKind::Success => (ph::CHECK_CIRCLE, p.success),
                            ToastKind::Error => (ph::WARNING, p.danger),
                        };
                        egui::Frame::default()
                            .fill(p.surface.gamma_multiply(a))
                            .stroke(Stroke::new(1.0, p.border.gamma_multiply(a)))
                            .corner_radius(CornerRadius::same(10))
                            .inner_margin(Margin::symmetric(14, 10))
                            .shadow(Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha((90.0 * a) as u8) })
                            .show(ui, |ui| {
                                ui.set_max_width(420.0);
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(icon).color(color.gamma_multiply(a)).size(16.0));
                                    ui.add(egui::Label::new(RichText::new(&t.text).color(p.text.gamma_multiply(a))).wrap());
                                });
                            });
                        ui.add_space(6.0);
                    }
                });
            });
    }
}

/// Open a folder or file with the system's default handler.
pub fn open_path(path: &std::path::Path) {
    #[cfg(windows)]
    let r = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("/usr/bin/open").arg(path).spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(path).spawn();
    if let Err(e) = r {
        tracing::warn!("open {}: {e}", path.display());
    }
}

/// Open the folder containing `path` and select the file where supported.
pub fn reveal_path(path: &std::path::Path) {
    #[cfg(windows)]
    {
        // `/select,"C:\a b\c.jpg"`: explorer ignores the switch if the whole argument is quoted,
        // which is what `arg` does for paths with spaces.
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn();
    }
    #[cfg(target_os = "macos")]
    if let Err(e) = std::process::Command::new("/usr/bin/open").arg("-R").arg(path).spawn() {
        tracing::warn!("reveal {}: {e}", path.display());
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        if let Some(dir) = path.parent() {
            open_path(dir);
        }
    }
}

pub fn human_size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1_073_741_824.0 {
        format!("{:.1} GB", b / 1_073_741_824.0)
    } else if b >= 1_048_576.0 {
        format!("{:.1} MB", b / 1_048_576.0)
    } else {
        format!("{:.0} KB", b / 1024.0)
    }
}

/// Give an unlabeled widget (e.g. a ComboBox without a visible label) an accessible name.
pub fn name_widget(resp: &Response, kind: egui::Role, name: &str) {
    resp.widget_info(|| egui::WidgetInfo::labeled(kind, true, name));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_on_the_accent_stays_readable() {
        let on = |a: [u8; 3]| Palette::new(Theme::Dark, a).on_accent;
        assert_eq!(on([139, 124, 246]), Color32::WHITE, "violet");
        assert_ne!(on([240, 170, 50]), Color32::WHITE, "amber needs dark text");
    }
}
