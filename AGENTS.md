# AGENTS.md

Guidance for AI agents (and humans) working in this repository.

## Project

We are porting **Redox OS (aarch64)** to the **PINE64 Pinebook Pro** (Rockchip RK3399). The deliverable is a **GitHub Actions workflow** that builds a self-contained microSD image: `dd` it to a card, boot the laptop, reach a serial console first, then storage/input/display.

`README.md` is the source of truth for the **research summary, target image layout, and the implementation checklist**. Keep it current.

## Ground rules

- **Language:** all repository content (docs, comments, commit messages, workflow names, log messages) is written in **English**.
- Docs are markdown; keep tables/mermaid valid. The README checklist uses GitHub task syntax (`- [ ]` / `- [x]`).
- Do not commit build artifacts, images, or generated `.config`.
- We do **not fork** upstream Redox repos. CI clones upstream (`gitlab.redox-os.org` primary) and we inject our own files: filesystem config(s), `.config`, scripts, patches (when unavoidable). Upstream changes live in `patches/<repo>/` and **mirror the upstream tree path**: a file upstream does not have is stored **verbatim** (`patches/base/drivers/storage/dwmmcd/src/main.rs`), a change to an existing file is **exactly one `<path>.patch`** per file (`patches/kernel/src/context/signal.rs.patch`), so two changes never share a patch file. `scripts/apply-patches.sh` copies the new files and applies the diffs; every repo directory has a `README.md` with its base commit and the reason for each entry. Details: `patches/README.md`.
- Verify claims against sources before adding "facts" to README/AGENTS. Mark inferences explicitly (`inferred`, `TBD`). Prefer linking evidence (upstream file + symbol).
- **Rust style: primitive numeric conversions use `as`, not `From`.** Write `a as u32`, not `u32::from(a)` — the driver code in `patches/` is dense register arithmetic and the casts read better; do not "fix" existing `as` casts into `From` impls.
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
- **One commit per experiment:** an experiment lands as **one** commit, made when the experiment is finished (code, patches, docs and the experiment log together). Do not leave a chain of intermediate commits for the iterations of the same experiment — amend, squash or rebase local work into that single commit first. Separate experiments are separate commits.
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
- initfs service ordering gotcha: redoxfs (`50_rootfs.service`) scans `/scheme` once with **no retry**. init runs services sequentially and a notify-type service blocks the scheduler until `daemon.ready()` (or exit, logged as "exited without notifying readiness"), but **target/unit order is not a reliable guarantee**: measured in experiment 6 iteration 9, `debugsh` (90_initfs.target) printed its banner before `dwmmcd` (40_drivers.target) logged its first line. In that same boot redoxfs still ran after dwmmcd finished, which is the ordering rootfs needs; consumers that must tolerate either order (debugsh waits up to 3 s for `disk.mmc`) are the safe pattern.
- Build env pitfall: `COOKBOOK_OFFLINE` is parsed as a Rust bool — the value must be `true`; `COOKBOOK_OFFLINE=1` silently means **online** and `repo cook` stalls on `git fetch` to the slow Redox GitLab (base has no GitHub mirror).
- Keyboard + touchpad are internal **USB HID** devices (behind a hub) → need USB host + HID stack, not PS/2/I2C. **Measured (experiment 8):** the installed U-Boot registers only the two **OHCI** halves (`Bus usb@fe3a0000`, `Bus usb@fe3e0000`; no `USB EHCI` line), so the internal hub and all its devices run at full speed on **`usb_host1_ohci` (`fe3e0000`, 4 devices in the U-Boot scan)** — an OHCI driver is sufficient, EHCI split transactions are not needed. Address map from `rk3399.dtsi` (Linux and U-Boot agree): `usb_host0_ehci` `fe380000` + `usb_host0_ohci` `fe3a0000` (2 devices), `usb_host1_ehci` `fe3c0000` + `usb_host1_ohci` `fe3e0000` (4 devices, internal hub). The DT compatible string is `generic-ohci`.
- SPI NOR (16 MiB) ships **empty**; flashing it with U-Boot/TOW-Boot is an alternative to embedding U-Boot in the SD image.
- **U-Boot leaves USB switched off at the EFI handover** (experiment 8): it stops its USB stack before starting `BOOTAA64.EFI`, so by the time Redox runs, the CRU USB gates are closed — `dwmmcd` measured `CLKGATE_CON(6) = 0x00000060`, i.e. `clk_usb2phy0_ref` (bit 5) and `clk_usb2phy1_ref` (bit 6) gated (RK3399 gates are `CLK_GATE_SET_TO_DISABLE`: 1 = off) — and the USB2 PHYs are powered down. Accessing an unclocked RK3399 peripheral **stalls the whole SoC** (no exception, no log; measured: the first hardware run of `ohcid` froze the machine at the first register access). Any USB driver must first un-gate `CLKGATE_CON(6)` bits 5/6 and `CLKGATE_CON(20)` bits 5-8 (`hclk_host0/1` + arbiters, `0x350`) and re-power the PHY through the GRF: `u2phyN` base is `0xe450`/`0xe460` in the GRF, bit 4 of the base enables the 480 MHz output, and bits [1:0] at base + 8 are the host-port suspend field (`2` = running). Registers are hiword write-masked; offsets follow Linux `clk-rk3399.c` and `phy-rockchip-inno-usb2.c`.
- **QEMU cannot use AAVMF for this port** (experiment 9): Debian's `qemu-efi-aarch64` has no `QemuFdtDxe`, so the guest sees ACPI only and the devicetree-only kernel initializes nothing. Boot the packaged U-Boot instead (`apt-get download u-boot-qemu` + `dpkg-deb -x`, no root) with `-M virt,gic-version=3`; `scripts/qemu-smoke.sh` does exactly that and asserts the boot reaches initfs. `gic-version=3` is mandatory (GICv2 panics the kernel). The bootloader menu auto-boots after 3 s, so a headless run needs no input.
- Mainline U-Boot defconfig: `pinebook-pro-rk3399_defconfig`; BL31 from ATF `PLAT=rk3399`; artifacts `idbloader.img` + `u-boot.itb`. TOW-Boot board name: `pine64-pinebookPro`.

