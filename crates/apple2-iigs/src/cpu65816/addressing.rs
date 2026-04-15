//! 65C816 addressing mode implementations.
//!
//! Each method reads operand bytes from the instruction stream and computes
//! the effective 24-bit address. The CPU's `pc` is advanced past the operand.

use super::Bus816;
use super::registers::Cpu65816;

impl Cpu65816 {
    // ── Fetch helpers ───────────────────────────────────────────────────

    /// Fetch a byte at the current PC and advance PC.
    #[inline]
    pub fn fetch8(&mut self, bus: &mut dyn Bus816) -> u8 {
        let addr = self.full_pc();
        let val = bus.read(addr, self.cycles);
        self.pc = self.pc.wrapping_add(1);
        val
    }

    /// Fetch a 16-bit value (little-endian) at the current PC and advance PC by 2.
    #[inline]
    pub fn fetch16(&mut self, bus: &mut dyn Bus816) -> u16 {
        let lo = self.fetch8(bus) as u16;
        let hi = self.fetch8(bus) as u16;
        (hi << 8) | lo
    }

    /// Fetch a 24-bit value (little-endian) at the current PC and advance PC by 3.
    #[inline]
    pub fn fetch24(&mut self, bus: &mut dyn Bus816) -> u32 {
        let lo = self.fetch8(bus) as u32;
        let mid = self.fetch8(bus) as u32;
        let hi = self.fetch8(bus) as u32;
        (hi << 16) | (mid << 8) | lo
    }

    // ── Read helpers (from arbitrary 24-bit address) ────────────────────

    /// Read a 16-bit value from a 24-bit address (little-endian).
    #[inline]
    pub fn read16(&self, bus: &mut dyn Bus816, addr: u32) -> u16 {
        let lo = bus.read(addr, self.cycles) as u16;
        let hi = bus.read(addr.wrapping_add(1), self.cycles) as u16;
        (hi << 8) | lo
    }

    /// Read a 24-bit value from a 24-bit address (little-endian).
    #[inline]
    pub fn read24(&self, bus: &mut dyn Bus816, addr: u32) -> u32 {
        let lo = bus.read(addr, self.cycles) as u32;
        let mid = bus.read(addr.wrapping_add(1), self.cycles) as u32;
        let hi = bus.read(addr.wrapping_add(2), self.cycles) as u32;
        (hi << 16) | (mid << 8) | lo
    }

    // ── Immediate ───────────────────────────────────────────────────────

    /// Immediate 8-bit: operand byte follows opcode.
    #[inline]
    pub fn addr_imm8(&mut self, bus: &mut dyn Bus816) -> u8 {
        self.fetch8(bus)
    }

    /// Immediate 16-bit: two operand bytes follow opcode.
    #[inline]
    pub fn addr_imm16(&mut self, bus: &mut dyn Bus816) -> u16 {
        self.fetch16(bus)
    }

    // ── Direct Page ─────────────────────────────────────────────────────

    /// Direct Page address: dp + operand. In emulation mode with DL=0,
    /// wraps within page 0.
    #[inline]
    pub fn addr_dp(&mut self, bus: &mut dyn Bus816) -> u32 {
        let offset = self.fetch8(bus) as u16;
        if self.emulation && (self.dp & 0xFF) == 0 {
            // Emulation mode with DL=0: wrap within page
            (self.dp & 0xFF00).wrapping_add(offset) as u32
        } else {
            self.dp.wrapping_add(offset) as u32
        }
    }

    /// Direct Page,X: dp + operand + X.
    #[inline]
    pub fn addr_dp_x(&mut self, bus: &mut dyn Bus816) -> u32 {
        let offset = self.fetch8(bus) as u16;
        let x = self.get_x();
        if self.emulation && (self.dp & 0xFF) == 0 {
            (self.dp & 0xFF00).wrapping_add(offset).wrapping_add(x) as u32 & 0xFF
        } else {
            self.dp.wrapping_add(offset).wrapping_add(x) as u32
        }
    }

