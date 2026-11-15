//! Visual effects for the wallpaper on the desktop. walle renders them into a copy of the
//! picture and hands that copy to the desktop, so the original is never changed and the last
//! rendered picture stays on the desktop when walle is not running.
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use image::{DynamicImage, RgbImage, imageops};
use serde::{Deserialize, Serialize};

use crate::{
    desktop::Display,
    model::FitMode,
    net::{fnv64, sanitize},
};

/// Steps between day and night. Each step is one re-render, so few of them.
pub const NIGHT_STEPS: u8 = 4;
/// Minutes over which night comes in after `from_hour`, and goes out before `to_hour`.
const NIGHT_RAMP_MINUTES: i32 = 60;

/// Dimmer and warmer during the night. This one needs walle running: it re-renders the
/// wallpaper as the evening comes in and the morning goes out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Night {
    pub enabled: bool,
    /// Hour (0-23) at which it starts to come in.
    pub from_hour: u8,
    /// Hour (0-23) by which it is gone again.
    pub to_hour: u8,
    /// Percent darker at full night.
    pub dim: f32,
    /// Percent warmer (less blue) at full night.
    pub warmth: f32,
}

impl Default for Night {
    fn default() -> Self {
        Self { enabled: false, from_hour: 20, to_hour: 7, dim: 30.0, warmth: 40.0 }
    }
}

impl Night {
    /// How much night it is at this time of day, in steps from 0 (day) to `NIGHT_STEPS`.
    pub fn level(&self, hour: u32, minute: u32) -> u8 {
        if !self.enabled {
            return 0;
        }
        let day = 24 * 60;
        let now = (hour as i32 * 60 + minute as i32).rem_euclid(day);
        let from = (self.from_hour.min(23) as i32) * 60;
        let to = (self.to_hour.min(23) as i32) * 60;
        // Minutes since night began, and how long it lasts (through midnight if need be).
        let since = (now - from).rem_euclid(day);
        let length = match (to - from).rem_euclid(day) {
            0 => day,
            l => l,
        };
        if since >= length {
            return 0;
        }
        let ramp = NIGHT_RAMP_MINUTES.min(length / 2).max(1);
        let amount = (since as f32 / ramp as f32).min((length - since) as f32 / ramp as f32).min(1.0);
        // Never 0 during the night: its first minutes are the first step already.
        ((amount * NIGHT_STEPS as f32).ceil() as u8).clamp(1, NIGHT_STEPS)
    }

    pub fn level_now(&self) -> u8 {
        use chrono::Timelike;
        let now = chrono::Local::now();
        self.level(now.hour(), now.minute())
    }
}

/// The effects, as the settings hold them. All amounts are percent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Effects {
    /// 0: sharp. 100: only colours are left.
    pub blur: f32,
    /// 0: as it is. 100: black.
    pub dim: f32,
    /// 0: grey. 100: as it is. 200: twice as colourful.
    pub saturation: f32,
    /// Colour the picture is tinted with.
    pub tint: [u8; 3],
    /// 0: no tint. 100: only the tint colour's hue is left.
    pub tint_strength: f32,
    /// 0: none. 100: the corners are black.
    pub vignette: f32,
    pub night: Night,
}

impl Default for Effects {
    fn default() -> Self {
        Self {
            blur: 0.0,
            dim: 0.0,
            saturation: 100.0,
            tint: [64, 110, 200],
            tint_strength: 0.0,
            vignette: 0.0,
            night: Night::default(),
        }
    }
}

/// The effects at one moment: the settings with the night level worked in.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    /// Blur radius as a fraction of the picture's shorter side.
    blur: f32,
    /// What is left of the brightness (1: all of it).
    light: f32,
    saturation: f32,
    tint: [f32; 3],
    tint_strength: f32,
    /// Per channel, for warmth.
    balance: [f32; 3],
    vignette: f32,
}

