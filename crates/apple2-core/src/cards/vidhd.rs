//! VidHD card emulation.
//!
//! The VidHD is a modern HDMI output card for the Apple IIe/IIgs that provides
//! super hi-res (SHR) video output. This emulation provides:
//! - ROM identification bytes so software can detect VidHD presence
//! - IIgs-style SHR status register emulation
//! - A status byte for VidHD detection
//!
//! Reference: source/Video.cpp

use crate::card::{Card, CardType};
use crate::error::Result;
use std::io::{Read, Write};

// ── VidHD identification bytes ───────────────────────────────────────────────
// The VidHD ROM contains specific identification bytes that software uses
// to detect the card's presence.

fn make_vidhd_rom() -> Box<[u8; 256]> {
    let mut rom = Box::new([0u8; 256]);
    // VidHD identification signature
    // $Cn00: 24 — BIT zpg (skip next byte)
    rom[0x00] = 0x24;
    // $Cn01: EA — NOP (detected by firmware scan)
    rom[0x01] = 0xEA;
    // $Cn02: 4C — JMP abs (standard card init pattern)
    rom[0x02] = 0x4C;

    // VidHD specific ID bytes at known offsets
    // These are checked by AppleWin and other emulators
    rom[0x05] = 0x38; // SEC
    rom[0x07] = 0x18; // CLC
    rom[0x0B] = 0x01; // card ID
    rom[0x0C] = 0x24; // VidHD marker

    // "VIDHD" ASCII signature at offset 0x10
    rom[0x10] = b'V';
    rom[0x11] = b'I';
    rom[0x12] = b'D';
    rom[0x13] = b'H';
    rom[0x14] = b'D';

    // SHR status register identification
    // Firmware version at 0xFB-0xFF
    rom[0xFB] = 0x01; // version major
    rom[0xFC] = 0x00; // version minor
    rom[0xFD] = 0x00; // revision
    rom[0xFE] = 0x00;
    rom[0xFF] = 0x00;
    rom
}

// ── SHR (Super Hi-Res) status bits ──────────────────────────────────────────

/// VidHD status register bits (read from slot I/O reg 0).
const SHR_STATUS_VBLANK: u8 = 0x80; // bit 7: vertical blank active
#[allow(dead_code)]
const SHR_STATUS_HBLANK: u8 = 0x40; // bit 6: horizontal blank active
const SHR_STATUS_SHR_EN: u8 = 0x20; // bit 5: SHR mode enabled

// ── Struct ─────────────────────────────────────────────────────────────────────

pub struct VidHdCard {
    slot: usize,
    /// Card ROM with identification bytes.
    rom: Box<[u8; 256]>,
    /// SHR status register.
    status: u8,
    /// True if SHR mode has been enabled by software.
    shr_enabled: bool,
    /// VBlank toggle counter for simulating vertical blank.
    vblank_counter: u32,
    /// New-video control register (IIgs-style).
    new_video: u8,
}

impl VidHdCard {
    pub fn new(slot: usize) -> Self {
        Self {
            slot,
            rom: make_vidhd_rom(),
            status: 0,
            shr_enabled: false,
            vblank_counter: 0,
            new_video: 0,
        }
    }

    /// Returns true if SHR mode is currently enabled.
    pub fn is_shr_enabled(&self) -> bool {
        self.shr_enabled
    }
}

impl Card for VidHdCard {
    fn card_type(&self) -> CardType {
        CardType::VidHD
    }
    fn slot(&self) -> usize {
        self.slot
    }

    fn io_read(&mut self, offset: u8, _cycles: u64) -> u8 {
        self.rom[offset as usize]
    }

    fn io_write(&mut self, _offset: u8, _value: u8, _cycles: u64) {}

    fn cx_rom(&self) -> Option<&[u8; 256]> {
        Some(&self.rom)
    }

    fn slot_io_read(&mut self, reg: u8, _cycles: u64) -> u8 {
        match reg & 0x0F {
            // Status register — returns SHR status + VBlank
            0x00 => {
                let mut s = self.status;
                if self.shr_enabled {
                    s |= SHR_STATUS_SHR_EN;
                }
                s
            }
            // New-video register (IIgs $C029 equivalent)
            0x01 => self.new_video,
            // VidHD presence detection — returns a non-FF value
            0x02 => 0x56, // 'V' for VidHD
            0x03 => 0x48, // 'H' for HD
            _ => 0xFF,
        }
    }

