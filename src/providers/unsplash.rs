//! https://unsplash.com/documentation — needs a free "Access Key" (demo apps: 50 req/hour).
//! API guidelines require hotlinking the returned URLs, attributing the photographer,
//! and pinging `links.download_location` when a photo is downloaded.
use std::time::Duration;

use anyhow::{Result, bail};
use async_trait::async_trait;
use serde::Deserialize;

use super::{Provider, ProviderCaps, ProviderInfo};
use crate::{
    model::{Orientation, SearchPage, SearchQuery, Sorting, Wallpaper},
    net::{RateLimiter, get_bytes_with, get_json_with},
};

const API: &str = "https://api.unsplash.com";
const PER_PAGE: u32 = 30;

pub struct Unsplash {
    client: reqwest::Client,
    key: Option<String>,
    limiter: RateLimiter,
}

impl Unsplash {
    pub fn new(client: reqwest::Client, key: Option<String>) -> Self {
        Self { client, key, limiter: RateLimiter::new(10, Duration::from_secs(60)) }
    }

    fn auth(&self) -> Result<String> {
        match &self.key {
            Some(k) => Ok(format!("Client-ID {k}")),
            None => bail!("Unsplash needs an access key (Settings > Sources)"),
        }
    }
}

#[derive(Deserialize)]
struct SearchResp {
    total: u64,
    total_pages: u32,
    results: Vec<Photo>,
}

#[derive(Deserialize)]
struct Photo {
    id: String,
    width: u32,
    height: u32,
    color: Option<String>,
    description: Option<String>,
    alt_description: Option<String>,
    created_at: Option<String>,
    urls: Urls,
    links: Links,
    user: User,
}

#[derive(Deserialize)]
struct Urls {
    full: String,
    regular: String,
    small: String,
}

#[derive(Deserialize)]
struct Links {
    html: String,
    download_location: String,
}

#[derive(Deserialize)]
struct User {
    name: String,
    links: UserLinks,
}

#[derive(Deserialize)]
struct UserLinks {
    html: String,
}

const UTM: &str = "?utm_source=walle&utm_medium=referral";

impl From<Photo> for Wallpaper {
    fn from(p: Photo) -> Self {
        let mut extra = std::collections::BTreeMap::new();
        extra.insert("download_location".into(), p.links.download_location);
        if let Some(d) = &p.description {
            extra.insert("description".into(), d.clone());
        }
        Wallpaper {
            provider: "unsplash".into(),
            remote_id: p.id,
            page_url: Some(format!("{}{UTM}", p.links.html)),
            full_url: p.urls.full,
            thumb_url: p.urls.small,
            preview_url: Some(p.urls.regular),
            width: p.width,
            height: p.height,
            file_type: Some("image/jpeg".into()),
            title: p.alt_description.or(p.description).map(capitalize),
            author: Some(p.user.name),
            author_url: Some(format!("{}{UTM}", p.user.links.html)),
            license: Some("Unsplash License".into()),
            colors: p.color.into_iter().collect(),
            created_at: p.created_at,
            extra,
            ..Default::default()
        }
    }
}

fn capitalize(s: String) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => s,
    }
}

#[async_trait]
impl Provider for Unsplash {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: "unsplash",
            name: "Unsplash",
            homepage: "https://unsplash.com",
            key_required: true,
            key_optional: false,
            order: 10,
            key_help: "Free access key: unsplash.com/oauth/applications > New Application > Access Key.",
            has_key: self.key.is_some(),
            caps: ProviderCaps {
                text_search: true,
                sortings: vec![Sorting::Relevance, Sorting::Latest],
                orientation: true,
                ..Default::default()
            },
        }
    }

    async fn search(&self, q: &SearchQuery, page: u32) -> Result<SearchPage> {
        let auth = self.auth()?;
        self.limiter.acquire().await;
        let headers = [("Authorization", auth.as_str()), ("Accept-Version", "v1")];
        let text = q.text.trim();
        if text.is_empty() {
            // The curated "Wallpapers" topic is the best default feed.
            let order = if q.sorting == Sorting::Latest { "latest" } else { "popular" };
            let mut url = format!("{API}/topics/wallpapers/photos?page={page}&per_page={PER_PAGE}&order_by={order}");
            match q.orientation {
                Orientation::Landscape => url += "&orientation=landscape",
                Orientation::Portrait => url += "&orientation=portrait",
                Orientation::Any => {}
            }
            let photos: Vec<Photo> = get_json_with(&self.client, &url, &headers).await?;
            let full = photos.len() as u32 == PER_PAGE;
            return Ok(SearchPage {
                items: photos.into_iter().map(Into::into).collect(),
                page,
                last_page: if full { None } else { Some(page) },
                ..Default::default()
            });
        }
        let order = if q.sorting == Sorting::Latest { "latest" } else { "relevant" };
        let mut url = format!(
            "{API}/search/photos?query={}&page={page}&per_page={PER_PAGE}&order_by={order}",
            urlencoding::encode(text)
        );
        match q.orientation {
            Orientation::Landscape => url += "&orientation=landscape",
            Orientation::Portrait => url += "&orientation=portrait",
            Orientation::Any => {}
        }
        let resp: SearchResp = get_json_with(&self.client, &url, &headers).await?;
        Ok(SearchPage {
            items: resp.results.into_iter().map(Into::into).collect(),
            page,
            last_page: Some(resp.total_pages),
            total: Some(resp.total),
            seed: None,
        })
    }

    async fn track_download(&self, wp: &Wallpaper) -> Result<()> {
        let Some(loc) = wp.extra.get("download_location") else { return Ok(()) };
        let auth = self.auth()?;
        get_bytes_with(&self.client, loc, &[("Authorization", auth.as_str())]).await?;
        Ok(())
    }
}
