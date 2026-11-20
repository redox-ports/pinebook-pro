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
- **Iteration 2** showed a second, independent cause for the same symptom: the
  bootloader did not maintain the caches before jumping into the freshly loaded
  kernel, so some image layouts execute stale instructions. Fix built, hardware
  check pending.
- Verified on hardware: rootfs userspace and uutils applets, timer tick, RedoxFS root
  mount, getty login (marker build).
- Build rule recorded in `AGENTS.md`: after touching a recipe's sources, rebuild that
  recipe; never assemble an image from existing packages.
- Open: the internal keyboard (`ohcid`, experiment 8) still does not deliver input.

## Iteration 2 — the rebuilt image hung again (2026-11-20 21:11)

The clean rebuild (`kernel_size: 0x17ec58`, ELF entry `0xffffff00000a5ce8`) stopped in
exactly the same place: the bootloader completes, `KernelArgs` is printed, no kernel
output at all (`logs/serial-20261120-211139.log`). So the stale artifact was not the
whole story.

### Comparison of the captures

| Capture               | kernel_size | ELF entry | Result         |
| --------------------- | ----------- | --------- | -------------- |
| 10:22 – 11:24         | `0x17e550`  | `0x6a108` | boots to login |
| 15:49                 | `0x17e5d8`  | `0x6a030` | boots to login |
| 18:09                 | `0x16e228`  | `0x63a80` | boots to login |
| 20:37 (diag, markers) | `0x17ecc0`  | `0x4e938` | boots to login |
| 19:59 (stale)         | `0x17ecb8`  | `0x8e468` | silent         |
| 21:11 (clean)         | `0x17ec58`  | `0xa5ce8` | silent         |

Two facts came out of it:

1. The same sources build reproducibly (`touch` + rebuild gives an identical
   binary), but adding the debug markers — 104 bytes of source — moves `kstart`
   by 357 KiB (`0x4e938` vs `0xa5ce8`). The kernel's function layout is reshuffled
   by a tiny source change.
2. The UART is healthy in the silent boots too (`mcr=0x3`, loopback off), so the
   silence is not a port state problem: the kernel never reaches its first write.

### Root cause (inferred): no cache maintenance before the handover

`src/os/uefi/arch/aarch64.rs` copies the kernel image into memory with normal stores
and jumps to it without cleaning the data cache or invalidating the instruction cache.
Instruction fetch is not coherent with the data cache on ARM, so a layout that lands on
physical pages whose lines are still dirty, or whose instructions are still in the
I-cache, can execute stale bytes — and whether that happens depends on which pages the
EFI allocator hands out, i.e. on the image layout. QEMU never shows it because it does
not model caches.

### Fix

`cache_sync_instructions(kernel_base, kernel_size)` in the bootloader, called right
before `entry_fn(args)`: `dc civac` for every data-cache line of the image, then
`dsb sy`, `ic iallu`, `dsb sy`, `isb`.

Status: built and QEMU-tested (smoke test green), hardware check pending.

## Iteration 3 — the kernel's first instruction never ran (2026-11-20 21:51)

A build with **one single marker** (`A`, the first instruction of `kstart`, before the
BSS test and the stack setup) produced **no `A` at all** (`logs/serial-20261120-215145.log`,
checked byte-by-byte with `od`). The marker is a plain MMIO store to the UART through the
physmap, the same code that printed `ABCDEF` in the working run. So in that boot the
kernel either never started, or its very first store never reached the port.

### What actually differs between the working run and the failures

| Time  | Bootloader                             | Kernel           | Result                   |
| ----- | -------------------------------------- | ---------------- | ------------------------ |
| 19:59 | old (firmware-clock menu)              | stale `0x17ecb8` | silent (explained above) |
| 20:37 | old + UART dump                        | markers A…P      | **boots to login**       |
| 21:11 | + arch-counter menu (10 ms units)      | clean            | silent                   |
| 21:35 | + arch-counter menu (µs) + cache clean | clean            | silent                   |
| 21:51 | + arch-counter menu (µs) + cache clean | marker `A` only  | silent, **no `A`**       |

Every boot after 20:37 carries the two bootloader changes made that evening (the
architected-counter menu clock at 20:22 and the cache clean at 21:23). The arch-counter
path is **never executed in QEMU** (QEMU runs the bootloader at EL1, the code requires
EL2), so it was only ever exercised on hardware — in the three failing boots.

Build determinism was re-verified (`touch` + rebuild gives an identical binary), the
marker-A kernel was disassembled (`kstart` and the `bl early_marker` are correct, `start`
resolved), and the kernel bytes were found intact inside the flashed image, so neither a
broken artifact nor a bad handoff address explains the missing `A`.

### Next experiment

Two images isolate the remaining variables (both with the clean kernel):

1. `build/diag-oldboot-clean.img` — bootloader reverted to the state that worked at 20:37
   (firmware-clock menu, no UART dump, no cache clean);
2. `build/diag-oldboot.img` — the same plus the read-only UART dump.

Interpretation: if (1) boots, one of the two bootloader changes broke the handover; if
only (2) boots, the dump itself matters; if both stay silent, the marker code in the
kernel is required for the boot and the marker side effects must be understood.

## Iteration 4 — the UART dump is the magic (2026-11-20 22:0x)

Result of the isolation pair (both with the clean kernel and the old, firmware-clock
bootloader):

| Image                    | Extra work before `kernel_entry`    | Result     |
| ------------------------ | ----------------------------------- | ---------- |
| `diag-oldboot-clean.img` | none                                | **silent** |
| `diag-oldboot.img`       | 4 UART register reads + one println | **boots**  |