impl Effects {
    /// `night_level` is from `Night::level`.
    pub fn look(&self, night_level: u8) -> Look {
        let pct = |v: f32| if v.is_finite() { (v / 100.0).clamp(0.0, 1.0) } else { 0.0 };
        let night = if self.night.enabled { night_level.min(NIGHT_STEPS) as f32 / NIGHT_STEPS as f32 } else { 0.0 };
        let warm = pct(self.night.warmth) * night;
        Look {
            blur: pct(self.blur) * 0.035,
            light: (1.0 - pct(self.dim)) * (1.0 - pct(self.night.dim) * night),
            saturation: if self.saturation.is_finite() { (self.saturation / 100.0).clamp(0.0, 2.0) } else { 1.0 },
            tint: self.tint.map(|c| c as f32 / 255.0),
            tint_strength: pct(self.tint_strength),
            balance: [1.0, 1.0 - 0.18 * warm, 1.0 - 0.5 * warm],
            vignette: pct(self.vignette),
        }
    }
}

impl Look {
    /// Nothing to render: the original can go on the desktop as it is.
    pub fn is_plain(&self) -> bool {
        self.blur <= 0.0
            && self.light >= 1.0
            && (self.saturation - 1.0).abs() < 0.005
            && self.tint_strength <= 0.0
            && self.balance == [1.0; 3]
            && self.vignette <= 0.0
    }

    /// Part of the rendered file's name: a different look is a different file.
    fn signature(&self) -> String {
        format!("{self:?}")
    }

    pub fn apply(&self, img: RgbImage) -> RgbImage {
        let (w, h) = img.dimensions();
        let mut img = if self.blur > 0.0 {
            let sigma = self.blur * w.min(h) as f32;
            if sigma >= 0.5 { imageops::fast_blur(&img, sigma) } else { img }
        } else {
            img
        };
        let colour_work = (self.saturation - 1.0).abs() >= 0.005 || self.tint_strength > 0.0;
        let tint_luma = (0.299 * self.tint[0] + 0.587 * self.tint[1] + 0.114 * self.tint[2]).max(0.25);
        // Vignette: the darkening per squared distance from the centre, looked up.
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        let corner = (cx * cx + cy * cy).max(1.0);
        let shade: Vec<f32> = (0..=256)
            .map(|i| {
                let d = (i as f32 / 256.0).sqrt();
                let t = ((d - 0.35) / 0.65).clamp(0.0, 1.0);
                1.0 - self.vignette * t * t * (3.0 - 2.0 * t)
            })
            .collect();
        for (y, row) in img.enumerate_rows_mut() {
            let dy = y as f32 + 0.5 - cy;
            for (x, _, px) in row {
                let mut c = [px[0] as f32, px[1] as f32, px[2] as f32];
                if colour_work {
                    let luma = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
                    for (i, v) in c.iter_mut().enumerate() {
                        *v = luma + (*v - luma) * self.saturation;
                        let tinted = luma * self.tint[i] / tint_luma;
                        *v += (tinted - *v) * self.tint_strength;
                    }
                }
                let mut light = self.light;
                if self.vignette > 0.0 {
                    let dx = x as f32 + 0.5 - cx;
                    let d2 = (dx * dx + dy * dy) / corner;
                    light *= shade[(d2 * 256.0) as usize];
                }
                // A little noise before rounding: without it a dimmed sky or a vignette shows
                // its 256 levels as bands. Whole values come out as they went in.
                let h = (x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA6B)).wrapping_mul(0xC2B2_AE35);
                let noise = (h >> 8) as f32 / (1u32 << 24) as f32 * 0.98 + 0.01;
                for (i, v) in c.iter().enumerate() {
                    px[i] = (v * light * self.balance[i] + noise).clamp(0.0, 255.0) as u8;
                }
            }
        }
        img
    }
}

/// One picture to put on the desktop: the original, or a rendered copy of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Rendered {
    pub path: PathBuf,
    /// How the desktop should place it (a rendered slice of a spanned picture fills).
    pub fit: FitMode,
}

/// Where rendered pictures are kept.
pub fn render_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("desktop")
}

