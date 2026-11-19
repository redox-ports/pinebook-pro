# Experiment 4 — First boot of the Redox image from microSD

**Status:** in progress (2026-11-20) — milestones 1–2 confirmed on hardware: U-Boot
started `BOOTAA64.EFI` and the Redox bootloader runs (output visible on UART); the chain
is waiting at the bootloader menu — kernel handover capture pending
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
U-Boot TPL 2021.07-1 (Jul 12 2021 - 13:27:45)
Channel 0: LPDDR4, 50MHz
BW=32 Col=10 Bk=8 CS0 Row=15 CS1 Row=15 CS=2 Die BW=16 Size=2048MB
Channel 1: LPDDR4, 50MHz
BW=32 Col=10 Bk=8 CS0 Row=15 CS1 Row=15 CS=2 Die BW=16 Size=2048MB
256B stride
lpddr4_set_rate: change freq to 400000000 mhz 0, 1
lpddr4_set_rate: change freq to 800000000 mhz 1, 0
Trying to boot from BOOTROM
Returning to boot ROM...

U-Boot SPL 2021.07-1 (Jul 12 2021 - 13:27:45 +0000)
Trying to boot from MMC1


U-Boot 2021.07-1 (Jul 12 2021 - 13:27:45 +0000) Manjaro ARM

SoC: Rockchip rk3399
Reset cause: POR
Model: Pine64 Pinebook Pro
DRAM:  3.9 GiB
PMIC:  RK808
MMC:   mmc@fe310000: 2, mmc@fe320000: 1, sdhci@fe330000: 0
Loading Environment from SPIFlash... SF: Detected gd25q128 with page size 256 Bytes, erase size 4 KiB, total 16 MiB
*** Warning - bad CRC, using default environment

In:    serial
Out:   vidconsole
Err:   vidconsole
Model: Pine64 Pinebook Pro
Net:   No ethernet found.
starting USB...
Bus usb@fe3a0000: USB OHCI 1.0
Bus usb@fe3e0000: USB OHCI 1.0
scanning bus usb@fe3a0000 for devices... 2 USB Device(s) found
scanning bus usb@fe3e0000 for devices... 4 USB Device(s) found
       scanning usb for storage devices... 0 Storage Device(s) found
Hit any key to stop autoboot:  0

Device 0: unknown device
switch to partitions #0, OK
mmc1 is current device
Scanning mmc 1:2...
libfdt fdt_check_header(): FDT_ERR_BADMAGIC
Card did not respond to voltage select! : -110
Scanning disk mmc@fe310000.blk...
Disk mmc@fe310000.blk not ready
Scanning disk mmc@fe320000.blk...
** Unrecognized filesystem type **
** Unrecognized filesystem type **
Scanning disk sdhci@fe330000.blk...
Found 7 disks
** Unable to read file ubootefi.var **
Failed to load EFI variables
BootOrder not defined
EFI boot manager: Cannot load any image
Found EFI removable media binary efi/boot/bootaa64.efi
163328 bytes read in 23 ms (6.8 MiB/s)
libfdt fdt_check_header(): FDT_ERR_BADMAGIC
Booting /efi\boot\bootaa64.efi

WARN - Failed to get EFI EDID from handle Handle(4109867680): Status(0x8000000000000003) "unsupported"
INFO - Currently in EL2
Redox OS Bootloader 1.0.0 on aarch64/UEFI
Hardware descriptor: DeviceTree(f3eb4000, 11000)
Looking for RedoxFS:
RedoxFS 091a3795-66ba-43e2-a5ad-c0ac79056194: 126 MiB

Output 0, best resolution: 1920x1080
Arrow keys and enter select mode
Press l to  enable live mode
Press e to edit boot environment

 1920x1080  16:9

