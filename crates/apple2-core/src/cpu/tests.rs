//! Comprehensive CPU unit tests for 6502 / 65C02 emulation.

use super::cpu6502::Cpu;
use super::dispatch;
use super::flags::Flags;
use crate::bus::Bus;
use crate::model::Apple2Model;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a minimal Bus with an empty 16 KB ROM (all zeros).
/// The ROM covers $C000-$FFFF; reset vector defaults to $0000.
fn make_bus() -> Bus {
    Bus::new(vec![0u8; 16384], Apple2Model::AppleIIeEnh)
}

/// Create a Bus whose reset vector points to `entry`.
fn _make_bus_with_reset(entry: u16) -> Bus {
    let mut rom = vec![0u8; 16384];
    // Reset vector is at $FFFC-$FFFD, ROM offset = addr - 0xC000
    let offset = 0xFFFC - 0xC000;
    rom[offset] = entry as u8;
    rom[offset + 1] = (entry >> 8) as u8;
    Bus::new(rom, Apple2Model::AppleIIeEnh)
}

/// Create a 6502 CPU (NMOS) positioned at `pc`.
fn cpu_6502(pc: u16) -> Cpu {
    let mut cpu = Cpu::new(false);
    cpu.pc = pc;
    cpu.flags = Flags::U; // clear all flags except U (always set)
    cpu
}

/// Create a 65C02 CPU (CMOS) positioned at `pc`.
fn cpu_65c02(pc: u16) -> Cpu {
    let mut cpu = Cpu::new(true);
    cpu.pc = pc;
    cpu.flags = Flags::U;
    cpu
}

/// Place bytes into main RAM starting at `addr`.
fn poke(bus: &mut Bus, addr: u16, bytes: &[u8]) {
    for (i, &b) in bytes.iter().enumerate() {
        bus.main_ram[addr as usize + i] = b;
    }
}

/// Read a byte from main RAM.
fn peek(bus: &Bus, addr: u16) -> u8 {
    bus.main_ram[addr as usize]
}

/// Execute one instruction via dispatch::step.
fn step(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    dispatch::step(cpu, bus)
}

/// Execute `n` instructions, returning total cycles.
fn step_n(cpu: &mut Cpu, bus: &mut Bus, n: usize) -> u64 {
    let start = cpu.cycles;
    for _ in 0..n {
        dispatch::step(cpu, bus);
    }
    cpu.cycles - start
}

// ===========================================================================
// 1. Load / Store instructions
// ===========================================================================

#[test]
fn lda_immediate() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    // LDA #$42
    poke(&mut bus, 0x0200, &[0xA9, 0x42]);
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x42);
    assert_eq!(cpu.pc, 0x0202);
    assert_eq!(cycles, 2);
    assert!(!cpu.flags.contains(Flags::Z));
    assert!(!cpu.flags.contains(Flags::N));
}

#[test]
fn lda_immediate_zero_flag() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    poke(&mut bus, 0x0200, &[0xA9, 0x00]);
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.flags.contains(Flags::Z));
    assert!(!cpu.flags.contains(Flags::N));
}

#[test]
fn lda_immediate_negative_flag() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    poke(&mut bus, 0x0200, &[0xA9, 0x80]);
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x80);
    assert!(!cpu.flags.contains(Flags::Z));
    assert!(cpu.flags.contains(Flags::N));
}

#[test]
fn lda_zero_page() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    bus.main_ram[0x10] = 0x55;
    poke(&mut bus, 0x0200, &[0xA5, 0x10]); // LDA $10
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x55);
    assert_eq!(cycles, 3);
}

#[test]
fn lda_zero_page_x() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.x = 0x05;
    bus.main_ram[0x15] = 0xAA;
    poke(&mut bus, 0x0200, &[0xB5, 0x10]); // LDA $10,X
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0xAA);
    assert_eq!(cycles, 4);
}

#[test]
fn lda_zero_page_x_wraps() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.x = 0x10;
    bus.main_ram[0x05] = 0xBB; // $F5 + $10 wraps to $05
    poke(&mut bus, 0x0200, &[0xB5, 0xF5]);
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0xBB);
}

#[test]
fn lda_absolute() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    bus.main_ram[0x1234] = 0x77;
    poke(&mut bus, 0x0200, &[0xAD, 0x34, 0x12]); // LDA $1234
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x77);
    assert_eq!(cpu.pc, 0x0203);
    assert_eq!(cycles, 4);
}

#[test]
fn lda_absolute_x_no_cross() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.x = 0x03;
    bus.main_ram[0x1003] = 0x99;
    poke(&mut bus, 0x0200, &[0xBD, 0x00, 0x10]); // LDA $1000,X
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x99);
    assert_eq!(cycles, 4); // no page cross
}

#[test]
fn lda_absolute_x_page_cross() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.x = 0x01;
    bus.main_ram[0x1100] = 0x66;
    poke(&mut bus, 0x0200, &[0xBD, 0xFF, 0x10]); // LDA $10FF,X crosses to $1100
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x66);
    assert_eq!(cycles, 5); // +1 for page cross
}

#[test]
fn lda_absolute_y_page_cross() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.y = 0x02;
    bus.main_ram[0x2001] = 0x44;
    poke(&mut bus, 0x0200, &[0xB9, 0xFF, 0x1F]); // LDA $1FFF,Y -> $2001
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x44);
    assert_eq!(cycles, 5); // page cross
}

#[test]
fn lda_indirect_x() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.x = 0x04;
    // Pointer at zp $14: lo=$34, hi=$12 -> $1234
    bus.main_ram[0x14] = 0x34;
    bus.main_ram[0x15] = 0x12;
    bus.main_ram[0x1234] = 0xCC;
    poke(&mut bus, 0x0200, &[0xA1, 0x10]); // LDA ($10,X)
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0xCC);
    assert_eq!(cycles, 6);
}

#[test]
fn lda_indirect_y_no_cross() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.y = 0x03;
    bus.main_ram[0x20] = 0x00;
    bus.main_ram[0x21] = 0x10; // pointer -> $1000
    bus.main_ram[0x1003] = 0xDD;
    poke(&mut bus, 0x0200, &[0xB1, 0x20]); // LDA ($20),Y
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0xDD);
    assert_eq!(cycles, 5); // no page cross
}

#[test]
fn lda_indirect_y_page_cross() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.y = 0x01;
    bus.main_ram[0x20] = 0xFF;
    bus.main_ram[0x21] = 0x10; // pointer -> $10FF
    bus.main_ram[0x1100] = 0xEE;
    poke(&mut bus, 0x0200, &[0xB1, 0x20]); // LDA ($20),Y -> $1100
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0xEE);
    assert_eq!(cycles, 6); // +1 for page cross
}

#[test]
fn ldx_immediate() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    poke(&mut bus, 0x0200, &[0xA2, 0x37]); // LDX #$37
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.x, 0x37);
}

#[test]
fn ldy_immediate() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    poke(&mut bus, 0x0200, &[0xA0, 0x99]); // LDY #$99
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.y, 0x99);
    assert!(cpu.flags.contains(Flags::N));
}

#[test]
fn sta_zero_page() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x42;
    poke(&mut bus, 0x0200, &[0x85, 0x30]); // STA $30
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x30), 0x42);
    assert_eq!(cycles, 3);
}

#[test]
fn sta_absolute() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0xFF;
    poke(&mut bus, 0x0200, &[0x8D, 0x00, 0x30]); // STA $3000
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x3000), 0xFF);
    assert_eq!(cycles, 4);
}

#[test]
fn stx_zero_page() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.x = 0x12;
    poke(&mut bus, 0x0200, &[0x86, 0x50]); // STX $50
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x50), 0x12);
}

#[test]
fn sty_zero_page() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.y = 0x34;
    poke(&mut bus, 0x0200, &[0x84, 0x60]); // STY $60
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x60), 0x34);
}

