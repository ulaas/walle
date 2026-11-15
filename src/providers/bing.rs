//! Bing "image of the day" (unofficial HPImageArchive endpoint, no key).
use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;

use super::{Provider, ProviderCaps, ProviderInfo};
use crate::{
    model::{SearchPage, SearchQuery, Wallpaper},
    net::get_json,
};

pub struct Bing {
    client: reqwest::Client,
    market: String,
}

impl Bing {
    pub fn new(client: reqwest::Client, market: Option<String>) -> Self {
        Self { client, market: market.unwrap_or_else(|| "en-US".into()) }
    }
}

#[derive(Deserialize)]
struct Resp {
    #[serde(default)]
    images: Vec<Img>,
}

#[derive(Deserialize)]
struct Img {
    startdate: String,
    urlbase: String,
    #[serde(default)]
    copyright: String,
    #[serde(default)]
    copyrightlink: String,
    #[serde(default)]
    title: String,
}

#[async_trait]
impl Provider for Bing {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: "bing",
            name: "Bing Daily",
            homepage: "https://www.bing.com",
            key_required: false,
            key_optional: false,
            order: 40,
            key_help: "No key needed. Option `market` (e.g. en-US, de-DE) in config.",
            has_key: false,
            caps: ProviderCaps::default(),
        }
    }

    async fn search(&self, _q: &SearchQuery, page: u32) -> Result<SearchPage> {
        // The archive only exposes ~15 days: idx 0..=7, n<=8.
        let idx = if page <= 1 { 0 } else { 7 };
        if page > 2 {
            return Ok(SearchPage { page, last_page: Some(2), ..Default::default() });
        }
        let url = format!(
            "https://www.bing.com/HPImageArchive.aspx?format=js&idx={idx}&n=8&mkt={}",
            urlencoding::encode(&self.market)
        );
        let resp: Resp = get_json(&self.client, &url).await?;
        let items = resp
            .images
            .into_iter()
            .map(|i| {
                let base = format!("https://www.bing.com{}", i.urlbase);
                let (title, author) = split_copyright(&i.copyright);
                Wallpaper {
                    provider: "bing".into(),
                    remote_id: i.startdate.clone(),
                    page_url: Some(i.copyrightlink).filter(|s| !s.is_empty()),
                    full_url: format!("{base}_UHD.jpg"),
                    thumb_url: format!("{base}_400x240.jpg"),
                    preview_url: Some(format!("{base}_1920x1080.jpg")),
                    width: 3840,
                    height: 2160,
                    file_type: Some("image/jpeg".into()),
                    title: Some(if i.title.is_empty() { title.clone() } else { format!("{} — {}", i.title, title) }),
                    author,
                    created_at: Some(i.startdate),
                    ..Default::default()
                }
            })
            .collect::<Vec<_>>();
        // Page 2 overlaps page 1 by one day; drop that duplicate.
        let items = if page == 2 { items.into_iter().skip(1).collect() } else { items };
        Ok(SearchPage { items, page, last_page: Some(2), total: None, seed: None })
    }
}

/// "Lake Braies, Dolomites, Italy (© leonid_tit/Getty Images)" → (place, credit)
fn split_copyright(s: &str) -> (String, Option<String>) {
    match s.rfind(" (©") {
        Some(i) => (
            s[..i].to_string(),
            Some(s[i + 2..].trim_end_matches(')').trim().to_string()),
        ),
        None => (s.to_string(), None),
    }
}
