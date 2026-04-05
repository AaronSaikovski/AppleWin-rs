//! SD Music card emulation — a Mockingboard-compatible audio card with
//! SD card music streaming capabilities.
//!
//! Architecture: 2 × 6522 VIA + 2 × AY-3-8910, plus an additional register
//! range for SD card control (stubbed — no real SD card attached).
//!
//! The base Mockingboard functionality is identical:
//!   $C0x0–$C0x7  VIA 1 (AY chip A)
//!   $C0x8–$C0xF  VIA 2 (AY chip B)
//!
//! The SD card control interface is exposed via VIA 2 port B auxiliary bits
//! and directly through the $Cn00 ROM page I/O area.  Since no real SD card
//! is present, SD reads return 0xFF (card not present).

use std::io::{Read, Write};
use apple2_audio::ay8910::Ay8910;
use crate::card::{Card, CardType};
use crate::error::Result;

// Re-use the Mockingboard firmware ROM.
static MB_FIRMWARE: &[u8; 256] = {
    const ROM: &[u8] = include_bytes!("../../roms/Mockingboard-D.rom");
    unsafe { &*(ROM.as_ptr() as *const [u8; 256]) }
};

// ── Simplified 6522 VIA (same as Mockingboard) ──────────────────────────────

struct Via {
    ora:  u8,
    orb:  u8,
    ddra: u8,
    ddrb: u8,
    t1cl: u8,
    t1ch: u8,
    t1ll: u8,
    t1lh: u8,
    t2cl: u8,
    t2ch: u8,
    sr:   u8,
    acr:  u8,
    pcr:  u8,
    ifr:  u8,
    ier:  u8,
    last_cycles: u64,
    t1_running: bool,
    t2_running: bool,
}

impl Via {
    fn new() -> Self {
        Self {
            ora: 0, orb: 0, ddra: 0, ddrb: 0,
            t1cl: 0, t1ch: 0, t1ll: 0, t1lh: 0,
            t2cl: 0, t2ch: 0, sr: 0, acr: 0, pcr: 0,
            ifr: 0, ier: 0,
            last_cycles: 0,
            t1_running: false,
            t2_running: false,
        }
    }

    fn reset(&mut self) { *self = Self::new(); }

