//! The walle logo as pixels: a W on the top sheet of a pile of wallpapers. It has no
//! dependencies because build.rs uses it too, to make the icon embedded in the exe.
//! `assets/logo.svg` is the same drawing, so change the two together.

/// The default accent, which is also the colour of the icon embedded in the exe.
pub const DEFAULT_ACCENT: [u8; 3] = [139, 124, 246];

/// All in unit coordinates, y down.
const CORNER_RADIUS: f32 = 0.23;
/// x0, y0, x1, y1, corner radius.
const TOP_SHEET: (f32, f32, f32, f32, f32) = (0.10, 0.30, 0.90, 0.84, 0.075);
const BACK_SHEET: (f32, f32, f32, f32, f32) = (0.15, 0.20, 0.85, 0.72, 0.07);
const BACK_SHEET_TILT_DEGREES: f32 = 11.0;
const BACK_SHEET_OPACITY: f32 = 0.45;
const W: [(f32, f32); 5] = [(0.235, 0.43), (0.365, 0.71), (0.50, 0.51), (0.635, 0.71), (0.765, 0.43)];
const W_HALF_WIDTH: f32 = 0.055;
const WHITE: [f32; 3] = [255.0, 255.0, 255.0];
const BLACK: [f32; 3] = [0.0, 0.0, 0.0];

/// Straight-alpha RGBA pixels of a `size`×`size` logo.
pub fn rgba(size: u32, accent: [u8; 3]) -> Vec<u8> {
    let s = size as f32;
    let accent = accent.map(f32::from);
    // Distance to a shape in unit coordinates, to its coverage of this pixel.
    let cover = |d: f32| (0.5 - d * s).clamp(0.0, 1.0);
    let (sin, cos) = BACK_SHEET_TILT_DEGREES.to_radians().sin_cos();
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (u, v) = ((x as f32 + 0.5) / s, (y as f32 + 0.5) / s);
            // Lighter at the top, like a sky.
            let mut c = mix(mix(accent, WHITE, 0.14), mix(accent, BLACK, 0.40), v);
            let (cu, cv) = (u - 0.5, v - 0.5);
            let back = rounded_rect(0.5 + cu * cos - cv * sin, 0.5 + cu * sin + cv * cos, BACK_SHEET);
            c = mix(c, WHITE, BACK_SHEET_OPACITY * cover(back));
            c = mix(c, WHITE, cover(rounded_rect(u, v, TOP_SHEET)));
            c = mix(c, mix(accent, BLACK, 0.28), cover(polyline(u, v) - W_HALF_WIDTH));
            let alpha = cover(rounded_rect(u, v, (0.0, 0.0, 1.0, 1.0, CORNER_RADIUS)));
            out.extend(c.map(|ch| (ch + 0.5) as u8));
            out.push((alpha * 255.0 + 0.5) as u8);
        }
    }
    out
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Signed distance to a rounded rectangle, negative inside.
fn rounded_rect(u: f32, v: f32, (x0, y0, x1, y1, r): (f32, f32, f32, f32, f32)) -> f32 {
    let dx = (x0 + r - u).max(u - (x1 - r));
    let dy = (y0 + r - v).max(v - (y1 - r));
    dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - r
}

/// Distance to the centre line of the W.
fn polyline(u: f32, v: f32) -> f32 {
    W.windows(2)
        .map(|seg| {
            let (a, b) = (seg[0], seg[1]);
            let (bx, by) = (b.0 - a.0, b.1 - a.1);
            let t = (((u - a.0) * bx + (v - a.1) * by) / (bx * bx + by * by)).clamp(0.0, 1.0);
            (u - a.0 - t * bx).hypot(v - a.1 - t * by)
        })
        .fold(f32::MAX, f32::min)
}
