# Experiment 8 — USB input: OHCI host driver for the internal keyboard

**Date:** 2026-11-20 (autonomous work session)
**Goal:** make the built-in keyboard usable. The keyboard and touchpad are
internal USB HID devices (`258a:001e`) behind a Genesys hub (`05e3:0608`), so
this needs a USB host controller driver, hub enumeration and HID report
parsing — none of which exist for this platform in Redox.

**Related:** experiment 4 (boot log with the U-Boot USB scan), README §6
Phase 4, risk 8; `patches/base-pinebook-pro.patch` (`drivers/usb/ohcid`).

## Where the devices are (verified)

The U-Boot boot log captured in experiment 4 (file
`logs/serial-20261120-001949.log`) shows:

```
starting USB...
Bus usb@fe3a0000: USB OHCI 1.0
Bus usb@fe3e0000: USB OHCI 1.0
scanning bus usb@fe3a0000 for devices... 2 USB Device(s) found
scanning bus usb@fe3e0000 for devices... 4 USB Device(s) found
```

Three conclusions:

1. **Only OHCI buses are registered.** There is no `USB EHCI 1.00` line, even
   though all four host nodes (`usb_host0/1_ehci/ohci`) are `status = "okay"` in
   U-Boot's `rk3399-pinebook-pro.dts`. **Inferred:** the EHCI halves either
   failed to probe or are not built into this U-Boot (`CONFIG_USB_EHCI_HCD`),
   and since U-Boot enumerated the hub and its devices on the OHCI bus, the
   whole tree is running at full speed there. For Redox this means an OHCI
   driver alone is enough — EHCI split transactions are not needed. The exact
   reason is not verified; only the observed state matters here.
2. `usb@fe3e0000` is **`usb_host1_ohci`** (the internal hub: 4 devices);
   `usb@fe3a0000` is **`usb_host0_ohci`** (2 devices). The EHCI companions are
   `usb@fe3c0000` and `usb@fe380000` respectively (`rk3399.dtsi`; Linux and
   U-Boot agree). The DT compatible string is `generic-ohci`, not a
   Rockchip-specific one.
3. The driver does **not** depend on that inference being the whole story: if a
   port were routed to EHCI, OHCI would simply report no connection and the
   log would say so (`port N status 0x...`, no `(connected)`).

## Clock and PHY bring-up (U-Boot does not leave USB running)

U-Boot stops its USB stack before it starts the EFI payload. On the Pinebook
Pro that leaves two visible pieces of state behind:

- **The RK3399 CRU USB gates are closed.** `dwmmcd` reads
  `CLKGATE_CON(6)` as `0x00000060` on hardware (`logs/serial-20261120-112448.log`),
  and bits 5/6 of that register are `clk_usb2phy0_ref`/`clk_usb2phy1_ref`
  (`CLK_GATE_SET_TO_DISABLE`: a set bit means the clock is off). The same
  applies to the host bus gates in `CLKGATE_CON(20)` bits 5-8
  (`hclk_host0`/`_arb`, `hclk_host1`/`_arb`).
- **The USB2 PHYs are powered down.** Linux drives them through the GRF
  (`phy-rockchip-inno-usb2.c`, `rk3399_phy_cfgs`): 480 MHz output clock enable
  is bit 4 of the block base (`0xe450`/`0xe460`), and the host port suspend
  field is bits [1:0] at base + 8 (`2` = running, `1` = suspended).

Accessing an OHCI register window while its bus clock is gated is not safe on
this SoC — the first read never returns, which is exactly what the first
hardware run showed: the serial log ended at `ohcid: starting` and nothing
else ran (no `dwmmcd`, no `vesad`), i.e. the whole machine stalled, not just
the driver.

The driver therefore does, in this order, before it touches the controller:

1. Map the CRU and **un-gate** `CLKGATE_CON(6)` bits 5/6 and
   `CLKGATE_CON(20)` bits 5-8 (hiword-masked writes, the Rockchip
   convention), then read both registers back and log them.
2. **Bail out** if a gate still reads as closed — the machine stays usable and
   the log says why, instead of freezing on an unclocked access.
3. Power both USB2 PHY host ports back on through the GRF (480 MHz clock out,
   leave suspend) and log the register state before/after.
4. Only then map the OHCI registers and read `HcRevision`.

## Emulation coverage

The driver can be exercised under QEMU even though `virt` has no RK3399 OHCI:
when the devicetree has no `generic-ohci` node it runs a **DMA self-test**
(allocate 4 KiB through `common::dma`, write/read back a pattern) and then
tries the known addresses anyway, which fail cleanly. Measured with
`u-boot-qemu` + `-M virt,gic-version=3`:

