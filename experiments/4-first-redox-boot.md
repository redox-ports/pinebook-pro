# Experiment 4 — First boot of the Redox image from microSD

**Status:** in progress (2026-11-20) — **the kernel boots to userspace**: GICv3, timer,
serial and the initfs `init` all run; `vesad`/`fbbootlogd` drive the 1920x1080 panel
(the yellow screen with kernel log text) and userspace logs reach UART2. Root RedoxFS
mount fails (no storage driver), `pcid` panics in the x86-only PCI fallback, and the
capture repeats after `init: switchroot to /usr /etc` — confirm whether that is a reboot
loop.
**Related:** README §3 (image layout), §4 (local loop), §6 Phase 3; experiment 0 (UART wiring), experiments 2–3 (eMMC boot chain)

## Goal

Boot the locally built, stock-layout Redox aarch64 image (`harddrive.img`, minimal
config) on the Pinebook Pro from microSD and capture the whole chain on UART2
(1500000 8N1). No repacking is needed for this test: the U-Boot already on the
eMMC (2021.07-1 Manjaro ARM, EFI distro boot) scans the microSD ESP at sector
2048 and starts `/EFI/BOOT/BOOTAA64.EFI`.

```
U-Boot (eMMC) -> EFI distro boot -> microSD ESP: /EFI/BOOT/BOOTAA64.EFI (Redox UEFI bootloader)
             -> Redox kernel (DTB-driven UART2) -> serial console
```

Success levels to record:

1. U-Boot finds the microSD ESP and starts `BOOTAA64.EFI`;
2. Redox bootloader hands over to the kernel — its banner may be visible **only
   on the built-in display**: experiment 0 reported U-Boot `Out: vidconsole`, so
   the EFI text console may not mirror to UART;
3. kernel output appears on UART2 (the kernel talks on the serial directly);
4. serial login prompt — full success (parity with the RPi 3 B+ precedent).

## Prerequisites

- Image built on the dev host: `build/redox/build/aarch64/minimal/harddrive.img`
  (`scripts/build-image.sh`).
- microSD flashed (`scripts/flash-sd.sh`, the `SU32G` card) and inserted into the
  Pinebook Pro. The eMMC stays untouched.
- USB-UART adapter wired as in experiment 0 (tip = TX, ring 1 = RX, **3.3 V**),
  UART switch towards the display hinges.
- Adapter visible on the dev host (`scripts/serial-log.sh --list`).

## Actions

On the dev host, before powering the laptop on:

```sh
# 1. Sanity check the image layout (safe: reads the image file, not the card)
sgdisk -p build/redox/build/aarch64/minimal/harddrive.img
# Optional (needs mtools): list the ESP content; ESP starts at 1 MiB
mdir -i build/redox/build/aarch64/minimal/harddrive.img@@1M ::/EFI/BOOT 2>/dev/null || true

# 2. Flash the card, then start the console capture BEFORE power-on
scripts/flash-sd.sh
scripts/serial-log.sh
```

Power on and let autoboot run. Expected markers:

- `Hit any key to stop autoboot` — do **not** press anything;
- distro boot scanning messages for `mmc1`, then something like
  `Found EFI removable media binary /efi/boot/bootaa64.efi` (wording varies by
  U-Boot version);
- then either Redox output, or silence with the chain stalled — record the last
  visible line either way.

If autoboot does not pick the card, interrupt it and run from the `=>` prompt:

```
mmc dev 1
ls mmc 1:1 /
ls mmc 1:1 efi/boot
setenv boot_targets mmc1
run distro_bootcmd
```

Everything lands in `logs/serial-<timestamp>.log` automatically; note the path in
Results.

## Results

### Image layout (pre-flight)

```
PASTE sgdisk -p OUTPUT HERE
```

### ESP content (optional, mtools)

```
PASTE mdir OUTPUT HERE OR WRITE "mtools not installed"
```

### Boot log