### Redox build system

- Primary upstream: `https://gitlab.redox-os.org/redox-os/redox` (GitHub = mirror).
- Key vars: `ARCH` (`aarch64`), `CONFIG_NAME`, `BOARD` (empty for us), `FILESYSTEM_CONFIG` (path, default `config/$ARCH/$BOARD/$CONFIG_NAME.toml`), `FILESYSTEM_SIZE`, `PODMAN_BUILD`, `CI=1`, `REPO_BINARY`, `REPO_OFFLINE`, `COOKBOOK_MAKE_JOBS`.
- Output: `build/$ARCH/$CONFIG_NAME/harddrive.img`.
- Local loop: `scripts/bootstrap-redox.sh` → `scripts/build-image.sh` → `scripts/flash-sd.sh` → `scripts/serial-log.sh`; build tree in `build/redox` (upstream clone, never forked; config overlay injected), image at `build/redox/build/aarch64/minimal/harddrive.img`. Host needs rustup/cargo even in Podman mode (fstools). `REPO_BINARY=1` is not the default (aarch64 binary packages unverified).
- Board config template: `config/aarch64/raspi3bp/minimal.toml` → copy as `config/aarch64/pinebook-pro/minimal.toml`, adjust `include = ["../../minimal.toml"]`, `filesystem_size`, `efi_partition_size`. Desktop: `config/aarch64/pinebook-pro/desktop-minimal.toml` (`include = ["../../desktop-minimal.toml"]` + sizes); `scripts/build-image.sh --config desktop-minimal` / `scripts/flash-sd.sh --config desktop-minimal`, image at `build/redox/build/aarch64/desktop-minimal/harddrive.img`.
- **Recipe sources drift on every online `repo cook`** (experiment 13): cookbook fetches each recipe's branch head before cooking. Measured 2026-11-21: `kernel e7f1d423→f64a8163`, `relibc b83feabc→74b5e7e3`, `uutils-grep 6f048c6→00e7bec` — an unverified newer kernel ships silently, and new crates.io deps break the offline cook (`uutils-grep@00e7bec` needs `uucore ^0.13.0`). Verified baseline: kernel `e7f1d423`, relibc `b83feabc`, uutils-grep `6f048c6`, base `8f576d27` (local clone, does not move); uutils `0.11.0` and uutils-procps `ee98c0a` are pinned checkouts. Build with `COOKBOOK_OFFLINE=true CARGO_NET_OFFLINE=true`; after an accidental online cook reset the moved sources (`git -C <src> reset --hard <baseline>`) and re-run `scripts/apply-patches.sh` (idempotent).
- Cookbook pitfalls: `repo cook` with `--filesystem` runs a TUI that needs a pty ≥ 40x120 (`script -qec "stty rows 40 cols 120; …"`); a failure opens a `[Skip][Inspect][Exit][Retry]` dialog and waits forever, and parallel cooks lock each other — always one cook at a time, `pkill -f "repo cook"` first. Single `repo cook <recipe>` (like `make r.<recipe>`) is plain output. `make image` still cooks any recipe whose source ident changed, so it fails on a drifted source even though it only assembles.
- GitLab vs GitHub: recipe sources clone from GitHub mirrors where they exist; `base.git` and `redox-log.git` have no mirror and are served from the local clone via `~/.gitconfig` `url … insteadOf` (git CLI only — cargo git deps ignore `insteadOf`, rely on `CARGO_NET_OFFLINE=true` + the cached `~/.cargo/git/checkouts`). crates.io works online; the gitlab SSL timeouts otherwise stall cooks.
- QEMU smoke test path: `mk/qemu.mk` aarch64 = `-M virt` + UEFI (AAVMF/edk2) + ramfb; disk default `nvme`.
- **SMP in the QEMU rig** (experiment 14): `-smp 4` needs a DTB with four CPU nodes — regenerate with `-machine dumpdtb=…` under `-smp 4`, then insert the two `generic-ohci` nodes **at the end of the root node** (dtc rejects subnodes before properties; `build/qemu-virt-smp4.dts`). Expect `CPU activation complete: 4 CPUs online` and `BSP: 4 CPUs`.
- Build profiles: every recipe builds with `--release`; the base recipe sets `CARGO_PROFILE_RELEASE_OPT_LEVEL=s` (size) for the initfs drivers, userspace crates keep the default release `-O3`.
- **CPU frequency (measured 2026-11-21, experiment 14):** U-Boot leaves APLL at 600 MHz and **divides the little cluster by 2 → 300 MHz** (`CLKSEL_CON1 = 0x501`, divider field `[4:0] = 1`); the big cluster is undivided (`CLKSEL_CON0 = 0x100`, BPLL 800 MHz). Redox has no cpufreq driver, so nothing ramps. `ohcid` now logs the CRU PLL/CLKSEL registers and raises the little divider to /1 (600 MHz) with a hiword-masked write — no PLL relock, no voltage change. Anything above that (816 MHz–1.2 GHz) needs APLL/BPLL retuning **and** the RK808 core voltage over I2C.
- Podman build is the "always works" mode; native build requires the Ubuntu package list from `native_bootstrap.sh`.

