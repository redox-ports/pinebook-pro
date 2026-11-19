# Experiment 2 — Raw sector verification

**Status:** pending
**Related:** README §3 (target image layout), §7 risk 1

## Goal

Empirically confirm the RK3399 layout this repo relies on:

- idbloader at **byte 32768 (sector 64)** — expect the ASCII magic `RKNS`;
- U-Boot proper at **sector 16384 (8 MiB)** — expect `U-Boot 20xx.xx` strings;
- resolve which medium hosts the running U-Boot (SPI NOR / eMMC / microSD);
- inspect the SPI NOR first-stage area and search the whole 16 MiB flash.

Experiment 0 left the boot medium ambiguous: SPL reported `Trying to boot from MMC1`
(U-Boot numbering), but the microSD has no room for `u-boot.itb` at 8 MiB (its first
partition starts at 4 MiB), while the eMMC layout does fit an embedded U-Boot.

## Prerequisites

- Devices identified in experiment 1: eMMC = `/dev/mmcblk2` (`DA4064`, 58.2 GiB),
  microSD = `/dev/mmcblk1` (`SU32G`, 29.7 GiB). Re-check with `lsblk` before running.
- All commands are read-only with respect to the flash devices. The optional backup
  writes a regular file only.

## Actions

Read **both** block devices and the SPI NOR; keep all outputs.

```sh
# Tool check: xxd may be absent on minimal images (it ships with vim/xxd);
# hexdump -C (util-linux) and grep -a are always available. strings is optional.
which hexdump od grep strings

# 1. idbloader at sector 64 (32 KiB) — eMMC and microSD
sudo dd if=/dev/mmcblk2 bs=512 skip=64 count=32 status=none | hexdump -C | head -20   # eMMC
sudo dd if=/dev/mmcblk1 bs=512 skip=64 count=32 status=none | hexdump -C | head -20   # microSD

# 2. U-Boot proper at sector 16384 (8 MiB) — eMMC and microSD
#    (grep -ao prints only the matched strings with byte offsets; no strings needed)
sudo dd if=/dev/mmcblk2 bs=512 skip=16384 count=4096 status=none | grep -aoi -m5 'u-boot'
sudo dd if=/dev/mmcblk1 bs=512 skip=16384 count=4096 status=none | grep -aoi -m5 'u-boot'

# 3. SPI NOR: first-stage area at sector 64, then search the whole flash
sudo dd if=/dev/mtd0 bs=512 skip=64 count=32 status=none | hexdump -C | head -20
sudo grep -abo 'RKNS' /dev/mtd0 | head
sudo grep -aboi -m5 'u-boot' /dev/mtd0

# 4. Optional: back up the entire 16 MiB SPI NOR (regular file, not the flash)
sudo dd if=/dev/mtd0 of="$HOME/spi-nor-backup.bin" bs=1M status=progress
sha256sum "$HOME/spi-nor-backup.bin"
```

`od -A x -t x1z` (coreutils) is a fallback if `hexdump` is missing. To get full
strings around a match, install `binutils` (`sudo pacman -S binutils`) and use
`strings -a <device>` instead of `grep -ao`.

Optionally, in the U-Boot prompt (`=>`, see experiment 0), confirm device types:

```
mmc dev 0; mmc info
mmc dev 1; mmc info
```

Remember that U-Boot and Linux number the controllers differently (experiment 1):
U-Boot `mmc0` = eMMC, `mmc1` = microSD, `mmc2` = SDIO; Linux `mmc0` = SDIO,
`mmc1` = microSD, `mmc2` = eMMC.

## Results

### idbloader hexdump (sector 64)

Expected: the first bytes contain `RKNS`. eMMC and microSD — paste both.

```
PASTE EMMC HEXDUMP HERE
PASTE MICROSD HEXDUMP HERE
```

### U-Boot strings (sector 16384)

eMMC and microSD — paste both.

```
PASTE EMMC STRINGS HERE (or "nothing found")
PASTE MICROSD STRINGS HERE (or "nothing found")
```

### SPI NOR

```
PASTE SPI SECTOR-64 HEXDUMP HERE
PASTE RKNS GREP OUTPUT HERE (or "no match")
PASTE U-BOOT STRINGS OUTPUT HERE (or "no match")
PASTE BACKUP SHA256 HERE (or "backup not taken")
```

### Observations

| Question                                            | Answer    |
| --------------------------------------------------- | --------- |
| `RKNS` magic at sector 64 (eMMC / microSD / SPI)?   | `FILL IN` |
| U-Boot version at sector 16384 (eMMC / microSD)?    | `FILL IN` |
| U-Boot strings anywhere in the SPI NOR?             | `FILL IN` |
| Which medium hosts the running U-Boot?              | `FILL IN` |
| SPI NOR: blank / env only / full bootloader / other | `FILL IN` |

### Verdict

`FILL IN`