/// The picture for one display (or, with `display` unknown, for a desktop that places it
/// itself). Returns the original when there is nothing to render.
pub fn render_one(source: &Path, look: &Look, fit: FitMode, display: Option<&Display>, dir: &Path) -> Result<Rendered> {
    // One display cannot span; it fills.
    let fit = if fit == FitMode::Span && display.is_some() { FitMode::Fill } else { fit };
    if look.is_plain() {
        return Ok(Rendered { path: source.to_path_buf(), fit });
    }
    let size = display.map(Display::pixels);
    let name = display.map_or("all", |d| d.name.as_str());
    let out = out_path(dir, name, source, &format!("{}|{fit:?}|{size:?}", look.signature()));
    if !is_file(&out) {
        let img = open(source)?;
        // No more pixels than the display shows: effects on a 6K original are wasted time.
        let img = match (size, fit) {
            (Some((w, h)), FitMode::Fill | FitMode::Span) => shrink_to_cover(img, w, h),
            (Some((w, h)), FitMode::Fit) => {
                if img.width() > w || img.height() > h { img.resize(w, h, imageops::FilterType::Triangle) } else { img }
            }
            (Some((w, h)), FitMode::Stretch) => {
                if img.width() > w && img.height() > h { img.resize_exact(w, h, imageops::FilterType::Triangle) } else { img }
            }
            _ => img,
        };
        write(&look.apply(img.into_rgb8()), &out)?;
    }
    forget_others(dir, name, &[&out]);
    Ok(Rendered { path: out, fit })
}

/// One picture across all displays: each display gets the part of it that lies behind it.
pub fn render_span(source: &Path, look: &Look, displays: &[Display], dir: &Path) -> Result<Vec<(String, Rendered)>> {
    let left = displays.iter().map(|d| d.x).min().context("no displays")?;
    let top = displays.iter().map(|d| d.y).min().unwrap_or(0);
    let right = displays.iter().map(|d| d.x + d.width as i32).max().unwrap_or(0);
    let bottom = displays.iter().map(|d| d.y + d.height as i32).max().unwrap_or(0);
    let (total_w, total_h) = ((right - left).max(1) as f32, (bottom - top).max(1) as f32);
    let layout: String = displays.iter().map(|d| format!("{}:{},{},{}x{}@{};", d.name, d.x, d.y, d.width, d.height, d.scale)).collect();
    let outs: Vec<PathBuf> = displays
        .iter()
        .map(|d| out_path(dir, &d.name, source, &format!("span|{}|{layout}", look.signature())))
        .collect();
    if !outs.iter().all(|o| is_file(o)) {
        let img = open(source)?;
        // The picture covers the box around all displays, centred, as a single display's would.
        let (iw, ih) = (img.width() as f32, img.height() as f32);
        let scale = (total_w / iw).max(total_h / ih);
        let (off_x, off_y) = ((iw * scale - total_w) / 2.0, (ih * scale - total_h) / 2.0);
        for (d, out) in displays.iter().zip(&outs) {
            let x = ((d.x - left) as f32 + off_x) / scale;
            let y = ((d.y - top) as f32 + off_y) / scale;
            let (w, h) = (d.width as f32 / scale, d.height as f32 / scale);
            let (x, y) = (x.max(0.0).min(iw - 1.0), y.max(0.0).min(ih - 1.0));
            let (w, h) = (w.min(iw - x).max(1.0), h.min(ih - y).max(1.0));
            let part = img.crop_imm(x.round() as u32, y.round() as u32, w.round().max(1.0) as u32, h.round().max(1.0) as u32);
            let (pw, ph) = d.pixels();
            let part = if part.width() > pw || part.height() > ph { shrink_to_cover(part, pw, ph) } else { part };
            write(&look.apply(part.into_rgb8()), out)?;
        }
    }
    for (d, out) in displays.iter().zip(&outs) {
        forget_others(dir, &d.name, &[out]);
    }
    Ok(displays
        .iter()
        .zip(outs)
        .map(|(d, path)| (d.name.clone(), Rendered { path, fit: FitMode::Fill }))
        .collect())
}

fn open(source: &Path) -> Result<DynamicImage> {
    let bytes = std::fs::read(source).with_context(|| format!("reading {}", source.display()))?;
    image::load_from_memory(&bytes).with_context(|| format!("decoding {}", source.display()))
}

/// Smaller, but still covering `w`x`h`. Never larger.
fn shrink_to_cover(img: DynamicImage, w: u32, h: u32) -> DynamicImage {
    let scale = (w as f32 / img.width() as f32).max(h as f32 / img.height() as f32);
    if scale >= 1.0 {
        return img;
    }
    let (nw, nh) = ((img.width() as f32 * scale).ceil().max(1.0) as u32, (img.height() as f32 * scale).ceil().max(1.0) as u32);
    img.resize_exact(nw, nh, imageops::FilterType::Triangle)
}

