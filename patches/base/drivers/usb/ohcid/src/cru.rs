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

/// Log the CPU clock registers once (diagnostics for the desktop speed:
/// Redox has no cpufreq driver, so the clusters run at whatever rate U-Boot
/// left; experiment 14 needs the actual values). Raw dumps: PLL CON0..CON3
/// groups (each PLL occupies 0x10 bytes starting at APLL 0x00) and the two
/// armclk selects CLKSEL_CON0/CON1 at 0x100/0x104.
pub fn log_cpu_clocks(log: &Logger) {
    let base = match unsafe {
        common::physmap(
            CRU_BASE,
            CRU_LEN,
            common::Prot::RW,
            common::MemoryType::DeviceMemory,
        )
    } {
        Ok(ptr) => ptr as usize,
        Err(_) => return,
    };
    let mut line = String::new();
    for off in (0x00..0xa0).step_by(0x10) {
        line.clear();
        for i in 0..4 {
            let v = unsafe { read32(base, off + i * 4) };
            line.push_str(&format!("{:08x} ", v));
        }
        log.log(format_args!("ohcid: cru pll {:#04x}: {}", off, line));
    }
    log.log(format_args!(
        "ohcid: cru clksel_con0 {:#010x} clksel_con1 {:#010x}",
        unsafe { read32(base, 0x100) },
        unsafe { read32(base, 0x104) }
    ));
}

/// Un-gate the USB2 host clocks and report the gate state before and after.
///
/// Returns `true` when the gates read back as enabled, which is the
/// precondition for touching an OHCI register window. A `false` return means
/// the caller must not probe the controllers (the machine would stall on the
/// first access to an unclocked peripheral).
/// Raise the little cluster from the divided-down rate U-Boot left it at.
///
/// Measured on the Pinebook Pro (experiment 14): `CLKSEL_CON1 = 0x501`, i.e.
/// the armclk divider field `[4:0] = 1` — the little cluster runs at half its
/// PLL rate (600 MHz / 2 = **300 MHz**), while the big cluster is undivided
/// (`CLKSEL_CON0 = 0x100`). Redox has no cpufreq driver, so the rate never
/// ramps; the single-threaded desktop (orbital, vesad) ends up on a 300 MHz
/// core and feels slow.
///
/// The divider is glitch-free hardware (it is the runtime-DVFS divider), so
/// writing /1 needs no PLL relock and no voltage change: the same PLLs already
/// run the big cluster at their full rate, which puts the CPU rails at or
/// above what 600 MHz A53 needs. The write is hiword-masked, like every other
/// CRU field on this SoC.
pub fn raise_cpu_clocks(log: &Logger) {
    const CLKSEL_CON1: usize = 0x104;
    let base = match unsafe {
        common::physmap(
            CRU_BASE,
            CRU_LEN,
            common::Prot::RW,
            common::MemoryType::DeviceMemory,
        )
    } {
        Ok(ptr) => ptr as usize,
        Err(_) => return,
    };
    let con1 = unsafe { read32(base, CLKSEL_CON1) };
    let divider = con1 & 0x1f;
    if divider == 0 {
        log.log(format_args!(
            "ohcid: cru armclk little already undivided (clksel_con1 {:#010x})",
            con1
        ));
        return;
    }
    // Bits [4:0] are the armclk divider (value + 1); the hiword carries the
    // write mask so the unrelated dividers in the same register survive.
    unsafe { write32(base, CLKSEL_CON1, 0x1f << 16) };
    let after = unsafe { read32(base, CLKSEL_CON1) };
    log.log(format_args!(
        "ohcid: cru armclk little /{} -> /{} (clksel_con1 {:#010x} -> {:#010x})",
        divider + 1,
        (after & 0x1f) + 1,
        con1,
        after
    ));
}

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
