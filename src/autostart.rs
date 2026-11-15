//! Start walle on login: HKCU Run key on Windows, XDG autostart entry on Linux, a launch
//! agent in ~/Library/LaunchAgents on macOS.
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

/// macOS: a per-user launch agent that launchd starts at login. It is only written, not
/// loaded: enabling it starts nothing now.
#[cfg(target_os = "macos")]
mod imp {
    use anyhow::{Context, Result};
    use std::path::PathBuf;

    const LABEL: &str = "io.github.ulaas.walle";

    fn entry() -> Option<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        Some(home.join("Library").join("LaunchAgents").join(format!("{LABEL}.plist")))
    }

    fn escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    fn unescape(s: &str) -> String {
        s.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
    }

    /// `"exe" args` (what `command_line` makes) as program arguments.
    fn split(cmd: &str) -> Vec<String> {
        let rest = cmd.trim().trim_start_matches('"');
        let (exe, args) = rest.split_once('"').unwrap_or((rest, ""));
        std::iter::once(exe.to_string()).chain(args.split_whitespace().map(String::from)).collect()
    }

    /// The program arguments of the agent, back as `"exe" args`.
    pub fn get() -> Option<String> {
        parse(&std::fs::read_to_string(entry()?).ok()?)
    }

    fn parse(s: &str) -> Option<String> {
        let array = s.split_once("<key>ProgramArguments</key>")?.1.split_once("</array>")?.0;
        let args: Vec<String> = array
            .split("<string>")
            .skip(1)
            .filter_map(|a| a.split_once("</string>").map(|(v, _)| unescape(v)))
            .collect();
        let (exe, rest) = args.split_first()?;
        Some(std::iter::once(format!("\"{exe}\"")).chain(rest.iter().cloned()).collect::<Vec<_>>().join(" "))
    }

    fn plist(cmd: &str) -> String {
        let args: String = split(cmd).iter().map(|a| format!("\t\t<string>{}</string>\n", escape(a))).collect();
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n<dict>\n\
             \t<key>Label</key>\n\t<string>{LABEL}</string>\n\
             \t<key>ProgramArguments</key>\n\t<array>\n{args}\t</array>\n\
             \t<key>RunAtLoad</key>\n\t<true/>\n\
             \t<key>ProcessType</key>\n\t<string>Interactive</string>\n\
             </dict>\n</plist>\n"
        )
    }

    pub fn set(cmd: Option<&str>) -> Result<()> {
        let Some(path) = entry() else { anyhow::bail!("no home directory") };
        match cmd {
            Some(c) => {
                std::fs::create_dir_all(path.parent().unwrap())?;
                std::fs::write(&path, plist(c)).with_context(|| format!("writing {}", path.display()))?;
            }
            None => {
                let _ = std::fs::remove_file(path);
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn arguments_round_trip() {
            let cmd = r#""/Applications/walle & co.app/Contents/MacOS/walle" --minimized"#;
            assert_eq!(super::split(cmd), ["/Applications/walle & co.app/Contents/MacOS/walle", "--minimized"]);
            let plist = super::plist(cmd);
            assert!(plist.contains("<string>/Applications/walle &amp; co.app/Contents/MacOS/walle</string>"), "{plist}");
            assert_eq!(super::parse(&plist).as_deref(), Some(cmd));
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
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
