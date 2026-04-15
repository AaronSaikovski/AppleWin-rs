//! Mega II compatibility layer.
//!
//! The Mega II chip in the Apple IIgs provides backwards compatibility with
//! the Apple IIe by handling the same soft-switch addresses at $C000-$C0FF.
//! It also adds IIgs-specific registers for new video modes, sound, speed
//! control, and shadowing.
//!
//! The Mega II operates at 1 MHz regardless of the system speed setting.
//! Accessing Mega II I/O temporarily forces the CPU to 1 MHz.

use apple2_core::bus::MemMode;
use serde::{Deserialize, Serialize};

/// Safety cap on speaker toggle accumulation (matches the IIe bus cap).
const SPEAKER_TOGGLES_MAX: usize = 65_536;

use crate::shadowing::ShadowReg;

/// Mega II state — IIe-compatible and IIgs-specific soft-switch registers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mega2 {
    /// IIe-compatible memory mode flags.
    pub mem_mode: MemMode,

    /// Keyboard data latch ($C000 read).
    pub keyboard_data: u8,

    /// Keyboard strobe (bit 7 of $C000). Set when key pressed, cleared by $C010.
    pub key_strobe: bool,

    // ── IIgs-specific registers ──────────────────────────────────────────
    /// NEWVIDEO register ($C029).
    /// Bit 7: Super Hi-Res enable (1 = SHR, 0 = IIe video)
    /// Bit 6: SHR line mode (1 = 200 lines, 0 = reserved)
    /// Bit 5: Linearize SHR bank $E1 (1 = linear, 0 = bank-switched)
    /// Bit 0: Monochrome SHR
    pub new_video: u8,

    /// Text/background colour register ($C022).
    /// Bits 7-4: background colour
    /// Bits 3-0: text colour
    pub text_color: u8,

    /// VGC interrupt register ($C023).
    /// Bit 7: VGC interrupt occurred
    /// Bit 6: reserved
    /// Bit 5: 1-second interrupt enable
    /// Bit 4: scanline interrupt enable
    /// Bit 3-0: reserved
    pub vgc_int: u8,

    /// Border color register ($C034).
    /// Bits 3-0: border color (from current SHR palette 0)
    pub border_color: u8,

    /// Shadow register ($C035).
    pub shadow: ShadowReg,

    /// Speed register / CYAREG ($C036).
    /// Bit 7: 1 = fast (2.8 MHz), 0 = slow (1 MHz)
    /// Bit 6: reserved
    /// Bit 5: slot 7 motor on forces slow
    /// Bit 4: shadow in all banks
    /// Bits 3-0: slot motor detect enables
    pub speed_reg: u8,

    /// State register / STATEREG ($C068).
    /// Mirrors ALTZP, PAGE2, RAMRD, RAMWRT, LCBANK2, LCRAM, INTCXROM, SLOTC3ROM.
    /// Writing this register sets multiple memory mode flags at once.
    pub state_reg: u8,

    /// Sound control register ($C03C) - GLU address for DOC access.
    pub sound_ctrl: u8,

    /// Sound data register ($C03D) - read/write DOC registers.
    pub sound_data: u8,

    /// Sound address low ($C03E).
    pub sound_addr_lo: u8,

    /// Sound address high ($C03F).
    pub sound_addr_hi: u8,

    /// ADB data register ($C026).
    pub adb_data: u8,

    /// ADB status register ($C027).
    pub adb_status: u8,

    /// ADB modifier key register ($C025).
    /// Bit 7: keyboard data available
    /// Bit 6: reserved
    /// Bit 5: reserved
    /// Bit 4: option key
    /// Bit 3: open-apple (command) key
    /// Bit 2: closed-apple (option/alt) key
    /// Bit 1: caps lock
    /// Bit 0: shift key
    pub adb_modifiers: u8,

    /// Slot ROM select ($C02D).
    pub slot_rom_select: u8,

    /// Language Card pre-write flag (double-read activation).
    pub lc_prewrite: bool,

    /// Language Card last access register.
    pub lc_last_access: u8,

    /// Speaker state (toggled by $C030 access).
    pub speaker_state: bool,

    /// Speaker toggle timestamps for audio synthesis.
    pub speaker_toggles: Vec<u64>,

    /// Annunciator outputs 0-3.
    pub ann: [bool; 4],

    /// VBLANK state: true when in vertical blanking interval.
    pub vblank: bool,

    /// Cycle count at start of current frame (for VBLANK timing).
    pub frame_start_cycles: u64,

    /// Mega II interrupt flags ($C041).
    pub mega2_int: u8,

    /// Diagnostic speed register ($C046) - read-only.
    pub diag_speed: u8,
}