// ===========================================================================
// 2. Arithmetic: ADC / SBC (binary mode)
// ===========================================================================

#[test]
fn adc_immediate_no_carry() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x10;
    poke(&mut bus, 0x0200, &[0x69, 0x20]); // ADC #$20
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x30);
    assert!(!cpu.flags.contains(Flags::C));
    assert!(!cpu.flags.contains(Flags::V));
    assert!(!cpu.flags.contains(Flags::Z));
    assert!(!cpu.flags.contains(Flags::N));
}

#[test]
fn adc_immediate_with_carry_in() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x10;
    cpu.flags.insert(Flags::C);
    poke(&mut bus, 0x0200, &[0x69, 0x20]);
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x31); // 0x10 + 0x20 + 1
}

#[test]
fn adc_carry_out() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0xFF;
    poke(&mut bus, 0x0200, &[0x69, 0x01]);
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.flags.contains(Flags::C));
    assert!(cpu.flags.contains(Flags::Z));
}

#[test]
fn adc_overflow_positive() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x50; // +80
    poke(&mut bus, 0x0200, &[0x69, 0x50]); // +80 => 160 => -96 in signed
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0xA0);
    assert!(cpu.flags.contains(Flags::V)); // signed overflow
    assert!(cpu.flags.contains(Flags::N));
    assert!(!cpu.flags.contains(Flags::C));
}

#[test]
fn adc_overflow_negative() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x80; // -128
    poke(&mut bus, 0x0200, &[0x69, 0xFF]); // + (-1) => -129 overflow
    step(&mut cpu, &mut bus);
    // $80 + $FF + 0 = $17F -> A=$7F, C=1
    assert_eq!(cpu.a, 0x7F);
    assert!(cpu.flags.contains(Flags::V));
    assert!(cpu.flags.contains(Flags::C));
}

#[test]
fn sbc_immediate_no_borrow() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x50;
    cpu.flags.insert(Flags::C); // no borrow
    poke(&mut bus, 0x0200, &[0xE9, 0x10]); // SBC #$10
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x40);
    assert!(cpu.flags.contains(Flags::C)); // no borrow
    assert!(!cpu.flags.contains(Flags::V));
}

#[test]
fn sbc_with_borrow() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x50;
    // carry clear = borrow
    poke(&mut bus, 0x0200, &[0xE9, 0x10]);
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x3F); // 0x50 - 0x10 - 1
    assert!(cpu.flags.contains(Flags::C));
}

#[test]
fn sbc_underflow() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x00;
    cpu.flags.insert(Flags::C);
    poke(&mut bus, 0x0200, &[0xE9, 0x01]);
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0xFF);
    assert!(!cpu.flags.contains(Flags::C)); // borrow occurred
    assert!(cpu.flags.contains(Flags::N));
}

// ===========================================================================
// 3. ADC / SBC BCD mode
// ===========================================================================

#[test]
fn adc_bcd_simple() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.flags.insert(Flags::D);
    cpu.a = 0x15; // BCD 15
    poke(&mut bus, 0x0200, &[0x69, 0x27]); // + BCD 27
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x42); // BCD 42
    assert!(!cpu.flags.contains(Flags::C));
}

#[test]
fn adc_bcd_carry() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.flags.insert(Flags::D);
    cpu.a = 0x58;
    poke(&mut bus, 0x0200, &[0x69, 0x46]); // 58 + 46 = 104 -> carry + 04
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x04);
    assert!(cpu.flags.contains(Flags::C));
}

#[test]
fn sbc_bcd_simple() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.flags.insert(Flags::D);
    cpu.flags.insert(Flags::C); // no borrow
    cpu.a = 0x42;
    poke(&mut bus, 0x0200, &[0xE9, 0x15]); // 42 - 15 = 27
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x27);
    assert!(cpu.flags.contains(Flags::C)); // no borrow
}

#[test]
fn sbc_bcd_borrow() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.flags.insert(Flags::D);
    cpu.flags.insert(Flags::C);
    cpu.a = 0x10;
    poke(&mut bus, 0x0200, &[0xE9, 0x20]); // 10 - 20 = -10 BCD
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x90); // BCD 90 with borrow
    assert!(!cpu.flags.contains(Flags::C));
}

// ===========================================================================
// 4. Logic: AND, ORA, EOR
// ===========================================================================

#[test]
fn and_immediate() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0xFF;
    poke(&mut bus, 0x0200, &[0x29, 0x0F]); // AND #$0F
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x0F);
    assert!(!cpu.flags.contains(Flags::N));
    assert!(!cpu.flags.contains(Flags::Z));
}

#[test]
fn ora_immediate() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0xF0;
    poke(&mut bus, 0x0200, &[0x09, 0x0F]); // ORA #$0F
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0xFF);
    assert!(cpu.flags.contains(Flags::N));
}

#[test]
fn eor_immediate() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0xFF;
    poke(&mut bus, 0x0200, &[0x49, 0xFF]); // EOR #$FF
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.flags.contains(Flags::Z));
}

// ===========================================================================
// 5. Shifts and rotates
// ===========================================================================

#[test]
fn asl_accumulator() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x81;
    poke(&mut bus, 0x0200, &[0x0A]); // ASL A
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x02);
    assert!(cpu.flags.contains(Flags::C)); // bit 7 was 1
    assert!(!cpu.flags.contains(Flags::N));
    assert_eq!(cycles, 2);
}

#[test]
fn asl_zero_page() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    bus.main_ram[0x30] = 0x40;
    poke(&mut bus, 0x0200, &[0x06, 0x30]); // ASL $30
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x30), 0x80);
    assert!(!cpu.flags.contains(Flags::C));
    assert!(cpu.flags.contains(Flags::N));
}

#[test]
fn lsr_accumulator() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x03;
    poke(&mut bus, 0x0200, &[0x4A]); // LSR A
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x01);
    assert!(cpu.flags.contains(Flags::C)); // bit 0 was 1
    assert!(!cpu.flags.contains(Flags::N));
}

#[test]
fn rol_accumulator() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x80;
    cpu.flags.insert(Flags::C);
    poke(&mut bus, 0x0200, &[0x2A]); // ROL A
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x01); // C rotated in, bit 7 rotated out
    assert!(cpu.flags.contains(Flags::C));
    assert!(!cpu.flags.contains(Flags::N));
}

#[test]
fn ror_accumulator() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x01;
    cpu.flags.insert(Flags::C);
    poke(&mut bus, 0x0200, &[0x6A]); // ROR A
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x80); // C rotated into bit 7
    assert!(cpu.flags.contains(Flags::C)); // bit 0 rotated out
    assert!(cpu.flags.contains(Flags::N));
}

// ===========================================================================
// 6. Increment / Decrement
// ===========================================================================

#[test]
fn inx_iny() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.x = 0xFE;
    cpu.y = 0x00;
    poke(&mut bus, 0x0200, &[0xE8, 0xC8]); // INX; INY
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.x, 0xFF);
    assert!(cpu.flags.contains(Flags::N));
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.y, 0x01);
    assert!(!cpu.flags.contains(Flags::N));
}

#[test]
fn inx_wraps() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.x = 0xFF;
    poke(&mut bus, 0x0200, &[0xE8]); // INX
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.x, 0x00);
    assert!(cpu.flags.contains(Flags::Z));
}

#[test]
fn dex_dey() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.x = 0x01;
    cpu.y = 0x01;
    poke(&mut bus, 0x0200, &[0xCA, 0x88]); // DEX; DEY
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.x, 0x00);
    assert!(cpu.flags.contains(Flags::Z));
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.y, 0x00);
    assert!(cpu.flags.contains(Flags::Z));
}

#[test]
fn inc_zero_page() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    bus.main_ram[0x40] = 0x7F;
    poke(&mut bus, 0x0200, &[0xE6, 0x40]); // INC $40
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x40), 0x80);
    assert!(cpu.flags.contains(Flags::N));
}

