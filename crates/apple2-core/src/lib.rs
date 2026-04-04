// apple2-core: pure Apple II emulation logic.
// No OS dependencies, no I/O, no UI.

pub mod bus;
pub mod card;
pub mod cards;
pub mod cpu;
pub mod emulator;
pub mod error;
pub mod model;
pub mod prodos;

#[cfg(test)]
mod bus_tests;

pub use emulator::Emulator;
pub use error::Error;
