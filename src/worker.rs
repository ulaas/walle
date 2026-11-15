//! Background worker: owns the tokio runtime, providers and all network/disk/decode work.
use std::{
    path::{Path, PathBuf},
    sync::{Arc, RwLock, mpsc},
};

use anyhow::{Context, Result};
use tokio::sync::{Semaphore, mpsc as tmpsc};

use crate::{
    config::{Config, Paths},
    desktop::{Display, Entry, Shown, Target},
    effects,
    model::{FitMode, SearchPage, SearchQuery, Wallpaper},
    net::{self, sanitize},
    providers::{self, Registry, local},
    rotation,
    setter,
};

/// What to do once a full-resolution image is on disk.
#[derive(Clone, Debug)]
pub enum After {
    Nothing,
    SetWallpaper(FitMode, Target),
    /// Decode it into a texture with this key.
    Preview(String),
}

pub enum Command {
    Search { req: u64, provider: String, query: SearchQuery, page: u32 },
    Details { provider: String, id: String },
    /// Fetch (disk-cached) and decode an image into a texture. `url` may be a local path.
    /// If fetching fails and `fallback` exists (e.g. the downloaded original), decode that instead.
    LoadImage { tex_key: String, url: String, max_dim: u32, fallback: Option<PathBuf> },
    /// Decode a local file into a texture (full size preview).
    LoadLocal { tex_key: String, path: PathBuf, max_dim: u32 },
    Download { wp: Wallpaper, after: After },
    SetLocal { key: String, path: PathBuf, fit: FitMode, target: Target },
    /// Ask the desktop for its displays and for what is on them (answers with
    /// `Event::Displays` and `Event::OnDesktop`).
    Desktop,
    /// Turn dropped/imported files into wallpapers.
    Import(Vec<PathBuf>),
    Reconfigure(Config),
    /// Change the wallpaper now using the rotation settings (resets the timer).
    RotateNow,
    /// Shrink the thumbnail cache to the configured limit (or clear it).
    PruneCache { clear: bool },
    Collections { provider: String },
    /// Fetch every page of a collection (bounded) for import into the library.
    ImportCollection { provider: String, query: SearchQuery, label: String },
    /// Size and presence of the kept files (touches every file, so never on the UI thread).
    LibraryStats,
}

/// An image waiting for a decode slot.
enum ImageJob {
    Cached { tex_key: String, url: String, max_dim: u32, fallback: Option<PathBuf> },
    File { tex_key: String, path: PathBuf, max_dim: u32 },
}

pub enum Event {
    Search { req: u64, result: Result<SearchPage, String> },
    Details { key: String, result: Result<Wallpaper, String> },
    Image { tex_key: String, result: Result<egui::ColorImage, String> },
    Downloaded { key: String, result: Result<PathBuf, String> },
    /// `wp` is set when the worker chose the wallpaper itself (rotation).
    WallpaperSet { key: String, target: Target, result: Result<(), String>, wp: Option<Wallpaper> },
    /// The displays that can each have their own wallpaper (empty: one for the whole desktop).
    Displays(Vec<Display>),
    /// Keys of the wallpapers that are on the desktop now.
    OnDesktop(Vec<String>),
    Imported(Vec<Wallpaper>),
    RotationScheduled(Option<std::time::SystemTime>),
    RotationFailed(String),
    CacheSize(u64),
    /// Perceptual hash of a wallpaper's thumbnail.
    Hash { key: String, hash: u64 },
    Collections { provider: String, result: Result<Vec<providers::Collection>, String> },
    CollectionImported { label: String, result: Result<Vec<Wallpaper>, String> },
    Stats(crate::library::Stats),
}

