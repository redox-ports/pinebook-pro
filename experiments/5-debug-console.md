# Experiment 5 — Interim serial console (`debugsh`) in the initfs phase

**Status:** iteration 3 (diagnostics) ready to flash (2026-11-20). Iteration 1: banner+prompt
OK, no input, `pcid` panic. Iteration 2: `pcid` panic and the `IER=0` bug are gone, **still
no input**. **The board reboots by itself** after `init: switchroot to /usr /etc` (device
owner confirmed: no manual reset). Iteration 3 instruments the UART and GICv3 distributor
state and adds the Group 1 SPI routing, plus a U-Boot-side watchdog check.
**Related:** experiment 4 (first userspace boot), README §4 (local loop), §6 Phase 3/4; `patches/base-initfs-debug-console.patch`, `patches/kernel-uart-16550-rx-irq.patch`, `patches/kernel-gicv3-spi-group1.patch`, `patches/debug/kernel-serial-irq-dump.patch`

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

## Iteration 3 — actions (diagnostics)

Kernel changes in this build:

- `patches/kernel-gicv3-spi-group1.patch` (new): mark all GICv3 SPIs Group 1
  non-secure before enabling the distributor (correct for an NS kernel, harmless if
  the GIC is single-security-state).
- `patches/debug/kernel-serial-irq-dump.patch` (temporary): prints UART2
  `IER/IIR/MCR/LSR/MSR` and GICD `CTLR/IGROUPR4/ISENABLER4/ISPENDR4/ICFGR8` at
  serial-init and on the first three serial interrupts.

Device-owner steps:

1. Flash the new image and capture a normal boot; type a few keys at the `debugsh>`
   prompt.
2. Power-cycle, and at `Hit any key to stop autoboot` press a key to stop U-Boot:
   - if the key press does not stop autoboot, UART RX is dead already in U-Boot;
   - else wait 3–4 minutes at the prompt: if the board resets by itself, the
     watchdog is armed before the OS (confirm the hypothesis);
   - also run `md 0xff830000 8` twice (DW watchdog registers: CR/TORR/CCVR/CRR/…; a
     counting `CCVR` or `CR` bit 0 means the watchdog is armed).

What to look for in the capture:

| Line                                   | Meaning                                                  |
| -------------------------------------- | -------------------------------------------------------- |
| `debug: init uart ier=0x1 mcr=0xf ...` | IER/MCR are correct after serial init                    |
| `debug: init gicd ... isenabler4=...`  | bit 4 of `isenabler4` set (IRQ 132 enabled)              |
| `debug: irq ...` lines                 | the UART interrupt does fire when keys are typed         |
| `igroupr4` bits                        | SPI group routing (bit 4 = IRQ 132)                      |
| no `debug: irq` at all                 | either RX data never arrives or the SPI is not delivered |

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
