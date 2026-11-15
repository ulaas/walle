//! Headless UI tests (egui_kittest). They use the offline "Local folders" provider with
//! generated images in a temp directory, so no network or user data is touched.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::Key;
use egui_kittest::{Harness, kittest::Queryable};

use super::{App, View, settings::Section, theme::ph};
use crate::{
    config::{Config, Paths, ProviderSettings},
    model::Status,
};

struct Sandbox {
    root: PathBuf,
    images: PathBuf,
    paths: Paths,
}

impl Sandbox {
    fn new(name: &str, n_images: usize) -> Self {
        let root = std::env::temp_dir().join(format!("walle-uitest-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let images = root.join("images");
        std::fs::create_dir_all(&images).unwrap();
        for i in 0..n_images {
            let img = image::RgbImage::from_fn(160, 90, |x, y| image::Rgb([(i * 60) as u8, x as u8, y as u8]));
            img.save(images.join(format!("wall-{i}.png"))).unwrap();
        }
        let paths = Paths {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
        };
        for d in [&paths.config_dir, &paths.data_dir, &paths.thumbs_dir()] {
            std::fs::create_dir_all(d).unwrap();
        }
        let mut cfg = Config::default();
        for ps in cfg.providers.values_mut() {
            ps.enabled = false;
        }
        cfg.providers.insert("local".into(), ProviderSettings { enabled: true, ..Default::default() });
        cfg.default_provider = "local".into();
        cfg.local_folders = vec![images.clone()];
        cfg.save(&paths).unwrap();
        Self { root, images, paths }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn harness(sb: &Sandbox) -> Harness<'static, App> {
    let paths = sb.paths.clone();
    Harness::builder()
        .with_size([1400.0, 900.0])
        .build_eframe(move |cc| App::new(cc, paths, super::StartOptions { tray: false, minimized: false, about: false, on_show: None }).expect("app"))
}

/// Step the UI until `cond` holds (the worker runs on its own thread).
fn wait_until(h: &mut Harness<'static, App>, what: &str, cond: impl Fn(&App) -> bool) {
    let start = Instant::now();
    while !cond(h.state()) {
        assert!(start.elapsed() < Duration::from_secs(20), "timed out waiting for {what}");
        h.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    h.step();
}

#[test]
fn browse_and_curate_with_keyboard() {
    let sb = Sandbox::new("curate", 3);
    let mut h = harness(&sb);
    wait_until(&mut h, "local results", |a| a.results.len() == 3);
    assert_eq!(h.state().provider, "local");
    assert!(h.state().results.iter().all(|w| w.width == 160 && w.height == 90));

    // First arrow press selects the first item; K keeps it.
    h.key_press(Key::ArrowRight);
    h.step();
    let first = h.state().selected.clone().expect("selection");
    h.key_press(Key::K);
    h.step();
    h.step();
    let e = h.state().lib.entry(&first.key()).unwrap().unwrap();
    assert_eq!(e.status, Status::Kept);
    assert_eq!(e.local_path.as_deref(), Some(Path::new(&first.full_url)), "local files are their own download");

    // Rate 4, then reject the next one with X.
    h.key_press(Key::Num4);
    h.step();
    assert_eq!(h.state().lib.entry(&first.key()).unwrap().unwrap().rating, 4);
    h.key_press(Key::ArrowRight);
    h.step();
    let second = h.state().selected.clone().unwrap();
    assert_ne!(first.key(), second.key());
    h.key_press(Key::X);
    h.step();
    h.step();
    assert!(h.state().rejected.contains(&second.key()));
    assert_eq!(h.state().current_items().len(), 2, "rejected are hidden while browsing");

    // C toggles considering on the current selection.
    let cur = h.state().selected.clone().unwrap();
    h.key_press(Key::C);
    h.step();
    h.step();
    assert_eq!(h.state().lib.entry(&cur.key()).unwrap().unwrap().status, Status::Considering);
    h.key_press(Key::C);
    h.step();
    h.step();
    assert_eq!(h.state().lib.entry(&cur.key()).unwrap().unwrap().status, Status::None);

    // Library view shows the kept wallpaper; Rejected view the rejected one.
    h.state_mut().actions.push(super::Action::SwitchView(View::Library));
    h.step();
    h.step();
    assert_eq!(h.state().lib_items.iter().map(|w| w.key()).collect::<Vec<_>>(), vec![first.key()]);
    h.state_mut().actions.push(super::Action::SwitchView(View::Rejected));
    h.step();
    h.step();
    assert_eq!(h.state().lib_items.len(), 1);
    assert_eq!(h.state().lib_items[0].key(), second.key());
    assert_eq!(h.state().counts, [1, 0, 1, 1]);
}

#[test]
fn multi_select_bulk_tag_and_forget() {
    let sb = Sandbox::new("bulk", 4);
    let mut h = harness(&sb);
    wait_until(&mut h, "local results", |a| a.results.len() == 4);
    h.step();

    // Ctrl+A selects all visible, bulk keep + tag.
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    h.step();
    h.step();
    assert_eq!(h.state().multi.len(), 4);
    h.state_mut().actions.push(super::Action::BulkStatus(Status::Kept));
    h.state_mut().actions.push(super::Action::BulkTag("calm, blue".into()));
    h.step();
    let tags = h.state().lib.all_tags().unwrap();
    assert_eq!(tags, vec![("blue".to_string(), 4), ("calm".to_string(), 4)]);
    assert_eq!(h.state().counts[2], 4);

    // Library filtering by tag + text.
    h.state_mut().actions.push(super::Action::SwitchView(View::Library));
    h.step();
    h.step();
    assert_eq!(h.state().lib_items.len(), 4);
    h.state_mut().lib_filter.text = "wall-2".into();
    h.state_mut().lib_dirty = true;
    h.step();
    assert_eq!(h.state().lib_items.len(), 1);

    // Forget never deletes the user's own local originals.
    let wp = h.state().lib_items[0].clone();
    h.state_mut().actions.push(super::Action::Forget(wp.clone()));
    h.step();
    h.step();
    assert!(h.state().lib.entry(&wp.key()).unwrap().is_none());
    assert!(Path::new(&wp.full_url).exists(), "local original must survive");
    assert!(sb.images.join("wall-2.png").exists());
}

#[test]
fn renders_every_view_without_panicking() {
    let sb = Sandbox::new("views", 2);
    let mut h = harness(&sb);
    wait_until(&mut h, "local results", |a| a.results.len() == 2);
    h.key_press(Key::ArrowRight);
    h.step();
    for v in [View::Browse, View::Considering, View::Library, View::Rejected, View::History, View::Settings] {
        h.state_mut().actions.push(super::Action::SwitchView(v));
        h.step();
        h.step();
        assert_eq!(h.state().view, v);
    }
    // Every settings section draws, and Sources lists every provider.
    for section in Section::ALL.into_iter().rev() {
        section_of_settings(&mut h, section);
        assert!(h.query_by_role_and_label(egui::Role::Tab, section.label()).is_some(), "{section:?} is in the list on the left");
    }
    assert!(h.query_by_label("Theme").is_some());
    section_of_settings(&mut h, Section::Sources);
    assert!(h.query_by_label_contains("Wallhaven").is_some());
    assert!(h.query_by_label_contains("Unsplash").is_some());
}

/// Every non-ASCII character used in UI/provider strings must exist in the installed fonts
/// (otherwise egui draws a "tofu" box).
#[test]
fn all_ui_glyphs_are_renderable() {
    let sb = Sandbox::new("glyphs", 0);
    let mut h = harness(&sb);
    h.step();
    let sources = [
        include_str!("mod.rs"),
        include_str!("grid.rs"),
        include_str!("detail.rs"),
        include_str!("sidebar.rs"),
        include_str!("settings.rs"),
        include_str!("lightbox.rs"),
        include_str!("theme.rs"),
        include_str!("compare.rs"),
        include_str!("titlebar.rs"),
        include_str!("about.rs"),
        include_str!("../model.rs"),
        include_str!("../providers/unsplash.rs"),
        include_str!("../providers/pexels.rs"),
        include_str!("../providers/pixabay.rs"),
        include_str!("../providers/nasa.rs"),
        include_str!("../providers/local.rs"),
    ];
    let mut chars: Vec<char> = sources
        .iter()
        .flat_map(|s| s.lines())
        .filter(|l| !l.trim_start().starts_with("//"))
        .flat_map(|l| l.chars())
        .filter(|c| !c.is_ascii())
        .collect();
    chars.sort();
    chars.dedup();
    let ctx = h.ctx.clone();
    let missing: Vec<char> = ctx.fonts_mut(|f| {
        chars
            .iter()
            .copied()
            .filter(|c| {
                !f.has_glyph(&egui::FontId::proportional(14.0), *c)
                    && !f.has_glyph(&egui::FontId::new(14.0, egui::FontFamily::Name(super::theme::SEMIBOLD.into())), *c)
            })
            .collect()
    });
    // The vendored icon fonts are wired up, for regular and semibold text and the filled variant.
    let fonts = [
        egui::FontId::proportional(14.0),
        egui::FontId::new(14.0, egui::FontFamily::Name(super::theme::SEMIBOLD.into())),
        egui::FontId::new(14.0, egui::FontFamily::Name(super::theme::ICON_FILL.into())),
    ];
    let glyph = ph::GEAR_SIX.chars().next().unwrap();
    assert!(ctx.fonts_mut(|f| fonts.iter().all(|font| f.has_glyph(font, glyph))), "icon font missing");
    // Negative control: the check must be able to detect a glyph that is not installed.
    assert!(!ctx.fonts_mut(|f| f.has_glyph(&egui::FontId::proportional(14.0), '\u{2192}')), "control glyph unexpectedly present");
    assert!(missing.is_empty(), "missing glyphs: {missing:?} (checked {} chars)", chars.len());
}

#[test]
fn compare_mode_end_to_end() {
    let sb = Sandbox::new("duel", 3);
    let mut h = harness(&sb);
    wait_until(&mut h, "local results", |a| a.results.len() == 3);
    let items = h.state().results.clone();
    for w in &items {
        h.state_mut().actions.push(super::Action::SetStatus(w.clone(), Status::Considering));
    }
    h.state_mut().actions.push(super::Action::SwitchView(View::Considering));
    h.step();
    h.step();
    assert_eq!(h.state().lib_items.len(), 3);
    let shortlist = h.state().lib_items.clone();
    h.state_mut().actions.push(super::Action::Compare(shortlist.clone()));
    h.step();
    assert!(h.state().duel.is_some());

    // Right (challenger) wins round 1 -> champion (left) rejected by default.
    h.key_press(Key::ArrowRight);
    h.step();
    h.step();
    let loser1 = shortlist[0].key();
    assert_eq!(h.state().lib.entry(&loser1).unwrap().unwrap().status, Status::Rejected);
    // Undo restores it.
    h.key_press(Key::Backspace);
    h.step();
    h.step();
    assert_eq!(h.state().lib.entry(&loser1).unwrap().unwrap().status, Status::Considering);
    // Left wins twice -> finished with shortlist[0] as champion.
    h.key_press(Key::ArrowLeft);
    h.step();
    h.key_press(Key::ArrowLeft);
    h.step();
    h.step();
    assert!(h.state().duel.as_ref().unwrap().finished());
    h.key_press(Key::Enter);
    h.step();
    h.step();
    assert!(h.state().duel.is_none());
    let st = |k: &str| h.state().lib.entry(k).unwrap().unwrap().status;
    assert_eq!(st(&shortlist[0].key()), Status::Kept);
    assert_eq!(st(&shortlist[1].key()), Status::Rejected);
    assert_eq!(st(&shortlist[2].key()), Status::Rejected);
}

/// On a desktop with a wallpaper per display: one display alone, all of them, a picture
/// spanned across them, effects rendered into copies, and rotation with one for each.
#[test]
fn wallpapers_per_display_with_effects() {
    use crate::desktop::{Display, Target};
    let sb = Sandbox::new("displays", 4);
    let display = |name: &str, x: i32| Display { name: name.into(), x, y: 0, width: 80, height: 45, scale: 1.0 };
    crate::worker::TEST_DISPLAYS.lock().unwrap().push((sb.paths.data_dir.clone(), vec![display("L", 0), display("R", 80)]));
    let mut h = harness(&sb);
    wait_until(&mut h, "local results", |a| a.results.len() == 4);
    wait_until(&mut h, "the displays", |a| a.displays.len() == 2);
    let items = h.state().results.clone();
    let placed = |sb: &Sandbox| -> Vec<crate::setter::Placement> {
        crate::worker::TEST_PLACED.lock().unwrap().iter().filter(|p| p.path.starts_with(&sb.root)).cloned().collect()
    };
    let rendered = crate::effects::render_dir(&sb.paths.data_dir);

    // One display alone gets the original: nothing to render without effects.
    h.state_mut().actions.push(super::Action::SetWallpaperOn(items[0].clone(), "R".into()));
    wait_until(&mut h, "the right display's wallpaper", |a| a.current_keys == vec![items[0].key()]);
    let log = placed(&sb);
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].target, Target::Display("R".into()));
    assert!(log[0].path.starts_with(&sb.images));
    assert_eq!(h.state().shown_on("R"), Some(items[0].key().as_str()));
    assert_eq!(h.state().shown_on("L"), None);

    // The other display: both are on the desktop now.
    h.state_mut().actions.push(super::Action::SetWallpaperOn(items[1].clone(), "L".into()));
    wait_until(&mut h, "both displays", |a| a.current_keys.len() == 2);
    assert_eq!(h.state().lib.history(10).unwrap().len(), 2);

    // Effects are switched on: what is on the desktop is rendered again, into copies.
    let mut cfg = h.state().config.clone();
    cfg.effects.dim = 50.0;
    h.state_mut().apply_config(cfg);
    wait_until(&mut h, "rendered copies", |_| placed(&sb).iter().filter(|p| p.path.starts_with(&rendered)).count() == 2);
    assert_eq!(h.state().current_keys.len(), 2, "the same two wallpapers");
    let copy = placed(&sb).into_iter().rev().find(|p| p.target == Target::Display("R".into())).unwrap();
    let px = image::open(&copy.path).unwrap().into_rgb8();
    assert_eq!(px.dimensions(), (80, 45), "no larger than the display");
    let original = image::open(sb.images.join("wall-0.png")).unwrap().into_rgb8();
    assert!(px.get_pixel(40, 22).0[1] < original.get_pixel(80, 45).0[1], "dimmed");
    assert!(original.get_pixel(80, 45).0[1] > 40, "the original is untouched");

    // One picture across both: each display gets its part, and "all" replaces their own.
    let mut cfg = h.state().config.clone();
    cfg.fit_mode = crate::model::FitMode::Span;
    h.state_mut().apply_config(cfg);
    let before = placed(&sb).len();
    h.state_mut().actions.push(super::Action::SetWallpaper(items[2].clone()));
    wait_until(&mut h, "the spanned wallpaper", |a| a.current_keys == vec![items[2].key()]);
    let span = placed(&sb)[before..].to_vec();
    assert_eq!(span.iter().map(|p| p.target.clone()).collect::<Vec<_>>(), vec![Target::All, Target::Display("L".into()), Target::Display("R".into())]);
    assert_ne!(span[1].path, span[2].path);
    assert!(h.state().shown.displays.is_empty());

    // Rotation with a wallpaper for each display picks two different ones.
    let mut cfg = h.state().config.clone();
    cfg.rotation.per_display = true;
    cfg.rotation.source = crate::config::RotationSource::Provider("local".into());
    cfg.fit_mode = crate::model::FitMode::Fill;
    h.state_mut().apply_config(cfg);
    h.state().remote_tx.send(super::Remote::Next).unwrap();
    wait_until(&mut h, "one for each display", |a| a.shown.displays.len() == 2);
    wait_until(&mut h, "both known to the UI", |a| a.current_keys.len() == 2);
    let (l, r) = (h.state().shown_on("L").unwrap().to_string(), h.state().shown_on("R").unwrap().to_string());
    assert_ne!(l, r);

    // Only what is on the desktop is kept of the rendered copies.
    let files = std::fs::read_dir(&rendered).unwrap().count();
    assert_eq!(files, 2, "older renderings are removed");
}

#[test]
fn rotation_next_and_toggle_via_remote_commands() {
    let sb = Sandbox::new("rotate", 3);
    let mut h = harness(&sb);
    wait_until(&mut h, "local results", |a| a.results.len() == 3);
    let items = h.state().results.clone();
    // Keep two; the third stays untriaged and must never be picked from "Kept".
    for w in &items[..2] {
        h.state_mut().actions.push(super::Action::SetStatus(w.clone(), Status::Kept));
    }
    h.step();
    let kept: Vec<String> = items[..2].iter().map(|w| w.key()).collect();

    let tx = h.state().remote_tx.clone();
    tx.send(super::Remote::Next).unwrap();
    wait_until(&mut h, "rotation to set a wallpaper", |a| !a.current_keys.is_empty());
    let first = h.state().current_keys[0].clone();
    assert!(kept.contains(&first), "rotated to a kept wallpaper");
    assert_eq!(h.state().lib.history(10).unwrap().len(), 1);
    let logged = crate::worker::TEST_SET_LOG.lock().unwrap().clone();
    assert!(logged.iter().any(|p| p.starts_with(&sb.images)), "set via the test hook, not the real desktop");

    // Second rotation avoids repeating the same wallpaper.
    tx.send(super::Remote::Next).unwrap();
    wait_until(&mut h, "second rotation", |a| a.current_keys.first().is_some_and(|k| *k != first));
    assert!(kept.contains(&h.state().current_keys[0]));

    // Tray "Rotate automatically" toggles and persists the setting.
    assert!(!h.state().config.rotation.enabled);
    tx.send(super::Remote::ToggleRotation).unwrap();
    h.step();
    h.step();
    assert!(h.state().config.rotation.enabled);
    assert!(Config::load(&sb.paths).rotation.enabled, "saved to config.toml");
    wait_until(&mut h, "next rotation time", |a| a.rotation_next.is_some());
}

fn scene(w: u32, h: u32, shift: f32) -> image::RgbImage {
    image::RgbImage::from_fn(w, h, |x, y| {
        let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
        let r = (255.0 * (u * 3.0 + shift).sin().abs()) as u8;
        let g = (255.0 * (v * 2.0 + shift).cos().abs()) as u8;
        image::Rgb([r, g, (255.0 * u * v) as u8])
    })
}

#[test]
fn duplicates_are_flagged_hidden_and_grouped() {
    let sb = Sandbox::new("dupes", 0);
    let a = scene(640, 360, 0.0);
    a.save(sb.images.join("a-original.png")).unwrap();
    image::imageops::resize(&a, 320, 180, image::imageops::FilterType::Triangle).save(sb.images.join("a-copy-small.jpg")).unwrap();
    scene(640, 360, 2.1).save(sb.images.join("b-different.png")).unwrap();

    let mut h = harness(&sb);
    wait_until(&mut h, "results", |a| a.results.len() == 3);
    wait_until(&mut h, "thumbnail hashes", |a| a.hashes.len() == 3);
    let by_name = |h: &Harness<'static, App>, n: &str| h.state().results.iter().find(|w| w.full_url.contains(n)).cloned().unwrap();
    let orig = by_name(&h, "a-original");
    let copy = by_name(&h, "a-copy-small");
    let other = by_name(&h, "b-different");

    // Keep the original; the resized copy is now flagged as a duplicate of a kept wallpaper.
    h.state_mut().actions.push(super::Action::SetStatus(orig.clone(), Status::Kept));
    h.step();
    h.step();
    assert_eq!(h.state().dup_of.get(&copy.key()).cloned().flatten().map(|d| d.0), Some(orig.key()));
    assert_eq!(h.state().dup_of.get(&other.key()).cloned().flatten(), None, "different image not flagged");

    // "Hide duplicates" removes it from browse results.
    assert_eq!(h.state().current_items().len(), 3);
    h.state_mut().config.hide_duplicates = true;
    let visible: Vec<String> = h.state().current_items().iter().map(|w| w.key()).collect();
    assert!(!visible.contains(&copy.key()) && visible.contains(&orig.key()) && visible.contains(&other.key()));
    h.state_mut().config.hide_duplicates = false;

    // Once both are in the library, the Library "Duplicates" filter groups them.
    h.state_mut().actions.push(super::Action::SetStatus(copy.clone(), Status::Considering));
    h.state_mut().actions.push(super::Action::SetStatus(other.clone(), Status::Kept));
    h.state_mut().actions.push(super::Action::SwitchView(View::Library));
    h.step();
    h.state_mut().lib_dupes = true;
    h.state_mut().lib_dirty = true;
    h.step();
    h.step();
    let mut keys: Vec<String> = h.state().lib_items.iter().map(|w| w.key()).collect();
    keys.sort();
    let mut want = vec![orig.key(), copy.key()];
    want.sort();
    assert_eq!(keys, want);
}

/// Changing the search while one is in flight must not mix the old answer into the new results.
#[test]
fn a_new_search_supersedes_the_one_in_flight() {
    let sb = Sandbox::new("stale", 3);
    let mut h = harness(&sb);
    wait_until(&mut h, "local results", |a| a.results.len() == 3);

    let app = h.state_mut();
    app.query.text = "does-not-match-anything".into();
    app.new_search();
    assert!(app.loading);
    let first_request = app.req_id;
    app.query.text = "wall-1".into();
    app.new_search();
    assert!(app.req_id > first_request, "the second search is a new request");

    wait_until(&mut h, "second search", |a| !a.loading);
    let titles: Vec<_> = h.state().results.iter().map(|w| w.display_title()).collect();
    assert_eq!(titles, ["wall-1"]);
    // The answer to the first search is late, not lost: keep stepping and it must stay out.
    for _ in 0..10 {
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(h.state().results.len(), 1);
    assert_eq!(h.state().search_error, None);
}

/// A page whose wallpapers are all hidden must not read as "no results" while more pages exist.
#[test]
fn a_fully_filtered_page_loads_the_next_one() {
    let sb = Sandbox::new("filtered", 61);
    let mut h = harness(&sb);
    wait_until(&mut h, "first page", |a| a.results.len() == 60);
    let app = h.state_mut();
    app.rejected = app.results.iter().map(|w| w.key()).collect();
    app.visible_dirty = true;
    wait_until(&mut h, "second page", |a| a.results.len() == 61);
    assert_eq!(h.state().current_items().len(), 1);
}

/// Holding X must not reject one wallpaper after another.
#[test]
fn key_repeat_only_moves_the_selection() {
    let sb = Sandbox::new("repeat", 4);
    let mut h = harness(&sb);
    wait_until(&mut h, "local results", |a| a.results.len() == 4);
    h.key_press(Key::ArrowRight);
    h.step();
    let first = h.state().selected.clone().unwrap();

    // Pressed again without a release is what the OS sends for a held key.
    let down = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    h.input_mut().events.push(down(Key::X));
    h.step();
    h.step();
    assert!(h.state().rejected.contains(&first.key()), "the first press counts");
    let after_first = h.state().selected.as_ref().unwrap().key();
    for _ in 0..3 {
        h.input_mut().events.push(down(Key::X));
        h.step();
        h.step();
    }
    assert_eq!(h.state().rejected.len(), 1, "repeats are not decisions");
    assert_eq!(h.state().selected.as_ref().unwrap().key(), after_first);

    h.input_mut().events.push(down(Key::ArrowRight));
    h.step();
    h.input_mut().events.push(down(Key::ArrowRight));
    h.step();
    assert_ne!(h.state().selected.as_ref().unwrap().key(), after_first, "moving does repeat");
}

/// The grid's list follows the changes that affect it, and only rebuilds for those.
#[test]
fn visible_list_is_cached_until_an_input_changes() {
    let sb = Sandbox::new("visible", 3);
    let mut h = harness(&sb);
    wait_until(&mut h, "local results", |a| a.results.len() == 3);
    h.step();
    let before = h.state().visible.clone();
    h.step();
    h.step();
    assert!(std::rc::Rc::ptr_eq(&before, &h.state().visible), "nothing changed, nothing rebuilt");

    let wp = h.state().results[0].clone();
    h.state_mut().actions.push(super::Action::SetStatus(wp.clone(), Status::Rejected));
    h.step();
    h.step();
    assert_eq!(h.state().visible.len(), 2);
    assert!(!h.state().visible_order.contains(&wp.key()));

    h.state_mut().config.show_rejected = true;
    h.step();
    assert_eq!(h.state().visible.len(), 3, "toggling a view option is noticed by itself");
}

/// Bulk actions drop wallpapers that left the list from the selection.
#[test]
fn selection_forgets_what_is_no_longer_listed() {
    let sb = Sandbox::new("prune", 3);
    let mut h = harness(&sb);
    wait_until(&mut h, "local results", |a| a.results.len() == 3);
    h.step();
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    h.step();
    h.step();
    assert_eq!(h.state().multi.len(), 3);
    h.state_mut().actions.push(super::Action::BulkStatus(Status::Rejected));
    h.step();
    h.step();
    assert!(h.state().multi.is_empty());
    assert_eq!(h.state().counts[3], 3, "all rejected in one batch");
}

#[test]
fn library_search_waits_for_a_pause_in_typing() {
    let sb = Sandbox::new("debounce", 2);
    let mut h = harness(&sb);
    wait_until(&mut h, "local results", |a| a.results.len() == 2);
    for w in h.state().results.clone() {
        h.state_mut().actions.push(super::Action::SetStatus(w, Status::Kept));
    }
    h.state_mut().actions.push(super::Action::SwitchView(View::Library));
    h.step();
    h.step();
    assert_eq!(h.state().lib_items.len(), 2);

    // What the sidebar does on a keystroke: no reload until the deadline has passed.
    let app = h.state_mut();
    app.lib_filter.text = "wall-1".into();
    let now = app.ctx.input(|i| i.time);
    app.filter_deadline = Some(now + 3600.0);
    h.step();
    assert_eq!(h.state().lib_items.len(), 2);
    h.state_mut().filter_deadline = Some(0.0);
    h.step();
    assert_eq!(h.state().lib_items.len(), 1);
    assert_eq!(h.state().filter_deadline, None);
}

#[test]
fn picker_results_are_applied() {
    let sb = Sandbox::new("pickers", 1);
    let mut h = harness(&sb);
    wait_until(&mut h, "results", |a| a.results.len() == 1);

    // Export to a chosen file.
    let out = sb.root.join("my-export.json");
    let tx = h.state().picker_tx.clone();
    tx.send(Some(super::pickers::Picked::ExportTo(out.clone()))).unwrap();
    h.step();
    h.step();
    assert!(out.exists(), "export written to the picked path");

    // A picked folder in Settings goes into the settings draft (saved with the rest).
    let extra = sb.root.join("more");
    std::fs::create_dir_all(&extra).unwrap();
    h.state_mut().actions.push(super::Action::SwitchView(View::Settings));
    h.step();
    tx.send(Some(super::pickers::Picked::LocalFolder(extra.clone()))).unwrap();
    h.step();
    h.step();
    assert!(h.state().settings_draft.as_ref().unwrap().local_folders.contains(&extra));
    assert!(!h.state().config.local_folders.contains(&extra), "not applied until Save");

    // Cancelled dialog: nothing happens, picker becomes available again.
    h.state_mut().picker_busy = true;
    tx.send(None).unwrap();
    h.step();
    assert!(!h.state().picker_busy);
}

#[test]
fn about_box_names_the_build() {
    let sb = Sandbox::new("about", 1);
    let mut h = harness(&sb);
    wait_until(&mut h, "results", |a| a.results.len() == 1);
    assert!(h.query_by_label_contains(crate::build_info::VERSION).is_none());

    h.key_press(Key::F1);
    h.step();
    h.step();
    assert!(h.state().show_about);
    assert!(h.query_by_label_contains(crate::build_info::VERSION).is_some(), "version shown");
    assert!(h.query_by_label_contains(crate::build_info::COMMIT_SHORT).is_some(), "commit shown");

    // Esc closes the box and nothing else: the selection stays.
    let wp = h.state().results[0].clone();
    h.state_mut().actions.push(super::Action::Select(wp));
    h.step();
    h.key_press(Key::Escape);
    h.step();
    h.step();
    assert!(!h.state().show_about);
    assert!(h.state().selected.is_some(), "Esc was for the box");

    // The tray's "About walle" opens it as well.
    let tx = h.state().remote_tx.clone();
    tx.send(super::Remote::About).unwrap();
    h.step();
    assert!(h.state().show_about);
}

/// Clicking the logo opens About; the window buttons are there, under accessible names.
#[test]
fn title_bar_has_logo_and_window_controls() {
    let sb = Sandbox::new("titlebar", 1);
    let mut h = harness(&sb);
    wait_until(&mut h, "results", |a| a.results.len() == 1);
    for name in ["Minimize", "Maximize", "Close"] {
        assert!(h.query_by_label(name).is_some(), "{name} button");
    }
    for view in ["Browse", "Considering", "Library", "Rejected", "History", "Settings"] {
        assert!(h.query_by_label(view).is_some(), "{view} tab");
    }
    h.get_by_label("About walle").click();
    h.step();
    h.step();
    assert!(h.state().show_about);
}

/// What the frame asks of the window system. Commands are per frame, so collect them.
fn step_collecting_commands(h: &mut Harness<'static, App>) -> Vec<egui::ViewportCommand> {
    let mut commands = vec![];
    for _ in 0..3 {
        h.step();
        if let Some(out) = h.output().viewport_output.get(&egui::ViewportId::ROOT) {
            commands.extend(out.commands.iter().cloned());
        }
    }
    commands
}

#[test]
fn window_buttons_send_their_commands() {
    use egui::ViewportCommand as C;
    let sb = Sandbox::new("winbuttons", 1);
    let mut h = harness(&sb);
    wait_until(&mut h, "results", |a| a.results.len() == 1);
    for (button, expected) in [("Minimize", C::Minimized(true)), ("Maximize", C::Maximized(true)), ("Close", C::Close)] {
        h.get_by_label(button).click();
        let sent = step_collecting_commands(&mut h);
        assert!(sent.contains(&expected), "{button}: {sent:?}");
    }
}

/// Pressing and moving on the empty part of the titlebar drags the window; widgets on the bar
/// keep their own clicks, and a double-click maximizes.
#[test]
fn title_bar_drags_and_maximizes() {
    use egui::{Event, PointerButton, ViewportCommand as C, pos2};
    let press = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    let sb = Sandbox::new("titledrag", 1);
    let mut h = harness(&sb);
    wait_until(&mut h, "results", |a| a.results.len() == 1);
    let empty = pos2(900.0, 22.0);

    h.input_mut().events.push(Event::PointerMoved(empty));
    h.step();
    h.input_mut().events.push(press(empty, true));
    h.step();
    h.input_mut().events.push(Event::PointerMoved(empty + egui::vec2(30.0, 10.0)));
    let sent = step_collecting_commands(&mut h);
    assert!(sent.contains(&C::StartDrag), "drag: {sent:?}");
    h.input_mut().events.push(press(empty + egui::vec2(30.0, 10.0), false));
    step_collecting_commands(&mut h);

    h.input_mut().events.push(Event::PointerMoved(empty));
    h.step();
    // Both clicks within one frame: the harness's frames are longer than a double-click.
    for _ in 0..2 {
        h.input_mut().events.push(press(empty, true));
        h.input_mut().events.push(press(empty, false));
    }
    let sent = step_collecting_commands(&mut h);
    assert!(sent.contains(&C::Maximized(true)), "double-click: {sent:?}");
    assert!(!sent.contains(&C::StartDrag), "a click is not a drag");
}

#[test]
fn window_edges_resize() {
    use egui::{Event, PointerButton, ResizeDirection as D, ViewportCommand as C, pos2};
    let sb = Sandbox::new("edges", 1);
    let mut h = harness(&sb);
    wait_until(&mut h, "results", |a| a.results.len() == 1);
    for (pos, direction) in [(pos2(1398.0, 400.0), D::East), (pos2(2.0, 400.0), D::West), (pos2(700.0, 898.0), D::South), (pos2(1398.0, 898.0), D::SouthEast)] {
        h.input_mut().events.push(Event::PointerMoved(pos));
        h.step();
        h.input_mut().events.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.step();
        let sent = h.output().viewport_output.get(&egui::ViewportId::ROOT).map(|o| o.commands.clone()).unwrap_or_default();
        assert!(sent.contains(&C::BeginResize(direction)), "{direction:?}: {sent:?}");
        h.input_mut().events.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.step();
    }
}

/// The titlebar stays usable above full-window overlays.
#[test]
fn overlays_leave_the_title_bar_free() {
    let sb = Sandbox::new("overlay", 2);
    let mut h = harness(&sb);
    wait_until(&mut h, "results", |a| a.results.len() == 2);
    let wp = h.state().results[0].clone();
    h.state_mut().actions.push(super::Action::Lightbox(wp));
    h.step();
    h.step();
    assert!(h.state().lightbox.is_some());
    h.get_by_label("Settings").click();
    h.step();
    h.step();
    assert_eq!(h.state().view, View::Settings, "the titlebar took the click");
}

/// Opening Settings is not an edit: Save stays disabled until something really changes.
#[test]
fn settings_are_unchanged_until_edited() {
    let sb = Sandbox::new("draft", 1);
    let mut h = harness(&sb);
    wait_until(&mut h, "results", |a| a.results.len() == 1);
    h.state_mut().actions.push(super::Action::SwitchView(View::Settings));
    for _ in 0..3 {
        h.step();
    }
    assert_eq!(h.state().settings_draft.as_ref(), Some(&h.state().config), "nothing edited yet");

    // Toggling rotation from the tray while the page is open doesn't read as an edit either.
    h.state().remote_tx.send(super::Remote::ToggleRotation).unwrap();
    h.step();
    h.step();
    assert!(h.state().config.rotation.enabled);
    assert_eq!(h.state().settings_draft.as_ref(), Some(&h.state().config));
}

/// Pasted text reaches every kind of Settings field, masked API keys included. (Getting it from
/// the OS clipboard is eframe's "clipboard" feature, which the headless harness doesn't exercise.)
#[test]
fn pasting_works_in_settings_fields() {
    use egui::{Event, Role};
    let sb = Sandbox::new("paste", 0);
    let paths = sb.paths.clone();
    // Tall enough that all the fields are on screen and can be clicked.
    let mut h: Harness<'static, App> = Harness::builder()
        .with_size([1400.0, 3200.0])
        .build_eframe(move |cc| App::new(cc, paths, super::StartOptions { tray: false, minimized: false, about: false, on_show: None }).expect("app"));
    h.step();
    h.state_mut().actions.push(super::Action::SwitchView(View::Settings));
    // The page's layout takes a few frames to settle, and clicks need the final positions.
    for _ in 0..6 {
        h.step();
    }

    let paste_into = |h: &mut Harness<'static, App>, role: Role, index: usize, text: &str| {
        // A click as a real mouse makes it, over several frames: text fields focus on release.
        let pos = h.query_all_by_role(role).nth(index).unwrap_or_else(|| panic!("{role:?} field {index}")).rect().center();
        let button = |pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        h.input_mut().events.push(Event::PointerMoved(pos));
        h.step();
        h.input_mut().events.push(button(true));
        h.step();
        h.input_mut().events.push(button(false));
        h.step();
        h.step();
        assert!(h.ctx.egui_wants_keyboard_input(), "{role:?} field {index} took focus");
        h.input_mut().events.push(Event::Paste(text.into()));
        h.step();
        h.step();
    };

    // API keys are masked, and the first one belongs to the first provider listed with a key.
    section_of_settings(&mut h, Section::Sources);
    let masked = h.query_all_by_role(Role::PasswordInput).count();
    assert!(masked >= 4, "masked key fields: {masked}");
    paste_into(&mut h, Role::PasswordInput, 0, "pasted-key");
    let keys: Vec<String> = h.state().settings_draft.as_ref().unwrap().providers.values().map(|p| p.api_key.clone()).collect();
    assert!(keys.iter().any(|k| k == "pasted-key"), "keys: {keys:?}");

    // Everything else: the import path, the folder, the Wallhaven username and the Bing market.
    let mut plain = 0;
    for section in Section::ALL {
        section_of_settings(&mut h, section);
        for i in 0..h.query_all_by_role(Role::TextInput).count() {
            paste_into(&mut h, Role::TextInput, i, &format!("pasted{plain}"));
            plain += 1;
        }
    }
    assert!(plain >= 4, "plain fields: {plain}");
    let app = h.state();
    let wallhaven = &app.settings_draft.as_ref().unwrap().providers["wallhaven"];
    let bing = &app.settings_draft.as_ref().unwrap().providers["bing"];
    assert!(app.import_input.contains("pasted"), "import: {:?}", app.import_input);
    assert!(app.folder_input.contains("pasted"), "folder: {:?}", app.folder_input);
    assert!(wallhaven.options["username"].contains("pasted"), "username: {:?}", wallhaven.options);
    assert!(bing.options["market"].contains("pasted"), "market: {:?}", bing.options);
}

/// A click as a real mouse makes it, over several frames.
fn real_click(h: &mut Harness<'static, App>, pos: egui::Pos2) {
    use egui::Event;
    let button = |pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    h.input_mut().events.push(Event::PointerMoved(pos));
    h.step();
    h.input_mut().events.push(button(true));
    h.step();
    h.input_mut().events.push(button(false));
    h.step();
    h.step();
}

fn open_settings(h: &mut Harness<'static, App>) {
    h.state_mut().actions.push(super::Action::SwitchView(View::Settings));
    // The page's layout takes a few frames to settle, and clicks need the final positions.
    for _ in 0..6 {
        h.step();
    }
}

/// Show one section of the settings page, as picking it on the left does.
fn section_of_settings(h: &mut Harness<'static, App>, section: Section) {
    h.state_mut().settings_section = section;
    for _ in 0..6 {
        h.step();
    }
}

/// The list on the left chooses what the settings page shows.
#[test]
fn settings_sections_are_picked_on_the_left() {
    use egui::Role;
    let sb = Sandbox::new("sections", 1);
    let mut h = harness(&sb);
    wait_until(&mut h, "results", |a| a.results.len() == 1);
    open_settings(&mut h);
    assert!(h.query_by_label("Theme").is_some(), "starts on Appearance");
    assert!(h.query_by_label("API key").is_none(), "one section at a time");

    let sources = h.get_by_role_and_label(Role::Tab, "Sources").rect().center();
    real_click(&mut h, sources);
    assert_eq!(h.state().settings_section, Section::Sources);
    h.step();
    assert!(h.query_all_by_label("API key").count() >= 4);
    assert!(h.query_by_label("Theme").is_none());

    // An edit made in one section is still there, and still unsaved, after looking at another.
    h.state_mut().settings_draft.as_mut().unwrap().providers.get_mut("wallhaven").unwrap().api_key = "edited".into();
    let rotation = h.get_by_role_and_label(Role::Tab, "Rotation").rect().center();
    real_click(&mut h, rotation);
    h.step();
    assert!(h.query_by_label_contains("Unsaved changes").is_some());
    assert_eq!(h.state().settings_draft.as_ref().unwrap().providers["wallhaven"].api_key, "edited");
}

fn save_button_pos(h: &Harness<'static, App>) -> egui::Pos2 {
    h.get_by_label_contains("Save changes").rect().center()
}

/// The whole path of an API key: pasted, saved, on disk, and in use without a restart.
#[test]
fn saving_settings_persists_and_reinitializes_the_sources() {
    use egui::{Event, Role};
    let sb = Sandbox::new("savekey", 1);
    let extra = sb.root.join("extra");
    std::fs::create_dir_all(&extra).unwrap();
    landscape(0.3).save(extra.join("from-new-folder.png")).unwrap();
    let paths = sb.paths.clone();
    let mut h: Harness<'static, App> = Harness::builder()
        .with_size([1400.0, 3200.0])
        .build_eframe(move |cc| App::new(cc, paths, super::StartOptions { tray: false, minimized: false, about: false, on_show: None }).expect("app"));
    wait_until(&mut h, "results", |a| a.results.len() == 1);
    open_settings(&mut h);
    section_of_settings(&mut h, Section::Sources);

    let field = h.query_all_by_role(Role::PasswordInput).next().expect("key field").rect().center();
    real_click(&mut h, field);
    h.input_mut().events.push(Event::Paste("pasted-key".into()));
    h.step();
    h.step();
    assert_eq!(h.state().settings_draft.as_ref().unwrap().providers["wallhaven"].api_key, "pasted-key");
    assert_ne!(Config::load(&sb.paths).providers["wallhaven"].api_key, "pasted-key", "nothing written before saving");
    // Also change where a source looks, to see the worker pick up the new settings.
    h.state_mut().settings_draft.as_mut().unwrap().local_folders.push(extra.clone());
    h.step();

    let save = save_button_pos(&h);
    real_click(&mut h, save);
    assert_eq!(Config::load(&sb.paths).providers["wallhaven"].api_key, "pasted-key", "written to config.toml");
    assert_eq!(h.state().config.providers["wallhaven"].api_key, "pasted-key");
    assert_eq!(h.state().settings_draft.as_ref(), Some(&h.state().config), "nothing left unsaved");
    assert!(h.state().infos.iter().any(|i| i.id == "wallhaven" && i.has_key), "the source knows its key");
    wait_until(&mut h, "results from the new folder", |a| a.results.iter().any(|w| w.full_url.contains("from-new-folder")));
    assert_eq!(h.state().search_error, None);
}

/// However far down the page the field is, the way to save it is on screen.
#[test]
fn save_button_stays_on_screen_while_scrolling_settings() {
    let sb = Sandbox::new("sticky", 1);
    let mut h = harness(&sb);
    wait_until(&mut h, "results", |a| a.results.len() == 1);
    open_settings(&mut h);
    section_of_settings(&mut h, Section::Sources);
    let before = h.get_by_label_contains("Save changes").rect();
    let top = h.get_by_label("Wallhaven").rect();
    let nav = h.get_by_role_and_label(egui::Role::Tab, "Sources").rect();

    let middle = egui::pos2(700.0, 500.0);
    h.input_mut().events.push(egui::Event::PointerMoved(middle));
    h.step();
    for _ in 0..8 {
        h.input_mut().events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -400.0),
            modifiers: Default::default(),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
        });
        h.step();
    }
    assert!(h.query_by_label("Wallhaven").is_none_or(|n| n.rect().top() < top.top() - 100.0), "the page did scroll");
    assert_eq!(h.get_by_role_and_label(egui::Role::Tab, "Sources").rect(), nav, "the list of sections did not");
    assert!(h.query_by_label_contains("Unsaved changes").is_none(), "nothing edited yet");
    assert_eq!(h.get_by_label_contains("Save changes").rect(), before, "the button did not");
    assert!(before.bottom() < 900.0);

    h.state_mut().settings_draft.as_mut().unwrap().providers.get_mut("wallhaven").unwrap().api_key = "edited".into();
    h.step();
    h.step();
    assert!(h.query_by_label_contains("Unsaved changes").is_some(), "says so in the fixed header");
    assert!(h.query_by_label("Not saved").is_some(), "and on the edited source");
}

