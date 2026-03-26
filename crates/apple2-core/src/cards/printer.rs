//! Generic Parallel Printer card emulation.
//!
//! On the Apple II, the parallel printer card maps:
//!   slot_io reg 0: data byte to print
//!   slot_io reg 1: control/strobe — write triggers output
//!
//! Output is collected in a buffer; when the buffer grows large or
//! a form-feed byte is seen, it's flushed to a file (if configured).
//!
//! Reference: source/Printer.cpp

use std::io::{Read, Write};
use crate::card::{Card, CardType};
use crate::error::Result;

pub struct PrinterCard {
    slot: usize,
    data: u8,
    buffer: Vec<u8>,
}

impl PrinterCard {
    pub fn new(slot: usize) -> Self {
        Self { slot, data: 0, buffer: Vec::new() }
    }

    fn strobe(&mut self) {
        // Output the data byte
        if self.data == b'\x0C' {
            // Form feed — flush
            self.flush();
        } else {
            self.buffer.push(self.data);
            if self.buffer.len() >= 4096 {
                self.flush();
            }
        }
    }

    fn flush(&mut self) {
        if self.buffer.is_empty() { return; }
        // Try to append to printer.txt in the current directory
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true).append(true).open("printer.txt")
        {
            let _ = f.write_all(&self.buffer);
        }
        self.buffer.clear();
    }
}

impl Card for PrinterCard {
    fn card_type(&self) -> CardType { CardType::GenericPrinter }
    fn slot(&self) -> usize { self.slot }

    fn io_read(&mut self, _offset: u8, _cycles: u64) -> u8 { 0xFF }
    fn io_write(&mut self, _offset: u8, _value: u8, _cycles: u64) {}

    fn slot_io_read(&mut self, reg: u8, _cycles: u64) -> u8 {
        match reg {
            0 => self.data,
            1 => 0x00, // Status: ready (bit 7 = 0 = not busy)
            _ => 0xFF,
        }
    }

    fn slot_io_write(&mut self, reg: u8, val: u8, _cycles: u64) {
        match reg {
            0 => self.data = val,
            1 => self.strobe(), // Strobe — output the data byte
            _ => {}
        }
    }

    fn reset(&mut self, _power_cycle: bool) { self.buffer.clear(); }
    fn update(&mut self, _cycles: u64) {}
    fn save_state(&self, _out: &mut dyn Write) -> Result<()> { Ok(()) }
    fn load_state(&mut self, _src: &mut dyn Read, _version: u32) -> Result<()> { Ok(()) }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}
