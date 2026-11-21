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

| recipe        | before     | after      |
| ------------- | ---------- | ---------- |
| `kernel`      | `e7f1d423` | `f64a8163` |
| `relibc`      | `b83feabc` | `74b5e7e3` |
| `uutils-grep` | `6f048c6`  | `00e7bec`  |

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
new `Mouse` path (a _second_ endpoint of the keyboard device) cannot be
reproduced with QEMU's stock USB devices — it needs the hardware.

The rig cannot mount the root filesystem (no virtio-mmio block driver), so
`redoxfs` fails and `orbital` never starts there: the rig validates boot and
input only.

## Built images

- `07bdfdc197ec6fd3aa927e8c0d4ad9dac8696ac12384d8cd0fa11f84bb57aefe` — the
  first desktop image (hardware run: desktop and keyboard work, touchpad
  misparsed, freeze);
- `98918225ccec40f112717e50582270f5ab55ef7512ce46a7d8a954fb69e4f66b` — the
  same tree with the report-descriptor layout parser and the first revive fix
  (hardware run: no freeze, Super key works, descriptor stalls so the layout
  fell back to boot; ~50 s port resets);
- `d8b403bb7379b185d73229837c2417f7a56c669daa732dfd1d713dab680f5216` — adds
  the measured Pinebook Pro touchpad layout and the split control/interrupt
  DMA arenas (2026-11-21 19:10); hardware run pending.

All are local builds (`build/aarch64/desktop-minimal/harddrive.img`), not
committed.

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

**Iteration 1 (2026-11-21, image `07bdfdc1…`): the desktop works on hardware.**
`orbital` starts on the panel, the keyboard types into it, and the boot ran
through to the desktop on the first try. One defect: the touchpad.

- the cursor is stuck at the left edge and cannot be moved right;
- moving a finger **left-right** moves the cursor **up-down**;
- pressing the left button moves the cursor slightly **right**;
- after a few touches/clicks everything froze.

**Iteration 2 (2026-11-21, image `98918225…`):** no more freeze, the Super key
reaches the desktop, the cursor still does not follow the finger. The log
shows why the descriptor-driven fix did not take effect.

## The touchpad report layout (2026-11-21)

Our parser assumed the boot layout `[buttons, dx, dy]`, i.e. `dx = byte 1`,
`dy = byte 2`. The iteration-1 symptoms already pinned the real layout:

- horizontal finger motion changed **byte 2** (our `dy`) → the cursor moved
  vertically;
- the button bit changed **byte 1** (our `dx`) → a click moved the cursor 1 px
  right;
- nothing reached our `dx` from real horizontal motion → the cursor could
  only drift left.

That is the layout `[report id, buttons, dx, dy]`: the chip **acknowledges
`SET_PROTOCOL(boot)` but keeps its own report protocol**, which is legal
(a device may accept the request and only apply it if it implements boot
reports).

The iteration-2 log confirmed it byte for byte: **130 raw reports, all
`01 00 XX YY`** — byte 0 constant `01` (report id), byte 1 zero (buttons;
no click in that capture), bytes 2/3 change with the finger **in both
directions** (29 and 64 values ≥ 0x80, i.e. negative as `i8`): signed 8-bit
relative deltas.

The descriptor could not settle the question: the chip **stalls every
request shape** — `0x81` (HID 1.11 7.1.1, standard/interface), `0x80` (Linux
`usb_get_descriptor`, standard/device) and `0xa1` (class/interface, the
previous attempt) all logged `report descriptor of interface 1 via …: Halted`.
So the driver uses the **measured layout for this chip** (`258a:001e`, vendor
and product read from the device descriptor during enumeration) as the second
fallback: `ReportLayout::pinebook_touchpad()` — report id 1, buttons in byte
1 (3 bits), X in byte 2, Y in byte 3, both relative. The descriptor path stays
first for devices that do serve it, and the boot-protocol probe with the
3-byte boot layout stays as the last resort before draining.

### A second bug the log exposed: control and interrupt buffers shared an arena

The 15 s liveness probe (experiment 8, iteration 14) started failing on a
**live** device: `input addr 1 silent and unresponsive, resetting its port`
at 35.7 s, 92.9 s, 126.4 s — a disruptive port reset about once a minute
(the keyboard chip is re-enumerated each time). Root cause: `control_transfer`
and the interrupt endpoints allocated from **one bump arena** that every
control transfer reset to offset 0. The setup packet of a control transfer
could therefore overwrite the buffer of an already-armed interrupt endpoint,
and the controller could DMA a report over the setup packet — a corrupt
SETUP makes the device stall, which the probe reads as "device dead".

Fix in `ohci.rs`: interrupt endpoints get **fixed slots** (`endpoint_buf`,
indexed by ED, stride 32 bytes, 1 KiB) that re-enumeration reuses, and control
transfers use their own region above them (`alloc_buf`, 1 KiB, reset per
transfer). The probe failure now logs its error (`liveness probe of addr N
failed: …`) so a future repeat is attributable.

Also fixed earlier in the same iteration: the revive path replaced only the
keyboard `HidDevice` after a port reset, leaving the touchpad entry pointing
at an ED that the new enumeration had rewritten (two entries polling one ED
is a wedge candidate — likely the iteration-1 freeze). A revive now replaces
**every** root-port endpoint of that controller.

Diagnostics kept in the image (cheap, bounded): the descriptor request result
is logged per request shape, and the first 40 reports of non-keyboard
endpoints are logged as `ohcid: raw ep 0x82 len N: …` — enough to verify or
correct the layout from the serial log alone.

Rig smoke (both images): boot + keyboard reports unchanged; the
two-interface touchpad path is not reproducible in QEMU.

Hardware run (2026-11-21, iterations after the image `98918225…` line):
the touchpad moves the pointer in the desktop with the measured layout
(`[report id 1, buttons, dx, dy]`, signed 8-bit relative); the ~50 s port
resets are gone after the DMA arena split (control vs interrupt buffers), and
the liveness heartbeat stopped reading healthy devices as dead. Cursor axes
and buttons behave as expected; the absolute-mode gesture collection of the
chip (report id `0x1e`) is ignored. Keyboard and touchpad verified together
with the desktop and 6-CPU SMP (experiment 14).

## Open questions

- The chip stalls the report descriptor on all three request shapes. If a
  future firmware honours one, the descriptor path takes over automatically.
- `orbital` on RGB565: `vesad` exports the framebuffer format from
  `FRAMEBUFFER_BPP`; colour mapping on the panel matches the VT, but fine
  gradients in the desktop theme have not been checked.
