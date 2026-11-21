//! ohcid - OHCI host controller driver for the RK3399 USB2 ports.
//!
//! The Pinebook Pro's internal keyboard and touchpad sit behind a Genesys hub
//! on `usb_host0`, which the installed U-Boot enumerates on the OHCI half of
//! the EHCI/OHCI pair (its boot log lists `Bus usb@fe3e0000: USB OHCI 1.0`
//! and no EHCI bus). This driver brings that controller up, enumerates the
//! root port(s) and one level of hub, and translates boot-protocol HID
//! keyboard reports into `inputd` events.
//!
//! Everything is polled: no interrupts are enabled, no `nanosleep` is used
//! (the kernel does not wake sleepers on the generic timer, README risk 13),
//! and the interrupt endpoint is scheduled in a single periodic slot, so the
//! keyboard is sampled roughly every 32 ms.
//!
//! The driver registers as an initfs service and calls `daemon.ready()`
//! before it touches hardware, so a failure here never blocks the boot: the
//! root filesystem still mounts and the serial login still works.

mod cru;
mod log;
mod ohci;
mod u2phy;

use std::{process, thread};

use fdt::Fdt;
use inputd::ProducerHandle;
use orbclient::ButtonEvent as OrbButtonEvent;
use orbclient::KeyEvent as OrbKeyEvent;
use orbclient::MouseRelativeEvent as OrbMouseRelativeEvent;

use crate::log::Logger;
use crate::ohci::{Error, Ohci};

/// The two OHCI companions of the RK3399 USB2 host controllers, from
/// `rk3399.dtsi` (Linux and U-Boot agree):
///
/// - `usb_host0_ohci` = 0xfe3a0000 (EHCI 0xfe380000)
/// - `usb_host1_ohci` = 0xfe3e0000 (EHCI 0xfe3c0000)
///
/// U-Boot's boot log lists 4 devices on `usb@fe3e0000` and 2 on
/// `usb@fe3a0000`, so the internal hub with the keyboard and touchpad is on
/// `usb_host1` and is scanned first.
const OHCI_BASES: [usize; 2] = [0xfe3e0000, 0xfe3a0000];

// USB standard descriptor types.
const DESC_DEVICE: u8 = 1;
const DESC_CONFIG: u8 = 2;
const DESC_HUB: u8 = 0x29;

// USB class codes.
const CLASS_HUB: u8 = 9;
const CLASS_HID: u8 = 3;

// Hub class features (USB 2.0, 11.5.4).
const FEAT_PORT_RESET: u16 = 4;
const FEAT_PORT_POWER: u16 = 8;
const FEAT_C_PORT_CONNECTION: u16 = 16;
const FEAT_C_PORT_RESET: u16 = 20;

// Hub port status bits.
const PORT_CONNECTION: u16 = 1 << 0;
const PORT_CONNECTION_CHANGE: u16 = 1 << 0;
const PORT_ENABLE: u16 = 1 << 1;
const PORT_RESET: u16 = 1 << 4;
const PORT_LOW_SPEED: u16 = 1 << 9;
const PORT_RESET_CHANGE: u16 = 1 << 4;

fn main() {
    daemon::Daemon::new(daemon);
}

fn daemon(daemon: daemon::Daemon) -> ! {
    // Never let USB bring-up hold the boot up: init blocks on notify-type
    // services, and the root filesystem does not depend on input. Signal
    // readiness before touching any hardware.
    daemon.ready();
    common::init();

    let mut driver = Driver::new();
    driver.log.log(format_args!("ohcid: starting"));

    driver.bring_up_all();

    if driver.hids.is_empty() {
        driver.log.log(format_args!(
            "ohcid: no HID keyboard found; input stays on the other drivers"
        ));
        // Stay alive but idle instead of exiting, so a later hotplug could
        // still be handled once re-enumeration is implemented.
        loop {
            thread::yield_now();
        }
    }

    driver.log.log(format_args!(
        "ohcid: polling {} keyboard(s)",
        driver.hids.len()
    ));
    driver.poll_forever();
}

struct Driver {
    log: Logger,
    /// One entry per controller that was brought up successfully.
    hcs: Vec<Ohci>,
    hids: Vec<HidDevice>,
    input: ProducerHandle,
}

impl Driver {
    fn new() -> Self {
        let log = Logger::new();
        let input = match wait_for_input(&log) {
            Some(input) => input,
            None => {
                log.log(format_args!("ohcid: /scheme/input never appeared, exiting"));
                process::exit(1);
            }
        };
        Driver {
            log,
            hcs: Vec::new(),
            hids: Vec::new(),
            input,
        }
    }

    fn bring_up_all(&mut self) {
        let dtb = read_dtb();
        self.log.log(format_args!(
            "ohcid: devicetree {} bytes",
            dtb.as_ref().map_or(0, |data| data.len())
        ));
        let bases = dtb_ohci_bases(dtb.as_deref());
        if bases.is_empty() {
            self.log.log(format_args!(
                "ohcid: no OHCI nodes in the devicetree, trying the known RK3399 addresses"
            ));
            // This is not a RK3399 (QEMU, for instance). Exercise the DMA
            // path anyway: it is the part of the driver that cannot be tested
            // by simply failing to map a register.
            self.selftest();
        } else {
            let mut list = String::new();
            for base in &bases {
                let _ = core::fmt::Write::write_fmt(&mut list, format_args!(" 0x{:08x}", base));
            }
            self.log
                .log(format_args!("ohcid: devicetree OHCI nodes:{}", list));
            // The CRU/PHY bring-up below applies to the RK3399's own
            // controllers only: U-Boot gates the USB clocks when it hands
            // over to the EFI payload, and an access to an unclocked
            // controller stalls the whole machine, so the gates must be open
            // before probing. Other platforms (emulation, other boards) must
            // not touch RK3399 clock registers at all.
            if bases.iter().any(|base| OHCI_BASES.contains(base)) {
                // One-shot CPU clock dump: the desktop speed depends on the
                // rate U-Boot left the clusters at (experiment 14).
                cru::log_cpu_clocks(&self.log);
                // U-Boot leaves the little cluster divided down (measured
                // 300 MHz); raise it to the PLL rate (600 MHz).
                cru::raise_cpu_clocks(&self.log);
                if !cru::enable_usb_host_clocks(&self.log) {
                    return;
                }
                // The PHY is powered down as well; without its 480 MHz UTMI
                // clock the controller sees no devices at all.
                u2phy::power_on_host_ports(&self.log);
            }
        }

        // Probe what the devicetree names; without devicetree information
        // fall back to the known RK3399 addresses.
        let probe: Vec<usize> = if bases.is_empty() {
            OHCI_BASES.to_vec()
        } else {
            bases.clone()
        };
        for base in probe {
            if let Err(err) = self.bring_up_controller(base) {
                self.log
                    .log(format_args!("ohcid: 0x{:08x} failed: {:?}", base, err));
            }
        }
    }

