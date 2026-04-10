//! SNES MAX controller card — two SNES controller serial interface.
//! Reference: source/SNESMAX.cpp

use crate::card::{Card, CardType};
use crate::error::Result;
use std::io::{Read, Write};

/// Number of bits per controller (16 buttons + 1 plugged-in status bit).
const BUTTON_COUNT: u8 = 17;

pub struct SnesMaxCard {
    slot: usize,
    btn_index: u8,   // current bit position (0..BUTTON_COUNT)
    ctrl1_bits: u32, // latched controller 1 state (inverted)
    ctrl2_bits: u32, // latched controller 2 state (inverted)
}

impl SnesMaxCard {
    pub fn new(slot: usize) -> Self {
        Self {
            slot,
            btn_index: 0,
            // Both controllers: all buttons released, plugged-in bit set (bit 16 = 1 → after invert = 0)
            // When no controller: NOT plugged-in so bit 16 = 0 → after invert = 1
            ctrl1_bits: 0,
            ctrl2_bits: 0,
        }
    }

    fn latch_controllers(&mut self) {
        // With no controllers connected, all bits = 0xFFFF (all high = not pressed, not plugged in)
        // Inverted (as in C++ ~controller): all 1s become all 0s
        self.ctrl1_bits = 0;
        self.ctrl2_bits = 0;
        self.btn_index = 0;
    }
}

impl Card for SnesMaxCard {
    fn card_type(&self) -> CardType {
        CardType::SnesMax
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
            0x00 => {
                // Data: bit 7 = ctrl1 current bit, bit 6 = ctrl2 current bit (active high)
                if self.btn_index < BUTTON_COUNT {
                    let b1 = ((self.ctrl1_bits >> self.btn_index) & 1) as u8;
                    let b2 = ((self.ctrl2_bits >> self.btn_index) & 1) as u8;
                    (b1 << 7) | (b2 << 6)
                } else {
                    0x00
                }
            }
            _ => 0xFF,
        }
    }

    fn slot_io_write(&mut self, reg: u8, _value: u8, _cycles: u64) {
        match reg & 0x0F {
            0x00 => self.latch_controllers(), // Latch: snapshot state, reset index
            0x01 => {
                // Clock: advance to next bit
                if self.btn_index < BUTTON_COUNT {
                    self.btn_index += 1;
                }
            }
            _ => {}
        }
    }

    fn reset(&mut self, _power_cycle: bool) {
        self.btn_index = 0;
        self.ctrl1_bits = 0;
        self.ctrl2_bits = 0;
    }

    fn update(&mut self, _cycles: u64) {}
    fn save_state(&self, out: &mut dyn Write) -> Result<()> {
        out.write_all(&[1u8])?;
        Ok(()) // controller state is volatile
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
