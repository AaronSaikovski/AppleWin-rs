//! Super Serial Card (SSC) — 6551 ACIA emulation.
//!
//! Implements the full 6551 ACIA register set in loopback/stub mode:
//! - TDRE is always 1 (Tx always ready; written bytes are dropped)
//! - Rx bytes can be injected via `push_rx_byte` for future extension
//! - Fires IRQ when Tx interrupt is enabled (Command bits 2:1 = 0b01) and DTR set
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

const STATUS_IRQ:  u8 = 0x80; // bit 7 — interrupt occurred
#[allow(dead_code)]
const STATUS_DSR:  u8 = 0x40; // bit 6 — Data Set Ready (active-low input; stub: not driven)
#[allow(dead_code)]
const STATUS_DCD:  u8 = 0x20; // bit 5 — Data Carrier Detect (active-low input; stub: not driven)
const STATUS_TDRE: u8 = 0x10; // bit 4 — Tx Data Register Empty (ready to send)
const STATUS_RDRF: u8 = 0x08; // bit 3 — Rx Data Register Full (data available)

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
            status:  STATUS_TDRE, // TDRE always set; DSR/DCD inactive (lines high = 0)
            command: 0x02,        // DTR=0, Tx IRQ disabled
            control: 0x00,
            rx_buf:  VecDeque::new(),
            rom:     make_ssc_rom(),
            irq:     false,
        }
    }

    /// Inject a byte into the Rx FIFO (for future host→emulator data path).
    #[allow(dead_code)]
    pub fn push_rx_byte(&mut self, byte: u8) {
        self.rx_buf.push_back(byte);
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
                // RS0 — Rx Data Register: reading clears RDRF
                self.status &= !STATUS_RDRF;
                self.rx_data
            }
            0x09 => {
                // RS1 — Status Register
                // TDRE is always 1 in stub mode; update RDRF from buffer.
                let mut s = self.status | STATUS_TDRE;
                if let Some(&byte) = self.rx_buf.front() {
                    self.rx_data = byte;
                    s |= STATUS_RDRF;
                } else {
                    s &= !STATUS_RDRF;
                }
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
                // RS0 — Tx Data Register: drop the byte (stub), keep TDRE set.
                self.tx_data = val;
                self.status |= STATUS_TDRE;
                // If Tx IRQ is enabled (Command bits 2:1 = 0b01) fire IRQ.
                if self.command & 0x06 == 0x02 {
                    self.status |= STATUS_IRQ;
                    self.irq = true;
                }
            }
            0x09 => {
                // RS1 write — Programmed Reset: clears all status except TDRE.
                self.status = STATUS_TDRE;
                self.irq = false;
            }
            0x0A => {
                // RS2 — Command Register
                self.command = val;
                // DTR bit 0 cleared → disable receiver (drain Rx FIFO).
                if val & 0x01 == 0 {
                    self.rx_buf.clear();
                    self.status &= !STATUS_RDRF;
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
        self.irq = false;
    }

    fn update(&mut self, _cycles: u64) {
        // Consume one byte from the Rx FIFO per update tick if RDRF was read.
        // Actual pacing can be added later based on the baud rate in `control`.
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
