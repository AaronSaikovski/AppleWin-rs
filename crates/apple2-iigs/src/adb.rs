//! Apple IIgs ADB (Apple Desktop Bus) micro-controller emulation.
//!
//! The IIgs uses a Mitsubishi M50740 micro-controller to manage the ADB bus.
//! The 65C816 communicates with it through GLU registers at $C024-$C027.
//!
//! This module emulates the micro-controller's behavior at the register level,
//! handling keyboard input, mouse data, BRAM access, and real-time clock.

use serde::{Deserialize, Serialize};

// ── ADB command types ───────────────────────────────────────────────────────

/// ADB GLU (keyboard micro-controller) commands written to `$C026`.
///
/// Command numbers match the real Apple IIgs ADB micro-controller as documented
/// in the IIgs Firmware Reference and implemented by KEGS/GSplus (`adb.c`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AdbCmd {
    /// Abort the current operation.
    Abort = 0x01,
    /// Flush the keyboard buffer.
    FlushKbd = 0x03,
    /// Set ADB mode bits (1 byte follows).
    SetModes = 0x04,
    /// Clear ADB mode bits (1 byte follows).
    ClearModes = 0x05,
    /// Set ADB configuration (3 bytes follow).
    SetConfig = 0x06,
    /// Synchronise (4 bytes on ROM 01, 8 bytes on ROM 03).
    Sync = 0x07,
    /// Write micro-controller memory (2 bytes follow).
    WriteMem = 0x08,
    /// Read micro-controller memory (2 bytes follow, responds 1 byte).
    ReadMem = 0x09,
    /// Read the ADB mode byte (responds 1 byte).
    ReadModes = 0x0A,
    /// Read the configuration bytes (responds 4 bytes).
    ReadConfig = 0x0B,
    /// Read the micro-controller version/revision (responds 1 byte).
    GetVersion = 0x0D,
    /// Read available character sets (responds 2 bytes).
    ReadCharSets = 0x0E,
    /// Read available keyboard layouts (responds 2 bytes).
    ReadKbdLayouts = 0x0F,
    /// Reset the micro-controller.
    Reset = 0x10,
    /// Send ADB key codes (1 byte follows).
    SendKeycodes = 0x11,
    /// Unknown / NOP.
    Unknown = 0xFF,
}

impl From<u8> for AdbCmd {
    fn from(val: u8) -> Self {
        match val {
            0x01 => AdbCmd::Abort,
            0x03 => AdbCmd::FlushKbd,
            0x04 => AdbCmd::SetModes,
            0x05 => AdbCmd::ClearModes,
            0x06 => AdbCmd::SetConfig,
            0x07 => AdbCmd::Sync,
            0x08 => AdbCmd::WriteMem,
            0x09 => AdbCmd::ReadMem,
            0x0A => AdbCmd::ReadModes,
            0x0B => AdbCmd::ReadConfig,
            0x0D => AdbCmd::GetVersion,
            0x0E => AdbCmd::ReadCharSets,
            0x0F => AdbCmd::ReadKbdLayouts,
            0x10 => AdbCmd::Reset,
            0x11 => AdbCmd::SendKeycodes,
            _ => AdbCmd::Unknown,
        }
    }
}

// ── ADB status register bits ($C027) ────────────────────────────────────────

/// ADB status register bit masks.
pub mod status {
    /// Command register full — GLU is processing a command.
    pub const CMD_FULL: u8 = 0x01;
    /// Mouse X-axis data available.
    pub const MOUSE_X: u8 = 0x02;
    /// Keyboard interrupt pending.
    pub const KEY_IRQ: u8 = 0x04;
    /// Key data available in data register.
    pub const KEY_DATA: u8 = 0x08;
    /// Mouse data interrupt pending.
    pub const MOUSE_IRQ: u8 = 0x10;
    /// Data register has valid response data.
    pub const DATA_VALID: u8 = 0x20;
    /// Command complete — response ready.
    pub const CMD_IRQ: u8 = 0x40;
    /// Mouse data available.
    pub const MOUSE_DATA: u8 = 0x80;
}

// ── ADB controller state ────────────────────────────────────────────────────