fn is_file(p: &Path) -> bool {
    p.metadata().map(|m| m.is_file() && m.len() > 0).unwrap_or(false)
}

/// `<display>-<hash of what it was made from>.jpg`.
fn out_path(dir: &Path, name: &str, source: &Path, recipe: &str) -> PathBuf {
    let modified = source
        .metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    let hash = fnv64(&format!("{}|{modified}|{recipe}", source.display()));
    dir.join(format!("{}-{hash:016x}.jpg", sanitize(name)))
}

fn write(img: &RgbImage, out: &Path) -> Result<()> {
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Written beside and renamed: the desktop must never read half a file.
    let tmp = out.with_extension("part");
    let mut file = std::io::BufWriter::new(std::fs::File::create(&tmp).with_context(|| format!("writing {}", tmp.display()))?);
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut file, 95);
    img.write_with_encoder(enc)?;
    std::io::Write::flush(&mut file)?;
    drop(file);
    std::fs::rename(&tmp, out).with_context(|| format!("writing {}", out.display()))
}

/// Remove this display's older rendered pictures.
fn forget_others(dir: &Path, name: &str, keep: &[&PathBuf]) {
    let prefix = format!("{}-", sanitize(name));
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let ours = p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
            // `<name>-<16 hex>.jpg` exactly: "DP-1-…" must not match the files of "DP".
            n.strip_prefix(&prefix).and_then(|rest| rest.strip_suffix(".jpg")).is_some_and(|hash| hash.len() == 16 && !hash.contains('-'))
        });
        if ours && !keep.contains(&&p) {
            let _ = std::fs::remove_file(&p);
        }
    }
}

