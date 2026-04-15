//! 65C816 CPU unit tests.

use crate::cpu65816::flags816::Flags816;
use crate::cpu65816::registers::Cpu65816;
use crate::cpu65816::{self, Bus816};

// ── Test bus ────────────────────────────────────────────────────────────

/// Flat 64KB memory bus for testing (bank 0 only, wraps for other banks).
struct TestBus {
    ram: Vec<u8>,
}

impl TestBus {
    fn new() -> Self {
        Self {
            ram: vec![0u8; 0x10000],
        }
    }

    /// Create a bus with 1MB of RAM (16 banks).
    fn new_large() -> Self {
        Self {
            ram: vec![0u8; 0x10_0000],
        }
    }
}

impl Bus816 for TestBus {
    fn read(&mut self, addr: u32, _cycles: u64) -> u8 {
        let idx = (addr as usize) % self.ram.len();
        self.ram[idx]
    }

    fn write(&mut self, addr: u32, val: u8, _cycles: u64) {
        let idx = (addr as usize) % self.ram.len();
        self.ram[idx] = val;
    }

    fn read_raw(&self, addr: u32) -> u8 {
        let idx = (addr as usize) % self.ram.len();
        self.ram[idx]
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn poke(bus: &mut TestBus, addr: u32, bytes: &[u8]) {
    for (i, &b) in bytes.iter().enumerate() {
        let a = addr + i as u32;
        let idx = (a as usize) % bus.ram.len();
        bus.ram[idx] = b;
    }
}

fn peek(bus: &TestBus, addr: u32) -> u8 {
    let idx = (addr as usize) % bus.ram.len();
    bus.ram[idx]
}

fn make_cpu(pc: u16) -> Cpu65816 {
    let mut cpu = Cpu65816::new();
    cpu.pc = pc;
    // Start in emulation mode (default), clear decimal
    cpu.flags = Flags816::M | Flags816::X | Flags816::I;
    cpu
}

fn make_native_cpu(pc: u16) -> Cpu65816 {
    let mut cpu = make_cpu(pc);
    // Switch to native mode with 16-bit registers
    cpu.emulation = false;
    cpu.flags = Flags816::I; // M=0, X=0 → 16-bit A and X/Y
    cpu
}

fn step(cpu: &mut Cpu65816, bus: &mut TestBus) -> u8 {
    cpu65816::step(cpu, bus)
}

// ── Reset ───────────────────────────────────────────────────────────────

#[test]
fn reset_reads_vector_from_bank0() {
    let mut bus = TestBus::new();
    poke(&mut bus, 0xFFFC, &[0x00, 0xC0]); // reset vector = $C000
    let mut cpu = Cpu65816::new();
    cpu.reset(&mut bus);
    assert_eq!(cpu.pc, 0xC000);
    assert_eq!(cpu.pbr, 0);
    assert!(cpu.emulation);
    assert!(cpu.flags.contains(Flags816::I));
}

// ── Emulation mode: basic load/store ────────────────────────────────────

#[test]
fn lda_immediate_emu() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    poke(&mut bus, 0x0200, &[0xA9, 0x42]); // LDA #$42
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x42);
    assert_eq!(cpu.pc, 0x0202);
    assert_eq!(cycles, 2);
    assert!(!cpu.flags.contains(Flags816::Z));
    assert!(!cpu.flags.contains(Flags816::N));
}

#[test]
fn lda_immediate_zero() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    poke(&mut bus, 0x0200, &[0xA9, 0x00]); // LDA #$00
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x00);
    assert!(cpu.flags.contains(Flags816::Z));
    assert!(!cpu.flags.contains(Flags816::N));
}

#[test]
fn lda_immediate_negative() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    poke(&mut bus, 0x0200, &[0xA9, 0x80]); // LDA #$80
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x80);
    assert!(!cpu.flags.contains(Flags816::Z));
    assert!(cpu.flags.contains(Flags816::N));
}

#[test]
fn ldx_immediate_emu() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    poke(&mut bus, 0x0200, &[0xA2, 0x55]); // LDX #$55
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.x & 0xFF, 0x55);
}

#[test]
fn ldy_immediate_emu() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    poke(&mut bus, 0x0200, &[0xA0, 0xAA]); // LDY #$AA
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.y & 0xFF, 0xAA);
}

#[test]
fn sta_absolute() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0x42;
    poke(&mut bus, 0x0200, &[0x8D, 0x00, 0x10]); // STA $1000
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0x42);
}

