# Experiment 10 — uutils binaries die at startup (`ls`, `pwd`) and at exit (`rm`, `mkdir`)

**Date:** 2026-11-20 (autonomous work session)
**Goal:** make the root-filesystem userspace usable — the login shell (`ion`)
worked, but every uutils applet crashed.

**Related:** experiment 6 (storage/rootfs, where the crashes were first seen),
README §7 risk 14; `patches/relibc/src/cxa.rs.patch`,
`patches/kernel/src/context/signal.rs.patch`.

## Symptom (device owner, 2026-11-20)

> "1) ls также падает, надо побороть в первую очередь! 2) Клавиатура на ноутбуке не работает, но пока консоль работает, давай пока
> оставим на потом"

After `redox login:` / `ion` came up, `/usr/bin/ls` produced no output at all
and the shell returned immediately. `/usr/bin/rm -rf /tmp` and
`/usr/bin/mkdir -m a=rwxt /tmp` printed a relibc panic instead. `ion`, `login`
and `redoxfs` (the other dynamic binaries) were unaffected.

`/usr/bin/ls`, `pwd`, `rm`, `mkdir` … are all symlinks to one multicall binary
(`/usr/bin/coreutils`, uutils 0.11), so the applets share code but take
different paths — which is why the failure looked applet-specific.

## Evidence from the serial logs

Two distinct crash sites, both stable across runs (`logs/`):

```
# rm / mkdir  (they reach main and die at exit)
RELIBC PANIC: panicked at src/cxa.rs:84:11:
RefCell already borrowed
ESR_EL1: 00000000F2000001          # EC=0x3C BRK, ISS=1 -> brk #1
ELR_EL1: 0000000000ACF534

# ls / pwd  (silent, no message at all)
kernel::arch::aarch64::interrupt::exception:ERROR -- FATAL: Not an SVC induced synchronous exception (ty=111100)
ELR_EL1: 0000000000C09E10
X0..X4: FFFFFFFFFFFFFFFF
X8:     000000000000009E          # 158 = SYS_YIELD
SPSR_EL1: 0000000020000000         # C set (by the `cmn` below)
```

### Resolving the addresses (module load bases)

The dumped PCs are absolute, and the dynamic loader does **not** put the
executable at address 0. From the same dump, `X19` is `relibc_start_v1`'s second
argument (`main`, saved by the first instructions) and the frame-pointer chain
holds `relibc_crt0` / `relibc_panic` return addresses:

| Dumped value | Symbol (offset in the file)  | Implied base         |
| ------------ | ---------------------------- | -------------------- |
| `X19=1F1834` | `main` @ `1F0834`            | executable `0x1000`  |
| `PC=EF034`   | `relibc_crt0` @ `EE034`      | executable `0x1000`  |
| `PC=EF048`   | `relibc_panic` @ `EE048`     | executable `0x1000`  |
| `X17=C0973C` | `relibc_start_v1` @ `18973C` | `libc.so` `0xA80000` |

With `libc.so` at `0xA80000` both recorded crash PCs land on `brk #1`
instructions inside `libc.so`:

- `0xACF534 - 0xA80000 = 0x4F534` → `brk #1` (the relibc panic/abort path)
- `0xC09E10 - 0xA80000 = 0x189E10` → `brk #1` right after the inlined
  `Sys::verify()` check of `relibc_start_v1`

(Independent confirmation: the difference between the two crash PCs,
`0x13A8DC`, equals the difference between those two `brk` sites.)

## Root cause 1 — relibc thread-atexit finalization is re-entrant (exit path)

`relibc/src/cxa.rs` held the thread-local `DTORS` `RefCell` borrow across the
destructor calls:

```rust
pub unsafe fn __cxa_thread_finalize() {
    let mut dtors = DTORS.borrow_mut();
    while let Some(entry) = dtors.pop() {
        (entry.func)(entry.obj);          // may call __cxa_thread_atexit_impl
    }
}
```

A destructor that registers another thread-atexit handler re-enters
`__cxa_thread_atexit_impl()`, whose `DTORS.borrow_mut()` is line 84 — the panic
in the log. `relibc_panic` prints and then `abort()`s, which is the `brk #1`,
and buffered stdout is lost. `__cxa_finalize()` had the same shape with its spin
mutex (destructors called while the lock is held).

**Fix, step 1** (`patches/relibc/src/cxa.rs.patch`, first revision): pop one
entry under a scoped borrow and run the destructor after the borrow is released.
On hardware this was **not enough** — the panic moved from `src/cxa.rs:84` to
`src/cxa.rs:93` (the same `DTORS.borrow_mut()` in `__cxa_thread_atexit_impl`,
shifted by the added comments) and still fired for _every_ applet, including
`ls`, `pwd` and `head`, which now got past startup (the kernel fix below). The
reason: a `RefCell` borrow flag is set for the whole
`borrow_mut`/`push`/`drop` sequence, and a registration that lands inside it —
from a nested finalize, or from a signal handler that the userspace signal
trampoline runs in the middle of the drain — trips it.

