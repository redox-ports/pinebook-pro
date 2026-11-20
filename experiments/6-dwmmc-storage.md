# Experiment 6 — DW MMC microSD storage driver (dwmmcd)

**Date:** 2026-11-20 (autonomous work session)
**Goal:** give the initfs a driver for the RK3399 microSD controller
(`sdmmc@fe320000`, Synopsys DesignWare Mobile Storage Host Controller) so
redoxfs can mount the root filesystem from the microSD card, `init` can
`switch_root`, and the system becomes a full OS instead of stopping at the
interim debug console.

**Related:** experiment 5 (interim console; this closes the gap it
documented), README §6 Phase 4; `patches/base-pinebook-pro.patch`,
`scripts/build-image.sh`, `scripts/flash-sd.sh`, `scripts/serial-log.sh`.

## Design (verified against primary sources)

- Register map: Linux `drivers/mmc/host/dw_mmc.h` and U-Boot **v2021.07**
  `include/dwmmc.h` + `drivers/mmc/dw_mmc.c` — v2021.07 is exactly the
  U-Boot installed on this Pinebook Pro, so its `fifo_mode` PIO code path is
  the reference model (polling, no IDMAC, no controller interrupts).
- Node selection: `fdt.all_compatible(["rockchip,rk3399-dw-mshc",
  "rockchip,rk3288-dw-mshc"])` filtered by first `reg` entry
  **0xfe320000**. The SDIO controller (`fe310000`) shares the compatible
  string, and the eMMC (`fe330000`, SDHCI) holds the baseline OS — the
  address filter guarantees neither is ever probed.
- FIFO data register: offset 0x200 for IP >= 2.40a (U-Boot hardcodes 0x200
  for every supported board including RK3399); selected from `VERID` at
  runtime like Linux does.
- `fifo-depth` from the devicetree (`<0x100>` per `rk3399.dtsi`), fallback
  256 words.
- Clock strategy ("inherited state", Phase A): U-Boot leaves the CRU
  `cclk_sdmmc` rate and CIU dividers in the state that just read the ESP and
  the kernel from this card. The driver never touches the CRU or `CLKSRC`;
  it reprograms `CLKDIV` through the CIU clock-update command
  (stop clock -> write divider -> restart, `CMD_UPD_CLK`) to `64` for the
  100-400 kHz identification window (inherited card clock is SD high speed,
  <= 50 MHz, so cclk/128 lands in range), and restores the inherited divider
  before selecting the card.
- Card enumeration: CMD0 (with `CMD_INIT` 80-clock preamble) -> CMD8 ->
  ACMD41 loop (2 s budget, 10 ms sleep between attempts) -> CMD2 -> CMD3
  (RCA) -> CMD9 (CSD, capacity via Linux `UNSTUFF_BITS` for CSD v1/v2) ->
  CMD7 -> CMD16 (512-byte blocks) -> ACMD6 + `CTYPE` 4-bit.
- Data path: FIFO reset before each data command, `BLKSIZ`/`BYTCNT`,
  CMD17/18/24/25 in chunks of at most 8 blocks, PIO loop driven from
  `STATUS` FIFO-empty/full (not RXDR/TXDR interrupts), DTO for completion,
  CMD12 after multi-block. Byte-addressed (SD v1.x, CCS=0) cards get byte
  offsets in data commands.
- Timeouts: kernel monotonic clock (`clock_gettime(CLOCK_MONOTONIC)` via
  `redox_syscall`), 1 s per command, 100 ms + 100 ms/block for data, 2 s
  ACMD41 budget — a dead card cannot hang the boot.
- Scheme: `disk.mmc` via `driver_block::DiskScheme` (the name redoxfs
  scans for), `TrivialExecutor` event loop like bcm2835-sdhcid.
- Ordering: `40_dwmmcd.service` (`type = "notify"`, aarch64 only) hangs off
  `40_drivers.target`, which `50_rootfs.service` requires weakly. init
  **blocks** on notify-type services until `daemon.ready()`
  (`DiskScheme::new` calls it), so redoxfs scans `/scheme` only after
  `disk.mmc` is registered (redoxfs does not retry).
- Bring-up logging: every driver message goes to stderr (log scheme) AND
  `/scheme/debug` (kernel debug scheme -> UART), so the whole enumeration is
  visible in the serial capture. The fd is opened before `setrens`.
- Self-test at startup: read block 0 and log the MBR signature (should be
  `55aa` — the image is GPT with a protective MBR).

## What was built

- `drivers/storage/dwmmcd/` — new static initfs crate in the `base` recipe
  (main.rs, dwmmc.rs, log.rs).
