# Experiment 14 — All CPUs: secondary boot on GICv3

**Date:** 2026-11-21
**Goal:** the desktop felt very slow. Two suspects: the build (release? no —
all recipes build with `--release`; base is even `opt-level = "s"`) and the
hardware running on **one core**: `CPU activation unavailable: BSP has no
directed-SGI target mask`. The RK3399 has six cores (4× A53 + 2× A72), so
five of them were parked.

**Related:** experiment 11 (GICv3 redistributor / PPIs), experiment 13
(desktop), README §7 risk 17 (CPU frequency), `patches/kernel/README.md`.

## Why the secondaries never started

`start_secondaries()` (`src/arch/aarch64/device/cpu/boot.rs`) prepares each AP
(stack, per-CPU block, boot args), calls PSCI `CPU_ON`, and then waits for a
handshake: the AP raises `STATE_LOCAL_READY` and the BSP sends it a wakeup SGI;
the AP must see that SGI and a timer interrupt before it goes online. The
guard before all of this demanded a GICv2 **target mask**, which does not exist
on GICv3: SGIs there are addressed by MPIDR affinity through
`ICC_SGI1R_EL1`. The same GICv2-only assumption sat in three more places
(`init_ap`, `cpu_capacity`, `enable_local_irq`, all IPIs).

## Changes (all in `patches/kernel`)

1. **Trait** (`src/dtb/irqchip.rs`): new default-error methods
   `init_ap_local()`, `send_sgi_to_mpidr(sgi, mpidr)`,
   `send_sgi_all_but_self(sgi)`, `smp_cpu_capacity()`.
2. **GICv3** (`gicv3.rs`): per-CPU init = system-register interface
   (`ICC_SRE/CTLR/IGRPEN1/PMR`) + the CPU's redistributor frame (wake, group,
   priority, trigger), plus SGIs 0–15 enabled in `GICR_ISENABLER0`; SGIs sent
   by writing `ICC_SGI1R_EL1` with a target list (one CPU) or `IRM = 1`
   (all but self); capacity = redistributor region / 128 KiB.
3. **Dispatcher** (`irqchip/mod.rs`): `init_ap`, `cpu_capacity`, `smp_ready`,
   `send_sgi_to_cpu`, `send_sgi_all_but_self` choose GICv2 or the root
   controller; `enable_local_irq` (the AP timer PPI) goes through the root
   controller for GICv3, whose `irq_enable` writes the running CPU's
   redistributor.
4. **Per-CPU state** (`device/mod.rs`): `ArchPercpuMisc.mpidr`, stored when
   the local GIC is initialized; `ipi_single()` addresses the target by MPIDR,
   `ipi()` uses the all-but-self SGI.
5. **Boot handshake** (`cpu/boot.rs`): guards use `smp_ready()`/capacity, the
   wakeup SGI goes through `send_sgi_to_cpu`.

## Two GICv3 traps found on the way

- **Redistributor stride must not use `GICD_TYPER.CPUNumber`.** The old code
  divided the region by that field. On GICv3 the field is not authoritative:
  QEMU reports 1 with four CPUs, so every frame above CPU 0 would land outside
  the region (`capacity` also came out as 1 and the boot guard refused to
  start anything). The stride is the fixed 128 KiB RD+SGI pair; the region
  length gives the capacity (QEMU: 123 slots, RK3399: exactly 6).
- **`GicV3CpuIf::irq_ack()` masked the IAR with `0x1ff`** (9 bits) instead of
  the 10-bit INTID (`0x3ff`) and `panic!`ed on the spurious id 1023. With the
  AP running, a real interrupt folded into INTID 511 and the AP aborted with
  `unexpected local IRQ 511`; a spurious read would have panicked the kernel.
  Fixed: `0x3ff`, and 1023 is returned to the caller, which already ignores
  ids ≥ 1020 without an EOI.

## QEMU validation

The rig DTB had to be regenerated with four CPUs: `-machine dumpdtb=…` with
`-smp 4`, insert the two `generic-ohci` nodes, compile with `dtc`
(`build/qemu-virt-smp4.dts`). Note dtc rejects subnodes before properties, so
the insertion goes at the end of the root node.

Run: `-M virt,gic-version=3 -smp 4 -machine dtb=build/qemu-virt-smp4.dtb`
with the desktop image. Result:

```
CPU logical #1: PSCI CPU_ON MPIDR=0x1
CPU logical #1: local SGI/timer self-test passed
CPU logical #1: online
… #2, #3 …
CPU activation complete: 4 CPUs online
BSP: 4 CPUs
```

The rig keeps running afterwards: heartbeats continue and `sendkey` still
produces keyboard reports (4 reports for two keys). No panics, no stalls.

## Image and hardware run

Image `build/aarch64/desktop-minimal/harddrive.img`,
sha256 `24b72c200b6989950948b707f7913c38be08eafce42889af7e42e9635e604fc9`
(local build 2026-11-21 19:52; includes the experiment 13 touchpad layout and
a one-shot CRU clock dump in `ohcid` — `ohcid: cru pll …` / `cru clksel_con0
…` — so the serial log answers the frequency question directly).

Expected on hardware (`RK3399`, six CPUs: MPIDR 0x0–0x3 little, 0x100/0x101
big):

```
CPU logical #1..#5: PSCI CPU_ON MPIDR=0x…
CPU logical #N: local SGI/timer self-test passed
CPU activation complete: 6 CPUs online
BSP: 6 CPUs
```

`FILL IN` — hardware result: how many CPUs come online, whether the desktop
is faster, and whether the touchpad/keyboard still behave (they did on one
core; SMP changes the IPI and timer paths).

## What this does _not_ fix

- **CPU frequency.** Redox has no cpufreq driver for RK3399: the clusters run
  at whatever U-Boot left (usually a few hundred MHz on the little cluster),
  and nothing ramps up. Raising it needs PLL changes plus RK808 voltage, i.e.
  a PMIC/I2C driver — a separate experiment, and risky without voltage
  control (README §7 risk 17).
- **Build settings.** All recipes build with `--release`; the base recipe
  sets `opt-level = "s"` (size) for the initfs drivers, userspace crates use
  the default release `-O3`.
