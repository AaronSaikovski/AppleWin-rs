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
            self.io_write_reg((offset & 0xFF) as u8, val, cycles);
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

        // Compose the CPU IRQ line from the Mega II/VGC sources plus the ADB
        // keyboard interrupt.
        let mut irq = self.mega2.irq_asserted();
        if self.adb.status & crate::adb::status::KEY_IRQ != 0 {
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

    fn wdm_trap(&mut self, signature: u8, sp: u16, pbr: u8, emulation: bool) -> Option<(u8, bool)> {
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
    //   C508: 42 FD  WDM $FD         (read block 0 → $0800)
    //   C50A: A2 50  LDX #$50        (unit: slot 5, drive 1 — for the boot block)
    //   C50C: A0 00  LDY #$00
    //   C50E: 4C 01 08  JMP $0801    (execute the loaded boot block)
    slot[0x08] = 0x42;
    slot[0x09] = SMARTPORT_BOOT_SIG;
    slot[0x0A] = 0xA2;
    slot[0x0B] = 0x50;
    slot[0x0C] = 0xA0;
    slot[0x0D] = 0x00;
    slot[0x0E] = 0x4C;
    slot[0x0F] = 0x01;
    slot[0x10] = 0x08;

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

    /// Boot trap ($C508): read block 0 of the first SmartPort device into
    /// `$00/0800` so the following `JMP $0801` runs the ProDOS/GS-OS boot block.
    fn smartport_boot(&mut self) -> (u8, bool) {
        let err = self.smartport_read_block(1, 0x0800, 0, 0x00);
        (err, err != 0)
    }

    /// ProDOS 8 block-driver trap ($C523). Parameters come from zero page:
    /// `$42` command, `$43` unit (`DSSS0000`), `$44-45` buffer, `$46-47` block.
    /// Returns (A = error code, carry = error).
    fn smartport_prodos(&mut self) -> (u8, bool) {
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
            0x01 => self.smartport_read_block(sp_unit, buf, block, 0x00),
            0x02 => self.smartport_write_block(sp_unit, buf, block, 0x00),
            _ => 0x00, // FORMAT/other — succeed
        };
        (err, err != 0)
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
