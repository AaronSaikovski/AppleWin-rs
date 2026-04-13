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
}

/// Step the CPU by one instruction. Returns the number of cycles consumed.
pub fn step(cpu: &mut Cpu65816, bus: &mut dyn Bus816) -> u8 {
    dispatch816::step(cpu, bus)
}
