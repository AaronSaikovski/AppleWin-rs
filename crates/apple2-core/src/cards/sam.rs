//! SAM (Software Automated Mouth) — 8-bit DAC audio card.
//! Any write to slot I/O $C0Nx outputs a sample (bit 7 inverted = signed).
//! Reference: source/SAM.cpp

use std::io::{Read, Write};
use crate::card::{Card, CardType};
use crate::error::Result;

pub struct SamCard {
    slot:         usize,
    /// Queue of DAC samples waiting to be mixed into audio output.
    samples:      Vec<f32>,
    /// Accumulates excess cycles for fractional sample timing.
    cycles_rem:   f64,
}

impl SamCard {
    pub fn new(slot: usize) -> Self {
        Self { slot, samples: Vec::with_capacity(1024), cycles_rem: 0.0 }
    }
}

impl Card for SamCard {
    fn card_type(&self) -> CardType { CardType::Sam }
    fn slot(&self) -> usize { self.slot }

    fn io_read(&mut self, _offset: u8, _cycles: u64) -> u8 { 0xFF }
    fn io_write(&mut self, _offset: u8, _value: u8, _cycles: u64) {}

    fn slot_io_read(&mut self, _reg: u8, _cycles: u64) -> u8 { 0xFF }

    fn slot_io_write(&mut self, _reg: u8, value: u8, _cycles: u64) {
        // Invert bit 7 to convert from unsigned to signed representation
        let signed = (value ^ 0x80) as i8;
        // Convert to f32 in [-1.0, 1.0]
        let sample = signed as f32 / 128.0;
        self.samples.push(sample);
    }

    fn fill_audio(&mut self, out: &mut Vec<f32>, _cycles_elapsed: u64, _sample_rate: u32) {
        // Mix any queued DAC samples into the output buffer.
        // Since we don't have cycle-accurate timing here, distribute evenly.
        if self.samples.is_empty() { return; }
        // If no output samples, just discard
        if out.is_empty() { self.samples.clear(); return; }
        let ratio = self.samples.len() as f64 / out.len() as f64;
        let mut src_pos = 0.0f64;
        for dst in out.iter_mut() {
            let idx = (src_pos as usize).min(self.samples.len() - 1);
            *dst += self.samples[idx] * 0.5; // mix at half volume
            src_pos += ratio;
        }
        self.samples.clear();
    }

    fn reset(&mut self, _power_cycle: bool) { self.samples.clear(); self.cycles_rem = 0.0; }
    fn update(&mut self, _cycles: u64) {}
    fn save_state(&self, out: &mut dyn Write) -> Result<()> {
        out.write_all(&[1u8])?;
        // samples buffer is transient audio — don't save it
        Ok(())
    }

    fn load_state(&mut self, src: &mut dyn Read, _version: u32) -> Result<()> {
        let mut ver = [0u8; 1];
        src.read_exact(&mut ver)?;
        Ok(())
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}
