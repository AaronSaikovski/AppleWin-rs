# AppleWin-rs

![CI](https://github.com/AaronSaikovski/AppleWin-rs/actions/workflows/ci.yml/badge.svg)
![Release](https://github.com/AaronSaikovski/AppleWin-rs/actions/workflows/release.yml/badge.svg)
![License: GPL v2](https://img.shields.io/badge/License-GPL%20v2-blue.svg)
![Version](https://img.shields.io/badge/version-1.1.1-green.svg)

A Rust rewrite of [AppleWin](https://github.com/AppleWin/AppleWin) — a fully-featured Apple II emulator originally written for Windows. This port provides cross-platform support (Windows, macOS, Linux) while maintaining cycle-accurate emulation.

> **Original project:** [https://github.com/AppleWin/AppleWin](https://github.com/AppleWin/AppleWin)
> **This port:** [https://github.com/AaronSaikovski/AppleWin-rs](https://github.com/AaronSaikovski/AppleWin-rs)

## Downloads

Pre-built binaries for Windows, macOS, and Linux are available on the [Releases](https://github.com/AaronSaikovski/AppleWin-rs/releases) page.

---

## Apple II Models Supported

- Apple II (`][`)
- Apple II+ (`][+`)
- Apple IIe (`//e`)
- Apple IIe Enhanced (`//e Enhanced`) — default
- Apple IIc (`//c`) — 32KB ROM, built-in peripherals, 128KB RAM
- **Apple IIgs** (`//gs`) — 65C816 CPU, Super Hi-Res graphics, Ensoniq audio

> No support currently for the //c+, Laser 128, or Laser 128EX/EX2.

---

### Apple IIgs Features

The Apple IIgs emulation includes:

| Feature | Description |
|---------|-------------|
| 65C816 CPU | Full 16-bit CPU with all 256 opcodes, emulation and native modes |
| Super Hi-Res | 320x200 (16 colors) and 640x200 (4 colors) per-scanline modes |
| Ensoniq DOC 5503 | 32-oscillator wavetable synthesizer with 64KB sound RAM |
| ADB | Apple Desktop Bus keyboard and mouse input |
| Mega II | Full Apple IIe backwards compatibility |
| Memory | 256KB to 8MB configurable RAM, bank-switched addressing |
| ROM support | ROM 00, ROM 01, and ROM 03 auto-detected from `roms/Apple_IIgs/` |
| SmartPort | Block-level 3.5" (.2mg) and hard disk image support |
| Speed control | 1 MHz (IIe compatible) and 2.8 MHz (native) switching |
| Shadowing | Bank $00/$01 to $E0/$E1 display memory mirroring |
| BRAM | 256-byte battery-backed parameter RAM with factory defaults |

> **ROM files:** IIgs ROMs are not included. Place your ROM file in `roms/Apple_IIgs/` next to the executable. ROM 03 (256KB) is recommended. The emulator auto-detects the ROM version.

---

## Peripheral Cards & Add-on Hardware

21 expansion cards are implemented across 8 slots plus an auxiliary slot:

| Card | Description |
|------|-------------|
| Disk II | 5.25" floppy controller (DSK/DO/PO/NIB/WOZ v1 & v2/D13) |
| Hard Disk Controller | ProDOS block device (HDV/PO/2MG), up to 8 drives |
| Mockingboard | Dual 6522 VIA + 2x AY-3-8910 PSG sound card |
| Phasor | Mockingboard superset with native dual-mode |
| MegaAudio | Mockingboard-compatible with enhanced 3rd PSG |
| SD Music | Mockingboard-compatible with SD card music streaming |
| SAM | Software Automated Mouth (8-bit DAC) |
| SSI263 | Phoneme-based speech synthesizer (used with Mockingboard/Phasor) |
| Super Serial Card | 6551 ACIA emulation with TCP/UDP support |
| Parallel Printer | Output to `printer.txt` file |
| Mouse Interface | Mouse card with firmware ROM |
| 80-Column Text Card | 1K and Extended 64K variants |
| RamWorks III | Auxiliary RAM expansion (64K-8192K configurable) |
| Language Card | 16K RAM expansion ($D000-$FFFF) |
| Saturn 128K | Up to 8 banks of 16K language card RAM |
| Uthernet I | CS8900A ethernet (register stubs for detection) |
| Uthernet II | WIZnet W5100 with TCP/UDP sockets and Virtual DNS |
| 4Play | 4-port digital joystick interface |
| SNES MAX | Dual SNES controller serial interface |
| VidHD | Modern video output card |
| Z80 SoftCard | CP/M card (card present, Z80 CPU not yet emulated) |
| No Slot Clock | Dallas DS1216 real-time clock |

**Additional hardware:**
- Game I/O connector copy protection dongles (5 types)
- Cassette tape I/O (WAV file loading)

---

## Architecture

The project is structured as a Cargo workspace with six crates, organised so that the core emulation has zero OS or I/O dependencies:

```
AppleWin-rs/
├── crates/
│   ├── apple2-core       # Pure emulation engine (CPU, bus, cards)
│   ├── apple2-iigs       # Apple IIgs emulation (65C816, SHR, Ensoniq, ADB)
│   ├── apple2-audio      # Audio synthesis (speaker, AY8910, SSI263)
│   ├── apple2-video      # Video rendering (NTSC, RGB, hi-res, double hi-res)
│   ├── apple2-debugger   # Symbolic debugger (disassembler, breakpoints, symbols)
│   └── applewin          # Main application, GUI, and platform I/O
├── roms/                 # All ROM files (embedded at compile time via include_bytes!)
│   └── firmware/         # ProDOS, DOS 3.3, and utility firmware binaries
└── bin/                  # Runtime resources (disk images, symbol tables)
```

### Crate Responsibilities

| Crate | Purpose | Key Dependencies |
|---|---|---|
| `apple2-core` | 6502/65C02 CPU, memory bus, 21 expansion card implementations | `bitflags`, `thiserror`, `tracing`, `serde` |
| `apple2-iigs` | 65C816 CPU, IIgs memory bus, Mega II, SHR video, Ensoniq audio, ADB, SmartPort | `apple2-core`, `bitflags`, `tracing`, `serde` |
| `apple2-audio` | AY8910 PSG synthesis, SSI263 speech, speaker emulation | `thiserror`, `tracing`, `serde` |
| `apple2-video` | Framebuffer (560x384), NTSC signal chain, all video mode rendering | `apple2-core`, `thiserror`, `tracing`, `serde` |
| `apple2-debugger` | Disassembler, breakpoint manager, symbol table loader | `apple2-core`, `thiserror`, `tracing`, `serde` |
| `applewin` | GUI (egui/eframe), audio output (cpal), gamepad (gilrs), config (toml) | all above + `eframe`, `cpal`, `gilrs`, `rfd`, `png` |

### Design Principles

- **`apple2-core` is OS-agnostic** — no platform code, no I/O; purely emulation logic.
- **Subsystem crates** (`audio`, `video`, `debugger`) depend only on `apple2-core`.
- **`applewin`** is the only crate with platform and GUI dependencies.
- **Apple II/IIe/IIc ROMs are centralised** in the top-level `roms/` directory and embedded at compile time via `include_bytes!`. **Apple IIgs ROMs** are loaded at runtime from `roms/Apple_IIgs/` (128-256KB, not distributed).
- **Headless mode** is available (no GUI/audio dependencies) for testing and CI.

---

## Building

### Prerequisites

- Rust toolchain: **1.85+** (edition 2024 workspace) — install via [rustup](https://rustup.rs)
- **Windows:** MSVC build tools (`x86_64-pc-windows-msvc` target)
- **Linux:** system packages for audio, GUI, and gamepad:
  ```sh
  sudo apt-get install libasound2-dev libgtk-3-dev \
      libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
      libxkbcommon-dev libudev-dev
  ```
- **macOS:** standard Xcode command-line tools

### Standard (GUI) Build

```sh
git clone https://github.com/AaronSaikovski/AppleWin-rs.git
cd AppleWin-rs
cargo build --release
```

The release binary is produced at:

```
target/release/applewin        # Linux/macOS
target/release/applewin.exe    # Windows
```

### Headless Build (no GUI, no audio)

Useful for CI, automated testing, or embedding:

```sh
cargo build --release --no-default-features --features headless
```

### Run

```sh
cargo run --release
```

Or run the compiled binary directly:

```sh
./target/release/applewin
```

### Cargo Features

| Feature | Default | Description |
|---|---|---|
| `gui` | yes | Enables the full egui/eframe GUI, audio (cpal), gamepad (gilrs), and file dialogs (rfd) |
| `headless` | no | Strips all GUI/audio/I/O for pure emulation builds |

### Testing

```sh
cargo test
```

Runs 467 tests across all crates:

| Crate | Tests | Coverage |
|---|---|---|
| `apple2-core` | 290 | CPU opcodes (6502/65C02/undocumented), addressing modes, BCD arithmetic, interrupts, soft switches, language card, ALTZP memory routing, expansion cards, Disk II controller, IWM compatibility, Apple IIc model (INTCXROM, ROM banking, IOUDIS/DHIRES gating), Via6522 (register read/write, timers, IRQ, state serialization), performance regression guards (dispatch-table equivalence, speaker toggle cap, card slot range checks) |
| `apple2-core` (integration) | 12 | Boot sequence, program execution, snapshots, Fibonacci, Apple IIc boot/reset/ROM execution |
| `apple2-iigs` | 89 | 65C816 CPU: all addressing modes, 8/16-bit arithmetic, BCD, mode switching (XCE/REP/SEP), block moves, interrupts, stack ops, TSB/TRB, COP |
| `apple2-iigs` (integration) | 15 | ROM boot, RAM programs, native mode 16-bit, bus banking, shadowing, Mega II soft-switches |
| `apple2-iigs` (peripherals) | 35 | Memory/ROM mapping, BRAM checksums, ADB protocol, SHR rendering, Ensoniq registers, SmartPort disk I/O (incl. firmware stub, READ BLOCK via WDM trap, NO DEVICE error path) |
| `apple2-audio` | 10 | Speaker interpolation, DC filter, amplitude, WAV recording |
| `apple2-video` | 14 | NTSC tables, text/lores/hires/dlores rendering, mixed mode |
| `apple2-debugger` | 2 | Disassembly |

---

## Disk Image Support

| Format | Extension | Description |
|---|---|---|
| DOS 3.3 | `.dsk`, `.do` | 140K floppy, 6+2 GCR encoding |
| ProDOS | `.po` | 140K floppy, ProDOS sector order |
| Nibble | `.nib` | Raw nibblized tracks |
| WOZ v1/v2 | `.woz` | Flux-level bitstream with weak bit support for copy-protected disks |
| DOS 3.2 | `.d13` | 113K floppy, 5+3 GCR encoding (13-sector) |
| Hard disk | `.hdv`, `.po`, `.2mg` | ProDOS block device (512-byte blocks) |
| Compressed | `.gz`, `.zip` | Auto-decompressed wrappers around any of the above |

---

## Features

- **Drag-and-drop** disk image loading
- **Clipboard** copy/paste (Ctrl+C copies text screen, Ctrl+V pastes as keystrokes)
- **Screenshot** capture (F12, saves PNG to screenshots directory)
- **WAV recording** of emulator audio (F9 toggle)
- **Disk activity indicators** — drive LEDs with real-time track numbers and HDD activity in the status bar
- **Cassette tape** I/O (load WAV files for tape-based software)
- **Symbolic debugger** with disassembler, breakpoints, watches, and single-step
- **Save/restore** emulator state (F11 / Shift+F11)
- **Gamepad support** via gilrs (Xbox, PlayStation, and other controllers)
- **TOML configuration** with platform-standard paths
- **Optimized hot paths** — per-instruction dispatch with hoisted 6502/65C02
  table selection and `#[inline]` on the ~90 most-executed opcode handlers,
  direct-to-display rendering with no intermediate framebuffer copy,
  compile-time SHR scale LUTs, and lock-free audio synthesis into reusable
  scratch buffers for the speaker, Ensoniq DOC, and Mockingboard paths

---

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| F1 | Hard reset |
| Ctrl+F2 | Soft reset |
| F7 | Toggle debugger |
| F9 | Toggle WAV audio recording |
| F11 | Save state |
| Shift+F11 | Load state |
| F12 | Screenshot (PNG) |
| Ctrl+Esc | Quit |
| Ctrl+C | Copy text screen to clipboard |
| Ctrl+V | Paste clipboard as keystrokes |
| Ctrl+0 | Speed 40x |
| Ctrl+1 | Speed 10x (normal) |
| Ctrl+3 | Speed 30x |
| Ctrl+4 | Video: Monochrome White |
| Ctrl+5 | Video: Monochrome Green |
| Ctrl+6 | Video: Color TV |
| Ctrl+7 | Video: Color Idealized |
| Ctrl+8 | Video: Color RGB |
| Ctrl+9 | Video: Color NTSC |

**Debugger keys:** Space (step), Ctrl+Space (step over), Shift+Space (step out), F5 (resume)

---

## Video Modes

| Mode | Description |
|------|-------------|
| Color TV | Color NTSC signal-chain TV rendering (default) |
| Color Idealized | Simplified NTSC colour-cell rendering |
| Color RGB | RGB card/monitor output |
| Color Monitor NTSC | Color NTSC signal-chain monitor rendering |
| Mono TV | Monochrome TV (white phosphor, composite bandwidth) |
| Mono Amber | Amber phosphor monochrome |
| Mono Green | Green phosphor monochrome |
| Mono White | Pure white phosphor monochrome |
| Mono Custom | Custom monochrome color (0xRRGGBB) |

Additional options: scanlines, color vertical blending, 50/60 Hz refresh rate.

---

## Configuration

On first run, `applewin` creates a TOML config file in the platform-standard location:

| Platform | Path |
|---|---|
| Windows | `%APPDATA%\applewin-rs\config.toml` |
| macOS | `~/Library/Application Support/applewin-rs/config.toml` |
| Linux | `$XDG_CONFIG_HOME/applewin-rs/config.toml` |

### Configurable Options

- **Machine:** Model (II/II+/IIe/IIe Enhanced/IIgs), CPU type (6502/65C02/65C816), slot card assignments
- **Video:** Mode, scanlines, color blending, monochrome color, refresh rate
- **Audio:** Master volume (0-100%)
- **Speed:** Emulation speed (0-40, 10 = normal), enhanced disk speed (16x during motor spin)
- **Input:** Joystick type per port, paddle trim, auto-fire, self-centering, button swap, mouse options
- **Memory:** RAM initialization pattern (0-7), custom ROM paths, IIgs RAM size (256KB-8MB), IIgs ROM path
- **Save state:** Auto-save on exit, custom save state path
- **UI:** Window scale, position, disk activity LEDs, confirm reboot dialog

### Save States

Save states are stored in YAML format alongside the config file as `applewin-rs.aws.yaml`. Use F11 to save and Shift+F11 to restore. Optionally enable auto-save on exit in the settings.

### Screenshots

Screenshots are saved as PNG files to `%APPDATA%\applewin-rs\screenshots\` on Windows, or the current directory on other platforms.

---

## Unofficial Ports of the Original AppleWin

These ports allow building the original C++ AppleWin on non-Windows platforms:

- [Linux](https://github.com/audetto/AppleWin)
- [macOS](https://github.com/sh95014/AppleWin)

---

## CI/CD

**Continuous Integration** runs on every push and PR to `main` and `development` branches:
- `cargo fmt --all --check` — code formatting
- `cargo clippy --workspace --all-targets -- -D warnings` — lint (all platforms)
- `cargo test` — test suite (all platforms)
- `cargo build --release` — GUI and headless builds (all platforms)

**Release builds** are triggered by version tags (`v*.*.*`) and produce archives for:
- Windows x86_64 (`.zip`)
- macOS x86_64 and aarch64 (`.tar.gz`)
- Linux x86_64 (`.tar.gz`)

Each release includes SHA256 checksums and auto-generated release notes.

---

## Contributing

This project is a Rust port of the original AppleWin. For background on the original emulator's design, see the [original repository](https://github.com/AppleWin/AppleWin) and its [CONTRIBUTING](https://github.com/AppleWin/AppleWin/blob/master/CONTRIBUTING.md) guide.

Please report issues for this Rust port at: [https://github.com/AaronSaikovski/AppleWin-rs/issues](https://github.com/AaronSaikovski/AppleWin-rs/issues)

---

## License

GPL-2.0-or-later — see [LICENSE](LICENSE) for details.

This project is based on [AppleWin](https://github.com/AppleWin/AppleWin), which is also licensed under GPL-2.0-or-later.

---

## Apple II Roms
Download Apple II and IIGS roms from [Virtual Apple ](https://www.virtualapple.org/),
