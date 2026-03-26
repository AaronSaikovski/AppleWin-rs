//! Z80 SoftCard emulation (Microsoft Z80 SoftCard, 1980).
//!
//! The real card contains a Z80B CPU running at ~2 MHz that takes over the
//! Apple II address/data bus while the 6502 is halted. When the Z80 executes
//! an OUT (0), n instruction the card releases the bus and the 6502 resumes.
//!
//! This implementation contains a self-contained Z80 interpreter covering the
//! full Z80 instruction set (main, CB, DD, ED, FD, DDCB, FDCB prefixes).
//! The Z80 operates on its own private 64 KiB RAM. On activation the bus
//! should copy Apple II main RAM into `z80_mem` before calling `execute_z80`.
//!
//! Reference: source/z80softcard.cpp, source/Z80VICE/

use std::io::{Read, Write};
use crate::card::{Card, CardType};
use crate::error::Result;

// ── Z80 clock speed relative to Apple II ──────────────────────────────────────

/// The Z80 SoftCard ran at ~2 MHz; Apple II runs at ~1.023 MHz.
/// We scale Z80 cycles proportionally: 2 Z80 cycles per Apple II cycle.
const Z80_CLOCK_RATIO: u64 = 2;

// ── Flag bit positions ─────────────────────────────────────────────────────────

const FLAG_C: u8 = 0x01; // Carry
const FLAG_N: u8 = 0x02; // Add/Subtract
const FLAG_PV: u8 = 0x04; // Parity/Overflow
const FLAG_X: u8 = 0x08; // Undocumented (bit 3 of result)
const FLAG_H: u8 = 0x10; // Half-carry
const FLAG_Y: u8 = 0x20; // Undocumented (bit 5 of result)
const FLAG_Z: u8 = 0x40; // Zero
const FLAG_S: u8 = 0x80; // Sign

// ── Parity lookup ─────────────────────────────────────────────────────────────

fn parity(v: u8) -> bool {
    v.count_ones().is_multiple_of(2)
}

// ── Z80 register file ─────────────────────────────────────────────────────────

#[derive(Clone)]
struct Regs {
    // Main registers
    a: u8, f: u8,
    b: u8, c: u8,
    d: u8, e: u8,
    h: u8, l: u8,
    // Alternate registers
    a2: u8, f2: u8,
    b2: u8, c2: u8,
    d2: u8, e2: u8,
    h2: u8, l2: u8,
    // Index registers
    ix: u16,
    iy: u16,
    // Special registers
    sp: u16,
    pc: u16,
    i:  u8,
    r:  u8,
    // Interrupt mode & flip-flops
    iff1: bool,
    iff2: bool,
    im:   u8,   // interrupt mode 0/1/2
    halted: bool,
}

impl Regs {
    fn new() -> Self {
        Self {
            a: 0xFF, f: 0xFF,
            b: 0xFF, c: 0xFF,
            d: 0xFF, e: 0xFF,
            h: 0xFF, l: 0xFF,
            a2: 0xFF, f2: 0xFF,
            b2: 0xFF, c2: 0xFF,
            d2: 0xFF, e2: 0xFF,
            h2: 0xFF, l2: 0xFF,
            ix: 0xFFFF, iy: 0xFFFF,
            sp: 0xFFFF, pc: 0x0000,
            i: 0, r: 0,
            iff1: false, iff2: false,
            im: 0,
            halted: false,
        }
    }

    fn af(&self) -> u16 { u16::from_be_bytes([self.a, self.f]) }
    fn bc(&self) -> u16 { u16::from_be_bytes([self.b, self.c]) }
    fn de(&self) -> u16 { u16::from_be_bytes([self.d, self.e]) }
    fn hl(&self) -> u16 { u16::from_be_bytes([self.h, self.l]) }

    fn set_bc(&mut self, v: u16) { let [b,c] = v.to_be_bytes(); self.b=b; self.c=c; }
    fn set_de(&mut self, v: u16) { let [d,e] = v.to_be_bytes(); self.d=d; self.e=e; }
    fn set_hl(&mut self, v: u16) { let [h,l] = v.to_be_bytes(); self.h=h; self.l=l; }
    fn set_af(&mut self, v: u16) { let [a,f] = v.to_be_bytes(); self.a=a; self.f=f; }
    fn flag(&self, mask: u8) -> bool { self.f & mask != 0 }
    fn set_flag(&mut self, mask: u8, val: bool) {
        if val { self.f |= mask; } else { self.f &= !mask; }
    }
}

// ── Serialise/deserialise Regs (raw bytes, endian-safe) ───────────────────────

fn write_regs(r: &Regs, out: &mut dyn Write) -> std::io::Result<()> {
    let data: [u8; 30] = [
        r.a, r.f, r.b, r.c, r.d, r.e, r.h, r.l,
        r.a2, r.f2, r.b2, r.c2, r.d2, r.e2, r.h2, r.l2,
        (r.ix >> 8) as u8, r.ix as u8,
        (r.iy >> 8) as u8, r.iy as u8,
        (r.sp >> 8) as u8, r.sp as u8,
        (r.pc >> 8) as u8, r.pc as u8,
        r.i, r.r,
        r.iff1 as u8, r.iff2 as u8,
        r.im,
        r.halted as u8,
    ];
    out.write_all(&data)
}

fn read_regs(r: &mut Regs, src: &mut dyn Read) -> std::io::Result<()> {
    let mut data = [0u8; 30];
    src.read_exact(&mut data)?;
    r.a=data[0]; r.f=data[1]; r.b=data[2]; r.c=data[3];
    r.d=data[4]; r.e=data[5]; r.h=data[6]; r.l=data[7];
    r.a2=data[8]; r.f2=data[9]; r.b2=data[10]; r.c2=data[11];
    r.d2=data[12]; r.e2=data[13]; r.h2=data[14]; r.l2=data[15];
    r.ix = u16::from_be_bytes([data[16], data[17]]);
    r.iy = u16::from_be_bytes([data[18], data[19]]);
    r.sp = u16::from_be_bytes([data[20], data[21]]);
    r.pc = u16::from_be_bytes([data[22], data[23]]);
    r.i=data[24]; r.r=data[25];
    r.iff1 = data[26] != 0; r.iff2 = data[27] != 0;
    r.im = data[28];
    r.halted = data[29] != 0;
    Ok(())
}

// ── Z80 CPU core ──────────────────────────────────────────────────────────────

/// Minimal I/O port interface. The SoftCard ignores most ports; the OUT (0)
/// port is used to return control to the 6502.
struct Ports {
    /// Set to true when Z80 executes OUT (n), A with n=0 → yield to 6502.
    yield_to_6502: bool,
}

impl Ports {
    fn new() -> Self { Self { yield_to_6502: false } }
    fn out(&mut self, port: u16, _val: u8) {
        if port & 0xFF == 0 { self.yield_to_6502 = true; }
    }
    fn inp(&self, _port: u16) -> u8 { 0xFF }
}

// ── CPU execution core ────────────────────────────────────────────────────────

/// Returns the number of T-states consumed for a single instruction.
/// `mem` is the full 64KiB address space.
/// Returns `None` if the Z80 should yield to the 6502 (OUT 0 executed).
fn execute_one(r: &mut Regs, mem: &mut [u8; 65536], ports: &mut Ports) -> u32 {
    if r.halted {
        return 4; // NOP equivalent while halted
    }
    r.r = r.r.wrapping_add(1);
    let op = fetch_byte(r, mem);
    decode_main(op, r, mem, ports)
}

fn fetch_byte(r: &mut Regs, mem: &[u8; 65536]) -> u8 {
    let v = mem[r.pc as usize];
    r.pc = r.pc.wrapping_add(1);
    v
}

fn fetch_word(r: &mut Regs, mem: &[u8; 65536]) -> u16 {
    let lo = fetch_byte(r, mem);
    let hi = fetch_byte(r, mem);
    u16::from_le_bytes([lo, hi])
}

fn mem_read(mem: &[u8; 65536], addr: u16) -> u8 {
    mem[addr as usize]
}

fn mem_write(mem: &mut [u8; 65536], addr: u16, val: u8) {
    mem[addr as usize] = val;
}

fn mem_read16(mem: &[u8; 65536], addr: u16) -> u16 {
    let lo = mem[addr as usize];
    let hi = mem[addr.wrapping_add(1) as usize];
    u16::from_le_bytes([lo, hi])
}

fn mem_write16(mem: &mut [u8; 65536], addr: u16, val: u16) {
    let [lo, hi] = val.to_le_bytes();
    mem[addr as usize] = lo;
    mem[addr.wrapping_add(1) as usize] = hi;
}

