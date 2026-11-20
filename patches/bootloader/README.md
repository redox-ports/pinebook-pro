# `redox-os/bootloader` changes

- Upstream: https://gitlab.redox-os.org/redox-os/bootloader.git
- Base commit: `f3d9261`
- Apply with: `scripts/apply-patches.sh bootloader`

A `<path>.patch` file is a diff against the upstream `<path>`; every other
file is new and is copied verbatim (see `../README.md`).

## Modified files

- `src/main.rs`
- `src/os/bios/vbe.rs`
- `src/os/mod.rs`
- `src/os/uefi/arch/aarch64.rs`
- `src/os/uefi/mod.rs`
- `src/os/uefi/video_mode.rs`

## GOP bit depth + non-blocking boot menu

Files: `src/main.rs`, `src/os/bios/vbe.rs`, `src/os/mod.rs`, `src/os/uefi/mod.rs`, `src/os/uefi/video_mode.rs`

`OsVideoMode` now carries `bpp`, derived after `SetMode` from `Mode.Info.PixelFormat`: 32 for the two 8-bit-per-channel formats and the top bit of the R/G/B masks for `PixelBitMask` (`0xffff` → 16). It is exported as `FRAMEBUFFER_BPP` in the boot environment (same blob as the other `FRAMEBUFFER_*` keys) and used for the framebuffer mapping size. Unknown formats export 0, which consumers treat as 32bpp; VBE keeps 32. The same patch makes the mode menu non-blocking: `Os` gained `get_key_timeout(ms)` (default falls back to the old blocking `get_key()`), the UEFI implementation polls `ReadKeyStroke` against a deadline from `RuntimeServices.GetTime` with a bounded-spin fallback for boards whose clock never ticks (no RTC driver on the Pinebook Pro), and `select_mode` boots the highlighted mode after 3 s.

## Cache maintenance and the UART2 handover reads

File: `src/os/uefi/arch/aarch64.rs`

Two changes right before `entry_fn(args)`:

1. `cache_sync_instructions(kernel_base, kernel_size)`: `dc civac` over every data
   cache line of the kernel image, then `dsb sy; ic iallu; dsb sy; isb`, so the freshly
   copied kernel is visible to instruction fetch (ARM I-cache/D-cache incoherence).
2. `dump_uart2("pre-kernel")`: four read-only loads from the RK3399 UART2
   (`0xff1a0004/0x0c/0x10/0x14` — IER, LCR, MCR, LSR), logged through EFI `ConOut`.

The reads are load-bearing on this board (experiment 12): without them every image
built after the debug markers were removed hangs before the kernel's first
instruction, with them the same kernel boots to login. Pure delay, a console write and
U-Boot's own LSR reads do not substitute, and the port is healthy either way
(`ier=0 lcr=3 mcr=3 lsr=0x60`), so the mechanism is unknown; the reads must not be
reordered or dropped (see README §7 risk 15).
