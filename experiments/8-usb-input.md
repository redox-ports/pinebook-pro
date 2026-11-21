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

## Iteration 2 — bring-up works, enumeration does not (2026-11-20 23:07)

The deliverable image (`logs/serial-20261120-230717.log`) answers the open question:

- **No SoC freeze.** `cru gates inherited: con6=0x00000060 con20=0x00000000` → after
  the fix `con6=0x00000000 con20=0x00000000`; `u2phy` host suspend fields move to `2`
  (running). Both controllers probe, read OHCI 1.0, go operational, and the root ports
  report connected devices. The unclocked-access stall is gone.
- **Enumeration fails on both ports, differently**:
  - `fe3e0000` (internal hub, full-speed): reset ok, then `enumeration failed: Halted`
    ~33 ms later — the first `GET_DESCRIPTOR(device, 8)` at address 0 ended in a
    STALL/ED-halt.
  - `fe3a0000`: low-speed device, reset ok, then `Timeout("transfer descriptor")` —
    the TD was never accessed; the device never saw a valid transaction.
- U-Boot enumerated these same devices minutes earlier in the same boot
  (`2 USB Device(s)` on `fe3a0000`, `4` on `fe3e0000`), so the hardware, the ports and
  the devices are fine; the defect is in our driver's protocol logic.

Next: reproduce the enumeration in emulation — QEMU provides `pci-ohci`, `usb-hub` and
`usb-kbd`, and the packaged U-Boot binds them (`In: serial,usbkbd`). The plan: boot the
smoke setup with `pci-ohci` attached, expose it to the DT-driven driver through a
patched copy of QEMU's own DTB (`-machine dtb=`), and debug the TD/ED/request sequence
at seconds per iteration instead of one flash per guess. Two small driver changes are
needed for that: the CRU/PHY step must run only on RK3399-compatible machines (it is
RK3399-specific but currently runs whenever any OHCI node exists), and the probe list
must come from the devicetree (today only the two hardcoded RK3399 addresses are
probed). Neither touches the enumeration logic under test.

## Iteration 3 — the QEMU rig works, the bug reproduces, the evidence is contradictory

(2026-11-20 late evening, continued past midnight)

The emulation rig is fully working (all steps are reproducible from the repo):

1. Two driver changes make ohcid place-independent: the CRU/PHY bring-up runs only when
   the devicetree names the RK3399 controllers, and the probe list comes from the
   devicetree (fallback to the hardcoded addresses when there is no node).
2. QEMU provides `pci-ohci`, `usb-hub`, `usb-kbd`; the packaged U-Boot enumerates them
   (`In: serial,usbkbd`). The DT-driven driver is connected by patching a copy of
   QEMU's own DTB (`-machine dumpdtb` → add a `generic-ohci` node at the BAR address →
   `-machine dtb=…`); the BAR (0x10100000) is read from the QEMU monitor
   (`info pci`). Support scripts: `build/qmon.py` (monitor socket), `build/rig_freeze.py`
   (freeze + RAM dump), DTB sources `build/qemu-virt-rig.dts`.
3. Topology mirrors the laptop: hub on root port 1, keyboard on hub port 2
   (`-device usb-kbd,bus=ohci0.0,port=1.2`).

**The bug reproduces exactly**: `port 1 enumeration failed: Halted` ~20 ms after the
reset — the same signature as the hardware hub port. The defect is in protocol logic
or in the memory path, not in RK3399 silicon. Iteration cost is ~2 minutes.

### The evidence trail (QEMU `-trace usb_ohci_td*`, monitor `xp`, driver diagnostics)

- The trace shows the controller transmitting **a SETUP packet of eight zero bytes**
  (`usb_ohci_td_pkt_full OUT data: 00 00 …`), so the emulated hub answers STALL and
  halts the ED — the `Halted` explained.
- The driver's own view of the same buffer shows the correct packet
  (`80 06 00 01 00 00 08 00` = GET_DESCRIPTOR(device, 8)).