fn stack_push(r: &mut Regs, mem: &mut [u8; 65536], val: u16) {
    r.sp = r.sp.wrapping_sub(2);
    mem_write16(mem, r.sp, val);
}

fn stack_pop(r: &mut Regs, mem: &[u8; 65536]) -> u16 {
    let v = mem_read16(mem, r.sp);
    r.sp = r.sp.wrapping_add(2);
    v
}

// ── 8-bit register accessors (for r-field encoding) ───────────────────────────
// r = 0:B 1:C 2:D 3:E 4:H 5:L 6:(HL) 7:A

fn get_r(r: &Regs, mem: &[u8; 65536], idx: u8) -> u8 {
    match idx {
        0 => r.b, 1 => r.c, 2 => r.d, 3 => r.e,
        4 => r.h, 5 => r.l,
        6 => mem_read(mem, r.hl()),
        7 => r.a,
        _ => unreachable!(),
    }
}

fn get_r_ix(r: &Regs, mem: &[u8; 65536], idx: u8, disp: i8) -> u8 {
    match idx {
        0 => r.b, 1 => r.c, 2 => r.d, 3 => r.e,
        4 => (r.ix >> 8) as u8,   // IXH
        5 => r.ix as u8,           // IXL
        6 => mem_read(mem, r.ix.wrapping_add(disp as i16 as u16)),
        7 => r.a,
        _ => unreachable!(),
    }
}

fn get_r_iy(r: &Regs, mem: &[u8; 65536], idx: u8, disp: i8) -> u8 {
    match idx {
        0 => r.b, 1 => r.c, 2 => r.d, 3 => r.e,
        4 => (r.iy >> 8) as u8,
        5 => r.iy as u8,
        6 => mem_read(mem, r.iy.wrapping_add(disp as i16 as u16)),
        7 => r.a,
        _ => unreachable!(),
    }
}

fn set_r(r: &mut Regs, mem: &mut [u8; 65536], idx: u8, val: u8) {
    match idx {
        0 => r.b = val, 1 => r.c = val, 2 => r.d = val, 3 => r.e = val,
        4 => r.h = val, 5 => r.l = val,
        6 => { let addr = r.hl(); mem_write(mem, addr, val); }
        7 => r.a = val,
        _ => unreachable!(),
    }
}

// ── 16-bit register pair accessors (dd/qq encoding) ───────────────────────────
// dd = 0:BC 1:DE 2:HL 3:SP
// qq = 0:BC 1:DE 2:HL 3:AF

fn get_dd(r: &Regs, idx: u8) -> u16 {
    match idx { 0 => r.bc(), 1 => r.de(), 2 => r.hl(), 3 => r.sp, _ => unreachable!() }
}
fn set_dd(r: &mut Regs, idx: u8, val: u16) {
    match idx { 0 => r.set_bc(val), 1 => r.set_de(val), 2 => r.set_hl(val), 3 => r.sp = val, _ => unreachable!() }
}
fn get_qq(r: &Regs, idx: u8) -> u16 {
    match idx { 0 => r.bc(), 1 => r.de(), 2 => r.hl(), 3 => r.af(), _ => unreachable!() }
}
fn set_qq(r: &mut Regs, idx: u8, val: u16) {
    match idx { 0 => r.set_bc(val), 1 => r.set_de(val), 2 => r.set_hl(val), 3 => r.set_af(val), _ => unreachable!() }
}

// ── Condition codes ───────────────────────────────────────────────────────────
// cc = 0:NZ 1:Z 2:NC 3:C 4:PO 5:PE 6:P 7:M

fn check_cc(r: &Regs, cc: u8) -> bool {
    match cc {
        0 => !r.flag(FLAG_Z),  // NZ
        1 =>  r.flag(FLAG_Z),  // Z
        2 => !r.flag(FLAG_C),  // NC
        3 =>  r.flag(FLAG_C),  // C
        4 => !r.flag(FLAG_PV), // PO
        5 =>  r.flag(FLAG_PV), // PE
        6 => !r.flag(FLAG_S),  // P (positive)
        7 =>  r.flag(FLAG_S),  // M (minus)
        _ => unreachable!(),
    }
}

// ── ALU operations ────────────────────────────────────────────────────────────

fn alu_add(r: &mut Regs, val: u8, carry: bool) {
    let c = carry as u8;
    let result16 = (r.a as u16).wrapping_add(val as u16).wrapping_add(c as u16);
    let result = result16 as u8;
    let half = (r.a & 0x0F).wrapping_add(val & 0x0F).wrapping_add(c) > 0x0F;
    let overflow = (!(r.a ^ val) & (r.a ^ result)) & 0x80 != 0;
    r.a = result;
    r.f = 0;
    r.set_flag(FLAG_S, result & 0x80 != 0);
    r.set_flag(FLAG_Z, result == 0);
    r.set_flag(FLAG_H, half);
    r.set_flag(FLAG_PV, overflow);
    r.set_flag(FLAG_N, false);
    r.set_flag(FLAG_C, result16 > 0xFF);
    r.set_flag(FLAG_X, result & 0x08 != 0);
    r.set_flag(FLAG_Y, result & 0x20 != 0);
}

fn alu_sub(r: &mut Regs, val: u8, carry: bool) {
    let a = r.a;
    let c = carry as u8;
    let full = (a as u16).wrapping_sub(val as u16).wrapping_sub(c as u16);
    let result = full as u8;
    let half = (a & 0x0F) < (val & 0x0F) + c;
    let overflow = ((a ^ val) & (a ^ result)) & 0x80 != 0;
    r.a = result;
    r.f = 0;
    r.set_flag(FLAG_S, result & 0x80 != 0);
    r.set_flag(FLAG_Z, result == 0);
    r.set_flag(FLAG_H, half);
    r.set_flag(FLAG_PV, overflow);
    r.set_flag(FLAG_N, true);
    r.set_flag(FLAG_C, full > 0xFF); // borrow: subtraction wrapped around
    r.set_flag(FLAG_X, result & 0x08 != 0);
    r.set_flag(FLAG_Y, result & 0x20 != 0);
}

fn alu_and(r: &mut Regs, val: u8) {
    r.a &= val;
    r.f = 0;
    r.set_flag(FLAG_H, true);
    r.set_flag(FLAG_S, r.a & 0x80 != 0);
    r.set_flag(FLAG_Z, r.a == 0);
    r.set_flag(FLAG_PV, parity(r.a));
    r.set_flag(FLAG_X, r.a & 0x08 != 0);
    r.set_flag(FLAG_Y, r.a & 0x20 != 0);
}

fn alu_or(r: &mut Regs, val: u8) {
    r.a |= val;
    r.f = 0;
    r.set_flag(FLAG_S, r.a & 0x80 != 0);
    r.set_flag(FLAG_Z, r.a == 0);
    r.set_flag(FLAG_PV, parity(r.a));
    r.set_flag(FLAG_X, r.a & 0x08 != 0);
    r.set_flag(FLAG_Y, r.a & 0x20 != 0);
}

fn alu_xor(r: &mut Regs, val: u8) {
    r.a ^= val;
    r.f = 0;
    r.set_flag(FLAG_S, r.a & 0x80 != 0);
    r.set_flag(FLAG_Z, r.a == 0);
    r.set_flag(FLAG_PV, parity(r.a));
    r.set_flag(FLAG_X, r.a & 0x08 != 0);
    r.set_flag(FLAG_Y, r.a & 0x20 != 0);
}

fn alu_cp(r: &mut Regs, val: u8) {
    let saved = r.a;
    alu_sub(r, val, false);
    // CP uses val's bits 3,5 for undocumented flags (not result's)
    r.set_flag(FLAG_X, val & 0x08 != 0);
    r.set_flag(FLAG_Y, val & 0x20 != 0);
    r.a = saved;
}

fn alu_inc(r: &mut Regs, val: u8) -> u8 {
    let result = val.wrapping_add(1);
    r.set_flag(FLAG_S, result & 0x80 != 0);
    r.set_flag(FLAG_Z, result == 0);
    r.set_flag(FLAG_H, (val & 0x0F) == 0x0F);
    r.set_flag(FLAG_PV, val == 0x7F);
    r.set_flag(FLAG_N, false);
    r.set_flag(FLAG_X, result & 0x08 != 0);
    r.set_flag(FLAG_Y, result & 0x20 != 0);
    result
}

fn alu_dec(r: &mut Regs, val: u8) -> u8 {
    let result = val.wrapping_sub(1);
    r.set_flag(FLAG_S, result & 0x80 != 0);
    r.set_flag(FLAG_Z, result == 0);
    r.set_flag(FLAG_H, (val & 0x0F) == 0x00);
    r.set_flag(FLAG_PV, val == 0x80);
    r.set_flag(FLAG_N, true);
    r.set_flag(FLAG_X, result & 0x08 != 0);
    r.set_flag(FLAG_Y, result & 0x20 != 0);
    result
}

