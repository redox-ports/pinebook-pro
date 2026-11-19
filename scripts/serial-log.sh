#!/usr/bin/env bash
# Open the Pinebook Pro serial console (UART2, 1500000 8N1) over the USB-UART
# adapter and capture everything into a timestamped log file under logs/.
#
# Wiring and the hardware switch are documented in experiments/0-serial-console.md.
# Typical flow: start this script, power on the laptop, watch U-Boot -> EFI ->
# Redox output; exit, then share the log file.
#
# Usage:
#   scripts/serial-log.sh [--port /dev/ttyUSB0] [--baud 1500000] [--list]

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BAUD=1500000
PORT=""
LIST=0

die() {
    echo "error: $*" >&2
    exit 1
}

usage() {
    echo "usage: scripts/serial-log.sh [--port /dev/ttyUSB0] [--baud 1500000] [--list]"
    exit 0
}

list_ports() {
    for dev in /dev/ttyUSB* /dev/ttyACM*; do
        [ -e "$dev" ] || continue
        desc=$(udevadm info -q property -n "$dev" 2>/dev/null | grep -E '^ID_(MODEL|VENDOR_FROM_DATABASE|VENDOR)=' | tr '\n' ' ' || true)
        echo "$dev  ${desc}"
    done
}

while [ $# -gt 0 ]; do
    case "$1" in
        --port) PORT="${2:?--port needs a value}"; shift ;;
        --baud) BAUD="${2:?--baud needs a value}"; shift ;;
        --list) LIST=1 ;;
        -h|--help) usage ;;
        *) die "unknown argument: $1 (see --help)" ;;
    esac
    shift
done

if [ "$LIST" -eq 1 ]; then
    list_ports
    exit 0
fi

if [ -z "$PORT" ]; then
    shopt -s nullglob
    ports=(/dev/ttyUSB* /dev/ttyACM*)
    shopt -u nullglob
    if [ "${#ports[@]}" -eq 1 ]; then
        PORT="${ports[0]}"
    else
        echo "serial ports found:"
        list_ports
        die "pass the adapter explicitly: --port /dev/ttyUSB0"
    fi
fi

[ -c "$PORT" ] || die "not a character device: $PORT"
if [ ! -r "$PORT" ] || [ ! -w "$PORT" ]; then
    echo "warning: no read/write permission on $PORT" >&2
    echo "         add yourself to the dialout (Debian/Ubuntu) or uucp (Arch) group," >&2
    echo "         or run this script via sudo" >&2
fi

mkdir -p "$REPO_ROOT/logs"
LOG="$REPO_ROOT/logs/serial-$(date +%Y%m%d-%H%M%S).log"

# tee keeps the console interactive while capturing the log.
if command -v tio >/dev/null 2>&1; then
    echo "tool: tio | log: $LOG | exit: Ctrl+T q"
    tio -b "$BAUD" "$PORT" 2>&1 | tee "$LOG"
elif command -v picocom >/dev/null 2>&1; then
    echo "tool: picocom | log: $LOG | exit: Ctrl+A Ctrl+X"
    picocom -b "$BAUD" "$PORT" 2>&1 | tee "$LOG"
elif command -v screen >/dev/null 2>&1; then
    echo "tool: screen | log: $LOG | exit: Ctrl+A then k, y"
    exec screen -L -Logfile "$LOG" "$PORT" "$BAUD"
else
    die "no serial terminal found: install tio, picocom or screen"
fi
