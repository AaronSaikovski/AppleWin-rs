//! MOS 6522 Versatile Interface Adapter (VIA) emulation.
//!
//! Shared by Mockingboard, Phasor, MegaAudio, and SD Music cards.
//! Each card uses one or two VIA instances to control AY-3-8910 PSG chips
//! via BDIR/BC1 lines encoded in the VIA's ORB register.

use crate::error::Result;
use std::io::{Read, Write};

/// Decode a Mockingboard-family `$Cn00–$CnFF` offset into `(via, register)`.
///
/// Matches the Sweet Micro Systems wiring (and AppleWin's `MB_Read`/`MB_Write`):
/// A7 selects the VIA (0 = `$Cn00`, 1 = `$Cn80`) and A3–A0 the register; A4–A6
/// are not decoded, so each VIA mirrors through its half of the page.
#[inline]
pub fn mb_decode(offset: u8) -> (usize, u8) {
    ((offset >> 7) as usize, offset & 0x0F)
}

/// Earliest enabled timer interrupt across a card's VIAs.
pub fn next_irq_cycle(vias: &[Via6522]) -> u64 {
    vias.iter()
        .map(Via6522::next_irq_cycle)
        .min()
        .unwrap_or(u64::MAX)
}

/// Minimal 6522 VIA model — registers needed for AY control and timer IRQs.
pub struct Via6522 {
    pub ora: u8,  // Output Register A (data bus to AY)
    pub orb: u8,  // Output Register B (BDIR/BC1 control lines)
    pub ddra: u8, // Data Direction Register A
    pub ddrb: u8, // Data Direction Register B
    // Timer registers
    pub t1cl: u8, // T1 counter low
    pub t1ch: u8, // T1 counter high
    pub t1ll: u8, // T1 latch low
    pub t1lh: u8, // T1 latch high
    pub t2cl: u8, // T2 counter low
    pub t2ch: u8, // T2 counter high
    pub sr: u8,   // Shift Register
    pub acr: u8,  // Auxiliary Control Register
    pub pcr: u8,  // Peripheral Control Register
    pub ifr: u8,  // Interrupt Flag Register
    pub ier: u8,  // Interrupt Enable Register
    /// Last CPU cycle count when timers were updated.
    pub last_cycles: u64,
    /// True when T1 is running (armed by write to T1CH).
    pub t1_running: bool,
    /// True when T2 is running (armed by write to T2CH).
    pub t2_running: bool,
}

