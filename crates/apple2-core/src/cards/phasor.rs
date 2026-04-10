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
use crate::error::Result;
use apple2_audio::ay8910::Ay8910;
use std::io::{Read, Write};

// AY clock for Apple II: ~1.02 MHz
const AY_CLOCK: f64 = 1_020_484.0;

// ── Minimal 6522 VIA ─────────────────────────────────────────────────────────

struct Via {
    ora: u8,
    orb: u8,
    ddra: u8,
    ddrb: u8,
    t1cl: u8,
    t1ch: u8,
    t1ll: u8,
    t1lh: u8,
    t2cl: u8,
    t2ch: u8,
    sr: u8,
    acr: u8,
    pcr: u8,
    ifr: u8,
    ier: u8,
    /// Last CPU cycle count when timers were updated.
    last_cycles: u64,
    /// True when T1 is running (armed by write to T1CH).
    t1_running: bool,
    /// True when T2 is running (armed by write to T2CH).
    t2_running: bool,
}

impl Via {
    fn new() -> Self {
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

    fn reset(&mut self) {
        *self = Self::new();
    }

    /// Update timer state based on elapsed CPU cycles (lazy evaluation).
    fn tick(&mut self, current_cycles: u64) {
        if current_cycles <= self.last_cycles {
            return;
        }
        let delta = (current_cycles - self.last_cycles) as u32;
        self.last_cycles = current_cycles;

        if self.t1_running {
            let t1 = ((self.t1ch as u32) << 8) | (self.t1cl as u32);
            if delta >= t1 + 2 {
                // T1 expired
                self.ifr |= 0x40; // bit 6 = T1 timeout
                let continuous = self.acr & 0x40 != 0; // ACR bit 6
                if continuous {
                    // Reload from latch and continue
                    let latch = ((self.t1lh as u32) << 8) | (self.t1ll as u32);
                    let remaining = delta - (t1 + 2);
                    let period = latch + 2;
                    let new_count = if period > 0 {
                        period - (remaining % period)
                    } else {
                        0
                    };
                    self.t1cl = (new_count & 0xFF) as u8;
                    self.t1ch = ((new_count >> 8) & 0xFF) as u8;
                } else {
                    // One-shot: set to 0xFFFF and stop
                    self.t1cl = 0xFF;
                    self.t1ch = 0xFF;
                    self.t1_running = false;
                }
            } else {
                let new_t1 = t1 - delta;
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
                let new_t2 = t2 - delta;
                self.t2cl = (new_t2 & 0xFF) as u8;
                self.t2ch = ((new_t2 >> 8) & 0xFF) as u8;
            }
        }
    }

    fn read(&self, reg: u8) -> u8 {
        match reg & 0x0F {
            0x0 => self.orb,
            0x1 => self.ora,
            0x2 => self.ddrb,
            0x3 => self.ddra,
            0x4 => self.t1cl,
            0x5 => self.t1ch,
            0x6 => self.t1ll,
            0x7 => self.t1lh,
            0x8 => self.t2cl,
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
            0xE => self.ier,
            _ => 0xFF,
        }
    }

    fn write(&mut self, reg: u8, val: u8) {
        match reg & 0x0F {
            0x0 => self.orb = val,
            0x1 => self.ora = val,
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
            0xE => self.ier = val,
            _ => {}
        }
    }
}

// ── PhasorCard ────────────────────────────────────────────────────────────────

/// Phasor card with 2 VIAs and 4 AY-3-8910 chips.
pub struct PhasorCard {
    slot: usize,
    via: [Via; 2],
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
            via: [Via::new(), Via::new()],
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
        self.via.iter().any(|v| v.ifr & v.ier & 0x7F != 0)
    }

    fn save_state(&self, out: &mut dyn Write) -> Result<()> {
        out.write_all(&[1u8])?; // version
        for via in &self.via {
            out.write_all(&[
                via.ora,
                via.orb,
                via.ddra,
                via.ddrb,
                via.t1cl,
                via.t1ch,
                via.t1ll,
                via.t1lh,
                via.t2cl,
                via.t2ch,
                via.sr,
                via.acr,
                via.pcr,
                via.ifr,
                via.ier,
                via.t1_running as u8,
                via.t2_running as u8,
            ])?;
            out.write_all(&via.last_cycles.to_le_bytes())?;
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
            let mut buf = [0u8; 15];
            src.read_exact(&mut buf)?;
            via.ora = buf[0];
            via.orb = buf[1];
            via.ddra = buf[2];
            via.ddrb = buf[3];
            via.t1cl = buf[4];
            via.t1ch = buf[5];
            via.t1ll = buf[6];
            via.t1lh = buf[7];
            via.t2cl = buf[8];
            via.t2ch = buf[9];
            via.sr = buf[10];
            via.acr = buf[11];
            via.pcr = buf[12];
            via.ifr = buf[13];
            via.ier = buf[14];
            let mut run_buf = [0u8; 2];
            src.read_exact(&mut run_buf)?;
            via.t1_running = run_buf[0] != 0;
            via.t2_running = run_buf[1] != 0;
            let mut cyc_buf = [0u8; 8];
            src.read_exact(&mut cyc_buf)?;
            via.last_cycles = u64::from_le_bytes(cyc_buf);
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
