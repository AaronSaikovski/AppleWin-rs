//! Ensoniq DOC 5503 wavetable synthesizer.
//!
//! The IIgs DOC has 32 oscillators, each reading 8-bit unsigned PCM samples
//! from 64KB of dedicated sound RAM at a programmable frequency.
//!
//! Access is through the Sound GLU registers:
//! - $C03C: Sound control / address register
//! - $C03D: Sound data register
//! - $C03E: Address low pointer
//! - $C03F: Address high pointer
//!
//! DOC internal register layout (per oscillator, 32 oscillators):
//! - $00-$1F: Frequency low (oscillators 0-31)
//! - $20-$3F: Frequency high
//! - $40-$5F: Volume
//! - $60-$7F: Waveform data (current sample position)
//! - $80-$9F: Waveform pointer
//! - $A0-$BF: Control (mode, halt, interrupt enable)
//! - $C0-$DF: Table size (resolution of wavetable)
//! - $E0:     Oscillator enable count (number of active oscillators)
//! - $E1:     A/D converter (not used in IIgs)

use serde::{Deserialize, Serialize};

/// Oscillator control mode bits.
const CTRL_HALT: u8 = 0x01; // Oscillator halted
const CTRL_MODE_MASK: u8 = 0x06; // Mode select (bits 1-2)
const CTRL_IE: u8 = 0x08; // Interrupt enable

/// Oscillator mode values (bits 1-2 of control register).
#[allow(dead_code)]
const MODE_FREE_RUN: u8 = 0x00; // Free-running
#[allow(dead_code)]
const MODE_ONE_SHOT: u8 = 0x02; // One-shot (halt at end)
#[allow(dead_code)]
const MODE_SYNC: u8 = 0x04; // Sync with paired oscillator
#[allow(dead_code)]
const MODE_SWAP: u8 = 0x06; // Swap with paired oscillator

/// DOC internal clock rate: 7.159 MHz / 8 = ~894.886 KHz
/// Each oscillator updates every N+2 clocks (N = number of enabled oscillators)
/// So with all 32 enabled, each oscillator updates at 894886 / 34 ≈ 26,320 Hz
const DOC_CLOCK_HZ: f64 = 894_886.0;

/// Ensoniq DOC 5503 state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ensoniq {
    /// DOC internal registers (256 bytes).
    #[serde(with = "serde_bytes")]
    pub regs: Vec<u8>,

    /// 64KB dedicated sound RAM.
    pub sound_ram: Vec<u8>,

    /// Current GLU address pointer (for register/RAM access).
    pub address: u16,

    /// Sound control register ($C03C).
    /// Bit 7: 1 = access sound RAM, 0 = access DOC registers
    /// Bit 6: auto-increment address
    /// Bit 5: busy flag (read-only)
    /// Bits 4-0: reserved
    pub control: u8,

    /// Oscillator accumulator positions (24-bit fractional, per oscillator).
    accum: [u32; 32],

    /// IRQ pending flag.
    pub irq_pending: bool,

    /// Number of enabled oscillators (from register $E0).
    enabled_count: u8,
}

impl Default for Ensoniq {
    fn default() -> Self {
        let mut regs = vec![0u8; 256];
        // All oscillators halted by default
        for i in 0..32 {
            regs[0xA0 + i] = CTRL_HALT;
        }
        // 1 oscillator enabled by default (minimum)
        regs[0xE0] = 0x00;

        Self {
            regs,
            sound_ram: vec![0u8; 65536],
            address: 0,
            control: 0,
            accum: [0u32; 32],
            irq_pending: false,
            enabled_count: 2,
        }
    }
}

impl Ensoniq {
    /// Write to sound control register ($C03C).
    pub fn write_control(&mut self, val: u8) {
        self.control = val & 0xE0; // only bits 5-7 are meaningful
    }

    /// Read sound control register ($C03C).
    pub fn read_control(&self) -> u8 {
        self.control & 0x60 // busy flag + auto-increment, clear bit 7 on read
    }

    /// Write to sound data register ($C03D).
    pub fn write_data(&mut self, val: u8) {
        if self.control & 0x80 != 0 {
            // Access sound RAM
            let addr = self.address as usize;
            if addr < self.sound_ram.len() {
                self.sound_ram[addr] = val;
            }
        } else {
            // Access DOC registers
            let reg = (self.address & 0xFF) as usize;
            self.regs[reg] = val;

            // Update enabled count when register $E0 is written.
            // Bits 4-1 encode (N/2 - 1) where N is the number of active oscillators.
            // So value 0x00 = 2 osc, 0x02 = 4 osc, ..., 0x1E = 32 osc.
            if reg == 0xE0 {
                self.enabled_count = (((val >> 1) & 0x0F) + 1) * 2;
                self.enabled_count = self.enabled_count.clamp(2, 32);
            }
        }

        // Auto-increment address if bit 6 is set
        if self.control & 0x40 != 0 {
            self.address = self.address.wrapping_add(1);
        }
    }

