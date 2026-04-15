# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **Apple IIgs disabled in Settings UI:** The Apple IIgs option is temporarily
  hidden from the Settings → Machine → Computer type dropdown while IIgs support
  stabilises. The underlying emulation code (`apple2-iigs` crate and all IIgs
  integration in `applewin`) remains intact and can be re-enabled by uncommenting
  `Apple2Model::AppleIIgs` in the machine selector in `crates/applewin/src/main.rs`.

### Added

- **Apple IIgs disk loading (SmartPort):** The GUI now routes IIgs disk image
  loading (drag-and-drop, File→Open, auto-load from config, recent disks) to
  `iigs.bus.smartport` when an Apple IIgs emulator is active. Supports `.2mg`,
  `.2img` (2IMG container), `.po`, and `.hdv` (raw ProDOS-order) formats.
- **SmartPort firmware trap:** Replaces the built-in slot 5 firmware with a
  custom stub that uses the 65C816 `WDM $FE` instruction to dispatch SmartPort
  MLI calls directly to `SmartPort::read_block`/`write_block`/status. Enables
  the IIgs ROM, ProDOS, and GS/OS to access SmartPort disks without needing
  full hardware register emulation. Implements STATUS, READ BLOCK, WRITE BLOCK,
  FORMAT, CONTROL, INIT, OPEN, and CLOSE commands.
- **`Bus816::wdm_trap` trait method:** New bus method with default no-op impl,
  invoked when the 65C816 executes `WDM $xx`. `IIgsBus` overrides it to handle
  the SmartPort trap signature (`$FE`).
- **Apple IIgs SmartPort tests (3 new):** Firmware stub installation, READ BLOCK
  via trap (verifies data transfer to RAM and return-address advancement),
  and NO DEVICE error path.

### Tests

- **Performance regression guards (6 new):** `step_with_table_equivalent_to_step_6502`
  and `step_with_table_equivalent_to_step_65c02` validate that the hoisted
  dispatch path produces identical register / memory / cycle state to the
  original `dispatch::step` for both CPU variants. `speaker_toggles_capped_at_65536`
  asserts the 65 536-entry ceiling on the speaker toggle ring-buffer guard.
  `slot_out_of_range_returns_none`, `slot_mut_out_of_range_returns_none`, and
  `slot_empty_in_range_returns_none` pin down `CardManager::slot` / `slot_mut`
  behaviour after Phase 2.3 replaced `get(slot)?` with an explicit
  `slot < NUM_SLOTS` range check.

### Performance

- **Direct-to-display rendering:** Removed the intermediate `pixel_buf: Vec<u8>`
  on `EmulatorApp` and now source the egui texture, BMP screenshot writer, and
  SHR scaler directly from `Framebuffer::pixels_as_bytes()`. This eliminates a
  ~860 KB `copy_from_slice(fb → pixel_buf)` per frame (~52 MB/s at 60 FPS) and
  drops one allocation from `EmulatorApp::new_inner`.
- **SHR scale lookup tables:** Precomputed `SHR_SRC_X: [u16; 560]` and
  `SHR_SRC_Y: [u16; 384]` at compile time (`const` with a `while`-loop
  initializer) and reference them in `scale_shr_to_framebuffer`. Removes the
  `dst_x * src_w / dst_w` division/modulo from every one of the ~215 K inner-
  loop iterations per SHR frame.
- **Speaker / Ensoniq audio drain into reusable scratch:** Speaker synthesis
  now fills a reusable `speaker_scratch: Vec<f32>` without holding the ring-
  buffer mutex, then bulk-pushes all samples under a single lock. Ensoniq DOC
  path replaces its per-frame `vec![0.0; n]` with a preallocated
  `ensoniq_scratch`. Shrinks the critical section the audio callback thread
  can wait on from ~735 iterations to a tight memcpy-style loop.
- **Card slot dispatch hot-path:** `CardManager::slot` and `slot_mut` are
  `#[inline]` and use an explicit `slot < NUM_SLOTS` range check (replacing
  the `.get(slot)?` pattern) so the bounds check collapses at the call site.
  `Bus::io_read` / `io_write` are `#[inline]` so LLVM can see through to the
  soft-switch dispatch on `$C000–$C0FF`, the hottest address range.
- **CPU dispatch hoisting:** The 6502-vs-65C02 dispatch table is now selected
  once per `Emulator::execute` batch instead of being re-chosen via an
  `is_65c02` branch on every instruction. The hot loop calls a new
  `dispatch::step_with_table()` that takes the pre-resolved `&[OpFn; 256]`.
  `dispatch::step()` is preserved for debugger / single-step callers.
- **Inlined hot 6502 opcode handlers:** Added `#[inline]` to the ~90 most-
  executed opcode handlers (all LDA/STA/LDX/LDY/STX/STY addressing modes,
  immediate ADC/SBC/CMP, all branches, JMP/JSR/RTS, INC/DEC variants,
  register transfers, flag sets/clears, push/pull, BIT, STZ). Leaves the
  256-entry dispatch table small enough to stay I-cache-friendly while
  eliminating prologue/epilogue overhead on the common path.
- **RGB lores / dlores batched pixel writes:** Replaced per-pixel
  `set_pixel()` calls in `render_lores` and `render_dlores` (which each do a
  bounds check per pixel) with `pixels[...].fill(color)` over 14-wide /
  7-wide contiguous spans. Eliminates ~21 K bounds checks per frame in text
  display modes and enables SIMD stores on the target CPU.

- **Phase 1 hot-path allocation fixes:** Eliminated several per-frame heap
  allocations on the rendering and audio paths. The IIgs SHR renderer now reuses
  a single 640×400 `u32` scratch buffer on `EmulatorApp` instead of
  `vec![0u32; 640*400]` every frame (~1 MiB per frame saved at 60 FPS). WAV
  recording reuses a `Vec<f32>` scratch buffer rather than `collect()`ing a
  fresh chunk per frame. Speaker toggle draining now uses `std::mem::swap`
  against a reusable scratch `Vec<u64>` on `EmulatorApp`, preserving the bus's
  preallocated 65 536-entry capacity across frames (previously `std::mem::take`
  dropped that capacity each frame, forcing a regrow-from-zero). Added
  safety caps to `Bus::mem_trace` (1 M entries) and `Bus::speaker_toggles` /
  `Mega2::speaker_toggles` (65 536 entries) to bound worst-case growth. The
  egui repaint request is now skipped when the debugger has halted execution
  (`AppMode::Stepping`) with no dialogs open, letting the UI thread idle on
  input instead of running a 60 Hz redraw loop.

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
