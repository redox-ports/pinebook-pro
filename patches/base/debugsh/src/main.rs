//! debugsh - interactive debug shell on the kernel debug scheme.
//!
//! Runs during the initfs phase of boot, before a root filesystem is
//! available, so storage, kernel and driver schemes can be inspected on
//! hardware (see the Pinebook Pro port). Input is the serial RX stream fed
//! into the `debug` scheme by the UART interrupt handler; output is written
//! straight back to the serial console. Provides a tiny line-discipline
//! (echo, backspace) plus a few built-ins; real work is done either by the
//! built-ins or by exec'ing binaries from the initfs.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::FromRawFd;
use std::process::Command;

use libredox::{Fd, flag};

const DEBUG_SCHEME: &str = "/scheme/debug";
const MAX_LINE: usize = 512;

fn switch_stdio() -> io::Result<()> {
    let stdin = Fd::open(DEBUG_SCHEME, flag::O_RDONLY, 0)?;
    let stdout = Fd::open(DEBUG_SCHEME, flag::O_WRONLY, 0)?;
    let stderr = Fd::open(DEBUG_SCHEME, flag::O_WRONLY, 0)?;

    stdin.dup2(0)?;
    stdout.dup2(1)?;
    stderr.dup2(2)?;

    Ok(())
}

fn main() {
    daemon::Daemon::new(run);
}

/// How long to wait for the storage driver at startup. dwmmcd registers its
/// scheme within a fraction of a second of the system starting; the login
/// prompt appears much later, so a short wait is enough to decide.
const STORAGE_WAIT_MS: u64 = 3000;

/// Spin (with yields, so the single CPU still runs the driver) until
/// `disk.mmc` appears or the timeout expires. Sleeping is not an option:
/// the bring-up kernel never wakes `nanosleep` on the timer.
fn wait_for_storage(timeout_ms: u64) -> bool {
    let start = std::time::Instant::now();
    loop {
        if storage_ready() {
            return true;
        }
        if start.elapsed().as_millis() as u64 >= timeout_ms {
            return false;
        }
        std::thread::yield_now();
    }
}

/// True when a `disk.mmc` scheme is registered in the scheme namespace.
fn storage_ready() -> bool {
    match fs::read_dir("/scheme") {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .any(|entry| entry.file_name().as_bytes() == b"disk.mmc"),
        Err(_) => false,
    }
}

fn run(daemon: daemon::Daemon) -> ! {
    if let Err(err) = switch_stdio() {
        panic!("debugsh: failed to open {}: {}", DEBUG_SCHEME, err);
    }

    // The process starts with an unusable working directory (the root scheme
    // is the initfs only after `switch_root`, and "/" itself does not
    // resolve until then), so point it at the initfs scheme: a bare `ls`
    // then lists the initfs root instead of failing with EACCES/ENODEV.
    if let Err(err) = std::env::set_current_dir("/scheme/initfs") {
        let mut lock = io::stdout().lock();
        let _ = writeln!(
            lock,
            "debugsh: failed to set cwd to /scheme/initfs: {}",
            err
        );
        let _ = lock.flush();
    }

    daemon.ready();

    // Once storage works, the real system provides a login prompt on the
    // same serial console, and this shell would steal its input (both read
    // the UART RX stream). init starts the initfs services concurrently
    // with the driver target (measured: this shell starts before dwmmcd
    // registers its scheme), so wait a bounded time for `disk.mmc` and
    // step aside when it appears. If it never does (storage failed), the
    // shell stays as the bring-up console.
    if wait_for_storage(STORAGE_WAIT_MS) {
        let mut lock = io::stdout().lock();
        let _ = writeln!(
            lock,
            "debugsh: disk.mmc is up, releasing the console to the login"
        );
        let _ = lock.flush();
        std::process::exit(0);
    }

    let stdout = io::stdout();
    // SAFETY: fd 0 was just opened by switch_stdio and stays open for the
    // lifetime of the process.
    let mut stdin = unsafe { File::from_raw_fd(0) };

    {
        let mut lock = stdout.lock();
        let _ = writeln!(lock, "debugsh: interactive console on {}", DEBUG_SCHEME);
        let _ = writeln!(lock, "debugsh: type 'help' to list built-in commands");
        let _ = lock.flush();
    }

    let mut line: Vec<u8> = Vec::new();
    let mut byte = [0_u8; 1];

    prompt(&stdout);
    loop {
        match stdin.read(&mut byte) {
            Ok(0) => continue,
            Ok(_) => {
                // Storage may have come up after the startup wait (slow
                // card, delayed driver): drop the console on the first
                // keypress instead of eating the whole login attempt.
                if storage_ready() {
                    let mut lock = stdout.lock();
                    let _ = writeln!(
                        lock,
                        "debugsh: disk.mmc is up, releasing the console to the login"
                    );
                    let _ = lock.flush();
                    std::process::exit(0);
                }
            }
            Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => {
                let mut lock = stdout.lock();
                let _ = writeln!(
                    lock,
                    "debugsh: failed to read from {}: {}",
                    DEBUG_SCHEME, err
                );
                let _ = lock.flush();
                continue;
            }
        }

        let mut lock = stdout.lock();
        match byte[0] {
            b'\r' | b'\n' => {
                drop(lock);

                let command = String::from_utf8_lossy(&line).into_owned();
                line.clear();

                let _ = writeln!(stdout.lock());
                let _ = stdout.lock().flush();

                execute(&command, &stdout);
                prompt(&stdout);
            }
            0x7F | 0x08 => {
                if line.pop().is_some() {
                    let _ = lock.write_all(b"\x08 \x08");
                    let _ = lock.flush();
                }
            }
            0x03 => {
                line.clear();
                let _ = lock.write_all(b"^C\n");
                let _ = lock.flush();
                drop(lock);

                prompt(&stdout);
            }
            0x15 => {
                line.clear();
                let _ = lock.write_all(b"\n");
                let _ = lock.flush();
                drop(lock);

                prompt(&stdout);
            }
            byte if byte.is_ascii_graphic() || byte == b' ' => {
                if line.len() < MAX_LINE {
                    line.push(byte);
                    let _ = lock.write_all(&[byte]);
                    let _ = lock.flush();
                }
            }
            _ => {}
        }
    }
}

