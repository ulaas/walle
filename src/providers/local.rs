//! Browse image files in local folders (configured in Settings, or dropped onto the window).
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant, SystemTime},
};

use anyhow::Result;
use async_trait::async_trait;

use super::{Provider, ProviderCaps, ProviderInfo};
use crate::{
    model::{SearchPage, SearchQuery, Sorting, Wallpaper},
    net::fnv64,
};

const PER_PAGE: usize = 60;
const EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "gif"];

pub struct LocalFolders {
    folders: Vec<PathBuf>,
    /// (scan time, files sorted newest first)
    cache: Mutex<Option<(Instant, Vec<(PathBuf, SystemTime)>)>>,
}

impl LocalFolders {
    pub fn new(folders: Vec<PathBuf>) -> Self {
        Self { folders, cache: Mutex::new(None) }
    }

    fn scan(&self) -> Vec<(PathBuf, SystemTime)> {
        let mut out = vec![];
        for f in &self.folders {
            walk(f, 0, &mut out);
        }
        out.sort_by(|a, b| b.1.cmp(&a.1));
        out.dedup_by(|a, b| a.0 == b.0);
        out
    }
}

pub fn is_image(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<(PathBuf, SystemTime)>) {
    if depth > 8 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(meta) = e.metadata() else { continue };
        if meta.is_dir() {
            walk(&p, depth + 1, out);
        } else if is_image(&p) {
            out.push((p, meta.modified().unwrap_or(SystemTime::UNIX_EPOCH)));
        }
    }
}

/// Build a `Wallpaper` for a local image file.
pub fn wallpaper_for(path: &Path) -> Wallpaper {
    let s = path.to_string_lossy().to_string();
    let (width, height) = image::image_dimensions(path).unwrap_or((0, 0));
    let file_size = std::fs::metadata(path).ok().map(|m| m.len());
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("jpg").to_ascii_lowercase();
    Wallpaper {
        provider: "local".into(),
        remote_id: format!("{:016x}", fnv64(&s)),
        page_url: None,
        full_url: s.clone(),
        thumb_url: s.clone(),
        preview_url: None,
        width,
        height,
        file_size,
        file_type: Some(format!("image/{}", if ext == "jpg" { "jpeg" } else { &ext })),
        title: path.file_stem().map(|f| f.to_string_lossy().to_string()),
        source: path.parent().map(|p| p.to_string_lossy().to_string()),
        ..Default::default()
    }
}

#[async_trait]
impl Provider for LocalFolders {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: "local",
            name: "Local folders",
            homepage: "",
            key_required: false,
            key_optional: false,
            order: 90,
            key_help: "Add folders in Settings, or drop a folder onto the window.",
            has_key: false,
            caps: ProviderCaps {
                text_search: true,
                sortings: vec![Sorting::DateAdded, Sorting::Random],
                ..Default::default()
            },
        }
    }

    async fn search(&self, q: &SearchQuery, page: u32) -> Result<SearchPage> {
        let page = page.max(1);
        let text = q.text.trim().to_lowercase();
        let sorting = q.sorting;
        let seed = q.seed.clone();
        let files = {
            let mut cache = self.cache.lock().unwrap();
            let stale = cache.as_ref().is_none_or(|(t, _)| page == 1 || t.elapsed() > Duration::from_secs(300));
            if stale {
                *cache = Some((Instant::now(), self.scan()));
            }
            cache.as_ref().unwrap().1.clone()
        };
        let items = tokio::task::spawn_blocking(move || {
            let mut matching: Vec<PathBuf> = files
                .into_iter()
                .map(|(p, _)| p)
                .filter(|p| text.is_empty() || p.to_string_lossy().to_lowercase().contains(&text))
                .collect();
            if sorting == Sorting::Random {
                // Deterministic shuffle so paging stays stable within a session.
                let s = fnv64(seed.as_deref().unwrap_or("walle"));
                matching.sort_by_key(|p| fnv64(&p.to_string_lossy()) ^ s);
            }
            let total = matching.len();
            let items: Vec<Wallpaper> = matching
                .iter()
                .skip((page as usize - 1) * PER_PAGE)
                .take(PER_PAGE)
                .map(|p| wallpaper_for(p))
                .collect();
            (items, total)
        })
        .await?;
        let (items, total) = items;
        let last = total.div_ceil(PER_PAGE).max(1) as u32;
        Ok(SearchPage {
            items,
            page,
            last_page: Some(last),
            total: Some(total as u64),
            seed: Some(q.seed.clone().unwrap_or_else(|| format!("{}", SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_nanos()))),
        })
    }
}
