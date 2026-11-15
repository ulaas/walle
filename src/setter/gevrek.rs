//! gevrek draws its wallpaper itself, from the `desktop.wallpaper` section of its running
//! configuration. walle changes that section through gevrek's own command line (which talks
//! to the running window manager): one picture for every display, or one per display.
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use super::Placement;
use crate::{
    desktop::{Display, Target},
    model::FitMode,
};

/// This session is a gevrek session.
pub fn detected() -> bool {
    ["XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP", "DESKTOP_SESSION"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .any(|v| names_gevrek(&v))
}

/// `gevrek`, `Gevrek`, `gevrek:wlroots`, `/usr/share/wayland-sessions/gevrek.desktop`.
fn names_gevrek(value: &str) -> bool {
    value.to_lowercase().split([':', ';', '/', '.']).any(|part| part == "gevrek")
}

/// The `gevrek` command: beside the session's own programs if it is not on the PATH.
fn cli() -> Command {
    let beside = dirs_home().map(|h| h.join(".local/bin/gevrek")).filter(|p| p.is_file());
    let on_path = std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join("gevrek").is_file()));
    let mut cmd = match (on_path, beside) {
        (false, Some(p)) => Command::new(p),
        _ => Command::new("gevrek"),
    };
    cmd.stdin(Stdio::null());
    cmd
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)
}

fn run(args: &[&str]) -> Result<String> {
    let out = cli().args(args).output().context("running gevrek (is it installed and on the PATH?)")?;
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr);
        let said = said.trim();
        bail!("gevrek {}: {}", args.first().unwrap_or(&""), if said.is_empty() { "failed" } else { said });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The displays of the running session.
pub fn displays() -> Result<Vec<Display>> {
    parse_displays(&run(&["state", "--json"])?)
}

fn parse_displays(state: &str) -> Result<Vec<Display>> {
    let state: Value = serde_json::from_str(state).context("reading gevrek's state")?;
    let monitors = state.get("monitors").and_then(Value::as_array).context("gevrek's state lists no displays")?;
    let number = |v: &Value, a: &str, b: &str| v.get(a).and_then(|p| p.get(b)).and_then(Value::as_f64);
    let mut out: Vec<Display> = monitors
        .iter()
        .filter_map(|m| {
            // The connector is what the wallpaper section names a display by.
            let name = m.get("connector").and_then(Value::as_str).filter(|s| !s.is_empty()).or(m.get("name").and_then(Value::as_str))?;
            Some(Display {
                name: name.to_string(),
                x: number(m, "position", "x").unwrap_or(0.0).round() as i32,
                y: number(m, "position", "y").unwrap_or(0.0).round() as i32,
                width: number(m, "size", "width").filter(|w| *w >= 1.0)? as u32,
                height: number(m, "size", "height").filter(|h| *h >= 1.0)? as u32,
                scale: m.get("scale").and_then(Value::as_f64).filter(|s| *s > 0.0).unwrap_or(1.0) as f32,
            })
        })
        .collect();
    // Left to right, then top to bottom: the order they are offered in.
    out.sort_by_key(|d| (d.x, d.y));
    Ok(out)
}

fn fit_name(fit: FitMode) -> &'static str {
    match fit {
        // One display cannot span; walle cuts a spanned picture into filling parts itself.
        FitMode::Fill | FitMode::Span => "fill",
        FitMode::Fit => "fit",
        FitMode::Stretch => "stretch",
        FitMode::Center => "centre",
        FitMode::Tile => "tile",
    }
}

/// Work the placements into the wallpaper section. A picture for all displays removes the
/// displays' own; everything else in the section (the colour, keys walle does not know) stays.
fn edit_section(section: &mut Value, placements: &[Placement]) {
    if !section.is_object() {
        *section = json!({});
    }
    for p in placements {
        let image = p.path.to_string_lossy().to_string();
        match &p.target {
            Target::All => {
                section["image"] = json!(image);
                section["fit"] = json!(fit_name(p.fit));
                section["outputs"] = json!([]);
            }
            Target::Display(name) => {
                if !section.get("outputs").is_some_and(Value::is_array) {
                    section["outputs"] = json!([]);
                }
                let outputs = section["outputs"].as_array_mut().expect("just made an array");
                let same = |o: &Value| o.get("name").and_then(Value::as_str).is_some_and(|n| n.eq_ignore_ascii_case(name));
                match outputs.iter_mut().find(|o| same(o)) {
                    Some(entry) => {
                        entry["image"] = json!(image);
                        entry["fit"] = json!(fit_name(p.fit));
                    }
                    None => outputs.push(json!({ "name": name, "image": image, "fit": fit_name(p.fit) })),
                }
            }
        }
    }
}

