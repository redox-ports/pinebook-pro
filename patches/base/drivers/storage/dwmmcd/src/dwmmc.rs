//! Synopsys DesignWare Mobile Storage Host Controller (dw_mshc) driver for
//! the Rockchip RK3399 microSD slot (`sdmmc@fe320000`).
//!
//! Register map and bit definitions follow Linux `drivers/mmc/host/dw_mmc.h`
//! and U-Boot v2021.07 `include/dwmmc.h` (the U-Boot version installed on
//! the Pinebook Pro). Data transfers use FIFO/PIO mode only: no IDMAC and no
//! controller interrupts, everything is polled. This mirrors the U-Boot
//! `fifo_mode` code path.
//!
//! Clocking strategy (bring-up): the U-Boot rk3399 model. The controller
//! `CLKDIV` stays at 0 (bypass — the block divides cclk by 2 internally)
//! and the card rate comes entirely from the CRU `CLKSEL_CON16`
//! (identification at xin24m/30, data at the U-Boot-inherited rate).
//! U-Boot's `dwmci_setup_bus` wraps the CLKENA register in two CIU
//! clock-update commands and waits for each by polling the CMD START bit;
//! this driver does the same, and falls back to plain register writes
//! with no CIU involvement if the updates never complete. If the card
//! still fails to enumerate the driver exits and init continues without
//! a `disk.mmc` scheme.

use core::fmt;

use std::sync::RwLock;

use driver_block::Disk;
use syscall::{Error, Result, EIO, EINVAL, ETIMEDOUT};

use crate::log::Logger;

// ---------------------------------------------------------------------------
// Register offsets (SDMMC_* from Linux dw_mmc.h; identical in U-Boot dwmmc.h)
// ---------------------------------------------------------------------------

const CTRL: usize = 0x000;
const PWREN: usize = 0x004;
const CLKDIV: usize = 0x008;
const CLKSRC: usize = 0x00c;
const CLKENA: usize = 0x010;
const TMOUT: usize = 0x014;
const CTYPE: usize = 0x018;
const BLKSIZ: usize = 0x01c;
const BYTCNT: usize = 0x020;
const INTMASK: usize = 0x024;
const CMDARG: usize = 0x028;
const CMD: usize = 0x02c;
const RESP0: usize = 0x030;
const RESP1: usize = 0x034;
const RESP2: usize = 0x038;
const RESP3: usize = 0x03c;
const RINTSTS: usize = 0x044;
const STATUS: usize = 0x048;
const FIFOTH: usize = 0x04c;
const CDETECT: usize = 0x050;
const VERID: usize = 0x06c;
const HCON: usize = 0x070;
const BMOD: usize = 0x080;

// ---------------------------------------------------------------------------
// RK3399 CRU (clock and reset unit), ff760000. Offsets follow Linux
// dt-bindings/clock/rk3399-cru.h: CLKSEL_CON(n) = 0x100 + 4n,
// CLKGATE_CON(n) = 0x300 + 4n. Gates are HIWORD (mask in [31:16], data in
// [15:0]) and set-to-disable: a data bit of 0 enables the clock.
// ---------------------------------------------------------------------------

pub const CRU_BASE: usize = 0xff760000;
pub const CRU_LEN: usize = 0x1000;

/// HCLK_SD (sdmmc bus-clock composite): CLKSEL_CON(13).
const CRU_CLKSEL_CON13_HCLK_SD: usize = 0x134;
/// SCLK_SDMMC (CIU clock composite): CLKSEL_CON(16).
const CRU_CLKSEL_CON16_SCLK_SDMMC: usize = 0x140;
/// SCLK_SDMMC gate (Linux: RK3399_CLKGATE_CON(6), bit 1).
const CRU_CLKGATE_CON6: usize = 0x318;
/// HCLK_SD gate (Linux: RK3399_CLKGATE_CON(12), bit 13).
const CRU_CLKGATE_CON12: usize = 0x330;
/// HCLK_SDMMC gate (Linux: RK3399_CLKGATE_CON(33), bit 8).
const CRU_CLKGATE_CON33: usize = 0x384;

const SDMMC_CIU_GATE_CON: usize = CRU_CLKGATE_CON6;
const SDMMC_CIU_GATE_BIT: u32 = 1;
const HCLK_SD_GATE_CON: usize = CRU_CLKGATE_CON12;
const HCLK_SD_GATE_BIT: u32 = 13;
const HCLK_SDMMC_GATE_CON: usize = CRU_CLKGATE_CON33;
const HCLK_SDMMC_GATE_BIT: u32 = 8;

/// CRU SOFTRST_CON(n) = 0x400 + 4n. Reset lines are hiword: mask in
/// [31:16], data in [15:0]; data bit 1 asserts the reset.
const CRU_SOFTRST_CON7: usize = 0x41c;
const CRU_SOFTRST_CON11: usize = 0x42c;

/// SRST_SDMMC = 122 (Linux dt-bindings reset/rk3399: cru_softrst_con7,
/// bit 10) — the controller's reset line, the one Linux pulses in
/// dw_mci_probe via resets = <&cru SRST_SDMMC>.
const SRST_SDMMC_CON: usize = CRU_SOFTRST_CON7;
const SRST_SDMMC_BIT: u32 = 10;
/// SRST_SD_NOC = 189 (cru_softrst_con11, bit 13) — the SD bus noc bridge.
const SRST_SD_NOC_CON: usize = CRU_SOFTRST_CON11;
const SRST_SD_NOC_BIT: u32 = 13;

/// FIFO data register. 0x100 for IP versions below 2.40a, 0x200 from 2.40a
/// on. U-Boot hardcodes 0x200 for every board it supports (including all
/// Rockchip SoCs); this driver reads VERID and picks like Linux does.
const DATA_OFFSET_PRE_240A: usize = 0x100;
const DATA_OFFSET_240A: usize = 0x200;

// CTRL bits
const CTRL_RESET: u32 = 1 << 0;
const CTRL_FIFO_RESET: u32 = 1 << 1;
const CTRL_DMA_RESET: u32 = 1 << 2;

// CLKENA bits
const CLKENA_ENABLE: u32 = 1 << 0;
const CLKENA_LOW_PWR: u32 = 1 << 16;

// CTYPE values
const CTYPE_4BIT: u32 = 1 << 0;

// Raw interrupt status bits (RINTSTS, write-1-to-clear; same map as INTMASK)
const INT_CDONE: u32 = 1 << 2; // command done
const INT_DTO: u32 = 1 << 3; // data transfer over
const INT_RE: u32 = 1 << 1; // response error
const INT_RCRC: u32 = 1 << 6; // response crc error
const INT_RTO: u32 = 1 << 8; // response timeout
const INT_HLE: u32 = 1 << 12; // hardware locked write error
const INT_EBE: u32 = 1 << 15; // end bit error (cmd/dat)
const INT_SBE: u32 = 1 << 13; // start bit error
const INT_FRUN: u32 = 1 << 11; // fifo underrun/overrun
const INT_DCRC: u32 = 1 << 7; // data crc error
const INT_DRTO: u32 = 1 << 9; // data read timeout
const INT_HTO: u32 = 1 << 10; // data starvation-by-host timeout

