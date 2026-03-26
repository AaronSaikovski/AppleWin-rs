//! VidHD card stub.
//!
//! The VidHD is a modern HDMI output card for the Apple IIe. It provides
//! super hi-res video output. This stub just occupies the slot so that
//! software that checks for its presence doesn't crash.
//!
//! Reference: source/Video.cpp

use std::io::{Read, Write};
use crate::card::{Card, CardType};
use crate::error::Result;

pub struct VidHdCard {
    slot: usize,
}

impl VidHdCard {
    pub fn new(slot: usize) -> Self { Self { slot } }
}

impl Card for VidHdCard {
    fn card_type(&self) -> CardType { CardType::VidHD }
    fn slot(&self) -> usize { self.slot }
    fn io_read(&mut self, _offset: u8, _cycles: u64) -> u8 { 0xFF }
    fn io_write(&mut self, _offset: u8, _value: u8, _cycles: u64) {}
    fn reset(&mut self, _power_cycle: bool) {}
    fn update(&mut self, _cycles: u64) {}
    fn save_state(&self, _out: &mut dyn Write) -> Result<()> { Ok(()) }
    fn load_state(&mut self, _src: &mut dyn Read, _version: u32) -> Result<()> { Ok(()) }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}
