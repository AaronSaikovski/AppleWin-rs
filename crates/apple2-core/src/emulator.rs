//! Top-level emulator state.
//!
//! Collapses all ~52 globals from the C++ codebase into one owned struct.
//! Mirrors the architecture section "Global State → Owned State" in the plan.

use crate::bus::{Bus, BusSnapshot};
use crate::cpu::cpu6502::{Cpu, CpuSnapshot};
use crate::cpu::dispatch;
use crate::model::{Apple2Model, CpuType};
use serde::{Deserialize, Serialize};

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
    pub cpu: Cpu,
    pub bus: Bus,
    pub model: Apple2Model,
    pub mode: AppMode,
    /// Next CPU cycle at which to emit a trace log line (debug diagnostic).
    next_trace_log: u64,
    /// Whether we've already dumped the game loop code (one-shot).
    code_dumped: bool,
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
            next_trace_log: 50_000_000,
            code_dumped: false,
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

        // Caller trace: log first 5 hits of $9C06 with stack dump
        let mut caller_logged: u32 = 0;

        while self.cpu.cycles < target {
            // ── Caller trace: dump return addresses when entering $9C06 ──
            if self.cpu.pc == 0x9C06 && caller_logged < 5 && !self.bus.disk_motor_on() {
                caller_logged += 1;
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("cpu_trace.log")
                {
                    // Dump 32 bytes of stack (return addresses)
                    let mut stack = [0u8; 32];
                    for (i, b) in stack.iter_mut().enumerate() {
                        *b = self.bus.read(
                            0x0100 + ((self.cpu.sp as u16 + 1 + i as u16) & 0xFF),
                            self.cpu.cycles,
                        );
                    }
                    // Also dump the outer code: 64 bytes starting from each return address
                    let ret1 = stack[0] as u16 | ((stack[1] as u16) << 8);
                    let ret2 = stack[2] as u16 | ((stack[3] as u16) << 8);
                    let mut outer1 = [0u8; 64];
                    let mut outer2 = [0u8; 64];
                    for i in 0..64u16 {
                        outer1[i as usize] = self.bus.read(ret1.wrapping_add(i), self.cpu.cycles);
                        outer2[i as usize] = self.bus.read(ret2.wrapping_add(i), self.cpu.cycles);
                    }
                    let _ = writeln!(
                        f,
                        "[CALLER] S=${:02X} cyc={} stack={:02X?}",
                        self.cpu.sp, self.cpu.cycles, stack
                    );
                    let _ = writeln!(f, "[CALLER] ret1=${:04X}: {:02X?}", ret1, outer1);
                    let _ = writeln!(f, "[CALLER] ret2=${:04X}: {:02X?}", ret2, outer2);
                }
            }

            // ── Periodic PC logger (persistent across execute() calls) ───
            if self.cpu.cycles >= self.next_trace_log {
                use std::io::Write;
                let pc = self.cpu.pc;
                let opcode = self.bus.read(pc, self.cpu.cycles);
                let b1 = self.bus.read(pc.wrapping_add(1), self.cpu.cycles);
                let b2 = self.bus.read(pc.wrapping_add(2), self.cpu.cycles);
                let motor = self.bus.disk_motor_on();
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("cpu_trace.log")
                {
                    let _ = writeln!(
                        f,
                        "[TRACE] PC=${:04X} A=${:02X} X=${:02X} Y={:02X} S=${:02X} P=${:02X} op={:02X} {:02X} {:02X} motor={} cyc={}",
                        pc,
                        self.cpu.a,
                        self.cpu.x,
                        self.cpu.y,
                        self.cpu.sp,
                        self.cpu.flags.bits(),
                        opcode,
                        b1,
                        b2,
                        if motor { "ON" } else { "off" },
                        self.cpu.cycles
                    );
                }
                // Dump game loop code once when PC enters the $9Cxx range
                if !self.code_dumped && (0x9C00..0x9E00).contains(&pc) && !motor {
                    self.code_dumped = true;
                    if let Ok(mut df) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open("cpu_trace.log")
                    {
                        for &(sa, len) in &[(0x9400u16, 0x120), (0x9C00, 0x120), (0xA500, 0x100)] {
                            let mut chunk = Vec::with_capacity(len);
                            for i in 0..len {
                                chunk.push(self.bus.read(sa + i as u16, self.cpu.cycles));
                            }
                            let _ = writeln!(df, "[DUMP] ${:04X}: {:02X?}", sa, chunk);
                        }
                        let mut zp = [0u8; 256];
                        for (i, b) in zp.iter_mut().enumerate() {
                            *b = self.bus.read(i as u16, self.cpu.cycles);
                        }
                        let _ = writeln!(df, "[DUMP] ZP: {:02X?}", &zp[..]);
                    }
                }
                // Use shorter interval (10M) to get more samples during the freeze
                self.next_trace_log = self.cpu.cycles + 10_000_000;
            }
            // 65C02 WAI: CPU is halted until an interrupt arrives.
            // Advance time by 1 cycle per iteration and check for pending IRQ/NMI.
            if self.cpu.waiting {
                if self.bus.irq_line || (self.cpu.irq_pending & 0x02) != 0 {
                    // Interrupt arrived — wake up and resume normal execution.
                    self.cpu.waiting = false;
                } else {
                    // Still waiting — consume one cycle and continue polling.
                    self.cpu.cycles += 1;
                    if self.cpu.cycles >= next_update {
                        self.bus.cards.update_all(self.cpu.cycles);
                        next_update += 17_030;
                    }
                    continue;
                }
            }
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
            } else {
                self.cpu.irq_pending &= !0x01;
                self.cpu.irq_defer = false;
            }

            dispatch::step(&mut self.cpu, &mut self.bus);

            // If IRQ was NOT asserted before but IS asserted after, it appeared on
            // the last cycle of this opcode → defer by one opcode (if I flag is clear).
            if !irq_before && self.bus.irq_line && !self.cpu.flags.contains(super::cpu::Flags::I) {
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
    ///
    /// `mem_init_pattern` selects the RAM fill pattern (0–7).
    /// The C++ AppleWin calls these "Memory Initialization Patterns" (MIP).
    pub fn reset_with_pattern(&mut self, power_cycle: bool, mem_init_pattern: u8) {
        if power_cycle {
            init_memory_pattern(&mut self.bus.main_ram, mem_init_pattern);
            init_memory_pattern(&mut self.bus.aux_ram, mem_init_pattern);
        }
        self.finish_reset(power_cycle);
    }

    /// Hard reset (power cycle) — pattern 0 (all zeros).
    pub fn reset(&mut self, power_cycle: bool) {
        if power_cycle {
            self.bus.main_ram.fill(0);
            self.bus.aux_ram.fill(0);
        }
        self.finish_reset(power_cycle);
    }

    /// Common reset tail shared by `reset()` and `reset_with_pattern()`.
    fn finish_reset(&mut self, power_cycle: bool) {
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

// ── Memory Initialization Patterns ───────────────────────────────────────────

/// Fill 64K RAM with one of the 8 Memory Initialization Patterns (MIP)
/// from the C++ AppleWin `Memory.cpp`.  Pattern 0 is the default (all zeros).
///
/// These patterns emulate the semi-random power-on state of real DRAM chips.
/// Some copy-protected software depends on specific patterns to detect
/// "cold boot" vs "warm boot" or to seed random numbers.
pub fn init_memory_pattern(ram: &mut [u8; 65536], pattern: u8) {
    match pattern {
        0 => ram.fill(0x00),
        1 => ram.fill(0xFF),
        2 => {
            // Alternating 00/FF per page (even pages = 0x00, odd pages = 0xFF).
            for page in 0..256 {
                let fill = if page & 1 == 0 { 0x00 } else { 0xFF };
                let start = page * 256;
                ram[start..start + 256].fill(fill);
            }
        }
        3 => {
            // Alternating FF/00 per page (even pages = 0xFF, odd pages = 0x00).
            for page in 0..256 {
                let fill = if page & 1 == 0 { 0xFF } else { 0x00 };
                let start = page * 256;
                ram[start..start + 256].fill(fill);
            }
        }
        4 => {
            // Alternating 00/FF per 128-byte half-page.
            for (i, byte) in ram.iter_mut().enumerate() {
                *byte = if (i >> 7) & 1 == 0 { 0x00 } else { 0xFF };
            }
        }
        5 => {
            // Alternating FF/00 per 128-byte half-page.
            for (i, byte) in ram.iter_mut().enumerate() {
                *byte = if (i >> 7) & 1 == 0 { 0xFF } else { 0x00 };
            }
        }
        6 => {
            // Pseudo-random pattern seeded from address (matches MIP6 in AppleWin).
            for (i, byte) in ram.iter_mut().enumerate() {
                *byte = ((i as u16).wrapping_mul(0x0101) >> 8) as u8;
            }
        }
        7 => {
            // Inverse pseudo-random.
            for (i, byte) in ram.iter_mut().enumerate() {
                *byte = !((i as u16).wrapping_mul(0x0101) >> 8) as u8;
            }
        }
        _ => ram.fill(0x00),
    }
}

/// Full emulator snapshot for save states.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmulatorSnapshot {
    pub version: u32,
    pub model: Apple2Model,
    pub cpu: CpuSnapshot,
    pub memory: BusSnapshot,
}

impl Emulator {
    pub fn take_snapshot(&self) -> EmulatorSnapshot {
        EmulatorSnapshot {
            version: 1,
            model: self.model,
            cpu: CpuSnapshot::from(&self.cpu),
            memory: self.bus.take_snapshot(),
        }
    }

    pub fn restore_snapshot(&mut self, snap: &EmulatorSnapshot) {
        self.model = snap.model;
        self.cpu.restore_snapshot(&snap.cpu);
        self.bus.restore_snapshot(&snap.memory);
    }
}
