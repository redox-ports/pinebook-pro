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
- `src/os/uefi/mod.rs`
- `src/os/uefi/video_mode.rs`

## GOP bit depth + non-blocking boot menu

Files: `src/main.rs`, `src/os/bios/vbe.rs`, `src/os/mod.rs`, `src/os/uefi/mod.rs`, `src/os/uefi/video_mode.rs`

`OsVideoMode` now carries `bpp`, derived after `SetMode` from `Mode.Info.PixelFormat`: 32 for the two 8-bit-per-channel formats and the top bit of the R/G/B masks for `PixelBitMask` (`0xffff` → 16). It is exported as `FRAMEBUFFER_BPP` in the boot environment (same blob as the other `FRAMEBUFFER_*` keys) and used for the framebuffer mapping size. Unknown formats export 0, which consumers treat as 32bpp; VBE keeps 32. The same patch makes the mode menu non-blocking: `Os` gained `get_key_timeout(ms)` (default falls back to the old blocking `get_key()`), the UEFI implementation polls `ReadKeyStroke` against a deadline from `RuntimeServices.GetTime` with a bounded-spin fallback for boards whose clock never ticks (no RTC driver on the Pinebook Pro), and `select_mode` boots the highlighted mode after 3 s.
