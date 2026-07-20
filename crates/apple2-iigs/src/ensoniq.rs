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
//! - $E0:     Oscillator interrupt register (which oscillator raised an IRQ)
//! - $E1:     Oscillator enable register (number of active oscillators - 1) << 1
//! - $E2:     A/D converter (reads 0x80 on the IIgs)

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

    /// Sound control register ($C03C), Sound GLU layout (per KEGS `doc.c`):
    /// - Bit 7: busy flag (read-only)
    /// - Bit 6: 1 = access sound RAM, 0 = access DOC registers
    /// - Bit 5: auto-increment address after each data access
    /// - Bits 4-0: master DOC output volume (only bits 3-0 used)
    pub control: u8,

    /// Oscillator accumulator positions (24-bit fractional, per oscillator).
    accum: [u32; 32],

    /// IRQ pending flag.
    pub irq_pending: bool,

    /// Number of enabled oscillators (from register $E1).
    enabled_count: u8,
}

impl Default for Ensoniq {
    fn default() -> Self {
        let mut regs = vec![0u8; 256];
        // All oscillators halted by default
        for i in 0..32 {
            regs[0xA0 + i] = CTRL_HALT;
        }
        // DOC oscillator-interrupt register ($E0): 0xFF means "no interrupt
        // pending" (bit 7 set). A pending interrupt is `osc << 1` with bit 7
        // clear. Defaulting this to 0x00 makes the ROM's IRQ dispatcher read a
        // phantom oscillator-0 interrupt it cannot claim, producing the fatal
        // "Unclaimed Sound Interrupt" that freezes boot. (Matches KEGS
        // `doc_reg_e0 = 0xff`.)
        regs[0xE0] = 0xFF;

        Self {
            regs,
            // Initialise sound RAM to the mid-point (128). An 8-bit DOC sample
            // is centred at 128, so an oscillator that is un-halted before its
            // waveform is loaded reads silence (0) rather than a full-scale
            // −1.0 DC level, which otherwise buzzes loudly.
            sound_ram: vec![128u8; 65536],
            address: 0,
            control: 0,
            accum: [0u32; 32],
            irq_pending: false,
            enabled_count: 2,
        }
    }
}

impl Ensoniq {
    /// `$C03C` bit 6: 1 = access sound RAM, 0 = access DOC registers.
    const CTL_RAM_SELECT: u8 = 0x40;
    /// `$C03C` bit 5: auto-increment the address pointer after each data access.
    const CTL_AUTO_INC: u8 = 0x20;

    /// Write to sound control register ($C03C).
    ///
    /// The full byte is retained: bits 3-0 are the master DOC output volume,
    /// bit 6 selects RAM vs. registers, bit 5 enables auto-increment.
    pub fn write_control(&mut self, val: u8) {
        self.control = val;
    }

    /// Read sound control register ($C03C). Returns the full register (KEGS
    /// `doc_read_c03c` returns `g_doc_sound_ctl` unmodified).
    pub fn read_control(&self) -> u8 {
        self.control
    }

    /// Write to sound data register ($C03D).
    pub fn write_data(&mut self, val: u8) {
        if self.control & Self::CTL_RAM_SELECT != 0 {
            // Access sound RAM
            let addr = self.address as usize;
            if addr < self.sound_ram.len() {
                self.sound_ram[addr] = val;
            }
        } else {
            // Access DOC registers
            let reg = (self.address & 0xFF) as usize;
            self.regs[reg] = val;

            // Update the active-oscillator count when the DOC oscillator-enable
            // register ($E1) is written. ($E0 is the interrupt register.) The
            // value is `(number_of_oscillators - 1) << 1`, so the count is
            // `((val >> 1) & 0x1F) + 1`, clamped to 1..=32.
            if reg == 0xE1 {
                self.enabled_count = (((val >> 1) & 0x1F) + 1).clamp(1, 32);
            }
        }

        // Auto-increment address if enabled (bit 5).
        if self.control & Self::CTL_AUTO_INC != 0 {
            self.address = self.address.wrapping_add(1);
        }
    }