#[test]
fn stx_absolute() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.x = 0x77;
    poke(&mut bus, 0x0200, &[0x8E, 0x00, 0x10]); // STX $1000
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0x77);
}

// ── Arithmetic ──────────────────────────────────────────────────────────

#[test]
fn adc_immediate_no_carry() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0x10;
    cpu.flags.remove(Flags816::C);
    poke(&mut bus, 0x0200, &[0x69, 0x20]); // ADC #$20
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x30);
    assert!(!cpu.flags.contains(Flags816::C));
}

#[test]
fn adc_immediate_with_carry_in() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0x10;
    cpu.flags.insert(Flags816::C);
    poke(&mut bus, 0x0200, &[0x69, 0x20]); // ADC #$20
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x31);
}

#[test]
fn adc_carry_out() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0xFF;
    cpu.flags.remove(Flags816::C);
    poke(&mut bus, 0x0200, &[0x69, 0x01]); // ADC #$01
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x00);
    assert!(cpu.flags.contains(Flags816::C));
    assert!(cpu.flags.contains(Flags816::Z));
}

#[test]
fn adc_overflow() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0x7F; // +127
    cpu.flags.remove(Flags816::C);
    poke(&mut bus, 0x0200, &[0x69, 0x01]); // ADC #$01
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x80); // -128 (overflow!)
    assert!(cpu.flags.contains(Flags816::V));
    assert!(cpu.flags.contains(Flags816::N));
}

#[test]
fn sbc_immediate_no_borrow() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0x50;
    cpu.flags.insert(Flags816::C); // no borrow
    poke(&mut bus, 0x0200, &[0xE9, 0x20]); // SBC #$20
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x30);
    assert!(cpu.flags.contains(Flags816::C)); // no borrow out
}

#[test]
fn sbc_with_borrow() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0x00;
    cpu.flags.insert(Flags816::C);
    poke(&mut bus, 0x0200, &[0xE9, 0x01]); // SBC #$01
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0xFF);
    assert!(!cpu.flags.contains(Flags816::C)); // borrow occurred
    assert!(cpu.flags.contains(Flags816::N));
}

// ── BCD arithmetic ──────────────────────────────────────────────────────

#[test]
fn adc_bcd_8bit() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.flags.insert(Flags816::D);
    cpu.flags.remove(Flags816::C);
    cpu.c = 0x19; // BCD 19
    poke(&mut bus, 0x0200, &[0x69, 0x01]); // ADC #$01
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x20); // BCD 20
    assert!(!cpu.flags.contains(Flags816::C));
}

#[test]
fn adc_bcd_carry() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.flags.insert(Flags816::D);
    cpu.flags.remove(Flags816::C);
    cpu.c = 0x99; // BCD 99
    poke(&mut bus, 0x0200, &[0x69, 0x01]); // ADC #$01
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x00); // BCD 00 with carry
    assert!(cpu.flags.contains(Flags816::C));
}

// ── Logic ───────────────────────────────────────────────────────────────

#[test]
fn and_immediate() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0xFF;
    poke(&mut bus, 0x0200, &[0x29, 0x0F]); // AND #$0F
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x0F);
}

#[test]
fn ora_immediate() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0xF0;
    poke(&mut bus, 0x0200, &[0x09, 0x0F]); // ORA #$0F
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0xFF);
}

#[test]
fn eor_immediate() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0xFF;
    poke(&mut bus, 0x0200, &[0x49, 0xAA]); // EOR #$AA
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x55);
}

// ── Shifts / Rotates ────────────────────────────────────────────────────

#[test]
fn asl_accumulator() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0x81;
    poke(&mut bus, 0x0200, &[0x0A]); // ASL A
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x02);
    assert!(cpu.flags.contains(Flags816::C)); // bit 7 was set
}

#[test]
fn lsr_accumulator() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0x03;
    poke(&mut bus, 0x0200, &[0x4A]); // LSR A
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x01);
    assert!(cpu.flags.contains(Flags816::C)); // bit 0 was set
}

#[test]
fn rol_accumulator() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0x80;
    cpu.flags.insert(Flags816::C);
    poke(&mut bus, 0x0200, &[0x2A]); // ROL A
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x01); // carry rotated in
    assert!(cpu.flags.contains(Flags816::C)); // bit 7 rotated out
}

#[test]
fn ror_accumulator() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0x01;
    cpu.flags.insert(Flags816::C);
    poke(&mut bus, 0x0200, &[0x6A]); // ROR A
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x80); // carry rotated in to bit 7
    assert!(cpu.flags.contains(Flags816::C)); // bit 0 rotated out
}

