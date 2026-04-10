//! Mouse Interface card emulation (Apple Mouse Interface Card).
//!
//! Provides a basic pass-through for the mouse firmware ROM and exposes
//! mouse position/button state via soft switches in the slot I/O space.
//!
//! Reference: source/MouseInterface.cpp, resource/MouseInterface.rom

use crate::card::{Card, CardType};
use crate::error::Result;
use std::io::{Read, Write};

static MOUSE_FIRMWARE: &[u8; 256] = {
    const ROM: &[u8] = include_bytes!("../../roms/MouseInterface.rom");
    unsafe { &*(ROM.as_ptr() as *const [u8; 256]) }
};

// ── I/O register offsets (slot peripheral space $C0x0–$C0xF) ──────────────
// The Apple Mouse Interface uses a 6821 PIA.
// We expose a simplified model: mouse state readable via PIA port A.

pub struct MouseCard {
    slot: usize,
    x: i16,
    y: i16,
    buttons: u8,

    // Minimal 6821 PIA registers
    pra: u8, // Port A data
    prb: u8, // Port B data
    ddra: u8,
    ddrb: u8,
    cra: u8,
    crb: u8,
}

impl MouseCard {
    pub fn new(slot: usize) -> Self {
        Self {
            slot,
            x: 0,
            y: 0,
            buttons: 0,
            pra: 0,
            prb: 0,
            ddra: 0,
            ddrb: 0,
            cra: 0,
            crb: 0,
        }
    }
}

impl Card for MouseCard {
    fn card_type(&self) -> CardType {
        CardType::MouseInterface
    }
    fn slot(&self) -> usize {
        self.slot
    }

    fn io_read(&mut self, offset: u8, _cycles: u64) -> u8 {
        *MOUSE_FIRMWARE.get(offset as usize).unwrap_or(&0xFF)
    }

    fn io_write(&mut self, _offset: u8, _value: u8, _cycles: u64) {}

    fn cx_rom(&self) -> Option<&[u8; 256]> {
        Some(MOUSE_FIRMWARE)
    }

    fn slot_io_read(&mut self, reg: u8, _cycles: u64) -> u8 {
        // Expose mouse state: port A = X low byte, port B = Y low byte
        // Button in bit 7 of CRA
        match reg & 0x03 {
            0x0 => {
                if self.cra & 0x04 != 0 {
                    self.pra
                } else {
                    self.ddra
                }
            }
            0x1 => self.cra,
            0x2 => {
                if self.crb & 0x04 != 0 {
                    self.prb
                } else {
                    self.ddrb
                }
            }
            0x3 => self.crb,
            _ => 0xFF,
        }
    }

    fn slot_io_write(&mut self, reg: u8, val: u8, _cycles: u64) {
        match reg & 0x03 {
            0x0 => {
                if self.cra & 0x04 != 0 {
                    self.pra = val;
                } else {
                    self.ddra = val;
                }
            }
            0x1 => self.cra = val & 0x3F,
            0x2 => {
                if self.crb & 0x04 != 0 {
                    self.prb = val;
                } else {
                    self.ddrb = val;
                }
            }
            0x3 => self.crb = val & 0x3F,
            _ => {}
        }
    }

    fn set_mouse_state(&mut self, x: i16, y: i16, buttons: u8) {
        self.x = x;
        self.y = y;
        self.buttons = buttons;
        // Map position to PIA port data
        self.pra = (x & 0xFF) as u8;
        self.prb = (y & 0xFF) as u8;
        // Button 0 sets bit 7 of CRA
        if buttons & 0x01 != 0 {
            self.cra |= 0x80;
        } else {
            self.cra &= !0x80;
        }
    }

    fn reset(&mut self, _power_cycle: bool) {
        self.pra = 0;
        self.prb = 0;
        self.ddra = 0;
        self.ddrb = 0;
        self.cra = 0;
        self.crb = 0;
    }

    fn update(&mut self, _cycles: u64) {}

    fn save_state(&self, _out: &mut dyn Write) -> Result<()> {
        Ok(())
    }
    fn load_state(&mut self, _src: &mut dyn Read, _version: u32) -> Result<()> {
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