/// Errors that abort a data transfer (U-Boot DWMCI_DATA_ERR | DWMCI_DATA_TOUT).
const DATA_ERROR_MASK: u32 =
    INT_EBE | INT_SBE | INT_HLE | INT_FRUN | INT_DCRC | INT_DRTO | INT_HTO;

// STATUS bits
const STATUS_FIFO_EMPTY: u32 = 1 << 2;
const STATUS_FIFO_FULL: u32 = 1 << 3;
const STATUS_BUSY: u32 = 1 << 9;

// CMD register bits
const CMD_START: u32 = 1 << 31;
const CMD_USE_HOLD_REG: u32 = 1 << 29;
const CMD_UPD_CLK: u32 = 1 << 21;
const CMD_INIT: u32 = 1 << 15; // 80 init clocks before the command
const CMD_ABORT_STOP: u32 = 1 << 14; // stop an ongoing data transfer
const CMD_PRV_DAT_WAIT: u32 = 1 << 13;
const CMD_DAT_WR: u32 = 1 << 10;
const CMD_DAT_EXP: u32 = 1 << 9;
const CMD_RESP_CRC: u32 = 1 << 8;
const CMD_RESP_LONG: u32 = 1 << 7;
const CMD_RESP_EXP: u32 = 1 << 6;
const CMD_INDEX_MASK: u32 = 0x3f;

// SD commands (index only; response shaping happens per command)
const CMD0_GO_IDLE: u32 = 0;
const CMD2_ALL_SEND_CID: u32 = 2;
const CMD3_SEND_REL_ADDR: u32 = 3;
const CMD7_SELECT_CARD: u32 = 7;
const CMD8_SEND_IF_COND: u32 = 8;
const CMD9_SEND_CSD: u32 = 9;
const CMD12_STOP_TRANSMISSION: u32 = 12;
const CMD16_SET_BLOCKLEN: u32 = 16;
const CMD17_READ_SINGLE: u32 = 17;
const CMD18_READ_MULTI: u32 = 18;
const CMD24_WRITE_SINGLE: u32 = 24;
const CMD25_WRITE_MULTI: u32 = 25;
const CMD55_APP_CMD: u32 = 55;
const ACMD6_SET_BUS_WIDTH: u32 = 6;
const ACMD41_SD_SEND_OP_COND: u32 = 41;

/// HCS (host capacity support) + 2.7-3.6 V window, no S18R (3.3 V slot).
const ACMD41_ARG: u32 = 0x40ff_8000;
const ACMD41_BUSY: u32 = 1 << 31;
const ACMD41_CCS: u32 = 1 << 30;
const CMD8_ARG: u32 = 0x0000_01aa; // check pattern + 2.7-3.6 V support
const CMD8_ECHO: u32 = 0x0000_01aa;

// Timeouts, in milliseconds (wall clock via the kernel monotonic clock).
const CMD_TIMEOUT_MS: u64 = 1000;
const CLK_UPDATE_TIMEOUT_MS: u64 = 100;
const FIFO_RESET_TIMEOUT_MS: u64 = 100;
const CTRL_RESET_TIMEOUT_MS: u64 = 500;
const BUSY_TIMEOUT_MS: u64 = 500;
const DATA_TIMEOUT_PER_BLOCK_MS: u64 = 100;
const DATA_TIMEOUT_BASE_MS: u64 = 500;
/// Total budget for ACMD41 busy polling (cards may take over a second).
const ACMD41_TIMEOUT_MS: u64 = 2000;
/// Milliseconds of sleep between ACMD41 attempts.
const ACMD41_RETRY_DELAY_MS: u64 = 10;

/// Upper bound for a single PIO data command (blocks per CMD18/CMD25).
const MAX_BLOCKS_PER_COMMAND: u64 = 8;

/// SCLK_SDMMC rate targets, written straight into CLKSEL_CON16 (a plain
/// read-modify-write register, unlike the hiword gate/reset registers).
/// Layout: mux bits [10:8] (0=cpll, 1=gpll, 2=npll, 3=ppll, 4=upll,
/// 5=xin24m), divider bits [6:0] with div = reg + 1. With controller
/// CLKDIV = 0 the block divides cclk by 2 internally (U-Boot rk3399:
/// "dwmmc controller have internal div 2, provide double in cru"), so the
/// card sees half the CRU rate.
/// Identification: xin24m / 30 / 2 = 400 kHz (U-Boot writes 0x51d here).
const CLKSEL_400K: u32 = 0x51d;
/// Field mask of CLKSEL_CON16: mux bits [10:8] and divider bits [6:0].
/// Linux declares SCLK_SDMMC with CLK_MUX_HIWORD_MASK |
/// CLK_DIVIDER_HIWORD_MASK (clk-rk3399.c), i.e. this register follows the
/// Rockchip write-mask convention: only bits whose mask bit in [31:16]
/// is set are written. A plain read-modify-write is silently dropped —
/// measured on hardware (readback stayed 0x10e after writing 0x51d).
const CLKSEL16_MASK: u32 = 0x77f;
/// Data-rate fallback: gpll (594 MHz) / 12 / 2 = 24.75 MHz, SD default
/// speed. Normally the inherited CLKSEL_CON16 (the rate U-Boot read the
/// ESP at) is preferred; this constant only covers an implausible
/// inherited value.
const CLKSEL_DATA: u32 = 0x10b;

pub struct DwmmcRegs {
    base: *mut u32,
    data_offset: usize,
}

impl DwmmcRegs {
    pub unsafe fn new(base: *mut u32) -> Self {
        let verid = unsafe { Self::read_at(base, VERID) } & 0xffff;
        DwmmcRegs {
            base,
            data_offset: if verid >= 0x240a {
                DATA_OFFSET_240A
            } else {
                DATA_OFFSET_PRE_240A
            },
        }
    }

    unsafe fn read_at(base: *mut u32, reg: usize) -> u32 {
        unsafe { core::ptr::read_volatile(base.add(reg / 4)) }
    }

    #[inline(always)]
    pub unsafe fn read(&self, reg: usize) -> u32 {
        unsafe { core::ptr::read_volatile(self.base.add(reg / 4)) }
    }

    #[inline(always)]
    pub unsafe fn write(&self, reg: usize, value: u32) {
        unsafe { core::ptr::write_volatile(self.base.add(reg / 4), value) }
    }

    #[inline(always)]
    pub unsafe fn read_data(&self) -> u32 {
        unsafe { core::ptr::read_volatile(self.base.add(self.data_offset / 4)) }
    }

    #[inline(always)]
    pub unsafe fn write_data(&self, value: u32) {
        unsafe { core::ptr::write_volatile(self.base.add(self.data_offset / 4), value) }
    }
}

/// Monotonic milliseconds from the kernel clock. Used for every timeout so
/// a dead card or controller can never hang the boot.
pub fn now_ms() -> u64 {
    let mut ts = syscall::TimeSpec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // A failure here would mean the kernel time scheme is broken; falling
    // back to zero would disable all timeouts, so panic instead.
    syscall::clock_gettime(syscall::flag::CLOCK_MONOTONIC, &mut ts)
        .expect("dwmmcd: clock_gettime(CLOCK_MONOTONIC) failed");
    ts.tv_sec as u64 * 1000 + ts.tv_nsec as u64 / 1_000_000
}