- Freezing the VM inside the diagnostic delay (buffer written, TD not yet processed)
  and dumping guest RAM through the monitor (`xp`) at the TD's cbp shows **the packet
  already present in RAM before the DMA read**; the dump is stable across 12 samples.
- After resuming, the DMA read of the **same guest-physical address** (cbp printed by
  the trace equals the address printed by the driver) still returns zeros.
- A 100 ms and a 5 s pause between the CPU write and the list start change nothing.
- Moving the buffers into the tail of the TD page (whose contents the controller reads
  correctly) changes nothing.
- `info mtree` shows a single flat RAM region (0x40000000-0x13fffffff) in both the
  system and the PCI DMA address space — no overlapping region at the buffer address.

So: guest RAM contains the packet, the guest CPU sees the packet, and the emulated
controller's DMA read of the same address sees zeros — in TCG, which does not model
caches at all. This is logically impossible for a correct emulation and is not
explained by anything in our driver; the remaining suspects are on the QEMU side
(Debian qemu-system-aarch64 version/patches, `pci-ohci` + PCIe-root quirks) — or a
kernel-side virt→phys divergence that both the monitor and the trace would have to
share, which the address consistency (driver-printed cbp == trace-printed cbp) makes
unlikely.

### Cross-checks (all confirm the anomaly is systematic)

- **QEMU 8.2.2 (noble build, unpacked into `build/qemu82-root`, run with
  `LD_LIBRARY_PATH`)** reproduces the identical behaviour (zeros on the wire, STALL,
  Halted) — so this is not a bug of one specific 10.2.1 build.
- **Writeback instead of Uncacheable** for the DMA mapping (throwaway edit in the
  build tree, reverted) — identical result. The MAIR attribute is not the trigger of
  the QEMU-side anomaly.
- **Full single-run protocol** (`build/rig_protocol.py`): `xp` during the diagnostic
  delay shows the packet at the TD's cbp (six samples, stable); after the failure the
  same address still shows the packet; the trace between those two dumps recorded the
  DMA read of the same address as eight zero bytes followed by STALL.
- `info mtree` shows a single flat RAM region (0x40000000-0x13fffffff) in both the
  system and the PCI DMA address space — no overlapping region at the buffer address.

The QEMU-side anomaly is left open (report upstream with this protocol if it keeps
blocking emulation work). The hardware side needs its own answer — see the next
iteration.

## Iteration 5 — root cause found: the control ED direction field (2026-11-21)

With the QEMU rig frozen mid-enumeration (trace + monitor + serial cross-checks) the
cause of every failure finally surfaced — a one-bit difference from U-Boot/Linux:

1. **The control ED carried a direction (D = IN/OUT).** U-Boot's `ed_fill` sets
   `D = 0` for control endpoints (the direction belongs to each TD, and SETUP only
   exists as TD_DP_SETUP). Our ED had D = IN for GET_DESCRIPTOR. The QEMU model —
   `ohci_service_td` — uses `ED_D` as the transaction direction whenever it is
   non-zero, so an IN-ED with a SETUP TD transmitted **an IN token instead of
   SETUP**; the hub answered STALL (the `Halted`), the low-speed device never
   answered (the `Timeout`). The real RK3399 OHCI behaves the same way. Fixed:
   control EDs get `D = 0`.
2. **The hub descriptor was requested with bmRequestType 0x80.** Hub descriptors
   are a _class_ request: bmRequestType must be 0xA0. Spec-compliant hubs STALL a
   standard GET_DESCRIPTOR aimed at type 0x29 (the QEMU hub does; the Genesys hub
   on the laptop would too). Fixed: `get_class_descriptor` (0xA0).