impl Default for Mega2 {
    fn default() -> Self {
        Self {
            mem_mode: MemMode::MF_BANK2 | MemMode::MF_WRITERAM,
            keyboard_data: 0,
            key_strobe: false,
            new_video: 0x01,  // SHR off, monochrome off initially. bit 0 set = 40col
            text_color: 0xF0, // white text, black background
            vgc_int: 0,
            border_color: 0,
            shadow: ShadowReg::default(),
            speed_reg: 0x80, // fast mode by default
            state_reg: 0,
            sound_ctrl: 0,
            sound_data: 0,
            sound_addr_lo: 0,
            sound_addr_hi: 0,
            adb_data: 0,
            adb_status: 0,
            adb_modifiers: 0,
            slot_rom_select: 0,
            lc_prewrite: false,
            lc_last_access: 0,
            speaker_state: false,
            speaker_toggles: Vec::new(),
            ann: [false; 4],
            vblank: false,
            frame_start_cycles: 0,
            mega2_int: 0,
            diag_speed: 0,
        }
    }
}

/// Total cycles per NTSC frame (262 lines * 65 cycles/line).
const CYCLES_PER_FRAME: u64 = 17_030;
/// Cycles in the visible region (192 lines * 65 cycles/line).
const CYCLES_VISIBLE: u64 = 12_480;