// ── Branches ────────────────────────────────────────────────────────────

#[test]
fn bne_taken() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.flags.remove(Flags816::Z); // Z=0, so BNE is taken
    poke(&mut bus, 0x0200, &[0xD0, 0x05]); // BNE +5
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0207); // $0202 + 5
    assert!(cycles >= 3); // taken branch = base + 1
}

#[test]
fn bne_not_taken() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.flags.insert(Flags816::Z); // Z=1, so BNE not taken
    poke(&mut bus, 0x0200, &[0xD0, 0x05]); // BNE +5
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0202); // not taken, just skip operand
}

#[test]
fn bra_always() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    poke(&mut bus, 0x0200, &[0x80, 0x10]); // BRA +$10
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0212);
}

#[test]
fn bra_backward() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0210);
    poke(&mut bus, 0x0210, &[0x80, 0xFE]); // BRA -2 (infinite loop)
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0210); // loops back to itself
}

// ── Jumps / Calls ───────────────────────────────────────────────────────

#[test]
fn jmp_absolute() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    poke(&mut bus, 0x0200, &[0x4C, 0x00, 0x10]); // JMP $1000
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x1000);
}

#[test]
fn jsr_rts_roundtrip() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    // JSR $1000 at $0200, then RTS at $1000
    poke(&mut bus, 0x0200, &[0x20, 0x00, 0x10]); // JSR $1000
    poke(&mut bus, 0x1000, &[0x60]); // RTS
    step(&mut cpu, &mut bus); // JSR
    assert_eq!(cpu.pc, 0x1000);
    step(&mut cpu, &mut bus); // RTS
    assert_eq!(cpu.pc, 0x0203); // return to instruction after JSR
}

#[test]
fn jsl_rtl_roundtrip() {
    let mut bus = TestBus::new_large();
    let mut cpu = make_cpu(0x0200);
    cpu.emulation = false;
    cpu.flags = Flags816::I | Flags816::M | Flags816::X;
    // JSL $01/1000 at $00/0200, then RTL at $01/1000
    poke(&mut bus, 0x0200, &[0x22, 0x00, 0x10, 0x01]); // JSL $01:1000
    poke(&mut bus, 0x01_1000, &[0x6B]); // RTL
    step(&mut cpu, &mut bus); // JSL
    assert_eq!(cpu.pc, 0x1000);
    assert_eq!(cpu.pbr, 0x01);
    step(&mut cpu, &mut bus); // RTL
    assert_eq!(cpu.pc, 0x0204); // return to instruction after JSL
    assert_eq!(cpu.pbr, 0x00);
}

// ── Stack ───────────────────────────────────────────────────────────────

#[test]
fn pha_pla_roundtrip() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0x42;
    poke(&mut bus, 0x0200, &[0x48]); // PHA
    step(&mut cpu, &mut bus);
    cpu.c = 0x00;
    poke(&mut bus, 0x0201, &[0x68]); // PLA
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x42);
}

#[test]
fn phx_plx_roundtrip() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.x = 0xAB;
    poke(&mut bus, 0x0200, &[0xDA]); // PHX
    step(&mut cpu, &mut bus);
    cpu.x = 0x00;
    poke(&mut bus, 0x0201, &[0xFA]); // PLX
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.x & 0xFF, 0xAB);
}

// ── Transfers ───────────────────────────────────────────────────────────

#[test]
fn tax_transfer() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0x55;
    poke(&mut bus, 0x0200, &[0xAA]); // TAX
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.x & 0xFF, 0x55);
}

#[test]
fn tay_transfer() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0x77;
    poke(&mut bus, 0x0200, &[0xA8]); // TAY
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.y & 0xFF, 0x77);
}

#[test]
fn txa_transfer() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.x = 0x33;
    poke(&mut bus, 0x0200, &[0x8A]); // TXA
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x33);
}

// ── Flag instructions ───────────────────────────────────────────────────

#[test]
fn sec_clc() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    poke(&mut bus, 0x0200, &[0x38, 0x18]); // SEC, CLC
    step(&mut cpu, &mut bus);
    assert!(cpu.flags.contains(Flags816::C));
    step(&mut cpu, &mut bus);
    assert!(!cpu.flags.contains(Flags816::C));
}

#[test]
fn sed_cld() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    poke(&mut bus, 0x0200, &[0xF8, 0xD8]); // SED, CLD
    step(&mut cpu, &mut bus);
    assert!(cpu.flags.contains(Flags816::D));
    step(&mut cpu, &mut bus);
    assert!(!cpu.flags.contains(Flags816::D));
}

