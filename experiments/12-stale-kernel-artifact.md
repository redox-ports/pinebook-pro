# Experiment 12 — A stale kernel artifact, and the first working rootfs userspace

**Date:** 2026-11-20
**Goal:** explain why the image built at 19:55 went silent right after the bootloader
handover, and confirm the experiment 10/11 fixes on hardware.

**Related:** experiments 10 (userspace crashes) and 11 (timer), README §7 risks 13/14;
`scripts/build-image.sh`, `patches/debug/README.md`.

## Symptom

`logs/serial-20261120-195914.log`: U-Boot → Redox bootloader → the mode menu times out
on its own → `usr/lib/boot/kernel` and `initfs` are read → `kernel_entry(...)` and
`KernelArgs {...}` are printed → **no further output at all**. The kernel never printed
its first line (`kernel::dtb::serial::INFO -- UART ...`), so no kernel log line was
available to locate the stop. Two boots in the same capture ended identically.

Correlation across every capture of the day: each boot that reached userspace also
showed the temporary bootloader markers (`012345`), and this was the first boot of a
bootloader without them. Two candidates: (a) the kernel died before its first output,
(b) the kernel ran, but the console was left in loopback — the markers also wrote
`MCR = 0x03`.

## Diagnostic image

A throwaway image (`build/diag-markers.img`, not committed) was built to separate the
two, with both probes applied by hand:

1. a **read-only** dump of the RK3399 UART2 registers in the bootloader, printed through
   EFI `ConOut` right before the handover (`uart2 pre-kernel: ier=… lcr=… mcr=… lsr=…`);
2. the kernel markers A…P from `patches/debug/kernel-early-uart-markers.patch`
   (`A` = `kstart` entry, `B` = before Rust, `C` = `start()`, `D` = args read,
   `E` = `graphical_debug`, `F` = DTB parse, `G` = `serial::init_early`, …).

## Results (2026-11-20 20:37, `logs/serial-20261120-203709.log`)

- `uart2 pre-kernel: ier=0x0 lcr=0x3 mcr=0x3 lsr=0x60` — 8N1, DLAB off, **loopback
  off**, THR empty and idle. The port is in a usable state at the handover, so the
  loopback theory is dead.
- `ABCDEF` appear and the normal kernel log follows: the kernel runs and reaches
  userspace.
- `dwmmcd` reports `card ready: rca=0xe624 size=30436 MiB (block addressed)` at
  `[   0.115]`, and the log timestamps advance (`[0.001]`, `[0.002]`, `[0.011]`, …): the
  architected timer ticks.
- `debugsh: disk.mmc is up, releasing the console to the login`; redoxfs mounts the root
  from `disk.mmc/0p2`; `redox login:` accepts `user`.
- **`ls /bin`, `ls /usr/` and `ls /usr/bin/` print complete listings** — the uutils
  startup crash of experiment 10 is gone.
- `exit` from the shell logs
  `[src/procmgr.rs:2071 ERROR] failed to send SIGCHLD to parent PID ProcessId(28): Operation not permitted`
  (harmless for the port so far; follow-up, probably a parent that already exited).

## Root cause

The kernel inside the 19:55 image was **not built from the sources that shipped it**.
`KernelArgs.kernel_size` was `0x17ecb8` (1 567 928 bytes), while a rebuild of the same
tree produces `0x17ebd8` (1 567 832) — 96 bytes of code that no longer exist anywhere in
the tree (inferred: leftover instrumentation from the experiment 11 debugging). It was
built at 18:33, the instrumentation was then removed from the sources **without
rebuilding**, and `make r.bootloader image` re-assembled the image around the stale
package.

`make image` does not rebuild recipes; it assembles whatever is already built. Only
`make all` (or `r.<recipe>`) rebuilds a changed source tree. The image used for this
run was rebuilt with `make r.bootloader r.kernel image` and the kernel dropped from
1567928 to 1567832 bytes.

## Verdict

- Silent-hang mystery closed: stale artifact in the image, not a UART or bootloader
  regression. The bootloader markers were never required.
- Verified on hardware: rootfs userspace and uutils applets, timer tick, RedoxFS root
  mount, getty login.
- Build rule recorded in `AGENTS.md`: after touching a recipe's sources, rebuild that
  recipe; never assemble an image from existing packages.
- Open: the internal keyboard (`ohcid`, experiment 8) still does not deliver input.
