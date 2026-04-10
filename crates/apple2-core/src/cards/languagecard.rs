//! 16K Language Card for Apple II / II+ models.
//!
//! Provides 16K of RAM at $D000–$FFFF, installable in slot 0.
//! On the Apple IIe and later this functionality is built-in; this card
//! is only needed for the original Apple II and Apple II+.
//!
//! The Language Card uses the same $C080–$C08F soft-switch mechanism already
//! handled by the bus (`lc_switch`).  This card simply holds the 16K of RAM
//! that the bus pages in/out via the `take_lc_bank_swap()` / `store_lc_bank()`
//! Card trait methods.
//!
//! Reference: source/LanguageCard.cpp (LanguageCardUnit class)

use crate::card::{Card, CardType};
use crate::error::Result;
use std::io::{Read, Write};

const LC_SIZE: usize = 16384; // $D000–$FFFF (actually $C000–$FFFF for bus swap)

pub struct LanguageCardCard {
    slot: usize,
    /// The Language Card RAM (16K).  When the bus activates the LC the contents
    /// are swapped into `aux_ram[$C000..]` and the displaced ROM/RAM is stored
    /// back here via `store_lc_bank()`.
    bank: Box<[u8; LC_SIZE]>,
    /// Pending swap data ready for the bus to pick up.
    pending_swap: Option<Box<[u8; LC_SIZE]>>,
    /// Set to true after first write through `slot_io_write` to trigger the
    /// initial bank swap.
    initialised: bool,
}

impl LanguageCardCard {
    pub fn new(slot: usize) -> Self {
        Self {
            slot,
            bank: Box::new([0u8; LC_SIZE]),
            pending_swap: None,
            initialised: false,
        }
    }
}

impl Card for LanguageCardCard {
    fn card_type(&self) -> CardType {
        CardType::LanguageCard
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
    fn slot_io_write(&mut self, _reg: u8, _value: u8, _cycles: u64) {
        // Language Card control ($C080–$C08F) is handled by the bus directly
        // via the standard `lc_switch()` mechanism, not by the card.
        // Nothing to do here.
    }

    fn take_lc_bank_swap(&mut self) -> Option<Box<[u8; LC_SIZE]>> {
        self.pending_swap.take()
    }

    fn store_lc_bank(&mut self, data: &[u8; LC_SIZE]) {
        self.bank.copy_from_slice(data);
    }

    fn reset(&mut self, _power_cycle: bool) {
        self.bank.fill(0);
        self.pending_swap = None;
        self.initialised = false;
    }

    fn update(&mut self, _cycles: u64) {}

    fn save_state(&self, out: &mut dyn Write) -> Result<()> {
        out.write_all(&[1u8])?; // version
        out.write_all(self.bank.as_ref())?;
        Ok(())
    }

    fn load_state(&mut self, src: &mut dyn Read, _version: u32) -> Result<()> {
        let mut ver = [0u8; 1];
        src.read_exact(&mut ver)?;
        src.read_exact(self.bank.as_mut())?;
        self.pending_swap = None;
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
