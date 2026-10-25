#!/usr/bin/env bash
# Install Spacebox for the current user (no root required).
#
#   ./packaging/install.sh              # build release + install to ~/.local
#   ./packaging/install.sh --no-build   # install an already-built binary
#   PREFIX=/usr/local ./packaging/install.sh   # system-wide (needs sudo for the dirs)
#
# What it does, and why each step matters for launcher discovery:
#   1. binary     -> $PREFIX/bin/spacebox        (so `spacebox` resolves on PATH)
#   2. .desktop   -> $XDG_DATA_HOME/applications (so rofi/wofi/KDE/GNOME list it)
#   3. icons      -> $XDG_DATA_HOME/icons/hicolor/<size>/apps (so it is not a blank tile)
#   4. caches     -> update-desktop-database + gtk-update-icon-cache
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname -- "$SCRIPT_DIR")"

PREFIX="${PREFIX:-$HOME/.local}"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
BIN_DIR="$PREFIX/bin"
APP_DIR="$DATA_HOME/applications"
ICON_ROOT="$DATA_HOME/icons/hicolor"
DESKTOP_ID="io.github.spacebox"

DO_BUILD=1
for arg in "$@"; do
  case "$arg" in
    --no-build) DO_BUILD=0 ;;
    -h|--help)  sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

say() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }

if [[ "$DO_BUILD" == 1 ]]; then
  say "Building release binary"
  ( cd "$PROJECT_DIR" && cargo build --release --locked 2>/dev/null || cargo build --release )
fi

BINARY="$PROJECT_DIR/target/release/spacebox"
if [[ ! -x "$BINARY" ]]; then
  echo "error: $BINARY not found. Run without --no-build." >&2
  exit 1
fi

say "Installing binary to $BIN_DIR"
mkdir -p "$BIN_DIR"
install -m 755 "$BINARY" "$BIN_DIR/spacebox"

say "Installing desktop entry to $APP_DIR"
mkdir -p "$APP_DIR"
# Resolve Exec= to the absolute binary path so entries work when ~/.local/bin
# is missing from the session PATH (a very common reason launchers fail).
sed "s|^Exec=spacebox|Exec=$BIN_DIR/spacebox|" \
  "$SCRIPT_DIR/$DESKTOP_ID.desktop" > "$APP_DIR/$DESKTOP_ID.desktop"
chmod 644 "$APP_DIR/$DESKTOP_ID.desktop"

say "Installing icons"
for size in 16 24 32 48 64 128 256; do
  dir="$ICON_ROOT/${size}x${size}/apps"
  mkdir -p "$dir"
  if command -v rsvg-convert >/dev/null 2>&1; then
    rsvg-convert -w "$size" -h "$size" "$PROJECT_DIR/assets/spacebox.svg" -o "$dir/spacebox.png"
  elif command -v magick >/dev/null 2>&1; then
    magick "$PROJECT_DIR/assets/spacebox.png" -resize "${size}x${size}" "$dir/spacebox.png"
  elif command -v convert >/dev/null 2>&1; then
    convert "$PROJECT_DIR/assets/spacebox.png" -resize "${size}x${size}" "$dir/spacebox.png"
  else
    warn "no image converter found; falling back to the single 256px source"
    mkdir -p "$ICON_ROOT/256x256/apps"
    cp "$PROJECT_DIR/assets/spacebox.png" "$ICON_ROOT/256x256/apps/spacebox.png"
    break
  fi
done
# Scalable original for HiDPI desktops.
mkdir -p "$ICON_ROOT/scalable/apps"
install -m 644 "$PROJECT_DIR/assets/spacebox.svg" "$ICON_ROOT/scalable/apps/spacebox.svg"

say "Refreshing desktop + icon caches"
command -v update-desktop-database >/dev/null 2>&1 \
  && update-desktop-database "$APP_DIR" 2>/dev/null || warn "update-desktop-database not found (optional)"
command -v gtk-update-icon-cache >/dev/null 2>&1 \
  && gtk-update-icon-cache -f -t "$ICON_ROOT" 2>/dev/null || true

# Validate the entry if the helper is available.
if command -v desktop-file-validate >/dev/null 2>&1; then
  desktop-file-validate "$APP_DIR/$DESKTOP_ID.desktop" \
    && say "Desktop entry validated" \
    || warn "desktop-file-validate reported issues above"
fi

echo
say "Done."
case ":$PATH:" in
  *":$BIN_DIR:"*) echo "  Run: spacebox   (or launch 'Spacebox' from your app launcher)" ;;
  *) warn "$BIN_DIR is not on your PATH. Add this to ~/.bashrc or ~/.zshrc:"
     echo "      export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac
