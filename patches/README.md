# Upstream changes

We do not fork the Redox repositories. Everything this port adds or changes is
kept here and injected into the build tree
(`build/redox/recipes/core/<repo>/source`) by `scripts/apply-patches.sh`.

## Layout

One directory per recipe, mirroring the upstream tree:

```
patches/
  kernel/       redox-os/kernel
  bootloader/   redox-os/bootloader
  relibc/       redox-os/relibc
  base/         redox-os/base
  debug/        optional instrumentation, NOT applied (see debug/README.md)
```

Two kinds of entries, distinguished by the file name:

| Entry                  | Meaning                                                                        |
| ---------------------- | ------------------------------------------------------------------------------ |
| `<path>` (no `.patch`) | **new file** — copied verbatim, parent directories created                     |
| `<path>.patch`         | **diff** of the upstream `<path>`, applied with `git apply` from the repo root |

There is exactly one patch per modified file, so a hunk can be inspected,
reviewed or dropped on its own. Each directory has a `README.md` with the base
commit, the list of files, and the reason for every change; that `README.md` is
documentation of the set and is never copied into the checkout.

New files are really new code (drivers, services, `debugsh`); upstream has no
counterpart, so there is nothing to diff them against.

## Applying

```sh
scripts/apply-patches.sh              # every known recipe
scripts/apply-patches.sh kernel base  # a subset
scripts/apply-patches.sh --check      # dry run, verifies every diff applies
scripts/apply-patches.sh --list       # show what belongs to each recipe
```

The script is idempotent: identical files are skipped and an already-applied
diff is reported as `up to date` instead of failing. A diff that neither
applies nor reverse-applies aborts with the checkout's current commit, which is
the signal that the base commit moved and the patch needs regenerating.

`patches/debug/` is not a recipe, so the script leaves it alone; those patches
are applied by hand when a bring-up needs the UART markers again.

## Index

| Directory     | Upstream              | Base commit | Contents                                                                                                                                                        |
| ------------- | --------------------- | ----------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `kernel/`     | `redox-os/kernel`     | `719e8f5c`  | GICv3 (root IRQ, SPI routing, redistributor/PPIs), NS16550 rx, inherited `interrupt-parent`, RGB565 framebuffer, signal delivery order — see `kernel/README.md` |
| `bootloader/` | `redox-os/bootloader` | `f3d9261`   | `FRAMEBUFFER_BPP` from the GOP pixel format, non-blocking boot menu — see `bootloader/README.md`                                                                |
| `relibc/`     | `redox-os/relibc`     | `b83feabc`  | thread-atexit registration vs. finalization — see `relibc/README.md`                                                                                            |
| `base/`       | `redox-os/base`       | `8f576d27`  | `debugsh`, `dwmmcd`, `ohcid`, `vesad` RGB565 output, initfs services — see `base/README.md`                                                                     |

## Regenerating a patch

After editing a checkout in the build tree, regenerate the affected entry from
it (never hand-edit a diff):

```sh
repo=kernel; file=src/context/signal.rs
git -C build/redox/recipes/core/$repo/source --no-pager diff -- "$file" \
    > patches/$repo/$file.patch
```

For a new file, copy it (keeping the tree):

```sh
install -D build/redox/recipes/core/$repo/source/$file patches/$repo/$file
```

Then `scripts/apply-patches.sh --check` on a pristine checkout of the base
commit must report every entry as applyable.
