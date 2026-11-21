//! Minimal Open Host Controller Interface (OHCI) driver core.
//!
//! Register layout and descriptor bit fields follow the OHCI 1.0a
//! specification and the Linux `drivers/usb/host/ohci.h` definitions, which
//! is also what U-Boot uses. Only what is needed to enumerate full/low speed
//! devices and poll one interrupt endpoint is implemented: there is no
//! interrupt handling (the controller is driven by polling) and no
//! isochronous or bulk scheduling.
//!
//! The RK3399 has two USB2 host controllers. Each is an EHCI/OHCI pair; the
//! installed U-Boot only brings up the OHCI halves (its boot log lists
//! `Bus usb@fe3a0000: USB OHCI 1.0` / `Bus usb@fe3e0000: USB OHCI 1.0` and no
//! EHCI bus), so the whole internal USB tree — hub, keyboard, touchpad —
//! enumerates as full/low speed devices on the companion controllers.

use core::mem::size_of;
use core::ptr;

use common::dma::Dma;

use crate::log::Logger;

/// Clean the data cache over a virtual range so a non-snooping USB
/// controller reads the freshly written descriptor/buffer content.
///
/// The DMA memory is requested as Uncacheable, but if the kernel mapping
/// does not honour that attribute (or the allocation lands on cacheable
/// pages) the controller reads stale zeros — exactly the enumeration
/// symptom seen on hardware (experiment 8, iteration 2). DC CVAC/DSB are
/// EL0-accessible; on other architectures this is a no-op.
#[cfg(target_arch = "aarch64")]
fn cache_clean_range(virt: usize, len: usize) {
    unsafe {
        let ctr: u64;
        core::arch::asm!("mrs {}, ctr_el0", out(reg) ctr, options(nomem, nostack));
        let dline = (4usize) << ((ctr >> 16) & 0xf);
        let mut a = virt & !(dline - 1);
        let end = (virt + len + dline - 1) & !(dline - 1);
        while a < end {
            core::arch::asm!("dc cvac, {}", in(reg) a, options(nostack));
            a += dline;
        }
        core::arch::asm!("dsb sy", options(nostack));
    }
}

#[cfg(not(target_arch = "aarch64"))]
fn cache_clean_range(_virt: usize, _len: usize) {}

/// Invalidate the data cache over a virtual range so the CPU reads the data
/// a non-snooping controller wrote back instead of its own stale lines.
///
/// Uses DC CIVAC (clean + invalidate): plain DC IVAC is EL1-restricted
/// (SCTLR_EL1.UCT) and faults from userspace.
#[cfg(target_arch = "aarch64")]
fn cache_invalidate_range(virt: usize, len: usize) {
    unsafe {
        let ctr: u64;
        core::arch::asm!("mrs {}, ctr_el0", out(reg) ctr, options(nomem, nostack));
        let dline = (4usize) << ((ctr >> 16) & 0xf);
        let mut a = virt & !(dline - 1);
        let end = (virt + len + dline - 1) & !(dline - 1);
        while a < end {
            core::arch::asm!("dc civac, {}", in(reg) a, options(nostack));
            a += dline;
        }
        core::arch::asm!("dsb sy", "isb", options(nostack));
    }
}

#[cfg(not(target_arch = "aarch64"))]
fn cache_invalidate_range(_virt: usize, _len: usize) {}

// Register offsets (OHCI spec section 7, `struct ohci_regs`).
pub const REG_REVISION: usize = 0x00;
pub const REG_CONTROL: usize = 0x04;
pub const REG_CMDSTATUS: usize = 0x08;
pub const REG_INTRSTATUS: usize = 0x0c;
pub const REG_INTRDISABLE: usize = 0x14;
pub const REG_HCCA: usize = 0x18;
pub const REG_ED_CONTROLHEAD: usize = 0x20;
pub const REG_ED_CONTROLCURRENT: usize = 0x24;
pub const REG_ED_BULKHEAD: usize = 0x28;
pub const REG_ED_BULKCURRENT: usize = 0x2c;
pub const REG_DONEHEAD: usize = 0x30;
pub const REG_FMINTERVAL: usize = 0x34;
pub const REG_PERIODICSTART: usize = 0x40;
pub const REG_LSTHRESH: usize = 0x44;
pub const REG_RH_A: usize = 0x48;
pub const REG_RH_STATUS: usize = 0x50;
/// `HcRhPortStatus[0]`; port numbers are 0-based in the register map.
pub const REG_RH_PORTSTATUS: usize = 0x54;

