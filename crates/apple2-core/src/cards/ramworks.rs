//! RamWorks III expansion card emulation.
//!
//! Provides up to 256 banks of 64K auxiliary RAM.  Bank switching is controlled
//! by a write to soft-switch $C073 (not a slot I/O register), so the actual
//! bank selection is performed by the bus.  This struct acts as a marker card
//! so that `CardType::RamWorksIII` can be installed in a slot.
//!
//! Reference: source/Memory.cpp (RamWorks III section)

use std::io::{Read, Write};
use crate::card::{Card, CardType};
use crate::error::Result;

/// Marker card for RamWorks III.
///
/// The actual bank switching (up to 256 × 64K aux banks) is handled by the bus
/// when it sees a write to $C073.
pub struct RamWorksCard {
    slot: usize,
}

impl RamWorksCard {
    pub fn new(slot: usize) -> Self {
        Self { slot }
    }
}

impl Card for RamWorksCard {
    fn card_type(&self) -> CardType { CardType::RamWorksIII }
    fn slot(&self) -> usize { self.slot }

    fn io_read(&mut self, _offset: u8, _cycles: u64) -> u8 { 0xFF }
    fn io_write(&mut self, _offset: u8, _value: u8, _cycles: u64) {}

    fn reset(&mut self, _power_cycle: bool) {}
    fn update(&mut self, _cycles: u64) {}

    fn save_state(&self, out: &mut dyn Write) -> Result<()> {
        out.write_all(&[1u8])?;
        Ok(())
    }

    fn load_state(&mut self, src: &mut dyn Read, _version: u32) -> Result<()> {
        let mut ver = [0u8; 1];
        src.read_exact(&mut ver)?;
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}
