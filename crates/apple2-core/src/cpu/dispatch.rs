//! CPU opcode dispatch tables.
//!
//! Each entry is a function that executes one instruction and returns the
//! number of *additional* cycles beyond the base count encoded in CYCLES_6502.
//!
//! The split into 6502 vs 65C02 tables mirrors the two execution paths in
//! `source/CPU/cpu6502.h` and `source/CPU/cpu65C02.h`.

use super::cpu6502::Cpu;
use super::flags::Flags;
use crate::bus::Bus;

/// Signature for an opcode handler.
pub type OpFn = fn(&mut Cpu, &mut Bus) -> u8;

/// Base cycle counts for all 256 opcodes (6502/65C02 share most values).
/// Extra cycles for page crosses, branches taken, etc. are returned by the handler.
#[rustfmt::skip]
pub const CYCLES_6502: [u8; 256] = [
//  0  1  2  3  4  5  6  7  8  9  A  B  C  D  E  F
    7, 6, 2, 8, 3, 3, 5, 5, 3, 2, 2, 2, 4, 4, 6, 6,  // 0x
    2, 5, 5, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7,  // 1x
    6, 6, 2, 8, 3, 3, 5, 5, 4, 2, 2, 2, 4, 4, 6, 6,  // 2x
    2, 5, 5, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7,  // 3x
    6, 6, 2, 8, 3, 3, 5, 5, 3, 2, 2, 2, 3, 4, 6, 6,  // 4x
    2, 5, 5, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7,  // 5x
    6, 6, 2, 8, 3, 3, 5, 5, 4, 2, 2, 2, 5, 4, 6, 6,  // 6x
    2, 5, 5, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7,  // 7x
    2, 6, 2, 6, 3, 3, 3, 3, 2, 2, 2, 2, 4, 4, 4, 4,  // 8x
    2, 6, 5, 6, 4, 4, 4, 4, 2, 5, 2, 5, 5, 5, 5, 5,  // 9x
    2, 6, 2, 6, 3, 3, 3, 3, 2, 2, 2, 2, 4, 4, 4, 4,  // Ax
    2, 5, 5, 5, 4, 4, 4, 4, 2, 4, 2, 4, 4, 4, 4, 4,  // Bx
    2, 6, 2, 8, 3, 3, 5, 5, 2, 2, 2, 2, 4, 4, 6, 6,  // Cx
    2, 5, 5, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7,  // Dx
    2, 6, 2, 8, 3, 3, 5, 5, 2, 2, 2, 2, 4, 4, 6, 6,  // Ex
    2, 5, 5, 8, 4, 4, 6, 6, 2, 4, 2, 7, 4, 4, 7, 7,  // Fx
];

// ── Opcode handler helpers ────────────────────────────────────────────────────

/// Execute one instruction.  Returns total cycles consumed.
pub fn step(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    // Check for pending NMI first, then IRQ
    if cpu.nmi_pending != 0 {
        cpu.nmi_pending = 0;
        cpu.service_interrupt(bus, true);
        cpu.cycles += 7;
        return 7;
    }
    if cpu.irq_pending != 0 && !cpu.flags.contains(Flags::I) {
        cpu.service_interrupt(bus, false);
        cpu.cycles += 7;
        return 7;
    }

    let opcode = bus.read(cpu.pc, cpu.cycles);
    cpu.pc = cpu.pc.wrapping_add(1);

    let base = CYCLES_6502[opcode as usize];
    let extra = if cpu.is_65c02 {
        DISPATCH_65C02[opcode as usize](cpu, bus)
    } else {
        DISPATCH_6502[opcode as usize](cpu, bus)
    };

    let total = base + extra;
    cpu.cycles += total as u64;
    total
}

// ── Shared instruction implementations ───────────────────────────────────────
// These are correct for both 6502 and 65C02 unless overridden in the 65C02 table.

fn op_brk(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    cpu.pc = cpu.pc.wrapping_add(1); // skip padding byte
    cpu.push(bus, (cpu.pc >> 8) as u8);
    cpu.push(bus, cpu.pc as u8);
    let p = cpu.flags.bits() | Flags::B.bits() | Flags::U.bits();
    cpu.push(bus, p);
    cpu.flags.insert(Flags::I);
    if cpu.is_65c02 {
        cpu.flags.remove(Flags::D);
    }
    let lo = bus.read_raw(0xFFFE) as u16;
    let hi = bus.read_raw(0xFFFF) as u16;
    cpu.pc = (hi << 8) | lo;
    0
}

fn op_ora_indx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_indx(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a |= val;
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_nop(cpu: &mut Cpu, _bus: &mut Bus) -> u8 { let _ = cpu; 0 }

fn op_ora_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a |= val;
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_asl_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_asl(val);
    bus.write(ea, result, cpu.cycles);
    0
}

fn op_php(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let p = cpu.flags.bits() | Flags::B.bits() | Flags::U.bits();
    cpu.push(bus, p);
    0
}

fn op_ora_imm(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let val = cpu.addr_imm(bus);
    cpu.a |= val;
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_asl_a(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    let a = cpu.a;
    cpu.a = cpu.op_asl(a);
    0
}

fn op_ora_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a |= val;
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_asl_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_asl(val);
    bus.write(ea, result, cpu.cycles);
    0
}

fn op_bpl(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let taken = !cpu.flags.contains(Flags::N);
    cpu.branch_target(bus, taken)
}

fn op_ora_indy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_indy(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a |= val;
    cpu.flags.set_nz(cpu.a);
    cross as u8
}

fn op_ora_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a |= val;
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_asl_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_asl(val);
    bus.write(ea, result, cpu.cycles);
    0
}

fn op_clc(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.flags.remove(Flags::C);
    0
}

fn op_ora_absy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_absy(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a |= val;
    cpu.flags.set_nz(cpu.a);
    cross as u8
}

fn op_ora_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_absx(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a |= val;
    cpu.flags.set_nz(cpu.a);
    cross as u8
}

fn op_asl_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_absx(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_asl(val);
    bus.write(ea, result, cpu.cycles);
    0
}

fn op_jsr(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let target = cpu.addr_abs(bus);
    let ret = cpu.pc.wrapping_sub(1);
    cpu.push(bus, (ret >> 8) as u8);
    cpu.push(bus, ret as u8);
    cpu.pc = target;
    0
}

