//! Apple IIgs ADB (Apple Desktop Bus) micro-controller emulation.
//!
//! The IIgs uses a Mitsubishi M50740 micro-controller to manage the ADB bus.
//! The 65C816 communicates with it through GLU registers at $C024-$C027.
//!
//! This module emulates the micro-controller's behavior at the register level,
//! handling keyboard input, mouse data, BRAM access, and real-time clock.

use serde::{Deserialize, Serialize};

// ── ADB command types ───────────────────────────────────────────────────────

/// ADB GLU commands written to $C026.
/// The ROM firmware writes command bytes and reads results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AdbCmd {
    /// Abort current operation.
    Abort = 0x01,
    /// Reset the ADB bus and all devices.
    ResetBus = 0x02,
    /// Flush keyboard buffer.
    FlushKbd = 0x03,
    /// Set ADB modes.
    SetModes = 0x04,
    /// Clear ADB modes.
    ClearModes = 0x05,
    /// Set ADB configuration (3 bytes follow).
    SetConfig = 0x06,
    /// Sync (used during ROM init).
    Sync = 0x07,
    /// Write to BRAM byte.
    WriteBram = 0x09,
    /// Read BRAM byte.
    ReadBram = 0x0A,
    /// Read/write ADB device register (Talk/Listen).
    AdbCommand = 0x0B,
    /// Read modifier keys.
    ReadModifiers = 0x0C,
    /// Read config bytes.
    ReadConfig = 0x0D,
    /// Unknown / NOP.
    Unknown = 0xFF,
}

impl From<u8> for AdbCmd {
    fn from(val: u8) -> Self {
        match val {
            0x01 => AdbCmd::Abort,
            0x02 => AdbCmd::ResetBus,
            0x03 => AdbCmd::FlushKbd,
            0x04 => AdbCmd::SetModes,
            0x05 => AdbCmd::ClearModes,
            0x06 => AdbCmd::SetConfig,
            0x07 => AdbCmd::Sync,
            0x09 => AdbCmd::WriteBram,
            0x0A => AdbCmd::ReadBram,
            0x0B => AdbCmd::AdbCommand,
            0x0C => AdbCmd::ReadModifiers,
            0x0D => AdbCmd::ReadConfig,
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

    /// Mouse X delta for the next mouse Talk register 0 response.
    mouse_x_delta: u8,
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

        // Determine how many parameter bytes this command needs
        let params_needed = match cmd {
            AdbCmd::SetModes | AdbCmd::ClearModes => 1,
            AdbCmd::SetConfig => 3,
            AdbCmd::WriteBram => 2,  // address + data
            AdbCmd::ReadBram => 1,   // address
            AdbCmd::AdbCommand => 1, // ADB command byte
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
                // Cancel current operation
                self.response_queue.clear();
            }
            AdbCmd::ResetBus => {
                // Reset all ADB devices
                self.key_buffer.clear();
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
                    self.config[0] = self.cmd_params[0];
                    self.config[1] = self.cmd_params[1];
                    self.config[2] = self.cmd_params[2];
                }
            }
            AdbCmd::Sync => {
                // ROM uses this during init — just acknowledge
                self.response_queue.push(0x00);
            }
            AdbCmd::WriteBram => {
                // Handled by the caller (bus.rs) which has access to BRAM
                // The params are: [address, data]
                // We just acknowledge
            }
            AdbCmd::ReadBram => {
                // Handled by the caller (bus.rs) which has access to BRAM
                // The param is: [address]
                // Response will be pushed by the caller
            }
            AdbCmd::AdbCommand => {
                // ADB bus command — device address + command type + register
                // Params: [adb_cmd_byte]
                if let Some(&adb_byte) = self.cmd_params.first() {
                    self.handle_adb_bus_command(adb_byte);
                }
            }
            AdbCmd::ReadModifiers => {
                self.response_queue.push(self.modifiers);
            }
            AdbCmd::ReadConfig => {
                self.response_queue.push(self.config[0]);
                self.response_queue.push(self.config[1]);
                self.response_queue.push(self.config[2]);
            }
            AdbCmd::Unknown => {
                // Unknown command — just acknowledge
            }
        }

        self.cmd_params.clear();
    }

    /// Handle an ADB bus command (Talk/Listen/Flush/SendReset to a device).
    fn handle_adb_bus_command(&mut self, adb_byte: u8) {
        let _device = (adb_byte >> 4) & 0x0F;
        let cmd_type = (adb_byte >> 2) & 0x03;
        let register = adb_byte & 0x03;

        match cmd_type {
            0x00 => {
                // SendReset — reset all devices
            }
            0x01 => {
                // Flush — clear device register
            }
            0x02 => {
                // Talk — device sends register data to host
                // For keyboard (device 2) register 0: return key data
                // For mouse (device 3) register 0: return position data
                match (_device, register) {
                    (2, 0) => {
                        // Keyboard Talk Register 0 — return keycode if available
                        if let Some(key) = self.key_buffer.first().copied() {
                            self.key_buffer.remove(0);
                            self.response_queue.push(key);
                            self.response_queue.push(0xFF); // key up
                        } else {
                            // No data — SRQ not asserted, empty response
                        }
                    }
                    (2, 3) => {
                        // Keyboard Talk Register 3 — device info
                        // Handler ID for Apple Standard Keyboard = $02
                        self.response_queue.push(0x62); // flags + handler
                        self.response_queue.push(0x02); // handler ID
                    }
                    (3, 0) => {
                        // Mouse Talk Register 0 — return current mouse state
                        self.response_queue.push(self.mouse_data);
                        self.response_queue.push(self.mouse_x_delta);
                        // Reset mouse state after reading
                        self.mouse_data = 0x80; // no button, no movement
                        self.mouse_x_delta = 0x80;
                        self.status &= !(status::MOUSE_DATA | status::MOUSE_IRQ);
                    }
                    (3, 3) => {
                        // Mouse Talk Register 3 — device info
                        self.response_queue.push(0x63);
                        self.response_queue.push(0x01); // handler ID for mouse
                    }
                    _ => {
                        // Unknown device/register — no response
                    }
                }
            }
            0x03 => {
                // Listen — host sends data to device
                // Consume parameter bytes
            }
            _ => {}
        }
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
        let x_clamped = dx.clamp(-63, 63);

        self.mouse_data = if button { 0x00 } else { 0x80 } | ((y_clamped as u8) & 0x7F);

        // Store X delta for when mouse Talk register 0 is read
        // (The second byte returned in handle_adb_bus_command)
        self.mouse_x_delta = (x_clamped as u8) & 0x7F | 0x80;

        if dx != 0 || dy != 0 || button {
            self.status |= status::MOUSE_DATA | status::MOUSE_IRQ;
        }
    }
}
