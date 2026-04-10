//! Unit tests for the Apple II memory bus (soft switches, language card, page tables).

use crate::bus::{Bus, MemMode, PageDst, PageSrc};

/// Create a minimal Bus with a 16K zero-filled ROM.
fn make_bus() -> Bus {
    Bus::new(vec![0u8; 16384])
}

/// Write a byte through raw write (bypasses I/O side-effects).
#[allow(dead_code)]
fn poke(bus: &mut Bus, addr: u16, val: u8) {
    bus.write_raw(addr, val);
}

/// Read a byte through raw read (bypasses I/O side-effects).
#[allow(dead_code)]
fn peek(bus: &Bus, addr: u16) -> u8 {
    bus.read_raw(addr)
}

// ===========================================================================
// Keyboard data register
// ===========================================================================

#[test]
fn keyboard_data_read() {
    let mut bus = make_bus();
    bus.key_press(b'A'); // sets bit 7
    assert_eq!(bus.keyboard_data, b'A' | 0x80);

    // Reading $C000 returns keyboard data with strobe
    let val = bus.read(0xC000, 0);
    assert_eq!(val, b'A' | 0x80);
}

#[test]
fn keyboard_strobe_clear_on_read() {
    let mut bus = make_bus();
    bus.key_press(b'A');

    // Reading $C010 clears strobe (bit 7) and returns old value
    let old = bus.read(0xC010, 0);
    assert_eq!(old, b'A' | 0x80);
    // After clearing, keyboard_data bit 7 should be 0
    assert_eq!(bus.keyboard_data & 0x80, 0);
    assert_eq!(bus.keyboard_data, b'A');
}

#[test]
fn keyboard_strobe_clear_on_write() {
    let mut bus = make_bus();
    bus.key_press(b'B');
    assert_eq!(bus.keyboard_data & 0x80, 0x80);

    // Writing to $C010 also clears strobe
    bus.write(0xC010, 0x00, 0);
    assert_eq!(bus.keyboard_data & 0x80, 0);
}

// ===========================================================================
// 80STORE soft switch
// ===========================================================================

#[test]
fn soft_switch_80store() {
    let mut bus = make_bus();
    assert!(!bus.mode.contains(MemMode::MF_80STORE));

    // Write to $C001 enables 80STORE
    bus.write(0xC001, 0, 0);
    assert!(bus.mode.contains(MemMode::MF_80STORE));

    // Write to $C000 disables 80STORE
    bus.write(0xC000, 0, 0);
    assert!(!bus.mode.contains(MemMode::MF_80STORE));
}

// ===========================================================================
// AUXREAD / AUXWRITE soft switches
// ===========================================================================

#[test]
fn soft_switch_auxread() {
    let mut bus = make_bus();
    assert!(!bus.mode.contains(MemMode::MF_AUXREAD));

    // $C003 enables AUXREAD
    bus.write(0xC003, 0, 0);
    assert!(bus.mode.contains(MemMode::MF_AUXREAD));

    // Verify page tables route reads through aux RAM for $02-$BF range
    assert!(matches!(bus.pages_r[0x02], PageSrc::Aux(_)));
    assert!(matches!(bus.pages_r[0xBF], PageSrc::Aux(_)));

    // $C002 disables AUXREAD
    bus.write(0xC002, 0, 0);
    assert!(!bus.mode.contains(MemMode::MF_AUXREAD));
    assert!(matches!(bus.pages_r[0x02], PageSrc::Main(_)));
}

#[test]
fn soft_switch_auxwrite() {
    let mut bus = make_bus();

    // $C005 enables AUXWRITE
    bus.write(0xC005, 0, 0);
    assert!(bus.mode.contains(MemMode::MF_AUXWRITE));
    assert!(matches!(bus.pages_w[0x02], PageDst::Aux(_)));

    // $C004 disables AUXWRITE
    bus.write(0xC004, 0, 0);
    assert!(!bus.mode.contains(MemMode::MF_AUXWRITE));
    assert!(matches!(bus.pages_w[0x02], PageDst::Main(_)));
}

// ===========================================================================
// ALTZP soft switch
// ===========================================================================