Everything else in the two images is identical (same kernel artifact, same bootloader
sources, same allocation layout). So the deciding factor is the `dump_uart2()` call
itself - which is read-only for the port - or the extra console write it performs, or
the time they take. This also re-frames every earlier observation: the marker kernels
only ever booted behind the dump bootloader, and the kernel-side markers were never the
fix.

### Splitting the dump

Two more images isolate which part matters (same old bootloader, clean kernel):

1. `build/diag-delay.img` — a pure ~100 ms spin loop where the dump was: no register
   access, no console output.
2. `build/diag-println.img` — a single `println!` (console write) with no register
   reads.

Interpretation: delay boots ⇒ the handover is time-sensitive (something needs to settle
after the last console output); println-only boots ⇒ the console/driver activity is
what matters; neither boots ⇒ the register reads themselves change the state the kernel
inherits.

## Iteration 5 — the reads themselves; the initfs-size theory dies (2026-11-20 22:2x)

Results of the split pair (same old bootloader, clean kernel):

| Image              | Where the dump was                             | Result                        |
| ------------------ | ---------------------------------------------- | ----------------------------- |
| `diag-delay.img`   | ~100 ms spin, no MMIO access, no output        | silent                        |
| `diag-println.img` | one console write (`handover: console primed`) | the line printed, then silent |

`logs/serial-20261120-222230.log`, `logs/serial-20261120-222445.log`.

- The println variant proves the extra code ran and the console still worked at that
  point; the kernel still never started. Not the console activity.
- The delay rules out "something needs time to settle". Not timing.
- U-Boot's console driver reads LSR (0x14) before every character it outputs —
  including the `KernelArgs` print in the very boots that hang — and that does not
  substitute.

What remains is the four 32-bit loads from `0xff1a0004/0x0c/0x10/0x14`, three of which
(IER, LCR, MCR) are registers U-Boot's console path never touches.

### The whole day in one table

Rebuilt from the captures (`initfs:`, `012345`, `Redox OS starting`, `redox login`
over `logs/`):

| Captures         | initfs                               | Bootloader touches UART2 registers | Kernel starts | Login              |
| ---------------- | ------------------------------------ | ---------------------------------- | ------------- | ------------------ |
| 00:20–02:13 (13) | 22 MiB                               | yes (markers `012345`)             | yes           | – (no storage yet) |
| 08:21–11:24 (9)  | 23 MiB                               | yes (markers)                      | yes           | 3 ×                |
| 13:52–18:09 (3)  | **24 MiB** (`ohcid` enters at 13:52) | yes (markers)                      | yes           | 2 ×                |
| 19:59            | 24 MiB                               | **no** (markers removed)           | **no**        | –                  |
| 20:37            | 24 MiB                               | yes (dump + kernel markers)        | yes           | yes                |
| 21:11–22:24 (8)  | 24 MiB                               | no                                 | no (silent)   | –                  |
| 22:11            | 24 MiB                               | yes (old bootloader + dump)        | yes           | yes                |

The markers wrote `MCR = 0x03` (a register **write**), the dump reads IER/LCR/MCR/LSR —
either works, nothing else does. **The "initfs grew 22→24 MiB" theory is disproven:
24 MiB images booted repeatedly between 13:52 and 18:09, behind the markers.** The
invariant is a bootloader access to a UART2 register that U-Boot's console driver never
touches, before `kernel_entry`.

## Iteration 6 — the deliverable image, hardware-verified (2026-11-20 23:07)

`dump_uart2()` is part of the committed patch set: the regenerated
`patches/bootloader/src/os/uefi/arch/aarch64.rs.patch` carries the cache clean and the
dump (`scripts/apply-patches.sh --check`: 46 entries up to date). Image rebuilt with
`scripts/build-image.sh`:

- EFI = fresh build (sha256 `3821aa00…`, byte-identical inside the image, contains the
  dump strings; no `handover: console primed` — the stale-artifact trap was checked
  explicitly),
- kernel = the clean artifact (sha256 `48bfe3d7…`, 1 567 832 bytes),
- QEMU smoke green: on `virt` the dump reads zeros (`0xff1a0000` is inside the RAM
  mapping; `uart2 pre-kernel: ier=0x0 lcr=0x0 mcr=0x0 lsr=0x0` in
  `build/qemu-smoke.log`), so emulation cannot reproduce the dependency — it only
  proves the dump is harmless there.

**Hardware (`logs/serial-20261120-230717.log`): boots to `redox login:`, `user` logs
in, `ls` works.** The combination — µs arch-counter menu + cache clean + GOP RGB565 +
`dump_uart2` + clean kernel — is the deliverable. `build/diag-oldboot.img` stays as the
proven fallback. The mechanism remains unknown (inferred candidates: EL2-stage TLB
entries created by the reads surviving the EL2→EL1 switch; an interconnect/
clock-domain side effect of transactions U-Boot never issues). Bisect variants are
designed (reads-without-println, per-register, unrelated MMIO, 8-bit width) but not
built — deprioritized behind input, see experiment 8.

### USB after this boot (input for experiment 8)

`ohcid` no longer freezes the SoC: CRU gates read back open, both USB2 PHYs re-powered,
both controllers operational. Enumeration fails on both ports with different
signatures — `fe3e0000` (internal hub, full-speed): `Halted` ~33 ms after reset (the
first GET_DESCRIPTOR ended in STALL/ED-halt); `fe3a0000`: low-speed device, TD never
accessed → `Timeout("transfer descriptor")`. U-Boot enumerated these same devices
minutes earlier in the same boot, so the hardware path works and the remaining defect
is in our driver's protocol logic — debuggable in emulation (`pci-ohci`/`usb-hub`/
`usb-kbd` exist in QEMU and the packaged U-Boot binds them).
