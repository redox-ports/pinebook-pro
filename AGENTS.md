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
- Use `!` after the type/scope (or a `BREAKING CHANGE:` footer) when a change breaks the workflow or image format.
- Body (optional): explain _why_, wrap at ~72 columns.

Examples:

```
docs: record RK3399 boot offsets and repack layout
ci: build aarch64 harddrive image on ubuntu-24.04
feat(scripts): repack ESP to sector 32768 and embed u-boot
fix(ci): prune runner disk before redox build
```

## Verified facts cheat sheet (do not re-derive)

### Pinebook Pro / RK3399

- Boot ROM loads next stage at **byte 32768 (sector 64)**; U-Boot proper goes at **sector 16384 (8 MiB)**. Boot order: SPI NOR → eMMC → microSD → USB OTG. NVMe (PCIe) is **not** ROM-bootable.
- Serial console = **UART2, 1500000 baud 8N1, 3.3 V**, via headphone jack (internal switch). Compatible string in DT: `snps,dw-apb-uart` / `rockchip,rk3399-uart`.
- Storage controllers: microSD = **DW MMC**; eMMC = **SDHCI (arasan)** @ `fe330000`; both lack upstream Redox drivers (biggest gap).
- Keyboard + touchpad are internal **USB HID** devices (behind a hub) → need USB host + HID stack, not PS/2/I2C.
- SPI NOR (16 MiB) ships **empty**; flashing it with U-Boot/TOW-Boot is an alternative to embedding U-Boot in the SD image.
- Mainline U-Boot defconfig: `pinebook-pro-rk3399_defconfig`; BL31 from ATF `PLAT=rk3399`; artifacts `idbloader.img` + `u-boot.itb`. TOW-Boot board name: `pine64-pinebookPro`.

### Redox build system

- Primary upstream: `https://gitlab.redox-os.org/redox-os/redox` (GitHub = mirror).
- Key vars: `ARCH` (`aarch64`), `CONFIG_NAME`, `BOARD` (empty for us), `FILESYSTEM_CONFIG` (path, default `config/$ARCH/$BOARD/$CONFIG_NAME.toml`), `FILESYSTEM_SIZE`, `PODMAN_BUILD`, `CI=1`, `REPO_BINARY`, `REPO_OFFLINE`, `COOKBOOK_MAKE_JOBS`.
- Output: `build/$ARCH/$CONFIG_NAME/harddrive.img`.
- Board config template: `config/aarch64/raspi3bp/minimal.toml` → copy as `config/aarch64/pinebook-pro/minimal.toml`, adjust `include = ["../../minimal.toml"]`, `filesystem_size`, `efi_partition_size`.
- QEMU smoke test path: `mk/qemu.mk` aarch64 = `-M virt` + UEFI (AAVMF/edk2) + ramfb; disk default `nvme`.
- Podman build is the "always works" mode; native build requires the Ubuntu package list from `native_bootstrap.sh`.

### Redox aarch64 boot chain (why this port is feasible)

- Bootloader (UEFI, `BOOTAA64.EFI`): drops EL2→EL1, builds page tables, reads EFI memory map, **passes DTB to kernel via env `DTB_ADDR`/`DTB_SIZE`**, framebuffer via `FRAMEBUFFER_ADDR/WIDTH/HEIGHT/STRIDE`, RedoxFS via `REDOXFS_UUID`/`REDOXFS_BLOCK`.
- Kernel serial driver is DTB-driven and supports `snps,dw-apb-uart` (16550 with u32 MMIO) → RK3399 UART2 works out of the box.
- IRQ: `gic.rs` = GICv2 (RK3399 GIC-400 compatible). Timer: ARM generic timer. PSCI supported.
- Precedent: Raspberry Pi 3 B+ (ARM64, U-Boot) boots to UART serial console; storage drivers are the missing piece everywhere on aarch64.

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
.github/workflows/   # build.yml (Phase 1+)
scripts/             # repack-pbp.sh, qemu-smoke.sh, deps install helpers (Phase 1+)
config/              # filesystem config overlay for the redox build (Phase 2+)
patches/             # upstream patches, only if unavoidable
```

## Workflow conventions

1. Before starting a task, read the checklist in `README.md` and pick/confirm the relevant item; after finishing, **update the checklist state**.
2. Every non-trivial change to the workflow must be verifiable in CI (build must stay green). Workflow-level validation for docs/scripts without CI: `yaml` lint for `.github/workflows/*`, `shellcheck` for `scripts/*`.
3. When researching, prefer primary sources (upstream code) over blog posts; record findings in README instead of leaving them in chat history only.
4. Bump/annotate risky items in "Risks & open questions" (README §7) instead of silently dropping them.

## Local development notes

- Primary development happens through GitHub Actions on Linux runners; local macOS cannot run the Redox build natively (Linux-only; Podman machine may work but is not the supported path).
- Hardware-facing milestones (Phase 3+) need a physical Pinebook Pro + UART cable; CI can only validate up to image composition + QEMU smoke tests.

## Quick links

- Redox build docs: https://doc.redox-os.org/book/ (chapters: `building-redox`, `configuration-settings`, `aarch64`, `boot-process`)
- Kernel aarch64: https://github.com/redox-os/kernel/tree/master/src/arch/aarch64
- Bootloader UEFI aarch64: https://github.com/redox-os/bootloader/blob/master/src/os/uefi/arch/aarch64.rs
- PINE64 wiki PBP: https://wiki.pine64.org/wiki/Pinebook_Pro
- Tow-Boot: https://github.com/Tow-Boot/Tow-Boot (boards/`pine64-pinebookPro`)
- U-Boot: https://source.denx.de/u-boot/u-boot
