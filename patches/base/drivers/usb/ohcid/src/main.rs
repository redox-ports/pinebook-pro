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
use orbclient::KeyEvent as OrbKeyEvent;

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
        loop {
            for hid in hids.iter_mut() {
                hid.poll(&mut hcs[hid.hc_index], input, log);
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

        let Some(info) = find_hid_keyboard(&config) else {
            self.log
                .log(format_args!("ohcid:   no HID keyboard interface, ignored"));
            return Ok(());
        };

        self.log.log(format_args!(
            "ohcid:   HID interface {} protocol {} endpoint 0x{:02x} mps {} interval {}",
            info.interface, info.protocol, info.ep_address, info.ep_mps, info.ep_interval
        ));
        if info.protocol == 2 {
            self.log.log(format_args!(
                "ohcid:   boot mouse interface, report parser not implemented yet"
            ));
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
        self.start_hid(addr, low_speed, info)?;
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
        if depth >= 1 {
            self.log
                .log(format_args!("ohcid:   nested hub ignored (one level only)"));
            return Ok(());
        }
        self.set_configuration(addr, low_speed, u16::from(mps0), 1)?;

        let mut hub = [0u8; 9];
        let got =
            self.get_class_descriptor(addr, low_speed, u16::from(mps0), DESC_HUB, 0, &mut hub)?;
        if got < 3 {
            return Err(Error::Protocol("short hub descriptor"));
        }
        let ports = hub[2];
        self.log
            .log(format_args!("ohcid:   hub with {} port(s)", ports));

        for port in 1..=ports {
            let (status, change) = self.hub_port_status(addr, low_speed, mps0, port)?;
            if change & PORT_CONNECTION_CHANGE != 0 {
                let _ = self.hub_clear_feature(addr, low_speed, mps0, port, FEAT_C_PORT_CONNECTION);
            }
            if status & PORT_CONNECTION == 0 {
                continue;
            }

            let _ = self.hub_set_feature(addr, low_speed, mps0, port, FEAT_PORT_POWER);
            self.hc.delay_ms(100);

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
            // One keyboard is enough for the bring-up milestone.
            if !self.found.is_empty() {
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

    fn start_hid(&mut self, addr: u8, low_speed: bool, info: HidInfo) -> Result<(), Error> {
        let ed = self.next_ed;
        let td = self.next_td;
        let dummy = self.next_td + 1;
        let slot = self.next_slot % 32;
        self.next_ed += 1;
        self.next_td += 2;
        self.next_slot += 1;

        let mps = info.ep_mps;
        let (buf_off, buf_phys) = self.hc.alloc_buf(usize::from(mps))?;
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
            "ohcid:   polling keyboard addr {} endpoint {} via periodic slot {}",
            addr, info.ep_number, slot
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
            prev_modifiers: 0,
            prev_keys: [0; 6],
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

/// Walk a configuration descriptor and return the first HID interface that
/// has an interrupt IN endpoint. Boot keyboards use subclass 1; some devices
/// report subclass 0, so only the class is required here.
fn find_hid_keyboard(config: &[u8]) -> Option<HidInfo> {
    let mut i = 0usize;
    let mut current: Option<(u8, u8)> = None;
    let mut fallback: Option<HidInfo> = None;
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
                    let info = HidInfo {
                        interface,
                        protocol,
                        ep_address: address,
                        ep_number: address & 0x0f,
                        ep_mps: mps,
                        ep_interval: config[i + 6],
                    };
                    // A boot keyboard (protocol 1) is the best match; other
                    // HID interfaces (consumer control, mouse) are only kept
                    // as a fallback.
                    if protocol == 1 {
                        return Some(info);
                    }
                    if fallback.is_none() {
                        fallback = Some(info);
                    }
                }
            }
        }
        i += len;
    }
    fallback
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
    prev_modifiers: u8,
    prev_keys: [u8; 6],
}

impl HidDevice {
    fn poll(&mut self, hc: &mut Ohci, input: &mut ProducerHandle, log: &Logger) {
        if !hc.interrupt_done(self.td) {
            return;
        }

        let cc = hc.interrupt_cc(self.td);
        let len = hc.td_transferred(self.td, self.buf_phys, usize::from(self.mps));
        let report = hc.buf_slice(self.buf_off, len).to_vec();

        if cc == ohci::TD_CC_NOERROR || cc == ohci::TD_CC_DATAUNDERRUN {
            if len >= 8 {
                self.handle_report(&report, input, log);
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
