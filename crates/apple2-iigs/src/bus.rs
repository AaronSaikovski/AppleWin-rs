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
            // Replace slot 5 firmware with our SmartPort stub
            install_smartport_stub(&mut cache);
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
                // Slot ROM area: $C100-$CFFF
                if (0xC100..=0xCFFF).contains(&offset) {
                    let idx = (offset - 0xC100) as usize;
                    return self.slot_rom_cache.get(idx).copied().unwrap_or(0);
                }
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

    fn wdm_trap(&mut self, signature: u8, sp: u16, pbr: u8, emulation: bool) -> Option<(u8, bool)> {
        if signature == SMARTPORT_TRAP_SIG {
            Some(self.smartport_trap(sp, pbr, emulation))
        } else {
            None
        }
    }
}

// ── SmartPort firmware stub ─────────────────────────────────────────────────

/// WDM signature byte used to mark a SmartPort firmware trap.
pub const SMARTPORT_TRAP_SIG: u8 = 0xFE;

/// Install a SmartPort firmware stub into the slot ROM cache at slot 5 ($C500-$C5FF).
fn install_smartport_stub(cache: &mut [u8]) {
    // Slot 5 is at offset $400-$4FF within the slot ROM cache ($C500 - $C100 = $400).
    let base = 0x400;
    if base + 0x100 > cache.len() {
        return;
    }
    let slot = &mut cache[base..base + 0x100];
    slot.fill(0x00);

    // Standard ProDOS/SmartPort identification pattern (signature bytes at odd offsets):
    //   LDX #$20; LDY #$00; LDX #$03; STX $3C
    slot[0x00] = 0xA2;
    slot[0x01] = 0x20;
    slot[0x02] = 0xA0;
    slot[0x03] = 0x00;
    slot[0x04] = 0xA2;
    slot[0x05] = 0x03;
    slot[0x06] = 0x86;
    slot[0x07] = 0x3C;

    // $C508: SmartPort entry — WDM $FE (trap) + RTS
    slot[0x08] = 0x42;
    slot[0x09] = SMARTPORT_TRAP_SIG;
    slot[0x0A] = 0x60;

    // $C50B: ProDOS 8 entry (same trap)
    slot[0x0B] = 0x42;
    slot[0x0C] = SMARTPORT_TRAP_SIG;
    slot[0x0D] = 0x60;

    // Pascal/SmartPort signature bytes at $CsFB-$CsFF
    slot[0xFB] = 0x20;
    slot[0xFC] = 0x00;
    slot[0xFD] = 0x00;
    slot[0xFE] = 0xBC; // SmartPort + extended status + read + write + format
    slot[0xFF] = 0x05; // Offset from $Cs00 to SmartPort entry
}

impl IIgsBus {
    /// Handle a SmartPort firmware trap (WDM $FE at $C508).
    ///
    /// Calling convention:
    ///   JSR $C508
    ///   .byte command
    ///   .word cmdlist_ptr
    ///
    /// Returns (accumulator = error code, carry_flag = error).
    /// Advances the pushed return address past the 3 inline parameter bytes.
    pub fn smartport_trap(&mut self, sp: u16, pbr: u8, emulation: bool) -> (u8, bool) {
        let stack_wrap = |s: u16, off: u16| -> u32 {
            if emulation {
                0x0100 | (s.wrapping_add(off) & 0xFF) as u32
            } else {
                s.wrapping_add(off) as u32
            }
        };

        let ret_lo_addr = stack_wrap(sp, 1);
        let ret_hi_addr = stack_wrap(sp, 2);
        let ret_lo = self.read_raw(ret_lo_addr);
        let ret_hi = self.read_raw(ret_hi_addr);
        let ret_minus_1 = ((ret_hi as u16) << 8) | ret_lo as u16;
        let inline_addr = ret_minus_1.wrapping_add(1);

        let pbr_base = (pbr as u32) << 16;
        let cmd = self.read_raw(pbr_base | inline_addr as u32);
        let cmdlist_lo = self.read_raw(pbr_base | (inline_addr.wrapping_add(1)) as u32);
        let cmdlist_hi = self.read_raw(pbr_base | (inline_addr.wrapping_add(2)) as u32);
        let cmdlist_ptr = ((cmdlist_hi as u16) << 8) | cmdlist_lo as u16;

        // Advance pushed return address past the 3 inline bytes
        let new_ret = ret_minus_1.wrapping_add(3);
        self.write(ret_lo_addr, new_ret as u8, 0);
        self.write(ret_hi_addr, (new_ret >> 8) as u8, 0);

        let error = self.dispatch_smartport_command(cmd, cmdlist_ptr, pbr);
        (error, error != 0)
    }