    fn bring_up_controller(&mut self, base: usize) -> Result<(), Error> {
        self.log.log(format_args!("ohcid: 0x{:08x}: probing", base));
        let mut hc = unsafe { Ohci::new(base, &self.log) }?;
        self.log
            .log(format_args!("ohcid: 0x{:08x}: reading HcRevision", base));
        let revision = hc.revision();
        self.log.log(format_args!(
            "ohcid: 0x{:08x}: OHCI revision 0x{:08x}",
            base, revision
        ));
        let ports = hc.num_ports();
        self.log
            .log(format_args!("ohcid: 0x{:08x}: {} port(s)", base, ports));
        hc.init()?;
        self.log.log(format_args!(
            "ohcid: 0x{:08x}: operational, control 0x{:08x} rh_a 0x{:08x}",
            base,
            hc.read(ohci::REG_CONTROL),
            hc.read(ohci::REG_RH_A)
        ));
        hc.power_ports()?;

        let hc_index = self.hcs.len();
        let found = {
            let mut en = Enumeration {
                hc: &mut hc,
                log: &self.log,
                hc_index,
                next_addr: 1,
                next_ed: 4, // slots 0..3 are the control pipe
                next_td: 4,
                next_slot: 0,
                root_port: None,
                found: Vec::new(),
            };

            for port in 0..ports {
                // The PHY may still be settling: give the root hub a moment
                // to report the connection instead of giving up on the first
                // read.
                let mut status = en.hc.port_status(port);
                let mut wait = 0;
                while status & 1 == 0 && wait < 5 {
                    en.hc.delay_ms(20);
                    status = en.hc.port_status(port);
                    wait += 1;
                }
                en.log.log(format_args!(
                    "ohcid: 0x{:08x}: port {} status 0x{:08x}{}",
                    base,
                    port + 1,
                    status,
                    if status & 1 != 0 { " (connected)" } else { "" }
                ));
                if status & 1 == 0 {
                    continue;
                }
                en.root_port = Some(port);

                match en.hc.reset_root_port(port) {
                    Ok((connected, low_speed, enabled)) => {
                        en.log.log(format_args!(
                        "ohcid: 0x{:08x}: port {} reset (connected {}, low-speed {}, enabled {})",
                        base,
                        port + 1,
                        connected,
                        low_speed,
                        enabled
                    ));
                        if !connected {
                            continue;
                        }
                        // USB 2.0, 7.1.7.5: 2 ms of recovery, generously padded.
                        en.hc.delay_ms(20);
                        if let Err(err) = en.enumerate_at_zero(low_speed, 0) {
                            en.log.log(format_args!(
                                "ohcid: 0x{:08x}: port {} enumeration failed: {:?}",
                                base,
                                port + 1,
                                err
                            ));
                        }
                    }
                    Err(err) => en.log.log(format_args!(
                        "ohcid: 0x{:08x}: port {} reset failed: {:?}",
                        base,
                        port + 1,
                        err
                    )),
                }
            }

            en.found
        };
        self.log.log(format_args!(
            "ohcid: 0x{:08x}: {} HID device(s)",
            base,
            found.len()
        ));
        self.hids.extend(found);
        // Keep the controller alive for the poll loop.
        self.hcs.push(hc);
        Ok(())
    }

    /// Verify that physically contiguous DMA memory can be allocated and is
    /// usable. Runs on platforms without OHCI nodes so the same code path can
    /// be tested under emulation.
    fn selftest(&mut self) {
        self.log.log(format_args!(
            "ohcid: selftest: allocating 4 KiB of DMA memory"
        ));
        match common::dma::Dma::<[u8]>::zeroed_slice(4096) {
            Ok(dma) => {
                let phys = dma.physical();
                let virt = dma.virt_addr();
                self.log.log(format_args!(
                    "ohcid: selftest: DMA ok, phys 0x{:08x} virt 0x{:08x}",
                    phys, virt
                ));
                let mut dma = unsafe { dma.assume_init() };
                dma[0] = 0x5a;
                dma[4095] = 0xa5;
                self.log.log(format_args!(
                    "ohcid: selftest: DMA readback 0x{:02x} 0x{:02x}",
                    dma[0], dma[4095]
                ));
            }
            Err(err) => {
                self.log
                    .log(format_args!("ohcid: selftest: DMA failed: {:?}", err));
            }
        }
    }

    fn poll_forever(&mut self) -> ! {
        // Disjoint field borrows: each device polls its own controller.
        let hcs = &mut self.hcs;
        let hids = &mut self.hids;
        let input = &mut self.input;
        let log = &self.log;
        // Heartbeat (experiment 8, iteration 10): a laptop-console freeze
        // happened with no ohcid error lines — this discriminates a wedged
        // driver (heartbeats stop or the report counter stalls) from a
        // downstream freeze (heartbeats continue after the console dies)
        // and proves the kernel + debug scheme alive through the freeze.
        let mut last_beat = ohci::now_ms();
        let mut beat_reports: u64 = 0;
        loop {
            for hid in hids.iter_mut() {
                hid.poll(&mut hcs[hid.hc_index], input, log);
            }
            // Self-healing (experiment 8, iteration 13): both input devices
            // can stop answering with the port still connected (the owner
            // correlates it with touching the touchpad). When a device that
            // previously delivered reports falls silent for 15 s, reset its
            // root port and enumerate it again — the port reset returns the
            // device to address 0, so the full control dance is redone.
            let now2 = ohci::now_ms();
            let stale = hids
                .iter()
                .find(|h| {
                    h.reports > 0
                        && h.root_port.is_some()
                        && now2.wrapping_sub(h.last_report_ms) >= 15_000
                })
                .map(|h| (h.hc_index, h.addr, h.low_speed, h.mps));
            if let Some((hc_index, addr, low_speed, mps)) = stale {
                // Before resetting: an idle (but healthy) device also has
                // no reports — its interrupt endpoint just NAKs until a key
                // is pressed. Probe the control endpoint instead: a live
                // device answers GET_DESCRIPTOR immediately, a wedged one
                // times out. Without this the driver would reset the
                // keyboard on every 15 s of plain idle (measured, iteration
                // 14).
                let probe = build_setup(0x80, 0x06, 0x0100, 0x0000, 8);
                let alive = match hcs[hc_index].control_transfer(
                    addr,
                    low_speed,
                    u16::from(mps),
                    probe,
                    None,
                    false,
                ) {
                    Ok(_) => true,
                    Err(err) => {
                        log.log(format_args!(
                            "ohcid: liveness probe of addr {} failed: {:?}",
                            addr, err
                        ));
                        false
                    }
                };
                if alive {
                    // Healthy idle: forget the stall, do not touch the port.
                    for h in hids
                        .iter_mut()
                        .filter(|h| h.hc_index == hc_index && h.addr == addr)
                    {
                        h.last_report_ms = now2;
                    }
                } else {
                    log.log(format_args!(
                        "ohcid: input addr {} silent and unresponsive, resetting its port",
                        addr
                    ));
                    let mut revived = false;
                    match hcs[hc_index].reset_root_port(0) {
                        Ok((connected, low_speed2, enabled)) => {
                            log.log(format_args!(
                                "ohcid: revive port reset: connected {} low-speed {} enabled {}",
                                connected, low_speed2, enabled
                            ));
                            if connected && enabled {
                                hcs[hc_index].delay_ms(20);
                                let mut en = Enumeration {
                                    hc: &mut hcs[hc_index],
                                    log,
                                    hc_index,
                                    next_addr: 1,
                                    next_ed: 4,
                                    next_td: 4,
                                    next_slot: 0,
                                    root_port: Some(0),
                                    found: Vec::new(),
                                };
                                match en.enumerate_at_zero(low_speed2, 0) {
                                    Ok(()) => {
                                        let fresh = std::mem::take(&mut en.found);
                                        if fresh.iter().any(|d| d.kind == HidKind::Keyboard) {
                                            // Replace every root-port HID
                                            // endpoint: the touchpad is a
                                            // second HidDevice of the same
                                            // device, and its old ED has been
                                            // rewritten by the new
                                            // enumeration (a stale entry kept
                                            // polling a foreign ED — a wedge
                                            // candidate, experiment 13).
                                            hids.retain(|h| {
                                                h.hc_index != hc_index || h.root_port.is_none()
                                            });
                                            let count = fresh.len();
                                            for mut d in fresh {
                                                d.last_report_ms = now2;
                                                hids.push(d);
                                            }
                                            log.log(format_args!(
                                                "ohcid: revived {} endpoint(s) previously on addr {}",
                                                count, addr
                                            ));
                                            revived = true;
                                        } else {
                                            log.log(format_args!(
                                                "ohcid: revive found no keyboard after reset"
                                            ));
                                        }
                                    }
                                    Err(err) => log.log(format_args!(
                                        "ohcid: revive enumeration failed: {:?}",
                                        err
                                    )),
                                }
                            }
                        }
                        Err(err) => {
                            log.log(format_args!("ohcid: revive port reset failed: {:?}", err))
                        }
                    }
                    if !revived {
                        // Nothing worked: back off so the next attempt is not
                        // an endless tight loop of resets.
                        for h in hids
                            .iter_mut()
                            .filter(|h| h.hc_index == hc_index && h.addr == addr)
                        {
                            h.last_report_ms = now2;
                        }
                    }
                }
            }
            let now = ohci::now_ms();
            if now.wrapping_sub(last_beat) >= 5_000 {
                let reports: u64 = hids.iter().map(|h| h.reports).sum();
                // Per-device state for the wedge diagnosis: ED head (halt /
                // toggle-carry bits), TD condition code, root port status and
                // the HC frame counter (advancing frame number = the HC is
                // alive and sending SOFs).
                for hid in hids.iter() {
                    let hc = &hcs[hid.hc_index];
                    log.log(format_args!(
                        "ohcid: alive, {} report(s); kb ed head {:#x} td cc {:#x} port {:#x} frame {:#x}",
                        reports.saturating_sub(beat_reports),
                        hc.ed(hid.ed).head,
                        hc.interrupt_cc(hid.td),
                        hc.port_status(0),
                        hc.frame_number(),
                    ));
                }
                if hids.is_empty() {
                    log.log(format_args!(
                        "ohcid: alive, {} report(s)",
                        reports.saturating_sub(beat_reports)
                    ));
                }
                beat_reports = reports;
                last_beat = now;
            }
            thread::yield_now();
        }
    }
}

