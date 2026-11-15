#!/usr/bin/env bash
# Packs a release build of walle into what a release offers.
#
#   scripts/package.sh <version> <windows-x64|linux-x64> [dist folder]
#
#   windows  walle-<v>-windows-x64.zip   walle.exe, README, licences
#            walle-<v>-windows-x64.msi   installer (WiX Toolset v5: `wix` on PATH with WixToolset.UI.wixext)
#   linux    walle-<v>-linux-x64.tar.gz  walle, launcher entry, icon, README, licences
#            walle-<v>-linux-x64.deb     package for Debian and Ubuntu (dpkg-deb)
#            walle-<v>-linux-x64.rpm     package for Fedora and openSUSE (rpmbuild)
#
# A package is skipped, with a note, when its tool is not installed; set
# WALLE_PACKAGE_STRICT=1 (as the release workflow does) to fail instead.
set -euo pipefail

version=${1:?version, like 0.6.1}
platform=${2:?windows-x64 or linux-x64}
root=$(cd "$(dirname "$0")/.." && pwd)
dist=${3:-$root/dist}
target=${CARGO_TARGET_DIR:-$root/target}
name="walle-$version-$platform"

need() {
  command -v "$1" > /dev/null && return 0
  if [ "${WALLE_PACKAGE_STRICT:-0}" = 1 ]; then
    echo "error: $1 is not installed" >&2
    exit 1
  fi
  echo "note: $1 is not installed, skipping the $2" >&2
  return 1
}

mkdir -p "$dist"
dist=$(cd "$dist" && pwd)
# A folder of its own for everything temporary, so cleaning up removes one known folder.
work=$(mktemp -d "$dist/.package-XXXXXX")
trap 'rm -rf -- "${work:?}"' EXIT
stage="$work/$name"

mkdir -p "$stage/licenses"
cp "$root/README.md" "$root/LICENSE" "$stage/"
cp "$root/assets/fonts/OFL.txt" "$stage/licenses/Inter-OFL.txt"
cp "$root/assets/fonts/Phosphor-LICENSE.txt" "$stage/licenses/Phosphor-LICENSE.txt"

case "$platform" in
  windows-x64)
    cp "$target/release/walle.exe" "$stage/"
    (cd "$work" && 7z a -tzip "$dist/$name.zip" "$name" > /dev/null)
    if need wix "MSI installer"; then
      # The .wixpdb lands next to the MSI, so build it in the work folder and move the MSI out.
      # build.rs writes the icon into its output folder; take the newest if there are several.
      icon=$(find "$target/release/build" -name walle.ico -printf '%T@ %p\n' | sort -rn | head -n 1 | cut -d' ' -f2-)
      if [ -z "$icon" ]; then
        echo "error: no walle.ico under $target/release/build (build walle for Windows first)" >&2
        exit 1
      fi
      (cd "$root/packaging/windows" && wix build walle.wxs -arch x64 -ext WixToolset.UI.wixext \
        -d "Version=${version%%-*}" -d "BinDir=$(cygpath -w "$stage")" -d "IconFile=$(cygpath -w "$icon")" \
        -o "$(cygpath -w "$work/$name.msi")")
      mv "$work/$name.msi" "$dist/$name.msi"
    fi
    ;;
  linux-x64)
    bin="$target/release/walle"
    install -m755 "$bin" "$stage/walle"
    cp "$root/packaging/linux/walle.desktop" "$stage/"
    cp "$root/assets/logo.svg" "$stage/walle.svg"
    tar -C "$work" -czf "$dist/$name.tar.gz" "$name"

    # One file tree for both packages.
    tree="$work/tree"
    install -Dm755 "$bin" "$tree/usr/bin/walle"
    install -Dm644 "$root/packaging/linux/walle.desktop" "$tree/usr/share/applications/walle.desktop"
    install -Dm644 "$root/assets/logo.svg" "$tree/usr/share/icons/hicolor/scalable/apps/walle.svg"
    install -Dm644 "$root/LICENSE" "$tree/usr/share/licenses/walle/LICENSE"
    install -Dm644 "$root/assets/fonts/OFL.txt" "$tree/usr/share/licenses/walle/Inter-OFL.txt"
    install -Dm644 "$root/assets/fonts/Phosphor-LICENSE.txt" "$tree/usr/share/licenses/walle/Phosphor-LICENSE.txt"
    echo "Shared libraries walle links against:"
    readelf -d "$bin" | grep NEEDED || true
    # Debian and rpm versions sort a pre-release before the release with "~".
    pkgversion=${version/-/\~}

    if need dpkg-deb ".deb package"; then
      deb="$work/deb"
      cp -a "$tree" "$deb"
      mkdir -p "$deb/DEBIAN" "$deb/usr/share/doc/walle"
      cp "$root/LICENSE" "$deb/usr/share/doc/walle/copyright"
      size=$(du -sk "$deb/usr" | cut -f1)
      sed -e "s/@VERSION@/$pkgversion/" -e "s/@SIZE@/$size/" "$root/packaging/linux/deb-control" > "$deb/DEBIAN/control"
      dpkg-deb --root-owner-group --build "$deb" "$dist/$name.deb"
    fi

    if need rpmbuild ".rpm package"; then
      rpmtop="$work/rpm"
      mkdir -p "$rpmtop"
      rpmbuild -bb --quiet --define "_topdir $rpmtop" --define "pkgver $pkgversion" --define "stage $tree" \
        "$root/packaging/linux/walle.spec"
      mv "$rpmtop"/RPMS/x86_64/walle-*.rpm "$dist/$name.rpm"
    fi
    ;;
  *)
    echo "error: unknown platform $platform" >&2
    exit 1
    ;;
esac

ls -l "$dist"