3. **The periodic ED registration wrote the HCCA through its physical address**
   (`hcca.physical() as *mut u32`) — userspace cannot dereference that; the fault
   (`Lacks grant`, FAULT at the HCCA physical address) only surfaced after the
   enumeration finally reached `start_hid`. Fixed: write through the virtual
   mapping (`hcca.virt_addr()`).
4. **The shared buffer arena was never reused** — control transfers allocate
   setup/data buffers from a finite pool marked "allocated once, never reused", so
   enumeration ran out of buffer space (`OutOfMemory`) after a few requests.
   Fixed: the arena restarts after each completed transfer (single-threaded, one
   transfer at a time, list stopped).
5. **DC IVAC is EL1-restricted** (SCTLR_EL1.UCT): the data-cache invalidate added
   in iteration 4 faulted from userspace. Switched to `dc civac` (clean+
   invalidate), which is EL0-accessible.

With all five fixes the QEMU rig **enumerates the whole tree** and reaches the
poll loop: hub descriptor via the class request (`hub with 8 port(s)`), port
status/reset cycles, keyboard behind the hub (`id 0627:0001`), boot protocol set,
interrupt ED armed, `polling 1 keyboard(s)` — in ~0.5 s after init, no crashes.

Hardware check pending (one flash). The QEMU DMA anomaly of iteration 3 (RAM shows
the packet, the trace shows a zero-read) is still unexplained — it vanished as a
_symptom_ once the ED direction was fixed: with D = 0 the SETUP transaction is
correct and the trace shows the real packet bytes, so the anomaly was part of the
wrong-direction path (the model services the ED with a mismatched direction in a
way that leaves usb_buf stale), not a memory-model bug.

## Iteration 4 — cache maintenance in the driver, and the decisive hardware test

(2026-11-21)

Regardless of the QEMU anomaly, the real device can still fail from cache
incoherence: the RK3399 OHCI does not snoop CPU caches, and the mapping is only
non-cacheable if every layer honours the request (the rmm aarch64 code looks correct:
`ENTRY_FLAG_UNCACHEABLE = MEM_ATTR_NC << 2`, wired through `PageFlags::uncacheable`
into the PTE attribute field — but it has never been verified on this hardware).

The driver now maintains the caches itself, which is both a candidate fix and a
diagnostic: `dc cvac` + `dsb sy` over the ED page, the TD page (descriptors and
buffers) and the HCCA before the controller touches them, and `dc ivac` over a data
buffer before the CPU reads back IN data (`cache_clean_range`/`cache_invalidate_range`
in `ohci.rs`; EL0-accessible instructions, no-ops on non-aarch64 builds).

**Test built, image sha256 `8839c619…`, hardware check pending** (one flash):

- if enumeration starts working (`root device … polling N keyboard(s)`) — the
  uncacheable mapping does not work on this kernel/hardware combination, and the
  driver-side cache maintenance is the practical fix (plus a kernel report);
- if it still fails with `Halted`/timeout — cache coherence is ruled out on hardware
  too, and the next step is comparing the wire-level behaviour with U-Boot's OHCI
  driver sequence register by register.

Diagnostic leftovers kept for this stage: the `dbg:` lines in the serial log, the
5-second delay between the buffer write and the list start, and the buffer arena
living in the TD page tail (all marked DIAGNOSTIC in the source).

### State and next steps

- Uncommitted driver state: diagnostics (`dbg:` lines), the diagnostic delay before
  the list start, the buffer relocation into the TD page, and the two
  place-independence changes. Keep for the next iteration.
- Next: (a) try a different QEMU version/build (upstream binary or `-M virt,highmem=off`)
  to falsify the emulator hypothesis; (b) check what memory type the aarch64 kernel
  actually applies to `zeroed@uc?phys_contiguous` mappings (the Dma code requests
  Uncacheable — verify it lands in the page flags on this path); (c) if the emulator
  hypothesis is confirmed, debug the hardware side directly: the RK3399 OHCI does not
  snoop caches, so the primary suspect for the real hardware failure is the same
  zeros-on-the-wire symptom caused by cache incoherence — test by flushing/invalidating
  the buffer range in the driver (DC CVAC/IVAC is EL0-accessible) or by making the
  buffer mapping device-typed.