fn sleep_ms(ms: u64) {
    // Busy-wait instead of nanosleep: the bring-up kernel does not wake a
    // sleeping thread on the generic timer (experiment 6 iteration 7: a
    // 10 ms sleep blocked for seconds/minutes until a serial interrupt
    // arrived). All delays in this driver are short and the driver runs
    // on a single CPU, so spinning is the robust choice here.
    let deadline = now_ms() + ms;
    while now_ms() < deadline {
        core::hint::spin_loop();
    }
}

/// One executed command: response words plus the raw interrupt status seen
/// on completion, so callers can distinguish timeouts from CRC errors.
pub struct CmdOutcome {
    pub response: [u32; 4],
}

#[derive(Debug, Clone, Copy)]
pub enum CmdError {
    Timeout,
    ResponseError,
    ResponseCrc,
    HardwareLocked,
}

impl fmt::Display for CmdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CmdError::Timeout => write!(f, "timeout"),
            CmdError::ResponseError => write!(f, "response error"),
            CmdError::ResponseCrc => write!(f, "response crc"),
            CmdError::HardwareLocked => write!(f, "hardware locked"),
        }
    }
}

/// Which response the card is expected to send for a command.
#[derive(Clone, Copy)]
pub enum Response {
    None,
    /// 48-bit, CRC checked (R1/R6/R7).
    Short,
    /// 48-bit, CRC not checked (R3, OCR).
    ShortNoCrc,
    /// 136-bit, CRC checked (R2, CID/CSD).
    Long,
}

pub struct DwmmcHost {
    regs: RwLock<DwmmcRegs>,
    /// CRU mapping (ff760000) when available; used read-only for
    /// diagnostics plus un-gating the SDMMC clocks if the inherited state
    /// has them gated.
    cru: RwLock<Option<DwmmcRegs>>,
    pub fifo_depth: u32,
    pub rca: u32,
    pub block_addressed: bool,
    pub size: u64,
    inherited_clkdiv: u32,
    inherited_ctrl: u32,
    /// CLKSEL_CON16 as left by U-Boot (the working data-transfer rate).
    inherited_clksel16: u32,
    /// True when card rates are driven by plain CRU writes because CIU
    /// clock-update commands never complete.
    cru_clocking: bool,
    log: Logger,
}

impl DwmmcHost {
    pub fn new(base: *mut u32, cru: Option<*mut u32>, fifo_depth: u32, log: Logger) -> Self {
        // SAFETY: the caller guarantees the pointers map the controller and
        // (when given) the CRU.
        let regs = unsafe { DwmmcRegs::new(base) };
        let cru = cru.map(|ptr| unsafe {
            let mut regs = DwmmcRegs::new(ptr);
            regs.data_offset = 0; // unused for the CRU
            regs
        });
        DwmmcHost {
            regs: RwLock::new(regs),
            cru: RwLock::new(cru),
            fifo_depth,
            rca: 0,
            block_addressed: true,
            size: 0,
            inherited_clkdiv: 0,
            inherited_ctrl: 0,
            inherited_clksel16: 0,
            cru_clocking: false,
            log,
        }
    }

    pub fn log_line(&self, args: fmt::Arguments) {
        self.log.log(args);
    }

    pub fn log_inherited_state(&self) {
        let regs = self.regs.read().unwrap();
        unsafe {
            self.log.log(format_args!(
                "dwmmcd: inherited verid=0x{:08x} hcon=0x{:08x} ctrl=0x{:08x}",
                regs.read(VERID),
                regs.read(HCON),
                regs.read(CTRL)
            ));
            self.log.log(format_args!(
                "dwmmcd: inherited clkdiv={} clksrc={} clkena=0x{:08x} ctype=0x{:08x} fifoth=0x{:08x} cdetect=0x{:08x}",
                regs.read(CLKDIV),
                regs.read(CLKSRC),
                regs.read(CLKENA),
                regs.read(CTYPE),
                regs.read(FIFOTH),
                regs.read(CDETECT)
            ));
            self.log.log(format_args!(
                "dwmmcd: fifo depth {} words, data reg at +0x{:03x}",
                self.fifo_depth,
                regs.data_offset
            ));
        }
    }

    /// Bring up the controller and enumerate the card.
    pub fn init(&mut self) -> Result<()> {
        self.save_inherited_clock();
        self.cru_diagnostics_and_ungate();
        self.controller_setup()?;
        self.card_init()?;
        self.log.log(format_args!(
            "dwmmcd: card ready: rca=0x{:04x} size={} MiB {}",
            self.rca,
            self.size / (1024 * 1024),
            if self.block_addressed {
                "(block addressed)"
            } else {
                "(byte addressed)"
            }
        ));
        Ok(())
    }

    fn save_inherited_clock(&mut self) {
        let regs = self.regs.read().unwrap();
        self.inherited_clkdiv = unsafe { regs.read(CLKDIV) };
        self.inherited_ctrl = unsafe { regs.read(CTRL) };
    }

