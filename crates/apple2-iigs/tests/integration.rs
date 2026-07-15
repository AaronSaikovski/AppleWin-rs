//! Integration tests for the Apple IIgs emulator.

use apple2_iigs::bus::IIgsBus;
use apple2_iigs::cpu65816::{self, Bus816, Cpu65816};
use apple2_iigs::memory::IIgsMemory;

/// Build a minimal ROM image with a reset vector and code at a given address.
fn build_test_rom(entry_bank: u8, entry_addr: u16, code: &[u8]) -> Vec<u8> {
    // Create a 128KB ROM (ROM 01 size), mapped to banks $FE-$FF
    let mut rom = vec![0xEA; 131072]; // fill with NOP

    // Reset vector at bank $FF, offset $FFFC (= ROM offset $1FFFC)
    let vec_offset = 0x1FFFC;
    rom[vec_offset] = entry_addr as u8;
    rom[vec_offset + 1] = (entry_addr >> 8) as u8;

    // Place code in bank $FF at the entry address
    // ROM bank $FF = second 64KB of the 128KB ROM, offset 0x10000
    if entry_bank == 0xFF {
        let code_offset = 0x10000 + entry_addr as usize;
        for (i, &b) in code.iter().enumerate() {
            if code_offset + i < rom.len() {
                rom[code_offset + i] = b;
            }
        }
    }

    rom
}

/// Build a ROM with code at bank $00 address (placed in RAM, not ROM).
fn setup_ram_program(bus: &mut IIgsBus, addr: u16, code: &[u8]) {
    for (i, &b) in code.iter().enumerate() {
        bus.write((addr as u32) + i as u32, b, 0);
    }
}

#[test]
fn emulator_boots_from_rom_reset_vector() {
    let rom = build_test_rom(0xFF, 0xFA00, &[0xEA]); // NOP at $FA00
    let mem = IIgsMemory::new(256, rom).unwrap();
    let mut bus = IIgsBus::new(mem);
    let mut cpu = Cpu65816::new();
    cpu.reset(&mut bus);

    assert_eq!(cpu.pc, 0xFA00);
    assert_eq!(cpu.pbr, 0x00); // reset always enters bank 0
    assert!(cpu.emulation); // starts in emulation mode
    assert!(cpu.flags.contains(apple2_iigs::cpu65816::Flags816::I));
}

#[test]
fn execute_nop_advances_pc() {
    let rom = build_test_rom(0xFF, 0xFA00, &[0xEA, 0xEA, 0xEA]);
    let mem = IIgsMemory::new(256, rom).unwrap();
    let mut bus = IIgsBus::new(mem);
    let mut cpu = Cpu65816::new();
    cpu.reset(&mut bus);

    let pc_start = cpu.pc;
    cpu65816::step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, pc_start + 1);
}

#[test]
fn ram_program_execution() {
    let rom = build_test_rom(0xFF, 0x0200, &[]); // reset to $0200 (in RAM)
    let mem = IIgsMemory::new(256, rom).unwrap();
    let mut bus = IIgsBus::new(mem);
    let mut cpu = Cpu65816::new();

    // Place a program in RAM at $0200
    #[rustfmt::skip]
    setup_ram_program(&mut bus, 0x0200, &[
        0xA9, 0x42,       // LDA #$42
        0x85, 0x50,       // STA $50
        0xDB,             // STP
    ]);

    cpu.reset(&mut bus);
    assert_eq!(cpu.pc, 0x0200);

    // Execute instructions
    cpu65816::step(&mut cpu, &mut bus); // LDA #$42
    assert_eq!(cpu.c & 0xFF, 0x42);
    cpu65816::step(&mut cpu, &mut bus); // STA $50
    assert_eq!(bus.read(0x50, 0), 0x42);
    cpu65816::step(&mut cpu, &mut bus); // STP
    assert!(cpu.stopped);
}

