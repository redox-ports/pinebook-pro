# Experiment 3 — eMMC hardware boot partitions

**Status:** done (2026-11-19) — hypothesis **rejected**: boot0/boot1 contain no
bootloader; the first stage is the idbloader in the eMMC **user area** at sector 64
**Related:** README §1 (boot chain), §3 (image layout), experiment 2 (follow-up)

## Goal

Experiment 2 showed that:

- the eMMC user area at sector 64 contains **no `RKNS` idbloader** (arbitrary data);
- U-Boot proper **is** present in the eMMC user area at sector 16384;
- SPI NOR and microSD contain no bootloader.

Therefore the first stage of the shipped boot chain is expected in an eMMC **hardware
boot partition** (`boot0`/`boot1`). This experiment confirms it, captures the boot
partition configuration, and pins down the complete shipped chain:

```
BootROM -> eMMC boot0/boot1 (idbloader: TPL+SPL) -> eMMC user area 8 MiB (u-boot.itb)
        -> EFI distro boot -> Manjaro on eMMC
```

**Outcome (2026-11-19):** the hypothesis was **rejected** — boot0/boot1 contain no
bootloader and the idbloader is in the **user area at sector 64**, as in the standard
RK3399 layout. See Analysis and Verdict below.

## Prerequisites

- All commands are **read-only**. Never write to `mmcblk2boot0`/`mmcblk2boot1` (or any
  other part of the eMMC): it holds the baseline OS and the boot areas are raw hardware
  partitions.
- The boot partitions are exposed by Linux as `/dev/mmcblk2boot0` and
  `/dev/mmcblk2boot1` (4 MiB each, confirmed in experiment 1).
- Optional: `mmc-utils` for reading EXT_CSD (`sudo pacman -S mmc-utils`).

## Actions

```sh
# 0. Sanity check: boot partitions exist and their size
lsblk -o NAME,SIZE,TYPE /dev/mmcblk2
ls -l /dev/mmcblk2boot0 /dev/mmcblk2boot1

# 1. First 64 KiB of each boot area (look for RKNS / U-Boot / anything)
sudo dd if=/dev/mmcblk2boot0 bs=512 count=128 status=none | hexdump -C | head -30
sudo dd if=/dev/mmcblk2boot1 bs=512 count=128 status=none | hexdump -C | head -30

# 2. Whole-area signature search (4 MiB each)
sudo grep -abo 'RKNS' /dev/mmcblk2boot0 | head
sudo grep -aboi -m5 'u-boot' /dev/mmcblk2boot0
sudo grep -abo 'RKNS' /dev/mmcblk2boot1 | head
sudo grep -aboi -m5 'u-boot' /dev/mmcblk2boot1

# 3. eMMC boot configuration (EXT_CSD) — optional but very useful
which mmc || sudo pacman -S mmc-utils
sudo mmc extcsd read /dev/mmcblk2 | grep -Ei 'PARTITION_CONFIG|BOOT_PARTITION|BOOT_ACK|BOOT_BUS|PARTITIONING|RPMB_SIZE'
```

Expected if the hypothesis is correct: `boot0` (or `boot1`) starts with a Rockchip
idbloader (`RKNS` magic) or otherwise contains bootloader code, and `PARTITION_CONFIG`
shows the boot partition enabled (`BOOT_PARTITION_ENABLE`) with boot acknowledge
settings.

## Results

### Boot partitions present

```
$ lsblk -o NAME,SIZE,TYPE /dev/mmcblk2
NAME          SIZE TYPE
mmcblk2      58,2G disk
├─mmcblk2p1 213,6M part
└─mmcblk2p2    58G part

$ ls -l /dev/mmcblk2boot0 /dev/mmcblk2boot1
brw-rw---- 1 root disk 179, 64 ноя 19 23:08 /dev/mmcblk2boot0
brw-rw---- 1 root disk 179, 96 ноя 19 23:08 /dev/mmcblk2boot1
```

### boot0 first 64 KiB

```
00000000  00 00 00 00 00 00 00 00  00 00 00 00 00 00 00 00  |................|
*
00010000
```

### boot1 first 64 KiB

```
00000000  00 00 00 00 00 00 00 00  00 00 00 00 00 00 00 00  |................|
*
00010000
```

### Signature search

```
# Nothing
```

### EXT_CSD boot configuration

```
# mmc-utils not installed
# No network
```

### Analysis: the idbloader was there all along

The premise "sector 64 contains no idbloader" came from expecting the ASCII magic
`RKNS`. That expectation is wrong for RK3399. In U-Boot `tools/rkcommon.c` (the device
runs U-Boot 2021.07-1):

- the first-stage header (`struct header0_info`) is stored at **SD/MMC block 64** and is
  **RC4-encoded** with a fixed key, so its first 512 bytes look like random data;
- the plaintext header has signature **`0x0ff0aa55`**, not `RKNS`;
- `init_offset = 4` blocks, so the payload starts at byte `0x800` and begins with the
  SoC's 4-byte marker — **`RK33`** for RK3399 (`RK_SPL_HDR_START`, `spl_infos[]`);
- `RKNS` (`0x534e4b52`, `RK_MAGIC_V2`) is the magic of the **v2** header used by newer
  SoCs (RK3568/RK3588 class), not by RK3399.

Decoding the first 16 bytes of the experiment-2 dump with the key from `rkcommon.c`
reproduces the v1 header exactly:

```
cipher: 3b 8c dc fc be 9f 9d 51 eb 30 34 ce 24 51 1f 98
plain:  55 aa f0 0f 00 00 00 00 01 00 00 00 04 00 00 00
        magic=0x0ff0aa55   reserved=0   disable_rc4=1   init_offset=4 (0x800 bytes)
```