```
U-Boot TPL 2021.07-1 (Jul 12 2021 - 13:27:45)
Channel 0: LPDDR4, 50MHz
BW=32 Col=10 Bk=8 CS0 Row=15 CS1 Row=15 CS=2 Die BW=16 Size=2048MB
Channel 1: LPDDR4, 50MHz
BW=32 Col=10 Bk=8 CS0 Row=15 CS1 Row=15 CS=2 Die BW=16 Size=2048MB
256B stride
lpddr4_set_rate: change freq to 400000000 mhz 0, 1
lpddr4_set_rate: change freq to 800000000 mhz 1, 0
Trying to boot from BOOTROM
Returning to boot ROM...

U-Boot SPL 2021.07-1 (Jul 12 2021 - 13:27:45 +0000)
Trying to boot from MMC1


U-Boot 2021.07-1 (Jul 12 2021 - 13:27:45 +0000) Manjaro ARM

SoC: Rockchip rk3399
Reset cause: POR
Model: Pine64 Pinebook Pro
DRAM:  3.9 GiB
PMIC:  RK808
MMC:   mmc@fe310000: 2, mmc@fe320000: 1, sdhci@fe330000: 0
Loading Environment from SPIFlash... SF: Detected gd25q128 with page size 256 Bytes, erase size 4 KiB, total 16 MiB
*** Warning - bad CRC, using default environment

In:    serial
Out:   vidconsole
Err:   vidconsole
Model: Pine64 Pinebook Pro
Net:   No ethernet found.
starting USB...
Bus usb@fe3a0000: USB OHCI 1.0
Bus usb@fe3e0000: USB OHCI 1.0
scanning bus usb@fe3a0000 for devices... 2 USB Device(s) found
scanning bus usb@fe3e0000 for devices... 4 USB Device(s) found
       scanning usb for storage devices... 0 Storage Device(s) found
Hit any key to stop autoboot:  0

Device 0: unknown device
switch to partitions #0, OK
mmc1 is current device
Scanning mmc 1:2...
libfdt fdt_check_header(): FDT_ERR_BADMAGIC
Card did not respond to voltage select! : -110
Scanning disk mmc@fe310000.blk...
Disk mmc@fe310000.blk not ready
Scanning disk mmc@fe320000.blk...
** Unrecognized filesystem type **
** Unrecognized filesystem type **
Scanning disk sdhci@fe330000.blk...
Found 7 disks
** Unable to read file ubootefi.var **
Failed to load EFI variables
BootOrder not defined
EFI boot manager: Cannot load any image
Found EFI removable media binary efi/boot/bootaa64.efi
163328 bytes read in 23 ms (6.8 MiB/s)
libfdt fdt_check_header(): FDT_ERR_BADMAGIC
Booting /efi\boot\bootaa64.efi

WARN - Failed to get EFI EDID from handle Handle(4109867680): Status(0x8000000000000003) "unsupported"
INFO - Currently in EL2
Redox OS Bootloader 1.0.0 on aarch64/UEFI
Hardware descriptor: DeviceTree(f3eb4000, 11000)
Looking for RedoxFS:
RedoxFS 091a3795-66ba-43e2-a5ad-c0ac79056194: 126 MiB

Output 0, best resolution: 1920x1080
Arrow keys and enter select mode
Press l to  enable live mode
Press e to edit boot environment

 1920x1080  16:9

usr/lib/boot/kernel: 1/1 MiB
usr/lib/boot/initfs: 22/22 MiB
kernel_entry(0xf3ed5000, 0xffff8000f3da9000, 0xffffff000000a6e8, 0xf4f29c50)
KernelArgs {
    kernel_base: 0xf3c1d000,
    kernel_size: 0x16baf0,
    stack_base: 0xf3d89000,
    stack_size: 0x20000,
    env_base: 0xf0f8d000,
    env_size: 0x134,
    acpi_rsdp_base: 0xf3eb4000,
    acpi_rsdp_size: 0x11000,
    areas_base: 0xf3f02050,
    areas_size: 0x6000,
    bootstrap_base: 0xf0f9d000,
    bootstrap_size: 0x1640000,
}

[... silence on the uninstrumented build: the kernel produced no UART output ...]
```

### Boot log — marker build (second capture)