fn op_and_indx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_indx(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a &= val;
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_bit_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_bit(val);
    0
}

fn op_and_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a &= val;
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_rol_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_rol(val);
    bus.write(ea, result, cpu.cycles);
    0
}

fn op_plp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let p = cpu.pop(bus);
    cpu.flags = Flags::from_bits_truncate(p) | Flags::U;
    0
}

fn op_and_imm(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let val = cpu.addr_imm(bus);
    cpu.a &= val;
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_rol_a(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    let a = cpu.a;
    cpu.a = cpu.op_rol(a);
    0
}

fn op_bit_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_bit(val);
    0
}

fn op_and_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a &= val;
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_rol_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_rol(val);
    bus.write(ea, result, cpu.cycles);
    0
}

fn op_bmi(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let taken = cpu.flags.contains(Flags::N);
    cpu.branch_target(bus, taken)
}

fn op_and_indy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_indy(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a &= val;
    cpu.flags.set_nz(cpu.a);
    cross as u8
}

fn op_and_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a &= val;
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_rol_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_rol(val);
    bus.write(ea, result, cpu.cycles);
    0
}

fn op_sec(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.flags.insert(Flags::C);
    0
}

fn op_and_absy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_absy(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a &= val;
    cpu.flags.set_nz(cpu.a);
    cross as u8
}

fn op_and_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_absx(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a &= val;
    cpu.flags.set_nz(cpu.a);
    cross as u8
}

fn op_rol_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_absx(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_rol(val);
    bus.write(ea, result, cpu.cycles);
    0
}

fn op_rti(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let p = cpu.pop(bus);
    cpu.flags = Flags::from_bits_truncate(p) | Flags::U;
    let lo = cpu.pop(bus) as u16;
    let hi = cpu.pop(bus) as u16;
    cpu.pc = (hi << 8) | lo;
    0
}

fn op_eor_indx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_indx(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a ^= val;
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_eor_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a ^= val;
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_lsr_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_lsr(val);
    bus.write(ea, result, cpu.cycles);
    0
}

fn op_pha(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let a = cpu.a;
    cpu.push(bus, a);
    0
}

fn op_eor_imm(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let val = cpu.addr_imm(bus);
    cpu.a ^= val;
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_lsr_a(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    let a = cpu.a;
    cpu.a = cpu.op_lsr(a);
    0
}

fn op_jmp_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    cpu.pc = cpu.addr_abs(bus);
    0
}

fn op_eor_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a ^= val;
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_lsr_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_lsr(val);
    bus.write(ea, result, cpu.cycles);
    0
}

fn op_bvc(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let taken = !cpu.flags.contains(Flags::V);
    cpu.branch_target(bus, taken)
}

fn op_eor_indy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_indy(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a ^= val;
    cpu.flags.set_nz(cpu.a);
    cross as u8
}

fn op_eor_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a ^= val;
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_lsr_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_lsr(val);
    bus.write(ea, result, cpu.cycles);
    0
}

fn op_cli(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.flags.remove(Flags::I);
    0
}

fn op_eor_absy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_absy(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a ^= val;
    cpu.flags.set_nz(cpu.a);
    cross as u8
}

fn op_eor_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_absx(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a ^= val;
    cpu.flags.set_nz(cpu.a);
    cross as u8
}

fn op_lsr_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_absx(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_lsr(val);
    bus.write(ea, result, cpu.cycles);
    0
}

fn op_rts(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let lo = cpu.pop(bus) as u16;
    let hi = cpu.pop(bus) as u16;
    cpu.pc = ((hi << 8) | lo).wrapping_add(1);
    0
}

fn op_adc_indx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_indx(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_adc(val);
    0
}

fn op_adc_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_adc(val);
    0
}

fn op_ror_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_ror(val);
    bus.write(ea, result, cpu.cycles);
    0
}

fn op_pla(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    cpu.a = cpu.pop(bus);
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_adc_imm(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let val = cpu.addr_imm(bus);
    cpu.op_adc(val);
    0
}

fn op_ror_a(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    let a = cpu.a;
    cpu.a = cpu.op_ror(a);
    0
}

fn op_jmp_ind(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    // NMOS 6502 bug: JMP ($xxFF) reads from $xxFF and $xx00 (not $xx+1:00)
    let ptr = cpu.addr_abs(bus);
    let lo = bus.read(ptr, cpu.cycles) as u16;
    // NMOS page-wrap bug
    let hi_addr = if !cpu.is_65c02 && ptr & 0xFF == 0xFF {
        ptr & 0xFF00
    } else {
        ptr.wrapping_add(1)
    };
    let hi = bus.read(hi_addr, cpu.cycles) as u16;
    cpu.pc = (hi << 8) | lo;
    0
}

fn op_adc_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_adc(val);
    0
}

fn op_ror_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_ror(val);
    bus.write(ea, result, cpu.cycles);
    0
}

fn op_bvs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let taken = cpu.flags.contains(Flags::V);
    cpu.branch_target(bus, taken)
}

fn op_adc_indy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_indy(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_adc(val);
    cross as u8
}

fn op_adc_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_adc(val);
    0
}

fn op_ror_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_ror(val);
    bus.write(ea, result, cpu.cycles);
    0
}

fn op_sei(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.flags.insert(Flags::I);
    0
}

fn op_adc_absy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_absy(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_adc(val);
    cross as u8
}

fn op_adc_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_absx(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_adc(val);
    cross as u8
}

fn op_ror_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_absx(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_ror(val);
    bus.write(ea, result, cpu.cycles);
    0
}

fn op_sta_indx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_indx(bus);
    bus.write(ea, cpu.a, cpu.cycles);
    0
}

fn op_sty_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    bus.write(ea, cpu.y, cpu.cycles);
    0
}

fn op_sta_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    bus.write(ea, cpu.a, cpu.cycles);
    0
}

fn op_stx_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    bus.write(ea, cpu.x, cpu.cycles);
    0
}

fn op_dey(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.y = cpu.y.wrapping_sub(1);
    cpu.flags.set_nz(cpu.y);
    0
}

fn op_txa(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.a = cpu.x;
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_sty_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    bus.write(ea, cpu.y, cpu.cycles);
    0
}