fn alu_add16(r: &mut Regs, hl: u16, val: u16) -> u16 {
    let result = (hl as u32).wrapping_add(val as u32);
    r.set_flag(FLAG_H, (hl & 0x0FFF) + (val & 0x0FFF) > 0x0FFF);
    r.set_flag(FLAG_N, false);
    r.set_flag(FLAG_C, result > 0xFFFF);
    let r16 = result as u16;
    r.set_flag(FLAG_X, r16.to_be_bytes()[0] & 0x08 != 0);
    r.set_flag(FLAG_Y, r16.to_be_bytes()[0] & 0x20 != 0);
    r16
}

// ── Rotate/shift helpers ──────────────────────────────────────────────────────

fn rlc(r: &mut Regs, val: u8) -> u8 {
    let c = val >> 7;
    let result = (val << 1) | c;
    r.set_flag(FLAG_C, c != 0); r.set_flag(FLAG_N, false); r.set_flag(FLAG_H, false);
    r.set_flag(FLAG_X, result & 0x08 != 0); r.set_flag(FLAG_Y, result & 0x20 != 0);
    result
}
fn rrc(r: &mut Regs, val: u8) -> u8 {
    let c = val & 1;
    let result = (val >> 1) | (c << 7);
    r.set_flag(FLAG_C, c != 0); r.set_flag(FLAG_N, false); r.set_flag(FLAG_H, false);
    r.set_flag(FLAG_X, result & 0x08 != 0); r.set_flag(FLAG_Y, result & 0x20 != 0);
    result
}
fn rl(r: &mut Regs, val: u8) -> u8 {
    let old_c = r.flag(FLAG_C) as u8;
    let new_c = val >> 7;
    let result = (val << 1) | old_c;
    r.set_flag(FLAG_C, new_c != 0); r.set_flag(FLAG_N, false); r.set_flag(FLAG_H, false);
    r.set_flag(FLAG_X, result & 0x08 != 0); r.set_flag(FLAG_Y, result & 0x20 != 0);
    result
}
fn rr(r: &mut Regs, val: u8) -> u8 {
    let old_c = r.flag(FLAG_C) as u8;
    let new_c = val & 1;
    let result = (val >> 1) | (old_c << 7);
    r.set_flag(FLAG_C, new_c != 0); r.set_flag(FLAG_N, false); r.set_flag(FLAG_H, false);
    r.set_flag(FLAG_X, result & 0x08 != 0); r.set_flag(FLAG_Y, result & 0x20 != 0);
    result
}
fn sla(r: &mut Regs, val: u8) -> u8 {
    let c = val >> 7;
    let result = val << 1;
    r.set_flag(FLAG_C, c != 0); r.set_flag(FLAG_N, false); r.set_flag(FLAG_H, false);
    r.set_flag(FLAG_X, result & 0x08 != 0); r.set_flag(FLAG_Y, result & 0x20 != 0);
    result
}
fn sra(r: &mut Regs, val: u8) -> u8 {
    let c = val & 1;
    let result = ((val as i8) >> 1) as u8;
    r.set_flag(FLAG_C, c != 0); r.set_flag(FLAG_N, false); r.set_flag(FLAG_H, false);
    r.set_flag(FLAG_X, result & 0x08 != 0); r.set_flag(FLAG_Y, result & 0x20 != 0);
    result
}
fn sll(r: &mut Regs, val: u8) -> u8 { // undocumented SLS/SL1
    let c = val >> 7;
    let result = (val << 1) | 1;
    r.set_flag(FLAG_C, c != 0); r.set_flag(FLAG_N, false); r.set_flag(FLAG_H, false);
    r.set_flag(FLAG_X, result & 0x08 != 0); r.set_flag(FLAG_Y, result & 0x20 != 0);
    result
}
fn srl(r: &mut Regs, val: u8) -> u8 {
    let c = val & 1;
    let result = val >> 1;
    r.set_flag(FLAG_C, c != 0); r.set_flag(FLAG_N, false); r.set_flag(FLAG_H, false);
    r.set_flag(FLAG_X, result & 0x08 != 0); r.set_flag(FLAG_Y, result & 0x20 != 0);
    result
}

fn set_szp_rot(r: &mut Regs, val: u8) {
    r.set_flag(FLAG_S, val & 0x80 != 0);
    r.set_flag(FLAG_Z, val == 0);
    r.set_flag(FLAG_PV, parity(val));
    r.set_flag(FLAG_X, val & 0x08 != 0);
    r.set_flag(FLAG_Y, val & 0x20 != 0);
}

// ── CB-prefix instruction decoder ─────────────────────────────────────────────

fn decode_cb(r: &mut Regs, mem: &mut [u8; 65536]) -> u32 {
    r.r = r.r.wrapping_add(1);
    let op = fetch_byte(r, mem);
    let reg_idx = op & 0x07;
    let val = get_r(r, mem, reg_idx);
    let cycles;

    let result = match op >> 6 {
        0 => {
            // Rotate/shift
            cycles = if reg_idx == 6 { 15 } else { 8 };
            match (op >> 3) & 0x07 {
                0 => rlc(r, val),
                1 => rrc(r, val),
                2 => rl(r, val),
                3 => rr(r, val),
                4 => sla(r, val),
                5 => sra(r, val),
                6 => sll(r, val),
                7 => srl(r, val),
                _ => unreachable!(),
            }
        }
        1 => {
            // BIT
            let bit = (op >> 3) & 0x07;
            cycles = if reg_idx == 6 { 12 } else { 8 };
            r.set_flag(FLAG_Z, val & (1 << bit) == 0);
            r.set_flag(FLAG_H, true);
            r.set_flag(FLAG_N, false);
            r.set_flag(FLAG_PV, val & (1 << bit) == 0);
            r.set_flag(FLAG_S, bit == 7 && val & 0x80 != 0);
            // Undocumented: bits 3,5 from (HL) or register
            r.set_flag(FLAG_X, val & 0x08 != 0);
            r.set_flag(FLAG_Y, val & 0x20 != 0);
            return cycles; // BIT doesn't store result
        }
        2 => {
            // RES
            let bit = (op >> 3) & 0x07;
            cycles = if reg_idx == 6 { 15 } else { 8 };
            val & !(1 << bit)
        }
        3 => {
            // SET
            let bit = (op >> 3) & 0x07;
            cycles = if reg_idx == 6 { 15 } else { 8 };
            val | (1 << bit)
        }
        _ => unreachable!(),
    };

    // For rotate/shift: set SZP flags
    if op >> 6 == 0 {
        set_szp_rot(r, result);
    }

    set_r(r, mem, reg_idx, result);
    cycles
}

// ── DDCB / FDCB prefix decoder ────────────────────────────────────────────────

fn decode_xycb(r: &mut Regs, mem: &mut [u8; 65536], base: u16) -> u32 {
    let disp = fetch_byte(r, mem) as i8;
    r.r = r.r.wrapping_add(1);
    let op = fetch_byte(r, mem);
    let addr = base.wrapping_add(disp as i16 as u16);
    let val = mem_read(mem, addr);
    let reg_idx = op & 0x07;

    let result = match op >> 6 {
        0 => {
            match (op >> 3) & 0x07 {
                0 => rlc(r, val),
                1 => rrc(r, val),
                2 => rl(r, val),
                3 => rr(r, val),
                4 => sla(r, val),
                5 => sra(r, val),
                6 => sll(r, val),
                7 => srl(r, val),
                _ => unreachable!(),
            }
        }
        1 => {
            let bit = (op >> 3) & 0x07;
            r.set_flag(FLAG_Z, val & (1 << bit) == 0);
            r.set_flag(FLAG_H, true);
            r.set_flag(FLAG_N, false);
            r.set_flag(FLAG_PV, val & (1 << bit) == 0);
            r.set_flag(FLAG_S, bit == 7 && val & 0x80 != 0);
            r.set_flag(FLAG_X, addr.to_be_bytes()[0] & 0x08 != 0);
            r.set_flag(FLAG_Y, addr.to_be_bytes()[0] & 0x20 != 0);
            return 20;
        }
        2 => {
            let bit = (op >> 3) & 0x07;
            val & !(1 << bit)
        }
        3 => {
            let bit = (op >> 3) & 0x07;
            val | (1 << bit)
        }
        _ => unreachable!(),
    };

    if op >> 6 == 0 { set_szp_rot(r, result); }
    mem_write(mem, addr, result);
    // Also store to reg if reg_idx != 6
    if reg_idx != 6 { set_r(r, mem, reg_idx, result); }
    23
}

