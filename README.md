# AppleWin-rs

![CI](https://github.com/AaronSaikovski/AppleWin-rs/actions/workflows/ci.yml/badge.svg)
![Release](https://github.com/AaronSaikovski/AppleWin-rs/actions/workflows/release.yml/badge.svg)
![License: GPL v2](https://img.shields.io/badge/License-GPL%20v2-blue.svg)
![Version](https://img.shields.io/badge/version-1.0.0-green.svg)

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
- Apple IIe Enhanced (`//e Enhanced`)
- Various clones (Pravets, TK3000, Base 64)

> No support currently for the //c, //c+, Laser 128, Laser 128EX/EX2, or Apple IIgs.

---

## Peripheral Cards & Add-on Hardware

- Mockingboard, Phasor and SAM sound cards
- SSI-263 speech synthesis chip (phoneme playback with IRQ)
- Disk II interface for floppy disk drives (DSK/DO/PO/NIB/WOZ v1 & v2)
- Hard disk controller (HDV/PO/2MG)
- Super Serial Card (SSC) with 6551 ACIA emulation
- Parallel printer card
- Mouse interface
- Apple IIe Extended 80-Column Text Card and RamWorks III (8MB)
- RGB cards: Apple's Extended 80-Column Text/AppleColor Adaptor Card and 'Le Chat Mauve' Féline
- CP/M SoftCard (Z80)
- Uthernet I (CS8900A) and Uthernet II (W5100) ethernet cards
- Language Card and Saturn 64/128K
- 4Play and SNES MAX joystick cards
- VidHD card (IIgs Super Hi-Res video modes with status register emulation)
- No Slot Clock (NSC)
- Game I/O Connector copy protection dongles
- Cassette tape I/O (WAV file loading)

---

## Architecture

The project is structured as a Cargo workspace with five crates, organised so that the core emulation has zero OS or I/O dependencies:

```
AppleWin-rs/
├── crates/
│   ├── apple2-core       # Pure emulation engine (CPU, bus, cards)
│   ├── apple2-audio      # Audio synthesis (speaker, AY8910, SSI263)
│   ├── apple2-video      # Video rendering (NTSC, RGB, hi-res, double hi-res)
│   ├── apple2-debugger   # Symbolic debugger (disassembler, breakpoints, symbols)
│   └── applewin          # Main application, GUI, and platform I/O
└── bin/                  # Runtime resources (disk images, symbol tables)
```

### Crate Responsibilities

| Crate | Purpose | Key Dependencies |
|---|---|---|
| `apple2-core` | 6502/65C02 CPU, memory bus, 19 expansion card implementations | `bitflags`, `thiserror`, `tracing`, `serde` |
| `apple2-audio` | AY8910 PSG synthesis, SSI263 speech, speaker emulation (sub-sample interpolation, DC removal) | `thiserror`, `tracing`, `serde` |
| `apple2-video` | Framebuffer, NTSC signal chain, all video mode rendering (text, lo-res, hi-res, double hi-res, double lo-res) | `apple2-core`, `thiserror`, `tracing`, `serde` |
| `apple2-debugger` | Disassembler, breakpoint manager, symbol table loader | `apple2-core`, `thiserror`, `tracing`, `serde` |
| `applewin` | GUI (egui/eframe), audio output (cpal), gamepad (gilrs), config (toml) | all above + `eframe`, `cpal`, `gilrs`, `rfd`, `winapi` |

### Design Principles

- **`apple2-core` is OS-agnostic** — no platform code, no I/O; purely emulation logic.
- **Subsystem crates** (`audio`, `video`, `debugger`) depend only on `apple2-core`.
- **`applewin`** is the only crate with platform and GUI dependencies.
- **ROMs are embedded at compile time** via `include_bytes!` — no runtime file loading required.
- **Headless mode** is available (no GUI/audio dependencies) for testing and CI.
- **Comprehensive test suite** — 297 tests covering CPU instructions, memory soft-switches, expansion cards, and integration scenarios.

---

## Building

### Prerequisites

- Rust toolchain: **1.85+** (edition 2024 workspace) — install via [rustup](https://rustup.rs)
- **Windows:** MSVC build tools (`x86_64-pc-windows-msvc` target)
- **Linux:** system packages for audio and GUI:
  ```sh
  sudo apt-get install libasound2-dev libgtk-3-dev \
      libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev
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

Or run the compiled binary directly and optionally pass a disk image:

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

Runs 297 tests across all crates:

| Crate | Tests | Coverage |
|---|---|---|
| `apple2-core` | 247 | CPU opcodes (6502/65C02/undocumented), addressing modes, BCD arithmetic, interrupts, soft switches, language card, expansion cards |
| `apple2-core` (integration) | 9 | Boot sequence, program execution, snapshots, Fibonacci |
| `apple2-audio` | 9 | Speaker interpolation, DC filter, amplitude |
| `apple2-video` | 14 | NTSC tables, text/lores/hires/dlores rendering, mixed mode |
| `apple2-debugger` | 18 | Disassembly, breakpoints, assembler |

---

## Disk Image Support

| Format | Extension | Description |
|---|---|---|
| DOS 3.3 | `.dsk`, `.do` | 140K floppy, 6+2 GCR encoding |
| ProDOS | `.po` | 140K floppy, ProDOS sector order |
| Nibble | `.nib` | Raw nibblized tracks |
| WOZ v1/v2 | `.woz` | Flux-level bitstream with weak bit support for copy-protected disks |
| DOS 3.2 | `.d13` | 113K floppy, 5+3 GCR encoding (13-sector) |
| Hard disk | `.hdv`, `.po`, `.2mg` | ProDOS block device |

---

## Features

- **Drag-and-drop** disk image loading
- **Clipboard** copy/paste (Ctrl+C copies text screen, Ctrl+V pastes as keystrokes)
- **Screenshot** capture (F12, saves BMP)
- **Cassette tape** I/O (load WAV files for tape-based software)
- **Symbolic debugger** with disassembler, breakpoints, and single-step
- **Save/restore** emulator state
- **TOML configuration** with platform-standard paths

---

## Configuration

On first run, `applewin` creates a TOML config file in the platform-standard location:

| Platform | Path |
|---|---|
| Windows | `%APPDATA%\applewin-rs\config.toml` |
| macOS | `~/Library/Application Support/applewin-rs/config.toml` |
| Linux | `$XDG_CONFIG_HOME/applewin-rs/config.toml` |

### Video Modes

`Mono Custom`, `Color Idealized`, `Color RGB`, `Color NTSC`, `Color TV`, `Mono TV`, `Mono Amber`, `Mono Green`, `Mono White`

### Joystick / Input Modes

`Disabled`, `Joystick 1`, `Joystick 2`, `Numeric Keypad`, `Arrow Keys`, `Mouse`

---

## Unofficial Ports of the Original AppleWin

These ports allow building the original C++ AppleWin on non-Windows platforms:

- [Linux](https://github.com/audetto/AppleWin)
- [macOS](https://github.com/sh95014/AppleWin)

---

## Contributing

This project is a Rust port of the original AppleWin. For background on the original emulator's design, see the [original repository](https://github.com/AppleWin/AppleWin) and its [CONTRIBUTING](https://github.com/AppleWin/AppleWin/blob/master/CONTRIBUTING.md) guide.

Please report issues for this Rust port at: [https://github.com/AaronSaikovski/AppleWin-rs/issues](https://github.com/AaronSaikovski/AppleWin-rs/issues)

---

## License

GPL-2.0-or-later — see [LICENSE](LICENSE) for details.

This project is based on [AppleWin](https://github.com/AppleWin/AppleWin), which is also licensed under GPL-2.0-or-later.
