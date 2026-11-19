# Experiment 0 — Serial console and current boot chain

**Status:** pending
**Related:** README §3 (boot chain), §7 risk 5

## Goal

Prove that the UART2 console works at **1500000 baud 8N1** and capture the complete
current boot chain of the machine as shipped (BootROM → SPL → U-Boot → Linux).

## Prerequisites

- USB-UART adapter with **3.3 V** logic level (check the jumper/voltage; early Pine
  Store cables used 5 V levels and must not be used).
- Bottom cover removed; UART/Audio switch (item 9 in the PINE64 wiki "Internal Layout"
  diagram) slid **towards the display hinges** (ON). Audio output over the headphone
  jack is disabled in this mode.
- Wiring, from the Pinebook Pro point of view: **tip = TX, ring 1 = RX, sleeve = GND**.
  Connect: adapter RX ↔ tip, adapter TX ↔ ring 1.

## Actions

1. Find the adapter on the host.

   macOS:

   ```sh
   ls /dev/tty.usbserial* /dev/tty.wchusbserial* 2>/dev/null
   ```

   Linux:

   ```sh
   ls /dev/ttyUSB* /dev/ttyACM* 2>/dev/null
   ```

2. Open the console at 1500000 8N1.

   macOS / Linux:

   ```sh
   screen /dev/tty.usbserial-XXXX 1500000
   # exit screen: Ctrl-A, then K
   # alternative: picocom -b 1500000 /dev/ttyUSB0
   ```

3. Power on the laptop and record **everything** from power-on until the Linux login
   prompt (or ~30 seconds if no prompt appears). Paste it into Results → Boot log.

4. Optional: if an autoboot countdown appears, press a key repeatedly (space/Enter) to
   stop it. If you get the `=>` U-Boot prompt, run and capture:

   ```
   version
   printenv boot_targets
   printenv bootcmd
   mmc list
   part list mmc 0
   part list mmc 1
   ```

If the output is garbled, the adapter most likely cannot sustain 1500000 baud reliably
(prefer CP2102 / FT232 / FT2232). Note it down and retry with another adapter.

## Results

### Boot log

```
PASTE FULL BOOT LOG HERE
```

### U-Boot prompt session

```
PASTE U-BOOT PROMPT SESSION HERE (or write "no prompt")
```

### Observations

| Question                                                  | Answer    |
| --------------------------------------------------------- | --------- |
| SPL/TPL messages visible (DDR init)?                      | `FILL IN` |
| U-Boot version string (exact)                             | `FILL IN` |
| Board/defconfig in the U-Boot banner                      | `FILL IN` |
| Boot device used by U-Boot (`mmc0` / `mmc1` / `nvme` / …) | `FILL IN` |
| Autoboot interruptible? (`yes` / `no`)                    | `FILL IN` |
| Anything unusual (warnings, resets, garbled characters)   | `FILL IN` |

### Verdict

`FILL IN` — e.g. "UART works at 1500000; current chain is eMMC → U-Boot → Linux;
autoboot not interruptible."