So `idbloader.img` **is** at eMMC user-area sector 64, and the shipped chain is the
standard RK3399 one. This also matches the boot log in experiment 0: the TPL runs from
the idbloader, returns to the BootROM (`Trying to boot from BOOTROM / Returning to boot
ROM...`), the BootROM loads the SPL from the same image, and the SPL loads `u-boot.itb`
from sector 16384 (the back-to-BootROM flow is inferred from the log and `rkcommon.c`).
The empty hardware boot partitions are therefore not part of the chain.

### Observations

| Question                                                                    | Answer                                                                                                                                                                                                     |
| --------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Boot area containing the first stage (boot0 / boot1 / neither)              | **Neither.** Both boot areas are all zeros in the first 64 KiB and contain no `RKNS`/`U-Boot` signatures in the full 4 MiB. The first stage is in the **eMMC user area at sector 64**.                     |
| `RKNS` magic present in boot0?                                              | No. Boot0 is erased — and `RKNS` is not the RK3399 magic anyway (see Analysis).                                                                                                                            |
| `RKNS` magic present in boot1?                                              | No. Boot1 is erased.                                                                                                                                                                                       |
| U-Boot strings present in boot0 / boot1?                                    | No.                                                                                                                                                                                                        |
| `PARTITION_CONFIG`: boot partition enable / boot ack                        | **Not read** (`mmc-utils` absent, no network). The result does not depend on it: the boot partitions contain no first stage. `mmc info` (experiment 2) reports 4 MiB boot areas, both not write-protected. |
| Does the shipped chain match the expected path (ROM → boot0 → 8 MiB → EFI)? | **No.** It matches the standard path: ROM → user-area sector 64 (`idbloader`, RC4-encoded v1 header + `RK33` TPL/SPL) → user-area sector 16384 (`u-boot.itb`) → EFI → Manjaro.                             |

### Verdict

Hypothesis **rejected**. The eMMC hardware boot partitions contain no bootloader and are
not part of the shipped boot chain. The first stage is the **idbloader in the eMMC user
area at sector 64**, where the standard RK3399 layout puts it. Experiment 2's "arbitrary
data, no idbloader" was a false negative caused by the wrong expected magic: the on-disk
header is RC4-encoded (`0x0ff0aa55`), and the visible payload marker is `RK33` at
`+0x800`. `RKNS` belongs to the v2 header of newer Rockchip SoCs.

```mermaid
graph LR
    A[BootROM] --> B[eMMC user LBA 64: idbloader v1 header + TPL]
    B -->|back to BootROM| C[BootROM loads SPL from the same image]
    C --> D[eMMC user LBA 16384: u-boot.itb]
    D --> E[EFI distro boot to Manjaro]
```

Consequences:

- **The deliverable is unchanged and now empirically confirmed:** the target SD layout
  (idbloader at sector 64, `u-boot.itb` at 16384) matches the working eMMC install at
  both bootloader offsets.
- **Verification rule:** never identify an RK3399 first stage by grepping for `RKNS`.
  Decode the v1 header or check for `RK33` at LBA 68. A blank sector 64 (`0x00` or
  `0xFF`) is the actual "no idbloader" signal.
- **Safety boundary unchanged:** the eMMC is the baseline OS; never write to the eMMC,
  including its (currently empty) boot partitions.
- `PARTITION_CONFIG` remains unread; it is now a nice-to-have, not a blocker.

### Follow-ups (read-only)

Optional confirmation of the decoded header on the live system:

```sh
# 1. Payload marker: LBA 64 + init_offset(4) = LBA 68; expect "RK33" (52 4b 33 33)
sudo dd if=/dev/mmcblk2 bs=512 skip=68 count=1 status=none | hexdump -C | head -4

# 2. Decode the full v1 header (no dependencies beyond python3)
sudo dd if=/dev/mmcblk2 bs=512 skip=64 count=1 status=none > /tmp/rk-hdr.bin
python3 - <<'PY'
import struct
key = [124, 78, 3, 4, 85, 5, 9, 7, 45, 44, 123, 56, 23, 13, 23, 17]
def rc4(data):
    s = list(range(256)); j = 0
    for i in range(256):
        j = (j + s[i] + key[i % 16]) % 256
        s[i], s[j] = s[j], s[i]
    out = bytearray(); i = j = 0
    for b in data:
        i = (i + 1) % 256
        j = (j + s[i]) % 256
        s[i], s[j] = s[j], s[i]
        out.append(b ^ s[(s[i] + s[j]) % 256])
    return bytes(out)
h = rc4(open('/tmp/rk-hdr.bin', 'rb').read())
print(f"magic=0x{struct.unpack('<I', h[0:4])[0]:08x} "
      f"disable_rc4={struct.unpack('<I', h[8:12])[0]} "
      f"init_offset={struct.unpack('<H', h[12:14])[0]} "
      f"init_size={struct.unpack('<H', h[506:508])[0]} "
      f"init_boot_size={struct.unpack('<H', h[508:510])[0]}")
PY

# 3. Optional: eMMC boot configuration (still not needed for the verdict)
sudo mmc extcsd read /dev/mmcblk2 | grep -Ei 'PARTITION_CONFIG|BOOT_PARTITION|BOOT_ACK|BOOT_BUS|PARTITIONING|RPMB_SIZE'
```

Expected: `RK33` at LBA 68; `magic=0x0ff0aa55`, `disable_rc4=1`, `init_offset=4`.
