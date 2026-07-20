//! 65C816 instruction implementations (ALU, load/store, transfer, control flow).
//!
//! Each instruction operates on the CPU state and bus, respecting the current
//! register width mode (8-bit vs 16-bit) as controlled by the M and X flags.

use super::Bus816;
use super::flags816::Flags816;
use super::registers::Cpu65816;

impl Cpu65816 {
    // ── Memory read/write helpers (width-aware) ─────────────────────────

    /// Address of the second byte of a 16-bit operand access. Accesses that
    /// begin in bank 0 (direct-page and stack-relative modes) wrap the high
    /// byte within bank 0 — e.g. a 16-bit access at `$00:FFFF` reads its high
    /// byte from `$00:0000`, not `$01:0000`. Accesses in any other bank use a
    /// full 24-bit increment.
    #[inline]
    fn hi_byte_addr(addr: u32) -> u32 {
        if addr & 0xFF_0000 == 0 {
            addr.wrapping_add(1) & 0xFFFF
        } else {
            addr.wrapping_add(1)
        }
    }

    /// Read an 8-bit or 16-bit value from memory, depending on M flag.
    #[inline]
    pub fn read_m(&self, bus: &mut dyn Bus816, addr: u32) -> u16 {
        if self.emulation || self.flags.acc_8bit() {
            bus.read(addr, self.cycles) as u16
        } else {
            let lo = bus.read(addr, self.cycles) as u16;
            let hi = bus.read(Self::hi_byte_addr(addr), self.cycles) as u16;
            (hi << 8) | lo
        }
    }

    /// Write an 8-bit or 16-bit value to memory, depending on M flag.
    #[inline]
    pub fn write_m(&self, bus: &mut dyn Bus816, addr: u32, val: u16) {
        if self.emulation || self.flags.acc_8bit() {
            bus.write(addr, val as u8, self.cycles);
        } else {
            bus.write(addr, val as u8, self.cycles);
            bus.write(Self::hi_byte_addr(addr), (val >> 8) as u8, self.cycles);
        }
    }

    /// Read an 8-bit or 16-bit value from memory, depending on X flag.
    #[inline]
    pub fn read_x(&self, bus: &mut dyn Bus816, addr: u32) -> u16 {
        if self.emulation || self.flags.idx_8bit() {
            bus.read(addr, self.cycles) as u16
        } else {
            let lo = bus.read(addr, self.cycles) as u16;
            let hi = bus.read(Self::hi_byte_addr(addr), self.cycles) as u16;
            (hi << 8) | lo
        }
    }

    /// Write an 8-bit or 16-bit value to memory, depending on X flag.
    #[inline]
    pub fn write_x(&self, bus: &mut dyn Bus816, addr: u32, val: u16) {
        if self.emulation || self.flags.idx_8bit() {
            bus.write(addr, val as u8, self.cycles);
        } else {
            bus.write(addr, val as u8, self.cycles);
            bus.write(Self::hi_byte_addr(addr), (val >> 8) as u8, self.cycles);
        }
    }

    /// Set N/Z flags from a value, respecting M flag width.
    #[inline]
    pub fn set_nz_m(&mut self, val: u16) {
        if self.emulation || self.flags.acc_8bit() {
            self.flags.set_nz8(val as u8);
        } else {
            self.flags.set_nz16(val);
        }
    }

    /// Set N/Z flags from a value, respecting X flag width.
    #[inline]
    pub fn set_nz_x(&mut self, val: u16) {
        if self.emulation || self.flags.idx_8bit() {
            self.flags.set_nz8(val as u8);
        } else {
            self.flags.set_nz16(val);
        }
    }

    // ── ADC ─────────────────────────────────────────────────────────────

    /// ADC — add with carry, handling 8-bit/16-bit and decimal mode.
    pub fn op_adc(&mut self, val: u16) {
        if self.emulation || self.flags.acc_8bit() {
            self.adc8(val as u8);
        } else {
            self.adc16(val);
        }
    }

