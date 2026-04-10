//! 80-column text card emulation.
//!
//! Provides the $Cn ROM page presence that signals 80-column capability on the
//! Apple IIe. The video renderer already handles 80-column display via aux RAM
//! interleaving — this card just needs to occupy a slot so that software sees
//! the card and the IIe ROM's internal 80-column firmware is activated.
//!
//! The card is normally placed in slot 3 so that it responds at $C300–$C3FF.
//! The self-identification byte at $C300 must be $C3 so that the IIe firmware
//! can detect the card's presence.
//!
//! Reference: source/Video.cpp, source/Configuration/PageSlot.cpp

use crate::card::{Card, CardType};
use crate::error::Result;
use std::io::{Read, Write};

// ── 256-byte ROM image ────────────────────────────────────────────────────────

/// Build the minimal 80-column card ROM.
///
/// The only byte the IIe firmware inspects is offset 0 ($Cn00), which must
/// equal $C3 for slot-3 detection to succeed.  All other bytes are zero.
fn make_col80_rom() -> Box<[u8; 256]> {
    let mut rom = Box::new([0u8; 256]);
    rom[0] = 0xC3; // self-ID byte: $C3 at $C300 identifies the 80-col card
    rom
}

// ── Col80Card ─────────────────────────────────────────────────────────────────

pub struct Col80Card {
    slot: usize,
    rom: Box<[u8; 256]>,
}

impl Col80Card {
    pub fn new(slot: usize) -> Self {
        Self {
            slot,
            rom: make_col80_rom(),
        }
    }
}

impl Card for Col80Card {
    fn card_type(&self) -> CardType {
        CardType::Col80
    }
    fn slot(&self) -> usize {
        self.slot
    }

    fn io_read(&mut self, offset: u8, _cycles: u64) -> u8 {
        self.rom[offset as usize]
    }

    fn io_write(&mut self, _offset: u8, _value: u8, _cycles: u64) {}

    fn cx_rom(&self) -> Option<&[u8; 256]> {
        Some(&self.rom)
    }

    fn reset(&mut self, _power_cycle: bool) {}
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

// ── Extended80ColCard ─────────────────────────────────────────────────────────

/// Extended 80-column card (Apple IIe Extended 80-Column Text Card).
/// Provides the same ROM presence plus signals aux-RAM capability.
pub struct Extended80ColCard {
    slot: usize,
    rom: Box<[u8; 256]>,
}

impl Extended80ColCard {
    pub fn new(slot: usize) -> Self {
        Self {
            slot,
            rom: make_col80_rom(),
        }
    }
}

impl Card for Extended80ColCard {
    fn card_type(&self) -> CardType {
        CardType::Extended80Col
    }
    fn slot(&self) -> usize {
        self.slot
    }

    fn io_read(&mut self, offset: u8, _cycles: u64) -> u8 {
        self.rom[offset as usize]
    }

    fn io_write(&mut self, _offset: u8, _value: u8, _cycles: u64) {}

    fn cx_rom(&self) -> Option<&[u8; 256]> {
        Some(&self.rom)
    }

    fn reset(&mut self, _power_cycle: bool) {}
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
