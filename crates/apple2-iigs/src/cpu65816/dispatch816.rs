//! 65C816 opcode dispatch table.
//!
//! All 256 opcodes are valid (no illegal opcodes on the 65C816).
//! The dispatch table maps each opcode to a handler function.

use super::Bus816;
use super::flags816::Flags816;
use super::registers::Cpu65816;

/// Handler function type: takes CPU + bus, returns extra cycles consumed.
type OpFn = fn(&mut Cpu65816, &mut dyn Bus816) -> u8;

/// Base cycle counts for each opcode (before width/page-cross adjustments).
/// These assume 8-bit register widths; 16-bit operations add 1 cycle for
/// the extra byte. Direct page non-aligned adds 1 cycle when DL != 0.
#[rustfmt::skip]
static BASE_CYCLES: [u8; 256] = [
//  x0 x1 x2 x3 x4 x5 x6 x7  x8 x9 xA xB xC xD xE xF
    7, 6, 7, 4, 5, 3, 5, 6,  3, 2, 2, 4, 6, 4, 6, 5, // 0x
    2, 5, 5, 7, 5, 4, 6, 6,  2, 4, 2, 2, 6, 4, 7, 5, // 1x
    6, 6, 8, 4, 3, 3, 5, 6,  4, 2, 2, 5, 4, 4, 6, 5, // 2x
    2, 5, 5, 7, 4, 4, 6, 6,  2, 4, 2, 2, 4, 4, 7, 5, // 3x
    6, 6, 2, 4, 3, 3, 5, 6,  3, 2, 2, 3, 3, 4, 6, 5, // 4x
    2, 5, 5, 7, 3, 4, 6, 6,  2, 4, 3, 2, 4, 4, 7, 5, // 5x
    6, 6, 6, 4, 3, 3, 5, 6,  4, 2, 2, 6, 5, 4, 6, 5, // 6x
    2, 5, 5, 7, 4, 4, 6, 6,  2, 4, 4, 2, 6, 4, 7, 5, // 7x
    3, 6, 4, 4, 3, 3, 3, 6,  2, 2, 2, 3, 4, 4, 4, 5, // 8x
    2, 6, 5, 7, 4, 4, 4, 6,  2, 5, 2, 2, 4, 5, 5, 5, // 9x
    2, 6, 2, 4, 3, 3, 3, 6,  2, 2, 2, 4, 4, 4, 4, 5, // Ax
    2, 5, 5, 7, 4, 4, 4, 6,  2, 4, 2, 2, 4, 4, 4, 5, // Bx
    2, 6, 3, 4, 3, 3, 5, 6,  2, 2, 2, 3, 4, 4, 6, 5, // Cx
    2, 5, 5, 7, 6, 4, 6, 6,  2, 4, 3, 3, 6, 4, 7, 5, // Dx
    2, 6, 3, 4, 3, 3, 5, 6,  2, 2, 2, 3, 4, 4, 6, 5, // Ex
    2, 5, 5, 7, 5, 4, 6, 6,  2, 4, 4, 2, 8, 4, 7, 5, // Fx
];

/// Step the CPU by one instruction. Returns cycles consumed.
pub fn step(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    // Check for NMI (highest priority)
    if cpu.nmi_pending != 0 {
        cpu.nmi_pending = 0;
        cpu.waiting = false;
        let vector = if cpu.emulation { 0xFFFA } else { 0xFFEA };
        cpu.service_interrupt(bus, vector, false);
        return 7;
    }

    // Check for IRQ
    if cpu.irq_pending != 0 && !cpu.flags.contains(Flags816::I) {
        if cpu.irq_defer {
            cpu.irq_defer = false;
        } else {
            cpu.waiting = false;
            let vector = if cpu.emulation { 0xFFFE } else { 0xFFEE };
            cpu.service_interrupt(bus, vector, false);
            return 7;
        }
    }

    // WAI: halted waiting for interrupt
    if cpu.waiting {
        cpu.cycles += 1;
        return 1;
    }

    // STP: completely stopped until reset
    if cpu.stopped {
        cpu.cycles += 1;
        return 1;
    }

    // Fetch opcode
    let opcode = bus.read(cpu.full_pc(), cpu.cycles);
    cpu.pc = cpu.pc.wrapping_add(1);

    // Dispatch
    let extra = DISPATCH[opcode as usize](cpu, bus);

    // Calculate total cycles
    let cycles = BASE_CYCLES[opcode as usize] + extra;

    // DL != 0 penalty for direct page addressing modes
    // (applied in the handlers that use DP addressing when needed)

    cpu.cycles += cycles as u64;
    cycles
}

// ── Opcode handlers ─────────────────────────────────────────────────────

// Helper macro for read-modify-write on memory with M-width.
macro_rules! rmw_m {
    ($cpu:ident, $bus:ident, $addr:expr, $op:ident) => {{
        let addr = $addr;
        let val = $cpu.read_m($bus, addr);
        let result = $cpu.$op(val);
        $cpu.write_m($bus, addr, result);
        0
    }};
}

// ── 0x00: BRK ──
fn op_00(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    cpu.pc = cpu.pc.wrapping_add(1); // skip signature byte
    let vector = if cpu.emulation { 0xFFFE } else { 0xFFE6 };
    cpu.service_interrupt(bus, vector, true);
    0
}