// ── ED-prefix instruction decoder ─────────────────────────────────────────────

fn decode_ed(r: &mut Regs, mem: &mut [u8; 65536], ports: &mut Ports) -> u32 {
    r.r = r.r.wrapping_add(1);
    let op = fetch_byte(r, mem);
    match op {
        // IN r, (C)
        0x40 | 0x48 | 0x50 | 0x58 | 0x60 | 0x68 | 0x78 => {
            let reg = (op >> 3) & 0x07;
            let val = ports.inp(r.c as u16);
            set_r(r, mem, reg, val);
            r.set_flag(FLAG_S, val & 0x80 != 0);
            r.set_flag(FLAG_Z, val == 0);
            r.set_flag(FLAG_H, false);
            r.set_flag(FLAG_PV, parity(val));
            r.set_flag(FLAG_N, false);
            12
        }
        0x70 => { // IN (C) — result discarded
            let val = ports.inp(r.c as u16);
            r.set_flag(FLAG_S, val & 0x80 != 0);
            r.set_flag(FLAG_Z, val == 0);
            r.set_flag(FLAG_H, false);
            r.set_flag(FLAG_PV, parity(val));
            r.set_flag(FLAG_N, false);
            12
        }
        // OUT (C), r
        0x41 | 0x49 | 0x51 | 0x59 | 0x61 | 0x69 | 0x79 => {
            let reg = (op >> 3) & 0x07;
            let val = get_r(r, mem, reg);
            let port = u16::from_be_bytes([r.b, r.c]);
            ports.out(port, val);
            12
        }
        0x71 => { // OUT (C), 0
            let port = u16::from_be_bytes([r.b, r.c]);
            ports.out(port, 0);
            12
        }
        // SBC HL, rr
        0x42 | 0x52 | 0x62 | 0x72 => {
            let dd = (op >> 4) & 0x03;
            let val = get_dd(r, dd);
            let hl = r.hl();
            let c = r.flag(FLAG_C) as u16;
            let result = (hl as u32).wrapping_sub(val as u32).wrapping_sub(c as u32);
            let r16 = result as u16;
            r.set_flag(FLAG_S, r16 & 0x8000 != 0);
            r.set_flag(FLAG_Z, r16 == 0);
            r.set_flag(FLAG_H, (hl & 0x0FFF) < (val & 0x0FFF) + c);
            r.set_flag(FLAG_PV, ((hl ^ val) & (hl ^ r16)) & 0x8000 != 0);
            r.set_flag(FLAG_N, true);
            r.set_flag(FLAG_C, result > 0xFFFF);
            r.set_flag(FLAG_X, r16.to_be_bytes()[0] & 0x08 != 0);
            r.set_flag(FLAG_Y, r16.to_be_bytes()[0] & 0x20 != 0);
            r.set_hl(r16);
            15
        }
        // ADC HL, rr
        0x4A | 0x5A | 0x6A | 0x7A => {
            let dd = (op >> 4) & 0x03;
            let val = get_dd(r, dd);
            let hl = r.hl();
            let c = r.flag(FLAG_C) as u16;
            let result = (hl as u32).wrapping_add(val as u32).wrapping_add(c as u32);
            let r16 = result as u16;
            r.set_flag(FLAG_S, r16 & 0x8000 != 0);
            r.set_flag(FLAG_Z, r16 == 0);
            r.set_flag(FLAG_H, (hl & 0x0FFF) + (val & 0x0FFF) + c > 0x0FFF);
            r.set_flag(FLAG_PV, (!(hl ^ val) & (hl ^ r16)) & 0x8000 != 0);
            r.set_flag(FLAG_N, false);
            r.set_flag(FLAG_C, result > 0xFFFF);
            r.set_flag(FLAG_X, r16.to_be_bytes()[0] & 0x08 != 0);
            r.set_flag(FLAG_Y, r16.to_be_bytes()[0] & 0x20 != 0);
            r.set_hl(r16);
            15
        }
        // LD (nn), rr
        0x43 | 0x53 | 0x63 | 0x73 => {
            let dd = (op >> 4) & 0x03;
            let addr = fetch_word(r, mem);
            let val = get_dd(r, dd);
            mem_write16(mem, addr, val);
            20
        }
        // LD rr, (nn)
        0x4B | 0x5B | 0x6B | 0x7B => {
            let dd = (op >> 4) & 0x03;
            let addr = fetch_word(r, mem);
            let val = mem_read16(mem, addr);
            set_dd(r, dd, val);
            20
        }
        // NEG
        0x44 | 0x4C | 0x54 | 0x5C | 0x64 | 0x6C | 0x74 | 0x7C => {
            let a = r.a;
            r.a = 0;
            alu_sub(r, a, false);
            r.set_flag(FLAG_PV, a == 0x80);
            r.set_flag(FLAG_C, a != 0);
            8
        }
        // RETN / RETI
        0x45 | 0x4D | 0x55 | 0x5D | 0x65 | 0x6D | 0x75 | 0x7D => {
            r.iff1 = r.iff2;
            r.pc = stack_pop(r, mem);
            14
        }
        // IM 0 / IM 1 / IM 2
        0x46 | 0x4E | 0x66 | 0x6E => { r.im = 0; 8 }
        0x56 | 0x76 => { r.im = 1; 8 }
        0x5E | 0x7E => { r.im = 2; 8 }
        // LD I, A
        0x47 => { r.i = r.a; 9 }
        // LD R, A
        0x4F => { r.r = r.a; 9 }
        // LD A, I
        0x57 => {
            let v = r.i;
            r.a = v;
            r.set_flag(FLAG_S, v & 0x80 != 0);
            r.set_flag(FLAG_Z, v == 0);
            r.set_flag(FLAG_H, false);
            r.set_flag(FLAG_PV, r.iff2);
            r.set_flag(FLAG_N, false);
            9
        }
        // LD A, R
        0x5F => {
            let v = r.r;
            r.a = v;
            r.set_flag(FLAG_S, v & 0x80 != 0);
            r.set_flag(FLAG_Z, v == 0);
            r.set_flag(FLAG_H, false);
            r.set_flag(FLAG_PV, r.iff2);
            r.set_flag(FLAG_N, false);
            9
        }
        // RRD
        0x67 => {
            let hl = r.hl();
            let m = mem_read(mem, hl);
            let new_m = (r.a << 4) | (m >> 4);
            r.a = (r.a & 0xF0) | (m & 0x0F);
            mem_write(mem, hl, new_m);
            r.set_flag(FLAG_S, r.a & 0x80 != 0);
            r.set_flag(FLAG_Z, r.a == 0);
            r.set_flag(FLAG_H, false);
            r.set_flag(FLAG_PV, parity(r.a));
            r.set_flag(FLAG_N, false);
            18
        }
        // RLD
        0x6F => {
            let hl = r.hl();
            let m = mem_read(mem, hl);
            let new_m = (m << 4) | (r.a & 0x0F);
            r.a = (r.a & 0xF0) | (m >> 4);
            mem_write(mem, hl, new_m);
            r.set_flag(FLAG_S, r.a & 0x80 != 0);
            r.set_flag(FLAG_Z, r.a == 0);
            r.set_flag(FLAG_H, false);
            r.set_flag(FLAG_PV, parity(r.a));
            r.set_flag(FLAG_N, false);
            18
        }
        // LDI
        0xA0 => {
            let v = mem_read(mem, r.de());
            mem_write(mem, r.hl(), v);
            let hl = r.hl().wrapping_add(1); r.set_hl(hl);
            let de = r.de().wrapping_add(1); r.set_de(de);
            let bc = r.bc().wrapping_sub(1); r.set_bc(bc);
            r.set_flag(FLAG_H, false);
            r.set_flag(FLAG_N, false);
            r.set_flag(FLAG_PV, bc != 0);
            let n = r.a.wrapping_add(v);
            r.set_flag(FLAG_X, n & 0x08 != 0);
            r.set_flag(FLAG_Y, n & 0x02 != 0);
            16
        }
        // CPI
        0xA1 => {
            let v = mem_read(mem, r.hl());
            let result = r.a.wrapping_sub(v);
            let hl = r.hl().wrapping_add(1); r.set_hl(hl);
            let bc = r.bc().wrapping_sub(1); r.set_bc(bc);
            r.set_flag(FLAG_S, result & 0x80 != 0);
            r.set_flag(FLAG_Z, result == 0);
            r.set_flag(FLAG_H, (r.a & 0x0F) < (v & 0x0F));
            r.set_flag(FLAG_PV, bc != 0);
            r.set_flag(FLAG_N, true);
            let n = result.wrapping_sub(r.flag(FLAG_H) as u8);
            r.set_flag(FLAG_X, n & 0x08 != 0);
            r.set_flag(FLAG_Y, n & 0x02 != 0);
            16
        }
        // INI
        0xA2 => {
            let port = u16::from_be_bytes([r.b, r.c]);
            let v = ports.inp(port);
            let hl = r.hl();
            mem_write(mem, hl, v);
            let hl = hl.wrapping_add(1); r.set_hl(hl);
            r.b = r.b.wrapping_sub(1);
            r.set_flag(FLAG_Z, r.b == 0);
            r.set_flag(FLAG_N, true);
            16
        }
        // OUTI
        0xA3 => {
            let v = mem_read(mem, r.hl());
            r.b = r.b.wrapping_sub(1);
            let port = u16::from_be_bytes([r.b, r.c]);
            ports.out(port, v);
            let hl = r.hl().wrapping_add(1); r.set_hl(hl);
            r.set_flag(FLAG_Z, r.b == 0);
            r.set_flag(FLAG_N, true);
            16
        }
        // LDD
        0xA8 => {
            let v = mem_read(mem, r.de());
            mem_write(mem, r.hl(), v);
            let hl = r.hl().wrapping_sub(1); r.set_hl(hl);
            let de = r.de().wrapping_sub(1); r.set_de(de);
            let bc = r.bc().wrapping_sub(1); r.set_bc(bc);
            r.set_flag(FLAG_H, false);
            r.set_flag(FLAG_N, false);
            r.set_flag(FLAG_PV, bc != 0);
            16
        }
        // CPD
        0xA9 => {
            let v = mem_read(mem, r.hl());
            let result = r.a.wrapping_sub(v);
            let hl = r.hl().wrapping_sub(1); r.set_hl(hl);
            let bc = r.bc().wrapping_sub(1); r.set_bc(bc);
            r.set_flag(FLAG_S, result & 0x80 != 0);
            r.set_flag(FLAG_Z, result == 0);
            r.set_flag(FLAG_H, (r.a & 0x0F) < (v & 0x0F));
            r.set_flag(FLAG_PV, bc != 0);
            r.set_flag(FLAG_N, true);
            16
        }
        // IND
        0xAA => {
            let port = u16::from_be_bytes([r.b, r.c]);
            let v = ports.inp(port);
            let hl = r.hl();
            mem_write(mem, hl, v);
            let hl = hl.wrapping_sub(1); r.set_hl(hl);
            r.b = r.b.wrapping_sub(1);
            r.set_flag(FLAG_Z, r.b == 0);
            r.set_flag(FLAG_N, true);
            16
        }
        // OUTD
        0xAB => {
            let v = mem_read(mem, r.hl());
            r.b = r.b.wrapping_sub(1);
            let port = u16::from_be_bytes([r.b, r.c]);
            ports.out(port, v);
            let hl = r.hl().wrapping_sub(1); r.set_hl(hl);
            r.set_flag(FLAG_Z, r.b == 0);
            r.set_flag(FLAG_N, true);
            16
        }
        // LDIR
        0xB0 => {
            let v = mem_read(mem, r.de());
            mem_write(mem, r.hl(), v);
            let hl = r.hl().wrapping_add(1); r.set_hl(hl);
            let de = r.de().wrapping_add(1); r.set_de(de);
            let bc = r.bc().wrapping_sub(1); r.set_bc(bc);
            r.set_flag(FLAG_H, false);
            r.set_flag(FLAG_N, false);
            r.set_flag(FLAG_PV, false);
            if bc != 0 {
                r.pc = r.pc.wrapping_sub(2);
                21
            } else {
                16
            }
        }
        // CPIR
        0xB1 => {
            let v = mem_read(mem, r.hl());
            let result = r.a.wrapping_sub(v);
            let hl = r.hl().wrapping_add(1); r.set_hl(hl);
            let bc = r.bc().wrapping_sub(1); r.set_bc(bc);
            r.set_flag(FLAG_S, result & 0x80 != 0);
            r.set_flag(FLAG_Z, result == 0);
            r.set_flag(FLAG_H, (r.a & 0x0F) < (v & 0x0F));
            r.set_flag(FLAG_N, true);
            r.set_flag(FLAG_PV, bc != 0);
            if bc != 0 && result != 0 {
                r.pc = r.pc.wrapping_sub(2);
                21
            } else {
                16
            }
        }
        // INIR
        0xB2 => {
            let port = u16::from_be_bytes([r.b, r.c]);
            let v = ports.inp(port);
            let hl = r.hl();
            mem_write(mem, hl, v);
            let hl = hl.wrapping_add(1); r.set_hl(hl);
            r.b = r.b.wrapping_sub(1);
            r.set_flag(FLAG_Z, r.b == 0);
            r.set_flag(FLAG_N, true);
            if r.b != 0 { r.pc = r.pc.wrapping_sub(2); 21 } else { 16 }
        }
        // OTIR
        0xB3 => {
            let v = mem_read(mem, r.hl());
            r.b = r.b.wrapping_sub(1);
            let port = u16::from_be_bytes([r.b, r.c]);
            ports.out(port, v);
            let hl = r.hl().wrapping_add(1); r.set_hl(hl);
            r.set_flag(FLAG_Z, r.b == 0);
            r.set_flag(FLAG_N, true);
            if r.b != 0 { r.pc = r.pc.wrapping_sub(2); 21 } else { 16 }
        }
        // LDDR
        0xB8 => {
            let v = mem_read(mem, r.de());
            mem_write(mem, r.hl(), v);
            let hl = r.hl().wrapping_sub(1); r.set_hl(hl);
            let de = r.de().wrapping_sub(1); r.set_de(de);
            let bc = r.bc().wrapping_sub(1); r.set_bc(bc);
            r.set_flag(FLAG_H, false);
            r.set_flag(FLAG_N, false);
            r.set_flag(FLAG_PV, false);
            if bc != 0 { r.pc = r.pc.wrapping_sub(2); 21 } else { 16 }
        }
        // CPDR
        0xB9 => {
            let v = mem_read(mem, r.hl());
            let result = r.a.wrapping_sub(v);
            let hl = r.hl().wrapping_sub(1); r.set_hl(hl);
            let bc = r.bc().wrapping_sub(1); r.set_bc(bc);
            r.set_flag(FLAG_S, result & 0x80 != 0);
            r.set_flag(FLAG_Z, result == 0);
            r.set_flag(FLAG_H, (r.a & 0x0F) < (v & 0x0F));
            r.set_flag(FLAG_N, true);
            r.set_flag(FLAG_PV, bc != 0);
            if bc != 0 && result != 0 { r.pc = r.pc.wrapping_sub(2); 21 } else { 16 }
        }
        // INDR
        0xBA => {
            let port = u16::from_be_bytes([r.b, r.c]);
            let v = ports.inp(port);
            let hl = r.hl();
            mem_write(mem, hl, v);
            let hl = hl.wrapping_sub(1); r.set_hl(hl);
            r.b = r.b.wrapping_sub(1);
            r.set_flag(FLAG_Z, r.b == 0);
            r.set_flag(FLAG_N, true);
            if r.b != 0 { r.pc = r.pc.wrapping_sub(2); 21 } else { 16 }
        }
        // OTDR
        0xBB => {
            let v = mem_read(mem, r.hl());
            r.b = r.b.wrapping_sub(1);
            let port = u16::from_be_bytes([r.b, r.c]);
            ports.out(port, v);
            let hl = r.hl().wrapping_sub(1); r.set_hl(hl);
            r.set_flag(FLAG_Z, r.b == 0);
            r.set_flag(FLAG_N, true);
            if r.b != 0 { r.pc = r.pc.wrapping_sub(2); 21 } else { 16 }
        }
        _ => 8, // undefined ED opcodes: NOP-like
    }
}