// HcControl bits.
const CTRL_CBSR: u32 = 0x3;
const CTRL_PLE: u32 = 1 << 2;
const CTRL_CLE: u32 = 1 << 4;
const CTRL_BLE: u32 = 1 << 5;
const CTRL_HCFS_OPER: u32 = 2 << 6;

// HcCommandStatus bits.
const CMD_HCR: u32 = 1 << 0;
const CMD_CLF: u32 = 1 << 1;

// HcInterruptStatus bits.
const INTR_WDH: u32 = 1 << 1;

// Root hub registers, per `ohci.h`.
const RH_A_NDP: u32 = 0xff;
const RH_A_PSM: u32 = 1 << 8;
const RH_A_NPS: u32 = 1 << 9;

const RH_PS_CCS: u32 = 0x0000_0001;
const RH_PS_PES: u32 = 0x0000_0002;
const RH_PS_PRS: u32 = 0x0000_0010;
const RH_PS_PPS: u32 = 0x0000_0100;
const RH_PS_LSDA: u32 = 0x0000_0200;
const RH_PS_CSC: u32 = 0x0001_0000;
const RH_PS_PRSC: u32 = 0x0010_0000;

const RH_HS_LPSC: u32 = 0x0001_0000;

// ED control word (`ED Control` dword, OHCI spec table 4-4).
const ED_LOWSPEED: u32 = 1 << 13;
const ED_IN: u32 = 0x02 << 11;
const ED_OUT: u32 = 0x01 << 11;

// TD control word (`TD Control` dword, OHCI spec table 4-6).
const TD_CC: u32 = 0xf000_0000;
const TD_CC_NOT_ACCESSED: u32 = 0x0f;
const TD_T_DATA0: u32 = 0x0200_0000;
const TD_T_DATA1: u32 = 0x0300_0000;
const TD_T_TOGGLE: u32 = 0x0000_0000;
const TD_DP_SETUP: u32 = 0x0000_0000;
const TD_DP_OUT: u32 = 0x0008_0000;
const TD_DP_IN: u32 = 0x0010_0000;
const TD_R: u32 = 0x0004_0000;

/// Completion codes that mean "the transfer ended as expected".
pub const TD_CC_NOERROR: u32 = 0x00;
pub const TD_CC_DATAUNDERRUN: u32 = 0x09;

/// One endpoint descriptor (16 bytes, 16-byte aligned).
#[repr(C, align(16))]
#[derive(Clone, Copy, Default)]
pub struct Ed {
    pub info: u32,
    pub tail: u32,
    pub head: u32,
    pub next: u32,
}

/// One transfer descriptor (16 bytes, 16-byte aligned).
#[repr(C, align(16))]
#[derive(Clone, Copy, Default)]
pub struct Td {
    pub info: u32,
    pub cbp: u32,
    pub next: u32,
    pub be: u32,
}

