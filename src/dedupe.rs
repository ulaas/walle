//! Near-duplicate detection with a 64-bit difference hash (dHash) of each thumbnail.
//! Images are reduced to 9×8 grey cells; each bit says whether a cell is brighter than its
//! right neighbour. Re-encodes, resizes and mild colour changes keep the Hamming distance small.

/// Hamming distance at or below which two images count as the same picture.
pub const THRESHOLD: u32 = 8;

pub fn dhash(img: &egui::ColorImage) -> u64 {
    let [w, h] = img.size;
    if w == 0 || h == 0 {
        return 0;
    }
    const GW: usize = 9;
    const GH: usize = 8;
    let mut cells = [[0f32; GW]; GH];
    for (cy, row) in cells.iter_mut().enumerate() {
        let y0 = cy * h / GH;
        let y1 = ((cy + 1) * h / GH).max(y0 + 1).min(h);
        for (cx, cell) in row.iter_mut().enumerate() {
            let x0 = cx * w / GW;
            let x1 = ((cx + 1) * w / GW).max(x0 + 1).min(w);
            // Average luma over the cell (sampled on a small grid for speed).
            let (sx, sy) = (((x1 - x0) / 6).max(1), ((y1 - y0) / 6).max(1));
            let (mut sum, mut n) = (0f32, 0f32);
            let mut y = y0;
            while y < y1 {
                let mut x = x0;
                while x < x1 {
                    let p = img.pixels[y * w + x];
                    sum += 0.299 * p.r() as f32 + 0.587 * p.g() as f32 + 0.114 * p.b() as f32;
                    n += 1.0;
                    x += sx;
                }
                y += sy;
            }
            *cell = sum / n.max(1.0);
        }
    }
    let mut hash = 0u64;
    for row in &cells {
        for x in 0..GW - 1 {
            hash = (hash << 1) | (row[x] > row[x + 1]) as u64;
        }
    }
    hash
}

pub fn distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Group keys whose hashes are within `THRESHOLD` of each other (union-find).
/// Only groups with at least two members are returned, largest first.
pub fn groups(items: &[(String, u64)]) -> Vec<Vec<String>> {
    let n = items.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    for i in 0..n {
        for j in i + 1..n {
            // Hash 0 means "flat/unknown"; never group on it.
            if items[i].1 != 0 && items[j].1 != 0 && distance(items[i].1, items[j].1) <= THRESHOLD {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                if a != b {
                    parent[a] = b;
                }
            }
        }
    }
    let mut map: std::collections::HashMap<usize, Vec<String>> = Default::default();
    for i in 0..n {
        let r = find(&mut parent, i);
        map.entry(r).or_default().push(items[i].0.clone());
    }
    let mut out: Vec<Vec<String>> = map.into_values().filter(|g| g.len() > 1).collect();
    out.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_color(img: &image::RgbImage) -> egui::ColorImage {
        let rgba = image::DynamicImage::ImageRgb8(img.clone()).to_rgba8();
        egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw())
    }

    fn scene(w: u32, h: u32, shift: f32) -> image::RgbImage {
        image::RgbImage::from_fn(w, h, |x, y| {
            let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
            let r = (255.0 * (u * 3.0 + shift).sin().abs()) as u8;
            let g = (255.0 * v) as u8;
            let b = (255.0 * ((u - 0.5).powi(2) + (v - 0.5).powi(2)).sqrt()) as u8;
            image::Rgb([r, g, b])
        })
    }

    #[test]
    fn resized_copy_matches_different_image_does_not() {
        let a = scene(640, 400, 0.0);
        let small = image::imageops::resize(&a, 200, 125, image::imageops::FilterType::Triangle);
        let other = scene(640, 400, 1.7);
        let (ha, hs, ho) = (dhash(&to_color(&a)), dhash(&to_color(&small)), dhash(&to_color(&other)));
        assert!(distance(ha, hs) <= THRESHOLD, "resized copy distance {}", distance(ha, hs));
        assert!(distance(ha, ho) > THRESHOLD, "different image distance {}", distance(ha, ho));
    }

    #[test]
    fn grouping() {
        let items = vec![("a".to_string(), 0b1111u64 << 8), ("b".into(), 0b1110 << 8), ("c".into(), u64::MAX), ("d".into(), 0)];
        let g = groups(&items);
        assert_eq!(g, vec![vec!["a".to_string(), "b".to_string()]]);
    }
}
