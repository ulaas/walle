use serde::{Deserialize, Serialize};

/// A wallpaper as described by a provider. Stored as JSON in the library.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Wallpaper {
    pub provider: String,
    pub remote_id: String,
    /// Human-facing page on the provider site.
    pub page_url: Option<String>,
    /// Full resolution image URL.
    pub full_url: String,
    /// Small thumbnail for the grid.
    pub thumb_url: String,
    /// Larger, aspect-preserving preview (optional).
    pub preview_url: Option<String>,
    pub width: u32,
    pub height: u32,
    pub file_size: Option<u64>,
    pub file_type: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub author_url: Option<String>,
    pub source: Option<String>,
    pub license: Option<String>,
    pub purity: Option<String>,
    pub category: Option<String>,
    pub colors: Vec<String>,
    /// Tags assigned by the provider (not the user's own tags).
    pub tags: Vec<String>,
    pub created_at: Option<String>,
    /// Provider-specific extras (e.g. Unsplash download-tracking URL).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub extra: std::collections::BTreeMap<String, String>,
}

impl Wallpaper {
    pub fn key(&self) -> String {
        make_key(&self.provider, &self.remote_id)
    }

    /// `key() == key`, without allocating (used when scanning long lists).
    pub fn has_key(&self, key: &str) -> bool {
        key.strip_prefix(self.provider.as_str()).and_then(|r| r.strip_prefix(':')) == Some(self.remote_id.as_str())
    }

    pub fn resolution(&self) -> String {
        if self.width == 0 || self.height == 0 {
            "?".into()
        } else {
            format!("{}×{}", self.width, self.height)
        }
    }

    pub fn display_title(&self) -> String {
        self.title.clone().unwrap_or_else(|| format!("{} {}", self.provider, self.remote_id))
    }

    /// File extension for the full image, guessed from mime type or URL.
    pub fn extension(&self) -> &'static str {
        let t = self.file_type.as_deref().unwrap_or("");
        let u = self.full_url.to_ascii_lowercase();
        if t.contains("png") || u.ends_with(".png") {
            "png"
        } else if t.contains("webp") || u.ends_with(".webp") {
            "webp"
        } else {
            "jpg"
        }
    }
}

pub fn make_key(provider: &str, id: &str) -> String {
    format!("{provider}:{id}")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Sorting {
    Relevance,
    DateAdded,
    Random,
    Views,
    Favorites,
    Toplist,
    Hot,
    Latest,
    Popular,
}

impl Sorting {
    pub fn label(self) -> &'static str {
        match self {
            Sorting::Relevance => "Relevance",
            Sorting::DateAdded => "Date added",
            Sorting::Random => "Random",
            Sorting::Views => "Views",
            Sorting::Favorites => "Favorites",
            Sorting::Toplist => "Toplist",
            Sorting::Hot => "Hot",
            Sorting::Latest => "Latest",
            Sorting::Popular => "Popular",
        }
    }
}

/// Generic search request; providers use the parts they support (see `ProviderCaps`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SearchQuery {
    pub text: String,
    pub sorting: Sorting,
    pub descending: bool,
    pub toplist_range: String,
    /// general, anime, people
    pub categories: [bool; 3],
    /// sfw, sketchy, nsfw
    pub purity: [bool; 3],
    pub min_resolution: Option<(u32, u32)>,
    pub ratios: Vec<String>,
    pub color: Option<String>,
    #[serde(default)]
    pub orientation: Orientation,
    /// Browse a provider collection (e.g. a Wallhaven collection id) instead of searching.
    #[serde(default)]
    pub collection: Option<String>,
    /// Seed for stable random paging (wallhaven).
    #[serde(skip)]
    pub seed: Option<String>,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            text: String::new(),
            sorting: Sorting::Toplist,
            descending: true,
            toplist_range: "1M".into(),
            categories: [true, true, false],
            purity: [true, false, false],
            min_resolution: None,
            ratios: vec![],
            color: None,
            orientation: Orientation::Any,
            collection: None,
            seed: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SearchPage {
    pub items: Vec<Wallpaper>,
    pub page: u32,
    pub last_page: Option<u32>,
    pub total: Option<u64>,
    pub seed: Option<String>,
}

impl SearchPage {
    pub fn has_more(&self) -> bool {
        match self.last_page {
            Some(last) => self.page < last,
            None => !self.items.is_empty(),
        }
    }
}

/// Curation state of a wallpaper in the user's library.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Status {
    #[default]
    None,
    Considering,
    Kept,
    Rejected,
}

impl Status {
    pub fn to_i64(self) -> i64 {
        match self {
            Status::None => 0,
            Status::Considering => 1,
            Status::Kept => 2,
            Status::Rejected => 3,
        }
    }
    pub fn from_i64(v: i64) -> Self {
        match v {
            1 => Status::Considering,
            2 => Status::Kept,
            3 => Status::Rejected,
            _ => Status::None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Status::None => "—",
            Status::Considering => "Considering",
            Status::Kept => "Kept",
            Status::Rejected => "Rejected",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FitMode {
    #[default]
    Fill,
    Fit,
    Stretch,
    Center,
    Tile,
    Span,
}

impl FitMode {
    pub const ALL: [FitMode; 6] = [
        FitMode::Fill,
        FitMode::Fit,
        FitMode::Stretch,
        FitMode::Center,
        FitMode::Tile,
        FitMode::Span,
    ];
    pub fn label(self) -> &'static str {
        match self {
            FitMode::Fill => "Fill",
            FitMode::Fit => "Fit",
            FitMode::Stretch => "Stretch",
            FitMode::Center => "Center",
            FitMode::Tile => "Tile",
            FitMode::Span => "Span",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Orientation {
    #[default]
    Any,
    Landscape,
    Portrait,
}

impl Orientation {
    pub const ALL: [Orientation; 3] = [Orientation::Any, Orientation::Landscape, Orientation::Portrait];
    pub fn label(self) -> &'static str {
        match self {
            Orientation::Any => "Any",
            Orientation::Landscape => "Landscape",
            Orientation::Portrait => "Portrait",
        }
    }
}

impl Wallpaper {
    /// True when the image lives on the local file system (local-folder provider).
    pub fn is_local(&self) -> bool {
        !self.full_url.starts_with("http://") && !self.full_url.starts_with("https://")
    }
}
