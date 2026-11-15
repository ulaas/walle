//! https://wallhaven.cc/help/api
use std::time::Duration;

use anyhow::{Result, bail};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use super::{Collection, Provider, ProviderCaps, ProviderInfo};
use crate::{
    model::{Orientation, SearchPage, SearchQuery, Sorting, Wallpaper},
    net::{RateLimiter, get_json},
};

const API: &str = "https://wallhaven.cc/api/v1";

pub struct Wallhaven {
    client: reqwest::Client,
    api_key: Option<String>,
    /// Whose collections to browse (collection contents are addressed by username).
    username: Option<String>,
    limiter: RateLimiter,
}

impl Wallhaven {
    pub fn new(client: reqwest::Client, api_key: Option<String>, username: Option<String>) -> Self {
        Self {
            client,
            api_key,
            username,
            // API allows 45 calls/min; stay a little below.
            limiter: RateLimiter::new(40, Duration::from_secs(60)),
        }
    }

    fn with_key(&self, mut url: String) -> String {
        if let Some(k) = &self.api_key {
            url.push(if url.contains('?') { '&' } else { '?' });
            url.push_str("apikey=");
            url.push_str(&urlencoding::encode(k));
        }
        url
    }

    /// Contents of a collection: `/collections/{username}/{id}` (purity filter applies).
    fn collection_url(&self, q: &SearchQuery, id: &str, page: u32) -> Result<String> {
        let Some(user) = &self.username else { bail!("set your Wallhaven username in Settings to open collections") };
        if !id.chars().all(|c| c.is_ascii_digit()) {
            bail!("invalid collection id {id:?}");
        }
        let bits = |b: [bool; 3]| b.iter().map(|&x| if x { '1' } else { '0' }).collect::<String>();
        let mut purity = q.purity;
        if self.api_key.is_none() {
            purity[2] = false;
        }
        if !purity.iter().any(|&x| x) {
            purity[0] = true;
        }
        Ok(self.with_key(format!("{API}/collections/{}/{id}?page={page}&purity={}", urlencoding::encode(user), bits(purity))))
    }

    pub fn search_url(&self, q: &SearchQuery, page: u32) -> String {
        let bits = |b: [bool; 3]| b.iter().map(|&x| if x { '1' } else { '0' }).collect::<String>();
        let mut purity = q.purity;
        if self.api_key.is_none() {
            purity[2] = false;
        }
        if !purity.iter().any(|&x| x) {
            purity[0] = true;
        }
        let mut cats = q.categories;
        if !cats.iter().any(|&x| x) {
            cats = [true; 3];
        }
        let sorting = match q.sorting {
            Sorting::Relevance => "relevance",
            Sorting::Random => "random",
            Sorting::Views => "views",
            Sorting::Favorites => "favorites",
            Sorting::Toplist => "toplist",
            Sorting::Hot => "hot",
            _ => "date_added",
        };
        let mut url = format!(
            "{API}/search?q={}&categories={}&purity={}&sorting={sorting}&order={}&page={page}",
            urlencoding::encode(q.text.trim()),
            bits(cats),
            bits(purity),
            if q.descending { "desc" } else { "asc" },
        );
        if q.sorting == Sorting::Toplist && !q.toplist_range.is_empty() {
            url += &format!("&topRange={}", q.toplist_range);
        }
        if let Some((w, h)) = q.min_resolution {
            url += &format!("&atleast={w}x{h}");
        }
        if !q.ratios.is_empty() {
            url += &format!("&ratios={}", q.ratios.join(","));
        } else if q.orientation != Orientation::Any {
            url += if q.orientation == Orientation::Portrait { "&ratios=portrait" } else { "&ratios=landscape" };
        }
        if let Some(c) = &q.color {
            url += &format!("&colors={}", c.trim_start_matches('#'));
        }
        if let Some(seed) = &q.seed {
            url += &format!("&seed={seed}");
        }
        self.with_key(url)
    }
}

#[derive(Deserialize)]
struct SearchResp {
    data: Vec<WhWallpaper>,
    meta: Option<Value>,
}

#[derive(Deserialize)]
struct CollectionsResp {
    data: Vec<WhCollection>,
}

#[derive(Deserialize)]
struct WhCollection {
    id: u64,
    label: String,
    count: Value,
}

#[derive(Deserialize)]
struct DetailResp {
    data: WhWallpaper,
}