fn prompt(stdout: &io::Stdout) {
    let mut lock = stdout.lock();
    let _ = write!(lock, "debugsh> ");
    let _ = lock.flush();
}

fn execute(command: &str, stdout: &io::Stdout) {
    let mut args = command.split_whitespace();
    let Some(program) = args.next() else {
        return;
    };
    let args: Vec<&str> = args.collect();

    let result = match program {
        "help" => help(stdout),
        "ls" => ls(args.first().copied().unwrap_or("."), stdout),
        "cat" => cat(&args, stdout),
        "echo" => echo(&args, stdout),
        "hexdump" => hexdump(&args, stdout),
        "env" => env(stdout),
        "exec" => exec(&args, stdout),
        "reboot" => reboot(stdout),
        "exit" => {
            let mut lock = stdout.lock();
            let _ = writeln!(lock, "debugsh: bye");
            let _ = lock.flush();
            std::process::exit(0);
        }
        _ => {
            let mut lock = stdout.lock();
            let _ = writeln!(
                lock,
                "debugsh: unknown command '{}', type 'help' for built-ins",
                program
            );
            let _ = lock.flush();
            Ok(())
        }
    };

    if let Err(err) = result {
        let mut lock = stdout.lock();
        let _ = writeln!(lock, "debugsh: {}: {}", program, err);
        let _ = lock.flush();
    }
}

fn help(stdout: &io::Stdout) -> io::Result<()> {
    let mut lock = stdout.lock();
    for line in [
        "debugsh: built-in commands:",
        "  help                     show this help",
        "  ls [PATH]                list directory entries (/ lists /scheme)",
        "  cat FILE...              print files to the console",
        "  echo TEXT... [> FILE]    print text, or write it to a file",
        "  hexdump FILE [BYTES]     hex dump a file (default 256 bytes)",
        "  env                      print environment variables",
        "  exec CMD [ARGS...]       run a binary (PATH=/scheme/initfs/bin)",
        "  reboot                   reset the board (kernel sys:kstop, PSCI)",
        "  exit                     leave the debug shell",
    ] {
        writeln!(lock, "{}", line)?;
    }
    lock.flush()
}

fn ls(path: &str, stdout: &io::Stdout) -> io::Result<()> {
    // During the initfs phase the filesystem root does not resolve (it
    // appears after `switch_root`), so map "/" to the scheme namespace,
    // which is what navigation from here actually needs.
    let path = if path == "/" { "/scheme" } else { path };

    let mut entries: BTreeMap<String, char> = BTreeMap::new();

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = String::from_utf8_lossy(entry.file_name().as_bytes()).into_owned();

        let suffix = match entry.file_type() {
            Ok(file_type) if file_type.is_dir() => '/',
            Ok(file_type) if file_type.is_symlink() => '@',
            Ok(_) => ' ',
            Err(_) => '?',
        };
        entries.insert(name, suffix);
    }

    let mut lock = stdout.lock();
    for (name, suffix) in &entries {
        writeln!(lock, "{}{}", name, suffix)?;
    }
    lock.flush()
}

