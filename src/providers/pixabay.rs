//! https://pixabay.com/api/docs/ — free key, 100 req/min.
//! Note: without "full API access" the largest URL is `largeImageURL` (max 1280 px);
//! approved keys also get `fullHDURL` / `imageURL`, which are preferred when present.
use std::time::Duration;

use anyhow::{Result, bail};
use async_trait::async_trait;
use serde::Deserialize;

use super::{Provider, ProviderCaps, ProviderInfo};
use crate::{
    model::{Orientation, SearchPage, SearchQuery, Sorting, Wallpaper},
    net::{RateLimiter, get_json},
};

const PER_PAGE: u32 = 50;

pub struct Pixabay {
    client: reqwest::Client,
    key: Option<String>,
    limiter: RateLimiter,
}

impl Pixabay {
    pub fn new(client: reqwest::Client, key: Option<String>) -> Self {
        Self { client, key, limiter: RateLimiter::new(60, Duration::from_secs(60)) }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Resp {
    total_hits: u64,
    hits: Vec<Hit>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Hit {
    id: u64,
    #[serde(rename = "pageURL")]
    page_url: String,
    tags: String,
    #[serde(rename = "webformatURL")]
    webformat_url: String,
    #[serde(rename = "largeImageURL")]
    large_image_url: String,
    #[serde(rename = "fullHDURL")]
    full_hd_url: Option<String>,
    #[serde(rename = "imageURL")]
    image_url: Option<String>,
    image_width: u32,
    image_height: u32,
    image_size: Option<u64>,
    user: String,
    user_id: u64,
}

/// Pixabay colour filter names, matched from our hex palette by hue.
fn color_name(hex: &str) -> Option<&'static str> {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).ok()?;
    let (r, g, b) = (((v >> 16) & 255) as f32, ((v >> 8) & 255) as f32, (v & 255) as f32);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    if max - min < 30.0 {
        return Some(if max < 60.0 { "black" } else if max > 200.0 { "white" } else { "gray" });
    }
    let h = if max == r {
        60.0 * (((g - b) / (max - min)) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / (max - min) + 2.0)
    } else {
        60.0 * ((r - g) / (max - min) + 4.0)
    };
    let h = if h < 0.0 { h + 360.0 } else { h };
    Some(match h as u32 {
        0..=15 | 346..=360 => "red",
        16..=40 => if max < 150.0 { "brown" } else { "orange" },
        41..=70 => "yellow",
        71..=160 => "green",
        161..=200 => "turquoise",
        201..=260 => "blue",
        261..=300 => "lilac",
        _ => "pink",
    })
}

#[async_trait]
impl Provider for Pixabay {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: "pixabay",
            name: "Pixabay",
            homepage: "https://pixabay.com",
            key_required: true,
            key_optional: false,
            order: 30,
            key_help: "Free API key: pixabay.com/api/docs (log in, key shown under 'Parameters'). Images are ≤1280px unless your key has full access.",
            has_key: self.key.is_some(),
            caps: ProviderCaps {
                text_search: true,
                sortings: vec![Sorting::Popular, Sorting::Latest],
                orientation: true,
                colors: true,
                min_resolution: true,
                ..Default::default()
            },
        }
    }

    async fn search(&self, q: &SearchQuery, page: u32) -> Result<SearchPage> {
        let Some(key) = &self.key else { bail!("Pixabay needs an API key (Settings > Sources)") };
        self.limiter.acquire().await;
        let order = if q.sorting == Sorting::Latest { "latest" } else { "popular" };
        let mut url = format!(
            "https://pixabay.com/api/?key={}&q={}&image_type=photo&safesearch=true&order={order}&page={page}&per_page={PER_PAGE}",
            urlencoding::encode(key),
            urlencoding::encode(q.text.trim()),
        );
        if q.text.trim().is_empty() {
            url += "&category=backgrounds";
        }
        match q.orientation {
            Orientation::Landscape => url += "&orientation=horizontal",
            Orientation::Portrait => url += "&orientation=vertical",
            Orientation::Any => {}
        }
        if let Some((w, h)) = q.min_resolution {
            url += &format!("&min_width={w}&min_height={h}");
        }
        if let Some(c) = q.color.as_deref().and_then(color_name) {
            url += &format!("&colors={c}");
        }
        let resp: Resp = get_json(&self.client, &url).await?;
        let last = (resp.total_hits as u32).div_ceil(PER_PAGE).max(1);
        let items = resp
            .hits
            .into_iter()
            .map(|h| Wallpaper {
                provider: "pixabay".into(),
                remote_id: h.id.to_string(),
                page_url: Some(h.page_url),
                full_url: h.image_url.or(h.full_hd_url).unwrap_or(h.large_image_url.clone()),
                thumb_url: h.webformat_url,
                preview_url: Some(h.large_image_url),
                width: h.image_width,
                height: h.image_height,
                file_size: h.image_size,
                author_url: Some(format!("https://pixabay.com/users/{}-{}/", h.user, h.user_id)),
                author: Some(h.user),
                license: Some("Pixabay Content License".into()),
                tags: h.tags.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect(),
                ..Default::default()
            })
            .collect();
        Ok(SearchPage { items, page, last_page: Some(last), total: Some(resp.total_hits), seed: None })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn colour_names() {
        assert_eq!(super::color_name("cc0000"), Some("red"));
        assert_eq!(super::color_name("0066cc"), Some("blue"));
        assert_eq!(super::color_name("000000"), Some("black"));
        assert_eq!(super::color_name("77cc33"), Some("green"));
    }
}