- The keyboard stays deferred until enumeration works; everything else in the
  deliverable image is hardware-verified (iteration 2 of experiment 12).

## Iteration 6 — hardware crash in cache maintenance, phys-as-virt

(2026-11-21)

The image with all five fixes (sha256 `972c4c46…`, log
`logs/serial-20261121-123248.log`) reached `redox login:` with the whole root
filesystem working, but ohcid died on the **first control transfer** of
`fe3a0000`:

```
ohcid:   dbg: setup phys 0x0329a400 bytes [80, 06, 00, 01, 00, 00, 08, 00]
kernel::context::memory:DEBUG -- Lacks grant
FATAL: Not an SVC induced synchronous exception
FAR_EL1: 0x3299000   X0: 0x3299000  X1: 0x10
```

Symbolising the crash against the built binary (`readelf`: ET_EXEC at vaddr
`0x400000`, so runtime PCs are ELF vaddrs; `addr2line` on `0x409060`) gives an
exact stack: `relibc_crt0 → main → daemon → enumerate_at_zero → control_transfer
→ cache_clean_range → FAULT`. X0/X1 match the call
`cache_clean_range(ed_phys(0), size_of::<Ed>())` = `(0x3299000, 0x10)`.

Root cause: **`dc cvac` translates its operand**. The ED pool address was
passed as the *physical* address (`ed_phys()`), which is not a mapped VA of the
ohcid context — on real hardware the translation fails and the CPU raises a
data abort at EL0 ("Lacks grant" in the kernel dump). The kernel kills the
driver; the boot continues without input devices. The companion TD-page clean
in the same function correctly used the Dma `virt_addr()`; only the two ED
cleans (control path and `arm_interrupt`) passed `ed_phys()`.

Why the QEMU rig never caught it: **QEMU TCG does not perform a translation for
cache-maintenance-by-VA instructions** — `dc cvac` to an unmapped address is
silently ignored. The rig enumerates the whole tree with the broken address, so
this bug class (phys-as-virt in cache maintenance) is invisible in emulation.
Only the real SoC walks the page tables for the operand. (This also explains
why the kernel-side trap surfaces as a data abort with FAR = the phys address
rather than anything USB-related.)

Fix: clean the ED through its Dma virtual mapping,
`cache_clean_range(self.eds.virt_addr() + ed_index * size_of::<Ed>(), …)`, in
both `control_transfer` and `arm_interrupt` (`ohci.rs`). Every other cache call
site already used the virtual address (HCCA, TD page, data buffers).