    /// Read from sound data register ($C03D).
    pub fn read_data(&mut self) -> u8 {
        let val = if self.control & Self::CTL_RAM_SELECT != 0 {
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
            match reg {
                // $E0: oscillator-interrupt register. Return the pending value;
                // reading it clears the interrupt (bit 7 clear = pending). Per
                // KEGS `doc_read_c03d` / `doc_remove_sound_irq`.
                0xE0 => {
                    let v = self.regs[0xE0];
                    if v & 0x80 == 0 {
                        self.regs[0xE0] = 0xFF;
                        self.irq_pending = false;
                    }
                    v
                }
                // $E1: oscillator-enable register reads back as (n-1) << 1.
                0xE1 => (self.enabled_count.saturating_sub(1)) << 1,
                // $E2: A/D converter — always reads 0x80 on the IIgs DOC.
                0xE2 => 0x80,
                _ => self.regs[reg],
            }
        };

        if self.control & Self::CTL_AUTO_INC != 0 {
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

    /// Fixed-point fractional bits for the oscillator phase accumulator, matching
    /// KEGS' `SND_PTR_SHIFT`. The byte index into sound RAM is `accum >> SHIFT`.
    const SND_PTR_SHIFT: u32 = 14;

    /// End-of-pass handler for oscillator `osc` (it read a `$00` byte or ran off
    /// the end of its wavetable). Mirrors KEGS `doc_sound_end`: free-running
    /// oscillators loop when they reach the end without a zero byte; one-shot and
    /// sync oscillators halt; swap-mode oscillators halt and start their partner.
    /// An end-of-pass raises an IRQ when the oscillator's interrupt-enable bit set.
    fn end_oscillator(&mut self, osc: usize, hit_zero: bool, size_mask: u32) {
        let ctrl = self.regs[0xA0 + osc];
        if ctrl & CTRL_IE != 0 {
            self.irq_pending = true;
            // Record the interrupting oscillator in the $E0 register (bit 7
            // clear = pending, bits 5-1 = oscillator number) so the ROM's sound
            // handler can identify and claim it. Only latch if none is already
            // pending, matching KEGS' single-level `doc_reg_e0` behaviour.
            if self.regs[0xE0] & 0x80 != 0 {
                self.regs[0xE0] = (osc as u8) << 1;
            }
        }
        let mode = ctrl & CTRL_MODE_MASK;
        let other = osc ^ 1;
        let omode = self.regs[0xA0 + other] & CTRL_MODE_MASK;

        if mode == MODE_FREE_RUN && !hit_zero {
            // Free-running, reached the table end without a zero byte — loop by
            // wrapping the phase accumulator within the table (preserving the
            // fractional phase), not resetting to 0. Resetting produced a phase
            // discontinuity — an audible click — at every loop boundary.
            self.accum[osc] &= size_mask;
        } else if mode == MODE_SWAP || omode == MODE_SWAP {
            // Swap: halt this oscillator and (re)start the partner from the top.
            self.regs[0xA0 + osc] |= CTRL_HALT;
            self.regs[0xA0 + other] &= !CTRL_HALT;
            self.accum[other] = 0;
        } else {
            // One-shot / sync / free-run-hit-zero: halt.
            self.regs[0xA0 + osc] |= CTRL_HALT;
        }
    }

    /// Generate audio samples into the output buffer.
    ///
    /// `out`: Output buffer (mono f32 samples, -1.0 to 1.0).
    /// `sample_rate`: Host audio sample rate (e.g., 44100).
    /// `cpu_cycles`: Number of CPU cycles elapsed since last call.
    ///
    /// The oscillator model follows the Ensoniq DOC 5503 as emulated by KEGS: a
    /// size-aligned wavetable pointer, a fixed-point phase accumulator, and the
    /// defining behaviour that a `$00` sample byte terminates the current pass in
    /// **every** mode (not just one-shot). Missing that last rule made free-running
    /// oscillators play straight through the zero terminator into whatever RAM
    /// followed — the source of the buzzing.
    pub fn fill_audio(&mut self, out: &mut [f32], sample_rate: u32, _cpu_cycles: u64) {
        if out.is_empty() {
            return;
        }

        let num_osc = (self.enabled_count as usize).clamp(1, 32);
        // DOC oscillator update rate divides the master clock by (osc_en + 2).
        let rate_scale = DOC_CLOCK_HZ / sample_rate as f64 / (num_osc as f64 + 2.0);

        // Master DOC output volume ($C03C bits 3-0), scaled 0..1. KEGS multiplies
        // every oscillator sample by `g_doc_vol`; without this the sound GLU
        // master volume control is inert.
        let master_vol = (self.control & 0x0F) as f32 / 15.0;

        for sample in out.iter_mut() {
            let mut mix: f32 = 0.0;
            let mut active_count = 0;

            for osc in 0..num_osc {
                let ctrl = self.regs[0xA0 + osc];
                if ctrl & CTRL_HALT != 0 {
                    continue;
                }

                let freq = ((self.regs[0x20 + osc] as u32) << 8) | self.regs[osc] as u32;
                let volume = self.regs[0x40 + osc] as f32 / 255.0;
                let wave_ptr = self.regs[0x80 + osc] as u32;
                let wave_size = self.regs[0xC0 + osc];

                // Table size = 2^sz bytes, sz = ((wavesize>>3)&7)+8 → 256..32768.
                // Resolution `res` (low 3 bits) scales the phase increment.
                let sz = (((wave_size >> 3) & 7) + 8) as u32;
                let res = (wave_size & 7) as i32;
                let size = 1u32 << sz; // bytes
                // Wave pointer is aligned down to the table size.
                let start_byte = (wave_ptr << 8) & !(size - 1);

                // Phase increment per output sample, in `SND_PTR_SHIFT` fixed point.
                let inc = (freq as f64 * rate_scale * 2f64.powi(sz as i32 - res - 3)) as u32;
                if inc == 0 {
                    continue; // not advancing — silent
                }

                let accum = self.accum[osc];
                let byte_off = accum >> Self::SND_PTR_SHIFT;
                let pos = ((start_byte + byte_off) & 0xFFFF) as usize;
                let raw = self.sound_ram[pos];

                let next = accum.wrapping_add(inc);
                self.accum[osc] = next;
                let end = (next >> Self::SND_PTR_SHIFT) >= size;

                if raw == 0 || end {
                    // A zero byte is the DOC's universal wavetable terminator.
                    let size_mask = (size << Self::SND_PTR_SHIFT).wrapping_sub(1);
                    self.end_oscillator(osc, raw == 0, size_mask);
                    if raw == 0 {
                        // The zero byte itself is not emitted.
                        continue;
                    }
                }

                let signed = (raw as f32 - 128.0) / 128.0;
                mix += signed * volume;
                active_count += 1;
            }

            *sample = if active_count > 0 {
                (mix / (active_count as f32).sqrt()) * master_vol
            } else {
                0.0
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn osc0(mode: u8) -> Ensoniq {
        let mut d = Ensoniq::default();
        d.regs[0x00] = 100; // freq low → advances ~1 byte/sample
        d.regs[0x20] = 0; // freq high
        d.regs[0x40] = 255; // full volume
        d.regs[0x80] = 0; // wave pointer → table at $0000
        d.regs[0xC0] = 0x00; // sz=8 (256-byte table), res=0
        d.regs[0xA0] = mode; // control: mode, not halted
        d.accum[0] = 0;
        d
    }

    #[test]
    fn osc_int_register_defaults_to_no_pending() {
        // $E0 must read 0xFF (bit 7 set = no interrupt) at power-on, otherwise
        // the ROM's IRQ dispatcher sees a phantom osc-0 interrupt and prints
        // the fatal "Unclaimed Sound Interrupt".
        let mut d = Ensoniq::default();
        d.write_control(0x00); // register access, no auto-increment
        d.write_addr_lo(0xE0);
        assert_eq!(d.read_data(), 0xFF);
    }

    #[test]
    fn osc_int_register_latches_and_clears_on_read() {
        // An interrupt-enabled oscillator that ends latches its number into $E0
        // (bit 7 clear), and reading $E0 clears the pending interrupt back to
        // 0xFF so the ROM can claim exactly one interrupt.
        let mut d = osc0(MODE_ONE_SHOT);
        d.regs[0xA0] = MODE_ONE_SHOT | CTRL_IE; // enable interrupt, not halted
        d.sound_ram[5] = 0x00; // zero terminator → one-shot ends
        let mut buf = [0.0f32; 64];
        d.fill_audio(&mut buf, 44_100, 0);

        d.write_control(0x00);
        d.write_addr_lo(0xE0);
        let pending = d.read_data();
        assert_eq!(
            pending & 0x80,
            0,
            "an interrupt must be pending (bit 7 clear)"
        );
        assert_eq!(pending >> 1, 0, "osc 0 must be the interrupting oscillator");

        d.write_addr_lo(0xE0);
        assert_eq!(d.read_data(), 0xFF, "reading $E0 must clear the interrupt");
    }

    #[test]
    fn osc_enable_register_reads_back() {
        // $E1 reads back as (num_osc_en - 1) << 1; $E2 (A/D) reads 0x80.
        let mut d = Ensoniq::default();
        d.write_control(0x00);
        d.write_addr_lo(0xE1);
        d.write_data(0x0A); // (val>>1 & 0x1F)+1 = 6 oscillators
        d.write_addr_lo(0xE1);
        assert_eq!(d.read_data(), (6 - 1) << 1);
        d.write_addr_lo(0xE2);
        assert_eq!(d.read_data(), 0x80);
    }

    #[test]
    fn one_shot_oscillator_halts_on_zero_byte() {
        let mut d = osc0(MODE_ONE_SHOT);
        // Zero terminator a few bytes in; earlier bytes are non-zero (128).
        d.sound_ram[5] = 0x00;
        let mut buf = [0.0f32; 64];
        d.fill_audio(&mut buf, 44_100, 0);
        assert!(
            d.regs[0xA0] & CTRL_HALT != 0,
            "one-shot oscillator must halt when it reads a $00 sample byte"
        );
    }

    #[test]
    fn free_run_oscillator_loops_not_halts_without_zero() {
        // No zero bytes anywhere (all 128) → free-run reaches the end and loops,
        // never halting. This is the behaviour that stops garbage playback/buzz.
        let mut d = osc0(MODE_FREE_RUN);
        let mut buf = [0.0f32; 4096];
        d.fill_audio(&mut buf, 44_100, 0);
        assert_eq!(
            d.regs[0xA0] & CTRL_HALT,
            0,
            "free-running oscillator must loop at the table end, not halt"
        );
    }

    #[test]
    fn zero_filled_ram_stays_silent() {
        // A zero terminator at the very first byte must yield pure silence, never
        // a full-scale −1.0 DC spike (the classic DOC buzz).
        let mut d = osc0(MODE_FREE_RUN);
        d.sound_ram.iter_mut().for_each(|b| *b = 0);
        let mut buf = [0.5f32; 128];
        d.fill_audio(&mut buf, 44_100, 0);
        assert!(
            buf.iter().all(|&s| s == 0.0),
            "an oscillator over zero-filled RAM must be silent"
        );
    }
}
