# `redox-os/kernel` changes

- Upstream: https://github.com/redox-os/kernel (mirror of https://gitlab.redox-os.org/redox-os/kernel.git)
- Base commit: `719e8f5c`
- Apply with: `scripts/apply-patches.sh kernel`

A `<path>.patch` file is a diff against the upstream `<path>`; every other
file is new and is copied verbatim (see `../README.md`).

## Modified files

- `src/arch/aarch64/device/generic_timer.rs`
- `src/arch/aarch64/device/irqchip/gic.rs`
- `src/arch/aarch64/device/irqchip/gicv3.rs`
- `src/arch/aarch64/device/irqchip/irq_bcm2835.rs`
- `src/arch/aarch64/device/serial.rs`
- `src/arch/aarch64/interrupt/exception.rs`
- `src/arch/riscv64/device/serial.rs`
- `src/context/signal.rs`
- `src/devices/graphical_debug/debug.rs`
- `src/devices/graphical_debug/mod.rs`
- `src/devices/serial.rs`
- `src/devices/uart_16550.rs`
- `src/dtb/irqchip.rs`
- `src/dtb/mod.rs`

## GICv3 root interrupt controller (devicetree parse)

Files: `src/dtb/irqchip.rs`

The RK3399 DT (U-Boot `rk3399.dtsi`) puts `interrupts = <GIC_PPI 9 ...>` on the root GICv3 node. `src/dtb/irqchip.rs` parses `interrupts` with `Node::property::<Interrupts>()`, whose parser requires the interrupt parent's `#interrupt-cells`; a root controller has none, so the kernel panics with `MissingRequiredProperty("interrupt-cells")` during IRQ chip init (experiment 4). The patch parses `interrupts` only for controllers that declare an explicit `interrupt-parent` (or `interrupts-extended`); root controllers are then handled as roots, as intended.

## Inherited `interrupt-parent` resolution

Files: `src/arch/aarch64/device/generic_timer.rs`, `src/arch/aarch64/device/irqchip/irq_bcm2835.rs`, `src/arch/aarch64/device/serial.rs`, `src/arch/riscv64/device/serial.rs`, `src/dtb/mod.rs`

The DT spec makes `interrupt-parent` inheritable, but the pinned `fdt` crate resolves a missing `interrupt-parent` to the immediate parent node and then looks for `#interrupt-cells` there. On RK3399 only the root node carries `interrupt-parent = <&gic>`, so parsing `/timer`'s `interrupts` panics with the same error during generic-timer init. The patch resolves the inherited parent by walking ancestors, parses the legacy `interrupts` property directly (1–4 cells; the GICv3 fourth PPI-partition cell is ignored for now), and updates the four `get_interrupt` call sites to pass the `Fdt`.

## NS16550 receive interrupt (UART2 console)

Files: `src/devices/serial.rs`, `src/devices/uart_16550.rs`

Two driver bugs that keep the RK3399 UART2 console from receiving input (experiment 5). (1) The NS16550 loopback self-test fails on this board, and `uart_16550::init()` returned early on failure — skipping the final `modem_ctrl = 0x0F` / `int_en = 0x01` writes, so the port kept loopback enabled and `IER = 0`. (2) `init()` enabled the FIFO with a 14-byte receive threshold (`FCR = 0xC7`); one keystroke never reaches it and this UART has no usable character-timeout interrupt, so no receive interrupt is ever raised (Linux 8250 uses a one-character trigger). The patch records the self-test outcome instead of returning early, always restores MCR/IER, logs the failure, uses `FCR = 0x07`, and implements `SerialKind::enable_irq` for the NS16550 variants (previously a documented TODO no-op).

## GICv3 SPI delivery: group 1 NS, IROUTER, GICD_CTLR

Files: `src/arch/aarch64/device/irqchip/gic.rs`

