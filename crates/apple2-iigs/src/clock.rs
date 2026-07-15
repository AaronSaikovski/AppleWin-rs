//! Apple IIgs clock GLU — real-time clock + battery-RAM (BRAM) access.
//!
//! The clock chip is accessed through two soft switches:
//!   `$C033` CLOCKDATA — the data byte transferred to/from the chip.
//!   `$C034` CLOCKCTL  — bit 7 starts a transaction, bit 6 = read/write, the low
//!                        nibble also holds the video border colour.
//!
//! A transaction is a small state machine: the first command byte (in
//! `$C033`) selects the operation (read/write the seconds counter, the internal
//! registers, or a battery-RAM location), and follow-up transactions carry the
//! data. GS/OS reads its configuration (and the date/time) from here during
//! startup, so it must return consistent values rather than a floating bus.
//!
//! Protocol ported from KEGS/GSplus (`clock.c`).

use serde::{Deserialize, Serialize};

/// Clock GLU transaction state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
enum ClkMode {
    #[default]
    Idle,
    Time,
    Internal,
    Bram1,
    Bram2,
}

/// Clock GLU state (RTC + BRAM access engine).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clock {
    /// `$C033` data register.
    data: u8,
    /// `$C034` control register (low 7 bits; bit 7 is the write-triggered start).
    ctl: u8,
    /// Transaction state.
    mode: ClkMode,
    /// Selected register / BRAM address for the current transaction.
    reg1: u8,
    /// Read (true) vs write (false) for the current transaction.
    read_flag: bool,
    /// Real-time clock: seconds since 1904-01-01 (Apple epoch).
    pub cur_time: u32,
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            data: 0,
            ctl: 0,
            mode: ClkMode::Idle,
            reg1: 0,
            read_flag: false,
            // 2020-01-01 00:00:00, seconds since 1904-01-01 (Apple epoch).
            cur_time: 3_660_681_600,
        }
    }
}

impl Clock {
    /// Read `$C033` (clock data register).
    pub fn read_data(&self) -> u8 {
        self.data
    }

    /// Read `$C034` (clock control). The low nibble is the border colour.
    pub fn read_ctl(&self) -> u8 {
        self.ctl
    }

    /// Write `$C033` (clock data register).
    pub fn write_data(&mut self, val: u8) {
        self.data = val;
    }

    /// Write `$C034` (clock control). Bit 7 starts a transaction against `bram`.
    pub fn write_ctl(&mut self, val: u8, bram: &mut [u8; 256]) {
        self.ctl = val & 0x7F;
        if val & 0x80 != 0 {
            self.transaction(bram);
        }
    }

    /// Run one step of the clock transaction state machine.
    fn transaction(&mut self, bram: &mut [u8; 256]) {
        let read = self.ctl & 0x40 != 0;
        match self.mode {
            ClkMode::Idle => {
                self.read_flag = (self.data >> 7) & 1 != 0;
                self.reg1 = (self.data >> 2) & 3;
                let op = (self.data >> 4) & 7;
                if read {
                    // Read while idle — no-op, stay idle.
                    self.mode = ClkMode::Idle;
                    return;
                }
                match op {
                    0x0 => self.mode = ClkMode::Time, // seconds counter
                    0x3 => {
                        if self.reg1 & 0x2 != 0 {
                            // Extended BRAM: high address bits from this byte.
                            self.mode = ClkMode::Bram2;
                            self.reg1 = (self.data & 7) << 5;
                        } else {
                            self.mode = ClkMode::Internal;
                        }
                    }
                    0x2 => {
                        self.mode = ClkMode::Bram1;
                        self.reg1 = self.reg1.wrapping_add(0x10); // BRAM $10-$13
                    }
                    0x4..=0x7 => {
                        self.mode = ClkMode::Bram1;
                        self.reg1 = (self.data >> 2) & 0x0F; // BRAM $00-$0F
                    }
                    _ => self.mode = ClkMode::Idle,
                }
            }
            ClkMode::Bram2 => {
                // Second byte of an extended BRAM address (low 5 bits).
                if !read && (self.data & 0x83) == 0x00 {
                    self.reg1 |= (self.data >> 2) & 0x1F;
                    self.mode = ClkMode::Bram1;
                } else {
                    self.mode = ClkMode::Idle;
                }
            }
            ClkMode::Bram1 => {
                let addr = self.reg1 as usize;
                if read && self.read_flag {
                    self.data = bram[addr];
                } else if !read && !self.read_flag {
                    bram[addr] = self.data;
                }
                self.mode = ClkMode::Idle;
            }
            ClkMode::Time => {
                let shift = (self.reg1 & 3) * 8;
                if read {
                    self.data = ((self.cur_time >> shift) & 0xFF) as u8;
                } else {
                    let mask = 0xFFu32 << shift;
                    self.cur_time = (self.cur_time & !mask) | ((self.data as u32) << shift);
                }
                self.mode = ClkMode::Idle;
            }
            ClkMode::Internal => {
                // Internal test / write-protect registers — accept and ignore.
                self.mode = ClkMode::Idle;
            }
        }
    }
}
