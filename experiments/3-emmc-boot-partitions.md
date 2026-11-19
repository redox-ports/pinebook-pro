# Experiment 3 — eMMC hardware boot partitions

**Status:** pending
**Related:** README §3 (image layout), experiment 2 (follow-up)

## Goal

Experiment 2 showed that:

- the eMMC user area at sector 64 contains **no `RKNS` idbloader** (arbitrary data);
- U-Boot proper **is** present in the eMMC user area at sector 16384;
- SPI NOR and microSD contain no bootloader.

Therefore the first stage of the shipped boot chain is expected in an eMMC **hardware
boot partition** (`boot0`/`boot1`). This experiment confirms it, captures the boot
partition configuration, and pins down the complete shipped chain:

```
BootROM -> eMMC boot0/boot1 (idbloader: TPL+SPL) -> eMMC user area 8 MiB (u-boot.itb)
        -> EFI distro boot -> Manjaro on eMMC
```

## Prerequisites

- All commands are **read-only**. Never write to `mmcblk2boot0`/`mmcblk2boot1`: they
  hold the shipped bootloader and the eMMC boot areas are hardware write targets.
- The boot partitions are exposed by Linux as `/dev/mmcblk2boot0` and
  `/dev/mmcblk2boot1` (4 MiB each, confirmed in experiment 1).
- Optional: `mmc-utils` for reading EXT_CSD (`sudo pacman -S mmc-utils`).

## Actions

```sh
# 0. Sanity check: boot partitions exist and their size
lsblk -o NAME,SIZE,TYPE /dev/mmcblk2
ls -l /dev/mmcblk2boot0 /dev/mmcblk2boot1

# 1. First 64 KiB of each boot area (look for RKNS / U-Boot / anything)
sudo dd if=/dev/mmcblk2boot0 bs=512 count=128 status=none | hexdump -C | head -30
sudo dd if=/dev/mmcblk2boot1 bs=512 count=128 status=none | hexdump -C | head -30

# 2. Whole-area signature search (4 MiB each)
sudo grep -abo 'RKNS' /dev/mmcblk2boot0 | head
sudo grep -aboi -m5 'u-boot' /dev/mmcblk2boot0
sudo grep -abo 'RKNS' /dev/mmcblk2boot1 | head
sudo grep -aboi -m5 'u-boot' /dev/mmcblk2boot1

# 3. eMMC boot configuration (EXT_CSD) — optional but very useful
which mmc || sudo pacman -S mmc-utils
sudo mmc extcsd read /dev/mmcblk2 | grep -Ei 'PARTITION_CONFIG|BOOT_PARTITION|BOOT_ACK|BOOT_BUS|PARTITIONING|RPMB_SIZE'
```

Expected if the hypothesis is correct: `boot0` (or `boot1`) starts with a Rockchip
idbloader (`RKNS` magic) or otherwise contains bootloader code, and `PARTITION_CONFIG`
shows the boot partition enabled (`BOOT_PARTITION_ENABLE`) with boot acknowledge
settings.

## Results

### Boot partitions present

```
PASTE lsblk / ls OUTPUT HERE
```

### boot0 first 64 KiB

```
PASTE BOOT0 HEXDUMP HERE
```

### boot1 first 64 KiB

```
PASTE BOOT1 HEXDUMP HERE
```

### Signature search

```
PASTE RKNS / U-BOOT GREP OUTPUT FOR boot0 HERE
PASTE RKNS / U-BOOT GREP OUTPUT FOR boot1 HERE
```

### EXT_CSD boot configuration

```
PASTE mmc extcsd OUTPUT HERE (or write "mmc-utils not installed")
```

### Observations

| Question                                                        | Answer    |
| --------------------------------------------------------------- | --------- |
| Boot area containing the first stage (boot0 / boot1 / neither)  | `FILL IN` |
| `RKNS` magic present in boot0?                                  | `FILL IN` |
| `RKNS` magic present in boot1?                                  | `FILL IN` |
| U-Boot strings present in boot0 / boot1?                        | `FILL IN` |
| `PARTITION_CONFIG`: boot partition enable / boot ack            | `FILL IN` |
| Does the shipped chain match the expected path (ROM → boot0 → 8 MiB → EFI)? | `FILL IN` |

### Verdict

`FILL IN`