/// State of the ADB micro-controller.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Adb {
    /// ADB status register ($C027).
    pub status: u8,

    /// Data register ($C026) — response data.
    pub data_reg: u8,

    /// Modifier key register ($C025).
    pub modifiers: u8,

    /// Mouse data register ($C024).
    pub mouse_data: u8,

    /// Current command being processed.
    cmd_pending: Option<u8>,

    /// Command parameter bytes collected.
    cmd_params: Vec<u8>,

    /// Number of parameter bytes expected for current command.
    cmd_params_expected: usize,

    /// Response queue — bytes waiting to be read via $C026.
    response_queue: Vec<u8>,

    /// Keyboard buffer — key codes waiting to be delivered.
    key_buffer: Vec<u8>,

    /// ADB modes byte.
    pub modes: u8,

    /// ADB configuration bytes (3 bytes).
    pub config: [u8; 3],

    /// Delay counter — simulates micro-controller processing time.
    /// The ROM polls $C027 waiting for CMD_FULL to clear.
    pub delay_cycles: u64,

    /// Cycle count at which the current command completes.
    pub cmd_done_at: u64,

    /// True on ROM 03 machines: the `Sync` command takes 8 parameter bytes
    /// (versus 4 on ROM 00/01) and `GetVersion` reports revision 6 (versus 5).
    pub rom03: bool,
}

impl Adb {
    /// Write to the ADB data/command register ($C026).
    pub fn write_command(&mut self, val: u8, cycles: u64) {
        // If we're collecting parameters for a multi-byte command, add this byte
        if self.cmd_params_expected > 0 {
            self.cmd_params.push(val);
            self.cmd_params_expected -= 1;
            if self.cmd_params_expected == 0 {
                // All params collected, execute the command
                self.execute_command(cycles);
            }
            return;
        }

        // New command
        let cmd = AdbCmd::from(val);
        self.cmd_pending = Some(val);
        self.status |= status::CMD_FULL;
        self.status &= !status::CMD_IRQ;

        // Determine how many parameter bytes this command needs. Getting these
        // right is essential: an undercount makes the GLU treat the following
        // parameter bytes as fresh commands, desynchronising the whole stream.
        let params_needed = match cmd {
            AdbCmd::SetModes | AdbCmd::ClearModes | AdbCmd::SendKeycodes => 1,
            AdbCmd::SetConfig => 3,
            AdbCmd::WriteMem | AdbCmd::ReadMem => 2,
            AdbCmd::Sync => {
                if self.rom03 {
                    8
                } else {
                    4
                }
            }
            _ => 0,
        };

        self.cmd_params.clear();
        self.cmd_params_expected = params_needed;

        if params_needed == 0 {
            self.execute_command(cycles);
        }
    }

    /// Execute the pending command after all parameters are collected.
    fn execute_command(&mut self, cycles: u64) {
        let cmd_byte = match self.cmd_pending.take() {
            Some(c) => c,
            None => return,
        };
        let cmd = AdbCmd::from(cmd_byte);

        // Set a short delay before the command "completes".
        // The ROM polls $C027 waiting for CMD_FULL to clear.
        // ~200 cycles is enough to not hang the ROM.
        self.cmd_done_at = cycles + 200;

        match cmd {
            AdbCmd::Abort => {
                self.response_queue.clear();
            }
            AdbCmd::FlushKbd => {
                self.key_buffer.clear();
            }
            AdbCmd::SetModes => {
                if let Some(&mode) = self.cmd_params.first() {
                    self.modes |= mode;
                }
            }
            AdbCmd::ClearModes => {
                if let Some(&mode) = self.cmd_params.first() {
                    self.modes &= !mode;
                }
            }
            AdbCmd::SetConfig => {
                if self.cmd_params.len() >= 3 {
                    self.config.copy_from_slice(&self.cmd_params[..3]);
                }
            }
            // Sync / WriteMem / SendKeycodes: consume params, no response.
            AdbCmd::Sync | AdbCmd::WriteMem | AdbCmd::SendKeycodes => {}
            AdbCmd::ReadMem => {
                // Micro-controller memory read — respond with one byte.
                self.response_queue.push(0x00);
            }
            AdbCmd::ReadModes => {
                self.response_queue.push(self.modes);
            }
            AdbCmd::ReadConfig => {
                // 4 bytes: [$82, (mouse<<4)|kbd, (charset<<4)|layout, repeat].
                // Mouse ADB address 3, keyboard ADB address 2 (standard).
                self.response_queue.push(0x82);
                self.response_queue.push(0x32);
                self.response_queue.push(self.config[0]);
                self.response_queue.push(self.config[1]);
            }
            AdbCmd::GetVersion => {
                // ROM 01 reports revision 5; ROM 03 requires >= 6.
                self.response_queue.push(if self.rom03 { 6 } else { 5 });
            }
            AdbCmd::ReadCharSets => {
                // Number of available character sets = 8.
                self.response_queue.push(0x08);
                self.response_queue.push(0x00);
            }
            AdbCmd::ReadKbdLayouts => {
                // Number of available keyboard layouts = 10.
                self.response_queue.push(0x0A);
                self.response_queue.push(0x00);
            }
            AdbCmd::Reset => {
                self.key_buffer.clear();
                self.response_queue.clear();
            }
            AdbCmd::Unknown => {}
        }

        // If the command produced a response, flag it immediately so the ROM's
        // tight poll of $C027 (DATA_VALID) sees it without waiting on `update`.
        if !self.response_queue.is_empty() {
            self.status |= status::DATA_VALID;
            self.data_reg = self.response_queue[0];
        }

        self.cmd_params.clear();
    }

