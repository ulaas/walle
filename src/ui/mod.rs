mod about;
mod compare;
mod detail;
mod grid;
pub mod icons;
mod lightbox;
mod pickers;
mod settings;
mod sidebar;
mod textures;
pub mod theme;
mod titlebar;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
};

use egui::{Color32, CornerRadius, FontId, Key, Margin, RichText, Stroke, vec2};

use crate::{
    config::{Config, Paths},
    library::{Entry, Library, LibraryFilter, SavedSearch, Stats},
    model::{SearchQuery, Status, Wallpaper},
    providers::{self, ProviderInfo},
    worker::{self, After, Command, Event, WorkerHandle},
};
use textures::TextureCache;
use theme::{Palette, ToastKind, Toasts, ph, semibold, with_icon};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum View {
    Browse,
    Considering,
    Library,
    Rejected,
    History,
    Settings,
}

impl View {
    fn label(self) -> &'static str {
        match self {
            View::Browse => "Browse",
            View::Considering => "Considering",
            View::Library => "Library",
            View::Rejected => "Rejected",
            View::History => "History",
            View::Settings => "Settings",
        }
    }
    fn icon(self) -> &'static str {
        match self {
            View::Browse => ph::MAGNIFYING_GLASS,
            View::Considering => ph::PUSH_PIN,
            View::Library => ph::IMAGES,
            View::Rejected => ph::TRASH,
            View::History => ph::CLOCK_COUNTER_CLOCKWISE,
            View::Settings => ph::GEAR_SIX,
        }
    }
}

/// UI intents collected while drawing and applied at the end of the frame.
pub enum Action {
    Select(Wallpaper),
    ToggleMulti(Wallpaper),
    RangeSelect(Wallpaper),
    SelectAll,
    ClearMulti,
    Lightbox(Wallpaper),
    SetStatus(Wallpaper, Status),
    BulkStatus(Status),
    Rate(Wallpaper, u8),
    SetWallpaper(Wallpaper),
    /// On one display only (where the desktop has a wallpaper per display).
    SetWallpaperOn(Wallpaper, String),
    Download(Wallpaper),
    BulkDownload,
    OpenUrl(String),
    OpenPath(PathBuf),
    Reveal(PathBuf),
    SearchText(String),
    Similar(Wallpaper),
    AddTag(Wallpaper, String),
    BulkTag(String),
    RemoveTag(Wallpaper, String),
    SetNotes(Wallpaper, String),
    Forget(Wallpaper),
    BulkForget,
    LoadMore,
    NewSearch,
    SwitchView(View),
    ApplySaved(SavedSearch),
    SaveSearch,
    DeleteSaved(i64),
    DownloadMissing,
    /// Export the library; `None` writes to the default exports folder.
    Export(Option<PathBuf>),
    Pick(pickers::Pick),
    ImportLibrary(PathBuf),
    ImportFiles(Vec<PathBuf>),
    AddFolder(PathBuf),
    ClearHistory,
    Compare(Vec<Wallpaper>),
    CleanDownloads,
}

/// Commands from outside the UI: tray menu, a second launch, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remote {
    Show,
    Next,
    ToggleRotation,
    #[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
    About,
    #[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
    Quit,
    /// macOS: the app was activated (Dock icon, Spotlight, Finder): bring back a hidden window.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Reopen,
}

/// Set when the window was closed to the tray on Linux, where a window cannot be hidden:
/// `main` then starts walle again without one (see `background`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub static TO_BACKGROUND: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// How the app was launched.
pub struct StartOptions {
    /// Create a system tray icon (Windows and Linux; on macOS, in the menu bar).
    #[cfg_attr(not(any(windows, target_os = "linux", target_os = "macos")), allow(dead_code))]
    pub tray: bool,
    /// Open the About box at once (asked for from the tray menu while there was no window).
    pub about: bool,
    /// Started with `--minimized` (autostart).
    pub minimized: bool,
    /// Single-instance hook: called when another launch asks us to show.
    pub on_show: Option<crate::single_instance::OnShow>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Persisted {
    provider: String,
    query: SearchQuery,
    view: View,
}

/// The scrolling body of a side panel, at exactly the panel's width. A scroll area expands to
/// fit what's inside it and a panel keeps the size of its contents, so anything slightly too wide
/// would otherwise widen the panel, make room for more, and creep on frame by frame.
fn panel_scroll(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let width = ui.available_width();
        let area = egui::Rect::from_min_size(ui.next_widget_position(), vec2(width, f32::INFINITY));
        // A child doesn't pass its size on; only what is allocated below counts.
        let mut body = ui.new_child(egui::UiBuilder::new().max_rect(area));
        add_contents(&mut body);
        ui.allocate_exact_size(vec2(width, body.min_rect().height()), egui::Sense::hover());
    });
}

/// The inputs of the visible list that are cheap to compare every frame.
type VisibleSig = (View, usize, usize, bool, bool);

pub struct App {
    paths: Paths,
    config: Config,
    worker: WorkerHandle,
    lib: Library,
    pal: Palette,
    infos: Vec<ProviderInfo>,
    enabled: Vec<ProviderInfo>,
    view: View,

    // browse
    provider: String,
    query: SearchQuery,
    results: Vec<Wallpaper>,
    result_keys: HashSet<String>,
    page: u32,
    has_more: bool,
    loading: bool,
    req_id: u64,
    total: Option<u64>,
    search_error: Option<String>,
    saved: Vec<SavedSearch>,

    // What the grid shows, rebuilt only when its inputs change. Long lists would otherwise be
    // filtered and cloned on every frame.
    visible: Rc<Vec<Wallpaper>>,
    visible_dirty: bool,
    visible_sig: Option<VisibleSig>,

    // library views
    lib_items: Vec<Wallpaper>,
    lib_filter: LibraryFilter,
    /// Status shown by the Library view (`None` = everything not rejected).
    lib_status: Option<Status>,
    lib_dirty: bool,
    /// Library totals, tags and duplicate data need reloading (coalesced to once per frame).
    counts_dirty: bool,
    /// Duplicate matches need recomputing, also coalesced.
    dup_dirty: bool,
    /// When typing in the library search has paused long enough to run it.
    filter_deadline: Option<f64>,
    counts: [u32; 4],
    all_tags: Vec<(String, u32)>,
    lib_providers: Vec<(String, u32)>,
    stats: Stats,

    // shared
    entries: HashMap<String, Rc<Entry>>,
    rejected: HashSet<String>,
    selected: Option<Wallpaper>,
    multi: HashSet<String>,
    details: HashMap<String, Wallpaper>,
    details_requested: HashSet<String>,
    textures: TextureCache,
    lightbox: Option<Wallpaper>,
    lb_zoom: f32,
    lb_pan: egui::Vec2,
    downloading: HashSet<String>,
    toasts: Toasts,
    actions: Vec<Action>,
    show_about: bool,
    /// The native window, for what only the OS can draw (shadow, rounded corners).
    #[cfg(windows)]
    native_window: Option<isize>,

    // grid navigation (from last frame)
    visible_order: Vec<String>,
    grid_cols: usize,
    grid_scroll: f32,
    scroll_to_selected: bool,

    // edit buffers
    tag_input: String,
    bulk_tag_input: String,
    notes_buf: (String, String),
    settings_draft: Option<Config>,
    settings_section: settings::Section,
    show_key: HashSet<String>,
    folder_input: String,
    /// Last checked folder input and whether it is a folder (not re-checked every frame).
    folder_check: (String, bool),
    import_input: String,
    theme_dirty: bool,
    duel: Option<compare::Duel>,