#[test]
fn dec_zero_page() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    bus.main_ram[0x40] = 0x00;
    poke(&mut bus, 0x0200, &[0xC6, 0x40]); // DEC $40
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x40), 0xFF);
    assert!(cpu.flags.contains(Flags::N));
}

// ===========================================================================
// 7. Compare
// ===========================================================================

#[test]
fn cmp_equal() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x42;
    poke(&mut bus, 0x0200, &[0xC9, 0x42]); // CMP #$42
    step(&mut cpu, &mut bus);
    assert!(cpu.flags.contains(Flags::Z));
    assert!(cpu.flags.contains(Flags::C));
    assert!(!cpu.flags.contains(Flags::N));
}

#[test]
fn cmp_greater() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x50;
    poke(&mut bus, 0x0200, &[0xC9, 0x30]); // CMP #$30
    step(&mut cpu, &mut bus);
    assert!(!cpu.flags.contains(Flags::Z));
    assert!(cpu.flags.contains(Flags::C)); // A >= val
}

#[test]
fn cmp_less() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x10;
    poke(&mut bus, 0x0200, &[0xC9, 0x30]); // CMP #$30
    step(&mut cpu, &mut bus);
    assert!(!cpu.flags.contains(Flags::Z));
    assert!(!cpu.flags.contains(Flags::C)); // A < val
    assert!(cpu.flags.contains(Flags::N)); // result is negative
}

#[test]
fn cpx_immediate() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.x = 0x10;
    poke(&mut bus, 0x0200, &[0xE0, 0x10]); // CPX #$10
    step(&mut cpu, &mut bus);
    assert!(cpu.flags.contains(Flags::Z));
    assert!(cpu.flags.contains(Flags::C));
}

#[test]
fn cpy_immediate() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.y = 0x20;
    poke(&mut bus, 0x0200, &[0xC0, 0x10]); // CPY #$10
    step(&mut cpu, &mut bus);
    assert!(!cpu.flags.contains(Flags::Z));
    assert!(cpu.flags.contains(Flags::C));
}

// ===========================================================================
// 8. Branches
// ===========================================================================

#[test]
fn bne_taken() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    // Z flag clear -> branch taken
    poke(&mut bus, 0x0200, &[0xD0, 0x05]); // BNE +5
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0207); // 0x0202 + 5
    assert_eq!(cycles, 3); // 2 base + 1 for taken
}

#[test]
fn bne_not_taken() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.flags.insert(Flags::Z);
    poke(&mut bus, 0x0200, &[0xD0, 0x05]); // BNE +5
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0202); // not taken
    assert_eq!(cycles, 2);
}

#[test]
fn beq_taken() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.flags.insert(Flags::Z);
    poke(&mut bus, 0x0200, &[0xF0, 0x10]); // BEQ +16
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0212);
    assert_eq!(cycles, 3);
}

#[test]
fn bpl_taken() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    // N clear
    poke(&mut bus, 0x0200, &[0x10, 0x0A]); // BPL +10
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x020C);
}

#[test]
fn bmi_taken() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.flags.insert(Flags::N);
    poke(&mut bus, 0x0200, &[0x30, 0x0A]); // BMI +10
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x020C);
}

#[test]
fn bcc_bcs() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    // BCC taken when C clear
    poke(&mut bus, 0x0200, &[0x90, 0x02]); // BCC +2
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0204);

    // BCS not taken when C clear
    cpu.pc = 0x0200;
    poke(&mut bus, 0x0200, &[0xB0, 0x02]); // BCS +2
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0202);
}

#[test]
fn branch_backward() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0210);
    poke(&mut bus, 0x0210, &[0xD0, 0xFC]); // BNE -4
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x020E); // 0x0212 + (-4)
}

#[test]
fn branch_page_cross_extra_cycle() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x04FD);
    // BNE forward: PC after fetch = $04FF, target = $04FF + $05 = $0504
    // Page changes from $04xx to $05xx -> page cross
    poke(&mut bus, 0x04FD, &[0xD0, 0x05]); // BNE +5
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0504); // crosses from $04xx to $05xx
    assert_eq!(cycles, 4); // 2 base + 2 for taken + page cross
}

// ===========================================================================
// 9. JMP, JSR, RTS, RTI
// ===========================================================================

#[test]
fn jmp_absolute() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    poke(&mut bus, 0x0200, &[0x4C, 0x00, 0x10]); // JMP $1000
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x1000);
    assert_eq!(cycles, 3);
}

#[test]
fn jmp_indirect() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    bus.main_ram[0x1000] = 0x34;
    bus.main_ram[0x1001] = 0x12;
    poke(&mut bus, 0x0200, &[0x6C, 0x00, 0x10]); // JMP ($1000)
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x1234);
}

#[test]
fn jmp_indirect_nmos_page_bug() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    // NMOS bug: JMP ($10FF) reads lo from $10FF, hi from $1000 (not $1100)
    bus.main_ram[0x10FF] = 0x34;
    bus.main_ram[0x1000] = 0x12; // hi wraps within page
    poke(&mut bus, 0x0200, &[0x6C, 0xFF, 0x10]);
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x1234);
}

#[test]
fn jmp_indirect_65c02_no_page_bug() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    bus.main_ram[0x10FF] = 0x34;
    bus.main_ram[0x1100] = 0x12; // correct hi byte location
    poke(&mut bus, 0x0200, &[0x6C, 0xFF, 0x10]);
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x1234);
}

#[test]
fn jsr_rts() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    // JSR $1000
    poke(&mut bus, 0x0200, &[0x20, 0x00, 0x10]);
    // At $1000: RTS
    poke(&mut bus, 0x1000, &[0x60]);

    let sp_before = cpu.sp;
    let jsr_cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x1000);
    assert_eq!(cpu.sp, sp_before.wrapping_sub(2)); // pushed 2 bytes
    assert_eq!(jsr_cycles, 6);

    let rts_cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0203); // return to instruction after JSR
    assert_eq!(cpu.sp, sp_before);
    assert_eq!(rts_cycles, 6);
}

#[test]
fn brk_rti() {
    // Set IRQ vector to $1000
    let mut rom = vec![0u8; 16384];
    let vec_off = 0xFFFE - 0xC000;
    rom[vec_off] = 0x00;
    rom[vec_off + 1] = 0x10;
    let mut bus = Bus::new(rom, Apple2Model::AppleIIeEnh);
    let mut cpu = cpu_6502(0x0200);
    cpu.flags = Flags::U; // clear I flag

    // BRK at $0200
    poke(&mut bus, 0x0200, &[0x00, 0x00]); // BRK + padding byte
    // RTI at $1000
    poke(&mut bus, 0x1000, &[0x40]); // RTI

    let sp_before = cpu.sp;
    let brk_cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x1000);
    assert!(cpu.flags.contains(Flags::I)); // I flag set after BRK
    assert_eq!(brk_cycles, 7);
    assert_eq!(cpu.sp, sp_before.wrapping_sub(3)); // pushed PC + flags

    let rti_cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0202); // return past BRK + padding
    assert_eq!(rti_cycles, 6);
    assert!(!cpu.flags.contains(Flags::I)); // flags restored
}

// ===========================================================================
// 10. Stack operations
// ===========================================================================

#[test]
fn pha_pla() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x42;
    poke(&mut bus, 0x0200, &[0x48, 0xA9, 0x00, 0x68]); // PHA; LDA #$00; PLA
    step(&mut cpu, &mut bus); // PHA
    assert_eq!(cpu.sp, 0xFE);
    step(&mut cpu, &mut bus); // LDA #$00
    assert_eq!(cpu.a, 0x00);
    step(&mut cpu, &mut bus); // PLA
    assert_eq!(cpu.a, 0x42);
    assert_eq!(cpu.sp, 0xFF);
}

