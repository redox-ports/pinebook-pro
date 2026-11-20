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
- Clock strategy (iteration 6, U-Boot rk3399 model): controller `CLKDIV`
  stays 0 — the block divides cclk by 2 internally and the card rate is
  controlled entirely from the CRU `CLKSEL_CON16` (identification:
  `0x51d` = xin24m/30 → 400 kHz card clock; data: the inherited `0x10e` =
  gpll/15 ≈ 19.8 MHz). CLKENA changes are wrapped in two CIU
  clock-update commands exactly like `dwmci_setup_bus`, each awaited by
  polling the **CMD START bit** self-clearing (U-Boot never checks CDONE
  for clock-only commands; this IP does not raise it — see iteration 6).
  If an update never completes, the driver falls back to plain register
  writes with no CIU involvement.
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
- Bring-up logging: every driver message goes to `/scheme/debug` (kernel
  debug scheme -> UART; stderr is not written — init children inherit
  init's stdio, which is already the debug scheme, so writing both
  duplicates every line, observed in iteration 1), each line timestamped
  `[s.ss]` since driver start. The fd is opened before `setrens`.
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
dwmmcd: pulsed SRST_SDMMC + SRST_SD_NOC
dwmmcd: post-SRST ctrl=... clkena=... clkdiv=... (was ...) clksrc=... status=...
dwmmcd: clksel16 ident-400k (hiword write): 0x0000010e -> 0x0000051d  <-- CRU rate set
dwmmcd: CMD8 ok (v2.0 card, 0x1aa)
dwmmcd: ACMD41 ready, ocr=0xc0ff8000 ccs=true
dwmmcd: clksel16 data-rate (hiword write): 0x0000051d -> 0x0000010e  <-- back to data rate
dwmmcd: card ready: rca=0x.... size=... MiB (block addressed)
dwmmcd: block 0 signature 55aa
[INFO redoxfs] mounted filesystem ...      <-- success
init: switchroot to /usr /etc              <-- root filesystem!
```

(`clksel16` line values are the expected ones: `0x10e` = the inherited
U-Boot data rate, `0x51d` = the U-Boot 400 kHz identification rate. If
instead a `clock update (ident) failed` line appears, the run switched to
the fallback and logs `direct clocks:` next — both paths are valid, only
the lines after them differ.)

4. If mount works: rootfs units start (getty etc.), `ion` shell on the
   serial console — record everything.
5. If it fails: capture the exact `dwmmcd:` line where it stopped.
   `debugsh` now has a `reboot` builtin (writes `reset` to the kernel
   `sys:kstop` scheme -> PSCI SYSTEM_RESET), so the board can be re-tested
   without power-cycling; the debugsh prompt also supports bare `ls` (cwd
   is `/scheme/initfs`) and `ls /` (lists `/scheme`).

## Expected failure modes and next actions

| Symptom (last dwmmcd line)                               | Meaning                                                                      | Next action                                                                                                                                 |
| -------------------------------------------------------- | ---------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `init failed: Connection timed out` (no CMD*)            | iteration 1: clock update stalled on the IDMAC-state controller              | fixed for iteration 2: full controller reset first (see above)                                                                              |
| `clksel16 ... (hiword write): X -> X` (unchanged) | CRU write-mask rejected both the hiword and the plain write | the CRU needs the write-mask convention and the value is still wrong: check `CLKSEL16_MASK` against Linux clk-rk3399.c (`SCLK_SDMMC` on CLKSEL_CON(16): mux [10:8], div [6:0]) |
| `clock update (ident/data) failed` (CMD readback logged) | CIU never accepts the update (START stuck) or accepts and stalls             | iteration 6 fallback engages automatically (`direct clocks:`); capture `cmd`/`status` from the line and compare with U-Boot `md 0xfe32002c` |
| `CMD0 failed: timeout` (cmd readback logged)             | controller not talking: `cmd` START set = CIU rejected it, cleared = stalled | check `post-SRST`/`direct clocks` lines; if CLKENA never latches, toggle the slot power regulator (GPIO)                                    |
| `CMD8 ok` then `ACMD41 busy timeout`                     | card never leaves busy; identification clock too slow/fast                   | change the identification CRU rate (`CLKSEL_400K` = xin24m/30) or compare with U-Boot `mmc dev 1; mmc rescan`                               |
| `CMD3 failed` / `card returned RCA 0`                    | RCA handshake broken (unlikely after ACMD41)                                 | compare response words with a U-Boot `mmc switch` trace                                                                                     |
| `data error at block ... rint=0x...`                     | data CRC/timing at full speed                                                | lower the data rate: larger divider in the CRU `CLKSEL_CON16` data value                                                                    |
| `data timeout at block ...`                              | FIFO starvation or DTO never fires                                           | check `STATUS` FIFO count in the log line, lower chunk size to 1 block                                                                      |
| `block 0 signature` not `55aa`                           | reads return garbage (byte order/sync issue)                                 | hexdump block 0 from debugsh: `hexdump disk.mmc: 512`? (needs scheme read)                                                                  |

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

## Iteration 3 result (`logs/serial-20261120-084441.log`)

Identical stall to iteration 2, now with proof that neither the IDMAC
reorder nor dropping `PRV_DAT_WAIT` is the answer:

```text
dwmmcd: controller reset ok (ctrl was 0x02000000)
dwmmcd: clock update (stop) failed: Connection timed out rint=0x00000000 status=0x00006106
```

The retry without `PRV_DAT_WAIT` also failed (no card lines were ever
reached — no CMD0/CMD8 logs). `RINTSTS = 0` means the CIU never even
started the command; `STATUS = 0x6106` (bit 9 BUSY + bits 12/13) is
consistent with a frozen CIU. A CIU that does not execute commands while
the register block still answers (BIU clock alive) points at the CIU clock
itself being gated.

Iteration 4 changes (built):

- The driver maps the CRU (`ff760000`) and logs the SDMMC-related state:
  `CLKSEL_CON(13)` (HCLK_SD mux/div), `CLKSEL_CON(16)` (SCLK_SDMMC
  mux/div, decoded to source name), and the gate registers
  `CLKGATE_CON(6)/CON(12)/CON(33)`.
- If any of the three SDMMC clocks is gated (SCLK_SDMMC = CIU, CON(6)
  bit 1; HCLK_SD = CON(12) bit 13; HCLK_SDMMC = CON(33) bit 8), the
  driver enables it with a hiword write (Linux `clk_prepare_enable`
  equivalent; all three are set-to-disable gates) and proceeds. This is
  the minimal CRU write the port has to make: it does not change any
  rate or mux, it only prevents an inherited gate from starving the
  controller.

Reference map used (verified in Linux `drivers/clk/rockchip/clk-rk3399.c`
and U-Boot v2021.07 `drivers/clk/rockchip/clk_rk3399.c`):
`COMPOSITE(SCLK_SDMMC, ..., CLKSEL_CON(16), 8,3 mux, 0,7 div,
CLKGATE_CON(6), 1)`, `GATE(HCLK_SDMMC, ..., CLKGATE_CON(33), 8)`,
`COMPOSITE(HCLK_SD, ..., CLKSEL_CON(13), CLKGATE_CON(12), 13)`. Note for
later phases: the dw_mshc CIU on RK3399 divides the CRU clock by two
internally ("dwmmc controller have internal div 2", U-Boot
`rk3399_mmc_set_clk`), and U-Boot switches the source to `xin24m`
(mux value 5) for the 400 kHz identification clock — relevant once the
gate theory is confirmed or disproved.

## Iteration 4 result (`logs/serial-20261120-085752.log`)

The CRU dump settled the clock-gate theory — against it:

```text
dwmmcd: cru hclk_sd sel=0x00000361 sclk_sdmmc sel=0x0000010e
dwmmcd: cru gates: con6=0x00000060 con12=0x00000000 con33=0x00000000
dwmmcd: sclk_sdmmc source gpll div 14
dwmmcd: controller reset ok (ctrl was 0x02000000)
dwmmcd: clock update (stop) failed: Connection timed out rint=0x00000000 status=0x00006106
```

Decoded: HCLK_SD = cpll/4 = 200 MHz (the dtsi `assigned-clock-rates`
value — U-Boot applied it), SCLK_SDMMC = gpll/15 ≈ 39.6 MHz with the
CIU gate (CON6 bit 1) **open**; the 0x60 bits in CON6 are the unused
USB2PHY reference gates. All three SDMMC clocks run, the controller
reset completes, and the CIU still refuses to execute the clock update
(`RINTSTS = 0`: the command never started). The remaining explanation:
the EFI loader's IDMAC traffic left the CIU wedged mid-command, and the
BIU-side CTRL reset cannot unwedge it (this also fits the persistent
`STATUS` bits 12/13 — see the bit-decode correction in the iteration 5
section below).

Iteration 5 changes (built):

- The driver pulses the controller reset lines through the CRU before
  any controller access, exactly like Linux `dw_mci_probe` (which gets
  `SRST_SDMMC` via `resets = <&cru SRST_SDMMC>`): SRST_SDMMC = 122 →
  `SOFTRST_CON(7)` bit 10, plus SRST_SD_NOC = 189 → `SOFTRST_CON(11)`
  `SOFTRST_CON(11)`
  bit 13, assert both, 10 ms, release, 10 ms. Reset lines are hiword
  (mask [31:16], data [15:0], data 1 = assert). Verified from Linux
  `dt-bindings/clock/rk3399-cru.h` (same file in U-Boot v2021.07).

Also corrected a bit-decode error from iteration 4: `STATUS 0x6106` does
**not** have bit 9 (BUSY) set — `0x6106` = bits 1, 2, 8, 12, 13. Bits 12/13
are the IDMAC request/acknowledge pair (the trace of U-Boot's DMA reads);
only bit 2 (FIFO_EMPTY, named in the U-Boot `dwmmc.h`) is a documented
status bit among the rest.

## Iteration 5 result (`logs/serial-20261120-090926.log`)

The SRST pulse **works** — the controller state visibly changed — but the
clock update still fails identically:

```text
dwmmcd: pulsed SRST_SDMMC + SRST_SD_NOC
dwmmcd: controller reset ok (ctrl was 0x02000000)
dwmmcd: clock update (stop) failed: Connection timed out rint=0x00000000 status=0x00000106
```

`STATUS` dropped from `0x6106` to `0x0106`: the SRST cleared the IDMAC
handshake bits 12/13 (and bit 8), while the FIFO status bits (1, 2)
remain — i.e. the reset line genuinely resets the block, and the
controller-reset, clock-gate and SRST theories are all dead as
explanations for the update failure. What survives scrutiny is the
failure observation itself: `RINTSTS = 0` on every attempt, on every
iteration — the CIU never _reports_ anything, not even an error.

The inherited state dump also confirms the full U-Boot clock model on
this board: `clkdiv=0` (controller bypass) with `sclk_sdmmc sel=0x10e`
(gpll/15 ≈ 39.6 MHz cclk, ~19.8 MHz card clock) and
`clkena=0x00010001` (ENABLE | LOW_PWR).

## Iteration 6: the wait mechanism was wrong (root cause found in the U-Boot source)

Re-reading the exact U-Boot that runs on this laptop (v2021.07,
`drivers/mmc/dw_mmc.c`) turned up the discrepancy that explains every
iteration so far:

```c
/* dwmci_setup_bus */
dwmci_writel(host, DWMCI_CMD, DWMCI_CMD_PRV_DAT_WAIT |
                DWMCI_CMD_UPD_CLK | DWMCI_CMD_START);