- `init.initfs.d/40_dwmmcd.service`, wired into `40_drivers.target`.
- debugsh polish (same patch): cwd set to `/scheme/initfs` so bare `ls`
  works, `ls /` maps to `/scheme` (the root does not resolve before
  `switch_root`).
- Patch regenerated as `patches/base-pinebook-pro.patch` (supersedes
  `base-initfs-debug-console.patch`, same base commit `8f576d27`).
- Image rebuilt: `CI=1 COOKBOOK_OFFLINE=true make r.base image`
  (note: the value must be `true`, not `1` — cookbook parses it as a Rust
  bool and `1` silently means online, which stalls on the slow GitLab
  fetch). `build/redox/build/aarch64/minimal/harddrive.img`
  (2026-11-20 08:01) verified by FUSE-mounting the image and comparing
  `/usr/lib/boot/initfs` against the recipe's `initfs.img` — identical md5.

## Test procedure (next hardware session)

1. `scripts/flash-sd.sh` (writes the new `harddrive.img` to the microSD).
2. `scripts/serial-log.sh` (capture to `logs/serial-*.log`).
3. Boot and watch for these milestones, in order:

```text
dwmmcd: dw-mshc node mmc@fe320000 at 0xfe320000
dwmmcd: mapped 0xfe320000 (len 0x...) to 0x...
dwmmcd: inherited verid=... hcon=... ctrl=...
dwmmcd: inherited clkdiv=... clksrc=... clkena=... ctype=... fifoth=...
dwmmcd: fifo depth 256 words, data reg at +0x200
dwmmcd: CMD8 ok (v2.0 card, 0x1aa)
dwmmcd: ACMD41 ready, ocr=0xc0ff8000 ccs=true
dwmmcd: card ready: rca=0x.... size=... MiB (block addressed)
dwmmcd: block 0 signature 55aa
[INFO redoxfs] mounted filesystem ...      <-- success
init: switchroot to /usr /etc              <-- root filesystem!
```

4. If mount works: rootfs units start (getty etc.), `ion` shell on the
   serial console — record everything.
5. If it fails: capture the exact `dwmmcd:` line where it stopped.
   `debugsh` now has a `reboot` builtin (writes `reset` to the kernel
   `sys:kstop` scheme -> PSCI SYSTEM_RESET), so the board can be re-tested
   without power-cycling; the debugsh prompt also supports bare `ls` (cwd
   is `/scheme/initfs`) and `ls /` (lists `/scheme`).

## Expected failure modes and next actions

| Symptom (last dwmmcd line)                    | Meaning                                                        | Next action                                                                    |
| --------------------------------------------- | -------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| `init failed: Connection timed out` (no CMD*) | iteration 1: clock update stalled on the IDMAC-state controller | fixed for iteration 2: full controller reset first (see above)                 |
| `clock update (stop/div/start) failed`        | CIU still not accepting updates after the reset                | capture the logged `rint`/`status`, compare with U-Boot `md 0xfe32002c`        |
| `CMD0 failed: timeout`                        | controller not talking (clocking/power state unexpected)       | dump `CLKDIV/CLKENA/CLKSRC` from the log, try U-Boot `md 0xfe320008` pre-boot  |
| `CMD8 ok` then `ACMD41 busy timeout`          | card never leaves busy; identification clock too slow/fast      | try `INIT_CLKDIV` 32/128, measure with U-Boot `mmc dev 1; mmc rescan`          |
| `CMD3 failed` / `card returned RCA 0`         | RCA handshake broken (unlikely after ACMD41)                   | compare response words with a U-Boot `mmc switch` trace                        |
| `data error at block ... rint=0x...`          | data CRC/timing at full speed                                  | keep the identification divider (run at cclk/130) as a workaround, then CRU    |
| `data timeout at block ...`                   | FIFO starvation or DTO never fires                             | check `STATUS` FIFO count in the log line, lower chunk size to 1 block          |
| `block 0 signature` not `55aa`                | reads return garbage (byte order/sync issue)                   | hexdump block 0 from debugsh: `hexdump disk.mmc: 512`? (needs scheme read)     |

## First hardware boot (iteration 1, `logs/serial-20261120-082135.log`)

What worked, in order:

- DTB node walk correct: SDIO `mmc@fe310000` skipped, microSD
  `mmc@fe320000` selected; mapped `(len 0x4000)`.
- Inherited state read and logged (all values sane, matching what U-Boot
  leaves after an EFI boot):
  `verid=0x5342270a` (IP 2.70a, FIFO data register +0x200 confirmed),
  `clkdiv=0, clksrc=0, clkena=0x00010001` (enable + low-power),
  `ctype=0x1` (4-bit), `fifoth=0x207f0080` (U-Boot watermark), and —
  decisive — `ctrl=0x02000000` = **CTRL bit 25, USE_IDMAC**: U-Boot's EFI
  path read the ESP through the internal DMA engine.
