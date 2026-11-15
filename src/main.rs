#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
#[cfg(target_os = "linux")]
mod background;
mod build_info;
mod config;
mod dedupe;
mod desktop;
mod effects;
mod icon;
mod icon_art;
mod library;
mod model;
#[cfg(windows)]
mod native_window;
mod net;
mod providers;
mod rotation;
mod setter;
mod single_instance;
#[cfg(windows)]
mod tray;
#[cfg(target_os = "linux")]
#[path = "tray_linux.rs"]
mod tray;
mod ui;
mod worker;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "walle=info".into()),
        )
        .init();

    // A second launch just brings the running instance to the front.
    let on_show = match single_instance::claim() {
        single_instance::Claim::Secondary => {
            tracing::info!("walle is already running; asked it to show its window");
            return Ok(());
        }
        single_instance::Claim::Primary(cb) => cb,
    };

    let paths = config::Paths::discover()?;
    tracing::info!("config: {}", paths.config_file().display());
    tracing::info!("data:   {}", paths.data_dir.display());
    let cfg = config::Config::load(&paths);
    if !config::Paths::is_isolated() && autostart::is_stale() {
        // walle moved since autostart was enabled: point the entry at this executable.
        let _ = autostart::set_enabled(true);
    }

    let minimized = std::env::args().any(|a| a == autostart::FLAG);
    #[cfg(target_os = "linux")]
    let about = {
        // Without a window: at login, or after the window was closed to the tray.
        let mut about = std::env::args().any(|a| a == background::ABOUT_FLAG);
        if minimized || std::env::args().any(|a| a == background::FLAG) {
            match background::run(&paths, &on_show) {
                Ok(background::Outcome::Quit) => return Ok(()),
                // A window cannot be added to this process afterwards (and the worker should
                // not live twice): start afresh.
                Ok(background::Outcome::Open { about }) => {
                    return Err(background::restart(if about { &[background::ABOUT_FLAG] } else { &[] }));
                }
                Err(e) => {
                    tracing::warn!("no tray icon here, opening the window: {e:#}");
                    about = false;
                }
            }
        }
        about
    };
    #[cfg(not(target_os = "linux"))]
    let about = false;
    let has_tray = cfg!(any(windows, target_os = "linux"));
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("walle")
        .with_app_id("walle")
        // walle draws its own titlebar and resize edges (ui/titlebar.rs).
        .with_decorations(false)
        .with_icon(icon::window_icon(cfg.accent))
        .with_inner_size([1400.0, 900.0])
        .with_min_inner_size([800.0, 500.0]);
    if minimized && cfg!(windows) {
        viewport = viewport.with_visible(false);
    }
    let persistence_path = config::Paths::is_isolated().then(|| paths.data_dir.join("app.ron"));
    let options = eframe::NativeOptions { viewport, persistence_path, ..Default::default() };
    let start = ui::StartOptions { tray: has_tray, minimized, about, on_show: Some(on_show) };
    eframe::run_native("walle", options, Box::new(move |cc| Ok(Box::new(ui::App::new(cc, paths, start)?))))
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    // The window was closed to the tray: go on without one.
    #[cfg(target_os = "linux")]
    if ui::TO_BACKGROUND.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(background::restart(&[background::FLAG]));
    }
    Ok(())
}
