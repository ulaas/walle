//! https://picsum.photos — Unsplash-sourced photos, no key, paged list only.
use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;

use super::{Provider, ProviderCaps, ProviderInfo};
use crate::{
    model::{SearchPage, SearchQuery, Wallpaper},
    net::get_json,
};

const PER_PAGE: usize = 30;

pub struct Picsum {
    client: reqwest::Client,
}

impl Picsum {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[derive(Deserialize)]
struct Item {
    id: String,
    author: String,
    width: u32,
    height: u32,
    url: String,
    download_url: String,
}

#[async_trait]
impl Provider for Picsum {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: "picsum",
            name: "Lorem Picsum",
            homepage: "https://picsum.photos",
            key_required: false,
            key_optional: false,
            order: 70,
            key_help: "No key needed.",
            has_key: false,
            caps: ProviderCaps::default(),
        }
    }

    async fn search(&self, _q: &SearchQuery, page: u32) -> Result<SearchPage> {
        let page = page.max(1);
        let url = format!("https://picsum.photos/v2/list?page={page}&limit={PER_PAGE}");
        let list: Vec<Item> = get_json(&self.client, &url).await?;
        let full = list.len() == PER_PAGE;
        let items = list
            .into_iter()
            .map(|i| Wallpaper {
                provider: "picsum".into(),
                remote_id: i.id.clone(),
                page_url: Some(i.url.clone()),
                full_url: i.download_url,
                thumb_url: format!("https://picsum.photos/id/{}/400/250", i.id),
                preview_url: Some(format!("https://picsum.photos/id/{}/1280/{}", i.id, 1280 * i.height / i.width.max(1))),
                width: i.width,
                height: i.height,
                file_type: Some("image/jpeg".into()),
                author: Some(i.author),
                source: Some(i.url),
                license: Some("Unsplash License".into()),
                ..Default::default()
            })
            .collect();
        Ok(SearchPage {
            items,
            page,
            last_page: if full { None } else { Some(page) },
            total: None,
            seed: None,
        })
    }
}
