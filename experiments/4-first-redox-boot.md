# Experiment 4 — First boot of the Redox image from microSD

**Status:** pending
**Related:** README §3 (image layout), §4 (local loop), §6 Phase 3; experiment 0 (UART wiring), experiments 2–3 (eMMC boot chain)

## Goal

Boot the locally built, stock-layout Redox aarch64 image (`harddrive.img`, minimal
config) on the Pinebook Pro from microSD and capture the whole chain on UART2
(1500000 8N1). No repacking is needed for this test: the U-Boot already on the
eMMC (2021.07-1 Manjaro ARM, EFI distro boot) scans the microSD ESP at sector
2048 and starts `/EFI/BOOT/BOOTAA64.EFI`.

```
U-Boot (eMMC) -> EFI distro boot -> microSD ESP: /EFI/BOOT/BOOTAA64.EFI (Redox UEFI bootloader)
             -> Redox kernel (DTB-driven UART2) -> serial console
```

Success levels to record:

1. U-Boot finds the microSD ESP and starts `BOOTAA64.EFI`;
2. Redox bootloader hands over to the kernel — its banner may be visible **only
   on the built-in display**: experiment 0 reported U-Boot `Out: vidconsole`, so
   the EFI text console may not mirror to UART;
3. kernel output appears on UART2 (the kernel talks on the serial directly);
4. serial login prompt — full success (parity with the RPi 3 B+ precedent).

## Prerequisites

- Image built on the dev host: `build/redox/build/aarch64/minimal/harddrive.img`
  (`scripts/build-image.sh`).
- microSD flashed (`scripts/flash-sd.sh`, the `SU32G` card) and inserted into the
  Pinebook Pro. The eMMC stays untouched.
- USB-UART adapter wired as in experiment 0 (tip = TX, ring 1 = RX, **3.3 V**),
  UART switch towards the display hinges.
- Adapter visible on the dev host (`scripts/serial-log.sh --list`).

## Actions

On the dev host, before powering the laptop on:

```sh
# 1. Sanity check the image layout (safe: reads the image file, not the card)
sgdisk -p build/redox/build/aarch64/minimal/harddrive.img
# Optional (needs mtools): list the ESP content; ESP starts at 1 MiB
mdir -i build/redox/build/aarch64/minimal/harddrive.img@@1M ::/EFI/BOOT 2>/dev/null || true

# 2. Flash the card, then start the console capture BEFORE power-on
scripts/flash-sd.sh
scripts/serial-log.sh
```

Power on and let autoboot run. Expected markers:

- `Hit any key to stop autoboot` — do **not** press anything;
- distro boot scanning messages for `mmc1`, then something like
  `Found EFI removable media binary /efi/boot/bootaa64.efi` (wording varies by
  U-Boot version);
- then either Redox output, or silence with the chain stalled — record the last
  visible line either way.

If autoboot does not pick the card, interrupt it and run from the `=>` prompt:

```
mmc dev 1
ls mmc 1:1 /
ls mmc 1:1 efi/boot
setenv boot_targets mmc1
run distro_bootcmd
```

Everything lands in `logs/serial-<timestamp>.log` automatically; note the path in
Results.

## Results

### Image layout (pre-flight)

```
PASTE sgdisk -p OUTPUT HERE
```

### ESP content (optional, mtools)

```
PASTE mdir OUTPUT HERE OR WRITE "mtools not installed"
```

### Boot log

```
PASTE THE BOOT LOG HERE (or the path to logs/serial-*.log)
```

### U-Boot prompt session (only if autoboot failed)

```
PASTE HERE OR WRITE "not needed"
```

### Observations

| Question                                                                        | Answer    |
| ------------------------------------------------------------------------------- | --------- |
| U-Boot found the microSD ESP (`Found EFI removable media binary ...`)?          | `FILL IN` |
| `BOOTAA64.EFI` started (Redox bootloader banner — on display and/or on UART)?   | `FILL IN` |
| Kernel output visible on UART2?                                                 | `FILL IN` |
| Serial login prompt reached?                                                    | `FILL IN` |
| Where did the chain stop (last visible line before silence)?                    | `FILL IN` |
| Log file path                                                                   | `FILL IN` |

### Verdict

`FILL IN`
