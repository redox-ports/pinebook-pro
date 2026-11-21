# Experiment 13 — The orbital desktop image

**Date:** 2026-11-21
**Goal:** build an image that boots into the Redox desktop (`orbital`) on the
Pinebook Pro, and keep the build reproducible after the recipe sources drifted
during the desktop cooks.

**Related:** experiment 8 (USB input — the touchpad interface attempt is its
iteration 16), experiment 7 (RGB565 framebuffer), README §4 (local build loop),
§7 risk 16 (recipe source drift).

## Image contents

`config/aarch64/pinebook-pro/desktop-minimal.toml` includes the upstream
`config/aarch64/desktop-minimal.toml` and only changes the sizes for the board
(`filesystem_size = 512`, `efi_partition_size = 128`). On top of the minimal
package set the image carries `orbdata`, `orbital`, `orbterm`, `orbutils` and
the `20_orbital` init script (`nowait VT=3 orbital orblogin launcher`); the
upstream config also disables `30_console_switch`.

`scripts/build-image.sh` and `scripts/flash-sd.sh` gained `--config NAME`
(default `minimal`) so the image lands in
`build/redox/build/aarch64/<config>/harddrive.img` and the flash helper picks
the same file.

## Build iterations

### Iteration 1 — three parallel `repo cook` runs deadlock each other

Three cooks were started in sequence while the previous one was still running
(`repo cook orbital --filesystem=…`, then `repo cook orbital orbterm orbutils`,
then `make all`). A `repo cook` failure opens a `FAILURE in <recipe>
[Skip][Inspect][Exit][Retry]` dialog and **waits for input forever**; the
later runs then wait for the cook lock. `make all` never produced an image.
Measured: `desktop-cook5.log` (16:44) sat in the orbital dialog while
`desktop-cook6.log` (17:12) and the `make all` run (17:24) blocked.

Rule: kill leftover cooks (`pkill -f "repo cook"`) before starting one, and run
exactly one at a time.

### Iteration 2 — the orbital cook failed on a gitlab fetch without offline mode

`repo cook orbital --filesystem=…` (no `CARGO_NET_OFFLINE`) failed with
`spurious network error … SSL error: syscall failure: Connection timed out`
while "Updating git repository `https://gitlab.redox-os.org/redox-os/base.git`",
then `failed to get graphics-ipc as a dependency of package orbital`. The
runtime user (`~/.gitconfig` `insteadOf` to the local base clone) is only used
by git; cargo resolves its own git dependencies and the flaky GitLab is not
usable for that. With `CARGO_NET_OFFLINE=true` and the `Cargo.lock` already
pinned to `base.git#c4f68cd`, the cached checkout under
`~/.cargo/git/checkouts/base-*` satisfies the dependency and the cook builds.

All desktop cooks must therefore run with `COOKBOOK_OFFLINE=true
CARGO_NET_OFFLINE=true`; the gui recipe sources are pre-cloned into
`recipes/gui/<name>/source` (GitHub mirrors) since the cookbook skips existing
source trees.

### Iteration 3 — recipe sources drift on every online cook

The one online run (`make all`, 17:24, no `COOKBOOK_OFFLINE`) moved the recipe
sources to the branch heads before cooking:

| recipe       | before     | after      |
| ------------ | ---------- | ---------- |
| `kernel`     | `e7f1d423` | `f64a8163` |
| `relibc`     | `b83feabc` | `74b5e7e3` |
| `uutils-grep`| `6f048c6`  | `00e7bec`  |

Two consequences were measured:

- the kernel package was rebuilt from `f64a8163` + our patches — an
  **unverified newer kernel** would have shipped silently (the port's verified
  baseline is `e7f1d423` + `patches/kernel`, hardware-verified in experiment 12);
- `uutils-grep@00e7bec` needs `uucore ^0.13.0`, which is not in the offline
  cargo cache, so the offline cook failed
  (`location searched: crates.io index … offline mode`). `make image` still
  cooks any recipe whose source ident changed, so the image build failed too.

Fix: reset the three sources to the verified commits, then re-apply the diffs
(`scripts/apply-patches.sh kernel relibc`; it is idempotent):

