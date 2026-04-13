//! Mockingboard card emulation (Sweet Micro Systems Mockingboard Sound/Speech I).
//!
//! Contains two 6522 VIAs, each controlling one AY-3-8910 PSG via BDIR/BC1
//! lines encoded in the VIA's ORB register.
//!
//! Memory map (slot IO space, $C0x0–$C0xF):
//!   $C0x0–$C0x3  VIA 1 (AY chip A):  ORB, ORA, DDRB, DDRA
//!   $C0x4–$C0x7  VIA 1 timer regs (not used in audio; pass-through)
//!   $C0x8–$C0xB  VIA 2 (AY chip B):  ORB, ORA, DDRB, DDRA
//!   $C0xC–$C0xF  VIA 2 timer regs
//!
//! BDIR/BC1 encoding (bits 1:0 of ORB after write):
//!   %00  INACTIVE
//!   %01  READ register from AY → ORA latched
//!   %10  WRITE register in AY ← ORA
//!   %11  LATCH address ← ORA (low 4 bits)
//!
//! Reference: source/Mockingboard.cpp, source/AY8910.cpp

use crate::card::{Card, CardType};
use crate::cards::mb_firmware::MB_FIRMWARE;
use crate::cards::ssi263::Ssi263;
use crate::cards::via6522::Via6522;
use crate::error::Result;
use apple2_audio::ay8910::Ay8910;
use std::io::{Read, Write};

// ── MockingboardCard ───────────────────────────────────────────────────────

/// AY clock for Apple II: 1.0 MHz (1,022,727 Hz PAL / 1,020,484 Hz NTSC)
const AY_CLOCK: f64 = 1_020_484.0;

pub struct MockingboardCard {
    slot: usize,
    via: [Via6522; 2],
    ay: [Ay8910; 2],
    /// Two SSI263 speech chips (one per VIA, only present in Mockingboard D).
    ssi: [Ssi263; 2],

    /// Accumulated cycles since last audio drain.
    cycles_pending: u64,
}

impl MockingboardCard {
    pub fn new(slot: usize) -> Self {
        Self {
            slot,
            via: [Via6522::new(), Via6522::new()],
            ay: [Ay8910::new(), Ay8910::new()],
            ssi: std::array::from_fn(|_| Ssi263::new()),
            cycles_pending: 0,
        }
    }

    /// Decode BDIR/BC1 from ORB bits[1:0] and apply command to AY chip `idx`.
    fn strobe_ay(&mut self, idx: usize) {
        let bc = self.via[idx].orb & 0x03;
        let data = self.via[idx].ora;
        match bc {
            0x03 => {
                // LATCH ADDRESS: select register
                self.ay[idx].select_reg(data);
            }
            0x02 => {
                // WRITE: write data to selected register
                self.ay[idx].write_reg(data);
            }
            0x01 => {
                // READ: latch AY register value into ORA
                self.via[idx].ora = self.ay[idx].read_reg();
            }
            _ => {
                // INACTIVE: nothing
            }
        }
    }
}

impl Card for MockingboardCard {
    fn card_type(&self) -> CardType {
        CardType::Mockingboard
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
        // If reading ORA (reg 1) and BC1=1 (read mode), latch AY value first
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
        // A write to ORB triggers the AY bus cycle
        if via_reg == 0x00 {
            self.strobe_ay(via_idx);
        }
        // SSI263: detect ORA writes with CB2 as output strobe (PCR[7:5] != 0b000)
        if via_reg == 0x01 && self.via[via_idx].pcr & 0xE0 != 0 {
            self.ssi[via_idx].write(val);
        }
    }

    fn fill_audio(&mut self, out: &mut Vec<f32>, cycles_elapsed: u64, sample_rate: u32) {
        self.cycles_pending += cycles_elapsed;
        // Compute expected number of output samples for the elapsed machine cycles.
        // Apple II runs at ~1.02 MHz; cycles_elapsed is in 1 MHz machine cycles.
        let n_samples = ((self.cycles_pending as f64 / 1_020_484.0) * sample_rate as f64) as usize;
        if n_samples == 0 {
            return;
        }
        // Consume used cycles
        self.cycles_pending -= (n_samples as f64 / sample_rate as f64 * 1_020_484.0) as u64;

        // Render directly into the caller's buffer at its current tail.
        // Reserve capacity first to avoid reallocation inside resize().
        // fill(0.0) is SIMD-vectorised; the prior loop was scalar.
        let base = out.len();
        out.reserve(n_samples);
        out.resize(base + n_samples, 0.0f32);
        for ay in &mut self.ay {
            ay.render(&mut out[base..], AY_CLOCK, sample_rate);
        }
        // Mix SSI263 speech chips into the buffer.
        for ssi in &mut self.ssi {
            ssi.fill_audio(&mut out[base..], sample_rate);
        }
        // Scale to avoid clipping (2 AY chips + 2 SSI263 chips all adding).
        for s in &mut out[base..] {
            *s *= 0.5;
        }
    }

    fn reset(&mut self, _power_cycle: bool) {
        for via in &mut self.via {
            via.reset();
        }
        for ay in &mut self.ay {
            ay.reset();
        }
        for ssi in &mut self.ssi {
            ssi.reset();
        }
        self.cycles_pending = 0;
    }

    fn update(&mut self, cycles: u64) {
        for i in 0..2 {
            if self.ssi[i].tick(cycles) {
                // READY asserted — set CA1 in IFR (bit 1)
                self.via[i].ifr |= 0x02;
            }
        }
    }

    fn irq_active(&self) -> bool {
        self.via.iter().any(|v| v.irq_active())
    }

    fn save_state(&self, out: &mut dyn Write) -> Result<()> {
        out.write_all(&[2u8])?; // version
        for via in &self.via {
            via.save_state(out)?;
        }
        for ay in &self.ay {
            out.write_all(&ay.regs)?;
            out.write_all(&[ay.selected_reg])?;
        }
        out.write_all(&self.cycles_pending.to_le_bytes())?;
        // version 2: SSI263 state
        for ssi in &self.ssi {
            out.write_all(&ssi.regs)?;
            out.write_all(&ssi.ready_countdown.to_le_bytes())?;
            out.write_all(&[ssi.powered as u8])?;
        }
        Ok(())
    }

    fn load_state(&mut self, src: &mut dyn Read, _version: u32) -> Result<()> {
        let mut ver = [0u8; 1];
        src.read_exact(&mut ver)?;
        // version 1 and 2: VIA and AY state
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
        // version 2: SSI263 state
        if ver[0] >= 2 {
            for ssi in &mut self.ssi {
                src.read_exact(&mut ssi.regs)?;
                let mut cd = [0u8; 8];
                src.read_exact(&mut cd)?;
                ssi.ready_countdown = u64::from_le_bytes(cd);
                let mut pw = [0u8; 1];
                src.read_exact(&mut pw)?;
                ssi.powered = pw[0] != 0;
            }
        }
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
