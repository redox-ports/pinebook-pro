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
[...  0.xxx] ohcid: 0xfe3e0000: OHCI revision 0x10, 1 port(s)
[...  0.xxx] ohcid: 0xfe3e0000: operational, control 0x... rh_a 0x...
[...  0.xxx] ohcid: 0xfe3e0000: port 1 status 0x00000101 (connected)
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
- Clocks and USB2 PHYs are inherited from U-Boot; the driver does not program
  the CRU or the RK3399 PHY registers. This works because U-Boot already set
  them up, and it is documented as a known dependency.
- Report descriptors are not parsed: a device that does not implement the boot
  protocol is assumed to use the 8-byte boot layout.

## Status

- Driver written, wired into the initfs and **built into the image** (commit
  contains the crate, the service unit and the target dependency).
- **Hardware verification pending** — the test above has not been run yet.