#[test]
fn soft_switch_altzp() {
    let mut bus = make_bus();

    // $C009 enables ALTZP
    bus.write(0xC009, 0, 0);
    assert!(bus.mode.contains(MemMode::MF_ALTZP));
    // Zero page and stack should route through aux
    assert!(matches!(bus.pages_r[0x00], PageSrc::Aux(_)));
    assert!(matches!(bus.pages_r[0x01], PageSrc::Aux(_)));
    assert!(matches!(bus.pages_w[0x00], PageDst::Aux(_)));
    assert!(matches!(bus.pages_w[0x01], PageDst::Aux(_)));

    // $C008 disables ALTZP
    bus.write(0xC008, 0, 0);
    assert!(!bus.mode.contains(MemMode::MF_ALTZP));
    assert!(matches!(bus.pages_r[0x00], PageSrc::Main(_)));
}

// ===========================================================================
// PAGE2 soft switch
// ===========================================================================

#[test]
fn soft_switch_page2_write() {
    let mut bus = make_bus();
    assert!(!bus.mode.contains(MemMode::MF_PAGE2));

    // $C055 sets PAGE2
    bus.write(0xC055, 0, 0);
    assert!(bus.mode.contains(MemMode::MF_PAGE2));

    // $C054 clears PAGE2
    bus.write(0xC054, 0, 0);
    assert!(!bus.mode.contains(MemMode::MF_PAGE2));
}

#[test]
fn soft_switch_page2_read_strobe() {
    let mut bus = make_bus();
    // Reading $C055 also acts as strobe to set PAGE2
    bus.read(0xC055, 0);
    assert!(bus.mode.contains(MemMode::MF_PAGE2));

    // Reading $C054 clears PAGE2
    bus.read(0xC054, 0);
    assert!(!bus.mode.contains(MemMode::MF_PAGE2));
}

// ===========================================================================
// HIRES soft switch
// ===========================================================================

#[test]
fn soft_switch_hires() {
    let mut bus = make_bus();
    assert!(!bus.mode.contains(MemMode::MF_HIRES));

    // $C057 sets HIRES
    bus.write(0xC057, 0, 0);
    assert!(bus.mode.contains(MemMode::MF_HIRES));

    // $C056 clears HIRES
    bus.write(0xC056, 0, 0);
    assert!(!bus.mode.contains(MemMode::MF_HIRES));
}

// ===========================================================================
// Language card banking ($C080-$C08F)
// ===========================================================================

#[test]
fn lc_c080_read_ram_bank2_write_protect() {
    let mut bus = make_bus();
    // $C080: read RAM, write-protect, bank 2
    bus.read(0xC080, 0);
    assert!(bus.mode.contains(MemMode::MF_HIGHRAM));
    assert!(!bus.mode.contains(MemMode::MF_WRITERAM));
    assert!(bus.mode.contains(MemMode::MF_BANK2));
}

#[test]
fn lc_c081_read_rom_write_enable_bank2() {
    let mut bus = make_bus();
    // $C081: read ROM, write-enable, bank 2
    // WRITERAM requires two consecutive reads of the same odd address
    bus.read(0xC081, 0);
    bus.read(0xC081, 0);
    assert!(!bus.mode.contains(MemMode::MF_HIGHRAM));
    assert!(bus.mode.contains(MemMode::MF_WRITERAM));
    assert!(bus.mode.contains(MemMode::MF_BANK2));
}

#[test]
fn lc_c082_read_rom_write_protect_bank2() {
    let mut bus = make_bus();
    // $C082: read ROM, write-protect, bank 2
    bus.read(0xC082, 0);
    assert!(!bus.mode.contains(MemMode::MF_HIGHRAM));
    assert!(!bus.mode.contains(MemMode::MF_WRITERAM));
    assert!(bus.mode.contains(MemMode::MF_BANK2));
}

#[test]
fn lc_c083_read_ram_write_enable_bank2() {
    let mut bus = make_bus();
    // $C083: read RAM, write-enable, bank 2
    // WRITERAM requires two consecutive reads of the same odd address
    bus.read(0xC083, 0);
    bus.read(0xC083, 0);
    assert!(bus.mode.contains(MemMode::MF_HIGHRAM));
    assert!(bus.mode.contains(MemMode::MF_WRITERAM));
    assert!(bus.mode.contains(MemMode::MF_BANK2));
}

