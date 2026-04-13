//! 65C816 processor status register (P).
//!
//! In native mode, bits 4 and 5 control index register width (X) and
//! accumulator width (M) respectively. In emulation mode, these bits
//! revert to the 6502 meanings (B flag at bit 4, unused at bit 5).

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

bitflags! {
    /// Processor status register for the 65C816.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Flags816: u8 {
        /// Carry flag.
        const C = 0x01;
        /// Zero flag.
        const Z = 0x02;
        /// IRQ disable.
        const I = 0x04;
        /// Decimal mode.
        const D = 0x08;
        /// Native mode: Index register width (0 = 16-bit, 1 = 8-bit).
        /// Emulation mode: Break flag.
        const X = 0x10;
        /// Native mode: Accumulator width (0 = 16-bit, 1 = 8-bit).
        /// Emulation mode: always 1 (unused bit).
        const M = 0x20;
        /// Overflow flag.
        const V = 0x40;
        /// Negative flag.
        const N = 0x80;
    }
}

impl Flags816 {
    /// Power-on state: all flags set except V and N (emulation mode defaults).
    pub fn power_on() -> Self {
        Self::C | Self::Z | Self::I | Self::D | Self::X | Self::M
    }

    /// Set N and Z flags from an 8-bit result.
    #[inline]
    pub fn set_nz8(&mut self, val: u8) {
        self.set(Self::N, val & 0x80 != 0);
        self.set(Self::Z, val == 0);
    }

    /// Set N and Z flags from a 16-bit result.
    #[inline]
    pub fn set_nz16(&mut self, val: u16) {
        self.set(Self::N, val & 0x8000 != 0);
        self.set(Self::Z, val == 0);
    }

    /// True if the accumulator is in 8-bit mode (M=1 or emulation mode).
    #[inline]
    pub fn acc_8bit(&self) -> bool {
        self.contains(Self::M)
    }

    /// True if index registers are in 8-bit mode (X=1 or emulation mode).
    #[inline]
    pub fn idx_8bit(&self) -> bool {
        self.contains(Self::X)
    }
}
