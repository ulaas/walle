//! The app icon and logo. The drawing is in `icon_art`; this hands it to the window and UI.

pub use crate::icon_art::rgba;

pub fn window_icon(accent: [u8; 3]) -> egui::IconData {
    egui::IconData { rgba: rgba(64, accent), width: 64, height: 64 }
}

/// Draw the logo inside the UI, drawn for the exact size on screen so it stays crisp.
pub fn paint(painter: &egui::Painter, rect: egui::Rect, accent: [u8; 3]) {
    use egui::emath::GuiRounding;
    let ctx = painter.ctx();
    let ppp = ctx.pixels_per_point();
    let rect = rect.round_to_pixels(ppp);
    let px = (rect.width() * ppp).round().max(1.0) as u32;
    let id = egui::Id::NULL.with(("walle_logo", px));
    let cached = ctx.data(|d| d.get_temp::<([u8; 3], egui::TextureHandle)>(id)).filter(|(a, _)| *a == accent);
    let texture = cached.map(|(_, t)| t).unwrap_or_else(|| {
        let side = px as usize;
        let image = egui::ColorImage::from_rgba_unmultiplied([side, side], &rgba(px, accent));
        let texture = ctx.load_texture("walle_logo", image, egui::TextureOptions::LINEAR);
        ctx.data_mut(|d| d.insert_temp(id, (accent, texture.clone())));
        texture
    });
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    painter.image(texture.id(), rect, uv, egui::Color32::WHITE);
}

#[cfg(test)]
mod tests {
    #[test]
    fn icon_has_transparent_corners_and_opaque_centre() {
        let px = super::rgba(32, [139, 124, 246]);
        assert_eq!(px.len(), 32 * 32 * 4);
        assert_eq!(px[3], 0, "top-left corner transparent");
        let c = ((16 * 32 + 16) * 4) as usize;
        assert_eq!(px[c + 3], 255, "centre opaque");
    }
}