    /// Read the CRU state feeding the controller and un-gate the SDMMC
    /// clocks if they are gated (Linux clk_prepare_enable equivalent; all
    /// three gates are hiword set-to-disable). Gated CIU clock matches the
    /// iteration 2/3 symptom exactly: registers respond (BIU clock) while
    /// every CIU command, including clock updates, hangs with a clean
    /// RINTSTS.
    fn cru_diagnostics_and_ungate(&mut self) {
        let guard = self.cru.read().unwrap();
        let cru = match guard.as_ref() {
            Some(cru) => cru,
            None => {
                self.log
                    .log(format_args!("dwmmcd: CRU not mapped, skipping clock checks"));
                return;
            }
        };
        unsafe {
            // Pulse the controller reset lines first (Linux dw_mci_probe
            // does exactly this through resets = <&cru SRST_SDMMC> before
            // any register access). The EFI loader leaves the block with
            // IDMAC armed and the CIU wedged mid-command; a CTRL register
            // reset only clears the BIU side and did not unwedge it
            // (iterations 2-4: clock updates never reached command-done).
            // Assert both the controller and its noc bridge, wait, release.
            self.log
                .log(format_args!("dwmmcd: asserting SRST_SDMMC + SRST_SD_NOC"));
            cru.write(
                SRST_SDMMC_CON,
                (1 << 16 + SRST_SDMMC_BIT) | (1 << SRST_SDMMC_BIT),
            );
            cru.write(
                SRST_SD_NOC_CON,
                (1 << 16 + SRST_SD_NOC_BIT) | (1 << SRST_SD_NOC_BIT),
            );
            sleep_ms(10);
            cru.write(SRST_SDMMC_CON, 1 << 16 + SRST_SDMMC_BIT);
            cru.write(SRST_SD_NOC_CON, 1 << 16 + SRST_SD_NOC_BIT);
            sleep_ms(10);
            self.log
                .log(format_args!("dwmmcd: pulsed SRST_SDMMC + SRST_SD_NOC"));

            // Prove the pulse reset the controller block: the CIU-side
            // clock registers should now read their reset defaults
            // (CLKENA 0) instead of the U-Boot-inherited state.
            {
                let regs = self.regs.read().unwrap();
                self.log.log(format_args!(
                    "dwmmcd: post-SRST ctrl=0x{:08x} clkena=0x{:08x} clkdiv={} (was {}) clksrc={} status=0x{:08x}",
                    regs.read(CTRL),
                    regs.read(CLKENA),
                    regs.read(CLKDIV),
                    self.inherited_clkdiv,
                    regs.read(CLKSRC),
                    regs.read(STATUS)
                ));
            }

            let sel13 = cru.read(CRU_CLKSEL_CON13_HCLK_SD);
            let sel16 = cru.read(CRU_CLKSEL_CON16_SCLK_SDMMC);
            self.inherited_clksel16 = sel16;
            let gate6 = cru.read(SDMMC_CIU_GATE_CON);
            let gate12 = cru.read(HCLK_SD_GATE_CON);
            let gate33 = cru.read(HCLK_SDMMC_GATE_CON);
            self.log.log(format_args!(
                "dwmmcd: cru hclk_sd sel=0x{:08x} sclk_sdmmc sel=0x{:08x}",
                sel13, sel16
            ));
            self.log.log(format_args!(
                "dwmmcd: cru gates: con6=0x{:08x} con12=0x{:08x} con33=0x{:08x}",
                gate6, gate12, gate33
            ));

            // SCLK_SDMMC source/div (CLKSEL_CON16): mux bits [10:8]
            // (0=cpll,1=gpll,2=npll,3=ppll,4=upll,5=xin24m), div bits [6:0].
            let mux = (sel16 >> 8) & 0x7;
            let div = sel16 & 0x7f;
            let source = match mux {
                0 => "cpll",
                1 => "gpll",
                2 => "npll",
                3 => "ppll",
                4 => "upll",
                5 => "xin24m",
                _ => "reserved",
            };
            self.log.log(format_args!(
                "dwmmcd: sclk_sdmmc source {} div {}",
                source, div
            ));

            // Un-gate each SDMMC-related clock if disabled (data bit 1).
            // HIWORD update: mask in [31:16], value in [15:0]; writing 0
            // with the mask set enables a set-to-disable gate.
            if gate6 & SDMMC_CIU_GATE_BIT != 0 {
                self.log.log(format_args!(
                    "dwmmcd: sclk_sdmmc gate closed, enabling"
                ));
                cru.write(SDMMC_CIU_GATE_CON, 1 << 16 + SDMMC_CIU_GATE_BIT);
            }
            if gate12 & HCLK_SD_GATE_BIT != 0 {
                self.log.log(format_args!(
                    "dwmmcd: hclk_sd gate closed, enabling"
                ));
                cru.write(HCLK_SD_GATE_CON, 1 << 16 + HCLK_SD_GATE_BIT);
            }
            if gate33 & HCLK_SDMMC_GATE_BIT != 0 {
                self.log.log(format_args!(
                    "dwmmcd: hclk_sdmmc gate closed, enabling"
                ));
                cru.write(HCLK_SDMMC_GATE_CON, 1 << 16 + HCLK_SDMMC_GATE_BIT);
            }
        }
    }

    /// Controller-level setup, mirroring U-Boot dwmci_init: a full
    /// controller reset first (CTRL/FIFO/DMA — this is required because
    /// U-Boot leaves the block in IDMAC/DMA state after loading the kernel
    /// through EFI; experiment 6 measured ctrl=0x02000000, the USE_IDMAC
    /// bit), then the polling-driver register state. The reset clears the
    /// CIU-side registers (CLKDIV/CLKENA/CTYPE back to defaults), which is
    /// fine: they are reprogrammed right after from known values. The CRU
    /// cclk rate feeding the controller is NOT touched by this reset.
    fn controller_setup(&mut self) -> Result<()> {
        let regs = self.regs.read().unwrap();
        unsafe {
            // Power enable (slot 0). Harmless if the board wires power
            // always-on.
            regs.write(PWREN, 1);

            // Full reset (U-Boot dwmci_wait_reset with DWMCI_RESET_ALL).
            // The bits self-clear; wait for them to go away.
            regs.write(CTRL, CTRL_RESET | CTRL_FIFO_RESET | CTRL_DMA_RESET);
            let deadline = now_ms() + CTRL_RESET_TIMEOUT_MS;
            loop {
                if regs.read(CTRL) & (CTRL_RESET | CTRL_FIFO_RESET | CTRL_DMA_RESET) == 0 {
                    break;
                }
                if now_ms() > deadline {
                    self.log.log(format_args!(
                        "dwmmcd: controller reset timeout, ctrl=0x{:08x}",
                        regs.read(CTRL)
                    ));
                    return Err(Error::new(ETIMEDOUT));
                }
            }
            self.log.log(format_args!(
                "dwmmcd: controller reset ok (ctrl was 0x{:08x})",
                self.inherited_ctrl
            ));

            // Polling driver: no controller interrupts may reach the GIC.
            regs.write(INTMASK, 0);
            // Clear any stale raw status.
            regs.write(RINTSTS, 0xffff_ffff);
            // Maximum response/data timeouts; the wall-clock guards below
            // are the real watchdogs.
            regs.write(TMOUT, 0xffff_ffff);

            // FIFO watermark: RX at half depth minus one, TX at half depth
            // (U-Boot dwmci_init_fifo values). Only the watermark matters in
            // PIO mode; RXDR/TXDR are never unmasked.
            let thr = self.fifo_depth / 2;
            let fifoth = (2u32 << 28) | ((thr - 1) << 16) | thr;
            regs.write(FIFOTH, fifoth);
        }
        Ok(())
    }

    /// Stop and restart cclk around a register change, exactly U-Boot
    /// dwmci_setup_bus: CLKENA 0, dividers to bypass, one clock-update
    /// command, CLKENA enable, a second clock-update command. The
    /// clock-update "command" only talks to the CIU, never to the card.
    fn clkena_cycle(&mut self, stage: &str) -> Result<()> {
        let regs = self.regs.read().unwrap();
        unsafe {
            regs.write(CLKENA, 0);
            regs.write(CLKSRC, 0); // slot 0
            regs.write(CLKDIV, 0); // bypass: the rate comes from the CRU
            if let Err(err) = clk_update(&regs) {
                self.log_update_failure(stage, &err);
                return Err(err);
            }
            regs.write(CLKENA, CLKENA_ENABLE | CLKENA_LOW_PWR);
            if let Err(err) = clk_update(&regs) {
                self.log_update_failure(stage, &err);
                return Err(err);
            }
        }
        Ok(())
    }

    fn log_update_failure(&self, stage: &str, err: &Error) {
        let regs = self.regs.read().unwrap();
        self.log.log(format_args!(
            "dwmmcd: clock update ({}) failed: {} cmd=0x{:08x} rint=0x{:08x} status=0x{:08x}",
            stage,
            err,
            unsafe { regs.read(CMD) },
            unsafe { regs.read(RINTSTS) },
            unsafe { regs.read(STATUS) }
        ));
    }