// ── DD/FD prefix helpers ───────────────────────────────────────────────────────

fn decode_dd_fd(r: &mut Regs, mem: &mut [u8; 65536], ports: &mut Ports, use_ix: bool) -> u32 {
    r.r = r.r.wrapping_add(1);
    let op = fetch_byte(r, mem);
    let xy = if use_ix { r.ix } else { r.iy };

    macro_rules! get_xy_h { () => { (xy >> 8) as u8 } }
    macro_rules! get_xy_l { () => { xy as u8 } }
    macro_rules! set_xy { ($val:expr) => {
        if use_ix { r.ix = $val; } else { r.iy = $val; }
    } }
    macro_rules! get_d { () => { fetch_byte(r, mem) as i8 } }

    match op {
        // ADD IX/IY, rr
        0x09 | 0x19 | 0x29 | 0x39 => {
            let dd = (op >> 4) & 0x03;
            let val = if dd == 2 { xy } else { get_dd(r, dd) };
            let result = alu_add16(r, xy, val);
            set_xy!(result);
            15
        }
        // LD IX/IY, nn
        0x21 => { let nn = fetch_word(r, mem); set_xy!(nn); 14 }
        // LD (nn), IX/IY
        0x22 => { let addr = fetch_word(r, mem); mem_write16(mem, addr, xy); 20 }
        // INC IX/IY
        0x23 => { set_xy!(xy.wrapping_add(1)); 10 }
        // INC IXH/IYH
        0x24 => {
            let v = alu_inc(r, get_xy_h!()); set_xy!((xy & 0x00FF) | ((v as u16) << 8)); 8
        }
        // DEC IXH/IYH
        0x25 => {
            let v = alu_dec(r, get_xy_h!()); set_xy!((xy & 0x00FF) | ((v as u16) << 8)); 8
        }
        // LD IXH/IYH, n
        0x26 => { let n = fetch_byte(r, mem); set_xy!((xy & 0x00FF) | ((n as u16) << 8)); 11 }
        // LD IX/IY, (nn)
        0x2A => { let addr = fetch_word(r, mem); set_xy!(mem_read16(mem, addr)); 20 }
        // DEC IX/IY
        0x2B => { set_xy!(xy.wrapping_sub(1)); 10 }
        // INC IXL/IYL
        0x2C => {
            let v = alu_inc(r, get_xy_l!()); set_xy!((xy & 0xFF00) | v as u16); 8
        }
        // DEC IXL/IYL
        0x2D => {
            let v = alu_dec(r, get_xy_l!()); set_xy!((xy & 0xFF00) | v as u16); 8
        }
        // LD IXL/IYL, n
        0x2E => { let n = fetch_byte(r, mem); set_xy!((xy & 0xFF00) | n as u16); 11 }
        // INC (IX+d)
        0x34 => {
            let d = get_d!();
            let addr = xy.wrapping_add(d as i16 as u16);
            let v = alu_inc(r, mem_read(mem, addr));
            mem_write(mem, addr, v);
            23
        }
        // DEC (IX+d)
        0x35 => {
            let d = get_d!();
            let addr = xy.wrapping_add(d as i16 as u16);
            let v = alu_dec(r, mem_read(mem, addr));
            mem_write(mem, addr, v);
            23
        }
        // LD (IX+d), n
        0x36 => {
            let d = get_d!();
            let n = fetch_byte(r, mem);
            let addr = xy.wrapping_add(d as i16 as u16);
            mem_write(mem, addr, n);
            19
        }
        // LD r, (IX+d) — 0x46, 0x4E, 0x56, 0x5E, 0x66, 0x6E, 0x7E
        op if (op & 0xC7) == 0x46 => {
            let d = get_d!();
            let addr = xy.wrapping_add(d as i16 as u16);
            let reg = (op >> 3) & 0x07;
            if reg == 6 { // LD H/L with (IX+d) doesn't exist in this form, treat as LD (IX+d)
                // Actually LD (IX+d) is a store; 0x76 is HALT, skip
                return 4;
            }
            let val = mem_read(mem, addr);
            set_r(r, mem, reg, val);
            19
        }
        // LD (IX+d), r — 0x70..0x77 excluding 0x76 (HALT)
        op if (op & 0xF8) == 0x70 && op != 0x76 => {
            let d = get_d!();
            let addr = xy.wrapping_add(d as i16 as u16);
            let reg = op & 0x07;
            let val = get_r(r, mem, reg);
            mem_write(mem, addr, val);
            19
        }
        // LD IXH/IYH, r (undocumented) — 0x60..0x67
        op if (0x60..=0x67).contains(&op) => {
            let src = op & 0x07;
            let val = if use_ix { get_r_ix(r, mem, src, 0) } else { get_r_iy(r, mem, src, 0) };
            set_xy!((xy & 0x00FF) | ((val as u16) << 8));
            8
        }
        // LD IXL/IYL, r (undocumented) — 0x68..0x6F
        op if (0x68..=0x6F).contains(&op) => {
            let src = op & 0x07;
            let val = if use_ix { get_r_ix(r, mem, src, 0) } else { get_r_iy(r, mem, src, 0) };
            set_xy!((xy & 0xFF00) | val as u16);
            8
        }
        // ALU group with IX/IY substitution: 0x80..0xBF where r=4 or r=5 means IXH/IYH, IXL/IYL
        // and r=6 means (IX+d)
        op if (0x80..=0xBF).contains(&op) => {
            let src = op & 0x07;
            let disp = if src == 6 { fetch_byte(r, mem) as i8 } else { 0 };
            let val = if use_ix { get_r_ix(r, mem, src, disp) } else { get_r_iy(r, mem, src, disp) };
            let cycles = if src == 6 { 19 } else { 8 };
            match (op >> 3) & 0x07 {
                0 => alu_add(r, val, false),
                1 => alu_add(r, val, true),
                2 => alu_sub(r, val, false),
                3 => alu_sub(r, val, true),
                4 => alu_and(r, val),
                5 => alu_xor(r, val),
                6 => alu_or(r, val),
                7 => alu_cp(r, val),
                _ => unreachable!(),
            }
            cycles
        }
        // PUSH IX/IY
        0xE5 => { stack_push(r, mem, xy); 15 }
        // POP IX/IY
        0xE1 => { let v = stack_pop(r, mem); set_xy!(v); 14 }
        // EX (SP), IX/IY
        0xE3 => {
            let old = mem_read16(mem, r.sp);
            mem_write16(mem, r.sp, xy);
            set_xy!(old);
            23
        }
        // JP (IX/IY)
        0xE9 => { r.pc = xy; 8 }
        // LD SP, IX/IY
        0xF9 => { r.sp = xy; 10 }
        // DDCB / FDCB
        0xCB => decode_xycb(r, mem, xy),
        // Anything else: treat the prefix as a NOP and re-decode the opcode
        // as a normal main instruction
        _ => decode_main(op, r, mem, ports),
    }
}

