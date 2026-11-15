//! The macOS app replaces itself: the release's disk image is mounted out of
//! sight, the walle.app in it is checked (same bundle, the release's version,
//! signed by the same Developer ID team as the running app, accepted by
//! Gatekeeper), copied beside the installed bundle and swapped in. The new
//! app is opened once the running one has quit.
//!
//! The parts that only look at paths and text are here for every system, so
//! their tests run everywhere; the parts that run Apple's tools are macOS only.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::path::{Path, PathBuf};

/// The requirement a new bundle's signature must meet: a Developer ID
/// Application certificate issued by Apple, for `team`.
pub fn requirement(team: &str) -> String {
    format!(
        "anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists \
         and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = \"{team}\""
    )
}

/// The `walle.app` an executable at `<app>/Contents/MacOS/<exe>` belongs to.
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    let is_app = app.extension().is_some_and(|e| e.eq_ignore_ascii_case("app"));
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && is_app).then(|| app.to_path_buf())
}

/// Whether a path lies in Homebrew's Caskroom.
pub fn in_caskroom(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str().eq_ignore_ascii_case("Caskroom"))
}

/// Whether macOS runs the app from where App Translocation put it: an app opened
/// from a quarantined download in place.
pub fn translocated(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == "AppTranslocation")
}

/// The team of a signature, from what `codesign -dv --verbose=2` prints: `None`
/// for an ad hoc signature (`TeamIdentifier=not set`) or anything that is not a team ID.
pub fn parse_team(codesign: &str) -> Option<String> {
    let team = codesign.lines().find_map(|l| l.trim().strip_prefix("TeamIdentifier="))?.trim();
    (team.len() == 10 && team.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())).then(|| team.to_string())
}

/// The bundle ID and the version of an app, from its Info.plist as JSON.
pub fn parse_info(json: &str) -> Result<(String, String), String> {
    let info: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("its Info.plist is unreadable ({e})"))?;
    let field = |key: &str| info.get(key).and_then(|v| v.as_str()).map(str::to_string);
    match (field("CFBundleIdentifier"), field("CFBundleShortVersionString")) {
        (Some(id), Some(version)) => Ok((id, version)),
        _ => Err("its Info.plist does not name the app and its version".into()),
    }
}

/// Whether the app in the disk image is the update to this one.
pub fn check_info(running_id: &str, new: &(String, String), version: &str) -> Result<(), String> {
    if new.0 != running_id {
        return Err(format!("the disk image holds {}, not {running_id}; nothing was installed", new.0));
    }
    if new.1 != version {
        return Err(format!("the disk image holds version {}, not {version}; nothing was installed", new.1));
    }
    Ok(())
}

/// The mount point in what `hdiutil attach -plist` prints.
pub fn mount_point(plist: &str) -> Option<PathBuf> {
    let mut rest = plist;
    while let Some(at) = rest.find("<key>mount-point</key>") {
        rest = &rest[at + "<key>mount-point</key>".len()..];
        let value = rest.trim_start().strip_prefix("<string>")?;
        let value = &value[..value.find("</string>")?];
        let value = value.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'");
        let value = value.replace("&amp;", "&");
        if !value.is_empty() {
            return Some(PathBuf::from(value));
        }
    }
    None
}

/// Where the new bundle is copied before the swap, and where the previous one
/// waits while it happens: hidden, beside the installed bundle, on its volume.
pub fn side_paths(app: &Path) -> (PathBuf, PathBuf) {
    let name = app.file_name().unwrap_or_default().to_string_lossy();
    let pid = std::process::id();
    (app.with_file_name(format!(".{name}.update-{pid}")), app.with_file_name(format!(".{name}.old-{pid}")))
}