struct Shared {
    paths: Paths,
    config: RwLock<Config>,
    registry: RwLock<Registry>,
    client: reqwest::Client,
    image_slots: Semaphore,
    /// Newest last. Slots take the newest job, so after a fast scroll the cards now on screen
    /// load first instead of waiting behind everything that was scrolled past.
    image_jobs: std::sync::Mutex<Vec<ImageJob>>,
    download_slots: Semaphore,
    events: mpsc::Sender<Event>,
    ctx: egui::Context,
    /// The worker's own library connection (SQLite WAL allows a second connection).
    lib: std::sync::Mutex<Option<crate::library::Library>>,
    rotation: std::sync::Mutex<rotation::State>,
    rotate_now: tokio::sync::Notify,
    /// What walle last put on the desktop. Held while the desktop is being changed, so two
    /// changes never interleave.
    shown: std::sync::Mutex<Shown>,
}

impl Shared {
    fn emit(&self, ev: Event) {
        let _ = self.events.send(ev);
        self.ctx.request_repaint();
    }
}

pub struct WorkerHandle {
    tx: tmpsc::UnboundedSender<Command>,
    pub events: mpsc::Receiver<Event>,
}

impl WorkerHandle {
    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }
}

pub fn spawn(ctx: egui::Context, paths: Paths, config: Config) -> WorkerHandle {
    let (tx, mut rx) = tmpsc::unbounded_channel::<Command>();
    let (etx, erx) = mpsc::channel();
    let lib = match crate::library::Library::open(&paths.db_file()) {
        Ok(l) => Some(l),
        Err(e) => {
            tracing::warn!("worker library connection: {e:#}");
            None
        }
    };
    let mut rot = rotation::State::default();
    rot.schedule(&config.rotation, std::time::Instant::now());
    let change_on_start = config.rotation.enabled && config.rotation.change_on_start;
    let shown = Shown::load(&paths);
    let shared = Arc::new(Shared {
        shown: std::sync::Mutex::new(shown),
        registry: RwLock::new(providers::build_registry(&config)),
        config: RwLock::new(config),
        paths,
        client: net::client(),
        image_slots: Semaphore::new(8),
        image_jobs: std::sync::Mutex::new(Vec::new()),
        download_slots: Semaphore::new(3),
        events: etx,
        ctx,
        lib: std::sync::Mutex::new(lib),
        rotation: std::sync::Mutex::new(rot),
        rotate_now: tokio::sync::Notify::new(),
    });
    std::thread::Builder::new()
        .name("walle-worker".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(async move {
                tokio::spawn(rotation_loop(shared.clone(), change_on_start));
                tokio::spawn(effects_loop(shared.clone()));
                // Also periodically: walle can stay in the tray for weeks.
                let prune = shared.clone();
                tokio::spawn(async move {
                    loop {
                        Box::pin(handle(prune.clone(), Command::PruneCache { clear: false })).await;
                        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                    }
                });
                while let Some(cmd) = rx.recv().await {
                    let sh = shared.clone();
                    // In order, not as a task of its own: the search that follows new settings
                    // must find the new API keys in place, not race to use the old ones.
                    if matches!(cmd, Command::Reconfigure(_)) {
                        handle(sh, cmd).await;
                    } else {
                        tokio::spawn(async move { handle(sh, cmd).await });
                    }
                }
            });
        })
        .expect("spawn worker");
    WorkerHandle { tx, events: erx }
}

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

