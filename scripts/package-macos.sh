#!/usr/bin/env bash
# Builds walle for Apple Silicon and packs it into a disk image.
#
#   scripts/package-macos.sh [dist folder]
#
#   walle-macos-arm64.dmg   walle.app and a link to Applications, to drag it across
#
# Needs Xcode's command line tools (codesign, hdiutil, iconutil, sips), Rust with the
# aarch64-apple-darwin target, and rsvg-convert for the icon (brew install librsvg).
#
# Signing: ad hoc unless MACOS_SIGN_IDENTITY names a Developer ID Application identity in
# the keychain. With one, set MACOS_NOTARY_PROFILE as well (a profile stored with
# `xcrun notarytool store-credentials`) to notarize the disk image and staple the ticket.
#
# WALLE_SKIP_BUILD=1 packs the release build already in target/ instead of building.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
dist=${1:-$root/dist}
target=${CARGO_TARGET_DIR:-$root/target}
triple=aarch64-apple-darwin
name=walle-macos-arm64
identity=${MACOS_SIGN_IDENTITY:--}
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -n 1)
# Bundle versions are numbers only: 0.7.1-rc.1 is 0.7.1.
bundle_version=${version%%-*}

for tool in codesign hdiutil iconutil sips plutil lipo rsvg-convert; do
  if ! command -v "$tool" > /dev/null; then
    echo "error: $tool is not installed${tool/rsvg-convert/ (brew install librsvg)}" >&2
    exit 1
  fi
done

if [ "${WALLE_SKIP_BUILD:-0}" != 1 ]; then
  rustup target add "$triple" > /dev/null 2>&1 || true
  # The oldest macOS the build runs on; the same as LSMinimumSystemVersion in Info.plist.
  MACOSX_DEPLOYMENT_TARGET=11.0 cargo build --release --locked --target "$triple" --manifest-path "$root/Cargo.toml"
fi
bin="$target/$triple/release/walle"
[ -x "$bin" ] || { echo "error: no release build at $bin" >&2; exit 1; }
if [ "$(lipo -archs "$bin")" != arm64 ]; then
  echo "error: $bin is not arm64 only ($(lipo -archs "$bin"))" >&2
  exit 1
fi

mkdir -p "$dist"
dist=$(cd "$dist" && pwd)
work=$(mktemp -d "$dist/.package-XXXXXX")
mounted=""
cleanup() {
  [ -n "$mounted" ] && hdiutil detach -quiet "$mounted" || true
  rm -rf -- "${work:?}"
}
trap cleanup EXIT

# --- The icon: every size from 16 to 1024 points, from the logo.
iconset="$work/walle.iconset"
mkdir -p "$iconset"
rsvg-convert -w 1024 -h 1024 "$root/assets/logo.svg" -o "$work/logo-1024.png"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$work/logo-1024.png" --out "$iconset/icon_${size}x${size}.png" > /dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$work/logo-1024.png" --out "$iconset/icon_${size}x${size}@2x.png" > /dev/null
done
iconutil -c icns "$iconset" -o "$work/walle.icns"

# --- The app.
app="$work/dmg/walle.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/licenses"
install -m755 "$bin" "$app/Contents/MacOS/walle"
cp "$work/walle.icns" "$app/Contents/Resources/walle.icns"
sed "s/@VERSION@/$bundle_version/g" "$root/packaging/macos/Info.plist" > "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"
cp "$root/LICENSE" "$app/Contents/Resources/licenses/LICENSE"
cp "$root/assets/fonts/OFL.txt" "$app/Contents/Resources/licenses/Inter-OFL.txt"
cp "$root/assets/fonts/Phosphor-LICENSE.txt" "$app/Contents/Resources/licenses/Phosphor-LICENSE.txt"
plutil -lint "$app/Contents/Info.plist" > /dev/null

# Nothing outside the bundle but the system's own libraries.
if otool -L "$bin" | tail -n +2 | awk '{print $1}' | grep -vE '^(/usr/lib/|/System/Library/)'; then
  echo "error: walle links against libraries the system does not have (above)" >&2
  exit 1
fi

# --- Signing. Nested code first; there is none today, but a framework added later is covered.
sign() {
  local flags=(--force --options runtime --sign "$identity")
  if [ "$identity" = - ]; then flags+=(--timestamp=none); else flags+=(--timestamp); fi
  codesign "${flags[@]}" "$@"
}
find "$app/Contents" \( -name '*.dylib' -o -name '*.framework' \) -prune -print0 | while IFS= read -r -d '' nested; do
  sign "$nested"
done
sign --deep "$app"
codesign --verify --deep --strict "$app"

# --- The disk image: the app beside a link to Applications.
ln -s /Applications "$work/dmg/Applications"
cp "$work/walle.icns" "$work/dmg/.VolumeIcon.icns"
SetFile -c icnC "$work/dmg/.VolumeIcon.icns" 2> /dev/null || true
rw="$work/$name-rw.dmg"
hdiutil create -quiet -volname "walle $version" -srcfolder "$work/dmg" -fs HFS+ -format UDRW -ov "$rw"
# The volume shows its icon only with the custom-icon flag on its root.
mounted=$(hdiutil attach -nobrowse -noautoopen -readwrite "$rw" | awk -F'\t' '/\/Volumes\// {print $NF; exit}')
SetFile -a C "$mounted" 2> /dev/null || true
hdiutil detach -quiet "$mounted"
mounted=""
rm -f "$dist/$name.dmg"
hdiutil convert -quiet "$rw" -format ULFO -o "$dist/$name.dmg"
if [ "$identity" != - ]; then
  sign "$dist/$name.dmg"
  if [ -n "${MACOS_NOTARY_PROFILE:-}" ]; then
    xcrun notarytool submit "$dist/$name.dmg" --keychain-profile "$MACOS_NOTARY_PROFILE" --wait
    xcrun stapler staple "$dist/$name.dmg"
  fi
fi

ls -l "$dist/$name.dmg"
