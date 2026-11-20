# Experiment 7 — Framebuffer is RGB565: why the screen turned yellow

**Date:** 2026-11-20 (autonomous work session)
**Goal:** explain and fix the yellow panel shown by the VT (`fbbootlogd` /
`fbcond`) once userspace takes the display over.

**Related:** experiment 4 (first framebuffer), README §3 boot chain, §6 Phase 4;
`patches/bootloader-gop-bpp.patch`, `patches/kernel-framebuffer-16bpp.patch`,
`patches/base-pinebook-pro.patch`.

## Symptom (device owner, 2026-11-20)

> "убери этот противный жёлтый цвет с экрана. Из-за него на консоли почти
> ничего не видно"

The kernel `graphical_debug` text (white on black) looked normal, but as soon as
the VT drew, the panel became a yellow field with barely readable lines. Only
after two Enters, i.e. when the VT clears and redraws.

## Root cause (verified against upstream sources)

The Pinebook Pro panel is driven over **eDP**. U-Boot v2021.07 — the installed
bootloader — hardcodes the VOP output depth per interface:

```c
/* drivers/video/rockchip/rk_vop.c, rkvop_ofdata_to_platdata() */
switch (vop_id) {
case VOP_MODE_EDP:            /* Pinebook Pro panel */
case VOP_MODE_LVDS:
	l2bpp = VIDEO_BPP16;
	break;
case VOP_MODE_HDMI:
case VOP_MODE_MIPI:
	l2bpp = VIDEO_BPP32;
	break;
default:
	l2bpp = VIDEO_BPP16;
}
...
uc_priv->bpix = l2bpp;
```

So U-Boot programs an **RGB565** surface and its GOP reports exactly that:

```c
/* lib/efi_loader/efi_gop.c */
if (bpix == VIDEO_BPP32) {
	gopobj->info.pixel_format = EFI_GOT_BGRA8;
} else {
	gopobj->info.pixel_format = EFI_GOT_BITMASK;
	gopobj->info.pixel_bitmask[0] = 0xf800; /* red   */
	gopobj->info.pixel_bitmask[1] = 0x07e0; /* green */
	gopobj->info.pixel_bitmask[2] = 0x001f; /* blue  */
}
```

`gop_set_mode` accepts only mode number 0 (a single mode), so the bit depth
cannot be changed from the UEFI side. The Redox bootloader ignored
`PixelFormat` and everything in Redox assumed 32 bits per pixel.

Why **yellow**, specifically: `console-draw` paints the background as
`0xFF000000` (ARGB black). On a little-endian 16bpp surface those four bytes are
two RGB565 pixels:

| bytes      | pixel            | RGB565 value | colour              |
| ---------- | ---------------- | ------------ | ------------------- |
| `00 00`    | first            | `0x0000`     | black               |
| `00 FF`    | second (u16 LE)  | `0xFF00`     | R=31, G=56, B=0 → yellow |

Every background pixel therefore became a black/yellow column pair — a dense
yellow dither across the whole screen, while the white glyph pixels
(`0xFFFFFFFF` → `0xFFFF`) stayed white, which is exactly the "yellow screen with
half-visible text" the owner described. During `graphical_debug` only the glyph
pixels are white and the rest is zeroed, which is why the boot log looked fine.

The kernel reserved `stride * height * 4` bytes for the framebuffer
(`0x7E9000` for 1920x1080) and `vesad` mapped the same wrong size; the real
surface is `stride * height * 2` bytes.

## Fix

Carry the firmware pixel format from GOP to every component that touches the
linear framebuffer; keep all clients (console-draw, orbital, …) on ARGB8888 and
convert once, at the last step.

1. **Bootloader** (`src/os/uefi/mod.rs`, `src/main.rs`, `src/os/mod.rs`):
   `OsVideoMode` gained `bpp`. After `SetMode` it is derived from
   `Mode.Info.PixelFormat` — 32 for the two 8-bit-per-channel formats, and
   `32 - leading_zeros(RedMask | GreenMask | BlueMask)` for `PixelBitMask`
   (`0xffff` → 16). The value is exported as `FRAMEBUFFER_BPP` in the boot
   environment (same blob as `FRAMEBUFFER_STRIDE`, so `init` passes it to
   services), and the framebuffer is mapped with the real size. VBE (x86) keeps
   32 bpp — that path already accepts only 32-bit modes.
2. **Kernel** (`src/devices/graphical_debug/`): parses `FRAMEBUFFER_BPP`
   (default 32 when absent/unknown), draws glyph pixels in the hardware format
   (RGB565 conversion for 16 bpp), clears rows within the real buffer, and
   reserves `stride * height * bpp / 8` bytes. The last text row is now clamped
   to the display height (1080 is not a multiple of 16).
3. **`vesad`** (`drivers/graphics/vesad/`): reads `FRAMEBUFFER_BPP`, keeps the
   dumb buffer in ARGB8888, and converts to RGB565 while copying to the
   framebuffer (`FrameBuffer::sync`); the "no framebuffer" clear path and the
   `physmap` size also use the real bpp. `FRAMEBUFFER{n}` (multi-output) gained
   an optional fifth field; absent means 32 bpp, so existing x86 setups are
   unaffected.

## Verification

- Build: `make r.base r.kernel r.bootloader image` succeeds; the image contains
  the new `bootloader.efi` (`FRAMEBUFFER_BPP=` in the ESP region) and the new
  kernel/`vesad` strings.
- Hardware: **pending** — boot the new image and check that the VT is white on
  black and that `vesad: 1920x1080 stride 1920 bpp 16 at 0xF7800000` appears in
  the serial log.

## Alternatives considered

- **Ask U-Boot for a 32bpp mode** — impossible in v2021.07: GOP exposes one
  mode and `gop_set_mode` refuses anything but 0.
- **Ship our own U-Boot in the SD image** — the board boots the eMMC U-Boot
  first (SPI → eMMC → microSD), so the SD copy would not run without touching
  the eMMC or SPI NOR; both are off-limits.
- **Patch `console-draw` to emit `0x00000000` backgrounds** — makes black
  background and white text look right by accident (RGB565 black is 0x0000), but
  every other colour (log-level ANSI colours, cursor, inverted cells, any GUI)
  stays broken. Rejected as a hack.
