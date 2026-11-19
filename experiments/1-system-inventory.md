# Experiment 1 — System inventory

**Status:** done (2026-11-19)
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

_(Russian tool output translated to English for repository consistency.)_

### Inventory output

```
Welcome to Manjaro-ARM
~~Website: https://manjaro.org
~~Forum:   https://forum.manjaro.org/c/arm
~~Matrix:  #manjaro-arm-public:matrix.org
Last login: Sat Mar 29 11:06:33 on ttyS2
[user@pbp ~]$ uname -a
cat /proc/device-tree/model; echo
cat /proc/device-tree/compatible; echo
lsblk -o NAME,SIZE,TYPE,MODEL,MOUNTPOINT
sudo fdisk -l
cat /proc/cmdline
ls /dev/mtd* 2>/dev/null; cat /proc/mtd 2>/dev/null
dmesg | grep -Ei 'mmc|sdhci|dw_mmc|spi-nor|rockchip' | head -40
lsusb
which fw_printenv flashrom mtd_debug
Linux pbp 5.13.4-1-MANJARO-ARM #1 SMP Tue Jul 20 18:40:45 UTC 2021 aarch64 GNU/Linux
Pine64 Pinebook Pro
pine64,pinebook-prorockchip,rk3399
NAME           SIZE TYPE MODEL MOUNTPOINT
mmcblk1       29.7G disk
└─mmcblk1p1   29.7G part
mmcblk2       58.2G disk
├─mmcblk2p1  213.6M part       /boot
└─mmcblk2p2     58G part       /
mmcblk2boot0     4M disk
mmcblk2boot1     4M disk
zram0          5.7G disk       [SWAP]
[sudo] password for user:
Disk /dev/mmcblk1: 29.72 GiB, 31914983424 bytes, 62333952 sectors
Units: sectors of 1 * 512 = 512 bytes
Sector size (logical/physical): 512 bytes / 512 bytes
I/O size (minimum/optimal): 512 bytes / 512 bytes
Disklabel type: dos
Disk identifier: 0x00000000

Device         Boot  Start      End  Sectors  Size Id Type
/dev/mmcblk1p1 *      8192 62333951 62325760 29.7G  b W95 FAT32


Disk /dev/mmcblk2: 58.24 GiB, 62537072640 bytes, 122142720 sectors
Units: sectors of 1 * 512 = 512 bytes
Sector size (logical/physical): 512 bytes / 512 bytes
I/O size (minimum/optimal): 512 bytes / 512 bytes
Disklabel type: gpt
Disk identifier: 4E429A30-0B30-4B95-88F8-22F04FF81DED

Device          Start       End   Sectors  Size Type
/dev/mmcblk2p1  62500    500000    437501 213.6M Microsoft basic data
/dev/mmcblk2p2 500001 122142686 121642686    58G Linux filesystem


Disk /dev/zram0: 5.66 GiB, 6074843136 bytes, 1483116 sectors
Units: sectors of 1 * 4096 = 4096 bytes
Sector size (logical/physical): 4096 bytes / 4096 bytes
I/O size (minimum/optimal): 4096 bytes / 4096 bytes
initrd=/initramfs-linux.img console=tty3 console=ttyS2,1500000 root=PARTUUID=3d4f89d4-871d-455d-9bca-e55c92830229 rw rootwait quiet splash plymouth.ignore-serial-consoles
/dev/mtd0  /dev/mtd0ro
dev:    size   erasesize  name
mtd0: 01000000 00001000 "spi0.0"
[    0.104431] platform ff770000.syscon:phy@f780: Fixing up cyclic dependency with fe330000.mmc
[    1.561757] spi-nor spi0.0: gd25q128 (16384 Kbytes)
[    1.761188] sdhci: Secure Digital Host Controller Interface driver
[    1.761195] sdhci: Copyright(c) Pierre Ossman
[    1.762302] dwmmc_rockchip fe320000.mmc: IDMAC supports 32-bit address mode.
[    1.762322] dwmmc_rockchip fe320000.mmc: Using internal DMA controller.
[    1.762334] dwmmc_rockchip fe320000.mmc: Version ID is 270a
[    1.762364] dwmmc_rockchip fe320000.mmc: DW MMC controller at irq 32,32 bit host data width,256 deep fifo
[    1.762413] sdhci-pltfm: SDHCI platform and OF driver helper
[    1.762459] dwmmc_rockchip fe310000.mmc: IDMAC supports 32-bit address mode.
[    1.762484] dwmmc_rockchip fe310000.mmc: Using internal DMA controller.
[    1.762499] dwmmc_rockchip fe310000.mmc: Version ID is 270a
[    1.762548] dwmmc_rockchip fe310000.mmc: DW MMC controller at irq 31,32 bit host data width,256 deep fifo
[    1.762578] dwmmc_rockchip fe320000.mmc: Got CD GPIO
[    1.762854] dwmmc_rockchip fe310000.mmc: allocated mmc-pwrseq
[    1.762863] mmc_host mmc0: card is non-removable.
[    1.764326] mmc2: CQHCI version 5.10
[    1.775352] mmc_host mmc1: Bus speed (slot 0) = 400000Hz (slot req 400000Hz, actual 400000HZ div = 0)
[    1.787912] mmc2: SDHCI controller on fe330000.mmc [fe330000.mmc] using ADMA
[    1.826248] rockchip-pcie f8000000.pcie: host bridge /pcie@f8000000 ranges:
[    1.826286] rockchip-pcie f8000000.pcie:      MEM 0x00fa000000..0x00fbdfffff -> 0x00fa000000
[    1.826300] rockchip-pcie f8000000.pcie:       IO 0x00fbe00000..0x00fbefffff -> 0x00fbe00000

[    1.826746] rockchip-pcie f8000000.pcie: no vpcie12v regulator found
[    1.857494] mmc_host mmc1: Bus speed (slot 0) = 50000000Hz (slot req 50000000Hz, actual 50000000HZ div = 0)
[    1.857568] mmc1: new high speed SDHC card at address e624
[    1.858460] mmcblk1: mmc1:e624 SU32G 29.7 GiB
[    1.863524]  mmcblk1: p1
[    1.863953] mmc2: Command Queue Engine enabled
[    1.863976] mmc2: new HS200 MMC card at address 0001
[    1.864703] mmcblk2: mmc2:0001 DA4064 58.2 GiB
[    1.864859] mmcblk2boot0: mmc2:0001 DA4064 4.00 MiB
[    1.865006] mmcblk2boot1: mmc2:0001 DA4064 4.00 MiB
[    1.865144] mmcblk2rpmb: mmc2:0001 DA4064 16.0 MiB, chardev (235:0)
[    1.868599]  mmcblk2: p1 p2
[    1.888289] mmc_host mmc0: Bus speed (slot 0) = 400000Hz (slot req 400000Hz, actual 400000HZ div = 0)
[    1.934598] mmc0: queuing unknown CIS tuple 0x80 (2 bytes)
[    1.936147] mmc0: queuing unknown CIS tuple 0x80 (3 bytes)
[    1.937684] mmc0: queuing unknown CIS tuple 0x80 (3 bytes)
[    1.940762] mmc0: queuing unknown CIS tuple 0x80 (7 bytes)
[    1.944262] mmc0: queuing unknown CIS tuple 0x81 (9 bytes)
Bus 004 Device 001: ID 1d6b:0003 Linux Foundation 3.0 root hub
Bus 003 Device 001: ID 1d6b:0002 Linux Foundation 2.0 root hub
Bus 006 Device 001: ID 1d6b:0003 Linux Foundation 3.0 root hub
Bus 005 Device 001: ID 1d6b:0002 Linux Foundation 2.0 root hub
Bus 002 Device 001: ID 1d6b:0001 Linux Foundation 1.1 root hub
Bus 001 Device 004: ID 0c45:6321 Microdia HP Integrated Webcam
Bus 001 Device 003: ID 046d:c52f Logitech, Inc. Unifying Receiver
Bus 001 Device 002: ID 05e3:0608 Genesys Logic, Inc. Hub
Bus 001 Device 001: ID 1d6b:0002 Linux Foundation 2.0 root hub
Bus 008 Device 002: ID 258a:001e HAILUCK CO.,LTD USB KEYBOARD
Bus 008 Device 001: ID 1d6b:0001 Linux Foundation 1.1 root hub
Bus 007 Device 001: ID 1d6b:0002 Linux Foundation 2.0 root hub
which: no fw_printenv in (/usr/local/sbin:/usr/local/bin:/usr/bin:/usr/bin/site_perl:/usr/bin/vendor_perl:/usr/bin/core_perl)
which: no flashrom in (/usr/local/sbin:/usr/local/bin:/usr/bin:/usr/bin/site_perl:/usr/bin/vendor_perl:/usr/bin/core_perl)
which: no mtd_debug in (/usr/local/sbin:/usr/local/bin:/usr/bin:/usr/bin/site_perl:/usr/bin/vendor_perl:/usr/bin/core_perl)
[user@pbp ~]$
```