fn op_sta_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    bus.write(ea, cpu.a, cpu.cycles);
    0
}

fn op_stx_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    bus.write(ea, cpu.x, cpu.cycles);
    0
}

fn op_bcc(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let taken = !cpu.flags.contains(Flags::C);
    cpu.branch_target(bus, taken)
}

fn op_sta_indy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_indy(bus);
    bus.write(ea, cpu.a, cpu.cycles);
    0
}

fn op_sty_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    bus.write(ea, cpu.y, cpu.cycles);
    0
}

fn op_sta_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    bus.write(ea, cpu.a, cpu.cycles);
    0
}

fn op_stx_zpy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpy(bus);
    bus.write(ea, cpu.x, cpu.cycles);
    0
}

fn op_tya(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.a = cpu.y;
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_sta_absy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_absy(bus);
    bus.write(ea, cpu.a, cpu.cycles);
    0
}

fn op_txs(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.sp = cpu.x;
    0
}

fn op_sta_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_absx(bus);
    bus.write(ea, cpu.a, cpu.cycles);
    0
}

fn op_ldy_imm(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    cpu.y = cpu.addr_imm(bus);
    cpu.flags.set_nz(cpu.y);
    0
}

fn op_lda_indx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_indx(bus);
    cpu.a = bus.read(ea, cpu.cycles);
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_ldx_imm(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    cpu.x = cpu.addr_imm(bus);
    cpu.flags.set_nz(cpu.x);
    0
}

fn op_ldy_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    cpu.y = bus.read(ea, cpu.cycles);
    cpu.flags.set_nz(cpu.y);
    0
}

fn op_lda_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    cpu.a = bus.read(ea, cpu.cycles);
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_ldx_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    cpu.x = bus.read(ea, cpu.cycles);
    cpu.flags.set_nz(cpu.x);
    0
}

fn op_tay(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.y = cpu.a;
    cpu.flags.set_nz(cpu.y);
    0
}

fn op_lda_imm(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    cpu.a = cpu.addr_imm(bus);
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_tax(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.x = cpu.a;
    cpu.flags.set_nz(cpu.x);
    0
}

fn op_ldy_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    cpu.y = bus.read(ea, cpu.cycles);
    cpu.flags.set_nz(cpu.y);
    0
}

fn op_lda_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    cpu.a = bus.read(ea, cpu.cycles);
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_ldx_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    cpu.x = bus.read(ea, cpu.cycles);
    cpu.flags.set_nz(cpu.x);
    0
}

fn op_bcs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let taken = cpu.flags.contains(Flags::C);
    cpu.branch_target(bus, taken)
}

fn op_lda_indy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_indy(bus);
    cpu.a = bus.read(ea, cpu.cycles);
    cpu.flags.set_nz(cpu.a);
    cross as u8
}

fn op_ldy_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    cpu.y = bus.read(ea, cpu.cycles);
    cpu.flags.set_nz(cpu.y);
    0
}

fn op_lda_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    cpu.a = bus.read(ea, cpu.cycles);
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_ldx_zpy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpy(bus);
    cpu.x = bus.read(ea, cpu.cycles);
    cpu.flags.set_nz(cpu.x);
    0
}

fn op_clv(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.flags.remove(Flags::V);
    0
}

fn op_lda_absy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_absy(bus);
    cpu.a = bus.read(ea, cpu.cycles);
    cpu.flags.set_nz(cpu.a);
    cross as u8
}

fn op_tsx(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.x = cpu.sp;
    cpu.flags.set_nz(cpu.x);
    0
}

fn op_ldy_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_absx(bus);
    cpu.y = bus.read(ea, cpu.cycles);
    cpu.flags.set_nz(cpu.y);
    cross as u8
}

fn op_lda_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_absx(bus);
    cpu.a = bus.read(ea, cpu.cycles);
    cpu.flags.set_nz(cpu.a);
    cross as u8
}

fn op_ldx_absy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_absy(bus);
    cpu.x = bus.read(ea, cpu.cycles);
    cpu.flags.set_nz(cpu.x);
    cross as u8
}

fn op_cpy_imm(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let val = cpu.addr_imm(bus);
    cpu.op_cmp(cpu.y, val);
    0
}

fn op_cmp_indx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_indx(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_cmp(cpu.a, val);
    0
}

fn op_cpy_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_cmp(cpu.y, val);
    0
}

fn op_cmp_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_cmp(cpu.a, val);
    0
}

fn op_dec_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_sub(1);
    cpu.flags.set_nz(val);
    bus.write(ea, val, cpu.cycles);
    0
}

fn op_iny(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.y = cpu.y.wrapping_add(1);
    cpu.flags.set_nz(cpu.y);
    0
}

fn op_cmp_imm(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let val = cpu.addr_imm(bus);
    cpu.op_cmp(cpu.a, val);
    0
}

fn op_dex(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.x = cpu.x.wrapping_sub(1);
    cpu.flags.set_nz(cpu.x);
    0
}

fn op_cpy_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_cmp(cpu.y, val);
    0
}

fn op_cmp_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_cmp(cpu.a, val);
    0
}

fn op_dec_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_sub(1);
    cpu.flags.set_nz(val);
    bus.write(ea, val, cpu.cycles);
    0
}

fn op_bne(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let taken = !cpu.flags.contains(Flags::Z);
    cpu.branch_target(bus, taken)
}

fn op_cmp_indy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_indy(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_cmp(cpu.a, val);
    cross as u8
}

fn op_cmp_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_cmp(cpu.a, val);
    0
}

fn op_dec_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_sub(1);
    cpu.flags.set_nz(val);
    bus.write(ea, val, cpu.cycles);
    0
}

fn op_cld(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.flags.remove(Flags::D);
    0
}

fn op_cmp_absy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_absy(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_cmp(cpu.a, val);
    cross as u8
}

fn op_cmp_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_absx(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_cmp(cpu.a, val);
    cross as u8
}

fn op_dec_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_absx(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_sub(1);
    cpu.flags.set_nz(val);
    bus.write(ea, val, cpu.cycles);
    0
}

fn op_cpx_imm(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let val = cpu.addr_imm(bus);
    cpu.op_cmp(cpu.x, val);
    0
}

fn op_sbc_indx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_indx(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_sbc(val);
    0
}