impl Mega2 {
    /// Handle a read from the I/O space ($C000-$C0FF).
    /// Returns the byte read.
    pub fn io_read(&mut self, offset: u8, cycles: u64) -> u8 {
        match offset {
            // ── Keyboard ────────────────────────────────────────────────
            0x00 => self.keyboard_data | if self.key_strobe { 0x80 } else { 0 },
            0x10 => {
                let val = self.keyboard_data | if self.key_strobe { 0x80 } else { 0 };
                self.key_strobe = false;
                val
            }

            // ── Memory mode flag reads (IIe compatible) ─────────────────
            0x11 => flag_byte(self.mem_mode.contains(MemMode::MF_BANK2)),
            0x12 => flag_byte(self.mem_mode.contains(MemMode::MF_HIGHRAM)),
            0x13 => flag_byte(self.mem_mode.contains(MemMode::MF_AUXREAD)),
            0x14 => flag_byte(self.mem_mode.contains(MemMode::MF_AUXWRITE)),
            0x15 => flag_byte(self.mem_mode.contains(MemMode::MF_INTCXROM)),
            0x16 => flag_byte(self.mem_mode.contains(MemMode::MF_ALTZP)),
            0x17 => flag_byte(self.mem_mode.contains(MemMode::MF_SLOTC3ROM)),
            0x18 => flag_byte(self.mem_mode.contains(MemMode::MF_80STORE)),
            0x19 => {
                // VBLANK: compute from cycle count relative to frame start
                let frame_offset = cycles.wrapping_sub(self.frame_start_cycles) % CYCLES_PER_FRAME;
                flag_byte(frame_offset >= CYCLES_VISIBLE)
            }
            0x1A => flag_byte(self.mem_mode.contains(MemMode::MF_GRAPHICS)),
            0x1B => flag_byte(self.mem_mode.contains(MemMode::MF_MIXED)),
            0x1C => flag_byte(self.mem_mode.contains(MemMode::MF_PAGE2)),
            0x1D => flag_byte(self.mem_mode.contains(MemMode::MF_HIRES)),
            0x1E => flag_byte(self.mem_mode.contains(MemMode::MF_ALTCHAR)),
            0x1F => flag_byte(self.mem_mode.contains(MemMode::MF_VID80)),

            // ── IIgs-specific registers ─────────────────────────────────
            0x22 => self.text_color,
            0x23 => self.vgc_int,
            // $C024-$C027: ADB registers — handled by the bus, not here
            0x28 => 0x00, // ROMBANK (ROM bank register) — not implemented
            0x29 => self.new_video,
            0x2B => 0x00, // Monochrome monitor mode
            0x2C => 0x00, // Slot interrupt flags (read-only)
            0x2D => self.slot_rom_select,
            0x2E => 0x00, // Byte disable register (VGC)
            0x2F => 0x00, // SCC read (serial)

            // ── Speaker ──────────────────────────────────────────────────
            0x30 => {
                self.speaker_state = !self.speaker_state;
                if self.speaker_toggles.len() < SPEAKER_TOGGLES_MAX {
                    self.speaker_toggles.push(cycles);
                }
                0x00
            }

            // ── VGC interrupt clear ($C032) ──────────────────────────────
            0x32 => {
                self.vgc_int &= !0x80; // clear interrupt flag
                0x00
            }

            // ── Border + shadow + speed ──────────────────────────────────
            0x34 => self.border_color,
            0x35 => self.shadow.bits(),
            0x36 => self.speed_reg,

            // ── Sound GLU registers ──────────────────────────────────────
            0x3C => self.sound_ctrl,
            0x3D => self.sound_data,
            0x3E => self.sound_addr_lo,
            0x3F => self.sound_addr_hi,

            // ── Diagnostic strobe ($C040) ────────────────────────────────
            0x40 => 0x00,

            // ── Mega II interrupt flags ──────────────────────────────────
            0x41 => self.mega2_int,

            // ── Diagnostic speed ─────────────────────────────────────────
            0x46 => {
                // Bit 0: 1 = fast, 0 = slow (read-only)
                if self.speed_reg & 0x80 != 0 {
                    0x01
                } else {
                    0x00
                }
            }

            // ── Game I/O (paddles/buttons) ───────────────────────────────
            0x61 => 0x00,        // PB0 (open apple) - not pressed
            0x62 => 0x00,        // PB1 (closed apple) - not pressed
            0x63 => 0x00,        // PB2
            0x64..=0x67 => 0x00, // Paddle values (not implemented)
            0x70 => 0x00,        // Paddle strobe

            // ── Video switches (IIe compatible) ──────────────────────────
            0x50 => {
                self.mem_mode.insert(MemMode::MF_GRAPHICS);
                0
            }
            0x51 => {
                self.mem_mode.remove(MemMode::MF_GRAPHICS);
                0
            }
            0x52 => {
                self.mem_mode.remove(MemMode::MF_MIXED);
                0
            }
            0x53 => {
                self.mem_mode.insert(MemMode::MF_MIXED);
                0
            }
            0x54 => {
                self.mem_mode.remove(MemMode::MF_PAGE2);
                0
            }
            0x55 => {
                self.mem_mode.insert(MemMode::MF_PAGE2);
                0
            }
            0x56 => {
                self.mem_mode.remove(MemMode::MF_HIRES);
                0
            }
            0x57 => {
                self.mem_mode.insert(MemMode::MF_HIRES);
                0
            }

            // DHIRES on/off
            0x5E => {
                self.mem_mode.insert(MemMode::MF_DHIRES);
                0
            }
            0x5F => {
                self.mem_mode.remove(MemMode::MF_DHIRES);
                0
            }

            // ── State register ───────────────────────────────────────────
            0x68 => self.read_state_reg(),

            // ── Language Card switches ($C080-$C08F) ─────────────────────
            0x80..=0x8F => {
                self.handle_language_card(offset);
                0x00
            }

            _ => 0x00,
        }
    }