    /// Reprogram SCLK_SDMMC through CLKSEL_CON16 — the U-Boot
    /// rk3399_mmc_set_clk way to change the card rate. The register is
    /// plain (not hiword): read it, keep the upper bits, replace mux+div.
    fn cru_set_clksel(&mut self, value: u32, label: &str) -> Result<()> {
        let guard = self.cru.read().unwrap();
        let cru = match guard.as_ref() {
            Some(cru) => cru,
            None => {
                self.log
                    .log(format_args!("dwmmcd: CRU not mapped, cannot set clksel"));
                return Err(Error::new(EIO));
            }
        };
        unsafe {
            let old = cru.read(CRU_CLKSEL_CON16_SCLK_SDMMC);
            // Write-mask (hiword) convention: mask in [31:16], value in
            // [15:0]. A plain write is dropped by the hardware.
            cru.write(
                CRU_CLKSEL_CON16_SCLK_SDMMC,
                (CLKSEL16_MASK << 16) | value,
            );
            let mut readback = cru.read(CRU_CLKSEL_CON16_SCLK_SDMMC);
            let mut how = "hiword";
            if readback & CLKSEL16_MASK != value {
                // Some Rockchip revisions accept plain writes; try that
                // before giving up so one code path covers both.
                cru.write(
                    CRU_CLKSEL_CON16_SCLK_SDMMC,
                    (old & !CLKSEL16_MASK) | value,
                );
                readback = cru.read(CRU_CLKSEL_CON16_SCLK_SDMMC);
                how = "plain";
            }
            self.log.log(format_args!(
                "dwmmcd: clksel16 {} ({} write): 0x{:08x} -> 0x{:08x}",
                label, how, old, readback
            ));
            if readback & CLKSEL16_MASK != value {
                return Err(Error::new(EIO));
            }
        }
        Ok(())
    }

    /// The data-rate CLKSEL_CON16 value: the inherited one (the state
    /// U-Boot used to read the ESP and the kernel from this card), or the
    /// gpll/12 default when the inherited value looks implausible
    /// (bypass divider or reserved mux).
    fn data_clksel(&self) -> u32 {
        let value = self.inherited_clksel16 & 0x7ff;
        let mux = (value >> 8) & 0x7;
        let div = value & 0x7f;
        if mux > 5 || div == 0 {
            CLKSEL_DATA
        } else {
            value
        }
    }

    /// Primary clock setup, mirroring U-Boot dwmci_setup_bus on rk3399:
    /// the CRU provides the rate first (400 kHz identification), then the
    /// CLKENA off/on pair of CIU clock-update commands latches it.
    fn setup_clocks_primary(&mut self) -> Result<()> {
        self.cru_set_clksel(CLKSEL_400K, "ident-400k")?;
        self.clkena_cycle("ident")
    }

    /// Fallback clock setup when CIU clock-update commands never
    /// complete: re-reset the controller (drops any half-written
    /// command), rebuild the polling-driver register state, then program
    /// the clock registers as plain writes with no CIU involvement and
    /// set the rate from the CRU.
    fn clocks_direct(&mut self) -> Result<()> {
        self.controller_setup()?;
        {
            let regs = self.regs.read().unwrap();
            unsafe {
                regs.write(CLKDIV, 0);
                regs.write(CLKSRC, 0);
                regs.write(CLKENA, CLKENA_ENABLE | CLKENA_LOW_PWR);
                self.log.log(format_args!(
                    "dwmmcd: direct clocks: clkdiv={} clksrc={} clkena=0x{:08x}",
                    regs.read(CLKDIV),
                    regs.read(CLKSRC),
                    regs.read(CLKENA)
                ));
            }
        }
        self.cru_set_clksel(CLKSEL_400K, "ident-400k")
    }

    /// Switch to the data rate before the first data command. U-Boot path:
    /// CRU rate, then the CLKENA cycle. Direct path: CRU rate only —
    /// U-Boot also rewrites CLKSEL_CON with cclk running (its clk_set_rate
    /// runs before the CLKENA stop), so a live clock survives this.
    fn set_data_rate(&mut self) -> Result<()> {
        let value = self.data_clksel();
        if self.cru_clocking {
            if let Err(err) = self.cru_set_clksel(value, "data-rate") {
                self.log.log(format_args!(
                    "dwmmcd: data-rate CRU switch failed ({}), keeping the current rate",
                    err
                ));
            }
            return Ok(());
        }
        if let Err(err) = self.cru_set_clksel(value, "data-rate") {
            // Keep running at the identification rate rather than dying:
            // a mount at 400 kHz is slow but works.
            self.log.log(format_args!(
                "dwmmcd: data-rate CRU switch failed ({}), keeping the identification rate",
                err
            ));
            return Ok(());
        }
        if let Err(err) = self.clkena_cycle("data") {
            self.log.log(format_args!(
                "dwmmcd: data-rate clock update failed ({}), switching to direct clocking",
                err
            ));
            self.cru_clocking = true;
            let regs = self.regs.read().unwrap();
            unsafe { regs.write(CLKENA, CLKENA_ENABLE | CLKENA_LOW_PWR) };
        }
        Ok(())
    }

    /// Reset the data FIFO before a data command (self-clearing bit).
    fn reset_fifo(&mut self) -> Result<()> {
        let regs = self.regs.read().unwrap();
        unsafe {
            let ctrl = regs.read(CTRL);
            regs.write(CTRL, ctrl | CTRL_FIFO_RESET);
            let deadline = now_ms() + FIFO_RESET_TIMEOUT_MS;
            loop {
                if regs.read(CTRL) & CTRL_FIFO_RESET == 0 {
                    return Ok(());
                }
                if now_ms() > deadline {
                    return Err(Error::new(ETIMEDOUT));
                }
            }
        }
    }

