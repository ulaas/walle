use std::{collections::BTreeMap, path::PathBuf};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::model::FitMode;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProviderSettings {
    pub enabled: bool,
    pub api_key: String,
    /// Free-form provider options (e.g. `market = "en-US"` for Bing).
    pub options: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub default_provider: String,
    pub fit_mode: FitMode,
    pub show_rejected: bool,
    pub thumb_size: f32,
    /// Linux: custom command to set the wallpaper; `{path}` is replaced.
    pub linux_set_command: String,
    /// Override for where full images are stored.
    pub download_dir: Option<PathBuf>,
    pub providers: BTreeMap<String, ProviderSettings>,
    /// Download the full image as soon as a wallpaper is kept.
    pub auto_download_kept: bool,
    /// Folders scanned by the "Local folders" provider.
    pub local_folders: Vec<PathBuf>,
    pub theme: Theme,
    /// Accent colour (RGB).
    pub accent: [u8; 3],
    pub rotation: RotationConfig,
    /// Visual effects rendered into the wallpaper on the desktop.
    pub effects: crate::effects::Effects,
    /// Closing the window keeps walle running in the system tray (where available).
    pub close_to_tray: bool,
    /// Limit for the thumbnail/preview cache in MB (oldest files are pruned).
    pub cache_limit_mb: u64,
    /// Hide browse results that look like wallpapers already in the library.
    pub hide_duplicates: bool,
    /// Ask GitHub once a day whether there is a newer walle.
    pub check_updates: bool,
}

/// Where automatic wallpaper rotation picks from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", content = "value")]
pub enum RotationSource {
    #[default]
    Kept,
    Considering,
    Tag(String),
    SavedSearch(i64),
    Provider(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RotationConfig {
    pub enabled: bool,
    pub interval_minutes: u32,
    pub source: RotationSource,
    pub landscape_only: bool,
    /// Library sources only.
    pub min_rating: u8,
    /// Change the wallpaper right after walle starts.
    pub change_on_start: bool,
    /// A different wallpaper on each display (where the desktop has a wallpaper per display).
    pub per_display: bool,
}

impl Default for RotationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_minutes: 60,
            source: RotationSource::Kept,
            landscape_only: true,
            min_rating: 0,
            change_on_start: false,
            per_display: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

/// Providers that are off until the user enables them.
pub fn default_enabled(id: &str) -> bool {
    !matches!(id, "openverse")
}

impl Default for Config {
    fn default() -> Self {
        let mut providers = BTreeMap::new();
        for id in ["wallhaven", "bing", "picsum", "nasa"] {
            providers.insert(
                id.to_string(),
                ProviderSettings { enabled: true, ..Default::default() },
            );
        }
        Self {
            default_provider: "wallhaven".into(),
            fit_mode: FitMode::Fill,
            show_rejected: false,
            thumb_size: 240.0,
            linux_set_command: String::new(),
            download_dir: None,
            providers,
            auto_download_kept: true,
            local_folders: vec![],
            theme: Theme::Dark,
            accent: crate::icon_art::DEFAULT_ACCENT,
            rotation: RotationConfig::default(),
            effects: Default::default(),
            close_to_tray: true,
            cache_limit_mb: 500,
            hide_duplicates: false,
            check_updates: true,
        }
    }
}

/// Env var that can supply a provider's API key.
pub fn key_env_var(provider: &str) -> String {
    format!("{}_API_KEY", provider.to_ascii_uppercase())
}

impl Config {
    pub fn provider(&self, id: &str) -> ProviderSettings {
        self.providers.get(id).cloned().unwrap_or(ProviderSettings {
            enabled: default_enabled(id),
            ..Default::default()
        })
    }

    /// API key from config, falling back to `<PROVIDER>_API_KEY` env var.
    pub fn api_key(&self, id: &str) -> Option<String> {
        let k = self.provider(id).api_key;
        if !k.trim().is_empty() {
            return Some(k.trim().to_string());
        }
        std::env::var(key_env_var(id)).ok().filter(|s| !s.trim().is_empty())
    }

    pub fn load(paths: &Paths) -> Self {
        let file = paths.config_file();
        match std::fs::read_to_string(&file) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!("config parse error in {}: {e}; using defaults", file.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        let file = paths.config_file();
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&file, toml::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", file.display()))
    }
}

#[derive(Clone, Debug)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl Paths {
    /// Set `WALLE_HOME` to run with all data in that folder, beside (not instead of) a normal
    /// instance: for trying a build without touching the real library.
    pub fn isolated_home() -> Option<PathBuf> {
        std::env::var_os("WALLE_HOME").filter(|h| !h.is_empty()).map(PathBuf::from)
    }

    pub fn is_isolated() -> bool {
        Self::isolated_home().is_some()
    }

    pub fn discover() -> Result<Self> {
        let p = match Self::isolated_home() {
            Some(home) => Self { config_dir: home.join("config"), data_dir: home.join("data"), cache_dir: home.join("cache") },
            None => {
                let dirs = ProjectDirs::from("", "", "walle").context("no home directory found")?;
                Self {
                    config_dir: dirs.config_dir().to_path_buf(),
                    data_dir: dirs.data_dir().to_path_buf(),
                    cache_dir: dirs.cache_dir().to_path_buf(),
                }
            }
        };
        for d in [&p.config_dir, &p.data_dir, &p.cache_dir, &p.thumbs_dir()] {
            std::fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
        }
        Ok(p)
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }
    pub fn db_file(&self) -> PathBuf {
        self.data_dir.join("library.db")
    }
    /// What walle remembers about updates between runs.
    pub fn update_file(&self) -> PathBuf {
        self.data_dir.join("update.json")
    }
    /// Where an update is downloaded to.
    pub fn update_dir(&self) -> PathBuf {
        self.cache_dir.join("update")
    }
    pub fn thumbs_dir(&self) -> PathBuf {
        self.cache_dir.join("thumbs")
    }
    pub fn wallpapers_dir(&self, cfg: &Config) -> PathBuf {
        cfg.download_dir.clone().unwrap_or_else(|| self.data_dir.join("wallpapers"))
    }
}