[... capture continues at the menu selection — see follow-up below ...]
```

### Notes (facts extracted from the log)

- **Milestone 1 confirmed:** U-Boot distro boot found the SD ESP — `Found EFI removable
media binary efi/boot/bootaa64.efi`, `163328 bytes read in 23 ms (6.8 MiB/s)`,
  `Booting /efi\boot\bootaa64.efi`. The stock (un-repacked) image layout works with the
  eMMC U-Boot as predicted.
- **Milestone 2 confirmed — and on UART:** `Redox OS Bootloader 1.0.0 on aarch64/UEFI`.
  The pre-run expectation that the bootloader banner might be display-only (`Out:
vidconsole`) was **wrong**: the bootloader's output reaches the serial console.
  `INFO - Currently in EL2` — the bootloader starts at EL2 (per README, it drops to EL1).
- **DTB is discovered:** `Hardware descriptor: DeviceTree(f3eb4000, 11000)` — the
  bootloader received U-Boot's FDT (address `0xf3eb4000`, size `0x11000` = 69,632 B).
  This is the `DTB_ADDR`/`DTB_SIZE` path from README §2 working end-to-end.
- **RedoxFS is recognized on the SD:** `RedoxFS 091a3795-66ba-43e2-a5ad-c0ac79056194:
126 MiB` (= `filesystem_size` 256 MiB − ESP 128 MiB − GPT overhead).
- **GOP works:** `Output 0, best resolution: 1920x1080` — U-Boot exposes a framebuffer
  with the native panel mode; the Phase 4 display path via `FRAMEBUFFER_*` is viable.
- **The chain stopped at the bootloader menu** (` 1920x1080  16:9`): it waits for the
  mode selection. U-Boot reported `In: serial`, so EFI console input should be the serial
  port — Enter can be pressed from the USB-UART terminal on the dev host.
- Scan noise, all harmless and expected: `Device 0: unknown device` (usb0, no storage),
  `Scanning mmc 1:2...` + `FDT_ERR_BADMAGIC` (probing p2 for a boot script), `Card did
not respond to voltage select! : -110` (mmc2 = SDIO Wi-Fi init failure),
  `** Unrecognized filesystem type **` (RedoxFS/ext4 partitions are not FAT for the
  U-Boot file loader), `ubootefi.var` / `BootOrder not defined` (default environment —
  the SPI env holds no valid env, experiment 0).

### U-Boot prompt session (only if autoboot failed)

```
PASTE HERE OR WRITE "not needed"
```

### Observations

| Question                                                                      | Answer                                                                                            |
| ----------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| U-Boot found the microSD ESP (`Found EFI removable media binary ...`)?        | **Yes** — `efi/boot/bootaa64.efi`, 163328 bytes read in 23 ms, `Booting /efi\boot\bootaa64.efi`   |
| `BOOTAA64.EFI` started (Redox bootloader banner — on display and/or on UART)? | **Yes, on UART** — `Redox OS Bootloader 1.0.0 on aarch64/UEFI`; also `INFO - Currently in EL2`    |
| Kernel output visible on UART2?                                               | **Not yet** — the chain is waiting at the bootloader menu; capture of the continuation is pending |
| Serial login prompt reached?                                                  | Not yet                                                                                           |
| Where did the chain stop (last visible line before silence)?                  | Bootloader menu: `Output 0, best resolution: 1920x1080` → ` 1920x1080  16:9`, waiting for Enter   |
| Log file path                                                                 | `FILL IN`                                                                                         |

### Verdict

`FILL IN` — after the menu selection is captured (kernel handover).

### Follow-up (in progress)

At the bootloader menu, press **Enter** (select the `1920x1080` mode) — from the serial
terminal on the dev host (`In: serial`), or the laptop keyboard. Expected continuation:

- the bootloader sets the mode and loads the kernel/initfs from RedoxFS
  (`/usr/lib/boot/{kernel,initfs}`);
- the first **Redox kernel** messages appear on UART2 (DTB-driven `snps,dw-apb-uart`);
- target: the serial login prompt.

If nothing appears after the selection, record the last line — that shifts the debugging
frontier (bootloader `Press e` edit mode, QEMU cross-check).