    /// Handle a write to the I/O space ($C000-$C0FF).
    pub fn io_write(&mut self, offset: u8, val: u8, cycles: u64) {
        match offset {
            // ── Memory mode switches (IIe compatible) ────────────────────
            0x00 => {
                // 80STORE off
                self.mem_mode.remove(MemMode::MF_80STORE);
            }
            0x01 => {
                // 80STORE on
                self.mem_mode.insert(MemMode::MF_80STORE);
            }
            0x02 => self.mem_mode.remove(MemMode::MF_AUXREAD),
            0x03 => self.mem_mode.insert(MemMode::MF_AUXREAD),
            0x04 => self.mem_mode.remove(MemMode::MF_AUXWRITE),
            0x05 => self.mem_mode.insert(MemMode::MF_AUXWRITE),
            0x06 => self.mem_mode.remove(MemMode::MF_INTCXROM),
            0x07 => self.mem_mode.insert(MemMode::MF_INTCXROM),
            0x08 => self.mem_mode.remove(MemMode::MF_ALTZP),
            0x09 => self.mem_mode.insert(MemMode::MF_ALTZP),
            0x0A => self.mem_mode.remove(MemMode::MF_SLOTC3ROM),
            0x0B => self.mem_mode.insert(MemMode::MF_SLOTC3ROM),

            // 80COL / ALTCHAR
            0x0C => self.mem_mode.remove(MemMode::MF_VID80),
            0x0D => self.mem_mode.insert(MemMode::MF_VID80),
            0x0E => self.mem_mode.remove(MemMode::MF_ALTCHAR),
            0x0F => self.mem_mode.insert(MemMode::MF_ALTCHAR),

            // Keyboard strobe clear
            0x10 => self.key_strobe = false,

            // ── IIgs-specific registers ──────────────────────────────────
            0x22 => self.text_color = val,
            0x23 => {
                // VGC interrupt register — enable bits
                // Bit 4: scanline interrupt enable
                // Bit 5: one-second interrupt enable
                self.vgc_int = (self.vgc_int & 0xE0) | (val & 0x1F);
            }
            // $C026-$C027: ADB writes — handled by the bus, not here
            0x29 => self.new_video = val,
            0x2D => self.slot_rom_select = val,
            0x32 => {
                // VGC interrupt clear
                self.vgc_int &= !0x80;
            }

            // ── Speaker ──────────────────────────────────────────────────
            0x30 => {
                self.speaker_state = !self.speaker_state;
                if self.speaker_toggles.len() < SPEAKER_TOGGLES_MAX {
                    self.speaker_toggles.push(cycles);
                }
            }

            // ── Border + shadow + speed ──────────────────────────────────
            0x34 => self.border_color = val & 0x0F,
            0x35 => self.shadow = ShadowReg::from_bits_truncate(val),
            0x36 => self.speed_reg = val,

            // ── Sound GLU registers ──────────────────────────────────────
            0x3C => self.sound_ctrl = val,
            0x3D => self.sound_data = val,
            0x3E => self.sound_addr_lo = val,
            0x3F => self.sound_addr_hi = val,

            // ── Video switches (IIe compatible) ──────────────────────────
            0x50 => self.mem_mode.insert(MemMode::MF_GRAPHICS),
            0x51 => self.mem_mode.remove(MemMode::MF_GRAPHICS),
            0x52 => self.mem_mode.remove(MemMode::MF_MIXED),
            0x53 => self.mem_mode.insert(MemMode::MF_MIXED),
            0x54 => self.mem_mode.remove(MemMode::MF_PAGE2),
            0x55 => self.mem_mode.insert(MemMode::MF_PAGE2),
            0x56 => self.mem_mode.remove(MemMode::MF_HIRES),
            0x57 => self.mem_mode.insert(MemMode::MF_HIRES),

            // DHIRES on/off
            0x5E => self.mem_mode.insert(MemMode::MF_DHIRES),
            0x5F => self.mem_mode.remove(MemMode::MF_DHIRES),

            // ── State register ───────────────────────────────────────────
            0x68 => self.write_state_reg(val),

            // ── Language Card switches ($C080-$C08F) ─────────────────────
            0x80..=0x8F => {
                self.handle_language_card(offset);
            }

            _ => {}
        }
    }