    /// Read from the ADB data register ($C026).
    /// Returns the next response byte, or 0 if none available.
    pub fn read_data(&mut self) -> u8 {
        if let Some(val) = self.response_queue.first().copied() {
            self.response_queue.remove(0);
            if self.response_queue.is_empty() {
                self.status &= !status::DATA_VALID;
            }
            val
        } else {
            self.status &= !status::DATA_VALID;
            0x00
        }
    }

    /// Read the ADB status register ($C027).
    pub fn read_status(&self) -> u8 {
        self.status
    }

    /// Update the ADB controller state. Called periodically.
    pub fn update(&mut self, cycles: u64) {
        // Check if a pending command has completed
        if self.status & status::CMD_FULL != 0 && cycles >= self.cmd_done_at {
            self.status &= !status::CMD_FULL;
            self.status |= status::CMD_IRQ;

            // If there's response data, flag it
            if !self.response_queue.is_empty() {
                self.status |= status::DATA_VALID;
                self.data_reg = self.response_queue[0];
            }
        }

        // Check for pending keyboard data
        if !self.key_buffer.is_empty() {
            self.status |= status::KEY_DATA | status::KEY_IRQ;
        }
    }

    /// Queue a key press from the host keyboard.
    pub fn key_press(&mut self, ascii: u8) {
        // Convert ASCII to ADB keycode + set strobe
        self.key_buffer.push(ascii);
        self.status |= status::KEY_DATA | status::KEY_IRQ;
    }

    /// Get the BRAM read address from the last ReadBram command params.
    pub fn bram_read_addr(&self) -> Option<u8> {
        if self.cmd_params.len() == 1 {
            Some(self.cmd_params[0])
        } else {
            None
        }
    }

    /// Get the BRAM write address and data from the last WriteBram command params.
    pub fn bram_write_params(&self) -> Option<(u8, u8)> {
        if self.cmd_params.len() >= 2 {
            Some((self.cmd_params[0], self.cmd_params[1]))
        } else {
            None
        }
    }

    /// Push a response byte (used by bus.rs for BRAM read results).
    pub fn push_response(&mut self, val: u8) {
        self.response_queue.push(val);
    }

    /// Update mouse position delta and button state.
    /// `dx`, `dy`: signed movement deltas (-63 to +63).
    /// `button`: true if mouse button is pressed.
    pub fn set_mouse_state(&mut self, dx: i8, dy: i8, button: bool) {
        // ADB mouse data format for Talk register 0:
        // Byte 0: bit 7 = !button, bits 6-0 = Y delta (signed, clamped)
        // Byte 1: bit 7 = always 1, bits 6-0 = X delta (signed, clamped)
        let y_clamped = dy.clamp(-63, 63);

        // Mouse register-0 low byte read at $C024: bit 7 = !button, bits 6-0 = Y.
        self.mouse_data = if button { 0x00 } else { 0x80 } | ((y_clamped as u8) & 0x7F);

        if dx != 0 || dy != 0 || button {
            self.status |= status::MOUSE_DATA | status::MOUSE_IRQ;
        }
    }
}