// ── Main instruction decoder ───────────────────────────────────────────────────

#[allow(clippy::too_many_lines)]
fn decode_main(op: u8, r: &mut Regs, mem: &mut [u8; 65536], ports: &mut Ports) -> u32 {
    match op {
        // NOP
        0x00 => 4,

        // LD rr, nn
        0x01 | 0x11 | 0x21 | 0x31 => {
            let dd = (op >> 4) & 0x03;
            let nn = fetch_word(r, mem);
            set_dd(r, dd, nn);
            10
        }
        // LD (BC), A
        0x02 => { let a = r.a; mem_write(mem, r.bc(), a); 7 }
        // INC rr
        0x03 | 0x13 | 0x23 | 0x33 => {
            let dd = (op >> 4) & 0x03;
            let v = get_dd(r, dd).wrapping_add(1); set_dd(r, dd, v); 6
        }
        // INC r (0x04, 0x0C, 0x14, 0x1C, 0x24, 0x2C, 0x34, 0x3C)
        op if op & 0xC7 == 0x04 => {
            let reg = (op >> 3) & 0x07;
            let v = get_r(r, mem, reg);
            let result = alu_inc(r, v);
            set_r(r, mem, reg, result);
            if reg == 6 { 11 } else { 4 }
        }
        // DEC r (0x05, 0x0D, 0x15, 0x1D, 0x25, 0x2D, 0x35, 0x3D)
        op if op & 0xC7 == 0x05 => {
            let reg = (op >> 3) & 0x07;
            let v = get_r(r, mem, reg);
            let result = alu_dec(r, v);
            set_r(r, mem, reg, result);
            if reg == 6 { 11 } else { 4 }
        }
        // LD r, n (0x06, 0x0E, 0x16, 0x1E, 0x26, 0x2E, 0x36, 0x3E)
        op if op & 0xC7 == 0x06 => {
            let reg = (op >> 3) & 0x07;
            let n = fetch_byte(r, mem);
            set_r(r, mem, reg, n);
            if reg == 6 { 10 } else { 7 }
        }
        // RLCA
        0x07 => {
            let c = r.a >> 7;
            r.a = (r.a << 1) | c;
            r.set_flag(FLAG_C, c != 0);
            r.set_flag(FLAG_N, false);
            r.set_flag(FLAG_H, false);
            r.set_flag(FLAG_X, r.a & 0x08 != 0);
            r.set_flag(FLAG_Y, r.a & 0x20 != 0);
            4
        }
        // EX AF, AF'
        0x08 => {
            core::mem::swap(&mut r.a, &mut r.a2);
            core::mem::swap(&mut r.f, &mut r.f2);
            4
        }
        // ADD HL, rr
        0x09 | 0x19 | 0x29 | 0x39 => {
            let dd = (op >> 4) & 0x03;
            let val = get_dd(r, dd);
            let hl = r.hl();
            let result = alu_add16(r, hl, val);
            r.set_hl(result);
            11
        }
        // LD A, (BC)
        0x0A => { r.a = mem_read(mem, r.bc()); 7 }
        // DEC rr
        0x0B | 0x1B | 0x2B | 0x3B => {
            let dd = (op >> 4) & 0x03;
            let v = get_dd(r, dd).wrapping_sub(1); set_dd(r, dd, v); 6
        }
        // RRCA
        0x0F => {
            let c = r.a & 1;
            r.a = (r.a >> 1) | (c << 7);
            r.set_flag(FLAG_C, c != 0);
            r.set_flag(FLAG_N, false);
            r.set_flag(FLAG_H, false);
            r.set_flag(FLAG_X, r.a & 0x08 != 0);
            r.set_flag(FLAG_Y, r.a & 0x20 != 0);
            4
        }
        // DJNZ e
        0x10 => {
            let e = fetch_byte(r, mem) as i8;
            r.b = r.b.wrapping_sub(1);
            if r.b != 0 {
                r.pc = r.pc.wrapping_add(e as i16 as u16);
                13
            } else {
                8
            }
        }
        // LD (DE), A
        0x12 => { let a = r.a; mem_write(mem, r.de(), a); 7 }
        // RLA
        0x17 => {
            let old_c = r.flag(FLAG_C) as u8;
            let new_c = r.a >> 7;
            r.a = (r.a << 1) | old_c;
            r.set_flag(FLAG_C, new_c != 0);
            r.set_flag(FLAG_N, false);
            r.set_flag(FLAG_H, false);
            r.set_flag(FLAG_X, r.a & 0x08 != 0);
            r.set_flag(FLAG_Y, r.a & 0x20 != 0);
            4
        }
        // JR e
        0x18 => {
            let e = fetch_byte(r, mem) as i8;
            r.pc = r.pc.wrapping_add(e as i16 as u16);
            12
        }
        // LD A, (DE)
        0x1A => { r.a = mem_read(mem, r.de()); 7 }
        // RRA
        0x1F => {
            let old_c = r.flag(FLAG_C) as u8;
            let new_c = r.a & 1;
            r.a = (r.a >> 1) | (old_c << 7);
            r.set_flag(FLAG_C, new_c != 0);
            r.set_flag(FLAG_N, false);
            r.set_flag(FLAG_H, false);
            r.set_flag(FLAG_X, r.a & 0x08 != 0);
            r.set_flag(FLAG_Y, r.a & 0x20 != 0);
            4
        }
        // JR NZ/Z/NC/C (0x20, 0x28, 0x30, 0x38)
        0x20 | 0x28 | 0x30 | 0x38 => {
            let e = fetch_byte(r, mem) as i8;
            let cc = (op >> 3) & 0x03; // 0:NZ 1:Z 2:NC 3:C
            let taken = check_cc(r, cc);
            if taken { r.pc = r.pc.wrapping_add(e as i16 as u16); 12 } else { 7 }
        }
        // LD (nn), HL
        0x22 => {
            let addr = fetch_word(r, mem);
            let hl = r.hl();
            mem_write16(mem, addr, hl);
            16
        }
        // DAA
        0x27 => {
            let mut a = r.a;
            let c = r.flag(FLAG_C);
            let h = r.flag(FLAG_H);
            let n = r.flag(FLAG_N);
            let mut adj = 0u8;
            let mut new_c = false;
            if !n {
                if h || (a & 0x0F) > 9 { adj |= 0x06; }
                if c || a > 0x99 { adj |= 0x60; new_c = true; }
                a = a.wrapping_add(adj);
            } else {
                if h { adj |= 0x06; }
                if c { adj |= 0x60; new_c = true; }
                a = a.wrapping_sub(adj);
            }
            r.a = a;
            r.set_flag(FLAG_S, a & 0x80 != 0);
            r.set_flag(FLAG_Z, a == 0);
            r.set_flag(FLAG_H, false); // simplified; full impl checks nibble
            r.set_flag(FLAG_PV, parity(a));
            r.set_flag(FLAG_C, new_c);
            r.set_flag(FLAG_X, a & 0x08 != 0);
            r.set_flag(FLAG_Y, a & 0x20 != 0);
            4
        }
        // LD HL, (nn)
        0x2A => {
            let addr = fetch_word(r, mem);
            let v = mem_read16(mem, addr);
            r.set_hl(v);
            16
        }
        // CPL
        0x2F => {
            r.a = !r.a;
            r.set_flag(FLAG_H, true);
            r.set_flag(FLAG_N, true);
            r.set_flag(FLAG_X, r.a & 0x08 != 0);
            r.set_flag(FLAG_Y, r.a & 0x20 != 0);
            4
        }
        // LD (nn), A
        0x32 => {
            let addr = fetch_word(r, mem);
            let a = r.a;
            mem_write(mem, addr, a);
            13
        }
        // SCF
        0x37 => {
            r.set_flag(FLAG_C, true);
            r.set_flag(FLAG_N, false);
            r.set_flag(FLAG_H, false);
            r.set_flag(FLAG_X, r.a & 0x08 != 0);
            r.set_flag(FLAG_Y, r.a & 0x20 != 0);
            4
        }
        // LD A, (nn)
        0x3A => {
            let addr = fetch_word(r, mem);
            r.a = mem_read(mem, addr);
            13
        }
        // CCF
        0x3F => {
            let old_c = r.flag(FLAG_C);
            r.set_flag(FLAG_H, old_c);
            r.set_flag(FLAG_C, !old_c);
            r.set_flag(FLAG_N, false);
            r.set_flag(FLAG_X, r.a & 0x08 != 0);
            r.set_flag(FLAG_Y, r.a & 0x20 != 0);
            4
        }
        // HALT
        0x76 => { r.halted = true; 4 }
        // LD r, r' (0x40–0x7F excluding 0x76)
        op if (0x40..=0x7F).contains(&op) => {
            let dst = (op >> 3) & 0x07;
            let src = op & 0x07;
            let val = get_r(r, mem, src);
            set_r(r, mem, dst, val);
            if dst == 6 || src == 6 { 7 } else { 4 }
        }
        // ALU ops with register (0x80–0xBF)
        op if (0x80..=0xBF).contains(&op) => {
            let src = op & 0x07;
            let val = get_r(r, mem, src);
            let cycles = if src == 6 { 7 } else { 4 };
            match (op >> 3) & 0x07 {
                0 => alu_add(r, val, false),
                1 => alu_add(r, val, true),
                2 => alu_sub(r, val, false),
                3 => alu_sub(r, val, true),
                4 => alu_and(r, val),
                5 => alu_xor(r, val),
                6 => alu_or(r, val),
                7 => alu_cp(r, val),
                _ => unreachable!(),
            }
            cycles
        }
        // RET cc (0xC0, 0xC8, 0xD0, 0xD8, 0xE0, 0xE8, 0xF0, 0xF8)
        op if op & 0xC7 == 0xC0 => {
            let cc = (op >> 3) & 0x07;
            if check_cc(r, cc) { r.pc = stack_pop(r, mem); 11 } else { 5 }
        }
        // POP rr (0xC1, 0xD1, 0xE1, 0xF1)
        op if op & 0xCF == 0xC1 => {
            let qq = (op >> 4) & 0x03;
            let v = stack_pop(r, mem);
            set_qq(r, qq, v);
            10
        }
        // JP cc, nn (0xC2, 0xCA, 0xD2, 0xDA, 0xE2, 0xEA, 0xF2, 0xFA)
        op if op & 0xC7 == 0xC2 => {
            let cc = (op >> 3) & 0x07;
            let nn = fetch_word(r, mem);
            if check_cc(r, cc) { r.pc = nn; }
            10
        }
        // JP nn
        0xC3 => { let nn = fetch_word(r, mem); r.pc = nn; 10 }
        // CB prefix
        0xCB => decode_cb(r, mem),
        // CALL cc, nn (0xC4, 0xCC, 0xD4, 0xDC, 0xE4, 0xEC, 0xF4, 0xFC)
        op if op & 0xC7 == 0xC4 => {
            let cc = (op >> 3) & 0x07;
            let nn = fetch_word(r, mem);
            if check_cc(r, cc) {
                let pc = r.pc;
                stack_push(r, mem, pc);
                r.pc = nn;
                17
            } else {
                10
            }
        }
        // PUSH rr (0xC5, 0xD5, 0xE5, 0xF5)
        op if op & 0xCF == 0xC5 => {
            let qq = (op >> 4) & 0x03;
            let v = get_qq(r, qq);
            stack_push(r, mem, v);
            11
        }
        // ALU ops with immediate (0xC6, 0xCE, 0xD6, 0xDE, 0xE6, 0xEE, 0xF6, 0xFE)
        op if op & 0xC7 == 0xC6 => {
            let n = fetch_byte(r, mem);
            match (op >> 3) & 0x07 {
                0 => alu_add(r, n, false),
                1 => alu_add(r, n, true),
                2 => alu_sub(r, n, false),
                3 => alu_sub(r, n, true),
                4 => alu_and(r, n),
                5 => alu_xor(r, n),
                6 => alu_or(r, n),
                7 => alu_cp(r, n),
                _ => unreachable!(),
            }
            7
        }
        // RST p (0xC7, 0xCF, 0xD7, 0xDF, 0xE7, 0xEF, 0xF7, 0xFF)
        op if op & 0xC7 == 0xC7 => {
            let p = (op & 0x38) as u16;
            let pc = r.pc;
            stack_push(r, mem, pc);
            r.pc = p;
            11
        }
        // RET
        0xC9 => { r.pc = stack_pop(r, mem); 10 }
        // DD prefix (IX)
        0xDD => decode_dd_fd(r, mem, ports, true),
        // ED prefix
        0xED => decode_ed(r, mem, ports),
        // FD prefix (IY)
        0xFD => decode_dd_fd(r, mem, ports, false),
        // OUT (n), A
        0xD3 => {
            let n = fetch_byte(r, mem);
            let port = u16::from_be_bytes([r.a, n]);
            let a = r.a;
            ports.out(port, a);
            11
        }
        // IN A, (n)
        0xDB => {
            let n = fetch_byte(r, mem);
            let port = u16::from_be_bytes([r.a, n]);
            r.a = ports.inp(port);
            11
        }
        // EX (SP), HL
        0xE3 => {
            let old = mem_read16(mem, r.sp);
            let hl = r.hl();
            mem_write16(mem, r.sp, hl);
            r.set_hl(old);
            19
        }
        // JP (HL)
        0xE9 => { r.pc = r.hl(); 4 }
        // EX DE, HL
        0xEB => {
            let de = r.de();
            let hl = r.hl();
            r.set_de(hl);
            r.set_hl(de);
            4
        }
        // EXX
        0xD9 => {
            core::mem::swap(&mut r.b, &mut r.b2);
            core::mem::swap(&mut r.c, &mut r.c2);
            core::mem::swap(&mut r.d, &mut r.d2);
            core::mem::swap(&mut r.e, &mut r.e2);
            core::mem::swap(&mut r.h, &mut r.h2);
            core::mem::swap(&mut r.l, &mut r.l2);
            4
        }
        // DI
        0xF3 => { r.iff1 = false; r.iff2 = false; 4 }
        // EI
        0xFB => { r.iff1 = true; r.iff2 = true; 4 }
        // LD SP, HL
        0xF9 => { r.sp = r.hl(); 6 }
        // CALL nn
        0xCD => {
            let nn = fetch_word(r, mem);
            let pc = r.pc;
            stack_push(r, mem, pc);
            r.pc = nn;
            17
        }
        // All remaining undefined opcodes → NOP
        _ => 4,
    }
}