fn wait_for_input(log: &Logger) -> Option<ProducerHandle> {
    let deadline = ohci::now_ms() + 5_000;
    loop {
        match ProducerHandle::new() {
            Ok(handle) => return Some(handle),
            Err(err) => {
                if ohci::now_ms() > deadline {
                    log.log(format_args!("ohcid: cannot open /scheme/input: {}", err));
                    return None;
                }
                thread::yield_now();
            }
        }
    }
}

fn read_dtb() -> Option<Vec<u8>> {
    std::fs::read("kernel.dtb:").ok()
}

/// Collect the `reg` base addresses of the enabled `rockchip,rk3399-ohci`
/// nodes. An empty result means "no devicetree information available", in
/// which case the caller falls back to the known addresses.
fn dtb_ohci_bases(data: Option<&[u8]>) -> Vec<usize> {
    let Some(data) = data else {
        return Vec::new();
    };
    let Ok(fdt) = Fdt::new_unaligned(data) else {
        return Vec::new();
    };
    let mut bases = Vec::new();
    // `rk3399.dtsi` (Linux and U-Boot) uses the generic binding; keep the
    // Rockchip-specific string for boards that carry it.
    for node in fdt.all_compatible(&["generic-ohci", "rockchip,rk3399-ohci"]) {
        let Some(reg) = node.reg() else { continue };
        let Some(Ok(entry)) = reg.iter::<usize, usize>().next() else {
            continue;
        };
        bases.push(entry.address);
    }
    bases
}

/// Enumeration state for one controller pass.
struct Enumeration<'a> {
    hc: &'a mut Ohci,
    log: &'a Logger,
    hc_index: usize,
    next_addr: u8,
    next_ed: usize,
    next_td: usize,
    next_slot: usize,
    /// Root hub port being enumerated (None while walking hub ports —
    /// reviving a device behind a hub needs a hub-port reset instead).
    root_port: Option<usize>,
    found: Vec<HidDevice>,
}

impl<'a> Enumeration<'a> {
    fn alloc_addr(&mut self) -> Result<u8, Error> {
        let addr = self.next_addr;
        if addr >= 127 {
            return Err(Error::Protocol("out of USB addresses"));
        }
        self.next_addr += 1;
        Ok(addr)
    }

    /// Enumerate the device that currently answers at address 0.
    /// `depth` is 0 for a root port device, 1 for a hub downstream port.
    fn enumerate_at_zero(&mut self, low_speed: bool, depth: u8) -> Result<(), Error> {
        // First 8 bytes: enough to learn wMaxPacketSize0. Some devices — the
        // internal Genesys hub on `fe3e0000` measured in experiment 8,
        // iteration 8 — stall the very first request after a port reset
        // while their firmware finishes booting; U-Boot tolerates this by
        // scanning much later. Retry with a settle delay before giving up.
        let mut first = [0u8; 8];
        let mut err = match self.get_descriptor(0, low_speed, 8, DESC_DEVICE, 0, &mut first) {
            Ok(_) => None,
            Err(e) => Some(e),
        };
        for attempt in 1..=3 {
            match err {
                None => break,
                Some(e) => {
                    self.log.log(format_args!(
                        "ohcid:   first get_descriptor attempt {} failed {:?}, retrying after 150 ms",
                        attempt, e
                    ));
                    self.hc.delay_ms(150);
                    match self.get_descriptor(0, low_speed, 8, DESC_DEVICE, 0, &mut first) {
                        Ok(_) => {
                            err = None;
                            break;
                        }
                        Err(re) => err = Some(re),
                    }
                }
            }
        }
        if let Some(err) = err {
            // Diagnostics for the first control transfer (experiment 8,
            // iterations 2, 7 and 8): the setup buffer of the second hardware
            // controller reads back as zeros although the TDs in the head of
            // the same page are delivered to the controller correctly.
            let d = self.hc.diag_setup_buffer();
            self.log.log(format_args!(
                "ohcid:   dbg: first get_descriptor failed {:?}; buf va 0x{:08x} phys 0x{:08x}; orig {:02x?}; write-rb {:02x?}; phys-view {:02x?}; canary {:02x?}; td0 {:08x?}",
                err,
                d.va,
                self.hc.bufs_physical(),
                d.original,
                d.write_readback,
                d.phys_view,
                d.canary,
                d.td0,
            ));
            return Err(err);
        }
        let mps0 = match first[7] {
            8 | 16 | 32 | 64 => first[7],
            other => {
                self.log.log(format_args!(
                    "ohcid:   implausible bMaxPacketSize0 {} (using 8)",
                    other
                ));
                8
            }
        };
        self.log.log(format_args!(
            "ohcid:   dbg: step1 ok (first {:02x?}), mps0 {}",
            first, mps0
        ));

        let addr = self.alloc_addr()?;
        self.set_address(0, low_speed, 8, addr)?;
        // USB 2.0, 9.2.6.3: 2 ms of recovery.
        self.hc.delay_ms(10);
        self.log
            .log(format_args!("ohcid:   dbg: step2 ok (addr {})", addr));

        let mut dev = [0u8; 18];
        self.get_descriptor(addr, low_speed, u16::from(mps0), DESC_DEVICE, 0, &mut dev)?;
        self.log
            .log(format_args!("ohcid:   dbg: step3 ok (dev {:02x?})", dev));

        let vid = u16::from_le_bytes([dev[8], dev[9]]);
        let pid = u16::from_le_bytes([dev[10], dev[11]]);
        let class = dev[4];
        let subclass = dev[5];
        let protocol = dev[6];
        self.log.log(format_args!(
            "ohcid:   {} addr {} id {:04x}:{:04x} class {:02x}/{:02x}/{:02x} mps0 {} {}",
            if depth == 0 {
                "root device"
            } else {
                "hub device"
            },
            addr,
            vid,
            pid,
            class,
            subclass,
            protocol,
            mps0,
            if low_speed { "low-speed" } else { "full-speed" }
        ));

        if class == CLASS_HUB {
            return self.enumerate_hub(addr, low_speed, mps0, depth);
        }

        // Configuration descriptor: header first for wTotalLength.
        let mut header = [0u8; 9];
        self.get_descriptor(
            addr,
            low_speed,
            u16::from(mps0),
            DESC_CONFIG,
            0,
            &mut header,
        )?;
        let total = u16::from_le_bytes([header[2], header[3]]) as usize;
        if !(9..=512).contains(&total) {
            return Err(Error::Protocol("bad configuration descriptor length"));
        }
        let mut config = vec![0u8; total];
        self.get_descriptor(
            addr,
            low_speed,
            u16::from(mps0),
            DESC_CONFIG,
            0,
            &mut config,
        )?;

        let config_value = config[5];
        self.set_configuration(addr, low_speed, u16::from(mps0), config_value)?;

        let eps = find_hid_endpoints(&config);
        if eps.is_empty() {
            self.log
                .log(format_args!("ohcid:   no HID interface, ignored"));
            return Ok(());
        }
        // The keyboard is the boot-protocol interface if present, else the
        // first HID endpoint (fallback, same as before).
        let kb_index = eps.iter().position(|e| e.protocol == 1).unwrap_or(0);
        for (n, info) in eps.iter().enumerate() {
            self.log.log(format_args!(
                "ohcid:   HID interface {} protocol {} endpoint 0x{:02x} mps {} interval {}",
                info.interface, info.protocol, info.ep_address, info.ep_mps, info.ep_interval
            ));
            if n == kb_index {
                continue;
            }
            // Every other interrupt endpoint on the device (the touchpad
            // interface of the keyboard chip) must be polled. Prefer the
            // report descriptor; the Pinebook Pro chip stalls every request
            // shape, so for that chip the measured layout is used
            // (`[report id, buttons, dx, dy]`, experiment 13). Without
            // either, fall back to the boot-protocol probe, and without
            // that keep draining.
            let described = self
                .hid_report_layout(addr, low_speed, mps0, info.interface)
                .filter(ReportLayout::usable);
            let layout = match described {
                Some(layout) => {
                    self.log.log(format_args!(
                        "ohcid:   mouse on endpoint 0x{:02x} of the keyboard device (report descriptor layout)",
                        info.ep_address
                    ));
                    Some(layout)
                }
                None if vid == 0x258a && pid == 0x001e => {
                    self.log.log(format_args!(
                        "ohcid:   mouse on endpoint 0x{:02x} of the keyboard device (measured Pinebook Pro touchpad layout)",
                        info.ep_address
                    ));
                    Some(ReportLayout::pinebook_touchpad())
                }
                None => None,
            };
            if let Some(layout) = layout {
                self.start_hid_kind(addr, low_speed, *info, HidKind::Mouse, layout)?;
            } else if self
                .hid_set_boot_protocol(addr, low_speed, mps0, info.interface)
                .is_ok()
            {
                self.start_hid_kind(
                    addr,
                    low_speed,
                    *info,
                    HidKind::Mouse,
                    ReportLayout::boot_mouse(),
                )?;
                self.log.log(format_args!(
                    "ohcid:   boot mouse on endpoint 0x{:02x} of the keyboard device",
                    info.ep_address
                ));
            } else {
                self.start_hid_kind(
                    addr,
                    low_speed,
                    *info,
                    HidKind::Drain,
                    ReportLayout::default(),
                )?;
                self.log.log(format_args!(
                    "ohcid:   draining extra endpoint 0x{:02x} of the keyboard device",
                    info.ep_address
                ));
            }
        }
        let info = eps[kb_index];
        if info.protocol == 2 {
            self.log.log(format_args!(
                "ohcid:   boot mouse interface, polling to drain reports"
            ));
            self.hid_set_protocol(addr, low_speed, mps0, info.interface)?;
            let _ = self.hid_set_idle(addr, low_speed, mps0, info.interface);
            self.start_hid_kind(
                addr,
                low_speed,
                info,
                HidKind::Drain,
                ReportLayout::default(),
            )?;
            return Ok(());
        }
        if info.protocol == 1 {
            // Boot protocol: fixed 8-byte keyboard reports.
            self.log
                .log(format_args!("ohcid:   dbg: before set_protocol"));
            self.hid_set_protocol(addr, low_speed, mps0, info.interface)?;
            self.log
                .log(format_args!("ohcid:   dbg: after set_protocol"));
        } else {
            self.log.log(format_args!(
                "ohcid:   device uses report protocol, assuming boot layout"
            ));
        }
        self.log.log(format_args!("ohcid:   dbg: before set_idle"));
        let _ = self.hid_set_idle(addr, low_speed, mps0, info.interface);
        self.log.log(format_args!("ohcid:   dbg: before start_hid"));
        self.start_hid_kind(
            addr,
            low_speed,
            info,
            HidKind::Keyboard,
            ReportLayout::default(),
        )?;
        self.log.log(format_args!("ohcid:   dbg: start_hid ok"));
        Ok(())
    }