#[test]
fn php_plp() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.flags = Flags::C | Flags::N | Flags::U;
    poke(&mut bus, 0x0200, &[0x08, 0x28]); // PHP; PLP
    step(&mut cpu, &mut bus); // PHP
    // PHP pushes flags with B and U set
    let pushed = bus.main_ram[0x01FF]; // SP was 0xFF, pushed at 0x01FF
    assert!(pushed & Flags::B.bits() != 0);
    assert!(pushed & Flags::U.bits() != 0);

    cpu.flags = Flags::U; // clear all
    step(&mut cpu, &mut bus); // PLP
    assert!(cpu.flags.contains(Flags::C));
    assert!(cpu.flags.contains(Flags::N));
    assert!(cpu.flags.contains(Flags::U)); // always set
}

// ===========================================================================
// 11. Transfers
// ===========================================================================

#[test]
fn tax_tay_txa_tya() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x42;
    // TAX
    poke(&mut bus, 0x0200, &[0xAA]);
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.x, 0x42);
    // TAY
    poke(&mut bus, 0x0201, &[0xA8]);
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.y, 0x42);
    // Load different value into X
    cpu.x = 0x99;
    poke(&mut bus, 0x0202, &[0x8A]); // TXA
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x99);
    // TYA
    cpu.y = 0x11;
    poke(&mut bus, 0x0203, &[0x98]); // TYA
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x11);
}

#[test]
fn txs_tsx() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.x = 0xAA;
    poke(&mut bus, 0x0200, &[0x9A]); // TXS
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.sp, 0xAA);
    // TXS does NOT affect flags

    cpu.sp = 0x80;
    poke(&mut bus, 0x0201, &[0xBA]); // TSX
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.x, 0x80);
    assert!(cpu.flags.contains(Flags::N)); // TSX sets flags
}

// ===========================================================================
// 12. Flag instructions
// ===========================================================================

#[test]
fn flag_instructions() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    poke(
        &mut bus,
        0x0200,
        &[
            0x38, // SEC
            0x18, // CLC
            0xF8, // SED
            0xD8, // CLD
            0x78, // SEI
            0x58, // CLI
        ],
    );
    step(&mut cpu, &mut bus);
    assert!(cpu.flags.contains(Flags::C));
    step(&mut cpu, &mut bus);
    assert!(!cpu.flags.contains(Flags::C));
    step(&mut cpu, &mut bus);
    assert!(cpu.flags.contains(Flags::D));
    step(&mut cpu, &mut bus);
    assert!(!cpu.flags.contains(Flags::D));
    step(&mut cpu, &mut bus);
    assert!(cpu.flags.contains(Flags::I));
    step(&mut cpu, &mut bus);
    assert!(!cpu.flags.contains(Flags::I));
}

#[test]
fn clv() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.flags.insert(Flags::V);
    poke(&mut bus, 0x0200, &[0xB8]); // CLV
    step(&mut cpu, &mut bus);
    assert!(!cpu.flags.contains(Flags::V));
}

// ===========================================================================
// 13. BIT instruction
// ===========================================================================

#[test]
fn bit_zero_page() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x0F;
    bus.main_ram[0x30] = 0xC0; // bits 7 and 6 set
    poke(&mut bus, 0x0200, &[0x24, 0x30]); // BIT $30
    step(&mut cpu, &mut bus);
    assert!(cpu.flags.contains(Flags::N)); // bit 7 of memory
    assert!(cpu.flags.contains(Flags::V)); // bit 6 of memory
    assert!(cpu.flags.contains(Flags::Z)); // A & M == 0
}

#[test]
fn bit_absolute_not_zero() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0xFF;
    bus.main_ram[0x1000] = 0x40;
    poke(&mut bus, 0x0200, &[0x2C, 0x00, 0x10]); // BIT $1000
    step(&mut cpu, &mut bus);
    assert!(!cpu.flags.contains(Flags::N));
    assert!(cpu.flags.contains(Flags::V));
    assert!(!cpu.flags.contains(Flags::Z)); // A & M != 0
}

// ===========================================================================
// 14. NOP
// ===========================================================================

#[test]
fn nop() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    poke(&mut bus, 0x0200, &[0xEA]); // NOP
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0201);
    assert_eq!(cycles, 2);
}

// ===========================================================================
// 15. 65C02-specific instructions
// ===========================================================================

#[test]
fn bra_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    poke(&mut bus, 0x0200, &[0x80, 0x10]); // BRA +16
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0212);
    assert_eq!(cycles, 3); // 2 base + 1 taken
}

#[test]
fn stz_zero_page() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    bus.main_ram[0x30] = 0xFF;
    poke(&mut bus, 0x0200, &[0x64, 0x30]); // STZ $30
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x30), 0x00);
}

#[test]
fn stz_absolute() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    bus.main_ram[0x1000] = 0xFF;
    poke(&mut bus, 0x0200, &[0x9C, 0x00, 0x10]); // STZ $1000
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0x00);
}

#[test]
fn tsb_zero_page() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.a = 0x0F;
    bus.main_ram[0x30] = 0xF0;
    poke(&mut bus, 0x0200, &[0x04, 0x30]); // TSB $30
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x30), 0xFF); // result = M | A
    assert!(cpu.flags.contains(Flags::Z)); // A & M was 0
}

#[test]
fn tsb_not_zero() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.a = 0x0F;
    bus.main_ram[0x30] = 0x03; // A & M = 0x03 != 0
    poke(&mut bus, 0x0200, &[0x04, 0x30]);
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x30), 0x0F);
    assert!(!cpu.flags.contains(Flags::Z));
}

#[test]
fn trb_zero_page() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.a = 0x0F;
    bus.main_ram[0x30] = 0xFF;
    poke(&mut bus, 0x0200, &[0x14, 0x30]); // TRB $30
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x30), 0xF0); // M & ~A
    assert!(!cpu.flags.contains(Flags::Z)); // A & M was non-zero
}

#[test]
fn bit_immediate_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.a = 0x0F;
    cpu.flags.insert(Flags::N | Flags::V); // pre-set N and V
    poke(&mut bus, 0x0200, &[0x89, 0xC0]); // BIT #$C0
    step(&mut cpu, &mut bus);
    // BIT immediate only sets Z, does NOT touch N/V
    assert!(cpu.flags.contains(Flags::Z)); // A & imm == 0
    assert!(cpu.flags.contains(Flags::N)); // unchanged
    assert!(cpu.flags.contains(Flags::V)); // unchanged
}

#[test]
fn phx_plx_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.x = 0x42;
    poke(&mut bus, 0x0200, &[0xDA, 0xFA]); // PHX; PLX
    step(&mut cpu, &mut bus); // PHX
    assert_eq!(cpu.sp, 0xFE);
    cpu.x = 0x00;
    step(&mut cpu, &mut bus); // PLX
    assert_eq!(cpu.x, 0x42);
    assert_eq!(cpu.sp, 0xFF);
}

#[test]
fn phy_ply_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.y = 0x99;
    poke(&mut bus, 0x0200, &[0x5A, 0x7A]); // PHY; PLY
    step(&mut cpu, &mut bus); // PHY
    cpu.y = 0x00;
    step(&mut cpu, &mut bus); // PLY
    assert_eq!(cpu.y, 0x99);
    assert!(cpu.flags.contains(Flags::N));
}

#[test]
fn inc_a_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.a = 0x41;
    poke(&mut bus, 0x0200, &[0x1A]); // INC A (65C02)
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x42);
}

#[test]
fn dec_a_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.a = 0x01;
    poke(&mut bus, 0x0200, &[0x3A]); // DEC A (65C02)
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.flags.contains(Flags::Z));
}

