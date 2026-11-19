# Experiment 2 — Raw sector verification

**Status:** done (2026-11-19) — follow-up in experiment 3
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

### Linux: idbloader hexdump (sector 64)

Expected: the first bytes contain `RKNS`.

eMMC (`/dev/mmcblk2`) — **non-zero data, no `RKNS` magic**:

```
00000000  3b 8c dc fc be 9f 9d 51  eb 30 34 ce 24 51 1f 98  |;......Q.04.$Q..|
00000010  ff 0c f2 36 05 50 c8 bb  3f ec dd bd 06 85 fa b7  |...6.P..?.......|
00000020  b3 ab 6a ea c8 68 e0 08  ad 9d 6f 9c 3c 98 b0 8c  |..j..h....o.<...|
00000030  45 13 54 1c 1d 1b 1f 15  a7 f1 f0 0b e3 4e 0c c7  |E.T..........N..|
00000040  60 96 01 6a b5 f0 e2 c1  50 c6 24 9e 12 f7 58 8e  |`..j....P.$...X.|
00000050  40 b9 b7 be 8b fa 25 dd  74 d7 6f 59 46 7e 13 41  |@.....%.t.oYF~.A|
00000060  ee fd f5 91 39 bc 74 95  25 3c 1a e1 f1 57 30 05  |....9.t.%<...W0.|
00000070  ca f8 72 9a 1b e6 9d 26  35 5d 81 2b 2b 93 bd 01  |..r....&5].++...|
00000080  3a 54 0c a1 4b 11 06 98  a1 91 19 4a 4e 92 30 1a  |:T..K......JN.0.|
00000090  f2 b2 d5 ae 59 6c 9e 96  fd f4 ff a4 88 e3 9f 87  |....Yl..........|
000000a0  49 6c 3a 76 6d 3d 1a ac  1e 77 0a 5e ff 92 52 61  |Il:vm=...w.^..Ra|
000000b0  19 ff 74 96 ee 13 93 7a  e0 b9 f0 1b 51 38 b3 8d  |..t....z....Q8..|
000000c0  2f 59 87 02 65 c1 88 6b  4c 21 aa 7b 16 d5 50 ce  |/Y..e..kL!.{..P.|
000000d0  37 80 a4 1f 46 df bd b0  d9 65 8f e1 15 ce 08 0f  |7...F....e......|
000000e0  7c ee 5a 0c 61 50 3c 90  bf 79 1a bf 05 96 b0 61  ||.Z.aP<..y.....a|
000000f0  eb ed 62 53 4b 3a ee a4  aa 77 95 df e9 e7 44 fb  |..bSK:...w....D.|
00000100  2d f8 ba 7e b2 1e e0 04  ea ea d2 d7 61 f9 90 92  |-..~........a...|
00000110  88 e3 07 b6 5a 8f 9b 4e  4f 9e f9 c8 38 d9 11 5d  |....Z..NO...8..]|
00000120  41 37 f4 dd 5b 78 47 95  8a ac f5 42 5d aa 0c 52  |A7..[xG....B]..R|
00000130  49 c8 0d 9d a7 32 2e d7  03 b1 41 95 49 50 2d 89  |I....2....A.IP-.|
```

microSD (`/dev/mmcblk1`) — **all zeros**:

```
00000000  00 00 00 00 00 00 00 00  00 00 00 00 00 00 00 00  |................|
*
00004000
```

### Linux: U-Boot strings (sector 16384)

eMMC (`/dev/mmcblk2`) — multiple `U-Boot` matches:

```
U-Boot
U-Boot
U-Boot
U-Boot
U-Boot
U-Boot
```

microSD (`/dev/mmcblk1`) — nothing found.

Note: `grep -ao` prints only the matches (no version string); the exact U-Boot version
is known from experiment 0: `U-Boot 2021.07-1 (Jul 12 2021) Manjaro ARM`.

### U-Boot prompt: `mmc info` (from the `=>` prompt)

```
=> mmc dev 0; mmc info
switch to partitions #0, OK
mmc0(part 0) is current device
Device: sdhci@fe330000
Manufacturer ID: 45
OEM: 100
Name: DA406
Bus Speed: 25000000
Mode: MMC High Speed (52MHz)
Rd Block Len: 512
MMC version 5.1
High Capacity: Yes
Capacity: 58.2 GiB
Bus Width: 8-bit
Erase Group Size: 512 KiB
HC WP Group Size: 8 MiB
User Capacity: 58.2 GiB WRREL
Boot Capacity: 4 MiB ENH
RPMB Capacity: 16 MiB ENH
Boot area 0 is not write protected
Boot area 1 is not write protected
=> mmc dev 1; mmc info
switch to partitions #0, OK
mmc1 is current device
Device: mmc@fe320000
Manufacturer ID: 3
OEM: 5344
Name: SU32G
Bus Speed: 20000000
Mode: SD High Speed (50MHz)
Rd Block Len: 512
SD version 3.0
High Capacity: Yes
Capacity: 29.7 GiB
Bus Width: 4-bit
Erase Group Size: 512 Bytes
=>
```

### Linux: SPI NOR

Sector 64 — all `0xFF` (erased):

```
00000000  ff ff ff ff ff ff ff ff  ff ff ff ff ff ff ff ff  |................|
*
00004000
```

`grep -abo 'RKNS' /dev/mtd0` — no matches.

`grep -aboi 'u-boot' /dev/mtd0` — no matches.

Full 16 MiB backup taken to a regular file:

```
dffab0dd410657cb30c7b2fd7f2586a4792e8472e58882b3532581f8111a646d  /home/user/spi-nor-backup.bin
```

### Observations

| Question                                            | Answer                                                                                                                                                                                    |
| --------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `RKNS` magic at sector 64 (eMMC / microSD / SPI)?   | **No on all three.** eMMC: non-zero data without `RKNS`; microSD: all zeros; SPI: all `0xFF`                                                                                              |
| U-Boot version at sector 16384 (eMMC / microSD)?    | eMMC: yes (`U-Boot` strings; version from experiment 0: `U-Boot 2021.07-1 Manjaro ARM`). microSD: nothing                                                                                 |
| U-Boot strings anywhere in the SPI NOR?             | No                                                                                                                                                                                        |
| Which medium hosts the running U-Boot?              | **eMMC** — `u-boot.itb` is in the user area at 8 MiB; SPI is blank and microSD is empty. The first stage is not at user-area sector 64 → follow-up in experiment 3 (eMMC boot partitions) |
| SPI NOR: blank / env only / full bootloader / other | **Blank (erased)**: `0xFF` at sector 64, no `RKNS`/`U-Boot` signatures in the whole 16 MiB; backup SHA256 recorded                                                                        |

### Notes (facts extracted from the run)

- **U-Boot `mmc info`:** eMMC (`sdhci@fe330000`) is MMC 5.1, High Speed (52 MHz), 8-bit,
  58.2 GiB, boot areas 4 MiB each (both **not write-protected**) and RPMB 16 MiB;
  microSD (`mmc@fe320000`) is SD 3.0, High Speed (50 MHz), 4-bit, 29.7 GiB.
- **The first stage is not in the eMMC user area at sector 64.** The user area there
  holds arbitrary non-zero data (no boot header). Since the `U-Boot` proper image is
  present at user-area 8 MiB, the BootROM/SPL first stage must come from an eMMC
  hardware boot partition (boot0/boot1) — experiment 3 checks this.
- **microSD is clean:** sector 64 is zeroed and there is no U-Boot at 8 MiB, so the
  card contains no bootloader remnants and can be overwritten by our image.
- **SPI NOR is blank:** this matches the factory state described by the PINE64 wiki.
  Flashing TOW-Boot/U-Boot to SPI therefore remains a viable alternative deployment
  path, and nothing is lost by overwriting the SD.
- **Our target SD layout is unaffected:** idbloader at sector 64 and `u-boot.itb` at
  16384 is the standard RK3399 BootROM contract for _our_ image; the factory eMMC was
  simply installed differently.
- The SPI backup is a regular file (`~/spi-nor-backup.bin`). Optional byte-level
  blankness check: `tr -d '\377' < ~/spi-nor-backup.bin | wc -c` should print `0`.

### Verdict

SPI NOR is blank (factory state), the microSD contains no bootloader, and U-Boot proper
lives on the eMMC user area at 8 MiB. The idbloader is **not** at user-area sector 64, so
the shipped system must boot its first stage from an eMMC hardware boot partition
(confirmed next in experiment 3). This does not change the deliverable: the
self-contained SD image will place idbloader at sector 64 and `u-boot.itb` at 16384, and
the BootROM will boot it whenever the eMMC is disabled, absent, or unbootable.
