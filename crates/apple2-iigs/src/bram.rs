//! Apple IIgs battery-backed parameter RAM (BRAM).
//!
//! 256 bytes of non-volatile storage accessible through ADB commands.
//! Contains system configuration, display settings, slot assignments, etc.
//! The ROM validates the checksum at bytes $FC-$FF during boot; if invalid,
//! it resets BRAM to factory defaults.

/// Generate factory-default BRAM contents.
///
/// These values match the IIgs factory defaults that the ROM expects.
/// Critical settings include display mode, slot assignments, and the
/// checksum at bytes $FC-$FF.
pub fn factory_default_bram() -> [u8; 256] {
    let mut bram = [0u8; 256];

    // System speed: bit 7 = fast (2.8 MHz)
    bram[0x00] = 0x80;

    // Slot assignments ($01-$07):
    // $00 = your card, $01 = ROM (built-in firmware)
    // Default: slots 1-6 = your card, slot 7 = AppleTalk (built-in)
    bram[0x01] = 0x00; // slot 1: your card
    bram[0x02] = 0x00; // slot 2: your card
    bram[0x03] = 0x00; // slot 3: your card
    bram[0x04] = 0x00; // slot 4: your card
    bram[0x05] = 0x00; // slot 5: your card
    bram[0x06] = 0x00; // slot 6: your card
    bram[0x07] = 0x01; // slot 7: built-in (AppleTalk)

    // Display settings ($08):
    // Bit 7: 0 = color, 1 = monochrome
    // Bit 6: 0 = 40 col, 1 = 80 col
    // Bit 5-4: display type (00 = color monitor)
    // Bit 3: 0 = DHGR color, 1 = DHGR mono
    bram[0x08] = 0x40; // 80-column color

    // Startup device ($09):
    // $00 = scan slots, $01-$07 = specific slot
    bram[0x09] = 0x00; // scan

    // Text color ($0A): foreground/background
    bram[0x0A] = 0xF0; // white on black

    // Background color ($0B)
    bram[0x0B] = 0x00;

    // Language ($0C): 0 = English
    bram[0x0C] = 0x00;

    // Keyboard layout ($0D): 0 = US
    bram[0x0D] = 0x00;

    // Repeat rate ($0E): moderate
    bram[0x0E] = 0x03;

    // Repeat delay ($0F): moderate
    bram[0x0F] = 0x03;

    // Double-click speed ($10)
    bram[0x10] = 0x03;

    // Flash rate ($11)
    bram[0x11] = 0x03;

    // Mouse tracking ($12)
    bram[0x12] = 0x02;

    // Startup/boot ($13)
    bram[0x13] = 0x00;

    // Sound volume ($14): max
    bram[0x14] = 0x07;

    // Miscellaneous system flags ($15-$1F)
    bram[0x15] = 0x00;

    // Date/time format ($20-$27) - not critical for boot

    // Printer settings ($28-$2F) - not critical for boot

    // Modem settings ($30-$37) - not critical for boot

    // RAM disk settings ($38-$3F)
    bram[0x38] = 0x00; // no RAM disk

    // Rest stays zero

    // Compute and store the checksum at $FC-$FF.
    // The ROM validates this during boot.
    compute_bram_checksum(&mut bram);

    bram
}

/// Compute the BRAM checksum and store it at bytes $FC-$FF.
///
/// The checksum algorithm:
/// - Sum all bytes $00-$FB (skipping nothing per the simpler algorithm)
/// - Store 16-bit sum at $FC-$FD (little-endian)
/// - Store sum XOR $AAAA at $FE-$FF (little-endian)
pub fn compute_bram_checksum(bram: &mut [u8; 256]) {
    let mut sum: u16 = 0;
    for &byte in bram.iter().take(0xFC) {
        sum = sum.wrapping_add(byte as u16);
        sum = sum.rotate_left(1);
    }

    bram[0xFC] = sum as u8;
    bram[0xFD] = (sum >> 8) as u8;

    let check = sum ^ 0xAAAA;
    bram[0xFE] = check as u8;
    bram[0xFF] = (check >> 8) as u8;
}

/// Validate the BRAM checksum. Returns true if valid.
pub fn validate_bram_checksum(bram: &[u8; 256]) -> bool {
    let mut sum: u16 = 0;
    for &byte in bram.iter().take(0xFC) {
        sum = sum.wrapping_add(byte as u16);
        sum = sum.rotate_left(1);
    }

    let stored_sum = (bram[0xFC] as u16) | ((bram[0xFD] as u16) << 8);
    let stored_check = (bram[0xFE] as u16) | ((bram[0xFF] as u16) << 8);

    sum == stored_sum && (sum ^ 0xAAAA) == stored_check
}
