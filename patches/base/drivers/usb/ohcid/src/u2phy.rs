//! RK3399 USB2 PHY (Innosilicon IP) power-on for the OHCI companions.
//!
//! U-Boot powers the PHY down when it stops its USB stack, and the OHCI needs
//! both the PHY's 480 MHz UTMI clock and a port that is out of suspend.
//!
//! Everything here is the `rockchip,rk3399-usb2phy` configuration from Linux
//! `drivers/phy/rockchip/phy-rockchip-inno-usb2.c` (`rk3399_phy_cfgs`): the
//! `u2phyN` blocks live in the GRF at 0xe450/0xe460, `clkout_ctl` is bit 4 of
//! the block base (0 turns the 480 MHz output on) and the host port suspend
//! field is bits [1:0] at base + 8, where 2 is "running" and 1 is "suspended".
//! The registers are HIWORD write-masked, like the CRU gates.

use crate::log::Logger;
use crate::ohci::now_ms;

/// GRF page that holds both `u2phy` blocks (`syscon@ff770000` + 0xe000).
const GRF_PHY_BASE: usize = 0xff77e000;
const GRF_PHY_LEN: usize = 0x1000;

/// `u2phy0` block: 480 MHz clock output and the host port suspend field.
const U2PHY0_CLKOUT: usize = 0x450;
const U2PHY0_HOST_SUS: usize = 0x458;
/// `u2phy1` block (`usb_host1`).
const U2PHY1_CLKOUT: usize = 0x460;
const U2PHY1_HOST_SUS: usize = 0x468;

/// Bit 4 of the block base gates the 480 MHz output clock.
const CLKOUT_ENABLE_MASK: u32 = 1 << 4;
/// Bits [1:0] of the port register: 2 = running, 1 = suspended.
const PORT_SUS_MASK: u32 = 0x3;
const PORT_RUNNING: u32 = 0x2;

/// Time the PHY needs to settle after leaving suspend (Linux uses 1.5-2 ms).
const SETTLE_MS: u64 = 3;

#[inline]
unsafe fn read32(base: usize, offset: usize) -> u32 {
    unsafe { core::ptr::read_volatile((base + offset) as *const u32) }
}

#[inline]
unsafe fn write32(base: usize, offset: usize, value: u32) {
    unsafe { core::ptr::write_volatile((base + offset) as *mut u32, value) }
}

/// Power both USB2 PHY host ports back on and report the register state.
///
/// Failure is not fatal: the caller still probes the controllers, it just
/// will not see any device behind a powered-down PHY.
pub fn power_on_host_ports(log: &Logger) {
    let base = match unsafe {
        common::physmap(
            GRF_PHY_BASE,
            GRF_PHY_LEN,
            common::Prot::RW,
            common::MemoryType::DeviceMemory,
        )
    } {
        Ok(ptr) => ptr as usize,
        Err(_) => {
            log.log(format_args!(
                "ohcid: cannot map the usb2 phy registers, leaving them alone"
            ));
            return;
        }
    };

    let clkout0 = unsafe { read32(base, U2PHY0_CLKOUT) };
    let sus0 = unsafe { read32(base, U2PHY0_HOST_SUS) };
    let clkout1 = unsafe { read32(base, U2PHY1_CLKOUT) };
    let sus1 = unsafe { read32(base, U2PHY1_HOST_SUS) };
    log.log(format_args!(
        "ohcid: u2phy inherited: clkout0=0x{:08x} host0=0x{:08x} clkout1=0x{:08x} host1=0x{:08x}",
        clkout0, sus0, clkout1, sus1
    ));

    // 480 MHz output clock on (clear bit 4 through the hiword mask).
    unsafe {
        write32(base, U2PHY0_CLKOUT, CLKOUT_ENABLE_MASK << 16);
        write32(base, U2PHY1_CLKOUT, CLKOUT_ENABLE_MASK << 16);
        // Leave suspend on both host ports.
        write32(base, U2PHY0_HOST_SUS, (PORT_SUS_MASK << 16) | PORT_RUNNING);
        write32(base, U2PHY1_HOST_SUS, (PORT_SUS_MASK << 16) | PORT_RUNNING);
    }

    let target = now_ms() + SETTLE_MS;
    while now_ms() < target {
        core::hint::spin_loop();
    }

    let clkout0 = unsafe { read32(base, U2PHY0_CLKOUT) };
    let sus0 = unsafe { read32(base, U2PHY0_HOST_SUS) };
    let clkout1 = unsafe { read32(base, U2PHY1_CLKOUT) };
    let sus1 = unsafe { read32(base, U2PHY1_HOST_SUS) };
    log.log(format_args!(
        "ohcid: u2phy now: clkout0=0x{:08x} host0=0x{:08x} clkout1=0x{:08x} host1=0x{:08x}",
        clkout0, sus0, clkout1, sus1
    ));
}