fn op_cpx_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_cmp(cpu.x, val);
    0
}

fn op_sbc_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_sbc(val);
    0
}

fn op_inc_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_add(1);
    cpu.flags.set_nz(val);
    bus.write(ea, val, cpu.cycles);
    0
}

fn op_inx(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.x = cpu.x.wrapping_add(1);
    cpu.flags.set_nz(cpu.x);
    0
}

fn op_sbc_imm(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let val = cpu.addr_imm(bus);
    cpu.op_sbc(val);
    0
}

fn op_cpx_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_cmp(cpu.x, val);
    0
}

fn op_sbc_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_sbc(val);
    0
}

fn op_inc_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_add(1);
    cpu.flags.set_nz(val);
    bus.write(ea, val, cpu.cycles);
    0
}

fn op_beq(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let taken = cpu.flags.contains(Flags::Z);
    cpu.branch_target(bus, taken)
}

fn op_sbc_indy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_indy(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_sbc(val);
    cross as u8
}

fn op_sbc_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_sbc(val);
    0
}

fn op_inc_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_add(1);
    cpu.flags.set_nz(val);
    bus.write(ea, val, cpu.cycles);
    0
}

fn op_sed(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.flags.insert(Flags::D);
    0
}

fn op_sbc_absy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_absy(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_sbc(val);
    cross as u8
}

fn op_sbc_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_absx(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_sbc(val);
    cross as u8
}

fn op_inc_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_absx(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_add(1);
    cpu.flags.set_nz(val);
    bus.write(ea, val, cpu.cycles);
    0
}

// ── 65C02-only opcodes ────────────────────────────────────────────────────────

fn op_tsb_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.flags.set(Flags::Z, cpu.a & val == 0);
    bus.write(ea, val | cpu.a, cpu.cycles);
    0
}

fn op_tsb_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.flags.set(Flags::Z, cpu.a & val == 0);
    bus.write(ea, val | cpu.a, cpu.cycles);
    0
}

fn op_trb_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.flags.set(Flags::Z, cpu.a & val == 0);
    bus.write(ea, val & !cpu.a, cpu.cycles);
    0
}

fn op_trb_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.flags.set(Flags::Z, cpu.a & val == 0);
    bus.write(ea, val & !cpu.a, cpu.cycles);
    0
}

fn op_stz_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    bus.write(ea, 0, cpu.cycles);
    0
}

fn op_stz_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    bus.write(ea, 0, cpu.cycles);
    0
}

fn op_stz_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    bus.write(ea, 0, cpu.cycles);
    0
}

fn op_stz_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_absx(bus);
    bus.write(ea, 0, cpu.cycles);
    0
}

/// 65C02: BIT immediate (no N/V update).
fn op_bit_imm(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let val = cpu.addr_imm(bus);
    cpu.flags.set(Flags::Z, cpu.a & val == 0);
    0
}

/// 65C02: BIT zpx.
fn op_bit_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_bit(val);
    0
}

/// 65C02: BIT absx.
fn op_bit_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_absx(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_bit(val);
    cross as u8
}

/// 65C02: PHX.
fn op_phx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let x = cpu.x;
    cpu.push(bus, x);
    0
}

/// 65C02: PHY.
fn op_phy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let y = cpu.y;
    cpu.push(bus, y);
    0
}

/// 65C02: PLX.
fn op_plx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    cpu.x = cpu.pop(bus);
    cpu.flags.set_nz(cpu.x);
    0
}

/// 65C02: PLY.
fn op_ply(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    cpu.y = cpu.pop(bus);
    cpu.flags.set_nz(cpu.y);
    0
}

/// 65C02: JMP (abs,X).
fn op_jmp_ind_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let base = cpu.addr_abs(bus);
    let ptr = base.wrapping_add(cpu.x as u16);
    let lo = bus.read(ptr, cpu.cycles) as u16;
    let hi = bus.read(ptr.wrapping_add(1), cpu.cycles) as u16;
    cpu.pc = (hi << 8) | lo;
    0
}

/// 65C02: ORA (zp) — zero-page indirect (no index).
fn op_ora_ind_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_ind_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a |= val;
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_and_ind_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_ind_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a &= val;
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_eor_ind_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_ind_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a ^= val;
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_adc_ind_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_ind_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_adc(val);
    0
}

fn op_sta_ind_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_ind_zp(bus);
    bus.write(ea, cpu.a, cpu.cycles);
    0
}

fn op_lda_ind_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_ind_zp(bus);
    cpu.a = bus.read(ea, cpu.cycles);
    cpu.flags.set_nz(cpu.a);
    0
}

fn op_cmp_ind_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_ind_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_cmp(cpu.a, val);
    0
}

fn op_sbc_ind_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_ind_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.op_sbc(val);
    0
}

/// 65C02: INC A (accumulator increment).
fn op_inc_a(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.a = cpu.a.wrapping_add(1);
    cpu.flags.set_nz(cpu.a);
    0
}

/// 65C02: DEC A (accumulator decrement).
fn op_dec_a(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.a = cpu.a.wrapping_sub(1);
    cpu.flags.set_nz(cpu.a);
    0
}

/// NMOS-only JAM (halt CPU).
fn op_jam(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.jammed = true;
    cpu.pc = cpu.pc.wrapping_sub(1); // freeze PC
    0
}

// ── NMOS 6502 undocumented opcodes ───────────────────────────────────────────

/// SLO — ASL memory, then ORA result into A. Sets N, Z from A; C from shift.
/// Addressing variants: (ind,X), zp, abs, (ind),Y, zp,X, abs,Y, abs,X
fn op_slo_indx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_indx(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_asl(val);
    bus.write(ea, result, cpu.cycles);
    cpu.a |= result;
    cpu.flags.set_nz(cpu.a);
    0
}
fn op_slo_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_asl(val);
    bus.write(ea, result, cpu.cycles);
    cpu.a |= result;
    cpu.flags.set_nz(cpu.a);
    0
}
fn op_slo_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_asl(val);
    bus.write(ea, result, cpu.cycles);
    cpu.a |= result;
    cpu.flags.set_nz(cpu.a);
    0
}
fn op_slo_indy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_indy(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_asl(val);
    bus.write(ea, result, cpu.cycles);
    cpu.a |= result;
    cpu.flags.set_nz(cpu.a);
    0
}
fn op_slo_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_asl(val);
    bus.write(ea, result, cpu.cycles);
    cpu.a |= result;
    cpu.flags.set_nz(cpu.a);
    0
}
fn op_slo_absy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_absy(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_asl(val);
    bus.write(ea, result, cpu.cycles);
    cpu.a |= result;
    cpu.flags.set_nz(cpu.a);
    0
}
fn op_slo_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_absx(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_asl(val);
    bus.write(ea, result, cpu.cycles);
    cpu.a |= result;
    cpu.flags.set_nz(cpu.a);
    0
}