async fn handle(sh: Arc<Shared>, cmd: Command) {
    match cmd {
        Command::Search { req, provider, query, page } => {
            let p = sh.registry.read().unwrap().get(&provider).cloned();
            let result = match p {
                Some(p) => p.search(&query, page).await.map_err(err),
                None => Err(format!("provider '{provider}' is not enabled")),
            };
            sh.emit(Event::Search { req, result });
        }
        Command::Details { provider, id } => {
            let key = crate::model::make_key(&provider, &id);
            let p = sh.registry.read().unwrap().get(&provider).cloned();
            let result = match p {
                Some(p) => p.details(&id).await.map_err(err),
                None => Err(format!("provider '{provider}' is not enabled")),
            };
            sh.emit(Event::Details { key, result });
        }
        Command::LoadImage { tex_key, url, max_dim, fallback } => {
            sh.image_jobs.lock().unwrap().push(ImageJob::Cached { tex_key, url, max_dim, fallback });
            run_next_image(&sh).await;
        }
        Command::LoadLocal { tex_key, path, max_dim } => {
            sh.image_jobs.lock().unwrap().push(ImageJob::File { tex_key, path, max_dim });
            run_next_image(&sh).await;
        }
        Command::LibraryStats => {
            let worker = sh.clone();
            let stats = tokio::task::spawn_blocking(move || {
                let guard = worker.lib.lock().unwrap();
                guard.as_ref().and_then(|l| l.stats().ok())
            })
            .await;
            if let Ok(Some(s)) = stats {
                sh.emit(Event::Stats(s));
            }
        }
        Command::Download { wp, after } => {
            let key = wp.key();
            let result = {
                let _permit = sh.download_slots.acquire().await;
                download(&sh, &wp).await
            };
            match result {
                Ok((path, fresh)) => {
                    if fresh {
                        let p = sh.registry.read().unwrap().get(&wp.provider).cloned();
                        if let Some(p) = p
                            && let Err(e) = p.track_download(&wp).await
                        {
                            tracing::warn!("download tracking for {key} failed: {e:#}");
                        }
                    }
                    sh.emit(Event::Downloaded { key: key.clone(), result: Ok(path.clone()) });
                    match after {
                        After::Nothing => {}
                        After::SetWallpaper(fit, target) => set_wallpaper(&sh, key, path, fit, target).await,
                        After::Preview(tex_key) => {
                            let _permit = sh.image_slots.acquire().await;
                            let result = decode_file(path, 4096).await.map_err(err);
                            sh.emit(Event::Image { tex_key, result });
                        }
                    }
                }
                Err(e) => {
                    let msg = err(e);
                    if let After::Preview(tex_key) = after {
                        sh.emit(Event::Image { tex_key, result: Err(msg.clone()) });
                    }
                    sh.emit(Event::Downloaded { key, result: Err(msg) });
                }
            }
        }
        Command::SetLocal { key, path, fit, target } => set_wallpaper(&sh, key, path, fit, target).await,
        Command::Desktop => {
            let worker = sh.clone();
            let found = tokio::task::spawn_blocking(move || {
                let custom = worker.config.read().unwrap().linux_set_command.clone();
                let displays = desktop_displays(&worker.paths, &custom);
                let mut shown = worker.shown.lock().unwrap();
                shown.retain_current(&desktop_current());
                let mut keys = shown.keys(&displays);
                if keys.is_empty() {
                    // Not put there by walle (or before it kept track): known if it is a
                    // file of the library.
                    let guard = worker.lib.lock().unwrap();
                    if let Some(lib) = guard.as_ref() {
                        keys = desktop_current().iter().filter_map(|p| lib.key_for_path(p).ok().flatten()).collect();
                    }
                }
                (displays, keys)
            })
            .await;
            if let Ok((displays, keys)) = found {
                sh.emit(Event::Displays(displays));
                sh.emit(Event::OnDesktop(keys));
            }
        }
        Command::Import(paths) => {
            let items = tokio::task::spawn_blocking(move || {
                let mut files = vec![];
                for p in paths {
                    if p.is_dir() {
                        collect_images(&p, 0, &mut files);
                    } else if local::is_image(&p) {
                        files.push(p);
                    }
                }
                files.iter().map(|p| local::wallpaper_for(p)).collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            sh.emit(Event::Imported(items));
        }
        Command::Reconfigure(cfg) => {
            *sh.registry.write().unwrap() = providers::build_registry(&cfg);
            let rotation_changed = sh.config.read().unwrap().rotation != cfg.rotation;
            let look_changed = sh.config.read().unwrap().effects != cfg.effects;
            if rotation_changed {
                let mut st = sh.rotation.lock().unwrap();
                st.schedule(&cfg.rotation, std::time::Instant::now());
                sh.emit(Event::RotationScheduled(st.next_wallclock()));
            }
            *sh.config.write().unwrap() = cfg;
            if look_changed {
                // What is on the desktop gets the new look at once.
                render_again(&sh).await;
            }
        }
        Command::RotateNow => sh.rotate_now.notify_one(),
        Command::Collections { provider } => {
            let p = sh.registry.read().unwrap().get(&provider).cloned();
            let result = match p {
                Some(p) => p.collections().await.map_err(err),
                None => Err(format!("provider '{provider}' is not enabled")),
            };
            sh.emit(Event::Collections { provider, result });
        }
        Command::ImportCollection { provider, query, label } => {
            let p = sh.registry.read().unwrap().get(&provider).cloned();
            let result = match p {
                Some(p) => fetch_all(p.as_ref(), &query, 50).await.map_err(err),
                None => Err(format!("provider '{provider}' is not enabled")),
            };
            sh.emit(Event::CollectionImported { label, result });
        }
        Command::PruneCache { clear } => {
            let dir = sh.paths.thumbs_dir();
            let limit = if clear { 0 } else { sh.config.read().unwrap().cache_limit_mb * 1024 * 1024 };
            let size = tokio::task::spawn_blocking(move || prune_dir(&dir, limit)).await.unwrap_or(0);
            sh.emit(Event::CacheSize(size));
        }
    }
}

/// Wait for a decode slot, then load the most recently requested image. Every job comes with
/// exactly one call of this, so all of them run eventually.
async fn run_next_image(sh: &Shared) {
    let _permit = sh.image_slots.acquire().await;
    let job = sh.image_jobs.lock().unwrap().pop();
    let Some(job) = job else { return };
    match job {
        ImageJob::Cached { tex_key, url, max_dim, fallback } => {
            let mut result = load_cached(sh, &tex_key, &url, max_dim).await;
            if result.is_err()
                && let Some(f) = fallback.filter(|f| f.exists())
            {
                result = decode_file(f, max_dim).await;
            }
            if let (Ok(img), Some(key)) = (&result, tex_key.strip_prefix("thumb:")) {
                sh.emit(Event::Hash { key: key.to_string(), hash: crate::dedupe::dhash(img) });
            }
            sh.emit(Event::Image { tex_key, result: result.map_err(err) });
        }
        ImageJob::File { tex_key, path, max_dim } => {
            let result = decode_file(path, max_dim).await.map_err(err);
            sh.emit(Event::Image { tex_key, result });
        }
    }
}

/// Delete the oldest files in `dir` until it is at most `limit` bytes. Returns the final size.
pub fn prune_dir(dir: &Path, limit: u64) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    let mut files: Vec<(PathBuf, u64, std::time::SystemTime)> = rd
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| (e.path(), m.len(), m.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH)))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort_by_key(|f| f.2);
    for (p, len, _) in files {
        if total <= limit {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total -= len;
        }
    }
    total
}

