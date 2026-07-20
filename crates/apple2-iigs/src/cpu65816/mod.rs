//! 65C816 CPU emulation.
//!
//! The 65C816 is a 16-bit extension of the 65C02 with a 24-bit address bus,
//! switchable 8/16-bit registers, and emulation mode for 65C02 compatibility.

mod addressing;
mod dispatch816;
mod flags816;
mod instructions;
mod registers;
#[cfg(test)]
mod tests;

pub use flags816::Flags816;
pub use registers::Cpu65816;

/// Memory bus interface for the 65C816.
///
/// All addresses are 24-bit (stored as `u32` with the top byte unused).
/// The CPU forms these by combining a bank byte with a 16-bit offset.
pub trait Bus816 {
    /// Read a byte with potential side-effects (e.g., soft-switch reads).
    fn read(&mut self, addr: u32, cycles: u64) -> u8;

    /// Write a byte with potential side-effects.
    fn write(&mut self, addr: u32, val: u8, cycles: u64);

    /// Read a byte without side-effects (for debugger / reset vector).
    fn read_raw(&self, addr: u32) -> u8;

    /// Handle a WDM trap with the given signature byte.
    ///
    /// Returns `Some((a, carry, xy))` if handled — the CPU updates A and the
    /// carry flag, and, when `xy` is `Some((x, y))`, the X and Y registers (used
    /// by SmartPort calls to report the transfer/parameter count). Returns
    /// `None` if unhandled (default).
    ///
    /// Used to implement SmartPort firmware dispatch via WDM instructions
    /// embedded in replacement slot ROMs.
    #[allow(clippy::type_complexity)]
    fn wdm_trap(
        &mut self,
        _signature: u8,
        _sp: u16,
        _pbr: u8,
        _emulation: bool,
    ) -> Option<(u8, bool, Option<(u16, u16)>)> {
        None
    }
}

/// Step the CPU by one instruction. Returns the number of cycles consumed.
pub fn step(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    dispatch816::step(cpu, bus)
}