    // background / tray
    remote_tx: std::sync::mpsc::Sender<Remote>,
    remote_rx: std::sync::mpsc::Receiver<Remote>,
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    tray: Option<crate::tray::Tray>,
    /// A tray icon is there to come back from (on Linux: a panel shows it right now).
    has_tray: bool,
    #[cfg_attr(not(any(windows, target_os = "linux", target_os = "macos")), allow(dead_code))]
    tray_rotation_shown: bool,
    /// What is on disk, so the periodic save only writes when something changed.
    saved_config: Config,
    quitting: bool,
    hidden: bool,
    tray_hint_shown: bool,
    /// Frames during which to (re-)hide the window: eframe shows it after the first paint.
    hide_frames: u8,
    /// Keys of the wallpapers on the desktop now (one, or one per display).
    current_keys: Vec<String>,
    /// Displays that can each have their own wallpaper (empty: one for the whole desktop).
    displays: Vec<crate::desktop::Display>,
    /// What walle put on the desktop, display by display.
    shown: crate::desktop::Shown,
    rotation_next: Option<std::time::SystemTime>,
    cache_bytes: Option<u64>,
    autostart_on: bool,
    // duplicate detection
    hashes: HashMap<String, u64>,
    lib_hashes: Vec<(String, u64, Status)>,
    /// key -> closest library wallpaper (key, status) within the dHash threshold.
    dup_of: HashMap<String, Option<(String, Status)>>,
    lib_dupes: bool,
    collections: HashMap<String, Result<Vec<crate::providers::Collection>, String>>,
    collections_requested: HashSet<String>,
    importing_collection: Option<String>,
    ctx: egui::Context,
    picker_tx: std::sync::mpsc::Sender<Option<pickers::Picked>>,
    picker_rx: std::sync::mpsc::Receiver<Option<pickers::Picked>>,
    picker_busy: bool,
    /// The update check and the update itself (update.rs).
    updater: crate::update::Updater,
    update_seen: crate::update::Seen,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, paths: Paths, start: StartOptions) -> anyhow::Result<Self> {
        let config = Config::load(&paths);
        let (remote_tx, remote_rx) = std::sync::mpsc::channel();
        let (picker_tx, picker_rx) = std::sync::mpsc::channel();
        if let Some(hook) = &start.on_show {
            let (tx, ctx) = (remote_tx.clone(), cc.egui_ctx.clone());
            *hook.lock().unwrap() = Some(Box::new(move || {
                let _ = tx.send(Remote::Show);
                ctx.request_repaint();
            }));
        }
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        let tray = if start.tray {
            #[cfg(any(windows, target_os = "macos"))]
            let made = crate::tray::Tray::new(cc.egui_ctx.clone(), remote_tx.clone(), config.accent, config.rotation.enabled);
            #[cfg(target_os = "linux")]
            let made = {
                let ctx = cc.egui_ctx.clone();
                crate::tray::Tray::new(move || ctx.request_repaint(), remote_tx.clone(), config.accent, config.rotation.enabled)
            };
            match made {
                Ok(t) => Some(t),
                Err(e) => {
                    tracing::warn!("tray icon unavailable: {e:#}");
                    None
                }
            }
        } else {
            None
        };
        #[cfg(any(windows, target_os = "macos"))]
        let has_tray = tray.is_some();
        #[cfg(target_os = "linux")]
        let has_tray = tray.as_ref().is_some_and(|t| t.is_shown());
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
        let has_tray = false;
        if start.minimized && !has_tray {
            cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        let lib = Library::open(&paths.db_file())?;
        let worker = worker::spawn(cc.egui_ctx.clone(), paths.clone(), config.clone());
        let persisted: Option<Persisted> = cc.storage.and_then(|s| eframe::get_value(s, eframe::APP_KEY));
        let pal = Palette::new(config.theme, config.accent);
        theme::install_fonts(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx, &pal);
        #[cfg(windows)]
        let native_window = crate::native_window::handle_of(cc);
        #[cfg(windows)]
        if let Some(window) = native_window {
            crate::native_window::style(window, pal.border, pal.bg, pal.dark);
        }

        let update_seen = crate::update::Seen::load(&paths.update_file());
        let updater = crate::update::Updater::new(Some(paths.update_dir()), update_seen.version.as_deref());

        let mut app = Self {
            infos: vec![],
            enabled: vec![],
            provider: config.default_provider.clone(),
            query: SearchQuery::default(),
            view: View::Browse,
            rejected: lib.rejected_keys().unwrap_or_default(),
            saved: lib.saved_searches().unwrap_or_default(),
            pal,
            paths,
            worker,
            lib,
            visible: Rc::new(vec![]),
            visible_dirty: true,
            visible_sig: None,
            results: vec![],
            result_keys: HashSet::new(),
            page: 0,
            has_more: false,
            loading: false,
            req_id: 0,
            total: None,
            search_error: None,
            lib_items: vec![],
            lib_filter: LibraryFilter::default(),
            lib_status: Some(Status::Kept),
            lib_dirty: true,
            counts_dirty: false,
            dup_dirty: false,
            filter_deadline: None,
            counts: [0; 4],
            all_tags: vec![],
            lib_providers: vec![],
            stats: Stats::default(),
            entries: HashMap::new(),
            selected: None,
            multi: HashSet::new(),
            details: HashMap::new(),
            details_requested: HashSet::new(),
            textures: TextureCache::new(),
            lightbox: None,
            lb_zoom: 1.0,
            lb_pan: egui::Vec2::ZERO,
            downloading: HashSet::new(),
            toasts: Toasts::default(),
            actions: vec![],
            show_about: false,
            #[cfg(windows)]
            native_window,
            visible_order: vec![],
            grid_cols: 1,
            grid_scroll: 0.0,
            scroll_to_selected: false,
            tag_input: String::new(),
            bulk_tag_input: String::new(),
            notes_buf: (String::new(), String::new()),
            settings_draft: None,
            settings_section: Default::default(),
            show_key: HashSet::new(),
            folder_input: String::new(),
            folder_check: (String::new(), false),
            import_input: String::new(),
            theme_dirty: false,
            duel: None,
            remote_tx,
            remote_rx,
            #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
            tray,
            has_tray,
            tray_rotation_shown: config.rotation.enabled,
            saved_config: config.clone(),
            quitting: false,
            hidden: cfg!(any(windows, target_os = "macos")) && start.minimized && has_tray,
            tray_hint_shown: false,
            hide_frames: if cfg!(any(windows, target_os = "macos")) && start.minimized && has_tray { 3 } else { 0 },
            current_keys: vec![],
            displays: vec![],
            shown: Default::default(),
            rotation_next: None,
            cache_bytes: None,
            autostart_on: crate::autostart::is_enabled(),
            hashes: HashMap::new(),
            lib_hashes: vec![],
            dup_of: HashMap::new(),
            lib_dupes: false,
            collections: HashMap::new(),
            collections_requested: HashSet::new(),
            importing_collection: None,
            ctx: cc.egui_ctx.clone(),
            picker_tx,
            picker_rx,
            picker_busy: false,
            updater,
            update_seen,
            config,
        };
        app.show_about = start.about;
        app.start_updates();
        app.shown = crate::desktop::Shown::load(&app.paths);
        app.worker.send(Command::Desktop);
        app.refresh_providers();
        if let Some(p) = persisted {
            if app.enabled.iter().any(|i| i.id == p.provider) {
                app.provider = p.provider;
            }
            app.query = p.query;
            app.view = if p.view == View::Settings { View::Browse } else { p.view };
        }
        if !app.enabled.iter().any(|i| i.id == app.provider)
            && let Some(first) = app.enabled.first()
        {
            app.provider = first.id.to_string();
        }
        app.refresh_counts();
        app.new_search();
        Ok(app)
    }

    // ---------- helpers ----------

    fn refresh_providers(&mut self) {
        self.infos = providers::all_infos(&self.config);
        let reg = providers::build_registry(&self.config);
        self.enabled = self.infos.iter().filter(|i| reg.contains_key(i.id)).cloned().collect();
    }

    fn info(&self, id: &str) -> Option<&ProviderInfo> {
        self.infos.iter().find(|i| i.id == id)
    }

    fn provider_name(&self, id: &str) -> String {
        self.info(id).map(|i| i.name.to_string()).unwrap_or_else(|| id.to_string())
    }

    pub fn entry(&mut self, key: &str) -> Rc<Entry> {
        if let Some(e) = self.entries.get(key) {
            return e.clone();
        }
        let e = Rc::new(self.lib.entry(key).ok().flatten().unwrap_or_default());
        self.entries.insert(key.to_string(), e.clone());
        e
    }

    fn invalidate(&mut self, key: &str) {
        self.entries.remove(key);
        self.lib_dirty = true;
    }

    fn refresh_counts(&mut self) {
        self.counts_dirty = false;
        self.counts = self.lib.counts().unwrap_or_default();
        self.all_tags = self.lib.all_tags().unwrap_or_default();
        self.lib_providers = self.lib.providers().unwrap_or_default();
        self.refresh_lib_hashes();
    }

    /// A browse result that looks like a *different* wallpaper already in the library.
    fn is_duplicate_elsewhere(&self, key: &str) -> bool {
        let in_library = self.entries.get(key).is_some_and(|e| e.added_at > 0);
        !in_library && self.dup_of.get(key).is_some_and(|d| d.is_some())
    }

    fn refresh_lib_hashes(&mut self) {
        self.lib_hashes = self.lib.hashes().unwrap_or_default();
        self.recompute_dups();
    }

    fn recompute_dups(&mut self) {
        self.dup_dirty = false;
        let dup_of = self.hashes.keys().map(|k| (k.clone(), self.compute_dup(k))).collect();
        self.dup_of = dup_of;
        if self.config.hide_duplicates {
            self.visible_dirty = true;
        }
    }

    /// Apply what events and actions marked as stale, once, however many of them there were.
    fn flush_dirty(&mut self) {
        if self.counts_dirty {
            self.refresh_counts();
        } else if self.dup_dirty {
            self.recompute_dups();
        }
    }

    /// Closest *other* library wallpaper that looks like `key` (by thumbnail dHash).
    fn compute_dup(&self, key: &str) -> Option<(String, Status)> {
        let h = *self.hashes.get(key)?;
        if h == 0 {
            return None;
        }
        self.lib_hashes
            .iter()
            .filter(|(k, lh, _)| k != key && *lh != 0)
            .map(|(k, lh, st)| (crate::dedupe::distance(h, *lh), k, st))
            .filter(|(d, _, _)| *d <= crate::dedupe::THRESHOLD)
            .min_by_key(|(d, _, _)| *d)
            .map(|(_, k, st)| (k.clone(), *st))
    }

    fn toast(&mut self, kind: ToastKind, msg: impl Into<String>) {
        self.toasts.push(kind, msg);
    }

    fn report<T>(&mut self, r: anyhow::Result<T>) -> Option<T> {
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                self.toast(ToastKind::Error, format!("{e:#}"));
                None
            }
        }
    }

    /// The wallpaper list shown by the current view.
    fn current_items(&self) -> Vec<&Wallpaper> {
        match self.view {
            View::Browse => self
                .results
                .iter()
                .filter(|w| self.config.show_rejected || !self.rejected.contains(&w.key()))
                .filter(|w| !self.config.hide_duplicates || !self.is_duplicate_elsewhere(&w.key()))
                .collect(),
            _ => self.lib_items.iter().collect(),
        }
    }

    /// Bring the grid's list up to date; costs nothing unless an input changed.
    fn refresh_visible(&mut self) {
        let sig = (self.view, self.results.len(), self.lib_items.len(), self.config.show_rejected, self.config.hide_duplicates);
        if !self.visible_dirty && self.visible_sig == Some(sig) {
            return;
        }
        self.visible_dirty = false;
        self.visible_sig = Some(sig);
        let items: Vec<Wallpaper> = self.current_items().into_iter().cloned().collect();
        self.visible_order = items.iter().map(|w| w.key()).collect();
        if !self.multi.is_empty() {
            // Wallpapers that dropped out of the list (just rejected, say) leave the selection.
            let shown: HashSet<&str> = self.visible_order.iter().map(String::as_str).collect();
            self.multi.retain(|k| shown.contains(k.as_str()));
        }
        self.visible = Rc::new(items);
    }

    fn reload_library_view(&mut self) {
        if !self.lib_dirty {
            return;
        }
        self.lib_dirty = false;
        self.refresh_counts();
        self.lib_items = match self.view {
            View::History => {
                let mut seen = HashSet::new();
                self.lib
                    .history(500)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(w, _)| w)
                    .filter(|w| seen.insert(w.key()))
                    .collect()
            }
            View::Library if self.lib_dupes => {
                let items: Vec<(String, u64)> = self
                    .lib_hashes
                    .iter()
                    .filter(|(_, _, st)| *st != Status::Rejected)
                    .map(|(k, h, _)| (k.clone(), *h))
                    .collect();
                crate::dedupe::groups(&items)
                    .into_iter()
                    .flatten()
                    .filter_map(|k| self.lib.wallpaper(&k).ok().flatten())
                    .collect()
            }
            v => {
                let status = match v {
                    View::Considering => Some(Status::Considering),
                    View::Rejected => Some(Status::Rejected),
                    _ => self.lib_status,
                };
                let f = LibraryFilter { status, ..self.lib_filter.clone() };
                let listed = self.lib.list(&f);
                self.report(listed).unwrap_or_default()
            }
        };
        self.visible_dirty = true;
        if self.view == View::Library {
            self.worker.send(Command::LibraryStats);
        }
    }

    // ---------- search ----------

    fn new_search(&mut self) {
        // A collection belongs to the provider it came from.
        if !self.info(&self.provider).is_some_and(|i| i.caps.collections) {
            self.query.collection = None;
        }
        // Supersede whatever is in flight: `load_more` takes a new request id, so a late answer
        // to the old query is ignored instead of being mixed into these results.
        self.loading = false;
        self.results.clear();
        self.result_keys.clear();
        self.page = 0;
        self.has_more = true;
        self.total = None;
        self.query.seed = None;
        self.search_error = None;
        self.grid_scroll = 0.0;
        if self.enabled.is_empty() {
            self.has_more = false;
            return;
        }
        self.load_more();
    }

    fn load_more(&mut self) {
        if self.loading || !self.has_more {
            return;
        }
        self.req_id += 1;
        self.loading = true;
        self.worker.send(Command::Search {
            req: self.req_id,
            provider: self.provider.clone(),
            query: self.query.clone(),
            page: self.page + 1,
        });
    }

    // ---------- worker events ----------

    fn process_events(&mut self, ctx: &egui::Context) {
        // Uploading many images in one frame shows as a hitch; spread them out.
        const MAX_UPLOADS_PER_FRAME: usize = 8;
        let mut uploads = 0;
        while let Ok(ev) = self.worker.events.try_recv() {
            match ev {
                Event::Search { req, result } => {
                    if req != self.req_id {
                        continue;
                    }
                    self.loading = false;
                    match result {
                        Ok(page) => {
                            self.page = page.page;
                            self.has_more = page.has_more();
                            self.total = page.total.or(self.total);
                            if page.seed.is_some() {
                                self.query.seed = page.seed.clone();
                            }
                            for w in page.items {
                                if self.result_keys.insert(w.key()) {
                                    self.results.push(w);
                                }
                            }
                        }
                        Err(e) => {
                            self.has_more = false;
                            self.search_error = Some(e.clone());
                            self.toast(ToastKind::Error, format!("Search failed: {e}"));
                        }
                    }
                }
                Event::Details { key, result } => match result {
                    Ok(d) => {
                        if self.selected.as_ref().is_some_and(|s| s.key() == key) {
                            self.selected = Some(d.clone());
                        }
                        if self.lib.entry(&key).ok().flatten().is_some() {
                            let _ = self.lib.upsert(&d);
                        }
                        self.details.insert(key, d);
                    }
                    Err(e) => {
                        tracing::warn!("details for {key}: {e}");
                        // Stops the "loading tags" spinner; selecting it again retries.
                        self.details_requested.remove(&key);
                    }
                },
                Event::Image { tex_key, result } => {
                    self.textures.put(ctx, tex_key, result);
                    uploads += 1;
                    if uploads >= MAX_UPLOADS_PER_FRAME {
                        ctx.request_repaint();
                        break;
                    }
                }
                Event::Stats(s) => self.stats = s,
                Event::Downloaded { key, result } => {
                    self.downloading.remove(&key);
                    match result {
                        Ok(path) => {
                            if let Some(wp) = self.find(&key)
                                && !wp.is_local()
                            {
                                let r = self.lib.set_local_path(&wp, Some(&path));
                                self.report(r);
                                self.invalidate(&key);
                            }
                        }
                        Err(e) => self.toast(ToastKind::Error, format!("Download failed: {e}")),
                    }
                }
                Event::Displays(d) => self.displays = d,
                Event::OnDesktop(keys) => {
                    for k in self.current_keys.iter().chain(&keys).cloned().collect::<Vec<_>>() {
                        self.invalidate(&k);
                    }
                    self.current_keys = keys;
                    self.shown = crate::desktop::Shown::load(&self.paths);
                }
                Event::WallpaperSet { key, target, result, wp } => match result {
                    Ok(()) => {
                        let auto = wp.is_some();
                        if let Some(wp) = wp.or_else(|| self.find(&key)) {
                            let r = self.lib.record_set(&wp);
                            self.report(r);
                            let what = match (&target, auto) {
                                (crate::desktop::Target::All, true) => "Rotated to".to_string(),
                                (crate::desktop::Target::All, false) => "Wallpaper set:".to_string(),
                                (crate::desktop::Target::Display(d), true) => format!("{d} rotated to"),
                                (crate::desktop::Target::Display(d), false) => format!("Wallpaper set on {d}:"),
                            };
                            self.toast(ToastKind::Success, format!("{what} {}", wp.display_title()));
                            self.invalidate(&key);
                            #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
                            if let Some(t) = &self.tray {
                                t.set_tooltip(&format!("walle — {}", wp.display_title()));
                            }
                        }
                    }
                    Err(e) => self.toast(ToastKind::Error, format!("Could not set wallpaper: {e}")),
                },
                Event::RotationScheduled(t) => self.rotation_next = t,
                Event::RotationFailed(e) => self.toast(ToastKind::Error, format!("Rotation: {e}")),
                Event::CacheSize(b) => self.cache_bytes = Some(b),
                Event::Collections { provider, result } => {
                    self.collections.insert(provider, result);
                }
                Event::CollectionImported { label, result } => {
                    self.importing_collection = None;
                    match result {
                        Ok(items) => {
                            let tag = format!("collection: {label}");
                            self.batched(|app| {
                                for wp in &items {
                                    // No auto-download for bulk imports; files are fetched when used.
                                    let r = app.lib.set_status(wp, Status::Kept).and_then(|_| app.lib.add_tag(wp, &tag));
                                    app.report(r);
                                    app.rejected.remove(&wp.key());
                                    app.entries.remove(&wp.key());
                                }
                            });
                            self.lib_dirty = true;
                            self.visible_dirty = true;
                            self.counts_dirty = true;
                            self.toast(ToastKind::Success, format!("Imported {} wallpapers from \"{label}\" (tagged \"{tag}\")", items.len()));
                        }
                        Err(e) => self.toast(ToastKind::Error, format!("Import failed: {e}")),
                    }
                }
                Event::Hash { key, hash } => {
                    self.hashes.insert(key.clone(), hash);
                    let in_library = self.entry(&key).added_at > 0;
                    if in_library && self.lib_hashes.iter().all(|(k, h, _)| *k != key || *h != hash) {
                        let _ = self.lib.set_hash(&key, hash);
                        let status = self.entry(&key).status;
                        match self.lib_hashes.iter_mut().find(|(k, _, _)| *k == key) {
                            Some(known) => known.1 = hash,
                            None => self.lib_hashes.push((key, hash, status)),
                        }
                        // Thumbnails arrive in bursts; recompute once, at the end of the frame.
                        self.dup_dirty = true;
                    } else {
                        let d = self.compute_dup(&key);
                        self.dup_of.insert(key, d);
                        if self.config.hide_duplicates {
                            self.visible_dirty = true;
                        }
                    }
                }
                Event::Imported(items) => {
                    let n = items.len();
                    self.batched(|app| {
                        for wp in items {
                            let path = PathBuf::from(&wp.full_url);
                            let r = app.lib.set_status(&wp, Status::Kept).and_then(|_| app.lib.set_local_path(&wp, Some(&path)));
                            app.report(r);
                            app.rejected.remove(&wp.key());
                            app.entries.remove(&wp.key());
                        }
                    });
                    self.lib_dirty = true;
                    self.visible_dirty = true;
                    self.counts_dirty = true;
                    self.toast(
                        if n > 0 { ToastKind::Success } else { ToastKind::Info },
                        if n > 0 { format!("Imported {n} image(s) into your library") } else { "No images found to import".into() },
                    );
                }
            }
        }
    }

    fn find(&self, key: &str) -> Option<Wallpaper> {
        if let Some(d) = self.details.get(key) {
            return Some(d.clone());
        }
        self.selected
            .iter()
            .chain(self.lightbox.iter())
            .chain(self.results.iter())
            .chain(self.lib_items.iter())
            .find(|w| w.has_key(key))
            .cloned()
            .or_else(|| self.lib.wallpaper(key).ok().flatten())
    }

    fn multi_wallpapers(&self) -> Vec<Wallpaper> {
        // Keep grid order for predictable bulk operations.
        self.visible
            .iter()
            .filter_map(|w| {
                let key = w.key();
                self.multi.contains(&key).then(|| self.details.get(&key).unwrap_or(w).clone())
            })
            .collect()
    }

    /// Run `f` with all its library writes in one transaction.
    fn batched<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let began = self.lib.begin();
        let out = f(self);
        if began {
            self.lib.commit();
        }
        out
    }

    // ---------- actions ----------

    fn apply_status(&mut self, wp: &Wallpaper, st: Status) {
        let r = self.lib.set_status(wp, st);
        self.report(r);
        if let Some(h) = self.hashes.get(&wp.key()).copied() {
            let _ = self.lib.set_hash(&wp.key(), h);
        }
        if wp.is_local() && st != Status::None {
            let r = self.lib.set_local_path(wp, Some(std::path::Path::new(&wp.full_url)));
            self.report(r);
        }
        if st == Status::Rejected {
            self.rejected.insert(wp.key());
        } else {
            self.rejected.remove(&wp.key());
        }
        self.visible_dirty = true;
        if st == Status::Kept && self.config.auto_download_kept && !wp.is_local() && self.local_file(wp).is_none() {
            self.downloading.insert(wp.key());
            self.worker.send(Command::Download { wp: wp.clone(), after: After::Nothing });
        }
        self.invalidate(&wp.key());
        self.counts_dirty = true;
    }

    fn forget(&mut self, wp: &Wallpaper) {
        let key = wp.key();
        let entry = self.entry(&key);
        if let Some(p) = &entry.local_path
            && !wp.is_local()
            && p.starts_with(self.paths.wallpapers_dir(&self.config))
        {
            let _ = std::fs::remove_file(p);
        }
        let r = self.lib.forget(&key);
        self.report(r);
        self.rejected.remove(&key);
        self.multi.remove(&key);
        self.visible_dirty = true;
        self.counts_dirty = true;
        if self.selected.as_ref().is_some_and(|s| s.key() == key) {
            self.selected = None;
        }
        self.invalidate(&key);
    }

    fn apply_actions(&mut self, ctx: &egui::Context) {
        for action in std::mem::take(&mut self.actions) {
            match action {
                Action::Select(wp) => {
                    self.multi.clear();
                    self.select(wp);
                }
                Action::ToggleMulti(wp) => {
                    if self.multi.is_empty()
                        && let Some(s) = &self.selected
                    {
                        self.multi.insert(s.key());
                    }
                    let k = wp.key();
                    if !self.multi.remove(&k) {
                        self.multi.insert(k);
                    }
                    self.select(wp);
                }
                Action::RangeSelect(wp) => {
                    let anchor = self.selected.as_ref().map(|s| s.key());
                    let pos = |k: &str| self.visible_order.iter().position(|x| x == k);
                    if let (Some(a), Some(b)) = (anchor.as_deref().and_then(pos), pos(&wp.key())) {
                        let (lo, hi) = (a.min(b), a.max(b));
                        for k in &self.visible_order[lo..=hi] {
                            self.multi.insert(k.clone());
                        }
                    } else {
                        self.multi.insert(wp.key());
                    }
                    self.select(wp);
                }
                Action::SelectAll => self.multi = self.visible_order.iter().cloned().collect(),
                Action::ClearMulti => self.multi.clear(),
                Action::Lightbox(wp) => {
                    self.select(wp.clone());
                    self.open_lightbox(wp);
                }
                Action::SetStatus(wp, st) => {
                    let current = self.entry(&wp.key()).status;
                    let st = if current == st { Status::None } else { st };
                    self.apply_status(&wp, st);
                    let msg = match st {
                        Status::None => format!("Cleared status of {}", wp.display_title()),
                        s => format!("{}: {}", s.label(), wp.display_title()),
                    };
                    self.toast(ToastKind::Info, msg);
                }
                Action::BulkStatus(st) => {
                    let items = self.multi_wallpapers();
                    self.batched(|app| {
                        for wp in &items {
                            app.apply_status(wp, st);
                        }
                    });
                    self.toast(ToastKind::Success, format!("{}: {} wallpaper(s)", st.label(), items.len()));
                }
                Action::Rate(wp, r) => {
                    let current = self.entry(&wp.key()).rating;
                    let r = if current == r { 0 } else { r };
                    let res = self.lib.set_rating(&wp, r);
                    self.report(res);
                    self.invalidate(&wp.key());
                }
                Action::SetWallpaper(wp) => self.set_wallpaper(wp, crate::desktop::Target::All),
                Action::SetWallpaperOn(wp, display) => self.set_wallpaper(wp, crate::desktop::Target::Display(display)),
                Action::Download(wp) => self.download(wp),
                Action::BulkDownload => {
                    let items = self.multi_wallpapers();
                    let n = items.len();
                    for wp in items {
                        self.download(wp);
                    }
                    self.toast(ToastKind::Info, format!("Downloading {n} wallpaper(s)…"));
                }
                Action::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
                Action::OpenPath(p) => theme::open_path(&p),
                Action::Reveal(p) => theme::reveal_path(&p),
                Action::SearchText(text) => {
                    self.view = View::Browse;
                    self.query.text = text;
                    self.new_search();
                }
                Action::Similar(wp) => {
                    self.view = View::Browse;
                    self.provider = wp.provider.clone();
                    self.query.text = format!("like:{}", wp.remote_id);
                    self.query.sorting = crate::model::Sorting::Relevance;
                    self.new_search();
                }
                Action::AddTag(wp, tag) => {
                    self.batched(|app| {
                        for t in tag.split(',') {
                            let r = app.lib.add_tag(&wp, t);
                            app.report(r);
                        }
                    });
                    self.invalidate(&wp.key());
                    self.counts_dirty = true;
                }
                Action::BulkTag(tag) => {
                    let items = self.multi_wallpapers();
                    self.batched(|app| {
                        for wp in &items {
                            for t in tag.split(',') {
                                let r = app.lib.add_tag(wp, t);
                                app.report(r);
                            }
                            app.invalidate(&wp.key());
                        }
                    });
                    self.counts_dirty = true;
                    self.toast(ToastKind::Success, format!("Tagged {} wallpaper(s)", items.len()));
                }
                Action::RemoveTag(wp, tag) => {
                    let r = self.lib.remove_tag(&wp.key(), &tag);
                    self.report(r);
                    self.invalidate(&wp.key());
                    self.counts_dirty = true;
                }
                Action::SetNotes(wp, notes) => {
                    let r = self.lib.set_notes(&wp, &notes);
                    self.report(r);
                    self.entries.remove(&wp.key());
                }
                Action::Forget(wp) => {
                    self.forget(&wp);
                    self.toast(ToastKind::Info, format!("Removed {} from the library", wp.display_title()));
                }
                Action::BulkForget => {
                    let items = self.multi_wallpapers();
                    self.batched(|app| {
                        for wp in &items {
                            app.forget(wp);
                        }
                    });
                    self.multi.clear();
                    self.toast(ToastKind::Info, format!("Removed {} wallpaper(s) from the library", items.len()));
                }
                Action::LoadMore => self.load_more(),
                Action::NewSearch => self.new_search(),
                Action::SwitchView(v) => {
                    if self.view == View::Settings && v != View::Settings {
                        self.commit_settings_draft();
                        self.settings_draft = None;
                    }
                    if v == View::Settings {
                        // The page lists every provider, so the draft has entries for all of them.
                        // Same for the config, or merely opening the page would read as an edit.
                        for info in &self.infos {
                            self.config.providers.entry(info.id.to_string()).or_insert_with(|| crate::config::ProviderSettings {
                                enabled: crate::config::default_enabled(info.id),
                                ..Default::default()
                            });
                        }
                        self.settings_draft = Some(self.config.clone());
                    }
                    self.view = v;
                    self.multi.clear();
                    self.lib_dirty = true;
                }
                Action::ApplySaved(s) => {
                    self.view = View::Browse;
                    if self.enabled.iter().any(|i| i.id == s.provider) {
                        self.provider = s.provider;
                    }
                    self.query = s.query;
                    self.new_search();
                }
                Action::SaveSearch => {
                    let name = if self.query.text.trim().is_empty() {
                        format!("{} · {}", self.provider_name(&self.provider), self.query.sorting.label())
                    } else {
                        self.query.text.trim().to_string()
                    };
                    let r = self.lib.save_search(&name, &self.provider, &self.query);
                    if self.report(r).is_some() {
                        self.saved = self.lib.saved_searches().unwrap_or_default();
                        self.toast(ToastKind::Success, format!("Saved search \"{name}\""));
                    }
                }
                Action::DeleteSaved(id) => {
                    let r = self.lib.delete_search(id);
                    self.report(r);
                    self.saved = self.lib.saved_searches().unwrap_or_default();
                }
                Action::DownloadMissing => {
                    let missing: Vec<Wallpaper> = self.lib.kept_without_file().unwrap_or_default().into_iter().filter(|w| !w.is_local()).collect();
                    let n = missing.len();
                    for wp in missing {
                        self.download(wp);
                    }
                    self.toast(ToastKind::Info, format!("Downloading {n} missing file(s)…"));
                }
                Action::Pick(kind) => self.open_picker(kind),
                Action::Export(target) => {
                    let file = target.unwrap_or_else(|| {
                        let dir = self.paths.data_dir.join("exports");
                        let _ = std::fs::create_dir_all(&dir);
                        dir.join(format!("walle-library-{}.json", chrono::Local::now().format("%Y%m%d-%H%M%S")))
                    });
                    let r = self.lib.export_json(&file);
                    if let Some(n) = self.report(r) {
                        self.toast(ToastKind::Success, format!("Exported {n} entries to {}", file.display()));
                        theme::reveal_path(&file);
                    }
                }
                Action::ImportLibrary(path) => {
                    let r = self.lib.import_json(&path);
                    if let Some(n) = self.report(r) {
                        self.rejected = self.lib.rejected_keys().unwrap_or_default();
                        self.entries.clear();
                        self.lib_dirty = true;
                        self.visible_dirty = true;
                        self.counts_dirty = true;
                        self.saved = self.lib.saved_searches().unwrap_or_default();
                        self.toast(ToastKind::Success, format!("Imported {n} library entries"));
                    }
                }
                Action::ImportFiles(paths) => {
                    self.toast(ToastKind::Info, "Importing…");
                    self.worker.send(Command::Import(paths));
                }
                Action::AddFolder(dir) => {
                    // A folder dropped on the Settings page leaves it.
                    self.commit_settings_draft();
                    self.settings_draft = None;
                    if !self.config.local_folders.contains(&dir) {
                        let mut cfg = self.config.clone();
                        cfg.local_folders.push(dir.clone());
                        cfg.providers.entry("local".into()).or_default().enabled = true;
                        self.apply_config(cfg);
                    }
                    self.view = View::Browse;
                    self.provider = "local".into();
                    self.query.text.clear();
                    self.new_search();
                    self.toast(ToastKind::Success, format!("Added folder {}", dir.display()));
                }
                Action::Compare(items) => self.start_duel(items),
                Action::CleanDownloads => self.clean_downloads(),
                Action::ClearHistory => {
                    let r = self.lib.clear_history();
                    self.report(r);
                    self.lib_dirty = true;
                }
            }
        }
        self.flush_dirty();
        // The change has to reach the screen even if no further input arrives.
        if self.lib_dirty || self.visible_dirty || self.theme_dirty || !self.actions.is_empty() {
            ctx.request_repaint();
        }
    }

    /// Delete downloads of wallpapers that are not Kept. Only files inside walle's own
    /// wallpapers folder are touched, and never the current desktop wallpaper.
    fn clean_downloads(&mut self) {
        let root = self.paths.wallpapers_dir(&self.config);
        // What is on the desktop: the originals walle put there, and what the desktop reports.
        let mut current = crate::desktop::Shown::load(&self.paths).files();
        current.extend(crate::setter::current_wallpapers());
        let mut freed = 0u64;
        let mut n = 0;
        for (key, path) in self.lib.downloads_not_kept().unwrap_or_default() {
            if !path.starts_with(&root) || current.contains(&path) {
                continue;
            }
            let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if std::fs::remove_file(&path).is_ok() || !path.exists() {
                freed += len;
                n += 1;
                let _ = self.lib.clear_local_path(&key);
                self.entries.remove(&key);
            }
        }
        self.toast(ToastKind::Success, format!("Removed {n} file(s), freed {}", theme::human_size(freed)));
    }

    fn select(&mut self, wp: Wallpaper) {
        let key = wp.key();
        let wp = self.details.get(&key).cloned().unwrap_or(wp);
        if self.notes_buf.0 != key {
            self.notes_buf = (key.clone(), self.entry(&key).notes.clone());
            self.tag_input.clear();
        }
        let wants_details = self.info(&wp.provider).is_some_and(|i| i.caps.details);
        if wants_details && self.details_requested.insert(key) {
            self.worker.send(Command::Details { provider: wp.provider.clone(), id: wp.remote_id.clone() });
        }
        self.selected = Some(wp);
    }

    fn local_file(&mut self, wp: &Wallpaper) -> Option<PathBuf> {
        let p = self
            .entry(&wp.key())
            .local_path
            .clone()
            .unwrap_or_else(|| worker::local_path_for(&self.paths, &self.config, wp));
        p.metadata().ok().filter(|m| m.len() > 0).map(|_| p)
    }

    fn download(&mut self, wp: Wallpaper) {
        if wp.is_local() || self.downloading.contains(&wp.key()) {
            return;
        }
        let _ = self.lib.upsert(&wp);
        self.downloading.insert(wp.key());
        self.worker.send(Command::Download { wp, after: After::Nothing });
    }

    fn set_wallpaper(&mut self, wp: Wallpaper, target: crate::desktop::Target) {
        let fit = self.config.fit_mode;
        let _ = self.lib.upsert(&wp);
        match self.local_file(&wp) {
            Some(path) => self.worker.send(Command::SetLocal { key: wp.key(), path, fit, target }),
            None => {
                self.toast(ToastKind::Info, format!("Downloading {}…", wp.display_title()));
                self.downloading.insert(wp.key());
                self.worker.send(Command::Download { wp, after: After::SetWallpaper(fit, target) });
            }
        }
    }

    fn open_lightbox(&mut self, wp: Wallpaper) {
        let tex_key = format!("full:{}", wp.key());
        if self.textures.peek(&tex_key).is_none() && !self.textures.is_loading(&tex_key) {
            self.textures.get(&tex_key, || {});
            match self.local_file(&wp) {
                Some(path) => self.worker.send(Command::LoadLocal { tex_key, path, max_dim: 4096 }),
                None => {
                    self.downloading.insert(wp.key());
                    self.worker.send(Command::Download { wp: wp.clone(), after: After::Preview(tex_key) });
                }
            }
        }
        if self.lightbox.as_ref().map(|l| l.key()) != Some(wp.key()) {
            self.lb_zoom = 1.0;
            self.lb_pan = egui::Vec2::ZERO;
        }
        self.lightbox = Some(wp);
    }

    // ---------- keyboard / drag & drop ----------

    fn handle_keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || self.view == View::Settings {
            return;
        }
        let (keys, mods) = ctx.input(|i| {
            let keys: Vec<Key> = i
                .events
                .iter()
                .filter_map(|e| match e {
                    // Held keys repeat. That suits moving around; for the rest it would flip a
                    // status back and forth, or reject one wallpaper after another with X.
                    egui::Event::Key { key, pressed: true, repeat, .. }
                        if !*repeat || matches!(key, Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown) =>
                    {
                        Some(*key)
                    }
                    _ => None,
                })
                .collect();
            (keys, i.modifiers)
        });
        if self.duel.is_some() {
            self.duel_keys(&keys);
            return;
        }
        if mods.command {
            if keys.contains(&Key::A) {
                self.actions.push(Action::SelectAll);
            }
            return;
        }
        if mods.alt {
            return;
        }
        for key in keys {
            if key == Key::Escape {
                if self.lightbox.is_some() {
                    self.lightbox = None;
                } else if !self.multi.is_empty() {
                    self.multi.clear();
                } else {
                    self.selected = None;
                }
                continue;
            }
            let delta: isize = match key {
                Key::ArrowRight => 1,
                Key::ArrowLeft => -1,
                Key::ArrowDown if self.lightbox.is_none() => self.grid_cols as isize,
                Key::ArrowUp if self.lightbox.is_none() => -(self.grid_cols as isize),
                _ => 0,
            };
            if delta != 0 {
                self.move_selection(delta);
                continue;
            }
            let Some(wp) = self.selected.clone() else { continue };
            match key {
                Key::C => self.actions.push(Action::SetStatus(wp, Status::Considering)),
                Key::K => self.actions.push(Action::SetStatus(wp, Status::Kept)),
                Key::X | Key::Delete => {
                    self.actions.push(Action::SetStatus(wp, Status::Rejected));
                    if self.view == View::Browse {
                        self.move_selection(1);
                    }
                }
                Key::Num0 => self.actions.push(Action::Rate(wp, 0)),
                Key::Num1 => self.actions.push(Action::Rate(wp, 1)),
                Key::Num2 => self.actions.push(Action::Rate(wp, 2)),
                Key::Num3 => self.actions.push(Action::Rate(wp, 3)),
                Key::Num4 => self.actions.push(Action::Rate(wp, 4)),
                Key::Num5 => self.actions.push(Action::Rate(wp, 5)),
                Key::Enter => self.actions.push(Action::SetWallpaper(wp)),
                Key::Space => {
                    if self.lightbox.is_some() {
                        self.lightbox = None;
                    } else {
                        self.actions.push(Action::Lightbox(wp));
                    }
                }
                Key::D => self.actions.push(Action::Download(wp)),
                _ => {}
            }
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.visible_order.is_empty() {
            return;
        }
        let cur = self.selected.as_ref().map(|s| s.key()).and_then(|s| self.visible_order.iter().position(|k| *k == s));
        let next = match cur {
            Some(i) => (i as isize + delta).clamp(0, self.visible_order.len() as isize - 1) as usize,
            None => 0,
        };
        let key = self.visible_order[next].clone();
        if let Some(wp) = self.find(&key) {
            let in_lightbox = self.lightbox.is_some();
            self.multi.clear();
            self.select(wp.clone());
            if in_lightbox {
                self.open_lightbox(wp);
            }
            self.scroll_to_selected = true;
        }
        if self.view == View::Browse && next + self.grid_cols * 2 >= self.visible_order.len() {
            self.load_more();
        }
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let (dropped, hovering) = ctx.input(|i| {
            (
                i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect::<Vec<_>>(),
                !i.raw.hovered_files.is_empty(),
            )
        });
        if hovering {
            let rect = Self::below_title_bar(ctx);
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop")));
            painter.rect_filled(rect, 0.0, Color32::from_black_alpha(170));
            painter.rect_stroke(rect.shrink(16.0), 16.0, Stroke::new(2.0, self.pal.accent), egui::StrokeKind::Inside);
            painter.text(rect.center() - vec2(0.0, 52.0), egui::Align2::CENTER_CENTER, ph::DOWNLOAD_SIMPLE, FontId::proportional(40.0), self.pal.accent);
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Drop images to import them into your library\nDrop a folder to browse it · Drop a walle export (.json) to restore",
                egui::FontId::proportional(18.0),
                Color32::WHITE,
            );
        }
        if dropped.is_empty() {
            return;
        }
        let mut files = vec![];
        for p in dropped {
            if p.is_dir() {
                self.actions.push(Action::AddFolder(p));
            } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) {
                self.actions.push(Action::ImportLibrary(p));
            } else {
                files.push(p);
            }
        }
        if !files.is_empty() {
            self.actions.push(Action::ImportFiles(files));
        }
    }

    // ---------- layout ----------

    /// The search row under the titlebar.
    fn browse_toolbar(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        ui.horizontal(|ui| {
            let current = self.provider_name(&self.provider);
            let combo = egui::ComboBox::from_id_salt("provider")
                .selected_text(semibold(current))
                .width(150.0)
                .height(400.0)
                .show_ui(ui, |ui| {
                    for i in self.enabled.clone() {
                        if ui.selectable_label(self.provider == i.id, i.name).clicked() && self.provider != i.id {
                            self.provider = i.id.to_string();
                            self.actions.push(Action::NewSearch);
                        }
                    }
                })
                .response;
            theme::name_widget(&combo, egui::Role::ComboBox, "Source");

            let total = self.total.map(|t| ui.painter().layout_no_wrap(format!("{t} results"), FontId::proportional(12.5), p.weak));
            let total_width = total.as_ref().map_or(0.0, |g| g.size().x + 16.0);
            let caps = self.info(&self.provider).map(|i| i.caps.clone()).unwrap_or_default();
            if caps.text_search {
                let hint = if self.provider == "wallhaven" {
                    "Search tags…   +tag  -tag  @user  like:id  type:png"
                } else {
                    "Search…"
                };
                // Room for the Search button and the save button.
                let width = (ui.available_width() - 76.0 - 38.0 - 16.0 - total_width).max(160.0);
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.query.text)
                        .hint_text(hint)
                        .desired_width(width)
                        .margin(Margin { left: 32, right: 30, top: 7, bottom: 7 }),
                );
                ui.painter().text(resp.rect.left_center() + vec2(11.0, 0.0), egui::Align2::LEFT_CENTER, ph::MAGNIFYING_GLASS, FontId::proportional(15.0), p.weak);
                if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    self.actions.push(Action::NewSearch);
                }
                if !self.query.text.is_empty() {
                    let spot = egui::Rect::from_center_size(resp.rect.right_center() - vec2(16.0, 0.0), egui::Vec2::splat(22.0));
                    let clear = ui.put(spot, egui::Button::new(RichText::new(ph::X).size(13.0).color(p.weak)).frame_when_inactive(false));
                    if clear.on_hover_text("Clear search").clicked() {
                        self.query.text.clear();
                        self.actions.push(Action::NewSearch);
                    }
                }
                if theme::primary_button(ui, &p, "Search").clicked() {
                    self.actions.push(Action::NewSearch);
                }
            } else if theme::primary_button(ui, &p, with_icon(ph::ARROW_CLOCKWISE, "Refresh")).clicked() {
                self.actions.push(Action::NewSearch);
            }
            if theme::icon_button(ui, ph::BOOKMARK_SIMPLE, "Save this search").clicked() {
                self.actions.push(Action::SaveSearch);
            }
            if let Some(galley) = total {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (r, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
                    ui.painter().galley(r.min, galley, p.weak);
                });
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        const HINTS: &str = "C consider  ·  K keep  ·  X reject  ·  1–5 rate  ·  Space preview  ·  Enter set";
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.set_min_height(18.0);
            let weak = |s: &str| RichText::new(s).size(11.5).color(p.weak);
            if self.loading {
                ui.add(egui::Spinner::new().size(12.0).color(p.accent));
                ui.label(weak("Loading…"));
            }
            if !self.downloading.is_empty() {
                ui.add(egui::Spinner::new().size(12.0).color(p.accent));
                ui.label(weak(&format!("{} download(s)", self.downloading.len())));
            }
            if let Some(name) = &self.importing_collection {
                ui.add(egui::Spinner::new().size(12.0).color(p.accent));
                ui.label(weak(&format!("Importing \"{name}\"")));
            }
            if !self.multi.is_empty() {
                ui.label(weak(&format!("{} selected", self.multi.len())));
            }
            if self.config.rotation.enabled {
                if let Some(t) = self.rotation_next {
                    let local: chrono::DateTime<chrono::Local> = t.into();
                    ui.label(weak(&format!("{}  Next wallpaper at {}", ph::ARROWS_CLOCKWISE, local.format("%H:%M"))));
                }
                let next = egui::Button::new(RichText::new(ph::SKIP_FORWARD).size(13.0).color(p.weak)).frame_when_inactive(false);
                if ui.add(next).on_hover_text("Change the wallpaper now").clicked() {
                    let _ = self.remote_tx.send(Remote::Next);
                }
            }
            // The shortcut reminder goes first when space runs out.
            let hints = ui.painter().layout_no_wrap(HINTS.into(), FontId::proportional(11.5), p.weak);
            if self.view != View::Settings && ui.available_width() > hints.size().x + 16.0 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (r, _) = ui.allocate_exact_size(hints.size(), egui::Sense::hover());
                    ui.painter().galley(r.min, hints, p.weak);
                });
            }
        });
    }

    fn bulk_bar(&mut self, ui: &mut egui::Ui) {
        if self.multi.is_empty() {
            return;
        }
        let p = self.pal;
        egui::Frame::default()
            .fill(p.accent_soft())
            .stroke(Stroke::new(1.0, p.accent.gamma_multiply(0.5)))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(semibold(format!("{} selected", self.multi.len())).color(p.accent_text()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Clear selection").clicked() {
                            self.actions.push(Action::ClearMulti);
                        }
                        if ui.button("Select all").clicked() {
                            self.actions.push(Action::SelectAll);
                        }
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button(with_icon(ph::PUSH_PIN, "Consider")).clicked() {
                        self.actions.push(Action::BulkStatus(Status::Considering));
                    }
                    if ui.button(with_icon(ph::CHECK, "Keep")).clicked() {
                        self.actions.push(Action::BulkStatus(Status::Kept));
                    }
                    if ui.button(with_icon(ph::X, "Reject")).clicked() {
                        self.actions.push(Action::BulkStatus(Status::Rejected));
                    }
                    if ui.button(with_icon(ph::DOWNLOAD_SIMPLE, "Download")).clicked() {
                        self.actions.push(Action::BulkDownload);
                    }
                    ui.add(egui::TextEdit::singleline(&mut self.bulk_tag_input).hint_text("tag(s)…").desired_width(120.0));
                    if ui.button(with_icon(ph::TAG, "Tag")).clicked() && !self.bulk_tag_input.trim().is_empty() {
                        self.actions.push(Action::BulkTag(std::mem::take(&mut self.bulk_tag_input)));
                    }
                    if self.multi.len() >= 2 && ui.button(with_icon(ph::SCALES, "Compare")).clicked() {
                        self.actions.push(Action::Compare(self.multi_wallpapers()));
                    }
                    if self.view != View::Browse && ui.button(with_icon(ph::TRASH, "Remove from library")).clicked() {
                        self.actions.push(Action::BulkForget);
                    }
                });
            });
        ui.add_space(8.0);
    }

    fn library_header(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        ui.horizontal(|ui| {
            ui.label(semibold(self.view.label()).size(21.0));
            ui.add_space(6.0);
            let n = self.lib_items.len();
            ui.label(RichText::new(format!("{n} wallpaper{}", if n == 1 { "" } else { "s" })).color(p.weak));
            if self.view == View::Library && self.stats.on_disk > 0 {
                ui.label(RichText::new(format!("· {} on disk", theme::human_size(self.stats.bytes))).color(p.weak));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match self.view {
                View::Library => {
                    if ui.button(with_icon(ph::EXPORT, "Export")).on_hover_text("Export the whole library (statuses, ratings, tags, notes) to JSON").clicked() {
                        self.actions.push(Action::Pick(pickers::Pick::ExportTo));
                    }
                    if ui.button(with_icon(ph::PLUS, "Import images")).on_hover_text("Add image files from your computer to the library").clicked() {
                        self.actions.push(Action::Pick(pickers::Pick::ImportImages));
                    }
                    if ui.button(with_icon(ph::FOLDER_OPEN, "Open folder")).clicked() {
                        let dir = self.paths.wallpapers_dir(&self.config);
                        let _ = std::fs::create_dir_all(&dir);
                        self.actions.push(Action::OpenPath(dir));
                    }
                    let missing = self.stats.total.saturating_sub(self.stats.on_disk);
                    if missing > 0 && ui.button(with_icon(ph::DOWNLOAD_SIMPLE, &format!("Download {missing} missing"))).clicked() {
                        self.actions.push(Action::DownloadMissing);
                    }
                }
                View::History => {
                    if !self.lib_items.is_empty() && ui.button(with_icon(ph::TRASH, "Clear history")).clicked() {
                        self.actions.push(Action::ClearHistory);
                    }
                }
                View::Considering => {
                    let enough = self.lib_items.len() >= 2;
                    let btn = ui.add_enabled_ui(enough, |ui| theme::primary_button(ui, &p, with_icon(ph::SCALES, "Compare all"))).inner;
                    if btn.on_hover_text("Pick favourites head-to-head (winner stays on)").on_disabled_hover_text("Shortlist at least two wallpapers").clicked() {
                        self.actions.push(Action::Compare(self.lib_items.clone()));
                    }
                }
                _ => {}
            });
        });
        if self.view == View::Library {
            ui.horizontal(|ui| {
                for (st, label) in [(Some(Status::Kept), "Kept"), (None, "Everything"), (Some(Status::None), "Untriaged")] {
                    if theme::chip(ui, &p, self.lib_status == st && !self.lib_dupes, label).clicked() {
                        self.lib_status = st;
                        self.lib_dupes = false;
                        self.lib_dirty = true;
                    }
                }
                ui.separator();
                if theme::chip(ui, &p, self.lib_dupes, "Duplicates")
                    .on_hover_text("Groups of near-identical wallpapers in your library (from thumbnails seen so far)")
                    .clicked()
                {
                    self.lib_dupes = !self.lib_dupes;
                    self.lib_dirty = true;
                }
            });
        }
        ui.add_space(8.0);
    }
}