async fn rotation_loop(sh: Arc<Shared>, change_on_start: bool) {
    sh.emit(Event::RotationScheduled(sh.rotation.lock().unwrap().next_wallclock()));
    let mut force = change_on_start;
    loop {
        if !force {
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(20)) => {}
                _ = sh.rotate_now.notified() => force = true,
            }
        }
        let due = sh.rotation.lock().unwrap().due(std::time::Instant::now());
        if !(force || due) {
            continue;
        }
        force = false;
        if let Err(e) = rotate_once(&sh).await {
            tracing::warn!("rotation: {e:#}");
            sh.emit(Event::RotationFailed(format!("{e:#}")));
        }
        let cfg = sh.config.read().unwrap().rotation.clone();
        let mut st = sh.rotation.lock().unwrap();
        st.schedule(&cfg, std::time::Instant::now());
        sh.emit(Event::RotationScheduled(st.next_wallclock()));
    }
}

/// Pick a wallpaper from the rotation source, download it if needed and set it.
async fn rotate_once(sh: &Arc<Shared>) -> Result<()> {
    use crate::config::RotationSource;
    let cfg = sh.config.read().unwrap().clone();
    let rot = cfg.rotation.clone();
    let lib_query = |f: crate::library::LibraryFilter| -> Result<Vec<Wallpaper>> {
        let guard = sh.lib.lock().unwrap();
        let lib = guard.as_ref().context("library unavailable")?;
        lib.list(&f)
    };
    let candidates: Vec<Wallpaper> = match &rot.source {
        RotationSource::SavedSearch(id) => {
            let saved = {
                let guard = sh.lib.lock().unwrap();
                let lib = guard.as_ref().context("library unavailable")?;
                lib.saved_searches()?.into_iter().find(|s| s.id == *id).context("saved search no longer exists")?
            };
            let p = sh.registry.read().unwrap().get(&saved.provider).cloned().context("saved search source is disabled")?;
            let rejected = {
                let guard = sh.lib.lock().unwrap();
                guard.as_ref().map(|l| l.rejected_keys().unwrap_or_default()).unwrap_or_default()
            };
            p.search(&saved.query, 1).await?.items.into_iter().filter(|w| !rejected.contains(&w.key())).collect()
        }
        RotationSource::Provider(id) => {
            let p = sh.registry.read().unwrap().get(id).cloned().context("rotation source is disabled")?;
            let rejected = {
                let guard = sh.lib.lock().unwrap();
                guard.as_ref().map(|l| l.rejected_keys().unwrap_or_default()).unwrap_or_default()
            };
            p.search(&SearchQuery::default(), 1).await?.items.into_iter().filter(|w| !rejected.contains(&w.key())).collect()
        }
        _ => lib_query(rotation::library_filter(&rot).unwrap())?,
    };
    // One for the whole desktop, or one for each display.
    let displays = if rot.per_display {
        let custom = cfg.linux_set_command.clone();
        let paths = sh.paths.clone();
        tokio::task::spawn_blocking(move || desktop_displays(&paths, &custom)).await.unwrap_or_default()
    } else {
        vec![]
    };
    let targets: Vec<Target> =
        if displays.len() > 1 { displays.iter().map(|d| Target::Display(d.name.clone())).collect() } else { vec![Target::All] };
    let on_desktop = sh.shown.lock().unwrap().keys(&displays);
    let mut jobs = vec![];
    let mut picked: Vec<(Wallpaper, Target)> = vec![];
    for target in targets {
        // Not what this display shows now, and not what another display just got.
        let avoid: Option<String> = match &target {
            Target::All => on_desktop.first().cloned(),
            Target::Display(name) => sh.shown.lock().unwrap().on(name).map(|e| e.key.clone()),
        };
        let wp = sh
            .rotation
            .lock()
            .unwrap()
            .pick(&candidates, &rot, avoid.as_deref())
            .with_context(|| format!("nothing to rotate from ({})", rotation::describe(&rot.source)))?;
        let (path, fresh) = {
            let _permit = sh.download_slots.acquire().await;
            download(sh, &wp).await?
        };
        if fresh {
            sh.emit(Event::Downloaded { key: wp.key(), result: Ok(path.clone()) });
        }
        sh.rotation.lock().unwrap().remember(wp.key());
        jobs.push(Job { target: target.clone(), key: wp.key(), source: path, fit: cfg.fit_mode });
        picked.push((wp, target));
    }
    let result = place_off_thread(sh, jobs).await;
    for (wp, target) in picked {
        sh.emit(Event::WallpaperSet { key: wp.key(), target, result: result.clone().map(|_| ()), wp: Some(wp) });
    }
    match result {
        Ok(keys) => {
            sh.emit(Event::OnDesktop(keys));
            Ok(())
        }
        Err(e) => Err(anyhow::anyhow!(e)),
    }
}