/// Edits are not thrown away by going to another page, or by closing the window.
#[test]
fn leaving_settings_keeps_the_edits() {
    let sb = Sandbox::new("leave", 1);
    let mut h = harness(&sb);
    wait_until(&mut h, "results", |a| a.results.len() == 1);
    open_settings(&mut h);
    h.state_mut().settings_draft.as_mut().unwrap().providers.get_mut("wallhaven").unwrap().api_key = "typed-then-left".into();
    h.step();
    h.state_mut().actions.push(super::Action::SwitchView(View::Browse));
    h.step();
    h.step();
    assert_eq!(h.state().config.providers["wallhaven"].api_key, "typed-then-left");
    assert_eq!(Config::load(&sb.paths).providers["wallhaven"].api_key, "typed-then-left");

    // Back again: the page shows what was saved.
    open_settings(&mut h);
    assert_eq!(h.state().settings_draft.as_ref().unwrap().providers["wallhaven"].api_key, "typed-then-left");
}

#[test]
fn ctrl_s_saves_settings() {
    let sb = Sandbox::new("ctrls", 1);
    let mut h = harness(&sb);
    wait_until(&mut h, "results", |a| a.results.len() == 1);
    open_settings(&mut h);
    h.state_mut().settings_draft.as_mut().unwrap().providers.get_mut("wallhaven").unwrap().api_key = "by-keyboard".into();
    h.step();
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::S);
    h.step();
    h.step();
    assert_eq!(Config::load(&sb.paths).providers["wallhaven"].api_key, "by-keyboard");
}