#[test]
fn subroutine_call_and_return() {
    let rom = build_test_rom(0xFF, 0x0200, &[]);
    let mem = IIgsMemory::new(256, rom).unwrap();
    let mut bus = IIgsBus::new(mem);
    let mut cpu = Cpu65816::new();

    #[rustfmt::skip]
    setup_ram_program(&mut bus, 0x0200, &[
        0x20, 0x00, 0x03, // JSR $0300
        0xDB,             // STP (reached after RTS)
    ]);
    #[rustfmt::skip]
    setup_ram_program(&mut bus, 0x0300, &[
        0xA9, 0xAA,       // LDA #$AA
        0x60,             // RTS
    ]);

    cpu.reset(&mut bus);

    // Run until stopped
    for _ in 0..10 {
        if cpu.stopped {
            break;
        }
        cpu65816::step(&mut cpu, &mut bus);
    }

    assert!(cpu.stopped);
    assert_eq!(cpu.c & 0xFF, 0xAA); // subroutine loaded $AA
    assert_eq!(cpu.pc, 0x0204); // STP is at $0203, stopped after
}

#[test]
fn native_mode_16bit_arithmetic() {
    let rom = build_test_rom(0xFF, 0x0200, &[]);
    let mem = IIgsMemory::new(256, rom).unwrap();
    let mut bus = IIgsBus::new(mem);
    let mut cpu = Cpu65816::new();

    #[rustfmt::skip]
    setup_ram_program(&mut bus, 0x0200, &[
        0x18,             // CLC
        0xFB,             // XCE (enter native mode — C now = old E = 1)
        0xC2, 0x30,       // REP #$30 (16-bit A and X/Y)
        0x18,             // CLC (clear carry before ADC)
        0xA9, 0x00, 0x10, // LDA #$1000
        0x69, 0x34, 0x12, // ADC #$1234
        0xDB,             // STP
    ]);

    cpu.reset(&mut bus);

    for _ in 0..20 {
        if cpu.stopped {
            break;
        }
        cpu65816::step(&mut cpu, &mut bus);
    }

    assert!(cpu.stopped);
    assert!(!cpu.emulation);
    assert_eq!(cpu.c, 0x2234); // $1000 + $1234 = $2234
}

#[test]
fn bus_bank_00_ram_access() {
    let rom = vec![0xEA; 131072];
    let mem = IIgsMemory::new(256, rom).unwrap();
    let mut bus = IIgsBus::new(mem);

    // Write to bank $00
    bus.write(0x00_1234, 0x42, 0);
    assert_eq!(bus.read(0x00_1234, 0), 0x42);
}

#[test]
fn bus_fast_ram_shadowing() {
    let rom = vec![0xEA; 131072];
    let mem = IIgsMemory::new(256, rom).unwrap();
    let mut bus = IIgsBus::new(mem);

    // Default: text page shadowing enabled
    // Write to bank $00 text page ($0400)
    bus.write(0x00_0400, 0xAB, 0);

    // Should be shadowed to bank $E0
    assert_eq!(bus.read(0xE0_0400, 0), 0xAB);
}

#[test]
fn bus_rom_read() {
    let mut rom = vec![0x00; 131072];
    rom[0x10000] = 0xBB; // bank $FF offset $0000
    let mem = IIgsMemory::new(256, rom).unwrap();
    let bus = IIgsBus::new(mem);

    assert_eq!(bus.read_raw(0xFF_0000), 0xBB);
}

#[test]
fn bus_expansion_ram() {
    let rom = vec![0xEA; 131072];
    let mem = IIgsMemory::new(1024, rom).unwrap(); // 1MB = 16 banks
    let mut bus = IIgsBus::new(mem);

    // Write to bank $05
    bus.write(0x05_ABCD, 0x77, 0);
    assert_eq!(bus.read(0x05_ABCD, 0), 0x77);
}

#[test]
fn bus_banks_80_df_not_populated() {
    // A 1 MB machine has RAM in banks $00-$0F. Banks $80-$DF are NOT RAM (and
    // must NOT alias $00-$5F — that would make GS/OS mis-size memory and corrupt
    // bank $00). Writes there are discarded; reads return 0.
    let rom = vec![0xEA; 131072];
    let mem = IIgsMemory::new(1024, rom).unwrap();
    let mut bus = IIgsBus::new(mem);

    bus.write(0x02_5678, 0xCC, 0);
    // Bank $82 must NOT reflect bank $02 (no mirror).
    assert_eq!(bus.read(0x82_5678, 0), 0x00);
    // Writes to $80-$DF must not leak into $00-$5F.
    bus.write(0x84_1234, 0x99, 0);
    assert_eq!(bus.read(0x04_1234, 0), 0x00);
    assert_eq!(bus.read(0x84_1234, 0), 0x00);
}