    fn slot_io_write(&mut self, reg: u8, val: u8, _cycles: u64) {
        match reg & 0x0F {
            // Control register — enable/disable SHR
            0x00 => {
                self.shr_enabled = val & 0x01 != 0;
            }
            // New-video register (IIgs $C029 equivalent)
            0x01 => {
                self.new_video = val;
                // Bit 7 of NEWVIDEO enables SHR mode on IIgs
                self.shr_enabled = val & 0x80 != 0;
            }
            _ => {}
        }
    }

    fn reset(&mut self, _power_cycle: bool) {
        self.status = 0;
        self.shr_enabled = false;
        self.vblank_counter = 0;
        self.new_video = 0;
    }

    fn update(&mut self, _cycles: u64) {
        // Simulate VBlank toggling (~60Hz)
        self.vblank_counter = self.vblank_counter.wrapping_add(1);
        // Toggle VBlank roughly every 4 update ticks (~60Hz at ~17ms per update)
        if self.vblank_counter.is_multiple_of(4) {
            self.status ^= SHR_STATUS_VBLANK;
        }
    }

    fn save_state(&self, out: &mut dyn Write) -> Result<()> {
        out.write_all(&[
            1u8, // version
            self.status,
            u8::from(self.shr_enabled),
            self.new_video,
        ])?;
        Ok(())
    }

    fn load_state(&mut self, src: &mut dyn Read, _version: u32) -> Result<()> {
        let mut buf = [0u8; 4];
        src.read_exact(&mut buf)?;
        // buf[0] = version
        self.status = buf[1];
        self.shr_enabled = buf[2] != 0;
        self.new_video = buf[3];
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rom_identification() {
        let card = VidHdCard::new(7);
        let rom = card.cx_rom().expect("VidHD should have ROM");
        // Check VIDHD ASCII signature
        assert_eq!(rom[0x10], b'V');
        assert_eq!(rom[0x11], b'I');
        assert_eq!(rom[0x12], b'D');
        assert_eq!(rom[0x13], b'H');
        assert_eq!(rom[0x14], b'D');
    }

    #[test]
    fn test_presence_detection() {
        let mut card = VidHdCard::new(7);
        // Read slot I/O presence bytes
        let v = card.slot_io_read(0x02, 0);
        let h = card.slot_io_read(0x03, 0);
        assert_eq!(v, 0x56, "Presence byte should be 'V'");
        assert_eq!(h, 0x48, "Presence byte should be 'H'");
    }

    #[test]
    fn test_shr_enable_disable() {
        let mut card = VidHdCard::new(7);
        assert!(!card.is_shr_enabled());
        // Enable SHR via control register
        card.slot_io_write(0x00, 0x01, 0);
        assert!(card.is_shr_enabled());
        // Status should reflect SHR enabled
        let status = card.slot_io_read(0x00, 0);
        assert_ne!(status & SHR_STATUS_SHR_EN, 0);
        // Disable
        card.slot_io_write(0x00, 0x00, 0);
        assert!(!card.is_shr_enabled());
    }

    #[test]
    fn test_new_video_register() {
        let mut card = VidHdCard::new(7);
        // Write NEWVIDEO with SHR bit set
        card.slot_io_write(0x01, 0x80, 0);
        assert!(card.is_shr_enabled());
        assert_eq!(card.slot_io_read(0x01, 0), 0x80);
        // Clear SHR
        card.slot_io_write(0x01, 0x00, 0);
        assert!(!card.is_shr_enabled());
    }

    #[test]
    fn test_vblank_toggles() {
        let mut card = VidHdCard::new(7);
        let initial = card.slot_io_read(0x00, 0) & SHR_STATUS_VBLANK;
        // Run update ticks
        for _ in 0..4 {
            card.update(0);
        }
        let after = card.slot_io_read(0x00, 0) & SHR_STATUS_VBLANK;
        assert_ne!(initial, after, "VBlank should toggle after update ticks");
    }

    #[test]
    fn test_reset() {
        let mut card = VidHdCard::new(7);
        card.slot_io_write(0x00, 0x01, 0); // enable SHR
        card.slot_io_write(0x01, 0x80, 0); // set NEWVIDEO
        card.reset(true);
        assert!(!card.is_shr_enabled());
        assert_eq!(card.slot_io_read(0x01, 0), 0x00);
    }
}
