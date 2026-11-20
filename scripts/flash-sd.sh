#!/usr/bin/env bash
# Write the built Redox image to the Pinebook Pro microSD card.
#
# Safety rails (the script refuses to guess):
#   * auto-detects only the known microSD (model SU32G); otherwise pass --device
#   * never touches the eMMC (model DA4064, mmcblk2*): it holds the baseline OS
#   * refuses the disk the running root filesystem lives on, and any disk with
#     mounted partitions
#   * requires an explicit YES (terminal prompt or --yes) before writing
#
# Usage:
#   scripts/flash-sd.sh [--image PATH] [--device DEV] [--yes] [--no-verify]

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="${PBP_IMAGE:-$REPO_ROOT/build/redox/build/aarch64/minimal/harddrive.img}"
DEVICE=""
ASSUME_YES=0
VERIFY=1

die() {
    echo "error: $*" >&2
    exit 1
}

usage() {
    echo "usage: scripts/flash-sd.sh [--image PATH] [--device DEV] [--yes] [--no-verify]"
    exit 0
}

while [ $# -gt 0 ]; do
    case "$1" in
        --image) IMAGE="${2:?--image needs a value}"; shift ;;
        --device) DEVICE="${2:?--device needs a value}"; shift ;;
        --yes) ASSUME_YES=1 ;;
        --no-verify) VERIFY=0 ;;
        -h|--help) usage ;;
        *) die "unknown argument: $1 (see --help)" ;;
    esac
    shift
done

[ -f "$IMAGE" ] || die "image not found: $IMAGE (build it first: scripts/build-image.sh)"
IMG_SIZE=$(stat -c %s "$IMAGE")

SUDO=""
if [ "$(id -u)" -ne 0 ]; then
    command -v sudo >/dev/null 2>&1 || die "sudo is required to write a raw device"
    SUDO="sudo"
fi

# Run a command, with sudo when not already root.
run() {
    if [ -n "$SUDO" ]; then
        "$SUDO" "$@"
    else
        "$@"
    fi
}

human_size() {
    numfmt --to=iec --suffix=B "$1" 2>/dev/null || echo "$1 bytes"
}

# --- pick the target device -------------------------------------------------
if [ -n "$DEVICE" ]; then
    [ -b "$DEVICE" ] || die "not a block device: $DEVICE"
else
    echo "==> looking for the microSD (removable disk, model SU32G)"
    CANDIDATES=$(lsblk -drno NAME,TYPE,RM,MODEL | awk -F',' '$2 == "disk" && $3 == 1 && $4 == "SU32G" { print "/dev/" $1 }')
    if [ "$(printf '%s' "$CANDIDATES" | grep -c .)" -eq 1 ]; then
        DEVICE="$CANDIDATES"
    else
        echo "no unique microSD found. Removable disks on this system:"
        lsblk -drno NAME,SIZE,TYPE,RM,MODEL | awk -F',' '$2 == "disk" && $3 == 1 { print "  /dev/" $1 "  " $2 "  " $5 }'
        die "pass the card explicitly: --device /dev/sdX"
    fi
fi
DEV="$DEVICE"

# --- safety checks ----------------------------------------------------------
case "$(basename "$DEV")" in
    mmcblk2*|*boot0*|*boot1*|*rpmb*)
        die "refusing $DEV: looks like the eMMC or its hardware partitions (baseline OS lives there)"
        ;;
esac

MODEL=$(lsblk -dn -o MODEL "$DEV" | xargs)
if [ "$MODEL" = "DA4064" ]; then
    die "refusing $DEV: this is the Pinebook Pro eMMC (baseline OS)"
fi

# Refuse any disk that hosts a currently mounted filesystem (root, home, ...).
while read -r SRC _; do
    case "$SRC" in
        /dev/*) ;;
        *) continue ;;
    esac
    PARENT=$(lsblk -dnro PKNAME "$SRC" 2>/dev/null || true)
    [ -n "$PARENT" ] || PARENT=$(basename "$SRC")
    if [ "$SRC" = "$DEV" ] || [ "/dev/$PARENT" = "$DEV" ]; then
        die "refusing $DEV: the running system uses it ($SRC is mounted from it)"
    fi
done < /proc/mounts

MOUNTED=$(lsblk -no MOUNTPOINT "$DEV" 2>/dev/null | sed '/^$/d' || true)
if [ -n "$MOUNTED" ]; then
    echo "$MOUNTED" | while read -r M; do echo "  mounted: $M" >&2; done
    die "refusing $DEV: partitions are mounted; unmount them first"
fi

DEV_SIZE=$(run blockdev --getsize64 "$DEV")
if [ "$DEV_SIZE" -lt "$IMG_SIZE" ]; then
    die "device too small: $(human_size "$DEV_SIZE") < image $(human_size "$IMG_SIZE")"
fi

# --- confirm ----------------------------------------------------------------
echo "image : $IMAGE ($(human_size "$IMG_SIZE"))"
echo "device: $DEV  model: ${MODEL:-?}  size: $(human_size "$DEV_SIZE")"
if [ "$ASSUME_YES" -ne 1 ]; then
    if [ -t 0 ]; then
        printf 'erase %s and write the image? type YES to confirm: ' "$DEV"
        read -r ANSWER
        [ "$ANSWER" = "YES" ] || die "aborted"
    else
        die "not a terminal: pass --yes to confirm"
    fi
fi

# --- write and verify -------------------------------------------------------
echo "==> writing (a few minutes)"
run dd if="$IMAGE" of="$DEV" bs=4M conv=fsync status=progress
sync

if [ "$VERIFY" -eq 1 ]; then
    echo "==> verifying the written bytes"
    run cmp -n "$IMG_SIZE" "$IMAGE" "$DEV"
    echo "verify: OK"
fi

# The kernel rescans the partition table once the raw writer closes the
# device, and desktop automounters (udisks) may grab the freshly created
# ESP afterwards. The image bytes are already on the media (dd conv=fsync
# flushed them and the verify step re-read them), but pulling a mounted
# FAT partition can still corrupt its metadata, so warn instead of
# staying silent.
MOUNTED=$(lsblk -no MOUNTPOINT "$DEV" 2>/dev/null | sed '/^$/d' || true)
if [ -n "$MOUNTED" ]; then
    echo "warning: something mounted partitions of the card after the write:" >&2
    echo "$MOUNTED" | while read -r M; do echo "  mounted: $M" >&2; done
    echo "unmount before removing the card, e.g.: udisksctl unmount -b <partition>" >&2
else
    echo "==> card contents are written and flushed; it is safe to remove it now"
fi

echo
echo "==> done. Put the card into the Pinebook Pro and run scripts/serial-log.sh"
echo "    before powering on, to capture the boot log."
