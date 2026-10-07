# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- **applewin: Mockingboard/Phasor/SAM audio is now mixed with the speaker
  instead of appended after it.** Both sources were pushed into the audio ring
  buffer back-to-back, so with a sound card installed each frame queued ~2×
  real time: latency climbed to the 2 s buffer cap and samples were dropped.
  Card output is now summed sample-by-sample into the speaker frame (and two
  cards are summed with each other) before a single push. Cards also render
  the emulated cycles actually run this frame rather than the fixed
  `cycles_per_frame()`, so they stay in step with the speaker and fall silent
  while the debugger is paused.

- **apple2-core: WOZ disks no longer spin ~1.75× too slowly.** `advance_bits`
  reset the drive clock to the poll cycle, discarding the sub-bit remainder on
  every read; a `LDA $C08C,X / BPL` loop polling every ~7 cycles advanced 1 bit
  instead of 1.75. The remainder now carries into the next poll.

- **apple2-core: Mockingboard, Phasor, MegaAudio and SD Music are reachable at
  their real addresses.** The VIAs were wired to the slot I/O range
  `$C0n0–$C0nF` with only 8 registers each, while `$Cnxx` returned
  Mockingboard-D ROM bytes, so standard Mockingboard software (which talks to
  `$Cn00`/`$Cn80`) produced no sound, and IER/IFR/ACR were unreachable. VIA 1 is
  now at `$Cn00–$Cn0F` and VIA 2 at `$Cn80–$Cn8F` (A4–A6 undecoded, as on the
  real card), with all 16 registers. SD Music keeps its control registers at
  `$CnF0–$CnFF`; Phasor keeps its mode switch at `$C0n9`. The unused
  `mb_firmware` module (and its `unsafe` cast) is removed.

- **apple2-core: VIA timer interrupts fire on their own.** Timers only
  advanced when the program touched the card, and the IRQ line was never
  refreshed after periodic card updates, so IRQ-driven music players never
  received timer interrupts. Cards now report their next timer expiry
  (`Card::next_event_cycle`); `Emulator::execute` services it at that cycle and
  refreshes the IRQ line. The IRQ line is also refreshed after every
  `update_all` and after `$Cnxx` accesses to timer cards.

- **apple2-core: 6522 register semantics.** IER writes now set or clear the
  selected bits according to bit 7 (and IER reads return bit 7 set); reading
  T1C-L / T2C-L acknowledges the T1 / T2 interrupt; register `$F` aliases ORA;
  timer counters no longer underflow (a debug-build panic) one cycle before
  expiry; long idle gaps saturate instead of truncating.

- **apple2-core: SSI263 speech timing.** `MockingboardCard::update` passed the
  absolute cycle count to `Ssi263::tick`, which expects elapsed cycles, so every
  phoneme finished at the next update. The card now tracks elapsed cycles
  (synchronising before each phoneme write) and schedules the READY interrupt.

- **applewin: Ctrl+C and Ctrl+X reach the emulated machine again.** Since the
  egui 0.30 upgrade, egui-winit turns Ctrl/Cmd+C and Ctrl/Cmd+X into
  `Event::Copy`/`Event::Cut` and drops the key press, so BASIC/monitor break
  (Ctrl-C) and cancel-line (Ctrl-X) were lost. Those events now send `$03` /
  `$18`, except while the debugger or an egui text field has the keyboard.

- **apple2-core: loading a save state keeps //c VBL interrupts.** Restore
  re-seeded the VBL schedule via `reset_vbl`, which also cleared the VBL
  interrupt enable and latched flag (neither was saved), so programs that had
  enabled VBL interrupts stopped receiving them. `BusSnapshot` now stores both
  (`#[serde(default)]`, so older save states still load) and restore only
  re-seeds the schedule (`Bus::reseed_vbl_schedule`).

- **apple2-core: free-running VIA T1 period is latch + 2 cycles.** After each
  expiry the counter was reloaded with the cycles left *until* the next expiry,
  which `tick` then treated as a count that expires 2 cycles later, so every
  period ran 2 cycles long (latch + 4).

- **apple2-video: hi-res and double hi-res NTSC frames are fully drawn.** The
  last odd framebuffer row of hi-res (row 383, or 319 in mixed mode) was never
  written, and in TV mode the -2 pixel composite offset left the 2 right-hand
  columns unwritten; both showed stale pixels from the previous mode. The
  render-coverage test now checks every row in NTSC, idealized and TV modes.

