# Experiment 2 — Raw sector verification

**Status:** pending
**Related:** README §3 (target image layout), §7 risk 1

## Goal

Empirically confirm the RK3399 layout this repo relies on:

- idbloader at **byte 32768 (sector 64)** — expect the ASCII magic `RKNS`;
- U-Boot proper at **sector 16384 (8 MiB)** — expect `U-Boot 20xx.xx` strings;
- SPI NOR blank (expected `ff` bytes at sector 64 of `/dev/mtd0`, if exposed).

## Prerequisites

- Boot device identified in experiment 1. The examples below use `/dev/mmcblk1` —
  **replace it with the actual boot device**. All commands are read-only.

## Actions

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

Expected: the first bytes contain `RKNS`.

```
PASTE HEXDUMP HERE
```

### U-Boot strings (sector 16384)

```
PASTE STRINGS OUTPUT HERE
```

### SPI NOR hexdump (sector 64)

```
PASTE SPI HEXDUMP HERE (or write "no /dev/mtd0")
```

### Observations

| Question                                             | Answer    |
| ---------------------------------------------------- | --------- |
| `RKNS` magic found at sector 64?                     | `FILL IN` |
| U-Boot version found at sector 16384? (exact string) | `FILL IN` |
| Which device was read (eMMC / microSD)?              | `FILL IN` |
| SPI NOR: blank / non-blank / not exposed             | `FILL IN` |

### Verdict

`FILL IN` — e.g. "Offsets confirmed on eMMC; SPI blank."
