//! The displays of the desktop, and what walle last put on each of them.
use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{config::Paths, model::FitMode};

/// One display, as the desktop reports it. Position and size are in the desktop's logical
/// pixels; `scale` turns them into the pixels of the panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Display {
    /// The name the desktop knows it by (`DP-2`).
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

impl Display {
    /// Size in the panel's own pixels: what a wallpaper is rendered at.
    pub fn pixels(&self) -> (u32, u32) {
        let s = if self.scale.is_finite() && self.scale > 0.0 { self.scale } else { 1.0 };
        (((self.width as f32) * s).round().max(1.0) as u32, ((self.height as f32) * s).round().max(1.0) as u32)
    }
}

/// Where a wallpaper goes.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Target {
    /// Every display (and the only target on desktops without per-display wallpapers).
    #[default]
    All,
    /// The display with this name.
    Display(String),
}

/// A wallpaper walle put on the desktop: enough to render it again with other effects.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Library key (`provider:id`).
    pub key: String,
    /// The original image, before effects.
    pub source: PathBuf,
    pub fit: FitMode,
    /// The files handed to the desktop: the original, or the copies rendered from it.
    #[serde(default)]
    pub placed: Vec<PathBuf>,
}

/// What walle last put on the desktop. Saved beside the library, so that after a restart walle
/// knows what it is looking at and can take over again.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shown {
    /// The wallpaper of every display without one of its own.
    pub all: Option<Entry>,
    /// Displays with a wallpaper of their own, by name.
    pub displays: BTreeMap<String, Entry>,
    /// The night level (see `effects::Night`) the pictures were rendered at.
    pub night_level: u8,
}

impl Shown {
    fn file(paths: &Paths) -> PathBuf {
        paths.data_dir.join("desktop.json")
    }

    pub fn load(paths: &Paths) -> Self {
        std::fs::read_to_string(Self::file(paths)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, paths: &Paths) {
        match serde_json::to_string_pretty(self) {
            Ok(s) => {
                // Written beside and renamed: never half a file, whatever ends walle.
                let (file, tmp) = (Self::file(paths), Self::file(paths).with_extension("json.part"));
                if let Err(e) = std::fs::write(&tmp, s).and_then(|()| std::fs::rename(&tmp, &file)) {
                    tracing::warn!("saving desktop state: {e}");
                }
            }
            Err(e) => tracing::warn!("saving desktop state: {e}"),
        }
    }

    /// Setting every display replaces what single displays had.
    pub fn record(&mut self, target: &Target, entry: Entry) {
        match target {
            Target::All => {
                self.all = Some(entry);
                self.displays.clear();
            }
            Target::Display(name) => {
                self.displays.insert(name.clone(), entry);
            }
        }
    }

    /// What the display with this name shows.
    pub fn on(&self, display: &str) -> Option<&Entry> {
        self.displays.get(display).or(self.all.as_ref())
    }

    /// Keys of everything on the desktop: the common wallpaper only while a display still
    /// shows it.
    pub fn keys(&self, displays: &[Display]) -> Vec<String> {
        let mut keys: Vec<String> = Vec::new();
        let mut add = |e: &Entry| {
            if !keys.contains(&e.key) {
                keys.push(e.key.clone());
            }
        };
        if displays.is_empty() {
            self.all.iter().for_each(&mut add);
            self.displays.values().for_each(&mut add);
        } else {
            displays.iter().filter_map(|d| self.on(&d.name)).for_each(&mut add);
        }
        keys
    }

    pub fn is_empty(&self) -> bool {
        self.all.is_none() && self.displays.is_empty()
    }

    /// Forget what is no longer on the desktop (someone else changed the wallpaper).
    /// `current` is what the desktop shows now; empty when it cannot tell, and then
    /// everything is believed.
    pub fn retain_current(&mut self, current: &[PathBuf]) {
        if current.is_empty() {
            return;
        }
        let there = |e: &Entry| e.placed.iter().any(|p| current.contains(p));
        if self.all.as_ref().is_some_and(|e| !there(e)) {
            self.all = None;
        }
        self.displays.retain(|_, e| there(e));
    }

    /// Every original that is on the desktop, and every file made from one.
    pub fn files(&self) -> Vec<PathBuf> {
        self.all.iter().chain(self.displays.values()).flat_map(|e| e.placed.iter().cloned().chain([e.source.clone()])).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str) -> Entry {
        let source = PathBuf::from(format!("/p/{key}.jpg"));
        Entry { key: key.into(), placed: vec![source.clone()], source, fit: FitMode::Fill }
    }

    fn display(name: &str) -> Display {
        Display { name: name.into(), x: 0, y: 0, width: 1920, height: 1080, scale: 1.0 }
    }

    #[test]
    fn one_display_then_all() {
        let mut shown = Shown::default();
        shown.record(&Target::All, entry("a"));
        shown.record(&Target::Display("DP-2".into()), entry("b"));
        assert_eq!(shown.on("DP-2").unwrap().key, "b");
        assert_eq!(shown.on("DP-3").unwrap().key, "a");
        let both = [display("DP-2"), display("DP-3")];
        assert_eq!(shown.keys(&both), vec!["b".to_string(), "a".to_string()]);
        // The common one is on no display once each has its own.
        shown.record(&Target::Display("DP-3".into()), entry("c"));
        assert_eq!(shown.keys(&both), vec!["b".to_string(), "c".to_string()]);
        shown.record(&Target::All, entry("d"));
        assert!(shown.displays.is_empty());
        assert_eq!(shown.keys(&both), vec!["d".to_string()]);
    }

    #[test]
    fn forgets_what_someone_else_replaced() {
        let mut shown = Shown::default();
        shown.record(&Target::All, entry("a"));
        shown.record(&Target::Display("DP-2".into()), entry("b"));
        // A desktop that cannot tell is believed.
        shown.retain_current(&[]);
        assert_eq!(shown.files().len(), 4);
        shown.retain_current(&[PathBuf::from("/p/b.jpg"), PathBuf::from("/elsewhere.png")]);
        assert!(shown.all.is_none());
        assert_eq!(shown.on("DP-2").unwrap().key, "b");
        assert!(shown.on("DP-3").is_none());
    }

    #[test]
    fn pixels_follow_the_scale() {
        let mut d = display("eDP-1");
        d.scale = 1.5;
        assert_eq!(d.pixels(), (2880, 1620));
        d.scale = 0.0;
        assert_eq!(d.pixels(), (1920, 1080));
    }
}
