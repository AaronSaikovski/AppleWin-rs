//! Apple IIgs shadow register ($C035).
//!
//! The shadow register controls which regions of bank $00/$01 are
//! automatically mirrored ("shadowed") to the corresponding addresses
//! in banks $E0/$E1 (fast RAM). The Mega II chip performs this shadowing
//! on every write to bank $00/$01 in a shadowed region.
//!
//! Bit layout of SHADOW register ($C035):
//!   Bit 7: reserved
//!   Bit 6: I/O + language card area inhibit (bank $00/$01 $C000-$FFFF)
//!   Bit 5: reserved
//!   Bit 4: Aux language card area inhibit (bank $01 $D000-$FFFF)
//!   Bit 3: Super Hi-Res inhibit (bank $01 $2000-$9FFF)
//!   Bit 2: Hi-Res page 2 inhibit (bank $00 $4000-$5FFF)
//!   Bit 1: Hi-Res page 1 inhibit (bank $00 $2000-$3FFF)
//!   Bit 0: Text/lo-res page inhibit (bank $00 $0400-$0BFF)
//!
//! When a bit is CLEAR (0), shadowing is ENABLED for that region.
//! When a bit is SET (1), shadowing is INHIBITED (disabled).

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

bitflags! {
    /// Shadow register bits (active-low: 0 = shadow enabled, 1 = inhibited).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ShadowReg: u8 {
        /// Text/lo-res page 1 ($0400-$0BFF in bank $00)
        const INHIBIT_TEXT     = 0x01;
        /// Hi-Res page 1 ($2000-$3FFF in bank $00)
        const INHIBIT_HIRES1  = 0x02;
        /// Hi-Res page 2 ($4000-$5FFF in bank $00)
        const INHIBIT_HIRES2  = 0x04;
        /// Super Hi-Res ($2000-$9FFF in bank $01)
        const INHIBIT_SHR     = 0x08;
        /// Aux language card ($D000-$FFFF in bank $01)
        const INHIBIT_AUX_LC  = 0x10;
        /// I/O + language card ($C000-$FFFF in bank $00)
        const INHIBIT_IO_LC   = 0x40;
    }
}

impl Default for ShadowReg {
    fn default() -> Self {
        // Power-on: all shadowing enabled (all bits clear)
        ShadowReg::empty()
    }
}

impl ShadowReg {
    /// Check if a write to bank $00 at the given offset should be shadowed
    /// to bank $E0.
    #[inline]
    pub fn should_shadow_bank0(&self, offset: u16) -> bool {
        match offset {
            0x0400..=0x0BFF => !self.contains(ShadowReg::INHIBIT_TEXT),
            0x2000..=0x3FFF => !self.contains(ShadowReg::INHIBIT_HIRES1),
            0x4000..=0x5FFF => !self.contains(ShadowReg::INHIBIT_HIRES2),
            0xC000..=0xFFFF => !self.contains(ShadowReg::INHIBIT_IO_LC),
            _ => false,
        }
    }

    /// Check if a write to bank $01 at the given offset should be shadowed
    /// to bank $E1.
    #[inline]
    pub fn should_shadow_bank1(&self, offset: u16) -> bool {
        match offset {
            0x0400..=0x0BFF => !self.contains(ShadowReg::INHIBIT_TEXT),
            0x2000..=0x9FFF => !self.contains(ShadowReg::INHIBIT_SHR),
            0xD000..=0xFFFF => !self.contains(ShadowReg::INHIBIT_AUX_LC),
            _ => false,
        }
    }
}