/// RLA — ROL memory, then AND result into A. Sets N, Z from A; C from rotate.
fn op_rla_indx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_indx(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_rol(val);
    bus.write(ea, result, cpu.cycles);
    cpu.a &= result;
    cpu.flags.set_nz(cpu.a);
    0
}
fn op_rla_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_rol(val);
    bus.write(ea, result, cpu.cycles);
    cpu.a &= result;
    cpu.flags.set_nz(cpu.a);
    0
}
fn op_rla_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_rol(val);
    bus.write(ea, result, cpu.cycles);
    cpu.a &= result;
    cpu.flags.set_nz(cpu.a);
    0
}
fn op_rla_indy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_indy(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_rol(val);
    bus.write(ea, result, cpu.cycles);
    cpu.a &= result;
    cpu.flags.set_nz(cpu.a);
    0
}
fn op_rla_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_rol(val);
    bus.write(ea, result, cpu.cycles);
    cpu.a &= result;
    cpu.flags.set_nz(cpu.a);
    0
}
fn op_rla_absy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_absy(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_rol(val);
    bus.write(ea, result, cpu.cycles);
    cpu.a &= result;
    cpu.flags.set_nz(cpu.a);
    0
}
fn op_rla_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_absx(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_rol(val);
    bus.write(ea, result, cpu.cycles);
    cpu.a &= result;
    cpu.flags.set_nz(cpu.a);
    0
}

/// SRE — LSR memory, then EOR result into A. Sets N, Z from A; C from shift.
fn op_sre_indx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_indx(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_lsr(val);
    bus.write(ea, result, cpu.cycles);
    cpu.a ^= result;
    cpu.flags.set_nz(cpu.a);
    0
}
fn op_sre_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_lsr(val);
    bus.write(ea, result, cpu.cycles);
    cpu.a ^= result;
    cpu.flags.set_nz(cpu.a);
    0
}
fn op_sre_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_lsr(val);
    bus.write(ea, result, cpu.cycles);
    cpu.a ^= result;
    cpu.flags.set_nz(cpu.a);
    0
}
fn op_sre_indy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_indy(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_lsr(val);
    bus.write(ea, result, cpu.cycles);
    cpu.a ^= result;
    cpu.flags.set_nz(cpu.a);
    0
}
fn op_sre_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_lsr(val);
    bus.write(ea, result, cpu.cycles);
    cpu.a ^= result;
    cpu.flags.set_nz(cpu.a);
    0
}
fn op_sre_absy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_absy(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_lsr(val);
    bus.write(ea, result, cpu.cycles);
    cpu.a ^= result;
    cpu.flags.set_nz(cpu.a);
    0
}
fn op_sre_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_absx(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_lsr(val);
    bus.write(ea, result, cpu.cycles);
    cpu.a ^= result;
    cpu.flags.set_nz(cpu.a);
    0
}

/// RRA — ROR memory, then ADC result into A.
fn op_rra_indx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_indx(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_ror(val);
    bus.write(ea, result, cpu.cycles);
    cpu.op_adc(result);
    0
}
fn op_rra_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_ror(val);
    bus.write(ea, result, cpu.cycles);
    cpu.op_adc(result);
    0
}
fn op_rra_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_ror(val);
    bus.write(ea, result, cpu.cycles);
    cpu.op_adc(result);
    0
}
fn op_rra_indy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_indy(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_ror(val);
    bus.write(ea, result, cpu.cycles);
    cpu.op_adc(result);
    0
}
fn op_rra_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_ror(val);
    bus.write(ea, result, cpu.cycles);
    cpu.op_adc(result);
    0
}
fn op_rra_absy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_absy(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_ror(val);
    bus.write(ea, result, cpu.cycles);
    cpu.op_adc(result);
    0
}
fn op_rra_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_absx(bus);
    let val = bus.read(ea, cpu.cycles);
    let result = cpu.op_ror(val);
    bus.write(ea, result, cpu.cycles);
    cpu.op_adc(result);
    0
}

/// SAX — store A & X to memory. No flags affected.
fn op_sax_indx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_indx(bus);
    bus.write(ea, cpu.a & cpu.x, cpu.cycles);
    0
}
fn op_sax_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    bus.write(ea, cpu.a & cpu.x, cpu.cycles);
    0
}
fn op_sax_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    bus.write(ea, cpu.a & cpu.x, cpu.cycles);
    0
}
fn op_sax_zpy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpy(bus);
    bus.write(ea, cpu.a & cpu.x, cpu.cycles);
    0
}

/// LAX — load A and X from memory. Sets N, Z.
fn op_lax_indx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_indx(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a = val;
    cpu.x = val;
    cpu.flags.set_nz(val);
    0
}
fn op_lax_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a = val;
    cpu.x = val;
    cpu.flags.set_nz(val);
    0
}
/// LAX immediate (0xAB) — AND imm with (A | 0xEE), load into A and X.
/// Behaviour is somewhat unstable on real hardware; this is a common approximation.
fn op_lax_imm(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let val = cpu.addr_imm(bus);
    let result = val & (cpu.a | 0xEE);
    cpu.a = result;
    cpu.x = result;
    cpu.flags.set_nz(result);
    0
}
fn op_lax_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a = val;
    cpu.x = val;
    cpu.flags.set_nz(val);
    0
}
fn op_lax_indy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_indy(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a = val;
    cpu.x = val;
    cpu.flags.set_nz(val);
    cross as u8
}
fn op_lax_zpy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpy(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a = val;
    cpu.x = val;
    cpu.flags.set_nz(val);
    0
}
fn op_lax_absy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_absy(bus);
    let val = bus.read(ea, cpu.cycles);
    cpu.a = val;
    cpu.x = val;
    cpu.flags.set_nz(val);
    cross as u8
}

