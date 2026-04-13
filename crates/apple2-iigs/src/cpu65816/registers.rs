//! 65C816 CPU register state.

use super::Bus816;
use super::flags816::Flags816;
use serde::{Deserialize, Serialize};

/// 65C816 CPU state.
///
/// Register widths are dynamic: in native mode, the M and X flags in P
/// control whether the accumulator and index registers operate as 8-bit
/// or 16-bit. In emulation mode (E=1), all registers behave as 8-bit
/// (matching 65C02 behaviour).
///
/// The full 16-bit values are always stored; 8-bit mode simply masks
/// or truncates as needed during instruction execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cpu65816 {
    /// Full 16-bit accumulator. A = low byte (C & 0xFF), B = high byte (C >> 8).
    /// In 8-bit mode, operations only affect the low byte.
    pub c: u16,

    /// Index register X (16-bit storage; 8-bit mode uses low byte only).
    pub x: u16,

    /// Index register Y (16-bit storage; 8-bit mode uses low byte only).
    pub y: u16,

    /// Stack pointer. In native mode: full 16-bit. In emulation mode:
    /// forced to page 1 (high byte = 0x01).
    pub sp: u16,

    /// Program counter (16-bit offset within the current program bank).
    pub pc: u16,

    /// Program Bank Register (K / PBR). Forms bits 16-23 of the instruction
    /// fetch address: effective PC = (pbr << 16) | pc.
    pub pbr: u8,

    /// Data Bank Register (B / DBR). Forms bits 16-23 of data addresses
    /// for most addressing modes (absolute, absolute indexed, etc.).
    pub dbr: u8,

    /// Direct Page register (D). Replaces the 6502's hardcoded zero page.
    /// Direct page addresses are computed as D + offset.
    pub dp: u16,

    /// Processor status register.
    pub flags: Flags816,

    /// Emulation mode flag (E). When true, the CPU behaves like a 65C02:
    /// 8-bit registers, stack in page 1, page wrapping for direct page.
    pub emulation: bool,

    /// Total accumulated cycles since power-on.
    pub cycles: u64,

    /// Pending IRQ sources bitmask.
    pub irq_pending: u32,

    /// Pending NMI sources bitmask.
    pub nmi_pending: u32,

    /// WAI: CPU halted waiting for an interrupt.
    pub waiting: bool,

    /// STP: CPU stopped until reset.
    pub stopped: bool,

    /// IRQ deferral flag — matches 6502 hardware behaviour.
    pub irq_defer: bool,
}

impl Default for Cpu65816 {
    fn default() -> Self {
        Self {
            c: 0,
            x: 0,
            y: 0,
            sp: 0x01FF, // emulation mode: page 1
            pc: 0,
            pbr: 0,
            dbr: 0,
            dp: 0,
            flags: Flags816::power_on(),
            emulation: true, // starts in emulation mode
            cycles: 0,
            irq_pending: 0,
            nmi_pending: 0,
            waiting: false,
            stopped: false,
            irq_defer: false,
        }
    }
}

impl Cpu65816 {
    /// Create a new 65C816 in emulation mode (power-on state).
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset the CPU. Reads the reset vector from $00:FFFC.
    pub fn reset(&mut self, bus: &mut dyn Bus816) {
        self.emulation = true;
        self.flags = Flags816::power_on();
        self.sp = 0x01FF;
        self.dp = 0;
        self.dbr = 0;
        self.pbr = 0;
        self.waiting = false;
        self.stopped = false;
        self.irq_pending = 0;
        self.nmi_pending = 0;
        self.irq_defer = false;

        // Reset vector is always in bank 0.
        let lo = bus.read_raw(0x00_FFFC) as u16;
        let hi = bus.read_raw(0x00_FFFD) as u16;
        self.pc = (hi << 8) | lo;
    }

    // ── Register accessors (width-aware) ────────────────────────────────

    /// Get the accumulator value, respecting the current width mode.
    #[inline]
    pub fn a(&self) -> u16 {
        if self.emulation || self.flags.acc_8bit() {
            self.c & 0xFF
        } else {
            self.c
        }
    }

    /// Set the accumulator, respecting the current width mode.
    /// In 8-bit mode, only the low byte of `c` is modified.
    #[inline]
    pub fn set_a(&mut self, val: u16) {
        if self.emulation || self.flags.acc_8bit() {
            self.c = (self.c & 0xFF00) | (val & 0xFF);
        } else {
            self.c = val;
        }
    }