`GicDistIf::init()` never programmed `GICD_IGROUPR`, `GICD_IROUTER`, and — the decisive bug found with register dumps (experiment 5, iterations 4–6) — overwrote `GICD_CTLR` with `0x3`, wiping the firmware-set `ARE_S` bit (`0x20`). With affinity routing dropped on one side, Group 1 NS SPIs (UART2, IRQ 132) stayed pending at the distributor forever and never reached the CPU, while the timer PPI kept working. The patch switches `GICD_CTLR` to read-modify-write (`ctlr` changed from 0x3 to 0x23), marks every SPI non-secure Group 1, and routes every SPI to the boot CPU (`IROUTER = 0`, affinity 0.0.0.0). All writes are harmless on GICv2 (RAZ/WI) and on a single-security-state GIC.

## Framebuffer bit depth (RGB565)

Files: `src/devices/graphical_debug/debug.rs`, `src/devices/graphical_debug/mod.rs`

The firmware framebuffer is not always 32 bits per pixel: U-Boot v2021.07 programs the RK3399 VOP in RGB565 for the eDP panel (`rk_vop.c` forces `VIDEO_BPP16` for `VOP_MODE_EDP`) and reports `PixelBitMask` `0xf800/0x07e0/0x001f`. `graphical_debug` wrote 32-bit ARGB pixels into that surface and reserved `stride*height*4` bytes, which appeared on the panel as a black/yellow dither (experiment 7). The patch reads `FRAMEBUFFER_BPP` (default 32, so existing platforms are unchanged), converts glyph colours to RGB565, clamps the last text row to the display height and reserves the real size.

## Signals are delivered after the syscall result is stored

Files: `src/arch/aarch64/interrupt/exception.rs`, `src/context/signal.rs`

`signal_handler()` runs from `sched_yield()` and from `context::switch::tick()`, i.e. while a syscall is still executing. It records `regs.sig_archdep_reg()` (x0) and the resume address (the instruction after `svc`) for the userspace signal trampoline, but the arch syscall epilogue writes the result into `scratch.x0` only after `syscall()` returns. A signal delivered in that window therefore saved the syscall *argument* as its return value, and `sigreturn` made the caller resume with it: relibc's `Sys::verify()` yield was entered with x0 = -1 and resumed with x0 = -1, so uutils binaries took the `b.hs -> brk #1` abort path at startup and died silently (experiment 10; the trap site was pinned by deriving the module bases from the dump: executable 0x1000, `libc.so` 0xa80000). The patch makes `signal_handler()` refuse to run while `percpu.inside_syscall` is set (`being_sigkilled` is still honoured first) and calls it from the aarch64 syscall epilogue right after `stack.scratch.x0 = ret`. Timer-tick delivery in userspace is unchanged.

## GICv3 redistributor: PPIs/SGIs (architected timer)

Files: `src/arch/aarch64/device/irqchip/gicv3.rs`

The architected timer PPI (27) never fired, so nothing preempted, `nanosleep` never returned, scheme timeouts never expired and the VT only repainted when input caused a syscall (experiment 11). Two defects in the GICv3 driver. (1) `GicV3::parse()` created a fresh `reg()` iterator per entry, so `chunk.iter().next()` returned the *GICD* chunk `#redistributor-regions` times — the boot log showed `gicrs: [(FEE00000, 10000)]`, i.e. the distributor instead of the RK3399 redistributor region at `fef00000` (six 0x20000 frames) — and stored the frames without `PHYS_OFFSET`. (2) The redistributor was never programmed: in GICv3 SGIs/PPIs are banked per CPU, so their enable/group/priority/trigger configuration lives in `GICR_ISENABLER0`/`GICR_IGROUPR0`/`GICR_IPRIORITYR`/`GICR_ICFGR1` of the redistributor's SGI frame and the frame has to be woken via `GICR_WAKER`; `GICD_ISENABLER` only covers SPIs, so `irq_enable(27)` wrote a RES0 bit and silently enabled nothing, while every SPI (UART2) kept working. The patch keeps one iterator for the redistributor region, wakes the current CPU's frame, marks PPIs Group 1 NS with priority 0xa0 and level trigger, and routes `irq_enable`/`irq_disable` for ids below 32 to `GICR_ISENABLER0`/`GICR_ICENABLER0`. Verified in QEMU: `sleep(10ms)`/`sleep(250ms)` return after 10.9 ms/250.2 ms, where before a 10 ms `nanosleep` never returned.
