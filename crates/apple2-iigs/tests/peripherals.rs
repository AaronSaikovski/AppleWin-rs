//! Integration tests for IIgs peripherals: memory, BRAM, ADB, SHR, Ensoniq, SmartPort.

use apple2_iigs::bram;
use apple2_iigs::memory::{IIgsMemory, IIgsRomVersion};
use apple2_iigs::shadowing::ShadowReg;

// ── Memory tests ────────────────────────────────────────────────────────────

#[test]
fn memory_creation_with_valid_rom() {
    let rom = vec![0xEA; 131072]; // 128KB ROM (ROM 01 size)
    let mem = IIgsMemory::new(1024, rom).unwrap();
    assert_eq!(mem.ram_size, 1024 * 1024);
    assert!(matches!(
        mem.rom_version,
        IIgsRomVersion::Rom00 | IIgsRomVersion::Rom01
    ));
}

#[test]
fn memory_creation_rom03() {
    let rom = vec![0xEA; 262144]; // 256KB ROM (ROM 03 size)
    let mem = IIgsMemory::new(1024, rom).unwrap();
    assert_eq!(mem.rom_version, IIgsRomVersion::Rom03);
}

#[test]
fn memory_creation_invalid_rom_size() {
    let rom = vec![0xEA; 12345]; // wrong size
    let result = IIgsMemory::new(1024, rom);
    assert!(result.is_err());
}

#[test]
fn memory_ram_read_write() {
    let rom = vec![0xEA; 131072];
    let mut mem = IIgsMemory::new(256, rom).unwrap();
    mem.ram_write(0, 0x1234, 0x42);
    assert_eq!(mem.ram_read(0, 0x1234), 0x42);
}

#[test]
fn memory_fast_ram_read_write() {
    let rom = vec![0xEA; 131072];
    let mut mem = IIgsMemory::new(256, rom).unwrap();
    mem.fast_ram_write(0, 0x5000, 0xAB);
    assert_eq!(mem.fast_ram_read(0, 0x5000), 0xAB);
}

#[test]
fn memory_rom_read() {
    let mut rom = vec![0x00; 131072];
    rom[0x1FFFC] = 0x62; // Reset vector low at bank $FF offset $FFFC
    rom[0x1FFFD] = 0xFA; // Reset vector high
    let mem = IIgsMemory::new(256, rom).unwrap();
    assert_eq!(mem.rom_read(0xFF, 0xFFFC), 0x62);
    assert_eq!(mem.rom_read(0xFF, 0xFFFD), 0xFA);
}

#[test]
fn memory_rom_read_rom03_banks() {
    let mut rom = vec![0x00; 262144]; // 256KB
    rom[0] = 0xAA; // First byte of bank $FC
    rom[0x10000] = 0xBB; // First byte of bank $FD
    rom[0x20000] = 0xCC; // First byte of bank $FE
    rom[0x30000] = 0xDD; // First byte of bank $FF
    let mem = IIgsMemory::new(256, rom).unwrap();
    assert_eq!(mem.rom_read(0xFC, 0x0000), 0xAA);
    assert_eq!(mem.rom_read(0xFD, 0x0000), 0xBB);
    assert_eq!(mem.rom_read(0xFE, 0x0000), 0xCC);
    assert_eq!(mem.rom_read(0xFF, 0x0000), 0xDD);
}

#[test]
fn memory_ram_clamp() {
    // Minimum RAM is 256KB
    let rom = vec![0xEA; 131072];
    let mem = IIgsMemory::new(64, rom).unwrap(); // requested 64KB, clamped to 256KB
    assert_eq!(mem.ram_size, 256 * 1024);
}

#[test]
fn memory_bank_count() {
    let rom = vec![0xEA; 131072];
    let mem = IIgsMemory::new(1024, rom).unwrap(); // 1MB
    assert_eq!(mem.ram_banks(), 16); // 1MB / 64KB = 16 banks
}

// ── BRAM tests ──────────────────────────────────────────────────────────────

#[test]
fn bram_factory_default_has_valid_checksum() {
    let bram_data = bram::factory_default_bram();
    assert!(bram::validate_bram_checksum(&bram_data));
}

#[test]
fn bram_checksum_detects_corruption() {
    let mut bram_data = bram::factory_default_bram();
    bram_data[0x00] ^= 0xFF; // corrupt a byte
    assert!(!bram::validate_bram_checksum(&bram_data));
}

#[test]
fn bram_recompute_after_modification() {
    let mut bram_data = bram::factory_default_bram();
    bram_data[0x08] = 0x00; // change display setting
    bram::compute_bram_checksum(&mut bram_data);
    assert!(bram::validate_bram_checksum(&bram_data));
}