**Fix, step 2** (same patch, final revision): the thread-atexit stack is an
intrusive list behind `Cell<*mut ThreadAtExitNode>`. Registering only allocates
and links a node, so it can never observe a drain in progress;
`__cxa_thread_finalize` detaches the whole stack before walking it and repeats
until no new nodes appear (LIFO order preserved, handlers registered by a
destructor run in the next pass). `__cxa_finalize` collects the atexit entries
under its lock, drops the lock, then calls them.

## Root cause 2 — a signal delivered inside a syscall returns its argument (startup path)

`relibc_start_v1()` begins with

```rust
unsafe { relibc_verify_host() };   // Sys::verify(): syscall5(SYS_YIELD, !0 x5).is_ok()
```

which compiles to exactly what the dump shows:

```asm
189744: mov  w8, #0x9e          ; SYS_YIELD = 158
189768: mov  x0, #-1            ; arg0
...
189778: svc  #0
18977c: cmn  w0, #0x84          ; is_ok()?
189780: b.hs 0x189e10           ; -> brk #1
```

`X0` still held the **argument** `-1`, not a result, and the carry flag in
`SPSR_EL1` shows the `cmn` saw `w0 < 0`, i.e. an error. But the kernel cannot
return an error here: `SYS_YIELD => sched_yield(token).map(|()| 0)` cannot fail.
A probe in a static initfs daemon confirmed the syscall is healthy:

```
[   0.001] yieldprobe: YIELD with !0 x5 -> 0 (0x0)
[   0.004] yieldprobe: yield with 0 x5 -> 0 (0x0)
[   0.004] yieldprobe: bogus 9999 -> -38 (0xffffffffffffffda)   # ENOSYS, as expected
```

The bug is the delivery order. `signal_handler()` is invoked from
`sched_yield()` (`src/syscall/time.rs`) and from `context::switch::tick()`
(timer tick), i.e. **while a syscall is still executing**, and it stores

```rust
let ip = regs.instr_pointer();                 // instruction after `svc`
let archdep_reg = regs.sig_archdep_reg();      // aarch64: scratch.x0
regs.set_instr_pointer(sigh_instr_ptr);
thread_ctl.saved_ip.set(ip);
thread_ctl.saved_archdep_reg.set(archdep_reg);
```

for the userspace signal trampoline. The arch syscall epilogue
(`src/arch/aarch64/interrupt/exception.rs`) writes the result only afterwards:

```rust
let ret = syscall::syscall(...);
stack.scratch.x0 = ret;
```

So a signal delivered in that window saves the syscall _argument_, and
`sigreturn` restores it as the return value. `ls`/`pwd` resumed at `0x18977c`
with `x0 = -1`, took the abort branch, and died silently. A freshly spawned
process can already have a pending signal (that is why `rm`/`mkdir` got past
startup and `ls`/`pwd` did not), so this was not a narrow race.

**Fix** (`patches/kernel/src/context/signal.rs.patch`):
`signal_handler()` returns early while `percpu.inside_syscall` is set (the
signal stays pending; `being_sigkilled` is still honoured before that check),
and the aarch64 syscall epilogue calls it immediately after
`stack.scratch.x0 = ret`. Delivery from a timer tick in userspace is unchanged.

## Verification

- Rebuilt `relibc` and the kernel; the image contains the new `libc.so`
  (sha256 compared with the recipe's staged binary) and the new kernel
  (same comparison).
- QEMU smoke test still passes (`scripts/qemu-smoke.sh`): U-Boot devicetree,
  UART bind, initfs userspace, `debugsh` console.
- **Hardware run 2 (kernel signal fix, step 1 of the relibc fix):** the startup
  crash is gone from the log — `ls`, `pwd` and `head` now reach `main` and the
  `ELR_EL1` that used to point into `relibc_start_v1` no longer appears. All
  applets then panic at exit with `panicked at src/cxa.rs:93:11: RefCell
already borrowed`, which is what step 2 above addresses.
- **Hardware result of step 2 (experiment 12):** verified — root login works,
  `ls /bin` and `ls /usr/bin | head` print complete listings (uutils applets
  run and exit cleanly), and the `brk #1` / `RefCell already borrowed` panics
  are gone from the log.
- QEMU cannot exercise the fix end to end: it has no root filesystem (no
  virtio-mmio block driver), and the initfs contains only static binaries, so
  no dynamic `libc.so` startup happens there.

## Follow-ups

- Pending signals survive `fork`/`exec` here (POSIX requires them cleared). The
  kernel fix makes that harmless, but clearing them on `exec` is still the
  correct behaviour and would remove the "inherited signal" trigger entirely.
- `[src/procmgr.rs:2071 ERROR] failed to send SIGCHLD to parent PID
ProcessId(28): Operation not permitted` is still unexplained; signals are
  plain `sigcontrol` state in this kernel generation, so the `EPERM` comes from
  the userspace side of the protocol.
