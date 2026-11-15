//! walle without a window (Linux): the tray icon, rotation and the effects that follow the
//! time of day go on, and nothing else is loaded. Wayland cannot hide a window, so closing
//! "to the tray" ends the window for good and the program starts itself again in this mode;
//! opening it from the tray starts it again with a window.
use std::{collections::HashMap, path::PathBuf, sync::mpsc, time::Duration};

use anyhow::Result;

use crate::{
    config::{Config, Paths},
    library::Library,
    single_instance::OnShow,
    tray::Tray,
    ui::Remote,
    worker::{self, Command, Event},
};

/// Start in this mode (set by walle itself when its window closes to the tray).
pub const FLAG: &str = "--background";
/// Open the window with the About box (from the tray menu).
pub const ABOUT_FLAG: &str = "--about";

/// Why the windowless mode ended.
pub enum Outcome {
    /// The window is wanted: start again with one.
    Open { about: bool },
    Quit,
}

/// Run until the window is asked for or walle is told to quit. An error means this mode
/// cannot work here (no session bus for the tray icon); the caller opens the window instead.
pub fn run(paths: &Paths, on_show: &OnShow) -> Result<Outcome> {
    let mut config = Config::load(paths);
    let (tx, rx) = mpsc::channel::<Remote>();
    let tray = Tray::new(|| {}, tx.clone(), config.accent, config.rotation.enabled)?;
    // A second launch of walle asks for the window.
    *on_show.lock().unwrap() = Some(Box::new(move || {
        let _ = tx.send(Remote::Show);
    }));
    let lib = Library::open(&paths.db_file())?;
    // The worker wakes a UI through this context; here nothing listens and events are polled.
    let worker = worker::spawn(egui::Context::default(), paths.clone(), config.clone());
    tracing::info!("running in the tray");
    let mut downloaded: HashMap<String, PathBuf> = HashMap::new();
    loop {
        while let Ok(event) = worker.events.try_recv() {
            match event {
                Event::Downloaded { key, result: Ok(path) } => {
                    downloaded.insert(key, path);
                }
                Event::WallpaperSet { key, result: Ok(()), wp: Some(wp), .. } => {
                    // What the window would do: history, and where the download is.
                    if let Err(e) = lib.record_set(&wp) {
                        tracing::warn!("history: {e:#}");
                    }
                    if let Some(path) = downloaded.remove(&key)
                        && !wp.is_local()
                    {
                        let _ = lib.set_local_path(&wp, Some(&path));
                    }
                    tray.set_tooltip(&format!("walle — {}", wp.display_title()));
                }
                Event::WallpaperSet { result: Err(e), .. } => tracing::warn!("could not set wallpaper: {e}"),
                Event::RotationFailed(e) => tracing::warn!("rotation: {e}"),
                _ => {}
            }
        }
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(Remote::Show) => return Ok(Outcome::Open { about: false }),
            Ok(Remote::About) => return Ok(Outcome::Open { about: true }),
            Ok(Remote::Quit) => return Ok(Outcome::Quit),
            Ok(Remote::Next) => worker.send(Command::RotateNow),
            Ok(Remote::ToggleRotation) => {
                config.rotation.enabled = !config.rotation.enabled;
                if let Err(e) = config.save(paths) {
                    tracing::warn!("saving settings: {e:#}");
                }
                tray.set_rotation(config.rotation.enabled);
                worker.send(Command::Reconfigure(config.clone()));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(Outcome::Quit),
        }
    }
}

/// Replace this process with a fresh walle (same pid, so whoever started it still has it).
/// Only returns if that failed.
pub fn restart(args: &[&str]) -> anyhow::Error {
    use std::os::unix::process::CommandExt;
    match std::env::current_exe() {
        Ok(exe) => anyhow::Error::from(std::process::Command::new(exe).args(args).exec()).context("restarting walle"),
        Err(e) => anyhow::Error::from(e).context("finding walle's own executable"),
    }
}