fn collect_images(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    if depth > 8 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_images(&p, depth + 1, out);
        } else if local::is_image(&p) {
            out.push(p);
        }
    }
}

async fn set_wallpaper(sh: &Arc<Shared>, key: String, path: PathBuf, fit: FitMode, target: Target) {
    let job = Job { target: target.clone(), key: key.clone(), source: path, fit };
    let result = place_off_thread(sh, vec![job]).await;
    let on_desktop = result.clone().ok();
    sh.emit(Event::WallpaperSet { key, target, result: result.map(|_| ()), wp: None });
    if let Some(keys) = on_desktop {
        sh.emit(Event::OnDesktop(keys));
    }
}

/// One wallpaper to put on the desktop.
struct Job {
    target: Target,
    key: String,
    /// The original image.
    source: PathBuf,
    fit: FitMode,
}

/// The keys of what is on the desktop afterwards.
async fn place_off_thread(sh: &Arc<Shared>, jobs: Vec<Job>) -> Result<Vec<String>, String> {
    let worker = sh.clone();
    tokio::task::spawn_blocking(move || place(&worker, jobs)).await.map_err(|e| e.to_string()).and_then(|r| r.map_err(err))
}

/// Render the jobs with the current effects, put them on the desktop in one change and
/// remember them. Returns the keys of what is on the desktop afterwards. Blocking: runs off
/// the async threads.
fn place(sh: &Shared, jobs: Vec<Job>) -> Result<Vec<String>> {
    let (custom, fx) = {
        let cfg = sh.config.read().unwrap();
        (cfg.linux_set_command.clone(), cfg.effects.clone())
    };
    // One change of the desktop at a time.
    let mut shown = sh.shown.lock().unwrap();
    let displays = desktop_displays(&sh.paths, &custom);
    if jobs.is_empty() {
        return Ok(shown.keys(&displays));
    }
    let level = fx.night.level_now();
    let look = fx.look(level);
    let dir = effects::render_dir(&sh.paths.data_dir);
    let mut placements = vec![];
    let mut entries = vec![];
    for job in jobs {
        let mut placed = vec![];
        let mut put = |target: Target, r: effects::Rendered| {
            placed.push(r.path.clone());
            placements.push(setter::Placement { target, path: r.path, fit: r.fit });
        };
        match &job.target {
            Target::All if displays.is_empty() => put(Target::All, effects::render_one(&job.source, &look, job.fit, None, &dir)?),
            Target::All => {
                let span = job.fit == FitMode::Span && displays.len() > 1;
                if look.is_plain() && !span {
                    // The desktop scales the original for each display itself.
                    put(Target::All, effects::render_one(&job.source, &look, job.fit, displays.first(), &dir)?);
                } else {
                    let parts = if span {
                        effects::render_span(&job.source, &look, &displays, &dir)?
                    } else {
                        let one = |d: &Display| Ok((d.name.clone(), effects::render_one(&job.source, &look, job.fit, Some(d), &dir)?));
                        displays.iter().map(one).collect::<Result<Vec<_>>>()?
                    };
                    // For all first, which clears what single displays had (and is what a
                    // display plugged in later shows); then each display's own rendering.
                    if let Some((_, first)) = parts.first() {
                        put(Target::All, first.clone());
                    }
                    for (name, r) in parts {
                        put(Target::Display(name), r);
                    }
                }
            }
            Target::Display(name) => {
                let d = displays
                    .iter()
                    .find(|d| d.name.eq_ignore_ascii_case(name))
                    .with_context(|| format!("display {name} is not connected (or this desktop has one wallpaper for all displays)"))?;
                put(Target::Display(d.name.clone()), effects::render_one(&job.source, &look, job.fit, Some(d), &dir)?);
            }
        }
        entries.push((job.target, Entry { key: job.key, source: job.source, fit: job.fit, placed }));
    }
    desktop_apply(&placements, &custom)?;
    for (target, entry) in entries {
        shown.record(&target, entry);
    }
    shown.night_level = level;
    shown.save(&sh.paths);
    // Rendered pictures nothing shows any more.
    effects::forget_all_but(&dir, &shown.files());
    Ok(shown.keys(&displays))
}