// ── Compare ─────────────────────────────────────────────────────────────

#[test]
fn cmp_equal() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0x42;
    poke(&mut bus, 0x0200, &[0xC9, 0x42]); // CMP #$42
    step(&mut cpu, &mut bus);
    assert!(cpu.flags.contains(Flags816::Z));
    assert!(cpu.flags.contains(Flags816::C));
}

#[test]
fn cmp_less_than() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0x10;
    poke(&mut bus, 0x0200, &[0xC9, 0x20]); // CMP #$20
    step(&mut cpu, &mut bus);
    assert!(!cpu.flags.contains(Flags816::Z));
    assert!(!cpu.flags.contains(Flags816::C));
}

// ── Increment / Decrement ───────────────────────────────────────────────

#[test]
fn inx_dex() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.x = 0x05;
    poke(&mut bus, 0x0200, &[0xE8, 0xCA]); // INX, DEX
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.x & 0xFF, 0x06);
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.x & 0xFF, 0x05);
}

#[test]
fn iny_dey() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.y = 0x10;
    poke(&mut bus, 0x0200, &[0xC8, 0x88]); // INY, DEY
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.y & 0xFF, 0x11);
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.y & 0xFF, 0x10);
}

#[test]
fn inc_dec_accumulator() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0xFF;
    poke(&mut bus, 0x0200, &[0x1A, 0x3A]); // INC A, DEC A
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x00);
    assert!(cpu.flags.contains(Flags816::Z));
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0xFF);
}

// ── BIT ─────────────────────────────────────────────────────────────────

#[test]
fn bit_immediate_only_sets_z() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0x0F;
    cpu.flags.insert(Flags816::N | Flags816::V);
    poke(&mut bus, 0x0200, &[0x89, 0xF0]); // BIT #$F0
    step(&mut cpu, &mut bus);
    assert!(cpu.flags.contains(Flags816::Z)); // $0F & $F0 = 0
    // N and V should NOT be modified by BIT immediate
    assert!(cpu.flags.contains(Flags816::N));
    assert!(cpu.flags.contains(Flags816::V));
}

// ── 65C816-specific: XBA ────────────────────────────────────────────────

#[test]
fn xba_swaps_a_and_b() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0x1234;
    poke(&mut bus, 0x0200, &[0xEB]); // XBA
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c, 0x3412);
    // N and Z are set from the NEW low byte (0x12)
    assert!(!cpu.flags.contains(Flags816::Z));
    assert!(!cpu.flags.contains(Flags816::N));
}

// ── 65C816-specific: XCE (mode switching) ───────────────────────────────

#[test]
fn xce_to_native_mode() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    assert!(cpu.emulation);
    // CLC + XCE = switch to native mode
    poke(&mut bus, 0x0200, &[0x18, 0xFB]); // CLC, XCE
    step(&mut cpu, &mut bus); // CLC
    step(&mut cpu, &mut bus); // XCE
    assert!(!cpu.emulation);
    assert!(cpu.flags.contains(Flags816::C)); // old E flag was 1 → C=1
}

#[test]
fn xce_to_emulation_mode() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.emulation = false;
    cpu.flags.remove(Flags816::C);
    // SEC + XCE = switch to emulation mode
    poke(&mut bus, 0x0200, &[0x38, 0xFB]); // SEC, XCE
    step(&mut cpu, &mut bus); // SEC
    step(&mut cpu, &mut bus); // XCE
    assert!(cpu.emulation);
    assert!(!cpu.flags.contains(Flags816::C)); // old E was 0 → C=0
    // M and X should be forced to 1 in emulation mode
    assert!(cpu.flags.contains(Flags816::M));
    assert!(cpu.flags.contains(Flags816::X));
}

// ── 65C816-specific: REP/SEP ────────────────────────────────────────────

#[test]
fn rep_clears_flags() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.emulation = false;
    cpu.flags = Flags816::M | Flags816::X | Flags816::I | Flags816::C;
    poke(&mut bus, 0x0200, &[0xC2, 0x30]); // REP #$30 (clear M and X)
    step(&mut cpu, &mut bus);
    assert!(!cpu.flags.contains(Flags816::M)); // 16-bit accumulator
    assert!(!cpu.flags.contains(Flags816::X)); // 16-bit index
}

#[test]
fn sep_sets_flags() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.emulation = false;
    cpu.flags = Flags816::I;
    poke(&mut bus, 0x0200, &[0xE2, 0x30]); // SEP #$30 (set M and X)
    step(&mut cpu, &mut bus);
    assert!(cpu.flags.contains(Flags816::M)); // 8-bit accumulator
    assert!(cpu.flags.contains(Flags816::X)); // 8-bit index
}