#[test]
fn bram_factory_defaults_are_reasonable() {
    let bram_data = bram::factory_default_bram();
    assert_eq!(bram_data[0x00] & 0x80, 0x80); // fast mode
    assert_eq!(bram_data[0x09], 0x00); // scan startup
    assert_eq!(bram_data[0x0C], 0x00); // English
}

// ── Shadow register tests ───────────────────────────────────────────────────

#[test]
fn shadow_default_all_enabled() {
    let shadow = ShadowReg::default();
    // Default = all bits clear = all shadowing enabled
    assert!(shadow.should_shadow_bank0(0x0400)); // text page
    assert!(shadow.should_shadow_bank0(0x2000)); // hires page 1
    assert!(shadow.should_shadow_bank0(0x4000)); // hires page 2
    assert!(shadow.should_shadow_bank1(0x2000)); // SHR
}

#[test]
fn shadow_inhibit_text() {
    let shadow = ShadowReg::INHIBIT_TEXT;
    assert!(!shadow.should_shadow_bank0(0x0400)); // text inhibited
    assert!(shadow.should_shadow_bank0(0x2000)); // hires still enabled
}

#[test]
fn shadow_inhibit_shr() {
    let shadow = ShadowReg::INHIBIT_SHR;
    assert!(!shadow.should_shadow_bank1(0x2000)); // SHR inhibited
    assert!(shadow.should_shadow_bank1(0x0400)); // text still enabled in bank 1
}

#[test]
fn shadow_non_display_areas_not_shadowed() {
    let shadow = ShadowReg::default();
    assert!(!shadow.should_shadow_bank0(0x0200)); // not a display area
    assert!(!shadow.should_shadow_bank0(0x1000)); // not a display area
    assert!(!shadow.should_shadow_bank1(0xA000)); // above SHR range
}

// ── ADB tests ───────────────────────────────────────────────────────────────

#[test]
fn adb_key_press_queues_data() {
    let mut adb = apple2_iigs::adb::Adb::default();
    adb.key_press(b'A');
    assert!(adb.status & apple2_iigs::adb::status::KEY_DATA != 0);
    assert!(adb.status & apple2_iigs::adb::status::KEY_IRQ != 0);
}

#[test]
fn adb_command_sets_busy_then_completes() {
    let mut adb = apple2_iigs::adb::Adb::default();
    // Send Sync command (0x07)
    adb.write_command(0x07, 1000);
    assert!(adb.status & apple2_iigs::adb::status::CMD_FULL != 0);
    // Update at a later cycle — command should complete
    adb.update(2000);
    assert!(adb.status & apple2_iigs::adb::status::CMD_FULL == 0);
    assert!(adb.status & apple2_iigs::adb::status::CMD_IRQ != 0);
}

#[test]
fn adb_mouse_state() {
    let mut adb = apple2_iigs::adb::Adb::default();
    adb.set_mouse_state(10, -5, true);
    assert!(adb.status & apple2_iigs::adb::status::MOUSE_DATA != 0);
}

// ── SHR rendering tests ────────────────────────────────────────────────────

#[test]
fn shr_render_320_mode_black_screen() {
    // Create a bank $E1 with all zeros (black screen in 320 mode)
    let fast_ram_e1 = vec![0u8; 0x10000];
    let mut pixels = vec![0u32; 640 * 400];
    apple2_iigs::shr::render_shr(&fast_ram_e1, &mut pixels);
    // Palette entry 0 with data 0x0000 = black (ABGR)
    // All pixels should be the same (palette 0, color 0)
    assert!(pixels.iter().all(|&p| p == pixels[0]));
}

#[test]
fn shr_render_with_palette() {
    let mut fast_ram_e1 = vec![0u8; 0x10000];
    // Set palette 0, entry 1 to white ($0FFF = R=F, G=F, B=F)
    let pal_offset = 0x9E00 + 2; // entry 1 = offset 2
    fast_ram_e1[pal_offset] = 0xFF; // lo byte: B=F, G=F
    fast_ram_e1[pal_offset + 1] = 0x0F; // hi byte: R=F
    // Set SCB for line 0: 320 mode, palette 0
    fast_ram_e1[0x9D00] = 0x00;
    // Set first pixel byte to use color 1 (high nibble = 0, low nibble = 1)
    fast_ram_e1[0x2000] = 0x01;

    let mut pixels = vec![0u32; 640 * 400];
    apple2_iigs::shr::render_shr(&fast_ram_e1, &mut pixels);

    // In 320 mode, byte 0x01 = left pixel color 0, right pixel color 1
    // Right pixel (color 1) should be white
    // Pixels 2 and 3 (doubled from right pixel)
    let white_abgr = 0xFF_FF_FF_FF_u32; // ABGR: A=FF, B=FF, G=FF, R=FF
    assert_eq!(pixels[2], white_abgr);
    assert_eq!(pixels[3], white_abgr);
}