#[test]
fn mega2_keyboard_press() {
    let rom = vec![0xEA; 131072];
    let mem = IIgsMemory::new(256, rom).unwrap();
    let mut bus = IIgsBus::new(mem);

    bus.mega2.key_press(b'X');
    assert!(bus.mega2.key_strobe);
    assert_eq!(bus.mega2.keyboard_data, b'X');

    // Read $C000 should return key data with strobe
    let val = bus.read(0x00_C000, 0);
    assert_eq!(val, b'X' | 0x80);
}

#[test]
fn mega2_soft_switch_80store() {
    let rom = vec![0xEA; 131072];
    let mem = IIgsMemory::new(256, rom).unwrap();
    let mut bus = IIgsBus::new(mem);

    // Enable 80STORE
    bus.write(0x00_C001, 0, 0);
    assert!(
        bus.mega2
            .mem_mode
            .contains(apple2_core::bus::MemMode::MF_80STORE)
    );

    // Disable 80STORE
    bus.write(0x00_C000, 0, 0);
    assert!(
        !bus.mega2
            .mem_mode
            .contains(apple2_core::bus::MemMode::MF_80STORE)
    );
}

#[test]
fn iigs_emulator_new_and_reset() {
    let rom = vec![0xEA; 131072];
    let emu = apple2_iigs::emulator::IIgsEmulator::new(1024, rom).unwrap();
    assert!(emu.cpu.emulation);
    assert_eq!(emu.cpu.pbr, 0);
    // PC should be set from ROM reset vector
    assert!(emu.cpu.pc != 0);
}

#[test]
fn iigs_emulator_execute_cycles() {
    let mut rom = vec![0xEA; 131072]; // NOP-filled
    // Set reset vector to $FA00 (in ROM)
    rom[0x1FFFC] = 0x00;
    rom[0x1FFFD] = 0xFA;
    // Put a STP at $FA00 so it stops quickly
    rom[0x1FA00] = 0xDB; // STP

    let mut emu = apple2_iigs::emulator::IIgsEmulator::new(256, rom).unwrap();
    let executed = emu.execute(100);
    assert!(executed > 0);
    assert!(emu.cpu.stopped);
}

#[test]
fn iigs_emulator_key_press() {
    let rom = vec![0xEA; 131072];
    let mut emu = apple2_iigs::emulator::IIgsEmulator::new(256, rom).unwrap();
    emu.key_press(b'Z');
    assert!(emu.bus.mega2.key_strobe);
    assert_eq!(emu.bus.mega2.keyboard_data, b'Z');
}

/// A 128KB ROM dump whose two 64KB halves are swapped (vectors in the low half,
/// zeros where bank $FF's vectors should be) must be normalized on load so the
/// reset vector at $00/FFFC resolves correctly. Regression for the ROM 01
/// (342-0077-B) boot failure.
#[test]
fn swapped_rom_halves_are_normalized() {
    // Build a 128KB ROM with the reset vector ($FA62) in the LOW half at the
    // bank-top offset, and zeros in the HIGH half — i.e. halves swapped.
    let mut rom = vec![0x00u8; 131072];
    rom[0x0_FFFC] = 0x62;
    rom[0x0_FFFD] = 0xFA;
    // Put a recognizable byte at bank $FF $0000 (low half offset 0) to confirm
    // the swap actually moved the high bank into place.
    rom[0x0_0000] = 0xAB;

    let mem = IIgsMemory::new(256, rom).unwrap();
    // After normalization bank $FF holds the vectors.
    assert_eq!(mem.rom_read(0xFF, 0xFFFC), 0x62);
    assert_eq!(mem.rom_read(0xFF, 0xFFFD), 0xFA);
    assert_eq!(mem.rom_read(0xFF, 0x0000), 0xAB);

    // Full emulator: reset must land on the real vector, not $0000.
    let mut rom2 = vec![0x00u8; 131072];
    rom2[0x0_FFFC] = 0x62;
    rom2[0x0_FFFD] = 0xFA;
    let emu = apple2_iigs::emulator::IIgsEmulator::new(256, rom2).unwrap();
    assert_eq!(emu.cpu.pc, 0xFA62);
}

