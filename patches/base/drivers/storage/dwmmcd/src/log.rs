//! Bring-up logging for dwmmcd.
//!
//! stdout/stderr of initfs daemons go to the `log` scheme, which lives in
//! RAM and is invisible on the serial console. The kernel `debug` scheme
//! (`/scheme/debug`) writes straight to the UART, so every driver message is
//! also mirrored there while the port is being brought up. The fd is opened
//! before the driver enters the null namespace, so it stays valid
//! afterwards.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::sync::Mutex;

use libredox::flag;

pub struct Logger {
    uart: Mutex<Option<std::fs::File>>,
    start_ms: u64,
}

fn now_ms() -> u64 {
    let mut ts = syscall::TimeSpec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // Best effort: a broken time scheme degrades the timestamps to zero
    // instead of panicking inside Logger::new.
    if syscall::clock_gettime(syscall::flag::CLOCK_MONOTONIC, &mut ts).is_err() {
        return 0;
    }
    ts.tv_sec as u64 * 1000 + ts.tv_nsec as u64 / 1_000_000
}

impl Logger {
    pub fn new() -> Self {
        let start_ms = now_ms();
        let uart = match OpenOptions::new()
            .write(true)
            .custom_flags(flag::O_WRONLY | flag::O_CLOEXEC)
            .open("/scheme/debug")
        {
            Ok(file) => Some(file),
            Err(err) => {
                // Not fatal: the log scheme still receives everything.
                let mut stderr = std::io::stderr();
                let _ = writeln!(stderr, "dwmmcd: cannot open /scheme/debug: {}", err);
                None
            }
        };
        Logger {
            uart: Mutex::new(uart),
            start_ms,
        }
    }

    /// Emit one line to the UART (kernel debug scheme). stderr is NOT
    /// written: initfs daemons inherit init's stdio, which is already the
    /// debug scheme, so writing both duplicates every line on the serial
    /// console (observed in experiment 6).
    pub fn log(&self, args: core::fmt::Arguments) {
        // Seconds since driver start, so the serial capture shows how long
        // each bring-up step actually takes.
        let elapsed = (now_ms().saturating_sub(self.start_ms)) as f64 / 1000.0;
        let line = format!("[{:8.3}] {}\n", elapsed, args);
        if let Some(uart) = self.uart.lock().unwrap().as_mut() {
            let _ = uart.write_all(line.as_bytes());
            let _ = uart.flush();
        }
    }
}
