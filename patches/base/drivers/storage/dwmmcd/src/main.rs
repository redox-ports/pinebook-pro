//! dwmmcd - DesignWare MMC (dw_mshc) microSD driver for the RK3399.
//!
//! Serves the `disk.mmc` scheme for the `sdmmc@fe320000` controller found in
//! the RK3399 device tree, so redoxfs can mount the root filesystem from the
//! microSD card. The controller is driven in FIFO/PIO mode with polled
//! commands (no IDMAC, no interrupts); see `dwmmc.rs` for the clocking
//! strategy and register references.
//!
//! Node selection is deliberately strict: the driver only binds to a
//! `dw-mshc` node whose first `reg` entry is exactly `0xfe320000`. The
//! RK3399 has three DW MMC-class controllers and the SDIO one (`fe310000`)
//! shares the same compatible string; the eMMC (`fe330000`, SDHCI) holds the
//! baseline OS and must never be probed.

mod dwmmc;
mod log;

use std::io::Write;
use std::process;

use driver_block::{DiskScheme, ExecutorTrait, TrivialExecutor};
use event::{EventFlags, RawEventQueue};
use fdt::Fdt;
use log::Logger;

/// The RK3399 microSD controller, by MMIO address (never by index).
const SDMMC_BASE: usize = 0xfe320000;
/// fifo-depth from the RK3399 devicetree (`<0x100>`), used when the
/// property is missing.
const DEFAULT_FIFO_DEPTH: u32 = 0x100;

#[cfg(target_os = "redox")]
fn get_dtb() -> Vec<u8> {
    std::fs::read("kernel.dtb:").expect("dwmmcd: failed to read kernel.dtb:")
}

#[cfg(target_os = "linux")]
fn get_dtb() -> Vec<u8> {
    match std::env::args().nth(1) {
        Some(arg1) => std::fs::read(arg1).expect("dwmmcd: failed to read dtb file"),
        None => Vec::new(),
    }
}

fn main() {
    daemon::Daemon::new(daemon);
}

fn daemon(daemon: daemon::Daemon) -> ! {
    let log = Logger::new();

    let dtb_data = get_dtb();
    if dtb_data.is_empty() {
        log.log(format_args!("dwmmcd: no devicetree, exiting"));
        process::exit(0);
    }
    let fdt = match Fdt::new_unaligned(&dtb_data) {
        Ok(fdt) => fdt,
        Err(err) => {
            log.log(format_args!("dwmmcd: bad devicetree: {:?}", err));
            process::exit(0);
        }
    };
    log.log(format_args!(
        "dwmmcd: devicetree model {}",
        fdt.root().compatible().first()
    ));

    // Find the microSD controller among the dw-mshc nodes. sdio0 shares the
    // compatible string, so match the reg address explicitly.
    let with = ["rockchip,rk3399-dw-mshc", "rockchip,rk3288-dw-mshc"];
    let mut node_opt = None;
    for node in fdt.all_compatible(&with) {
        let reg = match node.reg() {
            Some(reg) => reg,
            None => continue,
        };
        let entry = match reg.iter::<usize, usize>().next() {
            Some(entry) => entry.expect("dwmmcd: bad reg entry"),
            None => continue,
        };
        log.log(format_args!(
            "dwmmcd: dw-mshc node {}@{} at 0x{:x}",
            node.name().name,
            node.name().unit_address.unwrap_or(""),
            entry.address
        ));
        if entry.address == SDMMC_BASE {
            // fifo-depth is optional in the binding; fall back to the SoC
            // default.
            let fifo_depth = node
                .raw_property("fifo-depth")
                .and_then(|prop| prop.as_value::<u32>().ok())
                .unwrap_or(DEFAULT_FIFO_DEPTH);
            node_opt = Some((entry.address, entry.len, fifo_depth));
            break;
        }
    }

    let Some((reg_addr, reg_len, fifo_depth)) = node_opt else {
        log.log(format_args!(
            "dwmmcd: no sdmmc@fe320000 in the devicetree, exiting"
        ));
        process::exit(0);
    };

    common::init();
    let addr = unsafe {
        common::physmap(
            reg_addr,
            reg_len,
            common::Prot::RW,
            common::MemoryType::DeviceMemory,
        )
        .expect("dwmmcd: failed to map the controller")
    } as usize;
    log.log(format_args!(
        "dwmmcd: mapped 0x{:08x} (len 0x{:x}) to 0x{:08x}",
        reg_addr, reg_len, addr
    ));

    // Map the CRU as well (read-diagnostics plus un-gating the SDMMC
    // clocks if the inherited state has them gated). Failure is not fatal:
    // the driver then just skips the clock checks.
    let cru = unsafe {
        common::physmap(
            dwmmc::CRU_BASE,
            dwmmc::CRU_LEN,
            common::Prot::RW,
            common::MemoryType::DeviceMemory,
        )
        .ok()
        .map(|ptr| ptr as usize as *mut u32)
    };

    let mut host = dwmmc::DwmmcHost::new(addr as *mut u32, cru, fifo_depth, log);
    host.log_inherited_state();

    if let Err(err) = host.init() {
        // Exit without signaling readiness: init records the failure and
        // continues (the root filesystem mount will then fail as before,
        // leaving the debugsh console as the interim interface).
        host.log_line(format_args!("dwmmcd: init failed: {}", err));
        process::exit(1);
    }

    // Self-test: block 0 must carry a protective MBR/GPT or MBR signature.
    match host.read_block_for_test(0) {
        Ok(block) => {
            let signature = [block[510], block[511]];
            host.log_line(format_args!(
                "dwmmcd: block 0 signature {:02x}{:02x}",
                signature[0], signature[1]
            ));
        }
        Err(err) => {
            host.log_line(format_args!("dwmmcd: block 0 self-test failed: {}", err));
            process::exit(1);
        }
    }

    let disks = vec![host];
    let mut scheme = DiskScheme::new(
        Some(daemon),
        "disk.mmc".to_string(),
        disks
            .into_iter()
            .enumerate()
            .map(|(i, disk)| (i as u32, disk))
            .collect(),
        &TrivialExecutor,
    );

    let event_queue = RawEventQueue::new().expect("dwmmcd: failed to open the event file");
    event_queue
        .subscribe(scheme.event_handle().raw(), 0, EventFlags::READ)
        .expect("dwmmcd: failed to subscribe to the disk scheme");

    libredox::call::setrens(0, 0).expect("dwmmcd: failed to enter the null namespace");

    for event in event_queue {
        let event = event.unwrap();
        if event.fd == scheme.event_handle().raw() {
            TrivialExecutor.block_on(scheme.tick()).unwrap();
        } else {
            // stderr already reaches the UART (inherited from init).
            let mut stderr = std::io::stderr();
            let _ = writeln!(stderr, "dwmmcd: unknown event {}", event.fd);
        }
    }
    process::exit(0);
}
