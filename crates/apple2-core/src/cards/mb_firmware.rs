//! Shared Mockingboard firmware ROM (256 bytes).
//!
//! Used by Mockingboard, MegaAudio, and SD Music cards.

/// Mockingboard-D firmware ROM — first 256 bytes of the 2 KB ROM file,
/// mapped to the card's $Cn page.
pub static MB_FIRMWARE: &[u8; 256] = {
    const ROM: &[u8] = include_bytes!("../../../../roms/Mockingboard-D.rom");
    // Safety: ROM is guaranteed >= 256 bytes at compile time.
    unsafe { &*(ROM.as_ptr() as *const [u8; 256]) }
};