do {
        status = dwmci_readl(host, DWMCI_CMD);
        if (timeout-- < 0) { ... return -ETIMEDOUT; }
} while (status & DWMCI_CMD_START);
```

U-Boot waits for a clock update by polling the **CMD START bit
self-clearing** and never looks at CDONE. This driver polled
`RINTSTS` for CDONE — which this IP evidently does not raise for
clock-only commands (`RINTSTS` read 0 on every failed attempt). Every
"timeout" in iterations 1–5 may therefore have been a measurement
artifact: the update may have completed while the driver waited for an
interrupt flag that never comes.

The same source confirms the clocking model (also stated verbatim in the
`include/dwmmc.h` `get_mmc_clk` documentation: "If you want to handle the
clock external to DWMMC, use @freq to select the frequency and return
that value too. Then DWMMC will put itself in bypass mode"):

- `rockchip_dwmmc_get_mmc_clk` calls `clk_set_rate` (CRU) and returns the
  requested frequency, so `dwmci_setup_bus` always computes `div = 0`
  (bypass) on rk3399 — the card rate is controlled entirely from the CRU,
  and the controller divides cclk by 2 internally.
- A rate change is exactly two clock-update commands (CLKENA 0 → update,
  CLKENA ENABLE|LOW_PWR → update), not three.

Iteration 6 changes (built, hardware test pending):

- `clk_update_try` now polls `CMD & CMD_START == 0` (U-Boot semantics)
  instead of `RINTSTS` CDONE.
- The clock setup is rewritten as the U-Boot rk3399 model: CRU
  `CLKSEL_CON16` provides the rate (`0x51d` = xin24m/30 → 400 kHz card
  clock for identification; the inherited `0x10e` for data), controller
  `CLKDIV` stays 0, and `clkena_cycle` mirrors the two-command
  `dwmci_setup_bus` sequence.
- Fallback path: if a clock update still never completes (START stuck),
  the driver re-resets the controller and programs `CLKDIV/CLKSRC/CLKENA`
  as plain register writes with no CIU involvement, driving the rate
  from the CRU only — testing whether the CIU latch is needed at all.
- Observability: a post-SRST register dump (CTRL/CLKENA/CLKDIV/CLKSRC/
  STATUS) proves what the pulse reset; CMD0 failures log the CMD register
  readback (START still set = CIU never accepted the command; START
  cleared = accepted but stalled); every log line is timestamped
  (`[s.ss]` since driver start) so step durations are measurable.

Expected outcomes of the next hardware test:

| Outcome                                              | Meaning                                                      |
| ---------------------------------------------------- | ------------------------------------------------------------ |
| `clksel16 ident-400k: 0x10e -> 0x51d`, updates pass, | iterations 1–5 were the CDONE polling artifact; enumeration  |
| enumeration completes, `card ready`                  | proceeds at 400 kHz, data at the inherited rate              |
| updates still fail (START stuck), fallback engages,  | the CIU really is wedged; direct writes bypass it — if the   |
| `CMD0 failed ... cmd=0x8...` START set               | card answers, no CIU latch is needed at all                  |
| fallback engages, CMD0 ok but CMD8/ACMD41 timeout    | cclk_out never reached the card (direct CLKENA write did not |
|                                                      | take effect) — next: card power-cycle via the slot's GPIO    |
|                                                      | regulator, or GRF SDMMC_CON0/1 phase delays                  |

## Iteration 7: the CRU write-mask and the nanosleep stall

Iteration 6's hardware run (log `logs/serial-20261120-095236.log`) put the
driver through both new paths — and the logs pinpointed two independent
problems:

```text
[   0.006] dwmmcd: fifo depth 256 words, data reg at +0x200
debugsh> h[ 249.003] dwmmcd: pulsed SRST_SDMMC + SRST_SD_NOC
...
[ 249.007] dwmmcd: clksel16 ident-400k: 0x0000010e -> 0x0000010e
[ 249.007] dwmmcd: U-Boot clock sequence failed (I/O error), falling back to direct clocking
...
[ 249.010] dwmmcd: init failed: I/O error
```

**Problem 1 — `CLKSEL_CON16` ignores plain writes (write-mask).** The
`clksel16` line writes `0x51d` and reads back the unchanged `0x10e`: the
CRU register silently drops the update. Linux `clk-rk3399.c` declares
`SCLK_SDMMC` on `RK3399_CLKSEL_CON(16)` with
`CLK_MUX_HIWORD_MASK | CLK_DIVIDER_HIWORD_MASK`, i.e. the register follows
the Rockchip write-mask convention: **only bits whose mask bit in [31:16]
is set are written**. The driver now writes `(0x77f << 16) | value`
(`0x77f` = mux bits [10:8] + divider bits [6:0]) and checks the readback
masked to those bits, with a plain-write retry for revisions without the
protection. Note the boot ROM left `0x10e` (gpll/15, ~19.8 MHz card
clock) — U-Boot's own plain `rk_clrsetreg` calls never actually changed
this register either; the SD card was read through the whole U-Boot boot
at that inherited rate.

**Problem 2 — `nanosleep` never wakes on the timer.** The same log shows
the driver logging 7 lines in 6 ms, then its next line at `[249.003]` —
right at the moment a key was pressed at the `debugsh` prompt. The only
blocking call between those lines is `sleep_ms(10)` (the SRST settling
delay), implemented with `nanosleep`: the bring-up kernel does not wake
a sleeping thread on the generic timer and only a serial interrupt shakes
it loose (observed twice: 249 s before an `h` keypress, 6.3 s in the
reboot that followed when input arrived sooner). `sleep_ms` is now a
busy-wait on `now_ms()`; the delays here are short and the driver runs
single-CPU, so spinning is the robust option. This also matters for the
ACMD41 retry loop, which sleeps 10 ms per attempt.

Iteration 7 changes (built, hardware test pending):

- write-masked `CLKSEL_CON16` programming with readback verification
  (`clksel16 ... (hiword write): old -> new` log line) and a plain-write
  retry;
- `sleep_ms` busy-waits (no dependency on the broken `nanosleep` wakeup);
- an `asserting SRST_SDMMC + SRST_SD_NOC` line before the reset writes, so
  any future stall is localized to a specific register access;
- clock failures are no longer fatal: if the CRU switch still fails the
  driver continues at the inherited/identification rate instead of
  exiting (a slow mount beats no mount), logging what it kept.

## Status

**in progress** — iteration 7 built, hardware test pending:

- [x] flashed and booted (iteration 1, 2026-11-20 08:21)
- [x] dwmmcd enumeration lines seen on UART (node walk, mapping, inherited
      state)
- [x] SRST pulse verified on hardware (iteration 5: STATUS 0x6106 →
      0x0106, IDMAC bits cleared)
- [x] clock-update wait mechanism fixed to U-Boot semantics + CRU rate
      control (iteration 6; the CRU write needed the hiword write-mask —
      iteration 7. Not yet exercised on hardware: the iteration-6 run failed
      at the CRU write, before the first clock-update command)
- [x] `CLKSEL_CON16` write-mask discovered and handled (iteration 7, from
      the iteration-6 hardware log: plain writes dropped, readback 0x10e)
- [x] `nanosleep` stall characterized (iteration 6 log: 249 s/6.3 s blocks
      until serial input) and removed in favor of busy-waits (iteration 7)
- [ ] `disk.mmc` scheme registered
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
