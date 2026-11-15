//! Start walle on login: HKCU Run key on Windows, XDG autostart entry on Linux.
//! The registered command passes `--minimized` so walle starts in the tray/minimized.
use anyhow::{Context, Result};

pub const FLAG: &str = "--minimized";

fn command_line() -> Result<String> {
    let exe = std::env::current_exe().context("locating walle executable")?;
    Ok(format!("\"{}\" {FLAG}", exe.display()))
}

#[cfg(windows)]
mod imp {
    use anyhow::{Result, bail};
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegDeleteValueW,
            RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
        },
    };

    const RUN: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const NAME: &str = "walle";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn open(access: u32) -> Result<HKEY> {
        let mut key: HKEY = std::ptr::null_mut();
        let rc = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, wide(RUN).as_ptr(), 0, access, &mut key) };
        if rc != ERROR_SUCCESS {
            bail!("opening Run key failed: {rc}");
        }
        Ok(key)
    }

    pub fn get() -> Option<String> {
        let key = open(KEY_QUERY_VALUE).ok()?;
        let name = wide(NAME);
        let mut buf = vec![0u16; 2048];
        let mut len = (buf.len() * 2) as u32;
        let mut kind = 0u32;
        let rc = unsafe {
            RegQueryValueExW(key, name.as_ptr(), std::ptr::null_mut(), &mut kind, buf.as_mut_ptr() as *mut u8, &mut len)
        };
        unsafe { RegCloseKey(key) };
        if rc != ERROR_SUCCESS || kind != REG_SZ {
            return None;
        }
        let n = (len as usize / 2).min(buf.len());
        let s = String::from_utf16_lossy(&buf[..n]);
        Some(s.trim_end_matches('\0').to_string())
    }

    pub fn set(cmd: Option<&str>) -> Result<()> {
        let key = open(KEY_SET_VALUE)?;
        let name = wide(NAME);
        let rc = match cmd {
            Some(c) => {
                let v = wide(c);
                unsafe { RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, v.as_ptr() as *const u8, (v.len() * 2) as u32) }
            }
            None => unsafe { RegDeleteValueW(key, name.as_ptr()) },
        };
        unsafe { RegCloseKey(key) };
        if rc != ERROR_SUCCESS && !(cmd.is_none() && rc == ERROR_FILE_NOT_FOUND) {
            bail!("updating Run key failed: {rc}");
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use anyhow::Result;
    use std::path::PathBuf;

    fn entry() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("autostart").join("walle.desktop"))
    }

    pub fn get() -> Option<String> {
        let s = std::fs::read_to_string(entry()?).ok()?;
        s.lines().find_map(|l| l.strip_prefix("Exec=")).map(String::from)
    }

    pub fn set(cmd: Option<&str>) -> Result<()> {
        let Some(path) = entry() else { anyhow::bail!("no home directory") };
        match cmd {
            Some(c) => {
                std::fs::create_dir_all(path.parent().unwrap())?;
                std::fs::write(
                    &path,
                    format!(
                        "[Desktop Entry]\nType=Application\nName=walle\nComment=Wallpaper browser and rotator\nExec={c}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
                    ),
                )?;
            }
            None => {
                let _ = std::fs::remove_file(path);
            }
        }
        Ok(())
    }
}

pub fn is_enabled() -> bool {
    imp::get().is_some()
}

/// Enable/disable start-on-login for the current executable.
pub fn set_enabled(on: bool) -> Result<()> {
    if on {
        imp::set(Some(&command_line()?))
    } else {
        imp::set(None)
    }
}

/// True if the registered entry points at an executable that no longer exists
/// (walle was moved). A different-but-existing walle (e.g. a dev build) is left alone.
pub fn is_stale() -> bool {
    let Some(reg) = imp::get() else { return false };
    let exe = reg.trim_start_matches('"').split('"').next().unwrap_or("").trim();
    !exe.is_empty() && !std::path::Path::new(exe).exists()
}

#[cfg(test)]
mod tests {
    #[test]
    fn command_line_quotes_exe_and_flag() {
        let c = super::command_line().unwrap();
        assert!(c.starts_with('"') && c.ends_with(super::FLAG), "{c}");
    }
}