/// DCP — DEC memory, then CMP A with result. Sets N, Z, C.
fn op_dcp_indx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_indx(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_sub(1);
    bus.write(ea, val, cpu.cycles);
    cpu.op_cmp(cpu.a, val);
    0
}
fn op_dcp_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_sub(1);
    bus.write(ea, val, cpu.cycles);
    cpu.op_cmp(cpu.a, val);
    0
}
fn op_dcp_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_sub(1);
    bus.write(ea, val, cpu.cycles);
    cpu.op_cmp(cpu.a, val);
    0
}
fn op_dcp_indy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_indy(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_sub(1);
    bus.write(ea, val, cpu.cycles);
    cpu.op_cmp(cpu.a, val);
    0
}
fn op_dcp_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_sub(1);
    bus.write(ea, val, cpu.cycles);
    cpu.op_cmp(cpu.a, val);
    0
}
fn op_dcp_absy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_absy(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_sub(1);
    bus.write(ea, val, cpu.cycles);
    cpu.op_cmp(cpu.a, val);
    0
}
fn op_dcp_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_absx(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_sub(1);
    bus.write(ea, val, cpu.cycles);
    cpu.op_cmp(cpu.a, val);
    0
}

/// ISC/ISB — INC memory, then SBC A with result.
fn op_isc_indx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_indx(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_add(1);
    bus.write(ea, val, cpu.cycles);
    cpu.op_sbc(val);
    0
}
fn op_isc_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zp(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_add(1);
    bus.write(ea, val, cpu.cycles);
    cpu.op_sbc(val);
    0
}
fn op_isc_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_abs(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_add(1);
    bus.write(ea, val, cpu.cycles);
    cpu.op_sbc(val);
    0
}
fn op_isc_indy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_indy(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_add(1);
    bus.write(ea, val, cpu.cycles);
    cpu.op_sbc(val);
    0
}
fn op_isc_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let ea = cpu.addr_zpx(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_add(1);
    bus.write(ea, val, cpu.cycles);
    cpu.op_sbc(val);
    0
}
fn op_isc_absy(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_absy(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_add(1);
    bus.write(ea, val, cpu.cycles);
    cpu.op_sbc(val);
    0
}
fn op_isc_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, _) = cpu.addr_absx(bus);
    let val = bus.read(ea, cpu.cycles).wrapping_add(1);
    bus.write(ea, val, cpu.cycles);
    cpu.op_sbc(val);
    0
}

/// ANC — AND immediate, then copy N into C.
fn op_anc(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let val = cpu.addr_imm(bus);
    cpu.a &= val;
    cpu.flags.set_nz(cpu.a);
    let n = cpu.flags.contains(Flags::N);
    cpu.flags.set(Flags::C, n);
    0
}

/// ALR — AND immediate, then LSR accumulator.
fn op_alr(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let val = cpu.addr_imm(bus);
    cpu.a &= val;
    let a = cpu.a;
    cpu.a = cpu.op_lsr(a);
    0
}

/// ARR — AND immediate, then ROR accumulator with special C/V flags.
fn op_arr(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let val = cpu.addr_imm(bus);
    cpu.a &= val;
    let c_in = cpu.flags.contains(Flags::C) as u8;
    cpu.a = (c_in << 7) | (cpu.a >> 1);
    cpu.flags.set_nz(cpu.a);
    let bit6 = (cpu.a >> 6) & 1;
    let bit5 = (cpu.a >> 5) & 1;
    cpu.flags.set(Flags::C, bit6 != 0);
    cpu.flags.set(Flags::V, (bit6 ^ bit5) != 0);
    0
}

/// SBX/AXS — (A & X) - immediate → X. Sets N, Z, C.
fn op_sbx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let val = cpu.addr_imm(bus);
    let ax = cpu.a & cpu.x;
    cpu.flags.set(Flags::C, ax >= val);
    cpu.x = ax.wrapping_sub(val);
    cpu.flags.set_nz(cpu.x);
    0
}

/// LAS — memory & SP → A, X, SP. Sets N, Z.
fn op_las(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (ea, cross) = cpu.addr_absy(bus);
    let val = bus.read(ea, cpu.cycles) & cpu.sp;
    cpu.a = val;
    cpu.x = val;
    cpu.sp = val;
    cpu.flags.set_nz(val);
    cross as u8
}

/// SBC duplicate (0xEB) — identical to regular SBC immediate.
fn op_sbc_imm_eb(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let val = cpu.addr_imm(bus);
    cpu.op_sbc(val);
    0
}

/// Multi-byte NOP variants that consume operand bytes without doing anything.
/// 2-byte NOP: skip one immediate byte.
fn op_nop_imm(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let _ = cpu.addr_imm(bus); // consume the operand byte
    0
}
/// 2-byte NOP: skip one zero-page byte.
fn op_nop_zp(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let _ = cpu.addr_zp(bus);
    0
}
/// 2-byte NOP: skip one zero-page,X byte.
fn op_nop_zpx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let _ = cpu.addr_zpx(bus);
    0
}
/// 3-byte NOP: skip two absolute address bytes.
fn op_nop_abs(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let _ = cpu.addr_abs(bus);
    0
}
/// 3-byte NOP: skip two absolute,X address bytes (may add +1 cycle for page cross).
fn op_nop_absx(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    let (_, cross) = cpu.addr_absx(bus);
    cross as u8
}

// ── Dispatch tables ───────────────────────────────────────────────────────────

