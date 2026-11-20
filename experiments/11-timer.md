# Experiment 11 — the architected timer never fired (GICv3 redistributor was not programmed)

**Date:** 2026-11-20 (autonomous work session)
**Goal:** make the scheduler tick, i.e. give the kernel preemption and make
`nanosleep`/timeouts work. Reported symptom: the VT only repaints when a key
produces a syscall.

**Related:** experiment 4 (GICv3 bring-up), README §7 risk 13;
`patches/kernel-gicv3-redistributor-ppi.patch`.

## Symptom (device owner, 2026-11-20)

> "И ещё забыл сказать, твой таймер не работает, так же надо нажимать Enter."

Whatever was written to the console appeared on the panel only after pressing
Enter, i.e. only when input produced a syscall. The same root cause had already
shown up as a driver symptom in experiment 6:

> kernel `nanosleep` does not wake on the generic timer (a 10 ms sleep blocked
> until a serial interrupt; `dwmmcd` now busy-waits)

## Boot log evidence

```
kernel::arch::aarch64::device::irqchip::gicv3:INFO -- GicV3 { gic_dist_if: GicDistIf { address: FFFF8000FEE00000, ncpus: 6, nirqs: 120 }, gic_cpu_if: GicV3CpuIf, gicrs: [(FEE00000, 10000)], irq_range: (0, 0) }
kernel::arch::aarch64::device::generic_timer:INFO -- architected timer virq = 27 (virtual)
```

`gicrs: [(FEE00000, 10000)]` is the **distributor**, not a redistributor frame.
The RK3399 DT (`arm,gic-v3`) lists `fee00000`/`0x10000` (GICD) followed by
`fef00000`/`0xc0000` (six 0x20000 redistributor frames), then the legacy
GICC/GICH/GICV windows — and the kernel already maps all five regions as device
memory, so the devicetree was parsed correctly at that level.

## Root cause (two defects, both in `gicv3.rs`)

1. **`GicV3::parse()` re-created the `reg()` iterator per entry.**

   ```rust
   let chunks = node.reg().unwrap();
   for _ in 0..gicrs {
       if let Some(gicr) = chunks.iter::<usize, usize>().next() { ... }
   }
   ```

   `reg().iter()` always starts at the first chunk, so the loop pushed the GICD
   entry `#redistributor-regions` times. The frames were also stored without
   `PHYS_OFFSET`, unlike `gic_dist_if.address`.

2. **The redistributor was never programmed.** In GICv3 the SGIs (0..16) and
   PPIs (16..32) are banked per CPU: their enable, group, priority and trigger
   configuration live in the redistributor's SGI frame
   (`GICR_ISENABLER0`, `GICR_IGROUPR0`, `GICR_IPRIORITYR`, `GICR_ICFGR1`), and
   the frame must be woken through `GICR_WAKER`. `GICD_ISENABLER` only covers
   SPIs (ID 32+), so

   ```rust
   // irq_enable() in GicV3, before the patch
   unsafe { self.gic_dist_if.irq_enable(irq_num) }   // GICD_ISENABLER + ...
   ```

   wrote a **RES0** bit for ID 27 (the architected timer PPI). Nothing was
   enabled, and nothing reported an error. Every SPI-based interrupt (UART2,
   virq 132) worked, which is why the console looked healthy and only timing
   was missing:

   - `context::switch::tick()` is called _only_ from `generic_timer::handle()`,
     so without the PPI nothing preempts;
   - `nanosleep` blocks with `context.wake = Some(end)` and is only re-armed by
     `update_runnable` during a switch, so it never wakes;
   - `timeout::trigger()` is called from the same handler, so scheme timeouts
     never expire;
   - hence the VT repainted only when input caused a syscall.

## Fix

- Parse the redistributor region with a single iterator (the entry after the
  GICD) and translate it with `PHYS_OFFSET`, like the distributor.
- `current_cpu_gicr()`: a single `#redistributor-regions` entry describes all
  frames back to back, so the stride is `region_len / ncpus` (0xc0000 / 6 =
  0x20000 on the RK3399), and the frame index is the logical CPU id.
- `init_current_cpu_redist()`: clear `GICR_WAKER.ProcessorSleep`, wait
  (best effort — QEMU and the GIC-500 can leave undefined bits set, and the
  warning is at `debug!` level), then set `GICR_IGROUPR0 = 0xffff_ffff`
  (Group 1 NS, matching the SPI setup), `GICR_IPRIORITYR = 0xa0…`,
  `GICR_ICFGR1 = 0` (PPIs level-triggered, which is what the architected timer
  needs).
- `irq_enable`/`irq_disable`: route IDs below 32 (SGIs and PPIs) to
  `GICR_ISENABLER0`/`GICR_ICENABLER0`, everything else to the distributor.

## Verification

QEMU (`-M virt,gic-version=3`, i.e. the same GICv3 code path) with a temporary
probe in a static initfs daemon:

```
gicv3: GicV3 { gic_dist_if: GicDistIf { address: FFFF800008000000, ncpus: 1, nirqs: 120 }, ..., gicrs: [(FFFF8000080A0000, F60000)], irq_range: (0, 0) }
gicv3: redistributor 0xffff8000080a0000 ready, GICR_WAKER 0x6 -> 0x6
[   0.012] timerprobe: sleep(10ms) took 10.906032ms
[   0.264] timerprobe: sleep(250ms) took 250.249648ms
```

`gicrs` now reports the redistributor region (0x080a0000 / 0xf60000 with one
CPU), and `nanosleep` returns on time. Before the fix the same probe never
returned.

The boot still reaches the initfs console (`scripts/qemu-smoke.sh` passes), and
the probe was removed before the image was rebuilt. **Hardware result:
`FILL IN`** — the expectation is a VT that updates without keystrokes, a
working `sleep`, and no more busy-waiting workarounds in drivers.

## Follow-ups

- Secondary CPUs: `generic_timer::init_ap()` and `irqchip::enable_local_irq()`
  still write the distributor for PPI ids (see risk 13); the GICR path added
  here is the one `irq_enable` now uses for the BSP, so extending AP bring-up
  should reuse it instead of `gic::enable_local_irq()`.
- `gic::init_current_cpu()` (the GICv2 code path) also resets SGIs/PPIs through
  `GICD_*`; harmless on the BSP here because GICv3 never calls it, but it would
  be wrong for a GICv3 AP.