```
git -C build/redox/recipes/core/kernel/source reset --hard e7f1d423
git -C build/redox/recipes/core/relibc/source reset --hard b83feabc
git -C build/redox/recipes/tools/uutils-grep/source reset --hard 6f048c6
bash scripts/apply-patches.sh
```

`base` does not drift: its GitLab URL is rewritten to the local clone and the
checkout stays at `8f576d27` (the commit recorded in `patches/base/README.md`).
The `uutils` and `uutils-procps` recipes are pinned checkouts (`0.11.0`,
`ee98c0a`) and do not move either.

After the resets `make image CONFIG_NAME=desktop-minimal
FILESYSTEM_CONFIG=config/aarch64/pinebook-pro/desktop-minimal.toml` cooked
`kernel`, `relibc` and `uutils-grep` once more from the verified sources
(`cook … - successful`), everything else came from the cache, and the image was
assembled.

### Iteration 4 — the touchpad endpoint attempt had a compile error

The uncommitted `HidKind::Mouse` change (experiment 8, iteration 16) left one
`match kind` arm uncovered; the base cook failed with
`error[E0004]: non-exhaustive patterns: 'HidKind::Mouse' not covered`. Fixed
(new arm, the old `Drain` arm now logs "drain"), `make r.base` succeeded and
published base.

### Iteration 5 — QEMU rig smoke of the desktop image

The two-`pci-ohci` rig (experiment 8) booted the desktop image: bootloader →
kernel → initfs, `ohcid` probes both controllers, the keyboard (`0627:0001`
protocol 1, endpoint `0x81`) enumerates on `0x10100000` and the hub
(`0409:55aa`) plus the tablet behind hub port 2 enumerate on `0x10100100`.
Monitor `sendkey a b c` increased the report counter (0 → 2 → 6 → 0 per
heartbeat), `mouse_move`/`mouse_button` reports from the tablet were consumed
as well, and no heartbeat stalled or port reset fired: **no regression of the
input path** with the mouse code in. The tablet is a single-interface HID
device (protocol 0), so it is armed through the existing "first HID endpoint"
fallback and its 3-byte reports are ignored by the 8-byte keyboard guard; the
new `Mouse` path (a *second* endpoint of the keyboard device) cannot be
reproduced with QEMU's stock USB devices — it needs the hardware.

The rig cannot mount the root filesystem (no virtio-mmio block driver), so
`redoxfs` fails and `orbital` never starts there: the rig validates boot and
input only.

## Built image

`build/redox/build/aarch64/desktop-minimal/harddrive.img`, 512 MiB,
sha256 `07bdfdc197ec6fd3aa927e8c0d4ad9dac8696ac12384d8cd0fa11f84bb57aefe`
(local build 2026-11-21 18:02; not committed).

## Actions (hardware)

1. Flash the desktop image:

   ```
   scripts/flash-sd.sh --config desktop-minimal
   ```

2. Boot with the serial console attached (`scripts/serial-log.sh`) and inspect:
   - `20_orbital` starts `orbital`, `orblogin`, `launcher`;
   - two lines of interest from `ohcid` right after the keyboard enumerates:
     `boot mouse on endpoint 0x82 of the keyboard device` (the chip accepted
     `SET_PROTOCOL(boot)` on the touchpad interface) or
     `draining extra endpoint 0x82 of the keyboard device` (the request
     stalled, reports are read and dropped as before);
   - the panel shows the orbital desktop at 1920x1080 (RGB565).

## Results (owner)

`FILL IN` — panel photo/log, the `ohcid` endpoint line, whether moving a
finger moves the cursor, and whether the keyboard still works after touching
the touchpad (the experiment 8, iteration 15 failure mode).

## Open questions

- Does the keyboard chip accept `SET_PROTOCOL(boot)` on interface 1? Its
  interface descriptor says protocol 0 (no boot protocol), but many HID
  devices still honour the request. If it stalls, the touchpad stays a drain
  target and the cursor needs a different path (raw report parser for the
  generic HID interface).
- `orbital` on RGB565: `vesad` exports the framebuffer format from
  `FRAMEBUFFER_BPP`; the desktop has never run on this board yet, so colour
  mapping is unverified.
