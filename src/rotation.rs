//! Automatic wallpaper rotation: candidate selection (pure) + scheduling state.
use std::{
    collections::VecDeque,
    time::{Duration, Instant, SystemTime},
};

use crate::{
    config::{RotationConfig, RotationSource},
    library::{LibraryFilter, OrientationFilter},
    model::{Status, Wallpaper},
};

/// How many recently shown wallpapers to avoid repeating.
const RECENT: usize = 30;

#[derive(Default)]
pub struct State {
    pub next_at: Option<Instant>,
    recent: VecDeque<String>,
    seed: u64,
}

impl State {
    pub fn schedule(&mut self, cfg: &RotationConfig, from: Instant) {
        self.next_at = cfg.enabled.then(|| from + interval(cfg));
    }

    pub fn due(&self, now: Instant) -> bool {
        self.next_at.is_some_and(|t| now >= t)
    }

    /// Wall-clock time of the next change, for display.
    pub fn next_wallclock(&self) -> Option<SystemTime> {
        let t = self.next_at?;
        Some(SystemTime::now() + t.saturating_duration_since(Instant::now()))
    }

    pub fn remember(&mut self, key: String) {
        self.recent.retain(|k| *k != key);
        self.recent.push_back(key);
        while self.recent.len() > RECENT {
            self.recent.pop_front();
        }
    }

    fn next_random(&mut self) -> u64 {
        if self.seed == 0 {
            self.seed = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1) | 1;
        }
        // xorshift64*
        let mut x = self.seed;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.seed = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    /// Pick one candidate, avoiding recently shown ones (unless nothing else is left)
    /// and the wallpaper that is currently set.
    pub fn pick(&mut self, candidates: &[Wallpaper], cfg: &RotationConfig, current: Option<&str>) -> Option<Wallpaper> {
        let ok = |w: &&Wallpaper| !cfg.landscape_only || w.width == 0 || w.width >= w.height;
        let usable: Vec<&Wallpaper> = candidates.iter().filter(ok).collect();
        let fresh: Vec<&Wallpaper> = usable
            .iter()
            .copied()
            .filter(|w| !self.recent.contains(&w.key()) && Some(w.key().as_str()) != current)
            .collect();
        let pool = if fresh.is_empty() {
            usable.iter().copied().filter(|w| Some(w.key().as_str()) != current).collect::<Vec<_>>()
        } else {
            fresh
        };
        let pool = if pool.is_empty() { usable } else { pool };
        if pool.is_empty() {
            return None;
        }
        let i = (self.next_random() % pool.len() as u64) as usize;
        Some(pool[i].clone())
    }
}

pub fn interval(cfg: &RotationConfig) -> Duration {
    Duration::from_secs(cfg.interval_minutes.max(1) as u64 * 60)
}

/// Library filter for library-backed sources (`None` for search-backed ones).
pub fn library_filter(cfg: &RotationConfig) -> Option<LibraryFilter> {
    let base = LibraryFilter {
        min_rating: cfg.min_rating,
        orientation: if cfg.landscape_only { OrientationFilter::Landscape } else { OrientationFilter::Any },
        ..Default::default()
    };
    match &cfg.source {
        RotationSource::Kept => Some(LibraryFilter { status: Some(Status::Kept), ..base }),
        RotationSource::Considering => Some(LibraryFilter { status: Some(Status::Considering), ..base }),
        RotationSource::Tag(t) => Some(LibraryFilter { tag: Some(t.clone()), ..base }),
        RotationSource::SavedSearch(_) | RotationSource::Provider(_) => None,
    }
}

pub fn describe(src: &RotationSource) -> String {
    match src {
        RotationSource::Kept => "Kept wallpapers".into(),
        RotationSource::Considering => "Shortlist".into(),
        RotationSource::Tag(t) => format!("Tag \"{t}\""),
        RotationSource::SavedSearch(_) => "Saved search".into(),
        RotationSource::Provider(p) => format!("Source {p}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wp(id: &str, w: u32, h: u32) -> Wallpaper {
        Wallpaper { provider: "t".into(), remote_id: id.into(), width: w, height: h, ..Default::default() }
    }

    #[test]
    fn avoids_repeats_and_current() {
        let cfg = RotationConfig { landscape_only: true, ..Default::default() };
        let items = vec![wp("a", 1920, 1080), wp("b", 1920, 1080), wp("c", 1080, 1920), wp("d", 0, 0)];
        let mut st = State::default();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..3 {
            let p = st.pick(&items, &cfg, None).unwrap();
            assert_ne!(p.remote_id, "c", "portrait excluded");
            st.remember(p.key());
            seen.insert(p.remote_id);
        }
        assert_eq!(seen.len(), 3, "no repeats until the pool is exhausted: {seen:?}");
        // Pool exhausted: still picks something, but never the current one.
        for _ in 0..20 {
            let p = st.pick(&items, &cfg, Some("t:a")).unwrap();
            assert_ne!(p.remote_id, "a");
        }
        // Only one candidate and it is current: still returned (better than nothing).
        assert_eq!(st.pick(&items[..1], &cfg, Some("t:a")).unwrap().remote_id, "a");
        assert!(st.pick(&[], &cfg, None).is_none());
    }

    #[test]
    fn scheduling() {
        let mut st = State::default();
        let now = Instant::now();
        let mut cfg = RotationConfig { interval_minutes: 30, ..Default::default() };
        st.schedule(&cfg, now);
        assert!(st.next_at.is_none(), "disabled");
        cfg.enabled = true;
        st.schedule(&cfg, now);
        assert!(!st.due(now));
        assert!(st.due(now + Duration::from_secs(30 * 60)));
        assert!(library_filter(&RotationConfig { source: RotationSource::SavedSearch(1), ..cfg.clone() }).is_none());
        assert_eq!(library_filter(&cfg).unwrap().status, Some(Status::Kept));
    }
}