/// Put what walle has on the desktop there again, rendered with the effects as they are now.
/// What someone else replaced in the meantime is left alone.
async fn render_again(sh: &Arc<Shared>) {
    let worker = sh.clone();
    let result = tokio::task::spawn_blocking(move || {
        let jobs = {
            let mut shown = worker.shown.lock().unwrap();
            shown.retain_current(&desktop_current());
            let job = |target: Target, e: &Entry| Job { target, key: e.key.clone(), source: e.source.clone(), fit: e.fit };
            let custom = worker.config.read().unwrap().linux_set_command.clone();
            let connected = desktop_displays(&worker.paths, &custom);
            let mut jobs: Vec<Job> = shown.all.iter().map(|e| job(Target::All, e)).collect();
            jobs.extend(
                shown
                    .displays
                    .iter()
                    .filter(|(name, _)| connected.iter().any(|d| d.name.eq_ignore_ascii_case(name)))
                    .map(|(name, e)| job(Target::Display(name.clone()), e)),
            );
            jobs.retain(|j| j.source.is_file());
            jobs
        };
        place(&worker, jobs)
    })
    .await;
    match result {
        Ok(Ok(keys)) => sh.emit(Event::OnDesktop(keys)),
        Ok(Err(e)) => tracing::warn!("rendering the wallpaper again: {e:#}"),
        Err(e) => tracing::warn!("rendering the wallpaper again: {e}"),
    }
}