    fn enumerate_hub(
        &mut self,
        addr: u8,
        low_speed: bool,
        mps0: u8,
        depth: u8,
    ) -> Result<(), Error> {
        // Everything past this point hangs off hub ports, not the root hub.
        self.root_port = None;
        if depth >= 1 {
            self.log
                .log(format_args!("ohcid:   nested hub ignored (one level only)"));
            return Ok(());
        }
        self.set_configuration(addr, low_speed, u16::from(mps0), 1)?;

        let mut hub = [0u8; 9];
        let got =
            self.get_class_descriptor(addr, low_speed, u16::from(mps0), DESC_HUB, 0, &mut hub)?;
        if got < 6 {
            return Err(Error::Protocol("short hub descriptor"));
        }
        let ports = hub[2];
        // Hub descriptor byte 5: bPwrOn2PwrGood in 2 ms units — how long the
        // port needs after power-on before the connection status is valid
        // (U-Boot waits max(100 ms, 2*bPwrOn2PwrGood)).
        let pgood_ms = u32::from(hub[5].saturating_mul(2)).max(100);
        self.log.log(format_args!(
            "ohcid:   hub with {} port(s), power-on {} ms",
            ports, pgood_ms
        ));

        // U-Boot `usb_hub_power_on`: power every port first, wait for the
        // power-good time, only then ask for connections — a port without
        // power reports nothing connected (the Genesys hub measured in
        // experiment 8, iteration 9 keeps all four ports unpowered until
        // PORT_POWER is set).
        for port in 1..=ports {
            let _ = self.hub_set_feature(addr, low_speed, mps0, port, FEAT_PORT_POWER);
        }
        self.hc.delay_ms(u64::from(pgood_ms));

        for port in 1..=ports {
            let (status, change) = self.hub_port_status(addr, low_speed, mps0, port)?;
            if change & PORT_CONNECTION_CHANGE != 0 {
                let _ = self.hub_clear_feature(addr, low_speed, mps0, port, FEAT_C_PORT_CONNECTION);
            }
            if status & PORT_CONNECTION == 0 {
                continue;
            }

            self.hub_set_feature(addr, low_speed, mps0, port, FEAT_PORT_RESET)?;
            // The hub clears PORT_RESET when its 10 ms sequence finishes.
            let deadline = ohci::now_ms() + 500;
            loop {
                let (status, change) = self.hub_port_status(addr, low_speed, mps0, port)?;
                if status & PORT_RESET == 0 && change & PORT_RESET_CHANGE != 0 {
                    break;
                }
                if ohci::now_ms() > deadline {
                    self.log.log(format_args!(
                        "ohcid:   hub port {} reset timed out (status 0x{:04x})",
                        port, status
                    ));
                    break;
                }
                self.hc.delay_ms(2);
            }

            let _ = self.hub_clear_feature(addr, low_speed, mps0, port, FEAT_C_PORT_RESET);
            let _ = self.hub_clear_feature(addr, low_speed, mps0, port, FEAT_C_PORT_CONNECTION);

            let (status, _) = self.hub_port_status(addr, low_speed, mps0, port)?;
            if status & PORT_CONNECTION == 0 {
                self.log.log(format_args!(
                    "ohcid:   hub port {}: nothing connected after reset",
                    port
                ));
                continue;
            }

            let dev_low_speed = status & PORT_LOW_SPEED != 0;
            self.log.log(format_args!(
                "ohcid:   hub port {}: device attached (status 0x{:04x}, {}, enabled {})",
                port,
                status,
                if dev_low_speed {
                    "low-speed"
                } else {
                    "full-speed"
                },
                status & PORT_ENABLE != 0
            ));
            self.hc.delay_ms(20);

            if let Err(err) = self.enumerate_at_zero(dev_low_speed, depth + 1) {
                self.log.log(format_args!(
                    "ohcid:   hub port {} enumeration failed: {:?}",
                    port, err
                ));
            }
            // One keyboard is enough for the bring-up milestone, but keep
            // scanning past mice: the touchpad must be drained (iteration 12).
            if self.found.iter().any(|d| d.kind == HidKind::Keyboard) {
                break;
            }
        }
        Ok(())
    }