    /// Send a command and wait for its response (polling, no IRQs).
    fn send_cmd(
        &mut self,
        index: u32,
        arg: u32,
        response: Response,
    ) -> std::result::Result<CmdOutcome, CmdError> {
        // A stop command aborts a data transfer instead of waiting for it,
        // and every other command waits for the previous data to finish
        // (U-Boot dwmci_send_cmd_common flag shaping).
        self.wait_while_busy();

        let regs = self.regs.read().unwrap();
        unsafe {
            // Clear all raw status bits before starting.
            regs.write(RINTSTS, 0xffff_ffff);
            regs.write(CMDARG, arg);

            let mut flags = CMD_START | CMD_USE_HOLD_REG | (index & CMD_INDEX_MASK);
            match response {
                Response::None => {}
                Response::Short => flags |= CMD_RESP_EXP | CMD_RESP_CRC,
                Response::ShortNoCrc => flags |= CMD_RESP_EXP,
                Response::Long => flags |= CMD_RESP_EXP | CMD_RESP_CRC | CMD_RESP_LONG,
            }
            if index == CMD12_STOP_TRANSMISSION {
                flags |= CMD_ABORT_STOP;
            } else {
                flags |= CMD_PRV_DAT_WAIT;
            }
            if index == CMD0_GO_IDLE {
                flags |= CMD_INIT; // 80 initialization clocks first
            }

            regs.write(CMD, flags);

            let deadline = now_ms() + CMD_TIMEOUT_MS;
            let mut rint;
            loop {
                rint = regs.read(RINTSTS);
                if rint & INT_CDONE != 0 {
                    break;
                }
                if now_ms() > deadline {
                    return Err(CmdError::Timeout);
                }
            }

            if rint & INT_RTO != 0 {
                return Err(CmdError::Timeout);
            }
            if rint & INT_RE != 0 {
                return Err(CmdError::ResponseError);
            }
            if rint & INT_RCRC != 0 {
                return Err(CmdError::ResponseCrc);
            }
            if rint & INT_HLE != 0 {
                return Err(CmdError::HardwareLocked);
            }

            let mut outcome = CmdOutcome {
                response: [0; 4],
            };
            match response {
                Response::Short | Response::ShortNoCrc => {
                    outcome.response[0] = regs.read(RESP0);
                }
                Response::Long => {
                    // RESP3 holds bits 127..96 (Linux UNSTUFF_BITS resp[0]).
                    outcome.response[0] = regs.read(RESP3);
                    outcome.response[1] = regs.read(RESP2);
                    outcome.response[2] = regs.read(RESP1);
                    outcome.response[3] = regs.read(RESP0);
                }
                Response::None => {}
            }
            Ok(outcome)
        }
    }

    /// Wait for the card/controller to leave the busy state.
    fn wait_while_busy(&mut self) {
        let regs = self.regs.read().unwrap();
        unsafe {
            let deadline = now_ms() + BUSY_TIMEOUT_MS;
            while regs.read(STATUS) & STATUS_BUSY != 0 {
                if now_ms() > deadline {
                    // U-Boot logs and continues; a stuck busy line is not
                    // necessarily fatal for the next command.
                    self.log
                        .log(format_args!("dwmmcd: busy timeout, continuing"));
                    return;
                }
            }
        }
    }

    /// Full SD card enumeration: idle -> identify -> address -> select.
    fn card_init(&mut self) -> Result<()> {
        // Identification must run at 100-400 kHz. U-Boot's rk3399 model:
        // CLKDIV stays 0 and the rate comes from the CRU. Try the U-Boot
        // sequence (CRU rate, two clock-update commands); when the CIU
        // never accepts an update, fall back to plain register writes
        // with no CIU involvement at all.
        if let Err(err) = self.setup_clocks_primary() {
            self.log.log(format_args!(
                "dwmmcd: U-Boot clock sequence failed ({}), falling back to direct clocking",
                err
            ));
            self.cru_clocking = true;
            if let Err(err) = self.clocks_direct() {
                self.log.log(format_args!(
                    "dwmmcd: direct clocking failed ({}), continuing at the inherited rate",
                    err
                ));
            }
        }

        // Software-reset the internal DMA engine now, after the clocks are
        // settled (U-Boot writes BMOD = 1 after setup_bus, not before), so
        // no descriptor state leaks into the PIO data path.
        {
            let regs = self.regs.read().unwrap();
            unsafe { regs.write(BMOD, 1) };
        }

        // CMD0: go to idle. No response; a failure here means the
        // controller cannot talk to the bus at all.
        if let Err(err) = self.send_cmd(CMD0_GO_IDLE, 0, Response::None) {
            // The CMD register readback separates the two failure modes:
            // START still set means the CIU never accepted the command;
            // START cleared without command-done means it did and stalled.
            let regs = self.regs.read().unwrap();
            self.log.log(format_args!(
                "dwmmcd: CMD0 failed: {} cmd=0x{:08x} rint=0x{:08x} status=0x{:08x}",
                err,
                unsafe { regs.read(CMD) },
                unsafe { regs.read(RINTSTS) },
                unsafe { regs.read(STATUS) }
            ));
            return Err(Error::new(EIO));
        }

        // CMD8: interface condition. A timeout is legal: SD v1.x cards do
        // not answer. Anything else is a broken bus.
        match self.send_cmd(CMD8_SEND_IF_COND, CMD8_ARG, Response::Short) {
            Ok(outcome) => {
                if outcome.response[0] != CMD8_ECHO {
                    self.log.log(format_args!(
                        "dwmmcd: CMD8 echo mismatch: 0x{:08x}",
                        outcome.response[0]
                    ));
                    return Err(Error::new(EIO));
                }
                self.log.log(format_args!(
                    "dwmmcd: CMD8 ok (v2.0 card, 0x{:08x})",
                    outcome.response[0]
                ));
            }
            Err(CmdError::Timeout) => {
                // Treat as SD v1.x; ACMD41 without HCS still works.
                self.log
                    .log(format_args!("dwmmcd: CMD8 timeout, assuming SD v1.x"));
            }
            Err(err) => {
                self.log.log(format_args!("dwmmcd: CMD8 failed: {}", err));
                return Err(Error::new(EIO));
            }
        }

        // ACMD41: poll until the card leaves busy. CMD55 with RCA 0 while
        // the card is unaddressed.
        let deadline = now_ms() + ACMD41_TIMEOUT_MS;
        let mut ocr;
        loop {
            self.send_cmd(CMD55_APP_CMD, 0, Response::Short)
                .map_err(|err| {
                    self.log
                        .log(format_args!("dwmmcd: CMD55 failed: {}", err));
                    Error::new(EIO)
                })?;

            match self.send_cmd(ACMD41_SD_SEND_OP_COND, ACMD41_ARG, Response::ShortNoCrc) {
                Ok(outcome) => {
                    ocr = outcome.response[0];
                    if ocr & ACMD41_BUSY != 0 {
                        break;
                    }
                }
                Err(CmdError::Timeout) => {}
                Err(err) => {
                    self.log
                        .log(format_args!("dwmmcd: ACMD41 failed: {}", err));
                    return Err(Error::new(EIO));
                }
            }

            if now_ms() > deadline {
                self.log
                    .log(format_args!("dwmmcd: ACMD41 busy timeout"));
                return Err(Error::new(ETIMEDOUT));
            }
            sleep_ms(ACMD41_RETRY_DELAY_MS);
        }
        self.block_addressed = ocr & ACMD41_CCS != 0;
        self.log.log(format_args!(
            "dwmmcd: ACMD41 ready, ocr=0x{:08x} ccs={}",
            ocr,
            self.block_addressed
        ));

        // CMD2: all-send CID (R2). Presence is the point, not the value.
        self.send_cmd(CMD2_ALL_SEND_CID, 0, Response::Long)
            .map_err(|err| {
                self.log.log(format_args!("dwmmcd: CMD2 failed: {}", err));
                Error::new(EIO)
            })?;

        // CMD3: ask for the relative card address (R6, RCA in [31:16]).
        let outcome = self
            .send_cmd(CMD3_SEND_REL_ADDR, 0, Response::Short)
            .map_err(|err| {
                self.log
                    .log(format_args!("dwmmcd: CMD3 failed: {}", err));
                Error::new(EIO)
            })?;
        self.rca = outcome.response[0] >> 16;
        if self.rca == 0 {
            self.log
                .log(format_args!("dwmmcd: card returned RCA 0"));
            return Err(Error::new(EIO));
        }

        // CMD9: read the CSD (R2) and derive the capacity.
        let outcome = self
            .send_cmd(CMD9_SEND_CSD, self.rca << 16, Response::Long)
            .map_err(|err| {
                self.log.log(format_args!("dwmmcd: CMD9 failed: {}", err));
                Error::new(EIO)
            })?;
        match parse_capacity(&outcome.response) {
            Some(size) => self.size = size,
            None => {
                self.log.log(format_args!(
                    "dwmmcd: CSD parse failed: {:08x} {:08x} {:08x} {:08x}",
                    outcome.response[0],
                    outcome.response[1],
                    outcome.response[2],
                    outcome.response[3]
                ));
                return Err(Error::new(EIO));
            }
        }

        // Back to the data rate before selecting the card, so the first
        // data command already runs full speed (CRU-side switch; the
        // controller stays at CLKDIV 0/bypass).
        self.set_data_rate()?;

        // CMD7: select the card, moving it to transfer state.
        self.send_cmd(CMD7_SELECT_CARD, self.rca << 16, Response::Short)
            .map_err(|err| {
                self.log.log(format_args!("dwmmcd: CMD7 failed: {}", err));
                Error::new(EIO)
            })?;

        // CMD16: force 512-byte blocks. Fixed on SDHC/SDXC, needed on SDSC.
        self.send_cmd(CMD16_SET_BLOCKLEN, 512, Response::Short)
            .map_err(|err| {
                self.log.log(format_args!("dwmmcd: CMD16 failed: {}", err));
                Error::new(EIO)
            })?;

        // ACMD6: 4-bit bus, then flip the controller to match.
        self.send_cmd(CMD55_APP_CMD, self.rca << 16, Response::Short)
            .map_err(|err| {
                self.log.log(format_args!(
                    "dwmmcd: CMD55 (ACMD6) failed: {}",
                    err
                ));
                Error::new(EIO)
            })?;
        self.send_cmd(ACMD6_SET_BUS_WIDTH, 2, Response::Short)
            .map_err(|err| {
                self.log
                    .log(format_args!("dwmmcd: ACMD6 failed: {}", err));
                Error::new(EIO)
            })?;
        let regs = self.regs.read().unwrap();
        unsafe { regs.write(CTYPE, CTYPE_4BIT) };

        Ok(())
    }

