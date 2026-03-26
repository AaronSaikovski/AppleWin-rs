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

use std::io::{Read, Write};
use apple2_audio::ay8910::Ay8910;
use crate::card::{Card, CardType};
use crate::cards::ssi263::Ssi263;
use crate::error::Result;

// Mockingboard firmware ROM (256 bytes, from AppleWin resource)
static MB_FIRMWARE: &[u8; 256] = {
    // The Mockingboard-D.rom is 2 KB; we take the first 256 bytes as the $Cn page
    const ROM: &[u8] = include_bytes!("../../roms/Mockingboard-D.rom");
    // Safety: ROM is guaranteed >= 256 bytes at compile time
    // We use a const reference to the first 256 bytes
    unsafe { &*(ROM.as_ptr() as *const [u8; 256]) }
};

// ── Simplified 6522 VIA ────────────────────────────────────────────────────

/// Minimal 6522 VIA model — only the registers needed for AY control.
struct Via {
    ora:  u8,   // Output Register A (data bus to AY)
    orb:  u8,   // Output Register B (BDIR/BC1 control lines)
    ddra: u8,   // Data Direction Register A
    ddrb: u8,   // Data Direction Register B
    // Timer registers
    t1cl: u8,   // T1 counter low
    t1ch: u8,   // T1 counter high
    t1ll: u8,   // T1 latch low
    t1lh: u8,   // T1 latch high
    t2cl: u8,   // T2 counter low
    t2ch: u8,   // T2 counter high
    sr:   u8,   // Shift Register
    acr:  u8,   // Auxiliary Control Register
    pcr:  u8,   // Peripheral Control Register
    ifr:  u8,   // Interrupt Flag Register
    ier:  u8,   // Interrupt Enable Register
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
            ora: 0, orb: 0, ddra: 0, ddrb: 0,
            t1cl: 0, t1ch: 0, t1ll: 0, t1lh: 0,
            t2cl: 0, t2ch: 0, sr: 0, acr: 0, pcr: 0,
            ifr: 0, ier: 0,
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
                    let new_count = if period > 0 { period - (remaining % period) } else { 0 };
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
            // T1CL: update latch and counter low byte
            0x4 => { self.t1cl = val; self.t1ll = val; }
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
            0xA => self.sr   = val,
            0xB => self.acr  = val,
            0xC => self.pcr  = val,
            // IFR: writing 1s to bits CLEARS them (6522 behavior)
            0xD => self.ifr &= !(val & 0x7F),
            0xE => self.ier  = val,
            _   => {}
        }
    }
}

// ── MockingboardCard ───────────────────────────────────────────────────────

/// AY clock for Apple II: 1.0 MHz (1,022,727 Hz PAL / 1,020,484 Hz NTSC)
const AY_CLOCK: f64 = 1_020_484.0;

pub struct MockingboardCard {
    slot: usize,
    via:  [Via; 2],
    ay:   [Ay8910; 2],
    /// Two SSI263 speech chips (one per VIA, only present in Mockingboard D).
    ssi:  [Ssi263; 2],

    /// Accumulated cycles since last audio drain.
    cycles_pending: u64,
}

impl MockingboardCard {
    pub fn new(slot: usize) -> Self {
        Self {
            slot,
            via: [Via::new(), Via::new()],
            ay:  [Ay8910::new(), Ay8910::new()],
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
    fn card_type(&self) -> CardType { CardType::Mockingboard }
    fn slot(&self) -> usize { self.slot }

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
        for s in &mut out[base..] { *s *= 0.5; }
    }

    fn reset(&mut self, _power_cycle: bool) {
        for via in &mut self.via { via.reset(); }
        for ay  in &mut self.ay  { ay.reset(); }
        for ssi in &mut self.ssi { ssi.reset(); }
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
        self.via.iter().any(|v| v.ifr & v.ier & 0x7F != 0)
    }

    fn save_state(&self, out: &mut dyn Write) -> Result<()> {
        out.write_all(&[2u8])?; // version
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

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}
