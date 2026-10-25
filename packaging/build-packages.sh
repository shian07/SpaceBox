#!/usr/bin/env bash
# Build installable packages for Linux distributions plus a Windows .exe zip.
#
#   ./packaging/build-packages.sh              # everything the host can build
#   ./packaging/build-packages.sh deb rpm      # only the named formats
#
# Formats:
#   deb   Debian/Ubuntu/Mint/Pop!_OS     (needs dpkg-deb)
#   rpm   Fedora/RHEL/openSUSE           (needs rpmbuild)
#   arch  Arch/Manjaro/CachyOS/Endeavour (needs makepkg + bsdtar)
#   win   Windows x86_64                 (needs a mingw-w64 toolchain; see below)
#
# Everything lands in ./dist.
set -euo pipefail

HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/metadata.sh"

BUILD_ALL=0
REQUESTED=()
for arg in "$@"; do
  case "$arg" in
    -h|--help) sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    deb|rpm|arch|win) REQUESTED+=("$arg") ;;
    *) echo "unknown format: $arg" >&2; exit 2 ;;
  esac
done
[ ${#REQUESTED[@]} -eq 0 ] && BUILD_ALL=1

say()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarn:\033[0m %s\n' "$*" >&2; }
skip() { printf '\033[1;33mskip:\033[0m %s\n' "$*" >&2; }

wanted() {
  [ "$BUILD_ALL" = 1 ] && return 0
  local f
  for f in "${REQUESTED[@]}"; do [ "$f" = "$1" ] && return 0; done
  return 1
}

mkdir -p "$DIST_DIR"
# Repository root as a staging root for the Arch package.
STAGE="$PROJECT_DIR/target/pkg-stage"

# Render the icon set into a tree so each packager can install it.
stage_icons() {
  local root="$1"
  local size
  for size in 16 24 32 48 64 128 256; do
    mkdir -p "$root/usr/share/icons/hicolor/${size}x${size}/apps"
    if command -v rsvg-convert >/dev/null 2>&1; then
      rsvg-convert -w "$size" -h "$size" "$PROJECT_DIR/assets/spacebox.svg" \
        -o "$root/usr/share/icons/hicolor/${size}x${size}/apps/${PKG_NAME}.png"
    else
      cp "$PROJECT_DIR/assets/spacebox.png" \
        "$root/usr/share/icons/hicolor/${size}x${size}/apps/${PKG_NAME}.png"
    fi
  done
  mkdir -p "$root/usr/share/icons/hicolor/scalable/apps"
  install -m 644 "$PROJECT_DIR/assets/spacebox.svg" \
    "$root/usr/share/icons/hicolor/scalable/apps/${PKG_NAME}.svg"
}

# Desktop file with an absolute Exec so it works even if /usr/bin is absent
# from a session's PATH.
stage_desktop() {
  local dest="$1"
  mkdir -p "$(dirname "$dest")"
  sed "s|^Exec=$PKG_NAME|Exec=/usr/bin/$PKG_NAME|" \
    "$HERE/$DESKTOP_ID.desktop" > "$dest"
  chmod 644 "$dest"
}

stage_doc() {
  local root="$1"
  mkdir -p "$root/usr/share/doc/$PKG_NAME"
  install -m 644 "$PROJECT_DIR/README.md" "$PROJECT_DIR/LICENSE" \
    "$root/usr/share/doc/$PKG_NAME/"
}

# ---------------------------------------------------------------- build bin --
if [ ! -x "$BINARY" ]; then
  say "Building release binary"
  ( cd "$PROJECT_DIR" && cargo build --release )
fi
[ -x "$BINARY" ] || { echo "error: $BINARY missing" >&2; exit 1; }

# ------------------------------------------------------------------ deb ------
build_deb() {
  local dpkg_deb
  dpkg_deb="$(command -v dpkg-deb || true)"
  [ -z "$dpkg_deb" ] && { skip "deb: dpkg-deb not found (install dpkg-dev)"; return 0; }

  local root="$STAGE/deb"
  rm -rf "$root"
  mkdir -p "$root/DEBIAN" "$root/usr/bin"
  install -m 755 "$BINARY" "$root/usr/bin/$PKG_NAME"
  stage_desktop "$root/usr/share/applications/$DESKTOP_ID.desktop"
  stage_icons "$root"
  stage_doc "$root"

  # Installed size in KiB, as dpkg expects.
  local size_kb
  size_kb=$(du -sk "$root" --exclude=DEBIAN | cut -f1)

  cat > "$root/DEBIAN/control" <<EOF
Package: $PKG_NAME
Version: $PKG_VERSION-1
Section: utils
Priority: optional
Architecture: amd64
Depends: libc6 (>= 2.34), libgcc-s1, libstdc++6, libx11-6, libxft2, libxinerama1, libxcursor1, libxrender1, libxfixes3, libfontconfig1, libfreetype6, libpng16-16, libexpat1, libbz2-1.0, libbrotli1, zlib1g
Recommends: xdg-utils
Installed-Size: $size_kb
Maintainer: $PKG_MAINTAINER
Homepage: $PKG_URL
Description: $PKG_SUMMARY
 Spacebox draws a treemap of disk usage: every folder and file becomes a
 rectangle sized by the bytes it occupies, and you can drill into folders to
 find what is taking up space.
 .
 It scans in the background on a worker thread, reports allocated or logical
 size, groups small files per folder to keep memory use low, and can move
 files to the trash or delete them from the UI.
EOF

  cat > "$root/DEBIAN/postinst" <<EOF
#!/bin/sh
set -e
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database -q /usr/share/applications || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor || true
fi
exit 0
EOF
  chmod 755 "$root/DEBIAN/postinst"

  cat > "$root/DEBIAN/postrm" <<EOF
#!/bin/sh
set -e
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database -q /usr/share/applications || true
fi
exit 0
EOF
  chmod 755 "$root/DEBIAN/postrm"

  local out="$DIST_DIR/${PKG_NAME}_${PKG_VERSION}-1_amd64.deb"
  ( cd "$root" && find . -path ./DEBIAN -prune -o -type f -print0 \
      | xargs -0 md5sum > DEBIAN/md5sums )
  # Compress deterministically so rebuilds are reproducible.
  "$dpkg_deb" --root-owner-group -Zxz --build "$root" "$out" >/dev/null
  say "deb   -> $out ($(du -h "$out" | cut -f1))"
}

# ------------------------------------------------------------------ rpm ------
build_rpm() {
  local rpmbuild
  rpmbuild="$(command -v rpmbuild || true)"
  [ -z "$rpmbuild" ] && { skip "rpm: rpmbuild not found (install rpm-build)"; return 0; }

  local top="$STAGE/rpm"
  rm -rf "$top"
  mkdir -p "$top"/{BUILD,RPMS,SOURCES,SPECS,SRPMS,BUILDROOT}

  local tarball="spacebox-$PKG_VERSION.tar.gz"
  # Sources are laid out under spacebox-<version>/ inside the tarball.
  local src="$STAGE/srcroot/spacebox-$PKG_VERSION"
  rm -rf "$STAGE/srcroot"; mkdir -p "$src"
  cp "$BINARY" "$src/spacebox"
  install -m 644 "$PROJECT_DIR/README.md" "$PROJECT_DIR/LICENSE" "$src/"
  cp "$HERE/$DESKTOP_ID.desktop" "$src/"
  mkdir -p "$src/icons"
  for s in 16 24 32 48 64 128 256; do
    if command -v rsvg-convert >/dev/null 2>&1; then
      rsvg-convert -w "$s" -h "$s" "$PROJECT_DIR/assets/spacebox.svg" -o "$src/icons/$s.png"
    else
      cp "$PROJECT_DIR/assets/spacebox.png" "$src/icons/$s.png"
    fi
  done
  cp "$PROJECT_DIR/assets/spacebox.svg" "$src/icons/scalable.svg"
  ( cd "$STAGE/srcroot" && tar -czf "$top/SOURCES/$tarball" "spacebox-$PKG_VERSION" )

  cat > "$top/SPECS/$PKG_NAME.spec" <<EOF
Name:           $PKG_NAME
Version:        $PKG_VERSION
Release:        1%{?dist}
Summary:        $PKG_SUMMARY
License:        $PKG_LICENSE
URL:            $PKG_URL
Source0:        %{name}-%{version}.tar.gz
ExclusiveArch:  x86_64
Requires:       xdg-utils

%description
Spacebox draws a treemap of disk usage: every folder and file becomes a
rectangle sized by the bytes it occupies, and you can drill into folders to
find what is taking up space.

It scans in the background on a worker thread, reports allocated or logical
size, groups small files per folder to keep memory use low, and can move
files to the trash or delete them from the UI.

%prep
%setup -q

%build
# The binary is prebuilt by cargo before packaging; nothing to compile here.

%install
mkdir -p %{buildroot}%{_bindir}
install -m 755 spacebox %{buildroot}%{_bindir}/spacebox

install -d %{buildroot}%{_datadir}/applications
sed 's|^Exec=spacebox|Exec=%{_bindir}/spacebox|' $DESKTOP_ID.desktop \\
  > %{buildroot}%{_datadir}/applications/$DESKTOP_ID.desktop

for s in 16 24 32 48 64 128 256; do
  install -d %{buildroot}%{_datadir}/icons/hicolor/\${s}x\${s}/apps
  install -m 644 icons/\$s.png %{buildroot}%{_datadir}/icons/hicolor/\${s}x\${s}/apps/$PKG_NAME.png
done
install -d %{buildroot}%{_datadir}/icons/hicolor/scalable/apps
install -m 644 icons/scalable.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/$PKG_NAME.svg

%files
%{_bindir}/$PKG_NAME
%{_datadir}/applications/$DESKTOP_ID.desktop
%{_datadir}/icons/hicolor/*/apps/$PKG_NAME.*
%license LICENSE
%doc README.md

%changelog
* $(LC_ALL=C date '+%a %b %d %Y') $PKG_MAINTAINER - $PKG_VERSION-1
- Initial package
EOF

  # rpmbuild needs to find its rc/macros files; on a relocated install the
  # compiled-in /usr/lib/rpm does not exist, so pass it explicitly when found.
  local rpmrc_env=()
  for candidate in /usr/lib/rpm "${RPM_CONFIGDIR:-}"; do
    if [ -n "$candidate" ] && [ -f "$candidate/rpmrc" ] && [ "$candidate" != "/usr/lib/rpm" ]; then
      rpmrc_env=(env "RPM_CONFIGDIR=$candidate")
      break
    fi
  done

  # rpm wants a writable temp dir and database; point both inside the build
  # tree so packaging works in sandboxes and unprivileged containers.
  mkdir -p "$top/tmp" "$top/rpmdb"
  local rpm_args=(
    --define "_topdir $top"
    --define "_tmppath $top/tmp"
    --define "_dbpath $top/rpmdb"
    --define "_build_id_links none"
    --define "__os_install_post %{nil}"
    --define "debug_package %{nil}"
    --define "_binary_payload w5.zstdio"
  )

  if [ ${#rpmrc_env[@]} -gt 0 ]; then
    "${rpmrc_env[@]}" "$rpmbuild" "${rpm_args[@]}" -bb "$top/SPECS/$PKG_NAME.spec" \
      >"$STAGE/rpm.log" 2>&1 || { tail -25 "$STAGE/rpm.log" >&2; return 1; }
  else
    "$rpmbuild" "${rpm_args[@]}" -bb "$top/SPECS/$PKG_NAME.spec" \
      >"$STAGE/rpm.log" 2>&1 || { tail -25 "$STAGE/rpm.log" >&2; return 1; }
  fi

  local built
  built=$(find "$top/RPMS" -name '*.rpm' | head -1)
  [ -z "$built" ] && { warn "rpm: no output produced"; return 1; }
  cp "$built" "$DIST_DIR/"
  say "rpm   -> $DIST_DIR/$(basename "$built") ($(du -h "$DIST_DIR/$(basename "$built")" | cut -f1))"
}

# ----------------------------------------------------------------- arch ------
build_arch() {
  command -v makepkg >/dev/null 2>&1 || { skip "arch: makepkg not found (not an Arch based host)"; return 0; }

  local pkgdir="$STAGE/arch"
  rm -rf "$pkgdir"; mkdir -p "$pkgdir"
  # Prebuilt payload; makepkg only assembles it.
  local payload="$pkgdir/payload"
  mkdir -p "$payload/usr/bin"
  install -m 755 "$BINARY" "$payload/usr/bin/$PKG_NAME"
  stage_desktop "$payload/usr/share/applications/$DESKTOP_ID.desktop"
  stage_icons "$payload"
  stage_doc "$payload"

  cat > "$pkgdir/PKGBUILD" <<EOF
# Maintainer: $PKG_MAINTAINER
# Binary package produced by packaging/build-packages.sh
pkgname=$PKG_NAME
pkgver=$PKG_VERSION
pkgrel=1
pkgdesc="$PKG_DESC"
arch=('x86_64')
url="$PKG_URL"
license=('$PKG_LICENSE')
depends=('glibc' 'gcc-libs' 'libx11' 'libxft' 'libxinerama' 'libxcursor'
         'libxrender' 'libxfixes' 'fontconfig' 'freetype2' 'libpng'
         'expat' 'bzip2' 'brotli' 'zlib')
optdepends=('xdg-utils: open files and folders with the default handler')
options=('!strip')

package() {
  # \$startdir is the directory holding the PKGBUILD; makepkg wipes \$srcdir.
  ( cd "\$startdir/payload" && tar -cf - . ) | ( cd "\$pkgdir" && tar -xf - --no-same-owner )
}
EOF

  ( cd "$pkgdir" \
    && PKGDEST="$DIST_DIR" SRCDEST="$pkgdir" BUILDDIR="$pkgdir" \
       makepkg --noconfirm --nodeps --nodeps --cleanbuild --skipinteg \
               --noprepare --skipchecksums >"$STAGE/arch.log" 2>&1 ) \
    || { tail -20 "$STAGE/arch.log" >&2; return 1; }

  local built
  built=$(ls -1t "$DIST_DIR"/${PKG_NAME}-${PKG_VERSION}-1-*.pkg.tar.zst 2>/dev/null | head -1)
  [ -z "$built" ] && { warn "arch: no output produced"; return 1; }
  say "arch  -> $built ($(du -h "$built" | cut -f1))"
}

# ------------------------------------------------------------------ win ------
build_win() {
  local exe="$PROJECT_DIR/target/x86_64-pc-windows-gnu/release/spacebox.exe"
  if [ ! -f "$exe" ]; then
    if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
      skip "win: no mingw-w64 toolchain and no prebuilt exe
       install: sudo apt install mingw-w64 (Debian/Ubuntu)
                sudo dnf install mingw64-gcc-c++ (Fedora)
                sudo pacman -S mingw-w64-gcc (Arch)
       then:    cargo build --release --target x86_64-pc-windows-gnu"
      return 0
    fi
    say "Cross-compiling for Windows"
    ( cd "$PROJECT_DIR" && cargo build --release --target x86_64-pc-windows-gnu )
  fi

  local stage="$STAGE/win/spacebox-$PKG_VERSION-windows-x86_64"
  rm -rf "$stage"; mkdir -p "$stage"
  cp "$exe" "$stage/spacebox.exe"
  install -m 644 "$PROJECT_DIR/README.md" "$PROJECT_DIR/LICENSE" "$stage/"
  # A short readme so the zip is self-explanatory on Windows.
  cat > "$stage/HOW-TO-RUN.txt" <<'EOF'
Spacebox for Windows
====================

Double-click spacebox.exe (or run `spacebox.exe C:\path\to\scan`).

Windows SmartScreen may warn because the binary is not code-signed. Choose
"More info" then "Run anyway".

To add it to the Start menu: right-click spacebox.exe, "Create shortcut", and
move the shortcut to:
    %APPDATA%\Microsoft\Windows\Start Menu\Programs
EOF

  local out="$DIST_DIR/spacebox-${PKG_VERSION}-windows-x86_64.zip"
  rm -f "$out"
  local entry="spacebox-$PKG_VERSION-windows-x86_64"
  if command -v zip >/dev/null 2>&1; then
    ( cd "$STAGE/win" && zip -qr "$out" "$entry" )
  else
    # No zip binary: bsdtar writes a zip from the extension and libarchive.
    ( cd "$STAGE/win" && bsdtar -a -cf "$out" "$entry" )
  fi
  say "win   -> $out ($(du -h "$out" | cut -f1))"
}

# ----------------------------------------------------------------- run ------
status=0
for fmt in deb rpm arch win; do
  wanted "$fmt" || continue
  echo
  case "$fmt" in
    deb)  build_deb  || status=1 ;;
    rpm)  build_rpm  || status=1 ;;
    arch) build_arch || status=1 ;;
    win)  build_win  || status=1 ;;
  esac
done

echo
say "Artifacts in $DIST_DIR"
ls -1sh "$DIST_DIR" 2>/dev/null | tail -n +2
exit $status
