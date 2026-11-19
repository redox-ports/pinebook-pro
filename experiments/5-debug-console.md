# Experiment 5 — Interim serial console (`debugsh`) in the initfs phase

**Status:** iteration 2 pending hardware run (2026-11-20). Iteration 1 (first capture,
`logs/serial-20261120-011908.log`): **`debugsh` starts and prints its banner + prompt on
UART2**, but typed characters produce no echo/response, and `pcid` still panics — see
"Iteration 1 results". Both causes were found and fixed; the rebuild is ready to flash.
**Related:** experiment 4 (first userspace boot), README §4 (local loop), §6 Phase 3/4; `patches/base-initfs-debug-console.patch`, `patches/kernel-uart-16550-rx-irq.patch`

## Iteration 1 results (2026-11-20)

Capture: `logs/serial-20261120-011908.log` (the boot appears twice in it; assumed a
manual reset between runs — `FILL IN` whether that is correct).

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
   TODO no-op. **Fixed** by `patches/kernel-uart-16550-rx-irq.patch`: the test outcome
   is recorded instead of returning early, MCR/IER are always restored, and
   `enable_irq` now really enables RX interrupts for the NS16550 variants.
2. **`pcid` still panicked — it is spawned by `hwd`, not by an init service.**
   Gating `40_pcid-spawner-initfs.service` removed the spawner from the log, but
   `drivers/hwd/src/main.rs` calls `daemon::Daemon::spawn(Command::new("pcid"))`
   directly (deprecated legacy path). **Fixed** in
   `patches/base-initfs-debug-console.patch`: hwd skips the pcid spawn on aarch64.

Remaining question from iteration 1: whether the duplicated boot in the capture was a
manual reset (assumed) or a spontaneous reboot loop.

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