#[test]
fn rep_in_emulation_mode_cannot_clear_mx() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    assert!(cpu.emulation);
    cpu.flags = Flags816::M | Flags816::X | Flags816::I;
    poke(&mut bus, 0x0200, &[0xC2, 0x30]); // REP #$30
    step(&mut cpu, &mut bus);
    // In emulation mode, M and X are forced to 1
    assert!(cpu.flags.contains(Flags816::M));
    assert!(cpu.flags.contains(Flags816::X));
}

// ── Native mode: 16-bit operations ──────────────────────────────────────

#[test]
fn lda_immediate_16bit() {
    let mut bus = TestBus::new();
    let mut cpu = make_native_cpu(0x0200);
    poke(&mut bus, 0x0200, &[0xA9, 0x34, 0x12]); // LDA #$1234
    let cycles = step(&mut cpu, &mut bus);
    assert_eq!(cpu.c, 0x1234);
    assert_eq!(cpu.pc, 0x0203);
    assert_eq!(cycles, 3); // base 2 + 1 for 16-bit
}

#[test]
fn adc_16bit() {
    let mut bus = TestBus::new();
    let mut cpu = make_native_cpu(0x0200);
    cpu.c = 0x1000;
    cpu.flags.remove(Flags816::C);
    poke(&mut bus, 0x0200, &[0x69, 0x34, 0x12]); // ADC #$1234
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c, 0x2234);
}

#[test]
fn adc_16bit_carry() {
    let mut bus = TestBus::new();
    let mut cpu = make_native_cpu(0x0200);
    cpu.c = 0xFFFF;
    cpu.flags.remove(Flags816::C);
    poke(&mut bus, 0x0200, &[0x69, 0x01, 0x00]); // ADC #$0001
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c, 0x0000);
    assert!(cpu.flags.contains(Flags816::C));
    assert!(cpu.flags.contains(Flags816::Z));
}

#[test]
fn sta_16bit() {
    let mut bus = TestBus::new();
    let mut cpu = make_native_cpu(0x0200);
    cpu.c = 0xABCD;
    poke(&mut bus, 0x0200, &[0x8D, 0x00, 0x10]); // STA $1000
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0xCD); // low byte
    assert_eq!(peek(&bus, 0x1001), 0xAB); // high byte
}

#[test]
fn ldx_16bit() {
    let mut bus = TestBus::new();
    let mut cpu = make_native_cpu(0x0200);
    poke(&mut bus, 0x0200, &[0xA2, 0xEF, 0xBE]); // LDX #$BEEF
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.x, 0xBEEF);
}

// ── Addressing modes ────────────────────────────────────────────────────

#[test]
fn lda_direct_page() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    poke(&mut bus, 0x0050, &[0x42]); // value at DP $50
    poke(&mut bus, 0x0200, &[0xA5, 0x50]); // LDA $50
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x42);
}

#[test]
fn lda_absolute_x() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.x = 0x05;
    poke(&mut bus, 0x1005, &[0x77]); // value at $1005
    poke(&mut bus, 0x0200, &[0xBD, 0x00, 0x10]); // LDA $1000,X
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x77);
}

#[test]
fn lda_indirect_y() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.y = 0x03;
    poke(&mut bus, 0x0040, &[0x00, 0x10]); // pointer at DP $40 → $1000
    poke(&mut bus, 0x1003, &[0x99]); // value at $1000+3
    poke(&mut bus, 0x0200, &[0xB1, 0x40]); // LDA ($40),Y
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0x99);
}

#[test]
fn lda_stack_relative() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.emulation = false;
    cpu.flags = Flags816::M | Flags816::X | Flags816::I;
    cpu.sp = 0x01F0;
    poke(&mut bus, 0x01F3, &[0xAB]); // value at SP+3
    poke(&mut bus, 0x0200, &[0xA3, 0x03]); // LDA $03,S
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0xAB);
}

#[test]
fn lda_absolute_long() {
    let mut bus = TestBus::new_large();
    let mut cpu = make_cpu(0x0200);
    poke(&mut bus, 0x02_3456, &[0xEE]); // value at $02:3456
    poke(&mut bus, 0x0200, &[0xAF, 0x56, 0x34, 0x02]); // LDA $023456
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0xEE);
}

// ── Block moves ─────────────────────────────────────────────────────────