impl App {
    /// Key of the wallpaper walle put on this display.
    fn shown_on(&self, display: &str) -> Option<&str> {
        self.shown.on(display).map(|e| e.key.as_str())
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        self.hidden = false;
    }

    /// Theme or accent changed: everything outside egui that shows them has to follow.
    fn refresh_window_look(&self, ctx: &egui::Context) {
        let accent = self.config.accent;
        ctx.send_viewport_cmd(egui::ViewportCommand::Icon(Some(std::sync::Arc::new(crate::icon::window_icon(accent)))));
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        if let Some(t) = &self.tray {
            t.set_accent(accent);
        }
        #[cfg(windows)]
        if let Some(window) = self.native_window {
            crate::native_window::style(window, self.pal.border, self.pal.bg, self.pal.dark);
        }
    }

    fn handle_remote(&mut self, ctx: &egui::Context) {
        while let Ok(cmd) = self.remote_rx.try_recv() {
            match cmd {
                Remote::Show => self.show_window(ctx),
                Remote::About => {
                    self.show_window(ctx);
                    self.show_about = true;
                }
                Remote::Next => {
                    self.toast(ToastKind::Info, "Changing wallpaper…");
                    self.worker.send(Command::RotateNow);
                }
                Remote::ToggleRotation => {
                    let mut cfg = self.config.clone();
                    cfg.rotation.enabled = !cfg.rotation.enabled;
                    let on = cfg.rotation.enabled;
                    self.apply_config(cfg);
                    self.toast(ToastKind::Info, if on { "Rotation on" } else { "Rotation paused" });
                }
                Remote::Reopen => {
                    if self.hidden && self.hide_frames == 0 {
                        self.show_window(ctx);
                    }
                }
                Remote::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        // A panel with a tray can come and go while walle runs.
        #[cfg(target_os = "linux")]
        {
            self.has_tray = self.tray.as_ref().is_some_and(|t| t.is_shown());
        }
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        if self.tray_rotation_shown != self.config.rotation.enabled {
            self.tray_rotation_shown = self.config.rotation.enabled;
            if let Some(t) = &self.tray {
                t.set_rotation(self.config.rotation.enabled);
            }
        }
    }

    // ---------- updates ----------

    /// After a restart: say what became of an update, clear away what it left, and
    /// ask for the latest version if a day has passed.
    fn start_updates(&mut self) {
        // Tests neither report on, clean up after, nor ask the network about updates.
        if cfg!(test) {
            return;
        }
        match crate::update::Updater::restart_note() {
            Some(Ok(msg)) => self.toast(ToastKind::Success, msg),
            Some(Err(msg)) => self.toast(ToastKind::Error, msg),
            None => {}
        }
        self.updater.clean_up();
        let due = crate::update::check_due(self.config.check_updates, self.update_seen.last_check, crate::update::now());
        // A development build never updates itself, so it has no reason to ask.
        if due && !cfg!(debug_assertions) {
            self.check_for_updates(false);
        }
    }

    fn update_notify(&self) -> crate::update::Notify {
        let ctx = self.ctx.clone();
        std::sync::Arc::new(move || ctx.request_repaint())
    }

    /// Ask GitHub for the latest release. `manual`: the user asked.
    pub(super) fn check_for_updates(&mut self, manual: bool) {
        self.updater.check(manual, self.update_notify());
    }

    /// Download, verify and install the update on offer. Only ever on a click.
    pub(super) fn install_update(&mut self) {
        self.updater.install(self.update_notify());
    }

    fn poll_updates(&mut self, ctx: &egui::Context) {
        use crate::update::Event;
        for event in self.updater.poll() {
            match event {
                Event::Checked { found, manual } => {
                    let news = found.is_some() && found != self.update_seen.version;
                    self.update_seen = crate::update::Seen { last_check: Some(crate::update::now()), version: found.clone() };
                    self.update_seen.save(&self.paths.update_file());
                    // A check the user asked for answers where they asked; one found again is not news.
                    // A test-feed build (never a release) can be told to click "Update and restart" itself.
                    #[cfg(feature = "test-feed")]
                    if found.is_some() && std::env::var_os("WALLE_UPDATE_TEST_INSTALL").is_some() {
                        self.install_update();
                    }
                    if let Some(version) = found
                        && news
                        && !manual
                    {
                        self.toast(ToastKind::Info, format!("walle {version} is available. Press F1 to update."));
                    }
                }
                Event::CheckFailed { error, manual } => {
                    // Counts as today's check, so a network that is down is not asked again and again.
                    self.update_seen.last_check = Some(crate::update::now());
                    self.update_seen.save(&self.paths.update_file());
                    if !manual {
                        tracing::info!("update check failed: {error}");
                    }
                }
                Event::Ready => match self.updater.relaunch() {
                    Ok(()) => {
                        self.commit_settings_draft();
                        self.quitting = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    Err(e) => self.toast(ToastKind::Error, e),
                },
                Event::InstallFailed(error) => self.toast(ToastKind::Error, format!("The update was not installed: {error}")),
            }
        }
    }

    /// Closing the window hides it to the tray instead of quitting (when enabled).
    fn handle_close(&mut self, ctx: &egui::Context) {
        let close = ctx.input(|i| i.viewport().close_requested());
        if close {
            self.commit_settings_draft();
        }
        if close && self.has_tray && self.config.close_to_tray && !self.quitting && cfg!(target_os = "linux") {
            // A window cannot be hidden on Wayland: let it close, and go on without one.
            TO_BACKGROUND.store(true, std::sync::atomic::Ordering::Relaxed);
            return;
        }
        if close && self.has_tray && self.config.close_to_tray && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.hidden = true;
            // Hand the focus on, so that a click on the Dock icon activates walle again.
            #[cfg(target_os = "macos")]
            crate::tray::hide_app();
            if !self.tray_hint_shown {
                self.tray_hint_shown = true;
                let hint = if cfg!(target_os = "macos") {
                    "walle keeps running in the menu bar. Quit it from its menu there, or with Cmd+Q."
                } else {
                    "walle keeps running in the tray. Right-click the tray icon to quit."
                };
                self.toast(ToastKind::Info, hint);
            }
        }
    }
}

impl eframe::App for App {
    /// Runs every frame, and also while the window is hidden in the tray.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if std::mem::take(&mut self.theme_dirty) {
            theme::apply(ctx, &self.pal);
            self.refresh_window_look(ctx);
        }
        self.process_events(ctx);
        self.handle_remote(ctx);
        self.handle_picked();
        self.poll_updates(ctx);
        if self.hide_frames > 0 {
            self.hide_frames -= 1;
            if self.hidden {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                #[cfg(target_os = "macos")]
                crate::tray::hide_app();
            }
            ctx.request_repaint();
        }
        self.handle_close(ctx);
        self.flush_dirty();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.textures.begin_frame();
        if let Some(deadline) = self.filter_deadline
            && ctx.input(|i| i.time) >= deadline
        {
            self.filter_deadline = None;
            self.lib_dirty = true;
        }
        if self.view != View::Browse && self.view != View::Settings {
            self.reload_library_view();
        }
        let p = self.pal;

        self.title_bar(ui);
        if self.view == View::Browse {
            egui::Panel::top("toolbar")
                .frame(egui::Frame::default().fill(p.surface).inner_margin(Margin::symmetric(14, 9)))
                .show(ui, |ui| self.browse_toolbar(ui));
        }
        egui::Panel::bottom("status_bar")
            .frame(egui::Frame::default().fill(p.surface).inner_margin(Margin::symmetric(14, 5)))
            .show(ui, |ui| self.status_bar(ui));

        if self.view == View::Settings {
            egui::CentralPanel::default()
                .frame(egui::Frame::default().fill(p.bg).inner_margin(Margin::same(0)))
                .show(ui, |ui| self.settings_ui(ui));
        } else {
            // Not the old ids "sidebar" and "detail": those saved sizes are the stretched ones.
            egui::Panel::left("side-panel")
                .resizable(true)
                .default_size(240.0)
                .size_range(200.0..=380.0)
                .frame(egui::Frame::default().fill(p.surface).inner_margin(Margin::same(14)))
                .show(ui, |ui| {
                    panel_scroll(ui, |ui| {
                        if self.view == View::Browse {
                            self.browse_sidebar(ui)
                        } else {
                            self.library_sidebar(ui)
                        }
                    });
                });
            if self.selected.is_some() {
                egui::Panel::right("detail-panel")
                    .resizable(true)
                    .default_size(360.0)
                    .size_range(300.0..=640.0)
                    .frame(egui::Frame::default().fill(p.surface).inner_margin(Margin::same(14)))
                    .show(ui, |ui| {
                        panel_scroll(ui, |ui| self.detail_ui(ui));
                    });
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::default().fill(p.bg).inner_margin(Margin::same(14)))
                .show(ui, |ui| {
                    if self.view != View::Browse {
                        self.library_header(ui);
                    }
                    self.bulk_bar(ui);
                    self.grid_ui(ui);
                });
        }

        self.lightbox_ui(&ctx);
        self.duel_ui(&ctx);
        if ctx.input(|i| i.key_pressed(Key::F1)) {
            self.show_about = !self.show_about;
        }
        // Noted before drawing: the Esc that closes the box must not also clear the selection.
        let about_open = self.show_about;
        self.about_ui(&ctx);
        self.handle_drops(&ctx);
        self.toasts.show(&ctx, &p);
        if !about_open {
            self.handle_keys(&ctx);
        }
        self.apply_actions(&ctx);
        self.window_resize_edges(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // Quick tweaks (thumb size, fit mode, show rejected) are persisted too.
        if self.config != self.saved_config {
            match self.config.save(&self.paths) {
                Ok(()) => self.saved_config = self.config.clone(),
                Err(e) => tracing::warn!("saving config: {e:#}"),
            }
        }
        eframe::set_value(
            storage,
            eframe::APP_KEY,
            &Persisted { provider: self.provider.clone(), query: self.query.clone(), view: self.view },
        );
    }
}

#[cfg(test)]
mod tests;