/// Effects that change with the time of day: when the night level moves, what is on the
/// desktop is rendered again. Also right at the start, when walle takes over from the
/// pictures it left behind.
async fn effects_loop(sh: Arc<Shared>) {
    loop {
        let due = {
            let night = sh.config.read().unwrap().effects.night.clone();
            let shown = sh.shown.lock().unwrap();
            !shown.is_empty() && shown.night_level != night.level_now()
        };
        if due {
            render_again(&sh).await;
        }
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}

fn is_remote(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Load an image via the on-disk thumbnail cache. Large sources (local originals, big
/// previews) are cached as downscaled JPEGs so the next load is cheap.
async fn load_cached(sh: &Shared, tex_key: &str, url: &str, max_dim: u32) -> Result<egui::ColorImage> {
    // By source too: if a wallpaper's image moves, what was cached for it no longer applies.
    let cache = sh.paths.thumbs_dir().join(net::cache_file_name(&format!("{tex_key}|{url}")));
    if !is_stale_for_local_source(&cache, url).await
        && let Ok(b) = tokio::fs::read(&cache).await
        && !b.is_empty()
    {
        let cached = tokio::task::spawn_blocking(move || decode(&b, max_dim)).await?;
        if cached.is_ok() {
            return cached;
        }
    }
    let bytes = if is_remote(url) {
        net::get_bytes(&sh.client, url).await?
    } else {
        tokio::fs::read(url).await.with_context(|| format!("reading {url}"))?
    };
    let small_source = is_remote(url) && bytes.len() < 400_000;
    tokio::task::spawn_blocking(move || {
        let img = decode_dynamic(&bytes, max_dim)?;
        if small_source {
            let _ = std::fs::write(&cache, &bytes);
        } else {
            let mut out = std::io::Cursor::new(Vec::new());
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88);
            if img.to_rgb8().write_with_encoder(enc).is_ok() {
                let _ = std::fs::write(&cache, out.into_inner());
            }
        }
        Ok(to_color_image(&img))
    })
    .await?
}

/// A local file edited since it was cached must be read again.
async fn is_stale_for_local_source(cache: &Path, url: &str) -> bool {
    if is_remote(url) {
        return false;
    }
    let modified = |m: std::fs::Metadata| m.modified().ok();
    let source = tokio::fs::metadata(url).await.ok().and_then(modified);
    let cached = tokio::fs::metadata(cache).await.ok().and_then(modified);
    matches!((source, cached), (Some(s), Some(c)) if s > c)
}

async fn decode_file(path: PathBuf, max_dim: u32) -> Result<egui::ColorImage> {
    tokio::task::spawn_blocking(move || {
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        decode(&bytes, max_dim)
    })
    .await?
}

fn decode_dynamic(bytes: &[u8], max_dim: u32) -> Result<image::DynamicImage> {
    let img = image::load_from_memory(bytes).context("decoding image")?;
    Ok(if img.width() > max_dim || img.height() > max_dim {
        // Area averaging: as good as a filter when shrinking this much, and far faster.
        img.thumbnail(max_dim, max_dim)
    } else {
        img
    })
}

fn to_color_image(img: &image::DynamicImage) -> egui::ColorImage {
    let rgba = img.to_rgba8();
    egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw())
}

fn decode(bytes: &[u8], max_dim: u32) -> Result<egui::ColorImage> {
    decode_dynamic(bytes, max_dim).map(|i| to_color_image(&i))
}

pub fn local_path_for(paths: &Paths, cfg: &Config, wp: &Wallpaper) -> PathBuf {
    if wp.is_local() {
        return PathBuf::from(&wp.full_url);
    }
    paths
        .wallpapers_dir(cfg)
        .join(sanitize(&wp.provider))
        .join(format!("{}.{}", sanitize(&wp.remote_id), wp.extension()))
}

/// Download the full image. Returns the path and whether it was freshly fetched.
async fn download(sh: &Shared, wp: &Wallpaper) -> Result<(PathBuf, bool)> {
    let target = {
        let cfg = sh.config.read().unwrap();
        local_path_for(&sh.paths, &cfg, wp)
    };
    if is_nonempty(&target).await {
        return Ok((target, false));
    }
    if wp.is_local() {
        anyhow::bail!("file not found: {}", target.display());
    }
    if let Some(dir) = target.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let bytes = net::get_bytes(&sh.client, &wp.full_url).await?;
    // Reject HTML error pages served with 200.
    image::guess_format(&bytes).context("downloaded file is not an image")?;
    // Unique per download: the same wallpaper can be fetched twice at once (preview + set).
    static NEXT_TMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT_TMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = target.with_extension(format!("{n}.part"));
    tokio::fs::write(&tmp, &bytes).await?;
    if let Err(e) = tokio::fs::rename(&tmp, &target).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        // Losing the race to an identical download is fine.
        if !is_nonempty(&target).await {
            return Err(e.into());
        }
    }
    Ok((target, true))
}