```
[   0.002] ohcid: starting
[   0.006] ohcid: devicetree 8192 bytes
[   0.008] ohcid: no OHCI nodes in the devicetree, trying the known RK3399 addresses
[   0.008] ohcid: selftest: allocating 4 KiB of DMA memory
[   0.009] ohcid: selftest: DMA ok, phys 0x4324c000 virt 0x00015000
[   0.009] ohcid: selftest: DMA readback 0x5a 0xa5
[   0.010] ohcid: 0xfe3e0000: probing
[   0.010] ohcid: 0xfe3e0000: mapping the register window
kernel::context::memory:WARN -- Driver tried to physmap the allocator-frame [frame at 0xfe3e0000]!
[   0.011] ohcid: 0xfe3e0000 failed: Protocol("cannot map OHCI registers")
```

Two useful facts from that run:

- `common::dma` works on aarch64 (uncacheable, physically contiguous, VA→PA
  translation included), so a hardware hang cannot be blamed on the DMA path.
- The kernel refuses to physmap a frame that has page info. On `virt` the RAM
  reaches 0x1_0000_0000, so `0xfe3e0000` is inside the frame table and the map
  fails; on the Pinebook Pro RAM ends at `0xf8000000`, so the same call
  **succeeds**. This is why the failure looks different on the two machines
  and why the hardware run got further than the emulated one.

## Design

New initfs daemon `drivers/usb/ohcid` (static binary, `40_ohcid.service`,
`condition_architecture = ["aarch64"]`, part of `40_drivers.target`).

- **Node selection:** `reg` address `0xfe3e0000` (internal hub) and
  `0xfe3a0000`, cross-checked against the `generic-ohci` nodes in the
  devicetree when it is available. If the DTB has no such nodes the driver
  falls back to the known addresses; it never probes by index.
- **Registers:** mapped with `common::physmap(..., MemoryType::DeviceMemory)`,
  layout and bit fields from the OHCI 1.0a spec and Linux
  `drivers/usb/host/ohci.h` (identical to U-Boot's `ohci.h`).
- **Descriptors:** `common::dma::Dma` gives physically contiguous, uncached
  memory and both the virtual pointer and the physical address. The HCCA is one
  256-byte page-aligned block, and the ED/TD/buffer pools are allocated once
  (16 EDs, 64 TDs, 4 KiB of buffers) because a single keyboard needs only a
  handful.
- **No interrupts:** `HcInterruptEnable = 0`. Completion is detected by polling
  the transfer descriptor's condition code (`TD_CC` is initialised to
  `NotAccessed`, so `CC != 0xF` means the controller retired the TD), exactly
  the trick Linux uses for TD takeback. This avoids wiring the OHCI interrupt
  through the GIC, which is still single-CPU-only on this port.
- **No sleeps:** every delay is a busy-wait on `CLOCK_MONOTONIC`, because the
  kernel does not wake sleeper threads on the generic timer (README risk 13).
- **Control transfers:** setup TD (DATA0), optional data TD (DATA1, `R`), status
  TD in the opposite direction (DATA1), chained to a dummy TD which terminates
  the queue — the same structure as `td_submit_urb()` in `ohci-q.c`. The control
  list is enabled, kicked with `CLF`, and disabled again after each transfer, so
  no done-queue bookkeeping is needed.
- **Enumeration:** root ports are powered if the controller requires it, reset
  through `HcRhPortStatus` (polling `PRSC`), then the standard sequence at
  address 0: `GET_DESCRIPTOR(device, 8)` → `SET_ADDRESS` → full device
  descriptor → configuration descriptor → `SET_CONFIGURATION`.
- **Hubs:** a device whose `bDeviceClass` is 9 is configured and queried for its
  hub descriptor; each connected port is powered, reset, its change bits are
  acknowledged, and the downstream device is enumerated (recursion depth 1 —
  the internal tree is hub → keyboard/touchpad).
- **HID:** the first interface with an interrupt IN endpoint is used. A boot
  keyboard (`protocol = 1`) is switched to the boot protocol and its idle rate
  is set to 0. The 8-byte report is diffed against the previous one, modifier
  bits become usages `0xE0..0xE7`, and every usage is mapped to the same
  `orbclient` scancode table `usbhidd` uses. Events go to `/scheme/input`
  through `inputd::ProducerHandle`; `inputd` fills `character` from its keymap
  and tracks Shift, so the driver only sends key down/up.
- **Scheduling:** the interrupt ED is placed in one slot of the 32-entry HCCA
  periodic table, which the controller walks one entry per frame. The keyboard
  is therefore sampled every ~32 ms and is re-armed after each completed
  transfer (toggle carry preserved in `ED.HeadP` bit 1).

## Boot-time behaviour

