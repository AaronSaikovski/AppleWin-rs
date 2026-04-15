//! Apple IIgs top-level emulator.
//!
//! Owns the 65C816 CPU and IIgs bus, providing the main execution loop
//! with speed-aware cycle counting.

use crate::bus::IIgsBus;
use crate::cpu65816::{self, Cpu65816};
use crate::memory::IIgsMemory;

/// The complete Apple IIgs emulator state.
pub struct IIgsEmulator {
    /// 65C816 CPU.
    pub cpu: Cpu65816,

    /// IIgs memory bus.
    pub bus: IIgsBus,
}

impl IIgsEmulator {
    /// Create a new IIgs emulator.
    ///
    /// `ram_kb`: RAM size in kilobytes (256-8192).
    /// `rom_data`: ROM image loaded from file.
    pub fn new(ram_kb: usize, rom_data: Vec<u8>) -> Result<Self, String> {
        let mem = IIgsMemory::new(ram_kb, rom_data)?;
        let mut bus = IIgsBus::new(mem);
        let mut cpu = Cpu65816::new();
        cpu.reset(&mut bus);

        Ok(Self { cpu, bus })
    }

    /// Execute for approximately `cycles` reference (1 MHz) clock cycles.
    /// Returns the number of reference cycles actually executed.
    pub fn execute(&mut self, cycles: u64) -> u64 {
        if self.cpu.stopped {
            self.cpu.cycles += cycles;
            return cycles;
        }

        let start = self.cpu.cycles;
        let target = start + cycles;
        // Update more frequently (every ~1000 cycles) for responsive ADB/interrupt handling
        let mut next_update = start + 1000;

        while self.cpu.cycles < target {
            // WAI: CPU halted waiting for interrupt
            if self.cpu.waiting {
                self.bus.update_interrupts(self.cpu.cycles);
                if self.bus.irq_line {
                    self.cpu.waiting = false;
                    self.cpu.irq_pending |= 0x01;
                } else {
                    self.cpu.cycles += 1;
                    continue;
                }
            }

            // Sync IRQ line to CPU
            if self.bus.irq_line {
                self.cpu.irq_pending |= 0x01;
            } else {
                self.cpu.irq_pending &= !0x01;
            }

            // Execute one instruction
            cpu65816::step(&mut self.cpu, &mut self.bus);

            // Periodic updates — ADB, interrupts, VBL
            if self.cpu.cycles >= next_update {
                self.bus.update_interrupts(self.cpu.cycles);
                next_update = self.cpu.cycles + 1000;
            }
        }

        self.cpu.cycles - start
    }

    /// Execute one instruction. Returns cycles consumed.
    pub fn step(&mut self) -> u8 {
        cpu65816::step(&mut self.cpu, &mut self.bus)
    }

    /// Reset the emulator (power cycle or warm reset).
    pub fn reset(&mut self, power_cycle: bool) {
        if power_cycle {
            self.bus.mem.clear_ram();
        }
        self.bus.reset(power_cycle);
        self.cpu.reset(&mut self.bus);
    }

    /// Get a reference to the main RAM for video rendering.
    /// Returns the first 128KB (banks $00-$01) which contains the IIe-compatible
    /// display memory.
    pub fn main_ram(&self) -> &[u8] {
        let end = 0x20000.min(self.bus.mem.ram.len());
        &self.bus.mem.ram[..end]
    }

    /// Get a reference to fast RAM (banks $E0-$E1) for SHR rendering.
    pub fn fast_ram(&self) -> &[u8] {
        &self.bus.mem.fast_ram
    }

    /// Process a key press — routes through ADB controller and Mega II.
    pub fn key_press(&mut self, key: u8) {
        // Feed to both ADB (for firmware) and Mega II (for IIe compat)
        self.bus.adb.key_press(key);
        self.bus.mega2.key_press(key);
    }
}