#[test]
fn mvn_copies_forward() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.emulation = false;
    cpu.flags = Flags816::I | Flags816::M; // 8-bit A, 16-bit X/Y
    cpu.flags.remove(Flags816::X);
    // Source: $0300-$0302, Dest: $0400-$0402
    poke(&mut bus, 0x0300, &[0xAA, 0xBB, 0xCC]);
    cpu.x = 0x0300; // source start
    cpu.y = 0x0400; // dest start
    cpu.c = 0x0002; // count - 1 (3 bytes)
    // MVN dst_bank, src_bank
    poke(&mut bus, 0x0200, &[0x54, 0x00, 0x00]); // MVN $00,$00
    // Execute 3 iterations (one per byte)
    step(&mut cpu, &mut bus);
    step(&mut cpu, &mut bus);
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x0400), 0xAA);
    assert_eq!(peek(&bus, 0x0401), 0xBB);
    assert_eq!(peek(&bus, 0x0402), 0xCC);
}

// ── Interrupt handling ──────────────────────────────────────────────────

#[test]
fn brk_emulation_mode() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    poke(&mut bus, 0xFFFE, &[0x00, 0x10]); // BRK vector → $1000
    poke(&mut bus, 0x0200, &[0x00, 0x00]); // BRK (+ signature byte)
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x1000);
    assert!(cpu.flags.contains(Flags816::I)); // IRQ disable set
}

#[test]
fn brk_native_mode() {
    let mut bus = TestBus::new();
    let mut cpu = make_native_cpu(0x0200);
    cpu.flags.insert(Flags816::M | Flags816::X);
    cpu.pbr = 0x00;
    poke(&mut bus, 0xFFE6, &[0x00, 0x10]); // native BRK vector → $1000
    poke(&mut bus, 0x0200, &[0x00, 0x00]); // BRK
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x1000);
    assert_eq!(cpu.pbr, 0x00); // PBR cleared on interrupt
}

#[test]
fn rti_emulation_mode() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    // Push return state: flags=$30, PC=$1234
    cpu.push8(&mut bus, 0x12); // PC high
    cpu.push8(&mut bus, 0x34); // PC low
    cpu.push8(&mut bus, 0x30); // P register
    poke(&mut bus, 0x0200, &[0x40]); // RTI
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x1234);
    assert_eq!(cpu.flags.bits() & 0x30, 0x30); // M and X forced in emu mode
}

// ── WAI / STP ───────────────────────────────────────────────────────────

#[test]
fn wai_halts_cpu() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    poke(&mut bus, 0x0200, &[0xCB]); // WAI
    step(&mut cpu, &mut bus);
    assert!(cpu.waiting);
}

#[test]
fn stp_stops_cpu() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    poke(&mut bus, 0x0200, &[0xDB]); // STP
    step(&mut cpu, &mut bus);
    assert!(cpu.stopped);
}

// ── PHB/PLB, PHD/PLD, PHK ───────────────────────────────────────────────

#[test]
fn phb_plb_roundtrip() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.dbr = 0x42;
    poke(&mut bus, 0x0200, &[0x8B]); // PHB
    step(&mut cpu, &mut bus);
    cpu.dbr = 0x00;
    poke(&mut bus, 0x0201, &[0xAB]); // PLB
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.dbr, 0x42);
}

#[test]
fn phd_pld_roundtrip() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.emulation = false;
    cpu.flags = Flags816::M | Flags816::X | Flags816::I;
    cpu.dp = 0x1234;
    poke(&mut bus, 0x0200, &[0x0B]); // PHD
    step(&mut cpu, &mut bus);
    cpu.dp = 0x0000;
    poke(&mut bus, 0x0201, &[0x2B]); // PLD
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.dp, 0x1234);
}

#[test]
fn phk_pushes_pbr() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.pbr = 0x03;
    poke(&mut bus, 0x0200, &[0x4B]); // PHK
    let sp_before = cpu.sp;
    step(&mut cpu, &mut bus);
    let pushed = peek(&bus, sp_before as u32);
    assert_eq!(pushed, 0x03);
}

// ── TCS/TSC, TCD/TDC ───────────────────────────────────────────────────

#[test]
fn tcs_transfers_c_to_sp() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.emulation = false;
    cpu.flags = Flags816::M | Flags816::X | Flags816::I;
    cpu.c = 0x1FF0;
    poke(&mut bus, 0x0200, &[0x1B]); // TCS
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.sp, 0x1FF0);
}

#[test]
fn tcd_transfers_c_to_dp() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0x3000;
    poke(&mut bus, 0x0200, &[0x5B]); // TCD
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.dp, 0x3000);
}

