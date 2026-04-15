//! Phasor sound card emulation (Applied Engineering).
//!
//! The Phasor is a super-set of the Mockingboard. In native mode it
//! contains four AY-3-8910 PSGs (chips 0–3) controlled by two 6522 VIAs.
//!
//! Memory map (slot IO $C0x0–$C0xF):
//!   $C0x0–$C0x3  VIA 0: ORB, ORA, DDRB, DDRA
//!   $C0x4–$C0x7  VIA 0 timer regs (stubs)
//!   $C0x8–$C0xB  VIA 1: ORB, ORA, DDRB, DDRA
//!   $C0xC–$C0xF  VIA 1 timer regs (stubs)
//!
//! ORB bit encoding per VIA:
//!   bits 1:0  BDIR/BC1 (AY command lines)
//!   bit 3     chip-select for first AY on this VIA (Phasor native mode)
//!   bit 4     chip-select for second AY on this VIA (Phasor native mode)
//!
//! Reference: source/Mockingboard.cpp (Phasor section)

use crate::card::{Card, CardType};
use crate::cards::via6522::Via6522;
use crate::error::Result;
use apple2_audio::ay8910::Ay8910;
use std::io::{Read, Write};

// AY clock for Apple II: ~1.02 MHz
const AY_CLOCK: f64 = 1_020_484.0;

// ── PhasorCard ────────────────────────────────────────────────────────────────

/// Phasor card with 2 VIAs and 4 AY-3-8910 chips.
pub struct PhasorCard {
    slot: usize,
    via: [Via6522; 2],
    /// Four AY chips: chips 0 and 1 on VIA 0, chips 2 and 3 on VIA 1.
    ay: [Ay8910; 4],
    /// True = Phasor native mode (4 chips); false = Mockingboard compat (2 chips).
    phasor_mode: bool,
    /// Accumulated cycles since last audio drain.
    cycles_pending: u64,
}

impl PhasorCard {
    pub fn new(slot: usize) -> Self {
        Self {
            slot,
            via: [Via6522::new(), Via6522::new()],
            ay: std::array::from_fn(|_| Ay8910::new()),
            phasor_mode: true,
            cycles_pending: 0,
        }
    }

    /// Apply the AY bus command for VIA `via_idx`.
    /// In Phasor native mode: bits 3 and 4 of ORB select which chip(s) are active.
    /// In Mockingboard compat mode: always selects the one chip for this VIA.
    fn strobe_ay(&mut self, via_idx: usize) {
        let bc = self.via[via_idx].orb & 0x03;
        let data = self.via[via_idx].ora;

        if self.phasor_mode {
            // Bits 3 and 4 are chip-selects for the two chips on this VIA
            let cs0 = self.via[via_idx].orb & 0x08 != 0; // chip 0 or 2
            let cs1 = self.via[via_idx].orb & 0x10 != 0; // chip 1 or 3
            let base = via_idx * 2;
            if cs0 {
                self.apply_bc(base, bc, data, via_idx);
            }
            if cs1 {
                self.apply_bc(base + 1, bc, data, via_idx);
            }
        } else {
            // Mockingboard compat: VIA 0 -> AY 0, VIA 1 -> AY 1
            self.apply_bc(via_idx, bc, data, via_idx);
        }
    }

    fn apply_bc(&mut self, ay_idx: usize, bc: u8, data: u8, via_idx: usize) {
        match bc {
            0x03 => self.ay[ay_idx].select_reg(data),
            0x02 => self.ay[ay_idx].write_reg(data),
            0x01 => {
                self.via[via_idx].ora = self.ay[ay_idx].read_reg();
            }
            _ => {}
        }
    }
}

impl Card for PhasorCard {
    fn card_type(&self) -> CardType {
        CardType::Phasor
    }
    fn slot(&self) -> usize {
        self.slot
    }

