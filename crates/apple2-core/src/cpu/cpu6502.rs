use super::Flags;
use crate::bus::Bus;
use serde::{Deserialize, Serialize};

/// 6502 / 65C02 CPU state.
///
/// Corresponds to `regsrec` + the various globals in `source/CPU.h`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cpu {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub pc: u16,
    pub flags: Flags,

    /// Total accumulated cycles since power-on (g_nCumulativeCycles).
    pub cycles: u64,

    /// Pending IRQ sources bitmask.
    pub irq_pending: u32,

    /// Pending NMI sources bitmask.
    pub nmi_pending: u32,

    /// True when the CPU has hit an illegal JAM/KIL opcode (NMOS 6502 only).
    pub jammed: bool,

    /// Whether to emulate NMOS 6502 (false) or CMOS 65C02 (true) behaviour.
    pub is_65c02: bool,

    /// IRQ deferral flag (g_irqDefer1Opcode in C++).
    /// When an IRQ first asserts on the last cycle of an opcode, we defer by
    /// one opcode before taking the interrupt — matching 6502 hardware behaviour.
    pub irq_defer: bool,

    /// 65C02 WAI: CPU is halted waiting for an interrupt (IRQ or NMI).
    /// When true, the CPU does not execute instructions; it just advances
    /// time until an interrupt arrives, at which point `waiting` is cleared
    /// and the interrupt is serviced normally.
    pub waiting: bool,
}

impl Default for Cpu {
    fn default() -> Self {
        Self {
            a: 0,
            x: 0,
            y: 0,
            sp: 0xFF,
            pc: 0xFFFC, // reset vector placeholder
            flags: Flags::power_on(),
            cycles: 0,
            irq_pending: 0,
            nmi_pending: 0,
            jammed: false,
            is_65c02: true,
            irq_defer: false,
            waiting: false,
        }
    }
}

impl Cpu {
    pub fn new(is_65c02: bool) -> Self {
        Self {
            is_65c02,
            ..Default::default()
        }
    }

    /// Reset the CPU (power-cycle or warm reset).
    /// Reads the reset vector from $FFFC/$FFFD.
    pub fn reset(&mut self, bus: &mut Bus) {
        self.a = 0;
        self.x = 0;
        self.y = 0;
        self.sp = 0xFF;
        self.flags = Flags::power_on();
        self.jammed = false;
        self.waiting = false;
        self.irq_pending = 0;
        self.nmi_pending = 0;
        self.irq_defer = false;
        let lo = bus.read_raw(0xFFFC) as u16;
        let hi = bus.read_raw(0xFFFD) as u16;
        self.pc = (hi << 8) | lo;
    }

    // ── Stack helpers ────────────────────────────────────────────────────────

    #[inline]
    pub fn push(&mut self, bus: &mut Bus, val: u8) {
        bus.write_raw(0x0100 | self.sp as u16, val);
        self.sp = self.sp.wrapping_sub(1);
    }

