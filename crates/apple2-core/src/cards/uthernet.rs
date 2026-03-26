//! Uthernet I and II card stubs.
//!
//! The Uthernet I is based on the CS8900A ethernet controller.
//! The Uthernet II is based on the WIZnet W5100.
//! These stubs occupy the slots without providing actual network functionality.
//!
//! Reference: source/Uthernet1.cpp, source/Uthernet2.cpp

use std::io::{Read, Write};
use crate::card::{Card, CardType};
use crate::error::Result;

pub struct UthernCard {
    slot:      usize,
    card_type: CardType,
    regs:      [u8; 16],
}

impl UthernCard {
    pub fn new_uthernet1(slot: usize) -> Self {
        Self { slot, card_type: CardType::Uthernet, regs: [0u8; 16] }
    }
    pub fn new_uthernet2(slot: usize) -> Self {
        Self { slot, card_type: CardType::Uthernet2, regs: [0u8; 16] }
    }
}

impl Card for UthernCard {
    fn card_type(&self) -> CardType { self.card_type }
    fn slot(&self) -> usize { self.slot }
    fn io_read(&mut self, _offset: u8, _cycles: u64) -> u8 { 0xFF }
    fn io_write(&mut self, _offset: u8, _value: u8, _cycles: u64) {}
    fn slot_io_read(&mut self, reg: u8, _cycles: u64) -> u8 {
        self.regs[(reg & 0x0F) as usize]
    }
    fn slot_io_write(&mut self, reg: u8, val: u8, _cycles: u64) {
        self.regs[(reg & 0x0F) as usize] = val;
    }
    fn reset(&mut self, _power_cycle: bool) { self.regs.fill(0); }
    fn update(&mut self, _cycles: u64) {}
    fn save_state(&self, _out: &mut dyn Write) -> Result<()> { Ok(()) }
    fn load_state(&mut self, _src: &mut dyn Read, _version: u32) -> Result<()> { Ok(()) }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}