// ── Z80Card public struct ─────────────────────────────────────────────────────

/// Microsoft Z80 SoftCard emulation.
///
/// The card contains a Z80 CPU and its own 64 KiB address space. When
/// activated (z80_active == true) the host should call `execute_z80` each
/// quantum to run Z80 instructions. The Z80 stops when it executes an
/// `OUT (0), A` instruction (the SoftCard protocol for returning to 6502).
pub struct Z80Card {
    slot: usize,
    /// True when Z80 owns the bus (6502 is halted).
    z80_active: bool,
    /// Z80 register file.
    regs: Regs,
    /// Private 64 KiB RAM for the Z80. CP/M loads here.
    mem: Box<[u8; 65536]>,
}

impl Z80Card {
    pub fn new(slot: usize) -> Self {
        Self {
            slot,
            z80_active: false,
            regs: Regs::new(),
            mem: Box::new([0u8; 65536]),
        }
    }

    /// True if the Z80 is currently active (6502 halted).
    pub fn z80_active(&self) -> bool { self.z80_active }

    /// Copy a slice of Apple II main RAM into the Z80 address space.
    ///
    /// Call this before activating the Z80 so it has access to the same
    /// memory image. `src` must be at most 65536 bytes; the copy starts at
    /// Z80 address 0.
    pub fn load_from_apple_ram(&mut self, src: &[u8]) {
        let len = src.len().min(65536);
        self.mem[..len].copy_from_slice(&src[..len]);
    }