    fn io_read(&mut self, _offset: u8, _cycles: u64) -> u8 {
        0xFF
    }
    fn io_write(&mut self, _offset: u8, _value: u8, _cycles: u64) {}

    fn slot_io_read(&mut self, reg: u8, cycles: u64) -> u8 {
        let via_idx = if reg < 8 { 0 } else { 1 };
        let via_reg = reg & 0x07;
        self.via[via_idx].tick(cycles);
        if via_reg == 0x01 && self.via[via_idx].orb & 0x01 != 0 {
            self.strobe_ay(via_idx);
        }
        self.via[via_idx].read(via_reg)
    }

    fn slot_io_write(&mut self, reg: u8, val: u8, cycles: u64) {
        // Mode register: writing $C0x9 (reg 9 relative to slot base) selects Phasor mode
        if reg == 0x09 {
            self.phasor_mode = val & 0x01 != 0;
        }
        let via_idx = if reg < 8 { 0 } else { 1 };
        let via_reg = reg & 0x07;
        self.via[via_idx].tick(cycles);
        self.via[via_idx].write(via_reg, val);
        if via_reg == 0x00 {
            self.strobe_ay(via_idx);
        }
    }

    fn fill_audio(&mut self, out: &mut Vec<f32>, cycles_elapsed: u64, sample_rate: u32) {
        self.cycles_pending += cycles_elapsed;
        let n_samples = ((self.cycles_pending as f64 / 1_020_484.0) * sample_rate as f64) as usize;
        if n_samples == 0 {
            return;
        }
        self.cycles_pending -= (n_samples as f64 / sample_rate as f64 * 1_020_484.0) as u64;

        let base = out.len();
        out.reserve(n_samples);
        out.resize(base + n_samples, 0.0f32);

        let active_chips = if self.phasor_mode { 4 } else { 2 };
        for ay in &mut self.ay[..active_chips] {
            ay.render(&mut out[base..], AY_CLOCK, sample_rate);
        }
        // Scale to avoid clipping: active_chips each add up to 1.0
        let scale = 1.0 / active_chips as f32;
        for s in &mut out[base..] {
            *s *= scale;
        }
    }

    fn reset(&mut self, _power_cycle: bool) {
        for via in &mut self.via {
            via.reset();
        }
        for ay in &mut self.ay {
            ay.reset();
        }
        self.cycles_pending = 0;
        self.phasor_mode = true;
    }

    fn update(&mut self, _cycles: u64) {}

    fn irq_active(&self) -> bool {
        self.via.iter().any(|v| v.irq_active())
    }

    fn save_state(&self, out: &mut dyn Write) -> Result<()> {
        out.write_all(&[1u8])?; // version
        for via in &self.via {
            via.save_state(out)?;
        }
        for ay in &self.ay {
            out.write_all(&ay.regs)?;
            out.write_all(&[ay.selected_reg])?;
        }
        out.write_all(&self.cycles_pending.to_le_bytes())?;
        out.write_all(&[self.phasor_mode as u8])?;
        Ok(())
    }

    fn load_state(&mut self, src: &mut dyn Read, _version: u32) -> Result<()> {
        let mut ver = [0u8; 1];
        src.read_exact(&mut ver)?;
        // version 1
        for via in &mut self.via {
            via.load_state(src)?;
        }
        for ay in &mut self.ay {
            src.read_exact(&mut ay.regs)?;
            let mut reg_buf = [0u8; 1];
            src.read_exact(&mut reg_buf)?;
            ay.selected_reg = reg_buf[0];
            // Refresh caches after loading all regs
            ay.refresh_all_caches();
        }
        let mut cyc_buf = [0u8; 8];
        src.read_exact(&mut cyc_buf)?;
        self.cycles_pending = u64::from_le_bytes(cyc_buf);
        let mut mode_buf = [0u8; 1];
        src.read_exact(&mut mode_buf)?;
        self.phasor_mode = mode_buf[0] != 0;
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
