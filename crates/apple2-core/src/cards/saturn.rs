//! Saturn 128K expansion card emulation.
//!
//! Provides up to 8 banks of 16K language card RAM, switchable via slot I/O.
//! Bank 0 is initially live in `aux_ram[$C000..$FFFF]`.  When a bank switch
//! occurs the bus swaps the current contents out and the requested bank in.
//!
//! I/O: $C0Nx where N = 8 + slot
//!   (reg & 0x04) == 0  → Language Card control (handled by bus `lc_switch`)
//!   (reg & 0x04) != 0  → Bank select: bank = (reg >> 1) & 0x07
//!
//! Reference: source/LanguageCard.cpp (SaturnCard class)

use crate::card::{Card, CardType};
use crate::error::Result;
use std::io::{Read, Write};

const LC_SIZE: usize = 16384; // $C000..$FFFF in aux_ram
const MAX_BANKS: usize = 8;

pub struct Saturn128KCard {
    slot: usize,
    active_bank: u8,
    /// Bank that was active before the most recent switch (used by `store_lc_bank`).
    prev_bank: u8,
    /// All 8 banks stored in the card.  Bank 0 is initially zeroed because
    /// aux_ram starts zeroed; when the bus displaces bank 0 it gets stored here.
    banks: [Box<[u8; LC_SIZE]>; MAX_BANKS],
    /// Pending swap data to load into aux_ram on the next `take_lc_bank_swap` call.
    pending_swap: Option<Box<[u8; LC_SIZE]>>,
}

impl Saturn128KCard {
    pub fn new(slot: usize) -> Self {
        Self {
            slot,
            active_bank: 0,
            prev_bank: 0,
            banks: std::array::from_fn(|_| Box::new([0u8; LC_SIZE])),
            pending_swap: None,
        }
    }

    fn switch_to_bank(&mut self, new_bank: u8) {
        let new_bank = new_bank.min((MAX_BANKS - 1) as u8);
        if self.active_bank == new_bank {
            return;
        }
        self.prev_bank = self.active_bank;
        self.active_bank = new_bank;
        // Provide the bus with the data for the new bank so it can load it into aux_ram.
        self.pending_swap = Some(Box::new(*self.banks[new_bank as usize]));
    }
}

impl Card for Saturn128KCard {
    fn card_type(&self) -> CardType {
        CardType::Saturn128K
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

    fn slot_io_write(&mut self, reg: u8, _value: u8, _cycles: u64) {
        if reg & 0x04 != 0 {
            // Bank-select register: bank number encoded in bits 3:1
            let bank = (reg >> 1) & 0x07;
            self.switch_to_bank(bank);
        }
        // Language Card control (reg & 0x04 == 0) is handled by the bus via
        // normal $C080–$C08F dispatch.
    }

    fn take_lc_bank_swap(&mut self) -> Option<Box<[u8; LC_SIZE]>> {
        self.pending_swap.take()
    }

    fn store_lc_bank(&mut self, data: &[u8; LC_SIZE]) {
        // Copy the displaced bank data into our local storage.
        self.banks[self.prev_bank as usize].copy_from_slice(data);
    }

    fn reset(&mut self, _power_cycle: bool) {
        self.active_bank = 0;
        self.prev_bank = 0;
        self.pending_swap = None;
        for b in &mut self.banks {
            b.fill(0);
        }
    }

    fn update(&mut self, _cycles: u64) {}

    fn save_state(&self, out: &mut dyn Write) -> Result<()> {
        out.write_all(&[1u8])?; // version
        out.write_all(&[self.active_bank, self.prev_bank])?;
        for bank in &self.banks {
            out.write_all(bank.as_ref())?;
        }
        Ok(())
    }

    fn load_state(&mut self, src: &mut dyn Read, _version: u32) -> Result<()> {
        let mut ver = [0u8; 1];
        src.read_exact(&mut ver)?;
        let mut idx = [0u8; 2];
        src.read_exact(&mut idx)?;
        self.active_bank = idx[0].min((MAX_BANKS - 1) as u8);
        self.prev_bank = idx[1].min((MAX_BANKS - 1) as u8);
        for bank in &mut self.banks {
            src.read_exact(bank.as_mut())?;
        }
        self.pending_swap = None;
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