/// A canonical 128KB ROM (vectors already in bank $FF) must be left untouched.
#[test]
fn canonical_rom_layout_is_not_swapped() {
    let mut rom = vec![0x00u8; 131072];
    rom[0x1_FFFC] = 0x62; // bank $FF $FFFC
    rom[0x1_FFFD] = 0xFA;
    rom[0x1_0000] = 0xCD; // bank $FF $0000
    let mem = IIgsMemory::new(256, rom).unwrap();
    assert_eq!(mem.rom_read(0xFF, 0xFFFC), 0x62);
    assert_eq!(mem.rom_read(0xFF, 0x0000), 0xCD);
}

/// Banks $E0/$E1 expose the Mega II I/O aperture (not raw fast RAM). A keypress
/// must be readable at $E0/$C000 just as at $00/$C000, and $C010 clears the
/// strobe. Regression for the DBR=$E1 cold-start poll hang.
#[test]
fn fast_bank_has_io_aperture() {
    let rom = vec![0xEA; 131072];
    let mem = IIgsMemory::new(256, rom).unwrap();
    let mut bus = IIgsBus::new(mem);

    bus.mega2.key_press(b'A');
    // Keyboard data register visible through the $E0 aperture.
    assert_eq!(bus.read(0xE0_C000, 0), b'A' | 0x80);
    // $C010 (any-key-up / strobe clear) reached via the $E1 aperture.
    let _ = bus.read(0xE1_C010, 0);
    assert!(!bus.mega2.key_strobe);

    // Below the aperture, $E0/$E1 are still plain fast RAM.
    bus.write(0xE0_2000, 0x5A, 0);
    assert_eq!(bus.read(0xE0_2000, 0), 0x5A);
}

/// One video frame in reference cycles.
const FRAME: u64 = apple2_iigs::mega2::CYCLES_PER_FRAME;

/// Build a bus with a stub ROM for interrupt tests.
fn interrupt_test_bus() -> IIgsBus {
    let rom = vec![0xEA; 131072];
    let mem = IIgsMemory::new(256, rom).unwrap();
    IIgsBus::new(mem)
}

/// Enabling VBL interrupts ($C041 bit 3) makes the 60 Hz heartbeat assert the
/// CPU IRQ line and set the $C046 VBL flag; $C047 acknowledges it.
#[test]
fn vbl_interrupt_fires_and_clears() {
    let mut bus = interrupt_test_bus();

    bus.write(0x00_C041, 0x08, 0); // enable VBL interrupts
    bus.update_interrupts(0);
    assert!(!bus.irq_line, "no interrupt before a frame elapses");

    bus.update_interrupts(FRAME);
    assert!(bus.irq_line, "VBL interrupt after one frame");
    assert_ne!(bus.read(0x00_C046, 0) & 0x08, 0, "$C046 VBL status set");

    // $C047 acknowledges the VBL (and quarter-second) interrupt.
    let _ = bus.read(0x00_C047, 0);
    bus.update_interrupts(FRAME); // same frame index → no new heartbeat
    assert!(!bus.irq_line, "IRQ line drops after acknowledge");
}

/// The quarter-second interrupt ($C041 bit 4) fires once every 16 VBLs.
#[test]
fn quarter_second_interrupt_fires_every_16_frames() {
    let mut bus = interrupt_test_bus();

    bus.write(0x00_C041, 0x10, 0); // enable quarter-second only
    for f in 1..=15 {
        bus.update_interrupts(f * FRAME);
    }
    assert!(!bus.irq_line, "must not fire before 16 frames");

    bus.update_interrupts(16 * FRAME);
    assert!(bus.irq_line, "quarter-second interrupt at frame 16");
    assert_ne!(bus.read(0x00_C046, 0) & 0x10, 0, "$C046 1/4-sec status set");
}

/// The one-second interrupt ($C023 bit 2) fires once every 60 VBLs and is
/// acknowledged through $C032.
#[test]
fn one_second_interrupt_fires_after_60_frames() {
    let mut bus = interrupt_test_bus();

    bus.write(0x00_C023, 0x04, 0); // enable one-second interrupt
    for f in 1..=59 {
        bus.update_interrupts(f * FRAME);
    }
    assert!(!bus.irq_line, "must not fire before 60 frames");

    bus.update_interrupts(60 * FRAME);
    assert!(bus.irq_line, "one-second interrupt at frame 60");
    let c023 = bus.read(0x00_C023, 0);
    assert_ne!(c023 & 0x80, 0, "$C023 VGC-interrupt bit set");
    assert_ne!(c023 & 0x40, 0, "$C023 one-second status set");

    // $C032 with the one-second clear bit low acknowledges the interrupt.
    bus.write(0x00_C032, 0x00, 0);
    bus.update_interrupts(60 * FRAME); // same frame index → no new heartbeat
    assert!(!bus.irq_line, "IRQ line drops after acknowledge");
}