/// A failed write is said out loud, and retried by the next periodic save.
#[test]
fn a_failed_save_is_not_reported_as_saved() {
    let sb = Sandbox::new("savefail", 1);
    let mut h = harness(&sb);
    wait_until(&mut h, "results", |a| a.results.len() == 1);
    let mut cfg = h.state().config.clone();
    cfg.thumb_size = 300.0;
    // A folder where the config file should be makes the write fail.
    std::fs::remove_file(sb.paths.config_file()).unwrap();
    std::fs::create_dir_all(sb.paths.config_file()).unwrap();
    assert!(!h.state_mut().apply_config(cfg));
    assert_ne!(h.state().saved_config, h.state().config, "still counts as unsaved");
}

/// Selecting a wallpaper opens the detail panel once; from then on nothing may change size by
/// itself. Content overflowing a side panel widens it, which gives the content more room to
/// overflow, so the panel crept wider frame by frame and the grid shrank with it.
#[test]
fn side_panels_keep_their_size() {
    let sb = Sandbox::new("creep", 3);
    let mut h = harness(&sb);
    wait_until(&mut h, "results", |a| a.results.len() == 3);
    let mut wp = h.state().results[0].clone();
    h.state_mut().actions.push(super::Action::SetStatus(wp.clone(), Status::Kept));
    h.state_mut().actions.push(super::Action::AddTag(wp.clone(), "a-rather-long-tag-name-for-a-chip, another-quite-long-tag-name".into()));
    h.state_mut().actions.push(super::Action::SwitchView(View::Library));
    h.step();
    h.step();

    let edges = |h: &Harness<'static, App>| {
        // The grid header starts where the sidebar ends, the wallpaper button where the detail panel starts.
        (h.get_by_label("1 wallpaper").rect().left(), h.get_by_label_contains("Set as wallpaper").rect().left())
    };

    // An ordinary wallpaper, the way a click selects it.
    h.state_mut().actions.push(super::Action::Select(wp.clone()));
    for _ in 0..4 {
        h.step();
    }
    let first = edges(&h);
    for frame in [30, 60, 120, 240] {
        while h.state().ctx.cumulative_frame_nr() < frame {
            h.step();
        }
        assert_eq!(edges(&h), first, "(sidebar edge, detail edge) moved by frame {frame}");
    }
    // The window's 1400, less the detail panel's default 360, plus its margin of 14 and separator.
    assert_eq!(first.1, 1055.0, "detail panel at its default width");

    // Everything that could push against the panel's width.
    let long = "averyveryveryverylongwordwithoutanyplacetobreakitverylongindeedandthenabitmorebesides";
    wp.title = Some(format!("{long} {long}"));
    wp.author = Some(long.into());
    wp.source = Some(format!("https://example.com/{long}"));
    wp.license = Some(long.into());
    wp.tags = vec![long.into(), "short".into()];
    wp.colors = vec!["#112233".into(); 30];
    wp.extra.insert("description".into(), format!("{long} {long}"));
    h.state_mut().selected = Some(wp);
    for _ in 0..60 {
        h.step();
    }
    assert_eq!(edges(&h), first, "long content must wrap or be cut, not widen the panel");

    // Dragging is what changes a panel's size, and that still works (and then stays put).
    let edge = egui::pos2(first.1 - 14.0, 450.0);
    let dropped = edge - egui::vec2(60.0, 0.0);
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    h.input_mut().events.push(egui::Event::PointerMoved(edge));
    h.step();
    h.input_mut().events.push(button(edge, true));
    h.step();
    for x in [10.0, 40.0, 60.0] {
        h.input_mut().events.push(egui::Event::PointerMoved(edge - egui::vec2(x, 0.0)));
        h.step();
    }
    h.input_mut().events.push(button(dropped, false));
    for _ in 0..4 {
        h.step();
    }
    let dragged = edges(&h);
    assert!((dragged.1 - (first.1 - 60.0)).abs() <= 2.0, "panel follows the drag: {dragged:?}");
    for _ in 0..60 {
        h.step();
    }
    assert_eq!(edges(&h), dragged);
}

