pub mod flags;
pub mod cpu6502;
pub mod dispatch;

#[cfg(test)]
mod tests;

pub use cpu6502::Cpu;
pub use flags::Flags;