    /// Direct Page,Y: dp + operand + Y.
    #[inline]
    pub fn addr_dp_y(&mut self, bus: &mut dyn Bus816) -> u32 {
        let offset = self.fetch8(bus) as u16;
        let y = self.get_y();
        if self.emulation && (self.dp & 0xFF) == 0 {
            (self.dp & 0xFF00).wrapping_add(offset).wrapping_add(y) as u32 & 0xFF
        } else {
            self.dp.wrapping_add(offset).wrapping_add(y) as u32
        }
    }

    /// (Direct Page): indirect through dp.
    /// Reads a 16-bit pointer from the direct page address, combines with DBR.
    #[inline]
    pub fn addr_dp_ind(&mut self, bus: &mut dyn Bus816) -> u32 {
        let dp_addr = self.addr_dp(bus);
        let ptr = self.read16(bus, dp_addr);
        self.data_addr(ptr)
    }

    /// (Direct Page,X): pre-indexed indirect.
    /// dp + offset + X -> read 16-bit pointer, combine with DBR.
    #[inline]
    pub fn addr_dp_ind_x(&mut self, bus: &mut dyn Bus816) -> u32 {
        let dp_addr = self.addr_dp_x(bus);
        let ptr = self.read16(bus, dp_addr);
        self.data_addr(ptr)
    }

    /// (Direct Page),Y: post-indexed indirect.
    /// dp + offset -> read 16-bit pointer, combine with DBR, add Y.
    /// Returns (effective_address, page_crossed).
    #[inline]
    pub fn addr_dp_ind_y(&mut self, bus: &mut dyn Bus816) -> (u32, bool) {
        let dp_addr = self.addr_dp(bus);
        let ptr = self.read16(bus, dp_addr);
        let base = self.data_addr(ptr);
        let y = self.get_y() as u32;
        let ea = base.wrapping_add(y);
        let crossed = (base & 0xFF00) != (ea & 0xFF00);
        (ea, crossed)
    }

    /// [Direct Page]: indirect long.
    /// dp + offset -> read 24-bit pointer.
    #[inline]
    pub fn addr_dp_ind_long(&mut self, bus: &mut dyn Bus816) -> u32 {
        let dp_addr = self.addr_dp(bus);
        self.read24(bus, dp_addr)
    }

    /// [Direct Page],Y: indirect long indexed.
    /// dp + offset -> read 24-bit pointer, add Y.
    #[inline]
    pub fn addr_dp_ind_long_y(&mut self, bus: &mut dyn Bus816) -> u32 {
        let dp_addr = self.addr_dp(bus);
        let ptr = self.read24(bus, dp_addr);
        let y = self.get_y() as u32;
        ptr.wrapping_add(y)
    }

    // ── Absolute ────────────────────────────────────────────────────────

    /// Absolute: 16-bit address combined with DBR.
    #[inline]
    pub fn addr_abs(&mut self, bus: &mut dyn Bus816) -> u32 {
        let offset = self.fetch16(bus);
        self.data_addr(offset)
    }

    /// Absolute,X: 16-bit address + X, combined with DBR.
    /// Returns (effective_address, page_crossed).
    #[inline]
    pub fn addr_abs_x(&mut self, bus: &mut dyn Bus816) -> (u32, bool) {
        let offset = self.fetch16(bus);
        let base = self.data_addr(offset);
        let x = self.get_x() as u32;
        let ea = base.wrapping_add(x);
        let crossed = (base & 0xFF00) != (ea & 0xFF00);
        (ea, crossed)
    }

    /// Absolute,Y: 16-bit address + Y, combined with DBR.
    /// Returns (effective_address, page_crossed).
    #[inline]
    pub fn addr_abs_y(&mut self, bus: &mut dyn Bus816) -> (u32, bool) {
        let offset = self.fetch16(bus);
        let base = self.data_addr(offset);
        let y = self.get_y() as u32;
        let ea = base.wrapping_add(y);
        let crossed = (base & 0xFF00) != (ea & 0xFF00);
        (ea, crossed)
    }

