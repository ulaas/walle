//! https://api.openverse.org — openly licensed images (CC / public domain). Anonymous access,
//! limited to 20 results per page.
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;

use super::{Provider, ProviderCaps, ProviderInfo};
use crate::{
    model::{Orientation, SearchPage, SearchQuery, Wallpaper},
    net::{RateLimiter, get_json},
};

pub struct Openverse {
    client: reqwest::Client,
    limiter: RateLimiter,
}

impl Openverse {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client, limiter: RateLimiter::new(20, Duration::from_secs(60)) }
    }
}

#[derive(Deserialize)]
struct Resp {
    result_count: Option<u64>,
    page_count: Option<u32>,
    results: Vec<Img>,
}

#[derive(Deserialize)]
struct Img {
    id: String,
    title: Option<String>,
    foreign_landing_url: Option<String>,
    url: String,
    creator: Option<String>,
    creator_url: Option<String>,
    license: Option<String>,
    license_version: Option<String>,
    #[serde(default)]
    tags: Vec<Tag>,
    thumbnail: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    filesize: Option<u64>,
    filetype: Option<String>,
}

#[derive(Deserialize)]
struct Tag {
    name: String,
}

#[async_trait]
impl Provider for Openverse {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: "openverse",
            name: "Openverse",
            homepage: "https://openverse.org",
            key_required: false,
            key_optional: false,
            order: 60,
            key_help: "No key needed. Openly licensed images; check each licence for attribution.",
            has_key: false,
            caps: ProviderCaps { text_search: true, orientation: true, ..Default::default() },
        }
    }

    async fn search(&self, q: &SearchQuery, page: u32) -> Result<SearchPage> {
        self.limiter.acquire().await;
        let text = q.text.trim();
        let aspect = match q.orientation {
            Orientation::Portrait => "tall",
            _ => "wide",
        };
        let url = format!(
            "https://api.openverse.org/v1/images/?q={}&page={page}&page_size=20&size=large&aspect_ratio={aspect}",
            urlencoding::encode(if text.is_empty() { "landscape wallpaper" } else { text })
        );
        let resp: Resp = get_json(&self.client, &url).await?;
        let items = resp
            .results
            .into_iter()
            .map(|i| Wallpaper {
                provider: "openverse".into(),
                remote_id: i.id.clone(),
                page_url: i.foreign_landing_url.clone(),
                thumb_url: i.thumbnail.unwrap_or_else(|| format!("https://api.openverse.org/v1/images/{}/thumb/", i.id)),
                full_url: i.url,
                preview_url: None,
                width: i.width.unwrap_or(0),
                height: i.height.unwrap_or(0),
                file_size: i.filesize,
                file_type: i.filetype.map(|t| format!("image/{t}")),
                title: i.title,
                author: i.creator,
                author_url: i.creator_url,
                source: i.foreign_landing_url,
                license: i.license.map(|l| format!("CC {} {}", l.to_uppercase(), i.license_version.unwrap_or_default()).trim().to_string()),
                tags: i.tags.into_iter().map(|t| t.name).collect(),
                ..Default::default()
            })
            .collect();
        Ok(SearchPage { items, page, last_page: resp.page_count, total: resp.result_count, seed: None })
    }
}