- **applewin: debugger step-over and step-out work.** The handlers set
  `step_over_target`/`step_out_sp` and then called `deactivate()`, which
  cleared them, and nothing checked them during execution — both just resumed
  the program. `DebuggerState::begin_step_over`/`begin_step_out` now arm the
  target after resuming, and the GUI runs `Emulator::execute_until` (full
  card/VBL/IRQ servicing) until the JSR's return address is reached or an
  RTS/RTI pops above the starting stack level. Full-speed disk mode keeps
  working during a step (it runs through the same hook).

- **applewin: debugger breakpoints and trace work.** `BP <addr>` and
  `BPM <addr> [R|W]` added breakpoints that nothing ever checked, and `T <n>`
  set a counter nothing read. `DebuggerState::after_instruction` now runs after
  every instruction whenever a breakpoint is enabled or a trace/step is active
  (otherwise the emulator keeps its unhooked fast path): PC breakpoints halt
  before the instruction at their address, memory breakpoints halt after the
  instruction that read/wrote the address (via the bus `mem_trace` log, which
  is only enabled while a memory breakpoint exists; opcode fetches count as
  reads), and `T <n>` records `n` instructions in the trace buffer and halts.
  A stop opens the debugger view.

- **apple2-core: save states from another machine model are refused.**
  `Emulator::restore_snapshot` set `emu.model` from the file but kept the
  running ROM and bus wiring, so loading e.g. a //c state on a IIe built a
  hybrid machine. It now returns `Error::ModelMismatch` and leaves the machine
  untouched; F11 load shows the reason in the status bar, and the
  restore-on-launch state is skipped.

### Performance

- **applewin: the paused debugger no longer redraws at full frame rate.**
  Continuous repaint was gated on `AppMode::Stepping`, which is never set, and
  was forced on whenever the debugger was shown. It now stops while the
  debugger has halted execution; input still triggers repaints.

- **applewin: framebuffer → texture conversion ~12× cheaper.** The texture is
  built with `ColorImage::from_rgba_premultiplied` (all pixels are opaque, so
  the result is identical), skipping epaint's per-pixel unmultiply: ~10 µs vs
  ~126 µs per frame.

- **applewin: lock-free audio output.** The `Arc<Mutex<VecDeque>>` ring is
  replaced by an `rtrb` single-producer/single-consumer ring buffer, so the
  cpal realtime callback never blocks on the GUI thread; it reads samples in
  chunks. Discarding queued audio (after full-speed bursts, resets and machine
  changes) is a flush request the callback honours. WAV recording now captures
  exactly the samples queued to the device. The per-frame Mockingboard sample
  `Vec` is replaced by a reused scratch buffer.

- **applewin: no audio over-production above 1× speed.** Speaker samples are
  sized by `CPU_HZ × speed / sample_rate`, so faster emulation packs more
  cycles into each sample instead of generating 40× real time at 40× speed and
  dropping the excess at the buffer cap. Card output is resampled to the
  speaker frame length (`apple2_audio::mix::mix_resampled_into`). Synthesis is
  skipped while the ring is full. Output at 1× is unchanged.

- **applewin: static screens skip rendering and texture upload.** A frame
  cache keyed on the memory mode, flash phase and every video setting, plus a
  snapshot of the displayed text/hi-res pages (main and aux), skips
  `render_apple2`, `ColorImage` construction and the GPU upload when nothing
  changed. The debugger view and the IIgs always render.

- **apple2-core: WOZ bit reads resolve the track once per latch read** instead
  of per bit, and wrap the bit position with a compare instead of two `%`
  divisions.

- **apple2-core: paging switches only rebuild the page tables when the mode
  changes.** `$C000–$C00B` writes and language-card `$C080–$C08F` accesses
  previously rebuilt all 512 entries unconditionally; ProDOS re-selects the
  same LC state on every MLI call.

- **apple2-core: less duplicate disk-image memory.** The raw image copy is kept
  only for DOS 3.3 / ProDOS order images (the formats written back from it);
  WOZ, NIB and 13-sector images no longer hold a second copy. WOZ tracks
  without weak-bit areas no longer allocate a track-sized zeroed weak mask.

### Removed

- **apple2-debugger: never-compiled modules.** `assembler.rs`, `markup.rs` and
  `watch.rs` were not declared in `lib.rs`, so they never built and their tests
  never ran (`watch.rs` also duplicated the live `watches.rs`). They remain in
  git history if a mini-assembler or data markup is wired up later.
