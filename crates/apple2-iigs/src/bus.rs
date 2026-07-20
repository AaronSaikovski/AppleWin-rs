//! Apple IIgs memory bus.
//!
//! Implements the `Bus816` trait for the 65C816 CPU, providing the full
//! IIgs memory map with bank-switched RAM, ROM, I/O shadowing, and
//! Mega II compatibility.

use crate::adb::Adb;
use crate::bram;
use crate::clock::Clock;
use crate::cpu65816::Bus816;
use crate::ensoniq::Ensoniq;
use crate::fpi::Fpi;
use crate::iwm::Iwm;
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

    /// IWM (slot-6 5.25"/3.5" disk controller) — self-test registers only.
    pub iwm: Iwm,

    /// Battery-backed parameter RAM (256 bytes).
    pub bram: [u8; 256],

    /// Clock GLU (real-time clock + BRAM access via $C033/$C034).
    pub clock: Clock,

    /// IRQ line state — true when any interrupt source is active.
    pub irq_line: bool,

    /// Absolute video-frame index last processed by the interrupt heartbeat.
    /// Used to fire VBL/heartbeat interrupts once per 60 Hz frame.
    last_frame: u64,

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

        let mut adb = Adb::default();
        adb.rom03 = matches!(mem.rom_version, crate::memory::IIgsRomVersion::Rom03);

        Self {
            mem,
            mega2: Mega2::default(),
            fpi: Fpi::default(),
            adb,
            ensoniq: Ensoniq::default(),
            smartport: SmartPort::default(),
            iwm: Iwm::default(),
            bram: bram::factory_default_bram(),
            clock: Clock::default(),
            irq_line: false,
            last_frame: 0,
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

            // Banks $80-$DF: not populated on the IIgs — RAM lives in $00-$7F
            // (fast) and $E0-$E1 (slow). Reads return 0 (dummy memory) unless
            // the machine actually has RAM this high. Do NOT alias to $00-$5F:
            // GS/OS sizes RAM by probing banks, and an alias makes it detect
            // phantom RAM, then use it and corrupt the real bank $00.
            0x80..=0xDF => self.mem.ram_read(bank, offset),

            // Banks $E0-$E1: Fast RAM with the Mega II I/O aperture
            0xE0 | 0xE1 => self.read_fast_bank(bank - 0xE0, offset, cycles),

            // Banks $E2-$FB: unused / mirrors
            0xE2..=0xFB => 0x00,

            // Banks $FC-$FF: ROM
            0xFC..=0xFF => self.read_rom_bank(bank, offset),
        }
    }

    /// Dispatch a write based on bank and offset.
    fn bank_write(&mut self, bank: u8, offset: u16, val: u8, cycles: u64) {
        match bank {
            // Banks $00-$01: Slow RAM with I/O + shadowing
            0x00 | 0x01 => self.write_slow_bank(bank, offset, val, cycles),

            // Banks $02-$7F: Expansion RAM
            0x02..=0x7F => self.mem.ram_write(bank, offset, val),

            // Banks $80-$DF: not populated — writes beyond installed RAM are
            // discarded (see the read path). No aliasing to $00-$5F.
            0x80..=0xDF => self.mem.ram_write(bank, offset, val),

            // Banks $E0-$E1: Fast RAM with the Mega II I/O aperture
            0xE0 | 0xE1 => self.write_fast_bank(bank - 0xE0, offset, val, cycles),

            // Banks $E2-$FB: unused
            0xE2..=0xFB => {}

            // Banks $FC-$FF: ROM (read-only, writes ignored)
            0xFC..=0xFF => {}
        }
    }

    /// Read an I/O register in the `$C000-$C0FF` aperture.
    ///
    /// Shared by every bank that exposes the I/O aperture — the "slow" banks
    /// `$00`/`$01`, the "fast" banks `$E0`/`$E1`, and the ROM banks `$FC-$FF`.
    fn io_read_reg(&mut self, io_offset: u8, cycles: u64) -> u8 {
        self.fpi.io_access();

        // ADB and Ensoniq registers are handled by the bus directly.
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
            // Clock GLU ($C033 data / $C034 control).
            0x33 => self.clock.read_data(),
            0x34 => (self.clock.read_ctl() & 0xF0) | (self.mega2.border_color & 0x0F),
            // $C071-$C07F: not I/O — the IIgs exposes ROM bank $FF here, holding
            // the native interrupt-vector dispatch (e.g. the IRQ vector $C074 =
            // `CLV; JML $E10010`). Reads return the ROM byte.
            0x71..=0x7F => self.mem.rom_read(0xFF, 0xC000 | io_offset as u16),
            // IWM (slot 6 disk controller) — $C0E0-$C0EF.
            0xE0..=0xEF => self.iwm.access(io_offset & 0x0F, false, 0),
            _ => self.mega2.io_read(io_offset, cycles),
        };

        self.fpi.io_complete();
        val
    }

    /// Write an I/O register in the `$C000-$C0FF` aperture. Shared by every bank
    /// that exposes the I/O aperture (see [`io_read_reg`](Self::io_read_reg)).
    fn io_write_reg(&mut self, io_offset: u8, val: u8, cycles: u64) {
        self.fpi.io_access();

        match io_offset {
            0x26 => {
                // ADB micro-controller command / data write.
                self.adb.write_command(val, cycles);
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
            // Clock GLU ($C033 data / $C034 control). $C034's low nibble is the
            // video border colour, so update that too.
            0x33 => self.clock.write_data(val),
            0x34 => {
                self.mega2.border_color = val & 0x0F;
                self.clock.write_ctl(val, &mut self.bram);
            }
            // IWM (slot 6 disk controller) — $C0E0-$C0EF.
            0xE0..=0xEF => {
                self.iwm.access(io_offset & 0x0F, true, val);
            }
            _ => self.mega2.io_write(io_offset, val, cycles),
        }

        self.fpi.io_complete();
    }

    /// Read from a "slow" bank ($00 or $01) with I/O aperture handling.
    /// Select main (0) or auxiliary (1) memory for a bank-`$00` slow-RAM access,
    /// per the IIe-compatible soft switches (ALTZP for zero page/stack, RAMRD/
    /// RAMWRT for `$0200-$BFFF`, 80STORE+PAGE2 for the text/hi-res page-1 windows).
    /// The IIgs routes these bank-`$00` accesses to bank `$01` (aux); without this,
    /// GS/OS's aux-memory writes corrupt main memory. (Ported from KEGS.)
    fn aux_bank0(&self, offset: u16, is_write: bool) -> u8 {
        use apple2_core::bus::MemMode;
        let mm = self.mega2.mem_mode;

        // Zero page + stack ($0000-$01FF): ALTZP.
        if offset < 0x0200 {
            return mm.contains(MemMode::MF_ALTZP) as u8;
        }
        let st80 = mm.contains(MemMode::MF_80STORE);
        // Text page 1 ($0400-$07FF): 80STORE makes PAGE2 select the bank.
        if st80 && (0x0400..0x0800).contains(&offset) {
            return mm.contains(MemMode::MF_PAGE2) as u8;
        }
        // Hi-res page 1 ($2000-$3FFF): 80STORE+HIRES makes PAGE2 select the bank.
        if st80 && mm.contains(MemMode::MF_HIRES) && (0x2000..0x4000).contains(&offset) {
            return mm.contains(MemMode::MF_PAGE2) as u8;
        }
        // Everything else in $0200-$BFFF: RAMRD (read) / RAMWRT (write).
        let flag = if is_write {
            MemMode::MF_AUXWRITE
        } else {
            MemMode::MF_AUXREAD
        };
        mm.contains(flag) as u8
    }

    fn read_slow_bank(&mut self, bank: u8, offset: u16, cycles: u64) -> u8 {
        // I/O aperture: $C000-$C0FF
        if (0xC000..=0xC0FF).contains(&offset) {
            return self.io_read_reg((offset & 0xFF) as u8, cycles);
        }

        // Slot ROM area: $C100-$CFFF
        if (0xC100..=0xCFFF).contains(&offset) {
            // Return internal slot ROM from the ROM image
            let idx = (offset - 0xC100) as usize;
            return self.slot_rom_cache.get(idx).copied().unwrap_or(0);
        }

        // Language Card area: $D000-$FFFF — bank $00 access switches on ALTZP.
        if offset >= 0xD000 {
            let eff_bank = if bank == 0 {
                self.mega2
                    .mem_mode
                    .contains(apple2_core::bus::MemMode::MF_ALTZP) as u8
            } else {
                bank
            };
            return self.read_language_card(eff_bank, offset);
        }

        // Regular RAM — bank $00 accesses honour the aux-memory soft switches.
        let eff_bank = if bank == 0 {
            self.aux_bank0(offset, false)
        } else {
            bank
        };
        self.mem.ram_read(eff_bank, offset)
    }

    /// Write to a "slow" bank ($00 or $01) with I/O + shadowing.
    fn write_slow_bank(&mut self, bank: u8, offset: u16, val: u8, cycles: u64) {
        // I/O aperture: $C000-$C0FF
        if (0xC000..=0xC0FF).contains(&offset) {
            self.io_write_reg((offset & 0xFF) as u8, val, cycles);
            return;
        }

        // Slot ROM area: $C100-$CFFF — writes ignored (ROM)
        if (0xC100..=0xCFFF).contains(&offset) {
            return;
        }

        // Language Card area: $D000-$FFFF — bank $00 access switches on ALTZP.
        if offset >= 0xD000 {
            let eff_bank = if bank == 0 {
                self.mega2
                    .mem_mode
                    .contains(apple2_core::bus::MemMode::MF_ALTZP) as u8
            } else {
                bank
            };
            self.write_language_card(eff_bank, offset, val);
            // Shadow LC writes if enabled
            if bank == 0 {
                if self.mega2.shadow.should_shadow_bank0(offset) {
                    self.mem.fast_ram_write(eff_bank, offset, val);
                }
            } else if bank == 1 && self.mega2.shadow.should_shadow_bank1(offset) {
                self.mem.fast_ram_write(1, offset, val);
            }
            return;
        }

        // Regular RAM write — bank $00 accesses honour the aux-memory soft switches.
        let eff_bank = if bank == 0 {
            self.aux_bank0(offset, true)
        } else {
            bank
        };
        self.mem.ram_write(eff_bank, offset, val);

        // Apply shadowing: mirror to fast RAM ($E0/$E1). The shadow follows the
        // effective (main/aux) bank the write actually landed in.
        if bank == 0 {
            if self.mega2.shadow.should_shadow_bank0(offset) {
                self.mem.fast_ram_write(eff_bank, offset, val);
            }
        } else if bank == 1 && self.mega2.shadow.should_shadow_bank1(offset) {
            self.mem.fast_ram_write(1, offset, val);
        }
    }

    /// Read from a "fast" bank ($E0 or $E1).
    ///
    /// On real hardware banks $E0/$E1 are the Mega II side of memory and expose
    /// the same `$C000-$CFFF` I/O aperture and `$D000-$FFFF` language-card window
    /// as banks $00/$01, backed by fast RAM. The IIgs firmware runs its
    /// cold-start with `DBR=$E1` and polls hardware registers through this
    /// aperture, so it must be decoded here rather than read as plain RAM.
    fn read_fast_bank(&mut self, bank_offset: u8, offset: u16, cycles: u64) -> u8 {
        // I/O aperture: $C000-$C0FF
        if (0xC000..=0xC0FF).contains(&offset) {
            return self.io_read_reg((offset & 0xFF) as u8, cycles);
        }

        // Slot ROM area: $C100-$CFFF
        if (0xC100..=0xCFFF).contains(&offset) {
            let idx = (offset - 0xC100) as usize;
            return self.slot_rom_cache.get(idx).copied().unwrap_or(0);
        }

        // Language Card area: $D000-$FFFF
        if offset >= 0xD000 {
            return self.read_language_card_fast(bank_offset, offset);
        }

        self.mem.fast_ram_read(bank_offset, offset)
    }

    /// Write to a "fast" bank ($E0 or $E1). See [`read_fast_bank`](Self::read_fast_bank).
    fn write_fast_bank(&mut self, bank_offset: u8, offset: u16, val: u8, cycles: u64) {
        // I/O aperture: $C000-$C0FF
        if (0xC000..=0xC0FF).contains(&offset) {
            self.io_write_reg((offset & 0xFF) as u8, val, cycles);
            return;
        }

        // Slot ROM area: $C100-$CFFF — writes ignored (ROM)
        if (0xC100..=0xCFFF).contains(&offset) {
            return;
        }

        // Language Card area: $D000-$FFFF
        if offset >= 0xD000 {
            self.write_language_card_fast(bank_offset, offset, val);
            return;
        }

        self.mem.fast_ram_write(bank_offset, offset, val);
    }

    /// Language-card read for the fast banks — mirrors [`read_language_card`](Self::read_language_card)
    /// but backed by fast RAM ($E0/$E1).
    fn read_language_card_fast(&self, bank_offset: u8, offset: u16) -> u8 {
        use apple2_core::bus::MemMode;

        if self.mega2.mem_mode.contains(MemMode::MF_HIGHRAM) {
            if offset < 0xE000 && !self.mega2.mem_mode.contains(MemMode::MF_BANK2) {
                self.mem
                    .fast_ram_read(bank_offset, offset.wrapping_sub(0x1000))
            } else {
                self.mem.fast_ram_read(bank_offset, offset)
            }
        } else {
            self.mem.rom_read(0xFF, offset)
        }
    }

    /// Language-card write for the fast banks — mirrors [`write_language_card`](Self::write_language_card)
    /// but backed by fast RAM ($E0/$E1).
    fn write_language_card_fast(&mut self, bank_offset: u8, offset: u16, val: u8) {
        use apple2_core::bus::MemMode;

        if self.mega2.mem_mode.contains(MemMode::MF_WRITERAM) {
            if offset < 0xE000 && !self.mega2.mem_mode.contains(MemMode::MF_BANK2) {
                self.mem
                    .fast_ram_write(bank_offset, offset.wrapping_sub(0x1000), val);
            } else {
                self.mem.fast_ram_write(bank_offset, offset, val);
            }
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
    ///
    /// ROM banks are a linear image — the entire 64KB, including `$C000-$CFFF`,
    /// is ROM. Unlike banks `$00`/`$01`/`$E0`/`$E1`, the I/O aperture and slot
    /// ROM are *not* overlaid here: the firmware runs real code at `$FF/$C0xx`
    /// (e.g. `JSR $C085`) and reaches I/O via explicit bank-`$E0`/`$E1`/`$00`
    /// long addressing.
    fn read_rom_bank(&self, bank: u8, offset: u16) -> u8 {
        self.mem.rom_read(bank, offset)
    }

    /// Check if any SmartPort device has a disk inserted.
    pub fn smartport_has_disk(&self) -> bool {
        (0..4).any(|i| self.smartport.has_disk(i))
    }

    /// Update interrupt state. Called periodically from the emulator loop.
    pub fn update_interrupts(&mut self, cycles: u64) {
        // Update ADB controller
        self.adb.update(cycles);

        // Update VBL state
        self.mega2.update_vblank(cycles);

        // Drive the interrupt heartbeat once per elapsed video frame (60 Hz):
        // VBL, quarter-second, one-second, and VGC scan-line interrupts.
        let frame = cycles / crate::mega2::CYCLES_PER_FRAME;
        if frame != self.last_frame {
            // Cap the catch-up so a large cycle jump can't spin here.
            let elapsed = (frame - self.last_frame).min(4);
            self.last_frame = frame;
            let scan_wanted = self.scan_line_int_requested();
            for _ in 0..elapsed {
                self.mega2.heartbeat_vbl(scan_wanted);
            }
        }

        // Compose the CPU IRQ line from the Mega II/VGC sources, the ADB
        // keyboard interrupt, and the Ensoniq DOC oscillator interrupt. The DOC
        // interrupt is raised when an interrupt-enabled oscillator ends; the
        // firmware/GS-OS sound handler acknowledges it by reading DOC register
        // $E0 (which clears `ensoniq.irq_pending`), so this cannot storm. Many
        // sound routines (including the GS/OS startup jingle) sequence off this
        // interrupt and hang or loop the beep without it.
        let mut irq = self.mega2.irq_asserted();
        if self.adb.status & crate::adb::status::KEY_IRQ != 0 {
            irq = true;
        }
        if self.ensoniq.irq_pending {
            irq = true;
        }

        self.irq_line = irq;
    }

    /// True when Super Hi-Res is active and at least one scan-line control byte
    /// requests a scan-line interrupt (bit 6 of the SCB).
    ///
    /// The SCBs live in bank `$E1` at `$9D00-$9DC7` (200 lines). This is an
    /// approximation of the real per-scan-line timing: the interrupt is raised
    /// once per frame if any enabled line requests it, which is what heartbeat-
    /// driven software depends on.
    fn scan_line_int_requested(&self) -> bool {
        if !self.mega2.is_shr_enabled() {
            return false;
        }
        const SCB_BASE: usize = 0x1_9D00; // bank $E1 offset $9D00
        let fast = &self.mem.fast_ram;
        if fast.len() < SCB_BASE + 200 {
            return false;
        }
        fast[SCB_BASE..SCB_BASE + 200]
            .iter()
            .any(|&scb| scb & 0x40 != 0)
    }

    /// Reset the bus state (power cycle or warm reset).
    pub fn reset(&mut self, _power_cycle: bool) {
        self.mega2 = Mega2::default();
        self.fpi = Fpi::default();
        let rom03 = self.adb.rom03;
        self.adb = Adb::default();
        self.adb.rom03 = rom03;
        self.ensoniq = Ensoniq::default();
        self.iwm.reset();
        self.irq_line = false;
        self.last_frame = 0;
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

    fn wdm_trap(
        &mut self,
        signature: u8,
        sp: u16,
        pbr: u8,
        emulation: bool,
    ) -> Option<(u8, bool, Option<(u16, u16)>)> {
        match signature {
            SMARTPORT_TRAP_SIG => Some(self.smartport_trap(sp, pbr, emulation)),
            SMARTPORT_BOOT_SIG => Some(self.smartport_boot()),
            SMARTPORT_PRODOS_SIG => Some(self.smartport_prodos()),
            _ => None,
        }
    }
}

// ── SmartPort firmware stub ─────────────────────────────────────────────────

/// WDM signature byte used to mark a SmartPort firmware trap.
pub const SMARTPORT_TRAP_SIG: u8 = 0xFE;

/// WDM signature byte used to mark the SmartPort boot trap ($C508).
pub const SMARTPORT_BOOT_SIG: u8 = 0xFD;

/// WDM signature byte marking the ProDOS 8 block-driver trap ($C523).
pub const SMARTPORT_PRODOS_SIG: u8 = 0xFC;

/// Install a SmartPort firmware stub into the slot ROM cache at slot 5 ($C500-$C5FF).
fn install_smartport_stub(cache: &mut [u8]) {
    // Slot 5 is at offset $400-$4FF within the slot ROM cache ($C500 - $C100 = $400).
    let base = 0x400;
    if base + 0x100 > cache.len() {
        return;
    }
    let slot = &mut cache[base..base + 0x100];
    slot.fill(0x00);

    // ── Boot entry / identification ($C500) ─────────────────────────────
    // The firmware boots this slot by `JMP $C500`. The identification bytes
    // ($Cn01=$20, $Cn03=$00, $Cn05=$03, $Cn07=$00) double as the operands of
    // harmless LDX/LDY/LDA immediates, then execution falls into the loader.
    //   C500: A2 20  LDX #$20
    //   C502: A0 00  LDY #$00
    //   C504: A2 03  LDX #$03
    //   C506: A9 00  LDA #$00        ($Cn07 = $00 → SmartPort/block device)
    slot[0x00] = 0xA2;
    slot[0x01] = 0x20;
    slot[0x02] = 0xA0;
    slot[0x03] = 0x00;
    slot[0x04] = 0xA2;
    slot[0x05] = 0x03;
    slot[0x06] = 0xA9;
    slot[0x07] = 0x00;

    // ── Boot loader ($C508) ─────────────────────────────────────────────
    // The boot trap reads block 0 → $0800 and returns carry set when no
    // startup device is present. On failure we must NOT run the (unloaded)
    // boot block — executing the $00 $00 there is `BRK`, which drops the ROM
    // into the monitor. Instead we spin, retrying the read, mirroring real
    // hardware's "Check Startup Device" wait: as soon as a disk is inserted
    // the read succeeds and boot proceeds.
    //   C508: 42 FD     WDM $FD        (read block 0 → $0800; carry = error)
    //   C50A: B0 FC     BCS $C508      (no device → retry, i.e. wait for disk)
    //   C50C: A2 50     LDX #$50       (unit: slot 5, drive 1 — for boot block)
    //   C50E: A0 00     LDY #$00
    //   C510: 4C 01 08  JMP $0801      (execute the loaded boot block)
    slot[0x08] = 0x42;
    slot[0x09] = SMARTPORT_BOOT_SIG;
    slot[0x0A] = 0xB0;
    slot[0x0B] = 0xFC;
    slot[0x0C] = 0xA2;
    slot[0x0D] = 0x50;
    slot[0x0E] = 0xA0;
    slot[0x0F] = 0x00;
    slot[0x10] = 0x4C;
    slot[0x11] = 0x01;
    slot[0x12] = 0x08;

    // ── ProDOS block entry ($C523) + SmartPort entry ($C526) ────────────
    // The ProDOS block-driver entry is at $Cn00 + [$CnFF]; the SmartPort
    // dispatch entry is three bytes *higher* (ProDOS entry + 3), per the
    // Apple IIgs SmartPort ERS.
    //   C523: 42 FC 60  WDM $FC ; RTS   (ProDOS 8 — $42-$47 parameters)
    //   C526: 42 FE 60  WDM $FE ; RTS   (SmartPort — inline parameters)
    slot[0x23] = 0x42;
    slot[0x24] = SMARTPORT_PRODOS_SIG;
    slot[0x25] = 0x60;
    slot[0x26] = 0x42;
    slot[0x27] = SMARTPORT_TRAP_SIG;
    slot[0x28] = 0x60;

    // Pascal/SmartPort signature bytes at $CsFB-$CsFF.
    slot[0xFB] = 0x20;
    slot[0xFC] = 0x00;
    slot[0xFD] = 0x00;
    slot[0xFE] = 0xBC; // SmartPort + extended status + read + write + format
    slot[0xFF] = 0x23; // Offset from $Cs00 to the ProDOS block-driver entry
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
    pub fn smartport_trap(
        &mut self,
        sp: u16,
        pbr: u8,
        emulation: bool,
    ) -> (u8, bool, Option<(u16, u16)>) {
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

        // The inline call sequence after `JSR entry` is:
        //   DFB cmd ; DW cmd_list          (standard)
        //   DFB cmd|$40 ; ADRL cmd_list     (extended, 24-bit cmd_list)
        // Bit 6 of `cmd` marks an extended (GS/OS) call — see KEGS `do_c70d`.
        let pbr_base = (pbr as u32) << 16;
        let cmd = self.read_raw(pbr_base | inline_addr as u32);
        let ext = cmd & 0x40 != 0;
        let cl_lo = self.read_raw(pbr_base | (inline_addr.wrapping_add(1)) as u32) as u32;
        let cl_mid = self.read_raw(pbr_base | (inline_addr.wrapping_add(2)) as u32) as u32;
        let (cmdlist, inline_len) = if ext {
            let cl_hi = self.read_raw(pbr_base | (inline_addr.wrapping_add(3)) as u32) as u32;
            ((cl_hi << 16) | (cl_mid << 8) | cl_lo, 4u16)
        } else {
            // Standard: 16-bit cmd_list in the caller's program bank.
            (pbr_base | (cl_mid << 8) | cl_lo, 3u16)
        };

        // Advance the pushed return address past the inline parameter bytes.
        let new_ret = ret_minus_1.wrapping_add(inline_len);
        self.write(ret_lo_addr, new_ret as u8, 0);
        self.write(ret_hi_addr, (new_ret >> 8) as u8, 0);

        // On return, SmartPort reports the transfer/parameter count in X (low)
        // and Y (high) — GS/OS's device manager checks this.
        let (error, count) = self.dispatch_smartport_command(cmd, cmdlist, ext);
        let xy = (count & 0xFF, (count >> 8) & 0xFF);
        (error, error != 0, Some(xy))
    }

    /// Boot trap ($C508): read block 0 of the first SmartPort device into
    /// `$00/0800` so the following `JMP $0801` runs the ProDOS/GS-OS boot block.
    fn smartport_boot(&mut self) -> (u8, bool, Option<(u16, u16)>) {
        // Our SmartPort firmware lives at slot 5.
        const SLOT: u32 = 5;
        let err = self.smartport_read_block(1, 0x0800, 0);
        if err == 0 {
            // Set up the boot-device parameters exactly as KEGS `do_c700`, so
            // the boot block and GS/OS's device manager know where they booted
            // from. Without these, GS/OS renders the loader but can never bind
            // the boot volume and stalls. `$7F8` = boot slot; the ProDOS block
            // parameters at `$42-$47` describe the "read block 0 → $0800 from
            // unit slot<<4" that just happened.
            self.write(0x07F8, SLOT as u8, 0);
            self.write(0x42, 0x01, 0); // command = READ
            self.write(0x43, (SLOT << 4) as u8, 0); // unit ($50)
            self.write(0x44, 0x00, 0); // buffer lo
            self.write(0x45, 0x08, 0); // buffer hi ($0800)
            self.write(0x46, 0x00, 0); // block lo
            self.write(0x47, 0x00, 0); // block hi
        }
        // KEGS `do_c700` returns X = slot<<4 (the boot unit).
        (err, err != 0, Some(((SLOT << 4) as u16, 0)))
    }

    /// ProDOS 8 block-driver trap ($C523). Parameters come from zero page:
    /// `$42` command, `$43` unit (`DSSS0000`), `$44-45` buffer, `$46-47` block.
    /// Returns (A = error code, carry = error).
    fn smartport_prodos(&mut self) -> (u8, bool, Option<(u16, u16)>) {
        let cmd = self.read_raw(0x42);
        let unit = self.read_raw(0x43);
        let buf = (self.read_raw(0x44) as u16) | ((self.read_raw(0x45) as u16) << 8);
        let block = (self.read_raw(0x46) as u32) | ((self.read_raw(0x47) as u32) << 8);
        // ProDOS unit byte: bit 7 selects the drive; map to SmartPort device.
        let device = (unit >> 7) as usize;
        let sp_unit = device as u8 + 1;

        let err = match cmd {
            0x00 => {
                if self.smartport.has_disk(device) {
                    0x00
                } else {
                    0x28
                }
            }
            0x01 => self.smartport_read_block(sp_unit, buf as u32, block),
            0x02 => self.smartport_write_block(sp_unit, buf as u32, block),
            _ => 0x00, // FORMAT/other — succeed
        };
        // ProDOS block driver leaves X/Y unchanged.
        (err, err != 0, None)
    }

    /// Dispatch a SmartPort command. Faithful port of KEGS `do_c70d`: the low
    /// 6 bits of `cmd` select the operation, bit 6 (`ext`) marks an extended
    /// (GS/OS) call with 24-bit addresses. `cmdlist` is the full 24-bit address
    /// of the parameter list. Returns `(error_code, transfer_count)`; the count
    /// goes back to the caller in X/Y (0 = success).
    fn dispatch_smartport_command(&mut self, cmd: u8, cmdlist: u32, ext: bool) -> (u8, u16) {
        let rd = |bus: &Self, off: u32| bus.read_raw(cmdlist.wrapping_add(off) & 0xFF_FFFF);
        let unit = rd(self, 1);
        match cmd & 0x3F {
            0x00 => {
                // STATUS: status-list pointer (16-bit standard / 24-bit
                // extended) then the control code (KEGS: cmd_list + 4 + ext).
                let ptr = if ext {
                    (rd(self, 2) as u32)
                        | ((rd(self, 3) as u32) << 8)
                        | ((rd(self, 4) as u32) << 16)
                } else {
                    (rd(self, 2) as u32) | ((rd(self, 3) as u32) << 8)
                };
                let ctl = rd(self, if ext { 6 } else { 4 });
                self.smartport_status(unit, ctl, ptr & 0xFF_FFFF, ext)
            }
            0x01 => {
                let (buf, block) = self.sp_buf_block(cmdlist, ext);
                let err = self.smartport_read_block(unit, buf, block);
                (err, if err == 0 { 512 } else { 0 })
            }
            0x02 => {
                let (buf, block) = self.sp_buf_block(cmdlist, ext);
                let err = self.smartport_write_block(unit, buf, block);
                (err, if err == 0 { 512 } else { 0 })
            }
            // FORMAT / CONTROL / INIT / OPEN / CLOSE / READ/WRITE-char etc.
            0x03..=0x1F => (0x00, 0),
            _ => (0x21, 0), // BADCMD
        }
    }

    /// Extract the (buffer, block) parameters for READ/WRITE from the command
    /// list. Standard: 2-byte buffer + 3-byte block. Extended: 4-byte buffer +
    /// 4-byte block (KEGS `do_c70d` read/write paths).
    fn sp_buf_block(&self, cmdlist: u32, ext: bool) -> (u32, u32) {
        let rd = |off: u32| self.read_raw(cmdlist.wrapping_add(off) & 0xFF_FFFF) as u32;
        if ext {
            let buf = rd(2) | (rd(3) << 8) | (rd(4) << 16) | (rd(5) << 24);
            let block = rd(6) | (rd(7) << 8) | (rd(8) << 16) | (rd(9) << 24);
            (buf & 0xFF_FFFF, block)
        } else {
            let buf = rd(2) | (rd(3) << 8);
            let block = rd(4) | (rd(5) << 8) | (rd(6) << 16);
            (buf, block)
        }
    }

    /// SmartPort STATUS. `ptr` is the full 24-bit status-list address. Returns
    /// `(error_code, byte_count)` — the byte count is reported in X/Y.
    fn smartport_status(&mut self, unit: u8, ctl_code: u8, ptr: u32, ext: bool) -> (u8, u16) {
        let put =
            |bus: &mut Self, off: u32, v: u8| bus.write(ptr.wrapping_add(off) & 0xFF_FFFF, v, 0);

        // Unit 0, code 0: SmartPort bus / driver status (KEGS smartport.c:191).
        if unit == 0 && ctl_code == 0 {
            let count = (0..4).filter(|&i| self.smartport.has_disk(i)).count() as u8;
            put(self, 0, count); // number of connected devices
            put(self, 1, 0xFF); // interrupt status
            put(self, 2, 0x4B); // vendor id ($004B)
            put(self, 3, 0x00);
            put(self, 4, 0x00); // version ($1000)
            put(self, 5, 0x10);
            put(self, 6, 0x00);
            put(self, 7, 0x00);
            return (0x00, 8);
        }

        let device = unit as usize - 1;
        let present = unit >= 1 && device < 4 && self.smartport.has_disk(device);
        // Online/readable/writable/format bits, or $80 (offline) — NOT an error
        // for an empty-but-valid unit, matching KEGS.
        let stat_val = if present { 0xF8 } else { 0x80 };
        let blocks = if present {
            self.smartport.device_blocks(device)
        } else {
            0
        };

        match ctl_code {
            0x00 => {
                put(self, 0, stat_val);
                put(self, 1, blocks as u8);
                put(self, 2, (blocks >> 8) as u8);
                put(self, 3, (blocks >> 16) as u8);
                if ext {
                    put(self, 4, (blocks >> 24) as u8);
                }
                (0x00, if ext { 5 } else { 4 })
            }
            0x03 => {
                // Device Information Block.
                put(self, 0, stat_val);
                put(self, 1, blocks as u8);
                put(self, 2, (blocks >> 8) as u8);
                put(self, 3, (blocks >> 16) as u8);
                let base = if ext {
                    put(self, 4, (blocks >> 24) as u8);
                    5
                } else {
                    4
                };
                put(self, base, 4); // ID-string length
                let name = b"DISK            "; // 16 bytes, space-padded
                for (i, &b) in name.iter().enumerate() {
                    put(self, base + 1 + i as u32, b);
                }
                // Device type/subtype word ($0002 = block device) and version.
                put(self, base + 17, 0x02);
                put(self, base + 18, 0xC0);
                put(self, base + 19, 0x00);
                put(self, base + 20, 0x00);
                (if present { 0x00 } else { 0x28 }, if ext { 26 } else { 25 })
            }
            _ => (0x21, 0),
        }
    }

    /// SmartPort READBLOCK. `buf` is a full 24-bit destination address.
    fn smartport_read_block(&mut self, unit: u8, buf: u32, block: u32) -> u8 {
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
        for (i, &b) in data.iter().enumerate() {
            self.write(buf.wrapping_add(i as u32) & 0xFF_FFFF, b, 0);
        }
        0x00
    }

    /// SmartPort WRITEBLOCK. `buf` is a full 24-bit source address.
    fn smartport_write_block(&mut self, unit: u8, buf: u32, block: u32) -> u8 {
        if unit == 0 || unit > 4 {
            return 0x28;
        }
        let device = unit as usize - 1;
        if !self.smartport.has_disk(device) {
            return 0x28;
        }
        let mut data = vec![0u8; 512];
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = self.read_raw(buf.wrapping_add(i as u32) & 0xFF_FFFF);
        }
        if self.smartport.write_block(device, block, &data) {
            0x00
        } else {
            0x2B
        }
    }
}
