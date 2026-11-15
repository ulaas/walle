//! macOS: the desktop picture of each screen, through NSWorkspace. No Apple Events, so no
//! Automation prompt. Like the Desktop pane of System Settings, it changes the current Space.
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use objc2::{rc::Retained, runtime::AnyObject};
use objc2_app_kit::{
    NSImageScaling, NSScreen, NSWorkspace, NSWorkspaceDesktopImageAllowClippingKey, NSWorkspaceDesktopImageOptionKey,
    NSWorkspaceDesktopImageScalingKey,
};
use objc2_foundation::{MainThreadMarker, NSDictionary, NSNumber, NSURL};

use super::Placement;
use crate::{
    desktop::{Display, Target},
    model::FitMode,
};

/// AppKit wants screens asked for on the main thread. walle sets wallpapers from its worker,
/// so the main thread (whose run loop serves the main queue) does it and the worker waits.
fn on_main<R: Send + 'static>(f: impl FnOnce(MainThreadMarker) -> R + Send + 'static) -> Result<R> {
    if let Some(mtm) = MainThreadMarker::new() {
        return Ok(f(mtm));
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    dispatch2::DispatchQueue::main().exec_async(move || {
        let mtm = MainThreadMarker::new().expect("the main queue runs on the main thread");
        let _ = tx.send(f(mtm));
    });
    rx.recv_timeout(Duration::from_secs(10)).map_err(|_| anyhow!("the main thread did not answer"))
}

/// The screens with the names walle shows for them: the screen's own name, numbered where
/// two are alike.
fn named_screens(mtm: MainThreadMarker) -> Vec<(String, Retained<NSScreen>)> {
    let mut out: Vec<(String, Retained<NSScreen>)> = Vec::new();
    for screen in NSScreen::screens(mtm).iter() {
        let base = screen.localizedName().to_string();
        let mut name = base.clone();
        let mut n = 2;
        while out.iter().any(|(other, _)| *other == name) {
            name = format!("{base} ({n})");
            n += 1;
        }
        out.push((name, screen));
    }
    out
}

/// The screens, in the top-left based coordinates the rest of walle uses. AppKit counts from
/// the bottom left of the main screen, upwards.
pub fn displays() -> Result<Vec<Display>> {
    on_main(|mtm| {
        let screens = named_screens(mtm);
        let main_height = screens.first().map_or(0.0, |(_, s)| s.frame().size.height);
        screens
            .iter()
            .map(|(name, s)| {
                let f = s.frame();
                Display {
                    name: name.clone(),
                    x: f.origin.x.round() as i32,
                    y: (main_height - f.origin.y - f.size.height).round() as i32,
                    width: f.size.width.round().max(1.0) as u32,
                    height: f.size.height.round().max(1.0) as u32,
                    scale: s.backingScaleFactor() as f32,
                }
            })
            .collect()
    })
}

/// How each fit mode maps onto the desktop picture's options: (scaling, allow clipping).
/// The API has no tiling, so a tiled picture is centred; a span is already cut per screen.
fn options(fit: FitMode) -> (NSImageScaling, bool) {
    match fit {
        FitMode::Fill | FitMode::Span => (NSImageScaling::ScaleProportionallyUpOrDown, true),
        FitMode::Fit => (NSImageScaling::ScaleProportionallyUpOrDown, false),
        FitMode::Stretch => (NSImageScaling::ScaleAxesIndependently, true),
        FitMode::Center | FitMode::Tile => (NSImageScaling::ScaleNone, true),
    }
}

fn file_url(path: &Path) -> Result<Retained<NSURL>> {
    NSURL::from_file_path(path).with_context(|| format!("{} is not a file path", path.display()))
}

/// A picture that is not the wallpaper, to step through when the wallpaper's file was
/// rewritten under the same name: the desktop keeps showing what it cached for that name
/// otherwise.
fn interim_picture() -> Result<PathBuf> {
    let path = std::env::temp_dir().join("walle-interim.png");
    if !path.is_file() {
        image::RgbImage::from_pixel(1, 1, image::Rgb([0, 0, 0])).save(&path).context("writing the interim picture")?;
    }
    Ok(path)
}

fn set_on(workspace: &NSWorkspace, screen: &NSScreen, path: &Path, fit: FitMode) -> Result<()> {
    let url = file_url(path)?;
    let (scaling, clip) = options(fit);
    let scaling = NSNumber::new_usize(scaling.0);
    let clip = NSNumber::new_bool(clip);
    let keys: [&NSWorkspaceDesktopImageOptionKey; 2] =
        unsafe { [NSWorkspaceDesktopImageScalingKey, NSWorkspaceDesktopImageAllowClippingKey] };
    let values: [&AnyObject; 2] = [&scaling, &clip];
    let opts = NSDictionary::from_slices(&keys, &values);
    if workspace.desktopImageURLForScreen(screen).is_some_and(|now| now == url) {
        let interim = file_url(&interim_picture()?)?;
        let none = NSDictionary::from_slices::<NSWorkspaceDesktopImageOptionKey>(&[], &[]);
        // SAFETY: the options dictionary maps option keys to objects, as the method expects.
        let _ = unsafe { workspace.setDesktopImageURL_forScreen_options_error(&interim, screen, &none) };
    }
    // SAFETY: as above; the values are the NSNumbers the documented keys take.
    unsafe { workspace.setDesktopImageURL_forScreen_options_error(&url, screen, &opts) }
        .map_err(|e| anyhow!("{}", e.localizedDescription()))
}

/// Put each picture on its screen, or on every screen for `Target::All`, which comes first.
pub fn apply(placements: &[Placement]) -> Result<()> {
    let placements = placements.to_vec();
    on_main(move |mtm| {
        let workspace = NSWorkspace::sharedWorkspace();
        let screens = named_screens(mtm);
        for p in &placements {
            match &p.target {
                Target::All => {
                    for (name, screen) in &screens {
                        set_on(&workspace, screen, &p.path, p.fit).with_context(|| format!("on {name}"))?;
                    }
                }
                Target::Display(name) => {
                    let Some((_, screen)) = screens.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) else {
                        bail!("display {name} is not connected");
                    };
                    set_on(&workspace, screen, &p.path, p.fit)?;
                }
            }
        }
        Ok(())
    })?
}

/// The pictures on the screens now, one per screen that shows a file.
pub fn current() -> Vec<PathBuf> {
    let found = on_main(|mtm| {
        let workspace = NSWorkspace::sharedWorkspace();
        let mut paths: Vec<PathBuf> = Vec::new();
        for screen in NSScreen::screens(mtm).iter() {
            if let Some(path) = workspace.desktopImageURLForScreen(&screen).and_then(|u| u.to_file_path())
                && !paths.contains(&path)
            {
                paths.push(path);
            }
        }
        paths
    });
    found.unwrap_or_else(|e| {
        tracing::warn!("reading the desktop picture: {e:#}");
        vec![]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_modes_map_to_the_desktop_options() {
        assert_eq!(options(FitMode::Fill), (NSImageScaling::ScaleProportionallyUpOrDown, true));
        assert_eq!(options(FitMode::Fit), (NSImageScaling::ScaleProportionallyUpOrDown, false));
        assert_eq!(options(FitMode::Stretch).0, NSImageScaling::ScaleAxesIndependently);
        assert_eq!(options(FitMode::Center).0, NSImageScaling::ScaleNone);
        assert_eq!(options(FitMode::Tile), options(FitMode::Center));
        assert_eq!(options(FitMode::Span), options(FitMode::Fill));
    }
}
