//! https://www.pexels.com/api/documentation — free key, 200 req/hour. Attribution expected.
use std::time::Duration;

use anyhow::{Result, bail};
use async_trait::async_trait;
use serde::Deserialize;

use super::{Provider, ProviderCaps, ProviderInfo};
use crate::{
    model::{Orientation, SearchPage, SearchQuery, Wallpaper},
    net::{RateLimiter, get_json_with},
};

const PER_PAGE: u32 = 40;

pub struct Pexels {
    client: reqwest::Client,
    key: Option<String>,
    limiter: RateLimiter,
}

impl Pexels {
    pub fn new(client: reqwest::Client, key: Option<String>) -> Self {
        Self { client, key, limiter: RateLimiter::new(20, Duration::from_secs(60)) }
    }
}

#[derive(Deserialize)]
struct Resp {
    #[serde(default)]
    photos: Vec<Photo>,
    total_results: Option<u64>,
    next_page: Option<String>,
}

#[derive(Deserialize)]
struct Photo {
    id: u64,
    width: u32,
    height: u32,
    url: String,
    photographer: String,
    photographer_url: String,
    avg_color: Option<String>,
    alt: Option<String>,
    src: Src,
}

#[derive(Deserialize)]
struct Src {
    original: String,
    large: String,
    large2x: String,
}

impl From<Photo> for Wallpaper {
    fn from(p: Photo) -> Self {
        Wallpaper {
            provider: "pexels".into(),
            remote_id: p.id.to_string(),
            page_url: Some(p.url),
            full_url: p.src.original,
            thumb_url: p.src.large,
            preview_url: Some(p.src.large2x),
            width: p.width,
            height: p.height,
            title: p.alt.filter(|a| !a.is_empty()),
            author: Some(p.photographer),
            author_url: Some(p.photographer_url),
            license: Some("Pexels License".into()),
            colors: p.avg_color.into_iter().collect(),
            ..Default::default()
        }
    }
}

#[async_trait]
impl Provider for Pexels {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: "pexels",
            name: "Pexels",
            homepage: "https://www.pexels.com",
            key_required: true,
            key_optional: false,
            order: 20,
            key_help: "Free API key: pexels.com/api > Your API Key.",
            has_key: self.key.is_some(),
            caps: ProviderCaps { text_search: true, orientation: true, colors: true, ..Default::default() },
        }
    }

    async fn search(&self, q: &SearchQuery, page: u32) -> Result<SearchPage> {
        let Some(key) = &self.key else { bail!("Pexels needs an API key (Settings > Sources)") };
        self.limiter.acquire().await;
        let text = q.text.trim();
        let url = if text.is_empty() && q.color.is_none() {
            format!("https://api.pexels.com/v1/curated?page={page}&per_page={PER_PAGE}")
        } else {
            let mut url = format!(
                "https://api.pexels.com/v1/search?query={}&page={page}&per_page={PER_PAGE}&size=large",
                urlencoding::encode(if text.is_empty() { "wallpaper" } else { text })
            );
            match q.orientation {
                Orientation::Landscape => url += "&orientation=landscape",
                Orientation::Portrait => url += "&orientation=portrait",
                Orientation::Any => {}
            }
            if let Some(c) = &q.color {
                url += &format!("&color={}", urlencoding::encode(&format!("#{}", c.trim_start_matches('#'))));
            }
            url
        };
        let resp: Resp = get_json_with(&self.client, &url, &[("Authorization", key.as_str())]).await?;
        let more = resp.next_page.is_some();
        Ok(SearchPage {
            items: resp.photos.into_iter().map(Into::into).collect(),
            page,
            last_page: if more { None } else { Some(page) },
            total: resp.total_results,
            seed: None,
        })
    }
}