### U-Boot environment (if available)

```
sudo: fw_printenv: command not found
```

### Device identification

```
/sys/block/mmcblk1 -> SU32G
/sys/block/mmcblk2 -> DA4064
/sys/block/mmcblk2boot0 ->
/sys/block/mmcblk2boot1 ->
lrwxrwxrwx 1 root root 13 Nov 19 22:21 mmc-DA4064_0x61371c19 -> ../../mmcblk2
lrwxrwxrwx 1 root root 15 Nov 19 22:21 mmc-DA4064_0x61371c19-part1 -> ../../mmcblk2p1
lrwxrwxrwx 1 root root 15 Nov 19 22:21 mmc-DA4064_0x61371c19-part2 -> ../../mmcblk2p2
lrwxrwxrwx 1 root root 13 Nov 19 22:21 mmc-SU32G_0x19194da0 -> ../../mmcblk1
lrwxrwxrwx 1 root root 15 Nov 19 22:21 mmc-SU32G_0x19194da0-part1 -> ../../mmcblk1p1
```

### Observations

| Question                                              | Answer                                                                                                                                           |
| ----------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| eMMC device (`mmcblkX`) and model                     | `mmcblk2` — `DA4064`, 58.2 GiB (HS200, CQHCI 5.10, ADMA; boot0/boot1 4 MiB each, RPMB 16 MiB); GPT: p1 FAT `/boot` 213.6 MiB, p2 ext4 `/` 58 GiB |
| microSD device (`mmcblkX`) — insert the card to check | `mmcblk1` — `SU32G`, 29.7 GiB (SDHC, high speed); MBR, one FAT32 partition at LBA 8192, unmounted                                                |
| `/dev/mtd0` present (SPI exposed to Linux)?           | Yes — `/dev/mtd0` and `/dev/mtd0ro`; `mtd0: 01000000 00001000 "spi0.0"` (16 MiB); `spi-nor` reports `gd25q128`                                   |
| U-Boot environment location (if known)                | Not determined; `fw_printenv` is not installed. U-Boot (experiment 0) loads env from SPI and reports `bad CRC` → no valid saved environment      |
| microSD kernel driver (`dw_mmc_rockchip`?)            | Yes — `dwmmc_rockchip` on `fe320000.mmc` (Linux `mmc1`, IRQ 32)                                                                                  |
| eMMC kernel driver (`sdhci_arasan`?)                  | Yes — `sdhci` / `sdhci-pltfm` on `fe330000.mmc` (Linux `mmc2`, ADMA, HS200)                                                                      |

