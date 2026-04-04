//! Uthernet I and II card emulation.
//!
//! The Uthernet I is based on the CS8900A ethernet controller.
//! The Uthernet II is based on the WIZnet W5100.
//!
//! This implementation provides proper register emulation so that software
//! probing for the card will detect it correctly, but no actual networking
//! is performed.
//!
//! Reference: source/Uthernet1.cpp, source/Uthernet2.cpp

use std::io::{Read, Write};
use crate::card::{Card, CardType};
use crate::error::Result;

// ── CS8900A constants (Uthernet I) ───────────────────────────────────────────

/// CS8900A Product ID (read from PacketPage 0x0000-0x0001).
const CS8900A_PRODUCT_ID: u16 = 0x630E;
/// CS8900A Product ID register (little-endian in PacketPage).
const CS8900A_REVISION: u16 = 0x0C00; // rev C, stepping 0

/// PacketPage size (16-bit address space).
const PP_SIZE: usize = 4096;

// ── W5100 constants (Uthernet II) ─────────────────────────────────────────────

/// W5100 register space size.
const W5100_REG_SIZE: usize = 0x8000;

/// W5100 version register value.
const W5100_VERSION: u8 = 0x04;

/// W5100 common register offsets.
const W5100_MR: u16 = 0x0000;    // Mode Register
const W5100_VERSIONR: u16 = 0x0019; // Version Register

// Number of W5100 sockets.
#[allow(dead_code)]
const W5100_NUM_SOCKETS: usize = 4;

// Socket register base addresses.
#[allow(dead_code)]
const W5100_SOCK_BASE: u16 = 0x0400;
#[allow(dead_code)]
const W5100_SOCK_STRIDE: u16 = 0x0100;

// ── Uthernet I (CS8900A) ─────────────────────────────────────────────────────

/// CS8900A PacketPage-based register emulation.
struct Cs8900a {
    /// 4K PacketPage register file.
    packet_page: Box<[u8; PP_SIZE]>,
    /// PacketPage Pointer (set via I/O regs 0x0A/0x0B).
    pp_ptr: u16,
    /// Auto-increment mode active.
    pp_auto_inc: bool,
}