    // --- control requests -------------------------------------------------

    fn get_descriptor(
        &mut self,
        addr: u8,
        low_speed: bool,
        mps: u16,
        desc_type: u8,
        index: u8,
        buf: &mut [u8],
    ) -> Result<usize, Error> {
        let value = (u32::from(desc_type) << 8) | u32::from(index);
        let setup = build_setup(0x80, 6, value, 0, buf.len() as u16);
        self.hc
            .control_transfer(addr, low_speed, mps, setup, Some(buf), true)
    }

    /// Class-specific GET_DESCRIPTOR (bmRequestType 0xA0): hub descriptors
    /// are requested with a class request, a standard 0x80 request is
    /// answered with STALL by spec-compliant hubs.
    fn get_class_descriptor(
        &mut self,
        addr: u8,
        low_speed: bool,
        mps: u16,
        desc_type: u8,
        index: u8,
        buf: &mut [u8],
    ) -> Result<usize, Error> {
        let value = (u32::from(desc_type) << 8) | u32::from(index);
        let setup = build_setup(0xa0, 6, value, 0, buf.len() as u16);
        self.hc
            .control_transfer(addr, low_speed, mps, setup, Some(buf), true)
    }

    fn set_address(
        &mut self,
        addr: u8,
        low_speed: bool,
        mps: u16,
        new_addr: u8,
    ) -> Result<(), Error> {
        let setup = build_setup(0x00, 5, u32::from(new_addr), 0, 0);
        self.hc
            .control_transfer(addr, low_speed, mps, setup, None, false)?;
        Ok(())
    }

    fn set_configuration(
        &mut self,
        addr: u8,
        low_speed: bool,
        mps: u16,
        value: u8,
    ) -> Result<(), Error> {
        let setup = build_setup(0x00, 9, u32::from(value), 0, 0);
        self.hc
            .control_transfer(addr, low_speed, mps, setup, None, false)?;
        Ok(())
    }

    fn hid_set_protocol(
        &mut self,
        addr: u8,
        low_speed: bool,
        mps0: u8,
        interface: u8,
    ) -> Result<(), Error> {
        // bmRequestType 0x21 (class, interface), bRequest SET_PROTOCOL.
        let setup = build_setup(0x21, 0x0b, 1, u32::from(interface), 0);
        self.hc
            .control_transfer(addr, low_speed, u16::from(mps0), setup, None, false)?;
        Ok(())
    }

    /// SET_PROTOCOL(boot, wValue 0) on an interface. For the touchpad
    /// interface of the Pinebook Pro keyboard chip this is what turns its
    /// reports into the fixed 3-byte boot mouse layout (buttons/dx/dy) —
    /// if the chip supports it. A device that does not stalls the request,
    /// which the caller treats as "keep draining".
    fn hid_set_boot_protocol(
        &mut self,
        addr: u8,
        low_speed: bool,
        mps0: u8,
        interface: u8,
    ) -> Result<(), Error> {
        let setup = build_setup(0x21, 0x0b, 0, u32::from(interface), 0);
        self.hc
            .control_transfer(addr, low_speed, u16::from(mps0), setup, None, false)?;
        Ok(())
    }

    /// Fetch and log the HID report descriptor of an interface and parse the
    /// field layout out of it. Returns None when every request variant is
    /// stalled or the descriptor does not describe X/Y/buttons.
    ///
    /// Three request shapes exist in the wild: HID 1.11 7.1.1 says standard
    /// request, interface recipient (`0x81`); Linux `usb_get_descriptor`
    /// uses the device recipient (`0x80`); some chips only answer the class
    /// request (`0xa1`). The Pinebook Pro chip stalls all three.
    fn hid_report_layout(
        &mut self,
        addr: u8,
        low_speed: bool,
        mps0: u8,
        interface: u8,
    ) -> Option<ReportLayout> {
        let mut buf = [0u8; 128];
        let mut len = 0usize;
        for (request_type, index, what) in [
            (0x81u8, u32::from(interface), "standard/interface"),
            (0x80, 0, "standard/device"),
            (0xa1, u32::from(interface), "class/interface"),
        ] {
            let setup = build_setup(request_type, 0x06, 0x2200, index, buf.len() as u16);
            match self.hc.control_transfer(
                addr,
                low_speed,
                u16::from(mps0),
                setup,
                Some(&mut buf),
                true,
            ) {
                Ok(n) => {
                    self.log.log(format_args!(
                        "ohcid:   report descriptor of interface {} via {}: {} bytes",
                        interface, what, n
                    ));
                    len = n;
                    break;
                }
                Err(err) => self.log.log(format_args!(
                    "ohcid:   report descriptor of interface {} via {}: {:?}",
                    interface, what, err
                )),
            }
        }
        if len == 0 {
            return None;
        }
        let bytes = &buf[..len.min(buf.len())];
        let mut line = String::new();
        for (n, chunk) in bytes.chunks(16).enumerate() {
            line.clear();
            for b in chunk {
                line.push_str(&format!("{:02x} ", b));
            }
            self.log.log(format_args!(
                "ohcid:   report descriptor {} +{:#04x}: {}",
                interface,
                n * 16,
                line
            ));
        }
        let layout = parse_report_descriptor(bytes);
        self.log.log(format_args!(
            "ohcid:   layout: id {:?} buttons {:?}/{} x {:?} (abs {}) y {:?} (abs {})",
            layout.report_id,
            layout.buttons_byte,
            layout.buttons_bits,
            layout.x_byte,
            layout.x_absolute,
            layout.y_byte,
            layout.y_absolute
        ));
        Some(layout)
    }

    fn hid_set_idle(
        &mut self,
        addr: u8,
        low_speed: bool,
        mps0: u8,
        interface: u8,
    ) -> Result<(), Error> {
        // bmRequestType 0x21, bRequest SET_IDLE, idle rate 0 (report on change).
        let setup = build_setup(0x21, 0x0a, 0, u32::from(interface), 0);
        self.hc
            .control_transfer(addr, low_speed, u16::from(mps0), setup, None, false)?;
        Ok(())
    }

    fn hub_port_status(
        &mut self,
        addr: u8,
        low_speed: bool,
        mps0: u8,
        port: u8,
    ) -> Result<(u16, u16), Error> {
        let mut data = [0u8; 4];
        let setup = build_setup(0xa3, 0x00, 0, u32::from(port), 4);
        self.hc.control_transfer(
            addr,
            low_speed,
            u16::from(mps0),
            setup,
            Some(&mut data),
            true,
        )?;
        Ok((
            u16::from_le_bytes([data[0], data[1]]),
            u16::from_le_bytes([data[2], data[3]]),
        ))
    }

    fn hub_set_feature(
        &mut self,
        addr: u8,
        low_speed: bool,
        mps0: u8,
        port: u8,
        feature: u16,
    ) -> Result<(), Error> {
        let setup = build_setup(0x23, 0x03, u32::from(feature), u32::from(port), 0);
        self.hc
            .control_transfer(addr, low_speed, u16::from(mps0), setup, None, false)?;
        Ok(())
    }

    fn hub_clear_feature(
        &mut self,
        addr: u8,
        low_speed: bool,
        mps0: u8,
        port: u8,
        feature: u16,
    ) -> Result<(), Error> {
        let setup = build_setup(0x23, 0x01, u32::from(feature), u32::from(port), 0);
        self.hc
            .control_transfer(addr, low_speed, u16::from(mps0), setup, None, false)?;
        Ok(())
    }