// ── 0x01: ORA (dp,X) ──
fn op_01(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_ora(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x02: COP ──
fn op_02(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    cpu.pc = cpu.pc.wrapping_add(1); // skip signature byte
    let vector = if cpu.emulation { 0xFFF4 } else { 0xFFE4 };
    cpu.service_interrupt(bus, vector, false);
    0
}

// ── 0x03: ORA d,S ──
fn op_03(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_sr(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_ora(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x04: TSB dp ──
fn op_04(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_dp(bus), op_tsb)
}

// ── 0x05: ORA dp ──
fn op_05(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_ora(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x06: ASL dp ──
fn op_06(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_dp(bus), op_asl)
}

// ── 0x07: ORA [dp] ──
fn op_07(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_long(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_ora(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x08: PHP ──
fn op_08(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let p = cpu.flags.bits();
    cpu.push8(bus, p);
    0
}

// ── 0x09: ORA # ──
fn op_09(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.acc_8bit() {
        let val = cpu.addr_imm8(bus) as u16;
        cpu.op_ora(val);
        0
    } else {
        let val = cpu.addr_imm16(bus);
        cpu.op_ora(val);
        1
    }
}

// ── 0x0A: ASL A ──
fn op_0a(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    let val = cpu.a();
    let result = cpu.op_asl(val);
    cpu.set_a(result);
    0
}

// ── 0x0B: PHD ──
fn op_0b(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    cpu.push16(bus, cpu.dp);
    0
}

// ── 0x0C: TSB abs ──
fn op_0c(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_abs(bus), op_tsb)
}

// ── 0x0D: ORA abs ──
fn op_0d(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_ora(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x0E: ASL abs ──
fn op_0e(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_abs(bus), op_asl)
}

// ── 0x0F: ORA long ──
fn op_0f(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs_long(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_ora(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x10: BPL rel8 ──
fn op_10(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let taken = !cpu.flags.contains(Flags816::N);
    cpu.branch_rel8(bus, taken)
}

// ── 0x11: ORA (dp),Y ──
fn op_11(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_dp_ind_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_ora(val);
    let mut extra = if crossed { 1 } else { 0 };
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        extra += 1;
    }
    extra
}

// ── 0x12: ORA (dp) ──
fn op_12(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_ora(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x13: ORA (d,S),Y ──
fn op_13(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_sr_ind_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_ora(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x14: TRB dp ──
fn op_14(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_dp(bus), op_trb)
}

// ── 0x15: ORA dp,X ──
fn op_15(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_ora(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x16: ASL dp,X ──
fn op_16(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_dp_x(bus), op_asl)
}

// ── 0x17: ORA [dp],Y ──
fn op_17(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_long_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_ora(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x18: CLC ──
fn op_18(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    cpu.flags.remove(Flags816::C);
    0
}

// ── 0x19: ORA abs,Y ──
fn op_19(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_abs_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_ora(val);
    let mut extra = if crossed { 1 } else { 0 };
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        extra += 1;
    }
    extra
}

// ── 0x1A: INC A ──
fn op_1a(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    let val = cpu.a();
    let result = cpu.op_inc_mem(val);
    cpu.set_a(result);
    0
}

// ── 0x1B: TCS ──
fn op_1b(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    cpu.sp = cpu.c;
    if cpu.emulation {
        cpu.sp = 0x0100 | (cpu.sp & 0xFF);
    }
    0
}

// ── 0x1C: TRB abs ──
fn op_1c(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_abs(bus), op_trb)
}

// ── 0x1D: ORA abs,X ──
fn op_1d(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_abs_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_ora(val);
    let mut extra = if crossed { 1 } else { 0 };
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        extra += 1;
    }
    extra
}

// ── 0x1E: ASL abs,X ──
fn op_1e(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, _) = cpu.addr_abs_x(bus);
    let val = cpu.read_m(bus, addr);
    let result = cpu.op_asl(val);
    cpu.write_m(bus, addr, result);
    0
}

// ── 0x1F: ORA long,X ──
fn op_1f(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs_long_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_ora(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x20: JSR abs ──
fn op_20(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let target = cpu.fetch16(bus);
    cpu.push16(bus, cpu.pc.wrapping_sub(1));
    cpu.pc = target;
    0
}

// ── 0x21: AND (dp,X) ──
fn op_21(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_and(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x22: JSL long ──
fn op_22(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let target = cpu.fetch24(bus);
    cpu.push8(bus, cpu.pbr);
    cpu.push16(bus, cpu.pc.wrapping_sub(1));
    cpu.pbr = (target >> 16) as u8;
    cpu.pc = target as u16;
    0
}

// ── 0x23: AND d,S ──
fn op_23(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_sr(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_and(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x24: BIT dp ──
fn op_24(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_bit(val);
    0
}

// ── 0x25: AND dp ──
fn op_25(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_and(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x26: ROL dp ──
fn op_26(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_dp(bus), op_rol)
}

// ── 0x27: AND [dp] ──
fn op_27(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_long(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_and(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x28: PLP ──
fn op_28(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let p = cpu.pop8(bus);
    cpu.flags = Flags816::from_bits_truncate(p);
    cpu.update_mode_flags();
    0
}

// ── 0x29: AND # ──
fn op_29(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.acc_8bit() {
        let val = cpu.addr_imm8(bus) as u16;
        cpu.op_and(val);
        0
    } else {
        let val = cpu.addr_imm16(bus);
        cpu.op_and(val);
        1
    }
}

// ── 0x2A: ROL A ──
fn op_2a(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    let val = cpu.a();
    let result = cpu.op_rol(val);
    cpu.set_a(result);
    0
}

// ── 0x2B: PLD ──
fn op_2b(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    cpu.dp = cpu.pop16(bus);
    cpu.flags.set_nz16(cpu.dp);
    0
}

// ── 0x2C: BIT abs ──
fn op_2c(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_bit(val);
    0
}

// ── 0x2D: AND abs ──
fn op_2d(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_and(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x2E: ROL abs ──
fn op_2e(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_abs(bus), op_rol)
}

// ── 0x2F: AND long ──
fn op_2f(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs_long(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_and(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x30: BMI rel8 ──
fn op_30(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let taken = cpu.flags.contains(Flags816::N);
    cpu.branch_rel8(bus, taken)
}

// ── 0x31: AND (dp),Y ──
fn op_31(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_dp_ind_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_and(val);
    let mut extra = if crossed { 1 } else { 0 };
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        extra += 1;
    }
    extra
}

// ── 0x32: AND (dp) ──
fn op_32(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_and(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x33: AND (d,S),Y ──
fn op_33(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_sr_ind_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_and(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x34: BIT dp,X ──
fn op_34(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_bit(val);
    0
}

// ── 0x35: AND dp,X ──
fn op_35(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_and(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x36: ROL dp,X ──
fn op_36(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_dp_x(bus), op_rol)
}

// ── 0x37: AND [dp],Y ──
fn op_37(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_long_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_and(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x38: SEC ──
fn op_38(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    cpu.flags.insert(Flags816::C);
    0
}

// ── 0x39: AND abs,Y ──
fn op_39(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_abs_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_and(val);
    let mut extra = if crossed { 1 } else { 0 };
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        extra += 1;
    }
    extra
}

// ── 0x3A: DEC A ──
fn op_3a(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    let val = cpu.a();
    let result = cpu.op_dec_mem(val);
    cpu.set_a(result);
    0
}

// ── 0x3B: TSC ──
fn op_3b(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    cpu.c = cpu.sp;
    cpu.flags.set_nz16(cpu.c);
    0
}

// ── 0x3C: BIT abs,X ──
fn op_3c(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_abs_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_bit(val);
    if crossed { 1 } else { 0 }
}

// ── 0x3D: AND abs,X ──
fn op_3d(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_abs_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_and(val);
    let mut extra = if crossed { 1 } else { 0 };
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        extra += 1;
    }
    extra
}

// ── 0x3E: ROL abs,X ──
fn op_3e(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, _) = cpu.addr_abs_x(bus);
    rmw_m!(cpu, bus, addr, op_rol)
}

// ── 0x3F: AND long,X ──
fn op_3f(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs_long_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_and(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x40: RTI ──
fn op_40(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let p = cpu.pop8(bus);
    cpu.flags = Flags816::from_bits_truncate(p);
    cpu.update_mode_flags();
    cpu.pc = cpu.pop16(bus);
    if !cpu.emulation {
        cpu.pbr = cpu.pop8(bus);
    }
    0
}

// ── 0x41: EOR (dp,X) ──
fn op_41(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_eor(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x42: WDM ──
fn op_42(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let _ = cpu.fetch8(bus); // skip signature byte (reserved for future use)
    0
}

// ── 0x43: EOR d,S ──
fn op_43(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_sr(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_eor(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x44: MVP src,dst ──
fn op_44(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    cpu.op_mvp(bus);
    0
}

// ── 0x45: EOR dp ──
fn op_45(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_eor(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x46: LSR dp ──
fn op_46(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_dp(bus), op_lsr)
}

// ── 0x47: EOR [dp] ──
fn op_47(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_long(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_eor(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x48: PHA ──
fn op_48(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.acc_8bit() {
        cpu.push8(bus, cpu.c as u8);
        0
    } else {
        cpu.push16(bus, cpu.c);
        1
    }
}

// ── 0x49: EOR # ──
fn op_49(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.acc_8bit() {
        let val = cpu.addr_imm8(bus) as u16;
        cpu.op_eor(val);
        0
    } else {
        let val = cpu.addr_imm16(bus);
        cpu.op_eor(val);
        1
    }
}

// ── 0x4A: LSR A ──
fn op_4a(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    let val = cpu.a();
    let result = cpu.op_lsr(val);
    cpu.set_a(result);
    0
}

// ── 0x4B: PHK ──
fn op_4b(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    cpu.push8(bus, cpu.pbr);
    0
}

// ── 0x4C: JMP abs ──
fn op_4c(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    cpu.pc = cpu.fetch16(bus);
    0
}

// ── 0x4D: EOR abs ──
fn op_4d(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_eor(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x4E: LSR abs ──
fn op_4e(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_abs(bus), op_lsr)
}

// ── 0x4F: EOR long ──
fn op_4f(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs_long(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_eor(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x50: BVC rel8 ──
fn op_50(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let taken = !cpu.flags.contains(Flags816::V);
    cpu.branch_rel8(bus, taken)
}

// ── 0x51: EOR (dp),Y ──
fn op_51(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_dp_ind_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_eor(val);
    let mut extra = if crossed { 1 } else { 0 };
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        extra += 1;
    }
    extra
}

// ── 0x52: EOR (dp) ──
fn op_52(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_eor(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x53: EOR (d,S),Y ──
fn op_53(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_sr_ind_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_eor(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x54: MVN src,dst ──
fn op_54(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    cpu.op_mvn(bus);
    0
}

// ── 0x55: EOR dp,X ──
fn op_55(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_eor(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x56: LSR dp,X ──
fn op_56(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_dp_x(bus), op_lsr)
}

// ── 0x57: EOR [dp],Y ──
fn op_57(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_long_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_eor(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x58: CLI ──
fn op_58(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    cpu.flags.remove(Flags816::I);
    0
}

// ── 0x59: EOR abs,Y ──
fn op_59(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_abs_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_eor(val);
    let mut extra = if crossed { 1 } else { 0 };
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        extra += 1;
    }
    extra
}

// ── 0x5A: PHY ──
fn op_5a(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.idx_8bit() {
        cpu.push8(bus, cpu.y as u8);
        0
    } else {
        cpu.push16(bus, cpu.y);
        1
    }
}

// ── 0x5B: TCD ──
fn op_5b(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    cpu.dp = cpu.c;
    cpu.flags.set_nz16(cpu.dp);
    0
}

// ── 0x5C: JML long ──
fn op_5c(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.fetch24(bus);
    cpu.pbr = (addr >> 16) as u8;
    cpu.pc = addr as u16;
    0
}

// ── 0x5D: EOR abs,X ──
fn op_5d(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_abs_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_eor(val);
    let mut extra = if crossed { 1 } else { 0 };
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        extra += 1;
    }
    extra
}

// ── 0x5E: LSR abs,X ──
fn op_5e(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, _) = cpu.addr_abs_x(bus);
    rmw_m!(cpu, bus, addr, op_lsr)
}

// ── 0x5F: EOR long,X ──
fn op_5f(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs_long_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_eor(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x60: RTS ──
fn op_60(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    cpu.pc = cpu.pop16(bus).wrapping_add(1);
    0
}

// ── 0x61: ADC (dp,X) ──
fn op_61(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_adc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x62: PER ──
fn op_62(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let offset = cpu.fetch16(bus);
    let addr = cpu.pc.wrapping_add(offset);
    cpu.push16(bus, addr);
    0
}

// ── 0x63: ADC d,S ──
fn op_63(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_sr(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_adc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x64: STZ dp ──
fn op_64(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp(bus);
    cpu.write_m(bus, addr, 0);
    0
}

// ── 0x65: ADC dp ──
fn op_65(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_adc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x66: ROR dp ──
fn op_66(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_dp(bus), op_ror)
}

// ── 0x67: ADC [dp] ──
fn op_67(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_long(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_adc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x68: PLA ──
fn op_68(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.acc_8bit() {
        let val = cpu.pop8(bus);
        cpu.c = (cpu.c & 0xFF00) | val as u16;
        cpu.flags.set_nz8(val);
        0
    } else {
        cpu.c = cpu.pop16(bus);
        cpu.flags.set_nz16(cpu.c);
        1
    }
}

// ── 0x69: ADC # ──
fn op_69(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.acc_8bit() {
        let val = cpu.addr_imm8(bus) as u16;
        cpu.op_adc(val);
        0
    } else {
        let val = cpu.addr_imm16(bus);
        cpu.op_adc(val);
        1
    }
}

// ── 0x6A: ROR A ──
fn op_6a(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    let val = cpu.a();
    let result = cpu.op_ror(val);
    cpu.set_a(result);
    0
}

// ── 0x6B: RTL ──
fn op_6b(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    cpu.pc = cpu.pop16(bus).wrapping_add(1);
    cpu.pbr = cpu.pop8(bus);
    0
}

// ── 0x6C: JMP (abs) ──
fn op_6c(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let ptr = cpu.fetch16(bus) as u32;
    cpu.pc = cpu.read16(bus, ptr);
    0
}

// ── 0x6D: ADC abs ──
fn op_6d(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_adc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x6E: ROR abs ──
fn op_6e(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_abs(bus), op_ror)
}

// ── 0x6F: ADC long ──
fn op_6f(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs_long(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_adc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x70: BVS rel8 ──
fn op_70(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let taken = cpu.flags.contains(Flags816::V);
    cpu.branch_rel8(bus, taken)
}

// ── 0x71: ADC (dp),Y ──
fn op_71(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_dp_ind_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_adc(val);
    let mut extra = if crossed { 1 } else { 0 };
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        extra += 1;
    }
    extra
}

// ── 0x72: ADC (dp) ──
fn op_72(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_adc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x73: ADC (d,S),Y ──
fn op_73(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_sr_ind_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_adc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x74: STZ dp,X ──
fn op_74(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_x(bus);
    cpu.write_m(bus, addr, 0);
    0
}

// ── 0x75: ADC dp,X ──
fn op_75(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_adc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x76: ROR dp,X ──
fn op_76(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_dp_x(bus), op_ror)
}

// ── 0x77: ADC [dp],Y ──
fn op_77(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_long_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_adc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x78: SEI ──
fn op_78(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    cpu.flags.insert(Flags816::I);
    0
}

// ── 0x79: ADC abs,Y ──
fn op_79(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_abs_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_adc(val);
    let mut extra = if crossed { 1 } else { 0 };
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        extra += 1;
    }
    extra
}

// ── 0x7A: PLY ──
fn op_7a(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.idx_8bit() {
        let val = cpu.pop8(bus);
        cpu.y = val as u16;
        cpu.flags.set_nz8(val);
        0
    } else {
        cpu.y = cpu.pop16(bus);
        cpu.flags.set_nz16(cpu.y);
        1
    }
}

// ── 0x7B: TDC ──
fn op_7b(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    cpu.c = cpu.dp;
    cpu.flags.set_nz16(cpu.c);
    0
}

// ── 0x7C: JMP (abs,X) ──
fn op_7c(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs_ind_x(bus);
    cpu.pc = addr as u16;
    0
}

// ── 0x7D: ADC abs,X ──
fn op_7d(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_abs_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_adc(val);
    let mut extra = if crossed { 1 } else { 0 };
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        extra += 1;
    }
    extra
}

// ── 0x7E: ROR abs,X ──
fn op_7e(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, _) = cpu.addr_abs_x(bus);
    rmw_m!(cpu, bus, addr, op_ror)
}

// ── 0x7F: ADC long,X ──
fn op_7f(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs_long_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_adc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0x80: BRA rel8 ──
fn op_80(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    cpu.branch_rel8(bus, true)
}

// ── 0x81: STA (dp,X) ──
fn op_81(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_x(bus);
    let a = cpu.a();
    cpu.write_m(bus, addr, a);
    0
}

// ── 0x82: BRL rel16 ──
fn op_82(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    cpu.branch_rel16(bus);
    0
}

// ── 0x83: STA d,S ──
fn op_83(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_sr(bus);
    let a = cpu.a();
    cpu.write_m(bus, addr, a);
    0
}

// ── 0x84: STY dp ──
fn op_84(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp(bus);
    let y = cpu.get_y();
    cpu.write_x(bus, addr, y);
    0
}

// ── 0x85: STA dp ──
fn op_85(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp(bus);
    let a = cpu.a();
    cpu.write_m(bus, addr, a);
    0
}

// ── 0x86: STX dp ──
fn op_86(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp(bus);
    let x = cpu.get_x();
    cpu.write_x(bus, addr, x);
    0
}

// ── 0x87: STA [dp] ──
fn op_87(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_long(bus);
    let a = cpu.a();
    cpu.write_m(bus, addr, a);
    0
}

// ── 0x88: DEY ──
fn op_88(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    let val = cpu.get_y().wrapping_sub(1);
    cpu.set_y(val);
    cpu.set_nz_x(val);
    0
}

// ── 0x89: BIT # ──
fn op_89(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.acc_8bit() {
        let val = cpu.addr_imm8(bus) as u16;
        cpu.op_bit_imm(val);
        0
    } else {
        let val = cpu.addr_imm16(bus);
        cpu.op_bit_imm(val);
        1
    }
}

// ── 0x8A: TXA ──
fn op_8a(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.acc_8bit() {
        let val = (cpu.x & 0xFF) as u8;
        cpu.c = (cpu.c & 0xFF00) | val as u16;
        cpu.flags.set_nz8(val);
    } else {
        cpu.c = cpu.x;
        cpu.flags.set_nz16(cpu.c);
    }
    0
}

// ── 0x8B: PHB ──
fn op_8b(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    cpu.push8(bus, cpu.dbr);
    0
}

// ── 0x8C: STY abs ──
fn op_8c(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs(bus);
    let y = cpu.get_y();
    cpu.write_x(bus, addr, y);
    0
}

// ── 0x8D: STA abs ──
fn op_8d(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs(bus);
    let a = cpu.a();
    cpu.write_m(bus, addr, a);
    0
}

// ── 0x8E: STX abs ──
fn op_8e(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs(bus);
    let x = cpu.get_x();
    cpu.write_x(bus, addr, x);
    0
}

// ── 0x8F: STA long ──
fn op_8f(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs_long(bus);
    let a = cpu.a();
    cpu.write_m(bus, addr, a);
    0
}

// ── 0x90: BCC rel8 ──
fn op_90(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let taken = !cpu.flags.contains(Flags816::C);
    cpu.branch_rel8(bus, taken)
}

// ── 0x91: STA (dp),Y ──
fn op_91(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, _) = cpu.addr_dp_ind_y(bus);
    let a = cpu.a();
    cpu.write_m(bus, addr, a);
    0
}

// ── 0x92: STA (dp) ──
fn op_92(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind(bus);
    let a = cpu.a();
    cpu.write_m(bus, addr, a);
    0
}

// ── 0x93: STA (d,S),Y ──
fn op_93(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_sr_ind_y(bus);
    let a = cpu.a();
    cpu.write_m(bus, addr, a);
    0
}

// ── 0x94: STY dp,X ──
fn op_94(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_x(bus);
    let y = cpu.get_y();
    cpu.write_x(bus, addr, y);
    0
}

// ── 0x95: STA dp,X ──
fn op_95(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_x(bus);
    let a = cpu.a();
    cpu.write_m(bus, addr, a);
    0
}

// ── 0x96: STX dp,Y ──
fn op_96(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_y(bus);
    let x = cpu.get_x();
    cpu.write_x(bus, addr, x);
    0
}

// ── 0x97: STA [dp],Y ──
fn op_97(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_long_y(bus);
    let a = cpu.a();
    cpu.write_m(bus, addr, a);
    0
}

// ── 0x98: TYA ──
fn op_98(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.acc_8bit() {
        let val = (cpu.y & 0xFF) as u8;
        cpu.c = (cpu.c & 0xFF00) | val as u16;
        cpu.flags.set_nz8(val);
    } else {
        cpu.c = cpu.y;
        cpu.flags.set_nz16(cpu.c);
    }
    0
}

// ── 0x99: STA abs,Y ──
fn op_99(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, _) = cpu.addr_abs_y(bus);
    let a = cpu.a();
    cpu.write_m(bus, addr, a);
    0
}

// ── 0x9A: TXS ──
fn op_9a(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation {
        cpu.sp = 0x0100 | (cpu.x & 0xFF);
    } else {
        cpu.sp = cpu.x;
    }
    0
}

// ── 0x9B: TXY ──
fn op_9b(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    cpu.y = cpu.x;
    if cpu.emulation || cpu.flags.idx_8bit() {
        cpu.y &= 0xFF;
        cpu.flags.set_nz8(cpu.y as u8);
    } else {
        cpu.flags.set_nz16(cpu.y);
    }
    0
}

// ── 0x9C: STZ abs ──
fn op_9c(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs(bus);
    cpu.write_m(bus, addr, 0);
    0
}

// ── 0x9D: STA abs,X ──
fn op_9d(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, _) = cpu.addr_abs_x(bus);
    let a = cpu.a();
    cpu.write_m(bus, addr, a);
    0
}

// ── 0x9E: STZ abs,X ──
fn op_9e(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, _) = cpu.addr_abs_x(bus);
    cpu.write_m(bus, addr, 0);
    0
}

// ── 0x9F: STA long,X ──
fn op_9f(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs_long_x(bus);
    let a = cpu.a();
    cpu.write_m(bus, addr, a);
    0
}

// ── 0xA0: LDY # ──
fn op_a0(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.idx_8bit() {
        let val = cpu.addr_imm8(bus);
        cpu.y = val as u16;
        cpu.flags.set_nz8(val);
        0
    } else {
        let val = cpu.addr_imm16(bus);
        cpu.y = val;
        cpu.flags.set_nz16(val);
        1
    }
}

// ── 0xA1: LDA (dp,X) ──
fn op_a1(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.set_a(val);
    cpu.set_nz_m(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xA2: LDX # ──
fn op_a2(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.idx_8bit() {
        let val = cpu.addr_imm8(bus);
        cpu.x = val as u16;
        cpu.flags.set_nz8(val);
        0
    } else {
        let val = cpu.addr_imm16(bus);
        cpu.x = val;
        cpu.flags.set_nz16(val);
        1
    }
}

// ── 0xA3: LDA d,S ──
fn op_a3(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_sr(bus);
    let val = cpu.read_m(bus, addr);
    cpu.set_a(val);
    cpu.set_nz_m(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xA4: LDY dp ──
fn op_a4(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp(bus);
    let val = cpu.read_x(bus, addr);
    cpu.set_y(val);
    cpu.set_nz_x(val);
    0
}

// ── 0xA5: LDA dp ──
fn op_a5(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp(bus);
    let val = cpu.read_m(bus, addr);
    cpu.set_a(val);
    cpu.set_nz_m(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xA6: LDX dp ──
fn op_a6(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp(bus);
    let val = cpu.read_x(bus, addr);
    cpu.set_x(val);
    cpu.set_nz_x(val);
    0
}

// ── 0xA7: LDA [dp] ──
fn op_a7(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_long(bus);
    let val = cpu.read_m(bus, addr);
    cpu.set_a(val);
    cpu.set_nz_m(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xA8: TAY ──
fn op_a8(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.idx_8bit() {
        cpu.y = cpu.c & 0xFF;
        cpu.flags.set_nz8(cpu.y as u8);
    } else {
        cpu.y = cpu.c;
        cpu.flags.set_nz16(cpu.y);
    }
    0
}

// ── 0xA9: LDA # ──
fn op_a9(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.acc_8bit() {
        let val = cpu.addr_imm8(bus);
        cpu.c = (cpu.c & 0xFF00) | val as u16;
        cpu.flags.set_nz8(val);
        0
    } else {
        let val = cpu.addr_imm16(bus);
        cpu.c = val;
        cpu.flags.set_nz16(val);
        1
    }
}

// ── 0xAA: TAX ──
fn op_aa(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.idx_8bit() {
        cpu.x = cpu.c & 0xFF;
        cpu.flags.set_nz8(cpu.x as u8);
    } else {
        cpu.x = cpu.c;
        cpu.flags.set_nz16(cpu.x);
    }
    0
}

// ── 0xAB: PLB ──
fn op_ab(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    cpu.dbr = cpu.pop8(bus);
    cpu.flags.set_nz8(cpu.dbr);
    0
}

// ── 0xAC: LDY abs ──
fn op_ac(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs(bus);
    let val = cpu.read_x(bus, addr);
    cpu.set_y(val);
    cpu.set_nz_x(val);
    0
}

// ── 0xAD: LDA abs ──
fn op_ad(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs(bus);
    let val = cpu.read_m(bus, addr);
    cpu.set_a(val);
    cpu.set_nz_m(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xAE: LDX abs ──
fn op_ae(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs(bus);
    let val = cpu.read_x(bus, addr);
    cpu.set_x(val);
    cpu.set_nz_x(val);
    0
}

// ── 0xAF: LDA long ──
fn op_af(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs_long(bus);
    let val = cpu.read_m(bus, addr);
    cpu.set_a(val);
    cpu.set_nz_m(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xB0: BCS rel8 ──
fn op_b0(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let taken = cpu.flags.contains(Flags816::C);
    cpu.branch_rel8(bus, taken)
}

// ── 0xB1: LDA (dp),Y ──
fn op_b1(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_dp_ind_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.set_a(val);
    cpu.set_nz_m(val);
    let mut extra = if crossed { 1 } else { 0 };
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        extra += 1;
    }
    extra
}

// ── 0xB2: LDA (dp) ──
fn op_b2(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind(bus);
    let val = cpu.read_m(bus, addr);
    cpu.set_a(val);
    cpu.set_nz_m(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xB3: LDA (d,S),Y ──
fn op_b3(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_sr_ind_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.set_a(val);
    cpu.set_nz_m(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xB4: LDY dp,X ──
fn op_b4(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_x(bus);
    let val = cpu.read_x(bus, addr);
    cpu.set_y(val);
    cpu.set_nz_x(val);
    0
}

// ── 0xB5: LDA dp,X ──
fn op_b5(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.set_a(val);
    cpu.set_nz_m(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xB6: LDX dp,Y ──
fn op_b6(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_y(bus);
    let val = cpu.read_x(bus, addr);
    cpu.set_x(val);
    cpu.set_nz_x(val);
    0
}

// ── 0xB7: LDA [dp],Y ──
fn op_b7(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_long_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.set_a(val);
    cpu.set_nz_m(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xB8: CLV ──
fn op_b8(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    cpu.flags.remove(Flags816::V);
    0
}

// ── 0xB9: LDA abs,Y ──
fn op_b9(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_abs_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.set_a(val);
    cpu.set_nz_m(val);
    let mut extra = if crossed { 1 } else { 0 };
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        extra += 1;
    }
    extra
}

// ── 0xBA: TSX ──
fn op_ba(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.idx_8bit() {
        cpu.x = cpu.sp & 0xFF;
        cpu.flags.set_nz8(cpu.x as u8);
    } else {
        cpu.x = cpu.sp;
        cpu.flags.set_nz16(cpu.x);
    }
    0
}

// ── 0xBB: TYX ──
fn op_bb(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    cpu.x = cpu.y;
    if cpu.emulation || cpu.flags.idx_8bit() {
        cpu.x &= 0xFF;
        cpu.flags.set_nz8(cpu.x as u8);
    } else {
        cpu.flags.set_nz16(cpu.x);
    }
    0
}

// ── 0xBC: LDY abs,X ──
fn op_bc(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_abs_x(bus);
    let val = cpu.read_x(bus, addr);
    cpu.set_y(val);
    cpu.set_nz_x(val);
    if crossed { 1 } else { 0 }
}

// ── 0xBD: LDA abs,X ──
fn op_bd(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_abs_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.set_a(val);
    cpu.set_nz_m(val);
    let mut extra = if crossed { 1 } else { 0 };
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        extra += 1;
    }
    extra
}

// ── 0xBE: LDX abs,Y ──
fn op_be(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_abs_y(bus);
    let val = cpu.read_x(bus, addr);
    cpu.set_x(val);
    cpu.set_nz_x(val);
    if crossed { 1 } else { 0 }
}

// ── 0xBF: LDA long,X ──
fn op_bf(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs_long_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.set_a(val);
    cpu.set_nz_m(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xC0: CPY # ──
fn op_c0(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.idx_8bit() {
        let val = cpu.addr_imm8(bus) as u16;
        cpu.op_cpy(val);
        0
    } else {
        let val = cpu.addr_imm16(bus);
        cpu.op_cpy(val);
        1
    }
}

// ── 0xC1: CMP (dp,X) ──
fn op_c1(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_cmp(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xC2: REP # ──
fn op_c2(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let mask = cpu.fetch8(bus);
    cpu.op_rep(mask);
    0
}

// ── 0xC3: CMP d,S ──
fn op_c3(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_sr(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_cmp(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xC4: CPY dp ──
fn op_c4(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp(bus);
    let val = cpu.read_x(bus, addr);
    cpu.op_cpy(val);
    0
}

// ── 0xC5: CMP dp ──
fn op_c5(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_cmp(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xC6: DEC dp ──
fn op_c6(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_dp(bus), op_dec_mem)
}

// ── 0xC7: CMP [dp] ──
fn op_c7(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_long(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_cmp(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xC8: INY ──
fn op_c8(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    let val = cpu.get_y().wrapping_add(1);
    cpu.set_y(val);
    cpu.set_nz_x(val);
    0
}

// ── 0xC9: CMP # ──
fn op_c9(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.acc_8bit() {
        let val = cpu.addr_imm8(bus) as u16;
        cpu.op_cmp(val);
        0
    } else {
        let val = cpu.addr_imm16(bus);
        cpu.op_cmp(val);
        1
    }
}

// ── 0xCA: DEX ──
fn op_ca(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    let val = cpu.get_x().wrapping_sub(1);
    cpu.set_x(val);
    cpu.set_nz_x(val);
    0
}

// ── 0xCB: WAI ──
fn op_cb(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    cpu.waiting = true;
    0
}

// ── 0xCC: CPY abs ──
fn op_cc(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs(bus);
    let val = cpu.read_x(bus, addr);
    cpu.op_cpy(val);
    0
}

// ── 0xCD: CMP abs ──
fn op_cd(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_cmp(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xCE: DEC abs ──
fn op_ce(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_abs(bus), op_dec_mem)
}

// ── 0xCF: CMP long ──
fn op_cf(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs_long(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_cmp(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xD0: BNE rel8 ──
fn op_d0(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let taken = !cpu.flags.contains(Flags816::Z);
    cpu.branch_rel8(bus, taken)
}

// ── 0xD1: CMP (dp),Y ──
fn op_d1(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_dp_ind_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_cmp(val);
    let mut extra = if crossed { 1 } else { 0 };
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        extra += 1;
    }
    extra
}

// ── 0xD2: CMP (dp) ──
fn op_d2(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_cmp(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xD3: CMP (d,S),Y ──
fn op_d3(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_sr_ind_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_cmp(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xD4: PEI (dp) ──
fn op_d4(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp(bus);
    let val = cpu.read16(bus, addr);
    cpu.push16(bus, val);
    0
}

// ── 0xD5: CMP dp,X ──
fn op_d5(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_cmp(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xD6: DEC dp,X ──
fn op_d6(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_dp_x(bus), op_dec_mem)
}

// ── 0xD7: CMP [dp],Y ──
fn op_d7(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_long_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_cmp(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xD8: CLD ──
fn op_d8(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    cpu.flags.remove(Flags816::D);
    0
}

// ── 0xD9: CMP abs,Y ──
fn op_d9(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_abs_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_cmp(val);
    let mut extra = if crossed { 1 } else { 0 };
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        extra += 1;
    }
    extra
}

// ── 0xDA: PHX ──
fn op_da(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.idx_8bit() {
        cpu.push8(bus, cpu.x as u8);
        0
    } else {
        cpu.push16(bus, cpu.x);
        1
    }
}

// ── 0xDB: STP ──
fn op_db(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    cpu.stopped = true;
    0
}

// ── 0xDC: JML [abs] ──
fn op_dc(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let ptr = cpu.fetch16(bus) as u32;
    let target = cpu.read24(bus, ptr);
    cpu.pbr = (target >> 16) as u8;
    cpu.pc = target as u16;
    0
}

// ── 0xDD: CMP abs,X ──
fn op_dd(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_abs_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_cmp(val);
    let mut extra = if crossed { 1 } else { 0 };
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        extra += 1;
    }
    extra
}

// ── 0xDE: DEC abs,X ──
fn op_de(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, _) = cpu.addr_abs_x(bus);
    rmw_m!(cpu, bus, addr, op_dec_mem)
}

// ── 0xDF: CMP long,X ──
fn op_df(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs_long_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_cmp(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xE0: CPX # ──
fn op_e0(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.idx_8bit() {
        let val = cpu.addr_imm8(bus) as u16;
        cpu.op_cpx(val);
        0
    } else {
        let val = cpu.addr_imm16(bus);
        cpu.op_cpx(val);
        1
    }
}

// ── 0xE1: SBC (dp,X) ──
fn op_e1(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_sbc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xE2: SEP # ──
fn op_e2(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let mask = cpu.fetch8(bus);
    cpu.op_sep(mask);
    0
}

// ── 0xE3: SBC d,S ──
fn op_e3(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_sr(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_sbc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xE4: CPX dp ──
fn op_e4(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp(bus);
    let val = cpu.read_x(bus, addr);
    cpu.op_cpx(val);
    0
}

// ── 0xE5: SBC dp ──
fn op_e5(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_sbc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xE6: INC dp ──
fn op_e6(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_dp(bus), op_inc_mem)
}

// ── 0xE7: SBC [dp] ──
fn op_e7(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_long(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_sbc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xE8: INX ──
fn op_e8(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    let val = cpu.get_x().wrapping_add(1);
    cpu.set_x(val);
    cpu.set_nz_x(val);
    0
}

// ── 0xE9: SBC # ──
fn op_e9(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.acc_8bit() {
        let val = cpu.addr_imm8(bus) as u16;
        cpu.op_sbc(val);
        0
    } else {
        let val = cpu.addr_imm16(bus);
        cpu.op_sbc(val);
        1
    }
}

// ── 0xEA: NOP ──
fn op_ea(_cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    0
}

// ── 0xEB: XBA ──
fn op_eb(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    cpu.op_xba();
    0
}

// ── 0xEC: CPX abs ──
fn op_ec(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs(bus);
    let val = cpu.read_x(bus, addr);
    cpu.op_cpx(val);
    0
}

// ── 0xED: SBC abs ──
fn op_ed(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_sbc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xEE: INC abs ──
fn op_ee(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_abs(bus), op_inc_mem)
}

// ── 0xEF: SBC long ──
fn op_ef(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs_long(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_sbc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xF0: BEQ rel8 ──
fn op_f0(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let taken = cpu.flags.contains(Flags816::Z);
    cpu.branch_rel8(bus, taken)
}

// ── 0xF1: SBC (dp),Y ──
fn op_f1(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_dp_ind_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_sbc(val);
    let mut extra = if crossed { 1 } else { 0 };
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        extra += 1;
    }
    extra
}

// ── 0xF2: SBC (dp) ──
fn op_f2(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_sbc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xF3: SBC (d,S),Y ──
fn op_f3(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_sr_ind_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_sbc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xF4: PEA abs ──
fn op_f4(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let val = cpu.fetch16(bus);
    cpu.push16(bus, val);
    0
}

// ── 0xF5: SBC dp,X ──
fn op_f5(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_sbc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xF6: INC dp,X ──
fn op_f6(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    rmw_m!(cpu, bus, cpu.addr_dp_x(bus), op_inc_mem)
}

// ── 0xF7: SBC [dp],Y ──
fn op_f7(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_dp_ind_long_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_sbc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── 0xF8: SED ──
fn op_f8(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    cpu.flags.insert(Flags816::D);
    0
}

// ── 0xF9: SBC abs,Y ──
fn op_f9(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_abs_y(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_sbc(val);
    let mut extra = if crossed { 1 } else { 0 };
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        extra += 1;
    }
    extra
}

// ── 0xFA: PLX ──
fn op_fa(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    if cpu.emulation || cpu.flags.idx_8bit() {
        let val = cpu.pop8(bus);
        cpu.x = val as u16;
        cpu.flags.set_nz8(val);
        0
    } else {
        cpu.x = cpu.pop16(bus);
        cpu.flags.set_nz16(cpu.x);
        1
    }
}

// ── 0xFB: XCE ──
fn op_fb(cpu: &mut Cpu65816, _bus: &mut dyn Bus816) -> u8 {
    cpu.op_xce();
    0
}

// ── 0xFC: JSR (abs,X) ──
fn op_fc(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    // Return address = last byte of this 3-byte instruction.
    // PC currently points to the first operand byte, so last byte = PC + 1.
    // RTS will add 1 to this to get to the next instruction.
    cpu.push16(bus, cpu.pc.wrapping_add(1));
    let addr = cpu.addr_abs_ind_x(bus);
    cpu.pc = addr as u16;
    0
}

// ── 0xFD: SBC abs,X ──
fn op_fd(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, crossed) = cpu.addr_abs_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_sbc(val);
    let mut extra = if crossed { 1 } else { 0 };
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        extra += 1;
    }
    extra
}

// ── 0xFE: INC abs,X ──
fn op_fe(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let (addr, _) = cpu.addr_abs_x(bus);
    rmw_m!(cpu, bus, addr, op_inc_mem)
}

// ── 0xFF: SBC long,X ──
fn op_ff(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    let addr = cpu.addr_abs_long_x(bus);
    let val = cpu.read_m(bus, addr);
    cpu.op_sbc(val);
    if !cpu.emulation && !cpu.flags.acc_8bit() {
        1
    } else {
        0
    }
}

// ── Dispatch table ──────────────────────────────────────────────────────

#[rustfmt::skip]
static DISPATCH: [OpFn; 256] = [
//  x0     x1     x2     x3     x4     x5     x6     x7     x8     x9     xA     xB     xC     xD     xE     xF
    op_00, op_01, op_02, op_03, op_04, op_05, op_06, op_07, op_08, op_09, op_0a, op_0b, op_0c, op_0d, op_0e, op_0f, // 0x
    op_10, op_11, op_12, op_13, op_14, op_15, op_16, op_17, op_18, op_19, op_1a, op_1b, op_1c, op_1d, op_1e, op_1f, // 1x
    op_20, op_21, op_22, op_23, op_24, op_25, op_26, op_27, op_28, op_29, op_2a, op_2b, op_2c, op_2d, op_2e, op_2f, // 2x
    op_30, op_31, op_32, op_33, op_34, op_35, op_36, op_37, op_38, op_39, op_3a, op_3b, op_3c, op_3d, op_3e, op_3f, // 3x
    op_40, op_41, op_42, op_43, op_44, op_45, op_46, op_47, op_48, op_49, op_4a, op_4b, op_4c, op_4d, op_4e, op_4f, // 4x
    op_50, op_51, op_52, op_53, op_54, op_55, op_56, op_57, op_58, op_59, op_5a, op_5b, op_5c, op_5d, op_5e, op_5f, // 5x
    op_60, op_61, op_62, op_63, op_64, op_65, op_66, op_67, op_68, op_69, op_6a, op_6b, op_6c, op_6d, op_6e, op_6f, // 6x
    op_70, op_71, op_72, op_73, op_74, op_75, op_76, op_77, op_78, op_79, op_7a, op_7b, op_7c, op_7d, op_7e, op_7f, // 7x
    op_80, op_81, op_82, op_83, op_84, op_85, op_86, op_87, op_88, op_89, op_8a, op_8b, op_8c, op_8d, op_8e, op_8f, // 8x
    op_90, op_91, op_92, op_93, op_94, op_95, op_96, op_97, op_98, op_99, op_9a, op_9b, op_9c, op_9d, op_9e, op_9f, // 9x
    op_a0, op_a1, op_a2, op_a3, op_a4, op_a5, op_a6, op_a7, op_a8, op_a9, op_aa, op_ab, op_ac, op_ad, op_ae, op_af, // Ax
    op_b0, op_b1, op_b2, op_b3, op_b4, op_b5, op_b6, op_b7, op_b8, op_b9, op_ba, op_bb, op_bc, op_bd, op_be, op_bf, // Bx
    op_c0, op_c1, op_c2, op_c3, op_c4, op_c5, op_c6, op_c7, op_c8, op_c9, op_ca, op_cb, op_cc, op_cd, op_ce, op_cf, // Cx
    op_d0, op_d1, op_d2, op_d3, op_d4, op_d5, op_d6, op_d7, op_d8, op_d9, op_da, op_db, op_dc, op_dd, op_de, op_df, // Dx
    op_e0, op_e1, op_e2, op_e3, op_e4, op_e5, op_e6, op_e7, op_e8, op_e9, op_ea, op_eb, op_ec, op_ed, op_ee, op_ef, // Ex
    op_f0, op_f1, op_f2, op_f3, op_f4, op_f5, op_f6, op_f7, op_f8, op_f9, op_fa, op_fb, op_fc, op_fd, op_fe, op_ff, // Fx
];