// ── NOP ─────────────────────────────────────────────────────────────────

#[test]
fn nop_advances_pc() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    poke(&mut bus, 0x0200, &[0xEA]); // NOP
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0201);
}

// ── Integration: counting loop ──────────────────────────────────────────

#[test]
fn counting_loop() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    // Program: count from 0 to 5 then halt
    // LDA #$00; INX; CPX #$05; BNE -3; STP
    #[rustfmt::skip]
        poke(&mut bus, 0x0200, &[
            0xA2, 0x00,       // LDX #$00
            0xE8,             // loop: INX
            0xE0, 0x05,       // CPX #$05
            0xD0, 0xFB,       // BNE loop (-5 from BNE+2 = 0x0202)
            0xDB,             // STP
        ]);
    // Run up to 100 instructions
    for _ in 0..100 {
        if cpu.stopped {
            break;
        }
        step(&mut cpu, &mut bus);
    }
    assert!(cpu.stopped);
    assert_eq!(cpu.x & 0xFF, 0x05);
}

// ── BRL (branch long) ───────────────────────────────────────────────────

#[test]
fn brl_long_branch() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    // BRL $0100 (16-bit relative offset)
    poke(&mut bus, 0x0200, &[0x82, 0x00, 0x01]); // BRL +$0100
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0303); // $0203 + $0100
}

// ── PEA (Push Effective Absolute) ───────────────────────────────────────

#[test]
fn pea_pushes_16bit() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.emulation = false;
    cpu.flags = Flags816::M | Flags816::X | Flags816::I;
    poke(&mut bus, 0x0200, &[0xF4, 0x34, 0x12]); // PEA $1234
    let sp_before = cpu.sp;
    step(&mut cpu, &mut bus);
    let lo = peek(&bus, (sp_before.wrapping_sub(1)) as u32);
    let hi = peek(&bus, (sp_before) as u32);
    assert_eq!((hi as u16) << 8 | lo as u16, 0x1234);
}

// ── Direct Page relocation ──────────────────────────────────────────────

#[test]
fn direct_page_relocated() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.dp = 0x1000; // relocate direct page to $1000
    poke(&mut bus, 0x1050, &[0xBB]); // value at DP $50 → $1050
    poke(&mut bus, 0x0200, &[0xA5, 0x50]); // LDA $50
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c & 0xFF, 0xBB);
}

// ── Emulation mode stack wraps in page 1 ────────────────────────────────

#[test]
fn emulation_mode_stack_wraps_page1() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    assert!(cpu.emulation);
    cpu.sp = 0x0100; // stack at bottom of page 1
    cpu.c = 0x42;
    poke(&mut bus, 0x0200, &[0x48]); // PHA
    step(&mut cpu, &mut bus);
    // SP should wrap within page 1
    assert_eq!(cpu.sp & 0xFF00, 0x0100);
    assert_eq!(cpu.sp & 0xFF, 0xFF);
    assert_eq!(peek(&bus, 0x0100), 0x42); // pushed to $0100
}

// ── Index register truncation on entering emulation mode ────────────────

#[test]
fn entering_emulation_truncates_index_regs() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.emulation = false;
    cpu.flags = Flags816::I; // M=0, X=0 → 16-bit
    cpu.x = 0x1234;
    cpu.y = 0xABCD;
    // SEC + XCE → enter emulation mode
    poke(&mut bus, 0x0200, &[0x38, 0xFB]); // SEC, XCE
    step(&mut cpu, &mut bus); // SEC
    step(&mut cpu, &mut bus); // XCE
    assert!(cpu.emulation);
    assert_eq!(cpu.x, 0x34); // high byte cleared
    assert_eq!(cpu.y, 0xCD); // high byte cleared
}

// ── SEP sets X flag → high bytes of X/Y cleared ────────────────────────

#[test]
fn sep_x_flag_clears_index_high_bytes() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.emulation = false;
    cpu.flags = Flags816::I; // M=0, X=0 → 16-bit
    cpu.x = 0x1234;
    cpu.y = 0x5678;
    poke(&mut bus, 0x0200, &[0xE2, 0x10]); // SEP #$10 (set X flag → 8-bit index)
    step(&mut cpu, &mut bus);
    assert!(cpu.flags.contains(Flags816::X));
    assert_eq!(cpu.x, 0x34); // high byte cleared when X goes 8-bit
    assert_eq!(cpu.y, 0x78);
}

// ── CMP 16-bit ──────────────────────────────────────────────────────────