#[test]
fn jmp_ind_absx_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.x = 0x02;
    // JMP ($1000,X) -> reads from $1002
    bus.main_ram[0x1002] = 0x34;
    bus.main_ram[0x1003] = 0x12;
    poke(&mut bus, 0x0200, &[0x7C, 0x00, 0x10]);
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x1234);
}

#[test]
fn lda_ind_zp_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    // (zp) indirect without index
    bus.main_ram[0x30] = 0x00;
    bus.main_ram[0x31] = 0x10;
    bus.main_ram[0x1000] = 0xAB;
    poke(&mut bus, 0x0200, &[0xB2, 0x30]); // LDA ($30)
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0xAB);
}

// ===========================================================================
// 16. Undocumented NMOS 6502 opcodes
// ===========================================================================

#[test]
fn lax_zero_page() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    bus.main_ram[0x30] = 0x42;
    poke(&mut bus, 0x0200, &[0xA7, 0x30]); // LAX $30
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x42);
    assert_eq!(cpu.x, 0x42);
}

#[test]
fn lax_absolute() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    bus.main_ram[0x1000] = 0xCC;
    poke(&mut bus, 0x0200, &[0xAF, 0x00, 0x10]); // LAX $1000
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0xCC);
    assert_eq!(cpu.x, 0xCC);
    assert!(cpu.flags.contains(Flags::N));
}

#[test]
fn sax_zero_page() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0xFF;
    cpu.x = 0x0F;
    poke(&mut bus, 0x0200, &[0x87, 0x30]); // SAX $30
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x30), 0x0F); // A & X
}

#[test]
fn dcp_zero_page() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x10;
    bus.main_ram[0x30] = 0x11; // DEC -> 0x10, then CMP A,0x10
    poke(&mut bus, 0x0200, &[0xC7, 0x30]); // DCP $30
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x30), 0x10);
    assert!(cpu.flags.contains(Flags::Z)); // A == decremented value
    assert!(cpu.flags.contains(Flags::C));
}

#[test]
fn isc_zero_page() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x20;
    cpu.flags.insert(Flags::C); // no borrow
    bus.main_ram[0x30] = 0x09; // INC -> 0x0A, then SBC A,0x0A -> 0x16
    poke(&mut bus, 0x0200, &[0xE7, 0x30]); // ISC $30
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x30), 0x0A);
    assert_eq!(cpu.a, 0x16); // 0x20 - 0x0A
    assert!(cpu.flags.contains(Flags::C)); // no borrow
}

#[test]
fn slo_zero_page() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x01;
    bus.main_ram[0x30] = 0x40; // ASL -> 0x80, then ORA 0x01 | 0x80 -> 0x81
    poke(&mut bus, 0x0200, &[0x07, 0x30]); // SLO $30
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x30), 0x80);
    assert_eq!(cpu.a, 0x81);
    assert!(!cpu.flags.contains(Flags::C)); // bit 7 of original was 0
    assert!(cpu.flags.contains(Flags::N));
}

#[test]
fn rla_zero_page() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0xFF;
    cpu.flags.insert(Flags::C);
    bus.main_ram[0x30] = 0x80; // ROL with C=1 -> 0x01, C=1; then AND 0xFF & 0x01 -> 0x01
    poke(&mut bus, 0x0200, &[0x27, 0x30]); // RLA $30
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x30), 0x01);
    assert_eq!(cpu.a, 0x01);
    assert!(cpu.flags.contains(Flags::C));
}

#[test]
fn sre_zero_page() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0xFF;
    bus.main_ram[0x30] = 0x02; // LSR -> 0x01, C=0; then EOR 0xFF ^ 0x01 -> 0xFE
    poke(&mut bus, 0x0200, &[0x47, 0x30]); // SRE $30
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x30), 0x01);
    assert_eq!(cpu.a, 0xFE);
    assert!(!cpu.flags.contains(Flags::C));
}

#[test]
fn rra_zero_page() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x10;
    cpu.flags.insert(Flags::C);
    bus.main_ram[0x30] = 0x02; // ROR with C=1 -> 0x81, C=0; then ADC 0x10 + 0x81 + 0(carry from ROR was stored)
    poke(&mut bus, 0x0200, &[0x67, 0x30]); // RRA $30
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x30), 0x81);
    // After ROR: val=0x81, C=0. ADC: A = 0x10 + 0x81 + 0 = 0x91
    assert_eq!(cpu.a, 0x91);
}

// ===========================================================================
// 17. JAM (NMOS 6502 halt)
// ===========================================================================

#[test]
fn jam_halts_cpu() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    poke(&mut bus, 0x0200, &[0x02]); // JAM/KIL opcode
    step(&mut cpu, &mut bus);
    assert!(cpu.jammed);
    // PC should freeze (backed up by 1 to re-execute JAM)
    let _frozen_pc = cpu.pc;
    step(&mut cpu, &mut bus);
    // Once jammed, behavior is undefined, but the flag should stay set
    assert!(cpu.jammed);
}

// ===========================================================================
// 18. Interrupt handling
// ===========================================================================

#[test]
fn irq_taken_when_i_clear() {
    let mut rom = vec![0u8; 16384];
    // IRQ vector -> $1000
    let vec_off = 0xFFFE - 0xC000;
    rom[vec_off] = 0x00;
    rom[vec_off + 1] = 0x10;
    let mut bus = Bus::new(rom, Apple2Model::AppleIIeEnh);
    let mut cpu = cpu_6502(0x0200);
    cpu.flags.remove(Flags::I); // ensure I is clear
    cpu.irq_pending = 1;

    poke(&mut bus, 0x0200, &[0xEA]); // NOP (will be skipped by IRQ)
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x1000);
    assert!(cpu.flags.contains(Flags::I)); // I set during ISR
    assert_eq!(cycles, 7);
}

#[test]
fn irq_blocked_when_i_set() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.flags.insert(Flags::I);
    cpu.irq_pending = 1;

    poke(&mut bus, 0x0200, &[0xEA]); // NOP
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0201); // NOP executed normally
}

#[test]
fn nmi_always_taken() {
    let mut rom = vec![0u8; 16384];
    // NMI vector -> $2000
    let vec_off = 0xFFFA - 0xC000;
    rom[vec_off] = 0x00;
    rom[vec_off + 1] = 0x20;
    let mut bus = Bus::new(rom, Apple2Model::AppleIIeEnh);
    let mut cpu = cpu_6502(0x0200);
    cpu.flags.insert(Flags::I); // I flag does NOT block NMI
    cpu.nmi_pending = 1;

    poke(&mut bus, 0x0200, &[0xEA]);
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x2000);
    assert_eq!(cycles, 7);
    assert_eq!(cpu.nmi_pending, 0); // cleared after servicing
}

#[test]
fn nmi_has_priority_over_irq() {
    let mut rom = vec![0u8; 16384];
    let nmi_off = 0xFFFA - 0xC000;
    rom[nmi_off] = 0x00;
    rom[nmi_off + 1] = 0x20;
    let irq_off = 0xFFFE - 0xC000;
    rom[irq_off] = 0x00;
    rom[irq_off + 1] = 0x10;
    let mut bus = Bus::new(rom, Apple2Model::AppleIIeEnh);
    let mut cpu = cpu_6502(0x0200);
    cpu.nmi_pending = 1;
    cpu.irq_pending = 1;

    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x2000); // NMI wins
}

// ===========================================================================
// 19. Page-crossing cycle penalties (more thorough)
// ===========================================================================

#[test]
fn ora_absx_page_cross() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.x = 0x01;
    bus.main_ram[0x1100] = 0x42;
    poke(&mut bus, 0x0200, &[0x1D, 0xFF, 0x10]); // ORA $10FF,X
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x42);
    assert_eq!(cycles, 5); // 4 + 1 page cross
}

#[test]
fn lda_indy_no_page_cross_cycles() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.y = 0x00;
    bus.main_ram[0x30] = 0x00;
    bus.main_ram[0x31] = 0x10;
    bus.main_ram[0x1000] = 0x42;
    poke(&mut bus, 0x0200, &[0xB1, 0x30]);
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cycles, 5); // no page cross
}