    fn adc8(&mut self, val: u8) {
        let a = (self.c & 0xFF) as u8;
        let c = self.flags.contains(Flags816::C) as u8;

        if self.flags.contains(Flags816::D) {
            // BCD 8-bit. On the 65C02/65816 the N, V and Z flags reflect the
            // decimal-adjusted result (not the binary sum, as on the NMOS 6502):
            // N/Z come from the final BCD byte, C from the decimal carry, and V
            // from the intermediate value before the high nibble's +6 adjust.
            let lo_sum = (a & 0x0F) + (val & 0x0F) + c;
            let lo_carry = lo_sum > 9;
            let lo_res = if lo_carry {
                (lo_sum + 6) & 0x0F
            } else {
                lo_sum
            };
            let hi_raw = (a >> 4) + (val >> 4) + lo_carry as u8;
            // Intermediate (pre-high-adjust) value used only for the V flag.
            let inter = (hi_raw << 4) | lo_res;
            self.flags
                .set(Flags816::V, (!(a ^ val) & (a ^ inter)) & 0x80 != 0);
            let hi_carry = hi_raw > 9;
            let hi_res = if hi_carry {
                (hi_raw + 6) & 0x0F
            } else {
                hi_raw
            };
            let result = (hi_res << 4) | lo_res;
            self.flags.set(Flags816::N, (result & 0x80) != 0);
            self.flags.set(Flags816::Z, result == 0);
            self.flags.set(Flags816::C, hi_carry);
            self.c = (self.c & 0xFF00) | result as u16;
        } else {
            let sum = a as u16 + val as u16 + c as u16;
            let result = sum as u8;
            self.flags.set(Flags816::C, sum > 0xFF);
            self.flags
                .set(Flags816::V, (!(a ^ val) & (a ^ result)) & 0x80 != 0);
            self.flags.set_nz8(result);
            self.c = (self.c & 0xFF00) | result as u16;
        }
    }

    fn adc16(&mut self, val: u16) {
        let a = self.c;
        let c = self.flags.contains(Flags816::C) as u16;

        if self.flags.contains(Flags816::D) {
            // BCD 16-bit — see `adc8` for the flag semantics. Each nibble
            // produces exactly one decimal carry (0/1); the previous code used
            // `sum >> 4`, which yields 2 for invalid BCD digits and corrupts the
            // carry chain. N/Z come from the final result, V from the pre-adjust
            // top nibble.
            let mut result: u32 = 0;
            let mut carry = c as u32;
            let mut inter: u32 = 0;
            for nibble in 0..4 {
                let shift = nibble * 4;
                let a_nib = ((a >> shift) & 0xF) as u32;
                let v_nib = ((val >> shift) & 0xF) as u32;
                let sum = a_nib + v_nib + carry;
                let nib_carry = sum > 9;
                let res_nib = if nib_carry { (sum + 6) & 0xF } else { sum };
                if nibble == 3 {
                    // Intermediate for V: raw (pre-+6) top nibble over the low
                    // 12 bits of the decimal result.
                    inter = (result & 0x0FFF) | (sum << 12);
                }
                result |= res_nib << shift;
                carry = nib_carry as u32;
            }
            let result = result as u16;
            let inter = inter as u16;
            self.flags
                .set(Flags816::V, (!(a ^ val) & (a ^ inter)) & 0x8000 != 0);
            self.flags.set(Flags816::N, (result & 0x8000) != 0);
            self.flags.set(Flags816::Z, result == 0);
            self.flags.set(Flags816::C, carry != 0);
            self.c = result;
        } else {
            let sum = a as u32 + val as u32 + c as u32;
            let result = sum as u16;
            self.flags.set(Flags816::C, sum > 0xFFFF);
            self.flags
                .set(Flags816::V, (!(a ^ val) & (a ^ result)) & 0x8000 != 0);
            self.flags.set_nz16(result);
            self.c = result;
        }
    }

    // ── SBC ─────────────────────────────────────────────────────────────

    /// SBC — subtract with borrow.
    pub fn op_sbc(&mut self, val: u16) {
        if self.emulation || self.flags.acc_8bit() {
            self.sbc8(val as u8);
        } else {
            self.sbc16(val);
        }
    }