### Notes (facts extracted from the log)

- **Baseline OS:** Manjaro-ARM, kernel `5.13.4-1-MANJARO-ARM` (built 2021-07-20), hostname
  `pbp`, user `user`. Device tree: model `Pine64 Pinebook Pro`, compatible
  `pine64,pinebook-pro` + `rockchip,rk3399`.
- **The eMMC is the boot OS:** the kernel root `PARTUUID=3d4f89d4-871d-455d-9bca-e55c92830229`
  equals the GPT partition GUID of `mmcblk2p2`, which is also the p2 GUID shown by
  U-Boot's `part list mmc 0` in experiment 0.
- **U-Boot and Linux number the MMC controllers differently:**
  - U-Boot: `mmc0` = eMMC (`sdhci@fe330000`), `mmc1` = microSD (`mmc@fe320000`),
    `mmc2` = SDIO/Wi-Fi (`mmc@fe310000`);
  - Linux: `mmc0` = SDIO/Wi-Fi (`fe310000`, non-removable, `mmc-pwrseq`),
    `mmc1` = microSD (`fe320000`), `mmc2` = eMMC (`fe330000`).

  Always identify controllers by their MMIO address, never by bare index.

- **Serial login verified:** the whole session ran over `ttyS2`; the kernel cmdline
  contains `console=tty3 console=ttyS2,1500000`, confirming the UART path end-to-end.
- **SPI NOR is readable from Linux** as `/dev/mtd0` (16 MiB), so the flash can be dumped
  and inspected (and backed up) before any write.
- **USB topology:** internal keyboard is `258a:001e` (HAILUCK USB KEYBOARD) on a USB 2.0
  bus; webcam `0c45:6321` (Microdia); internal hub `05e3:0608` (Genesys Logic). All USB
  root hubs are present (USB 1.1/2.0/3.0).
- **PCIe:** `rockchip-pcie f8000000.pcie` initializes; "no vpcie12v regulator found" is
  expected on the Pinebook Pro.
- **Missing tools:** `fw_printenv`, `flashrom`, `mtd_debug` are not installed
  (`uboot-tools` / `mtd-utils` absent).
- The `for` loop output in the original paste was interleaved with the next command;
  the block above is the corrected form (verified by the `/dev/disk/by-id` listing).

### Verdict

The shipped baseline is Manjaro ARM (kernel 5.13.4) on the eMMC, and the eMMC is confirmed
as the boot medium (root PARTUUID matches the U-Boot-visible GPT GUID). Both storage
controllers work under Linux Rockchip drivers that we can use as references:
`dwmmc_rockchip` for the microSD (`fe320000`) and `sdhci` for the eMMC (`fe330000`). The
SPI NOR is fully exposed as `/dev/mtd0` (16 MiB, `gd25q128`), which enables backup and
inspection before any flashing. U-Boot and Linux use different MMC ordering, so all
scripts and docs must key off controller addresses. The serial console path is verified
for both U-Boot and Linux (`console=ttyS2,1500000`).