/// Errors reported by the controller or by the driver itself.
///
/// The payloads are only used by the derived `Debug` implementation (they
/// end up in the UART log), which dead-code analysis ignores.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum Error {
    /// A register never reached the expected state.
    Timeout(&'static str),
    /// The transfer descriptor completed with a non-zero condition code.
    Td(u32),
    /// The endpoint was halted (ED HeadP.H) during the transfer.
    Halted,
    /// The device answered, but the data does not fit the assumptions.
    Protocol(&'static str),
    /// The DMA arena ran out of space (the driver allocates once, up front).
    OutOfMemory,
}

pub type Result<T> = core::result::Result<T, Error>;

/// Number of endpoint descriptors in the preallocated pool.
const ED_POOL: usize = 16;
/// Number of transfer descriptors in the preallocated pool.
const TD_POOL: usize = 64;
/// Bytes of shared buffer space for setup packets, descriptors and reports.
const BUF_POOL: usize = 4096;

/// Result of the setup-buffer diagnostics (see `Ohci::diag_setup_buffer`).
pub struct SetupBufferDiag {
    /// Virtual address of the setup buffer area.
    pub va: usize,
    /// The first TD as the CPU sees it (retired values if a transfer ran).
    pub td0: [u32; 4],
    /// The setup area as the failed transfer left it.
    pub original: Vec<u8>,
    /// Pattern written to the setup area and read back through the Dma map.
    pub write_readback: Vec<u8>,
    /// The same area read through an independent Uncacheable physmap.
    pub phys_view: Option<Vec<u8>>,
    /// Canary written to an untouched tail offset and read back.
    pub canary: Vec<u8>,
}

#[inline]
fn ed_info(addr: u8, endpoint: u8, low_speed: bool, dir: u32, mps: u16) -> u32 {
    u32::from(addr & 0x7f)
        | (u32::from(endpoint & 0x0f) << 7)
        | dir
        | if low_speed { ED_LOWSPEED } else { 0 }
        | (u32::from(mps & 0x07ff) << 16)
}

pub struct Ohci {
    regs: usize,
    hcca: Dma<[u32; 64]>,
    eds: Dma<[Ed]>,
    tds: Dma<[Td]>,
    bufs: Dma<[u8]>,
    next_buf: usize,
}

impl Ohci {
    /// Map the controller registers and allocate the descriptors.
    ///
    /// # Safety
    /// `base` must be the MMIO base of an OHCI controller.
    pub unsafe fn new(base: usize, log: &Logger) -> Result<Self> {
        log.log(format_args!(
            "ohcid: 0x{:08x}: mapping the register window",
            base
        ));
        // Some firmware places controllers at unaligned BARs (the QEMU
        // pci-ohci rig lands the second controller at +0x100 inside the
        // first one's page); physmap needs a page-aligned address.
        let page = base & !0xfff;
        let off = base & 0xfff;
        let map = unsafe {
            common::physmap(
                page,
                0x1000,
                common::Prot::RW,
                common::MemoryType::DeviceMemory,
            )
        }
        .map_err(|_| Error::Protocol("cannot map OHCI registers"))? as usize;
        let regs = map + off;
        log.log(format_args!(
            "ohcid: 0x{:08x}: registers at 0x{:08x}",
            base, regs
        ));

        let hcca = Dma::new([0u32; 64]).map_err(|_| Error::OutOfMemory)?;
        log.log(format_args!(
            "ohcid: 0x{:08x}: hcca phys 0x{:08x}",
            base,
            hcca.physical()
        ));
        let eds = unsafe {
            Dma::<[Ed]>::zeroed_slice(ED_POOL)
                .map_err(|_| Error::OutOfMemory)?
                .assume_init()
        };
        log.log(format_args!(
            "ohcid: 0x{:08x}: {} eds phys 0x{:08x}",
            base,
            ED_POOL,
            eds.physical()
        ));
        let tds = unsafe {
            Dma::<[Td]>::zeroed_slice(TD_POOL)
                .map_err(|_| Error::OutOfMemory)?
                .assume_init()
        };
        log.log(format_args!(
            "ohcid: 0x{:08x}: {} tds phys 0x{:08x}",
            base,
            TD_POOL,
            tds.physical()
        ));
        let bufs = unsafe {
            Dma::<[u8]>::zeroed_slice(BUF_POOL)
                .map_err(|_| Error::OutOfMemory)?
                .assume_init()
        };
        log.log(format_args!(
            "ohcid: 0x{:08x}: {} byte buffer phys 0x{:08x}",
            base,
            BUF_POOL,
            bufs.physical()
        ));

        Ok(Ohci {
            regs,
            hcca,
            eds,
            tds,
            bufs,
            next_buf: 0,
        })
    }

    #[inline]
    pub fn read(&self, offset: usize) -> u32 {
        unsafe { ptr::read_volatile((self.regs + offset) as *const u32) }
    }

    #[inline]
    pub fn write(&self, offset: usize, value: u32) {
        unsafe { ptr::write_volatile((self.regs + offset) as *mut u32, value) }
    }

    pub fn revision(&self) -> u32 {
        self.read(REG_REVISION)
    }

    /// Bring the controller into the USB Operational state with empty
    /// schedules. Mirrors `ohci_run()` from Linux.
    pub fn init(&mut self) -> Result<()> {
        // Force the USB Reset state and let the bus settle, like
        // `ohci_run()` in Linux (50 ms), before the controller reset.
        self.write(REG_CONTROL, 0);
        self.delay_ms(50);
        self.write(REG_CMDSTATUS, CMD_HCR);

        let deadline = now_ms() + 100;
        while self.read(REG_CMDSTATUS) & CMD_HCR != 0 {
            if now_ms() > deadline {
                return Err(Error::Timeout("HcCommandStatus.HCR"));
            }
            core::hint::spin_loop();
        }

        let hcca_phys = self.hcca.physical();
        if hcca_phys & 0xff != 0 {
            return Err(Error::Protocol("HCCA is not 256-byte aligned"));
        }

        self.write(REG_INTRDISABLE, 0xffff_ffff);
        self.write(REG_INTRSTATUS, 0xffff_ffff);

        self.write(REG_ED_CONTROLHEAD, 0);
        self.write(REG_ED_CONTROLCURRENT, 0);
        self.write(REG_ED_BULKHEAD, 0);
        self.write(REG_ED_BULKCURRENT, 0);
        self.write(REG_HCCA, hcca_phys as u32);
        // The HC writes frame/done-head back into the HCCA through DMA.
        cache_clean_range(self.hcca.virt() as usize, 256);

        // Frame timing: 11999 means one frame every 1 ms. FSMPS (largest
        // packet) and PeriodicStart follow Linux `periodic_reinit()`.
        let fi = 0x2edfu32;
        let fsmps = (fi - 210) * 6 / 7;
        self.write(REG_FMINTERVAL, (fsmps << 16) | fi);
        self.write(REG_PERIODICSTART, fsmps * 9 / 10);
        self.write(REG_LSTHRESH, 0x628);

        // Go operational with all three lists enabled; the driver toggles
        // CLE per control transfer and keeps PLE on for the interrupt ED.
        self.write(
            REG_CONTROL,
            CTRL_CBSR | CTRL_PLE | CTRL_CLE | CTRL_BLE | CTRL_HCFS_OPER,
        );

        self.delay_ms(2);
        Ok(())
    }

    pub fn num_ports(&self) -> usize {
        (self.read(REG_RH_A) & RH_A_NDP) as usize
    }

    /// Power the root hub ports if the controller needs it.
    pub fn power_ports(&mut self) -> Result<()> {
        let rh_a = self.read(REG_RH_A);
        if rh_a & RH_A_NPS != 0 {
            // No power switching: ports are always powered.
            return Ok(());
        }
        if rh_a & RH_A_PSM != 0 {
            // Per-port power switching.
            for port in 0..self.num_ports() {
                let reg = REG_RH_PORTSTATUS + 4 * port;
                if self.read(reg) & RH_PS_PPS == 0 {
                    self.write(reg, RH_PS_PPS);
                }
            }
        } else {
            // Ganged power switching: one global power-on command.
            self.write(REG_RH_STATUS, RH_HS_LPSC);
        }
        // Power-on-to-power-good is at most 0xff * 2 ms.
        self.delay_ms(200);
        Ok(())
    }

    pub fn port_status(&self, port: usize) -> u32 {
        self.read(REG_RH_PORTSTATUS + 4 * port)
    }

    /// Reset a root hub port; returns `(connected, low_speed, enabled)`.
    pub fn reset_root_port(&mut self, port: usize) -> Result<(bool, bool, bool)> {
        let reg = REG_RH_PORTSTATUS + 4 * port;

        let status = self.read(reg);
        if status & RH_PS_CSC != 0 {
            self.write(reg, RH_PS_CSC);
        }
        if status & RH_PS_CCS == 0 {
            return Ok((false, false, false));
        }

        self.write(reg, RH_PS_PRS);
        let deadline = now_ms() + 500;
        loop {
            let status = self.read(reg);
            if status & RH_PS_PRSC != 0 {
                // Reset finished; acknowledge both change bits.
                self.write(reg, RH_PS_PRSC);
                if status & RH_PS_CSC != 0 {
                    self.write(reg, RH_PS_CSC);
                }
                let mut status = self.read(reg);
                // The OHCI root hub leaves enabling to software: after a
                // successful reset the port must be enabled explicitly
                // (SetPortFeature(PORT_ENABLE) == writing PES).
                if status & RH_PS_PES == 0 {
                    self.write(reg, RH_PS_PES);
                    self.delay_ms(10);
                    status = self.read(reg);
                }
                let connected = status & RH_PS_CCS != 0;
                let low_speed = status & RH_PS_LSDA != 0;
                let enabled = status & RH_PS_PES != 0;
                return Ok((connected, low_speed, enabled));
            }
            if now_ms() > deadline {
                return Err(Error::Timeout("root port reset"));
            }
            self.delay_ms(1);
        }
    }

    pub fn delay_ms(&self, ms: u64) {
        // The bring-up kernel does not wake `nanosleep` on the generic timer
        // (README risk 13), so every delay is a busy-wait.
        let target = now_ms() + ms;
        while now_ms() < target {
            core::hint::spin_loop();
        }
    }

    // --- descriptor pools -------------------------------------------------

    pub fn ed(&self, index: usize) -> &Ed {
        &self.eds[index]
    }

    fn ed_mut(&mut self, index: usize) -> &mut Ed {
        &mut self.eds[index]
    }

    pub fn ed_phys(&self, index: usize) -> usize {
        self.eds.physical() + index * size_of::<Ed>()
    }

    fn td_mut(&mut self, index: usize) -> &mut Td {
        &mut self.tds[index]
    }

    pub fn td_phys(&self, index: usize) -> usize {
        self.tds.physical() + index * size_of::<Td>()
    }

    /// Reserve `len` bytes in the shared buffer arena and return
    /// `(offset, physical)`. The arena is allocated once and never reused.
    ///
    /// DIAGNOSTIC (experiment 8, iteration 2): the buffers live in the tail
    /// of the TD page instead of their own allocation. The HC reads our TDs
    /// from the TD page correctly, but saw stale zeros in the separate
    /// phys_contiguous buffer page; if transfers work from this page, the
    /// separate allocation is the problem.
    pub fn alloc_buf(&mut self, len: usize) -> Result<(usize, usize)> {
        const TD_BUF_OFF: usize = 1024; // 64 TDs x 16 bytes
        let off = self.next_buf;
        let end = off + len.max(1);
        if TD_BUF_OFF + end > 4096 {
            return Err(Error::OutOfMemory);
        }
        self.next_buf = TD_BUF_OFF + ((end + 15) & !15);
        Ok((off, self.tds.physical() + TD_BUF_OFF + off))
    }

    fn td_page_bytes_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.tds.virt() as *mut u8, 4096) }
    }

    pub fn buf_slice_mut(&mut self, off: usize, len: usize) -> &mut [u8] {
        const TD_BUF_OFF: usize = 1024;
        let base = self.td_page_bytes_mut();
        &mut base[TD_BUF_OFF + off..TD_BUF_OFF + off + len]
    }

    pub fn buf_slice(&self, off: usize, len: usize) -> &[u8] {
        const TD_BUF_OFF: usize = 1024;
        let base = unsafe { core::slice::from_raw_parts(self.tds.virt() as *const u8, 4096) };
        &base[TD_BUF_OFF + off..TD_BUF_OFF + off + len]
    }

    /// Physical base address of the shared buffer arena (diagnostics).
    pub fn bufs_physical(&self) -> usize {
        const TD_BUF_OFF: usize = 1024;
        self.tds.physical() + TD_BUF_OFF
    }

    /// Read buffer bytes through the Dma mapping, volatile (diagnostics).
    pub fn buf_bytes_volatile(&self, off: usize, len: usize) -> Vec<u8> {
        const TD_BUF_OFF: usize = 1024;
        let base = unsafe { core::slice::from_raw_parts(self.tds.virt() as *const u8, 4096) };
        (0..len)
            .map(|i| unsafe { ptr::read_volatile(base.as_ptr().add(TD_BUF_OFF + off + i)) })
            .collect()
    }

    /// Read buffer bytes through an independent mapping of the same
    /// physical page (diagnostics: catches virt-to-phys divergence and cache
    /// attribute mismatches). RAM ranges are mapped Uncacheable; the device
    /// type is refused for RAM by the kernel.
    pub fn buf_bytes_via_physmap(&self, off: usize, len: usize) -> Option<Vec<u8>> {
        const TD_BUF_OFF: usize = 1024;
        let phys = self.tds.physical() + TD_BUF_OFF + off;
        let map = unsafe {
            common::physmap(phys, len, common::Prot::RW, common::MemoryType::Uncacheable)
        }
        .ok()?;
        let mut out = Vec::with_capacity(len);
        for i in 0..len {
            out.push(unsafe { ptr::read_volatile((map as usize + i) as *const u8) });
        }
        Some(out)
    }

    /// Diagnostics for the setup-buffer-zeros anomaly on the second hardware
    /// controller (experiment 8, iteration 7): the HC walks the TDs at the
    /// head of the TD page correctly but transmits zeros from the buffer
    /// tail of the very same page, and the driver read-back sees zeros too.
    /// Discriminates a write-broken mapping from a lost single write.
    pub fn diag_setup_buffer(&mut self) -> SetupBufferDiag {
        const TD_BUF_OFF: usize = 1024;
        let va = self.tds.virt_addr() + TD_BUF_OFF;
        let td0 = [
            self.tds[0].info,
            self.tds[0].cbp,
            self.tds[0].next,
            self.tds[0].be,
        ];

        // Preserve what the last transfer left in the setup area before the
        // pattern write below destroys the evidence.
        let original = self.buf_bytes_volatile(0, 8);

        // Write a known pattern into the setup area and read it back
        // through the same mapping.
        let pattern = [0x5au8, 0xa5, 0x3c, 0xc3, 0x0f, 0xf0, 0x55, 0xaa];
        self.buf_slice_mut(0, 8).copy_from_slice(&pattern);
        let wb = self.buf_bytes_volatile(0, 8);

        // Same through an independent Uncacheable mapping of the phys.
        let phys_view = self.buf_bytes_via_physmap(0, 8);

        // Canary in the untouched middle of the tail.
        self.buf_slice_mut(2048, 4)
            .copy_from_slice(&[0xc3u8, 0x3c, 0xa5, 0x5a]);
        let canary = self.buf_bytes_volatile(2048, 4);

        SetupBufferDiag {
            va,
            td0,
            original,
            write_readback: wb,
            phys_view,
            canary,
        }
    }

    /// Fill one general TD and link it to `next`.
    fn fill_td(&mut self, index: usize, info: u32, buf_phys: usize, len: usize, next: usize) {
        let td = self.td_mut(index);
        td.info = info;
        td.cbp = if len == 0 { 0 } else { buf_phys as u32 };
        td.next = next as u32;
        td.be = if len == 0 {
            0
        } else {
            (buf_phys + len - 1) as u32
        };
    }

    /// Put the control ED at the head of the control list and kick it.
    fn start_control(&mut self, ed_index: usize) {
        let ed_phys = self.ed_phys(ed_index);
        let control = self.read(REG_CONTROL);
        self.write(REG_CONTROL, control & !CTRL_CLE);
        self.write(REG_ED_CONTROLHEAD, ed_phys as u32);
        self.write(REG_ED_CONTROLCURRENT, 0);
        self.write(REG_CONTROL, (control & !CTRL_CLE) | CTRL_CLE);
        // Tell the controller the control list has been refilled.
        self.write(REG_CMDSTATUS, CMD_CLF);
    }

    fn stop_control(&mut self) {
        let control = self.read(REG_CONTROL);
        self.write(REG_CONTROL, control & !CTRL_CLE);
        self.write(REG_ED_CONTROLHEAD, 0);
    }

    /// Run one control transfer on the default pipe (endpoint 0).
    ///
    /// `data_in` selects the direction of the optional data stage; `setup`
    /// holds the 8-byte USB setup packet. Returns the number of bytes in the
    /// data stage (for IN transfers the data is copied back into `data`).
    pub fn control_transfer(
        &mut self,
        addr: u8,
        low_speed: bool,
        mps: u16,
        setup: [u8; 8],
        data: Option<&mut [u8]>,
        data_in: bool,
    ) -> Result<usize> {
        let (setup_off, setup_phys) = self.alloc_buf(8)?;
        self.buf_slice_mut(setup_off, 8).copy_from_slice(&setup);
        // DIAGNOSTIC (experiment 8, iteration 2): the HC reads zeros from
        // this buffer although the CPU view shows the packet; log the exact
        // physical address so the RAM can be inspected from the QEMU monitor
        // during the diagnostic delay below.
        println!(
            "ohcid:   dbg: setup phys 0x{:08x} bytes {:02x?}",
            setup_phys, setup
        );

        let data_len = data.as_ref().map_or(0, |d| d.len());
        let (data_off, data_phys) = if data_len > 0 {
            let (off, phys) = self.alloc_buf(data_len)?;
            if !data_in {
                if let Some(src) = data.as_deref() {
                    self.buf_slice_mut(off, data_len).copy_from_slice(src);
                }
            }
            (Some(off), phys)
        } else {
            (None, 0)
        };

        // Descriptor slots 0..3 are reserved for the control pipe.
        let ed_index = 0;
        let td_setup = 0;
        let td_data = 1;
        let td_status = 2;
        let td_dummy = 3;

        let status_next = self.td_phys(td_status);
        if data_len > 0 {
            self.fill_td(
                td_setup,
                TD_CC | TD_DP_SETUP | TD_T_DATA0,
                setup_phys,
                8,
                self.td_phys(td_data),
            );
            let info = TD_CC | TD_R | TD_T_DATA1 | if data_in { TD_DP_IN } else { TD_DP_OUT };
            self.fill_td(td_data, info, data_phys, data_len, status_next);
        } else {
            self.fill_td(
                td_setup,
                TD_CC | TD_DP_SETUP | TD_T_DATA0,
                setup_phys,
                8,
                status_next,
            );
        }

        // Status stage: opposite direction to the data stage, DATA1,
        // zero length. With no data stage the status stage is an IN.
        // (Linux `td_submit_urb`: `(is_out || !data_len) ? DP_IN : DP_OUT`.)
        let status_info = if data_in && data_len > 0 {
            TD_CC | TD_DP_OUT | TD_T_DATA1
        } else {
            TD_CC | TD_DP_IN | TD_T_DATA1
        };
        self.fill_td(td_status, status_info, 0, 0, self.td_phys(td_dummy));
        self.fill_td(td_dummy, 0, 0, 0, 0);

        let head = self.td_phys(td_setup) as u32;
        let tail = self.td_phys(td_dummy) as u32;
        {
            let ed = self.ed_mut(ed_index);
            ed.info = ed_info(
                addr, 0, low_speed,
                // Control-ED direction field must be 0 (U-Boot/Linux: the
                // direction is carried by each TD, and the SETUP token only
                // exists as TD_DP_SETUP). A non-zero ED.D makes the transaction
                // direction come from the ED — an IN-ed pair with a SETUP TD
                // sends an IN token instead of SETUP and the device STALLs
                // (experiment 8, iteration 4).
                0, /* direction comes from the TDs */
                mps,
            );
            ed.head = head;
            ed.tail = tail;
            ed.next = 0;
        }

        // Push every byte the controller will read out of the caches: the
        // ED page, and the TD page (the TDs live in its first KiB, the
        // shared buffers in its tail). DC CVAC translates its operand: the
        // address must be the Dma *virtual* mapping — passing the physical
        // address faults on real hardware (unmapped VA, "Lacks grant",
        // experiment 8 iteration 6; QEMU TCG skips the translation, which is
        // why the rig never caught it).
        cache_clean_range(
            self.eds.virt_addr() + ed_index * size_of::<Ed>(),
            size_of::<Ed>(),
        );
        cache_clean_range(self.tds.virt_addr(), 4096);

        self.start_control(ed_index);
        let result = self.wait_td(ed_index, td_status, 500);
        if result.is_err() {
            // Dump the retired TDs and the ED: the condition code in the
            // retired TD tells what the transaction ended with (STALL vs
            // data-toggle mismatch vs device-not-responding) — the closest
            // thing to a bus trace on real hardware.
            let ed_snapshot = self.ed(ed_index);
            println!(
                "ohcid:   dbg: td dump: setup {:#x}/{:#x}/{:#x}/{:#x} data {:#x}/{:#x}/{:#x}/{:#x} status {:#x}/{:#x}/{:#x}/{:#x}; ed {:#x} h {:#x} t {:#x}",
                self.tds[td_setup].info,
                self.tds[td_setup].cbp,
                self.tds[td_setup].next,
                self.tds[td_setup].be,
                self.tds[td_data].info,
                self.tds[td_data].cbp,
                self.tds[td_data].next,
                self.tds[td_data].be,
                self.tds[td_status].info,
                self.tds[td_status].cbp,
                self.tds[td_status].next,
                self.tds[td_status].be,
                ed_snapshot.info,
                ed_snapshot.head,
                ed_snapshot.tail,
            );
        }
        self.stop_control();

        // Clear a possible halt so the ED can be reused.
        let ed_head = self.ed(ed_index).head;
        self.ed_mut(ed_index).head = ed_head & !0x3;

        // The list is stopped and the transfer is complete: the buffer arena
        // can be reused from the start (single-threaded driver, one transfer
        // at a time). Without this the never-reused arena runs out after a
        // handful of control transfers (OutOfMemory during enumeration).
        self.next_buf = 0;

        result?;

        if data_in {
            if let (Some(off), Some(dst)) = (data_off, data) {
                // The controller may have written the data through DMA; drop
                // our possibly stale cache lines before reading it back.
                cache_invalidate_range(self.tds.virt_addr() + 1024 + off, data_len.max(1));
                let len = dst.len().min(data_len);
                let copy = self.buf_slice(off, len).to_vec();
                dst[..len].copy_from_slice(&copy);
            }
        }
        Ok(data_len)
    }

    /// Poll a TD until the controller retires it, then check its condition
    /// code. `timeout_ms` bounds the wait.
    fn wait_td(&self, ed_index: usize, td_index: usize, timeout_ms: u64) -> Result<u32> {
        let deadline = now_ms() + timeout_ms;
        loop {
            let info = self.tds[td_index].info;
            let cc = (info >> 28) & 0xf;
            if cc != TD_CC_NOT_ACCESSED {
                if cc == TD_CC_NOERROR || cc == TD_CC_DATAUNDERRUN {
                    return Ok(cc);
                }
                if self.ed(ed_index).head & 0x1 != 0 {
                    return Err(Error::Halted);
                }
                return Err(Error::Td(cc));
            }
            // A STALL or an error halts the ED instead of retiring the last
            // TD, so watch the halt bit as well.
            if self.ed(ed_index).head & 0x1 != 0 {
                return Err(Error::Halted);
            }
            if now_ms() >= deadline {
                return Err(Error::Timeout("transfer descriptor"));
            }
            core::hint::spin_loop();
        }
    }

    /// Queue one interrupt IN transfer on a periodic ED.
    #[allow(clippy::too_many_arguments)]
    pub fn arm_interrupt(
        &mut self,
        ed_index: usize,
        td_index: usize,
        dummy_index: usize,
        addr: u8,
        endpoint: u8,
        low_speed: bool,
        mps: u16,
        buf_phys: usize,
        slot: usize,
    ) -> Result<()> {
        self.fill_td(
            td_index,
            TD_CC | TD_R | TD_T_TOGGLE | TD_DP_IN,
            buf_phys,
            usize::from(mps),
            self.td_phys(dummy_index),
        );
        self.fill_td(dummy_index, 0, 0, 0, 0);

        let head = self.td_phys(td_index) as u32;
        let tail = self.td_phys(dummy_index) as u32;
        {
            let ed = self.ed_mut(ed_index);
            ed.info = ed_info(addr, endpoint, low_speed, ED_IN, mps);
            ed.next = 0;
            // Preserve the toggle carry (bit 1) across re-arms.
            let carry = ed.head & 0x2;
            ed.head = head | carry;
            ed.tail = tail;
        }

        // Put the ED into the periodic list. A 32-entry table is walked one
        // entry per frame, so one slot polls every ~32 ms — plenty for a
        // keyboard and far simpler than the interval tree Linux builds.
        let entry = self.hcca.virt_addr() as *mut u32;
        unsafe { ptr::write_volatile(entry.add(slot), self.ed_phys(ed_index) as u32) };
        // The HC reads the periodic table and the ED/TDs through DMA; clean
        // the cache lines it will touch.
        cache_clean_range(self.hcca.virt() as usize, 256);
        cache_clean_range(
            self.eds.virt_addr() + ed_index * size_of::<Ed>(),
            size_of::<Ed>(),
        );
        cache_clean_range(self.tds.virt_addr(), 4096);

        let control = self.read(REG_CONTROL);
        if control & CTRL_PLE == 0 {
            self.write(REG_CONTROL, control | CTRL_PLE);
        }
        Ok(())
    }

    /// Returns true when the periodic TD has been retired by the controller.
    pub fn interrupt_done(&self, td_index: usize) -> bool {
        ((self.tds[td_index].info >> 28) & 0xf) != TD_CC_NOT_ACCESSED
    }

    /// Condition code of a retired interrupt TD.
    pub fn interrupt_cc(&self, td_index: usize) -> u32 {
        (self.tds[td_index].info >> 28) & 0xf
    }

    /// Bytes actually transferred by a completed TD whose buffer starts at
    /// `buf_phys` and is `len` bytes long.
    pub fn td_transferred(&self, td_index: usize, buf_phys: usize, len: usize) -> usize {
        let td = &self.tds[td_index];
        if td.cbp == 0 {
            // Buffer end pointer reached: the whole buffer was filled.
            len
        } else {
            (td.cbp as usize).saturating_sub(buf_phys).min(len)
        }
    }

    /// Clear a halted interrupt ED (for example after a STALL).
    pub fn clear_interrupt_halt(&mut self, ed_index: usize) {
        let head = self.ed(ed_index).head;
        self.ed_mut(ed_index).head = head & !0x3;
    }

    /// Remove an ED from the periodic list.

    /// Read and acknowledge the done queue head so it does not overflow.
    /// Completion itself is detected through the TD condition codes.
    pub fn reap_done(&self) -> u32 {
        let status = self.read(REG_INTRSTATUS);
        if status & INTR_WDH != 0 {
            let head = self.read(REG_DONEHEAD);
            self.write(REG_INTRSTATUS, INTR_WDH);
            head
        } else {
            0
        }
    }
}

pub fn now_ms() -> u64 {
    let mut ts = syscall::TimeSpec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if syscall::clock_gettime(syscall::flag::CLOCK_MONOTONIC, &mut ts).is_err() {
        return 0;
    }
    ts.tv_sec as u64 * 1000 + ts.tv_nsec as u64 / 1_000_000
}