impl Via6522 {
    pub fn new() -> Self {
        Self {
            ora: 0,
            orb: 0,
            ddra: 0,
            ddrb: 0,
            t1cl: 0,
            t1ch: 0,
            t1ll: 0,
            t1lh: 0,
            t2cl: 0,
            t2ch: 0,
            sr: 0,
            acr: 0,
            pcr: 0,
            ifr: 0,
            ier: 0,
            last_cycles: 0,
            t1_running: false,
            t2_running: false,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Update timer state based on elapsed CPU cycles (lazy evaluation).
    pub fn tick(&mut self, current_cycles: u64) {
        if current_cycles <= self.last_cycles {
            return;
        }
        // Saturate rather than truncate: a timer idle for > 2^32 cycles has
        // expired regardless, and truncation could make it look un-expired.
        let delta = (current_cycles - self.last_cycles).min(u32::MAX as u64) as u32;
        self.last_cycles = current_cycles;

        if self.t1_running {
            let t1 = ((self.t1ch as u32) << 8) | (self.t1cl as u32);
            if delta >= t1 + 2 {
                // T1 expired
                self.ifr |= 0x40; // bit 6 = T1 timeout
                let continuous = self.acr & 0x40 != 0; // ACR bit 6
                if continuous {
                    // Reload from latch and continue.  The real 6522 period is
                    // latch + 2 cycles.  `remaining` cycles have run since the
                    // expiry, i.e. `r = remaining % period` into the current
                    // period, so the counter now reads `latch - r` — which
                    // expires (at count + 2) exactly one period after the last
                    // expiry.  r == latch + 1 is the reload cycle itself; the
                    // counter holds at 0 there (expiry then lands 1 cycle late).
                    let latch = ((self.t1lh as u32) << 8) | (self.t1ll as u32);
                    let remaining = delta - (t1 + 2);
                    let r = remaining % (latch + 2);
                    let new_count = latch.saturating_sub(r);
                    self.t1cl = (new_count & 0xFF) as u8;
                    self.t1ch = ((new_count >> 8) & 0xFF) as u8;
                } else {
                    // One-shot: set to 0xFFFF and stop
                    self.t1cl = 0xFF;
                    self.t1ch = 0xFF;
                    self.t1_running = false;
                }
            } else {
                // delta == t1 + 1 lands in the cycle between reaching 0 and the
                // expiry; hold at 0 (expiry then follows within 2 cycles)
                // instead of underflowing.
                let new_t1 = t1.saturating_sub(delta);
                self.t1cl = (new_t1 & 0xFF) as u8;
                self.t1ch = ((new_t1 >> 8) & 0xFF) as u8;
            }
        }

        if self.t2_running {
            let t2 = ((self.t2ch as u32) << 8) | (self.t2cl as u32);
            if delta >= t2 + 2 {
                self.ifr |= 0x20; // bit 5 = T2 timeout
                self.t2_running = false;
                self.t2cl = 0xFF;
                self.t2ch = 0xFF;
            } else {
                let new_t2 = t2.saturating_sub(delta);
                self.t2cl = (new_t2 & 0xFF) as u8;
                self.t2ch = ((new_t2 >> 8) & 0xFF) as u8;
            }
        }
    }

    /// Absolute CPU cycle at which a timer will next raise an *enabled*
    /// interrupt, or `u64::MAX` if none is pending.
    ///
    /// Mirrors the expiry condition in [`tick`](Self::tick) (`delta >= count + 2`),
    /// so ticking at the returned cycle sets the IFR flag.  Timers whose flag is
    /// already set are skipped: their IRQ is asserted until software clears it.
    pub fn next_irq_cycle(&self) -> u64 {
        let mut next = u64::MAX;
        if self.t1_running && self.ier & 0x40 != 0 && self.ifr & 0x40 == 0 {
            let t1 = ((self.t1ch as u64) << 8) | self.t1cl as u64;
            next = next.min(self.last_cycles + t1 + 2);
        }
        if self.t2_running && self.ier & 0x20 != 0 && self.ifr & 0x20 == 0 {
            let t2 = ((self.t2ch as u64) << 8) | self.t2cl as u64;
            next = next.min(self.last_cycles + t2 + 2);
        }
        next
    }

    /// Read a register.  Like the real 6522, reading T1C-L clears the T1
    /// interrupt flag and reading T2C-L clears the T2 flag — IRQ handlers rely
    /// on this to acknowledge timer interrupts.
    pub fn read(&mut self, reg: u8) -> u8 {
        match reg & 0x0F {
            0x0 => self.orb,
            0x1 | 0xF => self.ora,
            0x2 => self.ddrb,
            0x3 => self.ddra,
            0x4 => {
                self.ifr &= !0x40;
                self.t1cl
            }
            0x5 => self.t1ch,
            0x6 => self.t1ll,
            0x7 => self.t1lh,
            0x8 => {
                self.ifr &= !0x20;
                self.t2cl
            }
            0x9 => self.t2ch,
            0xA => self.sr,
            0xB => self.acr,
            0xC => self.pcr,
            // IFR bit 7: set if any enabled interrupt is active
            0xD => {
                self.ifr
                    | if self.ifr & self.ier & 0x7F != 0 {
                        0x80
                    } else {
                        0x00
                    }
            }
            // IER bit 7 always reads back as 1.
            _ => self.ier | 0x80,
        }
    }

    pub fn write(&mut self, reg: u8, val: u8) {
        match reg & 0x0F {
            0x0 => self.orb = val,
            0x1 | 0xF => self.ora = val,
            0x2 => self.ddrb = val,
            0x3 => self.ddra = val,
            // T1CL: update latch and counter low byte
            0x4 => {
                self.t1cl = val;
                self.t1ll = val;
            }
            // T1CH: load counter, arm timer, clear IFR bit 6
            0x5 => {
                self.t1ch = val;
                self.t1lh = val;
                self.t1_running = true;
                self.ifr &= !0x40;
            }
            // T1LL: just update latch, don't restart
            0x6 => self.t1ll = val,
            0x7 => self.t1lh = val,
            0x8 => self.t2cl = val,
            // T2CH: load counter, arm timer, clear IFR bit 5
            0x9 => {
                self.t2ch = val;
                self.t2_running = true;
                self.ifr &= !0x20;
            }
            0xA => self.sr = val,
            0xB => self.acr = val,
            0xC => self.pcr = val,
            // IFR: writing 1s to bits CLEARS them (6522 behavior)
            0xD => self.ifr &= !(val & 0x7F),
            // IER: bit 7 selects set (1) or clear (0) for the bits written as 1.
            _ => {
                if val & 0x80 != 0 {
                    self.ier |= val & 0x7F;
                } else {
                    self.ier &= !(val & 0x7F);
                }
            }
        }
    }

    /// Returns true if any enabled interrupt is active.
    pub fn irq_active(&self) -> bool {
        self.ifr & self.ier & 0x7F != 0
    }

    /// Serialize VIA state (26 bytes: 15 regs + 2 running flags + 8 last_cycles + 1 padding).
    pub fn save_state(&self, out: &mut dyn Write) -> Result<()> {
        out.write_all(&[
            self.ora,
            self.orb,
            self.ddra,
            self.ddrb,
            self.t1cl,
            self.t1ch,
            self.t1ll,
            self.t1lh,
            self.t2cl,
            self.t2ch,
            self.sr,
            self.acr,
            self.pcr,
            self.ifr,
            self.ier,
            self.t1_running as u8,
            self.t2_running as u8,
        ])?;
        out.write_all(&self.last_cycles.to_le_bytes())?;
        Ok(())
    }

    /// Deserialize VIA state (26 bytes).
    pub fn load_state(&mut self, src: &mut dyn Read) -> Result<()> {
        let mut buf = [0u8; 15];
        src.read_exact(&mut buf)?;
        self.ora = buf[0];
        self.orb = buf[1];
        self.ddra = buf[2];
        self.ddrb = buf[3];
        self.t1cl = buf[4];
        self.t1ch = buf[5];
        self.t1ll = buf[6];
        self.t1lh = buf[7];
        self.t2cl = buf[8];
        self.t2ch = buf[9];
        self.sr = buf[10];
        self.acr = buf[11];
        self.pcr = buf[12];
        self.ifr = buf[13];
        self.ier = buf[14];
        let mut run_buf = [0u8; 2];
        src.read_exact(&mut run_buf)?;
        self.t1_running = run_buf[0] != 0;
        self.t2_running = run_buf[1] != 0;
        let mut cyc_buf = [0u8; 8];
        src.read_exact(&mut cyc_buf)?;
        self.last_cycles = u64::from_le_bytes(cyc_buf);
        Ok(())
    }
}

impl Default for Via6522 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_initializes_all_fields_to_zero() {
        let via = Via6522::new();
        assert_eq!(via.ora, 0);
        assert_eq!(via.orb, 0);
        assert_eq!(via.ddra, 0);
        assert_eq!(via.ddrb, 0);
        assert_eq!(via.t1cl, 0);
        assert_eq!(via.t1ch, 0);
        assert_eq!(via.t1ll, 0);
        assert_eq!(via.t1lh, 0);
        assert_eq!(via.t2cl, 0);
        assert_eq!(via.t2ch, 0);
        assert_eq!(via.sr, 0);
        assert_eq!(via.acr, 0);
        assert_eq!(via.pcr, 0);
        assert_eq!(via.ifr, 0);
        assert_eq!(via.ier, 0);
        assert_eq!(via.last_cycles, 0);
        assert!(!via.t1_running);
        assert!(!via.t2_running);
    }

    #[test]
    fn reset_clears_state() {
        let mut via = Via6522::new();
        via.ora = 0xAB;
        via.t1_running = true;
        via.ifr = 0x40;
        via.last_cycles = 1000;
        via.reset();
        assert_eq!(via.ora, 0);
        assert!(!via.t1_running);
        assert_eq!(via.ifr, 0);
        assert_eq!(via.last_cycles, 0);
    }

    #[test]
    fn read_write_register_roundtrip() {
        let mut via = Via6522::new();
        // Write and read back each addressable register
        via.write(0x0, 0x12); // ORB
        via.write(0x1, 0x34); // ORA
        via.write(0x2, 0x56); // DDRB
        via.write(0x3, 0x78); // DDRA
        via.write(0xA, 0x9A); // SR
        via.write(0xB, 0xBC); // ACR
        via.write(0xC, 0xDE); // PCR
        via.write(0xE, 0xF0); // IER

        assert_eq!(via.read(0x0), 0x12);
        assert_eq!(via.read(0x1), 0x34);
        assert_eq!(via.read(0x2), 0x56);
        assert_eq!(via.read(0x3), 0x78);
        assert_eq!(via.read(0xA), 0x9A);
        assert_eq!(via.read(0xB), 0xBC);
        assert_eq!(via.read(0xC), 0xDE);
        assert_eq!(via.read(0xE), 0xF0);
    }

    #[test]
    fn write_t1ch_arms_timer_and_clears_ifr() {
        let mut via = Via6522::new();
        via.ifr = 0x40; // T1 interrupt flag set
        via.write(0x4, 0x10); // T1CL = 0x10 (also sets T1LL)
        via.write(0x5, 0x02); // T1CH = 0x02 (arms timer, clears IFR bit 6)

        assert!(via.t1_running);
        assert_eq!(via.ifr & 0x40, 0, "T1 IFR bit should be cleared");
        assert_eq!(via.t1cl, 0x10);
        assert_eq!(via.t1ch, 0x02);
        assert_eq!(via.t1ll, 0x10); // Latch updated by T1CL write
        assert_eq!(via.t1lh, 0x02); // Latch updated by T1CH write
    }

    #[test]
    fn write_t2ch_arms_timer_and_clears_ifr() {
        let mut via = Via6522::new();
        via.ifr = 0x20; // T2 interrupt flag set
        via.write(0x8, 0x50); // T2CL
        via.write(0x9, 0x01); // T2CH (arms timer, clears IFR bit 5)

        assert!(via.t2_running);
        assert_eq!(via.ifr & 0x20, 0, "T2 IFR bit should be cleared");
        assert_eq!(via.t2cl, 0x50);
        assert_eq!(via.t2ch, 0x01);
    }

    #[test]
    fn t1_one_shot_expires_and_stops() {
        let mut via = Via6522::new();
        via.acr = 0x00; // One-shot mode (ACR bit 6 = 0)
        via.write(0x4, 0x10); // T1CL = 0x10
        via.write(0x5, 0x00); // T1CH = 0x00 → counter = 0x0010 = 16

        // Advance past expiry: counter=16, needs delta >= 16+2 = 18
        via.tick(20);

        assert_eq!(via.ifr & 0x40, 0x40, "T1 IFR bit should be set");
        assert!(!via.t1_running, "One-shot timer should stop after expiry");
        assert_eq!(via.t1cl, 0xFF);
        assert_eq!(via.t1ch, 0xFF);
    }

    #[test]
    fn t1_continuous_reloads_from_latch() {
        let mut via = Via6522::new();
        via.acr = 0x40; // Continuous mode (ACR bit 6 = 1)
        // Set latch to 0x0020 = 32
        via.write(0x6, 0x20); // T1LL
        via.write(0x7, 0x00); // T1LH
        // Arm timer with counter = 0x0010 = 16
        via.write(0x4, 0x10); // T1CL
        via.write(0x5, 0x00); // T1CH

        // Advance past expiry
        via.tick(20);

        assert_eq!(via.ifr & 0x40, 0x40, "T1 IFR bit should be set");
        assert!(via.t1_running, "Continuous timer should keep running");
        // Counter reloaded from latch, not 0xFFFF
        assert_ne!(via.t1cl, 0xFF);
    }

    #[test]
    fn t2_one_shot_expires_and_stops() {
        let mut via = Via6522::new();
        via.write(0x8, 0x08); // T2CL = 0x08
        via.write(0x9, 0x00); // T2CH = 0x00 → counter = 8

        // Advance past expiry: needs delta >= 8+2 = 10
        via.tick(12);

        assert_eq!(via.ifr & 0x20, 0x20, "T2 IFR bit should be set");
        assert!(!via.t2_running, "T2 should stop after expiry");
        assert_eq!(via.t2cl, 0xFF);
        assert_eq!(via.t2ch, 0xFF);
    }

    #[test]
    fn tick_decrements_timer_without_expiry() {
        let mut via = Via6522::new();
        via.write(0x4, 0x00); // T1CL = 0x00
        via.write(0x5, 0x01); // T1CH = 0x01 → counter = 0x0100 = 256

        // Advance by 100 cycles (not enough to expire)
        via.tick(100);

        assert!(via.t1_running, "Timer should still be running");
        assert_eq!(via.ifr & 0x40, 0, "T1 IFR should not be set");
        // Counter should be 256 - 100 = 156 = 0x009C
        let counter = ((via.t1ch as u16) << 8) | via.t1cl as u16;
        assert_eq!(counter, 156);
    }

    #[test]
    fn tick_no_op_when_cycles_not_advanced() {
        let mut via = Via6522::new();
        via.write(0x4, 0x10);
        via.write(0x5, 0x00);
        via.tick(5);

        let cl_before = via.t1cl;
        let ch_before = via.t1ch;

        // Tick with same or earlier cycle count — should be a no-op
        via.tick(5);
        assert_eq!(via.t1cl, cl_before);
        assert_eq!(via.t1ch, ch_before);

        via.tick(3); // Earlier cycle count
        assert_eq!(via.t1cl, cl_before);
        assert_eq!(via.t1ch, ch_before);
    }

    #[test]
    fn ifr_writing_ones_clears_bits() {
        let mut via = Via6522::new();
        via.ifr = 0x60; // T1 + T2 flags set

        // Writing 0x40 to IFR should clear bit 6 (T1)
        via.write(0xD, 0x40);
        assert_eq!(via.ifr, 0x20, "Only T2 bit should remain");

        // Writing 0x20 clears T2
        via.write(0xD, 0x20);
        assert_eq!(via.ifr, 0x00);
    }

    #[test]
    fn ifr_read_sets_bit7_when_enabled_interrupt_active() {
        let mut via = Via6522::new();
        via.ifr = 0x40; // T1 flag set
        via.ier = 0x40; // T1 interrupt enabled

        // Reading IFR should have bit 7 set (enabled interrupt is active)
        let ifr_val = via.read(0xD);
        assert_eq!(ifr_val & 0x80, 0x80, "Bit 7 should be set");
        assert_eq!(ifr_val & 0x40, 0x40, "T1 flag should be present");
    }

    #[test]
    fn ifr_read_clears_bit7_when_no_enabled_interrupt() {
        let mut via = Via6522::new();
        via.ifr = 0x40; // T1 flag set
        via.ier = 0x20; // Only T2 interrupt enabled (not T1)

        let ifr_val = via.read(0xD);
        assert_eq!(ifr_val & 0x80, 0x00, "Bit 7 should be clear");
    }

    #[test]
    fn irq_active_reflects_enabled_flags() {
        let mut via = Via6522::new();
        assert!(!via.irq_active());

        via.ifr = 0x40;
        via.ier = 0x00;
        assert!(!via.irq_active(), "No enabled interrupts");

        via.ier = 0x40;
        assert!(via.irq_active(), "T1 enabled and flagged");

        via.ifr = 0x00;
        assert!(!via.irq_active(), "Flag cleared");
    }

    #[test]
    fn save_load_state_roundtrip() {
        let mut via = Via6522::new();
        via.ora = 0x11;
        via.orb = 0x22;
        via.ddra = 0x33;
        via.ddrb = 0x44;
        via.t1cl = 0x55;
        via.t1ch = 0x66;
        via.t1ll = 0x77;
        via.t1lh = 0x88;
        via.t2cl = 0x99;
        via.t2ch = 0xAA;
        via.sr = 0xBB;
        via.acr = 0xCC;
        via.pcr = 0xDD;
        via.ifr = 0x60;
        via.ier = 0x40;
        via.last_cycles = 123456789;
        via.t1_running = true;
        via.t2_running = false;

        // Save
        let mut buf = Vec::new();
        via.save_state(&mut buf).unwrap();
        assert_eq!(buf.len(), 25, "17 reg bytes + 8 cycle bytes");

        // Load into fresh VIA
        let mut via2 = Via6522::new();
        let mut cursor = std::io::Cursor::new(&buf);
        via2.load_state(&mut cursor).unwrap();

        assert_eq!(via2.ora, 0x11);
        assert_eq!(via2.orb, 0x22);
        assert_eq!(via2.ddra, 0x33);
        assert_eq!(via2.ddrb, 0x44);
        assert_eq!(via2.t1cl, 0x55);
        assert_eq!(via2.t1ch, 0x66);
        assert_eq!(via2.t1ll, 0x77);
        assert_eq!(via2.t1lh, 0x88);
        assert_eq!(via2.t2cl, 0x99);
        assert_eq!(via2.t2ch, 0xAA);
        assert_eq!(via2.sr, 0xBB);
        assert_eq!(via2.acr, 0xCC);
        assert_eq!(via2.pcr, 0xDD);
        assert_eq!(via2.ifr, 0x60);
        assert_eq!(via2.ier, 0x40);
        assert_eq!(via2.last_cycles, 123456789);
        assert!(via2.t1_running);
        assert!(!via2.t2_running);
    }

    #[test]
    fn write_t1ll_does_not_restart_timer() {
        let mut via = Via6522::new();
        // T1 not running
        assert!(!via.t1_running);

        // Write to T1LL (reg 6) — should update latch only, not arm timer
        via.write(0x6, 0xFF);
        assert!(!via.t1_running, "T1LL write should not arm timer");
        assert_eq!(via.t1ll, 0xFF);
    }

    #[test]
    fn reg_f_is_ora_without_handshake() {
        let mut via = Via6522::new();
        via.write(0xF, 0x5A);
        assert_eq!(via.ora, 0x5A);
        assert_eq!(via.read(0xF), 0x5A);
    }

    #[test]
    fn ier_write_bit7_sets_or_clears_selected_bits() {
        let mut via = Via6522::new();
        via.write(0xE, 0xC0); // set T1
        via.write(0xE, 0xA0); // set T2
        assert_eq!(via.ier, 0x60);
        via.write(0xE, 0x40); // clear T1 only
        assert_eq!(via.ier, 0x20);
        via.write(0xE, 0x7F); // clear all
        assert_eq!(via.ier, 0x00);
        assert_eq!(via.read(0xE), 0x80, "IER bit 7 reads as 1");
    }

    #[test]
    fn reading_t1cl_acknowledges_t1_interrupt() {
        let mut via = Via6522::new();
        via.ifr = 0x60;
        via.read(0x4);
        assert_eq!(via.ifr, 0x20, "T1C-L read clears only the T1 flag");
        via.read(0x8);
        assert_eq!(via.ifr, 0x00, "T2C-L read clears the T2 flag");
    }

    #[test]
    fn next_irq_cycle_is_where_tick_raises_the_flag() {
        let mut via = Via6522::new();
        via.last_cycles = 1000;
        via.write(0xE, 0xC0); // enable T1 IRQ
        via.write(0x4, 0x10);
        via.write(0x5, 0x00); // T1 = 16, armed
        let due = via.next_irq_cycle();
        assert_eq!(due, 1000 + 16 + 2);

        via.tick(due - 2);
        assert!(!via.irq_active(), "counter at 0 but not yet expired");
        via.tick(due);
        assert!(via.irq_active(), "expired exactly at next_irq_cycle");
        assert_eq!(via.next_irq_cycle(), u64::MAX, "no event while flag set");
    }

    /// Free-running T1 must interrupt every latch + 2 cycles, however late
    /// after each expiry the timer is next ticked (lazy evaluation).
    #[test]
    fn t1_free_running_period_is_latch_plus_two() {
        let mut via = Via6522::new();
        via.write(0xB, 0x40); // ACR: continuous
        via.write(0xE, 0xC0); // IER: T1
        via.write(0x4, 100);
        via.write(0x5, 0x00); // latch = counter = 100
        let mut expiries = Vec::new();
        for late in [0u64, 7, 99, 3] {
            let due = via.next_irq_cycle();
            via.tick(due + late); // observe the expiry `late` cycles after it
            assert!(via.irq_active());
            expiries.push(due);
            via.read(0x4); // acknowledge
        }
        let periods: Vec<u64> = expiries.windows(2).map(|w| w[1] - w[0]).collect();
        assert_eq!(periods, vec![102, 102, 102]);
    }

    #[test]
    fn next_irq_cycle_ignores_disabled_or_stopped_timers() {
        let mut via = Via6522::new();
        via.write(0x4, 0x10);
        via.write(0x5, 0x00); // armed, but IER clear
        assert_eq!(via.next_irq_cycle(), u64::MAX);
        via.write(0xE, 0xA0); // T2 enabled, T2 not running
        assert_eq!(via.next_irq_cycle(), u64::MAX);
    }

    #[test]
    fn register_addressing_masks_to_4_bits() {
        let mut via = Via6522::new();
        via.write(0x1, 0xAB); // ORA
        // Reading with upper bits set should still address ORA (reg & 0x0F = 1)
        assert_eq!(via.read(0x11), 0xAB);
        assert_eq!(via.read(0xF1), 0xAB);
    }
}