```
012345ABCDEFkernel::dtb::serial:INFO -- UART Compatible { string: "rockchip,rk3399-uart\0snps,dw-apb-uart" } at 0xFFFF8000FF1A0000 size 0x100
Gkernel::arch::aarch64::start:INFO -- Redox OS starting...
kernel::startup:DEBUG -- Kernel: F3C1D000:F3D88768
kernel::startup:DEBUG -- Env: F0F8D000:F0F8D134
kernel::startup:DEBUG -- HWDESC: F3EB4000:F3EC5000
kernel::startup:DEBUG -- Areas: F3F02038:F3F08038
kernel::startup:DEBUG -- Bootstrap: F0F9D000:F25DD000
Hkernel::startup::memory:DEBUG -- Registering Reclaim memory F3ED5000 size 1000
[... per-area Reclaim/Free/Reserved registration and per-area mapping lines ...]
kernel::startup::memory:INFO -- Memory: 3911 MB
kernel::startup::memory:INFO -- Highest huge page level: 0
[... Contiguously mapping lines, one per memory area ...]
kernel::startup::memory:DEBUG -- Table: 200000
kernel::startup::memory:DEBUG -- 256: 201403
kernel::startup::memory:DEBUG -- 510: 9AC403
kernel::startup::memory:INFO -- Permanently used: 7936 KB
IJkernel::acpi:ERROR -- NO RSDP FOUND
kernel::memory:DEBUG -- Initial freelist consistent
KLMNkernel::arch::aarch64::device:INFO -- PSCI INIT
kernel::arch::aarch64::device::psci:INFO -- PSCI 1.1 using smc conduit from device tree
kernel::arch::aarch64::device:INFO -- CPU TOPOLOGY INIT
kernel::arch::aarch64::device::cpu::topology:INFO -- DT CPU topology: 6 enabled, 0 disabled, BSP MPIDR=0x0, 5 PSCI secondaries
kernel::arch::aarch64::device::cpu::topology:DEBUG -- CPU topology logical #0: MPIDR=0x0 boot=true enable-method=psci
kernel::arch::aarch64::device::cpu::topology:DEBUG -- CPU topology logical #1: MPIDR=0x1 boot=false enable-method=psci
kernel::arch::aarch64::device::cpu::topology:DEBUG -- CPU topology logical #2: MPIDR=0x2 boot=false enable-method=psci
kernel::arch::aarch64::device::cpu::topology:DEBUG -- CPU topology logical #3: MPIDR=0x3 boot=false enable-method=psci
kernel::arch::aarch64::device::cpu::topology:DEBUG -- CPU topology logical #4: MPIDR=0x100 boot=false enable-method=psci
kernel::arch::aarch64::device::cpu::topology:DEBUG -- CPU topology logical #5: MPIDR=0x101 boot=false enable-method=psci
kernel::arch::aarch64::device::cpu::boot:INFO -- BSP translation-table walks are inner-shareable: TCR_EL1=0x12b5103510
kernel::arch::aarch64::device::cpu::boot:INFO -- CPU boot resources: 5 PSCI slots ready, trampoline=0xf6f18000, idmap=0xf6f19000
kernel::arch::aarch64::device::cpu::boot:DEBUG -- CPU boot slot logical #1: MPIDR=0x1 stack_phys=0xf6f20000..0xf6f30000 percpu_phys=0xf6f1d000 args_phys=0xf6f1e000
kernel::arch::aarch64::device::cpu::boot:DEBUG -- CPU boot slot logical #2: MPIDR=0x2 stack_phys=0xf6f30000..0xf6f40000 percpu_phys=0xf6f1f000 args_phys=0x8000000
kernel::arch::aarch64::device::cpu::boot:DEBUG -- CPU boot slot logical #3: MPIDR=0x3 stack_phys=0x8010000..0x8020000 percpu_phys=0x8001000 args_phys=0x8002000
kernel::arch::aarch64::device::cpu::boot:DEBUG -- CPU boot slot logical #4: MPIDR=0x100 stack_phys=0x8020000..0x8030000 percpu_phys=0x8003000 args_phys=0x8004000
kernel::arch::aarch64::device::cpu::boot:DEBUG -- CPU boot slot logical #5: MPIDR=0x101 stack_phys=0x8030000..0x8040000 percpu_phys=0x8005000 args_phys=0x8006000
kernel::arch::aarch64::device:INFO -- IRQCHIP INIT
kernel::dtb:DEBUG -- root parent = 0x00000001
kernel::dtb:DEBUG -- interrupt-controller@fee00000, compatible = arm,gic-v3, #interrupt-cells = 0x00000004, phandle = 0x00000001
kernel::dtb:DEBUG -- gpio0@ff720000, compatible = rockchip,gpio-bank, #interrupt-cells = 0x00000002, phandle = 0x00000021
kernel::dtb:DEBUG -- gpio1@ff730000, compatible = rockchip,gpio-bank, #interrupt-cells = 0x00000002, phandle = 0x00000086
kernel::dtb:DEBUG -- gpio2@ff780000, compatible = rockchip,gpio-bank, #interrupt-cells = 0x00000002, phandle = 0x00000016
kernel::dtb:DEBUG -- gpio3@ff788000, compatible = rockchip,gpio-bank, #interrupt-cells = 0x00000002, phandle = 0x00000080
kernel::dtb:DEBUG -- gpio4@ff790000, compatible = rockchip,gpio-bank, #interrupt-cells = 0x00000002, phandle = 0x000000cb
kernel::dtb::irqchip:DEBUG -- interrupt-controller@fee00000, compatible = arm,gic-v3, #interrupt-cells = 0x00000004, phandle = 0x00000001
KERNEL PANIC: panicked at src/dtb/irqchip.rs:133:48:
called `Result::unwrap()` on an `Err` value: MissingRequiredProperty("interrupt-cells")
  FP ffffff000010ab80: PC ffffff00000a8dd4
    FFFFFF00000A8C9C+0138
    kernel::panic::panic_handler_inner
[... panic stack trace ...]
  FP ffffff000010b2c0: PC ffffff0000012bd4
    FFFFFF00011E1C+0DB8
    kernel::dtb::irqchip::init
  FP ffffff000010b7c0: PC ffffff0000042b80
    FFFFFF0000042488+06F8
    kernel::arch::aarch64::device::init_devicetree
  FP ffffff000010ba10: PC ffffff0000079228
    FFFFFF0000078344+0EE4
    kernel::arch::aarch64::start::start
 ffffff000010bd70: EMPTY RETURN
CPU #0, CID <none>
HALT
```