    /// Get the hidden B accumulator (high byte of C).
    #[inline]
    pub fn b(&self) -> u8 {
        (self.c >> 8) as u8
    }

    /// Get index X, respecting the current width mode.
    #[inline]
    pub fn get_x(&self) -> u16 {
        if self.emulation || self.flags.idx_8bit() {
            self.x & 0xFF
        } else {
            self.x
        }
    }

    /// Set index X, respecting the current width mode.
    #[inline]
    pub fn set_x(&mut self, val: u16) {
        if self.emulation || self.flags.idx_8bit() {
            self.x = val & 0xFF;
        } else {
            self.x = val;
        }
    }

    /// Get index Y, respecting the current width mode.
    #[inline]
    pub fn get_y(&self) -> u16 {
        if self.emulation || self.flags.idx_8bit() {
            self.y & 0xFF
        } else {
            self.y
        }
    }

    /// Set index Y, respecting the current width mode.
    #[inline]
    pub fn set_y(&mut self, val: u16) {
        if self.emulation || self.flags.idx_8bit() {
            self.y = val & 0xFF;
        } else {
            self.y = val;
        }
    }

    // ── Stack helpers ───────────────────────────────────────────────────

    /// Push a byte onto the stack.
    #[inline]
    pub fn push8(&mut self, bus: &mut dyn Bus816, val: u8) {
        bus.write(self.sp as u32, val, self.cycles);
        if self.emulation {
            // In emulation mode, SP wraps within page 1.
            self.sp = 0x0100 | ((self.sp.wrapping_sub(1)) & 0xFF);
        } else {
            self.sp = self.sp.wrapping_sub(1);
        }
    }

    /// Pop a byte from the stack.
    #[inline]
    pub fn pop8(&mut self, bus: &mut dyn Bus816) -> u8 {
        if self.emulation {
            self.sp = 0x0100 | ((self.sp.wrapping_add(1)) & 0xFF);
        } else {
            self.sp = self.sp.wrapping_add(1);
        }
        bus.read(self.sp as u32, self.cycles)
    }

    /// Push a 16-bit value (high byte first, then low byte).
    #[inline]
    pub fn push16(&mut self, bus: &mut dyn Bus816, val: u16) {
        self.push8(bus, (val >> 8) as u8);
        self.push8(bus, val as u8);
    }

    /// Pop a 16-bit value (low byte first, then high byte).
    #[inline]
    pub fn pop16(&mut self, bus: &mut dyn Bus816) -> u16 {
        let lo = self.pop8(bus) as u16;
        let hi = self.pop8(bus) as u16;
        (hi << 8) | lo
    }

    // ── Address formation helpers ───────────────────────────────────────

    /// Form a 24-bit address from the data bank register and a 16-bit offset.
    #[inline]
    pub fn data_addr(&self, offset: u16) -> u32 {
        ((self.dbr as u32) << 16) | (offset as u32)
    }

    /// Form a 24-bit address from the program bank register and a 16-bit offset.
    #[inline]
    pub fn program_addr(&self, offset: u16) -> u32 {
        ((self.pbr as u32) << 16) | (offset as u32)
    }

    /// The full 24-bit current PC.
    #[inline]
    pub fn full_pc(&self) -> u32 {
        self.program_addr(self.pc)
    }

    // ── Mode management ─────────────────────────────────────────────────

    /// Switch to emulation mode. Called when XCE sets E=1.
    /// Forces M=1, X=1 (8-bit registers), high bytes of X/Y cleared,
    /// SP high byte forced to 0x01.
    pub fn enter_emulation(&mut self) {
        self.emulation = true;
        self.flags.insert(Flags816::M | Flags816::X);
        self.x &= 0xFF;
        self.y &= 0xFF;
        self.sp = 0x0100 | (self.sp & 0xFF);
    }

    /// Switch to native mode. Called when XCE clears E=0.
    pub fn enter_native(&mut self) {
        self.emulation = false;
        // M and X flags retain their values; software uses REP/SEP to change them.
    }

    /// Called after REP/SEP modifies the flags register.
    /// In emulation mode, M and X are forced to 1.
    /// When X is set to 1, high bytes of X/Y are cleared.
    pub fn update_mode_flags(&mut self) {
        if self.emulation {
            self.flags.insert(Flags816::M | Flags816::X);
        }
        if self.flags.idx_8bit() {
            self.x &= 0xFF;
            self.y &= 0xFF;
        }
    }
}
