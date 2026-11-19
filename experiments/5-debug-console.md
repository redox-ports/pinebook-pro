# Experiment 5 — Interim serial console (`debugsh`) in the initfs phase

**Status:** iteration 5 ready to flash (2026-11-20). Iteration 4 (tick dumps) proved
bytes physically reach the UART2 FIFO (`LSR` bit 0 set while typing) and the SPI is
grouped/enabled — yet stays **pending at the distributor forever** (ISPENDR4 bit set,
zero `debug: irq`). Root cause: nobody programmed `GICD_IROUTER` — with affinity
routing an SPI whose routing entry does not select a running CPU is never forwarded.
Iteration 5 routes every SPI to the boot CPU. Also: the earlier "spontaneous reboot"
turned out to be two separate capture files glued together (raw logs show no reset —
closed in README risk #11); the DW watchdog check in U-Boot found it disarmed.
**Related:** experiment 4 (first userspace boot), README §4 (local loop), §6 Phase 3/4; `patches/base-initfs-debug-console.patch`, `patches/kernel-uart-16550-rx-irq.patch`, `patches/kernel-gicv3-spi-group1.patch`, `patches/debug/kernel-serial-irq-dump.patch`, `scripts/github-mirrors.sh`

## Iteration 1 results (2026-11-20)

Capture: `logs/serial-20261120-011908.log`.

What worked:

- `debugsh` started: `debugsh: interactive console on /scheme/debug`, `debugsh>` prompt
  visible on UART2 (before `vesad` even ran); init service gating/target wiring works.
- Kernel, initfs userspace, framebuffer path unchanged from experiment 4.

What failed, and why:

1. **No echo — UART RX interrupt was never enabled.** `uart_16550::init()` runs a
   loopback self-test that fails on RK3399 UART2 and returned early — skipping the
   final `modem_ctrl = 0x0F` / `int_en = 0x01` writes. The port was left with
   `IER = 0` (no RX interrupts → `debug_input` never fires → blocking
   `read(/scheme/debug)` never returns) and `MCR = 0x1E` (loopback — the same reason
   the uninstrumented kernel was silent in experiment 4 until the marker helper
   cleared MCR). Additionally `SerialKind::enable_irq` for NS16550 was a documented
   TODO no-op. **Fixed** by `patches/kernel-uart-16550-rx-irq.patch`.
2. **`pcid` still panicked — it is spawned by `hwd`, not by an init service.**
   Gating `40_pcid-spawner-initfs.service` removed the spawner from the log, but
   `drivers/hwd/src/main.rs` calls `daemon::Daemon::spawn(Command::new("pcid"))`
   directly (deprecated legacy path). **Fixed** in
   `patches/base-initfs-debug-console.patch`: hwd skips the pcid spawn on aarch64.

## Iteration 2 results (2026-11-20)

Capture: `logs/serial-20261120-<time>.log` (second boot after the fixes).

- **`pcid` panic gone** — hwd no longer spawns it on aarch64. ✔
- **Still no echo**: IER is (per the driver) written, but no keystroke reaches
  `debugsh`. So the UART receive interrupt is still not delivered.
- **The board reboots on its own** after `init: switchroot to /usr /etc` (confirmed by
  the device owner: no manual power cycle). Reset time looks constant (~17–19 s after
  kernel start), which points at a hardware watchdog armed before Redox runs (likely
  the DW watchdog at `0xff830000`, possibly started by U-Boot) — Redox never pets it.

Hypotheses for the missing input, to be distinguished by iteration 3:

1. IER is not actually 0x01 (e.g. the write lands elsewhere, or the port is
   re-initialized later);
2. the GIC distributor does not deliver SPI 100 (e.g. Group 0 → FIQ → EL3/unhandled,
   or ISENABLER not set);
3. the UART receives no bytes at all (pinmux/RX wiring in this boot stage).

## Iteration 3 — diagnostics build and results (2026-11-20)

Kernel changes in this build:

- `patches/kernel-gicv3-spi-group1.patch` (new): mark all GICv3 SPIs Group 1
  non-secure before enabling the distributor (correct for an NS kernel, harmless if
  the GIC is single-security-state).
- `patches/debug/kernel-serial-irq-dump.patch` (temporary): prints UART2
  `IER/IIR/MCR/LSR/MSR` and GICD `CTLR/IGROUPR4/ISENABLER4/ISPENDR4/ICFGR8` at
  serial-init and on the first three serial interrupts.

Capture (`logs/serial-20261120-013658.log`) printed:

```
debug: init uart ier=0x1 iir=0xc1 mcr=0x3 lsr=0x0 msr=0x0
debug: init gicd ctlr=0x3 igroupr4=0xffffffff isenabler4=0x10 ispender4=0x0 icfgr8=0x55555555
```

- `IER=0x1`: receive interrupt enabled in the UART;
- `MCR=0x3`: loopback off (OUT1/OUT2 cleared by the temporary marker helper, which
  does not affect interrupts on this UART);
- `ISENABLER4=0x10`: IRQ 132 enabled in the GIC; `IGROUPR4=0xffffffff`: all SPIs
  are Group 1 non-secure; `ICFGR8=0x55555555`: level-triggered, as in the DT;
- **no `debug: irq` lines** after typing at the `debugsh>` prompt.

Conclusion: configuration is correct on both sides; the UART never raises the
receive interrupt. Likely cause: `uart_16550::init()` sets `FCR=0xC7`, i.e. a
**14-byte receive FIFO trigger** — one keystroke never reaches 14 bytes, and this
UART does not provide a usable character-timeout interrupt (the Linux `8250`
driver uses a one-character trigger for exactly this reason).

## Iteration 4 — FIFO threshold fix, tick dumps, results (2026-11-20)

Changes:

- `patches/kernel-uart-16550-rx-irq.patch`: `FCR` now `0x07` (one-character receive
  threshold) — single keystrokes must raise a receive interrupt.
- `patches/debug/kernel-serial-irq-dump.patch`: additionally dumps the UART/GIC state
  every ~2 s from the timer interrupt (tag `tick`), so it is visible whether bytes
  physically arrive (`LSR` bit 0) even if the receive interrupt still does not fire.

Capture: `logs/serial-20261120-015309.log`. Both changes worked; typed bytes are now
observable:

- before typing: `tick ... iir=0xc1 lsr=0x60` (THRE|TEMT, FIFO empty), `ispender4=0x0`;
- after typing: `tick ... iir=0xc4 lsr=0x61` (bit 0 = **data in RX FIFO**),
  `ispender4=0x10` — **SPI 100 pending at the distributor**;
- still zero `debug: irq` lines: the interrupt never reaches the CPU.

Conclusion: UART fine (data arrives, interrupt raised), GIC distributor fine
(pending registered) — delivery to the CPU fails. `GicDistIf::init()` never writes
`GICD_IROUTER`; with GICv3 affinity routing an SPI routed to a non-running peer is
held pending indefinitely. Fixed in `patches/kernel-gicv3-spi-group1.patch`:
`IROUTER<n> = 0` (boot CPU, affinity 0.0.0.0) for every SPI (this iteration). The
tick dump was extended with `typer`, `irouter132`, `gicr_waker` for confirmation.

Also closed this iteration:

- **No spontaneous reboot.** The raw capture files (`logs/serial-20261120-013658.log`,
  `014830.log`) end with `picocom Terminating` and contain no second boot and no
  reset: the "reboot loops" seen earlier were two separate capture sessions glued
  together in chat. README risk #11 resolved; the U-Boot `md 0xff830000` check
  confirmed the DW watchdog is disarmed (`CR` bit 0 = 0, `CCVR` static).

## Build-tree note: GitHub mirrors

Repeated `repo cook` runs stalled fetching recipe sources from gitlab.redox-os.org.
`scripts/github-mirrors.sh` retargets the mirrored recipe repos (12 of 14; `base`
and `ca-certificates` have no mirror and stay on GitLab) to github.com/redox-os/*
(same commit SHAs, official mirrors); `bootstrap-redox.sh` calls it automatically.
Notably the GitHub kernel mirror sits at `719e8f5c` — exactly the base of our
patches — while GitLab had moved ahead to `d698f668`.

## Iteration 5 — actions

Flash the image rebuilt at 02:04 (IROUTER fix + extended tick dump). Expected:

- typing echoes immediately; `debug: irq` lines appear on the first three keystrokes
  (irouter132=0x0, gicr_waker shows an awake redistributor);
- then the console is fully usable: `help`, `ls /scheme`, `cat`, `exec hwd`;
- afterwards remove the temporary instrumentation and proceed to the DW MMC storage
  driver (the real Phase 4 blocker).


## Goal

Get an interactive console on UART2 **before** storage exists. From experiment 4 we
know the boot stops in the initfs phase: the root RedoxFS mount fails (no microSD DW
MMC driver) and `init` cannot switchroot to `/usr`, so no shell ever starts. Instead of
a full getty/login stack (needs a PTY daemon and `/etc/passwd`, neither of which the
initfs carries), this experiment runs a purpose-built initfs daemon:

```
kernel (UART2 RX IRQ -> debug scheme) -> debugsh (echo + line editing + built-ins)
                                      -> exec: initfs binaries (hwd, ramfs, ...)
```

`debugsh` is added by `patches/base-initfs-debug-console.patch` (against
`redox-os/base` @ `8f576d27`):

- opens `/scheme/debug` (kernel debug scheme: reads = serial RX fed by the UART IRQ
  handler, writes = serial TX) and dups it to fds 0/1/2;
- provides echo, backspace, `Ctrl-C` (clear line), `Ctrl-U` (kill line);
- built-ins: `help`, `ls`, `cat`, `echo TEXT... [> FILE]` (write to scheme files!),
  `hexdump FILE [BYTES]`, `env`, `exec CMD [ARGS...]`, `exit`;
- `exec` inherits fds and `PATH=/scheme/initfs/bin`, so initfs binaries (e.g. `hwd`)
  can be run and inspected;
- started as an aarch64-only initfs service (`95_debugsh.service`, notify type) hung
  off `90_initfs.target` — units in the initfs config dir only start if reachable from
  the `00_runtime.target` / `90_initfs.target` chains, because the second
  `switch_root` **replaces** the config dir list with `/usr/lib/init.d`;
- the same patch gates both `pcid-spawner` services (initfs and rootfs phases) to
  x86/x86_64: `pcid`'s aarch64 PCI fallback is `todo!()` and panicked in experiment 4.

## Actions

1. Rebuild locally (the patches are already applied to `build/redox`):
   ```
   CI=1 make -C build/redox r.kernel r.base image
   ```
   (`repo`'s cook TUI needs `CI=1` when stdout is not a TTY; this is the upstream
   convention.)
2. Flash and capture:
   ```
   scripts/flash-sd.sh
   scripts/serial-log.sh
   ```
   Power the Pinebook Pro on with the microSD inserted and interrupt nothing — let it
   autoboot.

## Expected results

- experiment 4 log up to `init: switchroot to /scheme/initfs ...`;
- **no** `pcid` panic / EL0 exception dump (spawner is gated off on aarch64);
- after the rootfs mount failure (`not able to mount uuid ...`) and
  `init: switchroot to /usr /etc`, a banner on UART:
  `debugsh: interactive console on /scheme/debug` + `debugsh>` prompt;
- typed keys echo; `help`, `ls /scheme`, `cat /scheme/initfs/lib/init.d/95_debugsh.service`,
  `exec hwd` work.

## FILL IN after the run

### Capture

```
PASTE FULL UART CAPTURE HERE
```

### Interactive session

```
PASTE PROMPT SESSION (help, ls /scheme, cat ..., exec hwd) HERE
```

### Observations

| Question                                                         | Answer                                                                                                                                                                    |
| ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `debugsh>` prompt reached on UART2?                              | `FILL IN`                                                                                                                                                                 |
| Typed characters echo and backspace works?                       | `FILL IN`                                                                                                                                                                 |
| `pcid` panic gone?                                               | `FILL IN`                                                                                                                                                                 |
| Does serial RX input actually arrive (UART IRQ → `debug_input`)? | `FILL IN` — if there is no echo, the RX interrupt path is dead: check `SerialKind::enable_irq` (Ns16550u32 is a no-op TODO) and whether `init_early` ran with `skip_init` |
| `exec hwd` (or another initfs binary) works?                     | `FILL IN`                                                                                                                                                                 |
| Kernel log lines still interleaved with shell output?            | `FILL IN` (expected, cosmetic)                                                                                                                                            |
| Log file path                                                    | `FILL IN`                                                                                                                                                                 |

### Verdict

`FILL IN`