### Redox aarch64 boot chain (why this port is feasible)

- Bootloader (UEFI, `BOOTAA64.EFI`): drops EL2→EL1, builds page tables, reads EFI memory map, **passes DTB to kernel via env `DTB_ADDR`/`DTB_SIZE`**, framebuffer via `FRAMEBUFFER_ADDR/WIDTH/HEIGHT/STRIDE/BPP`, RedoxFS via `REDOXFS_UUID`/`REDOXFS_BLOCK`.
- **The handover needs the bootloader to touch UART2 registers** (experiment 12): four read-only loads at `0xff1a0004/0x0c/0x10/0x14` right before `kernel_entry` (`dump_uart2()`) decide between a booting kernel and silence before its first instruction. U-Boot's own LSR reads, pure delay and console writes do not substitute; the marker-era builds worked because they wrote `MCR=0x03`. Mechanism unknown (README risk 15).
- **The firmware framebuffer is not necessarily 32bpp:** U-Boot v2021.07 `rk_vop.c` sets `VIDEO_BPP16` for `VOP_MODE_EDP`, and its GOP reports `PixelBitMask` with masks `0xf800/0x07e0/0x001f` (RGB565); `gop_set_mode` accepts only mode 0, so the depth cannot be changed from UEFI. Our bootloader derives `FRAMEBUFFER_BPP` from `Mode.Info.PixelFormat` and the kernel `graphical_debug` plus `vesad` honour it (experiment 7). Writing 32-bit ARGB `0xFF000000` into an RGB565 surface produces black/yellow pixel pairs — that was the "yellow screen".
- Kernel serial driver is DTB-driven and supports `snps,dw-apb-uart` (16550 with u32 MMIO) → RK3399 UART2 works out of the box.
- IRQ: RK3399 is **GICv3** (`arm,gic-v3`, GIC-500) — not GICv2/GIC-400. `gicv3.rs` exists; **SMP works since experiment 14** (hardware-verified: 6 CPUs online; QEMU rig: `-smp 4`): per-CPU ICC setup + redistributor, SGIs enabled in `GICR_ISENABLER0`, SGI targeting by **MPIDR via `ICC_SGI1R_EL1`** (target list for one CPU, `IRM = 1` for all-but-self; MPIDR affinities: aff0 bits 0-7 → target-list bit, aff1 at bits 23:16, aff2 at 39:32, aff3 at 55:48), IPIs addressed by `ArchPercpuMisc.mpidr`. Two traps: the redistributor **stride is the fixed 128 KiB RD+SGI pair** (capacity = region length / 0x20000 — `GICD_TYPER.CPUNumber` is not authoritative on GICv3, QEMU reports 1 with 4 CPUs), and `ICC_IAR1_EL1` must be masked with **0x3ff** (10-bit INTID; the old `0x1ff` folded INTID 511 into 511 and panicked on the spurious 1023). Timer: ARM generic timer. PSCI 1.1 (SMC conduit) works. **GICv3 private interrupts need the redistributor, not the distributor:** SGIs (0–16) and PPIs (16–32, including the timer PPI 27) are banked per CPU and enabled through `GICR_ISENABLER0` in the redistributor's SGI frame (`RD_base + 0x10000`, one 0x20000 frame per CPU back to back, frames woken via `GICR_WAKER`), while `GICD_ISENABLER` covers only SPIs (32+). Writing GICD for a PPI id is a RES0 write that fails silently (experiment 11).
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
  (`05e3:0608`) → an **OHCI** USB2 host is required for input (measured in
  experiment 8; DWC2 and xHCI are not involved). The `ohcid` CRU/PHY bring-up is
  **hardware-verified (23:07 boot)**: no SoC freeze, both controllers operational.
  Enumeration protocol bugs fixed against the U-Boot 2021.07 driver (control-ED
  direction must be 0, class-descriptor bmRequestType 0xA0, HCCA write via
  `virt_addr()`, per-transfer buffer arena reset) and verified in the QEMU
  `pci-ohci` rig. **`dc cvac` translates its operand** (experiment 8, iteration 6):
  passing a DMA _physical_ address to cache maintenance faults at EL0 on real
  hardware ("Lacks grant", driver killed) and is **invisible in QEMU** (TCG does
  no translation for CM-by-VA) — always clean through the Dma virtual mapping.
  **Iteration 7 (2026-11-21): the keyboard works on hardware** — `258a:001e` sits directly on `fe3a0000` (low-speed), enumerates, boot protocol, reports into the VT. First-keystroke loss fixed (SET_IDLE 0 = report-on-change: the first report after arming is the first key press, not an idle snapshot — no first-report swallowing). The Genesys hub on `fe3e0000` (touchpad) wakes up with a retry: the first `GET_DESCRIPTOR` times out while the hub firmware boots (150 ms settle is enough, iteration 9) — `05e3:0608`, 4 ports, MPS0 64. Its ports must be **powered before polling connections** (the hub switches power per port; U-Boot's `usb_hub_power_on` order) — fix pending hardware confirmation. **Iteration 15 (root cause):** the touchpad is not the hub device — it is **interface 1 (endpoint `0x82`) of the same keyboard chip** (Tow-Boot issue #57): an unread report there wedges the whole chip. Fix: `find_hid_endpoints` returns **every** HID interrupt IN endpoint and each non-keyboard endpoint is polled (read+drop) — committed in `30fd030`, hardware-verified. **Iteration 16:** each extra endpoint is armed from its **HID report descriptor** (`parse_report_descriptor`) — report id, button byte/bits, X/Y offsets, relative vs absolute; **the Pinebook Pro chip stalls every descriptor request shape** (`0x81`, `0x80`, `0xa1`), so for `258a:001e` the driver uses the **measured layout** `[report id 1, buttons, dx, dy]` (signed 8-bit relative; reconstructed from 130 raw reports) — the chip ACKs `SET_PROTOCOL(boot)` but keeps its own report protocol, which misparsed the boot layout (dx/dy swapped, buttons in dx, cursor stuck at the left edge). Boot-protocol probe + 3-byte layout stays as fallback, then `HidKind::Drain`. **DMA arena split (same iteration):** interrupt endpoint buffers are fixed per-ED slots (`Ohci::endpoint_buf`) and control transfers use their own region (`alloc_buf`) — they used to share one bump arena that every control transfer reset to 0, so a control SETUP could land in a live interrupt buffer and a report could DMA over it, which made the 15 s liveness probe read a healthy device as dead and reset the port every ~50 s. The revive path replaces **every** root-port `HidDevice` of the controller. Diagnostics: descriptor request results, `ohcid: raw ep 0x82 len N: …` (first 40 reports), `liveness probe of addr N failed: …`. Desktop + keyboard hardware-verified.
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
  hardware (STATUS 0x6106 → 0x0106). Iterations 6–7 hardware runs also proved
  `CLKSEL_CON16` is write-mask (hiword) protected — plain writes are dropped —
  and exposed that kernel `nanosleep` did not wake on the generic timer (a 10 ms
  sleep blocked until a serial interrupt; `dwmmcd` was left busy-waiting). **The
  timer itself was broken (experiment 11):** the architected timer PPI (27)
  never fired, because `GicV3::parse()` reported the _GICD_ chunk as the
  redistributor (`gicrs: [(FEE00000, 10000)]`) and the redistributor was never
  programmed — in GICv3 SGIs/PPIs are banked per CPU and enabled through
  `GICR_ISENABLER0`, while `GICD_ISENABLER` only covers SPIs, so `irq_enable(27)`
  wrote a RES0 bit and silently enabled nothing. Fixed in
  `patches/kernel/src/arch/aarch64/device/irqchip/gicv3.rs.patch`; verified in QEMU
  (`sleep(10ms)` → 10.9 ms, `sleep(250ms)` → 250.2 ms). **Storage works
  end-to-end (iteration 8):** the microSD enumerates (`card ready: size=30436
MiB`, block 0 `55aa`), redoxfs mounts the root from `disk.mmc/0p2`, init
  switchroots to `/usr`, the rootfs userspace starts and the `redox login:`
  banner appears. `debugsh` now exits at startup when `disk.mmc` is registered,
  releasing the serial console to the login. **Rootfs userspace verified
  (experiment 12):** `ls /bin`, `ls /usr/` and `ls /usr/bin/` print complete
  listings, `dwmmcd` reports `card ready` at `[   0.115]` and the log timestamps
  advance (timer). Secondary CPUs were offline until experiment 14: the GICv3
  `init_ap` gap is fixed and **hardware-verified — 6 of 6 CPUs online**
  (4 in the QEMU rig); RK808 RTC/entropy drivers missing; temporary marker/dump
  instrumentation removed, clean image rebuilt. `exit` from the rootfs shell logs
  `procmgr: failed to send SIGCHLD to parent PID ProcessId(28)` (harmless, open).
  **Deliverable image hardware-verified (2026-11-20 23:07,
  `logs/serial-20261120-230717.log`):** the committed bootloader (µs menu + cache
  clean + RGB565 + `dump_uart2`) + clean kernel boots to `redox login:`; the open
  front is USB enumeration (experiment 8, iteration 2).
- **Dynamic userspace binaries (experiment 10):** the loader maps the executable at
  **0x1000** and `libc.so` at **0xa80000**, so absolute userspace PCs in an exception dump
  must be rebased before symbolising (identify the base with a register whose meaning you
  can name, e.g. `X19` = `main` in `relibc_start_v1`; both recorded crash PCs then landed
  on `brk` instructions in `libc.so`). Two defects killed every uutils applet:
  thread-atexit registration and finalization shared a `RefCell` (`DTORS`), so a
  destructor or signal handler registering a handler while the stack was being
  drained panicked with `RefCell already borrowed` (`brk #1` at exit, buffered
  stdout lost) — the fix keeps the stack as an intrusive list behind a `Cell`, so
  registration can never observe a drain; and `signal_handler()` runs from
  `sched_yield()`/the timer tick, i.e. **before** the arch syscall epilogue stores
  the result, so a signal delivered mid-syscall saved the syscall _argument_ as the
  return value (relibc's `Sys::verify()` yield resumed with `x0 = -1` and took its
  abort branch). Fixes: `patches/relibc/src/cxa.rs.patch`,
  `patches/kernel/src/context/signal.rs.patch`. QEMU cannot exercise this
  path: no root filesystem and the initfs holds only static binaries.

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
scripts/             # local loop: bootstrap-redox.sh, build-image.sh, flash-sd.sh, serial-log.sh,
                     # qemu-smoke.sh, apply-patches.sh; later: repack-pbp.sh
config/              # filesystem config overlay for the redox build (Phase 2+)
patches/             # upstream additions/fixes, upstream tree preserved:
                     #   <repo>/<upstream path>       new file, copied verbatim
                     #   <repo>/<upstream path>.patch change to an existing file
                     # see patches/README.md and scripts/apply-patches.sh
```

## Workflow conventions

1. Before starting a task, read the checklist in `README.md` and pick/confirm the relevant item; after finishing, **update the checklist state**.
2. Every non-trivial change to the workflow must be verifiable in CI (build must stay green). Workflow-level validation for docs/scripts without CI: `yaml` lint for `.github/workflows/*`, `shellcheck` for `scripts/*`.
3. When researching, prefer primary sources (upstream code) over blog posts; record findings in README instead of leaving them in chat history only.
4. Bump/annotate risky items in "Risks & open questions" (README §7) instead of silently dropping them.

## Local development notes

- Primary development happens through GitHub Actions on Linux runners; local Linux builds are supported via `scripts/bootstrap-redox.sh` + `scripts/build-image.sh` (see README §4 "Local build, flash and serial loop"); macOS cannot run the Redox build natively (Linux-only; Podman machine may work but is not the supported path).
- Hardware-facing milestones (Phase 3+) need a physical Pinebook Pro + UART cable; CI can only validate up to image composition + QEMU smoke tests.
- `make image` only **assembles** the image from packages that are already built; it does not rebuild a recipe whose sources changed. After editing a checkout run `make r.<recipe> image` (or `make all`), otherwise the image can ship a stale binary with no trace in the sources (experiment 12: a leftover-instrumentation kernel produced a silent hang). `scripts/build-image.sh` runs `make all`, so the documented loop is safe.

## Quick links

- Redox build docs: https://doc.redox-os.org/book/ (chapters: `building-redox`, `configuration-settings`, `aarch64`, `boot-process`)
- Kernel aarch64: https://github.com/redox-os/kernel/tree/master/src/arch/aarch64
- Bootloader UEFI aarch64: https://github.com/redox-os/bootloader/blob/master/src/os/uefi/arch/aarch64.rs
- PINE64 wiki PBP: https://wiki.pine64.org/wiki/Pinebook_Pro
- Tow-Boot: https://github.com/Tow-Boot/Tow-Boot (boards/`pine64-pinebookPro`)
- U-Boot: https://source.denx.de/u-boot/u-boot
