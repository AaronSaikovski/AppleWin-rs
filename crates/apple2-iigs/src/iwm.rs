//! Minimal IWM (Integrated Woz Machine) — the built-in 5.25"/3.5" disk
//! controller at slot 6 (`$C0E0-$C0EF`).
//!
//! The Apple IIgs firmware exercises the IWM's mode/status registers during its
//! power-on self-test (writing the mode register and reading it back through the
//! status register). This model implements that register handshake — and the
//! motor/phase/drive switches — well enough for the self-test to pass. No actual
//! 5.25" media is emulated: the data register reads back "no data / no disk", so
//! the firmware finds no bootable 5.25" drive and moves on to the other slots
//! (where the SmartPort answers).
//!
//! Register selection follows the standard IWM Q6/Q7 state machine:
//!   Q7=0 Q6=0 → read data register
//!   Q7=0 Q6=1 → read status register
//!   Q7=1 Q6=0 → read write-handshake register
//!   Q7=1 Q6=1 → write mode register (on a write access)

use serde::{Deserialize, Serialize};

/// IWM state (slot-6 disk controller).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Iwm {
    /// Q6 select line.
    q6: bool,
    /// Q7 select line.
    q7: bool,
    /// Motor on/off.
    motor: bool,
    /// Selected drive (false = drive 1, true = drive 2).
    drive2: bool,
    /// Mode register (set when writing with Q6=Q7=1).
    mode: u8,

    /// Write-handshake underrun toggle. With no real write-shift timing, the
    /// under-run bit alternates each read so firmware polling loops (which wait
    /// for either edge) always terminate.
    handshake_toggle: bool,
}

impl Iwm {
    /// Access a slot-6 register (`$C0E0-$C0EF`, `reg` = low nibble `0x0..=0xF`).
    ///
    /// Every access first toggles the addressed soft switch, then returns the
    /// byte the IWM would drive onto the bus. `is_write` marks a store access
    /// (which additionally latches the mode register when Q6=Q7=1).
    pub fn access(&mut self, reg: u8, is_write: bool, val: u8) -> u8 {
        match reg & 0x0F {
            // $C0E0-$C0E7: stepper phase lines — no stepper modelled.
            0x0..=0x7 => {}
            0x8 => self.motor = false,
            0x9 => self.motor = true,
            0xA => self.drive2 = false,
            0xB => self.drive2 = true,
            0xC => self.q6 = false,
            0xD => self.q6 = true,
            0xE => self.q7 = false,
            0xF => self.q7 = true,
            _ => unreachable!(),
        }

        // A write with both select lines high latches the mode register.
        if is_write && self.q6 && self.q7 {
            self.mode = val;
        }

        match (self.q7, self.q6) {
            // Read data register — no 5.25" media, so no valid data (bit 7 clear).
            (false, false) => 0x00,
            // Read status register: b7 SENSE, b6 reserved (0), b5 MOTOR,
            // b4-0 = mode register low bits. With no drive/disk the SENSE input
            // (write-protect) reads high, which the firmware's drive-detection
            // uses to fall through to a bounded read-and-timeout instead of
            // spinning forever waiting for a disk.
            (false, true) => 0x80 | (self.mode & 0x1F) | if self.motor { 0x20 } else { 0x00 },
            // Read write-handshake register: b7 = ready (always), b6 = no
            // under-run — toggled so both "wait for under-run" and "wait for
            // ready" firmware loops terminate without real shift-timing.
            (true, false) => {
                self.handshake_toggle = !self.handshake_toggle;
                0x80 | if self.handshake_toggle { 0x40 } else { 0x00 }
            }
            // Mode-register write path also reads back the mode register.
            (true, true) => self.mode,
        }
    }

    /// Reset the controller (power cycle / warm reset).
    pub fn reset(&mut self) {
        *self = Iwm::default();
    }
}