    fn start_hid_kind(
        &mut self,
        addr: u8,
        low_speed: bool,
        info: HidInfo,
        kind: HidKind,
        layout: ReportLayout,
    ) -> Result<(), Error> {
        let ed = self.next_ed;
        let td = self.next_td;
        let dummy = self.next_td + 1;
        let slot = self.next_slot % 32;
        self.next_ed += 1;
        self.next_td += 2;
        self.next_slot += 1;

        let mps = info.ep_mps;
        // Fixed slot per ED (see Ohci::endpoint_buf): survives re-enumeration.
        let (buf_off, buf_phys) = self.hc.endpoint_buf(ed - 4, usize::from(mps))?;
        self.hc.arm_interrupt(
            ed,
            td,
            dummy,
            addr,
            info.ep_number,
            low_speed,
            mps,
            buf_phys,
            slot,
        )?;

        self.log.log(format_args!(
            "ohcid:   polling {} addr {} endpoint {} via periodic slot {}",
            match kind {
                HidKind::Keyboard => "keyboard",
                HidKind::Mouse => "mouse",
                HidKind::Drain => "drain",
            },
            addr,
            info.ep_number,
            slot
        ));

        self.found.push(HidDevice {
            hc_index: self.hc_index,
            ed,
            td,
            dummy,
            buf_off,
            buf_phys,
            mps,
            slot,
            addr,
            ep: info.ep_number,
            low_speed,
            kind,
            root_port: self.root_port,
            prev_modifiers: 0,
            prev_keys: [0; 6],
            prev_buttons: 0,
            prev_abs: None,
            layout,
            raw_logged: 0,
            reports: 0,
            last_report_ms: ohci::now_ms(),
        });
        Ok(())
    }
}

fn build_setup(request_type: u8, request: u8, value: u32, index: u32, length: u16) -> [u8; 8] {
    let mut setup = [0u8; 8];
    setup[0] = request_type;
    setup[1] = request;
    setup[2..4].copy_from_slice(&(value as u16).to_le_bytes());
    setup[4..6].copy_from_slice(&(index as u16).to_le_bytes());
    setup[6..8].copy_from_slice(&length.to_le_bytes());
    setup
}

/// Where the fields of one HID input report live, according to the report
/// descriptor.
///
/// The touchpad interface of the Pinebook Pro keyboard chip keeps its own
/// report protocol even though it answers `SET_PROTOCOL(boot)` without a
/// stall: its reports are `[report id, buttons, dx, dy]`, not the 3-byte boot
/// layout `[buttons, dx, dy]` (experiment 13, iteration 2 — measured on
/// hardware: bytes 1/2 swapped the axes and the button bit landed in the X
/// byte). The descriptor is the only ground truth.
#[derive(Clone, Copy, Default)]
struct ReportLayout {
    /// Leading report-ID byte expected on every report (None = no IDs).
    report_id: Option<u8>,
    /// Byte offset (inside the full report, ID included) of the button bits.
    buttons_byte: Option<usize>,
    /// Number of button bits in that byte.
    buttons_bits: u32,
    /// Byte offsets of X and Y and whether the axes are absolute.
    x_byte: Option<usize>,
    y_byte: Option<usize>,
    x_absolute: bool,
    y_absolute: bool,
}

impl ReportLayout {
    /// The HID boot-mouse layout: `[buttons, dx, dy]` (appendix B.2).
    fn boot_mouse() -> Self {
        Self {
            report_id: None,
            buttons_byte: Some(0),
            buttons_bits: 3,
            x_byte: Some(1),
            y_byte: Some(2),
            x_absolute: false,
            y_absolute: false,
        }
    }

    /// The measured layout of the Pinebook Pro touchpad interface
    /// (`258a:001e` interface 1): `[report id 1, buttons, dx, dy]`, both
    /// axes signed 8-bit relative deltas. The chip answers the class
    /// `GET_DESCRIPTOR(0x22)` with a stall, so the layout cannot be read
    /// from the device; it was reconstructed from 130 raw reports
    /// (experiment 13): byte 0 constant 1, byte 1 zero until a click,
    /// bytes 2/3 change with the finger in both directions.
    fn pinebook_touchpad() -> Self {
        Self {
            report_id: Some(1),
            buttons_byte: Some(1),
            buttons_bits: 3,
            x_byte: Some(2),
            y_byte: Some(3),
            x_absolute: false,
            y_absolute: false,
        }
    }

    fn usable(&self) -> bool {
        self.buttons_byte.is_some() && self.x_byte.is_some() && self.y_byte.is_some()
    }
}