    #[inline]
    pub fn pop(&mut self, bus: &mut Bus) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        bus.read_raw(0x0100 | self.sp as u16)
    }

    // ── Addressing modes ────────────────────────────────────────────────────

    /// Immediate: operand byte follows PC.
    #[inline]
    pub fn addr_imm(&mut self, bus: &mut Bus) -> u8 {
        let val = bus.read(self.pc, self.cycles);
        self.pc = self.pc.wrapping_add(1);
        val
    }

    /// Zero-page address.
    #[inline]
    pub fn addr_zp(&mut self, bus: &mut Bus) -> u16 {
        let zp = bus.read(self.pc, self.cycles) as u16;
        self.pc = self.pc.wrapping_add(1);
        zp
    }

    /// Zero-page,X address (wraps within page 0).
    #[inline]
    pub fn addr_zpx(&mut self, bus: &mut Bus) -> u16 {
        let zp = bus.read(self.pc, self.cycles);
        self.pc = self.pc.wrapping_add(1);
        zp.wrapping_add(self.x) as u16
    }

    /// Zero-page,Y address (wraps within page 0).
    #[inline]
    pub fn addr_zpy(&mut self, bus: &mut Bus) -> u16 {
        let zp = bus.read(self.pc, self.cycles);
        self.pc = self.pc.wrapping_add(1);
        zp.wrapping_add(self.y) as u16
    }

    /// Absolute address (2-byte little-endian following PC).
    #[inline]
    pub fn addr_abs(&mut self, bus: &mut Bus) -> u16 {
        let lo = bus.read(self.pc, self.cycles) as u16;
        self.pc = self.pc.wrapping_add(1);
        let hi = bus.read(self.pc, self.cycles) as u16;
        self.pc = self.pc.wrapping_add(1);
        (hi << 8) | lo
    }

    /// Absolute,X — returns (effective_address, page_crossed).
    #[inline]
    pub fn addr_absx(&mut self, bus: &mut Bus) -> (u16, bool) {
        let base = self.addr_abs(bus);
        let ea = base.wrapping_add(self.x as u16);
        (ea, (base & 0xFF00) != (ea & 0xFF00))
    }

    /// Absolute,Y — returns (effective_address, page_crossed).
    #[inline]
    pub fn addr_absy(&mut self, bus: &mut Bus) -> (u16, bool) {
        let base = self.addr_abs(bus);
        let ea = base.wrapping_add(self.y as u16);
        (ea, (base & 0xFF00) != (ea & 0xFF00))
    }

    /// (Indirect,X) — pre-indexed indirect.
    #[inline]
    pub fn addr_indx(&mut self, bus: &mut Bus) -> u16 {
        let zp = bus.read(self.pc, self.cycles).wrapping_add(self.x);
        self.pc = self.pc.wrapping_add(1);
        let lo = bus.read(zp as u16, self.cycles) as u16;
        let hi = bus.read(zp.wrapping_add(1) as u16, self.cycles) as u16;
        (hi << 8) | lo
    }

    /// (Indirect),Y — post-indexed indirect; returns (ea, page_crossed).
    #[inline]
    pub fn addr_indy(&mut self, bus: &mut Bus) -> (u16, bool) {
        let zp = bus.read(self.pc, self.cycles);
        self.pc = self.pc.wrapping_add(1);
        let lo = bus.read(zp as u16, self.cycles) as u16;
        let hi = bus.read(zp.wrapping_add(1) as u16, self.cycles) as u16;
        let base = (hi << 8) | lo;
        let ea = base.wrapping_add(self.y as u16);
        (ea, (base & 0xFF00) != (ea & 0xFF00))
    }

    /// 65C02: (Indirect) — zero-page indirect without index.
    #[inline]
    pub fn addr_ind_zp(&mut self, bus: &mut Bus) -> u16 {
        let zp = bus.read(self.pc, self.cycles);
        self.pc = self.pc.wrapping_add(1);
        let lo = bus.read(zp as u16, self.cycles) as u16;
        let hi = bus.read(zp.wrapping_add(1) as u16, self.cycles) as u16;
        (hi << 8) | lo
    }

    // ── Branch helper ────────────────────────────────────────────────────────

    /// Consume a relative branch operand; return (new_pc, extra_cycles).
    /// Extra cycles: +1 for taken branch, +2 if also crosses page.
    #[inline]
    pub fn branch_target(&mut self, bus: &mut Bus, taken: bool) -> u8 {
        let offset = bus.read(self.pc, self.cycles) as i8;
        self.pc = self.pc.wrapping_add(1);
        if !taken {
            return 0;
        }
        let new_pc = self.pc.wrapping_add(offset as u16);
        let extra = if (new_pc & 0xFF00) != (self.pc & 0xFF00) {
            2
        } else {
            1
        };
        self.pc = new_pc;
        extra
    }

    // ── ALU helpers ──────────────────────────────────────────────────────────

    /// ADC — add with carry, handling decimal mode for NMOS/CMOS.
    #[inline]
    pub fn op_adc(&mut self, val: u8) {
        if self.flags.contains(Flags::D) {
            self.adc_decimal(val);
        } else {
            self.adc_binary(val);
        }
    }

    #[inline]
    fn adc_binary(&mut self, val: u8) {
        let c = self.flags.contains(Flags::C) as u16;
        let sum = self.a as u16 + val as u16 + c;
        let overflow = (!(self.a ^ val) & (self.a ^ sum as u8)) & 0x80 != 0;
        self.a = sum as u8;
        self.flags.set(Flags::C, sum > 0xFF);
        self.flags.set(Flags::V, overflow);
        self.flags.set_nz(self.a);
    }

    fn adc_decimal(&mut self, val: u8) {
        // BCD addition — NMOS 6502 behaviour.
        // N, V, Z are set from the *binary* result; C is set from BCD carry.
        let c = self.flags.contains(Flags::C) as u8;

        // Binary result used for N/V/Z flags (NMOS quirk).
        let bin_sum = self.a as u16 + val as u16 + c as u16;
        self.flags.set(Flags::N, (bin_sum & 0x80) != 0);
        self.flags.set(
            Flags::V,
            ((self.a ^ val) & 0x80 == 0) && ((self.a as u16 ^ bin_sum) & 0x80 != 0),
        );
        self.flags.set(Flags::Z, (bin_sum & 0xFF) == 0);

        // Low nibble BCD correction — save lo_carry BEFORE modifying lo.
        let mut lo = (self.a & 0x0F) + (val & 0x0F) + c;
        let lo_carry = lo > 9;
        if lo_carry {
            lo = (lo + 6) & 0x0F;
        }

        // High nibble BCD correction.
        let mut hi = (self.a >> 4) + (val >> 4) + lo_carry as u8;
        let hi_carry = hi > 9;
        if hi_carry {
            hi = (hi + 6) & 0x0F;
        }
        self.flags.set(Flags::C, hi_carry);
        self.a = (hi << 4) | lo;
    }

    /// SBC — subtract with borrow.
    #[inline]
    pub fn op_sbc(&mut self, val: u8) {
        if self.flags.contains(Flags::D) {
            self.sbc_decimal(val);
        } else {
            self.adc_binary(!val); // SBC = ADC with bitwise-NOT of operand
        }
    }

    fn sbc_decimal(&mut self, val: u8) {
        // BCD subtraction — NMOS 6502 behaviour.
        // N, V, Z are set from the *binary* result; C is set from BCD borrow.
        let borrow = 1 - self.flags.contains(Flags::C) as u8;

        // Binary result used for N/V/Z flags (NMOS quirk).
        let bin_diff = self.a as i16 - val as i16 - borrow as i16;
        self.flags.set(Flags::N, (bin_diff & 0x80) != 0);
        self.flags.set(
            Flags::V,
            ((self.a as i16 ^ bin_diff) & 0x80 != 0) && ((self.a ^ val) & 0x80 != 0),
        );
        self.flags.set(Flags::Z, (bin_diff & 0xFF) == 0);

        // Low nibble BCD correction — save lo_borrow BEFORE modifying lo.
        let mut lo = (self.a & 0x0F) as i8 - (val & 0x0F) as i8 - borrow as i8;
        let lo_borrow = lo < 0;
        if lo_borrow {
            lo -= 6;
        }

        // High nibble BCD correction.
        let mut hi = (self.a >> 4) as i8 - (val >> 4) as i8 - lo_borrow as i8;
        let hi_borrow = hi < 0;
        if hi_borrow {
            hi -= 6;
        }
        self.flags.set(Flags::C, !hi_borrow);
        self.a = (((hi & 0x0F) as u8) << 4) | ((lo & 0x0F) as u8);
    }

    /// CMP / CPX / CPY — compare register with memory.
    #[inline]
    pub fn op_cmp(&mut self, reg: u8, val: u8) {
        let result = reg.wrapping_sub(val);
        self.flags.set(Flags::C, reg >= val);
        self.flags.set_nz(result);
    }

    /// ASL (memory or accumulator).
    #[inline]
    pub fn op_asl(&mut self, val: u8) -> u8 {
        self.flags.set(Flags::C, val & 0x80 != 0);
        let result = val << 1;
        self.flags.set_nz(result);
        result
    }

    /// LSR.
    #[inline]
    pub fn op_lsr(&mut self, val: u8) -> u8 {
        self.flags.set(Flags::C, val & 0x01 != 0);
        let result = val >> 1;
        self.flags.set_nz(result);
        result
    }

    /// ROL.
    #[inline]
    pub fn op_rol(&mut self, val: u8) -> u8 {
        let c_in = self.flags.contains(Flags::C) as u8;
        self.flags.set(Flags::C, val & 0x80 != 0);
        let result = (val << 1) | c_in;
        self.flags.set_nz(result);
        result
    }

    /// ROR.
    #[inline]
    pub fn op_ror(&mut self, val: u8) -> u8 {
        let c_in = (self.flags.contains(Flags::C) as u8) << 7;
        self.flags.set(Flags::C, val & 0x01 != 0);
        let result = (val >> 1) | c_in;
        self.flags.set_nz(result);
        result
    }

    /// BIT — test bits.
    #[inline]
    pub fn op_bit(&mut self, val: u8) {
        self.flags.set(Flags::N, val & 0x80 != 0);
        self.flags.set(Flags::V, val & 0x40 != 0);
        self.flags.set(Flags::Z, self.a & val == 0);
    }

    /// Interrupt service (IRQ or NMI).
    pub fn service_interrupt(&mut self, bus: &mut Bus, is_nmi: bool) {
        self.push(bus, (self.pc >> 8) as u8);
        self.push(bus, self.pc as u8);
        // Push flags with B clear for hardware interrupts
        let p = (self.flags & !Flags::B).bits() | Flags::U.bits();
        self.push(bus, p);
        self.flags.insert(Flags::I);
        let vector = if is_nmi { 0xFFFA } else { 0xFFFE };
        let lo = bus.read_raw(vector) as u16;
        let hi = bus.read_raw(vector + 1) as u16;
        self.pc = (hi << 8) | lo;
    }
}

/// Save-state snapshot of the CPU registers only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuSnapshot {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub pc: u16,
    pub flags: u8,
    pub cycles: u64,
    pub is_65c02: bool,
}

impl From<&Cpu> for CpuSnapshot {
    fn from(cpu: &Cpu) -> Self {
        Self {
            a: cpu.a,
            x: cpu.x,
            y: cpu.y,
            sp: cpu.sp,
            pc: cpu.pc,
            flags: cpu.flags.bits(),
            cycles: cpu.cycles,
            is_65c02: cpu.is_65c02,
        }
    }
}

impl Cpu {
    pub fn restore_snapshot(&mut self, snap: &CpuSnapshot) {
        self.a = snap.a;
        self.x = snap.x;
        self.y = snap.y;
        self.sp = snap.sp;
        self.pc = snap.pc;
        self.flags = Flags::from_bits_truncate(snap.flags);
        self.cycles = snap.cycles;
        self.is_65c02 = snap.is_65c02;
    }
}