/// A disabled interrupt source never asserts the line even as frames elapse.
#[test]
fn disabled_interrupts_stay_quiet() {
    let mut bus = interrupt_test_bus();
    for f in 1..=120 {
        bus.update_interrupts(f * FRAME);
    }
    assert!(
        !bus.irq_line,
        "no interrupts while all sources are disabled"
    );
}

/// Language-card read-source truth table: `$C080`/`$C083` select read-from-RAM,
/// `$C081`/`$C082` select read-from-ROM. Regression for the inverted `$C082`
/// case that made the GS/OS / ProDOS 16 loader's `LDA $C082 : SEC : JSR $FE1F`
/// identity check read RAM garbage → "REQUIRES APPLE IIGS HARDWARE".
#[test]
fn language_card_read_source_switches() {
    // Canonical 128KB ROM (vectors in bank $FF) with a marker byte at $FF/$FE1F.
    let mut rom = vec![0x00u8; 131072];
    rom[0x1FFFC] = 0x00;
    rom[0x1FFFD] = 0xFA; // valid reset vector → no half-swap
    rom[0x1_0000 + 0xFE1F] = 0xCC; // bank $FF, $FE1F
    let mem = IIgsMemory::new(256, rom).unwrap();
    let mut bus = IIgsBus::new(mem);

    // Put a distinct value in bank $00 language-card RAM at $FE1F.
    // ($C083 = read RAM / write enable after two reads.)
    bus.read(0x00_C083, 0);
    bus.read(0x00_C083, 0);
    bus.write(0x00_FE1F, 0x42, 0);

    // $C082: read ROM, write-protect → must see the ROM marker, not RAM.
    bus.read(0x00_C082, 0);
    assert_eq!(bus.read(0x00_FE1F, 0), 0xCC, "$C082 must read ROM");

    // $C080: read RAM, write-protect → must see the RAM value.
    bus.read(0x00_C080, 0);
    assert_eq!(bus.read(0x00_FE1F, 0), 0x42, "$C080 must read RAM");

    // $C081: read ROM (write enable) → ROM again.
    bus.read(0x00_C081, 0);
    assert_eq!(bus.read(0x00_FE1F, 0), 0xCC, "$C081 must read ROM");

    // $C083: read RAM → RAM again.
    bus.read(0x00_C083, 0);
    assert_eq!(bus.read(0x00_FE1F, 0), 0x42, "$C083 must read RAM");
}

/// The `HIGHRAM` (read-from-RAM) memory-mode flag tracks the language-card
/// read source: set for `$C080`/`$C083`, clear for `$C081`/`$C082`.
#[test]
fn language_card_highram_flag_matches_hardware() {
    use apple2_core::bus::MemMode;
    let rom = vec![0xEA; 131072];
    let mem = IIgsMemory::new(256, rom).unwrap();
    let mut bus = IIgsBus::new(mem);

    let highram = |bus: &IIgsBus| bus.mega2.mem_mode.contains(MemMode::MF_HIGHRAM);

    bus.read(0x00_C080, 0);
    assert!(highram(&bus), "$C080 → read RAM");
    bus.read(0x00_C081, 0);
    assert!(!highram(&bus), "$C081 → read ROM");
    bus.read(0x00_C082, 0);
    assert!(!highram(&bus), "$C082 → read ROM");
    bus.read(0x00_C083, 0);
    assert!(highram(&bus), "$C083 → read RAM");
}