/// A colourful stand-in for a wallpaper.
fn landscape(hue: f32) -> image::RgbImage {
    image::RgbImage::from_fn(960, 540, |x, y| {
        let (u, v) = (x as f32 / 960.0, y as f32 / 540.0);
        let hills = 0.62 + 0.10 * (u * 7.0 + hue * 9.0).sin() + 0.05 * (u * 19.0 + hue).sin();
        let channel = |shift: f32, level: f32| 127.5 * (1.0 + (hue * std::f32::consts::TAU + shift).sin()) * level;
        let level = if v > hills { 0.25 + 0.2 * (1.0 - v) } else { 0.55 + 0.45 * (1.0 - v) };
        image::Rgb([channel(0.0, level) as u8, channel(2.1, level) as u8, channel(4.2, level) as u8])
    })
}

/// Renders every screen to PNGs, to look at the UI without running it:
/// `WALLE_SHOTS=some/folder cargo test screenshots -- --ignored`
#[test]
#[ignore]
fn screenshots() {
    let dir = PathBuf::from(std::env::var("WALLE_SHOTS").expect("set WALLE_SHOTS to a folder"));
    std::fs::create_dir_all(&dir).unwrap();
    let sb = Sandbox::new("shots", 0);
    for i in 0..18 {
        landscape(i as f32 / 18.0).save(sb.images.join(format!("Scene {i:02}.jpg"))).unwrap();
    }
    let paths = sb.paths.clone();
    let mut h: Harness<'static, App> = Harness::builder()
        .with_size([1400.0, 900.0])
        .wgpu()
        .build_eframe(move |cc| App::new(cc, paths, super::StartOptions { tray: false, minimized: false, about: false, on_show: None }).expect("app"));
    wait_until(&mut h, "results", |a| a.results.len() == 18);
    wait_until(&mut h, "thumbnails", |a| a.hashes.len() >= 12);
    let settle = |h: &mut Harness<'static, App>| {
        for _ in 0..30 {
            h.step();
            std::thread::sleep(Duration::from_millis(15));
        }
    };
    let shot = |h: &mut Harness<'static, App>, name: &str| {
        settle(h);
        h.render().unwrap().save(dir.join(format!("{name}.png"))).unwrap();
    };
    let view = |h: &mut Harness<'static, App>, v: View| {
        h.state_mut().actions.push(super::Action::SwitchView(v));
        h.step();
        h.step();
    };

    let items = h.state().results.clone();
    for (i, w) in items.iter().enumerate() {
        let st = match i % 5 {
            0 => Some(Status::Kept),
            1 => Some(Status::Considering),
            2 if i > 5 => Some(Status::Rejected),
            _ => None,
        };
        if let Some(st) = st {
            h.state_mut().actions.push(super::Action::SetStatus(w.clone(), st));
        }
        if i % 5 == 0 {
            h.state_mut().actions.push(super::Action::Rate(w.clone(), 4));
            h.state_mut().actions.push(super::Action::AddTag(w.clone(), "calm, blue".into()));
        }
    }
    h.step();
    h.state_mut().toasts.push(super::theme::ToastKind::Success, "Wallpaper set: Scene 03");
    shot(&mut h, "01-browse");

    h.state_mut().actions.push(super::Action::Select(items[3].clone()));
    h.step();
    h.input_mut().events.push(egui::Event::PointerMoved(egui::pos2(420.0, 300.0)));
    shot(&mut h, "02-browse-selected-hover");
    h.input_mut().events.push(egui::Event::PointerGone);

    view(&mut h, View::Library);
    shot(&mut h, "03-library");
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    shot(&mut h, "04-library-multi");
    h.state_mut().multi.clear();
    view(&mut h, View::Considering);
    shot(&mut h, "05-considering");
    h.state_mut().selected = None;
    view(&mut h, View::History);
    shot(&mut h, "06-history-empty");
    view(&mut h, View::Settings);
    shot(&mut h, "07-settings");
    section_of_settings(&mut h, Section::Sources);
    shot(&mut h, "07b-settings-sources");
    section_of_settings(&mut h, Section::Appearance);
    h.state_mut().show_about = true;
    shot(&mut h, "08-about");
    h.state_mut().show_about = false;
    view(&mut h, View::Browse);
    h.state_mut().actions.push(super::Action::Lightbox(items[4].clone()));
    h.step();
    wait_until(&mut h, "full image", |a| a.textures.is_ready(&format!("full:{}", items[4].key())));
    shot(&mut h, "09-lightbox");
    h.state_mut().lightbox = None;
    h.state_mut().actions.push(super::Action::Compare(items[..4].to_vec()));
    shot(&mut h, "10-compare");
    h.state_mut().duel = None;

    let mut cfg = h.state().config.clone();
    cfg.theme = crate::config::Theme::Light;
    h.state_mut().apply_config(cfg);
    h.state_mut().actions.push(super::Action::Select(items[3].clone()));
    shot(&mut h, "11-light-browse");
    view(&mut h, View::Settings);
    shot(&mut h, "12-light-settings");
    h.state_mut().show_about = true;
    shot(&mut h, "13-light-about");
    h.state_mut().show_about = false;
    view(&mut h, View::Browse);
    h.state_mut().actions.push(super::Action::Compare(items[..4].to_vec()));
    shot(&mut h, "14-light-compare");
    h.state_mut().duel = None;

    let mut cfg = h.state().config.clone();
    cfg.theme = crate::config::Theme::Dark;
    cfg.accent = [240, 170, 50];
    h.state_mut().apply_config(cfg);
    h.state_mut().selected = None;
    h.set_size(egui::vec2(820.0, 520.0));
    shot(&mut h, "15-narrow-amber");
}
