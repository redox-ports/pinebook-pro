# Experiment 1 — System inventory

**Status:** pending
**Related:** README §1 (hardware), §7 risk 3

## Goal

Record what the shipped system is (OS, kernel, U-Boot environment), how storage is
exposed (`dw_mmc` for microSD, `sdhci-arasan` for eMMC), and whether the SPI NOR flash
is visible to Linux.

## Actions

Run everything below **inside the installed Linux system** (boot normally and do not
stop autoboot at the U-Boot prompt). Keep the full output in Results.

```sh
uname -a
cat /proc/device-tree/model; echo
cat /proc/device-tree/compatible; echo
lsblk -o NAME,SIZE,TYPE,MODEL,MOUNTPOINT
sudo fdisk -l
cat /proc/cmdline
ls /dev/mtd* 2>/dev/null; cat /proc/mtd 2>/dev/null
dmesg | grep -Ei 'mmc|sdhci|dw_mmc|spi-nor|rockchip' | head -40
lsusb
which fw_printenv flashrom mtd_debug
```

If `fw_printenv` exists, also run:

```sh
sudo fw_printenv | head -40
```

`fw_printenv` is a Linux user-space tool (Manjaro package `uboot-tools`) and may require
`/etc/fw_env.config` pointing at the SPI environment area. If it fails or is missing,
record the error message — that is a valid result.

Identify the block devices explicitly (the eMMC usually reports a model name, SD cards
report the card name; this also tells you which `mmcblkX` is which):

```sh
for d in /sys/block/mmcblk*; do echo "$d -> $(cat $d/device/name 2>/dev/null)"; done
ls -l /dev/disk/by-id/ | grep -i mmc
```

## Results

### Inventory output

```
PASTE INVENTORY OUTPUT HERE
```

### U-Boot environment (if available)

```
PASTE fw_printenv OUTPUT HERE (or write "fw_printenv not available")
```

### Device identification

```
PASTE DEVICE IDENTIFICATION OUTPUT HERE
```

### Observations

| Question                                              | Answer    |
| ----------------------------------------------------- | --------- |
| eMMC device (`mmcblkX`) and model                     | `FILL IN` |
| microSD device (`mmcblkX`) — insert the card to check | `FILL IN` |
| `/dev/mtd0` present (SPI exposed to Linux)?           | `FILL IN` |
| U-Boot environment location (if known)                | `FILL IN` |
| microSD kernel driver (`dw_mmc_rockchip`?)            | `FILL IN` |
| eMMC kernel driver (`sdhci_arasan`?)                  | `FILL IN` |

### Verdict

`FILL IN`