    fn sbc8(&mut self, val: u8) {
        let a = (self.c & 0xFF) as u8;
        let borrow = 1 - self.flags.contains(Flags816::C) as u8;

        if self.flags.contains(Flags816::D) {
            // On the 65C02/65816 SBC, V follows the binary subtraction, but N
            // and Z follow the decimal-adjusted result (verified against the
            // SingleStepTests 65816 vectors).
            let bin = a as i16 - val as i16 - borrow as i16;
            self.flags.set(
                Flags816::V,
                ((a as i16 ^ bin) & 0x80 != 0) && ((a ^ val) & 0x80 != 0),
            );

            let mut lo = (a & 0x0F) as i8 - (val & 0x0F) as i8 - borrow as i8;
            let lo_borrow = lo < 0;
            if lo_borrow {
                lo -= 6;
            }
            let mut hi = (a >> 4) as i8 - (val >> 4) as i8 - lo_borrow as i8;
            let hi_borrow = hi < 0;
            if hi_borrow {
                hi -= 6;
            }
            let result = (((hi & 0x0F) as u8) << 4) | ((lo & 0x0F) as u8);
            self.flags.set(Flags816::N, (result & 0x80) != 0);
            self.flags.set(Flags816::Z, result == 0);
            self.flags.set(Flags816::C, !hi_borrow);
            self.c = (self.c & 0xFF00) | result as u16;
        } else {
            // Binary: SBC = ADC of complement
            let sum = a as u16 + (!val) as u16 + self.flags.contains(Flags816::C) as u16;
            let result = sum as u8;
            self.flags.set(Flags816::C, sum > 0xFF);
            self.flags
                .set(Flags816::V, (!(a ^ !val) & (a ^ result)) & 0x80 != 0);
            self.flags.set_nz8(result);
            self.c = (self.c & 0xFF00) | result as u16;
        }
    }

    fn sbc16(&mut self, val: u16) {
        let a = self.c;
        let c = self.flags.contains(Flags816::C) as u16;

        if self.flags.contains(Flags816::D) {
            // See `sbc8`: V follows the binary subtraction, N and Z follow the
            // decimal-adjusted result.
            let bin = a as i32 - val as i32 - (1 - c as i32);
            self.flags.set(
                Flags816::V,
                ((a as i32 ^ bin) & 0x8000 != 0) && ((a ^ val) & 0x8000 != 0),
            );

            let mut result: u32 = 0;
            let mut borrow: i32 = 1 - c as i32;
            for nibble in 0..4 {
                let shift = nibble * 4;
                let a_nib = ((a >> shift) & 0xF) as i32;
                let v_nib = ((val >> shift) & 0xF) as i32;
                let mut diff = a_nib - v_nib - borrow;
                if diff < 0 {
                    diff -= 6;
                    borrow = 1;
                } else {
                    borrow = 0;
                }
                result |= (diff as u32 & 0xF) << shift;
            }
            let result = result as u16;
            self.flags.set(Flags816::N, (result & 0x8000) != 0);
            self.flags.set(Flags816::Z, result == 0);
            self.flags.set(Flags816::C, borrow == 0);
            self.c = result;
        } else {
            let sum = a as u32 + (!val) as u32 + c as u32;
            let result = sum as u16;
            self.flags.set(Flags816::C, sum > 0xFFFF);
            self.flags
                .set(Flags816::V, (!(a ^ !val) & (a ^ result)) & 0x8000 != 0);
            self.flags.set_nz16(result);
            self.c = result;
        }
    }

    // ── CMP / CPX / CPY ─────────────────────────────────────────────────

    /// CMP — compare accumulator with memory (width follows M flag).
    pub fn op_cmp(&mut self, val: u16) {
        if self.emulation || self.flags.acc_8bit() {
            let a = (self.c & 0xFF) as u8;
            let v = val as u8;
            let result = a.wrapping_sub(v);
            self.flags.set(Flags816::C, a >= v);
            self.flags.set_nz8(result);
        } else {
            let a = self.c;
            let result = a.wrapping_sub(val);
            self.flags.set(Flags816::C, a >= val);
            self.flags.set_nz16(result);
        }
    }

