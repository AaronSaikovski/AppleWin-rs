//! Super Serial Card (SSC) — 6551 ACIA emulation.
//!
//! Implements the full 6551 ACIA register set:
//! - Tx bytes are buffered in `tx_buffer` (drainable via `take_tx_bytes`)
//! - Rx bytes can be injected via `push_rx_byte`
//! - RDRF flag set when rx has data, cleared on read
//! - Overrun detection when new byte arrives while RDRF is set
//! - IRQ on Rx data available when Command register bit 1 is clear (Rx IRQ enabled)
//! - IRQ on Tx when Command bits 2:1 = 0b01 and DTR set
//! - Card ROM at $Cn00 returns SSC identification bytes for ProDOS detection
//!
//! Reference: source/SerialComms.cpp

use std::collections::VecDeque;
use std::io::{Read, Write};
use crate::card::{Card, CardType};
use crate::error::Result;

// ── ROM ────────────────────────────────────────────────────────────────────────

fn make_ssc_rom() -> Box<[u8; 256]> {
    let mut rom = Box::new([0u8; 256]);
    // ProDOS / Pascal identification bytes
    rom[0x01] = 0x38;
    rom[0x03] = 0x18;
    rom[0x05] = 0x01;
    rom[0x07] = 0x31;
    // Pascal entry point markers
    rom[0xFB] = 0xD6;
    rom[0xFC] = 0xDC;
    rom[0xFD] = 0x09;
    rom[0xFE] = 0x00;
    rom[0xFF] = 0x00;
    rom
}

// ── Status register bit masks ──────────────────────────────────────────────────

const STATUS_IRQ:     u8 = 0x80; // bit 7 — interrupt occurred
#[allow(dead_code)]
const STATUS_DSR:     u8 = 0x40; // bit 6 — Data Set Ready
#[allow(dead_code)]
const STATUS_DCD:     u8 = 0x20; // bit 5 — Data Carrier Detect
const STATUS_TDRE:    u8 = 0x10; // bit 4 — Tx Data Register Empty
const STATUS_RDRF:    u8 = 0x08; // bit 3 — Rx Data Register Full
const STATUS_OVERRUN: u8 = 0x04; // bit 2 — Overrun error

// ── Struct ─────────────────────────────────────────────────────────────────────

pub struct SscCard {
    slot: usize,

    // 6551 ACIA registers
    rx_data: u8,   // RS0 read  — last received byte
    tx_data: u8,   // RS0 write — last transmitted byte
    status:  u8,   // RS1 read  — status register
    command: u8,   // RS2 r/w   — command register
    control: u8,   // RS3 r/w   — control register

    // Rx FIFO — bytes waiting to be consumed by the Apple II
    rx_buf: VecDeque<u8>,

    // Tx buffer — bytes written by the Apple II, waiting for host to drain
    tx_buffer: VecDeque<u8>,

    // Card $Cnxx ROM page
    rom: Box<[u8; 256]>,

    // IRQ latch
    irq: bool,
}

impl SscCard {
    pub fn new(slot: usize) -> Self {
        Self {
            slot,
            rx_data: 0,
            tx_data: 0,
            status:  STATUS_TDRE, // TDRE always set initially; DSR/DCD inactive
            command: 0x02,        // DTR=0, Tx IRQ disabled
            control: 0x00,
            rx_buf:  VecDeque::new(),
            tx_buffer: VecDeque::new(),
            rom:     make_ssc_rom(),
            irq:     false,
        }
    }

    /// Inject a byte into the Rx FIFO (host → emulator data path).
    /// If RDRF is already set (previous byte not yet read), sets overrun flag.
    pub fn push_rx_byte(&mut self, byte: u8) {
        if self.status & STATUS_RDRF != 0 {
            // Overrun: previous byte was not read before new one arrived
            self.status |= STATUS_OVERRUN;
        }
        self.rx_buf.push_back(byte);
        // Load the first pending byte into the data register
        if let Some(b) = self.rx_buf.pop_front() {
            self.rx_data = b;
            self.status |= STATUS_RDRF;
        }
        // Fire IRQ if Rx IRQ is enabled: Command bit 1 = 0 means Rx IRQ enabled
        // (with DTR set, Command bit 0 = 1)
        if self.command & 0x02 == 0 && self.command & 0x01 != 0 {
            self.status |= STATUS_IRQ;
            self.irq = true;
        }
    }

