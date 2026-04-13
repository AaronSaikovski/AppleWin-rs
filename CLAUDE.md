# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

AppleWin-rs is a Rust rewrite of the [AppleWin](https://github.com/AppleWin/AppleWin) Apple II emulator (C++ original), providing cross-platform support while maintaining cycle-accurate emulation of Apple II models and 19+ peripheral expansion cards.

## Code Quality Requirements

**MANDATORY: After every code change or update, you MUST run the following three checks in order before considering the work complete. No exceptions.**

1. Format all code: `cargo fmt --all`
2. Pass Clippy with no warnings: `cargo clippy --workspace --all-targets -- -D warnings`
3. Build cleanly with no errors or warnings: `cargo build --release`

All three checks must pass. If any check fails, fix all errors and warnings immediately before moving on. The codebase must be in a clean, formatted, warning-free state at all times.

**CI enforces these same checks** on every push and PR to `main` and `development` branches. A PR will not merge if any check fails.

## Build & Test Commands

```bash
# Standard GUI build (release)
cargo build --release

# Headless build (no GUI/audio — useful for CI and testing)
cargo build --release --no-default-features --features headless

# Run all tests
cargo test

# Run a specific test
cargo test <test_name>

# Lint
cargo clippy

# Format
cargo fmt
```

## Workspace Structure

The project is a Cargo workspace with 5 crates, each with a distinct responsibility:

| Crate | Purpose |
|-------|---------|
| `crates/apple2-core` | Pure emulation engine — CPU, memory bus, expansion cards. **Zero OS dependencies.** |
| `crates/apple2-audio` | Audio synthesis (speaker, AY-8910/Mockingboard, SSI263 speech) |
| `crates/apple2-video` | Video rendering pipeline (NTSC, RGB, hi-res, text modes) |
| `crates/apple2-debugger` | 6502 disassembler, breakpoints, symbol table — no GUI dependencies |
| `crates/applewin` | Main application: egui/eframe GUI, audio I/O (cpal), gamepad (gilrs), TOML config |

## Key Architectural Decisions

**Core has no platform dependencies.** `apple2-core` is strictly OS-agnostic. All platform code (GUI, audio, file dialogs, input) lives exclusively in `crates/applewin`. This is enforced by crate boundaries.

**ROMs are compiled in.** All ROM data is embedded via `include_bytes!` macros — there is no runtime ROM file loading. ROM files live under `crates/apple2-core/roms/` and `crates/applewin/roms/`.

**Headless mode.** Building with `--no-default-features --features headless` produces a binary with no GUI or audio, suitable for CI or library embedding.

## Core Emulation Internals

The main data flow:
```
Emulator (emulator.rs)
  └── Bus (bus.rs)         — 64KB address space, soft-switches, card slots
       └── Cpu (cpu/)      — 6502/65C02 registers, flags, instruction dispatch
       └── Cards (cards/)  — 19 expansion card implementations
```

- **`Emulator::execute(cycles)`** is the main entry point for cycle-accurate execution.
- **`Bus`** manages memory modes via 18 bitflags (`MemMode`), routes reads/writes to the correct card slot, and holds gamepad/IRQ state.
- **Cards** implement a shared trait with `init`, `read`, `write`, and `interrupt` lifecycle methods.

## Features

```toml
[features]
default = ["gui"]
gui     = ["eframe", "rfd", "cpal", "gilrs", "toml", "winapi"]
headless = []
```

GUI pulls in the full platform stack; headless strips it entirely.

## Configuration

At runtime, `applewin` stores a TOML config file at the platform config dir (`%APPDATA%\applewin-rs\config.toml` on Windows). Configurable: machine model, CPU type (6502/65C02), video mode, joystick mode, and per-slot card assignment.

## Supported Hardware

**Models**: Apple II, II+, IIe, IIe Enhanced (and some clones), Apple //c. Apple IIgs is **not** supported.

**Expansion cards** (19 implemented): Disk II, Hard Disk Controller, Mockingboard/Phasor, SAM, SSI263, 80-Column, RamWorks III, Language Card, Saturn, Mouse, 4Play, SNES MAX, Uthernet I/II, Printer, Super Serial, Z80 CP/M, VidHD, No Slot Clock.


## Workflow Orchestration

### 1. Plan Node Default

- Enter plan mode for ANY non-trivial task (3+ steps or architectural decisions)
- If something goes sideways, STOP and re-plan immediately – don't keep pushing
- Use plan mode for verification steps, not just building
- Write detailed specs upfront to reduce ambiguity

### 2. Subagent Strategy

- Use subagents liberally to keep main context window clean
- Offload research, exploration, and parallel analysis to subagents
- For complex problems, throw more compute at it via subagents
- One task per subagent for focused execution

### 3. Self-Improvement Loop

- After ANY correction from the user: update `tasks/lessons.md` with the pattern
- Write rules for yourself that prevent the same mistake
- Ruthlessly iterate on these lessons until mistake rate drops
- Review lessons at session start for relevant project

### 4. Verification Before Done

- **Always run `cargo fmt --all` after every change** — code must be formatted
- **Always run `cargo clippy --workspace --all-targets -- -D warnings` after every change** — zero warnings allowed
- **Always run `cargo build --release`** — must compile cleanly
- All three checks must pass before marking any task complete
- Never mark a task complete without proving it works
- Diff behavior between main and your changes when relevant
- Ask yourself: "Would a staff engineer approve this?"
- Run tests, check logs, demonstrate correctness

### 5. Demand Elegance (Balanced)

- For non-trivial changes: pause and ask "is there a more elegant way?"
- If a fix feels hacky: "Knowing everything I know now, implement the elegant solution"
- Skip this for simple, obvious fixes – don't over-engineer
- Challenge your own work before presenting it

### 6. Autonomous Bug Fixing

- When given a bug report: just fix it. Don't ask for hand-holding
- Point at logs, errors, failing tests – then resolve them
- Zero context switching required from the user
- Go fix failing CI tests without being told how

## Task Management

1. **Plan First**: Write plan to `tasks/todo.md` with checkable items
2. **Verify Plan**: Check in before starting implementation
3. **Track Progress**: Mark items complete as you go
4. **Explain Changes**: High-level summary at each step
5. **Document Results**: Add review section to `tasks/todo.md`
6. **Capture Lessons**: Update `tasks/lessons.md` after corrections

## Core Principles

- **Simplicity First**: Make every change as simple as possible. Impact minimal code.
- **No Laziness**: Find root causes. No temporary fixes. Senior developer standards.
- **Minimal Impact**: Changes should only touch what's necessary. Avoid introducing bugs.