/// Parse a HID report descriptor (HID 1.11, appendix B / section 6.2.2) far
/// enough to find the first mouse-like input report: its report ID (if any),
/// the button bit field and the X/Y fields (relative or absolute).
fn parse_report_descriptor(bytes: &[u8]) -> ReportLayout {
    let mut layout = ReportLayout::default();
    let mut usage_page: u16 = 0;
    let mut usages: Vec<(u16, u16)> = Vec::new();
    let mut usage_min: Option<u16> = None;
    let mut usage_max: Option<u16> = None;
    let mut report_size: u32 = 0;
    let mut report_count: u32 = 0;
    let mut report_id: Option<u8> = None;
    let mut logical_min: i32 = 0;
    // Bit offset within the current report's data (after the ID byte).
    let mut bit_offset: u32 = 0;

    let mut i = 0;
    while i < bytes.len() {
        let prefix = bytes[i];
        if prefix == 0xfe {
            // Long item: 0xfe, bDataSize, bLongItemTag, data.
            if i + 2 >= bytes.len() {
                break;
            }
            i += 3 + usize::from(bytes[i + 1]);
            continue;
        }
        let size = match prefix & 0x03 {
            0 => 0,
            1 => 1,
            2 => 2,
            _ => 4,
        };
        let item_type = (prefix >> 2) & 0x03;
        let tag = prefix >> 4;
        if i + 1 + size > bytes.len() {
            break;
        }
        let data = &bytes[i + 1..i + 1 + size];
        i += 1 + size;
        let unsigned = match size {
            0 => 0u32,
            1 => u32::from(data[0]),
            2 => u32::from(u16::from_le_bytes([data[0], data[1]])),
            _ => u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
        };
        let signed = match size {
            0 => 0i32,
            1 => i32::from(data[0] as i8),
            2 => i32::from(i16::from_le_bytes([data[0], data[1]])),
            _ => unsigned as i32,
        };
        match (item_type, tag) {
            // Global items.
            (1, 0) => usage_page = unsigned as u16,
            (1, 1) => logical_min = signed,
            (1, 7) => report_size = unsigned,
            (1, 8) => {
                // A descriptor may define several reports; stop after the
                // first complete mouse-like one.
                if layout.usable() {
                    break;
                }
                report_id = Some(unsigned as u8);
                bit_offset = 0;
                layout = ReportLayout::default();
            }
            (1, 9) => report_count = unsigned,
            // Local items.
            (2, 0) => usages.push((usage_page, unsigned as u16)),
            (2, 1) => usage_min = Some(unsigned as u16),
            (2, 2) => usage_max = Some(unsigned as u16),
            // Main item: Input.
            (0, 8) => {
                let bits = report_size.saturating_mul(report_count);
                let id_bytes = usize::from(report_id.is_some());
                // With Report Count > 1 and several Usages, the usages share
                // the item and are packed consecutively: X at bit_offset,
                // Y at bit_offset + report_size, ...
                let mut off = bit_offset;
                for (page, usage) in usages.iter().copied() {
                    let byte = id_bytes + (off / 8) as usize;
                    match (page, usage) {
                        (0x01, 0x30) => {
                            layout.x_byte = Some(byte);
                            layout.x_absolute = logical_min >= 0;
                        }
                        (0x01, 0x31) => {
                            layout.y_byte = Some(byte);
                            layout.y_absolute = logical_min >= 0;
                        }
                        (0x09, _) => match layout.buttons_byte {
                            None => {
                                layout.buttons_byte = Some(byte);
                                layout.buttons_bits = report_count.min(8);
                            }
                            // Separate Input items, one button each: extend
                            // the bit count when the field continues there.
                            Some(b) if b == byte => {
                                layout.buttons_bits = (layout.buttons_bits + report_count).min(8);
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                    off += report_size;
                }
                // A Usage Minimum/Maximum pair over buttons (page 0x09).
                if usages.is_empty()
                    && usage_page == 0x09
                    && usage_min.is_some()
                    && usage_max.is_some()
                    && layout.buttons_byte.is_none()
                {
                    layout.buttons_byte = Some(id_bytes + (bit_offset / 8) as usize);
                    layout.buttons_bits = report_count.min(8);
                }
                bit_offset += bits;
                usages.clear();
                usage_min = None;
                usage_max = None;
            }
            // Main items that also consume report bits / reset locals.
            (0, 9) | (0, 11) => {
                bit_offset += report_size.saturating_mul(report_count);
                usages.clear();
            }
            (0, 10) | (0, 12) => {
                usages.clear();
            }
            _ => {}
        }
    }
    layout.report_id = report_id;
    layout
}

/// Parsed HID keyboard interface.
#[derive(Clone, Copy)]
struct HidInfo {
    interface: u8,
    protocol: u8,
    ep_address: u8,
    ep_number: u8,
    ep_mps: u16,
    ep_interval: u8,
}

/// Walk a configuration descriptor and return every interrupt IN endpoint
/// of every HID interface, in descriptor order. On the Pinebook Pro the
/// keyboard chip (`258a:001e`) exposes **two** of them: interface 0 is the
/// boot keyboard (0x81) and interface 1 is the touchpad (0x82) — a report
/// waiting on 0x82 that nobody reads wedges the whole chip, taking the
/// keyboard down with it (Tow-Boot issue #57, confirmed on this device in
/// experiment 8, iteration 15). Every endpoint but the keyboard must be
/// polled and drained.
fn find_hid_endpoints(config: &[u8]) -> Vec<HidInfo> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut current: Option<(u8, u8)> = None;
    while i + 2 <= config.len() {
        let len = usize::from(config[i]);
        if len < 2 || i + len > config.len() {
            break;
        }
        let ty = config[i + 1];
        if ty == 4 && len >= 9 {
            let number = config[i + 2];
            let class = config[i + 5];
            let protocol = config[i + 7];
            current = if class == CLASS_HID {
                Some((number, protocol))
            } else {
                None
            };
        } else if ty == 5 && len >= 7 {
            if let Some((interface, protocol)) = current {
                let address = config[i + 2];
                let attributes = config[i + 3];
                let is_interrupt = attributes & 0x03 == 3;
                let is_in = address & 0x80 != 0;
                if is_interrupt && is_in {
                    let mps = u16::from_le_bytes([config[i + 4], config[i + 5]]) & 0x07ff;
                    out.push(HidInfo {
                        interface,
                        protocol,
                        ep_address: address,
                        ep_number: address & 0x0f,
                        ep_mps: mps,
                        ep_interval: config[i + 6],
                    });
                }
            }
        }
        i += len;
    }
    out
}

/// What kind of boot-protocol device a polled interrupt endpoint belongs to.
#[derive(Clone, Copy, PartialEq)]
enum HidKind {
    Keyboard,
    /// Boot mouse: 3-byte reports (buttons/dx/dy) turned into input events.
    Mouse,
    /// Reports are read and dropped: for the touchpad interface of the
    /// keyboard chip (endpoint 0x82, generic HID) and any other endpoint
    /// that refuses the boot protocol — an unread report waiting there
    /// wedges the whole keyboard chip (Tow-Boot issue #57, experiment 8,
    /// iteration 15).
    Drain,
}

struct HidDevice {
    hc_index: usize,
    ed: usize,
    td: usize,
    dummy: usize,
    buf_off: usize,
    buf_phys: usize,
    mps: u16,
    slot: usize,
    addr: u8,
    ep: u8,
    low_speed: bool,
    kind: HidKind,
    /// Root hub port this device sits on; `None` for devices behind a hub
    /// (reviving those needs a hub-port reset, a separate path).
    root_port: Option<usize>,
    prev_modifiers: u8,
    prev_keys: [u8; 6],
    /// Last boot-mouse button state (bit 0/1/2 = left/right/middle).
    prev_buttons: u8,
    /// Last absolute touchpad position, for absolute→relative conversion.
    prev_abs: Option<(i32, i32)>,
    /// Where the fields of a mouse report live (report-descriptor driven).
    layout: ReportLayout,
    /// Raw reports hex-dumped to the log so far (diagnostics).
    raw_logged: u32,
    /// Completed interrupt transfers (heartbeat diagnostics).
    reports: u64,
    /// Timestamp of the last completed report (stall detection).
    last_report_ms: u64,
}

impl HidDevice {
    fn poll(&mut self, hc: &mut Ohci, input: &mut ProducerHandle, log: &Logger) {
        if !hc.interrupt_done(self.td) {
            return;
        }
        self.reports += 1;
        self.last_report_ms = ohci::now_ms();

        let cc = hc.interrupt_cc(self.td);
        let len = hc.td_transferred(self.td, self.buf_phys, usize::from(self.mps));
        let report = hc.buf_slice(self.buf_off, len).to_vec();

        if cc == ohci::TD_CC_NOERROR || cc == ohci::TD_CC_DATAUNDERRUN {
            // Ground truth for the report format: hex-dump the first reports
            // of the non-keyboard endpoints (experiment 13, iteration 2).
            if matches!(self.kind, HidKind::Mouse | HidKind::Drain) && self.raw_logged < 40 {
                self.raw_logged += 1;
                let mut line = String::new();
                for b in report.iter().take(12) {
                    line.push_str(&format!("{:02x} ", b));
                }
                log.log(format_args!(
                    "ohcid: raw ep 0x{:02x} len {}: {}",
                    self.ep, len, line
                ));
            }
            match self.kind {
                HidKind::Keyboard if len >= 8 => {
                    self.handle_report(&report, input, log);
                }
                HidKind::Mouse => {
                    self.handle_mouse_report(&report, input, log);
                }
                // Draining is the point for HidKind::Drain: the reports are
                // read and dropped so the device never accumulates
                // undelivered data (see HidKind::Drain).
                _ => {}
            }
        } else {
            log.log(format_args!(
                "ohcid: keyboard {}: interrupt transfer cc {} (halted {})",
                self.addr,
                cc,
                hc.ed(self.ed).head & 0x1 != 0
            ));
            hc.clear_interrupt_halt(self.ed);
        }

        let _ = hc.arm_interrupt(
            self.ed,
            self.td,
            self.dummy,
            self.addr,
            self.ep,
            self.low_speed,
            self.mps,
            self.buf_phys,
            self.slot,
        );
        // Keep the done queue drained even though completion is detected
        // through the TD condition code.
        let _ = hc.reap_done();
    }

    fn handle_report(&mut self, report: &[u8], input: &mut ProducerHandle, log: &Logger) {
        let modifiers = report[0];
        let mut keys = [0u8; 6];
        keys.copy_from_slice(&report[2..8]);

        // No first-report swallowing: SET_IDLE 0 makes the keyboard report
        // only on state change, so the first report IS the first key press
        // (experiment 8, iteration 7: the login's first character was eaten
        // by an "first report = idle state" assumption). The diff below runs
        // against the initial all-keys-up state, which is correct for both a
        // state-change report and a genuine idle snapshot.

        // Modifier bits map to usages 0xE0..0xE7.
        for bit in 0..8u8 {
            let was = self.prev_modifiers & (1 << bit) != 0;
            let is = modifiers & (1 << bit) != 0;
            if was != is {
                if let Some(scancode) = usage_to_scancode(0xe0 + u16::from(bit)) {
                    send_key(input, scancode, is, log);
                }
            }
        }

        for old in self.prev_keys {
            if old != 0 && !keys.contains(&old) {
                if let Some(scancode) = usage_to_scancode(u16::from(old)) {
                    send_key(input, scancode, false, log);
                }
            }
        }
        for new in keys {
            if new != 0 && !self.prev_keys.contains(&new) {
                if let Some(scancode) = usage_to_scancode(u16::from(new)) {
                    send_key(input, scancode, true, log);
                }
            }
        }

        self.prev_modifiers = modifiers;
        self.prev_keys = keys;
    }

    /// Mouse report driven by the parsed report layout: buttons at the
    /// descriptor's button byte, X/Y at theirs (signed 8-bit fields).
    /// Absolute axes (a touchpad digitizer) are turned into relative motion
    /// by diffing consecutive positions. Everything goes out as
    /// `MouseRelativeEvent`/`ButtonEvent` — the shapes orbital consumes.
    fn handle_mouse_report(&mut self, report: &[u8], input: &mut ProducerHandle, log: &Logger) {
        let layout = self.layout;
        if !layout.usable() {
            return;
        }
        if let Some(id) = layout.report_id {
            if report.is_empty() || report[0] != id {
                return;
            }
        }
        let (Some(bb), Some(xb), Some(yb)) = (layout.buttons_byte, layout.x_byte, layout.y_byte)
        else {
            return;
        };
        let last = bb.max(xb).max(yb);
        if report.len() <= last {
            return;
        }
        let mask = if layout.buttons_bits >= 8 {
            0xff
        } else {
            (1u8 << layout.buttons_bits) - 1
        };
        let buttons = report[bb] & mask;
        let raw_dx = i32::from(report[xb] as i8);
        let raw_dy = i32::from(report[yb] as i8);
        let (dx, dy) = if layout.x_absolute || layout.y_absolute {
            let cur = (i32::from(report[xb]), i32::from(report[yb]));
            let (ddx, ddy) = match self.prev_abs {
                Some(prev) => (cur.0 - prev.0, cur.1 - prev.1),
                // First absolute sample: no motion yet.
                None => (0, 0),
            };
            self.prev_abs = Some(cur);
            (
                if layout.x_absolute {
                    ddx.clamp(-127, 127)
                } else {
                    raw_dx
                },
                if layout.y_absolute {
                    ddy.clamp(-127, 127)
                } else {
                    raw_dy
                },
            )
        } else {
            (raw_dx, raw_dy)
        };

        if dx != 0 || dy != 0 {
            let event = OrbMouseRelativeEvent { dx, dy };
            if let Err(err) = input.write_event(event.to_event()) {
                log.log(format_args!("ohcid: failed to send mouse motion: {}", err));
            }
        }

        if buttons != self.prev_buttons {
            self.prev_buttons = buttons;
            let event = OrbButtonEvent {
                left: buttons & 0x1 != 0,
                right: buttons & 0x2 != 0,
                middle: buttons & 0x4 != 0,
            };
            if let Err(err) = input.write_event(event.to_event()) {
                log.log(format_args!("ohcid: failed to send mouse button: {}", err));
            }
        }
    }
}

fn send_key(input: &mut ProducerHandle, scancode: u8, pressed: bool, log: &Logger) {
    let event = OrbKeyEvent {
        character: '\0',
        scancode,
        pressed,
    };
    if let Err(err) = input.write_event(event.to_event()) {
        log.log(format_args!("ohcid: failed to send key event: {}", err));
    }
}

/// HID usage (page 0x07) to `orbclient` scancode, same table as `usbhidd`.
fn usage_to_scancode(usage: u16) -> Option<u8> {
    Some(match usage {
        0x04 => orbclient::K_A,
        0x05 => orbclient::K_B,
        0x06 => orbclient::K_C,
        0x07 => orbclient::K_D,
        0x08 => orbclient::K_E,
        0x09 => orbclient::K_F,
        0x0A => orbclient::K_G,
        0x0B => orbclient::K_H,
        0x0C => orbclient::K_I,
        0x0D => orbclient::K_J,
        0x0E => orbclient::K_K,
        0x0F => orbclient::K_L,
        0x10 => orbclient::K_M,
        0x11 => orbclient::K_N,
        0x12 => orbclient::K_O,
        0x13 => orbclient::K_P,
        0x14 => orbclient::K_Q,
        0x15 => orbclient::K_R,
        0x16 => orbclient::K_S,
        0x17 => orbclient::K_T,
        0x18 => orbclient::K_U,
        0x19 => orbclient::K_V,
        0x1A => orbclient::K_W,
        0x1B => orbclient::K_X,
        0x1C => orbclient::K_Y,
        0x1D => orbclient::K_Z,
        0x1E => orbclient::K_1,
        0x1F => orbclient::K_2,
        0x20 => orbclient::K_3,
        0x21 => orbclient::K_4,
        0x22 => orbclient::K_5,
        0x23 => orbclient::K_6,
        0x24 => orbclient::K_7,
        0x25 => orbclient::K_8,
        0x26 => orbclient::K_9,
        0x27 => orbclient::K_0,
        0x28 => orbclient::K_ENTER,
        0x29 => orbclient::K_ESC,
        0x2A => orbclient::K_BKSP,
        0x2B => orbclient::K_TAB,
        0x2C => orbclient::K_SPACE,
        0x2D => orbclient::K_MINUS,
        0x2E => orbclient::K_EQUALS,
        0x2F => orbclient::K_BRACE_OPEN,
        0x30 => orbclient::K_BRACE_CLOSE,
        0x31 => orbclient::K_BACKSLASH,
        0x32 => 0x56,
        0x33 => orbclient::K_SEMICOLON,
        0x34 => orbclient::K_QUOTE,
        0x35 => orbclient::K_TICK,
        0x36 => orbclient::K_COMMA,
        0x37 => orbclient::K_PERIOD,
        0x38 => orbclient::K_SLASH,
        0x39 => orbclient::K_CAPS,
        0x3A => orbclient::K_F1,
        0x3B => orbclient::K_F2,
        0x3C => orbclient::K_F3,
        0x3D => orbclient::K_F4,
        0x3E => orbclient::K_F5,
        0x3F => orbclient::K_F6,
        0x40 => orbclient::K_F7,
        0x41 => orbclient::K_F8,
        0x42 => orbclient::K_F9,
        0x43 => orbclient::K_F10,
        0x44 => orbclient::K_F11,
        0x45 => orbclient::K_F12,
        0x46 => orbclient::K_PRTSC,
        0x47 => orbclient::K_SCROLL,
        0x49 => orbclient::K_INS,
        0x4A => orbclient::K_HOME,
        0x4B => orbclient::K_PGUP,
        0x4C => orbclient::K_DEL,
        0x4D => orbclient::K_END,
        0x4E => orbclient::K_PGDN,
        0x4F => orbclient::K_RIGHT,
        0x50 => orbclient::K_LEFT,
        0x51 => orbclient::K_DOWN,
        0x52 => orbclient::K_UP,
        0x53 => orbclient::K_NUM,
        0x54 => orbclient::K_NUM_SLASH,
        0x55 => orbclient::K_NUM_ASTERISK,
        0x56 => orbclient::K_NUM_MINUS,
        0x57 => orbclient::K_NUM_PLUS,
        0x58 => orbclient::K_NUM_ENTER,
        0x59 => orbclient::K_NUM_1,
        0x5A => orbclient::K_NUM_2,
        0x5B => orbclient::K_NUM_3,
        0x5C => orbclient::K_NUM_4,
        0x5D => orbclient::K_NUM_5,
        0x5E => orbclient::K_NUM_6,
        0x5F => orbclient::K_NUM_7,
        0x60 => orbclient::K_NUM_8,
        0x61 => orbclient::K_NUM_9,
        0x62 => orbclient::K_NUM_0,
        0x64 => orbclient::K_APP,
        0x66 => orbclient::K_POWER,
        0xE0 => orbclient::K_LEFT_CTRL,
        0xE1 => orbclient::K_LEFT_SHIFT,
        0xE2 => orbclient::K_ALT,
        0xE3 => orbclient::K_LEFT_SUPER,
        0xE4 => orbclient::K_RIGHT_CTRL,
        0xE5 => orbclient::K_RIGHT_SHIFT,
        0xE6 => orbclient::K_ALT_GR,
        0xE7 => orbclient::K_RIGHT_SUPER,
        _ => return None,
    })
}
