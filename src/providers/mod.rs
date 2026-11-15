use std::{collections::BTreeMap, sync::Arc};

use anyhow::{Result, bail};
use async_trait::async_trait;

use crate::{
    config::Config,
    model::{SearchPage, SearchQuery, Sorting, Wallpaper},
};

pub mod bing;
pub mod local;
pub mod nasa;
pub mod openverse;
pub mod pexels;
pub mod picsum;
pub mod pixabay;
pub mod unsplash;
pub mod wallhaven;

/// Which parts of `SearchQuery` a provider understands; drives the filter sidebar.
#[derive(Clone, Debug, Default)]
pub struct ProviderCaps {
    pub text_search: bool,
    pub sortings: Vec<Sorting>,
    pub order: bool,
    pub toplist_range: bool,
    pub categories: bool,
    pub purity: bool,
    /// NSFW purity is only allowed with an API key.
    pub nsfw_needs_key: bool,
    pub min_resolution: bool,
    pub ratios: bool,
    pub orientation: bool,
    pub colors: bool,
    pub details: bool,
    /// Supports `like:<id>` style "similar" search.
    pub similar: bool,
    /// Has user collections (see `Provider::collections`).
    pub collections: bool,
}

#[derive(Clone, Debug)]
pub struct ProviderInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub homepage: &'static str,
    pub key_required: bool,
    /// Shows an API key field even when the key is optional.
    pub key_optional: bool,
    pub key_help: &'static str,
    pub caps: ProviderCaps,
    pub has_key: bool,
    /// Sort position in pickers.
    pub order: u8,
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn info(&self) -> ProviderInfo;
    async fn search(&self, query: &SearchQuery, page: u32) -> Result<SearchPage>;
    async fn details(&self, _id: &str) -> Result<Wallpaper> {
        bail!("details not supported by {}", self.info().name)
    }
    /// Collections the user can browse (`SearchQuery::collection` = `Collection::id`).
    async fn collections(&self) -> Result<Vec<Collection>> {
        Ok(vec![])
    }
    /// Called after the full image was downloaded (some APIs require a tracking ping).
    async fn track_download(&self, _wp: &Wallpaper) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Collection {
    pub id: String,
    pub label: String,
    pub count: u64,
}

pub type Registry = BTreeMap<String, Arc<dyn Provider>>;

/// Static info for every known provider (enabled or not), in display order.
pub fn all_infos(cfg: &Config) -> Vec<ProviderInfo> {
    let mut v: Vec<_> = build_all(cfg).into_values().map(|p| p.info()).collect();
    v.sort_by_key(|i| i.order);
    v
}

fn build_all(cfg: &Config) -> Registry {
    let client = crate::net::client();
    let mut r: Registry = BTreeMap::new();
    let mut add = |p: Arc<dyn Provider>| {
        r.insert(p.info().id.to_string(), p);
    };
    add(Arc::new(wallhaven::Wallhaven::new(
        client.clone(),
        cfg.api_key("wallhaven"),
        cfg.provider("wallhaven").options.get("username").cloned().filter(|u| !u.trim().is_empty()),
    )));
    add(Arc::new(unsplash::Unsplash::new(client.clone(), cfg.api_key("unsplash"))));
    add(Arc::new(pexels::Pexels::new(client.clone(), cfg.api_key("pexels"))));
    add(Arc::new(pixabay::Pixabay::new(client.clone(), cfg.api_key("pixabay"))));
    add(Arc::new(bing::Bing::new(client.clone(), cfg.provider("bing").options.get("market").cloned())));
    add(Arc::new(nasa::Nasa::new(client.clone())));
    add(Arc::new(openverse::Openverse::new(client.clone())));
    add(Arc::new(picsum::Picsum::new(client)));
    add(Arc::new(local::LocalFolders::new(cfg.local_folders.clone())));
    r
}

/// Enabled providers whose key requirements are met.
pub fn build_registry(cfg: &Config) -> Registry {
    build_all(cfg)
        .into_iter()
        .filter(|(id, p)| {
            let info = p.info();
            cfg.provider(id).enabled && (!info.key_required || info.has_key)
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;

    /// Live network smoke test: `cargo test -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn live_providers() {
        let cfg = Config::default();
        for (id, p) in build_registry(&cfg).into_iter().filter(|(id, _)| id != "local") {
            let page = p.search(&SearchQuery::default(), 1).await.unwrap_or_else(|e| panic!("{id}: {e:#}"));
            assert!(!page.items.is_empty(), "{id} returned nothing");
            let w = &page.items[0];
            assert!(w.full_url.starts_with("https://") && w.thumb_url.starts_with("https://"), "{id}: {w:?}");
            println!("{id}: {} items, has_more={}, first={} {}", page.items.len(), page.has_more(), w.key(), w.resolution());
            if p.info().caps.details {
                let d = p.details(&w.remote_id).await.unwrap();
                assert_eq!(d.key(), w.key(), "{id} details are of the same wallpaper");
                println!("  tags: {:?}", &d.tags[..d.tags.len().min(5)]);
            }
        }
        // Public Wallhaven collections (no key needed): user "Gandalf" has collection 3.
        let wh = wallhaven::Wallhaven::new(crate::net::client(), None, Some("Gandalf".into()));
        let cols = wh.collections().await.unwrap();
        let c = cols.iter().find(|c| c.id == "3").expect("collection 3");
        println!("wallhaven collections: {} (e.g. {} with {})", cols.len(), c.label, c.count);
        let q = SearchQuery { collection: Some("3".into()), ..Default::default() };
        let page = wh.search(&q, 1).await.unwrap();
        assert!(!page.items.is_empty());
        println!("  collection page 1: {} items, has_more={}", page.items.len(), page.has_more());
        let p2 = build_registry(&cfg)["bing"].search(&SearchQuery::default(), 2).await.unwrap();
        println!("bing page 2: {} items", p2.items.len());
    }
}
