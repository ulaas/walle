use std::{os::windows::ffi::OsStrExt, path::Path};

use anyhow::{Result, bail};
use windows_sys::Win32::{
    Foundation::ERROR_SUCCESS,
    System::Registry::{HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegOpenKeyExW, RegSetValueExW},
    UI::WindowsAndMessaging::{SPI_GETDESKWALLPAPER, SPI_SETDESKWALLPAPER, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE, SystemParametersInfoW},
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn set_reg_sz(key: HKEY, name: &str, value: &str) -> Result<()> {
    let name = wide(name);
    let value = wide(value);
    let rc = unsafe {
        RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, value.as_ptr() as *const u8, (value.len() * 2) as u32)
    };
    if rc != ERROR_SUCCESS {
        bail!("RegSetValueExW failed: {rc}");
    }
    Ok(())
}

pub fn set(path: &Path, fit: crate::model::FitMode) -> Result<()> {
    use crate::model::FitMode::*;
    // (WallpaperStyle, TileWallpaper)
    let (style, tile) = match fit {
        Fill => ("10", "0"),
        Fit => ("6", "0"),
        Stretch => ("2", "0"),
        Center => ("0", "0"),
        Tile => ("0", "1"),
        Span => ("22", "0"),
    };
    unsafe {
        let subkey = wide("Control Panel\\Desktop");
        let mut hkey: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_SET_VALUE, &mut hkey) == ERROR_SUCCESS {
            let r1 = set_reg_sz(hkey, "WallpaperStyle", style);
            let r2 = set_reg_sz(hkey, "TileWallpaper", tile);
            RegCloseKey(hkey);
            if let Err(e) = r1.and(r2) {
                tracing::warn!("could not set fit mode: {e}");
            }
        }
    }

    // canonicalize() yields a \\?\ verbatim path which SPI does not accept.
    let s = path.as_os_str().to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    let mut wpath: Vec<u16> = std::ffi::OsStr::new(s).encode_wide().collect();
    wpath.push(0);
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_SETDESKWALLPAPER,
            0,
            wpath.as_mut_ptr() as *mut _,
            SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
        )
    };
    if ok == 0 {
        bail!("SystemParametersInfoW failed: {}", std::io::Error::last_os_error());
    }
    Ok(())
}

/// Path of the current desktop wallpaper, if any.
pub fn current() -> Option<std::path::PathBuf> {
    let mut buf = vec![0u16; 1024];
    let ok = unsafe { SystemParametersInfoW(SPI_GETDESKWALLPAPER, buf.len() as u32, buf.as_mut_ptr() as *mut _, 0) };
    if ok == 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
    (len > 0).then(|| std::path::PathBuf::from(String::from_utf16_lossy(&buf[..len])))
}