`daemon.ready()` is called **before** any hardware access, so USB bring-up can
never block `init` (notify-type services block the scheduler). If the input
scheme is missing the driver exits after 5 s; if no keyboard is found it stays
alive and idle.

## Test protocol (hardware)

1. Build and flash as usual: `scripts/build-image.sh`, `scripts/flash-sd.sh`,
   then `scripts/serial-log.sh`.
2. Boot the Pinebook Pro. Expected serial lines (timings relative to the
   driver start):

```
[...  0.xxx] ohcid: starting
[...  0.xxx] ohcid: devicetree 69632 bytes
[...  0.xxx] ohcid: devicetree OHCI nodes: 0xfe3a0000 0xfe3e0000
[...  0.xxx] ohcid: cru gates inherited: con6=0x00000060 con20=0x000001e0
[...  0.xxx] ohcid: cru gates now: con6=0x00000000 con20=0x00000000
[...  0.xxx] ohcid: u2phy inherited: clkout0=0x... host0=0x... clkout1=0x... host1=0x...
[...  0.xxx] ohcid: u2phy now: ...
[...  0.xxx] ohcid: 0xfe3e0000: probing
[...  0.xxx] ohcid: 0xfe3e0000: mapping the register window
[...  0.xxx] ohcid: 0xfe3e0000: registers at 0x...
[...  0.xxx] ohcid: 0xfe3e0000: hcca phys 0x...
[...  0.xxx] ohcid: 0xfe3e0000: reading HcRevision
[...  0.xxx] ohcid: 0xfe3e0000: OHCI revision 0x00000010
[...  0.xxx] ohcid: 0xfe3e0000: 1 port(s)
[...  0.xxx] ohcid: 0xfe3e0000: operational, control 0x... rh_a 0x...
[...  0.xxx] ohcid: 0xfe3e0000: port 1 status 0x... (connected)
[...  0.xxx] ohcid: 0xfe3e0000: port 1 reset (connected true, low-speed false, enabled ...)
[...  0.xxx] ohcid:   root device addr 1 id 05e3:0608 class 09/00/02 mps0 64 full-speed
[...  0.xxx] ohcid:   hub with 4 port(s)
[...  0.xxx] ohcid:   hub port N: device attached (status 0x..., full-speed, enabled true)
[...  0.xxx] ohcid:   hub device addr 2 id 258a:001e class 00/00/00 mps0 8 full-speed
[...  0.xxx] ohcid:   HID interface 0 protocol 1 endpoint 0x81 mps 8 interval 10
[...  0.xxx] ohcid:   polling keyboard addr 2 endpoint 1 via periodic slot 0
[...  0.xxx] ohcid: 0xfe3e0000: 1 HID device(s)
[...  0.xxx] ohcid: polling 1 keyboard(s)
```

The exact values differ (the DT size and the register contents depend on
U-Boot and the SoC), but the order of the lines is what localises a failure.
If `cru gates now` still shows a set bit, the driver stops on purpose and the
machine keeps running — the log then ends with
`ohcid: usb clocks still gated (0x...), not touching the controllers`.

3. Log in on the serial console (`root`/`password`), then press keys on the
   built-in keyboard and check that they appear on the VT/panel (or use the
   `debugsh` console before the login prompt).

Anything shorter than the list above pinpoints the failing stage; the log line
to report is the last `ohcid:` line plus whatever error follows it.

## Known limitations (first iteration)

- Only **one** HID keyboard per controller is used (the first found); the
  touchpad is recognised but its report parser is not implemented yet.
- Sampling is ~32 ms (one periodic slot); very short key presses could be
  missed.
- Addresses 1..127 are handed out without a free-address search; a re-plug
  after enumeration is not handled, and there is no hotplug/rescan.
- **Clocks and PHY:** the driver programs what it needs (CRU gates, USB2 PHY
  host ports) because U-Boot leaves both switched off; see the section above.
  It does **not** re-tune the PHY (no PLL/termination programming) — it only
  re-enables the reference/480 MHz clocks and leaves suspend, which assumes
  U-Boot's PHY configuration is still valid.
- Report descriptors are not parsed: a device that does not implement the boot
  protocol is assumed to use the 8-byte boot layout.

## Status

- Driver written, wired into the initfs and **built into the image** (commit
  contains the crate, the service unit and the target dependency).
- **First hardware run (serial log `logs/serial-20261120-135201.log`) froze the
  machine** right after `ohcid: starting`: no `dwmmcd`, no `vesad`, no login.
  Root cause analysis above; the CRU/PHY bring-up and step-by-step logging are
  the fix under test.
- The DMA path and the DTB handling are verified under QEMU; the RK3399 clock
  and PHY sequence needs the next hardware run.