    /// Read from sound data register ($C03D).
    pub fn read_data(&mut self) -> u8 {
        let val = if self.control & 0x80 != 0 {
            // Access sound RAM
            let addr = self.address as usize;
            if addr < self.sound_ram.len() {
                self.sound_ram[addr]
            } else {
                0
            }
        } else {
            // Access DOC registers
            let reg = (self.address & 0xFF) as usize;
            self.regs[reg]
        };

        if self.control & 0x40 != 0 {
            self.address = self.address.wrapping_add(1);
        }

        val
    }

    /// Write address low byte ($C03E).
    pub fn write_addr_lo(&mut self, val: u8) {
        self.address = (self.address & 0xFF00) | val as u16;
    }

    /// Read address low byte ($C03E).
    pub fn read_addr_lo(&self) -> u8 {
        self.address as u8
    }

    /// Write address high byte ($C03F).
    pub fn write_addr_hi(&mut self, val: u8) {
        self.address = (self.address & 0x00FF) | ((val as u16) << 8);
    }

    /// Read address high byte ($C03F).
    pub fn read_addr_hi(&self) -> u8 {
        (self.address >> 8) as u8
    }

    /// Generate audio samples into the output buffer.
    ///
    /// `out`: Output buffer (mono f32 samples, -1.0 to 1.0).
    /// `sample_rate`: Host audio sample rate (e.g., 44100).
    /// `cpu_cycles`: Number of CPU cycles elapsed since last call.
    pub fn fill_audio(&mut self, out: &mut [f32], sample_rate: u32, _cpu_cycles: u64) {
        if out.is_empty() {
            return;
        }

        let num_osc = self.enabled_count as usize;
        if num_osc == 0 {
            out.fill(0.0);
            return;
        }

        // Each oscillator is clocked at DOC_CLOCK_HZ / (num_osc + 2).
        // For each output sample, we need to advance by that many DOC clocks.
        let doc_clocks_per_sample = DOC_CLOCK_HZ / sample_rate as f64;
        // Each oscillator sees doc_clocks_per_sample / (num_osc + 2) updates.
        let updates_per_sample = doc_clocks_per_sample / (num_osc as f64 + 2.0);

        for sample in out.iter_mut() {
            let mut mix: f32 = 0.0;
            let mut active_count = 0;

            for osc in 0..num_osc.min(32) {
                let ctrl = self.regs[0xA0 + osc];

                // Skip halted oscillators
                if ctrl & CTRL_HALT != 0 {
                    continue;
                }

                let freq_lo = self.regs[osc] as u32;
                let freq_hi = self.regs[0x20 + osc] as u32;
                let freq = (freq_hi << 8) | freq_lo;
                let volume = self.regs[0x40 + osc] as f32 / 255.0;
                let wave_ptr = self.regs[0x80 + osc] as u32;
                let table_size_reg = self.regs[0xC0 + osc];

                // Table size: 256 << (table_size_reg & 0x07) bytes
                let table_shift = (table_size_reg & 0x07) as u32;
                let table_size = 256u32 << table_shift;
                let table_mask = table_size - 1;

                // Accumulator: 16-bit frequency added per DOC update.
                // Scale by updates_per_sample to get the increment per output sample.
                let accum = &mut self.accum[osc];
                let increment = (freq as f64 * updates_per_sample) as u32;
                *accum = accum.wrapping_add(increment);

                // The accumulator's upper bits index into the wavetable.
                // Resolution depends on table_size: for a 256-byte table, use
                // bits 15-8 of the accumulator. For larger tables, use more bits.
                let pos = ((*accum >> (16 - table_shift)) & table_mask) as usize;
                let ram_addr = ((wave_ptr as usize) << 8) + pos;

                // Read sample from sound RAM (unsigned 8-bit, center at 128)
                let raw = if ram_addr < self.sound_ram.len() {
                    self.sound_ram[ram_addr]
                } else {
                    128
                };

                // Check for one-shot mode: halt when we hit a zero byte
                let mode = ctrl & CTRL_MODE_MASK;
                if mode == MODE_ONE_SHOT && raw == 0 {
                    self.regs[0xA0 + osc] |= CTRL_HALT;
                    if ctrl & CTRL_IE != 0 {
                        self.irq_pending = true;
                    }
                    continue;
                }

                // Convert to signed float (-1.0 to 1.0) and apply volume
                let signed = (raw as f32 - 128.0) / 128.0;
                mix += signed * volume;
                active_count += 1;
            }

            // Normalize by number of active oscillators (avoid division by zero)
            if active_count > 0 {
                *sample = mix / (active_count as f32).sqrt();
            } else {
                *sample = 0.0;
            }
        }
    }
}
