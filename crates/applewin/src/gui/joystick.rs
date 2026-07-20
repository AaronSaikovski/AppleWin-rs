//! Joystick / paddle / mouse emulation: polls physical gamepads (gilrs),
//! keyboard arrow/numpad keys, and the mouse pointer, and drives the two
//! Apple II game-port joysticks (paddle values + buttons) per the configured
//! joystick modes.

use super::*;
use crate::config::JoystickType;

/// Apply paddle trim, clamped to the 0–255 paddle range.
fn apply_paddle_trim(val: u8, trim: i8) -> u8 {
    (val as i16 + trim as i16).clamp(0, 255) as u8
}

impl EmulatorApp {
    pub(super) fn handle_joysticks(&mut self, ctx: &egui::Context) {
        // Map a gilrs gamepad to joystick N (0 or 1).
        let poll_gilrs_gamepad =
            |gilrs: &gilrs::Gilrs, gp_id: gilrs::GamepadId, swap_buttons: bool| -> (u8, u8, u8) {
                let gp = gilrs.gamepad(gp_id);
                // Left stick axes → paddles (–1.0..+1.0 → 0..255)
                let ax = gp.value(gilrs::Axis::LeftStickX);
                let ay = gp.value(gilrs::Axis::LeftStickY);
                let p0 = ((ax + 1.0) * 127.5).clamp(0.0, 255.0) as u8;
                // Y-axis: gilrs reports +Y as up; Apple II paddle +Y is down.
                let p1 = ((-ay + 1.0) * 127.5).clamp(0.0, 255.0) as u8;
                // Buttons: South=btn0, East=btn1
                let mut btns = 0u8;
                let (b0_mask, b1_mask) = if swap_buttons {
                    (0x02, 0x01)
                } else {
                    (0x01, 0x02)
                };
                if gp.is_pressed(gilrs::Button::South) {
                    btns |= b0_mask;
                }
                if gp.is_pressed(gilrs::Button::East) {
                    btns |= b1_mask;
                }
                if gp.is_pressed(gilrs::Button::West) {
                    btns |= b0_mask;
                }
                if gp.is_pressed(gilrs::Button::North) {
                    btns |= b1_mask;
                }
                (p0, p1, btns)
            };

        // --- Physical gamepad polling (gilrs) ---
        // Process gilrs events to track active gamepad.
        if let Some(ref mut gilrs) = self.gilrs {
            while let Some(ev) = gilrs.next_event() {
                match ev.event {
                    gilrs::EventType::Connected if self.active_gamepad.is_none() => {
                        self.active_gamepad = Some(ev.id);
                    }
                    gilrs::EventType::Disconnected if self.active_gamepad == Some(ev.id) => {
                        self.active_gamepad = gilrs.gamepads().next().map(|(id, _)| id);
                    }
                    _ => {}
                }
            }
        }

        // Update joystick 0
        match self.config.joystick0_type {
            JoystickType::Joystick1 | JoystickType::Joystick2 => {
                if let Some(ref gilrs) = self.gilrs {
                    // Joystick1 → first gamepad, Joystick2 → second gamepad
                    let target_idx = if self.config.joystick0_type == JoystickType::Joystick1 {
                        0
                    } else {
                        1
                    };
                    if let Some((gp_id, _)) = gilrs.gamepads().nth(target_idx) {
                        let (p0, p1, btns) =
                            poll_gilrs_gamepad(gilrs, gp_id, self.config.joystick_swap_buttons);
                        self.emu.bus.gamepad.paddle0 =
                            apply_paddle_trim(p0, self.config.paddle_x_trim);
                        self.emu.bus.gamepad.paddle1 =
                            apply_paddle_trim(p1, self.config.paddle_y_trim);
                        self.emu.bus.gamepad.buttons =
                            (self.emu.bus.gamepad.buttons & !0x03) | btns;
                    }
                }
            }
            JoystickType::KeypadArrows => self.apply_keypad_arrows(ctx),
            JoystickType::KeypadNumeric => self.apply_keypad_numeric(ctx),
            JoystickType::Mouse => self.apply_mouse_joystick(ctx),
            JoystickType::Disabled => {}
        }

        // Update joystick 1 (same logic, separate config)
        match self.config.joystick1_type {
            JoystickType::Joystick1 | JoystickType::Joystick2 => {
                if let Some(ref gilrs) = self.gilrs {
                    let target_idx = if self.config.joystick1_type == JoystickType::Joystick1 {
                        0
                    } else {
                        1
                    };
                    if let Some((gp_id, _)) = gilrs.gamepads().nth(target_idx) {
                        let (p0, p1, btns) =
                            poll_gilrs_gamepad(gilrs, gp_id, self.config.joystick_swap_buttons);
                        // Joystick 1 uses paddle2/paddle3 in AppleWin, but the
                        // Apple II only has 2 paddles accessible via game port.
                        // For compatibility, joystick 1 also writes paddle0/1
                        // (same as AppleWin behaviour for second joystick).
                        self.emu.bus.gamepad.paddle0 =
                            apply_paddle_trim(p0, self.config.paddle_x_trim);
                        self.emu.bus.gamepad.paddle1 =
                            apply_paddle_trim(p1, self.config.paddle_y_trim);
                        self.emu.bus.gamepad.buttons =
                            (self.emu.bus.gamepad.buttons & !0x03) | btns;
                    }
                }
            }
            JoystickType::KeypadArrows => self.apply_keypad_arrows(ctx),
            JoystickType::KeypadNumeric => self.apply_keypad_numeric(ctx),
            // Joystick 1 does not support Mouse mode.
            JoystickType::Mouse | JoystickType::Disabled => {}
        }
    }