/// NMOS 6502 dispatch table.
#[rustfmt::skip]
pub static DISPATCH_6502: [OpFn; 256] = [
//  x0           x1              x2           x3
    op_brk,      op_ora_indx,    op_jam,      op_slo_indx,  // 00-03
    op_nop_zp,   op_ora_zp,      op_asl_zp,   op_slo_zp,    // 04-07
    op_php,      op_ora_imm,     op_asl_a,    op_anc,        // 08-0B
    op_nop_abs,  op_ora_abs,     op_asl_abs,  op_slo_abs,    // 0C-0F

    op_bpl,      op_ora_indy,    op_jam,      op_slo_indy,   // 10-13
    op_nop_zpx,  op_ora_zpx,     op_asl_zpx,  op_slo_zpx,   // 14-17
    op_clc,      op_ora_absy,    op_nop,      op_slo_absy,   // 18-1B
    op_nop_absx, op_ora_absx,    op_asl_absx, op_slo_absx,   // 1C-1F

    op_jsr,      op_and_indx,    op_jam,      op_rla_indx,   // 20-23
    op_bit_zp,   op_and_zp,      op_rol_zp,   op_rla_zp,    // 24-27
    op_plp,      op_and_imm,     op_rol_a,    op_anc,        // 28-2B
    op_bit_abs,  op_and_abs,     op_rol_abs,  op_rla_abs,    // 2C-2F

    op_bmi,      op_and_indy,    op_jam,      op_rla_indy,   // 30-33
    op_nop_zpx,  op_and_zpx,     op_rol_zpx,  op_rla_zpx,   // 34-37
    op_sec,      op_and_absy,    op_nop,      op_rla_absy,   // 38-3B
    op_nop_absx, op_and_absx,    op_rol_absx, op_rla_absx,   // 3C-3F

    op_rti,      op_eor_indx,    op_jam,      op_sre_indx,   // 40-43
    op_nop_zp,   op_eor_zp,      op_lsr_zp,   op_sre_zp,    // 44-47
    op_pha,      op_eor_imm,     op_lsr_a,    op_alr,        // 48-4B
    op_jmp_abs,  op_eor_abs,     op_lsr_abs,  op_sre_abs,    // 4C-4F

    op_bvc,      op_eor_indy,    op_jam,      op_sre_indy,   // 50-53
    op_nop_zpx,  op_eor_zpx,     op_lsr_zpx,  op_sre_zpx,   // 54-57
    op_cli,      op_eor_absy,    op_nop,      op_sre_absy,   // 58-5B
    op_nop_absx, op_eor_absx,    op_lsr_absx, op_sre_absx,   // 5C-5F

    op_rts,      op_adc_indx,    op_jam,      op_rra_indx,   // 60-63
    op_nop_zp,   op_adc_zp,      op_ror_zp,   op_rra_zp,    // 64-67
    op_pla,      op_adc_imm,     op_ror_a,    op_arr,        // 68-6B
    op_jmp_ind,  op_adc_abs,     op_ror_abs,  op_rra_abs,    // 6C-6F

    op_bvs,      op_adc_indy,    op_jam,      op_rra_indy,   // 70-73
    op_nop_zpx,  op_adc_zpx,     op_ror_zpx,  op_rra_zpx,   // 74-77
    op_sei,      op_adc_absy,    op_nop,      op_rra_absy,   // 78-7B
    op_nop_absx, op_adc_absx,    op_ror_absx, op_rra_absx,   // 7C-7F

    op_nop_imm,  op_sta_indx,    op_nop_imm,  op_sax_indx,   // 80-83
    op_sty_zp,   op_sta_zp,      op_stx_zp,   op_sax_zp,    // 84-87
    op_dey,      op_nop_imm,     op_txa,      op_nop,        // 88-8B
    op_sty_abs,  op_sta_abs,     op_stx_abs,  op_sax_abs,    // 8C-8F

    op_bcc,      op_sta_indy,    op_jam,      op_nop,        // 90-93
    op_sty_zpx,  op_sta_zpx,     op_stx_zpy,  op_sax_zpy,   // 94-97
    op_tya,      op_sta_absy,    op_txs,      op_nop,        // 98-9B
    op_nop,      op_sta_absx,    op_nop,      op_nop,        // 9C-9F

    op_ldy_imm,  op_lda_indx,    op_ldx_imm,  op_lax_indx,  // A0-A3
    op_ldy_zp,   op_lda_zp,      op_ldx_zp,   op_lax_zp,    // A4-A7
    op_tay,      op_lda_imm,     op_tax,      op_lax_imm,   // A8-AB
    op_ldy_abs,  op_lda_abs,     op_ldx_abs,  op_lax_abs,   // AC-AF

    op_bcs,      op_lda_indy,    op_jam,      op_lax_indy,  // B0-B3
    op_ldy_zpx,  op_lda_zpx,     op_ldx_zpy,  op_lax_zpy,  // B4-B7
    op_clv,      op_lda_absy,    op_tsx,      op_las,       // B8-BB
    op_ldy_absx, op_lda_absx,    op_ldx_absy, op_lax_absy, // BC-BF

    op_cpy_imm,  op_cmp_indx,    op_nop_imm,  op_dcp_indx,  // C0-C3
    op_cpy_zp,   op_cmp_zp,      op_dec_zp,   op_dcp_zp,   // C4-C7
    op_iny,      op_cmp_imm,     op_dex,      op_sbx,       // C8-CB
    op_cpy_abs,  op_cmp_abs,     op_dec_abs,  op_dcp_abs,   // CC-CF

    op_bne,      op_cmp_indy,    op_jam,      op_dcp_indy,  // D0-D3
    op_nop_zpx,  op_cmp_zpx,     op_dec_zpx,  op_dcp_zpx,  // D4-D7
    op_cld,      op_cmp_absy,    op_nop,      op_dcp_absy,  // D8-DB
    op_nop_absx, op_cmp_absx,    op_dec_absx, op_dcp_absx,  // DC-DF

    op_cpx_imm,  op_sbc_indx,    op_nop_imm,  op_isc_indx,  // E0-E3
    op_cpx_zp,   op_sbc_zp,      op_inc_zp,   op_isc_zp,   // E4-E7
    op_inx,      op_sbc_imm,     op_nop,      op_sbc_imm_eb, // E8-EB (EA=NOP, EB=SBC dup)
    op_cpx_abs,  op_sbc_abs,     op_inc_abs,  op_isc_abs,   // EC-EF

    op_beq,      op_sbc_indy,    op_jam,      op_isc_indy,  // F0-F3
    op_nop_zpx,  op_sbc_zpx,     op_inc_zpx,  op_isc_zpx,  // F4-F7
    op_sed,      op_sbc_absy,    op_nop,      op_isc_absy,  // F8-FB
    op_nop_absx, op_sbc_absx,    op_inc_absx, op_isc_absx,  // FC-FF
];