#[test]
fn shr_640_mode_flag() {
    let mut fast_ram_e1 = vec![0u8; 0x10000];
    // Set SCB for line 0: 640 mode (bit 7 set)
    fast_ram_e1[0x9D00] = 0x80;
    let mut pixels = vec![0u32; 640 * 400];
    apple2_iigs::shr::render_shr(&fast_ram_e1, &mut pixels);
    // Should render without crashing in 640 mode
}

// ── Ensoniq tests ───────────────────────────────────────────────────────────

#[test]
fn ensoniq_default_all_halted() {
    let doc = apple2_iigs::ensoniq::Ensoniq::default();
    // All oscillators should be halted by default
    for i in 0..32 {
        assert_eq!(doc.regs[0xA0 + i] & 0x01, 0x01); // HALT bit set
    }
}

#[test]
fn ensoniq_register_write_read() {
    let mut doc = apple2_iigs::ensoniq::Ensoniq::default();
    doc.control = 0x40; // auto-increment, DOC registers
    doc.address = 0x00; // frequency low, osc 0
    doc.write_data(0x42);
    assert_eq!(doc.regs[0x00], 0x42);
    assert_eq!(doc.address, 0x01); // auto-incremented
}

#[test]
fn ensoniq_sound_ram_access() {
    let mut doc = apple2_iigs::ensoniq::Ensoniq::default();
    doc.control = 0xC0; // auto-increment + sound RAM mode
    doc.address = 0x1234;
    doc.write_data(0xAB);
    assert_eq!(doc.sound_ram[0x1234], 0xAB);

    // Read it back
    doc.address = 0x1234;
    let val = doc.read_data();
    assert_eq!(val, 0xAB);
}

#[test]
fn ensoniq_fill_audio_silent_when_halted() {
    let mut doc = apple2_iigs::ensoniq::Ensoniq::default();
    let mut out = vec![0.0f32; 100];
    doc.fill_audio(&mut out, 44100, 1000);
    // All oscillators halted → silence
    assert!(out.iter().all(|&s| s == 0.0));
}

// ── SmartPort tests ─────────────────────────────────────────────────────────

#[test]
fn smartport_insert_and_read() {
    let mut sp = apple2_iigs::smartport::SmartPort::default();
    let data = vec![0xAA; 512 * 10]; // 10 blocks
    let disk = apple2_iigs::smartport::SmartPortDisk::from_raw(data, Some("test.po".to_string()));
    assert_eq!(disk.num_blocks, 10);

    sp.insert(0, disk);
    assert!(sp.has_disk(0));
    assert_eq!(sp.device_blocks(0), 10);

    let block = sp.read_block(0, 0).unwrap();
    assert_eq!(block.len(), 512);
    assert!(block.iter().all(|&b| b == 0xAA));
}

#[test]
fn smartport_write_block() {
    let mut sp = apple2_iigs::smartport::SmartPort::default();
    let data = vec![0x00; 512 * 5];
    let disk = apple2_iigs::smartport::SmartPortDisk::from_raw(data, None);
    sp.insert(0, disk);

    let write_data = vec![0xBB; 512];
    assert!(sp.write_block(0, 2, &write_data));

    let read_back = sp.read_block(0, 2).unwrap();
    assert!(read_back.iter().all(|&b| b == 0xBB));
}

#[test]
fn smartport_out_of_range() {
    let mut sp = apple2_iigs::smartport::SmartPort::default();
    let data = vec![0x00; 512 * 3];
    let disk = apple2_iigs::smartport::SmartPortDisk::from_raw(data, None);
    sp.insert(0, disk);

    assert!(sp.read_block(0, 5).is_none()); // block 5 doesn't exist
    assert!(!sp.write_block(0, 5, &[0; 512]));
}

#[test]
fn smartport_eject() {
    let mut sp = apple2_iigs::smartport::SmartPort::default();
    let data = vec![0x00; 512];
    let disk = apple2_iigs::smartport::SmartPortDisk::from_raw(data, None);
    sp.insert(0, disk);
    assert!(sp.has_disk(0));
    sp.eject(0);
    assert!(!sp.has_disk(0));
}

#[test]
fn smartport_2mg_format() {
    // Create a minimal 2IMG file
    let mut raw = vec![0u8; 64 + 512]; // 64-byte header + 1 block
    raw[0..4].copy_from_slice(b"2IMG"); // magic
    raw[8..12].copy_from_slice(&64u32.to_le_bytes()); // data offset
    raw[12..16].copy_from_slice(&512u32.to_le_bytes()); // data length
    raw[64] = 0xCC; // first byte of block data

    let disk = apple2_iigs::smartport::SmartPortDisk::from_2mg(&raw, None).unwrap();
    assert_eq!(disk.num_blocks, 1);
    assert_eq!(disk.data[0], 0xCC);
}