/// STATEREG ($C068) bit layout: `[7]ALTZP [6]PAGE2 [5]RAMRD [4]RAMWRT [3]RDROM
/// [2]LCBANK2 [1]ROMBANK [0]INTCXROM`. Bit 3 (RDROM) is the *inverse* of
/// HIGHRAM. Regression for the crash where `STATEREG = $0C` ("read ROM, bank 2")
/// was decoded as read-RAM, so the next ROM fetch hit uninitialised RAM (BRK).
#[test]
fn statereg_read_rom_bit() {
    use apple2_core::bus::MemMode;
    let rom = vec![0xEA; 131072];
    let mem = IIgsMemory::new(256, rom).unwrap();
    let mut bus = IIgsBus::new(mem);

    // $0C = RDROM (bit 3) + LCBANK2 (bit 2): read ROM, language-card bank 2.
    bus.write(0x00_C068, 0x0C, 0);
    assert!(
        !bus.mega2.mem_mode.contains(MemMode::MF_HIGHRAM),
        "RDROM set → read ROM (HIGHRAM clear)"
    );
    assert!(
        bus.mega2.mem_mode.contains(MemMode::MF_BANK2),
        "LCBANK2 set"
    );
    // Read-back reports RDROM (bit 3) set, LCBANK2 (bit 2) set.
    assert_eq!(bus.read(0x00_C068, 0) & 0x0C, 0x0C);

    // $00 = RDROM clear → read RAM (HIGHRAM set).
    bus.write(0x00_C068, 0x00, 0);
    assert!(
        bus.mega2.mem_mode.contains(MemMode::MF_HIGHRAM),
        "RDROM clear → read RAM (HIGHRAM set)"
    );
    assert_eq!(bus.read(0x00_C068, 0) & 0x08, 0x00, "read-back RDROM clear");

    // INTCXROM is bit 0.
    bus.write(0x00_C068, 0x01, 0);
    assert!(bus.mega2.mem_mode.contains(MemMode::MF_INTCXROM));
    assert_eq!(bus.read(0x00_C068, 0) & 0x01, 0x01);
}

/// ROM banks ($FC-$FF) are a linear image: `$C000-$CFFF` reads ROM, not the I/O
/// aperture or slot cache. The firmware runs real code at `$FF/$C0xx`.
/// Regression for the `JSR $C085`-into-I/O crash.
#[test]
fn rom_bank_c0xx_reads_rom_not_io() {
    let mut rom = vec![0x00u8; 131072];
    rom[0x1FFFC] = 0x00;
    rom[0x1FFFD] = 0xFA; // valid vector → no half-swap; bank $FF = second half
    rom[0x1_0000 + 0xC085] = 0xA4; // marker byte at $FF/$C085 (real ROM would be code)
    rom[0x1_0000 + 0xC0EE] = 0x5A; // marker at $FF/$C0EE (an "I/O" address)
    let mem = IIgsMemory::new(256, rom).unwrap();
    let mut bus = IIgsBus::new(mem);

    // Reading through bank $FF must return the ROM bytes, not I/O register values.
    assert_eq!(bus.read(0xFF_C085, 0), 0xA4, "$FF/$C085 is ROM");
    assert_eq!(bus.read(0xFF_C0EE, 0), 0x5A, "$FF/$C0EE is ROM, not I/O");
}

/// $C071-$C07F reads ROM bank $FF (the native interrupt-vector dispatch), not
/// I/O — even through the bank $00/$E0/$E1 apertures. The ROM's IRQ vector
/// points here (e.g. $C074 = `CLV; JML ...`); returning I/O ($00 = BRK) would
/// trap the CPU in a BRK loop the moment any interrupt fired.
#[test]
fn c07x_vector_area_reads_rom() {
    let mut rom = vec![0x00u8; 131072];
    rom[0x1FFFC] = 0x00;
    rom[0x1FFFD] = 0xFA; // valid vector → bank $FF = second half
    rom[0x1_0000 + 0xC074] = 0xB8; // bank $FF, $C074 (CLV) — the IRQ dispatch
    rom[0x1_0000 + 0xC071] = 0x4C; // bank $FF, $C071 (BRK vector target)
    let mem = IIgsMemory::new(256, rom).unwrap();
    let mut bus = IIgsBus::new(mem);

    // Through the bank $00 I/O aperture and the bank $E1 aperture.
    assert_eq!(bus.read(0x00_C074, 0), 0xB8, "$00/$C074 → ROM $FF/$C074");
    assert_eq!(bus.read(0x00_C071, 0), 0x4C, "$00/$C071 → ROM $FF/$C071");
    assert_eq!(bus.read(0xE1_C074, 0), 0xB8, "$E1/$C074 → ROM $FF/$C074");
    // $C070 (paddle trigger) and $C080 (language card) are NOT in the ROM window.
    assert_ne!(bus.read(0x00_C070, 0), 0xB8);
}

