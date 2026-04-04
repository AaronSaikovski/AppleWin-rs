//! Integration tests for the Apple II core emulation.
//!
//! These tests exercise the full Emulator struct (CPU + Bus together)
//! to verify that the system works end-to-end.

use apple2_core::cpu::Flags;
use apple2_core::emulator::Emulator;
use apple2_core::model::{Apple2Model, CpuType};

/// Build a ROM image with a reset vector pointing to `entry` and
/// optional code placed at an offset within the ROM.
fn build_rom(entry: u16, code_addr: u16, code: &[u8]) -> Vec<u8> {
    let mut rom = vec![0xEA; 16384]; // fill with NOP
    // Set reset vector at $FFFC
    let vec_off = 0xFFFC - 0xC000;
    rom[vec_off] = entry as u8;
    rom[vec_off + 1] = (entry >> 8) as u8;

    // Place code if it falls within ROM range ($C000-$FFFF)
    if code_addr >= 0xC000 {
        let off = (code_addr - 0xC000) as usize;
        for (i, &b) in code.iter().enumerate() {
            if off + i < rom.len() {
                rom[off + i] = b;
            }
        }
    }
    rom
}

// ===========================================================================
// Boot / reset vector
// ===========================================================================

#[test]
fn emulator_boots_to_reset_vector() {
    // Reset vector points to $C100
    let rom = build_rom(0xC100, 0xC100, &[0xEA]); // NOP at $C100
    let emu = Emulator::new(rom, Apple2Model::AppleIIeEnh, CpuType::Cpu65C02);
    assert_eq!(emu.cpu.pc, 0xC100);
    assert_eq!(emu.cpu.sp, 0xFF);
    assert!(emu.cpu.flags.contains(Flags::I));
}

#[test]
fn emulator_reset_restores_state() {
    let rom = build_rom(0xC100, 0xC100, &[0xEA]);
    let mut emu = Emulator::new(rom, Apple2Model::AppleIIeEnh, CpuType::Cpu65C02);

    // Mutate state
    emu.cpu.a = 0x42;
    emu.cpu.x = 0x99;
    emu.cpu.pc = 0x5000;
    emu.cpu.sp = 0x80;

    // Reset should restore to initial state
    emu.reset(true);
    assert_eq!(emu.cpu.a, 0);
    assert_eq!(emu.cpu.x, 0);
    assert_eq!(emu.cpu.pc, 0xC100);
    assert_eq!(emu.cpu.sp, 0xFF);
}

// ===========================================================================
// Execute small inline programs
// ===========================================================================

#[test]
fn execute_inline_ram_program() {
    // Build an emulator with reset vector pointing to $0200 (in RAM).
    let rom = build_rom(0x0200, 0, &[]);
    let mut emu = Emulator::new(rom, Apple2Model::AppleIIeEnh, CpuType::Cpu6502);

    // Place a program in RAM at $0200:
    // CLC; LDA #$10; ADC #$20; STA $50; NOP; NOP; JMP $020A (loop on NOPs)
    let program: &[u8] = &[
        0x18,             // $0200: CLC
        0xA9, 0x10,       // $0201: LDA #$10
        0x69, 0x20,       // $0203: ADC #$20
        0x85, 0x50,       // $0205: STA $50
        0xEA,             // $0207: NOP
        0xEA,             // $0208: NOP
        0x4C, 0x07, 0x02, // $0209: JMP $0207
    ];
    for (i, &b) in program.iter().enumerate() {
        emu.bus.main_ram[0x0200 + i] = b;
    }

    // Step through: CLC + LDA + ADC + STA = 4 instructions
    for _ in 0..4 {
        emu.step();
    }

    assert_eq!(emu.cpu.a, 0x30);
    assert_eq!(emu.bus.main_ram[0x50], 0x30);
}

#[test]
fn execute_loop_counting() {
    let rom = build_rom(0x0200, 0, &[]);
    let mut emu = Emulator::new(rom, Apple2Model::AppleIIeEnh, CpuType::Cpu6502);

    // Count from 0 to 10 in a loop
    let program: &[u8] = &[
        0xA2, 0x00,       // $0200: LDX #$00
        0xE8,             // $0202: INX
        0xE0, 0x0A,       // $0203: CPX #$0A
        0xD0, 0xFB,       // $0205: BNE $0202
        0x86, 0x50,       // $0207: STX $50
        0x4C, 0x09, 0x02, // $0209: JMP $0209 (halt)
    ];
    for (i, &b) in program.iter().enumerate() {
        emu.bus.main_ram[0x0200 + i] = b;
    }

    // Execute enough cycles to complete the loop (generous budget)
    let mut steps = 0;
    while emu.cpu.pc != 0x0209 && steps < 200 {
        emu.step();
        steps += 1;
    }

    assert_eq!(emu.cpu.x, 0x0A);
    assert_eq!(emu.bus.main_ram[0x50], 0x0A);
}

#[test]
fn execute_subroutine_call() {
    let rom = build_rom(0x0200, 0, &[]);
    let mut emu = Emulator::new(rom, Apple2Model::AppleIIeEnh, CpuType::Cpu6502);

    // Main: JSR $0300; STA $50; JMP halt
    // Sub at $0300: LDA #$42; RTS
    let main_code: &[u8] = &[
        0x20, 0x00, 0x03, // $0200: JSR $0300
        0x85, 0x50,       // $0203: STA $50
        0x4C, 0x05, 0x02, // $0205: JMP $0205 (halt)
    ];
    let sub_code: &[u8] = &[
        0xA9, 0x42,       // $0300: LDA #$42
        0x60,             // $0302: RTS
    ];
    for (i, &b) in main_code.iter().enumerate() {
        emu.bus.main_ram[0x0200 + i] = b;
    }
    for (i, &b) in sub_code.iter().enumerate() {
        emu.bus.main_ram[0x0300 + i] = b;
    }

    // Step: JSR, LDA, RTS, STA = 4 instructions to reach STA result
    for _ in 0..4 {
        emu.step();
    }

    assert_eq!(emu.cpu.a, 0x42);
    assert_eq!(emu.bus.main_ram[0x50], 0x42);
}

