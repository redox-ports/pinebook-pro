# AGENTS.md

Guidance for AI agents (and humans) working in this repository.

## Project

We are porting **Redox OS (aarch64)** to the **PINE64 Pinebook Pro** (Rockchip RK3399). The deliverable is a **GitHub Actions workflow** that builds a self-contained microSD image: `dd` it to a card, boot the laptop, reach a serial console first, then storage/input/display.

`README.md` is the source of truth for the **research summary, target image layout, and the implementation checklist**. Keep it current.

## Ground rules

- **Language:** all repository content (docs, comments, commit messages, workflow names, log messages) is written in **English**.
- Docs are markdown; keep tables/mermaid valid. The README checklist uses GitHub task syntax (`- [ ]` / `- [x]`).
- Do not commit build artifacts, images, or generated `.config`.
- We do **not fork** upstream Redox repos. CI clones upstream (`gitlab.redox-os.org` primary) and we inject our own files: filesystem config(s), `.config`, scripts, patches (when unavoidable). If an upstream patch becomes necessary, store it under `patches/` with a header explaining target repo + commit + reason, and apply it in CI.
- Verify claims against sources before adding "facts" to README/AGENTS. Mark inferences explicitly (`inferred`, `TBD`). Prefer linking evidence (upstream file + symbol).
- **Commits:** follow [Conventional Commits](https://en.wikipedia.org/wiki/Conventional_Commits) — see "Commit style" below.

## Commit style

Every commit message must follow [Conventional Commits](https://en.wikipedia.org/wiki/Conventional_Commits):

```
<type>(<scope>): <summary>
```

- **Types:** `feat`, `fix`, `docs`, `ci`, `build`, `refactor`, `test`, `chore`, `revert`.
- **Scopes used in this repo:** `ci` (GitHub Actions workflow), `scripts`, `config` (filesystem config overlay), `docs`.
- **Summary:** English, imperative mood ("add", not "added"), no trailing period, keep the first line ≤ 72 characters.
- **One line only:** the commit message must be a single line — no body, no footers. Extra context belongs in the PR/MR description or docs.
- Use `!` after the type/scope when a change breaks the workflow or image format.

Examples:

```
docs: record RK3399 boot offsets and repack layout
ci: build aarch64 harddrive image on ubuntu-24.04
feat(scripts): repack ESP to sector 32768 and embed u-boot
fix(ci): prune runner disk before redox build
```

## Verified facts cheat sheet (do not re-derive)

### Pinebook Pro / RK3399

- Boot ROM loads next stage at **byte 32768 (sector 64)**; U-Boot proper goes at **sector 16384 (8 MiB)**. The on-disk RK3399 header is the **v1** format (512 bytes, RC4-encoded, plaintext magic `0x0ff0aa55`); the payload starts at `+0x800` with `RK33`. ASCII `RKNS` is the **v2** magic (RK3568/RK3588 class), **not** RK3399. Boot order: SPI NOR → eMMC → microSD → USB OTG. NVMe (PCIe) is **not** ROM-bootable.
- Serial console = **UART2, 1500000 baud 8N1, 3.3 V**, via headphone jack (internal switch). Compatible string in DT: `snps,dw-apb-uart` / `rockchip,rk3399-uart`.
- Storage controllers: microSD = **DW MMC** (`fe320000`, compatible `rockchip,rk3399-dw-mshc` + `rk3288-dw-mshc`, IRQ GIC_SPI 65, fifo-depth 0x100, max-freq 150 MHz per `rk3399.dtsi`); eMMC = **SDHCI (arasan)** @ `fe330000`. Our `dwmmcd` driver covers microSD; eMMC stays untouched.
- DW MMC register map (verified from Linux `dw_mmc.h` + U-Boot v2021.07 `dwmmc.h`): `CTRL 0x000, PWREN 0x004, CLKDIV 0x008, CLKSRC 0x00c, CLKENA 0x010, TMOUT 0x014, CTYPE 0x018, BLKSIZ 0x01c, BYTCNT 0x020, INTMASK 0x024, CMDARG 0x028, CMD 0x02c, RESP0-3 0x030-0x03c, MINTSTS 0x040, RINTSTS 0x044 (W1C), STATUS 0x048 (FIFO_EMPTY bit2, FIFO_FULL bit3, BUSY bit9, FCNT bits 17-29), FIFOTH 0x04c, VERID 0x06c, HCON 0x070`; FIFO data register at **0x200** for IP >= 2.40a (U-Boot hardcodes 0x200 for RK3399). `RINTSTS`: CD bit0, RE 1, CDONE 2, DTO 3, TXDR 4, RXDR 5, RCRC 6, DCRC 7, RTO 8, DRTO 9, HTO 10, FRUN 11, HLE 12, SBE 13, ACD 14, EBE 15.
- initfs service ordering gotcha: redoxfs (`50_rootfs.service`) scans `/scheme` once with **no retry**; notify-type services block init until `daemon.ready()`, so a driver service in `40_drivers.target` is guaranteed registered before redoxfs starts. A driver that exits before ready is logged ("exited without notifying readiness") and boot continues.
- Build env pitfall: `COOKBOOK_OFFLINE` is parsed as a Rust bool — the value must be `true`; `COOKBOOK_OFFLINE=1` silently means **online** and `repo cook` stalls on `git fetch` to the slow Redox GitLab (base has no GitHub mirror).
- Keyboard + touchpad are internal **USB HID** devices (behind a hub) → need USB host + HID stack, not PS/2/I2C.
- SPI NOR (16 MiB) ships **empty**; flashing it with U-Boot/TOW-Boot is an alternative to embedding U-Boot in the SD image.
- Mainline U-Boot defconfig: `pinebook-pro-rk3399_defconfig`; BL31 from ATF `PLAT=rk3399`; artifacts `idbloader.img` + `u-boot.itb`. TOW-Boot board name: `pine64-pinebookPro`.

### Redox build system

- Primary upstream: `https://gitlab.redox-os.org/redox-os/redox` (GitHub = mirror).
- Key vars: `ARCH` (`aarch64`), `CONFIG_NAME`, `BOARD` (empty for us), `FILESYSTEM_CONFIG` (path, default `config/$ARCH/$BOARD/$CONFIG_NAME.toml`), `FILESYSTEM_SIZE`, `PODMAN_BUILD`, `CI=1`, `REPO_BINARY`, `REPO_OFFLINE`, `COOKBOOK_MAKE_JOBS`.
- Output: `build/$ARCH/$CONFIG_NAME/harddrive.img`.
- Local loop: `scripts/bootstrap-redox.sh` → `scripts/build-image.sh` → `scripts/flash-sd.sh` → `scripts/serial-log.sh`; build tree in `build/redox` (upstream clone, never forked; config overlay injected), image at `build/redox/build/aarch64/minimal/harddrive.img`. Host needs rustup/cargo even in Podman mode (fstools). `REPO_BINARY=1` is not the default (aarch64 binary packages unverified).
- Board config template: `config/aarch64/raspi3bp/minimal.toml` → copy as `config/aarch64/pinebook-pro/minimal.toml`, adjust `include = ["../../minimal.toml"]`, `filesystem_size`, `efi_partition_size`.
- QEMU smoke test path: `mk/qemu.mk` aarch64 = `-M virt` + UEFI (AAVMF/edk2) + ramfb; disk default `nvme`.
- Podman build is the "always works" mode; native build requires the Ubuntu package list from `native_bootstrap.sh`.

### Redox aarch64 boot chain (why this port is feasible)

- Bootloader (UEFI, `BOOTAA64.EFI`): drops EL2→EL1, builds page tables, reads EFI memory map, **passes DTB to kernel via env `DTB_ADDR`/`DTB_SIZE`**, framebuffer via `FRAMEBUFFER_ADDR/WIDTH/HEIGHT/STRIDE`, RedoxFS via `REDOXFS_UUID`/`REDOXFS_BLOCK`.
- Kernel serial driver is DTB-driven and supports `snps,dw-apb-uart` (16550 with u32 MMIO) → RK3399 UART2 works out of the box.
- IRQ: RK3399 is **GICv3** (`arm,gic-v3`, GIC-500) — not GICv2/GIC-400. `gicv3.rs` exists; `irqchip::init_ap` (secondary CPUs) currently supports only the GICv2 memory-mapped interface, so expect single-CPU until extended. Timer: ARM generic timer. PSCI 1.1 (SMC conduit) works.
- Precedent: Raspberry Pi 3 B+ (ARM64, U-Boot) boots to UART serial console; storage drivers are the missing piece everywhere on aarch64.

### Device state (verified 2026-11-19)

- UART2 console verified at **1500000 8N1** (see `experiments/0-serial-console.md`);
  serial login works (`console=ttyS2,1500000` in the kernel cmdline).
- Board runs U-Boot **2021.07-1 (Manjaro ARM)** with EFI distro boot; default env,
  `boot_targets=usb0 mmc1 mmc0 nvme0 pxe dhcp sf0`; autoboot interruptible.
- **MMC numbering differs:** U-Boot `mmc0`=eMMC (`sdhci@fe330000`), `mmc1`=microSD
  (`mmc@fe320000`), `mmc2`=SDIO/Wi-Fi (`mmc@fe310000`); Linux `mmc0`=SDIO,
  `mmc1`=microSD, `mmc2`=eMMC. Identify controllers by MMIO address, never by index.
- Baseline OS: Manjaro ARM, kernel `5.13.4-1-MANJARO-ARM`; eMMC = `DA4064` 58.2 GiB
  (`mmcblk2`), microSD = `SU32G` 29.7 GiB (`mmcblk1`). eMMC is the boot medium
  (root PARTUUID matches the U-Boot-visible GPT GUID) — **never modify it**.
- Linux reference drivers: `dwmmc_rockchip` (microSD `fe320000`, SDIO `fe310000`) and
  `sdhci`/`sdhci-pltfm` (eMMC `fe330000`, ADMA, HS200).
- SPI NOR `gd25q128` (16 MiB) is exposed as `/dev/mtd0` (`"spi0.0"`) and is
  **blank/erased**: `0xFF` at sector 64 (no v1 header possible) and no `U-Boot` strings
  in 16 MiB (backup + SHA256 recorded). Flashing TOW-Boot to SPI remains an option.
- eMMC boot chain (standard RK3399 layout, experiments 2–3): **idbloader at user-area
  sector 64** (RC4-encoded v1 header; payload marker `RK33` at LBA 68) and **U-Boot proper
  at sector 16384**. The hardware boot partitions (`boot0`/`boot1`, 4 MiB each) contain no
  bootloader and are unused. **Never write to any part of the eMMC** (it holds the baseline OS).
- Internal keyboard/touchpad are USB2 HID (`258a:001e`) behind a Genesys hub
  (`05e3:0608`) → USB2 host (DWC2) is required for input, not DWC3/xHCI.
- **Redox bring-up status (2026-11-20, experiments 4–6):** the kernel boots to initfs
  userspace — GICv3, generic timer (virq 27), serial (virq 132), `init`, drivers,
  `vesad` 1920x1080 + `fbbootlogd` (framebuffer works, kernel `graphical_debug` visible on
  the panel). **Interactive root console on UART2 works (experiment 5):** `debugsh`
  initfs daemon (echo, line editing, ls/cat/echo>/hexdump/env/exec) verified on
  hardware. Kernel fixes kept under `patches/`: NS16550 rx path (loopback self-test
  left IER=0 + 14-byte FIFO threshold swallowed keystrokes), GICv3 SPI delivery
  (`GICD_IGROUPR`/`IROUTER` never programmed; `GICD_CTLR` overwrite wiped the
  firmware `ARE_S` bit — SPIs pended forever), `pcid` gated to x86 (its aarch64 PCI
  fallback panics; `hwd` spawned it directly). **DW MMC microSD driver written and
  built (experiment 6):** `dwmmcd` initfs daemon (FIFO/PIO polling, U-Boot v2021.07
  rk3399 clocking model — CRU `CLKSEL_CON16` rates, controller `CLKDIV=0` bypass,
  clock updates awaited on the CMD START bit per `dwmci_setup_bus`, no-CIU fallback,
  `disk.mmc` scheme, ordered before `50_rootfs.service`). Hardware: five boot
  iterations characterized a clock-update timeout that iteration 6 traced to the
  wait mechanism (the driver polled RINTSTS CDONE; U-Boot polls CMD START and this
  IP never raises CDONE for clock-only commands); SRST_SDMMC pulse verified on
  hardware (STATUS 0x6106 → 0x0106). Enumeration test pending. Secondary CPUs stay
  offline (GICv3
  `init_ap` gap); RK808 RTC/entropy drivers missing; temporary marker/dump
  instrumentation removed, clean image rebuilt.

### Target SD image layout (produced by our repack)

| Sector range     | Content                                                                                                                     |
| ---------------- | --------------------------------------------------------------------------------------------------------------------------- |
| 64 … 16383       | `idbloader.img`                                                                                                             |
| 16384 … (~18783) | `u-boot.itb`                                                                                                                |
| 32768 … 264191   | ESP FAT32 `/EFI/BOOT/BOOTAA64.EFI` (ESP recreated here; stock installer puts ESP at sector 2048 which collides with U-Boot) |
| 264192 … end     | RedoxFS (untouched by repack)                                                                                               |

The repack (`scripts/repack-pbp.sh`, to be written) uses only `sgdisk`, `mkfs.vfat`, `mtools` (`mcopy`/`mtype`) — no root, no loop mounts, CI-safe.

## Expected repo layout

```
README.md            # research + plan + checklist (source of truth)
AGENTS.md            # this file
LICENSE
.gitignore
experiments/         # chronological hardware experiment logs (append-only)
.github/workflows/   # build.yml (Phase 1+)
scripts/             # local loop: bootstrap-redox.sh, build-image.sh, flash-sd.sh, serial-log.sh; later: repack-pbp.sh, qemu-smoke.sh
config/              # filesystem config overlay for the redox build (Phase 2+)
patches/             # upstream patches, only if unavoidable
```

## Workflow conventions

1. Before starting a task, read the checklist in `README.md` and pick/confirm the relevant item; after finishing, **update the checklist state**.
2. Every non-trivial change to the workflow must be verifiable in CI (build must stay green). Workflow-level validation for docs/scripts without CI: `yaml` lint for `.github/workflows/*`, `shellcheck` for `scripts/*`.
3. When researching, prefer primary sources (upstream code) over blog posts; record findings in README instead of leaving them in chat history only.
4. Bump/annotate risky items in "Risks & open questions" (README §7) instead of silently dropping them.

## Local development notes

- Primary development happens through GitHub Actions on Linux runners; local Linux builds are supported via `scripts/bootstrap-redox.sh` + `scripts/build-image.sh` (see README §4 "Local build, flash and serial loop"); macOS cannot run the Redox build natively (Linux-only; Podman machine may work but is not the supported path).
- Hardware-facing milestones (Phase 3+) need a physical Pinebook Pro + UART cable; CI can only validate up to image composition + QEMU smoke tests.

## Quick links

- Redox build docs: https://doc.redox-os.org/book/ (chapters: `building-redox`, `configuration-settings`, `aarch64`, `boot-process`)
- Kernel aarch64: https://github.com/redox-os/kernel/tree/master/src/arch/aarch64
- Bootloader UEFI aarch64: https://github.com/redox-os/bootloader/blob/master/src/os/uefi/arch/aarch64.rs
- PINE64 wiki PBP: https://wiki.pine64.org/wiki/Pinebook_Pro
- Tow-Boot: https://github.com/Tow-Boot/Tow-Boot (boards/`pine64-pinebookPro`)
- U-Boot: https://source.denx.de/u-boot/u-boot
