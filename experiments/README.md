# Hardware Experiments

Chronological log of hands-on experiments on the Pinebook Pro. Each experiment is a
separate file named `<number>-<slug>.md`, numbered in execution order.

- **History is append-only.** Never delete or renumber an experiment that has results.
  To repeat an experiment, add a new file with the next number.
- Every experiment has an **Actions** part (commands/steps) and a **Results** part filled
  by the device owner (logs, outputs, observations, verdict).
- Fill every `FILL IN` / `PASTE ... HERE` placeholder after running the steps.
- The index table below is authoritative for ordering; file names are never renamed.

## Rules

- **Read-only unless the experiment explicitly says otherwise.** Never write to the eMMC
  or SPI during reconnaissance (`dd if=...` only, never `of=`).
- **UART must be 3.3 V.** Never connect a 5 V USB-UART adapter.
- The eMMC installation stays untouched; all Redox tests happen from microSD.
- Device names (`mmcblk0` / `mmcblk1`) can swap between boots — always identify devices
  first (experiment 1) and substitute the right name in later commands.

## Device information

| Field                                                         | Value                                                     |
| ------------------------------------------------------------- | --------------------------------------------------------- |
| Pinebook Pro batch / version (v2.1, ANSI/ISO, eMMC size, RAM) | `FILL IN`                                                 |
| OS currently on eMMC (name + version)                         | Manjaro ARM (kernel 5.13.4-1-MANJARO-ARM; hostname `pbp`) |
| Kernel version (`uname -r`)                                   | `5.13.4-1-MANJARO-ARM`                                    |
| microSD card model / capacity                                 | `SU32G`, 29.7 GiB (SDHC)                                  |
| USB-UART adapter model (chip: CH340 / CP2102 / FT232 / other) | `FILL IN`                                                 |
| Host OS used for the serial console (macOS / Linux)           | `FILL IN`                                                 |

## Index

| #   | File                                                   | Goal                                                                         | Status                                                                                                                               |
| --- | ------------------------------------------------------ | ---------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| 0   | [0-serial-console.md](0-serial-console.md)             | Verify the UART console at 1500000 8N1 and capture the current boot chain    | **done** (2026-11-19)                                                                                                                |
| 1   | [1-system-inventory.md](1-system-inventory.md)         | Inventory OS, kernel drivers, block devices, SPI exposure                    | **done** (2026-11-19)                                                                                                                |
| 2   | [2-raw-sectors.md](2-raw-sectors.md)                   | Verify idbloader/U-Boot sector offsets and SPI blankness                     | **done** (2026-11-19)                                                                                                                |
| 3   | [3-emmc-boot-partitions.md](3-emmc-boot-partitions.md) | Locate the first stage: user area LBA 64 vs eMMC boot0/boot1; read EXT_CSD   | **done** (2026-11-19) — idbloader at user-area LBA 64                                                                                |
| 4   | [4-first-redox-boot.md](4-first-redox-boot.md)         | Boot the local Redox image from microSD via the eMMC U-Boot, capture UART    | **in progress** (2026-11-20) — kernel + initfs userspace + 1920x1080 display work; root mount blocked on storage driver              |
| 5   | [5-debug-console.md](5-debug-console.md)               | Interim serial console: `debugsh` initfs daemon over the kernel debug scheme | **done** (2026-11-20) — interactive console on UART2 verified; root causes: NS16550 rx, GICv3 SPI delivery (`ARE_S` wipe, `IROUTER`) |
| 6   | [6-dwmmc-storage.md](6-dwmmc-storage.md)               | DW MMC microSD driver (`dwmmcd`) so redoxfs can mount the root filesystem    | **in progress** (2026-11-20) — driver written and built into the image; first hardware boot pending                                             |