/// The IWM mode register ($C0EF write / $C0EE status read-back) round-trips, and
/// the status register reports the no-disk SENSE bit. Regression for the IIgs
/// power-on self-test that writes the mode register and polls it back.
#[test]
fn iwm_mode_register_selftest() {
    let rom = vec![0xEA; 131072];
    let mem = IIgsMemory::new(256, rom).unwrap();
    let mut bus = IIgsBus::new(mem);

    // Select Q6 (status/mode), then write the mode register via $C0EF (Q7 high).
    let _ = bus.read(0x00_C0ED, 0); // Q6 = 1
    bus.write(0x00_C0EF, 0x0F, 0); // Q7 = 1, write mode = $0F

    // Read the status register ($C0EE sets Q7 = 0, Q6 still 1): low 5 bits echo
    // the mode register, and bit 7 (SENSE) is high (no disk).
    let status = bus.read(0x00_C0EE, 0);
    assert_eq!(
        status & 0x1F,
        0x0F,
        "mode register reads back through status"
    );
    assert_ne!(status & 0x80, 0, "no-disk SENSE bit set");
}

/// The ADB micro-controller decodes GLU commands with the correct parameter
/// counts and responses. Regression for the boot init: a wrong `Sync` ($07)
/// length desynced the command stream, and a missing `ReadKbdLayouts` ($0F)
/// response timed out into the "Fatal system error $0911" death.
#[test]
fn adb_glu_commands_respond() {
    let rom = vec![0xEA; 131072];
    let mem = IIgsMemory::new(256, rom).unwrap();
    let mut bus = IIgsBus::new(mem); // ROM 01 → 4-byte Sync

    // Helper: send a command byte, run enough cycles for it to complete, then
    // read one response byte via $C026 (checking DATA_VALID in $C027 first).
    let read_response = |bus: &mut IIgsBus, cmd: u8, params: &[u8]| -> Vec<u8> {
        bus.write(0x00_C026, cmd, 0); // command
        for &p in params {
            bus.write(0x00_C026, p, 0);
        }
        bus.update_interrupts(1000);
        let mut out = Vec::new();
        for _ in 0..8 {
            if bus.read(0x00_C027, 0) & 0x20 == 0 {
                break; // no more DATA_VALID
            }
            out.push(bus.read(0x00_C026, 0));
        }
        out
    };

    // GetVersion ($0D) → revision 5 on ROM 01.
    assert_eq!(read_response(&mut bus, 0x0D, &[]), vec![0x05]);
    // ReadKbdLayouts ($0F) → 2 bytes: count = 10, 0.
    assert_eq!(read_response(&mut bus, 0x0F, &[]), vec![0x0A, 0x00]);
    // ReadCharSets ($0E) → 2 bytes: count = 8, 0.
    assert_eq!(read_response(&mut bus, 0x0E, &[]), vec![0x08, 0x00]);
    // ReadConfig ($0B) → 4 bytes starting with $82.
    let cfg = read_response(&mut bus, 0x0B, &[]);
    assert_eq!(cfg.len(), 4);
    assert_eq!(cfg[0], 0x82);

    // Sync ($07) consumes 4 parameter bytes on ROM 01 and produces no response,
    // so a following command is decoded correctly (stream stays in sync).
    assert_eq!(
        read_response(&mut bus, 0x07, &[0x00, 0x00, 0x00, 0x00]),
        Vec::<u8>::new()
    );
    assert_eq!(
        read_response(&mut bus, 0x0D, &[]),
        vec![0x05],
        "stream still aligned after Sync"
    );
}

