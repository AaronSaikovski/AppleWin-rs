//! Apple IIgs memory bus.
//!
//! Implements the `Bus816` trait for the 65C816 CPU, providing the full
//! IIgs memory map with bank-switched RAM, ROM, I/O shadowing, and
//! Mega II compatibility.

use crate::adb::Adb;
use crate::bram;
use crate::cpu65816::Bus816;
use crate::ensoniq::Ensoniq;
use crate::fpi::Fpi;
use crate::mega2::Mega2;
use crate::memory::IIgsMemory;
use crate::smartport::SmartPort;

/// The complete IIgs bus state.
pub struct IIgsBus {
    /// Memory subsystem (RAM + ROM).
    pub mem: IIgsMemory,

    /// Mega II compatibility layer (soft-switches, I/O).
    pub mega2: Mega2,

    /// FPI speed control.
    pub fpi: Fpi,

    /// ADB micro-controller (keyboard, mouse, BRAM, RTC).
    pub adb: Adb,

    /// Ensoniq DOC 5503 wavetable synthesizer.
    pub ensoniq: Ensoniq,

    /// SmartPort disk controller (3.5" and hard disk).
    pub smartport: SmartPort,

    /// Battery-backed parameter RAM (256 bytes).
    pub bram: [u8; 256],

    /// IRQ line state — true when any interrupt source is active.
    pub irq_line: bool,

    /// VBL interrupt enable and state.
    pub vbl_irq_enabled: bool,
    /// One-second interrupt enable.
    pub one_sec_irq_enabled: bool,
    /// Quarter-second interrupt counter.
    pub qsec_counter: u64,

    /// Slot ROM area: internal ROM for slots when INTCXROM is set.
    slot_rom_cache: Vec<u8>,
}

impl IIgsBus {
    /// Create a new IIgs bus.
    pub fn new(mem: IIgsMemory) -> Self {
        // Cache the slot ROM area from the ROM image.
        // In ROM 01/03, the slot firmware is in the last bank at $Cn00-$CFFF.
        let slot_rom_cache = {
            let rom_bank = 0xFF_u8;
            let mut cache = vec![0u8; 0x1000]; // $C100-$CFFF
            for (i, byte) in cache.iter_mut().enumerate() {
                *byte = mem.rom_read(rom_bank, 0xC100 + i as u16);
            }
            cache
        };

        Self {
            mem,
            mega2: Mega2::default(),
            fpi: Fpi::default(),
            adb: Adb::default(),
            ensoniq: Ensoniq::default(),
            smartport: SmartPort::default(),
            bram: bram::factory_default_bram(),
            irq_line: false,
            vbl_irq_enabled: false,
            one_sec_irq_enabled: false,
            qsec_counter: 0,
            slot_rom_cache,
        }
    }

    /// Dispatch a read based on bank and offset.
    fn bank_read(&mut self, bank: u8, offset: u16, cycles: u64) -> u8 {
        match bank {
            // Banks $00-$01: Slow RAM with I/O aperture
            0x00 | 0x01 => self.read_slow_bank(bank, offset, cycles),

            // Banks $02-$7F: Expansion RAM (direct access)
            0x02..=0x7F => self.mem.ram_read(bank, offset),

            // Banks $80-$DF: Mirror of $00-$5F (ROM 03 behavior)
            0x80..=0xDF => {
                let mirrored_bank = bank - 0x80;
                if mirrored_bank <= 0x01 {
                    self.read_slow_bank(mirrored_bank, offset, cycles)
                } else {
                    self.mem.ram_read(mirrored_bank, offset)
                }
            }

            // Banks $E0-$E1: Fast RAM (no I/O)
            0xE0 | 0xE1 => {
                let bank_offset = bank - 0xE0;
                self.mem.fast_ram_read(bank_offset, offset)
            }

            // Banks $E2-$FB: unused / mirrors
            0xE2..=0xFB => 0x00,

            // Banks $FC-$FF: ROM
            0xFC..=0xFF => self.read_rom_bank(bank, offset, cycles),
        }
    }

    /// Dispatch a write based on bank and offset.
    fn bank_write(&mut self, bank: u8, offset: u16, val: u8, cycles: u64) {
        match bank {
            // Banks $00-$01: Slow RAM with I/O + shadowing
            0x00 | 0x01 => self.write_slow_bank(bank, offset, val, cycles),

            // Banks $02-$7F: Expansion RAM
            0x02..=0x7F => self.mem.ram_write(bank, offset, val),

            // Banks $80-$DF: Mirror of $00-$5F
            0x80..=0xDF => {
                let mirrored_bank = bank - 0x80;
                if mirrored_bank <= 0x01 {
                    self.write_slow_bank(mirrored_bank, offset, val, cycles);
                } else {
                    self.mem.ram_write(mirrored_bank, offset, val);
                }
            }

            // Banks $E0-$E1: Fast RAM (no I/O, no shadowing)
            0xE0 | 0xE1 => {
                let bank_offset = bank - 0xE0;
                self.mem.fast_ram_write(bank_offset, offset, val);
            }

            // Banks $E2-$FB: unused
            0xE2..=0xFB => {}

            // Banks $FC-$FF: ROM (read-only, writes ignored)
            0xFC..=0xFF => {}
        }
    }