    /// Issue a data command and move the payload through the FIFO in PIO
    /// mode. `write` selects direction; `block` is the first LBA; `buffer`
    /// is the caller's slice (for writes it is only read).
    fn transfer(&mut self, block: u64, buffer: &mut [u8], write: bool) -> Result<usize> {
        if buffer.is_empty() || buffer.len() % 512 != 0 {
            return Err(Error::new(EINVAL));
        }
        let blocks = buffer.len() as u64 / 512;

        let mut done_blocks: u64 = 0;
        while done_blocks < blocks {
            let count = u64::min(MAX_BLOCKS_PER_COMMAND, blocks - done_blocks) as u32;
            let offset = done_blocks as usize * 512;
            let chunk = &mut buffer[offset..offset + count as usize * 512];

            let index = if write {
                if count == 1 {
                    CMD24_WRITE_SINGLE
                } else {
                    CMD25_WRITE_MULTI
                }
            } else if count == 1 {
                CMD17_READ_SINGLE
            } else {
                CMD18_READ_MULTI
            };

            self.data_command(index, block + done_blocks, count, chunk, write)?;

            done_blocks += count as u64;
        }

        Ok(buffer.len())
    }

    fn data_command(
        &mut self,
        index: u32,
        block: u64,
        count: u32,
        buffer: &mut [u8],
        write: bool,
    ) -> Result<()> {
        // Byte-addressed (SD v1.x) cards take byte offsets in read/write
        // commands; block-addressed cards take LBAs.
        let arg = if self.block_addressed {
            block as u32
        } else {
            (block * 512) as u32
        };

        self.reset_fifo()?;
        {
            let regs = self.regs.read().unwrap();
            unsafe {
                regs.write(BLKSIZ, 512);
                regs.write(BYTCNT, 512 * count);
            }
        }

        let mut flags = 0;
        if write {
            flags |= CMD_DAT_EXP | CMD_DAT_WR;
        } else {
            flags |= CMD_DAT_EXP;
        }
        // Data commands carry an R1 response; the extra transfer flags go
        // out with the same CMD register write.
        self.send_cmd_data(index, arg, flags)?;

        let timeout =
            now_ms() + DATA_TIMEOUT_BASE_MS + DATA_TIMEOUT_PER_BLOCK_MS * count as u64;

        let regs = self.regs.read().unwrap();
        let mut moved = 0usize; // bytes handed to/from the FIFO so far
        let total = buffer.len();
        unsafe {
            loop {
                // Move data whenever the FIFO has room/contents. This is
                // the U-Boot fifo_mode loop, driven from STATUS instead of
                // the RXDR/TXDR interrupts.
                if write {
                    while moved < total && regs.read(STATUS) & STATUS_FIFO_FULL == 0 {
                        let word = u32::from_ne_bytes([
                            buffer[moved],
                            buffer[moved + 1],
                            buffer[moved + 2],
                            buffer[moved + 3],
                        ]);
                        regs.write_data(word);
                        moved += 4;
                    }
                } else {
                    while moved < total && regs.read(STATUS) & STATUS_FIFO_EMPTY == 0 {
                        let word = regs.read_data();
                        buffer[moved..moved + 4].copy_from_slice(&word.to_ne_bytes());
                        moved += 4;
                    }
                }

                let rint = regs.read(RINTSTS);
                if rint & DATA_ERROR_MASK != 0 {
                    regs.write(RINTSTS, rint);
                    self.log.log(format_args!(
                        "dwmmcd: data error at block {} (moved {}): rint=0x{:08x}",
                        block,
                        moved,
                        rint
                    ));
                    drop(regs);
                    let _ = self.abort_transfer();
                    return Err(Error::new(EIO));
                }
                if rint & INT_DTO != 0 {
                    regs.write(RINTSTS, INT_DTO);
                    // Drain/fill whatever is left, then verify the length.
                    if write {
                        while moved < total && regs.read(STATUS) & STATUS_FIFO_FULL == 0 {
                            let word = u32::from_ne_bytes([
                                buffer[moved],
                                buffer[moved + 1],
                                buffer[moved + 2],
                                buffer[moved + 3],
                            ]);
                            regs.write_data(word);
                            moved += 4;
                        }
                    } else {
                        while moved < total && regs.read(STATUS) & STATUS_FIFO_EMPTY == 0 {
                            let word = regs.read_data();
                            buffer[moved..moved + 4].copy_from_slice(&word.to_ne_bytes());
                            moved += 4;
                        }
                    }
                    if moved < total {
                        self.log.log(format_args!(
                            "dwmmcd: data over with {} of {} bytes",
                            moved,
                            total
                        ));
                        drop(regs);
                        let _ = self.abort_transfer();
                        return Err(Error::new(EIO));
                    }
                    break;
                }

                if now_ms() > timeout {
                    self.log.log(format_args!(
                        "dwmmcd: data timeout at block {} (moved {} of {})",
                        block,
                        moved,
                        total
                    ));
                    drop(regs);
                    let _ = self.abort_transfer();
                    return Err(Error::new(ETIMEDOUT));
                }
            }
            drop(regs);
        }

        // Multi-block transfers need an explicit stop; the card stays busy
        // (R1b) until it is sent.
        if count > 1 {
            self.send_cmd(CMD12_STOP_TRANSMISSION, 0, Response::Short)
                .map_err(|err| {
                    self.log.log(format_args!(
                        "dwmmcd: CMD12 failed: {}",
                        err
                    ));
                    Error::new(EIO)
                })?;
        }

        Ok(())
    }