    /// (Absolute): indirect. Used by JMP (abs).
    /// Reads 16-bit pointer from bank 0. Result is in PBR bank (for JMP).
    #[inline]
    pub fn addr_abs_ind(&mut self, bus: &mut dyn Bus816) -> u32 {
        let ptr_addr = self.fetch16(bus) as u32;
        let target = self.read16(bus, ptr_addr);
        self.program_addr(target)
    }

    /// (Absolute,X): indexed indirect. Used by JMP/JSR (abs,X).
    /// Reads 16-bit pointer from PBR bank at (operand + X).
    #[inline]
    pub fn addr_abs_ind_x(&mut self, bus: &mut dyn Bus816) -> u32 {
        let operand = self.fetch16(bus);
        let x = self.get_x();
        let ptr_addr = ((self.pbr as u32) << 16) | operand.wrapping_add(x) as u32;
        let target = self.read16(bus, ptr_addr);
        self.program_addr(target)
    }

    /// [Absolute]: indirect long. Used by JML [abs].
    /// Reads 24-bit pointer from bank 0.
    #[inline]
    pub fn addr_abs_ind_long(&mut self, bus: &mut dyn Bus816) -> u32 {
        let ptr_addr = self.fetch16(bus) as u32;
        self.read24(bus, ptr_addr)
    }

    // ── Absolute Long ───────────────────────────────────────────────────

    /// Absolute Long: 24-bit address.
    #[inline]
    pub fn addr_abs_long(&mut self, bus: &mut dyn Bus816) -> u32 {
        self.fetch24(bus)
    }

    /// Absolute Long,X: 24-bit address + X.
    #[inline]
    pub fn addr_abs_long_x(&mut self, bus: &mut dyn Bus816) -> u32 {
        let addr = self.fetch24(bus);
        let x = self.get_x() as u32;
        addr.wrapping_add(x)
    }

    // ── Stack Relative ──────────────────────────────────────────────────

    /// Stack Relative: SP + operand (always bank 0).
    #[inline]
    pub fn addr_sr(&mut self, bus: &mut dyn Bus816) -> u32 {
        let offset = self.fetch8(bus) as u16;
        self.sp.wrapping_add(offset) as u32
    }

    /// (Stack Relative),Y: indirect through stack relative address.
    /// SP + offset -> read 16-bit pointer, combine with DBR, add Y.
    #[inline]
    pub fn addr_sr_ind_y(&mut self, bus: &mut dyn Bus816) -> u32 {
        let sr_addr = self.addr_sr(bus);
        let ptr = self.read16(bus, sr_addr);
        let base = self.data_addr(ptr);
        let y = self.get_y() as u32;
        base.wrapping_add(y)
    }

    // ── Branch ──────────────────────────────────────────────────────────

    /// Relative branch (8-bit offset). Returns extra cycles if taken.
    #[inline]
    pub fn branch_rel8(&mut self, bus: &mut dyn Bus816, taken: bool) -> u8 {
        let offset = self.fetch8(bus) as i8;
        if !taken {
            return 0;
        }
        let new_pc = self.pc.wrapping_add(offset as u16);
        let extra = if self.emulation && (new_pc & 0xFF00) != (self.pc & 0xFF00) {
            2 // page crossing penalty in emulation mode only
        } else {
            1
        };
        self.pc = new_pc;
        extra
    }

    /// Relative long branch (16-bit offset). Always 4 cycles, no page penalty.
    #[inline]
    pub fn branch_rel16(&mut self, bus: &mut dyn Bus816) {
        let offset = self.fetch16(bus) as i16;
        self.pc = self.pc.wrapping_add(offset as u16);
    }
}