fn cat(paths: &[&str], stdout: &io::Stdout) -> io::Result<()> {
    if paths.is_empty() {
        let mut lock = stdout.lock();
        writeln!(lock, "debugsh: usage: cat FILE...")?;
        return lock.flush();
    }

    for path in paths {
        let mut file = File::open(path)?;
        let mut buffer = [0_u8; 4096];

        loop {
            let count = read_up_to(&mut file, &mut buffer)?;
            if count == 0 {
                break;
            }

            let mut lock = stdout.lock();
            lock.write_all(&buffer[..count])?;
            lock.flush()?;
        }
    }

    Ok(())
}

fn echo(args: &[&str], stdout: &io::Stdout) -> io::Result<()> {
    let mut text: Vec<&str> = Vec::new();
    let mut redirect: Option<&str> = None;

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if *arg == ">" {
            redirect = iter.next().copied();
            break;
        }
        text.push(arg);
    }

    let mut line = text.join(" ");
    line.push('\n');

    match redirect {
        Some(path) => fs::write(path, line.as_bytes()),
        None => {
            let mut lock = stdout.lock();
            lock.write_all(line.as_bytes())?;
            lock.flush()
        }
    }
}

fn hexdump(args: &[&str], stdout: &io::Stdout) -> io::Result<()> {
    let Some(path) = args.first() else {
        let mut lock = stdout.lock();
        writeln!(lock, "debugsh: usage: hexdump FILE [BYTES]")?;
        return lock.flush();
    };

    let max = args
        .get(1)
        .and_then(|arg| arg.parse::<usize>().ok())
        .unwrap_or(256)
        .min(1 << 20);

    let mut file = File::open(path)?;
    let mut buffer = vec![0_u8; max];
    let count = read_up_to(&mut file, &mut buffer)?;

    let mut lock = stdout.lock();
    for (offset, chunk) in buffer[..count].chunks(16).enumerate() {
        write!(lock, "{:08x}  ", offset * 16)?;

        for (index, byte) in chunk.iter().enumerate() {
            if index == 8 {
                write!(lock, " ")?;
            }
            write!(lock, "{:02x} ", byte)?;
        }
        for _ in chunk.len()..16 {
            write!(lock, "   ")?;
        }

        write!(lock, " |")?;
        for byte in chunk {
            if byte.is_ascii_graphic() || *byte == b' ' {
                write!(lock, "{}", *byte as char)?;
            } else {
                write!(lock, ".")?;
            }
        }
        writeln!(lock, "|")?;
    }
    writeln!(lock, "{}", count)?;
    lock.flush()
}

fn env(stdout: &io::Stdout) -> io::Result<()> {
    let vars: BTreeMap<String, String> = std::env::vars_os()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            )
        })
        .collect();

    let mut lock = stdout.lock();
    for (key, value) in &vars {
        writeln!(lock, "{}={}", key, value)?;
    }
    lock.flush()
}

fn reboot(stdout: &io::Stdout) -> io::Result<()> {
    // PSCI SYSTEM_RESET through the kernel sys scheme; on success this
    // never returns.
    match fs::write("sys:kstop", b"reset") {
        Ok(()) => {
            let mut lock = stdout.lock();
            writeln!(lock, "debugsh: reset requested, waiting...")?;
            lock.flush()
        }
        Err(err) => {
            let mut lock = stdout.lock();
            writeln!(lock, "debugsh: failed to reset: {}", err)?;
            lock.flush()
        }
    }
}

fn exec(args: &[&str], stdout: &io::Stdout) -> io::Result<()> {
    let Some(program) = args.first() else {
        let mut lock = stdout.lock();
        writeln!(lock, "debugsh: usage: exec CMD [ARGS...]")?;
        return lock.flush();
    };

    let status = Command::new(program).args(&args[1..]).status()?;

    let mut lock = stdout.lock();
    match status.code() {
        Some(code) => writeln!(lock, "debugsh: {} exited with status {}", program, code)?,
        None => writeln!(lock, "debugsh: {} was terminated by a signal", program)?,
    }
    lock.flush()
}

// File::read returns as soon as some data is available on scheme files; keep
// reading until the buffer is full or EOF.
fn read_up_to(file: &mut File, buffer: &mut [u8]) -> io::Result<usize> {
    let mut count = 0;
    while count < buffer.len() {
        match file.read(&mut buffer[count..]) {
            Ok(0) => break,
            Ok(read) => count += read,
            Err(ref err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        }
    }
    Ok(count)
}
