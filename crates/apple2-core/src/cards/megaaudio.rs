//! MegaAudio card emulation — a Mockingboard-compatible audio card with
//! enhanced polyphony via a 3rd AY-3-8910 PSG.
//!
//! Architecture: 2 × 6522 VIA + 3 × AY-3-8910.
//! The first two AY chips are driven identically to Mockingboard (VIA 1 → AY A,
//! VIA 2 → AY B).  The 3rd AY chip mirrors VIA 1's bus commands, providing
//! three extra tone channels for expanded polyphony.
//!
//! Memory map is identical to Mockingboard:
//!   $C0x0–$C0x7  VIA 1 (AY chip A + AY chip C mirror)
//!   $C0x8–$C0xF  VIA 2 (AY chip B)

use crate::card::{Card, CardType};
use crate::cards::mb_firmware::MB_FIRMWARE;
use crate::cards::via6522::Via6522;
use crate::error::Result;
use apple2_audio::ay8910::Ay8910;
use std::io::{Read, Write};

// ── MegaAudioCard ────────────────────────────────────────────────────────────

/// AY clock: ~1.02 MHz (Apple II NTSC).
const AY_CLOCK: f64 = 1_020_484.0;

pub struct MegaAudioCard {
    slot: usize,
    via: [Via6522; 2],
    /// Three AY-3-8910 PSGs: ay[0] via VIA 1, ay[1] via VIA 2, ay[2] mirrors VIA 1.
    ay: [Ay8910; 3],
    cycles_pending: u64,
}

impl MegaAudioCard {
    pub fn new(slot: usize) -> Self {
        Self {
            slot,
            via: [Via6522::new(), Via6522::new()],
            ay: [Ay8910::new(), Ay8910::new(), Ay8910::new()],
            cycles_pending: 0,
        }
    }

    /// Decode BDIR/BC1 from ORB bits[1:0] and apply to the AY chip(s) for `via_idx`.
    fn strobe_ay(&mut self, via_idx: usize) {
        let bc = self.via[via_idx].orb & 0x03;
        let data = self.via[via_idx].ora;
        match bc {
            0x03 => {
                self.ay[via_idx].select_reg(data);
                // Mirror to 3rd AY chip when VIA 1 strobes
                if via_idx == 0 {
                    self.ay[2].select_reg(data);
                }
            }
            0x02 => {
                self.ay[via_idx].write_reg(data);
                if via_idx == 0 {
                    self.ay[2].write_reg(data);
                }
            }
            0x01 => {
                self.via[via_idx].ora = self.ay[via_idx].read_reg();
            }
            _ => {}
        }
    }
}

impl Card for MegaAudioCard {
    fn card_type(&self) -> CardType {
        CardType::MegaAudio
    }
    fn slot(&self) -> usize {
        self.slot
    }

    fn io_read(&mut self, offset: u8, _cycles: u64) -> u8 {
        *MB_FIRMWARE.get(offset as usize).unwrap_or(&0xFF)
    }

    fn io_write(&mut self, _offset: u8, _value: u8, _cycles: u64) {}

    fn cx_rom(&self) -> Option<&[u8; 256]> {
        Some(MB_FIRMWARE)
    }

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
        if n_samples == 0 {
            return;
        }
        self.cycles_pending -= (n_samples as f64 / sample_rate as f64 * AY_CLOCK) as u64;

        let base = out.len();
        out.reserve(n_samples);
        out.resize(base + n_samples, 0.0f32);
        for ay in &mut self.ay {
            ay.render(&mut out[base..], AY_CLOCK, sample_rate);
        }
        // Scale: 3 AY chips adding together — reduce to avoid clipping.
        for s in &mut out[base..] {
            *s *= 0.33;
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
        Ok(())
    }

    fn load_state(&mut self, src: &mut dyn Read, _version: u32) -> Result<()> {
        let mut ver = [0u8; 1];
        src.read_exact(&mut ver)?;
        for via in &mut self.via {
            via.load_state(src)?;
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
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