- **apple2-core: unused `AppMode::Stepping` and `AppMode::Benchmark`** (never
  set; the debugger tracks halting itself).
- **All crates: the unused `tracing` dependency.**
- **roms/Mockingboard-D.rom**, no longer referenced since the Mockingboard
  family has no slot ROM.

### Changed

- **Docs:** README version badge, //c ROM 4, IIgs ROM loading, test command
  and per-crate counts, Ctrl+C behaviour; CHANGELOG versions reordered newest
  first.

## [1.1.6] - 2026-07-21

### Changed

- **applewin: switched the embedded Apple //c ROM to version 4 (341-0445-B) for
  maximum software compatibility.** Replaces the earlier "3.5 ROM" (version 0,
  342-0033-A). ROM 4 is the last and most compatible 32KB //c firmware, adding
  Memory Expansion Card support and fixing the mouse-interrupt and disk-firmware
  bugs of the earlier ROMs. The //c boot-regression tests now run against ROM 4
  (title banner, DOS 3.3, ProDOS, and VBL all verified booting).

- **applewin: fixed the `float_literal_f32_fallback` lint** in the GUI status
  bar, toolbar, and widget stroke calls (`Stroke::new(1.0, …)` → `1.0_f32`),
  which had become a hard `-D warnings` error under newer rustc.