    /// Arrow keys drive the paddles; Alt is button 0.
    fn apply_keypad_arrows(&mut self, ctx: &egui::Context) {
        let (lx, rx, uy, dy, b0) = ctx.input(|i| {
            (
                i.key_down(Key::ArrowLeft),
                i.key_down(Key::ArrowRight),
                i.key_down(Key::ArrowUp),
                i.key_down(Key::ArrowDown),
                i.modifiers.alt,
            )
        });
        let center = self.config.joystick_self_centering;
        self.emu.bus.gamepad.paddle0 = apply_paddle_trim(
            if lx {
                0
            } else if rx {
                255
            } else if center {
                127
            } else {
                self.emu.bus.gamepad.paddle0
            },
            self.config.paddle_x_trim,
        );
        self.emu.bus.gamepad.paddle1 = apply_paddle_trim(
            if uy {
                0
            } else if dy {
                255
            } else if center {
                127
            } else {
                self.emu.bus.gamepad.paddle1
            },
            self.config.paddle_y_trim,
        );
        let btn = if self.config.joystick_swap_buttons {
            0x02u8
        } else {
            0x01u8
        };
        if b0 {
            self.emu.bus.gamepad.buttons |= btn;
        } else {
            self.emu.bus.gamepad.buttons &= !btn;
        }
    }

    /// Numpad drives the paddles (4=left, 6=right, 8=up, 2=down, 0/5=fire).
    fn apply_keypad_numeric(&mut self, ctx: &egui::Context) {
        let (lx, rx, uy, dy, b0, b1) = ctx.input(|i| {
            (
                i.key_down(Key::Num4),
                i.key_down(Key::Num6),
                i.key_down(Key::Num8),
                i.key_down(Key::Num2),
                i.key_down(Key::Num0),
                i.key_down(Key::Num5),
            )
        });
        let center = self.config.joystick_self_centering;
        self.emu.bus.gamepad.paddle0 = apply_paddle_trim(
            if lx {
                0
            } else if rx {
                255
            } else if center {
                127
            } else {
                self.emu.bus.gamepad.paddle0
            },
            self.config.paddle_x_trim,
        );
        self.emu.bus.gamepad.paddle1 = apply_paddle_trim(
            if uy {
                0
            } else if dy {
                255
            } else if center {
                127
            } else {
                self.emu.bus.gamepad.paddle1
            },
            self.config.paddle_y_trim,
        );
        let (b0_mask, b1_mask) = if self.config.joystick_swap_buttons {
            (0x02u8, 0x01u8)
        } else {
            (0x01u8, 0x02u8)
        };
        if b0 {
            self.emu.bus.gamepad.buttons |= b0_mask;
        } else {
            self.emu.bus.gamepad.buttons &= !b0_mask;
        }
        if b1 {
            self.emu.bus.gamepad.buttons |= b1_mask;
        } else {
            self.emu.bus.gamepad.buttons &= !b1_mask;
        }
    }

    /// The mouse pointer position drives the paddles; the primary button is
    /// joystick button 0. (Joystick 0 only.)
    fn apply_mouse_joystick(&mut self, ctx: &egui::Context) {
        // Map mouse pointer position to paddle values.
        // Use the full window rect as the reference area (0..255).
        if let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) {
            let r = ctx.input(|i| i.screen_rect);
            let nx = ((pos.x - r.left()) / r.width()).clamp(0.0, 1.0);
            let ny = ((pos.y - r.top()) / r.height()).clamp(0.0, 1.0);
            self.emu.bus.gamepad.paddle0 =
                apply_paddle_trim((nx * 255.0) as u8, self.config.paddle_x_trim);
            self.emu.bus.gamepad.paddle1 =
                apply_paddle_trim((ny * 255.0) as u8, self.config.paddle_y_trim);
        }
        // Mouse button → joystick button 0
        let mb = ctx.input(|i| i.pointer.primary_down());
        let btn = if self.config.joystick_swap_buttons {
            0x02u8
        } else {
            0x01u8
        };
        if mb {
            self.emu.bus.gamepad.buttons |= btn;
        } else {
            self.emu.bus.gamepad.buttons &= !btn;
        }
    }
}