/// Whether `file` is something an earlier update of `app` left beside it.
pub fn is_leftover(app_name: &str, file: &str) -> bool {
    [".update-", ".old-"].iter().any(|kind| {
        file.strip_prefix(&format!(".{app_name}{kind}")).is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// Swap two paths in one step (`renamex_np` with `RENAME_SWAP`).
#[cfg(target_os = "macos")]
fn rename_swap(a: &Path, b: &Path) -> std::io::Result<()> {
    use std::ffi::{CString, c_char, c_int, c_uint};
    use std::os::unix::ffi::OsStrExt;
    unsafe extern "C" {
        fn renamex_np(from: *const c_char, to: *const c_char, flags: c_uint) -> c_int;
    }
    const RENAME_SWAP: c_uint = 0x2;
    let path = |p: &Path| CString::new(p.as_os_str().as_bytes()).map_err(std::io::Error::other);
    let (a, b) = (path(a)?, path(b)?);
    // SAFETY: both paths are NUL-terminated and outlive the call.
    if unsafe { renamex_np(a.as_ptr(), b.as_ptr(), RENAME_SWAP) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Put the bundle at `staged` in place of `installed`, all or nothing. With
/// `atomic` the two are swapped in one step; otherwise, or where that is not
/// supported, `installed` is renamed to `backup` and `staged` to `installed`,
/// and undone if the second rename fails. Either way nothing is left at
/// `staged` or `backup` afterwards, and `installed` is never half a bundle.
pub fn replace_bundle(installed: &Path, staged: &Path, backup: &Path, atomic: bool) -> Result<(), String> {
    let drop_staged = || {
        let _ = std::fs::remove_dir_all(staged).or_else(|_| std::fs::remove_file(staged));
    };
    if !installed.is_dir() {
        drop_staged();
        return Err(format!("{} is not there any more; nothing was installed", installed.display()));
    }
    #[cfg(target_os = "macos")]
    if atomic && rename_swap(staged, installed).is_ok() {
        // `staged` now holds the previous version.
        if let Err(e) = std::fs::remove_dir_all(staged) {
            tracing::warn!("update: cannot remove the previous version at {}: {e}", staged.display());
        }
        return Ok(());
    }
    let _ = atomic;
    if let Err(e) = std::fs::rename(installed, backup) {
        drop_staged();
        return Err(format!("cannot set aside {}: {e}; nothing was installed", installed.display()));
    }
    if let Err(e) = std::fs::rename(staged, installed) {
        let back = std::fs::rename(backup, installed);
        drop_staged();
        return Err(match back {
            Ok(()) => format!("cannot put the new version in place ({e}); the previous one is unchanged"),
            Err(r) => format!(
                "cannot put the new version in place ({e}), and the previous one could not be put back ({r}); it is at {}",
                backup.display()
            ),
        });
    }
    if let Err(e) = std::fs::remove_dir_all(backup) {
        tracing::warn!("update: cannot remove the previous version at {}: {e}", backup.display());
    }
    Ok(())
}

/// Remove what an interrupted update left beside `app`.
pub fn clean_up(app: &Path) {
    let (Some(dir), Some(name)) = (app.parent(), app.file_name()) else { return };
    let name = name.to_string_lossy();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let file = entry.file_name();
        if is_leftover(&name, &file.to_string_lossy()) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(target_os = "macos")]
pub use tools::*;

/// What runs Apple's tools. Full paths: an app started from the Finder has a minimal PATH.
#[cfg(target_os = "macos")]
mod tools {
    use super::*;
    use crate::update::Progress;
    use std::process::{Command, Stdio};

    const HDIUTIL: &str = "/usr/bin/hdiutil";
    const CODESIGN: &str = "/usr/bin/codesign";
    const SPCTL: &str = "/usr/sbin/spctl";
    const DITTO: &str = "/usr/bin/ditto";
    const PLUTIL: &str = "/usr/bin/plutil";

    struct Ran {
        ok: bool,
        stdout: String,
        stderr: String,
    }

    fn run(tool: &str, args: &[&std::ffi::OsStr]) -> Result<Ran, String> {
        let out = Command::new(tool)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("cannot run {tool}: {e}"))?;
        Ok(Ran {
            ok: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        })
    }

    fn os(s: &str) -> &std::ffi::OsStr {
        std::ffi::OsStr::new(s)
    }

    /// The Developer ID team that signed `app`; `None` for an ad hoc signature or none.
    pub fn team_of(app: &Path) -> Option<String> {
        let ran = run(CODESIGN, &[os("-dv"), os("--verbose=2"), app.as_os_str()]).ok()?;
        // codesign describes a signature on stderr.
        if ran.ok { parse_team(&ran.stderr) } else { None }
    }

    pub fn info_of(app: &Path) -> Result<(String, String), String> {
        let plist = app.join("Contents/Info.plist");
        let ran = run(PLUTIL, &[os("-convert"), os("json"), os("-o"), os("-"), plist.as_os_str()])?;
        if !ran.ok {
            return Err(format!("cannot read {}: {}", plist.display(), ran.stderr));
        }
        parse_info(&ran.stdout)
    }

    /// The signature is whole, and from a Developer ID of `team`, and Gatekeeper
    /// accepts the app (it is notarized).
    pub fn check_signature(app: &Path, team: &str) -> Result<(), String> {
        let ran = run(CODESIGN, &[os("--verify"), os("--deep"), os("--strict"), app.as_os_str()])?;
        if !ran.ok {
            return Err(format!("the new app's signature is not valid ({}); nothing was installed", ran.stderr));
        }
        let req = format!("-R={}", requirement(team));
        let ran = run(CODESIGN, &[os("--verify"), os(&req), app.as_os_str()])?;
        if !ran.ok {
            return Err(format!(
                "the new app is not signed by the same developer (team {team}) as this one; nothing was installed"
            ));
        }
        let ran = run(SPCTL, &[os("--assess"), os("--type"), os("execute"), app.as_os_str()])?;
        if !ran.ok {
            return Err(format!("macOS does not accept the new app ({}); nothing was installed", ran.stderr));
        }
        Ok(())
    }

    /// A disk image mounted read-only, out of sight; detached when dropped.
    pub struct Mount {
        pub point: PathBuf,
    }

    impl Mount {
        pub fn attach(dmg: &Path, under: &Path) -> Result<Self, String> {
            std::fs::create_dir_all(under).map_err(|e| format!("cannot create {}: {e}", under.display()))?;
            let ran = run(
                HDIUTIL,
                &[
                    os("attach"),
                    os("-nobrowse"),
                    os("-readonly"),
                    os("-noautoopen"),
                    os("-mountrandom"),
                    under.as_os_str(),
                    os("-plist"),
                    dmg.as_os_str(),
                ],
            )?;
            let point = mount_point(&ran.stdout);
            match (ran.ok, point) {
                (true, Some(point)) => Ok(Self { point }),
                (ok, _) => {
                    // Whatever got mounted under `under` goes again.
                    for entry in std::fs::read_dir(under).into_iter().flatten().flatten() {
                        drop(Self { point: entry.path() });
                    }
                    Err(if ok {
                        "the disk image did not mount where expected".into()
                    } else {
                        format!("the disk image does not open ({})", ran.stderr)
                    })
                }
            }
        }
    }

    impl Drop for Mount {
        fn drop(&mut self) {
            let detached = run(HDIUTIL, &[os("detach"), os("-quiet"), self.point.as_os_str()]).is_ok_and(|r| r.ok);
            if !detached {
                let forced = run(HDIUTIL, &[os("detach"), os("-force"), os("-quiet"), self.point.as_os_str()]);
                if !forced.is_ok_and(|r| r.ok) {
                    tracing::warn!("update: cannot detach the disk image at {}", self.point.display());
                }
            }
        }
    }

    /// The one app at the top of a mounted disk image.
    pub fn single_app(dir: &Path) -> Result<PathBuf, String> {
        let apps: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|e| format!("cannot read the disk image: {e}"))?
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "app"))
            .map(|e| e.path())
            .collect();
        match apps.as_slice() {
            [app] if app.symlink_metadata().is_ok_and(|m| m.is_dir()) => Ok(app.clone()),
            [_] => Err("the app in the disk image is not a folder; nothing was installed".into()),
            [] => Err("the disk image holds no app; nothing was installed".into()),
            _ => Err("the disk image holds more than one app; nothing was installed".into()),
        }
    }

    /// Check the app in `dmg` and put it in place of `app`. On any error `app` is
    /// unchanged, nothing is left beside it, and the image is detached.
    pub fn install(app: &Path, dmg: &Path, work: &Path, version: &str, progress: &Progress) -> Result<(), String> {
        let (running_id, _) = info_of(app)?;
        let team = team_of(app).ok_or("this copy of walle is not signed with a Developer ID, so it does not replace itself")?;
        let mount = Mount::attach(dmg, &work.join("mount"))?;
        let new = single_app(&mount.point)?;
        check_info(&running_id, &info_of(&new)?, version)?;
        check_signature(&new, &team)?;
        progress.check_cancel()?;

        let (staged, backup) = side_paths(app);
        let gone = |p: &Path| {
            let _ = std::fs::remove_dir_all(p);
        };
        gone(&staged);
        gone(&backup);
        let copied = run(DITTO, &[new.as_os_str(), staged.as_os_str()]);
        drop(mount);
        let checked = match copied {
            Ok(r) if r.ok => {
                let r = run(CODESIGN, &[os("--verify"), os("--deep"), os("--strict"), staged.as_os_str()]);
                match r {
                    Ok(r) if r.ok => Ok(()),
                    Ok(r) => Err(format!("the copy of the new app is damaged ({})", r.stderr)),
                    Err(e) => Err(e),
                }
            }
            Ok(r) => Err(format!("cannot copy the new app beside this one ({})", r.stderr)),
            Err(e) => Err(e),
        };
        if let Err(e) = checked.and_then(|()| progress.check_cancel()) {
            gone(&staged);
            return Err(e);
        }
        replace_bundle(app, &staged, &backup, true)
    }

    /// Start a helper that waits until this process has exited, then opens `app`
    /// as a new instance with `env` set. Detached: no terminal, no output, its own
    /// process group, so it outlives walle.
    pub fn open_after_exit(app: &Path, env: &[(&str, String)]) -> Result<(), String> {
        use std::os::unix::process::CommandExt;
        const SCRIPT: &str =
            r#"pid=$1; shift; n=0; while /bin/kill -0 "$pid" 2>/dev/null; do [ $n -ge 3000 ] && exit 0; n=$((n+1)); /bin/sleep 0.2; done; exec /usr/bin/open -n "$@""#;
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg(SCRIPT).arg("sh").arg(std::process::id().to_string());
        for (key, value) in env {
            cmd.arg("--env").arg(format!("{key}={value}"));
        }
        cmd.arg(app).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0);
        // Not waited for: it finishes after walle.
        cmd.spawn().map(drop).map_err(|e| format!("cannot restart walle: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requirement_pins_developer_id_and_team() {
        assert_eq!(
            requirement("N6FA79GV97"),
            "anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate \
             leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = \"N6FA79GV97\""
        );
    }

    #[test]
    fn bundles_and_places() {
        let exe = Path::new("/Applications/walle.app/Contents/MacOS/walle");
        assert_eq!(bundle_of(exe), Some(PathBuf::from("/Applications/walle.app")));
        assert_eq!(bundle_of(Path::new("/Users/ada/bin/walle")), None);
        assert_eq!(bundle_of(Path::new("/opt/walle/Contents/MacOS/walle")), None);
        assert_eq!(bundle_of(Path::new("walle")), None);
        assert!(in_caskroom(Path::new("/opt/homebrew/Caskroom/walle/0.7.1/walle.app")));
        assert!(in_caskroom(Path::new("/usr/local/Caskroom/walle/0.7.1/walle.app")));
        assert!(!in_caskroom(Path::new("/Applications/walle.app")));
        assert!(translocated(Path::new("/private/var/folders/x/y/T/AppTranslocation/ABC-123/d/walle.app")));
        assert!(!translocated(Path::new("/Users/ada/Downloads/walle.app")));
    }

    #[test]
    fn teams_from_codesign() {
        let signed = "Executable=/Applications/walle.app/Contents/MacOS/walle\nIdentifier=io.github.ulaas.walle\n\
                      Authority=Developer ID Application: Aydemir Sahin (N6FA79GV97)\nTeamIdentifier=N6FA79GV97\n";
        assert_eq!(parse_team(signed).as_deref(), Some("N6FA79GV97"));
        assert_eq!(parse_team("Signature=adhoc\nTeamIdentifier=not set\n"), None);
        assert_eq!(parse_team("code object is not signed at all"), None);
        // Nothing that could change the requirement it goes into.
        assert_eq!(parse_team("TeamIdentifier=AB\" or true"), None);
    }

    #[test]
    fn the_new_app_must_match() {
        let json = r#"{"CFBundleIdentifier":"io.github.ulaas.walle","CFBundleShortVersionString":"0.8.0","CFBundleVersion":"0.8.0"}"#;
        let info = parse_info(json).unwrap();
        assert_eq!(info, ("io.github.ulaas.walle".to_string(), "0.8.0".to_string()));
        assert!(parse_info("{}").is_err());
        assert!(parse_info("not json").is_err());
        assert_eq!(check_info("io.github.ulaas.walle", &info, "0.8.0"), Ok(()));
        assert!(check_info("io.github.ulaas.walle", &info, "0.9.0").unwrap_err().contains("version 0.8.0"));
        assert!(check_info("com.example.other", &info, "0.8.0").unwrap_err().contains("not com.example.other"));
    }

    #[test]
    fn mount_point_from_hdiutil() {
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>system-entities</key><array>
<dict><key>content-hint</key><string>GUID_partition_scheme</string><key>dev-entry</key><string>/dev/disk9</string></dict>
<dict><key>content-hint</key><string>Apple_HFS</string><key>mount-point</key>
	<string>/tmp/work/mount/dmg.a1B2c3 &amp; more</string><key>volume-kind</key><string>hfs</string></dict>
</array></dict></plist>"#;
        assert_eq!(mount_point(plist), Some(PathBuf::from("/tmp/work/mount/dmg.a1B2c3 & more")));
        assert_eq!(mount_point("<plist><dict></dict></plist>"), None);
    }

    #[test]
    fn leftovers_beside_the_app() {
        let (staged, backup) = side_paths(Path::new("/Applications/walle.app"));
        let pid = std::process::id();
        assert_eq!(staged, PathBuf::from(format!("/Applications/.walle.app.update-{pid}")));
        assert_eq!(backup, PathBuf::from(format!("/Applications/.walle.app.old-{pid}")));
        assert!(is_leftover("walle.app", &format!(".walle.app.update-{pid}")));
        assert!(is_leftover("walle.app", ".walle.app.old-12"));
        assert!(!is_leftover("walle.app", "walle.app"));
        assert!(!is_leftover("walle.app", ".walle.app.update-"));
        assert!(!is_leftover("walle.app", ".walle.app.update-12x"));
        assert!(!is_leftover("walle.app", ".other.app.old-12"));
    }

    fn fake_app(at: &Path, version: &str) {
        std::fs::create_dir_all(at.join("Contents/MacOS")).unwrap();
        std::fs::write(at.join("Contents/MacOS/walle"), version).unwrap();
    }

    fn version_at(app: &Path) -> String {
        std::fs::read_to_string(app.join("Contents/MacOS/walle")).unwrap()
    }

    #[test]
    fn swap_and_rollback_of_a_bundle() {
        for atomic in [true, false] {
            let dir = std::env::temp_dir().join(format!("walle-bundle-{}-{atomic}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let app = dir.join("walle.app");
            let (staged, backup) = side_paths(&app);
            let listing = || {
                let mut names: Vec<String> =
                    std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into()).collect();
                names.sort();
                names
            };

            // The swap: the new bundle in place, nothing else left.
            fake_app(&app, "old");
            fake_app(&staged, "new");
            replace_bundle(&app, &staged, &backup, atomic).unwrap();
            assert_eq!(version_at(&app), "new");
            assert_eq!(listing(), ["walle.app"]);

            // Nothing staged: the installed bundle stays as it is.
            let err = replace_bundle(&app, &staged, &backup, atomic).unwrap_err();
            assert!(err.contains("previous one is unchanged"), "{err}");
            assert_eq!(version_at(&app), "new");
            assert_eq!(listing(), ["walle.app"]);

            // Nothing installed any more: the staged copy goes, nothing appears.
            std::fs::remove_dir_all(&app).unwrap();
            fake_app(&staged, "newer");
            assert!(replace_bundle(&app, &staged, &backup, atomic).unwrap_err().contains("not there"));
            assert!(listing().is_empty());

            // What the next start clears away.
            fake_app(&app, "new");
            fake_app(&staged, "half");
            fake_app(&backup, "old");
            fake_app(&dir.join(".walle.app.old-1"), "older");
            clean_up(&app);
            assert_eq!(listing(), ["walle.app"]);
            std::fs::remove_dir_all(dir).ok();
        }
    }

    /// The real tools on a small disk image with an ad hoc signed app in it:
    /// mounted out of sight, read, refused for its signature, and detached.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_disk_image_is_mounted_checked_and_detached() {
        use std::process::Command;
        let dir = std::env::temp_dir().join(format!("walle-dmg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = std::fs::canonicalize(dir).unwrap();
        let app = dir.join("src/walle.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::copy("/usr/bin/true", app.join("Contents/MacOS/walle")).unwrap();
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>walle</string>
<key>CFBundleIdentifier</key><string>io.github.ulaas.walle.test</string>
<key>CFBundleShortVersionString</key><string>9.9.9</string>
<key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>"#;
        std::fs::write(app.join("Contents/Info.plist"), plist).unwrap();
        let ok = |cmd: &mut Command| assert!(cmd.status().unwrap().success(), "{cmd:?}");
        ok(Command::new("/usr/bin/codesign").args(["--force", "--sign", "-"]).arg(&app));
        let dmg = dir.join("test.dmg");
        ok(Command::new("/usr/bin/hdiutil")
            .args(["create", "-quiet", "-fs", "HFS+", "-format", "UDZO", "-srcfolder"])
            .arg(dir.join("src"))
            .arg(&dmg));

        let mount = Mount::attach(&dmg, &dir.join("work/mount")).unwrap();
        let point = mount.point.clone();
        assert!(point.starts_with(dir.join("work/mount")), "{}", point.display());
        let new = single_app(&point).unwrap();
        assert_eq!(new.file_name().unwrap(), "walle.app");
        let info = info_of(&new).unwrap();
        assert_eq!(info, ("io.github.ulaas.walle.test".to_string(), "9.9.9".to_string()));
        assert_eq!(team_of(&new), None, "ad hoc");
        let err = check_signature(&new, "N6FA79GV97").unwrap_err();
        assert!(err.contains("not signed by the same developer"), "{err}");
        drop(mount);
        let info = Command::new("/usr/bin/hdiutil").arg("info").output().unwrap();
        assert!(!String::from_utf8_lossy(&info.stdout).contains(&*point.to_string_lossy()), "still mounted");

        // An ad hoc signed walle does not replace itself, and leaves nothing mounted or beside it.
        let err = install(&app, &dmg, &dir.join("work"), "9.9.9", &Default::default()).unwrap_err();
        assert!(err.contains("not signed with a Developer ID"), "{err}");
        let info = Command::new("/usr/bin/hdiutil").arg("info").output().unwrap();
        assert!(!String::from_utf8_lossy(&info.stdout).contains(&*dir.to_string_lossy()), "still mounted");
        let mut names: Vec<_> = std::fs::read_dir(dir.join("src")).unwrap().map(|e| e.unwrap().file_name()).collect();
        names.sort();
        assert_eq!(names, ["walle.app"]);
        std::fs::remove_dir_all(dir).ok();
    }
}