- **applewin: upgraded `eframe`/`egui` 0.23 → 0.30 and `rfd` 0.12 → 0.15.**
  Eliminates the `block v0.1.6` future-incompatibility warning (uninhabited
  static, [rust#74840](https://github.com/rust-lang/rust/issues/74840)), which
  was pulled in transitively on macOS via the old `cocoa`/`objc-foundation`
  stack. eframe 0.30 uses `winit` 0.30 + `objc2`, dropping `cocoa`, `objc`,
  and `block` entirely. Migration work: window setup moved from the removed
  `NativeOptions` fields to `egui::ViewportBuilder`; `Frame::close`/
  `set_fullscreen`/`set_window_size`/`info().window_info` replaced with
  `ctx.send_viewport_cmd(...)` and `ctx.input(|i| i.viewport())`; the app
  creator closure now returns `Result`; `ComboBox::from_id_source` renamed to
  `from_id_salt`; `egui::style::Margin` → `egui::Margin`. No behavior change.
  (Target 0.30 was chosen deliberately: it is the newest release that drops
  `block` while staying below egui's 0.31 `Margin`/`StrokeKind` and 0.34
  `App::ui`/`Panel` rewrites, keeping the migration minimal and low-risk.)
- **applewin: split the 4,400-line `main.rs` into a `gui/` module tree**
  (`emulation`, `audio`, `input`, `render`, `panels`, `settings`, `widgets`).
  Pure code motion — the eframe `update()` loop now delegates to named
  per-section `EmulatorApp` methods called in the same order as before, and
  the 18 `act_*` deferred-action locals became a `DeferredActions` struct.
  No behavior change. The headless build is also warning-free now
  (GUI-only items in `main.rs` are `#[cfg(feature = "gui")]`-gated).
- **apple2-core: moved the $C000–$C0FF soft-switch dispatch out of `bus.rs`**
  into `bus/soft_switches.rs` (`bus.rs` → `bus/mod.rs`, 1,481 → 994 lines).
  Pure code motion; covered by the //c boot-trace and bus unit tests.
- **applewin: decomposed the `gui/` tree further for maintainability.** The
  1,235-line `panels.rs` was split by responsibility into `menu`, `toolbar`,
  `statusbar`, `screen`, `debugger_panel`, and `dialogs`; the 500-line
  `settings.rs` tab `match` became per-tab `render_*_tab` methods in a new
  `settings_tabs` module; viewport/window helpers moved to `window.rs`; and
  the joystick/paddle/mouse polling moved to `joystick.rs`, hoisting the
  byte-identical keypad-arrows and keypad-numeric handling (previously
  duplicated between joystick 0 and 1) into shared methods and promoting the
  paddle-trim clamp to a free function. No module now exceeds ~505 lines.
  Pure code motion / identical-code deduplication — no behavior change.

### Performance

- **Release builds now use fat LTO and `codegen-units = 1`** for better
  cross-crate inlining in the CPU/bus/video hot paths.
- **Added `#[inline]` to small hot-path helpers**: `Bus::flag_byte`,
  `Bus::update_irq_line`, `Bus::process_card_dma`,
  `CardManager::any_irq_active` (apple2-core); `text_row_offset`,
  `hgr_row_offset` (apple2-video).
- **applewin: removed per-frame allocations** — the recent-disk/HDD menu lists
  are no longer cloned every frame the File menu is open, and the debugger
  command input is passed by reference instead of cloned per frame.
- **apple2-audio: removed the SSI-263 no-op render loop** (the stub summed 0.0
  over the whole output buffer). **apple2-iigs: preallocated the Mega II
  speaker-toggle buffer** to match apple2-core's bus.

### Fixed

- **Speaker: PWM sound effects rendered as a loud screech (no sub-sample
  averaging).** The GUI speaker synthesis emitted each output sample as a flat
  ±0.5 from the cone state *after* the last toggle in that sample period. Games
  that drive the speaker faster than the output sample rate — PWM /
  duty-cycle-modulated audio, e.g. *Airheart*'s start sound, measured toggling
  every 4–22 CPU cycles against ~23.2 cycles per 44.1 kHz sample — had their
  ultrasonic carrier aliased straight into the audible band: narrow pulses
  either vanished or blew up into full-amplitude samples, heard as a harsh
  screech. Each sample is now the time-weighted average of the speaker level
  across every toggle inside its period (duty-cycle averaging, as in
  `apple2-audio`'s `Speaker::render`), which reconstructs the intended sound
  envelope; normal square-wave beeps are unaffected. Also fixed the
  cycles-per-sample constant (was `floor()`ed, over-producing samples by ~0.9 %
  so the audio ring buffer slowly filled to its 2-second cap — growing latency,
  then steady sample drops), and speaker-state parity is now preserved for
  toggles that fall outside the rendered sample grid.
- **Apple //c: screeching sound on startup in some games (wrong VBL semantics).**
  `$C019` was implemented with Apple IIe semantics for every model: a live,
  active-low VBL signal (bit 7 = 1 during the visible scan lines). On the //c,
  `$C019` is instead a *latched* VBL interrupt-pending flag — set at the start of
  each vertical blanking period and held until acknowledged by an access to
  `$C070` — and `$C05A`/`$C05B` are the DISVBL/ENVBL interrupt masks (not
  annunciator 1). //c-aware games frame-sync their sound and music loops on this
  flag (`LDA $C019` / `BPL` poll, then a `$C070` ack); with the IIe behaviour the
  poll saw bit 7 set ~73 % of the time, so the once-per-frame wait fell through
  almost instantly and speaker routines free-ran at kHz rates — heard as a
  screech during startup music and sound effects. The bus now latches the //c
  VBL flag at each blanking boundary from the emulator execute loop, clears it on
  any `$C070` access, treats `$C05A`/`$C05B` as DISVBL/ENVBL on the //c, and
  raises the CPU IRQ line while the flag is pending with ENVBL set (the flag
  itself latches regardless of the mask, matching MAME). IIe/IIe-Enhanced
  `$C019` behaviour is unchanged, and the VBL schedule is re-seeded on reset and
  snapshot restore so it keeps firing after full-speed disk bursts and state
  loads.

## [1.1.5] - 2026-07-02

### Fixed

- **Apple //c: ProDOS disks would not boot (hung on a blank screen).** The //c
  forces its internal ROM, whose disk firmware drives the port as an IWM. ProDOS
  turns the drive motor off and then polls the IWM status register (`$C0EE` read
  with Q6 high) waiting for the enable bit (bit 5) to clear. Our discrete Disk II
  model kept returning the stale data latch (bit 5 set) throughout the ~1 s
  spin-down grace period, so the poll (`AND #$20` / `BNE`) looped forever and
  ProDOS never booted on the //c (it booted fine on the //e, which uses the card's
  own boot ROM). The IWM status read now reflects the enable latch — which clears
  the instant the motor is switched off — so `$C0EE` returns bit 5 clear while the
  motor is off, regardless of physical spin-down. This behaviour is scoped to the
  //c's IWM controller; discrete Disk II controllers (//e, II+, II) continue to
  hold the data latch during spin-down so a write-protect sense is never misread
  as "writable". DOS 3.3 and ProDOS now both boot on the //c and //e.
- **Apple //c: double hi-res (DHGR) never engaged, garbling //c DHGR software.**
  On the //c the `$C05E/$C05F` (AN3) soft switches were gated behind `IOUDIS`, so
  double hi-res only turned on if a program first set `IOUDIS` via `$C07E`. Real
  //c software (e.g. Broderbund's *Airheart*) enables DHGR with a bare `$C05E`
  and never touches `IOUDIS`, so on the //c the game's double-hi-res graphics
  were rendered as single hi-res (wrong colours / garbled). Per the Apple //c
  Technical Reference, AN3 controls double hi-res **independently of IOUDIS**, so
  `$C05E/$C05F` now toggle DHIRES on the //c exactly as on the //e. Airheart now
  renders correctly on the //c.

### Changed

- **Apple //c now uses the 32KB "3.5 ROM" (ROM version 0, 342-0033-A).** The //c
  model previously embedded ROM version 4 (341-0445-B). The application now
  embeds the earlier 3.5 ROM — the first 32KB //c firmware, adding UniDisk 3.5
  support, the Mini-Assembler, and the self-test diagnostic. It is a genuine
  dual-bank ROM (lower 16KB standard bank, upper 16KB alternate bank via the
  `$C028` ROM switch). Verified booting the DOS 3.3 System Master to the
  Applesoft/Integer BASIC greeting in //c mode; covered by new regression tests
  in `crates/apple2-core/tests/iic_boot_trace.rs`.

### Fixed

- **Double Hi-Res (DHGR) colours were completely wrong.** The 16-colour DHGR
  palette was indexed directly with the raw 4-bit value, but DHGR nibbles must
  first be remapped via AppleWin's `DoubleHiresPalIndex` — a rotate-left-by-1 of
  the nibble. Every colour was off (blue↔violet/magenta, orange↔green), which is
  why DHGR games such as Broderbund's *Airheart* showed the wrong colours. Fixed in
  both the NTSC and RGB renderers, verified against `RGBMonitor.cpp`'s
  `UpdateDHiResCell` (`color = ((bits&7)<<1)|((bits&8)>>3)`).
- **16-colour palette did not match AppleWin.** The lo-res/DHGR palette used
  hand-tweaked RGB values that differed substantially from AppleWin's, so even with
  correct nibble mapping the hues were off. Replaced both `LORES_PALETTE` (NTSC) and
  `RGB_PALETTE` with AppleWin's exact "lores & dhires" table from `RGBMonitor.cpp`
  (the Linards-tweaked `VideoInitializeOriginal` values). Notably AppleWin's DHGR
  *blue* is `0x0D,0xA1,0xFF` (a bright sky-blue), which is the correct *Airheart*
  sky colour — previously rendered as a pure, too-dark blue.
- **Default DHGR now uses AppleWin's faithful NTSC composite path.** The default
  (idealized) DHGR mode previously used a custom palette/blur renderer that either
  looked blocky or introduced colour grain. It now uses `render_dhires_ntsc`, whose
  colour tables are generated exactly like AppleWin's `initChromaPhaseTables()`
  (filtered `y1` luminance + filtered I/Q chrominance). High-frequency detail (text,
  stars) resolves to clean white and solid areas stay smooth and grain-free — the
  same algorithm AppleWin uses. The invented `render_dhires_smoothed` was removed.
  The composite DHGR path now scanline-doubles (duplicates each scanline) rather than
  vertically blending adjacent scanlines, keeping text and detail vertically crisp.

### Added

- **Idealized hi-res renderer was broken.** `render_hires_idealized`
  (`VT_COLOR_IDEALIZED`) had multiple bugs: an out-of-bounds black/white palette
  index (`bw[0|128]`), rendered 14 pixels per byte instead of 7 (overrunning the
  framebuffer), used the wrong shift cadence, and mapped the hi-bit-clear artifact
  colours incorrectly (green/orange/blue instead of violet/green/orange). Rewritten
  to faithfully match `RGBMonitor.cpp`'s `UpdateHiResRGBCell`, producing clean,
  saturated artifact colours.

### Changed

- **Default video mode is now "Color (Composite Idealized)".** Produces sharp,
  saturated colours (the clean artifact-colour look) instead of the softer/fringier
  NTSC composite emulation. Users can still select "Color TV" or any other mode in
  Settings.

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

## [1.1.3] - 2026-05-08

### Fixed

- **GitHub Actions: Publish macOS DMG to Releases.** Added a `publish-dmg-macos` job
  to the CI workflow that signs the app with `codesign` and packages a notarized
  `.dmg` into GitHub Releases artifacts.

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