    /// Dispatch a SmartPort MLI command. Returns error code (0 = success).
    fn dispatch_smartport_command(&mut self, cmd: u8, cmdlist_ptr: u16, pbr: u8) -> u8 {
        let pbr_base = (pbr as u32) << 16;
        let read_cmd_byte = |bus: &Self, offset: u16| -> u8 {
            bus.read_raw(pbr_base | cmdlist_ptr.wrapping_add(offset) as u32)
        };

        match cmd {
            0x00 => {
                // STATUS
                let unit = read_cmd_byte(self, 1);
                let list_lo = read_cmd_byte(self, 2);
                let list_hi = read_cmd_byte(self, 3);
                let status_code = read_cmd_byte(self, 4);
                let list_ptr = ((list_hi as u16) << 8) | list_lo as u16;
                self.smartport_status(unit, status_code, list_ptr, pbr)
            }
            0x01 => {
                // READ BLOCK
                let unit = read_cmd_byte(self, 1);
                let buf_lo = read_cmd_byte(self, 2);
                let buf_hi = read_cmd_byte(self, 3);
                let blk_lo = read_cmd_byte(self, 4);
                let blk_hi = read_cmd_byte(self, 5);
                let blk_bnk = read_cmd_byte(self, 6);
                let buf = ((buf_hi as u16) << 8) | buf_lo as u16;
                let block = (blk_bnk as u32) << 16 | (blk_hi as u32) << 8 | blk_lo as u32;
                self.smartport_read_block(unit, buf, block, pbr)
            }
            0x02 => {
                // WRITE BLOCK
                let unit = read_cmd_byte(self, 1);
                let buf_lo = read_cmd_byte(self, 2);
                let buf_hi = read_cmd_byte(self, 3);
                let blk_lo = read_cmd_byte(self, 4);
                let blk_hi = read_cmd_byte(self, 5);
                let blk_bnk = read_cmd_byte(self, 6);
                let buf = ((buf_hi as u16) << 8) | buf_lo as u16;
                let block = (blk_bnk as u32) << 16 | (blk_hi as u32) << 8 | blk_lo as u32;
                self.smartport_write_block(unit, buf, block, pbr)
            }
            0x03..=0x07 => 0x00, // FORMAT, CONTROL, INIT, OPEN, CLOSE = success
            _ => 0x21,           // BAD CMD
        }
    }

    fn smartport_status(&mut self, unit: u8, status_code: u8, list_ptr: u16, pbr: u8) -> u8 {
        let pbr_base = (pbr as u32) << 16;

        if unit == 0 {
            let device_count = (0..4).filter(|&i| self.smartport.has_disk(i)).count() as u8;
            self.write(pbr_base | list_ptr as u32, device_count, 0);
            self.write(pbr_base | list_ptr.wrapping_add(1) as u32, 0xFF, 0);
            self.write(pbr_base | list_ptr.wrapping_add(2) as u32, 0x00, 0);
            self.write(pbr_base | list_ptr.wrapping_add(3) as u32, 0x00, 0);
            self.write(pbr_base | list_ptr.wrapping_add(4) as u32, 0x00, 0);
            self.write(pbr_base | list_ptr.wrapping_add(5) as u32, 0x01, 0);
            self.write(pbr_base | list_ptr.wrapping_add(6) as u32, 0x0F, 0);
            self.write(pbr_base | list_ptr.wrapping_add(7) as u32, 0x00, 0);
            return 0x00;
        }

        let device = unit as usize - 1;
        if device >= 4 || !self.smartport.has_disk(device) {
            return 0x28; // NO DEVICE
        }
        let blocks = self.smartport.device_blocks(device);

        match status_code {
            0x00 => {
                self.write(pbr_base | list_ptr as u32, 0xF8, 0);
                self.write(pbr_base | list_ptr.wrapping_add(1) as u32, blocks as u8, 0);
                self.write(
                    pbr_base | list_ptr.wrapping_add(2) as u32,
                    (blocks >> 8) as u8,
                    0,
                );
                self.write(
                    pbr_base | list_ptr.wrapping_add(3) as u32,
                    (blocks >> 16) as u8,
                    0,
                );
                0x00
            }
            0x03 => {
                // Device info block
                self.write(pbr_base | list_ptr as u32, 0xF8, 0);
                self.write(pbr_base | list_ptr.wrapping_add(1) as u32, blocks as u8, 0);
                self.write(
                    pbr_base | list_ptr.wrapping_add(2) as u32,
                    (blocks >> 8) as u8,
                    0,
                );
                self.write(
                    pbr_base | list_ptr.wrapping_add(3) as u32,
                    (blocks >> 16) as u8,
                    0,
                );
                self.write(pbr_base | list_ptr.wrapping_add(4) as u32, 0x04, 0);
                let name = b"DISK";
                for (i, &b) in name.iter().enumerate() {
                    self.write(pbr_base | list_ptr.wrapping_add(5 + i as u16) as u32, b, 0);
                }
                for i in name.len()..16 {
                    self.write(
                        pbr_base | list_ptr.wrapping_add(5 + i as u16) as u32,
                        b' ',
                        0,
                    );
                }
                self.write(pbr_base | list_ptr.wrapping_add(21) as u32, 0x02, 0);
                self.write(pbr_base | list_ptr.wrapping_add(22) as u32, 0x20, 0);
                self.write(pbr_base | list_ptr.wrapping_add(23) as u32, 0x01, 0);
                self.write(pbr_base | list_ptr.wrapping_add(24) as u32, 0x00, 0);
                0x00
            }
            _ => 0x21,
        }
    }

    fn smartport_read_block(&mut self, unit: u8, buf: u16, block: u32, pbr: u8) -> u8 {
        if unit == 0 || unit > 4 {
            return 0x28;
        }
        let device = unit as usize - 1;
        if !self.smartport.has_disk(device) {
            return 0x28;
        }
        let Some(data) = self.smartport.read_block(device, block) else {
            return 0x2D;
        };
        let pbr_base = (pbr as u32) << 16;
        for (i, &b) in data.iter().enumerate() {
            self.write(pbr_base | buf.wrapping_add(i as u16) as u32, b, 0);
        }
        0x00
    }

    fn smartport_write_block(&mut self, unit: u8, buf: u16, block: u32, pbr: u8) -> u8 {
        if unit == 0 || unit > 4 {
            return 0x28;
        }
        let device = unit as usize - 1;
        if !self.smartport.has_disk(device) {
            return 0x28;
        }
        let pbr_base = (pbr as u32) << 16;
        let mut data = vec![0u8; 512];
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = self.read_raw(pbr_base | buf.wrapping_add(i as u16) as u32);
        }
        if self.smartport.write_block(device, block, &data) {
            0x00
        } else {
            0x2B
        }
    }
}
