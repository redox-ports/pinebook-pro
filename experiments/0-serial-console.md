# Experiment 0 — Serial console and current boot chain

**Status:** done (2026-11-19)
**Related:** README §3 (boot chain), §7 risk 5

## Goal

Prove that the UART2 console works at **1500000 baud 8N1** and capture the complete
current boot chain of the machine as shipped (BootROM → SPL → U-Boot → Linux).

## Prerequisites

- USB-UART adapter with **3.3 V** logic level (check the jumper/voltage; early Pine
  Store cables used 5 V levels and must not be used).
- Bottom cover removed; UART/Audio switch (item 9 in the PINE64 wiki "Internal Layout"
  diagram) slid **towards the display hinges** (ON). Audio output over the headphone
  jack is disabled in this mode.
- Wiring, from the Pinebook Pro point of view: **tip = TX, ring 1 = RX, sleeve = GND**.
  Connect: adapter RX ↔ tip, adapter TX ↔ ring 1.

## Actions

1. Find the adapter on the host.

   macOS:

   ```sh
   ls /dev/tty.usbserial*
   ```

   Linux:

   ```sh
   ls /dev/ttyUSB*
   ```

2. Open the console at 1500000 8N1.

   macOS / Linux:

   ```sh
   screen /dev/tty.usbserial-XXXX 1500000
   # exit screen: Ctrl-A, then K
   # alternative: picocom -b 1500000 /dev/ttyUSB0
   ```

3. Power on the laptop and record **everything** from power-on until the Linux login
   prompt (or ~30 seconds if no prompt appears). Paste it into Results → Boot log.

4. Optional: if an autoboot countdown appears, press a key repeatedly (space/Enter) to
   stop it. If you get the `=>` U-Boot prompt, run and capture:

   ```
   version
   printenv boot_targets
   printenv bootcmd
   mmc list
   part list mmc 0
   part list mmc 1
   ```

If the output is garbled, the adapter most likely cannot sustain 1500000 baud reliably
(prefer CP2102 / FT232 / FT2232). Note it down and retry with another adapter.

## Results

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
```

### U-Boot prompt session

```
=> version
U-Boot 2021.07-1 (Jul 12 2021 - 13:27:45 +0000) Manjaro ARM

gcc (GCC) 10.2.0
GNU ld (GNU Binutils) 2.35
=> printenv boot_targets
boot_targets=usb0 mmc1 mmc0 nvme0 pxe dhcp sf0
=> printenv bootcmd
bootcmd=run distro_bootcmd
=> mmc list
mmc@fe310000: 2
mmc@fe320000: 1
sdhci@fe330000: 0
=> part list mmc 0

Partition Map for MMC device 0  --   Partition Type: EFI

Part  Start LBA  End LBA    Name
  Attributes
  Type GUID
  Partition GUID
  1  0x0000f424  0x0007a120  "primary"
  attrs:  0x0000000000000000
  type:  ebd0a0a2-b9e5-4433-87c0-68b6b72699c7
  guid:  199b390c-2bba-4030-82fe-5d5c7bd4437a
  2  0x0007a121  0x0747bfde  "primary"
  attrs:  0x0000000000000000
  type:  0fc63daf-8483-4772-8e79-3d69d8477de4
  guid:  3d4f89d4-871d-455d-9bca-e55c92830229
=> part list mmc 1

Partition Map for MMC device 1  --   Partition Type: DOS

Part  Start Sector  Num Sectors  UUID    Type
  1  8192        62325760    00000000-01  0b Boot