// ===========================================================================
// 20. Reset vector
// ===========================================================================

#[test]
fn reset_reads_vector() {
    let mut rom = vec![0u8; 16384];
    let vec_off = 0xFFFC - 0xC000;
    rom[vec_off] = 0x00;
    rom[vec_off + 1] = 0x03;
    let mut bus = Bus::new(rom, Apple2Model::AppleIIeEnh);
    let mut cpu = Cpu::new(false);
    cpu.reset(&mut bus);
    assert_eq!(cpu.pc, 0x0300);
    assert_eq!(cpu.sp, 0xFF);
    assert!(cpu.flags.contains(Flags::I));
    assert!(cpu.flags.contains(Flags::U));
}

// ===========================================================================
// 21. Multi-instruction sequence
// ===========================================================================

#[test]
fn count_to_five() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    // Program: LDX #$00; loop: INX; CPX #$05; BNE loop; NOP
    poke(
        &mut bus,
        0x0200,
        &[
            0xA2, 0x00, // LDX #$00
            0xE8, // INX
            0xE0, 0x05, // CPX #$05
            0xD0, 0xFB, // BNE -5 (back to INX)
            0xEA, // NOP (exit)
        ],
    );
    // LDX + 5*(INX+CPX+BNE) but last BNE not taken
    // Execute until we hit the NOP
    let mut count = 0;
    while cpu.pc != 0x0207 && count < 100 {
        step(&mut cpu, &mut bus);
        count += 1;
    }
    assert_eq!(cpu.x, 0x05);
    assert!(cpu.flags.contains(Flags::Z));
}

#[test]
fn simple_add_program() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    // CLC; LDA #$28; ADC #$14; STA $50
    poke(
        &mut bus,
        0x0200,
        &[
            0x18, // CLC
            0xA9, 0x28, // LDA #$28
            0x69, 0x14, // ADC #$14
            0x85, 0x50, // STA $50
        ],
    );
    step_n(&mut cpu, &mut bus, 4);
    assert_eq!(peek(&bus, 0x50), 0x3C); // 0x28 + 0x14
    assert_eq!(cpu.a, 0x3C);
}

// ===========================================================================
// 22. Cycle counting accuracy
// ===========================================================================

#[test]
fn cycle_counts_basic() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);

    // LDA #imm = 2 cycles
    poke(&mut bus, 0x0200, &[0xA9, 0x42]);
    assert_eq!(step(&mut cpu, &mut bus), 2);

    // STA abs = 4 cycles
    poke(&mut bus, 0x0202, &[0x8D, 0x00, 0x10]);
    assert_eq!(step(&mut cpu, &mut bus), 4);

    // JSR = 6 cycles
    cpu.pc = 0x0300;
    poke(&mut bus, 0x0300, &[0x20, 0x00, 0x10]);
    assert_eq!(step(&mut cpu, &mut bus), 6);

    // RTS = 6 cycles
    poke(&mut bus, 0x1000, &[0x60]);
    assert_eq!(step(&mut cpu, &mut bus), 6);
}

// ===========================================================================
// 23. BVC / BVS branches
// ===========================================================================

#[test]
fn bvc_taken_when_v_clear() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    // V clear by default
    poke(&mut bus, 0x0200, &[0x50, 0x05]); // BVC +5
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0207);
    assert_eq!(cycles, 3);
}

#[test]
fn bvc_not_taken_when_v_set() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.flags.insert(Flags::V);
    poke(&mut bus, 0x0200, &[0x50, 0x05]); // BVC +5
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0202);
    assert_eq!(cycles, 2);
}

#[test]
fn bvs_taken_when_v_set() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.flags.insert(Flags::V);
    poke(&mut bus, 0x0200, &[0x70, 0x05]); // BVS +5
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0207);
    assert_eq!(cycles, 3);
}

#[test]
fn bvs_not_taken_when_v_clear() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    poke(&mut bus, 0x0200, &[0x70, 0x05]); // BVS +5
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0202);
    assert_eq!(cycles, 2);
}

// ===========================================================================
// 24. IRQ deferral
// ===========================================================================

#[test]
fn irq_deferral_unit_test() {
    // IRQ deferral is implemented at the Emulator::execute() level.
    // This unit test verifies the CPU's irq_defer field behavior:
    // when irq_defer is set and irq_pending is cleared, the CPU executes
    // normally. Once irq_pending is set on the next cycle, the IRQ is taken.
    let mut rom = vec![0u8; 16384];
    let vec_off = 0xFFFE - 0xC000;
    rom[vec_off] = 0x00;
    rom[vec_off + 1] = 0x10; // IRQ vector -> $1000
    let mut bus = Bus::new(rom, Apple2Model::AppleIIeEnh);
    let mut cpu = cpu_6502(0x0200);
    cpu.flags.remove(Flags::I);

    poke(&mut bus, 0x0200, &[0xEA, 0xEA, 0xEA]); // NOP; NOP; NOP

    // Simulate the deferred state: irq_defer is true but irq_pending is 0.
    // The execute() loop would have cleared irq_pending on the first cycle
    // after detecting the edge.
    cpu.irq_defer = true;
    cpu.irq_pending = 0;

    // First step: no IRQ pending, so NOP executes normally
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0201);

    // Now set IRQ pending (as the execute loop would on the next iteration
    // after clearing irq_defer)
    cpu.irq_defer = false;
    cpu.irq_pending = 1;
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x1000); // IRQ taken
    assert!(cpu.flags.contains(Flags::I));
}

// ===========================================================================
// 25. More addressing mode edge cases
// ===========================================================================

#[test]
fn sta_indirect_x() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0xAB;
    cpu.x = 0x02;
    // Pointer at zp $12: $34, $12 -> addr $1234
    bus.main_ram[0x12] = 0x34;
    bus.main_ram[0x13] = 0x12;
    poke(&mut bus, 0x0200, &[0x81, 0x10]); // STA ($10,X)
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1234), 0xAB);
    assert_eq!(cycles, 6);
}

#[test]
fn sta_indirect_y() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0xCD;
    cpu.y = 0x05;
    bus.main_ram[0x20] = 0x00;
    bus.main_ram[0x21] = 0x10; // pointer -> $1000
    poke(&mut bus, 0x0200, &[0x91, 0x20]); // STA ($20),Y
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1005), 0xCD);
    assert_eq!(cycles, 6);
}

#[test]
fn sta_absolute_x() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0xEF;
    cpu.x = 0x03;
    poke(&mut bus, 0x0200, &[0x9D, 0x00, 0x10]); // STA $1000,X
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1003), 0xEF);
    assert_eq!(cycles, 5);
}

#[test]
fn sta_absolute_y() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x77;
    cpu.y = 0x04;
    poke(&mut bus, 0x0200, &[0x99, 0x00, 0x10]); // STA $1000,Y
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1004), 0x77);
    assert_eq!(cycles, 5);
}

#[test]
fn ldx_zero_page_y() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.y = 0x05;
    bus.main_ram[0x15] = 0x77;
    poke(&mut bus, 0x0200, &[0xB6, 0x10]); // LDX $10,Y
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.x, 0x77);
    assert_eq!(cycles, 4);
}

#[test]
fn stx_zero_page_y() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.x = 0x33;
    cpu.y = 0x02;
    poke(&mut bus, 0x0200, &[0x96, 0x10]); // STX $10,Y
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x12), 0x33);
}

#[test]
fn sty_zero_page_x() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.y = 0x55;
    cpu.x = 0x03;
    poke(&mut bus, 0x0200, &[0x94, 0x10]); // STY $10,X
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x13), 0x55);
}

// ===========================================================================
// 26. More 65C02 addressing variants
// ===========================================================================