    /// CPX — compare X with memory (width follows X flag).
    pub fn op_cpx(&mut self, val: u16) {
        if self.emulation || self.flags.idx_8bit() {
            let x = (self.x & 0xFF) as u8;
            let v = val as u8;
            let result = x.wrapping_sub(v);
            self.flags.set(Flags816::C, x >= v);
            self.flags.set_nz8(result);
        } else {
            let x = self.x;
            let result = x.wrapping_sub(val);
            self.flags.set(Flags816::C, x >= val);
            self.flags.set_nz16(result);
        }
    }

    /// CPY — compare Y with memory (width follows X flag).
    pub fn op_cpy(&mut self, val: u16) {
        if self.emulation || self.flags.idx_8bit() {
            let y = (self.y & 0xFF) as u8;
            let v = val as u8;
            let result = y.wrapping_sub(v);
            self.flags.set(Flags816::C, y >= v);
            self.flags.set_nz8(result);
        } else {
            let y = self.y;
            let result = y.wrapping_sub(val);
            self.flags.set(Flags816::C, y >= val);
            self.flags.set_nz16(result);
        }
    }

    // ── Shifts and rotates ──────────────────────────────────────────────

    /// ASL — arithmetic shift left (M-width).
    pub fn op_asl(&mut self, val: u16) -> u16 {
        if self.emulation || self.flags.acc_8bit() {
            let v = val as u8;
            self.flags.set(Flags816::C, v & 0x80 != 0);
            let result = v << 1;
            self.flags.set_nz8(result);
            result as u16
        } else {
            self.flags.set(Flags816::C, val & 0x8000 != 0);
            let result = val << 1;
            self.flags.set_nz16(result);
            result
        }
    }

    /// LSR — logical shift right (M-width).
    pub fn op_lsr(&mut self, val: u16) -> u16 {
        if self.emulation || self.flags.acc_8bit() {
            let v = val as u8;
            self.flags.set(Flags816::C, v & 0x01 != 0);
            let result = v >> 1;
            self.flags.set_nz8(result);
            result as u16
        } else {
            self.flags.set(Flags816::C, val & 0x0001 != 0);
            let result = val >> 1;
            self.flags.set_nz16(result);
            result
        }
    }

    /// ROL — rotate left (M-width).
    pub fn op_rol(&mut self, val: u16) -> u16 {
        let c_in = self.flags.contains(Flags816::C);
        if self.emulation || self.flags.acc_8bit() {
            let v = val as u8;
            self.flags.set(Flags816::C, v & 0x80 != 0);
            let result = (v << 1) | c_in as u8;
            self.flags.set_nz8(result);
            result as u16
        } else {
            self.flags.set(Flags816::C, val & 0x8000 != 0);
            let result = (val << 1) | c_in as u16;
            self.flags.set_nz16(result);
            result
        }
    }

    /// ROR — rotate right (M-width).
    pub fn op_ror(&mut self, val: u16) -> u16 {
        let c_in = self.flags.contains(Flags816::C);
        if self.emulation || self.flags.acc_8bit() {
            let v = val as u8;
            self.flags.set(Flags816::C, v & 0x01 != 0);
            let result = (v >> 1) | ((c_in as u8) << 7);
            self.flags.set_nz8(result);
            result as u16
        } else {
            self.flags.set(Flags816::C, val & 0x0001 != 0);
            let result = (val >> 1) | ((c_in as u16) << 15);
            self.flags.set_nz16(result);
            result
        }
    }

    // ── BIT ─────────────────────────────────────────────────────────────

    /// BIT — test bits (M-width).
    pub fn op_bit(&mut self, val: u16) {
        if self.emulation || self.flags.acc_8bit() {
            let v = val as u8;
            let a = (self.c & 0xFF) as u8;
            self.flags.set(Flags816::N, v & 0x80 != 0);
            self.flags.set(Flags816::V, v & 0x40 != 0);
            self.flags.set(Flags816::Z, a & v == 0);
        } else {
            self.flags.set(Flags816::N, val & 0x8000 != 0);
            self.flags.set(Flags816::V, val & 0x4000 != 0);
            self.flags.set(Flags816::Z, self.c & val == 0);
        }
    }

    /// BIT immediate — only sets Z flag, does not affect N/V.
    pub fn op_bit_imm(&mut self, val: u16) {
        if self.emulation || self.flags.acc_8bit() {
            self.flags
                .set(Flags816::Z, (self.c as u8) & (val as u8) == 0);
        } else {
            self.flags.set(Flags816::Z, self.c & val == 0);
        }
    }