    fn tick(&mut self, current_cycles: u64) {
        if current_cycles <= self.last_cycles { return; }
        let delta = (current_cycles - self.last_cycles) as u32;
        self.last_cycles = current_cycles;

        if self.t1_running {
            let t1 = ((self.t1ch as u32) << 8) | (self.t1cl as u32);
            if delta >= t1 + 2 {
                self.ifr |= 0x40;
                let continuous = self.acr & 0x40 != 0;
                if continuous {
                    let latch = ((self.t1lh as u32) << 8) | (self.t1ll as u32);
                    let remaining = delta - (t1 + 2);
                    let period = latch + 2;
                    let new_count = if period > 0 { period - (remaining % period) } else { 0 };
                    self.t1cl = (new_count & 0xFF) as u8;
                    self.t1ch = ((new_count >> 8) & 0xFF) as u8;
                } else {
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
                self.ifr |= 0x20;
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
            0xD => self.ifr | if self.ifr & self.ier & 0x7F != 0 { 0x80 } else { 0x00 },
            0xE => self.ier,
            _   => 0xFF,
        }
    }

    fn write(&mut self, reg: u8, val: u8) {
        match reg & 0x0F {
            0x0 => self.orb  = val,
            0x1 => self.ora  = val,
            0x2 => self.ddrb = val,
            0x3 => self.ddra = val,
            0x4 => { self.t1cl = val; self.t1ll = val; }
            0x5 => {
                self.t1ch = val; self.t1lh = val;
                self.t1_running = true;
                self.ifr &= !0x40;
            }
            0x6 => self.t1ll = val,
            0x7 => self.t1lh = val,
            0x8 => self.t2cl = val,
            0x9 => {
                self.t2ch = val;
                self.t2_running = true;
                self.ifr &= !0x20;
            }
            0xA => self.sr   = val,
            0xB => self.acr  = val,
            0xC => self.pcr  = val,
            0xD => self.ifr &= !(val & 0x7F),
            0xE => self.ier  = val,
            _   => {}
        }
    }
}

// ── SdMusicCard ──────────────────────────────────────────────────────────────

/// AY clock: ~1.02 MHz (Apple II NTSC).
const AY_CLOCK: f64 = 1_020_484.0;

pub struct SdMusicCard {
    slot: usize,
    via:  [Via; 2],
    ay:   [Ay8910; 2],
    cycles_pending: u64,
    /// SD card control register (directly addressed via ROM-page I/O).
    /// Stubbed: reads return 0xFF (no card present).
    sd_ctrl: u8,
    /// SD card data register (last byte written for SPI-like commands).
    sd_data: u8,
}

impl SdMusicCard {
    pub fn new(slot: usize) -> Self {
        Self {
            slot,
            via: [Via::new(), Via::new()],
            ay:  [Ay8910::new(), Ay8910::new()],
            cycles_pending: 0,
            sd_ctrl: 0,
            sd_data: 0,
        }
    }

    fn strobe_ay(&mut self, idx: usize) {
        let bc = self.via[idx].orb & 0x03;
        let data = self.via[idx].ora;
        match bc {
            0x03 => { self.ay[idx].select_reg(data); }
            0x02 => { self.ay[idx].write_reg(data); }
            0x01 => { self.via[idx].ora = self.ay[idx].read_reg(); }
            _    => {}
        }
    }
}

impl Card for SdMusicCard {
    fn card_type(&self) -> CardType { CardType::SdMusic }
    fn slot(&self) -> usize { self.slot }

    fn io_read(&mut self, offset: u8, _cycles: u64) -> u8 {
        // Offsets 0x00–0xEF: firmware ROM.
        // Offsets 0xF0–0xFF: SD card status (stubbed — return 0xFF = no card).
        if offset >= 0xF0 {
            0xFF
        } else {
            *MB_FIRMWARE.get(offset as usize).unwrap_or(&0xFF)
        }
    }

    fn io_write(&mut self, offset: u8, value: u8, _cycles: u64) {
        // SD card control writes via ROM-page I/O space.
        if offset >= 0xF0 {
            match offset {
                0xF0 => self.sd_ctrl = value,
                0xF1 => self.sd_data = value,
                _    => {}
            }
        }
    }

    fn cx_rom(&self) -> Option<&[u8; 256]> { Some(MB_FIRMWARE) }

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
        let n_samples = ((self.cycles_pending as f64 / AY_CLOCK) * sample_rate as f64) as usize;
        if n_samples == 0 { return; }
        self.cycles_pending -= (n_samples as f64 / sample_rate as f64 * AY_CLOCK) as u64;

        let base = out.len();
        out.reserve(n_samples);
        out.resize(base + n_samples, 0.0f32);
        for ay in &mut self.ay {
            ay.render(&mut out[base..], AY_CLOCK, sample_rate);
        }
        // Scale: 2 AY chips adding together.
        for s in &mut out[base..] { *s *= 0.5; }
    }

    fn reset(&mut self, _power_cycle: bool) {
        for via in &mut self.via { via.reset(); }
        for ay in &mut self.ay { ay.reset(); }
        self.cycles_pending = 0;
        self.sd_ctrl = 0;
        self.sd_data = 0;
    }

    fn update(&mut self, _cycles: u64) {}

    fn irq_active(&self) -> bool {
        self.via.iter().any(|v| v.ifr & v.ier & 0x7F != 0)
    }

    fn save_state(&self, out: &mut dyn Write) -> Result<()> {
        out.write_all(&[1u8])?; // version
        for via in &self.via {
            out.write_all(&[via.ora, via.orb, via.ddra, via.ddrb,
                            via.t1cl, via.t1ch, via.t1ll, via.t1lh,
                            via.t2cl, via.t2ch, via.sr, via.acr,
                            via.pcr, via.ifr, via.ier,
                            via.t1_running as u8, via.t2_running as u8])?;
            out.write_all(&via.last_cycles.to_le_bytes())?;
        }
        for ay in &self.ay {
            out.write_all(&ay.regs)?;
            out.write_all(&[ay.selected_reg])?;
        }
        out.write_all(&self.cycles_pending.to_le_bytes())?;
        out.write_all(&[self.sd_ctrl, self.sd_data])?;
        Ok(())
    }

    fn load_state(&mut self, src: &mut dyn Read, _version: u32) -> Result<()> {
        let mut ver = [0u8; 1];
        src.read_exact(&mut ver)?;
        for via in &mut self.via {
            let mut buf = [0u8; 15];
            src.read_exact(&mut buf)?;
            via.ora = buf[0]; via.orb = buf[1]; via.ddra = buf[2]; via.ddrb = buf[3];
            via.t1cl = buf[4]; via.t1ch = buf[5]; via.t1ll = buf[6]; via.t1lh = buf[7];
            via.t2cl = buf[8]; via.t2ch = buf[9]; via.sr = buf[10]; via.acr = buf[11];
            via.pcr = buf[12]; via.ifr = buf[13]; via.ier = buf[14];
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
            ay.refresh_all_caches();
        }
        let mut cyc_buf = [0u8; 8];
        src.read_exact(&mut cyc_buf)?;
        self.cycles_pending = u64::from_le_bytes(cyc_buf);
        let mut sd_buf = [0u8; 2];
        src.read_exact(&mut sd_buf)?;
        self.sd_ctrl = sd_buf[0];
        self.sd_data = sd_buf[1];
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}