#[test]
fn sta_ind_zp_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.a = 0xBE;
    bus.main_ram[0x30] = 0x00;
    bus.main_ram[0x31] = 0x10;
    poke(&mut bus, 0x0200, &[0x92, 0x30]); // STA ($30) - 65C02
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0xBE);
}

#[test]
fn cmp_ind_zp_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.a = 0x42;
    bus.main_ram[0x30] = 0x00;
    bus.main_ram[0x31] = 0x10;
    bus.main_ram[0x1000] = 0x42;
    poke(&mut bus, 0x0200, &[0xD2, 0x30]); // CMP ($30) - 65C02
    step(&mut cpu, &mut bus);
    assert!(cpu.flags.contains(Flags::Z));
    assert!(cpu.flags.contains(Flags::C));
}

#[test]
fn ora_ind_zp_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.a = 0x0F;
    bus.main_ram[0x30] = 0x00;
    bus.main_ram[0x31] = 0x10;
    bus.main_ram[0x1000] = 0xF0;
    poke(&mut bus, 0x0200, &[0x12, 0x30]); // ORA ($30) - 65C02
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0xFF);
}

#[test]
fn and_ind_zp_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.a = 0x0F;
    bus.main_ram[0x30] = 0x00;
    bus.main_ram[0x31] = 0x10;
    bus.main_ram[0x1000] = 0x37;
    poke(&mut bus, 0x0200, &[0x32, 0x30]); // AND ($30) - 65C02
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x07);
}

#[test]
fn eor_ind_zp_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.a = 0xFF;
    bus.main_ram[0x30] = 0x00;
    bus.main_ram[0x31] = 0x10;
    bus.main_ram[0x1000] = 0x55;
    poke(&mut bus, 0x0200, &[0x52, 0x30]); // EOR ($30) - 65C02
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0xAA);
}

#[test]
fn adc_ind_zp_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.a = 0x10;
    bus.main_ram[0x30] = 0x00;
    bus.main_ram[0x31] = 0x10;
    bus.main_ram[0x1000] = 0x20;
    poke(&mut bus, 0x0200, &[0x72, 0x30]); // ADC ($30) - 65C02
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x30);
}

#[test]
fn sbc_ind_zp_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.a = 0x50;
    cpu.flags.insert(Flags::C);
    bus.main_ram[0x30] = 0x00;
    bus.main_ram[0x31] = 0x10;
    bus.main_ram[0x1000] = 0x10;
    poke(&mut bus, 0x0200, &[0xF2, 0x30]); // SBC ($30) - 65C02
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x40);
}

#[test]
fn bit_zpx_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.a = 0x0F;
    cpu.x = 0x05;
    bus.main_ram[0x35] = 0xC0;
    poke(&mut bus, 0x0200, &[0x34, 0x30]); // BIT $30,X - 65C02
    step(&mut cpu, &mut bus);
    assert!(cpu.flags.contains(Flags::N));
    assert!(cpu.flags.contains(Flags::V));
    assert!(cpu.flags.contains(Flags::Z)); // A & M == 0
}

#[test]
fn bit_absx_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.a = 0xFF;
    cpu.x = 0x02;
    bus.main_ram[0x1002] = 0x40;
    poke(&mut bus, 0x0200, &[0x3C, 0x00, 0x10]); // BIT $1000,X - 65C02
    step(&mut cpu, &mut bus);
    assert!(!cpu.flags.contains(Flags::N));
    assert!(cpu.flags.contains(Flags::V));
    assert!(!cpu.flags.contains(Flags::Z));
}

#[test]
fn stz_zpx_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.x = 0x05;
    bus.main_ram[0x35] = 0xFF;
    poke(&mut bus, 0x0200, &[0x74, 0x30]); // STZ $30,X - 65C02
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x35), 0x00);
}

#[test]
fn stz_absx_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.x = 0x03;
    bus.main_ram[0x1003] = 0xFF;
    poke(&mut bus, 0x0200, &[0x9E, 0x00, 0x10]); // STZ $1000,X - 65C02
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1003), 0x00);
}

#[test]
fn tsb_abs_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.a = 0x03;
    bus.main_ram[0x1000] = 0xFC;
    poke(&mut bus, 0x0200, &[0x0C, 0x00, 0x10]); // TSB $1000
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0xFF);
    assert!(cpu.flags.contains(Flags::Z)); // A & M was 0
}

#[test]
fn trb_abs_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    cpu.a = 0x0F;
    bus.main_ram[0x1000] = 0xFF;
    poke(&mut bus, 0x0200, &[0x1C, 0x00, 0x10]); // TRB $1000
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0xF0);
    assert!(!cpu.flags.contains(Flags::Z)); // A & M was non-zero
}

#[test]
fn brk_clears_decimal_on_65c02() {
    let mut rom = vec![0u8; 16384];
    let vec_off = 0xFFFE - 0xC000;
    rom[vec_off] = 0x00;
    rom[vec_off + 1] = 0x10;
    let mut bus = Bus::new(rom, Apple2Model::AppleIIeEnh);
    let mut cpu = cpu_65c02(0x0200);
    cpu.flags.insert(Flags::D);
    poke(&mut bus, 0x0200, &[0x00, 0x00]); // BRK
    step(&mut cpu, &mut bus);
    assert!(!cpu.flags.contains(Flags::D)); // 65C02 clears D on BRK
}

// ===========================================================================
// 27. More undocumented opcode addressing modes
// ===========================================================================

#[test]
fn lax_indirect_x() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.x = 0x04;
    bus.main_ram[0x14] = 0x34;
    bus.main_ram[0x15] = 0x12;
    bus.main_ram[0x1234] = 0x77;
    poke(&mut bus, 0x0200, &[0xA3, 0x10]); // LAX ($10,X)
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x77);
    assert_eq!(cpu.x, 0x77);
}

#[test]
fn lax_indirect_y() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.y = 0x02;
    bus.main_ram[0x20] = 0x00;
    bus.main_ram[0x21] = 0x10;
    bus.main_ram[0x1002] = 0x55;
    poke(&mut bus, 0x0200, &[0xB3, 0x20]); // LAX ($20),Y
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x55);
    assert_eq!(cpu.x, 0x55);
}

#[test]
fn lax_zero_page_y() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.y = 0x03;
    bus.main_ram[0x13] = 0xAA;
    poke(&mut bus, 0x0200, &[0xB7, 0x10]); // LAX $10,Y
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0xAA);
    assert_eq!(cpu.x, 0xAA);
}

#[test]
fn lax_absolute_y() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.y = 0x01;
    bus.main_ram[0x1001] = 0xBB;
    poke(&mut bus, 0x0200, &[0xBF, 0x00, 0x10]); // LAX $1000,Y
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0xBB);
    assert_eq!(cpu.x, 0xBB);
}

#[test]
fn sax_indirect_x() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0xF0;
    cpu.x = 0x04;
    bus.main_ram[0x14] = 0x34;
    bus.main_ram[0x15] = 0x12;
    poke(&mut bus, 0x0200, &[0x83, 0x10]); // SAX ($10,X)
    step(&mut cpu, &mut bus);
    // effective x for address is 0x04, but SAX stores A & X
    // X is used for the indirect address calculation, result = A & X = 0xF0 & 0x04 = 0x00
    // Wait -- SAX stores A & X, not A & operand. A=0xF0, X=0x04 => 0x00
    assert_eq!(peek(&bus, 0x1234), 0x00);
}

#[test]
fn sax_absolute() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0xFF;
    cpu.x = 0x33;
    poke(&mut bus, 0x0200, &[0x8F, 0x00, 0x10]); // SAX $1000
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0x33); // A & X = 0xFF & 0x33 = 0x33
}

