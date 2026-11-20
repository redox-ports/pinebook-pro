# `redox-os/relibc` changes

- Upstream: https://gitlab.redox-os.org/redox-os/relibc.git
- Base commit: `b83feabc`
- Apply with: `scripts/apply-patches.sh relibc`

A `<path>.patch` file is a diff against the upstream `<path>`; every other
file is new and is copied verbatim (see `../README.md`).

## Modified files

- `src/cxa.rs`

## Thread-atexit registration and finalization

Files: `src/cxa.rs`

Thread-atexit registration and finalization shared a thread-local `RefCell` (`DTORS`): `__cxa_thread_finalize()` held the borrow across the destructor calls, so a destructor that registers another handler re-entered `__cxa_thread_atexit_impl()` and panicked with `RefCell already borrowed` (`src/cxa.rs:84`, hardware: every uutils binary — `rm -rf /tmp`, `mkdir`, `ls`, `pwd`, `head`; experiment 10). `relibc_panic` then `abort()`s with `brk #1` and buffered stdout is lost, which is why some applets appeared to die without any output. A first revision (pop one entry under a scoped borrow) was insufficient on hardware — the panic simply moved along with the added comments, because the borrow flag is set for the whole `borrow_mut`/`push`/`drop` sequence and a registration landing inside it (nested finalize, or a signal handler run by the userspace trampoline mid-drain) still trips it. The final patch keeps the stack as an intrusive list behind `Cell<*mut ThreadAtExitNode>`: registering only allocates and links a node, and `__cxa_thread_finalize()` detaches the whole stack before walking it, repeating until no new nodes appear (LIFO preserved). `__cxa_finalize()` additionally collects its entries under the lock, drops the lock, then calls them (destructors may register handlers).
