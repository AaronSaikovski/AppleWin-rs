bitflags::bitflags! {
    /// 6502 processor status register (P).
    /// Bit 5 (RESERVED / UNUSED) is always set on real hardware.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
    pub struct Flags: u8 {
        /// Carry
        const C = 0x01;
        /// Zero
        const Z = 0x02;
        /// IRQ disable
        const I = 0x04;
        /// Decimal mode
        const D = 0x08;
        /// Break (software interrupt)
        const B = 0x10;
        /// Reserved — always 1
        const U = 0x20;
        /// Overflow
        const V = 0x40;
        /// Negative / Sign
        const N = 0x80;
    }
}

impl Flags {
    /// Reset to power-on state: I and U set, all others clear.
    #[inline]
    pub fn power_on() -> Self {
        Flags::I | Flags::U
    }

    /// Update N and Z flags based on `value`.
    #[inline]
    pub fn set_nz(&mut self, value: u8) {
        self.set(Flags::N, value & 0x80 != 0);
        self.set(Flags::Z, value == 0);
    }
}

impl Default for Flags {
    fn default() -> Self {
        Flags::power_on()
    }
}