#[test]
fn sax_zero_page_y() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0xAA;
    cpu.x = 0x55;
    cpu.y = 0x02;
    poke(&mut bus, 0x0200, &[0x97, 0x10]); // SAX $10,Y
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x12), 0x00); // A & X = 0xAA & 0x55 = 0x00
}

#[test]
fn dcp_absolute() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x05;
    bus.main_ram[0x1000] = 0x06;
    poke(&mut bus, 0x0200, &[0xCF, 0x00, 0x10]); // DCP $1000
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0x05);
    assert!(cpu.flags.contains(Flags::Z));
    assert!(cpu.flags.contains(Flags::C));
}

#[test]
fn isc_absolute() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x30;
    cpu.flags.insert(Flags::C);
    bus.main_ram[0x1000] = 0x0F; // INC -> 0x10, then SBC: 0x30 - 0x10 = 0x20
    poke(&mut bus, 0x0200, &[0xEF, 0x00, 0x10]); // ISC $1000
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0x10);
    assert_eq!(cpu.a, 0x20);
}

#[test]
fn slo_absolute() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x00;
    bus.main_ram[0x1000] = 0x80; // ASL -> 0x00 C=1, ORA 0x00|0x00 = 0x00
    poke(&mut bus, 0x0200, &[0x0F, 0x00, 0x10]); // SLO $1000
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0x00);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.flags.contains(Flags::C));
    assert!(cpu.flags.contains(Flags::Z));
}

#[test]
fn rla_absolute() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0xFF;
    bus.main_ram[0x1000] = 0x55; // ROL with C=0 -> 0xAA, C=0; AND 0xFF & 0xAA = 0xAA
    poke(&mut bus, 0x0200, &[0x2F, 0x00, 0x10]); // RLA $1000
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0xAA);
    assert_eq!(cpu.a, 0xAA);
}

#[test]
fn sre_absolute() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x00;
    bus.main_ram[0x1000] = 0x04; // LSR -> 0x02, C=0; EOR 0x00 ^ 0x02 = 0x02
    poke(&mut bus, 0x0200, &[0x4F, 0x00, 0x10]); // SRE $1000
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0x02);
    assert_eq!(cpu.a, 0x02);
}

#[test]
fn rra_absolute() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x10;
    bus.main_ram[0x1000] = 0x04; // ROR with C=0 -> 0x02, C=0; ADC 0x10 + 0x02 + 0 = 0x12
    poke(&mut bus, 0x0200, &[0x6F, 0x00, 0x10]); // RRA $1000
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0x02);
    assert_eq!(cpu.a, 0x12);
}

// ===========================================================================
// 28. Shift/rotate on memory (absolute)
// ===========================================================================

#[test]
fn asl_absolute_addr() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    bus.main_ram[0x1000] = 0x81;
    poke(&mut bus, 0x0200, &[0x0E, 0x00, 0x10]); // ASL $1000
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0x02);
    assert!(cpu.flags.contains(Flags::C));
    assert_eq!(cycles, 6);
}

#[test]
fn lsr_absolute_addr() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    bus.main_ram[0x1000] = 0x03;
    poke(&mut bus, 0x0200, &[0x4E, 0x00, 0x10]); // LSR $1000
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0x01);
    assert!(cpu.flags.contains(Flags::C));
}

#[test]
fn rol_absolute_addr() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.flags.insert(Flags::C);
    bus.main_ram[0x1000] = 0x80;
    poke(&mut bus, 0x0200, &[0x2E, 0x00, 0x10]); // ROL $1000
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0x01);
    assert!(cpu.flags.contains(Flags::C));
}

#[test]
fn ror_absolute_addr() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.flags.insert(Flags::C);
    bus.main_ram[0x1000] = 0x01;
    poke(&mut bus, 0x0200, &[0x6E, 0x00, 0x10]); // ROR $1000
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0x80);
    assert!(cpu.flags.contains(Flags::C));
}

// ===========================================================================
// 29. INC/DEC absolute
// ===========================================================================

#[test]
fn inc_absolute_addr() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    bus.main_ram[0x1000] = 0xFF;
    poke(&mut bus, 0x0200, &[0xEE, 0x00, 0x10]); // INC $1000
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0x00);
    assert!(cpu.flags.contains(Flags::Z));
}

#[test]
fn dec_absolute_addr() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    bus.main_ram[0x1000] = 0x01;
    poke(&mut bus, 0x0200, &[0xCE, 0x00, 0x10]); // DEC $1000
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0x00);
    assert!(cpu.flags.contains(Flags::Z));
}

// ===========================================================================
// 30. CPU snapshot round-trip
// ===========================================================================

#[test]
fn cpu_snapshot_round_trip() {
    use super::cpu6502::CpuSnapshot;

    let mut cpu = cpu_6502(0x1234);
    cpu.a = 0x42;
    cpu.x = 0xAA;
    cpu.y = 0xBB;
    cpu.sp = 0xCC;
    cpu.flags = Flags::C | Flags::N | Flags::U;
    cpu.cycles = 12345;

    let snap = CpuSnapshot::from(&cpu);
    assert_eq!(snap.a, 0x42);
    assert_eq!(snap.pc, 0x1234);

    // Modify CPU
    cpu.a = 0x00;
    cpu.pc = 0x0000;

    // Restore
    cpu.restore_snapshot(&snap);
    assert_eq!(cpu.a, 0x42);
    assert_eq!(cpu.x, 0xAA);
    assert_eq!(cpu.y, 0xBB);
    assert_eq!(cpu.sp, 0xCC);
    assert_eq!(cpu.pc, 0x1234);
    assert!(cpu.flags.contains(Flags::C));
    assert!(cpu.flags.contains(Flags::N));
    assert_eq!(cpu.cycles, 12345);
}

// ===========================================================================
// 31. 65C02: BRA backward, page cross
// ===========================================================================

#[test]
fn bra_backward_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0210);
    poke(&mut bus, 0x0210, &[0x80, 0xFC]); // BRA -4
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x020E);
}

#[test]
fn bra_page_cross_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x04FD);
    poke(&mut bus, 0x04FD, &[0x80, 0x05]); // BRA +5
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0504);
    assert_eq!(cycles, 4); // 2 base + 2 for taken + page cross
}

// ===========================================================================
// 32. SBC overflow flag
// ===========================================================================

#[test]
fn sbc_signed_overflow() {
    let mut bus = make_bus();
    let mut cpu = cpu_6502(0x0200);
    cpu.a = 0x80; // -128
    cpu.flags.insert(Flags::C);
    poke(&mut bus, 0x0200, &[0xE9, 0x01]); // SBC #$01 => -128 - 1 = -129 (overflow)
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.a, 0x7F);
    assert!(cpu.flags.contains(Flags::V));
    assert!(cpu.flags.contains(Flags::C));
}

// ===========================================================================
// 33. 65C02 undocumented opcodes become NOPs
// ===========================================================================

#[test]
fn undocumented_opcodes_nop_on_65c02() {
    let mut bus = make_bus();
    let mut cpu = cpu_65c02(0x0200);
    // On 65C02, opcode 0x02 (JAM on 6502) should be a NOP
    poke(&mut bus, 0x0200, &[0x02]);
    step(&mut cpu, &mut bus);
    assert!(!cpu.jammed); // 65C02 doesn't jam
}

// ===========================================================================
// 34. Multiple JAM opcodes on NMOS
// ===========================================================================

#[test]
fn jam_various_opcodes() {
    // Multiple opcodes map to JAM on NMOS: $02, $12, $22, $32, $42, $52, etc.
    let jam_opcodes: &[u8] = &[0x02, 0x12, 0x22, 0x32];
    for &opcode in jam_opcodes {
        let mut bus = make_bus();
        let mut cpu = cpu_6502(0x0200);
        poke(&mut bus, 0x0200, &[opcode]);
        step(&mut cpu, &mut bus);
        assert!(cpu.jammed, "opcode 0x{:02X} should JAM", opcode);
    }
}