Note on the 11:04 run (iteration 5's image `8839c619…`): it executed the same
first transfer *without* faulting, which means that image was built from a
mid-session tree that predates the `cache_clean_range(ed_phys())` lines —
the "cache hypothesis" test image and the committed a4d2672 state were not
identical. The `Halted`/`Timeout` results of that run still correctly
identified the ED-direction bugs (the direction fix made the new fault
reachable on the first transfer).

Hardware check pending (one flash, image rebuilt with the fix): expected
`root device …`, `hub with N port(s)` on `fe3e0000` and `polling N
keyboard(s)`; the low-speed device on `fe3a0000` may still fail enumeration
(for it, any timeout now reflects the device, not the driver).

## Iteration 7 — keyboard works on hardware; first keystroke; hub stall

(2026-11-21)

The fixed image (sha256 `17405451…`, log `logs/serial-20261121-125955.log`)
**enumerates and drives the internal keyboard on hardware.** The keyboard
(`258a:001e`, low-speed) turned out to sit **directly on `fe3a0000`
(`usb_host0_ohci`)** — not behind the Genesys hub: full enumeration
(`root device addr 1 id 258a:001e … low-speed`), boot protocol, and
`polling 1 keyboard(s)`; the owner typed the login name on the laptop keyboard
and it reached the VT.

Two follow-ups:

1. **The first keystroke was lost** ("user" arrived as "ser"). Cause: the
   driver sends `SET_IDLE` with duration 0, so the keyboard reports **only on
   state change** — the first interrupt report after arming is the first key
   press, not an idle snapshot. `handle_report` discarded the first report
   ("current idle state" assumption) and ate it. Fix: no first-report
   swallowing; the diff runs against the initial all-keys-up state, which is
   correct for both a state-change report and a genuine idle snapshot.

2. **`fe3e0000` (Genesys hub, touchpad behind it) still stalls** on the first
   `GET_DESCRIPTOR`: setup TD retires CC=0 (8 bytes transmitted and ACKed),
   data TD STALLs, and — the strange part — the setup buffer in the tail of
   the TD page reads back as **zeros** through the driver's own Dma mapping,
   while the TDs at the head of the **same page** were delivered to the HC
   correctly (it walked the chain with our cbp/be values and retired the TDs).
   The 11:04 run's read-back still showed the packet, so something between
   the two builds changed whether that write survives; the write path itself
   is identical.

   Not reproducible in emulation: the QEMU rig now has **two** `pci-ohci`
   controllers (second BAR at `0x10100100`, unaligned — the driver now maps
   page-aligned base + offset, which hardware never needs but tolerates), with
   the hub on the second controller — the whole tree enumerates and
   `polling 2 keyboard(s)`. So the failure is hardware/controller-specific,
   and the buffer-zeros symptom does not involve the driver's protocol logic.

   Diagnostics added for the next hardware run (`Ohci::diag_setup_buffer`,
   triggered only when the first get_descriptor fails): the buffer virtual
   address, a pattern **written and read back** through the Dma mapping
   (discriminates a write-broken mapping from a lost one-off write), the same
   area through an independent `physical@uc` physmap (RAM refuses `dev`;
   catches cache-attribute and virt-to-phys divergence), a canary at an
   untouched tail offset, and the first TD as the CPU sees it.

   Hypotheses still open: lost/stale store to the buffer area only (cache
   attribute of the mmap'd DMA page on this kernel?), an interconnect quirk of
   `usb_host1` reading that DRAM range, or the real hub stalling for its own
   reasons with the zeros being an artifact of the read-back path.

## Iteration 8 — first keystroke fixed; hub stall is not the mapping

(2026-11-21)

The image with the first-report fix (sha256 `b78cd2a2…`, log
`logs/serial-20261121-134028.log`) — **the keyboard now delivers the first
keystroke**: the serial log echoes the complete `user` at the login prompt
(previous boots show it swallowing the first character). Login, shell prompt,
`exit` and re-login all typed on the internal keyboard.

The `fe3e0000` (Genesys hub) failure is unchanged and deterministic (fourth
boot in a row), and the new diagnostics resolved several hypotheses at once:

```
buf va 0x0003f400 phys 0x0334f400
write-rb [5a a5 3c c3 0f f0 55 aa]   <- pattern write + readback: works
canary [c3 3c a5 5a]                 <- untouched tail offset: works
td0 [03000000, 0, 0, 0334f407]       <- retired TD visible to the CPU
phys-view None                        <- physmap of RAM refused (dev and uc)
```

- The Dma mapping is **not write-broken**: both a pattern write into the
  setup area and a canary at another tail offset read back correctly through
  the same virtual address.
- The disassembly of `buf_slice_mut`/`control_transfer` confirms the setup
  store goes to `tds.virt + 0x400 + off` — the same address the diagnostics
  use; there is no second address to mix up.
- The CPU sees the HC's TD writebacks through the mapping, so the page is
  shared correctly between CPU and controller.
- `physical@dev` and `physical@uc` both refuse to map the RAM page, so the
  independent phys view stays unavailable.

What remains: the hub stalls the data stage of the very first
`GET_DESCRIPTOR` while its firmware may still be settling after the port
reset (~10 ms after reset ends; U-Boot scans it orders of magnitude later).
The keyboard on `fe3a0000` — a simple low-speed device — answers immediately.
Next step, built into the next image: retry the first get_descriptor up to
three times with 150 ms pauses, and keep the original buffer content in the
diagnostics (the iteration-7 pattern write destroyed it before it could be
read).

## Iteration 9 — the hub wakes up: retry works, port power order fixed

(2026-11-21)

The retry image (sha256 `394e3d71…`, log `logs/serial-20261121-141101.log`)
**enumerates the Genesys hub on `fe3e0000`**:

```
[   0.799] first get_descriptor attempt 1 failed Timeout("transfer descriptor"), retrying after 150 ms
[   1.354] step1 ok (first [12, 01, 00, 02, 09, 00, 00, 40]), mps0 64
[   1.368] root device addr 1 id 05e3:0608 class 09/00/00 mps0 64 full-speed
[   1.370] hub with 4 port(s)
```

- The settle theory was right: the first attempt now fails with a **Timeout**
  (the td dump shows untouched TDs — CC never accessed, the device simply did
  not answer while its firmware booted), and 150 ms later the hub responds
  normally. The earlier `Halted` + setup-buffer-zeros signature was the same
  unreadiness seen through the old ED-direction bug and is now moot.
- The hub identifies as `05e3:0608` — the internal Genesys hub from the U-Boot
  scans, MPS0 64, 4 downstream ports.

But the four hub ports reported nothing connected, and the reason is in the
driver's port loop: it checked PORT_CONNECTION **before** powering the port.
With all four ports unpowered (the hub implements individual power
switching), CCS reads 0 for every port and the enumeration loop skips them
all. U-Boot does the opposite — `usb_hub_power_on()` powers **every** port
first, waits `max(100 ms, 2*bPwrOn2PwrGood)`, and only then polls for
connections (plus a 1 s connect window).

Fix (same shape as U-Boot): after reading the hub descriptor, power all ports
up front, wait `max(100 ms, 2*hub[5])` (bPwrOn2PwrGood), then walk the ports
and enumerate whatever is attached. The hub-descriptor length check was
raised from 3 to 6 bytes so `hub[5]` is really there.

QEMU rig: unchanged behaviour (`hub with 8 port(s), power-on 100 ms` — the
emulated hub reports bPwrOn2PwrGood 0 — keyboard behind the hub still
enumerates, `polling 2 keyboard(s)`).

Expected on the next hardware run: devices behind the Genesys hub (touchpad
and whatever else U-Boot sees there) finally appear; the keyboard polling
path is already proven on `fe3a0000`.

## Iteration 10 — devices behind the hub enumerate; laptop console freeze

(2026-11-21)

The port-power image (sha256 `f07db21f…`, log `logs/serial-20261121-142243.log`)
**enumerates the whole internal USB tree**:

```
hub with 4 port(s), power-on 100 ms
hub port 1: device attached  -> 046d:c52f (Logitech, touchpad), protocol 2
                                         "boot mouse interface, report parser not implemented yet"
hub port 2: device attached  -> 0c45:6321 (Sonix webcam), no HID, ignored
```

The keyboard path is unchanged and worked through a full serial session
(login, shell, `exit`, respawned login banner).

**New problem: the laptop console (framebuffer VT) froze at some point and
the keyboard stopped delivering.** The serial log shows no ohcid error after
`polling 1 keyboard(s)`, and the freeze moment is unknown ("didn't notice
when exactly"). Notably the devices behind the hub are passive for the bus
(USB is host-scheduled; without an armed interrupt ED nobody polls the
touchpad's reports), so touchpad motion cannot itself generate traffic —
the correlation may be coincidence.

Ruled out on the way: the periodic list with PLE=1 over a zeroed HCCA table
is the U-Boot/Linux pattern and NULL entries terminate the walk — U-Boot
does exactly this on this SoC during its own scans.

Diagnostics added for the next run: a poll-loop heartbeat
(`ohcid: alive, N report(s)` every 5 s). On the next freeze it separates:
- heartbeats continue, reports continue -> driver and kernel alive, freeze
  is downstream (inputd / vesad / console scheme);
- heartbeats continue, reports stall at 0 -> the interrupt ED stopped
  completing (HC wedge / device issue);
- heartbeats stop -> the driver or the kernel itself died.

## Iteration 11 — heartbeat localizes the wedge: interrupt ED stops completing

(2026-11-21)

The heartbeat image (sha256 `d23696d6…`, log `logs/serial-20261121-143439.log`)
caught the freeze in the act. The keyboard worked for about a minute
(8/2/19/17/8 reports per 5 s window while typing), then fell to **0 reports
permanently between 62 s and 67 s** while the heartbeats continued — the
driver process, the kernel and the debug scheme stayed alive. The owner
correlated the death explicitly with touching the touchpad this time.

So the poll loop spins but the interrupt TD never retires again. Silent
because `interrupt_done` only checks the TD condition code: an ED halted by
the HC with the TD left unretired (or a device that stopped answering)
leaves CC = NOT_ACCESSED forever and no error path logs anything.

Notable: the touchpad sits on the other controller (`fe3e0000`), is
enumerated but **not polled** (no armed ED) — USB is host-scheduled, so
touching it cannot itself create bus traffic. The correlation therefore has
to act through either the shared power rails, the hub re-reporting port
status, or something in the OHCI/runtime state — unknown yet.

Next diagnostics (image built, sha256 `…`): the heartbeat now carries per
keyboard device: ED head word (halt bit 0x1, toggle carry 0x2), TD condition
code, root port status (CCS/PES — did the keyboard detach?), and the HCCA
frame counter at the spec offset 128 (advancing = the HC keeps arbitrating
and sending SOFs). QEMU-verified: the frame counter advances at 1 kHz, the
fields show the idle state as expected (ed head without halt, cc 0xf, port
0x103).

## Iteration 12 — the keyboard dies from unpolled touchpad reports; drain it

(2026-11-21)

The extended heartbeat (sha256 `af624c60…`, log `logs/serial-20261121-144406.log`)
pinned the wedge precisely. At the moment the keyboard stops (after ~26 s of
working input):

```
kb ed head 0x329a040  <- no halt, no carry; head points at the armed TD
td cc 0xf             <- never retired again
port 0x303            <- CCS=1 PES=1 power on: the keyboard is attached
frame advancing       <- the HC is alive and arbitrating (SOF every 1 ms)
```

So the bus, the host controller and the port are all healthy — the keyboard
**device itself** stops producing reports. The owner narrowed the trigger:
merely **swiping the touchpad** (no clicking) is enough.

The touchpad (`046d:c52f`, boot mouse) is on the other controller, was
configured by our enumeration and then **never polled** — USB devices cannot
push, so its reports accumulate in the device. The theory: the touchpad MCU
chokes on its undelivered FIFO and (the PBP input devices share power/board
resources) drags the keyboard controller down with it. Manjaro never sees
this because it actually reads the touchpad.

Fix: treat a boot mouse as a polled device — set boot protocol, SET_IDLE 0,
arm its interrupt ED and **drain the reports without producing events** (a
report parser/mouse events are a separate follow-up). The hub-port loop now
also keeps scanning past mice instead of stopping at the first found device
("one keyboard is enough" checked the wrong predicate once mice counted).

QEMU rig with an added `usb-mouse`: the mouse arms and polls
(`polling mouse addr 2 endpoint 1 via periodic slot 1`), no regressions.

Hardware check pending: swipe the touchpad extensively with the keyboard
working — if the keyboard survives, the drain theory holds.