    /// Copy Z80 memory back to a slice of Apple II main RAM.
    ///
    /// Call this after the Z80 yields so that any writes made by CP/M
    /// programs become visible to the 6502.
    pub fn store_to_apple_ram(&self, dst: &mut [u8]) {
        let len = dst.len().min(65536);
        dst[..len].copy_from_slice(&self.mem[..len]);
    }

    /// Execute Z80 instructions for approximately `z80_cycles` T-states.
    ///
    /// Returns the number of T-states actually consumed. Sets `z80_active` to
    /// false (yielding to 6502) if the Z80 executes `OUT (0), A`.
    pub fn execute_z80(&mut self, z80_cycles: u64) -> u64 {
        if !self.z80_active { return 0; }
        let mut consumed: u64 = 0;
        let mut ports = Ports::new();
        while consumed < z80_cycles {
            let ticks = execute_one(&mut self.regs, &mut self.mem, &mut ports) as u64;
            consumed += ticks;
            if ports.yield_to_6502 {
                self.z80_active = false;
                break;
            }
        }
        consumed
    }

    /// Direct access to Z80 memory (for testing / BIOS setup).
    pub fn z80_mem(&self) -> &[u8; 65536] { &self.mem }

    /// Mutable access to Z80 memory.
    pub fn z80_mem_mut(&mut self) -> &mut [u8; 65536] { &mut self.mem }

    /// Current Z80 program counter.
    pub fn pc(&self) -> u16 { self.regs.pc }

    /// Set Z80 program counter (e.g. to start execution at a specific address).
    pub fn set_pc(&mut self, pc: u16) { self.regs.pc = pc; }
}

// ── Card trait ────────────────────────────────────────────────────────────────

impl Card for Z80Card {
    fn card_type(&self) -> CardType { CardType::Z80 }
    fn slot(&self) -> usize { self.slot }

    fn io_read(&mut self, _offset: u8, _cycles: u64) -> u8 { 0xFF }
    fn io_write(&mut self, _offset: u8, _value: u8, _cycles: u64) {}

    /// Reading $C0x4 (where x = slot + 8) activates the Z80 (halts 6502).
    /// This matches the real SoftCard's ROM-select/Z80-activate strobe.
    fn slot_io_read(&mut self, reg: u8, _cycles: u64) -> u8 {
        if reg == 0x04 { self.z80_active = true; }
        0xFF
    }

    /// Writing any value to reg 0 of the slot I/O space allows the host to
    /// force-clear the Z80 active flag (soft reset path).
    fn slot_io_write(&mut self, reg: u8, _val: u8, _cycles: u64) {
        if reg == 0x00 { self.z80_active = false; }
    }

    fn reset(&mut self, power_cycle: bool) {
        self.z80_active = false;
        self.regs = Regs::new();
        if power_cycle {
            self.mem.fill(0);
        }
    }

    /// Called once per Apple II execution quantum (~17 030 cycles at 1 MHz).
    /// Drives Z80 execution at 2× clock ratio when active.
    fn update(&mut self, cycles: u64) {
        if !self.z80_active { return; }
        let z80_cycles = cycles.saturating_mul(Z80_CLOCK_RATIO);
        self.execute_z80(z80_cycles);
    }

    // ── Save/load state ───────────────────────────────────────────────────────

    fn save_state(&self, out: &mut dyn Write) -> Result<()> {
        out.write_all(&[1u8])?; // format version
        out.write_all(&[self.z80_active as u8])?;
        write_regs(&self.regs, out)?;
        out.write_all(self.mem.as_ref())?;
        Ok(())
    }

    fn load_state(&mut self, src: &mut dyn Read, _version: u32) -> Result<()> {
        let mut ver = [0u8; 1];
        src.read_exact(&mut ver)?;
        // version byte is reserved for future format changes
        let mut flag = [0u8; 1];
        src.read_exact(&mut flag)?;
        self.z80_active = flag[0] != 0;
        read_regs(&mut self.regs, src)?;
        src.read_exact(self.mem.as_mut())?;
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}
