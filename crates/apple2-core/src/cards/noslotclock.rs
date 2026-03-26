//! No-Slot Clock (Dallas DS1216) emulation.
//! Sits in the ROM socket and provides real-time clock data via a 64-bit
//! serial shift-register protocol.
//! Reference: source/NoSlotClock.cpp

use std::io::{Read, Write};
use crate::card::{Card, CardType};
use crate::error::Result;

const MAGIC: u64 = 0x5CA33AC55CA33AC5;

pub struct NoSlotClockCard {
    slot:               usize,
    comparison_reg:     u64,    // shift register being loaded with input bits
    comparison_count:   u8,     // how many bits loaded so far (0-64)
    clock_reg:          u64,    // loaded once magic matches
    clock_count:        u8,     // how many bits read so far (0-64)
    clock_enabled:      bool,   // true after magic sequence matched
}

impl NoSlotClockCard {
    pub fn new(slot: usize) -> Self {
        Self {
            slot,
            comparison_reg:   0,
            comparison_count: 0,
            clock_reg:        0,
            clock_count:      0,
            clock_enabled:    false,
        }
    }

    fn load_clock_reg(&mut self) {
        use std::time::SystemTime;
        // Get current local time as best we can from SystemTime (UTC, good enough)
        let secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Very rough UTC time decomposition (no DST adjustment)
        let s = secs % 60;
        let m = (secs / 60) % 60;
        let h = (secs / 3600) % 24;
        let days_since_epoch = secs / 86400;
        // Day of week: Jan 1 1970 was Thursday (4). Sunday = 1 in DS1216.
        let dow = ((days_since_epoch + 3) % 7) + 1; // 1=Sunday
        // Rough date calculation
        let year_offset = days_since_epoch / 365; // approximate
        let year = (1970 + year_offset) % 100;
        let day_of_year = days_since_epoch % 365;
        let month = (day_of_year / 30).clamp(0, 11) + 1;
        let date = (day_of_year % 30) + 1;

        fn bcd(n: u64) -> u64 { ((n / 10) << 4) | (n % 10) }

        // Pack 64-bit clock register (8 bytes, each BCD-encoded, LSB first per field)
        self.clock_reg =
              bcd(0)           // centiseconds (always 0)
            | (bcd(s)    << 8)
            | (bcd(m)    << 16)
            | (bcd(h)    << 24)
            | (bcd(dow)  << 32)
            | (bcd(date) << 40)
            | (bcd(month)<< 48)
            | (bcd(year) << 56);
        self.clock_count = 0;
    }
}

impl Card for NoSlotClockCard {
    fn card_type(&self) -> CardType { CardType::GenericClock }
    fn slot(&self) -> usize { self.slot }

    /// NSC intercepts reads from $Cn ROM space. The address low bit provides serial data.
    fn io_read(&mut self, offset: u8, _cycles: u64) -> u8 {
        if self.clock_enabled && offset & 0x04 != 0 {
            // Clock read phase: return bit from clock register
            let bit = ((self.clock_reg >> self.clock_count) & 1) as u8;
            self.clock_count += 1;
            if self.clock_count >= 64 {
                self.clock_enabled = false;
                self.clock_count = 0;
                self.comparison_reg = 0;
                self.comparison_count = 0;
            }
            bit // bit on data line
        } else if offset & 0x04 == 0 {
            // Comparison write phase: clock in a bit from address bit 0
            let bit = (offset & 0x01) as u64;
            self.comparison_reg |= bit << self.comparison_count;
            self.comparison_count += 1;
            if self.comparison_count >= 64 {
                if self.comparison_reg == MAGIC {
                    self.load_clock_reg();
                    self.clock_enabled = true;
                }
                self.comparison_reg = 0;
                self.comparison_count = 0;
            }
            0xFF
        } else {
            0xFF
        }
    }

    fn io_write(&mut self, _offset: u8, _value: u8, _cycles: u64) {}
    fn slot_io_read(&mut self, _reg: u8, _cycles: u64) -> u8 { 0xFF }
    fn slot_io_write(&mut self, _reg: u8, _value: u8, _cycles: u64) {}

    fn reset(&mut self, _power_cycle: bool) {
        self.comparison_reg   = 0;
        self.comparison_count = 0;
        self.clock_reg        = 0;
        self.clock_count      = 0;
        self.clock_enabled    = false;
    }

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