    /// Drain the transmit buffer — returns all bytes written by the Apple II
    /// since the last call. The GUI/host layer can forward these to a serial
    /// port, TCP socket, or file.
    pub fn take_tx_bytes(&mut self) -> Vec<u8> {
        self.tx_buffer.drain(..).collect()
    }
}

// ── Card trait ─────────────────────────────────────────────────────────────────

impl Card for SscCard {
    fn card_type(&self) -> CardType { CardType::Ssc }
    fn slot(&self) -> usize { self.slot }

    // ── $Cnxx ROM reads ──────────────────────────────────────────────────────

    fn io_read(&mut self, offset: u8, _cycles: u64) -> u8 {
        self.rom[offset as usize]
    }

    fn io_write(&mut self, _offset: u8, _value: u8, _cycles: u64) {
        // ROM page is not writable.
    }

    fn cx_rom(&self) -> Option<&[u8; 256]> {
        Some(&self.rom)
    }

    // ── $C0x8–$C0xB peripheral I/O ────────────────────────────────────────────
    //
    // The 6551 uses two address lines (RS0, RS1 = A0, A1) to select the register.
    // The SSC maps these to $C0x8–$C0xB (offset 0x08–0x0B relative to $C0x0).

    fn slot_io_read(&mut self, reg: u8, _cycles: u64) -> u8 {
        match reg & 0x0F {
            0x08 => {
                // RS0 — Rx Data Register: reading clears RDRF and overrun
                self.status &= !(STATUS_RDRF | STATUS_OVERRUN);
                self.irq = false;
                self.status &= !STATUS_IRQ;
                // Load next byte from FIFO if available
                if let Some(b) = self.rx_buf.pop_front() {
                    self.rx_data = b;
                    self.status |= STATUS_RDRF;
                    // Re-assert IRQ if Rx IRQ enabled
                    if self.command & 0x02 == 0 && self.command & 0x01 != 0 {
                        self.status |= STATUS_IRQ;
                        self.irq = true;
                    }
                }
                self.rx_data
            }
            0x09 => {
                // RS1 — Status Register (read clears IRQ bit)
                let s = self.status | STATUS_TDRE;
                // Reading status clears the IRQ bit per 6551 spec
                self.status &= !STATUS_IRQ;
                self.irq = false;
                s
            }
            0x0A => self.command,
            0x0B => self.control,
            _ => 0xFF,
        }
    }

    fn slot_io_write(&mut self, reg: u8, val: u8, _cycles: u64) {
        match reg & 0x0F {
            0x08 => {
                // RS0 — Tx Data Register: buffer the byte for host retrieval.
                self.tx_data = val;
                self.tx_buffer.push_back(val);
                self.status |= STATUS_TDRE;
                // If Tx IRQ is enabled (Command bits 2:1 = 0b01) fire IRQ.
                if self.command & 0x06 == 0x02 {
                    self.status |= STATUS_IRQ;
                    self.irq = true;
                }
            }
            0x09 => {
                // RS1 write — Programmed Reset: clears overrun, IRQ, resets status.
                self.status = STATUS_TDRE;
                self.irq = false;
            }
            0x0A => {
                // RS2 — Command Register
                self.command = val;
                // DTR bit 0 cleared → disable receiver (drain Rx FIFO).
                if val & 0x01 == 0 {
                    self.rx_buf.clear();
                    self.status &= !(STATUS_RDRF | STATUS_OVERRUN);
                }
            }
            0x0B => {
                // RS3 — Control Register
                self.control = val;
            }
            _ => {}
        }
    }

    // ── IRQ ──────────────────────────────────────────────────────────────────

    fn irq_active(&self) -> bool {
        // IRQ is only driven when DTR is set (Command bit 0 = 1).
        self.irq && (self.command & 0x01 != 0)
    }

    // ── Reset ────────────────────────────────────────────────────────────────

    fn reset(&mut self, _power_cycle: bool) {
        self.rx_data = 0;
        self.tx_data = 0;
        self.status  = STATUS_TDRE;
        self.command = 0x02;
        self.control = 0x00;
        self.rx_buf.clear();
        self.tx_buffer.clear();
        self.irq = false;
    }