async fn is_nonempty(p: &Path) -> bool {
    tokio::fs::metadata(p).await.map(|m| m.len() > 0).unwrap_or(false)
}

/// Wallpapers "set" during tests (tests must never touch the real desktop).
#[cfg(test)]
pub static TEST_SET_LOG: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());
/// What tests set, with where it went.
#[cfg(test)]
pub static TEST_PLACED: std::sync::Mutex<Vec<setter::Placement>> = std::sync::Mutex::new(Vec::new());
/// The displays a test pretends the desktop has, by the data folder of its sandbox (tests run
/// side by side).
#[cfg(test)]
pub static TEST_DISPLAYS: std::sync::Mutex<Vec<(PathBuf, Vec<Display>)>> = std::sync::Mutex::new(Vec::new());

#[cfg(not(test))]
fn desktop_apply(placements: &[setter::Placement], custom: &str) -> Result<()> {
    setter::apply(placements, custom)
}

#[cfg(not(test))]
fn desktop_displays(_paths: &Paths, custom: &str) -> Vec<Display> {
    setter::displays(custom)
}

#[cfg(not(test))]
fn desktop_current() -> Vec<PathBuf> {
    setter::current_wallpapers()
}

#[cfg(test)]
fn desktop_apply(placements: &[setter::Placement], _custom: &str) -> Result<()> {
    TEST_SET_LOG.lock().unwrap().extend(placements.iter().map(|p| p.path.clone()));
    TEST_PLACED.lock().unwrap().extend(placements.iter().cloned());
    Ok(())
}

#[cfg(test)]
fn desktop_displays(paths: &Paths, _custom: &str) -> Vec<Display> {
    let all = TEST_DISPLAYS.lock().unwrap();
    all.iter().find(|(dir, _)| *dir == paths.data_dir).map(|(_, d)| d.clone()).unwrap_or_default()
}

#[cfg(test)]
fn desktop_current() -> Vec<PathBuf> {
    vec![]
}

/// Fetch up to `max_pages` pages of a query.
async fn fetch_all(p: &dyn providers::Provider, query: &SearchQuery, max_pages: u32) -> Result<Vec<Wallpaper>> {
    let mut out = vec![];
    let mut seen = std::collections::HashSet::new();
    for page in 1..=max_pages {
        let res = p.search(query, page).await?;
        let more = res.has_more();
        out.extend(res.items.into_iter().filter(|w| seen.insert(w.key())));
        if !more {
            break;
        }
    }
    Ok(out)
}