#[derive(Deserialize)]
struct WhWallpaper {
    id: String,
    url: String,
    #[serde(default)]
    source: String,
    purity: Option<String>,
    category: Option<String>,
    dimension_x: u32,
    dimension_y: u32,
    file_size: Option<u64>,
    file_type: Option<String>,
    created_at: Option<String>,
    #[serde(default)]
    colors: Vec<String>,
    path: String,
    thumbs: Thumbs,
    #[serde(default)]
    tags: Vec<WhTag>,
    uploader: Option<Uploader>,
}

#[derive(Deserialize)]
struct Thumbs {
    large: String,
    original: String,
    #[allow(dead_code)]
    small: String,
}

#[derive(Deserialize)]
struct WhTag {
    name: String,
}

#[derive(Deserialize)]
struct Uploader {
    username: String,
}

impl From<WhWallpaper> for Wallpaper {
    fn from(w: WhWallpaper) -> Self {
        let author_url = w.uploader.as_ref().map(|u| format!("https://wallhaven.cc/user/{}", u.username));
        Wallpaper {
            provider: "wallhaven".into(),
            remote_id: w.id.clone(),
            page_url: Some(w.url),
            full_url: w.path,
            thumb_url: w.thumbs.large,
            preview_url: Some(w.thumbs.original),
            width: w.dimension_x,
            height: w.dimension_y,
            file_size: w.file_size,
            file_type: w.file_type,
            title: None,
            author: w.uploader.map(|u| u.username),
            author_url,
            source: Some(w.source).filter(|s| !s.is_empty()),
            license: None,
            purity: w.purity,
            category: w.category,
            colors: w.colors,
            tags: w.tags.into_iter().map(|t| t.name).collect(),
            created_at: w.created_at,
            extra: Default::default(),
        }
    }
}

/// Wallhaven returns some numeric fields as strings; accept both.
fn num(v: Option<&Value>) -> Option<u64> {
    match v? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

#[async_trait]
impl Provider for Wallhaven {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: "wallhaven",
            name: "Wallhaven",
            homepage: "https://wallhaven.cc",
            key_required: false,
            key_optional: true,
            order: 0,
            key_help: "Optional. Enables NSFW and your private collections (wallhaven.cc/settings/account). Set your username to browse collections.",
            has_key: self.api_key.is_some(),
            caps: ProviderCaps {
                text_search: true,
                sortings: vec![
                    Sorting::Toplist,
                    Sorting::Hot,
                    Sorting::DateAdded,
                    Sorting::Relevance,
                    Sorting::Random,
                    Sorting::Views,
                    Sorting::Favorites,
                ],
                order: true,
                toplist_range: true,
                categories: true,
                purity: true,
                nsfw_needs_key: true,
                min_resolution: true,
                ratios: true,
                orientation: true,
                colors: true,
                details: true,
                similar: true,
                collections: true,
            },
        }
    }

    async fn search(&self, query: &SearchQuery, page: u32) -> Result<SearchPage> {
        self.limiter.acquire().await;
        let url = match &query.collection {
            Some(id) => self.collection_url(query, id, page)?,
            None => self.search_url(query, page),
        };
        let resp: SearchResp = get_json(&self.client, &url).await?;
        let meta = resp.meta.unwrap_or(Value::Null);
        Ok(SearchPage {
            items: resp.data.into_iter().map(Into::into).collect(),
            page: num(meta.get("current_page")).map(|n| n as u32).unwrap_or(page),
            last_page: num(meta.get("last_page")).map(|n| n as u32),
            total: num(meta.get("total")),
            seed: meta.get("seed").and_then(|s| s.as_str()).map(String::from),
        })
    }

    async fn collections(&self) -> Result<Vec<Collection>> {
        let url = match (&self.username, &self.api_key) {
            (Some(u), _) => self.with_key(format!("{API}/collections/{}", urlencoding::encode(u))),
            (None, Some(_)) => bail!("set your Wallhaven username in Settings to open collections"),
            (None, None) => return Ok(vec![]),
        };
        self.limiter.acquire().await;
        let resp: CollectionsResp = get_json(&self.client, &url).await?;
        Ok(resp
            .data
            .into_iter()
            .map(|c| Collection { id: c.id.to_string(), label: c.label, count: num(Some(&c.count)).unwrap_or(0) })
            .collect())
    }

    async fn details(&self, id: &str) -> Result<Wallpaper> {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
            bail!("invalid wallhaven id {id:?}");
        }
        self.limiter.acquire().await;
        let url = self.with_key(format!("{API}/w/{id}"));
        let resp: DetailResp = get_json(&self.client, &url).await?;
        Ok(resp.data.into())
    }
}