    /// Read from a "slow" bank ($00 or $01) with I/O aperture handling.
    fn read_slow_bank(&mut self, bank: u8, offset: u16, cycles: u64) -> u8 {
        // I/O aperture: $C000-$C0FF
        if (0xC000..=0xC0FF).contains(&offset) {
            self.fpi.io_access();
            let io_offset = (offset & 0xFF) as u8;

            // ADB and Ensoniq registers are handled by the bus directly
            let val = match io_offset {
                0x24 => self.adb.mouse_data,
                0x25 => self.adb.modifiers,
                0x26 => self.adb.read_data(),
                0x27 => {
                    self.adb.update(cycles);
                    self.adb.read_status()
                }
                // Ensoniq DOC registers
                0x3C => self.ensoniq.read_control(),
                0x3D => self.ensoniq.read_data(),
                0x3E => self.ensoniq.read_addr_lo(),
                0x3F => self.ensoniq.read_addr_hi(),
                _ => self.mega2.io_read(io_offset, cycles),
            };

            self.fpi.io_complete();
            return val;
        }

        // Slot ROM area: $C100-$CFFF
        if (0xC100..=0xCFFF).contains(&offset) {
            // Return internal slot ROM from the ROM image
            let idx = (offset - 0xC100) as usize;
            return self.slot_rom_cache.get(idx).copied().unwrap_or(0);
        }

        // Language Card area: $D000-$FFFF
        if offset >= 0xD000 {
            return self.read_language_card(bank, offset);
        }

        // Regular RAM
        self.mem.ram_read(bank, offset)
    }

    /// Write to a "slow" bank ($00 or $01) with I/O + shadowing.
    fn write_slow_bank(&mut self, bank: u8, offset: u16, val: u8, cycles: u64) {
        // I/O aperture: $C000-$C0FF
        if (0xC000..=0xC0FF).contains(&offset) {
            self.fpi.io_access();
            let io_offset = (offset & 0xFF) as u8;

            match io_offset {
                0x26 => {
                    // ADB command write — handle BRAM commands specially
                    self.adb.write_command(val, cycles);
                    self.handle_bram_command();
                }
                0x27 => {
                    // Writing to $C027 clears interrupt flags
                    self.adb.status &= !val;
                }
                // Speed register — update FPI after Mega2 stores the value
                0x36 => {
                    self.mega2.io_write(io_offset, val, cycles);
                    self.fpi.set_speed_from_reg(self.mega2.speed_reg);
                }
                // Ensoniq DOC registers
                0x3C => self.ensoniq.write_control(val),
                0x3D => self.ensoniq.write_data(val),
                0x3E => self.ensoniq.write_addr_lo(val),
                0x3F => self.ensoniq.write_addr_hi(val),
                _ => self.mega2.io_write(io_offset, val, cycles),
            }

            self.fpi.io_complete();
            return;
        }

        // Slot ROM area: $C100-$CFFF — writes ignored (ROM)
        if (0xC100..=0xCFFF).contains(&offset) {
            return;
        }

        // Language Card area: $D000-$FFFF
        if offset >= 0xD000 {
            self.write_language_card(bank, offset, val);
            // Shadow LC writes if enabled
            if bank == 0 && self.mega2.shadow.should_shadow_bank0(offset) {
                self.mem.fast_ram_write(0, offset, val);
            } else if bank == 1 && self.mega2.shadow.should_shadow_bank1(offset) {
                self.mem.fast_ram_write(1, offset, val);
            }
            return;
        }

        // Regular RAM write
        self.mem.ram_write(bank, offset, val);

        // Apply shadowing: mirror to fast RAM ($E0/$E1)
        if bank == 0 && self.mega2.shadow.should_shadow_bank0(offset) {
            self.mem.fast_ram_write(0, offset, val);
        } else if bank == 1 && self.mega2.shadow.should_shadow_bank1(offset) {
            self.mem.fast_ram_write(1, offset, val);
        }
    }

    /// Read from the Language Card area ($D000-$FFFF).
    fn read_language_card(&self, bank: u8, offset: u16) -> u8 {
        use apple2_core::bus::MemMode;

        if self.mega2.mem_mode.contains(MemMode::MF_HIGHRAM) {
            // RAM is active in the LC area
            if offset < 0xE000 && !self.mega2.mem_mode.contains(MemMode::MF_BANK2) {
                // Bank 1: $D000-$DFFF maps to a separate 4KB region
                // For simplicity, we store bank 1 data 4KB below bank 2
                // Bank 2 is at the normal offset, bank 1 is offset by -0x1000
                let adjusted = offset.wrapping_sub(0x1000);
                self.mem.ram_read(bank, adjusted)
            } else {
                self.mem.ram_read(bank, offset)
            }
        } else {
            // ROM is visible — read from the ROM image
            // The IIgs maps the last bank of ROM here
            self.mem.rom_read(0xFF, offset)
        }
    }