/// A file of our own that others cannot read ahead of us or swap: the configuration passes
/// through it.
fn scratch(name: &str) -> Result<(PathBuf, PathBuf)> {
    let base = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()).map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!("walle-gevrek-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok((dir.clone(), dir.join(name)))
}

fn text(path: &Path) -> Result<&str> {
    path.to_str().context("a path that is not text")
}

/// Put the placements on the desktop in one change of the configuration.
pub fn apply(placements: &[Placement]) -> Result<()> {
    if placements.is_empty() {
        return Ok(());
    }
    change(|section| edit_section(section, placements))
}

/// Read the wallpaper section of the running configuration, let `edit` change it, and hand
/// it back to gevrek.
fn change(edit: impl FnOnce(&mut Value)) -> Result<()> {
    let (dir, file) = scratch("config.json")?;
    let result = (|| {
        run(&["config-pull", text(&file)?])?;
        let mut config: Value =
            serde_json::from_str(&std::fs::read_to_string(&file)?).context("reading gevrek's configuration")?;
        let desktop = config.get_mut("desktop").filter(|d| d.is_object()).context("gevrek's configuration has no desktop section")?;
        if desktop.get("wallpaper").is_none() {
            desktop["wallpaper"] = json!({});
        }
        edit(&mut desktop["wallpaper"]);
        // Newer gevrek takes the section alone, which cannot undo a change someone else made
        // to the rest of the configuration in the meantime.
        let section = dir.join("wallpaper.json");
        std::fs::write(&section, serde_json::to_string_pretty(&desktop["wallpaper"])?)?;
        match cli().args(["wallpaper", "apply", text(&section)?]).output() {
            Ok(out) if out.status.success() => return Ok(()),
            Ok(out) => {
                let said = String::from_utf8_lossy(&out.stderr);
                // An older gevrek does not know the command; anything else is a real refusal.
                if !said.contains("unrecognized subcommand") {
                    bail!("gevrek wallpaper: {}", said.trim());
                }
            }
            Err(e) => return Err(e).context("running gevrek"),
        }
        std::fs::write(&file, serde_json::to_string_pretty(&config)?)?;
        run(&["push", text(&file)?]).map(|_| ())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// The pictures gevrek shows now: the common one, and each display's own.
pub fn current() -> Vec<PathBuf> {
    let Ok((dir, file)) = scratch("current.json") else { return vec![] };
    let read = (|| {
        run(&["config-pull", text(&file)?])?;
        let config: Value = serde_json::from_str(&std::fs::read_to_string(&file)?)?;
        anyhow::Ok(section_images(&config["desktop"]["wallpaper"]))
    })();
    let _ = std::fs::remove_dir_all(&dir);
    read.unwrap_or_default()
}

fn section_images(section: &Value) -> Vec<PathBuf> {
    let home = dirs_home();
    let path = |v: &Value| {
        let s = v.get("image").and_then(Value::as_str)?.trim();
        if s.is_empty() {
            return None;
        }
        Some(match (s.strip_prefix("~/"), &home) {
            (Some(rest), Some(home)) => home.join(rest),
            _ => PathBuf::from(s),
        })
    };
    let mut out: Vec<PathBuf> = path(section).into_iter().collect();
    if let Some(outputs) = section.get("outputs").and_then(Value::as_array) {
        out.extend(outputs.iter().filter_map(path));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_a_gevrek_session_by_its_name() {
        assert!(names_gevrek("gevrek"));
        assert!(names_gevrek("Gevrek:wlroots"));
        assert!(names_gevrek("/usr/share/wayland-sessions/gevrek.desktop"));
        assert!(!names_gevrek("KDE"));
        assert!(!names_gevrek("notgevrek"));
    }

    #[test]
    fn reads_the_displays() {
        let state = r#"{"monitors": [
            {"name": "DP-3", "connector": "DP-3", "position": {"x": 2560.0, "y": 0.0}, "size": {"width": 2560, "height": 1440}, "scale": 1.0},
            {"name": "DP-2", "connector": "DP-2", "position": {"x": 0.0, "y": 0.0}, "size": {"width": 1920, "height": 1080}, "scale": 1.5},
            {"name": "broken", "position": {"x": 0, "y": 0}, "size": {"width": 0, "height": 0}}
        ]}"#;
        let d = parse_displays(state).unwrap();
        assert_eq!(d.len(), 2);
        assert_eq!((d[0].name.as_str(), d[0].pixels()), ("DP-2", (2880, 1620)), "left first");
        assert_eq!((d[1].name.as_str(), d[1].x, d[1].width), ("DP-3", 2560, 2560));
        assert!(parse_displays("{}").is_err());
    }

    #[test]
    fn edits_only_what_it_sets() {
        let place = |target: Target, path: &str, fit| Placement { target, path: path.into(), fit };
        let mut section = json!({
            "image": "/old.jpg", "fit": "fit", "color": [1, 2, 3], "later": true,
            "outputs": [{"name": "DP-2", "image": "/two.jpg", "fit": "tile", "kept": 1}]
        });
        edit_section(&mut section, &[
            place(Target::Display("dp-2".into()), "/new-two.jpg", FitMode::Center),
            place(Target::Display("DP-3".into()), "/three.jpg", FitMode::Span),
        ]);
        assert_eq!(section["image"], "/old.jpg");
        assert_eq!(section["outputs"], json!([
            {"name": "DP-2", "image": "/new-two.jpg", "fit": "centre", "kept": 1},
            {"name": "DP-3", "image": "/three.jpg", "fit": "fill"}
        ]));
        assert_eq!(section_images(&section), vec![PathBuf::from("/old.jpg"), "/new-two.jpg".into(), "/three.jpg".into()]);
        // One for all: the displays' own go, the colour and unknown keys stay.
        edit_section(&mut section, &[place(Target::All, "/all.png", FitMode::Stretch)]);
        assert_eq!(section, json!({"image": "/all.png", "fit": "stretch", "color": [1, 2, 3], "later": true, "outputs": []}));
        // A configuration from before there was a wallpaper.
        let mut none = Value::Null;
        edit_section(&mut none, &[place(Target::Display("DP-1".into()), "/a.jpg", FitMode::Fill)]);
        assert_eq!(none, json!({"outputs": [{"name": "DP-1", "image": "/a.jpg", "fit": "fill"}]}));
    }

    /// Changes the real desktop for a few seconds, then puts back what was there. Only in a
    /// gevrek session, and only on request: `cargo test gevrek_live -- --ignored --nocapture`.
    /// `WALLE_TEST_PAUSE=<seconds>` holds each step for looking at.
    #[test]
    #[ignore]
    fn gevrek_live_per_display_span_and_effects() {
        use crate::effects::{Effects, render_one, render_span};
        assert!(detected(), "not a gevrek session");
        let pause = || {
            let secs = std::env::var("WALLE_TEST_PAUSE").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
            std::thread::sleep(std::time::Duration::from_secs(secs));
        };
        let mut before = Value::Null;
        change(|section| before = section.clone()).expect("reading the section");
        let dir = std::env::temp_dir().join(format!("walle-gevrek-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.png");
        // Red at the left, blue at the right, green growing downwards.
        image::RgbImage::from_fn(1600, 450, |x, y| image::Rgb([255 - (x * 255 / 1599) as u8, (y * 255 / 449) as u8, (x * 255 / 1599) as u8]))
            .save(&src)
            .unwrap();
        let result = std::panic::catch_unwind(|| {
            let displays = displays().expect("displays");
            assert!(!displays.is_empty());
            println!("displays: {displays:?}");
            let out = dir.join("out");
            // Each display its own, with a different look.
            let looks = [Effects { dim: 60.0, vignette: 80.0, ..Default::default() }, Effects { saturation: 0.0, blur: 40.0, ..Default::default() }];
            let placements: Vec<Placement> = displays
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    let r = render_one(&src, &looks[i % 2].look(0), FitMode::Fill, Some(d), &out).unwrap();
                    Placement { target: Target::Display(d.name.clone()), path: r.path, fit: r.fit }
                })
                .collect();
            apply(&placements).expect("per display");
            let now = current();
            for p in &placements {
                assert!(now.contains(&p.path), "{} is shown", p.path.display());
            }
            pause();
            // One picture across all of them.
            let parts = render_span(&src, &Effects::default().look(0), &displays, &out).unwrap();
            let span: Vec<Placement> =
                parts.into_iter().map(|(name, r)| Placement { target: Target::Display(name), path: r.path, fit: r.fit }).collect();
            apply(&span).expect("span");
            pause();
            // One for all: the displays' own entries go.
            apply(&[Placement { target: Target::All, path: src.clone(), fit: FitMode::Fit }]).expect("all");
            assert_eq!(current(), vec![src.clone()]);
            pause();
        });
        change(|section| *section = before.clone()).expect("putting the wallpaper back");
        let mut after = Value::Null;
        change(|section| after = section.clone()).unwrap();
        assert_eq!(after, before, "the wallpaper from before is back");
        let _ = std::fs::remove_dir_all(&dir);
        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
    }
}
