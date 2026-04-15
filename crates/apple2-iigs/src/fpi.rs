//! Fast Processor Interface (FPI) — speed control.
//!
//! The Apple IIgs can run at two speeds:
//! - Slow: 1.023 MHz (Apple IIe compatible)
//! - Fast: 2.8 MHz (native IIgs speed)
//!
//! The CYAREG ($C036) bit 7 controls the speed setting. However, accessing
//! Mega II I/O space ($C000-$C0FF in bank $00 or $E0) temporarily forces
//! the CPU to 1 MHz for the duration of the access.
//!
//! Cycle counting must account for speed: at 2.8 MHz, one "fast" cycle
//! is approximately 357ns vs 978ns at 1 MHz. The ratio is roughly 2.8:1.

use serde::{Deserialize, Serialize};

/// Speed state tracked by the FPI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CpuSpeed {
    /// 1.023 MHz (Apple IIe compatible).
    Slow,
    /// 2.8 MHz (native IIgs speed).
    #[default]
    Fast,
}

/// FPI speed control state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fpi {
    /// Current effective CPU speed.
    pub current_speed: CpuSpeed,
    /// Whether the speed register requests fast mode.
    pub speed_requested: bool,
    /// Whether I/O access has temporarily forced slow speed.
    pub io_slow_override: bool,
}

impl Default for Fpi {
    fn default() -> Self {
        Self {
            current_speed: CpuSpeed::Fast,
            speed_requested: true,
            io_slow_override: false,
        }
    }
}

impl Fpi {
    /// Update the speed setting from the CYAREG ($C036) register.
    pub fn set_speed_from_reg(&mut self, speed_reg: u8) {
        self.speed_requested = speed_reg & 0x80 != 0;
        self.update_effective_speed();
    }

    /// Called when an I/O access to Mega II space occurs.
    /// Temporarily forces slow speed.
    pub fn io_access(&mut self) {
        self.io_slow_override = true;
        self.update_effective_speed();
    }

    /// Called after the I/O access completes to restore normal speed.
    pub fn io_complete(&mut self) {
        self.io_slow_override = false;
        self.update_effective_speed();
    }

    /// Recalculate the effective speed.
    fn update_effective_speed(&mut self) {
        self.current_speed = if self.speed_requested && !self.io_slow_override {
            CpuSpeed::Fast
        } else {
            CpuSpeed::Slow
        };
    }

    /// Returns true if currently running at fast speed.
    #[inline]
    pub fn is_fast(&self) -> bool {
        self.current_speed == CpuSpeed::Fast
    }

    /// Convert fast cycles to slow-equivalent cycles for timing purposes.
    /// At 2.8 MHz, the CPU executes ~2.8 cycles per 1 MHz cycle.
    /// This returns the number of "reference" (1 MHz) cycles for the given
    /// number of CPU cycles at the current speed.
    #[inline]
    pub fn to_reference_cycles(&self, cpu_cycles: u64) -> u64 {
        match self.current_speed {
            CpuSpeed::Slow => cpu_cycles,
            // 2.8 MHz / 1.023 MHz ≈ 2.737. Use 8/3 ≈ 2.667 as a reasonable
            // approximation, or more precisely 35/13 ≈ 2.692.
            // For simplicity, we use the exact ratio: every fast cycle is
            // 1/2.8 of a reference cycle. To avoid floating point:
            // ref_cycles = cpu_cycles * 1023 / 2800
            // Simplified: ref_cycles = cpu_cycles * 93 / 255 ≈ cpu_cycles / 2.742
            CpuSpeed::Fast => cpu_cycles * 93 / 255,
        }
    }
}
