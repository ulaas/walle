//! Windows: what the OS should still draw for our frameless window. Without a native frame there
//! is no shadow or rounded corners unless DWM (the compositor) is asked for them explicitly.
use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};

use egui::Color32;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::{
    Foundation::{FALSE, HWND, TRUE},
    Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
        DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
    },
    Graphics::Gdi::{CreateSolidBrush, DeleteObject},
    UI::Controls::MARGINS,
    UI::WindowsAndMessaging::{GCLP_HBRBACKGROUND, SetClassLongPtrW},
};

/// The window handle as a plain number, so that `App` needn't hold a raw pointer.
pub fn handle_of(window: &impl HasWindowHandle) -> Option<isize> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

fn set_attribute<T>(hwnd: HWND, attribute: i32, value: &T) {
    // Fails harmlessly on Windows versions without the attribute (corners and border are 11+).
    unsafe {
        DwmSetWindowAttribute(hwnd, attribute as u32, value as *const T as *const c_void, size_of::<T>() as u32);
    }
}

/// COLORREF is 0x00BBGGRR.
fn colorref(c: Color32) -> u32 {
    u32::from(c.r()) | u32::from(c.g()) << 8 | u32::from(c.b()) << 16
}

/// The brush [`background`] gave the window class (0: none yet).
static BACKGROUND_BRUSH: AtomicIsize = AtomicIsize::new(0);

/// What Windows paints where a resize uncovers the window, until our next frame is there. The
/// window class comes without a background brush, and the unpainted part shows white.
fn background(hwnd: HWND, color: Color32) {
    // The brush belongs to the class from here on; the one it replaces is ours and unused.
    unsafe {
        let brush = CreateSolidBrush(colorref(color));
        if brush.is_null() {
            return;
        }
        SetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND, brush as isize);
        let old = BACKGROUND_BRUSH.swap(brush as isize, Ordering::Relaxed);
        if old != 0 {
            DeleteObject(old as _);
        }
    }
}

/// Rounded corners, a drop shadow, a border in the app's colour, and the app's background where
/// a resize uncovers the window.
pub fn style(handle: isize, border: Color32, bg: Color32, dark: bool) {
    let hwnd = handle as HWND;
    background(hwnd, bg);
    set_attribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &DWMWCP_ROUND);
    set_attribute(hwnd, DWMWA_BORDER_COLOR, &colorref(border));
    set_attribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &(if dark { TRUE } else { FALSE }));
    // A frame of any size makes DWM treat the window as framed, which brings the shadow. One pixel
    // at the bottom is covered by our own drawing, so nothing of it shows.
    let margins = MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 0, cyBottomHeight: 1 };
    unsafe {
        DwmExtendFrameIntoClientArea(hwnd, &margins);
    }
}