- Card enumeration did NOT start: the driver failed with
  `Connection timed out` (ETIMEDOUT) inside `set_clkdiv` — the CIU
  clock-update command never reached command-done. No CMD0/CMD8 lines were
  logged, so the failure is before any card traffic.

Diagnosis: the driver tiptoed around the inherited (IDMAC-active) state
instead of resetting it. U-Boot's own `dwmci_init` always starts from a full
`CTRL` reset (controller + FIFO + DMA) before touching clocks; with the DMA
engine still armed, the very first clock-only command stalls. The reset is
compatible with the inherited-clocking strategy: it clears only CIU-side
registers (CLKDIV/CLKENA/CTYPE), which are immediately reprogrammed from
known values — the CRU `cclk_sdmmc` rate is outside the block and untouched.

Iteration 2 changes (built, image regenerated):

- `controller_setup` now performs the full U-Boot reset sequence:
  `CTRL = RESET|FIFO_RESET|DMA_RESET` (wait for self-clear, 500 ms guard),
  then `BMOD = 1` (IDMAC software reset), then INTMASK/TMOUT/FIFOTH as
  before.
- The clock-update command drops `USE_HOLD_REG` to match U-Boot's
  clock-only command exactly (`START | PRV_DAT_WAIT | UPD_CLK`).
- `set_clkdiv` logs the failing stage (`stop`/`div`/`start`) with
  `RINTSTS`/`STATUS` on timeout, so the next capture pinpoints any
  remaining stall.
- Logger no longer writes stderr (initfs daemons inherit init's stdio,
  which is already the debug scheme — every line was duplicated on the
  UART in iteration 1).

Side observations from the same session (debugsh): bare `ls` and `ls /`
now work (`ls /` lists the scheme namespace: `chan? debug? display.vesa?
event? initfs/ input? irq/ kernel.dtb ...`); inputd logs a spurious
`invalid path ''` error line during boot (pre-existing, unrelated).

## Iteration 2 result (`logs/serial-20261120-083143.log`)

The full controller reset worked (`controller reset ok`, reset bits
self-cleared), and the driver still stalled at the first clock update —
but now with a precise register snapshot:

```text
dwmmcd: controller reset ok (ctrl was 0x02000000)
dwmmcd: clock update (stop) failed: Connection timed out rint=0x00000000 status=0x00006106
```

`RINTSTS = 0` means the command never started (no CDONE, no error bits);
`STATUS = 0x6106` has bit 9 (BUSY) and bits 12/13 set — the controller
reports residual data/DMA state right after the reset, and the clock-only
command carries `PRV_DAT_WAIT` ("wait for previous data"), which waits on
exactly that state. Also, `debugsh`'s `reboot` builtin was exercised and
works (PSCI SYSTEM_RESET through `sys:kstop`) — the test loop no longer
needs power-cycling.

Iteration 3 changes (built):

- The `BMOD = 1` (IDMAC software reset) write moved after the clock setup —
  U-Boot writes it after `setup_bus`, not before; writing it first may
  itself arm the busy/DMA state that stalls the clock update.
- Every clock update retries once without `PRV_DAT_WAIT` when the
  conventional form times out: a CIU-only command never touches the data
  lines, so waiting on stale data state is unnecessary; if the retry path
  is what works, the next log will show enumeration proceeding after a
  stop-stage timeout message.

## Status

**in progress** — iteration 3 built and flashed-pending:

- [x] flashed and booted (iteration 1, 2026-11-20 08:21)
- [x] dwmmcd enumeration lines seen on UART (node walk, mapping, inherited
      state)
- [ ] `disk.mmc` scheme registered (iteration 1 failed at clock update;
      reset-based iteration 2 pending)
- [ ] redoxfs mounted the root filesystem
- [ ] `init: switchroot to /usr /etc`
- [ ] rootfs userspace (getty/ion) reached

## Notes

- The eMMC (`fe330000`) and SDIO (`fe310000`) controllers are never probed:
  the driver binds only to a `dw-mshc` node whose first reg entry is exactly
  `0xfe320000`.
- redoxfs scans `/scheme` once with no retry, which is why the notify-type
  service ordering (blocking `50_rootfs.service` behind `40_drivers.target`)
  matters.
- Raw `grep` over `harddrive.img` is unreliable for verifying file contents
  (RedoxFS stores data fragmented across extents); FUSE-mount the image with
  `build/redox/build/fstools/bin/redoxfs -d <img> <mnt>` and compare md5s
  instead. `redoxfs-ar` **creates** filesystems — do not point it at an
  image you want to keep.
