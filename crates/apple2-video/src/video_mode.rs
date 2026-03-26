//! Video mode enumeration.
//!
//! Mirrors `eVideoType` from `source/Video.h`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum VideoMode {
    /// Monochrome (white on black).
    Mono,
    /// Colour NTSC.
    #[default]
    Color,
    /// Monochrome with TV-style scanlines.
    MonoTv,
    /// Colour with TV-style scanlines.
    ColorTv,
    /// RGB card output (VidHD / 80-col card).
    Rgb,
    /// Amber phosphor.
    Amber,
    /// Green phosphor.
    Green,
}