### Boot log — third capture (GICv3 works, timer panic)

Earlier output is identical to the second capture (minor address shifts), through
`IRQCHIP INIT`. The new part:

```
kernel::arch::aarch64::device:INFO -- IRQCHIP INIT
kernel::dtb:DEBUG -- root parent = 0x00000001
kernel::dtb:DEBUG -- interrupt-controller@fee00000, compatible = arm,gic-v3, #interrupt-cells = 0x00000004, phandle = 0x00000001
kernel::dtb:DEBUG -- gpio0@ff720000, compatible = rockchip,gpio-bank, #interrupt-cells = 0x00000002, phandle = 0x00000021
[... gpio1-gpio4 ...]
kernel::dtb::irqchip:DEBUG -- interrupt-controller@fee00000, compatible = arm,gic-v3, #interrupt-cells = 0x00000004, phandle = 0x00000001
kernel::dtb::irqchip:DEBUG -- gpio0@ff720000, compatible = rockchip,gpio-bank, #interrupt-cells = 0x00000002, phandle = 0x00000021
kernel::arch::aarch64::device::irqchip:WARN -- no driver for interrupt controller "rockchip,gpio-bank"
[... same warning for gpio1-gpio4 ...]
kernel::arch::aarch64::device::irqchip::gic:INFO -- gic: Distributor supports 6 CPUs and 288 IRQs
kernel::arch::aarch64::device::irqchip::gicv3:INFO -- GicV3 { gic_dist_if: GicDistIf { address: FFFF8000FEE00000, ncpus: 6, nirqs: 120 }, gic_cpu_if: GicV3CpuIf, gicrs: [(FEE00000, 10000)], irq_range: (0, 0) }
kernel::arch::aarch64::device::irqchip::gicv3:INFO -- gic irq_range = (0, 288)
kernel::arch::aarch64::device:INFO -- select 0 as root ic
kernel::arch::aarch64::device:INFO -- GIT INIT
kernel::arch::aarch64::device::generic_timer:DEBUG -- generic_timer use_virtual_timer = true
KERNEL PANIC: panicked at src/dtb/mod.rs:598:27:
called `Result::unwrap()` on an `Err` value: MissingRequiredProperty("interrupt-cells")
  FP ffffff000010ad90: PC ffffff000005a328
    FFFFFF000005A16C+01BC
    kernel::arch::aarch64::device::generic_timer::init
  FP ffffff000010b000: PC ffffff00000828d0
    FFFFFF0000081E08+0AC8
    kernel::arch::aarch64::device::init_devicetree
  FP ffffff000010b260: PC ffffff0000059c38
    FFFFFF00000595D8+0660
    kernel::arch::aarch64::start::start
 ffffff000010b4e0: EMPTY RETURN
CPU #0, CID <none>
HALT
```

