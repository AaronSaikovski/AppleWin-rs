//! FourPlay — 4-port digital joystick interface card.
//! Slot I/O reads at offsets 0-3 return the state of joysticks 1-4.
//! Reference: source/FourPlay.cpp

use crate::card::{Card, CardType};
use crate::error::Result;
use std::io::{Read, Write};

/// Bit 5 is always high on a resting controller (hardware marker).
const STATIONARY: u8 = 0x20;

pub struct FourPlayCard {
    slot: usize,
    /// State for each joystick port (0 = up/open, bits per above format).
    /// Default = STATIONARY (no input, centered).
    joy: [u8; 4],
}

impl FourPlayCard {
    pub fn new(slot: usize) -> Self {
        Self {
            slot,
            joy: [STATIONARY; 4],
        }
    }

    /// Update joystick state for port `idx` (0-3).
    pub fn set_joystick(&mut self, idx: usize, state: u8) {
        if idx < 4 {
            self.joy[idx] = state | STATIONARY;
        }
    }
}

impl Card for FourPlayCard {
    fn card_type(&self) -> CardType {
        CardType::FourPlay
    }
    fn slot(&self) -> usize {
        self.slot
    }

    fn io_read(&mut self, _offset: u8, _cycles: u64) -> u8 {
        0xFF
    }
    fn io_write(&mut self, _offset: u8, _value: u8, _cycles: u64) {}

    fn slot_io_read(&mut self, reg: u8, _cycles: u64) -> u8 {
        match reg & 0x0F {
            0..=3 => self.joy[reg as usize],
            _ => 0xFF,
        }
    }

    fn slot_io_write(&mut self, _reg: u8, _value: u8, _cycles: u64) {}

    fn reset(&mut self, _power_cycle: bool) {
        self.joy = [STATIONARY; 4];
    }

    fn update(&mut self, _cycles: u64) {}
    fn save_state(&self, out: &mut dyn Write) -> Result<()> {
        out.write_all(&[1u8])?;
        out.write_all(&self.joy)?;
        Ok(())
    }

    fn load_state(&mut self, src: &mut dyn Read, _version: u32) -> Result<()> {
        let mut ver = [0u8; 1];
        src.read_exact(&mut ver)?;
        src.read_exact(&mut self.joy)?;
        Ok(())
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
