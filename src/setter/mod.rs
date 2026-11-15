//! Set the desktop wallpaper.
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::{
    desktop::{Display, Target},
    model::FitMode,
};

#[cfg(target_os = "linux")]
mod gevrek;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// One picture and where it goes.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub target: Target,
    pub path: PathBuf,
    pub fit: FitMode,
}

/// gevrek's own interface is used (Linux, in a gevrek session, unless a custom command
/// overrides detection).
#[cfg(target_os = "linux")]
fn on_gevrek(custom_cmd: &str) -> bool {
    custom_cmd.trim().is_empty() && gevrek::detected()
}

/// The displays that can each have a wallpaper of their own. Empty where the desktop only
/// takes one wallpaper for everything (then `Target::All` is the only target).
#[cfg_attr(test, allow(dead_code))] // tests pretend the desktop (see worker)
pub fn displays(custom_cmd: &str) -> Vec<Display> {
    #[cfg(target_os = "linux")]
    if on_gevrek(custom_cmd) {
        return gevrek::displays().unwrap_or_else(|e| {
            tracing::warn!("asking gevrek for its displays: {e:#}");
            vec![]
        });
    }
    let _ = custom_cmd;
    #[cfg(target_os = "macos")]
    return macos::displays().unwrap_or_else(|e| {
        tracing::warn!("asking for the screens: {e:#}");
        vec![]
    });
    #[allow(unreachable_code)]
    {
        vec![]
    }
}

/// Put the pictures on the desktop, all in one change where the desktop allows it.
#[cfg_attr(test, allow(dead_code))]
pub fn apply(placements: &[Placement], custom_cmd: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    if on_gevrek(custom_cmd) {
        let absolute: Vec<Placement> = placements
            .iter()
            .map(|p| Ok(Placement { path: std::fs::canonicalize(&p.path)?, ..p.clone() }))
            .collect::<Result<_>>()?;
        return gevrek::apply(&absolute);
    }
    #[cfg(target_os = "macos")]
    {
        let _ = custom_cmd;
        let absolute: Vec<Placement> = placements
            .iter()
            .map(|p| Ok(Placement { path: std::fs::canonicalize(&p.path)?, ..p.clone() }))
            .collect::<Result<_>>()?;
        return macos::apply(&absolute);
    }
    #[allow(unreachable_code)]
    for p in placements {
        if let Target::Display(name) = &p.target {
            anyhow::bail!("this desktop takes one wallpaper for all displays (asked for {name})");
        }
        set_wallpaper(&p.path, p.fit, custom_cmd)?;
    }
    Ok(())
}

/// Set `path` as the wallpaper of the whole desktop. `custom_cmd` (Linux only) overrides
/// detection; `{path}` in it is replaced with the image path.
pub fn set_wallpaper(path: &Path, fit: FitMode, custom_cmd: &str) -> Result<()> {
    let path = std::fs::canonicalize(path)?;
    #[cfg(windows)]
    {
        let _ = custom_cmd;
        return windows::set(&path, fit);
    }
    #[cfg(target_os = "linux")]
    {
        if on_gevrek(custom_cmd) {
            return gevrek::apply(&[Placement { target: Target::All, path, fit }]);
        }
        return linux::set(&path, fit, custom_cmd);
    }
    #[cfg(target_os = "macos")]
    {
        let _ = custom_cmd;
        return macos::apply(&[Placement { target: Target::All, path, fit }]);
    }
    #[allow(unreachable_code)]
    {
        let _ = (path, fit, custom_cmd);
        anyhow::bail!("setting wallpapers is not supported on this platform yet")
    }
}

/// Human-readable description of the detected backend (for the settings page).
pub fn backend_name() -> String {
    #[cfg(windows)]
    return "Windows (SystemParametersInfo)".into();
    #[cfg(target_os = "linux")]
    return if gevrek::detected() { "gevrek (its own wallpaper, per display)".into() } else { linux::detect().name().into() };
    #[cfg(target_os = "macos")]
    return "macOS (NSWorkspace, per display)".into();
    #[allow(unreachable_code)]
    "unsupported".into()
}

/// The wallpapers currently on the desktop (where the platform can tell): one, or on a
/// desktop with a wallpaper per display, each of them.
pub fn current_wallpapers() -> Vec<PathBuf> {
    #[cfg(windows)]
    return windows::current().into_iter().collect();
    #[cfg(target_os = "linux")]
    return if gevrek::detected() { gevrek::current() } else { linux::current().into_iter().collect() };
    #[cfg(target_os = "macos")]
    return macos::current();
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    vec![]
}

/// The wallpaper currently set on the desktop (the first, where there are several).
#[cfg(test)]
pub fn current_wallpaper() -> Option<PathBuf> {
    current_wallpapers().into_iter().next()
}

#[cfg(test)]
mod tests {
    /// Sets a generated image, checks the desktop reports it, then restores the previous
    /// wallpaper. Changes the real desktop briefly, so it only runs on request:
    /// `cargo test setter -- --ignored`
    #[test]
    #[ignore]
    fn set_and_restore() {
        let before = super::current_wallpaper();
        let dir = std::env::temp_dir().join(format!("walle-setter-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = dir.join("walle-test.png");
        image::RgbImage::from_pixel(64, 36, image::Rgb([40, 80, 160])).save(&img).unwrap();
        super::set_wallpaper(&img, crate::model::FitMode::Fill, "").expect("set");
        let now = super::current_wallpaper().expect("current wallpaper readable");
        let restore = before.clone();
        if let Some(b) = &restore {
            let _ = super::set_wallpaper(b, crate::model::FitMode::Fill, "");
        }
        assert_eq!(std::fs::canonicalize(&now).unwrap(), std::fs::canonicalize(&img).unwrap());
        if before.as_ref().is_some_and(|b| b.exists()) {
            assert_eq!(super::current_wallpaper(), before, "previous wallpaper restored");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
