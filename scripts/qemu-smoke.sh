#!/usr/bin/env bash
# Boot the built Redox image under QEMU and assert that it reaches initfs.
#
# The Redox kernel on aarch64 is devicetree-only, and the packaged AAVMF/edk2
# firmware does not provide a devicetree (no QemuFdtDxe), so this script boots
# through U-Boot for `-M virt`, which installs its own DTB via the EFI
# configuration table. See experiments/9-qemu-smoke-test.md.
#
# Usage:
#   scripts/qemu-smoke.sh [--image PATH] [--log PATH] [--timeout SECONDS]
#                         [--uboot PATH]
#
# The U-Boot binary is taken from --uboot, $PBP_QEMU_UBOOT, an earlier
# extraction under build/qemu-uboot, or the system package (extracted without
# root into build/qemu-uboot).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="${PBP_IMAGE:-$REPO_ROOT/build/redox/build/aarch64/minimal/harddrive.img}"
LOG="$REPO_ROOT/build/qemu-smoke.log"
UBOOT="${PBP_QEMU_UBOOT:-}"
SECONDS_LIMIT=150

die() {
    echo "error: $*" >&2
    exit 1
}

usage() {
    sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit 0
}

while [ $# -gt 0 ]; do
    case "$1" in
        --image) IMAGE="${2:?--image needs a value}"; shift ;;
        --log) LOG="${2:?--log needs a value}"; shift ;;
        --timeout) SECONDS_LIMIT="${2:?--timeout needs a value}"; shift ;;
        --uboot) UBOOT="${2:?--uboot needs a value}"; shift ;;
        -h|--help) usage ;;
        *) die "unknown argument: $1 (see --help)" ;;
    esac
    shift
done

[ -f "$IMAGE" ] || die "image not found: $IMAGE (build it first: scripts/build-image.sh)"
command -v qemu-system-aarch64 >/dev/null 2>&1 \
    || die "qemu-system-aarch64 not found (install qemu-system-arm)"

# --- locate (or extract) a U-Boot binary for -M virt ------------------------
find_uboot() {
    if [ -n "$UBOOT" ] && [ -f "$UBOOT" ]; then
        printf '%s\n' "$UBOOT"
        return 0
    fi
    local cached="$REPO_ROOT/build/qemu-uboot/usr/lib/u-boot/qemu_arm64/u-boot.bin"
    if [ -f "$cached" ]; then
        printf '%s\n' "$cached"
        return 0
    fi
    return 1
}

if ! UBOOT_BIN=$(find_uboot); then
    echo "==> no U-Boot binary found, extracting the u-boot-qemu package"
    work=$(mktemp -d)
    if (cd "$work" && apt-get download u-boot-qemu >/dev/null 2>&1); then
        deb=$(ls "$work"/*.deb 2>/dev/null | head -n 1)
        if [ -n "$deb" ]; then
            mkdir -p "$REPO_ROOT/build/qemu-uboot"
            dpkg-deb -x "$deb" "$REPO_ROOT/build/qemu-uboot"
        fi
    fi
    rm -rf "$work"
    UBOOT_BIN=$(find_uboot) || die "could not obtain u-boot-qemu; pass --uboot PATH \
(download: apt-get download u-boot-qemu; dpkg-deb -x u-boot-qemu_*.deb build/qemu-uboot)"
fi
echo "==> u-boot: $UBOOT_BIN"

# --- run --------------------------------------------------------------------
mkdir -p "$(dirname "$LOG")"
rm -f "$LOG"

echo "==> booting $IMAGE for up to ${SECONDS_LIMIT}s (serial: $LOG)"
set +e
timeout "$SECONDS_LIMIT" qemu-system-aarch64 \
    -M virt,gic-version=3 -cpu cortex-a72 -m 4G -smp 1 \
    -bios "$UBOOT_BIN" \
    -drive if=none,id=d0,file="$IMAGE",format=raw,readonly=on \
    -device virtio-blk-device,drive=d0 \
    -device ramfb -display none \
    -serial "file:$LOG" -monitor none -no-reboot
status=$?
set -e
echo "==> qemu exit status $status (124 = the soft timeout fired, which is expected)"

# --- assertions -------------------------------------------------------------
fail=0
check() {
    local pattern="$1"
    local what="$2"
    if grep -q -- "$pattern" "$LOG"; then
        echo "  ok   $what"
    else
        echo "  FAIL $what (no '$pattern')"
        fail=1
    fi
}

echo "==> checking the boot log"
check "Hardware descriptor: DeviceTree" "U-Boot passed a devicetree"
check "dtb::serial:INFO -- UART Compatible" "kernel bound the UART from the DT"
check "init: switchroot to /scheme/initfs" "kernel reached initfs userspace"
check "debugsh: interactive console" "initfs console is up"

if [ "$fail" -ne 0 ]; then
    echo
    echo "last 20 log lines:"
    tail -n 20 "$LOG"
    die "smoke test failed (full log: $LOG)"
fi

echo
echo "==> smoke test passed. Known limits of this environment:"
echo "    - no RK3399 peripherals (DW MMC, OHCI, CRU/GRF, eDP): drivers only"
echo "      exercise their own error paths here"
echo "    - no root filesystem: Redox has no virtio-mmio block driver, so the"
echo "      boot stops after switchroot with the initfs console"