### Boot log — fourth capture (userspace)

After the timer fix, the kernel boots into userspace. Key excerpt (full log lives in
`logs/serial-*.log`):

```
kernel::arch::aarch64::device:generic_timer:DEBUG -- generic_timer use_virtual_timer = true
kernel::arch::aarch64::device:generic_timer:DEBUG -- architected timer irq = L3(1, 11, 8)
kernel::arch::aarch64::device:generic_timer:INFO -- architected timer virq = 27 (virtual)
kernel::arch::aarch64::device:INFO -- SERIAL INIT
kernel::arch::aarch64::device:serial:INFO -- serial_port virq = 132
kernel::arch::aarch64::device:INFO -- RTC INIT
kernel::arch::aarch64::device::rtc:WARN -- No PL031 RTC found
OPkernel::startup:INFO -- SECONDARY CPU INIT
kernel::arch::aarch64::device::cpu::boot:ERROR -- CPU activation unavailable: BSP has no directed-SGI target mask
kernel::scheme::acpi:WARN -- expected RXSDT_ENUM to be initialized before AcpiScheme, is ACPI available?
kernel::startup:DEBUG -- BSP: 1 CPUs
kernel::startup:DEBUG -- Env: Ok("DTB_ADDR=...\nDTB_SIZE=...\nREDOXFS_BLOCK=0000000000000000\nREDOXFS_UUID=429b8388-4b91-4ee7-9ed1-2fa89f4bd847\nFRAMEBUFFER_ADDR=00000000f7800000\n...\nFRAMEBUFFER_WIDTH=0000000000000780\nFRAMEBUFFER_HEIGHT=0000000000000438\nFRAMEBUFFER_STRIDE=0000000000000780\n")
kernel::syscall::process:DEBUG -- Bootstrap entry point: 3000
init: switchroot to /scheme/initfs /scheme/initfs/etc
randd: Seeding failed, no entropy source.  Random numbers on this platform are NOT SECURE
vesad: 1920x1080 stride 1920 at 0xF7800000
Finished graphical debug
fbbootlogd: mapped display
1970-01-01T00-00-18.205Z [@hwd:18 INFO] using devicetree backend
1970-01-01T00-00-18.223Z [@pcid::cfg_access:251 WARN] Couldn't retrieve PCIe info ... FDT error: Custom { kind: NotFound, error: "couldn't find pci-host-ecam-generic node in device tree" }
thread 'main' (1) panicked at drivers/pcid/src/cfg_access/fallback.rs:88:9:
not yet implemented: Pci::CfgAccess::read on this architecture
kernel::arch::aarch64::interrupt::exception:ERROR -- FATAL: Not an SVC induced synchronous exception (ty=111100)
FAR_EL1: 0x25000
ELR_EL1: 0000000000436C88
ESR_EL1: 00000000F2000001
kernel::context::signal:INFO -- UNHANDLED EXCEPTION, CPU #0, PID 15, NAME /scheme/initfs/bin/pcid, CONTEXT 21
daemon: INIT_NOTIFY="7" "pcid" exited without notifying readiness
init: env -i ... "pcid-spawner" "--initfs" failed with exit status: 1
[ERROR redoxfs] not able to mount uuid 429b8388-4b91-4ee7-9ed1-2fa89f4bd847
init: env -i ... "redoxfs" "--uuid" "429b8388-..." "file" "0000000000000000" failed with exit status: 1
init: switchroot to /usr /etc

[the entire kernel startup then repeats in the same capture]
```

### Notes (facts extracted from the log)

- **Milestone 1 confirmed:** U-Boot distro boot found the SD ESP — `Found EFI removable
media binary efi/boot/bootaa64.efi`, `163328 bytes read in 23 ms (6.8 MiB/s)`,
  `Booting /efi\boot\bootaa64.efi`. The stock (un-repacked) image layout works with the
  eMMC U-Boot as predicted.
