//! SAM (Software Automated Mouth) — 8-bit DAC audio card.
//!
//! Any write to slot I/O $C0Nx outputs a sample (bit 7 inverted = signed).
//! The card also records phoneme writes for external consumption.
//!
//! Reference: source/SAM.cpp

use crate::card::{Card, CardType};
use crate::error::Result;
use std::io::{Read, Write};

pub struct SamCard {
    slot: usize,
    /// Queue of DAC samples waiting to be mixed into audio output.
    samples: Vec<f32>,
    /// Accumulates excess cycles for fractional sample timing.
    cycles_rem: f64,
    /// Phoneme output buffer — records raw byte values written to the card.
    phoneme_buffer: Vec<u8>,
    /// Countdown of buzz samples remaining (generated when phonemes are written).
    buzz_samples_remaining: u32,
    /// Phase accumulator for buzz tone generation.
    buzz_phase: f64,
}

impl SamCard {
    pub fn new(slot: usize) -> Self {
        Self {
            slot,
            samples: Vec::with_capacity(1024),
            cycles_rem: 0.0,
            phoneme_buffer: Vec::new(),
            buzz_samples_remaining: 0,
            buzz_phase: 0.0,
        }
    }

    /// Drain the phoneme buffer — returns all phoneme bytes written since the
    /// last call. Useful for external speech synthesis or logging.
    pub fn take_phonemes(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.phoneme_buffer)
    }
}

impl Card for SamCard {
    fn card_type(&self) -> CardType {
        CardType::Sam
    }
    fn slot(&self) -> usize {
        self.slot
    }

    fn io_read(&mut self, _offset: u8, _cycles: u64) -> u8 {
        0xFF
    }
    fn io_write(&mut self, _offset: u8, _value: u8, _cycles: u64) {}

    fn slot_io_read(&mut self, _reg: u8, _cycles: u64) -> u8 {
        0xFF
    }

    fn slot_io_write(&mut self, _reg: u8, value: u8, _cycles: u64) {
        // Record the phoneme byte
        self.phoneme_buffer.push(value);

        // Invert bit 7 to convert from unsigned to signed representation
        let signed = (value ^ 0x80) as i8;
        // Convert to f32 in [-1.0, 1.0]
        let sample = signed as f32 / 128.0;
        self.samples.push(sample);

        // Trigger a brief buzz (~50ms at 22050 Hz) so that phoneme writes
        // produce audible output even without full speech synthesis.
        self.buzz_samples_remaining = self.buzz_samples_remaining.saturating_add(1100);
        // Cap at ~200ms to avoid unbounded growth
        self.buzz_samples_remaining = self.buzz_samples_remaining.min(4410);
    }

    fn fill_audio(&mut self, out: &mut Vec<f32>, _cycles_elapsed: u64, sample_rate: u32) {
        if self.samples.is_empty() && self.buzz_samples_remaining == 0 {
            return;
        }

        // If no output buffer, discard
        if out.is_empty() {
            self.samples.clear();
            self.buzz_samples_remaining = 0;
            return;
        }

        // Mix DAC samples into output (distributed evenly)
        if !self.samples.is_empty() {
            let ratio = self.samples.len() as f64 / out.len() as f64;
            let mut src_pos = 0.0f64;
            for dst in out.iter_mut() {
                let idx = (src_pos as usize).min(self.samples.len() - 1);
                *dst += self.samples[idx] * 0.5; // mix at half volume
                src_pos += ratio;
            }
            self.samples.clear();
        }

        // Generate buzz tone for phoneme activity (220 Hz square-ish wave)
        if self.buzz_samples_remaining > 0 {
            let buzz_freq = 220.0_f64;
            let sr = sample_rate as f64;
            let n = (self.buzz_samples_remaining as usize).min(out.len());
            for sample in out.iter_mut().take(n) {
                let phase_in_period = (self.buzz_phase * buzz_freq / sr).fract();
                let buzz = if phase_in_period < 0.5 {
                    0.15_f32
                } else {
                    -0.15_f32
                };
                *sample += buzz;
                self.buzz_phase += 1.0;
            }
            if n >= self.buzz_samples_remaining as usize {
                self.buzz_samples_remaining = 0;
            } else {
                self.buzz_samples_remaining -= n as u32;
            }
        }
    }

    fn reset(&mut self, _power_cycle: bool) {
        self.samples.clear();
        self.cycles_rem = 0.0;
        self.phoneme_buffer.clear();
        self.buzz_samples_remaining = 0;
        self.buzz_phase = 0.0;
    }

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

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_phoneme_buffer_records_writes() {
        let mut card = SamCard::new(4);
        card.slot_io_write(0x00, 0x10, 0);
        card.slot_io_write(0x00, 0x20, 0);
        card.slot_io_write(0x00, 0x30, 0);
        let phonemes = card.take_phonemes();
        assert_eq!(phonemes, vec![0x10, 0x20, 0x30]);
        // Second drain should be empty
        assert!(card.take_phonemes().is_empty());
    }

    #[test]
    fn test_dac_samples_generated() {
        let mut card = SamCard::new(4);
        // Write silence (0x80 XOR 0x80 = 0 → 0.0)
        card.slot_io_write(0x00, 0x80, 0);
        assert_eq!(card.samples.len(), 1);
        assert!((card.samples[0]).abs() < 0.01);
    }

    #[test]
    fn test_fill_audio_mixes_samples() {
        let mut card = SamCard::new(4);
        card.slot_io_write(0x00, 0xFF, 0); // max positive
        let mut out = vec![0.0_f32; 10];
        card.fill_audio(&mut out, 0, 22050);
        // Output should be non-zero (DAC sample mixed in)
        assert!(out[0].abs() > 0.01, "fill_audio should mix DAC samples");
    }

    #[test]
    fn test_buzz_generated_on_phoneme_write() {
        let mut card = SamCard::new(4);
        card.slot_io_write(0x00, 0x42, 0);
        assert!(card.buzz_samples_remaining > 0, "Buzz should be triggered");
        // Fill some audio and check it's non-silent
        card.samples.clear(); // clear DAC sample so only buzz contributes
        let mut out = vec![0.0_f32; 100];
        card.fill_audio(&mut out, 0, 22050);
        let has_audio = out.iter().any(|s| s.abs() > 0.01);
        assert!(has_audio, "Buzz tone should produce audible output");
    }

    #[test]
    fn test_reset_clears_state() {
        let mut card = SamCard::new(4);
        card.slot_io_write(0x00, 0x42, 0);
        card.reset(true);
        assert!(card.samples.is_empty());
        assert!(card.phoneme_buffer.is_empty());
        assert_eq!(card.buzz_samples_remaining, 0);
    }
}