    /// Write to the Language Card area ($D000-$FFFF).
    fn write_language_card(&mut self, bank: u8, offset: u16, val: u8) {
        use apple2_core::bus::MemMode;

        if self.mega2.mem_mode.contains(MemMode::MF_WRITERAM) {
            if offset < 0xE000 && !self.mega2.mem_mode.contains(MemMode::MF_BANK2) {
                let adjusted = offset.wrapping_sub(0x1000);
                self.mem.ram_write(bank, adjusted, val);
            } else {
                self.mem.ram_write(bank, offset, val);
            }
        }
        // Write-protect: ignore write
    }

    /// Read from a ROM bank ($FC-$FF).
    /// In ROM banks, the lower portion ($0000-$BFFF) may also map to RAM
    /// on some configurations. For simplicity, we return ROM for the entire bank.
    /// The I/O aperture at $C000-$C0FF in ROM banks mirrors bank $00 I/O.
    fn read_rom_bank(&mut self, bank: u8, offset: u16, cycles: u64) -> u8 {
        // I/O aperture in ROM banks — same as bank $00
        if (0xC000..=0xC0FF).contains(&offset) {
            self.fpi.io_access();
            let val = self.mega2.io_read((offset & 0xFF) as u8, cycles);
            self.fpi.io_complete();
            return val;
        }

        // Slot ROM in ROM banks
        if (0xC100..=0xCFFF).contains(&offset) {
            let idx = (offset - 0xC100) as usize;
            return self.slot_rom_cache.get(idx).copied().unwrap_or(0);
        }

        self.mem.rom_read(bank, offset)
    }

    /// Check if any SmartPort device has a disk inserted.
    pub fn smartport_has_disk(&self) -> bool {
        (0..4).any(|i| self.smartport.has_disk(i))
    }

    /// Handle BRAM read/write after an ADB command is processed.
    fn handle_bram_command(&mut self) {
        // Check if the last command was a BRAM read
        if let Some(addr) = self.adb.bram_read_addr() {
            let val = self.bram[addr as usize];
            self.adb.push_response(val);
        }
        // Check if the last command was a BRAM write
        if let Some((addr, data)) = self.adb.bram_write_params() {
            self.bram[addr as usize] = data;
        }
    }

    /// Update interrupt state. Called periodically from the emulator loop.
    pub fn update_interrupts(&mut self, cycles: u64) {
        // Update ADB controller
        self.adb.update(cycles);

        // Update VBL state
        self.mega2.update_vblank(cycles);

        // Check for VBL interrupt
        let mut irq = false;
        if self.vbl_irq_enabled && self.mega2.vblank {
            self.mega2.vgc_int |= 0x80; // VGC interrupt occurred
            irq = true;
        }

        // Check for ADB keyboard interrupt
        if self.adb.status & crate::adb::status::KEY_IRQ != 0 {
            irq = true;
        }

        self.irq_line = irq;
    }

    /// Reset the bus state (power cycle or warm reset).
    pub fn reset(&mut self, _power_cycle: bool) {
        self.mega2 = Mega2::default();
        self.fpi = Fpi::default();
        self.adb = Adb::default();
        self.ensoniq = Ensoniq::default();
        self.irq_line = false;
        self.vbl_irq_enabled = false;
        self.one_sec_irq_enabled = false;
        // Reinitialize BRAM with factory defaults if needed
        if !bram::validate_bram_checksum(&self.bram) {
            self.bram = bram::factory_default_bram();
        }
    }
}

impl Bus816 for IIgsBus {
    fn read(&mut self, addr: u32, cycles: u64) -> u8 {
        let bank = ((addr >> 16) & 0xFF) as u8;
        let offset = (addr & 0xFFFF) as u16;
        self.bank_read(bank, offset, cycles)
    }

    fn write(&mut self, addr: u32, val: u8, cycles: u64) {
        let bank = ((addr >> 16) & 0xFF) as u8;
        let offset = (addr & 0xFFFF) as u16;
        self.bank_write(bank, offset, val, cycles);
    }

    fn read_raw(&self, addr: u32) -> u8 {
        let bank = ((addr >> 16) & 0xFF) as u8;
        let offset = (addr & 0xFFFF) as u16;

        match bank {
            0x00 | 0x01 => {
                if offset >= 0xD000 {
                    return self.read_language_card(bank, offset);
                }
                self.mem.ram_read(bank, offset)
            }
            0x02..=0x7F => self.mem.ram_read(bank, offset),
            0x80..=0xDF => {
                let mirrored = bank - 0x80;
                self.mem.ram_read(mirrored, offset)
            }
            0xE0 | 0xE1 => self.mem.fast_ram_read(bank - 0xE0, offset),
            0xFC..=0xFF => self.mem.rom_read(bank, offset),
            _ => 0x00,
        }
    }
}
