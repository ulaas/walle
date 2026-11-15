//! Native file/folder dialogs (rfd). Dialogs run on a helper thread so the UI keeps painting;
//! results come back through a channel and are handled in `App::logic`.
use std::path::PathBuf;

use super::{Action, App, View};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    /// Add a folder to the "Local folders" source.
    LocalFolder,
    /// Import images into the library.
    ImportImages,
    /// Restore a walle export file.
    ImportExport,
    /// Choose where to write an export.
    ExportTo,
}

pub enum Picked {
    LocalFolder(PathBuf),
    ImportImages(Vec<PathBuf>),
    ImportExport(PathBuf),
    ExportTo(PathBuf),
}

const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "gif"];

impl App {
    pub(super) fn open_picker(&mut self, kind: Pick) {
        if self.picker_busy {
            return;
        }
        self.picker_busy = true;
        let tx = self.picker_tx.clone();
        let ctx = self.ctx.clone();
        let export_dir = self.paths.data_dir.join("exports");
        let pictures = directories::UserDirs::new().and_then(|u| u.picture_dir().map(|p| p.to_path_buf()));
        std::thread::spawn(move || {
            let mut d = rfd::FileDialog::new();
            if let Some(p) = &pictures {
                d = d.set_directory(p);
            }
            let picked = match kind {
                Pick::LocalFolder => d.set_title("Add a wallpaper folder").pick_folder().map(Picked::LocalFolder),
                Pick::ImportImages => d
                    .set_title("Import images into your library")
                    .add_filter("Images", IMAGE_EXTS)
                    .pick_files()
                    .map(Picked::ImportImages),
                Pick::ImportExport => rfd::FileDialog::new()
                    .set_title("Import a walle library export")
                    .set_directory(&export_dir)
                    .add_filter("walle export", &["json"])
                    .pick_file()
                    .map(Picked::ImportExport),
                Pick::ExportTo => {
                    let _ = std::fs::create_dir_all(&export_dir);
                    rfd::FileDialog::new()
                        .set_title("Export library")
                        .set_directory(&export_dir)
                        .set_file_name(format!("walle-library-{}.json", chrono::Local::now().format("%Y%m%d-%H%M%S")))
                        .add_filter("walle export", &["json"])
                        .save_file()
                        .map(Picked::ExportTo)
                }
            };
            let _ = tx.send(picked);
            ctx.request_repaint();
        });
    }

    pub(super) fn handle_picked(&mut self) {
        while let Ok(res) = self.picker_rx.try_recv() {
            self.picker_busy = false;
            match res {
                None => {}
                Some(Picked::LocalFolder(dir)) => {
                    if self.view == View::Settings
                        && let Some(d) = &mut self.settings_draft
                    {
                        if !d.local_folders.contains(&dir) {
                            d.local_folders.push(dir);
                            d.providers.entry("local".into()).or_default().enabled = true;
                        }
                    } else {
                        self.actions.push(Action::AddFolder(dir));
                    }
                }
                Some(Picked::ImportImages(files)) => self.actions.push(Action::ImportFiles(files)),
                Some(Picked::ImportExport(f)) => self.actions.push(Action::ImportLibrary(f)),
                Some(Picked::ExportTo(f)) => self.actions.push(Action::Export(Some(f))),
            }
        }
    }
}
