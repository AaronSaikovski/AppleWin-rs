/// Apple II machine model.
/// Mirrors `eApple2Type` from `source/Common.h`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, Default,
)]
#[repr(u32)]
pub enum Apple2Model {
    AppleII = 0,
    AppleIIPlus = 1,
    AppleIIe = 2,
    #[default]
    AppleIIeEnh = 3, // Enhanced //e
    AppleIIc = 4,
    AppleIIcPlus = 5,
    AppleIIgs = 6, // placeholder — not emulated
    Clone = 7,     // Pravets, Franklin, etc.
}

/// CPU variant installed in the machine.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, Default,
)]
#[repr(u32)]
pub enum CpuType {
    Cpu6502 = 1,
    #[default]
    Cpu65C02 = 2,
    CpuZ80 = 3,
}
