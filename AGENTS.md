# Repository Guidelines

## Project Overview

AppleWin-rs is a Rust rewrite of the C++ [AppleWin](https://github.com/AppleWin/AppleWin) emulator. It covers the Apple II, II+, IIe, IIe Enhanced and //c with cycle-accurate 6502/65C02 emulation, plus experimental Apple IIgs support (65C816). The workspace version is set once in root `Cargo.toml` `[workspace.package]` (currently 1.1.6). Edition 2024, license GPL-2.0-or-later.

Comments and module docs often cite the C++ original, e.g. `source/Memory.h`, `source/Debugger/Debugger_Assembler.cpp`, `g_bFullSpeed`. When behaviour is unclear, the AppleWin C++ source is the reference.

## Required Workflow

**Mandatory after every code change**, in this order. All four must pass with zero warnings before work is complete; CI enforces the same checks on pushes and PRs to `main` and `development`:

1. `cargo fmt --all`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo build --release`
4. `cargo test --workspace --no-default-features --features headless`

- **Changelog**: document every significant fix, feature, refactor or behaviour change in `CHANGELOG.md` (format under Important Files).
- **Plans**: for non-trivial work (3+ steps or architectural decisions), write a checkable plan in `tasks/todo.md`, mark items as you go, and add a review section when done. If the work goes sideways, stop and re-plan.
- **Lessons**: after any user correction, record the pattern and a preventive rule in `tasks/lessons.md`. Review it at session start.
- **Verification**: never mark work done without proving it works: run it, check output, and diff behaviour against `main` when relevant.
- **Principles**: simplicity first, minimal impact, root-cause fixes only (no temporary workarounds).
- **Crate boundaries**: `apple2-core` and the other library crates stay OS-agnostic; platform code (GUI, audio I/O, file dialogs, input) lives only in `crates/applewin`.

## Architecture & Data Flow

Crate dependency graph (`apple2-core` is **not** a leaf; it depends on `apple2-audio`):

```
apple2-audio ◄── apple2-core ◄── apple2-video, apple2-debugger
                      ▲
apple2-iigs ──────────┘ (also depends on audio + video)
applewin ──► all five, plus the optional GUI stack (eframe/egui 0.30, cpal, gilrs, rfd, toml)
```

- **Library crates have no OS dependencies and no Cargo features.** All platform code lives in `crates/applewin` behind `#[cfg(feature = "gui")]`.
- **Emulator (`apple2-core/src/emulator.rs`)**: `Emulator { cpu, bus, model, mode }`.
  - `execute(cycles)` is the hot loop. It picks the dispatch table once, handles //c VBL, WAI and IRQ deferral, and calls `cards.update_all` every 17_030 cycles.
  - `finish_reset` forces `MF_INTCXROM` on the //c.
- **Bus (`apple2-core/src/bus/mod.rs`)**:
  - Holds `main_ram`/`aux_ram` as `Box<[u8; 65536]>`, the ROM `Vec`, `cards: CardManager` and `mode: MemMode` (bitflags, `MF_*`).
  - Memory is routed per 256-byte page through `pages_r: [PageSrc; 256]` and `pages_w: [PageDst; 256]`. `rebuild_page_tables()` must run whenever a mode switch changes.
  - `read`/`write` have soft-switch side effects. `read_raw`/`write_raw` have none and are what the debugger uses.
  - `rom_read` takes a `$C000`-relative offset. The //c's 32K ROM banks through `MF_ALTROM0`.
- **Soft switches (`bus/soft_switches.rs`)** handle `$C000–$C0FF`:
  - `$C080–$C08F` go to the language card (`lc_read`).
  - `$C090–$C0FF` go to `card.slot_io_read(reg & 0xF)` with slot `(reg >> 4) - 8`, followed by `process_card_dma` and `update_irq_line`.
- **CPU (`apple2-core/src/cpu/`)**:
  - Opcodes are dispatched through static tables `DISPATCH_6502`/`DISPATCH_65C02` of `type OpFn = fn(&mut Cpu, &mut Bus) -> u8` (cycles). The tables are `#[rustfmt::skip]`.
  - `step_with_table` must stay equivalent to `step`; tests in `dispatch.rs` enforce this.
- **Cards (`apple2-core/src/card.rs`, `cards/`)**:
  - Cards implement `trait Card: Send + 'static`; `CardManager` holds 8 slots plus aux (`NUM_SLOTS = 8`, `SLOT_AUX = 8`).
  - `CardType` is `repr(u32)` and must stay value-compatible with C++ `SS_CARDTYPE`.
- **IIgs (`apple2-iigs`)**:
  - `IIgsEmulator { cpu: Cpu65816, bus: IIgsBus }`. `IIgsBus` implements `trait Bus816` with 24-bit `u32` addresses and owns Mega II, FPI, ADB, Ensoniq, SmartPort and BRAM.
  - `IIgsMemory::new` clamps RAM to 256–8192 KB.
- **GUI frame loop (`applewin/src/gui/`)**: emulation runs on the GUI thread, not a thread of its own. Each `EmulatorApp::update` (eframe):
  1. `run_emulation`: cycles = elapsed wall time (capped at 100 ms) × speed. IIgs runs at 2.8 or 1.023 MHz.
  2. Synthesise audio into `AudioBuf = Arc<Mutex<VecDeque<f32>>>`, which the cpal callback thread drains.
  3. Handle input.
  4. Render through `NtscRenderer` or `RgbRenderer` into an egui texture.
  5. Draw the panels and apply `DeferredActions`.
  6. Call `request_repaint`, unless the debugger is stepping.
- **IIgs in the GUI**: `EmulatorApp` still holds a dummy IIe `emu` alongside `iigs: Option<IIgsEmulator>`. Slot cards and save-state are skipped for IIgs.
- **Save-state**:
  - Format: `EmulatorSnapshot { version: 1, model, cpu, memory }`, serialised as YAML (`serde_yaml`, `serde_bytes` for RAM, `.aws.yaml` files).
  - **Card state is not included.** `Card::save_state`/`load_state` exist but are only called inside cards.

## Key Directories

| Path | Purpose |
|---|---|
| `crates/apple2-core/src/` | CPU, `Bus`, soft switches, `cards/` (23 modules), `prodos/` (volume creation), `disk_util.rs` (gz/zip images) |
| `crates/apple2-iigs/src/` | `cpu65816/`, `bus.rs`, `memory.rs`, Mega II, SHR, Ensoniq, ADB, SmartPort |
| `crates/apple2-video/src/` | `Framebuffer` (u32 ABGR), `ntsc.rs`, `rgb.rs`, scanline/blend filters |
| `crates/apple2-audio/src/` | `Speaker`, `Ay8910`, `Ssi263` (render is a simplified stub), `wav_writer.rs` (hound) |
| `crates/apple2-debugger/src/` | Disassembler, breakpoints, symbols, trace, watches; `DebuggerState` |
| `crates/applewin/src/` | `main.rs`, `config.rs`, `gui/` (`mod.rs`, `emulation.rs`, `audio.rs`, `render.rs`, `input.rs`, `settings.rs`, `widgets.rs`) |
| `roms/` | Embedded ROMs/firmware; `roms/Apple_IIgs/` holds runtime-loaded IIgs ROMs |
| `bin/` | Disk images and symbol tables (`A2_BASIC.SYM`, `APPLE2E.SYM`, …) used by tests and the debugger |
| `tasks/` | `todo.md` (phased plans, review notes, open //c gaps), `lessons.md` |
| `.github/workflows/` | `ci.yml`, `release.yml`, `apple-sign.yml` |

## Development Commands

```bash
cargo fmt --all                                         # CI: cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings   # zero warnings, GUI features on
cargo build --release                                   # GUI binary: target/release/applewin
cargo build --release --no-default-features --features headless
cargo test --workspace --no-default-features --features headless   # what CI runs

cargo run --release -p applewin                         # GUI
cargo run --release -p applewin --no-default-features --features headless  # runs IIe Enh for 1,023,000 cycles (1 s), prints PC
```

- There are no CLI flags. Machine, CPU, slots, ROM paths and IIgs RAM all come from `config.toml`.
- Config directory, from `config_dir()` in `crates/applewin/src/config.rs`:
  - Windows: `%APPDATA%\applewin-rs\`
  - macOS: `~/Library/Application Support/applewin-rs/`
  - Linux: `$XDG_CONFIG_HOME` or `~/.config`, then `applewin-rs/`
- The default save-state file `applewin-rs.aws.yaml` sits in the same directory.
- Linux builds need: `libasound2-dev libgtk-3-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libudev-dev`.
- No `rust-toolchain` file; CI uses stable. Edition 2024 implies Rust ≥ 1.85.
- No `rustfmt.toml`, `clippy.toml` or `[workspace.lints]`. Strictness comes only from `-D warnings`.
- Release: bump `[workspace.package].version` and push tag `vX.Y.Z`. `release.yml` builds 4 targets plus SHA256SUMS; `apple-sign.yml` builds ad-hoc-signed macOS DMGs.

## Code Conventions & Common Patterns

- **Errors**:
  - `apple2-core` uses its thiserror `Error`/`Result` (`src/error.rs`).
  - `apple2-iigs` and `applewin` use `Result<_, String>`.
  - GUI file I/O is best-effort (`if let Ok(..)`, `let _ =`).
  - No `anyhow`.
- **Logging**: there is no logging crate; diagnostics go through `eprintln!`/`println!`. Adopt one (e.g. `tracing`) only as a deliberate project-wide change.
- **bitflags v2 with serde** for hardware state: `MemMode`, `Flags`, `Flags816`, `ShadowReg`, prodos `Access`.
- **Hot paths**:
  - `#[inline]` on bus read/write.
  - No per-frame allocation: reuse preallocated scratch buffers (`lc_swap_buf`, `speaker_scratch`, `shr_scratch`).
  - Buffer growth is capped by constants such as `MEM_TRACE_MAX` and `SPEAKER_TOGGLES_MAX`.
- **`unsafe`**: only 2 sites.
  - `cards/mouse.rs` casts `&[u8]` to `&[u8; 256]` on an embedded ROM, so the ROM file size matters.
  - `apple2-video/src/framebuffer.rs` makes a u32-to-u8 view and carries a `// SAFETY:` comment.
  - Any new `unsafe` needs a `// SAFETY:` justification.
- **Style**:
  - let-chains (`if let … && let …`) are used widely.
  - Section banners look like `// ── Section ──`.
  - Names are snake_case; test names sometimes take a `test_` prefix.
- **Feature gating**:
  - `#[cfg(feature = "gui")]` gates `mod config`, `mod gui`, `EmuCore` and the IIgs ROM helpers in `main.rs`.
  - Headless is effectively `not(feature = "gui")`; the `headless` feature is an empty marker.
- **Adding an expansion card**:
  1. `crates/apple2-core/src/cards/<name>.rs`: a struct with `new(slot)` and `impl Card`.
     - Required methods: `card_type`, `slot`, `io_read`, `io_write`, `reset`, `update`, `save_state`, `load_state`, `as_any_mut`.
     - Optional overrides: `cx_rom`, `slot_io_read`/`slot_io_write`, `irq_active`, `fill_audio`, the DMA hooks, the LC bank hooks and the disk activity hooks.
  2. Add `pub mod <name>;` in `cards/mod.rs`, plus a new `CardType` variant if needed (keep the C++ values).
  3. Construct it in the `apply_slot_cards` match in `crates/applewin/src/gui/emulation.rs`. Unmatched types fall through to `_ => {}`, which silently leaves the slot empty.
  4. Add it to `IMPLEMENTED_CARDS` and to the exhaustive `card_name` match in `config.rs`.
  5. Put option popups in `gui/settings.rs`. Audio cards also need the card-type check in `gui/audio.rs`.
- The //c ignores the slot config. It hardcodes SSC in slots 1 and 2, Mouse in slot 4, and Disk II with IWM (`set_iwm(true)`) in slot 6.

## Important Files

- `crates/applewin/src/main.rs`:
  - Embeds `APPLE2E_ROM` and `APPLE2C_ROM`.
  - `make_emulator` handles custom 12K/16K/32K ROMs.
  - Defines `EmuCore { IIe(Emulator), IIgs(Box<IIgsEmulator>) }`.
  - `find_iigs_rom` tries `cfg.iigs_rom_path`, then `roms/`, `roms/Apple_IIgs/` and the exe dir, under both the exe dir and the cwd. The ROM must be 131072 or 262144 bytes. If it fails, the app falls back to the IIe Enhanced core.
- `crates/applewin/src/config.rs`: TOML `Config::load`/`save`, `IMPLEMENTED_CARDS`, `card_name`, `VideoType`.
- `crates/apple2-core/src/{emulator.rs, bus/mod.rs, bus/soft_switches.rs, card.rs, model.rs}`: the core engine. `Apple2Model::is_iic()`; `CpuType` defaults to 65C02.
- Assets embedded with `include_bytes!` (paths are relative to the source file, so moving or renaming either side breaks the build):
  - `roms/apple2e_enhanced.rom`
  - `roms/Apple_IIc/Apple IIc ROM 04 - 341-0445-B.bin`
  - `roms/Apple2e_Enhanced_Video.rom` (in `gui/render.rs`)
  - `roms/Hddrvr.bin`, `roms/MouseInterface.rom`
  - `roms/firmware/*.bin` (in `prodos/create.rs`)
  - `crates/applewin/icons/*.BMP` (in `gui/widgets.rs`)
- `CHANGELOG.md`: Keep a Changelog format with SemVer.
  - Sections are `## [X.Y.Z] - YYYY-MM-DD`, then `### Added|Changed|Fixed|Performance|Tests|Removed`.
  - Bullets look like `- **crate: summary.** detail`. Write "No behavior change" for pure refactors.
  - Every significant change needs an entry.

## Runtime/Tooling Preferences

- Cargo only: there are no scripts, Makefile or justfile. `Cargo.lock` is committed.
- Release profile: `lto = true`, `codegen-units = 1`. Release builds are slow; use debug builds for quick iteration.
- Paths under `roms/` and `bin/` contain spaces and parentheses. Always quote them in shell commands, e.g. `"roms/Apple_IIc/Apple IIc ROM 04 - 341-0445-B.bin"`.
- Commits use Conventional Commits, scoped by crate: `fix(audio): …`, `refactor(applewin): …`, `feat(apple2-iigs): …`, `chore(release): vX.Y.Z`.
  - Some history has unfilled template text such as `@type(@scope): @message`. Don't copy it.
- Branches are `feature/*` and `fixes/vX.Y.Z`. CI runs on `main` and `development`.

## Testing & QA

- Test tooling: built-in libtest only. The only dev-dependency is `tempfile`. There are no benches, coverage tools, proptest/insta or `#[ignore]` tests.
- About 490 `#[test]` functions, counted with grep. `applewin` has none.

| Crate | Location | Notes |
|---|---|---|
| apple2-core | `src/cpu/tests.rs` (156), `src/bus_tests.rs` (51), in-file `mod tests` in cards | Helpers: `make_bus()`, `make_iic_bus()`, `cpu_6502(pc)`, `cpu_65c02(pc)`, `poke`, `step` |
| apple2-core | `tests/integration.rs` | Full `Emulator::new(rom, model, cpu)` with synthetic ROMs (`build_rom`, `build_iic_rom`) |
| apple2-core | `tests/iic_boot_trace.rs` | **Slow.** Real files: //c ROM 4 (must be 32768 bytes), `bin/DOS 3.3 System Master - 680-0210-A.dsk`, `bin/ProDOS_2_4_3.po`; runs 30M–150M cycles |
| apple2-iigs | `src/cpu65816/tests.rs`, `tests/{integration,peripherals}.rs` | `TestBus` implements `Bus816`; synthetic ROMs only, no real IIgs ROM needed |
| apple2-video / audio | in-file tests | Video tests render every mode to check for out-of-bounds writes and full coverage |

```bash
cargo test -p apple2-core                              # one crate
cargo test -p apple2-core --lib                        # unit tests only
cargo test -p apple2-iigs --test peripherals           # one integration file
cargo test -p apple2-core jmp_indirect_nmos_page_bug   # by name
cargo test -p apple2-core -- --skip iic_rom4           # skip slow //c boot tests
cargo test --release -p apple2-core --test iic_boot_trace  # boot traces in reasonable time
```

- Integration tests assert through public state: `emu.bus.main_ram`, `emu.cpu.cycles`, `bus.mode.contains(MemMode::MF_*)`. Regression tests carry doc comments that name the bug they guard.
- From `tasks/lessons.md`: for a wrong-output bug (sound, video, timing), reproduce it first and capture real data. One approach that worked: a headless probe that boots the user's disk and drains `bus.speaker_toggles` each frame. A plausible bug found by reading code isn't necessarily the reported one; verify the symptom end-to-end.
- GUI changes have no automated coverage. Smoke-test by running the binary.

## Known Gotchas

- Open //c gaps tracked in `tasks/todo.md`:
  - `$C058–$C05D` behave as annunciators, not mouse interrupt switches.
  - `$C060` reads as cassette input, not the 80-column switch.
  - `$C048–$C04F` are unmapped.
- `CHANGELOG.md`: add new entries under `[Unreleased]` at the top; versions are listed newest first.
- Save states hold CPU, RAM and soft-switch state only (no cards, no ROM); `Emulator::restore_snapshot` rejects snapshots from another model.
