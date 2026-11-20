//! RK3399 CRU (clock & reset unit) handling for the USB2 host controllers.
//!
//! U-Boot shuts its USB stack down before it hands control to the EFI payload,
//! and that leaves the USB clock gates disabled: `dwmmcd` reads
//! `CLKGATE_CON(6)` as `0x00000060` on the Pinebook Pro, i.e. both USB2 PHY
//! reference gates are set (in the RK3399 the gate bit means "disabled").
//! Touching an OHCI register window whose bus clock is gated is not safe, so
//! the driver re-enables the gates before the first register access.
//!
//! Register numbering follows Linux `drivers/clk/rockchip/clk-rk3399.c`:
//! `CLKGATE_CON(n) = 0x300 + 4n`; gates are HIWORD protected (mask in
//! [31:16], data in [15:0]) and flagged `CLK_GATE_SET_TO_DISABLE`, so writing
//! a zero through the mask enables the clock.

use crate::log::Logger;
use crate::ohci::now_ms;

/// CRU register window (`clock-controller@ff760000`).
const CRU_BASE: usize = 0xff760000;
const CRU_LEN: usize = 0x1000;

/// `CLKGATE_CON(6)` = 0x318: bit 5 `clk_usb2phy0_ref`, bit 6 `clk_usb2phy1_ref`.
const CLKGATE_CON6: usize = 0x318;
/// `CLKGATE_CON(20)` = 0x350: bits 5/6 `hclk_host0`/`hclk_host0_arb`,
/// bits 7/8 `hclk_host1`/`hclk_host1_arb`.
const CLKGATE_CON20: usize = 0x350;

/// Both USB2 PHY reference gates.
const PHY_REF_GATES: u32 = (1 << 5) | (1 << 6);
/// Both host bus-clock gates (host0 and host1, including the arbiters).
const HOST_GATES: u32 = (1 << 5) | (1 << 6) | (1 << 7) | (1 << 8);

/// Time the bus clocks (and the PHY reference) need to come back up.
const SETTLE_MS: u64 = 5;

#[inline]
unsafe fn read32(base: usize, offset: usize) -> u32 {
    unsafe { core::ptr::read_volatile((base + offset) as *const u32) }
}

#[inline]
unsafe fn write32(base: usize, offset: usize, value: u32) {
    unsafe { core::ptr::write_volatile((base + offset) as *mut u32, value) }
}

/// Un-gate the USB2 host clocks and report the gate state before and after.
///
/// Returns `true` when the gates read back as enabled, which is the
/// precondition for touching an OHCI register window. A `false` return means
/// the caller must not probe the controllers (the machine would stall on the
/// first access to an unclocked peripheral).
pub fn enable_usb_host_clocks(log: &Logger) -> bool {
    let base = match unsafe {
        common::physmap(
            CRU_BASE,
            CRU_LEN,
            common::Prot::RW,
            common::MemoryType::DeviceMemory,
        )
    } {
        Ok(ptr) => ptr as usize,
        Err(_) => {
            log.log(format_args!(
                "ohcid: cannot map the CRU, leaving the usb clocks alone"
            ));
            return false;
        }
    };

    let con6 = unsafe { read32(base, CLKGATE_CON6) };
    let con20 = unsafe { read32(base, CLKGATE_CON20) };
    log.log(format_args!(
        "ohcid: cru gates inherited: con6=0x{:08x} con20=0x{:08x}",
        con6, con20
    ));

    if con6 & PHY_REF_GATES != 0 {
        unsafe { write32(base, CLKGATE_CON6, PHY_REF_GATES << 16) };
    }
    if con20 & HOST_GATES != 0 {
        unsafe { write32(base, CLKGATE_CON20, HOST_GATES << 16) };
    }

    let target = now_ms() + SETTLE_MS;
    while now_ms() < target {
        core::hint::spin_loop();
    }

    let con6 = unsafe { read32(base, CLKGATE_CON6) };
    let con20 = unsafe { read32(base, CLKGATE_CON20) };
    log.log(format_args!(
        "ohcid: cru gates now: con6=0x{:08x} con20=0x{:08x}",
        con6, con20
    ));

    let stuck = (con6 & PHY_REF_GATES) | (con20 & HOST_GATES);
    if stuck != 0 {
        log.log(format_args!(
            "ohcid: usb clocks still gated (0x{:08x}), not touching the controllers",
            stuck
        ));
        return false;
    }
    true
}