impl Cs8900a {
    fn new() -> Self {
        let mut pp = Box::new([0u8; PP_SIZE]);
        // Product ID at PP offset 0x0000 (little-endian)
        pp[0x0000] = (CS8900A_PRODUCT_ID & 0xFF) as u8;
        pp[0x0001] = (CS8900A_PRODUCT_ID >> 8) as u8;
        // Product ID / revision at PP offset 0x0002
        pp[0x0002] = (CS8900A_REVISION & 0xFF) as u8;
        pp[0x0003] = (CS8900A_REVISION >> 8) as u8;
        // IO base address at PP offset 0x0020 (default 0x0300)
        pp[0x0020] = 0x00;
        pp[0x0021] = 0x03;
        // SelfST register at PP offset 0x0136 — INITD bit set (init done)
        pp[0x0136] = 0x80;
        pp[0x0137] = 0x00;

        Self {
            packet_page: pp,
            pp_ptr: 0,
            pp_auto_inc: false,
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    /// Read a 16-bit value from the PacketPage at the given offset.
    fn pp_read16(&self, offset: u16) -> u16 {
        let off = (offset as usize) & (PP_SIZE - 1);
        let lo = self.packet_page[off] as u16;
        let hi = self.packet_page[off | 1] as u16;
        lo | (hi << 8)
    }

    /// Write a 16-bit value to the PacketPage at the given offset.
    #[allow(dead_code)]
    fn pp_write16(&mut self, offset: u16, val: u16) {
        let off = (offset as usize) & (PP_SIZE - 1);
        self.packet_page[off] = (val & 0xFF) as u8;
        self.packet_page[off | 1] = (val >> 8) as u8;
    }

    /// Handle I/O read (reg 0x00-0x0F).
    fn io_read(&mut self, reg: u8) -> u8 {
        match reg & 0x0F {
            // RTDATA port (16-bit, but we return byte at a time)
            0x00 => {
                let val = self.packet_page[(self.pp_ptr as usize) & (PP_SIZE - 1)];
                if self.pp_auto_inc {
                    self.pp_ptr = self.pp_ptr.wrapping_add(1);
                }
                val
            }
            0x01 => {
                let val = self.packet_page[(self.pp_ptr as usize | 1) & (PP_SIZE - 1)];
                val
            }
            // PacketPage Pointer low
            0x0A => (self.pp_ptr & 0xFF) as u8,
            // PacketPage Pointer high
            0x0B => (self.pp_ptr >> 8) as u8,
            // PacketPage Data Port 0 (low byte at pp_ptr)
            0x0C => {
                let val = self.pp_read16(self.pp_ptr);
                (val & 0xFF) as u8
            }
            // PacketPage Data Port 0 (high byte at pp_ptr)
            0x0D => {
                let val = self.pp_read16(self.pp_ptr);
                // Auto-increment after reading high byte
                if self.pp_auto_inc {
                    self.pp_ptr = self.pp_ptr.wrapping_add(2);
                }
                (val >> 8) as u8
            }
            _ => 0x00,
        }
    }

    /// Handle I/O write (reg 0x00-0x0F).
    fn io_write(&mut self, reg: u8, val: u8) {
        match reg & 0x0F {
            // PacketPage Pointer low
            0x0A => {
                self.pp_ptr = (self.pp_ptr & 0xFF00) | val as u16;
                // Bit 6 of high nibble sets auto-increment
                self.pp_auto_inc = self.pp_ptr & 0x8000 != 0;
            }
            // PacketPage Pointer high
            0x0B => {
                self.pp_ptr = (self.pp_ptr & 0x00FF) | ((val as u16) << 8);
                self.pp_auto_inc = self.pp_ptr & 0x8000 != 0;
            }
            // PacketPage Data Port 0 low byte
            0x0C => {
                let off = (self.pp_ptr as usize) & (PP_SIZE - 1);
                self.packet_page[off] = val;
            }
            // PacketPage Data Port 0 high byte
            0x0D => {
                let off = (self.pp_ptr as usize | 1) & (PP_SIZE - 1);
                self.packet_page[off] = val;
                if self.pp_auto_inc {
                    self.pp_ptr = self.pp_ptr.wrapping_add(2);
                }
            }
            _ => {}
        }
    }
}

// ── W5100 (Uthernet II) ─────────────────────────────────────────────────────

/// WIZnet W5100 register emulation.
struct W5100 {
    /// Register file (common + socket + Tx/Rx buffers = 32K).
    /// We only really need the first ~0x800 but allocate the full space
    /// for simplicity.
    regs: Vec<u8>,
    /// Current address for indirect mode (set via MR gateway regs).
    addr: u16,
    /// Set after address high byte is written, cleared after data r/w.
    #[allow(dead_code)]
    addr_phase: u8, // 0=expect addr high, 1=expect addr low, 2=expect data
}

impl W5100 {
    fn new() -> Self {
        let mut regs = vec![0u8; W5100_REG_SIZE];
        // Version register
        regs[W5100_VERSIONR as usize] = W5100_VERSION;
        Self {
            regs,
            addr: 0,
            addr_phase: 0,
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    /// Handle slot I/O read (Apple II uses 4 registers at offsets 0-3).
    fn io_read(&mut self, reg: u8) -> u8 {
        match reg & 0x03 {
            0 => {
                // Mode register
                self.regs[W5100_MR as usize]
            }
            1 => {
                // Address high byte
                (self.addr >> 8) as u8
            }
            2 => {
                // Address low byte
                self.addr as u8
            }
            3 => {
                // Data register — read from current address, auto-increment
                let addr = (self.addr as usize) & (W5100_REG_SIZE - 1);
                let val = self.regs[addr];
                self.addr = self.addr.wrapping_add(1);
                val
            }
            _ => 0xFF,
        }
    }

    /// Handle slot I/O write.
    fn io_write(&mut self, reg: u8, val: u8) {
        match reg & 0x03 {
            0 => {
                // Mode register
                if val & 0x80 != 0 {
                    // Software reset
                    self.reset();
                    return;
                }
                self.regs[W5100_MR as usize] = val;
            }
            1 => {
                // Address high byte
                self.addr = (self.addr & 0x00FF) | ((val as u16) << 8);
            }
            2 => {
                // Address low byte
                self.addr = (self.addr & 0xFF00) | val as u16;
            }
            3 => {
                // Data register — write to current address, auto-increment
                let addr = (self.addr as usize) & (W5100_REG_SIZE - 1);
                self.regs[addr] = val;
                self.addr = self.addr.wrapping_add(1);
            }
            _ => {}
        }
    }
}

// ── UthernCard ───────────────────────────────────────────────────────────────

enum UthernInner {
    Uthernet1(Cs8900a),
    Uthernet2(W5100),
}

pub struct UthernCard {
    slot:      usize,
    card_type: CardType,
    inner:     UthernInner,
}

impl UthernCard {
    pub fn new_uthernet1(slot: usize) -> Self {
        Self {
            slot,
            card_type: CardType::Uthernet,
            inner: UthernInner::Uthernet1(Cs8900a::new()),
        }
    }

    pub fn new_uthernet2(slot: usize) -> Self {
        Self {
            slot,
            card_type: CardType::Uthernet2,
            inner: UthernInner::Uthernet2(W5100::new()),
        }
    }
}

impl Card for UthernCard {
    fn card_type(&self) -> CardType { self.card_type }
    fn slot(&self) -> usize { self.slot }

    fn io_read(&mut self, _offset: u8, _cycles: u64) -> u8 { 0xFF }
    fn io_write(&mut self, _offset: u8, _value: u8, _cycles: u64) {}

    fn slot_io_read(&mut self, reg: u8, _cycles: u64) -> u8 {
        match &mut self.inner {
            UthernInner::Uthernet1(cs) => cs.io_read(reg),
            UthernInner::Uthernet2(w) => w.io_read(reg),
        }
    }

    fn slot_io_write(&mut self, reg: u8, val: u8, _cycles: u64) {
        match &mut self.inner {
            UthernInner::Uthernet1(cs) => cs.io_write(reg, val),
            UthernInner::Uthernet2(w) => w.io_write(reg, val),
        }
    }

    fn reset(&mut self, _power_cycle: bool) {
        match &mut self.inner {
            UthernInner::Uthernet1(cs) => cs.reset(),
            UthernInner::Uthernet2(w) => w.reset(),
        }
    }

    fn update(&mut self, _cycles: u64) {}

    fn save_state(&self, out: &mut dyn Write) -> Result<()> {
        out.write_all(&[1u8])?; // version
        Ok(())
    }

    fn load_state(&mut self, src: &mut dyn Read, _version: u32) -> Result<()> {
        let mut ver = [0u8; 1];
        src.read_exact(&mut ver)?;
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cs8900a_product_id() {
        let mut card = UthernCard::new_uthernet1(3);
        // Set PacketPage pointer to 0x0000 (Product ID register)
        card.slot_io_write(0x0A, 0x00, 0); // PP ptr low
        card.slot_io_write(0x0B, 0x00, 0); // PP ptr high
        // Read via PacketPage Data Port 0
        let lo = card.slot_io_read(0x0C, 0);
        let hi = card.slot_io_read(0x0D, 0);
        let product_id = lo as u16 | ((hi as u16) << 8);
        assert_eq!(product_id, CS8900A_PRODUCT_ID,
            "CS8900A Product ID should be 0x630E");
    }

    #[test]
    fn test_cs8900a_pp_write_read() {
        let mut card = UthernCard::new_uthernet1(3);
        // Write 0x1234 to PacketPage offset 0x0100
        card.slot_io_write(0x0A, 0x00, 0); // PP ptr low = 0x00
        card.slot_io_write(0x0B, 0x01, 0); // PP ptr high = 0x01 → offset 0x0100
        card.slot_io_write(0x0C, 0x34, 0); // data low
        card.slot_io_write(0x0D, 0x12, 0); // data high
        // Read it back
        card.slot_io_write(0x0A, 0x00, 0);
        card.slot_io_write(0x0B, 0x01, 0);
        let lo = card.slot_io_read(0x0C, 0);
        let hi = card.slot_io_read(0x0D, 0);
        assert_eq!(lo, 0x34);
        assert_eq!(hi, 0x12);
    }

    #[test]
    fn test_w5100_version() {
        let mut card = UthernCard::new_uthernet2(3);
        // Set address to W5100 version register (0x0019)
        card.slot_io_write(0x01, 0x00, 0); // addr high
        card.slot_io_write(0x02, 0x19, 0); // addr low
        let ver = card.slot_io_read(0x03, 0);
        assert_eq!(ver, W5100_VERSION, "W5100 version should be 0x04");
    }

    #[test]
    fn test_w5100_write_read() {
        let mut card = UthernCard::new_uthernet2(3);
        // Write 0xAB to address 0x0100
        card.slot_io_write(0x01, 0x01, 0); // addr high
        card.slot_io_write(0x02, 0x00, 0); // addr low
        card.slot_io_write(0x03, 0xAB, 0); // write data (auto-increments)
        // Read it back
        card.slot_io_write(0x01, 0x01, 0);
        card.slot_io_write(0x02, 0x00, 0);
        let val = card.slot_io_read(0x03, 0);
        assert_eq!(val, 0xAB);
    }

    #[test]
    fn test_w5100_software_reset() {
        let mut card = UthernCard::new_uthernet2(3);
        // Write something to a register
        card.slot_io_write(0x01, 0x01, 0);
        card.slot_io_write(0x02, 0x00, 0);
        card.slot_io_write(0x03, 0xFF, 0);
        // Software reset via MR bit 7
        card.slot_io_write(0x00, 0x80, 0);
        // Data should be cleared
        card.slot_io_write(0x01, 0x01, 0);
        card.slot_io_write(0x02, 0x00, 0);
        let val = card.slot_io_read(0x03, 0);
        assert_eq!(val, 0x00, "Software reset should clear registers");
    }

    #[test]
    fn test_reset_uthernet1() {
        let mut card = UthernCard::new_uthernet1(3);
        card.reset(true);
        // After reset, product ID should still be readable
        card.slot_io_write(0x0A, 0x00, 0);
        card.slot_io_write(0x0B, 0x00, 0);
        let lo = card.slot_io_read(0x0C, 0);
        assert_eq!(lo, 0x0E);
    }
}
