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
fn bus_bank_mirror() {
    let rom = vec![0xEA; 131072];
    let mem = IIgsMemory::new(1024, rom).unwrap();
    let mut bus = IIgsBus::new(mem);

    // Write to bank $02
    bus.write(0x02_5678, 0xCC, 0);
    // Bank $82 should mirror bank $02
    assert_eq!(bus.read(0x82_5678, 0), 0xCC);
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