    // ── INC / DEC ───────────────────────────────────────────────────────

    /// INC memory (M-width).
    pub fn op_inc_mem(&mut self, val: u16) -> u16 {
        if self.emulation || self.flags.acc_8bit() {
            let result = (val as u8).wrapping_add(1);
            self.flags.set_nz8(result);
            result as u16
        } else {
            let result = val.wrapping_add(1);
            self.flags.set_nz16(result);
            result
        }
    }

    /// DEC memory (M-width).
    pub fn op_dec_mem(&mut self, val: u16) -> u16 {
        if self.emulation || self.flags.acc_8bit() {
            let result = (val as u8).wrapping_sub(1);
            self.flags.set_nz8(result);
            result as u16
        } else {
            let result = val.wrapping_sub(1);
            self.flags.set_nz16(result);
            result
        }
    }

    // ── Logic (AND, ORA, EOR) ───────────────────────────────────────────

    /// AND — accumulator AND memory (M-width).
    pub fn op_and(&mut self, val: u16) {
        if self.emulation || self.flags.acc_8bit() {
            let result = (self.c as u8) & (val as u8);
            self.c = (self.c & 0xFF00) | result as u16;
            self.flags.set_nz8(result);
        } else {
            self.c &= val;
            self.flags.set_nz16(self.c);
        }
    }

    /// ORA — accumulator OR memory (M-width).
    pub fn op_ora(&mut self, val: u16) {
        if self.emulation || self.flags.acc_8bit() {
            let result = (self.c as u8) | (val as u8);
            self.c = (self.c & 0xFF00) | result as u16;
            self.flags.set_nz8(result);
        } else {
            self.c |= val;
            self.flags.set_nz16(self.c);
        }
    }

    /// EOR — accumulator XOR memory (M-width).
    pub fn op_eor(&mut self, val: u16) {
        if self.emulation || self.flags.acc_8bit() {
            let result = (self.c as u8) ^ (val as u8);
            self.c = (self.c & 0xFF00) | result as u16;
            self.flags.set_nz8(result);
        } else {
            self.c ^= val;
            self.flags.set_nz16(self.c);
        }
    }

    // ── TRB / TSB ───────────────────────────────────────────────────────

    /// TRB — test and reset bits (M-width).
    pub fn op_trb(&mut self, val: u16) -> u16 {
        if self.emulation || self.flags.acc_8bit() {
            let a = (self.c & 0xFF) as u8;
            self.flags.set(Flags816::Z, a & (val as u8) == 0);
            (val as u8 & !a) as u16
        } else {
            self.flags.set(Flags816::Z, self.c & val == 0);
            val & !self.c
        }
    }

    /// TSB — test and set bits (M-width).
    pub fn op_tsb(&mut self, val: u16) -> u16 {
        if self.emulation || self.flags.acc_8bit() {
            let a = (self.c & 0xFF) as u8;
            self.flags.set(Flags816::Z, a & (val as u8) == 0);
            (val as u8 | a) as u16
        } else {
            self.flags.set(Flags816::Z, self.c & val == 0);
            val | self.c
        }
    }

    // ── 65C816-specific instructions ────────────────────────────────────

    /// XBA — exchange B and A (high and low bytes of C).
    pub fn op_xba(&mut self) {
        let lo = self.c & 0xFF;
        let hi = self.c >> 8;
        self.c = (lo << 8) | hi;
        // N and Z are set from the NEW A (low byte), always 8-bit.
        self.flags.set_nz8(self.c as u8);
    }

    /// XCE — exchange carry and emulation flags.
    pub fn op_xce(&mut self) {
        let old_carry = self.flags.contains(Flags816::C);
        let old_emulation = self.emulation;
        self.flags.set(Flags816::C, old_emulation);
        if old_carry {
            self.enter_emulation();
        } else {
            self.enter_native();
        }
    }

    /// REP — reset (clear) processor status bits.
    pub fn op_rep(&mut self, mask: u8) {
        let new_bits = self.flags.bits() & !mask;
        self.flags = Flags816::from_bits_truncate(new_bits);
        self.update_mode_flags();
    }