#[test]
fn execute_with_cycle_budget() {
    let rom = build_rom(0x0200, 0, &[]);
    let mut emu = Emulator::new(rom, Apple2Model::AppleIIeEnh, CpuType::Cpu6502);

    // LDA #$42; NOP; NOP; NOP...
    let program: &[u8] = &[
        0xA9, 0x42,       // LDA #$42 (2 cycles)
        0xEA,             // NOP (2 cycles)
        0xEA,             // NOP (2 cycles)
        0xEA,             // NOP (2 cycles)
        0xEA,             // NOP (2 cycles)
    ];
    for (i, &b) in program.iter().enumerate() {
        emu.bus.main_ram[0x0200 + i] = b;
    }

    // Execute with a budget of 6 cycles: should execute LDA + 2 NOPs
    let actual = emu.execute(6);
    assert!(actual >= 6);
    assert_eq!(emu.cpu.a, 0x42);
}

#[test]
fn execute_65c02_specific_instructions() {
    let rom = build_rom(0x0200, 0, &[]);
    let mut emu = Emulator::new(rom, Apple2Model::AppleIIeEnh, CpuType::Cpu65C02);

    // Test 65C02: STZ, BRA, INC A
    let program: &[u8] = &[
        0xA9, 0x05,       // $0200: LDA #$05
        0x1A,             // $0202: INC A  -> A=6
        0x1A,             // $0203: INC A  -> A=7
        0x85, 0x50,       // $0204: STA $50
        0x64, 0x60,       // $0206: STZ $60
        0x80, 0x00,       // $0208: BRA +0 (to next instruction)
        0xEA,             // $020A: NOP (halt-ish)
    ];
    for (i, &b) in program.iter().enumerate() {
        emu.bus.main_ram[0x0200 + i] = b;
    }

    // Execute all instructions
    for _ in 0..7 {
        emu.step();
    }

    assert_eq!(emu.bus.main_ram[0x50], 0x07);
    assert_eq!(emu.bus.main_ram[0x60], 0x00);
}

#[test]
fn snapshot_round_trip() {
    let rom = build_rom(0x0200, 0, &[]);
    let mut emu = Emulator::new(rom, Apple2Model::AppleIIeEnh, CpuType::Cpu6502);

    // Mutate some state
    emu.cpu.a = 0x42;
    emu.cpu.x = 0x10;
    emu.bus.main_ram[0x50] = 0xDE;

    let snap = emu.take_snapshot();

    // Mutate more
    emu.cpu.a = 0x00;
    emu.bus.main_ram[0x50] = 0x00;

    // Restore
    emu.restore_snapshot(&snap);
    assert_eq!(emu.cpu.a, 0x42);
    assert_eq!(emu.cpu.x, 0x10);
    assert_eq!(emu.bus.main_ram[0x50], 0xDE);
}

// ===========================================================================
// Fibonacci sequence test (exercises ADC, STA, LDA, branches)
// ===========================================================================

#[test]
fn fibonacci_first_ten() {
    let rom = build_rom(0x0200, 0, &[]);
    let mut emu = Emulator::new(rom, Apple2Model::AppleIIeEnh, CpuType::Cpu6502);

    // Compute first 10 Fibonacci numbers, store at $50-$59
    // $00 = a (starts 0), $01 = b (starts 1), output at $50+
    let program: &[u8] = &[
        0xA9, 0x00,       // $0200: LDA #$00
        0x85, 0x00,       // $0202: STA $00 (a = 0)
        0xA9, 0x01,       // $0204: LDA #$01
        0x85, 0x01,       // $0206: STA $01 (b = 1)
        0xA2, 0x00,       // $0208: LDX #$00 (index)
        // loop:
        0xA5, 0x01,       // $020A: LDA $01 (load b)
        0x9D, 0x50, 0x00, // $020C: STA $0050,X (store fib[i])
        0x18,             // $020F: CLC
        0x65, 0x00,       // $0210: ADC $00 (b + a -> new)
        0x48,             // $0212: PHA     (push new)
        0xA5, 0x01,       // $0213: LDA $01
        0x85, 0x00,       // $0215: STA $00 (a = old b)
        0x68,             // $0217: PLA     (pull new)
        0x85, 0x01,       // $0218: STA $01 (b = new)
        0xE8,             // $021A: INX
        0xE0, 0x0A,       // $021B: CPX #$0A
        0xD0, 0xEB,       // $021D: BNE $020A (-21 from $021F)
        0x4C, 0x1F, 0x02, // $021F: JMP $021F (halt)
    ];
    for (i, &b) in program.iter().enumerate() {
        emu.bus.main_ram[0x0200 + i] = b;
    }

    let mut steps = 0;
    while emu.cpu.pc != 0x021F && steps < 500 {
        emu.step();
        steps += 1;
    }

    // Expected Fibonacci: 1, 1, 2, 3, 5, 8, 13, 21, 34, 55
    let expected = [1u8, 1, 2, 3, 5, 8, 13, 21, 34, 55];
    for (i, &exp) in expected.iter().enumerate() {
        assert_eq!(
            emu.bus.main_ram[0x50 + i], exp,
            "fib[{i}]: expected {exp}, got {}",
            emu.bus.main_ram[0x50 + i]
        );
    }
}