    /// Handle language card soft-switches ($C080-$C08F).
    fn handle_language_card(&mut self, offset: u8) {
        let reg = offset & 0x0F;

        // Determine BANK2 vs BANK1: bit 3 selects
        if reg & 0x08 == 0 {
            self.mem_mode.insert(MemMode::MF_BANK2);
        } else {
            self.mem_mode.remove(MemMode::MF_BANK2);
        }

        // Bits 0-1 determine read/write mode:
        // 00: read ROM, write protect
        // 01: read ROM, write enable (needs 2 reads)
        // 10: read RAM, write protect
        // 11: read RAM, write enable (needs 2 reads)
        let mode_bits = reg & 0x03;
        match mode_bits {
            0 => {
                self.mem_mode.remove(MemMode::MF_HIGHRAM);
                self.mem_mode.remove(MemMode::MF_WRITERAM);
                self.lc_prewrite = false;
            }
            1 => {
                self.mem_mode.remove(MemMode::MF_HIGHRAM);
                if self.lc_prewrite && self.lc_last_access == reg {
                    self.mem_mode.insert(MemMode::MF_WRITERAM);
                }
                self.lc_prewrite = true;
            }
            2 => {
                self.mem_mode.insert(MemMode::MF_HIGHRAM);
                self.mem_mode.remove(MemMode::MF_WRITERAM);
                self.lc_prewrite = false;
            }
            3 => {
                self.mem_mode.insert(MemMode::MF_HIGHRAM);
                if self.lc_prewrite && self.lc_last_access == reg {
                    self.mem_mode.insert(MemMode::MF_WRITERAM);
                }
                self.lc_prewrite = true;
            }
            _ => unreachable!(),
        }
        self.lc_last_access = reg;
    }

    /// Read the STATEREG ($C068) — packs multiple memory mode flags into one byte.
    fn read_state_reg(&self) -> u8 {
        let mut val = 0u8;
        if self.mem_mode.contains(MemMode::MF_ALTZP) {
            val |= 0x80;
        }
        if self.mem_mode.contains(MemMode::MF_PAGE2) {
            val |= 0x40;
        }
        if self.mem_mode.contains(MemMode::MF_AUXREAD) {
            val |= 0x20;
        }
        if self.mem_mode.contains(MemMode::MF_AUXWRITE) {
            val |= 0x10;
        }
        if self.mem_mode.contains(MemMode::MF_BANK2) {
            val |= 0x08;
        }
        if self.mem_mode.contains(MemMode::MF_HIGHRAM) {
            val |= 0x04;
        }
        if self.mem_mode.contains(MemMode::MF_INTCXROM) {
            val |= 0x02;
        }
        if self.mem_mode.contains(MemMode::MF_SLOTC3ROM) {
            val |= 0x01;
        }
        val
    }

    /// Write the STATEREG ($C068) — sets multiple memory mode flags at once.
    fn write_state_reg(&mut self, val: u8) {
        self.mem_mode.set(MemMode::MF_ALTZP, val & 0x80 != 0);
        self.mem_mode.set(MemMode::MF_PAGE2, val & 0x40 != 0);
        self.mem_mode.set(MemMode::MF_AUXREAD, val & 0x20 != 0);
        self.mem_mode.set(MemMode::MF_AUXWRITE, val & 0x10 != 0);
        self.mem_mode.set(MemMode::MF_BANK2, val & 0x08 != 0);
        self.mem_mode.set(MemMode::MF_HIGHRAM, val & 0x04 != 0);
        self.mem_mode.set(MemMode::MF_INTCXROM, val & 0x02 != 0);
        self.mem_mode.set(MemMode::MF_SLOTC3ROM, val & 0x01 != 0);
    }

    /// Process a key press from the host.
    pub fn key_press(&mut self, key: u8) {
        self.keyboard_data = key;
        self.key_strobe = true;
        // Set ADB data available flag
        self.adb_status |= 0x20;
    }

    /// Update VBLANK state based on cycle count.
    pub fn update_vblank(&mut self, cycles: u64) {
        let frame_offset = cycles.wrapping_sub(self.frame_start_cycles) % CYCLES_PER_FRAME;
        let new_vblank = frame_offset >= CYCLES_VISIBLE;
        if new_vblank && !self.vblank {
            // Entering VBL — advance frame start
            if frame_offset < CYCLES_PER_FRAME / 2 {
                // We wrapped around
                self.frame_start_cycles = cycles - frame_offset;
            }
        }
        self.vblank = new_vblank;
    }

    /// Check if the system is in fast mode (2.8 MHz).
    #[inline]
    pub fn is_fast_mode(&self) -> bool {
        self.speed_reg & 0x80 != 0
    }

    /// Check if SHR mode is enabled.
    #[inline]
    pub fn is_shr_enabled(&self) -> bool {
        self.new_video & 0x80 != 0
    }
}

/// Convert a boolean flag to a soft-switch read byte.
/// Bit 7 reflects the flag state.
#[inline]
fn flag_byte(set: bool) -> u8 {
    if set { 0x80 } else { 0x00 }
}
