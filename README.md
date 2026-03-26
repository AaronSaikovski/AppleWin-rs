# AppleWin-rs

A Rust rewrite of [AppleWin](https://github.com/AppleWin/AppleWin) — a fully-featured Apple II emulator originally written for Windows.

> **Original project:** [https://github.com/AppleWin/AppleWin](https://github.com/AppleWin/AppleWin)
> **This port:** [https://github.com/AaronSaikovski/AppleWin-rs](https://github.com/AaronSaikovski/AppleWin-rs)

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
- Disk II interface for floppy disk drives
- Hard disk controller
- Super Serial Card (SSC)
- Parallel printer card
- Mouse interface
- Apple IIe Extended 80-Column Text Card and RamWorks III (8MB)
- RGB cards: Apple's Extended 80-Column Text/AppleColor Adaptor Card and 'Le Chat Mauve' Féline
- CP/M SoftCard (Z80)
- Uthernet I and II (ethernet cards)
- Language Card and Saturn 64/128K
- 4Play and SNES MAX joystick cards
- VidHD card (IIgs Super Hi-Res video modes)
- No Slot Clock (NSC)
- Game I/O Connector copy protection dongles

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
| `apple2-audio` | AY8910 PSG synthesis, SSI263 speech, speaker emulation | `thiserror`, `tracing`, `serde` |
| `apple2-video` | Framebuffer, NTSC signal chain, all video mode rendering | `apple2-core`, `thiserror`, `tracing`, `serde` |
| `apple2-debugger` | Disassembler, breakpoint manager, symbol table loader | `apple2-core`, `thiserror`, `tracing`, `serde` |
| `applewin` | GUI (egui/eframe), audio output (cpal), gamepad (gilrs), config (toml) | all above + `eframe`, `cpal`, `gilrs`, `rfd`, `winapi` |

### Design Principles

- **`apple2-core` is OS-agnostic** — no platform code, no I/O; purely emulation logic.
- **Subsystem crates** (`audio`, `video`, `debugger`) depend only on `apple2-core`.
- **`applewin`** is the only crate with platform and GUI dependencies.
- **ROMs are embedded at compile time** via `include_bytes!` — no runtime file loading required.
- **Headless mode** is available (no GUI/audio dependencies) for testing and CI.

---

## Building

### Prerequisites

- Rust toolchain: **1.85+** (edition 2024 workspace)
- On Windows, MSVC build tools or the `x86_64-pc-windows-msvc` target
- On Linux/macOS, standard C toolchain (`gcc`/`clang`) for native audio/GUI crate build scripts

Install Rust via [rustup](https://rustup.rs):

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

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

---

## Workspace Dependencies

All shared dependency versions are pinned in the root `Cargo.toml` and referenced with `workspace = true` in each crate:

| Crate | Version |
|---|---|
| `bitflags` | 2.x (with `serde` feature) |
| `thiserror` | 1.x |
| `tracing` | 0.1 |
| `serde` | 1.x (with `derive` feature) |
| `serde_yaml` | 0.9 |
| `serde_bytes` | 0.11 |
| `eframe` / `egui` | 0.23 (GUI only) |
| `cpal` | 0.15 (GUI only) |
| `gilrs` | 0.10 (GUI only) |
| `rfd` | 0.12 (GUI only) |
| `toml` | 0.8 (GUI only) |

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
