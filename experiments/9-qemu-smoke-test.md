# Experiment 9 — QEMU boot test bed and a non-blocking boot menu

**Date:** 2026-11-20 (autonomous work session)
**Goal:** a repeatable boot test that does not need the laptop, so every change
(watchdog, drivers, framebuffer, bootloader) can be checked before it is
flashed to a microSD card.

**Related:** experiment 8 (the DMA self-test uses this test bed),
`patches/bootloader-gop-bpp.patch` (menu timeout), README §5 stage 4 and the
Phase 1 checklist item "QEMU aarch64 UEFI smoke test".

## Why AAVMF does not work for Redox

The Redox kernel on aarch64 is devicetree-only: `hwd` selects its backend from
the hardware descriptor the bootloader passes, and the kernel parses `DTB_ADDR`
from its environment. The obvious QEMU setup — `-M virt` plus the packaged
`AAVMF`/edk2 firmware — cannot provide that:

- Debian's `qemu-efi-aarch64` build does **not** include `QemuFdtDxe`, so the
  firmware never installs a devicetree configuration table.
- The bootloader therefore reports `Hardware descriptor: Acpi(...)` and the
  kernel initializes nothing: no serial, no IRQ chip, no output.
- Passing `-dtb` does not help (the firmware ignores it), and AAVMF has no
  graphics at all, so `ramfb` is unused.

## Working recipe

Use U-Boot instead: it builds its own devicetree for `-M virt` and installs it
through the EFI configuration table. The Debian package is enough and can be
unpacked without root:

```sh
apt-get download u-boot-qemu
mkdir -p /tmp/uboot-pkg
dpkg-deb -x u-boot-qemu_*.deb /tmp/uboot-pkg
```

The binary is `/tmp/uboot-pkg/usr/lib/u-boot/qemu_arm64/u-boot.bin`. Boot the
raw image with:

```sh
timeout 170 qemu-system-aarch64 -M virt,gic-version=3 -cpu cortex-a72 -m 4G -smp 1 \
  -bios /tmp/uboot-pkg/usr/lib/u-boot/qemu_arm64/u-boot.bin \
  -drive if=none,id=d0,file=build/redox/build/aarch64/minimal/harddrive.img,format=raw \
  -device virtio-blk-device,drive=d0 \
  -device ramfb -display none -serial file:/tmp/qemu.log -monitor none -no-reboot
```

Notes:

- `gic-version=3` is **mandatory**. The RK3399 is GICv3; with QEMU's default
  GICv2 the kernel panics during IRQ chip initialization.
- `-serial file:` (or `stdio` with a terminal) is enough; `-display none`
  keeps it headless.
- `-no-reboot` makes a boot loop visible instead of silently restarting.
- `-m 4G` matches the laptop, which matters for one detail: on `virt` the RAM
  reaches `0x1_0000_0000`, so MMIO addresses such as `0xfe3e0000` fall inside
  the kernel's frame table and `physmap` refuses them, while on the Pinebook
  Pro (RAM ends at `0xf8000000`) the same call succeeds. Expect driver logs to
  differ there; that is a property of the emulated memory map, not a bug.

## What the run validates

- Bootloader: firmware descriptor type, framebuffer discovery, kernel entry.
- Kernel: GICv3, generic timer, UART, DTB parsing, memory map.
- Initfs: `init`, `randd`, the driver daemons, `vesad`, `debugsh`, the
  `40_drivers.target` ordering, `switchroot to /usr`.
- Driver self-tests that do not need RK3399 hardware, in particular the
  `common::dma` path (`ohcid` allocates 4 KiB, writes and reads it back when
  the devicetree has no OHCI node).

Observed tail of a healthy run:

```
Hardware descriptor: DeviceTree(13d508000, 2000)
...
init: switchroot to /scheme/initfs /scheme/initfs/etc
[   0.002] ohcid: starting
[   0.008] ohcid: no OHCI nodes in the devicetree, trying the known RK3399 addresses
[   0.009] ohcid: selftest: DMA ok, phys 0x4324c000 virt 0x00015000
[   0.009] ohcid: selftest: DMA readback 0x5a 0xa5
...
vesad: No boot framebuffer
init: switchroot to /usr /etc
debugsh: interactive console on /scheme/debug
```

## Limits

- **No RK3399 peripherals.** `virt` exposes virtio devices, PL011, PL031 and
  GICv3; the DW MMC, the OHCI controllers, the Rockchip CRU/GRF and the eDP
  panel do not exist. Driver work for those is only checked for "does not
  crash" (the address is refused cleanly).
- **No usable display.** The packaged U-Boot has no `ramfb` video driver, so
  there is no GOP framebuffer: `vesad` logs `No boot framebuffer` and the VT
  cannot be exercised.
- **No root filesystem.** Redox has no virtio-mmio block driver, so `redoxfs`
  cannot mount the image; the run is expected to stop after `switchroot`, with
  the initfs `debugsh` console as the last reachable state. That is exactly the
  state we want to test in initfs anyway.
- Boot menu input: with `-serial file:` there is no stdin, which is why the
  menu must not block (next section).

## Boot menu timeout

The bootloader used to block in `get_key()` on the mode menu, so a boot with no
keyboard (headless QEMU, or the laptop before the USB driver works) hung at
`Arrow keys and enter select mode`.

Fix in `patches/bootloader-gop-bpp.patch`:

- `Os` gained `get_key_timeout(ms) -> OsKey`, whose default implementation
  calls the old blocking `get_key()`, so VBE and any other OS keeps working.
- The UEFI implementation polls the non-blocking `ReadKeyStroke` in a loop with
  a deadline from `RuntimeServices.GetTime` (10 ms units). The clock cannot be
  trusted: boards without an RTC driver return a constant time (the Pinebook
  Pro has no RK808 driver in the installed U-Boot), so the deadline is used
  only after the clock has been observed to tick; a dead or failing clock falls
  back to a bounded spin (400 000 iterations) instead of hanging.
- `select_mode` waits 3 s, then boots the highlighted mode; the hint reads
  `Arrow keys and enter select mode (booting in 3 s)`.

Verified in QEMU: with no stdin attached the boot continues on its own after
the timeout and reaches the kernel.

## Status

- Test bed works and is used by the driver work (experiment 8).
- Menu timeout implemented and **verified in QEMU**; hardware behaviour is the
  same code path (the clock fallback was added precisely because the laptop has
  no RTC driver), but it has not been re-verified on the laptop since the
  change.
- Still open for CI: the checklist item wants the standard `make qemu` (AAVMF)
  flow; that cannot work as-is for this platform (see above). The practical
  smoke test is the U-Boot recipe documented here, which needs the
  `u-boot-qemu` package in the runner image.
