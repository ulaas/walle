//! NASA Astronomy Picture of the Day, read from science.nasa.gov, where APOD has lived since the
//! old apod.nasa.gov pages began redirecting there.
//!
//! api.nasa.gov/planetary/apod still answers, but it reads the old pages, so every day comes back
//! with the title "NASA Science" and the NASA logo as its image. The site's own WordPress API has
//! the real data, and needs no key.
use std::time::Duration;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use chrono::NaiveDate;
use serde::Deserialize;

use super::{Provider, ProviderCaps, ProviderInfo};
use crate::{
    model::{SearchPage, SearchQuery, Wallpaper},
    net::{RateLimiter, get_json},
};

const POSTS: &str = "https://science.nasa.gov/wp-json/wp/v2/image-article";
/// The "apod" category of those posts.
const APOD_CATEGORY: u32 = 22766;
const PER_PAGE: usize = 30;
/// Where APOD's own pictures are stored. Posts without a picture have a site-wide placeholder
/// instead, which must not pass for a wallpaper.
const APOD_ASSETS: &str = "/cds/apod/";
const IMAGE_TYPES: &[&str] = &["jpg", "jpeg", "png", "webp", "gif"];
/// The image service scales on request, which saves fetching 30 MB originals for thumbnails.
const THUMB_SIZE: u32 = 640;
const PREVIEW_SIZE: u32 = 1600;

pub struct Nasa {
    client: reqwest::Client,
    limiter: RateLimiter,
}

impl Nasa {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client, limiter: RateLimiter::new(60, Duration::from_secs(60)) }
    }
}

#[derive(Deserialize)]
struct Rendered {
    rendered: String,
}

#[derive(Deserialize)]
struct Post {
    /// Local time at the APOD office, e.g. `2026-11-04T00:05:00`.
    date: String,
    link: String,
    title: Rendered,
    #[serde(default)]
    featured_image_url: Option<String>,
    /// Requested for the details only: it's what makes a full response 40 times the size.
    #[serde(default)]
    content: Option<Rendered>,
}

/// Replace the character references WordPress uses.
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let decoded = rest.find(';').filter(|end| *end <= 10).and_then(|end| {
            let name = &rest[1..end];
            let c = if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
            } else if let Some(dec) = name.strip_prefix('#') {
                dec.parse().ok().and_then(char::from_u32)
            } else {
                match name {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some(' '),
                    _ => None,
                }
            };
            c.map(|c| (c, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Markup to plain text on one line.
fn plain_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            c if !in_tag => text.push(c),
            _ => {}
        }
    }
    decode_entities(&text).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `APOD: 2026 November 4 – The Name` (or `APOD: 29 October 2026 – The Name`) to `The Name`.
fn clean_title(rendered: &str) -> String {
    let title = decode_entities(rendered);
    let title = title.trim();
    let without_prefix = title.strip_prefix("APOD:").map(str::trim).unwrap_or(title);
    match without_prefix.split_once(" \u{2013} ") {
        Some((_, name)) if !name.trim().is_empty() => name.trim().to_string(),
        _ => without_prefix.to_string(),
    }
}

/// The picture's address and the size given in its query (`...ic342.png?w=4144&h=2822&...`).
fn split_image_url(url: &str) -> (&str, u32, u32) {
    let (file, query) = url.split_once('?').unwrap_or((url, ""));
    let param = |name: &str| query.split('&').find_map(|kv| kv.strip_prefix(name)?.strip_prefix('=')?.parse().ok()).unwrap_or(0);
    (file, param("w"), param("h"))
}

fn scaled(file: &str, size: u32) -> String {
    format!("{file}?w={size}&h={size}&fit=clip")
}

fn to_wallpaper(post: Post) -> Option<Wallpaper> {
    let image = post.featured_image_url.as_deref()?;
    let (file, width, height) = split_image_url(image);
    let extension = file.rsplit('.').next()?.to_ascii_lowercase();
    if !file.contains(APOD_ASSETS) || !IMAGE_TYPES.contains(&extension.as_str()) {
        return None;
    }
    let date = post.date.get(..10)?.to_string();
    let mut wp = Wallpaper {
        provider: "nasa".into(),
        remote_id: date.clone(),
        page_url: Some(post.link),
        // Without a size: the plain address serves the original, while the size in the post can
        // be capped (older posts say 1200 for images of 4000 and more; the proportions are right).
        full_url: file.to_string(),
        thumb_url: scaled(file, THUMB_SIZE),
        preview_url: Some(scaled(file, PREVIEW_SIZE)),
        width,
        height,
        file_type: Some(format!("image/{}", if extension == "jpg" { "jpeg" } else { &extension })),
        title: Some(clean_title(&post.title.rendered)),
        license: Some("See the APOD page (often public domain)".into()),
        created_at: Some(date),
        ..Default::default()
    };
    if let Some(content) = post.content {
        add_page_details(&mut wp, &content.rendered);
    }
    Some(wp)
}

/// The text between the end of the tag containing `marker` and the next `until`.
fn text_after<'a>(html: &'a str, marker: &str, until: &str) -> Option<&'a str> {
    let after = &html[html.find(marker)? + marker.len()..];
    let start = after.find('>')? + 1;
    let inner = &after[start..];
    Some(&inner[..inner.find(until)?])
}

