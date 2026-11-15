//! Texture cache: lazy requests, LRU eviction bounded by entries and by memory.
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use egui::{ColorImage, Context, TextureFilter, TextureHandle, TextureOptions};

/// A full-size preview alone can take 64 MB, so a count limit is not enough.
const MAX_BYTES: usize = 768 * 1024 * 1024;
const MAX_ENTRIES: usize = 2000;
/// Failed images are requested again after this long: most failures are temporary.
const RETRY_AFTER: Duration = Duration::from_secs(30);

enum Slot {
    Loading,
    Ready(TextureHandle, usize),
    Failed(Instant),
}

pub struct TextureCache {
    map: HashMap<String, (Slot, u64)>,
    frame: u64,
    bytes: usize,
    max_bytes: usize,
    max_entries: usize,
}

impl TextureCache {
    pub fn new() -> Self {
        Self::with_limits(MAX_ENTRIES, MAX_BYTES)
    }

    fn with_limits(max_entries: usize, max_bytes: usize) -> Self {
        Self { map: HashMap::new(), frame: 0, bytes: 0, max_bytes, max_entries }
    }

    pub fn begin_frame(&mut self) {
        self.frame += 1;
        if self.map.len() > self.max_entries || self.bytes > self.max_bytes {
            self.evict();
        }
    }

    /// Drop everything (the thumbnail cache was cleared). Loads in flight still land.
    pub fn clear(&mut self) {
        self.map.clear();
        self.bytes = 0;
    }

    /// Ready texture, or `None` (calling `request` the first time the key is seen,
    /// and again once a failure is old enough to retry).
    pub fn get(&mut self, key: &str, request: impl FnOnce()) -> Option<TextureHandle> {
        let frame = self.frame;
        match self.map.get_mut(key) {
            Some((slot, used)) => {
                *used = frame;
                match slot {
                    Slot::Ready(t, _) => Some(t.clone()),
                    Slot::Failed(at) if at.elapsed() >= RETRY_AFTER => {
                        *slot = Slot::Loading;
                        request();
                        None
                    }
                    _ => None,
                }
            }
            None => {
                self.map.insert(key.to_string(), (Slot::Loading, frame));
                request();
                None
            }
        }
    }

    /// Ready texture without triggering a load.
    pub fn peek(&mut self, key: &str) -> Option<TextureHandle> {
        let frame = self.frame;
        match self.map.get_mut(key) {
            Some((Slot::Ready(t, _), used)) => {
                *used = frame;
                Some(t.clone())
            }
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn is_ready(&self, key: &str) -> bool {
        matches!(self.map.get(key), Some((Slot::Ready(..), _)))
    }

    pub fn is_loading(&self, key: &str) -> bool {
        matches!(self.map.get(key), Some((Slot::Loading, _)))
    }

    pub fn has_failed(&self, key: &str) -> bool {
        matches!(self.map.get(key), Some((Slot::Failed(_), _)))
    }

    pub fn put(&mut self, ctx: &Context, key: String, result: Result<ColorImage, String>) {
        let slot = match result {
            Ok(img) => {
                let [w, h] = img.size;
                // Mipmaps keep large sources crisp when shown small, for a third more memory.
                let bytes = w * h * 4 * 4 / 3;
                let options = TextureOptions::LINEAR.with_mipmap_mode(Some(TextureFilter::Linear));
                self.bytes += bytes;
                Slot::Ready(ctx.load_texture(key.clone(), img, options), bytes)
            }
            Err(e) => {
                tracing::warn!("image {key}: {e}");
                Slot::Failed(Instant::now())
            }
        };
        if let Some((Slot::Ready(_, old), _)) = self.map.insert(key, (slot, self.frame)) {
            self.bytes -= old;
        }
    }

    fn evict(&mut self) {
        let frame = self.frame;
        // Oldest first; never what was drawn in the last frames or is still loading.
        let mut old: Vec<(String, u64)> = self
            .map
            .iter()
            .filter(|(_, (s, used))| !matches!(s, Slot::Loading) && used + 2 < frame)
            .map(|(k, (_, u))| (k.clone(), *u))
            .collect();
        old.sort_by_key(|(_, u)| *u);
        let entries_goal = self.max_entries * 3 / 4;
        let bytes_goal = self.max_bytes * 3 / 4;
        for (k, _) in old {
            if self.map.len() <= entries_goal && self.bytes <= bytes_goal {
                break;
            }
            if let Some((Slot::Ready(_, b), _)) = self.map.remove(&k) {
                self.bytes -= b;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(side: usize) -> Result<ColorImage, String> {
        Ok(ColorImage::filled([side, side], egui::Color32::RED))
    }

    #[test]
    fn evicts_oldest_when_over_the_memory_budget() {
        let ctx = Context::default();
        // Room for about three 100×100 images.
        let mut cache = TextureCache::with_limits(100, 3 * 100 * 100 * 4 * 4 / 3 + 1);
        for name in ["a", "b", "c", "d"] {
            cache.put(&ctx, name.into(), image(100));
            cache.begin_frame();
            cache.begin_frame();
            cache.begin_frame();
        }
        assert!(cache.peek("d").is_some(), "newest survives");
        assert!(cache.peek("a").is_none(), "oldest evicted");
        assert!(cache.bytes <= cache.max_bytes);
    }

    #[test]
    fn recently_used_textures_are_not_evicted() {
        let ctx = Context::default();
        let mut cache = TextureCache::with_limits(100, 1);
        cache.put(&ctx, "on-screen".into(), image(10));
        cache.begin_frame();
        assert!(cache.peek("on-screen").is_some());
    }

    #[test]
    fn replacing_an_image_keeps_the_byte_count_right() {
        let ctx = Context::default();
        let mut cache = TextureCache::new();
        cache.put(&ctx, "a".into(), image(100));
        let one = cache.bytes;
        cache.put(&ctx, "a".into(), image(100));
        assert_eq!(cache.bytes, one);
        cache.clear();
        assert_eq!(cache.bytes, 0);
    }

    #[test]
    fn failures_retry_only_after_a_while() {
        let ctx = Context::default();
        let mut cache = TextureCache::new();
        cache.put(&ctx, "a".into(), Err("boom".into()));
        assert!(cache.has_failed("a"));
        let mut requested = false;
        assert!(cache.get("a", || requested = true).is_none());
        assert!(!requested, "too soon");
        if let Some((slot, _)) = cache.map.get_mut("a") {
            *slot = Slot::Failed(Instant::now() - RETRY_AFTER);
        }
        assert!(cache.get("a", || requested = true).is_none());
        assert!(requested);
        assert!(cache.is_loading("a"));
    }
}