#[test]
fn lc_c088_bank1() {
    let mut bus = make_bus();
    // $C088: read RAM, write-protect, bank 1 (bit 3 set -> bank1)
    bus.read(0xC088, 0);
    assert!(bus.mode.contains(MemMode::MF_HIGHRAM));
    assert!(!bus.mode.contains(MemMode::MF_WRITERAM));
    assert!(!bus.mode.contains(MemMode::MF_BANK2)); // bank 1
}

#[test]
fn lc_c08b_bank1_readwrite() {
    let mut bus = make_bus();
    // $C08B: read RAM, write-enable, bank 1
    // WRITERAM requires two consecutive reads of the same odd address
    bus.read(0xC08B, 0);
    bus.read(0xC08B, 0);
    assert!(bus.mode.contains(MemMode::MF_HIGHRAM));
    assert!(bus.mode.contains(MemMode::MF_WRITERAM));
    assert!(!bus.mode.contains(MemMode::MF_BANK2));
}

// ===========================================================================
// Page table rebuilding after soft-switch changes
// ===========================================================================

#[test]
fn page_table_default_state() {
    let bus = make_bus();
    // Default: all reads from main RAM, writes to main RAM
    assert!(matches!(bus.pages_r[0x00], PageSrc::Main(_)));
    assert!(matches!(bus.pages_r[0x50], PageSrc::Main(_)));
    assert!(matches!(bus.pages_w[0x50], PageDst::Main(_)));
    // I/O page
    assert!(matches!(bus.pages_r[0xC0], PageSrc::Io));
    // ROM pages (default: HIGHRAM off, so $D0-$FF read from ROM)
    assert!(matches!(bus.pages_r[0xD0], PageSrc::Rom(_)));
    assert!(matches!(bus.pages_r[0xFF], PageSrc::Rom(_)));
}

#[test]
fn page_table_highram_on() {
    let mut bus = make_bus();
    // Enable LC RAM reading
    bus.read(0xC080, 0); // HIGHRAM on, bank 2
    // $D0-$FF should now read from aux RAM (language card)
    assert!(matches!(bus.pages_r[0xD0], PageSrc::Aux(_)));
    assert!(matches!(bus.pages_r[0xE0], PageSrc::Aux(_)));
    assert!(matches!(bus.pages_r[0xFF], PageSrc::Aux(_)));
}

#[test]
fn page_table_writeram_on() {
    let mut bus = make_bus();
    // Enable LC RAM writing (read ROM)
    // WRITERAM requires two consecutive reads of the same odd address
    bus.read(0xC081, 0);
    bus.read(0xC081, 0); // HIGHRAM off, WRITERAM on, bank 2
    // $D0-$FF should read ROM but write to aux
    assert!(matches!(bus.pages_r[0xD0], PageSrc::Rom(_)));
    assert!(matches!(bus.pages_w[0xD0], PageDst::Aux(_)));
}

#[test]
fn page_table_auxread_preserves_zp() {
    let mut bus = make_bus();
    // AUXREAD only affects $02-$BF, not zero page
    bus.write(0xC003, 0, 0);
    assert!(matches!(bus.pages_r[0x00], PageSrc::Main(_))); // ZP still main
    assert!(matches!(bus.pages_r[0x01], PageSrc::Main(_))); // Stack still main
    assert!(matches!(bus.pages_r[0x02], PageSrc::Aux(_))); // $0200+ aux
}

// ===========================================================================
// Actual memory read/write through aux RAM
// ===========================================================================

#[test]
fn write_and_read_main_ram() {
    let mut bus = make_bus();
    bus.write(0x0300, 0x42, 0);
    assert_eq!(bus.read(0x0300, 0), 0x42);
    assert_eq!(bus.main_ram[0x0300], 0x42);
}

#[test]
fn write_and_read_aux_ram() {
    let mut bus = make_bus();
    // Enable aux write + aux read
    bus.write(0xC005, 0, 0); // AUXWRITE on
    bus.write(0xC003, 0, 0); // AUXREAD on

    bus.write(0x0300, 0x99, 0);
    assert_eq!(bus.read(0x0300, 0), 0x99);
    assert_eq!(bus.aux_ram[0x0300], 0x99);
    // Main RAM should be untouched
    assert_eq!(bus.main_ram[0x0300], 0x00);
}