/// Remove every rendered picture that is not in `keep` (after the look went back to plain).
pub fn forget_all_but(dir: &Path, keep: &[PathBuf]) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "jpg" || x == "part") && !keep.contains(&p) {
            let _ = std::fs::remove_file(&p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("walle-effects-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn display(name: &str, x: i32, w: u32, h: u32) -> Display {
        Display { name: name.into(), x, y: 0, width: w, height: h, scale: 1.0 }
    }

    #[test]
    fn night_comes_in_and_goes_out() {
        let n = Night { enabled: true, from_hour: 20, to_hour: 7, ..Default::default() };
        assert_eq!(n.level(12, 0), 0);
        assert_eq!(n.level(19, 59), 0);
        assert_eq!(n.level(20, 0), 1, "the first step at once");
        assert_eq!(n.level(20, 31), 3);
        assert_eq!(n.level(21, 0), NIGHT_STEPS);
        assert_eq!(n.level(2, 0), NIGHT_STEPS, "through midnight");
        assert_eq!(n.level(6, 50), 1);
        assert_eq!(n.level(7, 0), 0);
        // Daytime hours work too, and a zero-length night is the whole day.
        let nap = Night { enabled: true, from_hour: 13, to_hour: 14, ..Default::default() };
        assert_eq!(nap.level(13, 30), NIGHT_STEPS);
        assert_eq!(nap.level(14, 0), 0);
        let always = Night { enabled: true, from_hour: 5, to_hour: 5, ..Default::default() };
        assert_eq!(always.level(17, 0), NIGHT_STEPS);
        assert_eq!(Night::default().level(23, 0), 0, "off by default");
    }

    #[test]
    fn plain_look_leaves_the_original() {
        let dir = tmp("plain");
        let src = dir.join("src.png");
        RgbImage::from_pixel(8, 8, image::Rgb([10, 20, 30])).save(&src).unwrap();
        let look = Effects::default().look(0);
        assert!(look.is_plain());
        // Night settings do nothing during the day, or when switched off.
        let night = Effects { night: Night { enabled: true, ..Default::default() }, ..Default::default() };
        assert!(night.look(0).is_plain());
        assert!(!night.look(NIGHT_STEPS).is_plain());
        let r = render_one(&src, &look, FitMode::Span, Some(&display("DP-1", 0, 4, 4)), &dir.join("out")).unwrap();
        assert_eq!(r, Rendered { path: src, fit: FitMode::Fill });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn effects_change_the_pixels() {
        let img = RgbImage::from_pixel(64, 64, image::Rgb([200, 100, 50]));
        let dim = Effects { dim: 50.0, ..Default::default() }.look(0).apply(img.clone());
        assert_eq!(dim.get_pixel(32, 32).0, [100, 50, 25]);
        let grey = Effects { saturation: 0.0, ..Default::default() }.look(0).apply(img.clone());
        let g = grey.get_pixel(1, 1).0;
        assert!(g[0].abs_diff(g[1]) <= 1 && g[1].abs_diff(g[2]) <= 1, "{g:?}");
        let vig = Effects { vignette: 100.0, ..Default::default() }.look(0).apply(img.clone());
        assert_eq!(vig.get_pixel(32, 32).0, [200, 100, 50], "the middle is untouched");
        assert!(vig.get_pixel(0, 0).0[0] < 20, "the corner is dark");
        let warm = Effects { night: Night { enabled: true, dim: 0.0, warmth: 100.0, ..Default::default() }, ..Default::default() };
        let w = warm.look(NIGHT_STEPS).apply(img.clone()).get_pixel(5, 5).0;
        assert_eq!(w[0], 200);
        assert!(w[2] < 30 && w[1] < 100, "{w:?}");
        let tint = Effects { tint: [0, 0, 255], tint_strength: 100.0, ..Default::default() }.look(0).apply(img.clone());
        let t = tint.get_pixel(5, 5).0;
        assert!(t[2] > t[0] && t[2] > t[1], "{t:?}");
        let blur = Effects { blur: 50.0, ..Default::default() }.look(0);
        let mut sharp = RgbImage::from_pixel(64, 64, image::Rgb([0, 0, 0]));
        sharp.put_pixel(32, 32, image::Rgb([255, 255, 255]));
        let b = blur.apply(sharp);
        assert!(b.get_pixel(32, 32).0[0] < 255 && b.get_pixel(33, 32).0[0] > 0);
    }

    #[test]
    fn renders_once_and_forgets_the_old() {
        let dir = tmp("render");
        let src = dir.join("src.png");
        RgbImage::from_pixel(400, 200, image::Rgb([200, 100, 50])).save(&src).unwrap();
        let out = dir.join("out");
        let d = display("DP-1", 0, 100, 100);
        let dim = Effects { dim: 50.0, ..Default::default() }.look(0);
        let a = render_one(&src, &dim, FitMode::Fill, Some(&d), &out).unwrap();
        assert_ne!(a.path, src);
        let made = image::open(&a.path).unwrap();
        assert_eq!((made.width(), made.height()), (200, 100), "shrunk to cover the display, not cropped");
        let again = render_one(&src, &dim, FitMode::Fill, Some(&d), &out).unwrap();
        assert_eq!(a, again);
        // Another display's files stay; this display's older one goes.
        let other = render_one(&src, &dim, FitMode::Fill, Some(&display("DP-1-1", 0, 100, 100)), &out).unwrap();
        let darker = Effects { dim: 80.0, ..Default::default() }.look(0);
        let b = render_one(&src, &darker, FitMode::Fill, Some(&d), &out).unwrap();
        assert!(!a.path.exists() && b.path.exists() && other.path.exists());
        forget_all_but(&out, &[]);
        assert!(!b.path.exists() && !other.path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn span_gives_each_display_its_part() {
        let dir = tmp("span");
        let src = dir.join("src.png");
        // Left half red, right half blue.
        let mut img = RgbImage::from_pixel(400, 100, image::Rgb([255, 0, 0]));
        for y in 0..100 {
            for x in 200..400 {
                img.put_pixel(x, y, image::Rgb([0, 0, 255]));
            }
        }
        img.save(&src).unwrap();
        let displays = [display("L", 0, 100, 50), display("R", 100, 100, 50)];
        let parts = render_span(&src, &Effects::default().look(0), &displays, &dir.join("out")).unwrap();
        assert_eq!(parts.len(), 2);
        let px = |i: usize| image::open(&parts[i].1.path).unwrap().into_rgb8().get_pixel(50, 25).0;
        let (l, r) = (px(0), px(1));
        assert!(l[0] > 200 && l[2] < 60, "left is red: {l:?}");
        assert!(r[2] > 200 && r[0] < 60, "right is blue: {r:?}");
        assert_eq!(parts[0].1.fit, FitMode::Fill);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
