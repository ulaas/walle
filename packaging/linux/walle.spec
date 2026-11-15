# Packs the prebuilt release binary; built by scripts/package.sh with
#   rpmbuild -bb --define "_topdir <dir>" --define "pkgver <v>" --define "stage <dir>"
%global debug_package %{nil}
%global _build_id_links none

Name:           walle
Version:        %{pkgver}
Release:        1
Summary:        Browse, shortlist, compare, keep and set wallpapers
License:        MIT AND OFL-1.1
URL:            https://ulaas.github.io/walle/
Recommends:     libxkbcommon, libxkbcommon-x11, libwayland-client, libwayland-egl, libX11, libXcursor, libXi, libXrandr, libglvnd-glx, libglvnd-egl

%description
walle browses nine wallpaper sources in one window and is built around
deciding: a shortlist, side-by-side compare where the winner stays on,
an offline library with ratings, tags and notes, rotation and effects.
Everything is kept locally; there is no account and no telemetry.

%install
mkdir -p %{buildroot}
cp -a %{stage}/usr %{buildroot}/

%files
%{_bindir}/walle
%{_datadir}/applications/walle.desktop
%{_datadir}/icons/hicolor/scalable/apps/walle.svg
%license %{_datadir}/licenses/walle/LICENSE
%license %{_datadir}/licenses/walle/Inter-OFL.txt
%license %{_datadir}/licenses/walle/Phosphor-LICENSE.txt