/// Credit and explanation, which the post only has as part of its page.
fn add_page_details(wp: &mut Wallpaper, html: &str) {
    if let Some(text) = text_after(html, "media-detail-hero__description", "</p>") {
        let text = plain_text(text);
        let text = text.strip_prefix("Explanation:").unwrap_or(&text);
        // The page's pointer to the next day isn't part of the description.
        let text = text.split("Tomorrow's picture:").next().unwrap_or(text).split("Tomorrow\u{2019}s picture:").next().unwrap_or(text).trim();
        if !text.is_empty() {
            wp.extra.insert("description".into(), text.to_string());
        }
    }
    // A row of the details table, labelled "Credit" or "Credit & Copyright:".
    if let Some(row) = html.find(">Credit").map(|at| &html[at..])
        && let Some(cell) = text_after(row, "<td", "</td>")
    {
        let credit = plain_text(cell);
        let credit = credit.strip_prefix("Credit & Copyright:").or_else(|| credit.strip_prefix("Credit:")).unwrap_or(&credit).trim();
        if !credit.is_empty() {
            wp.author = Some(credit.to_string());
        }
    }
}

#[async_trait]
impl Provider for Nasa {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: "nasa",
            name: "NASA APOD",
            homepage: "https://science.nasa.gov/apod/",
            key_required: false,
            key_optional: false,
            order: 50,
            key_help: "No key needed.",
            has_key: false,
            caps: ProviderCaps { details: true, ..Default::default() },
        }
    }

    async fn search(&self, _q: &SearchQuery, page: u32) -> Result<SearchPage> {
        let page = page.max(1);
        self.limiter.acquire().await;
        let url = format!(
            "{POSTS}?categories={APOD_CATEGORY}&per_page={PER_PAGE}&page={page}&orderby=date&order=desc&_fields=date,link,title,featured_image_url"
        );
        let posts: Vec<Post> = match get_json(&self.client, &url).await {
            Ok(posts) => posts,
            // Asking past the last page is how the end looks when the last page was exactly full.
            Err(e) if page > 1 && format!("{e:#}").contains("rest_post_invalid_page_number") => {
                return Ok(SearchPage { page, last_page: Some(page), ..Default::default() });
            }
            Err(e) => return Err(e),
        };
        if page == 1 && posts.is_empty() {
            bail!("science.nasa.gov listed no APOD posts; the category may have changed");
        }
        let last_page = (posts.len() < PER_PAGE).then_some(page);
        let items = posts.into_iter().filter_map(to_wallpaper).collect();
        Ok(SearchPage { items, page, last_page, total: None, seed: None })
    }

    /// `id` is the date. Adds the credit and explanation to what the listing has.
    async fn details(&self, id: &str) -> Result<Wallpaper> {
        let date: NaiveDate = id.parse().with_context(|| format!("not a date: {id}"))?;
        self.limiter.acquire().await;
        let day_before = date.pred_opt().context("date out of range")?;
        let url = format!(
            "{POSTS}?categories={APOD_CATEGORY}&after={day_before}T23:59:59&before={date}T23:59:59&per_page=1&_fields=date,link,title,featured_image_url,content"
        );
        let posts: Vec<Post> = get_json(&self.client, &url).await?;
        posts.into_iter().find_map(to_wallpaper).with_context(|| format!("no APOD picture for {id}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(date: &str, title: &str, image: Option<&str>) -> Post {
        Post {
            date: format!("{date}T00:05:00"),
            link: format!("https://science.nasa.gov/image-article/apod-{date}/"),
            title: Rendered { rendered: title.into() },
            featured_image_url: image.map(str::to_string),
            content: None,
        }
    }

    const IMAGE: &str = "https://assets.science.nasa.gov/dynamicimage/assets/science/cds/apod/apod/2026/november/ic342.png?w=4144&h=2822&fit=clip&crop=faces%2Cfocalpoint";

    #[test]
    fn a_post_becomes_a_wallpaper() {
        let wp = to_wallpaper(post("2026-11-04", "APOD: 2026 November 4 &#8211; The Not-So-Hidden Galaxy, IC 342", Some(IMAGE))).unwrap();
        assert_eq!(wp.key(), "nasa:2026-11-04", "the same key as before the move");
        assert_eq!(wp.title.as_deref(), Some("The Not-So-Hidden Galaxy, IC 342"));
        assert_eq!((wp.width, wp.height), (4144, 2822));
        assert_eq!(wp.full_url, "https://assets.science.nasa.gov/dynamicimage/assets/science/cds/apod/apod/2026/november/ic342.png", "the original");
        assert!(wp.thumb_url.ends_with("ic342.png?w=640&h=640&fit=clip"));
        assert_eq!(wp.extension(), "png", "known from the type, as the URLs may have a query");
    }

    #[test]
    fn placeholders_and_logos_are_not_wallpapers() {
        for image in [
            "https://science.nasa.gov/wp-content/themes/nasa-child/assets/images/nasa-logo@2x.png",
            "https://assets.science.nasa.gov/dynamicimage/assets/science/astro/programs/cosmic-origins/images/misc/news-thumbnail.png?w=594&h=516&fit=clip",
            "https://assets.science.nasa.gov/content/dam/science/cds/apod/apod/2026/october/occultation.mp4",
        ] {
            assert!(to_wallpaper(post("2026-10-28", "APOD: x", Some(image))).is_none(), "{image}");
        }
        assert!(to_wallpaper(post("2026-10-28", "APOD: x", None)).is_none());
    }

    #[test]
    fn titles_in_every_form_seen() {
        for (raw, want) in [
            ("APOD: 2026 November 3 &#8211; Comet PanSTARRs and Rainbow Meteor", "Comet PanSTARRs and Rainbow Meteor"),
            ("APOD: 2026 November 1 \u{2013} Colors of the Moon", "Colors of the Moon"),
            ("APOD: 29 October 2026 &#8211; The Horns of Venus", "The Horns of Venus"),
            ("APOD: 2026 September 5 \u{2013} Chasing the Moon&#8217;s Shadow", "Chasing the Moon\u{2019}s Shadow"),
            ("APOD: 2020 May 1 &#8211; Arcs &amp; Rays \u{2013} Part 2", "Arcs & Rays \u{2013} Part 2"),
            ("Something Else", "Something Else"),
        ] {
            assert_eq!(clean_title(raw), want);
        }
    }

    #[test]
    fn entities_that_are_not_entities_survive() {
        assert_eq!(decode_entities("Q&A; R&D &#38; 5 &lt; 6 &bogus; &"), "Q&A; R&D & 5 < 6 &bogus; &");
    }

    #[test]
    fn credit_and_explanation_come_from_the_page() {
        let page = r#"<img alt="See Explanation. Clicking"/>
            <p class="p-md margin-0 media-detail-hero__description">
                <strong>Explanation:</strong> &#8220;I see&#8221; <a href="https://x/">IC 342</a>,
                the   Hidden Galaxy! Tomorrow's picture: open space
            </p>
            <tr class="media-detail-hero__meta-row"><th scope="row" class="label padding-y-1">Date</th><td>2026</td></tr>
            <tr class="media-detail-hero__meta-row"><th scope="row" class="label padding-y-1">Credit &amp; Copyright:</th>
                <td class="p-sm padding-y-1">
                    <a href="https://www.instagram.com/artofomin/" rel="noopener">Artem Mironov</a>
                </td></tr>"#;
        let mut wp = Wallpaper::default();
        add_page_details(&mut wp, page);
        assert_eq!(wp.extra["description"], "\u{201c}I see\u{201d} IC 342, the Hidden Galaxy!");
        assert_eq!(wp.author.as_deref(), Some("Artem Mironov"));

        // How older posts have it: the label in the cell.
        let mut wp = Wallpaper::default();
        add_page_details(&mut wp, r#"<th scope="row">Credit</th><td class="p-sm"> Credit &amp; Copyright:  Luc Perrot </td>"#);
        assert_eq!(wp.author.as_deref(), Some("Luc Perrot"));
        // Nothing to find: nothing added, nothing breaks.
        let mut wp = Wallpaper::default();
        add_page_details(&mut wp, "<p>Credit</p>");
        assert_eq!(wp.author, None);
        assert!(wp.extra.is_empty());
    }

    /// Against the real site: `cargo test live_apod -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_apod() {
        let nasa = Nasa::new(crate::net::client());
        let page = nasa.search(&SearchQuery::default(), 1).await.unwrap();
        assert!(page.items.len() >= 25, "{} items", page.items.len());
        let distinct: std::collections::HashSet<_> = page.items.iter().map(|w| &w.thumb_url).collect();
        assert_eq!(distinct.len(), page.items.len(), "every day has its own picture");
        assert!(page.items.iter().all(|w| !w.thumb_url.contains("logo") && !w.display_title().contains("NASA Science")));
        let first = &page.items[0];
        println!("{} {} {}", first.key(), first.display_title(), first.resolution());

        let bytes = crate::net::get_bytes(&crate::net::client(), &first.thumb_url).await.unwrap();
        let thumb = image::load_from_memory(&bytes).unwrap();
        println!("thumbnail {}x{}, {} KB", thumb.width(), thumb.height(), bytes.len() / 1024);
        assert_eq!(thumb.width().max(thumb.height()), THUMB_SIZE, "scaled as requested");

        let details = nasa.details(&first.remote_id).await.unwrap();
        assert_eq!(details.key(), first.key());
        assert_eq!(details.full_url, first.full_url);
        println!("credit: {:?}\nexplanation: {:?}", details.author, details.extra.get("description"));
        assert!(details.extra.get("description").is_some_and(|d| d.len() > 50));
        assert!(details.author.is_some());

        // Deep in the archive, and past its end.
        let old = nasa.search(&SearchQuery::default(), 200).await.unwrap();
        assert!(old.items.len() >= 25, "{} items on page 200", old.items.len());
        println!("page 200: {} ({})", old.items[0].key(), old.items[0].display_title());
        let end = nasa.search(&SearchQuery::default(), 99_999).await.unwrap();
        assert!(end.items.is_empty() && !end.has_more());
    }
}
