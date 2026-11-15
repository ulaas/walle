use std::{path::Path, process::Command};

use anyhow::{Context, Result, bail};

use crate::model::FitMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Gnome,
    Cinnamon,
    Mate,
    Kde,
    Xfce,
    Lxqt,
    Sway,
    Hyprland,
    Feh,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Backend::Gnome => "GNOME (gsettings)",
            Backend::Cinnamon => "Cinnamon (gsettings)",
            Backend::Mate => "MATE (gsettings)",
            Backend::Kde => "KDE Plasma (plasma-apply-wallpaperimage)",
            Backend::Xfce => "XFCE (xfconf-query)",
            Backend::Lxqt => "LXQt (pcmanfm-qt)",
            Backend::Sway => "Sway (swaymsg)",
            Backend::Hyprland => "Hyprland (swww / hyprpaper)",
            Backend::Feh => "feh (fallback)",
        }
    }
}

pub fn detect() -> Backend {
    let de = std::env::var("XDG_CURRENT_DESKTOP")
        .or_else(|_| std::env::var("DESKTOP_SESSION"))
        .unwrap_or_default()
        .to_lowercase();
    if std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok() || de.contains("hyprland") {
        Backend::Hyprland
    } else if std::env::var("SWAYSOCK").is_ok() || de.contains("sway") {
        Backend::Sway
    } else if de.contains("kde") || de.contains("plasma") {
        Backend::Kde
    } else if de.contains("xfce") {
        Backend::Xfce
    } else if de.contains("cinnamon") {
        Backend::Cinnamon
    } else if de.contains("mate") {
        Backend::Mate
    } else if de.contains("lxqt") {
        Backend::Lxqt
    } else if ["gnome", "unity", "ubuntu", "budgie", "pantheon", "cosmic"].iter().any(|d| de.contains(d)) {
        Backend::Gnome
    } else {
        Backend::Feh
    }
}

fn run(cmd: &str, args: &[&str]) -> Result<()> {
    let out = Command::new(cmd).args(args).output().with_context(|| format!("running {cmd}"))?;
    if !out.status.success() {
        bail!("{cmd} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

fn run_output(cmd: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(cmd).args(args).output().with_context(|| format!("running {cmd}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn set(path: &Path, fit: FitMode, custom_cmd: &str) -> Result<()> {
    let p = path.to_string_lossy().to_string();
    if !custom_cmd.trim().is_empty() {
        let cmd = custom_cmd.replace("{path}", &shell_quote(&p));
        return run("sh", &["-c", &cmd]);
    }
    let uri = format!("file://{p}");
    match detect() {
        Backend::Gnome => {
            let opt = match fit {
                FitMode::Fill => "zoom",
                FitMode::Fit => "scaled",
                FitMode::Stretch => "stretched",
                FitMode::Center => "centered",
                FitMode::Tile => "wallpaper",
                FitMode::Span => "spanned",
            };
            run("gsettings", &["set", "org.gnome.desktop.background", "picture-options", opt])?;
            run("gsettings", &["set", "org.gnome.desktop.background", "picture-uri", &uri])?;
            // Older GNOME has no dark key; ignore failure.
            let _ = run("gsettings", &["set", "org.gnome.desktop.background", "picture-uri-dark", &uri]);
            Ok(())
        }
        Backend::Cinnamon => run("gsettings", &["set", "org.cinnamon.desktop.background", "picture-uri", &uri]),
        Backend::Mate => run("gsettings", &["set", "org.mate.background", "picture-filename", &p]),
        Backend::Kde => run("plasma-apply-wallpaperimage", &[&p]),
        Backend::Xfce => {
            let props = run_output("xfconf-query", &["-c", "xfce4-desktop", "-l"])?;
            let style = match fit {
                FitMode::Center => "1",
                FitMode::Tile => "2",
                FitMode::Stretch => "3",
                FitMode::Fit => "4",
                FitMode::Fill => "5",
                FitMode::Span => "6",
            };
            let mut any = false;
            for prop in props.lines().filter(|l| l.ends_with("/last-image")) {
                run("xfconf-query", &["-c", "xfce4-desktop", "-p", prop, "-s", &p])?;
                let sp = prop.replace("/last-image", "/image-style");
                let _ = run("xfconf-query", &["-c", "xfce4-desktop", "-p", &sp, "-s", style]);
                any = true;
            }
            if !any {
                bail!("no xfce4-desktop last-image properties found");
            }
            Ok(())
        }
        Backend::Lxqt => {
            let mode = match fit {
                FitMode::Fill => "zoom",
                FitMode::Fit => "fit",
                FitMode::Stretch => "stretch",
                FitMode::Center => "center",
                FitMode::Tile => "tile",
                FitMode::Span => "zoom",
            };
            run("pcmanfm-qt", &["--set-wallpaper", &p, &format!("--wallpaper-mode={mode}")])
        }
        Backend::Sway => {
            let mode = match fit {
                FitMode::Fill | FitMode::Span => "fill",
                FitMode::Fit => "fit",
                FitMode::Stretch => "stretch",
                FitMode::Center => "center",
                FitMode::Tile => "tile",
            };
            run("swaymsg", &["output", "*", "bg", &p, mode])
        }
        Backend::Hyprland => {
            if run("swww", &["img", &p]).is_ok() {
                return Ok(());
            }
            run("hyprctl", &["hyprpaper", "preload", &p])?;
            run("hyprctl", &["hyprpaper", "wallpaper", &format!(",{p}")])
        }
        Backend::Feh => {
            let mode = match fit {
                FitMode::Fill | FitMode::Span => "--bg-fill",
                FitMode::Fit => "--bg-max",
                FitMode::Stretch => "--bg-scale",
                FitMode::Center => "--bg-center",
                FitMode::Tile => "--bg-tile",
            };
            run("feh", &[mode, &p])
        }
    }
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Path of the current wallpaper where the desktop exposes it.
pub fn current() -> Option<std::path::PathBuf> {
    let read = |schema: &str, key: &str| {
        let out = Command::new("gsettings").args(["get", schema, key]).output().ok()?;
        let s = String::from_utf8_lossy(&out.stdout).trim().trim_matches('\'').to_string();
        let s = s.strip_prefix("file://").unwrap_or(&s).to_string();
        (!s.is_empty()).then(|| std::path::PathBuf::from(s))
    };
    match detect() {
        Backend::Gnome => read("org.gnome.desktop.background", "picture-uri"),
        Backend::Cinnamon => read("org.cinnamon.desktop.background", "picture-uri"),
        Backend::Mate => read("org.mate.background", "picture-filename"),
        _ => None,
    }
}
