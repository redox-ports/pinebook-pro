# Debug instrumentation patches (temporary)

These patches are **not** part of the deliverable. They were used to locate the
first silent hang of the Redox kernel on the Pinebook Pro (experiment 4) and the
missing serial-console input (experiment 5). They are **not applied** to the build
tree anymore: the underlying issues are fixed (experiment 5, iteration 6), the
kernel logs `GICD_CTLR X -> Y` and the loopback self-test failure on its own, and a
clean image was rebuilt. Re-apply if a future bring-up needs them.

| Patch                             | Target repo           | Base commit (local clone) | Status            |
| --------------------------------- | --------------------- | ------------------------- | ----------------- |
| `kernel-early-uart-markers.patch` | `redox-os/kernel`     | `719e8f5c`                | removed from tree |
| `bootloader-uart-markers.patch`   | `redox-os/bootloader` | `f3d9261`                 | removed from tree |
| `kernel-serial-irq-dump.patch`    | `redox-os/kernel`     | `719e8f5c`                | removed from tree |

## What they do

Both patches write single ASCII markers directly to the RK3399 debug UART
(UART2, `0xff1a0000`) through volatile MMIO, bypassing EFI, the serial driver
and the logger. Each write also clears UART loopback mode (MCR = 0x03), so a
failed 16550 self-test cannot swallow the output.

Marker legend:

| Marker    | Location                                                 |
| --------- | -------------------------------------------------------- |
| `0`       | bootloader `kernel_entry` entry                          |
| `1`       | after `exit_boot_services()`                             |
| `2`       | after the EL2→EL1 drop (or when already at EL1)          |
| `3`       | after the MMU is enabled with the bootloader page tables |
| `4`       | after the kernel stack is loaded                         |
| `5`       | immediately before calling the kernel entry point        |
| `A`       | kernel `kstart` entry (before the BSS/data test)         |
| `B`       | kernel `kstart`, before `b start`                        |
| `C`       | kernel `start()` entry                                   |
| `D`       | after `args_ptr.read()`                                  |
| `E`       | after `graphical_debug::init`                            |
| `F` / `f` | after `args.dtb()` (OK / parse error)                    |
| `G`       | after `dtb::serial::init_early`                          |
| `H`       | after the first `info!` / `args.print()`                 |
| `I`       | after `startup::memory::init`                            |
| `J`       | after `paging::init`                                     |
| `K`       | after `memory::init_mm`                                  |
| `L`       | after `arch::misc::init`                                 |
| `M`       | after `allocator::init`                                  |
| `N`       | after `log::init`                                        |
| `O`       | after `dtb::init` / `device::init_devicetree`            |
| `P`       | before `startup::kmain`                                  |

The kernel markers use the physmap address `0xFFFF8000FF1A0000`, which is valid
both under the bootloader page tables (identity + `PHYS_OFFSET` mapping) and
after the kernel installs its own tables (UART2 is a registered device area).
The bootloader markers use the physical address `0xFF1A0000` (identity map).

`kernel-serial-irq-dump.patch` (experiment 5) prints the UART2 register state
(`IER`, `IIR`, `MCR`, `LSR`, `MSR`) and the GICv3 distributor state for SPI 100
(`GICD_CTLR`, `IGROUPR4`, `ISENABLER4`, `ISPENDR4`, `ICFGR8`) at serial-init
time and on the first three serial interrupts. It answers: is the receive
interrupt enabled in the UART, is the SPI enabled in the GIC, does the UART
raise an interrupt at all when keystrokes arrive. Apply it **after**
`patches/kernel/src/dtb/irqchip.rs.patch`, from the kernel `src` directory.

## How to apply locally

```sh
# in the local build tree created by scripts/bootstrap-redox.sh
git -C build/redox/recipes/core/kernel/source apply ../../../patches/debug/kernel-early-uart-markers.patch
git -C build/redox/recipes/core/bootloader/source apply ../../../patches/debug/bootloader-uart-markers.patch
make -C build/redox r.kernel r.bootloader image
```

(The paths above are relative to the repository root; adjust if the build tree
lives elsewhere via `PBP_REDOX_DIR`.)

Remove the patches once the bring-up issue is fixed; they must never reach CI or
upstream as-is.
