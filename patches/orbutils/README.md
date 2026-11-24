# `redox-os/orbutils` changes

- Upstream: https://gitlab.redox-os.org/redox-os/orbutils.git
- Base commit: `380995d`
- Apply with: `scripts/apply-patches.sh orbutils`

A `<path>.patch` file is a diff against the upstream `<path>`; every other
file is new and is copied verbatim (see `../README.md`). The recipe lives
under `recipes/gui/`, so `apply-patches.sh` discovers the checkout by recipe
name.

## Modified files

- `orbutils/src/orblogin/main.rs`

## `orblogin` — session-lifecycle logging to the serial console

**Temporary bring-up instrumentation** (experiment 15). `init` starts the
desktop as `nowait VT=3 orbital orblogin launcher`, so the login manager's
stdout/stderr go to VT3 and are invisible on UART. When a session ends the
login window is supposed to come back; it does not, and nothing about the
failure is visible in a serial capture.

The patch adds:

- a `/scheme/debug` writer (`serial()`), opened once at startup — before the
  first login switches namespaces — so the fd stays valid across the
  `apply_login_schemes`/`setns` cycles;
- a panic hook that mirrors the panic message to the same channel (panics
  normally die on VT3);
- one line per lifecycle step: login window opening, user + scheme switch,
  session pid started / ended / wait failure, namespace restored or failed,
  and the login-window error path.

Remove the patch (and re-cook `orbutils`) once the session-exit path is
understood.