- **Milestone 2 confirmed — and on UART:** `Redox OS Bootloader 1.0.0 on aarch64/UEFI`.
  The pre-run expectation that the bootloader banner might be display-only (`Out:
vidconsole`) was **wrong**: the bootloader's output reaches the serial console.
  `INFO - Currently in EL2` — the bootloader starts at EL2 (per README, it drops to EL1).
- **DTB is discovered:** `Hardware descriptor: DeviceTree(f3eb4000, 11000)` — the
  bootloader received U-Boot's FDT (address `0xf3eb4000`, size `0x11000` = 69,632 B).
  This is the `DTB_ADDR`/`DTB_SIZE` path from README §2 working end-to-end.
- **RedoxFS is recognized on the SD:** `RedoxFS 091a3795-66ba-43e2-a5ad-c0ac79056194:
126 MiB` (= `filesystem_size` 256 MiB − ESP 128 MiB − GPT overhead).
- **GOP works:** `Output 0, best resolution: 1920x1080` — U-Boot exposes a framebuffer
  with the native panel mode; the Phase 4 display path via `FRAMEBUFFER_*` is viable.
- **The chain stopped at the bootloader menu** (` 1920x1080  16:9`): it waits for the
  mode selection. U-Boot reported `In: serial`, so EFI console input should be the serial
  port — Enter can be pressed from the USB-UART terminal on the dev host.
- Scan noise, all harmless and expected: `Device 0: unknown device` (usb0, no storage),
  `Scanning mmc 1:2...` + `FDT_ERR_BADMAGIC` (probing p2 for a boot script), `Card did
not respond to voltage select! : -110` (mmc2 = SDIO Wi-Fi init failure),
  `** Unrecognized filesystem type **` (RedoxFS/ext4 partitions are not FAT for the
  U-Boot file loader), `ubootefi.var` / `BootOrder not defined` (default environment —
  the SPI env holds no valid env, experiment 0).
- **Kernel handover happened, then silence.** After Enter the bootloader loaded
  `usr/lib/boot/kernel` (1 MiB) and `usr/lib/boot/initfs` (22 MiB) from RedoxFS and
  called `kernel_entry(...)`; the kernel produced no output, at any time.
- Ruled out by source inspection (U-Boot v2021.07 / Redox master):
  - `/chosen/stdout-path = "serial2:1500000n8"` **is** present in
    `rk3399-pinebook-pro.dts`, so the kernel's DTB serial lookup has a target;
  - `uart_16550::init()` programs divisor 1, which on the RK3399 24 MHz UART clock is
    exactly **1500000 baud** — the rate is not silently changed;
  - the bootloader and kernel `KernelArgs` layouts match (`repr(C, packed(8))`, same
    fields), so the handover structure is not misread.
- Remaining candidates: a hang **before** `dtb::serial::init_early` (early asm,
  `graphical_debug`, DTB parse), a failed UART loopback self-test that leaves the port
  in loopback (logs exist but never reach the pin), or logging not yet active at the
  hang point.
- **Instrumentation prepared** (`patches/debug/`): raw UART2 markers in `kernel_entry`
  (`0`–`5`) and in kernel `kstart`/`start` (`A`–`P`), written through MMIO with
  loopback cleared. The image has been rebuilt with the markers; the next capture
  pinpoints the hang.
- **First marker run (invalid) and the resulting fix.** The first marker build produced
  `012345ABC`, but the `ABC` part was caused by the instrumentation itself: the marker
  calls inserted in `kstart` pass the byte in `w0` via `bl`, clobbering `x0` — the
  `KernelArgs` pointer handed over by the bootloader. `start()` therefore received
  `args_ptr = 66` and faulted in `args_ptr.read()` before marker `D`. The `kstart`
  markers now preserve `x0` in `x19` around the calls
  (`patches/debug/kernel-early-uart-markers.patch` regenerated) and the image was
  rebuilt. The valid, load-bearing result of this run is the bootloader prefix
  `012345`: `exit_boot_services`, the EL2→EL1 drop, MMU enable, stack setup and the
  call into the kernel all succeed.