/// CMOS 65C02 dispatch table.
/// Mostly identical to 6502, with additional legal opcodes.
#[rustfmt::skip]
pub static DISPATCH_65C02: [OpFn; 256] = [
//  x0               x1              x2           x3
    op_brk,          op_ora_indx,    op_nop,      op_nop, // 0x
    op_tsb_zp,       op_ora_zp,      op_asl_zp,   op_nop,
    op_php,          op_ora_imm,     op_asl_a,    op_nop,
    op_tsb_abs,      op_ora_abs,     op_asl_abs,  op_nop,

    op_bpl,          op_ora_indy,    op_ora_ind_zp,op_nop, // 1x
    op_trb_zp,       op_ora_zpx,     op_asl_zpx,  op_nop,
    op_clc,          op_ora_absy,    op_inc_a,    op_nop,
    op_trb_abs,      op_ora_absx,    op_asl_absx, op_nop,

    op_jsr,          op_and_indx,    op_nop,      op_nop, // 2x
    op_bit_zp,       op_and_zp,      op_rol_zp,   op_nop,
    op_plp,          op_and_imm,     op_rol_a,    op_nop,
    op_bit_abs,      op_and_abs,     op_rol_abs,  op_nop,

    op_bmi,          op_and_indy,    op_and_ind_zp,op_nop, // 3x
    op_bit_zpx,      op_and_zpx,     op_rol_zpx,  op_nop,
    op_sec,          op_and_absy,    op_dec_a,    op_nop,
    op_bit_absx,     op_and_absx,    op_rol_absx, op_nop,

    op_rti,          op_eor_indx,    op_nop,      op_nop, // 4x
    op_nop,          op_eor_zp,      op_lsr_zp,   op_nop,
    op_pha,          op_eor_imm,     op_lsr_a,    op_nop,
    op_jmp_abs,      op_eor_abs,     op_lsr_abs,  op_nop,

    op_bvc,          op_eor_indy,    op_eor_ind_zp,op_nop, // 5x
    op_nop,          op_eor_zpx,     op_lsr_zpx,  op_nop,
    op_cli,          op_eor_absy,    op_phy,      op_nop,
    op_nop,          op_eor_absx,    op_lsr_absx, op_nop,

    op_rts,          op_adc_indx,    op_nop,      op_nop, // 6x
    op_stz_zp,       op_adc_zp,      op_ror_zp,   op_nop,
    op_pla,          op_adc_imm,     op_ror_a,    op_nop,
    op_jmp_ind,      op_adc_abs,     op_ror_abs,  op_nop,

    op_bvs,          op_adc_indy,    op_adc_ind_zp,op_nop, // 7x
    op_stz_zpx,      op_adc_zpx,     op_ror_zpx,  op_nop,
    op_sei,          op_adc_absy,    op_ply,      op_nop,
    op_jmp_ind_absx, op_adc_absx,    op_ror_absx, op_nop,

    op_bra,          op_sta_indx,    op_nop,      op_nop, // 8x
    op_sty_zp,       op_sta_zp,      op_stx_zp,   op_nop,
    op_dey,          op_bit_imm,     op_txa,      op_nop,
    op_sty_abs,      op_sta_abs,     op_stx_abs,  op_nop,

    op_bcc,          op_sta_indy,    op_sta_ind_zp,op_nop, // 9x
    op_sty_zpx,      op_sta_zpx,     op_stx_zpy,  op_nop,
    op_tya,          op_sta_absy,    op_txs,      op_nop,
    op_stz_abs,      op_sta_absx,    op_stz_absx, op_nop,

    op_ldy_imm,      op_lda_indx,    op_ldx_imm,  op_nop, // Ax
    op_ldy_zp,       op_lda_zp,      op_ldx_zp,   op_nop,
    op_tay,          op_lda_imm,     op_tax,      op_nop,
    op_ldy_abs,      op_lda_abs,     op_ldx_abs,  op_nop,

    op_bcs,          op_lda_indy,    op_lda_ind_zp,op_nop, // Bx
    op_ldy_zpx,      op_lda_zpx,     op_ldx_zpy,  op_nop,
    op_clv,          op_lda_absy,    op_tsx,      op_nop,
    op_ldy_absx,     op_lda_absx,    op_ldx_absy, op_nop,

    op_cpy_imm,      op_cmp_indx,    op_nop,      op_nop, // Cx
    op_cpy_zp,       op_cmp_zp,      op_dec_zp,   op_nop,
    op_iny,          op_cmp_imm,     op_dex,      op_wai,
    op_cpy_abs,      op_cmp_abs,     op_dec_abs,  op_nop,

    op_bne,          op_cmp_indy,    op_cmp_ind_zp,op_nop, // Dx
    op_nop,          op_cmp_zpx,     op_dec_zpx,  op_nop,
    op_cld,          op_cmp_absy,    op_phx,      op_stp,
    op_nop,          op_cmp_absx,    op_dec_absx, op_nop,

    op_cpx_imm,      op_sbc_indx,    op_nop,      op_nop, // Ex
    op_cpx_zp,       op_sbc_zp,      op_inc_zp,   op_nop,
    op_inx,          op_sbc_imm,     op_nop,      op_nop,
    op_cpx_abs,      op_sbc_abs,     op_inc_abs,  op_nop,

    op_beq,          op_sbc_indy,    op_sbc_ind_zp,op_nop, // Fx
    op_nop,          op_sbc_zpx,     op_inc_zpx,  op_nop,
    op_sed,          op_sbc_absy,    op_plx,      op_nop,
    op_nop,          op_sbc_absx,    op_inc_absx, op_nop,
];

// 65C02 BRA (branch always)
fn op_bra(cpu: &mut Cpu, bus: &mut Bus) -> u8 {
    cpu.branch_target(bus, true)
}

/// 65C02 WAI ($CB): halt CPU until an interrupt (IRQ or NMI) arrives.
/// Sets `cpu.waiting = true`; the emulator execute loop must check this flag
/// and skip instruction dispatch while it is set, advancing time instead.
fn op_wai(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.waiting = true;
    0
}

/// 65C02 STP ($DB): stop the processor entirely (only a RESET can restart).
/// Reuses the existing `jammed` flag.
fn op_stp(cpu: &mut Cpu, _bus: &mut Bus) -> u8 {
    cpu.jammed = true;
    0
}
