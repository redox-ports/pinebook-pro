#!/usr/bin/env bash
# Build the minimal Pinebook Pro Redox image locally.
#
# Requires scripts/bootstrap-redox.sh to have been run once. The result is
# build/redox/build/aarch64/minimal/harddrive.img: a GPT image (ESP at
# sector 2048 + RedoxFS). For the first hardware tests it boots on the
# Pinebook Pro using the U-Boot already installed on the eMMC (distro boot
# scans the microSD ESP), so no repacking is needed yet.
#
# Usage:
#   scripts/build-image.sh [--repo-binary]
#
#   --repo-binary  override .config and use prebuilt aarch64 packages from
#                  the Redox server (much faster if they exist; availability
#                  unverified, see README section 7, risk 7)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REDOX_DIR="${PBP_REDOX_DIR:-$REPO_ROOT/build/redox}"
OVERLAY_SRC="$REPO_ROOT/config/aarch64/pinebook-pro/minimal.toml"
IMG="$REDOX_DIR/build/aarch64/minimal/harddrive.img"
REPO_BINARY=""

die() {
    echo "error: $*" >&2
    exit 1
}

usage() {
    echo "usage: scripts/build-image.sh [--repo-binary]"
    exit 0
}

while [ $# -gt 0 ]; do
    case "$1" in
        --repo-binary) REPO_BINARY=1 ;;
        -h|--help) usage ;;
        *) die "unknown argument: $1 (see --help)" ;;
    esac
    shift
done

[ -f "$REDOX_DIR/Makefile" ] || die "no Redox tree at $REDOX_DIR (run scripts/bootstrap-redox.sh first)"
[ -f "$OVERLAY_SRC" ] || die "overlay config missing: $OVERLAY_SRC"

# Re-inject the overlay on every run: this repository is the source of truth.
mkdir -p "$REDOX_DIR/config/aarch64/pinebook-pro"
cp -f "$OVERLAY_SRC" "$REDOX_DIR/config/aarch64/pinebook-pro/minimal.toml"

# The changes have to be injected after the cookbook has fetched a recipe's
# sources and before anything is compiled from them, so a fresh tree needs that
# fetch first. The cookbook skips sources that already exist and leaves local
# modifications alone, so this is a no-op on every later run.
for dir in "$REPO_ROOT"/patches/*/; do
    repo="$(basename "$dir")"
    [ "$repo" = debug ] && continue
    [ -d "$REDOX_DIR/recipes/core/$repo/source" ] && continue
    echo "==> fetching $repo sources"
    make -C "$REDOX_DIR" "f.$repo"
done

# Same idea for the upstream changes. Idempotent: identical files are skipped
# and an already-applied diff is reported as up to date, so this normally just
# verifies the checkout. It matters after a fresh bootstrap or whenever a source
# tree was reset.
echo "==> applying patches"
"$REPO_ROOT/scripts/apply-patches.sh"

echo "==> build settings"
grep -E '^(ARCH|CONFIG_NAME|FILESYSTEM_CONFIG|PODMAN_BUILD|REPO_BINARY)' "$REDOX_DIR/.config" || true
echo "==> building (the first run is long; later runs are incremental)"

make -C "$REDOX_DIR" all ${REPO_BINARY:+REPO_BINARY=$REPO_BINARY}

[ -f "$IMG" ] || die "build finished but the image is missing: $IMG"

echo
echo "==> image ready: $IMG ($(du -h "$IMG" | cut -f1))"
echo "Next: scripts/flash-sd.sh   (write to the microSD)"
echo "      scripts/serial-log.sh (watch UART during boot)"