- **Second capture (fixed markers): the kernel boots and panics in IRQ init.** The kernel
  reached userspace initialization: `Redox OS starting...`, memory map init (`Memory:
3911 MB`, `Permanently used: 7936 KB`), ACPI lookup (`NO RSDP FOUND` — expected for a
  DTB system), GICv3 discovery, PSCI 1.1 (`smc` conduit), CPU topology (6 CPUs, 5 PSCI
  secondaries), IRQ chip init — then
  `KERNEL PANIC ... src/dtb/irqchip.rs:133:48 ... MissingRequiredProperty("interrupt-cells")`
  followed by `HALT`.
- **Panic root cause:** the U-Boot DT (`rk3399.dtsi`) describes the root interrupt
  controller as `arm,gic-v3` with `#interrupt-cells = <4>` **and** a maintenance
  interrupt `interrupts = <GIC_PPI 9 IRQ_TYPE_LEVEL_HIGH 0>`. `IrqChipList::init_inner1`
  calls `node.property::<Interrupts>()` for every `interrupt-controller` node; the fdt
  parser needs the interrupt parent's `#interrupt-cells`, and a root controller has none,
  so the kernel panics (`fdt::parsing::Panic`). This is an upstream kernel limitation;
  RK3399 is simply the first tested DT with this shape.
- **Fix applied:** parse `interrupts` only when a parent with `#interrupt-cells` exists
  (`patches/kernel-gicv3-root-interrupts.patch`); root controllers are handled as roots
  with no parents, as intended. Kernel and image rebuilt; next capture pending.
- **Third capture: GICv3 works, next panic in the timer.** The GICv3 driver initialized
  (`gic: Distributor supports 6 CPUs and 288 IRQs`, `gic irq_range = (0, 288)`,
  `select 0 as root ic`) and `generic_timer` started (`use_virtual_timer = true`), then:
  `KERNEL PANIC ... src/dtb/mod.rs:598:27 ... MissingRequiredProperty("interrupt-cells")`
  in `get_interrupt` from `generic_timer::init`.
- **Root cause:** the DT makes `interrupt-parent` inheritable; the root node carries
  `interrupt-parent = <&gic>`, and `/timer` declares only `interrupts = <GIC_PPI 11 ...>`.
  The pinned `fdt` crate resolves a missing `interrupt-parent` to the **immediate parent
  node** (`/`) and then looks for `#interrupt-cells` there — missing → panic. It also
  cannot decode GICv3's four-cell specifiers.
- **Fix applied:** `get_interrupt` now walks ancestors to find the inherited
  `interrupt-parent`, resolves the phandle, parses the legacy `interrupts` property
  manually (1–4 cells; the fourth PPI-partition cell is ignored), and keeps the
  `interrupts-extended` path via the crate. The four call sites pass the `Fdt` explicitly
  (`patches/kernel-inherited-interrupt-parent.patch`). Kernel and image rebuilt; next
  capture pending.