    fn update(&mut self, _cycles: u64) {
        // Future: pace Rx byte delivery based on baud rate in `control`.
    }

    // ── Save / Load state ─────────────────────────────────────────────────────

    fn save_state(&self, out: &mut dyn Write) -> Result<()> {
        // Version byte, then registers and IRQ flag.
        out.write_all(&[
            1u8,            // version
            self.rx_data,
            self.tx_data,
            self.status,
            self.command,
            self.control,
            u8::from(self.irq),
        ])?;
        Ok(())
    }

    fn load_state(&mut self, src: &mut dyn Read, _version: u32) -> Result<()> {
        let mut buf = [0u8; 7];
        src.read_exact(&mut buf)?;
        // buf[0] is the inner version byte written by save_state.
        self.rx_data = buf[1];
        self.tx_data = buf[2];
        self.status  = buf[3];
        self.command = buf[4];
        self.control = buf[5];
        self.irq     = buf[6] != 0;
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tx_buffer_stores_bytes() {
        let mut card = SscCard::new(2);
        card.slot_io_write(0x08, 0x41, 0); // write 'A'
        card.slot_io_write(0x08, 0x42, 0); // write 'B'
        card.slot_io_write(0x08, 0x43, 0); // write 'C'
        let bytes = card.take_tx_bytes();
        assert_eq!(bytes, vec![0x41, 0x42, 0x43]);
        // Second drain should be empty
        assert!(card.take_tx_bytes().is_empty());
    }

    #[test]
    fn test_rx_rdrf_set_and_cleared() {
        let mut card = SscCard::new(2);
        // Enable DTR so IRQs can work
        card.command = 0x01; // DTR=1, Rx IRQ disabled (bit1=0 actually enables!)
        card.push_rx_byte(0x55);
        // Status should have RDRF set
        let status = card.slot_io_read(0x09, 0);
        assert_ne!(status & STATUS_RDRF, 0, "RDRF should be set after push_rx_byte");
        // Read data register — should return 0x55 and clear RDRF
        let data = card.slot_io_read(0x08, 0);
        assert_eq!(data, 0x55);
        // Now status should have RDRF cleared (no more bytes)
        let status2 = card.slot_io_read(0x09, 0);
        assert_eq!(status2 & STATUS_RDRF, 0, "RDRF should be cleared after reading data");
    }

    #[test]
    fn test_overrun_detection() {
        let mut card = SscCard::new(2);
        card.command = 0x03; // DTR=1, Rx IRQ disabled
        card.push_rx_byte(0xAA);
        // RDRF is now set; push another byte without reading
        card.push_rx_byte(0xBB);
        // Overrun flag should be set
        assert_ne!(card.status & STATUS_OVERRUN, 0, "Overrun should be set");
    }

    #[test]
    fn test_rx_irq_when_enabled() {
        let mut card = SscCard::new(2);
        // Command: DTR=1, Rx IRQ enabled (bit1=0)
        card.command = 0x01;
        card.push_rx_byte(0x42);
        assert!(card.irq_active(), "IRQ should fire when Rx IRQ enabled and data arrives");
    }

    #[test]
    fn test_rx_irq_not_fired_when_disabled() {
        let mut card = SscCard::new(2);
        // Command: DTR=1, Rx IRQ disabled (bit1=1)
        card.command = 0x03;
        card.push_rx_byte(0x42);
        assert!(!card.irq_active(), "IRQ should not fire when Rx IRQ disabled");
    }

    #[test]
    fn test_programmed_reset() {
        let mut card = SscCard::new(2);
        card.push_rx_byte(0xAA);
        card.push_rx_byte(0xBB); // overrun
        card.slot_io_write(0x09, 0x00, 0); // programmed reset
        assert_eq!(card.status, STATUS_TDRE, "Status should be reset to TDRE only");
        assert!(!card.irq, "IRQ should be cleared");
    }

    #[test]
    fn test_rom_identification() {
        let card = SscCard::new(2);
        let rom = card.cx_rom().expect("SSC should have ROM");
        assert_eq!(rom[0x01], 0x38);
        assert_eq!(rom[0x03], 0x18);
        assert_eq!(rom[0x05], 0x01);
        assert_eq!(rom[0x07], 0x31);
    }
}