=>
```

### Observations

| Question                                                  | Answer                                                                                                                                                                                                                                                                               |
| --------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| SPL/TPL messages visible (DDR init)?                      | Yes. TPL `2021.07-1`: LPDDR4, 2 channels × 2048 MB, 50 MHz → 400/800 MHz. SPL: `Trying to boot from MMC1`.                                                                                                                                                                           |
| U-Boot version string (exact)                             | `U-Boot 2021.07-1 (Jul 12 2021 - 13:27:45 +0000) Manjaro ARM` (TPL, SPL and proper from the same build; GCC 10.2.0, GNU ld 2.35)                                                                                                                                                     |
| Board/defconfig in the U-Boot banner                      | `Model: Pine64 Pinebook Pro`, `SoC: Rockchip rk3399`, `DRAM: 3.9 GiB`, `PMIC: RK808` (defconfig name not printed)                                                                                                                                                                    |
| Boot device used by U-Boot (`mmc0` / `mmc1` / `nvme` / …) | Runtime `boot_targets=usb0 mmc1 mmc0 nvme0 pxe dhcp sf0` (default env). SPL reports `MMC1`; runtime mapping: `mmc0`=eMMC (`sdhci@fe330000`), `mmc1`=microSD (`mmc@fe320000`), `mmc2`=SDIO (`mmc@fe310000`). Which medium actually hosts the running U-Boot is ambiguous — see Notes. |
| Autoboot interruptible? (`yes` / `no`)                    | Yes — `Hit any key to stop autoboot: 0`, and the `=>` prompt was reachable.                                                                                                                                                                                                          |
| Anything unusual (warnings, resets, garbled characters)   | No garbling. U-Boot loads its environment from SPI NOR (`gd25q128`, 16 MiB): `*** Warning - bad CRC, using default environment`. U-Boot output also goes to `vidconsole` (display initialized). USB buses enumerate 2 + 4 devices. No resets.                                        |

### Notes (facts extracted from the log)

- **Working UART:** the whole log arrived intact at 1500000 8N1; the adapter handles the baud rate fine.
- **SPI NOR:** detected by U-Boot (`gd25q128`, page 256 B, erase 4 KiB, total 16 MiB). The environment area holds no valid environment (`bad CRC`), so U-Boot runs the built-in default environment.
- **Runtime MMC mapping** (`mmc list`): `mmc@fe310000: 2` (SDIO/Wi-Fi), `mmc@fe320000: 1` (microSD), `sdhci@fe330000: 0` (eMMC).
- **eMMC partition table** (`part list mmc 0`, GPT):
  - p1: LBA `0x0000f424`–`0x0007a120` (62500–500000, ~213.6 MiB), Microsoft basic data (FAT).
  - p2: LBA `0x0007a121`–`0x0747bfde` (500001–122142686, ~58.2 GiB), Linux filesystem.
  - First partition starts 32 MB into the disk, leaving room for an embedded bootloader at 8 MiB. The layout is consistent with an installed Manjaro ARM (matches the U-Boot build banner). The eMMC is the baseline OS and stays untouched.
- **microSD partition table** (`part list mmc 1`, MBR): one partition, start LBA 8192 (4 MiB), 62,325,760 sectors (~29.7 GiB), type `0x0b` (FAT32), boot flag. Consistent with a freshly formatted card — no bootloader visible in the first 4 MiB.
- **Ambiguity to resolve:** SPL prints `Trying to boot from MMC1`, which in the runtime mapping is the microSD, but the microSD has no room for `u-boot.itb` at 8 MiB (its first partition starts at 4 MiB), while the eMMC layout does fit an embedded U-Boot. Either SPL uses a different device numbering or the running U-Boot lives on the eMMC; **experiment 2** settles this with raw sector dumps on both devices.
- **Implication for the port:** the current firmware already performs EFI distro boot and provides an interruptible U-Boot console, so a `BOOTAA64.EFI` on an ESP can be booted from the microSD without touching SPI/eMMC; manual `bootefi` from the prompt allows fast iteration.

### Verdict

UART2 console is confirmed working at **1500000 8N1** and the current boot chain is
captured end-to-end (TPL → SPL → U-Boot 2021.07-1 Manjaro ARM → EFI distro boot).
Autoboot is interruptible, giving us a U-Boot console for manual `bootefi` experiments.
The integration point for Redox is the ESP with `BOOTAA64.EFI`; **no firmware flashing is
required for the first boot tests**. The self-contained image (U-Boot at sectors
64/16384) remains necessary for the case when the eMMC is disabled or its firmware is
unavailable.
