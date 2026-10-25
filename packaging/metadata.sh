#!/usr/bin/env bash
# Shared metadata for every packager. Sourced, never executed.
#
# Override the maintainer fields before publishing:
#   SPACEBOX_MAINTAINER="Your Name <you@example.com>" ./packaging/build-packages.sh
set -euo pipefail

PKG_NAME="spacebox"
PKG_VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$(dirname "${BASH_SOURCE[0]}")/../Cargo.toml" | head -1)"
PKG_DESC="Fast disk usage treemap viewer"
PKG_SUMMARY="A fast treemap disk usage viewer (du/Baobab alternative)"
PKG_URL="https://github.com/shian07/spacebox"
PKG_LICENSE="MIT"
PKG_MAINTAINER="${SPACEBOX_MAINTAINER:-Spacebox contributors <maintainers@example.com>}"

# Human-facing app name and desktop id, matching packaging/*.desktop.
APP_NAME="Spacebox"
DESKTOP_ID="io.github.spacebox"

PROJECT_DIR="$(cd -- "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BINARY="${SPACEBOX_BINARY:-$PROJECT_DIR/target/release/spacebox}"
DIST_DIR="${DIST_DIR:-$PROJECT_DIR/dist}"