/// The 2IMG (`.2mg`) parser reads the data offset from header byte `$18` and the
/// data length from `$1C`. Regression for reading the wrong header fields
/// (`$08`/`$0C`), which yielded a 1-byte disk with zero blocks so nothing booted.
#[test]
fn parses_2mg_header_offsets() {
    use apple2_iigs::smartport::SmartPortDisk;

    // Two 512-byte blocks; block 0 begins with a recognisable marker.
    let mut payload = vec![0u8; 1024];
    payload[0] = 0x01;
    payload[1] = 0x38;
    payload[512] = 0xAA;

    let mut img = vec![0u8; 64];
    img[0..4].copy_from_slice(b"2IMG");
    img[8] = 64; // header size
    img[0x0C] = 0x01; // format = ProDOS (deliberately non-zero: must NOT be read as length)
    img[0x14..0x18].copy_from_slice(&2u32.to_le_bytes()); // block count
    img[0x18..0x1C].copy_from_slice(&64u32.to_le_bytes()); // data offset
    img[0x1C..0x20].copy_from_slice(&1024u32.to_le_bytes()); // data length
    img.extend_from_slice(&payload);

    let disk = SmartPortDisk::from_2mg(&img, None).expect("valid 2mg");
    assert_eq!(disk.read_block(0).map(|b| &b[..2]), Some(&[0x01, 0x38][..]));
    assert_eq!(disk.read_block(1).map(|b| b[0]), Some(0xAA));
    assert_eq!(disk.read_block(2), None, "only 2 blocks");
}

/// Booting slot 5: the firmware `JMP $C500` runs the stub's boot loader, whose
/// `WDM $FD` trap reads block 0 into `$0800`. This checks the trap wiring by
/// driving the stub's boot entry directly.
#[test]
fn smartport_boot_loads_block0() {
    use apple2_iigs::smartport::SmartPortDisk;

    // Minimal 2-block disk; block 0 carries a marker byte.
    let mut payload = vec![0u8; 1024];
    payload[0] = 0x99;
    payload[1] = 0x42;
    let mut img = vec![0u8; 64];
    img[0..4].copy_from_slice(b"2IMG");
    img[8] = 64;
    img[0x18..0x1C].copy_from_slice(&64u32.to_le_bytes());
    img[0x1C..0x20].copy_from_slice(&1024u32.to_le_bytes());
    img.extend_from_slice(&payload);

    let rom = vec![0xEA; 131072];
    let mem = IIgsMemory::new(1024, rom).unwrap();
    let mut bus = IIgsBus::new(mem);
    bus.smartport
        .insert(0, SmartPortDisk::from_2mg(&img, None).unwrap());

    // Execute the boot entry ($C500) — LDX/LDY/LDA ID bytes then WDM $FD boot
    // trap — via a tiny 65C816 CPU run. After the trap, $0800 holds block 0.
    let mut cpu = Cpu65816::new();
    cpu.reset(&mut bus);
    cpu.pbr = 0;
    cpu.pc = 0xC500;
    cpu.emulation = true;
    cpu.stopped = false;
    for _ in 0..12 {
        if cpu.stopped {
            break;
        }
        // Stop once the boot loader reaches its JMP $0801.
        if cpu.pbr == 0 && cpu.pc == 0x0801 {
            break;
        }
        cpu65816::step(&mut cpu, &mut bus);
    }

    assert_eq!(bus.read(0x0800, 0), 0x99, "block 0 byte 0 loaded to $0800");
    assert_eq!(bus.read(0x0801, 0), 0x42, "block 0 byte 1 loaded to $0801");
}

/// The clock GLU ($C033/$C034) reads and writes battery RAM. GS/OS reads its
/// configuration through this interface during startup; returning a floating
/// bus here made it run away and crash. Exercises a BRAM write-then-read.
#[test]
fn clock_glu_bram_access() {
    let rom = vec![0xEA; 131072];
    let mem = IIgsMemory::new(256, rom).unwrap();
    let mut bus = IIgsBus::new(mem);

    // Write BRAM location $05 = $AB.
    // Command byte: bit7=0 (write), addr $05 in bits 2-5 → $05<<2 = $14, with
    // bit 6 set (op 4-7 = BRAM $00-$0F) → $54.
    bus.write(0x00_C033, 0x54, 0); // command (write, addr 5)
    bus.write(0x00_C034, 0x80, 0); // start — parse command
    bus.write(0x00_C033, 0xAB, 0); // data
    bus.write(0x00_C034, 0x80, 0); // start — write phase
    assert_eq!(bus.bram[5], 0xAB, "BRAM $05 written via the clock GLU");

    // Read BRAM location $05 back (command bit7=1 = read).
    bus.write(0x00_C033, 0xD4, 0); // command (read, addr 5)
    bus.write(0x00_C034, 0x80, 0); // start — parse command
    bus.write(0x00_C034, 0xC0, 0); // start (bit6 = read) — read phase
    assert_eq!(
        bus.read(0x00_C033, 0),
        0xAB,
        "BRAM $05 read back via the clock GLU"
    );
}
