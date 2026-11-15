//! Records which commit is being built, for the About box. Builds outside a git checkout
//! (a source archive, say) still work and report the commit as unknown. On Windows it also
//! embeds the icon, which is what Explorer and shortcuts show for the exe.
use std::process::Command;

#[path = "src/icon_art.rs"]
mod icon_art;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// An .ico holding 32-bit bitmaps, largest needed by Explorer's "extra large" view.
fn ico(sizes: &[u32]) -> Vec<u8> {
    let mut dir = vec![0, 0, 1, 0];
    dir.extend((sizes.len() as u16).to_le_bytes());
    let mut images = Vec::new();
    let mut offset = 6 + 16 * sizes.len() as u32;
    for &size in sizes {
        let mut image = Vec::new();
        image.extend(40u32.to_le_bytes());
        image.extend(size.to_le_bytes());
        // Twice the height: the colours and the AND mask count as one bitmap.
        image.extend((size * 2).to_le_bytes());
        image.extend(1u16.to_le_bytes());
        image.extend(32u16.to_le_bytes());
        image.extend([0; 24]);
        let rgba = icon_art::rgba(size, icon_art::DEFAULT_ACCENT);
        // Bitmaps run bottom to top, in BGRA.
        for row in rgba.chunks_exact(size as usize * 4).rev() {
            for px in row.chunks_exact(4) {
                image.extend([px[2], px[1], px[0], px[3]]);
            }
        }
        // Alpha decides transparency, but the mask must exist: 1 bit a pixel, in 4-byte rows.
        let mask_row = (size as usize).div_ceil(32) * 4;
        image.resize(image.len() + mask_row * size as usize, 0);

        // In the directory, 256 is written as 0.
        dir.extend([size as u8, size as u8, 0, 0]);
        dir.extend(1u16.to_le_bytes());
        dir.extend(32u16.to_le_bytes());
        dir.extend((image.len() as u32).to_le_bytes());
        dir.extend(offset.to_le_bytes());
        offset += image.len() as u32;
        images.extend(image);
    }
    dir.extend(images);
    dir
}

fn embed_windows_icon() {
    let path = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("walle.ico");
    std::fs::write(&path, ico(&[16, 20, 24, 32, 40, 48, 64, 256])).unwrap();
    let mut res = winresource::WindowsResource::new();
    res.set_icon(path.to_str().unwrap());
    // Task Manager shows this as the app's name, so not the long Cargo description.
    res.set("FileDescription", "walle");
    if let Err(e) = res.compile() {
        println!("cargo:warning=the exe will have no icon: {e}");
    }
}

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_windows_icon();
    }

    let hash = git(&["rev-parse", "HEAD"]).unwrap_or_default();
    let short = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let date = git(&["log", "-1", "--format=%cs"]).unwrap_or_default();
    // --no-optional-locks: don't rewrite .git/index, which would trigger the next rebuild.
    let dirty = git(&["--no-optional-locks", "status", "--porcelain"]).is_some_and(|s| !s.is_empty());
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let compiler = Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    println!("cargo:rustc-env=WALLE_GIT_HASH={hash}");
    println!("cargo:rustc-env=WALLE_GIT_SHORT={short}");
    println!("cargo:rustc-env=WALLE_GIT_DATE={date}");
    println!("cargo:rustc-env=WALLE_GIT_DIRTY={dirty}");
    println!("cargo:rustc-env=WALLE_COMPILER={compiler}");
    println!("cargo:rustc-env=WALLE_PROFILE={}", std::env::var("PROFILE").unwrap_or_default());
    println!("cargo:rustc-env=WALLE_TARGET={}", std::env::var("TARGET").unwrap_or_default());

    // Rebuild when the commit moves or the working tree changes.
    println!("cargo:rerun-if-changed=build.rs");
    for path in ["src", "assets", "Cargo.toml", "Cargo.lock"] {
        println!("cargo:rerun-if-changed={path}");
    }
    for name in ["HEAD", "index", "packed-refs"] {
        if let Some(p) = git(&["rev-parse", "--git-path", name]) {
            println!("cargo:rerun-if-changed={p}");
        }
    }
    if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"])
        && let Some(p) = git(&["rev-parse", "--git-path", &branch])
    {
        println!("cargo:rerun-if-changed={p}");
    }
}
