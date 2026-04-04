//! Top-level emulator state.
//!
//! Collapses all ~52 globals from the C++ codebase into one owned struct.
//! Mirrors the architecture section "Global State → Owned State" in the plan.

use serde::{Deserialize, Serialize};
use crate::bus::{Bus, BusSnapshot};
use crate::cpu::cpu6502::{Cpu, CpuSnapshot};
use crate::cpu::dispatch;
use crate::model::{Apple2Model, CpuType};

/// Run mode, matching `g_nAppMode` / `AppMode_e` in the C++ source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AppMode {
    /// Startup/logo screen.
    #[default]
    Logo,
    /// Normal execution.
    Running,
    /// Single-step / debugger active.
    Stepping,
    /// Benchmark mode.
    Benchmark,
}

/// The complete emulator state.
pub struct Emulator {
    pub cpu:   Cpu,
    pub bus:   Bus,
    pub model: Apple2Model,
    pub mode:  AppMode,
}

impl Emulator {
    /// Create a new emulator with the given ROM image and model.
    pub fn new(rom: Vec<u8>, model: Apple2Model, cpu_type: CpuType) -> Self {
        let is_65c02 = cpu_type == CpuType::Cpu65C02;
        let mut cpu = Cpu::new(is_65c02);
        let mut bus = Bus::new(rom);
        cpu.reset(&mut bus);
        Self {
            cpu,
            bus,
            model,
            mode: AppMode::Logo,
        }
    }

    /// Execute exactly `cycles` clock cycles.
    /// Returns the number of cycles actually executed (may overshoot by up to
    /// the longest instruction — 7 cycles for BRK).
    pub fn execute(&mut self, cycles: u64) -> u64 {
        // Jammed CPUs just advance time — check once, outside the hot loop.
        if self.cpu.jammed {
            self.cpu.cycles += cycles;
            return cycles;
        }
        let start = self.cpu.cycles;
        let target = start + cycles;
        let mut next_update = start + 17_030; // one NTSC frame worth of cycles
        while self.cpu.cycles < target {
            // Snapshot the IRQ line *before* executing the instruction so we can
            // detect an edge (IRQ asserted during this opcode's last cycle).
            let irq_before = self.bus.irq_line;

            // Sync card IRQ line to CPU before each instruction.
            // Apply 6502 IRQ deferral: if IRQ first appeared on the last cycle of
            // the previous opcode (g_irqOnLastOpcodeCycle / g_irqDefer1Opcode in
            // AppleWin C++), skip taking the interrupt this one opcode.
            if self.bus.irq_line && !self.cpu.flags.contains(super::cpu::Flags::I) {
                if self.cpu.irq_defer {
                    // Second consecutive IRQ assertion — clear defer, take it.
                    self.cpu.irq_defer = false;
                    self.cpu.irq_pending |= 0x01;
                } else {
                    // IRQ is asserted; will decide after the instruction whether to defer.
                    self.cpu.irq_pending |= 0x01;
                }
            } else if !self.bus.irq_line {
                // IRQ line deasserted — clear pending and defer.
                // When I flag is set but irq_line is still asserted, keep
                // irq_pending so it fires when I is cleared (CLI/RTI).
                self.cpu.irq_pending &= !0x01;
                self.cpu.irq_defer = false;
            }

            dispatch::step(&mut self.cpu, &mut self.bus);

            // If IRQ was NOT asserted before but IS asserted after, it appeared on
            // the last cycle of this opcode → defer by one opcode (if I flag is clear).
            if !irq_before && self.bus.irq_line
                && !self.cpu.flags.contains(super::cpu::Flags::I)
            {
                // Clear pending so we don't take it immediately next opcode.
                self.cpu.irq_pending &= !0x01;
                self.cpu.irq_defer = true;
            }
            if self.cpu.cycles >= next_update {
                self.bus.cards.update_all(self.cpu.cycles);
                next_update += 17_030;
            }
        }
        self.cpu.cycles - start
    }

    /// Execute one instruction and return cycles consumed.
    pub fn step(&mut self) -> u8 {
        dispatch::step(&mut self.cpu, &mut self.bus)
    }

    /// Hard reset (power cycle).
    pub fn reset(&mut self, power_cycle: bool) {
        if power_cycle {
            self.bus.main_ram.fill(0);
            self.bus.aux_ram.fill(0);
        }
        // Reset memory soft-switches so the ROM is mapped at $D000-$FFFF before
        // reading the reset vector.  On real hardware the ROM is always accessible
        // during the vector fetch regardless of language-card state.
        self.bus.mode = crate::bus::MemMode::empty();
        self.bus.rebuild_page_tables();
        self.bus.cards.reset_all(power_cycle);
        self.bus.speaker_toggles.clear();
        self.cpu.reset(&mut self.bus);
        self.mode = AppMode::Running;
    }
}

/// Full emulator snapshot for save states.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmulatorSnapshot {
    pub version: u32,
    pub model:   Apple2Model,
    pub cpu:     CpuSnapshot,
    pub memory:  BusSnapshot,
}

impl Emulator {
    pub fn take_snapshot(&self) -> EmulatorSnapshot {
        EmulatorSnapshot {
            version: 1,
            model:   self.model,
            cpu:     CpuSnapshot::from(&self.cpu),
            memory:  self.bus.take_snapshot(),
        }
    }

    pub fn restore_snapshot(&mut self, snap: &EmulatorSnapshot) {
        self.model = snap.model;
        self.cpu.restore_snapshot(&snap.cpu);
        self.bus.restore_snapshot(&snap.memory);
    }
}