#[test]
fn cmp_16bit_equal() {
    let mut bus = TestBus::new();
    let mut cpu = make_native_cpu(0x0200);
    cpu.c = 0x1234;
    poke(&mut bus, 0x0200, &[0xC9, 0x34, 0x12]); // CMP #$1234
    step(&mut cpu, &mut bus);
    assert!(cpu.flags.contains(Flags816::Z));
    assert!(cpu.flags.contains(Flags816::C));
}

// ── TSB / TRB ───────────────────────────────────────────────────────────

#[test]
fn tsb_sets_bits() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0x0F;
    poke(&mut bus, 0x50, &[0xF0]); // memory value
    poke(&mut bus, 0x0200, &[0x04, 0x50]); // TSB $50
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x50), 0xFF); // $F0 | $0F = $FF
    assert!(cpu.flags.contains(Flags816::Z)); // ($0F & $F0) == 0
}

#[test]
fn trb_resets_bits() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.c = 0x0F;
    poke(&mut bus, 0x50, &[0xFF]); // memory value
    poke(&mut bus, 0x0200, &[0x14, 0x50]); // TRB $50
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x50), 0xF0); // $FF & ~$0F = $F0
    assert!(!cpu.flags.contains(Flags816::Z)); // ($0F & $FF) != 0
}

// ── STZ (store zero) ────────────────────────────────────────────────────

#[test]
fn stz_absolute() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    poke(&mut bus, 0x1000, &[0xFF]); // non-zero
    poke(&mut bus, 0x0200, &[0x9C, 0x00, 0x10]); // STZ $1000
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0x00);
}

// ── INC / DEC memory ────────────────────────────────────────────────────

#[test]
fn inc_memory_absolute() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    poke(&mut bus, 0x1000, &[0xFF]);
    poke(&mut bus, 0x0200, &[0xEE, 0x00, 0x10]); // INC $1000
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0x00);
    assert!(cpu.flags.contains(Flags816::Z));
}

#[test]
fn dec_memory_absolute() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    poke(&mut bus, 0x1000, &[0x00]);
    poke(&mut bus, 0x0200, &[0xCE, 0x00, 0x10]); // DEC $1000
    step(&mut cpu, &mut bus);
    assert_eq!(peek(&bus, 0x1000), 0xFF);
    assert!(cpu.flags.contains(Flags816::N));
}

// ── JMP indirect ────────────────────────────────────────────────────────

#[test]
fn jmp_indirect() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    poke(&mut bus, 0x3000, &[0x00, 0x10]); // pointer → $1000
    poke(&mut bus, 0x0200, &[0x6C, 0x00, 0x30]); // JMP ($3000)
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x1000);
}

// ── PHP / PLP roundtrip ─────────────────────────────────────────────────

#[test]
fn php_plp_roundtrip() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    cpu.flags = Flags816::N | Flags816::C | Flags816::M | Flags816::X;
    poke(&mut bus, 0x0200, &[0x08]); // PHP
    step(&mut cpu, &mut bus);
    cpu.flags = Flags816::empty();
    poke(&mut bus, 0x0201, &[0x28]); // PLP
    step(&mut cpu, &mut bus);
    assert!(cpu.flags.contains(Flags816::N));
    assert!(cpu.flags.contains(Flags816::C));
}

// ── SBC 16-bit ──────────────────────────────────────────────────────────

#[test]
fn sbc_16bit() {
    let mut bus = TestBus::new();
    let mut cpu = make_native_cpu(0x0200);
    cpu.c = 0x5000;
    cpu.flags.insert(Flags816::C); // no borrow
    poke(&mut bus, 0x0200, &[0xE9, 0x00, 0x10]); // SBC #$1000
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.c, 0x4000);
    assert!(cpu.flags.contains(Flags816::C)); // no borrow
}

// ── COP instruction ────────────────────────────────────────────────────

#[test]
fn cop_emulation_mode() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    poke(&mut bus, 0xFFF4, &[0x00, 0x08]); // COP vector → $0800
    poke(&mut bus, 0x0200, &[0x02, 0x00]); // COP #$00
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0800);
}

// ── WDM (reserved, 2-byte NOP) ──────────────────────────────────────────

#[test]
fn wdm_skips_byte() {
    let mut bus = TestBus::new();
    let mut cpu = make_cpu(0x0200);
    poke(&mut bus, 0x0200, &[0x42, 0xFF]); // WDM $FF
    step(&mut cpu, &mut bus);
    assert_eq!(cpu.pc, 0x0202); // skipped the signature byte
}
