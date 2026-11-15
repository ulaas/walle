//! Tray icon on Linux: a StatusNotifierItem on the session bus, which is what the panels of
//! gevrek, KDE and others (GNOME with an extension) show. Same menu as on Windows: open, next
//! wallpaper, pause rotation, about, quit.
//!
//! Wayland has no way to hide a window, so on Linux walle does not hide to the tray: it
//! closes its window for good and goes on without one (see `background`).
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::Sender,
};

use anyhow::Result;
use ksni::blocking::TrayMethods;

pub use crate::ui::Remote as TrayCmd;

struct Item {
    tx: Sender<TrayCmd>,
    /// Called after a command was sent (wakes whoever reads them).
    wake: Box<dyn Fn() + Send>,
    accent: [u8; 3],
    rotation_on: bool,
    tooltip: String,
    /// A panel is there to show the icon.
    shown: Arc<AtomicBool>,
}

impl Item {
    fn send(&self, cmd: TrayCmd) {
        let _ = self.tx.send(cmd);
        (self.wake)();
    }
}

/// walle's icon at `size`, as the bus wants it: ARGB, most significant byte first.
fn pixmap(size: u32, accent: [u8; 3]) -> ksni::Icon {
    let mut data = crate::icon::rgba(size, accent);
    for px in data.as_chunks_mut::<4>().0 {
        px.rotate_right(1);
    }
    ksni::Icon { width: size as i32, height: size as i32, data }
}

impl ksni::Tray for Item {
    fn id(&self) -> String {
        "walle".into()
    }

    fn title(&self) -> String {
        "walle".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        [22, 32, 48, 64].into_iter().map(|size| pixmap(size, self.accent)).collect()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: self.tooltip.clone(), ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayCmd::Show);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::{CheckmarkItem, StandardItem};
        let item = |label: &str, cmd: TrayCmd| -> ksni::MenuItem<Self> {
            StandardItem { label: label.into(), activate: Box::new(move |this: &mut Self| this.send(cmd)), ..Default::default() }.into()
        };
        vec![
            item("Open walle", TrayCmd::Show),
            item("Next wallpaper", TrayCmd::Next),
            CheckmarkItem {
                label: "Rotate automatically".into(),
                checked: self.rotation_on,
                activate: Box::new(|this: &mut Self| this.send(TrayCmd::ToggleRotation)),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            item("About walle", TrayCmd::About),
            item("Quit", TrayCmd::Quit),
        ]
    }

    fn watcher_online(&self) {
        self.shown.store(true, Ordering::Relaxed);
    }

    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        self.shown.store(false, Ordering::Relaxed);
        // Stay: a panel may come (back) later, as at login, when walle can be first.
        true
    }
}

pub struct Tray {
    handle: ksni::blocking::Handle<Item>,
    shown: Arc<AtomicBool>,
}

impl Tray {
    pub fn new(wake: impl Fn() + Send + 'static, tx: Sender<TrayCmd>, accent: [u8; 3], rotation_on: bool) -> Result<Self> {
        // Shown unless told otherwise: without a panel `watcher_offline` is called before
        // `spawn` returns.
        let shown = Arc::new(AtomicBool::new(true));
        let item = Item { tx, wake: Box::new(wake), accent, rotation_on, tooltip: "walle".into(), shown: shown.clone() };
        let handle = item.assume_sni_available(true).spawn().map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(Self { handle, shown })
    }

    /// A panel shows the icon now. Without one, closing the window must not leave walle
    /// running where nobody can reach it.
    pub fn is_shown(&self) -> bool {
        self.shown.load(Ordering::Relaxed) && !self.handle.is_closed()
    }

    pub fn set_rotation(&self, on: bool) {
        self.handle.update(|item| item.rotation_on = on);
    }

    /// Redraw the icon after the accent colour changed.
    pub fn set_accent(&self, accent: [u8; 3]) {
        self.handle.update(|item| item.accent = accent);
    }

    pub fn set_tooltip(&self, text: &str) {
        let text = text.to_string();
        self.handle.update(|item| item.tooltip = text);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pixmap_is_argb() {
        let accent = [139, 124, 246];
        let rgba = crate::icon::rgba(32, accent);
        let icon = super::pixmap(32, accent);
        assert_eq!((icon.width, icon.height, icon.data.len()), (32, 32, 32 * 32 * 4));
        let c = (16 * 32 + 16) * 4;
        assert_eq!(icon.data[c], rgba[c + 3], "alpha first");
        assert_eq!(icon.data[c + 1..c + 4], rgba[c..c + 3], "then red, green, blue");
    }
}
