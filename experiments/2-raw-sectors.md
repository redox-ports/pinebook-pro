# Experiment 2 — Raw sector verification

**Status:** pending
**Related:** README §3 (target image layout), §7 risk 1

## Goal

Empirically confirm the RK3399 layout this repo relies on:

- idbloader at **byte 32768 (sector 64)** — expect the ASCII magic `RKNS`;
- U-Boot proper at **sector 16384 (8 MiB)** — expect `U-Boot 20xx.xx` strings;
- SPI NOR blank (expected `ff` bytes at sector 64 of `/dev/mtd0`, if exposed);
- resolve which medium (eMMC or microSD) currently hosts the running U-Boot:
  experiment 0 left this ambiguous — SPL reported `Trying to boot from MMC1`, but the
  microSD has no room for `u-boot.itb` at 8 MiB (first partition at 4 MiB), while the
  eMMC layout does fit an embedded U-Boot.

## Prerequisites

- Devices identified in experiment 1. The examples below use `/dev/mmcblk1` —
  **replace it with the actual device names**. All commands are read-only.
- Run steps 1–2 for **both** block devices (eMMC and microSD) and keep both outputs;
  the comparison resolves the experiment 0 ambiguity.

## Actions

Optionally, in the U-Boot prompt, confirm device types with `mmc dev 0; mmc info` and
`mmc dev 1; mmc info` (`SD` vs `MMC`).

```sh
# 1. idbloader at sector 64 (32 KiB)
sudo dd if=/dev/mmcblk1 bs=512 skip=64 count=32 status=none | xxd | head -20

# 2. U-Boot proper at sector 16384 (8 MiB)
sudo dd if=/dev/mmcblk1 bs=512 skip=16384 count=4096 status=none | strings | grep -m 5 -i 'u-boot'

# 3. SPI NOR at sector 64 (only if /dev/mtd0 exists)
sudo dd if=/dev/mtd0 bs=512 skip=64 count=4 status=none | xxd
```

## Results

### idbloader hexdump (sector 64)

Expected: the first bytes contain `RKNS`. Run on eMMC and microSD; paste both.

```
PASTE HEXDUMP HERE
```

### U-Boot strings (sector 16384)

eMMC and microSD — paste both.

```
PASTE STRINGS OUTPUT HERE
```

### SPI NOR hexdump (sector 64)

```
PASTE SPI HEXDUMP HERE (or write "no /dev/mtd0")
```

### Observations

| Question                                          | Answer    |
| ------------------------------------------------- | --------- |
| `RKNS` magic found at sector 64 (eMMC / microSD)? | `FILL IN` |
| U-Boot version at sector 16384 (eMMC / microSD)?  | `FILL IN` |
| Which device hosts the running U-Boot?            | `FILL IN` |
| SPI NOR: blank / non-blank / not exposed          | `FILL IN` |

### Verdict

`FILL IN` — e.g. "Offsets confirmed on eMMC; SPI blank."