    /// SEP — set processor status bits.
    pub fn op_sep(&mut self, mask: u8) {
        let new_bits = self.flags.bits() | mask;
        self.flags = Flags816::from_bits_truncate(new_bits);
        self.update_mode_flags();
    }

    /// MVN — block move next (decrementing source and dest banks in operand).
    /// C = number of bytes - 1. Moves C+1 bytes from src to dst.
    pub fn op_mvn(&mut self, bus: &mut dyn Bus816) {
        let dst_bank = self.fetch8(bus) as u32;
        let src_bank = self.fetch8(bus) as u32;

        let src_addr = (src_bank << 16) | self.x as u32;
        let dst_addr = (dst_bank << 16) | self.y as u32;
        let val = bus.read(src_addr, self.cycles);
        bus.write(dst_addr, val, self.cycles);

        // Increment X and Y
        if self.emulation || self.flags.idx_8bit() {
            self.x = (self.x & 0xFF00) | ((self.x.wrapping_add(1)) & 0xFF);
            self.y = (self.y & 0xFF00) | ((self.y.wrapping_add(1)) & 0xFF);
        } else {
            self.x = self.x.wrapping_add(1);
            self.y = self.y.wrapping_add(1);
        }

        // Decrement C (accumulator = byte count - 1)
        self.c = self.c.wrapping_sub(1);
        self.dbr = dst_bank as u8;

        // If C != 0xFFFF, repeat (back up PC to re-execute this instruction)
        if self.c != 0xFFFF {
            self.pc = self.pc.wrapping_sub(3);
        }
    }

    /// MVP — block move previous (decrementing addresses).
    pub fn op_mvp(&mut self, bus: &mut dyn Bus816) {
        let dst_bank = self.fetch8(bus) as u32;
        let src_bank = self.fetch8(bus) as u32;

        let src_addr = (src_bank << 16) | self.x as u32;
        let dst_addr = (dst_bank << 16) | self.y as u32;
        let val = bus.read(src_addr, self.cycles);
        bus.write(dst_addr, val, self.cycles);

        // Decrement X and Y
        if self.emulation || self.flags.idx_8bit() {
            self.x = (self.x & 0xFF00) | ((self.x.wrapping_sub(1)) & 0xFF);
            self.y = (self.y & 0xFF00) | ((self.y.wrapping_sub(1)) & 0xFF);
        } else {
            self.x = self.x.wrapping_sub(1);
            self.y = self.y.wrapping_sub(1);
        }

        self.c = self.c.wrapping_sub(1);
        self.dbr = dst_bank as u8;

        if self.c != 0xFFFF {
            self.pc = self.pc.wrapping_sub(3);
        }
    }

    // ── Interrupt service ───────────────────────────────────────────────

    /// Service an interrupt (IRQ, NMI, BRK, or COP).
    /// In native mode, pushes PBR, then PC, then P.
    /// In emulation mode, pushes PC then P (like 6502).
    pub fn service_interrupt(&mut self, bus: &mut dyn Bus816, vector: u16, is_brk: bool) {
        if self.emulation {
            self.push16(bus, self.pc);
            let p = if is_brk {
                self.flags.bits() | Flags816::X.bits() // B flag set for BRK
            } else {
                self.flags.bits() & !Flags816::X.bits() // B flag clear for IRQ/NMI
            };
            self.push8(bus, p | Flags816::M.bits()); // bit 5 always set
            self.flags.insert(Flags816::I);
            // In emulation mode, D is cleared on interrupt (65C02 behavior)
            self.flags.remove(Flags816::D);
            self.pbr = 0; // interrupt vectors are in bank 0
        } else {
            // Native mode: push PBR
            self.push8(bus, self.pbr);
            self.push16(bus, self.pc);
            self.push8(bus, self.flags.bits());
            self.flags.insert(Flags816::I);
            self.flags.remove(Flags816::D);
            self.pbr = 0; // interrupt vectors are in bank 0
        }

        let lo = bus.read(vector as u32, self.cycles) as u16;
        let hi = bus.read((vector + 1) as u32, self.cycles) as u16;
        self.pc = (hi << 8) | lo;
    }
}