#[test]
fn altzp_routes_zero_page_to_aux() {
    let mut bus = make_bus();
    bus.write(0xC009, 0, 0); // ALTZP on

    bus.write(0x0010, 0xAA, 0);
    assert_eq!(bus.aux_ram[0x0010], 0xAA);
    assert_eq!(bus.main_ram[0x0010], 0x00);

    assert_eq!(bus.read(0x0010, 0), 0xAA);
}

// ===========================================================================
// Soft-switch status reads ($C011-$C01F)
// ===========================================================================

#[test]
fn soft_switch_status_reads() {
    let mut bus = make_bus();

    // Power-on default: MF_BANK2 | MF_WRITERAM are set (Apple IIe initial state).
    // BANK2 is active, so $C011 bit 7 is set; others are clear.
    assert_eq!(bus.read(0xC011, 0) & 0x80, 0x80); // BANK2 (on at power-up)
    assert_eq!(bus.read(0xC012, 0) & 0x80, 0x00); // HIGHRAM
    assert_eq!(bus.read(0xC013, 0) & 0x80, 0x00); // AUXREAD
    assert_eq!(bus.read(0xC018, 0) & 0x80, 0x00); // 80STORE

    // Enable some switches
    bus.write(0xC001, 0, 0); // 80STORE on
    assert_eq!(bus.read(0xC018, 0) & 0x80, 0x80);

    bus.write(0xC003, 0, 0); // AUXREAD on
    assert_eq!(bus.read(0xC013, 0) & 0x80, 0x80);
}

// ===========================================================================
// Speaker toggle
// ===========================================================================

#[test]
fn speaker_toggle_on_c030() {
    let mut bus = make_bus();
    assert!(!bus.speaker_state);

    bus.read(0xC030, 100);
    assert!(bus.speaker_state);
    assert_eq!(bus.speaker_toggles.len(), 1);
    assert_eq!(bus.speaker_toggles[0], 100);

    bus.read(0xC030, 200);
    assert!(!bus.speaker_state);
    assert_eq!(bus.speaker_toggles.len(), 2);
}

#[test]
fn speaker_toggle_on_write_c030() {
    let mut bus = make_bus();
    bus.write(0xC030, 0, 50);
    assert!(bus.speaker_state);
    assert_eq!(bus.speaker_toggles[0], 50);
}

// ===========================================================================
// Video mode switches
// ===========================================================================

#[test]
fn graphics_mixed_mode_switches() {
    let mut bus = make_bus();

    // $C050 sets GRAPHICS
    bus.write(0xC050, 0, 0);
    assert!(bus.mode.contains(MemMode::MF_GRAPHICS));

    // $C051 clears GRAPHICS
    bus.write(0xC051, 0, 0);
    assert!(!bus.mode.contains(MemMode::MF_GRAPHICS));

    // $C053 sets MIXED
    bus.write(0xC053, 0, 0);
    assert!(bus.mode.contains(MemMode::MF_MIXED));

    // $C052 clears MIXED
    bus.write(0xC052, 0, 0);
    assert!(!bus.mode.contains(MemMode::MF_MIXED));
}

// ===========================================================================
// DHIRES
// ===========================================================================

#[test]
fn dhires_switch() {
    let mut bus = make_bus();
    bus.write(0xC05E, 0, 0);
    assert!(bus.mode.contains(MemMode::MF_DHIRES));
    bus.write(0xC05F, 0, 0);
    assert!(!bus.mode.contains(MemMode::MF_DHIRES));
}

// ===========================================================================
// Raw read/write bypass I/O
// ===========================================================================

#[test]
fn raw_read_write_bypass_io() {
    let mut bus = make_bus();
    bus.main_ram[0x0500] = 0xDE;
    assert_eq!(bus.read_raw(0x0500), 0xDE);

    bus.write_raw(0x0500, 0xAD);
    assert_eq!(bus.main_ram[0x0500], 0xAD);
}
