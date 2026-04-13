# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Apple IIgs emulation** (new `apple2-iigs` crate): 65C816 CPU with all 256
  opcodes (emulation + native mode, 8/16-bit registers, 24-bit addressing),
  IIgs memory bus (256KB-8MB RAM, 128-256KB ROM), Mega II IIe compatibility,
  Super Hi-Res video (320x200/640x200), Ensoniq DOC 5503 wavetable audio
  (32 oscillators, 64KB sound RAM), ADB keyboard/mouse, BRAM with factory
  defaults, SmartPort disk I/O, speed control (1/2.8 MHz), shadow register.
  ROM 00/01/03 auto-detected from `roms/Apple_IIgs/`. GUI integration with
  machine type dropdown, IIgs settings (RAM size, ROM path), SHR rendering,
  Ensoniq audio, debugger support. 136 new tests (89 CPU + 32 peripheral +
  15 integration).

- **Apple IIc emulation:** Full Apple //c model support with 32KB ROM (v04,
  341-0445-B) embedded at compile time, built-in peripherals (Super Serial Card
  in slots 1 & 2, 80-column text in slot 3, Mouse in slot 4, Disk II in slot 6),
  128KB RAM, forced 65C02 CPU, ROM bank switching via $C028 (MF_ALTROM0), IOUDIS
  gating of DHIRES, and locked slot/CPU UI. Selectable from the Machine Type
  dropdown in Settings.

- **Apple IIc unit tests (12 new):** Bus tests for INTCXROM enforcement, ROM bank
  switching via $C028, IOUDIS gating of DHIRES, soft-switch no-ops on IIc, and
  IIe regression guards. Integration tests for IIc boot, reset persistence, and
  32KB ROM execution.

- **Disk II IWM compatibility tests (5 new):** Tests for Q7H write latch storage,
  IWM handshake echo, idle ready status, spinning latch return, and handshake vs
  idle fix interaction.

- **Via6522 unit tests (18 new):** Register read/write roundtrip, T1/T2 timer
  arming and expiry (one-shot and continuous modes), timer decrement without
  expiry, IFR write-to-clear behavior, IFR bit 7 composite flag, IRQ active
  detection, save/load state serialization roundtrip, T1LL latch-only write,
  unknown register return value, and 4-bit register address masking.

### Fixed

- **Apple IIc boot ROM garbled screen:** Fixed three issues that prevented the
  IIc boot ROM from executing correctly:
  - **ROM bank mapping:** The 32KB ROM bank offsets were inverted — the standard
    bank (lower 16K) was mapped as alternate and vice versa, causing the CPU to
    read from the empty upper bank.
  - **Padded ROM mirroring:** 16K ROMs padded to 32K now mirror the lower bank
    to the upper bank, so the $C028 ROM bank switch doesn't jump into zeros.
  - **IWM disk controller compatibility:** The Disk2Card now handles two IWM-specific
    polling loops in the IIc boot ROM: (1) the handshake loop at $CC29 that writes
    to Q7H and expects the value echoed back via Q7L, and (2) the ready loop at
    $CC3F that checks Q7L bit 5 for controller busy status. Without these fixes the
    CPU would loop indefinitely during boot.

### Removed

- **Debug CPU trace logging:** Removed diagnostic cpu_trace.log instrumentation
  from the emulator execute loop (caller trace, periodic PC logger, and memory
  dump code). These were temporary debugging aids used during the v1.1.0 Disk II
  and language card fixes.

### Changed

- **Refactor: Extract shared Via6522 module.** The 6522 VIA chip emulation
  (struct, timers, register read/write, state serialization) was duplicated
  identically across Mockingboard, Phasor, MegaAudio, and SD Music cards.
  Extracted into `cards/via6522.rs` (~680 lines of duplication removed). The
  shared Mockingboard firmware ROM was also extracted into `cards/mb_firmware.rs`
  (previously duplicated in 3 card files).
- `Bus::new()` now accepts an `Apple2Model` parameter for model-aware memory
  initialization and soft-switch behavior.
- Custom ROM loading accepts 32KB ROMs in addition to 12KB and 16KB.

## [1.1.0] - 2026-04-13

### Fixed

- **Language card RAM routing (critical):** Fixed a fundamental memory architecture
  bug where language card RAM ($D000-$FFFF) was always stored in auxiliary RAM
  regardless of the ALTZP soft-switch state. The C++ AppleWin correctly routes LC
  RAM through main RAM when ALTZP=0 and auxiliary RAM when ALTZP=1, giving each
  bank independent storage. The Rust port shared a single storage area for both
  banks, causing auxiliary memory writes to silently corrupt game code loaded into
  the main bank's language card area. This fix resolves hangs and infinite loops
  in software that uses both language card RAM and auxiliary memory, including
  Ultima V and other ProDOS-based titles.

- **Disk II: Odd-address return value:** Soft-switch reads at odd addresses
  ($C0E1, $C0E3, ..., $C0ED, $C0EF) now return 0 (floating bus approximation)
  instead of the data latch, matching the C++ AppleWin `MemReadFloatingBus()`
  behavior and UTAIIe Table 9.1.

- **Disk II: Spinning/spin-down delay:** Added a 1-second (~1M cycle) spin-down
  timer after motor-off, matching C++ `SPINNING_CYCLES`. Reads and writes are now
  only serviced while the drive is spinning. "DRIVES OFF forces the data register
  to hold its present state" (UTAIIe p9-12).

- **Disk II: LoadWriteProtect spinning guard:** The $C0xD (Q6H) write-protect
  check now respects the spinning state and will not update the data latch if the
  drive has stopped, matching C++ `LoadWriteProtect()` (GH#599).

- **Disk II: Motor-off clears magnet states:** Turning the motor off now clears
  the stepper magnet states (`phases = 0`), matching the C++ behavior described
  in UTAIIe p9-12 (GH#926, GH#1315).

- **Disk II: Drive select stops other drive:** Selecting drive 0 or 1 now
  immediately stops the other drive's spinning counter, matching C++ `Enable()`.

- **Disk II: Stepper ignores motor-off:** Phase changes are now ignored when
  the motor is off and the drive is not spinning, matching C++ `ControlStepper()`
  (GH#525).

### Added

- **Disk II: WOZ headWindow/latchDelay/MC3470 model:** Replaced the simplified
  WOZ shift register with the full C++ Logic State Sequencer model, including a
  4-bit head window tracking the last 4 raw magnetic flux transitions, MC3470
  output bit calculation with ~30% random bit generation for zero runs, and a
  latch delay mechanism (7 us hold after valid nibble with extension on zero
  shift register).

- **Disk II: WOZ even-address bit-stream advance:** Any even-address read now
  triggers the WOZ LSS to advance the bit stream, not just register $C0xC (Q6L).
  This matches the C++ `DataLatchReadWriteWOZ()` call at the bottom of `IORead`
  for all even addresses.

- **Unit tests:** Added 4 new bus tests for ALTZP-aware language card RAM
  routing: `lc_altzp_off_routes_to_main_ram`, `lc_altzp_on_routes_to_aux_ram`,
  `lc_main_and_aux_banks_are_independent`, and
  `lc_write_through_rom_respects_altzp`.

## [1.0.0] - 2026-03-17

### Added

- Initial release of AppleWin-rs.
- Full emulation of Apple II, II+, IIe, and IIe Enhanced models.
- 21 expansion card implementations.
- Cross-platform GUI with egui/eframe.
- WOZ v1/v2 bit-level disk emulation.
- Symbolic debugger with breakpoints and disassembler.
- Save/restore state, screenshot capture, WAV recording.
- Headless build mode for CI and embedding.