    /// send_cmd for data commands: appends the transfer flags to the R1
    /// response shaping.
    fn send_cmd_data(&mut self, index: u32, arg: u32, extra_flags: u32) -> Result<()> {
        self.wait_while_busy();

        let regs = self.regs.read().unwrap();
        unsafe {
            regs.write(RINTSTS, 0xffff_ffff);
            regs.write(CMDARG, arg);
            let flags = CMD_START
                | CMD_USE_HOLD_REG
                | (index & CMD_INDEX_MASK)
                | CMD_RESP_EXP
                | CMD_RESP_CRC
                | CMD_PRV_DAT_WAIT
                | extra_flags;
            regs.write(CMD, flags);

            let deadline = now_ms() + CMD_TIMEOUT_MS;
            let mut rint;
            loop {
                rint = regs.read(RINTSTS);
                if rint & INT_CDONE != 0 {
                    break;
                }
                if now_ms() > deadline {
                    self.log
                        .log(format_args!("dwmmcd: CMD{} timeout", index));
                    return Err(Error::new(ETIMEDOUT));
                }
            }
            if rint & (INT_RTO | INT_RE | INT_RCRC | INT_HLE) != 0 {
                self.log.log(format_args!(
                    "dwmmcd: CMD{} failed: rint=0x{:08x}",
                    index,
                    rint
                ));
                return Err(Error::new(EIO));
            }
        }
        Ok(())
    }

    fn abort_transfer(&mut self) -> Result<()> {
        let _ = self.send_cmd(CMD12_STOP_TRANSMISSION, 0, Response::Short);
        self.reset_fifo()
    }

    /// Read one sector and return it; used for the boot-time self-test.
    pub fn read_block_for_test(&mut self, block: u64) -> Result<[u8; 512]> {
        let mut buffer = [0u8; 512];
        self.transfer(block, &mut buffer, false)?;
        Ok(buffer)
    }
}

/// CIU clock-update command: start a CMD with only UPD_CLK set and wait
/// for the CIU to accept it. Never reaches the card (U-Boot
/// dwmci_setup_bus). Completion is the CMD START bit clearing — U-Boot
/// polls exactly that and never waits for CDONE; on this IP a
/// clock-only command does not raise CDONE in RINTSTS (iterations 2-5 of
/// experiment 6 timed out on every update with a clean RINTSTS).
unsafe fn clk_update_try(regs: &DwmmcRegs, prv_dat_wait: bool) -> Result<()> {
    unsafe {
        regs.write(RINTSTS, 0xffff_ffff);
        regs.write(CMDARG, 0);
        // The U-Boot clock-only command: START | PRV_DAT_WAIT | UPD_CLK
        // (no USE_HOLD_REG — U-Boot does not set it here). PRV_DAT_WAIT
        // defers the command until the data lines settle; iteration 2 of
        // experiment 6 measured a controller that reports BUSY/DMA state
        // right after the CTRL reset, which would stall such a command
        // forever, so the caller retries without it.
        let mut flags = CMD_START | CMD_UPD_CLK;
        if prv_dat_wait {
            flags |= CMD_PRV_DAT_WAIT;
        }
        regs.write(CMD, flags);

        let deadline = now_ms() + CLK_UPDATE_TIMEOUT_MS;
        loop {
            // U-Boot completion check: the CIU accepts the command by
            // clearing START (dwmci_setup_bus polls this bit).
            if regs.read(CMD) & CMD_START == 0 {
                // Clear whatever the CIU did raise so no stale state
                // leaks into the next command wait.
                regs.write(RINTSTS, 0xffff_ffff);
                return Ok(());
            }
            if now_ms() > deadline {
                return Err(Error::new(ETIMEDOUT));
            }
        }
    }
}

unsafe fn clk_update(regs: &DwmmcRegs) -> Result<()> {
    unsafe {
        if clk_update_try(regs, true).is_ok() {
            return Ok(());
        }
        // Retry without PRV_DAT_WAIT: a stale busy/data state (the EFI
        // loader leaves IDMAC armed) must not block a CIU-only clock
        // update, which never touches the data lines anyway. Iteration 2
        // of experiment 6 stalled here with status BUSY bits set.
        clk_update_try(regs, false)
    }
}

/// Capacity from the CSD, following Linux UNSTUFF_BITS. The response array
/// holds the 128-bit CSD big-endian: response[0] carries bits 127..96.
fn parse_capacity(csd: &[u32; 4]) -> Option<u64> {
    let value = ((csd[0] as u128) << 96)
        | ((csd[1] as u128) << 64)
        | ((csd[2] as u128) << 32)
        | csd[3] as u128;
    let structure = ((value >> 126) & 0b11) as u32;
    match structure {
        0 => {
            // CSD v1: (C_SIZE+1) * mult * blocklen.
            let read_bl_len = ((value >> 80) & 0xf) as u64;
            let c_size = ((value >> 62) & 0xfff) as u64;
            let c_size_mult = ((value >> 47) & 0b111) as u64;
            if !(9..=11).contains(&read_bl_len) {
                return None;
            }
            Some((c_size + 1) << (c_size_mult + 2 + read_bl_len))
        }
        1 => {
            // CSD v2: (C_SIZE+1) * 512 KiB, always 512-byte blocks.
            let c_size = ((value >> 48) & 0x3f_ffff) as u64;
            Some((c_size + 1) << 19)
        }
        _ => None,
    }
}

impl Disk for DwmmcHost {
    fn block_size(&self) -> u32 {
        512
    }

    fn size(&self) -> u64 {
        self.size
    }

    async fn read(&mut self, block: u64, buffer: &mut [u8]) -> Result<usize> {
        self.transfer(block, buffer, false)
    }

    async fn write(&mut self, block: u64, buffer: &[u8]) -> Result<usize> {
        // The PIO engine only reads the source on writes; reinterpreting
        // the slice through a raw pointer avoids a copy (same pattern as
        // bcm2835-sdhcid's Disk::write).
        let buffer =
            unsafe { core::slice::from_raw_parts_mut(buffer.as_ptr() as *mut u8, buffer.len()) };
        self.transfer(block, buffer, true)
    }
}
