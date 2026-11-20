# `redox-os/base` changes

- Upstream: https://gitlab.redox-os.org/redox-os/base.git
- Base commit: `8f576d27`
- Apply with: `scripts/apply-patches.sh base`

A `<path>.patch` file is a diff against the upstream `<path>`; every other
file is new and is copied verbatim (see `../README.md`).

## New files (copied verbatim)

- `debugsh/Cargo.toml`
- `debugsh/src/main.rs`
- `drivers/storage/dwmmcd/Cargo.toml`
- `drivers/storage/dwmmcd/src/dwmmc.rs`
- `drivers/storage/dwmmcd/src/log.rs`
- `drivers/storage/dwmmcd/src/main.rs`
- `drivers/usb/ohcid/Cargo.toml`
- `drivers/usb/ohcid/src/cru.rs`
- `drivers/usb/ohcid/src/log.rs`
- `drivers/usb/ohcid/src/main.rs`
- `drivers/usb/ohcid/src/ohci.rs`
- `drivers/usb/ohcid/src/u2phy.rs`
- `init.initfs.d/40_dwmmcd.service`
- `init.initfs.d/40_ohcid.service`
- `init.initfs.d/95_debugsh.service`

## Modified files

- `Cargo.lock`
- `Cargo.toml`
- `Makefile`
- `drivers/graphics/vesad/src/main.rs`
- `drivers/graphics/vesad/src/scheme.rs`
- `drivers/hwd/src/main.rs`
- `init.d/00_pcid-spawner.service`
- `init.initfs.d/40_drivers.target`
- `init.initfs.d/40_pcid-spawner-initfs.service`
- `init.initfs.d/90_initfs.target`

## Workspace and initfs build

Files: `Cargo.toml`, `Cargo.lock`, `Makefile`

`debugsh`, `dwmmcd` and `ohcid` become workspace members. `Makefile` adds
`debugsh` to `INITFS_BINS` (all architectures) and `dwmmcd`/`ohcid` to the
aarch64-only part of the initfs list next to `bcm2835-sdhcid`.

## `debugsh` — interim serial console

Files: `debugsh/Cargo.toml`, `debugsh/src/main.rs`,
`init.initfs.d/95_debugsh.service`, `init.initfs.d/90_initfs.target`

A static initfs daemon on the kernel `debug` scheme (experiment 5): line
discipline and editing, plus `help`, `ls`, `cat`, `echo >`, `hexdump`, `env`,
`exec` and `reboot` (PSCI). The working directory is `/scheme/initfs` and `ls /`
maps to `/scheme`. `95_debugsh.service` starts it on aarch64 only, because
x86_64 gets its console from the rootfs `getty`.

`debugsh` and the rootfs login share one UART RX stream, and the initfs
services start concurrently with the driver target (measured: the shell can
start before `dwmmcd` registers its scheme). The shell therefore waits up to 3 s
for `disk.mmc` and then releases the console to the login; if storage never
comes up it stays as the bring-up console.

## `pcid` stays off aarch64

Files: `drivers/hwd/src/main.rs`, `init.d/00_pcid-spawner.service`,
`init.initfs.d/40_pcid-spawner-initfs.service`

`pcid`'s PCI configuration-space access falls back to `todo!()` on aarch64 and
panics. `hwd` spawns `pcid` directly (legacy path), so gating the services is
not enough: the `hwd` patch skips the spawn on aarch64, and both spawner
services get `condition_architecture = ["x86", "x86_64"]`.

## `dwmmcd` — microSD storage

Files: `drivers/storage/dwmmcd/{Cargo.toml,src/dwmmc.rs,src/log.rs,src/main.rs}`,
`init.initfs.d/40_dwmmcd.service`, `init.initfs.d/40_drivers.target`

A static FIFO/PIO polling driver for the RK3399 DW MMC microSD controller
(`fe320000`), matched by compatible string _and_ `reg` address so the SDIO
controller and the baseline-OS eMMC are never probed (experiment 6). It inherits
U-Boot's CRU clocking, reprograms `CLKDIV` through the CIU clock-update command
(awaited on the CMD START bit — this IP never raises CDONE for clock-only
commands, and `CLKSEL_CON16` is write-mask protected), and registers the
`disk.mmc` scheme before `50_rootfs.service`. Bring-up logs are mirrored to
`/scheme/debug`.

## `vesad` — RGB565 output

Files: `drivers/graphics/vesad/src/main.rs`, `drivers/graphics/vesad/src/scheme.rs`

The firmware framebuffer on this board is RGB565, not 32 bpp (experiment 7).
`vesad` honours `FRAMEBUFFER_BPP`: it maps the real size, converts ARGB8888
client buffers to RGB565 on presentation and uses the same pixel size when
clearing.

## `ohcid` — USB keyboard and touchpad

Files: `drivers/usb/ohcid/{Cargo.toml,src/cru.rs,src/log.rs,src/main.rs,src/ohci.rs,src/u2phy.rs}`,
`init.initfs.d/40_ohcid.service`, `init.initfs.d/40_drivers.target`

A static initfs OHCI host driver for the RK3399 USB2 ports (experiment 8).
U-Boot brings up only the OHCI halves of the EHCI/OHCI pairs, so the internal
hub with the keyboard and touchpad runs at full speed there. The driver polls
(no interrupts, no `nanosleep`), uses `common::dma` for descriptors, enumerates
one level of hub, switches a boot keyboard to the boot protocol and feeds
`inputd`. It first un-gates `CLKGATE_CON(6)`/`CLKGATE_CON(20)` and re-powers the
USB2 PHYs through the GRF: U-Boot switches USB off before the EFI handover, and
touching an unclocked RK3399 peripheral stalls the whole SoC.
