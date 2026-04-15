//! Apple IIgs memory subsystem.
//!
//! Manages the IIgs memory map with bank-switched RAM and ROM:
//!
//! - Banks $00-$01: Conventional "slow" RAM (128KB) with I/O at $C000-$C0FF
//! - Banks $02-$7F: Expansion RAM (if present)
//! - Banks $80-$DF: Mirror of banks $00-$5F (ROM 03 behaviour)
//! - Banks $E0-$E1: Fast RAM (same content as $00-$01, no I/O aperture)
//! - Banks $FC-$FF: ROM (128KB or 256KB)
//!
//! The IIgs has a minimum of 256KB RAM and can be expanded up to 8MB.

use serde::{Deserialize, Serialize};

/// IIgs ROM version, detected from file size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IIgsRomVersion {
    /// ROM 00 (342-0077-A) - original IIgs, 128KB
    Rom00,
    /// ROM 01 (342-0077-B) - updated firmware, 128KB
    Rom01,
    /// ROM 03 (341-0728 + 341-0748/0749) - 256KB
    Rom03,
}

/// IIgs memory state.
pub struct IIgsMemory {
    /// System RAM. Minimum 256KB, up to 8MB.
    /// Indexed as: ram[bank * 0x10000 + offset]
    /// Banks 0-1 are "slow" RAM, banks 2+ are expansion.
    pub ram: Vec<u8>,

    /// Total RAM size in bytes.
    pub ram_size: usize,

    /// ROM image (128KB or 256KB), loaded from file.
    /// ROM 01: 128KB maps to banks $FE-$FF
    /// ROM 03: 256KB maps to banks $FC-$FF
    pub rom: Vec<u8>,

    /// Detected ROM version.
    pub rom_version: IIgsRomVersion,

    /// Fast RAM (banks $E0-$E1). Shadowed from banks $00-$01.
    /// 128KB, indexed as: fast_ram[bank_offset * 0x10000 + offset]
    /// where bank_offset = bank - 0xE0 (0 or 1).
    pub fast_ram: Vec<u8>,
}

impl IIgsMemory {
    /// Create a new IIgs memory subsystem.
    ///
    /// `ram_kb` is the total RAM in kilobytes (minimum 256, maximum 8192).
    /// `rom_data` is the ROM image loaded from file.
    pub fn new(ram_kb: usize, rom_data: Vec<u8>) -> Result<Self, String> {
        let ram_kb = ram_kb.clamp(256, 8192);
        let ram_size = ram_kb * 1024;

        let rom_version = match rom_data.len() {
            0x20000 => {
                // 128KB - could be ROM 00 or ROM 01. Check version byte.
                // ROM 00: byte at offset $FBBE = $00
                // ROM 01: byte at offset $FBBE = $01
                let version_byte = rom_data.get(0x1_FBBE).copied().unwrap_or(0);
                if version_byte == 0 {
                    IIgsRomVersion::Rom00
                } else {
                    IIgsRomVersion::Rom01
                }
            }
            0x40000 => IIgsRomVersion::Rom03,
            other => {
                return Err(format!(
                    "Invalid IIgs ROM size: {} bytes (expected 131072 for ROM 00/01 or 262144 for ROM 03)",
                    other
                ));
            }
        };

        let mut ram = vec![0u8; ram_size];
        // Initialize RAM with a pattern similar to real hardware
        for (i, byte) in ram.iter_mut().enumerate() {
            *byte = if (i >> 7) & 1 == 0 { 0x00 } else { 0xFF };
        }

        let fast_ram = vec![0u8; 0x20000]; // 128KB for banks $E0-$E1

        Ok(Self {
            ram,
            ram_size,
            rom: rom_data,
            rom_version,
            fast_ram,
        })
    }

    /// Total number of RAM banks available.
    pub fn ram_banks(&self) -> usize {
        self.ram_size / 0x10000
    }

    /// Read a byte from the flat RAM array given bank and offset.
    /// Returns 0 if the address is out of range.
    #[inline]
    pub fn ram_read(&self, bank: u8, offset: u16) -> u8 {
        let addr = (bank as usize) * 0x10000 + offset as usize;
        if addr < self.ram_size {
            self.ram[addr]
        } else {
            0x00
        }
    }

    /// Write a byte to the flat RAM array given bank and offset.
    /// No-op if the address is out of range.
    #[inline]
    pub fn ram_write(&mut self, bank: u8, offset: u16, val: u8) {
        let addr = (bank as usize) * 0x10000 + offset as usize;
        if addr < self.ram_size {
            self.ram[addr] = val;
        }
    }

    /// Read a byte from fast RAM (banks $E0-$E1).
    #[inline]
    pub fn fast_ram_read(&self, bank_offset: u8, offset: u16) -> u8 {
        let addr = (bank_offset as usize) * 0x10000 + offset as usize;
        if addr < self.fast_ram.len() {
            self.fast_ram[addr]
        } else {
            0x00
        }
    }

    /// Write a byte to fast RAM (banks $E0-$E1).
    #[inline]
    pub fn fast_ram_write(&mut self, bank_offset: u8, offset: u16, val: u8) {
        let addr = (bank_offset as usize) * 0x10000 + offset as usize;
        if addr < self.fast_ram.len() {
            self.fast_ram[addr] = val;
        }
    }

    /// Read a byte from ROM given a 24-bit address in the ROM bank range.
    ///
    /// ROM 01 (128KB): maps to banks $FE-$FF (offset within ROM = (bank - $FE) * 64K + addr)
    /// ROM 03 (256KB): maps to banks $FC-$FF (offset within ROM = (bank - $FC) * 64K + addr)
    #[inline]
    pub fn rom_read(&self, bank: u8, offset: u16) -> u8 {
        let rom_base_bank = match self.rom_version {
            IIgsRomVersion::Rom00 | IIgsRomVersion::Rom01 => 0xFE,
            IIgsRomVersion::Rom03 => 0xFC,
        };

        if bank < rom_base_bank {
            return 0x00;
        }

        let rom_offset = ((bank - rom_base_bank) as usize) * 0x10000 + offset as usize;
        self.rom.get(rom_offset).copied().unwrap_or(0x00)
    }

    /// Check if a bank number falls in the ROM range.
    #[inline]
    pub fn is_rom_bank(&self, bank: u8) -> bool {
        match self.rom_version {
            IIgsRomVersion::Rom00 | IIgsRomVersion::Rom01 => bank >= 0xFE,
            IIgsRomVersion::Rom03 => bank >= 0xFC,
        }
    }

    /// Clear all RAM (power cycle).
    pub fn clear_ram(&mut self) {
        self.ram.fill(0);
        self.fast_ram.fill(0);
    }
}
