//! System tray icon (Windows; the menu bar on macOS). Menu: open, next wallpaper, pause
//! rotation, quit.
//! Events are forwarded through a channel and wake the UI with `request_repaint`, which also
//! works while the window is hidden (eframe then runs `App::logic` only).
use std::sync::mpsc::Sender;

use anyhow::Result;
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

pub use crate::ui::Remote as TrayCmd;

pub struct Tray {
    icon: TrayIcon,
    rotation: CheckMenuItem,
    #[cfg(target_os = "macos")]
    _activation: mac::Observer,
}

impl Tray {
    pub fn new(ctx: egui::Context, tx: Sender<TrayCmd>, accent: [u8; 3], rotation_on: bool) -> Result<Self> {
        let open = MenuItem::new("Open walle", true, None);
        let next = MenuItem::new("Next wallpaper", true, None);
        let rotation = CheckMenuItem::new("Rotate automatically", true, rotation_on, None);
        let about = MenuItem::new("About walle", true, None);
        let quit = MenuItem::new("Quit", true, None);
        let menu = Menu::new();
        menu.append(&open)?;
        menu.append(&next)?;
        menu.append(&rotation)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&about)?;
        menu.append(&quit)?;

        let ids = [
            (open.id().clone(), TrayCmd::Show),
            (next.id().clone(), TrayCmd::Next),
            (rotation.id().clone(), TrayCmd::ToggleRotation),
            (about.id().clone(), TrayCmd::About),
            (quit.id().clone(), TrayCmd::Quit),
        ];
        let (tx2, ctx2) = (tx.clone(), ctx.clone());
        #[cfg(target_os = "macos")]
        let (tx_reopen, ctx_reopen) = (tx.clone(), ctx.clone());
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            if let Some((_, cmd)) = ids.iter().find(|(id, _)| *id == e.id) {
                let _ = tx2.send(*cmd);
                ctx2.request_repaint();
            }
        }));
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            let open = matches!(
                e,
                TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                    | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. }
            );
            if open {
                let _ = tx.send(TrayCmd::Show);
                ctx.request_repaint();
            }
        }));
        #[cfg(target_os = "macos")]
        let activation = mac::Observer::activation(Box::new(move || {
            let _ = tx_reopen.send(TrayCmd::Reopen);
            ctx_reopen.request_repaint();
        }));

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            // A menu bar item opens its menu on a click; a tray icon opens the window.
            .with_menu_on_left_click(cfg!(target_os = "macos"))
            .with_tooltip("walle")
            .with_icon(Icon::from_rgba(crate::icon::rgba(32, accent), 32, 32)?)
            .build()?;
        Ok(Self {
            icon,
            rotation,
            #[cfg(target_os = "macos")]
            _activation: activation,
        })
    }

    pub fn set_rotation(&self, on: bool) {
        self.rotation.set_checked(on);
    }

    /// Redraw the icon after the accent colour changed.
    pub fn set_accent(&self, accent: [u8; 3]) {
        match Icon::from_rgba(crate::icon::rgba(32, accent), 32, 32) {
            Ok(icon) => {
                let _ = self.icon.set_icon(Some(icon));
            }
            Err(e) => tracing::warn!("tray icon: {e}"),
        }
    }

    pub fn set_tooltip(&self, text: &str) {
        // Windows limits tray tooltips to 127 characters.
        let t: String = text.chars().take(120).collect();
        let _ = self.icon.set_tooltip(Some(t));
    }
}

/// Hide the app (macOS): its windows go, and the focus passes to the next app, so that a click
/// on walle's Dock icon activates it again (see [`mac::Observer::activation`]).
#[cfg(target_os = "macos")]
pub fn hide_app() {
    if let Some(mtm) = objc2_foundation::MainThreadMarker::new() {
        objc2_app_kit::NSApplication::sharedApplication(mtm).hide(None);
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ptr::NonNull;

    use block2::RcBlock;
    use objc2::{rc::Retained, runtime::ProtocolObject};
    use objc2_app_kit::NSApplicationDidBecomeActiveNotification;
    use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol};

    /// An observer of the app's notifications; removed again when dropped.
    pub struct Observer(Retained<ProtocolObject<dyn NSObjectProtocol>>);

    impl Observer {
        /// Calls `f` whenever walle becomes the active app: from the Dock, Spotlight, the
        /// Finder or the app switcher. winit has no event for a click on the Dock icon.
        pub fn activation(f: Box<dyn Fn()>) -> Self {
            let block = RcBlock::new(move |_: NonNull<NSNotification>| f());
            let center = NSNotificationCenter::defaultCenter();
            // SAFETY: no object filter, no queue (the block runs where the notification is
            // posted: AppKit posts it on the main thread), and the block only sends on a channel.
            let token = unsafe {
                center.addObserverForName_object_queue_usingBlock(
                    Some(NSApplicationDidBecomeActiveNotification),
                    None,
                    None,
                    &block,
                )
            };
            Self(token)
        }
    }

    impl Drop for Observer {
        fn drop(&mut self) {
            // SAFETY: the token is the observer `addObserverForName` returned.
            unsafe { NSNotificationCenter::defaultCenter().removeObserver(self.0.as_ref()) };
        }
    }
}