- **GICv3, not GICv2:** the log shows `arm,gic-v3` (GIC-500) — README §2 and `AGENTS.md`
  claimed GICv2/GIC-400 compatibility; corrected. Note that `irqchip/mod.rs::init_ap`
  currently supports only the memory-mapped GICv2 interface ("GICv3 remains on the safe
  single-CPU fallback path"), so secondary CPUs stay offline until that is extended.
- **Secondary finding:** `kernel::dtb:WARN -- failed to find /soc in devicetree` — the
  U-Boot `rk3399.dtsi` really has no `/soc` container (devices are flat under the root),
  so `register_dev_memory_ranges` only registers the root GIC and the diagnostic UART as
  device memory. Needs a follow-up before storage/input drivers rely on MMIO mappings.
- **UART mystery:** with the marker build the kernel prints; the uninstrumented build was
  silent. The marker helper clears UART loopback (MCR) before every byte, so a failed
  16550 loopback self-test leaving the port in loopback remains the leading explanation
  for the earlier silence. To be re-tested with an uninstrumented build after the panic
  fixes.
- **Fourth capture — userspace reached.** After the timer fix the kernel continues:
  `architected timer virq = 27 (virtual)`, `serial_port virq = 132`, `BSP: 1 CPUs`,
  then `init: switchroot to /scheme/initfs` and userspace services start
  (`randd`, `vesad`, `fbbootlogd`, `hwd`). The bootloader env is consumed end-to-end
  (`DTB_ADDR`, `REDOXFS_UUID/BLOCK`, `FRAMEBUFFER_*`).
- **Display works:** `vesad: 1920x1080 stride 1920 at 0xF7800000` and
  `fbbootlogd: mapped display`; the yellow panel with log text is the kernel
  `graphical_debug` output drawn through the GOP framebuffer (`FRAMEBUFFER_ADDR`
  `0xf7800000`, 1920x1080, stride 1920).
- **Secondary CPUs stay offline:** `CPU activation unavailable: BSP has no
directed-SGI target mask` — expected: `irqchip::init_ap` implements only the GICv2
  memory-mapped path; the kernel continues on the BSP.
- **`pcid` panics (non-fatal):** the U-Boot DT has no `pci-host-ecam-generic` node, so
  `pcid` falls back to PCI 3.0 port I/O, whose aarch64 implementation is
  `todo!()` (`cfg_access/fallback.rs:88`) → panic and an unhandled-EL0 exception dump.
  `init` treats the spawner as a weak dependency and continues.
- **Root mount fails (the real blocker):** `[ERROR redoxfs] not able to mount uuid
429b8388-...` — there is no block-device driver for the microSD (DW MMC), so the
  RedoxFS daemon cannot read the partition and `init: switchroot to /usr /etc` fails.
- **The capture then repeats the whole kernel startup.** Confirm whether the board
  reboots automatically (init exit → reset) or the capture simply contains a manual
  power cycle; this determines whether we have a boot loop.
- **RTC:** `No PL031 RTC found` — RK3399 has an RK808 RTC, no driver yet; benign for
  the console milestone.

### U-Boot prompt session (only if autoboot failed)

```
PASTE HERE OR WRITE "not needed"
```

### Observations

| Question                                                                      | Answer                                                                                                                   |
| ----------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| U-Boot found the microSD ESP (`Found EFI removable media binary ...`)?        | **Yes** — `efi/boot/bootaa64.efi`, 163328 bytes read in 23 ms, `Booting /efi\boot\bootaa64.efi`                          |
| `BOOTAA64.EFI` started (Redox bootloader banner — on display and/or on UART)? | **Yes, on UART** — `Redox OS Bootloader 1.0.0 on aarch64/UEFI`; also `INFO - Currently in EL2`                           |
| Kernel and initfs loaded from RedoxFS?                                        | **Yes** — `usr/lib/boot/kernel: 1/1 MiB`, `usr/lib/boot/initfs: 22/22 MiB`                                               |
| Kernel output visible on UART2?                                               | **Yes** (marker build) — full kernel log; the uninstrumented build was silent (loopback TBD)                             |
| Kernel reaches userspace (initfs / `init`)?                                   | **Yes** — `init: switchroot to /scheme/initfs`; `init`, `randd`, `vesad`, `fbbootlogd`, `hwd` run                        |
| Framebuffer / display working?                                                | **Yes** — `vesad: 1920x1080 stride 1920 at 0xF7800000`, `fbbootlogd: mapped display`                                     |
| Serial login prompt reached?                                                  | No — root RedoxFS mount fails (no microSD storage driver), then `switchroot` fails                                       |
| Where did the chain stop (last visible line before silence)?                  | `init: switchroot to /usr /etc`; the capture then repeats the kernel startup (reboot loop or manual reboot — to confirm) |
| Log file path                                                                 | `FILL IN`                                                                                                                |

### Verdict

`FILL IN` — but the hardware bring-up has moved well past the original goal: the
kernel, initfs userspace, UART and the 1920x1080 display all work. The remaining
blockers for a usable system are storage (root RedoxFS mount) and x86-centric
userspace bits (`pcid`).

### Follow-up (next milestone): storage and console

1. **MicroSD storage driver (DW MMC)** — the real blocker: without it the RedoxFS
   partition cannot be mounted and `init` cannot switchroot. This is the Phase 4
   driver work; the eMMC (SDHCI/arasan) is the easier controller to model but must
   stay untouched/read-only.
2. **Interim console:** disable or fix `pcid` (its aarch64 fallback is `todo!()`), and
   either add `getty` + `login` + a shell to the initfs or make init drop to a debug
   shell when the root mount fails — useful for poking at schemes before storage
   exists. **Done via the `debugsh` initfs daemon; see experiment 5.**
3. **Confirm the repeat:** check whether the board reboots by itself after
   `switchroot to /usr /etc` fails (boot loop) or the capture contains a manual power
   cycle.
